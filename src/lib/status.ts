import { confirm } from "@tauri-apps/plugin-dialog";
import type { AppError } from "../ipc/client";
import type { AgentProfile, AppState, BulkAction, BulkPlan, CatalogItem, ItemStatus, OperationOutcome, RepositoryState, SourceRemovalPlan, SourceState } from "../ipc/schemas";
import type { InfoNotice } from "../components/Notice";

export type AccentColor = "amber" | "blue" | "gray" | "green" | "red";

// The status values come from the backend and stay as they are; these labels are the plain words the window shows.
export function statusLabel(status: ItemStatus): string {
  switch (status) {
    case "available":
      return "Available";
    case "installed":
      return "Installed";
    case "updateAvailable":
      return "Update available";
    case "removed":
      return "No longer offered";
    case "modified":
      return "Changed on this computer";
    case "conflict":
      return "Files already there";
    case "sourceConflict":
      return "From another source";
    case "partiallyInstalled":
      return "Partly installed";
    case "missing":
      return "Restoring on next check";
  }
}

export function statusColor(status: ItemStatus): AccentColor {
  switch (status) {
    case "installed":
      return "green";
    case "updateAvailable":
      return "blue";
    case "available":
    case "missing":
      return "gray";
    case "removed":
    case "conflict":
    case "sourceConflict":
      return "amber";
    case "partiallyInstalled":
      return "amber";
    case "modified":
      return "red";
  }
}

export function primaryActionLabel(status: ItemStatus): string {
  switch (status) {
    case "available":
      return "Install";
    case "updateAvailable":
    case "partiallyInstalled":
      return "Update";
    case "installed":
    case "removed":
    case "missing":
      return "Uninstall";
    case "conflict":
      return "Replace…";
    case "modified":
      return "Restore original…";
    case "sourceConflict":
      return "From another source";
  }
}

export function primaryActionColor(status: ItemStatus): AccentColor {
  switch (status) {
    case "available":
    case "updateAvailable":
    case "partiallyInstalled":
      return "green";
    case "installed":
    case "removed":
    case "missing":
      return "red";
    case "conflict":
    case "modified":
      return "amber";
    case "sourceConflict":
      return "gray";
  }
}

/** The badge for a component kind, or null for one a person has no word for (a package no longer offered records none). */
export function componentLabel(kind: string): string | null {
  switch (kind) {
    case "skill":
      return "Skill";
    case "mcpServer":
      return "Connector";
    default:
      return null;
  }
}

export function itemCommandArgs(item: CatalogItem, componentId: string | undefined, extra: Record<string, unknown>): Record<string, unknown> {
  return componentId === undefined ? { sourceId: item.sourceId, localId: item.localId, ...extra } : { sourceId: item.sourceId, localId: item.localId, componentId, ...extra };
}

/** A changed package ("modified") is restored by replacing it: the changed copy is backed up first. */
export function commandForStatus(status: ItemStatus): "install_item" | "replace_item" | "uninstall_item" | null {
  if (status === "sourceConflict") {
    return null;
  }
  if (status === "conflict" || status === "modified") {
    return "replace_item";
  }
  if (status === "installed" || status === "removed" || status === "missing") {
    return "uninstall_item";
  }
  return "install_item";
}

const COMMAND_VERBS = { install_item: "install", replace_item: "replace", uninstall_item: "uninstall" } as const;

/** What clicking a package (or one of its parts) does: the command, the confirmation first, and the words around it. */
export type ItemCommand = Readonly<{ command: "install_item" | "replace_item" | "uninstall_item"; verb: string; review: (() => Promise<boolean>) | null; backupLead: string; trustApproved: boolean }>;

export function itemCommand(item: CatalogItem, componentId: string | undefined): ItemCommand | null {
  const component = componentId === undefined ? item : item.components.find((entry) => entry.id === componentId);
  const command = component === undefined ? null : commandForStatus(component.status);
  if (component === undefined || command === null) {
    return null;
  }
  // Restoring a changed package is a replace: the changed copy is backed up first.
  const restoring = component.status === "modified";
  let review: ItemCommand["review"] = null;
  if (command === "replace_item") {
    review = restoring ? async () => reviewRestore(item.name) : reviewReplace;
  } else if (component.status === "removed") {
    review = async () => reviewRemovedUninstall(item.name);
  }
  return {
    command,
    verb: restoring ? "restore" : COMMAND_VERBS[command],
    review,
    backupLead: restoring ? "Your changed copy was backed up to" : "The files that were there before were backed up to",
    trustApproved: command !== "uninstall_item" && component.requiresApproval
  };
}

