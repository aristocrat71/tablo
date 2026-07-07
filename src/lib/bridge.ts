// Thin wrappers over the Tauri command + event bridge. No top-level Tauri
// calls here so the module is safe to import during (browserless) prerender.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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
export const togglePanel = () => invoke("toggle_panel");
export const openDashboard = () => invoke("open_dashboard");
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

export function onState(cb: (s: Snapshot) => void): Promise<UnlistenFn> {
  return listen<Snapshot>("state-update", (e) => cb(e.payload));
}

// Theme is app-wide: any window can flip it, and Rust rebroadcasts so all
// surfaces update live.
export function onTheme(cb: (theme: string) => void): Promise<UnlistenFn> {
  return listen<string>("theme-update", (e) => cb(e.payload));
}
