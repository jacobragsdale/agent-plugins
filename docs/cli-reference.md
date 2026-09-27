# CLI reference

`agent-plugins` is the Agent Plugins executable. When its first argument names a command it runs headless — no window — so a person or an agent can drive it from a terminal. Every command shares the app's validator, locator, identity, and installer, so a CLI publish or install is the same operation the window performs, authenticated the same way ([ADR 0004](decisions/0004-internal-marketplace.md)).

The app puts its folder on your user `PATH` when it starts, and uninstalling takes it off again (the uninstaller runs `agent-plugins remove-from-path`). Otherwise call it by path:

```text
"%LOCALAPPDATA%\Agent Plugins\agent-plugins.com"
```

On Windows the installer also puts `agent-plugins.com` beside it: a small console program that runs the command through the app. Shells pick `.com` before `.exe` for a bare `agent-plugins`, and they wait for a console program and read its output, which they do not do for the app itself. Call `agent-plugins` without an extension from PowerShell, `cmd`, or a script. A first argument that is not a command, an option, or an `agent-plugins://` link prints the usage and exits with status 2 instead of opening the window. Stopping a command (Ctrl+Break, or closing the terminal) also stops the app it runs; a change it was making is undone the next time Agent Plugins starts.

One change runs at a time across the window and every command. A command that has to wait for another's change prints `Waiting for another Agent Plugins window or command to finish a change...` after 2 seconds, then carries on.

## Synopsis

```text
agent-plugins whoami [--json]
agent-plugins validate <path> | <https archive url> [--namespace <ns>] [--package-id <id>]
agent-plugins search [words...] [--json]
agent-plugins publish <path> --version <major.minor.patch> [--namespace <ns>] [--package-id <id>]
                             [--private | --visibility <inherit|public|private>] [--tags a,b]
                             [--changelog <text> | --changelog-file <path>] [--message <text>] [--dry-run] [--yes]
agent-plugins install <ns>/<package> | <ns>/<package>/<skill> | <ns>/<bundle> | <link> [--approve-mcp] [--replace]
agent-plugins install --local <path> [--approve-mcp]
agent-plugins list [--json]
agent-plugins status <ns>/<package> [--json]
agent-plugins uninstall <ns>/<package>[/<component>] [--force]
agent-plugins sync
agent-plugins withdraw <ns>/<package> <version> [--undo]
agent-plugins hold <ns>/<package> [--undo]
agent-plugins keep <ns>/<package>
agent-plugins share <ns>[/<id>] [--public | --private | --inherit] [--add <entry>]... [--remove <entry>]...
agent-plugins share <ns>[/<id>] --link [--reset]
agent-plugins team [<ns>]
agent-plugins team create <ns> --name <display name> [--public]
agent-plugins team add <ns> <account> [--owner]
agent-plugins team remove <ns> <account>
agent-plugins team leave <ns>
agent-plugins team rename <ns> --name <display name>
agent-plugins team invite <ns> [--reset]
agent-plugins team join <link>
agent-plugins team delete <ns> [--yes]
agent-plugins review [<number> --accept [--version <v>] | <number> --decline <note>]
agent-plugins revoke <ns>/<package> [--undo] [--yes]
agent-plugins bundle <ns>/<id>
agent-plugins bundle set <ns>/<id> --name <name> [--description <text>] <ns>/<package>...
agent-plugins bundle delete <ns>/<id> [--yes]
agent-plugins help
```

Failures print `error: <message>` on stderr. The exit status says what kind of failure it was:

| Status | Meaning                                                                                  |
| ------ | ---------------------------------------------------------------------------------------- |
| `0`    | Done.                                                                                    |
| `1`    | Any other failure.                                                                       |
| `2`    | Usage: an unknown command or option, a missing argument, or an option without its value. |
| `3`    | An MCP server needs `--approve-mcp`.                                                     |
| `4`    | Not found: an unknown package, bundle, or source, or one that is not installed.          |
| `5`    | The marketplace or another server could not be reached.                                  |

