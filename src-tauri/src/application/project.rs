use super::{LoadedRepository, LoadedSource};
use crate::agent_profiles;
use crate::app_state::{
    AppState, AutoUpdateReport, CatalogItemState, ComponentState, Connectivity, ListedSourceState,
    RepositoryState, SourceState, SourceStatus,
};
use crate::catalog::{CatalogComponentKind, CatalogItem};
use crate::ledger::{self, InstallationRecord};
use crate::locator::Locator;
use crate::paths::SystemPaths;
use crate::source::{self, ConfiguredSource, SourceSnapshot};
use crate::sources::{cache_base_dir, config_base_dir};
use std::collections::BTreeSet;

pub(super) fn build_app_state(
    paths: &SystemPaths,
    repositories: &[LoadedRepository],
    loaded: &[LoadedSource],
    checked: u64,
    report: AutoUpdateReport,
    catalog_message: Option<String>,
) -> Result<AppState, String> {
    let ledger_state = crate::executor::read_ledger(paths)?;
    let profiles = crate::planner::current_profiles(paths);
    let mut current_ids = BTreeSet::new();
    let mut items = Vec::new();
    let mut sources = Vec::new();
    let configured_urls = loaded
        .iter()
        .map(|source| source.definition.locator.url().to_string())
        .collect::<BTreeSet<_>>();
    let mut repository_states = repositories
        .iter()
        .map(|repository| repository_state(repository, &configured_urls, checked))
        .collect::<Vec<_>>();
    for loaded_source in loaded {
        let catalog_errors = loaded_source
            .snapshot
            .as_ref()
            .map_or_else(Vec::new, |snapshot| snapshot.catalog.errors.clone());
        sources.push(SourceState {
            source_id: loaded_source.definition.source_id.clone(),
            source_key: loaded_source.definition.source_key.clone(),
            name: loaded_source.definition.name.clone(),
            description: loaded_source.definition.description.clone(),
            url: loaded_source.definition.url().to_string(),
            repository_key: loaded_source.definition.repository_key.clone(),
            status: loaded_source.status,
            refresh_failed: loaded_source.refresh_failed,
            message: loaded_source.message.clone(),
            commit: loaded_source
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.commit.clone()),
            checked_at_epoch_seconds: checked,
            last_success_at_epoch_seconds: loaded_source.last_success_at,
            catalog_errors,
        });
        if let Some(snapshot) = &loaded_source.snapshot {
            for item in snapshot.catalog.items.values() {
                current_ids.insert(item.id.clone());
                items.push(current_item_state(
                    paths,
                    &ledger_state,
                    &profiles,
                    &loaded_source.definition,
                    snapshot,
                    item,
                ));
            }
        }
    }
    for (id, record) in &ledger_state.items {
        if current_ids.contains(id) || super::sync::is_unsupported_legacy_install(record) {
            continue;
        }
        let definition = record_source(loaded, record);
        items.push(removed_item_state(
            paths,
            &ledger_state,
            &definition,
            id,
            record,
        ));
    }
    items.sort_by(|left, right| left.id.cmp(&right.id));
    sources.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    repository_states.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.repository_id.cmp(&right.repository_id))
    });
    let agent_profiles = agent_profiles::states(paths)?;
    Ok(AppState {
        checked_at_epoch_seconds: checked,
        connectivity: Connectivity::Online,
        sync_in_progress: false,
        auto_update_report: report,
        catalog_message,
        repositories: repository_states,
        sources,
        items,
        bundles: Vec::new(),
        tutorial: crate::tutorial::offer(paths, &agent_profiles),
        agent_profiles,
        marketplace_url: None,
        download_url: crate::locator::download_url().map(str::to_string),
        identity: None,
        preflight: None,
        notifications: Vec::new(),
        log_path: Some(crate::startup::log_path(paths).display().to_string()),
    })
}

/// The source an installed package came from: the loaded one, or, when it is
/// no longer configured, one rebuilt from the ledger so the package can still
/// be removed.
pub(super) fn record_source(
    loaded: &[LoadedSource],
    record: &InstallationRecord,
) -> ConfiguredSource {
    loaded
        .iter()
        .find(|source| source.definition.source_key == record.source_key)
        .map(|source| source.definition.clone())
        .unwrap_or_else(|| ConfiguredSource {
            source_key: record.source_key.clone(),
            source_id: record.source_id.clone(),
            name: record.source_id.clone(),
            description: "This source is no longer configured.".to_string(),
            locator: Locator::parse(&record.source_url)
                .unwrap_or_else(|_| Locator::display_url(record.source_url.clone())),
            repository_key: None,
        })
}

