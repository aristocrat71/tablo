//! Changelog section for the running build, shown once after an update.
//!
//! `CHANGELOG.md` is compiled in, so the notes match the binary for every
//! install path (updater, DMG, install.sh). Parsed to a struct rather than
//! handed over as markdown because `{@html}` is banned app-wide.

use serde::Serialize;

const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NoteGroup {
    /// "Added" / "Changed" / "Fixed", or empty for a section with no headings.
    pub heading: String,
    pub items: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseNotes {
    pub version: String,
    pub groups: Vec<NoteGroup>,
}

pub fn notes_for(version: &str) -> Option<ReleaseNotes> {
    parse(CHANGELOG, version)
}

fn clean(s: &str) -> String {
    s.replace("**", "").replace('`', "")
}

fn parse(doc: &str, version: &str) -> Option<ReleaseNotes> {
    // The closing bracket keeps this exact: "2.1" can't match "## [2.1.0]".
    let header = format!("## [{version}]");
    let mut lines = doc.lines().skip_while(|l| !l.starts_with(&header));
    // Skips the heading, so its date never reaches the UI. None = no match.
    lines.next()?;

    let mut groups: Vec<NoteGroup> = Vec::new();
    for raw in lines {
        let line = raw.trim_end();
        if line.starts_with("## [") {
            break;
        }
        if let Some(h) = line.strip_prefix("### ") {
            groups.push(NoteGroup {
                heading: h.trim().to_string(),
                items: Vec::new(),
            });
            continue;
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        // A section can open with prose before any heading ("First public release.").
        if groups.is_empty() {
            groups.push(NoteGroup {
                heading: String::new(),
                items: Vec::new(),
            });
        }
        let group = groups.last_mut().unwrap();
        match text.strip_prefix("- ") {
            Some(item) => group.items.push(clean(item)),
            // Continuation of the bullet above — the changelog hard-wraps.
            None => match group.items.last_mut() {
                Some(prev) => {
                    prev.push(' ');
                    prev.push_str(&clean(text));
                }
                None => group.items.push(clean(text)),
            },
        }
    }

    groups.retain(|g| !g.items.is_empty());
    if groups.is_empty() {
        return None;
    }
    Some(ReleaseNotes {
        version: version.to_string(),
        groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "\
# Changelog

Preamble that must not leak into any section.

## [2.1.0] - Unreleased

### Added

- **Notification sound.** The toast now plays a chime. **On by
  default** — turn it off under Settings.
- Cat animations toggle.

### Fixed

- A `backslash` path bug.

## [2.0.0] - 2026-07-27

### Added

- Anonymous usage stats.

## [0.5.0] - 2026-07-10

First public release.
";

    #[test]
    fn parses_groups_and_bullets() {
        let n = parse(DOC, "2.1.0").unwrap();
        assert_eq!(n.version, "2.1.0");
        assert_eq!(n.groups.len(), 2);
        assert_eq!(n.groups[0].heading, "Added");
        assert_eq!(n.groups[1].heading, "Fixed");
        assert_eq!(n.groups[0].items.len(), 2);
    }

    #[test]
    fn joins_wrapped_bullets_and_strips_markdown() {
        let n = parse(DOC, "2.1.0").unwrap();
        assert_eq!(
            n.groups[0].items[0],
            "Notification sound. The toast now plays a chime. On by default — turn it off under Settings."
        );
        assert_eq!(n.groups[1].items[0], "A backslash path bug.");
    }

    #[test]
    fn stops_at_the_next_version() {
        let n = parse(DOC, "2.0.0").unwrap();
        assert_eq!(n.groups.len(), 1);
        assert_eq!(n.groups[0].items, vec!["Anonymous usage stats."]);
    }

    #[test]
    fn final_section_without_headings() {
        let n = parse(DOC, "0.5.0").unwrap();
        assert_eq!(n.groups[0].heading, "");
        assert_eq!(n.groups[0].items, vec!["First public release."]);
    }

    #[test]
    fn unknown_version_is_none() {
        assert!(parse(DOC, "9.9.9").is_none());
    }

    #[test]
    fn version_match_is_exact_not_a_prefix() {
        assert!(parse(DOC, "2.1").is_none());
        assert!(parse(DOC, "2").is_none());
    }

    /// Guards against bumping the version without writing the changelog entry.
    #[test]
    fn bundled_changelog_has_notes_for_the_running_version() {
        let version = env!("CARGO_PKG_VERSION");
        assert!(
            notes_for(version).is_some(),
            "CHANGELOG.md has no '## [{version}]' section"
        );
    }
}
