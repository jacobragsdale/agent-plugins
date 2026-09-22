import { Component, computed, inject, resource } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { RouterLink } from "@angular/router";
import type { PackageDetail, Space } from "../api";
import { Api, ApiError } from "../api";
import { formatAge } from "../format";
import { Session } from "../session";
import { editVisibility } from "../shared/dialogs";
import type { IconName } from "../shared/icon";
import { Icon } from "../shared/icon";
import type { PackageStatus } from "../shared/status";
import { newestVersion, packageStatus } from "../shared/status";
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
}

const statusIcons: Readonly<Record<PackageStatus["tone"], IconName>> = { live: "check", pending: "schedule", rejected: "flag" };

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

@Component({
  selector: "app-mine",
  imports: [RouterLink, MatButtonModule, MatProgressBarModule, Icon],
  template: `
    <div class="page stack">
      <header class="row">
        <div>
          <h1>My skills</h1>
          <p class="lead">Everything you've shared, and whether it's live.</p>
        </div>
        <span class="spacer"></span>
        <a mat-flat-button routerLink="/publish"><app-icon name="add" />Share a skill</a>
      </header>

      @if (session.state().kind !== "loading" && session.me() === null) {
        <div class="card">
          <h2>Sign in to see your skills</h2>
          <p class="muted">Open this page on your work PC; Windows signs you in automatically.</p>
        </div>
      } @else if (mine.error(); as error) {
        <p class="problem">{{ message(error) }}</p>
      } @else if (mine.isLoading()) {
        <mat-progress-bar mode="indeterminate" aria-label="Loading your skills" />
      } @else {
        @for (group of groups(); track group.space.namespace) {
          <section class="stack">
            <div class="row">
              <h2>{{ group.space.displayName }}</h2>
              <span [class]="group.badge">{{ group.label }}</span>
              <span class="spacer"></span>
              <button mat-button type="button" (click)="visibility(group.space)"><app-icon name="visibility" />Who can see this space</button>
            </div>
            <ul class="rows">
              @for (row of group.rows; track row.item.id) {
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
            </ul>
          </section>
        }
      }
    </div>
  `,
  styles: `
    section {
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

    const { spaces, packages } = this.mine.value();
    return spaces.map((space) => ({
      space,
      label: laneLabel(space),
      badge: `badge ${space.lane}`,
      rows: packages
        .filter((item) => item.namespace === space.namespace)
        .map((item) => {
          const status = packageStatus(item);
          return { item, status, icon: statusIcons[status.tone], updated: formatAge(newestVersion(item)?.publishedAt ?? new Date().toISOString()) };
        })
    }));
  });

  protected visibility(space: Space): void {
    runTask(this.editVisibility(space));
  }

  private async editVisibility(space: Space): Promise<void> {
    if (await editVisibility(this.dialog, { namespace: space.namespace, label: `everything in ${space.displayName}` })) {
      this.snackBar.open("Visibility saved. A package's own setting still wins over its space's.", undefined, { duration: 5000 });
    }
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }
}
