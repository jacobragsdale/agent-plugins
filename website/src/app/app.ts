import { Component, inject, signal } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { MatFormFieldModule } from "@angular/material/form-field";
import { MatInputModule } from "@angular/material/input";
import { RouterLink, RouterOutlet } from "@angular/router";
import { Session } from "./session";
import { runTask } from "./shared/tasks";
import { TopBar } from "./shared/top-bar";

@Component({ selector: "app-root", imports: [RouterOutlet, RouterLink, MatButtonModule, MatFormFieldModule, MatInputModule, TopBar], templateUrl: "./app.html", styleUrl: "./app.scss" })
export class App {
  protected readonly session = inject(Session);
  protected readonly devAccount = signal("");

  constructor() {
    runTask(this.session.ready());
  }

  protected signIn(event: SubmitEvent): void {
    event.preventDefault();
    const account = this.devAccount().trim();
    if (account.length > 0) {
      runTask(this.session.signInAs(account));
    }
  }

  protected setDevAccount(event: Event): void {
    if (event.target instanceof HTMLInputElement) {
      this.devAccount.set(event.target.value);
    }
  }
}
