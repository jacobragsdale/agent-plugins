import { Component, computed, inject, input, model, output, resource } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatSelectModule } from "@angular/material/select";
import { MatSnackBar } from "@angular/material/snack-bar";
import { Router, RouterLink } from "@angular/router";
import type { PackageDetail, PackageStats, PackageVersion, Report, ReportKind } from "../api";
import { Api, ApiError, archiveUrl, versionFiles } from "../api";
import { formatAge, formatBytes, formatDate, worksIn } from "../format";
import { Session } from "../session";
import { AccessBadge } from "../shared/access-badge";
import { prompt } from "../shared/dialogs";
import { FileViewer } from "../shared/file-viewer";
import { Icon } from "../shared/icon";
import { InstallButton } from "../shared/install-button";
import { LaneBadge } from "../shared/lane-badge";
import { share } from "../shared/share-dialog";
import type { PackageStatus } from "../shared/status";
import { copyText, runTask } from "../shared/tasks";

@Component({
  selector: "app-package-header",
  imports: [RouterLink, LaneBadge, AccessBadge],
  template: `
    <h1>{{ item().name }}</h1>
    <div class="meta">
      <a class="publisher" [routerLink]="['/browse']" [queryParams]="{ space: item().namespace }" [title]="'More from ' + item().publisher.displayName">
        <app-lane-badge [lane]="item().lane" [publisher]="item().publisher.displayName" />
      </a>
      <app-access-badge [restricted]="item().effective === 'private'" [sharedWithYou]="item().sharedWithYou" [showPublic]="item().owned" />
      @if (kinds() !== "Skill") {
        <span class="badge">{{ kinds() }}</span>
      }
      @for (tag of item().tags; track tag) {
        <a class="tag" [routerLink]="['/browse']" [queryParams]="{ tag }">{{ tag }}</a>
      }
    </div>
    <p class="lead">{{ item().description }}</p>
  `,
  styles: `
    :host {
      display: grid;
      gap: 0.5rem;
    }
    .lead {
      margin: 0.25rem 0 0;
    }
    .tag {
      border: var(--border);
      border-radius: 4px;
      padding: 0 0.4rem;
      font: var(--mat-sys-label-medium);
      color: inherit;
      text-decoration: none;
      &:hover {
        border-color: var(--mat-sys-outline);
      }
    }
    .publisher {
      color: inherit;
      text-decoration: none;
      &:hover {
        text-decoration: underline;
      }
    }
  `
})
export class PackageHeader {
  public readonly item = input.required<PackageDetail>();
  public readonly kinds = input.required<string>();
}

/** What the owner sees: the package's status and what they can do next. */
@Component({
  selector: "app-owner-panel",
  imports: [RouterLink, MatButtonModule, Icon],
  template: `
    <div class="row">
      <span [class]="status().badge">{{ status().label }}</span>
      <p class="detail">{{ status().detail }}</p>
    </div>
    <div class="row">
      @if (!revoked()) {
        @if (editable()) {
          <a mat-flat-button [routerLink]="['/p', ns(), pkg(), 'upload']" [queryParams]="{ edit: 'true' }"><app-icon name="edit" />Edit</a>
        }
        <a [matButton]="editable() ? 'outlined' : 'filled'" [routerLink]="['/p', ns(), pkg(), 'upload']"><app-icon name="upload" />Upload a new version</a>
      }
      <button mat-stroked-button type="button" (click)="share()"><app-icon name="share" />Share</button>
      <span class="spacer"></span>
      @if (revoked()) {
        @if (!revokedByAdmin() || admin()) {
          <button mat-button type="button" (click)="setRevoked(false)">Restore</button>
        }
      } @else {
        <button mat-button type="button" class="danger" (click)="setRevoked(true)"><app-icon name="delete" />Remove from every PC</button>
      }
      <button mat-button type="button" class="danger" (click)="remove()">Delete</button>
    </div>
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
    }
    .detail {
      margin: 0;
      flex: 1 1 20rem;
    }
  `,
  host: { class: "card", role: "region", "aria-label": "Your package" }
})
export class OwnerPanel {
  public readonly ns = input.required<string>();
  public readonly pkg = input.required<string>();
  public readonly name = input.required<string>();
  public readonly spaceName = input.required<string>();
  public readonly owners = input.required<string>();
  public readonly revoked = input.required<boolean>();
  /** An admin pulled it, so only an admin may put it back. */
  public readonly revokedByAdmin = input.required<boolean>();
  public readonly admin = input.required<boolean>();
  /** One SKILL.md and nothing else, so it can be edited here. */
  public readonly editable = input.required<boolean>();
  public readonly status = input.required<PackageStatus>();
  public readonly changed = output();

