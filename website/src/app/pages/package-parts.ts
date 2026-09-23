import { Component, computed, inject, input, model, output } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatSelectModule } from "@angular/material/select";
import { MatSnackBar } from "@angular/material/snack-bar";
import { RouterLink } from "@angular/router";
import type { IndexPackage, PackageDetail, PackageStats, PackageVersion } from "../api";
import { archiveUrl } from "../api";
import { formatAge, formatBytes, formatDate } from "../format";
import { editVisibility } from "../shared/dialogs";
import { FileViewer } from "../shared/file-viewer";
import { Icon } from "../shared/icon";
import { LaneBadge } from "../shared/lane-badge";
import type { PackageStatus } from "../shared/status";
import { versionState } from "../shared/status";
import { runTask } from "../shared/tasks";

@Component({
  selector: "app-package-header",
  imports: [Icon, LaneBadge],
  template: `
    <h1>{{ item().name }}</h1>
    <div class="row badges">
      @if (entry(); as listed) {
        <app-lane-badge [lane]="listed.lane" [publisher]="listed.publisher.displayName" />
        @if (listed.restricted) {
          <span class="badge"><app-icon name="lock" />Only some people can see this</span>
        }
      }
      <span class="badge">{{ kinds() }}</span>
      @for (tag of item().tags; track tag) {
        <span class="badge tag">#{{ tag }}</span>
      }
    </div>
    <p class="lead">{{ item().description }}</p>
  `,
  styles: `
    .badges {
      gap: 0.4rem;
      margin-bottom: 0.75rem;
    }
    .tag {
      background: none;
      border: var(--border);
    }
  `
})
export class PackageHeader {
  public readonly item = input.required<PackageDetail>();
  public readonly entry = input.required<IndexPackage | null>();
  public readonly kinds = input.required<string>();
}

/** What the owner sees: the review status and what they can do next. */
@Component({
  selector: "app-owner-panel",
  imports: [RouterLink, MatButtonModule, Icon],
  template: `
    <div class="row">
      <span [class]="status().badge">{{ status().label }}</span>
      <p class="detail">{{ status().detail }}</p>
    </div>
    <div class="row">
      <a mat-flat-button [routerLink]="['/p', ns(), pkg(), 'upload']"><app-icon name="upload" />Upload a new version</a>
      <button mat-stroked-button type="button" (click)="visibility()"><app-icon name="visibility" />Who can see this</button>
    </div>
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
      border-color: var(--mat-sys-primary);
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
  public readonly status = input.required<PackageStatus>();
  public readonly changed = output();

  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected visibility(): void {
    runTask(this.editVisibility());
  }

  private async editVisibility(): Promise<void> {
    if (await editVisibility(this.dialog, { namespace: this.ns(), packageId: this.pkg(), label: this.name() })) {
      this.snackBar.open("Visibility saved.", undefined, { duration: 4000 });
      this.changed.emit();
    }
  }
}

/** The files of the version being shown; owners can switch to pending or older versions. */
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
      <app-file-viewer [ns]="ns()" [packageId]="pkg()" [version]="version" />
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

  protected readonly options = computed(() => (this.owner() ? this.versions().map((version) => ({ value: version.version, label: `${version.version} · ${versionState(version).label}` })) : []));

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
    @if (owner()) {
      <button mat-button type="button" [class.withdraw]="!version().yanked" (click)="toggle()">{{ version().yanked ? "Restore" : "Withdraw" }}</button>
    }
  `,
  styles: `
    .withdraw {
      color: var(--mat-sys-error);
    }
  `,
  host: { class: "row" }
})
export class VersionActions {
  public readonly version = input.required<PackageVersion>();
  /** The zip, for versions the viewer may read: owners any, everyone else live ones. */
  public readonly archive = input.required<string | null>();
  public readonly owner = input.required<boolean>();
  public readonly withdraw = output<PackageVersion>();
  public readonly restore = output<PackageVersion>();

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
            <p>{{ changelog }}</p>
          }
          @if (row.version.reviewNote; as note) {
            <p class="note"><strong>Reviewer:</strong> {{ note }}</p>
          }
          <app-version-actions class="actions" [version]="row.version" [archive]="row.archive" [owner]="owner()" (withdraw)="withdraw.emit($event)" (restore)="restore.emit($event)" />
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
    .note {
      color: var(--mat-sys-on-surface-variant);
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
  public readonly withdraw = output<PackageVersion>();
  public readonly restore = output<PackageVersion>();

  protected readonly rows = computed<readonly VersionRow[]>(() =>
    this.versions().map((version) => {
      const state = versionState(version);
      const badges = [...(this.owner() || version.yanked ? [state] : []), ...(version.version === this.liveVersion() ? [{ label: "Live", badge: "badge live" }] : [])];
      const readable = this.owner() || (version.reviewState === "approved" && !version.yanked);
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

/** How to get it, the facts, and the technical details, in the sidebar. */
@Component({ selector: "app-package-side", imports: [RouterLink, MatButtonModule, Icon], templateUrl: "./package-side.html", styleUrl: "./package-side.scss" })
export class PackageSide {
  public readonly item = input.required<PackageDetail>();
  public readonly entry = input.required<IndexPackage | null>();
  public readonly version = input.required<PackageVersion | null>();
  public readonly canReport = input.required<boolean>();
  public readonly report = output();

  private readonly snackBar = inject(MatSnackBar);

  protected readonly hasServer = computed(() => this.version()?.componentKinds.includes("mcpServer") === true);
  protected readonly installCommand = computed(() => `agent-plugins install ${this.item().id}`);
  protected readonly updated = computed(() => {
    const entry = this.entry();
    return entry === null ? "" : formatAge(entry.publishedAt);
  });
  protected readonly size = computed(() => formatBytes(this.version()?.sizeBytes ?? 0));

  protected copy(): void {
    runTask(this.copyCommand());
  }

  private async copyCommand(): Promise<void> {
    try {
      await navigator.clipboard.writeText(this.installCommand());
      this.snackBar.open("Copied.", undefined, { duration: 2000 });
    } catch {
      this.snackBar.open("The browser blocked the clipboard; select the command and copy it instead.", "Dismiss");
    }
  }
}
