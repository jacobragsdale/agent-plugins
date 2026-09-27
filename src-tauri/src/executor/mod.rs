//! The only filesystem writer for planned package resources.

use crate::catalog::{CatalogComponentKind, CatalogItem};
use crate::fs_retry;
use crate::install::OperationOutcome;
use crate::ledger::{
    self, BindingRecord, InstallationLedger, InstallationRecord, OwnedPath, OwnedPathKind,
    OwnedResource, OwnedStructuredEntry, OwnedTextBlock, ResourceRecord,
};
use crate::managed_documents;
use crate::paths::SystemPaths;
use crate::planner;
use crate::resource::normalized_path as normalize_path;
#[cfg(test)]
use crate::resource::StructuredFormat;
use crate::resource::{DesiredResource, DesiredStructuredEntry, OperationPlan};
use crate::source::{ConfiguredSource, SourceSnapshot};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

mod activate;
mod journal;
pub(crate) use journal::{UNDO_PENDING, UNDO_RUNNING};
mod matching;
mod stage;

use activate::{commit, persist_reset_ledger, update_document_digests_from_journal};
use journal::{
    cleanup_staging, read_ledger_raw, recover_unless_busy, JournalMutation, TransactionJournal,
    TransactionLock, JOURNAL_FILE,
};
use matching::plan_matches_ledger;
use stage::{identical_to_desired, mutation_backup, stage_changes, StageRequest};

pub(crate) use journal::recover;
pub(crate) use matching::{installation_state, plan_satisfied, resource_state, ContentState};

/// Staging names the executor and the ledger create next to their targets.
/// Backups (`.resource-previous-*`) are not listed: one may be the only copy
/// of a user's file while a rollback is pending.
const STAGING_PREFIXES: [&str; 4] = [
    ".resource-installing-",
    ".document-writing-",
    ".installations-writing-",
    ".resource-transaction-writing-",
];
const STALE_STAGING_AGE: Duration = Duration::from_secs(10 * 60);

/// The ledger for display. Reads never wait on recovery: a rollback that
/// cannot finish yet is retried by the next change or sync.
pub(crate) fn read_ledger(paths: &SystemPaths) -> Result<InstallationLedger, String> {
    if let Err(error) = recover_unless_busy(paths) {
        eprintln!("{error}");
    }
    read_ledger_raw(paths)
}

/// The ledger a change starts from, after recovery, and the lock that keeps
/// another process from changing it until this change commits. A ledger that
/// a newer version wrote is shown but never changed.
fn ledger_for_change(paths: &SystemPaths) -> Result<(TransactionLock, InstallationLedger), String> {
    let lock = TransactionLock::acquire(paths)?;
    recover(paths)?;
    let ledger = read_ledger_raw(paths)?;
    if ledger.read_only {
        return Err(ledger::NEWER_LEDGER_MESSAGE.to_string());
    }
    Ok((lock, ledger))
}

/// How a refusal to overwrite a person's edits reads.
pub(crate) const LOCAL_CHANGES: &str = "contains local changes";

fn protected_error(installation_id: &str, state: &ContentState, action: &str) -> String {
    match state {
        ContentState::Unknown(error) => {
            format!("{installation_id} could not be checked, so it cannot be {action}. {error}")
        }
        _ => format!("{installation_id} {LOCAL_CHANGES} and cannot be {action}."),
    }
}

/// Why nothing in `plan` can go to any detected app: each app's own reason.
fn unusable_here(item: &CatalogItem, plan: &OperationPlan) -> String {
    let reasons = plan
        .compatibility
        .iter()
        .filter_map(|report| match &report.capability {
            crate::resource::CapabilityResult::Unsupported { reason }
            | crate::resource::CapabilityResult::Blocked { reason, .. } => Some(reason.as_str()),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    format!(
        "None of the AI apps on this computer can use {}. {}",
        item.name,
        reasons.into_iter().collect::<Vec<_>>().join(" ")
    )
    .trim_end()
    .to_string()
}

/// Packages that skipped an agent because its settings file was unreadable
/// or locked. The next sync retries them.
// ponytail: in memory, so a restart forgets the retry; persist it if agents stay skipped.
fn skipped_agents() -> &'static Mutex<BTreeSet<String>> {
    static SKIPPED: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());
    &SKIPPED
}

/// Whether `installation_id` skipped an agent since the last call, clearing it.
pub(crate) fn take_skipped_agent(installation_id: &str) -> bool {
    skipped_agents()
        .lock()
        .is_ok_and(|mut skipped| skipped.remove(installation_id))
}

/// One agent's unreadable or locked settings file, or a skill folder that
/// cannot be written to, skips only the agents writing there: the bindings
/// that would write to it leave the plan, and each skipped place becomes a
/// warning. Fails only when nothing else is left to install.
fn skip_unusable_documents(
    plan: &mut OperationPlan,
    installation_id: &str,
) -> Result<Vec<String>, String> {
    let mut problems = BTreeMap::<PathBuf, (String, bool)>::new();
    for planned in plan.resources.values() {
        let Some(place) = written_in(&planned.desired) else {
            continue;
        };
        if problems.contains_key(place) {
            continue;
        }
        let problem = match &planned.desired {
            DesiredResource::StructuredEntry(desired) => {
                document_problem(desired, &planned.adapter_id)
            }
            _ => folder_problem(
                place,
                planned
                    .consumer_binding_ids
                    .iter()
                    .filter_map(|binding_id| plan.bindings.get(binding_id))
                    .map(|binding| binding.target_id.as_str()),
            ),
        };
        if let Some(problem) = problem {
            problems.insert(place.to_path_buf(), problem);
        }
    }
    if problems.is_empty() {
        return Ok(Vec::new());
    }
    let dropped_resources = plan
        .resources
        .values()
        .filter(|planned| {
            written_in(&planned.desired).is_some_and(|place| problems.contains_key(place))
        })
        .map(|planned| planned.id.clone())
        .collect::<BTreeSet<_>>();
    let dropped_bindings = plan
        .bindings
        .values()
        .filter(|binding| {
            binding
                .resource_ids
                .iter()
                .any(|resource_id| dropped_resources.contains(resource_id))
        })
        .map(|binding| binding.id.clone())
        .collect::<BTreeSet<_>>();
    let problems = problems.into_values().collect::<Vec<_>>();
    if dropped_bindings.len() == plan.bindings.len() {
        let texts = problems.iter().map(|(text, _)| text.as_str());
        return Err(texts.collect::<Vec<_>>().join(" "));
    }
    plan.bindings
        .retain(|binding_id, _| !dropped_bindings.contains(binding_id));
    plan.resources.retain(|_, planned| {
        planned
            .consumer_binding_ids
            .retain(|binding_id| !dropped_bindings.contains(binding_id));
        !planned.consumer_binding_ids.is_empty()
    });
    if let Ok(mut skipped) = skipped_agents().lock() {
        skipped.insert(installation_id.to_string());
    }
    // A busy file frees up on its own and the next sync retries it, so only an
    // unreadable one is worth a warning: someone has to fix that file.
    Ok(problems
        .into_iter()
        .filter(|(_, busy)| !busy)
        .map(|(text, _)| text)
        .collect())
}

/// Where a resource is written: its settings file, or the folder a whole
/// skill goes into.
fn written_in(desired: &DesiredResource) -> Option<&Path> {
    match desired {
        DesiredResource::StructuredEntry(entry) => Some(&entry.document_path),
        DesiredResource::Path(path) => path.path.parent(),
        DesiredResource::TextBlock(_) => None,
    }
}

/// A skill folder that cannot be written to, such as a synced folder that a
/// policy or a sync error keeps read-only. It stays so until someone fixes it,
/// so it is a warning rather than a quiet retry.
fn folder_problem<'a>(
    folder: &Path,
    targets: impl IntoIterator<Item = &'a str>,
) -> Option<(String, bool)> {
    let probe = folder.join(format!(".agent-plugins-write-test-{}", std::process::id()));
    let written = fs::create_dir_all(folder).and_then(|()| fs::write(&probe, b"ok"));
    let _ = fs::remove_file(&probe);
    let error = written.err()?;
    let apps = managed_documents::app_names(targets);
    let why = if error.kind() == std::io::ErrorKind::PermissionDenied {
        format!(
            "Agent Plugins isn't allowed to write to {}",
            folder.display()
        )
    } else {
        format!(
            "Agent Plugins could not write to {}: {}",
            folder.display(),
            fs_retry::plain(&error).trim_end_matches('.')
        )
    };
    Some((format!("{apps} did not get the package: {why}."), false))
}

/// Why an agent's settings file cannot take the entry now, and whether that is
/// only because it is busy.
fn document_problem(desired: &DesiredStructuredEntry, adapter_id: &str) -> Option<(String, bool)> {
    let path = &desired.document_path;
    // Before reading: an app that holds the file without sharing makes the
    // read fail too, and a busy file is no reason to tell anyone to delete it.
    if document_locked(path) {
        let app = managed_documents::app_names([adapter_id]);
        return Some((
            format!(
                "{app} is using its settings file {}, so the package was not added to {app}. Close {app}; Agent Plugins tries again the next time it checks for updates.",
                path.display()
            ),
            true,
        ));
    }
    managed_documents::read_or_empty(path, desired.format)
        .and_then(|contents| {
            managed_documents::entry_value(&contents, desired.format, &desired.key_path)
        })
        .err()
        .map(|error| {
            (
                managed_documents::document_error(path, [adapter_id], &error),
                false,
            )
        })
}

/// A settings file another process holds without sharing, which a rename
/// cannot replace until that app closes.
#[cfg(windows)]
fn document_locked(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    // The rename that replaces it needs delete access too: an app holding it
    // without FILE_SHARE_DELETE passes a write probe and still fails the rename.
    const DELETE: u32 = 0x0001_0000;
    let busy = |options: &fs::OpenOptions| {
        options
            .open(path)
            .is_err_and(|error| matches!(error.raw_os_error(), Some(32 | 33)))
    };
    busy(fs::OpenOptions::new().append(true)) || busy(fs::OpenOptions::new().access_mode(DELETE))
}

#[cfg(not(windows))]
fn document_locked(_path: &Path) -> bool {
    false
}

/// How long a backup of a person's edited copy is kept. It may hold a whole
/// settings file with their secrets in it, so it does not stay forever.
const BACKUP_KEEP: std::time::Duration = std::time::Duration::from_secs(30 * 86_400);

/// Removes backups older than `BACKUP_KEEP`, oldest first. Each backup is one
/// folder per change under `~/.agents/.agent-plugins-backups`.
pub(crate) fn prune_backups(paths: &SystemPaths) {
    let root = paths.home.join(".agents").join(".agent-plugins-backups");
    let Ok(entries) = fs::read_dir(&root) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > BACKUP_KEEP);
        if old {
            if let Err(error) = remove_any(&entry.path()) {
                eprintln!("{error}");
            }
        }
    }
}

