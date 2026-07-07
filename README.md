# tablo

A tiny floating cat that watches your Claude Code agents. Tablo lives in a
corner of your screen and reflects live activity through its animation: it
**breathes sage** when idle, **pulses amber** while agents are working, and
**flushes coral** when a session's context window crosses the danger line.
**Tap Tablo** to open a panel with a live context-window meter per session; from
there, open a fuller **dashboard**.

Built with **Tauri 2** (Rust backend) + **SvelteKit / Svelte 5** (webview).

## What it does (Phase 1)

- **Avatar** — a ~120×140 transparent, always-on-top window that is
  *click-through everywhere except the cat*. Drag to reposition (persisted);
  tap to toggle the panel.
- **Live context meter** — tails the JSONL transcripts under
  `~/.claude/projects/` (byte-offset incremental reads, no whole-file re-parse),
  computes each active session's context occupancy from the latest `assistant`
  usage block, and pushes updates to every window.
- **Avatar state machine** — `idle` (no active session) → `running` (≥1 active)
  → `alarmed` (any session past the warning line). Alarmed overrides running
  overrides idle. The active-session count shows as a badge outside the hex.
- **Panel** — groups sessions (Input-requested first, then Working), each with a
  segmented-LED context bar coloured by threshold (sage < 60% < amber < 85% <
  coral), project name, branch, and raw token counts. Dismisses on tap-away or a
  second tap.
- **Dashboard** — a larger window with headline stats and per-session gauges.
- **Themes** — warm dark (hero) and light, per `tablo-mockups-v3.html`. Toggle
  via the panel's ☾ button; persisted.

## Architecture

```
src-tauri/src/
  config.rs    persisted config (avatar pos, thresholds, theme) — JSON in app-config dir
  scanner.rs   transcript discovery, incremental tail, context %, aggregate Snapshot
  lib.rs       windows, event/scan loop, cursor hit-test (click-through), commands
src/
  routes/+page.svelte     selects a surface by window label
  lib/Avatar.svelte       hex + glyph + glow + count badge + tap/drag
  lib/Panel.svelte        grouped session meters
  lib/Dashboard.svelte    deep view
  lib/state.svelte.ts     shared reactive store (fetch snapshot + subscribe)
```

Rust pushes one `state-update` event carrying the full `Snapshot`; each window
takes what it needs. A newly opened window fetches the current snapshot via the
`get_snapshot` command, then subscribes.

## Context-window detection (Open Question #4)

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

Config lives at the Tauri app-config dir (macOS:
`~/Library/Application Support/com.projektdreamscape.tablo/config.json`).

## Plan usage — intentionally not built

Anthropic's plan-quota data (5h/weekly %, resets-in) is **not** stored in any
local file — it only rides the API's `anthropic-ratelimit-*` response headers,
which Claude Code holds in memory. The only way to read it is to replay your
Claude Code subscription token against the API, which is a Terms-of-Service grey
area (those tokens are authorized for use *by Claude Code*). We deliberately do
**not** do that, so Tablo has no plan/quota widget.

## Jump to session

Each session card has a **jump** button that focuses the terminal a session is
running in. Sessions self-report their location through a passive
`SessionStart` / `UserPromptSubmit` hook; jump then switches tmux to the pane (if
any) and brings the host terminal to the front.

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

## Develop

```bash
bun install
bun run tauri dev      # launch the app (avatar appears bottom-right)
bun run tauri build    # produce a bundle
cargo test scan_real_transcripts -- --nocapture   # (in src-tauri) inspect the live snapshot
```

## Roadmap (later phases)

- **2** multi-session list refinements
- **4** permission approve/deny via Claude Code hooks (the panel's "Input
  requested" group is already wired to render `ask` sessions)
- **5** browser-served localhost dashboard
