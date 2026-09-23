import { NgTemplateOutlet } from "@angular/common";
import { Component, computed, input, model, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { RouterLink } from "@angular/router";
import type { ApiError } from "../api";
import type { Bump } from "../format";
import type { SkillGroup } from "../format";
import { groupBySkill, isJunk, parseSkillMd, slugify, titleCase } from "../format";
import { Icon } from "../shared/icon";
import { runTask } from "../shared/tasks";

export interface Picked {
  readonly path: string;
  readonly file: File;
}

function text(event: Event): string {
  return event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement ? event.target.value : "";
}

/** Drops a folder name every path shares, as the server does. */
function relativePaths(files: readonly Picked[]): string[] {
  const first = files[0]?.path ?? "";
  const top = first.includes("/") ? first.slice(0, first.indexOf("/") + 1) : "";
  const strip = top !== "" && files.every((file) => file.path.startsWith(top));
  return files.map((file) => (strip ? file.path.slice(top.length) : file.path));
}

/** A single picked .zip is sent as an archive; everything else as files with paths. */
export function pickedArchive(files: readonly Picked[]): File | null {
  const [only, ...rest] = files;
  return only !== undefined && rest.length === 0 && only.path.toLowerCase().endsWith(".zip") ? only.file : null;
}

function isFileEntry(entry: FileSystemEntry): entry is FileSystemFileEntry {
  return entry.isFile;
}

function isDirectoryEntry(entry: FileSystemEntry): entry is FileSystemDirectoryEntry {
  return entry.isDirectory;
}

/** Reads a dropped file or folder, recursively, keeping each file's path. */
async function readEntry(entry: FileSystemEntry): Promise<Picked[]> {
  if (isFileEntry(entry)) {
    const file = await new Promise<File>((resolve, reject) => {
      entry.file(resolve, reject);
    });
    return [{ path: entry.fullPath.replace(/^\//u, ""), file }];
  }

  if (!isDirectoryEntry(entry)) {
    return [];
  }

  const reader = entry.createReader();
  const children: FileSystemEntry[] = [];
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => {
      reader.readEntries(resolve, reject);
    });
    if (batch.length === 0) {
      break;
    }

    children.push(...batch);
  }

  return (await Promise.all(children.map(readEntry))).flat();
}

/** What the server will make of the picked files, in words. */
function describeUpload(files: readonly Picked[]): string | null {
  if (files.length === 0) {
    return null;
  }

  if (pickedArchive(files) !== null) {
    return "A zip file. It's unpacked and checked on the server.";
  }

  const paths = relativePaths(files);
  if (paths.includes("agent-plugins.json")) {
    return "A source tree with its own agent-plugins.json.";
  }

  if (paths.includes("SKILL.md")) {
    return "One skill.";
  }

  const skills = paths.filter((path) => /^[^/]+\/SKILL\.md$/u.test(path)).length;
  if (skills > 0) {
    return `A skill pack with ${String(skills)} skills.`;
  }

  return paths.length === 1 && paths[0]?.endsWith(".json") === true ? "An MCP server document." : "No SKILL.md found. Pick a skill's folder, or a folder of skill folders.";
}

/** Upload: drag and drop, or choose files or a folder. */
@Component({
  selector: "app-upload-picker",
  imports: [NgTemplateOutlet, MatButtonModule, MatFormFieldModule, MatInputModule, Icon],
  templateUrl: "./upload-picker.html",
  styleUrl: "./upload-picker.scss",
  host: { class: "card stack" }
})
export class UploadPicker {
  public readonly showName = input.required<boolean>();
  public readonly picked = model.required<readonly Picked[]>();
  public readonly title = model.required<string>();

  protected readonly dragging = signal(false);
  protected readonly kind = computed(() => describeUpload(this.picked()));
  protected readonly groups = computed(() => groupBySkill(this.picked()));
  protected readonly text = text;

  protected fileCount(group: SkillGroup<Picked>): string {
    return group.files.length === 1 ? "1 file" : `${String(group.files.length)} files`;
  }

  /** A button inside <summary> would also toggle the section, so the click stops here. */
  protected removeGroup(event: Event, group: SkillGroup<Picked>): void {
    event.preventDefault();
    this.picked.update((files) => files.filter((file) => !group.files.includes(file)));
  }

  protected removeFolder(event: Event, path: string): void {
    event.preventDefault();
    this.picked.update((files) => files.filter((file) => !file.path.startsWith(path)));
  }

  protected choose(event: Event): void {
    const target = event.target;
    if (!(target instanceof HTMLInputElement) || target.files === null) {
      return;
    }

    const files = Array.from(target.files, (file) => ({ path: file.webkitRelativePath.length > 0 ? file.webkitRelativePath : file.name, file }));
    target.value = "";
    runTask(this.add(files));
  }

  protected dragOver(event: DragEvent): void {
    event.preventDefault();
    this.dragging.set(true);
  }

  protected drop(event: DragEvent): void {
    event.preventDefault();
    this.dragging.set(false);
    const entries = Array.from(event.dataTransfer?.items ?? [])
      .map((item) => item.webkitGetAsEntry())
      .filter((entry): entry is FileSystemEntry => entry !== null);
    runTask(this.readDropped(entries));
  }

  protected clear(): void {
    this.picked.set([]);
    if (this.showName()) {
      this.title.set("");
    }
  }

  protected removeFile(path: string): void {
    this.picked.update((files) => files.filter((file) => file.path !== path));
  }

  private async readDropped(entries: readonly FileSystemEntry[]): Promise<void> {
    await this.add((await Promise.all(entries.map(readEntry))).flat());
  }

  /** Adds files, and suggests a name from the skill's frontmatter or the folder. */
  private async add(files: readonly Picked[]): Promise<void> {
    const kept = files.filter((file) => !isJunk(file.path));
    this.picked.update((current) => [...current.filter((existing) => !kept.some((file) => file.path === existing.path)), ...kept]);
    if (!this.showName() || this.title().length > 0) {
      return;
    }

    const all = this.picked();
    const skillIndex = relativePaths(all).indexOf("SKILL.md");
    const skillFile = skillIndex >= 0 ? all[skillIndex]?.file : undefined;
    const skill = skillFile === undefined ? null : parseSkillMd(await skillFile.text());
    const first = all[0]?.path ?? "";
    const name = skill?.name ?? (first.includes("/") ? first.slice(0, first.indexOf("/")) : first.replace(/\.[^.]+$/u, ""));
    this.title.set(titleCase(slugify(name)));
  }
}

/** Version, notes, tags, and the technical details a publisher may want to see. */
@Component({
  selector: "app-version-fields",
  imports: [RouterLink, MatButtonToggleModule, MatFormFieldModule, MatInputModule],
  templateUrl: "./version-fields.html",
  styleUrl: "./version-fields.scss",
  host: { class: "card stack" }
})
export class VersionFields {
  public readonly isNew = input.required<boolean>();
  public readonly reviewed = input.required<boolean>();
  public readonly version = input.required<string>();
  public readonly space = input.required<string>();
  public readonly packageId = input.required<string>();
  public readonly idProblem = input.required<string | null>();
  public readonly bump = model.required<Bump>();
  public readonly changelog = model.required<string>();
  public readonly tags = model.required<string>();
  public readonly idOverride = model.required<string | null>();

  protected readonly text = text;
  protected readonly noteLabel = computed(() => (this.isNew() ? "Note for the reviewer (optional)" : "What changed? (optional)"));

  protected setBump(value: unknown): void {
    if (value === "patch" || value === "minor" || value === "major") {
      this.bump.set(value);
    }
  }

  protected setId(event: Event): void {
    this.idOverride.set(text(event).trim());
  }
}

/** A failed publish, with each validator finding on its own line. */
@Component({
  selector: "app-problem",
  template: `
    @if (problem(); as failure) {
      <div class="problem" role="alert">
        <strong>{{ failure.message }}</strong>
        @if (failure.details.length > 0) {
          <ul>
            @for (detail of failure.details; track $index) {
              <li>
                @if (detail.path.length > 0) {
                  <code>{{ detail.path }}</code
                  >:
                }
                {{ detail.message }}
              </li>
            }
          </ul>
        }
      </div>
    }
  `
})
export class Problem {
  public readonly problem = input.required<ApiError | null>();
}
