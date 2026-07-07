//! Phase 1 core: watch Claude Code JSONL transcripts and compute per-session
//! context occupancy plus the aggregate avatar state.
//!
//! Transcripts live at `~/.claude/projects/<slug>/<session-id>.jsonl`. We only
//! look at files *directly* inside each project slug dir — the `subagents/`
//! subdirectory is intentionally skipped, since "agent count" is defined as the
//! number of active top-level sessions (CLAUDE.md, Avatar state model).

use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;

/// One line in a session's rolling activity log — the raw material for the
/// dashboard's terminal preview. `kind` is the block's own type ("tool" for a
/// tool call, "text" for spoken output, "think" for thinking), distinct from the
/// working/waiting UI state on `SessionView`. `seq` is monotonic per session so
/// the frontend can key each line stably and only animate genuinely-new ones.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub seq: u64,
    pub kind: String,
    pub text: String,
}

/// One active session, shaped for the webviews (camelCase JSON).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub id: String,
    pub project: String,
    pub path: String,
    pub branch: Option<String>,
    /// Context fill, 0..100, one decimal.
    pub pct: f64,
    pub used: u64,
    pub limit: u64,
    pub model: String,
    /// "run" (working) or "ask" (input-requested — reserved for Phase 4).
    pub state: String,
    /// "ok" | "warn" | "crit" per the configured thresholds.
    pub level: String,
    /// Transcript mtime, ms since epoch.
    pub last_active: i64,
    /// Claude Code's AI-generated one-line title for the session (window-render).
    /// Disambiguates same-project sessions; None until Claude Code writes one.
    pub title: Option<String>,
    /// One-line preview of the session's current activity, e.g. "editing
    /// scanner.rs", "running cargo check". Empty until an assistant line lands.
    pub activity: String,
    /// UI hint for the activity: "working" | "waiting" | "thinking" | "".
    pub activity_kind: String,
    /// Rolling tail of recent activity lines for the dashboard terminal preview
    /// (oldest → newest, capped). Empty until an assistant line lands.
    pub activity_log: Vec<ActivityEntry>,
    /// Whether Tablo knows where this session lives (drives the "jump" button).
    /// Set false by `scan`; the emit path overlays it from the location cache.
    pub can_jump: bool,
}

/// Full aggregate pushed to every window as the `state-update` event.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Avatar state: "idle" | "running" | "alarmed".
    pub state: String,
    pub agent_count: u32,
    pub waiting: u32,
    pub projects: u32,
    pub sessions: Vec<SessionView>,
    pub generated_at: i64,
    /// False when `~/.claude/projects` doesn't exist (friendly empty state).
    pub has_projects_dir: bool,
    /// Raw subscription tier (e.g. "default_claude_max_5x"), or None if absent.
    /// Static account metadata — not live quota. Frontend renders a chip.
    pub plan_tier: Option<String>,
    /// Tool calls awaiting a human approve/deny (Phase 4). Populated in the emit
    /// path, not by the scan itself — `scan` always leaves it empty.
    pub pending: Vec<crate::permission::PendingRequest>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            agent_count: 0,
            waiting: 0,
            projects: 0,
            sessions: Vec::new(),
            generated_at: 0,
            has_projects_dir: true,
            plan_tier: None,
            pending: Vec::new(),
        }
    }
}

/// Per-file tail state, kept across scans so we read only appended bytes
/// (CLAUDE.md Step 1.3 — no whole-file re-parse).
#[derive(Default, Clone)]
pub struct FileState {
    offset: u64,
    used: u64,
    model: String,
    cwd: String,
    branch: Option<String>,
    session_id: String,
    /// Sticky once we've inferred the 1M window for this session.
    is_1m: bool,
    /// Live activity preview (window-render): a one-line summary of what the
    /// session is doing right now, plus its kind for UI styling, and Claude
    /// Code's own AI-generated title for the session.
    title: Option<String>,
    activity: String,
    /// "working" | "waiting" | "thinking" | "" (unknown / no assistant line yet).
    activity_kind: String,
    /// Rolling recent-activity buffer (capped at ACTIVITY_LOG_CAP) + its
    /// monotonic sequence source, for the dashboard terminal.
    log: Vec<ActivityEntry>,
    seq: u64,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn projects_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("projects"))
}

