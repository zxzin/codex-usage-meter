use std::{
    f64::consts::PI,
    ffi::c_void,
    ptr::{self, NonNull},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

use block2::RcBlock;
use objc2::{
    rc::{autoreleasepool, Retained},
    runtime::AnyObject,
    AnyThread, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{NSAutoresizingMaskOptions, NSColor, NSImage, NSView};
use objc2_core_foundation::{CFData, CFRetained, CGAffineTransform, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, CGContext,
    CGDataProvider, CGImage, CGImageAlphaInfo,
};
use objc2_core_video::{kCVReturnSuccess, CVDisplayLink, CVOptionFlags, CVReturn, CVTimeStamp};
use objc2_foundation::{NSData, NSError, NSNumber, NSString};
use objc2_quartz_core::{CALayer, CATransaction};
use objc2_web_kit::{WKSnapshotConfiguration, WKWebView};
use serde::Deserialize;
use tauri::Manager;

const FRAME_WIDTH: usize = 176;
const FRAME_HEIGHT: usize = 176;
const CAPTURE_WIDTH: usize = FRAME_WIDTH * 2;
const CAPTURE_HEIGHT: usize = FRAME_HEIGHT * 2;
const DISPLAY_SCALE: f64 = 2.0;
const ANIMATION_INTERVAL: Duration = Duration::from_micros(16_667);
// Native layer interpolation bridges scheduler jitter without cross-fading whole UI snapshots.
const ANIMATION_TWEEN_SECONDS: f64 = 0.020;
const STATIC_SNAPSHOT_INTERVAL: Duration = Duration::from_millis(500);
const LOOP_START_DELAY: Duration = Duration::from_millis(150);
const BEE_BODY_PNG: &[u8] = include_bytes!("../../src/assets/living/bee-body-wingless-ui.png");
const BEE_WING_PNG: &[u8] = include_bytes!("../../src/assets/living/bee-wing-ui.png");
const NEAR_GHOST_BLUR_SOURCE_PX: usize = 4;
const FAR_GHOST_BLUR_SOURCE_PX: usize = 7;

const INSTALL_CAPTURE_PAIR: &str = r##"(() => {
  const stateKey = "__tokenMeterAppStoreCapture";
  if (window[stateKey]) return true;

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

  const state = {
    cloneNodeLists: [],
    inlineStyleLists: [],
    structureKey: "",
  };

  const sourceElement = () => document.querySelector("#root > .app");

  const structureKey = (nodes) => nodes
    .map((node) => `${node.nodeName}:${node.getAttribute?.("class") ?? ""}:${node.childElementCount}`)
    .join("|");

  const rebuildClones = (source, sourceNodes, key) => {
    state.cloneNodeLists = [];
    state.inlineStyleLists = [];
    for (const scaler of host.querySelectorAll(".token-meter-capture-scaler")) {
      const clone = source.cloneNode(true);
      scaler.replaceChildren(clone);
      state.cloneNodeLists.push([clone, ...clone.querySelectorAll("*")]);
      state.inlineStyleLists.push(new Array(sourceNodes.length).fill(null));
    }
    state.structureKey = key;
    return sourceNodes;
  };

  const syncAttributes = (sourceNode, cloneNode) => {
    const sourceAttributes = new Set();
    for (const attribute of sourceNode.attributes ?? []) {
      sourceAttributes.add(attribute.name);
      if (attribute.name === "style") continue;
      if (cloneNode.getAttribute(attribute.name) !== attribute.value) {
        cloneNode.setAttribute(attribute.name, attribute.value);
      }
    }
    for (const attribute of [...(cloneNode.attributes ?? [])]) {
      if (attribute.name !== "style" && !sourceAttributes.has(attribute.name)) {
        cloneNode.removeAttribute(attribute.name);
      }
    }

    if (sourceNode.childElementCount === 0 && cloneNode.textContent !== sourceNode.textContent) {
      cloneNode.textContent = sourceNode.textContent;
    }
  };

  const syncFrame = () => {
    const source = sourceElement();
    if (source) {
      let sourceNodes = [source, ...source.querySelectorAll("*")];
      const key = structureKey(sourceNodes);
      if (
        key !== state.structureKey
        || state.cloneNodeLists.some((nodes) => nodes.length !== sourceNodes.length)
      ) {
        sourceNodes = rebuildClones(source, sourceNodes, key);
      }

      const computedStyles = sourceNodes.map((node) =>
        node instanceof HTMLElement ? getComputedStyle(node) : null
      );
      for (let cloneListIndex = 0; cloneListIndex < state.cloneNodeLists.length; cloneListIndex += 1) {
        const cloneNodes = state.cloneNodeLists[cloneListIndex];
        const inlineStyles = state.inlineStyleLists[cloneListIndex];
        for (let index = 0; index < sourceNodes.length; index += 1) {
          const sourceNode = sourceNodes[index];
          const cloneNode = cloneNodes[index];
          if (!(sourceNode instanceof Element) || !(cloneNode instanceof Element)) continue;
          syncAttributes(sourceNode, cloneNode);

          const computed = computedStyles[index];
          if (!(sourceNode instanceof HTMLElement) || !(cloneNode instanceof HTMLElement) || !computed) {
            continue;
          }
          const sourceInlineStyle = sourceNode.getAttribute("style") ?? "";
          if (inlineStyles[index] !== sourceInlineStyle) {
            cloneNode.style.cssText = sourceInlineStyle;
            inlineStyles[index] = sourceInlineStyle;
          }
          cloneNode.style.setProperty("animation", "none", "important");
          cloneNode.style.setProperty("transition", "none", "important");
          cloneNode.style.setProperty("transform", computed.transform, "important");
          cloneNode.style.setProperty("opacity", computed.opacity, "important");
          if (cloneNode.matches(".bee-unit")) {
            cloneNode.style.setProperty("display", "none", "important");
          }
        }
      }
    }
    window.setTimeout(syncFrame, 500);
  };

  window[stateKey] = state;
  syncFrame();
  return true;
})();"##;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
struct BeeMotionContract {
    speed_full_rate_per_min: f64,
    speed_log_base_rate_per_min: f64,
    orbit_idle_ratio: f64,
    orbit_slow_seconds: f64,
    orbit_fast_seconds: f64,
    trail_start_ratio: f64,
    trail_near_max_opacity: f64,
    trail_far_max_opacity: f64,
    wing_slow_ms: f64,
    wing_fast_ms: f64,
    orbit_base_radius_px: f64,
    orbit_fast_radius_px: f64,
    orbit_radius_curve: f64,
    orbit_smoothing_per_second: f64,
    max_frame_seconds: f64,
    motion_acceleration_per_second: f64,
    motion_deceleration_per_second: f64,
    motion_idle_decay_per_second: f64,
    motion_stop_ratio: f64,
    motion_render_epsilon: f64,
    bee_width_px: f64,
    bee_aspect_width: f64,
    bee_aspect_height: f64,
    bee_facing_rotation_deg: f64,
    bee_active_scale: f64,
    bee_count: usize,
    static_placements: Vec<StaticPlacement>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StaticPlacement {
    x_px: f64,
    y_px: f64,
    rotation_deg: f64,
    scale: f64,
    flip_x: bool,
}

#[derive(Clone, Copy)]
struct BeeLayers {
    main: usize,
    left_wing: usize,
    right_wing: usize,
    near_ghost: usize,
    far_ghost: usize,
}

struct NativeMotionState {
    target_ratio: f64,
    visible_ratio: f64,
    angle_deg: f64,
    current_duration_seconds: f64,
    current_radius_px: f64,
    wing_phase: f64,
    last_update: Option<Instant>,
}

impl NativeMotionState {
    fn new(contract: &BeeMotionContract) -> Self {
        Self {
            target_ratio: 0.0,
            visible_ratio: 0.0,
            angle_deg: 0.0,
            current_duration_seconds: contract.orbit_slow_seconds,
            current_radius_px: contract.orbit_base_radius_px,
            wing_phase: 0.0,
            last_update: None,
        }
    }
}

#[derive(Clone, Copy)]
struct MotionFrame {
    active: bool,
    ratio: f64,
    angle_deg: f64,
    radius_px: f64,
    wing_phase: f64,
}

#[derive(Clone, Copy)]
struct WingPose {
    x: f64,
    y: f64,
    rotation_deg: f64,
    scale_x: f64,
    scale_y: f64,
    opacity: f64,
}

const LEFT_WING_POSES: [WingPose; 4] = [
    WingPose {
        x: -1.0,
        y: -3.0,
        rotation_deg: -18.0,
        scale_x: 1.08,
        scale_y: 1.18,
        opacity: 1.0,
    },
    WingPose {
        x: 0.0,
        y: 0.0,
        rotation_deg: 2.0,
        scale_x: 1.12,
        scale_y: 0.72,
        opacity: 0.84,
    },
    WingPose {
        x: 1.0,
        y: 3.0,
        rotation_deg: 16.0,
        scale_x: 1.2,
        scale_y: 0.36,
        opacity: 0.58,
    },
    WingPose {
        x: 0.0,
        y: 0.0,
        rotation_deg: -4.0,
        scale_x: 1.1,
        scale_y: 0.78,
        opacity: 0.82,
    },
];

const RIGHT_WING_POSES: [WingPose; 4] = [
    WingPose {
        x: 1.0,
        y: -3.0,
        rotation_deg: 16.0,
        scale_x: 1.06,
        scale_y: 1.14,
        opacity: 0.95,
    },
    WingPose {
        x: 0.0,
        y: 0.0,
        rotation_deg: -2.0,
        scale_x: 1.12,
        scale_y: 0.7,
        opacity: 0.8,
    },
    WingPose {
        x: -1.0,
        y: 3.0,
        rotation_deg: -15.0,
        scale_x: 1.2,
        scale_y: 0.34,
        opacity: 0.54,
    },
    WingPose {
        x: 0.0,
        y: 0.0,
        rotation_deg: 5.0,
        scale_x: 1.1,
        scale_y: 0.76,
        opacity: 0.78,
    },
];

static ROOT_LAYER: OnceLock<usize> = OnceLock::new();
static SNAPSHOT_LAYER: OnceLock<usize> = OnceLock::new();
static BEE_LAYERS: OnceLock<Vec<BeeLayers>> = OnceLock::new();
static WEBVIEW_POINTER: OnceLock<usize> = OnceLock::new();
static SNAPSHOT_LOOP_STARTED: OnceLock<()> = OnceLock::new();
static ANIMATION_LOOP_STARTED: OnceLock<()> = OnceLock::new();
static ANIMATION_APP_HANDLE: OnceLock<tauri::AppHandle> = OnceLock::new();
static DISPLAY_LINK_POINTER: OnceLock<usize> = OnceLock::new();
static DISPLAY_CALLBACK_COUNT: AtomicU64 = AtomicU64::new(0);
static SNAPSHOT_PENDING: AtomicBool = AtomicBool::new(false);
static ANIMATION_PENDING: AtomicBool = AtomicBool::new(false);
static BEE_FRAME_RENDERED: AtomicBool = AtomicBool::new(false);
static CAPTURE_PAIR_READY: AtomicBool = AtomicBool::new(false);
static MOTION_STATE: OnceLock<Mutex<NativeMotionState>> = OnceLock::new();

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
        snapshot_layer.setZPosition(3.0);
        root_layer.addSublayer(&snapshot_layer);

