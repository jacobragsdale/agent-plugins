# Marketplace API reference

The marketplace server is the only endpoint the desktop app and CLI talk to. It is a .NET 10 API in `server/`, backed by PostgreSQL for the index and metrics and by Artifact Keeper for immutable package archives. [ADR 0004](decisions/0004-internal-marketplace.md) records the decision, and [ADR 0007](decisions/0007-self-service-marketplace.md) the teams, sharing, and review model. The generated OpenAPI document is [`server/openapi.json`](../server/openapi.json).

The same server serves the web portal at `/` and the installers at `/downloads` ([ADR 0006](decisions/0006-web-portal-and-review.md)). Those need no sign-in; an unknown `/api/*` route is still a `404` problem document.

JSON is camelCase and timestamps are ISO 8601 UTC. An `account` is a Windows account such as `CORP\jane`; a `displayName` falls back to the account's username.

## Authentication

Every endpoint except `GET /api/health` and the OpenAPI document at `/openapi/v1.json` requires an authenticated principal.

| Scheme      | When                                          | Header                                                 |
| ----------- | --------------------------------------------- | ------------------------------------------------------ |
| `Negotiate` | Production. Kerberos validated with a keytab. | `Authorization: Negotiate <token>`                     |
| `DevHeader` | Development, or `Auth:AllowDevHeader`.        | `X-Dev-User: <username>`, optional `X-Dev-Groups: a,b` |

`Auth:AllowDevHeader` beside Negotiate outside Development logs a startup warning: anyone who can reach the server can claim any account. The server still starts, because the home lab runs that way on purpose.

Group claims come from LDAP when `Auth:LdapDomain` is configured; the claim value is the AD group's CN, and every group comparison is case-insensitive. Groups are optional: they grant admin through `Auth:AdminGroup` and can appear in share lists, but teams never depend on them. LDAP binds as `Auth:LdapMachineAccountName` when set, and otherwise with the Kerberos client credentials in `KRB5_CLIENT_KTNAME` or the ticket cache; `GET /api/health` reports whether that bind works.

An `Auth:AdminAccounts` or `Auth:OfficialPublishers` entry with a domain (`CORP\jane`, `jane@corp.example`) matches that account only; a bare `jane` matches any account whose username is `jane`.

### Blocked accounts

