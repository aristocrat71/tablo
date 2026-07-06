//! Phase 4 — tool-call approvals.
//!
//! Claude Code fires a `PreToolUse` hook before a matched tool runs. Our hook
//! (`hook.sh`) POSTs the tool call to the tiny loopback server here and blocks;
//! the server registers a pending decision, wakes the UI, and holds the request
//! open until the user taps Approve/Deny (or a timeout defers to Claude Code's
//! normal flow). The decision is returned to the hook as `hookSpecificOutput`
//! JSON, which unblocks (or blocks) the tool.
//!
//! Nothing here modifies the user's `~/.claude/settings.json` unless they
//! explicitly enable approvals from the UI (`set_hook_enabled`).

use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};
use tiny_http::{Header, Method, Response, Server};

use crate::config::Config;
use crate::AppState;

/// Seconds shaved off the hook timeout so the server always answers (with a
/// defer) before Claude Code would kill the hook. Non-domain timing constant.
const RESPONSE_MARGIN_SECS: u64 = 5;

/// A tool call awaiting a human decision. The serialized view pushed to the UI;
/// the matching responder channel lives in `AppState.responders`, keyed by `id`.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PendingRequest {
    pub id: String,
    /// Tool name, e.g. "Bash".
    pub tool: String,
    /// Best-effort one-line preview of the tool input (command, file path…).
    pub detail: String,
    pub session_id: String,
    pub project: String,
    pub path: String,
    /// ms since epoch when the request arrived.
    pub created_at: i64,
}

/// The three outcomes the hook understands. `Ask` means "no decision" — defer
/// to Claude Code's normal permission flow (empty hook output).
#[derive(Clone, Copy, Debug)]
pub enum PermDecision {
    Allow,
    Deny,
    Ask,
}

impl PermDecision {
    fn from_str(s: &str) -> Self {
        match s {
            "allow" => PermDecision::Allow,
            "deny" => PermDecision::Deny,
            _ => PermDecision::Ask,
        }
    }

    /// The `PreToolUse` hook stdout JSON for this decision.
    fn hook_json(self) -> String {
        match self {
            PermDecision::Allow => hook_output("allow", "Approved in tablo"),
            PermDecision::Deny => hook_output("deny", "Denied in tablo"),
            PermDecision::Ask => "{}".to_string(),
        }
    }
}

fn hook_output(decision: &str, reason: &str) -> String {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        }
    })
    .to_string()
}

// ============================ server ============================

/// Bind the loopback approval server and serve it on a background thread. On a
/// bind failure (port busy) approvals are simply unavailable — the hook then
/// defers every call — so this never panics.
pub fn spawn_server(app: AppHandle) {
    let port = app.state::<AppState>().config.lock().unwrap().permission_port;
    std::thread::spawn(move || {
        let server = match Server::http(("127.0.0.1", port)) {
            Ok(s) => s,
            Err(_) => return,
        };
        app.state::<AppState>()
            .perm_server_up
            .store(true, std::sync::atomic::Ordering::Relaxed);
        for request in server.incoming_requests() {
            let app = app.clone();
            // One thread per request: each blocks until the user decides.
            std::thread::spawn(move || handle_request(&app, request));
        }
    });
}

fn handle_request(app: &AppHandle, mut request: tiny_http::Request) {
    let is_decide = request.method() == &Method::Post && request.url().starts_with("/decide");
    if !is_decide {
        let _ = request.respond(Response::from_string("tablo"));
        return;
    }
    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        let _ = request.respond(Response::from_string(PermDecision::Ask.hook_json()));
        return;
    }
    let decision = wait_for_decision(app, &body);
    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header");
    let _ = request.respond(Response::from_string(decision.hook_json()).with_header(ct));
}

