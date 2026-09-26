# Preflight reference

Agent Plugins runs a preflight at the end of every sync, including the one the window starts when it opens, and on demand from the System Status panel. Every check has a stable ID, one of four statuses, an optional remediation, and a blocking flag. The report is stored in app state, rendered in the UI, and sent with the `heartbeat` event so fleet problems are visible centrally. [ADR 0004](decisions/0004-internal-marketplace.md) records the decision.

## Report

```json
{
  "startedAtEpochSeconds": 1756425600,
  "durationMillis": 1840,
  "blocked": false,
  "checks": [
    { "id": "auth.identity", "group": "auth", "title": "Marketplace sign-in", "status": "ok", "detail": "CORP\\jacob (namespace jacob)", "remediation": null, "blocking": false, "durationMillis": 212 }
  ]
}
```

| Field         | Meaning                                                                                                           |
| ------------- | ----------------------------------------------------------------------------------------------------------------- |
| `id`          | Stable dotted identifier. Never renamed; retire an ID instead.                                                    |
| `group`       | `host`, `auth`, `server`, `agents`, or `dependencies`.                                                            |
| `status`      | `ok`, `warn`, `fail`, or `skipped`. `skipped` means a prerequisite check failed or the platform differs.          |
| `detail`      | One line for a person. May be empty.                                                                              |
| `remediation` | `null`, `{ "kind": "autoFixed" }`, `{ "kind": "action", "action": "…" }`, or `{ "kind": "manual", "text": "…" }`. |
| `blocking`    | `true` only when a `fail` status makes every install and update fail until it is fixed.                           |

Checks run in parallel where independent, each with a timeout. A timed-out check reports `fail` with the timeout in `detail`. The UI never waits longer than the longest timeout before showing partial results.

## Checks

### Host

| ID               | Checks                                                                                                                                                                        | Status rules                                                                                                 | Remediation                                 |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | ------------------------------------------- |
| `host.platform`  | OS name, build, and architecture.                                                                                                                                             | `ok` on Windows 11 x64; `warn` on other Windows; `skipped` elsewhere.                                        | —                                           |
| `host.domain`    | Domain membership via `NetGetJoinInformation`; user principal via `GetUserNameEx`.                                                                                            | `ok` when domain-joined; `fail` when workgroup unless the server allows the `DevHeader` scheme, then `warn`. | manual                                      |
| `host.serverUrl` | The configured base URL is HTTPS and its host is a fully qualified name, not an IP address or a short name.                                                                   | `fail` otherwise (Kerberos falls back to NTLM).                                                              | manual                                      |
| `host.dns`       | The server host resolves.                                                                                                                                                     | `fail` when it does not.                                                                                     | manual                                      |
| `host.tls`       | A TLS handshake to the server succeeds through the platform trust store; records the issuing CA.                                                                              | `fail` on handshake error.                                                                                   | manual                                      |
| `host.proxy`     | Proxy variables are normalized. The system proxy applies to the app's own process only; only proxy variables the user set are published to the user session.                  | `warn` when a user-set proxy variable could not be published.                                                | autoFixed when the system proxy was applied |
| `host.clock`     | Local time versus the server `Date` header.                                                                                                                                   | `warn` above 60 s; `fail` above 300 s.                                                                       | manual                                      |
| `host.homeDirs`  | The app data and cache directories, and the home skill directories of the agents in use (`~/.claude/skills` only with Claude Code), exist or can be created and are writable. | `fail` and blocking when an app directory is unwritable; `warn` when an agent skill directory is.            | autoFixed when creation succeeds            |
| `host.disk`      | Free space on the home volume.                                                                                                                                                | `warn` below 1 GB; `fail` below 200 MB.                                                                      | manual                                      |
| `host.longPaths` | `HKLM\SYSTEM\CurrentControlSet\Control\FileSystem\LongPathsEnabled`.                                                                                                          | `warn` when disabled.                                                                                        | manual                                      |
| `host.path`      | The user `Environment` PATH contains the tool directories the app publishes.                                                                                                  | autoFixed.                                                                                                   | autoFixed                                   |

### Auth

