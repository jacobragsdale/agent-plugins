import { Component, inject, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { RouterLink, RouterOutlet } from "@angular/router";
import { devAccountPattern, Session } from "./session";
import { runTask } from "./shared/tasks";
import { TopBar } from "./shared/top-bar";

@Component({ selector: "app-root", imports: [RouterOutlet, RouterLink, MatButtonModule, MatFormFieldModule, MatInputModule, TopBar], templateUrl: "./app.html", styleUrl: "./app.scss" })
export class App {
  protected readonly session = inject(Session);
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
}
