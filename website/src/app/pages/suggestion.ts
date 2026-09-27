import { Component, computed, inject, input, output, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MAT_DIALOG_DATA, MatDialog, MatDialogModule } from "@angular/material/dialog";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatSnackBar } from "@angular/material/snack-bar";
import { Router, RouterLink } from "@angular/router";
import { firstValueFrom } from "rxjs";
import type { Suggestion } from "../api";
import { Api, ApiError, suggestionFiles } from "../api";
import type { Bump } from "../format";
import { formatAge, nextVersion } from "../format";
import { Session } from "../session";
import { prompt } from "../shared/dialogs";
import { FileViewer } from "../shared/file-viewer";
import { Icon } from "../shared/icon";
import { describeKinds } from "../shared/package-card";
import { suggestionStates } from "../shared/suggestion-list";
import { runTask } from "../shared/tasks";

interface AcceptData {
  readonly name: string;
  readonly by: string;
  readonly versions: readonly string[];
}

/** Publishing a suggestion: the same three kinds of change as a new version, and the number that follows. */
@Component({
  selector: "app-accept-dialog",
  imports: [MatDialogModule, MatButtonModule, MatButtonToggleModule],
  template: `
    <h2 mat-dialog-title>Publish this change to {{ data.name }}?</h2>
    <mat-dialog-content class="stack">
      <p>It becomes the new version, credited to {{ data.by }}, and reaches everyone who has it at their next check.</p>
      <span id="accept-bump" class="label">What kind of change is this?</span>
      <mat-button-toggle-group aria-labelledby="accept-bump" [value]="bump()" (change)="setBump($event.value)">
        <mat-button-toggle value="patch">Small fix</mat-button-toggle>
        <mat-button-toggle value="minor">New feature</mat-button-toggle>
        <mat-button-toggle value="major">Big change</mat-button-toggle>
      </mat-button-toggle-group>
      <p class="muted">This will be version {{ version() }}.</p>
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button type="button" mat-dialog-close>Cancel</button>
      <button mat-flat-button type="button" [mat-dialog-close]="version()">Publish</button>
    </mat-dialog-actions>
  `
})
export class AcceptDialog {
  protected readonly data = inject<AcceptData>(MAT_DIALOG_DATA);
  protected readonly bump = signal<Bump>("patch");
  protected readonly version = computed(() => nextVersion(this.data.versions, this.bump()));

  protected setBump(value: unknown): void {
    if (value === "patch" || value === "minor" || value === "major") {
      this.bump.set(value);
    }
  }
}

/** What the suggester said, what became of it, and the decision buttons while it waits. */
@Component({
  selector: "app-suggestion-summary",
  imports: [RouterLink, MatButtonModule, Icon],
  template: `
    <h2>What they changed</h2>
    <p class="message">{{ suggestion().message }}</p>
    @if (suggestion().state === "pending" && suggestion().baseVersion !== suggestion().liveVersion) {
      <p class="notice">
        This was based on version {{ suggestion().baseVersion }}, and version {{ suggestion().liveVersion }} is live now. Publishing it replaces whatever changed since, so check the files first.
      </p>
    }
    @if (suggestion().note; as note) {
      <p class="note"><strong>The owners said:</strong> {{ note }}</p>
    }
    @if (suggestion().acceptedVersion; as version) {
      <p>Published as version {{ version }}. <a [routerLink]="['/p', suggestion().namespace, suggestion().packageId]">Open the skill</a></p>
    }
    @if (suggestion().state === "pending") {
      <div class="row">
        <button mat-flat-button type="button" [hidden]="!owner()" [disabled]="busy()" (click)="accept.emit()"><app-icon name="check" />Publish it</button>
        <button mat-stroked-button type="button" [hidden]="!owner()" [disabled]="busy()" (click)="decline.emit()"><app-icon name="close" />Don't publish</button>
        <button mat-button type="button" [hidden]="!mine()" [disabled]="busy()" (click)="withdraw.emit()">Withdraw my suggestion</button>
      </div>
    }
  `,
  styles: `
    p {
      margin: 0;
    }
    .message {
      white-space: pre-wrap;
    }
    [hidden] {
      display: none !important;
    }
  `,
  host: { class: "card stack" }
})
export class SuggestionSummary {
  public readonly suggestion = input.required<Suggestion>();
  /** The caller owns the package's space, so they decide. */
  public readonly owner = input.required<boolean>();
  /** The caller made the suggestion, so they may withdraw it. */
  public readonly mine = input.required<boolean>();
  public readonly busy = input.required<boolean>();
  public readonly accept = output();
  public readonly decline = output();
  public readonly withdraw = output();
}

