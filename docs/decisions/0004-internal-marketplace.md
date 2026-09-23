# ADR 0004: Internal marketplace with a server, Windows identity, and metrics

- Status: accepted
- Date: 2026-08-28
- Supersedes: the anonymous build-time catalog fetch and the "no server" stance in [ADR 0003](0003-artifact-only-catalog.md). Artifact-only acquisition, `sourceKey` identity, and the manifest v2 package contract from ADR 0001–0003 remain.

## Context

Agent Plugins is a consumer of a curated feed. A build-time catalog lists a handful of sources, each published by a team's CI as a zip that overwrites `…-latest.zip` on the artifact host, and the client fetches everything anonymously. Nobody publishes from the app, nobody is identified, and the only discovery metadata is a name and a description.

The company wants an internal marketplace: any employee publishes a skill from their desktop, anyone finds it and reads it before trusting it, installing lands in every agent they use, and the company can see how many people use the app and each skill. The artifact host is now Artifact Keeper. Its RBAC is per repository, not per path, its download telemetry is admin-only and counts whichever principal fetched the blob, and it has no Kerberos support, so it cannot express "each user writes under their own namespace" and cannot identify Windows users on its own.

Clients run only on corporate Windows 11 virtual machines joined to Active Directory. A server is available. Expected scale is 50 users at launch, realistically 200, at most a few thousand. The primary publisher is a person at their desktop, and publishing should be possible from inside an agent session.

## Decision

### The marketplace server is the only endpoint the client talks to

A .NET 10 API (`server/`) runs in Docker and owns the index, identity, metrics, and publishing. Artifact Keeper stays behind the server as immutable blob storage reached with one service credential. Users never hold Artifact Keeper accounts, and the host can be replaced without touching clients.

The client keeps fetching HTTPS artifacts exactly as ADR 0003 describes; the URLs now point at the server. The build-time constant is the marketplace base URL. The server publishes the catalog at `/api/catalog` in the existing `agent-plugins-repository.json` shape and each namespace archive at `/api/sources/{namespace}/archive`. Locator canonicalization, `sourceKey`, digest revisions, and validator refresh are unchanged.

### Identity is the Windows logon

Requests carry `Authorization: Negotiate` produced by SSPI for the SPN `HTTP/<server FQDN>`; the server validates Kerberos with a keytab. A user's publishing namespace is the lowercase sAMAccountName. Authorization is server policy: a user always owns `{username}`; `official` is an allowlist; team namespaces keyed to AD groups are a follow-up.

**Amended 2026-09-22:** the lowercase sAMAccountName is only the derived name. Two accounts can derive the same one, so the first account to publish claims it and the other gets a numbered variant (`christopher-jo-2`); a derived name equal to `official` or a team namespace becomes `u-<name>`. The server settles the namespace and returns it from `/api/me`; clients never derive it. [The API reference](../marketplace-api.md#personal-namespaces) has the rules.

Negotiate on a Linux container validates Kerberos only. A client that reaches the server by IP address or short hostname falls back to NTLM and is rejected, so the preflight requires the configured base URL to be the FQDN named in the SPN, and clock skew above five minutes is a preflight failure.

A `DevHeader` scheme that trusts `X-Dev-User` exists for the home lab, whose test VM is not domain-joined. The server registers it only when `ASPNETCORE_ENVIRONMENT=Development` or `Auth:AllowDevHeader` is set; the corporate server does neither and ignores the header, so a client that sends it is harmless. On a workgroup machine the client sends the header with its local account name; on a domain-joined machine it sends a Kerberos token.

### A person publishes a package; the server materializes a namespace source

The unit a person publishes is one manifest v2 package: a skill or an MCP document plus its metadata. The server validates it, stores each version immutably in Artifact Keeper under `marketplace/{namespace}/{package}/{version}.zip`, and regenerates that namespace's source archive: one zip whose `agent-plugins.json` carries `source.id = namespace` and the latest version of every non-yanked package. The client therefore sees each publisher as one source and each of their skills as one package, which is the existing catalog model.

`source.id` in an uploaded manifest must equal the authenticated namespace; the server rejects a mismatch. Authorship is stamped from identity, never self-declared.

Version pins, rollback in the client, and per-package archives are follow-ups on a v3 client contract. Server-side versions are immutable now so that contract has history to bind to.

### The marketplace catalog is subscribed, not browsed one source at a time

Catalog v1 gains optional listing fields (`publisher`, `packageCount`, `updatedAt`). The client auto-adds every source listed by the marketplace catalog on sync, so packages from every namespace appear without a Manage Sources step. Removing a marketplace source locally hides it until the next sync; the catalog is the authority.

`GET /api/index` returns per-package marketplace metadata: publisher display name, version, tags, published date, install count, and installed base. The client joins it by canonical package ID and shows it on cards. Search is client-side over that index, which is adequate to several thousand packages.

### Metrics come from client events, not download logs

After every committed transaction and on each scheduled sync the client posts events to `/api/events`: `heartbeat` (app version, OS build, detected agents, installed package set) and `install`, `update`, `uninstall`. Publishes are recorded by the server itself. The server derives daily, weekly, and monthly active users, per-package installs over time, current installed base, agent mix, and client version adoption. Publishers see the numbers for their packages in the app; administrators see the fleet.

Usage is recorded under the Windows identity. The app says so in its status panel. Events carry package identities and agent names, never project paths or file contents.

### Publishing runs through a CLI that the official `publish` skill wraps

The application binary gains `validate`, `publish`, `search`, `install`, and `whoami` subcommands. They reuse the crate's validator, locator, and Windows identity, so a CLI launched by an agent authenticates without a browser. The CLI refuses secrets and `.env` files in the tree, oversized bundles, and a namespace that is not the caller's.

An official `publish` skill instructs the agent to locate the skill directory, run `validate`, tighten the description, propose tags and a version, and run `publish`. The agent does the judgment; the CLI does the trust-sensitive part. Agent Plugins itself still never executes source content; the agent executes the skill.

The server re-validates every upload with the same Rust `validate-source` binary, built without the desktop feature (no Tauri, GTK, or WebKit) inside the server image. There is one validator.

### Startup is a visible preflight

Startup runs declarative checks for the Windows host, authentication, server reachability, detected agents, and dependencies. Each check has a stable ID, a status, a remediation, and a blocking flag. The report is shown in the app, can be rerun, and is included in the heartbeat so fleet problems are visible without tickets. [The preflight reference](../preflight-reference.md) lists the checks.

Only an unreadable ledger, no writable home directory, and a client below the server's minimum version block the app. Everything else degrades: an unreachable server leaves the cached catalog and installed packages usable with publishing and statistics disabled.

## Consequences

The client contract changes are small: an auth header on marketplace URLs, optional catalog fields, auto-subscription, an index join, event posting, a preflight IPC, and CLI subcommands. The planner, adapters, executor, journal, and ledger are untouched.

The server is a new component with its own CI, image, Postgres database, keytab, and Artifact Keeper service credential. Artifact Keeper's own telemetry is not used.

Kerberos cannot be exercised in the home lab, which has no domain. The `DevHeader` scheme covers end-to-end testing there; the Negotiate path is verified for token production on Windows and validated against a domain during the corporate migration.

One archive per namespace means a publisher with many packages republishes the whole namespace on every publish. At the expected scale that is a few hundred kilobytes per publish and one `HEAD` per namespace per sync.

The home-lab deployment is `marketplace.ragsdale.dev`; the corporate deployment changes the base URL, the SPN, the keytab, and the Artifact Keeper endpoint, and nothing else.
