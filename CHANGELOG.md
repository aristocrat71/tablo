# Changelog

All notable changes to tablo are recorded here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
tablo uses [semantic versioning](https://semver.org/)

**This file is published.** The release workflow extracts the section matching
the tag being built and uses it as the GitHub release body, which `tauri-action`
also copies into `latest.json` — so it becomes the release notes every installed
copy of tablo sees when it checks for updates. A tag with no matching section
here fails the release before anything is built. Write the entry as you merge,
not at tag time.

## [2.1.0] - Unreleased

### Added

- **Notification sound.** The waiting toast now plays a soft chime. **On by
  default** — existing installs will start making a sound after updating. Turn
  it off under Settings → Notification sound.
- **Cat animations toggle.** Settings → Cat animations. Off holds the cat on a
  still pose with a steady glow; the state colour still tells you what's
  happening.
- **Automatic updates setting.** On by default. Turning it off doesn't stop
  tablo looking for releases — it stops it applying them: you get a
  notification and a one-tap Install button in Settings instead of a silent
  background upgrade.

### Changed

- The dashboard window is built when you first open it rather than at launch,
  which drops idle memory by roughly one webview process.

### Fixed

- The panel and the waiting toast no longer slide underneath the macOS Dock.
  Both are now clamped to the screen's work area instead of the full display,
  so their lower rows stay visible and clickable.
- Jump to session resolves from the session registry and re-resolves at click
  time, so it no longer targets a window that has since moved or closed.
- Windows releases now publish their `.sha256` checksum sidecars. v2.0.0
  shipped without them because the workflow's path matcher didn't handle
  backslash separators, which broke checksum verification in `install.ps1`.

## [2.0.0] - 2026-07-27

### Added

- Anonymous usage stats so active installs can be counted — no session data,
  paths, prompts, or tokens. Opt out under Settings.
- A one-time notice on first launch explaining the locator hook that tablo
  installs into Claude Code for "jump to session".
- unravel.tech branding across the app and site.

### Security

- The loopback IPC server used by the approvals hook is now authenticated and
  bounded.
- `install.sh` / `install.ps1` verify the installer's checksum and fail closed
  on a mismatch, and no longer bypass OS gatekeepers.
- Fonts are self-hosted and HTML output is guarded against injection.
- Release workflow actions are pinned to commit SHAs and the signing job runs
  with least privilege behind a protected environment.

### Changed

- The website moved from Netlify to Cloudflare Pages.

## [1.0.1] - 2026-07-11

### Added

- The Working and Waiting groups collapse and expand, and the state is shared
  between the panel and the dashboard.

### Fixed

- Settings that only apply on macOS are hidden on Windows and Linux instead of
  showing as dead toggles.
- Typography moved to the mono family for session data.

## [1.0.0] - 2026-07-10

### Added

- AeroSpace support (macOS): tablo follows the focused workspace.

### Fixed

- Improved de-focus behaviour when dismissing the panel.

## [0.5.2] - 2026-07-10

### Fixed

- Jump to session was flaky; marked experimental while it stabilised.

## [0.5.1] - 2026-07-10

### Added

- Credits in the dashboard.

### Fixed

- Jump to session under tmux, and when the editor is Zed.

## [0.5.0] - 2026-07-10

First public release.
