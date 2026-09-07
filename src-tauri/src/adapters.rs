//! Compile-time target registry. Adapters translate components and never mutate the machine.

use crate::agent_profiles::{AgentProfile, TargetId, CLAUDE_DESKTOP_MSIX, COWORK_RELATIVE};
use crate::catalog::{CatalogComponent, CatalogComponentKind};
use crate::ledger::OwnedPathKind;
use crate::mcp::McpServer;
use crate::paths::SystemPaths;
use crate::resource::{
    CapabilityResult, DesiredPath, DesiredResource, DesiredStructuredEntry, PathMaterialization,
    StructuredFormat,
};
use serde_json::{json, Map, Value};
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

pub(crate) trait TargetAdapter: Sync {
    fn target_id(&self) -> TargetId;

    fn plan(
        &self,
        component: &CatalogComponent,
        profile: &AgentProfile,
        context: &PlanningContext<'_>,
    ) -> Result<TargetPlan, String>;
}

#[derive(Clone, Copy)]
enum SkillProjection {
    NativeClaude,
    SharedAgents,
    /// Skills reach the app only through a claude.ai account upload.
    ClaudeAccountUpload,
    /// Microsoft 365 Copilot Cowork reads skills from OneDrive.
    CoworkOneDrive,
}

#[derive(Clone, Copy)]
enum McpMapping {
    StandardJson {
        relative: &'static str,
        key: &'static str,
    },
    Toml {
        relative: &'static str,
        key: &'static str,
    },
    OpenCode,
    /// `claude_desktop_config.json`, stdio servers only.
    ClaudeDesktop,
    /// No local configuration file exists for this target.
    Unsupported,
}

#[derive(Clone, Copy)]
struct TargetSpec {
    target_id: TargetId,
    skill: SkillProjection,
    unknown_dialect_allows_shared_skills: bool,
    mcp: McpMapping,
    sse_unsupported: bool,
}

impl TargetSpec {
    fn skill_display_root(self) -> &'static str {
        match self.skill {
            SkillProjection::NativeClaude => "~/.claude/skills",
            SkillProjection::SharedAgents => "~/.agents/skills",
            SkillProjection::ClaudeAccountUpload => "claude.ai > Customize > Skills (upload)",
            SkillProjection::CoworkOneDrive => "OneDrive/Documents/Cowork/skills",
        }
    }

    fn reads_shared_agents(self) -> bool {
        matches!(self.skill, SkillProjection::SharedAgents)
    }

    fn mcp_document(
        self,
        paths: &SystemPaths,
    ) -> Option<(PathBuf, StructuredFormat, &'static str)> {
        let home = &paths.home;
        match self.mcp {
            McpMapping::StandardJson { relative, key } => {
                Some((home.join(relative), StructuredFormat::Json, key))
            }
            McpMapping::Toml { relative, key } => {
                Some((home.join(relative), StructuredFormat::Toml, key))
            }
            McpMapping::OpenCode => Some((
                home.join(".config/opencode/opencode.jsonc"),
                StructuredFormat::Jsonc,
                "mcp",
            )),
            // The MSIX build reads a virtualized copy under its package folder,
            // which exists once the app has been launched. The roaming path
            // serves the older installer and macOS.
            McpMapping::ClaudeDesktop => {
                let package = paths.local_data.join("Packages").join(CLAUDE_DESKTOP_MSIX);
                let document = if package.is_dir() {
                    package.join("LocalCache/Roaming/Claude/claude_desktop_config.json")
                } else {
                    paths.config.join("Claude/claude_desktop_config.json")
                };
                Some((document, StructuredFormat::Json, "mcpServers"))
            }
            McpMapping::Unsupported => None,
        }
    }

    fn mcp_value(self, server: &McpServer) -> Result<Value, String> {
        match self.mcp {
            McpMapping::StandardJson { .. } => standard_mcp_value(server),
            McpMapping::Toml { .. } | McpMapping::ClaudeDesktop => untyped_mcp_value(server),
            McpMapping::OpenCode => Ok(opencode_mcp_value(server)),
            McpMapping::Unsupported => Err(format!(
                "{} has no local MCP configuration file.",
                self.target_id.display_name()
            )),
        }
    }
}

