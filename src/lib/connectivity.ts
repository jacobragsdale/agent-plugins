import type { AppState } from "../ipc/schemas";

export function formatEpoch(epochSeconds: number): string {
  return new Date(epochSeconds * 1000).toLocaleString();
}

/** "Last checked" is the last sync that reached the servers; 0 means none has yet. */
export function lastCheckedLabel(state: AppState | null): string {
  return state === null || state.checkedAtEpochSeconds === 0 ? "Not yet" : formatEpoch(state.checkedAtEpochSeconds);
}

/** When a source or catalog last loaded; its saved copy is what the window shows until the next good check. */
export function savedCopyLabel(lastSuccessAtEpochSeconds: number | null): string {
  return lastSuccessAtEpochSeconds === null ? "Saved copy" : `Saved copy from ${formatEpoch(lastSuccessAtEpochSeconds)}`;
}

/** A sync is running, started here or by the backend on its own. */
export function isChecking(state: AppState | null, syncing: boolean): boolean {
  return syncing || state?.syncInProgress === true;
}

/** Offline as far as the window knows: the last sync said so, or an action just failed for lack of a connection. */
export function isOffline(state: AppState | null, offlineHint: boolean): boolean {
  return offlineHint || state?.connectivity === "offline";
}

function hasPackages(state: AppState): boolean {
  return state.sources.length > 0 || state.items.length > 0;
}

/** The names of the sources and catalogs the last sync couldn't reach. */
export function unreachableNames(state: AppState): readonly string[] {
  return [...state.repositories, ...state.sources].filter((entry) => entry.status === "stale").map((entry) => entry.name);
}

/**
 * The one calm banner for a missing connection. None while the page itself
 * says it is offline (nothing saved yet), and none when every server answered.
 */
export function offlineBanner(state: AppState | null, offlineHint: boolean): string | null {
  if (state === null || !hasPackages(state)) {
    return null;
  }
  const retry = "Agent Plugins will retry automatically.";
  if (isOffline(state, offlineHint)) {
    return state.checkedAtEpochSeconds === 0 ? `Offline — showing saved packages. ${retry}` : `Offline — showing packages as of ${formatEpoch(state.checkedAtEpochSeconds)}. ${retry}`;
  }
  if (state.connectivity === "degraded") {
    const names = unreachableNames(state);
    return `Couldn't reach ${names.length === 0 ? "some sources" : names.join(", ")}, so their saved copy is shown. ${retry}`;
  }
  return null;
}

export const OFFLINE_EMPTY = "You're offline, and no packages are saved on this computer yet. They'll appear here once Agent Plugins can reach the server. It will keep trying on its own.";
export const NOTHING_PUBLISHED = "No packages published yet.";

/** What fills the page: the spinner, a sentence saying why it is empty, or the package list. */
export type CatalogBody = Readonly<{ kind: "loading" }> | Readonly<{ kind: "empty"; text: string; offline: boolean }> | Readonly<{ kind: "list" }>;

export function catalogBody(state: AppState | null, offlineHint: boolean, filtering: boolean): CatalogBody {
  if (state === null) {
    return offlineHint ? { kind: "empty", text: OFFLINE_EMPTY, offline: true } : { kind: "loading" };
  }
  if (filtering || hasPackages(state)) {
    return { kind: "list" };
  }
  if (isOffline(state, offlineHint)) {
    return { kind: "empty", text: OFFLINE_EMPTY, offline: true };
  }
  return state.syncInProgress ? { kind: "loading" } : { kind: "empty", text: NOTHING_PUBLISHED, offline: false };
}
