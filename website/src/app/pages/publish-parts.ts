import { Component, computed, input, model, output, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { RouterLink } from "@angular/router";
import type { ApiError } from "../api";
import type { Bump } from "../format";
import { isJunk, parseSkillMd, slugify, titleCase } from "../format";
import { Icon } from "../shared/icon";
import { Markdown } from "../shared/markdown";
import { runTask } from "../shared/tasks";

export interface Draft {
  readonly key: number;
  readonly title: string;
  readonly description: string;
  readonly body: string;
  /** When editing: the skill's path in the package and its text as published. */
  readonly path?: string;
  readonly original?: string;
}

export type DraftChange = Partial<Pick<Draft, "title" | "description" | "body">>;

export interface Picked {
  readonly path: string;
  readonly file: File;
}

function text(event: Event): string {
  return event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement ? event.target.value : "";
}

/** Drops a folder name every path shares, as the server does. */
export function relativePaths(files: readonly Picked[]): string[] {
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

const bodyPlaceholder = `# Meeting summaries

When I share meeting notes or a transcript:

1. Start with a two-sentence summary.
2. List decisions, then action items with owners and dates.
3. Keep it under 200 words.`;

/** One skill in the guided editor: name, when to use it, and the instructions with a preview. */
@Component({
  selector: "app-skill-editor",
  imports: [MatButtonModule, MatFormFieldModule, MatInputModule, Icon, Markdown],
  templateUrl: "./skill-editor.html",
  styleUrl: "./skill-editor.scss",
  host: { class: "card stack" }
})
export class SkillEditor {
  public readonly draft = input.required<Draft>();
  public readonly heading = input.required<string>();
  public readonly showName = input.required<boolean>();
  public readonly removable = input.required<boolean>();
  public readonly changed = output<DraftChange>();
  public readonly remove = output();

  protected readonly bodyPlaceholder = bodyPlaceholder;
  protected readonly previewing = signal(false);
  protected readonly hasBody = computed(() => this.draft().body.trim().length > 0);
  protected readonly text = text;
}

/** Upload: drag and drop, or choose files or a folder. */
@Component({
  selector: "app-upload-picker",
  imports: [MatButtonModule, MatFormFieldModule, MatInputModule, Icon],
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
  protected readonly text = text;

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
  public readonly spaceLabel = input.required<string>();
  public readonly packageId = input.required<string>();
  public readonly taken = input.required<boolean>();
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
    const value = text(event).trim();
    this.idOverride.set(value.length === 0 ? null : value);
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

/** The loading and error states of an edit, above the editors. */
@Component({
  selector: "app-edit-status",
  imports: [MatProgressBarModule],
  template: `
    @if (loading()) {
      <mat-progress-bar mode="indeterminate" aria-label="Loading the published version" />
    } @else if (error(); as message) {
      <p class="problem">{{ message }}</p>
    }
  `
})
export class EditStatus {
  public readonly loading = input.required<boolean>();
  public readonly error = input.required<string | null>();
}

let nextKey = 1;

export function newDraft(title = "", description = "", body = ""): Draft {
  return { key: nextKey++, title, description, body };
}

/** The guided editor: one or more skills, plus the listing fields when editing or making a pack. */
@Component({ selector: "app-write-form", imports: [MatButtonModule, MatFormFieldModule, MatInputModule, Icon, SkillEditor], templateUrl: "./write-form.html", styleUrl: "./write-form.scss" })
export class WriteForm {
  public readonly editing = input.required<boolean>();
  public readonly drafts = model.required<readonly Draft[]>();
  public readonly packTitle = model.required<string>();
  public readonly packDescription = model.required<string>();

  protected readonly text = text;
  protected readonly isPack = computed(() => !this.editing() && this.drafts().length > 1);

  protected heading(draft: Draft, index: number): string {
    if (this.editing()) {
      return titleCase(draft.title);
    }

    return this.drafts().length > 1 ? `Skill ${String(index + 1)}` : "Your skill";
  }

  protected update(key: number, change: DraftChange): void {
    this.drafts.update((items) => items.map((item) => (item.key === key ? { ...item, ...change } : item)));
  }

  protected add(): void {
    this.drafts.update((items) => [...items, newDraft()]);
  }

  protected remove(key: number): void {
    this.drafts.update((items) => items.filter((item) => item.key !== key));
  }
}
