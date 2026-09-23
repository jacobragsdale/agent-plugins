# Install your first package

This tutorial takes you from a freshly installed Agent Plugins to a skill your coding agent can use, and then to an MCP server, which needs one extra approval. It takes about ten minutes, and everything you do here is reversible.

You need:

- Windows 11 on the corporate domain.
- Agent Plugins installed, and at least one of Claude Desktop, ChatGPT, Microsoft 365 Copilot, Cursor, Claude Code, Codex, OpenCode, Grok Build, or GitHub Copilot.
- Nothing else. Your Windows logon is the sign-in; there is no account to create.

## Open the app and read the header

Launch **Agent Plugins**. The window lists every package the marketplace publishes, grouped by the source that published it.

Look at the top right. There are four controls:

| Control            | What it does                                                             |
| ------------------ | ------------------------------------------------------------------------ |
| Your namespace     | Opens **System status**. A green dot means every check passed.           |
| **Manage sources** | Lists the sources the catalog offers, and which of them you have added.  |
| **Refresh**        | Re-reads the catalog and the sources now, instead of waiting 15 minutes. |
| **Reset**          | Uninstalls everything and deletes Agent Plugins' own data.               |

If the button reads a check name in red rather than your namespace, something failed. Open it, read the row, and follow [the troubleshooting guide](troubleshoot.md) before continuing.

## Confirm the app found your agent

Select the button with your namespace to open **System status**. The panel leads with three rows: your Windows sign-in, the marketplace server, and the agents on this machine.

Read the **Agents** row. It should name the agent you have installed, with its version, and then say where skills will go, for example:

```text
Cursor 1.7.55
Skills go to ~/.agents/skills.
```

You never choose agents. Agent Plugins configures every agent it finds, and stops configuring one that disappears. If the row says no supported agent was found, install one, then select **Refresh**.

Close the panel.

## Install a skill

In the search box, type part of a package name. The count beside the box tells you how many packages matched.

Pick a package whose badges say **Skill** and not **Connector** — a plain skill needs no approval, so this first install shows you the shortest path. Select **Install**.

The button spins briefly and the card changes to **Installed**. Nothing else happens: no dialog, no restart. Agent Plugins wrote the skill, recorded that it owns those files, and committed the change in one transaction. If any part had failed, all of it would have rolled back and the card would still read **Available**.

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

Files on disk prove Agent Plugins did its job. They do not prove your agent noticed. Agents read skills at their own boundaries, so reload yours first:

- **Cursor** — reload the window, then open the Skills settings surface.
- **Claude Code** — start a new session; the skill appears in its skills listing.
- **Codex** — start a fresh session.
- **OpenCode**, **Grok Build**, **GitHub Copilot** — use that client's own configuration or skills listing.
- **ChatGPT** — quit and reopen the app, switch to Codex, and type `$` to see the skill.
- **Microsoft 365 Copilot** — wait for OneDrive to finish syncing, then start a new Cowork conversation.
- **Claude Desktop** — skills are not installed to disk; the Chat and Cowork tabs use the skills enabled on your claude.ai account under Customize > Skills. MCP servers from the config file appear under Settings > Developer.

Your skill should appear under its prefixed name. That round trip — install, reload, confirm — is the one to repeat whenever you doubt an install.

## Install a package that runs an MCP server

Now search for a package whose badges include **Connector**, and select **Install**. A connector is how the window names an MCP server.

This time a dialog appears before anything is written:

```text
Allow connector
<Package> includes a connector that runs a program on this computer. Every AI app found here will run it.
database: node server.js
```

The difference matters. A skill is text your agent reads. An MCP server is a program your agent starts on this machine, so Agent Plugins shows you the command, its arguments, and the environment variables it wants, and writes nothing until you select **Allow and install**.

Select **Allow and install**. The card becomes **Installed**, and the server is registered in each agent's own MCP configuration file — `~/.cursor/mcp.json`, `~/.claude.json`, `~/.codex/config.toml`, and so on. Agent Plugins edits those files in place and leaves your own entries and comments alone.

Select **Cancel** instead, and nothing at all is written. Approval is never remembered and never inferred: a background update that would add or change an MCP server waits for you rather than approving itself.

## Undo it

Select **Uninstall** on the package you just installed. It disappears from the agent configuration, and the card returns to **Available**.

Uninstall the skill too, then check the directory again:

```powershell
dir $env:USERPROFILE\.agents\skills
```

The prefixed directory is gone. A shared skill is deleted only when its last consumer goes away, so if you had installed it for four agents, uninstalling the package removes all four bindings and then the one directory.

## What you learned

- Agent Plugins configures every agent it detects; there is no per-agent switch.
- Skills are files, shared at `~/.agents/skills`, with Claude Code keeping its own copy.
- MCP servers run code, so they cost you one approval that shows the exact command.
- Every operation is a single transaction: it fully applies or fully rolls back.

Next:

- [Publish to the marketplace](publish-to-marketplace.md) — put your own skill in that catalog.
- [App reference](app-reference.md) — every package state, destination, and background behavior.
- [Troubleshooting](troubleshoot.md) — when an install refuses or an agent does not see a skill.
