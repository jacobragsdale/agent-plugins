use super::RuntimeState;
use crate::app_state::{BulkAction, BulkFailure, BulkPlan, BulkPlanEntry, BulkResult, ItemsPlan};
use crate::catalog::{CatalogComponentKind, CatalogItem};
use crate::executor::ContentState;
use crate::install::{self, ItemStatus, OperationOutcome, SourceRemovalPlan};
use crate::marketplace::{self, ClientEvent};
use crate::paths::SystemPaths;
use crate::planner;
use crate::source::{self, ConfiguredSource, SourceSnapshot};
use crate::sources::{cache_base_dir, config_base_dir};
use std::collections::BTreeSet;
use std::io;

/// Target IDs of the agents the app currently configures, for usage events.
pub(super) fn enabled_agent_ids(paths: &SystemPaths) -> Vec<String> {
    crate::agent_profiles::read(paths)
        .into_iter()
        .filter(|profile| profile.enabled)
        .map(|profile| profile.target_id.as_str().to_string())
        .collect()
}

/// Target IDs the ledger binds `canonical_id` to, for usage events.
pub(super) fn installed_agent_ids(
    ledger: &crate::ledger::InstallationLedger,
    canonical_id: &str,
) -> Vec<String> {
    let Some(record) = ledger.items.get(canonical_id) else {
        return Vec::new();
    };
    record
        .binding_ids
        .iter()
        .filter_map(|id| ledger.bindings.get(id))
        .map(|binding| binding.target_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Statuses that mean the package is on this machine, for the installed set.
pub(super) fn counts_as_installed(status: ItemStatus) -> bool {
    matches!(
        status,
        ItemStatus::Installed
            | ItemStatus::UpdateAvailable
            | ItemStatus::Modified
            | ItemStatus::PartiallyInstalled
            | ItemStatus::Missing
    )
}

fn marketplace_version(canonical_id: &str) -> Option<String> {
    let cache = cache_base_dir().ok()?;
    marketplace::read_cached_index(&cache)?
        .package(canonical_id)
        .map(|package| package.version.clone())
}

/// Reports a completed install or uninstall to the marketplace, best-effort.
fn report_operation(kind: BulkAction, canonical_ids: &[String]) {
    if canonical_ids.is_empty() {
        return;
    }
    let Ok(paths) = SystemPaths::from_system() else {
        return;
    };
    let agents = enabled_agent_ids(&paths);
    let ledger = crate::executor::read_ledger(&paths).ok();
    let events = canonical_ids
        .iter()
        .map(|id| match kind {
            BulkAction::Install | BulkAction::Replace => {
                // The apps the package went to, not every app detected: a
                // skill never reaches Claude Desktop, and a connector can be
                // kept out of an app.
                let agents = ledger
                    .as_ref()
                    .map_or_else(|| agents.clone(), |ledger| installed_agent_ids(ledger, id));
                ClientEvent::install(id, marketplace_version(id), agents)
            }
            BulkAction::Uninstall => ClientEvent::uninstall(id, agents.clone()),
        })
        .collect();
    marketplace::send_events_background(events);
}

pub(crate) async fn install_item(
    runtime: &RuntimeState,
    source_id: &str,
    local_id: &str,
    trust_approved: bool,
    component_id: Option<&str>,
) -> Result<OperationOutcome, String> {
    let _guard = runtime.operation_lock.lock().await;
    let (paths, source, snapshot, item) = item_context(source_id, local_id)?;
    let ids = requested_component_ids(&item, component_id)?;
    let outcome = install::install_item_components_approved(
        &paths,
        &source,
        &snapshot,
        &item,
        trust_approved,
        ids.as_deref(),
    )?;
    report_operation(BulkAction::Install, std::slice::from_ref(&item.id));
    Ok(outcome)
}

pub(crate) async fn replace_item(
    runtime: &RuntimeState,
    source_id: &str,
    local_id: &str,
    trust_approved: bool,
    component_id: Option<&str>,
) -> Result<OperationOutcome, String> {
    let _guard = runtime.operation_lock.lock().await;
    let (paths, source, snapshot, item) = item_context(source_id, local_id)?;
    let ids = requested_component_ids(&item, component_id)?;
    let outcome = install::replace_item_components_approved(
        &paths,
        &source,
        &snapshot,
        &item,
        trust_approved,
        ids.as_deref(),
    )?;
    report_operation(BulkAction::Replace, std::slice::from_ref(&item.id));
    Ok(outcome)
}

/// Saves whether the package's skills (or one of them) run only when asked,
/// then reinstalls the installed ones so their SKILL.md carries the choice.
/// A reinstall that fails puts the previous choice back.
pub(crate) async fn set_manual_invocation(
    runtime: &RuntimeState,
    source_id: &str,
    local_id: &str,
    component_id: Option<&str>,
    manual: bool,
) -> Result<OperationOutcome, String> {
    let _guard = runtime.operation_lock.lock().await;
    let (paths, source, snapshot, item) = item_context(source_id, local_id)?;
    if let Some(component_id) = component_id {
        crate::planner::validate_component_id(&item, component_id)?;
    }
    let skills = item
        .components
        .iter()
        .filter(|component| {
            component.kind == CatalogComponentKind::Skill
                && component_id.is_none_or(|id| component.id == id)
        })
        .collect::<Vec<_>>();
    if skills.is_empty() {
        return Err(format!("{} has no skill to change.", item.id));
    }
    let previous = crate::invocation::read(&paths)?;
    let mut next = previous.clone();
    for component in &skills {
        crate::invocation::set(&mut next, &item.id, component, manual);
    }
    if next == previous {
        return Ok(OperationOutcome::default());
    }
    crate::invocation::write(&paths, &next)?;
    let installed = crate::executor::read_ledger(&paths)?
        .items
        .get(&item.id)
        .map(|record| planner::selected_component_ids(record, &item))
        .unwrap_or_default();
    let reinstall = skills
        .iter()
        .filter(|component| installed.contains(&component.id))
        .map(|component| component.id.clone())
        .collect::<Vec<_>>();
    if reinstall.is_empty() {
        return Ok(OperationOutcome::default());
    }
    install::install_item_components_approved(
        &paths,
        &source,
        &snapshot,
        &item,
        false,
        Some(&reinstall),
    )
    .inspect_err(|_| {
        if let Err(error) = crate::invocation::write(&paths, &previous) {
            eprintln!("Could not restore the skill invocation choices: {error}");
        }
    })
}

pub(super) fn requested_component_ids(
    item: &CatalogItem,
    component_id: Option<&str>,
) -> Result<Option<Vec<String>>, String> {
    match component_id {
        None => Ok(None),
        Some(component_id) => {
            crate::planner::validate_component_id(item, component_id)?;
            Ok(Some(vec![component_id.to_string()]))
        }
    }
}

/// Removes a package or one component. `force` removes one the person
/// changed too, after saving their copy to the backups folder.
pub(crate) async fn uninstall_item(
    runtime: &RuntimeState,
    source_id: &str,
    local_id: &str,
    component_id: Option<&str>,
    force: bool,
) -> Result<OperationOutcome, String> {
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let config = config_base_dir()?;
    let canonical_id = format!("{source_id}/{local_id}");
    // What is installed decides: a package whose source is no longer
    // configured, or one installed from a local folder, still comes off.
    let ledger = crate::executor::read_ledger(&paths)?;
    let source = match (
        source::configured_source(&config, source_id),
        ledger.items.get(&canonical_id),
    ) {
        (Ok(source), Some(record)) if source.source_key != record.source_key => {
            super::project::record_source(&[], record)
        }
        (Ok(source), _) => source,
        (Err(_), Some(record)) => super::project::record_source(&[], record),
        (Err(error), None) => return Err(error),
    };
    let ids = component_id.map(|component_id| vec![component_id.to_string()]);
    let outcome =
        install::uninstall_item_components(&paths, &source, &canonical_id, ids.as_deref(), force)?;
    if ids.is_none() {
        forget_hold(&paths, &canonical_id);
        report_operation(BulkAction::Uninstall, &[canonical_id]);
    }
    Ok(outcome)
}

/// A hold belongs to the install it was set on; installing again starts fresh.
fn forget_hold(paths: &SystemPaths, canonical_id: &str) {
    let mut choices = crate::choices::read_or_default(paths);
    if choices.held.remove(canonical_id) {
        if let Err(error) = crate::choices::write(paths, &choices) {
            eprintln!("Could not forget the update hold on {canonical_id}: {error}");
        }
    }
}

/// Stops managing a package and leaves its files as they are, so the person
/// keeps their own edited copy.
pub(crate) async fn keep_my_version(
    runtime: &RuntimeState,
    source_id: &str,
    local_id: &str,
) -> Result<(), String> {
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let canonical_id = format!("{source_id}/{local_id}");
    crate::executor::forget(&paths, &canonical_id)?;
    forget_hold(&paths, &canonical_id);
    Ok(())
}

/// Holds a package's background updates, or lets them run again.
pub(crate) async fn set_held(
    runtime: &RuntimeState,
    source_id: &str,
    local_id: &str,
    held: bool,
) -> Result<(), String> {
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let mut choices = crate::choices::read(&paths)?;
    let id = format!("{source_id}/{local_id}");
    if held {
        choices.held.insert(id);
    } else {
        choices.held.remove(&id);
    }
    crate::choices::write(&paths, &choices)
}

/// Keeps a component out of some apps and applies it at once. Leaving an app
/// only removes what it had; adding one back installs there, which for an MCP
/// server needs the approval the window asked for.
pub(crate) async fn set_excluded_apps(
    runtime: &RuntimeState,
    source_id: &str,
    local_id: &str,
    component_id: &str,
    excluded: Vec<String>,
    trust_approved: bool,
) -> Result<OperationOutcome, String> {
    let _guard = runtime.operation_lock.lock().await;
    let (paths, source, snapshot, item) = item_context(source_id, local_id)?;
    planner::validate_component_id(&item, component_id)?;
    if let Some(unknown) = excluded.iter().find(|id| {
        !crate::agent_profiles::TargetId::ALL
            .iter()
            .any(|target| target.as_str() == id.as_str())
    }) {
        return Err(format!("{unknown} is not an app Agent Plugins knows."));
    }
    let previous = crate::choices::read(&paths)?;
    let mut next = previous.clone();
    next.set_excluded(&item.id, component_id, excluded.into_iter().collect());
    if next == previous {
        return Ok(OperationOutcome::default());
    }
    crate::choices::write(&paths, &next)?;
    let installed = crate::executor::read_ledger(&paths)?
        .items
        .get(&item.id)
        .is_some_and(|record| {
            planner::selected_component_ids(record, &item)
                .iter()
                .any(|id| id == component_id)
        });
    if !installed {
        return Ok(OperationOutcome::default());
    }
    install::install_item_components_approved(
        &paths,
        &source,
        &snapshot,
        &item,
        trust_approved,
        Some(&[component_id.to_string()]),
    )
    .inspect_err(|_| {
        if let Err(error) = crate::choices::write(&paths, &previous) {
            eprintln!("Could not restore the app choices: {error}");
        }
    })
}

/// Saves connector settings such as API keys where the person's AI apps read
/// environment variables.
pub(crate) async fn save_connector_settings(
    values: std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    for (name, value) in values {
        crate::startup::save_user_variable(&name, value.trim())?;
    }
    Ok(())
}

pub(crate) async fn bulk_plan(
    runtime: &RuntimeState,
    source_id: &str,
    action: BulkAction,
) -> Result<BulkPlan, String> {
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let cache = cache_base_dir()?;
    let config = config_base_dir()?;
    let source = source::configured_source(&config, source_id)?;
    let snapshot = source::load_current(&cache, &source)?
        .ok_or_else(|| format!("{} has no validated revision.", source.source_id))?;
    let ledger_state = crate::executor::read_ledger(&paths)?;
    Ok(BulkPlan {
        entries: plan_entries(
            &paths,
            &ledger_state,
            &snapshot,
            snapshot.catalog.items.values(),
            action,
        ),
        source_id: source.source_id,
        action,
    })
}

/// What `action` does to each of `items`: a whole source's, or the packages a
/// bundle or a person picked.
fn plan_entries<'a>(
    paths: &SystemPaths,
    ledger_state: &crate::ledger::InstallationLedger,
    snapshot: &SourceSnapshot,
    items: impl IntoIterator<Item = &'a CatalogItem>,
    action: BulkAction,
) -> Vec<BulkPlanEntry> {
    items
        .into_iter()
        .map(|item| {
            let status =
                super::status::refined_item_status(paths, ledger_state, snapshot, item, None, None);
            BulkPlanEntry {
                id: item.id.clone(),
                local_id: item.local_id.clone(),
                status,
                will_run: match action {
                    BulkAction::Install => match status {
                        ItemStatus::Available | ItemStatus::UpdateAvailable => true,
                        // Installing all never adds components the user left
                        // out, so a partial install with its selection in
                        // place has nothing to do.
                        ItemStatus::PartiallyInstalled => {
                            !selection_satisfied(paths, ledger_state, snapshot, item)
                        }
                        _ => false,
                    },
                    BulkAction::Replace => status == ItemStatus::Conflict,
                    BulkAction::Uninstall => {
                        matches!(
                            status,
                            ItemStatus::Installed
                                | ItemStatus::UpdateAvailable
                                | ItemStatus::PartiallyInstalled
                                | ItemStatus::Missing
                        )
                    }
                },
            }
        })
        .collect()
}

/// Whether everything the installed selection of a package writes is already
/// recorded.
fn selection_satisfied(
    paths: &SystemPaths,
    ledger_state: &crate::ledger::InstallationLedger,
    snapshot: &SourceSnapshot,
    item: &CatalogItem,
) -> bool {
    ledger_state.items.get(&item.id).is_some_and(|record| {
        let selected = planner::selected_component_ids(record, item);
        planner::plan(paths, snapshot, item, None, Some(&selected))
            .is_ok_and(|plan| crate::executor::plan_satisfied(ledger_state, &plan).unwrap_or(false))
    })
}

/// A configured source, its snapshot, and the packages of it that were asked for.
type ItemSource = (ConfiguredSource, SourceSnapshot, Vec<CatalogItem>);

/// Finds each canonical id (`source-id/package-id`) in the configured sources,
/// grouped by source. Ids that no source on this computer lists come back on
/// their own: a bundle can name a package from a source the next sync adds.
fn item_sources(ids: &[String]) -> Result<(Vec<ItemSource>, Vec<String>), String> {
    let cache = cache_base_dir()?;
    let config = config_base_dir()?;
    let mut grouped = std::collections::BTreeMap::<&str, Vec<&str>>::new();
    let mut missing = Vec::new();
    for id in ids {
        match id.split_once('/') {
            Some((source_id, local_id)) => {
                let local_ids = grouped.entry(source_id).or_default();
                if !local_ids.contains(&local_id) {
                    local_ids.push(local_id);
                }
            }
            None => missing.push(id.clone()),
        }
    }
    let mut sources = Vec::new();
    for (source_id, local_ids) in grouped {
        let loaded = source::configured_source(&config, source_id)
            .ok()
            .and_then(|source| {
                let snapshot = source::load_current(&cache, &source).ok().flatten()?;
                Some((source, snapshot))
            });
        let Some((source, snapshot)) = loaded else {
            missing.extend(local_ids.iter().map(|id| format!("{source_id}/{id}")));
            continue;
        };
        let mut items = Vec::new();
        for local_id in local_ids {
            match snapshot.catalog.items.get(local_id) {
                Some(item) => items.push(item.clone()),
                None => missing.push(format!("{source_id}/{local_id}")),
            }
        }
        sources.push((source, snapshot, items));
    }
    Ok((sources, missing))
}

/// What `action` does to each package in `ids`, whichever sources they come
/// from. Packages not on this computer yet are left out.
pub(crate) async fn plan_items(
    runtime: &RuntimeState,
    ids: &[String],
    action: BulkAction,
) -> Result<ItemsPlan, String> {
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let ledger_state = crate::executor::read_ledger(&paths)?;
    let (sources, _) = item_sources(ids)?;
    let entries = sources
        .iter()
        .flat_map(|(_, snapshot, items)| {
            plan_entries(&paths, &ledger_state, snapshot, items, action)
        })
        .collect();
    Ok(ItemsPlan { action, entries })
}

pub(crate) async fn bulk_run(
    runtime: &RuntimeState,
    source_id: &str,
    action: BulkAction,
    trust_approved: bool,
) -> Result<BulkResult, String> {
    let ids = bulk_plan(runtime, source_id, action)
        .await?
        .entries
        .into_iter()
        .map(|entry| entry.id)
        .collect::<Vec<_>>();
    run_items(runtime, &ids, action, trust_approved).await
}

/// Applies `action` to every package in `ids` that it would change.
pub(crate) async fn run_items(
    runtime: &RuntimeState,
    ids: &[String],
    action: BulkAction,
    trust_approved: bool,
) -> Result<BulkResult, String> {
    let plan = plan_items(runtime, ids, action).await?;
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let (sources, missing) = item_sources(ids)?;
    let mut result = BulkResult {
        completed: Vec::new(),
        failures: Vec::new(),
        backup_paths: Vec::new(),
    };
    if action != BulkAction::Uninstall {
        result.failures = missing
            .into_iter()
            .map(|id| BulkFailure {
                message: format!(
                    "{id} isn't available on this computer yet. Refresh, then try again."
                ),
                id,
            })
            .collect();
    }
    // Each package is its own transaction, so one that fails its checks
    // leaves the others to finish and reports its own message.
    for entry in plan.entries.into_iter().filter(|entry| entry.will_run) {
        let Some((source, snapshot, _)) = sources
            .iter()
            .find(|(_, _, items)| items.iter().any(|item| item.id == entry.id))
        else {
            continue;
        };
        let outcome = match action {
            BulkAction::Install | BulkAction::Replace => {
                bulk_install(&paths, source, snapshot, &entry, action, trust_approved)
            }
            BulkAction::Uninstall => {
                install::uninstall_item_components(&paths, source, &entry.id, None, false)
            }
        };
        match outcome {
            Ok(outcome) => {
                result.completed.push(entry.id);
                result.backup_paths.extend(outcome.backup_paths);
            }
            Err(message) => result.failures.push(BulkFailure {
                id: entry.id,
                message,
            }),
        }
    }
    report_operation(action, &result.completed);
    Ok(result)
}

fn bulk_install(
    paths: &SystemPaths,
    source: &ConfiguredSource,
    snapshot: &SourceSnapshot,
    entry: &BulkPlanEntry,
    action: BulkAction,
    trust_approved: bool,
) -> Result<OperationOutcome, String> {
    let item = snapshot
        .catalog
        .items
        .get(&entry.local_id)
        .ok_or_else(|| format!("Unknown catalog item: {}", entry.id))?;
    let selected = crate::executor::read_ledger(paths)?
        .items
        .get(&item.id)
        .map(|record| planner::selected_component_ids(record, item));
    let install = if action == BulkAction::Replace {
        install::replace_item_components_approved
    } else {
        install::install_item_components_approved
    };
    install(
        paths,
        source,
        snapshot,
        item,
        trust_approved,
        selected.as_deref(),
    )
}

pub(crate) async fn plan_source_removal(
    runtime: &RuntimeState,
    source_id: &str,
) -> Result<SourceRemovalPlan, String> {
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let config = config_base_dir()?;
    let source = source::configured_source(&config, source_id)?;
    install::source_removal_plan(&paths, &source)
}

pub(crate) async fn remove_source(
    runtime: &RuntimeState,
    source_id: &str,
    acknowledge_modified_paths: bool,
) -> Result<BulkResult, String> {
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let cache = cache_base_dir()?;
    let config = config_base_dir()?;
    let source = source::configured_source(&config, source_id)?;
    let plan = install::source_removal_plan(&paths, &source)?;
    if plan
        .items
        .iter()
        .flat_map(|item| &item.paths)
        .any(|path| path.modified)
        && !acknowledge_modified_paths
    {
        return Err(
            "Source cleanup includes locally modified paths. Confirm the warning before continuing."
                .to_string(),
        );
    }
    let records = crate::executor::read_ledger(&paths)?
        .items
        .values()
        .filter(|record| record.source_key == source.source_key)
        .map(|record| format!("{}/{}", record.source_id, record.local_id))
        .collect::<Vec<_>>();
    let mut result = BulkResult {
        completed: Vec::new(),
        failures: Vec::new(),
        backup_paths: Vec::new(),
    };
    for id in records {
        match install::uninstall_item_components(
            &paths,
            &source,
            &id,
            None,
            acknowledge_modified_paths,
        ) {
            Ok(outcome) => {
                result.completed.push(id);
                result.backup_paths.extend(outcome.backup_paths);
            }
            Err(message) => result.failures.push(BulkFailure { id, message }),
        }
    }
    report_operation(BulkAction::Uninstall, &result.completed);
    // The source stays configured while any of its packages is still installed.
    if !result.failures.is_empty() {
        return Ok(result);
    }
    let mut config_file = source::read_sources_config(&config)?;
    config_file
        .sources
        .retain(|configured| configured.source_key != source.source_key);
    source::write_sources_config(&config, &config_file)?;
    // The source is gone once the configuration says so; a cache file held by
    // antivirus or the indexer is swept by a later sync.
    if let Err(error) = source::remove_source_cache(&cache, &source.source_key) {
        eprintln!(
            "Could not remove the cache of {}: {error}",
            source.source_id
        );
    }
    Ok(result)
}

pub(crate) async fn reset_app(runtime: &RuntimeState) -> Result<BulkResult, String> {
    let _sync_guard = runtime.sync_lock.lock().await;
    let _guard = runtime.operation_lock.lock().await;
    discard_pending(runtime).await;
    let paths = SystemPaths::from_system()?;
    let cache = cache_base_dir()?;
    let config = config_base_dir()?;
    let sources = source::read_sources_config(&config)?
        .sources
        .into_iter()
        .map(|source| {
            let snapshot = source::load_current(&cache, &source).ok().flatten();
            (source, snapshot)
        })
        .collect::<Vec<_>>();
    // Reset must work when the ledger cannot be read; the executor then
    // removes what it can find and the state wipe takes the rest.
    let records = crate::executor::read_ledger(&paths)
        .map(|ledger| ledger.items.into_keys().collect::<Vec<_>>())
        .unwrap_or_default();
    let outcome = match crate::executor::reset_app(&paths, &sources) {
        Ok(outcome) => outcome,
        Err(message) => {
            let failures = records
                .into_iter()
                .map(|id| BulkFailure {
                    id,
                    message: format!("App reset transaction rolled back: {message}"),
                })
                .collect();
            return Ok(BulkResult {
                completed: Vec::new(),
                failures,
                backup_paths: Vec::new(),
            });
        }
    };
    if crate::executor::read_ledger(&paths)
        .is_ok_and(|ledger| !ledger.read_only && !ledger.items.is_empty())
    {
        return Ok(BulkResult {
            completed: Vec::new(),
            failures: vec![BulkFailure {
                id: "app".to_string(),
                message: "Resources were reset, but ledger cleanup was incomplete.".to_string(),
            }],
            backup_paths: Vec::new(),
        });
    }
    wipe_app_state(&paths)?;
    Ok(BulkResult {
        completed: records,
        failures: Vec::new(),
        backup_paths: outcome.backup_paths,
    })
}

async fn discard_pending(runtime: &RuntimeState) {
    let pending_sources = {
        let mut pending = runtime.pending_sources.lock().await;
        std::mem::take(&mut *pending)
    };
    for candidate in pending_sources.into_values() {
        source::discard_candidate(&candidate);
    }
}

fn wipe_app_state(paths: &SystemPaths) -> Result<(), String> {
    crate::ledger::remove_files(&paths.app_data())?;
    crate::tutorial::remove_skill(paths)?;
    for root in paths.state_roots() {
        match crate::fs_retry::remove_dir_all(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!("Could not remove {}: {error}", root.display()));
            }
        }
    }
    Ok(())
}

