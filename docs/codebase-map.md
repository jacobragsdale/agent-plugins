# Codebase map

Where each responsibility lives. This describes the layout; [architecture](architecture.md) explains why the boundaries fall here, and [how to work on Agent Plugins](development.md) covers the workflows that cross them.

```text
src/           React window: presentation and Zod validation only
src-tauri/     Rust: everything that decides or writes
server/        .NET 10 marketplace API; also serves the web portal and installers
website/       Angular web portal: browse, publish, review
schemas/       Generated JSON Schemas for the manifest contracts
docs/          This documentation set, with ADRs under decisions/
```

## Rust: the pipeline

A source becomes files on disk in one direction. Each stage may use the stage above it and nothing below.

| Stage        | Modules                                                                                                                         | Responsibility                                                                                                                                                                                                                                       |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Acquire      | `locator.rs`, `artifact.rs`, `sources.rs`, `source.rs`, `repository.rs`                                                         | Resolve an HTTPS locator, download, verify, extract safely, and cache a snapshot.                                                                                                                                                                    |
| Normalize    | `manifest.rs`, `catalog.rs`, `mcp.rs`, `digest.rs`                                                                              | Parse the pinned manifest, materialize skill names, validate MCP documents, digest content.                                                                                                                                                          |
| Plan         | `adapters.rs`, `agent_profiles.rs`, `planner.rs`, `resource.rs`, `invocation.rs`, `choices.rs`                                  | Detect agents, fan components across targets, coalesce identities, run structural preflight. Apply the person's skill invocation, held-update, and connector-app choices.                                                                            |
| Execute      | `executor/`, `managed_documents.rs`, `fs_retry.rs`                                                                              | Stage, journal, activate, and roll back. The only writer of planned resources.                                                                                                                                                                       |
| Own          | `ledger.rs`, `install.rs`                                                                                                       | Record installations, bindings, and physical resources; derive item status.                                                                                                                                                                          |
| Serve        | `application/`, `ipc.rs`, `ipc_error.rs`, `app_state.rs`                                                                        | Sequence use cases behind locks, project state for the UI, and classify errors.                                                                                                                                                                      |
| Reach out    | `marketplace.rs`, `host_identity.rs`, `preflight.rs`                                                                            | Identity, marketplace requests, events, and the startup checks.                                                                                                                                                                                      |
| Host         | `paths.rs`, `startup.rs`, `process.rs`, `parallel.rs`, `qa_paths.rs`, `tray.rs`, `notify.rs`, `tutorial.rs`, `app_locations.rs` | Filesystem roots, environment repair and saved connector settings, the log file, bounded subprocesses, QA isolation, tray and Launch at Login, desktop notifications, the skill tutorial and **Create a skill**, and where each AI app's program is. |
| Entry points | `main.rs`, `lib.rs`, `cli.rs`, `deep_link.rs`, `staging.rs`, `bin/`                                                             | Window, command registration, headless verbs, website links, publish staging, validator binaries.                                                                                                                                                    |

### Modules worth knowing before you change anything

