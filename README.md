# Agent Plugins

Agent Plugins is a desktop app and an internal marketplace. People publish Agent Skills and MCP server configurations under their own namespace; anyone can find them, read them, and install them onto the coding agents on their machine: Cursor, Claude Code, Codex, OpenCode, Grok Build, and GitHub Copilot.

A **source** is an HTTPS archive with `skill-manager.json` at its root. That file is the source manifest: it names the source and lists packages of skills and MCP servers. The **marketplace server** publishes one source per publisher namespace and a catalog that lists them; the app subscribes to that catalog, so every published package appears without configuration. Identity is the Windows logon. The server records installs and heartbeats so publishers see how many people use their packages.

The app plans the files and config each enabled agent needs, shows compatibility and trust, then applies the change in one recovery journal and ownership-ledger commit. It never executes source content.

The `skill-manager` command line (`validate`, `publish`, `search`, `install`, `whoami`) does the same work from a terminal or from an agent; the official `publish` and `marketplace` skills wrap it.

## Learn

- [Publish to the marketplace](docs/publish-to-marketplace.md) — publish a skill from your machine with the CLI or the `publish` skill.
- [Publish a source](docs/publish-source.md) — write a portable package by hand and publish it as a zip.
- [Publish a source repository](docs/publish-source-repository.md) — publish a browseable catalog.
- [Run the marketplace server](server/README.md) — configuration, Kerberos, Artifact Keeper, Docker.

## Look up

- [Marketplace API](docs/marketplace-api.md) — endpoints, events, and configuration; [`server/openapi.json`](server/openapi.json) is generated.
- [Preflight checks](docs/preflight-reference.md) — every startup check, its status rules, and remediation.
- [Source manifest](docs/manifest-reference.md) — `skill-manager.json`, `SKILL.md`, and MCP document fields.
- [Source repository](docs/source-repository-reference.md) — catalog document, locators, and identity.
- [Target adapter contract](docs/adapter-contract.md) — pinned target mappings.

## Understand

- [Architecture](docs/architecture.md) — desired resources, transactions, ownership, trust, and the marketplace.
- [Native extension evaluation](docs/native-extensions-evaluation.md) — why hooks and in-process plugins stay out of the portable contract.
- [ADR 0001](docs/decisions/0001-multi-agent-desired-state.md) — product and migration decisions.
- [ADR 0002](docs/decisions/0002-source-repositories-and-locators.md) — catalogs and locators.
- [ADR 0003](docs/decisions/0003-artifact-only-catalog.md) — artifact-only distribution.
- [ADR 0004](docs/decisions/0004-internal-marketplace.md) — the marketplace server, Windows identity, metrics, and publishing.

## Develop

Requirements: Rust, Node.js, pnpm, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
pnpm install
pnpm tauri dev
```

Run local verification:

```bash
pnpm typecheck
pnpm lint
pnpm format:check
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

`pnpm install` also configures the tracked pre-commit hook, which runs both formatting checks before each commit.

Regenerate the checked-in source and source-repository schema paths after changing the Rust contract:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin generate-schema
```

The marketplace server lives in `server/` (.NET 10). It needs the .NET 10 SDK and Docker (the integration tests start PostgreSQL in a container and use the Rust validator when `src-tauri/target/debug/validate-source` exists):

```bash
cargo build --manifest-path src-tauri/Cargo.toml --no-default-features --bin validate-source
dotnet build server/Marketplace.sln
dotnet run --project server/tests/Marketplace.Api.Tests
docker build -f server/Dockerfile -t marketplace-api .
```

`docker compose -f server/compose.yaml up --build` runs the API in Development mode on `http://localhost:8080` with the `X-Dev-User` header standing in for Windows authentication.
