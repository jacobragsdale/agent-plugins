import { TestBed } from "@angular/core/testing";
import { provideRouter, Router } from "@angular/router";
import { routes } from "./app.routes";
import type { SessionState } from "./session";
import { Session } from "./session";

describe("help routes", () => {
  it("redirects only the bare /help", async () => {
    TestBed.configureTestingModule({ providers: [provideRouter(routes)] });
    const router = TestBed.inject(Router);
    for (const [url, landed] of [
      ["/help", "/help/getting-started"],
      ["/help/source-manifest", "/help/source-manifest"],
      ["/help/source-repository", "/help/source-repository"],
      ["/", "/"],
      ["/help/publish", "/help/publish"]
    ] as const) {
      await router.navigateByUrl(url);
      expect(router.url).toBe(landed);
    }
  });
});

describe("admin route", () => {
  it("sends everyone else to the home page", async () => {
    const signedOut: SessionState = { kind: "signed-out", devSignIn: false };
    TestBed.configureTestingModule({ providers: [provideRouter(routes), { provide: Session, useValue: { ready: () => Promise.resolve(signedOut) } }] });
    const router = TestBed.inject(Router);
    await router.navigateByUrl("/admin");
    expect(router.url).toBe("/");
  });
});