/// Cached view of `~/.claude.json`: which project dirs last ran an extended
/// (1M) model. This is the reliable on-disk source for the context window that
/// the transcript itself omits — Claude Code records the model string *with* its
/// `[1m]` marker under `projects[cwd].lastModelUsage`.
#[derive(Default)]
pub struct ClaudeConfigCache {
    loaded: bool,
    mtime: i64,
    /// project cwd -> last session used a `[1m]` model.
    windows: HashMap<String, bool>,
    /// Whether the user predominantly runs the extended window across all
    /// projects with history — the fallback for a project with no record.
    global_ext: Option<bool>,
    /// Raw subscription tier from `oauthAccount.organizationRateLimitTier`
    /// (e.g. "default_claude_max_5x"). The one plan fact that lives on disk —
    /// live 5h/weekly quota does not (Phase 3 was otherwise cancelled). The
    /// frontend maps this to a friendly label.
    plan_tier: Option<String>,
}

fn claude_json_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

/// Re-read `~/.claude.json` when it changes and rebuild the per-project window
/// map from each project's `lastModelUsage` keys.
fn refresh_claude_config(cache: &mut ClaudeConfigCache) {
    let path = match claude_json_path() {
        Some(p) => p,
        None => return,
    };
    let mtime = std::fs::metadata(&path).ok().map(|m| mtime_ms(&m)).unwrap_or(0);
    if cache.loaded && mtime == cache.mtime {
        return; // unchanged since last parse
    }
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return,
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return, // mid-write / malformed — keep the previous map
    };
    let mut windows = HashMap::new();
    if let Some(projects) = v.get("projects").and_then(|p| p.as_object()) {
        for (cwd, entry) in projects {
            if let Some(usage) = entry.get("lastModelUsage").and_then(|m| m.as_object()) {
                if usage.is_empty() {
                    continue;
                }
                let extended = usage.keys().any(|k| {
                    let kl = k.to_lowercase();
                    kl.contains("[1m]") || kl.contains("-1m")
                });
                windows.insert(cwd.clone(), extended);
            }
        }
    }
    // Global lean: is at least half of the projects-with-history on the extended
    // window? Used when a specific project has no record.
    let total = windows.len();
    cache.global_ext = if total > 0 {
        let ext = windows.values().filter(|v| **v).count();
        Some(ext * 2 >= total)
    } else {
        None
    };
    cache.plan_tier = v
        .get("oauthAccount")
        .and_then(|o| o.get("organizationRateLimitTier"))
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    cache.loaded = true;
    cache.mtime = mtime;
    cache.windows = windows;
}

/// Read bytes appended after `from`. Returns `(new_offset, complete_lines)`.
/// A partial trailing line (mid-write) is left unconsumed for the next scan.
fn read_new_lines(path: &Path, from: u64) -> (u64, Vec<String>) {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return (from, Vec::new()),
    };
    if file.seek(SeekFrom::Start(from)).is_err() {
        return (from, Vec::new());
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return (from, Vec::new());
    }
    // Consume only up to and including the last newline.
    let consume = buf.iter().rposition(|b| *b == b'\n').map(|i| i + 1).unwrap_or(0);
    let text = String::from_utf8_lossy(&buf[..consume]);
    let lines = text
        .split('\n')
        .filter(|l| !l.trim().is_empty())
        .map(|s| s.to_string())
        .collect();
    (from + consume as u64, lines)
}

/// `input + cache_creation + cache_read + output` from a usage object.
fn usage_total(usage: &serde_json::Value) -> u64 {
    let g = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    g("input_tokens")
        + g("cache_creation_input_tokens")
        + g("cache_read_input_tokens")
        + g("output_tokens")
}