        let _ = ROOT_LAYER.set(Retained::as_ptr(&root_layer) as usize);
        let _ = SNAPSHOT_LAYER.set(Retained::as_ptr(&snapshot_layer) as usize);
        create_bee_layers(&root_layer);

        parent.addSubview(&render_view);
        parent.addSubview(webview);
    });

    update_bee_layers();
    start_snapshot_loop(window.app_handle().clone());
    start_animation_loop(window.app_handle().clone());
}

pub fn set_animation_burn_rate(rate_per_min: f64) {
    let contract = motion_contract();
    let target = bee_motion_ratio(rate_percent(rate_per_min, contract), contract);
    if let Ok(mut state) = motion_state().lock() {
        state.target_ratio = target;
    }
}

fn motion_contract() -> &'static BeeMotionContract {
    static CONTRACT: OnceLock<BeeMotionContract> = OnceLock::new();
    CONTRACT.get_or_init(|| {
        let contract: BeeMotionContract =
            serde_json::from_str(include_str!("../../src/bee-motion-contract.json"))
                .expect("bee motion contract must be valid");
        assert!(
            contract.motion_render_epsilon.is_finite() && contract.motion_render_epsilon > 0.0,
            "bee motion render epsilon must be positive"
        );
        assert!(contract.bee_count > 0, "bee count must be positive");
        assert_eq!(
            contract.static_placements.len(),
            contract.bee_count,
            "every bee must have an idle placement"
        );
        contract
    })
}

