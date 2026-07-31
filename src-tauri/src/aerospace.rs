//! AeroSpace workspace-follow (macOS).
//!
//! AeroSpace is an i3-like tiling window manager that emulates its *own*
//! workspaces instead of using native macOS Spaces. Tablo's
//! `visibleOnAllWorkspaces` flag pins its windows across *native* Spaces, but
//! AeroSpace bypasses that entirely: on a workspace switch it reassigns Tablo's
//! windows to a workspace and moves them off-screen, so the cat vanishes the
//! moment you swap workspaces.
//!
//! AeroSpace has no native "sticky / on-all-workspaces" window, so rather than
//! stay put, Tablo *follows*: a light background poll reads the focused workspace
//! via the `aerospace` CLI and, whenever it changes, moves Tablo's widget windows
//! onto it (`aerospace move-node-to-workspace --window-id <id> <ws>`).
//!
//! Self-contained and cost-free off AeroSpace: when the `aerospace` binary is
//! absent or unresponsive the loop backs off to a slow probe and never touches a
//! window. The dashboard (a normal, tile-managed app window) is deliberately left
//! alone — only the avatar/panel/toast companion surfaces follow.

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::AppState;

/// The dashboard window's title (set by `open_dashboard`, which builds it).
/// Excluded from following so AeroSpace keeps tiling it like any normal window.
const DASHBOARD_TITLE: &str = "tablo dashboard";

/// Poll cadence while AeroSpace is present and responding — small enough that the
/// avatar reappears on the new workspace almost immediately after a switch.
const FOLLOW_POLL: Duration = Duration::from_millis(300);
/// Slow probe cadence when AeroSpace isn't detected (or following is off), so a
/// non-AeroSpace machine spends essentially nothing here.
const IDLE_PROBE: Duration = Duration::from_secs(5);

/// Whether AeroSpace has been seen responding this run. Read by the scanner to
/// decide whether to surface the "Follow AeroSpace" toggle in Settings — the
/// control only makes sense to a user actually running AeroSpace.
static AVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn available() -> bool {
    AVAILABLE.load(Ordering::Relaxed)
}

/// Run an `aerospace` subcommand, returning trimmed stdout on exit 0, or None if
/// the binary is missing / errored (⇒ AeroSpace not active right now).
fn aerospace(args: &[&str]) -> Option<String> {
    let out = Command::new("aerospace").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Name of the currently focused AeroSpace workspace, or None if AeroSpace isn't
/// running / reachable.
fn focused_workspace() -> Option<String> {
    aerospace(&["list-workspaces", "--focused"]).filter(|s| !s.is_empty())
}

/// `(window-id, workspace)` for every Tablo widget window AeroSpace can see,
/// excluding the dashboard (kept under normal tiling).
fn widget_windows(bundle_id: &str) -> Vec<(String, String)> {
    let Some(raw) = aerospace(&[
        "list-windows",
        "--all",
        "--app-bundle-id",
        bundle_id,
        "--format",
        // Tab-delimited so the (space-bearing) title parses cleanly last.
        "%{window-id}\t%{workspace}\t%{window-title}",
    ]) else {
        return Vec::new();
    };
    raw.lines()
        .filter_map(|line| {
            let mut it = line.splitn(3, '\t');
            let id = it.next()?.trim().to_string();
            let ws = it.next()?.trim().to_string();
            let title = it.next().unwrap_or("").trim();
            if id.is_empty() || title == DASHBOARD_TITLE {
                None
            } else {
                Some((id, ws))
            }
        })
        .collect()
}

/// Pull every Tablo widget window that isn't already on `focused` onto it.
fn follow_to(focused: &str, bundle_id: &str) {
    for (id, ws) in widget_windows(bundle_id) {
        if ws != focused {
            let _ = aerospace(&["move-node-to-workspace", "--window-id", &id, focused]);
        }
    }
}

/// Flip the availability flag; on a genuine change, re-emit so Settings shows /
/// hides the toggle live without waiting for the next scan.
fn set_available(app: &AppHandle, val: bool) {
    if AVAILABLE.swap(val, Ordering::Relaxed) != val {
        crate::recompute_and_emit(app);
    }
}

/// Spawn the follow loop. Gated to macOS at the call site; the loop itself
/// no-ops (slow probe) whenever AeroSpace is absent or the user turns following
/// off, so it's safe to run unconditionally there.
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let bundle_id = app.config().identifier.clone();
        let mut last_focused: Option<String> = None;
        loop {
            // Respect the live Settings toggle without a restart.
            let enabled = app
                .try_state::<AppState>()
                .map(|st| st.config.lock().unwrap().aerospace_follow)
                .unwrap_or(true);

            if !enabled {
                set_available(&app, false);
                last_focused = None;
                std::thread::sleep(IDLE_PROBE);
                continue;
            }

            match focused_workspace() {
                Some(focused) => {
                    set_available(&app, true);
                    // Only reconcile when the focused workspace actually changed —
                    // steady state is one cheap `list-workspaces` call per tick.
                    if last_focused.as_deref() != Some(focused.as_str()) {
                        follow_to(&focused, &bundle_id);
                        last_focused = Some(focused);
                    }
                    std::thread::sleep(FOLLOW_POLL);
                }
                None => {
                    set_available(&app, false);
                    last_focused = None;
                    std::thread::sleep(IDLE_PROBE);
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // Parsing is the only pure, testable part (the CLI calls need a live
    // AeroSpace). We can't call `widget_windows` without the binary, so exercise
    // the same split/exclude logic against representative rows.
    fn parse(raw: &str) -> Vec<(String, String)> {
        raw.lines()
            .filter_map(|line| {
                let mut it = line.splitn(3, '\t');
                let id = it.next()?.trim().to_string();
                let ws = it.next()?.trim().to_string();
                let title = it.next().unwrap_or("").trim();
                if id.is_empty() || title == DASHBOARD_TITLE {
                    None
                } else {
                    Some((id, ws))
                }
            })
            .collect()
    }

    #[test]
    fn excludes_dashboard_keeps_widgets() {
        let raw = "12\t1\ttablo\n\
                   34\t2\ttablo panel\n\
                   56\t3\ttablo dashboard\n\
                   78\t1\ttablo";
        let got = parse(raw);
        assert_eq!(
            got,
            vec![
                ("12".to_string(), "1".to_string()),
                ("34".to_string(), "2".to_string()),
                ("78".to_string(), "1".to_string()),
            ],
            "dashboard row excluded, avatar/panel/toast kept"
        );
    }

    #[test]
    fn tolerates_blank_and_malformed_rows() {
        let raw = "\n90\t4\ttablo\n\tgarbage";
        assert_eq!(parse(raw), vec![("90".to_string(), "4".to_string())]);
    }
}
