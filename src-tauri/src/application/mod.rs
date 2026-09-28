//! Application service for source synchronization and file installation.

mod items;
mod project;
mod sources;
pub(crate) mod status;
mod sync;

use crate::app_state::SourceStatus;
use crate::source::SourceCandidate;
use crate::source::{ConfiguredRepository, ConfiguredSource, RepositorySnapshot, SourceSnapshot};
use std::collections::BTreeMap;
use std::time::SystemTime;
#[cfg(feature = "app")]
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tokio::sync::Mutex;

pub(crate) use items::{
    bulk_plan, bulk_run, install_item, keep_my_version, plan_items, plan_source_removal,
    remove_source, replace_item, reset_app, run_items, save_connector_settings, set_excluded_apps,
    set_held, set_manual_invocation, uninstall_item, Shown,
};
pub(crate) use sources::{cancel_prepared_source, confirm_source, prepare_source};
pub(crate) use sync::{load_cached_app_state, run_preflight, sync_app_state};

#[cfg(feature = "app")]
const SCHEDULED_SYNC_EVENT: &str = "scheduled-sync";
/// How often the scheduler looks at the clock.
#[cfg(feature = "app")]
const SCHEDULER_TICK: std::time::Duration = std::time::Duration::from_secs(30);
/// A wall-clock step this much longer than a tick means the machine slept.
#[cfg(feature = "app")]
const RESUME_JUMP: std::time::Duration = std::time::Duration::from_secs(60);
/// Focusing the window syncs when the last sync is older than this.
const FOCUS_SYNC_AFTER_SECONDS: u64 = 60;
/// Focus comes and goes with every Alt+Tab; act on it at most this often.
#[cfg(feature = "app")]
const FOCUS_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

pub(crate) struct RuntimeState {
    pub(super) operation_lock: Mutex<()>,
    pub(super) sync_lock: Mutex<()>,
    pub(super) pending_sources: Mutex<BTreeMap<String, SourceCandidate>>,
}

impl RuntimeState {
    pub(crate) fn new() -> Self {
        Self {
            operation_lock: Mutex::new(()),
            sync_lock: Mutex::new(()),
            pending_sources: Mutex::new(BTreeMap::new()),
        }
    }
}

pub(super) struct LoadedSource {
    pub(super) definition: ConfiguredSource,
    pub(super) snapshot: Option<SourceSnapshot>,
    pub(super) status: SourceStatus,
    pub(super) refresh_failed: bool,
    pub(super) message: Option<String>,
    pub(super) last_success_at: Option<u64>,
}

pub(super) struct LoadedRepository {
    pub(super) definition: ConfiguredRepository,
    pub(super) snapshot: Option<RepositorySnapshot>,
    pub(super) status: SourceStatus,
    pub(super) refresh_failed: bool,
    pub(super) message: Option<String>,
    pub(super) last_success_at: Option<u64>,
}

/// How `run_blocking` reports a worker that died instead of returning.
pub(crate) const WORKER_FAILED: &str = "worker failed";

pub(super) async fn run_blocking<T, F>(context: &'static str, task: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(task)
        .await
        .map_err(|error| format!("{context} {WORKER_FAILED}: {error}"))?
}

pub(super) use crate::marketplace::epoch_seconds_now as current_epoch_seconds;

/// Calls the marketplace API off the async runtime and returns its JSON
/// answer as it is, or `null` for an answer without a body.
#[cfg(feature = "app")]
pub(crate) async fn marketplace_call(
    method: reqwest::Method,
    path: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    run_blocking("Marketplace request", move || {
        let text = crate::marketplace::api(method, &path, body.as_ref())?;
        if text.trim().is_empty() {
            return Ok(serde_json::Value::Null);
        }
        serde_json::from_str(&text).map_err(|error| {
            format!("The marketplace answered with something Agent Plugins can't read: {error}")
        })
    })
    .await
}

/// Gives the app the tutorial skill and opens it with the tutorial prompt.
/// Returns what the person does next.
#[cfg(feature = "app")]
pub(crate) async fn run_tutorial(app: crate::app_locations::App) -> Result<String, String> {
    run_blocking("Tutorial", move || {
        crate::tutorial::run(&crate::paths::SystemPaths::from_system()?, app)
    })
    .await
}

/// Opens the app with a prompt that writes a skill with the person and
/// publishes it. Returns what the person does next.
#[cfg(feature = "app")]
pub(crate) async fn create_skill(app: crate::app_locations::App) -> Result<String, String> {
    run_blocking("Create a skill", move || {
        crate::tutorial::create_skill(&crate::paths::SystemPaths::from_system()?, app)
    })
    .await
}

/// Stops offering the skill tutorial.
#[cfg(feature = "app")]
pub(crate) async fn dismiss_tutorial() -> Result<(), String> {
    run_blocking("Tutorial", move || {
        crate::tutorial::dismiss(&crate::paths::SystemPaths::from_system()?)
    })
    .await
}

