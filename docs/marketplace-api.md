# Marketplace API reference

The marketplace server is the only endpoint the desktop app and CLI talk to. It is a .NET 10 API in `server/`, backed by PostgreSQL for the index and metrics and by Artifact Keeper for immutable package archives. [ADR 0004](decisions/0004-internal-marketplace.md) records the decision. The generated OpenAPI document is [`server/openapi.json`](../server/openapi.json).

## Authentication

Every endpoint except `GET /api/health` requires an authenticated principal.

| Scheme      | When                                          | Header                             |
| ----------- | --------------------------------------------- | ---------------------------------- |
| `Negotiate` | Production. Kerberos validated with a keytab. | `Authorization: Negotiate <token>` |
| `DevHeader` | `ASPNETCORE_ENVIRONMENT=Development` only.    | `X-Dev-User: <username>`           |

The principal's namespace is its lowercase sAMAccountName. Group claims come from LDAP when `Auth:LdapDomain` is configured; the claim value is the AD group's CN, and every group comparison is case-insensitive. A configured or listed account matches by username, so `CORP\jane`, `jane@corp.example`, and `jane` name the same person.

### Access

Who may see and install what is server policy ([ADR 0005](decisions/0005-marketplace-access-control.md)). A namespace (`ns`) or one package (`ns/packageId`) may carry an allowlist of accounts and groups. No list means public. A package list replaces its namespace list. Owners of a namespace and admins always see it. Everything the caller may not see answers `404`, never `403`, so the desktop app treats it as gone.

A **team namespace** is a `source.id` owned by an AD group: `Auth:TeamNamespaces` lists `{ namespace, group, displayName }` entries, and every member of `group` may publish and yank there and manage its access lists. Who may _read_ a team namespace is a separate access list. Use a `team-` prefix so a team never collides with a person's derived namespace.

## Endpoints

### `GET /api/health`

Anonymous. Returns server version and client version policy.

```json
{ "serverVersion": "0.1.0", "minimumClientVersion": "0.1.0", "latestClientVersion": "0.1.0", "environment": "Production", "authSchemes": ["Negotiate"] }
```

`authSchemes` lets the preflight report `warn` instead of `fail` for a workgroup machine talking to a development server.

### `GET /api/me`

Returns the caller's identity.

```json
{ "account": "CORP\\jacob", "namespace": "jacob", "displayName": "Jacob Ragsdale", "namespaces": ["jacob", "team-data"], "admin": false, "groups": ["Data Engineering"] }
```

`namespaces` includes every team namespace the caller's groups own; `groups` is the resolved group claims, useful when a team-restricted package is unexpectedly missing.

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

Only packages the caller may see are listed. `lane` is `official`, `team`, or `personal`. `restricted` is true when the package or its namespace carries an access list. `installs` counts install events over all time; `installedBase` counts principals whose latest heartbeat includes the package.

### `GET /api/packages/{namespace}/{packageId}`

Returns the package's versions, newest first, with `yanked` flags and each version's `archiveDigest`. `404` when the caller may not see the package.

### `POST /api/packages/{namespace}/{packageId}/versions`

Publishes a version. Multipart form: `archive` (zip, tar, or tar.gz containing exactly one package), `version` (semver string), optional `tags` (comma separated), optional `changelog` (text).

Server checks, in order: caller owns `{namespace}`; archive within 50 MB and free of unsafe entries; `agent-plugins.json` present with `source.id == namespace`, exactly one package whose `id == packageId`; the Rust `validate-source` binary reports no errors; the version is not already published. On success the version is stored in Artifact Keeper, the namespace archive is regenerated, and the response is `201` with the version document. Validation failures return `422` with the validator messages.

### `POST /api/packages/{namespace}/{packageId}/versions/{version}/yank`

Hides a version from the namespace archive and index. Existing installations are unaffected. Owner or admin only.

### `POST /api/events`

Batched client events. Each event carries `kind`, `occurredAt`, `clientVersion`, and a kind-specific body.

| Kind        | Body                                                                                                        |
| ----------- | ----------------------------------------------------------------------------------------------------------- |
| `heartbeat` | `osBuild`, `agents` (detected target IDs), `installed` (canonical package IDs), `checks` (`{ id: status }`) |
| `install`   | `packageId`, `version`, `agents`                                                                            |
| `update`    | `packageId`, `fromVersion`, `toVersion`, `agents`                                                           |
| `uninstall` | `packageId`, `agents`                                                                                       |

Returns `202`. The server deduplicates by `(principal, kind, occurredAt, packageId)`.

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

Admin group only. Active users (1, 7, 30 days), client version distribution, agent mix, top packages, and preflight failure counts by check ID over the last 7 days.

### `POST /api/reports`

Flags a package. Body: `packageId`, `reason`. Stored and surfaced in the admin summary.

## Errors

Errors are RFC 9457 problem documents. `401` carries `WWW-Authenticate: Negotiate`. `403` names the namespace the caller does not own. `422` carries `errors[]` with `path` and `message`, matching the Rust validator's report.

## Configuration

| Setting                                   | Meaning                                                                                        |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `ConnectionStrings:Marketplace`           | PostgreSQL connection string.                                                                  |
| `ArtifactKeeper:BaseUrl`                  | Artifact Keeper API base, for example `http://artifact-keeper:8080`.                           |
| `ArtifactKeeper:Repository`               | Generic repository name, for example `files`.                                                  |
| `ArtifactKeeper:Prefix`                   | Path prefix inside the repository, for example `marketplace`.                                  |
| `ArtifactKeeper:Username` / `Password`    | Service credential used to obtain a bearer token.                                              |
| `Auth:LdapDomain`                         | Optional. Enables LDAP group claims for Negotiate.                                             |
| `Auth:AdminGroup`                         | AD group whose members may call `/api/admin/*`.                                                |
| `Auth:AdminAccounts`                      | Accounts that may call `/api/admin/*`.                                                         |
| `Auth:OfficialPublishers`                 | Principals that may publish under `official`.                                                  |
| `Auth:TeamNamespaces`                     | `[{ namespace, group, displayName }]`: team namespaces owned by a group. Validated at startup. |
| `Client:MinimumVersion` / `LatestVersion` | Values returned by `/api/health`.                                                              |
| `Validator:Path`                          | Path to the `validate-source` binary inside the image.                                         |
