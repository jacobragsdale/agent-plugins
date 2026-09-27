import { HttpClient } from "@angular/common/http";
import { Component, computed, inject, input, resource } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { RouterLink } from "@angular/router";
import { firstValueFrom } from "rxjs";
import { z } from "zod";
import type { IndexPackage } from "../api";
import { Api } from "../api";
import { formatBytes } from "../format";
import { Icon } from "../shared/icon";
import { PackageCard } from "../shared/package-card";

/** Tauri names installers like "Agent Plugins_0.2.2_x64-setup.exe", so spaces are allowed; each segment is URL-encoded. */
const safeReleasePath = z
  .string()
  .regex(/^[a-zA-Z0-9 ._/-]+$/u)
  .refine((path) => !path.startsWith("/") && !path.split("/").includes(".."), "unsafe path");

const platformSchema = z
  .strictObject({
    id: z.enum(["windows", "linux"]),
    name: z.string(),
    format: z.string(),
    architecture: z.string(),
    file: safeReleasePath.nullable(),
    sizeBytes: z.number().int().positive().nullable(),
    sha256: z
      .string()
      .regex(/^[a-f0-9]{64}$/u)
      .nullable()
  })
  .readonly();
type Platform = z.infer<typeof platformSchema>;

/** `/downloads/manifest.json`, written by whoever publishes an installer (server/releases/). */
export const downloadsSchema = z.strictObject({ schemaVersion: z.literal(1), release: z.strictObject({ version: z.string(), platforms: z.array(platformSchema).readonly() }).readonly() }).readonly();

function detectPlatform(): Platform["id"] | null {
  const agent = navigator.userAgent.toLowerCase();
  if (agent.includes("windows")) {
    return "windows";
  }

  return agent.includes("linux") && !agent.includes("android") ? "linux" : null;
}

interface Download {
  readonly platform: Platform;
  readonly href: string | null;
  readonly details: string;
  readonly recommended: boolean;
}

/** One platform's installer, as a row of the download panel. */
@Component({
  selector: "app-download-card",
  imports: [MatButtonModule, Icon],
  template: `
    <div class="line">
      <div class="text">
        <strong>{{ download().platform.name }}</strong>
        @if (download().recommended) {
          <span class="badge">This PC</span>
        }
        <div class="meta">{{ download().details }}</div>
      </div>
      @if (download().href; as href) {
        <a download [href]="href" [matButton]="download().recommended ? 'filled' : 'outlined'"><app-icon name="download" />Download</a>
      } @else {
        <span class="muted">Not available yet</span>
      }
    </div>
    @if (download().platform.sha256; as sha256) {
      <details>
        <summary>SHA-256</summary>
        <code>{{ sha256 }}</code>
      </details>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 0.25rem;
      padding-top: 0.75rem;
      border-top: var(--border);
    }
    .line {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: 0.75rem;
    }
    .badge {
      margin-left: 0.4rem;
    }
    details {
      font: var(--mat-sys-body-small);
      color: var(--muted);
    }
    summary {
      cursor: pointer;
    }
    code {
      display: block;
      margin-top: 0.25rem;
      overflow-wrap: anywhere;
    }
  `
})
export class DownloadCard {
  public readonly download = input.required<Download>();
}

/** A row of skill cards under a heading; nothing at all when there are none. */
@Component({
  selector: "app-skill-shelf",
  imports: [RouterLink, MatButtonModule, PackageCard],
  template: `
    @if (items().length > 0) {
      <section class="stack" [attr.aria-label]="heading()">
        <div class="page-head">
          <h2>{{ heading() }}</h2>
          <a mat-button routerLink="/browse" [hidden]="!all()">All skills</a>
        </div>
        <div class="grid">
          @for (item of items(); track item.id) {
            <app-package-card [item]="item" />
          }
        </div>
      </section>
    }
  `
})
export class SkillShelf {
  public readonly heading = input.required<string>();
  public readonly items = input.required<readonly IndexPackage[]>();
  public readonly all = input(false);
}

@Component({ selector: "app-home", imports: [RouterLink, MatButtonModule, DownloadCard, SkillShelf], templateUrl: "./home.html", styleUrl: "./home.scss" })
export class HomePage {
  private readonly http = inject(HttpClient);
  private readonly api = inject(Api);
  private readonly detected = detectPlatform();

  protected readonly release = resource({ loader: async () => downloadsSchema.parse(await firstValueFrom(this.http.get<unknown>("/downloads/manifest.json"))).release });

  /** The index needs a signed-in caller; without one the page simply has no skills to show. */
  private readonly index = resource({ loader: () => this.api.index() });
  /** Skills someone shared with the caller by name, a team, or a link; newest first. */
  protected readonly shared = computed(() =>
    this.index.hasValue()
      ? this.index
          .value()
          .packages.filter((item) => item.sharedWithYou)
          .toSorted((left, right) => Date.parse(right.publishedAt) - Date.parse(left.publishedAt))
          .slice(0, 6)
      : []
  );

  protected readonly popular = computed(() =>
    this.index.hasValue()
      ? this.index
          .value()
          .packages.toSorted((left, right) => right.installedBase - left.installedBase)
          .slice(0, 6)
      : []
  );

  protected readonly downloads = computed<readonly Download[]>(() => {
    if (!this.release.hasValue()) {
      return [];
    }

    return [...this.release.value().platforms]
      .sort((left, right) =>
        (left.id === this.detected) === (right.id === this.detected) ? left.name.localeCompare(right.name) : Number(right.id === this.detected) - Number(left.id === this.detected)
      )
      .map((platform) => ({
        platform,
        href: platform.file === null ? null : `/downloads/${platform.file.split("/").map(encodeURIComponent).join("/")}`,
        details: [platform.architecture, platform.format, platform.sizeBytes === null ? null : formatBytes(platform.sizeBytes)].filter((part) => part !== null).join(" · "),
        recommended: platform.id === this.detected
      }));
  });
}
