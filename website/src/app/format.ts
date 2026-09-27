// Pure helpers shared by the pages. Everything here is covered by format.spec.ts.

export type Bump = "patch" | "minor" | "major";

type SemVer = readonly [number, number, number, string | null];

const semverPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z][0-9A-Za-z.-]*))?$/u;

function parseSemVer(text: string): SemVer | null {
  const match = semverPattern.exec(text);
  return match === null ? null : [Number(match[1]), Number(match[2]), Number(match[3]), match[4] ?? null];
}

function compare(left: SemVer, right: SemVer): number {
  if (left[0] !== right[0]) {
    return left[0] - right[0];
  }

  if (left[1] !== right[1] || left[2] !== right[2]) {
    return left[1] !== right[1] ? left[1] - right[1] : left[2] - right[2];
  }

  if (left[3] === right[3]) {
    return 0;
  }

  // ponytail: numeric-aware string order, close to but not exactly semver's per-identifier precedence.
  return left[3] === null ? 1 : right[3] === null ? -1 : left[3].localeCompare(right[3], "en", { numeric: true });
}

/**
 * The next version after the highest one ever published, including pending, rejected, and withdrawn
 * versions: the server refuses a number that was used before.
 */
export function nextVersion(existing: readonly string[], bump: Bump): string {
  const highest = existing
    .map(parseSemVer)
    .filter((version): version is SemVer => version !== null)
    .reduce<SemVer | null>((best, version) => (best === null || compare(version, best) > 0 ? version : best), null);
  if (highest === null) {
    return "1.0.0";
  }

  const [major, minor, patch] = highest;
  switch (bump) {
    case "major":
      return `${String(major + 1)}.0.0`;
    case "minor":
      return `${String(major)}.${String(minor + 1)}.0`;
    case "patch":
      return `${String(major)}.${String(minor)}.${String(patch + 1)}`;
  }
}

/** Whether `version` is at least `minimum`; anything that is not semver is too old. */
export function versionAtLeast(version: string, minimum: string): boolean {
  const have = parseSemVer(version);
  const need = parseSemVer(minimum);
  return have !== null && need !== null && compare(have, need) >= 0;
}

/** The first desktop app that opens `agent-plugins://` links from this site. */
export const linkMinimumVersion = "0.2.0";

export type InstallState = "install" | "installed" | "get" | "update";

/**
 * What an install button offers, from the caller's desktop app as its last check-in reported it.
 * `app` is undefined while unknown (not signed in yet); `covers` are the packages that must all be
 * installed for it to count as installed (empty for one skill of a pack, which the app cannot report).
 */
export function installState(app: { readonly version: string; readonly installed: readonly string[] } | null | undefined, covers: readonly string[]): InstallState {
  if (app === undefined) {
    return "install";
  }

  if (app === null) {
    return "get";
  }

  if (!versionAtLeast(app.version, linkMinimumVersion)) {
    return "update";
  }

  return covers.length > 0 && covers.every((id) => app.installed.includes(id)) ? "installed" : "install";
}

/** The desktop app's link: `install` a package, bundle, or one pack skill (`ns/pkg/skill`), or `open` one. */
export function appLink(verb: "install" | "open", target: string): string {
  return `agent-plugins://${verb}/${target}`;
}

/** Namespaces: 2–16 lowercase letters, digits, and single hyphens, starting with a letter. */
export const namespacePattern = /^[a-z](?:[a-z0-9]|-(?=[a-z0-9])){1,15}$/u;

/** A team's short name from its display name: "Data Engineering" becomes "data-engineering". */
export function suggestNamespace(displayName: string): string {
  const slug = slugify(displayName).slice(0, 16).replace(/-+$/u, "");
  const lettered = /^[a-z]/u.test(slug) ? slug : `t-${slug}`.slice(0, 16).replace(/-+$/u, "");
  return namespacePattern.test(lettered) ? lettered : "";
}

/** Sorts version strings newest first; anything that is not semver sorts last. */
export function newestFirst(versions: readonly string[]): string[] {
  return [...versions].sort((left, right) => {
    const a = parseSemVer(left);
    const b = parseSemVer(right);
    if (a === null || b === null) {
      return (a === null ? 1 : 0) - (b === null ? 1 : 0);
    }

    return compare(b, a);
  });
}

