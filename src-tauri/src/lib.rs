use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::utils::config::Color;
use tauri::{Emitter, EventTarget, LogicalPosition, Manager, PhysicalPosition};
use walkdir::WalkDir;

#[cfg(all(target_os = "macos", feature = "app-store"))]
use std::sync::mpsc;
#[cfg(all(target_os = "macos", feature = "app-store"))]
use tauri::LogicalSize;

#[cfg(all(target_os = "macos", feature = "app-store"))]
mod macos_native_renderer;
#[cfg(all(target_os = "macos", feature = "app-store"))]
mod macos_store;

const SPEED_WINDOW_SECONDS: i64 = 60;
const ACTIVE_GRACE_SECONDS: i64 = 90;
const RECENT_FILE_SECONDS: u64 = 2 * 24 * 60 * 60;
const MAX_SESSION_FILES: usize = 120;
const CODEX_USAGE_ENDPOINT: &str = "https://chatgpt.com/backend-api/wham/usage";
const ACCOUNT_USAGE_TIMEOUT_SECONDS: u64 = 6;
const ACCOUNT_USAGE_CACHE_SECONDS: i64 = 8;
const ACCOUNT_USAGE_ERROR_CACHE_SECONDS: i64 = 3;
const ACCOUNT_LIMIT_SAME_CYCLE_TOLERANCE_SECONDS: u64 = 2 * 60 * 60;
const ACTIVITY_WAKE_RATE_PER_MIN: f64 = 42_000.0;
const CODEX_ACCOUNT_CACHE_FILE: &str = "codex-account-cache.json";
const CLAUDE_STATE_FILE: &str = "claude-status.json";
const CLAUDE_EVENT_MIN_RATE_SECONDS: i64 = 10;
const FIVE_HOUR_WINDOW_MINUTES: u64 = 5 * 60;
const WEEKLY_WINDOW_MINUTES: u64 = 7 * 24 * 60;

static SCAN_CACHE: OnceLock<Mutex<ScanCache>> = OnceLock::new();
static ACCOUNT_USAGE_CACHE: OnceLock<Mutex<AccountUsageCache>> = OnceLock::new();
static CONTEXT_MENU_CACHE: OnceLock<Mutex<Option<tauri::menu::Menu<tauri::Wry>>>> = OnceLock::new();
static APP_DATA_HOME: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    generated_at: i64,
    burn_rate_per_min: f64,
    animation_burn_rate_per_min: f64,
    state: MeterState,
    active_sessions: usize,
    activity_sessions: usize,
    observed_sessions: usize,
    window_seconds: i64,
    active_grace_seconds: i64,
    total_recent_tokens: u64,
    latest_total_tokens: u64,
    primary: Option<LimitWindow>,
    secondary: Option<LimitWindow>,
    reset_credits_available: Option<u64>,
    sessions: Vec<SessionSummary>,
    source: SourceStatus,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    provider: UsageProvider,
    provider_label: String,
    data_home: String,
    events_path: String,
    codex_home: String,
    sessions_dir: String,
    scanned_files: usize,
    message: String,
}

