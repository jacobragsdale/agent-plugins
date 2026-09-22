import { Component, computed, input } from "@angular/core";
import { marked } from "marked";

/**
 * Renders a skill's Markdown. The HTML goes through [innerHTML], so Angular's sanitizer strips
 * scripts, event handlers, and javascript: links; nothing here bypasses it.
 */
@Component({ selector: "app-markdown", template: `<div class="markdown" [innerHTML]="html()"></div>` })
export class Markdown {
  public readonly source = input.required<string>();
  protected readonly html = computed(() => marked.parse(this.source(), { async: false, gfm: true }));
}

/** The Markdown after a SKILL.md's frontmatter, which readers do not need to see. */
export function withoutFrontmatter(text: string): string {
  const match = /^\uFEFF?---\r?\n[\s\S]*?\r?\n---\r?\n?/u.exec(text);
  return match === null ? text : text.slice(match[0].length);
}
