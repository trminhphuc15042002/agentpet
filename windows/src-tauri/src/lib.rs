pub mod cli;
pub mod geometry;
pub mod hooks;
pub mod server;
pub mod statemap;
pub mod transcript;

// Unit-test executables also need Common Controls v6 for TaskDialogIndirect.
// Never apply this to production: tauri-build already links the bin resource,
// and MSVC rejects two copies of its VERSION/manifest records (CVT1100).
#[cfg(all(test, windows))]
#[cfg_attr(target_env = "gnu", link(name = "resource", kind = "static", modifiers = "+whole-archive"))]
// MSVC's resource.lib is an RC output (.res), not a COFF archive. Pass it
// directly, without bundling or /WHOLEARCHIVE (which rejects this format).
#[cfg_attr(target_env = "msvc", link(name = "resource.lib", kind = "static", modifiers = "-bundle,+verbatim"))]
extern "C" {}

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder};

/// Tray menu items kept around so the language switcher can re-label them live.
struct TrayItems {
    show_pet: tauri::menu::CheckMenuItem<tauri::Wry>,
    settings: MenuItem<tauri::Wry>,
    updates: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
    tray: tauri::tray::TrayIcon<tauri::Wry>,
}

/// The pet's opaque region in physical pixels, relative to the window's top-left.
/// The frontend reports this (canvas + visible bubble) so the background thread
/// can make the transparent rest of the window click-through.
#[derive(Default, Clone, Copy)]
#[cfg_attr(not(windows), allow(dead_code))]
struct HitRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Default)]
struct PetWinState {
    hit: HitRect,
    last_w: f64,
    last_h: f64,
    pet_width: f64,
    /// In-window pet shift, logical CSS px (returned to the webview).
    pet_offset: f64,
    /// Physical bottom-center of the pet; stable across resizes.
    anchor: Option<(f64, f64)>,
    last_origin: Option<(i32, i32)>,
    last_saved: Option<(i32, i32)>,
}

type PetWinMap = Mutex<HashMap<String, PetWinState>>;

fn is_pet_label(label: &str) -> bool {
    label == "pet" || label.starts_with("pet-")
}

fn qa_profile_root() -> Option<PathBuf> {
    #[cfg(debug_assertions)]
    {
        let raw = std::env::var("AGENTPET_QA_PROFILE").ok()?;
        if raw.is_empty() {
            return None;
        }
        Some(PathBuf::from(raw))
    }
    #[cfg(not(debug_assertions))]
    {
        None
    }
}

pub(crate) fn app_config_dir() -> Option<PathBuf> {
    if let Some(root) = qa_profile_root() {
        return Some(root.join("AgentPet"));
    }
    dirs::config_dir().map(|d| d.join("AgentPet"))
}

fn webview_browser_args() -> Option<String> {
    #[cfg(not(debug_assertions))]
    {
        return None;
    }
    #[cfg(debug_assertions)]
    {
        let raw = std::env::var("AGENTPET_CDP_PORT").ok()?;
        let port: u32 = raw.parse().ok()?;
        if !(1..=65535).contains(&port) {
            return None;
        }
        Some(format!(
            "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --remote-debugging-port={port} --remote-allow-origins=http://127.0.0.1:{port}"
        ))
    }
}

fn decorate_webview<R: tauri::Runtime, M: tauri::Manager<R>>(
    mut builder: WebviewWindowBuilder<R, M>,
) -> WebviewWindowBuilder<R, M> {
    if let Some(args) = webview_browser_args() {
        builder = builder.additional_browser_args(&args);
    }
    if let Some(root) = qa_profile_root() {
        builder = builder.data_directory(root.join("WebView2"));
    }
    builder
}

fn primary_mouse_down() -> bool {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn GetAsyncKeyState(v_key: i32) -> i16;
        }
        const VK_LBUTTON: i32 = 0x01;
        unsafe { GetAsyncKeyState(VK_LBUTTON) as u16 & 0x8000 != 0 }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn split_wanted() -> &'static Mutex<Option<Vec<String>>> {
    static W: OnceLock<Mutex<Option<Vec<String>>>> = OnceLock::new();
    W.get_or_init(|| Mutex::new(None))
}

fn split_worker_running() -> &'static Mutex<bool> {
    static R: OnceLock<Mutex<bool>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(false))
}

/// Append a line to %APPDATA%/AgentPet/debug.log , lightweight field
/// diagnostics for the Windows build (no console there).
pub(crate) fn dlog(msg: &str) {
    if let Some(p) = app_config_dir().map(|d| d.join("debug.log")) {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            use std::io::Write;
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let _ = writeln!(f, "[{ts}] {msg}");
        }
    }
}

#[tauri::command]
fn log_debug(msg: String) {
    dlog(&msg);
}

fn pos_file() -> Option<std::path::PathBuf> {
    app_config_dir().map(|d| d.join("pos"))
}

