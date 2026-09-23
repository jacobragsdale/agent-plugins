import { Component, computed, inject, resource, signal } from "@angular/core";
import { MatDialog } from "@angular/material/dialog";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { MatTabsModule } from "@angular/material/tabs";
import type { PendingReview, Report } from "../api";
import { Api, ApiError } from "../api";
import { Session } from "../session";
import { prompt } from "../shared/dialogs";
import { Icon } from "../shared/icon";
import { runTask } from "../shared/tasks";
import { ReportList, ReviewCard, UsageSummary } from "./admin-parts";

@Component({ selector: "app-admin", imports: [MatProgressBarModule, MatTabsModule, Icon, ReviewCard, ReportList, UsageSummary], templateUrl: "./admin.html", styleUrl: "./admin.scss" })
export class AdminPage {
  private readonly api = inject(Api);
  private readonly session = inject(Session);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected readonly reviews = resource({ loader: () => this.api.reviews() });
  protected readonly reports = resource({ loader: () => this.api.reports() });
  protected readonly summary = resource({ loader: () => this.api.summary() });
  protected readonly busy = signal(false);

  protected readonly reviewsLabel = computed(() => `Waiting for review (${String(this.reviews.hasValue() ? this.reviews.value().length : 0)})`);
  protected readonly reportsLabel = computed(() => `Reports (${String(this.reports.hasValue() ? this.reports.value().filter((report) => report.resolvedAt === null).length : 0)})`);

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }

  protected approve(review: PendingReview): void {
    runTask(this.decide(review, "approve", ""));
  }

  protected reject(review: PendingReview): void {
    runTask(this.askForChanges(review));
  }

  protected resolve(report: Report): void {
    runTask(this.markResolved(report));
  }

  private async askForChanges(review: PendingReview): Promise<void> {
    const note = await prompt(this.dialog, {
      title: `Ask for changes to ${review.name}`,
      message: "The publisher sees your note on My skills and can publish a fixed version.",
      confirm: "Send back",
      field: { label: "What needs to change?", hint: "Be specific and kind.", required: true },
      danger: true
    });
    if (note !== undefined) {
      await this.decide(review, "reject", note);
    }
  }

  private async markResolved(report: Report): Promise<void> {
    try {
      await this.api.resolveReport(report.id);
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    } finally {
      this.reports.reload();
      this.summary.reload();
    }
  }

  private async decide(review: PendingReview, decision: "approve" | "reject", note: string): Promise<void> {
    this.busy.set(true);
    try {
      await this.api.review(review.namespace, review.packageId, review.version, decision, note);
      this.snackBar.open(decision === "approve" ? `${review.name} ${review.version} is live.` : `${review.name} ${review.version} was sent back.`, undefined, { duration: 4000 });
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    } finally {
      this.busy.set(false);
      this.reviews.reload();
      await this.session.refreshReviews();
    }
  }
}
