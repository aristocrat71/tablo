//! OpenCode support: read the SQLite session store and map it onto the same
//! per-session model the Claude Code and Codex scanners produce.
//!
//! Unlike the other two agents, OpenCode keeps no JSONL transcript — everything
//! lives in `~/.local/share/opencode/opencode.db` (WAL mode). So there is no
//! byte-offset tail here: each changed-DB scan re-queries, and the result is
//! cached against the db/wal mtime so an idle DB costs one stat().

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use crate::config::Config;
use crate::scanner::{
    abbreviate_home, level_for, mtime_ms, snippet, truncate_activity, ActivityEntry, SessionView,
    Subagent, ACTIVITY_LOG_CAP, ACTIVITY_MAX,
};

/// Sessions whose row was touched within this window are candidates. Generous
/// because `session.time_updated` only moves at turn start/end (never during),
/// so real liveness is recomputed from `part` below.
const CANDIDATE_WINDOW_MS: i64 = 6 * 60 * 60 * 1000;
/// Cap on candidate sessions inspected per changed-DB scan.
const CANDIDATE_LIMIT: usize = 40;
/// Recent `part` rows pulled per session for the terminal log.
const PART_WINDOW: usize = 24;
/// Recent `message` rows pulled per session to find the latest usage.
const MSG_WINDOW: usize = 8;

fn xdg_dir(var: &str, fallback: &[&str]) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| fallback.iter().fold(h, |a, s| a.join(s))))
}

/// OpenCode uses XDG paths on every platform, not the OS-native app dirs.
pub fn db_path() -> Option<PathBuf> {
    xdg_dir("XDG_DATA_HOME", &[".local", "share"]).map(|d| d.join("opencode").join("opencode.db"))
}

fn models_path() -> Option<PathBuf> {
    xdg_dir("XDG_CACHE_HOME", &[".cache"]).map(|d| d.join("opencode").join("models.json"))
}

/// The models.dev catalog OpenCode caches on disk: "providerID/modelID" → context
/// window. It's ~3.6 MB, so it's parsed only when its mtime moves.
#[derive(Default)]
pub struct Catalog {
    loaded: bool,
    mtime: i64,
    limits: HashMap<String, u64>,
}

impl Catalog {
    fn get(&self, provider: &str, model: &str) -> Option<u64> {
        self.limits.get(&format!("{provider}/{model}")).copied()
    }
}

fn refresh_catalog(cat: &mut Catalog) {
    let Some(path) = models_path() else { return };
    let mtime = std::fs::metadata(&path).ok().map(|m| mtime_ms(&m)).unwrap_or(0);
    if cat.loaded && mtime == cat.mtime {
        return;
    }
    let Ok(text) = std::fs::read_to_string(&path) else { return };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return };
    let mut limits = HashMap::new();
    if let Some(providers) = v.as_object() {
        for (pid, entry) in providers {
            let Some(models) = entry.get("models").and_then(|m| m.as_object()) else { continue };
            for (mid, m) in models {
                let ctx = m.get("limit").and_then(|l| l.get("context")).and_then(|c| c.as_u64());
                if let Some(c) = ctx.filter(|c| *c > 0) {
                    limits.insert(format!("{pid}/{mid}"), c);
                }
            }
        }
    }
    cat.limits = limits;
    cat.mtime = mtime;
    cat.loaded = true;
}

/// Cross-scan state: the model catalog plus the last query result, replayed
/// while the DB is untouched.
#[derive(Default)]
pub struct State {
    catalog: Catalog,
    stamp: i64,
    cache: Vec<SessionView>,
}

/// Newest mtime across the db and its WAL sidecar — the WAL is what actually
/// moves while OpenCode is running.
fn db_stamp(db: &Path) -> i64 {
    let wal = db.with_extension("db-wal");
    [db.to_path_buf(), wal]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| mtime_ms(&m))
        .max()
        .unwrap_or(0)
}

pub fn available() -> bool {
    db_path().is_some_and(|p| p.exists())
}

