import { Component, computed, input, output, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { RouterLink } from "@angular/router";
import type { PublicReview, Report, Summary } from "../api";
import { archiveUrl, versionFiles } from "../api";
import { formatAge } from "../format";
import { FileViewer } from "../shared/file-viewer";
import { Icon } from "../shared/icon";
import { describeKinds } from "../shared/package-card";

/** A package with an MCP server that everyone could see, waiting for an admin, with its files a click away. */
@Component({
  selector: "app-review-card",
  imports: [RouterLink, MatButtonModule, FileViewer, Icon],
  template: `
    <div class="row">
      <div class="grow">
        <h2>
          <a [routerLink]="['/p', review().namespace, review().packageId]">{{ review().name }}</a> <span class="muted">{{ review().version }}</span>
        </h2>
        <p class="muted">
          <code>{{ review().id }}</code> · {{ kinds() }} · by {{ review().publishedBy }} · {{ age() }}
        </p>
      </div>
      <span class="badge rejected" title="Runs a program on people's PCs"><app-icon name="shield" />MCP server</span>
    </div>
    <p class="muted">Its owners, their team, and people they shared it with can already use it. Approving lets everyone see it, including later versions.</p>
    @if (review().changelog; as changelog) {
      <p class="note"><strong>Publisher's note:</strong> {{ changelog }}</p>
    }
    <div class="row">
      <button mat-stroked-button type="button" [attr.aria-expanded]="open()" (click)="open.set(!open())"><app-icon name="description" />{{ open() ? "Hide files" : "Read the files" }}</button>
      <a mat-button [href]="archive()" download><app-icon name="download" />Download zip</a>
      <span class="spacer"></span>
      <button mat-button type="button" [disabled]="busy()" (click)="decline.emit(review())"><app-icon name="close" />Decline</button>
      <button mat-flat-button type="button" [disabled]="busy()" (click)="approve.emit(review())"><app-icon name="check" />Approve for everyone</button>
    </div>
    @if (open()) {
      <app-file-viewer [source]="files()" />
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
    }
    h2 {
      font: var(--mat-sys-title-large);
      margin: 0;
    }
    .grow {
      flex: 1 1 20rem;
      p {
        margin: 0.25rem 0 0;
      }
    }
    p {
      margin: 0;
    }
  `,
  host: { class: "card" }
})
export class ReviewCard {
  public readonly review = input.required<PublicReview>();
  public readonly busy = input.required<boolean>();
  public readonly approve = output<PublicReview>();
  public readonly decline = output<PublicReview>();

  protected readonly open = signal(false);
  protected readonly kinds = computed(() => describeKinds(this.review().componentKinds));
  protected readonly age = computed(() => formatAge(this.review().publishedAt));
  protected readonly archive = computed(() => archiveUrl(this.review().namespace, this.review().packageId, this.review().version));
  protected readonly files = computed(() => versionFiles(this.review().namespace, this.review().packageId, this.review().version));
}

interface ReportRow {
  readonly report: Report;
  readonly link: readonly string[];
  readonly age: string;
}

@Component({
  selector: "app-report-list",
  imports: [RouterLink, MatButtonModule, Icon],
  template: `
    @for (row of rows(); track row.report.id) {
      <article class="card" [class.resolved]="row.report.resolvedAt !== null">
        <div class="row">
          <a [routerLink]="row.link"
            ><code>{{ row.report.packageId }}</code></a
          >
          <span class="muted">reported by {{ row.report.account }} · {{ row.age }}</span>
          <span class="spacer"></span>
          @if (row.report.resolvedBy; as resolvedBy) {
            <span class="badge live">Resolved by {{ resolvedBy }}</span>
          } @else {
            <button mat-stroked-button type="button" (click)="resolve.emit(row.report)"><app-icon name="check" />Mark resolved</button>
          }
        </div>
        <p>{{ row.report.reason }}</p>
      </article>
    } @empty {
      <p class="muted">No reports.</p>
    }
    <p class="muted">To take something down, open it and choose Remove from every PC.</p>
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
    }
    article p {
      margin: 0.75rem 0 0;
    }
    .resolved {
      opacity: 0.7;
    }
  `
})
export class ReportList {
  public readonly reports = input.required<readonly Report[]>();
  public readonly resolve = output<Report>();

  protected readonly rows = computed<readonly ReportRow[]>(() => this.reports().map((report) => ({ report, link: ["/p", ...report.packageId.split("/")], age: formatAge(report.createdAt) })));
}

interface Bar {
  readonly label: string;
  readonly count: number;
  readonly share: number;
}

/** A single series as labelled bars, largest first, widths relative to the largest (one hue, no legend). */
function bars(rows: readonly (readonly [string, number])[]): readonly Bar[] {
  const sorted = rows.toSorted(([, left], [, right]) => right - left).slice(0, 10);
  const top = Math.max(1, ...sorted.map(([, count]) => count));
  return sorted.map(([label, count]) => ({ label, count, share: Math.max(2, Math.round((count / top) * 100)) }));
}

@Component({ selector: "app-usage-summary", templateUrl: "./usage-summary.html", styleUrl: "./usage-summary.scss" })
export class UsageSummary {
  public readonly summary = input.required<Summary>();

  protected readonly tiles = computed(() => {
    const usage = this.summary();
    return [
      { label: "people active today", value: usage.activeUsers.day },
      { label: "this week", value: usage.activeUsers.week },
      { label: "this month", value: usage.activeUsers.month },
      { label: "live packages", value: usage.packages },
      { label: "publishers", value: usage.publishers }
    ];
  });

  protected readonly charts = computed(() => {
    const usage = this.summary();
    return [
      { title: "Most used (people, 30 days)", rows: bars(usage.topPackages.map((row) => [row.id, row.installedBase] as const)) },
      { title: "AI assistants", rows: bars(Object.entries(usage.agentMix)) },
      { title: "App versions", rows: bars(Object.entries(usage.clientVersions)) },
      { title: "Failed setup checks (7 days)", rows: bars(Object.entries(usage.preflightFailures)) }
    ];
  });
}
