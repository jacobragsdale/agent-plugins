import { Component, computed, effect, inject, input, linkedSignal, model, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatDialog } from "@angular/material/dialog";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { MatRadioModule } from "@angular/material/radio";
import { MatSnackBar } from "@angular/material/snack-bar";
import { Router, RouterLink } from "@angular/router";
import type { IndexPackage } from "../api";
import { Api, ApiError } from "../api";
import { idPattern, slugify } from "../format";
import { Session } from "../session";
import { prompt } from "../shared/dialogs";
import { Icon } from "../shared/icon";
import { runTask } from "../shared/tasks";

const maxMembers = 50;

function text(event: Event): string {
  return event.target instanceof HTMLInputElement || event.target instanceof HTMLTextAreaElement ? event.target.value : "";
}

/** Whose bundle it is and its ID, both fixed once it exists. */
@Component({
  selector: "app-bundle-identity",
  imports: [MatFormFieldModule, MatInputModule, MatRadioModule],
  template: `
    @if (!editing() && spaces().length > 1) {
      <span id="space-label" class="label">Whose bundle is it?</span>
      <mat-radio-group class="choices" aria-labelledby="space-label" [value]="space()" (change)="setSpace($event.value)">
        @for (option of spaces(); track option.namespace) {
          <mat-radio-button [value]="option.namespace">{{ option.label }}</mat-radio-button>
        }
      </mat-radio-group>
    }
    @if (!editing()) {
      <details class="technical" [open]="idProblem() !== null">
        <summary>Technical details</summary>
        <dl>
          <dt>Bundle ID</dt>
          <dd>
            <mat-form-field appearance="outline" subscriptSizing="dynamic">
              <input matInput aria-label="Bundle ID" [value]="bundleId()" (input)="setId($event)" />
            </mat-form-field>
          </dd>
        </dl>
      </details>
      @if (idProblem(); as problem) {
        <p class="problem" role="alert">{{ problem }}</p>
      }
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
    }
    .label {
      font: var(--mat-sys-title-small);
    }
    .choices {
      display: flex;
      flex-wrap: wrap;
      gap: 0.5rem 1.5rem;
    }
    p {
      margin: 0;
    }
  `
})
export class BundleIdentity {
  public readonly editing = input.required<boolean>();
  public readonly spaces = input.required<readonly { readonly namespace: string; readonly label: string }[]>();
  public readonly idProblem = input.required<string | null>();
  public readonly space = model.required<string>();
  public readonly bundleId = model.required<string>();

  protected setSpace(value: unknown): void {
    if (typeof value === "string") {
      this.space.set(value);
    }
  }

  protected setId(event: Event): void {
    this.bundleId.set(text(event).trim());
  }
}

/** The bundle's skills, and a search over every skill the person can see to add more. */
@Component({
  selector: "app-bundle-members",
  imports: [MatButtonModule, MatFormFieldModule, MatInputModule, Icon],
  template: `
    <h2>Skills in this bundle ({{ members().length }})</h2>
    <ul>
      @for (member of chosen(); track member.id) {
        <li>
          <span class="grow"
            >{{ member.name }} <span class="muted">{{ member.id }}</span></span
          >
          <button mat-icon-button type="button" [attr.aria-label]="'Remove ' + member.name" (click)="remove(member.id)"><app-icon name="close" /></button>
        </li>
      } @empty {
        <li class="muted">Add at least one skill below.</li>
      }
    </ul>
    <mat-form-field appearance="outline" subscriptSizing="dynamic">
      <mat-label>Find skills to add</mat-label>
      <app-icon matPrefix name="search" class="prefix" />
      <input matInput type="search" placeholder="Meeting notes, code review…" [value]="query()" (input)="query.set(text($event))" />
    </mat-form-field>
    <ul>
      @for (item of found(); track item.id) {
        <li>
          <span class="grow"
            >{{ item.name }} <span class="muted">{{ item.publisher.displayName }}</span></span
          >
          <button mat-button type="button" [disabled]="members().length >= maxMembers" (click)="add(item.id)"><app-icon name="add" />Add</button>
        </li>
      } @empty {
        <li class="muted">{{ query().trim().length > 0 ? "Nothing matches." : "Type to search every skill you can see." }}</li>
      }
    </ul>
  `,
  styles: `
    ul {
      list-style: none;
      margin: 0;
      padding: 0;
    }
    li {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      min-height: 2.5rem;
      border-bottom: var(--border);
    }
    .grow {
      flex: 1;
      overflow-wrap: anywhere;
    }
    .prefix {
      margin: 0 0.25rem 0 0.75rem;
      color: var(--muted);
    }
  `
})
export class BundleMembers {
  public readonly packages = input.required<readonly IndexPackage[]>();
  public readonly members = model.required<readonly string[]>();