fn motion_state() -> &'static Mutex<NativeMotionState> {
    MOTION_STATE.get_or_init(|| Mutex::new(NativeMotionState::new(motion_contract())))
}

fn start_snapshot_loop(app: tauri::AppHandle) {
    if SNAPSHOT_LOOP_STARTED.set(()).is_err() {
        return;
    }

    thread::spawn(move || {
        thread::sleep(LOOP_START_DELAY);
        loop {
            let frame_started = Instant::now();
            if !SNAPSHOT_PENDING.swap(true, Ordering::AcqRel) {
                let scheduled = app.run_on_main_thread(request_static_snapshot);
                if scheduled.is_err() {
                    SNAPSHOT_PENDING.store(false, Ordering::Release);
                }
            }
            thread::sleep(STATIC_SNAPSHOT_INTERVAL.saturating_sub(frame_started.elapsed()));
        }
    });
}

fn start_animation_loop(app: tauri::AppHandle) {
    if ANIMATION_LOOP_STARTED.set(()).is_err() {
        return;
    }

    let _ = ANIMATION_APP_HANDLE.set(app.clone());
    if start_display_link() {
        return;
    }

    start_animation_fallback(app);
}

#[allow(deprecated)]
fn start_display_link() -> bool {
    unsafe {
        let mut display_link_pointer = ptr::null_mut::<CVDisplayLink>();
        let Some(output) = NonNull::new(&mut display_link_pointer) else {
            return false;
        };
        if CVDisplayLink::create_with_active_cg_displays(output) != kCVReturnSuccess {
            return false;
        }
        let Some(display_link) = display_link_pointer.as_ref() else {
            return false;
        };
        if display_link.set_output_callback(Some(display_link_callback), ptr::null_mut())
            != kCVReturnSuccess
        {
            return false;
        }
        if display_link.start() != kCVReturnSuccess {
            return false;
        }
        let _ = DISPLAY_LINK_POINTER.set(display_link_pointer as usize);
        true
    }
}

