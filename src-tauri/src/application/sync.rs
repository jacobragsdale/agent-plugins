use super::items::{extend_installs_to_new_agents, repair_missing_installs};
use super::{current_epoch_seconds, run_blocking, LoadedRepository, LoadedSource, RuntimeState};
use crate::agent_profiles;
use crate::app_state::{
    AppState, AutoUpdateReport, Connectivity, ItemFailure, ItemReference, SourceStatus,
};
use crate::artifact;
use crate::install::{self, ItemStatus};
use crate::ledger::InstallationRecord;
use crate::locator::{default_catalog_locator, Locator};
use crate::paths::SystemPaths;
use crate::repository::ListedSource;
use crate::source::{
    self, ConfiguredRepository, ConfiguredSource, RepositoryCandidate, SourceCandidate,
    SourcesConfig,
};
use crate::sources::{cache_base_dir, config_base_dir};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

pub(crate) async fn load_cached_app_state(
    runtime: &RuntimeState,
) -> Result<Option<AppState>, String> {
    let _guard = runtime.operation_lock.lock().await;
    run_blocking("Cached source load", || {
        let paths = SystemPaths::from_system()?;
        let cache = cache_base_dir()?;
        let config = config_base_dir()?;
        cached_app_state(&paths, &cache, &config).map(Some)
    })
    .await
}

/// The state the window reloads after every operation. It carries what the
/// last sync learned - the identity, the preflight, the marketplace index, and
/// each source's refresh result - because dropping those would blank the
/// header and the badges and hide an outage.
fn cached_app_state(paths: &SystemPaths, cache: &Path, config: &Path) -> Result<AppState, String> {
    retire_unsupported_legacy_installs(paths);
    agent_profiles::apply_detected_defaults(paths)?;
    super::project::cached_state(paths, cache, config)
}

/// Re-runs the preflight on demand against the cached state. The ledger is
/// read under `operation_lock`, so its journal recovery can never run in the
/// middle of an install; the server checks run after the lock is released.
pub(crate) async fn run_preflight(
    runtime: &RuntimeState,
) -> Result<crate::preflight::PreflightReport, String> {
    let _sync_guard = runtime.sync_lock.lock().await;
    let (paths, state, ledger_error) = {
        let _operation_guard = runtime.operation_lock.lock().await;
        run_blocking("Preflight", || {
            agent_profiles::clear_detection_cache();
            let paths = SystemPaths::from_system()?;
            let state = super::project::cached_state_now()?;
            let ledger_error = crate::executor::read_ledger(&paths).err();
            Ok((paths, state, ledger_error))
        })
        .await?
    };
    run_blocking("Preflight", move || {
        let cache = cache_base_dir()?;
        let (report, findings) = crate::preflight::run(&crate::preflight::PreflightInput {
            paths: &paths,
            startup: crate::STARTUP_REPORT.get(),
            profiles: &state.agent_profiles,
            items: &state.items,
            catalog_age_seconds: catalog_age(&read_health(&cache)),
            ledger_error,
        });
        report.write_cache(&cache);
        super::project::remember_identity(&cache, &report, findings.identity);
        Ok(report)
    })
    .await
}

static SYNC_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static LAST_PASS_FINISHED: AtomicU64 = AtomicU64::new(0);
static FAILED_PASSES: AtomicU32 = AtomicU32::new(0);

/// True while a sync runs, so a cached state can say a fresher one is coming.
pub(super) fn sync_in_progress() -> bool {
    SYNC_IN_PROGRESS.load(Ordering::SeqCst)
}

/// When the last sync finished (epoch seconds, 0 for never) and how many
/// passes in a row have failed, for the scheduler's retry ladder.
pub(super) fn pass_record() -> (u64, u32) {
    (
        LAST_PASS_FINISHED.load(Ordering::SeqCst),
        FAILED_PASSES.load(Ordering::SeqCst),
    )
}

struct InProgress;

