# CLI reference

`agent-plugins` is the Agent Plugins executable. When its first argument names a command it runs headless — no window — so a person or an agent can drive it from a terminal. Every command shares the app's validator, locator, identity, and installer, so a CLI publish or install is the same operation the window performs, authenticated the same way ([ADR 0004](decisions/0004-internal-marketplace.md)).

The app puts its folder on your user `PATH` when it starts, and uninstalling takes it off again (the uninstaller runs `agent-plugins remove-from-path`). Otherwise call it by path:

```text
"%LOCALAPPDATA%\Agent Plugins\agent-plugins.com"
```

On Windows the installer also puts `agent-plugins.com` beside it: a small console program that runs the command through the app. Shells pick `.com` before `.exe` for a bare `agent-plugins`, and they wait for a console program and read its output, which they do not do for the app itself. Call `agent-plugins` without an extension from PowerShell, `cmd`, or a script. A first argument that is not a command, an option, or an `agent-plugins://` link prints the usage and exits with status 2 instead of opening the window.

## Synopsis

```text
agent-plugins whoami
agent-plugins validate <path>
agent-plugins search [query]
agent-plugins publish <path> --version <major.minor.patch> [--namespace <ns>] [--package-id <id>]
                             [--tags a,b] [--changelog <text>] [--message <text>] [--yes]
agent-plugins install <ns>/<package> | <ns>/<package>/<skill> | <ns>/<bundle> | <link> [--approve-mcp]
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

Exit status is `0` on success and `1` on failure; failures print `error: <message>` on stderr. A first argument that starts with `-` and is not `--help` or `-h`, such as `--background`, starts the desktop app instead, and so does an `agent-plugins://` link (see [Links](#links)). `--help` or `-h` anywhere after a command prints the usage and does nothing else. `-y` is short for `--yes`.

## `whoami`

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
```

| Line                  | Meaning                                                                                                                                               |
| --------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `host account`        | Who the process runs as, according to Windows.                                                                                                        |
| `identity`            | The scheme used for marketplace requests: `Windows authentication (<domain>)`, or `development header as <account>`.                                  |
| `marketplace account` | Who the server says you are.                                                                                                                          |
| `namespace`           | Your personal publish namespace: your lowercase account name, or a numbered variant such as `christopher-jo-2` when another account claimed it first. |
| `publishes to`        | Every namespace you may publish to: yours, `official` when you are allowlisted, and your teams.                                                       |
| `teams`               | The teams you belong to; `(owner)` marks the ones you manage.                                                                                         |
| `groups`              | The AD groups the server resolved for you. They only matter for share lists that name a group.                                                        |
| `admin`               | Whether the server grants administrative rights.                                                                                                      |
| `suggestions waiting` | Suggested changes to your packages that wait for your answer. See [`review`](#review).                                                                |

## `validate <path>`

Validates a source tree or a published HTTPS archive against the same rules the marketplace server applies: source containment, component names, MCP shape, portability, symlinks, and repository limits. Writes nothing.

`<path>` is a directory containing `agent-plugins.json`, or an HTTPS URL of a zip, tar, or tar.gz of that tree.

```text
acme: 3 valid install(s), 0 catalog error(s)
```

Errors print one per line as `<path>: <message>`. The command fails when there is any error, or when no install is valid.

## `search [query]`

Lists marketplace packages, filtered by a case-insensitive substring match against the package ID, name, description, tags, and publisher display name. With no query, lists everything.

```text
package                          version    publisher          installs  users  tags
jacob/review                     1.2.0      Jacob Ragsdale           47     31  review,git
    Reviews a change before it is submitted.
```

`installs` counts install events; `users` is the current installed base. Matching bundles follow in their own table, with the number of packages each holds.

## `publish <path> --version <major.minor.patch>`

Stages `<path>` into a one-package source tree, refuses anything that looks like a credential, validates it, and uploads it to your namespace. Every version goes live as soon as it is published; there is no review queue ([ADR 0007](decisions/0007-self-service-marketplace.md)).

| Argument                        | Required | Meaning                                                                                                                                                                                                   |
| ------------------------------- | -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<path>`                        | yes      | A skill directory containing `SKILL.md`, a folder of skill directories (a skill pack), an MCP document in the `mcp.json` shape, or a source tree with `agent-plugins.json` declaring exactly one package. |
| `--version <major.minor.patch>` | yes      | A release version; the server refuses a pre-release such as `1.0.0-beta.1`. Immutable once published. Not used for a suggestion.                                                                          |
| `--namespace <ns>`              | no       | Publish somewhere other than your own namespace: one of your teams, or `official` when you are allowlisted.                                                                                               |
| `--package-id <id>`             | no       | Override the derived package ID.                                                                                                                                                                          |
| `--tags a,b`                    | no       | Comma-separated. Blank entries are dropped. At most 10, each up to 32 lowercase letters, digits, and single hyphens.                                                                                      |
| `--changelog <text>`            | no       | One line recorded against this version. At most 4,096 characters.                                                                                                                                         |
| `--message <text>`              | no       | For a suggestion: what you changed and why. At most 4,096 characters.                                                                                                                                     |
| `--yes`, `-y`                   | no       | Skip the confirmation prompt.                                                                                                                                                                             |

