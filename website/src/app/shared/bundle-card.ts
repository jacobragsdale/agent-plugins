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
    <article class="card item">
      <div class="row head">
        <h3>
          <a class="name" [routerLink]="link()"><app-icon name="layers" />{{ bundle().name }}</a>
        </h3>
        <app-access-badge [restricted]="bundle().restricted" [sharedWithYou]="bundle().sharedWithYou" />
      </div>
      @if (bundle().description.length > 0) {
        <p class="description">{{ bundle().description }}</p>
      }
      <div class="row meta">
        <app-lane-badge [lane]="bundle().lane" [publisher]="bundle().publisher.displayName" />
        <span class="muted">{{ count() }}</span>
      </div>
      <app-install-button class="install" [target]="bundle().id" [covers]="bundle().members" label="Install all" [compact]="true" />
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
      border-style: dashed;
      &:hover,
      &:focus-within {
        border-color: var(--mat-sys-primary);
      }
    }
    .head {
      justify-content: space-between;
      flex-wrap: nowrap;
      h3 {
        margin: 0;
      }
    }
    .name {
      display: inline-flex;
      align-items: center;
      gap: 0.4rem;
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
    .description {
      margin: 0;
      color: var(--muted);
    }
    .meta {
      gap: 0.4rem;
      font: var(--mat-sys-body-small);
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
        <h2 id="bundles-heading">Bundles</h2>
        <div class="grid">
          @for (bundle of bundles(); track bundle.id) {
            <app-bundle-card [bundle]="bundle" />
          }
        </div>
      </section>
      <h2>Skills</h2>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 1rem;
    }
    h2 {
      font: var(--mat-sys-title-large);
      margin: 0.5rem 0 0;
    }
    .grid {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
      gap: 1rem;
    }
  `
})
export class BundleGrid {
  public readonly bundles = input.required<readonly IndexBundle[]>();
}
