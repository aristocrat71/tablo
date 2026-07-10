//! Tablo — a tiny floating cat that watches Claude Code agents.
//!
//! Three windows share one webview bundle and select their surface by label:
//!   - `avatar`    transparent, always-on-top, click-through-except-on-the-cat
//!   - `panel`     toggled open on tap, dismissed on blur / second tap
//!   - `dashboard` the deep view, opened from the panel
//!
//! A background thread tails the transcripts (see `scanner`) and pushes a
//! `state-update` event to every window; a second thread polls the cursor to
//! keep only the cat interactive.

// AeroSpace is a macOS-only tiling WM, so the follow module only compiles there.
// Elsewhere a tiny shim keeps `aerospace::available()` (called by the scanner)
// resolvable — always false, so the Settings toggle never appears.
#[cfg(target_os = "macos")]
mod aerospace;
#[cfg(not(target_os = "macos"))]
mod aerospace {
    pub fn available() -> bool {
        false
    }
}
mod codex;
mod codex_locate;
mod config;
mod locate;
mod permission;
mod scanner;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalPosition, Manager, PhysicalPosition, State, WebviewWindow,
    WindowEvent,
};

use config::Config;
use permission::{PendingRequest, PermDecision};
use scanner::{ClaudeConfigCache, FileState, Snapshot};

// Interaction/timing tuning (not domain data — window geometry lives in
// tauri.conf.json, all business values in `config::Config`).
/// Fraction of the avatar window's smaller dimension treated as the cat's
/// interactive radius.
const HIT_RADIUS_FRACTION: f64 = 0.38;
/// How often the cursor is polled for click-through hit-testing.
const CURSOR_POLL: Duration = Duration::from_millis(70);
/// Base cadence of the transcript scan (also ages sessions to idle).
const SCAN_INTERVAL: Duration = Duration::from_millis(1000);
/// Coalesce a burst of file-watch events before rescanning.
const EVENT_DEBOUNCE: Duration = Duration::from_millis(150);
/// A tap arriving within this of a blur-hide is treated as the close, not a reopen.
const PANEL_DISMISS_GUARD: Duration = Duration::from_millis(250);

/// Shared, thread-safe application state.
pub(crate) struct AppState {
    pub(crate) config: Mutex<Config>,
    config_dir: PathBuf,
    snapshot: Mutex<Snapshot>,
    files: Mutex<HashMap<PathBuf, FileState>>,
    /// Cached per-project window map from `~/.claude.json`.
    claude_cfg: Mutex<ClaudeConfigCache>,
    /// Last emitted level per session id, for one-shot warning notifications.
    prev_levels: Mutex<HashMap<String, String>>,
    /// Last seen activity kind per session id (perm/working/thinking/waiting/""),
    /// to fire a one-shot "session-waiting" event on a working→waiting crossing.
    prev_activity: Mutex<HashMap<String, String>>,
    /// True while the avatar is being dragged (keeps it interactive).
    dragging: AtomicBool,
    /// When the panel was last auto-hidden on blur, to debounce the tap that
    /// caused it against an immediate re-open.
    panel_last_hidden: Mutex<Option<Instant>>,
    /// Bundle id of the app that had focus when the panel was opened, so an
    /// Esc/shortcut close can hand focus back to it. None ⇒ nothing to restore.
    prev_app: Mutex<Option<String>>,
    /// Phase 4 — tool calls awaiting approval, and the channels that unblock
    /// their held hook requests (keyed by pending id).
    pub(crate) pending: Mutex<Vec<PendingRequest>>,
    pub(crate) responders: Mutex<HashMap<String, Sender<PermDecision>>>,
    /// Monotonic source of pending-request ids.
    pub(crate) perm_seq: AtomicU64,
    /// Whether the loopback approval server bound this run.
    pub(crate) perm_server_up: AtomicBool,
    /// Pending ids already notified, so each approval notifies once.
    prev_pending: Mutex<HashSet<String>>,
    /// Window-render — session id → where it lives, self-reported by the locate
    /// hook, for "jump to session".
    pub(crate) session_locations: Mutex<HashMap<String, locate::SessionLocation>>,
}

#[derive(Serialize)]
struct Point {
    x: i32,
    y: i32,
}

fn win(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    app.get_webview_window(label)
        .ok_or_else(|| format!("window '{label}' not found"))
}

// ============================ commands ============================

#[tauri::command]
fn get_snapshot(state: State<'_, AppState>) -> Snapshot {
    state.snapshot.lock().unwrap().clone()
}

