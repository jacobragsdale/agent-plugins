import type { Routes } from "@angular/router";
import { adminOnly } from "./session";

export const routes: Routes = [
  { path: "", title: "Agent Plugins", loadComponent: () => import("./pages/home").then((page) => page.HomePage) },
  { path: "browse", title: "Browse skills · Agent Plugins", loadComponent: () => import("./pages/browse").then((page) => page.BrowsePage) },
  { path: "p/:ns/:pkg", title: "Skill · Agent Plugins", loadComponent: () => import("./pages/package").then((page) => page.PackagePage) },
  { path: "p/:ns/:pkg/edit", title: "Edit · Agent Plugins", data: { mode: "edit" }, loadComponent: () => import("./pages/publish").then((page) => page.PublishPage) },
  { path: "p/:ns/:pkg/upload", title: "New version · Agent Plugins", data: { mode: "upload" }, loadComponent: () => import("./pages/publish").then((page) => page.PublishPage) },
  { path: "publish", title: "Share a skill · Agent Plugins", data: { mode: "new" }, loadComponent: () => import("./pages/publish").then((page) => page.PublishPage) },
  { path: "mine", title: "My skills · Agent Plugins", loadComponent: () => import("./pages/mine").then((page) => page.MinePage) },
  { path: "admin", title: "Admin · Agent Plugins", canMatch: [adminOnly], loadComponent: () => import("./pages/admin").then((page) => page.AdminPage) },
  { path: "help", redirectTo: "help/publish" },
  { path: "help/:topic", title: "Help · Agent Plugins", loadComponent: () => import("./pages/help").then((page) => page.HelpPage) },
  { path: "**", title: "Not found · Agent Plugins", loadComponent: () => import("./pages/not-found").then((page) => page.NotFoundPage) }
];
