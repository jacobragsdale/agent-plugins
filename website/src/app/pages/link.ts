import { Component, inject, input, resource, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { Router, RouterLink } from "@angular/router";
import type { LinkPreview, LinkResult } from "../api";
import { Api, ApiError } from "../api";
import { Session } from "../session";
import { Icon } from "../shared/icon";
import { runTask } from "../shared/tasks";

const deadLink = "This link no longer works. Ask the person who sent it for a new one.";

/** Where a shared link lands. The shared thing's page, with the banner, is its address. */
function destination(result: LinkResult): { readonly commands: readonly string[]; readonly queryParams: Readonly<Record<string, string>> } {
  const [ns = "", id = ""] = result.target.split("/");
  switch (result.targetKind) {
    case "package":
      return { commands: ["/p", ns, id], queryParams: { shared: result.by } };
    case "bundle":
      return { commands: ["/b", ns, id], queryParams: { shared: result.by } };
    case "space":
      return { commands: ["/browse"], queryParams: { space: ns, shared: result.by } };
    case "team":
      return { commands: ["/teams", ns], queryParams: {} };
  }
}

/**
 * Opens a link someone sent. A share link takes effect on arrival: being given access costs nothing,
 * and installing is still a separate choice. A team invite asks first, since joining makes you a
 * publisher that other members see.
 */
@Component({
  selector: "app-link",
  imports: [RouterLink, MatButtonModule, MatProgressBarModule, Icon],
  template: `
    <div class="page narrow">
      @if (preview.hasValue()) {
        @let link = preview.value();
        @if (link.kind === "invite") {
          <div class="card stack">
            <h1>Join {{ link.name }}?</h1>
            <p class="lead">{{ link.by }} invited you. You'll be able to publish skills to the team and see its private skills, and members will see you on the team.</p>
            @if (problem(); as message) {
              <p class="problem" role="alert">{{ message }}</p>
            }
            <div class="row">
              <button mat-flat-button type="button" [disabled]="joining()" (click)="join()"><app-icon name="group" />Join {{ link.name }}</button>
              <a mat-button routerLink="/">Not now</a>
            </div>
          </div>
        } @else {
          <mat-progress-bar mode="indeterminate" aria-label="Opening what was shared" />
        }
      } @else if (preview.error(); as error) {
        <div class="card">
          <h1>Link not working</h1>
          <p class="muted">{{ message(error) }}</p>
          <a mat-stroked-button routerLink="/browse">Browse skills</a>
        </div>
      } @else {
        <mat-progress-bar mode="indeterminate" aria-label="Opening the link" />
      }
    </div>
  `
})
export class LinkPage {
  public readonly code = input.required<string>();

  private readonly api = inject(Api);
  private readonly session = inject(Session);
  private readonly router = inject(Router);

  protected readonly joining = signal(false);
  protected readonly problem = signal<string | null>(null);

  /** A share link is redeemed right here; an invite waits for Join. */
  protected readonly preview = resource({
    params: () => ({ code: this.code() }),
    loader: async ({ params }): Promise<LinkPreview> => {
      await this.session.ready();
      const preview = await this.api.linkPreview(params.code);
      if (preview.kind === "share") {
        await this.go(await this.api.redeemLink(params.code));
      }

      return preview;
    }
  });

  protected join(): void {
    runTask(this.redeem());
  }

  protected message(error: unknown): string {
    const failure = ApiError.from(error);
    return failure.status === 404 ? deadLink : failure.message;
  }

  private async redeem(): Promise<void> {
    this.joining.set(true);
    this.problem.set(null);
    try {
      const result = await this.api.redeemLink(this.code());
      await this.session.refreshMe();
      await this.go(result);
    } catch (error) {
      this.problem.set(this.message(error));
    } finally {
      this.joining.set(false);
    }
  }

  private async go(result: LinkResult): Promise<void> {
    const { commands, queryParams } = destination(result);
    await this.router.navigate(commands, { queryParams, replaceUrl: true });
  }
}
