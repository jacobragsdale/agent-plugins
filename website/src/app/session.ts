import type { HttpInterceptorFn } from "@angular/common/http";
import { Injectable, computed, inject, signal } from "@angular/core";
import type { CanMatchFn } from "@angular/router";
import { Router } from "@angular/router";
import type { Health, Me } from "./api";
import { Api, ApiError } from "./api";

const devUserKey = "agent-plugins.dev-user";

/**
 * The account typed into the development sign-in. At work the browser signs in with Windows
 * automatically; only a server running with the development header offers this.
 */
@Injectable({ providedIn: "root" })
export class DevUser {
  public readonly account = signal(DevUser.read());

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
      return localStorage.getItem(devUserKey);
    } catch {
      return null;
    }
  }
}

export const devUserInterceptor: HttpInterceptorFn = (request, next) => {
  const account = inject(DevUser).account();
  return account !== null && request.url.startsWith("/api/") ? next(request.clone({ setHeaders: { "X-Dev-User": account } })) : next(request);
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
  /** Versions waiting for an admin; shown as a badge on the Admin link. */
  public readonly pendingReviews = signal(0);

  private readonly api = inject(Api);
  private readonly devUser = inject(DevUser);
  private loading: Promise<SessionState> | null = null;

  /** Resolves once the caller's identity is known. */
  public ready(): Promise<SessionState> {
    this.loading ??= this.load();
    return this.loading;
  }

  public signInAs(account: string): Promise<SessionState> {
    this.devUser.set(account.trim());
    this.loading = this.load();
    return this.loading;
  }

  public signOut(): Promise<SessionState> {
    this.devUser.set(null);
    this.loading = this.load();
    return this.loading;
  }

  public async refreshReviews(): Promise<void> {
    if (!this.isAdmin()) {
      this.pendingReviews.set(0);
      return;
    }

    try {
      this.pendingReviews.set((await this.api.reviews()).length);
    } catch {
      // The badge is a convenience; the admin page reports its own errors.
    }
  }

  private async load(): Promise<SessionState> {
    this.state.set({ kind: "loading" });
    const state = await this.resolve();
    this.state.set(state);
    await this.refreshReviews();
    return state;
  }

  private async resolve(): Promise<SessionState> {
    let health: Health;
    try {
      health = await this.api.health();
      this.health.set(health);
    } catch (error) {
      return { kind: "unavailable", message: ApiError.from(error).message };
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
  const session = inject(Session);
  const state = await session.ready();
  return state.kind === "signed-in" && state.me.admin ? true : inject(Router).createUrlTree(["/"]);
};
