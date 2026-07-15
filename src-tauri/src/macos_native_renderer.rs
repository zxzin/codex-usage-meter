use std::{
    ffi::c_void,
    ptr,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    thread,
    time::Duration,
};

use block2::RcBlock;
use objc2::{
    rc::{autoreleasepool, Retained},
    runtime::AnyObject,
    MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSImage, NSView};
use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGContext,
    CGDataProvider, CGImage, CGImageAlphaInfo,
};
use objc2_foundation::{NSError, NSNumber, NSString};
use objc2_quartz_core::{CALayer, CATransaction};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use tauri::Manager;

const FRAME_WIDTH: usize = 176;
const FRAME_HEIGHT: usize = 176;
const CAPTURE_WIDTH: usize = FRAME_WIDTH * 2;
const CAPTURE_HEIGHT: usize = FRAME_HEIGHT * 2;
const DISPLAY_SCALE: f64 = 2.0;
const SNAPSHOT_INTERVAL: Duration = Duration::from_millis(16);
const SNAPSHOT_START_DELAY: Duration = Duration::from_millis(150);

const BUILD_CAPTURE_PAIR: &str = r##"(() => {
  const source = document.querySelector(".app");
  if (!source) return false;

  const sourceNodes = [source, ...source.querySelectorAll("*")];
  const buildClone = () => {
    const clone = source.cloneNode(true);
    const cloneNodes = [clone, ...clone.querySelectorAll("*")];
    for (let index = 0; index < sourceNodes.length; index += 1) {
      const sourceNode = sourceNodes[index];
      const cloneNode = cloneNodes[index];
      if (!(sourceNode instanceof HTMLElement) || !(cloneNode instanceof HTMLElement)) continue;
      const computed = getComputedStyle(sourceNode);
      cloneNode.style.setProperty("animation", "none", "important");
      cloneNode.style.setProperty("transition", "none", "important");
      cloneNode.style.setProperty("transform", computed.transform, "important");
      cloneNode.style.setProperty("opacity", computed.opacity, "important");
    }
    return clone;
  };

  let host = document.getElementById("token-meter-app-store-capture");
  if (!host) {
    host = document.createElement("div");
    host.id = "token-meter-app-store-capture";
    host.setAttribute("aria-hidden", "true");
    host.inert = true;
    host.style.cssText = "position:fixed;inset:0;width:88px;height:88px;z-index:2147483647;display:flex;pointer-events:none;overflow:hidden";
    for (const color of ["#000000", "#ffffff"]) {
      const panel = document.createElement("div");
      panel.style.cssText = `position:relative;width:44px;height:88px;flex:0 0 44px;overflow:hidden;background:${color}`;
      const scaler = document.createElement("div");
      scaler.className = "token-meter-capture-scaler";
      scaler.style.cssText = "position:absolute;left:0;top:0;width:88px;height:88px;transform:scale(0.5);transform-origin:0 0";
      panel.appendChild(scaler);
      host.appendChild(panel);
    }
    document.documentElement.appendChild(host);
  }

  for (const scaler of host.querySelectorAll(".token-meter-capture-scaler")) {
    scaler.replaceChildren(buildClone());
  }
  return true;
})();"##;

static SNAPSHOT_LAYER: OnceLock<usize> = OnceLock::new();
static WEBVIEW_POINTER: OnceLock<usize> = OnceLock::new();
static SNAPSHOT_LOOP_STARTED: OnceLock<()> = OnceLock::new();
static SNAPSHOT_PENDING: AtomicBool = AtomicBool::new(false);

pub fn install(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|platform_webview| unsafe {
        let webview_ptr = platform_webview.inner();
        if webview_ptr.is_null() {
            return;
        }

        let webview = &*(webview_ptr.cast::<NSView>());
        webview.setAlphaValue(0.0);
        let wk_webview = &*(webview_ptr.cast::<WKWebView>());
        wk_webview.setUnderPageBackgroundColor(Some(&NSColor::blackColor()));
        let _ = WEBVIEW_POINTER.set(webview_ptr as usize);

        if SNAPSHOT_LAYER.get().is_some() {
            return;
        }

        let Some(parent) = webview.superview() else {
            return;
        };
        let marker = MainThreadMarker::new_unchecked();
        let render_view = NSView::initWithFrame(NSView::alloc(marker), webview.frame());
        render_view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        render_view.setWantsLayer(true);
        let Some(root_layer) = render_view.layer() else {
            return;
        };
        root_layer.setContentsScale(DISPLAY_SCALE);
        root_layer.setMasksToBounds(false);

        let snapshot_layer = CALayer::layer();
        snapshot_layer.setFrame(root_layer.bounds());
        snapshot_layer.setContentsScale(DISPLAY_SCALE);
        root_layer.addSublayer(&snapshot_layer);

        parent.addSubview(&render_view);
        parent.addSubview(webview);
        let _ = SNAPSHOT_LAYER.set(Retained::as_ptr(&snapshot_layer) as usize);
    });

    start_snapshot_loop(window.app_handle().clone());
}