#[derive(Debug, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageProvider {
    Codex,
    Claude,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindow {
    used_percent: f64,
    remaining_percent: f64,
    window_minutes: Option<u64>,
    resets_at: Option<i64>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    id: String,
    cwd: Option<String>,
    path: String,
    last_seen: i64,
    total_tokens: u64,
    recent_tokens: u64,
    burn_rate_per_min: f64,
    active: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum MeterState {
    Waiting,
    Idle,
    Live,
    Warm,
    Hot,
    LimitNear,
    Stale,
}

#[derive(Debug, Clone)]
struct TokenEvent {
    ts: i64,
    total_tokens: u64,
    rate_limits: Option<RateLimits>,
}

#[derive(Debug, Clone)]
struct RateLimits {
    primary: Option<LimitWindow>,
    secondary: Option<LimitWindow>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AccountRateLimits {
    primary: Option<LimitWindow>,
    secondary: Option<LimitWindow>,
    reset_credits_available: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedAccountRateLimitsFile {
    cached_at: i64,
    limits: AccountRateLimits,
}

#[cfg_attr(all(target_os = "macos", feature = "app-store"), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderMode {
    Auto,
    Codex,
    Claude,
}

#[derive(Debug, Deserialize)]
struct CodexAuth {
    tokens: Option<CodexAuthTokens>,
}

#[derive(Debug, Deserialize)]
struct CodexAuthTokens {
    access_token: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CodexUsageResponse {
    rate_limit: Option<CodexRateLimit>,
    rate_limit_reset_credits: Option<CodexRateLimitResetCredits>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimit {
    primary_window: Option<CodexLimitWindow>,
    secondary_window: Option<CodexLimitWindow>,
}

#[derive(Debug, Deserialize)]
struct CodexLimitWindow {
    used_percent: Option<f64>,
    limit_window_seconds: Option<u64>,
    reset_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct CodexRateLimitResetCredits {
    available_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeBridgeState {
    sessions: Option<HashMap<String, ClaudeBridgeSession>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClaudeBridgeSession {
    id: Option<String>,
    session_name: Option<String>,
    cwd: Option<String>,
    project_dir: Option<String>,
    transcript_path: Option<String>,
    updated_at: Option<i64>,
    last_event_at: Option<i64>,
    total_observed_tokens: Option<u64>,
    rate_limits: Option<ClaudeBridgeRateLimits>,
    events: Option<Vec<ClaudeUsageEvent>>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClaudeBridgeRateLimits {
    five_hour: Option<ClaudeBridgeLimit>,
    seven_day: Option<ClaudeBridgeLimit>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClaudeBridgeLimit {
    used_percent: Option<f64>,
    resets_at: Option<i64>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClaudeUsageEvent {
    ts: i64,
    tokens: u64,
}

#[derive(Debug, Clone)]
struct SessionScan {
    id: String,
    cwd: Option<String>,
    path: PathBuf,
    events: Vec<TokenEvent>,
    last_activity_ts: i64,
}

#[derive(Debug, Default)]
struct ScanCache {
    entries: HashMap<PathBuf, CachedSession>,
}

#[derive(Debug, Default)]
struct AccountUsageCache {
    fetched_at: i64,
    result: Option<Result<AccountRateLimits, String>>,
}

#[derive(Debug, Clone)]
struct CachedSession {
    len: u64,
    processed_len: u64,
    scan: Option<SessionScan>,
}

#[tauri::command]
async fn get_usage_snapshot() -> Result<UsageSnapshot, String> {
    let snapshot = tauri::async_runtime::spawn_blocking(|| {
        collect_usage_snapshot().map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("Usage snapshot task failed: {error}"))??;

    #[cfg(all(target_os = "macos", feature = "app-store"))]
    macos_native_renderer::set_animation_burn_rate(snapshot.animation_burn_rate_per_min);

    Ok(snapshot)
}

#[tauri::command]
async fn ensure_codex_access(app: tauri::AppHandle) -> Result<bool, String> {
    request_codex_folder_access(app, false).await
}

#[tauri::command]
async fn choose_codex_folder(app: tauri::AppHandle) -> Result<bool, String> {
    request_codex_folder_access(app, true).await
}

async fn request_codex_folder_access(
    app: tauri::AppHandle,
    force_picker: bool,
) -> Result<bool, String> {
    #[cfg(all(target_os = "macos", feature = "app-store"))]
    {
        let bookmark_path = app_store_bookmark_path()?;
        if !force_picker && macos_store::activate_saved_codex_home(&bookmark_path).is_ok() {
            return Ok(true);
        }

        let suggested_directory = picker_home_dir();
        let (sender, receiver) = mpsc::channel();
        app.run_on_main_thread(move || {
            let result = macos_store::choose_codex_home(&bookmark_path, &suggested_directory);
            let _ = sender.send(result);
        })
        .map_err(|error| error.to_string())?;

        return tauri::async_runtime::spawn_blocking(move || {
            receiver
                .recv()
                .map_err(|_| "Codex folder picker closed unexpectedly.".to_string())?
                .map(|_| true)
        })
        .await
        .map_err(|error| error.to_string())?;
    }

    #[cfg(not(all(target_os = "macos", feature = "app-store")))]
    {
        let _ = (app, force_picker);
        Ok(true)
    }
}

#[tauri::command]
fn start_window_drag(window: tauri::Window) -> Result<(), String> {
    window.start_dragging().map_err(|error| error.to_string())
}

#[tauri::command]
fn refresh_window_chrome(window: tauri::WebviewWindow) -> Result<(), String> {
    apply_transparent_window_chrome(&window);
    Ok(())
}

#[tauri::command]
fn show_context_menu(window: tauri::Window, x: f64, y: f64) -> Result<(), String> {
    let app = window.app_handle();
    let reload = MenuItemBuilder::with_id("context-reload", "Reload")
        .build(app)
        .map_err(|error| error.to_string())?;
    #[cfg(all(target_os = "macos", feature = "app-store"))]
    let menu = {
        let connect = MenuItemBuilder::with_id("context-connect", "Connect Codex Folder…")
            .build(app)
            .map_err(|error| error.to_string())?;
        MenuBuilder::new(app)
            .items(&[&reload, &connect])
            .build()
            .map_err(|error| error.to_string())?
    };
    #[cfg(not(all(target_os = "macos", feature = "app-store")))]
    let menu = MenuBuilder::new(app)
        .item(&reload)
        .build()
        .map_err(|error| error.to_string())?;

    let mut context_menu = context_menu_cache()
        .lock()
        .map_err(|_| "context menu cache is poisoned".to_string())?;
    *context_menu = Some(menu);
    let Some(menu) = context_menu.as_ref() else {
        return Err("context menu was not retained".to_string());
    };

    window
        .popup_menu_at(menu, LogicalPosition::new(x.max(0.0), y.max(0.0)))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_release_channel() -> &'static str {
    if cfg!(all(target_os = "macos", feature = "app-store")) {
        "app_store"
    } else {
        "direct"
    }
}

#[tauri::command]
fn set_subscription_window_mode(window: tauri::WebviewWindow, paywall: bool) -> Result<(), String> {
    #[cfg(all(target_os = "macos", feature = "app-store"))]
    {
        if paywall {
            macos_native_renderer::set_enabled(&window, false);
            window
                .set_resizable(false)
                .map_err(|error| error.to_string())?;
            window
                .set_size(LogicalSize::new(380.0, 500.0))
                .map_err(|error| error.to_string())?;
            window.center().map_err(|error| error.to_string())?;
        } else {
            window
                .set_size(LogicalSize::new(88.0, 88.0))
                .map_err(|error| error.to_string())?;
            window
                .set_resizable(true)
                .map_err(|error| error.to_string())?;
            apply_transparent_window_chrome(&window);
            macos_native_renderer::install(&window);
            macos_native_renderer::set_enabled(&window, true);
            position_main_window(&window);
        }
    }

    #[cfg(not(all(target_os = "macos", feature = "app-store")))]
    {
        let _ = (window, paywall);
    }

    Ok(())
}

pub fn run() {
    let builder = tauri::Builder::default();
    #[cfg(all(target_os = "macos", feature = "app-store"))]
    let builder = builder
        .plugin(tauri_plugin_iap::init())
        .plugin(tauri_plugin_opener::init());

    let app = builder
        .invoke_handler(tauri::generate_handler![
            get_usage_snapshot,
            ensure_codex_access,
            choose_codex_folder,
            start_window_drag,
            refresh_window_chrome,
            show_context_menu,
            get_release_channel,
            set_subscription_window_mode
        ])
        .setup(|app| {
            configure_app_data_home(app);
            configure_app_activation(app);
            configure_main_window(app);
            app.on_menu_event(|app, event| {
                handle_context_menu_event(app, event.id().as_ref());
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Token Meter");

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if matches!(event, tauri::RunEvent::Reopen { .. }) {
            restore_main_window(app);
        }
    });
}

fn configure_app_data_home(app: &tauri::App) {
    if let Ok(path) = app.path().app_data_dir() {
        let _ = fs::create_dir_all(&path);
        let _ = APP_DATA_HOME.set(path);
    }
}

#[cfg(all(target_os = "macos", feature = "app-store"))]
fn app_store_bookmark_path() -> Result<PathBuf, String> {
    APP_DATA_HOME
        .get()
        .map(|path| path.join("codex-folder.bookmark"))
        .ok_or_else(|| "Token Meter application data folder is unavailable.".to_string())
}

#[cfg(all(target_os = "macos", feature = "app-store"))]
fn picker_home_dir() -> PathBuf {
    env::var_os("USER")
        .filter(|value| !value.is_empty())
        .map(|user| PathBuf::from("/Users").join(user))
        .unwrap_or_else(home_dir)
}

fn configure_app_activation(app: &mut tauri::App) {
    #[cfg(target_os = "macos")]
    {
        app.set_activation_policy(tauri::ActivationPolicy::Regular);
        let _ = app.handle().set_dock_visibility(true);
    }
}

fn context_menu_cache() -> &'static Mutex<Option<tauri::menu::Menu<tauri::Wry>>> {
    CONTEXT_MENU_CACHE.get_or_init(|| Mutex::new(None))
}

fn clear_context_menu_cache() {
    if let Ok(mut context_menu) = context_menu_cache().lock() {
        *context_menu = None;
    }
}

fn handle_context_menu_event(app: &tauri::AppHandle, id: &str) {
    if id == "context-connect" {
        let _ = app.emit_to(
            EventTarget::webview_window("main"),
            "context-menu-connect",
            (),
        );
        clear_context_menu_cache();
        return;
    }

    if id == "context-reload" {
        let _ = app.emit_to(
            EventTarget::webview_window("main"),
            "context-menu-reload",
            (),
        );
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.eval("window.dispatchEvent(new CustomEvent('token-meter-reload'))");
        }
        clear_context_menu_cache();
        return;
    }
}

fn configure_main_window(app: &tauri::App) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    apply_transparent_window_chrome(&window);
    position_main_window(&window);
}

#[cfg(target_os = "macos")]
fn restore_main_window(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    apply_transparent_window_chrome(&window);
    position_main_window(&window);
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn apply_transparent_window_chrome(window: &tauri::WebviewWindow) {
    let _ = window.set_background_color(Some(Color(0, 0, 0, 0)));
    let _ = window.set_shadow(false);
    #[cfg(target_os = "macos")]
    apply_macos_native_transparency(window);
}

#[cfg(target_os = "macos")]
fn apply_macos_native_transparency(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|platform_webview| {
        use objc2_app_kit::{NSColor, NSView, NSWindow};
        use objc2_web_kit::WKWebView;

        #[cfg(feature = "direct-download")]
        use objc2::{msg_send, runtime::AnyObject, sel};

        unsafe {
            let clear = NSColor::clearColor();

            let ns_window_ptr = platform_webview.ns_window();
            if !ns_window_ptr.is_null() {
                let ns_window = &*(ns_window_ptr.cast::<NSWindow>());
                ns_window.setOpaque(false);
                ns_window.setBackgroundColor(Some(&clear));
                ns_window.setHasShadow(false);
            }

            let webview_ptr = platform_webview.inner();
            if webview_ptr.is_null() {
                return;
            }

            let wk_webview = &*(webview_ptr.cast::<WKWebView>());
            wk_webview.setUnderPageBackgroundColor(Some(&clear));

            let ns_view = &*(webview_ptr.cast::<NSView>());
            ns_view.setWantsLayer(true);
            ns_view.setNeedsDisplay(true);

            #[cfg(feature = "direct-download")]
            {
                let object = &*(webview_ptr.cast::<AnyObject>());
                let responds_to_set_opaque: bool =
                    msg_send![object, respondsToSelector: sel!(setOpaque:)];
                if responds_to_set_opaque {
                    let _: () = msg_send![object, setOpaque: false];
                }

                let responds_to_set_draws_background: bool =
                    msg_send![object, respondsToSelector: sel!(setDrawsBackground:)];
                if responds_to_set_draws_background {
                    let _: () = msg_send![object, setDrawsBackground: false];
                }
            }
        }
    });
}

fn position_main_window(window: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = window.primary_monitor() else {
        return;
    };
    let Ok(window_size) = window.outer_size() else {
        return;
    };

    let area = monitor.work_area();
    let margin = (12.0 * monitor.scale_factor()).round() as i32;
    let floating_control_clearance = (96.0 * monitor.scale_factor()).round() as i32;
    let x = area.position.x + area.size.width as i32
        - window_size.width as i32
        - margin
        - floating_control_clearance;
    let y = area.position.y + area.size.height as i32 - window_size.height as i32 - margin;
    let _ = window.set_position(PhysicalPosition::new(
        x.max(area.position.x),
        y.max(area.position.y),
    ));
}

fn collect_usage_snapshot() -> Result<UsageSnapshot, Box<dyn std::error::Error>> {
    match provider_mode_from_env() {
        ProviderMode::Codex => collect_codex_usage_snapshot(),
        ProviderMode::Claude => collect_claude_usage_snapshot(),
        ProviderMode::Auto => collect_auto_usage_snapshot(),
    }
}

fn collect_auto_usage_snapshot() -> Result<UsageSnapshot, Box<dyn std::error::Error>> {
    let codex = collect_codex_usage_snapshot();
    let claude = collect_claude_usage_snapshot();

    match (codex, claude) {
        (Ok(codex), Ok(claude)) => {
            if provider_score(&claude) > provider_score(&codex) {
                Ok(claude)
            } else {
                Ok(codex)
            }
        }
        (Ok(codex), Err(_)) => Ok(codex),
        (Err(_), Ok(claude)) => Ok(claude),
        (Err(codex_error), Err(claude_error)) => Err(format!(
            "Codex provider failed: {codex_error}; Claude provider failed: {claude_error}"
        )
        .into()),
    }
}

fn provider_score(snapshot: &UsageSnapshot) -> (u8, i64, usize) {
    let latest = snapshot
        .sessions
        .iter()
        .map(|session| session.last_seen)
        .max()
        .unwrap_or(0);

    let tier = if snapshot.animation_burn_rate_per_min > 0.0 {
        4
    } else if snapshot.active_sessions > 0 {
        3
    } else if snapshot.primary.is_some() || snapshot.secondary.is_some() {
        2
    } else if snapshot.observed_sessions > 0 {
        1
    } else {
        0
    };

    (tier, latest, snapshot.observed_sessions)
}

fn provider_mode_from_env() -> ProviderMode {
    #[cfg(all(target_os = "macos", feature = "app-store"))]
    {
        ProviderMode::Codex
    }

    #[cfg(not(all(target_os = "macos", feature = "app-store")))]
    {
        [
            "USAGE_METER_PROVIDER",
            "CODEX_USAGE_PROVIDER",
            "TOKEN_USAGE_PROVIDER",
        ]
        .iter()
        .find_map(|key| env::var(key).ok())
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "codex" => ProviderMode::Codex,
            "claude" | "claude-code" | "claude_code" => ProviderMode::Claude,
            _ => ProviderMode::Auto,
        })
        .unwrap_or(ProviderMode::Auto)
    }
}

fn collect_codex_usage_snapshot() -> Result<UsageSnapshot, Box<dyn std::error::Error>> {
    let now = Utc::now().timestamp();
    #[cfg(all(target_os = "macos", feature = "app-store"))]
    let codex_home = macos_store::active_codex_home()
        .ok_or("Select the .codex folder to connect Token Meter to Codex.")?;
    #[cfg(not(all(target_os = "macos", feature = "app-store")))]
    let codex_home = codex_home();
    let sessions_dir = codex_home.join("sessions");
    let account_limits = account_rate_limits_cached(&codex_home, now);

    if !sessions_dir.exists() {
        let (primary, secondary, reset_credits_available, message) =
            account_limits_for_source(account_limits, "No Codex sessions directory found yet.");
        return Ok(empty_snapshot(
            now,
            codex_home,
            sessions_dir,
            primary,
            secondary,
            reset_credits_available,
            message,
        ));
    }

    let files = recent_session_files(&sessions_dir)?;
    let scanned_files = files.len();
    prune_scan_cache(&files);
    let mut sessions = Vec::new();

    for path in files {
        if let Ok(Some(scan)) = scan_session_file_cached(&path) {
            sessions.push(scan);
        }
    }

    if sessions.is_empty() {
        let (primary, secondary, reset_credits_available, message) =
            account_limits_for_source(account_limits, "Waiting for Codex token_count events.");
        return Ok(empty_snapshot(
            now,
            codex_home,
            sessions_dir,
            primary,
            secondary,
            reset_credits_available,
            message,
        ));
    }

    let window_start = now - SPEED_WINDOW_SECONDS;
    let active_cutoff = now - ACTIVE_GRACE_SECONDS;
    let mut primary = None;
    let mut secondary = None;
    let mut latest_total_tokens = 0_u64;
    let mut summaries = Vec::new();
    let mut total_recent_tokens = 0_u64;
    let mut animation_burn_rate_per_min = 0.0;
    let mut activity_sessions = 0_usize;

    for scan in sessions {
        for event in &scan.events {
            if let Some(rate_limits) = &event.rate_limits {
                primary = stabilize_limit_window(rate_limits.primary.clone(), primary, now);
                secondary = stabilize_limit_window(rate_limits.secondary.clone(), secondary, now);
            }
        }

        let latest = scan.events.last().cloned();
        let activity_active = scan.last_activity_ts >= active_cutoff;
        let (recent_tokens, rate) = recent_delta_and_rate(&scan.events, window_start, now);
        let token_active = latest
            .as_ref()
            .is_some_and(|latest| latest.ts >= active_cutoff && recent_tokens > 0);
        let animation_rate = animation_rate_for_session(rate, token_active, activity_active);

        if animation_rate > 0.0 {
            activity_sessions += 1;
            animation_burn_rate_per_min += animation_rate;
        }

        let Some(latest) = latest else {
            if activity_active {
                summaries.push(SessionSummary {
                    id: scan.id,
                    cwd: scan.cwd,
                    path: scan.path.display().to_string(),
                    last_seen: scan.last_activity_ts,
                    total_tokens: 0,
                    recent_tokens: 0,
                    burn_rate_per_min: 0.0,
                    active: false,
                });
            }

            continue;
        };

        latest_total_tokens = latest_total_tokens.saturating_add(latest.total_tokens);

        if token_active {
            total_recent_tokens = total_recent_tokens.saturating_add(recent_tokens);
        }

        summaries.push(SessionSummary {
            id: scan.id,
            cwd: scan.cwd,
            path: scan.path.display().to_string(),
            last_seen: latest.ts.max(scan.last_activity_ts),
            total_tokens: latest.total_tokens,
            recent_tokens,
            burn_rate_per_min: if token_active { rate } else { 0.0 },
            active: token_active,
        });
    }

    summaries.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.recent_tokens.cmp(&a.recent_tokens))
            .then(b.last_seen.cmp(&a.last_seen))
    });

    let active_sessions = summaries.iter().filter(|session| session.active).count();
    let burn_rate_per_min = summaries
        .iter()
        .filter(|session| session.active)
        .map(|session| session.burn_rate_per_min)
        .sum::<f64>();
    let (primary, secondary, reset_credits_available, source_message) =
        merge_account_and_session_limits(account_limits, primary, secondary, now);

    let state = derive_state(
        burn_rate_per_min,
        active_sessions.max(activity_sessions),
        primary.as_ref(),
        secondary.as_ref(),
        now,
    );

    Ok(UsageSnapshot {
        generated_at: now,
        burn_rate_per_min,
        animation_burn_rate_per_min,
        state,
        active_sessions,
        activity_sessions,
        observed_sessions: summaries.len(),
        window_seconds: SPEED_WINDOW_SECONDS,
        active_grace_seconds: ACTIVE_GRACE_SECONDS,
        total_recent_tokens,
        latest_total_tokens,
        primary,
        secondary,
        reset_credits_available,
        sessions: summaries.into_iter().take(8).collect(),
        source: codex_source_status(codex_home, sessions_dir, scanned_files, source_message),
    })
}

fn empty_snapshot(
    now: i64,
    codex_home: PathBuf,
    sessions_dir: PathBuf,
    primary: Option<LimitWindow>,
    secondary: Option<LimitWindow>,
    reset_credits_available: Option<u64>,
    message: String,
) -> UsageSnapshot {
    let state = derive_state(0.0, 0, primary.as_ref(), secondary.as_ref(), now);
    UsageSnapshot {
        generated_at: now,
        burn_rate_per_min: 0.0,
        animation_burn_rate_per_min: 0.0,
        state,
        active_sessions: 0,
        activity_sessions: 0,
        observed_sessions: 0,
        window_seconds: SPEED_WINDOW_SECONDS,
        active_grace_seconds: ACTIVE_GRACE_SECONDS,
        total_recent_tokens: 0,
        latest_total_tokens: 0,
        primary,
        secondary,
        reset_credits_available,
        sessions: Vec::new(),
        source: codex_source_status(codex_home, sessions_dir, 0, message),
    }
}

fn codex_source_status(
    codex_home: PathBuf,
    sessions_dir: PathBuf,
    scanned_files: usize,
    message: String,
) -> SourceStatus {
    SourceStatus {
        provider: UsageProvider::Codex,
        provider_label: "Codex".to_string(),
        data_home: codex_home.display().to_string(),
        events_path: sessions_dir.display().to_string(),
        codex_home: codex_home.display().to_string(),
        sessions_dir: sessions_dir.display().to_string(),
        scanned_files,
        message,
    }
}

fn collect_claude_usage_snapshot() -> Result<UsageSnapshot, Box<dyn std::error::Error>> {
    let now = Utc::now().timestamp();
    let data_home = usage_meter_home();
    let state_path = claude_state_path(&data_home);

    if !state_path.exists() {
        return Ok(empty_snapshot_from_source(
            now,
            None,
            None,
            None,
            claude_source_status(
                data_home,
                state_path,
                0,
                "Claude bridge is not installed yet. Run `npm run install:claude` once."
                    .to_string(),
            ),
        ));
    }

    let file = File::open(&state_path)?;
    let state: ClaudeBridgeState = serde_json::from_reader(file)?;
    let sessions = state.sessions.unwrap_or_default();

    if sessions.is_empty() {
        return Ok(empty_snapshot_from_source(
            now,
            None,
            None,
            None,
            claude_source_status(
                data_home,
                state_path,
                1,
                "Claude bridge is installed; waiting for Claude Code statusLine data.".to_string(),
            ),
        ));
    }

    let window_start = now - SPEED_WINDOW_SECONDS;
    let active_cutoff = now - ACTIVE_GRACE_SECONDS;
    let mut summaries = Vec::new();
    let mut total_recent_tokens = 0_u64;
    let mut latest_total_tokens = 0_u64;
    let mut animation_burn_rate_per_min = 0.0;
    let mut activity_sessions = 0_usize;
    let mut latest_limits: Option<(i64, ClaudeBridgeRateLimits)> = None;

    for (key, session) in sessions {
        let events = session.events.clone().unwrap_or_default();
        let (recent_tokens, rate) = recent_event_tokens_and_rate(&events, window_start, now);
        let last_event_at = session
            .last_event_at
            .or_else(|| events.iter().map(|event| event.ts).max())
            .unwrap_or(0);
        let updated_at = session.updated_at.unwrap_or(last_event_at);
        let last_seen = updated_at.max(last_event_at);

        if let Some(rate_limits) = session.rate_limits.clone() {
            if latest_limits
                .as_ref()
                .is_none_or(|(latest_ts, _)| last_seen >= *latest_ts)
            {
                latest_limits = Some((last_seen, rate_limits));
            }
        }

        let active = last_event_at >= active_cutoff && recent_tokens > 0;
        let animation_rate = animation_rate_for_session(rate, active, active);
        if animation_rate > 0.0 {
            activity_sessions += 1;
            animation_burn_rate_per_min += animation_rate;
        }
        if active {
            total_recent_tokens = total_recent_tokens.saturating_add(recent_tokens);
        }

        let total_tokens = session
            .total_observed_tokens
            .unwrap_or_else(|| events.iter().map(|event| event.tokens).sum());
        latest_total_tokens = latest_total_tokens.saturating_add(total_tokens);

        summaries.push(SessionSummary {
            id: session.id.filter(|id| !id.trim().is_empty()).unwrap_or(key),
            cwd: session.cwd.or(session.project_dir).or(session.session_name),
            path: session
                .transcript_path
                .unwrap_or_else(|| state_path.display().to_string()),
            last_seen,
            total_tokens,
            recent_tokens,
            burn_rate_per_min: if active { rate } else { 0.0 },
            active,
        });
    }

    summaries.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.recent_tokens.cmp(&a.recent_tokens))
            .then(b.last_seen.cmp(&a.last_seen))
    });

    let active_sessions = summaries.iter().filter(|session| session.active).count();
    let burn_rate_per_min = summaries
        .iter()
        .filter(|session| session.active)
        .map(|session| session.burn_rate_per_min)
        .sum::<f64>();
    let (primary, secondary) = latest_limits
        .map(|(_, limits)| {
            (
                limit_from_claude_window(limits.five_hour, FIVE_HOUR_WINDOW_MINUTES),
                limit_from_claude_window(limits.seven_day, WEEKLY_WINDOW_MINUTES),
            )
        })
        .unwrap_or((None, None));
    let state = derive_state(
        burn_rate_per_min,
        active_sessions,
        primary.as_ref(),
        secondary.as_ref(),
        now,
    );

    Ok(UsageSnapshot {
        generated_at: now,
        burn_rate_per_min,
        animation_burn_rate_per_min,
        state,
        active_sessions,
        activity_sessions,
        observed_sessions: summaries.len(),
        window_seconds: SPEED_WINDOW_SECONDS,
        active_grace_seconds: ACTIVE_GRACE_SECONDS,
        total_recent_tokens,
        latest_total_tokens,
        primary,
        secondary,
        reset_credits_available: None,
        sessions: summaries.into_iter().take(8).collect(),
        source: claude_source_status(
            data_home,
            state_path,
            1,
            "Reading Claude Code quota and token events from the local statusLine bridge."
                .to_string(),
        ),
    })
}

fn empty_snapshot_from_source(
    now: i64,
    primary: Option<LimitWindow>,
    secondary: Option<LimitWindow>,
    reset_credits_available: Option<u64>,
    source: SourceStatus,
) -> UsageSnapshot {
    let state = derive_state(0.0, 0, primary.as_ref(), secondary.as_ref(), now);
    UsageSnapshot {
        generated_at: now,
        burn_rate_per_min: 0.0,
        animation_burn_rate_per_min: 0.0,
        state,
        active_sessions: 0,
        activity_sessions: 0,
        observed_sessions: 0,
        window_seconds: SPEED_WINDOW_SECONDS,
        active_grace_seconds: ACTIVE_GRACE_SECONDS,
        total_recent_tokens: 0,
        latest_total_tokens: 0,
        primary,
        secondary,
        reset_credits_available,
        sessions: Vec::new(),
        source,
    }
}

fn claude_source_status(
    data_home: PathBuf,
    state_path: PathBuf,
    scanned_files: usize,
    message: String,
) -> SourceStatus {
    SourceStatus {
        provider: UsageProvider::Claude,
        provider_label: "Claude Code".to_string(),
        data_home: data_home.display().to_string(),
        events_path: state_path.display().to_string(),
        codex_home: String::new(),
        sessions_dir: state_path.display().to_string(),
        scanned_files,
        message,
    }
}

fn usage_meter_home() -> PathBuf {
    #[cfg(all(target_os = "macos", feature = "app-store"))]
    if let Some(path) = APP_DATA_HOME.get() {
        return path.clone();
    }

    env::var("TOKEN_METER_HOME")
        .or_else(|_| env::var("CODEX_USAGE_METER_HOME"))
        .or_else(|_| env::var("USAGE_METER_HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let next = home_dir().join(".token-meter");
            let legacy = home_dir().join(".codex-usage-meter");
            if legacy.exists() && !next.exists() {
                legacy
            } else {
                next
            }
        })
}

fn claude_state_path(data_home: &Path) -> PathBuf {
    env::var("CLAUDE_TOKEN_METER_STATE")
        .or_else(|_| env::var("CLAUDE_USAGE_METER_STATE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| data_home.join(CLAUDE_STATE_FILE))
}

fn recent_event_tokens_and_rate(
    events: &[ClaudeUsageEvent],
    window_start: i64,
    now: i64,
) -> (u64, f64) {
    let recent: Vec<&ClaudeUsageEvent> = events
        .iter()
        .filter(|event| event.ts >= window_start && event.ts <= now)
        .collect();
    let tokens = recent
        .iter()
        .fold(0_u64, |sum, event| sum.saturating_add(event.tokens));
    if tokens == 0 {
        return (0, 0.0);
    }

    let first_ts = recent
        .iter()
        .map(|event| event.ts)
        .min()
        .unwrap_or(window_start);
    let elapsed = (now - first_ts)
        .max(CLAUDE_EVENT_MIN_RATE_SECONDS)
        .min(SPEED_WINDOW_SECONDS) as f64;
    (tokens, tokens as f64 / elapsed * 60.0)
}

fn limit_from_claude_window(
    window: Option<ClaudeBridgeLimit>,
    window_minutes: u64,
) -> Option<LimitWindow> {
    let window = window?;
    let used_percent = window.used_percent?.clamp(0.0, 100.0);
    Some(LimitWindow {
        used_percent,
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        window_minutes: Some(window_minutes),
        resets_at: window.resets_at,
    })
}

fn account_limits_for_source(
    account_limits: Result<AccountRateLimits, String>,
    fallback_message: &str,
) -> (
    Option<LimitWindow>,
    Option<LimitWindow>,
    Option<u64>,
    String,
) {
    match account_limits {
        Ok(limits) => (
            limits.primary,
            limits.secondary,
            limits.reset_credits_available,
            "Reading Codex account quota from /wham/usage; waiting for local token speed events."
                .to_string(),
        ),
        Err(error) => (
            None,
            None,
            None,
            format!("{fallback_message} Account quota unavailable: {error}"),
        ),
    }
}

fn merge_account_and_session_limits(
    account_limits: Result<AccountRateLimits, String>,
    _session_primary: Option<LimitWindow>,
    _session_secondary: Option<LimitWindow>,
    _now: i64,
) -> (
    Option<LimitWindow>,
    Option<LimitWindow>,
    Option<u64>,
    String,
) {
    match account_limits {
        Ok(limits) => (
            limits.primary,
            limits.secondary,
            limits.reset_credits_available,
            "Reading Codex account quota from /wham/usage; token speed from local session events."
                .to_string(),
        ),
        Err(error) => (
            None,
            None,
            None,
            format!(
                "Codex account quota unavailable; keeping token speed local. /wham/usage unavailable: {error}"
            ),
        ),
    }
}

fn account_rate_limits_cached(codex_home: &Path, now: i64) -> Result<AccountRateLimits, String> {
    let mut previous_success = None;

    if let Ok(cache) = ACCOUNT_USAGE_CACHE
        .get_or_init(|| Mutex::new(AccountUsageCache::default()))
        .lock()
    {
        if let Some(result) = &cache.result {
            if let Ok(limits) = result {
                previous_success = Some(limits.clone());
            }

            let ttl = if result.is_ok() {
                ACCOUNT_USAGE_CACHE_SECONDS
            } else {
                ACCOUNT_USAGE_ERROR_CACHE_SECONDS
            };
            if now - cache.fetched_at < ttl {
                return result.clone();
            }
        }
    }

    let cached_success = read_cached_account_rate_limits();
    let result = match fetch_account_rate_limits(codex_home) {
        Ok(limits) => {
            let limits = stabilize_account_limits(limits, cached_success, now);
            if has_account_windows(&limits) {
                let _ = write_cached_account_rate_limits(&limits, now);
            }
            Ok(limits)
        }
        Err(error) => previous_success
            .or(cached_success)
            .ok_or_else(|| error.to_string()),
    };

    if let Ok(mut cache) = ACCOUNT_USAGE_CACHE
        .get_or_init(|| Mutex::new(AccountUsageCache::default()))
        .lock()
    {
        cache.fetched_at = now;
        cache.result = Some(result.clone());
    }

    result
}

fn stabilize_account_limits(
    mut current: AccountRateLimits,
    fallback: Option<AccountRateLimits>,
    now: i64,
) -> AccountRateLimits {
    if let Some(fallback) = fallback {
        if !current
            .primary
            .as_ref()
            .is_some_and(is_unlimited_five_hour_window)
        {
            current.primary = stabilize_limit_window(current.primary, fallback.primary, now);
        }
        current.secondary = stabilize_limit_window(current.secondary, fallback.secondary, now);
        if current.reset_credits_available.is_none() {
            current.reset_credits_available = fallback.reset_credits_available;
        }
    }

    current
}

fn stabilize_limit_window(
    current: Option<LimitWindow>,
    previous: Option<LimitWindow>,
    now: i64,
) -> Option<LimitWindow> {
    match (current, previous) {
        (None, previous) => previous,
        (current, None) => current,
        (Some(current), Some(previous)) => {
            let current_cycle_active = current.resets_at.map_or(true, |resets_at| resets_at > now);
            let previous_cycle_active =
                previous.resets_at.map_or(true, |resets_at| resets_at > now);

            if current_cycle_active != previous_cycle_active {
                return if current_cycle_active {
                    Some(current)
                } else {
                    Some(previous)
                };
            }

            if !current_cycle_active && !previous_cycle_active {
                return if current.resets_at > previous.resets_at {
                    Some(current)
                } else {
                    Some(previous)
                };
            }

            let same_window = current.window_minutes.is_none()
                || previous.window_minutes.is_none()
                || current.window_minutes == previous.window_minutes;
            let same_cycle = match (current.resets_at, previous.resets_at) {
                (Some(current_reset), Some(previous_reset)) => {
                    current_reset.abs_diff(previous_reset)
                        <= ACCOUNT_LIMIT_SAME_CYCLE_TOLERANCE_SECONDS
                }
                (None, None) => true,
                _ => false,
            };

            if same_window && !same_cycle {
                return match (current.resets_at, previous.resets_at) {
                    (Some(current_reset), Some(previous_reset))
                        if current_reset > previous_reset =>
                    {
                        Some(current)
                    }
                    (Some(_), None) => Some(current),
                    _ => Some(previous),
                };
            }

            let usage_went_backwards = current.used_percent < previous.used_percent;

            if same_window && same_cycle && previous_cycle_active && usage_went_backwards {
                Some(previous)
            } else {
                Some(current)
            }
        }
    }
}

fn has_account_windows(limits: &AccountRateLimits) -> bool {
    limits.primary.is_some() || limits.secondary.is_some()
}

fn codex_account_cache_path() -> PathBuf {
    usage_meter_home().join(CODEX_ACCOUNT_CACHE_FILE)
}

fn read_cached_account_rate_limits() -> Option<AccountRateLimits> {
    let file = File::open(codex_account_cache_path()).ok()?;
    serde_json::from_reader::<_, CachedAccountRateLimitsFile>(file)
        .ok()
        .map(|cache| cache.limits)
        .filter(has_account_windows)
}

fn write_cached_account_rate_limits(
    limits: &AccountRateLimits,
    now: i64,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = codex_account_cache_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let cache = CachedAccountRateLimitsFile {
        cached_at: now,
        limits: limits.clone(),
    };
    fs::write(path, serde_json::to_vec_pretty(&cache)?)?;
    Ok(())
}

fn fetch_account_rate_limits(
    codex_home: &Path,
) -> Result<AccountRateLimits, Box<dyn std::error::Error>> {
    let auth_path = codex_home.join("auth.json");
    let auth_file = File::open(&auth_path)?;
    let auth: CodexAuth = serde_json::from_reader(auth_file)?;
    let tokens = auth.tokens.ok_or("Codex auth tokens are missing")?;
    let access_token = tokens
        .access_token
        .filter(|token| !token.trim().is_empty())
        .ok_or("Codex access token is missing")?;

    let client = Client::builder()
        .timeout(Duration::from_secs(ACCOUNT_USAGE_TIMEOUT_SECONDS))
        .build()?;
    let mut request = client
        .get(CODEX_USAGE_ENDPOINT)
        .bearer_auth(access_token)
        .header("originator", "Codex Desktop")
        .header("User-Agent", "Codex Desktop/usage-meter")
        .header("OAI-Language", "en");

    if let Some(account_id) = tokens
        .account_id
        .filter(|account_id| !account_id.trim().is_empty())
    {
        request = request.header("ChatGPT-Account-Id", account_id);
    }

    let response = request.send()?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("Codex account usage request failed with HTTP {status}").into());
    }

    let usage: CodexUsageResponse = response.json()?;
    Ok(account_limits_from_usage(usage))
}

fn account_limits_from_usage(usage: CodexUsageResponse) -> AccountRateLimits {
    let (primary, secondary) = usage
        .rate_limit
        .map(account_windows_by_duration)
        .unwrap_or((None, None));

    AccountRateLimits {
        primary,
        secondary,
        reset_credits_available: usage
            .rate_limit_reset_credits
            .and_then(|credits| credits.available_count),
    }
}

fn account_windows_by_duration(
    rate_limit: CodexRateLimit,
) -> (Option<LimitWindow>, Option<LimitWindow>) {
    let mut primary = None;
    let mut secondary = None;

    for (index, window) in [rate_limit.primary_window, rate_limit.secondary_window]
        .into_iter()
        .enumerate()
    {
        let Some(window) = limit_from_account_window(window.as_ref()) else {
            continue;
        };

        match window.window_minutes {
            Some(FIVE_HOUR_WINDOW_MINUTES) => primary = Some(window),
            Some(WEEKLY_WINDOW_MINUTES) => secondary = Some(window),
            _ if index == 0 => primary = Some(window),
            _ => secondary = Some(window),
        }
    }

    if primary.is_none() && secondary.is_some() {
        primary = Some(unlimited_five_hour_window());
    }

    (primary, secondary)
}

fn unlimited_five_hour_window() -> LimitWindow {
    LimitWindow {
        used_percent: 0.0,
        remaining_percent: 100.0,
        window_minutes: Some(FIVE_HOUR_WINDOW_MINUTES),
        resets_at: None,
    }
}

fn is_unlimited_five_hour_window(window: &LimitWindow) -> bool {
    window.window_minutes == Some(FIVE_HOUR_WINDOW_MINUTES)
        && window.resets_at.is_none()
        && window.used_percent == 0.0
        && window.remaining_percent == 100.0
}

fn limit_from_account_window(window: Option<&CodexLimitWindow>) -> Option<LimitWindow> {
    let window = window?;
    let used_percent = window.used_percent?.clamp(0.0, 100.0);
    Some(LimitWindow {
        used_percent,
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        window_minutes: window.limit_window_seconds.map(|seconds| seconds / 60),
        resets_at: window.reset_at,
    })
}

#[cfg(not(all(target_os = "macos", feature = "app-store")))]
fn codex_home() -> PathBuf {
    env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home_dir().join(".codex"))
}

fn home_dir() -> PathBuf {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        })
        .or_else(|| {
            let drive = env::var_os("HOMEDRIVE")?;
            let path = env::var_os("HOMEPATH")?;
            Some(PathBuf::from(format!(
                "{}{}",
                drive.to_string_lossy(),
                path.to_string_lossy()
            )))
        })
        .unwrap_or_else(|| PathBuf::from("."))
}

