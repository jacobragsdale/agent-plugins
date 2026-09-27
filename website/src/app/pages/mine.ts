import { Component, computed, inject, input, output, resource } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { RouterLink } from "@angular/router";
import type { Bundle, PackageDetail, Report, Space, Suggestion } from "../api";
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
      return "Team";
    case "personal":
      return "Personal";
  }
}

/** Suggestions waiting for your answer, then your own; nothing when there are none. */
@Component({
  selector: "app-mine-suggestions",
  imports: [SuggestionList],
  template: `
    @if (waiting().length > 0) {
      <section class="card stack">
        <h2>
          Suggestions waiting for you <span class="count">{{ waiting().length }}</span>
        </h2>
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
    .count {
      color: var(--muted);
      font-weight: 400;
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
    <div class="head">
      <h2>{{ group().space.displayName }}</h2>
      <span class="meta">{{ group().label }} · {{ group().space.visibility === "private" ? "Private" : "Public" }}</span>
      <span class="spacer"></span>
      @if (group().space.lane === "team") {
        <a mat-button [routerLink]="['/teams', group().space.namespace]"><app-icon name="group" />Team</a>
      }
      @if (group().space.lane !== "team" || group().space.role === "owner") {
        <button mat-button type="button" (click)="share.emit(group().space)"><app-icon name="share" />Who can see it</button>
      }
    </div>
    <ul class="list-box">
      @for (row of group().rows; track row.item.id) {
        <li>
          <div class="text">
            <a class="name" [routerLink]="['/p', row.item.namespace, row.item.packageId]">{{ row.item.name }}</a>
            <p class="muted">{{ row.status.tone === "live" ? row.item.description : row.status.detail }}</p>
          </div>
          <span class="uses muted">{{ row.item.installedBase === 1 ? "1 using it" : row.item.installedBase + " using it" }}</span>
          <span [class]="row.status.badge"><app-icon [name]="row.icon" />{{ row.status.label }}</span>
          <span class="when muted">{{ row.updated }}</span>
          <button mat-icon-button type="button" [attr.aria-label]="'Share ' + row.item.name" (click)="sharePackage.emit(row.item)"><app-icon name="share" /></button>
        </li>
      }
      @for (bundle of group().bundles; track bundle.id) {
        <li>
          <div class="text">
            <a class="name" [routerLink]="['/b', bundle.namespace, bundle.bundleId]"><app-icon name="layers" />{{ bundle.name }}</a>
            <p class="muted">Bundle · {{ bundle.members.length === 1 ? "1 skill" : bundle.members.length + " skills" }}</p>
          </div>
          <a mat-button [routerLink]="['/b', bundle.namespace, bundle.bundleId, 'edit']">Edit</a>
        </li>
      }
      @if (group().rows.length + group().bundles.length === 0) {
        <li class="muted">Nothing here yet. <a [routerLink]="['/publish']" [queryParams]="{ to: group().space.namespace }">Share a skill here</a>.</li>
      }
    </ul>
  `,
  styles: `
    :host {
      display: grid;
      grid-template-columns: minmax(0, 1fr);
      gap: 0.5rem;
      margin-top: 1rem;
    }
    .head {
      display: flex;
      flex-wrap: wrap;
      align-items: baseline;
      gap: 0.25rem 0.75rem;
    }
    li {
      display: flex;
      align-items: center;
      gap: 0.5rem 1rem;
    }
    .text {
      flex: 1;
      min-width: 0;
      p {
        margin: 0.125rem 0 0;
        font: var(--mat-sys-body-medium);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
    }
    .name {
      display: inline-flex;
      align-items: center;
      gap: 0.35rem;
      font-weight: 600;
      text-decoration: none;
      color: inherit;
      &:hover {
        color: var(--mat-sys-primary);
      }
    }
    .when {
      flex: 0 0 7.5rem;
      text-align: right;
      font: var(--mat-sys-body-small);
    }
    .uses {
      font: var(--mat-sys-body-small);
      white-space: nowrap;
    }
    @media (max-width: 600px) {
      li {
        flex-wrap: wrap;
      }
      .text {
        flex-basis: 100%;
      }
      .when {
        flex-basis: auto;
      }
    }
  `
})
export class SpaceSection {
  public readonly group = input.required<Group>();
  public readonly share = output<Space>();
  public readonly sharePackage = output<PackageDetail>();
}

/** What you reported or told owners, and their answers. */
@Component({
  selector: "app-my-reports",
  imports: [RouterLink],
  template: `
    @if (reports().length > 0) {
      <section class="stack">
        <h2>Your reports and feedback</h2>
        <ul class="list-box">
          @for (report of reports(); track report.id) {
            <li>
              <div class="row">
                <a [routerLink]="['/p', ...report.packageId.split('/')]"
                  ><code>{{ report.packageId }}</code></a
                >
                <span class="muted">{{ report.kind === "problem" ? "Problem" : "Feedback" }} · {{ age(report.createdAt) }}</span>
                <span class="spacer"></span>
                <span [class]="report.resolvedAt === null ? 'badge pending' : 'badge live'">{{ report.resolvedAt === null ? "Open" : "Answered" }}</span>
              </div>
              <p class="muted">{{ report.reason }}</p>
              @if (report.note; as note) {
                <p><strong>The answer:</strong> {{ note }}</p>
              }
            </li>
          }
        </ul>
      </section>
    }
  `,
  styles: `
    p {
      margin: 0.35rem 0 0;
      white-space: pre-line;
    }
  `
})
export class MyReports {
  public readonly reports = input.required<readonly Report[]>();

  protected age(at: string): string {
    return formatAge(at);
  }
}

@Component({
  selector: "app-mine",
  imports: [RouterLink, MatButtonModule, MatProgressBarModule, Icon, MineSuggestions, SpaceSection, MyReports],
  template: `
    <div class="page stack">
      <div class="page-head">
        <h1>My skills</h1>
        <a mat-button routerLink="/bundles/new"><app-icon name="layers" />New bundle</a>
      </div>

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
          <app-space-section [group]="group" (share)="shareSpace($event)" (sharePackage)="sharePackage($event)" />
        }
        <app-my-reports [reports]="reports.hasValue() ? reports.value() : []" />
      }
    </div>
  `
})
export class MinePage {
  protected readonly session = inject(Session);
  private readonly api = inject(Api);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected readonly mine = resource({ loader: () => this.api.mine() });
  protected readonly reports = resource({ loader: () => this.api.myReports() });

  protected readonly groups = computed<readonly Group[]>(() => {
    if (!this.mine.hasValue()) {
      return [];
    }

    const { spaces, packages, bundles } = this.mine.value();
    return spaces.map((space) => ({
      space,
      label: laneLabel(space),
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

  protected sharePackage(item: PackageDetail): void {
    runTask(this.editPackageSharing(item));
  }

  private async editPackageSharing(item: PackageDetail): Promise<void> {
    const words = spaceWords(this.session.me(), item.namespace, item.publisher.displayName);
    if (await share(this.dialog, { namespace: item.namespace, id: item.packageId, label: item.name, ...words })) {
      this.snackBar.open("Sharing saved.", undefined, { duration: 4000 });
      this.mine.reload();
    }
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