| ID              | Checks                                                                                                                            | Status rules                                                                                                                                                   | Remediation |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------- |
| `auth.ticket`   | SSPI can produce a Negotiate token for `HTTP/<server host>`. Proves the domain controller is reachable and the SPN is registered. | `fail` when SSPI errors; `skipped` when not domain-joined.                                                                                                     | manual      |
| `auth.identity` | `GET /api/me` succeeds.                                                                                                           | `ok` with the account and namespace; `fail` on 401 (SPN or keytab mismatch), 403 (not authorized), or network error; `skipped` when the server is unreachable. | manual      |

### Server

| ID                     | Checks                                                                                   | Status rules                                                                                                                                | Remediation     |
| ---------------------- | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | --------------- |
| `server.health`        | `GET /api/health` returns a document.                                                    | `fail` when unreachable; the app enters offline mode. A sync that could not connect to the marketplace records `fail` without asking again. | —               |
| `server.clientVersion` | The client version against `minimumClientVersion` and `latestClientVersion` from health. | `fail` below minimum; `warn` below latest.                                                                                                  | action `update` |
| `server.catalog`       | The marketplace catalog fetched during the last sync.                                    | `warn` when none was fetched yet, or the cached copy is older than 24 h.                                                                    | action `sync`   |

### Agents

| ID                | Checks                                                                                                                                                | Status rules                                                           | Remediation                                   |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------- | --------------------------------------------- |
| `agents.detected` | At least one supported agent is installed.                                                                                                            | `warn` when none.                                                      | manual                                        |
| `agents.ledger`   | The ledger file can be read, and this version may change it. A damaged ledger has already been replaced by its `.previous` copy, so it does not fail. | `fail` and blocking when unreadable, or when a newer version wrote it. | manual; `update` for a newer version's ledger |
| `agents.journal`  | Whether `resource-transaction.json` is present.                                                                                                       | `warn` while an interrupted transaction waits for recovery.            | —                                             |
| `agents.drift`    | Owned resources whose digest no longer matches the ledger.                                                                                            | `warn` with the count and paths.                                       | action `showDrift`                            |

### Dependencies

| ID                          | Checks                                                                                                         | Status rules                                                                                                                                                                                       | Remediation                                 |
| --------------------------- | -------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------- |
| `dependencies.uv`           | `uv` and `uvx` are on the login PATH.                                                                          | autoFixed by a background download after launch, checked against the published SHA-256 checksum. Up to five attempts per session; a preflight retries while attempts remain. `warn` while missing. | autoFixed, or manual after the last attempt |
| `dependencies.node`         | `npx` resolves. Only evaluated when an installed MCP server uses `npx`.                                        | `warn` when missing; `skipped` when unused.                                                                                                                                                        | manual                                      |
| `dependencies.cli`          | The application's own executable directory is on the user PATH so `agent-plugins publish` works from an agent. | autoFixed: release builds add their directory to the user PATH. `warn` otherwise.                                                                                                                  | autoFixed                                   |
| `dependencies.publishSkill` | The official `publish` skill is installed.                                                                     | `warn` when absent.                                                                                                                                                                                | action `installPublishSkill`                |

Planned, not yet emitted: `agents.<target>.config` (each shared document the adapter can write parses in its declared format) and `agents.<target>.version` (detected version against the adapter's tested range).

## Presentation

The header carries one button. It stays quiet — a green dot beside the namespace — until a check _fails_; warnings never colour it. A failure turns it red and names the failing check, and the same line appears under the catalog header. The panel behind it leads with three rows, Windows sign-in, marketplace server, and the agents on this machine, each showing the failing check's detail and remediation when one failed. Failures outside those three get a row of their own. Everything else, warnings included, sits in the collapsed **Agent details** and **All checks** sections.

## Blocking

Only `host.homeDirs` and `agents.ledger` may block, because either failure makes every install and update fail on its own. Nothing else enforces the flag: a blocked app still loads the catalog and keeps syncing, and the UI shows the report with its remediation.

## Heartbeat

The `heartbeat` event carries `checks` as `{ "<id>": "<status>" }`. Detail text and paths stay on the machine.