/// The configured sources that have a cached snapshot.
fn cached_sources() -> Result<(SystemPaths, Vec<(ConfiguredSource, SourceSnapshot)>), String> {
    let paths = SystemPaths::from_system()?;
    let cache = cache_base_dir()?;
    let config = config_base_dir()?;
    let sources = source::read_sources_config(&config)?
        .sources
        .into_iter()
        .filter_map(|source| {
            let snapshot = source::load_current(&cache, &source).ok().flatten()?;
            Some((source, snapshot))
        })
        .collect();
    Ok((paths, sources))
}

/// Re-creates the files of installed packages that went missing, from the
/// cached snapshot of the version that is installed (D1). The sync calls it
/// while it holds the operation lock; one package failing never stops the
/// others. Also sweeps staging an interrupted run left behind. Returns the
/// names of the packages it repaired.
pub(crate) fn repair_missing_installs() -> Result<Vec<String>, String> {
    let (paths, sources) = cached_sources()?;
    crate::executor::sweep_stale_staging(&paths);
    crate::executor::prune_backups(&paths);
    repair_in(&paths, &sources)
}

fn repair_in(
    paths: &SystemPaths,
    sources: &[(ConfiguredSource, SourceSnapshot)],
) -> Result<Vec<String>, String> {
    let mut repaired = Vec::new();
    let mut ledger = crate::executor::read_ledger(paths)?;
    if crate::ledger::take_restored_marker(&paths.app_data()) {
        forget_missing_after_restore(paths, sources, &ledger);
        return Ok(repaired);
    }
    for (source, snapshot) in sources {
        for item in snapshot.catalog.items.values() {
            let Some(record) = ledger.items.get(&item.id) else {
                continue;
            };
            if record.source_key != source.source_key
                || crate::executor::installation_state(paths, &ledger, &item.id, None)
                    != ContentState::Missing
            {
                continue;
            }
            let mut selected = planner::selected_component_ids(record, item);
            let Ok(mut plan) = planner::plan(paths, snapshot, item, None, Some(&selected)) else {
                continue;
            };
            // Only the installed version comes back; an update is the sync's.
            if record.item_digest != item.digest
                && !super::status::selection_current(&ledger, record, item, &plan)
            {
                continue;
            }
            // An agent found after the install has no approved MCP entry, so
            // put back everything else rather than nothing, as extending does.
            if !planner::mcp_entries_owned(&plan, &ledger, None) {
                selected.retain(|id| {
                    item.components.iter().any(|component| {
                        component.id == *id && component.kind != CatalogComponentKind::McpServer
                    })
                });
                let Ok(skills_only) = planner::plan(paths, snapshot, item, None, Some(&selected))
                else {
                    continue;
                };
                plan = skills_only;
            }
            // An MCP entry comes back only when the ledger still holds that
            // exact entry, which the user approved when installing it.
            let approved = planner::mcp_entries_owned(&plan, &ledger, None);
            match install::install_item_components_approved(
                paths,
                source,
                snapshot,
                item,
                approved,
                Some(&selected),
            ) {
                Ok(_) => {
                    ledger = crate::executor::read_ledger(paths)?;
                    // Only a package that is whole again counts, so a part that
                    // cannot come back does not repeat the notice every sync.
                    if crate::executor::installation_state(paths, &ledger, &item.id, None)
                        != ContentState::Missing
                    {
                        repaired.push(item.name.clone());
                    }
                }
                Err(error) => eprintln!(
                    "Could not restore the missing files of {}: {error}",
                    item.id
                ),
            }
        }
    }
    Ok(repaired)
}