export async function reviewRestore(name: string): Promise<boolean> {
  return confirm(`Put back the original ${name}? Your changed copy is kept as a backup, and Agent Plugins shows you where after it restores the original.`, {
    title: "Restore original",
    kind: "warning",
    okLabel: "Back up and restore",
    cancelLabel: "Cancel"
  });
}

// Nothing offers a "no longer offered" package any more, so once uninstalled it cannot come back.
export async function reviewRemovedUninstall(name: string): Promise<boolean> {
  return confirm(`${name} is no longer offered by its source, so you won't be able to install it again. Uninstall?`, {
    title: "Uninstall",
    kind: "warning",
    okLabel: "Uninstall",
    cancelLabel: "Cancel"
  });
}

export async function reviewReplace(): Promise<boolean> {
  return confirm("Files that Agent Plugins did not install are already in the way. They will be backed up, then replaced.", {
    title: "Replace",
    kind: "warning",
    okLabel: "Back up and replace",
    cancelLabel: "Cancel"
  });
}

export function bulkLabels(action: BulkAction): Readonly<{ action: string; title: string; button: string; warning: string }> {
  switch (action) {
    case "install":
      return { action: "Install or update", title: "Install all", button: "Install", warning: "" };
    case "replace":
      return { action: "Replace", title: "Replace all", button: "Replace", warning: " The files already there will be backed up first." };
    case "uninstall":
      return { action: "Uninstall", title: "Uninstall all", button: "Uninstall", warning: "" };
  }
}

// A connector (an MCP server) is a program an AI app starts on this machine, or
// an online service it sends requests to, so the app asks before it installs one.
export async function reviewApproval(name: string, riskDetails: readonly string[]): Promise<boolean> {
  const detail = riskDetails.length === 0 ? "" : `\n\n${riskDetails.join("\n")}`;
  return confirm(`${name} includes a connector, a program or online service that AI apps use. Every AI app found here will use it.${detail}`, {
    title: "Allow connector",
    kind: "warning",
    okLabel: "Allow and install",
    cancelLabel: "Cancel"
  });
}

export async function reviewBulkApproval(names: readonly string[], riskDetails: readonly string[]): Promise<boolean> {
  const detail = riskDetails.length === 0 ? "" : `\n\n${riskDetails.join("\n")}`;
  return confirm(`${names.join(", ")} include connectors, programs or online services that AI apps use. Every AI app found here will use them.${detail}`, {
    title: "Allow connectors",
    kind: "warning",
    okLabel: "Allow and install",
    cancelLabel: "Cancel"
  });
}

// Each sync adds back every source the catalog lists (the app only ever adds the default catalog), but not its packages.
export async function reviewSourceRemoval(source: SourceState, plan: SourceRemovalPlan, repositories: readonly RepositoryState[]): Promise<boolean> {
  const count = plan.items.length;
  const modified = plan.items.flatMap((item) => item.paths).filter((path) => path.modified);
  const warning =
    modified.length === 0
      ? ""
      : ` ${String(modified.length)} file${modified.length === 1 ? " has" : "s have"} local changes that were not made by Agent Plugins: ${modified.map((path) => path.path).join(", ")}.`;
  const listed = repositories.some((repository) => repository.sources.some((entry) => entry.url === source.url || entry.sourceId === source.sourceId));
  const returns = listed ? " The catalog still lists it, so it will be added back on the next check. Its packages will not be reinstalled." : "";
  const question = count === 0 ? `Remove ${source.name}?` : `Remove ${source.name} and uninstall ${String(count)} package${count === 1 ? "" : "s"} it installed?`;
  return confirm(`${question}${warning}${returns}`, { title: "Remove source", kind: "warning", okLabel: modified.length === 0 ? "Remove" : "Remove and discard changes", cancelLabel: "Cancel" });
}

