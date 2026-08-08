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
    /// The session's permission mode for the read-only badge: "normal" | "auto"
    /// | "plan" | "bypass". Derived from the last *resting* `permissionMode`;
    /// Claude Code's transient "auto" execution-heartbeat is filtered out.
    pub mode: String,
    /// "ok" | "warn" | "crit" per the configured thresholds.
    pub level: String,
    /// Whether the context window (the denominator behind `pct`/`limit`) was
    /// actually resolved from real signals. False = it's only a configured
    /// fallback: the UI greys the meter and shows no percentage, and `pct` is
    /// forced to 0 / `level` to "ok" so a guess can never alarm.
    pub ctx_resolved: bool,
    /// Last conversational activity (user/agent line, from the line's own
    /// timestamp), ms since epoch; transcript mtime until one is seen.
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
    /// Which agent this session belongs to: "claude" | "codex". Drives the small
    /// source tag on the session row.
    pub source: String,
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
    /// False only when *neither* agent has a data dir (`~/.claude/projects` and
    /// `~/.codex/sessions` both missing) — the friendly "nothing yet" empty state.
    pub has_projects_dir: bool,
    /// Raw subscription tier (e.g. "default_claude_max_5x"), or None if absent.
    /// Static account metadata — not live quota. Frontend renders a chip.
    pub plan_tier: Option<String>,
    /// Tool calls awaiting a human approve/deny (Phase 4). Populated in the emit
    /// path, not by the scan itself — `scan` always leaves it empty.
    pub pending: Vec<crate::permission::PendingRequest>,
    /// Context warn threshold (percent), echoed so every window can label the
    /// Critical group ("> N%") without its own config fetch.
    pub warn_pct: f64,
    /// Early-cancel grace window (minutes), echoed so the Settings input shows
    /// the live floored value.
    pub cancel_grace_mins: u64,
    /// Waiting-session clear window (minutes), echoed for the Settings input.
    pub clear_waiting_mins: u64,
    /// Whether Codex sessions are being watched, echoed for the Settings toggle.
    pub watch_codex: bool,
    /// Whether the global panel-summon hotkey is enabled, echoed for the Settings toggle.
    pub panel_shortcut_enabled: bool,
    /// Whether Tablo follows the focused AeroSpace workspace, echoed for the Settings toggle.
    pub aerospace_follow: bool,
    /// Whether anonymous usage pings are enabled, echoed for the Settings toggle.
    pub telemetry_enabled: bool,
    /// Whether new releases install themselves, echoed for the Settings toggle.
    pub auto_update: bool,
    /// Version of a release that was found but *not* installed (auto-update off),
    /// so the dashboard can offer a one-tap install. Populated in the emit path,
    /// not by the scan itself — `scan` always leaves it None.
    pub update_available: Option<String>,
    /// Release notes for the running build, unless already seen. Set in the emit
    /// path — `scan` always leaves it None.
    pub whats_new: Option<crate::whatsnew::ReleaseNotes>,
    /// Whether AeroSpace was detected this run — gates the "Follow AeroSpace"
    /// toggle so it only appears to users actually running AeroSpace.
    pub aerospace_available: bool,
    /// Whether "jump to session" is supported — macOS only (window-raise is a
    /// no-op elsewhere). Gates the Jump toggles off Windows/Linux.
    pub jump_supported: bool,
    /// Whether tool approvals are supported — the hook is a POSIX sh + curl
    /// script, so everywhere except Windows. Gates the approvals toggle.
    pub approvals_supported: bool,
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
            warn_pct: 60.0,
            cancel_grace_mins: 3,
            clear_waiting_mins: 10,
            watch_codex: true,
            panel_shortcut_enabled: true,
            aerospace_follow: true,
            telemetry_enabled: true,
            auto_update: true,
            update_available: None,
            whats_new: None,
            aerospace_available: false,
            jump_supported: cfg!(target_os = "macos"),
            approvals_supported: !cfg!(target_os = "windows"),
        }
    }
}

/// Which agent produced a transcript. Selects the parser and a few source-specific
/// render choices (project resolution, context-window sizing, mode badge).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Source {
    #[default]
    Claude,
    Codex,
}

