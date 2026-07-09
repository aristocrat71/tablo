//! "Jump to session" for **Codex** — the Codex-side twin of `locate.rs`.
//!
//! The entire jump *engine* (caching a session's window location, then focusing
//! it: `SessionLocation`, `store_location`, `focus`, `jump_to_session`) lives in
//! `locate.rs` and is source-agnostic — it's keyed purely by session id. So the
//! only thing Codex needs is to *self-report* its location the same way Claude
//! Code does, into the same loopback `/locate` endpoint and the same cache.
//!
//! Codex's hook system mirrors Claude Code's: a `SessionStart` + `UserPromptSubmit`
//! hook in `~/.codex/hooks.json` runs a command with the hook JSON (incl.
//! `session_id`) on stdin. We install the **exact same** `LOCATE_TEMPLATE` script
//! there — it reads the session env (`$TMUX_PANE`, `$TERM_PROGRAM`, tty) and POSTs
//! to `127.0.0.1:<port>/locate`, keyed by the Codex session id (the same UUID the
//! scanner reports on `SessionView.id`). `jump_to_session` then works unchanged.
//!
//! This module deliberately touches nothing in `locate.rs` or the Claude path —
//! it only reuses `locate::LOCATE_TEMPLATE`, `locate::LocateStatus`, and the pure
//! JSON-merge helpers in `permission.rs`. Reverting = drop this module + its
//! wiring; Claude "jump" is unaffected.
//!
//! One UX note: adding a hook makes Codex show a one-time "hooks are new or
//! changed — review before they run" prompt in the TUI. Once the user approves,
//! Codex records the trust and the hook runs on every turn. Existing hooks (e.g.
//! the Wel time hook) are appended-after and keep their trust.

use std::path::PathBuf;

use serde_json::{json, Value};
use tauri::State;

use crate::config::Config;
use crate::locate::{LocateStatus, LOCATE_TEMPLATE};
use crate::permission::{self, apply_install_event, apply_uninstall_event, entry_is_tablo};
use crate::AppState;

/// `~/.codex/hooks.json` — Codex's global hook config.
fn hooks_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex").join("hooks.json"))
}

/// Read `~/.codex/hooks.json` into a JSON object (or `{}` if missing/malformed).
/// Shape matches Claude's settings.json hook subtree: `{ "hooks": { <Event>: [ {
/// "hooks": [ {"type":"command","command":"…"} ] } ] } }`.
fn read_hooks() -> Value {
    let v = hooks_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(|| json!({}));
    if v.is_object() {
        v
    } else {
        json!({})
    }
}

fn write_hooks(root: &Value) -> std::io::Result<()> {
    let path = hooks_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(root).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(path, text)
}

/// The Codex locate hook script path (distinct from Claude's `locate.sh`).
pub fn locate_script_path() -> Option<PathBuf> {
    permission::hook_dir().map(|d| d.join("codex-locate.sh"))
}

/// Write (or refresh) the Codex locate hook script — byte-identical to the Claude
/// locator (same reporting logic), just a separate file. Idempotent and harmless:
/// it's Tablo's own file; wiring it into `hooks.json` is the gated step.
pub fn write_locate_script(port: u16) -> std::io::Result<PathBuf> {
    let dir = permission::hook_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("codex-locate.sh");
    std::fs::write(&path, LOCATE_TEMPLATE.replace("__PORT__", &port.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

/// Hook events the locator installs under (same two lifecycle events as the Claude
/// locator): SessionStart captures new sessions; UserPromptSubmit refreshes and
/// catches already-running ones.
const LOCATE_EVENTS: [&str; 2] = ["SessionStart", "UserPromptSubmit"];

/// Whether our Codex locate hook is present under the given event in hooks.json.
fn is_event_installed(event: &str, script: &str) -> bool {
    read_hooks()
        .get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().any(|e| entry_is_tablo(e, script)))
        .unwrap_or(false)
}

pub fn install(cfg: &Config) -> Result<(), String> {
    let script = write_locate_script(cfg.permission_port).map_err(|e| e.to_string())?;
    let s = script.to_string_lossy().to_string();
    let mut root = read_hooks();
    // `apply_install_event` appends after any existing entries (e.g. a Wel hook)
    // and removes only prior tablo entries — so foreign hooks keep their slot and
    // their Codex trust.
    for ev in LOCATE_EVENTS {
        apply_install_event(&mut root, ev, &s, None, None);
    }
    write_hooks(&root).map_err(|e| e.to_string())
}

pub fn uninstall() -> Result<(), String> {
    let s = locate_script_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut root = read_hooks();
    for ev in LOCATE_EVENTS {
        apply_uninstall_event(&mut root, ev, &s);
    }
    write_hooks(&root).map_err(|e| e.to_string())
}

fn status() -> LocateStatus {
    let script = locate_script_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    LocateStatus {
        installed: is_event_installed(LOCATE_EVENTS[0], &script),
        script_path: script,
    }
}

#[tauri::command]
pub fn codex_locate_status() -> LocateStatus {
    status()
}

/// Whether the Codex locate hook is wired into `~/.codex/hooks.json`. Gates the
/// "jump" affordance on Codex rows so a disabled setting hides it everywhere.
pub fn is_installed() -> bool {
    status().installed
}

/// Enable/disable Codex session-location reporting. Edits `~/.codex/hooks.json`,
/// so it's only ever called on explicit user action.
#[tauri::command]
pub fn set_codex_locate_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<LocateStatus, String> {
    let cfg = state.config.lock().unwrap().clone();
    if enabled {
        install(&cfg)?;
    } else {
        uninstall()?;
    }
    Ok(status())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The install/uninstall JSON merge must produce Codex's exact hooks.json
    /// shape, preserve a foreign (Wel-style) hook, and round-trip cleanly.
    #[test]
    fn install_preserves_foreign_hooks_and_round_trips() {
        let script = "/Users/x/.claude/tablo/codex-locate.sh";
        // Start from a hooks.json that already has a foreign SessionStart hook.
        let mut root = json!({
            "hooks": {
                "SessionStart": [
                    { "hooks": [ { "type": "command", "command": "\"/wel/hook.sh\" start" } ] }
                ]
            }
        });
        for ev in LOCATE_EVENTS {
            apply_install_event(&mut root, ev, script, None, None);
        }
        // Foreign hook still first, ours appended after → its trust index is kept.
        let ss = root["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 2);
        assert!(entry_is_tablo(&ss[0], script) == false);
        assert!(entry_is_tablo(&ss[1], script));
        // UserPromptSubmit gained our entry too.
        assert!(entry_is_tablo(&root["hooks"]["UserPromptSubmit"][0], script));
        // Our entries carry no matcher (lifecycle events don't take one).
        assert!(ss[1].get("matcher").is_none());

        // Uninstall removes only ours; the foreign hook and event survive.
        for ev in LOCATE_EVENTS {
            apply_uninstall_event(&mut root, ev, script);
        }
        let ss = root["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 1);
        assert!(entry_is_tablo(&ss[0], script) == false);
        // UserPromptSubmit was ours-only → pruned entirely.
        assert!(root["hooks"].get("UserPromptSubmit").is_none());
    }
}