/// Removes staging that an interrupted run left next to agent files and in
/// Agent Plugins' own data folder. Only names older than a few minutes are
/// touched, and nothing while a transaction is pending.
pub(crate) fn sweep_stale_staging(paths: &SystemPaths) {
    if path_entry_exists(&paths.app_data().join(JOURNAL_FILE)) {
        return;
    }
    let mut directories = crate::adapters::managed_skill_roots(paths)
        .into_iter()
        .collect::<BTreeSet<_>>();
    directories.insert(paths.app_data());
    if let Ok(ledger) = read_ledger_raw(paths) {
        for resource in ledger.resources.values() {
            let owned = match &resource.owned {
                OwnedResource::Path(owned) => &owned.path,
                OwnedResource::StructuredEntry(owned) => &owned.document_path,
                OwnedResource::TextBlock(owned) => &owned.document_path,
            };
            if let Some(parent) = Path::new(owned).parent() {
                directories.insert(parent.to_path_buf());
            }
        }
    }
    for directory in directories {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !STAGING_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
            {
                continue;
            }
            let stale = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > STALE_STAGING_AGE);
            if stale {
                if let Err(error) = remove_any(&entry.path()) {
                    eprintln!("{error}");
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn install(
    paths: &SystemPaths,
    source: &ConfiguredSource,
    snapshot: &SourceSnapshot,
    item: &CatalogItem,
    replace_unmanaged: bool,
    trust_approved: bool,
) -> Result<OperationOutcome, String> {
    install_components(
        paths,
        source,
        snapshot,
        item,
        replace_unmanaged,
        trust_approved,
        None,
    )
}

pub(crate) fn install_components(
    paths: &SystemPaths,
    source: &ConfiguredSource,
    snapshot: &SourceSnapshot,
    item: &CatalogItem,
    replace_unmanaged: bool,
    trust_approved: bool,
    component_ids: Option<&[String]>,
) -> Result<OperationOutcome, String> {
    let (_lock, ledger_state) = ledger_for_change(paths)?;
    let existing = ledger_state.items.get(&item.id).cloned();
    let operate_on = resolve_operate_on(item, component_ids)?;
    let mut plan = planner::plan(paths, snapshot, item, None, Some(&operate_on))?;
    if planner::needs_approval(item, &plan, &ledger_state) && !trust_approved {
        return Err(format!(
            "{} contains an MCP server and requires explicit Tier 3 approval.",
            item.id
        ));
    }
    let warnings = skip_unusable_documents(&mut plan, &item.id)?;
    // A component no AI app here can use would be recorded as installed while
    // nothing uses it. Say which apps cannot, and why, instead.
    let newly_selected = existing.as_ref().is_none_or(|record| {
        let selected = planner::selected_component_ids(record, item);
        !operate_on.iter().all(|id| selected.contains(id))
    });
    if newly_selected && plan.bindings.is_empty() {
        return Err(unusable_here(item, &plan));
    }

    // Replacing an installed package restores the published content; the
    // user's changed copies are moved to kept backups.
    let restore = replace_unmanaged && existing.is_some();
    if let Some(record) = &existing {
        if record.source_key != source.source_key {
            return Err(format!("{} is owned by a different source.", item.id));
        }
        let state = installation_state(paths, &ledger_state, &item.id, Some(&operate_on));
        if state.is_protected() && !restore {
            return Err(protected_error(&item.id, &state, "updated"));
        }
        let already_selected = operate_on.iter().all(|component_id| {
            planner::selected_component_ids(record, item).contains(component_id)
        });
        // Bindings the plan no longer produces belong to agents that are no
        // longer detected; applying the plan releases them.
        let stale_bindings = record.binding_ids.iter().any(|binding_id| {
            ledger_state
                .bindings
                .get(binding_id)
                .is_some_and(|binding| {
                    operate_on.contains(&binding.component_id)
                        && !plan.bindings.contains_key(binding_id)
                })
        });
        if state == ContentState::Match
            && record.item_digest == item.digest
            && already_selected
            && !stale_bindings
            && plan_matches_ledger(&ledger_state, &plan)?
        {
            return Err(format!("{} is already installed.", item.id));
        }
    }
    planner::preflight_installed_conflicts(&ledger_state, source, item, &plan)?;
    validate_plan_paths(&plan)?;

    let mut next = ledger_state.clone();
    let removed = if let Some(record) = &existing {
        // Components the publisher removed are released with any change.
        let mut detach = operate_on.clone();
        detach.extend(
            binding_component_ids(&ledger_state, record)
                .into_iter()
                .filter(|id| !item.components.iter().any(|component| component.id == *id)),
        );
        detach_components(&mut next, &item.id, &detach)
    } else {
        Vec::new()
    };
    let replaced_identities = removed
        .iter()
        .map(|resource| resource.identity.clone())
        .collect::<BTreeSet<_>>();
    preflight_new_resources(&next, &plan, &replaced_identities, replace_unmanaged)?;

    for binding in plan.bindings.values() {
        next.bindings.insert(
            binding.id.clone(),
            BindingRecord {
                id: binding.id.clone(),
                installation_id: binding.installation_id.clone(),
                component_id: binding.component_id.clone(),
                target_id: binding.target_id.clone(),
                dialect_id: binding.dialect_id.clone(),
                scope: binding.scope.clone(),
                capability: binding.capability.clone(),
                resource_ids: binding.resource_ids.clone(),
            },
        );
    }

    let transaction_id = transaction_id(&item.id);
    let (journal, installed, backup_paths) = stage_changes(&StageRequest {
        paths,
        transaction_id: &transaction_id,
        plan: &plan,
        removed: &removed,
        remaining_ledger: &next,
        replace_unmanaged,
        force_modified: restore,
    })?;
    for mut record in installed {
        if let Some(existing_resource) = next.resource_by_identity_mut(&record.identity) {
            if existing_resource.desired_digest != record.desired_digest {
                cleanup_staging(&journal);
                return Err(format!(
                    "{} has conflicting desired content.",
                    record.identity
                ));
            }
            for consumer in record.consumer_binding_ids.drain(..) {
                if !existing_resource.consumer_binding_ids.contains(&consumer) {
                    existing_resource.consumer_binding_ids.push(consumer);
                }
            }
            existing_resource.consumer_binding_ids.sort();
        } else {
            next.resources.insert(record.id.clone(), record);
        }
    }
    update_document_digests_from_journal(&mut next, &journal)?;
    let selected_component_ids =
        merged_selected_component_ids(item, existing.as_ref(), component_ids, &operate_on);
    let mut binding_ids = next
        .items
        .get(&item.id)
        .map(|record| record.binding_ids.clone())
        .unwrap_or_default();
    binding_ids.extend(plan.bindings.keys().cloned());
    binding_ids.sort();
    binding_ids.dedup();
    let destination = compatibility_destination(&next, &plan, paths)
        .inspect_err(|_| cleanup_staging(&journal))?;
    next.items.insert(
        item.id.clone(),
        InstallationRecord {
            source_key: source.source_key.clone(),
            source_url: source.url().to_string(),
            source_id: source.source_id.clone(),
            local_id: item.local_id.clone(),
            commit: snapshot.commit.clone(),
            item_digest: item.digest.clone(),
            name: item.name.clone(),
            description: item.description.clone(),
            disable_model_invocation: item.disable_model_invocation,
            source: item.source.clone(),
            destination,
            manifest_version: item.manifest_version,
            component_kind: "package".to_string(),
            binding_ids,
            selected_component_ids,
            conflicts_with: item.conflicts_with.clone(),
        },
    );
    next.last_transaction_id = Some(transaction_id);
    commit(paths, &journal, &next)?;
    Ok(OperationOutcome {
        backup_paths: backup_paths
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        warnings,
    })
}

pub(crate) struct BatchInstall<'a> {
    pub(crate) source: &'a ConfiguredSource,
    pub(crate) snapshot: &'a SourceSnapshot,
    pub(crate) item: &'a CatalogItem,
    pub(crate) replace_unmanaged: bool,
}

pub(crate) fn install_batch(
    paths: &SystemPaths,
    requests: &[BatchInstall<'_>],
    trust_approved: bool,
) -> Result<OperationOutcome, String> {
    if requests.is_empty() {
        return Ok(OperationOutcome::default());
    }
    let (_lock, original) = ledger_for_change(paths)?;
    let mut next = original.clone();
    let mut warnings = Vec::new();
    let batch_ids = requests
        .iter()
        .map(|request| request.item.id.clone())
        .collect::<BTreeSet<_>>();
    let mut combined = OperationPlan::default();
    let mut item_plans = BTreeMap::new();
    let mut removed = Vec::new();
    for request in requests {
        let item = request.item;
        let selected = original
            .items
            .get(&item.id)
            .map(|record| planner::selected_component_ids(record, item));
        let mut plan = planner::plan(paths, request.snapshot, item, None, selected.as_deref())?;
        if planner::needs_approval(item, &plan, &original) && !trust_approved {
            return Err(format!(
                "{} contains an MCP server and requires explicit Tier 3 approval.",
                item.id
            ));
        }
        warnings.extend(skip_unusable_documents(&mut plan, &item.id)?);
        if item
            .conflicts_with
            .iter()
            .any(|conflict| batch_ids.contains(conflict))
        {
            return Err(format!(
                "{} conflicts with another package in this batch.",
                item.id
            ));
        }
        if let Some(existing) = original.items.get(&item.id) {
            if request.replace_unmanaged {
                return Err(format!(
                    "{} is already managed; use the normal update operation.",
                    item.id
                ));
            }
            if existing.source_key != request.source.source_key {
                return Err(format!("{} is owned by a different source.", item.id));
            }
            let state = installation_state(paths, &original, &item.id, None);
            if state.is_protected() {
                return Err(protected_error(&item.id, &state, "updated"));
            }
        }
        planner::preflight_installed_conflicts(&original, request.source, item, &plan)?;
        validate_plan_paths(&plan)?;
        removed.extend(detach_installation(&mut next, &item.id));
        merge_plan(&mut combined, &plan)?;
        item_plans.insert(item.id.clone(), plan);
    }
    validate_plan_paths(&combined)?;
    let replaced_identities = removed
        .iter()
        .map(|resource| resource.identity.clone())
        .collect::<BTreeSet<_>>();
    preflight_new_resources(
        &next,
        &combined,
        &replaced_identities,
        requests.iter().any(|request| request.replace_unmanaged),
    )?;
    for binding in combined.bindings.values() {
        next.bindings.insert(
            binding.id.clone(),
            BindingRecord {
                id: binding.id.clone(),
                installation_id: binding.installation_id.clone(),
                component_id: binding.component_id.clone(),
                target_id: binding.target_id.clone(),
                dialect_id: binding.dialect_id.clone(),
                scope: binding.scope.clone(),
                capability: binding.capability.clone(),
                resource_ids: binding.resource_ids.clone(),
            },
        );
    }
    let transaction_id = transaction_id("batch-install");
    let (journal, installed, backup_paths) = stage_changes(&StageRequest {
        paths,
        transaction_id: &transaction_id,
        plan: &combined,
        removed: &removed,
        remaining_ledger: &next,
        replace_unmanaged: requests.iter().any(|request| request.replace_unmanaged),
        force_modified: false,
    })?;
    for mut resource in installed {
        if let Some(existing) = next.resource_by_identity_mut(&resource.identity) {
            for consumer in resource.consumer_binding_ids.drain(..) {
                if !existing.consumer_binding_ids.contains(&consumer) {
                    existing.consumer_binding_ids.push(consumer);
                }
            }
            existing.consumer_binding_ids.sort();
        } else {
            next.resources.insert(resource.id.clone(), resource);
        }
    }
    update_document_digests_from_journal(&mut next, &journal)?;
    for request in requests {
        let item = request.item;
        let plan = &item_plans[&item.id];
        let mut record =
            installation_record(paths, &next, plan, request.source, request.snapshot, item)
                .inspect_err(|_| cleanup_staging(&journal))?;
        record.selected_component_ids = original
            .items
            .get(&item.id)
            .map(|existing| existing.selected_component_ids.clone())
            .unwrap_or_default();
        next.items.insert(item.id.clone(), record);
    }
    next.last_transaction_id = Some(transaction_id);
    commit(paths, &journal, &next)?;
    Ok(OperationOutcome {
        backup_paths: backup_paths
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        warnings,
    })
}

pub(crate) fn uninstall_batch(
    paths: &SystemPaths,
    source: &ConfiguredSource,
    installation_ids: &[String],
    force_modified: bool,
) -> Result<OperationOutcome, String> {
    if installation_ids.is_empty() {
        return Ok(OperationOutcome::default());
    }
    let (_lock, original) = ledger_for_change(paths)?;
    for installation_id in installation_ids {
        let record = original
            .items
            .get(installation_id)
            .ok_or_else(|| format!("{installation_id} is not installed."))?;
        if record.source_key != source.source_key {
            return Err(format!("{installation_id} is owned by a different source."));
        }
        let state = installation_state(paths, &original, installation_id, None);
        if !force_modified && state.is_protected() {
            return Err(protected_error(installation_id, &state, "removed"));
        }
    }
    let mut next = original;
    let mut removed = Vec::new();
    for installation_id in installation_ids {
        removed.extend(detach_installation(&mut next, installation_id));
    }
    let transaction_id = transaction_id("batch-uninstall");
    let (journal, _, backup_paths) = stage_changes(&StageRequest {
        paths,
        transaction_id: &transaction_id,
        plan: &OperationPlan::default(),
        removed: &removed,
        remaining_ledger: &next,
        replace_unmanaged: false,
        force_modified,
    })?;
    update_document_digests_from_journal(&mut next, &journal)?;
    next.last_transaction_id = Some(transaction_id);
    commit(paths, &journal, &next)?;
    Ok(OperationOutcome {
        backup_paths: backup_paths
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        ..OperationOutcome::default()
    })
}

pub(crate) fn reset_source(
    paths: &SystemPaths,
    source: &ConfiguredSource,
    snapshot: Option<&SourceSnapshot>,
) -> Result<OperationOutcome, String> {
    let _lock = TransactionLock::acquire(paths)?;
    let original = read_ledger(paths)?;
    let catalog_ids = snapshot
        .map(|snapshot| {
            snapshot
                .catalog
                .items
                .keys()
                .cloned()
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let installation_ids = crate::install::source_reset_ids(&original, source, &catalog_ids);
    let (next, mut backup_paths) =
        force_detach_installations(paths, &installation_ids, "source-reset")?;
    backup_paths.extend(remove_leftover_source_skills(
        paths,
        source,
        snapshot,
        &next,
        "source-reset",
    ));
    backup_paths.sort();
    backup_paths.dedup();
    Ok(OperationOutcome {
        backup_paths,
        ..OperationOutcome::default()
    })
}

pub(crate) fn reset_app(
    paths: &SystemPaths,
    sources: &[(ConfiguredSource, Option<SourceSnapshot>)],
) -> Result<OperationOutcome, String> {
    let _lock = TransactionLock::acquire(paths)?;
    // An undo still pending holds the person's originals beside their files;
    // wiping its journal would leave them hidden there for good.
    recover(paths)?;
    let mut backup_paths = Vec::new();
    // Reset is the way out of a damaged or newer ledger, so without a usable
    // one it removes what it can find and leaves the rest to the state wipe.
    let usable = read_ledger(paths);
    if !usable.as_ref().is_ok_and(|ledger| !ledger.read_only) {
        let resources = usable
            .map(|ledger| ledger.resources.into_values().collect::<Vec<_>>())
            .unwrap_or_default();
        backup_paths.extend(best_effort_remove(paths, &resources, "app-reset"));
        for (source, snapshot) in sources {
            backup_paths.extend(remove_leftover_source_skills(
                paths,
                source,
                snapshot.as_ref(),
                &InstallationLedger::default(),
                "app-reset",
            ));
        }
        return Ok(OperationOutcome {
            backup_paths,
            ..OperationOutcome::default()
        });
    }
    for (source, snapshot) in sources {
        backup_paths.extend(reset_source(paths, source, snapshot.as_ref())?.backup_paths);
    }
    let remaining = read_ledger(paths)?
        .items
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    let (_, leftover) = force_detach_installations(paths, &remaining, "app-reset")?;
    backup_paths.extend(leftover);
    backup_paths.sort();
    backup_paths.dedup();
    Ok(OperationOutcome {
        backup_paths,
        ..OperationOutcome::default()
    })
}

fn force_detach_installations(
    paths: &SystemPaths,
    installation_ids: &[String],
    label: &str,
) -> Result<(InstallationLedger, Vec<String>), String> {
    let mut next = read_ledger_raw(paths)?;
    let mut removed = Vec::new();
    for installation_id in installation_ids {
        removed.extend(detach_installation(&mut next, installation_id));
    }
    let mut backup_paths = Vec::new();
    if !removed.is_empty() {
        match commit_forced_removal(paths, &mut next, &removed, label) {
            Ok(backups) => backup_paths.extend(backups),
            Err(_) => {
                persist_reset_ledger(paths, &mut next, label)?;
                backup_paths.extend(best_effort_remove(paths, &removed, label));
            }
        }
    }
    Ok((next, backup_paths))
}

fn commit_forced_removal(
    paths: &SystemPaths,
    next: &mut InstallationLedger,
    removed: &[ResourceRecord],
    label: &str,
) -> Result<Vec<String>, String> {
    let transaction_id = transaction_id(label);
    let (journal, _, backup_paths) = stage_changes(&StageRequest {
        paths,
        transaction_id: &transaction_id,
        plan: &OperationPlan::default(),
        removed,
        remaining_ledger: next,
        replace_unmanaged: false,
        force_modified: true,
    })?;
    update_document_digests_from_journal(next, &journal)?;
    next.last_transaction_id = Some(transaction_id);
    commit(paths, &journal, next)?;
    Ok(backup_paths
        .into_iter()
        .map(|path| path.display().to_string())
        .collect())
}

fn best_effort_remove(paths: &SystemPaths, removed: &[ResourceRecord], label: &str) -> Vec<String> {
    let mut backup_paths = Vec::new();
    for resource in removed {
        match &resource.owned {
            OwnedResource::Path(owned) => {
                if let Some(backup) = best_effort_remove_path(paths, Path::new(&owned.path), label)
                {
                    backup_paths.push(backup);
                }
            }
            OwnedResource::StructuredEntry(owned) => {
                let _ = strip_structured_entry(Path::new(&owned.document_path), owned);
            }
            OwnedResource::TextBlock(owned) => {
                let _ = strip_text_block(Path::new(&owned.document_path), owned);
            }
        }
    }
    backup_paths
}

fn best_effort_remove_path(paths: &SystemPaths, target: &Path, label: &str) -> Option<String> {
    if !path_entry_exists(target) {
        return None;
    }
    if let Ok(backup) = mutation_backup(paths, label, target, true) {
        if fs_retry::rename(target, &backup).is_ok() {
            return Some(backup.display().to_string());
        }
    }
    let _ = remove_any(target);
    None
}

fn strip_structured_entry(path: &Path, owned: &OwnedStructuredEntry) -> Result<(), String> {
    let original = managed_documents::read_or_empty(path, owned.format)?;
    let updated = managed_documents::remove_entries(
        &original,
        owned.format,
        std::slice::from_ref(&owned.key_path),
    )?;
    if updated != original {
        fs_retry::replace_file(path, &updated)
            .map_err(|error| format!("Could not update {}: {error}", path.display()))?;
    }
    Ok(())
}

fn strip_text_block(path: &Path, owned: &OwnedTextBlock) -> Result<(), String> {
    let original = fs::read(path).unwrap_or_default();
    let updated =
        managed_documents::remove_text_blocks(&original, std::slice::from_ref(&owned.marker_id))?;
    if updated != original {
        fs_retry::replace_file(path, &updated)
            .map_err(|error| format!("Could not update {}: {error}", path.display()))?;
    }
    Ok(())
}

fn remove_leftover_source_skills(
    paths: &SystemPaths,
    source: &ConfiguredSource,
    snapshot: Option<&SourceSnapshot>,
    remaining: &InstallationLedger,
    label: &str,
) -> Vec<String> {
    let owned_paths = remaining
        .resources
        .values()
        .filter_map(|resource| match &resource.owned {
            OwnedResource::Path(owned) => Some(normalize_path(Path::new(&owned.path))),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    let mut names = BTreeSet::new();
    if let Some(snapshot) = snapshot {
        for item in snapshot.catalog.items.values() {
            for component in &item.components {
                if component.kind == CatalogComponentKind::Skill {
                    names.insert(component.effective_name.clone());
                }
            }
        }
    }
    let prefix = format!("{}-", source.source_id);
    let mut backup_paths = Vec::new();
    for root in crate::adapters::managed_skill_roots(paths) {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.starts_with(&prefix) && !names.contains(name) {
                continue;
            }
            let target = entry.path();
            if owned_paths.contains(&normalize_path(&target)) {
                continue;
            }
            if let Some(backup) = best_effort_remove_path(paths, &target, label) {
                backup_paths.push(backup);
            }
        }
    }
    backup_paths
}

/// Stops managing an installed package and leaves its files where they are,
/// so a person keeps their own edited copy. Nothing is removed or backed up;
/// the package can be installed again later like any other.
pub(crate) fn forget(paths: &SystemPaths, installation_id: &str) -> Result<(), String> {
    let (_lock, mut ledger_state) = ledger_for_change(paths)?;
    if !ledger_state.items.contains_key(installation_id) {
        return Err(format!("{installation_id} is not installed."));
    }
    detach_installation(&mut ledger_state, installation_id);
    ledger::write(&paths.app_data(), &ledger_state)
}

#[cfg(test)]
pub(crate) fn uninstall(
    paths: &SystemPaths,
    source: &ConfiguredSource,
    installation_id: &str,
    force_modified: bool,
) -> Result<OperationOutcome, String> {
    uninstall_components(paths, source, installation_id, None, force_modified)
}

pub(crate) fn uninstall_components(
    paths: &SystemPaths,
    source: &ConfiguredSource,
    installation_id: &str,
    component_ids: Option<&[String]>,
    force_modified: bool,
) -> Result<OperationOutcome, String> {
    let (_lock, ledger_state) = ledger_for_change(paths)?;
    let record = ledger_state
        .items
        .get(installation_id)
        .ok_or_else(|| format!("{installation_id} is not installed."))?;
    if record.source_key != source.source_key {
        return Err(format!("{installation_id} is owned by a different source."));
    }
    let operate_on = match component_ids {
        None | Some([]) => None,
        Some(ids) => {
            let installed = binding_component_ids(&ledger_state, record);
            if ids.iter().all(|id| installed.contains(id))
                && installed.iter().all(|id| ids.contains(id))
            {
                None
            } else {
                Some(ids.to_vec())
            }
        }
    };
    let state = installation_state(paths, &ledger_state, installation_id, operate_on.as_deref());
    if !force_modified && state.is_protected() {
        return Err(protected_error(installation_id, &state, "uninstalled"));
    }
    let mut next = ledger_state.clone();
    let removed = match &operate_on {
        Some(ids) => detach_components(&mut next, installation_id, ids),
        None => detach_installation(&mut next, installation_id),
    };
    let transaction_id = transaction_id(installation_id);
    let (journal, _, backup_paths) = stage_changes(&StageRequest {
        paths,
        transaction_id: &transaction_id,
        plan: &OperationPlan::default(),
        removed: &removed,
        remaining_ledger: &next,
        replace_unmanaged: false,
        force_modified,
    })?;
    update_document_digests_from_journal(&mut next, &journal)?;
    next.last_transaction_id = Some(transaction_id);
    commit(paths, &journal, &next)?;
    Ok(OperationOutcome {
        backup_paths: backup_paths
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        ..OperationOutcome::default()
    })
}

fn merge_plan(combined: &mut OperationPlan, plan: &OperationPlan) -> Result<(), String> {
    for binding in plan.bindings.values() {
        combined.add_binding(binding.clone())?;
    }
    for planned in plan.resources.values() {
        if let Some(existing) = combined.resources.get_mut(&planned.id) {
            if existing.desired.identity() != planned.desired.identity()
                || existing.desired.desired_digest()? != planned.desired.desired_digest()?
            {
                return Err(format!(
                    "Conflicting batch content targets {}.",
                    planned.desired.identity()
                ));
            }
            for consumer in &planned.consumer_binding_ids {
                if !existing.consumer_binding_ids.contains(consumer) {
                    existing.consumer_binding_ids.push(consumer.clone());
                }
            }
            existing.consumer_binding_ids.sort();
        } else {
            combined
                .resources
                .insert(planned.id.clone(), planned.clone());
        }
    }
    combined.compatibility.extend(plan.compatibility.clone());
    combined.warnings.extend(plan.warnings.clone());
    combined.warnings.sort();
    combined.warnings.dedup();
    Ok(())
}

fn installation_record(
    paths: &SystemPaths,
    ledger: &InstallationLedger,
    plan: &OperationPlan,
    source: &ConfiguredSource,
    snapshot: &SourceSnapshot,
    item: &CatalogItem,
) -> Result<InstallationRecord, String> {
    Ok(InstallationRecord {
        source_key: source.source_key.clone(),
        source_url: source.url().to_string(),
        source_id: source.source_id.clone(),
        local_id: item.local_id.clone(),
        commit: snapshot.commit.clone(),
        item_digest: item.digest.clone(),
        name: item.name.clone(),
        description: item.description.clone(),
        disable_model_invocation: item.disable_model_invocation,
        source: item.source.clone(),
        destination: compatibility_destination(ledger, plan, paths)?,
        manifest_version: item.manifest_version,
        component_kind: "package".to_string(),
        binding_ids: plan.bindings.keys().cloned().collect(),
        selected_component_ids: Vec::new(),
        conflicts_with: item.conflicts_with.clone(),
    })
}

fn resolve_operate_on(
    item: &CatalogItem,
    component_ids: Option<&[String]>,
) -> Result<Vec<String>, String> {
    match component_ids {
        None | Some([]) => Ok(planner::package_component_ids(item)),
        Some(ids) => {
            for component_id in ids {
                planner::validate_component_id(item, component_id)?;
            }
            Ok(ids.to_vec())
        }
    }
}

fn merged_selected_component_ids(
    item: &CatalogItem,
    existing: Option<&InstallationRecord>,
    requested: Option<&[String]>,
    operate_on: &[String],
) -> Vec<String> {
    if requested.is_none() {
        return Vec::new();
    }
    let all = planner::package_component_ids(item);
    let mut selected = match existing {
        Some(record) => planner::selected_component_ids(record, item),
        None => Vec::new(),
    };
    for component_id in operate_on {
        if !selected.contains(component_id) {
            selected.push(component_id.clone());
        }
    }
    if selected.len() == all.len() && all.iter().all(|id| selected.contains(id)) {
        Vec::new()
    } else {
        selected
    }
}

fn binding_component_ids(ledger: &InstallationLedger, record: &InstallationRecord) -> Vec<String> {
    record
        .binding_ids
        .iter()
        .filter_map(|binding_id| {
            ledger
                .bindings
                .get(binding_id)
                .map(|binding| binding.component_id.clone())
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn detach_components(
    ledger: &mut InstallationLedger,
    installation_id: &str,
    component_ids: &[String],
) -> Vec<ResourceRecord> {
    let Some(record) = ledger.items.get(installation_id) else {
        return Vec::new();
    };
    let component_set = component_ids.iter().cloned().collect::<BTreeSet<_>>();
    let binding_ids = record
        .binding_ids
        .iter()
        .filter(|binding_id| {
            ledger
                .bindings
                .get(*binding_id)
                .is_some_and(|binding| component_set.contains(&binding.component_id))
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    for binding_id in &binding_ids {
        ledger.bindings.remove(binding_id);
    }
    let (remaining_binding_ids, was_all) =
        if let Some(record) = ledger.items.get_mut(installation_id) {
            record
                .binding_ids
                .retain(|binding_id| !binding_ids.contains(binding_id));
            let remaining = record.binding_ids.clone();
            let was_all = record.selected_component_ids.is_empty();
            record
                .selected_component_ids
                .retain(|component_id| !component_set.contains(component_id));
            (remaining, was_all)
        } else {
            (Vec::new(), false)
        };
    if was_all {
        let remaining_components = remaining_binding_ids
            .iter()
            .filter_map(|binding_id| {
                ledger
                    .bindings
                    .get(binding_id)
                    .map(|binding| binding.component_id.clone())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if let Some(record) = ledger.items.get_mut(installation_id) {
            record.selected_component_ids = remaining_components;
        }
    }
    if remaining_binding_ids.is_empty() {
        ledger.items.remove(installation_id);
    }
    let mut orphan_ids = Vec::new();
    for (resource_id, resource) in &mut ledger.resources {
        resource
            .consumer_binding_ids
            .retain(|consumer| !binding_ids.contains(consumer));
        if resource.consumer_binding_ids.is_empty() {
            orphan_ids.push(resource_id.clone());
        }
    }
    orphan_ids
        .into_iter()
        .filter_map(|resource_id| ledger.resources.remove(&resource_id))
        .collect()
}

fn detach_installation(
    ledger: &mut InstallationLedger,
    installation_id: &str,
) -> Vec<ResourceRecord> {
    let Some(record) = ledger.items.remove(installation_id) else {
        return Vec::new();
    };
    let binding_ids = record.binding_ids.into_iter().collect::<BTreeSet<_>>();
    for binding_id in &binding_ids {
        ledger.bindings.remove(binding_id);
    }
    let mut orphan_ids = Vec::new();
    for (resource_id, resource) in &mut ledger.resources {
        resource
            .consumer_binding_ids
            .retain(|consumer| !binding_ids.contains(consumer));
        if resource.consumer_binding_ids.is_empty() {
            orphan_ids.push(resource_id.clone());
        }
    }
    orphan_ids
        .into_iter()
        .filter_map(|resource_id| ledger.resources.remove(&resource_id))
        .collect()
}

/// Whether installing `plan` has to replace a skill folder Agent Plugins did
/// not put there, which only Replace does. Settings entries are left to the
/// install itself: checking them would read every app's settings file for
/// every card, and a same-named entry is rare.
pub(crate) fn blocked_by_unmanaged(ledger: &InstallationLedger, plan: &OperationPlan) -> bool {
    plan.resources
        .values()
        .any(|planned| match &planned.desired {
            DesiredResource::Path(desired) => {
                ledger
                    .resource_by_identity(&planned.desired.identity())
                    .is_none()
                    && path_entry_exists(&desired.path)
                    && !identical_to_desired(desired, &desired.path)
            }
            _ => false,
        })
}

fn preflight_new_resources(
    ledger: &InstallationLedger,
    plan: &OperationPlan,
    replaced_identities: &BTreeSet<String>,
    replace_unmanaged: bool,
) -> Result<(), String> {
    for planned in plan.resources.values() {
        let identity = planned.desired.identity();
        if let Some(existing) = ledger.resource_by_identity(&identity) {
            if existing.desired_digest != planned.desired.desired_digest()? {
                return Err(format!("{identity} is owned with different content."));
            }
            continue;
        }
        if replaced_identities.contains(&identity) {
            continue;
        }
        match &planned.desired {
            DesiredResource::Path(desired) if path_entry_exists(&desired.path) => {
                // A folder that already holds exactly this content, such as a
                // copy synced from another computer, is taken over.
                if !replace_unmanaged && !identical_to_desired(desired, &desired.path) {
                    return Err(format!(
                        "{} already exists and is not an owned destination.",
                        desired.path.display()
                    ));
                }
            }
            DesiredResource::StructuredEntry(desired) => {
                let contents =
                    managed_documents::read_or_empty(&desired.document_path, desired.format)?;
                if managed_documents::entry_value(&contents, desired.format, &desired.key_path)?
                    .is_some_and(|value| value != desired.value)
                    && !replace_unmanaged
                {
                    return Err(format!(
                        "Configuration entry {} in {} already exists and is unmanaged.",
                        desired.key_path.join("."),
                        desired.document_path.display()
                    ));
                }
            }
            DesiredResource::TextBlock(desired) => {
                let contents = fs::read(&desired.document_path).unwrap_or_default();
                if managed_documents::text_block_body(&contents, &desired.marker_id)?.is_some()
                    && !replace_unmanaged
                {
                    return Err(format!(
                        "Instruction block {} in {} already exists and is unmanaged.",
                        desired.marker_id,
                        desired.document_path.display()
                    ));
                }
            }
            DesiredResource::Path(_) => {}
        }
    }
    Ok(())
}

fn compatibility_destination(
    ledger: &InstallationLedger,
    plan: &OperationPlan,
    paths: &SystemPaths,
) -> Result<OwnedPath, String> {
    for planned in plan.resources.values() {
        if let Some(resource) = ledger.resource_by_identity(&planned.desired.identity()) {
            match &resource.owned {
                OwnedResource::Path(owned) => return Ok(owned.clone()),
                OwnedResource::StructuredEntry(owned) => {
                    return Ok(OwnedPath {
                        path: owned.document_path.clone(),
                        kind: OwnedPathKind::File,
                        installed_digest: owned.document_digest.clone(),
                    });
                }
                OwnedResource::TextBlock(owned) => {
                    return Ok(OwnedPath {
                        path: owned.document_path.clone(),
                        kind: OwnedPathKind::File,
                        installed_digest: owned.document_digest.clone(),
                    });
                }
            }
        }
    }
    let placeholder = paths.home.join(".agents/skills");
    Ok(OwnedPath {
        path: placeholder.display().to_string(),
        kind: OwnedPathKind::Directory,
        installed_digest: ledger::bytes_digest(b"no physical resource"),
    })
}

fn resource_document(resource: &ResourceRecord) -> Option<&Path> {
    match &resource.owned {
        OwnedResource::StructuredEntry(owned) => Some(Path::new(&owned.document_path)),
        OwnedResource::TextBlock(owned) => Some(Path::new(&owned.document_path)),
        OwnedResource::Path(_) => None,
    }
}

fn validate_plan_paths(plan: &OperationPlan) -> Result<(), String> {
    let mut whole_paths = Vec::new();
    let mut documents = Vec::new();
    for planned in plan.resources.values() {
        match &planned.desired {
            DesiredResource::Path(desired) => whole_paths.push(&desired.path),
            DesiredResource::StructuredEntry(desired) => documents.push(&desired.document_path),
            DesiredResource::TextBlock(desired) => documents.push(&desired.document_path),
        }
    }
    for (index, left) in whole_paths.iter().enumerate() {
        for right in whole_paths.iter().skip(index + 1) {
            if left.starts_with(right) || right.starts_with(left) {
                return Err(format!(
                    "Planned owned paths overlap: {} and {}.",
                    left.display(),
                    right.display()
                ));
            }
        }
        if let Some(document) = documents
            .iter()
            .find(|document| document.starts_with(left.as_path()))
        {
            return Err(format!(
                "Planned path {} contains managed document {}.",
                left.display(),
                document.display()
            ));
        }
    }
    Ok(())
}

fn validate_absolute_owned_path(paths: &SystemPaths, path: &Path) -> Result<PathBuf, String> {
    paths.validate_destination(path)
}

fn existing_path_digest(path: &Path) -> Option<String> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_dir() {
        ledger::path_digest(path, OwnedPathKind::Directory).ok()
    } else {
        ledger::path_digest(path, OwnedPathKind::File).ok()
    }
}

fn transaction_id(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    crate::resource::stable_id("tx", &format!("{label}:{nanos}:{}", std::process::id()))
}

fn path_entry_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn remove_any(path: &Path) -> Result<(), String> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Could not inspect {}: {error}", path.display())),
    };
    if metadata.file_type().is_dir() {
        fs_retry::remove_dir_all(path)
            .map_err(|error| format!("Could not remove {}: {error}", path.display()))
    } else {
        fs_retry::remove_file(path)
            .map_err(|error| format!("Could not remove {}: {error}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn a_settings_file_held_without_sharing_reads_as_busy_not_broken() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = tempfile::tempdir().expect("temp");
        let path = root.path().join("mcp.json");
        fs::write(&path, "{}").expect("write");
        let _held = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&path)
            .expect("hold");
        let desired = DesiredStructuredEntry {
            document_path: path,
            format: crate::resource::StructuredFormat::Json,
            key_path: vec!["mcpServers".to_string(), "x".to_string()],
            value: serde_json::json!({}),
        };
        let (problem, busy) = document_problem(&desired, "cursor").expect("problem");
        assert!(
            busy && problem.contains("is using its settings file"),
            "{problem}"
        );
    }
    use crate::agent_profiles::TargetId;
    use crate::catalog::read_manifest_catalog;
    use crate::source::TEST_SOURCE_KEY;

    fn paths(root: &Path) -> SystemPaths {
        SystemPaths {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            local_data: root.join("local-data"),
            cache: root.join("cache"),
        }
    }

    fn fixture(root: &Path) -> (ConfiguredSource, SourceSnapshot, CatalogItem) {
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
        let catalog = read_manifest_catalog(&source_root, TEST_SOURCE_KEY).expect("catalog");
        let source = ConfiguredSource {
            source_key: TEST_SOURCE_KEY.to_string(),
            source_id: "acme".to_string(),
            name: "Acme".to_string(),
            description: "Test".to_string(),
            locator: crate::locator::Locator::display_url(
                "https://nexus.example.com/repository/raw/sources/acme-latest.zip".to_string(),
            ),
            repository_key: None,
        };
        let item = catalog.items["review"].clone();
        let snapshot = SourceSnapshot {
            definition: source.clone(),
            commit: "a".repeat(40),
            path: source_root,
            catalog,
        };
        (source, snapshot, item)
    }

    fn batch_fixture(root: &Path) -> (ConfiguredSource, SourceSnapshot, Vec<CatalogItem>) {
        let source_root = root.join("batch-source");
        for name in ["review", "debug"] {
            let skill_root = source_root.join("skills").join(name);
            fs::create_dir_all(&skill_root).expect("skill");
            fs::write(
                skill_root.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name} code\n---\nBody\n"),
            )
            .expect("skill file");
        }
        fs::write(
            source_root.join("agent-plugins.json"),
            r#"{"version":2,"source":{"id":"acme","name":"Acme","description":"Test"},"packages":[{"id":"review","components":[{"kind":"skill","id":"review","path":"skills/review"}]},{"id":"debug","components":[{"kind":"skill","id":"debug","path":"skills/debug"}]}]}"#,
        )
        .expect("manifest");
        let catalog = read_manifest_catalog(&source_root, TEST_SOURCE_KEY).expect("catalog");
        let source = ConfiguredSource {
            source_key: TEST_SOURCE_KEY.to_string(),
            source_id: "acme".to_string(),
            name: "Acme".to_string(),
            description: "Test".to_string(),
            locator: crate::locator::Locator::display_url(
                "https://nexus.example.com/repository/raw/sources/acme-latest.zip".to_string(),
            ),
            repository_key: None,
        };
        let items = ["review", "debug"]
            .into_iter()
            .map(|id| catalog.items[id].clone())
            .collect();
        let snapshot = SourceSnapshot {
            definition: source.clone(),
            commit: "d".repeat(40),
            path: source_root,
            catalog,
        };
        (source, snapshot, items)
    }

    #[test]
    fn transaction_installs_and_reference_counted_uninstall_removes_path() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, crate::agent_profiles::TargetId::Cursor, true)
            .expect("enable");
        let (source, snapshot, item) = fixture(root.path());
        install(&paths, &source, &snapshot, &item, false, false).expect("install");
        assert!(paths.home.join(".agents/skills/acme-review").is_dir());
        let ledger = read_ledger(&paths).expect("ledger");
        assert_eq!(ledger.items.len(), 1);
        assert_eq!(ledger.bindings.len(), 1);
        assert_eq!(ledger.resources.len(), 1);
        uninstall(&paths, &source, &item.id, false).expect("uninstall");
        assert!(!paths.home.join(".agents/skills/acme-review").exists());
    }

    #[test]
    fn chosen_invocation_reinstalls_the_skill_without_local_changes() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, crate::agent_profiles::TargetId::Cursor, true)
            .expect("enable");
        let (source, snapshot, item) = fixture(root.path());
        install(&paths, &source, &snapshot, &item, false, false).expect("install");
        let skill_file = paths.home.join(".agents/skills/acme-review/SKILL.md");
        assert!(!fs::read_to_string(&skill_file)
            .expect("skill")
            .contains("disable-model-invocation"));

        let mut overrides = crate::invocation::Overrides::new();
        crate::invocation::set(&mut overrides, &item.id, &item.components[0], true);
        crate::invocation::write(&paths, &overrides).expect("write");
        let ids = [item.components[0].id.clone()];
        install_components(&paths, &source, &snapshot, &item, false, false, Some(&ids))
            .expect("reinstall");
        assert!(fs::read_to_string(&skill_file)
            .expect("skill")
            .contains("disable-model-invocation: true"));
        assert_eq!(
            installation_state(
                &paths,
                &read_ledger(&paths).expect("ledger"),
                &item.id,
                None
            ),
            ContentState::Match
        );

        // Choosing the source's value again forgets the choice.
        crate::invocation::set(&mut overrides, &item.id, &item.components[0], false);
        assert!(overrides.is_empty());
    }

    #[test]
    fn batch_preflight_is_all_or_nothing_and_success_uses_one_ledger_commit() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, crate::agent_profiles::TargetId::Cursor, true)
            .expect("enable");
        let (source, snapshot, items) = batch_fixture(root.path());
        fs::create_dir_all(paths.home.join(".agents/skills/acme-debug"))
            .expect("unmanaged conflict");
        let requests = items
            .iter()
            .map(|item| BatchInstall {
                source: &source,
                snapshot: &snapshot,
                item,
                replace_unmanaged: false,
            })
            .collect::<Vec<_>>();
        assert!(install_batch(&paths, &requests, false)
            .expect_err("conflict")
            .contains("already exists"));
        assert!(!paths.home.join(".agents/skills/acme-review").exists());
        assert!(read_ledger(&paths).expect("ledger").items.is_empty());

        fs::remove_dir(paths.home.join(".agents/skills/acme-debug")).expect("remove conflict");
        install_batch(&paths, &requests, false).expect("batch install");
        let ledger = read_ledger(&paths).expect("ledger");
        assert_eq!(ledger.items.len(), 2);
        assert!(ledger.last_transaction_id.is_some());
        assert!(paths.home.join(".agents/skills/acme-review").is_dir());
        assert!(paths.home.join(".agents/skills/acme-debug").is_dir());

        let ids = items.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
        uninstall_batch(&paths, &source, &ids, false).expect("batch uninstall");
        assert!(!paths.home.join(".agents/skills/acme-review").exists());
        assert!(!paths.home.join(".agents/skills/acme-debug").exists());
        assert!(read_ledger(&paths).expect("ledger").items.is_empty());
    }

    #[test]
    fn recovery_rolls_back_only_mutations_that_activated() {
        for activated_count in 0..=3 {
            let root = tempfile::tempdir().expect("root");
            let paths = paths(root.path());
            fs::create_dir_all(&paths.home).expect("home");
            let mutations = ["first", "second", "third"]
                .into_iter()
                .map(|name| {
                    let target = paths.home.join(format!("{name}.txt"));
                    let staging = paths.home.join(format!("{name}-stage.txt"));
                    let backup = paths.home.join(format!("{name}-backup.txt"));
                    fs::write(&target, format!("old-{name}")).expect("target");
                    fs::write(&staging, format!("new-{name}")).expect("staging");
                    JournalMutation {
                        target: target.display().to_string(),
                        staging: Some(staging.display().to_string()),
                        backup: Some(backup.display().to_string()),
                        persistent_backup: false,
                        target_existed: true,
                        original_digest: Some(ledger::bytes_digest(
                            format!("old-{name}").as_bytes(),
                        )),
                    }
                })
                .collect::<Vec<_>>();
            let journal = TransactionJournal {
                version: 1,
                transaction_id: format!("tx-interrupted-{activated_count}"),
                mutations,
            };
            super::journal::write_journal(&paths, &journal).expect("journal");
            for mutation in journal.mutations.iter().take(activated_count) {
                fs::rename(
                    Path::new(&mutation.target),
                    Path::new(mutation.backup.as_ref().expect("backup")),
                )
                .expect("backup target");
                fs::rename(
                    Path::new(mutation.staging.as_ref().expect("staging")),
                    Path::new(&mutation.target),
                )
                .expect("activate target");
            }

            recover(&paths).expect("recover");
            for name in ["first", "second", "third"] {
                assert_eq!(
                    fs::read_to_string(paths.home.join(format!("{name}.txt"))).expect("target"),
                    format!("old-{name}")
                );
                assert!(!paths.home.join(format!("{name}-backup.txt")).exists());
            }
            assert!(!paths.app_data().join(JOURNAL_FILE).exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_journal_that_cannot_be_removed_refuses_changes_until_it_can() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        fs::create_dir_all(&paths.home).expect("home");
        let target = paths.home.join("skill.txt");
        let staging = paths.home.join("skill-stage.txt");
        let backup = paths.home.join("skill-backup.txt");
        fs::write(&target, "old").expect("target");
        fs::write(&staging, "new").expect("staging");
        let journal = TransactionJournal {
            version: 1,
            transaction_id: "tx-killed".to_string(),
            mutations: vec![JournalMutation {
                target: target.display().to_string(),
                staging: Some(staging.display().to_string()),
                backup: Some(backup.display().to_string()),
                persistent_backup: false,
                target_existed: true,
                original_digest: Some(ledger::bytes_digest(b"old")),
            }],
        };
        super::journal::write_journal(&paths, &journal).expect("journal");
        fs::rename(&target, &backup).expect("backup");
        fs::rename(&staging, &target).expect("activate");

        // The journal's folder can't be changed, so the journal can't be removed.
        let app_data = paths.app_data();
        fs::set_permissions(&app_data, fs::Permissions::from_mode(0o555)).expect("read-only");
        let refused = recover(&paths);
        fs::set_permissions(&app_data, fs::Permissions::from_mode(0o755)).expect("writable");
        assert!(refused.expect_err("refused").contains(UNDO_PENDING));
        assert_eq!(
            fs::read_to_string(&target).expect("target"),
            "old",
            "rolled back"
        );

        fs::write(&target, "changed since").expect("a later change");
        recover(&paths).expect("recovered");
        assert_eq!(
            fs::read_to_string(&target).expect("target"),
            "changed since",
            "not rolled back twice"
        );
        assert!(!app_data.join(JOURNAL_FILE).exists());
    }

    #[cfg(unix)]
    #[test]
    fn reset_finishes_a_pending_undo_or_refuses() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let skills = paths.home.join("skills");
        fs::create_dir_all(&skills).expect("skills");
        let target = skills.join("skill.txt");
        let staging = skills.join("skill-stage.txt");
        let backup = skills.join(".resource-previous-skill.txt");
        fs::write(&target, "original").expect("target");
        fs::write(&staging, "new").expect("staging");
        let journal = TransactionJournal {
            version: 1,
            transaction_id: "tx-undo-pending".to_string(),
            mutations: vec![JournalMutation {
                target: target.display().to_string(),
                staging: Some(staging.display().to_string()),
                backup: Some(backup.display().to_string()),
                persistent_backup: false,
                target_existed: true,
                original_digest: Some(ledger::bytes_digest(b"original")),
            }],
        };
        super::journal::write_journal(&paths, &journal).expect("journal");
        fs::rename(&target, &backup).expect("backup");
        fs::rename(&staging, &target).expect("activate");

        // The folder can't change yet, so the undo can't run.
        fs::set_permissions(&skills, fs::Permissions::from_mode(0o555)).expect("read-only");
        let refused = reset_app(&paths, &[]);
        fs::set_permissions(&skills, fs::Permissions::from_mode(0o755)).expect("writable");
        assert!(refused.expect_err("refused").contains(UNDO_PENDING));
        assert!(
            paths.app_data().join(JOURNAL_FILE).exists(),
            "the undo is still owed"
        );

        reset_app(&paths, &[]).expect("reset");
        assert_eq!(fs::read_to_string(&target).expect("target"), "original");
        assert!(!backup.exists());
        assert!(!paths.app_data().join(JOURNAL_FILE).exists());
    }

    #[test]
    fn a_read_leaves_the_live_journal_of_another_process_alone() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        fs::create_dir_all(&paths.home).expect("home");
        let target = paths.home.join("skill.txt");
        let staging = paths.home.join("skill-stage.txt");
        fs::write(&staging, "new").expect("staging");
        let journal = TransactionJournal {
            version: 1,
            transaction_id: "tx-in-flight".to_string(),
            mutations: vec![JournalMutation {
                target: target.display().to_string(),
                staging: Some(staging.display().to_string()),
                backup: None,
                persistent_backup: false,
                target_existed: false,
                original_digest: None,
            }],
        };
        super::journal::write_journal(&paths, &journal).expect("journal");
        fs::rename(&staging, &target).expect("activate");
        // Another process is mid-commit: it holds the lock through its own handle.
        let other = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(paths.app_data().join("transaction.lock"))
            .expect("lock file");
        other.lock().expect("other process");
        read_ledger(&paths).expect("ledger");
        assert!(target.exists(), "a read rolled back a live transaction");
        assert!(paths.app_data().join(JOURNAL_FILE).exists());
        drop(other);
        read_ledger(&paths).expect("ledger");
        assert!(!target.exists(), "a crashed transaction is rolled back");
    }

    #[test]
    fn recovery_keeps_a_transaction_whose_ledger_committed() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        fs::create_dir_all(&paths.home).expect("home");
        let target = paths.home.join("managed.txt");
        let backup = paths.home.join("managed-backup.txt");
        fs::write(&target, "new").expect("target");
        fs::write(&backup, "old").expect("backup");
        let transaction_id = "tx-committed".to_string();
        let journal = TransactionJournal {
            version: 1,
            transaction_id: transaction_id.clone(),
            mutations: vec![JournalMutation {
                target: target.display().to_string(),
                staging: None,
                backup: Some(backup.display().to_string()),
                persistent_backup: false,
                target_existed: true,
                original_digest: Some(ledger::bytes_digest(b"old")),
            }],
        };
        let ledger_state = InstallationLedger {
            last_transaction_id: Some(transaction_id),
            ..InstallationLedger::default()
        };
        ledger::write(&paths.app_data(), &ledger_state).expect("ledger");
        super::journal::write_journal(&paths, &journal).expect("journal");

        recover(&paths).expect("recover");
        assert_eq!(fs::read_to_string(target).expect("target"), "new");
        assert!(!backup.exists());
    }

    #[test]
    fn installed_package_conflicts_are_enforced_in_both_install_orders() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let source_root = root.path().join("conflict-source");
        for name in ["old", "new"] {
            let skill_root = source_root.join("skills").join(name);
            fs::create_dir_all(&skill_root).expect("skill");
            fs::write(
                skill_root.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {name} skill\n---\nBody\n"),
            )
            .expect("skill file");
        }
        fs::write(
            source_root.join("agent-plugins.json"),
            r#"{
              "version":2,
              "source":{"id":"acme","name":"Acme","description":"Test"},
              "packages":[
                {"id":"old","components":[{"kind":"skill","path":"skills/old"}],"conflictsWith":["acme/new"]},
                {"id":"new","components":[{"kind":"skill","path":"skills/new"}]}
              ]
            }"#,
        )
        .expect("manifest");
        let catalog = read_manifest_catalog(&source_root, TEST_SOURCE_KEY).expect("catalog");
        let source = ConfiguredSource {
            source_key: TEST_SOURCE_KEY.to_string(),
            source_id: "acme".to_string(),
            name: "Acme".to_string(),
            description: "Test".to_string(),
            locator: crate::locator::Locator::display_url(
                "https://nexus.example.com/repository/raw/sources/acme-latest.zip".to_string(),
            ),
            repository_key: None,
        };
        let old = catalog.items["old"].clone();
        let new = catalog.items["new"].clone();
        let snapshot = SourceSnapshot {
            definition: source.clone(),
            commit: "e".repeat(40),
            path: source_root,
            catalog,
        };
        crate::agent_profiles::set_enabled(&paths, TargetId::Codex, true).expect("enable");

        install(&paths, &source, &snapshot, &old, false, false).expect("install old");
        assert!(install(&paths, &source, &snapshot, &new, false, false)
            .expect_err("conflict")
            .contains("can't be installed while"));
        assert!(paths.home.join(".agents/skills/acme-old").is_dir());
        assert!(!paths.home.join(".agents/skills/acme-new").exists());
        assert_eq!(read_ledger(&paths).expect("ledger").items.len(), 1);
    }

    #[test]
    fn source_reset_wipes_foreign_source_key_and_leaves_other_sources() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, crate::agent_profiles::TargetId::Cursor, true)
            .expect("enable");
        let (source, snapshot, item) = fixture(root.path());
        install(&paths, &source, &snapshot, &item, false, false).expect("install");
        let other_root = root.path().join("other-source");
        fs::create_dir_all(other_root.join("skills/keep")).expect("other skill");
        fs::write(
            other_root.join("skills/keep/SKILL.md"),
            "---\nname: keep\ndescription: Keep this\n---\nBody\n",
        )
        .expect("other skill file");
        fs::write(
            other_root.join("agent-plugins.json"),
            r#"{"version":2,"source":{"id":"other","name":"Other","description":"Keep"},"packages":[{"id":"keep","components":[{"kind":"skill","path":"skills/keep"}]}]}"#,
        )
        .expect("other manifest");
        let other_key = "source-other000000";
        let other_catalog = read_manifest_catalog(&other_root, other_key).expect("other catalog");
        let other_source = ConfiguredSource {
            source_key: other_key.to_string(),
            source_id: "other".to_string(),
            name: "Other".to_string(),
            description: "Keep".to_string(),
            locator: crate::locator::Locator::display_url(
                "https://nexus.example.com/repository/raw/sources/other-latest.zip".to_string(),
            ),
            repository_key: None,
        };
        let other_item = other_catalog.items["keep"].clone();
        let other_snapshot = SourceSnapshot {
            definition: other_source.clone(),
            commit: "f".repeat(40),
            path: other_root,
            catalog: other_catalog,
        };
        install(
            &paths,
            &other_source,
            &other_snapshot,
            &other_item,
            false,
            false,
        )
        .expect("install other");

        let mut ledger = read_ledger(&paths).expect("ledger");
        ledger.items.get_mut(&item.id).expect("record").source_key = "stale-source-key".to_string();
        crate::ledger::write(&paths.app_data(), &ledger).expect("rewrite");
        assert_eq!(
            crate::application::status::item_status(
                &paths,
                &read_ledger(&paths).expect("ledger"),
                Some(&item),
                &item.id
            ),
            crate::install::ItemStatus::SourceConflict
        );
        assert!(uninstall(&paths, &source, &item.id, true)
            .expect_err("foreign")
            .contains("owned by a different source"));

        let target = paths.home.join(".agents/skills/acme-review");
        fs::write(target.join("local.txt"), "edit").expect("local edit");
        let outcome = reset_source(&paths, &source, Some(&snapshot)).expect("reset");
        assert!(!outcome.backup_paths.is_empty());
        assert!(!target.exists());
        assert!(paths.home.join(".agents/skills/other-keep").is_dir());
        let ledger = read_ledger(&paths).expect("ledger");
        assert!(!ledger.items.contains_key(&item.id));
        assert!(ledger.items.contains_key(&other_item.id));
        assert_eq!(
            crate::application::status::item_status(&paths, &ledger, Some(&item), &item.id),
            crate::install::ItemStatus::Available
        );
    }

    #[test]
    fn source_reset_is_a_noop_when_the_source_has_no_ledger_records() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let (source, snapshot, _) = fixture(root.path());
        let outcome = reset_source(&paths, &source, Some(&snapshot)).expect("reset");
        assert!(outcome.backup_paths.is_empty());
        assert!(read_ledger(&paths).expect("ledger").items.is_empty());
    }

    #[test]
    fn source_reset_clears_conflict_when_a_resource_cannot_be_staged() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, crate::agent_profiles::TargetId::Cursor, true)
            .expect("enable");
        let (source, snapshot, item) = fixture(root.path());
        install(&paths, &source, &snapshot, &item, false, false).expect("install");
        let mut ledger = read_ledger(&paths).expect("ledger");
        let binding_id = ledger.items[&item.id].binding_ids[0].clone();
        let poison = paths.data.join("agent-plugins/poison.toml");
        let resource_id = "resource-poison00000000000000".to_string();
        ledger.resources.insert(
            resource_id.clone(),
            ResourceRecord {
                id: resource_id.clone(),
                identity: "entry://poison:key".to_string(),
                desired_digest: "a".repeat(64),
                owned: OwnedResource::StructuredEntry(OwnedStructuredEntry {
                    document_path: poison.display().to_string(),
                    format: StructuredFormat::Toml,
                    key_path: vec!["poison".to_string()],
                    value_digest: "a".repeat(64),
                    document_digest: "a".repeat(64),
                }),
                consumer_binding_ids: vec![binding_id.clone()],
                adapter_id: "cursor".to_string(),
                dialect_id: "cursor-2026-08".to_string(),
            },
        );
        ledger
            .bindings
            .get_mut(&binding_id)
            .expect("binding")
            .resource_ids
            .push(resource_id);
        ledger.items.get_mut(&item.id).expect("record").source_key = "stale-source-key".to_string();
        crate::ledger::write(&paths.app_data(), &ledger).expect("rewrite");

        reset_source(&paths, &source, Some(&snapshot)).expect("reset");
        let ledger = read_ledger(&paths).expect("ledger");
        assert!(!ledger.items.contains_key(&item.id));
        assert_eq!(
            crate::application::status::item_status(&paths, &ledger, Some(&item), &item.id),
            crate::install::ItemStatus::Available
        );
        assert!(!paths.home.join(".agents/skills/acme-review").exists());
    }

    #[test]
    fn source_reset_removes_leftover_namespaced_skill_directories() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let (source, snapshot, _) = fixture(root.path());
        let leftover = paths.home.join(".agents/skills/acme-review");
        fs::create_dir_all(&leftover).expect("leftover");
        fs::write(leftover.join("SKILL.md"), "stale").expect("stale skill");
        reset_source(&paths, &source, Some(&snapshot)).expect("reset");
        assert!(!leftover.exists());
    }

    #[test]
    fn app_reset_uninstalls_every_source_and_leftover_directories() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, crate::agent_profiles::TargetId::Cursor, true)
            .expect("enable");
        let (source, snapshot, item) = fixture(root.path());
        install(&paths, &source, &snapshot, &item, false, false).expect("install");
        let other_root = root.path().join("other-source");
        fs::create_dir_all(other_root.join("skills/keep")).expect("other skill");
        fs::write(
            other_root.join("skills/keep/SKILL.md"),
            "---\nname: keep\ndescription: Keep this\n---\nBody\n",
        )
        .expect("other skill file");
        fs::write(
            other_root.join("agent-plugins.json"),
            r#"{"version":2,"source":{"id":"other","name":"Other","description":"Keep"},"packages":[{"id":"keep","components":[{"kind":"skill","path":"skills/keep"}]}]}"#,
        )
        .expect("other manifest");
        let other_key = "source-other000000";
        let other_catalog = read_manifest_catalog(&other_root, other_key).expect("other catalog");
        let other_source = ConfiguredSource {
            source_key: other_key.to_string(),
            source_id: "other".to_string(),
            name: "Other".to_string(),
            description: "Keep".to_string(),
            locator: crate::locator::Locator::display_url(
                "https://nexus.example.com/repository/raw/sources/other-latest.zip".to_string(),
            ),
            repository_key: None,
        };
        let other_item = other_catalog.items["keep"].clone();
        let other_snapshot = SourceSnapshot {
            definition: other_source.clone(),
            commit: "f".repeat(40),
            path: other_root,
            catalog: other_catalog,
        };
        install(
            &paths,
            &other_source,
            &other_snapshot,
            &other_item,
            false,
            false,
        )
        .expect("install other");
        let leftover = paths.home.join(".agents/skills/acme-stale");
        fs::create_dir_all(&leftover).expect("leftover");
        fs::write(leftover.join("SKILL.md"), "stale").expect("stale skill");

        reset_app(
            &paths,
            &[
                (source, Some(snapshot)),
                (other_source, Some(other_snapshot)),
            ],
        )
        .expect("reset");

        assert!(!paths.home.join(".agents/skills/acme-review").exists());
        assert!(!paths.home.join(".agents/skills/other-keep").exists());
        assert!(!leftover.exists());
        assert!(read_ledger(&paths).expect("ledger").items.is_empty());
    }

    #[test]
    fn app_reset_uninstalls_installs_even_when_the_source_is_no_longer_configured() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, crate::agent_profiles::TargetId::Cursor, true)
            .expect("enable");
        let (source, snapshot, item) = fixture(root.path());
        install(&paths, &source, &snapshot, &item, false, false).expect("install");
        reset_app(&paths, &[]).expect("reset");
        assert!(!paths.home.join(".agents/skills/acme-review").exists());
        assert!(read_ledger(&paths).expect("ledger").items.is_empty());
    }

    #[test]
    fn app_reset_is_a_noop_when_nothing_is_installed() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let (source, snapshot, _) = fixture(root.path());
        let outcome = reset_app(&paths, &[(source, Some(snapshot))]).expect("reset");
        assert!(outcome.backup_paths.is_empty());
        assert!(read_ledger(&paths).expect("ledger").items.is_empty());
    }

    #[test]
    fn mcp_install_requires_trust_and_preserves_user_content() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let source_root = root.path().join("source-config");
        fs::create_dir_all(source_root.join("mcp")).expect("mcp");
        fs::write(
            source_root.join("mcp/database.json"),
            r#"{
              "$schema":"https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
              "mcpServers":{"database":{"type":"stdio","command":"node","args":["server.js"],"env":{"MODE":"safe"}}}
            }"#,
        )
        .expect("mcp config");
        fs::write(
            source_root.join("agent-plugins.json"),
            r#"{
              "version":2,
              "source":{"id":"acme","name":"Acme","description":"Test"},
              "packages":[{
                "id":"tools",
                "components":[
                  {"kind":"mcpServer","id":"database","path":"mcp/database.json"}
                ]
              }]
            }"#,
        )
        .expect("manifest");
        let catalog = read_manifest_catalog(&source_root, TEST_SOURCE_KEY).expect("catalog");
        let source = ConfiguredSource {
            source_key: TEST_SOURCE_KEY.to_string(),
            source_id: "acme".to_string(),
            name: "Acme".to_string(),
            description: "Test".to_string(),
            locator: crate::locator::Locator::display_url(
                "https://nexus.example.com/repository/raw/sources/acme-latest.zip".to_string(),
            ),
            repository_key: None,
        };
        let item = catalog.items["tools"].clone();
        let snapshot = SourceSnapshot {
            definition: source.clone(),
            commit: "c".repeat(40),
            path: source_root,
            catalog,
        };
        crate::agent_profiles::set_enabled(&paths, TargetId::Codex, true).expect("enable");
        fs::create_dir_all(paths.home.join(".codex")).expect("codex");
        fs::write(
            paths.home.join(".codex/config.toml"),
            "model = \"gpt\" # keep\n",
        )
        .expect("config");

        assert!(install(&paths, &source, &snapshot, &item, false, false)
            .expect_err("approval")
            .contains("Tier 3"));
        install(&paths, &source, &snapshot, &item, false, true).expect("install");
        let config = fs::read_to_string(paths.home.join(".codex/config.toml")).expect("config");
        assert!(config.contains("model = \"gpt\" # keep"));
        assert!(config.contains("acme-database"));

        uninstall(&paths, &source, &item.id, false).expect("uninstall");
        let config = fs::read_to_string(paths.home.join(".codex/config.toml")).expect("config");
        assert!(config.contains("model = \"gpt\" # keep"));
        assert!(!config.contains("acme-database"));
    }

    fn mixed_fixture(root: &Path) -> (ConfiguredSource, SourceSnapshot, CatalogItem) {
        let source_root = root.join("mixed-source");
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
              "source":{"id":"acme","name":"Acme","description":"Test"},
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
        let source = ConfiguredSource {
            source_key: TEST_SOURCE_KEY.to_string(),
            source_id: "acme".to_string(),
            name: "Acme".to_string(),
            description: "Test".to_string(),
            locator: crate::locator::Locator::display_url(
                "https://nexus.example.com/repository/raw/sources/acme-latest.zip".to_string(),
            ),
            repository_key: None,
        };
        let item = catalog.items["tools"].clone();
        let snapshot = SourceSnapshot {
            definition: source.clone(),
            commit: "e".repeat(40),
            path: source_root,
            catalog,
        };
        (source, snapshot, item)
    }

    #[test]
    fn mcp_install_creates_a_missing_copilot_config() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, TargetId::GithubCopilot, true).expect("enable");
        let (source, snapshot, item) = mixed_fixture(root.path());

        install(&paths, &source, &snapshot, &item, false, true).expect("install");

        assert!(paths.home.join(".agents/skills/acme-review").is_dir());
        let config =
            fs::read_to_string(paths.home.join(".copilot/mcp-config.json")).expect("config");
        assert!(config.contains("acme-database"));
    }

    #[test]
    fn components_can_be_installed_and_uninstalled_independently() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, TargetId::Codex, true).expect("enable");
        let (source, snapshot, item) = mixed_fixture(root.path());
        fs::create_dir_all(paths.home.join(".codex")).expect("codex");
        fs::write(paths.home.join(".codex/config.toml"), "model = \"gpt\"\n").expect("config");

        install_components(
            &paths,
            &source,
            &snapshot,
            &item,
            false,
            false,
            Some(&["review".to_string()]),
        )
        .expect("install skill");
        assert!(paths.home.join(".agents/skills/acme-review").is_dir());
        assert!(!fs::read_to_string(paths.home.join(".codex/config.toml"))
            .expect("config")
            .contains("acme-database"));
        let ledger = read_ledger(&paths).expect("ledger");
        assert_eq!(
            ledger.items[&item.id].selected_component_ids,
            vec!["review".to_string()]
        );

        install_components(
            &paths,
            &source,
            &snapshot,
            &item,
            false,
            true,
            Some(&["database".to_string()]),
        )
        .expect("install mcp");
        assert!(paths.home.join(".agents/skills/acme-review").is_dir());
        assert!(fs::read_to_string(paths.home.join(".codex/config.toml"))
            .expect("config")
            .contains("acme-database"));
        let ledger = read_ledger(&paths).expect("ledger");
        assert!(ledger.items[&item.id].selected_component_ids.is_empty());

        uninstall_components(
            &paths,
            &source,
            &item.id,
            Some(&["review".to_string()]),
            false,
        )
        .expect("uninstall skill");
        assert!(!paths.home.join(".agents/skills/acme-review").exists());
        assert!(fs::read_to_string(paths.home.join(".codex/config.toml"))
            .expect("config")
            .contains("acme-database"));
        let ledger = read_ledger(&paths).expect("ledger");
        assert_eq!(
            ledger.items[&item.id].selected_component_ids,
            vec!["database".to_string()]
        );

        uninstall_components(
            &paths,
            &source,
            &item.id,
            Some(&["database".to_string()]),
            false,
        )
        .expect("uninstall mcp");
        assert!(!ledger_has_item(&paths, &item.id));
        assert!(!fs::read_to_string(paths.home.join(".codex/config.toml"))
            .expect("config")
            .contains("acme-database"));
    }

    fn forget_ledger(paths: &SystemPaths) {
        for name in ["installations.json", "installations.json.previous"] {
            let _ = fs::remove_file(paths.app_data().join(name));
        }
    }

    fn ledger_has_item(paths: &SystemPaths, id: &str) -> bool {
        read_ledger(paths).expect("ledger").items.contains_key(id)
    }

    #[test]
    #[cfg(unix)]
    fn failed_rollback_keeps_the_journal_for_the_next_recovery() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let folder = paths.home.join("config");
        fs::create_dir_all(&folder).expect("folder");
        let target = folder.join("settings.json");
        let backup = folder.join("settings-backup.json");
        fs::write(&target, "new").expect("activated target");
        fs::write(&backup, "old").expect("backup");
        let journal = TransactionJournal {
            version: 1,
            transaction_id: "tx-stuck".to_string(),
            mutations: vec![JournalMutation {
                target: target.display().to_string(),
                staging: Some(folder.join("settings-stage.json").display().to_string()),
                backup: Some(backup.display().to_string()),
                persistent_backup: false,
                target_existed: true,
                original_digest: Some(ledger::bytes_digest(b"old")),
            }],
        };
        super::journal::write_journal(&paths, &journal).expect("journal");
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).expect("lock folder");
        if fs::write(folder.join("probe"), "").is_ok() {
            return; // Running as root; permissions cannot simulate the failure.
        }
        assert!(recover(&paths).is_err());
        assert!(paths.app_data().join(JOURNAL_FILE).exists());
        read_ledger(&paths).expect("reads do not wait on recovery");

        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).expect("unlock folder");
        recover(&paths).expect("retried rollback");
        assert_eq!(fs::read_to_string(&target).expect("restored"), "old");
        assert!(!paths.app_data().join(JOURNAL_FILE).exists());
    }

    #[test]
    fn unreadable_journal_is_moved_aside_and_reads_continue() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        fs::create_dir_all(paths.app_data()).expect("data");
        fs::write(paths.app_data().join(JOURNAL_FILE), "{not json").expect("journal");
        read_ledger(&paths).expect("ledger");
        assert!(!paths.app_data().join(JOURNAL_FILE).exists());
        assert!(fs::read_dir(paths.app_data())
            .expect("data")
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-")));
    }

    #[test]
    fn missing_files_are_missing_not_modified_and_uninstall_succeeds() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, TargetId::Cursor, true).expect("enable");
        let (source, snapshot, item) = fixture(root.path());
        install(&paths, &source, &snapshot, &item, false, false).expect("install");
        fs::remove_dir_all(paths.home.join(".agents/skills/acme-review")).expect("delete");
        let ledger = read_ledger(&paths).expect("ledger");
        assert_eq!(
            crate::application::status::item_status(&paths, &ledger, Some(&item), &item.id),
            crate::install::ItemStatus::Missing
        );
        uninstall(&paths, &source, &item.id, false).expect("uninstall");
        assert!(read_ledger(&paths).expect("ledger").items.is_empty());
    }

    #[test]
    fn replace_restores_a_modified_install_and_keeps_the_users_copy() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, TargetId::Cursor, true).expect("enable");
        let (source, snapshot, item) = fixture(root.path());
        install(&paths, &source, &snapshot, &item, false, false).expect("install");
        let target = paths.home.join(".agents/skills/acme-review");
        fs::write(target.join("local.txt"), "mine").expect("edit");
        let outcome = install(&paths, &source, &snapshot, &item, true, false).expect("restore");
        assert_eq!(outcome.backup_paths.len(), 1);
        assert!(Path::new(&outcome.backup_paths[0])
            .join("local.txt")
            .exists());
        assert!(!target.join("local.txt").exists());
        let ledger = read_ledger(&paths).expect("ledger");
        assert_eq!(
            crate::application::status::item_status(&paths, &ledger, Some(&item), &item.id),
            crate::install::ItemStatus::Installed
        );
    }

    #[test]
    fn replace_backs_up_same_named_originals_of_two_apps_separately() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, TargetId::Cursor, true).expect("cursor");
        crate::agent_profiles::set_enabled(&paths, TargetId::ClaudeCode, true).expect("claude");
        let (source, snapshot, item) = fixture(root.path());
        for (folder, text) in [
            (".agents/skills", "shared copy"),
            (".claude/skills", "claude copy"),
        ] {
            let own = paths.home.join(folder).join("acme-review");
            fs::create_dir_all(&own).expect("own skill");
            fs::write(own.join("SKILL.md"), text).expect("own text");
        }
        let outcome = install(&paths, &source, &snapshot, &item, true, false).expect("replace");
        let mut kept = outcome
            .backup_paths
            .iter()
            .map(|backup| fs::read_to_string(Path::new(backup).join("SKILL.md")).expect("backup"))
            .collect::<Vec<_>>();
        kept.sort();
        assert_eq!(kept, ["claude copy", "shared copy"]);
    }

    #[test]
    fn unreadable_agent_config_skips_only_that_agent() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let (source, snapshot, item) = mixed_fixture(root.path());
        crate::agent_profiles::set_enabled(&paths, TargetId::Cursor, true).expect("cursor");
        crate::agent_profiles::set_enabled(&paths, TargetId::Codex, true).expect("codex");
        fs::create_dir_all(paths.home.join(".codex")).expect("codex");
        fs::write(paths.home.join(".codex/config.toml"), "model = = broken").expect("broken");
        let outcome = install(&paths, &source, &snapshot, &item, false, true).expect("install");
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].starts_with("Codex"));
        assert!(fs::read_to_string(paths.home.join(".cursor/mcp.json"))
            .expect("cursor config")
            .contains("acme-database"));
        assert!(paths.home.join(".agents/skills/acme-review").is_dir());
        assert!(take_skipped_agent(&item.id));
    }

    #[test]
    fn reinstall_over_an_unrecorded_identical_entry_keeps_the_ledger_whole() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let (source, snapshot, item) = mixed_fixture(root.path());
        crate::agent_profiles::set_enabled(&paths, TargetId::Cursor, true).expect("cursor");
        crate::agent_profiles::set_enabled(&paths, TargetId::Codex, true).expect("codex");
        fs::create_dir_all(paths.home.join(".codex")).expect("codex");
        fs::write(paths.home.join(".codex/config.toml"), "model = = broken").expect("broken");
        install(&paths, &source, &snapshot, &item, false, true).expect("install");
        let ledger_file = paths.app_data().join("installations.json");
        let skipped = fs::read(&ledger_file).expect("ledger");
        fs::write(paths.home.join(".codex/config.toml"), "").expect("fixed");
        install(&paths, &source, &snapshot, &item, false, true).expect("add codex");
        // The last good copy from before Codex was added comes back, as after a damaged ledger.
        fs::write(&ledger_file, skipped).expect("restore");
        install(&paths, &source, &snapshot, &item, false, true).expect("reinstall");
        let ledger = read_ledger(&paths).expect("ledger");
        // Reading prunes references to missing resources, so a lost entry shows
        // up as a binding that owns nothing.
        for binding in ledger.bindings.values() {
            assert!(
                !binding.resource_ids.is_empty()
                    && binding
                        .resource_ids
                        .iter()
                        .all(|resource| ledger.resources.contains_key(resource)),
                "{} {} lost its resources",
                binding.component_id,
                binding.target_id
            );
        }
        let plan = planner::plan(&paths, &snapshot, &item, None, None).expect("plan");
        assert!(plan_satisfied(&ledger, &plan).expect("satisfied"));
    }

    #[test]
    fn a_component_no_detected_app_can_use_is_refused_not_recorded() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let (source, snapshot, item) = mixed_fixture(root.path());
        // Claude Desktop takes skills only through claude.ai.
        crate::agent_profiles::set_enabled(&paths, TargetId::ClaudeDesktop, true).expect("app");
        let error = install_components(
            &paths,
            &source,
            &snapshot,
            &item,
            false,
            false,
            Some(&["review".to_string()]),
        )
        .expect_err("nothing can use it");
        assert!(
            error.starts_with("None of the AI apps on this computer can use"),
            "{error}"
        );
        assert!(!ledger_has_item(&paths, &item.id));
    }

    #[cfg(unix)]
    #[test]
    fn a_skill_folder_that_cannot_be_written_skips_only_its_agents() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let (source, snapshot, item) = fixture(root.path());
        crate::agent_profiles::set_enabled(&paths, TargetId::Codex, true).expect("codex");
        crate::agent_profiles::set_enabled(&paths, TargetId::ClaudeCode, true).expect("claude");
        let shared = paths.home.join(".agents/skills");
        fs::create_dir_all(&shared).expect("skills");
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o555)).expect("read-only");
        let outcome = install(&paths, &source, &snapshot, &item, false, false);
        fs::set_permissions(&shared, fs::Permissions::from_mode(0o755)).expect("writable");
        let outcome = outcome.expect("installs for Claude Code");
        assert!(
            outcome.warnings.len() == 1
                && outcome.warnings[0].starts_with("Codex did not get the package"),
            "{:?}",
            outcome.warnings
        );
        assert!(paths.home.join(".claude/skills/acme-review").is_dir());
    }

    #[test]
    fn stale_staging_is_swept_and_fresh_staging_is_kept() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let skills = paths.home.join(".agents/skills");
        fs::create_dir_all(&skills).expect("skills");
        let stale = skills.join(".resource-installing-1-1");
        let fresh = skills.join(".resource-installing-1-2");
        let backup = skills.join(".resource-previous-1-3");
        for path in [&stale, &fresh, &backup] {
            fs::write(path, "staged").expect("staging");
        }
        let hour_ago = SystemTime::now() - Duration::from_secs(60 * 60);
        for path in [&stale, &backup] {
            fs::File::options()
                .write(true)
                .open(path)
                .and_then(|file| file.set_modified(hour_ago))
                .expect("age");
        }
        sweep_stale_staging(&paths);
        assert!(!stale.exists());
        assert!(fresh.exists());
        assert!(backup.exists());
    }

    #[test]
    fn identical_unmanaged_folder_is_taken_over() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(&paths, TargetId::Cursor, true).expect("enable");
        let (source, snapshot, item) = fixture(root.path());
        install(&paths, &source, &snapshot, &item, false, false).expect("install");
        // Another computer installed it into a shared folder; this one has no record.
        forget_ledger(&paths);
        let outcome = install(&paths, &source, &snapshot, &item, false, false).expect("adopt");
        assert!(outcome.backup_paths.is_empty());
        assert!(ledger_has_item(&paths, &item.id));

        forget_ledger(&paths);
        fs::write(
            paths.home.join(".agents/skills/acme-review/extra.txt"),
            "different",
        )
        .expect("differ");
        assert!(install(&paths, &source, &snapshot, &item, false, false)
            .expect_err("different content")
            .contains("already exists"));
    }
}
