import { Component, computed, inject, input, linkedSignal, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSelectModule } from "@angular/material/select";
import { MatSlideToggleModule } from "@angular/material/slide-toggle";
import { Router, RouterLink } from "@angular/router";
import type { IndexBundle, IndexPackage } from "../api";
import { Api, ApiError } from "../api";
import type { BrowseSort, BrowseState } from "../format";
import { browseParams, parseBrowse } from "../format";
import { Session } from "../session";
import { BundleGrid } from "../shared/bundle-card";
import { Icon } from "../shared/icon";
import { PackageCard } from "../shared/package-card";
import { SharedBanner } from "../shared/shared-banner";
import { runTask } from "../shared/tasks";

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

const pageSize = 60;

/** The matching skills, 60 at a time, or what to do when nothing matches. */
@Component({
  selector: "app-browse-results",
  imports: [RouterLink, MatButtonModule, PackageCard],
  template: `
    <h2 id="skills-heading">
      Skills <span class="count">{{ results().length }}</span>
    </h2>
    <p class="visually-hidden" aria-live="polite">{{ summary() }}</p>
    <div class="grid">
      @for (item of shown(); track item.id) {
        <app-package-card [item]="item" />
      }
    </div>
    <button mat-stroked-button type="button" class="more" [hidden]="shown().length >= results().length" (click)="limit.set(limit() + pageSize)">
      Show more ({{ results().length - shown().length }} left)
    </button>
    <div class="muted empty" [hidden]="results().length > 0">
      <p>No skills match.</p>
      <p class="row">
        <a mat-button routerLink="/browse" [hidden]="!filtered()">Clear the filters</a>
        <a mat-button routerLink="/publish">Share the one you were looking for</a>
      </p>
    </div>
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
      margin-top: 0.5rem;
    }
    h2 {
      margin: 0;
    }
    .count {
      margin-left: 0.25rem;
      color: var(--muted);
      font-weight: 400;
    }
    .more {
      justify-self: center;
    }
    .empty p {
      margin: 0;
    }
  `,
  host: { role: "region", "aria-labelledby": "skills-heading" }
})
export class BrowseResults {
  public readonly results = input.required<readonly IndexPackage[]>();
  public readonly filtered = input.required<boolean>();

  protected readonly pageSize = pageSize;
  /** Back to the first page whenever the results change. */
  protected readonly limit = linkedSignal({ source: () => this.results(), computation: () => pageSize });
  protected readonly shown = computed(() => this.results().slice(0, this.limit()));
  protected readonly summary = computed(() => {
    const count = this.results().length;
    return count === 1 ? "1 skill" : `${String(count)} skills`;
  });
}