#[tauri::command]
fn get_config(state: State<'_, AppState>) -> Config {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn set_theme(app: AppHandle, state: State<'_, AppState>, theme: String) {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.theme = theme.clone();
        cfg.save(&state.config_dir);
    }
    // Broadcast to every window so the panel, dashboard, and avatar switch in
    // lockstep — theme is a single app-wide setting, not per-surface.
    let _ = app.emit("theme-update", &theme);
}

/// Set the context warn threshold (percent). Persists and re-levels every
/// session immediately so the Critical group updates without a restart.
#[tauri::command]
fn set_warn_pct(app: AppHandle, state: State<'_, AppState>, pct: f64) {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.warn_pct = pct.clamp(1.0, 100.0);
        cfg.save(&state.config_dir);
    }
    recompute_and_emit(&app);
}

/// Set the early-cancel grace window (minutes). A typed prompt with no assistant
/// response and a transcript silent past this window is treated as cancelled.
/// Floored at 1 minute. Persists and re-scans so the change takes effect at once.
#[tauri::command]
fn set_cancel_grace_mins(app: AppHandle, state: State<'_, AppState>, mins: u64) {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.cancel_grace_mins = mins.max(1);
        cfg.save(&state.config_dir);
    }
    recompute_and_emit(&app);
}

/// Set how long a finished (waiting) session lingers before it clears from the
/// panel (minutes). Floored at 1. Persists and re-scans so it applies at once.
#[tauri::command]
fn set_clear_waiting_mins(app: AppHandle, state: State<'_, AppState>, mins: u64) {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.clear_waiting_mins = mins.max(1);
        cfg.save(&state.config_dir);
    }
    recompute_and_emit(&app);
}

/// Toggle whether Codex CLI sessions are watched alongside Claude Code. Persists
/// and re-scans so the change takes effect at once.
#[tauri::command]
fn set_watch_codex(app: AppHandle, state: State<'_, AppState>, enabled: bool) {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.watch_codex = enabled;
        cfg.save(&state.config_dir);
    }
    recompute_and_emit(&app);
}

/// Toggle whether Tablo follows the focused AeroSpace workspace (macOS tiling WM)
/// so the widget survives workspace switches. The follow loop reads this live on
/// its next tick — no restart needed.
#[tauri::command]
fn set_aerospace_follow(app: AppHandle, state: State<'_, AppState>, enabled: bool) {
    {
        let mut cfg = state.config.lock().unwrap();
        cfg.aerospace_follow = enabled;
        cfg.save(&state.config_dir);
    }
    recompute_and_emit(&app);
}

/// Bundle id of the frontmost app, via `lsappinfo` (in /usr/bin, no permission
/// needed). None off-macOS or on failure. Lets the panel remember who to hand
/// focus back to when it closes.
#[cfg(target_os = "macos")]
fn frontmost_bundle_id() -> Option<String> {
    let asn = std::process::Command::new("lsappinfo").arg("front").output().ok()?;
    let asn = String::from_utf8_lossy(&asn.stdout).trim().to_string();
    if asn.is_empty() {
        return None;
    }
    let out = std::process::Command::new("lsappinfo")
        .args(["info", "-only", "bundleID", &asn])
        .output()
        .ok()?;
    // Output is `"CFBundleIdentifier"="com.example.app"` → take the 4th "-field.
    String::from_utf8_lossy(&out.stdout)
        .split('"')
        .nth(3)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

#[cfg(not(target_os = "macos"))]
fn frontmost_bundle_id() -> Option<String> {
    None
}

/// Background: remember the most recent NON-Tablo frontmost app so closing the
/// panel can hand focus back even when it was opened by tapping the cat (which
/// makes Tablo frontmost before we can read who was there). Cheap poll; macOS-only
/// (elsewhere `frontmost_bundle_id` is None, so this would no-op — skip it).
#[cfg(target_os = "macos")]
fn spawn_frontmost_tracker(app: AppHandle) {
    std::thread::spawn(move || {
        let ours = app.config().identifier.clone();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            if let Some(b) = frontmost_bundle_id() {
                if b != ours {
                    if let Some(st) = app.try_state::<AppState>() {
                        *st.prev_app.lock().unwrap() = Some(b);
                    }
                }
            }
        }
    });
}

/// Bring the app with `bundle_id` to the front (best-effort, no permission).
fn activate_bundle(bundle_id: &str) {
    let _ = std::process::Command::new("open").args(["-b", bundle_id]).output();
}

