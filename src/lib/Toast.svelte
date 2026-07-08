<script lang="ts">
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import { prefs } from "./prefs.svelte";
  import { showToast, hideToast, onSessionWaiting } from "./bridge";

  // Surface a gentle toast when Rust reports a session finished working and is now
  // waiting on the user. Detection lives in the backend (`session-waiting`); here
  // we just present it, for the configured hover time.
  let toast = $state<{ project: string; title: string | null } | null>(null);
  let clearT: ReturnType<typeof setTimeout> | undefined;
  let hideT: ReturnType<typeof setTimeout> | undefined;

  const OUTRO_MS = 320;

  function fire(project: string, title: string | null) {
    clearTimeout(clearT);
    clearTimeout(hideT);
    toast = { project, title };
    showToast().catch(() => {});
    const visible = prefs.waitingToastSecs * 1000;
    clearT = setTimeout(() => (toast = null), visible);
    hideT = setTimeout(() => hideToast().catch(() => {}), visible + OUTRO_MS + 60);
  }

  onMount(() => {
    const un = onSessionWaiting((sessions) => {
      if (!prefs.notifyOnWaiting || sessions.length === 0) return;
      // Multiple at once → a count (no single title); otherwise project + title.
      if (sessions.length > 1) fire(`${sessions.length} sessions`, null);
      else fire(sessions[0].project, sessions[0].title);
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
  </div>
{/if}

<style>
  .toast {
    display: flex;
    align-items: center;
    max-width: 238px;
    padding: 12px 15px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: var(--shadow-panel);
  }
  .txt {
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
</style>
