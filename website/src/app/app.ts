import { HttpClient } from "@angular/common/http";
import { Component, inject, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { MatSnackBar } from "@angular/material/snack-bar";
import { RouterLink, RouterOutlet } from "@angular/router";
import { firstValueFrom } from "rxjs";
import { ApiError } from "./api";
import { devAccountPattern, DevUser, Session } from "./session";
import { runTask } from "./shared/tasks";
import { TopBar } from "./shared/top-bar";

@Component({
  selector: "app-root",
  imports: [RouterOutlet, RouterLink, MatButtonModule, MatFormFieldModule, MatInputModule, TopBar],
  templateUrl: "./app.html",
  styleUrl: "./app.scss",
  host: { "(document:click)": "downloadWithDevHeader($event)" }
})
export class App {
  protected readonly session = inject(Session);
  private readonly devUser = inject(DevUser);
  private readonly http = inject(HttpClient);
  private readonly snackBar = inject(MatSnackBar);
  protected readonly devAccount = signal("");
  protected readonly devAccountProblem = signal("");

  constructor() {
    runTask(this.session.ready());
  }

  protected signIn(event: SubmitEvent): void {
    event.preventDefault();
    const account = this.devAccount().trim();
    if (devAccountPattern.test(account)) {
      this.session.signInAs(account);
    } else {
      this.devAccountProblem.set("Use up to 256 plain ASCII characters, like CORP\\jane.");
    }
  }

  protected retry(): void {
    runTask(this.session.retry());
  }

  /** The page has <base href="/">, so a bare #content link would navigate to the home page. */
  protected skipToContent(event: Event): void {
    event.preventDefault();
    document.getElementById("content")?.focus();
  }

  protected setDevAccount(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.devAccount.set(event.target.value);
    }
  }

  /**
   * A plain download link can't carry the development header, so off the domain the server answers 401 and the
   * browser asks for a Windows password that can never work. Signed in that way, API downloads are fetched with
   * the header and saved instead.
   */
  protected downloadWithDevHeader(event: MouseEvent): void {
    const link = event.target instanceof Element ? event.target.closest("a[href^='/api/']") : null;
    const href = link?.getAttribute("href");
    if (href === null || href === undefined || this.devUser.account() === null || event.defaultPrevented || event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey) {
      return;
    }

    event.preventDefault();
    runTask(this.save(href));
  }

  private async save(href: string): Promise<void> {
    try {
      const response = await firstValueFrom(this.http.get(href, { observe: "response", responseType: "blob" }));
      const name = /filename="?([^";]+)"?/u.exec(response.headers.get("Content-Disposition") ?? "")?.[1] ?? href.split(/[/?]/u).filter(Boolean).pop() ?? "download";
      const url = URL.createObjectURL(response.body ?? new Blob());
      const save = document.createElement("a");
      save.href = url;
      save.download = name;
      save.click();
      setTimeout(() => {
        URL.revokeObjectURL(url);
      }, 60_000);
    } catch (error) {
      this.snackBar.open(ApiError.from(error).message, "Dismiss");
    }
  }
}
