# Agent Plugins

Agent Plugins is a desktop app and an internal marketplace. People publish Agent Skills and MCP server configurations under their own namespace; anyone can find them, read them, and install them onto the AI apps on their machine. The primary apps are GitHub Copilot, Cursor, and Claude (Claude Code, and Claude Desktop for MCP servers); OpenCode, pi, Codex, the ChatGPT app's Codex mode, and Grok Build are supported too.

A **source** is an HTTPS archive with `agent-plugins.json` at its root. That file is the source manifest: it names the source and lists packages of skills and MCP servers. The **marketplace server** publishes one source per publisher namespace and a catalog that lists them; the app subscribes to that catalog, so every published package appears without configuration. Identity is the Windows logon. The server records installs and heartbeats so publishers see how many people use their packages.

The app plans the files and config each detected agent needs, shows which apps get what and whether IT checked a connector, then applies the change in one recovery journal and ownership-ledger commit. It never executes source content.

The **web portal**, served by the marketplace, is where people browse skills and install them into the app with one click, download the app, write or upload their own skills, skill packs, and bundles, create teams, share with people, answer suggestions and feedback, and read their notifications. Admins review public MCP servers, see who has what, and read the audit log there.

The `agent-plugins` command line (`validate`, `publish`, `search`, `install`, `list`, `status`, `uninstall`, `sync`, `withdraw`, `hold`, `keep`, `share`, `team`, `bundle`, `review`, `revoke`, `whoami`) does the same work from a terminal or from an agent; the official `publish` and `marketplace` skills wrap it.

## Learn

- [Install your first package](docs/install-a-package.md) — from a fresh install to a skill your agent uses, and an MCP server after it.
- [Publish to the marketplace](docs/publish-to-marketplace.md) — publish a skill from your machine with the CLI or the `publish` skill.
- [Publish a source](docs/publish-source.md) — write a portable package by hand and publish it as a zip.
- [Publish a source repository](docs/publish-source-repository.md) — publish a browseable catalog outside the marketplace server.

## Do

- [Troubleshooting](docs/troubleshoot.md) — a refused install, a skill an agent cannot see, a connector that does not work, a red **Status** button, an offline window.
- [Work on Agent Plugins](docs/development.md) — set up, verify, add an adapter, change a contract, run the server.
- [Roll out Agent Plugins in a company](docs/rollout-guide.md) — Kerberos, a production server, fleet policy for the desktop app, backups, and incidents.
- [Run the marketplace server](server/README.md) — configuration, Kerberos, Artifact Keeper, Docker.
- [Work on the web portal](website/README.md) — the Angular dev loop against a local server.

## Look up

- [App reference](docs/app-reference.md) — window and tray controls, notices and retries, package states, destinations, background behavior.
- [CLI reference](docs/cli-reference.md) — every command, its exit codes, and its JSON output.
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
- [ADR 0005](docs/decisions/0005-marketplace-access-control.md) — per-user and per-team access.
- [ADR 0006](docs/decisions/0006-web-portal-and-review.md) — the web portal and review before publishing.
- [ADR 0007](docs/decisions/0007-self-service-marketplace.md) — self-service teams, sharing, owner review, bundles, and installing from the website.
- [ADR 0008](docs/decisions/0008-primary-targets-and-connector-choices.md) — the primary apps, connector choices, and approvals that follow changes.

## Develop

Requirements: Rust, Node.js, pnpm, and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
pnpm install
pnpm tauri dev
```

Run local verification before pushing:

```bash
pnpm typecheck && pnpm lint && pnpm test && pnpm format:check && pnpm build
pnpm --filter website lint && pnpm --filter website test && pnpm --filter website build
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo clippy --manifest-path src-tauri/Cargo.toml --no-default-features --features tools --bins -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-targets
```

`pnpm install` also configures the tracked pre-commit hook, which runs both formatting checks before each commit.

[How to work on Agent Plugins](docs/development.md) covers the rest: throwaway state with `AGENT_PLUGINS_QA_ROOT`, adding a target adapter, changing the manifest contract, regenerating the schemas, and building and testing the marketplace server in `server/`.
