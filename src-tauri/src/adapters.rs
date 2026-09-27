//! Compile-time target registry. Adapters translate components and never mutate the machine.

use crate::agent_profiles::{AgentProfile, TargetId, CLAUDE_DESKTOP_MSIX};
use crate::catalog::{CatalogComponent, CatalogComponentKind};
use crate::ledger::OwnedPathKind;
use crate::mcp::McpServer;
use crate::paths::SystemPaths;
use crate::resource::{
    CapabilityResult, DesiredPath, DesiredResource, DesiredStructuredEntry, PathMaterialization,
    StructuredFormat,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) struct PlanningContext<'a> {
    pub(crate) paths: &'a SystemPaths,
    pub(crate) source_root: &'a Path,
}

pub(crate) struct TargetPlan {
    pub(crate) capability: CapabilityResult,
    pub(crate) resources: Vec<DesiredResource>,
    pub(crate) warnings: Vec<String>,
}

impl TargetPlan {
    fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            capability: CapabilityResult::Unsupported {
                reason: reason.into(),
            },
            resources: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn blocked(reason: impl Into<String>, required_action: impl Into<String>) -> Self {
        Self {
            capability: CapabilityResult::Blocked {
                reason: reason.into(),
                required_action: required_action.into(),
            },
            resources: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
enum SkillProjection {
    NativeClaude,
    SharedAgents,
    /// Skills reach the app only through a claude.ai account upload.
    ClaudeAccountUpload,
}

/// How a target spells the portable document's `${NAME}` environment
/// variable reference.
#[derive(Clone, Copy)]
enum EnvSyntax {
    /// `${NAME}`: the app expands it itself (Claude Code, Copilot CLI).
    Dollar,
    /// `${env:NAME}`: Cursor and VS Code.
    DollarEnv,
    /// `{env:NAME}`: OpenCode.
    BraceEnv,
}

#[derive(Clone, Copy)]
enum McpMapping {
    /// `mcpServers` in a JSON file, with `type` spelled `stdio`, `http`, `sse`.
    ClaudeCode,
    /// `~/.cursor/mcp.json`: the portable shape with `${env:NAME}`.
    Cursor,
    /// `~/.copilot/mcp-config.json`, which Copilot CLI and VS Code's agent
    /// host read, plus the user `mcp.json` of each VS Code edition in use,
    /// which VS Code's own chat reads.
    GithubCopilot,
    /// Codex's `config.toml`, shared by the ChatGPT app; Grok Build copies it.
    CodexToml {
        relative: &'static str,
    },
    OpenCode,
    /// `claude_desktop_config.json`, stdio servers only.
    ClaudeDesktop,
    /// The target does not use MCP servers; the reason says what it uses.
    Unsupported(&'static str),
}

#[derive(Clone, Copy)]
pub(crate) struct TargetSpec {
    target_id: TargetId,
    skill: SkillProjection,
    unknown_dialect_allows_shared_skills: bool,
    mcp: McpMapping,
}

/// One MCP entry to write: the document, its format, the key holding the
/// servers, and the server's value there.
type McpEntry = (PathBuf, StructuredFormat, &'static str, Value);

impl TargetSpec {
    fn skill_display_root(self) -> &'static str {
        match self.skill {
            SkillProjection::NativeClaude => "~/.claude/skills",
            SkillProjection::SharedAgents => "~/.agents/skills",
            SkillProjection::ClaudeAccountUpload => {
                "None: Claude Desktop takes skills from claude.ai > Customize > Skills"
            }
        }
    }

    fn reads_shared_agents(self) -> bool {
        matches!(self.skill, SkillProjection::SharedAgents)
    }

    fn sse_unsupported(self) -> bool {
        matches!(
            self.mcp,
            McpMapping::CodexToml { .. } | McpMapping::OpenCode
        )
    }

    /// Where and how this target keeps `server`, or the plain reason it
    /// cannot take it.
    fn mcp_entries(self, server: &McpServer, paths: &SystemPaths) -> Result<Vec<McpEntry>, String> {
        let home = &paths.home;
        let name = self.target_id.display_name();
        match self.mcp {
            McpMapping::ClaudeCode => Ok(vec![(
                home.join(".claude.json"),
                StructuredFormat::Json,
                "mcpServers",
                json_server(server, EnvSyntax::Dollar, Some(["stdio", "http", "sse"])),
            )]),
            McpMapping::Cursor => Ok(vec![(
                home.join(".cursor").join("mcp.json"),
                StructuredFormat::Json,
                "mcpServers",
                json_server(
                    server,
                    EnvSyntax::DollarEnv,
                    Some(["stdio", "streamable-http", "sse"]),
                ),
            )]),
            McpMapping::GithubCopilot => {
                let mut cli =
                    json_server(server, EnvSyntax::Dollar, Some(["local", "http", "sse"]));
                if let Value::Object(object) = &mut cli {
                    object.insert("tools".to_string(), json!(["*"]));
                }
                let mut entries = vec![(
                    home.join(".copilot").join("mcp-config.json"),
                    StructuredFormat::Json,
                    "mcpServers",
                    cli,
                )];
                for user in vscode_user_dirs(paths) {
                    entries.push((
                        user.join("mcp.json"),
                        StructuredFormat::Jsonc,
                        "servers",
                        json_server(server, EnvSyntax::DollarEnv, Some(["stdio", "http", "sse"])),
                    ));
                }
                Ok(entries)
            }
            McpMapping::CodexToml { relative } => Ok(vec![(
                home.join(relative),
                StructuredFormat::Toml,
                "mcp_servers",
                codex_server(server, name)?,
            )]),
            McpMapping::OpenCode => Ok(vec![(
                home.join(".config").join("opencode").join("opencode.jsonc"),
                StructuredFormat::Jsonc,
                "mcp",
                opencode_mcp_value(server),
            )]),
            McpMapping::ClaudeDesktop => {
                if !matches!(server, McpServer::Stdio { .. }) {
                    return Err("Claude Desktop reads only local command servers from its configuration file. Add remote servers in Claude Desktop under Settings > Connectors.".to_string());
                }
                let names = server.environment_names();
                if !names.is_empty() {
                    return Err(format!(
                        "Claude Desktop can't read {} from your settings, so this connector isn't added there.",
                        names.into_iter().collect::<Vec<_>>().join(", ")
                    ));
                }
                // The MSIX build reads a virtualized copy under its package
                // folder, which exists once the app has been launched. The
                // roaming path serves the older installer and macOS.
                let package = paths.local_data.join("Packages").join(CLAUDE_DESKTOP_MSIX);
                let document = if package.is_dir() {
                    package
                        .join("LocalCache")
                        .join("Roaming")
                        .join("Claude")
                        .join("claude_desktop_config.json")
                } else {
                    paths
                        .config
                        .join("Claude")
                        .join("claude_desktop_config.json")
                };
                Ok(vec![(
                    document,
                    StructuredFormat::Json,
                    "mcpServers",
                    json_server(server, EnvSyntax::Dollar, None),
                )])
            }
            McpMapping::Unsupported(reason) => Err(reason.to_string()),
        }
    }
}

/// The primary targets first, matching `TargetId::ALL`.
static SPECS: [TargetSpec; 9] = [
    TargetSpec {
        target_id: TargetId::GithubCopilot,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::GithubCopilot,
    },
    TargetSpec {
        target_id: TargetId::Cursor,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::Cursor,
    },
    TargetSpec {
        target_id: TargetId::ClaudeCode,
        skill: SkillProjection::NativeClaude,
        unknown_dialect_allows_shared_skills: false,
        mcp: McpMapping::ClaudeCode,
    },
    TargetSpec {
        target_id: TargetId::ClaudeDesktop,
        skill: SkillProjection::ClaudeAccountUpload,
        unknown_dialect_allows_shared_skills: false,
        mcp: McpMapping::ClaudeDesktop,
    },
    TargetSpec {
        target_id: TargetId::OpenCode,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::OpenCode,
    },
    TargetSpec {
        target_id: TargetId::Pi,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::Unsupported(
            "pi doesn't use MCP servers; it works with skills and command-line tools.",
        ),
    },
    TargetSpec {
        target_id: TargetId::Codex,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::CodexToml {
            relative: ".codex/config.toml",
        },
    },
    // The ChatGPT app shares Codex's home directory, so it is the Codex entry
    // behind a different detector; shared resources coalesce.
    TargetSpec {
        target_id: TargetId::Chatgpt,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::CodexToml {
            relative: ".codex/config.toml",
        },
    },
    TargetSpec {
        target_id: TargetId::GrokBuild,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::CodexToml {
            relative: ".grok/config.toml",
        },
    },
];

impl TargetSpec {
    pub(crate) fn plan(
        &self,
        component: &CatalogComponent,
        profile: &AgentProfile,
        context: &PlanningContext<'_>,
    ) -> Result<TargetPlan, String> {
        if profile.dialect_id != self.target_id.current_dialect() {
            let shared_skill_is_stable = component.kind == CatalogComponentKind::Skill
                && self.unknown_dialect_allows_shared_skills;
            if !shared_skill_is_stable {
                return Ok(TargetPlan::blocked(
                    format!(
                        "Dialect {} is not recognized by the built-in {} adapter.",
                        profile.dialect_id,
                        self.target_id.display_name()
                    ),
                    "Select a supported dialect after its configuration contract has been verified.",
                ));
            }
        }
        match component.kind {
            CatalogComponentKind::Skill => self.plan_skill(component, context),
            CatalogComponentKind::McpServer => self.plan_mcp(component, context),
        }
    }

    fn plan_skill(
        &self,
        component: &CatalogComponent,
        context: &PlanningContext<'_>,
    ) -> Result<TargetPlan, String> {
        let home = &context.paths.home;
        let (root, capability) = match self.skill {
            SkillProjection::NativeClaude => (
                home.join(".claude").join("skills"),
                CapabilityResult::Native,
            ),
            SkillProjection::SharedAgents => (
                home.join(".agents").join("skills"),
                CapabilityResult::LosslessTranslation,
            ),
            SkillProjection::ClaudeAccountUpload => {
                return Ok(TargetPlan::unsupported(
                    "Claude Desktop takes skills only from your claude.ai account (Customize > Skills), not from this computer.",
                ));
            }
        };
        Ok(TargetPlan {
            capability,
            resources: vec![DesiredResource::Path(DesiredPath {
                path: root.join(&component.effective_name),
                kind: OwnedPathKind::Directory,
                source: context.source_root.join(&component.source),
                source_digest: component.digest.clone(),
                materialization: PathMaterialization::AgentSkill {
                    effective_name: component.effective_name.clone(),
                    disable_model_invocation: None,
                },
            })],
            warnings: Vec::new(),
        })
    }

    fn plan_mcp(
        &self,
        component: &CatalogComponent,
        context: &PlanningContext<'_>,
    ) -> Result<TargetPlan, String> {
        let server = component
            .mcp_server
            .as_ref()
            .ok_or_else(|| format!("MCP component {} has no server definition.", component.id))?;
        if matches!(server, McpServer::Sse { .. }) && self.sse_unsupported() {
            return Ok(TargetPlan::unsupported(format!(
                "{} doesn't support the older SSE connection this server uses.",
                self.target_id.display_name()
            )));
        }
        let entries = match self.mcp_entries(server, context.paths) {
            Ok(entries) => entries,
            Err(reason) => return Ok(TargetPlan::unsupported(reason)),
        };
        Ok(TargetPlan {
            capability: CapabilityResult::LosslessTranslation,
            resources: entries
                .into_iter()
                .map(|(document_path, format, key_root, value)| {
                    DesiredResource::StructuredEntry(DesiredStructuredEntry {
                        document_path,
                        format,
                        key_path: vec![key_root.to_string(), component.effective_name.clone()],
                        value,
                    })
                })
                .collect(),
            warnings: vec![
                "This MCP server may start a local process or access a remote service when the target uses it."
                    .to_string(),
            ],
        })
    }
}

/// The user settings folder of each VS Code edition that has been started
/// here. VS Code's chat reads `mcp.json` there.
fn vscode_user_dirs(paths: &SystemPaths) -> Vec<PathBuf> {
    ["Code", "Code - Insiders"]
        .into_iter()
        .map(|edition| paths.config.join(edition).join("User"))
        .filter(|user| user.is_dir())
        .collect()
}

/// `text` with each `${NAME}` spelled the way `syntax` says.
fn with_syntax(text: &str, syntax: EnvSyntax) -> String {
    let mut spelled = String::with_capacity(text.len());
    let mut last = 0;
    for (range, name) in crate::mcp::environment_references(text) {
        spelled.push_str(&text[last..range.start]);
        match syntax {
            EnvSyntax::Dollar => spelled.push_str(&text[range.clone()]),
            EnvSyntax::DollarEnv => spelled.push_str(&format!("${{env:{name}}}")),
            EnvSyntax::BraceEnv => spelled.push_str(&format!("{{env:{name}}}")),
        }
        last = range.end;
    }
    spelled.push_str(&text[last..]);
    spelled
}

fn spelled_map(map: &BTreeMap<String, String>, syntax: EnvSyntax) -> Value {
    Value::Object(
        map.iter()
            .map(|(key, value)| (key.clone(), Value::String(with_syntax(value, syntax))))
            .collect(),
    )
}

/// The server as a JSON `mcpServers` entry. `types` spells `type` for stdio,
/// streamable HTTP, and SSE; `None` leaves it out for readers that go by the
/// fields present.
fn json_server(server: &McpServer, syntax: EnvSyntax, types: Option<[&str; 3]>) -> Value {
    let text = |value: &str| Value::String(with_syntax(value, syntax));
    let mut object = Map::new();
    let kind = match server {
        McpServer::Stdio {
            command,
            args,
            env,
            cwd,
        } => {
            object.insert("command".to_string(), text(command));
            if !args.is_empty() {
                object.insert(
                    "args".to_string(),
                    Value::Array(args.iter().map(|arg| text(arg)).collect()),
                );
            }
            if !env.is_empty() {
                object.insert("env".to_string(), spelled_map(env, syntax));
            }
            if let Some(cwd) = cwd {
                object.insert("cwd".to_string(), text(cwd));
            }
            0
        }
        McpServer::StreamableHttp { url, headers } | McpServer::Sse { url, headers } => {
            object.insert("url".to_string(), text(url));
            if !headers.is_empty() {
                object.insert("headers".to_string(), spelled_map(headers, syntax));
            }
            if matches!(server, McpServer::Sse { .. }) {
                2
            } else {
                1
            }
        }
    };
    if let Some(types) = types {
        object.insert("type".to_string(), Value::String(types[kind].to_string()));
    }
    Value::Object(object)
}

/// Codex's `[mcp_servers.<name>]` table. Codex expands no references: it
/// forwards named variables (`env_vars`) and reads header values from
/// variables (`env_http_headers`, `bearer_token_env_var`), so a reference
/// anywhere else cannot be expressed.
fn codex_server(server: &McpServer, app: &str) -> Result<Value, String> {
    let cannot = |place: &str| {
        format!("{app} can't fill in an environment variable inside {place}, so this connector isn't added there.")
    };
    let has_reference = |text: &str| !crate::mcp::environment_references(text).is_empty();
    let mut object = Map::new();
    match server {
        McpServer::Stdio {
            command,
            args,
            env,
            cwd,
        } => {
            if has_reference(command) || args.iter().any(|arg| has_reference(arg)) {
                return Err(cannot("its command line"));
            }
            object.insert("command".to_string(), Value::String(command.clone()));
            if !args.is_empty() {
                object.insert("args".to_string(), json!(args));
            }
            let mut literal = Map::new();
            let mut forwarded = Vec::new();
            for (key, value) in env {
                match crate::mcp::environment_references(value).as_slice() {
                    [] => {
                        literal.insert(key.clone(), Value::String(value.clone()));
                    }
                    [(range, name)] if *name == key && range.len() == value.len() => {
                        forwarded.push(Value::String(key.clone()));
                    }
                    _ => {
                        return Err(format!(
                            "{app} can pass an environment variable to a connector only under its own name, and this one sets {key} from something else."
                        ))
                    }
                }
            }
            if !literal.is_empty() {
                object.insert("env".to_string(), Value::Object(literal));
            }
            if !forwarded.is_empty() {
                object.insert("env_vars".to_string(), Value::Array(forwarded));
            }
            if let Some(cwd) = cwd {
                if has_reference(cwd) {
                    return Err(cannot("its working folder"));
                }
                object.insert("cwd".to_string(), Value::String(cwd.clone()));
            }
        }
        McpServer::StreamableHttp { url, headers } | McpServer::Sse { url, headers } => {
            if has_reference(url) {
                return Err(cannot("its address"));
            }
            object.insert("url".to_string(), Value::String(url.clone()));
            let mut literal = Map::new();
            let mut from_env = Map::new();
            for (header, value) in headers {
                let references = crate::mcp::environment_references(value);
                match references.as_slice() {
                    [] => {
                        literal.insert(header.clone(), Value::String(value.clone()));
                    }
                    [(range, name)] if range.len() == value.len() => {
                        from_env.insert(header.clone(), Value::String((*name).to_string()));
                    }
                    [(range, name)]
                        if header.eq_ignore_ascii_case("authorization")
                            && value[..range.start].eq_ignore_ascii_case("bearer ")
                            && range.end == value.len() =>
                    {
                        object.insert(
                            "bearer_token_env_var".to_string(),
                            Value::String((*name).to_string()),
                        );
                    }
                    _ => return Err(cannot(&format!("its {header} header"))),
                }
            }
            if !literal.is_empty() {
                object.insert("http_headers".to_string(), Value::Object(literal));
            }
            if !from_env.is_empty() {
                object.insert("env_http_headers".to_string(), Value::Object(from_env));
            }
        }
    }
    Ok(Value::Object(object))
}

fn opencode_mcp_value(server: &McpServer) -> Value {
    let text = |value: &str| Value::String(with_syntax(value, EnvSyntax::BraceEnv));
    match server {
        McpServer::Stdio {
            command,
            args,
            env,
            cwd,
        } => {
            let mut object = Map::from_iter([
                ("type".to_string(), Value::String("local".to_string())),
                (
                    "command".to_string(),
                    Value::Array(
                        std::iter::once(command)
                            .chain(args)
                            .map(|arg| text(arg))
                            .collect(),
                    ),
                ),
                ("enabled".to_string(), Value::Bool(true)),
            ]);
            if !env.is_empty() {
                object.insert(
                    "environment".to_string(),
                    spelled_map(env, EnvSyntax::BraceEnv),
                );
            }
            if let Some(cwd) = cwd {
                object.insert("cwd".to_string(), text(cwd));
            }
            Value::Object(object)
        }
        McpServer::StreamableHttp { url, headers } | McpServer::Sse { url, headers } => {
            let mut object = Map::from_iter([
                ("type".to_string(), Value::String("remote".to_string())),
                ("url".to_string(), text(url)),
                ("enabled".to_string(), Value::Bool(true)),
            ]);
            if !headers.is_empty() {
                object.insert(
                    "headers".to_string(),
                    spelled_map(headers, EnvSyntax::BraceEnv),
                );
            }
            Value::Object(object)
        }
    }
}

pub(crate) fn adapter(target_id: TargetId) -> &'static TargetSpec {
    SPECS
        .iter()
        .find(|candidate| candidate.target_id == target_id)
        .expect("every stable target has a built-in spec")
}

pub(crate) fn reads_shared_agents(target_id: TargetId) -> bool {
    adapter(target_id).reads_shared_agents()
}

pub(crate) fn skill_display_root(target_id: TargetId) -> &'static str {
    adapter(target_id).skill_display_root()
}

/// The folder `target_id` reads skills from, when it reads them from disk.
pub(crate) fn skill_root(target_id: TargetId, paths: &SystemPaths) -> Option<PathBuf> {
    let home = &paths.home;
    match adapter(target_id).skill {
        SkillProjection::NativeClaude => Some(home.join(".claude").join("skills")),
        SkillProjection::SharedAgents => Some(home.join(".agents").join("skills")),
        SkillProjection::ClaudeAccountUpload => None,
    }
}

pub(crate) fn managed_skill_roots(paths: &SystemPaths) -> Vec<PathBuf> {
    let home = &paths.home;
    let mut roots = vec![
        home.join(".agents").join("skills"),
        home.join(".claude").join("skills"),
        home.join(".cursor").join("skills"),
        home.join(".copilot").join("skills"),
        home.join(".grok").join("skills"),
        home.join(".config").join("opencode").join("skills"),
    ];
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogComponent, CatalogComponentKind};
    use std::collections::BTreeMap;
    use std::fs;

    fn paths(root: &Path) -> SystemPaths {
        SystemPaths {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            local_data: root.join("local-data"),
            cache: root.join("cache"),
        }
    }

    fn write_skill_source(root: &Path) -> PathBuf {
        let dir = root.join("skills/review");
        fs::create_dir_all(&dir).expect("skill dir");
        fs::write(
            dir.join("SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\nBody\n",
        )
        .expect("skill file");
        dir
    }

    fn skill_component() -> CatalogComponent {
        CatalogComponent {
            id: "review".to_string(),
            kind: CatalogComponentKind::Skill,
            source: "skills/review".to_string(),
            source_is_directory: true,
            digest: "skill-digest".to_string(),
            effective_name: "acme-review".to_string(),
            description: "Review code.".to_string(),
            disable_model_invocation: false,
            mcp_server: None,
        }
    }

    fn stdio_component() -> CatalogComponent {
        CatalogComponent {
            id: "database".to_string(),
            kind: CatalogComponentKind::McpServer,
            source: "mcp/database.json".to_string(),
            source_is_directory: false,
            digest: "mcp-digest".to_string(),
            effective_name: "acme-database".to_string(),
            description: "Runs node.".to_string(),
            disable_model_invocation: false,
            mcp_server: Some(McpServer::Stdio {
                command: "node".to_string(),
                args: vec!["server.js".to_string()],
                env: BTreeMap::new(),
                cwd: None,
            }),
        }
    }

    fn plan_for(
        target_id: TargetId,
        component: &CatalogComponent,
        paths: &SystemPaths,
        source_root: &Path,
    ) -> TargetPlan {
        let profile = AgentProfile {
            target_id,
            enabled: true,
            scopes: vec!["user".to_string()],
            dialect_id: target_id.current_dialect(),
        };
        adapter(target_id)
            .plan(component, &profile, &PlanningContext { paths, source_root })
            .expect("plan")
    }

    #[test]
    fn opencode_stdio_mapping_preserves_command_cwd_and_environment() {
        let server = McpServer::Stdio {
            command: "node".to_string(),
            args: vec!["server.js".to_string()],
            env: BTreeMap::from([("MODE".to_string(), "safe".to_string())]),
            cwd: Some("/tmp/plugin".to_string()),
        };
        assert_eq!(
            opencode_mcp_value(&server),
            json!({
                "type": "local",
                "command": ["node", "server.js"],
                "enabled": true,
                "environment": {"MODE": "safe"},
                "cwd": "/tmp/plugin"
            })
        );
    }

    #[test]
    fn unknown_dialect_blocks_shared_config_but_allows_shared_skills() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let profile = AgentProfile {
            target_id: TargetId::Codex,
            enabled: true,
            scopes: vec!["user".to_string()],
            dialect_id: "codex-future".to_string(),
        };
        let context = PlanningContext {
            paths: &paths,
            source_root: root.path(),
        };
        let skill = skill_component();
        let skill_plan = adapter(TargetId::Codex)
            .plan(&skill, &profile, &context)
            .expect("skill plan");
        assert!(skill_plan.capability.is_supported());
        assert_eq!(skill_plan.resources.len(), 1);

        let mcp = stdio_component();
        let mcp_plan = adapter(TargetId::Codex)
            .plan(&mcp, &profile, &context)
            .expect("MCP plan");
        assert!(matches!(
            mcp_plan.capability,
            CapabilityResult::Blocked { .. }
        ));
        assert!(mcp_plan.resources.is_empty());
    }

    #[test]
    fn every_target_projects_the_current_skill_and_mcp_identities() {
        let root = tempfile::tempdir().expect("root");
        write_skill_source(root.path());
        let paths = paths(root.path());
        let context = PlanningContext {
            paths: &paths,
            source_root: root.path(),
        };
        let skill = skill_component();
        let mcp = stdio_component();
        let expected_skill = [
            (
                TargetId::Cursor,
                paths.home.join(".agents/skills/acme-review"),
            ),
            (
                TargetId::ClaudeCode,
                paths.home.join(".claude/skills/acme-review"),
            ),
            (
                TargetId::Codex,
                paths.home.join(".agents/skills/acme-review"),
            ),
            (
                TargetId::OpenCode,
                paths.home.join(".agents/skills/acme-review"),
            ),
            (
                TargetId::GrokBuild,
                paths.home.join(".agents/skills/acme-review"),
            ),
            (
                TargetId::GithubCopilot,
                paths.home.join(".agents/skills/acme-review"),
            ),
            (
                TargetId::Chatgpt,
                paths.home.join(".agents/skills/acme-review"),
            ),
            (TargetId::Pi, paths.home.join(".agents/skills/acme-review")),
        ];
        let expected_mcp = [
            (
                TargetId::Cursor,
                paths.home.join(".cursor/mcp.json"),
                StructuredFormat::Json,
                vec!["mcpServers".to_string(), "acme-database".to_string()],
            ),
            (
                TargetId::ClaudeCode,
                paths.home.join(".claude.json"),
                StructuredFormat::Json,
                vec!["mcpServers".to_string(), "acme-database".to_string()],
            ),
            (
                TargetId::Codex,
                paths.home.join(".codex/config.toml"),
                StructuredFormat::Toml,
                vec!["mcp_servers".to_string(), "acme-database".to_string()],
            ),
            (
                TargetId::OpenCode,
                paths.home.join(".config/opencode/opencode.jsonc"),
                StructuredFormat::Jsonc,
                vec!["mcp".to_string(), "acme-database".to_string()],
            ),
            (
                TargetId::GrokBuild,
                paths.home.join(".grok/config.toml"),
                StructuredFormat::Toml,
                vec!["mcp_servers".to_string(), "acme-database".to_string()],
            ),
            (
                TargetId::GithubCopilot,
                paths.home.join(".copilot/mcp-config.json"),
                StructuredFormat::Json,
                vec!["mcpServers".to_string(), "acme-database".to_string()],
            ),
            (
                TargetId::Chatgpt,
                paths.home.join(".codex/config.toml"),
                StructuredFormat::Toml,
                vec!["mcp_servers".to_string(), "acme-database".to_string()],
            ),
            (
                TargetId::ClaudeDesktop,
                paths.config.join("Claude/claude_desktop_config.json"),
                StructuredFormat::Json,
                vec!["mcpServers".to_string(), "acme-database".to_string()],
            ),
        ];
        assert_eq!(SPECS.len(), TargetId::ALL.len());
        for (target_id, skill_path) in expected_skill {
            let profile = AgentProfile {
                target_id,
                enabled: true,
                scopes: vec!["user".to_string()],
                dialect_id: target_id.current_dialect(),
            };
            let plan = adapter(target_id)
                .plan(&skill, &profile, &context)
                .expect("skill plan");
            match &plan.resources[0] {
                DesiredResource::Path(path) => assert_eq!(path.path, skill_path),
                other => panic!("expected skill path, got {other:?}"),
            }
        }
        for (target_id, document, format, key_path) in expected_mcp {
            let profile = AgentProfile {
                target_id,
                enabled: true,
                scopes: vec!["user".to_string()],
                dialect_id: target_id.current_dialect(),
            };
            let plan = adapter(target_id)
                .plan(&mcp, &profile, &context)
                .expect("mcp plan");
            match &plan.resources[0] {
                DesiredResource::StructuredEntry(entry) => {
                    assert_eq!(entry.document_path, document);
                    assert_eq!(entry.format, format);
                    assert_eq!(entry.key_path, key_path);
                }
                other => panic!("expected MCP entry, got {other:?}"),
            }
        }
    }

    #[test]
    fn claude_desktop_skills_are_unsupported_with_upload_guidance() {
        let root = tempfile::tempdir().expect("root");
        let plan = plan_for(
            TargetId::ClaudeDesktop,
            &skill_component(),
            &paths(root.path()),
            root.path(),
        );
        match plan.capability {
            CapabilityResult::Unsupported { reason } => {
                assert!(reason.contains("claude.ai"));
            }
            other => panic!("expected unsupported, got {other:?}"),
        }
        assert!(plan.resources.is_empty());
    }

    #[test]
    fn claude_desktop_prefers_the_msix_config_when_the_package_folder_exists() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let package = paths.local_data.join("Packages").join(CLAUDE_DESKTOP_MSIX);
        fs::create_dir_all(&package).expect("package dir");
        let plan = plan_for(
            TargetId::ClaudeDesktop,
            &stdio_component(),
            &paths,
            root.path(),
        );
        match &plan.resources[0] {
            DesiredResource::StructuredEntry(entry) => {
                assert_eq!(
                    entry.document_path,
                    package.join("LocalCache/Roaming/Claude/claude_desktop_config.json")
                );
                assert_eq!(entry.value.get("type"), None);
                assert_eq!(entry.value["command"], "node");
            }
            other => panic!("expected MCP entry, got {other:?}"),
        }
    }

