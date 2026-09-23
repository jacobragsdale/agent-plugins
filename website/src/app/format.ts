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