unsafe extern "C-unwind" fn display_link_callback(
    _display_link: NonNull<CVDisplayLink>,
    _now: NonNull<CVTimeStamp>,
    output_time: NonNull<CVTimeStamp>,
    _flags_in: CVOptionFlags,
    _flags_out: NonNull<CVOptionFlags>,
    _user_info: *mut c_void,
) -> CVReturn {
    let output_time = output_time.as_ref();
    let refresh_seconds = if output_time.videoTimeScale > 0 {
        output_time.videoRefreshPeriod as f64 / output_time.videoTimeScale as f64
    } else {
        0.0
    };
    let callback_index = DISPLAY_CALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);
    if refresh_seconds > 0.0 && refresh_seconds < 0.012 && callback_index % 2 == 1 {
        return kCVReturnSuccess;
    }

    schedule_animation_frame();
    kCVReturnSuccess
}

fn schedule_animation_frame() {
    if ANIMATION_PENDING.swap(true, Ordering::AcqRel) {
        return;
    }
    let Some(app) = ANIMATION_APP_HANDLE.get() else {
        ANIMATION_PENDING.store(false, Ordering::Release);
        return;
    };
    let scheduled = app.run_on_main_thread(|| {
        update_bee_layers();
        ANIMATION_PENDING.store(false, Ordering::Release);
    });
    if scheduled.is_err() {
        ANIMATION_PENDING.store(false, Ordering::Release);
    }
}

fn start_animation_fallback(app: tauri::AppHandle) {
    thread::spawn(move || loop {
        let frame_started = Instant::now();
        let _ = &app;
        schedule_animation_frame();
        thread::sleep(ANIMATION_INTERVAL.saturating_sub(frame_started.elapsed()));
    });
}

fn request_static_snapshot() {
    let Some(webview_pointer) = WEBVIEW_POINTER.get().copied() else {
        SNAPSHOT_PENDING.store(false, Ordering::Release);
        return;
    };

    if !CAPTURE_PAIR_READY.load(Ordering::Acquire) {
        evaluate_script(
            webview_pointer,
            INSTALL_CAPTURE_PAIR,
            move |capture_ready| {
                if !capture_ready {
                    finish_static_capture();
                    return;
                }
                CAPTURE_PAIR_READY.store(true, Ordering::Release);
                capture_static_frame(webview_pointer);
            },
        );
        return;
    }

    capture_static_frame(webview_pointer);
}

fn capture_static_frame(webview_pointer: usize) {
    take_snapshot(webview_pointer, move |frame| {
        if let Some(frame) = frame {
            set_snapshot_layer(&frame);
        }
        finish_static_capture();
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

fn finish_static_capture() {
    SNAPSHOT_PENDING.store(false, Ordering::Release);
}

fn create_bee_layers(root_layer: &CALayer) {
    let Some(body_image) = image_from_png(BEE_BODY_PNG) else {
        return;
    };
    let Some(wing_image) = image_from_png(BEE_WING_PNG) else {
        return;
    };
    let near_ghost_image = blurred_image(&body_image, NEAR_GHOST_BLUR_SOURCE_PX);
    let far_ghost_image = blurred_image(&body_image, FAR_GHOST_BLUR_SOURCE_PX);
    let contract = motion_contract();
    let width = contract.bee_width_px;
    let height = width * contract.bee_aspect_height / contract.bee_aspect_width;
    let bounds = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(width, height));
    let mut layers = Vec::with_capacity(contract.bee_count);

    for _ in 0..contract.bee_count {
        let far_ghost = content_layer(bounds, far_ghost_image.as_deref().unwrap_or(&body_image));
        far_ghost.setZPosition(4.0);
        root_layer.addSublayer(&far_ghost);

        let near_ghost = content_layer(bounds, near_ghost_image.as_deref().unwrap_or(&body_image));
        near_ghost.setZPosition(5.0);
        root_layer.addSublayer(&near_ghost);

        let main = CALayer::layer();
        main.setBounds(bounds);
        main.setContentsScale(DISPLAY_SCALE);
        main.setZPosition(6.0);

        let (left_clip, left_wing) = wing_layer(bounds, &wing_image, 0.0, 0.52, 0.34, 0.26);
        let (right_clip, right_wing) = wing_layer(bounds, &wing_image, 0.4, 0.6, 0.53, 0.25);
        main.addSublayer(&left_clip);
        main.addSublayer(&right_clip);

        let body = content_layer(bounds, &body_image);
        body.setZPosition(2.0);
        main.addSublayer(&body);
        root_layer.addSublayer(&main);

        layers.push(BeeLayers {
            main: Retained::as_ptr(&main) as usize,
            left_wing: Retained::as_ptr(&left_wing) as usize,
            right_wing: Retained::as_ptr(&right_wing) as usize,
            near_ghost: Retained::as_ptr(&near_ghost) as usize,
            far_ghost: Retained::as_ptr(&far_ghost) as usize,
        });
    }

    let _ = BEE_LAYERS.set(layers);
}

fn content_layer(bounds: CGRect, image: &CGImage) -> Retained<CALayer> {
    let layer = CALayer::layer();
    layer.setBounds(bounds);
    layer.setPosition(CGPoint::new(
        bounds.size.width / 2.0,
        bounds.size.height / 2.0,
    ));
    layer.setContentsScale(DISPLAY_SCALE);
    set_layer_contents(&layer, image);
    layer
}

fn wing_layer(
    bounds: CGRect,
    image: &CGImage,
    clip_x_ratio: f64,
    clip_width_ratio: f64,
    anchor_x_ratio: f64,
    anchor_y_from_top_ratio: f64,
) -> (Retained<CALayer>, Retained<CALayer>) {
    let width = bounds.size.width;
    let height = bounds.size.height;
    let clip_x = width * clip_x_ratio;
    let clip_width = width * clip_width_ratio;
    let clip = CALayer::layer();
    clip.setFrame(CGRect::new(
        CGPoint::new(clip_x, 0.0),
        CGSize::new(clip_width, height),
    ));
    clip.setMasksToBounds(true);
    clip.setZPosition(1.0);

    let wing = content_layer(bounds, image);
    let anchor_y = 1.0 - anchor_y_from_top_ratio;
    wing.setAnchorPoint(CGPoint::new(anchor_x_ratio, anchor_y));
    wing.setPosition(CGPoint::new(
        width * anchor_x_ratio - clip_x,
        height * anchor_y,
    ));
    clip.addSublayer(&wing);
    (clip, wing)
}

fn image_from_png(bytes: &[u8]) -> Option<Retained<CGImage>> {
    unsafe {
        let data = NSData::dataWithBytes_length(bytes.as_ptr().cast::<c_void>(), bytes.len());
        let image = NSImage::initWithData(NSImage::alloc(), &data)?;
        let mut proposed_rect = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(160.0, 144.0));
        image.CGImageForProposedRect_context_hints(&mut proposed_rect, None, None)
    }
}

fn blurred_image(image: &CGImage, radius: usize) -> Option<CFRetained<CGImage>> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    let mut pixels = vec![0_u8; width * height * 4];
    let color_space = CGColorSpace::new_device_rgb()?;
    let bitmap_info = CGBitmapInfo(CGImageAlphaInfo::PremultipliedLast.0 | (4 << 12));
    let context = unsafe {
        CGBitmapContextCreate(
            pixels.as_mut_ptr().cast::<c_void>(),
            width,
            height,
            8,
            width * 4,
            Some(&color_space),
            bitmap_info.0,
        )?
    };
    CGContext::draw_image(
        Some(&context),
        CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(width as f64, height as f64),
        ),
        Some(image),
    );
    let blurred = box_blur_rgba(&pixels, width, height, radius);
    rgba_image(&blurred, width, height, true)
}

