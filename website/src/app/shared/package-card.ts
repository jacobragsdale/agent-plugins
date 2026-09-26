import { Component, computed, input } from "@angular/core";
import { RouterLink } from "@angular/router";
import type { IndexPackage } from "../api";
import { AccessBadge } from "./access-badge";
import { InstallButton } from "./install-button";
import { LaneBadge } from "./lane-badge";

@Component({
  selector: "app-package-card",
  imports: [RouterLink, AccessBadge, InstallButton, LaneBadge],
  template: `
    <article class="card item">
      <div class="row head">
        <h3>
          <a class="name" [routerLink]="link()">{{ item().name }}</a>
        </h3>
        <app-access-badge [restricted]="item().restricted" [sharedWithYou]="item().sharedWithYou" />
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
      @if (install()) {
        <app-install-button class="install" [target]="item().id" label="Install" [compact]="true" />
      }
    </article>
  `,
  styles: `
    :host {
      display: block;
    }
    .item {
      position: relative;
      display: grid;
      gap: 0.6rem;
      height: 100%;
      transition:
        border-color 120ms,
        box-shadow 120ms,
        transform 120ms;
      &:hover,
      &:focus-within {
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
    /* The name's link covers the card, so the whole card opens the skill; the install button sits above it. */
    .name {
      color: inherit;
      text-decoration: none;
      &::after {
        content: "";
        position: absolute;
        inset: 0;
        border-radius: inherit;
      }
      &:focus-visible {
        outline: none;
      }
    }
    .install {
      position: relative;
      z-index: 1;
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
  public readonly install = input(true);

  protected readonly link = computed(() => ["/p", this.item().namespace, this.item().packageId]);
  protected readonly kind = computed(() => describeKinds(this.item().componentKinds));
}

/** "Skill", "MCP server", or "Skill + MCP server", in words non-developers know. */
export function describeKinds(kinds: readonly string[]): string {
  const words = kinds.map((kind) => (kind === "mcpServer" ? "MCP server" : kind === "skill" ? "Skill" : kind));
  return words.length === 0 ? "Package" : words.join(" + ");
}
