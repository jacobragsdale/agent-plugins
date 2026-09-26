import { Component, computed, inject, input, resource } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { RouterLink } from "@angular/router";
import type { IndexPackage } from "../api";
import { Api, ApiError } from "../api";
import { Session } from "../session";
import { AccessBadge } from "../shared/access-badge";
import { Icon } from "../shared/icon";
import { InstallButton } from "../shared/install-button";
import { LaneBadge } from "../shared/lane-badge";
import { PackageCard } from "../shared/package-card";
import { share, spaceWords } from "../shared/share-dialog";
import { SharedBanner } from "../shared/shared-banner";
import { runTask } from "../shared/tasks";

/** A bundle: skills someone grouped so they install together, each still installable on its own. */
@Component({
  selector: "app-bundle",
  imports: [RouterLink, MatButtonModule, MatProgressBarModule, AccessBadge, Icon, InstallButton, LaneBadge, PackageCard, SharedBanner],
  template: `
    <div class="page stack">
      <a class="back" routerLink="/browse"><app-icon name="arrowBack" />All skills</a>
      <app-shared-banner [by]="shared()" what="this bundle" />
      @if (bundle.hasValue()) {
        @let current = bundle.value();
        <header class="stack tight">
          <h1>{{ current.name }}</h1>
          <div class="meta">
            <app-lane-badge [lane]="current.lane" [publisher]="current.publisher.displayName" />
            <span>· Bundle of {{ count() }}</span>
            <app-access-badge [restricted]="current.effective === 'private'" [showPublic]="current.owned" />
          </div>
          @if (current.description.length > 0) {
            <p class="lead">{{ current.description }}</p>
          }
        </header>

        <div class="row">
          <app-install-button [target]="current.id" [covers]="current.members" label="Install all" />
          @if (current.owned) {
            <span class="spacer"></span>
            <a mat-button [routerLink]="['/b', current.namespace, current.bundleId, 'edit']"><app-icon name="edit" />Edit</a>
            <button mat-button type="button" (click)="openShare()"><app-icon name="share" />Share</button>
          }
        </div>

        <h2>Skills in this bundle</h2>
        <div class="grid">
          @for (item of members(); track item.id) {
            <app-package-card [item]="item" />
          }
        </div>
      } @else if (bundle.error(); as error) {
        <div class="card">
          <h1>{{ status(error) === 404 ? "Not found" : "Couldn't load this bundle" }}</h1>
          <p class="muted">{{ status(error) === 404 ? "This bundle doesn't exist, was deleted, or isn't shared with you." : message(error) }}</p>
          <a mat-stroked-button routerLink="/browse">Browse skills</a>
        </div>
      } @else {
        <mat-progress-bar mode="indeterminate" aria-label="Loading" />
      }
    </div>
  `,
  styles: `
    .tight {
      gap: 0.5rem;
    }
    .lead {
      margin: 0;
    }
  `
})
export class BundlePage {
  public readonly ns = input.required<string>();
  public readonly id = input.required<string>();
  /** Who shared it, when the page was opened from a share link. */
  public readonly shared = input<string>();

  private readonly api = inject(Api);
  private readonly session = inject(Session);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected readonly bundle = resource({ params: () => ({ ns: this.ns(), id: this.id() }), loader: ({ params }) => this.api.bundle(params.ns, params.id) });
  protected readonly index = resource({ loader: () => this.api.index() });

  /** Members in the bundle's order, with the index's details for each card. */
  protected readonly members = computed<readonly IndexPackage[]>(() => {
    if (!this.bundle.hasValue() || !this.index.hasValue()) {
      return [];
    }

    const packages = this.index.value().packages;
    return this.bundle.value().members.flatMap((id) => packages.find((item) => item.id === id) ?? []);
  });

  /** "3 skills", plus how many more aren't shared with you. */
  protected readonly count = computed(() => {
    const bundle = this.bundle.hasValue() ? this.bundle.value() : null;
    const members = bundle?.members.length ?? 0;
    const hidden = bundle?.hiddenMembers ?? 0;
    return `${members === 1 ? "1 skill" : `${String(members)} skills`}${hidden > 0 ? `, and ${String(hidden)} not shared with you` : ""}`;
  });

  protected status(error: unknown): number {
    return ApiError.from(error).status;
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }

  protected openShare(): void {
    runTask(this.editSharing());
  }

  private async editSharing(): Promise<void> {
    if (!this.bundle.hasValue()) {
      return;
    }

    const bundle = this.bundle.value();
    const words = spaceWords(this.session.me(), bundle.namespace, bundle.publisher.displayName);
    if (await share(this.dialog, { namespace: bundle.namespace, id: bundle.bundleId, label: bundle.name, ...words })) {
      this.snackBar.open("Sharing saved. Sharing a bundle doesn't share its skills; people see only the ones they already can.", undefined, { duration: 6000 });
      this.bundle.reload();
    }
  }
}