/// Seconds to wait after a sync before the next scheduled one: 15 minutes
/// after a good pass; 1, 2, then 5 minutes after failed passes in a row; then
/// back to 15 minutes.
fn scheduled_sync_delay(failed_passes: u32) -> u64 {
    match failed_passes {
        1 => 60,
        2 => 2 * 60,
        3 => 5 * 60,
        _ => 15 * 60,
    }
}

/// Syncs on the retry ladder above, and right away when the machine resumes
/// from sleep. Every sync counts, whoever started it, so a failed manual check
/// also brings the next scheduled one forward.
#[cfg(feature = "app")]
pub(crate) async fn run_scheduled_sync<R: Runtime>(app: AppHandle<R>) {
    let started = current_epoch_seconds();
    let mut last_tick = SystemTime::now();
    loop {
        tokio::time::sleep(SCHEDULER_TICK).await;
        let now = SystemTime::now();
        let resumed =
            now.duration_since(last_tick).unwrap_or_default() > SCHEDULER_TICK + RESUME_JUMP;
        last_tick = now;
        let (finished, failed) = sync::pass_record();
        let due = finished.max(started) + scheduled_sync_delay(failed) <= current_epoch_seconds();
        if !resumed && !due {
            continue;
        }
        let Some(runtime) = app.try_state::<RuntimeState>() else {
            eprintln!("Scheduled source sync stopped because runtime state is unavailable.");
            return;
        };
        let result = sync_app_state(runtime.inner()).await;
        if let Ok(state) = &result {
            crate::notify::after_sync(&app, state);
        }
        let event = crate::app_state::ScheduledSync::from_result(result);
        if let Err(error) = app.emit(SCHEDULED_SYNC_EVENT, &event) {
            eprintln!("Could not publish scheduled source sync: {error}");
        }
    }
}

/// When the window comes forward: looks again for installed agents, so an app
/// installed or removed since the last look shows up without a restart, then
/// syncs if the set changed or the last sync finished over a minute ago. A
/// changed set also extends installed packages to a newly found agent. Runs
/// at most once a minute, counting from launch, since loading syncs anyway.
/// The result arrives as the scheduled-sync event.
#[cfg(feature = "app")]
pub(crate) fn sync_on_focus<R: Runtime>(app: &AppHandle<R>) {
    use std::sync::{OnceLock, PoisonError};
    use std::time::Instant;
    static LAST: OnceLock<std::sync::Mutex<Instant>> = OnceLock::new();
    {
        let mut last = LAST
            .get_or_init(|| std::sync::Mutex::new(Instant::now()))
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if last.elapsed() < FOCUS_INTERVAL {
            return;
        }
        *last = Instant::now();
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let agents_changed =
            tauri::async_runtime::spawn_blocking(crate::agent_profiles::refresh_detection)
                .await
                .unwrap_or(false);
        let (finished, _) = sync::pass_record();
        if focus_wants_sync(
            agents_changed,
            sync::sync_in_progress(),
            finished,
            current_epoch_seconds(),
        ) {
            spawn_app_sync(app);
        }
    });
}

/// A changed agent set always syncs: a sync already running may have read the
/// old set, so this one queues behind it on the sync lock. Otherwise focus
/// syncs only when none is running and the last one finished long enough ago.
fn focus_wants_sync(agents_changed: bool, in_progress: bool, finished: u64, now: u64) -> bool {
    agents_changed || (!in_progress && now >= finished + FOCUS_SYNC_AFTER_SECONDS)
}

#[cfg(feature = "app")]
pub(crate) fn spawn_app_sync<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let Some(runtime) = app.try_state::<RuntimeState>() else {
            return;
        };
        // Report a failed manual check the same way the scheduler does, so
        // "Check for Updates Now" is never silent.
        let result = sync_app_state(runtime.inner()).await;
        if let Ok(state) = &result {
            crate::notify::after_sync(&app, state);
        }
        let _ = app.emit(
            SCHEDULED_SYNC_EVENT,
            crate::app_state::ScheduledSync::from_result(result),
        );
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn failed_passes_retry_sooner_then_fall_back_to_the_normal_interval() {
        let delays = (0..6).map(super::scheduled_sync_delay).collect::<Vec<_>>();
        assert_eq!(delays, [900, 60, 120, 300, 900, 900]);
    }

    #[test]
    fn focus_syncs_on_changed_agents_or_an_old_sync_but_not_over_a_running_one() {
        use super::focus_wants_sync;
        assert!(focus_wants_sync(true, true, 1_000, 1_001));
        assert!(!focus_wants_sync(false, false, 1_000, 1_059));
        assert!(focus_wants_sync(false, false, 1_000, 1_060));
        assert!(!focus_wants_sync(false, true, 1_000, 5_000));
        assert!(focus_wants_sync(false, false, 0, 1_000));
    }
}

#[cfg(test)]
mod live_nexus_tests {
    use super::*;
    use crate::agent_profiles::TargetId;
    use crate::app_state::AppState;
    use crate::install::ItemStatus;