fn read_anchor(win: &tauri::WebviewWindow) -> Option<(f64, f64)> {
    let s = std::fs::read_to_string(pos_file()?).ok()?;
    let pos = geometry::parse_saved_pos(&s)?;
    let mons = collect_monitors(win);
    let fallback = win.scale_factor().unwrap_or(1.0);
    Some(geometry::saved_anchor_on_monitors(pos, &mons, fallback))
}

fn write_anchor(x: f64, y: f64) {
    if let Some(p) = pos_file() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, geometry::format_anchor(x, y));
    }
}

/// Report the pet's opaque rectangle (physical px, window-relative) so empty
/// transparent areas of the overlay let clicks pass through to apps below.
/// Async commands keep the UI thread free: the geometry worker can hold this
/// mutex while waiting for a native window operation on that thread.
#[tauri::command]
async fn set_hit_rect(window: tauri::WebviewWindow, app: tauri::AppHandle, x: f64, y: f64, w: f64, h: f64) {
    let label = window.label().to_string();
    if let Some(state) = app.try_state::<PetWinMap>() {
        if let Ok(mut map) = state.lock() {
            map.entry(label).or_default().hit = HitRect { x, y, w, h };
        }
    }
}

#[derive(serde::Serialize)]
struct ResizeReply {
    #[serde(rename = "petOffset")]
    pet_offset: f64,
    #[serde(rename = "windowWidth")]
    window_width: f64,
}

fn collect_monitors(win: &tauri::WebviewWindow) -> Vec<geometry::MonitorGeom> {
    win.available_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            let p = m.position();
            let s = m.size();
            let wa = m.work_area();
            geometry::MonitorGeom {
                frame: geometry::VisibleRect::from_pos_size(
                    p.x as f64,
                    p.y as f64,
                    s.width as f64,
                    s.height as f64,
                ),
                work: geometry::VisibleRect::from_pos_size(
                    wa.position.x as f64,
                    wa.position.y as f64,
                    wa.size.width as f64,
                    wa.size.height as f64,
                ),
                scale: m.scale_factor(),
            }
        })
        .collect()
}

fn frame_physical(win: &tauri::WebviewWindow) -> Option<(f64, f64, f64, f64)> {
    let pos = win.outer_position().ok()?;
    let size = win.outer_size().ok()?;
    Some((pos.x as f64, pos.y as f64, size.width as f64, size.height as f64))
}

fn refresh_drag_anchor(win: &tauri::WebviewWindow, st: &mut PetWinState) {
    if let Some((x, y, w, h)) = frame_physical(win) {
        let origin = (x as i32, y as i32);
        // Don't recapture rounded programmatic coordinates on every resize:
        // repeated half-pixel rounding would slowly move the pet at 150% DPI.
        if st.anchor.is_none() || st.last_origin != Some(origin) {
            let scale = win.scale_factor().unwrap_or(1.0);
            st.anchor = Some(geometry::anchor_from_frame(x, y, w, h, st.pet_offset * scale));
        }
        st.last_origin = Some(origin);
    }
}

fn pick_work_area(
    win: &tauri::WebviewWindow,
    anchor_x: f64,
    anchor_y: f64,
) -> Option<geometry::VisibleRect> {
    let mons = collect_monitors(win);
    let (cx, cy) = frame_physical(win)
        .map(|(x, y, w, h)| (x + w / 2.0, y + h / 2.0))
        .unwrap_or((anchor_x, anchor_y));
    let idx = geometry::monitor_containing_pet(anchor_x, anchor_y, cx, cy, &mons)
        .or_else(|| if mons.is_empty() { None } else { Some(0) })?;
    Some(mons[idx].work)
}

fn apply_window_layout(
    win: &tauri::WebviewWindow,
    st: &mut PetWinState,
    logical_w: f64,
    logical_h: f64,
    pet_width_logical: f64,
) -> Result<ResizeReply, String> {
    let scale = win.scale_factor().unwrap_or(1.0);
    let phys_w = geometry::logical_to_physical(logical_w, scale).round();
    let phys_h = geometry::logical_to_physical(logical_h, scale).round();
    let pet_w = geometry::logical_to_physical(pet_width_logical.max(1.0), scale);
    refresh_drag_anchor(win, st);
    let (ax, ay) = st.anchor.unwrap_or((0.0, 0.0));
    let visible = pick_work_area(win, ax, ay)
        .unwrap_or(geometry::VisibleRect::from_pos_size(0.0, 0.0, phys_w, phys_h));
    let layout = geometry::layout_window(ax, ay, phys_w, phys_h, pet_w, visible);
    set_pet_frame(win, layout.origin_x.round() as i32, layout.origin_y.round() as i32,
        phys_w as i32, phys_h as i32)?;
    st.last_w = logical_w;
    st.last_h = logical_h;
    st.pet_width = pet_width_logical;
    st.last_origin = Some((layout.origin_x.round() as i32, layout.origin_y.round() as i32));
    st.pet_offset = geometry::physical_to_logical(layout.pet_offset + layout.origin_x - layout.origin_x.round(), scale);
    st.anchor = Some(geometry::anchor_from_frame(
        layout.origin_x,
        layout.origin_y,
        phys_w,
        phys_h,
        layout.pet_offset,
    ));
    Ok(ResizeReply {
        pet_offset: st.pet_offset,
        window_width: logical_w,
    })
}

