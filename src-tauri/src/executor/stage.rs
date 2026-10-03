//! Staging of planned path and document mutations.

use super::journal::cleanup_mutations;
use super::matching::{resource_state, ContentState};
use super::{
    existing_path_digest, normalize_path, path_entry_exists, remove_any, resource_document,
    validate_absolute_owned_path, JournalMutation, TransactionJournal,
};
use crate::catalog::materialize_agent_skill;
use crate::fs_retry;
use crate::ledger::{
    self, InstallationLedger, OwnedPath, OwnedPathKind, OwnedResource, OwnedStructuredEntry,
    OwnedTextBlock, ResourceRecord,
};
use crate::managed_documents;
use crate::paths::SystemPaths;
use crate::resource::{DesiredResource, OperationPlan, PathMaterialization, StructuredFormat};
use crate::sources::{copy_directory, temporary_path};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub(super) struct DocumentWork {
    path: PathBuf,
    /// False when the document already reads as planned. Nothing is written
    /// then, but its planned entries are still recorded as owned.
    changed: bool,
    updated: Vec<u8>,
    persistent_backup: bool,
}

#[derive(Clone, Copy)]
pub(super) struct StageRequest<'a> {
    pub(super) paths: &'a SystemPaths,
    pub(super) transaction_id: &'a str,
    pub(super) plan: &'a OperationPlan,
    pub(super) removed: &'a [ResourceRecord],
    pub(super) remaining_ledger: &'a InstallationLedger,
    pub(super) replace_unmanaged: bool,
    pub(super) force_modified: bool,
}

/// Stages every planned change next to its target. On failure nothing staged
/// is left behind.
pub(super) fn stage_changes(
    request: &StageRequest<'_>,
) -> Result<(TransactionJournal, Vec<ResourceRecord>, Vec<PathBuf>), String> {
    let mut mutations = Vec::new();
    match stage_into(request, &mut mutations) {
        Ok((installed, backup_paths)) => {
            mutations.sort_by(|left, right| left.target.cmp(&right.target));
            Ok((
                TransactionJournal {
                    version: 1,
                    transaction_id: request.transaction_id.to_string(),
                    mutations,
                },
                installed,
                backup_paths,
            ))
        }
        Err(error) => {
            cleanup_mutations(&mutations);
            Err(error)
        }
    }
}

fn protected_error(target: &Path, state: &ContentState) -> String {
    match state {
        ContentState::Unknown(error) => error.clone(),
        _ => format!("{} {}.", target.display(), super::LOCAL_CHANGES),
    }
}