    #[test]
    #[ignore = "hits the live Nexus catalog; run with AGENT_PLUGINS_QA_ROOT set"]
    fn live_nexus_catalog_round_trip() {
        assert!(
            crate::qa_paths::root().expect("qa root").is_some(),
            "AGENT_PLUGINS_QA_ROOT must name a directory under the process temp dir"
        );
        let tokio_runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        tokio_runtime.block_on(async {
            let runtime = RuntimeState::new();
            match live_step().as_str() {
                "sync" => {
                    print_live_state(&sync_app_state(&runtime).await.expect("sync"));
                }
                "add" => {
                    print_live_state(&add_listed_skillbook(&runtime).await.expect("add"));
                }
                "refresh" => {
                    print_live_state(&sync_app_state(&runtime).await.expect("refresh"));
                }
                "install" => {
                    print_live_state(&install_git_ops(&runtime).await.expect("install"));
                }
                "remove" => {
                    let result = remove_source(&runtime, "skillbook", false)
                        .await
                        .expect("remove");
                    assert!(
                        result.failures.is_empty(),
                        "remove failed: {:?}",
                        result.failures
                    );
                    print_live_state(
                        &load_cached_app_state(&runtime)
                            .await
                            .expect("load")
                            .expect("state"),
                    );
                }
                _ => {
                    let added = add_listed_skillbook(&runtime).await.expect("add");
                    assert!(
                        added.catalog_message.is_none(),
                        "{:?}",
                        added.catalog_message
                    );
                    assert_eq!(added.repositories.len(), 1);
                    assert_eq!(added.repositories[0].name, "Ragsdale sources");
                    assert_eq!(
                        added.repositories[0].description,
                        "Official portable sources published from repo.ragsdale.dev."
                    );
                    assert_eq!(added.repositories[0].sources[0].name, "Skillbook");
                    assert_eq!(added.sources.len(), 1);
                    assert_eq!(added.items.len(), 27);
                    assert!(added
                        .items
                        .iter()
                        .all(|item| item.status == ItemStatus::Available));
                    let commit = added.sources[0].commit.clone();
                    let refreshed = sync_app_state(&runtime).await.expect("refresh");
                    assert_eq!(refreshed.sources[0].commit, commit);
                    assert!(!refreshed.sources[0].refresh_failed);
                    let removed = remove_source(&runtime, "skillbook", false)
                        .await
                        .expect("remove");
                    assert!(removed.failures.is_empty(), "{:?}", removed.failures);
                    let after = load_cached_app_state(&runtime)
                        .await
                        .expect("load")
                        .expect("state");
                    assert!(after.sources.is_empty());
                    assert_eq!(after.repositories.len(), 1);
                    assert!(!after.repositories[0].sources[0].already_added);
                    print_live_state(&after);
                }
            }
        });
    }

    fn live_step() -> String {
        std::env::var("AGENT_PLUGINS_LIVE_STEP").unwrap_or_else(|_| "all".to_string())
    }

    async fn add_listed_skillbook(runtime: &RuntimeState) -> Result<AppState, String> {
        let state = sync_app_state(runtime).await?;
        if state
            .sources
            .iter()
            .any(|source| source.source_id == "skillbook")
        {
            return Ok(state);
        }
        let repository = state
            .repositories
            .first()
            .ok_or_else(|| "Live catalog was not added.".to_string())?;
        let listed = repository
            .sources
            .first()
            .ok_or_else(|| "Live catalog listed no sources.".to_string())?;
        let prepared =
            prepare_source(runtime, &listed.url, repository.repository_key.clone()).await?;
        confirm_source(runtime, &prepared.token).await
    }

    async fn install_git_ops(runtime: &RuntimeState) -> Result<AppState, String> {
        let state = add_listed_skillbook(runtime).await?;
        if !state.agent_profiles.iter().any(|profile| profile.enabled) {
            crate::agent_profiles::set_enabled(
                &crate::paths::SystemPaths::from_system()?,
                TargetId::GrokBuild,
                true,
            )?;
        }
        install_item(runtime, "skillbook", "git-ops", true, None, None).await?;
        load_cached_app_state(runtime)
            .await?
            .ok_or_else(|| "App state missing after install.".to_string())
    }

    fn print_live_state(state: &AppState) {
        let repository = state.repositories.first();
        println!(
            "LIVE catalog_message={} repo_name={} repo_refresh_failed={} listed={} already_added={}",
            state.catalog_message.as_deref().unwrap_or("-"),
            repository.map_or("-", |repository| repository.name.as_str()),
            repository.is_some_and(|repository| repository.refresh_failed),
            repository.map_or(0, |repository| repository.sources.len()),
            repository
                .and_then(|repository| repository.sources.first())
                .is_some_and(|source| source.already_added)
        );
        if let Some(source) = state.sources.first() {
            let installed = state
                .items
                .iter()
                .filter(|item| item.status == ItemStatus::Installed)
                .count();
            println!(
                "LIVE source={} commit={} items={} installed={} refresh_failed={}",
                source.source_id,
                source.commit.as_deref().unwrap_or("-"),
                state.items.len(),
                installed,
                source.refresh_failed
            );
        } else {
            println!("LIVE source=-");
        }
    }
}