/// Per-file tail state, kept across scans so we read only appended bytes
/// (CLAUDE.md Step 1.3 — no whole-file re-parse). Fields are `pub(crate)` so the
/// sibling `codex` parser can populate the same shape.
#[derive(Default, Clone)]
pub struct FileState {
    /// Which agent this transcript belongs to (set by the scan from the file's
    /// directory; picks the parser). Default Claude.
    pub(crate) source: Source,
    pub(crate) offset: u64,
    pub(crate) used: u64,
    pub(crate) model: String,
    pub(crate) cwd: String,
    pub(crate) branch: Option<String>,
    pub(crate) session_id: String,
    /// Last *resting* permission mode. For Claude Code the raw value ("default" |
    /// "acceptEdits" | "plan" | "bypassPermissions"); for Codex the raw
    /// `approval_policy`. Mapped to the badge by each source's `display_mode`.
    /// Empty until first seen.
    pub(crate) mode: String,
    /// Sticky once we've inferred the 1M window for this session (Claude only).
    pub(crate) is_1m: bool,
    /// Context window reported verbatim by the transcript (Codex `token_count` /
    /// `task_started` `model_context_window`). 0 = unknown → source fallback.
    pub(crate) ctx_window: u64,
    /// Live activity preview (window-render): a one-line summary of what the
    /// session is doing right now, plus its kind for UI styling, and (Claude
    /// Code's) AI-generated title / (Codex) first-prompt stand-in title.
    pub(crate) title: Option<String>,
    pub(crate) activity: String,
    /// "working" | "waiting" | "thinking" | "" (unknown / no assistant line yet).
    pub(crate) activity_kind: String,
    /// True after a typed prompt set the optimistic "thinking…" placeholder but
    /// before any assistant line has confirmed the turn. Cleared by the first
    /// assistant line (real work) or an interrupt. While true, a long transcript
    /// silence is read as an early cancel (see `cancel_grace_mins`).
    pub(crate) awaiting_first: bool,
    /// Rolling recent-activity buffer (capped at ACTIVITY_LOG_CAP) + its
    /// monotonic sequence source, for the dashboard terminal.
    pub(crate) log: Vec<ActivityEntry>,
    pub(crate) seq: u64,
    /// Epoch ms of the newest *conversational* line (Claude user/assistant;
    /// Codex event/response), read from the line's own timestamp. Metadata
    /// appends (ai-title, mode, permission-mode) don't move it — Claude Code
    /// batch-flushes those to idle sessions, bumping mtime with no real
    /// activity, so presence and waiting-clear judge silence by this instead.
    /// 0 = none seen yet (fall back to mtime).
    pub(crate) last_convo: i64,
}

