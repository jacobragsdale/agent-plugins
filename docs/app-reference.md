# App reference

What the Agent Plugins window shows, where it writes, and what it does on its own. For the checks behind **System status** see [the preflight reference](preflight-reference.md); for the terminal equivalents see [the CLI reference](cli-reference.md).

## Window

| Control                          | Effect                                                                                                                          |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Status button                    | Opens **System status**. Shows a green dot and your namespace while no check failed; turns red and names the failure otherwise. |
| **Manage Sources**               | Lists the catalog's sources with **Add** or **Remove**, plus any added source the catalog no longer lists.                      |
| **Refresh**                      | Runs a sync now: preflight, catalog, sources, background updates.                                                               |
| **Reset**                        | Confirms, then uninstalls every package and deletes Agent Plugins' config, cache, and data.                                     |
| Search box                       | Filters on package name, description, publisher, and tags. The count beside it is the number of matching packages.              |
| Package **Install/Uninstall**    | Applies the whole package.                                                                                                      |
| Component row buttons            | Apply one skill or one MCP server. Multi-component packages expand to show them.                                                |
| Source **Install/Uninstall all** | Applies every eligible package in that source as one batch.                                                                     |

Warnings never colour the status button; only failures do. A failing check is also repeated as a line under the header.

## Tray

| Menu item                 | Effect                                                                     |
| ------------------------- | -------------------------------------------------------------------------- |
| **Open Agent Plugins**    | Shows the window.                                                          |
| **Check for Updates Now** | Runs a sync in the background and reports the result, success or failure.  |
| **Launch at Login**       | Toggles autostart. The app then starts with `--background`, window hidden. |
| **Quit Agent Plugins**    | Exits. Scheduled syncs stop until it runs again.                           |

## Package states

The badge on a card, and the button beside it.

| State                | Badge               | Button            | Meaning                                                                                                                                     |
| -------------------- | ------------------- | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `available`          | none                | Install           | Not installed.                                                                                                                              |
| `installed`          | none                | Uninstall         | Installed, and every owned file still matches the ledger.                                                                                   |
| `updateAvailable`    | Update Available    | Update            | The source publishes a different digest for this package.                                                                                   |
| `partiallyInstalled` | Partially Installed | Install remaining | Some components are installed; others are not, or an owned resource no longer matches the plan.                                             |
| `modified`           | Local Changes       | Protected         | An owned file was edited outside Agent Plugins. Disabled: it will not overwrite your edit.                                                  |
| `removed`            | Removed Upstream    | Uninstall         | Installed, but the source no longer publishes it.                                                                                           |
| `sourceConflict`     | Owned Elsewhere     | Owned Elsewhere   | Another source already owns this package ID. Disabled.                                                                                      |
| `conflict`           | Unmanaged Conflict  | Replace…          | Reserved for an unmanaged file at a destination. The current build reports that as an install error instead, so this state does not appear. |

Component rows carry the same states. A package whose components are all skills marked `disable-model-invocation` also shows **Manual Invocation**; a package published to the official lane shows **Official**.

An update applies only the components already installed on that package. Installing a component you skipped the first time is a separate action.

## Destinations

| Target         | Skills             | MCP configuration            |
| -------------- | ------------------ | ---------------------------- |
| Cursor         | `~/.agents/skills` | `~/.cursor/mcp.json`         |
| Claude Code    | `~/.claude/skills` | `~/.claude.json`             |
| Codex          | `~/.agents/skills` | `~/.codex/config.toml`       |
| OpenCode       | `~/.agents/skills` | user `opencode.jsonc`        |
| Grok Build     | `~/.agents/skills` | `~/.grok/config.toml`        |
| GitHub Copilot | `~/.agents/skills` | `~/.copilot/mcp-config.json` |

A skill directory is named `<sourceId>-<skillName>`. Every target except Claude Code shares one copy under `~/.agents/skills`; the ledger records each target as a consumer, and the directory is deleted only when the last one goes away. Shared configuration files are edited in place, preserving comments where the format allows, and untouched keys stay untouched.

Replacing an unmanaged destination, or force-removing modified content, first copies the original to `~/.agents/.skill-manager-backups`. The app reports the backup path when it makes one.

## Approvals and confirmations

| Operation                               | Prompt                                                                                    |
| --------------------------------------- | ----------------------------------------------------------------------------------------- |
| Install a package with an MCP server    | Names the package and lists each server's command, arguments, and environment variables.  |
| Install a package or component (no MCP) | None. Proceeds immediately.                                                               |
| Uninstall all / Replace all in a source | Confirms the count.                                                                       |
| Remove a source                         | Confirms the package count, and names any file with local changes that will be discarded. |
| Reset                                   | Confirms.                                                                                 |

MCP approval is per operation and is never inferred. A background update that would add or change an MCP server fails rather than approving itself, and reports the failure in the catalog notice.

## Background behavior

- **Detection.** Every installed agent is configured. Detection results are cached for 60 seconds; a sync or an on-demand diagnostics run clears that cache.
- **Sync.** Every 15 minutes, and on **Refresh** or **Check for Updates Now**. It runs preflight, refreshes the catalog, then the sources, subscribes to any catalog source not yet added, and posts a `heartbeat` event.
- **Updates.** Installed packages whose state is `updateAvailable` are updated during sync, without approval, so anything touching an MCP server is left pending and reported. Nothing is updated while no agent is detected.
- **Offline.** A failed refresh keeps the last validated snapshot. A failed catalog fetch does not stop source refresh.
- **Events.** `install`, `update`, `uninstall`, and `heartbeat` are posted to the marketplace from a background thread. Reporting never blocks or fails an operation.

## State on disk

| Path                                                | Contents                                                                               |
| --------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `%APPDATA%\skill-manager\sources.json`              | Configured catalogs and sources, version 6.                                            |
| `%APPDATA%\skill-manager\agent-profiles.json`       | Detected agent profiles.                                                               |
| `%APPDATA%\skill-manager\installations.json`        | The ownership ledger, version 4.                                                       |
| `%APPDATA%\skill-manager\resource-transaction.json` | The recovery journal. Present only while a transaction is in flight.                   |
| `%LOCALAPPDATA%\skill-manager\`                     | Source snapshots, the marketplace index, the preflight report, and the last sync time. |

Each of `sources.json`, `agent-profiles.json`, and `installations.json` keeps a `.previous` copy. On launch, a journal whose transaction is absent from the ledger is rolled back; one already committed is cleaned up.

## See also

- [Install your first package](install-a-package.md) — the guided walkthrough.
- [Troubleshooting](troubleshoot.md) — what to do when an install refuses.
- [Architecture](architecture.md) — why ownership, transactions, and trust tiers work this way.
