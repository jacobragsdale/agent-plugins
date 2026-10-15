# Make, publish, and improve skills: implementation plan

**Status:** Ready to implement<br>
**Date:** 2026-10-14<br>
**Written against:** upstream commit `31161ba` (version 0.2.2)

Making, publishing, and improving a skill takes too many steps. Skills that live in Git drift away from the marketplace, and agents can't send feedback. MCP servers can't ship their own code. Our `mcp.json` claims the Agent Plugins 1.0 schema but doesn't follow it.

This plan fixes that in four phases. Each phase is one release and leaves `main` working.

| Phase                                                          | What changes                                                                                                                                                         | Touches                                      |
| -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------- |
| [1. Quick wins](#3-phase-1-quick-wins)                         | No rate limiting; publish without picking a version; publish a repository from a PC; feedback and suggestions from agents; **Create a skill** runs a published skill | Server, CLI, window, official skills, docs   |
| [2. Agent Plugins 1.0](#4-phase-2-agent-plugins-10)            | Our MCP documents follow the spec; plugin folders publish as packages; MCP servers can ship their own code                                                           | Rust core, validator, server inspector, docs |
| [3. Provenance](#5-phase-3-provenance)                         | Each version records the repository, folder, and commit it came from; manual changes to a Git-published package warn                                                 | Server, CI templates, portal                 |
| [4. Connected repositories](#6-phase-4-connected-repositories) | A team connects a GitHub or Azure DevOps repository and the marketplace publishes from it; suggestions become pull requests                                          | Server, portal                               |

Phases 1 and 2 are independent. Phase 4 needs phase 3.

## 1. Rules for the agent

1. **Find code by symbol, not by line.** [Appendix A](#appendix-a-symbol-map) lists every symbol as it is upstream. If you work in a fork where a symbol is missing, find the code that does the same job and note the mapping before changing it.
2. **Behavior is the spec.** Where this plan shows JSON, endpoint shapes, or message text, keep them. Where it describes UI, build the same behavior with the components you have.
3. **Reuse before you write.** Every phase leans on code that already exists: `NextVersionAsync`, `RepositoryPublishService`, the reports and suggestions endpoints, `PathMaterialization::Copy`, and the launch code in `tutorial.rs`. Don't write a second version of any of them.
4. **No new dependencies** in Rust, .NET, or either frontend. The GitHub and Azure DevOps calls are plain `HttpClient` requests.
5. **Keep the clippy gate clean** (`AGENTS.md`): delete what a change makes dead, including tests of deleted functions.
6. **Update `WORK_PORT_PLAN.md` in the same change** as anything that adds configuration or a rollout step ([TODO.md](TODO.md) says why).
7. **ASK marks a question for Jacob.** [Section 9](#9-open-questions) gives a default for each one. Use the default and keep going.

## 2. Decisions already made

| Decision                                                                                                                                                    | Why                                                                                                                                                                                     |
| ----------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Remove rate limiting outright.                                                                                                                              | Jacob: it isn't needed. A reverse proxy can still limit; the client keeps its retry on 429.                                                                                             |
| Every document we store or serve follows Agent Plugins 1.0. What the spec doesn't cover lives under our own `extensions` namespace.                         | Jacob: don't break the JSON spec where it can be avoided. The only place it can't is versions already published, because versions are immutable ([4.7](#47-compatibility-and-release)). |
| The marketplace picks version numbers everywhere unless `--version` is given.                                                                               | CI and the portal already do this; only the CLI asks.                                                                                                                                   |
| A connected repository is the source of truth for its packages.                                                                                             | Two writable copies drift: today the next merge silently undoes a portal edit or an accepted suggestion.                                                                                |
| Native plugin installs (VS Code `chat.pluginLocations`, Claude Code and Copilot CLI marketplaces, Cursor `~/.cursor/plugins/local`) wait for a later round. | First see whether MCP servers that ship their own code still feel second-class. Phase 2 stores each MCP server as a valid plugin folder, so that round can register it as is.           |

## 3. Phase 1: Quick wins

### 3.1 Remove rate limiting

Delete:

- `Program.cs`: the `AddRateLimiter` block with its `RateLimitOptions` read, `app.UseRateLimiter()`, and `using System.Threading.RateLimiting`.
- `Configuration/MarketplaceOptions.cs`: `RateLimitOptions`.
- `appsettings.json`: the `RateLimits` section.
- Tests: the two `RateLimits:*` settings in `MarketplaceApiFactory`, and the test in `FleetAndFeedbackTests` that sets `RateLimits:UploadsPerHour` to 1 and expects `Too many changes`.
- Docs: the paragraph in `docs/marketplace-api.md` that starts "Writes are limited per account", the `429` row of its status table, the `RateLimits:*` configuration row, and the `HTTP 429` row in `docs/publish-from-ci.md`.

Keep `artifact::is_transient` treating 429 as retryable, because proxies send it. Keep the Artifact Keeper throttling comments in `ArtifactKeeperStore`: they're about Artifact Keeper's own limit. Check the deployment config (`server/compose.yaml`, the home-server compose, the work `deployments` repo) for `RateLimits__*` variables and remove any you find.

### 3.2 Publish without picking a version; publish a repository

**Server.** In `POST /api/packages/{ns}/{packageId}/versions`, `version` becomes optional. When it is missing, use `PublishService.NextVersionAsync`: `1.0.0` for a new package, otherwise the patch after the highest version ever used. That's the rule `RepositoryPublishService` already follows. A dry run returns the version it would use. The portal keeps sending its own version, so it doesn't change.

**CLI** (`cli.rs`):

- `publish` drops `--version is required`. Without the flag, the dry run shows `version 1.2.1 (next)` and the publish uses it. A suggestion still takes no version. `--tags` and `--changelog` are already optional.
- Add `--repository`: `agent-plugins publish <folder> --repository [--namespace <team>] [--changelog <text>] [--dry-run] [--yes]`. It publishes every package in the folder, each as its own package, exactly the way CI does:
  1. Zip the folder, leaving out `.git` and every name `digest::is_tool_leftover` skips, and refuse anything over `MAX_ARCHIVE_BYTES`.
  2. Post it to `POST /api/namespaces/{ns}/publish` with `dryRun=true` and `Accept: text/markdown, application/problem+json`. A markdown-only `Accept` gets an empty body on errors.
  3. Print the plan it returns, then ask `Publish? [y/N]` unless `--yes` is given.
  4. Post again without `dryRun`.

  Exit with the same codes as `publish`.

- Without `--repository`, nothing changes meaning. A folder of skill folders is still one skill pack. When `stage_tree` finds no package in a folder that has skills further down, its error ends with: `To publish every skill in it as its own package, add --repository.`

**`publish` skill** (`marketplace/skills/publish/SKILL.md`): delete the version step. Tags and changelog become one proposal shown with the dry run. Add a short section on `--repository` for a folder or repository of skills. The skill goes from eight steps to five: identify, namespace, validate, dry run, publish.

### 3.3 Feedback and suggestions from agents

The server already has everything. This phase adds the CLI commands and the guidance that tells agents when to use them.

**`agent-plugins feedback <target> "<text>" [--problem] [--yes]`**

- `<target>` is `<namespace>/<package>`, or the installed name of a skill or MCP server, such as `jacob-review`. Installed names resolve through the installed items in `AppState`.
- When the package is installed, the CLI adds a final line, `Installed version: 1.2.0`.
- It prints what it will send and to whom (`the owners of jacob`). It asks before sending unless `--yes` is given.
- It posts `{ kind, reason }` to `POST /api/packages/{ns}/{packageId}/reports`. `kind` is `problem` with `--problem`, otherwise `feedback`.
- Text longer than the server's `MaxReportReason` (2,048 characters) is refused before sending.

**`agent-plugins suggest <target> <folder> --message "<text>" [--dry-run] [--yes]`** is a thin front for the suggestion path `publish` already takes for a namespace you don't own. When you own the package it refuses with `You own <id>; publish it instead.` Phase 4 relaxes that rule for connected packages.

**`marketplace` skill** (`marketplace/skills/marketplace/SKILL.md`): add a section, "When a marketplace skill goes wrong". Marketplace skills are the ones whose names start with a namespace (`agent-plugins list --json` lists them).

- When one gave wrong or outdated instructions, failed, or the user is unhappy with its result, say so and offer to tell its owners. Never send anything without the user's yes.
- Draft the feedback: what the user asked, what the skill told you to do, and what happened. Leave out secrets, file contents, customer data, and project paths. Show the draft, then run `feedback`.
- If the fix is an edit to the skill, copy the installed skill folder to a temporary folder, make the edit there, show the user the diff, and run `suggest` with the message.

Widen the skill's frontmatter `description` so it triggers on these moments: "…or when a marketplace skill misbehaved and the user wants its owners told, or wants to suggest a fix."

### 3.4 Create a skill runs a published skill

**The skill.** Add `marketplace/skills/create-skill/SKILL.md` and publish it to `official`, where its installed name is `official-create-skill`. It replaces `CREATE_PROMPT` and the tutorial's `SKILL` text. It has to be faster than today's interview:

1. One opening question: what job the skill does, with one real example of the input and of a great result. Explain in one sentence what a skill is.
2. Write a first draft straight away from that answer. Don't interview through every topic first. Ask only about what the draft can't guess, one question at a time, in plain words.
3. Write it to the folder named in the message that opened the session. The folder and the frontmatter `name` are both `<namespace>-<name>`; the namespace comes from `whoami`, as today.
4. Suggest trying it. Revise until the user is happy.
5. Publish:
   - Run `agent-plugins publish "<folder>" [--namespace <team>] [--private] --dry-run`. It's private unless the user named a team.
   - Propose up to three tags and a one-line changelog along with the dry run.
   - Only after a yes, run it again with `--yes` in place of `--dry-run`.

   There is no version question. The skill finds the CLI the way the other official skills do (their "Find the command" section). Keep today's rules: no secrets, under 150 lines, never work around the secret scan.

**The app** (`tutorial.rs`):

- `create_skill(paths, app)` finds `skills/create-skill/SKILL.md` in the current cached snapshot of the `official` source, using `source::source_cache_root` and `read_current_pointer`. Sync already downloads every listed source, so no install is needed.
- It opens `app` with one line: `Read <path to that SKILL.md> and follow the instructions in it. Write the new skill in <skills_root(app)>.` It reuses `open`, `next_step`, and `skills_root` unchanged.
- When the snapshot has no `create-skill`, it fails with: `Create a skill isn't available yet: the marketplace has no official create-skill. You can write a skill in the web portal instead.` The window offers the portal button it already shows when no AI app is found.
- The tutorial banner runs the same command, then `dismiss_tutorial`.
- Delete `SKILL`, `CREATE_PROMPT`, `create_prompt`, `command_line`, `run`, `write_skill`, `tutorial_prompt`, and their tests. Also delete the `run_tutorial` command from `ipc.rs`, `lib.rs`, `application/mod.rs`, `src/ipc/schemas.ts`, and `App.tsx`.
- Keep `remove_skill` and `SKILL_NAME`, so **Reset** still removes the sample skill that earlier versions wrote.

**Rollout:** publish `official/create-skill` before the desktop release reaches PCs. In the work port this is a step: publish `marketplace/skills/create-skill` to `official` as an allowlisted account.

## 4. Phase 2: Agent Plugins 1.0

The spec is at https://agent-plugins.org/specification. Its schemas are `https://agent-plugins.org/schemas/1.0.0/plugin.schema.json` and `…/mcp.schema.json`. VS Code, Copilot CLI, Cursor, and Codex load plugins in this format natively.

### 4.1 What following the spec changes

| Rule                                                              | Today (`mcp.rs` `McpServer::validate`)     | Agent Plugins 1.0                                | New documents                                                                                                |
| ----------------------------------------------------------------- | ------------------------------------------ | ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------ |
| `command`                                                         | A bare name on `PATH`; refuses `./`        | A bare name, or a `./` path inside the plugin    | As the spec, with the path checked to stay inside the plugin folder                                          |
| `${PLUGIN_ROOT}`, `${PLUGIN_DATA}` in `args`, `env` values, `cwd` | Refused                                    | The client must fill them in                     | Filled in at install ([4.5](#45-install))                                                                    |
| `cwd` set                                                         | Anything except `./…` and the placeholders | Only `./…`, `${PLUGIN_ROOT}…`, `${PLUGIN_DATA}…` | As the spec                                                                                                  |
| `cwd` missing                                                     | The AI app's default                       | The plugin folder                                | The plugin folder                                                                                            |
| `${NAME}` anywhere else                                           | Each AI app fills it in                    | Plain text; clients must not expand it           | Not allowed in `mcp.json`; `env` and header values move to our extension ([4.2](#42-api-keys-our-extension)) |
| `Authorization` and other sensitive headers                       | Must be `${NAME}` or `Scheme ${NAME}`      | No expansion; no credentials in the package      | Not allowed in `mcp.json`; added through our extension                                                       |
| URL with a `#fragment`                                            | Allowed                                    | Refused                                          | Refused                                                                                                      |
| Empty `mcpServers`                                                | Refused                                    | Valid                                            | Still refused when publishing; there's nothing to install. This is publishing policy, not a document rule.   |

Add `McpConfig::from_spec_slice` with the spec's rules. The current `McpConfig::from_slice` becomes the legacy reader for file-form components ([4.3](#43-manifest-directory-form-mcp-components)) and stays as it is.

### 4.2 API keys: our extension

What the spec leaves to clients goes under one reverse-domain namespace in `plugin.json` `extensions`, as the spec allows. Put the namespace in one constant in `mcp.rs`, `MCP_EXTENSION = "io.github.jacobragsdale.agent-plugins"` (**ASK 1**). It is stored in every archive from now on, so don't change it after the first release.

```json
{
  "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
  "name": "tracker",
  "extensions": {
    "io.github.jacobragsdale.agent-plugins": { "mcpServers": { "database": { "env": { "API_KEY": "${API_KEY}" } }, "tracker": { "headers": { "Authorization": "Bearer ${TRACKER_TOKEN}" } } } }
  }
}
```

Rules:

- The object is closed: `mcpServers` only. Each key names a server in the plugin's `mcp.json`. A server takes `env` (stdio only) or `headers` (remote only).
- Values use today's `${NAME}` grammar (`mcp::environment_references`) and today's rules, so sensitive headers are exactly `${NAME}` or `Scheme ${NAME}`.
- A key may not also appear in the server's own `env` or `headers` in `mcp.json`: one place per key.
- When the catalog reads the package, it merges the extension into the server. The resulting `McpServer` is the same value today's document would have produced, so `environment_names`, **Connector settings**, the approval dialog, and every adapter in `adapters.rs` keep working unchanged.
- A client that doesn't know the namespace ignores it, as the spec requires. The server then starts without the key, and its authentication is up to that client, which is also what the spec says.

### 4.3 Manifest: directory-form MCP components

An `mcpServer` component's `path` in `agent-plugins.json` may name a **directory** holding `mcp.json`, `plugin.json`, and any files the server ships. That directory is the server's plugin folder. No new manifest fields, and `deny_unknown_fields` stays.

| `path` names | Rules                                                                                                    | Who writes it                                   |
| ------------ | -------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| A directory  | Agent Plugins 1.0 (`from_spec_slice`), the extension from its `plugin.json`, and plugin-folder semantics | Staging, for every publish from this release on |
| A file       | Today's rules (`from_slice`) and today's behavior                                                        | Only versions published before this release     |

`catalog.rs` (the `ManifestComponent::McpServer` branch) reads either form. For the directory form, the component is `source_is_directory: true`. Its digest covers the directory with `directory_digest` plus the merged server, so a change to bundled code is an update. Update `docs/manifest-reference.md` and regenerate the schemas (`generate-schema`) if the path description is in them.

### 4.4 Staging

`staging::stage_in` gains one input and changes how it writes MCP servers:

| Input                                                                          | Today                      | From this release                                                                                                                                                                                                                                                                                                                                                                           |
| ------------------------------------------------------------------------------ | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A **plugin folder**: `plugin.json` at the top with the Agent Plugins `$schema` | Refused as "not a skill"   | One package. The ID is `plugin.json` `name`, with a `<namespace>-` prefix dropped and `.` turned into `-`; refuse it, naming the rule, if it still isn't a valid ID. `--package-id` overrides it. `skills/*/SKILL.md` become skill components, staged like a skill pack. `mcp.json`, `plugin.json`, and every other file outside `skills/` go to one directory-form component, `mcp/<id>/`. |
| An MCP `.json` file                                                            | Copied as a file component | Converted to `mcp/<id>/` holding a spec `mcp.json` and a generated `plugin.json`                                                                                                                                                                                                                                                                                                            |
| `agent-plugins.json` with a file-form MCP component                            | Copied                     | Converted the same way, with the manifest path rewritten                                                                                                                                                                                                                                                                                                                                    |
| `agent-plugins.json` with a directory-form component                           | —                          | Copied and validated                                                                                                                                                                                                                                                                                                                                                                        |
| A skill or skill pack                                                          | Unchanged                  | Unchanged                                                                                                                                                                                                                                                                                                                                                                                   |

**Conversion**, one function: `staging::conformant_mcp(document) -> (spec document, extension)`.

- `env` values and header values that hold a `${NAME}` move into the extension. Literal values stay.
- A `${NAME}` anything else uses (`command`, `args`, `cwd`, `url`) is refused, naming the variable and the place, for example: `API_KEY is used in args. Agent Plugins 1.0 doesn't fill in variables there: pass it through env and have the server read it from its environment.`
- An absolute `cwd` is refused with the spec's rule.

Authors keep writing what they write today, and what's stored follows the spec.

**Generated `plugin.json`:** `$schema`, `name` (`<namespace>-<id>`), and `extensions` when there's anything to put there. Nothing else, so the bytes are deterministic and re-publishing the same input reports "unchanged". A `version` in an author's `plugin.json` is ignored, because the marketplace picks versions. Say so in `docs/manifest-reference.md`.

**Discovery** (`staging::discover`): a folder with an Agent Plugins `plugin.json` is one package, and discovery doesn't look inside it, just as with a skill folder. Duplicate names fail as they do today.

### 4.5 Install

**The plugin folder.**

- A directory-form component installs its folder once at `<app data>/plugins/<source-id>-<component-id>/`, named after the manifest component, so every server its `mcp.json` declares shares it.
- It is one `DesiredPath` with kind `Directory` and `PathMaterialization::Copy`. That materialization exists today and is only used by tests; this is its first real user, so remove its `cfg_attr(not(test), allow(dead_code))`.
- Every target that takes the server shares it, the way skills share `~/.agents/skills`. It goes when the server's last binding goes. Drift and backups work as for skill folders.

**The data folder.** `<app data>/plugin-data/<source-id>-<package-id>/`. The install and uninstall use cases in `application/items.rs` create it at install, keep it across updates as the spec requires, and remove it when the package is uninstalled. It isn't a ledger resource, because the server writes into it. Both folders live under app data, so `qa_paths.rs` already relocates them.

**Filling in placeholders.** A pure function in `planner.rs` runs before the adapters, after the extension merge:

- `${PLUGIN_ROOT}` and `${PLUGIN_DATA}` in `args`, `env` values, and `cwd` become absolute paths.
- A `./` `command` becomes the absolute path inside the plugin folder.
- A missing `cwd` becomes the plugin folder.
- `PLUGIN_ROOT` and `PLUGIN_DATA` are added to `env` last, so they win, as the spec's overlay order says.

The adapters then see plain absolute paths and need no change. Claude Desktop can take these servers, since they hold no references; extension keys are still unsupported there, as today.

**Approval.**

- `planner::needs_approval` counts a change to the plugin folder's digest as a change, so a new build of the bundled server asks each person again.
- In the approval dialog, a bundled command reads `Starts <command>, which comes with this package.`, followed by the full command line, as today.
- `docs/app-reference.md` gets the same sentence.

**Windows.** Executables under `%LOCALAPPDATA%` can be blocked by AppLocker or WDAC, and SmartScreen ignores files without Mark-of-the-Web. Recommend `node`, `uv`, or `python` running a bundled script over a bundled `.exe` in `docs/manifest-reference.md`.

### 4.6 Server

The validator, credential scan, and staging are the shared Rust binary, so portal uploads and repository publishes get plugin folders and conversion for free. The C# side only reads archives:

- `ArchiveInspector` reads directory-form components. The `EnvNames` and `HeaderNames` in `McpServerSummary` come from the merged server.
- Add `BundleDigest`: SHA-256 over the plugin folder's files. With it, `LaunchSpec` changes when bundled code changes, and a public package's MCP server goes back to an admin.
- Serialize `BundleDigest` with `JsonIgnoreCondition.WhenWritingNull`, so summaries stored for file-form versions produce byte-identical `LaunchSpec` strings and no approved package is sent for review again. Add a test that pins this.
- The portal upload picker already takes folders. Its help text names "a plugin folder (`plugin.json`)" next to the other inputs.

### 4.7 Compatibility and release

- Clients older than this release can't read directory-form components, and one unreadable package fails a whole namespace archive.
  1. Ship the desktop release first.
  2. Deploy the server and validator that write directory-form components once PCs have it.
  3. In that same deploy, set `Client:MinimumVersion` to this release.

  Add this to `WORK_PORT_PLAN.md`.

- Versions published before this release keep their file-form documents. They are immutable, so they're served as they are and keep today's behavior. This is the one place we can't follow the spec, and it ends with each package's next version.
- Republishing an existing MCP package converts it. Its server now runs in its own folder, which is the spec's default, so each person approves it once. `docs/manifest-reference.md` says: a server that needs a workspace takes it as an argument, or uses MCP roots.

## 5. Phase 3: Provenance

**Data.** Add three nullable columns to `PackageVersion`, in one EF migration:

- `SourceRepository`: the repository's web URL.
- `SourceCommit`: the full SHA.
- `SourcePath`: the package's folder in the repository; empty means the top.

**Where it comes from:**

- **CI.** `POST /api/namespaces/{ns}/publish` takes form fields `repository` and `commit`, accepted from machine identities only.
  - `ci/publish-skills/action.yml` sends `${{ github.server_url }}/${{ github.repository }}` and `${{ github.sha }}`.
  - `ci/publish-skills/azure-pipelines.yml` sends `$(Build.Repository.Uri)` and `$(Build.SourceVersion)`.
  - `RepositoryPublishService` records each package's discovery path as `SourcePath`.
  - A pipeline can only claim provenance for its own team's packages. `ponytail:` take GitHub's `repository` and `sha` from the OIDC claims if that ever matters.
- **Connected repositories** (phase 4) record what they fetched.
- **People publishing from a PC or the portal** record nothing.

**Shown.**

- The version views from `GET /api/packages/{ns}/{packageId}` gain `source: { repository, commit, path, url }`, where `url` links to the folder at that commit:
  - GitHub: `<repository>/tree/<commit>/<path>`
  - Azure DevOps: `<repository>?version=GC<commit>&path=/<path>`
- The portal's package page shows `From <repository>, folder <path>, commit <7 characters>`, linking to `url`.
- Regenerate `server/openapi.json`.

**Warnings.**

- A person publishing a new version of a package whose live version has a `SourceRepository` gets `This package is published from <repository>. The next change there replaces this version.` in `warnings`. The CLI prints it, and the portal shows it in the check before **Publish**.
- Accepting a suggestion on such a package shows the same sentence in the review.
- Phase 4 turns this into a refusal for connected packages; the warning stays for repositories that only use CI.

## 6. Phase 4: Connected repositories

### 6.1 What admins configure

```json
{
  "Git": {
    "PollMinutes": 5,
    "Hosts": [
      { "Kind": "github", "Url": "https://github.com", "ApiUrl": "https://api.github.com", "Token": "<secret>" },
      { "Kind": "azure-devops", "Url": "https://dev.azure.com/contoso", "Token": "<secret>" }
    ]
  }
}
```

- `Url` is the web prefix a repository URL starts with. GitHub Enterprise Server sets `ApiUrl` to `https://<host>/api/v3`. Azure DevOps Server uses its collection URL.
- The token needs read access to the repositories teams connect. For pull requests it also needs branch create and pull request create, on GitHub `contents: write` and `pull_requests: write`. Without write access, connections still publish; suggestions just stay in the marketplace.
- `ponytail:` one token per host. A GitHub App and Azure workload identity come when security asks for them (**ASK 2**).
- Without `Git:Hosts`, the feature is off and the portal hides it. List the settings in `docs/marketplace-api.md` and add them to `WORK_PORT_PLAN.md`'s facts table.

### 6.2 Connecting a repository

New table `RepositoryConnection`, in one migration:

| Column                        | Notes                               |
| ----------------------------- | ----------------------------------- |
| `Id`                          |                                     |
| `Namespace`                   | The team it publishes to            |
| `HostUrl`                     | The matching `Git:Hosts` entry      |
| `Repository`                  | `owner/repo`, or `org/project/repo` |
| `Branch`                      | Default `main`                      |
| `Path`                        | Folder; empty for the top           |
| `CreatedBy`, `CreatedAt`      |                                     |
| `LastCommit`, `LastCheckedAt` |                                     |
| `LastResultJson`              | The last `RepositoryPublish` report |

Endpoints (people only; machines get 403):

| Endpoint                                                           | Who                 | Does                                                                                                                                                       |
| ------------------------------------------------------------------ | ------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /api/namespaces/{ns}/repositories` `{ url, branch?, path? }` | Team owners         | Parses the URL, matches a host, checks access and the proof file, saves, and publishes once right away. Answers with the connection and that first report. |
| `GET /api/namespaces/{ns}/repositories`                            | Team members        | Connections with their last result                                                                                                                         |
| `POST /api/repositories/{id}/check`                                | Team owners         | Runs the check now                                                                                                                                         |
| `DELETE /api/repositories/{id}`                                    | Team owners, admins | Disconnects. Packages stay and become editable again.                                                                                                      |

URL forms:

- GitHub: `https://<host>/<owner>/<repo>[.git]`.
- Azure DevOps Services: `https://dev.azure.com/<org>/<project>/_git/<repo>`, or the same on `<org>.visualstudio.com`.
- Azure DevOps Server: `https://<host>/<collection>/<project>/_git/<repo>`.

**Proof file.** The connected folder holds `.agent-plugins-team`, whose first line is the team's namespace. Only someone who can push to the repository can add it, so a team can't connect a repository that isn't theirs, even though the server's token can read it. The check runs at connect and on every run. Deleting the file stops publishing, and the report says why.

**Portal.** The team page gets **Repositories**:

- **Connect a repository** asks for URL, branch, and folder. A refusal shows the server's message as it is; the messages name the fix, for example the proof file's exact name and contents.
- Each connection shows its last result (the same markdown CI shows), **Check now**, and **Disconnect**.

### 6.3 Checking and publishing

`RepositoryPoller`, a `BackgroundService` with a `PeriodicTimer` of `Git:PollMinutes`:

1. Each pass takes `pg_try_advisory_lock`, so only one replica runs it.
2. For each connection, it reads the branch's newest commit and its subject.
3. If the commit equals `LastCommit`, it only updates `LastCheckedAt`.
4. Otherwise it checks the proof file at that commit and downloads the folder as a zip.
5. It calls `RepositoryPublishService.PublishAsync`:
   - with a machine identity named `github:<owner>/<repo>` or `ado:<org>/<project>/<repo>`, so blocks, publisher labels, and the audit log work as they do for CI;
   - with the commit subject as the changelog, and the provenance from [phase 3](#5-phase-3-provenance).

   Add an overload that skips `CheckOwner`: the connection was authorized when an owner created it, and the proof file is checked on every run.

6. It stores the commit and the report. On failure it notifies the team's owners once per commit, linking to the team page.
7. Errors talking to the host are recorded and retried on the next pass.

`ponytail:` polling only. A webhook that triggers a check sooner comes when five minutes is too slow.

`RepositoryPublishService.PublishAsync` takes an optional `folder`, so a host zip of the whole repository publishes only `Path`.

**Host calls.** `IGitHost` with two implementations, `GitHubHost` and `AzureDevOpsHost`:

| Operation                 | GitHub                                                                                                           | Azure DevOps (`api-version=7.1`; check Server's)                                                                                     |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Newest commit and subject | `GET /repos/{o}/{r}/commits/{branch}`                                                                            | `GET …/_apis/git/repositories/{repo}/commits?searchCriteria.itemVersion.version={branch}&$top=1`                                     |
| Read one file             | `GET /repos/{o}/{r}/contents/{path}?ref={sha}`, raw media type                                                   | `GET …/items?path={path}&versionDescriptor.version={sha}&versionDescriptor.versionType=commit`                                       |
| Folder as zip             | `GET /repos/{o}/{r}/zipball/{sha}`; follow the redirect; the top folder is stripped by `ArchiveInspector.Survey` | `GET …/items?scopePath=/{path}&recursionLevel=full&$format=zip&versionDescriptor.version={sha}&versionDescriptor.versionType=commit` |
| Pull request              | Blobs, a tree on `base_tree`, a commit, `POST /git/refs`, then `POST /pulls`                                     | One `POST …/pushes` creating the branch with the commit, then `POST …/pullrequests`                                                  |

### 6.4 Connected packages are read-only; suggestions become pull requests

A package is **connected** when its live version's `SourceRepository` and `SourcePath` belong to an active connection in its namespace.

- **Publishing by hand is refused, for everyone.** `POST …/versions` from a person answers 409: `<id> is published from <repository>. Propose a change instead: it opens a pull request there.` The portal replaces **Edit** and **Upload a new version** with **Propose a change**, and `agent-plugins publish` prints the same message. **ASK 3** asks whether admins get an override; by default they disconnect first. Sharing, withdrawing, removing from every PC, and deleting are unchanged: they control distribution, not content.
- **Suggestions open pull requests.**
  - `SuggestionService.CreateAsync` allows owners to suggest on a connected package. Today it answers 409 "You own this package".
  - After the suggestion is stored, and if the host can write, the server opens a pull request:
    - Branch `agent-plugins/suggestion-<id>`, from the commit the live version came from.
    - The commit holds only the files that differ from the live version. Those are the files `SuggestionService` already compares.
    - Titled `<package>: <first line of the message>`, with a body that names the person (display name and account) and links to the suggestion.
  - The suggestion gets a `PullRequestUrl` column (in phase 4's migration) and the state `sent`, and the owners' notification links to the pull request. Merging it publishes it at the next check. `ponytail:` the marketplace doesn't follow the pull request's state.
  - If opening it fails, or the host has no write access, the suggestion stays a normal pending suggestion with a note, and today's review applies.
- **Mapping files back to the repository:**
  - A skill package: `skills/<id>/X` → `<SourcePath>/X`.
  - A plugin folder: `skills/<skill>/X` → `<SourcePath>/skills/<skill>/X`, and `mcp/<id>/X` → `<SourcePath>/X`. The generated `plugin.json` is never written back.
  - A repository with `agent-plugins.json`: component paths are the same in the repository.
  - `SKILL.md` keeps the repository's own `name:` line, read at the base commit.
  - Anything that doesn't map falls back to a normal suggestion.
- **Feedback** on a connected package stays in the marketplace. `ponytail:` filing it as an issue or work item comes later.
- **CI and a connection on the same repository** publish the same thing, and the second answers "unchanged". The docs recommend keeping CI for pull request checks only.

### 6.5 Security

- The server's token reads only what admins grant it. The proof file stops one team from publishing another team's repository.
- Pull requests only ever push branches under `agent-plugins/`, never the target branch, and need a human merge.
- New audit events: `repository.connected`, `repository.disconnected`. Every publish is already audited.
- Tokens come from the secret store, like every other server secret.

## 7. Documentation

| Doc                                              | Change                                                                                                                                                                                                                        |
| ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `README.md`                                      | Publishing paths: one command, `--repository`, connected repositories; plugin folders; the MCP guardrail sentence about code that comes with a package                                                                        |
| `docs/publish-to-marketplace.md`                 | Versions are optional; `--repository`; plugin folders; connected repositories; feedback and suggestions from agents                                                                                                           |
| `docs/publish-from-ci.md`                        | Point to connected repositories for teams that only need publishing; provenance fields; drop the 429 row                                                                                                                      |
| `docs/manifest-reference.md`                     | Plugin folders; the spec rules table; our extension; directory-form components; bundled servers; ignored `version`                                                                                                            |
| `docs/cli-reference.md`                          | `publish` without `--version`, `--repository`, `feedback`, `suggest`                                                                                                                                                          |
| `docs/marketplace-api.md`, `server/openapi.json` | Optional `version`, provenance fields, repository endpoints, `Git:*` settings; no rate limits                                                                                                                                 |
| `docs/app-reference.md`                          | Approval wording for bundled servers; **Create a skill** without a built-in prompt                                                                                                                                            |
| `docs/decisions/0009-agent-plugins-1-0.md`       | New ADR: conforming to Agent Plugins 1.0, the extension namespace, directory-form components, connected repositories as the source of truth. It amends the "native plugin trees" lines in ADR 0001 and `adapter-contract.md`. |
| `WORK_PORT_PLAN.md`                              | Publish `official/create-skill`; `Client:MinimumVersion` with phase 2; `Git:Hosts` facts and secrets; the ADO template's new form fields                                                                                      |
| `TODO.md`                                        | Remove "Provenance" from "Not planned" when phase 3 lands                                                                                                                                                                     |

## 8. Checks

Run the CI gate (`.github/workflows/ci.yml`) for every phase. Beyond it:

| Phase | Tests                                                                                                                                                                                                                                                                                 | By hand                                                                                                                                                                                                                                                                                                            |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1     | Server: no limiter; `versions` without `version` publishes `1.0.0`, then the next patch, and a dry run reports it. Rust: `parse_publish_args` without `--version`; `--repository` zips without `.git` and leftovers; feedback target resolution; the `tutorial.rs` tests that remain. | Windows VM: **Create a skill** in VS Code, Cursor, and Claude Code, from the button and the banner, through to a private publish. `publish --repository` and `feedback` against the live marketplace.                                                                                                              |
| 2     | `mcp.rs` table tests for every row of [4.1](#41-what-following-the-spec-changes); `conformant_mcp`; plugin-folder staging and discovery; extension merge equals today's parse; placeholder filling; the `LaunchSpec` stability test; directory-form reinstall on digest change        | Windows VM: a bundled `node ${PLUGIN_ROOT}/server.js` connector in Claude Code, VS Code, Cursor, Copilot CLI, and Codex. A key through the extension still works in Cursor, Codex, and Claude Code. A 0.2.x client is told to update.                                                                              |
| 3     | Migration; provenance recorded from both templates and from discovery paths; the warning on manual publish                                                                                                                                                                            | The GitHub template against the private CI test repository, on a temporary self-hosted runner as when CI publishing was first tested; the portal line and link                                                                                                                                                     |
| 4     | A fake `IGitHost` in the test project: connect, proof file present and missing, unchanged commit, changed commit, host error, refusal of manual publish, suggestion to pull request with path mapping, fallback without write                                                         | Connect `jacobragsdale/agent-plugins-ci-test` with a fine-grained token on the live server: a merge publishes within five minutes; a suggestion opens a pull request. Azure DevOps stays untested until someone runs it at work, so say so in `docs/publish-to-marketplace.md` as `publish-from-ci.md` does today. |

## 9. Open questions

| #   | Question                                                                                                                                                                                                   | Default                                                |
| --- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| 1   | The extension namespace stored in every package                                                                                                                                                            | `io.github.jacobragsdale.agent-plugins`                |
| 2   | Which Git hosts and credentials work allows: github.com or GitHub Enterprise Server, Azure DevOps Services or Server, tokens or a GitHub App and workload identity                                         | A token per host, github.com and Azure DevOps Services |
| 3   | Do admins get an override to publish by hand to a connected package?                                                                                                                                       | No: disconnect first                                   |
| 4   | Should every installed skill end with one line telling the agent how to send feedback, so it works without the `marketplace` skill installed? It changes every installed skill's bytes on its next update. | No; the rule lives in the `marketplace` skill          |
| 5   | Where it's built, and how commits land                                                                                                                                                                     | Upstream, one release per phase, committed to `main`   |

## 10. Left out, and when to add it

| Left out                                                                                 | Add when                                                           |
| ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------ |
| Native plugin installs in VS Code, Claude Code, Copilot CLI, and Cursor                  | The next round; phase 2's plugin folders are already valid plugins |
| An `agent-plugins mcp` server with search, install, publish, and feedback tools          | Agents keep tripping over finding the CLI                          |
| An MCP Registry (`/v0.1/servers`) endpoint for approved connectors                       | Work wants Copilot's registry-only MCP policy                      |
| Webhooks for connected repositories                                                      | Five-minute polling is too slow                                    |
| Following a suggestion's pull request to merged or closed                                | Owners ask why a suggestion still says "sent"                      |
| Feedback filed as an issue or work item                                                  | A team wants feedback in its tracker                               |
| Claude Code's `.claude-plugin/` format and repository `marketplace.json` files as inputs | Someone has a repository in that format                            |
| Provenance in the desktop window                                                         | People ask the app where a skill lives                             |
| Declared versions, pruning removed packages                                              | As in `TODO.md`                                                    |

## Appendix A: Symbol map

### Server (`server/src/Marketplace.Api`)

| Symbol                                                                                           | File                                   | Used in     |
| ------------------------------------------------------------------------------------------------ | -------------------------------------- | ----------- |
| `AddRateLimiter` block, `UseRateLimiter`                                                         | `Program.cs`                           | 3.1         |
| `RateLimitOptions`                                                                               | `Configuration/MarketplaceOptions.cs`  | 3.1         |
| `Publish` endpoint (`MapPost("/packages/{ns}/{packageId}/versions")`)                            | `Endpoints/MarketplaceEndpoints.cs`    | 3.2, 5, 6.4 |
| `PublishRepository` endpoint (`MapPost("/namespaces/{ns}/publish")`)                             | `Endpoints/MarketplaceEndpoints.cs`    | 3.2, 5      |
| `Report` endpoint, `MaxReportReason`                                                             | `Endpoints/MarketplaceEndpoints.cs`    | 3.3         |
| `PublishService.NextVersionAsync`, `CheckOwner`, `UnchangedVersionAsync`                         | `Packages/PublishService.cs`           | 3.2, 6.3    |
| `RepositoryPublishService.PublishAsync`                                                          | `Packages/RepositoryPublishService.cs` | 3.2, 5, 6.3 |
| `SuggestionService.CreateAsync`                                                                  | `Packages/SuggestionService.cs`        | 6.4         |
| `ArchiveInspector`, `ArchiveInspector.Survey`, `McpServerSummary`, `McpServerSummary.LaunchSpec` | `Packages/ArchiveInspector.cs`         | 4.6, 6.3    |
| `PackageVersion`, `Suggestion`                                                                   | `Data/Entities.cs`                     | 5, 6.4      |
| `MarketplaceIdentity.IsMachine`, `MachineAuthentication`                                         | `Auth/`                                | 6.3         |
| `NotificationService.Notify`, `OwnersAsync`                                                      | `Notifications/`                       | 6.3, 6.4    |
| `MarketplaceApiFactory`, `FleetAndFeedbackTests`                                                 | `tests/Marketplace.Api.Tests/`         | 3.1         |

### Rust (`src-tauri/src`)

| Symbol                                                                                                         | File                                     | Used in                          |
| -------------------------------------------------------------------------------------------------------------- | ---------------------------------------- | -------------------------------- |
| `publish`, `parse_publish_args`, `PublishArgs`, `MAX_ARCHIVE_BYTES`                                            | `cli.rs`                                 | 3.2, 3.3                         |
| `stage_tree`, `stage_in`, `discover`, `stage_pack`, `copy_publishable`, `write_manifest`, `zip_tree`           | `staging.rs`                             | 3.2, 4.4                         |
| `digest::is_tool_leftover`                                                                                     | `digest.rs`                              | 3.2                              |
| `McpConfig::from_slice`, `McpServer::validate`, `environment_references`, `environment_names`, `MCP_SCHEMA_V1` | `mcp.rs`                                 | 4.1, 4.2                         |
| `ManifestComponent::McpServer`, `SOURCE_MANIFEST_FILE`                                                         | `manifest.rs`                            | 4.3                              |
| `CatalogComponent`, the `ManifestComponent::McpServer` branch, `directory_digest`                              | `catalog.rs`                             | 4.3, 4.5                         |
| `DesiredPath`, `PathMaterialization::Copy`, `OwnedPathKind::Directory`                                         | `resource.rs`                            | 4.5                              |
| `json_server`, `codex_server`, `opencode_mcp_value`, `McpMapping::ClaudeDesktop`                               | `adapters.rs`                            | 4.5 (unchanged; read to confirm) |
| `needs_approval`                                                                                               | `planner.rs`                             | 4.5                              |
| `create_skill`, `run`, `dismiss`, `remove_skill`, `skills_root`, `open`, `next_step`, `SKILL`, `CREATE_PROMPT` | `tutorial.rs`                            | 3.4                              |
| `source_cache_root`, `read_current_pointer`                                                                    | `source.rs`                              | 3.4                              |
| `run_tutorial`, `create_skill`, `dismiss_tutorial` commands                                                    | `ipc.rs`, `lib.rs`, `application/mod.rs` | 3.4                              |
| `tutorial_offered`, `prompt_apps`                                                                              | `application/project.rs`                 | 3.4                              |

### Window, portal, CI, skills

| Symbol                                                  | File                                                             | Used in       |
| ------------------------------------------------------- | ---------------------------------------------------------------- | ------------- |
| `run_tutorial` / `create_skill` call, `tutorialOffered` | `src/App.tsx`, `src/ipc/schemas.ts`                              | 3.4           |
| `ApprovalDialog`                                        | `src/components/ApprovalDialog.tsx`                              | 4.5           |
| Package page, publish page, team page                   | `website/src/app/pages/package*.ts`, `publish*.ts`, `team.ts`    | 5, 6.2, 6.4   |
| Action inputs, template parameters                      | `ci/publish-skills/action.yml`, `azure-pipelines.yml`            | 5             |
| Official skills                                         | `marketplace/skills/{publish,marketplace,create-skill}/SKILL.md` | 3.2, 3.3, 3.4 |
