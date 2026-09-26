import { Component, computed, inject, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import type { MatDialog } from "@angular/material/dialog";
import { MAT_DIALOG_DATA, MatDialogModule, MatDialogRef } from "@angular/material/dialog";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { firstValueFrom } from "rxjs";

export interface PromptData {
  readonly title: string;
  readonly message: string;
  readonly confirm: string;
  /** Ask for text (a report reason, a review note); required when set. `single` asks for one line, starting at `value`. */
  readonly field?: { readonly label: string; readonly hint: string; readonly required: boolean; readonly single?: boolean; readonly value?: string };
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
          @if (field.single === true) {
            <input matInput maxlength="256" [value]="text()" (input)="setText($event)" (keydown.enter)="enter($event)" />
          } @else {
            <textarea matInput rows="4" maxlength="2048" [value]="text()" (input)="setText($event)"></textarea>
          }
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
  protected readonly text = signal(this.data.field?.value ?? "");
  protected readonly ready = computed(() => this.data.field?.required !== true || this.text().trim().length > 0);
  private readonly dialog = inject<MatDialogRef<PromptDialog, string>>(MatDialogRef);

  protected setText(event: Event): void {
    if (event.target instanceof HTMLTextAreaElement || event.target instanceof HTMLInputElement) {
      this.text.set(event.target.value);
    }
  }

  protected enter(event: Event): void {
    event.preventDefault();
    if (this.ready()) {
      this.done();
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
