import type { HttpInterceptorFn } from "@angular/common/http";
import { HttpErrorResponse } from "@angular/common/http";
import { Injectable, computed, inject, signal } from "@angular/core";
import type { CanMatchFn } from "@angular/router";
import { Router } from "@angular/router";
import { throwError } from "rxjs";
import type { Health, Me } from "./api";
import { Api, ApiError } from "./api";
import { runTask } from "./shared/tasks";

const devUserKey = "agent-plugins.dev-user";

/** What the server's X-Dev-User header accepts; anything else would break every request. */
export const devAccountPattern = /^[\x20-\x7E]{1,256}$/u;

/**
 * The account typed into the development sign-in. At work the browser signs in with Windows
 * automatically; only a server running with the development header offers this.
 */
@Injectable({ providedIn: "root" })
export class DevUser {
  public readonly account = signal(DevUser.read());
  /** Whether this server trusts the development header, so a signed-out browser here is off the domain. */
  public readonly offered = signal(false);

  public set(account: string | null): void {
    try {
      if (account === null) {
        localStorage.removeItem(devUserKey);
      } else {
        localStorage.setItem(devUserKey, account);
      }
    } catch {
      // Storage can be unavailable (private windows); the sign-in then lasts until reload.
    }

    this.account.set(account);
  }

  private static read(): string | null {
    try {
      const account = localStorage.getItem(devUserKey);
      return account !== null && devAccountPattern.test(account) ? account : null;
    } catch {
      return null;
    }
  }
}

export const devUserInterceptor: HttpInterceptorFn = (request, next) => {
  const devUser = inject(DevUser);
  const account = devUser.account();
  if (!request.url.startsWith("/api/")) {
    return next(request);
  }
  if (account !== null) {
    return next(request.clone({ setHeaders: { "X-Dev-User": account } }));
  }
  // Signed out where the server trusts the development header: the request would carry no credentials, and the
  // 401's Negotiate challenge makes a browser off the domain pop up a password box that can never succeed.
  if (devUser.offered() && !request.url.startsWith("/api/health")) {
    return throwError(() => new HttpErrorResponse({ status: 401, statusText: "Sign in first", url: request.url }));
  }
  return next(request);
};

export type SessionState =
  | { readonly kind: "loading" }
  | { readonly kind: "signed-in"; readonly me: Me }
  | { readonly kind: "signed-out"; readonly devSignIn: boolean }
  | { readonly kind: "unavailable"; readonly message: string };

@Injectable({ providedIn: "root" })
export class Session {
  public readonly state = signal<SessionState>({ kind: "loading" });
  public readonly health = signal<Health | null>(null);
  public readonly me = computed(() => {
    const state = this.state();
    return state.kind === "signed-in" ? state.me : null;
  });
  public readonly isAdmin = computed(() => this.me()?.admin === true);
  public readonly devSignedIn = computed(() => this.devUser.account() !== null);
  /** MCP servers waiting for an admin before everyone can see them, plus open reports; a badge on the Admin link. */
  public readonly pendingReviews = signal(0);

  private readonly api = inject(Api);
  private readonly devUser = inject(DevUser);
  private loading: Promise<SessionState> | null = null;

  constructor() {
    // Installing from the page happens in the desktop app; coming back shows what it installed.
    // The app's window usually only covers the browser, so the tab never hides: focus is the signal.
    window.addEventListener("focus", () => {
      runTask(this.refreshMe());
    });
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "visible") {
        runTask(this.refreshMe());
      }
    });
  }

  /** Re-reads who the caller is and what their app reports installed, without reloading the page. */
  public async refreshMe(): Promise<void> {
    if (this.state().kind !== "signed-in") {
      return;
    }

    try {
      this.state.set({ kind: "signed-in", me: await this.api.me() });
    } catch {
      // The next navigation or retry reports a real outage; a stale install state is harmless.
    }
  }

  /** Resolves once the caller's identity is known. */
  public ready(): Promise<SessionState> {
    this.loading ??= this.load();
    return this.loading;
  }

  /** A new account reloads the page, so no page keeps the previous account's data. */
  public signInAs(account: string): void {
    this.devUser.set(account);
    location.reload();
  }

  public signOut(): void {
    this.devUser.set(null);
    location.reload();
  }

  public retry(): Promise<SessionState> {
    this.loading = this.load();
    return this.loading;
  }

  public async refreshReviews(): Promise<void> {
    if (!this.isAdmin()) {
      this.pendingReviews.set(0);
      return;
    }

    try {
      const [reviews, summary] = await Promise.all([this.api.reviews(), this.api.summary()]);
      this.pendingReviews.set(reviews.length + summary.openReports);
    } catch {
      // The badge is a convenience; the admin page reports its own errors.
    }
  }

  private async load(): Promise<SessionState> {
    this.state.set({ kind: "loading" });
    const state = await this.resolve();
    this.state.set(state);
    if (state.kind === "unavailable") {
      // The next guard or page that waits on the session tries again.
      this.loading = null;
    }

    await this.refreshReviews();
    return state;
  }

  private async resolve(): Promise<SessionState> {
    let health: Health;
    try {
      health = await this.api.health();
      this.health.set(health);
      this.devUser.offered.set(health.authSchemes.includes("DevHeader"));
    } catch (error) {
      return { kind: "unavailable", message: ApiError.from(error).message };
    }

    // Without a development account the request would carry no credentials, and the 401's Negotiate
    // challenge makes a browser off the domain pop up a password box that can never succeed.
    if (health.authSchemes.includes("DevHeader") && this.devUser.account() === null) {
      return { kind: "signed-out", devSignIn: true };
    }

    try {
      return { kind: "signed-in", me: await this.api.me() };
    } catch (error) {
      const failure = ApiError.from(error);
      return failure.status === 401 ? { kind: "signed-out", devSignIn: health.authSchemes.includes("DevHeader") } : { kind: "unavailable", message: failure.message };
    }
  }
}

/** Admin pages load only for admins; everyone else lands on the home page. */
export const adminOnly: CanMatchFn = async () => {
  // inject() works only before the first await.
  const session = inject(Session);
  const router = inject(Router);
  const state = await session.ready();
  return state.kind === "signed-in" && state.me.admin ? true : router.createUrlTree(["/"]);
};