A first argument that starts with `-` and is not `--help` or `-h`, such as `--background`, starts the desktop app instead, and so does an `agent-plugins://` link (see [Links](#links)). `--help` or `-h` anywhere after a command prints the usage and does nothing else. `-y` is short for `--yes`.

## `whoami [--json]`

Prints the identity this machine publishes and installs under. Requires a reachable marketplace; it calls `GET /api/me`.

```text
host account: CORP\jacob
identity: Windows authentication (CORP)
marketplace account: CORP\jacob
namespace: jacob
publishes to: jacob, official, data-team
teams: data-team (owner)
groups: Data Engineering
admin: false
suggestions waiting: 2
reports waiting: 1
unread notifications: 3
```

| Line                   | Meaning                                                                                                                                               |
| ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `host account`         | Who the process runs as, according to Windows.                                                                                                        |
| `identity`             | The scheme used for marketplace requests: `Windows authentication (<domain>)`, or `development header as <account>`.                                  |
| `marketplace account`  | Who the server says you are.                                                                                                                          |
| `namespace`            | Your personal publish namespace: your lowercase account name, or a numbered variant such as `christopher-jo-2` when another account claimed it first. |
| `publishes to`         | Every namespace you may publish to: yours, `official` when you are allowlisted, and your teams.                                                       |
| `teams`                | The teams you belong to; `(owner)` marks the ones you manage.                                                                                         |
| `groups`               | The AD groups the server resolved for you. They only matter for share lists that name a group.                                                        |
| `admin`                | Whether the server grants administrative rights.                                                                                                      |
| `suggestions waiting`  | Suggested changes to your packages that wait for your answer. See [`review`](#review).                                                                |
| `reports waiting`      | Open reports and feedback on your packages. Answer them in the portal.                                                                                |
| `unread notifications` | Marketplace notifications you have not read in the portal.                                                                                            |

`--json` prints one object with `hostAccount`, `identity`, `account`, `namespace`, `namespaces`, `teams` (each `namespace`, `displayName`, `owner`), `groups`, `admin`, `suggestionsWaiting`, `reportsWaiting`, and `unreadNotifications`.

## `validate <path>`

Checks what [`publish`](#publish-path---version-majorminorpatch) would send, without the network, and writes nothing. `<path>` takes every input `publish` does. The command stages it, then runs the source validation (containment, component names, MCP shape, portability, symlinks), the credential scan, and the desktop app's size limits: 2,000 files, 50 MB unzipped, 50 MB zipped. Then it lists, for each skill and MCP server, whether each app can use it.

| Option              | Meaning                                                                                            |
| ------------------- | -------------------------------------------------------------------------------------------------- |
| `--namespace <ns>`  | The namespace to stage under. Defaults to the manifest's `source.id`, else your Windows user name. |
| `--package-id <id>` | Check one package. Without it, a source tree with several packages checks each one.                |

```text
jacob/review: 3 file(s), 12 KB (4 KB zipped)
  skill review
    GitHub Copilot     yes
    Cursor             yes
    Claude Code        yes
    Claude Desktop     no: Claude Desktop takes skills only from your claude.ai account (Customize > Skills), not from this computer.
    OpenCode           yes
    pi                 yes
    Codex              yes
    ChatGPT (Codex)    yes
    Grok Build         yes
```

A problem adds `, not publishable` to the first line and one line each: `error: <path>: <message>`, `secret: <path>: <reason>`, or `too big: <limit>`. The app verdicts come from planning against an empty home folder, so they do not depend on what is installed on this computer. The command fails when any package is not publishable.

Given an `https://` address instead of a path, `validate` downloads that source archive and checks it the way the app reads it: the source ID, the number of valid packages, and each catalog error. The local-path checks above do not apply to it.

## `search [words...] [--json]`

Lists marketplace packages whose ID, name, description, tags, or publisher display name contain every word you give, in any order, ignoring case. With no words, lists everything.

```text
package                          version    publisher          installs  users  tags
jacob/review                     1.2.0      Jacob Ragsdale           47     31  review,git
    Reviews a change before it is submitted.
```

`installs` counts the people who installed it; `users` is the current installed base. Matching bundles follow in their own table, with the number of packages each holds. `No packages match.` when nothing does.

`--json` prints `{ "packages": [...], "bundles": [...] }` in the shapes of the [index](marketplace-api.md#get-apiindex).

## `publish <path> --version <major.minor.patch>`

Stages `<path>` into a one-package source tree, refuses anything that looks like a credential, validates it, and uploads it to your namespace. Every version goes live as soon as it is published; there is no review queue ([ADR 0007](decisions/0007-self-service-marketplace.md)). Staging copies only what the package declares and leaves out tool leftovers such as `.git`, `node_modules`, `.venv`, and `__pycache__`; see [the manifest reference](manifest-reference.md#what-publishing-leaves-out).

| Argument                        | Required | Meaning                                                                                                                                                                                                               |
| ------------------------------- | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<path>`                        | yes      | A skill directory containing `SKILL.md` (`skill.md` and `SKILL.md.txt` count too), a folder of skill directories (a skill pack), an MCP document in the `mcp.json` shape, or a source tree with `agent-plugins.json`. |
| `--version <major.minor.patch>` | yes      | A release version; the server refuses a pre-release such as `1.0.0-beta.1`. Immutable once published. Not used for a suggestion.                                                                                      |
| `--namespace <ns>`              | no       | Publish somewhere other than your own namespace: one of your teams, or `official` when you are allowlisted.                                                                                                           |
| `--package-id <id>`             | no       | Override the derived package ID. For a source tree with several packages, choose the one to publish.                                                                                                                  |
| `--private`                     | no       | Only you, the space's owners, and people it is shared with can see it. The way to try a first version yourself.                                                                                                       |
| `--visibility <v>`              | no       | `inherit` (the same as its space), `public`, or `private`. Set in the same transaction as the publish. Without it, a new package follows its space and an existing one keeps its setting.                             |
| `--tags a,b`                    | no       | Comma-separated. Blank entries are dropped. At most 10, each up to 32 lowercase letters, digits, and single hyphens.                                                                                                  |
| `--changelog <text>`            | no       | Recorded against this version. At most 4,096 characters.                                                                                                                                                              |
| `--changelog-file <path>`       | no       | Reads the changelog from a file instead.                                                                                                                                                                              |
| `--dry-run`                     | no       | Runs every check, on this computer and on the server, and publishes nothing. Needs no confirmation.                                                                                                                   |
| `--message <text>`              | no       | For a suggestion: what you changed and why. At most 4,096 characters.                                                                                                                                                 |
| `--yes`, `-y`                   | no       | Skip the confirmation prompt.                                                                                                                                                                                         |

The package ID comes from the input: a skill directory uses the `SKILL.md` frontmatter `name` with a `<yourname>-` prefix stripped; a skill pack uses the folder name, and each subfolder becomes one skill component named by its `SKILL.md`; an MCP document uses the file name; a source tree uses the declared package ID, or the one `--package-id` chooses. A source tree's `source.id` is replaced by the namespace you publish to.

It prints a summary and asks before uploading. `who` appears when you chose a visibility:

```text
publish jacob/review 1.0.0
  as        CORP\jacob
  contents  3 file(s), 12 KB (4 KB zipped)
  who       only you, the space's owners, and people it is shared with
  tags      review, git
  changelog First release.
Publish? [y/N]
```

`--dry-run` prints each file that is new, changed, or removed against the live version, a count of unchanged ones, and any warnings:

```text
  changed  skills/review/SKILL.md
  new      skills/review/checklist.md
  1 file(s) unchanged
dry run: the checks passed and nothing was published
```

On success, with the package's page in the web portal:

```text
published jacob/review 1.1.0
  https://marketplace.example.com/p/jacob/review
```

Any warning from the server follows as `warning: <text>`, for example that a changed MCP server waits for an admin again. Running the same publish again after a timeout is safe: when the version already holds the same files, it prints `<id> <version> was already published with these files; nothing changed`.

A package with an MCP server that everyone can see needs an admin's approval before people outside its space see it. You and your team have it at once:

```text
published data-team/warehouse 1.0.0; everyone else sees it once an admin approves its MCP server
  https://marketplace.example.com/p/data-team/warehouse
```

### Suggesting a change to someone else's package

When `--namespace` names a space you don't publish to and the package already exists there and you can see it, `publish` offers to send your version to its owners as a suggestion. `--message` (or, failing that, `--changelog`) says what you changed; `--version` is not needed, because the owner picks the version when they accept. `--dry-run` runs the local checks and sends nothing.

```text
suggest a change to data-team/review
  as        CORP\jane
  contents  3 file(s), 12 KB (4 KB zipped)
  message   Tightened step 3.
You don't own data-team/review. Send this to its owners as a suggestion? [y/N] y
suggested a change to data-team/review (#12); its owners decide whether to publish it
  https://marketplace.example.com/p/data-team/review
```

The server repeats the credential scan, so a file that slips past this one is still refused.

Refusals, before anything is uploaded:

| Condition                                          | Message                                                      |
| -------------------------------------------------- | ------------------------------------------------------------ |
| The namespace is not yours and has no such package | `<account> may publish to <list> but not to <namespace>.`    |
| A suggestion has no `--message` or `--changelog`   | `A suggestion needs --message "<what you changed and why>".` |
| A staged file looks like a secret                  | Each finding on stderr as `secret: <path>`, then a refusal.  |
| Validation failed                                  | Each error on stderr, then `The package failed validation.`  |
| The zipped archive exceeds 50 MB                   | `The package archive is larger than the 50 MB limit.`        |
| `--private` and `--visibility` together            | `Choose --private or --visibility, not both.`                |
| No marketplace is configured in this build         | `No marketplace is configured.`                              |

A server rejection prints `HTTP <status>: <title>`, then the problem's `detail` and one line per field error, each indented. A version lower than the live one, or one already used, adds `Publish <version> or higher.` with the next free version. A package removed from every PC must be restored before a new version. See [Publish to the marketplace](publish-to-marketplace.md) for the walkthrough and [the API reference](marketplace-api.md) for the request itself.

## `install <target>`

Syncs, then installs onto every detected agent, as the window would. The sync first prints what it changed in other packages: `updated <id> (<name>) <from> -> <to>`, `removed <name>: its publisher or an admin pulled it`, `repaired <name>: put back files that were missing`, `added <name> to newly found apps`, and `could not update <id>: <reason>` on stderr.

| `<target>`                  | Installs                                                                                                                        |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `<ns>/<package>`            | One package. `agent-plugins search` prints the ID.                                                                              |
| `<ns>/<package>/<skill>`    | One skill or connector from a package with several.                                                                             |
| `<ns>/<bundle>`             | Every package in the bundle that is not installed yet, each in its own transaction. One that fails leaves the others installed. |
| A share link (`…/l/<code>`) | Adds you to what the link shares, then installs it: a package, a bundle, or every package in a shared space.                    |

| Option           | Meaning                                                                                                                                                                                                                                                                                         |
| ---------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--approve-mcp`  | Grants the Tier 3 approval. Required whenever something being installed contains an MCP server; without it the install fails with `<names> includes a connector that runs a program on this computer. Run again with --approve-mcp to allow it.`                                                |
| `--replace`      | Backs up files or settings entries that Agent Plugins did not install and that are in the way, then installs. Without it such a package fails with `Files or settings that Agent Plugins didn't install are in the way of <target>. Run again with --replace to back them up and replace them.` |
| `--local <path>` | Installs a folder from this computer instead, for testing without publishing; see below.                                                                                                                                                                                                        |

```text
installed jacob/review (Review workflow)
  backed up C:\Users\jacob\.agents\.agent-plugins-backups\...
```

Backup lines appear only when an existing destination had to be preserved, and `warning: <text>` names an app that was skipped because its settings file could not be read. `note: <text>` names an app skipped only because it had its settings file open; the next sync adds the package there, from the app or the CLI. A package already installed prints `<target> is already installed.` and succeeds, including when another install of it finished first. An unknown ID fails with ``<id> is not in the catalog. Try `agent-plugins search`.`` A team invite link is refused with a pointer to `team join`.

### `install --local <path>`

Installs `<path>`, any input [`publish`](#publish-path---version-majorminorpatch) takes, onto every detected agent as the package `local/<id>`, so its skills land in `local-<name>` folders and never take a published package's place. Installing the same folder again updates that install. Background syncs leave it alone. Files deleted by hand are not put back, and the window lists it as no longer offered.

```text
installed local/review (Review) from C:\Users\jacob\src\review
  remove it with: agent-plugins uninstall local/review
```

## `list [--json]`

Lists the packages installed on this computer from the app's saved state, without the network:

```text
package                          status               version    name
jacob/review                     installed            1.2.0      Review
data-team/warehouse              updateAvailable (held) -        Warehouse
```

`status` is the [package state](app-reference.md#package-states); `(held)` marks a package whose updates you hold. `version` is the marketplace version when the installed copy is it. With no saved state it fails with ``Agent Plugins has no saved state yet. Run `agent-plugins sync`.``

`--json` prints an array of `{ id, name, status, installedVersion, latestVersion, held }`. `installedVersion` is null unless the package is installed, changed, or partly installed at the marketplace's current version.

## `status <ns>/<package> [--json]`

Shows a package as the marketplace sees it, and on this computer:

```text
Review (jacob/review)
  live version  1.2.0
  visible to    public
  installs      47 (31 using it in the last 30 days)
  this computer installed
  1.2.0        2026-09-20
  1.1.0        2026-09-02  withdrawn
```

`removed from every PC by an admin` (or `by its owners`) appears for a revoked package, and `MCP review <state>` with any `review note` for a package with an MCP server.

`--json` prints `{ id, name, liveVersion, visibility, revoked, revokedByAdmin, review, reviewNote, installs, installedBase, versions, local }`. Each version is `{ version, publishedAt, publishedBy, changelog, withdrawn, purged }`. `local` is `{ status, installedVersion, held }`, or null when the package is not installed here. When the marketplace answers not found (the package was deleted, or you lost access) but it is installed here, `status` still shows this computer's copy, with `marketplace   not found` in place of the marketplace lines (and those fields null in `--json`), and exits 0.

## `uninstall <ns>/<package>[/<component>]`

Removes a package, or one skill or connector of it, from every agent. A package you changed on this computer fails with `… contains local changes … Run again with --force to remove it anyway; your copy is saved to backups first.`

| Option    | Meaning                                                                                      |
| --------- | -------------------------------------------------------------------------------------------- |
| `--force` | Removes a changed package too, after saving your copy to `~/.agents/.agent-plugins-backups`. |

## `sync`

Runs a full sync, as **Refresh** in the window does, and prints what it changed in the lines [`install`](#install-target) uses, or `Everything is up to date.` When servers answered but nothing usable came back (a Wi-Fi sign-in page, or errors), it prints `Some sources couldn't be refreshed, so their saved copies are in use. Agent Plugins tries again soon.` instead. It fails when any package could not be updated.

## `withdraw <ns>/<package> <version>`

Withdraws a version: it leaves the catalog, and PCs that have it move to the newest version left at their next check. MCP packages whose server differs wait for approval. `--undo` restores it. Owners and admins only. The version number stays used either way.

## `hold <ns>/<package>`

Stops background updates of an installed package on this computer. `agent-plugins install <ns>/<package>` still updates it when you choose. `--undo` lets it update on its own again. The window shows it as **Updates held**.

## `keep <ns>/<package>`

Stops managing an installed package and leaves its files as they are, for example to keep your own edited copy. Agent Plugins no longer updates, restores, or removes it; you can install it again later.

## `share <ns>[/<id>]`

Shows or changes who may see and install a space, a package, or a bundle. You must own the space: your own, one of your teams, or any space as an admin. On a team's space itself, only the team's owners may change it.

| Option                              | Meaning                                                                                                                                                                       |
| ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--public`                          | Everyone who signs in can see it.                                                                                                                                             |
| `--private`                         | Only the space's owners and the share list can see it.                                                                                                                        |
| `--inherit`                         | A package or bundle follows its space again.                                                                                                                                  |
| `--add <entry>`, `--remove <entry>` | Repeatable. `<entry>` is an account (`CORP\jane`, `jane@corp.example`, or `jane`), `team:<ns>`, or `group:<AD group>`. Removing an entry that is not on the list is an error. |
| `--link`                            | Prints the share link, creating it the first time. Anyone at the company who opens it is added to the list.                                                                   |
| `--reset`                           | With `--link`: replaces the link, so the old one stops working. People it already added stay on the list.                                                                     |

With no options it prints the current setting. Otherwise it changes only what you name and prints the result:

```text
share data-team/review
  visibility  same as its space (private)
  people      Alice Chen (CORP\alice)
  teams       Platform Team (team:platform)
  link        none; --link makes one
```

The share list is kept when general access changes. Anything a person may no longer see leaves their catalog, `search`, and the app; what they installed from it shows **No longer offered** until they uninstall it.

## `team`

Teams are spaces anyone can create. Members publish, withdraw, share, and edit bundles there; owners also manage members and the invite link, rename the team, and delete it while it is empty.

| Command                                  | Effect                                                                                                                                     |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `team`                                   | Lists your teams with your role, their visibility, and their size.                                                                         |
| `team <ns>`                              | Shows the team's members and invite link.                                                                                                  |
| `team create <ns> --name <display name>` | Creates a team you own. `<ns>` is 2–16 lowercase letters, digits, and single hyphens. Its skills are private to members unless `--public`. |
| `team add <ns> <account> [--owner]`      | Adds someone, or changes whether they are an owner. Owners only.                                                                           |
| `team remove <ns> <account>`             | Removes someone. Owners only. The last owner can't be removed.                                                                             |
| `team leave <ns>`                        | Leaves the team. The last owner can't leave.                                                                                               |
| `team rename <ns> --name <display name>` | Changes the name people see. The namespace never changes.                                                                                  |
| `team invite <ns> [--reset]`             | Prints the invite link, creating it the first time. `--reset` (owners only) replaces it.                                                   |
| `team join <link>`                       | Joins the team a link invites you to.                                                                                                      |
| `team delete <ns> [--yes]`               | Deletes a team with no packages or bundles. Owners only.                                                                                   |

## `review`

Answers suggested changes to your packages.

| Command                                    | Effect                                                                                         |
| ------------------------------------------ | ---------------------------------------------------------------------------------------------- |
| `review`                                   | Lists the suggestions waiting for you, noting any made against an older version.               |
| `review <number> --accept [--version <v>]` | Publishes the suggestion, credited to whoever made it. The version defaults to the next patch. |
| `review <number> --decline <note>`         | Declines it. The note is required; the person who made the suggestion sees it.                 |

## `revoke <ns>/<package>`

Pulls a package from every PC: it leaves the catalog, and every copy of the app uninstalls it at its next check, backing up any copy someone edited. Asks first unless `--yes`. Owners and admins only.

`--undo` offers the package again. PCs that removed it do not reinstall it on their own.

## `bundle`

A bundle is a named list of packages, from any space, that install together; each package can still be installed on its own.

| Command                                                                       | Effect                                                                                                                               |
| ----------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| `bundle <ns>/<id>`                                                            | Shows the bundle: its packages, and how many you can't see.                                                                          |
| `bundle set <ns>/<id> --name <name> [--description <text>] <ns>/<package>...` | Creates the bundle, or replaces its name, description, and packages. 1 to 50 packages, each one you can see. You must own the space. |
| `bundle delete <ns>/<id> [--yes]`                                             | Deletes the bundle. Its packages stay installed wherever they are.                                                                   |

A bundle ID can't be the same as a package ID in its space.

## Links

The marketplace portal's **Install** buttons open `agent-plugins://install/<ns>/<id>` (a package or bundle), `agent-plugins://install/<ns>/<package>/<skill>`, or `agent-plugins://open/<ns>/<id>`. The app registers itself for these links in `HKCU\Software\Classes\agent-plugins` when it starts, and `remove-from-path`, which the uninstaller runs, removes that registration when it still points at this copy.

Given such a link as its argument, the executable opens the window, or hands the link to the one already running, instead of running a command. The window always asks before it installs anything; a link carries IDs only and never skips the MCP approval. Anything that is not exactly one of these forms is ignored.

## Environment

The CLI prepares the host the same way the app does before it installs: it locates `uv`/`uvx`, normalizes proxy variables, and sets `UV_NATIVE_TLS` (`UV_SYSTEM_CERTS` for a uv that takes it). Requests to the marketplace origin carry a Kerberos `Negotiate` token on a domain-joined host, or the `X-Dev-User` header where the server accepts it. Usage events are flushed as the process exits and never affect the exit status.

| Variable                   | Effect                                                                                                                                    |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `AGENT_PLUGINS_DEV_USER`   | Sends `X-Dev-User` with this account instead of Kerberos, even on a domain-joined host.                                                   |
| `AGENT_PLUGINS_DEV_GROUPS` | Comma-separated groups sent as `X-Dev-Groups` alongside the development header, so share lists that name a group can be tried without AD. |

Only a server that accepts `X-Dev-User` honors either variable: always in Development, otherwise only with `Auth:AllowDevHeader`. Any other server treats the request as unauthenticated.

## See also

- [Publish to the marketplace](publish-to-marketplace.md) — the publishing tutorial.
- [App reference](app-reference.md) — the same operations in the window.
- [Marketplace API](marketplace-api.md) — the endpoints these commands call.
