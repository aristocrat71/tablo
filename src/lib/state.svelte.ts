// Shared reactive store (Svelte 5 runes). Every window fetches the current
// snapshot on mount, then subscribes to live `state-update` events.

import { EMPTY_SNAPSHOT, LOADING_CONFIG, type Snapshot } from "./types";
import { getConfig, getSnapshot, onState } from "./bridge";

export const store = $state({
  snap: EMPTY_SNAPSHOT as Snapshot,
  config: LOADING_CONFIG,
  ready: false,
});

export function applyTheme(theme: string) {
  document.documentElement.setAttribute("data-theme", theme);
}

let started = false;

export async function initStore() {
  if (started) return;
  started = true;
  try {
    store.config = await getConfig();
  } catch {
    /* keep defaults */
  }
  applyTheme(store.config.theme);
  try {
    store.snap = await getSnapshot();
  } catch {
    /* keep empty */
  }
  store.ready = true;
  onState((s) => {
    store.snap = s;
  }).catch(() => {});
}
