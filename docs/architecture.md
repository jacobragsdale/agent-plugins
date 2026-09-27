# Architecture

Agent Plugins turns immutable source packages into desired resources for explicitly enabled coding agents. Acquisition, normalization, planning, execution, ownership, and presentation are separate boundaries.

## Source snapshots and normalization

A **source** is a tree with a top-level `agent-plugins.json` source manifest. A **source repository** is a catalog that lists HTTPS source archives; adding it never writes `sources[]`. See [source acquisition](diagrams/source-acquisition.mmd), [ADR 0002](decisions/0002-source-repositories-and-locators.md), and [ADR 0003](decisions/0003-artifact-only-catalog.md).

`sourceId` is the short namespace published in `agent-plugins.json`. `sourceKey` is a hash of the artifact URL identity. A publisher cannot transfer cache or installation ownership by changing its display namespace.

An artifact refresh HEADs for validators, sends a conditional GET (`If-None-Match`, `If-Modified-Since`) when they differ, extracts a zip/tar/tar.gz (or reads catalog JSON), and uses the payload digest as the revision. A failed refresh leaves the prior validated snapshot active. Sync fetches catalogs and sources a few at a time; a catalog failure does not block source refresh, and a host that could not be connected to once is skipped for the rest of that pass. When the default catalog URL is set, sync adds that catalog if it is missing.

`sync-health.json` in the cache records, per catalog and source, the last successful fetch, the last result, and a not-found count. Cached loads read it, so a window reopened offline still marks which sources are a saved copy, and the pass as a whole is `online`, `degraded`, or `offline`. A 404 or 410 is not trusted on first sight, because the marketplace answers 404 for both an unpublished namespace and revoked access: a source is retired only after three not-found answers spanning three days with no success in between, and the default catalog never is.

Manifest v2 normalizes packages containing skills and MCP servers. Invalid package entries are reported independently; source-wide ambiguity remains fatal. Leftover v1 file-tree and Agent Plugin installs are retired on sync; a locked file there is logged and retried rather than blocking the load.

The same types parse two ways. Documents the app fetches (source manifests and catalogs) are parsed tolerantly: fields this build does not know are ignored, and an invalid package or listing is dropped with one message while the rest stays usable. Publishing, the `validate-source` and `validate-source-repository` binaries run on a local tree, and the generated JSON Schemas stay strict: unknown fields and any invalid package fail the whole document. A newer publisher's document therefore still loads in an older client, while the marketplace accepts only what the current contract describes.

## Profiles, adapters, and plans

Agent profiles are stored separately from sources. Detection is the configuration set: every installed agent is configured, and an agent that disappears is dropped. A probe that times out or errors is inconclusive and keeps the agent as it was, so a slow machine does not uninstall anything. Leftover editor config or plugins from upgraded or removed IDEs do not count as an install. Version output stays advisory.

Each stable target selects a pinned dialect. A built-in adapter reports `native`, `losslessTranslation`, `lossyTranslation`, `unsupported`, or `blocked`, then returns typed desired resources. It cannot mutate the machine.

The planner fans every package component across detected agents and coalesces identical physical identities. GitHub Copilot, Cursor, OpenCode, pi, Codex, Grok Build, and the ChatGPT app share one namespaced skill under `~/.agents/skills`. Claude Code uses `~/.claude/skills`. Claude Desktop's Chat and Cowork tabs take skills only from the claude.ai account, so that adapter reports skills unsupported; pi reports MCP servers unsupported. Detection is the configuration set, with one exception a person makes: a connector can be kept out of chosen apps, recorded in `package-choices.json` and skipped by the planner ([ADR 0008](decisions/0008-primary-targets-and-connector-choices.md)).

