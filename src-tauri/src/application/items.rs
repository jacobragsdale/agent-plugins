use super::RuntimeState;
use crate::app_state::{BulkAction, BulkFailure, BulkPlan, BulkPlanEntry, BulkResult};
use crate::catalog::{CatalogComponentKind, CatalogItem};
use crate::executor::ContentState;
use crate::install::{self, ItemStatus, OperationOutcome, SourceRemovalPlan};
use crate::marketplace::{self, ClientEvent};
use crate::paths::SystemPaths;
use crate::planner;
use crate::resource::DesiredResource;
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
    let events = canonical_ids
        .iter()
        .map(|id| match kind {
            BulkAction::Install | BulkAction::Replace => {
                ClientEvent::install(id, marketplace_version(id), agents.clone())
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

pub(crate) async fn uninstall_item(
    runtime: &RuntimeState,
    source_id: &str,
    local_id: &str,
    component_id: Option<&str>,
) -> Result<OperationOutcome, String> {
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let config = config_base_dir()?;
    let source = source::configured_source(&config, source_id)?;
    let ids = component_id.map(|component_id| vec![component_id.to_string()]);
    let canonical_id = format!("{source_id}/{local_id}");
    let outcome =
        install::uninstall_item_components(&paths, &source, &canonical_id, ids.as_deref(), false)?;
    if ids.is_none() {
        report_operation(BulkAction::Uninstall, &[canonical_id]);
    }
    Ok(outcome)
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
        entries: bulk_entries(&paths, &ledger_state, &snapshot, action),
        source_id: source.source_id,
        action,
    })
}

fn bulk_entries(
    paths: &SystemPaths,
    ledger_state: &crate::ledger::InstallationLedger,
    snapshot: &SourceSnapshot,
    action: BulkAction,
) -> Vec<BulkPlanEntry> {
    snapshot
        .catalog
        .items
        .values()
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

pub(crate) async fn bulk_run(
    runtime: &RuntimeState,
    source_id: &str,
    action: BulkAction,
    trust_approved: bool,
) -> Result<BulkResult, String> {
    let plan = bulk_plan(runtime, source_id, action).await?;
    let entries = plan
        .entries
        .into_iter()
        .filter(|entry| entry.will_run)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Ok(BulkResult {
            completed: Vec::new(),
            failures: Vec::new(),
            backup_paths: Vec::new(),
        });
    }
    let _guard = runtime.operation_lock.lock().await;
    let paths = SystemPaths::from_system()?;
    let cache = cache_base_dir()?;
    let config = config_base_dir()?;
    let source = source::configured_source(&config, source_id)?;
    let snapshot = source::load_current(&cache, &source)?
        .ok_or_else(|| format!("{} has no validated revision.", source.source_id))?;
    // Each package is its own transaction, so one that fails its checks
    // leaves the others to finish and reports its own message.
    let mut result = BulkResult {
        completed: Vec::new(),
        failures: Vec::new(),
        backup_paths: Vec::new(),
    };
    for entry in entries {
        let outcome = match action {
            BulkAction::Install | BulkAction::Replace => {
                bulk_install(&paths, &source, &snapshot, &entry, action, trust_approved)
            }
            BulkAction::Uninstall => {
                install::uninstall_item_components(&paths, &source, &entry.id, None, false)
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
            if !mcp_entries_owned(&plan, &ledger) {
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
            let approved = mcp_entries_owned(&plan, &ledger);
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

/// Whether every MCP entry the plan writes is one the ledger already owns with
/// the same content, meaning the user approved it when installing.
fn mcp_entries_owned(
    plan: &crate::resource::OperationPlan,
    ledger: &crate::ledger::InstallationLedger,
) -> bool {
    plan.resources.values().all(|planned| {
        !matches!(planned.desired, DesiredResource::StructuredEntry(_))
            || ledger
                .resource_by_identity(&planned.desired.identity())
                .is_some_and(|owned| {
                    planned
                        .desired
                        .desired_digest()
                        .is_ok_and(|digest| digest == owned.desired_digest)
                })
    })
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
            let ids = planner::selected_component_ids(record, item)
                .into_iter()
                .filter(|id| {
                    retry
                        || item.components.iter().any(|component| {
                            component.id == *id && component.kind != CatalogComponentKind::McpServer
                        })
                })
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
            if ids.is_empty() || current == plan.bindings.keys().collect() {
                continue;
            }
            match install::install_item_components_approved(
                paths,
                source,
                snapshot,
                item,
                retry,
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
            onedrive_commercial: None,
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

        let entries = bulk_entries(&paths, &ledger, &snapshot, BulkAction::Install);
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