An admin can [block](#admin-blocks) an account. A blocked account may still read, report its heartbeat, and mark notifications read, but every other `POST`, `PUT`, or `DELETE` answers `403` ("Your account is blocked from changing anything in the marketplace. Contact the marketplace admins."). Its personal namespace is hidden from everyone but admins and itself, so it leaves the catalog of every other PC.

### Client versions and rate limits

A request whose `User-Agent` is `agent-plugins/<major.minor.patch>` below `Client:MinimumVersion` answers `426` on every authenticated route except `GET /api/me` and `POST /api/events`, which stay open so the account and its old version still show up for admins:

```json
{
  "title": "Update Agent Plugins",
  "status": 426,
  "detail": "This version of Agent Plugins (0.1.9) is too old for the marketplace. Download 0.3.0 from https://marketplace.corp.example/#download.",
  "downloadUrl": "https://marketplace.corp.example/#download"
}
```

Writes are limited per account: publishes and suggestions to `RateLimits:UploadsPerHour`, every other `POST`, `PUT`, and `DELETE` to `RateLimits:WritesPerMinute`. Reads are never limited. Past a limit the answer is `429` with `Retry-After` in seconds and the title "Too many changes in a short time. Wait 42 seconds and try again."

### Personal namespaces

The principal's personal namespace is derived from its sAMAccountName: lowercased, each run of other characters replaced by one hyphen, prefixed `u-` when it does not start with a letter, and cut to 16 characters. A name with no ASCII letter or digit (`иван`) becomes `u-` plus the first 8 hex digits of the SHA-256 of the lowercased name. A derived name equal to `official` or to a team becomes `u-<name>` (`u-official`).

Two accounts can derive the same name: `christopher.johnson` and `christopher.johnston` both derive `christopher-john`. The first account to publish claims it. Any other account gets the first of `<first 14 characters>-2` through `-9` that it has claimed or that is still free (`christopher-jo-2`); when all are taken, the account has no personal namespace. Clients read `namespace` from [`GET /api/me`](#get-apime) and never derive it locally.

### Teams

Anyone can create a team: a namespace whose **members** publish, withdraw, share, revoke, and edit bundles there. **Owners** also add and remove members, hand out and reset the invite link (joining makes someone a publisher), rename the team, set who sees the team's space, and delete a team that never published. The last owner cannot leave, be removed, or be demoted. Admins can do what owners can, and also delete a team that has published: its packages are revoked so every PC removes them, and the namespace stays reserved.

A team member entry with a domain (`CORP\jane`, `jane@corp.example`) matches that account only; a bare `jane` matches any account whose username is `jane`. People join through the team's invite link or are added by an owner.

A team's namespace is permanent, because installed skill names are built from it; its display name can change. Its first-publish lane is `team`.

### Visibility and sharing

A namespace (a space) is `public` or `private`. A package or bundle is `inherit`, `public`, or `private`; `inherit` follows its space. The most specific setting wins.

**Private** means the space's owners (the person, or every member of the team) plus the share list: accounts, teams, and AD groups. A package or bundle that inherits a private space is shared with the space's list and its own. A package or bundle set explicitly to private uses only its own list. A public target ignores its list, which is kept for when it becomes private again. Share entries for accounts match by username, so `CORP\jane`, `jane@corp.example`, and `jane` name the same person.

Everything the caller may not see answers `404`, never `403`, so the desktop app treats it as gone. Owners and admins always see their own spaces. Sharing grants seeing and installing only; publishing needs membership.

### Links

A link is `https://<Server:PublicBaseUrl>/l/<code>`: 22 characters of base64url for 128 random bits. It is reusable and never expires. It is created the first time someone asks for it; a reset replaces it and the old one stops working.

| Kind     | Target                                       | Redeeming it                                                                          |
| -------- | -------------------------------------------- | ------------------------------------------------------------------------------------- |
| `invite` | A team (`ns`)                                | Makes the caller a member.                                                            |
| `share`  | A space (`ns`), package, or bundle (`ns/id`) | Adds the caller's account to the target's share list, unless they can already see it. |

Redeeming twice changes nothing. A reset does not remove anyone a link already added.

### Public MCP servers

Every publish goes live at once. The one gate: a package whose live version has an MCP server is visible only to its space's owners and admins until an admin approves it, when it can reach as many people as a public package can. That is when its effective visibility is public, or private with an AD group on its share list or a share link, which anyone can forward. A package shared privately with named people and teams needs no approval.

An approval covers what the servers launch: each server's name, transport, command, arguments, and URL. A later version that launches the same things stays approved; one that launches anything else waits again, and the publish answers with a warning. A decline carries a note for the owners, and the next publish clears it. An admin's own publish is approved automatically.

### Suggestions and revocation

Someone who can see a package but does not own its space may **suggest** a change: an upload that waits for the package's owners, who accept it as their next version (credited to the suggester) or decline it with a note. Owners and admins may **revoke** a package: it leaves the catalog, the index, and the namespace archive, and clients uninstall it at their next sync, provided the app is running (it starts with Windows unless the person or a policy turned that off). Revoking can be undone, but clients do not reinstall on their own. A package an admin revoked can be restored only by an admin, and its owners are told.

### Notifications

People are told, in the portal and by the desktop app, when someone suggests a change to their package or decides on their suggestion, when an admin decides on their MCP server or revokes their package, when someone reports a problem or leaves feedback on their package or resolves their report, and when they are added to a team or a share list by name. Admins can also have problem reports, and publishes that put a package in the MCP review queue, posted to `Notifications:WebhookUrl`.

## Endpoints

### `GET /api/health`

Anonymous. Returns server version, client version policy, and what the server depends on.

```json
{
  "serverVersion": "0.3.0",
  "minimumClientVersion": "0.2.0",
  "latestClientVersion": "0.3.0",
  "environment": "Production",
  "authSchemes": ["Negotiate"],
  "adGroups": true,
  "artifactStore": "ok",
  "ldap": "ok"
}
```

`authSchemes` lets the preflight report `warn` instead of `fail` for a workgroup machine talking to a development server. `adGroups` is true when `Auth:LdapDomain` is set, so clients offer AD groups in share lists only where they resolve. `artifactStore` is `ok` when Artifact Keeper accepts the service credential and `error` otherwise; `ldap` is `ok` when a bind to `Auth:LdapDomain` works, `error` when it fails, and `off` without a domain. Both are checked at most once a minute. The answer is `503` with a problem document only when the database is unreachable, so a container health check keeps the server up while Artifact Keeper or LDAP is down.

### `GET /api/me`

Returns the caller's identity, teams, and desktop app.

```json
{
  "account": "CORP\\jacob",
  "namespace": "jacob",
  "displayName": "Jacob Ragsdale",
  "namespaces": ["jacob", "official", "data-team"],
  "admin": false,
  "groups": [],
  "teams": [{ "namespace": "data-team", "displayName": "Data Team", "owner": true }],
  "suggestionsWaiting": 2,
  "app": { "version": "0.2.0", "os": "10.0.26100", "lastSeenAt": "2026-09-26T20:00:00Z", "installed": ["jacob/review"] },
  "reportsWaiting": 1,
  "unreadNotifications": 3
}
```

`namespace` is the caller's settled [personal namespace](#personal-namespaces); `namespaces` adds `official` when allowlisted and every team the caller belongs to. `suggestionsWaiting` counts pending suggestions, and `reportsWaiting` open [reports and feedback](#reports-and-feedback), on packages in those namespaces. `unreadNotifications` counts [notifications](#notifications-1) not yet marked read. `app` is the account's most recently heard-from PC, `null` when none has sent a heartbeat for 30 days; `installed` is that heartbeat's installed packages with later install and uninstall events applied.

### `GET /api/catalog`

Returns an `agent-plugins-repository.json` document. Each listed source is one namespace with at least one live package the caller may see; `packageCount` counts only those. The listing carries the optional marketplace fields `publisher`, `packageCount`, `updatedAt`, and `digest`: exactly the `ETag` value, without quotes, that [the namespace archive](#get-apisourcesnamespacearchive) answers this caller with, so a client whose cached copy has that validator needs no request for it. `ETag` is the digest of the document, and the response is compressed when the client accepts it.

### `GET /api/sources/{namespace}/archive`

Returns the namespace's current source archive: a zip whose root `agent-plugins.json` has `source.id` equal to the namespace and one package per live version. Supports `HEAD`, `ETag`, and `Last-Modified`.

A caller who may see only some of the namespace's packages receives an archive holding just those. Its `ETag` is a digest of the full archive's digest and the visible package ids, stable for the same subset of the same archive; `Last-Modified` is the full archive's. A caller who may see none answers `404`.

### `GET /api/index`

Per-package and per-bundle marketplace metadata joined by canonical ID. Served with a strong `ETag` (`If-None-Match` answers `304`) and compressed when the client accepts it.

```json
{
  "generatedAt": "2026-08-28T20:00:00Z",
  "packages": [
    {
      "id": "jacob/review",
      "namespace": "jacob",
      "packageId": "review",
      "name": "Review workflow",
      "description": "Reviews a change before it is submitted.",
      "version": "1.2.0",
      "publisher": { "account": "CORP\\jacob", "displayName": "Jacob Ragsdale" },
      "lane": "personal",
      "tags": ["review", "git"],
      "componentKinds": ["skill"],
      "publishedAt": "2026-08-28T19:12:03Z",
      "installs": 34,
      "installedBase": 12,
      "restricted": false,
      "sharedWithYou": false,
      "createdAt": "2026-08-01T09:30:00Z",
      "changelog": "Handles merge commits.",
      "mcpApproved": null,
      "mcpTransports": []
    }
  ],
  "bundles": [
    {
      "id": "data-team/starter",
      "namespace": "data-team",
      "bundleId": "starter",
      "name": "Starter kit",
      "description": "",
      "publisher": { "account": "data-team", "displayName": "Data Team" },
      "lane": "team",
      "members": ["jacob/review", "data-team/sql"],
      "updatedAt": "2026-09-26T18:00:00Z",
      "restricted": false,
      "sharedWithYou": false
    }
  ],
  "revoked": ["jacob/old-thing"]
}
```

`generatedAt` is when the listed content last changed, so the same content is the same bytes and `ETag`. Only packages and bundles the caller may see are listed. `lane` is `official`, `team`, or `personal`. `publisher.account` is the account that claimed a personal namespace, and the namespace itself for the other lanes. `restricted` is true when the effective visibility is private; `sharedWithYou` when the caller sees it only because a share list names them. `installs` counts accounts that ever installed it; `installedBase` counts accounts with a PC whose heartbeat in the last 30 days includes the package. `createdAt` is when the package was first published, and `changelog` the live version's. `mcpApproved` is `null` when the live version has no MCP server, and otherwise whether an admin approved it for the public. `mcpTransports` lists the live version's MCP transports (`stdio`, `streamable-http`, `sse`), empty when it has none or was published before they were recorded.

A bundle's `members` are the ones the caller can see; a bundle with none is left out. `revoked` lists revoked packages the caller could see, plus any in the caller's latest heartbeat, so a client that lost access still uninstalls them.

### `GET /api/packages/{namespace}/{packageId}`

The package and its versions, newest first.

```json
{
  "id": "jacob/review",
  "namespace": "jacob",
  "packageId": "review",
  "name": "Review workflow",
  "description": "Reviews a change before it is submitted.",
  "tags": ["review"],
  "liveVersion": "1.2.0",
  "versions": [
    {
      "version": "1.2.0",
      "archiveDigest": "…",
      "sizeBytes": 4096,
      "publishedBy": "CORP\\jacob",
      "publishedAt": "…",
      "changelog": null,
      "yanked": false,
      "componentKinds": ["skill", "mcpServer"],
      "purged": false
    }
  ],
  "owned": true,
  "visibility": "inherit",
  "effective": "public",
  "sharedWithYou": false,
  "revoked": false,
  "publicReview": { "state": "waiting", "note": null },
  "revokedByAdmin": false,
  "createdAt": "…",
  "publisher": { "account": "CORP\\jacob", "displayName": "Jacob Ragsdale" },
  "lane": "personal",
  "installs": 34,
  "installedBase": 12,
  "mcpServers": [{ "name": "database", "transport": "stdio", "command": "uvx", "args": ["acme-db-mcp"], "envNames": ["DB_TOKEN"], "url": null, "headerNames": [] }]
}
```

`liveVersion` is the highest version not withdrawn, and `null` for a revoked package. `owned` is true for the space's owners and admins. `visibility` is the package's own setting and `effective` the one that applies. `publicReview` is `null` unless the live version has an MCP server; `state` is `approved`, `declined` (with the admin's `note`), or `waiting`. `revokedByAdmin` is true when an admin revoked it, so only an admin may restore it. `publisher`, `lane`, `installs`, and `installedBase` are as in [the index](#get-apiindex). `mcpServers` describes what each MCP server in the live version runs or connects to, with environment variable and header names but never their values; it is empty for a version without one, or published before they were recorded. A `purged` version has no files any more. `404` when the caller may not see the package; a revoked package is visible to its owners and admins only.

### `GET /api/packages/{namespace}/{packageId}/versions/{version}/archive`

The version's stored zip, as published. `Content-Disposition: attachment; filename="{namespace}-{packageId}-{version}.zip"`. Supports `HEAD`; `ETag` is the strong archive digest and `If-None-Match` answers `304`. Readable by owners and admins for any version that was not [purged](#delete-apiadminpackagesnamespacepackageidversionsversion), and by others only for live versions they may see; otherwise `404`.

### `GET /api/packages/{namespace}/{packageId}/versions/{version}/files`

The version's files as `[{ "path": "skills/review/SKILL.md", "size": 812 }]`, relative to the source root. `GET …/files/{path}` returns one file: UTF-8 text up to 1 MB as `text/plain; charset=utf-8`, anything else (binary, or larger than 1 MB) as an `application/octet-stream` attachment. Readable by owners and admins, and by others only for live versions they may see.

### `GET /api/mine`

What the portal's My skills page shows.

```json
{
  "spaces": [{ "namespace": "jacob", "displayName": "Jacob Ragsdale", "lane": "personal", "visibility": "public", "role": "owner" }],
  "packages": [],
  "bundles": [],
  "suggestions": { "waiting": [], "yours": [] }
}
```

`spaces` are the namespaces the caller may publish to; `role` is `owner` or `member` (a person owns their own space; admins own `official`), and `visibility` is the space's own, so a portal can say whether "my space" is public. `packages` are in the [package shape](#get-apipackagesnamespacepackageid) and include revoked ones. `bundles` are [bundle documents](#bundles). `suggestions.waiting` are pending [suggestions](#suggestions) on the caller's packages, oldest first; `suggestions.yours` are the caller's latest 50, newest first.

### `POST /api/packages/{namespace}/{packageId}/versions`

Publishes a version. Multipart form: `version`, optional `tags` (comma separated), optional `changelog` (text), optional `visibility`, optional `dryRun`, and the content as one of:

- `archive`: a zip. With `agent-plugins.json` at its root it must declare exactly one package; without one, its contents are wrapped as below.
- `files` with one `paths` value each (relative, forward slashes): a skill directory (`SKILL.md` at the root), a folder of skill directories (a skill pack), one MCP document (`.json`), or a source tree. A single top-level folder shared by every path is dropped. Optional `name` and `description` label a wrapped package.

| Field        | Rule                                                                                                                                                                                                                       |
| ------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `version`    | A release version, `major.minor.patch`. A pre-release such as `1.0.0-beta.1` is refused.                                                                                                                                   |
| `tags`       | At most 10, each up to 32 lowercase letters, digits, and single hyphens. Lowercased and deduplicated; an invalid tag is refused by name.                                                                                   |
| `changelog`  | At most 4,096 characters.                                                                                                                                                                                                  |
| `visibility` | `inherit`, `public`, or `private`: becomes the package's own rule in the same transaction, so a private first version is never public, even briefly. Left out, an existing rule stays and a new package follows its space. |
| `dryRun`     | `true` runs every check below, the namespace size included, and stores nothing. See [dry run](#dry-run).                                                                                                                   |

Wrapping is `validate-source stage`, the same code as `agent-plugins publish`. Server checks, in order:

1. The upload is within 50 MB (`413`).
2. The caller owns `{namespace}` (`403`).
3. The upload is free of unsafe paths, and `version`, `tags`, and `changelog` follow the rules above (`422`).
4. `agent-plugins.json` has `source.id == namespace` and exactly one package whose `id == packageId` (`422`).
5. The Rust validator, with its credential scan, reports no errors (`422` with the validator messages).
6. The version was never published before; withdrawn numbers stay used. The same version with the same archive digest is a retry, not a conflict: it answers `200` with the version already published. Otherwise `409` suggests the patch after the highest used version ("Publish 1.0.1 or later.") and carries it as `suggestedVersion`.
7. The package is not revoked (`409`; restore it first, or ask an admin when an admin revoked it).
8. The version is not lower than the live one, which no PC would ever get (`409` with `suggestedVersion`).
9. A new package id is not a bundle in the namespace (`409`).
10. A personal namespace nobody has published to yet is created only by its owner, not by an admin publishing on their behalf (`403`).
11. Every live package in the namespace together still fits what the desktop app downloads from one source: 50 MB zipped, 2,000 files, and 50 MB unzipped (`422`, naming the totals). One package must fit the same numbers.

On success the version is stored in Artifact Keeper, goes live, and regenerates the namespace archive. The response is `201`:

```json
{
  "id": "jacob/review",
  "namespace": "jacob",
  "packageId": "review",
  "version": "1.2.0",
  "archiveDigest": "…",
  "sizeBytes": 4096,
  "publishedBy": "CORP\\jacob",
  "publishedAt": "…",
  "componentKinds": ["skill", "mcpServer"],
  "tags": ["review"],
  "waitingForPublicReview": true,
  "warnings": ["Only its owners see Review workflow until an admin approves its MCP server; PCs that have it keep the version they have."]
}
```

`waitingForPublicReview` is true when the package's [MCP server waits for an admin](#public-mcp-servers), so only its space's owners see it yet. `warnings` are sentences to show the publisher, such as who stops seeing the package meanwhile or that a changed server needs approval again. The first publish to a namespace claims it: a personal namespace for the publishing account, `official` for the namespace itself.

#### Dry run

With `dryRun=true` the answer is `200` and nothing is stored or claimed:

```json
{ "version": "1.3.0", "componentKinds": ["skill"], "sizeBytes": 4096, "warnings": [], "files": [{ "path": "skills/review/SKILL.md", "status": "changed" }] }
```

`files` compares the upload with the live version: `new`, `changed`, `removed`, or `same`. Every error a real publish would answer, it answers the same way.

### `PUT` and `DELETE /api/packages/{namespace}/{packageId}/versions/{version}/yank`

`PUT` withdraws a version: it leaves the namespace archive, the catalog, and the index. `DELETE` restores it. Both return `204` and are idempotent. Withdrawing the live version makes the version below it live, and PCs move to it at their next sync like any other change to the live version; when that changes an MCP server, each person is asked first. The package's name, description, and tags follow whichever version is live afterwards. A purged version cannot be restored (`409`). Owner or admin only (`403`); `404` when the version does not exist.

### `PUT` and `DELETE /api/packages/{namespace}/{packageId}/revoke`

`PUT` revokes the package: it leaves the catalog, index, and namespace archive, and appears in the index's `revoked` list so clients uninstall it. `DELETE` restores it; clients do not reinstall it. Both return `204` and are idempotent. Owner or admin only (`403`); restoring a package an admin revoked needs an admin (`403`: "An admin removed this package from every PC, so only an admin can restore it."). `404` when the package does not exist.

### `DELETE /api/packages/{namespace}/{packageId}`

Deletes the package, its versions, suggestions, reports, access rule, and share link, takes it out of bundles, and frees its id. Owners may while nobody ever installed it; otherwise `409` ("People have installed Review workflow, so it can't be deleted. Use Remove from every PC instead."). Admins always may. `204`. The stored archives are deleted last, best effort.

### Suggestions

A suggestion document:

```json
{
  "id": 12,
  "target": "data-team/review",
  "namespace": "data-team",
  "packageId": "review",
  "name": "Review workflow",
  "baseVersion": "1.2.0",
  "liveVersion": "1.3.0",
  "message": "Tightened step 3.",
  "suggestedBy": "CORP\\jane",
  "suggestedByName": "Jane Doe",
  "suggestedAt": "…",
  "state": "pending",
  "decidedBy": null,
  "decidedAt": null,
  "note": null,
  "acceptedVersion": null,
  "sizeBytes": 1234,
  "componentKinds": ["skill"]
}
```

`baseVersion` is the live version the suggestion was made against; when it differs from `liveVersion`, the package changed since. `state` is `pending`, `accepted`, `declined`, or `withdrawn`. `note` is the owner's reason for declining.

| Endpoint                                          | Who                                        | Does                                                                                                                                                                                                                                                                                                                                             |
| ------------------------------------------------- | ------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `POST /api/packages/{ns}/{packageId}/suggestions` | Can see the package and does not own `ns`  | The publish form's content fields (`archive`, or `files` with `paths`, plus `name` and `description`) and a required `message` of up to 4,096 characters. The same staging, size limit, validator, and credential scan as a publish. `201` with the suggestion; `409` for an owner.                                                              |
| `GET /api/packages/{ns}/{packageId}/suggestions`  | Anyone who can see the package             | Every suggestion for its owners; the caller's own for anyone else. Newest first.                                                                                                                                                                                                                                                                 |
| `GET /api/suggestions/{id}`                       | The target's owners, the suggester, admins | The suggestion.                                                                                                                                                                                                                                                                                                                                  |
| `GET /api/suggestions/{id}/files`                 | Same                                       | `[{ "path", "size", "status" }]`, where `status` compares with the live version: `added`, `changed`, `removed`, or `unchanged`.                                                                                                                                                                                                                  |
| `GET /api/suggestions/{id}/files/{path}`          | Same                                       | One file, by the same rules as a version's files. A removed file is `404`.                                                                                                                                                                                                                                                                       |
| `POST /api/suggestions/{id}`                      | The target's owners                        | `{ "decision": "accept" \| "decline", "version": "1.3.1", "note": "…" }`. Accepting publishes the stored archive as `version` (default: the patch after the highest used), credited to the suggester, with the message as its changelog. Declining needs a note of up to 2,048 characters. Returns the suggestion; `409` when it is not pending. |
| `DELETE /api/suggestions/{id}`                    | The suggester                              | Withdraws a pending suggestion. `204`; `409` when it is not pending.                                                                                                                                                                                                                                                                             |

### `POST /api/events`

Batched client events. Each event carries `kind`, `occurredAt`, `clientVersion`, and a kind-specific body.

| Kind        | Body                                                                                                                                                                                            |
| ----------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `heartbeat` | `osBuild`, `agents` (detected target IDs), `installed` (canonical package IDs), `checks` (`{ id: status }`), optional `device` (the PC's name) and `installedVersions` (`{ "ns/id": "1.2.0" }`) |
| `install`   | `packageId`, `version`, `agents`                                                                                                                                                                |
| `update`    | `packageId`, `fromVersion`, `toVersion`, `agents`                                                                                                                                               |
| `uninstall` | `packageId`, `agents`                                                                                                                                                                           |

Returns `202` with `{ accepted, duplicates, rejected, problems[] }`. The server deduplicates by `(principal, kind, occurredAt, packageId)`. A null event, a null string in an array, or an install, update, or uninstall of a package id the marketplace never had counts as `rejected`. A batch holds at most 500 events (`422`). The server keeps the latest heartbeat per account and `device`, so one person's laptop and virtual desktop are both listed.

### `GET /api/stats/packages/{namespace}/{packageId}`

Publisher-facing statistics: installs by day for 90 days, installed base, and agent mix, each counting accounts rather than events. `404` when the caller may not see the package.

### Notifications

| Endpoint                       | Does                                                                                                                                                                                                                       |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /api/notifications`       | `{ "items": [{ "id", "kind", "text", "link", "createdAt", "read" }] }`, newest first. `?after={id}` returns only newer ones, oldest first, for a client that remembers the last it showed. `?limit=` is 1–200, default 50. |
| `POST /api/notifications/read` | Body `{ "upTo": 42 }` marks every notification up to that id read. `204`.                                                                                                                                                  |

`kind` is `suggestion.created`, `suggestion.decided`, `review.decided`, `report.created`, `report.resolved`, `team.added`, `share.added`, or `package.revoked`. `text` is one sentence to show as it is. `link` is a portal path such as `/p/jacob/review`, `/suggestions/12`, or `/teams/data-team`, or `null`.

### `GET /api/access/{namespace}` and `GET /api/access/{namespace}/{id}`

Who may see a space, or a package or bundle `id` in it. Anyone who owns the space (every member of a team) and admins; anyone else gets `403`.

```json
{
  "target": "data-team/review",
  "visibility": "inherit",
  "effective": "private",
  "users": [{ "account": "CORP\\alice", "displayName": "Alice Chen" }],
  "teams": [{ "namespace": "platform", "displayName": "Platform Team" }],
  "groups": [],
  "link": "https://marketplace.example.com/l/Xk3qP0v4n1aZr8yWb2cTdA"
}
```

`visibility` is `public` or `private` for a space and `inherit`, `public`, or `private` for a package or bundle; `effective` is the setting that applies. `link` is the target's share link, or `null` when nobody has asked for one.

### `PUT /api/access/{namespace}` and `PUT /api/access/{namespace}/{id}`

Replaces the visibility and share lists. Body `{ "visibility": "private", "users": [...], "teams": [...], "groups": [...] }`; the lists hold plain strings and replace the stored ones. Returns the resulting document.

- Entries are trimmed and deduplicated case-insensitively, at most 256 characters each and 200 in total (`422`).
- `teams` must name existing teams (`422` names the unknown ones).
- A space is `public` or `private` (`422` for `inherit`).
- Changing a team's space needs a team owner (`403`: "Only the team's owners can change who sees Data Team."). Any member sets a package or bundle.
- A personal namespace nobody has published to yet answers `409` ("Publish to fresh before you set who may see it.").
- An `id` must be a package or bundle in the namespace (`404`).
- Without `visibility`, any entry means `private`, and no entries mean `public` for a space and `inherit` for a package or bundle.

### `POST /api/access/{namespace}/link` and `POST /api/access/{namespace}/{id}/link`

Body `{ "reset": false }`, or none. Returns `{ "link": "https://…/l/<code>" }`: the target's [share link](#links), created on first request; `reset: true` replaces it. Anyone who owns the space; `404` for an unknown `id`.

### Teams

A team document:

```json
{
  "namespace": "data-team",
  "displayName": "Data Team",
  "visibility": "private",
  "role": "owner",
  "members": [{ "account": "CORP\\jacob", "displayName": "Jacob Ragsdale", "owner": true, "joinedAt": "…" }],
  "invite": "https://marketplace.example.com/l/Xk3qP0v4n1aZr8yWb2cTdA"
}
```

`role` is the caller's: `owner`, `member`, or `admin` for an admin who is not a member. `invite` is `null` until someone asks for it.

| Endpoint                                   | Who                                      | Does                                                                                                                                                                                                                                                                                                 |
| ------------------------------------------ | ---------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /api/teams`                          | Anyone                                   | `{ "namespace", "displayName", "visibility" }`; `visibility` is `private` (the default) or `public`. `201` with the team; the caller is its owner. `422` for a namespace outside the `source.id` pattern, `official`, or a display name outside 1–120 characters. `409` when the namespace is taken. |
| `GET /api/teams`                           | Anyone                                   | `[{ "namespace", "displayName", "visibility", "role", "memberCount" }]` for the caller's teams.                                                                                                                                                                                                      |
| `GET /api/teams/{ns}`                      | Members, admins                          | The team. `404` for anyone else.                                                                                                                                                                                                                                                                     |
| `PUT /api/teams/{ns}`                      | Owners                                   | `{ "displayName" }`. Returns the team.                                                                                                                                                                                                                                                               |
| `POST /api/teams/{ns}/members`             | Owners                                   | `{ "account", "owner" }`: adds the member, or changes whether they are an owner. `account` is 1–256 characters. Returns the team; `409` when it would leave no owner.                                                                                                                                |
| `DELETE /api/teams/{ns}/members?account=…` | Owners; any member for their own account | Removes the member, or leaves. `204`; `409` for the last owner.                                                                                                                                                                                                                                      |
| `POST /api/teams/{ns}/invite`              | Owners                                   | Body `{ "reset": false }`, or none. Returns `{ "invite": "https://…/l/<code>" }`, created on first request; `reset: true` replaces it. `403` for a member, because joining makes someone a publisher.                                                                                                |
| `DELETE /api/teams/{ns}`                   | Owners; admins                           | Deletes a team that has never published a package or bundle, with its members, links, and access rules. `204`; `409` for an owner otherwise. An admin may delete one that has: its packages are revoked, its bundles, members, links, and rules are removed, and the namespace stays reserved.       |

### `GET /api/directory?q=<text>`

`{ "people": [{ "account", "displayName" }], "teams": [{ "namespace", "displayName" }] }`: at most 10 of each whose account, display name, or namespace contains the text, ignoring case. `people` are accounts that have signed in; `teams` are the ones with members that the caller may see (their own, public ones, and ones shared with them). No text answers empty lists.

### `GET /api/links/{code}`

A preview: `{ "kind": "invite" | "share", "targetKind": "team" | "space" | "package" | "bundle", "target": "data-team/review", "name": "Review workflow", "by": "Jacob Ragsdale" }`. `by` is whoever created the link. An unknown or reset code, or a target that is gone, answers `404` with the title "This link no longer works. Ask the person who sent it for a new one."

### `POST /api/links/{code}`

Redeems the [link](#links) and returns the preview plus `"changed": bool`, false when the caller was already a member or could already see the target.

### Bundles

A bundle document:

```json
{
  "id": "data-team/starter",
  "namespace": "data-team",
  "bundleId": "starter",
  "name": "Starter kit",
  "description": "",
  "members": ["jacob/review"],
  "hiddenMembers": 0,
  "publisher": { "account": "data-team", "displayName": "Data Team" },
  "lane": "team",
  "updatedAt": "…",
  "updatedBy": "CORP\\jacob",
  "owned": true,
  "visibility": "inherit",
  "effective": "public"
}
```

`members` are the ones the caller can see and `hiddenMembers` counts the rest. Members follow their own access; sharing a bundle does not share its members.

| Endpoint                        | Who                           | Does                                                                                                                                                                                                                                                                                                                                         |
| ------------------------------- | ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `PUT /api/bundles/{ns}/{id}`    | Anyone who owns `ns`          | `{ "name", "description", "members" }`: creates or replaces the bundle. `name` is 1–120 characters, `description` up to 1,024. `members` are 1–50 canonical `ns/package` ids of live packages the caller can see, from any namespace (`422` names the others). `id` follows the package id pattern and may not be a package in `ns` (`409`). |
| `GET /api/bundles/{ns}/{id}`    | Anyone who may see the bundle | The bundle, or `404`.                                                                                                                                                                                                                                                                                                                        |
| `DELETE /api/bundles/{ns}/{id}` | Anyone who owns `ns`          | Deletes the bundle, its access rule, and its share link. `204`.                                                                                                                                                                                                                                                                              |

### Reports and feedback

A report document: `{ "id", "account", "packageId", "kind", "reason", "createdAt", "resolvedAt", "resolvedBy", "note" }`. `kind` is `problem` or `feedback`; `note` is what whoever resolved it told the reporter.

| Endpoint                                      | Who                            | Does                                                                                                                                                                                            |
| --------------------------------------------- | ------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /api/packages/{ns}/{packageId}/reports` | Anyone who can see the package | Body `{ "reason": "…", "kind": "problem" }`; `reason` is 1 to 2,048 characters and `kind` defaults to `problem`. `202`. Both kinds reach the package's owners; a `problem` also reaches admins. |
| `GET /api/packages/{ns}/{packageId}/reports`  | The package's owners, admins   | Its latest 200 reports, open first, newest first. `403` for anyone else who can see the package.                                                                                                |
| `GET /api/reports/mine`                       | Anyone                         | The caller's own latest 200, newest first, with the resolution.                                                                                                                                 |
| `POST /api/reports/{id}/resolve`              | The package's owners, admins   | Optional body `{ "note": "…" }` (at most 2,048 characters), which the reporter sees. `204`; `403` for an owner on a `problem`, which only admins close; `404` for anyone else.                  |

### `GET /api/admin/summary`

Admins only. Active users (1, 7, 30 days), live package and publisher counts, client version distribution (one count per PC), agent mix, top packages, preflight failure counts by check ID over the last 7 days, and the number of unresolved `problem` reports.

### `GET /api/admin/reviews`

Admins only. The packages whose [MCP server waits for an admin](#public-mcp-servers), oldest first: `[{ "id", "namespace", "packageId", "name", "version", "publishedBy", "publishedAt", "changelog", "componentKinds", "mcpServers", "audience", "installedBase" }]`, where the version fields describe the live version. `mcpServers` is in [the package shape](#get-apipackagesnamespacepackageid). `audience` says who would see it: `Everyone`, or for example `Private: 3 people, 1 team, 1 AD group, a share link`.

### `POST /api/admin/reviews/{namespace}/{packageId}`

Admins only. Body `{ "decision": "approve" | "decline", "note": "…" }`; a decline needs a note (at most 2,048 characters), which the owners see. Approval lets everyone the package is shared with see it, and covers later versions that launch the same servers. The owners are notified either way. `204`; `404` when the package has no live version with an MCP server.

### `GET /api/admin/reports` and `POST /api/admin/reports/{id}/resolve`

Admins only. The latest 200 `problem` reports in [the report shape](#reports-and-feedback), unresolved first. Resolving takes the same optional `{ "note" }` and returns `204`.

### `GET /api/admin/packages/{namespace}/{packageId}/installs`

Admins only. Every PC whose latest heartbeat lists the package, most recently seen first: `[{ "account", "device", "version", "clientVersion", "lastSeenAt" }]`. `version` is `null` for an app that does not report versions. `?format=csv` downloads the same columns as CSV.

### `GET /api/admin/audit`

Admins only. Who changed what, newest first: `[{ "id", "at", "actor", "action", "target", "detail" }]`. `?before={id}` pages back, `?limit=` is 1–1,000 (default 100), and `?format=csv` downloads CSV; a cell that a spreadsheet would run as a formula is prefixed with `'`. `action` is one of `publish`, `yank`, `unyank`, `revoke`, `restore`, `delete`, `purge`, `review.approve`, `review.decline`, `access.set`, `link.create`, `link.reset`, `link.redeem`, `team.create`, `team.rename`, `team.member.set`, `team.member.remove`, `team.delete`, `bundle.save`, `bundle.delete`, `block`, `unblock`, `namespace.transfer`, `report.resolve`, and `suggestion.decide`.

### Admin blocks

| Endpoint                             | Does                                                                                                                                                  |
| ------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `GET /api/admin/blocks`              | `[{ "account", "blockedBy", "blockedAt", "reason" }]`.                                                                                                |
| `PUT /api/admin/blocks/{account}`    | Optional body `{ "reason": "…" }` (up to 1,024 characters). [Blocks](#blocked-accounts) the account; URL-encode the backslash (`CORP%5Cjane`). `204`. |
| `DELETE /api/admin/blocks/{account}` | Lifts the block. `204`; `404` when there is none.                                                                                                     |

An entry with a domain matches that account only; a bare username matches it in any domain.

### `DELETE /api/admin/packages/{namespace}/{packageId}/versions/{version}`

Admins only. Purges a version, for a leaked secret say: it is withdrawn and marked purged in the database, the namespace archive is rebuilt, and then its stored archive, and that of the suggestion accepted as it, are deleted from Artifact Keeper. The number stays used. `204`; `502` when Artifact Keeper refused a delete, in which case the database is already updated and calling again retries the delete. Purging does not reach PCs that installed the version; [revoke](#put-and-delete-apipackagesnamespacepackageidrevoke) the package first when they must lose it.

### `PUT /api/admin/namespaces/{namespace}/owner`

Admins only. Body `{ "account": "CORP\\jane" }`. Hands a personal namespace to another account, for an owner who left: the namespace becomes a team owned by that account, so it and every installed skill name built from it stay the same, and the account is notified. The previous owner gets a new personal namespace the next time they sign in. `204`; `404` for an unknown namespace; `409` when it is not personal.

## Errors

Errors are RFC 9457 problem documents. `title` is a full sentence written for the person using the client, so show it as it is. When `detail` is present, show it too.

| Status | When                                                                                                                                                                                                                                                                                                                                                                                           |
| ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `400`  | A malformed JSON body, route value, or upload form. `title` is `Failed to read parameter … as JSON.`; `detail` names the JSON path, for example `Path: $.events[0].occurredAt`.                                                                                                                                                                                                                |
| `401`  | No credentials. Carries `WWW-Authenticate: Negotiate`.                                                                                                                                                                                                                                                                                                                                         |
| `403`  | The caller does not own the namespace (`title` names the account and the namespace), or is a team member doing what only owners may. Also any authenticated request other than `GET` or `HEAD` whose `Sec-Fetch-Site` is present and not `same-origin` or `none` (a page on another site).                                                                                                     |
| `404`  | The target is missing or hidden, with the same sentence either way: `The package jacob/review was not found, or you do not have access to it.` A dead link answers `This link no longer works. Ask the person who sent it for a new one.`                                                                                                                                                      |
| `409`  | A conflict with stored state: a reused version number or one below the live version (both carry `suggestedVersion`), a revoked package, a package or bundle id already taken in the namespace, a taken team name, a team left without an owner, a team or package that cannot be deleted, a suggestion that is not pending, a purged version, a personal namespace claimed by another account. |
| `413`  | The request is above the 50 MB upload limit: `The request is larger than the 50 MB limit.`                                                                                                                                                                                                                                                                                                     |
| `422`  | The request breaks a rule; `title` says which. A validator failure also carries `errors[]` with `path` and `message`, matching the Rust validator's report.                                                                                                                                                                                                                                    |
| `426`  | The desktop app or CLI is older than `Client:MinimumVersion`; the document carries `downloadUrl`. See [client versions](#client-versions-and-rate-limits).                                                                                                                                                                                                                                     |
| `429`  | Too many changes from one account; `Retry-After` says how many seconds to wait.                                                                                                                                                                                                                                                                                                                |
| `502`  | Artifact Keeper refused to delete a purged archive; retrying the purge retries the delete.                                                                                                                                                                                                                                                                                                     |
| `503`  | PostgreSQL is unreachable, Artifact Keeper is unavailable after one automatic retry, or the package validator is missing or timed out. `title` says which and when to try again.                                                                                                                                                                                                               |

## Configuration

| Setting                                                      | Meaning                                                                                                                                                                                                                                                                  |
| ------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `ConnectionStrings:Marketplace`                              | PostgreSQL connection string.                                                                                                                                                                                                                                            |
| `ArtifactKeeper:BaseUrl`                                     | Artifact Keeper API base, for example `http://artifact-keeper:8080`.                                                                                                                                                                                                     |
| `ArtifactKeeper:Repository`                                  | Generic repository name, for example `files`.                                                                                                                                                                                                                            |
| `ArtifactKeeper:Prefix`                                      | Path prefix inside the repository, for example `marketplace`.                                                                                                                                                                                                            |
| `ArtifactKeeper:Username` / `Password`                       | Service credential used to obtain a bearer token.                                                                                                                                                                                                                        |
| `Database:MigrateOnStartup`                                  | Applies pending EF Core migrations at startup. Default `true`. When this flag applies the migration that ends version review, it also rebuilds every namespace archive once; migrations applied another way need a restart with the flag on, or a publish per namespace. |
| `Auth:EnableNegotiate`                                       | Registers the Negotiate (Kerberos) scheme. Default `true`; the integration tests turn it off.                                                                                                                                                                            |
| `Auth:AllowDevHeader`                                        | Trusts `X-Dev-User` outside Development. Beside Negotiate it logs a startup warning.                                                                                                                                                                                     |
| `Auth:LdapDomain`                                            | Optional. Enables LDAP group claims for Negotiate, and AD groups in share lists.                                                                                                                                                                                         |
| `Auth:LdapMachineAccountName` / `LdapMachineAccountPassword` | Optional. The account LDAP binds as. Unset, it binds with the Kerberos client keytab in `KRB5_CLIENT_KTNAME` or the ticket cache.                                                                                                                                        |
| `Auth:AdminGroup`                                            | AD group whose members may call `/api/admin/*`.                                                                                                                                                                                                                          |
| `Auth:AdminAccounts`                                         | Accounts that may call `/api/admin/*`. An entry with a domain matches that account only.                                                                                                                                                                                 |
| `Auth:OfficialPublishers`                                    | Principals that may publish under `official`. Matched like `AdminAccounts`.                                                                                                                                                                                              |
| `Client:MinimumVersion` / `LatestVersion`                    | Returned by `/api/health`. Older clients get `426` (defaults `0.2.0` and `0.2.2`).                                                                                                                                                                                       |
| `RateLimits:WritesPerMinute` / `UploadsPerHour`              | Per-account limits on changes and on publishes and suggestions. Defaults `120` and `60`.                                                                                                                                                                                 |
| `Notifications:WebhookUrl`                                   | Optional. Receives `POST { "text", "link" }` for each publish that puts a package in the MCP review queue and each problem report, for example a Teams or Slack incoming webhook.                                                                                        |
| `Validator:Path`                                             | Path to the `validate-source` binary inside the image.                                                                                                                                                                                                                   |
| `Validator:TimeoutSeconds`                                   | How long one validator run may take before the publish answers `503`. Default `60`.                                                                                                                                                                                      |
| `Server:PublicBaseUrl`                                       | Required in every deployment. The HTTPS base clients reach, for example `https://marketplace.corp.example`; every catalog source URL and link is built from it. The default is a placeholder.                                                                            |
| `Server:CatalogId` / `CatalogName` / `CatalogDescription`    | The `repository` block of `/api/catalog`. Defaults `marketplace`, `Marketplace`, and a one-line description.                                                                                                                                                             |
| `Server:DownloadsPath`                                       | Folder served at `/downloads`: `manifest.json` and `releases/`. The image sets `/srv/downloads`.                                                                                                                                                                         |
| `Server:TrustedProxies`                                      | The reverse proxies whose `X-Forwarded-For` and `X-Forwarded-Proto` are trusted, as IP addresses or CIDR ranges. Empty trusts any sender and logs a warning outside Development.                                                                                         |
