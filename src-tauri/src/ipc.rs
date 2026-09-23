//! Thin Tauri command surface for the desktop UI. Every command rejects with
//! an [`IpcError`], classified here from the internal message.

use crate::app_state::{AppState, BulkAction, BulkPlan, BulkResult, PreparedSource};
use crate::application::{self, RuntimeState};
use crate::install::{OperationOutcome, SourceRemovalPlan};
use crate::ipc_error::IpcError;
use tauri::State;

#[tauri::command]
pub(crate) async fn load_cached_manifest_state(
    runtime: State<'_, RuntimeState>,
) -> Result<Option<AppState>, IpcError> {
    application::load_cached_app_state(runtime.inner())
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn run_preflight(
    runtime: State<'_, RuntimeState>,
) -> Result<crate::preflight::PreflightReport, IpcError> {
    application::run_preflight(runtime.inner())
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn sync_manifest_state(
    runtime: State<'_, RuntimeState>,
) -> Result<AppState, IpcError> {
    application::sync_app_state(runtime.inner())
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn prepare_source(
    runtime: State<'_, RuntimeState>,
    url: &str,
    repository_key: &str,
) -> Result<PreparedSource, IpcError> {
    application::prepare_source(runtime.inner(), url, repository_key.to_string())
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn confirm_source(
    runtime: State<'_, RuntimeState>,
    token: &str,
) -> Result<AppState, IpcError> {
    application::confirm_source(runtime.inner(), token)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn cancel_prepared_source(
    runtime: State<'_, RuntimeState>,
    token: &str,
) -> Result<(), IpcError> {
    application::cancel_prepared_source(runtime.inner(), token)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn install_item(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    local_id: &str,
    trust_approved: bool,
    component_id: Option<String>,
) -> Result<OperationOutcome, IpcError> {
    application::install_item(
        runtime.inner(),
        source_id,
        local_id,
        trust_approved,
        component_id.as_deref(),
    )
    .await
    .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn replace_item(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    local_id: &str,
    trust_approved: bool,
    component_id: Option<String>,
) -> Result<OperationOutcome, IpcError> {
    application::replace_item(
        runtime.inner(),
        source_id,
        local_id,
        trust_approved,
        component_id.as_deref(),
    )
    .await
    .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn uninstall_item(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    local_id: &str,
    component_id: Option<String>,
) -> Result<OperationOutcome, IpcError> {
    application::uninstall_item(
        runtime.inner(),
        source_id,
        local_id,
        component_id.as_deref(),
    )
    .await
    .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn plan_bulk_items(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    action: BulkAction,
) -> Result<BulkPlan, IpcError> {
    application::bulk_plan(runtime.inner(), source_id, action)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn run_bulk_items(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    action: BulkAction,
    trust_approved: bool,
) -> Result<BulkResult, IpcError> {
    application::bulk_run(runtime.inner(), source_id, action, trust_approved)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn plan_source_removal(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
) -> Result<SourceRemovalPlan, IpcError> {
    application::plan_source_removal(runtime.inner(), source_id)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn remove_manifest_source(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    acknowledge_modified_paths: bool,
) -> Result<BulkResult, IpcError> {
    application::remove_source(runtime.inner(), source_id, acknowledge_modified_paths)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn reset_app(runtime: State<'_, RuntimeState>) -> Result<BulkResult, IpcError> {
    application::reset_app(runtime.inner())
        .await
        .map_err(IpcError::from)
}