fn start_snapshot_loop(app: tauri::AppHandle) {
    if SNAPSHOT_LOOP_STARTED.set(()).is_err() {
        return;
    }

    thread::spawn(move || {
        thread::sleep(SNAPSHOT_START_DELAY);
        loop {
            if !SNAPSHOT_PENDING.swap(true, Ordering::AcqRel) {
                let scheduled = app.run_on_main_thread(request_snapshot);
                if scheduled.is_err() {
                    SNAPSHOT_PENDING.store(false, Ordering::Release);
                }
            }
            thread::sleep(SNAPSHOT_INTERVAL);
        }
    });
}

fn request_snapshot() {
    let Some(webview_pointer) = WEBVIEW_POINTER.get().copied() else {
        SNAPSHOT_PENDING.store(false, Ordering::Release);
        return;
    };

    evaluate_script(webview_pointer, BUILD_CAPTURE_PAIR, move |capture_ready| {
        if !capture_ready {
            finish_capture(webview_pointer);
            return;
        }

        take_snapshot(webview_pointer, move |frame| {
            if let Some(frame) = frame {
                set_snapshot_layer(&frame);
            }
            finish_capture(webview_pointer);
        });
    });
}

fn evaluate_script<F>(webview_pointer: usize, script: &'static str, completion: F)
where
    F: Fn(bool) + 'static,
{
    unsafe {
        let webview = &*(webview_pointer as *const WKWebView);
        let script = NSString::from_str(script);
        let block = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
            completion(error.is_null() && !result.is_null());
        });
        webview.evaluateJavaScript_completionHandler(&script, Some(&block));
    }
}

fn take_snapshot<F>(webview_pointer: usize, completion: F)
where
    F: Fn(Option<Vec<u8>>) + 'static,
{
    unsafe {
        let marker = MainThreadMarker::new_unchecked();
        let webview = &*(webview_pointer as *const WKWebView);
        let configuration = WKSnapshotConfiguration::new(marker);
        configuration.setAfterScreenUpdates(true);
        let snapshot_width = NSNumber::new_f64(176.0);
        configuration.setSnapshotWidth(Some(&snapshot_width));
        let block = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            let frame = autoreleasepool(|_| {
                if image.is_null() || !error.is_null() {
                    None
                } else {
                    snapshot_pair_rgba(&*image)
                }
            });
            completion(frame);
        });
        webview.takeSnapshotWithConfiguration_completionHandler(Some(&configuration), &block);
    }
}

fn finish_capture(_webview_pointer: usize) {
    SNAPSHOT_PENDING.store(false, Ordering::Release);
}

unsafe fn snapshot_pair_rgba(image: &NSImage) -> Option<Vec<u8>> {
    let mut proposed_rect = CGRect::new(
        CGPoint::new(0.0, 0.0),
        CGSize::new(FRAME_WIDTH as f64, FRAME_HEIGHT as f64),
    );
    let source = image.CGImageForProposedRect_context_hints(&mut proposed_rect, None, None)?;
    let mut pixels = vec![0_u8; CAPTURE_WIDTH * CAPTURE_HEIGHT * 4];
    let color_space = CGColorSpace::new_device_rgb()?;
    let bitmap_info = CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0 | (4 << 12));
    let context = CGBitmapContextCreate(
        pixels.as_mut_ptr().cast::<c_void>(),
        CAPTURE_WIDTH,
        CAPTURE_HEIGHT,
        8,
        CAPTURE_WIDTH * 4,
        Some(&color_space),
        bitmap_info.0,
    )?;
    CGContext::draw_image(
        Some(&context),
        CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(CAPTURE_WIDTH as f64, CAPTURE_HEIGHT as f64),
        ),
        Some(&source),
    );

    recover_transparent_pair(&pixels)
}

fn recover_transparent_pair(capture: &[u8]) -> Option<Vec<u8>> {
    if capture.len() != CAPTURE_WIDTH * CAPTURE_HEIGHT * 4 {
        return None;
    }

    let top_score = pair_content_score(capture, 0);
    let bottom_score = pair_content_score(capture, FRAME_HEIGHT);
    let y_offset = if top_score >= bottom_score {
        0
    } else {
        FRAME_HEIGHT
    };
    if !pair_background_valid(capture, y_offset) {
        return None;
    }

    let mut output = vec![0_u8; FRAME_WIDTH * FRAME_HEIGHT * 4];
    for y in 0..FRAME_HEIGHT {
        for x in 0..FRAME_WIDTH {
            let black_offset = ((y + y_offset) * CAPTURE_WIDTH + x) * 4;
            let white_offset = ((y + y_offset) * CAPTURE_WIDTH + x + FRAME_WIDTH) * 4;
            let output_offset = (y * FRAME_WIDTH + x) * 4;
            recover_pixel(
                &capture[black_offset..black_offset + 4],
                &capture[white_offset..white_offset + 4],
                &mut output[output_offset..output_offset + 4],
            );
        }
    }
    Some(output)
}

