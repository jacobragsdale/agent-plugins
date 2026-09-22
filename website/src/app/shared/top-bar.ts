import { NgOptimizedImage } from "@angular/common";
import { Component, inject } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import { RouterLink, RouterLinkActive } from "@angular/router";
import { Session } from "../session";
import { runTask } from "./tasks";
import { Icon } from "./icon";

/** The signed-in person, the main action, and (development only) sign-out. */
@Component({
  selector: "app-account",
  imports: [RouterLink, MatButtonModule, Icon],
  template: `
    @if (session.me(); as me) {
      <a mat-flat-button routerLink="/publish"><app-icon name="add" />Share a skill</a>
      <span class="who" [title]="me.account">
        <app-icon name="person" />
        <span class="who-name">{{ me.displayName }}</span>
      </span>
      @if (session.devSignedIn()) {
        <button mat-button type="button" (click)="signOut()">Sign out</button>
      }
    }
  `,
  styles: `
    :host {
      display: flex;
      align-items: center;
      gap: 1rem;
    }
    .who {
      display: inline-flex;
      align-items: center;
      gap: 0.35rem;
      color: var(--muted);
      font: var(--mat-sys-label-large);
    }
    @media (max-width: 720px) {
      .who-name {
        display: none;
      }
    }
  `
})
export class Account {
  protected readonly session = inject(Session);

  protected signOut(): void {
    runTask(this.session.signOut());
  }
}

@Component({
  selector: "app-top-bar",
  imports: [NgOptimizedImage, RouterLink, RouterLinkActive, MatProgressBarModule, Account],
  template: `
    <div class="bar">
      <a class="brand" routerLink="/">
        <img ngSrc="icon.svg" alt="" width="32" height="32" priority />
        <span>Agent Plugins</span>
      </a>
      <nav aria-label="Main">
        <a routerLink="/browse" routerLinkActive="active">Browse</a>
        @if (session.me()) {
          <a routerLink="/mine" routerLinkActive="active">My skills</a>
        }
        @if (session.isAdmin()) {
          <a routerLink="/admin" routerLinkActive="active" class="with-count">
            Admin
            @if (session.pendingReviews() > 0) {
              <span class="count" [attr.aria-label]="session.pendingReviews() + ' waiting for review'">{{ session.pendingReviews() }}</span>
            }
          </a>
        }
        <a routerLink="/help" routerLinkActive="active">Help</a>
      </nav>
      <span class="spacer"></span>
      <app-account />
    </div>
    @if (session.state().kind === "loading") {
      <mat-progress-bar mode="indeterminate" aria-label="Signing in" />
    }
  `,
  styleUrl: "./top-bar.scss"
})
export class TopBar {
  protected readonly session = inject(Session);
}
