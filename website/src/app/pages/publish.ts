import { Component, computed, inject, input, linkedSignal, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSelectModule } from "@angular/material/select";
import { MatSnackBar } from "@angular/material/snack-bar";
import { Router, RouterLink } from "@angular/router";
import { z } from "zod";
import type { PackageDetail, PublishForm } from "../api";
import { Api, ApiError } from "../api";
import type { Bump } from "../format";
import { buildSkillMd, editSkillMd, idPattern, nextVersion, parseSkillMd, slugify } from "../format";
import { Session } from "../session";
import { Icon } from "../shared/icon";
import { newestVersion } from "../shared/status";
import { runTask } from "../shared/tasks";
import type { Draft, Picked } from "./publish-parts";
import { EditStatus, newDraft, pickedArchive, Problem, relativePaths, UploadPicker, VersionFields, WriteForm } from "./publish-parts";

type Mode = "new" | "edit" | "upload";
type Method = "write" | "upload";

/**
 * The fields of `agent-plugins.json` an edit changes. The file is round-tripped, so fields the
 * portal does not know stay as the publisher wrote them (hence loose objects).
 */
const manifestSchema = z.looseObject({ packages: z.array(z.looseObject({ name: z.string().optional(), description: z.string().optional() })).min(1) });
type Manifest = z.infer<typeof manifestSchema>;

interface EditBase {
  readonly version: string;
  readonly manifest: Manifest;
}

const headings: Readonly<Record<Mode, readonly [string, string]>> = {
  new: ["Share a skill", "Write instructions for your AI assistant once, and everyone at work can use them. No files or code needed."],
  edit: ["Edit", "Change the description or the instructions. Your edits become a new version."],
  upload: ["New version", "Upload the updated files. They replace the current version once published."]
};

@Component({
  selector: "app-publish",
  imports: [RouterLink, MatButtonModule, MatButtonToggleModule, MatFormFieldModule, MatProgressBarModule, MatSelectModule, Icon, EditStatus, WriteForm, UploadPicker, VersionFields, Problem],
  templateUrl: "./publish.html",
  styleUrl: "./publish.scss"
})
export class PublishPage {
  public readonly mode = input.required<Mode>();
  public readonly ns = input<string>();
  public readonly pkg = input<string>();

  protected readonly session = inject(Session);
  private readonly api = inject(Api);
  private readonly snackBar = inject(MatSnackBar);
  private readonly router = inject(Router);

  protected readonly mine = resource({ loader: () => this.api.mine() });

  /** The package a new version is for, when editing or uploading one. */
  protected readonly existing = resource({
    params: () => {
      const ns = this.ns();
      const pkg = this.pkg();
      return ns !== undefined && pkg !== undefined ? { ns, pkg } : undefined;
    },
    loader: ({ params }) => this.api.package(params.ns, params.pkg)
  });

  protected readonly method = linkedSignal<Mode, Method>({ source: () => this.mode(), computation: (mode) => (mode === "upload" ? "upload" : "write") });
  protected readonly space = linkedSignal(() => this.ns() ?? this.session.me()?.namespace ?? "");
  protected readonly drafts = signal<readonly Draft[]>([newDraft()]);
  protected readonly packTitle = signal("");
  protected readonly packDescription = signal("");
  protected readonly idOverride = signal<string | null>(null);
  protected readonly bump = signal<Bump>("patch");
  protected readonly changelog = signal("");
  protected readonly tags = signal("");
  protected readonly picked = signal<readonly Picked[]>([]);
  protected readonly uploadTitle = signal("");
  protected readonly submitting = signal(false);
  protected readonly problem = signal<ApiError | null>(null);

  /** When editing: the version the changes are laid over, and its manifest. Loading it fills the editors. */
  protected readonly base = resource({
    params: () => (this.mode() === "edit" && this.existing.hasValue() ? { detail: this.existing.value() } : undefined),
    loader: ({ params }) => this.loadForEdit(params.detail)
  });

