# Install your first package

This tutorial takes you from a freshly installed Agent Plugins to a skill your coding agent can use, and then to an MCP server, which needs one extra approval. It takes about ten minutes, and everything you do here is reversible.

You need:

- Windows 11, joined to the corporate domain or to Microsoft Entra ID.
- Agent Plugins installed, and at least one of GitHub Copilot, Cursor, Claude Code, or Claude Desktop. OpenCode, pi, Codex, ChatGPT, and Grok Build work too.
- Nothing else. Your Windows logon is the sign-in; there is no account to create.

## Open the app and read the header

Launch **Agent Plugins**. The window lists every package the marketplace publishes, grouped by the source that published it.

Look at the top right. These are the controls this tutorial uses:

| Control     | What it does                                                             |
| ----------- | ------------------------------------------------------------------------ |
| **Status**  | Opens **System status**. A green dot means every check passed.           |
| **Help**    | Opens the marketplace's getting-started page in your browser.            |
| **Refresh** | Re-reads the catalog and the sources now, instead of waiting 15 minutes. |

If the **Status** button reads a check name in red, something failed, and a plain sentence under the header says what. Open it, read the row, and follow [the troubleshooting guide](troubleshoot.md) before continuing.

## Confirm the app found your agent

Select **Status** to open **System status**. The panel leads with three rows: your Windows sign-in, the marketplace server, and the agents on this machine.

Read the **Agents** row. It should name the agent you have installed, with its version, and then say where skills will go, for example:

```text
Cursor 1.7.55
Skills go to ~/.agents/skills.
```

You never choose agents. Agent Plugins configures every agent it finds, and stops configuring one that disappears. If the row says no supported agent was found, install one, then select **Refresh**.

Close the panel.

## Install a skill

In the search box, type a word or two from a package's name or description. The count beside the box tells you how many packages matched.

Pick a package whose badges say **Skill** and not **Connector** — a plain skill needs no approval, so this first install shows you the shortest path. Select **Install**.

The button spins briefly, the card changes to **Installed**, and a notice says how to use the skill in each app that got it:

```text
Review workflow is installed. In Cursor: reload the window, then type /acme-review or just ask. In Claude Code: start a new session and type /acme-review.
```

There was no dialog. Agent Plugins wrote the skill, recorded that it owns those files, and committed the change in one transaction. If any part had failed, all of it would have rolled back and the card would still read **Available**.

If the card says `Can't be added here` instead of offering **Install**, none of your apps can take it. The line gives the reason; for example, Claude Desktop takes skills only from your claude.ai account.

## See what it wrote

Open a terminal and list the skill directory:

```powershell
dir $env:USERPROFILE\.agents\skills
```

You will see a directory named `<source>-<skill>` — the publisher's namespace, a hyphen, and the skill name. The prefix is why two publishers can both ship a `review` skill without colliding.

If you have Claude Code, it keeps its own copy, because it reads a different directory:

```powershell
dir $env:USERPROFILE\.claude\skills
```

Every other agent shares `~/.agents/skills`. Installing one skill for four agents writes the files once; the ledger records four consumers of that one directory.

## Check that your agent loaded it

Files on disk prove Agent Plugins did its job. They do not prove your agent noticed. Agents read skills at their own boundaries, so do what the notice said for yours:

- **GitHub Copilot** — reload VS Code, or start a new Copilot CLI session. Copilot uses the skill when your request fits.
- **Cursor** — reload the window, then type `/` and the skill's name, or open the Skills settings.
- **Claude Code** — start a new session; the skill appears in its skills listing.
- **OpenCode** and **Grok Build** — start a new session.
- **pi** — start a new session and type `/skill:` to list skills.
- **Codex** — start a fresh session and type `$`.
- **ChatGPT** — switch to Codex in the app and type `$` to see the skill.
- **Claude Desktop** — skills are not installed to disk; the Chat and Cowork tabs use the skills enabled on your claude.ai account under Customize > Skills. MCP servers from the config file appear under Settings > Developer after you quit and reopen the app.