fn recent_session_files(sessions_dir: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let now = SystemTime::now();
    let mut files: Vec<(SystemTime, PathBuf)> = Vec::new();

    for entry in WalkDir::new(sessions_dir).follow_links(false) {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }

        let metadata = fs::metadata(path)?;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        if age.as_secs() <= RECENT_FILE_SECONDS {
            files.push((modified, path.to_path_buf()));
        }
    }

    files.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(files
        .into_iter()
        .take(MAX_SESSION_FILES)
        .map(|(_, path)| path)
        .collect())
}

fn scan_session_file(path: &Path) -> Result<Option<SessionScan>, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut scan = empty_session_scan(path);

    for line in reader.lines() {
        apply_session_line(&mut scan, &line?);
    }

    Ok(finalize_session_scan(scan))
}

fn empty_session_scan(path: &Path) -> SessionScan {
    SessionScan {
        id: path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("unknown-session")
            .to_string(),
        cwd: None,
        path: path.to_path_buf(),
        events: Vec::new(),
        last_activity_ts: 0,
    }
}

fn apply_session_line(scan: &mut SessionScan, line: &str) {
    let fast_ts = timestamp_from_line(line);
    if let Some(ts) = fast_ts {
        scan.last_activity_ts = scan.last_activity_ts.max(ts);
    }

    if !(line.contains("session_meta") || line.contains("\"token_count\"")) {
        return;
    }

    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };

    let event_ts = fast_ts.or_else(|| timestamp_from_value(&value));
    if let Some(ts) = event_ts {
        scan.last_activity_ts = scan.last_activity_ts.max(ts);
    }

    match value.get("type").and_then(Value::as_str) {
        Some("session_meta") => {
            if let Some(payload) = value.get("payload") {
                if let Some(payload_id) = payload
                    .get("id")
                    .or_else(|| payload.get("session_id"))
                    .and_then(Value::as_str)
                {
                    scan.id = payload_id.to_string();
                }
                scan.cwd = payload
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(ToString::to_string);
            }
        }
        Some("event_msg") => {
            let Some(payload) = value.get("payload") else {
                return;
            };
            if payload.get("type").and_then(Value::as_str) != Some("token_count") {
                return;
            }
            let Some(ts) = event_ts else {
                return;
            };
            let total_tokens = payload
                .pointer("/info/total_token_usage/total_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0);

            scan.events.push(TokenEvent {
                ts,
                total_tokens,
                rate_limits: parse_rate_limits(payload.get("rate_limits")),
            });
        }
        _ => {}
    }
}

