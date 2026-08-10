// UI-only view preferences for the multi-session views (Phase 2). These are
// presentation choices, not domain config (which lives in Rust `config::Config`),
// so they stay frontend-side. Persisted to localStorage so a choice survives a
// panel reopen / app restart and is shared live across windows (same origin).
// Guarded for the browserless prerender pass.

import { browser } from "$app/environment";
import type { SessionView } from "./types";

export type SortMode = "context" | "recent";
export type StateFilter = "working" | "waiting";
export type SourceFilter = "claude" | "codex";
// The two state groups the panel/dashboard can fold away to save scrolling.
export type CollapseGroup = "working" | "waiting" | "subagents";

const KEY = "tablo.viewPrefs";

interface Prefs {
  sort: SortMode;
  // Per-state visibility toggles for the panel groups. Both on by default (show
  // everything); the Permission Request group is never filtered.
  showWorking: boolean;
  showWaiting: boolean;
  // Per-source (which agent) visibility toggles. Both on by default; only surface
  // when sessions from more than one source are present.
  showClaude: boolean;
  showCodex: boolean;
  // Toast when a session finishes working and starts waiting on you. Default on.
  notifyOnWaiting: boolean;
  // How long that toast stays on screen, in seconds. Default 4.
  waitingToastSecs: number;
  // Play a chime with that toast. Default on, so the nudge is audible without
  // having to find the setting
  notifySound: boolean;
  // Animate the cat (frame loops, breathing glow, sleep/wake transitions).
  // Default on; off holds each state on a still pose with a steady glow.
  animations: boolean;
  // Fold the Working / Waiting groups closed; both expanded by default, shared live across panel + dashboard.
  collapseWorking: boolean;
  collapseWaiting: boolean;
  collapseSubagents: boolean;
}

// Bounds for the toast hover time (seconds).
export const TOAST_SECS_MIN = 1;
export const TOAST_SECS_MAX = 30;

function clampSecs(n: unknown): number {
  const v = Math.round(Number(n));
  if (!Number.isFinite(v)) return 4;
  return Math.min(TOAST_SECS_MAX, Math.max(TOAST_SECS_MIN, v));
}

const DEFAULTS: Prefs = {
  sort: "context",
  showWorking: true,
  showWaiting: true,
  showClaude: true,
  showCodex: true,
  notifyOnWaiting: true,
  waitingToastSecs: 4,
  notifySound: true,
  animations: true,
  collapseWorking: false,
  collapseWaiting: false,
  collapseSubagents: false,
};

function coerce(raw: unknown): Prefs {
  const p = (raw ?? {}) as Partial<Prefs>;
  return {
    sort: p.sort === "recent" ? "recent" : "context",
    showWorking: p.showWorking !== false,
    showWaiting: p.showWaiting !== false,
    showClaude: p.showClaude !== false,
    showCodex: p.showCodex !== false,
    notifyOnWaiting: p.notifyOnWaiting !== false,
    waitingToastSecs: p.waitingToastSecs == null ? 4 : clampSecs(p.waitingToastSecs),
    notifySound: p.notifySound !== false,
    animations: p.animations !== false,
    collapseWorking: p.collapseWorking === true,
    collapseWaiting: p.collapseWaiting === true,
    collapseSubagents: p.collapseSubagents === true,
  };
}

function load(): Prefs {
  if (!browser) return { ...DEFAULTS };
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? coerce(JSON.parse(raw)) : { ...DEFAULTS };
  } catch {
    return { ...DEFAULTS };
  }
}

export const prefs = $state<Prefs>(load());

function persist() {
  if (!browser) return;
  try {
    localStorage.setItem(
      KEY,
      JSON.stringify({
        sort: prefs.sort,
        showWorking: prefs.showWorking,
        showWaiting: prefs.showWaiting,
        showClaude: prefs.showClaude,
        showCodex: prefs.showCodex,
        notifyOnWaiting: prefs.notifyOnWaiting,
        waitingToastSecs: prefs.waitingToastSecs,
        notifySound: prefs.notifySound,
        animations: prefs.animations,
        collapseWorking: prefs.collapseWorking,
        collapseWaiting: prefs.collapseWaiting,
        collapseSubagents: prefs.collapseSubagents,
      })
    );
  } catch {
    /* storage unavailable — keep the in-memory value */
  }
}

export function setSort(mode: SortMode) {
  prefs.sort = mode;
  persist();
}

export function toggleFilter(kind: StateFilter) {
  if (kind === "working") prefs.showWorking = !prefs.showWorking;
  else prefs.showWaiting = !prefs.showWaiting;
  persist();
}

export function toggleSource(kind: SourceFilter) {
  if (kind === "claude") prefs.showClaude = !prefs.showClaude;
  else prefs.showCodex = !prefs.showCodex;
  persist();
}

export function toggleCollapse(kind: CollapseGroup) {
  if (kind === "working") prefs.collapseWorking = !prefs.collapseWorking;
  else if (kind === "subagents") prefs.collapseSubagents = !prefs.collapseSubagents;
  else prefs.collapseWaiting = !prefs.collapseWaiting;
  persist();
}

export function setNotifyOnWaiting(on: boolean) {
  prefs.notifyOnWaiting = on;
  persist();
}

export function setWaitingToastSecs(secs: number) {
  prefs.waitingToastSecs = clampSecs(secs);
  persist();
}

export function setNotifySound(on: boolean) {
  prefs.notifySound = on;
  persist();
}

export function setAnimations(on: boolean) {
  prefs.animations = on;
  persist();
}

// Live-sync when another window (panel <-> dashboard) changes the preference.
// The `storage` event only fires in *other* windows, so the writer isn't
// double-applied.
if (browser) {
  window.addEventListener("storage", (e) => {
    if (e.key !== KEY || e.newValue == null) return;
    try {
      const next = coerce(JSON.parse(e.newValue));
      prefs.sort = next.sort;
      prefs.showWorking = next.showWorking;
      prefs.showWaiting = next.showWaiting;
      prefs.showClaude = next.showClaude;
      prefs.showCodex = next.showCodex;
      prefs.notifyOnWaiting = next.notifyOnWaiting;
      prefs.waitingToastSecs = next.waitingToastSecs;
      prefs.notifySound = next.notifySound;
      prefs.animations = next.animations;
      prefs.collapseWorking = next.collapseWorking;
      prefs.collapseWaiting = next.collapseWaiting;
      prefs.collapseSubagents = next.collapseSubagents;
    } catch {
      /* ignore malformed cross-window payload */
    }
  });
}

// Comparator for ordering sessions *within* a state group. Kept pure so both the
// panel (grouped) and dashboard (flat) can share one sort definition.
export function byMode(mode: SortMode): (a: SessionView, b: SessionView) => number {
  return mode === "recent"
    ? (a, b) => b.lastActive - a.lastActive
    : (a, b) => b.pct - a.pct;
}

// Flat ordering used by surfaces that don't split into groups: waiting-on-you
// sessions stay first (so a pending question is never buried), then the chosen
// mode. Returns a new array; never mutates the input.
export function sortSessions(list: SessionView[], mode: SortMode): SessionView[] {
  const cmp = byMode(mode);
  const askRank = (s: SessionView) => (s.state === "ask" ? 0 : 1);
  return [...list].sort((a, b) => askRank(a) - askRank(b) || cmp(a, b));
}
