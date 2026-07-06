// Shapes mirror the Rust `scanner::Snapshot` / `SessionView` (camelCase JSON).

export type AvatarState = "idle" | "running" | "alarmed";
export type Level = "ok" | "warn" | "crit";
export type SessionState = "run" | "ask";

export interface SessionView {
  id: string;
  project: string;
  path: string;
  branch: string | null;
  pct: number; // 0..100
  used: number;
  limit: number;
  model: string;
  state: SessionState;
  level: Level;
  lastActive: number; // ms epoch
}

export interface PlanUsage {
  fiveHourTokens: number;
  weekTokens: number;
  fiveHourBudget: number;
  weekBudget: number;
  fiveHourPct: number;
  weekPct: number;
  fiveHourLevel: Level;
  weekLevel: Level;
}

export interface Snapshot {
  state: AvatarState;
  agentCount: number;
  waiting: number;
  projects: number;
  sessions: SessionView[];
  plan: PlanUsage;
  generatedAt: number;
  hasProjectsDir: boolean;
}

// Mirrors the Rust `config::Config` — the single source of truth. The frontend
// only reads `theme` and the context-window values (for the header toggle);
// everything else is consumed backend-side.
export interface Config {
  avatarX: number | null;
  avatarY: number | null;
  activeWindowSecs: number;
  defaultContextLimit: number;
  standardContextLimit: number;
  extendedContextLimit: number;
  extendedWindowMarkers: string[];
  warnPct: number;
  critPct: number;
  fiveHourSecs: number;
  weekSecs: number;
  fiveHourTokenBudget: number;
  weekTokenBudget: number;
  initialTailCapBytes: number;
  notifyOnWarn: boolean;
  theme: "dark" | "light";
}

export const EMPTY_PLAN: PlanUsage = {
  fiveHourTokens: 0,
  weekTokens: 0,
  fiveHourBudget: 0,
  weekBudget: 0,
  fiveHourPct: 0,
  weekPct: 0,
  fiveHourLevel: "ok",
  weekLevel: "ok",
};

// Loading placeholder only — real values are fetched from the backend on mount,
// so no domain defaults are duplicated here.
export const LOADING_CONFIG: Config = {
  avatarX: null,
  avatarY: null,
  activeWindowSecs: 0,
  defaultContextLimit: 0,
  standardContextLimit: 0,
  extendedContextLimit: 0,
  extendedWindowMarkers: [],
  warnPct: 0,
  critPct: 0,
  fiveHourSecs: 0,
  weekSecs: 0,
  fiveHourTokenBudget: 0,
  weekTokenBudget: 0,
  initialTailCapBytes: 0,
  notifyOnWarn: false,
  theme: "dark",
};

export const EMPTY_SNAPSHOT: Snapshot = {
  state: "idle",
  agentCount: 0,
  waiting: 0,
  projects: 0,
  sessions: [],
  plan: EMPTY_PLAN,
  generatedAt: 0,
  hasProjectsDir: true,
};
