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
}

/// Account-wide token usage over rolling windows — a proxy for plan utilisation
/// computed from transcripts. A rate-limit-header source can later fill this
/// exact same shape with no UI change.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PlanUsage {
    pub five_hour_tokens: u64,
    pub week_tokens: u64,
    pub five_hour_budget: u64,
    pub week_budget: u64,
    pub five_hour_pct: f64,
    pub week_pct: f64,
    pub five_hour_level: String,
    pub week_level: String,
}

impl Default for PlanUsage {
    fn default() -> Self {
        // Zeroed placeholder — real values (incl. budgets) always come from
        // `build_plan` using the config.
        Self {
            five_hour_tokens: 0,
            week_tokens: 0,
            five_hour_budget: 0,
            week_budget: 0,
            five_hour_pct: 0.0,
            week_pct: 0.0,
            five_hour_level: "ok".into(),
            week_level: "ok".into(),
        }
    }
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
    pub plan: PlanUsage,
    pub generated_at: i64,
    /// False when `~/.claude/projects` doesn't exist (friendly empty state).
    pub has_projects_dir: bool,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            state: "idle".into(),
            agent_count: 0,
            waiting: 0,
            projects: 0,
            sessions: Vec::new(),
            plan: PlanUsage::default(),
            generated_at: 0,
            has_projects_dir: true,
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
    /// `(timestamp_ms, fresh_tokens)` per assistant message, for the rollup.
    events: Vec<(i64, u64)>,
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

/// Newly-processed tokens for the usage rollup: `input + output + cache
/// creation`. Cache *reads* are excluded — they re-read the same context every
/// turn and would balloon the total far past real throughput.
fn usage_fresh(usage: &serde_json::Value) -> u64 {
    let g = |k: &str| usage.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    g("input_tokens") + g("output_tokens") + g("cache_creation_input_tokens")
}

/// Parse a leading `YYYY-MM-DDTHH:MM:SS` (assumed UTC) to epoch ms. Fractional
/// seconds / timezone offset are ignored — fine for windowing.
fn parse_iso_to_ms(s: &str) -> Option<i64> {
    if s.len() < 19 {
        return None;
    }
    let p = |a: usize, b: usize| s.get(a..b).and_then(|x| x.parse::<i64>().ok());
    let (year, mon, day) = (p(0, 4)?, p(5, 7)?, p(8, 10)?);
    let (hour, min, sec) = (p(11, 13)?, p(14, 16)?, p(17, 19)?);
    // days_from_civil (Howard Hinnant)
    let y = if mon <= 2 { year - 1 } else { year };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if mon > 2 { mon - 3 } else { mon + 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some((days * 86400 + hour * 3600 + min * 60 + sec) * 1000)
}

fn build_plan(five: u64, week: u64, cfg: &Config) -> PlanUsage {
    let ratio = |used: u64, budget: u64| {
        if budget > 0 {
            (((used as f64 / budget as f64) * 100.0).clamp(0.0, 100.0) * 10.0).round() / 10.0
        } else {
            0.0
        }
    };
    let fp = ratio(five, cfg.five_hour_token_budget);
    let wp = ratio(week, cfg.week_token_budget);
    PlanUsage {
        five_hour_tokens: five,
        week_tokens: week,
        five_hour_budget: cfg.five_hour_token_budget,
        week_budget: cfg.week_token_budget,
        five_hour_pct: fp,
        week_pct: wp,
        five_hour_level: level_for(fp, cfg).into(),
        week_level: level_for(wp, cfg).into(),
    }
}

/// Pick the context denominator (Open Question #4). Bumps (stickily) to the
/// extended window when the model advertises it or observed usage exceeds the
/// standard window — a standard session compacts before it could get there, so
/// exceeding it reliably implies the extended window. All sizes come from config.
fn resolve_limit(model: &str, used: u64, sticky_ext: bool, cfg: &Config) -> (u64, bool) {
    let m = model.to_lowercase();
    let marker = cfg
        .extended_window_markers
        .iter()
        .any(|mk| !mk.is_empty() && m.contains(&mk.to_lowercase()));
    let is_ext = sticky_ext || marker || used > cfg.standard_context_limit;
    (
        if is_ext { cfg.extended_context_limit } else { cfg.default_context_limit },
        is_ext,
    )
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
        // Context occupancy comes from the *latest* assistant usage block; the
        // rollup accumulates fresh tokens per timestamped message.
        if v.get("type").and_then(|x| x.as_str()) == Some("assistant") {
            if let Some(usage) = v.get("message").and_then(|m| m.get("usage")) {
                state.used = usage_total(usage);
                if let Some(ts) = v.get("timestamp").and_then(|x| x.as_str()).and_then(parse_iso_to_ms) {
                    state.events.push((ts, usage_fresh(usage)));
                }
            }
            if let Some(model) = v.get("message").and_then(|m| m.get("model")).and_then(|x| x.as_str()) {
                state.model = model.to_string();
            }
        }
    }
}

/// One full pass: refresh tail state for every active transcript and build the
/// aggregate snapshot. `files` is retained across calls for incremental reads.
pub fn scan(cfg: &Config, files: &mut HashMap<PathBuf, FileState>) -> Snapshot {
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
    let five_ms = cfg.five_hour_secs as i64 * 1000;
    let week_ms = cfg.week_secs as i64 * 1000;
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
            // Files older than the week window feed neither the session list nor
            // the rollup.
            if now - mtime > week_ms {
                continue;
            }
            seen.push(path.clone());
            let active = now - mtime <= active_ms; // Step 1.2

            let size = meta.len();
            let st = files.entry(path.clone()).or_default();
            // Truncation / rotation → reset the tail.
            if size < st.offset {
                *st = FileState::default();
            }
            // First sight of a large file: skip to a bounded tail.
            if st.offset == 0 && size > cfg.initial_tail_cap_bytes {
                st.offset = size - cfg.initial_tail_cap_bytes;
            }
            if size > st.offset {
                let (new_offset, lines) = read_new_lines(&path, st.offset);
                st.offset = new_offset;
                ingest(st, &lines);
            }
            // Bound rollup memory to a week per file.
            st.events.retain(|(ts, _)| now - *ts <= week_ms);

            if !active {
                continue; // recent enough for the rollup, but not a live session
            }

            let (limit, is_ext) = resolve_limit(&st.model, st.used, st.is_1m, cfg);
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
            let slug = pdir.file_name().and_then(|s| s.to_str()).unwrap_or("");
            let (project, root) = resolve_project(slug, &st.cwd);

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
            });
        }
    }

    // Drop tail state for files outside the week window / removed, so the map
    // can't grow without bound.
    files.retain(|k, _| seen.contains(k));

    // Plan-usage rollup: sum fresh tokens across every recent file's events.
    let (mut five, mut week) = (0u64, 0u64);
    for st in files.values() {
        for (ts, tok) in &st.events {
            if now - *ts <= week_ms {
                week += *tok;
                if now - *ts <= five_ms {
                    five += *tok;
                }
            }
        }
    }
    let plan = build_plan(five, week, cfg);

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
        plan,
        generated_at: now,
        has_projects_dir: true,
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
        let snap = scan(&cfg, &mut files);
        println!(
            "\nsnapshot: state={} active={} projects={} waiting={} has_dir={}",
            snap.state, snap.agent_count, snap.projects, snap.waiting, snap.has_projects_dir
        );
        for s in &snap.sessions {
            println!(
                "  {:<22} [{:^4}] {:>5}%  {:>8}/{:<9} model={:<20} branch={:?}",
                s.project, s.level, s.pct, s.used, s.limit, s.model, s.branch
            );
        }
        println!(
            "plan: 5h={} tokens ({}%)  7d={} tokens ({}%)",
            snap.plan.five_hour_tokens, snap.plan.five_hour_pct, snap.plan.week_tokens, snap.plan.week_pct
        );
        // Every reported session must have a sane percentage.
        for s in &snap.sessions {
            assert!(s.pct >= 0.0 && s.pct <= 100.0, "pct out of range: {}", s.pct);
            assert!(s.limit > 0, "limit must be positive");
        }
    }
}