fn finalize_session_scan(mut scan: SessionScan) -> Option<SessionScan> {
    if scan.events.is_empty() && scan.last_activity_ts == 0 {
        return None;
    }

    scan.events.sort_by(|a, b| a.ts.cmp(&b.ts));
    Some(scan)
}

fn timestamp_from_line(line: &str) -> Option<i64> {
    let marker = "\"timestamp\":\"";
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    timestamp_to_unix(&rest[..end])
}

fn timestamp_from_value(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(timestamp_to_unix)
}

fn timestamp_to_unix(timestamp: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc).timestamp())
}

fn scan_session_file_cached(
    path: &Path,
) -> Result<Option<SessionScan>, Box<dyn std::error::Error>> {
    let metadata = fs::metadata(path)?;
    let len = metadata.len();
    let key = path.to_path_buf();

    if let Some(cached) = SCAN_CACHE
        .get_or_init(|| Mutex::new(ScanCache::default()))
        .lock()
        .ok()
        .and_then(|cache| cache.entries.get(&key).cloned())
    {
        if cached.len == len {
            return Ok(cached.scan);
        }

        if len > cached.processed_len {
            let scan = scan_session_file_append(path, &cached, len)?;
            cache_session_scan(key, len, scan.processed_len, scan.scan.clone());
            return Ok(scan.scan);
        }
    }

    let scan = scan_session_file(path)?;
    let processed_len = len;

    cache_session_scan(key, len, processed_len, scan.clone());

    Ok(scan)
}

