import { Component, inject } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { RouterLink } from "@angular/router";
import { Session } from "../session";

@Component({
  selector: "app-not-found",
  imports: [RouterLink, MatButtonModule],
  template: `
    <div class="page">
      <div class="card">
        <h1>Page not found</h1>
        <p class="muted">The link may be old, or the page moved.</p>
        <div class="row">
          <a mat-flat-button routerLink="/browse">Browse skills</a>
          @if (session.me()) {
            <a mat-button routerLink="/mine">My skills</a>
          }
        </div>
      </div>
    </div>
  `
})
export class NotFoundPage {
  protected readonly session = inject(Session);
}
