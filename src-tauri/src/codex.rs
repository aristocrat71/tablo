//! Codex support: watch OpenAI Codex CLI rollout transcripts and map them onto
//! the same per-session model the Claude Code scanner produces, so Codex sessions
//! flow through Tablo's avatar / panel / dashboard unchanged.
//!
//! Rollouts live at `~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-<ts>-<uuid>.jsonl`.
//! Each line is `{ timestamp, type, payload }`. Unlike Claude Code, Codex emits
//! explicit turn-lifecycle events — `task_started` / `task_complete` /
//! `turn_aborted` — so the working ↔ waiting transition is read directly from the
//! stream rather than inferred, and the context window is reported verbatim in
//! every `token_count` (`model_context_window`). We reuse the scanner's
//! `FileState`, rolling-log, and text helpers; only the parse differs.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::scanner::{
    base_name, mtime_ms, push_log, snippet, truncate_activity, FileState, ACTIVITY_MAX,
    TERM_LINE_MAX,
};

/// `~/.codex/sessions` — the root of the date-nested rollout tree.
pub fn sessions_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".codex").join("sessions"))
}

/// Recursively collect `*.jsonl` rollouts under the date-nested tree that were
/// modified within the active window. Depth is naturally bounded (YYYY/MM/DD), so
/// full recursion is cheap; only files still inside `active_ms` are returned as
/// `(path, size, mtime)`.
pub fn collect_active(dir: &Path, now: i64, active_ms: i64, out: &mut Vec<(PathBuf, u64, i64)>) {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for e in rd.flatten() {
        let path = e.path();
        let meta = match e.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            collect_active(&path, now, active_ms, out);
        } else if path.extension().and_then(|x| x.to_str()) == Some("jsonl") {
            let mtime = mtime_ms(&meta);
            if now - mtime <= active_ms {
                out.push((path, meta.len(), mtime));
            }
        }
    }
}

/// Project name + root path for a Codex session. Codex records the launch `cwd`
/// on `session_meta` / `turn_context` (no encoded slug like Claude Code), so the
/// project is simply that directory's basename.
pub fn resolve_project(cwd: &str) -> (String, String) {
    if cwd.is_empty() {
        return ("codex".into(), String::new());
    }
    let name = Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("codex")
        .to_string();
    (name, cwd.to_string())
}

/// Recover a session id from a rollout filename when `session_meta` was skipped
/// (only on the huge-file first-sight tail). The file is named
/// `rollout-<ts>-<uuid>.jsonl`, and the trailing UUID is the session id — it's
/// always the last five hyphen groups (8-4-4-4-12).
pub fn session_id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() < 5 {
        return None;
    }
    let uuid = parts[parts.len() - 5..].join("-");
    // Sanity: a UUID is 36 chars (32 hex + 4 dashes).
    (uuid.len() == 36).then_some(uuid)
}

/// Map Codex's `approval_policy` to the read-only mode badge (shared vocabulary
/// with Claude Code's badge). "never" auto-runs everything → the most permissive
/// "bypass"; "on-failure" mostly auto-approves → "auto"; otherwise it asks →
/// "normal".
pub fn display_mode(approval_policy: &str) -> &'static str {
    match approval_policy {
        "never" => "bypass",
        "on-failure" => "auto",
        _ => "normal", // on-request / untrusted / unknown
    }
}

/// Ingest newly-appended Codex rollout lines into a session's tail state. Mirrors
/// `scanner::ingest`, translating the Codex event vocabulary into the same
/// `activity` / `activity_kind` / rolling-log / usage fields.
pub fn ingest(state: &mut FileState, lines: &[String]) {
    for line in lines {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // malformed / partial line (Step 1.3)
        };
        let payload = v.get("payload");
        match v.get("type").and_then(|x| x.as_str()) {
            // Session identity + launch cwd ride the opening meta line.
            Some("session_meta") => {
                if let Some(p) = payload {
                    set_id_cwd(state, p);
                }
            }
            // Per-turn context: the current model + cwd, and the approval policy
            // that drives the mode badge.
            Some("turn_context") => {
                if let Some(p) = payload {
                    set_id_cwd(state, p);
                    if let Some(m) = str_field(p, "model") {
                        state.model = m.to_string();
                    }
                    if let Some(ap) = str_field(p, "approval_policy") {
                        state.mode = ap.to_string();
                    }
                }
            }
            Some("event_msg") => {
                if let Some(p) = payload {
                    ingest_event(state, p);
                }
            }
            Some("response_item") => {
                if let Some(p) = payload {
                    ingest_response_item(state, p);
                }
            }
            _ => {}
        }
    }
}

