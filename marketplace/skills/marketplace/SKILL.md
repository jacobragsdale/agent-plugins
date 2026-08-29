---
name: marketplace
description: Find and install skills and MCP servers from the company marketplace with the Agent Plugins CLI. Use when the user asks whether a skill exists for a task, wants to search or browse the marketplace, or asks to install a published skill by name.
---

# Marketplace search and install

Agent Plugins' command line, `skill-manager`, searches the company marketplace
and installs packages into every coding agent on this machine in one
transaction. Authentication is the user's Windows logon.

## Find the command

Try `skill-manager help`. If it is not on PATH, use the installed executable:

- Windows: `"%LOCALAPPDATA%\Programs\Agent Plugins\skill-manager.exe"`
- macOS (development): `/Applications/Agent Plugins.app/Contents/MacOS/skill-manager`

## Search

```
skill-manager search <words>
```

Prints matching packages as `namespace/package`, version, publisher, install
count, active users, and tags. Summarize the best matches for the user and
say who published each one; publisher and usage numbers are how people judge
trust here.

## Install

```
skill-manager install <namespace>/<package>
```

Skills install without further approval. If the package contains an MCP
server the CLI refuses unless `--approve-mcp` is given: MCP servers run code,
so describe what the server does and get the user's explicit approval before
adding the flag. After a successful install, tell the user which agents were
configured and that a running agent may need a reload to see the new skill.

## Rules

- Do not install a package the user did not ask for.
- Prefer packages in the `official` namespace when several match.
- Report the CLI's output faithfully; a failed transaction rolls back
  completely and nothing was changed.
