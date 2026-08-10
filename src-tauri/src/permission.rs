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

use std::io::Read;
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

/// Header the generated hooks carry the per-run secret in; requests without a
/// matching value are rejected before the body is read.
const SECRET_HEADER: &str = "X-Tablo-Secret";
/// Hard cap on a request body. Bounds memory per request against a flood of
/// oversized POSTs. A `/decide` body carries the tool input (a Write can be a
/// whole file), so the cap is generous; `/locate` bodies are tiny.
const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Most `/decide` requests held (blocked) at once. Past this we defer instead of
/// parking another thread for the ~10-minute window — bounds thread/queue growth.
const MAX_PENDING: usize = 64;

/// A fresh per-run secret (48 hex chars from the OS CSPRNG), baked into the
/// generated hook scripts and required on every request. Rotates each launch, so
/// a stale script from a previous run can't drive the current server.
pub(crate) fn new_secret() -> String {
    let mut buf = [0u8; 24];
    // The OS RNG only fails in exotic sandboxes; fall back to a still-unique
    // (if lower-entropy) seed rather than crashing the app on launch.
    if getrandom::getrandom(&mut buf).is_err() {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mix = t ^ (std::process::id() as u128) << 17;
        buf.copy_from_slice(&mix.to_le_bytes()[..8].repeat(3));
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time-ish equality so a wrong secret can't be recovered by timing the
/// reply. Length mismatch short-circuits (the length isn't secret).
fn secret_matches(got: &str, want: &str) -> bool {
    let (g, w) = (got.as_bytes(), want.as_bytes());
    if g.len() != w.len() {
        return false;
    }
    g.iter().zip(w).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

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

/// Bind the loopback approval server and serve it on a background thread. Binds
/// **synchronously** and returns whether it took the port: a busy port means
/// another process owns it, so the caller must not point hooks at it. Never
/// panics; on failure approvals are simply unavailable and the caller skips
/// wiring hooks.
pub fn spawn_server(app: AppHandle) -> bool {
    let port = app.state::<AppState>().config.lock().unwrap().permission_port;
    let server = match Server::http(("127.0.0.1", port)) {
        Ok(s) => s,
        Err(_) => return false,
    };
    app.state::<AppState>()
        .perm_server_up
        .store(true, std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let app = app.clone();
            // One thread per request: each blocks until the user decides.
            std::thread::spawn(move || handle_request(&app, request));
        }
    });
    true
}

/// Read the named request header (case-insensitive), if present.
fn header_value(request: &tiny_http::Request, name: &'static str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str().to_string())
}

/// Read at most `MAX_BODY_BYTES` of the request body. `None` if the declared
/// length already exceeds the cap (rejected before reading a single byte).
fn read_bounded_body(request: &mut tiny_http::Request) -> Option<String> {
    if request.body_length().is_some_and(|n| n > MAX_BODY_BYTES) {
        return None;
    }
    let mut body = String::new();
    request
        .as_reader()
        .take(MAX_BODY_BYTES as u64)
        .read_to_string(&mut body)
        .ok()
        .map(|_| body)
}

fn handle_request(app: &AppHandle, mut request: tiny_http::Request) {
    // Reject anything a browser could originate: our hooks never set Origin, so a
    // cross-site fetch (DNS-rebind / drive-by) is refused before it can act.
    if header_value(&request, "Origin").is_some() {
        let _ = request.respond(Response::from_string("").with_status_code(403));
        return;
    }
    // Authenticate before touching the body or any state. A request without the
    // per-run secret (another local process, a webpage) gets nothing done.
    let secret = app.state::<AppState>().ipc_secret.clone();
    let authed = header_value(&request, SECRET_HEADER)
        .is_some_and(|got| secret_matches(&got, &secret));
    if !authed {
        let _ = request.respond(Response::from_string("").with_status_code(401));
        return;
    }

    let is_post = request.method() == &Method::Post;
    let url = request.url().to_string();
    // Exact paths only — no prefix matching.
    if is_post && url == "/locate" {
        // Passive session-locator report ("jump to session"): cache it and answer
        // immediately — never blocks, unlike /decide.
        let body = read_bounded_body(&mut request).unwrap_or_default();
        crate::locate::store_location(app, &body);
        let _ = request.respond(Response::from_string("ok"));
        return;
    }
    if !(is_post && url == "/decide") {
        let _ = request.respond(Response::from_string("tablo"));
        return;
    }
    let body = match read_bounded_body(&mut request) {
        Some(b) => b,
        // Oversized/unreadable body — defer to Claude Code's normal flow.
        None => {
            let _ = request.respond(Response::from_string(PermDecision::Ask.hook_json()));
            return;
        }
    };
    let decision = wait_for_decision(app, &body);
    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("static header");
    let _ = request.respond(Response::from_string(decision.hook_json()).with_header(ct));
}

/// Register the incoming tool call, wake the UI, then block until the user
/// resolves it in the widget or the wait elapses. On timeout we fail **closed**
/// (deny) — the user chose a long window with auto-deny. (Tablo being *down* is
/// a different case: the hook's curl fails before reaching here and defers.)
/// Whether Tablo owns the approval decision in this permission mode. Only the
/// default mode prompts: `acceptEdits`/`auto`, `plan`, `bypassPermissions` and
/// `dontAsk` all mean the user already chose not to be asked per call, so the
/// hook defers (`{}`) and Claude Code applies that mode itself — deferring, not
/// auto-allowing, so Tablo can never be more permissive than the chosen mode.
/// An absent field (older Claude Code) keeps the previous always-prompt behavior.
pub(crate) fn prompts_in_mode(raw_mode: &str) -> bool {
    raw_mode.is_empty() || crate::scanner::display_mode(raw_mode) == "normal"
}

fn wait_for_decision(app: &AppHandle, body: &str) -> PermDecision {
    let parsed: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => return PermDecision::Ask, // malformed — don't gate the tool
    };
    // Any non-default mode is the user having already said "stop asking me", so
    // Tablo steps aside before registering anything.
    let mode = parsed.get("permission_mode").and_then(|v| v.as_str()).unwrap_or("");
    if !prompts_in_mode(mode) {
        return PermDecision::Ask;
    }
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
    // Bound how many hook requests we hold open at once. Past the cap, defer
    // rather than park another thread for the full (~10-minute) window — a flood
    // of /decide POSTs can't exhaust threads or grow the queue without limit.
    if state.pending.lock().unwrap().len() >= MAX_PENDING {
        return PermDecision::Ask;
    }

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
        install_hook(&cfg, &state.ipc_secret)?;
    } else {
        uninstall_hook()?;
    }
    Ok(build_status(&state, &cfg))
}