/// Hide the panel and hand focus back to whatever app had it when the panel
/// opened. Used by the shortcut toggle and the Esc command — NOT by blur, where a
/// click-away has already moved focus itself.
fn hide_panel_restoring(app: &AppHandle) {
    if let Ok(panel) = win(app, "panel") {
        let _ = panel.hide();
    }
    let prev = app.state::<AppState>().prev_app.lock().unwrap().take();
    if let Some(bundle) = prev {
        activate_bundle(&bundle);
    }
}

/// Toggle the panel open/closed, anchored near the avatar. Shared by the tap
/// command and the global shortcut.
fn toggle_panel_impl(app: &AppHandle) -> Result<(), String> {
    let panel = win(app, "panel")?;
    if panel.is_visible().unwrap_or(false) {
        hide_panel_restoring(app);
        return Ok(());
    }
    // If a blur just hid the panel, this same tap is what closed it — don't
    // immediately re-open.
    let recently_hidden = app
        .state::<AppState>()
        .panel_last_hidden
        .lock()
        .unwrap()
        .map(|t| t.elapsed() < PANEL_DISMISS_GUARD)
        .unwrap_or(false);
    if recently_hidden {
        return Ok(());
    }
    // Capture who had focus so the next close can restore it. Only overwrite when
    // we read a real (non-Tablo) app: opening by tapping the cat makes Tablo
    // frontmost first, so there we keep the value the background tracker saved.
    if let Some(b) = frontmost_bundle_id() {
        if b != app.config().identifier {
            *app.state::<AppState>().prev_app.lock().unwrap() = Some(b);
        }
    }
    place_panel(app);
    panel.show().map_err(|e| e.to_string())?;
    panel.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

/// Close the panel and return focus to the previously-active app. The Esc key in
/// the panel calls this (so it matches the shortcut's close behavior).
#[tauri::command]
fn hide_panel(app: AppHandle) {
    hide_panel_restoring(&app);
}

#[tauri::command]
fn toggle_panel(app: AppHandle) -> Result<(), String> {
    toggle_panel_impl(&app)
}

/// Register or unregister the global panel hotkey to match `enabled`. Empty
/// keystroke ⇒ nothing to do. Best-effort: any stale registration is cleared
/// first so re-enabling can't double-register, and errors are ignored (a bad
/// accelerator, or a combo already owned by another app, just no-ops).
fn apply_panel_shortcut(app: &AppHandle, enabled: bool, shortcut: &str) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    if shortcut.is_empty() {
        return;
    }
    let gs = app.global_shortcut();
    let _ = gs.unregister(shortcut);
    if enabled {
        let _ = gs.on_shortcut(shortcut, |app, _s, event| {
            if event.state == ShortcutState::Pressed {
                let _ = toggle_panel_impl(app);
            }
        });
    }
}

/// Enable/disable the global panel hotkey. Persists and applies live, then
/// re-emits so the Settings toggle reflects it across windows.
#[tauri::command]
fn set_panel_shortcut_enabled(app: AppHandle, state: State<'_, AppState>, enabled: bool) {
    let shortcut = {
        let mut cfg = state.config.lock().unwrap();
        cfg.panel_shortcut_enabled = enabled;
        cfg.save(&state.config_dir);
        cfg.panel_shortcut.clone()
    };
    apply_panel_shortcut(&app, enabled, &shortcut);
    recompute_and_emit(&app);
}

/// macOS: control whether Tablo shows in the Dock + Cmd+Tab app switcher.
/// `Accessory` hides it (a pure floating widget); `Regular` presents it like a
/// normal app. We flip to `Regular` only while the dashboard is open, so the
/// avatar and panel alone never clutter the switcher. No-op off macOS.
fn set_switcher_visible(app: &AppHandle, visible: bool) {
    #[cfg(target_os = "macos")]
    {
        let policy = if visible {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        };
        let _ = app.set_activation_policy(policy);
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, visible);
}

/// Hide the dashboard (kept alive so it can reopen) and drop Tablo back to a
/// switcher-hidden widget. Shared by the Esc command and the OS close button.
fn hide_dashboard_impl(app: &AppHandle) {
    if let Some(dash) = app.get_webview_window("dashboard") {
        let _ = dash.hide();
    }
    set_switcher_visible(app, false);
}

/// Route the dashboard's OS close button to a hide (not destroy) + switcher reset.
fn install_dashboard_close(app: &AppHandle, window: &WebviewWindow) {
    let h = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            hide_dashboard_impl(&h);
        }
    });
}