export async function reviewReset(): Promise<boolean> {
  return confirm("Uninstall every package and delete all Agent Plugins data? Sources from the catalog are added back automatically.", {
    title: "Reset",
    kind: "warning",
    okLabel: "Reset",
    cancelLabel: "Cancel"
  });
}

// Closing the app is the one thing here that can cost the person work, so they agree to it first.
export async function reviewTutorial(app: string): Promise<boolean> {
  return confirm(`This closes ${app} if it's open, adds a small tutorial skill, then reopens ${app} with a prompt that uses it. Save your work in ${app} first.`, {
    title: "Try a skill",
    kind: "info",
    okLabel: `Close and reopen ${app}`,
    cancelLabel: "Cancel"
  });
}

export async function reviewBulk(source: SourceState, action: BulkAction, plan: BulkPlan): Promise<boolean> {
  const eligible = plan.entries.filter((entry) => entry.willRun);
  const labels = bulkLabels(action);
  return confirm(`${labels.action} ${String(eligible.length)} package${eligible.length === 1 ? "" : "s"} from ${source.name}?${labels.warning}`, {
    title: labels.title,
    kind: action === "install" ? "info" : "warning",
    okLabel: labels.button,
    cancelLabel: "Cancel"
  });
}

export function supportsBulkAction(status: ItemStatus, action: BulkAction): boolean {
  switch (action) {
    case "install":
      return status === "available" || status === "updateAvailable" || status === "partiallyInstalled";
    case "replace":
      return status === "conflict";
    case "uninstall":
      return status === "installed" || status === "updateAvailable" || status === "partiallyInstalled" || status === "missing";
  }
}

/** An enabled agent that was not detected but left a message: detection itself was inconclusive, so it is not "not installed". */
export function detectionUnsure(profile: AgentProfile): boolean {
  return !profile.detected && profile.enabled && profile.detectionMessage !== null;
}

/** Whether an AI app is (or may be) here; an inconclusive check does not count as "none found". */
export function hasDetectedAgent(profiles: readonly AgentProfile[]): boolean {
  return profiles.some((profile) => profile.detected || detectionUnsure(profile));
}

/** A package's display name, or its id when the window no longer lists it. */
export function packageName(items: readonly CatalogItem[], id: string): string {
  return items.find((item) => item.id === id)?.name ?? id;
}

/**
 * What a background sync did that is worth a line: the packages it updated.
 * Files it put back, apps it added packages to, and updates it will retry
 * happen quietly; a person has nothing to do about them.
 */
export type ReportNotice = Readonly<{ text: string }>;

export function reportNotice(state: AppState): ReportNotice | null {
  const { updatedItems } = state.autoUpdateReport;
  return updatedItems.length === 0 ? null : { text: `Updated ${updatedItems.map((item) => packageName(state.items, item.id)).join(", ")}.` };
}

/** A batch that partly failed, summarized by package name with each raw reason behind "Details". */
export function failuresError(verb: string, failures: readonly Readonly<{ id: string; message: string }>[], items: readonly CatalogItem[], sourceName?: string): AppError {
  const names = failures.map((failure) => packageName(items, failure.id));
  const count = `${String(failures.length)} package${failures.length === 1 ? "" : "s"}`;
  const from = sourceName === undefined ? "" : ` from ${sourceName}`;
  return { kind: null, summary: `Couldn't ${verb} ${count}${from}: ${names.join(", ")}.`, detail: failures.map((failure) => `${failure.id}: ${failure.message}`).join("\n") };
}

/** What a finished action leaves to read: AI apps it skipped (amber), then where backups went. */
export function outcomeNotice(backupLead: string, outcome: OperationOutcome): InfoNotice | null {
  const parts = [...outcome.warnings];
  if (outcome.backupPaths.length > 0) {
    parts.push(`${backupLead} ${outcome.backupPaths.join(", ")}.`);
  }
  return parts.length === 0 ? null : { text: parts.join(" "), folder: outcome.backupPaths[0] ?? null, caution: outcome.warnings.length > 0 };
}