struct IncrementalSessionScan {
    scan: Option<SessionScan>,
    processed_len: u64,
}

fn scan_session_file_append(
    path: &Path,
    cached: &CachedSession,
    len: u64,
) -> Result<IncrementalSessionScan, Box<dyn std::error::Error>> {
    if len < cached.processed_len {
        return Ok(IncrementalSessionScan {
            scan: scan_session_file(path)?,
            processed_len: len,
        });
    }

    let mut scan = cached
        .scan
        .clone()
        .unwrap_or_else(|| empty_session_scan(path));
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(cached.processed_len))?;
    let mut appended = String::new();
    file.read_to_string(&mut appended)?;

    let complete_len = appended.rfind('\n').map(|index| index + 1).unwrap_or(0);
    let complete_append = &appended[..complete_len];
    for line in complete_append.lines() {
        apply_session_line(&mut scan, line);
    }

    Ok(IncrementalSessionScan {
        scan: finalize_session_scan(scan),
        processed_len: cached.processed_len + complete_len as u64,
    })
}

fn cache_session_scan(key: PathBuf, len: u64, processed_len: u64, scan: Option<SessionScan>) {
    if let Ok(mut cache) = SCAN_CACHE
        .get_or_init(|| Mutex::new(ScanCache::default()))
        .lock()
    {
        cache.entries.insert(
            key,
            CachedSession {
                len,
                processed_len,
                scan,
            },
        );
    }
}

