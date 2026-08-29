use super::{current_epoch_seconds, run_blocking, LoadedRepository, LoadedSource, RuntimeState};
use crate::agent_profiles;
use crate::app_state::{AppState, AutoUpdateReport, ItemFailure, ItemReference, SourceStatus};
use crate::install::{self, ItemStatus};
use crate::ledger::InstallationRecord;
use crate::locator::{default_catalog_locator, Locator};
use crate::paths::SystemPaths;
use crate::source::{self, ConfiguredRepository, ConfiguredSource, SourcesConfig};
use crate::sources::{cache_base_dir, config_base_dir};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) async fn load_cached_app_state(
    runtime: &RuntimeState,
) -> Result<Option<AppState>, String> {
    let _guard = runtime.operation_lock.lock().await;
    run_blocking("Cached source load", || {
        let paths = SystemPaths::from_system()?;
        let cache = cache_base_dir()?;
        let config = config_base_dir()?;
        cached_app_state(&paths, &cache, &config).map(Some)
    })
    .await
}

/// The state the window reloads after every operation. It carries what the
/// last sync learned - the identity, the preflight, and the marketplace
/// index - because dropping those would blank the header and the badges.
fn cached_app_state(
    paths: &SystemPaths,
    cache: &std::path::Path,
    config: &std::path::Path,
) -> Result<AppState, String> {
    retire_unsupported_legacy_installs(paths)?;
    agent_profiles::apply_detected_defaults(paths)?;
    let checked = read_last_sync(cache).unwrap_or_else(current_epoch_seconds);
    let config_file = source::read_sources_config(config)?;
    let repositories = config_file
        .repositories
        .into_iter()
        .map(
            |definition| match source::load_current_repository(cache, &definition) {
                Ok(snapshot) => LoadedRepository {
                    definition,
                    snapshot,
                    status: SourceStatus::Cached,
                    refresh_failed: false,
                    message: None,
                },
                Err(message) => LoadedRepository {
                    definition,
                    snapshot: None,
                    status: SourceStatus::Error,
                    refresh_failed: true,
                    message: Some(message),
                },
            },
        )
        .collect::<Vec<_>>();
    let loaded = config_file
        .sources
        .into_iter()
        .map(
            |definition| match source::load_current(cache, &definition) {
                Ok(snapshot) => LoadedSource {
                    definition,
                    snapshot,
                    status: SourceStatus::Cached,
                    refresh_failed: false,
                    message: None,
                },
                Err(message) => LoadedSource {
                    definition,
                    snapshot: None,
                    status: SourceStatus::Error,
                    refresh_failed: true,
                    message: Some(message),
                },
            },
        )
        .collect::<Vec<_>>();
    let mut state = super::project::build_app_state(
        paths,
        &repositories,
        &loaded,
        checked,
        AutoUpdateReport::default(),
        None,
    )?;
    super::project::apply_cached_marketplace(&mut state, cache);
    Ok(state)
}

/// Re-runs the preflight on demand against the cached state.
pub(crate) async fn run_preflight(
    runtime: &RuntimeState,
) -> Result<crate::preflight::PreflightReport, String> {
    let _sync_guard = runtime.sync_lock.lock().await;
    run_blocking("Preflight", || {
        agent_profiles::clear_detection_cache();
        let paths = SystemPaths::from_system()?;
        let cache = cache_base_dir()?;
        let state = super::project::cached_state_now()?;
        let ledger_error = crate::executor::read_ledger(&paths).err();
        let catalog_age = state
            .repositories
            .iter()
            .find(|repository| {
                crate::locator::default_catalog_locator()
                    .ok()
                    .flatten()
                    .is_some_and(|default| {
                        Locator::parse(&repository.url).ok().as_ref() == Some(&default)
                    })
            })
            .and_then(|repository| repository.revision.as_ref().map(|_| 0));
        let (report, findings) = crate::preflight::run(&crate::preflight::PreflightInput {
            paths: &paths,
            startup: crate::STARTUP_REPORT.get(),
            profiles: &state.agent_profiles,
            items: &state.items,
            catalog_age_seconds: catalog_age,
            ledger_error,
        });
        report.write_cache(&cache);
        let identity = findings
            .identity
            .map(|me| crate::app_state::MarketplaceIdentity {
                account: me.account,
                namespace: me.namespace,
                display_name: me.display_name,
                admin: me.admin,
                auth_mode: report.auth_mode.clone(),
            });
        super::project::write_identity_cache(&cache, identity.as_ref());
        Ok(report)
    })
    .await
}

