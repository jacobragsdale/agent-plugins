# Publish to the marketplace

This tutorial publishes a skill you already have on your machine to the company marketplace. It takes a minute, and the skill is live as soon as it is published. Authentication is your Windows logon; there is nothing to sign in to.

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

   The `namespace` line is where your packages publish: your lowercase account name, or that name with a number on the end (`christopher-jo-2`) when another account claimed it first. `publishes to` adds every team you are in; publish to one with `--namespace <team>`.

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
   published jacob/review 1.0.0
     https://marketplace.example.com/p/jacob/review
   ```

   The version is live at once: everyone who can see your space finds it at their app's next check, and PCs that already have the package update to it. The card shows your name, the version, and how many people use it.

   A package with an MCP server that everyone can see waits for an admin once, the first time it is shared with everyone: the CLI then prints `published jacob/review 1.0.0; everyone else sees it once an admin approves its MCP server`. You, your team, and people you share it with can use it straight away. The admin's decision, and any note, is on the package's page in the portal.

## Choose who can see it

A new package follows its space: your personal space is public, and a team's space is whatever its owners chose when they created it. To change one package, choose **Share** on its page in the portal, or **Share…** on its card in the app:

- **Same as its space**, **Private**, or **Everyone at the company**.
- Add people and teams to the list below. Private means the space's owners plus that list. Sharing lets people see and install it, never change it.
- **Copy link** makes a link to send. Anyone at the company who opens it gets the package and is added to the list. **Reset link** stops the old link working; people already on the list stay.

On the command line, `agent-plugins share jacob/review --private --add CORP\jane --add team:data-team` does the same, and `agent-plugins share jacob/review --link` prints the link.

## Improve someone else's skill

You can't publish to a space you don't own, but you can suggest a change. Choose **Suggest a change** on the package's page and upload your improved files with a note, or run `agent-plugins publish` on your copy: the CLI offers to send it as a suggestion. The owners see which files changed, then publish it (it becomes the next version, credited to you) or say why not. Their answer is on **My skills**.

## Pack or bundle?

| You want to                                                   | Publish                                                     |
| ------------------------------------------------------------- | ----------------------------------------------------------- |
| Ship several skills of your own that change together          | A skill pack: one folder of skill folders, one version.     |
| Group skills that already exist, yours or other people's      | A bundle: **New bundle** in the portal or the app.          |
| Let people install everything at once, or pick just one skill | Either: both offer **Install all** and one skill at a time. |

## What you can publish

| Input                                   | Result                                                                                           |
| --------------------------------------- | ------------------------------------------------------------------------------------------------ |
| A directory with `SKILL.md`             | One `skill` package. The package ID is the frontmatter `name` (a `yourname-` prefix is dropped). |
| A directory of skill directories        | A skill pack: one package, one `skill` component per subfolder, named after the directory.       |
| An MCP document in the `mcp.json` shape | One `mcpServer` package named after the file.                                                    |
| A tree with `agent-plugins.json`        | The single package it declares; `source.id` must equal your namespace.                           |

`--package-id` overrides the package ID. `--namespace official` publishes to the official lane when your account is allowlisted.

## Versions

Versions are immutable release versions, `major.minor.patch`; a pre-release such as `1.0.0-beta.1` is refused. Publish a new version to change anything; the namespace archive always carries the latest non-yanked version of each package, so clients update on their next sync. Withdraw a version from its page in the portal, or yank it with the API, when it must disappear from the catalog (installed copies are unaffected). A withdrawn version can be restored the same way. A version number that was ever used, including one you withdrew, cannot be reused; the server names the next free one. To take a package off every PC that has it, choose **Remove from every PC** on its page, or run `agent-plugins revoke <namespace>/<package>`; each app uninstalls it at its next check.

## Rules the CLI enforces

- The archive is at most 50 MB and contains no symbolic links.
- No `.env`, key, certificate, or token-looking content.
- `SKILL.md` has a non-empty `name` and `description`; the name equals the package ID.
- The package validates: the server runs the same validator and credential scan and rejects a mismatch.

The server also refuses more than 10 tags, a tag that is not up to 32 lowercase letters, digits, and single hyphens, and a changelog over 4,096 characters.

See [the API reference](marketplace-api.md) for the request the CLI makes, and [ADR 0004](decisions/0004-internal-marketplace.md) and [ADR 0007](decisions/0007-self-service-marketplace.md) for why publishing works this way.