/** Package and skill IDs: lowercase letters, digits, and single hyphens, at most 64 characters. */
export const idPattern = /^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9])){0,63}$/u;

export function slugify(text: string): string {
  return text
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/gu, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/gu, "-")
    .replace(/^-+|-+$/gu, "")
    .slice(0, 64)
    .replace(/-+$/u, "");
}

export function titleCase(id: string): string {
  return id
    .split("-")
    .filter((part) => part.length > 0)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export interface SkillText {
  readonly name: string;
  readonly description: string;
  readonly body: string;
}

interface Frontmatter {
  readonly lines: readonly string[];
  readonly body: string;
}

function splitFrontmatter(text: string): Frontmatter | null {
  const normalized = text.replace(/^\uFEFF/u, "").replace(/\r\n/gu, "\n");
  const lines = normalized.split("\n");
  if (lines[0]?.trim() !== "---") {
    return null;
  }

  const end = lines.findIndex((line, index) => index > 0 && line.trim() === "---");
  if (end < 0) {
    return null;
  }

  return { lines: lines.slice(1, end), body: lines.slice(end + 1).join("\n") };
}

/** The lines a top-level key occupies: its own line plus any indented continuation lines. */
function keyExtent(lines: readonly string[], key: string): { readonly start: number; readonly end: number } | null {
  const start = lines.findIndex((line) => line.startsWith(`${key}:`));
  if (start < 0) {
    return null;
  }

  let end = start + 1;
  while (end < lines.length && /^\s+\S/u.test(lines[end] ?? "")) {
    end += 1;
  }

  return { start, end };
}

function scalar(lines: readonly string[]): string {
  const [first = "", ...rest] = lines;
  const value = first.slice(first.indexOf(":") + 1).trim();
  if (value.startsWith(">") || value.startsWith("|")) {
    const joined = rest.map((line) => line.trim());
    return (value.startsWith(">") ? joined.join(" ") : joined.join("\n")).trim();
  }

  if (value.startsWith('"')) {
    try {
      const parsed: unknown = JSON.parse(value);
      return typeof parsed === "string" ? parsed : value;
    } catch {
      return value.slice(1, -1);
    }
  }

  if (value.startsWith("'") && value.endsWith("'")) {
    return value.slice(1, -1).replace(/''/gu, "'");
  }

  return [value, ...rest.map((line) => line.trim())].join(" ").trim();
}

/** Reads `name`, `description`, and the body of a SKILL.md, or null without frontmatter. */
export function parseSkillMd(text: string): SkillText | null {
  const parts = splitFrontmatter(text);
  if (parts === null) {
    return null;
  }

  const field = (key: string): string => {
    const extent = keyExtent(parts.lines, key);
    return extent === null ? "" : scalar(parts.lines.slice(extent.start, extent.end));
  };
  return { name: field("name"), description: field("description"), body: parts.body.trim() };
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${String(bytes)} B`;
  }

  const kilobytes = bytes / 1024;
  if (kilobytes < 1024) {
    return `${kilobytes < 10 ? kilobytes.toFixed(1) : kilobytes.toFixed(0)} KB`;
  }

  const megabytes = kilobytes / 1024;
  return `${megabytes < 10 ? megabytes.toFixed(1) : megabytes.toFixed(0)} MB`;
}

const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium" });
const relativeFormat = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

export function formatDate(iso: string): string {
  return dateFormat.format(new Date(iso));
}

/** "3 days ago" for the last month, then the date. */
export function formatAge(iso: string, now: Date = new Date()): string {
  const seconds = (new Date(iso).getTime() - now.getTime()) / 1000;
  const units: readonly (readonly [Intl.RelativeTimeFormatUnit, number])[] = [
    ["day", 86_400],
    ["hour", 3_600],
    ["minute", 60]
  ];
  if (Math.abs(seconds) >= 30 * 86_400) {
    return formatDate(iso);
  }

  for (const [unit, size] of units) {
    if (Math.abs(seconds) >= size) {
      return relativeFormat.format(Math.round(seconds / size), unit);
    }
  }

  return "just now";
}

/** Files a browser picks up that never belong in a package. */
const junkFolders = new Set([".git", "node_modules", "__pycache__", ".venv", ".ruff_cache", ".pytest_cache", ".mypy_cache"]);
const junkFiles = new Set([".DS_Store", "Thumbs.db", "desktop.ini"]);

export function isJunk(path: string): boolean {
  const parts = path.split("/");
  const name = parts.at(-1) ?? "";
  return parts.some((part) => junkFolders.has(part)) || junkFiles.has(name) || name.endsWith(".pyc");
}

export interface TreeFile<T> {
  readonly name: string;
  readonly item: T;
}

export interface FileTree<T> {
  readonly name: string;
  /** The folder's full path with a trailing slash, so it prefixes every file below it. */
  readonly path: string;
  readonly files: readonly TreeFile<T>[];
  readonly folders: readonly FileTree<T>[];
}

interface Branch<T> {
  readonly files: TreeFile<T>[];
  readonly folders: Map<string, Branch<T>>;
}

/** SKILL.md sorts first, then everything by name. */
const sortKey = (name: string): string => (name === "SKILL.md" ? "" : name);

/** Nests files under their folders below `root`, at any depth. */
export function fileTree<T extends { readonly path: string }>(files: readonly T[], root = ""): FileTree<T> {
  const top: Branch<T> = { files: [], folders: new Map() };
  for (const item of files) {
    const parts = item.path.slice(root.length).split("/");
    const name = parts.pop() ?? "";
    let branch = top;
    for (const part of parts) {
      const next = branch.folders.get(part) ?? { files: [], folders: new Map<string, Branch<T>>() };
      branch.folders.set(part, next);
      branch = next;
    }

    branch.files.push({ name, item });
  }

  const freeze = (branch: Branch<T>, name: string, path: string): FileTree<T> => ({
    name,
    path,
    files: branch.files.sort((left, right) => sortKey(left.name).localeCompare(sortKey(right.name))),
    folders: [...branch.folders].sort(([left], [right]) => left.localeCompare(right)).map(([child, next]) => freeze(next, child, `${path}${child}/`))
  });
  return freeze(top, "", root);
}

export interface SkillGroup<T> {
  /** The skill's folder with a trailing slash ("" for a SKILL.md at the top), or null for files in no skill. */
  readonly root: string | null;
  readonly name: string;
  readonly files: readonly T[];
  readonly tree: FileTree<T>;
}

/** Groups files by the folder of the nearest SKILL.md above them, skills by name and other files last. */
export function groupBySkill<T extends { readonly path: string }>(files: readonly T[]): SkillGroup<T>[] {
  const roots = files
    .filter((file) => file.path === "SKILL.md" || file.path.endsWith("/SKILL.md"))
    .map((file) => file.path.slice(0, -"SKILL.md".length))
    .sort((left, right) => right.length - left.length);
  const groups = new Map<string | null, T[]>();
  for (const file of files) {
    const root = roots.find((candidate) => file.path.startsWith(candidate)) ?? null;
    groups.set(root, [...(groups.get(root) ?? []), file]);
  }

  return [...groups]
    .map(([root, grouped]) => ({ root, name: root === null ? "Other files" : (root.split("/").at(-2) ?? "skill"), files: grouped, tree: fileTree(grouped, root ?? "") }))
    .sort((left, right) => ((left.root === null) === (right.root === null) ? left.name.localeCompare(right.name) : left.root === null ? 1 : -1));
}

/** What each AI app takes from a package. Primary apps come first; skills go to every app but Claude Desktop. */
const apps: readonly { readonly name: string; readonly primary: boolean; readonly skills: string | null; readonly mcp: readonly string[]; readonly noMcp?: string }[] = [
  { name: "GitHub Copilot", primary: true, skills: null, mcp: ["stdio", "streamable-http", "sse"] },
  { name: "Cursor", primary: true, skills: null, mcp: ["stdio", "streamable-http", "sse"] },
  { name: "Claude Code", primary: true, skills: null, mcp: ["stdio", "streamable-http", "sse"] },
  { name: "Claude Desktop", primary: true, skills: "Skills come from your claude.ai account (Customize > Skills).", mcp: ["stdio"] },
  { name: "OpenCode", primary: false, skills: null, mcp: ["stdio", "streamable-http"] },
  { name: "pi", primary: false, skills: null, mcp: [], noMcp: "pi doesn't use MCP servers." },
  { name: "Codex and the ChatGPT app", primary: false, skills: null, mcp: ["stdio", "streamable-http"] },
  { name: "Grok Build", primary: false, skills: null, mcp: ["stdio", "streamable-http"] }
];

export interface WorksIn {
  readonly app: string;
  readonly primary: boolean;
  readonly works: "yes" | "some" | "no";
  /** Why an app gets less than the whole package. */
  readonly note: string | null;
}

/**
 * Which apps get what from a package with these component kinds and MCP transports. Unknown
 * transports (an empty list) count as working wherever the app takes MCP servers at all.
 */
export function worksIn(kinds: readonly string[], transports: readonly string[]): WorksIn[] {
  const wantsSkill = kinds.includes("skill");
  const wantsMcp = kinds.includes("mcpServer");
  return apps.map((app) => {
    const notes: string[] = [];
    let parts = 0;
    let got = 0;
    if (wantsSkill) {
      parts += 1;
      if (app.skills === null) {
        got += 1;
      } else {
        notes.push(app.skills);
      }
    }

    if (wantsMcp) {
      parts += 1;
      const missing = transports.filter((transport) => !app.mcp.includes(transport));
      if (app.mcp.length > 0 && missing.length === 0) {
        got += 1;
      } else {
        notes.push(app.noMcp ?? (app.mcp.includes("stdio") && app.mcp.length === 1 ? "It only runs MCP servers on this PC, not online ones." : "It can't use this kind of MCP server."));
      }
    }

    return { app: app.name, primary: app.primary, works: got === parts ? "yes" : got === 0 ? "no" : "some", note: notes.length === 0 ? null : notes.join(" ") };
  });
}

/** A card's short warning when a main app gets nothing from the package, such as "Not in Claude Desktop". */
export function worksInNote(kinds: readonly string[], transports: readonly string[]): string | null {
  const missing = worksIn(kinds, transports)
    .filter((row) => row.primary && row.works === "no")
    .map((row) => row.app);
  return missing.length === 0 ? null : `Not in ${missing.join(" or ")}`;
}

/**
 * A new package's ID from its name: installed skills are named `<space>-<id>` in at most 64 characters,
 * so a name that already starts with the space drops it, and the ID leaves room for the prefix.
 */
export function packageIdFor(name: string, space: string): string {
  const slug = slugify(name);
  const bare = slug.startsWith(`${space}-`) ? slug.slice(space.length + 1) : slug;
  return bare.slice(0, Math.max(1, 63 - space.length)).replace(/-+$/u, "");
}

/** A SKILL.md from what someone typed: the description is quoted, so a colon or a quote in it stays valid YAML. */
export function skillMd(skill: SkillText): string {
  return `---\nname: ${skill.name}\ndescription: ${JSON.stringify(skill.description.replace(/\s+/gu, " ").trim())}\n---\n\n${skill.body.trim()}\n`;
}

export type BrowseSort = "popular" | "new" | "updated";
export type BrowseLane = "all" | "official" | "team" | "personal";

export interface BrowseState {
  readonly q: string;
  readonly lane: BrowseLane;
  readonly sort: BrowseSort;
  readonly tag: string | null;
  readonly installed: boolean;
}

/** The Browse page's filters from its query string; anything unrecognised falls back to the default. */
export function parseBrowse(params: {
  readonly q?: string | undefined;
  readonly lane?: string | undefined;
  readonly sort?: string | undefined;
  readonly tag?: string | undefined;
  readonly installed?: string | undefined;
}): BrowseState {
  const lane = params.lane;
  const sort = params.sort;
  return {
    q: params.q ?? "",
    lane: lane === "official" || lane === "team" || lane === "personal" ? lane : "all",
    sort: sort === "new" || sort === "updated" ? sort : "popular",
    tag: params.tag !== undefined && params.tag.length > 0 ? params.tag : null,
    installed: params.installed === "true"
  };
}

/** The query string for a Browse state, leaving defaults out so links stay short. */
export function browseParams(state: BrowseState): Record<string, string | null> {
  return {
    q: state.q.length > 0 ? state.q : null,
    lane: state.lane === "all" ? null : state.lane,
    sort: state.sort === "popular" ? null : state.sort,
    tag: state.tag,
    installed: state.installed ? "true" : null
  };
}
