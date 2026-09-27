# Target adapter contract

This reference defines the acceptance bar for a built-in target adapter. Adapters are pure planners: they may inspect a validated component and profile, but they may not write files, start processes, change the ledger, download code, or suppress errors.

## Stable targets and pinned dialects

GitHub Copilot, Cursor, Claude Code, and Claude Desktop are the primary targets; the window lists them first. OpenCode and pi follow, then Codex, ChatGPT, and Grok Build.

| Target          | Stable ID        | User-scope dialect       | Skills                                | MCP                                                                               |
| --------------- | ---------------- | ------------------------ | ------------------------------------- | --------------------------------------------------------------------------------- |
| GitHub Copilot  | `github-copilot` | `github-copilot-2026-08` | `~/.agents/skills`                    | `~/.copilot/mcp-config.json`, plus each started VS Code edition's user `mcp.json` |
| Cursor          | `cursor`         | `cursor-2026-08`         | `~/.agents/skills`                    | `~/.cursor/mcp.json`                                                              |
| Claude Code     | `claude-code`    | `claude-code-2026-08`    | `~/.claude/skills`                    | `~/.claude.json`                                                                  |
| Claude Desktop  | `claude-desktop` | `claude-desktop-2026-09` | Unsupported: claude.ai account upload | `claude_desktop_config.json`, stdio only, no environment references               |
| OpenCode        | `opencode`       | `opencode-2026-08`       | `~/.agents/skills`                    | user `opencode.jsonc`                                                             |
| pi              | `pi`             | `pi-2026-09`             | `~/.agents/skills`                    | Unsupported: pi does not use MCP servers                                          |
| Codex           | `codex`          | `codex-2026-08`          | `~/.agents/skills`                    | `~/.codex/config.toml`                                                            |
| ChatGPT (Codex) | `chatgpt`        | `chatgpt-2026-09`        | `~/.agents/skills`                    | `~/.codex/config.toml`                                                            |
| Grok Build      | `grok-build`     | `grok-build-2026-08`     | `~/.agents/skills`                    | `~/.grok/config.toml`                                                             |

Microsoft 365 Copilot (`m365-copilot`) is no longer a target. A saved profile naming it is dropped, and a sync releases what a ledger recorded for it, removing those files ([ADR 0008](decisions/0008-primary-targets-and-connector-choices.md)).

A v2 package may contain several skill and MCP components. Agent Plugins does not install native `agent-plugin@1.0.0` package trees or always-on instruction files.

`github-copilot-2026-08` writes MCP servers to `~/.copilot/mcp-config.json`, which Copilot CLI and VS Code's agent host read, and to the user `mcp.json` (`servers` key, JSONC) of each VS Code edition whose user folder exists: `<config>/Code/User` and `<config>/Code - Insiders/User`, where `<config>` is `%APPDATA%` on Windows. VS Code's own chat does not read the Copilot CLI file unless its MCP discovery setting is on.

`opencode-2026-08` pins the stable `opencode.jsonc` contract with a root `mcp` object. The separate beta/v2 `mcp.servers` contract requires a new dialect and is not selected implicitly.

`claude-desktop-2026-09` covers the Chat and Cowork tabs. Both load skills from the claude.ai account, so the skill result is unsupported and says where skills are managed. The MCP document is the MSIX build's virtualized copy under `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\` when that package folder exists, otherwise `%APPDATA%\Claude\` (macOS: `~/Library/Application Support/Claude/`). The file accepts only stdio servers; remote transports are unsupported and point at Settings > Connectors. A server that reads any `${NAME}` is unsupported too, because Claude Desktop does not expand references.

`pi-2026-09` shares `~/.agents/skills`, which pi reads alongside `.agents/skills` in a project. pi has no MCP support by design, so every MCP component is unsupported.

`chatgpt-2026-09` is the Codex contract behind a different detector: the Windows app shares Codex's home directory, and its skills and MCP servers appear in its Codex mode. Its shared resources coalesce with Codex's.

## MCP spelling

The portable document spells a transport as `stdio`, `streamable-http`, or `sse`, and an environment reference as `${NAME}`. Each target writes its own spelling:

| Target                             | Key                  | `type` for stdio / streamable HTTP / SSE | `${NAME}` becomes | Other fields                                    |
| ---------------------------------- | -------------------- | ---------------------------------------- | ----------------- | ----------------------------------------------- |
| Claude Code                        | `mcpServers`         | `stdio` / `http` / `sse`                 | `${NAME}`         |                                                 |
| Cursor                             | `mcpServers`         | `stdio` / `streamable-http` / `sse`      | `${env:NAME}`     |                                                 |
| GitHub Copilot, `mcp-config.json`  | `mcpServers`         | `local` / `http` / `sse`                 | `${NAME}`         | `"tools": ["*"]`                                |
| GitHub Copilot, VS Code `mcp.json` | `servers`            | `stdio` / `http` / `sse`                 | `${env:NAME}`     |                                                 |
| OpenCode                           | `mcp`                | `local` / `remote` / SSE unsupported     | `{env:NAME}`      | `command` array, `environment`, `enabled: true` |
| Codex, ChatGPT, Grok Build         | `mcp_servers` (TOML) | no `type` / no `type` / SSE unsupported  | see below         |                                                 |
| Claude Desktop                     | `mcpServers`         | no `type` / unsupported / unsupported    | unsupported       |                                                 |