    #[test]
    fn claude_desktop_rejects_remote_servers() {
        let root = tempfile::tempdir().expect("root");
        let mut component = stdio_component();
        component.mcp_server = Some(McpServer::StreamableHttp {
            url: "https://mcp.example.com/mcp".to_string(),
            headers: BTreeMap::new(),
        });
        let plan = plan_for(
            TargetId::ClaudeDesktop,
            &component,
            &paths(root.path()),
            root.path(),
        );
        assert!(matches!(
            plan.capability,
            CapabilityResult::Unsupported { .. }
        ));
        assert!(plan.resources.is_empty());
    }

    fn remote_component(headers: &[(&str, &str)]) -> CatalogComponent {
        let mut component = stdio_component();
        component.mcp_server = Some(McpServer::StreamableHttp {
            url: "https://mcp.example.com/mcp".to_string(),
            headers: headers
                .iter()
                .map(|(key, value)| (key.to_string(), value.to_string()))
                .collect(),
        });
        component
    }

    fn entries(plan: &TargetPlan) -> Vec<&DesiredStructuredEntry> {
        plan.resources
            .iter()
            .map(|resource| match resource {
                DesiredResource::StructuredEntry(entry) => entry,
                other => panic!("expected an MCP entry, got {other:?}"),
            })
            .collect()
    }