fn box_blur_rgba(input: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    if radius == 0 || input.len() != width * height * 4 {
        return input.to_vec();
    }

    let mut horizontal = vec![0_u8; input.len()];
    for y in 0..height {
        for x in 0..width {
            let start = x.saturating_sub(radius);
            let end = (x + radius).min(width - 1);
            let count = (end - start + 1) as u32;
            for channel in 0..4 {
                let sum = (start..=end)
                    .map(|sample_x| input[(y * width + sample_x) * 4 + channel] as u32)
                    .sum::<u32>();
                horizontal[(y * width + x) * 4 + channel] = (sum / count) as u8;
            }
        }
    }

    let mut output = vec![0_u8; input.len()];
    for y in 0..height {
        let start = y.saturating_sub(radius);
        let end = (y + radius).min(height - 1);
        let count = (end - start + 1) as u32;
        for x in 0..width {
            for channel in 0..4 {
                let sum = (start..=end)
                    .map(|sample_y| horizontal[(sample_y * width + x) * 4 + channel] as u32)
                    .sum::<u32>();
                output[(y * width + x) * 4 + channel] = (sum / count) as u8;
            }
        }
    }
    output
}

fn rgba_image(
    bytes: &[u8],
    width: usize,
    height: usize,
    premultiplied: bool,
) -> Option<CFRetained<CGImage>> {
    let data = unsafe { CFData::new(None, bytes.as_ptr(), bytes.len() as isize) }?;
    let provider = CGDataProvider::with_cf_data(Some(&data))?;
    let color_space = CGColorSpace::new_device_rgb()?;
    let alpha_info = if premultiplied {
        CGImageAlphaInfo::PremultipliedLast
    } else {
        CGImageAlphaInfo::Last
    };
    let bitmap_info = CGBitmapInfo(alpha_info.0 | (4 << 12));
    unsafe {
        CGImage::new(
            width,
            height,
            8,
            32,
            width * 4,
            Some(&color_space),
            bitmap_info,
            Some(&provider),
            ptr::null(),
            true,
            CGColorRenderingIntent::RenderingIntentDefault,
        )
    }
}

fn set_layer_contents(layer: &CALayer, image: &CGImage) {
    unsafe {
        let contents = (image as *const CGImage).cast::<AnyObject>().as_ref();
        layer.setContents(contents);
    }
}