fn stage_into(
    request: &StageRequest<'_>,
    mutations: &mut Vec<JournalMutation>,
) -> Result<(Vec<ResourceRecord>, Vec<PathBuf>), String> {
    let StageRequest {
        paths,
        transaction_id,
        plan,
        removed,
        remaining_ledger,
        replace_unmanaged,
        force_modified,
    } = *request;
    let mut installed = Vec::new();
    let mut backup_paths = Vec::new();
    let desired_identities = plan
        .resources
        .values()
        .map(|resource| resource.desired.identity())
        .collect::<BTreeSet<_>>();

    for planned in plan.resources.values() {
        let identity = planned.desired.identity();
        if let Some(existing) = remaining_ledger.resource_by_identity(&identity) {
            installed.push(ResourceRecord {
                id: planned.id.clone(),
                identity,
                desired_digest: planned.desired.desired_digest()?,
                owned: existing.owned.clone(),
                consumer_binding_ids: planned.consumer_binding_ids.clone(),
                adapter_id: planned.adapter_id.clone(),
                dialect_id: planned.dialect_id.clone(),
            });
            continue;
        }
        let DesiredResource::Path(desired) = &planned.desired else {
            continue;
        };
        let target = validate_absolute_owned_path(paths, &desired.path)?;
        let exists = path_entry_exists(&target);
        // An unmanaged copy being replaced, or a modified owned copy being
        // restored, is the user's: it moves to a backup that is kept.
        let persistent = exists
            && match removed
                .iter()
                .find(|resource| resource.identity == identity)
            {
                None => replace_unmanaged,
                // A changed copy that already equals what replaces it loses nothing.
                Some(old) => {
                    force_modified
                        && resource_state(paths, old).is_protected()
                        && !identical_to_desired(desired, &target)
                }
            };
        let original_digest = existing_path_digest(&target);
        let staging = stage_path(desired, &target)?;
        let backup = match mutation_backup(paths, transaction_id, &target, persistent) {
            Ok(backup) => backup,
            Err(error) => {
                let _ = remove_any(&staging);
                return Err(error);
            }
        };
        mutations.push(JournalMutation {
            target: target.display().to_string(),
            staging: Some(staging.display().to_string()),
            backup: exists.then(|| backup.display().to_string()),
            persistent_backup: persistent,
            target_existed: exists,
            original_digest,
        });
        if persistent {
            backup_paths.push(backup);
        }
        installed.push(ResourceRecord {
            id: planned.id.clone(),
            identity,
            desired_digest: planned.desired.desired_digest()?,
            owned: OwnedResource::Path(OwnedPath {
                path: target.display().to_string(),
                kind: desired.kind,
                installed_digest: ledger::path_digest(&staging, desired.kind)?,
            }),
            consumer_binding_ids: planned.consumer_binding_ids.clone(),
            adapter_id: planned.adapter_id.clone(),
            dialect_id: planned.dialect_id.clone(),
        });
    }

    for old in removed {
        if desired_identities.contains(&old.identity) {
            continue;
        }
        let OwnedResource::Path(owned) = &old.owned else {
            continue;
        };
        let target = validate_absolute_owned_path(paths, Path::new(&owned.path))?;
        if !path_entry_exists(&target) {
            continue;
        }
        let state = resource_state(paths, old);
        if state.is_protected() && !force_modified {
            return Err(protected_error(&target, &state));
        }
        let persistent = state.is_protected();
        let backup = mutation_backup(paths, transaction_id, &target, persistent)?;
        if persistent {
            backup_paths.push(backup.clone());
        }
        mutations.push(JournalMutation {
            target: target.display().to_string(),
            staging: None,
            backup: Some(backup.display().to_string()),
            persistent_backup: persistent,
            target_existed: true,
            original_digest: existing_path_digest(&target),
        });
    }

    let documents = stage_documents(request, &mut backup_paths)?;
    for work in documents.values() {
        let target = validate_absolute_owned_path(paths, &work.path)?;
        if work.changed {
            let exists = path_entry_exists(&target);
            let original_digest = existing_path_digest(&target);
            let backup = mutation_backup(paths, transaction_id, &target, work.persistent_backup)?;
            let staging = stage_bytes(&target, &work.updated)?;
            mutations.push(JournalMutation {
                target: target.display().to_string(),
                staging: Some(staging.display().to_string()),
                backup: exists.then(|| backup.display().to_string()),
                persistent_backup: work.persistent_backup,
                target_existed: exists,
                original_digest,
            });
        }
        let document_digest = ledger::bytes_digest(&work.updated);
        for planned in plan.resources.values() {
            match &planned.desired {
                DesiredResource::StructuredEntry(desired) if desired.document_path == work.path => {
                    installed.push(ResourceRecord {
                        id: planned.id.clone(),
                        identity: planned.desired.identity(),
                        desired_digest: planned.desired.desired_digest()?,
                        owned: OwnedResource::StructuredEntry(OwnedStructuredEntry {
                            document_path: work.path.display().to_string(),
                            format: desired.format,
                            key_path: desired.key_path.clone(),
                            value_digest: managed_documents::value_digest(&desired.value)?,
                            document_digest: document_digest.clone(),
                        }),
                        consumer_binding_ids: planned.consumer_binding_ids.clone(),
                        adapter_id: planned.adapter_id.clone(),
                        dialect_id: planned.dialect_id.clone(),
                    });
                }
                DesiredResource::TextBlock(desired) if desired.document_path == work.path => {
                    installed.push(ResourceRecord {
                        id: planned.id.clone(),
                        identity: planned.desired.identity(),
                        desired_digest: planned.desired.desired_digest()?,
                        owned: OwnedResource::TextBlock(OwnedTextBlock {
                            document_path: work.path.display().to_string(),
                            marker_id: desired.marker_id.clone(),
                            body_digest: ledger::bytes_digest(desired.body.trim_end().as_bytes()),
                            document_digest: document_digest.clone(),
                        }),
                        consumer_binding_ids: planned.consumer_binding_ids.clone(),
                        adapter_id: planned.adapter_id.clone(),
                        dialect_id: planned.dialect_id.clone(),
                    });
                }
                _ => {}
            }
        }
    }
    Ok((installed, backup_paths))
}