    #[test]
    fn remote_servers_use_each_apps_own_type_and_reference_spelling() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let remote = remote_component(&[("Authorization", "Bearer ${ACME_TOKEN}")]);
        let value = |target| {
            entries(&plan_for(target, &remote, &paths, root.path()))[0]
                .value
                .clone()
        };
        assert_eq!(
            value(TargetId::ClaudeCode),
            json!({"type": "http", "url": "https://mcp.example.com/mcp", "headers": {"Authorization": "Bearer ${ACME_TOKEN}"}})
        );
        assert_eq!(
            value(TargetId::Cursor)["headers"]["Authorization"],
            "Bearer ${env:ACME_TOKEN}"
        );
        assert_eq!(
            value(TargetId::GithubCopilot),
            json!({"type": "http", "url": "https://mcp.example.com/mcp", "tools": ["*"], "headers": {"Authorization": "Bearer ${ACME_TOKEN}"}})
        );
        assert_eq!(
            value(TargetId::OpenCode)["headers"]["Authorization"],
            "Bearer {env:ACME_TOKEN}"
        );
        assert_eq!(
            value(TargetId::Codex),
            json!({"url": "https://mcp.example.com/mcp", "bearer_token_env_var": "ACME_TOKEN"})
        );
        let plain = remote_component(&[("X-Api-Key", "${ACME_KEY}"), ("X-Team", "blue")]);
        assert_eq!(
            entries(&plan_for(TargetId::Codex, &plain, &paths, root.path()))[0].value,
            json!({
                "url": "https://mcp.example.com/mcp",
                "http_headers": {"X-Team": "blue"},
                "env_http_headers": {"X-Api-Key": "ACME_KEY"}
            })
        );
    }

    #[test]
    fn codex_forwards_variables_by_name_and_refuses_what_it_cannot_express() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let mut component = stdio_component();
        component.mcp_server = Some(McpServer::Stdio {
            command: "uvx".to_string(),
            args: vec!["weather-mcp".to_string()],
            env: BTreeMap::from([
                ("API_KEY".to_string(), "${API_KEY}".to_string()),
                ("MODE".to_string(), "safe".to_string()),
            ]),
            cwd: None,
        });
        assert_eq!(
            entries(&plan_for(TargetId::Codex, &component, &paths, root.path()))[0].value,
            json!({"command": "uvx", "args": ["weather-mcp"], "env": {"MODE": "safe"}, "env_vars": ["API_KEY"]})
        );
        component.mcp_server = Some(McpServer::Stdio {
            command: "uvx".to_string(),
            args: vec!["--key=${API_KEY}".to_string()],
            env: BTreeMap::new(),
            cwd: None,
        });
        let plan = plan_for(TargetId::Codex, &component, &paths, root.path());
        assert!(matches!(
            plan.capability,
            CapabilityResult::Unsupported { .. }
        ));
        assert_eq!(
            entries(&plan_for(
                TargetId::ClaudeCode,
                &component,
                &paths,
                root.path()
            ))[0]
                .value["args"],
            json!(["--key=${API_KEY}"])
        );
    }

    #[test]
    fn github_copilot_also_writes_each_started_vscode_editions_user_mcp_json() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let only_cli = plan_for(
            TargetId::GithubCopilot,
            &stdio_component(),
            &paths,
            root.path(),
        );
        assert_eq!(entries(&only_cli).len(), 1);
        assert_eq!(entries(&only_cli)[0].value["type"], "local");
        fs::create_dir_all(paths.config.join("Code/User")).expect("vscode user dir");
        let plan = plan_for(
            TargetId::GithubCopilot,
            &stdio_component(),
            &paths,
            root.path(),
        );
        let vscode = entries(&plan)[1];
        assert_eq!(
            vscode.document_path,
            paths.config.join("Code/User/mcp.json")
        );
        assert_eq!(vscode.format, StructuredFormat::Jsonc);
        assert_eq!(vscode.key_path, ["servers", "acme-database"]);
        assert_eq!(vscode.value["type"], "stdio");
    }

    #[test]
    fn pi_takes_skills_but_no_mcp_servers() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let plan = plan_for(TargetId::Pi, &stdio_component(), &paths, root.path());
        assert!(matches!(
            plan.capability,
            CapabilityResult::Unsupported { .. }
        ));
        assert!(plan.resources.is_empty());
    }

    #[test]
    fn claude_desktop_refuses_servers_that_read_environment_variables() {
        let root = tempfile::tempdir().expect("root");
        let mut component = stdio_component();
        component.mcp_server = Some(McpServer::Stdio {
            command: "uvx".to_string(),
            args: Vec::new(),
            env: BTreeMap::from([("API_KEY".to_string(), "${API_KEY}".to_string())]),
            cwd: None,
        });
        let plan = plan_for(
            TargetId::ClaudeDesktop,
            &component,
            &paths(root.path()),
            root.path(),
        );
        match plan.capability {
            CapabilityResult::Unsupported { reason } => assert!(reason.contains("API_KEY")),
            other => panic!("expected unsupported, got {other:?}"),
        }
    }
}