/// `event_msg` payloads: turn lifecycle, token counts, and the human/agent
/// messages.
fn ingest_event(state: &mut FileState, p: &Value) {
    match p.get("type").and_then(|x| x.as_str()) {
        // Turn begins: the model is running but hasn't emitted output yet.
        Some("task_started") => {
            if let Some(w) = u64_field(p, "model_context_window") {
                state.ctx_window = w;
            }
            state.activity = "thinking…".into();
            state.activity_kind = "working".into();
            state.awaiting_first = true;
        }
        // Context occupancy: `last_token_usage.total_tokens` is the size of the
        // most recent request+response, i.e. what's currently in the window (the
        // whole conversation is re-sent each turn). `model_context_window` is the
        // denominator, reported verbatim here.
        Some("token_count") => {
            if let Some(info) = p.get("info") {
                if let Some(w) = u64_field(info, "model_context_window") {
                    state.ctx_window = w;
                }
                let used = info
                    .get("last_token_usage")
                    .and_then(|u| u64_field(u, "total_tokens"))
                    .or_else(|| info.get("total_token_usage").and_then(|u| u64_field(u, "total_tokens")));
                if let Some(u) = used {
                    state.used = u;
                }
            }
        }
        // The human's typed prompt — logs a line and (re)arms the turn as working.
        Some("user_message") => {
            if let Some(msg) = str_field(p, "message") {
                let sn = snippet(msg);
                if !sn.is_empty() {
                    push_log(state, "user", &sn);
                    // Codex has no AI title; the first prompt disambiguates the row.
                    if state.title.is_none() {
                        state.title = Some(sn.clone());
                    }
                }
            }
            state.activity = "thinking…".into();
            state.activity_kind = "working".into();
            state.awaiting_first = true;
        }
        // Agent narration mid-turn — live "what it's saying" preview. Still
        // working; `task_complete` flips to waiting.
        Some("agent_message") => {
            if let Some(msg) = str_field(p, "message") {
                let sn = snippet(msg);
                if !sn.is_empty() {
                    push_log(state, "text", &sn);
                    state.activity = truncate_activity(&sn, ACTIVITY_MAX);
                    state.activity_kind = "working".into();
                    state.awaiting_first = false;
                }
            }
        }
        // Turn finished → waiting on the user; the parting line is the preview.
        Some("task_complete") => {
            if let Some(msg) = str_field(p, "last_agent_message") {
                let sn = snippet(msg);
                if !sn.is_empty() {
                    state.activity = truncate_activity(&sn, ACTIVITY_MAX);
                }
            }
            state.activity_kind = "waiting".into();
            state.awaiting_first = false;
        }
        // Esc / Ctrl+C mid-turn → the agent stopped and handed back.
        Some("turn_aborted") => {
            state.activity = "interrupted".into();
            state.activity_kind = "waiting".into();
            state.awaiting_first = false;
        }
        _ => {}
    }
}

/// `response_item` payloads: the tool calls + reasoning that make up a turn.
fn ingest_response_item(state: &mut FileState, p: &Value) {
    match p.get("type").and_then(|x| x.as_str()) {
        Some("function_call") => {
            let name = p.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
            let args = p.get("arguments").and_then(|x| x.as_str()).unwrap_or("");
            let summary = summarize_call(name, args);
            push_log(state, "tool", &summary);
            state.activity = truncate_activity(&summary, ACTIVITY_MAX);
            state.activity_kind = "working".into();
            state.awaiting_first = false;
        }
        Some("reasoning") => {
            push_log(state, "think", "thinking…");
            state.activity = "thinking…".into();
            state.activity_kind = "thinking".into();
            state.awaiting_first = false;
        }
        // `message` (developer/user/system context) and `function_call_output`
        // don't change the live activity.
        _ => {}
    }
}

fn set_id_cwd(state: &mut FileState, p: &Value) {
    if let Some(id) = str_field(p, "id") {
        state.session_id = id.to_string();
    }
    if let Some(cwd) = str_field(p, "cwd") {
        state.cwd = cwd.to_string();
    }
}

fn str_field<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

fn u64_field(v: &Value, key: &str) -> Option<u64> {
    v.get(key).and_then(|x| x.as_u64())
}

