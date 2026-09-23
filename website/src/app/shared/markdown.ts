import { Component, computed, input, output } from "@angular/core";
import { marked } from "marked";

/**
 * Renders a skill's Markdown. The HTML goes through [innerHTML], so Angular's sanitizer strips
 * scripts, event handlers, and javascript: links; nothing here bypasses it.
 */
@Component({ selector: "app-markdown", template: `<div class="markdown" [innerHTML]="html()"></div>`, host: { "(click)": "follow($event)" } })
export class Markdown {
  public readonly source = input.required<string>();
  /** The previewed file's path in the package; relative links resolve against it. */
  public readonly path = input.required<string>();
  /** A link to another file in the same package, as a package path. */
  public readonly opened = output<string>();
  protected readonly html = computed(() => marked.parse(this.source(), { async: false, gfm: true }));

  /** With <base href="/">, a relative or #fragment link would leave the app; package links open here instead. */
  protected follow(event: Event): void {
    const href = event.target instanceof Element ? event.target.closest("a")?.getAttribute("href") : null;
    if (href === null || href === undefined || /^(?:[a-z][a-z0-9+.-]*:|\/\/)/iu.test(href)) {
      return;
    }

    event.preventDefault();
    if (!href.startsWith("#")) {
      this.opened.emit(decodeURIComponent(new URL(href, `file:///${this.path()}`).pathname.slice(1)));
    }
  }
}

/** The Markdown after a SKILL.md's frontmatter, which readers do not need to see. */
export function withoutFrontmatter(text: string): string {
  const match = /^\uFEFF?---\r?\n[\s\S]*?\r?\n---\r?\n?/u.exec(text);
  return match === null ? text : text.slice(match[0].length);
}
