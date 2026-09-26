import { Component, computed, inject, input, output, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { Router, RouterLink } from "@angular/router";
import type { Member, Team } from "../api";
import { Api, ApiError } from "../api";
import { Session } from "../session";
import { prompt } from "../shared/dialogs";
import { Icon } from "../shared/icon";
import type { Picked } from "../shared/people-picker";
import { PeoplePicker } from "../shared/people-picker";
import { share } from "../shared/share-dialog";
import { copyText, runTask } from "../shared/tasks";

/** The link that invites people; any member can copy it, owners can replace it. */
@Component({
  selector: "app-team-invite",
  imports: [MatButtonModule, Icon],
  template: `
    <h2>Invite people</h2>
    <p class="muted">Send this link to colleagues. Opening it lets them join the team and publish here. Anyone who has the link can join.</p>
    @if (team().invite; as link) {
      <p>
        <code class="link">{{ link }}</code>
      </p>
    }
    <div class="row">
      <button mat-flat-button type="button" [disabled]="busy()" (click)="copyLink.emit()"><app-icon name="link" />Copy invite link</button>
      @if (team().role !== "member" && team().invite !== null) {
        <button mat-button type="button" [disabled]="busy()" (click)="resetLink.emit()">Reset link</button>
      }
    </div>
  `,
  styles: `
    h2 {
      font: var(--mat-sys-title-large);
      margin: 0;
    }
    p {
      margin: 0;
    }
    .link {
      overflow-wrap: anywhere;
    }
  `,
  host: { class: "card stack" }
})
export class TeamInvite {
  public readonly team = input.required<Team>();
  public readonly busy = input.required<boolean>();
  public readonly copyLink = output();
  public readonly resetLink = output();
}

/** Who is on the team; owners add people, make owners, and remove them. */
@Component({
  selector: "app-team-members",
  imports: [MatButtonModule, Icon, PeoplePicker],
  template: `
    <h2>Members ({{ team().members.length }})</h2>
    @if (manage()) {
      <app-people-picker label="Add people" (picked)="add.emit($event)" />
    }
    <ul>
      @for (member of team().members; track member.account) {
        <li>
          <app-icon name="person" />
          <span class="grow">
            {{ member.displayName }} <span class="muted">{{ member.account }}</span>
            @if (member.owner) {
              <span class="badge team">Owner</span>
            }
          </span>
          @if (isYou(member)) {
            @if (!(member.owner && lastOwner())) {
              <button mat-button type="button" [disabled]="busy()" (click)="leave.emit()">Leave team</button>
            }
          } @else if (manage()) {
            <button mat-button type="button" [disabled]="busy()" (click)="setOwner.emit(member)">{{ member.owner ? "Remove as owner" : "Make owner" }}</button>
            <button mat-button type="button" class="danger" [disabled]="busy()" (click)="remove.emit(member)">Remove</button>
          }
        </li>
      }
    </ul>
  `,
  styles: `
    h2 {
      font: var(--mat-sys-title-large);
      margin: 0;
    }
    ul {
      list-style: none;
      margin: 0;
      padding: 0;
    }
    li {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: 0.5rem;
      padding: 0.5rem 0;
      border-bottom: var(--border);
    }
    .grow {
      flex: 1 1 16rem;
    }
    .danger {
      color: var(--mat-sys-error);
    }
  `,
  host: { class: "card stack" }
})
export class TeamMembers {
  public readonly team = input.required<Team>();
  public readonly busy = input.required<boolean>();
  /** The signed-in account, whose row offers Leave instead. */
  public readonly you = input.required<string>();
  public readonly add = output<Picked>();
  /** Flips the member's owner flag. */
  public readonly setOwner = output<Member>();
  public readonly remove = output<Member>();
  public readonly leave = output();

  protected readonly manage = computed(() => this.team().role !== "member");
  /** A team keeps at least one owner, so the only one can't leave until they make someone else an owner. */
  protected readonly lastOwner = computed(() => this.team().members.filter((member) => member.owner).length === 1);

  protected isYou(member: Member): boolean {
    return member.account.toLowerCase() === this.you().toLowerCase();
  }
}

/** One team: who is on it, the link that invites more, and who can see its skills. */
@Component({
  selector: "app-team",
  imports: [RouterLink, MatButtonModule, MatProgressBarModule, Icon, TeamInvite, TeamMembers],
  template: `
    <div class="page stack">
      <a class="back" routerLink="/teams"><app-icon name="arrowBack" />Teams</a>
      @if (team.hasValue()) {
        @let current = team.value();
        <header class="row">
          <div class="grow">
            <h1>{{ current.displayName }}</h1>
            <div class="row badges">
              <span class="badge team"><app-icon name="group" />Team · {{ current.namespace }}</span>
              <span class="badge"
                ><app-icon [name]="current.visibility === 'private' ? 'lock' : 'visibility'" />{{ current.visibility === "private" ? "Private" : "Everyone can see its skills" }}</span
              >
              <span class="badge">{{ roles[current.role] }}</span>
            </div>
          </div>
          <a mat-stroked-button routerLink="/browse" [queryParams]="{ space: current.namespace }">See its skills</a>
          <a mat-flat-button routerLink="/publish" [queryParams]="{ to: current.namespace }"><app-icon name="add" />Share a skill</a>
        </header>

        <app-team-invite [team]="current" [busy]="busy()" (copyLink)="copyInvite(current)" (resetLink)="resetInvite()" />
        <app-team-members [team]="current" [busy]="busy()" [you]="you()" (add)="add($event)" (setOwner)="setOwner($event, !$event.owner)" (remove)="remove($event)" (leave)="leave(current)" />
        @if (canManage()) {
          <section class="card stack">
            <h2>Settings</h2>
            <div class="row">
              <button mat-stroked-button type="button" [disabled]="busy()" (click)="shareSpace(current)"><app-icon name="share" />Who can see its skills</button>
              <button mat-stroked-button type="button" [disabled]="busy()" (click)="rename(current)"><app-icon name="edit" />Rename</button>
              <span class="spacer"></span>
              <button mat-button type="button" class="danger" [disabled]="busy()" (click)="remove(null)"><app-icon name="delete" />Delete team</button>
            </div>
            <p class="muted">A team can be deleted once it has no skills or bundles left.</p>
          </section>
        }
      } @else if (team.error(); as error) {
        <div class="card">
          <h1>{{ status(error) === 404 ? "Team not found" : "Couldn't load this team" }}</h1>
          <p class="muted">{{ status(error) === 404 ? "It doesn't exist, or you aren't on it. Ask a member for an invite link." : message(error) }}</p>
        </div>
      } @else {
        <mat-progress-bar mode="indeterminate" aria-label="Loading the team" />
      }
    </div>
  `,
  styles: `
    .page {
      max-width: 960px;
    }
    .back {
      display: inline-flex;
      align-items: center;
      gap: 0.35rem;
      color: var(--muted);
      text-decoration: none;
      font: var(--mat-sys-label-large);
      width: fit-content;
    }
    .grow {
      flex: 1 1 16rem;
    }
    .badges {
      gap: 0.4rem;
    }
    h2 {
      font: var(--mat-sys-title-large);
      margin: 0;
    }
    p {
      margin: 0;
    }
    .danger {
      color: var(--mat-sys-error);
    }
  `
})
export class TeamPage {
  public readonly ns = input.required<string>();

  private readonly session = inject(Session);
  private readonly api = inject(Api);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);
  private readonly router = inject(Router);

  protected readonly team = resource({ params: () => ({ ns: this.ns() }), loader: ({ params }) => this.api.team(params.ns) });
  protected readonly busy = signal(false);
  protected readonly canManage = computed(() => this.team.hasValue() && this.team.value().role !== "member");
  protected readonly you = computed(() => this.session.me()?.account ?? "");
  protected readonly roles = { owner: "You're an owner", member: "You're a member", admin: "You're an admin" } as const;

  protected status(error: unknown): number {
    return ApiError.from(error).status;
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }

  protected copyInvite(team: Team): void {
    runTask(this.act(async () => copyText(this.snackBar, await this.api.teamInvite(team.namespace, false), "Invite link copied.")));
  }

  protected resetInvite(): void {
    runTask(this.confirmResetInvite());
  }

  protected add(picked: Picked): void {
    if (picked.kind === "user") {
      runTask(this.act(async () => this.api.setMember(this.ns(), picked.account, false), `${picked.displayName} is on the team.`));
    }
  }

  protected setOwner(member: Member, owner: boolean): void {
    runTask(this.act(async () => this.api.setMember(this.ns(), member.account, owner), owner ? `${member.displayName} is an owner now.` : `${member.displayName} is a member now.`));
  }

  /** Removes a member, or with null deletes the whole team. */
  protected remove(member: Member | null): void {
    runTask(member === null ? this.confirmDelete() : this.confirmRemove(member));
  }

  protected leave(team: Team): void {
    runTask(this.confirmLeave(team));
  }

  protected shareSpace(team: Team): void {
    runTask(this.openShare(team));
  }

  protected rename(team: Team): void {
    runTask(this.askName(team));
  }

  private async confirmResetInvite(): Promise<void> {
    const confirmed = await prompt(this.dialog, {
      title: "Reset the invite link?",
      message: "The old link stops working. People who already joined stay on the team.",
      confirm: "Reset",
      danger: true
    });
    if (confirmed !== undefined) {
      await this.act(async () => copyText(this.snackBar, await this.api.teamInvite(this.ns(), true), "New invite link copied. The old one no longer works."));
    }
  }

  private async confirmRemove(member: Member): Promise<void> {
    const confirmed = await prompt(this.dialog, {
      title: `Remove ${member.displayName}?`,
      message: "They can no longer publish here. If the team's skills are private, they stop seeing them too.",
      confirm: "Remove",
      danger: true
    });
    if (confirmed !== undefined) {
      await this.act(() => this.api.removeMember(this.ns(), member.account), `${member.displayName} was removed.`);
    }
  }

  private async confirmLeave(team: Team): Promise<void> {
    const confirmed = await prompt(this.dialog, {
      title: `Leave ${team.displayName}?`,
      message: "You can no longer publish here. To come back, you need an invite link from a member.",
      confirm: "Leave",
      danger: true
    });
    const account = this.session.me()?.account;
    if (confirmed === undefined || account === undefined) {
      return;
    }

    if (await this.act(() => this.api.removeMember(team.namespace, account), `You left ${team.displayName}.`, false)) {
      await this.session.refreshMe();
      await this.router.navigate(["/teams"]);
    }
  }

  private async confirmDelete(): Promise<void> {
    const confirmed = await prompt(this.dialog, { title: "Delete this team?", message: "Its members, invite link, and settings are removed. This can't be undone.", confirm: "Delete", danger: true });
    if (confirmed !== undefined && (await this.act(() => this.api.deleteTeam(this.ns()), "Team deleted.", false))) {
      await this.session.refreshMe();
      await this.router.navigate(["/teams"]);
    }
  }

  private async openShare(team: Team): Promise<void> {
    if (await share(this.dialog, { namespace: team.namespace, label: `everything in ${team.displayName}`, spaceName: team.displayName, owners: `members of ${team.displayName}` })) {
      this.snackBar.open("Saved. A skill's own setting still wins over the team's.", undefined, { duration: 5000 });
      this.team.reload();
    }
  }

  private async askName(team: Team): Promise<void> {
    const name = await prompt(this.dialog, {
      title: "Rename the team",
      message: "The new name shows everywhere the team's skills do. Its short name stays the same.",
      confirm: "Rename",
      field: { label: "Team name", hint: "Up to 120 characters", required: true, single: true, value: team.displayName }
    });
    if (name !== undefined) {
      await this.act(() => this.api.renameTeam(team.namespace, name), "Renamed.");
    }
  }

  /** Runs a change, reports it, and reloads the team; false when it failed. */
  private async act(change: () => Promise<unknown>, done?: string, reload = true): Promise<boolean> {
    this.busy.set(true);
    try {
      await change();
      if (done !== undefined) {
        this.snackBar.open(done, undefined, { duration: 4000 });
      }

      if (reload) {
        this.team.reload();
      }

      return true;
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
      return false;
    } finally {
      this.busy.set(false);
    }
  }
}
