<script lang="ts">
  import { onMount } from "svelte";
  import { store } from "./state.svelte";
  import { pct, planTier } from "./format";
  import { prefs, byMode } from "./prefs.svelte";
  import { hookStatus, setHookEnabled, resolvePermission } from "./bridge";
  import type { HookStatus, PermDecision, PendingRequest, SessionView } from "./types";

  type Card = {
    key: string;
    project: string;
    path: string;
    branch: string | null;
    session: SessionView | null;
    requests: PendingRequest[];
  };

  let snap = $derived(store.snap);
  let tier = $derived(planTier(snap.planTier));

  // One card per session, its context gauge unified with any pending requests
  // it owns. Needs-input sessions sort first, then the panel's chosen sort.
  let cards = $derived.by(() => {
    const byId = new Map<string, Card>();
    for (const s of snap.sessions) {
      byId.set(s.id, { key: s.id, project: s.project, path: s.path, branch: s.branch, session: s, requests: [] });
    }
    for (const p of snap.pending) {
      let c = byId.get(p.sessionId);
      if (!c) {
        c = { key: p.sessionId, project: p.project, path: p.path, branch: null, session: null, requests: [] };
        byId.set(p.sessionId, c);
      }
      c.requests.push(p);
    }
    const cmp = byMode(prefs.sort);
    return [...byId.values()].sort((a, b) => {
      const an = a.requests.length ? 0 : 1;
      const bn = b.requests.length ? 0 : 1;
      if (an !== bn) return an - bn;
      if (a.session && b.session) return cmp(a.session, b.session);
      return a.session ? -1 : b.session ? 1 : 0;
    });
  });

  // ---- Phase 4: approvals hook status ----
  let hook = $state<HookStatus | null>(null);
  let busy = $state(false);

  onMount(() => {
    hookStatus().then((s) => (hook = s)).catch(() => {});
  });

  async function toggleApprovals() {
    if (busy) return;
    busy = true;
    try {
      hook = await setHookEnabled(!(hook?.installed ?? false));
    } catch {
      /* leave prior status */
    } finally {
      busy = false;
    }
  }

  function decide(id: string, decision: PermDecision) {
    resolvePermission(id, decision);
  }

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
    <div class="head-meta">
      {#if hook}
        <button
          class="approvals-toggle"
          class:on={hook.installed}
          disabled={busy}
          title={hook.serverUp
            ? `Intercepts ${hook.tools.join(", ")} on :${hook.port}`
            : "Approval server not running"}
          onclick={toggleApprovals}
        >
          <span class="dot"></span>
          approvals {hook.installed ? "on" : "off"}
        </button>
      {/if}
      {#if tier}
        <div class="plan-chip" title="Subscription tier (live quota isn't exposed locally)">
          {tier} <span>plan</span>
        </div>
      {/if}
      <div class="live">live</div>
    </div>
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
      <h3>Sessions <span class="n">{cards.length}</span></h3>
      {#if cards.length === 0}
        <div class="dash-empty">Nothing running. Tablo is watching.</div>
      {:else}
        {#each cards as c (c.key)}
          <div class="scard" class:needs={c.requests.length > 0}>
            <div class="drow">
              <span class="st {c.requests.length ? 'ask' : 'run'}"></span>
              <div class="info">
                <div class="p">{c.project}</div>
                <div class="path">{c.path}{c.branch ? ` · ${c.branch}` : ""}</div>
              </div>
              {#if c.session}
                <div class="gauge">
                  <div class="lab">
                    <span>context</span>
                    <span class="val" class:warn={c.session.level === "warn"} class:crit={c.session.level === "crit"}>
                      {pct(c.session.pct)}
                    </span>
                  </div>
                  <div class="tk"><i class={c.session.level} style="width:{c.session.pct}%"></i></div>
                </div>
              {/if}
            </div>
            {#each c.requests as p (p.id)}
              <div class="arow">
                <div class="ainfo">
                  <div class="atool"><span class="tag">{p.tool}</span></div>
                  {#if p.detail}<div class="adetail">{p.detail}</div>{/if}
                </div>
                <div class="aacts">
                  <button class="act deny" onclick={() => decide(p.id, "deny")}>Deny</button>
                  <button class="act allow" onclick={() => decide(p.id, "allow")}>Approve</button>
                </div>
              </div>
            {/each}
          </div>
        {/each}
      {/if}
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
  .head-meta {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .plan-chip {
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    color: var(--amber);
    background: var(--amber-soft);
    border: 1px solid color-mix(in srgb, var(--amber) 30%, transparent);
    border-radius: 999px;
    padding: 4px 11px;
    white-space: nowrap;
  }
  .plan-chip span {
    color: var(--ink-faint);
    font-weight: 500;
  }
  .approvals-toggle {
    display: flex;
    align-items: center;
    gap: 7px;
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    color: var(--ink-faint);
    background: var(--bg-raised);
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 4px 11px;
    cursor: pointer;
    transition: color 0.18s var(--ease), border-color 0.18s var(--ease);
  }
  .approvals-toggle:hover {
    color: var(--ink-dim);
  }
  .approvals-toggle:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .approvals-toggle .dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    background: var(--ink-faint);
  }
  .approvals-toggle.on {
    color: var(--sage);
    border-color: color-mix(in srgb, var(--sage) 40%, var(--border));
  }
  .approvals-toggle.on .dot {
    background: var(--sage);
    box-shadow: 0 0 7px var(--sage);
  }

  /* one session block: its context row + any pending requests, unified */
  .scard {
    padding: 11px 0;
    border-bottom: 1px solid var(--border-soft);
  }
  .scard:last-child {
    border-bottom: none;
  }
  .scard.needs {
    border: 1px solid color-mix(in srgb, var(--coral) 38%, var(--border-soft));
    background: color-mix(in srgb, var(--coral) 6%, transparent);
    border-radius: var(--r-md);
    padding: 6px 12px 8px;
    margin: 8px 0;
  }
  .arow {
    display: flex;
    align-items: flex-start;
    gap: 14px;
    padding: 10px 0 2px;
    margin-top: 8px;
    border-top: 1px solid color-mix(in srgb, var(--coral) 15%, var(--border-soft));
  }
  .ainfo {
    flex: 1;
    min-width: 0;
  }
  .atool {
    font-size: 13px;
    font-weight: 600;
  }
  .atool .tag {
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 600;
    color: var(--coral);
    background: var(--coral-soft);
    padding: 2px 6px;
    border-radius: 5px;
  }
  .adetail {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-dim);
    background: var(--bg-inset);
    border: 1px solid var(--border-soft);
    border-radius: var(--r-sm);
    padding: 7px 9px;
    margin: 6px 0 5px;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 90px;
    overflow-y: auto;
  }
  .aacts {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }
  .aacts .act {
    padding: 7px 16px;
    border-radius: var(--r-sm);
    font-family: var(--font-round);
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    border: 1px solid transparent;
    transition: transform 0.12s var(--ease);
  }
  .aacts .act:hover {
    transform: translateY(-1px);
  }
  .aacts .deny {
    background: transparent;
    border-color: color-mix(in srgb, var(--coral) 45%, var(--border));
    color: var(--coral);
  }
  .aacts .allow {
    background: var(--sage);
    color: var(--bg-surface);
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
    grid-template-columns: 1fr;
    gap: 18px;
  }
  @media (max-width: 720px) {
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
    padding: 3px 0;
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

  .dash-empty {
    padding: 24px 0;
    text-align: center;
    color: var(--ink-faint);
    font-family: var(--font-mono);
    font-size: 12px;
  }
</style>