/// One scan's worth of OpenCode sessions. Re-queries only when the DB changed;
/// the time-based guards are re-applied every call so a cached replay still ages.
pub fn collect(cfg: &Config, now: i64, active_ms: i64, st: &mut State) -> Vec<SessionView> {
    let Some(db) = db_path().filter(|p| p.exists()) else { return Vec::new() };
    let stamp = db_stamp(&db);
    if stamp != st.stamp {
        st.stamp = stamp;
        refresh_catalog(&mut st.catalog);
        st.cache = query(&db, cfg, now, &st.catalog).unwrap_or_default();
    }

    let cancel_ms = cfg.cancel_grace_mins.max(1) as i64 * 60_000;
    let clear_ms = cfg.clear_waiting_mins.max(1) as i64 * 60_000;
    let mut out = Vec::new();
    for s in &st.cache {
        if now - s.last_active > active_ms {
            continue;
        }
        let mut s = s.clone();
        // A killed OpenCode never stamps `time.completed`, so a dead turn would
        // otherwise read "working" forever.
        if s.activity_kind != "waiting" && now - s.last_active > cancel_ms {
            s.activity_kind = "waiting".into();
            s.activity = "cancelled".into();
        }
        if s.activity_kind == "waiting" && now - s.last_active > clear_ms {
            continue;
        }
        out.push(s);
    }
    out
}

fn open_ro(db: &Path) -> rusqlite::Result<Connection> {
    let uri = format!("file:{}?mode=ro", db.to_string_lossy());
    Connection::open_with_flags(uri, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI)
}

struct Row {
    id: String,
    parent_id: Option<String>,
    directory: String,
    title: String,
    model: String,
    agent: String,
    compacting: bool,
    worktree: String,
    created: i64,
    updated: i64,
}

fn query(db: &Path, cfg: &Config, now: i64, cat: &Catalog) -> rusqlite::Result<Vec<SessionView>> {
    let conn = open_ro(db)?;
    let floor = now - CANDIDATE_WINDOW_MS;
    let mut stmt = conn.prepare(
        "SELECT s.id, s.parent_id, s.directory, s.title, s.model, s.agent, s.time_compacting,
                COALESCE(p.worktree, ''), s.time_created, s.time_updated
         FROM session s LEFT JOIN project p ON p.id = s.project_id
         WHERE s.time_updated >= ?1 AND s.time_archived IS NULL
         ORDER BY s.time_updated DESC LIMIT ?2",
    )?;
    let rows: Vec<Row> = stmt
        .query_map(rusqlite::params![floor, CANDIDATE_LIMIT as i64], |r| {
            Ok(Row {
                id: r.get(0)?,
                parent_id: r.get::<_, Option<String>>(1)?.filter(|s| !s.is_empty()),
                directory: r.get(2)?,
                title: r.get(3)?,
                model: r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                agent: r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                compacting: r.get::<_, Option<i64>>(6)?.is_some(),
                worktree: r.get(7)?,
                created: r.get(8)?,
                updated: r.get(9)?,
            })
        })?
        .filter_map(Result::ok)
        .collect();

    let mut parsed: HashMap<String, (Row, Tail)> = HashMap::new();
    for row in rows {
        let tail = read_tail(&conn, &row)?;
        parsed.insert(row.id.clone(), (row, tail));
    }

    // Children are subagents of their parent, never rows of their own.
    let parents: HashSet<String> = parsed.keys().cloned().collect();
    let mut kids: HashMap<String, Vec<Subagent>> = HashMap::new();
    for (row, tail) in parsed.values() {
        let Some(pid) = row.parent_id.as_ref().filter(|p| parents.contains(*p)) else { continue };
        if tail.kind == "waiting" {
            continue;
        }
        kids.entry(pid.clone()).or_default().push(Subagent {
            id: row.id.clone(),
            name: truncate_activity(clean_title(&row.title).unwrap_or(&row.agent), ACTIVITY_MAX),
            agent_type: row.agent.clone(),
            started_at: row.created,
            is_async: false,
        });
    }
    for list in kids.values_mut() {
        list.sort_by_key(|s| s.started_at);
    }

    let mut out = Vec::new();
    for (row, tail) in parsed.values() {
        if row.parent_id.is_some() {
            continue;
        }
        let subagents = kids.remove(&row.id).unwrap_or_default();
        out.push(build_view(row, tail, subagents, cfg, cat));
    }
    Ok(out)
}

#[derive(Default)]
struct Tail {
    used: u64,
    provider: String,
    model: String,
    kind: String,
    activity: String,
    log: Vec<ActivityEntry>,
    last_active: i64,
}

