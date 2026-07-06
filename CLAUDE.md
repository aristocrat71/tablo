# Tablo — Implementation Plan

**Tablo** is a small floating animated cat that watches your Claude Code agents. It lives unobtrusively on screen and reflects activity through animation: when agents are running, Tablo runs and shows the agent count; when a session's context window enters the danger zone (>60%), Tablo becomes alarmed. **Tapping Tablo opens a panel** with the live context-window meter. This avatar-plus-panel model replaces an always-on-top window, which would take up too much screen space.

This document is the build plan for Claude Code to follow. Phase 1 ships Tablo (the avatar) and the context meter only; later phases add permissions, plan usage, and a dashboard.

---

## Project overview

**Goal (Phase 1):** A tiny always-on-top animated cat avatar that watches Claude Code session transcripts and reflects activity through its animation state. Tapping it opens a panel showing each active session's context-window fill as a live percentage bar, with a danger warning above 60%.

**Interaction model — two surfaces:**
- **Avatar** — a tiny (~64×64), transparent, always-on-top, click-through-except-on-the-cat window. Always visible. Animation is driven by live data (idle / running with agent count / alarmed).
- **Panel** — a second, larger window that toggles open near the avatar when the cat is tapped, and dismisses on tap-away or a second tap. Contains the context meter(s). Not always visible.

**Stack:**
- **Tauri 2** (Rust backend + webview frontend) — chosen for small footprint, native always-on-top, transparency, and multi-window support.
- **Frontend:** plain HTML/CSS/JS or a lightweight framework (see Open Questions). Keep it minimal for Phase 1.
- **File watching:** Rust-side, using the `notify` crate to tail JSONL transcripts.
- **State bridge:** Tauri events (`emit`/`listen`) to push updates from Rust to both windows.
- **Avatar art:** sprite-sheet or SVG/CSS animation (see Open Questions) — designed separately; the plan leaves an asset slot.

**Non-goals for Phase 1:** permission approval, plan/quota usage, dashboard, multi-machine/LAN. These are scoped in later phases so the architecture leaves room for them but does not build them yet.

---

## Data model reference

Claude Code writes one JSONL transcript per session:

```
~/.claude/projects/<project-path-slug>/<session-id>.jsonl
```

Each line is a JSON event. The relevant lines are `assistant` messages, which carry a `usage` object:

```json
{
  "type": "assistant",
  "message": {
    "model": "claude-...",
    "usage": {
      "input_tokens": 4521,
      "cache_creation_input_tokens": 18023,
      "cache_read_input_tokens": 112400,
      "output_tokens": 843
    }
  }
}
```

**Context occupancy** is computed from the **latest** assistant message (not a sum across turns):

```
context_used = input_tokens
             + cache_creation_input_tokens
             + cache_read_input_tokens
             + output_tokens

context_pct  = context_used / context_limit
```

`context_limit` is model-dependent (200,000 standard; 1,000,000 if the extended-window beta is active). Read `message.model` to pick the denominator.

**Compaction note:** Claude Code auto-compacts near ~95%, which drops effective context. Expect the value to climb then snap down — handle this so it does not look like a bug.

---

## Avatar state model

The cat's animation is a pure function of the aggregate live state. Define these states explicitly so animation and data stay decoupled:

| State | Trigger | Animation |
|-------|---------|-----------|
| **idle** | no active session | cat sits / sleeps |
| **running** | ≥1 active agent | cat runs; agent count rendered on/beside it |
| **alarmed** | any active session's context > 60% | cat agitated (still shows count) |

Precedence: **alarmed** overrides **running** overrides **idle**. The agent count is displayed in both running and alarmed states.

**"Agent count" definition (resolved):** the count is the number of active **sessions** — distinct recently-modified transcripts under `~/.claude/projects/`. This falls directly out of the active-session detection in Step 1.2, so no extra tracking is needed. (Subagent counting — parsing `Task` spawns within a session — was considered and rejected as overengineered for the avatar; it may resurface as a per-session detail in a later phase.)

---

## Tablo — design brief (for the sprite designer)

Tablo is a cat. Keep it small, readable at ~64px, and expressive enough that its state is legible at a glance without reading the number.

**Phase 1 uses a text-glyph placeholder, not art.** Until the sprite exists, Tablo renders as a single centered glyph that reflects state:

