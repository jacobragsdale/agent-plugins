import { Component, computed, inject, input, linkedSignal, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatRadioModule } from "@angular/material/radio";
import { MatSnackBar } from "@angular/material/snack-bar";
import { Router, RouterLink } from "@angular/router";
import type { PublishForm } from "../api";
import { Api, ApiError } from "../api";
import type { Bump } from "../format";
import { idPattern, nextVersion, slugify } from "../format";
import { Session } from "../session";
import { Icon } from "../shared/icon";
import { runTask } from "../shared/tasks";
import type { Picked } from "./publish-parts";
import { pickedArchive, Problem, UploadPicker, VersionFields } from "./publish-parts";

type Mode = "new" | "upload" | "suggest";

const headings: Readonly<Record<Mode, readonly [string, string]>> = {
  new: ["Share a skill", "Upload a skill folder or zip. You choose who can install it."],
  upload: ["New version", "The files you upload replace the current version when you publish."],
  suggest: ["Suggest a change", "Upload your improved files. The owners decide whether to publish them."]
};

@Component({
  selector: "app-publish",
  imports: [RouterLink, MatButtonModule, MatFormFieldModule, MatInputModule, MatProgressBarModule, MatRadioModule, Icon, UploadPicker, VersionFields, Problem],
  templateUrl: "./publish.html",
  styleUrl: "./publish.scss"
})
export class PublishPage {
  public readonly mode = input.required<Mode>();
  public readonly ns = input<string>();
  public readonly pkg = input<string>();
  /** The space to publish to, preselected when a team page sent the person here. */
  public readonly to = input<string>();

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

  protected readonly spaces = computed(() => (this.mine.value()?.spaces ?? []).map((space) => ({ namespace: space.namespace, label: space.lane === "personal" ? "Just me" : space.displayName })));
  protected readonly suggesting = computed(() => this.mode() === "suggest");

  protected readonly hasTeam = computed(() => this.mine.value()?.spaces.some((space) => space.lane === "team") === true);
  protected readonly title = computed(() => (this.mode() === "new" ? this.uploadTitle() : (this.existing.value()?.name ?? "")));
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
  protected readonly idProblem = computed(() => {
    if (this.taken()) {
      return `${this.spaceLabel()} already has a package called “${this.packageId()}”. Pick another name, or publish a new version of that one from My skills.`;
    }

    return this.mode() === "new" && this.uploadTitle().trim().length > 0 && !idPattern.test(this.packageId())
      ? "The package ID can only use lowercase letters, digits, and single hyphens. Set one under Technical details."
      : null;
  });

  /** A failed publish, or the package a new version is for failing to load. */
  protected readonly shownProblem = computed(() => this.problem() ?? (this.existing.error() === undefined ? null : ApiError.from(this.existing.error())));
  protected readonly spaceLabel = computed(() => this.mine.value()?.spaces.find((space) => space.namespace === this.space())?.displayName ?? this.space());
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
      idPattern.test(this.packageId()) &&
      !this.taken() &&
      this.space() !== "" &&
      this.picked().length > 0 &&
      (this.mode() === "new" ? this.uploadTitle().trim().length > 0 : this.existing.hasValue()) &&
      (!this.suggesting() || this.message().trim().length > 0)
  );

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
        : `Published version ${published.version}. It's live now.`;
      this.snackBar.open(message, undefined, { duration: 8000 });
      await this.router.navigate(["/p", ns, packageId]);
    } catch (error) {
      this.problem.set(ApiError.from(error));
    } finally {
      this.submitting.set(false);
    }
  }

  /** A zip goes as an archive; anything else as files with paths. New versions keep the listing. */
  private form(): PublishForm {
    const picked = this.picked();
    const archive = pickedArchive(picked);
    const files = archive === null ? picked : [];
    const detail = this.existing.value();
    return {
      version: this.version(),
      changelog: this.changelog().trim(),
      tags: this.tags()
        .split(",")
        .map(slugify)
        .filter((tag) => tag.length > 0),
      files,
      ...(archive === null ? {} : { archive }),
      name: this.title().trim(),
      ...(detail === undefined ? {} : { description: detail.description })
    };
  }
}
