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
import { formatAge } from "../format";
import { Session } from "../session";
import { BundleGrid } from "../shared/bundle-card";
import { Icon } from "../shared/icon";
import { PackageCard } from "../shared/package-card";
import { SharedBanner } from "../shared/shared-banner";

type LaneFilter = Lane | "all";

/** What narrowed the list: someone shared a space, or a team page asked for its skills. */
@Component({
  selector: "app-browse-scope",
  imports: [RouterLink, MatButtonModule, SharedBanner],
  template: `
    <app-shared-banner [by]="shared()" what="these skills" />
    @if (space() !== undefined) {
      <div class="row">
        <span class="badge team">Showing {{ spaceName() }} only</span>
        <a mat-button routerLink="/browse">Show everything</a>
      </div>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
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
      <header class="row">
        <div class="grow">
          <h1>Browse skills</h1>
          <p class="lead">Everything here also appears in the Agent Plugins app on your PC, ready to install.</p>
        </div>
        <a mat-stroked-button routerLink="/bundles/new"><app-icon name="layers" />New bundle</a>
      </header>

      <app-browse-scope [shared]="shared()" [space]="space()" [spaceName]="spaceName()" />

      <div class="filters row">
        <mat-form-field appearance="outline" subscriptSizing="dynamic" class="search">
          <mat-label>Search</mat-label>
          <app-icon matPrefix name="search" class="prefix" />
          <input matInput type="search" placeholder="Meeting notes, code review, tone…" [value]="query()" (input)="setQuery($event)" />
        </mat-form-field>
        <mat-button-toggle-group aria-label="Who published it" [value]="lane()" (change)="setLane($event.value)">
          <mat-button-toggle value="all">All</mat-button-toggle>
          <mat-button-toggle value="official">Official</mat-button-toggle>
          <mat-button-toggle value="team">Teams</mat-button-toggle>
          <mat-button-toggle value="personal">People</mat-button-toggle>
        </mat-button-toggle-group>
        <mat-form-field appearance="outline" subscriptSizing="dynamic" class="teams">
          <mat-label>Teams</mat-label>
          <mat-select multiple placeholder="All teams" [disabled]="teams().length === 0" [value]="selectedTeams()" (selectionChange)="setTeams($event.value)">
            @for (team of teams(); track team.namespace) {
              <mat-option [value]="team.namespace">{{ team.displayName }} ({{ team.count }})</mat-option>
            }
          </mat-select>
        </mat-form-field>
      </div>

      @if (index.hasValue()) {
        <app-bundle-grid [bundles]="bundles()" />
        <p class="muted" aria-live="polite">{{ summary() }}</p>
        @if (results().length > 0) {
          <div class="grid">
            @for (item of results(); track item.id) {
              <app-package-card [item]="item" [age]="age(item)" />
            }
          </div>
        } @else {
          <div class="card empty">
            <h2>Nothing matches yet</h2>
            <p class="muted">Try a shorter search, or share the skill you were looking for.</p>
            @if (session.me()) {
              <a mat-flat-button routerLink="/publish">Share a skill</a>
            }
          </div>
        }
      } @else if (index.error(); as error) {
        <p class="problem">{{ message(error) }}</p>
      } @else {
        <mat-progress-bar mode="indeterminate" aria-label="Loading skills" />
      }
    </div>
  `,
  styles: `
    .filters {
      justify-content: space-between;
    }
    .search {
      flex: 1 1 20rem;
      max-width: 36rem;
    }
    .teams {
      flex: 0 1 16rem;
    }
    .prefix {
      margin: 0 0.25rem 0 0.75rem;
      color: var(--muted);
    }
    .grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
      gap: 1rem;
    }
    .empty {
      text-align: center;
    }
    .grow {
      flex: 1 1 24rem;
    }
  `
})
export class BrowsePage {
  /** One space's skills, when a link or a team page asked for them. */
  public readonly space = input<string>();
  /** Who shared the space, when the page was opened from a share link. */
  public readonly shared = input<string>();

  protected readonly session = inject(Session);
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

  protected age(item: IndexPackage): string {
    return formatAge(item.publishedAt);
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }
}