/** A suggested change: what it changes, and for the owners, publish it or say why not. */
@Component({
  selector: "app-suggestion",
  imports: [RouterLink, MatButtonModule, MatProgressBarModule, FileViewer, Icon, SuggestionSummary],
  template: `
    <div class="page stack">
      @if (suggestion.hasValue()) {
        @let current = suggestion.value();
        <a class="back" [routerLink]="['/p', current.namespace, current.packageId]"><app-icon name="arrowBack" />{{ current.name }}</a>
        <header class="stack">
          <h1>Suggested change to {{ current.name }}</h1>
          <div class="row">
            <span [class]="states[current.state].badge">{{ states[current.state].label }}</span>
            <span class="muted">{{ current.suggestedByName }} · {{ age() }} · {{ kinds() }}</span>
          </div>
        </header>

        <app-suggestion-summary [suggestion]="current" [owner]="owner()" [mine]="mine()" [busy]="busy()" (accept)="accept(current)" (decline)="decline(current)" (withdraw)="withdraw(current)" />

        <section class="stack">
          <h2>Files</h2>
          <p class="muted">Marked against the live version: New, Changed, or Removed.</p>
          <app-file-viewer [source]="files()" />
        </section>
      } @else if (suggestion.error(); as error) {
        <div class="card">
          <h1>{{ status(error) === 404 ? "Not found" : "Couldn't load this suggestion" }}</h1>
          <p class="muted">{{ status(error) === 404 ? "It doesn't exist, or it isn't yours to see." : message(error) }}</p>
          <a mat-stroked-button routerLink="/mine">My skills</a>
        </div>
      } @else {
        <mat-progress-bar mode="indeterminate" aria-label="Loading" />
      }
    </div>
  `,
  styles: `
    h1 {
      margin: 0;
    }
  `
})
export class SuggestionPage {
  public readonly id = input.required<string>();

  private readonly api = inject(Api);
  private readonly session = inject(Session);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);
  private readonly router = inject(Router);

  protected readonly busy = signal(false);
  protected readonly suggestion = resource({ params: () => ({ id: Number(this.id()) }), loader: ({ params }) => this.api.suggestion(params.id) });
  protected readonly files = computed(() => suggestionFiles(Number(this.id())));

  protected readonly owner = computed(() => {
    const me = this.session.me();
    return me !== null && this.suggestion.hasValue() && (me.admin || me.namespaces.includes(this.suggestion.value().namespace));
  });

  protected readonly mine = computed(() => this.suggestion.hasValue() && this.suggestion.value().suggestedBy.toLowerCase() === this.session.me()?.account.toLowerCase());
  protected readonly states = suggestionStates;

  protected readonly age = computed(() => (this.suggestion.hasValue() ? formatAge(this.suggestion.value().suggestedAt) : ""));
  protected readonly kinds = computed(() => describeKinds(this.suggestion.hasValue() ? this.suggestion.value().componentKinds : []));

  protected status(error: unknown): number {
    return ApiError.from(error).status;
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }

  protected accept(suggestion: Suggestion): void {
    runTask(this.confirmAccept(suggestion));
  }

  protected decline(suggestion: Suggestion): void {
    runTask(this.confirmDecline(suggestion));
  }

  protected withdraw(suggestion: Suggestion): void {
    runTask(this.confirmWithdraw(suggestion));
  }

  private async confirmAccept(suggestion: Suggestion): Promise<void> {
    const detail = await this.api.package(suggestion.namespace, suggestion.packageId);
    const data: AcceptData = { name: suggestion.name, by: suggestion.suggestedByName, versions: detail.versions.map((candidate) => candidate.version) };
    const version = await firstValueFrom(this.dialog.open<AcceptDialog, AcceptData, string>(AcceptDialog, { data, width: "32rem" }).afterClosed());
    if (version === undefined) {
      return;
    }

    if (await this.act(() => this.api.decideSuggestion(suggestion.id, { decision: "accept", version }), `Published version ${version}.`)) {
      await this.session.refreshMe();
      await this.router.navigate(["/p", suggestion.namespace, suggestion.packageId]);
    }
  }

  private async confirmDecline(suggestion: Suggestion): Promise<void> {
    const note = await prompt(this.dialog, {
      title: "Don't publish this change?",
      message: `${suggestion.suggestedByName} sees your note.`,
      confirm: "Send",
      field: { label: "Why not?", hint: "Be specific and kind.", required: true }
    });
    if (note !== undefined && (await this.act(() => this.api.decideSuggestion(suggestion.id, { decision: "decline", note }), "Sent."))) {
      await this.session.refreshMe();
    }
  }

  private async confirmWithdraw(suggestion: Suggestion): Promise<void> {
    const confirmed = await prompt(this.dialog, { title: "Withdraw your suggestion?", message: "The owners won't see it anymore.", confirm: "Withdraw", danger: true });
    if (confirmed !== undefined) {
      await this.act(() => this.api.withdrawSuggestion(suggestion.id), "Withdrawn.");
    }
  }

  /** Runs a decision and reloads; false when it failed. */
  private async act(change: () => Promise<unknown>, done: string): Promise<boolean> {
    this.busy.set(true);
    try {
      await change();
      this.snackBar.open(done, undefined, { duration: 4000 });
      this.suggestion.reload();
      return true;
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
      return false;
    } finally {
      this.busy.set(false);
    }
  }
}
