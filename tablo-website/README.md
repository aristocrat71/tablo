# tablo — website

The marketing site for **tablo**, built as a single-screen warm-phosphor CRT
world you scroll through. The pixel cat wakes, watches, works, and sleeps as you
move down the page. Built with **Astro** (static output, islands-only JS).

Design spec: [`../tablo-website-design.md`](../tablo-website-design.md).

## Run

```bash
bun install
bun run dev        # dev server (astro dev)
bun run build      # static build -> ./dist
bun run preview    # serve the build
```

> Astro is pinned to `7.0.5` because a `minimum-release-age` install policy
> blocks anything newer than 7 days. Bump it when a newer version ages in.

## Structure

```
public/
  sprites/           reused pixel-cat sheets (running / sleeping / shocked / r2s / playing)
  tablo-cat-logo.png, tablo-logo-v4.png, favicon.png   (copied from ../assets, ../static)
src/
  styles/
    tokens.css       colours, fonts, motion, phosphor tokens (from tablo-mockups-v3.html)
    crt.css          the CRT overlay: bezel, scanlines, beam, grain + reduced-motion kill switch
    global.css       reset, bracket buttons, LED meter, cursor, LED scrollbar, [hidden] guard
  layouts/Screen.astro     html shell + CRT frame + block cursor + ClientRouter/OG
  components/
    Cat.astro        the sprite state machine (idle/alert/working/settling/play), cursor-reactive
    LiveWidget.astro the real tablo panel, recreated + run on a loop (dual-agent + permission beat)
    InstallDeck.astro faux terminal, OS-detect tabs, copy buttons (commands from ../release-plan.md)
  pages/index.astro  the six acts (boot → hero → watches → live → install → sign-off) + orchestration
```

## The scroll journey

`Act 0` cold boot (mounts both agents, power-on flash) · `Act 1` the familiar
(reactive hero cat) · `Act 2` it watches (state chips + one-cat-two-agents coda)
· `Act 3` live widget (claude + codex, live activity, approve/deny, context
warning, toast) · `Act 4` get it (install one-liners) · `Act 5` sign-off (cat
sleeps).

## Notes

- **Assets are reused, not redrawn.** Sprite frame math is baked into `Cat.astro`
  (uniform-grid `steps()`; frame counts 4/4/5/5/4).
- **No emojis** — LED dots, CSS shapes, ASCII, and the pixel sprites only.
- **Accessibility:** `prefers-reduced-motion` kills the CRT/animations and freezes
  sprites; a manual `[ crt: off ]` toggle (footer) does the same, persisted.
- **Deploy:** static `dist/` to any CDN. Set the real domain in `astro.config.mjs`
  (`site`) and confirm the `install.sh` / `install.ps1` URLs once they land.

Easter eggs: konami code, triple-click the cat.
