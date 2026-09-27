//! Desktop notifications for what happened while the window was out of
//! sight: skills removed or updated in the background, and marketplace news
//! such as a suggestion waiting or a connector approved.

use crate::app_state::AppState;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

/// At most this many notifications per sync; the rest fold into one line.
const MOST_AT_ONCE: usize = 3;

pub(crate) fn after_sync<R: Runtime>(app: &AppHandle<R>, state: &AppState) {
    let in_view = app
        .get_webview_window("main")
        .is_some_and(|window| window.is_focused().unwrap_or(false));
    if in_view {
        return;
    }
    let lines = lines(state);
    let extra = lines.len().saturating_sub(MOST_AT_ONCE);
    for line in lines.iter().take(MOST_AT_ONCE) {
        show(app, line);
    }
    if extra > 0 {
        show(
            app,
            &format!("And {extra} more. Open Agent Plugins to see them."),
        );
    }
}

/// What the person would want to know, most important first.
fn lines(state: &AppState) -> Vec<String> {
    let report = &state.auto_update_report;
    let name = |id: &str| {
        state
            .items
            .iter()
            .find(|item| item.id == id)
            .map_or_else(|| id.to_string(), |item| item.name.clone())
    };
    report
        .removed_items
        .iter()
        .map(|removed| format!("{removed} was removed from this computer by its publisher or IT."))
        .chain(
            report
                .updated_items
                .iter()
                .map(|updated| match &updated.to_version {
                    Some(version) => {
                        format!("{} was updated to version {version}.", name(&updated.id))
                    }
                    None => format!("{} was updated.", name(&updated.id)),
                }),
        )
        .chain(state.notifications.iter().map(|news| news.text.clone()))
        .collect()
}

pub(crate) fn show<R: Runtime>(app: &AppHandle<R>, body: &str) {
    if let Err(error) = app
        .notification()
        .builder()
        .title("Agent Plugins")
        .body(body)
        .show()
    {
        eprintln!("Could not show a notification: {error}");
    }
}
