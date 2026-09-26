import { Component, computed, input } from "@angular/core";
import { RouterLink } from "@angular/router";
import type { IndexBundle } from "../api";
import { AccessBadge } from "./access-badge";
import { Icon } from "./icon";
import { InstallButton } from "./install-button";
import { LaneBadge } from "./lane-badge";

/** A bundle in a grid: its skills install together, from its page or here. */
@Component({
  selector: "app-bundle-card",
  imports: [RouterLink, AccessBadge, Icon, InstallButton, LaneBadge],
  template: `
    <div class="head">
      <h3>
        <a class="name" [routerLink]="link()"><app-icon name="layers" />{{ bundle().name }}</a>
      </h3>
      <app-access-badge [restricted]="bundle().restricted" [sharedWithYou]="bundle().sharedWithYou" />
    </div>
    @if (bundle().description.length > 0) {
      <p class="description">{{ bundle().description }}</p>
    }
    <div class="foot">
      <div class="meta">
        <app-lane-badge [lane]="bundle().lane" [publisher]="bundle().publisher.displayName" />
        <span>· {{ count() }}</span>
      </div>
      <app-install-button class="install" [target]="bundle().id" [covers]="bundle().members" label="Install all" [compact]="true" />
    </div>
  `,
  styles: `
    :host {
      position: relative;
      display: grid;
      align-content: space-between;
      gap: 0.375rem;
      padding: 1rem 1.125rem;
      background: var(--card);
      border: var(--border);
      border-radius: var(--radius);
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
    .name {
      display: inline-flex;
      align-items: center;
      gap: 0.4rem;
      color: inherit;
      text-decoration: none;
      app-icon {
        color: var(--muted);
      }
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
    }
    .foot {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 0.5rem;
      min-height: 36px;
    }
    .install {
      position: relative;
      z-index: 1;
    }
  `
})
export class BundleCard {
  public readonly bundle = input.required<IndexBundle>();

  protected readonly link = computed(() => ["/b", this.bundle().namespace, this.bundle().bundleId]);
  protected readonly count = computed(() => (this.bundle().members.length === 1 ? "1 skill" : `${String(this.bundle().members.length)} skills`));
}

/** The bundles that match, above the skills; nothing when there are none. */
@Component({
  selector: "app-bundle-grid",
  imports: [BundleCard],
  template: `
    @if (bundles().length > 0) {
      <section class="stack" aria-labelledby="bundles-heading">
        <h2 id="bundles-heading">
          Bundles <span class="count">{{ bundles().length }}</span>
        </h2>
        <div class="grid">
          @for (bundle of bundles(); track bundle.id) {
            <app-bundle-card [bundle]="bundle" />
          }
        </div>
      </section>
    }
  `,
  styles: `
    section {
      margin-top: 0.5rem;
    }
    .count {
      margin-left: 0.25rem;
      color: var(--muted);
      font-weight: 400;
    }
  `
})
export class BundleGrid {
  public readonly bundles = input.required<readonly IndexBundle[]>();
}