/// Human, verb-led one-liner for a Codex tool call. `arguments` arrives as a JSON
/// *string*, so we parse it first. Mirrors the tone of the Claude summarizer
/// ("running cargo check", "editing scanner.rs").
pub fn summarize_call(name: &str, args_str: &str) -> String {
    let args: Value = serde_json::from_str(args_str).unwrap_or(Value::Null);
    let get = |k: &str| args.get(k).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());

    let s: String = match name {
        // Codex's shell tools. `cmd` may be a plain string or an argv array.
        "exec_command" | "shell" | "local_shell" | "container.exec" => {
            let cmd = get("cmd")
                .map(str::to_string)
                .or_else(|| args.get("cmd").and_then(cmd_from_value))
                .or_else(|| args.get("command").and_then(cmd_from_value));
            match cmd {
                Some(c) => format!("running {c}"),
                None => "running a command".into(),
            }
        }
        // Patch application — pull the first touched file out of the patch body.
        "apply_patch" | "edit_file" | "apply_diff" => {
            patch_target(&args).unwrap_or_else(|| "editing files".into())
        }
        "read_file" | "read" => get("path")
            .or_else(|| get("file_path"))
            .map(|p| format!("reading {}", base_name(p)))
            .unwrap_or_else(|| "reading a file".into()),
        "update_plan" => "updating the plan".into(),
        "web_search" | "web.search" => get("query")
            .map(|q| format!("searching the web: {q}"))
            .unwrap_or_else(|| "searching the web".into()),
        "view_image" => "viewing an image".into(),
        // MCP-style `server__tool`, or a bare tool name — humanize underscores.
        other => {
            if let Some((server, tool)) = other.split_once("__") {
                format!("{server}: {}", tool.replace('_', " "))
            } else {
                other.replace('_', " ")
            }
        }
    };
    truncate_activity(&s, TERM_LINE_MAX)
}

/// A command value that may be a string or an argv array → one display string.
fn cmd_from_value(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => {
            let t = s.trim();
            (!t.is_empty()).then(|| t.to_string())
        }
        Value::Array(parts) => {
            let joined = parts
                .iter()
                .filter_map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            let t = joined.trim().to_string();
            (!t.is_empty()).then_some(t)
        }
        _ => None,
    }
}

/// Extract the first file an apply_patch touches, from either a structured
/// `path`/`file_path` arg or the `*** Update/Add/Delete File: <path>` header in a
/// unified-patch `input`/`patch` body.
fn patch_target(args: &Value) -> Option<String> {
    for key in ["path", "file_path"] {
        if let Some(p) = args.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()) {
            return Some(format!("editing {}", base_name(p)));
        }
    }
    let body = args
        .get("input")
        .or_else(|| args.get("patch"))
        .and_then(|v| v.as_str())?;
    for line in body.lines() {
        let l = line.trim_start_matches(|c: char| c == '*' || c == ' ');
        for tag in ["Update File:", "Add File:", "Delete File:"] {
            if let Some(rest) = l.strip_prefix(tag) {
                let p = rest.trim();
                if !p.is_empty() {
                    return Some(format!("editing {}", base_name(p)));
                }
            }
        }
    }
    None
}

