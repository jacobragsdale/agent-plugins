//! The error every command and the scheduled-sync event hand the window.
//! Internals report plain `String`s; this sorts them by what the app can do
//! next, using the markers the modules that write them export.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum IpcErrorKind {
    /// A server could not be reached at all.
    Offline,
    /// Another app holds a file Agent Plugins needs.
    Locked,
    /// Trying again later can work without anyone doing anything.
    Retryable,
    /// A person has to decide or fix something first.
    NeedsUser,
    /// Agent Plugins itself failed.
    Bug,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct IpcError {
    pub(crate) kind: IpcErrorKind,
    /// The first sentence of the error.
    pub(crate) message: String,
    /// Everything after the first sentence, when there is more.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) detail: Option<String>,
}

impl From<String> for IpcError {
    fn from(text: String) -> Self {
        let kind = classify(&text);
        let (message, detail) = split_first_sentence(&text);
        Self {
            kind,
            message,
            detail,
        }
    }
}

fn classify(text: &str) -> IpcErrorKind {
    use crate::executor::{LOCAL_CHANGES, UNDO_PENDING, UNDO_RUNNING};
    // A refusal to touch a person's edits, or a ledger only a newer version
    // may change, stays the person's call whatever caused it underneath.
    if text.contains(LOCAL_CHANGES) || text.contains(crate::ledger::NEWER_LEDGER_MESSAGE) {
        IpcErrorKind::NeedsUser
    } else if text.contains(crate::application::WORKER_FAILED) && text.contains("panicked") {
        IpcErrorKind::Bug
    } else if crate::fs_retry::is_locked(text) {
        IpcErrorKind::Locked
    } else if crate::artifact::is_connect_failure(text) {
        IpcErrorKind::Offline
    } else if crate::artifact::is_transient(text)
        || text.contains(UNDO_PENDING)
        || text.contains(UNDO_RUNNING)
    {
        IpcErrorKind::Retryable
    } else {
        IpcErrorKind::NeedsUser
    }
}

/// Splits at the first full stop followed by white space.
fn split_first_sentence(text: &str) -> (String, Option<String>) {
    let text = text.trim();
    let end = text
        .char_indices()
        .zip(text.chars().skip(1))
        .find(|((_, current), next)| *current == '.' && next.is_whitespace())
        .map(|((index, _), _)| index + 1);
    match end {
        Some(end) if !text[end..].trim().is_empty() => (
            text[..end].to_string(),
            Some(text[end..].trim().to_string()),
        ),
        _ => (text.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(text: &str) -> IpcErrorKind {
        IpcError::from(text.to_string()).kind
    }

    #[test]
    fn classifies_by_the_owning_modules_markers() {
        let cases = [
            (
                "Could not connect to https://repo.example.com/a.zip: dns error",
                IpcErrorKind::Offline,
            ),
            ("https://repo.example.com/a.zip timed out.", IpcErrorKind::Retryable),
            (
                "Could not download the source (HTTP 503): Service Unavailable",
                IpcErrorKind::Retryable,
            ),
            (
                "Could not replace C:\\skills\\publish: another app is using it. Close that app, then try again.",
                IpcErrorKind::Locked,
            ),
            (
                "The process cannot access the file because it is being used by another process. (os error 32)",
                IpcErrorKind::Locked,
            ),
            (
                "review/publish contains local changes and cannot be replaced.",
                IpcErrorKind::NeedsUser,
            ),
            (
                "review/publish could not be checked, so it cannot be removed. Claude Code uses a settings file that Agent Plugins could not read: C:\\x.json. Fix or remove that file, then try again.",
                IpcErrorKind::NeedsUser,
            ),
            (crate::ledger::NEWER_LEDGER_MESSAGE, IpcErrorKind::NeedsUser),
            (
                "An earlier change could not be undone yet: rename failed Close any app that is using these files. Agent Plugins tries again the next time it checks for updates.",
                IpcErrorKind::Retryable,
            ),
            (
                "An earlier change is still being undone. Close any app that is using its files, then try again.",
                IpcErrorKind::Retryable,
            ),
            (
                "Install worker failed: task 12 panicked with message \"oops\"",
                IpcErrorKind::Bug,
            ),
            ("Unknown catalog item: review/nope", IpcErrorKind::NeedsUser),
        ];
        for (text, expected) in cases {
            assert_eq!(kind(text), expected, "{text}");
        }
    }

    #[test]
    fn splits_the_first_sentence_from_the_rest() {
        let error = IpcError::from(
            "An earlier change is still being undone. Close any app, then try again.".to_string(),
        );
        assert_eq!(error.message, "An earlier change is still being undone.");
        assert_eq!(
            error.detail.as_deref(),
            Some("Close any app, then try again.")
        );

        let single = IpcError::from("C:\\Users\\sam\\.claude contains local changes.".to_string());
        assert_eq!(
            single.message,
            "C:\\Users\\sam\\.claude contains local changes."
        );
        assert_eq!(single.detail, None);
    }

    #[test]
    fn serializes_the_shape_the_window_parses() {
        let value = serde_json::to_value(IpcError::from("Offline.".to_string())).expect("json");
        assert_eq!(
            value,
            serde_json::json!({ "kind": "needsUser", "message": "Offline." })
        );
        let value = serde_json::to_value(IpcError::from("One. Two.".to_string())).expect("json");
        assert_eq!(value["detail"], "Two.");
    }
}
