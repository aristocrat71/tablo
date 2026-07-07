//! "Jump to session": focus the terminal a Claude Code session runs in.
//!
//! A session can't be mapped to its OS window from the transcript alone, so it
//! *self-reports* where it lives: a passive `SessionStart`/`UserPromptSubmit`
//! hook (`locate.sh`) POSTs its tmux pane + tty to the loopback server, keyed by
//! session id. `jump_to_session` then switches tmux to that pane and raises the
//! host app. Unlike approvals it never blocks a tool.

use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};

use crate::config::Config;
use crate::permission;
use crate::AppState;

/// Where a session lives, as reported by the locate hook (all best-effort). Only
/// the non-tmux path reads `term_program`/`zed_term`; inside tmux they're
/// server-inherited and unreliable, so we resolve the host from the process tree.
#[derive(Clone, Debug, Default)]
pub struct SessionLocation {
    /// `$TMUX` = "<socket>,<pid>,<session>"; the socket is what we target.
    pub tmux: String,
    /// `$TMUX_PANE`, e.g. "%22" — the exact pane, unambiguous across windows.
    pub tmux_pane: String,
    /// `$TERM_PROGRAM` (masked to "tmux" inside tmux).
    pub term_program: String,
    /// `$ZED_TERM` == "true" when hosted inside the Zed editor.
    pub zed_term: String,
    /// Controlling terminal, e.g. "/dev/ttys017" — for a direct terminal tab
    /// this is the tab's own tty, which lets us focus that exact tab.
    pub tty: String,
}

/// The hook's POST body.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocatePayload {
    #[serde(default)]
    session_id: String,
    #[serde(default)]
    tmux: String,
    #[serde(default)]
    tmux_pane: String,
    #[serde(default)]
    term_program: String,
    #[serde(default)]
    zed_term: String,
    #[serde(default)]
    tty: String,
}

/// Handle a `POST /locate` body from the hook: cache session -> location. Emits
/// only when a session's jump-ability first becomes known (so a card can gain
/// its button) — steady-state re-reports don't churn the UI.
pub fn store_location(app: &AppHandle, body: &str) {
    let p: LocatePayload = match serde_json::from_str(body) {
        Ok(p) => p,
        Err(_) => return,
    };
    if p.session_id.is_empty() {
        return;
    }
    let loc = SessionLocation {
        tmux: p.tmux,
        tmux_pane: p.tmux_pane,
        term_program: p.term_program,
        zed_term: p.zed_term,
        tty: p.tty,
    };
    let state = app.state::<AppState>();
    let is_new = {
        let mut map = state.session_locations.lock().unwrap();
        map.insert(p.session_id.clone(), loc).is_none()
    };
    if is_new {
        crate::recompute_and_emit(app);
    }
}

// ============================ jump ============================

/// Focus the window a session lives in. Best-effort: brings the tmux pane to the
/// foreground and activates the host GUI app; returns a short description of what
/// it did, or an error if nothing was locatable.
#[tauri::command]
pub fn jump_to_session(state: State<'_, AppState>, session_id: String) -> Result<String, String> {
    let loc = state
        .session_locations
        .lock()
        .unwrap()
        .get(&session_id)
        .cloned()
        .ok_or_else(|| "no known location for this session yet".to_string())?;
    focus(&loc)
}

/// Bring a session's window to the front. Two layers: a portable tmux pane-switch
/// (every OS, no permission), then a platform-gated GUI raise (macOS today).
/// Ok(what landed) or Err if nothing could be located.
fn focus(loc: &SessionLocation) -> Result<String, String> {
    let socket = tmux_socket(&loc.tmux);
    // The terminal the user is in = the most-recently-active tmux client. We move
    // *this* client onto the target and raise *its* app, so the switch and the
    // focus stay coherent even across sessions.
    let client = if loc.tmux_pane.is_empty() {
        None
    } else {
        most_recent_client(&socket)
    };

    let mut done: Vec<String> = Vec::new();
    if !loc.tmux_pane.is_empty() {
        let ctty = client.as_ref().map(|(t, _)| t.as_str());
        if tmux_focus_pane(&socket, &loc.tmux_pane, ctty).is_ok() {
            done.push(format!("tmux {}", loc.tmux_pane));
        }
    }
    if let Some(what) = raise_window(&socket, loc, client.as_ref()) {
        done.push(what);
    }

    if done.is_empty() {
        Err("couldn't locate this session's window".into())
    } else {
        Ok(done.join(", "))
    }
}

