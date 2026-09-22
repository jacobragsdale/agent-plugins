import { HttpClient } from "@angular/common/http";
import { NgOptimizedImage } from "@angular/common";
import { Component, computed, inject, input, resource } from "@angular/core";
import { MatButtonModule } from "@angular/material/button";
import { RouterLink } from "@angular/router";
import { firstValueFrom } from "rxjs";
import { z } from "zod";
import { formatBytes } from "../format";
import { Session } from "../session";
import { Icon } from "../shared/icon";

const safeReleasePath = z
  .string()
  .regex(/^[a-zA-Z0-9._/-]+$/u)
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
const downloadsSchema = z.strictObject({ schemaVersion: z.literal(1), release: z.strictObject({ version: z.string(), platforms: z.array(platformSchema).readonly() }).readonly() }).readonly();

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

@Component({
  selector: "app-download-card",
  imports: [MatButtonModule, Icon],
  template: `
    <div class="row">
      <h3>{{ download().platform.name }}</h3>
      @if (download().recommended) {
        <span class="badge official">Your computer</span>
      }
    </div>
    @if (download().href; as href) {
      <a mat-flat-button download [href]="href"><app-icon name="download" />Download for {{ download().platform.name }}</a>
      <p class="muted details">{{ download().details }}</p>
    } @else {
      <p class="muted">Not available yet.</p>
    }
    @if (download().platform.sha256; as sha256) {
      <details class="technical">
        <summary>Checksum</summary>
        <code class="checksum">SHA-256 {{ sha256 }}</code>
      </details>
    }
  `,
  styles: `
    :host {
      display: grid;
      gap: 0.75rem;
      align-content: start;
    }
    :host(.recommended) {
      border-color: var(--mat-sys-primary);
    }
    h3,
    p {
      margin: 0;
    }
    .details {
      font: var(--mat-sys-body-small);
    }
    .checksum {
      display: block;
      margin-top: 0.5rem;
      overflow-wrap: anywhere;
    }
  `,
  host: { class: "card", "[class.recommended]": "download().recommended" }
})
export class DownloadCard {
  public readonly download = input.required<Download>();
}

@Component({ selector: "app-home", imports: [NgOptimizedImage, RouterLink, MatButtonModule, Icon, DownloadCard], templateUrl: "./home.html", styleUrl: "./home.scss" })
export class HomePage {
  protected readonly session = inject(Session);
  private readonly http = inject(HttpClient);
  private readonly detected = detectPlatform();

  protected readonly release = resource({ loader: async () => downloadsSchema.parse(await firstValueFrom(this.http.get<unknown>("/downloads/manifest.json"))).release });

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
