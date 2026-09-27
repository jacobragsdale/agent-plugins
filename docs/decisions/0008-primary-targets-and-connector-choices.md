# ADR 0008: Primary apps, connector choices, and approvals that follow changes

- Status: accepted
- Date: 2026-09-26
- Amends: [ADR 0001](0001-multi-agent-desired-state.md) (detection is the configuration set, for connectors), [ADR 0006](0006-web-portal-and-review.md) (no in-browser editor), and [ADR 0007](0007-self-service-marketplace.md) (MCP approval on every update)

## Context

A review of every workflow, from a first install to an admin's incident, found that the app promised apps it could not serve and asked for approval it did not need.

Microsoft 365 Copilot was a target by mistake. The people who use Agent Plugins work in GitHub Copilot, Cursor, and Claude, and the window, the portal, and the docs listed those apps next to one they do not use. The portal said a skill would reach Claude Desktop, which takes skills only from a claude.ai account. VS Code, where most people use GitHub Copilot, never saw an MCP server, because the app wrote only the Copilot CLI's file.

Every update of a package with an MCP server waited for approval, even when only a skill changed, so those packages fell behind. A person could not keep a connector out of one app: deleting its entry by hand was undone at the next sync. A server that needed an API key installed without saying so, and a beginner had no way to set one.

Writing a skill needed files or Cursor. The portal took uploads only, and **Create a skill** worked only in Cursor.

## Decision

### The primary apps

GitHub Copilot, Cursor, and Claude (Claude Code, and Claude Desktop for MCP servers) are the primary targets. They come first in every list, and the copy names them. OpenCode and pi are supported; Codex, the ChatGPT app's Codex mode, and Grok Build stay.

Microsoft 365 Copilot is removed. A saved profile that names it is dropped, and the next sync releases what a ledger recorded for it, which removes those copies from OneDrive.

pi reads `~/.agents/skills`, so it shares the skill copy the other apps use. It has no MCP support by design, so connectors report it unsupported. It counts as installed only once `~/.pi/agent` exists, because `pi` alone is too common a command name to prove anything.

GitHub Copilot's MCP servers also go to the user `mcp.json` of each VS Code edition that has been started.

### Each app gets its own spelling

An adapter writes an MCP server the way its app reads one: its `type` values, and an environment reference as `${NAME}`, `${env:NAME}`, `{env:NAME}`, or Codex's forwarded variables. A placement an app cannot express is unsupported for that app, with the reason, instead of a file the app misreads. Claude Desktop does not expand references, so it takes only servers that read none.

The window says which apps an item goes to, and why none can take it when that is so.

### Approval covers what was approved

An approval covers the MCP entries the ledger records. An update whose entries are all unchanged installs like any other, so a skill fix reaches everyone. A new or changed entry waits for the person, and the dialog marks the connector as changed.

When a newer app spells an entry differently, a sync rewrites the approved entries for the apps that already have them, without asking. A connector never spreads to a newly detected app without approval.

### A person may keep a connector out of an app

Detection still decides which apps exist, but a person may keep one connector out of chosen apps. The choice is saved per component, the planner applies it before any adapter runs, and updates keep it. Putting an app back is an install, so it asks for approval. Skills do not get this choice; a shared skill copy cannot be withheld from one app anyway.

### Settings a connector needs

The approval dialog lists every environment variable a connector reads and saves what the person types to their user environment (`HKCU\Environment` on Windows), the way **Edit environment variables for your account** would. The value never goes to the ledger, the log, or the marketplace. Connectors in the portal and the app say whether an admin checked them.

### Writing a skill in the portal

ADR 0006 kept the portal to uploads. The portal now has **Write it here**: a name, when the assistant should use the skill, and instructions, which the browser turns into a `SKILL.md`. **Edit** opens a published single-file skill in the same form. It is the only way to write a skill for people without Cursor, and **Create a skill** in the app opens it for them.

### The app keeps running

Launch at Login is on from the first run, because removals and updates reach a PC only while the app runs. A person can turn it off in the tray, and IT can force it either way with the `LaunchAtLogin` policy, next to `MarketplaceUrl` and `DownloadUrl`.

### Withdrawing moves PCs back

The app updates whenever a package's published content changes, so withdrawing the live version moves every PC to the version below it at the next sync, subject to the same MCP approval. The docs used to say installed copies were left alone.

## Consequences

The primary audience sees only apps it uses, and a connector that needs a key says so before it is installed. Packages with connectors stay current. Power users can keep a connector out of an app without fighting the sync, and can keep their own edited copy with **Keep my version**.

The cost is an adapter surface that follows each app's own spelling, which has to be re-checked when an app changes its format. Only some of those spellings have been checked against the running apps. Saved connector values are plain text in the user's environment, as they would be if the person set them by hand. A choice to keep a connector out of an app is local to one computer.