/// Position and size must reach Windows in one operation. Two Tauri setters
/// expose a frame with the new origin and OLD width, briefly moving the pet by
/// half the bubble's width change even when called together on the UI thread.
fn set_pet_frame(win: &tauri::WebviewWindow, x: i32, y: i32, w: i32, h: i32) -> Result<(), String> {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn SetWindowPos(hwnd: *mut std::ffi::c_void, after: *mut std::ffi::c_void,
                x: i32, y: i32, w: i32, h: i32, flags: u32) -> i32;
        }
        let hwnd = win.hwnd().map_err(|e| e.to_string())?;
        // SWP_NOZORDER | SWP_NOACTIVATE: never raise or focus the overlay.
        if unsafe { SetWindowPos(hwnd.0 as _, std::ptr::null_mut(), x, y, w, h, 0x0014) } == 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        win.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
        win.set_size(tauri::PhysicalSize::new(w as u32, h as u32)).map_err(|e| e.to_string())
    }
}

fn emit_pet_geometry(win: &tauri::WebviewWindow, st: &PetWinState) {
    let _ = win.emit(
        "pet-geometry",
        serde_json::json!({
            "petOffset": st.pet_offset,
            "windowWidth": st.last_w,
        }),
    );
}

fn ensure_on_screen(win: &tauri::WebviewWindow, st: &mut PetWinState) {
    let Some((ox, oy, w, h)) = frame_physical(win) else { return };
    let scale = win.scale_factor().unwrap_or(1.0);
    let pet_off = geometry::logical_to_physical(st.pet_offset, scale);
    refresh_drag_anchor(win, st);
    let (ax, ay) = st.anchor.unwrap_or_else(|| geometry::anchor_from_frame(ox, oy, w, h, pet_off));
    if primary_mouse_down() {
        return;
    }
    let mons = collect_monitors(win);
    let (cx, cy) = (ox + w / 2.0, oy + h / 2.0);
    let Some(idx) = geometry::monitor_containing_pet(ax, ay, cx, cy, &mons)
        .or_else(|| if mons.is_empty() { None } else { Some(0) })
    else {
        return;
    };
    let work = mons[idx].work;
    let oversized = w > (work.max_x - work.min_x) + 0.5 || h > (work.max_y - work.min_y) + 0.5;
    if !geometry::frame_outside_work(ox, oy, w, h, work) && !oversized {
        return;
    }
    let pet_w = if st.pet_width > 0.0 {
        geometry::logical_to_physical(st.pet_width, scale)
    } else {
        pet_off.abs() * 2.0 + 1.0
    };
    let layout = geometry::layout_from_live_frame(ox, oy, w, h, pet_off, pet_w.max(1.0), work);
    if (layout.origin_x - ox).abs() < 0.5 && (layout.origin_y - oy).abs() < 0.5 {
        st.pet_offset = geometry::physical_to_logical(layout.pet_offset, scale);
        return;
    }
    let _ = win.set_position(PhysicalPosition::new(
        layout.origin_x.round() as i32,
        layout.origin_y.round() as i32,
    ));
    st.last_origin = Some((layout.origin_x.round() as i32, layout.origin_y.round() as i32));
    st.pet_offset = geometry::physical_to_logical(layout.pet_offset, scale);
    st.anchor = Some(geometry::anchor_from_frame(
        layout.origin_x,
        layout.origin_y,
        w,
        h,
        layout.pet_offset,
    ));
    if st.last_w <= 0.0 {
        st.last_w = geometry::physical_to_logical(w, scale);
    }
    emit_pet_geometry(win, st);
}

/// Calling-window resize: hug content, keep the pet's bottom-center on the
/// monitor it already occupies, clamp to that display's work area / DPI.
#[tauri::command]
async fn resize_pet_window(
    window: tauri::WebviewWindow,
    app: tauri::AppHandle,
    width: f64,
    height: f64,
    pet_width: f64,
) -> Result<ResizeReply, String> {
    if !(width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0) {
        return Err("invalid size".into());
    }
    // Commit position, size and anchor together on the UI thread; otherwise a
    // second resize can mistake a half-applied frame for a user drag.
    let (tx, rx) = std::sync::mpsc::channel();
    let target = window.clone();
    window.run_on_main_thread(move || {
        let result = resize_pet_window_impl(&target, &app, width, height, pet_width);
        let _ = tx.send(result);
    }).map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        rx.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?
    }).await.map_err(|e| e.to_string())?
}

