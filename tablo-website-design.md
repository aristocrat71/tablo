# tablo — website design

> A single-screen CRT world you fall into. The cat wakes, watches, works, and
> sleeps as you scroll. No pricing table, no feature grid, no "trusted by."
> Just Tablo's world, rendered loud.

**Stack:** Astro (static, islands-only JS). **Theme:** warm-phosphor retro / CRT.
**Lives in:** `tablo-website/` at repo root. **Reuses:** the existing pixel-cat
sprite sheets, the logo, and the panel CSS from `tablo-mockups-v3.html`.

**What the app is now (source of truth = `README.md`):** one cat that watches
**two agents** — **Claude Code** (`~/.claude/projects/`) *and* **OpenAI Codex**
(`~/.codex/sessions/`) — side by side. Each session row carries a live one-line
activity, a neutral `claude` / `codex` source tag, a permission-mode badge, and a
segmented-LED context meter; Claude tool calls can be gated behind an
Approve/Deny tap; a jump button focuses the terminal a session runs in. The site
must tell the *two-agents-one-cat* story and mirror the real panel anatomy — use
the `tablo-mockups-v3.html` panel CSS as the styling base, but bring the rows up
to the current README (source tag + activity line + permission rows).

This is a build spec, not a mockup. Every token, effect, asset, and act below is
meant to be implemented as written.

---

## 1. North star

Tablo the app is a *cozy, restrained* retro — warm lamp glow, LED accents, no
full CRT (they deliberately rejected scanlines/pixel-fonts as too heavy for an
all-day companion). **The website is the opposite end of the same world: the
maximalist expression.** It's the arcade-cabinet boot screen the cozy widget
lives inside. Same palette, same cat, same two fonts — but dialled to eleven.

The reconciliation that keeps it on-brand: **warm amber phosphor, not neon
matrix-green.** Every generic "hacker" site is green-on-black. Tablo is a *warm*
CRT — amber/sage/coral phosphor on near-black warm brown. That single palette
choice is what makes it read as Tablo and not as a Linux distro landing page.

**The throughline — a day in the life of the familiar:**

```
  scroll ↓            cat state           lamp glow
  ───────────────────────────────────────────────────
  boot                cold → warm-up      off → flicker on
  act 1  the familiar sits, blinks        sage breathe
  act 2  it watches    gets up, trots     amber shifts
  act 3  live widget   working / alarmed  amber → coral
  act 4  get it        sits, attentive    steady amber
  act 5  sign-off      curls up, sleeps   dims to ember
```

The whole page is one continuous scene. The cat is the scroll indicator, the
mascot, and the demo, all at once.

---

## 2. Design principles

1. **Vibe over information.** The copy is < 150 words total. Everything else is
   motion, glow, and the cat. If a section reads like marketing, cut it.
2. **The cat is alive.** It reacts to cursor, scroll, and idle time. It is never
   a static image except in the favicon.
3. **One palette, three meanings.** amber = working, sage = calm, coral = alert.
   Color is a signal on the site exactly as it is in the app. Never decorative.
4. **Everything glows, nothing shouts.** Phosphor bloom on lit text; the dark
   does the heavy lifting. High contrast where it counts (body copy), soft
   everywhere else.
5. **Retro is earned, not pasted.** Scanlines/curvature/flicker are a coherent
   CRT system with a single source of truth, and a hard kill-switch for
   `prefers-reduced-motion`. No random glitch for glitch's sake.