/// Pick the context denominator (Open Question #4). Priority:
/// 1. **Certain** extended signals — already-sticky, a `[1m]` marker in the
///    model string, or usage past the standard window (a standard session
///    compacts before it could get there).
/// 2. A `window_hint` from `~/.claude.json` — the project's last-used model, or
///    the user's global lean — the reliable on-disk answer for the ambiguous
///    sub-standard case.
/// 3. Otherwise fall back to `default_context_limit`.
/// All sizes come from config.
fn resolve_limit(
    model: &str,
    used: u64,
    sticky_ext: bool,
    window_hint: Option<bool>,
    cfg: &Config,
) -> (u64, bool) {
    let m = model.to_lowercase();
    let marker = cfg
        .extended_window_markers
        .iter()
        .any(|mk| !mk.is_empty() && m.contains(&mk.to_lowercase()));
    let certain_ext = sticky_ext || marker || used > cfg.standard_context_limit;
    let is_ext = certain_ext || window_hint == Some(true);
    let limit = if is_ext {
        cfg.extended_context_limit
    } else if window_hint == Some(false) {
        cfg.standard_context_limit // definitively on the standard window
    } else {
        cfg.default_context_limit // no signal at all — configured fallback
    };
    (limit, is_ext)
}

/// Claude Code names each project slug dir after the launch cwd, encoding
/// separators as dashes.
fn encode_path(p: &str) -> String {
    p.chars()
        .map(|c| if c == '/' || c == '.' || c == ' ' { '-' } else { c })
        .collect()
}

/// Resolve the human project name + root path. A session may `cd` into a
/// subdirectory (e.g. `src-tauri`), so `basename(cwd)` is unreliable; instead we
/// walk cwd's ancestors and pick the one whose encoded form matches the slug dir
/// — that's the directory the session was launched from.
fn resolve_project(slug: &str, cwd: &str) -> (String, String) {
    if !cwd.is_empty() {
        let mut cur = Path::new(cwd);
        loop {
            if let Some(cs) = cur.to_str() {
                if encode_path(cs) == slug {
                    let name = cur
                        .file_name()
                        .and_then(|s| s.to_str())
                        .filter(|s| !s.is_empty())
                        .unwrap_or(slug)
                        .to_string();
                    return (name, cs.to_string());
                }
            }
            match cur.parent() {
                Some(p) => cur = p,
                None => break,
            }
        }
    }
    let name = Path::new(cwd)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(slug)
        .to_string();
    let path = if cwd.is_empty() { slug.to_string() } else { cwd.to_string() };
    (name, path)
}

fn abbreviate_home(path: &str) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Some(h) = home.to_str() {
            if let Some(rest) = path.strip_prefix(h) {
                return format!("~{}", rest);
            }
        }
    }
    path.to_string()
}

fn level_for(pct: f64, cfg: &Config) -> &'static str {
    if pct >= cfg.crit_pct {
        "crit"
    } else if pct >= cfg.warn_pct {
        "warn"
    } else {
        "ok"
    }
}

/// Ingest newly-appended lines into a file's tail state.
fn ingest(state: &mut FileState, lines: &[String]) {
    for line in lines {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // malformed/partial line — ignore (Step 1.3)
        };
        // cwd / branch / session id ride along on every event.
        if let Some(cwd) = v.get("cwd").and_then(|x| x.as_str()) {
            if !cwd.is_empty() {
                state.cwd = cwd.to_string();
            }
        }
        if let Some(b) = v.get("gitBranch").and_then(|x| x.as_str()) {
            state.branch = if b.is_empty() { None } else { Some(b.to_string()) };
        }
        if let Some(sid) = v.get("sessionId").and_then(|x| x.as_str()) {
            state.session_id = sid.to_string();
        }
        match v.get("type").and_then(|x| x.as_str()) {
            // Context occupancy comes from the *latest* assistant usage block;
            // the same line also updates the live activity preview.
            Some("assistant") => {
                if let Some(msg) = v.get("message") {
                    if let Some(usage) = msg.get("usage") {
                        state.used = usage_total(usage);
                    }
                    if let Some(model) = msg.get("model").and_then(|x| x.as_str()) {
                        state.model = model.to_string();
                    }
                    update_activity(state, msg);
                }
            }
            // Claude Code's own AI-generated session title.
            Some("ai-title") => {
                if let Some(t) = v.get("aiTitle").and_then(|x| x.as_str()) {
                    let t = t.trim();
                    if !t.is_empty() {
                        state.title = Some(t.to_string());
                    }
                }
            }
            // A fresh user prompt means the agent is about to work again — clear a
            // stale "waiting for you" until the next assistant line lands. Tool
            // results (content is a list of tool_result blocks) are not prompts.
            Some("user") => {
                // The human's own typed prompt gets its own line in the tail.
                if let Some(text) = typed_prompt_text(&v) {
                    push_log(state, "user", &text);
                }
                if is_user_prompt(v.get("message")) {
                    state.activity = "thinking…".into();
                    state.activity_kind = "working".into();
                }
            }
            _ => {}
        }
    }
}

