import { Component, computed, input } from "@angular/core";
import type { Lane } from "../api";
import type { IconName } from "./icon";
import { Icon } from "./icon";

/** Who stands behind a package, as a byline: the company (official, with a check), a team, or one person. */
@Component({
  selector: "app-lane-badge",
  imports: [Icon],
  template: `<app-icon [name]="icon()" /><span class="visually-hidden">{{ word() }}: </span>{{ publisher() }}`,
  styles: `
    :host {
      display: inline-flex;
      align-items: center;
      gap: 0.3rem;
      color: var(--muted);
    }
    app-icon {
      font-size: 0.95em;
    }
    :host(.official) {
      color: var(--mat-sys-on-surface);
      app-icon {
        color: var(--mat-sys-primary);
      }
    }
  `,
  host: { "[class]": "lane()", "[title]": "title()" }
})
export class LaneBadge {
  public readonly lane = input.required<Lane>();
  public readonly publisher = input.required<string>();

  protected readonly icon = computed<IconName>(() => {
    switch (this.lane()) {
      case "official":
        return "verified";
      case "team":
        return "group";
      case "personal":
        return "person";
    }
  });

  /** The lane for screen readers, which skip the icon and rarely read the title. */
  protected readonly word = computed(() => {
    switch (this.lane()) {
      case "official":
        return "Official";
      case "team":
        return "Team";
      case "personal":
        return "Colleague";
    }
  });

  protected readonly title = computed(() => {
    switch (this.lane()) {
      case "official":
        return "Official: published by the company";
      case "team":
        return "Published by a team";
      case "personal":
        return "Published by a colleague";
    }
  });
}
