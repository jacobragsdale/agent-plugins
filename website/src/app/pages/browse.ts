import { Component, computed, inject, input, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSelectModule } from "@angular/material/select";
import { RouterLink } from "@angular/router";
import type { IndexBundle, IndexPackage, Lane } from "../api";
import { Api, ApiError } from "../api";
import { BundleGrid } from "../shared/bundle-card";
import { Icon } from "../shared/icon";
import { PackageCard } from "../shared/package-card";
import { SharedBanner } from "../shared/shared-banner";

type LaneFilter = Lane | "all";

/** What narrowed the list: someone shared a space, or a team page asked for its skills. */
@Component({
  selector: "app-browse-scope",
  imports: [RouterLink, SharedBanner],
  template: `
    <app-shared-banner [by]="shared()" what="these skills" />
    @if (space() !== undefined) {
      <p class="notice row">
        Showing {{ spaceName() }} only.
        <a routerLink="/browse">Show everything</a>
      </p>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
    }
    p {
      margin: 0;
      gap: 0.5rem;
    }
  `
})
export class BrowseScope {
  public readonly shared = input.required<string | undefined>();
  public readonly space = input.required<string | undefined>();
  public readonly spaceName = input.required<string | undefined>();
}

@Component({
  selector: "app-browse",
  imports: [RouterLink, MatButtonModule, MatButtonToggleModule, MatFormFieldModule, MatInputModule, MatProgressBarModule, MatSelectModule, Icon, PackageCard, BundleGrid, BrowseScope],
  template: `
    <div class="page stack">
      <div class="page-head">
        <h1>Skills</h1>
        <a mat-button routerLink="/bundles/new"><app-icon name="layers" />New bundle</a>
      </div>

      <app-browse-scope [shared]="shared()" [space]="space()" [spaceName]="spaceName()" />

      <div class="filters">
        <mat-form-field appearance="outline" subscriptSizing="dynamic" class="search">
          <app-icon matPrefix name="search" class="prefix" />
          <input matInput type="search" aria-label="Search skills" placeholder="Search skills" [value]="query()" (input)="setQuery($event)" />
        </mat-form-field>
        <mat-button-toggle-group aria-label="Who published it" hideSingleSelectionIndicator [value]="lane()" (change)="setLane($event.value)">
          <mat-button-toggle value="all">All</mat-button-toggle>
          <mat-button-toggle value="official">Official</mat-button-toggle>
          <mat-button-toggle value="team">Teams</mat-button-toggle>
          <mat-button-toggle value="personal">People</mat-button-toggle>
        </mat-button-toggle-group>
        @if (lane() === "team" && teams().length > 1) {
          <mat-form-field appearance="outline" subscriptSizing="dynamic" class="teams">
            <mat-select multiple aria-label="Teams" placeholder="All teams" [value]="selectedTeams()" (selectionChange)="setTeams($event.value)">
              @for (team of teams(); track team.namespace) {
                <mat-option [value]="team.namespace">{{ team.displayName }} ({{ team.count }})</mat-option>
              }
            </mat-select>
          </mat-form-field>
        }
      </div>

      @if (index.hasValue()) {
        <app-bundle-grid [bundles]="bundles()" />
        <section class="stack" aria-labelledby="skills-heading">
          <h2 id="skills-heading">
            Skills <span class="count">{{ results().length }}</span>
          </h2>
          <p class="visually-hidden" aria-live="polite">{{ summary() }}</p>
          @if (results().length > 0) {
            <div class="grid">
              @for (item of results(); track item.id) {
                <app-package-card [item]="item" />
              }
            </div>
          } @else {
            <p class="muted">No skills match. <a routerLink="/publish">Share the one you were looking for</a>.</p>
          }
        </section>
      } @else if (index.error(); as error) {
        <p class="problem">{{ message(error) }}</p>
      } @else {
        <mat-progress-bar mode="indeterminate" aria-label="Loading skills" />
      }
    </div>
  `,
  styles: `
    .filters {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 0.75rem;
    }
    .search {
      flex: 0 1 26rem;
    }
    .teams {
      flex: 0 1 14rem;
    }
    .prefix {
      margin: 0 0.25rem 0 0.75rem;
      color: var(--muted);
    }
    .count {
      margin-left: 0.25rem;
      color: var(--muted);
      font-weight: 400;
    }
    section {
      margin-top: 0.5rem;
    }
  `
})
export class BrowsePage {
  /** One space's skills, when a link or a team page asked for them. */
  public readonly space = input<string>();
  /** Who shared the space, when the page was opened from a share link. */
  public readonly shared = input<string>();

  protected readonly query = signal("");
  protected readonly lane = signal<LaneFilter>("all");
  /** Team namespaces to show; only set while the Teams lane is on, and empty shows every team. */
  protected readonly selectedTeams = signal<readonly string[]>([]);
  private readonly api = inject(Api);

  protected readonly index = resource({ loader: () => this.api.index() });

  /** Every team with a visible skill, for the picker. */
  protected readonly teams = computed(() => {
    const teams = new Map<string, { namespace: string; displayName: string; count: number }>();
    for (const item of this.index.hasValue() ? this.index.value().packages : []) {
      if (item.lane === "team") {
        const team = teams.get(item.namespace) ?? { namespace: item.namespace, displayName: item.publisher.displayName, count: 0 };
        team.count += 1;
        teams.set(item.namespace, team);
      }
    }
    return [...teams.values()].toSorted((left, right) => left.displayName.localeCompare(right.displayName));
  });

  private readonly words = computed(() =>
    this.query()
      .toLowerCase()
      .split(/\s+/u)
      .filter((word) => word.length > 0)
  );

  /** Whether an item passes the lane, team, space, and search filters. */
  private matches(item: IndexPackage | IndexBundle, text: readonly string[]): boolean {
    const lane = this.lane();
    const teams = this.selectedTeams();
    const space = this.space();
    const haystack = [item.name, item.description, item.id, item.publisher.displayName, ...text].join(" ").toLowerCase();
    return (
      (lane === "all" || item.lane === lane) &&
      (teams.length === 0 || teams.includes(item.namespace)) &&
      (space === undefined || item.namespace === space) &&
      this.words().every((word) => haystack.includes(word))
    );
  }

  protected readonly spaceName = computed(
    () => (this.index.hasValue() ? this.index.value().packages.find((item) => item.namespace === this.space())?.publisher.displayName : undefined) ?? this.space()
  );

  protected readonly bundles = computed<readonly IndexBundle[]>(() => (this.index.hasValue() ? this.index.value().bundles.filter((bundle) => this.matches(bundle, [])) : []));

  protected readonly results = computed<readonly IndexPackage[]>(() => {
    if (!this.index.hasValue()) {
      return [];
    }

    return this.index
      .value()
      .packages.filter((item) => this.matches(item, item.tags))
      .toSorted((left, right) => (right.installedBase !== left.installedBase ? right.installedBase - left.installedBase : left.name.localeCompare(right.name)));
  });

  protected readonly summary = computed(() => {
    const count = this.results().length;
    return count === 1 ? "1 skill" : `${String(count)} skills`;
  });

  protected setQuery(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.query.set(event.target.value);
    }
  }

  protected setLane(value: unknown): void {
    if (value === "all" || value === "official" || value === "team" || value === "personal") {
      this.lane.set(value);
      if (value !== "team") {
        this.selectedTeams.set([]);
      }
    }
  }

  protected setTeams(value: unknown): void {
    if (Array.isArray(value)) {
      this.selectedTeams.set(value.filter((team): team is string => typeof team === "string"));
      this.lane.set("team");
    }
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }
}