/// Portable tmux pane-switch: bring `pane` in front of the user. The one piece of
/// "jump" that behaves identically across OSes — no GUI focus, no permission.
/// - `select-window`/`select-pane` make the pane current *within its session*.
/// - `switch-client -c <client> -t <session>` then moves the user's own client
///   onto that session, so it works even when they were viewing a *different*
///   tmux session. Crucially the switch target is the session **name**, not the
///   pane id (a pane isn't a valid `switch-client` target); the preceding
///   `select-window` has already made `pane`'s window current in that session.
fn tmux_focus_pane(socket: &Option<String>, pane: &str, client: Option<&str>) -> Result<(), String> {
    let mut ok = false;
    let mut err = String::new();
    for args in [["select-window", "-t", pane], ["select-pane", "-t", pane]] {
        match run_tmux(socket, &args) {
            Ok(_) => ok = true,
            Err(e) => err = e,
        }
    }
    let session = run_tmux(socket, &["display-message", "-p", "-t", pane, "#{session_name}"])
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if !session.is_empty() {
        let mut args: Vec<&str> = vec!["switch-client"];
        if let Some(c) = client {
            args.push("-c");
            args.push(c);
        }
        args.push("-t");
        args.push(&session);
        match run_tmux(socket, &args) {
            Ok(_) => ok = true,
            Err(e) => err = e,
        }
    }
    if ok {
        Ok(())
    } else if err.is_empty() {
        Err("no output".into())
    } else {
        Err(err)
    }
}

/// The most-recently-active tmux client across all sessions — the terminal the
/// user is currently (or was last) looking at. Returns its `(tty, pid)`: the tty
/// targets `switch-client -c` and AppleScript tab-matching, the pid lets us walk
/// the process tree to that client's real host app.
fn most_recent_client(socket: &Option<String>) -> Option<(String, i32)> {
    let mut rows: Vec<(i64, String, i32)> = run_tmux(
        socket,
        &["list-clients", "-F", "#{client_activity}\t#{client_tty}\t#{client_pid}"],
    )
    .unwrap_or_default()
    .lines()
    .filter_map(|l| {
        let mut it = l.splitn(3, '\t');
        let act = it.next()?.trim().parse::<i64>().unwrap_or(0);
        let tty = it.next()?.trim().to_string();
        let pid = it.next()?.trim().parse::<i32>().ok()?;
        (!tty.is_empty()).then_some((act, tty, pid))
    })
    .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    rows.into_iter().next().map(|(_, tty, pid)| (tty, pid))
}

/// macOS raise: bring the user's current terminal to the front, returning what we
/// focused. The tty a terminal app knows is the tab's own pty — for a non-tmux
/// session that's `loc.tty`; inside tmux it's the *client* tty (the outer tab).
/// Try an exact-tab focus (Terminal.app / iTerm2) first; otherwise resolve the
/// host app from the client's process tree and activate it.
#[cfg(target_os = "macos")]
fn raise_window(_socket: &Option<String>, loc: &SessionLocation, client: Option<&(String, i32)>) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(t) = valid_tty(&loc.tty) {
        candidates.push(t);
    }
    if let Some((tty, _)) = client {
        if let Some(v) = valid_tty(tty) {
            if !candidates.contains(&v) {
                candidates.push(v);
            }
        }
    }

    // Exact-tab focus against terminals that are actually running (scripting a
    // non-running app would launch it). A permission failure reads as "not found"
    // and falls through to app activation below.
    let kinds: Vec<TermKind> = [TermKind::Terminal, TermKind::ITerm]
        .into_iter()
        .filter(|k| term_app_running(*k))
        .collect();
    for tty in &candidates {
        for &kind in &kinds {
            if applescript_focus_tab(kind, tty) {
                return Some(format!("tab {} {tty}", kind.app()));
            }
        }
    }

    // Otherwise raise the host app (can't pick the exact tab). Resolve it from the
    // client's process tree first — env-var-proof, and it finds hosts that aren't
    // tab-scriptable (Zed, Ghostty). Only then the non-tmux env mapping.
    let app = client.and_then(|(_, pid)| walk_to_app(*pid)).or_else(|| gui_app(loc))?;
    // AppKit activation needs no prompt and wins cooperative activation; `open -a`
    // is the fallback to launch the app if it isn't running.
    let raised = activate_app(&app)
        || Command::new("open").arg("-a").arg(&app).output().map(|o| o.status.success()).unwrap_or(false);
    raised.then(|| format!("app {app}"))
}

