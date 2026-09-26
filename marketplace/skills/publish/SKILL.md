---
name: publish
description: Publish a skill or MCP server configuration to the company marketplace with the Agent Plugins CLI. Use when the user wants to share, publish, release, or upload a skill they wrote, bump a published version, or asks how to put something on the marketplace.
---

# Publish to the marketplace

Agent Plugins ships a command line, `agent-plugins`, that publishes a package
under the user's own namespace. Authentication is the user's Windows logon, so
there is nothing to log in to. You do the judgment work (description, tags,
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
2. **Confirm identity.** Run `agent-plugins whoami` and tell the user the
   namespace the package will publish under.
3. **Validate.** Run `agent-plugins validate <path>` when the path holds an
   `agent-plugins.json`; otherwise skip to the dry run in step 6, which
   validates the staged package.
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
6. **Dry run.** Run the publish command without `--yes`; it prints the
   package, file count, size, tags, and changelog, then waits for confirmation.
   Answer `n`, show the summary to the user, and ask them to confirm.
7. **Publish.** Run with `--yes`:

   ```
   agent-plugins publish <path> --version <semver> --tags <a,b,c> --changelog "<text>" --yes
   ```

   Report the printed marketplace URL. If the CLI refuses because of a secret
   or a validation error, show the message and fix the cause with the user;
   never work around the secret scan.

## Rules

- Publish only under the user's namespace unless they say otherwise and
  `whoami` lists the other namespace.
- Never include `.env` files, private keys, tokens, or personal data.
- Do not edit the CLI's output or claim a publish succeeded without the
  `published` line.
