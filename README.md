# tablo

A tiny floating cat that watches your Claude Code agents. Tablo lives in a
corner of your screen and reflects live activity through its glow: it
**breathes sage** when idle, **pulses amber** while agents are working, and
**flushes coral** when a session needs your input or its context window crosses
the danger line. **Tap Tablo** to open a panel with a live context meter per
session; from there, open a fuller **dashboard** with settings.

Built with **Tauri 2** (Rust backend) + **SvelteKit / Svelte 5** (webview).

> The cat art is still in design — until it lands, Tablo renders as a state
> glyph (`I` idle · `A` working · `!` alarmed) inside a hexagon. The render layer
> is a single state→glyph table, so the sprite drops in behind it without
> touching any of the state or event wiring.

## Surfaces (four windows)

- **Avatar** — a 92×108 transparent, always-on-top window that is
  *click-through everywhere except the cat*. Drag to reposition (persisted);
  tap to toggle the panel. Its glow and glyph are a pure function of aggregate
  live state.
- **Panel** — opens on tap, anchored by the avatar. Grouped session meters.
  Dismisses on a second tap, on tap-away/blur, or with **Esc**.
- **Dashboard** — a larger, resizable window with per-session gauges, a live
  terminal-style activity preview, permission approvals, and a settings pane.
- **Toast** — a small, quiet notification that flies out of the avatar when a
  working session finishes and starts waiting on you (toggleable).

## What it does

### Avatar state machine + count pips

The avatar's glow is aggregate live state, precedence **alarmed → running → idle**:

| State | Glyph | Glow | Trigger |
|-------|-------|------|---------|
| **idle** | `I` | sage, slow breathe | no active sessions, or every one is just waiting on you |
| **running** | `A` | amber, quicker pulse | any session is actively working |
| **alarmed** | `!` | coral, urgent flush | any session past the context warning line, **or** any pending permission request (harder shake) |

Up to three **count pips** stack off the top-right of the hex, each shown only
when its count is non-zero:

- **coral** — sessions with a pending permission request,
- **sage** — sessions waiting on you,
- **amber** — sessions actively working.

A one-shot side-to-side shake fires when a waiting notification launches.

### Live context meter

Tails the JSONL transcripts under `~/.claude/projects/` (byte-offset
incremental reads, no whole-file re-parse), computes each active session's
context occupancy from the latest `assistant` usage block, and pushes one
`state-update` snapshot to every window. A session counts as *active* if its
transcript was modified within `activeWindowSecs` (default 15 min). Compaction
snap-backs animate down rather than flicker.

### Panel — grouped session meters

Sessions group by state, most urgent first:

1. **Critical** (pinned top) — any session past the warning line, under a red
   `Context window warning ! > N%` header (`N` = your configured limit).
2. **Permission Request** — tool calls awaiting approve/deny.
3. **Waiting** — handed back to you.
4. **Working** — actively running.

Each row shows the project name, an optional AI-generated session title, a state
badge, context %, path/branch, and a segmented-LED context bar with the raw
token count, coloured by threshold (sage < warn < amber < crit < coral).
Filter chips toggle the Waiting / Working groups when more than one session is
active.

### Notifications (toast)

When a working session finishes and starts waiting on you, a gentle toast slides
out of the avatar showing `project · title` and a **jump** button. The hover
time is configurable (seconds), and the whole feature can be switched off. Toasts
overshadow rather than stack.

### Permissions — approve / deny

Tool-call approvals via a Claude Code `PreToolUse` hook + a loopback IPC server:

- The hook `curl`s each intercepted tool call to Tablo's local server and blocks.
- Tablo registers a pending decision, forces the avatar to **alarmed**, and
  surfaces it in the panel's **Permission Request** group and on the dashboard.
- You tap **Approve** / **Deny**; the decision returns to the hook and
  un/blocks the tool.
- Only mutating tools are intercepted by default
  (`Bash`/`Write`/`Edit`/`MultiEdit`/`NotebookEdit`); read-only tools never pay
  the round-trip.
- **Fail-closed:** if you never decide within `hookTimeoutSecs` (~10 min), it
  **denies**. If Tablo is *down*, the hook's `curl` fails fast and Claude Code
  proceeds normally (never hangs).

The hook script is written to Tablo's own dir on launch (harmless), but
`~/.claude/settings.json` is only edited to actually intercept tools when you
flip **approvals on** in the dashboard.

### Dashboard + Settings

A larger view with per-session context gauges, a live terminal-style activity
preview (`$ live preview`), and the same live approvals. A gear opens an
in-window **Settings** pane:

- **Tool approvals** — install/remove the `PreToolUse` hook.
- **Jump** — enable/disable the jump-to-session buttons (on by default when the
  locator hook is installed).
- **Context window limit** — the warning threshold, `1`–`100` (default `60`).
  Applies live across every window.
- **Waiting notifications** — toggle the toast, and set its hover time (≥ 1 s).
- **Theme** — dark / light.

### Themes

Warm **dark** (hero) and **light**, per `tablo-mockups-v3.html`. The toggle lives
in dashboard Settings and syncs across every window (Rust broadcasts a
`theme-update` event); the choice is persisted.

### Quiet by default

