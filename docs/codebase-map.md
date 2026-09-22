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

| Stage        | Modules                                                                         | Responsibility                                                                               |
| ------------ | ------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Acquire      | `locator.rs`, `artifact.rs`, `sources.rs`, `source.rs`, `repository.rs`         | Resolve an HTTPS locator, download, verify, extract safely, and cache a snapshot.            |
| Normalize    | `manifest.rs`, `catalog.rs`, `mcp.rs`, `digest.rs`                              | Parse the pinned manifest, materialize skill names, validate MCP documents, digest content.  |
| Plan         | `adapters.rs`, `agent_profiles.rs`, `planner.rs`, `resource.rs`                 | Detect agents, fan components across targets, coalesce identities, run structural preflight. |
| Execute      | `executor/`, `managed_documents.rs`, `fs_retry.rs`                              | Stage, journal, activate, and roll back. The only writer of planned resources.               |
| Own          | `ledger.rs`, `install.rs`                                                       | Record installations, bindings, and physical resources; derive item status.                  |
| Serve        | `application/`, `ipc.rs`, `app_state.rs`                                        | Sequence use cases behind locks and project state for the UI.                                |
| Reach out    | `marketplace.rs`, `host_identity.rs`, `preflight.rs`                            | Identity, marketplace requests, events, and the startup checks.                              |
| Host         | `paths.rs`, `startup.rs`, `process.rs`, `parallel.rs`, `qa_paths.rs`, `tray.rs` | Filesystem roots, environment repair, bounded subprocesses, QA isolation, tray.              |
| Entry points | `main.rs`, `lib.rs`, `cli.rs`, `staging.rs`, `bin/`                             | Window, command registration, headless verbs, publish staging, validator binaries.           |

### Modules worth knowing before you change anything

| Module                 | Note                                                                                                                                                                                         |
| ---------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `executor/mod.rs`      | The only filesystem writer for planned resources. `stage.rs` prepares, `journal.rs` recovers, `activate.rs` commits the ledger, `matching.rs` decides whether disk still matches the ledger. |
| `planner.rs`           | Pure. Fan-out, coalescing, structural preflight, and `requires_approval` — the Tier 3 gate.                                                                                                  |
| `ledger.rs`            | Ledger v4: installations, bindings, resources. A resource dies when its last consumer binding does.                                                                                          |
| `agent_profiles.rs`    | Detection is the configuration set. Results are cached for 60 seconds; a sync clears the cache.                                                                                              |
| `managed_documents.rs` | Comment-preserving JSONC and TOML edits. Why installing an MCP server does not destroy a user's own config.                                                                                  |
| `preflight.rs`         | Declarative checks with stable IDs. IDs are retired, never renamed.                                                                                                                          |
| `qa_paths.rs`          | Debug-only. `AGENT_PLUGINS_QA_ROOT` relocates every root beneath the temp directory.                                                                                                         |

### `application/`

The use-case layer. Mutations serialize on one operation lock; refresh uses a separate sync lock.

| File         | Responsibility                                                                                                     |
| ------------ | ------------------------------------------------------------------------------------------------------------------ |
| `mod.rs`     | `RuntimeState`, the locks, the 15-minute sync scheduler, and the re-exports `ipc.rs` calls.                        |
| `sync.rs`    | Cached load, on-demand preflight, full sync, catalog auto-subscribe, and background updates of installed packages. |
| `sources.rs` | Prepare, confirm, and cancel a source. Preparation stages a candidate; confirmation writes `sources.json`.         |
| `items.rs`   | Install, replace, uninstall, bulk plan and run, source removal, and app reset.                                     |
| `status.rs`  | Derives the item and component states the cards show.                                                              |
| `project.rs` | Builds `AppState` for the window: catalog items, compatibility, approval details, marketplace metadata.            |

### Binaries

| Binary                       | Purpose                                                                               |
| ---------------------------- | ------------------------------------------------------------------------------------- |
| `agent-plugins`              | The app. A recognized first argument runs a CLI verb headless instead (`cli.rs`).     |
| `validate-source`            | Validates a source tree or archive. The marketplace server shells out to this binary. |
| `validate-source-repository` | Validates a catalog document.                                                         |
| `generate-schema`            | Regenerates the checked-in JSON Schemas from the Rust types.                          |

The `app` feature is on by default. `--no-default-features` builds the validators without Tauri, which is how the server image avoids GTK and WebKit.

## React

Presentation only. It validates every IPC response with Zod and owns no filesystem or manifest policy.

| File                                                       | Responsibility                                                          |
| ---------------------------------------------------------- | ----------------------------------------------------------------------- |
| `App.tsx`                                                  | State, IPC calls, confirmation flow, and the header.                    |
| `ipc/client.ts`, `ipc/schemas.ts`                          | The invoke wrapper and the Zod schema for every command result.         |
| `lib/status.ts`                                            | Status labels and colors, action labels, and every confirmation dialog. |
| `components/ItemCard.tsx`                                  | A package card and its component rows.                                  |
| `components/SourceGroup.tsx`                               | One source and its bulk actions.                                        |
| `components/SystemStatusDialog.tsx`                        | The preflight summary, agent details, and full check list.              |
| `components/ManageSourcesDialog.tsx`                       | Catalog sources, added sources, and sources the catalog dropped.        |
| `components/CatalogToolbar.tsx`                            | Search, the drift filter, the status button, and the sync line.         |
| `components/AgentSetupNotice.tsx`, `components/Notice.tsx` | Callouts above the catalog.                                             |

Adding a command means touching `ipc.rs`, `lib.rs`, and `src/ipc/schemas.ts` together.

## Server

| Path                            | Responsibility                                                                                   |
| ------------------------------- | ------------------------------------------------------------------------------------------------ |
| `src/Marketplace.Api/Endpoints` | Every HTTP endpoint, plus `PortalHosting`: the portal, `/downloads`, security headers.           |
| `src/Marketplace.Api/Auth`      | Negotiate, and the Development-only `X-Dev-User` handler.                                        |
| `src/Marketplace.Api/Packages`  | Publish, validation via the Rust binary, SemVer, archive inspection, namespace archive building. |
| `src/Marketplace.Api/Catalog`   | The `/api/catalog` document, one listed source per namespace.                                    |
| `src/Marketplace.Api/Storage`   | Artifact Keeper client behind `IArtifactStore`.                                                  |
| `src/Marketplace.Api/Events`    | Install, update, uninstall, and heartbeat ingestion.                                             |
| `src/Marketplace.Api/Data`      | EF Core model and migrations, applied at startup.                                                |
| `tests/Marketplace.Api.Tests`   | Integration tests against a throwaway PostgreSQL container.                                      |

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