fn pair_background_valid(capture: &[u8], y_offset: usize) -> bool {
    let corners = [
        (0, y_offset),
        (FRAME_WIDTH - 1, y_offset),
        (0, y_offset + FRAME_HEIGHT - 1),
        (FRAME_WIDTH - 1, y_offset + FRAME_HEIGHT - 1),
    ];
    corners
        .into_iter()
        .filter(|(x, y)| {
            let black_offset = (y * CAPTURE_WIDTH + x) * 4;
            let white_offset = (y * CAPTURE_WIDTH + x + FRAME_WIDTH) * 4;
            capture[black_offset..black_offset + 3]
                .iter()
                .all(|channel| *channel <= 8)
                && capture[white_offset..white_offset + 3]
                    .iter()
                    .all(|channel| *channel >= 247)
        })
        .count()
        >= 3
}

fn pair_content_score(capture: &[u8], y_offset: usize) -> u64 {
    let mut score = 0_u64;
    for y in (y_offset..y_offset + FRAME_HEIGHT).step_by(4) {
        for x in (0..FRAME_WIDTH).step_by(4) {
            let black_offset = (y * CAPTURE_WIDTH + x) * 4;
            let white_offset = (y * CAPTURE_WIDTH + x + FRAME_WIDTH) * 4;
            score += capture[black_offset..black_offset + 3]
                .iter()
                .map(|channel| *channel as u64)
                .sum::<u64>();
            score += capture[white_offset..white_offset + 3]
                .iter()
                .map(|channel| (255 - *channel) as u64)
                .sum::<u64>();
        }
    }
    score
}

fn recover_pixel(black: &[u8], white: &[u8], output: &mut [u8]) {
    let mut matte_channels = [0_i32; 3];
    for channel in 0..3 {
        matte_channels[channel] = (white[channel] as i32 - black[channel] as i32).clamp(0, 255);
    }
    matte_channels.sort_unstable();
    let alpha = 255 - matte_channels[1];
    if alpha <= 2 {
        output.fill(0);
        return;
    }

    for channel in 0..3 {
        output[channel] = ((black[channel] as i32 * 255 + alpha / 2) / alpha).clamp(0, 255) as u8;
    }
    output[3] = alpha as u8;
}

fn cg_image(bytes: &[u8]) -> Option<CFRetained<CGImage>> {
    let data = unsafe { CFData::new(None, bytes.as_ptr(), bytes.len() as isize) }?;
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let color_space = CGColorSpace::new_device_rgb()?;
    let bitmap_info = CGBitmapInfo(CGImageAlphaInfo::Last.0 | (4 << 12));
    unsafe {
        CGImage::new(
            FRAME_WIDTH,
            FRAME_HEIGHT,
            8,
            32,
            FRAME_WIDTH * 4,
            Some(&color_space),
            bitmap_info,
            Some(&provider),
            ptr::null(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

fn set_snapshot_layer(frame: &[u8]) {
    let Some(layer_pointer) = SNAPSHOT_LAYER.get().copied() else {
        return;
    };
    let Some(image) = cg_image(frame) else {
        return;
    };
    unsafe {
        let layer = &*(layer_pointer as *const CALayer);
        let contents = CFRetained::as_ptr(&image).cast::<AnyObject>().as_ref();
        CATransaction::begin();
        CATransaction::setAnimationDuration(0.04);
        layer.setContents(Some(contents));
        CATransaction::commit();
    }
}

#[cfg(test)]
mod tests {
    use super::recover_pixel;

    #[test]
    fn opaque_color_is_preserved() {
        let mut output = [0_u8; 4];
        recover_pixel(&[23, 35, 56, 255], &[23, 35, 56, 255], &mut output);
        assert_eq!(output, [23, 35, 56, 255]);
    }

    #[test]
    fn translucent_white_recovers_color_and_alpha() {
        let mut output = [0_u8; 4];
        recover_pixel(&[128, 128, 128, 255], &[255, 255, 255, 255], &mut output);
        assert_eq!(output, [255, 255, 255, 128]);
    }

    #[test]
    fn background_only_pixel_is_transparent() {
        let mut output = [255_u8; 4];
        recover_pixel(&[0, 0, 0, 255], &[255, 255, 255, 255], &mut output);
        assert_eq!(output, [0, 0, 0, 0]);
    }
}
