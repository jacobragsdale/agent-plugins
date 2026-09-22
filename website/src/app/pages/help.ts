import { Component, input } from "@angular/core";
import { RouterLink, RouterLinkActive } from "@angular/router";

/** The technical reference: publishing from the command line and the two file formats. */
@Component({ selector: "app-help", imports: [RouterLink, RouterLinkActive], templateUrl: "./help.html", styleUrl: "./help.scss" })
export class HelpPage {
  public readonly topic = input.required<string>();
}