Codex expands no references. An `env` entry whose value is exactly `${KEY}`, with `KEY` its own name, becomes `env_vars = ["KEY"]`, which forwards that variable. For a remote server, a header whose value is exactly `${NAME}` goes to `env_http_headers`, `Authorization: Bearer ${NAME}` becomes `bearer_token_env_var = "NAME"`, and a header with no reference goes to `http_headers`. A reference anywhere else (in the command line, the working folder, the URL, or under another variable's name) makes the server unsupported for that target, with a reason that names where.

Configuration files are edited through their syntax tree, JSON included: the entry is added, replaced, or removed, and other keys keep their order, formatting, and comments. When a later version of the app spells an entry differently, a sync rewrites the approved entries for the targets that already have them; see [the app reference](app-reference.md#approvals-and-confirmations).

These spellings follow each app's documentation as of 2026-09. They have not all been checked against the running apps; treat them as file-level evidence until each passes the [registration checks](#registration-checks).

## Capability result

Every component/target pair returns one of `native`, `losslessTranslation`, `lossyTranslation`, `unsupported`, or `blocked`. Lossy results list each lost semantic. Unsupported and blocked results include an actionable reason and are never collapsed into success.

Detection, not a user enable list, chooses which agents are configured. A person can keep one connector out of chosen apps; that choice lives in `package-choices.json`, and the planner skips those pairs before any adapter runs. Skills for every detected agent except Claude Code share `~/.agents/skills`. Claude Code uses `~/.claude/skills`. Cursor and other compatibility scanners may also see the Claude folder.

GitHub Copilot counts only when the Copilot CLI is present, or when a currently installed VS Code/Insiders or JetBrains IDE still has the Copilot extension or plugin. Leftover `github.copilot*` folders after uninstalling the editor, Copilot plugins under older JetBrains config directories, and a bare `~/.copilot` tree do not count.

Desktop apps are detected without running them. On Windows, Claude Desktop (`Claude_pzs8sxrjxfjjc`) and ChatGPT (`OpenAI.ChatGPT-Desktop_2p2nqsd0c76g0`, older `OpenAI.Codex_2p2nqsd0c76g0`) are read from the per-user MSIX package repository in the registry, which also yields the version, or from the package's folder under `%LOCALAPPDATA%\Packages`, which appears on first launch. The pre-2026 Squirrel install of Claude Desktop under `%LOCALAPPDATA%\AnthropicClaude` counts too. On macOS the app bundles under `/Applications` count.

pi counts only when `~/.pi/agent` exists, which pi creates on its first run; `pi` is a common word, so the command alone is not proof. `pi --version` then supplies the version when it resolves.

An unknown dialect may use only the documented shared `~/.agents/skills` projection. Shared-config entries and target-specific skill locations remain blocked until that dialect is explicitly supported.

## Conformance checklist

An adapter is complete only when it has:

- stable target and dialect IDs;
- official documentation links and a last-verified date;
- advisory version detection with conservative behavior for unknown dialects;
- a capability result for every canonical component kind;
- pure desired-resource fixtures and coalescing coverage;
- documented user scope, naming, precedence, and reload semantics;
- transaction and drift coverage for every emitted resource type;
- UI copy for lossy, blocked, and unsupported mappings; and
- disposable runtime discovery evidence. Static file checks prove disk state, not that an agent loaded it.

## Registration checks

Runtime checks must inspect registration without intentionally starting source executables:

- Cursor: reload the window, then inspect the Skills and MCP settings surfaces.
- Claude Code: use its MCP listing command and inspect discovered user skills.
- Codex: inspect configured MCP servers and start a fresh session for skill discovery.
- OpenCode and Grok Build: use the client configuration/MCP inspection surface for the pinned release.
- GitHub Copilot: in VS Code, open the chat's configure-tools and skills views, or run **MCP: List Servers**; in Copilot CLI, list its MCP servers and skills.
- Claude Desktop: quit from the tray, relaunch, then check Settings > Developer or the connectors menu for the MCP server. Repeat before the app's first launch to learn which config path the build reads.
- pi: start a session and type `/skill:` to list skills; `/reload` re-reads them in a running session.
- ChatGPT: open the app, switch to Codex, type `$` for the skill, and check its MCP servers. With Codex CLI also installed, confirm one `config.toml` entry with two consumer bindings.

Run these checks in a disposable home. Record the target version, operating system, reload boundary, command/output, and whether the evidence is file-only or observed runtime discovery.