/// Convenience for the smoke test: the newest rollout on disk (or None).
#[cfg(test)]
pub fn newest_rollout() -> Option<PathBuf> {
    let dir = sessions_dir()?;
    let mut files = Vec::new();
    collect_active(&dir, crate::scanner::now_ms(), i64::MAX, &mut files); // MAX = all
    files.into_iter().max_by_key(|(_, _, m)| *m).map(|(p, _, _)| p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::read_all_lines;

    #[test]
    fn summarize_shell_and_patch() {
        // exec_command with a string cmd → "running <cmd>".
        assert_eq!(
            summarize_call("exec_command", r#"{"cmd":"cargo check","workdir":"/x"}"#),
            "running cargo check"
        );
        // argv-array command form.
        assert_eq!(
            summarize_call("shell", r#"{"command":["git","status","--short"]}"#),
            "running git status --short"
        );
        // apply_patch pulls the file out of the patch header.
        assert_eq!(
            summarize_call(
                "apply_patch",
                r#"{"input":"*** Begin Patch\n*** Update File: src/lib/Panel.svelte\n@@\n-old\n+new\n*** End Patch"}"#
            ),
            "editing Panel.svelte"
        );
        assert_eq!(summarize_call("update_plan", "{}"), "updating the plan");
        // Unknown tool: humanized.
        assert_eq!(summarize_call("some_tool", "not json"), "some tool");
    }

    #[test]
    fn approval_policy_maps_to_badge() {
        assert_eq!(display_mode("never"), "bypass");
        assert_eq!(display_mode("on-failure"), "auto");
        assert_eq!(display_mode("on-request"), "normal");
        assert_eq!(display_mode("untrusted"), "normal");
    }

    #[test]
    fn turn_lifecycle_drives_working_then_waiting() {
        let mut st = FileState::default();
        // Meta sets identity + cwd.
        ingest(
            &mut st,
            &[r#"{"type":"session_meta","payload":{"id":"abc","cwd":"/Users/x/proj"}}"#.into()],
        );
        assert_eq!(st.session_id, "abc");
        assert_eq!(st.cwd, "/Users/x/proj");

        // turn_context carries model + approval policy.
        ingest(
            &mut st,
            &[r#"{"type":"turn_context","payload":{"model":"gpt-5.5","approval_policy":"on-request","cwd":"/Users/x/proj"}}"#.into()],
        );
        assert_eq!(st.model, "gpt-5.5");
        assert_eq!(display_mode(&st.mode), "normal");

        // task_started → working, window recorded.
        ingest(
            &mut st,
            &[r#"{"type":"event_msg","payload":{"type":"task_started","model_context_window":258400}}"#.into()],
        );
        assert_eq!(st.activity_kind, "working");
        assert_eq!(st.ctx_window, 258400);
        assert!(st.awaiting_first);

        // A user prompt logs + doubles as the title.
        ingest(
            &mut st,
            &[r#"{"type":"event_msg","payload":{"type":"user_message","message":"analyse this project"}}"#.into()],
        );
        assert_eq!(st.title.as_deref(), Some("analyse this project"));

        // token_count sets used from last_token_usage.total_tokens.
        ingest(
            &mut st,
            &[r#"{"type":"event_msg","payload":{"type":"token_count","info":{"model_context_window":258400,"last_token_usage":{"total_tokens":13194},"total_token_usage":{"total_tokens":25055}}}}"#.into()],
        );
        assert_eq!(st.used, 13194);

        // A tool call → working, human summary.
        ingest(
            &mut st,
            &[r#"{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"rg --files\"}"}}"#.into()],
        );
        assert_eq!(st.activity_kind, "working");
        assert_eq!(st.activity, "running rg --files");

        // task_complete → waiting, parting line is the preview.
        ingest(
            &mut st,
            &[r#"{"type":"event_msg","payload":{"type":"task_complete","last_agent_message":"Done — here's what I found."}}"#.into()],
        );
        assert_eq!(st.activity_kind, "waiting");
        assert_eq!(st.activity, "Done — here's what I found.");
        assert!(!st.awaiting_first);
    }

    #[test]
    fn turn_aborted_hands_back_to_user() {
        let mut st = FileState::default();
        ingest(
            &mut st,
            &[r#"{"type":"event_msg","payload":{"type":"task_started","model_context_window":258400}}"#.into()],
        );
        assert_eq!(st.activity_kind, "working");
        ingest(
            &mut st,
            &[r#"{"type":"event_msg","payload":{"type":"turn_aborted","reason":"interrupted"}}"#.into()],
        );
        assert_eq!(st.activity_kind, "waiting");
        assert_eq!(st.activity, "interrupted");
    }

    /// Smoke test against the real `~/.codex/sessions` rollouts, mirroring the
    /// Claude scanner's. Prints the newest session's parsed shape end-to-end.
    #[test]
    fn parse_real_rollout() {
        let Some(path) = newest_rollout() else {
            eprintln!("no codex rollouts on disk — skipping");
            return;
        };
        let lines = read_all_lines(&path);
        let mut st = FileState::default();
        ingest(&mut st, &lines);
        println!(
            "\ncodex {:?}\n  model={} used={} window={} kind={} activity={:?}\n  title={:?}",
            path.file_name().unwrap(),
            st.model,
            st.used,
            st.ctx_window,
            st.activity_kind,
            st.activity,
            st.title
        );
        // A parsed rollout must at least know its model + a plausible window.
        assert!(!st.model.is_empty(), "model should be set from turn_context");
    }
}