impl Drop for InProgress {
    fn drop(&mut self) {
        SYNC_IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

pub(crate) async fn sync_app_state(runtime: &RuntimeState) -> Result<AppState, String> {
    let _sync_guard = runtime.sync_lock.lock().await;
    SYNC_IN_PROGRESS.store(true, Ordering::SeqCst);
    let _in_progress = InProgress;
    let result = synchronize(runtime).await;
    if result
        .as_ref()
        .is_ok_and(|state| state.connectivity == Connectivity::Online)
    {
        FAILED_PASSES.store(0, Ordering::SeqCst);
    } else {
        FAILED_PASSES.fetch_add(1, Ordering::SeqCst);
    }
    LAST_PASS_FINISHED.store(current_epoch_seconds(), Ordering::SeqCst);
    result
}

/// Network I/O runs under `sync_lock` alone, so installs and removals stay
/// responsive while a server is slow. `operation_lock` is held only to read the
/// configuration, then to activate what was fetched and reconcile the installed
/// packages - the steps that must agree with the ledger.
async fn synchronize(runtime: &RuntimeState) -> Result<AppState, String> {
    let config = {
        let _operation_guard = runtime.operation_lock.lock().await;
        run_blocking("Source configuration", || {
            source::read_sources_config(&config_base_dir()?)
        })
        .await?
    };
    let fetched = run_blocking("Source download", move || {
        // A sync is the app looking at the machine again, agents included.
        agent_profiles::clear_detection_cache();
        Ok(fetch_all(&cache_base_dir()?, config))
    })
    .await?;
    let (state, marketplace) = {
        let _operation_guard = runtime.operation_lock.lock().await;
        run_blocking("Source activation", move || apply_fetched(fetched)).await?
    };
    run_blocking("Marketplace check", move || {
        Ok(enrich_with_marketplace(state, marketplace))
    })
    .await
}

/// What one fetch of a catalog or source produced, before anything is activated.
enum Fetch<C> {
    Ready(C),
    /// The artifact no longer exists at its URL (HTTP 404 or 410).
    Gone,
    /// The server was not reached, timed out, or was overloaded; try again later.
    Unreachable(String),
    Failed(String),
}

impl<C> Fetch<C> {
    fn from_result(result: Result<C, String>) -> Self {
        match result {
            Ok(candidate) => Self::Ready(candidate),
            Err(message) if artifact::is_gone(&message) => Self::Gone,
            Err(message) if artifact::is_transient(&message) => Self::Unreachable(message),
            Err(message) => Self::Failed(message),
        }
    }
}

/// Hosts that could not be connected to during this pass. Later fetches from
/// the same host are skipped, and marked stale, instead of each waiting out
/// its own timeouts.
#[derive(Default)]
struct DeadHosts(std::sync::Mutex<BTreeSet<String>>);

impl DeadHosts {
    fn host(url: &str) -> String {
        url::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_default()
    }

    fn contains(&self, url: &str) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(&Self::host(url))
    }

    fn fetch<C>(&self, url: &str, fetch: impl FnOnce() -> Result<C, String>) -> Fetch<C> {
        let host = Self::host(url);
        if self.contains(url) {
            return Fetch::Unreachable(format!(
                "Skipped this check because {host} could not be reached. The app tries again soon."
            ));
        }
        let result = fetch();
        if let Err(message) = &result {
            if artifact::is_connect_failure(message) {
                self.0
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(host);
            }
        }
        Fetch::from_result(result)
    }
}

/// Everything the network part of a sync produced.
struct Fetched {
    cache: PathBuf,
    catalog_message: Option<String>,
    /// The default catalog when it is not configured yet.
    default_catalog: Option<Fetch<RepositoryCandidate>>,
    repositories: Vec<(ConfiguredRepository, Fetch<RepositoryCandidate>)>,
    /// Sources the marketplace catalog lists that this machine has not added.
    subscribed: Vec<(String, Fetch<SourceCandidate>)>,
    sources: Vec<(ConfiguredSource, Fetch<SourceCandidate>)>,
    dead_hosts: DeadHosts,
}

impl Fetched {
    /// Online when every fetch reached its server, offline when none did.
    fn connectivity(&self) -> Connectivity {
        fn unreachable<C>(fetch: &Fetch<C>) -> bool {
            matches!(fetch, Fetch::Unreachable(_))
        }
        let outcomes = self
            .default_catalog
            .iter()
            .map(unreachable)
            .chain(
                self.repositories
                    .iter()
                    .map(|(_, fetch)| unreachable(fetch)),
            )
            .chain(self.subscribed.iter().map(|(_, fetch)| unreachable(fetch)))
            .chain(self.sources.iter().map(|(_, fetch)| unreachable(fetch)))
            .collect::<Vec<_>>();
        let unreachable = outcomes.iter().filter(|unreachable| **unreachable).count();
        if unreachable == 0 {
            Connectivity::Online
        } else if unreachable == outcomes.len() {
            Connectivity::Offline
        } else {
            Connectivity::Degraded
        }
    }
}

/// Fetches every catalog and source, a few at a time. Nothing is activated
/// here, so this runs without `operation_lock`.
fn fetch_all(cache: &Path, config: SourcesConfig) -> Fetched {
    let dead_hosts = DeadHosts::default();
    let (default, catalog_message) = match default_catalog_locator() {
        Ok(locator) => (locator, None),
        Err(message) => (None, Some(message)),
    };
    let default_catalog = default
        .as_ref()
        .filter(|locator| {
            !config
                .repositories
                .iter()
                .any(|repository| repository.locator.same_identity(locator))
        })
        .map(|locator| {
            dead_hosts.fetch(locator.url(), || {
                source::prepare_new_repository(locator, cache)
            })
        });
    let repositories = crate::parallel::map(&config.repositories, |definition| {
        dead_hosts.fetch(definition.url(), || prepare_repository(cache, definition))
    });
    let repositories = config
        .repositories
        .into_iter()
        .zip(repositories)
        .collect::<Vec<_>>();
    let listings = default
        .as_ref()
        .and_then(|default| catalog_listing(cache, default, &default_catalog, &repositories))
        .map(|(repository_key, listed)| {
            listed
                .into_iter()
                .filter_map(|entry| {
                    let locator = entry.locator().ok()?;
                    let configured = config.sources.iter().any(|source| {
                        source.locator.same_identity(&locator)
                            || entry.source_id.as_deref() == Some(source.source_id.as_str())
                    });
                    (!configured).then(|| (entry, locator, repository_key.clone()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let subscribed = crate::parallel::map(&listings, |(entry, locator, repository_key)| {
        let fetch = dead_hosts.fetch(locator.url(), || {
            source::prepare_new_source(
                locator,
                cache,
                Some(repository_key.clone()),
                entry.source_id.as_deref(),
            )
        });
        (entry.name.clone(), fetch)
    });
    let sources = crate::parallel::map(&config.sources, |definition| {
        dead_hosts.fetch(definition.url(), || prepare_source(cache, definition))
    });
    Fetched {
        cache: cache.to_path_buf(),
        catalog_message,
        default_catalog,
        repositories,
        subscribed,
        sources: config.sources.into_iter().zip(sources).collect(),
        dead_hosts,
    }
}

/// The sources the default catalog lists, from this pass's fetch when it
/// produced a usable catalog, otherwise from the saved copy.
fn catalog_listing(
    cache: &Path,
    default: &Locator,
    new_default: &Option<Fetch<RepositoryCandidate>>,
    repositories: &[(ConfiguredRepository, Fetch<RepositoryCandidate>)],
) -> Option<(String, Vec<ListedSource>)> {
    if let Some(Fetch::Ready(candidate)) = new_default {
        return Some((
            candidate.definition.repository_key.clone(),
            candidate.manifest.canonical_sources().ok()?,
        ));
    }
    let (definition, fetch) = repositories
        .iter()
        .find(|(definition, _)| definition.locator.same_identity(default))?;
    let listed = match fetch {
        Fetch::Ready(candidate)
            if candidate.definition.repository_id == definition.repository_id =>
        {
            candidate.manifest.canonical_sources().ok()?
        }
        _ => source::load_current_repository(cache, definition)
            .ok()??
            .manifest
            .canonical_sources()
            .ok()?,
    };
    Some((definition.repository_key.clone(), listed))
}

/// Prepares a catalog refresh; an unreadable cache is wiped and fetched again.
fn prepare_repository(
    cache: &Path,
    definition: &ConfiguredRepository,
) -> Result<RepositoryCandidate, String> {
    source::prepare_repository_refresh(definition, cache).or_else(|message| {
        if !source::is_corrupt_cache_error(&message) {
            return Err(message);
        }
        eprintln!(
            "Wiping the unreadable cache of the catalog {} and refreshing it again: {message}",
            definition.name
        );
        source::remove_repository_cache(cache, &definition.repository_key)?;
        source::prepare_repository_refresh(definition, cache)
    })
}

/// Prepares a source refresh; an unreadable cache is wiped and fetched again.
fn prepare_source(cache: &Path, definition: &ConfiguredSource) -> Result<SourceCandidate, String> {
    source::prepare_refresh(definition, cache).or_else(|message| {
        if !source::is_corrupt_cache_error(&message) {
            return Err(message);
        }
        eprintln!(
            "Wiping the unreadable cache of the source {} and refreshing it again: {message}",
            definition.name
        );
        source::remove_source_cache(cache, &definition.source_key)?;
        source::prepare_refresh(definition, cache)
    })
}

/// What the marketplace step after activation needs; it runs without
/// `operation_lock`, so everything that reads the ledger is gathered first.
struct MarketplaceCheck {
    paths: SystemPaths,
    cache: PathBuf,
    ledger_error: Option<String>,
    agents: Vec<String>,
    /// Updated package ids with the marketplace version they had before.
    updates: Vec<(String, Option<String>)>,
    catalog_age_seconds: Option<u64>,
    marketplace_unreachable: bool,
}

/// Activates what was fetched and reconciles installed packages. Runs under
/// `operation_lock`. The configuration is read again, so a source added or
/// removed while the network part ran is kept or stays removed.
fn apply_fetched(fetched: Fetched) -> Result<(AppState, MarketplaceCheck), String> {
    let paths = SystemPaths::from_system()?;
    let config_dir = config_base_dir()?;
    let cache = fetched.cache.clone();
    let now = current_epoch_seconds();
    let connectivity = fetched.connectivity();
    let marketplace_unreachable =
        crate::locator::marketplace_base_url().is_some_and(|url| fetched.dead_hosts.contains(url));
    let mut health = read_health(&cache);
    let mut config = source::read_sources_config(&config_dir)?;
    let mut messages = fetched.catalog_message.into_iter().collect::<Vec<_>>();
    let mut fetched_repositories = fetched.repositories;
    match fetched.default_catalog {
        Some(Fetch::Ready(candidate))
            if !config.repositories.iter().any(|repository| {
                repository
                    .locator
                    .same_identity(&candidate.definition.locator)
            }) =>
        {
            config.repositories.push(candidate.definition.clone());
            fetched_repositories.push((candidate.definition.clone(), Fetch::Ready(candidate)));
        }
        Some(Fetch::Ready(candidate)) => source::discard_repository(&candidate),
        Some(Fetch::Failed(message) | Fetch::Unreachable(message)) => messages.push(message),
        Some(Fetch::Gone) => {
            messages.push("The marketplace catalog was not found on the server.".to_string())
        }
        None => {}
    }
    let mut fetched_sources = fetched.sources;
    let mut problems = Vec::new();
    for (name, fetch) in fetched.subscribed {
        match fetch {
            Fetch::Ready(candidate)
                if !config.sources.iter().any(|source| {
                    source.locator.same_identity(&candidate.definition.locator)
                        || source.source_id == candidate.definition.source_id
                }) =>
            {
                config.sources.push(candidate.definition.clone());
                fetched_sources.push((candidate.definition.clone(), Fetch::Ready(candidate)));
            }
            Fetch::Ready(candidate) => source::discard_candidate(&candidate),
            Fetch::Failed(message) => problems.push(format!("{name}: {message}")),
            Fetch::Gone => problems.push(format!("{name}: it was not found on the server.")),
            // Unreachable listings are added by a later sync.
            Fetch::Unreachable(_) => {}
        }
    }
    if !problems.is_empty() {
        messages.push(format!(
            "Some marketplace sources could not be added: {}",
            problems.join(" ")
        ));
    }
    let installed_keys = crate::executor::read_ledger(&paths)
        .map(|ledger| {
            ledger
                .items
                .values()
                .map(|record| record.source_key.clone())
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    let (repositories, loaded_repositories, retired_repositories) = apply_repositories(
        &cache,
        &mut health,
        now,
        config.repositories,
        fetched_repositories,
    );
    let (mut sources, loaded_sources, retired_sources) = apply_sources(
        &cache,
        &mut health,
        now,
        config.sources,
        fetched_sources,
        &installed_keys,
    );
    messages.extend(retired_message(&retired_repositories, &retired_sources));
    sources.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    let configured_keys = repositories
        .iter()
        .map(|repository| repository.repository_key.clone())
        .chain(sources.iter().map(|source| source.source_key.clone()))
        .collect::<BTreeSet<_>>();
    source::write_sources_config(
        &config_dir,
        &SourcesConfig {
            repositories,
            sources,
        },
    )?;
    health
        .entries
        .retain(|key, _| configured_keys.contains(key));
    health.connectivity = connectivity;
    write_health(&cache, &health);

    retire_unsupported_legacy_installs(&paths);
    agent_profiles::apply_detected_defaults(&paths)?;
    let previous_index = crate::marketplace::read_cached_index(&cache);
    let mut report = reconcile_installed_items(&paths, &loaded_sources)?;
    match repair_missing_installs() {
        Ok(names) => report.repaired_items = names,
        Err(error) => eprintln!("Could not check installed packages for missing files: {error}"),
    }
    match extend_installs_to_new_agents() {
        Ok(names) => report.extended_items = names,
        Err(error) => eprintln!("Could not add installed packages to newly found agents: {error}"),
    }
    let updates = report
        .updated_items
        .iter()
        .map(|item| {
            let from = previous_index
                .as_ref()
                .and_then(|index| index.package(&item.id))
                .map(|package| package.version.clone());
            (item.id.clone(), from)
        })
        .collect();
    // "Last checked" is the last pass that reached the servers.
    let checked = if connectivity == Connectivity::Offline {
        read_last_sync(&cache).unwrap_or(0)
    } else {
        write_last_sync(&cache, now);
        now
    };
    let mut state = super::project::build_app_state(
        &paths,
        &loaded_repositories,
        &loaded_sources,
        checked,
        report,
        (!messages.is_empty()).then(|| messages.join(" ")),
    )?;
    state.connectivity = connectivity;
    let check = MarketplaceCheck {
        ledger_error: crate::executor::read_ledger(&paths).err(),
        agents: super::items::enabled_agent_ids(&paths),
        paths,
        updates,
        catalog_age_seconds: catalog_age(&health),
        marketplace_unreachable,
        cache,
    };
    Ok((state, check))
}

const LAST_SYNC_FILE: &str = "last-sync.json";

/// The header says when the app last reached the sources. Cached loads happen
/// after every operation, so they read this instead of claiming "just now".
fn write_last_sync(cache: &Path, checked: u64) {
    let _ = std::fs::create_dir_all(cache);
    let _ = std::fs::write(cache.join(LAST_SYNC_FILE), checked.to_string());
}

pub(super) fn read_last_sync(cache: &Path) -> Option<u64> {
    std::fs::read_to_string(cache.join(LAST_SYNC_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()
}

const HEALTH_FILE: &str = "sync-health.json";
/// A source is retired after this many "gone" answers in a row...
const GONE_RETIRE_COUNT: u32 = 3;
/// ...spanning at least this long (decision D2).
const GONE_RETIRE_SECONDS: u64 = 3 * 86_400;

/// What the syncs learned about each catalog and source, keyed by
/// `repositoryKey` or `sourceKey`. Cached loads show it; the gone grace period
/// counts with it.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SyncHealth {
    #[serde(default)]
    pub(super) connectivity: Connectivity,
    #[serde(default)]
    pub(super) entries: BTreeMap<String, HealthEntry>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct HealthEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) last_success_at: Option<u64>,
    /// The last sync's result, shown again by cached loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) status: Option<SourceStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gone_since: Option<u64>,
    #[serde(default)]
    gone_count: u32,
}

impl HealthEntry {
    fn record(&mut self, status: SourceStatus, message: Option<&String>, now: u64) {
        if status == SourceStatus::Fresh {
            self.last_success_at = Some(now);
            self.gone_since = None;
            self.gone_count = 0;
        }
        self.status = Some(status);
        self.message = message.cloned();
    }

    /// The server answered with something other than "gone", so the next
    /// "gone" starts a new streak. Unreachable answers leave it alone.
    fn not_gone(&mut self) {
        self.gone_since = None;
        self.gone_count = 0;
    }

    /// Counts one more "gone" answer; true once there have been three in a
    /// row spanning at least three days.
    fn gone(&mut self, now: u64) -> bool {
        self.gone_count += 1;
        let since = *self.gone_since.get_or_insert(now);
        self.gone_count >= GONE_RETIRE_COUNT && now.saturating_sub(since) >= GONE_RETIRE_SECONDS
    }

    fn gone_message(&self, name: &str, retires: bool) -> String {
        let since = crate::marketplace::rfc3339_from_epoch(self.gone_since.unwrap_or_default());
        let date = since.get(..10).unwrap_or(&since);
        if retires {
            format!(
                "{name} was not found on the server (first noticed {date}). The saved copy stays available, and it is removed if it is still missing after 3 days."
            )
        } else {
            format!(
                "{name} was not found on the server (first noticed {date}). The saved copy stays available."
            )
        }
    }
}

pub(super) fn read_health(cache: &Path) -> SyncHealth {
    std::fs::read(cache.join(HEALTH_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_health(cache: &Path, health: &SyncHealth) {
    if let Ok(json) = serde_json::to_vec_pretty(health) {
        let _ = std::fs::create_dir_all(cache);
        let _ = std::fs::write(cache.join(HEALTH_FILE), json);
    }
}

/// Seconds since the default catalog was last fetched, for the preflight.
fn catalog_age(health: &SyncHealth) -> Option<u64> {
    let key = default_catalog_locator().ok().flatten()?.repository_key();
    let at = health.entries.get(&key)?.last_success_at?;
    Some(current_epoch_seconds().saturating_sub(at))
}

/// Joins the marketplace index, runs the preflight, and reports the heartbeat.
/// Every step is best-effort: an unreachable server leaves the cached catalog
/// and installed packages usable.
fn enrich_with_marketplace(mut state: AppState, check: MarketplaceCheck) -> AppState {
    let Some(base_url) = crate::locator::marketplace_base_url() else {
        return state;
    };
    state.marketplace_url = Some(base_url.to_string());
    let index = if check.marketplace_unreachable {
        crate::marketplace::read_cached_index(&check.cache)
    } else {
        crate::marketplace::index_with_cache(&check.cache)
    };
    if let Some(index) = &index {
        super::project::apply_index(&mut state.items, index);
    }
    let (report, findings) = crate::preflight::run(&crate::preflight::PreflightInput {
        paths: &check.paths,
        startup: crate::STARTUP_REPORT.get(),
        profiles: &state.agent_profiles,
        items: &state.items,
        catalog_age_seconds: check.catalog_age_seconds,
        ledger_error: check.ledger_error,
    });
    report.write_cache(&check.cache);
    state.identity = super::project::remember_identity(&check.cache, &report, findings.identity);
    let checks = report.status_map();
    state.preflight = Some(report);

    let installed = state
        .items
        .iter()
        .filter(|item| super::items::counts_as_installed(item.status))
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let mut events = vec![crate::marketplace::ClientEvent::heartbeat(
        check.agents.clone(),
        installed,
        checks,
    )];
    for (id, from_version) in check.updates {
        let version = index
            .as_ref()
            .and_then(|index| index.package(&id))
            .map(|package| package.version.clone());
        events.push(crate::marketplace::ClientEvent::update(
            &id,
            from_version,
            version,
            check.agents.clone(),
        ));
    }
    crate::marketplace::send_events_background(events);
    state
}

/// One line naming what sync retired because it no longer exists upstream.
fn retired_message(repositories: &[String], sources: &[String]) -> Option<String> {
    let mut parts = Vec::new();
    if !repositories.is_empty() {
        parts.push(format!(
            "Removed the retired catalog{} {}.",
            if repositories.len() == 1 { "" } else { "s" },
            repositories.join(", ")
        ));
    }
    if !sources.is_empty() {
        parts.push(format!(
            "Removed the retired source{} {}.",
            if sources.len() == 1 { "" } else { "s" },
            sources.join(", ")
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Takes the fetch result for the definition `matches` picks, if any.
fn take_fetch<D, C>(
    fetched: &mut Vec<(D, Fetch<C>)>,
    matches: impl Fn(&D) -> bool,
) -> Option<Fetch<C>> {
    let index = fetched
        .iter()
        .position(|(definition, _)| matches(definition))?;
    Some(fetched.swap_remove(index).1)
}

/// Activates each catalog's fetch. A catalog that is not found keeps its
/// saved copy until it has been gone three times over three days; then it is
/// retired - dropped from the configuration and its cache wiped - unless it is
/// the default catalog. The returned names are the retired catalogs.
fn apply_repositories(
    cache: &Path,
    health: &mut SyncHealth,
    now: u64,
    definitions: Vec<ConfiguredRepository>,
    mut fetched: Vec<(ConfiguredRepository, Fetch<RepositoryCandidate>)>,
) -> (
    Vec<ConfiguredRepository>,
    Vec<LoadedRepository>,
    Vec<String>,
) {
    let default = default_catalog_locator().ok().flatten();
    let mut updated = Vec::with_capacity(definitions.len());
    let mut loaded = Vec::with_capacity(definitions.len());
    let mut retired = Vec::new();
    for definition in definitions {
        let entry = health
            .entries
            .entry(definition.repository_key.clone())
            .or_default();
        let fetch = take_fetch(&mut fetched, |fetched| {
            fetched.repository_key == definition.repository_key
        });
        let (status, message) = match fetch {
            // Configured while this sync ran; the next one refreshes it.
            None => (SourceStatus::Cached, None),
            Some(Fetch::Ready(candidate))
                if candidate.definition.repository_id != definition.repository_id =>
            {
                source::discard_repository(&candidate);
                (
                    SourceStatus::Error,
                    Some(format!(
                        "The catalog changed repository.id from {} to {}. The last validated revision remains active.",
                        definition.repository_id, candidate.definition.repository_id
                    )),
                )
            }
            Some(Fetch::Ready(candidate)) => match source::activate_repository(cache, candidate) {
                Ok(snapshot) => {
                    entry.record(SourceStatus::Fresh, None, now);
                    updated.push(snapshot.definition.clone());
                    loaded.push(LoadedRepository {
                        definition: snapshot.definition.clone(),
                        snapshot: Some(snapshot),
                        status: SourceStatus::Fresh,
                        refresh_failed: false,
                        message: None,
                        last_success_at: Some(now),
                    });
                    continue;
                }
                Err(message) => {
                    entry.not_gone();
                    (SourceStatus::Error, Some(message))
                }
            },
            Some(Fetch::Gone) => {
                let is_default = default
                    .as_ref()
                    .is_some_and(|default| definition.locator.same_identity(default));
                if entry.gone(now) && !is_default {
                    if let Err(error) =
                        source::remove_repository_cache(cache, &definition.repository_key)
                    {
                        eprintln!(
                            "Could not remove the cache of the retired catalog {}: {error}",
                            definition.name
                        );
                    }
                    retired.push(definition.name.clone());
                    continue;
                }
                (
                    SourceStatus::Error,
                    Some(entry.gone_message(&definition.name, !is_default)),
                )
            }
            Some(Fetch::Unreachable(message)) => (SourceStatus::Stale, Some(message)),
            Some(Fetch::Failed(message)) => {
                entry.not_gone();
                (SourceStatus::Error, Some(message))
            }
        };
        entry.record(status, message.as_ref(), now);
        let snapshot = source::load_current_repository(cache, &definition)
            .ok()
            .flatten();
        loaded.push(LoadedRepository {
            definition: definition.clone(),
            snapshot,
            status,
            refresh_failed: status != SourceStatus::Cached,
            message,
            last_success_at: entry.last_success_at,
        });
        updated.push(definition);
    }
    for (_, fetch) in fetched {
        if let Fetch::Ready(candidate) = fetch {
            source::discard_repository(&candidate);
        }
    }
    (updated, loaded, retired)
}

/// Activates each source's fetch. A source that is not found keeps its saved
/// copy until it has been gone three times over three days. Then, with nothing
/// installed from it, it is retired: dropped from the configuration and its
/// cache wiped. One with installed packages keeps its definition, without the
/// saved copy, so the packages can still be removed. The returned names are
/// the retired sources.
fn apply_sources(
    cache: &Path,
    health: &mut SyncHealth,
    now: u64,
    definitions: Vec<ConfiguredSource>,
    mut fetched: Vec<(ConfiguredSource, Fetch<SourceCandidate>)>,
    installed_keys: &BTreeSet<String>,
) -> (Vec<ConfiguredSource>, Vec<LoadedSource>, Vec<String>) {
    let mut claimed = definitions
        .iter()
        .map(|source| (source.source_id.clone(), source.source_key.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut updated = Vec::with_capacity(definitions.len());
    let mut loaded = Vec::with_capacity(definitions.len());
    let mut retired = Vec::new();
    for definition in definitions {
        let entry = health
            .entries
            .entry(definition.source_key.clone())
            .or_default();
        let fetch = take_fetch(&mut fetched, |fetched| {
            fetched.source_key == definition.source_key
        });
        let (status, message) = match fetch {
            // Configured while this sync ran; the next one refreshes it.
            None => (SourceStatus::Cached, None),
            Some(Fetch::Ready(candidate)) => {
                let source_id_changed = candidate.definition.source_id != definition.source_id;
                let duplicate_namespace = claimed
                    .get(&candidate.definition.source_id)
                    .is_some_and(|source_key| source_key != &definition.source_key);
                if source_id_changed || duplicate_namespace {
                    source::discard_candidate(&candidate);
                    let message = if source_id_changed {
                        format!(
                            "The source changed source.id from {} to {}. The last validated revision remains active.",
                            definition.source_id, candidate.definition.source_id
                        )
                    } else {
                        format!(
                            "The namespace {} is already claimed by another source.",
                            candidate.definition.source_id
                        )
                    };
                    (SourceStatus::Error, Some(message))
                } else {
                    match source::activate_candidate(cache, candidate) {
                        Ok(snapshot) => {
                            claimed.insert(
                                snapshot.definition.source_id.clone(),
                                snapshot.definition.source_key.clone(),
                            );
                            entry.record(SourceStatus::Fresh, None, now);
                            updated.push(snapshot.definition.clone());
                            loaded.push(LoadedSource {
                                definition: snapshot.definition.clone(),
                                snapshot: Some(snapshot),
                                status: SourceStatus::Fresh,
                                refresh_failed: false,
                                message: None,
                                last_success_at: Some(now),
                            });
                            continue;
                        }
                        Err(message) => {
                            entry.not_gone();
                            (SourceStatus::Error, Some(message))
                        }
                    }
                }
            }
            Some(Fetch::Gone) => {
                if entry.gone(now) {
                    if let Err(error) = source::remove_source_cache(cache, &definition.source_key) {
                        eprintln!(
                            "Could not remove the cache of the retired source {}: {error}",
                            definition.name
                        );
                    }
                    if !installed_keys.contains(&definition.source_key) {
                        retired.push(definition.name.clone());
                        continue;
                    }
                    // The definition stays so installed packages can be uninstalled, but the
                    // snapshot is gone: nothing else from the source may be installed from cache.
                    (
                        SourceStatus::Error,
                        Some(format!(
                            "{} is no longer available. Its installed packages remain until you uninstall them.",
                            definition.name
                        )),
                    )
                } else {
                    (
                        SourceStatus::Error,
                        Some(entry.gone_message(&definition.name, true)),
                    )
                }
            }
            Some(Fetch::Unreachable(message)) => (SourceStatus::Stale, Some(message)),
            Some(Fetch::Failed(message)) => {
                entry.not_gone();
                (SourceStatus::Error, Some(message))
            }
        };
        entry.record(status, message.as_ref(), now);
        let snapshot = source::load_current(cache, &definition).ok().flatten();
        loaded.push(LoadedSource {
            definition: definition.clone(),
            snapshot,
            status,
            refresh_failed: status != SourceStatus::Cached,
            message,
            last_success_at: entry.last_success_at,
        });
        updated.push(definition);
    }
    for (_, fetch) in fetched {
        if let Fetch::Ready(candidate) = fetch {
            source::discard_candidate(&candidate);
        }
    }
    (updated, loaded, retired)
}

pub(super) fn is_unsupported_legacy_install(record: &InstallationRecord) -> bool {
    record.manifest_version == 1
        || matches!(
            record.component_kind.as_str(),
            "agentPlugin" | "legacyFileTree" | "fileTree"
        )
}

/// Uninstalls installs this version no longer supports. A failure - a locked
/// file, an unreadable ledger - is logged and the next load or sync tries
/// again; it never blocks the app.
pub(super) fn retire_unsupported_legacy_installs(paths: &SystemPaths) {
    let ledger = match crate::executor::read_ledger(paths) {
        Ok(ledger) => ledger,
        Err(error) => {
            eprintln!("Could not check for retired legacy installs: {error}");
            return;
        }
    };
    let mut groups = BTreeMap::<String, (ConfiguredSource, Vec<String>)>::new();
    for (id, record) in &ledger.items {
        if !is_unsupported_legacy_install(record) {
            continue;
        }
        groups
            .entry(record.source_key.clone())
            .or_insert_with(|| {
                (
                    ConfiguredSource {
                        source_key: record.source_key.clone(),
                        source_id: record.source_id.clone(),
                        name: record.source_id.clone(),
                        description: "Retired unsupported legacy installation.".to_string(),
                        locator: Locator::parse(&record.source_url)
                            .unwrap_or_else(|_| Locator::display_url(record.source_url.clone())),
                        repository_key: None,
                    },
                    Vec::new(),
                )
            })
            .1
            .push(id.clone());
    }
    for (source, ids) in groups.into_values() {
        if let Err(error) = crate::executor::uninstall_batch(paths, &source, &ids, true) {
            eprintln!(
                "Could not remove the retired legacy installs from {}; the next sync tries again: {error}",
                source.name
            );
        }
    }
}

pub(super) fn reconcile_installed_items(
    paths: &SystemPaths,
    loaded: &[LoadedSource],
) -> Result<AutoUpdateReport, String> {
    let mut report = AutoUpdateReport::default();
    let agents_enabled = agent_profiles::read(paths)?
        .iter()
        .any(|profile| profile.enabled);
    for source in loaded {
        let Some(snapshot) = &source.snapshot else {
            continue;
        };
        let ledger_state = crate::executor::read_ledger(paths)?;
        let mut candidates = Vec::new();
        for item in snapshot.catalog.items.values() {
            if item.manifest_version == 2 && !agents_enabled {
                continue;
            }
            if super::status::refined_item_status(paths, &ledger_state, snapshot, item, None)
                != ItemStatus::UpdateAvailable
            {
                continue;
            }
            let selected = ledger_state
                .items
                .get(&item.id)
                .map(|record| crate::planner::selected_component_ids(record, item));
            let identities = crate::planner::plan(paths, snapshot, item, None, selected.as_deref())
                .map(|plan| {
                    plan.resources
                        .values()
                        .map(|resource| resource.desired.identity())
                        .collect()
                })
                .unwrap_or_else(|_| BTreeSet::from([format!("item:{}", item.id)]));
            candidates.push(UpdateCandidate { item, identities });
        }

        for group in overlapping_update_groups(candidates) {
            let result = if let [candidate] = group.as_slice() {
                let current = crate::executor::read_ledger(paths)?;
                let selected = current
                    .items
                    .get(&candidate.item.id)
                    .map(|record| crate::planner::selected_component_ids(record, candidate.item));
                install::install_item_components_approved(
                    paths,
                    &source.definition,
                    snapshot,
                    candidate.item,
                    false,
                    selected.as_deref(),
                )
            } else {
                let requests = group
                    .iter()
                    .map(|candidate| crate::executor::BatchInstall {
                        source: &source.definition,
                        snapshot,
                        item: candidate.item,
                        replace_unmanaged: false,
                    })
                    .collect::<Vec<_>>();
                crate::executor::install_batch(paths, &requests, false)
            };
            match result {
                Ok(_) => report
                    .updated_items
                    .extend(group.iter().map(|candidate| ItemReference {
                        id: candidate.item.id.clone(),
                        source_id: candidate.item.source_id.clone(),
                        local_id: candidate.item.local_id.clone(),
                    })),
                Err(message) => {
                    report
                        .failed_items
                        .extend(group.iter().map(|candidate| ItemFailure {
                            id: candidate.item.id.clone(),
                            message: message.clone(),
                        }))
                }
            }
        }
    }
    Ok(report)
}

struct UpdateCandidate<'a> {
    item: &'a crate::catalog::CatalogItem,
    identities: BTreeSet<String>,
}

fn overlapping_update_groups<'a>(
    candidates: Vec<UpdateCandidate<'a>>,
) -> Vec<Vec<UpdateCandidate<'a>>> {
    let mut groups = Vec::<Vec<UpdateCandidate<'a>>>::new();
    for candidate in candidates {
        let mut connected = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            if group
                .iter()
                .any(|member| !member.identities.is_disjoint(&candidate.identities))
            {
                connected.push(index);
            }
        }
        if connected.is_empty() {
            groups.push(vec![candidate]);
            continue;
        }
        let first = connected[0];
        groups[first].push(candidate);
        for index in connected.into_iter().skip(1).rev() {
            let merged = groups.remove(index);
            groups[first].extend(merged);
        }
    }
    groups
}

#[cfg(test)]
mod retire_tests {
    use super::*;
    use crate::locator::Locator;
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::net::TcpListener;

    fn serve_not_found(requests: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        std::thread::spawn(move || {
            for _ in 0..requests {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok() {
                    if line == "\r\n" || line == "\n" || line.is_empty() {
                        break;
                    }
                    line.clear();
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        format!("http://{address}")
    }

    fn refresh_sources(
        cache: &Path,
        health: &mut SyncHealth,
        now: u64,
        definitions: Vec<ConfiguredSource>,
        installed: &BTreeSet<String>,
    ) -> (Vec<ConfiguredSource>, Vec<LoadedSource>, Vec<String>) {
        let hosts = DeadHosts::default();
        let fetched = definitions
            .iter()
            .map(|definition| {
                let fetch = hosts.fetch(definition.url(), || prepare_source(cache, definition));
                (definition.clone(), fetch)
            })
            .collect();
        apply_sources(cache, health, now, definitions, fetched, installed)
    }

    const DAY: u64 = 86_400;

    fn cache_snapshot(cache: &Path, definition: &ConfiguredSource) {
        let staged = cache.join("staged");
        std::fs::create_dir_all(staged.join("skills/review")).expect("skill");
        std::fs::write(
            staged.join("skills/review/SKILL.md"),
            "---\nname: review\ndescription: Reviews code\n---\nBody\n",
        )
        .expect("skill");
        std::fs::write(
            staged.join("agent-plugins.json"),
            r#"{"version":2,"source":{"id":"retired","name":"Retired","description":"Gone"},
               "packages":[{"id":"review","components":[{"kind":"skill","path":"skills/review"}]}]}"#,
        )
        .expect("manifest");
        let catalog = crate::catalog::read_manifest_catalog(&staged, &definition.source_key)
            .expect("catalog");
        source::activate_candidate(
            cache,
            SourceCandidate {
                definition: definition.clone(),
                commit: "a".repeat(64),
                path: staged,
                catalog,
                staged: true,
                validators: crate::artifact::ArtifactValidators::default(),
            },
        )
        .expect("snapshot");
    }

    #[test]
    fn any_other_answer_from_the_server_restarts_the_gone_streak() {
        let mut entry = HealthEntry::default();
        assert!(!entry.gone(0));
        assert!(!entry.gone(DAY));
        entry.not_gone();
        assert!(
            !entry.gone(10 * DAY),
            "one gone answer after a reset is not three"
        );
    }

    #[test]
    fn a_gone_source_is_retired_only_after_three_gone_answers_over_three_days() {
        let base = serve_not_found(12);
        let cache = tempfile::tempdir().expect("cache");
        let locator = Locator::display_url(format!("{base}/retired-latest.zip"));
        let definition = ConfiguredSource {
            source_key: locator.source_key(),
            source_id: "retired".to_string(),
            name: "Retired".to_string(),
            description: "Gone".to_string(),
            locator,
            repository_key: None,
        };
        let root = source::source_cache_root(cache.path(), &definition.source_key);
        cache_snapshot(cache.path(), &definition);
        let mut health = SyncHealth::default();
        let none = BTreeSet::new();

        for now in [DAY, 2 * DAY, 3 * DAY] {
            let (updated, loaded, retired) = refresh_sources(
                cache.path(),
                &mut health,
                now,
                vec![definition.clone()],
                &none,
            );
            assert_eq!(
                updated.len(),
                1,
                "day {}: kept during the grace period",
                now / DAY
            );
            assert!(retired.is_empty());
            assert!(loaded[0].snapshot.is_some(), "the saved copy stays usable");
            assert!(loaded[0]
                .message
                .as_deref()
                .is_some_and(|message| message.contains("not found on the server")));
        }
        let (updated, _, retired) = refresh_sources(
            cache.path(),
            &mut health,
            4 * DAY,
            vec![definition.clone()],
            &none,
        );
        assert!(updated.is_empty());
        assert_eq!(retired, vec!["Retired".to_string()]);
        assert!(!root.exists(), "a retired source keeps no cache");

        let installed = BTreeSet::from([definition.source_key.clone()]);
        let (updated, loaded, retired) = refresh_sources(
            cache.path(),
            &mut health,
            5 * DAY,
            vec![definition],
            &installed,
        );
        assert_eq!(updated.len(), 1, "installed packages keep the definition");
        assert!(retired.is_empty());
        assert!(loaded[0].snapshot.is_none());
        assert!(loaded[0]
            .message
            .as_deref()
            .is_some_and(|message| message.contains("no longer available")));
    }

    #[test]
    fn a_gone_catalog_is_retired_after_the_grace_period() {
        let base = serve_not_found(2);
        let cache = tempfile::tempdir().expect("cache");
        let locator = Locator::display_url(format!("{base}/catalog.json"));
        let definition = ConfiguredRepository {
            repository_key: locator.repository_key(),
            repository_id: "nexus".to_string(),
            name: "Nexus".to_string(),
            description: "Retired host".to_string(),
            locator,
        };
        let mut health = SyncHealth::default();
        health.entries.insert(
            definition.repository_key.clone(),
            HealthEntry {
                gone_since: Some(0),
                gone_count: 2,
                ..HealthEntry::default()
            },
        );
        let fetch = DeadHosts::default().fetch(definition.url(), || {
            prepare_repository(cache.path(), &definition)
        });
        let (updated, loaded, retired) = apply_repositories(
            cache.path(),
            &mut health,
            3 * DAY,
            vec![definition.clone()],
            vec![(definition, fetch)],
        );
        assert!(updated.is_empty());
        assert!(loaded.is_empty());
        assert_eq!(retired, vec!["Nexus".to_string()]);
        assert_eq!(
            retired_message(&retired, &[]).as_deref(),
            Some("Removed the retired catalog Nexus.")
        );
    }

    #[test]
    fn an_unreachable_host_is_skipped_for_the_rest_of_the_pass() {
        let closed = TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", closed.local_addr().expect("addr"));
        drop(closed);
        let hosts = DeadHosts::default();
        let first = hosts.fetch(&format!("{base}/one.zip"), || {
            crate::artifact::head_artifact(&format!("{base}/one.zip"))
        });
        assert!(matches!(first, Fetch::Unreachable(_)));
        let mut called = false;
        let second = hosts.fetch(&format!("{base}/two.zip"), || {
            called = true;
            Ok(())
        });
        assert!(!called, "a second source on a dead host is not fetched");
        assert!(matches!(second, Fetch::Unreachable(message) if message.contains("Skipped")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::read_manifest_catalog;
    use crate::source::{ConfiguredSource, SourceSnapshot, TEST_SOURCE_KEY};
    use std::fs;
    use std::path::Path;

    fn paths(root: &Path) -> SystemPaths {
        SystemPaths {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            local_data: root.join("local-data"),
            cache: root.join("cache"),
            onedrive_commercial: None,
        }
    }

    #[test]
    fn cached_state_keeps_what_the_last_sync_learned() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let cache = root.path().join("cache-base");
        let config = root.path().join("config-base");
        fs::create_dir_all(&cache).expect("cache");
        fs::create_dir_all(&config).expect("config");
        let identity = crate::app_state::MarketplaceIdentity {
            account: "CORP\\jacob".to_string(),
            namespace: "jacob".to_string(),
            display_name: "Jacob".to_string(),
            admin: false,
            auth_mode: "Negotiate".to_string(),
        };
        super::super::project::write_identity_cache(&cache, Some(&identity));

        let state = cached_app_state(&paths, &cache, &config).expect("cached state");

        assert_eq!(
            state.identity.map(|identity| identity.namespace),
            crate::locator::marketplace_base_url().map(|_| "jacob".to_string()),
            "a cached load must carry the identity the last sync stored"
        );
    }

    #[test]
    fn cached_state_keeps_the_last_refresh_result() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let cache = root.path().join("cache-base");
        let config = root.path().join("config-base");
        let source = ConfiguredSource::test_fixture(
            "skillbook",
            "https://nexus.example.com/repository/raw/sources/skillbook-latest.zip",
        );
        source::write_sources_config(
            &config,
            &SourcesConfig {
                repositories: Vec::new(),
                sources: vec![source.clone()],
            },
        )
        .expect("sources");
        let mut health = SyncHealth {
            connectivity: Connectivity::Offline,
            ..SyncHealth::default()
        };
        health.entries.insert(
            source.source_key.clone(),
            HealthEntry {
                last_success_at: Some(42),
                status: Some(SourceStatus::Stale),
                message: Some("Could not connect to nexus.example.com".to_string()),
                ..HealthEntry::default()
            },
        );
        write_health(&cache, &health);
        write_last_sync(&cache, 42);

        let state = cached_app_state(&paths, &cache, &config).expect("cached state");

        assert_eq!(state.connectivity, Connectivity::Offline);
        assert_eq!(state.checked_at_epoch_seconds, 42);
        assert_eq!(state.sources[0].status, SourceStatus::Stale);
        assert!(state.sources[0].refresh_failed);
        assert_eq!(state.sources[0].last_success_at_epoch_seconds, Some(42));
        assert!(state.sources[0]
            .message
            .as_deref()
            .is_some_and(|message| message.contains("Could not connect")));
    }

    fn snapshot(root: &Path, body: &str, commit: char) -> (ConfiguredSource, SourceSnapshot) {
        let source_root = root.join(format!("source-{commit}"));
        let skill_root = source_root.join("skills/python-standards");
        fs::create_dir_all(&skill_root).expect("skill directory");
        fs::write(
            skill_root.join("SKILL.md"),
            format!("---\nname: python-standards\ndescription: Python standards\n---\n{body}\n"),
        )
        .expect("skill");
        fs::write(
            source_root.join("agent-plugins.json"),
            r#"{
              "version": 2,
              "source": {"id":"skillbook","name":"Skillbook","description":"Skills"},
              "packages": [
                {
                  "id":"python-standards",
                  "components":[{"kind":"skill","path":"skills/python-standards"}]
                },
                {
                  "id":"python",
                  "components":[
                    {"kind":"skill","id":"python-standards","path":"skills/python-standards"}
                  ]
                }
              ]
            }"#,
        )
        .expect("manifest");
        let catalog = read_manifest_catalog(&source_root, TEST_SOURCE_KEY).expect("catalog");
        let mut source = ConfiguredSource::test_fixture(
            "skillbook",
            "https://nexus.example.com/repository/raw/sources/skillbook-latest.zip",
        );
        source.source_key = TEST_SOURCE_KEY.to_string();
        let snapshot = SourceSnapshot {
            definition: source.clone(),
            commit: commit.to_string().repeat(40),
            path: source_root,
            catalog,
        };
        (source, snapshot)
    }

    #[test]
    fn background_update_reconciles_packages_that_share_a_changed_component() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        crate::agent_profiles::set_enabled(
            &paths,
            crate::agent_profiles::TargetId::ClaudeCode,
            true,
        )
        .expect("enable Claude");
        let (source, original) = snapshot(root.path(), "Old guidance", 'a');
        for id in ["python-standards", "python"] {
            crate::executor::install(
                &paths,
                &source,
                &original,
                &original.catalog.items[id],
                false,
                false,
            )
            .expect("initial install");
        }

        let (_, updated) = snapshot(root.path(), "New guidance", 'b');
        let loaded = [LoadedSource {
            definition: source,
            snapshot: Some(updated.clone()),
            status: SourceStatus::Fresh,
            refresh_failed: false,
            message: None,
            last_success_at: None,
        }];
        let report = reconcile_installed_items(&paths, &loaded).expect("reconcile");

        assert!(report.failed_items.is_empty());
        assert_eq!(report.updated_items.len(), 2);
        let installed = fs::read_to_string(
            paths
                .home
                .join(".claude/skills/skillbook-python-standards/SKILL.md"),
        )
        .expect("installed skill");
        assert!(installed.contains("New guidance"));
        let ledger = crate::executor::read_ledger(&paths).expect("ledger");
        assert!(updated.catalog.items.values().all(|item| {
            ledger
                .items
                .get(&item.id)
                .is_some_and(|record| record.item_digest == item.digest)
        }));
        assert_eq!(
            ledger
                .resources
                .values()
                .next()
                .expect("shared resource")
                .consumer_binding_ids
                .len(),
            2
        );
    }
}
