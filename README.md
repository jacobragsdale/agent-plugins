# Agent Plugins

Agent Plugins is a desktop app and an internal marketplace. People publish Agent Skills and MCP server configurations under their own namespace; anyone can find them, read them, and install them onto the coding agents on their machine: Cursor, Claude Code, Codex, OpenCode, Grok Build, and GitHub Copilot.

A **source** is an HTTPS archive with `agent-plugins.json` at its root. That file is the source manifest: it names the source and lists packages of skills and MCP servers. The **marketplace server** publishes one source per publisher namespace and a catalog that lists them; the app subscribes to that catalog, so every published package appears without configuration. Identity is the Windows logon. The server records installs and heartbeats so publishers see how many people use their packages.

The app plans the files and config each detected agent needs, shows compatibility and trust, then applies the change in one recovery journal and ownership-ledger commit. It never executes source content.

The `agent-plugins` command line (`validate`, `publish`, `search`, `install`, `whoami`) does the same work from a terminal or from an agent; the official `publish` and `marketplace` skills wrap it.

## Learn

- [Install your first package](docs/install-a-package.md) — from a fresh install to a skill your agent uses, and an MCP server after it.
- [Publish to the marketplace](docs/publish-to-marketplace.md) — publish a skill from your machine with the CLI or the `publish` skill.
- [Publish a source](docs/publish-source.md) — write a portable package by hand and publish it as a zip.
- [Publish a source repository](docs/publish-source-repository.md) — publish a browseable catalog outside the marketplace server.

## Do

- [Troubleshooting](docs/troubleshoot.md) — a refused install, a skill an agent cannot see, a red status button.
- [Work on Agent Plugins](docs/development.md) — set up, verify, add an adapter, change a contract, run the server.
- [Run the marketplace server](server/README.md) — configuration, Kerberos, Artifact Keeper, Docker.

## Look up

- [App reference](docs/app-reference.md) — window and tray controls, package states, destinations, background behavior.
- [CLI reference](docs/cli-reference.md) — `validate`, `publish`, `search`, `install`, `whoami`.
- [Marketplace API](docs/marketplace-api.md) — endpoints, events, and configuration; [`server/openapi.json`](server/openapi.json) is generated.
- [Preflight checks](docs/preflight-reference.md) — every startup check, its status rules, and remediation.
- [Source manifest](docs/manifest-reference.md) — `agent-plugins.json`, `SKILL.md`, and MCP document fields.
- [Source repository](docs/source-repository-reference.md) — catalog document, locators, and identity.
- [Target adapter contract](docs/adapter-contract.md) — pinned target mappings.
- [Codebase map](docs/codebase-map.md) — what lives where, in Rust, React, and the server.

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

Run local verification before pushing:

```bash
pnpm typecheck && pnpm lint && pnpm format:check && pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

`pnpm install` also configures the tracked pre-commit hook, which runs both formatting checks before each commit.

[How to work on Agent Plugins](docs/development.md) covers the rest: throwaway state with `AGENT_PLUGINS_QA_ROOT`, adding a target adapter, changing the manifest contract, regenerating the schemas, and building and testing the marketplace server in `server/`.
