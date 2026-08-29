# How to fix a failed install

Use this when Agent Plugins refuses an operation, an agent does not see what you installed, or the status button turns red. Each section starts from what you actually saw. For the meaning of a badge or a check, see [the app reference](app-reference.md) and [the preflight reference](preflight-reference.md).

Before anything else, select **Refresh**. A stale snapshot explains a surprising number of these, and a sync re-detects your agents, re-reads the catalog, and re-runs every check.

## The agent does not see a skill you installed

Confirm the files exist first — that separates an Agent Plugins problem from an agent problem:

```powershell
dir $env:USERPROFILE\.agents\skills
dir $env:USERPROFILE\.claude\skills
```

The directory is named `<sourceId>-<skillName>`.

**The directory is there.** Your agent has not re-read it. Reload at that agent's own boundary: reload the window in Cursor, start a new session in Claude Code or Codex, or use the client's configuration surface for OpenCode, Grok Build, and Copilot. Nothing in Agent Plugins can force this.

**The directory is missing and the card says Installed.** The ledger and the disk disagree. Select **Refresh**; the card should move to **Local Changes** or **Partially Installed**, and the sections below apply.

**Only Claude Code is missing it.** Claude Code reads `~/.claude/skills`, not the shared directory. If **System status** does not list Claude Code under Agents, Agent Plugins never wrote that copy — install or repair Claude Code and select **Refresh**.

## The card says Local Changes and the button is disabled

Something edited a file Agent Plugins owns. It will not overwrite your edit, and it will not uninstall over it either.

To see everything in this state at once, open **System status**, expand **All checks**, and select **Show modified packages** on the `agents.drift` row. That filters the catalog to exactly those packages; **Show all** in the toolbar clears the filter.

Then choose:

- **Keep your edit.** Do nothing. The package stays as it is and stops receiving updates.
- **Move your edit somewhere Agent Plugins does not own.** Copy the changed file out of the skill directory into your own skill, then discard the change below.
- **Discard the edit and start clean.** Open **Manage Sources**, select **Remove** on that source, and confirm **Remove and Discard Changes** — the dialog names each file it will discard. Add the source again and reinstall the package.

**Reset** does the same for every source at once, and also deletes Agent Plugins' own config, cache, and data. Reach for it only when more than one source is tangled.

## An install fails with an error message

The message names the reason. Match it here.

| Message                                                                   | Cause                                                                                | Fix                                                                                                             |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------- |
| `… requires explicit Tier 3 approval`                                     | The package installs an MCP server and the approval was declined or never asked for. | Install it from the app and select **Approve and Install**, or pass `--approve-mcp` to `agent-plugins install`. |
| `… contains local changes`                                                | An owned file was edited outside the app.                                            | See _Local Changes_ above.                                                                                      |
| `… is owned by a different source`                                        | Another source already installed a package with this ID.                             | Uninstall the other copy, or remove the source that owns it, then install again.                                |
| `… conflicts with another package in this batch`                          | Two packages in one **Install all** declare `conflictsWith` each other.              | Install them individually and keep only one.                                                                    |
| `Configuration entry … is unmanaged` / `Instruction block … is unmanaged` | An agent's config file already has an entry at the key this package wants.           | Remove that entry from the agent's own config file by hand, then install again.                                 |
| `… is already managed; use the normal update operation`                   | A replace was requested for a package the app already owns.                          | Use **Update** instead.                                                                                         |

Every one of these fails before anything is written. Your machine is unchanged.

## Background updates failed

A notice above the catalog lists what a sync could not update. The usual cause is an update that would add or change an MCP server: sync never grants Tier 3 approval on your behalf, so it leaves the package pending.

Select **Update** on the package and approve it. Anything else in the notice is an ordinary install failure — look it up in the table above.

## The status button is red

Open it. The panel leads with three rows — Windows sign-in, marketplace server, agents — and shows the failing check's detail and its remediation. Select **Run diagnostics** to re-run every check against the machine as it is now.

| Row                | Common failure                                           | What to do                                                                                                                                                                        |
| ------------------ | -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows sign-in    | Not domain-joined, or no Kerberos ticket for the server. | Connect to the corporate network or VPN and sign in again. `klist` should show a ticket-granting ticket.                                                                          |
| Marketplace server | DNS, TLS, or the server itself is unreachable.           | Check the URL in the row resolves and answers. A failed server leaves the app offline, on the last snapshot it validated.                                                         |
| Marketplace server | The client is older than the server's minimum.           | Select **Update Agent Plugins**, which opens the download site when this build configures one and otherwise tells you to ask your administrator. This one blocks every operation. |
| Agents             | No supported coding agent found.                         | Install Cursor, Claude Code, Codex, OpenCode, Grok Build, or GitHub Copilot, then select **Refresh**.                                                                             |

Warnings never turn the button red. Expand **All checks** if you want to read them.

If a check fails and the message tells you nothing you can act on, the check's ID (`auth.ticket`, `host.tls`, and so on) is the thing to quote when you ask for help; [the preflight reference](preflight-reference.md) lists what each one probes.

## Everything is blocked

Three checks can stop the app from planning, installing, or syncing at all:

| Check                  | Meaning                                                    | Fix                                                                             |
| ---------------------- | ---------------------------------------------------------- | ------------------------------------------------------------------------------- |
| `host.homeDirs`        | A skill directory or the app data directory is unwritable. | Fix the permissions on `~/.agents`, `~/.claude`, and `%APPDATA%\agent-plugins`. |
| `agents.ledger`        | `installations.json` is unreadable or too new.             | Restore `installations.json.previous` beside it, or **Reset**.                  |
| `server.clientVersion` | The client is below the server's minimum.                  | Update the app.                                                                 |

## A source disappeared from Manage Sources

Sources you added that the catalog no longer lists move to **Other sources**, where the only action is **Remove**. Removing one uninstalls everything it installed. If you expected a source to be there and it is not, the catalog owner has to list it — the app does not take pasted URLs.

## An operation was interrupted

Close the app and start it again. On launch it reads the recovery journal: a transaction that never committed is rolled back, and one that did is cleaned up. Either way the machine ends in a state the ledger describes, and `agents.journal` reports what it did.

## Nothing here matches

Collect this before asking for help:

```powershell
agent-plugins whoami
```

plus the failing check IDs from **System status**, the exact error text, and the package ID. The report behind the status button is the same one the app sends with its heartbeat, so quoting check IDs lets someone match your machine to what the server already sees.