pub(super) fn stage_documents(
    request: &StageRequest<'_>,
    backup_paths: &mut Vec<PathBuf>,
) -> Result<BTreeMap<String, DocumentWork>, String> {
    let StageRequest {
        paths,
        transaction_id,
        plan,
        removed,
        remaining_ledger,
        replace_unmanaged,
        force_modified,
    } = *request;
    let mut grouped = BTreeMap::<String, (PathBuf, Option<StructuredFormat>)>::new();
    let mut apps = BTreeMap::<String, BTreeSet<String>>::new();
    for old in removed {
        match &old.owned {
            OwnedResource::StructuredEntry(owned) => {
                let key = normalize_path(Path::new(&owned.document_path));
                apps.entry(key.clone())
                    .or_default()
                    .insert(old.adapter_id.clone());
                grouped.insert(
                    key,
                    (PathBuf::from(&owned.document_path), Some(owned.format)),
                );
            }
            OwnedResource::TextBlock(owned) => {
                let key = normalize_path(Path::new(&owned.document_path));
                apps.entry(key.clone())
                    .or_default()
                    .insert(old.adapter_id.clone());
                grouped
                    .entry(key)
                    .or_insert((PathBuf::from(&owned.document_path), None));
            }
            OwnedResource::Path(_) => {}
        }
    }
    for planned in plan.resources.values() {
        match &planned.desired {
            DesiredResource::StructuredEntry(desired) => {
                let key = normalize_path(&desired.document_path);
                apps.entry(key.clone())
                    .or_default()
                    .insert(planned.adapter_id.clone());
                let entry = grouped
                    .entry(key)
                    .or_insert((desired.document_path.clone(), Some(desired.format)));
                if entry.1.is_some_and(|format| format != desired.format) {
                    return Err(format!(
                        "{} is planned with incompatible structured formats.",
                        desired.document_path.display()
                    ));
                }
                entry.1 = Some(desired.format);
            }
            DesiredResource::TextBlock(desired) => {
                let key = normalize_path(&desired.document_path);
                apps.entry(key.clone())
                    .or_default()
                    .insert(planned.adapter_id.clone());
                grouped
                    .entry(key)
                    .or_insert((desired.document_path.clone(), None));
            }
            DesiredResource::Path(_) => {}
        }
    }
    let desired_identities = plan
        .resources
        .values()
        .map(|resource| resource.desired.identity())
        .collect::<BTreeSet<_>>();
    let mut output = BTreeMap::new();
    for (key, (path, format)) in grouped {
        validate_absolute_owned_path(paths, &path)?;
        let named = |error: String| {
            managed_documents::document_error(
                &path,
                apps.get(&key).into_iter().flatten().map(String::as_str),
                &error,
            )
        };
        let mut persistent = false;
        for old in removed {
            if resource_document(old) != Some(&path) {
                continue;
            }
            let state = resource_state(paths, old);
            if !state.is_protected() {
                continue;
            }
            if force_modified {
                persistent = true;
            } else if !desired_identities.contains(&old.identity) {
                return Err(match state {
                    ContentState::Unknown(error) => error,
                    _ => format!("{} contains a modified managed entry.", path.display()),
                });
            }
        }
        let original = match fs::read(&path) {
            Ok(contents) => contents,
            // Nothing to take out of a file that is gone, and writing an empty one would bring
            // back the folder of an app that was uninstalled, so it would look installed again.
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && !plan
                        .resources
                        .values()
                        .any(|planned| match &planned.desired {
                            DesiredResource::StructuredEntry(desired) => {
                                desired.document_path == path
                            }
                            DesiredResource::TextBlock(desired) => desired.document_path == path,
                            DesiredResource::Path(_) => false,
                        }) =>
            {
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => match format {
                Some(format) => managed_documents::read_or_empty(&path, format)?,
                None => Vec::new(),
            },
            Err(error) => {
                return Err(named(format!(
                    "Could not read it: {}",
                    crate::fs_retry::plain(&error)
                )))
            }
        };
        let mut updated = original.clone();
        for old in removed {
            if desired_identities.contains(&old.identity) || resource_document(old) != Some(&path) {
                continue;
            }
            match &old.owned {
                OwnedResource::StructuredEntry(owned) => {
                    updated = managed_documents::remove_entries(
                        &updated,
                        owned.format,
                        std::slice::from_ref(&owned.key_path),
                    )
                    .map_err(named)?;
                }
                OwnedResource::TextBlock(owned) => {
                    updated = managed_documents::remove_text_blocks(
                        &updated,
                        std::slice::from_ref(&owned.marker_id),
                    )
                    .map_err(named)?;
                }
                OwnedResource::Path(_) => {}
            }
        }
        for planned in plan.resources.values() {
            let owned = remaining_ledger
                .resource_by_identity(&planned.desired.identity())
                .is_some()
                || removed
                    .iter()
                    .any(|old| old.identity == planned.desired.identity());
            match &planned.desired {
                DesiredResource::StructuredEntry(desired) if desired.document_path == path => {
                    // An identical unmanaged entry is adopted rather than refused.
                    let unmanaged = !owned
                        && managed_documents::entry_value(
                            &updated,
                            desired.format,
                            &desired.key_path,
                        )
                        .map_err(named)?
                        .is_some_and(|value| value != desired.value);
                    if unmanaged && !replace_unmanaged {
                        return Err(format!(
                            "Configuration entry {} in {} is unmanaged.",
                            desired.key_path.join("."),
                            path.display()
                        ));
                    }
                    persistent |= unmanaged;
                    updated = managed_documents::set_entries(
                        &updated,
                        desired.format,
                        &[(desired.key_path.clone(), desired.value.clone())],
                    )
                    .map_err(named)?;
                }
                DesiredResource::TextBlock(desired) if desired.document_path == path => {
                    let unmanaged = !owned
                        && managed_documents::text_block_body(&updated, &desired.marker_id)
                            .map_err(named)?
                            .is_some();
                    if unmanaged && !replace_unmanaged {
                        return Err(format!(
                            "Instruction block {} in {} is unmanaged.",
                            desired.marker_id,
                            path.display()
                        ));
                    }
                    persistent |= unmanaged;
                    updated = managed_documents::set_text_blocks(
                        &updated,
                        &[(desired.marker_id.clone(), desired.body.clone())],
                    )
                    .map_err(named)?;
                }
                _ => {}
            }
        }
        // An unchanged document still goes out: the entries it already holds
        // were just detached from the ledger and are recorded again from it.
        let changed = updated != original;
        if changed && persistent && path_entry_exists(&path) {
            backup_paths.push(mutation_backup(paths, transaction_id, &path, true)?);
        }
        output.insert(
            key,
            DocumentWork {
                path,
                changed,
                updated,
                persistent_backup: persistent,
            },
        );
    }
    Ok(output)
}

/// Whether an existing, unmanaged path already holds exactly what installing
/// `desired` would put there, so it can be taken over without a backup.
pub(super) fn identical_to_desired(desired: &crate::resource::DesiredPath, target: &Path) -> bool {
    let Some(existing) = existing_path_digest(target) else {
        return false;
    };
    let Ok(staging) = stage_path(desired, target) else {
        return false;
    };
    let same = ledger::path_digest(&staging, desired.kind).is_ok_and(|digest| digest == existing);
    let _ = remove_any(&staging);
    same
}

pub(super) fn stage_path(
    desired: &crate::resource::DesiredPath,
    target: &Path,
) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("{} has no parent.", target.display()))?;
    fs_retry::create_dir_all(parent).map_err(|error| {
        format!(
            "Could not create {}: {}",
            parent.display(),
            fs_retry::plain(&error)
        )
    })?;
    let staging = temporary_path(parent, "resource-installing");
    let result = match desired.kind {
        OwnedPathKind::Directory => match &desired.materialization {
            PathMaterialization::Copy => copy_directory(&desired.source, &staging),
            PathMaterialization::AgentSkill {
                effective_name,
                disable_model_invocation,
            } => materialize_agent_skill(
                &desired.source,
                &staging,
                effective_name,
                *disable_model_invocation,
            ),
        },
        OwnedPathKind::File => fs_retry::copy(&desired.source, &staging)
            .map(|_| ())
            .map_err(|error| {
                format!(
                    "Could not copy {} into {}: {}",
                    desired.source.display(),
                    parent.display(),
                    fs_retry::plain(&error)
                )
            }),
    };
    if let Err(error) = result {
        let _ = remove_any(&staging);
        return Err(error);
    }
    Ok(staging)
}

