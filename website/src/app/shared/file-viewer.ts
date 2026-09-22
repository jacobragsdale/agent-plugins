import { Component, computed, inject, input, linkedSignal, resource } from "@angular/core";
import { MatProgressBarModule } from "@angular/material/progress-bar";
import type { PackageFile } from "../api";
import { Api, ApiError, fileUrl } from "../api";
import { formatBytes } from "../format";
import { Icon } from "./icon";
import { Markdown, withoutFrontmatter } from "./markdown";

const previewLimit = 1024 * 1024;

/** Lists a version's files and previews the selected one: Markdown rendered, anything else as text. */
@Component({
  selector: "app-file-viewer",
  imports: [MatProgressBarModule, Icon, Markdown],
  template: `
    @if (files.hasValue()) {
      <div class="viewer">
        <ul class="list" aria-label="Files">
          @for (file of files.value(); track file.path) {
            <li>
              <button type="button" [class.selected]="file.path === selected()?.path" [attr.aria-current]="file.path === selected()?.path" (click)="selected.set(file)">
                <app-icon [name]="file.path.endsWith('.md') ? 'description' : 'code'" />
                <span class="path">{{ file.path }}</span>
                <span class="size">{{ size(file) }}</span>
              </button>
            </li>
          }
        </ul>
        <section class="preview" aria-live="polite">
          @if (selected(); as file) {
            @if (file.size > previewLimit) {
              <p class="muted">This file is too large to preview. <a [href]="download(file)">Download it</a>.</p>
            } @else if (content.isLoading()) {
              <mat-progress-bar mode="indeterminate" aria-label="Loading file" />
            } @else if (rendered(); as markdown) {
              <app-markdown [source]="markdown" />
            } @else if (content.hasValue()) {
              <pre><code>{{ content.value() }}</code></pre>
            } @else {
              <p class="muted">This file can't be shown here. <a [href]="download(file)">Download it</a>.</p>
            }
          }
        </section>
      </div>
    } @else if (files.error(); as error) {
      <p class="problem">{{ message(error) }}</p>
    } @else {
      <mat-progress-bar mode="indeterminate" aria-label="Loading files" />
    }
  `,
  styles: `
    .viewer {
      display: grid;
      grid-template-columns: minmax(14rem, 18rem) 1fr;
      gap: 1rem;
      align-items: start;
    }
    .list {
      list-style: none;
      margin: 0;
      padding: 0.25rem;
      border: var(--border);
      border-radius: var(--radius-small);
      max-height: 32rem;
      overflow: auto;
    }
    button {
      display: flex;
      align-items: center;
      gap: 0.5rem;
      width: 100%;
      border: 0;
      border-radius: 8px;
      padding: 0.45rem 0.6rem;
      background: none;
      color: inherit;
      font: var(--mat-sys-body-medium);
      text-align: left;
      cursor: pointer;
      &:hover {
        background: var(--mat-sys-surface-container-high);
      }
      &.selected {
        background: var(--mat-sys-secondary-container);
        color: var(--mat-sys-on-secondary-container);
      }
    }
    .path {
      flex: 1;
      overflow-wrap: anywhere;
    }
    .size {
      color: var(--muted);
      font: var(--mat-sys-label-small);
    }
    .preview {
      min-width: 0;
      pre {
        margin: 0;
        max-height: 40rem;
      }
    }
    @media (max-width: 720px) {
      .viewer {
        grid-template-columns: 1fr;
      }
    }
  `
})
export class FileViewer {
  public readonly ns = input.required<string>();
  public readonly packageId = input.required<string>();
  public readonly version = input.required<string>();

  protected readonly previewLimit = previewLimit;
  private readonly api = inject(Api);

  protected readonly files = resource({
    params: () => ({ ns: this.ns(), packageId: this.packageId(), version: this.version() }),
    loader: ({ params }) => this.api.files(params.ns, params.packageId, params.version)
  });

  /** Starts on the first SKILL.md, which is what most readers came for. */
  protected readonly selected = linkedSignal<readonly PackageFile[] | undefined, PackageFile | undefined>({
    source: () => (this.files.hasValue() ? this.files.value() : undefined),
    computation: (files) => files?.find((file) => file.path.endsWith("SKILL.md")) ?? files?.[0]
  });

  protected readonly content = resource({
    params: () => {
      const file = this.selected();
      return file === undefined || file.size > previewLimit ? undefined : { ns: this.ns(), packageId: this.packageId(), version: this.version(), path: file.path };
    },
    loader: ({ params }) => this.api.fileText(params.ns, params.packageId, params.version, params.path)
  });

  protected readonly rendered = computed(() => {
    const file = this.selected();
    return file?.path.endsWith(".md") === true && this.content.hasValue() ? withoutFrontmatter(this.content.value()) : null;
  });

  protected size(file: PackageFile): string {
    return formatBytes(file.size);
  }

  protected download(file: PackageFile): string {
    return fileUrl(this.ns(), this.packageId(), this.version(), file.path);
  }

  protected message(error: unknown): string {
    return ApiError.from(error).message;
  }
}
