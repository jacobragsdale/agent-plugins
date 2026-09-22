import { Component, computed, inject, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { RouterLink } from "@angular/router";
import type { IndexPackage, Lane } from "../api";
import { Api, ApiError } from "../api";
import { formatAge } from "../format";
import { Session } from "../session";
import { Icon } from "../shared/icon";
import { PackageCard } from "../shared/package-card";

type LaneFilter = Lane | "all";

@Component({
  selector: "app-browse",
  imports: [RouterLink, MatButtonModule, MatButtonToggleModule, MatFormFieldModule, MatInputModule, MatProgressBarModule, Icon, PackageCard],
  template: `
    <div class="page stack">
      <header class="row">
        <div>
          <h1>Browse skills</h1>
          <p class="lead">Everything here also appears in the Agent Plugins app on your PC, ready to install.</p>
        </div>
      </header>

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
      </div>

      @if (index.hasValue()) {
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
  `
})
export class BrowsePage {
  protected readonly session = inject(Session);
  protected readonly query = signal("");
  protected readonly lane = signal<LaneFilter>("all");
  private readonly api = inject(Api);

  protected readonly index = resource({ loader: () => this.api.index() });

  protected readonly results = computed<readonly IndexPackage[]>(() => {
    if (!this.index.hasValue()) {
      return [];
    }

    const words = this.query()
      .toLowerCase()
      .split(/\s+/u)
      .filter((word) => word.length > 0);
    const lane = this.lane();
    return this.index
      .value()
      .filter((item) => lane === "all" || item.lane === lane)
      .filter((item) => {
        const haystack = [item.name, item.description, item.id, item.publisher.displayName, ...item.tags].join(" ").toLowerCase();
        return words.every((word) => haystack.includes(word));
      })
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
    }
  }

  protected age(item: IndexPackage): string {
    return formatAge(item.publishedAt);
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }
}
