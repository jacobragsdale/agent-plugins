# Marketplace API reference

The marketplace server is the only endpoint the desktop app and CLI talk to. It is a .NET 10 API in `server/`, backed by PostgreSQL for the index and metrics and by Artifact Keeper for immutable package archives. [ADR 0004](decisions/0004-internal-marketplace.md) records the decision. The generated OpenAPI document is [`server/openapi.json`](../server/openapi.json).

The same server serves the web portal at `/` and the installers at `/downloads` ([ADR 0006](decisions/0006-web-portal-and-review.md)). Those need no sign-in; an unknown `/api/*` route is still a `404` problem document.

## Authentication

Every endpoint except `GET /api/health` requires an authenticated principal.

| Scheme      | When                                          | Header                                                 |
| ----------- | --------------------------------------------- | ------------------------------------------------------ |
| `Negotiate` | Production. Kerberos validated with a keytab. | `Authorization: Negotiate <token>`                     |
| `DevHeader` | Development, or `Auth:AllowDevHeader`.        | `X-Dev-User: <username>`, optional `X-Dev-Groups: a,b` |

`Auth:AllowDevHeader` beside Negotiate outside Development logs a startup warning: anyone who can reach the server can claim any account. The server still starts, because the home lab runs that way on purpose.

Group claims come from LDAP when `Auth:LdapDomain` is configured; the claim value is the AD group's CN, and every group comparison is case-insensitive. A configured or listed account matches by username, so `CORP\jane`, `jane@corp.example`, and `jane` name the same person.

### Personal namespaces

The principal's personal namespace is derived from its sAMAccountName: lowercased, each run of other characters replaced by one hyphen, prefixed `u-` when it does not start with a letter, and cut to 16 characters. A derived name equal to `official` or a configured team namespace becomes `u-<name>` (`u-official`).

