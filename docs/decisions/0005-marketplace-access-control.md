# ADR 0005: Per-user and per-team access on the marketplace

- Status: accepted
- Date: 2026-09-05
- Extends: [ADR 0004](0004-internal-marketplace.md), which left team namespaces keyed to AD groups as a follow-up

## Context

Every authenticated user sees and can install every package, and publishing is per-person plus the `official` allowlist. Teams want private sources and single packages (a skill or an MCP server) visible only to named people and AD groups, and a shared namespace their group publishes to. Clients are domain-joined Windows machines; the server already validates the Windows logon and can resolve AD groups over LDAP.

## Decision

### Enforcement is on the server; hidden means 404

The server filters the catalog, the index, each namespace archive, and the package endpoints per caller. Anything the caller may not see answers `404`, never `403`. The desktop app already treats a 404 archive as gone and a package missing from the next snapshot as Removed Upstream, so revocation reaches every client through existing behaviour. The one client change is that a gone source with installed packages now drops its cached snapshot instead of keeping the whole namespace installable from cache.

### One allowlist per target, most specific wins

An access rule is one row per target, `ns` or `ns/packageId`, holding `users[]` and `groups[]`. No row means public, which is today's behaviour. A package rule replaces its namespace rule. Owners and admins always see their own namespaces. Users match by username so `CORP\jane`, `jane@corp.example`, and `jane` are one person; groups are the AD group CNs the LDAP adapter emits, compared case-insensitively. Group membership is managed in AD; the marketplace stores only the mapping.

There is no deny rule, no category gate on MCP servers, and no per-component rule: marketplace packages are one skill or one MCP document each, so package granularity is component granularity.

### The owner or an admin manages the list

`GET` and `PUT /api/access/{ns}/{packageId?}` require `Owns(ns)`, the same predicate that gates publish and yank. The CLI verb `agent-plugins access` wraps them with the caller's Windows identity, the way `publish` does. There is no in-app editor; the card shows a Restricted badge.

### A filtered archive is derived from the stored one

The full namespace archive stays the one stored artifact. A caller who may see a subset gets `NamespaceArchiveBuilder.Filter` applied to it: the manifest's `packages[]` is reduced and only entries under kept package directories are copied, with the same fixed timestamps, so one subset has one digest. `ETag` is that digest and `Last-Modified` is the full archive's, which is what the client's validator check needs. It is rebuilt per request; a cache keyed by (namespace, digest, kept ids) is the upgrade if profiles ask for it.

### Team namespaces are configuration

`Auth:TeamNamespaces` lists `{ namespace, group, displayName }`. Members of the group own the namespace for publish, yank, and access management, and the index reports lane `team`. Who may read a team namespace is an ordinary access rule, because publisher and reader groups usually differ. The `team-` prefix is a convention, not enforced.

## Consequences

The catalog and archive are per caller, so `Cache-Control` is `private`. The catalog, index, and archive each cost one extra query for live packages plus one small rules table read; at the expected few hundred users this is negligible. An `X-Dev-Groups` header, sent by the client from `AGENT_PLUGINS_DEV_GROUPS`, lets the home lab exercise team rules without a domain; the production server ignores it. Kerberos and LDAP group resolution can only be verified against the corporate domain; an empty `groups:` line in `agent-plugins whoami` is the signal that LDAP claims are not arriving.
