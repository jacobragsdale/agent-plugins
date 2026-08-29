# Publish to the marketplace

This tutorial publishes a skill you already have on your machine to the company marketplace. It takes a minute. Authentication is your Windows logon; there is nothing to sign in to.

## From an agent

Install the official `publish` skill from the marketplace (search for `official/publish` in Agent Plugins, or run `skill-manager install official/publish`). Then tell your agent:

> Publish my `review` skill to the marketplace.

The skill validates the package, proposes a better description and tags, runs a dry run, and publishes with your confirmation. The rest of this page is what it does by hand.

## From a terminal

`skill-manager` is the Agent Plugins executable. It is on PATH after the app's first run; otherwise use `"%LOCALAPPDATA%\Programs\Agent Plugins\skill-manager.exe"`.

1. Check who you are:

   ```
   skill-manager whoami
   ```

   The `namespace` line is where your packages publish: your lowercase account name.

2. Point at the skill directory (it contains `SKILL.md`):

   ```
   skill-manager publish %USERPROFILE%\.claude\skills\review --version 1.0.0 --tags review,git --changelog "First release."
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

3. The response names the package:

   ```
   published jacob/review 1.0.0
     https://marketplace.example.com/api/packages/jacob/review
   ```

   Every client sees it at its next sync; the card shows your name, the version, and how many people use it.

## What you can publish

| Input                                   | Result                                                                                           |
| --------------------------------------- | ------------------------------------------------------------------------------------------------ |
| A directory with `SKILL.md`             | One `skill` package. The package ID is the frontmatter `name` (a `yourname-` prefix is dropped). |
| An MCP document in the `mcp.json` shape | One `mcpServer` package named after the file.                                                    |
| A tree with `skill-manager.json`        | The single package it declares; `source.id` must equal your namespace.                           |

`--package-id` overrides the package ID. `--namespace official` publishes to the official lane when your account is allowlisted.

## Versions

Versions are immutable semantic versions. Publish a new version to change anything; the namespace archive always carries the latest non-yanked version of each package, so clients update on their next sync. Yank a version with the API when it must disappear from the catalog (installed copies are unaffected).

## Rules the CLI enforces

- The archive is at most 50 MB and contains no symbolic links.
- No `.env`, key, certificate, or token-looking content.
- `SKILL.md` has a non-empty `name` and `description`; the name equals the package ID.
- The package validates: the server runs the same validator and rejects a mismatch.

See [the API reference](marketplace-api.md) for the request the CLI makes and [ADR 0004](decisions/0004-internal-marketplace.md) for why publishing works this way.
