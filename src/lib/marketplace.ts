import type { AgentProfile, AppIdentity, AppState, BundleState, CatalogComponent, CatalogItem, DeepLink, ItemStatus, TargetId } from "../ipc/schemas";

/** A space's name: `source.id`, 2–16 lowercase letters, digits, and single hyphens, starting with a letter. */
export const NAMESPACE_PATTERN = /^[a-z](?:[a-z0-9]|-(?=[a-z0-9])){1,15}$/;
/** A package or bundle id: 1–64 lowercase letters, digits, and single hyphens. */
export const ID_PATTERN = /^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9])){0,63}$/;

/** Lowercase letters, digits, and single hyphens, accents dropped, at most `max` characters. */
function slug(text: string, max: number): string {
  return text
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+/, "")
    .slice(0, max)
    .replace(/-+$/, "");
}

/** A team's space name from its display name, so most people never type one: "Data Team" → "data-team". */
export function suggestNamespace(displayName: string): string {
  return slug(slug(displayName, 200).replace(/^[^a-z]+/, ""), 16);
}

/** A bundle id from its name: "Starter kit" → "starter-kit". */
export function suggestId(name: string): string {
  return slug(name, 64);
}

/** A full account (`CORP\jane`, `jane@corp.example`) can be added even when the directory has never seen it. */
export function typedAccount(query: string): string | null {
  const text = query.trim();
  return /^[^\s\\@]+[\\@][^\s\\@]+$/.test(text) ? text : null;
}

/** Whether this person may change things in the space: their own, their teams', and any an admin may. */
export function ownsSpace(identity: AppIdentity | null, namespace: string): boolean {
  return identity !== null && (identity.admin || identity.namespaces.includes(namespace));
}

/** How a space reads in a picker: "My space" for one's own, the team's name, or the namespace itself. */
export function spaceLabel(identity: AppIdentity | null, namespace: string): string {
  if (identity?.namespace === namespace) {
    return "My space";
  }
  if (namespace === "official") {
    return "Official";
  }
  return identity?.teams.find((team) => team.namespace === namespace)?.displayName ?? namespace;
}

/** "2 skills", "1 connector": what a package's parts are, in plain words. */
export function partCounts(components: readonly CatalogComponent[]): readonly string[] {
  const skills = components.filter((component) => component.kind === "skill").length;
  const servers = components.filter((component) => component.kind === "mcpServer").length;
  const parts: string[] = [];
  if (skills > 0) {
    parts.push(`${String(skills)} skill${skills === 1 ? "" : "s"}`);
  }
  if (servers > 0) {
    parts.push(`${String(servers)} connector${servers === 1 ? "" : "s"}`);
  }
  return parts;
}

/** "A", "A and B", "A, B and C". */
export function listPhrase(names: readonly string[]): string {
  return names.length <= 1 ? (names[0] ?? "") : `${names.slice(0, -1).join(", ")} and ${names.at(-1) ?? ""}`;
}

function supported(level: string): boolean {
  return level === "native" || level === "losslessTranslation" || level === "lossyTranslation";
}

/** The apps here that get `item` (or one of its parts), by display name, in the window's app order. */
export function appsFor(item: CatalogItem, profiles: readonly AgentProfile[], componentId?: string | null): readonly string[] {
  const targets = new Set(
    item.compatibility
      .filter((report) => (componentId ?? null) === null || report.componentId === componentId)
      .filter((report) => supported(report.capability.level))
      .map((report) => report.targetId)
  );
  return profiles.filter((profile) => targets.has(profile.targetId)).map((profile) => profile.displayName);
}

/** The AI apps an install lands in, as a phrase. */
export function appsPhrase(apps: readonly string[]): string {
  return apps.length === 0 ? "the AI apps Agent Plugins finds" : listPhrase(apps);
}

/**
 * Why none of the apps here can take `item`, in the first app's own words,
 * or null when at least one can (or nothing was planned yet).
 */
export function unusableReason(item: CatalogItem): string | null {
  if (item.compatibility.length === 0 || item.compatibility.some((report) => supported(report.capability.level))) {
    return null;
  }
  for (const report of item.compatibility) {
    if (report.capability.level === "unsupported" || report.capability.level === "blocked") {
      return report.capability.reason;
    }
  }
  return null;
}