/// True when a `user` line is a real human prompt, not a tool_result carrier.
fn is_user_prompt(msg: Option<&serde_json::Value>) -> bool {
    match msg.and_then(|m| m.get("content")) {
        Some(serde_json::Value::String(s)) => !s.trim().is_empty(),
        Some(serde_json::Value::Array(blocks)) => !blocks.iter().any(|b| {
            b.get("type").and_then(|t| t.as_str()) == Some("tool_result")
        }),
        _ => false,
    }
}

/// Derive the live activity from an assistant message. Claude Code writes
/// thinking / text / tool_use as their own lines, so the newest assistant line
/// is the current action. Every meaningful block is appended to the rolling log
/// (the terminal tail); the last block also drives the one-line preview + the
/// working/waiting/thinking state used by both surfaces.
fn update_activity(state: &mut FileState, msg: &serde_json::Value) {
    let end_turn = msg.get("stop_reason").and_then(|v| v.as_str()) == Some("end_turn");
    // Meaningful blocks in order: (block-kind, text).
    let mut blocks: Vec<(&'static str, String)> = Vec::new();
    match msg.get("content") {
        Some(serde_json::Value::String(s)) => {
            let sn = snippet(s);
            if !sn.is_empty() {
                blocks.push(("text", sn));
            }
        }
        Some(serde_json::Value::Array(arr)) => {
            for b in arr {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("tool_use") => {
                        let name = b.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
                        let input = b.get("input").cloned().unwrap_or(serde_json::Value::Null);
                        blocks.push(("tool", summarize_activity(name, &input)));
                    }
                    Some("text") => {
                        let sn = snippet(b.get("text").and_then(|x| x.as_str()).unwrap_or(""));
                        if !sn.is_empty() {
                            blocks.push(("text", sn));
                        }
                    }
                    Some("thinking") => blocks.push(("think", "thinking…".into())),
                    _ => {}
                }
            }
        }
        _ => {}
    }

    if blocks.is_empty() {
        // A bare end-of-turn (e.g. a stop with no renderable block): the agent
        // handed back to the user.
        if end_turn {
            state.activity_kind = "waiting".into();
        }
        return;
    }

    // Append every block to the rolling terminal log.
    for (kind, text) in &blocks {
        push_log(state, kind, text);
    }

    // The last block is the current action → the single-line preview + state.
    let (last_kind, last_text) = blocks.last().unwrap();
    state.activity = last_text.clone();
    state.activity_kind = match *last_kind {
        "tool" => "working",
        "think" => "thinking",
        // A finished text turn means the agent handed back to the user.
        _ if end_turn => "waiting",
        _ => "working",
    }
    .into();
}

const ACTIVITY_MAX: usize = 52;
/// Recent-activity lines retained per session for the terminal tail.
const ACTIVITY_LOG_CAP: usize = 8;

/// Append one line to the rolling terminal log, skipping an exact repeat of the
/// last line (a re-emitted message shouldn't duplicate) and capping the buffer.
fn push_log(state: &mut FileState, kind: &str, text: &str) {
    let dup = state
        .log
        .last()
        .map(|e| e.kind == kind && e.text == text)
        .unwrap_or(false);
    if dup {
        return;
    }
    state.seq += 1;
    state.log.push(ActivityEntry {
        seq: state.seq,
        kind: kind.into(),
        text: text.into(),
    });
    if state.log.len() > ACTIVITY_LOG_CAP {
        state.log.drain(0..state.log.len() - ACTIVITY_LOG_CAP);
    }
}

/// The clean text of a prompt the human actually typed, or None. Claude Code
/// marks these with `promptSource: "typed"`; tool-results and auto-injected
/// context ride other `user` lines without it. Content is a plain string, or a
/// block list whose `text` blocks we join.
fn typed_prompt_text(v: &serde_json::Value) -> Option<String> {
    if v.get("promptSource").and_then(|s| s.as_str()) != Some("typed") {
        return None;
    }
    let content = v.get("message").and_then(|m| m.get("content"))?;
    let raw = match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
            .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(" "),
        _ => return None,
    };
    let sn = snippet(&raw);
    if sn.is_empty() {
        None
    } else {
        Some(sn)
    }
}

