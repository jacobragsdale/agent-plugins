//! Recovery journal, activation, and rollback.

use super::{existing_path_digest, path_entry_exists, remove_any};
use crate::fs_retry;
use crate::ledger::{self, InstallationLedger, LegacyPathRoots};
use crate::paths::SystemPaths;
use crate::sources::{sync_directory, temporary_path};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub(super) const JOURNAL_FILE: &str = "resource-transaction.json";
const PARKED_PREFIX: &str = "resource-transaction.cleanup-";
/// An interrupted change that recovery could not roll back; the next sync retries.
pub(crate) const UNDO_PENDING: &str = "could not be undone yet";
/// A new change found an interrupted one still waiting to be rolled back.
pub(crate) const UNDO_RUNNING: &str = "is still being undone";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct TransactionJournal {
    pub(super) version: u8,
    pub(super) transaction_id: String,
    pub(super) mutations: Vec<JournalMutation>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct JournalMutation {
    pub(super) target: String,
    pub(super) staging: Option<String>,
    pub(super) backup: Option<String>,
    pub(super) persistent_backup: bool,
    pub(super) target_existed: bool,
    pub(super) original_digest: Option<String>,
}

/// Finishes or undoes a transaction a crash or failure left behind. A journal
/// that cannot be parsed is moved aside instead of blocking every read, and a
/// committed transaction whose leftovers cannot be removed yet is parked for a
/// later sweep. Only a rollback that fails keeps the journal, so the next
/// start retries it and no new change can start on top of it.
pub(crate) fn recover(paths: &SystemPaths) -> Result<(), String> {
    let journal_path = paths.app_data().join(JOURNAL_FILE);
    let contents = match fs::read(&journal_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            sweep_parked_cleanups(paths);
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "Could not read {}: {}",
                journal_path.display(),
                fs_retry::plain(&error)
            ))
        }
    };
    let journal = match serde_json::from_slice::<TransactionJournal>(&contents) {
        Ok(journal) if journal.version == 1 => journal,
        Ok(_) => return quarantine_journal(paths, "it uses an unsupported version"),
        Err(error) => return quarantine_journal(paths, &error.to_string()),
    };
    let committed = read_ledger_raw(paths)?.last_transaction_id.as_deref()
        == Some(journal.transaction_id.as_str());
    if committed {
        finish_committed(paths, &journal);
    } else {
        rollback(&journal).map_err(|error| {
            format!(
                "An earlier change {UNDO_PENDING}: {error} Close any app that is using these files. Agent Plugins tries again the next time it checks for updates."
            )
        })?;
        remove_journal(paths);
    }
    sweep_parked_cleanups(paths);
    Ok(())
}

/// Removes a committed transaction's staging and backups. What cannot be
/// removed yet stays listed in a parked copy of the journal, and the
/// transaction still counts as committed.
pub(super) fn finish_committed(paths: &SystemPaths, journal: &TransactionJournal) {
    let journal_path = paths.app_data().join(JOURNAL_FILE);
    if let Err(error) = cleanup_committed(journal) {
        let parked = paths
            .app_data()
            .join(format!("{PARKED_PREFIX}{}.json", journal.transaction_id));
        eprintln!("Will retry cleaning up a finished change later: {error}");
        if fs_retry::rename(&journal_path, &parked).is_err() {
            return;
        }
    } else {
        remove_journal(paths);
    }
    if let Err(error) = sync_directory(&paths.app_data()) {
        eprintln!("{error}");
    }
}

fn remove_journal(paths: &SystemPaths) {
    let journal_path = paths.app_data().join(JOURNAL_FILE);
    if let Err(error) = fs_retry::remove_file(&journal_path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            eprintln!("Could not remove {}: {error}", journal_path.display());
        }
    }
}

fn quarantine_journal(paths: &SystemPaths, reason: &str) -> Result<(), String> {
    let journal_path = paths.app_data().join(JOURNAL_FILE);
    let target = ledger::quarantine_path(&paths.app_data(), &format!("{JOURNAL_FILE}.corrupt-"));
    fs_retry::rename(&journal_path, &target).map_err(|error| {
        format!(
            "Could not move the damaged {} aside: {}",
            journal_path.display(),
            fs_retry::plain(&error)
        )
    })?;
    eprintln!(
        "Moved the unreadable {} aside to {} because {reason}.",
        journal_path.display(),
        target.display()
    );
    sync_directory(&paths.app_data())
}

/// Retries the cleanup of committed transactions parked by
/// [`finish_committed`]. Best effort: what still fails stays parked.
fn sweep_parked_cleanups(paths: &SystemPaths) {
    let Ok(entries) = fs::read_dir(paths.app_data()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(PARKED_PREFIX) {
            continue;
        }
        let path = entry.path();
        let done = match fs::read(&path)
            .ok()
            .and_then(|contents| serde_json::from_slice::<TransactionJournal>(&contents).ok())
        {
            Some(journal) => cleanup_committed(&journal).is_ok(),
            None => true,
        };
        if done {
            let _ = fs_retry::remove_file(&path);
        }
    }
}

