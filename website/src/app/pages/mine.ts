import { Component, computed, inject, input, output, resource } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { RouterLink } from "@angular/router";
import type { Bundle, PackageDetail, Space, Suggestion } from "../api";
import { Api, ApiError } from "../api";
import { formatAge } from "../format";
import { Session } from "../session";
import type { IconName } from "../shared/icon";
import { Icon } from "../shared/icon";
import { share, spaceWords } from "../shared/share-dialog";
import type { PackageStatus } from "../shared/status";
import { packageStatus } from "../shared/status";
import { SuggestionList } from "../shared/suggestion-list";
import { runTask } from "../shared/tasks";

interface Row {
  readonly item: PackageDetail;
  readonly status: PackageStatus;
  readonly icon: IconName;
  readonly updated: string;
}

interface Group {
  readonly space: Space;
  readonly label: string;
  readonly badge: string;
  readonly rows: readonly Row[];
  readonly bundles: readonly Bundle[];
}

const statusIcons: Readonly<Record<PackageStatus["tone"], IconName>> = { live: "check", pending: "schedule", rejected: "flag" };

const noSuggestions = { waiting: [], yours: [] } as const;

function laneLabel(space: Space): string {
  switch (space.lane) {
    case "official":
      return "Official";
    case "team":
      return "Team space";
    case "personal":
      return "Your space";
  }
}

/** Suggestions waiting for your answer, then your own; nothing when there are none. */
@Component({
  selector: "app-mine-suggestions",
  imports: [SuggestionList],
  template: `
    @if (waiting().length > 0) {
      <section class="card stack">
        <h2>Waiting on you ({{ waiting().length }})</h2>
        <p class="muted">People suggested changes to your skills. Open one to read it, then publish it or say why not.</p>
        <app-suggestion-list [suggestions]="waiting()" [showPackage]="true" />
      </section>
    }
    @if (yours().length > 0) {
      <section class="stack">
        <h2>Your suggestions</h2>
        <app-suggestion-list [suggestions]="yours()" [showPackage]="true" />
      </section>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
    }
    h2 {
      font: var(--mat-sys-title-large);
      margin: 0;
    }
    p {
      margin: 0;
    }
  `
})
export class MineSuggestions {
  public readonly waiting = input.required<readonly Suggestion[]>();
  public readonly yours = input.required<readonly Suggestion[]>();
}

/** One space: its packages with their status, and its bundles. */
@Component({
  selector: "app-space-section",
  imports: [RouterLink, MatButtonModule, Icon],
  template: `
    <div class="row">
      <h2>{{ group().space.displayName }}</h2>
      <span [class]="group().badge">{{ group().label }}</span>
      <span class="badge"><app-icon [name]="group().space.visibility === 'private' ? 'lock' : 'visibility'" />{{ group().space.visibility === "private" ? "Private" : "Public" }}</span>
      <span class="spacer"></span>
      @if (group().space.lane === "team") {
        <a mat-button [routerLink]="['/teams', group().space.namespace]"><app-icon name="group" />Team</a>
      }
      @if (group().space.lane !== "team" || group().space.role === "owner") {
        <button mat-button type="button" (click)="share.emit(group().space)"><app-icon name="share" />Who can see this space</button>
      }
    </div>
    <ul class="rows">
      @for (row of group().rows; track row.item.id) {
        <li class="card">
          <div class="text">
            <a class="name" [routerLink]="['/p', row.item.namespace, row.item.packageId]">{{ row.item.name }}</a>
            <p class="muted">{{ row.item.description }}</p>
            <div class="row status">
              <span [class]="row.status.badge"><app-icon [name]="row.icon" />{{ row.status.label }}</span>
              <span class="muted">{{ row.status.detail }}</span>
            </div>
          </div>
          <div class="actions">
            <span class="muted">Updated {{ row.updated }}</span>
            <a mat-stroked-button [routerLink]="['/p', row.item.namespace, row.item.packageId]">Open</a>
          </div>
        </li>
      } @empty {
        <li class="muted empty">Nothing here yet.</li>
      }
      @for (bundle of group().bundles; track bundle.id) {
        <li class="card">
          <div class="text">
            <a class="name" [routerLink]="['/b', bundle.namespace, bundle.bundleId]"><app-icon name="layers" /> {{ bundle.name }}</a>
            <p class="muted">Bundle · {{ bundle.members.length === 1 ? "1 skill" : bundle.members.length + " skills" }}</p>
          </div>
          <div class="actions">
            <a mat-stroked-button [routerLink]="['/b', bundle.namespace, bundle.bundleId, 'edit']">Edit</a>
          </div>
        </li>
      }
    </ul>
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
      margin-top: 1rem;
    }
    h2 {
      font: var(--mat-sys-title-large);
      margin: 0;
    }
    .empty {
      list-style: none;
    }
    .rows {
      list-style: none;
      margin: 0;
      padding: 0;
      display: grid;
      gap: 0.75rem;
    }
    li {
      display: flex;
      flex-wrap: wrap;
      gap: 1rem;
      align-items: center;
      padding: 1rem 1.25rem;
    }
    .text {
      flex: 1 1 24rem;
      p {
        margin: 0.25rem 0 0.5rem;
      }
    }
    .name {
      display: inline-flex;
      align-items: center;
      gap: 0.3rem;
      font: var(--mat-sys-title-medium);
      text-decoration: none;
      color: inherit;
      &:hover {
        color: var(--mat-sys-primary);
      }
    }
    .status {
      gap: 0.5rem;
      font: var(--mat-sys-body-small);
    }
    .actions {
      display: flex;
      align-items: center;
      gap: 1rem;
      font: var(--mat-sys-body-small);
    }
  `
})
export class SpaceSection {
  public readonly group = input.required<Group>();
  public readonly share = output<Space>();
}

