<script lang="ts">
  import { store } from "./state.svelte";
  import { beginDrag, moveAvatar, endDrag, togglePanel } from "./bridge";
  import type { AvatarState } from "./types";

  // Single source of truth for the state -> render mapping. Swapping the glyph
  // for a sprite sheet later means changing only this table, not the wiring.
  const GLYPH: Record<AvatarState, string> = { idle: "I", running: "A", alarmed: "!" };
  const CLASS: Record<AvatarState, string> = {
    idle: "idle",
    running: "working",
    alarmed: "alarmed",
  };

  let s = $derived(store.snap.state);
  let count = $derived(store.snap.agentCount);
  let showCount = $derived(s !== "idle" && count >= 1);
  // Pending tool approvals (Phase 4) — drive the extra-prominent "needs input"
  // shake, distinct from the gentler context-alarm.
  let needsInput = $derived(store.snap.waiting > 0);

  // --- tap vs. drag ---
  const DRAG_THRESHOLD = 6; // px of movement before a press becomes a drag
  let downScreen: { x: number; y: number } | null = null;
  let origin: { x: number; y: number } | null = null;
  let moved = false;
  let last: { x: number; y: number } | null = null;

  let raf = 0;
  let pending: { x: number; y: number } | null = null;
  function scheduleMove(x: number, y: number) {
    pending = { x, y };
    if (!raf) raf = requestAnimationFrame(flushMove);
  }
  function flushMove() {
    raf = 0;
    if (pending) {
      moveAvatar(pending.x, pending.y);
      pending = null;
    }
  }

  async function onDown(e: PointerEvent) {
    if (e.button !== 0) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    downScreen = { x: e.screenX, y: e.screenY };
    moved = false;
    origin = null;
    last = null;
    try {
      origin = await beginDrag();
      last = { ...origin };
    } catch {
      /* backend unavailable */
    }
  }

  function onMove(e: PointerEvent) {
    if (!downScreen || !origin) return;
    const dx = e.screenX - downScreen.x;
    const dy = e.screenY - downScreen.y;
    if (!moved && Math.hypot(dx, dy) > DRAG_THRESHOLD) moved = true;
    if (moved) {
      last = { x: origin.x + dx, y: origin.y + dy };
      scheduleMove(last.x, last.y);
    }
  }

  async function onUp(e: PointerEvent) {
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    const wasDrag = moved;
    const end = last ?? origin;
    downScreen = null;
    origin = null;
    moved = false;
    if (wasDrag && end) {
      if (raf) {
        cancelAnimationFrame(raf);
        raf = 0;
      }
      await moveAvatar(end.x, end.y);
      endDrag(end.x, end.y);
    } else {
      // A tap toggles the panel.
      togglePanel();
    }
  }
</script>

<div
  class="avatar-hit"
  role="button"
  tabindex="0"
  aria-label="Tablo — {s}"
  onpointerdown={onDown}
  onpointermove={onMove}
  onpointerup={onUp}