fn update_bee_layers() {
    let Some(root_pointer) = ROOT_LAYER.get().copied() else {
        ANIMATION_PENDING.store(false, Ordering::Release);
        return;
    };
    let Some(snapshot_pointer) = SNAPSHOT_LAYER.get().copied() else {
        ANIMATION_PENDING.store(false, Ordering::Release);
        return;
    };
    let Some(bee_layers) = BEE_LAYERS.get() else {
        ANIMATION_PENDING.store(false, Ordering::Release);
        return;
    };

    let contract = motion_contract();
    let frame = next_motion_frame(contract);
    unsafe {
        let root_layer = &*(root_pointer as *const CALayer);
        let snapshot_layer = &*(snapshot_pointer as *const CALayer);
        let root_bounds = root_layer.bounds();
        let center = CGPoint::new(root_bounds.size.width / 2.0, root_bounds.size.height / 2.0);

        CATransaction::begin();
        if BEE_FRAME_RENDERED.swap(true, Ordering::AcqRel) {
            CATransaction::setAnimationDuration(ANIMATION_TWEEN_SECONDS);
        } else {
            CATransaction::setDisableActions(true);
        }
        snapshot_layer.setFrame(root_bounds);

        let trail_intensity = trail_intensity(frame.ratio, contract);
        let near_opacity = trail_intensity * contract.trail_near_max_opacity * 0.8;
        let far_opacity = trail_intensity * contract.trail_far_max_opacity * 0.8;

        for (index, layers) in bee_layers.iter().enumerate() {
            let main = &*(layers.main as *const CALayer);
            let near = &*(layers.near_ghost as *const CALayer);
            let far = &*(layers.far_ghost as *const CALayer);

            if frame.active {
                let base_angle = index as f64 * (360.0 / contract.bee_count as f64) - 18.0;
                set_orbit_pose(
                    far,
                    center,
                    frame.angle_deg + base_angle - 24.0,
                    frame.radius_px,
                    contract.bee_facing_rotation_deg,
                    contract.bee_active_scale,
                );
                set_orbit_pose(
                    near,
                    center,
                    frame.angle_deg + base_angle - 12.0,
                    frame.radius_px,
                    contract.bee_facing_rotation_deg,
                    contract.bee_active_scale,
                );
                set_orbit_pose(
                    main,
                    center,
                    frame.angle_deg + base_angle,
                    frame.radius_px,
                    contract.bee_facing_rotation_deg,
                    contract.bee_active_scale,
                );
                far.setOpacity(far_opacity as f32);
                near.setOpacity(near_opacity as f32);
                main.setOpacity(1.0);
            } else {
                let placement =
                    &contract.static_placements[index % contract.static_placements.len()];
                set_static_pose(main, center, placement);
                far.setOpacity(0.0);
                near.setOpacity(0.0);
                main.setOpacity(1.0);
            }

            let left_wing = &*(layers.left_wing as *const CALayer);
            let right_wing = &*(layers.right_wing as *const CALayer);
            let phase = if frame.active { frame.wing_phase } else { 0.0 };
            let static_opacity = if frame.active { 1.0 } else { 0.86 };
            apply_wing_pose(
                left_wing,
                sample_wing_pose(&LEFT_WING_POSES, phase),
                static_opacity,
            );
            apply_wing_pose(
                right_wing,
                sample_wing_pose(&RIGHT_WING_POSES, phase),
                static_opacity,
            );
        }

        CATransaction::commit();
    }
}

fn next_motion_frame(contract: &BeeMotionContract) -> MotionFrame {
    let now = Instant::now();
    let mut state = motion_state()
        .lock()
        .expect("native bee motion mutex poisoned");
    let frame_seconds = state
        .last_update
        .map(|last| now.duration_since(last).as_secs_f64())
        .unwrap_or(0.0)
        .clamp(0.0, contract.max_frame_seconds);
    state.last_update = Some(now);

    state.visible_ratio = next_motion_ratio(
        state.visible_ratio,
        state.target_ratio,
        frame_seconds,
        contract,
    );
    let active = state.visible_ratio > contract.motion_stop_ratio;
    let target_duration = orbit_duration(state.visible_ratio, contract);
    let target_radius = orbit_radius(state.visible_ratio, contract);
    let smoothing = 1.0 - (-frame_seconds * contract.orbit_smoothing_per_second).exp();
    state.current_duration_seconds +=
        (target_duration - state.current_duration_seconds) * smoothing;
    state.current_radius_px += (target_radius - state.current_radius_px) * smoothing;

    if active {
        state.angle_deg = (state.angle_deg
            + frame_seconds * 360.0 / state.current_duration_seconds.max(0.2))
            % 360.0;
        let wing_duration_seconds = wing_duration_ms(state.visible_ratio, contract) / 1000.0;
        state.wing_phase =
            (state.wing_phase + frame_seconds / wing_duration_seconds.max(0.001)) % 1.0;
    }

    MotionFrame {
        active,
        ratio: state.visible_ratio,
        angle_deg: state.angle_deg,
        radius_px: state.current_radius_px,
        wing_phase: state.wing_phase,
    }
}

fn set_orbit_pose(
    layer: &CALayer,
    center: CGPoint,
    angle_deg: f64,
    radius_px: f64,
    facing_rotation_deg: f64,
    scale: f64,
) {
    let angle = angle_deg.to_radians();
    layer.setPosition(CGPoint::new(
        center.x + radius_px * angle.cos(),
        center.y - radius_px * angle.sin(),
    ));
    layer.setAffineTransform(affine_transform(
        -(angle_deg + facing_rotation_deg).to_radians(),
        scale,
        scale,
        0.0,
        0.0,
    ));
}

