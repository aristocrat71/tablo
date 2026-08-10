<script lang="ts">
  // Popover with the state (waiting/working) + source (claude/codex/opencode) filters,
  // shared by the panel and dashboard toolbars. Drives the shared `prefs` store.
  import { prefs, toggleFilter, toggleSource } from "./prefs.svelte";

  let { showSource = false }: { showSource?: boolean } = $props();
  let open = $state(false);
  let wrap: HTMLDivElement;

  // How many filters are currently hiding something, to badge the button.
  let hidden = $derived(
    (prefs.showWaiting ? 0 : 1) +
      (prefs.showWorking ? 0 : 1) +
      (showSource && !prefs.showClaude ? 1 : 0) +
      (showSource && !prefs.showCodex ? 1 : 0) +
      (showSource && !prefs.showOpencode ? 1 : 0),
  );

  // Dismiss on outside-click / Esc, bound only while open. Capture phase so Esc
  // closes the popover instead of the surrounding window.
  $effect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => wrap && !wrap.contains(e.target as Node) && (open = false);
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && ((open = false), e.stopPropagation());
    document.addEventListener("mousedown", onDown, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", onDown, true);
      document.removeEventListener("keydown", onKey, true);
    };
  });
</script>

<div class="wrap" bind:this={wrap}>
  <button class="btn" class:on={open || hidden > 0} aria-haspopup="menu" aria-expanded={open} onclick={() => (open = !open)}>
    filter
    {#if hidden > 0}<span class="badge">{hidden}</span>{/if}
    <span class="caret">&gt;</span>
  </button>

  {#if open}
    <div class="pop" role="menu">
      <div class="label">state</div>
      <button class="item" class:off={!prefs.showWaiting} role="menuitemcheckbox" aria-checked={prefs.showWaiting} onclick={() => toggleFilter("waiting")}>
        <span class="led wait"></span>waiting
      </button>
      <button class="item" class:off={!prefs.showWorking} role="menuitemcheckbox" aria-checked={prefs.showWorking} onclick={() => toggleFilter("working")}>
        <span class="led work"></span>working
      </button>
      {#if showSource}
        <div class="label">source</div>
        <button class="item" class:off={!prefs.showClaude} role="menuitemcheckbox" aria-checked={prefs.showClaude} onclick={() => toggleSource("claude")}>
          <span class="led src"></span>claude
        </button>
        <button class="item" class:off={!prefs.showCodex} role="menuitemcheckbox" aria-checked={prefs.showCodex} onclick={() => toggleSource("codex")}>
          <span class="led src"></span>codex
        </button>
        <button class="item" class:off={!prefs.showOpencode} role="menuitemcheckbox" aria-checked={prefs.showOpencode} onclick={() => toggleSource("opencode")}>
          <span class="led src"></span>opencode
        </button>
      {/if}
    </div>
  {/if}
</div>

<style>
  .wrap { position: relative; display: inline-flex; }
  .btn {
    display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 9px;
    border-radius: 8px; border: 1px solid var(--border-soft); background: var(--bg-surface);
    color: var(--ink-faint); cursor: pointer;
    font-family: var(--font-mono); font-size: 10.5px; font-weight: 600; letter-spacing: 0.02em;
    transition: color 0.18s var(--ease), background-color 0.18s var(--ease), border-color 0.18s var(--ease);
  }
  .btn:hover { color: var(--ink-dim); }
  .btn.on { color: var(--ink); border-color: var(--border); background: var(--bg-raised); }
  .badge {
    display: inline-flex; align-items: center; justify-content: center; min-width: 14px; height: 14px;
    padding: 0 3px; border-radius: 999px; background: var(--amber); color: var(--bg-surface);
    font-size: 9px; font-weight: 700; line-height: 1;
  }
  .caret { font-size: 10px; line-height: 1; opacity: 0.7; }

  .pop {
    position: absolute; top: calc(100% + 6px); right: 0; z-index: 40; min-width: 148px; padding: 6px;
    border-radius: 10px; background: var(--bg-surface); border: 1px solid var(--border); box-shadow: var(--shadow-panel);
  }
  .label {
    padding: 6px 8px 3px; font-family: var(--font-mono); font-size: 9px; font-weight: 700;
    letter-spacing: 0.1em; text-transform: uppercase; color: var(--ink-faint);
  }
  .label:first-child { padding-top: 2px; }
  .item {
    display: flex; align-items: center; gap: 8px; width: 100%; padding: 6px 8px; border: none;
    border-radius: 6px; background: transparent; color: var(--ink); cursor: pointer;
    font-family: var(--font-mono); font-size: 11px; font-weight: 600;
    transition: background-color 0.15s var(--ease), opacity 0.15s var(--ease);
  }
  .item:hover { background: var(--bg-raised); }
  .item.off { color: var(--ink-faint); opacity: 0.72; }
  .led { width: 8px; height: 8px; border-radius: 999px; flex-shrink: 0; transition: background-color 0.15s var(--ease); }
  .led.wait { background: var(--sage); box-shadow: 0 0 6px var(--sage); }
  .led.work { background: var(--amber); box-shadow: 0 0 6px var(--amber); }
  .led.src { background: var(--ink-dim); box-shadow: 0 0 5px color-mix(in srgb, var(--ink-dim) 60%, transparent); }
  .item.off .led { background: color-mix(in srgb, var(--ink-faint) 55%, transparent); box-shadow: none; }
</style>