The package ID comes from the input: a skill directory uses the `SKILL.md` frontmatter `name` with a `<yourname>-` prefix stripped; a skill pack uses the folder name, and each subfolder becomes one skill component named by its `SKILL.md`; an MCP document uses the file name; a source tree uses the declared package ID, and its `source.id` must equal the namespace.

It prints a summary and asks before uploading:

```text
publish jacob/review 1.0.0
  as        CORP\jacob
  contents  3 file(s), 12 KB (4 KB zipped)
  tags      review, git
  changelog First release.
Publish? [y/N]
```

On success, with the package's page in the web portal:

```text
published jacob/review 1.1.0
  https://marketplace.example.com/p/jacob/review
```

A package with an MCP server that everyone can see needs an admin's approval once before people outside its space see it. You and your team have it at once:

```text
published data-team/warehouse 1.0.0; everyone else sees it once an admin approves its MCP server
  https://marketplace.example.com/p/data-team/warehouse
```

### Suggesting a change to someone else's package

When `--namespace` names a space you don't publish to and the package already exists there and you can see it, `publish` offers to send your version to its owners as a suggestion. `--message` (or, failing that, `--changelog`) says what you changed; `--version` is not needed, because the owner picks the version when they accept.

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
| No marketplace is configured in this build         | `No marketplace is configured.`                              |

A server rejection prints `HTTP <status>: <title>`, then the problem's `detail` and one line per field error, each indented. See [Publish to the marketplace](publish-to-marketplace.md) for the walkthrough and [the API reference](marketplace-api.md) for the request itself.

## `install <target>`

Syncs, then installs onto every detected agent, exactly as the window would.

| `<target>`                  | Installs                                                                                                                        |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `<ns>/<package>`            | One package. `agent-plugins search` prints the ID.                                                                              |
| `<ns>/<package>/<skill>`    | One skill or connector from a package with several.                                                                             |
| `<ns>/<bundle>`             | Every package in the bundle that is not installed yet, each in its own transaction. One that fails leaves the others installed. |
| A share link (`…/l/<code>`) | Adds you to what the link shares, then installs it: a package, a bundle, or every package in a shared space.                    |

| Option          | Meaning                                                                                                                                                                                                                                          |
| --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `--approve-mcp` | Grants the Tier 3 approval. Required whenever something being installed contains an MCP server; without it the install fails with `<names> includes a connector that runs a program on this computer. Run again with --approve-mcp to allow it.` |

```text
installed jacob/review (Review workflow)
  backed up C:\Users\jacob\.agents\.agent-plugins-backups\...
```

Backup lines appear only when an existing destination had to be preserved. An unknown ID fails with ``<id> is not in the catalog. Try `agent-plugins search`.`` A team invite link is refused with a pointer to `team join`.

There is no `uninstall` command; uninstall from the app.

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

The CLI prepares the host the same way the app does before it installs: it locates `uv`/`uvx`, normalizes proxy variables, and sets `UV_NATIVE_TLS`. Requests to the marketplace origin carry a Kerberos `Negotiate` token on a domain-joined host, or the `X-Dev-User` header where the server accepts it. Usage events are flushed as the process exits and never affect the exit status.

| Variable                   | Effect                                                                                                                                    |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `AGENT_PLUGINS_DEV_USER`   | Sends `X-Dev-User` with this account instead of Kerberos, even on a domain-joined host.                                                   |
| `AGENT_PLUGINS_DEV_GROUPS` | Comma-separated groups sent as `X-Dev-Groups` alongside the development header, so share lists that name a group can be tried without AD. |

Only a server that accepts `X-Dev-User` honors either variable: always in Development, otherwise only with `Auth:AllowDevHeader`. Any other server treats the request as unauthenticated.

## See also

- [Publish to the marketplace](publish-to-marketplace.md) — the publishing tutorial.
- [App reference](app-reference.md) — the same operations in the window.
- [Marketplace API](marketplace-api.md) — the endpoints these commands call.
