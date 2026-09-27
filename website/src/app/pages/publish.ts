import { Component, computed, effect, inject, input, linkedSignal, resource, signal, untracked } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { Router, RouterLink } from "@angular/router";
import type { PublishForm, Visibility } from "../api";
import { Api, ApiError, archiveUrl, versionFiles } from "../api";
import type { Bump } from "../format";
import { filesWritingDrops, idPattern, nextVersion, packageIdFor, parseSkillMd, skillMd, slugify } from "../format";
import { Session } from "../session";
import { Icon } from "../shared/icon";
import { runTask } from "../shared/tasks";
import type { Picked } from "./publish-parts";
import { describeUpload, FileChanges, pickedArchive, Problem, PublishTarget, SkillEditor, UploadPicker, VersionFields } from "./publish-parts";

type Mode = "new" | "upload" | "suggest";

const headings: Readonly<Record<Mode, readonly [string, string]>> = {
  new: ["Share a skill", "Write it here, or upload a skill folder or zip. You choose who can install it."],
  upload: ["New version", "The files you upload replace the current version when you publish."],
  suggest: ["Suggest a change", "Upload your improved files. The owners decide whether to publish them."]
};

type Source = "write" | "upload";

@Component({
  selector: "app-publish",
  imports: [
    RouterLink,
    MatButtonModule,
    MatButtonToggleModule,
    MatFormFieldModule,
    MatInputModule,
    MatProgressBarModule,
    Icon,
    UploadPicker,
    SkillEditor,
    FileChanges,
    PublishTarget,
    VersionFields,
    Problem
  ],
  templateUrl: "./publish.html",
  styleUrl: "./publish.scss"
})
export class PublishPage {
  public readonly mode = input.required<Mode>();
  public readonly ns = input<string>();
  public readonly pkg = input<string>();
  /** The space to publish to, preselected when a team page sent the person here. */
  public readonly to = input<string>();
  /** `true` opens a new version in the editor, filled in from the live SKILL.md. */
  public readonly edit = input<string>();

  protected readonly session = inject(Session);
  private readonly api = inject(Api);
  private readonly snackBar = inject(MatSnackBar);
  private readonly router = inject(Router);

  protected readonly mine = resource({ loader: () => this.api.mine() });

  /** The package a new version is for. */
  protected readonly existing = resource({
    params: () => {
      const ns = this.ns();
      const pkg = this.pkg();
      return ns !== undefined && pkg !== undefined ? { ns, pkg } : undefined;
    },
    loader: ({ params }) => this.api.package(params.ns, params.pkg)
  });

  protected readonly space = linkedSignal(() => this.ns() ?? this.to() ?? this.session.me()?.namespace ?? "");
  protected readonly source = linkedSignal<Source>(() => (this.edit() === "true" ? "write" : this.mode() === "new" ? "write" : "upload"));
  /** A personal space starts private, so a first try reaches nobody by surprise; a team follows its own setting. */
  protected readonly visibility = linkedSignal<Visibility>(() => (this.space() === this.session.me()?.namespace ? "private" : "inherit"));
  protected readonly skillDescription = signal("");
  protected readonly skillBody = signal("");
  protected readonly idOverride = signal<string | null>(null);
  protected readonly bump = signal<Bump>("patch");
  protected readonly changelog = signal("");
  /** Why a suggestion is worth publishing, for the owners. */
  protected readonly message = signal("");
  protected readonly tags = signal("");
  protected readonly picked = signal<readonly Picked[]>([]);
  protected readonly uploadTitle = signal("");
  protected readonly submitting = signal(false);
  protected readonly problem = signal<ApiError | null>(null);

  protected readonly spaces = computed(() =>
    (this.mineValue()?.spaces ?? []).map((space) => ({
      namespace: space.namespace,
      label: `${space.lane === "personal" ? "My space" : space.displayName} · ${space.visibility === "private" ? "private" : "everyone can see it"}`
    }))
  );
  protected readonly suggesting = computed(() => this.mode() === "suggest");