Tablo runs as a macOS **Accessory** app — hidden from the Dock and Cmd+Tab. It
becomes a **Regular** app (so it appears in Cmd+Tab) only while the dashboard is
open, then drops back.

## Jump to session

Each session card — and each toast — has a **jump** button that focuses the
terminal a session is running in. Sessions self-report their location through a
passive `SessionStart` / `UserPromptSubmit` hook; jump then switches tmux to the
pane (if any) and brings the host terminal to the front.

Pinpointing the *exact tab* needs a terminal that exposes a per-tab tty to
AppleScript (or a focus-by-id CLI). Support on macOS:

| Terminal | No tmux | Inside tmux |
|----------|---------|-------------|
| Terminal.app | Exact window/tab | Exact pane |
| iTerm2 | Exact tab \* | Exact pane |
| Ghostty, WezTerm, kitty, Alacritty, Warp, Hyper, Tabby | App only \*\* | Exact pane |

\* The iTerm2 path exists but is currently unverified.

\*\* Brings the app to the front. With a single tab that's the right one; with
multiple tabs and no tmux it lands on the current tab — these terminals expose no
scriptable per-tab tty. **Inside tmux, jump is always exact**: tmux selects the
pane and Tablo just foregrounds the app.

Cross-platform: the tmux pane-switch works everywhere (incl. WSL); the GUI
window-raise is macOS-only today — Linux (X11 via `wmctrl`/`xdotool`) and Windows
(`SetForegroundWindow`) are stubbed, and Wayland / Windows-Terminal tabs stay
honest no-ops.

## Context-window detection

The transcript records the model (e.g. `claude-opus-4-8`) but **not** whether the
1M-token beta window is active. That signal *is* on disk though —
`~/.claude.json` records each project's last-used model string **with** its
`[1m]` marker under `projects[<cwd>].lastModelUsage`. So Tablo resolves each
session's window in priority order:

1. **Certain**: usage already past the standard window, a `[1m]` marker, or a
   session previously seen extended (a standard session compacts before it could
   exceed 200k, so exceeding it *is* proof of the extended window).
2. **Per-project** from `~/.claude.json` (cached, re-read on change): a `[1m]`
   model ⇒ 1M, otherwise the standard window.
3. **Global lean**: for a project with no record, the majority window across all
   projects that do.
4. **Fallback**: `defaultContextLimit` from config.

All window sizes are config values. This is fully automatic — no manual toggle;
the per-session `used / limit` readout (e.g. `354k / 1M`) shows what was detected.

## Plan usage — intentionally not built

Anthropic's plan-quota data (5h/weekly %, resets-in) is **not** stored in any
local file — it only rides the API's `anthropic-ratelimit-*` response headers,
which Claude Code holds in memory. The only way to read it is to replay your
Claude Code subscription token against the API, which is a Terms-of-Service grey
area (those tokens are authorized for use *by Claude Code*). We deliberately do
**not** do that, so Tablo has no plan/quota widget — only the static subscription
tier, which does live on disk, is read.

## Architecture

```
src-tauri/src/
  config.rs      persisted config (avatar pos, thresholds, theme, ports) — JSON in app-config dir
  scanner.rs     transcript discovery, incremental tail, context %, activity preview, aggregate Snapshot
  permission.rs  loopback approval server, PreToolUse hook script, settings.json install/uninstall
  locate.rs      session-location hook (self-reported cwd/tmux) + jump target resolution
  lib.rs         windows, event/scan loop, cursor hit-test (click-through), toast placement, commands
src/
  routes/+page.svelte     selects a surface by window label (avatar | panel | dashboard | toast)
  lib/Avatar.svelte       hex + glyph + glow + count pips + tap/drag + notif shake
  lib/Panel.svelte        grouped session meters (Critical / Permission / Waiting / Working) + filters
  lib/Dashboard.svelte    deep view + live terminal preview + approvals + settings pane
  lib/Toast.svelte        waiting notification
  lib/state.svelte.ts     shared reactive store (fetch snapshot + subscribe to state/theme)
  lib/prefs.svelte.ts     UI prefs in localStorage, cross-window synced via the storage event
```

Rust pushes one `state-update` event carrying the full `Snapshot`; each window
takes what it needs. A newly opened window fetches the current snapshot via the
`get_snapshot` command, then subscribes. Config lives at the Tauri app-config dir
(macOS: `~/Library/Application Support/com.projektdreamscape.tablo/config.json`).

## Develop

```bash
bun install
bun run tauri dev      # launch the app (avatar appears bottom-right)
bun run tauri build    # produce a bundle
cargo test scan_real_transcripts -- --nocapture   # (in src-tauri) inspect the live snapshot
```

## Status by phase

| Phase | Deliverable | State |
|-------|-------------|-------|
| 0 | Avatar + panel two-window scaffold | ✅ built |
| 1 | Animated cat (state + counts) + live context panel | ✅ built |
| 2 | Multi-session list + grouping / filters | ✅ built |
| 3 | Plan / session usage | ✖ cancelled (no live quota on disk; static tier only) |
| 4 | Permission approve / deny | ✅ built |
| 5 | Browser-served localhost dashboard | ⏸ on hold |
