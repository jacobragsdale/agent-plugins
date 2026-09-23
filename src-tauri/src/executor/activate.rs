//! Ledger commit after a staged journal activates.

use super::journal::{activate, cleanup_staging, finish_committed, rollback, write_journal};
use super::{resource_document, transaction_id, TransactionJournal, JOURNAL_FILE};
use crate::fs_retry;
use crate::ledger::{self, InstallationLedger, OwnedResource};
use crate::paths::SystemPaths;
use crate::sources::sync_directory;
use std::fs;
use std::path::Path;

pub(super) fn commit(
    paths: &SystemPaths,
    journal: &TransactionJournal,
    ledger_state: &InstallationLedger,
) -> Result<(), String> {
    if let Err(error) = write_journal(paths, journal) {
        cleanup_staging(journal);
        return Err(error);
    }
    if let Err(error) = activate(journal) {
        return Err(undo(paths, journal, error));
    }
    if let Err(error) = ledger::write(&paths.app_data(), ledger_state) {
        return Err(undo(paths, journal, error));
    }
    finish_committed(paths, journal);
    Ok(())
}

/// Rolls back a transaction that failed before its ledger committed. The
/// journal is removed only when the rollback succeeded; otherwise it stays so
/// recovery retries the rollback and restores the user's files.
fn undo(paths: &SystemPaths, journal: &TransactionJournal, error: String) -> String {
    match rollback(journal) {
        Ok(()) => {
            if let Err(remove_error) = fs_retry::remove_file(&paths.app_data().join(JOURNAL_FILE))
            {
                eprintln!("Could not remove the rolled-back transaction journal: {remove_error}");
            }
            if let Err(sync_error) = sync_directory(&paths.app_data()) {
                eprintln!("{sync_error}");
            }
            error
        }
        Err(rollback_error) => format!(
            "{error} Some files could not be put back yet ({rollback_error}). Agent Plugins tries again the next time it checks for updates."
        ),
    }
}

pub(super) fn persist_reset_ledger(
    paths: &SystemPaths,
    next: &mut InstallationLedger,
    label: &str,
) -> Result<(), String> {
    next.last_transaction_id = Some(transaction_id(label));
    crate::ledger::write(&paths.app_data(), next)
}

pub(super) fn update_document_digests_from_journal(
    ledger: &mut InstallationLedger,
    journal: &TransactionJournal,
) -> Result<(), String> {
    for mutation in &journal.mutations {
        let Some(staging) = &mutation.staging else {
            continue;
        };
        let target = Path::new(&mutation.target);
        let affects_document = ledger
            .resources
            .values()
            .any(|resource| resource_document(resource).is_some_and(|path| path == target));
        if !affects_document {
            continue;
        }
        let bytes = fs::read(staging).map_err(|error| {
            cleanup_staging(journal);
            format!("Could not hash {staging}: {error}")
        })?;
        let digest = ledger::bytes_digest(&bytes);
        for resource in ledger.resources.values_mut() {
            match &mut resource.owned {
                OwnedResource::StructuredEntry(owned)
                    if Path::new(&owned.document_path) == target =>
                {
                    owned.document_digest.clone_from(&digest);
                }
                OwnedResource::TextBlock(owned) if Path::new(&owned.document_path) == target => {
                    owned.document_digest.clone_from(&digest);
                }
                _ => {}
            }
        }
    }
    Ok(())
}
