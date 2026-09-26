# Publish to the marketplace

This tutorial publishes a skill you already have on your machine to the company marketplace. It takes a minute. Authentication is your Windows logon; there is nothing to sign in to.

## From scratch, in Cursor

No skill yet? Choose **Create a skill** in Agent Plugins. Cursor opens and offers to create a chat with a prompt; choose **Create Chat**, then send it. The agent asks what the skill should do, writes it where Cursor reads skills so you can try it, and publishes it once you say it's ready.

## From the web portal

Open the marketplace in your browser and choose **Share a skill**. Pick who it is for (just you, or one of your teams), then drag in what you already have: a skill folder, a folder of skill folders (a skill pack), a `SKILL.md`, a zip, or an MCP server's `.json`. Skills you use in Claude Code live in `%USERPROFILE%\.claude\skills`. To change a published skill, edit it on your machine and choose **Upload a new version** on its page. The rest of this page is the command-line route.

## From an agent

Install the official `publish` skill from the marketplace (search for `official/publish` in Agent Plugins, or run `agent-plugins install official/publish`). Then tell your agent:

> Publish my `review` skill to the marketplace.

The skill validates the package, proposes a better description and tags, runs a dry run, and publishes with your confirmation. The rest of this page is what it does by hand.

## From a terminal

`agent-plugins` is the Agent Plugins executable. The app adds its folder to your user PATH when it starts; until then, run it as `"%LOCALAPPDATA%\Agent Plugins\agent-plugins.com"`.

1. Check who you are:

   ```
   agent-plugins whoami
   ```

   The `namespace` line is where your packages publish: your lowercase account name, or that name with a number on the end (`christopher-jo-2`) when another account claimed it first.

2. Point at the skill directory (it contains `SKILL.md`):

   ```
   agent-plugins publish %USERPROFILE%\.claude\skills\review --version 1.0.0 --tags review,git --changelog "First release."
   ```

   The CLI stages a one-package source tree, refuses files that look like credentials, validates it with the same rules the server applies, and prints a summary:

   ```
   publish jacob/review 1.0.0
     as        CORP\jacob
     contents  3 file(s), 12 KB (4 KB zipped)
     tags      review, git
     changelog First release.
   Publish? [y/N]
   ```

   Answer `y`. Add `--yes` to skip the prompt.

3. The response names the package and its page in the portal:

   ```
   submitted jacob/review 1.0.0 for review; it goes live when an admin approves it
     https://marketplace.example.com/p/jacob/review
   ```

   An admin reviews a package's first version. Once it is approved, every client sees it at its next sync; the card shows your name, the version, and how many people use it. Later versions go live immediately unless they contain an MCP server, which is reviewed every time. The status and any reviewer's note are on **My skills** in the portal.

## What you can publish

| Input                                   | Result                                                                                           |
| --------------------------------------- | ------------------------------------------------------------------------------------------------ |
| A directory with `SKILL.md`             | One `skill` package. The package ID is the frontmatter `name` (a `yourname-` prefix is dropped). |
| A directory of skill directories        | A skill pack: one package, one `skill` component per subfolder, named after the directory.       |
| An MCP document in the `mcp.json` shape | One `mcpServer` package named after the file.                                                    |
| A tree with `agent-plugins.json`        | The single package it declares; `source.id` must equal your namespace.                           |

`--package-id` overrides the package ID. `--namespace official` publishes to the official lane when your account is allowlisted.

## Versions

Versions are immutable release versions, `major.minor.patch`; a pre-release such as `1.0.0-beta.1` is refused. Publish a new version to change anything; the namespace archive always carries the latest non-yanked version of each package, so clients update on their next sync. Withdraw a version from its page in the portal, or yank it with the API, when it must disappear from the catalog (installed copies are unaffected). A withdrawn version can be restored the same way. A version number that was ever used, including one an admin sent back or one you withdrew, cannot be reused; the server names the next free one.

## Rules the CLI enforces

- The archive is at most 50 MB and contains no symbolic links.
- No `.env`, key, certificate, or token-looking content.
- `SKILL.md` has a non-empty `name` and `description`; the name equals the package ID.
- The package validates: the server runs the same validator and credential scan and rejects a mismatch.

The server also refuses more than 10 tags, a tag that is not up to 32 lowercase letters, digits, and single hyphens, and a changelog over 4,096 characters.

See [the API reference](marketplace-api.md) for the request the CLI makes and [ADR 0004](decisions/0004-internal-marketplace.md) for why publishing works this way.
