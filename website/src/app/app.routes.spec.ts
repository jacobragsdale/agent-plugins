import { TestBed } from "@angular/core/testing";
import { provideRouter, Router } from "@angular/router";
import { routes } from "./app.routes";

describe("help routes", () => {
  it("redirects only the bare /help", async () => {
    TestBed.configureTestingModule({ providers: [provideRouter(routes)] });
    const router = TestBed.inject(Router);
    for (const [url, landed] of [
      ["/help", "/help/publish"],
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
