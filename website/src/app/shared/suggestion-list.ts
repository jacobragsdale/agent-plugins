import { Component, computed, input } from "@angular/core";
import { RouterLink } from "@angular/router";
import type { Suggestion, SuggestionState } from "../api";
import { formatAge } from "../format";

export const suggestionStates: Readonly<Record<SuggestionState, { readonly label: string; readonly badge: string }>> = {
  pending: { label: "Waiting for the owners", badge: "badge pending" },
  accepted: { label: "Published", badge: "badge live" },
  declined: { label: "Not accepted", badge: "badge rejected" },
  withdrawn: { label: "Withdrawn", badge: "badge" }
};

/** Suggested changes, newest first, each opening its own page. */
@Component({
  selector: "app-suggestion-list",
  imports: [RouterLink],
  template: `
    <ul>
      @for (row of rows(); track row.suggestion.id) {
        <li>
          <div class="row">
            <a [routerLink]="['/suggestions', row.suggestion.id]">{{ showPackage() ? row.suggestion.name : "Suggestion" }}</a>
            <span [class]="row.state.badge">{{ row.state.label }}</span>
            <span class="spacer"></span>
            <span class="muted">{{ row.suggestion.suggestedByName }} · {{ row.age }}</span>
          </div>
          <p class="muted message">{{ row.suggestion.message }}</p>
        </li>
      } @empty {
        <li class="muted">{{ empty() }}</li>
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
      border-bottom: var(--border);
      padding: 0.75rem 0;
    }
    a {
      font: var(--mat-sys-title-small);
    }
    .message {
      margin: 0.35rem 0 0;
      display: -webkit-box;
      -webkit-line-clamp: 2;
      -webkit-box-orient: vertical;
      overflow: hidden;
    }
  `
})
export class SuggestionList {
  public readonly suggestions = input.required<readonly Suggestion[]>();
  /** Name the package on each row, where the list spans several. */
  public readonly showPackage = input(false);
  public readonly empty = input("No suggestions.");

  protected readonly rows = computed(() =>
    this.suggestions()
      .toSorted((left, right) => Date.parse(right.suggestedAt) - Date.parse(left.suggestedAt))
      .map((suggestion) => ({ suggestion, state: suggestionStates[suggestion.state], age: formatAge(suggestion.suggestedAt) }))
  );
}