  // A failed load keeps its error in the resource and shows it through the problem notice;
  // reading value() from an errored resource throws, so everything else reads these.
  protected readonly existingDetail = computed(() => (this.existing.hasValue() ? this.existing.value() : undefined));
  private readonly mineValue = computed(() => (this.mine.hasValue() ? this.mine.value() : undefined));
  protected readonly hasTeam = computed(() => this.mineValue()?.spaces.some((space) => space.lane === "team") === true);
  protected readonly title = computed(() => (this.mode() === "new" ? this.uploadTitle() : (this.existingDetail()?.name ?? "")));
  protected readonly heading = computed(() => {
    const [heading] = headings[this.mode()];
    return this.mode() === "new" ? heading : `${heading}: ${this.title()}`;
  });

  protected readonly lead = computed(() => headings[this.mode()][1]);
  protected readonly packageId = computed(() => this.pkg() ?? this.idOverride() ?? packageIdFor(this.title(), this.space()));
  /** Where the suggester can get the files they are improving. */
  protected readonly currentFiles = computed(() => {
    const live = this.existingDetail()?.liveVersion;
    return live === null || live === undefined ? null : archiveUrl(this.space(), this.packageId(), live);
  });

  /** The live SKILL.md, read once to fill in the editor for a new version. */
  private readonly liveSkill = resource({
    params: () => {
      const detail = this.existingDetail();
      const live = detail?.liveVersion ?? null;
      return this.edit() === "true" && detail !== undefined && live !== null ? { files: versionFiles(detail.namespace, detail.packageId, live) } : undefined;
    },
    loader: async ({ params }) => {
      const path = (await this.api.files(params.files)).find((file) => file.path.endsWith("SKILL.md"))?.path;
      return path === undefined ? null : parseSkillMd(await this.api.fileText(params.files, path));
    }
  });
  /** The live version's files, for a new version: writing it here publishes a SKILL.md alone. */
  private readonly liveFiles = resource({
    params: () => {
      const detail = this.existingDetail();
      const live = detail?.liveVersion ?? null;
      return this.mode() === "upload" && detail !== undefined && live !== null ? { files: versionFiles(detail.namespace, detail.packageId, live) } : undefined;
    },
    loader: ({ params }) => this.api.files(params.files)
  });

  /** Files a written version would remove from every PC: the editor is for single-file skills only. */
  protected readonly writeDrops = computed(() =>
    this.source() === "write" && this.mode() === "upload" && this.liveFiles.hasValue() ? filesWritingDrops(this.liveFiles.value().map((file) => file.path)) : []
  );

  protected readonly version = computed(() => {
    const detail = this.existingDetail();
    return detail === undefined
      ? "1.0.0"
      : nextVersion(
          detail.versions.map((version) => version.version),
          this.bump()
        );
  });

  protected readonly taken = computed(() => this.mode() === "new" && this.mineValue()?.packages.some((item) => item.namespace === this.space() && item.packageId === this.packageId()) === true);
  protected readonly idProblem = computed(() => {
    if (this.taken()) {
      return `${this.spaceLabel()} already has a package called “${this.packageId()}”. Pick another name, or publish a new version of that one from My skills.`;
    }

    if (this.mode() !== "new" || this.uploadTitle().trim().length === 0) {
      return null;
    }

    if (!idPattern.test(this.packageId())) {
      return "The package ID can only use lowercase letters, digits, and single hyphens. Set one under Technical details.";
    }

    // Installed skills are named <space>-<id>, which must fit in 64 characters.
    return this.packageId().length > 63 - this.space().length
      ? `The package ID can be at most ${String(63 - this.space().length)} characters in this space. Shorten it under Technical details.`
      : null;
  });

  /** What will be sent: the written skill as one SKILL.md, or the picked files. */
  protected readonly files = computed<readonly Picked[]>(() => {
    if (this.source() === "upload") {
      return this.picked();
    }

    const description = this.skillDescription().trim();
    const body = this.skillBody().trim();
    if (description.length === 0 || body.length === 0) {
      return [];
    }

    // An edit keeps the live file's other frontmatter, such as `disable-model-invocation`.
    const extra = this.liveSkill.hasValue() ? (this.liveSkill.value()?.extra ?? []) : [];
    const text = skillMd({ name: slugify(this.packageId()), description, body, extra });
    return [{ path: "SKILL.md", file: new File([text], "SKILL.md", { type: "text/markdown" }) }];
  });