fn base_name(p: &str) -> &str {
    Path::new(p).file_name().and_then(|s| s.to_str()).unwrap_or(p)
}

fn url_host(u: &str) -> &str {
    let after = u.split_once("://").map(|(_, r)| r).unwrap_or(u);
    after.split('/').next().unwrap_or(after)
}

fn arg<'a>(input: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    input.get(key).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

/// Human, verb-led one-liner for a tool call ("editing scanner.rs", "running
/// cargo check"). Richer than the permission card's raw preview — this is the
/// at-a-glance "what's happening" line. The leading icon/state suffix are added
/// by the UI from `activity_kind`, so this returns just the phrase.
fn summarize_activity(tool: &str, input: &serde_json::Value) -> String {
    let s: String = match tool {
        "Bash" => arg(input, "description")
            .map(str::to_string)
            .or_else(|| arg(input, "command").map(|c| format!("running {c}")))
            .unwrap_or_else(|| "running a command".into()),
        "Read" => arg(input, "file_path")
            .map(|p| format!("reading {}", base_name(p)))
            .unwrap_or_else(|| "reading a file".into()),
        "Edit" | "MultiEdit" => arg(input, "file_path")
            .map(|p| format!("editing {}", base_name(p)))
            .unwrap_or_else(|| "editing a file".into()),
        "Write" => arg(input, "file_path")
            .map(|p| format!("writing {}", base_name(p)))
            .unwrap_or_else(|| "writing a file".into()),
        "NotebookEdit" => arg(input, "notebook_path")
            .map(|p| format!("editing {}", base_name(p)))
            .unwrap_or_else(|| "editing a notebook".into()),
        "Grep" => arg(input, "pattern")
            .map(|p| format!("searching \"{p}\""))
            .unwrap_or_else(|| "searching the code".into()),
        "Glob" => arg(input, "pattern")
            .map(|p| format!("finding {p}"))
            .unwrap_or_else(|| "finding files".into()),
        "Task" | "Agent" => arg(input, "description")
            .or_else(|| arg(input, "subagent_type"))
            .map(|d| format!("agent: {d}"))
            .unwrap_or_else(|| "running an agent".into()),
        "WebFetch" => arg(input, "url")
            .map(|u| format!("fetching {}", url_host(u)))
            .unwrap_or_else(|| "fetching a page".into()),
        "WebSearch" => arg(input, "query")
            .map(|q| format!("searching the web: {q}"))
            .unwrap_or_else(|| "searching the web".into()),
        "TodoWrite" => "updating the plan".into(),
        "ExitPlanMode" => "presenting a plan".into(),
        "AskUserQuestion" => "asking you a question".into(),
        t if t.starts_with("Task") => "planning".into(),
        t if t.starts_with("mcp__") => {
            let mut it = t.splitn(3, "__");
            match (it.next(), it.next(), it.next()) {
                (_, Some(server), Some(name)) => format!("{server}: {}", name.replace('_', " ")),
                _ => t.to_string(),
            }
        }
        other => other.to_string(),
    };
    truncate_activity(&s, ACTIVITY_MAX)
}

/// Collapse whitespace and trim leading markdown so a text block reads as one
/// clean line.
fn snippet(s: &str) -> String {
    let cleaned = s.trim_start_matches(|c: char| "#*->` \t".contains(c));
    let one_line = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_activity(&one_line, ACTIVITY_MAX)
}

fn truncate_activity(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", head.trim_end())
    }
}