/// Non-macOS raise: not yet implemented. Linux (wmctrl/xdotool on X11) and Windows
/// (SetForegroundWindow) go here; Wayland forbids external activation so it stays
/// a no-op. The portable tmux pane-switch has already run regardless.
#[cfg(not(target_os = "macos"))]
fn raise_window(_socket: &Option<String>, _loc: &SessionLocation, _client: Option<&(String, i32)>) -> Option<String> {
    None
}

/// Run a tmux command on the session's socket; Ok(stdout) / Err(stderr|ioerr).
fn run_tmux(socket: &Option<String>, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("tmux");
    if let Some(s) = socket {
        cmd.arg("-S").arg(s);
    }
    let out = cmd.args(args).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Walk a process's ancestry to the first recognizable terminal/editor GUI app.
#[cfg(target_os = "macos")]
fn walk_to_app(start: i32) -> Option<String> {
    let mut pid = start;
    for _ in 0..12 {
        let (ppid, comm) = ps_parent_comm(pid)?;
        if let Some(app) = app_from_comm(&comm) {
            return Some(app);
        }
        if ppid <= 1 {
            break;
        }
        pid = ppid;
    }
    None
}

/// `(ppid, comm)` for a pid via `ps`; comm is the full executable path on macOS.
#[cfg(target_os = "macos")]
fn ps_parent_comm(pid: i32) -> Option<(i32, String)> {
    let out = Command::new("ps")
        .args(["-o", "ppid=,comm=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let binding = String::from_utf8_lossy(&out.stdout);
    let line = binding.trim();
    let cut = line.find(char::is_whitespace)?;
    let ppid: i32 = line[..cut].trim().parse().ok()?;
    Some((ppid, line[cut..].trim().to_string()))
}

/// Map a process's executable path/name to an `open -a` app name. Prefers the
/// `.app` bundle name from a path (".../Zed.app/Contents/MacOS/zed" → "Zed");
/// falls back to a bare-name lookup for the common terminals.
#[cfg(target_os = "macos")]
fn app_from_comm(comm: &str) -> Option<String> {
    if let Some(idx) = comm.find(".app/") {
        if let Some(name) = comm[..idx].rsplit('/').next() {
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    let base = comm.rsplit('/').next().unwrap_or(comm).to_ascii_lowercase();
    let app = match base.trim_start_matches('-') {
        "zed" => "Zed",
        "terminal" => "Terminal",
        "iterm2" | "iterm" => "iTerm",
        "ghostty" => "Ghostty",
        "wezterm-gui" | "wezterm" => "WezTerm",
        "kitty" => "kitty",
        "alacritty" => "Alacritty",
        "hyper" => "Hyper",
        "tabby" => "Tabby",
        _ => return None,
    };
    Some(app.to_string())
}

/// Bring a running app to the front by name, with **no Automation prompt**, via
/// AppKit's `NSRunningApplication.activate`. This does what `open -a` often won't
/// from a foreground caller: macOS cooperative activation blocks a plain `open`
/// when the caller (Tablo) is the active app, so the target only bounces in the
/// Dock. Returns false if the app isn't running (so the caller can `open` it).
#[cfg(target_os = "macos")]
#[allow(deprecated)] // activateWithOptions: is the deployment-portable activator
fn activate_app(name: &str) -> bool {
    use objc2_app_kit::{NSApplicationActivationOptions, NSWorkspace};
    let ws = NSWorkspace::sharedWorkspace();
    let apps = ws.runningApplications();
    let count = apps.count();
    for i in 0..count {
        let app = apps.objectAtIndex(i);
        if let Some(ln) = app.localizedName() {
            if ln.to_string() == name {
                return app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows);
            }
        }
    }
    false
}

/// Whether a scriptable terminal is running (so we don't launch it by scripting).
#[cfg(target_os = "macos")]
fn term_app_running(kind: TermKind) -> bool {
    let proc = match kind {
        TermKind::Terminal => "Terminal",
        TermKind::ITerm => "iTerm2",
    };
    Command::new("pgrep")
        .arg("-x")
        .arg(proc)
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum TermKind {
    Terminal,
    ITerm,
}

#[cfg(target_os = "macos")]
impl TermKind {
    fn app(self) -> &'static str {
        match self {
            TermKind::Terminal => "Terminal",
            TermKind::ITerm => "iTerm",
        }
    }
}

/// Accept only a real `/dev/tty…` path — guards the AppleScript interpolation
/// against anything unexpected in the reported tty.
#[cfg(target_os = "macos")]
fn valid_tty(tty: &str) -> Option<String> {
    let t = tty.trim();
    let ok = t.starts_with("/dev/tty")
        && t.len() > 8
        && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'/');
    if ok {
        Some(t.to_string())
    } else {
        None
    }
}

/// Select the tab whose tty matches and bring its window forward via AppleScript.
/// true = focused; false = not found here or scripting failed (e.g. missing
/// Automation permission), in which case the caller falls back to app activation.
#[cfg(target_os = "macos")]
fn applescript_focus_tab(kind: TermKind, tty: &str) -> bool {
    let script = match kind {
        TermKind::Terminal => format!(
            r#"tell application "Terminal"
	set res to "notfound"
	repeat with w in windows
		repeat with t in tabs of w
			try
				if (tty of t) is "{tty}" then
					set selected of t to true
					set res to "found"
					activate
					try
						set frontmost of w to true
					end try
					try
						set index of w to 1
					end try
				end if
			end try
		end repeat
	end repeat
	return res
end tell"#
        ),
        TermKind::ITerm => format!(
            r#"tell application "iTerm2"
	set res to "notfound"
	repeat with w in windows
		repeat with t in tabs of w
			repeat with s in sessions of t
				try
					if (tty of s) is "{tty}" then
						select w
						select t
						select s
						set res to "found"
						activate
					end if
				end try
			end repeat
		end repeat
	end repeat
	return res
end tell"#
        ),
    };
    Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("found"))
        .unwrap_or(false)
}

/// The `$TMUX` socket path is the segment before the first comma.
fn tmux_socket(tmux: &str) -> Option<String> {
    tmux.split(',').next().filter(|s| !s.is_empty()).map(str::to_string)
}

/// Map a **non-tmux** session's reported host env to an `open -a` app name. Inside
/// tmux `ZED_TERM`/`TERM_PROGRAM` are server-inherited and unreliable (callers use
/// the process tree there instead), so this returns None for tmux panes.
#[cfg(target_os = "macos")]
fn gui_app(loc: &SessionLocation) -> Option<String> {
    if !loc.tmux_pane.is_empty() {
        return None;
    }
    if loc.zed_term == "true" {
        return Some("Zed".into());
    }
    let app = match loc.term_program.as_str() {
        "iTerm.app" => "iTerm",
        "Apple_Terminal" => "Terminal",
        "vscode" => "Visual Studio Code",
        "ghostty" | "Ghostty" => "Ghostty",
        "WezTerm" => "WezTerm",
        "Hyper" => "Hyper",
        "Tabby" => "Tabby",
        "WarpTerminal" | "Warp" => "Warp",
        "kitty" => "kitty",
        "Alacritty" => "Alacritty",
        _ => return None,
    };
    Some(app.to_string())
}

// ============================ hook script + install ============================

pub fn locate_script_path() -> Option<PathBuf> {
    permission::hook_dir().map(|d| d.join("locate.sh"))
}

const LOCATE_TEMPLATE: &str = r#"#!/bin/sh
# tablo session-locator hook — generated by Tablo. Do not edit by hand.
# Reports where this session lives (tmux pane / terminal app) so Tablo can focus
# its window. Passive: posts env locators to the loopback server and exits 0 with
# no stdout, so it never blocks or alters the session.
PORT=__PORT__
in=$(cat)
sid=$(printf '%s' "$in" | sed -n 's/.*"session_id"[^"]*"\([^"]*\)".*/\1/p')
[ -z "$sid" ] && exit 0
# The session's controlling terminal — read from the process, NOT `tty` (stdin
# here is the piped payload). For a direct terminal tab this is the tab's own
# tty (so Tablo can focus that exact tab); inside tmux it's the pane pty.
rawtty=$(ps -o tty= -p $$ 2>/dev/null | tr -d '[:space:]')
case "$rawtty" in
  ''|'??') tty='' ;;
  /dev/*) tty="$rawtty" ;;
  *) tty="/dev/$rawtty" ;;
esac
curl -s -m 2 -X POST -H 'Content-Type: application/json' \
  "http://127.0.0.1:${PORT}/locate" --data-binary @- >/dev/null 2>&1 <<JSON
{"sessionId":"$sid","tmux":"$TMUX","tmuxPane":"$TMUX_PANE","termProgram":"$TERM_PROGRAM","zedTerm":"$ZED_TERM","tty":"$tty"}
JSON
exit 0
"#;

/// Write (or refresh) the locate hook script. Idempotent, safe every launch —
/// it's Tablo's own file; wiring it into settings.json is the gated step.
pub fn write_locate_script(port: u16) -> std::io::Result<PathBuf> {
    let dir = permission::hook_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("locate.sh");
    std::fs::write(&path, LOCATE_TEMPLATE.replace("__PORT__", &port.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// Hook events the locator installs under. SessionStart captures new sessions
/// immediately; UserPromptSubmit refreshes and catches already-running ones.
const LOCATE_EVENTS: [&str; 2] = ["SessionStart", "UserPromptSubmit"];

pub fn install(cfg: &Config) -> Result<(), String> {
    let script = write_locate_script(cfg.permission_port).map_err(|e| e.to_string())?;
    let s = script.to_string_lossy().to_string();
    let mut root = permission::read_settings();
    for ev in LOCATE_EVENTS {
        permission::apply_install_event(&mut root, ev, &s, None, None);
    }
    permission::write_settings(&root).map_err(|e| e.to_string())
}

pub fn uninstall() -> Result<(), String> {
    let s = locate_script_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut root = permission::read_settings();
    for ev in LOCATE_EVENTS {
        permission::apply_uninstall_event(&mut root, ev, &s);
    }
    permission::write_settings(&root).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocateStatus {
    /// Whether our locate hook is wired into `~/.claude/settings.json`.
    pub installed: bool,
    pub script_path: String,
}

fn status() -> LocateStatus {
    let script = locate_script_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    LocateStatus {
        installed: permission::is_event_installed(LOCATE_EVENTS[0], &script),
        script_path: script,
    }
}

#[tauri::command]
pub fn locate_status() -> LocateStatus {
    status()
}

/// Enable/disable session location reporting. Like approvals this edits
/// `~/.claude/settings.json`, so it's only ever called on explicit user action.
#[tauri::command]
pub fn set_locate_enabled(state: State<'_, AppState>, enabled: bool) -> Result<LocateStatus, String> {
    let cfg = state.config.lock().unwrap().clone();
    if enabled {
        install(&cfg)?;
    } else {
        uninstall()?;
    }
    Ok(status())
}
