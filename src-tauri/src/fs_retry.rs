//! Filesystem mutations that survive Windows' transient sharing failures.
//!
//! Windows keeps a handle open on a file for a short while after it is
//! written: antivirus scanners, the search indexer, and Explorer all take
//! brief opportunistic handles. A rename or delete issued inside that window
//! fails with a sharing violation even though the identical call succeeds a
//! moment later. Agent Plugins renames and deletes directories immediately
//! after writing them, so it meets that window regularly on Windows and
//! effectively never on Unix.
//!
//! These wrappers retry only the errors that are known to be transient, and
//! only on Windows. Every other platform calls straight through to `std::fs`.

use std::io::{self, Write};
use std::path::Path;
#[cfg(windows)]
use std::time::{Duration, Instant};

/// How long a transient failure is retried. A scanner or OneDrive usually lets
/// go within a second or two; a genuinely locked file fails after this.
#[cfg(windows)]
const RETRY_BUDGET: Duration = Duration::from_secs(4);
#[cfg(windows)]
const MAX_DELAY: Duration = Duration::from_millis(500);

/// `ERROR_ACCESS_DENIED`, `ERROR_SHARING_VIOLATION`, `ERROR_LOCK_VIOLATION`,
/// and `ERROR_DIR_NOT_EMPTY`. The last appears when a directory delete races a
/// scanner that still holds one of the children.
#[cfg(windows)]
fn is_transient(error: &io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5 | 32 | 33 | 145))
}

#[cfg(windows)]
fn retrying<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let started = Instant::now();
    let mut delay = Duration::from_millis(10);
    loop {
        let outcome = operation();
        match &outcome {
            Err(error) if is_transient(error) && started.elapsed() < RETRY_BUDGET => {}
            _ => return outcome,
        }
        std::thread::sleep(delay);
        delay = (delay * 2).min(MAX_DELAY);
    }
}

#[cfg(not(windows))]
fn retrying<T>(mut operation: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    operation()
}

pub(crate) fn rename(from: &Path, to: &Path) -> io::Result<()> {
    retrying(|| std::fs::rename(from, to))
}

pub(crate) fn remove_dir_all(path: &Path) -> io::Result<()> {
    retrying(|| std::fs::remove_dir_all(path))
}

pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    retrying(|| std::fs::remove_file(path))
}

pub(crate) fn create_dir_all(path: &Path) -> io::Result<()> {
    retrying(|| std::fs::create_dir_all(path))
}

pub(crate) fn copy(from: &Path, to: &Path) -> io::Result<u64> {
    retrying(|| std::fs::copy(from, to))
}

/// Creates `path` and flushes it to disk before returning, so a crash cannot
/// leave a journal or staged document that exists but is empty.
pub(crate) fn write_synced(path: &Path, contents: &[u8]) -> io::Result<()> {
    retrying(|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.write_all(contents)?;
        file.sync_all()
    })
}

/// Replaces a file through a synced sibling and a rename, so a failure leaves
/// either the old or the new contents and never half of each.
pub(crate) fn replace_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let staging = crate::sources::temporary_path(parent, "document-writing");
    let result = write_synced(&staging, contents).and_then(|()| rename(&staging, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&staging);
    }
    result
}

/// How `plain` words a sharing or lock violation.
const IN_USE: &str = "another app is using it";

/// True when an error message says another app holds the file: `plain`'s
/// wording, or the raw Windows sharing and lock violations.
pub(crate) fn is_locked(message: &str) -> bool {
    message.contains(IN_USE)
        || message.contains("(os error 32)")
        || message.contains("(os error 33)")
}

/// An I/O error in words a non-developer can act on. Full disks and denied
/// access are the two failures people can fix themselves.
pub(crate) fn plain(error: &io::Error) -> String {
    // ERROR_HANDLE_DISK_FULL and ERROR_DISK_FULL on Windows.
    if error.kind() == io::ErrorKind::StorageFull || matches!(error.raw_os_error(), Some(39 | 112))
    {
        return "the disk is full. Free up some space, then try again.".to_string();
    }
    // ERROR_SHARING_VIOLATION and ERROR_LOCK_VIOLATION on Windows.
    if cfg!(windows) && matches!(error.raw_os_error(), Some(32 | 33)) {
        return format!("{IN_USE}. Close that app, then try again.");
    }
    if error.kind() == io::ErrorKind::PermissionDenied {
        return "access was denied. Close any app that is using it and check that you can change files there, then try again.".to_string();
    }
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_file_swaps_contents_and_leaves_no_staging() {
        let root = tempfile::tempdir().expect("root");
        let path = root.path().join("config.json");
        std::fs::write(&path, "old").expect("old");
        replace_file(&path, b"new").expect("replace");
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "new");
        assert_eq!(std::fs::read_dir(root.path()).expect("dir").count(), 1);
    }

    #[test]
    fn plain_names_a_full_disk() {
        let error = io::Error::from(io::ErrorKind::StorageFull);
        assert!(plain(&error).contains("disk is full"));
    }
}
