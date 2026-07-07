<script lang="ts">
  import { store, applyTheme } from "./state.svelte";
  import { setTheme, openDashboard, resolvePermission } from "./bridge";
  import { tokens, pct, activitySuffix } from "./format";
  import { prefs, setSort, setPanelMode, byMode } from "./prefs.svelte";
  import type { SessionView, PermDecision, PendingRequest } from "./types";

  // One card per session, unifying its working/context state with any pending
  // tool approvals it owns — so it's obvious which session is asking.
  type Card = {
    key: string;
    project: string;
    path: string;
    branch: string | null;
    session: SessionView | null; // null = orphan request (no active session)
    requests: PendingRequest[];
  };

  let snap = $derived(store.snap);
  let pending = $derived(snap.pending);

  let cards = $derived.by(() => {
    const byId = new Map<string, Card>();
    // Seed from active sessions — their resolved project/path/branch win over a
    // request's raw shell-cwd basename.
    for (const s of snap.sessions) {
      byId.set(s.id, { key: s.id, project: s.project, path: s.path, branch: s.branch, session: s, requests: [] });
    }
    // Attach each request to its session by id; an orphan gets a minimal card.
    for (const p of pending) {
      let c = byId.get(p.sessionId);
      if (!c) {
        c = { key: p.sessionId, project: p.project, path: p.path, branch: null, session: null, requests: [] };
        byId.set(p.sessionId, c);
      }
      c.requests.push(p);
    }
    // Needs-input sessions first, then the chosen sort among the rest.
    const cmp = byMode(prefs.sort);
    return [...byId.values()].sort((a, b) => {
      const an = a.requests.length ? 0 : 1;
      const bn = b.requests.length ? 0 : 1;
      if (an !== bn) return an - bn;
      if (a.session && b.session) return cmp(a.session, b.session);
      return a.session ? -1 : b.session ? 1 : 0;
    });
  });

  let needsCards = $derived(cards.filter((c) => c.requests.length > 0));
  let workCards = $derived(cards.filter((c) => c.requests.length === 0));
  // Compact collapses only the purely-working sessions; requests never hide.
  let compact = $derived(prefs.panelMode === "compact" && workCards.length > 1);

  let sub = $derived.by(() => {
    if (!snap.hasProjectsDir) return "Claude Code hasn't run yet";
    const n = snap.sessions.length;
    if (n === 0 && pending.length === 0) return "no active sessions";
    const base = n > 0 ? `${n} session${n > 1 ? "s" : ""}` : "idle";
    return snap.waiting > 0 ? `${base} · ${snap.waiting} waiting on you` : base;
  });

  function decide(id: string, decision: PermDecision) {
    resolvePermission(id, decision);
  }

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
      <div class="panel-actions">
        <button class="dash-link" title="Open dashboard" onclick={() => openDashboard()}>
          dashboard <span class="arr">↗</span>
        </button>
        <button class="mini" title="Toggle theme" onclick={toggleTheme} aria-label="Toggle theme">☾</button>
      </div>
    </div>

    {#if snap.sessions.length > 1}
      <div class="panel-toolbar">
        <div class="seg" role="group" aria-label="Sort sessions">
          <button class:on={prefs.sort === "context"} onclick={() => setSort("context")}>context</button>
          <button class:on={prefs.sort === "recent"} onclick={() => setSort("recent")}>recent</button>
        </div>
        <div class="seg" role="group" aria-label="View mode">
          <button class:on={!compact} onclick={() => setPanelMode("expanded")}>list</button>
          <button class:on={compact} onclick={() => setPanelMode("compact")}>compact</button>
        </div>
      </div>
    {/if}

    <div class="panel-body">
      {#if cards.length === 0}
        <div class="empty">
          <div class="empty-glyph">I</div>
          <p>{snap.hasProjectsDir ? "Nothing running right now." : "No Claude Code sessions found yet."}</p>
          <span>Tablo wakes up when an agent starts working.</span>
        </div>
      {:else}
        {#if needsCards.length}
          <div class="group-head">
            <span class="group-dot attn"></span>
            <span class="group-name">Input requested</span>
            <span class="group-count">{pending.length}</span>
          </div>
          {#each needsCards as c (c.key)}
            {@render sessionCard(c)}
          {/each}
        {/if}

        {#if workCards.length}
          <div class="group-head">
            <span class="group-dot work"></span>
            <span class="group-name">Working</span>
            <span class="group-count">{workCards.length}</span>
          </div>
          {#if compact}
            {@render sessionCard(workCards[0])}
            <button class="more" onclick={() => setPanelMode("expanded")}>
              +{workCards.length - 1} more session{workCards.length - 1 > 1 ? "s" : ""}
            </button>
          {:else}
            {#each workCards as c (c.key)}
              {@render sessionCard(c)}
            {/each}
          {/if}
        {/if}
      {/if}
    </div>
  </div>
</div>

{#snippet sessionCard(c: Card)}
  <div class="ucard" class:needs={c.requests.length > 0}>
    <div class="session-line1">
      <span class="session-proj">{c.project}</span>
      {#if c.session?.title}
        <span class="session-sep">·</span>
        <span class="session-title">{c.session.title}</span>
      {/if}
      {#if c.session}
        <span
          class="session-pct"
          class:warn={c.session.level === "warn"}
          class:crit={c.session.level === "crit"}
        >
          {pct(c.session.pct)}
        </span>
      {:else}
        <span class="session-badge ask">WAITING</span>
      {/if}
    </div>
    <div class="session-path">{c.path}{c.branch ? ` · ${c.branch}` : ""}</div>

    {#if c.session?.activity}
      <div class="session-activity {c.session.activityKind}">
        <span class="act-dot"></span>
        <span class="act-text">{c.session.activity}</span>
        {#if activitySuffix(c.session.activityKind)}
          <span class="act-suffix">· {activitySuffix(c.session.activityKind)}</span>
        {/if}
      </div>
    {/if}

    {#if c.session}
      <div class="ctx">
        <div class="ctx-track">
          <div class="ctx-fill {c.session.level}" style="width:{c.session.pct}%"></div>
        </div>
        <span class="ctx-cap">{tokens(c.session.used)} / {tokens(c.session.limit)}</span>
      </div>
    {/if}

    {#each c.requests as p (p.id)}
      <div class="req">
        <div class="req-top">
          <span class="session-badge ask">{p.tool}</span>
          <div class="approval-actions">
            <button class="act deny" onclick={() => decide(p.id, "deny")}>Deny</button>
            <button class="act allow" onclick={() => decide(p.id, "allow")}>Approve</button>
          </div>
        </div>
        {#if p.detail}<div class="approval-detail">{p.detail}</div>{/if}
      </div>
    {/each}
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
  .panel-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .dash-link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    padding: 0 11px;
    border-radius: 8px;
    border: none;
    background: var(--ink);
    color: var(--bg-surface);
    cursor: pointer;
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.01em;
    white-space: nowrap;
    transition: transform 0.15s var(--ease), opacity 0.2s var(--ease);
  }
  .dash-link:hover {
    transform: translateY(-1px);
    opacity: 0.92;
  }
  .dash-link .arr {
    font-size: 12.5px;
    line-height: 1;
  }

  .panel-toolbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    padding: 9px 18px;
    border-bottom: 1px solid var(--border-soft);
    background: var(--bg-inset);
  }
  .seg {
    display: inline-flex;
    padding: 2px;
    border-radius: 8px;
    background: var(--bg-surface);
    border: 1px solid var(--border-soft);
  }
  .seg button {
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.02em;
    padding: 4px 9px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--ink-faint);
    cursor: pointer;
    transition: color 0.18s var(--ease), background-color 0.18s var(--ease);
  }
  .seg button:hover {
    color: var(--ink-dim);
  }
  .seg button.on {
    background: var(--amber-soft);
    color: var(--amber);
  }

  .more {
    width: 100%;
    margin-top: 4px;
    padding: 10px;
    border-radius: var(--r-md);
    border: 1px dashed var(--border);
    background: transparent;
    color: var(--ink-dim);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.03em;
    cursor: pointer;
    transition: color 0.18s var(--ease), border-color 0.18s var(--ease);
  }
  .more:hover {
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

  .ucard {
    display: flex;
    flex-direction: column;
    gap: 11px;
    padding: 15px 14px;
    border-radius: var(--r-md);
    background: var(--bg-raised);
    border: 1px solid var(--border-soft);
    margin-bottom: 10px;
    transition: border-color 0.2s var(--ease);
  }
  .ucard:hover {
    border-color: var(--border);
  }
  /* a session with pending approvals — coral card, sorted to the top */
  .ucard.needs {
    background: color-mix(in srgb, var(--coral) 8%, var(--bg-raised));
    border-color: color-mix(in srgb, var(--coral) 42%, var(--border-soft));
  }
  /* one pending request inside a session card, divided from what's above */
  .req {
    display: flex;
    flex-direction: column;
    gap: 7px;
    padding-top: 9px;
    border-top: 1px solid color-mix(in srgb, var(--coral) 18%, var(--border-soft));
  }
  .req-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .approval-detail {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-dim);
    background: var(--bg-inset);
    border: 1px solid var(--border-soft);
    border-radius: var(--r-sm);
    padding: 7px 9px;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 84px;
    overflow-y: auto;
  }
  .approval-actions {
    display: flex;
    gap: 7px;
    margin-left: auto;
  }
  .act {
    padding: 6px 14px;
    border-radius: var(--r-sm);
    font-family: var(--font-round);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    border: 1px solid transparent;
    transition: transform 0.12s var(--ease), opacity 0.2s var(--ease);
  }
  .act:hover {
    transform: translateY(-1px);
  }
  .act.deny {
    background: transparent;
    border-color: color-mix(in srgb, var(--coral) 45%, var(--border));
    color: var(--coral);
  }
  .act.allow {
    background: var(--sage);
    color: var(--bg-surface);
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
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .session-sep {
    color: var(--ink-faint);
    flex-shrink: 0;
  }
  /* aiTitle — grows to fill, ellipsizes, so RUN + % stay pinned right */
  .session-title {
    flex: 1;
    min-width: 0;
    font-size: 12px;
    color: var(--ink-dim);
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

  /* live activity preview (window-render) */
  .session-activity {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 3px;
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-dim);
    white-space: nowrap;
    overflow: hidden;
  }
  /* LED status dot, colored per kind (no glyph/emoji) */
  .session-activity .act-dot {
    flex-shrink: 0;
    width: 6px;
    height: 6px;
    border-radius: 999px;
    background: var(--ink-faint);
  }
  .session-activity .act-text {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .session-activity .act-suffix {
    flex-shrink: 0;
    color: var(--ink-faint);
  }
  /* working: amber dot, gently pulsing so it reads as live */
  .session-activity.working {
    color: var(--ink);
  }
  .session-activity.working .act-dot {
    background: var(--amber);
    box-shadow: 0 0 7px var(--amber);
    animation: act-pulse 1.6s var(--ease) infinite;
  }
  /* waiting for you: calm sage dot */
  .session-activity.waiting .act-dot {
    background: var(--sage);
    box-shadow: 0 0 6px color-mix(in srgb, var(--sage) 70%, transparent);
  }
  .session-activity.thinking .act-dot {
    background: var(--ink-faint);
  }
  @keyframes act-pulse {
    0%,
    100% {
      opacity: 0.45;
    }
    50% {
      opacity: 1;
    }
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