@Component({
  selector: "app-mine",
  imports: [RouterLink, MatButtonModule, MatProgressBarModule, Icon, MineSuggestions, SpaceSection],
  template: `
    <div class="page stack">
      <header class="row">
        <div>
          <h1>My skills</h1>
          <p class="lead">Everything you've shared, and whether it's live.</p>
        </div>
        <span class="spacer"></span>
        <a mat-stroked-button routerLink="/bundles/new"><app-icon name="layers" />New bundle</a>
        <a mat-flat-button routerLink="/publish"><app-icon name="add" />Share a skill</a>
      </header>

      @if (session.state().kind === "signed-out") {
        <div class="card">
          <h2>Sign in to see your skills</h2>
          <p class="muted">Open this page on your work PC; Windows signs you in automatically.</p>
        </div>
      } @else if (mine.error(); as error) {
        <p class="problem">{{ message(error) }}</p>
      } @else if (mine.isLoading()) {
        <mat-progress-bar mode="indeterminate" aria-label="Loading your skills" />
      } @else {
        <app-mine-suggestions [waiting]="waiting()" [yours]="yours()" />
        @for (group of groups(); track group.space.namespace) {
          <app-space-section [group]="group" (share)="shareSpace($event)" />
        }
      }
    </div>
  `,
  styles: `
    h2 {
      font: var(--mat-sys-title-large);
      margin: 0;
    }
  `
})
export class MinePage {
  protected readonly session = inject(Session);
  private readonly api = inject(Api);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected readonly mine = resource({ loader: () => this.api.mine() });

  protected readonly groups = computed<readonly Group[]>(() => {
    if (!this.mine.hasValue()) {
      return [];
    }

    const { spaces, packages, bundles } = this.mine.value();
    return spaces.map((space) => ({
      space,
      label: laneLabel(space),
      badge: `badge ${space.lane}`,
      bundles: bundles.filter((bundle) => bundle.namespace === space.namespace),
      rows: packages
        .filter((item) => item.namespace === space.namespace)
        .map((item) => {
          const status = packageStatus(item);
          return {
            item,
            status,
            icon: statusIcons[status.tone],
            updated: formatAge(item.versions.map((version) => version.publishedAt).reduce((newest, at) => (Date.parse(at) > Date.parse(newest) ? at : newest)))
          };
        })
    }));
  });

  private readonly suggestions = computed(() => (this.mine.hasValue() ? this.mine.value().suggestions : noSuggestions));
  protected readonly waiting = computed(() => this.suggestions().waiting);
  protected readonly yours = computed(() => this.suggestions().yours.filter((suggestion) => suggestion.state !== "withdrawn"));

  protected shareSpace(space: Space): void {
    runTask(this.editSharing(space));
  }

  private async editSharing(space: Space): Promise<void> {
    const words = spaceWords(this.session.me(), space.namespace, space.displayName);
    if (await share(this.dialog, { namespace: space.namespace, label: `everything in ${words.spaceName}`, ...words })) {
      this.snackBar.open("Saved. A skill's own setting still wins over its space's.", undefined, { duration: 5000 });
      this.mine.reload();
    }
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }
}
