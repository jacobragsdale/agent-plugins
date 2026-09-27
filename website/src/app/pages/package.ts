import { Component, computed, effect, inject, input, linkedSignal, resource } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { MatTabsModule } from "@angular/material/tabs";
import { Title } from "@angular/platform-browser";
import { RouterLink } from "@angular/router";
import { z } from "zod";
import type { PackageVersion, ReportKind } from "../api";
import { Api, ApiError, versionFiles } from "../api";
import { newestFirst } from "../format";
import { Session } from "../session";
import { prompt } from "../shared/dialogs";
import { Icon } from "../shared/icon";
import { describeKinds } from "../shared/package-card";
import { spaceWords } from "../shared/share-dialog";
import { SharedBanner } from "../shared/shared-banner";
import { packageStatus } from "../shared/status";
import { SuggestionList } from "../shared/suggestion-list";
import { runTask } from "../shared/tasks";
import { OwnerPanel, PackageHeader, PackageReports, PackageSide, PackageUsage, VersionFiles, VersionList } from "./package-parts";

/** Just enough of agent-plugins.json to list a pack's skills. */
const manifestSchema = z.object({ packages: z.array(z.object({ components: z.array(z.object({ kind: z.string(), id: z.string().optional() })) })) });

@Component({
  selector: "app-package",
  imports: [
    RouterLink,
    MatButtonModule,
    MatProgressBarModule,
    MatTabsModule,
    Icon,
    PackageHeader,
    OwnerPanel,
    VersionFiles,
    VersionList,
    PackageUsage,
    PackageReports,
    PackageSide,
    SuggestionList,
    SharedBanner
  ],
  templateUrl: "./package.html",
  styleUrl: "./package.scss"
})
export class PackagePage {
  public readonly ns = input.required<string>();
  public readonly pkg = input.required<string>();
  /** Who shared it, when the page was opened from a share link. */
  public readonly shared = input<string>();

  protected readonly session = inject(Session);
  private readonly api = inject(Api);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected readonly detail = resource({ params: () => ({ ns: this.ns(), pkg: this.pkg() }), loader: ({ params }) => this.api.package(params.ns, params.pkg) });

  protected readonly loadProblem = computed(() => (this.detail.error() === undefined ? null : ApiError.from(this.detail.error())));

  protected readonly owner = computed(() => {
    if (this.detail.hasValue()) {
      return this.detail.value().owned;
    }

    const me = this.session.me();
    return me !== null && (me.admin || me.namespaces.includes(this.ns()));
  });

  protected readonly live = computed(() => this.detail.hasValue() && this.detail.value().liveVersion !== null && !this.detail.value().revoked);
  protected readonly canReport = computed(() => this.session.me() !== null && !this.owner());
  protected readonly canSuggest = computed(() => this.canReport() && this.live());
  protected readonly space = computed(() => spaceWords(this.session.me(), this.ns(), this.detail.value()?.publisher.displayName ?? this.ns()));

  /** Owners see every suggestion; anyone else sees their own, so the tab shows only when there is something. */
  protected readonly suggestions = resource({
    params: () => (this.session.me() !== null && this.detail.hasValue() ? { ns: this.ns(), pkg: this.pkg() } : undefined),
    loader: ({ params }) => this.api.suggestions(params.ns, params.pkg)
  });

  protected readonly suggestionRows = computed(() => (this.suggestions.hasValue() ? this.suggestions.value() : []));
  protected readonly waiting = computed(() => this.suggestionRows().filter((suggestion) => suggestion.state === "pending").length);

  /** A pack's skills, read from the live version's manifest, so each can be installed on its own. */
  protected readonly skills = resource({
    params: () => (this.live() ? { files: versionFiles(this.ns(), this.pkg(), this.detail.value()?.liveVersion ?? "") } : undefined),
    loader: async ({ params }) => {
      const manifest = manifestSchema.safeParse(JSON.parse(await this.api.fileText(params.files, "agent-plugins.json")));
      const skills = manifest.success ? (manifest.data.packages[0]?.components ?? []).filter((component) => component.kind === "skill").flatMap((component) => component.id ?? []) : [];
      return skills.length > 1 ? skills : [];
    }
  });

  protected readonly packSkills = computed(() => (this.skills.hasValue() ? this.skills.value() : []));

  /** The live version is one SKILL.md (beside the manifest the server writes), so the browser editor can change it. */
  protected readonly liveFiles = resource({
    params: () => (this.owner() && this.live() ? { files: versionFiles(this.ns(), this.pkg(), this.detail.value()?.liveVersion ?? "") } : undefined),
    loader: ({ params }) => this.api.files(params.files)
  });

  protected readonly editable = computed(() => {
    const files = this.liveFiles.hasValue() ? this.liveFiles.value().filter((file) => file.path !== "agent-plugins.json") : [];
    return files.length === 1 && files[0]?.path.endsWith("SKILL.md") === true;
  });

