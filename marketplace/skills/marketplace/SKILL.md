---
name: marketplace
description: Find and install skills, MCP servers, and bundles from the company marketplace with the Agent Plugins CLI. Use when the user asks whether a skill exists for a task, wants to search or browse the marketplace, asks to install a published skill or bundle by name, or pastes a marketplace link someone shared.
---

# Marketplace search and install

Agent Plugins' command line, `agent-plugins`, searches the company marketplace
and installs packages into every coding agent on this machine in one
transaction. Authentication is the user's Windows logon.

## Find the command

Try `agent-plugins help`. If it is not on PATH, use the installed executable:

- Windows: `"%LOCALAPPDATA%\Agent Plugins\agent-plugins.com"`. An older install has only `agent-plugins.exe`, which PowerShell neither waits for nor reads, so pipe it: `& "$env:LOCALAPPDATA\Agent Plugins\agent-plugins.exe" help | Out-String`.
- macOS (development): `/Applications/Agent Plugins.app/Contents/MacOS/agent-plugins`

If a command prints nothing in PowerShell, run it again piped to `Out-String`.

## Search

```
agent-plugins search <words>
```

Every word must appear somewhere in a package's name, description, ID,
publisher, or tags. Prints matching packages as `namespace/package`, version,
publisher, install count, active users, and tags, then matching bundles: named
sets of packages that install together. Add `--json` to read the results as
data. Summarize the best matches for the user and say who published each one;
publisher and usage numbers are how people judge trust here.

## Install

```
agent-plugins install <namespace>/<package>
agent-plugins install <namespace>/<package>/<skill>   # one skill from a package with several
agent-plugins install <namespace>/<bundle>            # every package in a bundle
agent-plugins install <link>                          # something shared with a marketplace link
```

A link someone shared looks like `https://…/l/<code>`; installing it first
gives the user access to what it shares. A team invite link is different: it
joins the user to a team with `agent-plugins team join <link>`, and only when
the user asks to join.

Skills install without further approval. If the package contains an MCP
server the CLI refuses unless `--approve-mcp` is given: MCP servers run code,
so describe what the server does and get the user's explicit approval before
adding the flag. After a successful install, tell the user which agents were
configured and that a running agent may need a reload to see the new skill.
Report any `warning:` lines, and anything the command says it updated or
removed on the way.

If the install refuses because files Agent Plugins didn't install are in the
way, ask the user before running it again with `--replace`, which backs those
files up and replaces them.

## What is installed

```
agent-plugins list                              # installed packages and their state
agent-plugins status <namespace>/<package>      # versions, reviews, usage, and this PC's state
agent-plugins uninstall <namespace>/<package>   # add /<skill> for one part
agent-plugins sync                              # check for updates now
```

`list` and `status` take `--json`. An uninstall refuses a package the user
edited; `--force` removes it anyway after saving their copy to backups, so
ask first. `hold <namespace>/<package>` stops a package updating on its own,
and `--undo` lets it update again.

## Rules

- Do not install a package the user did not ask for.
- Prefer packages in the `official` namespace when several match.
- Report the CLI's output faithfully; a failed transaction rolls back
  completely and nothing was changed.
- Exit codes: 3 means the package needs `--approve-mcp`, 4 means it wasn't
  found, 5 means the marketplace couldn't be reached.