/** How to use a newly installed skill in each app, where `name` is what the app lists it as. */
const USAGE: Readonly<Record<TargetId, (name: string) => string>> = {
  "github-copilot": () => "reload VS Code (or start a new Copilot CLI session); Copilot uses it when your request fits",
  cursor: (name) => `reload the window, then type /${name} or just ask`,
  "claude-code": (name) => `start a new session and type /${name}`,
  "claude-desktop": () => "quit and reopen it",
  opencode: () => "start a new session; it uses the skill when your request fits",
  pi: (name) => `start a new session and type /skill:${name}`,
  codex: (name) => `start a new session and type $${name}`,
  chatgpt: (name) => `switch to Codex and type $${name}`,
  "grok-build": () => "start a new session"
};

/**
 * One line per app that got the install: what to do there to use it. A
 * connector only needs its app restarted.
 */
export function usageLines(item: CatalogItem, profiles: readonly AgentProfile[], componentId?: string | null): readonly string[] {
  const parts = item.components.filter((component) => (componentId ?? null) === null || component.id === componentId);
  const skill = parts.find((component) => component.kind === "skill");
  const name = skill === undefined ? item.localId : `${item.sourceId}-${skill.id}`;
  const targets = new Set(
    item.compatibility
      .filter((report) => parts.some((component) => component.id === report.componentId))
      .filter((report) => supported(report.capability.level))
      .map((report) => report.targetId)
  );
  return profiles.filter((profile) => targets.has(profile.targetId)).map((profile) => `In ${profile.displayName}: ${skill === undefined ? "restart it to connect" : USAGE[profile.targetId](name)}.`);
}

/** Whether every word of `query` appears somewhere in `text`, ignoring case. */
export function matchesAllWords(text: string, query: string): boolean {
  const haystack = text.toLowerCase();
  return query
    .toLowerCase()
    .split(/\s+/)
    .filter((word) => word.length > 0)
    .every((word) => haystack.includes(word));
}

/** A page of the marketplace portal, such as `/p/ns/id`. */
export function portalUrl(marketplaceUrl: string, path: string): string {
  return `${marketplaceUrl.replace(/\/+$/, "")}${path}`;
}

/** What an install link may do with a package (or part) in `status`. A link never uninstalls or restores. */
export function linkAction(status: ItemStatus): "install" | "installed" | "blocked" {
  switch (status) {
    case "available":
    case "updateAvailable":
    case "partiallyInstalled":
    case "conflict":
      return "install";
    case "installed":
    case "removed":
    case "missing":
    case "modified":
      return "installed";
    case "sourceConflict":
      return "blocked";
  }
}

/** A bundle's members this window can show, in the bundle's order. */
export function bundleMembers(bundle: BundleState, items: readonly CatalogItem[]): readonly CatalogItem[] {
  return bundle.members.flatMap((id) => items.filter((item) => item.id === id));
}

function isInstalled(status: ItemStatus): boolean {
  return !(status === "available" || status === "conflict" || status === "sourceConflict");
}

/** How far a bundle is installed, and so which button its card shows. */
export type BundleSummary = Readonly<{ members: readonly CatalogItem[]; installed: number; missing: number; action: "install" | "installRest" | "installed" | "empty" }>;

export function bundleSummary(bundle: BundleState, items: readonly CatalogItem[]): BundleSummary {
  const members = bundleMembers(bundle, items);
  const installed = members.filter((item) => isInstalled(item.status)).length;
  const missing = bundle.members.length - members.length;
  let action: BundleSummary["action"] = "installRest";
  if (members.length === 0) {
    action = "empty";
  } else if (installed === 0) {
    action = "install";
  } else if (installed === members.length) {
    action = "installed";
  }
  return { members, installed, missing, action };
}

/** What a link points at in this window's catalog, or null when it isn't here (yet, or for this person). */
export type LinkTarget = Readonly<{ kind: "item"; item: CatalogItem; componentId: string | null }> | Readonly<{ kind: "bundle"; bundle: BundleState; members: readonly CatalogItem[] }>;

export function resolveLink(link: DeepLink, state: AppState): LinkTarget | null {
  const id = `${link.namespace}/${link.id}`;
  const item = state.items.find((candidate) => candidate.id === id);
  if (item !== undefined) {
    if (link.component === null) {
      return { kind: "item", item, componentId: null };
    }
    return item.components.some((component) => component.id === link.component) ? { kind: "item", item, componentId: link.component } : null;
  }
  const bundle = state.bundles.find((candidate) => candidate.id === id);
  return bundle === undefined || link.component !== null ? null : { kind: "bundle", bundle, members: bundleMembers(bundle, state.items) };
}

/** The element id a card carries, so a link can scroll to it. */
export function cardDomId(id: string): string {
  return `card-${id.replace(/[^a-z0-9-]/g, "_")}`;
}