/// Epoch ms from a transcript line's ISO-8601 UTC timestamp
/// ("2026-08-02T07:43:00.123Z"). Fixed-format — both agents write Zulu; None on
/// anything else so callers fall back to mtime.
pub(crate) fn parse_iso_ms(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| s.get(r).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut ms = 0i64;
    if b.len() > 19 && b[19] == b'.' {
        let frac: String = s[20..].chars().take_while(|c| c.is_ascii_digit()).collect();
        let take = frac.len().min(3);
        if take > 0 {
            ms = frac[..take].parse::<i64>().ok()? * [100, 10, 1][take - 1];
        }
    }
    // Civil date → days since epoch (Howard Hinnant's days_from_civil).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = if y2 >= 0 { y2 } else { y2 - 399 } / 400;
    let yoe = y2 - era * 400;
    let doy = (153 * ((mo + 9) % 12) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400_000 + (h * 3600 + mi * 60 + sec) * 1000 + ms)
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub(crate) fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
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
                    kl.contains("[1m]") || kl.contains("-1m") || native_extended(&kl)
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

/// Read a whole transcript into its non-empty lines. Test-only convenience for
/// the smoke tests that parse a real rollout end-to-end.
#[cfg(test)]
pub(crate) fn read_all_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .map(|t| t.lines().filter(|l| !l.trim().is_empty()).map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

/// `input + cache_creation + cache_read + output` from a usage object.
fn usage_total(usage: &serde_json::Value) -> u64 {
    let g = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    g("input_tokens")
        + g("cache_creation_input_tokens")
        + g("cache_read_input_tokens")
        + g("output_tokens")
}

/// Models whose context window is natively extended: Claude Code writes NO
/// `[1m]` marker for them — the suffix only exists where 1M is an opt-in
/// variant of a standard model. For these the model name itself is the certain
/// signal. Lowercase substring match against the model string, like the config
/// markers; kept as a built-in list (not a config default) because the marker
/// list is persisted in existing configs and a new default would never load.
const NATIVE_EXTENDED_MODELS: &[&str] = &["fable", "mythos"];

fn native_extended(model_lower: &str) -> bool {
    NATIVE_EXTENDED_MODELS.iter().any(|m| model_lower.contains(m))
}

/// Pick the context denominator (Open Question #4). Priority:
/// 1. **Certain** extended signals — already-sticky, a `[1m]` marker in the
///    model string, a natively-extended model name, or usage past the standard
///    window (a standard session compacts before it could get there).
/// 2. A `window_hint` from `~/.claude.json` — the project's last-used model, or
///    the user's global lean — the reliable on-disk answer for the ambiguous
///    sub-standard case.
/// 3. Otherwise the window is **unresolved** (`resolved` = false): the returned
///    limit is only the configured fallback and the UI must not present a
///    percentage computed from it as fact.
/// All sizes come from config. Returns (limit, is_ext, resolved).
fn resolve_limit(
    model: &str,
    used: u64,
    sticky_ext: bool,
    window_hint: Option<bool>,
    cfg: &Config,
) -> (u64, bool, bool) {
    let m = model.to_lowercase();
    let marker = cfg
        .extended_window_markers
        .iter()
        .any(|mk| !mk.is_empty() && m.contains(&mk.to_lowercase()));
    let certain_ext = sticky_ext || marker || native_extended(&m) || used > cfg.standard_context_limit;
    let is_ext = certain_ext || window_hint == Some(true);
    let (limit, resolved) = if is_ext {
        (cfg.extended_context_limit, true)
    } else if window_hint == Some(false) {
        (cfg.standard_context_limit, true) // definitively on the standard window
    } else {
        (cfg.default_context_limit, false) // no signal at all — a guess, flagged
    };
    (limit, is_ext, resolved)
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

/// Map Claude Code's raw `permissionMode` to the read-only badge label. Both
/// "auto" (current name) and "acceptEdits" (older name) are the ⏵⏵ accept-edits
/// mode. A session with no explicit mode signal reads "normal".
fn display_mode(raw: &str) -> &'static str {
    match raw {
        "auto" | "acceptEdits" => "auto",
        "plan" => "plan",
        "bypassPermissions" => "bypass",
        _ => "normal",
    }
}

/// Ingest newly-appended lines into a file's tail state.
fn ingest(state: &mut FileState, lines: &[String]) {
    for line in lines {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue, // malformed/partial line — ignore (Step 1.3)
        };
        // Only user/assistant lines are conversation; metadata lines (ai-title,
        // mode, permission-mode…) carry no timestamp and must not refresh it.
        if matches!(v.get("type").and_then(|x| x.as_str()), Some("user") | Some("assistant")) {
            if let Some(ts) = v.get("timestamp").and_then(|x| x.as_str()).and_then(parse_iso_ms) {
                state.last_convo = state.last_convo.max(ts);
            }
        }
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
        // Permission mode rides top-level on the meta `permission-mode` line and
        // on typed prompts. Last value wins — the tail reflects the session's
        // current mode ("auto" = the user's ⏵⏵ accept-edits mode; "default" =
        // normal). Skip an empty value so a value-less line never clears it.
        if let Some(pm) = v.get("permissionMode").and_then(|x| x.as_str()) {
            if !pm.is_empty() {
                state.mode = pm.to_string();
            }
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
                    // An assistant line confirms the turn is really running, so
                    // the "awaiting first line" cancel-guard no longer applies.
                    state.awaiting_first = false;
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
                // A manual interrupt (Esc / Ctrl+C) writes a `user` line that
                // looks like a prompt (a bare "[Request interrupted…]" text
                // block, no tool_result), but the agent has actually stopped and
                // handed control back. Treat it as waiting-on-you so the avatar
                // drops out of "working" instead of showing a phantom "thinking…".
                if is_interrupt(&v) {
                    state.activity = "interrupted".into();
                    state.activity_kind = "waiting".into();
                    state.awaiting_first = false;
                } else if is_user_prompt(v.get("message")) {
                    state.activity = "thinking…".into();
                    state.activity_kind = "working".into();
                    // Optimistic — not yet confirmed by an assistant line. If the
                    // user Ctrl+C's before the assistant responds, no marker is
                    // written; the cancel-guard downgrades this after a silence.
                    state.awaiting_first = true;
                }
            }
            _ => {}
        }
    }
}