fn read_tail(conn: &Connection, row: &Row) -> rusqlite::Result<Tail> {
    let mut t = Tail { last_active: row.updated, ..Default::default() };

    let mut stmt = conn.prepare_cached(
        "SELECT data, time_updated FROM message WHERE session_id = ?1
         ORDER BY time_created DESC, id DESC LIMIT ?2",
    )?;
    let msgs: Vec<(Value, i64)> = stmt
        .query_map(rusqlite::params![row.id, MSG_WINDOW as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .filter_map(Result::ok)
        .filter_map(|(s, ts)| serde_json::from_str(&s).ok().map(|v| (v, ts)))
        .collect();

    if let Some((_, ts)) = msgs.first() {
        t.last_active = t.last_active.max(*ts);
    }
    let assistants: Vec<&Value> =
        msgs.iter().map(|(v, _)| v).filter(|v| v.get("role").and_then(|r| r.as_str()) == Some("assistant")).collect();

    // Tokens are written only at turn completion, so an in-flight message reports
    // zeros — the newest *completed* one is what the meter must read.
    if let Some(m) = assistants.iter().find(|m| tokens_of(m) > 0) {
        t.used = tokens_of(m);
        t.provider = str_of(m, "providerID").unwrap_or_default().to_string();
        t.model = str_of(m, "modelID").unwrap_or_default().to_string();
    }
    if let Some(m) = assistants.first() {
        if t.model.is_empty() {
            t.provider = str_of(m, "providerID").unwrap_or_default().to_string();
            t.model = str_of(m, "modelID").unwrap_or_default().to_string();
        }
        let done = m.get("time").and_then(|x| x.get("completed")).is_some();
        t.kind = if done { "waiting" } else { "working" }.into();
    }
    // A prompt with no assistant reply yet is still a turn in flight.
    if assistants.is_empty() && !msgs.is_empty() {
        t.kind = "working".into();
    }

    let mut stmt = conn.prepare_cached(
        "SELECT rowid, data, time_updated FROM part WHERE session_id = ?1
         ORDER BY rowid DESC LIMIT ?2",
    )?;
    let mut parts: Vec<(i64, Value, i64)> = stmt
        .query_map(rusqlite::params![row.id, PART_WINDOW as i64], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))
        })?
        .filter_map(Result::ok)
        .filter_map(|(id, s, ts)| serde_json::from_str(&s).ok().map(|v| (id, v, ts)))
        .collect();
    if let Some((_, _, ts)) = parts.first() {
        t.last_active = t.last_active.max(*ts);
    }
    parts.reverse();

    for (rowid, p, _) in &parts {
        if let Some((kind, text)) = log_line(p) {
            let dup = t.log.last().is_some_and(|e| e.kind == kind && e.text == text);
            if !dup {
                t.log.push(ActivityEntry { seq: *rowid as u64, kind: kind.into(), text });
            }
        }
    }
    if t.log.len() > ACTIVITY_LOG_CAP {
        t.log.drain(0..t.log.len() - ACTIVITY_LOG_CAP);
    }

    t.activity = parts
        .iter()
        .rev()
        .find_map(|(_, p, _)| activity_of(p))
        .unwrap_or_else(|| if t.kind == "working" { "thinking…".into() } else { String::new() });
    if row.compacting {
        t.activity = "compacting…".into();
        t.kind = "working".into();
    }
    Ok(t)
}

/// A user's own prompt rides a `text` part with no `time`; assistant output has one.
fn is_user_text(p: &Value) -> bool {
    p.get("time").is_none()
}

fn log_line(p: &Value) -> Option<(&'static str, String)> {
    match p.get("type").and_then(|x| x.as_str())? {
        "tool" => Some(("tool", tool_summary(p))),
        "reasoning" => Some(("think", "thinking…".into())),
        "text" => {
            let sn = snippet(p.get("text").and_then(|x| x.as_str())?);
            (!sn.is_empty()).then(|| (if is_user_text(p) { "user" } else { "text" }, sn))
        }
        _ => None,
    }
}

fn activity_of(p: &Value) -> Option<String> {
    match p.get("type").and_then(|x| x.as_str())? {
        "tool" => Some(truncate_activity(&tool_summary(p), ACTIVITY_MAX)),
        "reasoning" => Some("thinking…".into()),
        "text" if !is_user_text(p) => {
            let sn = snippet(p.get("text").and_then(|x| x.as_str())?);
            (!sn.is_empty()).then(|| truncate_activity(&sn, ACTIVITY_MAX))
        }
        _ => None,
    }
}

/// OpenCode already stores a human label per tool call (`state.title`) — a
/// command line, a file path — so there's nothing to reconstruct from arguments.
fn tool_summary(p: &Value) -> String {
    let tool = p.get("tool").and_then(|x| x.as_str()).unwrap_or("tool");
    let state = p.get("state");
    let title = state
        .and_then(|s| s.get("title"))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let verb = match tool {
        "bash" => "running",
        "read" => "reading",
        "edit" | "write" | "patch" => "editing",
        "grep" | "glob" | "list" => "searching",
        "webfetch" => "fetching",
        "task" => "delegating",
        "todowrite" | "todoread" => "planning",
        _ => "",
    };
    match (verb, title) {
        ("", Some(t)) => format!("{}: {t}", tool.replace('_', " ")),
        ("", None) => tool.replace('_', " "),
        (v, Some(t)) => format!("{v} {}", short_target(tool, t)),
        (v, None) => format!("{v}…"),
    }
}