>
  <div class="tablo-wrap" class:needs-input={needsInput}>
    <div class="tablo {CLASS[s]}" class:needs-input={needsInput}>
      <span class="glyph">{GLYPH[s]}</span>
    </div>
    {#if showCount}
      <span class="count" class:alarmed={s === "alarmed"}>{count}</span>
    {/if}
  </div>
</div>

<style>
  .avatar-hit {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: transparent;
  }

  .tablo-wrap {
    position: relative;
    display: inline-grid;
    place-items: center;
  }

  /* hexagonal avatar. clip-path clips box-shadow, so the glow uses a
     drop-shadow filter (which follows the hex), and the rim is a colored base
     layer with an inset fill. */
  .tablo {
    position: relative;
    width: 58px;
    height: 64px;
    display: grid;
    place-items: center;
    font-family: var(--font-mono);
    font-weight: 700;
    font-size: 20px;
    cursor: pointer;
    --hex: polygon(50% 0%, 100% 25%, 100% 75%, 50% 100%, 0% 75%, 0% 25%);
    clip-path: var(--hex);
    background: var(--border);
    transition: transform 0.25s var(--ease), filter 0.35s var(--ease);
  }
  .tablo::before {
    content: "";
    position: absolute;
    inset: 2px;
    clip-path: var(--hex);
    background: var(--bg-surface);
    z-index: 0;
  }
  .tablo .glyph {
    position: relative;
    z-index: 1;
  }
  .avatar-hit:hover .tablo {
    transform: translateY(-3px);
  }

  /* count badge — floats outside the hex, off the top-right edge */
  .count {
    position: absolute;
    top: -6px;
    right: -9px;
    min-width: 19px;
    height: 19px;
    padding: 0 4px;
    border-radius: 999px;
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 700;
    display: grid;
    place-items: center;
    border: 2px solid var(--bg-inset);
    z-index: 3;
    background: var(--amber);
    color: #1c1409;
  }
  :global([data-theme="light"]) .count {
    color: #fff;
  }
  .count.alarmed {
    background: var(--coral);
    color: #fff;
  }

  /* idle: gentle sage, slow breathing glow */
  .tablo.idle {
    color: var(--sage);
    background: color-mix(in srgb, var(--sage) 45%, var(--border));
    animation: breathe 4.5s var(--ease) infinite;
  }
  @keyframes breathe {
    0%,
    100% {
      filter: drop-shadow(0 0 7px color-mix(in srgb, var(--sage) 30%, transparent));
    }
    50% {
      filter: drop-shadow(0 0 13px color-mix(in srgb, var(--sage) 55%, transparent));
    }
  }

  /* working: amber lamp glow, quicker pulse */
  .tablo.working {
    color: var(--amber);
    background: color-mix(in srgb, var(--amber) 55%, var(--border));
    animation: work 1.8s var(--ease) infinite;
  }
  @keyframes work {
    0%,
    100% {
      filter: drop-shadow(0 0 9px color-mix(in srgb, var(--amber) 40%, transparent));
    }
    50% {
      filter: drop-shadow(0 0 16px color-mix(in srgb, var(--amber) 75%, transparent));
    }
  }

  /* alarmed: coral, urgent flush */
  .tablo.alarmed {
    color: var(--coral);
    background: color-mix(in srgb, var(--coral) 60%, var(--border));
    animation: alarm 0.9s var(--ease) infinite;
  }
  @keyframes alarm {
    0%,
    100% {
      filter: drop-shadow(0 0 9px color-mix(in srgb, var(--coral) 45%, transparent));
      transform: translateX(0);
    }
    25% {
      transform: translateX(-1.5px);
    }
    50% {
      filter: drop-shadow(0 0 17px color-mix(in srgb, var(--coral) 80%, transparent));
    }
    75% {
      transform: translateX(1.5px);
    }
  }

  /* needs-input: a pending tool approval — much more agitated than the context
     alarm. The shake lives on the wrap so the count badge rattles along with the
     hex; the hex itself does a hard coral flush. */
  .tablo-wrap.needs-input {
    animation: shake 0.5s var(--ease) infinite;
  }
  @keyframes shake {
    0%,
    100% {
      transform: translate3d(0, 0, 0) rotate(0deg);
    }
    15% {
      transform: translate3d(-3px, 1px, 0) rotate(-4deg);
    }
    30% {
      transform: translate3d(3px, -1px, 0) rotate(4deg);
    }
    45% {
      transform: translate3d(-3px, 1px, 0) rotate(-3deg);
    }
    60% {
      transform: translate3d(3px, -1px, 0) rotate(3deg);
    }
    75% {
      transform: translate3d(-2px, 0, 0) rotate(-2deg);
    }
    90% {
      transform: translate3d(1px, 0, 0) rotate(1deg);
    }
  }
  /* Overrides the .alarmed animation with a harder, faster coral pulse. */
  .tablo.needs-input {
    color: var(--coral);
    background: color-mix(in srgb, var(--coral) 70%, var(--border));
    animation: needs-glow 0.85s var(--ease) infinite;
  }
  @keyframes needs-glow {
    0%,
    100% {
      filter: drop-shadow(0 0 11px color-mix(in srgb, var(--coral) 60%, transparent));
    }
    50% {
      filter: drop-shadow(0 0 24px color-mix(in srgb, var(--coral) 95%, transparent));
    }
  }
</style>