@Component({
  selector: "app-browse",
  imports: [
    RouterLink,
    MatButtonModule,
    MatButtonToggleModule,
    MatFormFieldModule,
    MatInputModule,
    MatProgressBarModule,
    MatSelectModule,
    MatSlideToggleModule,
    Icon,
    BundleGrid,
    BrowseScope,
    BrowseResults
  ],
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
        <mat-button-toggle-group aria-label="Who published it" hideSingleSelectionIndicator [value]="state().lane" (change)="setLane($event.value)">
          <mat-button-toggle value="all">All</mat-button-toggle>
          <mat-button-toggle value="official">Official</mat-button-toggle>
          <mat-button-toggle value="team">Teams</mat-button-toggle>
          <mat-button-toggle value="personal">People</mat-button-toggle>
        </mat-button-toggle-group>
        @if (state().lane === "team" && teams().length > 1) {
          <mat-form-field appearance="outline" subscriptSizing="dynamic" class="teams">
            <mat-select multiple aria-label="Teams" placeholder="All teams" [value]="selectedTeams()" (selectionChange)="setTeams($event.value)">
              @for (team of teams(); track team.namespace) {
                <mat-option [value]="team.namespace">{{ team.displayName }} ({{ team.count }})</mat-option>
              }
            </mat-select>
          </mat-form-field>
        }
        <mat-form-field appearance="outline" subscriptSizing="dynamic" class="sort">
          <mat-select aria-label="Sort" [value]="state().sort" (selectionChange)="go({ sort: $event.value })">
            <mat-option value="popular">Most used</mat-option>
            <mat-option value="new">Newest</mat-option>
            <mat-option value="updated">Recently updated</mat-option>
          </mat-select>
        </mat-form-field>
        @if (installedIds() !== null) {
          <mat-slide-toggle [checked]="state().installed" (change)="go({ installed: $event.checked })">Installed on my PC</mat-slide-toggle>
        }
      </div>
      @if (state().tag; as tag) {
        <p class="notice row tag-filter">
          Tagged “{{ tag }}”.
          <button mat-button type="button" (click)="go({ tag: null })"><app-icon name="close" />Show every tag</button>
        </p>
      }

      @if (index.hasValue()) {
        <app-bundle-grid [bundles]="bundles()" />
        <app-browse-results [results]="results()" [filtered]="filtered()" />
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
    .teams,
    .sort {
      flex: 0 1 12rem;
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
    .tag-filter {
      margin: 0;
      gap: 0.5rem;
    }
  `
})
export class BrowsePage {
  /** One space's skills, when a link or a team page asked for them. */
  public readonly space = input<string>();
  /** Who shared the space, when the page was opened from a share link. */
  public readonly shared = input<string>();
  // The filters live in the query string, so Back and shared links keep them.
  public readonly q = input<string>();
  public readonly lane = input<string>();
  public readonly sort = input<string>();
  public readonly tag = input<string>();
  public readonly installed = input<string>();

  protected readonly state = computed(() => parseBrowse({ q: this.q(), lane: this.lane(), sort: this.sort(), tag: this.tag(), installed: this.installed() }));
  /** What's typed, ahead of the URL catching up. */
  protected readonly query = linkedSignal(() => this.state().q);
  /** Team namespaces to show; only set while the Teams lane is on, and empty shows every team. */
  protected readonly selectedTeams = signal<readonly string[]>([]);
  private readonly api = inject(Api);
  private readonly router = inject(Router);
  private readonly session = inject(Session);

  protected readonly index = resource({ loader: () => this.api.index() });
  /** What the caller's desktop app last reported installed; null when there is no app to ask. */
  protected readonly installedIds = computed(() => this.session.me()?.app?.installed ?? null);
  protected readonly filtered = computed(() => {
    const state = this.state();
    return this.query().trim() !== "" || state.lane !== "all" || state.tag !== null || state.installed || this.space() !== undefined;
  });

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
    const lane = this.state().lane;
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

  protected readonly bundles = computed<readonly IndexBundle[]>(() =>
    this.index.hasValue() && this.state().tag === null && !this.state().installed ? this.index.value().bundles.filter((bundle) => this.matches(bundle, [])) : []
  );

  protected readonly results = computed<readonly IndexPackage[]>(() => {
    if (!this.index.hasValue()) {
      return [];
    }

    const { tag, installed, sort } = this.state();
    const mine = this.installedIds() ?? [];
    return this.index
      .value()
      .packages.filter((item) => this.matches(item, item.tags) && (tag === null || item.tags.includes(tag)) && (!installed || mine.includes(item.id)))
      .toSorted(order[sort]);
  });

  protected setQuery(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.query.set(event.target.value);
      this.go({ q: event.target.value });
    }
  }

  protected setLane(value: unknown): void {
    if (value === "all" || value === "official" || value === "team" || value === "personal") {
      this.go({ lane: value });
      if (value !== "team") {
        this.selectedTeams.set([]);
      }
    }
  }

  protected setTeams(value: unknown): void {
    if (Array.isArray(value)) {
      this.selectedTeams.set(value.filter((team): team is string => typeof team === "string"));
      this.go({ lane: "team" });
    }
  }

  /** Changes the filters in the URL, replacing the history entry so Back leaves the page. */
  protected go(change: Partial<BrowseState>): void {
    // Whatever is read back from the URL goes through parseBrowse, so a bad value falls back to the default.
    runTask(this.router.navigate([], { queryParams: browseParams({ ...this.state(), q: this.query(), ...change }), queryParamsHandling: "merge", replaceUrl: true }));
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }
}

/** Ties go alphabetically. */
function by(key: (item: IndexPackage) => number): (left: IndexPackage, right: IndexPackage) => number {
  return (left, right) => {
    const difference = key(right) - key(left);
    return difference !== 0 ? difference : left.name.localeCompare(right.name);
  };
}

const order: Readonly<Record<BrowseSort, (left: IndexPackage, right: IndexPackage) => number>> = {
  popular: by((item) => item.installedBase),
  new: by((item) => Date.parse(item.createdAt)),
  updated: by((item) => Date.parse(item.publishedAt))
};