| State | Glyph | Color |
|-------|-------|-------|
| **idle** | `I` | neutral/green |
| **running** | `A` | active/green (agent count shown alongside) |
| **alarmed** | `!` | warning/critical color |

Build the render layer so the glyph and the eventual sprite are **interchangeable behind the same state input** — swapping in the sprite sheet should not touch the state machine, event wiring, or agent-count badge logic. The glyph is a drop-in placeholder, not throwaway code.

**Animation states required (must map 1:1 to the Avatar State Model):**

| State | What Tablo does | Notes |
|-------|-----------------|-------|
| **idle** | sitting or curled asleep, slow breathing / occasional blink | calm, low-motion; this is the resting default |
| **running** | trotting / running in place | energy scales with activity; agent count badge visible |
| **alarmed** | ears back, wide eyes, agitated — startled posture | reads as "something needs attention"; count still visible |

**Agent count badge:** the active-session count, rendered as a small badge that floats **outside** Tablo at the top-right (notification-pip style), shown in running/alarmed states. See the Visual design direction section for the settled treatment.

**Deliverable format (see Open Questions #7):** confirm sprite sheet (PNG frames, fixed grid, documented frame order) vs. SVG/CSS animation. The code needs a documented frame/naming convention so the state→animation mapping can be built against it. Ship at least the three states above; extra flourishes (tap-to-hop, sleep Zs) are optional polish.

**Constraints:** transparent background, crisp at 64×64 (and ideally 2× for retina), consistent silhouette across states so transitions don't jump.

---

## Visual design direction

The look was explored across mockup iterations; this is the settled direction. Reference file: `tablo-mockups-v3.html` (interactive, both themes, all three surfaces). The build should match it.

**Concept — a warm lamp-lit desk, with subtle retro accents.** The feeling is a small warm light in a dark room: cozy, calm, easy to have on-screen all day. Two retro cues are folded in as *accents only* — a soft amber "phosphor" glow on active elements, and segmented-LED style meter bars. Explicitly avoided: full-CRT treatment (scanlines, pixel fonts, screen vignette) — it was tried (v2) and judged too heavy for an all-day desktop companion. Also deliberately avoided the terracotta-on-cream "AI default" palette.

**Two themes, both first-class:**
- **Dark** — warm off-white text on near-black warm browns; the hero mode (night work, nocturnal-watcher feel).
- **Light** — soft warm-paper / beige daytime counterpart.

**Palette (semantic, not decorative):**
- **Amber** (desk-lamp glow) = *working / active*.
- **Sage** (calm green) = *idle / healthy context*.
- **Coral** (soft red) = *alarmed / danger*, used only past the 60% line and for input-requested.
- Backgrounds are layered warm neutrals (room → surface → raised card → inset well).

The three status colors map 1:1 to the Avatar State Model, so color is a data signal, not styling.

**Typography:** a rounded, friendly face for Tablo's name and headings (warmth = the companion side) paired with a monospace for all session data — paths, percentages, token counts (precision = the monitor side). This two-face split encodes Tablo's dual character and should be preserved.

**Signature element:** the glow itself. The avatar breathes a soft colored glow that matches its state (slow sage when idle, amber pulse when working, urgent coral when alarmed). This is the one memorable flourish; everything else stays quiet.

**Avatar shape:** **hexagonal.** The glyph/sprite sits in a hexagon with a thin rim and the state glow following the hex outline. The active-session **count badge floats *outside* the hex** at the top-right (like a notification pip), not inside it. Note for implementation: in the real app the hexagon is just how the sprite is drawn (SVG `<polygon>`/`clipPath`, or a hex-shaped transparent PNG) — the transparent click-through window means the "shape" is simply the opaque art, so the hex is a drawing decision, not a code constraint, and doesn't lock out a plain-cat silhouette later.

**Wordmark:** lowercase **tablo** everywhere in the UI.

**Per-surface notes:**
- **Avatar** — hexagon + glyph (`I` / `A` / `!` placeholder pre-sprite) + external count badge. Glow animates per state.
- **Panel** — opens on tap. Header (logo, "tablo", session summary). Body groups sessions by state: **Input requested** first (coral accent, shows the pending question/approval inline), then **Working**. Each session row: project name, state badge, context %, path/branch, and a segmented-LED context bar with raw token count. Footer holds the full-width **Open dashboard** button showing the localhost address.
- **Dashboard** (browser, localhost) — compact inline headline stats (active / waiting / projects) kept space-efficient (no large stat cards; "peak context" was dropped). A sessions list with per-session context gauges, and a **plan-usage** block rendered as a **vertical** segmented-LED bar (matching the context-bar language) beside a plan-meta list (5h window, resets-in, weekly, plan tier).

**Meter styling:** context and plan bars use a segmented-LED look (discrete cells) with a soft glow on the lit portion, colored by the same amber/sage/coral thresholds. Warning percentages carry a gentle phosphor text-glow.

---

## Phase 0 — Project scaffolding

Set up a working Tauri app with the two-window (avatar + panel) shell and nothing else yet.

1. **Init the Tauri project.** Scaffold a Tauri 2 app (`create-tauri-app`). Confirm it builds and runs on the target OS (macOS first, per the dev machine).
2. **Configure the avatar window** in `tauri.conf.json`:
   - `decorations: false` (frameless)
   - `alwaysOnTop: true`
   - `transparent: true`
   - `resizable: false`
   - tiny size (~64×64), positioned in a corner (e.g. bottom-right)
   - `skipTaskbar: true`
   - **click-through everywhere except Tablo** — set the window ignore-cursor-events on, then selectively re-enable hit-testing over Tablo's bounds (the glyph in Phase 1, the sprite later) so only Tablo is interactive and the transparent margin passes clicks through to apps beneath. (Tauri: `set_ignore_cursor_events`.)
3. **Configure the panel window:** a second window, hidden on launch, frameless, not always-on-top by default, sized for the meter list (e.g. 300×220). It opens positioned adjacent to the avatar's current location.
4. **Wire tap → panel toggle.** Clicking the cat toggles the panel: show it anchored near the avatar, hide it on a second click or on blur/tap-away. Distinguish a *tap* (toggle panel) from a *drag* (reposition avatar) by a small movement threshold.
5. **Make the avatar draggable.** Dragging the cat repositions the avatar window; the panel follows on next open. Persist avatar position to config (Tauri app-config dir or `~/.config/`) and restore on launch.
6. **Verify the event bridge.** Emit a dummy event from Rust on a timer; confirm **both** windows receive it via `listen`. This proves the Rust→webview channel to each surface before real logic.

**Exit criteria:** A tiny transparent always-on-top cat window sits in a corner and passes clicks through except over the sprite; tapping it toggles a separate panel window; dragging moves it and the position persists; both windows receive a periodic test event from Rust.

---

## Phase 1 — Context meter (core deliverable)

### Step 1.1 — Locate transcripts
- Resolve `~/.claude/projects/` at runtime (expand home dir; do not hardcode).
- Enumerate subdirectories (each is a project slug) and the `.jsonl` files within.
- Handle the directory not existing (Claude Code never run) gracefully with a friendly "no sessions" state.

### Step 1.2 — Identify the active session(s)
- A session is "active" if its `.jsonl` was modified recently (e.g. within the last N minutes — make N configurable, default ~15).
- **Note:** the avatar's agent count and alarmed state are aggregate signals, so even in Phase 1 the backend must track *all* active sessions, not just one. The *panel* may still display a single session first (see Open Questions), but the active-session set is computed regardless. Keep parsing per-file so multi-session render is a later change, not a re-architecture.

### Step 1.3 — Tail the JSONL
- Use the `notify` crate to watch the active file(s) for append/modify events.
- On change, read the newly appended lines. Do **not** re-parse the whole file each time — track a byte offset per file and read only from there. (On file truncation/rotation, reset the offset.)
- Parse each new line as JSON; ignore malformed/partial trailing lines (a write may be mid-flight — read again on the next event).

### Step 1.4 — Compute context %
- Track the **latest** `assistant` message's `usage` block per session.
- Compute `context_used` and `context_pct` per the Data Model Reference above.
- Pick `context_limit` from `message.model`; keep the model→limit mapping in one small lookup table that is easy to update.
- Clamp/round for display. Store raw + percentage.

### Step 1.5 — Aggregate and push to the webviews
- From all active sessions, derive:
  - the **avatar state** (`idle` / `running` / `alarmed`) per the Avatar State Model precedence,
  - the **agent count** (per the resolved definition),
  - the **per-session context data** for the panel.
- Emit two events:
  - `avatar-update` → `{ state, agentCount }` to the avatar window.
  - `context-update` → `[{ session_id, project, pct, used, limit, model }, ...]` to the panel window.
- Debounce: coalesce rapid file events so updates fire at most a few times per second.

### Step 1.6 — Render Tablo (glyph placeholder in Phase 1)
- Match the **Visual design direction** section and `tablo-mockups-v3.html`: hexagonal frame, state-colored glow, lowercase wordmark.
- Map `avatar-update.state` to Tablo's glyph: idle → `I`, running → `A`, alarmed → `!` (sage / amber / coral respectively).
- Render the agent count (active-session count) as a badge **outside** the hex at the top-right when running or alarmed (hide when idle or count ≤ 1, per taste — decide in Open Questions).
- Keep the state→render mapping in **one** place so the eventual sprite sheet / SVG-CSS swaps in behind the same state input without touching the state machine or agent-count logic.
- Transition cleanly between states (no jarring flip when a session crosses 60% or an agent starts/stops); the glow cross-fades between state colors.

### Step 1.7 — Render the panel meter
- Match the panel in the **Visual design direction** section / `tablo-mockups-v3.html`: grouped by state (**Input requested** first, then **Working**), segmented-LED context bars, per-session project/badge/%/path.
- On tap-open, show one bar **per active session** (Phase 1 may show just the top session — see Open Questions), each with percentage and session/project name.
- **Color states:** green below 60%, warning 60–85%, critical above 85% (thresholds configurable; 60% is the required warning line per spec).
- Handle the compaction snap-back smoothly (animate down rather than flicker).
- Empty state ("no active sessions") when nothing is active.

### Step 1.8 — Warning behavior at >60%
- Avatar: enters **alarmed** animation.
- Panel: the affected bar turns to the warning color.
- Optional (config-gated, default off for Phase 1): a one-time OS notification when a session first crosses 60% upward. Fire once per crossing, not every update.

**Exit criteria:** With a real Claude Code session running, Tablo's glyph shows `A` with the correct active-session count, switches to `!` when any session's context passes 60%, and returns to `I` when work stops. Tapping Tablo opens the panel showing a live context-% bar that climbs, changes color past 60%, and recovers correctly after a compaction.

---

## Phase 2 — Multi-session view (light)

- Render more than one active session as a stacked/compact list.
- Sort by most-recently-active or by highest context %.
- Collapse/expand between compact (one bar) and expanded (list) modes.

*(No new data paths — this is a rendering extension of Phase 1.)*

---

## Phase 3 — Plan / session usage — **CANCELLED**

**Status: cancelled.** The core deliverable (live session-window / weekly plan
utilization) is not buildable from local data. Verified against disk: there is no
`anthropic-ratelimit-*`, resets-in, or quota-% anywhere under `~/.claude`
(`~/.claude.json`, `sessions/`, `cache/`, `stats-cache.json`, transcripts). The
live quota rides API response headers Claude Code keeps only in memory; obtaining
it would mean replaying the subscription OAuth token against the API — a ToS grey
area we chose not to enter. A token-rollup proxy was prototyped and deleted
(commit `2b3b163`).

**What was salvaged instead:** the one plan fact that *is* on disk — the static
subscription tier (`oauthAccount.organizationRateLimitTier`, e.g.
`default_claude_max_5x`) — is surfaced as a small "Max 5×" chip in the dashboard
header. No quota, no ToS issue. That's the entirety of Phase 3 that ships.

Original (unbuilt) intent, kept for context:
- ~~Add the second data source: Anthropic `anthropic-ratelimit-*` headers and/or the `~/.claude/` usage cache.~~
- ~~Show session-window and weekly plan utilization alongside context %.~~

---

## Phase 4 — Permissions (approve/deny) — **BUILT**

Tool-call approvals via a `PreToolUse` hook + a loopback IPC channel. Shipped on
branch `feat/multi-sessions`.

**Flow:** Claude Code fires the `PreToolUse` hook → `~/.claude/tablo/hook.sh`
(generated by Tablo) `curl`s the tool call to Tablo's loopback server and blocks
→ the server registers a pending decision, wakes the UI, and holds the request
open → the user taps **Approve/Deny** in the panel or dashboard → the decision
returns to the hook as `hookSpecificOutput` JSON (`permissionDecision`
allow/deny), which unblocks or blocks the tool.

**Key pieces:**
- `src-tauri/src/permission.rs` — a `tiny_http` server on `127.0.0.1:<permission_port>`
  (thread-per-request, each long-polls a channel until resolved), the tool-call
  summarizer, the generated hook script, and the `~/.claude/settings.json`
  install/uninstall (pure JSON merge, unit-tested: preserves existing keys,
  idempotent, keeps foreign hooks, prunes on removal).
- Pending approvals ride the existing `state-update` `Snapshot.pending` — the
  panel's "Input requested" group (previously empty) now renders them with
  Approve/Deny; the dashboard shows them too and hosts the enable toggle.
- Any pending approval forces the avatar to **alarmed** and drives the `waiting`
  count; a one-shot OS notification fires per new request (`notify_on_permission`).

**Fail-safe by design:** if Tablo is down / the port is busy / the user doesn't
decide within `hook_timeout_secs`, the hook emits `{}` → Claude Code falls back
to its normal permission flow. Nothing ever hangs a session.

**Consent:** the hook script is written on launch (Tablo's own dir, harmless),
but editing `~/.claude/settings.json` to actually intercept tools only happens
when the user flips **approvals on** in the dashboard (`set_hook_enabled`).

**Scope note:** only mutating tools are intercepted by default
(`intercept_tools` = Bash/Write/Edit/MultiEdit/NotebookEdit) so read-only tools
never pay the round-trip. All tunables live in `config::Config`
(`permission_port`, `intercept_tools`, `hook_timeout_secs`, `notify_on_permission`).
Requires `curl` (present on macOS).

---

## Phase 5 — Dashboard

- A localhost browser view aggregating context %, plan usage, permission history, and session list. Opened from the panel's **Open dashboard** button.
- Match the dashboard in the **Visual design direction** section / `tablo-mockups-v3.html`: compact inline headline stats (active / waiting / projects), a sessions list with per-session context gauges, and a **vertical** segmented-LED plan-usage bar with a plan-meta list.
- The avatar stays the tiny glanceable surface; the panel links out to this deep view.

---

## Build order summary

| Phase | Deliverable | New data path |
|-------|-------------|---------------|
| 0 | Avatar + panel two-window scaffold | — |
| 1 | Animated cat (state + agent count) + live context-% panel | Tail JSONL |
| 2 | Multi-session list in panel | — (render only) |
| 3 | ~~Plan/session usage~~ **cancelled** (only static plan-tier chip shipped) | — (no live quota on disk) |
| 4 | Permission approve/deny **(built)** | Claude Code `PreToolUse` hook + loopback server |
| 5 | Dashboard | — (aggregation) |

---

## Open questions

Resolve these before or during Phase 1; they affect Phase 1 rendering and config but not the core architecture.

1. **Panel display target for Phase 1:** show only the top/most-active session, or all active sessions from the start? (Backend tracks all regardless for the avatar — this only affects the panel's Phase 1 render.)
2. **Agent-count display rule:** show the session count always when running, or only when count > 1? Hide entirely when idle?
3. **"Active" threshold:** what modification-recency window counts a session as active? Default assumed ~15 min — confirm.
4. **Model→limit mapping:** confirm the exact model strings your sessions produce and their context limits (200k vs 1M), so the lookup table is correct against real transcripts.
5. **Warning thresholds:** 60% is required. Confirm the upper "critical" band (assumed 85%) and whether crossing 60% should trigger an OS notification or stay purely visual in Phase 1.
6. **Tablo art pipeline:** sprite sheet (PNG frames) or SVG/CSS animation? How many animation states/frames will the designed cat ship with, and what's the frame naming/loading convention the code should expect?
7. **Tap vs. drag threshold:** what movement distance separates a tap (toggle panel) from a drag (move avatar)?
8. **Panel dismissal:** dismiss on tap-away/blur, second tap, both? Should the panel be always-on-top while open?
9. **Frontend approach:** plain HTML/CSS/JS vs a light framework (Svelte/Solid). Plain keeps Phase 1 smallest; a framework eases the Phase 5 dashboard later.
10. **Config location & format:** where Tablo stores avatar position/thresholds (Tauri app-config dir vs a `~/.config/` path), and format (JSON/TOML).
11. **Multi-project scope:** watch all projects under `~/.claude/projects/`, or only the current working directory / a user-pinned project?
12. **Compaction display:** should the snap-back be silent, or briefly indicate "compacted" so the drop is understood?
