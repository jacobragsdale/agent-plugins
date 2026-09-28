#![cfg_attr(not(feature = "app"), allow(dead_code, unused_imports))]

mod adapters;
mod agent_profiles;
mod app_locations;
mod app_state;
mod application;
mod artifact;
mod catalog;
mod choices;
#[cfg(feature = "app")]
mod cli;
mod deep_link;
mod digest;
mod executor;
mod fs_retry;
mod host_identity;
mod install;
mod invocation;
#[cfg(feature = "app")]
mod ipc;
mod ipc_error;
mod ledger;
mod locator;
mod managed_documents;
pub mod manifest;
mod marketplace;
mod mcp;
#[cfg(feature = "app")]
mod notify;
mod parallel;
mod paths;
mod planner;
mod preflight;
mod process;
mod qa_paths;
pub mod repository;
mod resource;
mod source;
mod sources;
pub mod staging;
mod startup;
mod tutorial;

/// The window is running, so marketplace news can be shown. A command-line
/// sync leaves it unread for the window to show later.
pub(crate) static WINDOW_RUNNING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The host preparation report, for the preflight.
pub(crate) static STARTUP_REPORT: startup::SharedReport = startup::SharedReport::new();

/// Runs host preparation for the command line, `uv` download included. The
/// report stays out of the terminal: an agent reading stderr would take it
/// for a problem, and preflight reports the same facts.
pub(crate) fn prepare_host() {
    STARTUP_REPORT.set(startup::prepare());
}

pub use repository::{
    validate_source_repository, RepositoryValidationError, RepositoryValidationReport,
};
pub use source::{
    validate_source, validate_source_locator, validate_source_repository_locator,
    SourceValidationError, SourceValidationReport,
};
#[cfg(all(feature = "app", desktop))]
mod tray;

#[cfg(feature = "app")]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Some(code) = cli::maybe_run() {
        std::process::exit(code);
    }
    startup::log_to_file();
    WINDOW_RUNNING.store(true, std::sync::atomic::Ordering::Relaxed);
    let runtime_state = application::RuntimeState::new();
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        // A second launch is often a portal link; the running window gets it.
        if let Some(link) = deep_link::from_args(&args) {
            deep_link::set_pending(link.clone());
            if let Err(error) = tauri::Emitter::emit(app, deep_link::EVENT, link) {
                eprintln!("Could not hand the link to the window: {error}");
            }
        }
        crate::tray::open_main_window(app);
    }));
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec![crate::tray::BACKGROUND_ARG]),
    ));

    builder
        .manage(runtime_state)
        .setup(|app| {
            // Plugins set up first, so a second launch has already handed
            // over to the running app. Nothing here touches the network, and
            // the event loop runs no command until setup returns, so every
            // sync sees the repaired proxy variables.
            // A link that started the app waits until the window asks for it.
            if let Some(link) = deep_link::from_args(std::env::args()) {
                deep_link::set_pending(link);
            }
            let report = startup::prepare_process();
            report.log();
            STARTUP_REPORT.set(report);
            #[cfg(desktop)]
            crate::tray::setup(app)?;
            let _scheduler =
                tauri::async_runtime::spawn(application::run_scheduled_sync(app.handle().clone()));
            startup::finish_in_background();
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            {
                if window.label() != "main" {
                    return;
                }
                match event {
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        api.prevent_close();
                        if let Err(error) = window.hide() {
                            eprintln!("Could not hide Agent Plugins in the system tray: {error}");
                        }
                        crate::tray::explain_first_close(tauri::Manager::app_handle(window));
                    }
                    tauri::WindowEvent::Focused(true) => {
                        application::sync_on_focus(tauri::Manager::app_handle(window));
                    }
                    _ => {}
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            ipc::load_cached_manifest_state,
            ipc::run_preflight,
            ipc::sync_manifest_state,
            ipc::prepare_source,
            ipc::confirm_source,
            ipc::cancel_prepared_source,
            ipc::install_item,
            ipc::replace_item,
            ipc::set_manual_invocation,
            ipc::uninstall_item,
            ipc::keep_my_version,
            ipc::set_held,
            ipc::set_excluded_apps,
            ipc::save_connector_settings,
            ipc::plan_bulk_items,
            ipc::run_bulk_items,
            ipc::plan_source_removal,
            ipc::remove_manifest_source,
            ipc::reset_app,
            ipc::run_tutorial,
            ipc::dismiss_tutorial,
            ipc::create_skill,
            ipc::take_pending_link,
            ipc::list_teams,
            ipc::get_team,
            ipc::create_team,
            ipc::rename_team,
            ipc::add_team_member,
            ipc::remove_team_member,
            ipc::team_invite,
            ipc::delete_team,
            ipc::search_directory,
            ipc::preview_link,
            ipc::redeem_link,
            ipc::get_share,
            ipc::set_share,
            ipc::share_link,
            ipc::save_bundle,
            ipc::delete_bundle,
            ipc::plan_items,
            ipc::run_items
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