const SPECS: [TargetSpec; 9] = [
    TargetSpec {
        target_id: TargetId::Cursor,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::StandardJson {
            relative: ".cursor/mcp.json",
            key: "mcpServers",
        },
        sse_unsupported: false,
    },
    TargetSpec {
        target_id: TargetId::ClaudeCode,
        skill: SkillProjection::NativeClaude,
        unknown_dialect_allows_shared_skills: false,
        mcp: McpMapping::StandardJson {
            relative: ".claude.json",
            key: "mcpServers",
        },
        sse_unsupported: false,
    },
    TargetSpec {
        target_id: TargetId::Codex,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::Toml {
            relative: ".codex/config.toml",
            key: "mcp_servers",
        },
        sse_unsupported: true,
    },
    TargetSpec {
        target_id: TargetId::OpenCode,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::OpenCode,
        sse_unsupported: true,
    },
    TargetSpec {
        target_id: TargetId::GrokBuild,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::Toml {
            relative: ".grok/config.toml",
            key: "mcp_servers",
        },
        sse_unsupported: true,
    },
    TargetSpec {
        target_id: TargetId::GithubCopilot,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::StandardJson {
            relative: ".copilot/mcp-config.json",
            key: "mcpServers",
        },
        sse_unsupported: false,
    },
    TargetSpec {
        target_id: TargetId::ClaudeDesktop,
        skill: SkillProjection::ClaudeAccountUpload,
        unknown_dialect_allows_shared_skills: false,
        mcp: McpMapping::ClaudeDesktop,
        sse_unsupported: false,
    },
    // The ChatGPT app shares Codex's home directory, so it is the Codex entry
    // behind a different detector; shared resources coalesce.
    TargetSpec {
        target_id: TargetId::Chatgpt,
        skill: SkillProjection::SharedAgents,
        unknown_dialect_allows_shared_skills: true,
        mcp: McpMapping::Toml {
            relative: ".codex/config.toml",
            key: "mcp_servers",
        },
        sse_unsupported: true,
    },
    TargetSpec {
        target_id: TargetId::M365Copilot,
        skill: SkillProjection::CoworkOneDrive,
        unknown_dialect_allows_shared_skills: false,
        mcp: McpMapping::Unsupported,
        sse_unsupported: false,
    },
];

struct BuiltInAdapter {
    spec: TargetSpec,
}

impl TargetAdapter for BuiltInAdapter {
    fn target_id(&self) -> TargetId {
        self.spec.target_id
    }