  /** For a new version: what changes against the live one, checked on the server without publishing. */
  protected readonly check = resource({
    params: () => (this.mode() === "upload" && this.source() === "upload" && this.files().length > 0 && this.existing.hasValue() ? { files: this.files(), version: this.version() } : undefined),
    loader: ({ params }) => this.api.checkPublish(this.space(), this.packageId(), this.formFor(params.files, params.version))
  });

  protected readonly checked = computed(() => (this.check.hasValue() ? this.check.value() : null));
  protected readonly checkProblem = computed(() => (this.check.error() === undefined ? null : ApiError.from(this.check.error())));

  /** A failed publish, or the package a new version is for failing to load. */
  protected readonly shownProblem = computed(() => this.problem() ?? (this.existing.error() === undefined ? null : ApiError.from(this.existing.error())));
  protected readonly spaceLabel = computed(() => this.mineValue()?.spaces.find((space) => space.namespace === this.space())?.displayName ?? this.space());
  protected readonly cancelLink = computed(() => (this.mode() === "new" ? ["/mine"] : ["/p", this.space(), this.packageId()]));

  protected readonly submitLabel = computed(() => {
    if (this.submitting()) {
      return "Sending…";
    }

    return this.suggesting() ? "Send suggestion" : `Publish version ${this.version()}`;
  });

  protected readonly ready = computed(
    () =>
      !this.submitting() &&
      this.idProblem() === null &&
      this.space() !== "" &&
      this.files().length > 0 &&
      this.writeDrops().length === 0 &&
      describeUpload(this.files())?.usable === true &&
      (this.mode() === "new" ? this.uploadTitle().trim().length > 0 : this.existing.hasValue()) &&
      (!this.suggesting() || this.message().trim().length > 0)
  );

  constructor() {
    // Opening the editor on a new version starts from the live SKILL.md.
    effect(() => {
      const skill = this.liveSkill.hasValue() ? this.liveSkill.value() : null;
      if (skill !== null) {
        untracked(() => {
          this.skillDescription.set(skill.description);
          this.skillBody.set(skill.body);
        });
      }
    });
  }

  protected setSource(value: unknown): void {
    if (value === "write" || value === "upload") {
      this.source.set(value);
    }
  }

  protected submit(event: SubmitEvent): void {
    event.preventDefault();
    if (this.ready()) {
      runTask(this.send());
    }
  }

  protected setMessage(event: Event): void {
    if (event.target instanceof HTMLTextAreaElement) {
      this.message.set(event.target.value);
    }
  }

  private async send(): Promise<void> {
    this.submitting.set(true);
    this.problem.set(null);
    try {
      const ns = this.space();
      const packageId = this.packageId();
      if (this.suggesting()) {
        const suggestion = await this.api.suggest(ns, packageId, this.message().trim(), this.form());
        this.snackBar.open("Sent. The owners will take a look.", undefined, { duration: 6000 });
        await this.router.navigate(["/suggestions", suggestion.id]);
        return;
      }

      const published = await this.api.publish(ns, packageId, this.form());
      const message = published.waitingForPublicReview
        ? `Published version ${published.version}. You and the people you share it with can use it now; everyone else sees it once an admin checks its MCP server.`
        : `Published version ${published.version}. Install it now from its page; PCs that already have it get it within about 15 minutes.`;
      // A warning stays until it is dismissed.
      this.snackBar.open([message, ...published.warnings].join(" "), "OK", published.warnings.length > 0 ? {} : { duration: 8000 });
      await this.router.navigate(["/p", ns, packageId]);
    } catch (error) {
      this.problem.set(ApiError.from(error));
    } finally {
      this.submitting.set(false);
    }
  }

  private form(): PublishForm {
    return this.formFor(this.files(), this.version());
  }

  /**
   * A zip goes as an archive; anything else as files with paths. A new version keeps the listing's
   * name, and its description follows the new SKILL.md; only a new package chooses who can install it.
   */
  private formFor(picked: readonly Picked[], version: string): PublishForm {
    const archive = pickedArchive(picked);
    return {
      version,
      changelog: this.changelog().trim(),
      tags: this.tags()
        .split(",")
        .map(slugify)
        .filter((tag) => tag.length > 0),
      files: archive === null ? picked : [],
      ...(archive === null ? {} : { archive }),
      name: this.title().trim(),
      ...(this.mode() === "new" ? { visibility: this.visibility() } : {})
    };
  }
}
