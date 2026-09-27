import { HttpClient, provideHttpClient, withInterceptors } from "@angular/common/http";
import { HttpTestingController, provideHttpClientTesting } from "@angular/common/http/testing";
import { TestBed } from "@angular/core/testing";
import { firstValueFrom } from "rxjs";
import { DevUser, devUserInterceptor } from "./session";

describe("devUserInterceptor", () => {
  function setup(): { http: HttpClient; backend: HttpTestingController; devUser: DevUser } {
    TestBed.configureTestingModule({ providers: [provideHttpClient(withInterceptors([devUserInterceptor])), provideHttpClientTesting()] });
    return { http: TestBed.inject(HttpClient), backend: TestBed.inject(HttpTestingController), devUser: TestBed.inject(DevUser) };
  }

  it("keeps a signed-out request off a development-header server, so no password box opens", async () => {
    const { http, backend, devUser } = setup();
    devUser.set(null);
    devUser.offered.set(true);
    await expect(firstValueFrom(http.get("/api/notifications"))).rejects.toMatchObject({ status: 401, error: { title: "Sign in above to see this." } });
    backend.expectNone("/api/notifications");
    const health = firstValueFrom(http.get("/api/health"));
    backend.expectOne("/api/health").flush({});
    await health;
  });

  it("sends a signed-in request with the development header", () => {
    const { http, backend, devUser } = setup();
    devUser.set("CORP\\jane");
    devUser.offered.set(true);
    http.get("/api/me").subscribe();
    expect(backend.expectOne("/api/me").request.headers.get("X-Dev-User")).toBe("CORP\\jane");
  });
});
