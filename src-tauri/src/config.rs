//! Persistent user configuration — the single source of truth for every tunable
//! value (thresholds, context windows, rollup windows, plan budgets, timings).
//! No domain value is hardcoded elsewhere; the scanner and UI read from here.
//!
//! Stored as JSON in the Tauri app-config dir. All fields have defaults so an
//! older/partial config file still loads cleanly (Open Question #10).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    // ---- avatar ----
    /// Saved avatar window position (logical pixels). `None` until first placed.
    pub avatar_x: Option<i32>,
    pub avatar_y: Option<i32>,

    // ---- session activity ----
    /// A transcript modified within this many seconds counts as an active
    /// session (Open Question #3).
    pub active_window_secs: u64,

    // ---- context window sizing (Open Question #4) ----
    /// Denominator used when a session's window can't be determined and its
    /// usage is below `standard_context_limit`. Set to `extended_context_limit`
    /// if you primarily run the 1M beta.
    pub default_context_limit: u64,
    /// The standard window. Also the threshold: usage above this implies the
    /// extended window (a standard session compacts before it could get there).
    pub standard_context_limit: u64,
    /// The extended (beta) window.
    pub extended_context_limit: u64,
    /// Model-string markers (lowercased, substring match) that force the
    /// extended window when present.
    pub extended_window_markers: Vec<String>,

    // ---- alarm thresholds (percent) ----
    /// Context-fill warning line. 60 is the required alarm line.
    pub warn_pct: f64,
    /// Context-fill critical line.
    pub crit_pct: f64,

    // ---- plan-usage rollup ----
    /// Rolling windows (seconds) the token rollup sums over.
    pub five_hour_secs: u64,
    pub week_secs: u64,
    /// Token budgets the rollup percentages are scaled against. The counts are
    /// real (summed from transcripts); the *percentage* is usage vs. these, so
    /// tune them to your plan. (Unused once rate-limit headers become the source.)
    pub five_hour_token_budget: u64,
    pub week_token_budget: u64,

    // ---- tuning ----
    /// On first sight of a transcript larger than this, skip to a bounded tail
    /// (we only need recent messages + the file's end).
    pub initial_tail_cap_bytes: u64,

    // ---- misc ----
    /// Fire a one-time OS notification when a session first crosses `warn_pct`
    /// (Open Question #5, default off for Phase 1).
    pub notify_on_warn: bool,
    /// "dark" (hero) or "light".
    pub theme: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            avatar_x: None,
            avatar_y: None,
            active_window_secs: 900,
            default_context_limit: 200_000,
            standard_context_limit: 200_000,
            extended_context_limit: 1_000_000,
            extended_window_markers: vec!["[1m]".into(), "-1m".into()],
            warn_pct: 60.0,
            crit_pct: 85.0,
            five_hour_secs: 5 * 3600,
            week_secs: 7 * 24 * 3600,
            five_hour_token_budget: 20_000_000,
            week_token_budget: 200_000_000,
            initial_tail_cap_bytes: 2_000_000,
            notify_on_warn: false,
            theme: "dark".into(),
        }
    }
}

impl Config {
    pub fn path(config_dir: &Path) -> PathBuf {
        config_dir.join("config.json")
    }

    pub fn load(config_dir: &Path) -> Self {
        let p = Self::path(config_dir);
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, config_dir: &Path) {
        let _ = std::fs::create_dir_all(config_dir);
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(config_dir), s);
        }
    }
}
