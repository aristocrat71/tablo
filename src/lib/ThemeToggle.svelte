<script lang="ts">
  import { store, toggleTheme } from "./state.svelte";

  // Reflects the *active* theme: moon while dark, sun while light. Drawn as
  // line-art SVG (currentColor) so it stays crisp and on-brand — no emoji glyph.
  let dark = $derived(store.config.theme !== "light");
</script>

<button
  class="theme-toggle"
  onclick={toggleTheme}
  title={dark ? "Switch to light theme" : "Switch to dark theme"}
  aria-label="Toggle theme"
>
  {#if dark}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
    </svg>
  {:else}
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <circle cx="12" cy="12" r="4.2" />
      <line x1="12" y1="2.5" x2="12" y2="4.5" />
      <line x1="12" y1="19.5" x2="12" y2="21.5" />
      <line x1="2.5" y1="12" x2="4.5" y2="12" />
      <line x1="19.5" y1="12" x2="21.5" y2="12" />
      <line x1="5.4" y1="5.4" x2="6.8" y2="6.8" />
      <line x1="17.2" y1="17.2" x2="18.6" y2="18.6" />
      <line x1="5.4" y1="18.6" x2="6.8" y2="17.2" />
      <line x1="17.2" y1="6.8" x2="18.6" y2="5.4" />
    </svg>
  {/if}
</button>

<style>
  .theme-toggle {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--ink-dim);
    cursor: pointer;
    transition:
      color 0.2s var(--ease),
      border-color 0.2s var(--ease),
      background-color 0.2s var(--ease);
  }
  .theme-toggle:hover {
    color: var(--ink);
    border-color: var(--ink-faint);
  }
  .theme-toggle svg {
    width: 15px;
    height: 15px;
    display: block;
  }
</style>
