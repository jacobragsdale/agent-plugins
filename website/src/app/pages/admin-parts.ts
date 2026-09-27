import { Component, computed, inject, input, output, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatSnackBar } from "@angular/material/snack-bar";
import { RouterLink } from "@angular/router";
import type { AuditEntry, PublicReview, Report, Summary } from "../api";
import { Api, ApiError, archiveUrl, auditCsvUrl, installsUrl, versionFiles } from "../api";
import { formatAge } from "../format";
import { FileViewer } from "../shared/file-viewer";
import { Icon } from "../shared/icon";
import { prompt } from "../shared/dialogs";
import { McpServerFacts } from "../shared/mcp-server-facts";
import { describeKinds } from "../shared/package-card";
import { runTask } from "../shared/tasks";

/** A package with an MCP server that everyone could see, waiting for an admin, with its files a click away. */
@Component({
  selector: "app-review-card",
  imports: [RouterLink, MatButtonModule, FileViewer, Icon, McpServerFacts],
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
    <p class="muted">
      Who can see it now: {{ review().audience }} · {{ review().installedBase === 1 ? "1 person uses it" : review().installedBase + " people use it" }}. Approving lets everyone see it. A later version
      that runs something else comes back here.
    </p>
    @for (server of review().mcpServers; track server.name) {
      <app-mcp-server-facts [server]="server" />
    }
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
          <span [class]="row.report.kind === 'problem' ? 'badge rejected' : 'badge'">{{ row.report.kind === "problem" ? "Problem" : "Feedback" }}</span>
          <span class="muted">from {{ row.report.account }} · {{ row.age }}</span>
          <span class="spacer"></span>
          @if (row.report.resolvedBy; as resolvedBy) {
            <span class="badge live">Resolved by {{ resolvedBy }}</span>
          } @else {
            <button mat-stroked-button type="button" (click)="resolve.emit(row.report)"><app-icon name="check" />Mark resolved</button>
          }
        </div>
        <p class="reason">{{ row.report.reason }}</p>
        @if (row.report.note; as note) {
          <p class="muted"><strong>Answer:</strong> {{ note }}</p>
        }
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
    .reason {
      white-space: pre-line;
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

/** Who has a package installed, per PC, as a table and as a CSV download. */
@Component({
  selector: "app-installs-lookup",
  imports: [MatButtonModule, MatFormFieldModule, MatInputModule, Icon],
  template: `
    <form class="row" (submit)="look($event)">
      <mat-form-field appearance="outline" subscriptSizing="dynamic">
        <mat-label>Package ID</mat-label>
        <input matInput required placeholder="data-team/sql-helper" [value]="id()" (input)="setId($event)" />
      </mat-form-field>
      <button mat-flat-button type="submit" [disabled]="target() === null">Look up</button>
      @if (csv(); as href) {
        <a mat-button [href]="href" download><app-icon name="download" />Download CSV</a>
      }
    </form>
    @if (rows.hasValue()) {
      <table>
        <thead>
          <tr>
            <th>Person</th>
            <th>PC</th>
            <th>Version</th>
            <th>App</th>
            <th>Last seen</th>
          </tr>
        </thead>
        <tbody>
          @for (row of rows.value(); track $index) {
            <tr>
              <td>{{ row.account }}</td>
              <td>{{ row.device ?? "—" }}</td>
              <td>{{ row.version ?? "—" }}</td>
              <td>{{ row.clientVersion ?? "—" }}</td>
              <td>{{ age(row.lastSeenAt) }}</td>
            </tr>
          } @empty {
            <tr>
              <td colspan="5" class="muted">Nobody's PC reports it installed.</td>
            </tr>
          }
        </tbody>
      </table>
    } @else if (rows.error(); as error) {
      <p class="problem">{{ message(error) }}</p>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
    }
    mat-form-field {
      flex: 0 1 24rem;
    }
    table {
      border-collapse: collapse;
      width: 100%;
    }
    th,
    td {
      text-align: left;
      padding: 0.4rem 0.6rem;
      border-bottom: var(--border);
    }
  `
})
export class InstallsLookup {
  protected readonly id = signal("");
  private readonly looked = signal<readonly [string, string] | null>(null);
  private readonly api = inject(Api);

  protected readonly target = computed(() => {
    const [ns, pkg, ...rest] = this.id().trim().split("/");
    return ns !== undefined && ns.length > 0 && pkg !== undefined && pkg.length > 0 && rest.length === 0 ? ([ns, pkg] as const) : null;
  });

  protected readonly rows = resource({ params: () => this.looked() ?? undefined, loader: ({ params }) => this.api.installs(params[0], params[1]) });
  protected readonly csv = computed(() => {
    const looked = this.looked();
    return looked === null ? null : `${installsUrl(looked[0], looked[1])}?format=csv`;
  });

  protected setId(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.id.set(event.target.value);
    }
  }

  protected look(event: SubmitEvent): void {
    event.preventDefault();
    this.looked.set(this.target());
  }

  protected age(at: string): string {
    return formatAge(at);
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }
}

/** Every change anyone made, newest first, a hundred at a time. */
@Component({
  selector: "app-audit-log",
  imports: [MatButtonModule, Icon],
  template: `
    <div class="row">
      <p class="muted grow">Publishing, sharing, links, teams, bundles, reviews, removals, and blocks, with who did it.</p>
      <a mat-button [href]="csv" download><app-icon name="download" />Download CSV</a>
    </div>
    <table>
      <thead>
        <tr>
          <th>When</th>
          <th>Who</th>
          <th>What</th>
          <th>On</th>
          <th>Details</th>
        </tr>
      </thead>
      <tbody>
        @for (entry of entries(); track entry.id) {
          <tr>
            <td [title]="entry.at">{{ age(entry.at) }}</td>
            <td>{{ entry.actor }}</td>
            <td>{{ entry.action }}</td>
            <td>
              <code>{{ entry.target }}</code>
            </td>
            <td>{{ entry.detail ?? "" }}</td>
          </tr>
        }
      </tbody>
    </table>
    @if (problem(); as failure) {
      <p class="problem">{{ failure }}</p>
    }
    @if (more()) {
      <button mat-stroked-button type="button" class="older" [disabled]="loading()" (click)="load()">Older</button>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
    }
    .grow {
      flex: 1;
      margin: 0;
    }
    table {
      border-collapse: collapse;
      width: 100%;
    }
    th,
    td {
      text-align: left;
      padding: 0.4rem 0.6rem;
      border-bottom: var(--border);
      vertical-align: top;
    }
    .older {
      justify-self: center;
    }
  `
})
export class AuditLog {
  protected readonly csv = auditCsvUrl;
  protected readonly entries = signal<readonly AuditEntry[]>([]);
  protected readonly more = signal(true);
  protected readonly loading = signal(false);
  protected readonly problem = signal<string | null>(null);
  private readonly api = inject(Api);

  constructor() {
    this.load();
  }

  protected load(): void {
    runTask(this.fetch());
  }

  protected age(at: string): string {
    return formatAge(at);
  }

  private async fetch(): Promise<void> {
    this.loading.set(true);
    try {
      const page = await this.api.audit(this.entries().at(-1)?.id);
      this.entries.update((entries) => [...entries, ...page]);
      this.more.set(page.length === 100);
      this.problem.set(null);
    } catch (error) {
      this.problem.set(ApiError.from(error).message);
    } finally {
      this.loading.set(false);
    }
  }
}

/** Blocked accounts, and handing a personal space to someone else. */
@Component({
  selector: "app-people-admin",
  imports: [MatButtonModule, MatFormFieldModule, MatInputModule, Icon],
  template: `
    <section class="stack">
      <h2>Blocked accounts</h2>
      <p class="muted">A blocked account can't publish, suggest, share, create teams, or join through links, and its space is hidden from everyone but admins.</p>
      <form class="row" (submit)="block($event)">
        <mat-form-field appearance="outline" subscriptSizing="dynamic">
          <mat-label>Account</mat-label>
          <input matInput required placeholder="CORP\\jane" [value]="account()" (input)="account.set(value($event))" />
        </mat-form-field>
        <button mat-stroked-button type="submit" class="danger" [disabled]="account().trim() === ''">Block</button>
      </form>
      @if (blocks.hasValue()) {
        <ul class="list-box">
          @for (entry of blocks.value(); track entry.account) {
            <li class="row">
              <strong>{{ entry.account }}</strong>
              <span class="muted">by {{ entry.blockedBy }} · {{ age(entry.blockedAt) }}{{ entry.reason === null ? "" : " · " + entry.reason }}</span>
              <span class="spacer"></span>
              <button mat-button type="button" (click)="unblock(entry.account)">Unblock</button>
            </li>
          } @empty {
            <li class="muted">Nobody is blocked.</li>
          }
        </ul>
      } @else if (blocks.error(); as error) {
        <p class="problem">{{ message(error) }}</p>
      }
    </section>

    <section class="stack">
      <h2>Hand over a space</h2>
      <p class="muted">
        When someone leaves, give their personal space and its skills to a colleague. The space becomes a team they own, installed skills keep their names, and the previous owner gets a new personal
        space.
      </p>
      <form class="row" (submit)="transfer($event)">
        <mat-form-field appearance="outline" subscriptSizing="dynamic">
          <mat-label>Space</mat-label>
          <input matInput required placeholder="jane" [value]="space()" (input)="space.set(value($event))" />
        </mat-form-field>
        <mat-form-field appearance="outline" subscriptSizing="dynamic">
          <mat-label>New owner</mat-label>
          <input matInput required placeholder="CORP\\sam" [value]="owner()" (input)="owner.set(value($event))" />
        </mat-form-field>
        <button mat-stroked-button type="submit" [disabled]="space().trim() === '' || owner().trim() === ''"><app-icon name="person" />Hand over</button>
      </form>
    </section>
  `,
  styles: `
    :host {
      display: grid;
      gap: 2rem;
    }
    h2,
    p {
      margin: 0;
    }
  `
})
export class PeopleAdmin {
  protected readonly account = signal("");
  protected readonly space = signal("");
  protected readonly owner = signal("");
  private readonly api = inject(Api);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected readonly blocks = resource({ loader: () => this.api.blocks() });

  protected value(event: Event): string {
    return event.target instanceof HTMLInputElement ? event.target.value : "";
  }

  protected age(at: string): string {
    return formatAge(at);
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }

  protected block(event: SubmitEvent): void {
    event.preventDefault();
    runTask(this.confirmBlock(this.account().trim()));
  }

  protected unblock(account: string): void {
    runTask(this.setBlocked(account, null));
  }

  protected transfer(event: SubmitEvent): void {
    event.preventDefault();
    runTask(this.handOver(this.space().trim(), this.owner().trim()));
  }

  private async confirmBlock(account: string): Promise<void> {
    const reason = await prompt(this.dialog, {
      title: `Block ${account}?`,
      message: "They keep what's installed on their PC, but can't publish or share, and their space is hidden.",
      confirm: "Block",
      field: { label: "Why? (optional)", hint: "Other admins see this.", required: false },
      danger: true
    });
    if (reason !== undefined) {
      await this.setBlocked(account, reason);
    }
  }

  /** `reason: null` unblocks. */
  private async setBlocked(account: string, reason: string | null): Promise<void> {
    try {
      await this.api.setBlocked(account, reason);
      this.snackBar.open(reason === null ? `${account} is unblocked.` : `${account} is blocked.`, undefined, { duration: 4000 });
      this.account.set("");
      this.blocks.reload();
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }

  private async handOver(ns: string, account: string): Promise<void> {
    const confirmed = await prompt(this.dialog, { title: `Give ${ns} to ${account}?`, message: `${account} owns the space and everything in it from now on.`, confirm: "Hand over" });
    if (confirmed === undefined) {
      return;
    }

    try {
      await this.api.transferSpace(ns, account);
      this.snackBar.open(`${ns} now belongs to ${account}.`, undefined, { duration: 4000 });
      this.space.set("");
      this.owner.set("");
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }
}