// ============================ hook script ============================

pub(crate) fn hook_dir() -> Option<PathBuf> {
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
  -H 'Content-Type: application/json' -H 'X-Tablo-Secret: __SECRET__' --data-binary @- \
  "http://127.0.0.1:${PORT}/decide" 2>/dev/null)
if [ -n "$resp" ]; then
  printf '%s' "$resp"
else
  printf '{}'
fi
exit 0
"#;

/// Write (or refresh) the hook script with the current port/timeout/secret baked
/// in. Idempotent and safe to call on every launch — it's Tablo's own file.
/// Written `0o700` so only the owner can read the embedded secret.
pub fn write_hook_script(port: u16, timeout: u64, secret: &str) -> std::io::Result<PathBuf> {
    let dir = hook_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("hook.sh");
    let script = HOOK_TEMPLATE
        .replace("__PORT__", &port.to_string())
        .replace("__TIMEOUT__", &timeout.to_string())
        .replace("__SECRET__", secret);
    std::fs::write(&path, script)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

// ============================ settings.json ============================

fn settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

pub(crate) fn read_settings() -> Value {
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

pub(crate) fn write_settings(root: &Value) -> std::io::Result<()> {
    let path = settings_path()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home dir"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(root).unwrap_or_else(|_| "{}".to_string());
    std::fs::write(path, text)
}

/// Does this hook entry point at the given tablo script?
pub(crate) fn entry_is_tablo(entry: &Value, script: &str) -> bool {
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
    is_event_installed("PreToolUse", script)
}

/// Whether a tablo entry for `script` is present under the given hook event.
pub(crate) fn is_event_installed(event: &str, script: &str) -> bool {
    read_settings()
        .get("hooks")
        .and_then(|h| h.get(event))
        .and_then(|p| p.as_array())
        .map(|arr| arr.iter().any(|e| entry_is_tablo(e, script)))
        .unwrap_or(false)
}

/// Add (or refresh) a tablo hook entry under `event`, in place. `matcher` and
/// `timeout` are optional (tool events like PreToolUse use a matcher; lifecycle
/// events like SessionStart / UserPromptSubmit take neither). Pure (no
/// filesystem) so it's unit-tested; preserves existing keys and is idempotent.
pub(crate) fn apply_install_event(
    root: &mut Value,
    event: &str,
    script: &str,
    matcher: Option<&str>,
    timeout: Option<u64>,
) {
    if !root.is_object() {
        *root = json!({});
    }
    let obj = root.as_object_mut().unwrap();
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks_obj = hooks.as_object_mut().unwrap();
    let ev = hooks_obj.entry(event).or_insert_with(|| json!([]));
    if !ev.is_array() {
        *ev = json!([]);
    }
    let arr = ev.as_array_mut().unwrap();
    arr.retain(|e| !entry_is_tablo(e, script));
    let mut hook = json!({ "type": "command", "command": script });
    if let Some(t) = timeout {
        hook["timeout"] = json!(t);
    }
    let mut entry = serde_json::Map::new();
    if let Some(m) = matcher {
        entry.insert("matcher".into(), json!(m));
    }
    entry.insert("hooks".into(), json!([hook]));
    arr.push(Value::Object(entry));
}

/// Remove a tablo entry under `event`, pruning now-empty containers. Pure/in place.
pub(crate) fn apply_uninstall_event(root: &mut Value, event: &str, script: &str) {
    let obj = match root.as_object_mut() {
        Some(o) => o,
        None => return,
    };
    let hooks = match obj.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        Some(h) => h,
        None => return,
    };
    if let Some(arr) = hooks.get_mut(event).and_then(|p| p.as_array_mut()) {
        arr.retain(|e| !entry_is_tablo(e, script));
        if arr.is_empty() {
            hooks.remove(event);
        }
    }
    if hooks.is_empty() {
        obj.remove("hooks");
    }
}

/// PreToolUse install/uninstall — thin wrappers over the generic per-event form.
fn apply_install(root: &mut Value, script: &str, matcher: &str, timeout: u64) {
    apply_install_event(root, "PreToolUse", script, Some(matcher), Some(timeout));
}

fn apply_uninstall(root: &mut Value, script: &str) {
    apply_uninstall_event(root, "PreToolUse", script);
}

pub fn install_hook(cfg: &Config, secret: &str) -> Result<(), String> {
    let script = write_hook_script(cfg.permission_port, cfg.hook_timeout_secs, secret)
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
    fn matcherless_event_install_and_uninstall() {
        // Lifecycle events (SessionStart / UserPromptSubmit) take no matcher and
        // no timeout — the locate hook's shape.
        let mut root = json!({});
        apply_install_event(&mut root, "SessionStart", SCRIPT, None, None);
        apply_install_event(&mut root, "UserPromptSubmit", SCRIPT, None, None);
        let ss = root["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 1);
        assert!(ss[0].get("matcher").is_none(), "lifecycle events take no matcher");
        assert_eq!(ss[0]["hooks"][0]["command"], SCRIPT);
        assert!(ss[0]["hooks"][0].get("timeout").is_none(), "no timeout");
        // Idempotent.
        apply_install_event(&mut root, "SessionStart", SCRIPT, None, None);
        assert_eq!(root["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
        // Uninstall both prunes the containers away.
        apply_uninstall_event(&mut root, "SessionStart", SCRIPT);
        apply_uninstall_event(&mut root, "UserPromptSubmit", SCRIPT);
        assert!(root.get("hooks").is_none(), "empty hooks pruned");
    }

    #[test]
    fn only_the_default_mode_prompts() {
        assert!(prompts_in_mode("default"));
        // Every "stop asking me" mode hands the decision back to Claude Code.
        for m in ["acceptEdits", "auto", "plan", "bypassPermissions", "dontAsk"] {
            assert!(!prompts_in_mode(m), "{m} must not raise a tablo prompt");
        }
        // Absent field (older Claude Code) keeps the previous behavior.
        assert!(prompts_in_mode(""));
        // An unknown future mode is treated as default rather than silently skipped.
        assert!(prompts_in_mode("someNewMode"));
    }

    /// The skip must produce the defer response, never an approval — deferring
    /// keeps Claude Code's own flow (acceptEdits still gates Bash), whereas
    /// auto-allowing would be strictly more permissive than the chosen mode.
    #[test]
    fn deferring_is_not_approving() {
        assert_eq!(PermDecision::Ask.hook_json(), "{}");
        assert!(PermDecision::Allow.hook_json().contains("\"allow\""));
    }

    #[test]
    fn secret_matches_is_exact() {
        let s = new_secret();
        assert!(secret_matches(&s, &s));
        assert!(!secret_matches("", &s));
        assert!(!secret_matches(&s[..s.len() - 1], &s), "length mismatch rejected");
        let mut wrong = s.clone();
        wrong.replace_range(0..1, if s.starts_with('a') { "b" } else { "a" });
        assert!(!secret_matches(&wrong, &s), "one-byte difference rejected");
    }

    #[test]
    fn new_secret_is_hex_and_fresh() {
        let a = new_secret();
        assert_eq!(a.len(), 48);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, new_secret(), "each call yields a fresh secret");
    }

    #[test]
    fn hook_script_embeds_secret_header() {
        // The template must carry the auth header + a secret slot the writer fills.
        assert!(HOOK_TEMPLATE.contains("X-Tablo-Secret: __SECRET__"));
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