/// Register the incoming tool call, wake the UI, then block until the user
/// resolves it in the widget or the wait elapses. On timeout we fail **closed**
/// (deny) — the user chose a long window with auto-deny. (Tablo being *down* is
/// a different case: the hook's curl fails before reaching here and defers.)
fn wait_for_decision(app: &AppHandle, body: &str) -> PermDecision {
    let parsed: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return PermDecision::Ask, // malformed — don't gate the tool
    };
    let tool = parsed.get("tool_name").and_then(|v| v.as_str()).unwrap_or("tool");
    let cwd = parsed.get("cwd").and_then(|v| v.as_str()).unwrap_or("");
    let session_id = parsed
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let input = parsed.get("tool_input").cloned().unwrap_or(Value::Null);
    let detail = summarize(tool, &input);
    let (project, path) = project_and_path(cwd);

    let state = app.state::<AppState>();
    let id = {
        let n = state
            .perm_seq
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        format!("p{n}")
    };
    let (tx, rx) = channel::<PermDecision>();
    let wait = {
        let secs = state
            .config
            .lock()
            .unwrap()
            .hook_timeout_secs
            .saturating_sub(RESPONSE_MARGIN_SECS)
            .max(1);
        Duration::from_secs(secs)
    };

    state.responders.lock().unwrap().insert(id.clone(), tx);
    state.pending.lock().unwrap().push(PendingRequest {
        id: id.clone(),
        tool: tool.to_string(),
        detail,
        session_id,
        project,
        path,
        created_at: crate::scanner::now_ms(),
    });
    crate::recompute_and_emit(app);

    // Fail closed: no decision within the (long) window => deny.
    let decision = rx.recv_timeout(wait).unwrap_or(PermDecision::Deny);

    // On a decision, `resolve_permission` already removed the entry; on timeout
    // we clean it up ourselves. Both cases re-emit so the row disappears.
    let removed = take_pending(&state, &id);
    if removed {
        crate::recompute_and_emit(app);
    }
    decision
}

/// Remove a pending request + its responder. Returns whether it was present.
fn take_pending(state: &AppState, id: &str) -> bool {
    let had_responder = state.responders.lock().unwrap().remove(id).is_some();
    let mut pending = state.pending.lock().unwrap();
    let before = pending.len();
    pending.retain(|p| p.id != id);
    had_responder || pending.len() != before
}

// ============================ commands ============================

/// Deliver a decision for pending `id` and drop its row. Sending unblocks the
/// held hook request; removing clears the widget.
fn resolve_inner(app: &AppHandle, id: &str, dec: PermDecision) {
    let state = app.state::<AppState>();
    if let Some(tx) = state.responders.lock().unwrap().remove(id) {
        let _ = tx.send(dec);
    }
    state.pending.lock().unwrap().retain(|p| p.id != id);
    crate::recompute_and_emit(app);
}

/// Resolve a pending approval from the UI. `decision` is "allow" | "deny" |
/// "ask" (ask = defer to Claude Code's own prompt).
#[tauri::command]
pub fn resolve_permission(app: AppHandle, id: String, decision: String) {
    resolve_inner(&app, &id, PermDecision::from_str(&decision));
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookStatus {
    /// Whether our PreToolUse entry is present in `~/.claude/settings.json`.
    pub installed: bool,
    /// Whether the loopback server bound successfully this run.
    pub server_up: bool,
    pub script_path: String,
    pub port: u16,
    /// Tools that will be intercepted (for display).
    pub tools: Vec<String>,
}

fn build_status(state: &AppState, cfg: &Config) -> HookStatus {
    let script = hook_script_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    HookStatus {
        installed: is_installed(&script),
        server_up: state
            .perm_server_up
            .load(std::sync::atomic::Ordering::Relaxed),
        script_path: script,
        port: cfg.permission_port,
        tools: cfg.intercept_tools.clone(),
    }
}

#[tauri::command]
pub fn hook_status(state: State<'_, AppState>) -> HookStatus {
    let cfg = state.config.lock().unwrap().clone();
    build_status(&state, &cfg)
}

/// Enable or disable approvals by installing/removing the hook from
/// `~/.claude/settings.json`. This is the only place that edits the user's
/// Claude Code settings, and only on explicit user action.
#[tauri::command]
pub fn set_hook_enabled(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<HookStatus, String> {
    let cfg = state.config.lock().unwrap().clone();
    if enabled {
        install_hook(&cfg)?;
    } else {
        uninstall_hook()?;
    }
    Ok(build_status(&state, &cfg))
}

// ============================ hook script ============================

fn hook_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("tablo"))
}

pub fn hook_script_path() -> Option<PathBuf> {
    hook_dir().map(|d| d.join("hook.sh"))
}