fn prune_scan_cache(files: &[PathBuf]) {
    let keep: HashSet<PathBuf> = files.iter().cloned().collect();
    if let Ok(mut cache) = SCAN_CACHE
        .get_or_init(|| Mutex::new(ScanCache::default()))
        .lock()
    {
        cache.entries.retain(|path, _| keep.contains(path));
    }
}

fn parse_rate_limits(value: Option<&Value>) -> Option<RateLimits> {
    let value = value?;
    if value
        .get("limit_id")
        .and_then(Value::as_str)
        .is_some_and(|limit_id| limit_id != "codex")
    {
        return None;
    }

    Some(RateLimits {
        primary: parse_limit_window(value.get("primary")),
        secondary: parse_limit_window(value.get("secondary")),
    })
}

fn parse_limit_window(value: Option<&Value>) -> Option<LimitWindow> {
    let value = value?;
    let used_percent = value.get("used_percent")?.as_f64()?;
    let used_percent = used_percent.clamp(0.0, 100.0);
    Some(LimitWindow {
        used_percent,
        remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
        window_minutes: value.get("window_minutes").and_then(Value::as_u64),
        resets_at: value.get("resets_at").and_then(Value::as_i64),
    })
}

fn recent_delta_and_rate(events: &[TokenEvent], window_start: i64, now: i64) -> (u64, f64) {
    if events.len() < 2 {
        return (0, 0.0);
    }

    let mut baseline = events[0].clone();
    let mut latest = events.last().cloned().unwrap_or_else(|| baseline.clone());

    for event in events {
        if event.ts <= window_start {
            baseline = event.clone();
        }
        if event.ts >= window_start {
            latest = event.clone();
        }
    }

    if latest.ts < window_start || latest.total_tokens <= baseline.total_tokens {
        return (0, 0.0);
    }

    let delta = latest.total_tokens.saturating_sub(baseline.total_tokens);
    let elapsed = (now.max(latest.ts) - baseline.ts)
        .max(1)
        .min(SPEED_WINDOW_SECONDS) as f64;
    let rate = delta as f64 / elapsed * 60.0;
    (delta, rate)
}

fn animation_rate_for_session(token_rate: f64, token_active: bool, activity_active: bool) -> f64 {
    if token_active {
        token_rate
    } else if activity_active {
        ACTIVITY_WAKE_RATE_PER_MIN
    } else {
        0.0
    }
}

