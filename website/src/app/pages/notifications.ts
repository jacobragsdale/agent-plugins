import { Component, computed, inject, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { RouterLink } from "@angular/router";
import type { Notification } from "../api";
import { Api, ApiError } from "../api";
import { formatAge } from "../format";
import { Session } from "../session";
import { runTask } from "../shared/tasks";

/** A full page means there may be older ones; the server answers 50 at a time. */
const PAGE = 50;

/** What happened to your skills, suggestions, reports, and teams, newest first; what the page shows is marked read. */
@Component({
  selector: "app-notifications",
  imports: [RouterLink, MatButtonModule, MatProgressBarModule],
  template: `
    <div class="page narrow stack">
      <h1>Notifications</h1>
      @if (items.hasValue()) {
        <ul class="list-box">
          @for (item of rows(); track item.id) {
            <li [class.unread]="!item.read">
              @if (item.link; as link) {
                <a [routerLink]="link">{{ item.text }}</a>
              } @else {
                <span>{{ item.text }}</span>
              }
              <span class="muted when">{{ item.age }}</span>
            </li>
          } @empty {
            <li class="muted">Nothing yet. Suggestions, answers to your reports, and news about your skills show up here.</li>
          }
        </ul>
        @if (olderError(); as error) {
          <p class="problem">{{ error }}</p>
        }
        @if (more()) {
          <button mat-stroked-button type="button" [disabled]="loadingOlder()" (click)="showOlder()">Show older</button>
        }
      } @else if (items.error(); as error) {
        <p class="problem">{{ message(error) }}</p>
      } @else {
        <mat-progress-bar mode="indeterminate" aria-label="Loading notifications" />
      }
    </div>
  `,
  styles: `
    li {
      display: flex;
      flex-wrap: wrap;
      justify-content: space-between;
      gap: 0.25rem 1rem;
    }
    .unread {
      font-weight: 600;
    }
    .when {
      font: var(--mat-sys-body-small);
    }
  `
})
export class NotificationsPage {
  private readonly api = inject(Api);
  private readonly session = inject(Session);

  protected readonly items = resource({
    loader: async () => {
      const items = await this.api.notifications();
      this.older.set([]);
      this.more.set(items.length >= PAGE);
      this.markShown(items);
      return items;
    }
  });

  private readonly older = signal<readonly Notification[]>([]);
  protected readonly more = signal(false);
  protected readonly loadingOlder = signal(false);
  protected readonly olderError = signal<string | undefined>(undefined);

  protected readonly rows = computed(() => (this.items.hasValue() ? [...this.items.value(), ...this.older()] : []).map((item) => ({ ...item, age: formatAge(item.createdAt) })));

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }

  protected showOlder(): void {
    runTask(this.loadOlder());
  }

  private async loadOlder(): Promise<void> {
    const shown = this.rows();
    const oldest = shown.at(-1)?.id;
    if (oldest === undefined) {
      return;
    }

    this.loadingOlder.set(true);
    this.olderError.set(undefined);
    try {
      const page = await this.api.notifications(oldest);
      this.older.update((items) => [...items, ...page]);
      this.more.set(page.length >= PAGE);
      this.markShown(page);
    } catch (error) {
      this.olderError.set(ApiError.from(error).message);
    } finally {
      this.loadingOlder.set(false);
    }
  }

  /** Marks read only the ids this page showed, so older unseen ones stay unread. */
  private markShown(page: readonly Notification[]): void {
    if (!page.some((item) => !item.read)) {
      return;
    }

    const ids = page.map((item) => item.id);
    runTask(this.markRead(Math.min(...ids), Math.max(...ids)));
  }

  private async markRead(from: number, upTo: number): Promise<void> {
    await this.api.markNotificationsRead(from, upTo);
    await this.session.refreshMe();
  }
}