/// File-shaped tool titles arrive as full paths; the basename is what reads.
fn short_target(tool: &str, title: &str) -> String {
    if matches!(tool, "read" | "edit" | "write" | "patch") && title.contains('/') {
        crate::scanner::base_name(title).to_string()
    } else {
        title.to_string()
    }
}

fn tokens_of(m: &Value) -> u64 {
    let Some(t) = m.get("tokens") else { return 0 };
    if let Some(total) = t.get("total").and_then(|x| x.as_u64()).filter(|n| *n > 0) {
        return total;
    }
    let n = |k: &str| t.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    let cache = |k: &str| t.get("cache").and_then(|c| c.get(k)).and_then(|x| x.as_u64()).unwrap_or(0);
    n("input") + n("output") + n("reasoning") + cache("read") + cache("write")
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

/// OpenCode titles an untitled session "New session - <ISO>"; that's a placeholder,
/// not a title, and the row already shows the project.
fn clean_title(title: &str) -> Option<&str> {
    let t = title.trim();
    (!t.is_empty() && !t.starts_with("New session - ")).then_some(t)
}

/// `session.model` is a JSON blob in current builds and a plain string in older
/// ones; both appear in the wild since the DB survives upgrades.
fn session_model(raw: &str) -> (String, String) {
    match serde_json::from_str::<Value>(raw) {
        Ok(v) if v.is_object() => (
            str_of(&v, "providerID").unwrap_or_default().to_string(),
            str_of(&v, "id").unwrap_or_default().to_string(),
        ),
        _ => (String::new(), raw.trim().to_string()),
    }
}

/// "plan" is OpenCode's read-only agent; everything else asks per tool.
fn display_mode(agent: &str) -> &'static str {
    match agent {
        "plan" => "plan",
        _ => "normal",
    }
}

/// The project root, preferring the VCS worktree. Sessions outside a repo land in
/// OpenCode's "global" project whose worktree is "/", which names nothing.
fn resolve_project(worktree: &str, directory: &str) -> (String, String) {
    let root = match worktree.trim() {
        "" | "/" => directory.trim(),
        w => w,
    };
    if root.is_empty() {
        return ("opencode".into(), String::new());
    }
    let name = Path::new(root)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("opencode");
    (name.to_string(), root.to_string())
}

fn pct_of(used: u64, limit: u64) -> f64 {
    if limit == 0 {
        return 0.0;
    }
    let pct = ((used as f64 / limit as f64) * 100.0).clamp(0.0, 100.0);
    (pct * 10.0).round() / 10.0
}