  private readonly api = inject(Api);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);
  private readonly router = inject(Router);

  protected remove(): void {
    runTask(this.confirmDelete());
  }

  private async confirmDelete(): Promise<void> {
    const confirmed = await prompt(this.dialog, {
      title: `Delete ${this.name()}?`,
      message: this.admin()
        ? "Every version and its files are deleted, and the name can be used again. PCs that have it lose it at their next check."
        : "Every version is deleted and the name can be used again. This only works while nobody has installed it and no problem report is open; otherwise, remove it from every PC instead.",
      confirm: "Delete",
      danger: true
    });
    if (confirmed === undefined) {
      return;
    }

    try {
      await this.api.deletePackage(this.ns(), this.pkg());
      this.snackBar.open(`${this.name()} is deleted.`, undefined, { duration: 5000 });
      await this.router.navigate(["/mine"]);
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }

  protected share(): void {
    runTask(this.openShare());
  }

  protected setRevoked(revoked: boolean): void {
    runTask(this.confirmRevoked(revoked));
  }

  private async openShare(): Promise<void> {
    if (await share(this.dialog, { namespace: this.ns(), id: this.pkg(), label: this.name(), spaceName: this.spaceName(), owners: this.owners() })) {
      this.snackBar.open("Sharing saved.", undefined, { duration: 4000 });
      this.changed.emit();
    }
  }

  private async confirmRevoked(revoked: boolean): Promise<void> {
    const confirmed = await prompt(
      this.dialog,
      revoked
        ? {
            title: `Remove ${this.name()} from every PC?`,
            message: "Nobody can install it anymore, and Agent Plugins removes it from every PC that has it at its next check. You can restore it later, but it won't come back on its own.",
            confirm: "Remove from every PC",
            danger: true
          }
        : { title: `Restore ${this.name()}?`, message: "People can find and install it again. PCs it was removed from don't get it back until someone installs it.", confirm: "Restore" }
    );
    if (confirmed === undefined) {
      return;
    }

    try {
      await this.api.setRevoked(this.ns(), this.pkg(), revoked);
      this.snackBar.open(revoked ? `${this.name()} is being removed from every PC.` : `${this.name()} is offered again.`, undefined, { duration: 5000 });
      this.changed.emit();
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }
}

/** The files of the version being shown; owners can switch to older or withdrawn versions. */
@Component({
  selector: "app-version-files",
  imports: [MatFormFieldModule, MatSelectModule, FileViewer],
  template: `
    @if (shown(); as version) {
      @if (options().length > 1) {
        <mat-form-field appearance="outline" subscriptSizing="dynamic" class="pick">
          <mat-label>Version</mat-label>
          <mat-select [value]="version" (selectionChange)="select($event.value)">
            @for (option of options(); track option.value) {
              <mat-option [value]="option.value">{{ option.label }}</mat-option>
            }
          </mat-select>
        </mat-form-field>
      }
      <app-file-viewer [source]="source(version)" />
    } @else {
      <p class="muted">No version to show.</p>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
      padding-top: 1rem;
    }
    .pick {
      max-width: 22rem;
    }
  `
})
export class VersionFiles {
  public readonly ns = input.required<string>();
  public readonly pkg = input.required<string>();
  public readonly versions = input.required<readonly PackageVersion[]>();
  public readonly owner = input.required<boolean>();
  public readonly shown = model<string | null>(null);

  protected readonly options = computed(() =>
    this.owner()
      ? this.versions()
          .filter((version) => !version.purged)
          .map((version) => ({ value: version.version, label: version.yanked ? `${version.version} · Withdrawn` : version.version }))
      : []
  );

  protected source(version: string): string {
    return versionFiles(this.ns(), this.pkg(), version);
  }

  protected select(value: unknown): void {
    if (typeof value === "string") {
      this.shown.set(value);
    }
  }
}

/** A version's download link, and for owners the button that withdraws or restores it. */
@Component({
  selector: "app-version-actions",
  imports: [MatButtonModule, Icon],
  template: `
    @if (archive(); as href) {
      <a mat-button [href]="href" download><app-icon name="download" />Download</a>
    }
    <button mat-button type="button" [hidden]="!owner() || version().purged" [class.danger]="!version().yanked" (click)="toggle()">{{ version().yanked ? "Restore" : "Withdraw" }}</button>
    <button mat-button type="button" class="danger" [hidden]="!admin() || version().purged" (click)="purge.emit(version())">Purge</button>
  `,
  host: { class: "row" }
})
export class VersionActions {
  public readonly version = input.required<PackageVersion>();
  /** The zip, for versions the viewer may read: owners any, everyone else live ones. */
  public readonly archive = input.required<string | null>();
  public readonly owner = input.required<boolean>();
  public readonly admin = input.required<boolean>();
  public readonly withdraw = output<PackageVersion>();
  public readonly restore = output<PackageVersion>();
  /** Admins only: delete the version's files for good, say after a leaked secret. */
  public readonly purge = output<PackageVersion>();

  protected toggle(): void {
    const version = this.version();
    (version.yanked ? this.restore : this.withdraw).emit(version);
  }
}

interface VersionRow {
  readonly version: PackageVersion;
  readonly badges: readonly { readonly label: string; readonly badge: string }[];
  readonly when: string;
  readonly archive: string | null;
}

@Component({
  selector: "app-version-list",
  imports: [VersionActions],
  template: `
    <ol>
      @for (row of rows(); track row.version.version) {
        <li>
          <div class="row">
            <strong>{{ row.version.version }}</strong>
            @for (badge of row.badges; track badge.label) {
              <span [class]="badge.badge">{{ badge.label }}</span>
            }
            <span class="spacer"></span>
            <span class="muted">{{ row.when }} · {{ row.version.publishedBy }}</span>
          </div>
          @if (row.version.changelog; as changelog) {
            <p class="changelog">{{ changelog }}</p>
          }
          <app-version-actions
            class="actions"
            [version]="row.version"
            [archive]="row.archive"
            [owner]="owner()"
            [admin]="admin()"
            (withdraw)="withdraw.emit($event)"
            (restore)="restore.emit($event)"
            (purge)="purge.emit($event)"
          />
        </li>
      }
    </ol>
  `,
  styles: `
    ol {
      list-style: none;
      margin: 1rem 0 0;
      padding: 0;
    }
    li {
      border-bottom: var(--border);
      padding: 0.9rem 0;
    }
    p {
      margin: 0.5rem 0 0;
    }
    .changelog {
      white-space: pre-line;
    }
    .actions {
      margin-top: 0.25rem;
    }
  `
})
export class VersionList {
  public readonly ns = input.required<string>();
  public readonly pkg = input.required<string>();
  public readonly versions = input.required<readonly PackageVersion[]>();
  public readonly liveVersion = input.required<string | null>();
  public readonly owner = input.required<boolean>();
  public readonly admin = input.required<boolean>();
  public readonly withdraw = output<PackageVersion>();
  public readonly restore = output<PackageVersion>();
  public readonly purge = output<PackageVersion>();

  protected readonly rows = computed<readonly VersionRow[]>(() =>
    this.versions().map((version) => {
      const badges = [
        ...(version.purged ? [{ label: "Purged", badge: "badge rejected" }] : version.yanked ? [{ label: "Withdrawn", badge: "badge" }] : []),
        ...(version.version === this.liveVersion() ? [{ label: "Live", badge: "badge live" }] : [])
      ];
      const readable = !version.purged && (this.owner() || !version.yanked);
      return { version, badges, when: formatDate(version.publishedAt), archive: readable ? archiveUrl(this.ns(), this.pkg(), version.version) : null };
    })
  );
}

/** Installs for the owner: one headline pair and where it is installed, as labelled bars. */
@Component({
  selector: "app-package-usage",
  template: `
    <div class="tiles">
      <div class="tile">
        <span class="value">{{ stats().installedBase }}</span>
        <span class="muted">people using it (30 days)</span>
      </div>
      <div class="tile">
        <span class="value">{{ stats().installs }}</span>
        <span class="muted">installs, all time</span>
      </div>
    </div>
    @if (agents().length > 0) {
      <h3>Where it's installed</h3>
      <ul class="bars">
        @for (row of agents(); track row.agent) {
          <li [title]="row.agent + ': ' + row.count">
            <span class="label">{{ row.agent }}</span>
            <span class="bar" aria-hidden="true"><span [style.width.%]="row.share"></span></span>
            <span class="count">{{ row.count }}</span>
          </li>
        }
      </ul>
    }
  `,
  styles: `
    :host {
      display: block;
      padding-top: 1rem;
    }
    .tiles {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(12rem, 1fr));
      gap: 1rem;
      margin-bottom: 1.5rem;
    }
    .tile {
      display: grid;
      gap: 0.25rem;
      border: var(--border);
      border-radius: var(--radius-small);
      padding: 1rem;
    }
    .value {
      font: var(--mat-sys-headline-medium);
      font-variant-numeric: tabular-nums;
    }
    .bars {
      list-style: none;
      margin: 0;
      padding: 0;
      display: grid;
      gap: 0.6rem;
    }
    li {
      display: grid;
      grid-template-columns: 10rem 1fr 3rem;
      align-items: center;
      gap: 0.75rem;
    }
    .bar {
      height: 10px;
      span {
        display: block;
        height: 100%;
        border-radius: 0 4px 4px 0;
        background: var(--mat-sys-primary);
      }
    }
    .count {
      text-align: right;
      font-variant-numeric: tabular-nums;
    }
  `
})
export class PackageUsage {
  public readonly stats = input.required<PackageStats>();

  protected readonly agents = computed(() => {
    const mix = Object.entries(this.stats().agentMix).toSorted(([, left], [, right]) => right - left);
    const top = Math.max(1, ...mix.map(([, count]) => count));
    return mix.map(([agent, count]) => ({ agent, count, share: Math.max(2, Math.round((count / top) * 100)) }));
  });
}

/** Install it through the desktop app, whole or one skill of a pack at a time. */
@Component({
  selector: "app-get-it",
  imports: [InstallButton, Icon],
  template: `
    @if (item().liveVersion !== null && !item().revoked) {
      <app-install-button [target]="item().id" />
      <p class="muted">Adds it to each app below that can use it. The app asks first.</p>
      @if (skills().length > 0) {
        <details class="skills">
          <summary>Install one skill instead</summary>
          <ul>
            @for (skill of skills(); track skill) {
              <li>
                <span>{{ skill }}</span>
                <app-install-button [target]="item().id + '/' + skill" label="Install" />
              </li>
            }
          </ul>
        </details>
      }
    } @else {
      <p class="muted">{{ unavailable() }}</p>
    }
    @if (hasServer()) {
      <p class="notice">
        This includes an <strong>MCP server</strong>, which lets the assistant use a tool or an online service. The app shows exactly what it runs or connects to and asks you before installing.
        @if (item().publicReview?.state === "approved") {
          <span class="checked"><app-icon name="verified" />Checked by IT.</span>
        } @else {
          <span class="unchecked"><app-icon name="shield" />Not checked by IT: shared by {{ item().publisher.displayName }}.</span>
        }
      </p>
    }
  `,
  styleUrl: "./package-side.scss"
})
export class GetIt {
  public readonly item = input.required<PackageDetail>();

  protected readonly unavailable = computed(() => {
    const item = this.item();
    if (item.revoked) {
      return item.revokedByAdmin ? "An admin removed it from every PC. It can't be installed." : "Its publisher removed it from every PC. It can't be installed.";
    }

    return "Every version was withdrawn, so there is nothing to install right now.";
  });
  /** The skills of a pack, each installable on its own; empty for a single-skill package. */
  public readonly skills = input.required<readonly string[]>();
  public readonly hasServer = input.required<boolean>();
}

/** Which apps get the package, and what the others miss. */
@Component({
  selector: "app-works-in",
  imports: [Icon],
  template: `
    <h2 id="works-heading">Works in</h2>
    <ul aria-labelledby="works-heading">
      @for (row of rows(); track row.app) {
        <li [class.no]="row.works === 'no'">
          <app-icon [name]="row.works === 'no' ? 'close' : 'check'" />
          <span>
            {{ row.app }}
            @if (row.works === "some") {
              <span class="muted">(partly)</span>
            }
            @if (row.note; as note) {
              <span class="note">{{ note }}</span>
            }
          </span>
        </li>
      }
    </ul>
  `,
  styles: `
    h2 {
      margin: 0 0 0.5rem;
      font: var(--mat-sys-title-small);
    }
    ul {
      list-style: none;
      margin: 0;
      padding: 0;
      display: grid;
      gap: 0.35rem;
      font: var(--mat-sys-body-medium);
    }
    li {
      display: flex;
      gap: 0.4rem;
      app-icon {
        color: var(--success);
      }
      &.no {
        color: var(--muted);
        app-icon {
          color: var(--muted);
        }
      }
    }
    .note {
      display: block;
      color: var(--muted);
      font: var(--mat-sys-body-small);
    }
  `
})
export class WorksInList {
  public readonly kinds = input.required<readonly string[]>();
  public readonly transports = input.required<readonly string[]>();

  protected readonly rows = computed(() => worksIn(this.kinds(), this.transports()));
}

/** Reports and feedback about a package, for its owners: read each, then mark it done with a note. */
@Component({
  selector: "app-package-reports",
  imports: [MatButtonModule, Icon],
  template: `
    @if (reports.hasValue()) {
      @for (report of reports.value(); track report.id) {
        <article class="card" [class.resolved]="report.resolvedAt !== null">
          <div class="row">
            <span [class]="report.kind === 'problem' ? 'badge rejected' : 'badge'">{{ report.kind === "problem" ? "Problem" : "Feedback" }}</span>
            <span class="muted">{{ report.account }} · {{ age(report.createdAt) }}</span>
            <span class="spacer"></span>
            @if (report.resolvedBy; as resolvedBy) {
              <span class="badge live">Answered by {{ resolvedBy }}</span>
            } @else if (report.kind === "problem" && !session.isAdmin()) {
              <span class="muted">The marketplace admins close problem reports.</span>
            } @else {
              <button mat-stroked-button type="button" (click)="answer(report)"><app-icon name="check" />Mark done</button>
            }
          </div>
          <p class="reason">{{ report.reason }}</p>
          @if (report.note; as note) {
            <p class="muted"><strong>Answer:</strong> {{ note }}</p>
          }
        </article>
      } @empty {
        <p class="muted">Nobody has reported a problem or sent feedback.</p>
      }
    } @else if (reports.error(); as error) {
      <p class="problem">{{ message(error) }}</p>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
      padding-top: 1rem;
    }
    p {
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
export class PackageReports {
  public readonly ns = input.required<string>();
  public readonly pkg = input.required<string>();
  public readonly changed = output();

  private readonly api = inject(Api);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);
  protected readonly session = inject(Session);

  protected readonly reports = resource({ params: () => ({ ns: this.ns(), pkg: this.pkg() }), loader: ({ params }) => this.api.packageReports(params.ns, params.pkg) });

  protected age(at: string): string {
    return formatAge(at);
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }

  protected answer(report: Report): void {
    runTask(this.resolve(report));
  }

  private async resolve(report: Report): Promise<void> {
    const note = await prompt(this.dialog, {
      title: "Mark this done?",
      message: `${report.account} sees that it's done, with your note if you write one.`,
      confirm: "Mark done",
      field: { label: "Note (optional)", hint: "What you changed, or why nothing needs to.", required: false }
    });
    if (note === undefined) {
      return;
    }

    try {
      await this.api.resolveReport(report.id, note);
      this.reports.reload();
      this.changed.emit();
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }
}

