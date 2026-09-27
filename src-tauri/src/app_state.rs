//! Application and IPC DTOs. Commands serialize these; they do not own use-case logic.

use crate::agent_profiles::AgentProfileState;
use crate::catalog::CatalogError;
use crate::install::ItemStatus;
use crate::resource::CompatibilityReport;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum SourceStatus {
    Fresh,
    Cached,
    /// The last check could not reach the server; the saved copy is shown and
    /// the app retries on its own.
    Stale,
    Error,
}

/// How the last sync went, for the one offline banner.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Connectivity {
    /// Every server answered.
    #[default]
    Online,
    /// No server could be reached; everything shown is the saved copy.
    Offline,
    /// Some servers answered and some did not, or a source reported an error.
    Degraded,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogItemState {
    pub(crate) id: String,
    pub(crate) local_id: String,
    pub(crate) source_id: String,
    pub(crate) source_key: String,
    pub(crate) source_name: String,
    pub(crate) source_url: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) manual_invocation: bool,
    pub(crate) source: String,
    pub(crate) source_is_directory: bool,
    pub(crate) manifest_version: u8,
    pub(crate) components: Vec<ComponentState>,
    pub(crate) compatibility: Vec<CompatibilityReport>,
    pub(crate) destination: Option<String>,
    pub(crate) status: ItemStatus,
    /// True when installing this package runs an MCP server and needs the
    /// Tier 3 approval. The app asks before it installs.
    pub(crate) requires_approval: bool,
    /// One line per MCP server: what it runs, so a person can decide.
    pub(crate) risk_details: Vec<String>,
    /// What each MCP server does, in plain words, for the approval prompt.
    pub(crate) connectors: Vec<ConnectorState>,
    /// The package's content digest. An approval sends back the one its
    /// dialog showed, so a package that changed meanwhile is asked about again.
    pub(crate) digest: String,
    /// The person holds this package's updates.
    pub(crate) held: bool,
    /// Marketplace index metadata, when the package is listed there.
    pub(crate) marketplace: Option<MarketplaceMeta>,
}

/// One MCP server in a package, as a person deciding whether to allow it
/// needs to see it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectorState {
    pub(crate) component_id: String,
    /// The name the AI apps list it under.
    pub(crate) name: String,
    /// "Starts a program called uvx on this computer." or "Sends requests to
    /// mcp.example.com."
    pub(crate) summary: String,
    /// The exact command line or address, for people who read those.
    pub(crate) detail: String,
    /// Environment variables it reads, such as an API key.
    pub(crate) environment: Vec<String>,
    /// Those not set for this person yet.
    pub(crate) missing_environment: Vec<String>,
    /// What to install first when its program is not on this computer.
    pub(crate) missing_program: Option<String>,
    /// The apps that get it, by display name.
    pub(crate) apps: Vec<String>,
    /// It is installed and this version changes what it runs.
    pub(crate) changed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarketplaceMeta {
    pub(crate) publisher: String,
    pub(crate) publisher_account: String,
    pub(crate) version: String,
    pub(crate) lane: String,
    pub(crate) tags: Vec<String>,
    pub(crate) published_at: String,
    pub(crate) installs: u64,
    pub(crate) installed_base: u64,
    pub(crate) restricted: bool,
    pub(crate) shared_with_you: bool,
    /// What the live version changed.
    pub(crate) changelog: Option<String>,
    /// Whether an admin let everyone see its MCP server; `None` without one.
    pub(crate) mcp_approved: Option<bool>,
}

impl MarketplaceMeta {
    pub(crate) fn from_index(package: &crate::marketplace::IndexPackage) -> Self {
        Self {
            publisher: package.publisher.display_name.clone(),
            publisher_account: package.publisher.account.clone(),
            version: package.version.clone(),
            lane: package.lane.clone(),
            tags: package.tags.clone(),
            published_at: package.published_at.clone(),
            installs: package.installs,
            installed_base: package.installed_base,
            restricted: package.restricted,
            shared_with_you: package.shared_with_you,
            changelog: package.changelog.clone(),
            mcp_approved: package.mcp_approved,
        }
    }
}