| Module                 | Note                                                                                                                                                                                         |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `executor/mod.rs`      | The only filesystem writer for planned resources. `stage.rs` prepares, `journal.rs` recovers, `activate.rs` commits the ledger, `matching.rs` decides whether disk still matches the ledger. |
| `planner.rs`           | Pure apart from reading the person's choices. Fan-out, coalescing, structural preflight, and `needs_approval` — the Tier 3 gate, which passes MCP entries the ledger already owns unchanged. |
| `ledger.rs`            | Ledger v4: installations, bindings, resources. A resource dies when its last consumer binding does.                                                                                          |
| `agent_profiles.rs`    | Detection is the configuration set. Results are cached for 60 seconds; a sync, diagnostics, or window focus clears the cache. A timed-out or failed probe keeps the agent as it was.         |
| `managed_documents.rs` | Comment-preserving JSONC and TOML edits. Why installing an MCP server does not destroy a user's own config.                                                                                  |
| `preflight.rs`         | Declarative checks with stable IDs. IDs are retired, never renamed.                                                                                                                          |
| `ipc_error.rs`         | Turns an internal error string into the `IpcError` every command rejects with. See [IPC errors](#ipc-errors).                                                                                |
| `fs_retry.rs`          | Retries transient Windows sharing errors for up to 4 seconds, and words I/O errors (`is_locked`, disk full, access denied) for people.                                                       |
| `choices.rs`           | `package-choices.json`: packages whose updates are held, and the apps each connector is kept out of.                                                                                         |
| `locator.rs`           | The marketplace and download URLs: the `Software\Policies\AgentPlugins` values, else the build-time constants.                                                                               |
| `deep_link.rs`         | Parses `agent-plugins://install/…` and `open/…` links strictly, and holds the newest one until the window takes it. `startup.rs` registers the link type per user.                           |
| `qa_paths.rs`          | Debug-only. `AGENT_PLUGINS_QA_ROOT` relocates every root beneath the temp directory.                                                                                                         |

### `application/`

The use-case layer. Mutations and ledger reads serialize on the operation lock. A sync holds the sync lock for its whole run, fetches under it alone, and takes the operation lock only to read the configuration and then to activate and reconcile.

| File         | Responsibility                                                                                                                                                                                              |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `mod.rs`     | `RuntimeState`, the locks, the sync scheduler (15 minutes; 1, 2, then 5 after failed passes; on resume), the focus sync, and the re-exports `ipc.rs` calls.                                                 |
| `sync.rs`    | Cached load, on-demand preflight, full sync, catalog auto-subscribe, background updates, `sync-health.json`, connectivity, and the not-found grace period.                                                  |
| `sources.rs` | Prepare, confirm, and cancel a source. Preparation stages a candidate; confirmation writes `sources.json`.                                                                                                  |
| `items.rs`   | Install, replace, uninstall, batch plan and run across sources (source **Install all** and bundles), source removal, app reset, re-creating missing files, and extending installs to newly detected agents. |
| `status.rs`  | Derives the item and component states the cards show.                                                                                                                                                       |
| `project.rs` | Builds `AppState` for the window: catalog items, compatibility, approval details, marketplace metadata.                                                                                                     |

### IPC errors

Every command, and a failed `scheduled-sync` event, rejects with `{ kind, message, detail? }`. `message` is the first sentence of the internal error; `detail` is the rest. `ipc_error.rs` picks the kind from markers the owning modules export, in this order:

| Kind        | Chosen when                                                                                                     | The window                                                     |
| ----------- | --------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `needsUser` | The message refuses to touch local changes, or names a ledger from a newer version.                             | Shows it.                                                      |
| `bug`       | A worker thread panicked.                                                                                       | Shows `Something went wrong.` with the text under **Details**. |
| `locked`    | `fs_retry::is_locked`: another app holds the file.                                                              | Retries once after 3 seconds, then shows it.                   |
| `offline`   | `artifact::is_connect_failure`: the connection was refused, the name did not resolve, or the connect timed out. | Shows the offline banner instead.                              |
| `retryable` | `artifact::is_transient` (timeout, 5xx, 429), or an interrupted change still being undone.                      | Retries once after 3 seconds, then shows it.                   |
| `needsUser` | Anything else.                                                                                                  | Shows it.                                                      |

### Binaries

| Binary                       | Purpose                                                                                                                                                                                |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `agent-plugins`              | The app. A recognized first argument runs a CLI verb headless instead (`cli.rs`).                                                                                                      |
| `agent-plugins-console`      | Windows installs it as `agent-plugins.com`, the console twin shells wait for; it runs the app with its console and returns the exit code. `windows/installer-hooks.nsh` puts it there. |
| `validate-source`            | Validates a source tree or archive. The marketplace server shells out to this binary.                                                                                                  |
| `validate-source-repository` | Validates a catalog document.                                                                                                                                                          |
| `generate-schema`            | Regenerates the checked-in JSON Schemas from the Rust types.                                                                                                                           |

The `app` feature is on by default. The `tools` feature builds the three tools above; it is off by default because the installers ship every binary cargo builds. `--no-default-features --features tools` builds them without Tauri, which is how the server image avoids GTK and WebKit.

## React

Presentation only. It validates every IPC response with Zod and owns no filesystem or manifest policy.

| File                                                       | Responsibility                                                                                     |
| ---------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| `App.tsx`                                                  | State, IPC calls, confirmation flow, and the header.                                               |
| `ipc/client.ts`                                            | The invoke wrapper, `toAppError`, and the one automatic retry.                                     |
| `ipc/schemas.ts`                                           | The Zod schema for every command result and for `IpcError`.                                        |
| `ipc/fixtures/app-state.json`                              | An `AppState` generated by a Rust test; the contract the schemas parse.                            |
| `lib/status.ts`                                            | Status labels and colors, action labels, and every confirmation dialog.                            |
| `lib/connectivity.ts`                                      | **Last checked**, the offline banner text, and the empty-page states.                              |
| `components/ErrorBoundary.tsx`                             | Replaces a crashed window with **Reload**.                                                         |
| `components/ItemCard.tsx`                                  | A package card, its notes and **More** menu, and its component rows.                               |
| `components/SourceGroup.tsx`                               | One source and its bulk actions.                                                                   |
| `components/SystemStatusDialog.tsx`                        | The preflight summary, agent details, and full check list.                                         |
| `components/ManageSourcesDialog.tsx`                       | Catalog sources, added sources, and sources the catalog dropped.                                   |
| `components/TeamsDialog.tsx`, `components/ShareDialog.tsx` | Teams (create, members, invite link) and who can see a space, package, or bundle.                  |
| `components/Bundles.tsx`                                   | The Bundles group and the New bundle dialog.                                                       |
| `components/LinkDialogs.tsx`                               | **Open a link** and the confirmation a website link opens.                                         |
| `components/ApprovalDialog.tsx`                            | **Allow connector?** and **Connector settings**: what each connector runs and the values it needs. |
| `components/AppsDialog.tsx`                                | **Which apps use it?**: the apps a connector is kept out of.                                       |
| `components/CatalogToolbar.tsx`                            | Search, the show and sort menus, the drift filter, the **Status** button, and the sync line.       |
| `components/AgentSetupNotice.tsx`, `components/Notice.tsx` | Callouts above the catalog, the offline banner, and error **Details**.                             |
| `*.test.ts`                                                | Vitest unit tests beside the module they cover.                                                    |

Adding a command means touching `ipc.rs`, `lib.rs`, and `src/ipc/schemas.ts` together. Changing `AppState` also means regenerating the fixture; see [how to work on Agent Plugins](development.md#change-the-ipc-surface).

## Server

| Path                            | Responsibility                                                                                                                                      |
| ------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src/Marketplace.Api/Endpoints` | Every HTTP endpoint, plus `PortalHosting`: the portal, `/downloads`, security headers.                                                              |
| `src/Marketplace.Api/Auth`      | Negotiate, and the Development-only `X-Dev-User` handler.                                                                                           |
| `src/Marketplace.Api/Packages`  | Publish, revoke, the public MCP gate, suggestions, bundles, validation via the Rust binary, SemVer, archive inspection, namespace archive building. |
| `src/Marketplace.Api/Teams`     | Teams, members, invite and share links, and the people-and-teams directory.                                                                         |
| `src/Marketplace.Api/Access`    | Who may see what: public, private, share lists, and the public MCP gate. Every read goes through it.                                                |
| `src/Marketplace.Api/Catalog`   | The `/api/catalog` document, one listed source per namespace.                                                                                       |
| `src/Marketplace.Api/Storage`   | Artifact Keeper client behind `IArtifactStore`.                                                                                                     |
| `src/Marketplace.Api/Events`    | Install, update, uninstall, and heartbeat ingestion.                                                                                                |
| `src/Marketplace.Api/Data`      | EF Core model and migrations, applied at startup.                                                                                                   |
| `tests/Marketplace.Api.Tests`   | Integration tests against a throwaway PostgreSQL container.                                                                                         |

## Web portal

`website/` is an Angular app (standalone components, signals, Angular Material) built into the server image. See [website/README.md](../website/README.md).

| Path                 | Responsibility                                                                  |
| -------------------- | ------------------------------------------------------------------------------- |
| `src/app/api.ts`     | Every API call; Zod schemas parse each response.                                |
| `src/app/session.ts` | Who is signed in, the development sign-in, and the admin route guard.           |
| `src/app/format.ts`  | Pure helpers: versions, IDs, SKILL.md frontmatter. Covered by `format.spec.ts`. |
| `src/app/pages/`     | One page per route, with its smaller parts in `*-parts.ts`.                     |
| `src/app/shared/`    | Components used across pages: icons, file viewer, dialogs, badges.              |

## Documentation

| Path                                                                                                                                                                                | Mode                       |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------- |
| `install-a-package.md`, `publish-to-marketplace.md`, `publish-source.md`, `publish-source-repository.md`                                                                            | Tutorials.                 |
| `troubleshoot.md`, `development.md`                                                                                                                                                 | How-to guides.             |
| `app-reference.md`, `cli-reference.md`, `manifest-reference.md`, `source-repository-reference.md`, `preflight-reference.md`, `adapter-contract.md`, `marketplace-api.md`, this page | Reference.                 |
| `architecture.md`, `native-extensions-evaluation.md`, `decisions/`                                                                                                                  | Explanation and decisions. |