  protected readonly maxMembers = maxMembers;
  protected readonly text = text;
  protected readonly query = signal("");

  protected readonly chosen = computed(() => this.members().map((id) => this.packages().find((item) => item.id === id) ?? { id, name: id }));

  protected readonly found = computed(() => {
    const words = this.query()
      .toLowerCase()
      .split(/\s+/u)
      .filter((word) => word.length > 0);
    if (words.length === 0) {
      return [];
    }

    return this.packages()
      .filter((item) => !this.members().includes(item.id))
      .filter((item) => {
        const haystack = [item.name, item.description, item.id, item.publisher.displayName, ...item.tags].join(" ").toLowerCase();
        return words.every((word) => haystack.includes(word));
      })
      .slice(0, 20);
  });

  protected add(id: string): void {
    this.members.update((members) => (members.includes(id) ? members : [...members, id]));
  }

  protected remove(id: string): void {
    this.members.update((members) => members.filter((member) => member !== id));
  }
}

/** Makes or changes a bundle: a name, a space, and skills that already exist (anyone's). */
@Component({
  selector: "app-bundle-edit",
  imports: [RouterLink, MatButtonModule, MatFormFieldModule, MatInputModule, MatProgressBarModule, Icon, BundleIdentity, BundleMembers],
  template: `
    <div class="page stack narrow">
      <header>
        <h1>{{ editing() ? "Edit bundle" : "New bundle" }}</h1>
        <p class="lead">Group skills, yours or anyone's, so people can install them together.</p>
      </header>

      @if (ready()) {
        <form class="stack" (submit)="save($event)">
          <section class="card stack">
            <mat-form-field appearance="outline">
              <mat-label>Name</mat-label>
              <input matInput maxlength="120" required placeholder="New starter kit" [value]="name()" (input)="name.set(text($event))" />
            </mat-form-field>
            <mat-form-field appearance="outline">
              <mat-label>What is it for? (optional)</mat-label>
              <textarea matInput rows="2" maxlength="1024" [value]="description()" (input)="description.set(text($event))"></textarea>
            </mat-form-field>
            <app-bundle-identity [editing]="editing()" [spaces]="spaces()" [idProblem]="idProblem()" [(space)]="space" [(bundleId)]="bundleId" />
          </section>

          <app-bundle-members class="card stack" [packages]="packages()" [(members)]="members" />

          @if (problem(); as failure) {
            <p class="problem" role="alert">{{ failure }}</p>
          }
          <div class="row">
            @if (editing()) {
              <button mat-button type="button" class="danger" [disabled]="saving()" (click)="remove()"><app-icon name="delete" />Delete bundle</button>
            }
            <span class="spacer"></span>
            <a mat-button [routerLink]="cancelLink()">Cancel</a>
            <button mat-flat-button type="submit" [disabled]="!canSave()"><app-icon name="check" />{{ saving() ? "Saving…" : "Save bundle" }}</button>
          </div>
        </form>
      } @else if (loadProblem(); as failure) {
        <p class="problem">{{ failure }}</p>
      } @else {
        <mat-progress-bar mode="indeterminate" aria-label="Loading" />
      }
    </div>
  `
})
export class BundleEditPage {
  public readonly ns = input<string>();
  public readonly id = input<string>();

