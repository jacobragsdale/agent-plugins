import { Component, input } from "@angular/core";
import { Icon } from "./icon";

/** Says who shared what, when a page was opened from a share link. */
@Component({
  selector: "app-shared-banner",
  imports: [Icon],
  template: `
    @if (by(); as name) {
      <p class="notice" role="status"><app-icon name="share" /> {{ name }} shared {{ what() }} with you. It's in Agent Plugins for you now.</p>
    }
  `,
  styles: `
    p {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      margin: 0;
    }
  `
})
export class SharedBanner {
  public readonly by = input.required<string | undefined>();
  public readonly what = input.required<string>();
}
