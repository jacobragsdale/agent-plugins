# Publish to the marketplace

This tutorial publishes a skill to the company marketplace. It takes a minute, and the skill is live as soon as it is published; colleagues see it within about 15 minutes, or at once through its page's **Install in Agent Plugins** button. Authentication is your Windows logon; there is nothing to sign in to.

## From scratch, in the web portal

No skill yet? Open the marketplace in your browser, choose **Share a skill**, then **Write it here**. Give it a **Name**, say **When should the assistant use it?** in a sentence or two, and write the **Instructions**. **Create a skill** in the app opens this page when it finds no AI app it can open.

Choose where it goes under **Publish to**, and **Who can install it**:

- **Only me, and people I share it with** — the default for your own space. Use it to try the skill yourself first.
- **Everyone** — anyone at the company can find and install it.
- **Same as the space** — follows the space's setting, which each space label shows.

To fix a typo later, choose **Edit** on the skill's page; it opens the editor filled in from the published `SKILL.md` and publishes the next version.

## From scratch, in your AI app

With an AI app such as GitHub Copilot, Cursor, or Claude installed, **Create a skill** in Agent Plugins asks which app to use, then opens it with a message; the window says what to do next, such as pressing Enter to send it. The agent asks what the skill should do, writes it where that app reads skills so you can try it, and publishes it once you say it's ready. "Just for me" publishes it privately.

## From files, in the web portal

Choose **Share a skill**, then drag in what you already have: a skill folder, a folder of skill folders (a skill pack), a `SKILL.md`, a zip, or an MCP server's `.json`. Skills Agent Plugins installed for you live in `%USERPROFILE%\.agents\skills`, and Claude Code's own in `%USERPROFILE%\.claude\skills`. **Publish** stays off until the upload holds a usable `SKILL.md`. To change a published skill, choose **Upload a new version** on its page: the portal compares your files with the live version first and names any that the new version would drop. The rest of this page is the command-line route.

## From an agent

Install the official `publish` skill from the marketplace (search for `official/publish` in Agent Plugins, or run `agent-plugins install official/publish`). Then tell your agent:

> Publish my `review` skill to the marketplace.

The skill validates the package, proposes a better description and tags, runs a dry run, and publishes with your confirmation. Ask for it "just for me" and it publishes privately. The rest of this page is what it does by hand.

## From a terminal

`agent-plugins` is the Agent Plugins executable. The app adds its folder to your user PATH when it starts; until then, run it as `"%LOCALAPPDATA%\Agent Plugins\agent-plugins.com"`.

1. Check who you are:

   ```
   agent-plugins whoami
   ```

   The `namespace` line is where your packages publish: your lowercase account name, or that name with a number on the end (`christopher-jo-2`) when another account claimed it first. `publishes to` adds every team you are in; publish to one with `--namespace <team>`.

2. Check the skill directory (it contains `SKILL.md`):

   ```
   agent-plugins validate %USERPROFILE%\.claude\skills\review
   ```

   It stages the package the way `publish` does, runs every check that needs no network, and lists which apps can use each part. Fix anything it reports.

3. Publish it privately first, with a dry run to see what the server would do:

   ```
   agent-plugins publish %USERPROFILE%\.claude\skills\review --version 1.0.0 --private --tags review,git --changelog "First release." --dry-run
   ```

   Then run the same command without `--dry-run`. The CLI prints a summary:

   ```
   publish jacob/review 1.0.0
     as        CORP\jacob
     contents  3 file(s), 12 KB (4 KB zipped)
     who       only you, the space's owners, and people it is shared with
     tags      review, git
     changelog First release.
   Publish? [y/N]
   ```

   Answer `y`. Add `--yes` to skip the prompt.

4. The response names the package and its page in the portal:

   ```
   published jacob/review 1.0.0
     https://marketplace.example.com/p/jacob/review
   ```

   The version is live at once: everyone who can see it finds it at their app's next check, within about 15 minutes, and PCs that already have the package update to it then. A package with an MCP server updates on its own only when the server itself is unchanged; otherwise each person approves the change. When you're happy with it, share it (next section) or choose **Everyone**.

   A package with an MCP server that everyone can see waits for an admin once, the first time it is shared with everyone: the CLI then prints `published jacob/review 1.0.0; everyone else sees it once an admin approves its MCP server`. You, your team, and people you share it with can use it straight away. The admin's decision, and any note, is on the package's page in the portal.

## Choose who can see it

A new package published without a choice follows its space: your personal space is public, and a team's space is whatever its owners chose when they created it. The portal asks **Who can install it**, and `--private` or `--visibility` sets it on the command line. To change one package later, choose **Share** on its page in the portal, or **Share…** on its card in the app:

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

| Input                                   | Result                                                                                                                                  |
| --------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| A directory with `SKILL.md`             | One `skill` package. The package ID is the frontmatter `name` (a `yourname-` prefix is dropped, on the command line and in the portal). |
| A directory of skill directories        | A skill pack: one package, one `skill` component per subfolder, named after the directory.                                              |
| An MCP document in the `mcp.json` shape | One `mcpServer` package named after the file.                                                                                           |
| A tree with `agent-plugins.json`        | The package it declares, or the one `--package-id` names; `source.id` becomes your namespace. Only the declared paths are published.    |

`--package-id` overrides the package ID. `--namespace official` publishes to the official lane when your account is allowlisted.

## Versions

Versions are immutable release versions, `major.minor.patch`; a pre-release such as `1.0.0-beta.1` is refused. Publish a new version to change anything; the namespace archive always carries the highest version that is not withdrawn, so clients update on their next sync. A version lower than the live one is refused, with the next free version named. Publishing to a package that was removed from every PC is refused until it is restored.

Withdraw a version from its page in the portal, or with `agent-plugins withdraw <namespace>/<package> <version>`, when it must disappear from the catalog. Withdrawing the live version makes the one below it live, and PCs move back to it at their next check; a package whose MCP server differs asks each person first. `--undo`, or **Restore** on the page, brings a withdrawn version back. A version number that was ever used, including one you withdrew, cannot be reused.

A publish that timed out can be run again: when the version already holds the same files, it answers that nothing changed. To take a package off every PC that has it, choose **Remove from every PC** on its page, or run `agent-plugins revoke <namespace>/<package>`; each app uninstalls it at its next check.

## Rules the CLI enforces

- The package is at most 2,000 files, 50 MB unzipped, and 50 MB zipped, and contains no symbolic links. The server also refuses a publish that would push the whole namespace past those limits.
- No credential files, private keys, token-shaped strings, or fixed secrets in an MCP document; see [the credential scan](manifest-reference.md#credential-scan).
- `SKILL.md` has a non-empty `name` and `description`. The name is rewritten to the package ID.
- The package validates: the server runs the same validator and credential scan and rejects a mismatch.

The server also refuses more than 10 tags, a tag that is not up to 32 lowercase letters, digits, and single hyphens, and a changelog over 4,096 characters.

See [the API reference](marketplace-api.md) for the request the CLI makes, and [ADR 0004](decisions/0004-internal-marketplace.md) and [ADR 0007](decisions/0007-self-service-marketplace.md) for why publishing works this way.
