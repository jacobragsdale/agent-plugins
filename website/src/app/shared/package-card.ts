import { Component, computed, input } from "@angular/core";
import { RouterLink } from "@angular/router";
import type { IndexPackage } from "../api";
import { worksInNote } from "../format";
import { AccessBadge } from "./access-badge";
import { InstallButton } from "./install-button";
import { LaneBadge } from "./lane-badge";

@Component({
  selector: "app-package-card",
  imports: [RouterLink, AccessBadge, InstallButton, LaneBadge],
  template: `
    <div class="head">
      <h3>
        <a class="name" [routerLink]="link()">{{ item().name }}</a>
      </h3>
      <app-access-badge [restricted]="item().restricted" [sharedWithYou]="item().sharedWithYou" />
    </div>
    <p class="description">{{ item().description }}</p>
    <div class="foot">
      <div class="meta">
        <app-lane-badge [lane]="item().lane" [publisher]="item().publisher.displayName" />
        @if (item().installedBase > 0) {
          <span>· {{ item().installedBase }} using it</span>
        }
        @if (hasServer()) {
          <span class="badge" title="Lets the assistant use a tool or service">MCP server</span>
        }
        @if (note(); as missing) {
          <span class="badge">{{ missing }}</span>
        }
      </div>
      @if (install()) {
        <app-install-button class="install" [target]="item().id" label="Install" [compact]="true" />
      }
    </div>
  `,
  styles: `
    :host {
      position: relative;
      display: grid;
      grid-template-rows: auto 1fr auto;
      gap: 0.375rem;
      padding: 1rem 1.125rem;
      background: var(--card);
      border: var(--border);
      border-radius: var(--radius);
      transition: border-color 120ms;
      &:hover,
      &:focus-within {
        border-color: var(--mat-sys-outline);
      }
    }
    .head {
      display: flex;
      align-items: baseline;
      justify-content: space-between;
      gap: 0.5rem;
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
    :host:has(.name:focus-visible) {
      outline: 2px solid var(--mat-sys-primary);
      outline-offset: 2px;
    }
    .description {
      margin: 0;
      color: var(--muted);
      font: var(--mat-sys-body-medium);
      display: -webkit-box;
      -webkit-line-clamp: 2;
      -webkit-box-orient: vertical;
      overflow: hidden;
    }
    .foot {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 0.5rem;
      min-height: 36px;
      margin-top: 0.375rem;
    }
    .install {
      position: relative;
      z-index: 1;
    }
  `
})
export class PackageCard {
  public readonly item = input.required<IndexPackage>();
  public readonly install = input(true);

  protected readonly link = computed(() => ["/p", this.item().namespace, this.item().packageId]);
  protected readonly hasServer = computed(() => this.item().componentKinds.includes("mcpServer"));
  protected readonly note = computed(() => worksInNote(this.item().componentKinds, this.item().mcpTransports));
}

/** "Skill", "MCP server", or "Skill + MCP server", in words non-developers know. */
export function describeKinds(kinds: readonly string[]): string {
  const words = kinds.map((kind) => (kind === "mcpServer" ? "MCP server" : kind === "skill" ? "Skill" : kind));
  return words.length === 0 ? "Package" : words.join(" + ");
}
