# CLI reference

`skill-manager` is the Agent Plugins executable. When its first argument names a command it runs headless — no window — so a person or an agent can drive it from a terminal. Every command shares the app's validator, locator, identity, and installer, so a CLI publish or install is the same operation the window performs, authenticated the same way ([ADR 0004](decisions/0004-internal-marketplace.md)).

The app puts itself on `PATH` on first run. Otherwise call it by path:

```text
"%LOCALAPPDATA%\Programs\Agent Plugins\skill-manager.exe"
```

## Synopsis

```text
skill-manager whoami
skill-manager validate <path>
skill-manager search [query]
skill-manager publish <path> --version <semver> [--namespace <ns>] [--package-id <id>]
                             [--tags a,b] [--changelog <text>] [--yes]
skill-manager install <namespace>/<package> [--approve-mcp]
skill-manager help
```

Exit status is `0` on success and `1` on failure; failures print `error: <message>` on stderr. Any argument that is not one of `validate`, `publish`, `search`, `install`, `whoami`, `help`, or `--help` starts the desktop app instead.

## `whoami`

Prints the identity this machine publishes and installs under. Requires a reachable marketplace; it calls `GET /api/me`.

```text
host account: CORP\jacob
identity: Kerberos (Negotiate)
marketplace account: CORP\jacob
namespace: jacob
publishes to: jacob, official
admin: false
```

| Line                  | Meaning                                                                |
| --------------------- | ---------------------------------------------------------------------- |
| `host account`        | Who the process runs as, according to Windows.                         |
| `identity`            | The scheme used for marketplace requests: Kerberos, or the dev header. |
| `marketplace account` | Who the server says you are.                                           |
| `namespace`           | Your default publish namespace: your lowercase account name.           |
| `publishes to`        | Every namespace you may publish to, including allowlisted lanes.       |
| `admin`               | Whether the server grants administrative rights.                       |

## `validate <path>`

Validates a source tree or a published HTTPS archive against the same rules the marketplace server applies: source containment, component names, MCP shape, portability, symlinks, and repository limits. Writes nothing.

`<path>` is a directory containing `skill-manager.json`, or an HTTPS URL of a zip, tar, or tar.gz of that tree.

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

`installs` counts install events; `users` is the current installed base.

## `publish <path> --version <semver>`

Stages `<path>` into a one-package source tree, refuses anything that looks like a credential, validates it, and uploads it to your namespace.

| Argument             | Required | Meaning                                                                                                                                                     |
| -------------------- | -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<path>`             | yes      | A skill directory containing `SKILL.md`, an MCP document in the `mcp.json` shape, or a source tree with `skill-manager.json` declaring exactly one package. |
| `--version <semver>` | yes      | `major.minor.patch`. Immutable once published.                                                                                                              |
| `--namespace <ns>`   | no       | Publish somewhere other than your own namespace. You must be allowlisted for it, or an admin.                                                               |
| `--package-id <id>`  | no       | Override the derived package ID.                                                                                                                            |
| `--tags a,b`         | no       | Comma-separated. Blank entries are dropped.                                                                                                                 |
| `--changelog <text>` | no       | One line recorded against this version.                                                                                                                     |
| `--yes`, `-y`        | no       | Skip the confirmation prompt.                                                                                                                               |

The package ID comes from the input: a skill directory uses the `SKILL.md` frontmatter `name` with a `<yourname>-` prefix stripped; an MCP document uses the file name; a source tree uses the declared package ID, and its `source.id` must equal the namespace.

It prints a summary and asks before uploading:

```text
publish jacob/review 1.0.0
  as        CORP\jacob
  contents  3 file(s), 12 KB (4 KB zipped)
  tags      review, git
  changelog First release.
Publish? [y/N]
```

On success:

```text
published jacob/review 1.0.0
  https://marketplace.example.com/api/packages/jacob/review
```

Refusals, before anything is uploaded:

| Condition                                  | Message                                                     |
| ------------------------------------------ | ----------------------------------------------------------- |
| The namespace is not yours                 | `<account> may publish to <list> but not to <namespace>.`   |
| A staged file looks like a secret          | Each finding on stderr as `secret: <path>`, then a refusal. |
| Validation failed                          | Each error on stderr, then `The package failed validation.` |
| The zipped archive exceeds 50 MB           | `The package archive is larger than the 50 MB limit.`       |
| No marketplace is configured in this build | `No marketplace is configured.`                             |

A server rejection prints `HTTP <status>: <title>` plus one indented line per field error. See [Publish to the marketplace](publish-to-marketplace.md) for the walkthrough and [the API reference](marketplace-api.md) for the request itself.

## `install <namespace>/<package>`

Syncs, then installs one catalog package onto every detected agent, exactly as the window would.

| Argument        | Meaning                                                                                                      |
| --------------- | ------------------------------------------------------------------------------------------------------------ |
| `<ns>/<pkg>`    | The catalog ID. `skill-manager search` prints it.                                                            |
| `--approve-mcp` | Grants the Tier 3 approval. Required for any package containing an MCP server; without it the install fails. |

```text
installed jacob/review (Review workflow)
  backed up C:\Users\jacob\.agents\.skill-manager-backups\...
```

Backup lines appear only when an existing destination had to be preserved. An unknown ID fails with `<id> is not in the catalog. Try 'skill-manager search'.`

There is no `uninstall` command; uninstall from the app.

## Environment

The CLI prepares the host the same way the app does before it installs: it locates `uv`/`uvx`, normalizes proxy variables, and sets `UV_NATIVE_TLS`. Requests to the marketplace origin carry a Kerberos `Negotiate` token on a domain-joined host, or the `X-Dev-User` header where the server accepts it. Usage events are flushed as the process exits and never affect the exit status.

## See also

- [Publish to the marketplace](publish-to-marketplace.md) — the publishing tutorial.
- [App reference](app-reference.md) — the same operations in the window.
- [Marketplace API](marketplace-api.md) — the endpoints these commands call.
