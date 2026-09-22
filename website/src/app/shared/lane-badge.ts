import { Component, computed, input } from "@angular/core";
import type { Lane } from "../api";
import type { IconName } from "./icon";
import { Icon } from "./icon";

/** Who stands behind a package: the company (official), a team, or one person. */
@Component({ selector: "app-lane-badge", imports: [Icon], template: `<span [class]="classes()" [title]="title()"><app-icon [name]="icon()" />{{ publisher() }}</span>` })
export class LaneBadge {
  public readonly lane = input.required<Lane>();
  public readonly publisher = input.required<string>();

  protected readonly classes = computed(() => `badge ${this.lane()}`);

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
