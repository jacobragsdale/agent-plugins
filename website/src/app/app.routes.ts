import type { CanMatchFn, Routes } from "@angular/router";
import { adminOnly } from "./session";

/** Only the topics help.html has a case for; any other /help/... falls through to not found. */
const helpTopic: CanMatchFn = (_route, segments) => ["publish", "source-manifest", "source-repository"].includes(segments[1]?.path ?? "");

export const routes: Routes = [
  { path: "", title: "Agent Plugins", loadComponent: () => import("./pages/home").then((page) => page.HomePage) },
  { path: "browse", title: "Browse skills · Agent Plugins", loadComponent: () => import("./pages/browse").then((page) => page.BrowsePage) },
  { path: "p/:ns/:pkg", title: "Skill · Agent Plugins", loadComponent: () => import("./pages/package").then((page) => page.PackagePage) },
  { path: "p/:ns/:pkg/upload", title: "New version · Agent Plugins", data: { mode: "upload" }, loadComponent: () => import("./pages/publish").then((page) => page.PublishPage) },
  { path: "p/:ns/:pkg/suggest", title: "Suggest a change · Agent Plugins", data: { mode: "suggest" }, loadComponent: () => import("./pages/publish").then((page) => page.PublishPage) },
  { path: "suggestions/:id", title: "Suggestion · Agent Plugins", loadComponent: () => import("./pages/suggestion").then((page) => page.SuggestionPage) },
  { path: "b/:ns/:id", title: "Bundle · Agent Plugins", loadComponent: () => import("./pages/bundle").then((page) => page.BundlePage) },
  { path: "b/:ns/:id/edit", title: "Edit bundle · Agent Plugins", loadComponent: () => import("./pages/bundle-edit").then((page) => page.BundleEditPage) },
  { path: "bundles/new", title: "New bundle · Agent Plugins", loadComponent: () => import("./pages/bundle-edit").then((page) => page.BundleEditPage) },
  { path: "teams", title: "Teams · Agent Plugins", loadComponent: () => import("./pages/teams").then((page) => page.TeamsPage) },
  { path: "teams/:ns", title: "Team · Agent Plugins", loadComponent: () => import("./pages/team").then((page) => page.TeamPage) },
  { path: "l/:code", title: "Opening a link · Agent Plugins", loadComponent: () => import("./pages/link").then((page) => page.LinkPage) },
  { path: "publish", title: "Share a skill · Agent Plugins", data: { mode: "new" }, loadComponent: () => import("./pages/publish").then((page) => page.PublishPage) },
  { path: "mine", title: "My skills · Agent Plugins", loadComponent: () => import("./pages/mine").then((page) => page.MinePage) },
  { path: "admin", title: "Admin · Agent Plugins", canMatch: [adminOnly], loadComponent: () => import("./pages/admin").then((page) => page.AdminPage) },
  { path: "help", pathMatch: "full", redirectTo: "help/publish" },
  { path: "help/:topic", canMatch: [helpTopic], title: "Help · Agent Plugins", loadComponent: () => import("./pages/help").then((page) => page.HelpPage) },
  { path: "**", title: "Not found · Agent Plugins", loadComponent: () => import("./pages/not-found").then((page) => page.NotFoundPage) }
];