  protected readonly stats = resource({
    params: () => (this.owner() && this.detail.hasValue() && this.detail.value().liveVersion !== null ? { ns: this.ns(), pkg: this.pkg() } : undefined),
    loader: ({ params }) => this.api.stats(params.ns, params.pkg)
  });

  protected readonly usage = computed(() => (this.stats.hasValue() ? this.stats.value() : null));

  protected readonly versions = computed<readonly PackageVersion[]>(() => {
    if (!this.detail.hasValue()) {
      return [];
    }

    const detail = this.detail.value();
    return newestFirst(detail.versions.map((version) => version.version)).flatMap((number) => detail.versions.filter((version) => version.version === number));
  });

  /** Readers see the live version; owners start on the newest one that isn't withdrawn. */
  protected readonly shown = linkedSignal<string | null>(() => {
    if (!this.detail.hasValue()) {
      return null;
    }

    const live = this.detail.value().liveVersion;
    return this.owner() ? (this.versions().find((version) => !version.yanked)?.version ?? live) : live;
  });

  protected readonly shownVersion = computed(() => this.versions().find((version) => version.version === this.shown()) ?? null);
  protected readonly ownerStatus = computed(() => (this.owner() && this.detail.hasValue() ? packageStatus(this.detail.value()) : null));
  protected readonly kinds = computed(() => describeKinds(this.shownVersion()?.componentKinds ?? []));

  constructor() {
    const title = inject(Title);
    effect(() => {
      const name = this.detail.value()?.name;
      if (name !== undefined) {
        title.setTitle(`${name} · Agent Plugins`);
      }
    });
  }

  protected purge(version: PackageVersion): void {
    runTask(this.confirmPurge(version));
  }

  protected withdraw(version: PackageVersion): void {
    runTask(this.confirmWithdraw(version));
  }

  protected restore(version: PackageVersion): void {
    runTask(this.confirmRestore(version));
  }

  protected report(kind: ReportKind): void {
    runTask(this.sendReport(kind));
  }

  private async confirmPurge(version: PackageVersion): Promise<void> {
    const confirmed = await prompt(this.dialog, {
      title: `Purge version ${version.version}?`,
      message:
        "Its files are deleted for good, from storage and from every suggestion based on it, and its number can't be used again. PCs that have it keep their copy until the next version reaches them; remove the package from every PC as well if the files must go everywhere.",
      confirm: "Purge",
      danger: true
    });
    if (confirmed === undefined) {
      return;
    }

    try {
      await this.api.purgeVersion(this.ns(), this.pkg(), version.version);
      this.snackBar.open(`Version ${version.version} purged.`, undefined, { duration: 4000 });
      this.detail.reload();
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }

  private async confirmWithdraw(version: PackageVersion): Promise<void> {
    const live = version.version === this.detail.value()?.liveVersion;
    const confirmed = await prompt(this.dialog, {
      title: `Withdraw version ${version.version}?`,
      message: live
        ? "The previous version, if any, becomes the live one again, and PCs that have this one go back to it at their next check."
        : "It will no longer be offered to anyone. You can restore it later.",
      confirm: "Withdraw",
      danger: true
    });
    if (confirmed === undefined) {
      return;
    }

    try {
      await this.api.withdraw(this.ns(), this.pkg(), version.version);
      this.snackBar.open(`Version ${version.version} withdrawn.`, undefined, { duration: 4000 });
      this.detail.reload();
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }

  private async confirmRestore(version: PackageVersion): Promise<void> {
    const confirmed = await prompt(this.dialog, {
      title: `Restore version ${version.version}?`,
      message: "It will be offered again. If it's the newest version, it becomes the live one.",
      confirm: "Restore"
    });
    if (confirmed === undefined) {
      return;
    }

    try {
      await this.api.restore(this.ns(), this.pkg(), version.version);
      this.snackBar.open(`Version ${version.version} restored.`, undefined, { duration: 4000 });
      this.detail.reload();
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }

  private async sendReport(kind: ReportKind): Promise<void> {
    const reason = await prompt(
      this.dialog,
      kind === "problem"
        ? {
            title: "Report a problem",
            message: "Tell the owners and the admins what's wrong: it doesn't work, it's harmful, or it contains something private.",
            confirm: "Send report",
            field: { label: "What's wrong?", hint: "Up to 2,048 characters", required: true }
          }
        : {
            title: "Tell the owners",
            message: "What works, what it gets wrong, what you wish it did. No files needed.",
            confirm: "Send",
            field: { label: "Your feedback", hint: "Up to 2,048 characters", required: true }
          }
    );
    if (reason === undefined) {
      return;
    }

    try {
      await this.api.report(this.ns(), this.pkg(), reason, kind);
      this.snackBar.open(kind === "problem" ? "Thanks. The owners and the admins have your report." : "Thanks. The owners have your feedback.", undefined, { duration: 4000 });
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }
}