6. **No emojis, ever.** All glyphs are ASCII, CSS shapes, inline SVG (Lucide), or
   the pixel sprites. (Carries the app's house rule to the web.)

---

## 3. Tech stack & project shape

**Astro** — static HTML output, zero JS shipped by default, hydrate only the
interactive islands. Matches Tablo's small-footprint ethos and gives us
`<ClientRouter />` view-transitions for the "reboot" gag.

**No UI framework.** The interactivity is small and bespoke: IntersectionObserver
for scroll triggers, `requestAnimationFrame` + CSS `steps()` for sprites, a few
`<script>` islands. Pulling in React/Svelte would dwarf the payload. If a reactive
store is wanted for the live-widget loop, use `nanostores` (~1 KB) — nothing
heavier.

**Fonts:** self-host via `@fontsource` — `Quicksand` (400/500/600/700) and
`JetBrains Mono` (400/500/700). `font-display: swap`. Two faces only, same split
as the app: rounded = warmth/headings, mono = precision/all data & terminal.

**Assets:** copy the sheets and logo into the site; do **not** re-derive them.

```
tablo-website/
├─ astro.config.mjs          # site url, ClientRouter, sitemap
├─ package.json
├─ public/
│  ├─ sprites/               # copied from ../static/sprites/*
│  │  ├─ running-sprite-sheet.png
│  │  ├─ sleeping-sprite-sheet.png
│  │  ├─ shocked-sprite-sheet.png
│  │  ├─ running-to-sleeping-sprite-sheet.png
│  │  └─ playing-sprite-sheet.png        # concat of the 3 parts (see §8)
│  ├─ tablo-logo-v4.png      # from ../assets — keep its green bg, DO NOT strip
│  ├─ tablo-cat-logo.png     # from ../assets — head, transparent
│  └─ favicon.png            # from ../static (128×128)
├─ src/
│  ├─ layouts/Screen.astro   # the CRT bezel + overlay wrapper (§7)
│  ├─ styles/
│  │  ├─ tokens.css          # §6 — all custom properties
│  │  └─ crt.css             # §7 — scanline/flicker/curvature layer
│  ├─ components/
│  │  ├─ BootSequence.astro  # act 0 island
│  │  ├─ Cat.astro           # sprite state machine island (§8)
│  │  ├─ StateChips.astro    # act 2
│  │  ├─ LiveWidget.astro    # act 3 — reuses mockup panel CSS
│  │  ├─ InstallDeck.astro   # act 4 — os tabs + copy (§12)
│  │  ├─ LedMeter.astro      # the segmented-LED motif, reused everywhere
│  │  ├─ Marquee.astro       # ticker
│  │  └─ Cursor.astro        # block cursor + phosphor trail
│  └─ pages/
│     └─ index.astro         # the single page; composes the acts
└─ README.md
```

One route. One page. Everything is a scroll.

---

## 4. Design tokens

Pulled straight from `tablo-mockups-v3.html` so the site and app share a
bloodline, then extended with website-only "phosphor" tokens the app doesn't need.

### 4.1 Color — dark is the hero (and the default)

```css
:root, [data-theme="dark"] {
  /* room + surfaces (warm near-black browns) */
  --bg-room:     #14110e;   /* the dark room — page base */
  --bg-surface:  #201b16;   /* raised panels */
  --bg-raised:   #2a231c;   /* cards */
  --bg-inset:    #191410;   /* wells, meter tracks, terminal bg */
  --border:      #372e25;
  --border-soft: #2c241d;

  /* ink */
  --ink:       #f3e8dc;     /* warm off-white — body */
  --ink-dim:   #b7a794;
  --ink-faint: #7d6f5f;

  /* semantic phosphor — the three meanings */
  --amber: #e0a458;         /* working / active — the desk lamp */
  --sage:  #8faa7e;         /* idle / calm / healthy */
  --coral: #e08267;         /* alarmed / danger / needs-you */
  --amber-soft: #3a2c1a;
  --sage-soft:  #29301f;
  --coral-soft: #3a231c;
}
```

**Light theme is optional and secondary.** The app ships both, but the website's
hero *is* the dark room. If a light mode is added, port the `[data-theme="light"]`
block from the mockup verbatim and gate the CRT effects down (scanlines nearly
invisible on paper). Recommendation: ship dark-only for v1, add a small
`[ ☀ / ☾ ]` toggle later only if wanted. Do not spend the vibe budget on it.

### 4.2 Phosphor / glow tokens (website-only)

```css
:root {
  --glow-amber: 0 0 12px color-mix(in srgb, var(--amber) 60%, transparent);
  --glow-sage:  0 0 12px color-mix(in srgb, var(--sage) 55%, transparent);
  --glow-coral: 0 0 12px color-mix(in srgb, var(--coral) 60%, transparent);
  --text-bloom: 0 0 18px, 0 0 40px;         /* two-stop text-shadow bloom */
  --scanline-opacity: 0.06;                  /* dial 0 for reduced-motion */
  --scanline-gap: 3px;
  --flicker-depth: 0.03;
  --grain-opacity: 0.045;
}
```

### 4.3 Type scale

Mono is for *everything that is data or terminal*. Rounded is for the wordmark
and the (very few) human headings.

```
--font-round: 'Quicksand', system-ui, sans-serif;   /* headings, wordmark */
--font-mono:  'JetBrains Mono', ui-monospace, monospace; /* data, terminal, labels */

display   clamp(3.5rem, 12vw, 9rem)   700   -0.03em   wordmark only
h-scene   clamp(1.75rem, 4vw, 3rem)   600   -0.02em   act titles
lead      clamp(1rem, 2.2vw, 1.4rem)  500              taglines
body      1rem / 1.6                   400              the rare paragraph
mono-lg   1.125rem                     500              install commands
mono      0.8125rem / 1.4              400   0.02em     labels, meters, paths
mono-xs   0.6875rem                    500   0.06em     ticker, captions, uppercase
```

### 4.4 Space, radius, motion

```
space:  4 · 8 · 12 · 16 · 24 · 40 · 64 · 96 · 160   (px, 4/8 rhythm)
radius: --r-lg 20 · --r-md 14 · --r-sm 9 · --r-pill 999
        (the CRT glass itself uses a larger, uneven curvature — §7)
ease:   --ease cubic-bezier(.22,.61,.36,1)     (default, from the app)
        --ease-out-back cubic-bezier(.34,1.56,.64,1)  (cat perks, chip pops)
        --step steps(N)                        (sprites — never smooth)
motion: micro 150–250ms · scene reveals 300–500ms · sprite loops 0.4–1.2s
        boot sequence ~1.5s (skippable, once per session)
z:      0 room · 10 content · 20 cat · 30 fixed marquee · 40 cursor
        · 90 CRT overlay (scanlines/vignette, pointer-events:none) · 100 boot
```

---

## 5. The CRT / phosphor effect system

This is the "highly graphic" layer. It's **one fixed overlay** (`crt.css`) that
sits above everything at `z-90` with `pointer-events: none`, plus a few per-element
treatments. Single source of truth so it can be killed in one place.

### 5.1 The glass (bezel + curvature)

Desktop: the viewport is framed by a chunky monitor bezel — a `--bg-raised`
rounded rectangle with a soft inner shadow and two "screws" (CSS radial dots) in
the corners, and a tiny amber power LED bottom-right that pulses. Content scrolls
*inside* the glass. Mobile: bezel collapses to a thin frame, edge-to-edge glass.

Subtle barrel curvature on the glass via an inset `box-shadow` vignette + a very
slight `border-radius` growth toward corners. Do **not** use a heavy SVG
displacement filter (perf + text legibility). The vignette + rounded corners
read as "tube" enough.

### 5.2 Scanlines (the always-on texture)

```css
.crt-overlay::before {
  content: ""; position: fixed; inset: 0; z-index: 90; pointer-events: none;
  background: repeating-linear-gradient(
    to bottom,
    transparent 0 calc(var(--scanline-gap) - 1px),
    rgba(0,0,0, var(--scanline-opacity)) var(--scanline-gap)
  );
  mix-blend-mode: multiply;
}
```

### 5.3 Beam sweep + flicker

A single faint horizontal band drifts top→bottom every ~8s (the "refresh beam"),
and the whole overlay does a barely-there opacity flicker (`--flicker-depth`).
Both are pure CSS keyframes, both disabled under reduced-motion.

### 5.4 Phosphor bloom + chromatic aberration

Lit text (wordmark, active labels, meter values) gets the bloom:

```css
.phosphor { text-shadow: var(--text-bloom) currentColor; }
/* RGB split — apply sparingly, headings only */
.aberrate { text-shadow: -1px 0 rgba(224,130,103,.5), 1px 0 rgba(143,170,126,.5); }
```

### 5.5 Grain

A tiny tiled PNG/SVG noise at `--grain-opacity`, fixed, `pointer-events:none`,
above the room and below content. Kills the flat-gradient look.

### 5.6 Kill switch (mandatory)

```css
@media (prefers-reduced-motion: reduce) {
  :root { --scanline-opacity: 0; --flicker-depth: 0; --grain-opacity: .02; }
  /* beam sweep, boot animation, sprite loops → paused / first-frame only */
}
```

Also expose a manual `[ crt: on/off ]` chip in the footer that toggles a
`data-crt="off"` attribute on `<html>` for anyone who finds it much. Persist to
`localStorage`.

---

## 6. Assets & the sprite animation system

**Reuse only — never redraw.** Every cat state on the site maps to a sheet we
already have. All sprites render with `image-rendering: pixelated;` (crisp 8-bit,
never blurred).

### 6.1 Inventory & frame math

Measured dimensions, so `steps()` counts are exact:

| Sheet (`/public/sprites/`) | Sheet px | Frames | Frame px | Drives |
|---|---|---|---|---|
| `running-sprite-sheet.png` | 2400×477 | **4** | 600×477 | **working** — trot loop |
| `sleeping-sprite-sheet.png` | 1024×196 | **4** | 256×196 | **idle at rest** — curl/breathe loop |
| `shocked-sprite-sheet.png` | 1908×544 | **5** (verify)¹ | ~382×544 | **alarmed** / cursor-startled |
| `running-to-sleeping-sprite-sheet.png` | 1030×180 | verify¹ | — | scroll transition into sleep |
| `playing-sprite-sheet.png` | concat of 3 parts² | ~11 | — | easter egg (pounce) |

Static marks (not animated):

| File | Use |
|---|---|
| `tablo-cat-logo.png` (562×512, transparent head) | hero mark, nav, sign-off |
| `tablo-logo-v4.png` (1191×915, **green bg baked in**) | social/OG image, boot splash — **do not strip the bg** |
| `favicon.png` (128×128) | favicon / apple-touch |

¹ `shocked` (1908/5 = 381.6) and `running-to-sleeping` don't divide to whole
pixels — the sheets aren't a clean uniform grid. **Resolve one of two ways at
build time (choose in §16):** (a) re-export each sheet to a uniform grid, or
(b) ship a `sprites.json` frame-map (`{sheet, frameW, frames:[{x,w}]}`) and drive
`background-position` from JS instead of a single `steps()`. The two clean 4-frame
sheets (`running`, `sleeping`) work with pure CSS `steps()` today.

² The playing animation currently lives in three PNGs at repo root
(`playing-sprite-sheet-part1/2/3.png`). Concatenate them into one horizontal
`playing-sprite-sheet.png` during the copy step. Easter-egg only, so low priority.

### 6.2 The pure-CSS sprite pattern (clean sheets)

```css
.cat {
  --fw: 600px; --fh: 477px; --frames: 4; --dur: .6s;
  width: var(--fw); height: var(--fh);
  background-image: url('/sprites/running-sprite-sheet.png');
  background-size: calc(var(--fw) * var(--frames)) var(--fh);
  image-rendering: pixelated;
  animation: sprite var(--dur) steps(var(--frames)) infinite;
}
@keyframes sprite { to { background-position-x: calc(-1 * var(--fw) * var(--frames)); } }
```

Scale for layout with `transform: scale()` (keeps pixels crisp) — not by resizing
`--fw` (which resamples).

### 6.3 The Cat island — one state machine

`Cat.astro` owns a single element whose state is set by a `data-state` attribute;
a small script swaps the sheet + frame vars. **This is the same decoupling the app
uses** (one state → one render table), so the mental model matches the product.

```
data-state   sheet                       loop      set by
──────────────────────────────────────────────────────────────────────
idle         sleeping                    slow      default / cursor idle >4s
alert        shocked                     once→hold cursor moves fast, or hover cat
working      running                     fast      scroll enters act 2 / act 3
sleeping     running-to-sleeping→sleeping once→loop scroll reaches act 5
play         playing                     once      konami / triple-click cat
```

Precedence mirrors the app: **alarmed > working > idle**. Eyes-follow-cursor is a
cheap parallax (translate the sprite ±3px toward the pointer); it sells "alive"
more than any single animation.

---

## 7. The scroll journey (the acts)

Six acts, one continuous vertical scroll inside the glass. Each is roughly one
viewport but they bleed into each other — the cat and the lamp glow carry across
cuts so it never feels sectioned. Think scenes, not slides.

```
┌─ CRT bezel ────────────────────────────────────────────┐
│  ▓ marquee ticker (fixed top) ────────────────────────▓ │
│                                                         │
│   ░░░ scroll ░░░                                        │
│                                                         │
│   [0] cold boot      → warm-up flash                    │
│   [1] the familiar   → giant cat, wordmark, cursor      │
│   [2] it watches     → cat trots, 3 state chips         │
│   [3] live widget    → the real panel, live meters      │
│   [4] get it         → terminal install deck            │
│   [5] sign-off       → cat curls up, lamp dims          │
│                                                         │
│                              · pwr ●  (amber LED)       │
└─────────────────────────────────────────────────────────┘
```

### Act 0 — COLD BOOT (`z-100`, ~1.5s, once per session)

Black glass. One amber block cursor blinks twice. A fast mono boot log types out
— note it mounts **both** agents, planting the two-agents story from the first
frame:

```
tablo v0.1.0  ·  familiar daemon
> mounting ~/.claude/projects  (claude code) ... ok
> mounting ~/.codex/sessions   (codex) ....... ok
> tailing transcripts ....................... ok
> familiar: ONLINE
```

Then a classic CRT power-on: a bright horizontal line snaps open vertically
(scaleY 0→1 white flash), the room fades up, scanlines settle in. `sessionStorage`
gates it to first load; `prefers-reduced-motion` → skip straight to Act 1;
`[ skip ]` bracket link bottom-right always available.

### Act 1 — THE FAMILIAR (hero)

- **Cat**, large, centered-low, `idle` (sleeping/breathing) sprite, sage glow
  pooling under it (the lamp). Eyes track cursor. Move fast → `alert` (shocked)
  for ~1s, then settles.
- **Wordmark** `tablo` behind/above the cat — `--font-round`, display size,
  `.phosphor` amber bloom, one slow scanline sweep across the letters on load.
- **Tagline**, mono, with a blinking block cursor:
  `a tiny cat that watches your coding agents▮`
- **Sub-tagline** (mono-xs, `--ink-faint`), the two-agents hook stated once, up
  top: `claude code + codex · one cat · your corner`. The two agent names render
  as the neutral source tags (§7 Act 3), never state-colored.
- A soft amber desk-lamp light-pool at the very bottom edge, always present,
  color-shifting per act.
- Scroll cue: a vertical **segmented-LED bar** (the app's meter motif) on the
  right edge = the page scroll progress. It *is* the scrollbar. Amber fill.

### Act 2 — IT WATCHES (the concept, told through motion)

As Act 2 enters view, the cat **stands and trots** (`working`/running loop) along
an invisible shelf from left toward center; the lamp glow warms from sage→amber.
Three state chips reveal in sequence (stagger 120ms), each an `LedMeter` + label:

```
◦ sage   IDLE      "curled up, nothing running"       breathe
◦ amber  WORKING   "agents on the move — count shown"  pulse
◦ coral  ALARMED   "context past 60%, or needs you"    flush
```

Each chip, when it enters, momentarily pushes the cat + lamp into that state
(sage→amber→coral) so you *watch* the state machine instead of reading it. This
is the entire "what it does" story — no feature list.

**Coda — one cat, two agents.** As Act 2 hands off to Act 3, two source tags
slide in beneath the cat and dock together on the same shelf:

```
   ▮ claude    ▮ codex          one cat. both agents. same corner.
```

The cat glances between them (eyes parallax left↔right once). Tags are neutral
(`--ink-dim` on `--bg-raised`, never state-colored) — this is the visual promise
that the single avatar count, panel, and dashboard fold Claude Code *and* Codex
together. That's the whole "two agents" section: one line, one glance, no grid.

### Act 3 — LIVE WIDGET (the money shot)

A pixel-faithful recreation of the actual Tablo panel, **using the
`tablo-mockups-v3.html` panel CSS as the styling base** (the `.panel`,
`.session`, `.ctx-*` LED bars, `.group-*` — lift them wholesale into
`LiveWidget.astro`) **but with rows brought up to the current README anatomy**.
It sits on a faux desktop backdrop (a blurred warm wallpaper + a mini avatar hex
in the corner).

**Row anatomy (match the real panel, not the older mockup).** Each session row
shows, in order:

```
┌───────────────────────────────────────────────────────────┐
│ optilife            ⟨ai title⟩        ▮ claude   ASK  64%  │  ← project · title · src tag · state · pct
│ ● editing scanner.rs                          mode: plan   │  ← live activity + status dot · perm-mode badge
│ ▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░  128k / 200k               [ jump ]   │  ← segmented-LED context bar + tokens · jump btn
└───────────────────────────────────────────────────────────┘
```

- **source tag** — neutral `▮ claude` / `▮ codex` pill, `--ink-dim`, never
  state-colored (this is what makes the two-agents story legible in the money shot).
- **live activity line** — a mono one-liner with a colored status dot:
  `editing scanner.rs`, `running cargo check`, `waiting for you`. Ticks/changes
  during the loop — the single most "alive" detail after the cat.
- **permission-mode badge** — small read-only `mode: plan` / `accept-edits`.
- **jump button** — a `[ jump ]` bracket-button (decorative here; glitch-on-hover).

**Groups, most-urgent-first (match the panel):**
`Context window warning` (pinned top, red `> N%` header) · `Permission Request` ·
`Waiting` · `Working`.

It's **live on a loop**, not a screenshot — and the loop tells the two-agents +
permissions story:

1. Rows for **both agents** — a `claude` session and a `codex` session — sit in
   **Working**, context bars climbing via `requestAnimationFrame`, activity lines
   ticking (`running cargo check` → `editing lib.rs`).
2. A **Permission Request** appears: a Claude row rises into that group with an
   inline `Allow rm -rf ./build ?` and **`[ Approve ] [ Deny ]`** bracket-buttons.
   The mini-avatar flips to `alarmed` and a coral count pip lights.
3. The fake cursor taps **Approve**; the row settles back to Working (sage flash).
4. A session crosses the 60% warning line → its bar flips amber→coral, the
   `session-pct` gains the coral text-glow, and the row slides up into the pinned
   **Context window warning** group; the Act-3 lamp glow flushes coral.
5. That session finishes → drops to **Waiting** (`waiting for you`), a **toast**
   slides out of the corner avatar (`optilife · <title>  [ jump ]`), then the loop
   resets after ~10s.

This shows both agents, the live activity, permissions, the context warning, and
the toast — the whole product — without embedding a video. Everything animates
`transform`/`opacity`/`width` only; respects reduced-motion (freeze at the
alarmed frame with the Approve/Deny visible, no loop).

**Optional companion — the dashboard terminal.** Beside or below the panel, a
second faux window can show the dashboard's live `$ live preview` activity feed
(the app literally renders a terminal-style stream of what each agent is doing).
It's the most on-theme element in the whole product for a CRT site — a real
scrolling terminal. Ship it if there's room; it doubles as the Act-3 backdrop.

### Act 4 — GET IT (install) — see §9 for full spec

Framed as a terminal window inside the glass. This is the one place with a real
call to action, and it's a copy-paste line, not a button funnel.

### Act 5 — SIGN-OFF

The cat plays the `running-to-sleeping` transition then loops `sleeping`; the lamp
dims to a low ember; scanlines dim; the marquee slows. A mono sign-off:

```
made in a dark room.
tablo  ·  [ github ]  ·  MIT  ·  [ crt: on ]  ·  [ ^ reboot ]
```

`[ ^ reboot ]` uses the View Transition to fade to black and replay Act 0 (the
gag: scrolling to the top literally reboots the machine).

---

## 8. Signature graphic moments (the "crazy" catalogue)

The vibe budget, itemized. Ship the ★ ones for v1; the rest are polish.

- ★ **Live reactive cat** — cursor tracking + idle→sleep + startle. The single
  most important effect. Nothing else matters if the cat feels dead.
- ★ **CRT glass** — bezel, scanlines, vignette, power LED (§5).
- ★ **Cold-boot sequence** — types + power-on flash (Act 0).
- ★ **LED scroll bar** — the scrollbar is the app's segmented meter, filling amber.
- ★ **Live widget loop** — Act 3 money shot.
- ★ **Marquee ticker** (fixed top): a faux live status feed scrolling right→left,
  mono, tagged by agent, e.g.
  `claude·optilife 64% ▮ codex·api 71% ▮ claude·tablo 23% ▮ 3 sessions · 1 waiting · 1 approval ▮`.
  Pure decoration, sells "it's watching both."
- **Block cursor + phosphor trail** — replace the OS cursor with a chunky amber
  block that leaves a short decaying phosphor trail (respect touch → off).
- **Type-on headings** — act titles typewriter-reveal on scroll-in via
  IntersectionObserver.
- **Glitch-on-hover** — bracket buttons (`[ ... ]`) do a 1-frame RGB-split jitter
  on hover, then snap. 80ms, never idle-looping.
- **ASCII dust** — faint drifting mono characters in the dark background
  (parallax, very low opacity) — the "particles in the lamp light."
- **Degauss on theme/CRT toggle** — a quick wobble+flash when toggling, like a
  real monitor degauss.

---

## 9. Install section (from `release-plan.md`)

The only "conversion" moment. Rendered as a terminal window: `--bg-inset` body,
mono, a fake title bar (`● ● ●  tablo — install`), a blinking prompt. **OS
auto-detected** from `navigator.platform`/UA; that tab preselected. Tabs are
`[ macos ]` `[ linux ]` `[ windows ]` bracket-buttons.

**Commands (per the release plan — GitHub Releases, quiet install, unsigned):**

macOS / Linux tab:
```
curl -fsSL https://raw.githubusercontent.com/unravel-team/tablo/main/install.sh | bash
```
Caption (mono-xs, `--ink-faint`):
`downloads from GitHub Releases · strips the quarantine flag so Gatekeeper never prompts`

Windows tab:
```
irm https://raw.githubusercontent.com/unravel-team/tablo/main/install.ps1 | iex
```
Caption: `runs Unblock-File so SmartScreen stays quiet · PowerShell`

Each command block has a **copy button** (right-aligned, `[ copy ]`). On click:
copy to clipboard, swap label to `[ ✓ copied ]` in sage `.phosphor` for 1.5s,
fire a tiny degauss flash. (Checkmark is an inline SVG or the ASCII `✓`, not an
emoji.)

Below the tabs, three quiet mono lines (no cards, no icons):
```
free · unsigned · Actions-built    no Apple/MS fee, the install script skips the OS prompt
mac · win · linux                  one binary per platform, straight from Releases
open source                        [ github.com/unravel-team/tablo ]  ·  MIT
```

A single secondary bracket-link: `[ view all releases → ]` →
`github.com/unravel-team/tablo/releases`. That's the whole install act. No email
capture, no "get started free," no tiers.

> Keep the exact install URLs in sync with `install.sh`/`install.ps1` once those
> land (release-plan TODO). Until the first `v0.1.0` tag exists, point the
> `[ releases ]` link at the repo and note "first release soon" in mono-xs.

---

## 10. Motion & interaction spec

| Element | Trigger | Motion | Duration / easing |
|---|---|---|---|
| Boot log | load (1st/session) | type-on + power flash | ~1.5s, skippable |
| Wordmark | load | scanline sweep + bloom fade-in | 700ms `--ease` |
| Act titles | 40% in view | typewriter | 40ms/char |
| State chips | in view | stagger fade+rise | 120ms apart, `--ease-out-back` |
| Cat (all) | data-state | `steps()` sprite, never eased | 0.4–1.2s loops |
| Cat eyes | pointermove | ±3px translate toward cursor | 120ms |
| Context bars | Act 3 loop | width climb | rAF, ~6s cycle |
| Bracket btn | hover | RGB-split jitter → snap | 80ms |
| Copy btn | click | label swap + degauss | 1.5s hold |
| Reboot | click `^ reboot` | view-transition fade-to-black → Act 0 | 600ms |
| **All of the above** | `prefers-reduced-motion` | first-frame static, no loops, instant reveals | — |

Standard rules: transform/opacity only (no width/top animation except the meters,
which are `will-change: width` and short); enter ease-out, exit ease-in and
faster; nothing loops that isn't the cat, the lamp, the marquee, or a meter.

---

## 11. Responsive strategy

- **≥1024px** — full bezel, cat at hero scale, live widget at true size, block
  cursor on. This is the showcase.
- **768–1024px** — bezel thins, cat scales ~0.7, widget scales to fit, chips wrap.
- **<768px (mobile)** — bezel → thin frame, edge-to-edge glass. Cat scales to
  ~40vw and sits above the wordmark. Marquee stays. **Block cursor + trail OFF**
  (touch). Live widget becomes a single stacked column (the app's own mobile
  fallback). Install tabs become a full-width stack. Scanlines stay but lighter.
- Type via `clamp()` throughout; no fixed-px containers; `min-h-dvh` not `100vh`.
- Test at 375 / 768 / 1024 / 1440. No horizontal scroll (the marquee uses
  `transform`, not overflow, so it never adds page width).

---

## 12. Accessibility (non-negotiable)

- **Contrast:** body `--ink #f3e8dc` on `--bg-room #14110e` ≈ 14:1. Keep all
  real reading copy on `--ink`/`--ink-dim`; reserve `--ink-faint` for
  decoration/captions only. Semantic colors used for meaning always pair with a
  text label (never color-alone) — same rule as the app.
- **Reduced motion:** the §5.6 kill switch is mandatory. Boot skips, sprites
  freeze on a sensible frame, marquee stops, no beam/flicker/trail. The site must
  be fully legible and navigable static.
- **Keyboard:** OS tabs, copy buttons, all `[ bracket ]` links are real
  `<button>`/`<a>` with visible focus rings (2px `--amber`, not removed). Tab
  order matches visual order. `[ skip ]` on the boot is focusable first.
- **Screen readers:** sprites are decorative (`role="img"` + concise `aria-label`
  like "pixel cat, sleeping"; the marquee is `aria-hidden`). Real content
  (tagline, state meanings, install commands) is live DOM text, not baked into
  images. Commands are selectable text, not screenshots.
- **The cursor:** custom block cursor never removes the ability to click; hide it
  and restore the system cursor under reduced-motion / no-hover.
- **No zoom lock**, `viewport` meta standard, respects Dynamic Type / browser
  zoom (clamp scales, nothing truncates).

---

## 13. Performance budget

- **JS:** < 30 KB total shipped (islands only). No framework runtime.
- **Sprites:** the sheets are the heaviest assets (running is 2400px wide).
  Serve as-is (they're small PNGs, 85–180 KB) but `loading="lazy"` on
  below-fold acts; only the hero cat sheet is eager. Consider a `webp` copy for
  the static logo marks (keep PNG sprites for crisp pixels).
- **Fonts:** two families, subset to latin, `swap`, preload only the two used in
  the hero (Quicksand 700, JetBrains Mono 400).
- **CLS:** reserve the cat's box (fixed `--fw`/`--fh` × scale) and the widget's
  height so nothing jumps as sprites/JS load. Boot overlay is fixed, not layout.
- **LCP:** the wordmark (text) is the LCP — instant. The boot must not block it
  under reduced-motion.
- Astro static output + `ClientRouter`; host on any static CDN (Vercel/Netlify/
  GH Pages). Add `astro-sitemap` and OG tags using `tablo-logo-v4.png`.

---

## 14. Copy deck (Tablo's voice: terse, warm, a little nocturnal)

Total word count target: **< 150.** Every line earns its glow.

```
wordmark      tablo
tagline       a tiny cat that watches your coding agents
sub-tagline   claude code + codex · one cat · your corner

boot          tablo v0.1.0 · familiar daemon
              > mounting ~/.claude/projects  (claude code) ... ok
              > mounting ~/.codex/sessions   (codex) ....... ok
              > tailing transcripts ... ok
              > familiar: ONLINE

act 2 title   it watches, so you don't have to
  idle        curled up. nothing running.
  working     agents on the move. it counts them.
  alarmed     context past 60%, or something needs you.
  coda        one cat. both agents. same corner.

act 3 title   the whole thing, live
  sub         tap the cat — every session's context, activity, and approvals,
              claude and codex side by side, in real time
  perms       gate a tool call behind a tap: approve, or deny.

act 4 title   put it on your desk
  mac/linux   one line. no gatekeeper dance.
  windows     one line. no smartscreen shrug.
  footnote    free · unsigned · open source

sign-off      made in a dark room.
              tablo · [ github ] · MIT · [ ^ reboot ]

marquee       (faux live) claude·optilife 64% ▮ codex·api 71% ▮ claude·tablo 23% ▮
              3 sessions · 1 waiting · 1 approval ▮  (repeat)
```

Voice rules: lowercase everywhere (matches the `tablo` wordmark rule); mono for
anything that looks like data or a command; no exclamation points except the app's
own `!` alarmed glyph; never say "AI-powered", "revolutionize", "seamless", or
"boost productivity."

---

## 15. Easter eggs (optional, low priority)

- **Konami code** (`↑↑↓↓←→←→ba`) → cat plays the `playing` sprite (pounce), lamp
  strobes amber, marquee prints `> meow`.
- **Triple-click the cat** → same play animation, once.
- **`[ crt: off ]`** in footer really does strip the whole CRT layer (persisted) —
  a genuine feature disguised as an egg.
- **Idle 30s anywhere** → cat yawns and curls to sleep; first mouse move wakes it
  with a startle. (Reinforces "it's watching *you* too.")

---

## 16. Open decisions (for Mitul)

1. **Light theme:** ship dark-only v1 (recommended) or port the light palette now?
2. **Sprite sheet slicing:** re-export `shocked`/`running-to-sleeping` to a clean
   uniform grid, or ship a `sprites.json` frame-map and drive from JS?
   (Recommend the JSON map — no asset re-authoring, and it future-proofs new
   sheets.)
3. **Custom block cursor:** on by default desktop, or opt-in? (Some find replaced
   cursors annoying — recommend on, with the CRT-off toggle also restoring it.)
4. **Boot sequence frequency:** once per session (recommended) or every load?
5. **Host + domain:** GH Pages under the repo, or a `tablo.*` domain? Affects
   `astro.config` `site` and the install-script URLs.
6. **Install URLs:** confirm final `install.sh` / `install.ps1` locations before
   wiring the copy blocks (blocked on the release-plan TODO).
7. **Dashboard terminal in Act 3:** include the live `$ live preview` terminal
   companion (very on-theme, more to build/animate) or keep Act 3 to the panel
   only? (Recommend include — it's the most CRT-native element the product has.)
8. **Codex parity in copy:** README says Codex is on by default but a couple of
   its extras are opt-in (Codex jump). The site treats both agents as first-class
   and doesn't surface per-agent toggles — confirm that's the framing you want.

---

## 17. Build order

1. **Scaffold** `tablo-website/` (Astro, fontsource, tokens.css, crt.css, copy
   assets in). Bezel + scanlines + one static cat visible. — the frame.
2. **Cat island** — state machine + the two clean sprite loops + cursor tracking.
   Get "the cat is alive" landing first; it's the whole site.
3. **Acts 1 & 5** — hero + sign-off (bookends), lamp glow carry, LED scrollbar.
4. **Act 2** — trot + state chips + state-driven lamp + two-agents coda tags.
5. **Act 3** — lift the mockup panel CSS, upgrade rows to the README anatomy
   (source tag + activity line + perm-mode badge), wire the live loop (both
   agents → permission request → Approve → context warning → toast). Optional
   dashboard `$ live preview` terminal companion.
6. **Act 4** — install deck, OS detect, copy buttons (real URLs when ready).
7. **Act 0** — boot sequence last (it's the intro but depends on everything
   being warm to fade into).
8. **Polish** — marquee, type-on, glitch hover, ASCII dust, easter eggs.
9. **Pass** — reduced-motion audit, contrast check, 375/768/1024/1440,
   keyboard + SR, perf budget. Ship.

---

*The cat wakes, watches, works, and sleeps. If a visitor scrolls the whole page
without reading a word and still gets it — that's the site.*
