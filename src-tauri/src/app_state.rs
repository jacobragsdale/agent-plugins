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
    /// Marketplace index metadata, when the package is listed there.
    pub(crate) marketplace: Option<MarketplaceMeta>,
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
    pub(crate) agent_profiles: Vec<AgentProfileState>,
    pub(crate) marketplace_url: Option<String>,
    /// Where a person downloads a newer client, when the build configures one.
    pub(crate) download_url: Option<String>,
    pub(crate) identity: Option<MarketplaceIdentity>,
    pub(crate) preflight: Option<crate::preflight::PreflightReport>,
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
    Updated {
        state: Box<AppState>,
    },
    /// `message` is the raw text, kept for older windows; `error` is typed.
    Failed {
        message: String,
        error: crate::ipc_error::IpcError,
    },
}

impl ScheduledSync {
    pub(crate) fn from_result(result: Result<AppState, String>) -> Self {
        match result {
            Ok(state) => Self::Updated {
                state: Box::new(state),
            },
            Err(message) => Self::Failed {
                error: message.clone().into(),
                message,
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
                target_id: text("m365-copilot"),
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
                }],
                failed_items: vec![ItemFailure {
                    id: text("team-data/sql-helper"),
                    message: text("Could not replace C:\\Users\\sam\\.claude\\skills\\sql-helper: another app is using it. Close that app, then try again."),
                }],
                repaired_items: vec![text("Meeting notes")],
                extended_items: vec![text("Publish")],
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
                    TargetId::M365Copilot,
                    "Microsoft 365 Copilot",
                    "m365",
                    false,
                    "C:\\Users\\sam\\AppData\\Local\\AgentPlugins\\m365",
                    false,
                ),
            ],
            marketplace_url: Some(text("https://marketplace.ragsdale.dev")),
            download_url: None,
            identity: Some(MarketplaceIdentity {
                account: text("CORP\\sam"),
                namespace: text("sam"),
                display_name: text("Sam Lee"),
                admin: false,
                auth_mode: text("Windows sign-in (Kerberos)"),
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
                }],
                compatibility: Vec::new(),
                destination: None,
                status: ItemStatus::Available,
                requires_approval: false,
                risk_details: Vec::new(),
                marketplace: None,
            }],
            agent_profiles: Vec::new(),
            marketplace_url: None,
            download_url: None,
            identity: None,
            preflight: None,
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
