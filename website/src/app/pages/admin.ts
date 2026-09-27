import { Component, computed, inject, resource, signal } from "@angular/core";
import { MatDialog } from "@angular/material/dialog";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { MatTabsModule } from "@angular/material/tabs";
import type { PublicReview, Report } from "../api";
import { Api, ApiError } from "../api";
import { Session } from "../session";
import { prompt } from "../shared/dialogs";
import { Icon } from "../shared/icon";
import { runTask } from "../shared/tasks";
import { AuditLog, InstallsLookup, PeopleAdmin, ReportList, ReviewCard, UsageSummary } from "./admin-parts";

@Component({
  selector: "app-admin",
  imports: [MatProgressBarModule, MatTabsModule, Icon, ReviewCard, ReportList, UsageSummary, InstallsLookup, AuditLog, PeopleAdmin],
  templateUrl: "./admin.html",
  styleUrl: "./admin.scss"
})
export class AdminPage {
  private readonly api = inject(Api);
  private readonly session = inject(Session);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected readonly reviews = resource({ loader: () => this.api.reviews() });
  protected readonly reports = resource({ loader: () => this.api.reports() });
  protected readonly summary = resource({ loader: () => this.api.summary() });
  protected readonly busy = signal(false);

  protected readonly reviewsLabel = computed(() => `MCP servers for everyone (${String(this.reviews.hasValue() ? this.reviews.value().length : 0)})`);
  protected readonly reportsLabel = computed(() => `Reports (${String(this.reports.hasValue() ? this.reports.value().filter((report) => report.resolvedAt === null).length : 0)})`);

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }

  protected approve(review: PublicReview): void {
    runTask(this.decide(review, "approve", ""));
  }

  protected decline(review: PublicReview): void {
    runTask(this.askWhy(review));
  }

  protected resolve(report: Report): void {
    runTask(this.markResolved(report));
  }

  private async askWhy(review: PublicReview): Promise<void> {
    const note = await prompt(this.dialog, {
      title: `Keep ${review.name} from everyone?`,
      message: "The people who can use it now keep it. The publisher sees your note, and their next version asks again.",
      confirm: "Decline",
      field: { label: "What needs to change?", hint: "Be specific and kind.", required: true },
      danger: true
    });
    if (note !== undefined) {
      await this.decide(review, "decline", note);
    }
  }

  private async markResolved(report: Report): Promise<void> {
    const note = await prompt(this.dialog, {
      title: "Mark this resolved?",
      message: `${report.account} sees that it's resolved, with your note if you write one.`,
      confirm: "Mark resolved",
      field: { label: "Note (optional)", hint: "What was done about it.", required: false }
    });
    if (note === undefined) {
      return;
    }

    try {
      await this.api.resolveReport(report.id, note);
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    } finally {
      this.reports.reload();
      this.summary.reload();
      await this.session.refreshReviews();
    }
  }

  private async decide(review: PublicReview, decision: "approve" | "decline", note: string): Promise<void> {
    this.busy.set(true);
    try {
      await this.api.review(review.namespace, review.packageId, decision, note);
      this.snackBar.open(decision === "approve" ? `Everyone can see ${review.name} now.` : `${review.name} stays with the people who have it.`, undefined, { duration: 4000 });
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    } finally {
      this.busy.set(false);
      this.reviews.reload();
      await this.session.refreshReviews();
    }
  }
}
