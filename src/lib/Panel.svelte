<script lang="ts">
  import { store, applyTheme } from "./state.svelte";
  import { setTheme, openDashboard } from "./bridge";
  import { tokens, pct } from "./format";
  import type { SessionView } from "./types";

  let snap = $derived(store.snap);
  let asks = $derived(snap.sessions.filter((s) => s.state === "ask"));
  let works = $derived(snap.sessions.filter((s) => s.state === "run"));

  let sub = $derived.by(() => {
    const n = snap.sessions.length;
    if (!snap.hasProjectsDir) return "Claude Code hasn't run yet";
    if (n === 0) return "no active sessions";
    const base = `${n} session${n > 1 ? "s" : ""}`;
    return snap.waiting > 0 ? `${base} · ${snap.waiting} waiting on you` : base;
  });

  function toggleTheme() {
    const next = store.config.theme === "dark" ? "light" : "dark";
    store.config.theme = next;
    applyTheme(next);
    setTheme(next);
  }
</script>

<div class="panel-shell">
  <div class="panel">
    <div class="panel-top">
      <div class="panel-glyph">a</div>
      <div class="panel-titles">
        <div class="name">tablo</div>
        <div class="sub">{sub}</div>
      </div>
      <button class="mini" title="Toggle theme" onclick={toggleTheme}>☾</button>
    </div>

    <div class="panel-body">
      {#if snap.sessions.length === 0}
        <div class="empty">
          <div class="empty-glyph">I</div>
          <p>{snap.hasProjectsDir ? "Nothing running right now." : "No Claude Code sessions found yet."}</p>
          <span>Tablo wakes up when an agent starts working.</span>
        </div>
      {:else}
        {#if asks.length}
          <div class="group-head">
            <span class="group-dot attn"></span>
            <span class="group-name">Input requested</span>
            <span class="group-count">{asks.length}</span>
          </div>
          {#each asks as s (s.id)}
            {@render sessionRow(s)}
          {/each}
        {/if}

        {#if works.length}
          <div class="group-head">
            <span class="group-dot work"></span>
            <span class="group-name">Working</span>
            <span class="group-count">{works.length}</span>
          </div>
          {#each works as s (s.id)}
            {@render sessionRow(s)}
          {/each}
        {/if}
      {/if}
    </div>

    <div class="plan-strip">
      <div class="plan-head">
        <span class="group-name">Plan usage</span>
        <span class="plan-basis">tokens · rollup</span>
      </div>
      <div class="plan-row">
        <span class="plan-lab">5h</span>
        <div class="plan-track">
          <div class="plan-fill {snap.plan.fiveHourLevel}" style="width:{snap.plan.fiveHourPct}%"></div>
        </div>
        <span class="plan-val">{tokens(snap.plan.fiveHourTokens)}</span>
      </div>
      <div class="plan-row">
        <span class="plan-lab">7d</span>
        <div class="plan-track">
          <div class="plan-fill {snap.plan.weekLevel}" style="width:{snap.plan.weekPct}%"></div>
        </div>
        <span class="plan-val">{tokens(snap.plan.weekTokens)}</span>
      </div>
    </div>

    <div class="panel-foot">
      <button class="dash-btn" onclick={() => openDashboard()}>
        Open dashboard
        <span class="host">the deep view</span>
      </button>
    </div>
  </div>
</div>

{#snippet sessionRow(s: SessionView)}
  <div class="session" class:needs={s.state === "ask"}>
    <div class="session-line1">
      <span class="session-proj">{s.project}</span>
      <span class="session-badge {s.state === 'ask' ? 'ask' : 'run'}">
        {s.state === "ask" ? "ASK" : "RUN"}
      </span>
      <span class="session-pct" class:warn={s.level === "warn"} class:crit={s.level === "crit"}>
        {pct(s.pct)}
      </span>
    </div>
    <div class="session-path">
      {s.path}{s.branch ? ` · ${s.branch}` : ""}
    </div>
    <div class="ctx">
      <div class="ctx-track">
        <div class="ctx-fill {s.level}" style="width:{s.pct}%"></div>
      </div>
      <span class="ctx-cap">{tokens(s.used)} / {tokens(s.limit)}</span>
    </div>
  </div>
{/snippet}

<style>
  .panel-shell {
    width: 100%;
    height: 100%;
    padding: 8px;
  }
  .panel {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--bg-surface);
    border: 1px solid var(--border);
    border-radius: var(--r-lg);
    box-shadow: var(--shadow-panel);
    overflow: hidden;
  }
  .panel-top {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 16px 18px 14px;
    border-bottom: 1px solid var(--border-soft);
  }
  .panel-glyph {
    width: 34px;
    height: 38px;
    display: grid;
    place-items: center;
    font-family: var(--font-mono);
    font-weight: 700;
    font-size: 15px;
    color: var(--amber);
    background: var(--amber-soft);
    clip-path: polygon(50% 0%, 100% 25%, 100% 75%, 50% 100%, 0% 75%, 0% 25%);
  }
  .panel-titles {
    flex: 1;
    line-height: 1.25;
  }
  .panel-titles .name {
    font-size: 15px;
    font-weight: 700;
    letter-spacing: -0.01em;
  }
  .panel-titles .sub {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-faint);
    margin-top: 1px;
  }
  .mini {
    width: 28px;
    height: 28px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--ink-dim);
    cursor: pointer;
    display: grid;
    place-items: center;
    transition: all 0.2s var(--ease);
  }
  .mini:hover {
    color: var(--ink);
    border-color: var(--ink-faint);
  }

  .panel-body {
    flex: 1;
    padding: 6px 12px 12px;
    overflow-y: auto;
  }

  .group-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 14px 6px 8px;
  }
  .group-dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
  }
  .group-dot.attn {
    background: var(--coral);
    box-shadow: 0 0 8px var(--coral);
  }
  .group-dot.work {
    background: var(--amber);
    box-shadow: 0 0 8px var(--amber);
  }
  .group-name {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--ink-dim);
  }
  .group-count {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-faint);
    margin-left: auto;
  }

  .session {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-radius: var(--r-md);
    background: var(--bg-raised);
    border: 1px solid var(--border-soft);
    margin-bottom: 7px;
    transition: border-color 0.2s var(--ease), transform 0.12s var(--ease);
  }
  .session:hover {
    border-color: var(--border);
    transform: translateX(2px);
  }
  .session.needs {
    border-color: color-mix(in srgb, var(--coral) 40%, var(--border-soft));
  }

  .session-line1 {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .session-proj {
    font-size: 13.5px;
    font-weight: 600;
    letter-spacing: -0.01em;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .session-badge {
    font-family: var(--font-mono);
    font-size: 9.5px;
    font-weight: 600;
    padding: 2px 6px;
    border-radius: 5px;
    letter-spacing: 0.03em;
    white-space: nowrap;
  }
  .session-badge.ask {
    background: var(--coral-soft);
    color: var(--coral);
  }
  .session-badge.run {
    background: var(--sage-soft);
    color: var(--sage);
  }
  .session-pct {
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 600;
    color: var(--ink-dim);
  }
  .session-pct.warn {
    color: var(--amber);
    text-shadow: 0 0 12px color-mix(in srgb, var(--amber) 55%, transparent);
  }
  .session-pct.crit {
    color: var(--coral);
    text-shadow: 0 0 12px color-mix(in srgb, var(--coral) 55%, transparent);
  }

  .session-path {
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--ink-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    margin-top: -2px;
  }

  .ctx {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .ctx-track {
    flex: 1;
    height: 8px;
    border-radius: 3px;
    background: var(--bg-inset);
    overflow: hidden;
    border: 1px solid var(--border-soft);
    background-image: repeating-linear-gradient(
      90deg,
      transparent 0 5px,
      color-mix(in srgb, var(--bg-surface) 75%, black) 5px 6px
    );
  }
  .ctx-fill {
    height: 100%;
    transition: width 0.5s var(--ease), background-color 0.4s var(--ease);
    background-image: repeating-linear-gradient(90deg, transparent 0 5px, rgba(0, 0, 0, 0.25) 5px 6px);
  }
  .ctx-fill.ok {
    background-color: var(--sage);
    box-shadow: 0 0 8px -1px var(--sage);
  }
  .ctx-fill.warn {
    background-color: var(--amber);
    box-shadow: 0 0 9px -1px var(--amber);
  }
  .ctx-fill.crit {
    background-color: var(--coral);
    box-shadow: 0 0 9px -1px var(--coral);
  }
  .ctx-cap {
    font-family: var(--font-mono);
    font-size: 9.5px;
    color: var(--ink-faint);
    white-space: nowrap;
  }

  .plan-strip {
    padding: 10px 14px 12px;
    border-top: 1px solid var(--border-soft);
    background: var(--bg-inset);
    display: flex;
    flex-direction: column;
    gap: 7px;
  }
  .plan-head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin-bottom: 1px;
  }
  .plan-basis {
    margin-left: auto;
    font-family: var(--font-mono);
    font-size: 9.5px;
    color: var(--ink-faint);
    letter-spacing: 0.02em;
  }
  .plan-row {
    display: flex;
    align-items: center;
    gap: 9px;
  }
  .plan-lab {
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 600;
    color: var(--ink-dim);
    width: 20px;
  }
  .plan-track {
    flex: 1;
    height: 7px;
    border-radius: 3px;
    background: var(--bg-surface);
    overflow: hidden;
    border: 1px solid var(--border-soft);
    background-image: repeating-linear-gradient(
      90deg,
      transparent 0 5px,
      color-mix(in srgb, var(--bg-inset) 75%, black) 5px 6px
    );
  }
  .plan-fill {
    height: 100%;
    transition: width 0.5s var(--ease), background-color 0.4s var(--ease);
    background-image: repeating-linear-gradient(90deg, transparent 0 5px, rgba(0, 0, 0, 0.25) 5px 6px);
  }
  .plan-fill.ok {
    background-color: var(--sage);
    box-shadow: 0 0 8px -1px var(--sage);
  }
  .plan-fill.warn {
    background-color: var(--amber);
    box-shadow: 0 0 9px -1px var(--amber);
  }
  .plan-fill.crit {
    background-color: var(--coral);
    box-shadow: 0 0 9px -1px var(--coral);
  }
  .plan-val {
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 600;
    color: var(--ink-dim);
    width: 46px;
    text-align: right;
  }

  .panel-foot {
    padding: 12px;
    border-top: 1px solid var(--border-soft);
    background: var(--bg-inset);
  }
  .dash-btn {
    width: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 9px;
    padding: 11px;
    border-radius: var(--r-md);
    background: var(--ink);
    color: var(--bg-surface);
    border: none;
    cursor: pointer;
    font-family: var(--font-round);
    font-size: 13.5px;
    font-weight: 600;
    letter-spacing: -0.01em;
    transition: transform 0.15s var(--ease), opacity 0.2s var(--ease);
  }
  .dash-btn:hover {
    transform: translateY(-1px);
    opacity: 0.92;
  }
  .dash-btn .host {
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 500;
    opacity: 0.6;
    letter-spacing: 0;
  }

  .empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    text-align: center;
    padding: 46px 20px;
    color: var(--ink-dim);
  }
  .empty-glyph {
    width: 46px;
    height: 51px;
    display: grid;
    place-items: center;
    font-family: var(--font-mono);
    font-weight: 700;
    font-size: 18px;
    color: var(--sage);
    background: color-mix(in srgb, var(--sage) 20%, var(--bg-raised));
    clip-path: polygon(50% 0%, 100% 25%, 100% 75%, 50% 100%, 0% 75%, 0% 25%);
    margin-bottom: 6px;
  }
  .empty p {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--ink);
  }
  .empty span {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-faint);
  }
</style>