fn resize_pet_window_impl(
    window: &tauri::WebviewWindow,
    app: &tauri::AppHandle,
    width: f64,
    height: f64,
    pet_width: f64,
) -> Result<ResizeReply, String> {
    let pet_w = if pet_width.is_finite() && pet_width > 0.0 { pet_width } else { 160.0 };
    let label = window.label().to_string();
    let Some(state) = app.try_state::<PetWinMap>() else {
        return Err("no window state".into());
    };
    let mut map = state.lock().map_err(|e| e.to_string())?;
    let st = map.entry(label).or_default();
    let scale = window.scale_factor().unwrap_or(1.0);
    let matches_viewport = window.inner_size().map(|size| {
        !geometry::size_changed(width, height, size.width as f64 / scale, size.height as f64 / scale, 1.0)
    }).unwrap_or(false);
    if matches_viewport && st.last_w > 0.0 && !geometry::size_changed(width, height, st.last_w, st.last_h, 1.0) {
        return Ok(ResizeReply {
            pet_offset: st.pet_offset,
            window_width: st.last_w,
        });
    }
    apply_window_layout(window, st, width, height, pet_w)
}

fn lang_file() -> Option<std::path::PathBuf> {
    app_config_dir().map(|d| d.join("lang"))
}

fn read_lang() -> String {
    lang_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "en".into())
}

fn write_lang(code: &str) {
    if let Some(p) = lang_file() {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, code);
    }
}

/// Localised tray labels (the only app text on the Rust side).
fn tray_labels(code: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    match code {
        "vi" => ("Hiện pet", "Cài đặt", "Kiểm tra cập nhật", "Thoát AgentPet"),
        "zh" => ("显示宠物", "设置", "检查更新", "退出 AgentPet"),
        "zh-TW" => ("顯示寵物", "設定", "檢查更新", "結束 AgentPet"),
        _ => ("Show pet", "Settings", "Check for updates", "Quit AgentPet"),
    }
}

#[tauri::command]
fn list_agents() -> Vec<hooks::AgentInfo> {
    hooks::catalog()
}

#[tauri::command]
fn is_installed(kind: String) -> bool {
    hooks::is_installed(&kind)
}

#[tauri::command]
fn toggle_install(kind: String) -> Result<bool, String> {
    hooks::toggle(&kind)
}

/// Window creation MUST NOT run inside a sync command / menu callback on
/// Windows (the webview build deadlocks against the blocked event loop), so
/// every caller goes through this thread-spawning wrapper.
fn open_settings_impl(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        dlog("open_settings: worker thread");
        if let Some(w) = app.get_webview_window("settings") {
            dlog("open_settings: existing window, showing");
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            return;
        }
        match decorate_webview(
            WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
                .title("AgentPet")
                .inner_size(640.0, 620.0)
                .resizable(false),
        )
        .build()
        {
            Ok(_) => dlog("open_settings: window created"),
            Err(e) => dlog(&format!("open_settings: BUILD FAILED: {e}")),
        }
    });
}

#[tauri::command]
async fn open_settings(app: tauri::AppHandle) {
    dlog("open_settings called");
    open_settings_impl(app);
}

/// Open an external link in the default browser (About tab buttons).
/// A row for the tray's "Sessions" submenu, built by the pet window.
#[derive(serde::Deserialize)]
struct TraySession {
    session: String,
    label: String,
}

/// Rebuild the tray's "Sessions" submenu from the live session list (the pet
/// window pushes it every render). Picking an item opens that session in
/// OpenChamber, same deep link as the `open_session` command.
#[tauri::command]
fn set_tray_sessions(app: tauri::AppHandle, sessions: Vec<TraySession>) {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    let Some(items) = app.try_state::<Mutex<TrayItems>>() else { return };
    let Ok(it) = items.lock() else { return };
    let header = match read_lang().as_str() {
        "vi" => "Phiên đang chạy",
        "zh" => "会话",
        "zh-TW" => "工作階段",
        _ => "Sessions",
    };
    let Ok(sub) = Submenu::with_id(&app, "sessions", header, true) else { return };
    if sessions.is_empty() {
        if let Ok(none) = MenuItem::with_id(&app, "no_sessions", "—", false, None::<&str>) {
            let _ = sub.append(&none);
        }
    }
    for s in sessions.iter().take(12) {
        if let Ok(mi) = MenuItem::with_id(&app, format!("session:{}", s.session), &s.label, true, None::<&str>) {
            let _ = sub.append(&mi);
        }
    }
    let Ok(sep) = PredefinedMenuItem::separator(&app) else { return };
    let mut entries: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> =
        vec![&it.show_pet, &it.settings, &it.updates, &sep, &sub, &it.quit];
    let Ok(menu) = Menu::with_items(&app, &entries) else { return };
    let _ = it.tray.set_menu(Some(menu));
}

#[tauri::command]
fn open_url(url: String) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd").args(["/c", "start", "", &url]).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(&url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(&url).spawn();
    }
}