const HOOK_TEMPLATE: &str = r#"#!/bin/sh
# tablo PreToolUse approval hook — generated by Tablo. Do not edit by hand.
# Forwards the tool call to the running tablo app and prints its decision.
# The widget is the decision surface. If tablo is unreachable the request never
# arrives (curl fails) and this prints {} so Claude Code falls back to its normal
# permission flow — the session never hangs.
PORT=__PORT__
resp=$(cat | curl -s -m __TIMEOUT__ -X POST \
  -H 'Content-Type: application/json' --data-binary @- \
  "http://127.0.0.1:${PORT}/decide" 2>/dev/null)
if [ -n "$resp" ]; then
  printf '%s' "$resp"
else
  printf '{}'
fi
exit 0
"#;

/// Write (or refresh) the hook script with the current port/timeout baked in.
/// Idempotent and safe to call on every launch — it's Tablo's own file.
pub fn write_hook_script(port: u16, timeout: u64) -> std::io::Result<PathBuf> {
    let dir = hook_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("hook.sh");
    let script = HOOK_TEMPLATE
        .replace("__PORT__", &port.to_string())
        .replace("__TIMEOUT__", &timeout.to_string());
    std::fs::write(&path, script)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(path)
}

// ============================ settings.json ============================

fn settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

fn read_settings() -> Value {
    let v = settings_path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .unwrap_or_else(|| json!({}));
    if v.is_object() {
        v
    } else {
        json!({})
    }
}

fn write_settings(root: &Value) -> std::io::Result<()> {
    let path = settings_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(root).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(path, text)
}

/// Does this PreToolUse entry point at our hook script?
fn entry_is_tablo(entry: &Value, script: &str) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|hooks| {
            hooks.iter().any(|h| {
                h.get("command").and_then(|c| c.as_str()) == Some(script)
            })
        })
        .unwrap_or(false)
}

fn is_installed(script: &str) -> bool {
    read_settings()
        .get("hooks")
        .and_then(|h| h.get("PreToolUse"))
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().any(|e| entry_is_tablo(e, script)))
        .unwrap_or(false)
}

/// Add (or refresh) our PreToolUse entry in a settings document, in place.
/// Pure: no filesystem, so it's unit-tested. Existing keys are preserved and
/// re-running is idempotent (a prior tablo entry is replaced, not duplicated).
fn apply_install(root: &mut Value, script: &str, matcher: &str, timeout: u64) {
    if !root.is_object() {
        *root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks_obj = hooks.as_object_mut().unwrap();
    let pre = hooks_obj.entry("PreToolUse").or_insert_with(|| json!([]));
    if !pre.is_array() {
        *pre = json!([]);
    }
    let arr = pre.as_array_mut().unwrap();
    arr.retain(|e| !entry_is_tablo(e, script));
    arr.push(json!({
        "matcher": matcher,
        "hooks": [{
            "type": "command",
            "command": script,
            "timeout": timeout,
        }],
    }));
}

/// Remove our PreToolUse entry, pruning now-empty containers. Pure/in place.
fn apply_uninstall(root: &mut Value, script: &str) {
    let obj = match root.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    let hooks = match obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        Some(h) => h,
        None => return,
    };
    if let Some(arr) = hooks.get_mut("PreToolUse").and_then(|p| p.as_array_mut()) {
        arr.retain(|e| !entry_is_tablo(e, script));
        if arr.is_empty() {
            hooks.remove("PreToolUse");
        }
    }
    if hooks.is_empty() {
        obj.remove("hooks");
    }
}

pub fn install_hook(cfg: &Config) -> Result<(), String> {
    let script = write_hook_script(cfg.permission_port, cfg.hook_timeout_secs)
        .map_err(|e| e.to_string())?;
    let script_str = script.to_string_lossy().to_string();
    let matcher = if cfg.intercept_tools.is_empty() {
        "*".to_string()
    } else {
        cfg.intercept_tools.join("|")
    };
    let mut root = read_settings();
    apply_install(&mut root, &script_str, &matcher, cfg.hook_timeout_secs);
    write_settings(&root).map_err(|e| e.to_string())
}

pub fn uninstall_hook() -> Result<(), String> {
    let script = hook_script_path()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut root = read_settings();
    apply_uninstall(&mut root, &script);
    write_settings(&root).map_err(|e| e.to_string())
}

// ============================ helpers ============================