Your skill should appear under its prefixed name. That round trip — install, reload, confirm — is the one to repeat whenever you doubt an install.

## Install a package that runs an MCP server

Now search for a package whose badges include **Connector**, and select **Install**. A connector is how the window names an MCP server.

This time a dialog appears before anything is written:

```text
Allow connector?
A connector is a program on this computer or an online service that your AI apps use. Allow it only if you trust who published it.

Review workflow   Checked by IT
From Acme
acme-database: Starts a program called node on this computer.
Goes to Cursor and Claude Code.
acme-database runs: node server.js
```

The difference matters. A skill is text your agent reads. An MCP server is a program your agent starts on this machine, so Agent Plugins shows you what it runs, which apps get it, and whether IT checked it, and writes nothing until you select **Allow and install**. **Checked by IT** means an admin let everyone at the company see it; **Not checked by IT** means you see it through a team or a share without that check.

If the connector needs a setting such as an API key, the dialog lists it under **Settings it needs** with a field to paste it in. Agent Plugins saves it for your Windows account, where your AI apps read it. You can also leave it empty and add it later with **Set…** on the card.

Select **Allow and install**. The card becomes **Installed**, and the server is registered in each agent's own MCP configuration file — `~/.cursor/mcp.json`, `~/.claude.json`, `~/.copilot/mcp-config.json`, and so on. Agent Plugins edits those files in place and leaves your own entries, their order, and your comments alone.

Select **Cancel** instead, and nothing at all is written. Approval covers exactly what you saw: a later update that changes the server asks again, and one that leaves it alone installs on its own.

## Install from the marketplace website

The marketplace website can hand a package to the app, so you can install while you read about it. Open the marketplace in your browser, pick a package, and select **Install in Agent Plugins** beside its files.

The first time, the browser asks before it opens the app:

```text
This site is trying to open Agent Plugins.
https://marketplace.example.com wants to open this application.
```

Tick **Always allow**, then select **Open**. The browser never asks again for this site.

Agent Plugins comes to the front, starting if it was closed, and asks in its own window:

```text
Install Writing Pack?
e2e-bob · v1.0.0
Adds 2 skills to Cursor and Claude Code.
```

The apps named are the ones that will get it. Select **Install**. A connector still shows the **Allow connector?** dialog from the previous section; the website cannot approve one for you. When you switch back to the browser, the button reads **Installed** once the app has reported the install, which takes a few seconds.

A skill pack also offers **Install one skill instead**, with an **Install** button for each skill in it. A bundle page offers **Install all**, which lists every package in the bundle and asks once for any connectors among them.

If the page says **Get Agent Plugins**, the marketplace has not heard from the app on your PC in the last 30 days; install the app, open it once, and reload the page. If it says **Update Agent Plugins**, your app is older than 0.2.0 and cannot open website links.

Administrators can skip the browser's question for everyone: the Edge and Chrome policy `AutoLaunchProtocolsFromOrigins` with the protocol `agent-plugins` and the marketplace's origin allows the site to open the app without asking.

## Undo it

Select **Uninstall** on the package you just installed. It disappears from the agent configuration, and the card returns to **Available**.

Uninstall the skill too, then check the directory again:

```powershell
dir $env:USERPROFILE\.agents\skills
```

The prefixed directory is gone. A shared skill is deleted only when its last consumer goes away, so if you had installed it for four agents, uninstalling the package removes all four bindings and then the one directory.

## What you learned

- Agent Plugins configures every agent it detects. The one exception is yours to make: **Apps…** on an installed connector keeps it out of the apps you clear.
- Skills are files, shared at `~/.agents/skills`, with Claude Code keeping its own copy.
- MCP servers run code, so they cost you one approval that shows what they run, and another only when they change.
- Every operation is a single transaction: it fully applies or fully rolls back.

Next:

- [Publish to the marketplace](publish-to-marketplace.md) — put your own skill in that catalog.
- [App reference](app-reference.md) — every package state, destination, and background behavior.
- [Troubleshooting](troubleshoot.md) — when an install refuses or an agent does not see a skill.
