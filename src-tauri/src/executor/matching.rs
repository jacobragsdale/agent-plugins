//! Ledger and filesystem matching for planned resources.

use super::validate_absolute_owned_path;
use crate::ledger::{self, InstallationLedger, OwnedPathKind, OwnedResource, ResourceRecord};
use crate::managed_documents;
use crate::paths::SystemPaths;
use crate::resource::OperationPlan;
use std::fs;
use std::path::Path;

/// What an owned resource looks like on disk compared with what was installed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ContentState {
    Match,
    /// The user or another app changed it.
    Modified,
    /// It is gone. The sync re-creates it; an uninstall has nothing to remove.
    Missing,
    /// It could not be read, so nothing is known about it.
    Unknown(String),
}

impl ContentState {
    /// The state of several resources together: a change outranks a read
    /// failure, which outranks a missing file.
    fn combine(self, other: Self) -> Self {
        fn rank(state: &ContentState) -> u8 {
            match state {
                ContentState::Match => 0,
                ContentState::Missing => 1,
                ContentState::Unknown(_) => 2,
                ContentState::Modified => 3,
            }
        }
        if rank(&other) > rank(&self) {
            other
        } else {
            self
        }
    }

    /// Modified or unreadable content, which only a confirmed operation may touch.
    pub(crate) fn is_protected(&self) -> bool {
        matches!(self, Self::Modified | Self::Unknown(_))
    }
}

/// The combined state of an installation's resources, limited to some
/// components when `component_ids` is given.
pub(crate) fn installation_state(
    paths: &SystemPaths,
    ledger: &InstallationLedger,
    installation_id: &str,
    component_ids: Option<&[String]>,
) -> ContentState {
    let Some(record) = ledger.items.get(installation_id) else {
        return ContentState::Match;
    };
    let mut state = ContentState::Match;
    for binding_id in &record.binding_ids {
        let Some(binding) = ledger.bindings.get(binding_id) else {
            continue;
        };
        if component_ids.is_some_and(|ids| !ids.contains(&binding.component_id)) {
            continue;
        }
        for resource_id in &binding.resource_ids {
            let resource_state = ledger
                .resources
                .get(resource_id)
                .map_or(ContentState::Missing, |resource| {
                    resource_state(paths, resource)
                });
            state = state.combine(resource_state);
        }
    }
    state
}

pub(crate) fn resource_state(paths: &SystemPaths, resource: &ResourceRecord) -> ContentState {
    match &resource.owned {
        OwnedResource::Path(owned) => {
            let path = match validate_absolute_owned_path(paths, Path::new(&owned.path)) {
                Ok(path) => path,
                Err(error) => return ContentState::Unknown(error),
            };
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return ContentState::Missing
                }
                Err(error) => {
                    return ContentState::Unknown(format!(
                        "Could not check {}: {}",
                        path.display(),
                        crate::fs_retry::plain(&error)
                    ))
                }
            };
            if metadata.is_dir() != (owned.kind == OwnedPathKind::Directory) {
                return ContentState::Modified;
            }
            match ledger::path_digest(&path, owned.kind) {
                Ok(digest) if digest == owned.installed_digest => ContentState::Match,
                Ok(_) => ContentState::Modified,
                Err(error) => ContentState::Unknown(error),
            }
        }
        OwnedResource::StructuredEntry(owned) => {
            let path = Path::new(&owned.document_path);
            let contents = match read_document(paths, path) {
                Ok(Some(contents)) => contents,
                Ok(None) => return ContentState::Missing,
                Err(error) => return ContentState::Unknown(error),
            };
            let value =
                match managed_documents::entry_value(&contents, owned.format, &owned.key_path) {
                    Ok(Some(value)) => value,
                    Ok(None) => return ContentState::Missing,
                    Err(error) => {
                        return ContentState::Unknown(managed_documents::document_error(
                            path,
                            [resource.adapter_id.as_str()],
                            &error,
                        ))
                    }
                };
            match managed_documents::value_digest(&value) {
                Ok(digest) if digest == owned.value_digest => ContentState::Match,
                Ok(_) => ContentState::Modified,
                Err(error) => ContentState::Unknown(error),
            }
        }
        OwnedResource::TextBlock(owned) => {
            let path = Path::new(&owned.document_path);
            let contents = match read_document(paths, path) {
                Ok(Some(contents)) => contents,
                Ok(None) => return ContentState::Missing,
                Err(error) => return ContentState::Unknown(error),
            };
            match managed_documents::text_block_body(&contents, &owned.marker_id) {
                Ok(Some(body)) if ledger::bytes_digest(body.as_bytes()) == owned.body_digest => {
                    ContentState::Match
                }
                Ok(Some(_)) | Err(_) => ContentState::Modified,
                Ok(None) => ContentState::Missing,
            }
        }
    }
}

/// A managed document's bytes, or `None` when it does not exist.
fn read_document(paths: &SystemPaths, path: &Path) -> Result<Option<Vec<u8>>, String> {
    let path = validate_absolute_owned_path(paths, path)?;
    match fs::read(&path) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Could not read {}: {}",
            path.display(),
            crate::fs_retry::plain(&error)
        )),
    }
}

pub(crate) fn plan_satisfied(
    ledger: &InstallationLedger,
    plan: &OperationPlan,
) -> Result<bool, String> {
    if plan
        .bindings
        .keys()
        .any(|binding_id| !ledger.bindings.contains_key(binding_id))
    {
        return Ok(false);
    }
    for planned in plan.resources.values() {
        let Some(existing) = ledger.resource_by_identity(&planned.desired.identity()) else {
            return Ok(false);
        };
        if existing.desired_digest != planned.desired.desired_digest()? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(super) fn plan_matches_ledger(
    ledger: &InstallationLedger,
    plan: &OperationPlan,
) -> Result<bool, String> {
    plan_satisfied(ledger, plan)
}
