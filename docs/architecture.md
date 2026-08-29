# Architecture

Agent Plugins turns immutable source packages into desired resources for explicitly enabled coding agents. Acquisition, normalization, planning, execution, ownership, and presentation are separate boundaries.

## Source snapshots and normalization

A **source** is a tree with a top-level `agent-plugins.json` source manifest. A **source repository** is a catalog that lists HTTPS source archives; adding it never writes `sources[]`. See [source acquisition](diagrams/source-acquisition.mmd), [ADR 0002](decisions/0002-source-repositories-and-locators.md), and [ADR 0003](decisions/0003-artifact-only-catalog.md).

`sourceId` is the short namespace published in `agent-plugins.json`. `sourceKey` is a hash of the artifact URL identity. A publisher cannot transfer cache or installation ownership by changing its display namespace.

An artifact refresh HEADs for validators, GETs when needed, extracts a zip/tar/tar.gz (or reads catalog JSON), and uses the payload digest as the revision. A failed refresh leaves the prior validated snapshot active. Scheduled sync refreshes catalogs first, then sources; a catalog failure does not block source refresh. When the default catalog URL is set, sync adds that catalog if it is missing.

Manifest v2 normalizes packages containing skills and MCP servers. Invalid package entries are reported independently; source-wide ambiguity remains fatal. Leftover v1 file-tree and Agent Plugin installs are retired on sync.

## Profiles, adapters, and plans

Agent profiles are stored separately from sources. Detection is the configuration set: every installed agent is configured, and an agent that disappears is dropped. Leftover editor config or plugins from upgraded or removed IDEs do not count as an install. Version output stays advisory.

Each stable target selects a pinned dialect. A built-in adapter reports `native`, `losslessTranslation`, `lossyTranslation`, `unsupported`, or `blocked`, then returns typed desired resources. It cannot mutate the machine.

The planner fans every package component across detected agents and coalesces identical physical identities. Cursor, Codex, OpenCode, Grok Build, and GitHub Copilot share one namespaced skill under `~/.agents/skills`. Claude Code uses `~/.claude/skills`. There is no per-agent opt-out: detection is the configuration set.

The initial resources are:

- whole files or directories with installed digests;
- semantic JSON, JSONC, or TOML entries with key/value digests; and
- bindings from a package/component/target/scope to those resources.

Global preflight rejects cross-source identity collisions, different desired content at one identity, unsafe path overlap, exact owned-resource drift, malformed shared documents, explicit `conflictsWith` packages, and missing Tier 3 MCP approval.

## Central execution and recovery

Only `executor.rs` mutates planned resources. It stages every path and complete rewritten document before writing `resource-transaction.json`. Activation is deterministic and checks the original whole-path/document digest again. The ledger is atomically replaced only after all resource activations succeed.

An activation or ledger error rolls back the complete operation. On launch, a journal whose transaction ID is absent from the ledger is rolled back; a journal already committed in the ledger is cleaned up. Bulk install/uninstall and source removal share the same all-or-nothing boundary. App reset also uses that transaction when it can, but it still wipes ledger ownership if a leftover file cannot be staged, then best-effort removes namespaced skill directories and backs up modified destinations. After resources are gone it deletes Agent Plugins' own config, cache, and data so sources must be added again.

Unmanaged replacement and force-removal of modified content create a persistent backup under `~/.agents/.agent-plugins-backups`. Normal update and uninstall stop on drift. Shared config mutations preserve comments where the target format permits and never claim unrelated keys.

## Ledger v4

Ledger v4 has three indexes:

- installations retain package provenance, source digest, and removed-upstream identity;
- bindings record component, target, dialect, scope, capability, and resource IDs; and
- resources record physical identity, exact ownership digest, adapter/dialect, and consumer bindings.

Removing a binding deletes a resource only after its last consumer disappears. Migration maps each v3 destination to a legacy binding/resource. Previously recorded Cursor and Copilot plugin copies are adopted only when a `plugin.json` is present and the digest still matches, so leftover uninstall can remove those paths. Divergent or untracked content remains untouched, and the original v3 ledger is retained until v4 is written and reread successfully.