/** How to get it, the facts, and the technical details, in the sidebar. */
@Component({ selector: "app-package-side", imports: [RouterLink, MatButtonModule, Icon, GetIt, WorksInList], templateUrl: "./package-side.html", styleUrl: "./package-side.scss" })
export class PackageSide {
  public readonly item = input.required<PackageDetail>();
  public readonly version = input.required<PackageVersion | null>();
  /** The skills of a pack, each installable on its own; empty for a single-skill package. */
  public readonly skills = input.required<readonly string[]>();
  public readonly canReport = input.required<boolean>();
  public readonly canSuggest = input.required<boolean>();
  /** A problem goes to the owners and the admins; feedback to the owners. */
  public readonly report = output<ReportKind>();

  private readonly snackBar = inject(MatSnackBar);

  protected readonly hasServer = computed(() => this.version()?.componentKinds.includes("mcpServer") === true);
  protected readonly transports = computed(() => this.item().mcpServers.map((server) => server.transport));
  protected readonly installCommand = computed(() => `agent-plugins install ${this.item().id}`);
  protected readonly live = computed(() => this.item().versions.find((version) => version.version === this.item().liveVersion) ?? null);
  protected readonly updated = computed(() => {
    const live = this.live();
    return live === null ? "" : formatAge(live.publishedAt);
  });
  protected readonly size = computed(() => formatBytes(this.version()?.sizeBytes ?? 0));

  protected copy(): void {
    runTask(copyText(this.snackBar, this.installCommand()));
  }
}
