<script lang="ts">
  import { store } from "./state.svelte";
  import { pct, tokens } from "./format";

  let snap = $derived(store.snap);
  let lead = $derived.by(() => {
    if (!snap.hasProjectsDir) return "No Claude Code sessions found yet";
    const n = snap.sessions.length;
    if (n === 0) return "No active sessions right now";
    return `Watching ${n} Claude Code session${n > 1 ? "s" : ""} across ${snap.projects} project${snap.projects > 1 ? "s" : ""}`;
  });
</script>

<div class="dash">
  <div class="dash-head">
    <div class="lead">
      <h2><span class="g">a</span> tablo</h2>
      <p>{lead}</p>
    </div>
    <div class="live">live</div>
  </div>

  <div class="stats">
    <div class="stat"><div class="v">{snap.agentCount}</div><div class="k">Active</div></div>
    <div class="stat">
      <div class="v" style={snap.waiting ? "color:var(--coral)" : ""}>{snap.waiting}</div>
      <div class="k">Waiting</div>
    </div>
    <div class="stat"><div class="v">{snap.projects}</div><div class="k">Projects</div></div>
  </div>

  <div class="dash-grid">
    <div class="card">
      <h3>Sessions <span class="n">{snap.sessions.length}</span></h3>
      {#if snap.sessions.length === 0}
        <div class="dash-empty">Nothing running. Tablo is watching.</div>
      {:else}
        {#each snap.sessions as s (s.id)}
          <div class="drow">
            <span class="st {s.state === 'ask' ? 'ask' : 'run'}"></span>
            <div class="info">
              <div class="p">{s.project}</div>
              <div class="path">{s.path}{s.branch ? ` · ${s.branch}` : ""}</div>
            </div>
            <div class="gauge">
              <div class="lab">
                <span>context</span>
                <span class="val" class:warn={s.level === "warn"} class:crit={s.level === "crit"}>
                  {pct(s.pct)}
                </span>
              </div>
              <div class="tk"><i class={s.level} style="width:{s.pct}%"></i></div>
            </div>
          </div>
        {/each}
      {/if}
    </div>

    <div class="card">
      <h3>Plan usage <span class="n">tokens</span></h3>
      <div class="plan-vert">
        <div class="vbar">
          <div class="vbar-track">
            <div class="vbar-fill {snap.plan.fiveHourLevel}" style="height:{snap.plan.fiveHourPct}%"></div>
          </div>
          <div class="vbar-cap {snap.plan.fiveHourLevel}">{pct(snap.plan.fiveHourPct)}</div>
        </div>
        <div class="plan-meta">
          <div class="row"><span>5h window</span><b>{tokens(snap.plan.fiveHourTokens)}</b></div>
          <div class="row"><span>5h of budget</span><b>{pct(snap.plan.fiveHourPct)}</b></div>
          <div class="row"><span>7 days</span><b>{tokens(snap.plan.weekTokens)}</b></div>
          <div class="row"><span>7d of budget</span><b>{pct(snap.plan.weekPct)}</b></div>
        </div>
      </div>
      <div class="plan-note">
        Live token rollup from transcripts — a proxy, not Anthropic's exact quota.
      </div>
    </div>
  </div>
</div>

<style>
  .dash {
    padding: 26px 28px 30px;
    max-width: 940px;
    margin: 0 auto;
  }
  .dash-head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    gap: 16px;
    margin-bottom: 24px;
    flex-wrap: wrap;
  }
  .lead h2 {
    font-size: 26px;
    font-weight: 700;
    letter-spacing: -0.025em;
    display: flex;
    align-items: center;
    gap: 11px;
  }
  .lead h2 .g {
    width: 30px;
    height: 34px;
    background: var(--amber-soft);
    color: var(--amber);
    display: grid;
    place-items: center;
    font-family: var(--font-mono);
    font-size: 14px;
    font-weight: 700;
    clip-path: polygon(50% 0%, 100% 25%, 100% 75%, 50% 100%, 0% 75%, 0% 25%);
  }
  .lead p {
    font-size: 13px;
    color: var(--ink-dim);
    margin-top: 5px;
    font-weight: 500;
  }
  .live {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--sage);
    display: flex;
    align-items: center;
    gap: 7px;
    font-weight: 600;
  }
  .live::before {
    content: "";
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--sage);
    box-shadow: 0 0 8px var(--sage);
    animation: pulse 2s infinite;
  }
  @keyframes pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.4;
    }
  }

  .stats {
    display: flex;
    gap: 10px;
    margin-bottom: 20px;
  }
  .stat {
    flex: 1;
    display: flex;
    align-items: baseline;
    gap: 8px;
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: var(--r-md);
    padding: 10px 14px;
  }
  .stat .v {
    font-size: 22px;
    font-weight: 700;
    letter-spacing: -0.03em;
    line-height: 1;
  }
  .stat .k {
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--ink-faint);
    text-transform: uppercase;
    letter-spacing: 0.06em;
  }

  .dash-grid {
    display: grid;
    grid-template-columns: 1.55fr 1fr;
    gap: 18px;
  }
  @media (max-width: 720px) {
    .dash-grid {
      grid-template-columns: 1fr;
    }
    .stats {
      flex-wrap: wrap;
    }
  }

  .card {
    background: var(--bg-raised);
    border: 1px solid var(--border-soft);
    border-radius: var(--r-md);
    padding: 18px;
  }
  .card h3 {
    font-size: 12px;
    font-family: var(--font-mono);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--ink-dim);
    margin-bottom: 16px;
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .card h3 .n {
    color: var(--ink-faint);
    margin-left: auto;
  }

  .drow {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 11px 0;
    border-bottom: 1px solid var(--border-soft);
  }
  .drow:last-child {
    border-bottom: none;
  }
  .drow .st {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .drow .st.run {
    background: var(--sage);
    box-shadow: 0 0 7px var(--sage);
  }
  .drow .st.ask {
    background: var(--coral);
    box-shadow: 0 0 7px var(--coral);
  }
  .drow .info {
    flex: 1;
    min-width: 0;
  }
  .drow .info .p {
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .drow .info .path {
    font-family: var(--font-mono);
    font-size: 10px;
    color: var(--ink-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .drow .gauge {
    width: 108px;
    flex-shrink: 0;
  }
  .drow .gauge .lab {
    display: flex;
    justify-content: space-between;
    font-family: var(--font-mono);
    font-size: 9.5px;
    color: var(--ink-faint);
    margin-bottom: 4px;
  }
  .drow .gauge .lab .val {
    font-weight: 600;
  }
  .drow .gauge .lab .val.warn {
    color: var(--amber);
  }
  .drow .gauge .lab .val.crit {
    color: var(--coral);
  }
  .drow .gauge .tk {
    height: 7px;
    border-radius: 2px;
    background: var(--bg-inset);
    overflow: hidden;
    background-image: repeating-linear-gradient(
      90deg,
      transparent 0 4px,
      color-mix(in srgb, var(--bg-surface) 75%, black) 4px 5px
    );
  }
  .drow .gauge .tk i {
    display: block;
    height: 100%;
    transition: width 0.5s var(--ease);
    background-image: repeating-linear-gradient(90deg, transparent 0 4px, rgba(0, 0, 0, 0.25) 4px 5px);
  }
  .drow .gauge .tk i.ok {
    background-color: var(--sage);
    box-shadow: 0 0 7px -1px var(--sage);
  }
  .drow .gauge .tk i.warn {
    background-color: var(--amber);
    box-shadow: 0 0 8px -1px var(--amber);
  }
  .drow .gauge .tk i.crit {
    background-color: var(--coral);
    box-shadow: 0 0 8px -1px var(--coral);
  }

  .plan-vert {
    display: flex;
    align-items: stretch;
    gap: 18px;
  }
  .vbar {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
  .vbar-track {
    width: 22px;
    height: 96px;
    border-radius: 4px;
    background: var(--bg-inset);
    border: 1px solid var(--border-soft);
    overflow: hidden;
    display: flex;
    flex-direction: column-reverse;
    background-image: repeating-linear-gradient(
      0deg,
      transparent 0 6px,
      color-mix(in srgb, var(--bg-surface) 75%, black) 6px 7px
    );
  }
  .vbar-fill {
    width: 100%;
    background-color: var(--sage);
    box-shadow: 0 0 9px -1px var(--sage);
    background-image: repeating-linear-gradient(0deg, transparent 0 6px, rgba(0, 0, 0, 0.25) 6px 7px);
    transition: height 0.5s var(--ease), background-color 0.4s var(--ease);
  }
  .vbar-fill.warn {
    background-color: var(--amber);
    box-shadow: 0 0 9px -1px var(--amber);
  }
  .vbar-fill.crit {
    background-color: var(--coral);
    box-shadow: 0 0 9px -1px var(--coral);
  }
  .vbar-cap {
    font-family: var(--font-mono);
    font-size: 13px;
    font-weight: 700;
    color: var(--sage);
    text-shadow: 0 0 12px color-mix(in srgb, var(--sage) 45%, transparent);
  }
  .vbar-cap.warn {
    color: var(--amber);
    text-shadow: 0 0 12px color-mix(in srgb, var(--amber) 45%, transparent);
  }
  .vbar-cap.crit {
    color: var(--coral);
    text-shadow: 0 0 12px color-mix(in srgb, var(--coral) 45%, transparent);
  }
  .plan-meta {
    font-size: 12px;
    flex: 1;
    align-self: center;
  }
  .plan-meta .row {
    display: flex;
    justify-content: space-between;
    padding: 6px 0;
    border-bottom: 1px solid var(--border-soft);
  }
  .plan-meta .row:last-child {
    border-bottom: none;
  }
  .plan-meta .row span {
    color: var(--ink-dim);
  }
  .plan-meta .row b {
    font-family: var(--font-mono);
    font-weight: 600;
  }
  .plan-note {
    margin-top: 14px;
    font-family: var(--font-mono);
    font-size: 10.5px;
    color: var(--ink-faint);
    text-align: center;
  }
  .dash-empty {
    padding: 24px 0;
    text-align: center;
    color: var(--ink-faint);
    font-family: var(--font-mono);
    font-size: 12px;
  }
</style>