const IDENTITY_CACHE_FILE: &str = "marketplace-identity.json";

/// Attaches marketplace index metadata to the items it lists, and its bundles.
pub(super) fn apply_index(state: &mut AppState, index: &crate::marketplace::Index) {
    drop_revoked(&mut state.items, &index.revoked);
    for item in &mut state.items {
        item.marketplace = index
            .package(&item.id)
            .map(crate::app_state::MarketplaceMeta::from_index);
    }
    state.bundles = index
        .bundles
        .iter()
        .map(crate::app_state::BundleState::from_index)
        .collect();
}

/// A revoked package leaves the catalog even when a saved copy of its source
/// still lists it. One still on this computer stays, so removing it can be
/// retried or done by hand.
fn drop_revoked(items: &mut Vec<crate::app_state::CatalogItemState>, revoked: &[String]) {
    items.retain(|item| {
        !revoked.contains(&item.id) || super::items::counts_as_installed(item.status)
    });
}

/// Stores what the marketplace said about the caller and returns the identity
/// to show. Only a 401 or 403 means the identity is gone; when the server
/// could not be asked, the cached identity stays.
pub(super) fn remember_identity(
    cache: &std::path::Path,
    report: &crate::preflight::PreflightReport,
    me: Option<crate::marketplace::Me>,
) -> Option<crate::app_state::MarketplaceIdentity> {
    let identity = me.map(|me| crate::app_state::MarketplaceIdentity {
        account: me.account,
        namespace: me.namespace,
        display_name: me.display_name,
        admin: me.admin,
        auth_mode: report.auth_mode.clone(),
        namespaces: me.namespaces,
        teams: me.teams,
        suggestions_waiting: me.suggestions_waiting,
        reports_waiting: me.reports_waiting,
        unread_notifications: me.unread_notifications,
    });
    let rejected = report.checks.iter().any(|check| {
        check.id == "auth.identity"
            && (check.detail.contains("HTTP 401") || check.detail.contains("HTTP 403"))
    });
    match identity {
        Some(identity) => {
            write_identity_cache(cache, Some(&identity));
            Some(identity)
        }
        None if rejected => {
            write_identity_cache(cache, None);
            None
        }
        None => read_identity_cache(cache),
    }
}

