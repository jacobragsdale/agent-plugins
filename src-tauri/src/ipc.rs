//! Thin Tauri command surface for the desktop UI. Every command rejects with
//! an [`IpcError`], classified here from the internal message.

use crate::app_locations::App;
use crate::app_state::{AppState, BulkAction, BulkPlan, BulkResult, ItemsPlan, PreparedSource};
use crate::application::{self, RuntimeState};
use crate::install::{OperationOutcome, SourceRemovalPlan};
use crate::ipc_error::IpcError;
use reqwest::Method;
use serde_json::{json, Value};
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
    shown: Option<application::Shown>,
    component_id: Option<String>,
) -> Result<OperationOutcome, IpcError> {
    application::install_item(
        runtime.inner(),
        source_id,
        local_id,
        trust_approved,
        shown.as_ref(),
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
    shown: Option<application::Shown>,
    component_id: Option<String>,
) -> Result<OperationOutcome, IpcError> {
    application::replace_item(
        runtime.inner(),
        source_id,
        local_id,
        trust_approved,
        shown.as_ref(),
        component_id.as_deref(),
    )
    .await
    .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn set_manual_invocation(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    local_id: &str,
    component_id: Option<String>,
    manual: bool,
) -> Result<OperationOutcome, IpcError> {
    application::set_manual_invocation(
        runtime.inner(),
        source_id,
        local_id,
        component_id.as_deref(),
        manual,
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
    force: Option<bool>,
) -> Result<OperationOutcome, IpcError> {
    application::uninstall_item(
        runtime.inner(),
        source_id,
        local_id,
        component_id.as_deref(),
        force.unwrap_or(false),
    )
    .await
    .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn keep_my_version(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    local_id: &str,
) -> Result<(), IpcError> {
    application::keep_my_version(runtime.inner(), source_id, local_id)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn set_held(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    local_id: &str,
    held: bool,
) -> Result<(), IpcError> {
    application::set_held(runtime.inner(), source_id, local_id, held)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn set_excluded_apps(
    runtime: State<'_, RuntimeState>,
    source_id: &str,
    local_id: &str,
    component_id: &str,
    excluded: Vec<String>,
    trust_approved: bool,
    shown: Option<application::Shown>,
) -> Result<OperationOutcome, IpcError> {
    application::set_excluded_apps(
        runtime.inner(),
        source_id,
        local_id,
        component_id,
        excluded,
        trust_approved,
        shown.as_ref(),
    )
    .await
    .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn save_connector_settings(
    values: std::collections::BTreeMap<String, String>,
) -> Result<(), IpcError> {
    application::save_connector_settings(values)
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
    shown: Option<application::Shown>,
) -> Result<BulkResult, IpcError> {
    application::bulk_run(
        runtime.inner(),
        source_id,
        action,
        trust_approved,
        shown.as_ref(),
    )
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

#[tauri::command]
pub(crate) async fn run_tutorial(app: App) -> Result<String, IpcError> {
    application::run_tutorial(app).await.map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn create_skill(app: App) -> Result<String, IpcError> {
    application::create_skill(app).await.map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn dismiss_tutorial() -> Result<(), IpcError> {
    application::dismiss_tutorial()
        .await
        .map_err(IpcError::from)
}

/// The link that opened the app, once; the window asks on start and whenever
/// the `deep-link` event says another arrived.
#[tauri::command]
pub(crate) fn take_pending_link() -> Option<crate::deep_link::DeepLink> {
    crate::deep_link::take_pending()
}

async fn call(method: Method, path: String, body: Option<Value>) -> Result<Value, IpcError> {
    application::marketplace_call(method, path, body)
        .await
        .map_err(IpcError::from)
}

fn space(namespace: &str) -> Result<&str, IpcError> {
    crate::marketplace::namespace_path(namespace).map_err(IpcError::from)
}

fn target(target: &str) -> Result<&str, IpcError> {
    crate::marketplace::target_path(target).map_err(IpcError::from)
}

fn link_code(link: &str) -> Result<String, IpcError> {
    crate::marketplace::link_code(link).map_err(IpcError::from)
}

/// One string field of an answer, such as a team's invite link.
fn text_field(value: &Value, field: &str) -> Result<String, IpcError> {
    value[field]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| IpcError::from(format!("The marketplace sent no {field}.")))
}

#[tauri::command]
pub(crate) async fn list_teams() -> Result<Value, IpcError> {
    call(Method::GET, "teams".to_string(), None).await
}

#[tauri::command]
pub(crate) async fn get_team(namespace: String) -> Result<Value, IpcError> {
    call(Method::GET, format!("teams/{}", space(&namespace)?), None).await
}

#[tauri::command]
pub(crate) async fn create_team(
    namespace: String,
    display_name: String,
    visibility: String,
) -> Result<Value, IpcError> {
    let body =
        json!({ "namespace": namespace, "displayName": display_name, "visibility": visibility });
    call(Method::POST, "teams".to_string(), Some(body)).await
}

#[tauri::command]
pub(crate) async fn rename_team(
    namespace: String,
    display_name: String,
) -> Result<Value, IpcError> {
    let path = format!("teams/{}", space(&namespace)?);
    call(
        Method::PUT,
        path,
        Some(json!({ "displayName": display_name })),
    )
    .await
}

#[tauri::command]
pub(crate) async fn add_team_member(
    namespace: String,
    account: String,
    owner: bool,
) -> Result<Value, IpcError> {
    let path = format!("teams/{}/members", space(&namespace)?);
    call(
        Method::POST,
        path,
        Some(json!({ "account": account, "owner": owner })),
    )
    .await
}

#[tauri::command]
pub(crate) async fn remove_team_member(namespace: String, account: String) -> Result<(), IpcError> {
    let path = format!(
        "teams/{}/members?account={}",
        space(&namespace)?,
        crate::marketplace::query(&account)
    );
    call(Method::DELETE, path, None).await.map(drop)
}

#[tauri::command]
pub(crate) async fn team_invite(namespace: String, reset: bool) -> Result<String, IpcError> {
    let path = format!("teams/{}/invite", space(&namespace)?);
    let answer = call(Method::POST, path, Some(json!({ "reset": reset }))).await?;
    text_field(&answer, "invite")
}

#[tauri::command]
pub(crate) async fn delete_team(namespace: String) -> Result<(), IpcError> {
    let path = format!("teams/{}", space(&namespace)?);
    call(Method::DELETE, path, None).await.map(drop)
}

#[tauri::command]
pub(crate) async fn search_directory(query: String) -> Result<Value, IpcError> {
    let path = format!("directory?q={}", crate::marketplace::query(&query));
    call(Method::GET, path, None).await
}

#[tauri::command]
pub(crate) async fn preview_link(link: String) -> Result<Value, IpcError> {
    call(Method::GET, format!("links/{}", link_code(&link)?), None).await
}

#[tauri::command]
pub(crate) async fn redeem_link(link: String) -> Result<Value, IpcError> {
    call(Method::POST, format!("links/{}", link_code(&link)?), None).await
}

#[tauri::command]
pub(crate) async fn get_share(target: String) -> Result<Value, IpcError> {
    let path = format!("access/{}", self::target(&target)?);
    call(Method::GET, path, None).await
}

#[tauri::command]
pub(crate) async fn set_share(
    target: String,
    visibility: String,
    users: Vec<String>,
    teams: Vec<String>,
    groups: Vec<String>,
) -> Result<Value, IpcError> {
    let path = format!("access/{}", self::target(&target)?);
    let body =
        json!({ "visibility": visibility, "users": users, "teams": teams, "groups": groups });
    call(Method::PUT, path, Some(body)).await
}

#[tauri::command]
pub(crate) async fn share_link(target: String, reset: bool) -> Result<String, IpcError> {
    let path = format!("access/{}/link", self::target(&target)?);
    let answer = call(Method::POST, path, Some(json!({ "reset": reset }))).await?;
    text_field(&answer, "link")
}

#[tauri::command]
pub(crate) async fn save_bundle(
    namespace: String,
    bundle_id: String,
    name: String,
    description: String,
    members: Vec<String>,
) -> Result<Value, IpcError> {
    let path = format!("bundles/{}", target(&format!("{namespace}/{bundle_id}"))?);
    let body = json!({ "name": name, "description": description, "members": members });
    call(Method::PUT, path, Some(body)).await
}

#[tauri::command]
pub(crate) async fn delete_bundle(namespace: String, bundle_id: String) -> Result<(), IpcError> {
    let path = format!("bundles/{}", target(&format!("{namespace}/{bundle_id}"))?);
    call(Method::DELETE, path, None).await.map(drop)
}

#[tauri::command]
pub(crate) async fn plan_items(
    runtime: State<'_, RuntimeState>,
    ids: Vec<String>,
    action: BulkAction,
) -> Result<ItemsPlan, IpcError> {
    application::plan_items(runtime.inner(), &ids, action)
        .await
        .map_err(IpcError::from)
}

#[tauri::command]
pub(crate) async fn run_items(
    runtime: State<'_, RuntimeState>,
    ids: Vec<String>,
    action: BulkAction,
    trust_approved: bool,
    shown: Option<application::Shown>,
) -> Result<BulkResult, IpcError> {
    application::run_items(
        runtime.inner(),
        &ids,
        action,
        trust_approved,
        shown.as_ref(),
    )
    .await
    .map_err(IpcError::from)
}