pub(crate) async fn sync_app_state(runtime: &RuntimeState) -> Result<AppState, String> {
    let _sync_guard = runtime.sync_lock.lock().await;
    let _operation_guard = runtime.operation_lock.lock().await;
    run_blocking("Source synchronization", synchronize).await
}

pub(super) fn synchronize() -> Result<AppState, String> {
    // A sync is the app looking at the machine again, agents included.
    agent_profiles::clear_detection_cache();
    let paths = SystemPaths::from_system()?;
    let cache = cache_base_dir()?;
    let config = config_base_dir()?;
    let checked = current_epoch_seconds();
    let mut config_file = source::read_sources_config(&config)?;
    let mut catalog_message = ensure_default_catalog(&cache, &mut config_file.repositories);
    let installed_keys = crate::executor::read_ledger(&paths)
        .map(|ledger| {
            ledger
                .items
                .values()
                .map(|record| record.source_key.clone())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let (updated_repositories, loaded_repositories, retired_repositories) =
        refresh_repositories(&cache, config_file.repositories);
    if let Some(message) =
        subscribe_marketplace_sources(&cache, &loaded_repositories, &mut config_file.sources)
    {
        catalog_message = Some(match catalog_message {
            Some(existing) => format!("{existing} {message}"),
            None => message,
        });
    }
    let (updated_sources, loaded_sources, retired_sources) =
        refresh_sources(&cache, config_file.sources, &installed_keys);
    if let Some(message) = retired_message(&retired_repositories, &retired_sources) {
        catalog_message = Some(match catalog_message {
            Some(existing) => format!("{existing} {message}"),
            None => message,
        });
    }
    source::write_sources_config(
        &config,
        &SourcesConfig {
            repositories: updated_repositories,
            sources: updated_sources,
        },
    )?;
    retire_unsupported_legacy_installs(&paths)?;
    agent_profiles::apply_detected_defaults(&paths)?;
    let report = reconcile_installed_items(&paths, &loaded_sources)?;
    let updated_ids = report
        .updated_items
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let mut state = super::project::build_app_state(
        &paths,
        &loaded_repositories,
        &loaded_sources,
        checked,
        report,
        catalog_message,
    )?;
    enrich_with_marketplace(
        &paths,
        &cache,
        &loaded_repositories,
        &mut state,
        &updated_ids,
    );
    write_last_sync(&cache, checked);
    Ok(state)
}

const LAST_SYNC_FILE: &str = "last-sync.json";

/// The header says when the app last reached the sources. Cached loads happen
/// after every operation, so they read this instead of claiming "just now".
fn write_last_sync(cache: &std::path::Path, checked: u64) {
    let _ = std::fs::create_dir_all(cache);
    let _ = std::fs::write(cache.join(LAST_SYNC_FILE), checked.to_string());
}

fn read_last_sync(cache: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(cache.join(LAST_SYNC_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Adds every source the marketplace catalog lists that is not configured yet.
/// The catalog is the authority: browsing the marketplace needs no Manage
/// Sources step. Returns a message when some listed source could not be added.
pub(super) fn subscribe_marketplace_sources(
    cache: &std::path::Path,
    repositories: &[LoadedRepository],
    sources: &mut Vec<ConfiguredSource>,
) -> Option<String> {
    let default = crate::locator::default_catalog_locator().ok().flatten()?;
    let repository = repositories
        .iter()
        .find(|repository| repository.definition.locator.same_identity(&default))?;
    let snapshot = repository.snapshot.as_ref()?;
    let listed = snapshot.manifest.canonical_sources().ok()?;
    let mut problems = Vec::new();
    let mut added = false;
    for entry in listed {
        let Ok(locator) = entry.locator() else {
            continue;
        };
        let configured = sources.iter().any(|source| {
            source.locator.same_identity(&locator)
                || entry.source_id.as_deref() == Some(source.source_id.as_str())
        });
        if configured {
            continue;
        }
        match source::prepare_new_source(
            &locator,
            cache,
            Some(repository.definition.repository_key.clone()),
            entry.source_id.as_deref(),
        ) {
            Ok(candidate) => match source::activate_candidate(cache, candidate) {
                Ok(activated) => {
                    sources.push(activated.definition);
                    added = true;
                }
                Err(message) => problems.push(format!("{}: {message}", entry.name)),
            },
            Err(message) => problems.push(format!("{}: {message}", entry.name)),
        }
    }
    if added {
        sources.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.source_id.cmp(&right.source_id))
        });
    }
    (!problems.is_empty()).then(|| {
        format!(
            "Some marketplace sources could not be added: {}",
            problems.join(" ")
        )
    })
}

/// Joins the marketplace index, runs the preflight, and reports the heartbeat.
/// Every step is best-effort: an unreachable server leaves the cached catalog
/// and installed packages usable.
fn enrich_with_marketplace(
    paths: &SystemPaths,
    cache: &std::path::Path,
    repositories: &[LoadedRepository],
    state: &mut AppState,
    updated_ids: &[String],
) {
    let Some(base_url) = crate::locator::marketplace_base_url() else {
        return;
    };
    state.marketplace_url = Some(base_url.to_string());
    let index = crate::marketplace::index_with_cache(cache);
    if let Some(index) = &index {
        super::project::apply_index(&mut state.items, index);
    }
    let catalog_age = crate::locator::default_catalog_locator()
        .ok()
        .flatten()
        .and_then(|default| {
            repositories
                .iter()
                .find(|repository| repository.definition.locator.same_identity(&default))
        })
        .and_then(|repository| {
            repository.snapshot.as_ref().map(|_| {
                if repository.refresh_failed {
                    25 * 60 * 60
                } else {
                    0
                }
            })
        });
    let ledger_error = crate::executor::read_ledger(paths).err();
    let (report, findings) = crate::preflight::run(&crate::preflight::PreflightInput {
        paths,
        startup: crate::STARTUP_REPORT.get(),
        profiles: &state.agent_profiles,
        items: &state.items,
        catalog_age_seconds: catalog_age,
        ledger_error,
    });
    report.write_cache(cache);
    let identity = findings
        .identity
        .map(|me| crate::app_state::MarketplaceIdentity {
            account: me.account,
            namespace: me.namespace,
            display_name: me.display_name,
            admin: me.admin,
            auth_mode: report.auth_mode.clone(),
        });
    super::project::write_identity_cache(cache, identity.as_ref());
    state.identity = identity;
    let checks = report.status_map();
    state.preflight = Some(report);

    let agents = super::items::enabled_agent_ids(paths);
    let installed = state
        .items
        .iter()
        .filter(|item| super::items::counts_as_installed(item.status))
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let mut events = vec![crate::marketplace::ClientEvent::heartbeat(
        agents.clone(),
        installed,
        checks,
    )];
    for id in updated_ids {
        let version = index
            .as_ref()
            .and_then(|index| index.package(id))
            .map(|package| package.version.clone());
        events.push(crate::marketplace::ClientEvent::update(
            id,
            None,
            version,
            agents.clone(),
        ));
    }
    crate::marketplace::send_events_background(events);
}

pub(super) fn ensure_default_catalog(
    cache: &std::path::Path,
    repositories: &mut Vec<ConfiguredRepository>,
) -> Option<String> {
    let locator = match default_catalog_locator() {
        Ok(Some(locator)) => locator,
        Ok(None) => return None,
        Err(message) => return Some(message),
    };
    if repositories
        .iter()
        .any(|repository| repository.locator.same_identity(&locator))
    {
        return None;
    }
    match source::prepare_new_repository(&locator, cache) {
        Ok(candidate) => match source::activate_repository(cache, candidate) {
            Ok(snapshot) => {
                repositories.push(snapshot.definition);
                None
            }
            Err(message) => Some(message),
        },
        Err(message) => Some(message),
    }
}

/// One line naming what sync retired because it no longer exists upstream.
fn retired_message(repositories: &[String], sources: &[String]) -> Option<String> {
    let mut parts = Vec::new();
    if !repositories.is_empty() {
        parts.push(format!(
            "Removed the retired catalog{} {}.",
            if repositories.len() == 1 { "" } else { "s" },
            repositories.join(", ")
        ));
    }
    if !sources.is_empty() {
        parts.push(format!(
            "Removed the retired source{} {}.",
            if sources.len() == 1 { "" } else { "s" },
            sources.join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Refreshes every configured catalog. A catalog whose URL no longer exists
/// (HTTP 404/410) is retired: dropped from the configuration and its cache
/// wiped, so a decommissioned host never surfaces as a persistent error. The
/// returned names are the retired catalogs.
pub(super) fn refresh_repositories(
    cache: &std::path::Path,
    definitions: Vec<ConfiguredRepository>,
) -> (
    Vec<ConfiguredRepository>,
    Vec<LoadedRepository>,
    Vec<String>,
) {
    let mut updated = Vec::with_capacity(definitions.len());
    let mut loaded = Vec::with_capacity(definitions.len());
    let mut retired = Vec::new();
    for definition in definitions {
        let prepared = source::prepare_repository_refresh(&definition, cache).or_else(|message| {
            if !source::is_corrupt_cache_error(&message) {
                return Err(message);
            }
            eprintln!(
                "Wiping the unreadable cache of the catalog {} and refreshing it again: {message}",
                definition.name
            );
            source::remove_repository_cache(cache, &definition.repository_key)?;
            source::prepare_repository_refresh(&definition, cache)
        });
        match prepared {
            Ok(candidate) => {
                if candidate.definition.repository_id != definition.repository_id {
                    source::discard_repository(&candidate);
                    let snapshot = source::load_current_repository(cache, &definition)
                        .ok()
                        .flatten();
                    let message = format!(
                        "The catalog changed repository.id from {} to {}. The last validated revision remains active.",
                        definition.repository_id, candidate.definition.repository_id
                    );
                    updated.push(definition.clone());
                    loaded.push(LoadedRepository {
                        definition,
                        snapshot,
                        status: SourceStatus::Error,
                        refresh_failed: true,
                        message: Some(message),
                    });
                    continue;
                }
                match source::activate_repository(cache, candidate) {
                    Ok(snapshot) => {
                        updated.push(snapshot.definition.clone());
                        loaded.push(LoadedRepository {
                            definition: snapshot.definition.clone(),
                            snapshot: Some(snapshot),
                            status: SourceStatus::Fresh,
                            refresh_failed: false,
                            message: None,
                        });
                    }
                    Err(message) => {
                        let snapshot = source::load_current_repository(cache, &definition)
                            .ok()
                            .flatten();
                        updated.push(definition.clone());
                        loaded.push(LoadedRepository {
                            definition,
                            snapshot,
                            status: SourceStatus::Error,
                            refresh_failed: true,
                            message: Some(message),
                        });
                    }
                }
            }
            Err(message) => {
                if crate::artifact::is_gone(&message) {
                    if let Err(error) =
                        source::remove_repository_cache(cache, &definition.repository_key)
                    {
                        eprintln!(
                            "Could not remove the cache of the retired catalog {}: {error}",
                            definition.name
                        );
                    }
                    retired.push(definition.name.clone());
                    continue;
                }
                let snapshot = source::load_current_repository(cache, &definition)
                    .ok()
                    .flatten();
                updated.push(definition.clone());
                loaded.push(LoadedRepository {
                    definition,
                    snapshot,
                    status: SourceStatus::Error,
                    refresh_failed: true,
                    message: Some(message),
                });
            }
        }
    }
    (updated, loaded, retired)
}

/// Refreshes every configured source. A source whose archive no longer exists
/// (HTTP 404/410) and has nothing installed from it is retired: dropped from
/// the configuration and its cache wiped. One with installed packages is kept
/// with a note so the packages can still be removed. The returned names are
/// the retired sources.
pub(super) fn refresh_sources(
    cache: &std::path::Path,
    definitions: Vec<ConfiguredSource>,
    installed_keys: &BTreeSet<String>,
) -> (Vec<ConfiguredSource>, Vec<LoadedSource>, Vec<String>) {
    let mut claimed = definitions
        .iter()
        .map(|source| (source.source_id.clone(), source.source_key.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut updated_definitions = Vec::with_capacity(definitions.len());
    let mut loaded = Vec::with_capacity(definitions.len());
    let mut retired = Vec::new();

    for definition in definitions {
        let prepared = source::prepare_refresh(&definition, cache).or_else(|message| {
            if !source::is_corrupt_cache_error(&message) {
                return Err(message);
            }
            eprintln!(
                "Wiping the unreadable cache of the source {} and refreshing it again: {message}",
                definition.name
            );
            source::remove_source_cache(cache, &definition.source_key)?;
            source::prepare_refresh(&definition, cache)
        });
        match prepared {
            Ok(candidate) => {
                let source_id_changed = candidate.definition.source_id != definition.source_id;
                let duplicate_namespace = claimed
                    .get(&candidate.definition.source_id)
                    .is_some_and(|source_key| source_key != &definition.source_key);
                if source_id_changed || duplicate_namespace {
                    let message = if source_id_changed {
                        format!(
                            "The source changed source.id from {} to {}. The last validated revision remains active.",
                            definition.source_id, candidate.definition.source_id
                        )
                    } else {
                        format!(
                            "The namespace {} is already claimed by another source.",
                            candidate.definition.source_id
                        )
                    };
                    source::discard_candidate(&candidate);
                    let snapshot = source::load_current(cache, &definition).ok().flatten();
                    updated_definitions.push(definition.clone());
                    loaded.push(LoadedSource {
                        definition,
                        snapshot,
                        status: SourceStatus::Error,
                        refresh_failed: true,
                        message: Some(message),
                    });
                    continue;
                }
                match source::activate_candidate(cache, candidate) {
                    Ok(snapshot) => {
                        claimed.insert(
                            snapshot.definition.source_id.clone(),
                            snapshot.definition.source_key.clone(),
                        );
                        updated_definitions.push(snapshot.definition.clone());
                        loaded.push(LoadedSource {
                            definition: snapshot.definition.clone(),
                            snapshot: Some(snapshot),
                            status: SourceStatus::Fresh,
                            refresh_failed: false,
                            message: None,
                        });
                    }
                    Err(message) => push_refresh_error(
                        cache,
                        definition,
                        message,
                        &mut updated_definitions,
                        &mut loaded,
                    ),
                }
            }
            Err(message) => {
                if crate::artifact::is_gone(&message) {
                    if !installed_keys.contains(&definition.source_key) {
                        if let Err(error) =
                            source::remove_source_cache(cache, &definition.source_key)
                        {
                            eprintln!(
                                "Could not remove the cache of the retired source {}: {error}",
                                definition.name
                            );
                        }
                        retired.push(definition.name.clone());
                        continue;
                    }
                    let message = format!(
                        "{} is no longer published at its URL. Its installed packages remain until you remove the source.",
                        definition.name
                    );
                    push_refresh_error(
                        cache,
                        definition,
                        message,
                        &mut updated_definitions,
                        &mut loaded,
                    );
                    continue;
                }
                push_refresh_error(
                    cache,
                    definition,
                    message,
                    &mut updated_definitions,
                    &mut loaded,
                )
            }
        }
    }
    (updated_definitions, loaded, retired)
}

pub(super) fn is_unsupported_legacy_install(record: &InstallationRecord) -> bool {
    record.manifest_version == 1
        || matches!(
            record.component_kind.as_str(),
            "agentPlugin" | "legacyFileTree" | "fileTree"
        )
}

pub(super) fn retire_unsupported_legacy_installs(paths: &SystemPaths) -> Result<(), String> {
    let ledger = crate::executor::read_ledger(paths)?;
    let mut groups = BTreeMap::<String, (ConfiguredSource, Vec<String>)>::new();
    for (id, record) in &ledger.items {
        if !is_unsupported_legacy_install(record) {
            continue;
        }
        groups
            .entry(record.source_key.clone())
            .or_insert_with(|| {
                (
                    ConfiguredSource {
                        source_key: record.source_key.clone(),
                        source_id: record.source_id.clone(),
                        name: record.source_id.clone(),
                        description: "Retired unsupported legacy installation.".to_string(),
                        locator: Locator::parse(&record.source_url)
                            .unwrap_or_else(|_| Locator::display_url(record.source_url.clone())),
                        repository_key: None,
                    },
                    Vec::new(),
                )
            })
            .1
            .push(id.clone());
    }
    for (source, ids) in groups.into_values() {
        crate::executor::uninstall_batch(paths, &source, &ids, true)?;
    }
    Ok(())
}

pub(super) fn push_refresh_error(
    cache: &std::path::Path,
    definition: ConfiguredSource,
    message: String,
    updated_definitions: &mut Vec<ConfiguredSource>,
    loaded: &mut Vec<LoadedSource>,
) {
    let snapshot = source::load_current(cache, &definition).ok().flatten();
    updated_definitions.push(definition.clone());
    loaded.push(LoadedSource {
        definition,
        snapshot,
        status: SourceStatus::Error,
        refresh_failed: true,
        message: Some(message),
    });
}

pub(super) fn reconcile_installed_items(
    paths: &SystemPaths,
    loaded: &[LoadedSource],
) -> Result<AutoUpdateReport, String> {
    let mut report = AutoUpdateReport::default();
    let agents_enabled = agent_profiles::read(paths)?
        .iter()
        .any(|profile| profile.enabled);
    for source in loaded {
        let Some(snapshot) = &source.snapshot else {
            continue;
        };
        let ledger_state = crate::executor::read_ledger(paths)?;
        let mut candidates = Vec::new();
        for item in snapshot.catalog.items.values() {
            if item.manifest_version == 2 && !agents_enabled {
                continue;
            }
            if super::status::refined_item_status(paths, &ledger_state, snapshot, item, None)
                != ItemStatus::UpdateAvailable
            {
                continue;
            }
            let selected = ledger_state
                .items
                .get(&item.id)
                .map(|record| crate::planner::selected_component_ids(record, item));
            let identities = crate::planner::plan(paths, snapshot, item, None, selected.as_deref())
                .map(|plan| {
                    plan.resources
                        .values()
                        .map(|resource| resource.desired.identity())
                        .collect()
                })
                .unwrap_or_else(|_| BTreeSet::from([format!("item:{}", item.id)]));
            candidates.push(UpdateCandidate { item, identities });
        }

        for group in overlapping_update_groups(candidates) {
            let result = if let [candidate] = group.as_slice() {
                let current = crate::executor::read_ledger(paths)?;
                let selected = current
                    .items
                    .get(&candidate.item.id)
                    .map(|record| crate::planner::selected_component_ids(record, candidate.item));
                install::install_item_components_approved(
                    paths,
                    &source.definition,
                    snapshot,
                    candidate.item,
                    false,
                    selected.as_deref(),
                )
            } else {
                let requests = group
                    .iter()
                    .map(|candidate| crate::executor::BatchInstall {
                        source: &source.definition,
                        snapshot,
                        item: candidate.item,
                        replace_unmanaged: false,
                    })
                    .collect::<Vec<_>>();
                crate::executor::install_batch(paths, &requests, false)
            };
            match result {
                Ok(_) => report
                    .updated_items
                    .extend(group.iter().map(|candidate| ItemReference {
                        id: candidate.item.id.clone(),
                        source_id: candidate.item.source_id.clone(),
                        local_id: candidate.item.local_id.clone(),
                    })),
                Err(message) => {
                    report
                        .failed_items
                        .extend(group.iter().map(|candidate| ItemFailure {
                            id: candidate.item.id.clone(),
                            message: message.clone(),
                        }))
                }
            }
        }
    }
    Ok(report)
}

struct UpdateCandidate<'a> {
    item: &'a crate::catalog::CatalogItem,
    identities: BTreeSet<String>,
}

fn overlapping_update_groups<'a>(
    candidates: Vec<UpdateCandidate<'a>>,
) -> Vec<Vec<UpdateCandidate<'a>>> {
    let mut groups = Vec::<Vec<UpdateCandidate<'a>>>::new();
    for candidate in candidates {
        let mut connected = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            if group
                .iter()
                .any(|member| !member.identities.is_disjoint(&candidate.identities))
            {
                connected.push(index);
            }
        }
        if connected.is_empty() {
            groups.push(vec![candidate]);
            continue;
        }
        let first = connected[0];
        groups[first].push(candidate);
        for index in connected.into_iter().skip(1).rev() {
            let merged = groups.remove(index);
            groups[first].extend(merged);
        }
    }
    groups
}

#[cfg(test)]
mod retire_tests {
    use super::*;
    use crate::locator::Locator;
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::TcpListener;

    fn serve_not_found(requests: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            for _ in 0..requests {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok() {
                    if line == "\r\n" || line == "\n" || line.is_empty() {
                        break;
                    }
                    line.clear();
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        format!("http://{address}")
    }

    #[test]
    fn a_gone_source_without_installs_is_retired_and_its_cache_wiped() {
        let base = serve_not_found(4);
        let cache = tempfile::tempdir().expect("cache");
        let locator = Locator::display_url(format!("{base}/retired-latest.zip"));
        let definition = ConfiguredSource {
            source_key: locator.source_key(),
            source_id: "retired".to_string(),
            name: "Retired".to_string(),
            description: "Gone".to_string(),
            locator,
            repository_key: None,
        };
        let root = source::source_cache_root(cache.path(), &definition.source_key);
        std::fs::create_dir_all(&root).expect("cache root");
        std::fs::write(root.join("current.json"), b"{}").expect("stale pointer");

        let (updated, loaded, retired) =
            refresh_sources(cache.path(), vec![definition.clone()], &BTreeSet::new());
        assert!(
            updated.is_empty(),
            "{:?}",
            loaded
                .iter()
                .map(|source| source.message.clone())
                .collect::<Vec<_>>()
        );
        assert!(loaded.is_empty());
        assert_eq!(retired, vec!["Retired".to_string()]);
        assert!(!root.exists());

        let installed = BTreeSet::from([definition.source_key.clone()]);
        let (updated, loaded, retired) =
            refresh_sources(cache.path(), vec![definition], &installed);
        assert_eq!(updated.len(), 1);
        assert_eq!(retired.len(), 0);
        assert!(loaded[0].refresh_failed);
        assert!(loaded[0]
            .message
            .as_deref()
            .is_some_and(|message| message.contains("no longer published")));
    }

    #[test]
    fn a_gone_catalog_is_retired() {
        let base = serve_not_found(2);
        let cache = tempfile::tempdir().expect("cache");
        let locator = Locator::display_url(format!("{base}/catalog.json"));
        let definition = ConfiguredRepository {
            repository_key: locator.repository_key(),
            repository_id: "nexus".to_string(),
            name: "Nexus".to_string(),
            description: "Retired host".to_string(),
            locator,
        };
        let (updated, loaded, retired) = refresh_repositories(cache.path(), vec![definition]);
        assert!(updated.is_empty());
        assert!(loaded.is_empty());
        assert_eq!(retired, vec!["Nexus".to_string()]);
        assert_eq!(
            retired_message(&retired, &[]).as_deref(),
            Some("Removed the retired catalog Nexus.")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::read_manifest_catalog;
    use crate::source::{ConfiguredSource, SourceSnapshot, TEST_SOURCE_KEY};
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

    #[test]
    fn cached_state_keeps_what_the_last_sync_learned() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let cache = root.path().join("cache-base");
        let config = root.path().join("config-base");
        fs::create_dir_all(&cache).expect("cache");
        fs::create_dir_all(&config).expect("config");
        let identity = crate::app_state::MarketplaceIdentity {
            account: "CORP\\jacob".to_string(),
            namespace: "jacob".to_string(),
            display_name: "Jacob".to_string(),
            admin: false,
            auth_mode: "Negotiate".to_string(),
        };
        super::super::project::write_identity_cache(&cache, Some(&identity));

        let state = cached_app_state(&paths, &cache, &config).expect("cached state");

        assert_eq!(
            state.identity.map(|identity| identity.namespace),
            crate::locator::marketplace_base_url().map(|_| "jacob".to_string()),
            "a cached load must carry the identity the last sync stored"
        );
    }

    fn snapshot(root: &Path, body: &str, commit: char) -> (ConfiguredSource, SourceSnapshot) {
        let source_root = root.join(format!("source-{commit}"));
        let skill_root = source_root.join("skills/python-standards");
        fs::create_dir_all(&skill_root).expect("skill directory");
        fs::write(
            skill_root.join("SKILL.md"),
            format!("---\nname: python-standards\ndescription: Python standards\n---\n{body}\n"),
        )
        .expect("skill");
        fs::write(
            source_root.join("skill-manager.json"),
            r#"{
              "version": 2,
              "source": {"id":"skillbook","name":"Skillbook","description":"Skills"},
              "packages": [
                {
                  "id":"python-standards",
                  "components":[{"kind":"skill","path":"skills/python-standards"}]
                },
                {
                  "id":"python",
                  "components":[
                    {"kind":"skill","id":"python-standards","path":"skills/python-standards"}
                  ]
                }
              ]
            }"#,
        )
        .expect("manifest");
        let catalog = read_manifest_catalog(&source_root, TEST_SOURCE_KEY).expect("catalog");
        let mut source = ConfiguredSource::test_fixture(
            "skillbook",
            "https://nexus.example.com/repository/raw/sources/skillbook-latest.zip",
        );
        source.source_key = TEST_SOURCE_KEY.to_string();
        let snapshot = SourceSnapshot {
            definition: source.clone(),
            commit: commit.to_string().repeat(40),
            path: source_root,
            catalog,
        };
        (source, snapshot)
    }

    #[test]
    fn background_update_reconciles_packages_that_share_a_changed_component() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(
            &paths,
            crate::agent_profiles::TargetId::ClaudeCode,
            true,
        )
        .expect("enable Claude");
        let (source, original) = snapshot(root.path(), "Old guidance", 'a');
        for id in ["python-standards", "python"] {
            crate::executor::install(
                &paths,
                &source,
                &original,
                &original.catalog.items[id],
                false,
                false,
            )
            .expect("initial install");
        }

        let (_, updated) = snapshot(root.path(), "New guidance", 'b');
        let loaded = [LoadedSource {
            definition: source,
            snapshot: Some(updated.clone()),
            status: SourceStatus::Fresh,
            refresh_failed: false,
            message: None,
        }];
        let report = reconcile_installed_items(&paths, &loaded).expect("reconcile");

        assert!(report.failed_items.is_empty());
        assert_eq!(report.updated_items.len(), 2);
        let installed = fs::read_to_string(
            paths
                .home
                .join(".claude/skills/skillbook-python-standards/SKILL.md"),
        )
        .expect("installed skill");
        assert!(installed.contains("New guidance"));
        let ledger = crate::executor::read_ledger(&paths).expect("ledger");
        assert!(updated.catalog.items.values().all(|item| {
            ledger
                .items
                .get(&item.id)
                .is_some_and(|record| record.item_digest == item.digest)
        }));
        assert_eq!(
            ledger
                .resources
                .values()
                .next()
                .expect("shared resource")
                .consumer_binding_ids
                .len(),
            2
        );
    }
}
