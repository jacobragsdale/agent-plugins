import { Component, computed, inject, input, model, output, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MAT_DIALOG_DATA, MatDialog, MatDialogModule, MatDialogRef } from "@angular/material/dialog";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatRadioModule } from "@angular/material/radio";
import { MatSnackBar } from "@angular/material/snack-bar";
import { firstValueFrom } from "rxjs";
import type { Me, Person, TeamName, Visibility } from "../api";
import { Api, ApiError } from "../api";
import { Session } from "../session";
import { prompt } from "./dialogs";
import { Icon } from "./icon";
import type { Picked } from "./people-picker";
import { PeoplePicker } from "./people-picker";
import { copyText, runTask } from "./tasks";

export interface ShareData {
  readonly namespace: string;
  /** A package or bundle in the space; absent for the whole space. */
  readonly id?: string;
  /** What is shared, as a person reads it: "Review workflow", "everything in Data Team". */
  readonly label: string;
  readonly spaceName: string;
  /** Who always sees the space: "you", or "members of Data Team". */
  readonly owners: string;
}

function lines(text: string): string[] {
  return text
    .split(/[\n,]/u)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

/** Who else can see it: people and teams, and Windows groups where the server knows them. */
@Component({
  selector: "app-share-list",
  imports: [MatButtonModule, MatFormFieldModule, MatInputModule, Icon, PeoplePicker],
  template: `
    <app-people-picker label="Add a person or team" [teams]="true" (picked)="add($event)" />
    <ul class="entries">
      @for (user of users(); track user.account) {
        <li>
          <app-icon name="person" />
          <span class="grow"
            >{{ user.displayName }} <span class="muted">{{ user.account === user.displayName ? "" : user.account }}</span></span
          >
          <button mat-icon-button type="button" [attr.aria-label]="'Remove ' + user.displayName" (click)="removeUser(user)"><app-icon name="close" /></button>
        </li>
      }
      @for (team of teams(); track team.namespace) {
        <li>
          <app-icon name="group" />
          <span class="grow">{{ team.displayName }} <span class="muted">team</span></span>
          <button mat-icon-button type="button" [attr.aria-label]="'Remove ' + team.displayName" (click)="removeTeam(team)"><app-icon name="close" /></button>
        </li>
      }
    </ul>
    <p class="muted">{{ note() }}</p>
    @if (adGroups()) {
      <mat-form-field appearance="outline" class="full">
        <mat-label>Windows groups</mat-label>
        <textarea matInput rows="2" placeholder="Finance Team" [value]="groups()" (input)="setGroups($event)"></textarea>
        <mat-hint>Active Directory group names, one per line</mat-hint>
      </mat-form-field>
    }
  `,
  styles: `
    .full {
      width: 100%;
      margin-top: 0.75rem;
    }
    .entries {
      list-style: none;
      margin: 0.5rem 0 0;
      padding: 0;
      li {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        min-height: 2.5rem;
      }
    }
    .grow {
      flex: 1;
      overflow-wrap: anywhere;
    }
    p {
      margin: 0.5rem 0 0;
    }
  `
})
export class ShareList {
  public readonly visibility = input.required<Visibility>();
  public readonly adGroups = input.required<boolean>();
  public readonly users = model.required<readonly Person[]>();
  public readonly teams = model.required<readonly TeamName[]>();
  public readonly groups = model.required<string>();

  protected readonly note = computed(() => {
    const anyone = this.users().length + this.teams().length > 0;
    if (!anyone) {
      return "Nobody else yet.";
    }

    return this.visibility() === "public" ? "While it's public, everyone can see it anyway. The list is kept for when it's private again." : "";
  });

  protected setGroups(event: Event): void {
    if (event.target instanceof HTMLTextAreaElement) {
      this.groups.set(event.target.value);
    }
  }

  protected add(picked: Picked): void {
    if (picked.kind === "team") {
      this.teams.update((teams) => (teams.some((team) => team.namespace === picked.namespace) ? teams : [...teams, { namespace: picked.namespace, displayName: picked.displayName }]));
    } else {
      this.users.update((users) =>
        users.some((user) => user.account.toLowerCase() === picked.account.toLowerCase()) ? users : [...users, { account: picked.account, displayName: picked.displayName }]
      );
    }
  }

  protected removeUser(person: Person): void {
    this.users.update((users) => users.filter((user) => user !== person));
  }

  protected removeTeam(name: TeamName): void {
    this.teams.update((teams) => teams.filter((team) => team !== name));
  }
}

/** The link that shares it: shown once it exists, copied on request, replaceable. */
@Component({
  selector: "app-share-link",
  imports: [MatButtonModule, Icon],
  template: `
    @if (link(); as url) {
      <p>
        <code class="link">{{ url }}</code>
      </p>
    }
    <div class="row">
      <button mat-stroked-button type="button" [disabled]="busy()" (click)="copyLink.emit()"><app-icon name="link" />Copy link</button>
      @if (link() !== null) {
        <button mat-button type="button" [disabled]="busy()" (click)="resetLink.emit()">Reset link</button>
      }
    </div>
    <p class="muted">Anyone at the company who opens the link can see and install it, and is added to the list above.</p>
  `,
  styles: `
    p {
      margin: 0.5rem 0 0;
    }
    .link {
      overflow-wrap: anywhere;
    }
  `
})
export class ShareLink {
  public readonly link = input.required<string | null>();
  public readonly busy = input.required<boolean>();
  public readonly copyLink = output();
  public readonly resetLink = output();
}

/** General access (same as the space, private, or public), who else can see it, and a link that shares it. */
@Component({
  selector: "app-share-dialog",
  imports: [MatDialogModule, MatButtonModule, MatRadioModule, ShareList, ShareLink],
  template: `
    <h2 mat-dialog-title>Share {{ data.label }}</h2>
    <mat-dialog-content>
      @if (loaded()) {
        <h3 id="access-label">General access</h3>
        <mat-radio-group class="access" aria-labelledby="access-label" [value]="visibility()" (change)="setVisibility($event.value)">
          @if (data.id !== undefined) {
            <mat-radio-button value="inherit">Same as {{ data.spaceName }} ({{ spaceAccess() }})</mat-radio-button>
          }
          <mat-radio-button value="private">Private</mat-radio-button>
          <mat-radio-button value="public">Everyone at the company</mat-radio-button>
        </mat-radio-group>
        <p class="muted">{{ explanation() }}</p>

        <h3>People and teams</h3>
        <app-share-list [visibility]="visibility()" [adGroups]="adGroups()" [(users)]="users" [(teams)]="teams" [(groups)]="groups" />

        <h3>Share link</h3>
        <app-share-link [link]="link()" [busy]="linkBusy()" (copyLink)="copyLink()" (resetLink)="resetLink()" />
      } @else if (error() === null) {
        <p class="muted">Loading…</p>
      }
      @if (error(); as message) {
        <p class="problem" role="alert">{{ message }}</p>
      }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button type="button" mat-dialog-close>Cancel</button>
      <button mat-flat-button type="button" [disabled]="!loaded() || saving()" (click)="save()">Save</button>
    </mat-dialog-actions>
  `,
  styles: `
    h3 {
      font: var(--mat-sys-title-small);
      margin: 1.25rem 0 0.5rem;
    }
    .access {
      display: grid;
    }
    p {
      margin: 0.5rem 0 0;
    }
  `
})
export class ShareDialog {
  protected readonly data = inject<ShareData>(MAT_DIALOG_DATA);
  protected readonly loaded = signal(false);
  protected readonly visibility = signal<Visibility>("inherit");
  /** The space's own setting, which "Same as its space" follows. */
  protected readonly spacePublic = signal(true);
  protected readonly users = signal<readonly Person[]>([]);
  protected readonly teams = signal<readonly TeamName[]>([]);
  protected readonly groups = signal("");
  protected readonly link = signal<string | null>(null);
  protected readonly linkBusy = signal(false);
  protected readonly error = signal<string | null>(null);
  protected readonly saving = signal(false);
  protected readonly adGroups = computed(() => this.session.health()?.adGroups === true || lines(this.groups()).length > 0);

  protected readonly spaceAccess = computed(() => (this.spacePublic() ? "everyone" : "private"));

  protected readonly explanation = computed(() => {
    const everyone = "Everyone who signs in can find and install it.";
    const some = `Only ${this.data.owners} and the people and teams below can find and install it.`;
    switch (this.visibility()) {
      case "public":
        return everyone;
      case "private":
        return some;
      case "inherit":
        return `It follows ${this.data.spaceName}. ${this.spacePublic() ? everyone : some}`;
    }
  });

  private readonly api = inject(Api);
  private readonly session = inject(Session);
  private readonly dialog = inject<MatDialogRef<ShareDialog, boolean>>(MatDialogRef);
  private readonly dialogs = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  constructor() {
    runTask(this.load());
  }

  protected setVisibility(value: unknown): void {
    if (value === "inherit" || value === "public" || value === "private") {
      this.visibility.set(value);
    }
  }

  protected copyLink(): void {
    runTask(this.fetchLink(false));
  }

  protected resetLink(): void {
    runTask(this.confirmReset());
  }

  protected save(): void {
    runTask(this.store());
  }

  private async confirmReset(): Promise<void> {
    const confirmed = await prompt(this.dialogs, {
      title: "Reset the share link?",
      message: "The old link stops working. People who already opened it keep access; remove them from the list if they shouldn't.",
      confirm: "Reset",
      danger: true
    });
    if (confirmed !== undefined) {
      await this.fetchLink(true);
    }
  }

  private async fetchLink(reset: boolean): Promise<void> {
    this.linkBusy.set(true);
    this.error.set(null);
    try {
      const link = await this.api.shareLink(this.data.namespace, this.data.id, reset);
      this.link.set(link);
      await copyText(this.snackBar, link, reset ? "New link copied. The old one no longer works." : "Link copied.");
    } catch (error) {
      this.error.set(ApiError.from(error).message);
    } finally {
      this.linkBusy.set(false);
    }
  }

  private async store(): Promise<void> {
    this.saving.set(true);
    this.error.set(null);
    try {
      await this.api.setShare(this.data.namespace, this.data.id, {
        visibility: this.visibility(),
        users: this.users().map((user) => user.account),
        teams: this.teams().map((team) => team.namespace),
        groups: lines(this.groups())
      });
      this.dialog.close(true);
    } catch (error) {
      this.error.set(ApiError.from(error).message);
    } finally {
      this.saving.set(false);
    }
  }

  private async load(): Promise<void> {
    try {
      const [share, space] = await Promise.all([this.api.share(this.data.namespace, this.data.id), this.data.id === undefined ? null : this.api.share(this.data.namespace)]);
      this.visibility.set(share.visibility);
      this.spacePublic.set((space ?? share).effective === "public");
      this.users.set(share.users);
      this.teams.set(share.teams);
      this.groups.set(share.groups.join("\n"));
      this.link.set(share.link);
      this.loaded.set(true);
    } catch (error) {
      this.error.set(ApiError.from(error).message);
    }
  }
}

/** Opens the share dialog; true when the person saved. */
export async function share(dialog: MatDialog, data: ShareData): Promise<boolean> {
  return (await firstValueFrom(dialog.open<ShareDialog, ShareData, boolean>(ShareDialog, { data, width: "36rem", autoFocus: "dialog" }).afterClosed())) === true;
}

/** How the share dialog names a space and who always sees it, from the caller's point of view. */
export function spaceWords(me: Me | null, ns: string, fallback: string): Pick<ShareData, "spaceName" | "owners"> {
  const team = me?.teams.find((candidate) => candidate.namespace === ns);
  if (team !== undefined) {
    return { spaceName: team.displayName, owners: `members of ${team.displayName}` };
  }

  if (ns === me?.namespace) {
    return { spaceName: "your space", owners: "you" };
  }

  return ns === "official" ? { spaceName: "Official", owners: "official publishers" } : { spaceName: fallback, owners: `the owners of ${fallback}` };
}