/// One full pass: refresh tail state for every active transcript and build the
/// aggregate snapshot. `files` is retained across calls for incremental reads.
pub fn scan(
    cfg: &Config,
    files: &mut HashMap<PathBuf, FileState>,
    claude_cfg: &mut ClaudeConfigCache,
) -> Snapshot {
    refresh_claude_config(claude_cfg);
    let dir = match projects_dir() {
        Some(d) => d,
        None => {
            return Snapshot {
                has_projects_dir: false,
                generated_at: now_ms(),
                ..Default::default()
            }
        }
    };
    if !dir.exists() {
        return Snapshot {
            has_projects_dir: false,
            generated_at: now_ms(),
            ..Default::default()
        };
    }

    let now = now_ms();
    let active_ms = cfg.active_window_secs as i64 * 1000;
    let mut sessions: Vec<SessionView> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    // project slug dirs
    let project_dirs = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return Snapshot { generated_at: now, ..Default::default() },
    };

    for pd in project_dirs.flatten() {
        let pdir = pd.path();
        if !pdir.is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(&pdir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for e in entries.flatten() {
            let path = e.path();
            // top-level *.jsonl only (skips subagents/…)
            if !path.is_file() || path.extension().and_then(|x| x.to_str()) != Some("jsonl") {
                continue;
            }
            let meta = match e.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mtime = mtime_ms(&meta);
            if now - mtime > active_ms {
                continue; // not active (Step 1.2)
            }
            seen.push(path.clone());

            let size = meta.len();
            let st = files.entry(path.clone()).or_default();
            // Truncation / rotation → reset the tail.
            if size < st.offset {
                *st = FileState::default();
            }
            // First sight of a large file: skip to a bounded tail (we only need
            // the file's end for the latest usage).
            if st.offset == 0 && size > cfg.initial_tail_cap_bytes {
                st.offset = size - cfg.initial_tail_cap_bytes;
            }
            if size > st.offset {
                let (new_offset, lines) = read_new_lines(&path, st.offset);
                st.offset = new_offset;
                ingest(st, &lines);
            }

            let slug = pdir.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let (project, root) = resolve_project(slug, &st.cwd);
            // Definitive window from ~/.claude.json, keyed by the project root
            // (falling back to the raw cwd).
            let project_ext = claude_cfg
                .windows
                .get(&root)
                .or_else(|| claude_cfg.windows.get(&st.cwd))
                .copied();
            // Per-project record wins; else the user's global lean.
            let window_hint = project_ext.or(claude_cfg.global_ext);

            let (limit, is_ext) = resolve_limit(&st.model, st.used, st.is_1m, window_hint, cfg);
            st.is_1m = is_ext;
            let pct = if limit > 0 {
                ((st.used as f64 / limit as f64) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            let pct = (pct * 10.0).round() / 10.0;

            let session_id = if st.session_id.is_empty() {
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("session").to_string()
            } else {
                st.session_id.clone()
            };

            sessions.push(SessionView {
                id: session_id,
                project,
                path: abbreviate_home(&root),
                branch: st.branch.clone(),
                pct,
                used: st.used,
                limit,
                model: st.model.clone(),
                state: "run".into(), // Phase 4 will surface "ask"
                level: level_for(pct, cfg).into(),
                last_active: mtime,
                title: st.title.clone(),
                activity: st.activity.clone(),
                activity_kind: st.activity_kind.clone(),
                activity_log: st.log.clone(),
                can_jump: false, // overlaid in the emit path from the location cache
            });
        }
    }

    // Drop tail state for files no longer active / removed, so the map can't
    // grow without bound.
    files.retain(|k, _| seen.contains(k));

    // Sort: input-requested first (Phase 4), then by highest context fill.
    sessions.sort_by(|a, b| {
        let ask = |s: &SessionView| (s.state != "ask") as u8;
        ask(a)
            .cmp(&ask(b))
            .then(b.pct.partial_cmp(&a.pct).unwrap_or(std::cmp::Ordering::Equal))
    });

    let agent_count = sessions.len() as u32;
    let waiting = sessions.iter().filter(|s| s.state == "ask").count() as u32;
    let projects = {
        let mut p: Vec<&str> = sessions.iter().map(|s| s.path.as_str()).collect();
        p.sort();
        p.dedup();
        p.len() as u32
    };
    let state = if sessions.is_empty() {
        "idle"
    } else if sessions.iter().any(|s| s.level != "ok") {
        "alarmed"
    } else {
        "running"
    };

    Snapshot {
        state: state.into(),
        agent_count,
        waiting,
        projects,
        sessions,
        generated_at: now,
        has_projects_dir: true,
        plan_tier: claude_cfg.plan_tier.clone(),
        pending: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Smoke test against the real `~/.claude/projects` transcripts. Prints the
    /// computed snapshot so we can eyeball the Phase 1 data path end-to-end.
    #[test]
    fn scan_real_transcripts() {
        let cfg = Config::default();
        let mut files = HashMap::new();
        let mut claude_cfg = ClaudeConfigCache::default();
        let snap = scan(&cfg, &mut files, &mut claude_cfg);
        println!(
            "\nsnapshot: state={} active={} projects={} waiting={} has_dir={}",
            snap.state, snap.agent_count, snap.projects, snap.waiting, snap.has_projects_dir
        );
        for s in &snap.sessions {
            println!(
                "  {:<22} [{:^4}] {:>5}%  {:>8}/{:<9} branch={:?}",
                s.project, s.level, s.pct, s.used, s.limit, s.branch
            );
            println!(
                "      title={:?}\n      activity=[{}] {}",
                s.title, s.activity_kind, s.activity
            );
            for e in &s.activity_log {
                println!("        {:>3} {:<5} {}", e.seq, e.kind, e.text);
            }
        }
        // Every reported session must have a sane percentage.
        for s in &snap.sessions {
            assert!(s.pct >= 0.0 && s.pct <= 100.0, "pct out of range: {}", s.pct);
            assert!(s.limit > 0, "limit must be positive");
        }
    }

    #[test]
    fn activity_summaries_are_human() {
        use serde_json::json;
        // Bash prefers the human description, falls back to the command.
        assert_eq!(
            summarize_activity("Bash", &json!({ "command": "cargo check", "description": "Type-check the crate" })),
            "Type-check the crate"
        );
        assert_eq!(summarize_activity("Bash", &json!({ "command": "ls -la" })), "running ls -la");
        // File tools show just the basename with a verb.
        assert_eq!(
            summarize_activity("Edit", &json!({ "file_path": "/Users/x/proj/src/scanner.rs" })),
            "editing scanner.rs"
        );
        assert_eq!(
            summarize_activity("Read", &json!({ "file_path": "/a/b/Panel.svelte" })),
            "reading Panel.svelte"
        );
        // MCP tools decode server + tool name.
        assert_eq!(
            summarize_activity("mcp__wel__list_projects", &json!({})),
            "wel: list projects"
        );
    }

    #[test]
    fn activity_tracks_tool_then_waits_on_end_turn() {
        let mut st = FileState::default();
        // A tool call ⇒ working.
        ingest(
            &mut st,
            &[r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Read","input":{"file_path":"/x/lib.rs"}}]}}"#.into()],
        );
        assert_eq!(st.activity_kind, "working");
        assert_eq!(st.activity, "reading lib.rs");
        // The rolling terminal log captured the tool line.
        assert_eq!(st.log.len(), 1);
        assert_eq!((st.log[0].kind.as_str(), st.log[0].text.as_str()), ("tool", "reading lib.rs"));

        // A finished text turn ⇒ waiting for the user.
        ingest(
            &mut st,
            &[r#"{"type":"assistant","message":{"stop_reason":"end_turn","content":[{"type":"text","text":"All done — tests pass."}]}}"#.into()],
        );
        assert_eq!(st.activity_kind, "waiting");
        assert_eq!(st.activity, "All done — tests pass.");
        // Log now has the tool line then the output line, with rising seqs.
        assert_eq!(st.log.len(), 2);
        assert_eq!(st.log[1].kind, "text");
        assert!(st.log[1].seq > st.log[0].seq);

        // The AI title rides along.
        ingest(&mut st, &[r#"{"type":"ai-title","aiTitle":"Fix the scanner"}"#.into()]);
        assert_eq!(st.title.as_deref(), Some("Fix the scanner"));

        // A typed user prompt clears the stale "waiting" AND joins the tail.
        ingest(
            &mut st,
            &[r#"{"type":"user","promptSource":"typed","message":{"content":"now do X"}}"#.into()],
        );
        assert_eq!(st.activity_kind, "working");
        assert_eq!(st.log.len(), 3);
        assert_eq!((st.log[2].kind.as_str(), st.log[2].text.as_str()), ("user", "now do X"));

        // A tool_result is neither a typed prompt nor a reset — no log line, no
        // activity overwrite.
        st.activity = "editing a.rs".into();
        st.activity_kind = "working".into();
        ingest(
            &mut st,
            &[r#"{"type":"user","message":{"content":[{"type":"tool_result","content":"ok"}]}}"#.into()],
        );
        assert_eq!(st.activity, "editing a.rs", "tool_result should not overwrite activity");
        assert_eq!(st.log.len(), 3, "tool_result must not add a log line");
    }
}
