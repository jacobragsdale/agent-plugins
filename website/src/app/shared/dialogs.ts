import { Component, computed, inject, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatButtonToggleModule } from "@angular/material/button-toggle";
import type { MatDialog } from "@angular/material/dialog";
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from "@angular/material/dialog";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { firstValueFrom } from "rxjs";
import { Api, ApiError } from "../api";
import { runTask } from "./tasks";

export interface PromptData {
  readonly title: string;
  readonly message: string;
  readonly confirm: string;
  /** Ask for text (a report reason, a review note); required when set. */
  readonly field?: { readonly label: string; readonly hint: string; readonly required: boolean };
  readonly danger?: boolean;
}

/** Confirms an action, optionally with a note. Closes with the note (or "") when confirmed, undefined when cancelled. */
@Component({
  selector: "app-prompt-dialog",
  imports: [MatDialogModule, MatButtonModule, MatFormFieldModule, MatInputModule],
  template: `
    <h2 mat-dialog-title>{{ data.title }}</h2>
    <mat-dialog-content>
      <p>{{ data.message }}</p>
      @if (data.field; as field) {
        <mat-form-field appearance="outline" class="full">
          <mat-label>{{ field.label }}</mat-label>
          <textarea matInput rows="4" maxlength="2048" [value]="text()" (input)="setText($event)"></textarea>
          <mat-hint>{{ field.hint }}</mat-hint>
        </mat-form-field>
      }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button type="button" mat-dialog-close>Cancel</button>
      <button mat-flat-button type="button" [class.danger]="data.danger === true" [disabled]="!ready()" (click)="done()">{{ data.confirm }}</button>
    </mat-dialog-actions>
  `,
  styles: `
    .full {
      width: 100%;
    }
    .danger {
      --mat-button-filled-container-color: var(--mat-sys-error);
      --mat-button-filled-label-text-color: var(--mat-sys-on-error);
    }
  `
})
export class PromptDialog {
  protected readonly data = inject<PromptData>(MAT_DIALOG_DATA);
  protected readonly text = signal("");
  protected readonly ready = computed(() => this.data.field?.required !== true || this.text().trim().length > 0);
  private readonly dialog = inject<MatDialogRef<PromptDialog, string>>(MatDialogRef);

  protected setText(event: Event): void {
    if (event.target instanceof HTMLTextAreaElement) {
      this.text.set(event.target.value);
    }
  }

  protected done(): void {
    this.dialog.close(this.text().trim());
  }
}

/** Opens a PromptDialog and resolves to the note when confirmed, or undefined when cancelled. */
export async function prompt(dialog: MatDialog, data: PromptData): Promise<string | undefined> {
  return firstValueFrom(dialog.open<PromptDialog, PromptData, string>(PromptDialog, { data, width: "32rem", autoFocus: data.field === undefined ? "dialog" : "first-tabbable" }).afterClosed());
}

export interface VisibilityData {
  readonly namespace: string;
  readonly packageId?: string;
  readonly label: string;
}

function lines(text: string): string[] {
  return text
    .split(/[\n,]/u)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
}

/** Who may see a package or a whole space: everyone, or a list of people and Windows groups. */
@Component({
  selector: "app-visibility-dialog",
  imports: [MatDialogModule, MatButtonModule, MatButtonToggleModule, MatFormFieldModule, MatInputModule],
  template: `
    <h2 mat-dialog-title>Who can see {{ data.label }}?</h2>
    <mat-dialog-content>
      @if (loaded()) {
        <mat-button-toggle-group aria-label="Visibility" [value]="everyone() ? 'everyone' : 'some'" (change)="everyone.set($event.value === 'everyone')">
          <mat-button-toggle value="everyone">Everyone</mat-button-toggle>
          <mat-button-toggle value="some">Only some people</mat-button-toggle>
        </mat-button-toggle-group>
        @if (everyone()) {
          <p class="muted">Everyone who signs in can find and install it.</p>
        } @else {
          <p class="muted">Only these people and group members can find and install it. You and admins always can.</p>
          <mat-form-field appearance="outline" class="full">
            <mat-label>People</mat-label>
            <textarea matInput rows="3" placeholder="CORP\\jane&#10;bob@corp.example" [value]="users()" (input)="users.set(value($event))"></textarea>
            <mat-hint>Windows accounts, one per line</mat-hint>
          </mat-form-field>
          <mat-form-field appearance="outline" class="full">
            <mat-label>Groups</mat-label>
            <textarea matInput rows="3" placeholder="Finance Team" [value]="groups()" (input)="groups.set(value($event))"></textarea>
            <mat-hint>Windows (Active Directory) group names, one per line</mat-hint>
          </mat-form-field>
        }
        @if (error(); as message) {
          <p class="problem" role="alert">{{ message }}</p>
        }
      } @else {
        <p class="muted">Loading…</p>
      }
    </mat-dialog-content>
    <mat-dialog-actions align="end">
      <button mat-button type="button" mat-dialog-close>Cancel</button>
      <button mat-flat-button type="button" [disabled]="!canSave()" (click)="save()">Save</button>
    </mat-dialog-actions>
  `,
  styles: `
    .full {
      width: 100%;
      margin-top: 0.75rem;
    }
    p {
      margin-top: 1rem;
    }
  `
})
export class VisibilityDialog {
  protected readonly data = inject<VisibilityData>(MAT_DIALOG_DATA);
  protected readonly loaded = signal(false);
  protected readonly everyone = signal(true);
  protected readonly users = signal("");
  protected readonly groups = signal("");
  protected readonly error = signal<string | null>(null);
  protected readonly saving = signal(false);
  protected readonly canSave = computed(() => this.loaded() && !this.saving() && (this.everyone() || lines(this.users()).length + lines(this.groups()).length > 0));
  private readonly api = inject(Api);
  private readonly dialog = inject<MatDialogRef<VisibilityDialog, boolean>>(MatDialogRef);

  constructor() {
    runTask(this.load());
  }

  protected value(event: Event): string {
    return event.target instanceof HTMLTextAreaElement ? event.target.value : "";
  }

  protected async save(): Promise<void> {
    this.saving.set(true);
    this.error.set(null);
    try {
      const everyone = this.everyone();
      await this.api.setAccess(this.data.namespace, this.data.packageId, everyone ? [] : lines(this.users()), everyone ? [] : lines(this.groups()));
      this.dialog.close(true);
    } catch (error) {
      this.error.set(ApiError.from(error).message);
    } finally {
      this.saving.set(false);
    }
  }

  private async load(): Promise<void> {
    try {
      const access = await this.api.access(this.data.namespace, this.data.packageId);
      this.everyone.set(access.users.length + access.groups.length === 0);
      this.users.set(access.users.join("\n"));
      this.groups.set(access.groups.join("\n"));
    } catch (error) {
      this.error.set(ApiError.from(error).message);
    } finally {
      this.loaded.set(true);
    }
  }
}

export async function editVisibility(dialog: MatDialog, data: VisibilityData): Promise<boolean> {
  return (await firstValueFrom(dialog.open<VisibilityDialog, VisibilityData, boolean>(VisibilityDialog, { data, width: "34rem" }).afterClosed())) === true;
}