/// Bring the terminal running a session to the front when its bubble row is
/// clicked. Warp exports a `warp://session/<uuid>` deep link that focuses the
/// exact pane , the one reliable cross-platform "focus exact tab". Other
/// terminals have no dependable tab-focus API on Windows/Linux, so we only
/// best-effort activate the app on macOS (where the Tauri build is dev-only).
/// A safe `warp://session/<uuid>` deep link: scheme + only URL-safe characters,
/// so it can never carry shell metacharacters into `cmd /c start`.
fn is_safe_warp_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("warp://") else { return false };
    !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

#[tauri::command]
fn focus_terminal(program: String, focus_url: String) {
    if is_safe_warp_url(&focus_url) {
        #[cfg(windows)]
        { let _ = std::process::Command::new("cmd").args(["/c", "start", "", &focus_url]).spawn(); }
        #[cfg(target_os = "macos")]
        { let _ = std::process::Command::new("open").arg(&focus_url).spawn(); }
        #[cfg(all(unix, not(target_os = "macos")))]
        { let _ = std::process::Command::new("xdg-open").arg(&focus_url).spawn(); }
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let app = match program.as_str() {
            "Apple_Terminal" => Some("Terminal"),
            "iTerm.app" => Some("iTerm"),
            _ => None,
        };
        if let Some(a) = app {
            let _ = std::process::Command::new("open").args(["-a", a]).spawn();
        }
    }
    let _ = program;
}

