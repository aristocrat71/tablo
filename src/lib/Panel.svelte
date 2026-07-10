<script lang="ts">
  import { onMount } from "svelte";
  import { store } from "./state.svelte";
  import { openDashboard, resolvePermission, jumpToSession, hidePanel } from "./bridge";
  import { tokens, pct } from "./format";

  // Esc collapses the panel (matches tap-away). The webview persists across
  // show/hide, so binding once on mount covers every open.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") hidePanel().catch(() => {});
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });
  import { prefs, setSort, byMode } from "./prefs.svelte";
  import FilterButton from "./FilterButton.svelte";
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

  // Source filter — only when sessions span >1 agent; orphan requests are Claude-side.
  let sources = $derived(new Set(snap.sessions.map((s) => s.source)));
  let sourceFilterActive = $derived(sources.size > 1);
  const srcVisible = (c: Card) =>
    !sourceFilterActive || ((c.session?.source ?? "claude") === "codex" ? prefs.showCodex : prefs.showClaude);

  // Sessions past the context warn threshold get pulled into a Critical group,
  // always pinned to the top, ahead of every other state.
  const isCrit = (c: Card) => !!c.session && c.session.level !== "ok";
  let critCards = $derived(cards.filter((c) => isCrit(c) && srcVisible(c)));
  let needsCards = $derived(cards.filter((c) => !isCrit(c) && c.requests.length > 0 && srcVisible(c)));
  // Split the remaining non-request sessions: agents waiting on the user get
  // their own green-LED group, distinct from the ones actively working.
  let waitingCards = $derived(
    cards.filter(
      (c) => !isCrit(c) && c.requests.length === 0 && c.session?.activityKind === "waiting" && srcVisible(c)
    )
  );
  let workCards = $derived(
    cards.filter(
      (c) => !isCrit(c) && c.requests.length === 0 && c.session?.activityKind !== "waiting" && srcVisible(c)
    )
  );
  // State filters only apply while the toolbar is visible (>1 session), so a
  // lone session can never be filtered out with no way to bring it back.
  let filtersActive = $derived(snap.sessions.length > 1);
  let showWork = $derived(!filtersActive || prefs.showWorking);
  let showWait = $derived(!filtersActive || prefs.showWaiting);
  // Everything filtered away (e.g. both toggles off) with no permission request
  // forcing itself in — show a friendly placeholder instead of blank space.
  let nothingVisible = $derived(
    critCards.length === 0 &&
      needsCards.length === 0 &&
      !(showWait && waitingCards.length) &&
      !(showWork && workCards.length)
  );

  let sub = $derived.by(() => {
    if (!snap.hasProjectsDir) return "no agents have run yet";
    const n = snap.sessions.length;
    if (n === 0 && pending.length === 0) return "no active sessions";
    const base = n > 0 ? `${n} session${n > 1 ? "s" : ""}` : "idle";
    return snap.waiting > 0 ? `${base} · ${snap.waiting} waiting on you` : base;
  });

  // Warn threshold (%), shown in the Critical group header (live from the snapshot).
  let warnPct = $derived(Math.round(snap.warnPct));

  function decide(id: string, decision: PermDecision) {
    resolvePermission(id, decision);
  }

  function jump(sessionId: string) {
    jumpToSession(sessionId).catch(() => {});
  }
</script>

