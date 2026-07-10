// Thin wrappers over the Tauri command + event bridge. No top-level Tauri
// calls here so the module is safe to import during (browserless) prerender.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Config, HookStatus, LocateStatus, PermDecision, Snapshot } from "./types";

export function currentLabel(): string {
  // Read synchronously from the injected window metadata; avoids pulling the
  // window API into the prerender import graph.
  try {
    // @ts-expect-error internal metadata, present under Tauri
    return window.__TAURI_INTERNALS__.metadata.currentWindow.label as string;
  } catch {
    return "avatar";
  }
}

export const getSnapshot = () => invoke<Snapshot>("get_snapshot");
export const getConfig = () => invoke<Config>("get_config");
export const setTheme = (theme: string) => invoke("set_theme", { theme });
export const setWarnPct = (pct: number) => invoke("set_warn_pct", { pct });
export const setCancelGraceMins = (mins: number) => invoke("set_cancel_grace_mins", { mins });
export const setClearWaitingMins = (mins: number) => invoke("set_clear_waiting_mins", { mins });
export const setWatchCodex = (enabled: boolean) => invoke("set_watch_codex", { enabled });
export const setAerospaceFollow = (enabled: boolean) =>
  invoke("set_aerospace_follow", { enabled });
export const setPanelShortcutEnabled = (enabled: boolean) =>
  invoke("set_panel_shortcut_enabled", { enabled });
export const togglePanel = () => invoke("toggle_panel");
// Close the panel AND hand focus back to the app that had it before the panel opened.
export const hidePanel = () => invoke("hide_panel");
export const openDashboard = () => invoke("open_dashboard");
// Hide whichever window this webview lives in (Esc dismiss). The panel's blur
// handler then records the dismiss so a follow-up avatar tap doesn't re-open it.
export const hideCurrentWindow = () => getCurrentWindow().hide();
// Dashboard Esc: hide + drop Tablo back out of the Dock / Cmd+Tab switcher.
export const hideDashboard = () => invoke("hide_dashboard");
// Toast window: position next to the avatar + reveal / hide after its animation.
export const showToast = () => invoke("show_toast");
export const hideToast = () => invoke("hide_toast");
export const beginDrag = () => invoke<{ x: number; y: number }>("begin_drag");
export const moveAvatar = (x: number, y: number) =>
  invoke("move_avatar", { x: Math.round(x), y: Math.round(y) });
export const endDrag = (x: number, y: number) =>
  invoke("end_drag", { x: Math.round(x), y: Math.round(y) });

// ---- Phase 4: permissions ----
export const resolvePermission = (id: string, decision: PermDecision) =>
  invoke("resolve_permission", { id, decision });
export const hookStatus = () => invoke<HookStatus>("hook_status");
export const setHookEnabled = (enabled: boolean) =>
  invoke<HookStatus>("set_hook_enabled", { enabled });

// ---- window-render: jump to session ----
export const jumpToSession = (sessionId: string) =>
  invoke<string>("jump_to_session", { sessionId });
export const locateStatus = () => invoke<LocateStatus>("locate_status");
export const setLocateEnabled = (enabled: boolean) =>
  invoke<LocateStatus>("set_locate_enabled", { enabled });
// Codex "jump" — a separate hook installed into ~/.codex/hooks.json; shares the
// same jump engine (jumpToSession) and LocateStatus shape.
export const codexLocateStatus = () => invoke<LocateStatus>("codex_locate_status");
export const setCodexLocateEnabled = (enabled: boolean) =>
  invoke<LocateStatus>("set_codex_locate_enabled", { enabled });

export function onState(cb: (s: Snapshot) => void): Promise<UnlistenFn> {
  return listen<Snapshot>("state-update", (e) => cb(e.payload));
}

// Theme is app-wide: any window can flip it, and Rust rebroadcasts so all
// surfaces update live.
export function onTheme(cb: (theme: string) => void): Promise<UnlistenFn> {
  return listen<string>("theme-update", (e) => cb(e.payload));
}

// Fired when sessions cross from working → waiting; payload is the sessions'
// project + title. Drives the toast (toast window) and a one-shot avatar shake.
export type WaitingSession = { id: string; project: string; title: string | null; canJump: boolean };
export function onSessionWaiting(cb: (sessions: WaitingSession[]) => void): Promise<UnlistenFn> {
  return listen<WaitingSession[]>("session-waiting", (e) => cb(e.payload));
}
