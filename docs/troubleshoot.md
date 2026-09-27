# How to fix a failed install

Use this when Agent Plugins refuses an operation, an agent does not see what you installed, a connector does not work, or the **Status** button turns red. Each section starts from what you actually saw. For the meaning of a badge or a check, see [the app reference](app-reference.md) and [the preflight reference](preflight-reference.md).

Before anything else, select **Refresh**. A stale snapshot explains a surprising number of these, and a sync re-detects your agents, re-reads the catalog, puts back missing files, and re-runs every check.

Some failures need nothing from you. Agent Plugins retries a locked file or a dropped connection once on its own, puts back files that were deleted by hand, recovers its own damaged settings files from the last good copy, and keeps a source that briefly disappears from the server. The [app reference](app-reference.md#automatic-retries) lists what it retries and when.

## The agent does not see a skill you installed

Confirm the files exist first — that separates an Agent Plugins problem from an agent problem:

```powershell
dir $env:USERPROFILE\.agents\skills
dir $env:USERPROFILE\.claude\skills
```

The directory is named `<sourceId>-<skillName>`.

**The directory is there.** Your agent has not re-read it. Reload at that agent's own boundary: reload VS Code (or start a new Copilot CLI session) for GitHub Copilot, reload the window in Cursor, start a new session in Claude Code, OpenCode, pi, Codex, or Grok Build, and switch to Codex in the ChatGPT app. The notice after the install says what to type in each. Nothing in Agent Plugins can force this.

**The app is Claude Desktop.** Claude Desktop does not read skills from this computer; its Chat and Cowork tabs use the skills on your claude.ai account (Customize > Skills). The card says so with `Can't be added here` when Claude Desktop is your only app. It does take connectors.

**The directory is missing.** Select **Refresh**. The card reads **Restoring on next check** until a sync re-creates the files from the saved copy; it does this quietly. If the card says **Changed on this computer** instead, another file of the package was edited; see the next section.

**Only Claude Code is missing it.** Claude Code reads `~/.claude/skills`, not the shared directory. If **System status** does not list Claude Code under Agents, Agent Plugins never wrote that copy — install or repair Claude Code and select **Refresh**.

## The card says Changed on this computer

Something edited a file Agent Plugins owns. It will not update or uninstall over your edit.

To see everything in this state at once, open **System status**, expand **All checks**, and select **Show modified packages** on the `agents.drift` row. That filters the catalog to exactly those packages; **Show all** in the toolbar clears the filter.

Then choose:

- **Make it yours.** Choose **More** > **Keep my version…**. Agent Plugins stops managing the package and leaves your files as they are: no more updates, restores, or removal. Install the package again later if you want the published one back. `agent-plugins keep <ns>/<package>` does the same.
- **Restore the original.** Select **Restore original…** and confirm **Back up and restore**. Your changed copy goes to `~/.agents/.agent-plugins-backups`, and **Open folder** in the result notice shows it.
- **Remove it.** Choose **More** > **Remove…**, or run `agent-plugins uninstall <ns>/<package> --force`. Your changed copy is backed up first.

Doing nothing also works: the package stays as it is and stops receiving updates. Backups older than 30 days are deleted.

A `__pycache__` folder, `.DS_Store`, or similar leftover in a skill folder does not count as a change.

## An install fails with an error message

The notice's first line says what failed; the reason is under **Details**. Match it here.

| Message                                                                                   | Cause                                                                                | Fix                                                                                                                                                                     |
| ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `… requires explicit Tier 3 approval`                                                     | The package installs an MCP server and the approval was declined or never asked for. | Install it from the app and select **Allow and install**, or pass `--approve-mcp` to `agent-plugins install`.                                                           |
| `… contains local changes`                                                                | An owned file was edited outside the app.                                            | See _Changed on this computer_ above.                                                                                                                                   |
| `… could not be checked, so it cannot be …`                                               | An owned file or an agent's settings file could not be read.                         | Follow the rest of the message: fix or remove the named file, or check that you can read it, then try again.                                                            |
| `… another app is using it …`                                                             | A file stayed locked through the automatic retry.                                    | Close the app the message names, or whatever has that folder open, then try again.                                                                                      |
| `… the disk is full`                                                                      | No space left on the drive being written.                                            | Free up space, then try again.                                                                                                                                          |
| `A newer version of Agent Plugins manages the packages on this computer.`                 | A newer build wrote the ledger. This build shows it but will not change it.          | Update Agent Plugins.                                                                                                                                                   |
| `… is owned by a different source`                                                        | Another source already installed a package with this ID.                             | Uninstall the other copy, or remove the source that owns it, then install again.                                                                                        |
| `… conflicts with another package in this batch`                                          | Two packages in one **Install all** declare `conflictsWith` each other.              | Install them individually and keep only one.                                                                                                                            |
| `… can't be installed while … is installed; uninstall … first.`                           | The two packages declare `conflictsWith`.                                            | Uninstall the named package, then install again.                                                                                                                        |
| `None of the AI apps on this computer can use …`                                          | Every detected app reports the component unsupported; the reasons follow.            | Follow the reason, for example add a remote connector in the app's own settings.                                                                                        |
| `Configuration entry … is unmanaged` / `… already exists and is not an owned destination` | A folder or config entry you or another tool created is where the package goes.      | The card shows **Files already there**: select **Replace…** to back it up and replace it, or remove it by hand and install again. On the command line, add `--replace`. |
| `… is already managed; use the normal update operation`                                   | A replace was requested for a package the app already owns.                          | Use **Update** instead.                                                                                                                                                 |

Each of these leaves your machine unchanged: the operation either fails before anything is written or rolls back completely.

## A notice says an AI app was skipped

The package installed for every other app; the amber result notice names the app and its settings file.

| Notice                                                                             | Fix                                                                                                                                                                              |
| ---------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<App> uses a settings file that Agent Plugins could not read: <path>.`            | Fix the error the message quotes, or remove the file if you do not use it, then select **Refresh**. The sync adds the package to that app, as long as Agent Plugins stayed open. |
| `<App> did not get the package: Agent Plugins isn't allowed to write to <folder>.` | Get write access to that folder, then select **Refresh**, or install the package again.                                                                                          |

An app that merely had its settings file open is skipped without a notice; the next sync adds the package to it, as long as Agent Plugins stays open until then.

## A package stays Update available

Background updates that fail are retried on later syncs without a notice. A package that keeps its **Update available** badge failed every time, is held (the card shows **Updates held**; **More** > **Resume automatic updates** undoes it), or changes an MCP server you allowed: select **Update** to apply it, and allow the connector when asked. If it fails, the error says why — look it up in the table above.

## A connector does not work

Check the card first:

- `Needs <program>, which isn't on this computer` — the connector starts a program you don't have, such as Node.js for `npx`. Install it, or ask IT to, then quit and reopen the AI app.
- `Needs <NAME> before it works.` — the connector reads a setting such as an API key. Select **Set…**, paste the value from the publisher's instructions, save, then quit and reopen the AI app. **More** > **Connector settings…** changes a value later.

If the card shows neither, check which apps got it: the notice after the install lists them, and **Apps…** (or **More** > **Choose apps…**) shows them. Claude Desktop takes only connectors that start a program and read no setting from your environment; add others in Claude Desktop under Settings > Connectors. pi does not use connectors at all.

## The window says Offline

The amber notice `Offline — showing skills as of <time>` means the last sync reached no server. Installed packages keep working, and Agent Plugins retries within minutes on its own. A source that alone could not be refreshed — its server was down, or answered with an error — shows a grey **Saved copy from <time>** badge instead, whose tooltip gives the reason.

If it persists, connect to the corporate network or VPN and select **Try now**. If one server stays unreachable while others answer, open **System status** and read the **Marketplace server** row.

## The Status button is red

Open it. The panel leads with three rows — Windows sign-in, marketplace server, agents — and shows the failing check's detail and its remediation. Select **Run diagnostics** to re-run every check against the machine as it is now.

| Row                | Common failure                                                                        | What to do                                                                                                                                                                        |
| ------------------ | ------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows sign-in    | Not joined to the domain or Microsoft Entra ID, or no Kerberos ticket for the server. | Connect to the corporate network or VPN and sign in again. `klist` should show a ticket-granting ticket.                                                                          |
| Marketplace server | `The security certificate of <server> isn't trusted by this computer`.                | Check the computer's date and time. Otherwise your network inspects secure connections: ask IT to add its certificate authority to Windows.                                       |
| Marketplace server | DNS, TLS, or the server itself is unreachable.                                        | Check the URL in the row resolves and answers. A failed server leaves the app offline, on the last snapshot it validated.                                                         |
| Marketplace server | The client is older than the server's minimum.                                        | Select **Update Agent Plugins**, which opens the marketplace's download page, or the address IT set.                                                                              |
| Agents             | No supported AI app found.                                                            | Install GitHub Copilot, Cursor, or Claude (Claude Code or Claude Desktop), or another supported app such as OpenCode, pi, Codex, ChatGPT, or Grok Build, then select **Refresh**. |

An agent listed as `couldn't check just now` is not a failure: its detection timed out, and Agent Plugins keeps configuring it and checks again.

Warnings never turn the button red. Expand **All checks** if you want to read them.

If a check fails and the message tells you nothing you can act on, the check's ID (`auth.ticket`, `host.tls`, and so on) is the thing to quote when you ask for help; [the preflight reference](preflight-reference.md) lists what each one probes.

## Every install and update fails

Two checks mean every install and update will fail until they are fixed. The catalog still loads and syncs.

| Check           | Meaning                                                                                                                    | Fix                                                                                                                                                                                                                                 |
| --------------- | -------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `host.homeDirs` | Agent Plugins' own data or cache directory is unwritable.                                                                  | Get write access to `%APPDATA%\agent-plugins` and `%LOCALAPPDATA%\agent-plugins`. An unwritable agent skill directory is only a warning: skills cannot go to that agent.                                                            |
| `agents.ledger` | `installations.json` exists but cannot be read, for example because of its permissions or a lock.                          | Close anything that has the file open and check you can read it. Otherwise select **Reset…** under **Advanced** in **System status**, which works without a readable ledger: it removes what it can find and clears the app's data. |
| `agents.ledger` | A newer version of Agent Plugins wrote `installations.json`, for example on another computer sharing your roaming profile. | Select **Update Agent Plugins** and install the newer version.                                                                                                                                                                      |

A ledger that is merely damaged does not fail this check: Agent Plugins sets it aside as `installations.json.corrupt-<timestamp>` and falls back to `installations.json.previous` on its own. The same applies to `sources.json` and `agent-profiles.json`.

## A source was not found on the server

A source whose **Saved copy** badge's tooltip reads `<name> was not found on the server (first noticed <date>)` answers 404: it was unpublished, or you may no longer see it. Packages you installed from it stay installed and can still be removed, but nothing new is installed from its saved copy: an install says the package was not found on the server (exit status 4 in the CLI). Agent Plugins retires it only after 3 not-found results spanning at least 3 days; a successful fetch in between starts the count over. The default catalog is never retired.

If you expected to keep access, see [A package or source you expected is missing](#a-package-or-source-you-expected-is-missing) before the grace period ends.

## A source disappeared from Manage sources

**Manage sources…** is under **Advanced** in **System status**. Sources you added that the catalog no longer lists move to **Other sources**, where the only action is **Remove**. Removing one uninstalls everything it installed. If you expected a source to be there and it is not, the catalog owner has to list it — the app does not take pasted URLs.

A source retired as not found disappears on its own once nothing from it is installed, and **Manage sources** says `Removed the retired source <name>.`

## A package or source you expected is missing

The marketplace shows each person only what they may see. A private space, package, or bundle is absent from the catalog, from `agent-plugins search`, and from the app for anyone it is not shared with, and nothing says so.

Ask the owner to share it with you: from its **Share** button in the portal, they can add you, add a team you are in, or send you a share link. On the command line it is `agent-plugins share <namespace>/<package> --add <your account>`. If the owner shared it with a team, run `agent-plugins whoami` and check that the team is on its `teams` line.

A package with an MCP server that is shared with everyone stays hidden from people outside its space until an admin approves it. Its page tells the owner when it is waiting.

If you had installed the package, its card now says **No longer offered** with an **Uninstall** button, and the source stays in **Manage sources** until nothing from it is installed.

## An operation was interrupted

Nothing to do in most cases. Before every read and change, Agent Plugins checks the recovery journal: a transaction that never committed is rolled back, and one that did is cleaned up. Either way the machine ends in a state the ledger describes, and `agents.journal` warns while a journal is still waiting.

If a message says `An earlier change could not be undone yet` or `An earlier change is still being undone`, a file the rollback needs is locked. Close the app that is using it; the next sync finishes the rollback, and new changes are refused until it does.

## Agents keep using an old proxy

Builds before this one copied the Windows proxy setting into your user environment variables, where agents kept using it after the proxy changed. On its first launch this build removes each copied `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, and `NO_PROXY` value that still matches the current Windows proxy, once per computer. It now applies the Windows proxy to its own process only.

A copy of an older proxy setting cannot be told apart from one you set yourself, so it stays. Remove it by hand:

1. Open **Edit environment variables for your account** from the Start menu.
2. Delete the stale `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, or `NO_PROXY` entries under **User variables**.
3. Restart Agent Plugins and the affected agents.

## Nothing here matches

Collect this before asking for help:

```powershell
agent-plugins whoami
```

plus the failing check IDs from **System status**, the exact error text and its **Details**, the package ID, and the log: **System status** > **Advanced** > **Open log** shows `agent-plugins.log` in its folder. The report behind the **Status** button is the same one the app sends with its heartbeat, so quoting check IDs lets someone match your machine to what the server already sees. Any `*.corrupt-<timestamp>` file in `%APPDATA%\agent-plugins` is a damaged copy the app set aside; include it too.
