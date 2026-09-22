import { Component, computed, input } from "@angular/core";
import { RouterLink } from "@angular/router";
import type { IndexPackage } from "../api";
import { Icon } from "./icon";
import { LaneBadge } from "./lane-badge";

@Component({
  selector: "app-package-card",
  imports: [RouterLink, Icon, LaneBadge],
  template: `
    <a class="card item" [routerLink]="link()">
      <div class="row head">
        <h3>{{ item().name }}</h3>
        @if (item().restricted) {
          <span class="badge" title="Only some people can see this"><app-icon name="lock" />Limited</span>
        }
      </div>
      <p class="description">{{ item().description }}</p>
      <div class="row meta">
        <app-lane-badge [lane]="item().lane" [publisher]="item().publisher.displayName" />
        <span class="kind">{{ kind() }}</span>
      </div>
      <div class="row foot muted">
        <span>v{{ item().version }}</span>
        <span>·</span>
        <span>{{ age() }}</span>
        @if (item().installedBase > 0) {
          <span>·</span>
          <span>{{ item().installedBase }} using it</span>
        }
      </div>
    </a>
  `,
  styles: `
    .item {
      display: grid;
      gap: 0.6rem;
      height: 100%;
      color: inherit;
      text-decoration: none;
      transition:
        border-color 120ms,
        box-shadow 120ms,
        transform 120ms;
      &:hover,
      &:focus-visible {
        border-color: var(--mat-sys-primary);
        box-shadow: 0 8px 24px color-mix(in srgb, var(--mat-sys-shadow) 12%, transparent);
        transform: translateY(-1px);
      }
    }
    .head {
      justify-content: space-between;
      flex-wrap: nowrap;
      h3 {
        margin: 0;
      }
    }
    .description {
      margin: 0;
      color: var(--muted);
      display: -webkit-box;
      -webkit-line-clamp: 3;
      -webkit-box-orient: vertical;
      overflow: hidden;
    }
    .meta,
    .foot {
      gap: 0.4rem;
      font: var(--mat-sys-body-small);
    }
    .kind {
      color: var(--muted);
    }
    .foot {
      margin-top: auto;
    }
  `
})
export class PackageCard {
  public readonly item = input.required<IndexPackage>();
  public readonly age = input.required<string>();

  protected readonly link = computed(() => ["/p", this.item().namespace, this.item().packageId]);
  protected readonly kind = computed(() => describeKinds(this.item().componentKinds));
}

/** "Skill", "MCP server", or "Skill + MCP server", in words non-developers know. */
export function describeKinds(kinds: readonly string[]): string {
  const words = kinds.map((kind) => (kind === "mcpServer" ? "MCP server" : kind === "skill" ? "Skill" : kind));
  return words.length === 0 ? "Package" : words.join(" + ");
}
