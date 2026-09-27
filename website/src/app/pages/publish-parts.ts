import { NgTemplateOutlet } from "@angular/common";
import { Component, computed, input, model, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatRadioModule } from "@angular/material/radio";
import { RouterLink } from "@angular/router";
import type { ApiError, DryRun, Visibility } from "../api";
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

/** What the server will make of the picked files, in words, and whether it can make anything of them. */
export function describeUpload(files: readonly Picked[]): { readonly text: string; readonly usable: boolean } | null {
  if (files.length === 0) {
    return null;
  }

  if (pickedArchive(files) !== null) {
    return { text: "A zip file. It's unpacked and checked on the server.", usable: true };
  }

  const paths = relativePaths(files);
  if (paths.includes("agent-plugins.json")) {
    return { text: "A source tree with its own agent-plugins.json.", usable: true };
  }

  if (paths.includes("SKILL.md")) {
    return { text: "One skill.", usable: true };
  }

  const skills = paths.filter((path) => /^[^/]+\/SKILL\.md$/u.test(path)).length;
  if (skills > 0) {
    return { text: `A skill pack with ${String(skills)} skills.`, usable: true };
  }

  if (paths.some((path) => /(^|\/)skill\.md(\.txt)?$/iu.test(path))) {
    return { text: "The skill file must be named exactly SKILL.md, in capitals, with no .txt at the end. Rename it and pick it again.", usable: false };
  }

  return paths.length === 1 && paths[0]?.endsWith(".json") === true
    ? { text: "An MCP server document.", usable: true }
    : { text: "No SKILL.md found. Pick a skill's folder, or a folder of skill folders.", usable: false };
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

/** A skill written in the browser: the three things a SKILL.md needs, in plain words. */
@Component({
  selector: "app-skill-editor",
  imports: [MatFormFieldModule, MatInputModule],
  template: `
    @if (showName()) {
      <mat-form-field appearance="outline">
        <mat-label>Name</mat-label>
        <input matInput maxlength="120" required placeholder="Meeting summaries" [value]="name()" (input)="name.set(text($event))" />
      </mat-form-field>
    }
    <mat-form-field appearance="outline">
      <mat-label>When should the assistant use it?</mat-label>
      <textarea matInput rows="2" required maxlength="1024" [value]="description()" (input)="description.set(text($event))"></textarea>
      <mat-hint>One or two sentences, like “Use when someone asks for a standup update from rough notes.” The assistant reads this to decide.</mat-hint>
    </mat-form-field>
    <mat-form-field appearance="outline">
      <mat-label>Instructions</mat-label>
      <textarea matInput rows="14" required [value]="body()" (input)="body.set(text($event))"></textarea>
      <mat-hint>What the assistant should do, step by step. Markdown works: # headings, - lists, **bold**.</mat-hint>
    </mat-form-field>
  `,
  styles: `
    mat-form-field {
      width: 100%;
    }
  `,
  host: { class: "card stack" }
})
export class SkillEditor {
  public readonly showName = input.required<boolean>();
  public readonly name = model.required<string>();
  public readonly description = model.required<string>();
  public readonly body = model.required<string>();

  protected readonly text = text;
}

/** Who can install a new package: its own rule, or its space's. */
const audiences: readonly { readonly value: Visibility; readonly label: string }[] = [
  { value: "private", label: "Only me, and people I share it with" },
  { value: "public", label: "Everyone" },
  { value: "inherit", label: "Same as the space" }
];

/** Where a new package goes and who can install it. */
@Component({
  selector: "app-publish-target",
  imports: [RouterLink, MatRadioModule],
  template: `
    <h2 id="space-heading">Publish to</h2>
    <mat-radio-group class="choices" aria-labelledby="space-heading" [value]="space()" (change)="space.set($event.value)">
      @for (option of spaces(); track option.namespace) {
        <mat-radio-button [value]="option.namespace">{{ option.label }}</mat-radio-button>
      }
    </mat-radio-group>
    <p class="muted">
      <span [hidden]="!hasTeam()">A team space is shared: everyone on the team can publish new versions.</span>
      Need a shared space? <a routerLink="/teams">Create a team</a>.
    </p>
    <h2 id="audience-heading">Who can install it</h2>
    <mat-radio-group class="choices" aria-labelledby="audience-heading" [value]="visibility()" (change)="visibility.set($event.value)">
      @for (option of audiences; track option.value) {
        <mat-radio-button [value]="option.value">{{ option.label }}</mat-radio-button>
      }
    </mat-radio-group>
    <p class="muted">You can share it with more people, or with everyone, later.</p>
  `,
  styles: `
    h2,
    p {
      margin: 0;
    }
    .choices {
      display: flex;
      flex-wrap: wrap;
      gap: 0.5rem 1.5rem;
    }
  `,
  host: { class: "card stack" }
})
export class PublishTarget {
  public readonly spaces = input.required<readonly { readonly namespace: string; readonly label: string }[]>();
  public readonly hasTeam = input.required<boolean>();
  public readonly space = model.required<string>();
  public readonly visibility = model.required<Visibility>();

  protected readonly audiences = audiences;
}

/** How a new version's files compare with the live one, from a dry run on the server. */
@Component({
  selector: "app-file-changes",
  imports: [MatProgressBarModule, Problem],
  template: `
    @if (loading()) {
      <mat-progress-bar mode="indeterminate" aria-label="Comparing with the live version" />
    }
    <app-problem [problem]="problem()" />
    @if (check(); as result) {
      <h2>Compared with the live version</h2>
      @if (removed() > 0) {
        <p class="notice" role="status">
          {{ removed() === 1 ? "1 file" : removed() + " files" }} in the live version {{ removed() === 1 ? "isn't" : "aren't" }} in what you picked, so
          {{ removed() === 1 ? "it goes" : "they go" }} away when you publish.
        </p>
      }
      <ul>
        @for (file of changed(); track file.path) {
          <li>
            <code>{{ file.path }}</code> <span [class]="badges[file.status].badge">{{ badges[file.status].label }}</span>
          </li>
        } @empty {
          <li class="muted">Nothing changed.</li>
        }
      </ul>
      @for (warning of result.warnings; track warning) {
        <p class="notice">{{ warning }}</p>
      }
    }
  `,
  styles: `
    ul {
      list-style: none;
      margin: 0;
      padding: 0;
      display: grid;
      gap: 0.35rem;
    }
    h2,
    p {
      margin: 0;
    }
  `,
  host: { class: "card stack", "[hidden]": "!loading() && problem() === null && check() === null" }
})
export class FileChanges {
  public readonly check = input.required<DryRun | null>();
  public readonly loading = input.required<boolean>();
  public readonly problem = input.required<ApiError | null>();

  protected readonly badges = {
    new: { label: "New", badge: "badge live" },
    changed: { label: "Changed", badge: "badge pending" },
    removed: { label: "Removed", badge: "badge rejected" },
    same: { label: "Same", badge: "badge" }
  } as const;
  protected readonly changed = computed(() => (this.check()?.files ?? []).filter((file) => file.status !== "same"));
  protected readonly removed = computed(() => this.changed().filter((file) => file.status === "removed").length);
}

/** Version, notes, tags, and the technical details a publisher may want to see. */
@Component({
  selector: "app-version-fields",
  imports: [MatButtonToggleModule, MatFormFieldModule, MatInputModule],
  templateUrl: "./version-fields.html",
  styleUrl: "./version-fields.scss",
  host: { class: "card stack" }
})
export class VersionFields {
  public readonly isNew = input.required<boolean>();
  public readonly version = input.required<string>();
  public readonly space = input.required<string>();
  public readonly packageId = input.required<string>();
  public readonly idProblem = input.required<string | null>();
  public readonly bump = model.required<Bump>();
  public readonly changelog = model.required<string>();
  public readonly tags = model.required<string>();
  public readonly idOverride = model.required<string | null>();

  protected readonly text = text;
  protected readonly noteLabel = computed(() => (this.isNew() ? "Notes for this version (optional)" : "What changed? (optional)"));

  protected setBump(value: unknown): void {
    if (value === "patch" || value === "minor" || value === "major") {
      this.bump.set(value);
    }
  }

  protected setId(event: Event): void {
    this.idOverride.set(text(event).trim());
  }
}
