import { Component, computed, inject, input, linkedSignal, resource } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { MatTabsModule } from "@angular/material/tabs";
import { RouterLink } from "@angular/router";
import type { PackageVersion } from "../api";
import { Api, ApiError } from "../api";
import { newestFirst } from "../format";
import { Session } from "../session";
import { prompt } from "../shared/dialogs";
import { Icon } from "../shared/icon";
import { describeKinds } from "../shared/package-card";
import { packageStatus } from "../shared/status";
import { runTask } from "../shared/tasks";
import { OwnerPanel, PackageHeader, PackageSide, PackageUsage, VersionFiles, VersionList } from "./package-parts";

@Component({
  selector: "app-package",
  imports: [RouterLink, MatButtonModule, MatProgressBarModule, MatTabsModule, Icon, PackageHeader, OwnerPanel, VersionFiles, VersionList, PackageUsage, PackageSide],
  templateUrl: "./package.html",
  styleUrl: "./package.scss"
})
export class PackagePage {
  public readonly ns = input.required<string>();
  public readonly pkg = input.required<string>();

  private readonly session = inject(Session);
  private readonly api = inject(Api);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected readonly detail = resource({ params: () => ({ ns: this.ns(), pkg: this.pkg() }), loader: ({ params }) => this.api.package(params.ns, params.pkg) });

  /** Publisher, lane, and install counts come from the index, which lists live packages only. */
  protected readonly listing = resource({ params: () => ({ id: `${this.ns()}/${this.pkg()}` }), loader: async ({ params }) => (await this.api.index()).find((item) => item.id === params.id) ?? null });

  protected readonly loadProblem = computed(() => (this.detail.error() === undefined ? null : ApiError.from(this.detail.error())));
  protected readonly entry = computed(() => (this.listing.hasValue() ? this.listing.value() : null));

  protected readonly owner = computed(() => {
    const me = this.session.me();
    return me !== null && (me.admin || me.namespaces.includes(this.ns()));
  });

  protected readonly canReport = computed(() => this.session.me() !== null && !this.owner());

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

  /** Readers see the live version; owners start on the newest one, pending or not. */
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

  protected withdraw(version: PackageVersion): void {
    runTask(this.confirmWithdraw(version));
  }

  protected restore(version: PackageVersion): void {
    runTask(this.confirmRestore(version));
  }

  protected report(): void {
    runTask(this.sendReport());
  }

  private async confirmWithdraw(version: PackageVersion): Promise<void> {
    const live = version.version === this.detail.value()?.liveVersion;
    const confirmed = await prompt(this.dialog, {
      title: `Withdraw version ${version.version}?`,
      message: live
        ? "People who don't have it yet won't be able to install it. The previous approved version, if any, becomes the live one again."
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
      this.listing.reload();
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }

  private async confirmRestore(version: PackageVersion): Promise<void> {
    const confirmed = await prompt(this.dialog, {
      title: `Restore version ${version.version}?`,
      message:
        version.reviewState === "approved"
          ? "It will be offered again. If it's the newest approved version, it becomes the live one."
          : "It comes back in its review state and goes live only if an admin approves it.",
      confirm: "Restore"
    });
    if (confirmed === undefined) {
      return;
    }

    try {
      await this.api.restore(this.ns(), this.pkg(), version.version);
      this.snackBar.open(`Version ${version.version} restored.`, undefined, { duration: 4000 });
      this.detail.reload();
      this.listing.reload();
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }

  private async sendReport(): Promise<void> {
    const reason = await prompt(this.dialog, {
      title: "Report a problem",
      message: "Tell the admins what's wrong: it doesn't work, it's harmful, or it contains something private. They'll take a look.",
      confirm: "Send report",
      field: { label: "What's wrong?", hint: "Up to 2,048 characters", required: true }
    });
    if (reason === undefined) {
      return;
    }

    try {
      await this.api.report(this.ns(), this.pkg(), reason);
      this.snackBar.open("Thanks. The admins have your report.", undefined, { duration: 4000 });
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }
}