  protected readonly baseError = computed(() => {
    const error = this.base.error() ?? this.existing.error();
    return error === undefined ? null : ApiError.from(error).message;
  });

  protected readonly spaces = computed(() =>
    (this.mine.value()?.spaces ?? []).map((space) => ({ namespace: space.namespace, label: `${space.displayName} (${space.lane === "personal" ? "you" : space.lane})` }))
  );

  protected readonly isPack = computed(() => this.method() === "write" && this.mode() === "new" && this.drafts().length > 1);

  protected readonly title = computed(() => {
    if (this.mode() !== "new") {
      return this.existing.value()?.name ?? "";
    }

    if (this.method() === "upload") {
      return this.uploadTitle();
    }

    return this.isPack() ? this.packTitle() : (this.drafts()[0]?.title ?? "");
  });

  protected readonly heading = computed(() => {
    const [heading] = headings[this.mode()];
    return this.mode() === "new" ? heading : `${heading}: ${this.title()}`;
  });

  protected readonly lead = computed(() => headings[this.mode()][1]);
  protected readonly packageId = computed(() => this.pkg() ?? this.idOverride() ?? slugify(this.title()));
  protected readonly version = computed(() => {
    const detail = this.existing.value();
    return detail === undefined
      ? "1.0.0"
      : nextVersion(
          detail.versions.map((version) => version.version),
          this.bump()
        );
  });

  protected readonly taken = computed(() => this.mode() === "new" && this.mine.value()?.packages.some((item) => item.namespace === this.space() && item.packageId === this.packageId()) === true);
  protected readonly reviewed = computed(() => this.mode() === "new" && !this.session.isAdmin());
  protected readonly spaceLabel = computed(() => this.mine.value()?.spaces.find((space) => space.namespace === this.space())?.displayName ?? this.space());
  protected readonly cancelLink = computed(() => (this.mode() === "new" ? ["/mine"] : ["/p", this.space(), this.packageId()]));

  protected readonly submitLabel = computed(() => {
    if (this.submitting()) {
      return "Sending…";
    }

    return this.reviewed() ? "Send for review" : `Publish version ${this.version()}`;
  });

  protected readonly ready = computed(() => {
    if (this.submitting() || !idPattern.test(this.packageId()) || this.taken() || this.space() === "") {
      return false;
    }

    if (this.method() === "upload") {
      return this.picked().length > 0 && (this.mode() !== "new" || this.uploadTitle().trim().length > 0);
    }

    const editing = this.mode() === "edit";
    const complete = this.drafts().every((item) => item.description.trim().length > 0 && item.body.trim().length > 0 && (editing || slugify(item.title).length > 0));
    return complete && (!this.isPack() || this.packTitle().trim().length > 0) && (!editing || this.base.hasValue());
  });

  protected setMethod(value: unknown): void {
    if (value === "write" || value === "upload") {
      this.method.set(value);
    }
  }

  protected setSpace(value: unknown): void {
    if (typeof value === "string") {
      this.space.set(value);
    }
  }

  protected submit(event: SubmitEvent): void {
    event.preventDefault();
    if (this.ready()) {
      runTask(this.send());
    }
  }

  private async send(): Promise<void> {
    this.submitting.set(true);
    this.problem.set(null);
    try {
      const ns = this.space();
      const packageId = this.packageId();
      const published = await this.api.publish(ns, packageId, this.form());
      const message = published.reviewState === "pending" ? "Sent for review. You'll see the result on My skills." : `Published version ${published.version}. It's live now.`;
      this.snackBar.open(message, undefined, { duration: 6000 });
      await this.router.navigate(["/p", ns, packageId]);
    } catch (error) {
      this.problem.set(ApiError.from(error));
    } finally {
      this.submitting.set(false);
    }
  }