    fn plan(
        &self,
        component: &CatalogComponent,
        profile: &AgentProfile,
        context: &PlanningContext<'_>,
    ) -> Result<TargetPlan, String> {
        if profile.target_id != self.spec.target_id {
            return Err("An adapter received a profile for a different target.".to_string());
        }
        if profile.dialect_id != self.spec.target_id.current_dialect() {
            let shared_skill_is_stable = component.kind == CatalogComponentKind::Skill
                && self.spec.unknown_dialect_allows_shared_skills;
            if !shared_skill_is_stable {
                return Ok(TargetPlan::blocked(
                    format!(
                        "Dialect {} is not recognized by the built-in {} adapter.",
                        profile.dialect_id,
                        self.spec.target_id.display_name()
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
}

impl BuiltInAdapter {
    fn plan_skill(
        &self,
        component: &CatalogComponent,
        context: &PlanningContext<'_>,
    ) -> Result<TargetPlan, String> {
        let home = &context.paths.home;
        let mut warnings = Vec::new();
        let (root, capability) = match self.spec.skill {
            SkillProjection::NativeClaude => {
                (home.join(".claude/skills"), CapabilityResult::Native)
            }
            SkillProjection::SharedAgents => (
                home.join(".agents/skills"),
                CapabilityResult::LosslessTranslation,
            ),
            SkillProjection::ClaudeAccountUpload => {
                return Ok(TargetPlan::unsupported(
                    "Claude Desktop loads skills from your claude.ai account, under Customize > Skills.",
                ));
            }
            SkillProjection::CoworkOneDrive => {
                let Some(onedrive) = &context.paths.onedrive_commercial else {
                    return Ok(TargetPlan::unsupported(
                        "OneDrive for work or school was not found, so Microsoft 365 Copilot has nowhere to read skills from.",
                    ));
                };
                if let Some(reason) =
                    cowork_limit_violation(&context.source_root.join(&component.source))?
                {
                    return Ok(TargetPlan::unsupported(reason));
                }
                warnings.push(
                    "Microsoft 365 Copilot sees a skill after OneDrive finishes syncing and a new Cowork conversation starts."
                        .to_string(),
                );
                (
                    onedrive.join(COWORK_RELATIVE).join("skills"),
                    CapabilityResult::LosslessTranslation,
                )
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
                },
            })],
            warnings,
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
        let Some((document_path, format, key_root)) = self.spec.mcp_document(context.paths) else {
            return Ok(TargetPlan::unsupported(format!(
                "{} has no local MCP configuration file.",
                self.spec.target_id.display_name()
            )));
        };
        if matches!(server, McpServer::Sse { .. }) && self.spec.sse_unsupported {
            return Ok(TargetPlan::unsupported(
                "This target dialect does not expose a distinct legacy SSE transport.",
            ));
        }
        if matches!(self.spec.mcp, McpMapping::ClaudeDesktop)
            && !matches!(server, McpServer::Stdio { .. })
        {
            return Ok(TargetPlan::unsupported(
                "Claude Desktop reads only local command servers from its configuration file. Add remote servers in Claude Desktop under Settings > Connectors.",
            ));
        }
        let value = self.spec.mcp_value(server)?;
        Ok(TargetPlan {
            capability: CapabilityResult::LosslessTranslation,
            resources: vec![DesiredResource::StructuredEntry(DesiredStructuredEntry {
                document_path,
                format,
                key_path: vec![key_root.to_string(), component.effective_name.clone()],
                value,
            })],
            warnings: vec![
                "This MCP server may start a local process or access a remote service when the target uses it."
                    .to_string(),
            ],
        })
    }
}

fn standard_mcp_value(server: &McpServer) -> Result<Value, String> {
    serde_json::to_value(server)
        .map_err(|error| format!("Could not serialize a portable MCP server: {error}"))
}

/// The portable shape without `type`, for files whose readers key transport
/// off the fields present.
fn untyped_mcp_value(server: &McpServer) -> Result<Value, String> {
    let value = standard_mcp_value(server)?;
    let mut object = value
        .as_object()
        .cloned()
        .ok_or_else(|| "A portable MCP server did not serialize as an object.".to_string())?;
    object.remove("type");
    Ok(Value::Object(object))
}

fn opencode_mcp_value(server: &McpServer) -> Value {
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
                            .cloned()
                            .map(Value::String)
                            .collect(),
                    ),
                ),
                ("enabled".to_string(), Value::Bool(true)),
            ]);
            if !env.is_empty() {
                object.insert(
                    "environment".to_string(),
                    serde_json::to_value(env).unwrap_or_else(|_| json!({})),
                );
            }
            if let Some(cwd) = cwd {
                object.insert("cwd".to_string(), Value::String(cwd.clone()));
            }
            Value::Object(object)
        }
        McpServer::StreamableHttp { url, headers } | McpServer::Sse { url, headers } => {
            let mut object = Map::from_iter([
                ("type".to_string(), Value::String("remote".to_string())),
                ("url".to_string(), Value::String(url.clone())),
                ("enabled".to_string(), Value::Bool(true)),
            ]);
            if !headers.is_empty() {
                object.insert(
                    "headers".to_string(),
                    serde_json::to_value(headers).unwrap_or_else(|_| json!({})),
                );
            }
            Value::Object(object)
        }
    }
}

static ADAPTERS: [BuiltInAdapter; 9] = [
    BuiltInAdapter { spec: SPECS[0] },
    BuiltInAdapter { spec: SPECS[1] },
    BuiltInAdapter { spec: SPECS[2] },
    BuiltInAdapter { spec: SPECS[3] },
    BuiltInAdapter { spec: SPECS[4] },
    BuiltInAdapter { spec: SPECS[5] },
    BuiltInAdapter { spec: SPECS[6] },
    BuiltInAdapter { spec: SPECS[7] },
    BuiltInAdapter { spec: SPECS[8] },
];

pub(crate) fn adapter(target_id: TargetId) -> &'static dyn TargetAdapter {
    ADAPTERS
        .iter()
        .find(|adapter| adapter.spec.target_id == target_id)
        .expect("every stable target has a built-in adapter")
}

fn spec(target_id: TargetId) -> TargetSpec {
    SPECS
        .into_iter()
        .find(|candidate| candidate.target_id == target_id)
        .expect("every stable target has a built-in spec")
}

pub(crate) fn reads_shared_agents(target_id: TargetId) -> bool {
    spec(target_id).reads_shared_agents()
}

