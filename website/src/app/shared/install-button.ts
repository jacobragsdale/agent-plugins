import { Component, DestroyRef, computed, inject, input, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { RouterLink } from "@angular/router";
import { appLink, installState } from "../format";
import { Session } from "../session";
import { Icon } from "./icon";
import { runTask } from "./tasks";

/**
 * Installs through the desktop app: the button opens an `agent-plugins://` link, and the app asks
 * before it changes anything. What it offers follows the app's last check-in (`/api/me.app`).
 */
@Component({
  selector: "app-install-button",
  imports: [RouterLink, MatButtonModule, Icon],
  template: `
    <div class="row">
      @if (state() === "installed") {
        <span class="badge live"><app-icon name="check" />Installed</span>
        @if (!compact()) {
          <button mat-button type="button" (click)="open('open')">Open in Agent Plugins</button>
        }
      } @else if (needsApp(); as app) {
        <a mat-flat-button routerLink="/" fragment="download"><app-icon name="download" />{{ app.action }}</a>
        <button mat-button type="button" (click)="open('install')">{{ app.anyway }}</button>
      } @else if (state() === "install") {
        <button mat-flat-button type="button" (click)="open('install')"><app-icon name="download" />{{ label() }}</button>
      }
    </div>
    @if (opening()) {
      <p class="muted hint" role="status">Opening Agent Plugins… Nothing happened? <a routerLink="/" fragment="download">Get the app</a></p>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 0.4rem;
    }
    .row {
      gap: 0.5rem;
    }
    .hint {
      margin: 0;
      font: var(--mat-sys-body-small);
    }
  `
})
export class InstallButton {
  /** `ns/pkg`, `ns/bundle`, or `ns/pkg/skill` for one skill of a pack. */
  public readonly target = input.required<string>();
  /** Packages that must all be installed to show Installed; defaults to the target itself. */
  public readonly covers = input<readonly string[]>();
  public readonly label = input("Install in Agent Plugins");
  /** On a card: just Install or Installed, and nothing when the app is missing or too old. */
  public readonly compact = input(false);

  private readonly session = inject(Session);
  protected readonly opening = signal(false);
  private timer: ReturnType<typeof setTimeout> | undefined;
  private poll: ReturnType<typeof setInterval> | undefined;

  /** What must be installed to count as done; one skill of a pack is not tracked. */
  private readonly watched = computed(() => this.covers() ?? (this.target().split("/").length === 2 ? [this.target()] : []));

  protected readonly state = computed(() => {
    const me = this.session.me();
    return installState(me === null ? undefined : me.app, this.watched());
  });

  constructor() {
    inject(DestroyRef).onDestroy(() => {
      clearTimeout(this.timer);
      clearInterval(this.poll);
    });
  }

  /** Getting or updating the app comes first; a card leaves that to the skill's page. */
  protected readonly needsApp = computed(() => {
    if (this.compact()) {
      return null;
    }

    switch (this.state()) {
      case "get":
        return { action: "Get Agent Plugins", anyway: "Already have it? Open it" };
      case "update":
        return { action: "Update Agent Plugins", anyway: "Open it anyway" };
      case "install":
      case "installed":
        return null;
    }
  });

  protected open(verb: "install" | "open"): void {
    location.href = appLink(verb, this.target());
    this.opening.set(true);
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.opening.set(false);
    }, 8000);
    if (verb === "install" && this.watched().length > 0) {
      this.watch();
    }
  }

  /**
   * The app reports an install a few seconds after it finishes, which can be after this window gets focus
   * back, so keep asking for up to a minute until it shows as installed.
   */
  private watch(): void {
    clearInterval(this.poll);
    const until = Date.now() + 60_000;
    this.poll = setInterval(() => {
      if (this.state() === "installed" || Date.now() > until) {
        clearInterval(this.poll);
        return;
      }
      runTask(this.session.refreshMe());
    }, 3000);
  }
}
