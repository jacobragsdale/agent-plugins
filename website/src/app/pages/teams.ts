import { Component, computed, inject, linkedSignal, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatRadioModule } from "@angular/material/radio";
import { Router, RouterLink } from "@angular/router";
import { Api, ApiError } from "../api";
import { namespacePattern, suggestNamespace } from "../format";
import { Session } from "../session";
import { Icon } from "../shared/icon";
import { runTask } from "../shared/tasks";

function text(event: Event): string {
  return event.target instanceof HTMLInputElement ? event.target.value : "";
}

/** Starts a team: its name, its short name for links, and who can see its skills. */
@Component({
  selector: "app-create-team",
  imports: [MatButtonModule, MatFormFieldModule, MatInputModule, MatRadioModule, Icon],
  template: `
    <form class="card stack create" (submit)="create($event)">
      <h2>Create a team</h2>
      <p class="muted">Everyone on a team can publish skills to its space. Send the invite link to bring colleagues in.</p>
      <mat-form-field appearance="outline">
        <mat-label>Team name</mat-label>
        <input matInput maxlength="120" required placeholder="Data Engineering" [value]="displayName()" (input)="displayName.set(text($event))" />
      </mat-form-field>
      <mat-form-field appearance="outline">
        <mat-label>Short name</mat-label>
        <input matInput maxlength="16" required [value]="namespace()" (input)="namespace.set(text($event).trim())" />
        <mat-hint>Used in links and in the names of installed skills. It can't change later.</mat-hint>
      </mat-form-field>
      @if (namespaceProblem(); as problem) {
        <p class="problem" role="alert">{{ problem }}</p>
      }
      <div class="stack tight">
        <span id="visibility-label" class="label">Who can see the team's skills?</span>
        <mat-radio-group class="choices" aria-labelledby="visibility-label" [value]="visibility()" (change)="setVisibility($event.value)">
          <mat-radio-button value="private">Only team members</mat-radio-button>
          <mat-radio-button value="public">Everyone at the company</mat-radio-button>
        </mat-radio-group>
        <span class="muted">You can share single skills with others either way, and change this later.</span>
      </div>
      @if (problem(); as failure) {
        <p class="problem" role="alert">{{ failure }}</p>
      }
      <div class="row">
        <button mat-flat-button type="submit" [disabled]="!ready()"><app-icon name="add" />Create team</button>
      </div>
    </form>
  `,
  styles: `
    .tight {
      gap: 0.4rem;
    }
    .label {
      font: var(--mat-sys-title-small);
    }
    .choices {
      display: grid;
    }
  `
})
export class CreateTeam {
  private readonly session = inject(Session);
  private readonly api = inject(Api);
  private readonly router = inject(Router);

  protected readonly displayName = signal("");
  /** Follows the name until the person edits it. */
  protected readonly namespace = linkedSignal(() => suggestNamespace(this.displayName()));
  protected readonly visibility = signal<"public" | "private">("private");
  protected readonly creating = signal(false);
  protected readonly problem = signal<string | null>(null);
  protected readonly text = text;

  protected readonly namespaceProblem = computed(() => {
    const namespace = this.namespace();
    return namespace.length === 0 || namespacePattern.test(namespace) ? null : "Use 2 to 16 lowercase letters, digits, and single hyphens, starting with a letter.";
  });

  protected readonly ready = computed(() => !this.creating() && this.displayName().trim().length > 0 && namespacePattern.test(this.namespace()));

  protected setVisibility(value: unknown): void {
    if (value === "public" || value === "private") {
      this.visibility.set(value);
    }
  }

  protected create(event: SubmitEvent): void {
    event.preventDefault();
    if (this.ready()) {
      runTask(this.send());
    }
  }

  private async send(): Promise<void> {
    this.creating.set(true);
    this.problem.set(null);
    try {
      const team = await this.api.createTeam(this.namespace(), this.displayName().trim(), this.visibility());
      // The new team is a space you can publish to; the session learns it before the next page asks.
      await this.session.refreshMe();
      await this.router.navigate(["/teams", team.namespace]);
    } catch (error) {
      this.problem.set(ApiError.from(error).message);
    } finally {
      this.creating.set(false);
    }
  }
}

/** Your teams, and a new one: anyone can start a team and invite people with a link. */
@Component({
  selector: "app-teams",
  imports: [RouterLink, MatProgressBarModule, Icon, CreateTeam],
  template: `
    <div class="page stack narrow">
      <h1>Teams</h1>

      @if (teams.hasValue()) {
        <ul class="list-box teams">
          @for (team of teams.value(); track team.namespace) {
            <li>
              <app-icon name="group" />
              <div class="grow">
                <a class="name" [routerLink]="['/teams', team.namespace]">{{ team.displayName }}</a>
                <p class="muted">{{ team.memberCount === 1 ? "1 member" : team.memberCount + " members" }} · {{ team.visibility === "private" ? "Private" : "Everyone can see its skills" }}</p>
              </div>
              <span class="muted">{{ team.role === "owner" ? "Owner" : "Member" }}</span>
            </li>
          } @empty {
            <li class="muted">You aren't on a team yet.</li>
          }
        </ul>
      } @else if (teams.error(); as error) {
        <p class="problem">{{ message(error) }}</p>
      } @else {
        <mat-progress-bar mode="indeterminate" aria-label="Loading your teams" />
      }

      <app-create-team />
    </div>
  `,
  styles: `
    .teams li {
      display: flex;
      align-items: center;
      gap: 1rem;
      app-icon {
        color: var(--muted);
      }
    }
    .grow {
      flex: 1;
      p {
        margin: 0.25rem 0 0;
      }
    }
    .name {
      font-weight: 600;
      color: inherit;
      text-decoration: none;
      &:hover {
        color: var(--mat-sys-primary);
      }
    }
  `
})
export class TeamsPage {
  private readonly api = inject(Api);

  protected readonly teams = resource({ loader: () => this.api.teams() });

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }
}