/// After the ledger came back from its older backup, a package whose files
/// are all gone was most likely uninstalled after that backup was taken, so
/// it is forgotten instead of reinstalled (and its connector re-added)
/// without the user asking.
fn forget_missing_after_restore(
    paths: &SystemPaths,
    sources: &[(ConfiguredSource, SourceSnapshot)],
    ledger: &crate::ledger::InstallationLedger,
) {
    for (id, record) in &ledger.items {
        if crate::executor::installation_state(paths, ledger, id, None) != ContentState::Missing {
            continue;
        }
        let Some((source, _)) = sources
            .iter()
            .find(|(source, _)| source.source_key == record.source_key)
        else {
            continue;
        };
        if let Err(error) = install::uninstall_item_components(paths, source, id, None, false) {
            eprintln!("Could not forget {id} after restoring the package list: {error}");
        }
    }
}

/// Adds installed packages to agents detected after they were installed, and
/// releases what they installed for agents that are no longer detected. Skills
/// follow the detected agents freely; an MCP server is added again only to a
/// package that skipped an agent whose settings file was unreadable or
/// locked. The sync calls it while it holds the operation lock. Returns the
/// names of the packages it changed.
pub(crate) fn extend_installs_to_new_agents() -> Result<Vec<String>, String> {
    let (paths, sources) = cached_sources()?;
    extend_in(&paths, &sources)
}

