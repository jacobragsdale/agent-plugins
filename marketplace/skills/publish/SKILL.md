---
name: publish
description: Publish a skill or MCP server configuration to the company marketplace with the Agent Plugins CLI, for the user or one of their teams, or suggest a change to someone else's. Use when the user wants to share, publish, release, or upload a skill they wrote, bump a published version, improve a colleague's skill, or asks how to put something on the marketplace.
---

# Publish to the marketplace

Agent Plugins ships a command line, `agent-plugins`, that publishes a package
under the user's own namespace or one of their teams. Authentication is the
user's Windows logon, so there is nothing to log in to. A published version is
live at once; there is no review queue: people who can see it can find it and
install it, and PCs that already have it update within about 15 minutes. You do the judgment work (description, tags,
version, changelog); the CLI does the trust-sensitive work (validation, secret
scan, upload).

## Find the command

Try `agent-plugins help`. If it is not on PATH, use the installed executable:

- Windows: `"%LOCALAPPDATA%\Agent Plugins\agent-plugins.com"`. An older install has only `agent-plugins.exe`, which PowerShell neither waits for nor reads, so pipe it: `& "$env:LOCALAPPDATA\Agent Plugins\agent-plugins.exe" help | Out-String`.
- macOS (development): `/Applications/Agent Plugins.app/Contents/MacOS/agent-plugins`

If a command prints nothing in PowerShell, run it again piped to `Out-String`.

Every command below accepts the full path in place of `agent-plugins`.

## Steps

1. **Identify what to publish.** Ask the user which skill if it is unclear. A
   skill is a directory with `SKILL.md`; look in `~/.claude/skills`,
   `~/.agents/skills`, and the project's `.claude/skills`. An MCP server is a
   JSON document in the Agent Plugins `mcp.json` shape. Do not publish a skill
   that Agent Plugins installed from the marketplace (its name starts with a
   publisher namespace such as `jacob-`) unless the user owns it.
2. **Confirm identity and space.** Run `agent-plugins whoami`. If its `teams`
   line names any teams, ask whether the package is just for the user or for
   one of those teams. Tell the user the namespace it will publish under: the
   team's, or the `namespace` line. Pass a team as `--namespace <team>`. Just
   for the user means `--private`: only they can see it until they share it.
   A first publish with `--private` is also the way to try a package before
   anyone else sees it.
3. **Validate.** Run `agent-plugins validate <path> [--namespace <team>]`. It
   takes everything `publish` takes, checks it the way the marketplace will,
   runs the secret scan, and lists which AI apps can use each skill and MCP
   server. Fix what it reports before going on.
4. **Improve the listing.** Read `SKILL.md`. The frontmatter `description` is
   what other people's agents use to decide when to trigger the skill, so make
   sure it states _what_ the skill does and _when_ to use it, in one or two
   sentences, with concrete trigger phrases. Propose an edit if it is vague;
   apply it only with the user's agreement.
5. **Choose version, tags, and changelog.** Run `agent-plugins search <name>`
   to see whether a version is already published. Propose the next semantic
   version (`1.0.0` for a first publish; bump patch for fixes, minor for new
   behavior). Propose up to five lowercase tags that someone would search for.
   Write a one-line changelog.
6. **Dry run.** Run the publish command with `--dry-run`. The marketplace
   checks everything, including the version number, and lists the files that
   are new, changed, or removed compared with the live version; nothing is
   published. Show the result to the user and ask them to confirm.
7. **Publish.** Run the same command with `--yes` instead of `--dry-run`:

   ```
   agent-plugins publish <path> [--namespace <team>] [--private] --version <semver> --tags <a,b,c> --changelog "<text>" --yes
   ```

   Report the printed marketplace URL and any `warning:` lines. `published`
   means people who can see it can find it and install it now, and PCs that
   already have it update within about 15 minutes. For an MCP server that
   everyone can see, the CLI adds that others see it once an admin approves
   it; the user's team has it at once. If the version is taken or lower than
   the live one, use the version the error suggests. If the command timed out,
   run it again: it says when that version is already published. If the CLI
   refuses because of a secret or a validation error, show the message and fix
   the cause with the user; never work around the secret scan.

8. **Check on it later.** `agent-plugins status <ns>/<package>` shows the live
   version, every version, the MCP review state, and how many people use it.
   If a version turns out bad, `agent-plugins withdraw <ns>/<package> <version>`
   takes it back; PCs move to the newest version left at their next check.

## Improving someone else's package

To change a package the user doesn't own, publish the edited copy to the
owner's namespace with a message instead of a version. The CLI sends it to the
owners as a suggestion; they decide whether to publish it, credited to the
user:

```
agent-plugins publish <path> --namespace <owner-namespace> --message "<what changed and why>"
```

Owners answer suggestions with `agent-plugins review`, which lists them, then
`agent-plugins review <number> --accept` or `--decline "<note>"`.

## Who can see it

A package follows its space unless it is shared differently.
`agent-plugins share <ns>/<package>` shows who can see it; `--private` limits
it to the space and a share list, `--add <account>` or `--add team:<ns>` adds
to that list, and `--link` prints a link that gives anyone who opens it
access. Change visibility only when the user asks.

## Rules

- Publish only under the user's namespace, or a team that `whoami` lists
  when the user chooses it. Anything else is a suggestion to its owners.
- Never include `.env` files, private keys, tokens, or personal data.
- Do not edit the CLI's output or claim a publish succeeded without the
  `published` line.
