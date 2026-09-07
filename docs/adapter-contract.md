# Target adapter contract

This reference defines the acceptance bar for a built-in target adapter. Adapters are pure planners: they may inspect a validated component and profile, but they may not write files, start processes, change the ledger, download code, or suppress errors.

## Stable targets and pinned dialects

| Target                | Stable ID        | User-scope dialect       | Skills                                | MCP                                      |
| --------------------- | ---------------- | ------------------------ | ------------------------------------- | ---------------------------------------- |
| Cursor                | `cursor`         | `cursor-2026-08`         | `~/.agents/skills`                    | `~/.cursor/mcp.json`                     |
| Claude Code           | `claude-code`    | `claude-code-2026-08`    | `~/.claude/skills`                    | `~/.claude.json`                         |
| Codex                 | `codex`          | `codex-2026-08`          | `~/.agents/skills`                    | `~/.codex/config.toml`                   |
| OpenCode              | `opencode`       | `opencode-2026-08`       | `~/.agents/skills`                    | user `opencode.jsonc`                    |
| Grok Build            | `grok-build`     | `grok-build-2026-08`     | `~/.agents/skills`                    | `~/.grok/config.toml`                    |
| GitHub Copilot        | `github-copilot` | `github-copilot-2026-08` | `~/.agents/skills`                    | `~/.copilot/mcp-config.json`             |
| Claude Desktop        | `claude-desktop` | `claude-desktop-2026-09` | Unsupported: claude.ai account upload | `claude_desktop_config.json`, stdio only |
| ChatGPT               | `chatgpt`        | `chatgpt-2026-09`        | `~/.agents/skills`                    | `~/.codex/config.toml`                   |
| Microsoft 365 Copilot | `m365-copilot`   | `m365-copilot-2026-09`   | OneDrive `Documents/Cowork/skills`    | Unsupported                              |

A v2 package may contain several skill and MCP components. Agent Plugins does not install native `agent-plugin@1.0.0` package trees or always-on instruction files.

`opencode-2026-08` pins the stable `opencode.jsonc` contract with a root `mcp` object. The separate beta/v2 `mcp.servers` contract requires a new dialect and is not selected implicitly.

`claude-desktop-2026-09` covers the Chat and Cowork tabs. Both load skills from the claude.ai account, so the skill result is unsupported and says where skills are managed. The MCP document is the MSIX build's virtualized copy under `%LOCALAPPDATA%\Packages\Claude_pzs8sxrjxfjjc\LocalCache\Roaming\Claude\` when that package folder exists, otherwise `%APPDATA%\Claude\` (macOS: `~/Library/Application Support/Claude/`). The file accepts only stdio servers; remote transports are unsupported and point at Settings > Connectors.

`chatgpt-2026-09` is the Codex contract behind a different detector: the Windows app shares Codex's home directory. Its shared resources coalesce with Codex's.

`m365-copilot-2026-09` writes skills to `Documents/Cowork/skills/<name>` in the OneDrive for work or school sync root, which Cowork reads on the web, in the Microsoft 365 Copilot app, and on mobile at the start of each new conversation. Skills over Microsoft's limits (SKILL.md 1 MB, 20 companion files, 10 MB per skill) are unsupported. There is no local MCP surface.

## Capability result

Every component/target pair returns one of `native`, `losslessTranslation`, `lossyTranslation`, `unsupported`, or `blocked`. Lossy results list each lost semantic. Unsupported and blocked results include an actionable reason and are never collapsed into success.

Detection, not a user enable list, chooses which agents are configured. Skills for every detected agent except Claude Code share `~/.agents/skills`. Claude Code uses `~/.claude/skills`. Cursor and other compatibility scanners may also see the Claude folder.

GitHub Copilot counts only when the Copilot CLI is present, or when a currently installed VS Code/Insiders or JetBrains IDE still has the Copilot extension or plugin. Leftover `github.copilot*` folders after uninstalling the editor, Copilot plugins under older JetBrains config directories, and a bare `~/.copilot` tree do not count.

Desktop apps are detected without running them. On Windows, Claude Desktop (`Claude_pzs8sxrjxfjjc`) and ChatGPT (`OpenAI.ChatGPT-Desktop_2p2nqsd0c76g0`, older `OpenAI.Codex_2p2nqsd0c76g0`) are read from the per-user MSIX package repository in the registry, which also yields the version, or from the package's folder under `%LOCALAPPDATA%\Packages`, which appears on first launch. The pre-2026 Squirrel install of Claude Desktop under `%LOCALAPPDATA%\AnthropicClaude` counts too. On macOS the app bundles under `/Applications` count. Microsoft 365 Copilot counts only when OneDrive for work or school is signed in (`OneDriveCommercial`, or the `Business1` account in the registry) and its `Documents\Cowork` folder already exists; OneDrive alone is on nearly every corporate machine and would otherwise create a Cowork folder for everyone.

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
- GitHub Copilot: inspect skills in the IDE Copilot agent, or use Copilot CLI if it is installed.
- Claude Desktop: quit from the tray, relaunch, then check Settings > Developer or the connectors menu for the MCP server. Repeat before the app's first launch to learn which config path the build reads.
- ChatGPT: open the app, switch to Codex, type `$` for the skill, and check its MCP servers. With Codex CLI also installed, confirm one `config.toml` entry with two consumer bindings.
- Microsoft 365 Copilot: wait for OneDrive to sync, start a new Cowork conversation at m365.cloud.microsoft, and confirm the skill is listed. Confirm nothing is written when `Documents\Cowork` is absent.

Run these checks in a disposable home. Record the target version, operating system, reload boundary, command/output, and whether the evidence is file-only or observed runtime discovery.