Two accounts can derive the same name: `christopher.johnson` and `christopher.johnston` both derive `christopher-john`. The first account to publish claims it. Any other account gets the first of `<first 14 characters>-2` through `-9` that it has claimed or that is still free (`christopher-jo-2`); when all are taken, every authenticated request answers `409`. Clients read `namespace` from [`GET /api/me`](#get-apime) and never derive it locally.

### Access

Who may see and install what is server policy ([ADR 0005](decisions/0005-marketplace-access-control.md)). A namespace (`ns`) or one package (`ns/packageId`) may carry an allowlist of accounts and groups. No list means public. A package list replaces its namespace list. Owners of a namespace and admins always see it. Everything the caller may not see answers `404`, never `403`, so the desktop app treats it as gone.

A **team namespace** is a `source.id` owned by an AD group: `Auth:TeamNamespaces` lists `{ namespace, group, displayName }` entries, and every member of `group` may publish and yank there and manage its access lists. Who may _read_ a team namespace is a separate access list. Use a `team-` prefix by convention; a person whose name derives to a team namespace gets `u-<name>` instead.

### Review

Each version is `pending`, `approved`, or `rejected` ([ADR 0006](decisions/0006-web-portal-and-review.md)). A publish is approved at once when the publisher is an admin, or when the package already has a live version and the new version contains no MCP server; otherwise it waits for an admin. Only approved, non-yanked versions are **live**: the catalog, index, namespace archives, and everyone but owners and admins see nothing else. A package with no live version answers `404` to them. A pending version leaves the package's name, description, and tags unchanged until it is approved.

## Endpoints

### `GET /api/health`

Anonymous. Returns server version and client version policy.

```json
{ "serverVersion": "0.1.0", "minimumClientVersion": "0.1.0", "latestClientVersion": "0.1.0", "environment": "Production", "authSchemes": ["Negotiate"] }
```

`authSchemes` lets the preflight report `warn` instead of `fail` for a workgroup machine talking to a development server. `503` with a problem document when the database is unreachable.

### `GET /api/me`

Returns the caller's identity.

```json
{ "account": "CORP\\jacob", "namespace": "jacob", "displayName": "Jacob Ragsdale", "namespaces": ["jacob", "team-data"], "admin": false, "groups": ["Data Engineering"] }
```

`namespace` is the caller's settled [personal namespace](#personal-namespaces), which can differ from their account name. `namespaces` includes every team namespace the caller's groups own; `groups` is the resolved group claims, useful when a team-restricted package is unexpectedly missing.

### `GET /api/catalog`

Returns an `agent-plugins-repository.json` document. Each listed source is one namespace with at least one non-yanked package the caller may see; `packageCount` counts only those. The listing carries the optional marketplace fields `publisher`, `packageCount`, and `updatedAt`. `ETag` is the digest of the document.

### `GET /api/sources/{namespace}/archive`

Returns the namespace's current source archive: a zip whose root `agent-plugins.json` has `source.id` equal to the namespace and one package per latest non-yanked version. Supports `HEAD`, `ETag`, and `Last-Modified`.

A caller who may see only some of the namespace's packages receives an archive holding just those. Its `ETag` is the digest of that subset, stable for the same subset of the same archive; `Last-Modified` is the full archive's. A caller who may see none answers `404`.

### `GET /api/index`

Per-package marketplace metadata joined by canonical ID.

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
      "publishedAt": "2026-08-28T19:12:03Z",
      "installs": 34,
      "installedBase": 12,
      "restricted": false
    }
  ]
}
```

Only packages the caller may see are listed. `lane` is `official`, `team`, or `personal`. `publisher.account` is the account that claimed a personal namespace, and the namespace itself (`official`, `team-platform`) for the other lanes. `restricted` is true when the package or its namespace carries an access list. `installs` counts install events over all time; `installedBase` counts principals whose latest heartbeat includes the package.

### `GET /api/packages/{namespace}/{packageId}`

The package and its versions, newest first: `liveVersion` (null until one is approved) and, per version, `archiveDigest`, `sizeBytes`, `publishedBy`, `publishedAt`, `changelog`, `yanked`, `componentKinds`, `reviewState`, and `reviewNote`. Owners and admins see every version and the reviewer's note; everyone else sees approved versions only. `404` when the caller may not see the package.

### `GET /api/packages/{namespace}/{packageId}/versions/{version}/archive`

The version's stored zip, as published. `Content-Disposition: attachment; filename="{namespace}-{packageId}-{version}.zip"`. Supports `HEAD`; `ETag` is the strong archive digest and `If-None-Match` answers `304`. Readable by owners and admins for any version, and by others only for live versions they may see; otherwise `404`.

### `GET /api/packages/{namespace}/{packageId}/versions/{version}/files`

The version's files as `[{ "path": "skills/review/SKILL.md", "size": 812 }]`, relative to the source root. `GET …/files/{path}` returns one file: UTF-8 text up to 1 MB as `text/plain; charset=utf-8`, anything else (binary, or larger than 1 MB) as an `application/octet-stream` attachment. Readable by owners and admins, and by others only for live versions they may see.

### `GET /api/mine`

What the portal's My skills page shows: `spaces[]` (`namespace`, `displayName`, `lane`) the caller may publish to, and `packages[]` in those namespaces in the package-detail shape, including pending and rejected versions.

### `POST /api/packages/{namespace}/{packageId}/versions`

Publishes a version. Multipart form: `version`, optional `tags` (comma separated), optional `changelog` (text), and the content as one of:

- `archive`: a zip. With `agent-plugins.json` at its root it must declare exactly one package; without one, its contents are wrapped as below.
- `files` with one `paths` value each (relative, forward slashes): a skill directory (`SKILL.md` at the root), a folder of skill directories (a skill pack), one MCP document (`.json`), or a source tree. A single top-level folder shared by every path is dropped. Optional `name` and `description` label a wrapped package.

| Field       | Rule                                                                                                                                     |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| `version`   | A release version, `major.minor.patch`. A pre-release such as `1.0.0-beta.1` is refused.                                                 |
| `tags`      | At most 10, each up to 32 lowercase letters, digits, and single hyphens. Lowercased and deduplicated; an invalid tag is refused by name. |
| `changelog` | At most 4,096 characters.                                                                                                                |

Wrapping is `validate-source stage`, the same code as `agent-plugins publish`. Server checks, in order:

1. The upload is within 50 MB (`413`).
2. The caller owns `{namespace}` (`403`).
3. The upload is free of unsafe paths, and `version`, `tags`, and `changelog` follow the rules above (`422`).
4. `agent-plugins.json` has `source.id == namespace` and exactly one package whose `id == packageId` (`422`).
5. The Rust validator, with its credential scan, reports no errors (`422` with the validator messages).
6. The version was never published before; pending, rejected, and yanked numbers stay used. `409` suggests the patch after the highest used version ("Publish 1.0.1 or later.").
7. A personal namespace nobody has published to yet is created only by its owner, not by an admin publishing on their behalf (`403`).

On success the version is stored in Artifact Keeper and the response is `201` with the version document, whose `reviewState` says whether it is live or waiting. Only an approved version regenerates the namespace archive. The first publish to a namespace claims it: a personal namespace for the publishing account, `official` and team namespaces for the namespace itself.

### `PUT` and `DELETE /api/packages/{namespace}/{packageId}/versions/{version}/yank`

`PUT` yanks a version: it leaves the namespace archive, the catalog, and the index. `DELETE` restores it. Both return `204` and are idempotent. Existing installations are unaffected. The package's name, description, and tags follow whichever version is live afterwards. Owner or admin only (`403`); `404` when the version does not exist.

### `POST /api/events`

Batched client events. Each event carries `kind`, `occurredAt`, `clientVersion`, and a kind-specific body.

| Kind        | Body                                                                                                        |
| ----------- | ----------------------------------------------------------------------------------------------------------- |
| `heartbeat` | `osBuild`, `agents` (detected target IDs), `installed` (canonical package IDs), `checks` (`{ id: status }`) |
| `install`   | `packageId`, `version`, `agents`                                                                            |
| `update`    | `packageId`, `fromVersion`, `toVersion`, `agents`                                                           |
| `uninstall` | `packageId`, `agents`                                                                                       |

Returns `202` with `{ accepted, duplicates, rejected, problems[] }`. The server deduplicates by `(principal, kind, occurredAt, packageId)`. A null event or a null string in an array counts as `rejected`. A batch holds at most 500 events (`422`).

### `GET /api/stats/packages/{namespace}/{packageId}`

Publisher-facing statistics: installs by day for 90 days, installed base, and agent mix. `404` when the caller may not see the package.

### `GET /api/access/{namespace}` and `GET /api/access/{namespace}/{packageId}`

The allowlist for a namespace or a package. Owner or admin only; anyone else gets `403`.

```json
{ "target": "jacob/review", "users": ["CORP\\jane"], "groups": ["Data Engineering"] }
```

Empty lists mean public.

### `PUT /api/access/{namespace}` and `PUT /api/access/{namespace}/{packageId}`

Replaces the allowlist. Body `{ "users": [...], "groups": [...] }`; both lists empty makes the target public again. Entries are trimmed and deduplicated case-insensitively, at most 256 characters each and 200 in total. Owner or admin only (`403`); a package target must be published (`404`). Returns the resulting document. The CLI verb is `agent-plugins access`.

### `GET /api/admin/summary`

Admins only. Active users (1, 7, 30 days), live package and publisher counts, client version distribution, agent mix, top packages, preflight failure counts by check ID over the last 7 days, and the number of unresolved reports.

### `GET /api/admin/reviews`

Admins only. The pending versions, oldest first, with `firstVersion` (the package has never been approved), `liveVersion`, `componentKinds`, `publishedBy`, and `changelog`.

### `POST /api/admin/reviews/{namespace}/{packageId}/{version}`

Admins only. Body `{ "decision": "approve" | "reject", "note": "…" }`; a rejection needs a note (at most 2,048 characters), which the publisher sees. Approval makes the version live and regenerates the namespace archive. `204`; `409` when the version is not pending, naming why (yanked, already approved, or already rejected); `404` when it does not exist.

### `GET /api/admin/reports` and `POST /api/admin/reports/{id}/resolve`

Admins only. The latest 200 reports, unresolved first: `id`, `account`, `packageId` (canonical `ns/id`), `reason`, `createdAt`, `resolvedAt`, and `resolvedBy`. Resolving one returns `204`.

### `POST /api/packages/{namespace}/{packageId}/reports`

Flags a package. Body `{ "reason": "…" }`, 1 to 2,048 characters (`422` otherwise). Returns `202`. Stored, counted in the admin summary until resolved, and listed at `/api/admin/reports`. `404` when the caller may not see the package.

## Errors

Errors are RFC 9457 problem documents. `title` is a full sentence written for the person using the client, so show it as it is. When `detail` is present, show it too.

| Status | When                                                                                                                                                                             |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `400`  | A malformed JSON body, route value, or upload form. `title` is `Failed to read parameter … as JSON.`; `detail` names the JSON path, for example `Path: $.events[0].occurredAt`.  |
| `401`  | No credentials. Carries `WWW-Authenticate: Negotiate`.                                                                                                                           |
| `403`  | The caller does not own the namespace; `title` names the account and the namespace.                                                                                              |
| `404`  | The target is missing or hidden, with the same sentence either way: `The package jacob/review was not found, or you do not have access to it.`                                   |
| `409`  | A conflict with stored state: a reused version number, a review of a version that is not pending, a personal namespace claimed by another account.                               |
| `413`  | The request is above the 50 MB upload limit: `The request is larger than the 50 MB limit.`                                                                                       |
| `422`  | The request breaks a rule; `title` says which. A validator failure also carries `errors[]` with `path` and `message`, matching the Rust validator's report.                      |
| `503`  | PostgreSQL is unreachable, Artifact Keeper is unavailable after one automatic retry, or the package validator is missing or timed out. `title` says which and when to try again. |

## Configuration

| Setting                                   | Meaning                                                                                          |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `ConnectionStrings:Marketplace`           | PostgreSQL connection string.                                                                    |
| `ArtifactKeeper:BaseUrl`                  | Artifact Keeper API base, for example `http://artifact-keeper:8080`.                             |
| `ArtifactKeeper:Repository`               | Generic repository name, for example `files`.                                                    |
| `ArtifactKeeper:Prefix`                   | Path prefix inside the repository, for example `marketplace`.                                    |
| `ArtifactKeeper:Username` / `Password`    | Service credential used to obtain a bearer token.                                                |
| `Auth:AllowDevHeader`                     | Trusts `X-Dev-User` outside Development. Beside Negotiate it logs a startup warning.             |
| `Auth:LdapDomain`                         | Optional. Enables LDAP group claims for Negotiate.                                               |
| `Auth:AdminGroup`                         | AD group whose members may call `/api/admin/*`.                                                  |
| `Auth:AdminAccounts`                      | Accounts that may call `/api/admin/*`.                                                           |
| `Auth:OfficialPublishers`                 | Principals that may publish under `official`.                                                    |
| `Auth:TeamNamespaces`                     | `[{ namespace, group, displayName }]`: team namespaces owned by a group. Validated at startup.   |
| `Client:MinimumVersion` / `LatestVersion` | Values returned by `/api/health`.                                                                |
| `Validator:Path`                          | Path to the `validate-source` binary inside the image.                                           |
| `Server:DownloadsPath`                    | Folder served at `/downloads`: `manifest.json` and `releases/`. The image sets `/srv/downloads`. |
