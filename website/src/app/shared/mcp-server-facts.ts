import { Component, computed, input } from "@angular/core";
import type { McpServer } from "../api";

/** What one MCP server runs or connects to, and which keys it needs, as a short list of facts. */
@Component({
  selector: "app-mcp-server-facts",
  template: `
    <dl>
      <dt>Server</dt>
      <dd>
        <strong>{{ server().name }}</strong> <span class="muted">({{ where() }})</span>
      </dd>
      <dt>{{ remote() ? "Connects to" : "Runs" }}</dt>
      <dd>
        <code>{{ target() }}</code>
      </dd>
      <dt>Needs</dt>
      <dd>{{ needs() }}</dd>
    </dl>
  `,
  styles: `
    dl {
      display: grid;
      grid-template-columns: max-content 1fr;
      gap: 0.25rem 1rem;
      margin: 0;
      padding: 0.75rem 1rem;
      border: var(--border);
      border-radius: var(--radius-small);
    }
    dt {
      color: var(--muted);
    }
    dd {
      margin: 0;
      overflow-wrap: anywhere;
    }
  `
})
export class McpServerFacts {
  public readonly server = input.required<McpServer>();

  protected readonly remote = computed(() => (this.server().url ?? null) !== null);
  protected readonly where = computed(() => (this.server().transport === "stdio" ? "runs on the PC" : `online service, ${this.server().transport}`));
  protected readonly target = computed(() => {
    const server = this.server();
    return server.url ?? [server.command ?? "", ...server.args].join(" ").trim();
  });

  protected readonly needs = computed(() => {
    const names = [...this.server().envNames, ...this.server().headerNames.map((header) => `${header} header`)];
    return names.length === 0 ? "Nothing to set up" : names.join(", ");
  });
}