pub(crate) fn skill_display_root(target_id: TargetId) -> &'static str {
    spec(target_id).skill_display_root()
}

pub(crate) fn managed_skill_roots(paths: &SystemPaths) -> Vec<PathBuf> {
    let home = &paths.home;
    let mut roots = vec![
        home.join(".agents/skills"),
        home.join(".claude/skills"),
        home.join(".cursor/skills"),
        home.join(".copilot/skills"),
        home.join(".grok/skills"),
        home.join(".config/opencode/skills"),
    ];
    if let Some(onedrive) = &paths.onedrive_commercial {
        roots.push(onedrive.join(COWORK_RELATIVE).join("skills"));
    }
    roots.sort();
    roots.dedup();
    roots
}

const COWORK_SKILL_FILE_LIMIT: u64 = 1024 * 1024;
const COWORK_COMPANION_LIMIT: usize = 20;
const COWORK_SKILL_BYTES_LIMIT: u64 = 10 * 1024 * 1024;

/// Microsoft 365 Copilot Cowork ignores skills over its published limits, so
/// the plan says so instead of reporting an install nothing can see.
fn cowork_limit_violation(skill_dir: &Path) -> Result<Option<String>, String> {
    let mut skill_file = 0;
    let mut companions = 0;
    let mut total = 0;
    let mut stack = vec![skill_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|error| format!("Could not read {}: {error}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let bytes = entry.metadata().map_err(|error| error.to_string())?.len();
            total += bytes;
            if dir == skill_dir && entry.file_name() == "SKILL.md" {
                skill_file = bytes;
            } else {
                companions += 1;
            }
        }
    }
    Ok(if skill_file > COWORK_SKILL_FILE_LIMIT {
        Some(format!(
            "Microsoft 365 Copilot limits SKILL.md to 1 MB; this one is {} KB.",
            skill_file / 1024
        ))
    } else if companions > COWORK_COMPANION_LIMIT {
        Some(format!(
            "Microsoft 365 Copilot allows at most {COWORK_COMPANION_LIMIT} files besides SKILL.md; this skill has {companions}."
        ))
    } else if total > COWORK_SKILL_BYTES_LIMIT {
        Some(format!(
            "Microsoft 365 Copilot allows 10 MB per skill; this one is {} MB.",
            total / (1024 * 1024)
        ))
    } else {
        None
    })
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
            onedrive_commercial: Some(root.join("onedrive")),
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
            (
                TargetId::M365Copilot,
                root.path()
                    .join("onedrive/Documents/Cowork/skills/acme-review"),
            ),
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

    #[test]
    fn m365_copilot_needs_onedrive_and_has_no_mcp() {
        let root = tempfile::tempdir().expect("root");
        write_skill_source(root.path());
        let mut paths = paths(root.path());
        let mcp = plan_for(
            TargetId::M365Copilot,
            &stdio_component(),
            &paths,
            root.path(),
        );
        assert!(matches!(
            mcp.capability,
            CapabilityResult::Unsupported { .. }
        ));
        assert!(mcp.resources.is_empty());
        paths.onedrive_commercial = None;
        let skill = plan_for(
            TargetId::M365Copilot,
            &skill_component(),
            &paths,
            root.path(),
        );
        assert!(matches!(
            skill.capability,
            CapabilityResult::Unsupported { .. }
        ));
        assert!(skill.resources.is_empty());
    }

    #[test]
    fn m365_copilot_rejects_skills_over_microsoft_limits() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let skill_dir = write_skill_source(root.path());
        fs::write(skill_dir.join("SKILL.md"), vec![b'a'; 1024 * 1024 + 1]).expect("big skill");
        let plan = plan_for(
            TargetId::M365Copilot,
            &skill_component(),
            &paths,
            root.path(),
        );
        match plan.capability {
            CapabilityResult::Unsupported { reason } => assert!(reason.contains("1 MB")),
            other => panic!("expected unsupported, got {other:?}"),
        }
        write_skill_source(root.path());
        for index in 0..=COWORK_COMPANION_LIMIT {
            fs::write(skill_dir.join(format!("note-{index}.md")), b"x").expect("companion");
        }
        let plan = plan_for(
            TargetId::M365Copilot,
            &skill_component(),
            &paths,
            root.path(),
        );
        match plan.capability {
            CapabilityResult::Unsupported { reason } => assert!(reason.contains("20")),
            other => panic!("expected unsupported, got {other:?}"),
        }
    }
}