fn derive_state(
    burn_rate_per_min: f64,
    active_sessions: usize,
    primary: Option<&LimitWindow>,
    secondary: Option<&LimitWindow>,
    now: i64,
) -> MeterState {
    let limit_near = primary
        .or(secondary)
        .is_some_and(|limit| limit.remaining_percent <= 15.0);

    if limit_near {
        return MeterState::LimitNear;
    }

    let stale = primary
        .or(secondary)
        .and_then(|limit| limit.resets_at)
        .is_some_and(|reset| reset < now);
    if stale {
        return MeterState::Stale;
    }

    if active_sessions == 0 {
        return MeterState::Idle;
    }

    match burn_rate_per_min {
        rate if rate >= 180_000.0 => MeterState::Hot,
        rate if rate >= 60_000.0 => MeterState::Warm,
        _ => MeterState::Live,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_rate(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 0.000_001,
            "expected rate {expected}, got {actual}"
        );
    }

    fn event(ts: i64, total_tokens: u64) -> TokenEvent {
        TokenEvent {
            ts,
            total_tokens,
            rate_limits: None,
        }
    }

    fn limit(used_percent: f64) -> LimitWindow {
        LimitWindow {
            used_percent,
            remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
            window_minutes: Some(300),
            resets_at: None,
        }
    }

    fn limit_with_reset(used_percent: f64, window_minutes: u64, resets_at: i64) -> LimitWindow {
        LimitWindow {
            used_percent,
            remaining_percent: (100.0 - used_percent).clamp(0.0, 100.0),
            window_minutes: Some(window_minutes),
            resets_at: Some(resets_at),
        }
    }

    #[test]
    fn account_usage_windows_map_to_quota_limits() {
        let limits = account_limits_from_usage(CodexUsageResponse {
            rate_limit: Some(CodexRateLimit {
                primary_window: Some(CodexLimitWindow {
                    used_percent: Some(20.0),
                    limit_window_seconds: Some(18_000),
                    reset_at: Some(1_782_568_538),
                }),
                secondary_window: Some(CodexLimitWindow {
                    used_percent: Some(13.0),
                    limit_window_seconds: Some(604_800),
                    reset_at: Some(1_782_978_288),
                }),
            }),
            rate_limit_reset_credits: Some(CodexRateLimitResetCredits {
                available_count: Some(2),
            }),
        });

        let primary = limits.primary.expect("primary quota");
        let secondary = limits.secondary.expect("secondary quota");

        assert_eq!(primary.used_percent, 20.0);
        assert_eq!(primary.remaining_percent, 80.0);
        assert_eq!(primary.window_minutes, Some(300));
        assert_eq!(primary.resets_at, Some(1_782_568_538));
        assert_eq!(secondary.remaining_percent, 87.0);
        assert_eq!(secondary.window_minutes, Some(10_080));
        assert_eq!(limits.reset_credits_available, Some(2));
    }

    #[test]
    fn weekly_only_account_window_maps_to_weekly_and_marks_five_hour_unlimited() {
        let limits = account_limits_from_usage(CodexUsageResponse {
            rate_limit: Some(CodexRateLimit {
                primary_window: Some(CodexLimitWindow {
                    used_percent: Some(1.0),
                    limit_window_seconds: Some(604_800),
                    reset_at: Some(1_784_522_993),
                }),
                secondary_window: None,
            }),
            rate_limit_reset_credits: Some(CodexRateLimitResetCredits {
                available_count: Some(3),
            }),
        });

        let primary = limits
            .primary
            .expect("temporarily unlimited five-hour quota");
        let secondary = limits.secondary.expect("weekly quota");

        assert_eq!(primary.remaining_percent, 100.0);
        assert_eq!(primary.window_minutes, Some(300));
        assert_eq!(primary.resets_at, None);
        assert_eq!(secondary.remaining_percent, 99.0);
        assert_eq!(secondary.window_minutes, Some(10_080));
        assert_eq!(secondary.resets_at, Some(1_784_522_993));
    }

    #[test]
    fn failed_account_usage_does_not_fallback_to_session_quota() {
        let (primary, secondary, reset_credits, message) = merge_account_and_session_limits(
            Err("timeout".to_string()),
            Some(limit(0.0)),
            Some(limit(0.0)),
            0,
        );

        assert!(primary.is_none());
        assert!(secondary.is_none());
        assert_eq!(reset_credits, None);
        assert!(message.contains("Codex account quota unavailable"));
    }

    #[test]
    fn missing_account_windows_keep_cached_quota() {
        let current = AccountRateLimits {
            primary: None,
            secondary: Some(limit(50.0)),
            reset_credits_available: None,
        };
        let cached = AccountRateLimits {
            primary: Some(limit(25.0)),
            secondary: Some(limit(40.0)),
            reset_credits_available: Some(2),
        };

        let stabilized = stabilize_account_limits(current, Some(cached), 0);

        assert_eq!(stabilized.primary.expect("primary").used_percent, 25.0);
        assert_eq!(stabilized.secondary.expect("secondary").used_percent, 50.0);
        assert_eq!(stabilized.reset_credits_available, Some(2));
    }

    #[test]
    fn removed_five_hour_limit_replaces_cached_five_hour_quota() {
        let now = 1_784_000_000;
        let current = AccountRateLimits {
            primary: Some(unlimited_five_hour_window()),
            secondary: Some(limit_with_reset(1.0, 10_080, 1_784_522_993)),
            reset_credits_available: Some(3),
        };
        let cached = AccountRateLimits {
            primary: Some(limit_with_reset(42.0, 300, 1_784_010_000)),
            secondary: Some(limit_with_reset(38.0, 10_080, 1_784_356_548)),
            reset_credits_available: Some(3),
        };

        let stabilized = stabilize_account_limits(current, Some(cached), now);
        let primary = stabilized.primary.expect("unlimited five-hour quota");

        assert_eq!(primary.remaining_percent, 100.0);
        assert_eq!(primary.resets_at, None);
        assert_eq!(
            stabilized.secondary.expect("weekly quota").used_percent,
            1.0
        );
    }

    #[test]
    fn transient_quota_drop_does_not_overwrite_active_cycle() {
        let now = 1_783_673_398;
        let current = AccountRateLimits {
            primary: Some(limit_with_reset(1.0, 300, 1_783_683_659)),
            secondary: Some(limit_with_reset(0.0, 10_080, 1_784_252_351)),
            reset_credits_available: Some(3),
        };
        let cached = AccountRateLimits {
            primary: Some(limit_with_reset(52.0, 300, 1_783_683_532)),
            secondary: Some(limit_with_reset(16.0, 10_080, 1_784_252_324)),
            reset_credits_available: Some(3),
        };

        let stabilized = stabilize_account_limits(current, Some(cached), now);

        assert_eq!(stabilized.primary.expect("primary").used_percent, 52.0);
        assert_eq!(stabilized.secondary.expect("secondary").used_percent, 16.0);
    }

    #[test]
    fn transient_quota_drop_with_hour_scale_reset_drift_keeps_active_cycle() {
        let now = 1_783_838_861;
        let current = AccountRateLimits {
            primary: None,
            secondary: Some(limit_with_reset(0.0, 10_080, 1_784_359_418)),
            reset_credits_available: Some(3),
        };
        let cached = AccountRateLimits {
            primary: None,
            secondary: Some(limit_with_reset(14.0, 10_080, 1_784_356_548)),
            reset_credits_available: Some(3),
        };

        let stabilized = stabilize_account_limits(current, Some(cached), now);

        assert_eq!(stabilized.secondary.expect("secondary").used_percent, 14.0);
    }

    #[test]
    fn quota_drop_is_accepted_after_previous_reset() {
        let now = 1_783_683_600;
        let current = AccountRateLimits {
            primary: Some(limit_with_reset(1.0, 300, 1_783_701_600)),
            secondary: None,
            reset_credits_available: Some(3),
        };
        let cached = AccountRateLimits {
            primary: Some(limit_with_reset(52.0, 300, 1_783_683_532)),
            secondary: None,
            reset_credits_available: Some(3),
        };

        let stabilized = stabilize_account_limits(current, Some(cached), now);

        assert_eq!(stabilized.primary.expect("primary").used_percent, 1.0);
    }

    #[test]
    fn quota_drop_with_shifted_reset_starts_new_cycle() {
        let now = 1_783_752_634;
        let current = AccountRateLimits {
            primary: None,
            secondary: Some(limit_with_reset(0.0, 10_080, 1_784_356_548)),
            reset_credits_available: Some(3),
        };
        let cached = AccountRateLimits {
            primary: None,
            secondary: Some(limit_with_reset(20.0, 10_080, 1_784_252_324)),
            reset_credits_available: Some(3),
        };

        let stabilized = stabilize_account_limits(current, Some(cached), now);

        assert_eq!(stabilized.secondary.expect("secondary").used_percent, 0.0);
    }

    #[test]
    fn older_active_cycle_cannot_override_newer_active_cycle() {
        let now = 1_783_755_232;
        let newer = limit_with_reset(1.0, 10_080, 1_784_356_548);
        let older = limit_with_reset(20.0, 10_080, 1_784_252_324);

        let stabilized =
            stabilize_limit_window(Some(older), Some(newer), now).expect("newer quota cycle");

        assert_eq!(stabilized.used_percent, 1.0);
        assert_eq!(stabilized.resets_at, Some(1_784_356_548));
    }

    #[test]
    fn expired_session_quota_cannot_override_active_account_cycle() {
        let now = 1_783_683_600;
        let active_account = limit_with_reset(1.0, 300, 1_783_701_600);
        let expired_session = limit_with_reset(99.0, 300, 1_783_683_532);

        let stabilized = stabilize_limit_window(Some(expired_session), Some(active_account), now)
            .expect("active quota");

        assert_eq!(stabilized.used_percent, 1.0);
        assert_eq!(stabilized.resets_at, Some(1_783_701_600));
    }

    #[test]
    fn successful_account_usage_does_not_restore_removed_session_windows() {
        let account_limits = AccountRateLimits {
            primary: Some(limit(20.0)),
            secondary: None,
            reset_credits_available: Some(1),
        };
        let (primary, secondary, reset_credits, _) = merge_account_and_session_limits(
            Ok(account_limits),
            Some(limit(30.0)),
            Some(limit(40.0)),
            0,
        );

        assert_eq!(primary.expect("primary").used_percent, 20.0);
        assert!(secondary.is_none());
        assert_eq!(reset_credits, Some(1));
    }

    #[test]
    fn session_usage_cannot_override_authoritative_account_snapshot() {
        let now = 1_783_673_398;
        let account_limits = AccountRateLimits {
            primary: Some(limit_with_reset(1.0, 300, 1_783_683_659)),
            secondary: Some(limit_with_reset(0.0, 10_080, 1_784_252_351)),
            reset_credits_available: Some(3),
        };

        let (primary, secondary, _, _) = merge_account_and_session_limits(
            Ok(account_limits),
            Some(limit_with_reset(52.0, 300, 1_783_683_532)),
            Some(limit_with_reset(16.0, 10_080, 1_784_252_324)),
            now,
        );

        assert_eq!(primary.expect("primary").used_percent, 1.0);
        assert_eq!(secondary.expect("secondary").used_percent, 0.0);
    }

    #[test]
    fn parse_rate_limits_ignores_model_specific_limit_ids() {
        let value = serde_json::json!({
            "limit_id": "codex_bengalfox",
            "primary": {
                "used_percent": 0.0,
                "window_minutes": 300,
                "resets_at": 1_783_489_617
            },
            "secondary": {
                "used_percent": 0.0,
                "window_minutes": 10_080,
                "resets_at": 1_784_011_992
            }
        });

        assert!(parse_rate_limits(Some(&value)).is_none());
    }

    #[test]
    fn parse_rate_limits_keeps_account_limit_id() {
        let value = serde_json::json!({
            "limit_id": "codex",
            "primary": {
                "used_percent": 16.0,
                "window_minutes": 300,
                "resets_at": 1_783_489_617
            }
        });

        let limits = parse_rate_limits(Some(&value)).expect("account limits");

        assert_eq!(limits.primary.expect("primary").used_percent, 16.0);
    }

    #[test]
    fn scan_keeps_activity_only_session_for_animation_wake() {
        let path = std::env::temp_dir().join(format!(
            "token-meter-activity-only-{}.jsonl",
            std::process::id()
        ));
        let line = r#"{"timestamp":"2026-06-27T09:00:00Z","type":"event_msg","payload":{"type":"task_started"}}"#;
        std::fs::write(&path, line).expect("write test session");

        let scan = scan_session_file(&path)
            .expect("scan test session")
            .expect("activity-only scan should be retained");
        let _ = std::fs::remove_file(path);

        assert!(scan.events.is_empty());
        assert!(scan.last_activity_ts > 0);
    }

    #[test]
    fn scan_cache_reads_appended_session_tail() {
        let path = std::env::temp_dir().join(format!(
            "token-meter-incremental-{}.jsonl",
            std::process::id()
        ));
        let first = concat!(
            r#"{"timestamp":"2026-06-27T09:00:00Z","type":"session_meta","payload":{"id":"scan-test","cwd":"/tmp/project"}}"#,
            "\n",
            r#"{"timestamp":"2026-06-27T09:00:10Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":100}}}}"#,
            "\n"
        );
        std::fs::write(&path, first).expect("write initial session");

        let initial = scan_session_file_cached(&path)
            .expect("initial cached scan")
            .expect("initial scan should exist");
        assert_eq!(initial.events.len(), 1);
        assert_eq!(initial.events[0].total_tokens, 100);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open append session");
        use std::io::Write;
        writeln!(
            file,
            r#"{{"timestamp":"2026-06-27T09:00:20Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"total_tokens":250}}}}}}}}"#
        )
        .expect("append token count");

        let updated = scan_session_file_cached(&path)
            .expect("updated cached scan")
            .expect("updated scan should exist");
        let _ = std::fs::remove_file(path);

        assert_eq!(updated.id, "scan-test");
        assert_eq!(updated.cwd.as_deref(), Some("/tmp/project"));
        assert_eq!(updated.events.len(), 2);
        assert_eq!(updated.events[1].total_tokens, 250);
    }

    #[test]
    fn scan_cache_keeps_partial_appended_line_until_complete() {
        let path =
            std::env::temp_dir().join(format!("token-meter-partial-{}.jsonl", std::process::id()));
        let first = concat!(
            r#"{"timestamp":"2026-06-27T09:00:00Z","type":"session_meta","payload":{"id":"partial-test"}}"#,
            "\n",
            r#"{"timestamp":"2026-06-27T09:00:10Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":100}}}}"#,
            "\n"
        );
        std::fs::write(&path, first).expect("write initial partial session");

        let initial = scan_session_file_cached(&path)
            .expect("initial partial scan")
            .expect("initial partial scan should exist");
        assert_eq!(initial.events.len(), 1);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open partial append session");
        use std::io::Write;
        write!(
            file,
            r#"{{"timestamp":"2026-06-27T09:00:20Z","type":"event_msg","payload":{{"type":"token_count","info":{{"total_token_usage":{{"total_tokens":"#
        )
        .expect("append partial token count");
        drop(file);

        let partial = scan_session_file_cached(&path)
            .expect("partial cached scan")
            .expect("partial scan should still exist");
        assert_eq!(partial.events.len(), 1);

        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open completed append session");
        writeln!(file, r#"275}}}}}}}}"#).expect("finish partial token count");

        let complete = scan_session_file_cached(&path)
            .expect("complete cached scan")
            .expect("complete scan should exist");
        let _ = std::fs::remove_file(path);

        assert_eq!(complete.id, "partial-test");
        assert_eq!(complete.events.len(), 2);
        assert_eq!(complete.events[1].total_tokens, 275);
    }

    #[test]
    fn recent_rate_uses_now_so_stopped_sessions_decay() {
        let events = vec![event(1_000, 1_000), event(1_030, 4_000)];
        let (delta, rate) = recent_delta_and_rate(&events, 990, 1_050);

        assert_eq!(delta, 3_000);
        assert_rate(rate, 3_600.0);
    }

    #[test]
    fn recent_rate_drops_to_zero_after_window_passes_latest_event() {
        let events = vec![event(1_000, 1_000), event(1_030, 4_000)];
        let (delta, rate) = recent_delta_and_rate(&events, 1_031, 1_091);

        assert_eq!(delta, 0);
        assert_eq!(rate, 0.0);
    }

    #[test]
    fn recent_rate_uses_last_baseline_before_window() {
        let events = vec![event(900, 100), event(940, 500), event(980, 2_500)];
        let (delta, rate) = recent_delta_and_rate(&events, 940, 1_000);

        assert_eq!(delta, 2_000);
        assert_rate(rate, 2_000.0);
    }

    #[test]
    fn animation_rate_ignores_non_token_activity() {
        let rate = animation_rate_for_session(0.0, false, false);

        assert_eq!(rate, 0.0);
    }

    #[test]
    fn animation_rate_wakes_on_recent_codex_activity() {
        let rate = animation_rate_for_session(0.0, false, true);

        assert_rate(rate, ACTIVITY_WAKE_RATE_PER_MIN);
    }

    #[test]
    fn animation_rate_keeps_real_token_rate() {
        let rate = animation_rate_for_session(80_000.0, true, true);

        assert_rate(rate, 80_000.0);
    }
}