## Trust boundaries

Agent Plugins never executes source content. Generic file trees are Tier 1, skills with invokable assets are Tier 2, and MCP servers are Tier 3. Clicking Install is the approval for Tier 1 and Tier 2 resources; a Tier 3 package first shows what each MCP server would run - command, arguments, working directory, and environment variable names - and installs only after the person approves that, the way the CLI takes `--approve-mcp`. Sensitive headers must reference an environment variable rather than embedding a secret.

Every detected agent is configured; the app has no per-agent switch, so a newly installed agent picks up the packages on the next install or update. Background source refresh never invents Tier 3 approval; an MCP-affecting update remains pending until the user installs or updates it.

Hooks, monitors, in-process plugins, background services, LSP servers, and native agents/subagents remain target-qualified Tier 4 candidates. They are not portable components and require separate lifecycle, permission, threat-model, ownership, and runtime-verification decisions.

See [ADR 0001](decisions/0001-multi-agent-desired-state.md) for the product decisions and [the adapter contract](adapter-contract.md) for the pinned target matrix.

## Marketplace

The marketplace server (`server/`, .NET 10) is the only endpoint the app talks to. It publishes the catalog at `/api/catalog` in the source-repository shape, one listed source per publisher namespace, and each namespace archive at `/api/sources/{namespace}/archive`; the acquisition path above is unchanged. Artifact Keeper stores every published version immutably behind the server. See [ADR 0004](decisions/0004-internal-marketplace.md) and [the API reference](marketplace-api.md).

`locator.rs` holds the build-time marketplace URL. `marketplace.rs` attaches identity to every request for that origin: a Kerberos `Negotiate` token from SSPI (`host_identity.rs`) on a domain-joined Windows host, otherwise the `X-Dev-User` header that only a Development server trusts. Sync auto-subscribes to every source the marketplace catalog lists, joins `/api/index` metadata (publisher, version, tags, installs, installed base) onto catalog items, and posts a `heartbeat` event; item operations post `install`, `update`, and `uninstall` events from a background thread. Usage reporting never blocks or fails an operation.

`preflight.rs` runs the checks in [the preflight reference](preflight-reference.md) during sync and on demand. The report is cached, shown in the System Status dialog, and summarized in the heartbeat. `cli.rs` adds `validate`, `publish`, `search`, `install`, and `whoami` to the application binary; `publish` stages a bare skill directory or MCP document into a one-package source tree, scans it for credentials, validates it with the same code as the app, and uploads it with the same identity.

## Application and UI

On launch, `startup.rs` repairs the host environment before any catalog work: it locates `uv`/`uvx` on the process PATH, the user's login Path, and Windows App Paths, and installs `uv` only when it is still missing. Agents launch MCP servers themselves, so Node is not installed or required. Found `uv` directories go on PATH, corporate proxy variables are normalized, and `UV_NATIVE_TLS` is set so `uv` uses the platform certificate store. Windows writes PATH/proxy to the user `Environment` key (never the machine Path). On macOS the same values are published with `launchctl setenv`.

The application service serializes mutations and profile reconciliation with one operation lock, while refresh uses a separate sync lock. IPC returns plain compatibility, preview, profile, catalog, and outcome data. React validates every response with Zod and does not own filesystem or manifest policy.

The catalog presents packages and their components. Multi-component packages expand so each skill and MCP server can be installed or removed on its own, and those rows show the component description plus a Manual Invocation tag when `disable-model-invocation` is true. The package card shows that tag only when the package has at least one skill and every skill is manual; MCP servers do not change it. If no agent is detected, the app says so above the catalog and skips portable background updates until one is installed. Installing a package proceeds immediately; replacing unmanaged files, uninstalling a whole source, and removing a source all confirm before changing the machine. Updates apply only the components already selected on that package.