fn build_view(
    row: &Row,
    t: &Tail,
    subagents: Vec<Subagent>,
    cfg: &Config,
    cat: &Catalog,
) -> SessionView {
    let (s_provider, s_model) = session_model(&row.model);
    let provider = if t.provider.is_empty() { s_provider } else { t.provider.clone() };
    let model = if t.model.is_empty() { s_model } else { t.model.clone() };

    let resolved = cat.get(&provider, &model);
    let ctx_resolved = resolved.is_some() && t.used > 0;
    let limit = resolved.unwrap_or(cfg.opencode_context_limit);
    let pct = if ctx_resolved { pct_of(t.used, limit) } else { 0.0 };
    let (project, root) = resolve_project(&row.worktree, &row.directory);

    let mut kind = t.kind.clone();
    let mut activity = t.activity.clone();
    if !subagents.is_empty() {
        if kind == "waiting" || activity.is_empty() {
            let n = subagents.len();
            activity = format!("{n} agent{} running", if n == 1 { "" } else { "s" });
        }
        kind = "working".into();
    }

    SessionView {
        id: row.id.clone(),
        project,
        path: abbreviate_home(&root),
        branch: None,
        pct,
        used: t.used,
        limit,
        model,
        state: "run".into(),
        mode: display_mode(&row.agent).into(),
        level: if ctx_resolved { level_for(pct, cfg).into() } else { "ok".into() },
        ctx_resolved,
        last_active: t.last_active,
        title: clean_title(&row.title).map(str::to_string),
        activity,
        activity_kind: kind,
        activity_log: t.log.clone(),
        subagents,
        can_jump: false,
        source: "opencode".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_prefer_total_then_sum() {
        let m: Value = serde_json::from_str(
            r#"{"tokens":{"total":9818,"input":42,"output":32,"reasoning":16,"cache":{"read":9728,"write":0}}}"#,
        )
        .unwrap();
        assert_eq!(tokens_of(&m), 9818);

        // In-flight message: all zeros, no total — must read as "no usage yet".
        let inflight: Value =
            serde_json::from_str(r#"{"tokens":{"input":0,"output":0,"reasoning":0,"cache":{"read":0,"write":0}}}"#)
                .unwrap();
        assert_eq!(tokens_of(&inflight), 0);

        // Older rows without `total` fall back to the component sum.
        let legacy: Value =
            serde_json::from_str(r#"{"tokens":{"input":40,"output":30,"cache":{"read":100,"write":5}}}"#).unwrap();
        assert_eq!(tokens_of(&legacy), 175);
    }

    #[test]
    fn session_model_handles_both_shapes() {
        let (p, m) = session_model(r#"{"id":"longcat-2.0-free","providerID":"opencode","variant":"default"}"#);
        assert_eq!((p.as_str(), m.as_str()), ("opencode", "longcat-2.0-free"));
        let (p, m) = session_model("anthropic/claude-sonnet-4-6");
        assert_eq!((p.as_str(), m.as_str()), ("", "anthropic/claude-sonnet-4-6"));
    }

    #[test]
    fn tool_summary_uses_stored_title() {
        let bash: Value =
            serde_json::from_str(r#"{"type":"tool","tool":"bash","state":{"title":"ls -la"}}"#).unwrap();
        assert_eq!(tool_summary(&bash), "running ls -la");

        let read: Value =
            serde_json::from_str(r#"{"type":"tool","tool":"read","state":{"title":"/a/b/sample.txt"}}"#).unwrap();
        assert_eq!(tool_summary(&read), "reading sample.txt");

        let unknown: Value =
            serde_json::from_str(r#"{"type":"tool","tool":"some_tool","state":{"title":"x"}}"#).unwrap();
        assert_eq!(tool_summary(&unknown), "some tool: x");
    }

    #[test]
    fn user_and_assistant_text_parts_differ() {
        let user: Value = serde_json::from_str(r#"{"type":"text","text":"hey there"}"#).unwrap();
        assert_eq!(log_line(&user), Some(("user", "hey there".into())));
        assert_eq!(activity_of(&user), None);

        let asst: Value =
            serde_json::from_str(r#"{"type":"text","text":"Done.","time":{"start":1,"end":2}}"#).unwrap();
        assert_eq!(log_line(&asst), Some(("text", "Done.".into())));
        assert_eq!(activity_of(&asst).as_deref(), Some("Done."));
    }

    #[test]
    fn placeholder_titles_are_dropped() {
        assert_eq!(clean_title("New session - 2026-08-10T09:56:32.810Z"), None);
        assert_eq!(clean_title("  "), None);
        assert_eq!(clean_title("Greeting"), Some("Greeting"));
    }

    #[test]
    fn project_falls_back_when_outside_a_repo() {
        assert_eq!(
            resolve_project("/Users/m/Projects/tablo", "/Users/m/Projects/tablo/src"),
            ("tablo".into(), "/Users/m/Projects/tablo".into())
        );
        // The "global" project's worktree is "/" — use the session's own cwd.
        assert_eq!(resolve_project("/", "/tmp/scratch"), ("scratch".into(), "/tmp/scratch".into()));
    }

    #[test]
    fn agent_maps_to_mode_badge() {
        assert_eq!(display_mode("plan"), "plan");
        assert_eq!(display_mode("build"), "normal");
    }

    /// Smoke test against the real OpenCode DB, mirroring the Codex scanner's.
    #[test]
    fn read_real_db() {
        let Some(db) = db_path().filter(|p| p.exists()) else {
            eprintln!("no opencode db on disk — skipping");
            return;
        };
        let cfg = Config::default();
        let mut cat = Catalog::default();
        refresh_catalog(&mut cat);
        println!("\ncatalog models: {}", cat.limits.len());
        let views = query(&db, &cfg, crate::scanner::now_ms(), &cat).expect("query the opencode db");
        println!("opencode sessions (6h candidates): {}", views.len());
        for v in views.iter().take(5) {
            println!(
                "  {} model={} used={}/{} resolved={} kind={} activity={:?} log={} subs={}",
                v.project,
                v.model,
                v.used,
                v.limit,
                v.ctx_resolved,
                v.activity_kind,
                v.activity,
                v.activity_log.len(),
                v.subagents.len()
            );
        }
    }
}
