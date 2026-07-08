<script lang="ts">
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import { prefs } from "./prefs.svelte";
  import { showToast, hideToast, onSessionWaiting, jumpToSession, type WaitingSession } from "./bridge";

  // Surface a gentle toast when Rust reports a session finished working and is now
  // waiting on the user. Detection lives in the backend (`session-waiting`); here
  // we just present it, for the configured hover time.
  let toast = $state<{ id: string; project: string; title: string | null; canJump: boolean } | null>(null);
  let clearT: ReturnType<typeof setTimeout> | undefined;
  let hideT: ReturnType<typeof setTimeout> | undefined;

  const OUTRO_MS = 320;

  function fire(t: { id: string; project: string; title: string | null; canJump: boolean }) {
    clearTimeout(clearT);
    clearTimeout(hideT);
    toast = t;
    showToast().catch(() => {});
    const visible = prefs.waitingToastSecs * 1000;
    clearT = setTimeout(() => (toast = null), visible);
    hideT = setTimeout(() => hideToast().catch(() => {}), visible + OUTRO_MS + 60);
  }

  onMount(() => {
    const un = onSessionWaiting((sessions: WaitingSession[]) => {
      if (!prefs.notifyOnWaiting || sessions.length === 0) return;
      // Multiple at once → a count (no title / jump); otherwise a single session.
      if (sessions.length > 1) fire({ id: "", project: `${sessions.length} sessions`, title: null, canJump: false });
      else fire(sessions[0]);
    });
    return () => un.then((u) => u()).catch(() => {});
  });
</script>

{#if toast}
  <div class="toast" transition:fly={{ x: 16, duration: OUTRO_MS }}>
    <div class="txt">
      <div class="head">
        <span class="proj">{toast.project}</span>
        {#if toast.title}
          <span class="sep">·</span>
          <span class="ttl">{toast.title}</span>
        {/if}
      </div>
      <div class="sub">waiting for you</div>
    </div>
    {#if toast.canJump}
      <button class="jump" onclick={() => jumpToSession(toast!.id).catch(() => {})}>jump &rarr;</button>
    {/if}
  </div>
{/if}

<style>
  .toast {
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: 276px;
    padding: 12px 15px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: var(--shadow-panel);
  }
  .txt {
    flex: 1;
    min-width: 0;
    line-height: 1.3;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 6px;
    white-space: nowrap;
  }
  .proj {
    flex-shrink: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--ink);
  }
  .sep {
    flex-shrink: 0;
    color: var(--ink-faint);
  }
  .ttl {
    min-width: 0;
    font-size: 12px;
    color: var(--ink-dim);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-faint);
    margin-top: 1px;
  }
  .jump {
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--amber);
    background: var(--amber-soft);
    border: 1px solid color-mix(in srgb, var(--amber) 32%, transparent);
    border-radius: 6px;
    padding: 5px 9px;
    cursor: pointer;
    white-space: nowrap;
  }
</style>