pub(super) fn stage_bytes(target: &Path, contents: &[u8]) -> Result<PathBuf, String> {
    let parent = target
        .parent()
        .ok_or_else(|| format!("{} has no parent.", target.display()))?;
    fs_retry::create_dir_all(parent).map_err(|error| {
        format!(
            "Could not create {}: {}",
            parent.display(),
            fs_retry::plain(&error)
        )
    })?;
    let staging = temporary_path(parent, "document-writing");
    if let Err(error) = fs_retry::write_synced(&staging, contents) {
        let _ = fs::remove_file(&staging);
        return Err(format!(
            "Could not save {}: {}",
            target.display(),
            fs_retry::plain(&error)
        ));
    }
    Ok(staging)
}

pub(super) fn mutation_backup(
    paths: &SystemPaths,
    transaction_id: &str,
    target: &Path,
    persistent: bool,
) -> Result<PathBuf, String> {
    if !persistent {
        return Ok(temporary_path(
            target.parent().expect("validated target has parent"),
            "resource-previous",
        ));
    }
    // Mirror the target's place under the home folder: every backup of a transaction is chosen
    // before anything moves, so Cursor's and VS Code's mcp.json (or a skill in two apps' folders)
    // would otherwise share one path, and the second move lost the first original.
    let backups = paths
        .home
        .join(".agents")
        .join(".agent-plugins-backups")
        .join(transaction_id);
    let directory = match target
        .parent()
        .and_then(|parent| parent.strip_prefix(&paths.home).ok())
    {
        Some(relative) => backups.join(relative),
        None => backups,
    };
    fs_retry::create_dir_all(&directory).map_err(|error| {
        format!(
            "Could not create {}: {}",
            directory.display(),
            fs_retry::plain(&error)
        )
    })?;
    let filename = target
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("resource");
    for suffix in 0..10_000_u16 {
        let name = if suffix == 0 {
            filename.to_string()
        } else {
            format!("{suffix}-{filename}")
        };
        let candidate = directory.join(name);
        if !path_entry_exists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "Could not choose a backup path in {}.",
        directory.display()
    ))
}
