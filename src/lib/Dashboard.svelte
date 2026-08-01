<script lang="ts">
  import { onMount } from "svelte";
  import { store } from "./state.svelte";
  import { fade } from "svelte/transition";
  import { pct, activityMark, terminalStatus } from "./format";
  import {
    prefs,
    byMode,
    toggleCollapse,
    setNotifyOnWaiting,
    setWaitingToastSecs,
    setNotifySound,
    setAnimations,
    TOAST_SECS_MIN,
    TOAST_SECS_MAX,
  } from "./prefs.svelte";
  import FilterButton from "./FilterButton.svelte";
  import {
    hookStatus,
    setHookEnabled,
    resolvePermission,
    locateStatus,
    setLocateEnabled,
    jumpToSession,
    hideDashboard,
    setWarnPct,
    setCancelGraceMins,
    setClearWaitingMins,
    setWatchCodex,
    setAerospaceFollow,
    setTelemetryEnabled,
    setAutoUpdate,
    installUpdate,
    dismissWhatsNew,
    setPanelShortcutEnabled,
    codexLocateStatus,
    setCodexLocateEnabled,
  } from "./bridge";
  import type { HookStatus, LocateStatus, PermDecision, PendingRequest, SessionView } from "./types";
  import ThemeToggle from "./ThemeToggle.svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { openUrl } from "@tauri-apps/plugin-opener";

  const REPO_URL = "https://github.com/unravel-team/tablo";
  const WEBSITE_URL = "https://tablo.unravel.tech/";
  const UNRAVEL_URL = "https://unravel.tech";

  type Card = {
    key: string;
    project: string;
    path: string;
    branch: string | null;
    session: SessionView | null;
    requests: PendingRequest[];
  };

  let snap = $derived(store.snap);

  // Header tabs: the sessions dashboard vs. the settings pane.
  let view = $state<"dashboard" | "settings">("dashboard");

  // About dialog — a modal popover, plus the app version (from tauri.conf.json)
  // shown inside it.
  let showAbout = $state(false);
  let version = $state("");

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

  // Source filter — mirrors the Panel; only when sessions span >1 agent.
  let sources = $derived(new Set(snap.sessions.map((s) => s.source)));
  let sourceFilterActive = $derived(sources.size > 1);
  const srcVisible = (c: Card) =>
    !sourceFilterActive || ((c.session?.source ?? "claude") === "codex" ? prefs.showCodex : prefs.showClaude);

  // Sessions past the context warn threshold get their own Critical card, pinned
  // above the rest.
  const isCrit = (c: Card) => !!c.session && c.session.level !== "ok";
  let critCards = $derived(cards.filter((c) => isCrit(c) && srcVisible(c)));
  let restCards = $derived(cards.filter((c) => !isCrit(c)));
  // Working/waiting state filters, mirroring the Panel. Only apply with >1 session
  // so a lone session can't be filtered out with no way to bring it back; a pending
  // permission request always shows, others gate by their state.
  let filtersActive = $derived(snap.sessions.length > 1);
  let showWork = $derived(!filtersActive || prefs.showWorking);
  let showWait = $derived(!filtersActive || prefs.showWaiting);
  // Panel-style groups: requests pin to top (never collapsed), then collapsible Waiting, then Working.
  let reqCards = $derived(restCards.filter((c) => srcVisible(c) && c.requests.length > 0));
  let waitCards = $derived(
    restCards.filter(
      (c) => srcVisible(c) && c.requests.length === 0 && c.session?.activityKind === "waiting" && showWait,
    ),
  );
  let workCards = $derived(
    restCards.filter(
      (c) => srcVisible(c) && c.requests.length === 0 && c.session?.activityKind !== "waiting" && showWork,
    ),
  );
  let visibleCount = $derived(reqCards.length + waitCards.length + workCards.length);
  let warnPct = $derived(Math.round(snap.warnPct));
  let cancelGraceMins = $derived(Math.round(snap.cancelGraceMins));
  let clearWaitingMins = $derived(Math.round(snap.clearWaitingMins));
  let watchCodex = $derived(snap.watchCodex);
  let panelShortcutEnabled = $derived(snap.panelShortcutEnabled);
  let aerospaceFollow = $derived(snap.aerospaceFollow);
  let telemetryEnabled = $derived(snap.telemetryEnabled);
  let autoUpdate = $derived(snap.autoUpdate);
  let whatsNew = $derived(snap.whatsNew);
  // Only set while auto-update is off and the background check found something.
  let updateAvailable = $derived(snap.updateAvailable);
  let aerospaceAvailable = $derived(snap.aerospaceAvailable);
  // Jump is macOS-only; approvals run everywhere but Windows. Backend echoes both.
  let jumpSupported = $derived(snap.jumpSupported);
  let approvalsSupported = $derived(snap.approvalsSupported);

  // ---- Phase 4: approvals hook status ----
  let hook = $state<HookStatus | null>(null);
  let busy = $state(false);
  // ---- window-render: session-location ("jump") hook status ----
  let loc = $state<LocateStatus | null>(null);
  let locBusy = $state(false);
  // Codex jump — a separate hook (~/.codex/hooks.json), independent toggle.
  let codexLoc = $state<LocateStatus | null>(null);
  let codexLocBusy = $state(false);

  onMount(() => {
    hookStatus().then((s) => (hook = s)).catch(() => {});
    locateStatus().then((s) => (loc = s)).catch(() => {});
    codexLocateStatus().then((s) => (codexLoc = s)).catch(() => {});
    getVersion().then((v) => (version = v)).catch(() => {});
    // Esc closes the dashboard window (it hides, so it can reopen later) and
    // returns Tablo to a switcher-hidden widget.
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (showAbout) {
        showAbout = false;
        return;
      }
      hideDashboard().catch(() => {});
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
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

  async function toggleLocate() {
    if (locBusy) return;
    locBusy = true;
    try {
      loc = await setLocateEnabled(!(loc?.installed ?? false));
    } catch {
      /* leave prior status */
    } finally {
      locBusy = false;
    }
  }

  async function toggleCodexLocate() {
    if (codexLocBusy) return;
    codexLocBusy = true;
    try {
      codexLoc = await setCodexLocateEnabled(!(codexLoc?.installed ?? false));
    } catch {
      /* leave prior status */
    } finally {
      codexLocBusy = false;
    }
  }

  // Install the update the background check parked. A success never returns —
  // the backend restarts the app — so only the failure path lands back here.
  let updateBusy = $state(false);
  let updateError = $state("");
  async function runInstallUpdate() {
    if (updateBusy) return;
    updateBusy = true;
    updateError = "";
    try {
      await installUpdate();
    } catch (e) {
      updateError = `Update failed: ${e}. Try again, or grab the release manually.`;
    } finally {
      updateBusy = false;
    }
  }

  function decide(id: string, decision: PermDecision) {
    resolvePermission(id, decision);
  }

  function jump(sessionId: string) {
    jumpToSession(sessionId).catch(() => {});
  }
</script>

<div class="dash">
  <div class="dash-head">
    <div class="lead">
      <h2><img class="g" src="/tablo-logo-v4.png" alt="" /> tablo</h2>
      {#if !snap.hasProjectsDir}
        <p class="statline muted">No agent sessions found yet</p>
      {:else if snap.agentCount === 0 && snap.waiting === 0}
        <p class="statline muted">No active sessions right now</p>
      {:else}
        <p class="statline">
          <span><b>{snap.agentCount}</b> active</span>
          <span class="sep">·</span>
          <span class:warn={snap.waiting > 0}><b>{snap.waiting}</b> waiting</span>
          <span class="sep">·</span>
          <span><b>{snap.projects}</b> project{snap.projects === 1 ? "" : "s"}</span>
        </p>
      {/if}
    </div>
    {#if view === "dashboard"}
      <div class="head-actions">
        <button class="settings-btn" title="About tablo" aria-label="About tablo" onclick={() => (showAbout = true)}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="12" cy="12" r="10" />
            <line x1="12" y1="16" x2="12" y2="12" />
            <line x1="12" y1="8" x2="12.01" y2="8" />
          </svg>
        </button>
        <button class="settings-btn" title="Settings" aria-label="Settings" onclick={() => (view = "settings")}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
        </button>
      </div>
    {/if}
  </div>

  {#if updateAvailable}
    <div class="update-bar">
      <span class="update-led"></span>
      <span class="update-txt">tablo <b>v{updateAvailable}</b> is available</span>
      <button class="update-act" disabled={updateBusy} onclick={runInstallUpdate}>
        {updateBusy ? "installing…" : "Install & restart"}
      </button>
    </div>
    {#if updateError}
      <p class="update-err">{updateError}</p>
    {/if}
  {/if}

  {#if whatsNew}
    <div class="wn" transition:fade={{ duration: 140 }}>
      <div class="wn-head">
        <span class="wn-led"></span>
        <span class="wn-txt">Updated to <b>v{whatsNew.version}</b></span>
        <button class="wn-close" title="Dismiss" aria-label="Dismiss release notes" onclick={() => dismissWhatsNew().catch(() => {})}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true">
            <line x1="18" y1="6" x2="6" y2="18" />
            <line x1="6" y1="6" x2="18" y2="18" />
          </svg>
        </button>
      </div>
      {#each whatsNew.groups as g}
        {#if g.heading}<div class="wn-h">{g.heading}</div>{/if}
        <ul class="wn-list">
          {#each g.items as item}
            <li>{item}</li>
          {/each}
        </ul>
      {/each}
    </div>
  {/if}

  {#if view === "dashboard"}
  <div class="dash-grid">
    {#if critCards.length}
      <div class="card crit-card">
        <h3 class="crit-h3">
          <span class="crit-led"></span>
          Context window warning ! &gt;{warnPct}%
          <span class="n">{critCards.length}</span>
        </h3>
        {#each critCards as c (c.key)}
          {@render sessionCard(c)}
        {/each}
      </div>
    {/if}

    {#if cards.length === 0 || restCards.length}
      <div class="card">
        <h3>
          Sessions
          {#if restCards.length > 0}
            <span class="sep">·</span>
            <span class="legend" aria-hidden="true">
              <span><i class="lmk user">#</i> user prompt</span>
              <span><i class="lmk agent">&gt;</i> agent response</span>
            </span>
          {/if}
          {#if filtersActive}
            <span class="h3-filter"><FilterButton showSource={sourceFilterActive} /></span>
          {/if}
          <span class="n">{restCards.length}</span>
        </h3>
        {#if cards.length === 0}
          <div class="dash-empty">Nothing running. Tablo is watching.</div>
        {:else if visibleCount === 0}
          <div class="dash-empty">Everything's filtered out.</div>
        {:else}
          {#each reqCards as c (c.key)}
            {@render sessionCard(c)}
          {/each}
          {#if waitCards.length}
            {@render groupHead("waiting", "Waiting", waitCards.length, prefs.collapseWaiting)}
            {#if !prefs.collapseWaiting}
              {#each waitCards as c (c.key)}
                {@render sessionCard(c)}
              {/each}
            {/if}
          {/if}
          {#if workCards.length}
            {@render groupHead("working", "Working", workCards.length, prefs.collapseWorking)}
            {#if !prefs.collapseWorking}
              {#each workCards as c (c.key)}
                {@render sessionCard(c)}
              {/each}
            {/if}
          {/if}
        {/if}
      </div>
    {/if}
  </div>
  {:else}
  <button class="back" onclick={() => (view = "dashboard")}>
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <line x1="19" y1="12" x2="5" y2="12" />
      <polyline points="12 19 5 12 12 5" />
    </svg>
    Back to Dashboard
  </button>
  <div class="dash-grid">
    <div class="card settings-card">
      <h3>Settings</h3>

      <div class="setting-grid">
        {#if hook && approvalsSupported}
          {@render toggle("Tool approvals", hook.serverUp ? `Intercept ${hook.tools.join(", ")} so you can approve or deny before they run.` : "Approval server not running.", hook.installed, busy, toggleApprovals)}
        {/if}
        {#if loc && jumpSupported}
          {@render toggle("Jump to Claude session (experimental)", "Focus the terminal window a session lives in (reads its tmux pane / terminal app).", loc.installed, locBusy, toggleLocate)}
        {/if}
      </div>

      <div class="setting-grid">
        {@render toggle("Watch Codex", "Show OpenAI Codex CLI sessions (~/.codex) alongside Claude Code.", watchCodex, false, () => setWatchCodex(!watchCodex))}
        {#if codexLoc && watchCodex && jumpSupported}
          {@render toggle("Jump to Codex session (experimental)", "Focus the terminal a Codex session lives in (installs a hook in ~/.codex/hooks.json — Codex asks you to trust it once).", codexLoc.installed, codexLocBusy, toggleCodexLocate)}
        {/if}
      </div>

      {@render toggle("Panel shortcut", "Summon the panel from anywhere with Ctrl+Cmd+P — no need to click the widget.", panelShortcutEnabled, false, () => setPanelShortcutEnabled(!panelShortcutEnabled))}

      {#if aerospaceAvailable}
        {@render toggle("Follow AeroSpace workspace", "AeroSpace hides windows on workspace switch — keep the widget with you by moving it to whichever workspace you focus.", aerospaceFollow, false, () => setAerospaceFollow(!aerospaceFollow))}
      {/if}

      <div class="setting">
        <div class="setting-main">
          <div class="setting-title">Context window limit</div>
          <div class="setting-sub">Warn (and mark critical) once a session's context passes this.</div>
        </div>
        <div class="num">
          <input
            type="number"
            min="1"
            max="100"
            step="1"
            value={warnPct}
            onchange={(e) => setWarnPct(Math.max(1, Math.min(100, Math.round(+e.currentTarget.value) || 60)))}
          />
          <span class="unit">%</span>
        </div>
      </div>

      <div class="setting">
        <div class="setting-main">
          <div class="setting-title">Cancelled-prompt grace</div>
          <div class="setting-sub">If you Ctrl+C a prompt before Claude responds, Tablo can't tell it apart from a long think — so it waits this long, then drops back to idle.</div>
        </div>
        <div class="num">
          <input
            type="number"
            min="1"
            step="1"
            value={cancelGraceMins}
            onchange={(e) => setCancelGraceMins(Math.max(1, Math.round(+e.currentTarget.value) || 3))}
          />
          <span class="unit">min</span>
        </div>
      </div>

      <div class="setting">
        <div class="setting-main">
          <div class="setting-title">Clear waiting sessions</div>
          <div class="setting-sub">How long a finished session waiting on you stays in the panel before it clears. It reappears the moment you send it a new prompt.</div>
        </div>
        <div class="num">
          <input
            type="number"
            min="1"
            step="1"
            value={clearWaitingMins}
            onchange={(e) => setClearWaitingMins(Math.max(1, Math.round(+e.currentTarget.value) || 10))}
          />
          <span class="unit">min</span>
        </div>
      </div>

      {@render toggle("Waiting notifications", "A gentle nudge from the widget when a session finishes and starts waiting on you.", prefs.notifyOnWaiting, false, () => setNotifyOnWaiting(!prefs.notifyOnWaiting))}

      {#if prefs.notifyOnWaiting}
        {@render toggle("Notification sound", "Play a soft chime with the waiting toast.", prefs.notifySound, false, () => setNotifySound(!prefs.notifySound))}

        <div class="setting">
          <div class="setting-main">
            <div class="setting-title">Notification hover time</div>
            <div class="setting-sub">How long the waiting toast stays on screen.</div>
          </div>
          <div class="num">
            <input
              type="number"
              min={TOAST_SECS_MIN}
              max={TOAST_SECS_MAX}
              step="1"
              value={prefs.waitingToastSecs}
              onchange={(e) => setWaitingToastSecs(+e.currentTarget.value)}
            />
            <span class="unit">sec</span>
          </div>
        </div>
      {/if}

      {@render toggle("Anonymous usage stats", "Send an anonymous ping so we can count active users. Never any session data, paths, prompts, or tokens — only that tablo ran, plus your OS and app version.", telemetryEnabled, false, () => setTelemetryEnabled(!telemetryEnabled))}

      {@render toggle("Automatic updates", "Install new releases in the background and restart. Off still checks for them — you'll get a notification and an Install button here instead.", autoUpdate, false, () => setAutoUpdate(!autoUpdate))}

      {@render toggle("Cat animations", "Let the cat sleep, trot and startle. Off holds it on a still pose with a steady glow — the state colour still tells you what's happening.", prefs.animations, false, () => setAnimations(!prefs.animations))}

      <div class="setting">
        <div class="setting-main">
          <div class="setting-title">Theme</div>
          <div class="setting-sub">Switch between the warm dark and light looks.</div>
        </div>
        <ThemeToggle />
      </div>
    </div>
  </div>
  {/if}
</div>

{#if showAbout}
  <div class="about-overlay" transition:fade={{ duration: 120 }}>
    <button class="about-backdrop" aria-label="Close about dialog" onclick={() => (showAbout = false)}></button>
    <div class="about-dialog" role="dialog" aria-modal="true" aria-label="About tablo" tabindex="-1">
      <button class="about-close" title="Close" aria-label="Close" onclick={() => (showAbout = false)}>
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <line x1="18" y1="6" x2="6" y2="18" />
          <line x1="6" y1="6" x2="18" y2="18" />
        </svg>
      </button>
      <div class="about-body">
        <div class="about-cat" aria-hidden="true"></div>
        <div class="about-text">
          <div class="about-name">tablo</div>
          <div class="about-ver">{version ? `v${version}` : "—"}</div>
          <p class="about-tag">A tiny floating cat that watches your agents work</p>
          <p class="about-web">
            <button class="about-link" onclick={() => openUrl(WEBSITE_URL)}>tablo.unravel.tech</button>
          </p>
          <dl class="about-meta">
            <div>
              <dt>Open Source</dt>
              <dd>
                <button class="about-link" onclick={() => openUrl(REPO_URL)}>github.com/unravel-team/tablo</button>
              </dd>
            </div>
            <div><dt>License</dt><dd>MIT</dd></div>
          </dl>
          <p class="about-foot">Updates install automatically in the background.</p>
          <p class="about-credit">
            Made with
            <svg class="credit-heart" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
              <path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z" />
            </svg>
            by
            <button class="about-link logo-link" onclick={() => openUrl(UNRAVEL_URL)} aria-label="unravel.tech">
              <img class="unravel-logo" src="/unravel-logo.png" alt="unravel" />
            </button>
          </p>
        </div>
      </div>
    </div>
  </div>
{/if}

{#snippet toggle(title: string, sub: string, on: boolean, busy: boolean, onToggle: () => void)}
  <div class="setting">
    <div class="setting-main">
      <div class="setting-title">{title}</div>
      <div class="setting-sub">{sub}</div>
    </div>
    <button class="approvals-toggle" class:on disabled={busy} onclick={onToggle}>
      <span class="dot"></span>
      {on ? "on" : "off"}
    </button>
  </div>
{/snippet}

{#snippet groupHead(kind: "working" | "waiting", name: string, count: number, collapsed: boolean)}
  <button class="dgroup" class:collapsed onclick={() => toggleCollapse(kind)} aria-expanded={!collapsed}>
    <span class="dgroup-dot {kind === 'waiting' ? 'wait' : 'work'}"></span>
    <span class="dgroup-name">{name}</span>
    <span class="dgroup-count">{count}</span>
    <span class="dgroup-caret">&gt;</span>
  </button>
{/snippet}

{#snippet sessionCard(c: Card)}
  <div class="scard" class:needs={c.requests.length > 0}>
    <div class="drow">
      <span class="st {c.requests.length ? 'ask' : c.session?.activityKind || 'run'}"></span>
      <div class="info">
        <div class="p">
          <span class="pname">{c.project}</span>
          {#if c.session?.title}
            <span class="psep">·</span>
            <span class="ptitle">{c.session.title}</span>
          {/if}
        </div>
        <div class="path">{c.path}{c.branch ? ` · ${c.branch}` : ""}</div>
      </div>
      {#if c.session}
        <span class="src-tag">{c.session.source}</span>
        <span class="mode-badge">mode : <span class="mode-val {c.session.mode}">{c.session.mode}</span></span>
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
      {#if c.session?.canJump}
        <button class="jump" title="Focus the window this session lives in" onclick={() => jump(c.session!.id)}>
          jump &rarr;
        </button>
      {/if}
    </div>

    {#if c.session}
      {@render terminal(c.session)}
    {/if}
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
{/snippet}

<!-- Per-session terminal preview: a recessed amber-phosphor screen tailing the
     agent's recent activity. State lives on the wrapper's kind class. -->
{#snippet terminal(s: SessionView)}
  <div class="term {s.activityKind || 'idle'}">
    <div class="term-bar">
      <span class="term-name"><span class="term-dollar">$</span> live preview</span>
      <span class="term-state">{terminalStatus(s.activityKind)}</span>
    </div>
    <div class="term-scroll">
      {#if s.activityLog.length === 0}
        <div class="term-line think"><span class="mk">&gt;</span><span class="tx">awaiting activity</span></div>
      {:else}
        {#each s.activityLog as ln (ln.seq)}
          <div class="term-line {ln.kind}" in:fade={{ duration: 160 }}>
            <span class="mk">{activityMark(ln.kind)}</span>
            <span class="tx">{ln.text}</span>
          </div>
        {/each}
      {/if}
      <div class="term-line caret">
        <span class="mk">&gt;</span>
        <span class="cur" class:blink={s.activityKind === "working"}></span>
      </div>
    </div>
  </div>
{/snippet}

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
    height: 34px;
    width: auto; /* keep the logo's own (built-in-background) aspect — no crop */
    border-radius: 7px;
    flex: none;
  }
  /* settings gear — top-right of the header, dashboard view only */
  .settings-btn {
    align-self: flex-start;
    width: 34px;
    height: 34px;
    display: grid;
    place-items: center;
    border: 1px solid var(--border);
    border-radius: 9px;
    background: var(--bg-raised);
    color: var(--ink-dim);
    cursor: pointer;
    transition: color 0.18s var(--ease), border-color 0.18s var(--ease);
  }
  .settings-btn:hover,
  .back:hover {
    color: var(--ink);
    border-color: var(--ink-faint);
  }
  .settings-btn svg {
    width: 17px;
    height: 17px;
    display: block;
  }
  /* info (i) + gear grouped at the header's top-right */
  .head-actions {
    align-self: flex-start;
    display: flex;
    gap: 8px;
  }

  /* about — modal dialog popover */
  .about-overlay {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: center;
    padding: 24px;
    background: rgba(0, 0, 0, 0.5);
    -webkit-backdrop-filter: blur(2px);
    backdrop-filter: blur(2px);
  }
  .about-backdrop {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    padding: 0;
    border: none;
    background: transparent;
    cursor: default;
  }
  .about-dialog {
    position: relative;
    z-index: 1;
    width: 100%;
    max-width: 520px;
    padding: 26px 24px 20px;
    border: 1px solid var(--border);
    border-radius: 16px;
    background: var(--bg-surface);
    box-shadow: 0 20px 55px rgba(0, 0, 0, 0.5);
    animation: about-pop 0.14s var(--ease) both;
  }
  @keyframes about-pop {
    from {
      opacity: 0;
      transform: translateY(6px) scale(0.98);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  .about-close {
    position: absolute;
    top: 12px;
    right: 12px;
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--ink-faint);
    cursor: pointer;
    transition: color 0.15s var(--ease), background 0.15s var(--ease);
  }
  .about-close:hover {
    color: var(--ink);
    background: var(--bg-raised);
  }
  .about-close svg {
    width: 16px;
    height: 16px;
    display: block;
  }
  .about-body {
    display: flex;
    align-items: flex-start;
    gap: 18px;
  }
  /* first frame (of 5) of the shocked cat sprite — the sheet is 1024×292, so
     each frame is 204.8×292 (h/w = 365/256). Show frame 0 via background-position. */
  .about-cat {
    --w: 104px;
    flex: none;
    width: var(--w);
    height: calc(var(--w) * 365 / 256);
    margin-top: 2px;
    background-image: url(/sprites/shocked-sprite-sheet.png);
    background-repeat: no-repeat;
    background-position: 0 0;
    background-size: calc(var(--w) * 5) 100%;
  }
  .about-text {
    min-width: 0;
  }
  .about-name {
    font-family: var(--font-round);
    font-size: 22px;
    font-weight: 700;
    color: var(--ink);
    line-height: 1.1;
  }
  .about-ver {
    margin-top: 3px;
    font-family: var(--font-mono);
    font-size: 12.5px;
    font-weight: 600;
    color: var(--amber, var(--ink-dim));
  }
  .about-tag {
    margin: 12px 0 0;
    font-size: 13px;
    line-height: 1.55;
    color: var(--ink-dim);
  }
  .about-web {
    margin: 6px 0 0;
  }
  .about-meta {
    margin: 14px 0 0;
    display: grid;
    gap: 9px;
  }
  .about-meta > div {
    display: flex;
    align-items: baseline;
    gap: 12px;
  }
  .about-meta dt {
    flex: none;
    width: 104px;
    font-family: var(--font-mono);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--ink-faint);
  }
  .about-meta dd {
    margin: 0;
    font-size: 13px;
    color: var(--ink);
  }
  .about-link {
    padding: 0;
    border: none;
    background: none;
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--amber);
    cursor: pointer;
    text-decoration: none;
  }
  .about-link:hover {
    text-decoration: underline;
  }
  .about-foot {
    margin: 18px 0 0;
    font-size: 11.5px;
    color: var(--ink-faint);
  }
  .about-credit {
    display: flex;
    align-items: center;
    gap: 5px;
    margin: 12px 0 0;
    font-size: 11.5px;
    color: var(--ink-faint);
  }
  .about-credit .credit-heart {
    width: 12px;
    height: 12px;
    color: var(--coral);
  }
  .about-credit .logo-link {
    display: inline-flex;
    align-items: center;
    line-height: 0;
    transition: opacity 0.15s var(--ease);
  }
  .about-credit .logo-link:hover {
    opacity: 0.7;
    text-decoration: none;
  }
  .about-credit .unravel-logo {
    height: 15px;
    width: auto;
    display: block;
  }
  /* back link, settings view */
  .back {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    margin-bottom: 16px;
    padding: 6px 12px 6px 9px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg-raised);
    color: var(--ink-dim);
    font-family: var(--font-round);
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    transition: color 0.18s var(--ease), border-color 0.18s var(--ease);
  }
  .back svg {
    width: 15px;
    height: 15px;
    display: block;
  }
  .statline {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 500;
    color: var(--ink-dim);
  }
  .statline b {
    color: var(--ink);
    font-weight: 700;
  }
  .statline .sep {
    color: var(--ink-faint);
  }
  .statline .warn,
  .statline .warn b {
    color: var(--coral);
  }
  .statline.muted {
    color: var(--ink-faint);
  }
  /* settings pane rows */
  .settings-card {
    display: flex;
    flex-direction: column;
  }
  /* approvals + jump share one two-column row; the on/off + theme rows stack
     below it, each a full-width row divided by a top border. */
  .setting-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 28px;
    padding-bottom: 4px;
  }
  .setting {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
  }
  .settings-card > .setting {
    padding: 15px 0;
    border-top: 1px solid var(--border-soft);
  }
  .setting-main {
    min-width: 0;
  }
  .num {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .num input {
    width: 50px;
    padding: 5px 8px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg-inset);
    color: var(--ink);
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 600;
    text-align: right;
  }
  .num input:focus {
    outline: none;
    border-color: var(--ink-faint);
  }
  /* plain text box — no spinner arrows */
  .num input::-webkit-outer-spin-button,
  .num input::-webkit-inner-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }
  .num .unit {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-faint);
  }
  .setting-title {
    font-size: 13.5px;
    font-weight: 600;
    color: var(--ink);
  }
  .setting-sub {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--ink-faint);
    margin-top: 3px;
    line-height: 1.5;
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
    padding: 16px 0;
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
  /* update banner — only rendered while auto-update is off and the background
     check found a release. Amber (the "something is happening" colour), not
     coral: an available update is news, not an alarm. */
  .update-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 18px;
    padding: 11px 14px;
    border: 1px solid color-mix(in srgb, var(--amber) 40%, var(--border));
    border-radius: var(--r-md);
    background: color-mix(in srgb, var(--amber) 8%, var(--bg-raised));
  }
  .update-led {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--amber);
    box-shadow: 0 0 8px var(--amber);
    flex-shrink: 0;
  }
  .update-txt {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: 12.5px;
    color: var(--ink-dim);
  }
  .update-txt b {
    color: var(--ink);
    font-weight: 700;
  }
  .update-act {
    flex-shrink: 0;
    padding: 7px 16px;
    border: 1px solid transparent;
    border-radius: var(--r-sm);
    background: var(--amber);
    color: var(--bg-surface);
    font-family: var(--font-round);
    font-size: 12.5px;
    font-weight: 600;
    cursor: pointer;
    transition: transform 0.12s var(--ease);
  }
  .update-act:hover:not(:disabled) {
    transform: translateY(-1px);
  }
  .update-act:disabled {
    opacity: 0.6;
    cursor: default;
  }
  /* what's new — sage, not amber: the update already happened, nothing to act on. */
  .wn {
    margin-bottom: 18px;
    padding: 12px 14px 14px;
    border: 1px solid color-mix(in srgb, var(--sage) 34%, var(--border));
    border-radius: var(--r-md);
    background: color-mix(in srgb, var(--sage) 7%, var(--bg-raised));
  }
  .wn-head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .wn-led {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--sage);
    box-shadow: 0 0 8px var(--sage);
    flex-shrink: 0;
  }
  .wn-txt {
    flex: 1;
    min-width: 0;
    font-family: var(--font-mono);
    font-size: 12.5px;
    color: var(--ink-dim);
  }
  .wn-txt b {
    color: var(--ink);
    font-weight: 700;
  }
  .wn-close {
    flex-shrink: 0;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--ink-faint);
    cursor: pointer;
  }
  .wn-close:hover {
    color: var(--ink);
    background: var(--bg-inset);
  }
  .wn-close svg {
    width: 13px;
    height: 13px;
  }
  .wn-h {
    margin: 12px 0 4px;
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--sage);
  }
  .wn-list {
    margin: 0;
    padding-left: 17px;
    list-style: disc;
  }
  .wn-list li {
    font-size: 12.5px;
    line-height: 1.55;
    color: var(--ink-dim);
  }
  .wn-list li::marker {
    color: color-mix(in srgb, var(--sage) 70%, transparent);
  }
  .update-err {
    margin: -10px 0 18px;
    font-family: var(--font-mono);
    font-size: 12px;
    color: var(--coral);
  }

  .dash-grid {
    display: grid;
    grid-template-columns: 1fr;
    gap: 18px;
  }

  .card {
    background: var(--bg-raised);
    border: 1px solid var(--border-soft);
    border-radius: var(--r-md);
    padding: 18px;
  }
  /* Light mode only: the card sits on a soft warm beige, gently darker than the
     near-white page (see app.css) — the full "room" beige read too muddy. */
  :global([data-theme="light"]) .card {
    background: var(--bg-inset);
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

  /* Critical card: over-threshold sessions, pinned above the rest with a red
     border + warning header. */
  .card.crit-card {
    border-color: color-mix(in srgb, var(--coral) 45%, var(--border));
  }
  .card h3.crit-h3 {
    color: var(--coral);
  }
  .crit-led {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    background: var(--coral);
    box-shadow: 0 0 8px var(--coral);
    flex-shrink: 0;
  }

  /* separator + inline key for the terminal markers, beside the Sessions title */
  .card h3 .sep {
    color: var(--ink-faint);
    font-weight: 400;
  }
  .card h3 .legend {
    display: inline-flex;
    align-items: center;
    gap: 14px;
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0.02em;
    color: var(--ink-faint);
    text-transform: none;
  }
  .card h3 .legend .lmk {
    font-style: normal;
    font-weight: 700;
    margin-right: 5px;
    color: var(--amber);
  }

  .drow {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 3px 0;
  }
  .drow .st {
    width: 8px;
    height: 8px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  /* per-session state LED: working = amber, waiting-for-input = sage,
     permission request = coral. `run` is the fallback for unknown activity. */
  .drow .st.run,
  .drow .st.waiting {
    background: var(--sage);
    box-shadow: 0 0 7px var(--sage);
  }
  .drow .st.working {
    background: var(--amber);
    box-shadow: 0 0 7px var(--amber);
  }
  .drow .st.thinking {
    background: var(--ink-faint);
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
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 13px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
  }
  .drow .info .p .pname {
    flex-shrink: 0;
  }
  .drow .info .p .psep {
    color: var(--ink-faint);
    flex-shrink: 0;
  }
  .drow .info .p .ptitle {
    font-weight: 500;
    color: var(--ink-dim);
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
    margin-top: 3px;
  }
  /* ===== terminal preview (window-render) — a recessed amber-phosphor screen.
     Colors are fixed dark (a "screen" reads as intentional in both themes). ===== */
  .term {
    margin-top: 13px;
    border-radius: 11px;
    border: 1px solid #2b2015;
    background:
      radial-gradient(130% 80% at 50% -20%, rgba(224, 164, 88, 0.1), transparent 55%),
      #140e07;
    box-shadow:
      inset 0 1px 0 rgba(224, 164, 88, 0.06),
      inset 0 0 44px rgba(0, 0, 0, 0.55),
      0 2px 12px -6px rgba(0, 0, 0, 0.7);
    overflow: hidden;
    position: relative;
  }
  /* faint scanlines — kept low so it never turns into full CRT */
  .term::after {
    content: "";
    position: absolute;
    inset: 0;
    pointer-events: none;
    background: repeating-linear-gradient(0deg, transparent 0 2px, rgba(0, 0, 0, 0.12) 2px 3px);
    opacity: 0.5;
  }
  .term-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    border-bottom: 1px solid #241a10;
    background: linear-gradient(180deg, rgba(224, 164, 88, 0.05), transparent);
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.03em;
    position: relative;
    z-index: 1;
  }
  .term-name {
    color: #9c8c78;
  }
  .term-dollar {
    color: #e0a458;
    font-weight: 700;
    margin-right: 3px;
  }
  .term-state {
    margin-left: auto;
    color: #6f6250;
  }
  /* jump button — top-right of the row, beside the context gauge */
  .drow .jump {
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: 10.5px;
    font-weight: 600;
    letter-spacing: 0.02em;
    color: var(--amber);
    background: var(--amber-soft);
    border: 1px solid color-mix(in srgb, var(--amber) 32%, transparent);
    border-radius: 6px;
    padding: 5px 10px;
    cursor: pointer;
    white-space: nowrap;
    align-self: flex-end; /* lower edge lines up with the mode text */
    transition:
      background-color 0.15s var(--ease),
      transform 0.12s var(--ease);
  }
  .drow .jump:hover {
    background: color-mix(in srgb, var(--amber) 22%, var(--amber-soft));
    transform: translateY(-1px);
  }
  /* agent source tag (claude / codex) — neutral (never a state color), sits just
     left of the mode display, dropped to the row's bottom so they line up */
  .drow .src-tag {
    flex-shrink: 0;
    align-self: flex-end;
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
  /* read-only permission-mode badge — sits just left of the context gauge,
     dropped to the bottom of the row so it lines up with the path/branch text */
  .drow .mode-badge {
    flex-shrink: 0;
    align-self: flex-end;
    font-family: var(--font-mono);
    font-size: 10px;
    letter-spacing: 0.02em;
    color: var(--ink-faint);
    white-space: nowrap;
  }
  .drow .mode-val {
    font-weight: 600;
    color: var(--ink-dim);
  }
  .drow .mode-val.auto {
    color: var(--amber);
  }
  .drow .mode-val.plan {
    color: var(--sage);
  }
  .drow .mode-val.bypass {
    color: var(--coral);
  }
  .term.working .term-state {
    color: #e0a458;
  }
  .term.waiting .term-state {
    color: #8faa7e;
  }

  .term-scroll {
    padding: 11px 13px 12px;
    font-family: var(--font-mono);
    font-size: 11px;
    line-height: 1.75;
    min-height: 92px;
    max-height: 176px;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    justify-content: flex-end; /* stick to the bottom like tail -f */
    position: relative;
    z-index: 1;
  }
  .term-line {
    display: flex;
    gap: 9px;
    white-space: nowrap;
    overflow: hidden;
  }
  .term-line .mk {
    flex-shrink: 0;
    width: 9px;
    text-align: center;
  }
  .term-line .tx {
    flex: 1;
    min-width: 0; /* lets text-overflow ellipsis kick in inside the flex row */
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* the human's own prompt — brightest line, and a little air above marks a
     new turn (you asked, then the agent did X, Y, Z) */
  .term-line.user {
    margin-top: 7px;
  }
  .term-line.user:first-child {
    margin-top: 0;
  }
  .term-line.user .mk {
    color: #e0a458;
    font-weight: 700;
    text-shadow: 0 0 9px rgba(224, 164, 88, 0.5);
  }
  .term-line.user .tx {
    color: #f6ead8;
    font-weight: 600;
  }
  /* tool call = the command line, bright amber phosphor */
  .term-line.tool .mk {
    color: #e0a458;
  }
  .term-line.tool .tx {
    color: #f1ddc0;
    text-shadow: 0 0 10px rgba(224, 164, 88, 0.3);
  }
  /* spoken text = output, warm cream */
  .term-line.text .mk {
    color: #6f6250;
  }
  .term-line.text .tx {
    color: #c9b8a0;
  }
  /* thinking = dim comment */
  .term-line.think {
    opacity: 0.62;
  }
  .term-line.think .mk {
    color: #6f6250;
  }
  .term-line.think .tx {
    color: #8a7c69;
    font-style: italic;
  }
  /* live caret line */
  .term-line.caret .mk {
    color: #e0a458;
  }
  .cur {
    display: inline-block;
    width: 8px;
    height: 13px;
    align-self: center;
    background: #e0a458;
    box-shadow: 0 0 9px rgba(224, 164, 88, 0.55);
    opacity: 0.22;
  }
  .cur.blink {
    animation: term-blink 1.05s steps(1) infinite;
  }
  @keyframes term-blink {
    0%,
    50% {
      opacity: 0.95;
    }
    50.01%,
    100% {
      opacity: 0.08;
    }
  }
  .drow .gauge {
    width: 168px;
    flex-shrink: 0;
    align-self: flex-end; /* bar's lower edge lines up with the mode text */
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
    /* Outline the well so the empty track stays legible even when it shares the
       card's color (light mode) — matches the panel's context track. */
    border: 1px solid var(--border);
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

  /* inline filter in the Sessions header — reset the header's uppercasing so its labels stay lowercase */
  .h3-filter {
    display: inline-flex;
    text-transform: none;
    letter-spacing: normal;
  }

  /* collapsible Working / Waiting subgroup headers — caret points down when open, right when collapsed */
  .dgroup {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 12px 2px 8px;
    border: none;
    background: none;
    cursor: pointer;
    color: var(--ink-dim);
    font-family: var(--font-mono);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    transition: color 0.15s var(--ease);
  }
  .dgroup:hover {
    color: var(--ink);
  }
  .dgroup-dot {
    width: 7px;
    height: 7px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .dgroup-dot.wait {
    background: var(--sage);
    box-shadow: 0 0 8px var(--sage);
  }
  .dgroup-dot.work {
    background: var(--amber);
    box-shadow: 0 0 8px var(--amber);
  }
  .dgroup-count {
    margin-left: auto;
    color: var(--ink-faint);
  }
  .dgroup-caret {
    color: var(--ink-faint);
    transform: rotate(90deg);
    transition: transform 0.18s var(--ease);
  }
  .dgroup.collapsed .dgroup-caret {
    transform: none;
  }

  .dash-empty {
    padding: 24px 0;
    text-align: center;
    color: var(--ink-faint);
    font-family: var(--font-mono);
    font-size: 12px;
  }
</style>