<div class="panel-shell">
  <div class="panel">
    <div class="panel-top">
      <img class="panel-glyph" src="/tablo-logo-v4.png" alt="" />
      <div class="panel-titles">
        <div class="name">tablo</div>
        <div class="sub">{sub}</div>
      </div>
      <div class="panel-actions">
        <button class="dash-link" title="Open dashboard" onclick={() => openDashboard()}>
          dashboard <span class="arr">↗</span>
        </button>
      </div>
    </div>

    {#if snap.sessions.length > 1}
      <div class="panel-toolbar">
        <div class="seg" role="group" aria-label="Sort sessions">
          <button class:on={prefs.sort === "context"} onclick={() => setSort("context")}>context</button>
          <button class:on={prefs.sort === "recent"} onclick={() => setSort("recent")}>recent</button>
        </div>
        <FilterButton showSource={sourceFilterActive} />
      </div>
    {/if}

    <div class="panel-body">
      {#if cards.length === 0}
        <div class="empty">
          <div class="empty-glyph">~_~</div>
          <p>{snap.hasProjectsDir ? "Nothing running right now." : "No agent sessions found yet."}</p>
          <span>Tablo wakes up when an agent starts working.</span>
        </div>
      {:else}
        {#if critCards.length}
          <div class="group-head crit-head">
            <span class="group-dot attn"></span>
            <span class="group-name">Context window warning ! &gt;{warnPct}%</span>
            <span class="group-count">{critCards.length}</span>
          </div>
          {#each critCards as c (c.key)}
            {@render sessionCard(c)}
          {/each}
        {/if}

        {#if needsCards.length}
          <div class="group-head">
            <span class="group-dot attn"></span>
            <span class="group-name">Permission Request</span>
            <span class="group-count">{needsCards.length}</span>
          </div>
          {#each needsCards as c (c.key)}
            {@render sessionCard(c)}
          {/each}
        {/if}

        {#if waitingCards.length && showWait}
          <div class="group-head">
            <span class="group-dot wait"></span>
            <span class="group-name">Waiting</span>
            <span class="group-count">{waitingCards.length}</span>
          </div>
          {#each waitingCards as c (c.key)}
            {@render sessionCard(c)}
          {/each}
        {/if}

        {#if workCards.length && showWork}
          <div class="group-head">
            <span class="group-dot work"></span>
            <span class="group-name">Working</span>
            <span class="group-count">{workCards.length}</span>
          </div>
          {#each workCards as c (c.key)}
            {@render sessionCard(c)}
          {/each}
        {/if}

        {#if nothingVisible}
          <div class="filtered-empty">
            <div class="huh">Huh ~_~ ?</div>
            <span>Everything's filtered out.</span>
          </div>
        {/if}
      {/if}
    </div>
  </div>
</div>

{#snippet sessionCard(c: Card)}
  <div class="ucard" class:needs={c.requests.length > 0} class:crit={!!c.session && c.session.level !== "ok"}>
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

    {#if c.session}
      <div class="card-foot">
        {#if c.session.canJump}
          <button class="jump" title="Focus this session's window" onclick={() => jump(c.session!.id)}>
            jump &rarr;
          </button>
        {/if}
        <span class="src-tag">{c.session.source}</span>
        <span class="mode-badge">mode : <span class="mode-val {c.session.mode}">{c.session.mode}</span></span>
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
  /* critical group header — the context-window warning line, red */
  .crit-head .group-name {
    color: var(--coral);
  }
  .panel-top {
    display: flex;
    align-items: center;
    gap: 11px;
    padding: 16px 18px 14px;
    border-bottom: 1px solid var(--border-soft);
  }
  .panel-glyph {
    height: 36px;
    width: auto; /* keep the logo's own (built-in-background) aspect — no crop */
    border-radius: 7px;
    flex: none;
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
    display: inline-flex;
    align-items: center;
    gap: 5px;
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
  .group-dot.wait {
    background: var(--sage);
    box-shadow: 0 0 8px var(--sage);
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
  /* a session with pending approvals, or past the context warn line — coral card */
  .ucard.needs,
  .ucard.crit {
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
  /* agent source tag (e.g. codex) — neutral so it never reads as a state color */
  .src-tag {
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: 8.5px;
    font-weight: 700;
    letter-spacing: 0.07em;
    text-transform: uppercase;
    padding: 2px 5px;
    border-radius: 4px;
    background: var(--bg-inset);
    color: var(--ink-faint);
    border: 1px solid var(--border-soft);
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
  /* card footer: jump pinned left, read-only mode badge pinned right */
  .card-foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  /* source tag + mode badge sit together, pinned to the right end (even when the
     jump button is absent) — the tag leads, the mode display follows */
  .card-foot .src-tag {
    margin-left: auto;
  }
  .mode-badge {
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.02em;
    color: var(--ink-faint);
    white-space: nowrap;
  }
  .mode-val {
    font-weight: 600;
    color: var(--ink-dim);
  }
  .mode-val.auto {
    color: var(--amber);
  }
  .mode-val.plan {
    color: var(--sage);
  }
  .mode-val.bypass {
    color: var(--coral);
  }
  .jump {
    font-family: var(--font-mono);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--amber);
    background: var(--amber-soft);
    border: 1px solid color-mix(in srgb, var(--amber) 32%, transparent);
    border-radius: 6px;
    padding: 3px 8px;
    cursor: pointer;
    white-space: nowrap;
    transition:
      background-color 0.15s var(--ease),
      transform 0.12s var(--ease);
  }
  .jump:hover {
    transform: translateY(-1px);
    background: color-mix(in srgb, var(--amber) 24%, var(--amber-soft));
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
    border: 1px solid var(--border);
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

  /* shown when every group is filtered out (e.g. both toggles off) */
  .filtered-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    text-align: center;
    padding: 44px 20px;
  }
  .filtered-empty .huh {
    font-family: var(--font-mono);
    font-size: 20px;
    font-weight: 700;
    letter-spacing: 0.06em;
    color: var(--ink-dim);
  }
  .filtered-empty span {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-faint);
  }
  .empty-glyph {
    font-family: var(--font-mono);
    font-weight: 700;
    font-size: 22px;
    letter-spacing: 0.06em;
    color: var(--sage);
    margin-bottom: 8px;
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