#[tauri::command]
fn open_dashboard(app: AppHandle) -> Result<(), String> {
    // Becoming a Regular app puts Tablo in the Dock + Cmd+Tab while the dashboard
    // is up; hiding it (Esc / close button) flips back to Accessory.
    set_switcher_visible(&app, true);
    // Recreate the window if it was ever destroyed (belt-and-suspenders — the
    // close handler normally hides it instead).
    let dash = match app.get_webview_window("dashboard") {
        Some(w) => w,
        None => {
            let w = tauri::WebviewWindowBuilder::new(&app, "dashboard", tauri::WebviewUrl::default())
                // Distinct from the widget windows so the AeroSpace follow loop
                // can leave the dashboard under normal tiling (see `aerospace`).
                .title("tablo dashboard")
                .inner_size(980.0, 720.0)
                .min_inner_size(640.0, 480.0)
                .resizable(true)
                .visible(false)
                .build()
                .map_err(|e| e.to_string())?;
            install_dashboard_close(&app, &w);
            w
        }
    };
    let _ = dash.unminimize();
    dash.show().map_err(|e| e.to_string())?;
    dash.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

/// Hide the dashboard from the frontend (Esc). Also returns Tablo to a
/// switcher-hidden widget.
#[tauri::command]
fn hide_dashboard(app: AppHandle) {
    hide_dashboard_impl(&app);
}

/// Begin an avatar drag: pin it interactive and report its current logical
/// top-left so the frontend can compute deltas from the pointer.
#[tauri::command]
fn begin_drag(app: AppHandle, state: State<'_, AppState>) -> Result<Point, String> {
    state.dragging.store(true, Ordering::Relaxed);
    let avatar = win(&app, "avatar")?;
    let pos = avatar.outer_position().map_err(|e| e.to_string())?;
    let sf = avatar.scale_factor().unwrap_or(1.0);
    Ok(Point {
        x: (pos.x as f64 / sf).round() as i32,
        y: (pos.y as f64 / sf).round() as i32,
    })
}

#[tauri::command]
fn move_avatar(app: AppHandle, x: i32, y: i32) -> Result<(), String> {
    let avatar = win(&app, "avatar")?;
    avatar
        .set_position(LogicalPosition::new(x as f64, y as f64))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn end_drag(app: AppHandle, state: State<'_, AppState>, x: i32, y: i32) -> Result<(), String> {
    state.dragging.store(false, Ordering::Relaxed);
    let mut cfg = state.config.lock().unwrap();
    cfg.avatar_x = Some(x);
    cfg.avatar_y = Some(y);
    cfg.save(&state.config_dir);
    // Position was already applied live during the drag; nothing else to do.
    let _ = app;
    Ok(())
}

// ============================ window placement ============================

fn position_avatar(avatar: &WebviewWindow, cfg: &Config) {
    if let (Some(x), Some(y)) = (cfg.avatar_x, cfg.avatar_y) {
        let _ = avatar.set_position(LogicalPosition::new(x as f64, y as f64));
        return;
    }
    // Default: tucked into the bottom-right of the current monitor.
    if let Ok(Some(mon)) = avatar.current_monitor() {
        let sf = mon.scale_factor();
        let msz = mon.size();
        let mpos = mon.position();
        let asz = avatar.outer_size().unwrap_or(tauri::PhysicalSize::new(126, 134));
        let margin = (24.0 * sf) as i32;
        let x = mpos.x + msz.width as i32 - asz.width as i32 - margin;
        let y = mpos.y + msz.height as i32 - asz.height as i32 - margin * 2;
        let _ = avatar.set_position(PhysicalPosition::new(x, y));
    }
}

/// Anchor the panel just left of (or right of, if clipped) the avatar, bottoms
/// aligned, clamped to the monitor.
fn place_panel(app: &AppHandle) {
    let (avatar, panel) = match (app.get_webview_window("avatar"), app.get_webview_window("panel")) {
        (Some(a), Some(p)) => (a, p),
        _ => return,
    };
    let ap = avatar.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let asz = avatar.outer_size().unwrap_or(tauri::PhysicalSize::new(126, 134));
    let sf = avatar.scale_factor().unwrap_or(1.0);
    let mut psz = panel.outer_size().unwrap_or(tauri::PhysicalSize::new(0, 0));
    if psz.width == 0 || psz.height == 0 {
        psz = tauri::PhysicalSize::new((360.0 * sf) as u32, (540.0 * sf) as u32);
    }
    let gap = (12.0 * sf) as i32;

    let mut px = ap.x - gap - psz.width as i32;
    let mut py = ap.y + asz.height as i32 - psz.height as i32;

    if let Ok(Some(mon)) = avatar.current_monitor() {
        let mpos = mon.position();
        let msz = mon.size();
        if px < mpos.x {
            px = ap.x + asz.width as i32 + gap; // flip to the right
        }
        let max_x = (mpos.x + msz.width as i32 - psz.width as i32).max(mpos.x);
        let max_y = (mpos.y + msz.height as i32 - psz.height as i32).max(mpos.y);
        px = px.clamp(mpos.x, max_x);
        py = py.clamp(mpos.y, max_y);
    }
    let _ = panel.set_position(PhysicalPosition::new(px, py));
}

/// Anchor the toast just left of (or right of, if clipped) the avatar, vertically
/// centered on it, clamped to the monitor. Mirrors `place_panel`.
fn place_toast(app: &AppHandle) {
    let (avatar, toast) = match (app.get_webview_window("avatar"), app.get_webview_window("toast")) {
        (Some(a), Some(t)) => (a, t),
        _ => return,
    };
    let ap = avatar.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let asz = avatar.outer_size().unwrap_or(tauri::PhysicalSize::new(126, 134));
    let sf = avatar.scale_factor().unwrap_or(1.0);
    let mut tsz = toast.outer_size().unwrap_or(tauri::PhysicalSize::new(0, 0));
    if tsz.width == 0 || tsz.height == 0 {
        tsz = tauri::PhysicalSize::new((300.0 * sf) as u32, (84.0 * sf) as u32);
    }
    // Anchor the toast just OUTSIDE the avatar's circular cursor hit-test. That
    // radius (see `cursor_over_cat`) flips the avatar interactive when the cursor
    // is near the cat — and since the toast's right-aligned "jump" button sits on
    // the side nearest the cat, tucking it into the avatar's margin let the avatar
    // steal the button's clicks (the "jump works only sometimes" bug). Placing the
    // toast's right edge a clearance past the steal-zone keeps the button clickable
    // and stops it overlapping the sprite. Flip to the right if the left runs off.
    let r = (asz.width.min(asz.height) as f64 * HIT_RADIUS_FRACTION) as i32;
    let cx = ap.x + asz.width as i32 / 2;
    let clearance = (8.0 * sf) as i32;

    let mut tx = cx - r - clearance - tsz.width as i32; // toast right edge left of the steal-zone
    let mut ty = ap.y + (asz.height as i32 - tsz.height as i32) / 2;

    if let Ok(Some(mon)) = avatar.current_monitor() {
        let mpos = mon.position();
        let msz = mon.size();
        if tx < mpos.x {
            tx = cx + r + clearance; // flip to the right, just past the steal-zone
        }
        let max_x = (mpos.x + msz.width as i32 - tsz.width as i32).max(mpos.x);
        let max_y = (mpos.y + msz.height as i32 - tsz.height as i32).max(mpos.y);
        tx = tx.clamp(mpos.x, max_x);
        ty = ty.clamp(mpos.y, max_y);
    }
    let _ = toast.set_position(PhysicalPosition::new(tx, ty));
}

/// Position the toast next to the avatar and reveal it (non-activating).
#[tauri::command]
fn show_toast(app: AppHandle) -> Result<(), String> {
    place_toast(&app);
    if let Some(t) = app.get_webview_window("toast") {
        t.show().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Hide the toast once its dismiss animation has finished.
#[tauri::command]
fn hide_toast(app: AppHandle) -> Result<(), String> {
    if let Some(t) = app.get_webview_window("toast") {
        t.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ============================ background loops ============================

/// One scan → attach pending → notify → emit cycle. Called both by the watcher
/// loop and by the permission threads (so approvals surface instantly), hence
/// `pub(crate)`.
pub(crate) fn recompute_and_emit(app: &AppHandle) {
    let state = app.state::<AppState>();
    let cfg = state.config.lock().unwrap().clone();

    let mut snap = {
        let mut files = state.files.lock().unwrap();
        let mut claude_cfg = state.claude_cfg.lock().unwrap();
        scanner::scan(&cfg, &mut files, &mut claude_cfg)
    };

    // Overlay pending approvals: they drive the "waiting" count and force the
    // avatar to its attention (alarmed) state regardless of context fill.
    let pending = state.pending.lock().unwrap().clone();
    if !pending.is_empty() {
        snap.waiting = pending.len() as u32;
        snap.state = "alarmed".into();
    }
    snap.pending = pending;

    // Overlay "jump to session" availability from the location cache — but only
    // while the relevant locate hook is enabled, so a disabled jump setting hides
    // every jump affordance (even for sessions that reported before it was turned
    // off). Gated per source: Claude sessions on the Claude locate hook, Codex
    // sessions on the Codex one — each is independent.
    {
        let claude_on = locate::is_installed();
        let codex_on = codex_locate::is_installed();
        let locs = state.session_locations.lock().unwrap();
        for s in &mut snap.sessions {
            let on = if s.source == "codex" { codex_on } else { claude_on };
            s.can_jump = on && locs.contains_key(&s.id);
        }
    }

    fire_notifications(app, &state, &cfg, &snap);
    fire_permission_notifications(app, &state, &cfg, &snap);
    fire_waiting_events(app, &state, &snap);

    let changed = {
        let mut cur = state.snapshot.lock().unwrap();
        let differs = signature(&cur) != signature(&snap);
        *cur = snap.clone();
        differs
    };
    if changed {
        let _ = app.emit("state-update", &snap);
    }
}

/// Change signature that ignores the wall-clock timestamp.
fn signature(s: &Snapshot) -> String {
    let mut c = s.clone();
    c.generated_at = 0;
    serde_json::to_string(&c).unwrap_or_default()
}

fn fire_notifications(app: &AppHandle, state: &State<'_, AppState>, cfg: &Config, snap: &Snapshot) {
    use tauri_plugin_notification::NotificationExt;
    let mut prev = state.prev_levels.lock().unwrap();
    let mut next = HashMap::new();
    for s in &snap.sessions {
        let old = prev.get(&s.id).map(|x| x.as_str()).unwrap_or("ok");
        if cfg.notify_on_warn && old == "ok" && s.level != "ok" {
            let _ = app
                .notification()
                .builder()
                .title("tablo — context warning")
                .body(format!("{} crossed {:.0}% context", s.project, s.pct))
                .show();
        }
        next.insert(s.id.clone(), s.level.clone());
    }
    *prev = next;
}

/// Emit `session-waiting` (with the project names) when a session finishes
/// working and starts waiting on the user. The toast and the avatar both listen
/// and decide whether to react per the frontend `notifyOnWaiting` preference. A
/// session with a pending permission request is tracked as "perm" so approval
/// flows never masquerade as a work→wait crossing.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct WaitingSession {
    id: String,
    project: String,
    title: Option<String>,
    can_jump: bool,
}

fn fire_waiting_events(app: &AppHandle, state: &State<'_, AppState>, snap: &Snapshot) {
    let pending_ids: HashSet<&str> = snap.pending.iter().map(|p| p.session_id.as_str()).collect();
    let mut prev = state.prev_activity.lock().unwrap();
    let mut arrived: Vec<WaitingSession> = Vec::new();
    let mut next = HashMap::new();
    for s in &snap.sessions {
        let kind = if pending_ids.contains(s.id.as_str()) {
            "perm"
        } else {
            s.activity_kind.as_str()
        };
        if kind == "waiting" {
            if let Some(p) = prev.get(&s.id) {
                if p == "working" || p == "thinking" {
                    arrived.push(WaitingSession {
                        id: s.id.clone(),
                        project: s.project.clone(),
                        title: s.title.clone(),
                        can_jump: s.can_jump,
                    });
                }
            }
        }
        next.insert(s.id.clone(), kind.to_string());
    }
    *prev = next;
    if !arrived.is_empty() {
        let _ = app.emit("session-waiting", &arrived);
    }
}

/// Notify once per newly-arrived approval request (Phase 4).
fn fire_permission_notifications(
    app: &AppHandle,
    state: &State<'_, AppState>,
    cfg: &Config,
    snap: &Snapshot,
) {
    use tauri_plugin_notification::NotificationExt;
    let mut prev = state.prev_pending.lock().unwrap();
    if cfg.notify_on_permission {
        for p in &snap.pending {
            if !prev.contains(&p.id) {
                let body = if p.detail.is_empty() {
                    p.project.clone()
                } else {
                    format!("{} · {}", p.project, p.detail)
                };
                let _ = app
                    .notification()
                    .builder()
                    .title(format!("tablo — approve {}?", p.tool))
                    .body(body)
                    .show();
            }
        }
    }
    *prev = snap.pending.iter().map(|p| p.id.clone()).collect();
}

/// Tail the transcripts. Uses `notify` for responsiveness and a 1s timeout so
/// sessions still age to idle without new writes.
fn spawn_watcher(app: AppHandle) {
    use notify::Watcher;
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .ok();
        if let (Some(w), Some(dir)) = (watcher.as_mut(), scanner::projects_dir()) {
            let _ = w.watch(&dir, notify::RecursiveMode::Recursive);
        }

        recompute_and_emit(&app); // initial paint
        loop {
            match rx.recv_timeout(SCAN_INTERVAL) {
                Ok(_) => {
                    // Coalesce a burst of file events.
                    while rx.recv_timeout(EVENT_DEBOUNCE).is_ok() {}
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    // Watcher unavailable — fall back to plain polling.
                    std::thread::sleep(SCAN_INTERVAL);
                }
            }
            recompute_and_emit(&app);
        }
    });
}

/// Keep only the cat interactive: poll the cursor and toggle click-through.
fn spawn_hittest(app: AppHandle) {
    std::thread::spawn(move || {
        let mut interactive = false; // window starts click-through
        loop {
            std::thread::sleep(CURSOR_POLL);
            let avatar = match app.get_webview_window("avatar") {
                Some(w) => w,
                None => continue,
            };
            let dragging = app.state::<AppState>().dragging.load(Ordering::Relaxed);
            let want = dragging || cursor_over_cat(&app, &avatar).unwrap_or(false);
            if want != interactive {
                let _ = avatar.set_ignore_cursor_events(!want);
                interactive = want;
            }
        }
    });
}

fn cursor_over_cat(app: &AppHandle, avatar: &WebviewWindow) -> Option<bool> {
    let cur = app.cursor_position().ok()?;
    let pos = avatar.outer_position().ok()?;
    let sz = avatar.outer_size().ok()?;
    let cx = pos.x as f64 + sz.width as f64 / 2.0;
    let cy = pos.y as f64 + sz.height as f64 / 2.0;
    let dx = cur.x - cx;
    let dy = cur.y - cy;
    // Interactive radius derived from the actual window size (not a fixed pixel
    // count), so it tracks whatever avatar dimensions tauri.conf.json declares.
    let r = sz.width.min(sz.height) as f64 * HIT_RADIUS_FRACTION;
    Some(dx * dx + dy * dy <= r * r)
}

// ============================ tray ============================

/// Menu-bar tray for the Accessory app: show/hide the widget, open the dashboard,
/// quit. The `toggle` item is cloned into the handler so its label can flip.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::TrayIconBuilder;

    let toggle = MenuItem::with_id(app, "toggle_widget", "Hide widget", true, None::<&str>)?;
    let dashboard = MenuItem::with_id(app, "dashboard", "Dashboard", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit tablo", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&toggle, &dashboard, &sep, &quit])?;

    let toggle_item = toggle.clone();
    let mut builder = TrayIconBuilder::with_id("tablo-tray")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("tablo")
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "toggle_widget" => toggle_widget(app, &toggle_item),
            "dashboard" => {
                let _ = open_dashboard(app.clone());
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Toggle the avatar (tucking away the panel/toast when hiding), keeping the tray
/// label in sync with the result.
fn toggle_widget(app: &AppHandle, item: &tauri::menu::MenuItem<tauri::Wry>) {
    let Some(avatar) = app.get_webview_window("avatar") else { return };
    if avatar.is_visible().unwrap_or(true) {
        let _ = avatar.hide();
        if let Some(p) = app.get_webview_window("panel") { let _ = p.hide(); }
        if let Some(t) = app.get_webview_window("toast") { let _ = t.hide(); }
        let _ = item.set_text("Show widget");
    } else {
        let _ = avatar.show();
        let _ = item.set_text("Hide widget");
    }
}

// ============================ auto-update ============================

/// How often to poll for a new release after the initial post-launch check.
const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Check GitHub for a newer release; if one exists, download, install, and
/// relaunch. Silent + best-effort — offline, no release yet, or a signature
/// mismatch all just no-op. Skipped while an approval is pending so an update
/// never interrupts a decision in flight.
async fn try_update(app: AppHandle) {
    use tauri_plugin_updater::UpdaterExt;
    if !app.state::<AppState>().pending.lock().unwrap().is_empty() {
        return;
    }
    let update = match app.updater() {
        Ok(u) => match u.check().await {
            Ok(Some(update)) => update,
            _ => return, // up to date, unreachable, or no release published yet
        },
        Err(_) => return,
    };
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title("tablo is updating")
        .body(format!("Installing v{}…", update.version))
        .show();
    if update.download_and_install(|_, _| {}, || {}).await.is_ok() {
        app.restart();
    }
}

/// Poll for updates: once shortly after launch, then on a long interval.
fn spawn_updater(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(8));
        loop {
            tauri::async_runtime::block_on(try_update(app.clone()));
            std::thread::sleep(UPDATE_INTERVAL);
        }
    });
}

// ============================ entrypoint ============================

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();

            // Start as a switcher-hidden widget: no Dock icon, no Cmd+Tab entry.
            // Opening the dashboard promotes Tablo to a Regular app (see
            // open_dashboard); hiding it drops back here.
            set_switcher_visible(&handle, false);

            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("tablo"));
            let mut cfg = Config::load(&config_dir);
            // Capture hook params before `cfg` is moved into managed state.
            let (hook_port, hook_timeout) = (cfg.permission_port, cfg.hook_timeout_secs);
            let panel_shortcut = cfg.panel_shortcut.clone();
            let panel_shortcut_enabled = cfg.panel_shortcut_enabled;

            if let Some(avatar) = app.get_webview_window("avatar") {
                let _ = avatar.set_ignore_cursor_events(true);
                position_avatar(&avatar, &cfg);
            }


            // Panel dismisses itself when it loses focus (tap-away).
            if let Some(panel) = app.get_webview_window("panel") {
                let h = handle.clone();
                panel.on_window_event(move |event| {
                    if let WindowEvent::Focused(focused) = event {
                        if !*focused {
                            if let Some(p) = h.get_webview_window("panel") {
                                let _ = p.hide();
                                if let Some(st) = h.try_state::<AppState>() {
                                    *st.panel_last_hidden.lock().unwrap() = Some(Instant::now());
                                }
                            }
                        }
                    }
                });
            }

            // Dashboard hides (not destroys) on close so it can reopen, and
            // resets Tablo to a switcher-hidden widget.
            if let Some(dash) = app.get_webview_window("dashboard") {
                install_dashboard_close(&handle, &dash);
            }

            // "Jump to session" is on by default: wire the locate hook into
            // ~/.claude/settings.json on the first launch only. The guard flag
            // means a later user-disable sticks — we never re-enable behind them.
            if !cfg.locate_default_applied {
                let _ = locate::install(&cfg);
                cfg.locate_default_applied = true;
                cfg.save(&config_dir);
            }

            app.manage(AppState {
                config: Mutex::new(cfg),
                config_dir,
                snapshot: Mutex::new(Snapshot::default()),
                files: Mutex::new(HashMap::new()),
                claude_cfg: Mutex::new(ClaudeConfigCache::default()),
                prev_levels: Mutex::new(HashMap::new()),
                prev_activity: Mutex::new(HashMap::new()),
                dragging: AtomicBool::new(false),
                panel_last_hidden: Mutex::new(None),
                prev_app: Mutex::new(None),
                pending: Mutex::new(Vec::new()),
                responders: Mutex::new(HashMap::new()),
                perm_seq: AtomicU64::new(0),
                perm_server_up: AtomicBool::new(false),
                prev_pending: Mutex::new(HashSet::new()),
                session_locations: Mutex::new(HashMap::new()),
            });

            // Keep the hook script in sync with the current port/timeout (safe,
            // idempotent — it's Tablo's own file). Enabling approvals, which
            // edits ~/.claude/settings.json, stays an explicit UI action.
            let _ = permission::write_hook_script(hook_port, hook_timeout);
            // Locate hook scripts are written too (harmless, Tablo's own files);
            // wiring them into the agents' hook configs stays an explicit UI
            // action. Claude → ~/.claude/settings.json; Codex → ~/.codex/hooks.json.
            let _ = locate::write_locate_script(hook_port);
            let _ = codex_locate::write_locate_script(hook_port);
            permission::spawn_server(handle.clone());

            build_tray(&handle)?;

            // Global hotkey to summon the panel from anywhere, without touching
            // the widget. Config-driven + toggleable in Settings.
            apply_panel_shortcut(&handle, panel_shortcut_enabled, &panel_shortcut);

            // Poll GitHub Releases for updates and self-install them.
            spawn_updater(handle.clone());

            // Track the frontmost app so closing the panel can restore focus to it.
            #[cfg(target_os = "macos")]
            spawn_frontmost_tracker(handle.clone());

            // Follow the focused AeroSpace workspace so the widget survives
            // workspace switches (AeroSpace ignores `visibleOnAllWorkspaces`).
            // No-ops off AeroSpace; macOS-only (AeroSpace doesn't exist elsewhere).
            #[cfg(target_os = "macos")]
            aerospace::spawn(handle.clone());

            spawn_watcher(handle.clone());
            spawn_hittest(handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            get_config,
            set_theme,
            set_warn_pct,
            set_cancel_grace_mins,
            set_clear_waiting_mins,
            set_watch_codex,
            set_aerospace_follow,
            set_panel_shortcut_enabled,
            toggle_panel,
            hide_panel,
            open_dashboard,
            hide_dashboard,
            show_toast,
            hide_toast,
            begin_drag,
            move_avatar,
            end_drag,
            permission::resolve_permission,
            permission::hook_status,
            permission::set_hook_enabled,
            locate::jump_to_session,
            locate::locate_status,
            locate::set_locate_enabled,
            codex_locate::codex_locate_status,
            codex_locate::set_codex_locate_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
