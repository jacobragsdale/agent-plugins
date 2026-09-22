// Pure helpers shared by the pages. Everything here is covered by format.spec.ts.

export type Bump = "patch" | "minor" | "major";

type SemVer = readonly [number, number, number];

const semverPattern = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/u;

function parseSemVer(text: string): SemVer | null {
  const match = semverPattern.exec(text);
  return match === null ? null : [Number(match[1]), Number(match[2]), Number(match[3])];
}

function compare(left: SemVer, right: SemVer): number {
  if (left[0] !== right[0]) {
    return left[0] - right[0];
  }

  return left[1] !== right[1] ? left[1] - right[1] : left[2] - right[2];
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

/** JSON strings are valid YAML double-quoted scalars, so quoting through JSON is always safe. */
export function buildSkillMd(skill: SkillText): string {
  return `---\nname: ${JSON.stringify(skill.name)}\ndescription: ${JSON.stringify(skill.description)}\n---\n\n${skill.body.trim()}\n`;
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

/**
 * Replaces the description and body of an existing SKILL.md, keeping every other frontmatter key
 * (and its formatting) exactly as the publisher wrote it.
 */
export function editSkillMd(original: string, description: string, body: string): string {
  const parts = splitFrontmatter(original);
  if (parts === null) {
    return buildSkillMd({ name: "", description, body });
  }

  const lines = [...parts.lines];
  const replacement = `description: ${JSON.stringify(description)}`;
  const extent = keyExtent(lines, "description");
  if (extent === null) {
    lines.push(replacement);
  } else {
    lines.splice(extent.start, extent.end - extent.start, replacement);
  }

  return `---\n${lines.join("\n")}\n---\n\n${body.trim()}\n`;
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
export function isJunk(path: string): boolean {
  const parts = path.split("/");
  const name = parts.at(-1) ?? "";
  return parts.some((part) => part === ".git" || part === "node_modules") || name === ".DS_Store" || name === "Thumbs.db" || name === "desktop.ini";
}