fn set_static_pose(layer: &CALayer, center: CGPoint, placement: &StaticPlacement) {
    layer.setPosition(CGPoint::new(
        center.x + placement.x_px,
        center.y - placement.y_px,
    ));
    let scale_x = if placement.flip_x {
        -placement.scale
    } else {
        placement.scale
    };
    layer.setAffineTransform(affine_transform(
        -placement.rotation_deg.to_radians(),
        scale_x,
        placement.scale,
        0.0,
        0.0,
    ));
}

fn apply_wing_pose(layer: &CALayer, pose: WingPose, opacity_multiplier: f64) {
    layer.setAffineTransform(affine_transform(
        -pose.rotation_deg.to_radians(),
        pose.scale_x,
        pose.scale_y,
        pose.x,
        -pose.y,
    ));
    layer.setOpacity((pose.opacity * opacity_multiplier) as f32);
}

fn affine_transform(
    rotation_radians: f64,
    scale_x: f64,
    scale_y: f64,
    translate_x: f64,
    translate_y: f64,
) -> CGAffineTransform {
    let cosine = rotation_radians.cos();
    let sine = rotation_radians.sin();
    CGAffineTransform {
        a: cosine * scale_x,
        b: sine * scale_x,
        c: -sine * scale_y,
        d: cosine * scale_y,
        tx: translate_x,
        ty: translate_y,
    }
}

fn sample_wing_pose(poses: &[WingPose; 4], phase: f64) -> WingPose {
    let wrapped = phase.rem_euclid(1.0) * poses.len() as f64;
    let index = wrapped.floor() as usize % poses.len();
    let next = (index + 1) % poses.len();
    let local = wrapped.fract();
    let eased = 0.5 - 0.5 * (PI * local).cos();
    let from = poses[index];
    let to = poses[next];
    WingPose {
        x: lerp(from.x, to.x, eased),
        y: lerp(from.y, to.y, eased),
        rotation_deg: lerp(from.rotation_deg, to.rotation_deg, eased),
        scale_x: lerp(from.scale_x, to.scale_x, eased),
        scale_y: lerp(from.scale_y, to.scale_y, eased),
        opacity: lerp(from.opacity, to.opacity, eased),
    }
}

fn lerp(from: f64, to: f64, amount: f64) -> f64 {
    from + (to - from) * amount
}

fn rate_percent(rate: f64, contract: &BeeMotionContract) -> f64 {
    let safe_rate = if rate.is_finite() { rate.max(0.0) } else { 0.0 };
    let normalized = (1.0 + safe_rate / contract.speed_log_base_rate_per_min).ln()
        / (1.0 + contract.speed_full_rate_per_min / contract.speed_log_base_rate_per_min).ln();
    normalized.clamp(0.0, 1.0)
}

fn bee_motion_ratio(speed_ratio: f64, contract: &BeeMotionContract) -> f64 {
    let safe_ratio = if speed_ratio.is_finite() {
        speed_ratio.max(0.0)
    } else {
        0.0
    };
    if safe_ratio <= 0.0 {
        0.0
    } else {
        safe_ratio.max(contract.orbit_idle_ratio).min(1.0)
    }
}

fn next_motion_ratio(
    current: f64,
    target: f64,
    frame_seconds: f64,
    contract: &BeeMotionContract,
) -> f64 {
    let safe_current = current.clamp(0.0, 1.0);
    let safe_target = target.clamp(0.0, 1.0);
    let safe_frame_seconds = frame_seconds.clamp(0.0, contract.max_frame_seconds);
    if safe_target <= 0.0 {
        let next =
            safe_current * (-safe_frame_seconds * contract.motion_idle_decay_per_second).exp();
        return if next <= contract.motion_stop_ratio {
            0.0
        } else {
            next
        };
    }

    let speed = if safe_target > safe_current {
        contract.motion_acceleration_per_second
    } else {
        contract.motion_deceleration_per_second
    };
    let smoothing = 1.0 - (-safe_frame_seconds * speed).exp();
    safe_current + (safe_target - safe_current) * smoothing
}

fn orbit_duration(ratio: f64, contract: &BeeMotionContract) -> f64 {
    let safe_ratio = ratio.clamp(0.0, 1.0);
    contract.orbit_slow_seconds
        - safe_ratio * (contract.orbit_slow_seconds - contract.orbit_fast_seconds)
}

fn orbit_radius(ratio: f64, contract: &BeeMotionContract) -> f64 {
    let intensity = ratio.clamp(0.0, 1.0).powf(contract.orbit_radius_curve);
    contract.orbit_base_radius_px
        + intensity * (contract.orbit_fast_radius_px - contract.orbit_base_radius_px)
}

fn trail_intensity(ratio: f64, contract: &BeeMotionContract) -> f64 {
    ((ratio.clamp(0.0, 1.0) - contract.trail_start_ratio) / (1.0 - contract.trail_start_ratio))
        .clamp(0.0, 1.0)
}