  private form(): PublishForm {
    const common = {
      version: this.version(),
      changelog: this.changelog().trim(),
      tags: this.tags()
        .split(",")
        .map((tag) => tag.trim())
        .filter((tag) => tag.length > 0)
    };
    if (this.method() === "upload") {
      return { ...common, ...this.uploadFiles() };
    }

    if (this.mode() === "edit") {
      return { ...common, ...this.editFiles() };
    }

    const drafts = this.drafts();
    const single = drafts.length === 1;
    const files = drafts.map((item) => {
      const id = single ? this.packageId() : slugify(item.title);
      const text = buildSkillMd({ name: id, description: item.description.trim(), body: item.body });
      return { path: single ? "SKILL.md" : `${id}/SKILL.md`, file: new Blob([text], { type: "text/markdown" }) };
    });
    return this.isPack()
      ? { ...common, files, name: this.packTitle().trim(), description: this.packDescription().trim() }
      : { ...common, files, name: this.title().trim(), description: drafts[0]?.description.trim() ?? "" };
  }

  /** A zip goes as an archive; anything else as files with paths. New versions keep the listing. */
  private uploadFiles(): Pick<PublishForm, "files" | "archive" | "name" | "description"> {
    const picked = this.picked();
    const archive = pickedArchive(picked);
    const paths = relativePaths(picked);
    const files = archive === null ? picked.map((item, index) => ({ path: paths[index] ?? item.path, file: item.file })) : [];
    const detail = this.existing.value();
    return { files, ...(archive === null ? {} : { archive }), name: this.title().trim(), ...(detail === undefined ? {} : { description: detail.description }) };
  }

  /** Only changed files are sent; the server lays them over the base version. */
  private editFiles(): Pick<PublishForm, "files" | "base"> {
    const base = this.base.value();
    if (base === undefined) {
      throw new ApiError(0, "The published version is still loading.");
    }

    const files: { path: string; file: Blob }[] = [];
    for (const item of this.drafts()) {
      if (item.path === undefined || item.original === undefined) {
        continue;
      }

      const text = editSkillMd(item.original, item.description.trim(), item.body);
      if (text !== item.original) {
        files.push({ path: item.path, file: new Blob([text], { type: "text/markdown" }) });
      }
    }

    const [first, ...others] = base.manifest.packages;
    const title = this.packTitle().trim();
    const description = this.packDescription().trim();
    if (first !== undefined && (first.name !== title || first.description !== description)) {
      const manifest = { ...base.manifest, packages: [{ ...first, name: title, description }, ...others] };
      files.push({ path: "agent-plugins.json", file: new Blob([`${JSON.stringify(manifest, null, 2)}\n`], { type: "application/json" }) });
    }

    if (files.length === 0) {
      throw new ApiError(0, "Nothing changed yet. Edit a description or the instructions first.");
    }

    return { files, base: base.version };
  }

  private async loadForEdit(detail: PackageDetail): Promise<EditBase> {
    const version = newestVersion(detail)?.version;
    if (version === undefined) {
      throw new ApiError(0, "There is no version to edit. Upload one instead.");
    }

    const files = await this.api.files(detail.namespace, detail.packageId, version);
    const manifest = manifestSchema.parse(JSON.parse(await this.api.fileText(detail.namespace, detail.packageId, version, "agent-plugins.json")));
    const skills = files.filter((file) => /^skills\/[^/]+\/SKILL\.md$/u.test(file.path));
    const drafts = await Promise.all(
      skills.map(async (file) => {
        const original = await this.api.fileText(detail.namespace, detail.packageId, version, file.path);
        const parsed = parseSkillMd(original);
        return { ...newDraft(parsed?.name ?? file.path, parsed?.description ?? "", parsed?.body ?? original), path: file.path, original };
      })
    );
    this.drafts.set(drafts);
    this.packTitle.set(manifest.packages[0]?.name ?? detail.name);
    this.packDescription.set(manifest.packages[0]?.description ?? detail.description);
    return { version, manifest };
  }
}
