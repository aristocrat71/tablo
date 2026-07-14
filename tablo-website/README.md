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
    Cat.astro         the sprite state machine (idle/alert/working/settling/play), cursor-reactive
    DesktopDemo.astro Act 3's interactive macOS-desktop scene (coffee wallpaper, menu bar with
                      tablo in it, a terminal running an agent session, cat asleep top-right).
                      Owns the type-to-run interaction, the panel + dashboard pops, and the toast.
    LiveWidget.astro  the tablo panel — faithful port of the app's Panel.svelte: grouped sessions
                      (critical/permission/waiting/working), sort seg + filter popover
                      (state + source, mirrors FilterButton.svelte), approve/deny. Pops from the cat.
    Dashboard.astro   the larger tablo window — port of the app's Dashboard.svelte: statline header
                      (active·waiting·projects) + info/gear, a pinned critical card + sessions card,
                      filter popover. Opened from the panel's "dashboard" link.
    SessionCard.astro one dashboard session row + its amber-phosphor "$ live preview" terminal
                      (#/> markers, user/tool/text lines, live caret) — from Dashboard.svelte's snippets.
    InstallDeck.astro faux terminal, OS-detect tabs, copy buttons (commands from ../release-plan.md)
  pages/index.astro  the six acts (boot → hero → watches → live → install → sign-off) + orchestration
```

## The scroll journey

`Act 0` cold boot (mounts both agents, power-on flash) · `Act 1` the familiar
(reactive hero cat + mood slider) · `Act 2` what it does (7-item phosphor feature
readout; the marquee ticker was removed) · `Act 3` **the live demo** — an
interactive macOS desktop: coffee wallpaper, menu bar with tablo living in it, a
terminal running a claude session, the cat asleep top-right. Type a prompt (≤20
chars) and send → the cat wakes and **runs** (amber), then **naps** (sage) and a
**waiting toast** flies out to the cat's left for 3s. **Tap the cat** → the real
**panel** pops (anchored by the cat); its **`dashboard ↗`** link → the
**dashboard** window (scaled to 0.75, scrolls internally, contained in the
screen). · `Act 4` get it (install one-liners) · `Act 5` sign-off (cat sleeps).

## Notes

- **The panel / dashboard / toast are faithful ports** of the app's
  `src/lib/Panel.svelte`, `Dashboard.svelte`, `SessionCard`↔`Dashboard.svelte`
  snippets, `FilterButton.svelte`, and `Toast.svelte`. When the app UI changes,
  re-port them (markup + scoped styles copied over; sample data + progressive-
  enhancement scripts are the only additions). The desktop demo uses static
  sample sessions, not live data.
- **Assets are reused, not redrawn.** Sprite frame math is baked into `Cat.astro`
  (uniform-grid `steps()`; frame counts 4/4/5/5/4).
- **No emojis** — LED dots, CSS shapes, ASCII, and the pixel sprites only.
- **Accessibility:** `prefers-reduced-motion` kills the CRT/animations and freezes
  sprites; a manual `[ crt: off ]` toggle (footer) does the same, persisted.
- **Deploy — Cloudflare Pages** (live at **https://tablo.unravel.tech**). The site
  lives in this subdirectory of the Tauri repo, so set in the Pages project:
  - **Root directory:** `tablo-website`
  - **Build command:** `bun run build` (npm works too — the package manager is
    detected from `bun.lock`)
  - **Build output directory:** `dist`
  - **Custom domain:** add `tablo.unravel.tech` under the project's *Custom domains*;
    since `unravel.tech` is on Cloudflare the CNAME is created automatically (else
    add `CNAME tablo → <project>.pages.dev` at your DNS host).

  The `build` script first runs `scripts/ensure-root-tsconfig.mjs`, which stubs the
  gitignored `../.svelte-kit/tsconfig.json` on a fresh clone so Astro's bundler
  doesn't abort reading the repo-root tsconfig. It never clobbers a real generated
  one, so local `bun run build` is unaffected.

  The canonical URL is set in `astro.config.mjs` (`site`) — used for OG/Twitter
  image URLs. Confirm the `install.sh` / `install.ps1` URLs once they land.

Easter eggs: konami code, triple-click the cat.