fn wing_duration_ms(ratio: f64, contract: &BeeMotionContract) -> f64 {
    let safe_ratio = ratio.clamp(0.0, 1.0);
    contract.wing_slow_ms - safe_ratio * (contract.wing_slow_ms - contract.wing_fast_ms)
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
    rgba_image(bytes, FRAME_WIDTH, FRAME_HEIGHT, false)
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
        CATransaction::setDisableActions(true);
        layer.setContents(Some(contents));
        CATransaction::commit();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bee_motion_ratio, box_blur_rgba, motion_contract, rate_percent, recover_pixel,
        trail_intensity, WingPose, FAR_GHOST_BLUR_SOURCE_PX, LEFT_WING_POSES,
        NEAR_GHOST_BLUR_SOURCE_PX, RIGHT_WING_POSES,
    };

    fn assert_pose(pose: WingPose, expected: [f64; 6]) {
        assert_eq!(pose.x, expected[0]);
        assert_eq!(pose.y, expected[1]);
        assert_eq!(pose.rotation_deg, expected[2]);
        assert_eq!(pose.scale_x, expected[3]);
        assert_eq!(pose.scale_y, expected[4]);
        assert_eq!(pose.opacity, expected[5]);
    }

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

    #[test]
    fn subtle_trail_pixel_keeps_low_alpha() {
        let mut output = [0_u8; 4];
        recover_pixel(&[26, 20, 0, 255], &[255, 249, 229, 255], &mut output);
        assert_eq!(output[3], 26);
        assert!(output[0] >= 250);
        assert!((190..=205).contains(&output[1]));
    }

    #[test]
    fn shared_contract_preserves_speed_and_trail_thresholds() {
        let contract = motion_contract();
        assert_eq!(rate_percent(0.0, contract), 0.0);
        assert_eq!(bee_motion_ratio(0.0, contract), 0.0);
        assert_eq!(
            rate_percent(contract.speed_full_rate_per_min, contract),
            1.0
        );
        assert_eq!(trail_intensity(contract.trail_start_ratio, contract), 0.0);
        assert_eq!(trail_intensity(1.0, contract), 1.0);
        assert!(contract.motion_render_epsilon > 0.0);
        assert_eq!(contract.static_placements.len(), contract.bee_count);
    }

    #[test]
    fn trail_blur_spreads_alpha_without_changing_dimensions() {
        let mut input = vec![0_u8; 3 * 3 * 4];
        input[(4 * 4) + 3] = 255;
        let output = box_blur_rgba(&input, 3, 3, 1);
        assert_eq!(output.len(), input.len());
        assert!(output[3] > 0);
        assert!(output[(4 * 4) + 3] < 255);
    }

    #[test]
    fn native_wing_and_trail_design_matches_web_renderer() {
        let styles = include_str!("../../src/styles.css");
        for keyframe in [
            "translate3d(-1px, -3px, 0) rotate(-18deg) scaleY(1.18) scaleX(1.08)",
            "translate3d(0, 0, 0) rotate(2deg) scaleY(0.72) scaleX(1.12)",
            "translate3d(1px, 3px, 0) rotate(16deg) scaleY(0.36) scaleX(1.2)",
            "translate3d(0, 0, 0) rotate(-4deg) scaleY(0.78) scaleX(1.1)",
            "translate3d(1px, -3px, 0) rotate(16deg) scaleY(1.14) scaleX(1.06)",
            "translate3d(0, 0, 0) rotate(-2deg) scaleY(0.7) scaleX(1.12)",
            "translate3d(-1px, 3px, 0) rotate(-15deg) scaleY(0.34) scaleX(1.2)",
            "translate3d(0, 0, 0) rotate(5deg) scaleY(0.76) scaleX(1.1)",
        ] {
            assert!(styles.contains(keyframe));
        }

        for (pose, expected) in LEFT_WING_POSES.into_iter().zip([
            [-1.0, -3.0, -18.0, 1.08, 1.18, 1.0],
            [0.0, 0.0, 2.0, 1.12, 0.72, 0.84],
            [1.0, 3.0, 16.0, 1.2, 0.36, 0.58],
            [0.0, 0.0, -4.0, 1.1, 0.78, 0.82],
        ]) {
            assert_pose(pose, expected);
        }
        for (pose, expected) in RIGHT_WING_POSES.into_iter().zip([
            [1.0, -3.0, 16.0, 1.06, 1.14, 0.95],
            [0.0, 0.0, -2.0, 1.12, 0.7, 0.8],
            [-1.0, 3.0, -15.0, 1.2, 0.34, 0.54],
            [0.0, 0.0, 5.0, 1.1, 0.76, 0.78],
        ]) {
            assert_pose(pose, expected);
        }

        let contract = motion_contract();
        let source_to_display = contract.bee_width_px / contract.bee_aspect_width;
        let near_display_blur = NEAR_GHOST_BLUR_SOURCE_PX as f64 * source_to_display;
        let far_display_blur = FAR_GHOST_BLUR_SOURCE_PX as f64 * source_to_display;
        assert!((near_display_blur - 0.42).abs() < 0.04);
        assert!((far_display_blur - 0.8).abs() < 0.04);
        assert!(styles.contains("blur(0.42px)"));
        assert!(styles.contains("blur(0.8px)"));
    }
}