pub(super) fn read_ledger_raw(paths: &SystemPaths) -> Result<InstallationLedger, String> {
    ledger::read(
        &paths.app_data(),
        LegacyPathRoots {
            home: &paths.home,
            config: &paths.config,
            data: &paths.data,
            local_data: &paths.local_data,
            cache: &paths.cache,
        },
    )
}

pub(super) fn write_journal(
    paths: &SystemPaths,
    journal: &TransactionJournal,
) -> Result<(), String> {
    fs_retry::create_dir_all(&paths.app_data()).map_err(|error| {
        format!(
            "Could not create {}: {}",
            paths.app_data().display(),
            fs_retry::plain(&error)
        )
    })?;
    let path = paths.app_data().join(JOURNAL_FILE);
    if path.exists() {
        return Err(format!(
            "An earlier change {UNDO_RUNNING}. Close any app that is using its files, then try again."
        ));
    }
    let staging = temporary_path(&paths.app_data(), "resource-transaction-writing");
    let mut bytes = serde_json::to_vec_pretty(journal)
        .map_err(|error| format!("Could not serialize the transaction journal: {error}"))?;
    bytes.push(b'\n');
    let written =
        fs_retry::write_synced(&staging, &bytes).and_then(|()| fs_retry::rename(&staging, &path));
    if let Err(error) = written {
        let _ = fs::remove_file(&staging);
        return Err(format!(
            "Could not record the change in {}: {}",
            paths.app_data().display(),
            fs_retry::plain(&error)
        ));
    }
    sync_directory(&paths.app_data())
}

pub(super) fn activate(journal: &TransactionJournal) -> Result<(), String> {
    for mutation in &journal.mutations {
        let target = Path::new(&mutation.target);
        if let Some(expected) = &mutation.original_digest {
            let current = existing_path_digest(target);
            if current.as_ref() != Some(expected) {
                return Err(format!(
                    "{} changed after preflight; no changes were committed.",
                    target.display()
                ));
            }
        } else if mutation.target_existed != path_entry_exists(target) {
            return Err(format!(
                "{} changed existence after preflight; no changes were committed.",
                target.display()
            ));
        }
        if let Some(backup) = &mutation.backup {
            let backup = Path::new(backup);
            if let Some(parent) = backup.parent() {
                fs_retry::create_dir_all(parent).map_err(|error| {
                    format!(
                        "Could not create {}: {}",
                        parent.display(),
                        fs_retry::plain(&error)
                    )
                })?;
            }
            fs_retry::rename(target, backup).map_err(|error| {
                format!(
                    "Could not change {}: {}",
                    target.display(),
                    fs_retry::plain(&error)
                )
            })?;
        }
        if let Some(staging) = &mutation.staging {
            let staging = Path::new(staging);
            fs_retry::rename(staging, target).map_err(|error| {
                format!(
                    "Could not save {}: {}",
                    target.display(),
                    fs_retry::plain(&error)
                )
            })?;
        }
        if let Some(parent) = target.parent() {
            sync_directory(parent)?;
        }
    }
    Ok(())
}

pub(super) fn rollback(journal: &TransactionJournal) -> Result<(), String> {
    let mut errors = Vec::new();
    for mutation in journal.mutations.iter().rev() {
        let target = Path::new(&mutation.target);
        if let Some(backup) = &mutation.backup {
            let backup = Path::new(backup);
            if path_entry_exists(backup) {
                if path_entry_exists(target) {
                    if let Err(error) = remove_any(target) {
                        errors.push(error);
                        continue;
                    }
                }
                if let Err(error) = fs_retry::rename(backup, target) {
                    errors.push(format!("Could not restore {}: {error}", target.display()));
                }
            }
        } else if let Some(staging) = &mutation.staging {
            if !path_entry_exists(Path::new(staging)) && path_entry_exists(target) {
                if let Err(error) = remove_any(target) {
                    errors.push(error);
                }
            }
        }
        if let Some(staging) = &mutation.staging {
            let staging = Path::new(staging);
            if path_entry_exists(staging) {
                let _ = remove_any(staging);
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" "))
    }
}

pub(super) fn cleanup_committed(journal: &TransactionJournal) -> Result<(), String> {
    for mutation in &journal.mutations {
        if let Some(staging) = &mutation.staging {
            let staging = Path::new(staging);
            if path_entry_exists(staging) {
                remove_any(staging)?;
            }
        }
        if !mutation.persistent_backup {
            if let Some(backup) = &mutation.backup {
                let backup = Path::new(backup);
                if path_entry_exists(backup) {
                    remove_any(backup)?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn cleanup_mutations(mutations: &[JournalMutation]) {
    for mutation in mutations {
        if let Some(staging) = &mutation.staging {
            let _ = remove_any(Path::new(staging));
        }
    }
}

pub(super) fn cleanup_staging(journal: &TransactionJournal) {
    cleanup_mutations(&journal.mutations);
}
