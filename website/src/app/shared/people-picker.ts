import { Component, computed, inject, input, output, resource, signal } from "@angular/core";
import { MatAutocompleteModule } from "@angular/material/autocomplete";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { Api } from "../api";
import { Icon } from "./icon";

export type Picked = { readonly kind: "user"; readonly account: string; readonly displayName: string } | { readonly kind: "team"; readonly namespace: string; readonly displayName: string };

interface Option {
  readonly value: Picked;
  readonly label: string;
  readonly detail: string;
}

/** Finds people (and optionally teams) as you type; what you typed can be added as an account too. */
@Component({
  selector: "app-people-picker",
  imports: [MatAutocompleteModule, MatFormFieldModule, MatInputModule, Icon],
  template: `
    <mat-form-field appearance="outline" subscriptSizing="dynamic" class="full">
      <mat-label>{{ label() }}</mat-label>
      <app-icon matPrefix name="search" class="prefix" />
      <input matInput [placeholder]="teams() ? 'Name, account, or team' : 'Name or account'" [matAutocomplete]="auto" [value]="query()" (input)="setQuery($event)" />
      <mat-autocomplete #auto [displayWith]="blank" (optionSelected)="choose($event.option.value)">
        @for (option of options(); track option.label + option.detail) {
          <mat-option [value]="option.value">
            <span>{{ option.label }}</span>
            <span class="muted detail">{{ option.detail }}</span>
          </mat-option>
        }
      </mat-autocomplete>
    </mat-form-field>
  `,
  styles: `
    .full {
      width: 100%;
    }
    .prefix {
      margin: 0 0.25rem 0 0.75rem;
      color: var(--muted);
    }
    .detail {
      margin-left: 0.5rem;
      font: var(--mat-sys-body-small);
    }
  `
})
export class PeoplePicker {
  public readonly label = input.required<string>();
  public readonly teams = input(false);
  public readonly picked = output<Picked>();

  protected readonly query = signal("");
  private readonly api = inject(Api);

  private readonly found = resource({
    params: () => {
      const query = this.query().trim();
      return query.length > 0 ? { query } : undefined;
    },
    loader: ({ params }) => this.api.directory(params.query)
  });

  protected readonly options = computed<readonly Option[]>(() => {
    const typed = this.query().trim();
    const found = this.found.hasValue() ? this.found.value() : { people: [], teams: [] };
    const people = found.people.map((person) => ({ value: { kind: "user" as const, ...person }, label: person.displayName, detail: person.account }));
    const teams = this.teams() ? found.teams.map((team) => ({ value: { kind: "team" as const, ...team }, label: team.displayName, detail: "Team" })) : [];
    const exact = found.people.some((person) => person.account.toLowerCase() === typed.toLowerCase());
    // Someone who never signed in isn't in the directory yet; their Windows account still works. A CI pipeline's account has a prefix, as in github:owner/repo.
    const detail = typed.includes(":") ? "CI pipeline" : "Windows account";
    const raw = typed.length > 0 && typed.length <= 256 && !exact ? [{ value: { kind: "user" as const, account: typed, displayName: typed }, label: `Add “${typed}”`, detail }] : [];
    return [...people, ...teams, ...raw];
  });

  protected readonly blank = (): string => "";

  protected setQuery(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.query.set(event.target.value);
    }
  }

  protected choose(value: Picked): void {
    this.picked.emit(value);
    this.query.set("");
  }
}
