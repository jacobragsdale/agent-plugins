import type { AppState, PreflightCheck } from "../ipc/schemas";

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

/**
 * The one calm banner for a missing connection. None while the page itself
 * says it is offline (nothing saved yet), and none while any server answers:
 * a source that alone could not be reached says so with its own grey badge,
 * and the next check retries it without anyone doing anything.
 */
export function offlineBanner(state: AppState | null, offlineHint: boolean): string | null {
  if (state === null || !hasPackages(state) || !isOffline(state, offlineHint)) {
    return null;
  }
  const retry = "Agent Plugins will retry automatically.";
  return state.checkedAtEpochSeconds === 0 ? `Offline — showing saved skills. ${retry}` : `Offline — showing skills as of ${formatEpoch(state.checkedAtEpochSeconds)}. ${retry}`;
}

/** Checks that fail whenever the machine is offline; the offline banner already says so. */
const OFFLINE_CHECKS: readonly string[] = ["server.health", "host.dns"];

/**
 * The failures worth a line under the header: not the ones the offline
 * banner already explains, and one per cause, so an untrusted certificate
 * (`host.tls`) is not counted again as an unreachable server.
 */
export function headerProblems(problems: readonly PreflightCheck[], offline: boolean): readonly PreflightCheck[] {
  const certificate = problems.some((check) => check.id === "host.tls");
  return problems.filter((check) => !(offline && OFFLINE_CHECKS.includes(check.id)) && !(certificate && check.id === "server.health"));
}

export const OFFLINE_EMPTY = "You're offline, and no skills are saved on this computer yet. They'll appear here once Agent Plugins can reach the server. It will keep trying on its own.";
export const NOTHING_PUBLISHED = "No skills published yet.";

/** Why a search or the local-changes filter shows nothing, or null when it shows something (or no filter is on). */
export function noMatchesText(query: string, driftOnly: boolean, shown: number): string | null {
  const needle = query.trim();
  if (shown > 0 || (needle.length === 0 && !driftOnly)) {
    return null;
  }
  if (needle.length === 0) {
    return "No skills have local changes.";
  }
  return driftOnly ? `No skills with local changes match “${needle}”.` : `No skills match “${needle}”.`;
}

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