/// A safe OpenCode session id: alphanumerics, `_` and `-` only, so it can never
/// carry shell metacharacters into `cmd /c start`.
fn is_safe_opencode_session(id: &str) -> bool {
    !id.is_empty()
        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

/// Open an OpenCode session inside OpenChamber using its
/// `openchamber://session/<id>` deep link. `session_id` is the raw store key
/// (`opencode:ses_...`) so callers can pass `Session.session` verbatim.
#[tauri::command]
fn open_session(session_id: String) {
    let id = session_id.strip_prefix("opencode:").unwrap_or(session_id.as_str());
    if !is_safe_opencode_session(id) { return; }
    let url = format!("openchamber://session/{id}");
    #[cfg(windows)]
    { let _ = std::process::Command::new("cmd").args(["/c", "start", "", &url]).spawn(); }
    #[cfg(target_os = "macos")]
    { let _ = std::process::Command::new("open").arg(&url).spawn(); }
    #[cfg(all(unix, not(target_os = "macos")))]
    { let _ = std::process::Command::new("xdg-open").arg(&url).spawn(); }
}

/// Deliver the user's Allow/Deny decision for a gated tool call back to the
/// parked hook request (see server::handle_approval).
#[tauri::command]
fn resolve_approval(id: String, decision: String) {
    crate::server::resolve_approval(&id, &decision);
}

/// Split-pet: ensure exactly one extra pet window `pet-<projectId>` exists per
/// configured project, cloning the main pet's chrome. Each loads `index.html`
/// with a `?project=<id>` query so its script shows only that project. Closing
/// happens for any `pet-*` window no longer in the list (merge back). With split
/// off, the frontend calls this with an empty list, so all extras close.
fn apply_split_windows(app: &tauri::AppHandle, projects: &[String]) {
    use std::collections::HashSet;
    let want: HashSet<String> = projects.iter().map(|id| format!("pet-{id}")).collect();
    for (label, win) in app.webview_windows() {
        if label.starts_with("pet-") && !want.contains(&label) {
            let _ = win.close();
        }
    }
    for (i, id) in projects.iter().enumerate() {
        let label = format!("pet-{id}");
        if app.get_webview_window(&label).is_some() {
            continue;
        }
        let url = format!("index.html?project={id}");
        match decorate_webview(
            WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
                .title("AgentPet")
                .inner_size(260.0, 320.0)
                .position(1200.0 - (i as f64 + 1.0) * 60.0, 600.0)
                .transparent(true)
                .decorations(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .resizable(false)
                .shadow(false)
                .focused(false),
        )
        .build()
        {
            Ok(_) => dlog(&format!("sync_project_windows: created {label}")),
            Err(e) => dlog(&format!("sync_project_windows: BUILD FAILED {label}: {e}")),
        }
    }
}

#[tauri::command]
fn sync_project_windows(app: tauri::AppHandle, projects: Vec<String>) {
    if let Ok(mut want) = split_wanted().lock() {
        *want = Some(projects);
    }
    {
        let mut running = match split_worker_running().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if *running {
            return;
        }
        *running = true;
    }
    std::thread::spawn(move || loop {
        let batch = split_wanted().lock().ok().and_then(|mut g| g.take());
        let Some(projects) = batch else {
            if let Ok(mut running) = split_worker_running().lock() {
                *running = false;
            }
            break;
        };
        apply_split_windows(&app, &projects);
    });
}

/// Persist the chosen language (for the tray on next launch) and re-label the
/// tray menu items now. Called by the Settings language switcher.
#[tauri::command]
fn set_lang(app: tauri::AppHandle, code: String) {
    write_lang(&code);
    let (p, s, u, q) = tray_labels(&code);
    if let Some(items) = app.try_state::<Mutex<TrayItems>>() {
        if let Ok(it) = items.lock() {
            let _ = it.show_pet.set_text(p);
            let _ = it.settings.set_text(s);
            let _ = it.updates.set_text(u);
            let _ = it.quit.set_text(q);
        }
    }
}

/// Live agent counts from the pet window → tray tooltip (the macOS app shows
/// the count next to the menu bar icon; the Windows tray equivalent).
#[tauri::command]
fn set_tray_status(app: tauri::AppHandle, working: u32, waiting: u32) {
    if let Some(items) = app.try_state::<Mutex<TrayItems>>() {
        if let Ok(it) = items.lock() {
            let tip = if waiting > 0 {
                format!("AgentPet , {waiting} waiting for you")
            } else if working > 0 {
                format!("AgentPet , {working} working")
            } else {
                "AgentPet".to_string()
            };
            let _ = it.tray.set_tooltip(Some(tip));
        }
    }
}

#[tauri::command]
fn get_pet_visible(app: tauri::AppHandle) -> bool {
    app.get_webview_window("pet")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(true)
}

/// Show the popover (the macOS menu-bar popover equivalent) near the cursor.
fn show_popover(app: &tauri::AppHandle) {
    let win = match app.get_webview_window("popover") {
        Some(w) => w,
        None => {
            match decorate_webview(
                WebviewWindowBuilder::new(app, "popover", WebviewUrl::App("popover.html".into()))
                    .title("AgentPet")
                    .inner_size(300.0, 430.0)
                    .decorations(false)
                    .transparent(true)
                    .always_on_top(true)
                    .skip_taskbar(true)
                    .resizable(false)
                    .focused(true)
                    .visible(false),
            )
            .build()
            {
                Ok(w) => {
                    dlog("popover: window created");
                    // Transient popover: losing focus hides it (Rust-side net,
                    // independent of the webview's own blur listener).
                    let wh = w.clone();
                    w.on_window_event(move |ev| {
                        if let tauri::WindowEvent::Focused(false) = ev {
                            let _ = wh.hide();
                        }
                    });
                    w
                }
                Err(e) => {
                    dlog(&format!("popover: BUILD FAILED: {e}"));
                    return;
                }
            }
        }
    };
    // Place near the cursor, clamped onto the monitor under it.
    if let Ok(cur) = app.cursor_position() {
        let sf = win.scale_factor().unwrap_or(1.0);
        let (w, h) = (300.0 * sf, 430.0 * sf);
        let mut x = cur.x - w / 2.0;
        let mut y = cur.y - h - 12.0; // prefer above the cursor (tray at bottom)
        if let Ok(Some(mon)) = app.monitor_from_point(cur.x, cur.y) {
            let mp = mon.position();
            let ms = mon.size();
            // Keep the popover above the cursor and clamp to the monitor top.
            // Dropping below the cursor used to land it on the pet, which is an
            // always-on-top window and drew over the card.
            x = x.max(mp.x as f64).min(mp.x as f64 + ms.width as f64 - w);
            y = y.max(mp.y as f64).min(mp.y as f64 + ms.height as f64 - h);
        }
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }
    let _ = win.show();
    // Re-assert topmost on every show so the card sits above the always-on-top
    // pet window (creation-time topmost is lost once the pet is re-shown).
    let _ = win.set_always_on_top(true);
    let _ = win.set_focus();
    let _ = win.emit("popover-shown", ());
}

#[tauri::command]
async fn open_popover(app: tauri::AppHandle) {
    dlog("open_popover called");
    std::thread::spawn(move || show_popover(&app));
}

/// Show/hide the pet overlay (tray toggle , the macOS "Show pet" switch).
#[tauri::command]
fn set_pet_visible(app: tauri::AppHandle, visible: bool) {
    if let Some(win) = app.get_webview_window("pet") {
        if visible {
            let _ = win.show();
        } else {
            let _ = win.hide();
        }
    }
    if let Some(p) = app_config_dir().map(|d| d.join("petvisible")) {
        let _ = std::fs::write(p, if visible { "1" } else { "0" });
    }
    if let Some(items) = app.try_state::<Mutex<TrayItems>>() {
        if let Ok(it) = items.lock() {
            let _ = it.show_pet.set_checked(visible);
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be the first plugin: a second launch (double-clicking the
        // shortcut while the app runs) exits immediately and the running
        // instance opens Settings instead , no duplicate pets.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            open_settings_impl(app.clone());
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            list_agents,
            is_installed,
            toggle_install,
            open_settings,
            open_url,
            focus_terminal,
            open_session,
            resolve_approval,
            sync_project_windows,
            set_lang,
            set_tray_status,
            set_tray_sessions,
            set_pet_visible,
            get_pet_visible,
            open_popover,
            log_debug,
            set_hit_rect,
            resize_pet_window
        ])
        .setup(|app| {
            server::start(app.handle().clone());
            app.manage(Mutex::new(HashMap::<String, PetWinState>::new()));

            dlog(&format!("cdp args={:?}", webview_browser_args()));
            if app.get_webview_window("pet").is_none() {
                let built = decorate_webview(
                    WebviewWindowBuilder::new(app.handle(), "pet", WebviewUrl::App("index.html".into()))
                        .title("AgentPet")
                        .inner_size(260.0, 320.0)
                        .position(1200.0, 600.0)
                        .transparent(true)
                        .decorations(false)
                        .always_on_top(true)
                        .skip_taskbar(true)
                        .resizable(false)
                        .shadow(false)
                        .focused(false),
                )
                .build();
                match built {
                    Ok(_) => dlog("pet window created"),
                    Err(e) => dlog(&format!("pet window BUILD FAILED: {e}")),
                }
            }

            // Restore the pet's bottom-center anchor. First run (no saved
            // position) parks it near the bottom-right of the primary work area.
            if let Some(win) = app.get_webview_window("pet") {
                let on_screen = |x: f64, y: f64| {
                    win.available_monitors().map_or(false, |mons| {
                        mons.iter().any(|m| {
                            let p = m.position();
                            let s = m.size();
                            x >= p.x as f64
                                && x < p.x as f64 + s.width as f64
                                && y >= p.y as f64
                                && y < p.y as f64 + s.height as f64
                        })
                    })
                };
                if let Some((ax, ay)) = read_anchor(&win).filter(|&(x, y)| on_screen(x, y)) {
                    if let Some(state) = app.try_state::<PetWinMap>() {
                        if let Ok(mut map) = state.lock() {
                            let st = map.entry("pet".into()).or_default();
                            st.anchor = Some((ax, ay));
                            st.last_origin = win.outer_position().ok().map(|p| (p.x, p.y));
                            let _ = apply_window_layout(&win, st, 260.0, 320.0, 160.0);
                        }
                    }
                } else if let Ok(Some(mon)) = win.primary_monitor() {
                    let wa = mon.work_area();
                    let s = mon.scale_factor();
                    let w = 260.0 * s;
                    let h = 320.0 * s;
                    let x = (wa.position.x as f64 + wa.size.width as f64 - w - 40.0 * s)
                        .max(wa.position.x as f64);
                    let y = (wa.position.y as f64 + wa.size.height as f64 - h - 70.0 * s)
                        .max(wa.position.y as f64);
                    let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
                    if let Some(state) = app.try_state::<PetWinMap>() {
                        if let Ok(mut map) = state.lock() {
                            let st = map.entry("pet".into()).or_default();
                            st.anchor = Some(geometry::anchor_from_frame(x, y, w, h, 0.0));
                            st.last_w = 260.0;
                            st.last_h = 320.0;
                        }
                    }
                }
            }

            // Background loop: (1) make transparent areas of the overlay
            // click-through by toggling cursor-event capture based on whether the
            // cursor is over the pet's opaque rect (cross-platform via tao), and
            // (2) persist the pet's position so it survives a restart.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut last_ignore: HashMap<String, bool> = HashMap::new();
                let mut last_popover: Option<bool> = None;
                let mut flip_logs: u32 = 0;
                let mut tick: u32 = 0;
                loop {
                    std::thread::sleep(Duration::from_millis(30));
                    let pets: Vec<(String, tauri::WebviewWindow)> = handle
                        .webview_windows()
                        .into_iter()
                        .filter(|(label, _)| is_pet_label(label))
                        .collect();
                    if pets.is_empty() {
                        continue;
                    }

                    // While the popover is open, drop every pet's topmost flag so
                    // the popover (also topmost) is drawn above it. Creation/
                    // show-time topmost on the popover was not enough: the pet,
                    // being topmost too, still covered the card. Restored as soon
                    // as the popover hides.
                    let popover_open = handle
                        .get_webview_window("popover")
                        .and_then(|p| p.is_visible().ok())
                        .unwrap_or(false);
                    if Some(popover_open) != last_popover {
                        for (_, win) in &pets {
                            let _ = win.set_always_on_top(!popover_open);
                        }
                        last_popover = Some(popover_open);
                    }

                    // Cross-platform (tao): cursor + window in physical px.
                    // Fail-safe: while the hit rect is unknown (webview still
                    // booting) or the cursor can't be read, keep the window
                    // INTERACTIVE , a clickable pet beats an untouchable one.
                    let cursor = handle.cursor_position();
                    for (label, win) in &pets {
                        match (cursor.as_ref(), win.outer_position()) {
                            (Ok(cur), Ok(wp)) => {
                                let rect = handle.try_state::<PetWinMap>().and_then(|s| {
                                    s.lock().ok().and_then(|m| {
                                        m.get(label).map(|st| (st.hit.x, st.hit.y, st.hit.w, st.hit.h))
                                    })
                                });
                                let inside = match rect {
                                    Some((x, y, w, h)) if w > 0.0 => {
                                        let rx = cur.x - wp.x as f64;
                                        let ry = cur.y - wp.y as f64;
                                        rx >= x && rx <= x + w && ry >= y && ry <= y + h
                                    }
                                    _ => true, // no rect yet , stay interactive
                                };
                                // ignore_cursor_events = true -> clicks pass through.
                                let ignore = !inside;
                                if last_ignore.get(label).copied() != Some(ignore) {
                                    let _ = win.set_ignore_cursor_events(ignore);
                                    last_ignore.insert(label.clone(), ignore);
                                    if flip_logs < 30 {
                                        flip_logs += 1;
                                        dlog(&format!(
                                            "hit flip {label}: ignore={ignore} cur=({:.0},{:.0}) win=({},{}) rect={:?}",
                                            cur.x, cur.y, wp.x, wp.y, rect
                                        ));
                                    }
                                }
                            }
                            (Err(e), _) => {
                                if last_ignore.get(label).copied() != Some(false) {
                                    dlog(&format!("cursor_position error: {e} , forcing interactive"));
                                    let _ = win.set_ignore_cursor_events(false);
                                    last_ignore.insert(label.clone(), false);
                                }
                            }
                            _ => {}
                        }
                    }

                    tick = tick.wrapping_add(1);
                    if tick % 33 == 0 {
                        let maintenance = handle.clone();
                        let _ = handle.run_on_main_thread(move || {
                        if let Some(state) = maintenance.try_state::<PetWinMap>() {
                            if let Ok(mut map) = state.lock() {
                                for (label, win) in &pets {
                                    let st = map.entry(label.clone()).or_default();
                                    ensure_on_screen(win, st);
                                }
                                if let Some(st) = map.get_mut("pet") {
                                    if let Some((ax, ay)) = st.anchor {
                                        let key = (ax.round() as i32, ay.round() as i32);
                                        if st.last_saved != Some(key) {
                                            write_anchor(ax, ay);
                                            st.last_saved = Some(key);
                                        }
                                    }
                                }
                            }
                        }
                        });
                    }
                }
            });

            // Tray menu , the pet window is frameless, so this is how you reach
            // Settings or quit the app. Labels start in the saved language; the
            // Settings switcher re-labels them live via the `set_lang` command.
            let (p_lbl, s_lbl, u_lbl, q_lbl) = tray_labels(&read_lang());
            let pet_visible = app_config_dir()
                .map(|d| d.join("petvisible"))
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.trim() != "0")
                .unwrap_or(true);
            let show_pet_i = tauri::menu::CheckMenuItem::with_id(
                app, "show_pet", p_lbl, true, pet_visible, None::<&str>)?;
            let settings_i = MenuItem::with_id(app, "settings", s_lbl, true, None::<&str>)?;
            let updates_i = MenuItem::with_id(app, "check_updates", u_lbl, true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", q_lbl, true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_pet_i, &settings_i, &updates_i, &quit_i])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("AgentPet")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    // Left-click on the tray icon opens Settings; the pet's
                    // right-click popover covers the quick controls.
                    if let tauri::tray::TrayIconEvent::Click {
                        button: tauri::tray::MouseButton::Left,
                        button_state: tauri::tray::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        open_settings_impl(tray.app_handle().clone());
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show_pet" => {
                        let now_visible = app
                            .get_webview_window("pet")
                            .and_then(|w| w.is_visible().ok())
                            .unwrap_or(true);
                        set_pet_visible(app.clone(), !now_visible);
                    }
                    "settings" => open_settings_impl(app.clone()),
                    id if id.starts_with("session:") => {
                        open_session(id["session:".len()..].to_string());
                    }
                    "check_updates" => {
                        // The updater plugin is driven from JS; the pet window is
                        // always alive (hidden, never closed) so it receives this
                        // and runs check()/downloadAndInstall() with notification
                        // feedback. Gives Linux (appindicator has no tray-click)
                        // and Windows a reliable Updates entry point.
                        if let Some(win) = app.get_webview_window("pet") {
                            let _ = win.emit("check-updates", ());
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            let tray = tray.build(app)?;
            app.manage(Mutex::new(TrayItems {
                show_pet: show_pet_i.clone(),
                settings: settings_i.clone(),
                updates: updates_i.clone(),
                quit: quit_i.clone(),
                tray,
            }));
            if !pet_visible {
                if let Some(win) = app.get_webview_window("pet") {
                    let _ = win.hide();
                }
            }

            dlog("setup complete, tray + loop running");
            // First run: open Settings with the welcome overlay so the user knows
            // to pick a pet and connect an agent (a port of the macOS onboarding).
            let marker = app_config_dir().map(|d| d.join(".onboarded"));
            if let Some(m) = marker {
                if !m.exists() {
                    let h = app.handle().clone();
                    std::thread::spawn(move || {
                        let _ = decorate_webview(
                            WebviewWindowBuilder::new(
                                &h, "settings", WebviewUrl::App("settings.html?onboarding=1".into()))
                                .title("AgentPet")
                                .inner_size(640.0, 620.0)
                                .resizable(false),
                        )
                        .build();
                    });
                    if let Some(parent) = m.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let _ = std::fs::write(&m, "1");
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AgentPet");
}