/// A bundle from the marketplace index: packages that install together.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BundleState {
    pub(crate) id: String,
    pub(crate) namespace: String,
    pub(crate) bundle_id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    /// The publisher's display name.
    pub(crate) publisher: String,
    pub(crate) lane: String,
    /// Canonical ids of the members this person may see.
    pub(crate) members: Vec<String>,
    pub(crate) updated_at: String,
    pub(crate) restricted: bool,
    pub(crate) shared_with_you: bool,
}

impl BundleState {
    pub(crate) fn from_index(bundle: &crate::marketplace::IndexBundle) -> Self {
        Self {
            id: bundle.id.clone(),
            namespace: bundle.namespace.clone(),
            bundle_id: bundle.bundle_id.clone(),
            name: bundle.name.clone(),
            description: bundle.description.clone(),
            publisher: bundle.publisher.display_name.clone(),
            lane: bundle.lane.clone(),
            members: bundle.members.clone(),
            updated_at: bundle.updated_at.clone(),
            restricted: bundle.restricted,
            shared_with_you: bundle.shared_with_you,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MarketplaceIdentity {
    pub(crate) account: String,
    pub(crate) namespace: String,
    pub(crate) display_name: String,
    pub(crate) admin: bool,
    pub(crate) auth_mode: String,
    /// Every space this person may publish to.
    #[serde(default)]
    pub(crate) namespaces: Vec<String>,
    #[serde(default)]
    pub(crate) teams: Vec<crate::marketplace::TeamMembership>,
    #[serde(default)]
    pub(crate) suggestions_waiting: u64,
    /// Open reports and feedback on this person's packages.
    #[serde(default)]
    pub(crate) reports_waiting: u64,
    #[serde(default)]
    pub(crate) unread_notifications: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ComponentState {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) description: String,
    pub(crate) manual_invocation: bool,
    pub(crate) status: ItemStatus,
    pub(crate) requires_approval: bool,
    /// Target IDs the person kept this component out of.
    pub(crate) excluded_apps: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceState {
    pub(crate) source_id: String,
    pub(crate) source_key: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) url: String,
    pub(crate) repository_key: Option<String>,
    pub(crate) status: SourceStatus,
    pub(crate) refresh_failed: bool,
    pub(crate) message: Option<String>,
    pub(crate) commit: Option<String>,
    pub(crate) checked_at_epoch_seconds: u64,
    /// When a fetch of this source last succeeded; `None` if it never has.
    pub(crate) last_success_at_epoch_seconds: Option<u64>,
    pub(crate) catalog_errors: Vec<CatalogError>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemReference {
    pub(crate) id: String,
    pub(crate) source_id: String,
    pub(crate) local_id: String,
    /// For an update: the marketplace versions before and after.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) from_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) to_version: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemFailure {
    pub(crate) id: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutoUpdateReport {
    pub(crate) updated_items: Vec<ItemReference>,
    pub(crate) failed_items: Vec<ItemFailure>,
    /// Display names of installed packages whose missing files the sync put back.
    pub(crate) repaired_items: Vec<String>,
    /// Display names of installed packages the sync added to newly found agents.
    pub(crate) extended_items: Vec<String>,
    /// Display names of installed packages the sync took out of agents no
    /// longer found, or kept out.
    pub(crate) released_items: Vec<String>,
    /// A pulled package still in an app whose settings file was busy or
    /// damaged: why, one line each. The next sync tries again.
    pub(crate) still_pulled: Vec<String>,
    /// Display names of packages the sync uninstalled because their publisher
    /// or an admin pulled them from every PC.
    pub(crate) removed_items: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AppState {
    /// When a sync last reached the servers; 0 when none has yet.
    pub(crate) checked_at_epoch_seconds: u64,
    pub(crate) connectivity: Connectivity,
    /// A sync is running right now; a fresher state follows when it ends.
    pub(crate) sync_in_progress: bool,
    pub(crate) auto_update_report: AutoUpdateReport,
    pub(crate) catalog_message: Option<String>,
    pub(crate) repositories: Vec<RepositoryState>,
    pub(crate) sources: Vec<SourceState>,
    pub(crate) items: Vec<CatalogItemState>,
    pub(crate) bundles: Vec<BundleState>,
    pub(crate) agent_profiles: Vec<AgentProfileState>,
    /// The detected app the skill tutorial can demonstrate, until it has run.
    pub(crate) tutorial: Option<crate::agent_profiles::TargetId>,
    pub(crate) marketplace_url: Option<String>,
    /// Where a person downloads a newer client, when the build configures one.
    pub(crate) download_url: Option<String>,
    pub(crate) identity: Option<MarketplaceIdentity>,
    pub(crate) preflight: Option<crate::preflight::PreflightReport>,
    /// Marketplace news that arrived since the last sync.
    pub(crate) notifications: Vec<crate::marketplace::Notification>,
    /// Where the app writes its log, for "Open logs".
    pub(crate) log_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListedSourceState {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) url: String,
    pub(crate) source_id: Option<String>,
    pub(crate) already_added: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RepositoryState {
    pub(crate) repository_id: String,
    pub(crate) repository_key: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) url: String,
    pub(crate) status: SourceStatus,
    pub(crate) refresh_failed: bool,
    pub(crate) message: Option<String>,
    pub(crate) revision: Option<String>,
    pub(crate) checked_at_epoch_seconds: u64,
    /// When a fetch of this catalog last succeeded; `None` if it never has.
    pub(crate) last_success_at_epoch_seconds: Option<u64>,
    pub(crate) sources: Vec<ListedSourceState>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedSource {
    pub(crate) token: String,
    pub(crate) source_id: String,
    pub(crate) source_key: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) url: String,
    pub(crate) commit: String,
    pub(crate) item_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkPlanEntry {
    pub(crate) id: String,
    pub(crate) local_id: String,
    pub(crate) status: ItemStatus,
    pub(crate) will_run: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BulkAction {
    Install,
    Replace,
    Uninstall,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkPlan {
    pub(crate) source_id: String,
    pub(crate) action: BulkAction,
    pub(crate) entries: Vec<BulkPlanEntry>,
}

/// What installing or removing a list of packages, from any sources, would do.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemsPlan {
    pub(crate) action: BulkAction,
    pub(crate) entries: Vec<BulkPlanEntry>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkFailure {
    pub(crate) id: String,
    pub(crate) message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkResult {
    pub(crate) completed: Vec<String>,
    pub(crate) failures: Vec<BulkFailure>,
    pub(crate) backup_paths: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum ScheduledSync {
    Updated { state: Box<AppState> },
    Failed { error: crate::ipc_error::IpcError },
}

impl ScheduledSync {
    pub(crate) fn from_result(result: Result<AppState, String>) -> Self {
        match result {
            Ok(state) => Self::Updated {
                state: Box::new(state),
            },
            Err(message) => Self::Failed {
                error: message.into(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../src/ipc/fixtures/app-state.json"
    );
    const OFFICIAL_URL: &str = "https://marketplace.ragsdale.dev/api/sources/official/archive";
    const DATA_URL: &str = "https://marketplace.ragsdale.dev/api/sources/team-data/archive";

    fn text(value: &str) -> String {
        value.to_string()
    }

    fn component(id: &str, kind: &str, status: ItemStatus, approval: bool) -> ComponentState {
        ComponentState {
            id: text(id),
            kind: text(kind),
            description: format!("The {id} {kind}."),
            manual_invocation: false,
            status,
            requires_approval: approval,
            excluded_apps: Vec::new(),
        }
    }

    /// A single-skill package; callers adjust what differs.
    fn item(source_id: &str, local_id: &str, name: &str, status: ItemStatus) -> CatalogItemState {
        let (source_key, source_name, source_url) = if source_id == "official" {
            ("source-a1b2c3", "Official", OFFICIAL_URL)
        } else {
            ("source-d4e5f6", "Data team", DATA_URL)
        };
        let installed = !matches!(status, ItemStatus::Available | ItemStatus::Removed);
        CatalogItemState {
            id: format!("{source_id}/{local_id}"),
            local_id: text(local_id),
            source_id: text(source_id),
            source_key: text(source_key),
            source_name: text(source_name),
            source_url: text(source_url),
            name: text(name),
            description: format!("{name} for the whole company."),
            manual_invocation: false,
            source: format!("skills/{local_id}"),
            source_is_directory: true,
            manifest_version: 2,
            components: vec![component(local_id, "skill", status, false)],
            compatibility: Vec::new(),
            destination: installed.then(|| format!("C:\\Users\\sam\\.claude\\skills\\{local_id}")),
            status,
            requires_approval: false,
            risk_details: Vec::new(),
            connectors: Vec::new(),
            digest: String::new(),
            held: false,
            marketplace: None,
        }
    }

    fn profile(
        target: crate::agent_profiles::TargetId,
        name: &str,
        dialect: &str,
        detected: bool,
        directory: &str,
        shared: bool,
    ) -> AgentProfileState {
        AgentProfileState {
            target_id: target,
            display_name: text(name),
            enabled: true,
            scopes: vec![text("user")],
            dialect_id: text(dialect),
            detected,
            detected_version: detected.then(|| text("1.2.3")),
            detection_message: None,
            verification_guidance: format!("Open {name} and look for the new skills."),
            reload_guidance: format!("Quit {name} and open it again."),
            skill_directory: text(directory),
            skill_directory_shared: shared,
        }
    }

    /// A realistic offline state: every status the window draws, a stale and
    /// an errored source, and the auto-update report of the last good sync.
    fn fixture_state() -> AppState {
        use crate::agent_profiles::TargetId;
        use crate::preflight::{CheckStatus, PreflightCheck, PreflightReport, Remediation};
        use crate::resource::CapabilityResult;

        let mut publish = item("official", "publish", "Publish", ItemStatus::Installed);
        publish.manual_invocation = true;
        publish.components[0].manual_invocation = true;
        publish.compatibility = vec![
            CompatibilityReport {
                component_id: text("publish"),
                target_id: text("claude-desktop"),
                capability: CapabilityResult::Native,
            },
            CompatibilityReport {
                component_id: text("publish"),
                target_id: text("pi"),
                capability: CapabilityResult::LossyTranslation {
                    losses: vec![text("Scripts are not supported.")],
                },
            },
        ];
        publish.marketplace = Some(MarketplaceMeta {
            publisher: text("Platform team"),
            publisher_account: text("CORP\\platform"),
            version: text("1.4.0"),
            lane: text("official"),
            tags: vec![text("marketplace")],
            published_at: text("2026-09-01T12:00:00Z"),
            installs: 120,
            installed_base: 87,
            restricted: false,
            shared_with_you: false,
            changelog: Some(text("Publishes skill packs from a folder.")),
            mcp_approved: None,
        });

        let mut sql = item(
            "team-data",
            "sql-helper",
            "SQL helper",
            ItemStatus::PartiallyInstalled,
        );
        sql.source = text("packages/sql-helper");
        sql.components = vec![
            component("sql-style", "skill", ItemStatus::Installed, false),
            component("warehouse", "mcpServer", ItemStatus::Available, true),
        ];
        sql.compatibility = vec![CompatibilityReport {
            component_id: text("warehouse"),
            target_id: text("chatgpt"),
            capability: CapabilityResult::Unsupported {
                reason: text("ChatGPT does not load local servers."),
            },
        }];
        sql.requires_approval = true;
        sql.risk_details = vec![text("warehouse runs: uvx warehouse-mcp --read-only")];

        let items = vec![
            publish,
            sql,
            item(
                "team-data",
                "chart-style",
                "Chart style",
                ItemStatus::Conflict,
            ),
            item("official", "review", "Review", ItemStatus::Available),
            item(
                "official",
                "release-notes",
                "Release notes",
                ItemStatus::UpdateAvailable,
            ),
            item(
                "official",
                "meeting-notes",
                "Meeting notes",
                ItemStatus::Missing,
            ),
            item("official", "tone", "Tone of voice", ItemStatus::Modified),
            item(
                "official",
                "legacy-style",
                "Legacy style",
                ItemStatus::Removed,
            ),
            item(
                "team-data",
                "dashboards",
                "Dashboards",
                ItemStatus::SourceConflict,
            ),
        ];

        AppState {
            checked_at_epoch_seconds: 1_790_000_000,
            connectivity: Connectivity::Offline,
            sync_in_progress: false,
            auto_update_report: AutoUpdateReport {
                updated_items: vec![ItemReference {
                    id: text("official/publish"),
                    source_id: text("official"),
                    local_id: text("publish"),
                    from_version: Some(text("1.3.2")),
                    to_version: Some(text("1.4.0")),
                }],
                failed_items: vec![ItemFailure {
                    id: text("team-data/sql-helper"),
                    message: text("Could not replace C:\\Users\\sam\\.claude\\skills\\sql-helper: another app is using it. Close that app, then try again."),
                }],
                repaired_items: vec![text("Meeting notes")],
                extended_items: vec![text("Publish")],
                released_items: Vec::new(),
                still_pulled: Vec::new(),
                removed_items: Vec::new(),
            },
            catalog_message: None,
            repositories: vec![RepositoryState {
                repository_id: text("company"),
                repository_key: text("repo-3f2a9c"),
                name: text("Company catalog"),
                description: text("Sources approved for this company."),
                url: text("https://marketplace.ragsdale.dev/api/catalog"),
                status: SourceStatus::Stale,
                refresh_failed: true,
                message: Some(text("Could not connect to https://marketplace.ragsdale.dev/api/catalog: dns error")),
                revision: Some(text("rev-42")),
                checked_at_epoch_seconds: 1_790_000_000,
                last_success_at_epoch_seconds: Some(1_789_990_000),
                sources: vec![
                    ListedSourceState {
                        name: text("Official"),
                        description: text("Packages published by the platform team."),
                        url: text(OFFICIAL_URL),
                        source_id: Some(text("official")),
                        already_added: true,
                    },
                    ListedSourceState {
                        name: text("Data team"),
                        description: text("Helpers for analysts."),
                        url: text(DATA_URL),
                        source_id: Some(text("team-data")),
                        already_added: true,
                    },
                    ListedSourceState {
                        name: text("Legal"),
                        description: text("Contract review skills."),
                        url: text("https://marketplace.ragsdale.dev/api/sources/legal/archive"),
                        source_id: None,
                        already_added: false,
                    },
                ],
            }],
            sources: vec![
                SourceState {
                    source_id: text("official"),
                    source_key: text("source-a1b2c3"),
                    name: text("Official"),
                    description: text("Packages published by the platform team."),
                    url: text(OFFICIAL_URL),
                    repository_key: Some(text("repo-3f2a9c")),
                    status: SourceStatus::Stale,
                    refresh_failed: true,
                    message: Some(format!("Could not connect to {OFFICIAL_URL}: dns error")),
                    commit: Some(text("9f8e7d6c")),
                    checked_at_epoch_seconds: 1_790_000_000,
                    last_success_at_epoch_seconds: Some(1_789_990_000),
                    catalog_errors: Vec::new(),
                },
                SourceState {
                    source_id: text("team-data"),
                    source_key: text("source-d4e5f6"),
                    name: text("Data team"),
                    description: text("Helpers for analysts."),
                    url: text(DATA_URL),
                    repository_key: Some(text("repo-3f2a9c")),
                    status: SourceStatus::Error,
                    refresh_failed: true,
                    message: Some(text("The latest Data team download is not a valid source. Showing the last good copy.")),
                    commit: Some(text("1a2b3c4d")),
                    checked_at_epoch_seconds: 1_789_990_000,
                    last_success_at_epoch_seconds: None,
                    catalog_errors: vec![CatalogError {
                        path: text("skills/broken/SKILL.md"),
                        message: text("missing required field `description`"),
                    }],
                },
            ],
            items,
            bundles: vec![BundleState {
                id: text("team-data/starter"),
                namespace: text("team-data"),
                bundle_id: text("starter"),
                name: text("Data starter kit"),
                description: text("Everything a new analyst needs."),
                publisher: text("Data team"),
                lane: text("team"),
                members: vec![text("team-data/sql-helper"), text("team-data/chart-style")],
                updated_at: text("2026-09-20T12:00:00Z"),
                restricted: false,
                shared_with_you: false,
            }],
            agent_profiles: vec![
                profile(
                    TargetId::ClaudeDesktop,
                    "Claude Desktop",
                    "claude",
                    true,
                    "C:\\Users\\sam\\.claude\\skills",
                    true,
                ),
                profile(
                    TargetId::Pi,
                    "pi",
                    "pi",
                    false,
                    "~/.agents/skills",
                    true,
                ),
            ],
            tutorial: Some(TargetId::Cursor),
            marketplace_url: Some(text("https://marketplace.ragsdale.dev")),
            download_url: None,
            identity: Some(MarketplaceIdentity {
                account: text("CORP\\sam"),
                namespace: text("sam"),
                display_name: text("Sam Lee"),
                admin: false,
                auth_mode: text("Windows sign-in (Kerberos)"),
                namespaces: vec![text("sam"), text("team-data")],
                teams: vec![crate::marketplace::TeamMembership {
                    namespace: text("team-data"),
                    display_name: text("Data team"),
                    owner: true,
                }],
                suggestions_waiting: 0,
                reports_waiting: 0,
                unread_notifications: 0,
            }),
            preflight: Some(PreflightReport {
                started_at_epoch_seconds: 1_789_999_000,
                duration_millis: 1_840,
                blocked: false,
                auth_mode: text("Windows sign-in (Kerberos)"),
                checks: vec![
                    PreflightCheck {
                        id: text("server.health"),
                        group: text("server"),
                        title: text("Marketplace server"),
                        status: CheckStatus::Fail,
                        detail: text("Could not reach marketplace.ragsdale.dev."),
                        remediation: Some(Remediation::Manual {
                            text: text("Check your network connection."),
                        }),
                        blocking: true,
                        duration_millis: 120,
                    },
                    PreflightCheck {
                        id: text("dependencies.uv"),
                        group: text("dependencies"),
                        title: text("uv"),
                        status: CheckStatus::Ok,
                        detail: text("uv is installed."),
                        remediation: Some(Remediation::AutoFixed),
                        blocking: false,
                        duration_millis: 30,
                    },
                ],
            }),
            notifications: vec![crate::marketplace::Notification {
                id: 41,
                kind: text("suggestion.created"),
                text: text("Dana suggested a change to SQL helper."),
                link: Some(text("/suggestions/12")),
                read: false,
            }],
            log_path: Some(text("C:\\Users\\sam\\AppData\\Local\\agent-plugins\\agent-plugins.log")),
        }
    }

    /// The window's contract test parses this file. Regenerate with
    /// `UPDATE_FIXTURES=1 cargo test --manifest-path src-tauri/Cargo.toml app_state_fixture`,
    /// then `pnpm exec prettier --write src/ipc/fixtures/app-state.json`; the
    /// comparison is by value, so Prettier's layout does not matter.
    #[test]
    fn app_state_fixture_matches_the_checked_in_contract() {
        let state = fixture_state();
        if std::env::var_os("UPDATE_FIXTURES").is_some_and(|value| value == "1") {
            // From the struct, not a `Value`, so fields keep their declared order.
            let mut text = serde_json::to_string_pretty(&state).expect("json");
            text.push('\n');
            std::fs::write(FIXTURE, text).expect("write fixture");
            return;
        }
        let generated = serde_json::to_value(&state).expect("json");
        let checked_in: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(FIXTURE).expect("read fixture"))
                .expect("parse fixture");
        assert!(
            generated == checked_in,
            "{FIXTURE} is out of date; rerun this test with UPDATE_FIXTURES=1"
        );
    }

    #[test]
    fn app_state_json_includes_catalog_fields() {
        let state = AppState {
            checked_at_epoch_seconds: 1,
            connectivity: Connectivity::Offline,
            sync_in_progress: true,
            auto_update_report: AutoUpdateReport::default(),
            catalog_message: None,
            repositories: Vec::new(),
            sources: vec![SourceState {
                source_id: "review".to_string(),
                source_key: "source-test".to_string(),
                name: "Review".to_string(),
                description: "Skills".to_string(),
                url: "https://nexus.example.com/repository/raw/sources/review-latest.zip"
                    .to_string(),
                repository_key: None,
                status: SourceStatus::Cached,
                refresh_failed: false,
                message: None,
                commit: None,
                checked_at_epoch_seconds: 1,
                last_success_at_epoch_seconds: Some(1),
                catalog_errors: Vec::new(),
            }],
            bundles: Vec::new(),
            items: vec![CatalogItemState {
                id: "review/python-standards".to_string(),
                local_id: "python-standards".to_string(),
                source_id: "review".to_string(),
                source_key: "source-test".to_string(),
                source_name: "Review".to_string(),
                source_url: "https://nexus.example.com/repository/raw/sources/review-latest.zip"
                    .to_string(),
                name: "Python standards".to_string(),
                description: "Python".to_string(),
                manual_invocation: false,
                source: "skills/python-standards".to_string(),
                source_is_directory: true,
                manifest_version: 2,
                components: vec![ComponentState {
                    id: "python-standards".to_string(),
                    kind: "skill".to_string(),
                    description: "Python".to_string(),
                    manual_invocation: false,
                    status: ItemStatus::Available,
                    requires_approval: false,
                    excluded_apps: Vec::new(),
                }],
                compatibility: Vec::new(),
                destination: None,
                status: ItemStatus::Available,
                requires_approval: false,
                risk_details: Vec::new(),
                connectors: Vec::new(),
                digest: String::new(),
                held: false,
                marketplace: None,
            }],
            agent_profiles: Vec::new(),
            tutorial: None,
            marketplace_url: None,
            download_url: None,
            identity: None,
            preflight: None,
            notifications: Vec::new(),
            log_path: None,
        };
        let value = serde_json::to_value(&state).expect("json");
        assert!(value
            .get("repositories")
            .and_then(serde_json::Value::as_array)
            .is_some());
        assert!(value["catalogMessage"].is_null());
        assert!(value["sources"][0]["repositoryKey"].is_null());
        assert!(value["sources"][0].get("locatorKind").is_none());
        assert!(value["items"][0].get("locatorKind").is_none());
        assert_eq!(value["items"][0]["components"][0]["status"], "available");
        assert_eq!(value["items"][0]["components"][0]["description"], "Python");
        assert_eq!(
            value["items"][0]["components"][0]["manualInvocation"],
            false
        );
        assert_eq!(value["items"][0]["status"], "available");
        assert_eq!(value["connectivity"], "offline");
        assert_eq!(value["syncInProgress"], true);
        assert_eq!(value["sources"][0]["lastSuccessAtEpochSeconds"], 1);
        assert!(value["autoUpdateReport"]["repairedItems"].is_array());
    }
}
