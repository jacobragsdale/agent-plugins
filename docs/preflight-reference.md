# Preflight reference

Agent Plugins runs a preflight at launch, on demand from the System Status panel, and before each scheduled sync. Every check has a stable ID, one of four statuses, an optional remediation, and a blocking flag. The report is stored in app state, rendered in the UI, and sent with the `heartbeat` event so fleet problems are visible centrally. [ADR 0004](decisions/0004-internal-marketplace.md) records the decision.

## Report

```json
{
  "startedAtEpochSeconds": 1756425600,
  "durationMillis": 1840,
  "blocked": false,
  "checks": [{ "id": "auth.identity", "group": "auth", "title": "Signed in", "status": "ok", "detail": "CORP\\jacob (namespace jacob)", "remediation": null, "blocking": false, "durationMillis": 212 }]
}
```

| Field         | Meaning                                                                                                           |
| ------------- | ----------------------------------------------------------------------------------------------------------------- |
| `id`          | Stable dotted identifier. Never renamed; retire an ID instead.                                                    |
| `group`       | `host`, `auth`, `server`, `agents`, or `dependencies`.                                                            |
| `status`      | `ok`, `warn`, `fail`, or `skipped`. `skipped` means a prerequisite check failed or the platform differs.          |
| `detail`      | One line for a person. May be empty.                                                                              |
| `remediation` | `null`, `{ "kind": "autoFixed" }`, `{ "kind": "action", "action": "…" }`, or `{ "kind": "manual", "text": "…" }`. |
| `blocking`    | `true` only when a `fail` status must stop the app from operating.                                                |

Checks run in parallel where independent, each with a timeout. A timed-out check reports `fail` with the timeout in `detail`. The UI never waits longer than the longest timeout before showing partial results.

## Checks

### Host

| ID               | Checks                                                                                                       | Status rules                                                                                                 | Remediation                          |
| ---------------- | ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------ | ------------------------------------ |
| `host.platform`  | OS name, build, and architecture.                                                                            | `ok` on Windows 11 x64; `warn` on other Windows; `skipped` elsewhere.                                        | —                                    |
| `host.domain`    | Domain membership via `NetGetJoinInformation`; user principal via `GetUserNameEx`.                           | `ok` when domain-joined; `fail` when workgroup unless the server allows the `DevHeader` scheme, then `warn`. | manual                               |
| `host.serverUrl` | The configured base URL is HTTPS and its host is a fully qualified name, not an IP address or a short name.  | `fail` otherwise (Kerberos falls back to NTLM).                                                              | manual                               |
| `host.dns`       | The server host resolves.                                                                                    | `fail` when it does not.                                                                                     | manual                               |
| `host.tls`       | A TLS handshake to the server succeeds through the platform trust store; records the issuing CA.             | `fail` on handshake error.                                                                                   | manual                               |
| `host.proxy`     | Proxy variables are normalized (existing behavior) and the server is reachable through the resolved proxy.   | `warn` on inconsistent variables; `fail` when the proxy refuses the server.                                  | autoFixed for variable normalization |
| `host.clock`     | Local time versus the server `Date` header.                                                                  | `warn` above 60 s; `fail` above 300 s.                                                                       | manual                               |
| `host.homeDirs`  | `~/.agents/skills`, `~/.claude/skills`, and the app data directory exist or can be created and are writable. | `fail` when any is unwritable. Blocking.                                                                     | autoFixed when creation succeeds     |
| `host.disk`      | Free space on the home volume.                                                                               | `warn` below 1 GB; `fail` below 200 MB.                                                                      | manual                               |
| `host.longPaths` | `HKLM\SYSTEM\CurrentControlSet\Control\FileSystem\LongPathsEnabled`.                                         | `warn` when disabled.                                                                                        | manual                               |
| `host.path`      | The user `Environment` PATH contains the tool directories the app publishes.                                 | autoFixed.                                                                                                   | autoFixed                            |

### Auth

| ID              | Checks                                                                                                                            | Status rules                                                                                                                                                   | Remediation |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| `auth.ticket`   | SSPI can produce a Negotiate token for `HTTP/<server host>`. Proves the domain controller is reachable and the SPN is registered. | `fail` when SSPI errors; `skipped` when not domain-joined.                                                                                                     | manual      |
| `auth.identity` | `GET /api/me` succeeds.                                                                                                           | `ok` with the account and namespace; `fail` on 401 (SPN or keytab mismatch), 403 (not authorized), or network error; `skipped` when the server is unreachable. | manual      |

### Server

| ID                     | Checks                                                                                   | Status rules                                            | Remediation     |
| ---------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------- | --------------- |
| `server.health`        | `GET /api/health` returns a document.                                                    | `fail` when unreachable; the app enters offline mode.   | —               |
| `server.clientVersion` | The client version against `minimumClientVersion` and `latestClientVersion` from health. | `fail` and blocking below minimum; `warn` below latest. | action `update` |
| `server.catalog`       | The marketplace catalog fetched during the last sync.                                    | `warn` when the cached copy is older than 24 h.         | action `sync`   |

### Agents

| ID                | Checks                                                                | Status rules                           | Remediation        |
| ----------------- | --------------------------------------------------------------------- | -------------------------------------- | ------------------ |
| `agents.detected` | At least one supported agent is installed.                            | `warn` when none.                      | manual             |
| `agents.ledger`   | The ledger reads and its version is current.                          | `fail` and blocking when unreadable.   | manual             |
| `agents.journal`  | The recovery journal was clean, rolled back, or cleaned up at launch. | `warn` when a rollback or cleanup ran. | —                  |
| `agents.drift`    | Owned resources whose digest no longer matches the ledger.            | `warn` with the count and paths.       | action `showDrift` |

### Dependencies

| ID                          | Checks                                                                                                         | Status rules                                | Remediation                  |
| --------------------------- | -------------------------------------------------------------------------------------------------------------- | ------------------------------------------- | ---------------------------- |
| `dependencies.uv`           | `uv` and `uvx` are on the login PATH (existing behavior).                                                      | autoFixed by download when missing.         | autoFixed                    |
| `dependencies.node`         | `npx` resolves. Only evaluated when an installed MCP server uses `npx`.                                        | `warn` when missing; `skipped` when unused. | manual                       |
| `dependencies.cli`          | The application's own executable directory is on the user PATH so `skill-manager publish` works from an agent. | autoFixed.                                  | autoFixed                    |
| `dependencies.publishSkill` | The official `publish` skill is installed.                                                                     | `warn` when absent.                         | action `installPublishSkill` |

Planned, not yet emitted: `agents.<target>.config` (each shared document the adapter can write parses in its declared format) and `agents.<target>.version` (detected version against the adapter's tested range).

## Presentation

The header carries one button. It stays quiet — a green dot beside the namespace — until a check _fails_; warnings never colour it. A failure turns it red and names the failing check, and the same line appears under the catalog header. The panel behind it leads with three rows, Windows sign-in, marketplace server, and the agents on this machine, each showing the failing check's detail and remediation when one failed. Failures outside those three get a row of their own. Everything else, warnings included, sits in the collapsed **Agent details** and **All checks** sections.

## Blocking

Only `host.homeDirs`, `agents.ledger`, and `server.clientVersion` may block. A blocked app shows the report and offers the remediation; it does not plan, install, or sync.

## Heartbeat

The `heartbeat` event carries `checks` as `{ "<id>": "<status>" }`. Detail text and paths stay on the machine.
