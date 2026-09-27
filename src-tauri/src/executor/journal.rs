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

const LOCK_FILE: &str = "transaction.lock";

/// Serializes ledger changes across processes. The window and the command
/// line both change the ledger, and recovery must never roll back a journal
/// another process is still activating. The OS releases the lock when its
/// holder exits, so a crash never leaves it held. Within one process the
/// operation lock already serializes changes, so holding it nests.
pub(crate) struct TransactionLock(std::path::PathBuf);

type HeldLocks = std::collections::HashMap<std::path::PathBuf, (fs::File, usize)>;

/// The lock files this process holds, by path, and how many holders share each.
fn held() -> std::sync::MutexGuard<'static, HeldLocks> {
    static HELD: std::sync::OnceLock<std::sync::Mutex<HeldLocks>> = std::sync::OnceLock::new();
    HELD.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl TransactionLock {
    /// Waits for a change in another process to finish, then holds the lock.
    pub(crate) fn acquire(paths: &SystemPaths) -> Result<Self, String> {
        let path = paths.app_data().join(LOCK_FILE);
        if let Some((_, holders)) = held().get_mut(&path) {
            *holders += 1;
            return Ok(Self(path));
        }
        let file = open_lock_file(paths)?;
        file.lock()
            .map_err(|error| format!("Could not lock {}: {error}", path.display()))?;
        held()
            .entry(path.clone())
            .and_modify(|(_, holders)| *holders += 1)
            .or_insert((file, 1));
        Ok(Self(path))
    }

    /// Holds the lock only when nobody holds it now, this process included:
    /// a holder is in the middle of a change and owns its journal.
    fn try_acquire(paths: &SystemPaths) -> Option<Self> {
        let path = paths.app_data().join(LOCK_FILE);
        let mut held = held();
        if held.contains_key(&path) {
            return None;
        }
        let file = open_lock_file(paths).ok()?;
        file.try_lock().ok()?;
        held.insert(path.clone(), (file, 1));
        Some(Self(path))
    }
}

impl Drop for TransactionLock {
    fn drop(&mut self) {
        let mut held = held();
        if let Some((_, holders)) = held.get_mut(&self.0) {
            *holders -= 1;
            if *holders == 0 {
                // Closing the file releases the lock.
                held.remove(&self.0);
            }
        }
    }
}

fn open_lock_file(paths: &SystemPaths) -> Result<fs::File, String> {
    let directory = paths.app_data();
    fs_retry::create_dir_all(&directory).map_err(|error| {
        format!(
            "Could not create {}: {}",
            directory.display(),
            fs_retry::plain(&error)
        )
    })?;
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join(LOCK_FILE))
        .map_err(|error| format!("Could not open {LOCK_FILE}: {}", fs_retry::plain(&error)))
}

/// Recovery for a read: skipped while any change holds the lock, because its
/// journal is live. Reads never wait for a change to finish.
pub(super) fn recover_unless_busy(paths: &SystemPaths) -> Result<(), String> {
    match TransactionLock::try_acquire(paths) {
        Some(_lock) => recover(paths),
        None => Ok(()),
    }
}

/// Finishes or undoes a transaction a crash or failure left behind. A journal
/// that cannot be parsed is moved aside instead of blocking every read, and a
/// committed transaction whose leftovers cannot be removed yet is parked for a
/// later sweep. Only a rollback that fails keeps the journal, so the next
/// start retries it and no new change can start on top of it.
/// An undone transaction whose journal could not be removed, and when. Every
/// read runs recovery, and each removal waits out the file retry budget, so
/// for a while after a failure one quick try stands in for another full one.
static STUCK_JOURNAL: std::sync::Mutex<Option<(std::path::PathBuf, std::time::Instant, String)>> =
    std::sync::Mutex::new(None);
const STUCK_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(30);

pub(crate) fn recover(paths: &SystemPaths) -> Result<(), String> {
    let journal_path = paths.app_data().join(JOURNAL_FILE);
    let stuck = STUCK_JOURNAL
        .lock()
        .ok()
        .and_then(|stuck| stuck.clone())
        .filter(|(path, since, _)| *path == journal_path && since.elapsed() < STUCK_RETRY_AFTER);
    if let Some((_, _, message)) = stuck {
        // The undo itself already ran; only the journal is left to remove.
        if fs::remove_file(&journal_path).is_err() && journal_path.exists() {
            return Err(message);
        }
        if let Ok(mut stuck) = STUCK_JOURNAL.lock() {
            *stuck = None;
        }
    }
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
        // A journal left behind would be rolled back again at every read, over
        // whatever changed since, so nothing may change until it is gone.
        remove_journal(paths).map_err(|error| {
            let message = format!(
                "An earlier change {UNDO_PENDING}: Could not remove {}: {} Close any app that is using it. Agent Plugins tries again the next time it checks for updates.",
                paths.app_data().join(JOURNAL_FILE).display(),
                fs_retry::plain(&error)
            );
            if let Ok(mut stuck) = STUCK_JOURNAL.lock() {
                *stuck = Some((
                    paths.app_data().join(JOURNAL_FILE),
                    std::time::Instant::now(),
                    message.clone(),
                ));
            }
            message
        })?;
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
    } else if let Err(error) = remove_journal(paths) {
        eprintln!(
            "Could not remove {}: {error}",
            paths.app_data().join(JOURNAL_FILE).display()
        );
    }
    if let Err(error) = sync_directory(&paths.app_data()) {
        eprintln!("{error}");
    }
}

fn remove_journal(paths: &SystemPaths) -> std::io::Result<()> {
    match fs_retry::remove_file(&paths.app_data().join(JOURNAL_FILE)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
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
