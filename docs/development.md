# How to work on Agent Plugins

Practical routes through this repository: getting it running, the verification loop, and the recurring changes — adding a target adapter, changing the manifest contract, touching the executor, and working on the server. For what each module is, see [the codebase map](codebase-map.md); for why the design is what it is, see [architecture](architecture.md).

## Set up and run

Install Rust, Node.js, pnpm, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/), then:

```bash
pnpm install
pnpm tauri dev
```

`pnpm install` also configures the tracked pre-commit hook, which runs `prettier --check` and `cargo fmt --check` before every commit.

The desktop app is the default `skill-manager` binary. Passing a CLI verb as the first argument runs headless instead, so `cargo run --bin skill-manager -- search` exercises the CLI without a window.

Work on the server needs the .NET 10 SDK and Docker; see [`server/README.md`](../server/README.md).

## Verify a change

Run what the change touched, and everything before you push:

```bash
pnpm typecheck                                                    # both tsconfigs
pnpm lint                                                         # eslint, zero warnings
pnpm format:check
pnpm build                                                        # typecheck + vite build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

There is no frontend test runner. React is held to `typecheck` and `lint`; the behavior worth testing lives in Rust, so a change that matters belongs in a Rust test.

Rust tests build real trees in `tempfile` directories and drive the actual planner, executor, and ledger. Copy the shape from `src-tauri/src/application/project.rs`: build a `SystemPaths` under a temp root, write a source tree, `read_manifest_catalog`, then assert on what the code produced.

## Run the app against throwaway state

A debug build honours `SKILL_MANAGER_QA_ROOT`, which relocates the home, config, data, and cache roots. It must be an absolute path strictly beneath the system temp directory — anything else is refused, so a mistake cannot point the app at a real home.

```bash
SKILL_MANAGER_QA_ROOT="$TMPDIR/agent-plugins-qa" pnpm tauri dev
```

Release builds ignore it. Use it whenever a change writes files: it is the only way to exercise install, drift, rollback, and reset without touching your own agents.

## Add or change a target adapter

An adapter is a pure planner. It may inspect a validated component and profile; it may not write files, start processes, change the ledger, download anything, or swallow an error.

1. Read [the adapter contract](adapter-contract.md). The conformance checklist there is the acceptance bar, not a suggestion.
2. Add the target and its pinned dialect to `adapters.rs` and the detection to `agent_profiles.rs`. Detection is the configuration set — an installed agent is configured, a removed one is dropped — so detection has to be conservative. Leftover editor config from an uninstalled IDE is not an install.
3. Return a capability result for every canonical component kind: `native`, `losslessTranslation`, `lossyTranslation`, `unsupported`, or `blocked`. A lossy result lists each lost semantic; unsupported and blocked carry an actionable reason.
4. Emit desired resources, and let the planner coalesce. Skills for every target except Claude Code share one `~/.agents/skills` directory; if your target needs its own copy, that is a deliberate decision to record, not a default.
5. Add fixtures for the desired resources and for coalescing, plus transaction and drift coverage for every resource type you emit.
6. Update the pinned matrix in [the adapter contract](adapter-contract.md) and the destinations table in [the app reference](app-reference.md).
7. Verify on the real client in a disposable home. Record the target version, the OS, the reload boundary, and whether your evidence is file-only or observed runtime discovery. A file on disk is not proof the agent loaded it.

## Change the source manifest contract

The Rust types are authoritative; the JSON Schemas are generated from them.

1. Change the types in `manifest.rs` (or `repository.rs` for the catalog document).
2. Regenerate the checked-in schemas:

   ```bash
   cargo run --manifest-path src-tauri/Cargo.toml --bin generate-schema
   ```

3. Update [the source manifest reference](manifest-reference.md) or [the source repository reference](source-repository-reference.md) in the same commit. Unknown fields are rejected, so anything undocumented is unusable by definition.
4. Check the server still agrees: `server/src/Marketplace.Api/Packages/PackageValidator.cs` shells out to `validate-source`, so a contract change reaches publish-time validation through that binary, not through a second implementation.

## Touch the executor

`executor/` is the only thing in the codebase that writes planned resources. Two rules hold everything else together:

- **Stage, then activate.** Every path and every fully rewritten document is staged, the journal is written, and only then is anything activated. Activation re-checks the original digest before it replaces anything.
- **All or nothing.** An activation or ledger error rolls the whole operation back. Bulk install, uninstall, and source removal share that boundary.

So a change here needs a test that fails in the middle and asserts the machine came back unchanged. Recovery is exercised through `executor/journal.rs`: a journal whose transaction ID is absent from the ledger is rolled back at launch, and one already committed is cleaned up.

If a change would let anything else write to a planned path, it is the wrong change.

## Add a preflight check

Checks are declarative and live in `preflight.rs`. Give the check a stable dotted ID in an existing group, a status rule, a remediation (`autoFixed`, an `action` the UI knows, or `manual` text), and a `blocking` flag.

Only make a check blocking when the app genuinely cannot operate — today only `host.homeDirs`, `agents.ledger`, and `server.clientVersion` qualify. An ID is never renamed: retire it and add a new one, because the report is sent with the heartbeat and read centrally.

A new `action` remediation needs a matching handler in `App.tsx` and a label in `SystemStatusDialog.tsx`, or the button will render its raw action name. Update [the preflight reference](preflight-reference.md) with the new row.

## Change the IPC surface

Commands live in `ipc.rs`, are registered in `lib.rs`, and are validated on the React side by a Zod schema in `src/ipc/schemas.ts`. All three change together; a command registered but unparsed fails at runtime, not at build time.

IPC returns plain data. Filesystem and manifest policy stays in Rust — React must not decide what is safe to write.

## Work on the marketplace server

```bash
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --bin validate-source
dotnet build server/Marketplace.sln
dotnet run --project server/tests/Marketplace.Api.Tests
```

`--no-default-features` drops the `app` feature so the validator builds without Tauri, GTK, or WebKit — that is what the server image does. The test suite starts PostgreSQL in a container and uses the real validator when `src-tauri/target/debug/validate-source` exists.

`docker compose -f server/compose.yaml up --build` runs the API in Development mode on `http://localhost:8080`, where `X-Dev-User` stands in for Windows authentication:

```bash
curl -H 'X-Dev-User: CORP\jacob' http://localhost:8080/api/me
```

After changing endpoints, regenerate `server/openapi.json` with the command in [`server/README.md`](../server/README.md) and update [the API reference](marketplace-api.md).

## Where documentation goes

| Change                                  | Also update                                                                                                                            |
| --------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| Target, dialect, or destination         | [adapter contract](adapter-contract.md), [app reference](app-reference.md)                                                             |
| Manifest or catalog field               | [manifest reference](manifest-reference.md) or [source repository reference](source-repository-reference.md), and the generated schema |
| Preflight check                         | [preflight reference](preflight-reference.md)                                                                                          |
| CLI command or flag                     | [CLI reference](cli-reference.md)                                                                                                      |
| Server endpoint                         | [marketplace API](marketplace-api.md) and `server/openapi.json`                                                                        |
| A user-visible state, button, or prompt | [app reference](app-reference.md), and [troubleshooting](troubleshoot.md) if it can fail                                               |
| A decision, not just a mechanism        | a new ADR under [`decisions/`](decisions/)                                                                                             |