  private readonly api = inject(Api);
  private readonly session = inject(Session);
  private readonly router = inject(Router);
  private readonly dialog = inject(MatDialog);
  private readonly snackBar = inject(MatSnackBar);

  protected readonly text = text;
  protected readonly editing = computed(() => this.id() !== undefined);

  protected readonly mine = resource({ loader: () => this.api.mine() });
  protected readonly index = resource({ loader: () => this.api.index() });
  protected readonly existing = resource({
    params: () => {
      const ns = this.ns();
      const id = this.id();
      return ns !== undefined && id !== undefined ? { ns, id } : undefined;
    },
    loader: ({ params }) => this.api.bundle(params.ns, params.id)
  });

  protected readonly name = signal("");
  protected readonly description = signal("");
  protected readonly members = signal<readonly string[]>([]);
  protected readonly space = linkedSignal(() => this.ns() ?? this.session.me()?.namespace ?? "");
  protected readonly bundleId = linkedSignal(() => this.id() ?? slugify(this.name()));
  protected readonly saving = signal(false);
  protected readonly problem = signal<string | null>(null);

  protected readonly ready = computed(() => this.mine.hasValue() && this.index.hasValue() && (!this.editing() || this.existing.hasValue()));
  protected readonly loadProblem = computed(() => {
    const error = this.mine.error() ?? this.index.error() ?? this.existing.error();
    return error === undefined ? null : ApiError.from(error).message;
  });

  protected readonly spaces = computed(() =>
    (this.mine.hasValue() ? this.mine.value().spaces : []).map((space) => ({ namespace: space.namespace, label: space.lane === "personal" ? "Mine" : space.displayName }))
  );

  protected readonly packages = computed<readonly IndexPackage[]>(() => (this.index.hasValue() ? this.index.value().packages : []));

  protected readonly idProblem = computed(() =>
    this.name().trim().length > 0 && !idPattern.test(this.bundleId()) ? "The bundle ID can only use lowercase letters, digits, and single hyphens. Set one under Technical details." : null
  );

  protected readonly canSave = computed(
    () => !this.saving() && this.name().trim().length > 0 && idPattern.test(this.bundleId()) && this.space() !== "" && this.members().length > 0 && this.members().length <= maxMembers
  );

  protected readonly cancelLink = computed(() => (this.editing() ? ["/b", this.ns() ?? "", this.id() ?? ""] : ["/browse"]));

  constructor() {
    // An edit starts from the saved bundle.
    effect(() => {
      if (this.existing.hasValue()) {
        const bundle = this.existing.value();
        this.name.set(bundle.name);
        this.description.set(bundle.description);
        this.members.set(bundle.members);
      }
    });
  }

  protected remove(): void {
    runTask(this.confirmDelete());
  }

  protected save(event: SubmitEvent): void {
    event.preventDefault();
    if (this.canSave()) {
      runTask(this.store());
    }
  }

  private async store(): Promise<void> {
    this.saving.set(true);
    this.problem.set(null);
    try {
      const bundle = await this.api.saveBundle(this.space(), this.bundleId(), { name: this.name().trim(), description: this.description().trim(), members: this.members() });
      this.snackBar.open("Bundle saved.", undefined, { duration: 4000 });
      await this.router.navigate(["/b", bundle.namespace, bundle.bundleId]);
    } catch (error) {
      this.problem.set(ApiError.from(error).message);
    } finally {
      this.saving.set(false);
    }
  }

  private async confirmDelete(): Promise<void> {
    const confirmed = await prompt(this.dialog, { title: "Delete this bundle?", message: "The skills in it stay where they are, and nobody's installs change.", confirm: "Delete", danger: true });
    const ns = this.ns();
    const id = this.id();
    if (confirmed === undefined || ns === undefined || id === undefined) {
      return;
    }

    try {
      await this.api.deleteBundle(ns, id);
      this.snackBar.open("Bundle deleted.", undefined, { duration: 4000 });
      await this.router.navigate(["/mine"]);
    } catch (error) {
      this.problem.set(ApiError.from(error).message);
    }
  }
}