fn extend_in(
    paths: &SystemPaths,
    sources: &[(ConfiguredSource, SourceSnapshot)],
) -> Result<Vec<String>, String> {
    let mut extended = Vec::new();
    let mut ledger = crate::executor::read_ledger(paths)?;
    for (source, snapshot) in sources {
        for item in snapshot.catalog.items.values() {
            let Some(record) = ledger.items.get(&item.id) else {
                continue;
            };
            if record.source_key != source.source_key
                || crate::executor::installation_state(paths, &ledger, &item.id, None)
                    .is_protected()
                || (record.item_digest != item.digest
                    && !installed_agents_current(paths, snapshot, item, &ledger, record))
            {
                continue;
            }
            let retry = crate::executor::take_skipped_agent(&item.id);
            let selected = planner::selected_component_ids(record, item);
            let is_mcp = |id: &str| {
                item.components.iter().any(|component| {
                    component.id == id && component.kind == CatalogComponentKind::McpServer
                })
            };
            // An approved MCP server may be rewritten for the agents that
            // already have it, when this version of the app spells their
            // entries differently; it never spreads to a new agent unasked.
            let mcp_agents_unchanged = record.item_digest == item.digest
                && planner::plan(paths, snapshot, item, None, Some(&selected)).is_ok_and(|plan| {
                    plan.bindings
                        .iter()
                        .filter(|(_, binding)| is_mcp(&binding.component_id))
                        .all(|(binding_id, _)| record.binding_ids.contains(binding_id))
                });
            let ids = selected
                .into_iter()
                .filter(|id| retry || mcp_agents_unchanged || !is_mcp(id))
                .collect::<Vec<_>>();
            let Ok(plan) = planner::plan(paths, snapshot, item, None, Some(&ids)) else {
                continue;
            };
            let current = record
                .binding_ids
                .iter()
                .filter(|binding_id| {
                    ledger
                        .bindings
                        .get(*binding_id)
                        .is_some_and(|binding| ids.contains(&binding.component_id))
                })
                .collect::<BTreeSet<_>>();
            if ids.is_empty()
                || (current == plan.bindings.keys().collect()
                    && crate::executor::plan_satisfied(&ledger, &plan).unwrap_or(true))
            {
                continue;
            }
            match install::install_item_components_approved(
                paths,
                source,
                snapshot,
                item,
                retry || mcp_agents_unchanged,
                Some(&ids),
            ) {
                Ok(outcome) => {
                    if outcome.warnings.is_empty() {
                        extended.push(item.name.clone());
                    }
                    ledger = crate::executor::read_ledger(paths)?;
                }
                Err(error) => eprintln!("Could not update the agents of {}: {error}", item.id),
            }
        }
    }
    Ok(extended)
}

