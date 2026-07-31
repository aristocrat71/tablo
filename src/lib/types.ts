// Shapes mirror the Rust `scanner::Snapshot` / `SessionView` (camelCase JSON).

export type AvatarState = "idle" | "running" | "alarmed";
export type Level = "ok" | "warn" | "crit";
export type SessionState = "run" | "ask";
// Read-only permission-mode badge: normal / auto-accept edits / plan / bypass.
export type SessionMode = "normal" | "auto" | "plan" | "bypass";

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
  mode: SessionMode; // read-only permission-mode badge
  level: Level;
  lastActive: number; // ms epoch
  title: string | null; // Claude Code's AI session title (disambiguates rows)
  activity: string; // live one-liner: "editing scanner.rs", "" if unknown
  activityKind: ActivityKind; // UI hint for icon + state suffix
  activityLog: ActivityEntry[]; // rolling tail for the dashboard terminal
  canJump: boolean; // Tablo knows where this session lives (jump button)
  source: SessionSource; // which agent produced this session
}

// Which agent a session belongs to — drives the small source tag on each row.
export type SessionSource = "claude" | "codex";

export type ActivityKind = "working" | "waiting" | "thinking" | "";

// One line in the terminal tail. `kind` is the block type ("tool" | "text" |
// "think"); `seq` is monotonic per session so the view can key + animate lines.
export interface ActivityEntry {
  seq: number;
  kind: "user" | "tool" | "text" | "think";
  text: string;
}

// A tool call awaiting a human approve/deny (Phase 4). Mirrors Rust
// `permission::PendingRequest`.
export interface PendingRequest {
  id: string;
  tool: string;
  detail: string;
  sessionId: string;
  project: string;
  path: string;
  createdAt: number;
}

export type PermDecision = "allow" | "deny" | "ask";

export interface Snapshot {
  state: AvatarState;
  agentCount: number;
  waiting: number;
  projects: number;
  sessions: SessionView[];
  generatedAt: number;
  hasProjectsDir: boolean;
  planTier: string | null; // raw account tier, e.g. "default_claude_max_5x"
  pending: PendingRequest[];
  warnPct: number; // context warn threshold (%), echoed for the Critical group
  cancelGraceMins: number; // early-cancel grace window (min), echoed for Settings
  clearWaitingMins: number; // waiting-session clear window (min), echoed for Settings
  watchCodex: boolean; // whether Codex sessions are watched, echoed for Settings
  panelShortcutEnabled: boolean; // whether the global panel hotkey is on, echoed for Settings
  aerospaceFollow: boolean; // whether Tablo follows the focused AeroSpace workspace, echoed for Settings
  telemetryEnabled: boolean; // whether anonymous usage pings are on, echoed for Settings
  autoUpdate: boolean; // whether new releases install themselves, echoed for Settings
  updateAvailable: string | null; // version found but not installed (auto-update off) — offer a one-tap install
  aerospaceAvailable: boolean; // whether AeroSpace was detected (gates the follow toggle)
  jumpSupported: boolean; // whether "jump to session" works here (macOS only) — gates the Jump toggles
  approvalsSupported: boolean; // whether the approvals hook can run here (not Windows) — gates the approvals toggle
}

// Mirrors Rust `permission::HookStatus`.
export interface HookStatus {
  installed: boolean;
  serverUp: boolean;
  scriptPath: string;
  port: number;
  tools: string[];
}

// Mirrors Rust `locate::LocateStatus` — session-location reporting for "jump".
export interface LocateStatus {
  installed: boolean;
  scriptPath: string;
}

// Mirrors the Rust `config::Config` — the single source of truth. The frontend
// only reads `theme`; everything else is consumed backend-side.
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
  initialTailCapBytes: number;
  notifyOnWarn: boolean;
  theme: "dark" | "light";
}

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
  generatedAt: 0,
  hasProjectsDir: true,
  planTier: null,
  pending: [],
  warnPct: 60,
  cancelGraceMins: 3,
  clearWaitingMins: 10,
  watchCodex: true,
  panelShortcutEnabled: true,
  aerospaceFollow: true,
  telemetryEnabled: true,
  autoUpdate: true,
  updateAvailable: null,
  aerospaceAvailable: false,
  // Hide-until-confirmed (like aerospaceAvailable); the real snapshot sets the true platform value.
  jumpSupported: false,
  approvalsSupported: false,
};
