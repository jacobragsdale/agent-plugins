#![cfg_attr(not(feature = "app"), allow(dead_code, unused_imports))]

mod adapters;
mod agent_profiles;
mod app_state;
mod application;
mod artifact;
mod catalog;
#[cfg(feature = "app")]
mod cli;
mod digest;
mod executor;
mod fs_retry;
mod host_identity;
mod install;
#[cfg(feature = "app")]
mod ipc;
mod ledger;
mod locator;
mod managed_documents;
pub mod manifest;
mod marketplace;
mod mcp;
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

/// The host preparation report from process start, for the preflight.
pub(crate) static STARTUP_REPORT: std::sync::OnceLock<startup::StartupReport> =
    std::sync::OnceLock::new();

/// Runs host preparation once and remembers the report.
pub(crate) fn prepare_host() -> &'static startup::StartupReport {
    STARTUP_REPORT.get_or_init(|| {
        let report = startup::prepare();
        report.log();
        report
    })
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
    prepare_host();
    let runtime_state =
        application::RuntimeState::new().expect("could not initialize the Agent Plugins runtime");
    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        match crate::tray::show_main_window(app) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("Could not open Agent Plugins because its main window is unavailable.");
            }
            Err(error) => eprintln!("Could not open Agent Plugins: {error}"),
        }
    }));
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_autostart::init(
        tauri_plugin_autostart::MacosLauncher::LaunchAgent,
        Some(vec![crate::tray::BACKGROUND_ARG]),
    ));

    builder
        .manage(runtime_state)
        .setup(|app| {
            #[cfg(desktop)]
            crate::tray::setup(app)?;
            let _scheduler =
                tauri::async_runtime::spawn(application::run_scheduled_sync(app.handle().clone()));
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            {
                if window.label() != "main" {
                    return;
                }
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    if let Err(error) = window.hide() {
                        eprintln!("Could not hide Agent Plugins in the system tray: {error}");
                    }
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
            ipc::uninstall_item,
            ipc::plan_bulk_items,
            ipc::run_bulk_items,
            ipc::plan_source_removal,
            ipc::remove_manifest_source,
            ipc::reset_app
        ])
        .run(tauri::generate_context!())
        .expect("error while running Tauri application");
}