Each adapter writes an MCP server in its app's own spelling: the `type` values it expects, and `${NAME}` as `${NAME}`, `${env:NAME}`, or `{env:NAME}`, or as Codex's forwarded variables. A placement an app cannot express makes that pairing unsupported, with the reason. See [MCP spelling](adapter-contract.md#mcp-spelling).

The initial resources are:

- whole files or directories with installed digests;
- semantic JSON, JSONC, or TOML entries with key/value digests; and
- bindings from a package/component/target/scope to those resources.

Global preflight rejects cross-source identity collisions, different desired content at one identity, unsafe path overlap, exact owned-resource drift, malformed shared documents, explicit `conflictsWith` packages, and missing Tier 3 MCP approval. A folder or settings entry someone else put where a package goes is reported as the `conflict` state before an install is attempted, so the window offers Replace instead of an install that can only fail.

## Central execution and recovery

Only `executor/` mutates planned resources. It stages every path and complete rewritten document before writing `resource-transaction.json`. Activation is deterministic and checks the original whole-path/document digest again. The ledger is atomically replaced only after all resource activations succeed.

An activation or ledger error rolls back the complete operation. Recovery runs before every ledger read and every change: a journal whose transaction ID is absent from the ledger is rolled back; a journal already committed in the ledger is cleaned up, and leftovers that cannot be removed yet are parked for a later sweep. A journal that cannot be parsed is moved aside. Only a failed rollback keeps the journal, and new changes are refused until a later sync finishes it. Reads never wait on recovery. Bulk install/uninstall and source removal run each package as its own all-or-nothing transaction, so one that fails reports its own message and the rest finish; a source stays configured while any of its packages remains installed. App reset also uses that transaction when the ledger is usable, but it still wipes ledger ownership if a leftover file cannot be staged, then best-effort removes namespaced skill directories and backs up modified destinations. With a damaged, unreadable, or newer ledger it removes the resources it can find and the namespaced skill directories instead, since reset is the way out of that state. After resources are gone it deletes Agent Plugins' own config, cache, and data so sources must be added again.

One agent's unreadable or locked settings file skips only that agent: its bindings leave the plan, the outcome carries a warning naming the app, and the next sync retries it while the app stays open. JSON settings files with a byte order mark, or empty ones, read as settings.

Unmanaged replacement and force-removal of modified content create a persistent backup under `~/.agents/.agent-plugins-backups`. Normal update and uninstall stop on drift. Shared config mutations preserve comments where the target format permits and never claim unrelated keys.

## Ledger v4

Ledger v4 has three indexes:

- installations retain package provenance, source digest, and removed-upstream identity;
- bindings record component, target, dialect, scope, capability, and resource IDs; and
- resources record physical identity, exact ownership digest, adapter/dialect, and consumer bindings.

Removing a binding deletes a resource only after its last consumer disappears.

Matching disk against the ledger has four answers: match, modified, missing, and unknown. Directory digests leave out tool leftovers such as `__pycache__` and `.DS_Store`; a digest recorded before that rule is still accepted. A changed file is the user's and is never overwritten without a backup. A missing file holds nothing of the user's, so the next sync re-creates it from the snapshot of the installed version, re-adding an MCP entry only when the ledger still holds the exact entry that was approved; uninstalling a missing package succeeds. A file that cannot be read proves nothing, so the package keeps its last status.

Local metadata heals instead of blocking. `installations.json`, `sources.json`, and `agent-profiles.json` each keep a `.previous` last good copy. An unparsable ledger is moved aside to `installations.json.corrupt-<timestamp>` and replaced by that copy, and one unreadable record is dropped without its package's neighbours. A ledger written by a newer version is read-only. A damaged `sources.json` falls back to its copy, then to a list rebuilt from the cached default catalog and the installed packages' source URLs; damaged profiles fall back to their copy, then to fresh detection. Migration maps each v3 destination to a legacy binding/resource. Previously recorded Cursor and Copilot plugin copies are adopted only when a `plugin.json` is present and the digest still matches, so leftover uninstall can remove those paths. Divergent or untracked content remains untouched, and the original v3 ledger is retained until v4 is written and reread successfully.

## Trust boundaries

Agent Plugins never executes source content. Generic file trees are Tier 1, skills with invokable assets are Tier 2, and MCP servers are Tier 3. Clicking Install is the approval for Tier 1 and Tier 2 resources; a Tier 3 package first shows what each MCP server would run, which apps get it, and whether an admin checked it, and installs only after the person approves that, the way the CLI takes `--approve-mcp`. Sensitive headers must reference an environment variable rather than embedding a secret; the approval dialog saves each person's values to their user environment.

An approval covers exactly the MCP entries the ledger records. An update whose MCP entries are all owned unchanged needs no new approval, so a skill change in a package with a connector updates in the background; one that adds or changes an entry stays pending until the person approves it. When a newer app version spells an app's entry differently, a sync rewrites the approved entries for the apps that already have them. Background refresh never extends a connector to a newly detected app.

Hooks, monitors, in-process plugins, background services, LSP servers, and native agents/subagents remain target-qualified Tier 4 candidates. They are not portable components and require separate lifecycle, permission, threat-model, ownership, and runtime-verification decisions.

See [ADR 0001](decisions/0001-multi-agent-desired-state.md) for the product decisions and [the adapter contract](adapter-contract.md) for the pinned target matrix.

## Marketplace

The marketplace server (`server/`, .NET 10) is the only endpoint the app talks to. It publishes the catalog at `/api/catalog` in the source-repository shape, one listed source per publisher namespace, and each namespace archive at `/api/sources/{namespace}/archive`; the acquisition path above is unchanged. Artifact Keeper stores every published version immutably behind the server. See [ADR 0004](decisions/0004-internal-marketplace.md) and [the API reference](marketplace-api.md).

Access is server policy ([ADR 0005](decisions/0005-marketplace-access-control.md), [ADR 0007](decisions/0007-self-service-marketplace.md)): a space, package, or bundle is public or private, a private one is visible to its space's owners and a share list of people, teams, and optional AD groups, the catalog, index, and archives are filtered per caller, and anything hidden answers 404 so the client's gone-source handling applies unchanged. A team is a namespace anyone can create; its members own it. A package that an owner or admin revokes is listed under `revoked` in the index, and sync uninstalls it.

`locator.rs` holds the marketplace URL: the `MarketplaceUrl` policy under `Software\Policies\AgentPlugins` when IT sets one, otherwise the build-time constant. `marketplace.rs` attaches identity to every request for that origin: a Kerberos `Negotiate` token from SSPI (`host_identity.rs`) on a Windows host joined to a domain or to Microsoft Entra ID, otherwise the `X-Dev-User` header that only a Development server trusts. The index is requested with its saved ETag, and a namespace archive whose catalog `digest` matches the saved copy's ETag is not requested at all. Sync auto-subscribes to every source the marketplace catalog lists, joins `/api/index` metadata (publisher, version, tags, installs, installed base) onto catalog items, and posts a `heartbeat` event; item operations post `install`, `update`, and `uninstall` events from a background thread. Usage reporting never blocks or fails an operation.

Undelivered events wait in `events-outbox.json` in the cache and are replayed, oldest first, before the next report; the outbox keeps the newest 1,000 events from seven days and only the latest heartbeat. An `update` event carries the version the marketplace index listed before the sync. Marketplace and artifact requests share one sender with a 10-second connect timeout and two retries on connect failure, timeout, 5xx, or 429, honouring `Retry-After`. The cached identity the header shows is dropped only when the server answers 401 or 403, never when it could not be reached.

`preflight.rs` runs the checks in [the preflight reference](preflight-reference.md) during sync and on demand. The report is cached, shown in the System Status dialog, and summarized in the heartbeat. `cli.rs` adds the [command-line verbs](cli-reference.md) to the application binary; `publish` stages a bare skill directory, skill pack, MCP document, or one package of a source tree into a one-package source tree, scans it for credentials, validates it with the same code as the app, and uploads it with the same identity.

## Application and UI

On launch, `startup.rs` repairs the host environment before any catalog work: it locates `uv`/`uvx` on the process PATH, the user's login Path, and Windows App Paths, normalizes corporate proxy variables, and sets `UV_NATIVE_TLS` so `uv` uses the platform certificate store. Agents launch MCP servers themselves, so Node is not installed or required. A background thread then publishes the `uv` and app directories, `UV_NATIVE_TLS`, and any proxy variables the user set to the user session, writing only values that change, and downloads `uv` when it is still missing. The system proxy applies to the app's own process only. Windows writes to the user `Environment` key (never the machine Path); macOS uses `launchctl setenv`. `session-environment.json` in the app data directory records what the app wrote. The Windows uninstallers run `agent-plugins remove-from-path` to take the app directory back off the user Path; the `uv` directory stays, because `uv` and the tools it installed stay too.

The application service serializes mutations, ledger reads, and profile reconciliation with one operation lock; a sync holds a separate sync lock for its whole run. Network fetches hold only the sync lock. A sync takes the operation lock twice, briefly: to read the configuration, then to activate what it fetched, repair and update installed packages, and extend them to newly detected agents. Installs and removals therefore stay responsive while a server is slow, and the configuration is read again before activation so a source added or removed meanwhile is kept. On-demand preflight reads the ledger under the operation lock, so journal recovery never runs in the middle of an install, and probes the server after releasing it.

IPC returns plain compatibility, preview, profile, catalog, and outcome data. Internals report plain strings; `ipc_error.rs` turns each into an `IpcError` with a `kind` (`offline`, `locked`, `retryable`, `needsUser`, or `bug`) chosen from markers the owning modules export; the first sentence becomes `message` and the rest `detail`. The window acts on the kind alone: `offline` feeds the offline banner, `locked` and `retryable` retry the action once, and the others are shown. React validates every response with Zod, ignoring fields a newer backend adds and dropping a single malformed source or package rather than the whole state, and does not own filesystem or manifest policy.

The catalog presents packages and their components. Multi-component packages expand so each skill and MCP server can be installed or removed on its own, and those rows show the component description plus an **Only when you ask** or **Used automatically** toggle for each skill. The toggle starts from the source's `disable-model-invocation`; a person's different choice is saved in `invocation-overrides.json`, and planning writes it into the materialized `SKILL.md`, so the ledger records it as the installed content rather than a local change. The window calls an MCP server a connector. The package card's toggle reads manual only when the package has at least one skill and every skill is manual; MCP servers do not change it. If no agent is detected, the app says so above the catalog and skips portable background updates until one is installed. Installing a package proceeds immediately; installing or uninstalling a whole source, replacing unmanaged files, restoring a changed package's original, keeping or removing a changed copy, and removing a source all confirm before changing the machine. After a sync, the app raises desktop notifications for removals, updates, and marketplace news while its window is not focused. Updates apply only the components already selected on that package.
