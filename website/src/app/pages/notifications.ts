import { Component, computed, inject, resource } from "@angular/core";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { RouterLink } from "@angular/router";
import { Api, ApiError } from "../api";
import { formatAge } from "../format";
import { Session } from "../session";
import { runTask } from "../shared/tasks";

/** What happened to your skills, suggestions, reports, and teams, newest first; opening the page marks it read. */
@Component({
  selector: "app-notifications",
  imports: [RouterLink, MatProgressBarModule],
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
      const newest = items.reduce((top, item) => Math.max(top, item.id), 0);
      if (items.some((item) => !item.read)) {
        runTask(this.markRead(newest));
      }

      return items;
    }
  });

  protected readonly rows = computed(() => (this.items.hasValue() ? this.items.value().map((item) => ({ ...item, age: formatAge(item.createdAt) })) : []));

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }

  private async markRead(upTo: number): Promise<void> {
    await this.api.markNotificationsRead(upTo);
    await this.session.refreshMe();
  }
}
