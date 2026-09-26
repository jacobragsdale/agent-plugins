import { Component, input } from "@angular/core";
import { Icon } from "./icon";

/** Who can see something: shared with you, private, or (where it helps) public. */
@Component({
  selector: "app-access-badge",
  imports: [Icon],
  template: `
    @if (sharedWithYou()) {
      <span class="badge team" title="Someone shared this with you"><app-icon name="share" />Shared with you</span>
    } @else if (restricted()) {
      <span class="badge" title="Only some people can see this"><app-icon name="lock" />Private</span>
    } @else if (showPublic()) {
      <span class="badge" title="Everyone who signs in can see this"><app-icon name="visibility" />Public</span>
    }
  `
})
export class AccessBadge {
  public readonly restricted = input.required<boolean>();
  public readonly sharedWithYou = input(false);
  public readonly showPublic = input(false);
}