pub(super) fn write_identity_cache(
    cache: &std::path::Path,
    identity: Option<&crate::app_state::MarketplaceIdentity>,
) {
    let path = cache.join(IDENTITY_CACHE_FILE);
    match identity {
        Some(identity) => {
            if let Ok(json) = serde_json::to_vec(identity) {
                let _ = std::fs::create_dir_all(cache);
                let _ = crate::fs_retry::replace_file(&path, &json);
            }
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn read_identity_cache(cache: &std::path::Path) -> Option<crate::app_state::MarketplaceIdentity> {
    let bytes = std::fs::read(cache.join(IDENTITY_CACHE_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Fills the marketplace fields of a cached state from what the last sync stored.
pub(super) fn apply_cached_marketplace(state: &mut AppState, cache: &std::path::Path) {
    let Some(base_url) = crate::locator::marketplace_base_url() else {
        return;
    };
    state.marketplace_url = Some(base_url.to_string());
    if let Some(index) = crate::marketplace::read_cached_index(cache) {
        apply_index(state, &index);
    }
    state.identity = read_identity_cache(cache);
    state.preflight = crate::preflight::PreflightReport::read_cache(cache);
}

pub(super) fn repository_state(
    loaded: &LoadedRepository,
    configured_urls: &BTreeSet<String>,
    checked: u64,
) -> RepositoryState {
    let listed = loaded
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.manifest.canonical_sources().ok())
        .unwrap_or_default()
        .into_iter()
        .map(|source| {
            let already_added = source
                .locator()
                .is_ok_and(|locator| configured_urls.contains(locator.url()));
            ListedSourceState {
                name: source.name,
                description: source.description,
                url: source.url,
                source_id: source.source_id,
                already_added,
            }
        })
        .collect();
    RepositoryState {
        repository_id: loaded.definition.repository_id.clone(),
        repository_key: loaded.definition.repository_key.clone(),
        name: loaded.definition.name.clone(),
        description: loaded.definition.description.clone(),
        url: loaded.definition.url().to_string(),
        status: loaded.status,
        refresh_failed: loaded.refresh_failed,
        message: loaded.message.clone(),
        revision: loaded
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.revision.clone()),
        checked_at_epoch_seconds: checked,
        last_success_at_epoch_seconds: loaded.last_success_at,
        sources: listed,
    }
}

pub(super) fn current_item_state(
    paths: &SystemPaths,
    ledger_state: &ledger::InstallationLedger,
    profiles: &[agent_profiles::AgentProfile],
    source: &ConfiguredSource,
    snapshot: &SourceSnapshot,
    item: &CatalogItem,
) -> CatalogItemState {
    let plan = crate::planner::plan(paths, snapshot, item, Some(profiles), None).ok();
    let compatibility = plan
        .as_ref()
        .map(|plan| plan.compatibility.clone())
        .unwrap_or_default();
    let record = ledger_state.items.get(&item.id);
    let status = super::status::refined_item_status(
        paths,
        ledger_state,
        snapshot,
        item,
        plan.as_ref(),
        Some(profiles),
    );
    let approval = plan.as_ref().map_or_else(
        || (false, Vec::new()),
        |plan| {
            let preview = crate::planner::preview(item, plan);
            (
                crate::planner::needs_approval(item, plan, ledger_state),
                preview.risk_details,
            )
        },
    );
    let connectors = plan
        .as_ref()
        .map(|plan| connectors(item, plan, ledger_state, record))
        .unwrap_or_default();
    let choices = crate::choices::read_or_default(paths);
    let overrides = crate::invocation::read_or_default(paths);
    let manual = |component| crate::invocation::effective(&overrides, &item.id, component);
    let skills = item
        .components
        .iter()
        .filter(|component| component.kind == CatalogComponentKind::Skill)
        .collect::<Vec<_>>();
    CatalogItemState {
        id: item.id.clone(),
        local_id: item.local_id.clone(),
        source_id: source.source_id.clone(),
        source_key: source.source_key.clone(),
        source_name: source.name.clone(),
        source_url: source.url().to_string(),
        name: item.name.clone(),
        description: item.description.clone(),
        manual_invocation: !skills.is_empty() && skills.iter().all(|component| manual(component)),
        source: item.source.clone(),
        source_is_directory: item.source_is_directory,
        manifest_version: item.manifest_version,
        components: item
            .components
            .iter()
            .map(|component| ComponentState {
                id: component.id.clone(),
                kind: component_kind_label(component.kind).to_string(),
                description: component.description.clone(),
                manual_invocation: manual(component),
                status: super::status::component_status(
                    paths,
                    ledger_state,
                    snapshot,
                    item,
                    &component.id,
                    status,
                    Some(profiles),
                ),
                requires_approval: crate::planner::requires_approval(item, &[component])
                    && !plan.as_ref().is_some_and(|plan| {
                        crate::planner::mcp_entries_owned(plan, ledger_state, Some(&component.id))
                    }),
                excluded_apps: choices.excluded(&item.id, &component.id),
            })
            .collect(),
        compatibility,
        destination: record.and_then(|record| destination(paths, record)),
        status,
        requires_approval: approval.0,
        risk_details: approval.1,
        connectors,
        held: choices.held.contains(&item.id),
        marketplace: None,
    }
}

/// Each MCP server the package would install, as the approval prompt shows it.
fn connectors(
    item: &CatalogItem,
    plan: &crate::resource::OperationPlan,
    ledger_state: &ledger::InstallationLedger,
    record: Option<&InstallationRecord>,
) -> Vec<crate::app_state::ConnectorState> {
    item.components
        .iter()
        .filter_map(|component| Some((component, component.mcp_server.as_ref()?)))
        .map(|(component, server)| {
            let environment = server
                .environment_names()
                .into_iter()
                .filter(|name| crate::startup::is_connector_setting(name))
                .collect::<Vec<_>>();
            let apps = plan
                .compatibility
                .iter()
                .filter(|report| {
                    report.component_id == component.id && report.capability.is_supported()
                })
                .filter_map(|report| {
                    agent_profiles::TargetId::ALL
                        .into_iter()
                        .find(|target| target.as_str() == report.target_id)
                        .map(|target| target.display_name().to_string())
                })
                .collect();
            let installed = record.is_some_and(|record| {
                record.binding_ids.iter().any(|binding_id| {
                    ledger_state
                        .bindings
                        .get(binding_id)
                        .is_some_and(|binding| binding.component_id == component.id)
                })
            });
            let (summary, missing_program) = match server {
                crate::mcp::McpServer::Stdio { command, .. } => (
                    format!("Starts a program called {command} on this computer."),
                    crate::startup::find_program(command)
                        .is_none()
                        .then(|| program_to_install(command)),
                ),
                crate::mcp::McpServer::StreamableHttp { url, .. }
                | crate::mcp::McpServer::Sse { url, .. } => (
                    format!(
                        "Sends your requests to {}.",
                        url::Url::parse(url)
                            .ok()
                            .and_then(|url| url.host_str().map(str::to_string))
                            .unwrap_or_else(|| url.clone())
                    ),
                    None,
                ),
            };
            crate::app_state::ConnectorState {
                component_id: component.id.clone(),
                name: component.effective_name.clone(),
                summary,
                detail: crate::planner::risk_detail(&component.effective_name, server),
                missing_environment: environment
                    .iter()
                    .filter(|name| !crate::startup::environment_has(name))
                    .cloned()
                    .collect(),
                environment,
                missing_program,
                apps,
                changed: installed
                    && !crate::planner::mcp_entries_owned(plan, ledger_state, Some(&component.id)),
            }
        })
        .collect()
}

/// What a person installs to get `command`, named the way IT would know it.
fn program_to_install(command: &str) -> String {
    match command
        .to_ascii_lowercase()
        .trim_end_matches(".exe")
        .trim_end_matches(".cmd")
    {
        "npx" | "node" | "npm" => "Node.js".to_string(),
        "uvx" | "uv" => "uv".to_string(),
        "docker" => "Docker Desktop".to_string(),
        "python" | "python3" | "py" => "Python".to_string(),
        other => other.to_string(),
    }
}

/// Where a record says the install lives. One unresolvable record shows no
/// destination instead of failing the whole state.
fn destination(paths: &SystemPaths, record: &InstallationRecord) -> Option<String> {
    match paths.resolve_owned(&record.destination) {
        Ok(path) => Some(path.display().to_string()),
        Err(error) => {
            eprintln!(
                "Could not resolve where {} is installed: {error}",
                record.name
            );
            None
        }
    }
}

pub(super) fn removed_item_state(
    paths: &SystemPaths,
    ledger_state: &ledger::InstallationLedger,
    source: &ConfiguredSource,
    id: &str,
    record: &InstallationRecord,
) -> CatalogItemState {
    CatalogItemState {
        id: id.to_string(),
        local_id: record.local_id.clone(),
        source_id: record.source_id.clone(),
        source_key: record.source_key.clone(),
        source_name: source.name.clone(),
        source_url: record.source_url.clone(),
        name: record.name.clone(),
        description: format!(
            "{} This install is no longer published by its source.",
            record.description
        ),
        manual_invocation: record.disable_model_invocation,
        source: record.source.clone(),
        source_is_directory: false,
        manifest_version: record.manifest_version,
        components: vec![ComponentState {
            id: record.local_id.clone(),
            kind: record.component_kind.clone(),
            description: record.description.clone(),
            manual_invocation: record.disable_model_invocation,
            status: super::status::item_status(paths, ledger_state, None, id),
            requires_approval: false,
            excluded_apps: Vec::new(),
        }],
        compatibility: Vec::new(),
        destination: destination(paths, record),
        status: super::status::item_status(paths, ledger_state, None, id),
        // An uninstall never needs the Tier 3 approval.
        requires_approval: false,
        risk_details: Vec::new(),
        connectors: Vec::new(),
        held: false,
        marketplace: None,
    }
}

pub(super) fn component_kind_label(kind: CatalogComponentKind) -> &'static str {
    match kind {
        CatalogComponentKind::Skill => "skill",
        CatalogComponentKind::McpServer => "mcpServer",
    }
}

pub(super) fn cached_state_now() -> Result<AppState, String> {
    cached_state(
        &SystemPaths::from_system()?,
        &cache_base_dir()?,
        &config_base_dir()?,
    )
}

/// The state from saved copies alone, with what the last sync learned about
/// each source: its result and message, when it last succeeded, and whether
/// the servers were reachable.
pub(super) fn cached_state(
    paths: &SystemPaths,
    cache: &std::path::Path,
    config: &std::path::Path,
) -> Result<AppState, String> {
    let health = super::sync::read_health(cache);
    let checked = super::sync::read_last_sync(cache).unwrap_or(0);
    let config_file = source::read_sources_config(config)?;
    let last_sync = |key: &str, load_error: Option<&String>| {
        let entry = health.entries.get(key).cloned().unwrap_or_default();
        let (status, message) = match load_error {
            Some(message) => (SourceStatus::Error, Some(message.clone())),
            None => match entry.status {
                Some(status @ (SourceStatus::Stale | SourceStatus::Error)) => {
                    (status, entry.message)
                }
                _ => (SourceStatus::Cached, None),
            },
        };
        (status, message, entry.last_success_at)
    };
    let repositories = config_file
        .repositories
        .into_iter()
        .map(|definition| {
            let snapshot = source::load_current_repository(cache, &definition);
            let (status, message, last_success_at) =
                last_sync(&definition.repository_key, snapshot.as_ref().err());
            LoadedRepository {
                snapshot: snapshot.ok().flatten(),
                definition,
                status,
                refresh_failed: status != SourceStatus::Cached,
                message,
                last_success_at,
            }
        })
        .collect::<Vec<_>>();
    let loaded = config_file
        .sources
        .into_iter()
        .map(|definition| {
            let snapshot = source::load_current(cache, &definition);
            let (status, message, last_success_at) =
                last_sync(&definition.source_key, snapshot.as_ref().err());
            LoadedSource {
                snapshot: snapshot.ok().flatten(),
                definition,
                status,
                refresh_failed: status != SourceStatus::Cached,
                message,
                last_success_at,
            }
        })
        .collect::<Vec<_>>();
    let mut state = build_app_state(
        paths,
        &repositories,
        &loaded,
        checked,
        AutoUpdateReport::default(),
        None,
    )?;
    state.connectivity = health.connectivity;
    state.sync_in_progress = super::sync::sync_in_progress();
    apply_cached_marketplace(&mut state, cache);
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::read_manifest_catalog;
    use crate::source::TEST_SOURCE_KEY;
    use std::fs;
    use std::path::Path;

    fn paths(root: &Path) -> SystemPaths {
        SystemPaths {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            local_data: root.join("local-data"),
            cache: root.join("cache"),
        }
    }

    fn catalog_item(
        id: &str,
        status: crate::install::ItemStatus,
    ) -> crate::app_state::CatalogItemState {
        let (source_id, local_id) = id.split_once('/').expect("canonical id");
        crate::app_state::CatalogItemState {
            id: id.to_string(),
            local_id: local_id.to_string(),
            source_id: source_id.to_string(),
            source_key: "source-a1b2c3".to_string(),
            source_name: source_id.to_string(),
            source_url: format!("https://marketplace.test/api/sources/{source_id}/archive"),
            name: local_id.to_string(),
            description: local_id.to_string(),
            manual_invocation: false,
            source: format!("skills/{local_id}"),
            source_is_directory: true,
            manifest_version: 2,
            components: Vec::new(),
            compatibility: Vec::new(),
            destination: None,
            status,
            requires_approval: false,
            risk_details: Vec::new(),
            connectors: Vec::new(),
            held: false,
            marketplace: None,
        }
    }

    #[test]
    fn a_revoked_package_is_not_offered_from_a_saved_copy() {
        use crate::install::ItemStatus;
        let mut items = vec![
            catalog_item("bob/pulled", ItemStatus::Available),
            catalog_item("bob/stuck", ItemStatus::Installed),
            catalog_item("bob/kept", ItemStatus::Available),
        ];
        drop_revoked(
            &mut items,
            &["bob/pulled".to_string(), "bob/stuck".to_string()],
        );
        let ids = items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            ["bob/stuck", "bob/kept"],
            "installed stays so it can be removed"
        );
    }

    #[test]
    fn only_a_rejection_clears_the_cached_identity() {
        let cache = tempfile::tempdir().expect("cache");
        let identity = crate::app_state::MarketplaceIdentity {
            account: "CORP\\jacob".to_string(),
            namespace: "jacob".to_string(),
            display_name: "Jacob".to_string(),
            admin: false,
            auth_mode: "Negotiate".to_string(),
            namespaces: Vec::new(),
            teams: Vec::new(),
            suggestions_waiting: 0,
            reports_waiting: 0,
            unread_notifications: 0,
        };
        write_identity_cache(cache.path(), Some(&identity));
        let report = |detail: &str| crate::preflight::PreflightReport {
            started_at_epoch_seconds: 0,
            duration_millis: 0,
            blocked: false,
            auth_mode: "Negotiate".to_string(),
            checks: vec![crate::preflight::PreflightCheck {
                id: "auth.identity".to_string(),
                group: "auth".to_string(),
                title: "Marketplace sign-in".to_string(),
                status: crate::preflight::CheckStatus::Fail,
                detail: detail.to_string(),
                remediation: None,
                blocking: false,
                duration_millis: 0,
            }],
        };

        let offline = remember_identity(
            cache.path(),
            &report("Could not connect to https://marketplace.example.com/api/me"),
            None,
        );
        assert_eq!(
            offline.map(|identity| identity.namespace).as_deref(),
            Some("jacob")
        );
        let rejected = remember_identity(
            cache.path(),
            &report("The marketplace rejected this machine's identity (HTTP 401)."),
            None,
        );
        assert!(rejected.is_none());
        assert!(read_identity_cache(cache.path()).is_none());
    }

    /// The app asks before installing an MCP server, so the state has to say
    /// which packages need that approval and what they would run.
    #[test]
    fn item_state_reports_the_mcp_approval_and_what_it_runs() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, crate::agent_profiles::TargetId::Cursor, true)
            .expect("enable");
        let source_root = root.path().join("source");
        fs::create_dir_all(source_root.join("skills/review")).expect("skill");
        fs::write(
            source_root.join("skills/review/SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\nBody\n",
        )
        .expect("skill");
        fs::create_dir_all(source_root.join("mcp")).expect("mcp");
        fs::write(
            source_root.join("mcp/database.json"),
            r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json","mcpServers":{"database":{"type":"stdio","command":"node","args":["server.js"]}}}"#,
        )
        .expect("mcp");
        fs::write(
            source_root.join("agent-plugins.json"),
            r#"{
              "version":2,
              "source":{"id":"acme","name":"Acme","description":"Shared config."},
              "packages":[{
                "id":"tools",
                "components":[
                  {"kind":"skill","id":"review","path":"skills/review"},
                  {"kind":"mcpServer","id":"database","path":"mcp/database.json"}
                ]
              }]
            }"#,
        )
        .expect("manifest");
        let catalog = read_manifest_catalog(&source_root, TEST_SOURCE_KEY).expect("catalog");
        let item = catalog.items["tools"].clone();
        let mut source = ConfiguredSource::test_fixture(
            "acme",
            "https://nexus.example.com/repository/raw/sources/acme-latest.zip",
        );
        source.source_key = TEST_SOURCE_KEY.to_string();
        let snapshot = SourceSnapshot {
            definition: source.clone(),
            commit: "a".repeat(40),
            path: source_root,
            catalog,
        };
        let ledger_state = crate::executor::read_ledger(&paths).expect("ledger");

        let state = current_item_state(
            &paths,
            &ledger_state,
            &crate::agent_profiles::read(&paths),
            &source,
            &snapshot,
            &item,
        );

        assert!(state.requires_approval);
        assert!(
            state
                .risk_details
                .iter()
                .any(|detail| detail.contains("node")),
            "the approval prompt needs the command it would run: {:?}",
            state.risk_details
        );
        let component = |id: &str| {
            state
                .components
                .iter()
                .find(|component| component.id == id)
                .expect("component")
        };
        assert!(component("database").requires_approval);
        assert!(!component("review").requires_approval);
    }
}