/// Whether the selected components are the published ones for the agents they
/// are installed for, so following the detected agents does not also update
/// them.
fn installed_agents_current(
    paths: &SystemPaths,
    snapshot: &SourceSnapshot,
    item: &CatalogItem,
    ledger: &crate::ledger::InstallationLedger,
    record: &crate::ledger::InstallationRecord,
) -> bool {
    let targets = record
        .binding_ids
        .iter()
        .filter_map(|binding_id| ledger.bindings.get(binding_id))
        .map(|binding| binding.target_id.as_str())
        .collect::<BTreeSet<_>>();
    let profiles = planner::current_profiles(paths)
        .into_iter()
        .filter(|profile| targets.contains(profile.target_id.as_str()))
        .collect::<Vec<_>>();
    let selected = planner::selected_component_ids(record, item);
    planner::plan(paths, snapshot, item, Some(&profiles), Some(&selected))
        .is_ok_and(|plan| super::status::selection_current(ledger, record, item, &plan))
}

pub(super) fn item_context(
    source_id: &str,
    local_id: &str,
) -> Result<(SystemPaths, ConfiguredSource, SourceSnapshot, CatalogItem), String> {
    let paths = SystemPaths::from_system()?;
    let cache = cache_base_dir()?;
    let config = config_base_dir()?;
    let source = source::configured_source(&config, source_id)?;
    let snapshot = source::load_current(&cache, &source)?
        .ok_or_else(|| format!("{} has no validated revision.", source.source_id))?;
    let item = snapshot
        .catalog
        .items
        .get(local_id)
        .cloned()
        .ok_or_else(|| format!("Unknown catalog item: {source_id}/{local_id}"))?;
    Ok((paths, source, snapshot, item))
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn skill_source(root: &Path) -> (ConfiguredSource, SourceSnapshot) {
        let source_root = root.join("source");
        fs::create_dir_all(source_root.join("skills/review")).expect("skill");
        fs::write(
            source_root.join("skills/review/SKILL.md"),
            "---\nname: review\ndescription: Review code\n---\nBody\n",
        )
        .expect("skill");
        fs::write(
            source_root.join("agent-plugins.json"),
            r#"{"version":2,"source":{"id":"acme","name":"Acme","description":"Test"},"packages":[{"id":"review","components":[{"kind":"skill","path":"skills/review"}]}]}"#,
        )
        .expect("manifest");
        let catalog = crate::catalog::read_manifest_catalog(&source_root, source::TEST_SOURCE_KEY)
            .expect("catalog");
        let mut source = ConfiguredSource::test_fixture(
            "acme",
            "https://nexus.example.com/repository/raw/sources/acme-latest.zip",
        );
        source.source_key = source::TEST_SOURCE_KEY.to_string();
        let snapshot = SourceSnapshot {
            definition: source.clone(),
            commit: "a".repeat(40),
            path: source_root,
            catalog,
        };
        (source, snapshot)
    }

    #[test]
    fn sync_repairs_missing_files_and_follows_detected_agents() {
        use crate::agent_profiles::{set_enabled, TargetId};
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        set_enabled(&paths, TargetId::Cursor, true).expect("cursor");
        let (source, snapshot) = skill_source(root.path());
        let item = snapshot.catalog.items["review"].clone();
        install::install_item_components_approved(&paths, &source, &snapshot, &item, false, None)
            .expect("install");
        let sources = vec![(source, snapshot)];
        let shared = paths.home.join(".agents/skills/acme-review");
        let claude = paths.home.join(".claude/skills/acme-review");

        assert!(repair_in(&paths, &sources).expect("noop").is_empty());
        fs::remove_dir_all(&shared).expect("delete");
        assert_eq!(
            repair_in(&paths, &sources).expect("repair"),
            std::slice::from_ref(&item.name)
        );
        assert!(shared.join("SKILL.md").is_file());

        set_enabled(&paths, TargetId::ClaudeCode, true).expect("claude");
        assert_eq!(
            extend_in(&paths, &sources).expect("extend"),
            std::slice::from_ref(&item.name)
        );
        assert!(claude.join("SKILL.md").is_file());
        assert!(extend_in(&paths, &sources).expect("noop").is_empty());

        set_enabled(&paths, TargetId::ClaudeCode, false).expect("claude gone");
        assert_eq!(
            extend_in(&paths, &sources).expect("release"),
            std::slice::from_ref(&item.name)
        );
        assert!(!claude.exists());
        assert!(shared.is_dir());
    }

    #[test]
    fn usage_events_name_the_apps_a_package_went_to() {
        use crate::agent_profiles::{set_enabled, TargetId};
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        for target in [
            TargetId::Cursor,
            TargetId::ClaudeCode,
            TargetId::ClaudeDesktop,
        ] {
            set_enabled(&paths, target, true).expect("detected");
        }
        let (source, snapshot) = skill_source(root.path());
        let item = snapshot.catalog.items["review"].clone();
        install::install_item_components_approved(&paths, &source, &snapshot, &item, false, None)
            .expect("install");
        let ledger = crate::executor::read_ledger(&paths).expect("ledger");

        // Claude Desktop is detected but takes no skills.
        assert_eq!(
            installed_agent_ids(&ledger, &item.id),
            ["claude-code", "cursor"]
        );
        assert!(installed_agent_ids(&ledger, "acme/other").is_empty());
    }

    #[test]
    fn a_retired_app_releases_what_it_was_given() {
        use crate::agent_profiles::{set_enabled, TargetId};
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        set_enabled(&paths, TargetId::Cursor, true).expect("cursor");
        set_enabled(&paths, TargetId::ClaudeCode, true).expect("claude");
        let (source, snapshot) = skill_source(root.path());
        let item = snapshot.catalog.items["review"].clone();
        install::install_item_components_approved(&paths, &source, &snapshot, &item, false, None)
            .expect("install");
        // Ledgers written before Microsoft 365 Copilot was dropped still name it.
        let ledger_path = paths.app_data().join("installations.json");
        let ledger = fs::read_to_string(&ledger_path).expect("ledger");
        fs::write(&ledger_path, ledger.replace("claude-code", "m365-copilot")).expect("retire");
        set_enabled(&paths, TargetId::ClaudeCode, false).expect("claude gone");
        let sources = vec![(source, snapshot)];

        assert_eq!(
            extend_in(&paths, &sources).expect("release"),
            std::slice::from_ref(&item.name)
        );
        assert!(!paths.home.join(".claude/skills/acme-review").exists());
        assert!(paths.home.join(".agents/skills/acme-review").is_dir());
    }

    fn mcp_source(root: &Path) -> (ConfiguredSource, SourceSnapshot) {
        let (source, mut snapshot) = skill_source(root);
        fs::create_dir_all(snapshot.path.join("mcp")).expect("mcp dir");
        fs::write(
            snapshot.path.join("mcp/database.json"),
            r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json","mcpServers":{"database":{"type":"stdio","command":"node","args":["server.js"]}}}"#,
        )
        .expect("mcp");
        fs::write(
            snapshot.path.join("agent-plugins.json"),
            r#"{"version":2,"source":{"id":"acme","name":"Acme","description":"Test"},"packages":[{"id":"review","components":[{"kind":"skill","id":"review","path":"skills/review"},{"kind":"mcpServer","id":"database","path":"mcp/database.json"}]}]}"#,
        )
        .expect("manifest");
        snapshot.catalog =
            crate::catalog::read_manifest_catalog(&snapshot.path, source::TEST_SOURCE_KEY)
                .expect("catalog");
        (source, snapshot)
    }

    #[test]
    fn an_approved_mcp_server_follows_its_agents_new_files_but_not_new_agents() {
        use crate::agent_profiles::{set_enabled, TargetId};
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        set_enabled(&paths, TargetId::GithubCopilot, true).expect("copilot");
        let (source, snapshot) = mcp_source(root.path());
        let item = snapshot.catalog.items["review"].clone();
        install::install_item_components_approved(&paths, &source, &snapshot, &item, true, None)
            .expect("install");
        let sources = vec![(source, snapshot)];
        assert!(extend_in(&paths, &sources).expect("noop").is_empty());

        // VS Code starts for the first time: Copilot's server goes to its file too.
        fs::create_dir_all(paths.config.join("Code/User")).expect("vscode");
        assert_eq!(
            extend_in(&paths, &sources).expect("vscode"),
            std::slice::from_ref(&item.name)
        );
        let vscode = fs::read_to_string(paths.config.join("Code/User/mcp.json")).expect("mcp.json");
        assert!(vscode.contains("acme-database"));

        // A newly detected agent gets the skill, never the server, without asking.
        set_enabled(&paths, TargetId::ClaudeCode, true).expect("claude");
        extend_in(&paths, &sources).expect("claude");
        assert!(paths.home.join(".claude/skills/acme-review").is_dir());
        assert!(!paths.home.join(".claude.json").exists());
    }

    #[test]
    fn install_all_skips_a_partial_install_that_is_in_place() {
        use crate::application::status::tests::{enable_cursor, two_component_snapshot};
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        enable_cursor(&paths);
        let (source, snapshot) = two_component_snapshot(root.path());
        let item = snapshot.catalog.items["tools"].clone();
        install::install_item_components_approved(
            &paths,
            &source,
            &snapshot,
            &item,
            false,
            Some(&["review".to_string()]),
        )
        .expect("install review");
        let ledger = crate::executor::read_ledger(&paths).expect("ledger");

        let entries = plan_entries(
            &paths,
            &ledger,
            &snapshot,
            snapshot.catalog.items.values(),
            BulkAction::Install,
        );
        assert_eq!(entries[0].status, ItemStatus::PartiallyInstalled);
        assert!(!entries[0].will_run);
    }

    #[test]
    fn a_partial_install_is_repaired_after_an_unrelated_upstream_change() {
        use crate::agent_profiles::{set_enabled, TargetId};
        use crate::application::status::tests::{enable_cursor, reread, two_component_snapshot};
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        enable_cursor(&paths);
        let (source, snapshot) = two_component_snapshot(root.path());
        let item = snapshot.catalog.items["tools"].clone();
        install::install_item_components_approved(
            &paths,
            &source,
            &snapshot,
            &item,
            false,
            Some(&["review".to_string()]),
        )
        .expect("install review");
        let change_docs = |snapshot: &SourceSnapshot, body: &str| {
            fs::write(
                snapshot.path.join("skills/docs/SKILL.md"),
                format!("---\nname: docs\ndescription: docs\nlicense: MIT\n---\n{body}\n"),
            )
            .expect("change docs");
            let snapshot = reread(snapshot);
            let item = snapshot.catalog.items["tools"].clone();
            (snapshot, item)
        };

        let (snapshot, item) = change_docs(&snapshot, "Changed");
        let sources = vec![(source.clone(), snapshot.clone())];
        set_enabled(&paths, TargetId::ClaudeCode, true).expect("claude");
        assert_eq!(
            extend_in(&paths, &sources).expect("extend"),
            std::slice::from_ref(&item.name)
        );
        assert!(paths
            .home
            .join(".claude/skills/skillbook-review/SKILL.md")
            .is_file());

        let (snapshot, item) = change_docs(&snapshot, "Changed again");
        let review = paths.home.join(".agents/skills/skillbook-review");
        fs::remove_dir_all(&review).expect("delete");

        let ledger = crate::executor::read_ledger(&paths).expect("ledger");
        let status = super::super::status::refined_item_status(
            &paths, &ledger, &snapshot, &item, None, None,
        );
        assert_eq!(status, ItemStatus::Missing);
        assert_eq!(
            super::super::status::component_status(
                &paths, &ledger, &snapshot, &item, "review", status, None,
            ),
            ItemStatus::Missing
        );
        let sources = vec![(source, snapshot)];
        assert_eq!(
            repair_in(&paths, &sources).expect("repair"),
            std::slice::from_ref(&item.name)
        );
        assert!(review.join("SKILL.md").is_file());
    }

    #[test]
    fn a_restored_package_list_forgets_what_was_uninstalled_after_the_backup() {
        use crate::agent_profiles::{set_enabled, TargetId};
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        set_enabled(&paths, TargetId::Cursor, true).expect("cursor");
        let (source, snapshot) = skill_source(root.path());
        let item = snapshot.catalog.items["review"].clone();
        install::install_item_components_approved(&paths, &source, &snapshot, &item, false, None)
            .expect("install");
        // The backup now holds the install; the uninstall is only in the live file.
        install::uninstall_item(&paths, &source, &item.id, false).expect("uninstall");
        fs::remove_file(paths.app_data().join("installations.json")).expect("lose the live file");
        let shared = paths.home.join(".agents/skills/acme-review");
        let sources = vec![(source, snapshot)];

        assert!(repair_in(&paths, &sources).expect("repair").is_empty());
        assert!(!shared.exists(), "an uninstalled package is not put back");
        let ledger = crate::executor::read_ledger(&paths).expect("ledger");
        assert!(!ledger.items.contains_key(&item.id));
    }

    #[test]
    fn wipe_app_state_removes_every_agent_plugins_state_root() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        for directory in paths.state_roots() {
            fs::create_dir_all(&directory).expect("state root");
            fs::write(directory.join("marker.txt"), "keep out").expect("marker");
        }
        fs::create_dir_all(paths.home.join("other")).expect("other");
        fs::write(paths.home.join("other/file.txt"), "keep").expect("other file");

        wipe_app_state(&paths).expect("wipe");

        for directory in paths.state_roots() {
            assert!(!directory.exists(), "{}", directory.display());
        }
        assert_eq!(
            fs::read_to_string(paths.home.join("other/file.txt")).expect("kept"),
            "keep"
        );
    }

    #[test]
    fn wipe_app_state_ignores_missing_state_roots() {
        let root = tempfile::tempdir().expect("root");
        wipe_app_state(&paths(root.path())).expect("wipe");
    }
}