/// Best-effort one-line preview of a tool's input for the approval card.
fn summarize(tool: &str, input: &Value) -> String {
    let raw = match tool {
        "Bash" => input.get("command").and_then(|v| v.as_str()).map(str::to_string),
        "Write" | "Edit" | "MultiEdit" => {
            input.get("file_path").and_then(|v| v.as_str()).map(str::to_string)
        }
        "NotebookEdit" => input
            .get("notebook_path")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        _ => None,
    }
    .unwrap_or_else(|| serde_json::to_string(input).unwrap_or_default());
    truncate(raw.trim(), 300)
}

fn project_and_path(cwd: &str) -> (String, String) {
    let project = std::path::Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(cwd)
        .to_string();
    (project, abbreviate_home(cwd))
}

fn abbreviate_home(path: &str) -> String {
    if let Some(home) = dirs::home_dir().and_then(|h| h.to_str().map(str::to_string)) {
        if let Some(rest) = path.strip_prefix(&home) {
            return format!("~{rest}");
        }
    }
    path.to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "/home/u/.claude/tablo/hook.sh";

    #[test]
    fn install_preserves_existing_settings() {
        let mut root = json!({
            "model": "opus",
            "hooks": {
                "PostToolUse": [{ "matcher": "Bash", "hooks": [] }]
            }
        });
        apply_install(&mut root, SCRIPT, "Bash|Edit", 120);

        // Untouched keys survive.
        assert_eq!(root["model"], "opus");
        assert!(root["hooks"]["PostToolUse"].is_array());
        // Our entry is present with the matcher + command + timeout.
        let pre = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1);
        assert_eq!(pre[0]["matcher"], "Bash|Edit");
        assert_eq!(pre[0]["hooks"][0]["command"], SCRIPT);
        assert_eq!(pre[0]["hooks"][0]["timeout"], 120);
    }

    #[test]
    fn install_is_idempotent() {
        let mut root = json!({});
        apply_install(&mut root, SCRIPT, "Bash", 120);
        apply_install(&mut root, SCRIPT, "Bash", 120);
        let pre = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "re-install must not duplicate the entry");
    }

    #[test]
    fn install_keeps_foreign_pretooluse_hooks() {
        let mut root = json!({
            "hooks": { "PreToolUse": [{ "matcher": "Write", "hooks": [
                { "type": "command", "command": "/other/hook.sh" }
            ]}]}
        });
        apply_install(&mut root, SCRIPT, "Bash", 120);
        let pre = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2, "a foreign PreToolUse hook must be kept");
    }

    #[test]
    fn uninstall_removes_only_ours_and_prunes() {
        let mut root = json!({});
        apply_install(&mut root, SCRIPT, "Bash", 120);
        apply_uninstall(&mut root, SCRIPT);
        // With nothing else present, empty containers are pruned away.
        assert!(root.get("hooks").is_none(), "empty hooks should be removed");
    }

    #[test]
    fn uninstall_leaves_foreign_hooks_intact() {
        let mut root = json!({
            "hooks": { "PreToolUse": [{ "matcher": "Write", "hooks": [
                { "type": "command", "command": "/other/hook.sh" }
            ]}]}
        });
        apply_install(&mut root, SCRIPT, "Bash", 120);
        apply_uninstall(&mut root, SCRIPT);
        let pre = root["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1);
        assert_eq!(pre[0]["matcher"], "Write");
    }

    #[test]
    fn decision_json_shapes() {
        assert_eq!(PermDecision::Ask.hook_json(), "{}");
        let allow: Value = serde_json::from_str(&PermDecision::Allow.hook_json()).unwrap();
        assert_eq!(allow["hookSpecificOutput"]["permissionDecision"], "allow");
        assert_eq!(allow["hookSpecificOutput"]["hookEventName"], "PreToolUse");
        let deny: Value = serde_json::from_str(&PermDecision::Deny.hook_json()).unwrap();
        assert_eq!(deny["hookSpecificOutput"]["permissionDecision"], "deny");
    }

    #[test]
    fn summarize_picks_the_key_arg() {
        assert_eq!(summarize("Bash", &json!({ "command": "npm test" })), "npm test");
        assert_eq!(
            summarize("Edit", &json!({ "file_path": "/a/b.rs", "old": "x" })),
            "/a/b.rs"
        );
        // Unknown tool falls back to compact JSON of the input.
        assert!(summarize("Weird", &json!({ "k": 1 })).contains("\"k\""));
    }
}