/// True when a `user` line is a manual interrupt (Esc / Ctrl+C) rather than a
/// prompt or tool result. Claude Code tags these with a top-level
/// `interruptedMessageId` and/or a leading "[Request interrupted…]" text block;
/// the field is present on only some of them, so both signals are checked.
fn is_interrupt(v: &serde_json::Value) -> bool {
    if v.get("interruptedMessageId").is_some() {
        return true;
    }
    let starts_with_marker = |t: &str| t.trim_start().starts_with("[Request interrupted");
    match v.get("message").and_then(|m| m.get("content")) {
        Some(serde_json::Value::String(s)) => starts_with_marker(s),
        Some(serde_json::Value::Array(blocks)) => blocks.iter().any(|b| {
            b.get("type").and_then(|t| t.as_str()) == Some("text")
                && b.get("text").and_then(|t| t.as_str()).is_some_and(starts_with_marker)
        }),
        _ => false,
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
    // The log keeps the long form (dashboard terminal); the panel's compact
    // one-liner gets a shorter cut.
    let (last_kind, last_text) = blocks.last().unwrap();
    state.activity = truncate_activity(last_text, ACTIVITY_MAX);
    state.activity_kind = match *last_kind {
        "tool" => "working",
        "think" => "thinking",
        // A finished text turn means the agent handed back to the user.
        _ if end_turn => "waiting",
        _ => "working",
    }
    .into();
}

/// Max chars for the panel's compact single-line activity preview.
pub(crate) const ACTIVITY_MAX: usize = 52;
/// Max chars kept per rolling-log line. The dashboard terminal is far wider than
/// the panel, so it stores a longer line and lets CSS ellipsize the overflow.
pub(crate) const TERM_LINE_MAX: usize = 120;
/// Recent-activity lines retained per session for the terminal tail.
const ACTIVITY_LOG_CAP: usize = 8;

/// Append one line to the rolling terminal log, skipping an exact repeat of the
/// last line (a re-emitted message shouldn't duplicate) and capping the buffer.
pub(crate) fn push_log(state: &mut FileState, kind: &str, text: &str) {
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

pub(crate) fn base_name(p: &str) -> &str {
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
    truncate_activity(&s, TERM_LINE_MAX)
}

/// Collapse whitespace and trim leading markdown so a text block reads as one
/// clean line.
pub(crate) fn snippet(s: &str) -> String {
    let cleaned = s.trim_start_matches(|c: char| "#*->` \t".contains(c));
    let one_line = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_activity(&one_line, TERM_LINE_MAX)
}

pub(crate) fn truncate_activity(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{}…", head.trim_end())
    }
}

/// Context fill as a clamped, one-decimal percentage.
fn pct_of(used: u64, limit: u64) -> f64 {
    if limit == 0 {
        return 0.0;
    }
    let pct = ((used as f64 / limit as f64) * 100.0).clamp(0.0, 100.0);
    (pct * 10.0).round() / 10.0
}

/// Incrementally fold a transcript's newly-appended bytes into its tail state and
/// apply the cross-source guards (early-cancel, waiting-clear). `source` selects
/// the parser and is recorded on the entry (a file's source is fixed by its
/// directory). Returns false when the session should drop from the panel now (a
/// long-finished waiting session). Shared by the Claude Code and Codex walks.
fn tail_file(
    path: &Path,
    size: u64,
    mtime: i64,
    now: i64,
    source: Source,
    files: &mut HashMap<PathBuf, FileState>,
    cfg: &Config,
) -> bool {
    // Enforce the 1-minute floor the UI also guards.
    let cancel_grace_ms = cfg.cancel_grace_mins.max(1) as i64 * 60_000;
    let clear_waiting_ms = cfg.clear_waiting_mins.max(1) as i64 * 60_000;

    let st = files.entry(path.to_path_buf()).or_default();
    st.source = source;
    // Truncation / rotation → reset the tail (keep the known source).
    if size < st.offset {
        *st = FileState { source, ..Default::default() };
    }
    // First sight of a large file: skip to a bounded tail (we only need the
    // file's end for the latest usage).
    if st.offset == 0 && size > cfg.initial_tail_cap_bytes {
        st.offset = size - cfg.initial_tail_cap_bytes;
    }
    if size > st.offset {
        let (new_offset, lines) = read_new_lines(path, st.offset);
        st.offset = new_offset;
        match source {
            Source::Claude => ingest(st, &lines),
            Source::Codex => crate::codex::ingest(st, &lines),
        }
    }

    // Silence is judged by the last *conversational* line when known, not mtime —
    // Claude Code batch-appends metadata to idle sessions, bumping mtime with no
    // real input, which must not reset these clocks.
    let act = if st.last_convo > 0 { st.last_convo } else { mtime };

    // Early-cancel guard (see FileState::awaiting_first): a typed prompt with no
    // assistant line yet, and a transcript silent past the grace window, reads as
    // a Ctrl+C before the response — hand back to the user so the avatar drops out
    // of "working". Checked here (not in ingest) precisely because the tell is the
    // *absence* of new lines.
    if st.awaiting_first && now - act > cancel_grace_ms {
        st.activity_kind = "waiting".into();
        st.activity = "cancelled".into();
        st.awaiting_first = false;
    }

    // Dedicated waiting-clear: a finished (waiting-on-you) session the user hasn't
    // returned to leaves the panel sooner than the general active window. It stays
    // in `seen` (tail state retained), so it reappears the instant they submit
    // again — the activity clock bumps back under the threshold. Working sessions
    // are untouched.
    !(st.activity_kind == "waiting" && now - act > clear_waiting_ms)
}

/// One full pass: refresh tail state for every active transcript (Claude Code and
/// Codex both) and build the aggregate snapshot. `files` is retained across calls
/// for incremental reads, keyed by absolute path so the two source trees never
/// collide.
pub fn scan(
    cfg: &Config,
    files: &mut HashMap<PathBuf, FileState>,
    claude_cfg: &mut ClaudeConfigCache,
) -> Snapshot {
    refresh_claude_config(claude_cfg);
    let claude_dir = projects_dir().filter(|d| d.exists());
    let codex_dir = if cfg.watch_codex {
        crate::codex::sessions_dir().filter(|d| d.exists())
    } else {
        None
    };
    // Neither agent has any data dir → the friendly "nothing yet" empty state.
    if claude_dir.is_none() && codex_dir.is_none() {
        return Snapshot {
            has_projects_dir: false,
            generated_at: now_ms(),
            watch_codex: cfg.watch_codex,
            panel_shortcut_enabled: cfg.panel_shortcut_enabled,
            aerospace_follow: cfg.aerospace_follow,
            auto_update: cfg.auto_update,
            aerospace_available: crate::aerospace::available(),
            ..Default::default()
        };
    }

    let now = now_ms();
    let active_ms = cfg.active_window_secs as i64 * 1000;
    let mut sessions: Vec<SessionView> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    // Claude processes currently running, per the session registry. None when the
    // registry doesn't exist (pre-2.1 Claude Code) → fall back to time-based
    // presence only.
    let live_ids = crate::locate::live_session_ids();

    // ---- Claude Code: ~/.claude/projects/<slug>/*.jsonl ----
    if let Some(dir) = claude_dir {
        if let Ok(project_dirs) = std::fs::read_dir(&dir) {
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
                    if !tail_file(&path, meta.len(), mtime, now, Source::Claude, files, cfg) {
                        continue;
                    }

                    let st = files.get_mut(&path).unwrap();
                    // The mtime gate above is only a cheap pre-filter — once the
                    // tail is parsed, presence is judged by conversational
                    // recency, so a metadata flush can't resurrect a session
                    // whose last real line is outside the window.
                    if st.last_convo > 0 && now - st.last_convo > active_ms {
                        continue;
                    }
                    let slug = pdir.file_name().and_then(|s| s.to_str()).unwrap_or("");
                    let (project, root) = resolve_project(slug, &st.cwd);
                    // Definitive window from ~/.claude.json, keyed by the project
                    // root (falling back to the raw cwd).
                    let project_ext = claude_cfg
                        .windows
                        .get(&root)
                        .or_else(|| claude_cfg.windows.get(&st.cwd))
                        .copied();
                    // Per-project record wins; else the user's global lean.
                    let window_hint = project_ext.or(claude_cfg.global_ext);
                    let (limit, is_ext, ctx_resolved) =
                        resolve_limit(&st.model, st.used, st.is_1m, window_hint, cfg);
                    st.is_1m = is_ext;
                    let st = &*st; // reborrow immutable for the view build
                    let pct = if ctx_resolved { pct_of(st.used, limit) } else { 0.0 };
                    let session_id = if st.session_id.is_empty() {
                        path.file_stem().and_then(|s| s.to_str()).unwrap_or("session").to_string()
                    } else {
                        st.session_id.clone()
                    };
                    // The session's claude process is gone (exit / Ctrl+C removes
                    // its registry row) — the session is over, drop it now rather
                    // than letting it age out of the window.
                    if live_ids.as_ref().is_some_and(|ids| !ids.contains(&session_id)) {
                        continue;
                    }

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
                        mode: display_mode(&st.mode).into(),
                        level: if ctx_resolved { level_for(pct, cfg).into() } else { "ok".into() },
                        ctx_resolved,
                        last_active: if st.last_convo > 0 { st.last_convo } else { mtime },
                        title: st.title.clone(),
                        activity: st.activity.clone(),
                        activity_kind: st.activity_kind.clone(),
                        activity_log: st.log.clone(),
                        can_jump: false, // overlaid in the emit path from the cache
                        source: "claude".into(),
                    });
                }
            }
        }
    }

    // ---- Codex: ~/.codex/sessions/<YYYY>/<MM>/<DD>/rollout-*.jsonl ----
    if let Some(dir) = codex_dir {
        let mut rollouts = Vec::new();
        crate::codex::collect_active(&dir, now, active_ms, &mut rollouts);
        for (path, size, mtime) in rollouts {
            seen.push(path.clone());
            if !tail_file(&path, size, mtime, now, Source::Codex, files, cfg) {
                continue;
            }
            let st = files.get(&path).unwrap();
            // Same conversational-recency gate as the Claude walk.
            if st.last_convo > 0 && now - st.last_convo > active_ms {
                continue;
            }
            let (project, root) = crate::codex::resolve_project(&st.cwd);
            // Codex reports the window verbatim; until the first token_count /
            // task_started lands it's unresolved (config value = display fallback
            // only, never presented as a percentage).
            let ctx_resolved = st.ctx_window > 0;
            let limit = if ctx_resolved { st.ctx_window } else { cfg.codex_context_limit };
            let pct = if ctx_resolved { pct_of(st.used, limit) } else { 0.0 };
            let session_id = if st.session_id.is_empty() {
                crate::codex::session_id_from_path(&path)
                    .unwrap_or_else(|| path.file_stem().and_then(|s| s.to_str()).unwrap_or("session").to_string())
            } else {
                st.session_id.clone()
            };

            sessions.push(SessionView {
                id: session_id,
                project,
                path: abbreviate_home(&root),
                branch: None, // Codex doesn't record the git branch
                pct,
                used: st.used,
                limit,
                model: st.model.clone(),
                state: "run".into(),
                mode: crate::codex::display_mode(&st.mode).into(),
                level: if ctx_resolved { level_for(pct, cfg).into() } else { "ok".into() },
                ctx_resolved,
                last_active: if st.last_convo > 0 { st.last_convo } else { mtime },
                title: st.title.clone(),
                activity: st.activity.clone(),
                activity_kind: st.activity_kind.clone(),
                activity_log: st.log.clone(),
                can_jump: false, // "jump" is Claude-only for now
                source: "codex".into(),
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
    // Avatar state (Avatar State Model), precedence alarmed > running > idle:
    //  - "alarmed": any session's context is in the danger zone (>60%, level != ok).
    //    Permission requests also force "alarmed" in `recompute_and_emit`.
    //  - "running": any session is actively working — anything but waiting-on-you.
    //  - "idle": no sessions, or every one is just waiting.
    let state = if sessions.iter().any(|s| s.level != "ok") {
        "alarmed"
    } else if sessions.iter().any(|s| s.activity_kind != "waiting") {
        "running"
    } else {
        "idle"
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
        warn_pct: cfg.warn_pct,
        cancel_grace_mins: cfg.cancel_grace_mins.max(1),
        clear_waiting_mins: cfg.clear_waiting_mins.max(1),
        watch_codex: cfg.watch_codex,
        panel_shortcut_enabled: cfg.panel_shortcut_enabled,
        aerospace_follow: cfg.aerospace_follow,
        telemetry_enabled: cfg.telemetry_enabled,
        auto_update: cfg.auto_update,
        update_available: None,
        whats_new: None,
        aerospace_available: crate::aerospace::available(),
        jump_supported: cfg!(target_os = "macos"),
        approvals_supported: !cfg!(target_os = "windows"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn iso_timestamps_parse_to_epoch_ms() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_ms("2000-03-01T00:00:00Z"), Some(951_868_800_000));
        // Fractional seconds, 1–3 digits, scale to ms.
        let base = parse_iso_ms("2026-08-02T07:43:00Z").unwrap();
        assert_eq!(parse_iso_ms("2026-08-02T07:43:00.5Z"), Some(base + 500));
        assert_eq!(parse_iso_ms("2026-08-02T07:43:00.123Z"), Some(base + 123));
        assert_eq!(parse_iso_ms("junk"), None);
        assert_eq!(parse_iso_ms(""), None);
        assert_eq!(parse_iso_ms("2026-13-02T07:43:00Z"), None);
    }

    #[test]
    fn metadata_lines_do_not_refresh_conversational_activity() {
        let mut st = FileState::default();
        ingest(
            &mut st,
            &[r#"{"type":"user","timestamp":"2026-08-02T07:00:00Z","message":{"content":"hi"}}"#.into()],
        );
        let t0 = st.last_convo;
        assert!(t0 > 0);
        // The batch metadata flush Claude Code appends to idle sessions: bumps
        // the file's mtime, but must not move the activity clock.
        ingest(
            &mut st,
            &[
                r#"{"type":"ai-title","aiTitle":"Some title","sessionId":"x"}"#.into(),
                r#"{"type":"mode","sessionId":"x"}"#.into(),
                r#"{"type":"permission-mode","permissionMode":"default","sessionId":"x"}"#.into(),
            ],
        );
        assert_eq!(st.last_convo, t0);
        // A real assistant line moves it forward.
        ingest(
            &mut st,
            &[r#"{"type":"assistant","timestamp":"2026-08-02T07:05:00Z","message":{"content":[{"type":"text","text":"done"}],"stop_reason":"end_turn"}}"#.into()],
        );
        assert_eq!(st.last_convo, t0 + 5 * 60_000);
    }

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

    /// Exercise the full `scan()` path over both source trees, widening the
    /// active window so historical Codex rollouts count as active. Prints any
    /// Codex-sourced sessions so we can eyeball the end-to-end wiring (project,
    /// context %, model, source tag). Non-asserting: a machine without Codex just
    /// prints zero.
    #[test]
    fn scan_surfaces_codex_sessions() {
        let mut cfg = Config::default();
        cfg.active_window_secs = 60 * 60 * 24 * 3650; // ~10y — reach old rollouts
        cfg.clear_waiting_mins = 60 * 24 * 3650; // don't clear the (finished) old ones
        let mut files = HashMap::new();
        let mut cc = ClaudeConfigCache::default();
        let snap = scan(&cfg, &mut files, &mut cc);
        let codex: Vec<_> = snap.sessions.iter().filter(|s| s.source == "codex").collect();
        println!("\ncodex sessions via scan(): {}", codex.len());
        for s in &codex {
            println!(
                "  {:<16} [{}] {:>5}% used={}/{} model={} kind={}",
                s.project, s.source, s.pct, s.used, s.limit, s.model, s.activity_kind
            );
            assert_eq!(s.source, "codex");
            assert!(s.limit > 0, "codex session must have a positive window");
            assert!(s.pct >= 0.0 && s.pct <= 100.0);
        }
    }

    #[test]
    fn natively_extended_models_get_the_big_window() {
        let cfg = Config::default();
        // Fable writes no [1m] marker and ~/.claude.json therefore hints
        // "standard" for its project — with usage still under 200k, the model
        // name alone must pick the 1M window.
        let (limit, ext, resolved) = resolve_limit("claude-fable-5", 170_000, false, Some(false), &cfg);
        assert_eq!(limit, cfg.extended_context_limit);
        assert!(ext);
        assert!(resolved);
        // A marker-less opt-in model with the same hint stays on the standard
        // window (its 1M variant would carry [1m]).
        let (limit, ext, resolved) = resolve_limit("claude-opus-5", 100_000, false, Some(false), &cfg);
        assert_eq!(limit, cfg.standard_context_limit);
        assert!(!ext);
        assert!(resolved);
        // The marker path is untouched.
        let (limit, ext, resolved) = resolve_limit("claude-opus-5[1m]", 100_000, false, Some(false), &cfg);
        assert_eq!(limit, cfg.extended_context_limit);
        assert!(ext);
        assert!(resolved);
        // No marker, no hint, sub-standard usage: nothing on disk answers the
        // question — the window must be flagged unresolved, never guessed as fact.
        let (limit, ext, resolved) = resolve_limit("claude-opus-5", 100_000, false, None, &cfg);
        assert_eq!(limit, cfg.default_context_limit);
        assert!(!ext);
        assert!(!resolved);
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
    fn permission_mode_maps_last_value() {
        let mut st = FileState::default();
        // Unseen ⇒ normal.
        assert_eq!(display_mode(&st.mode), "normal");
        // "auto" is the user's ⏵⏵ accept-edits mode → shown as "auto".
        ingest(&mut st, &[r#"{"type":"permission-mode","permissionMode":"auto"}"#.into()]);
        assert_eq!(display_mode(&st.mode), "auto");
        // "acceptEdits" (older name for the same mode) → also "auto".
        ingest(&mut st, &[r#"{"type":"permission-mode","permissionMode":"acceptEdits"}"#.into()]);
        assert_eq!(display_mode(&st.mode), "auto");
        // Plan carries through; last value wins.
        ingest(&mut st, &[r#"{"type":"permission-mode","permissionMode":"plan"}"#.into()]);
        assert_eq!(display_mode(&st.mode), "plan");
        // A typed prompt carries the mode too; default ⇒ normal.
        ingest(
            &mut st,
            &[r#"{"type":"user","promptSource":"typed","permissionMode":"default","message":{"content":"go"}}"#.into()],
        );
        assert_eq!(display_mode(&st.mode), "normal");
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

    #[test]
    fn manual_interrupt_returns_to_waiting() {
        // Mid tool-call, so the last real activity was "working".
        let mut st = FileState::default();
        ingest(
            &mut st,
            &[r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Bash","input":{"command":"sleep 100"}}]}}"#.into()],
        );
        assert_eq!(st.activity_kind, "working");
        let log_before = st.log.len();

        // Esc/Ctrl+C writes a user line with the interrupt marker (no
        // interruptedMessageId on this variant) — must flip to waiting, not read
        // as a fresh prompt, and must not add a log line (not a typed prompt).
        ingest(
            &mut st,
            &[r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user for tool use]"}]}}"#.into()],
        );
        assert_eq!(st.activity_kind, "waiting", "interrupt should hand back to the user");
        assert_eq!(st.activity, "interrupted");
        assert_eq!(st.log.len(), log_before, "interrupt must not add a log line");

        // The structured-field variant (top-level interruptedMessageId) also counts.
        st.activity_kind = "working".into();
        ingest(
            &mut st,
            &[r#"{"type":"user","interruptedMessageId":"msg_123","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#.into()],
        );
        assert_eq!(st.activity_kind, "waiting");
    }

    #[test]
    fn awaiting_first_gates_the_cancel_guard() {
        let mut st = FileState::default();
        // A typed prompt arms the guard (optimistic "thinking…", unconfirmed).
        ingest(
            &mut st,
            &[r#"{"type":"user","promptSource":"typed","message":{"content":"do the thing"}}"#.into()],
        );
        assert_eq!(st.activity_kind, "working");
        assert!(st.awaiting_first, "a fresh prompt should arm the cancel guard");

        // The first assistant line confirms real work → guard disarmed, so a
        // later silence must NOT be mistaken for a cancel.
        ingest(
            &mut st,
            &[r#"{"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Read","input":{"file_path":"/x/a.rs"}}]}}"#.into()],
        );
        assert!(!st.awaiting_first, "an assistant line disarms the guard");

        // A new prompt re-arms it; an interrupt (not a silence) disarms immediately.
        ingest(
            &mut st,
            &[r#"{"type":"user","promptSource":"typed","message":{"content":"again"}}"#.into()],
        );
        assert!(st.awaiting_first);
        ingest(
            &mut st,
            &[r#"{"type":"user","interruptedMessageId":"m","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#.into()],
        );
        assert!(!st.awaiting_first, "an interrupt disarms the guard too");
    }
}
