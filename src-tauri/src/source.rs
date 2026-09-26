//! Manifest-aware source and source-repository configuration and snapshots.

use crate::artifact::{
    download_artifact, extract_source_archive, head_artifact, is_unreachable,
    require_repository_json, validators_match, ArtifactValidators, DownloadedBytes,
};
use crate::catalog::{read_manifest_catalog, ManifestCatalog};
use crate::fs_retry;
use crate::locator::Locator;
use crate::manifest::{SourceManifest, SOURCE_MANIFEST_FILE};
use crate::repository::{
    report_manifest, RepositoryManifest, RepositoryValidationReport, REPOSITORY_MANIFEST_FILE,
};
use crate::sources::{sync_directory, temporary_path, valid_commit_sha};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const SOURCES_VERSION: u8 = 6;
const CURRENT_POINTER_VERSION: u8 = 2;
const LEGACY_CURRENT_POINTER_VERSION: u8 = 1;
const SOURCES_FILE: &str = "sources.json";
const SOURCES_BACKUP_FILE: &str = "sources.json.previous";
const CURRENT_POINTER_FILE: &str = "current.json";
const CURRENT_POINTER_BACKUP_FILE: &str = "current.json.previous";
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ConfiguredSource {
    pub(crate) source_key: String,
    pub(crate) source_id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) locator: Locator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) repository_key: Option<String>,
}

#[cfg(test)]
pub(crate) const TEST_SOURCE_KEY: &str = "source-test00000000";

impl ConfiguredSource {
    pub(crate) fn url(&self) -> &str {
        self.locator.url()
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(source_id: &str, url: &str) -> Self {
        let locator = Locator::parse(url).expect("test locator");
        Self {
            source_key: locator.source_key(),
            source_id: source_id.to_string(),
            name: source_id.to_string(),
            description: format!("{source_id} source"),
            locator,
            repository_key: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct ConfiguredRepository {
    pub(crate) repository_key: String,
    pub(crate) repository_id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) locator: Locator,
}

impl ConfiguredRepository {
    pub(crate) fn url(&self) -> &str {
        self.locator.url()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SourcesConfig {
    pub(crate) repositories: Vec<ConfiguredRepository>,
    pub(crate) sources: Vec<ConfiguredSource>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SourcesFile {
    version: u8,
    #[serde(default)]
    repositories: Vec<ConfiguredRepository>,
    sources: Vec<ConfiguredSource>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CurrentPointer {
    version: u8,
    revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_modified: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LegacyCurrentPointer {
    version: u8,
    commit: String,
}

#[derive(Clone, Debug)]
pub(crate) struct SourceCandidate {
    pub(crate) definition: ConfiguredSource,
    pub(crate) commit: String,
    pub(crate) path: PathBuf,
    pub(crate) catalog: ManifestCatalog,
    pub(crate) staged: bool,
    pub(crate) validators: ArtifactValidators,
}

#[derive(Clone, Debug)]
pub(crate) struct SourceSnapshot {
    pub(crate) definition: ConfiguredSource,
    pub(crate) commit: String,
    pub(crate) path: PathBuf,
    pub(crate) catalog: ManifestCatalog,
}

#[derive(Clone, Debug)]
pub(crate) struct RepositoryCandidate {
    pub(crate) definition: ConfiguredRepository,
    pub(crate) revision: String,
    pub(crate) path: PathBuf,
    pub(crate) manifest: RepositoryManifest,
    pub(crate) staged: bool,
    pub(crate) validators: ArtifactValidators,
}

#[derive(Clone, Debug)]
pub(crate) struct RepositorySnapshot {
    pub(crate) definition: ConfiguredRepository,
    pub(crate) revision: String,
    pub(crate) manifest: RepositoryManifest,
}

pub(crate) fn sources_path(config_base: &Path) -> PathBuf {
    config_base.join(SOURCES_FILE)
}

/// Reads `sources.json`. An unreadable file falls back to its last good copy
/// (`sources.json.previous`), then to a configuration rebuilt from the cached
/// default catalog and the installed packages' sources, so one damaged file
/// never leaves the app unable to start. The damaged file is kept beside it.
pub(crate) fn read_sources_config(config_base: &Path) -> Result<SourcesConfig, String> {
    read_sources_config_or_rebuild(config_base, rebuild_sources_config)
}

fn read_sources_config_or_rebuild(
    config_base: &Path,
    rebuild: impl FnOnce() -> SourcesConfig,
) -> Result<SourcesConfig, String> {
    recover_sources_file(config_base)?;
    let path = sources_path(config_base);
    let contents = match fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let config = SourcesConfig {
                repositories: Vec::new(),
                sources: Vec::new(),
            };
            write_sources_config(config_base, &config)?;
            return Ok(config);
        }
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    };
    let error = match parse_valid_sources_file(&path, &contents) {
        Ok(config) => return Ok(config),
        Err(error) => error,
    };
    // A version this build does not read is not damage; replacing it would
    // throw away a newer build's configuration.
    let version = serde_json::from_slice::<serde_json::Value>(&contents)
        .ok()
        .and_then(|value| value.get("version").and_then(serde_json::Value::as_u64));
    if version.is_some_and(|version| version != u64::from(SOURCES_VERSION)) {
        return Err(error);
    }
    let backup = config_base.join(SOURCES_BACKUP_FILE);
    let config = match fs::read(&backup)
        .map_err(|error| error.to_string())
        .and_then(|contents| parse_valid_sources_file(&backup, &contents))
    {
        Ok(config) => {
            eprintln!("{error} Restored the previous copy of the source list.");
            config
        }
        Err(_) => {
            eprintln!(
                "{error} Rebuilt the source list from the saved catalog and installed packages."
            );
            // The damaged copy must not become the next backup.
            let _ = fs_retry::remove_file(&backup);
            rebuild()
        }
    };
    let quarantine =
        crate::ledger::quarantine_path(config_base, &format!("{SOURCES_FILE}.corrupt-"));
    fs_retry::rename(&path, &quarantine)
        .map_err(|error| format!("Could not set aside {}: {error}", path.display()))?;
    write_sources_config(config_base, &config)?;
    Ok(config)
}

fn parse_valid_sources_file(path: &Path, contents: &[u8]) -> Result<SourcesConfig, String> {
    let config = parse_sources_file(path, contents)?;
    validate_sources_config(&config)?;
    Ok(config)
}

/// The configuration the saved caches can vouch for: the default catalog with
/// the sources it lists, and every source something is installed from. A
/// source without a saved copy keeps its installation-record name until the
/// next sync fetches it.
fn rebuild_sources_config() -> SourcesConfig {
    let installed = crate::paths::SystemPaths::from_system()
        .and_then(|paths| crate::executor::read_ledger(&paths))
        .map(|ledger| {
            ledger
                .items
                .values()
                .map(|record| (record.source_id.clone(), record.source_url.clone()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    match crate::sources::cache_base_dir() {
        Ok(cache) => rebuild_sources(
            &cache,
            crate::locator::default_catalog_locator().ok().flatten(),
            &installed,
        ),
        Err(_) => SourcesConfig::default(),
    }
}

fn rebuild_sources(
    cache_base: &Path,
    default_catalog: Option<Locator>,
    installed: &[(String, String)],
) -> SourcesConfig {
    let mut config = SourcesConfig::default();
    let mut wanted = Vec::new();
    if let Some(locator) = default_catalog {
        let key = locator.repository_key();
        let manifest = read_current_pointer(&repository_cache_root(cache_base, &key))
            .ok()
            .flatten()
            .and_then(|pointer| {
                RepositoryManifest::from_path_tolerant(&repository_revision_path(
                    cache_base,
                    &key,
                    &pointer.revision,
                ))
                .ok()
            });
        if let Some(manifest) = manifest {
            for listed in &manifest.sources {
                if let Ok(listed) = listed.locator() {
                    wanted.push((listed, Some(key.clone()), None));
                }
            }
            config
                .repositories
                .push(configured_from_repository_manifest(key, locator, &manifest));
        }
    }
    for (source_id, url) in installed {
        if let Ok(locator) = Locator::parse(url) {
            wanted.push((locator, None, Some(source_id.as_str())));
        }
    }
    for (locator, repository_key, recorded_id) in wanted {
        if config
            .sources
            .iter()
            .any(|source| source.locator.same_identity(&locator))
        {
            continue;
        }
        let key = locator.source_key();
        let cached = read_current_pointer(&source_cache_root(cache_base, &key))
            .ok()
            .flatten()
            .and_then(|pointer| {
                read_manifest_catalog(&revision_path(cache_base, &key, &pointer.revision), &key)
                    .ok()
            });
        let definition = match (cached, recorded_id) {
            (Some(catalog), _) => configured_from_catalog(key, locator, repository_key, &catalog),
            (None, Some(source_id)) => ConfiguredSource {
                source_key: key,
                source_id: source_id.to_string(),
                name: source_id.to_string(),
                description: "Restored from the installed packages.".to_string(),
                locator,
                repository_key,
            },
            (None, None) => continue,
        };
        if !config
            .sources
            .iter()
            .any(|source| source.source_id == definition.source_id)
        {
            config.sources.push(definition);
        }
    }
    config.sources.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source_id.cmp(&right.source_id))
    });
    if validate_sources_config(&config).is_err() {
        config.sources.retain(|source| {
            validate_sources(std::slice::from_ref(source), &config.repositories).is_ok()
        });
    }
    config
}

pub(crate) fn read_sources(config_base: &Path) -> Result<Vec<ConfiguredSource>, String> {
    Ok(read_sources_config(config_base)?.sources)
}

pub(crate) fn write_sources_config(
    config_base: &Path,
    config: &SourcesConfig,
) -> Result<(), String> {
    validate_sources_config(config)?;
    fs::create_dir_all(config_base)
        .map_err(|error| format!("Could not create {}: {error}", config_base.display()))?;
    recover_sources_file(config_base)?;
    let file = SourcesFile {
        version: SOURCES_VERSION,
        repositories: config.repositories.clone(),
        sources: config.sources.clone(),
    };
    let mut contents = serde_json::to_vec_pretty(&file)
        .map_err(|error| format!("Could not serialize source configuration: {error}"))?;
    contents.push(b'\n');
    atomic_write_with_backup(
        config_base,
        &sources_path(config_base),
        &config_base.join(SOURCES_BACKUP_FILE),
        "sources-writing",
        &contents,
    )
}

pub(crate) fn configured_source(
    config_base: &Path,
    source_id: &str,
) -> Result<ConfiguredSource, String> {
    read_sources(config_base)?
        .into_iter()
        .find(|source| source.source_id == source_id)
        .ok_or_else(|| format!("Unknown source: {source_id}"))
}

pub(crate) fn source_cache_root(cache_base: &Path, source_key: &str) -> PathBuf {
    cache_base.join("sources").join(source_key)
}

pub(crate) fn repository_cache_root(cache_base: &Path, repository_key: &str) -> PathBuf {
    cache_base.join("repositories").join(repository_key)
}

pub(crate) fn revision_path(cache_base: &Path, source_key: &str, commit: &str) -> PathBuf {
    source_cache_root(cache_base, source_key)
        .join("revisions")
        .join(commit)
}

pub(crate) fn repository_revision_path(
    cache_base: &Path,
    repository_key: &str,
    revision: &str,
) -> PathBuf {
    repository_cache_root(cache_base, repository_key)
        .join("revisions")
        .join(revision)
}

pub(crate) fn load_current(
    cache_base: &Path,
    definition: &ConfiguredSource,
) -> Result<Option<SourceSnapshot>, String> {
    let source_root = source_cache_root(cache_base, &definition.source_key);
    let Some(pointer) = read_current_pointer(&source_root)? else {
        return Ok(None);
    };
    let path = revision_path(cache_base, &definition.source_key, &pointer.revision);
    let catalog = read_manifest_catalog(&path, &definition.source_key)?;
    if catalog.manifest.source().id != definition.source_id {
        return Err(format!(
            "Source {} changed its manifest id from {} to {}. The last validated revision remains active.",
            definition.url(),
            definition.source_id,
            catalog.manifest.source().id
        ));
    }
    let normalized = configured_from_catalog(
        definition.source_key.clone(),
        definition.locator.clone(),
        definition.repository_key.clone(),
        &catalog,
    );
    Ok(Some(SourceSnapshot {
        definition: normalized,
        commit: pointer.revision,
        path,
        catalog,
    }))
}

pub(crate) fn load_current_repository(
    cache_base: &Path,
    definition: &ConfiguredRepository,
) -> Result<Option<RepositorySnapshot>, String> {
    let root = repository_cache_root(cache_base, &definition.repository_key);
    let Some(pointer) = read_current_pointer(&root)? else {
        return Ok(None);
    };
    let path = repository_revision_path(cache_base, &definition.repository_key, &pointer.revision);
    let manifest = RepositoryManifest::from_path_tolerant(&path)?;
    if manifest.repository.id != definition.repository_id {
        return Err(format!(
            "Source repository {} changed its id from {} to {}. The last validated revision remains active.",
            definition.url(),
            definition.repository_id,
            manifest.repository.id
        ));
    }
    Ok(Some(repository_snapshot(
        configured_from_repository_manifest(
            definition.repository_key.clone(),
            definition.locator.clone(),
            &manifest,
        ),
        pointer.revision,
        manifest,
    )?))
}

pub(crate) fn prepare_new_source(
    locator: &Locator,
    cache_base: &Path,
    repository_key: Option<String>,
    expected_source_id: Option<&str>,
) -> Result<SourceCandidate, String> {
    let mut candidate = prepare_candidate(locator, &locator.source_key(), cache_base)?;
    candidate.definition.repository_key = repository_key;
    if let Some(expected) = expected_source_id {
        if candidate.definition.source_id != expected {
            discard_candidate(&candidate);
            return Err(format!(
                "The catalog listed source.id {expected}, but the fetched source publishes {}.",
                candidate.definition.source_id
            ));
        }
    }
    Ok(candidate)
}

pub(crate) fn prepare_refresh(
    source: &ConfiguredSource,
    cache_base: &Path,
) -> Result<SourceCandidate, String> {
    let mut candidate = prepare_candidate(&source.locator, &source.source_key, cache_base)?;
    candidate.definition.repository_key = source.repository_key.clone();
    Ok(candidate)
}

pub(crate) fn prepare_new_repository(
    locator: &Locator,
    cache_base: &Path,
) -> Result<RepositoryCandidate, String> {
    prepare_repository_candidate(locator, &locator.repository_key(), cache_base)
}

pub(crate) fn prepare_repository_refresh(
    repository: &ConfiguredRepository,
    cache_base: &Path,
) -> Result<RepositoryCandidate, String> {
    prepare_repository_candidate(&repository.locator, &repository.repository_key, cache_base)
}

fn prepare_candidate(
    locator: &Locator,
    source_key: &str,
    cache_base: &Path,
) -> Result<SourceCandidate, String> {
    let source_root = source_cache_root(cache_base, source_key);
    fs::create_dir_all(&source_root)
        .map_err(|error| format!("Could not create {}: {error}", source_root.display()))?;
    prepare_artifact_source(source_key, locator, locator.url(), cache_base, &source_root)
}

fn prepare_artifact_source(
    source_key: &str,
    locator: &Locator,
    url: &str,
    cache_base: &Path,
    source_root: &Path,
) -> Result<SourceCandidate, String> {
    let stored = read_current_pointer(source_root)?;
    if let Some(pointer) = &stored {
        match head_artifact(url) {
            Ok(remote) if validators_match(&pointer.validators(), &remote) => {
                if let Some(mut current) = reuse_source_revision(
                    cache_base,
                    source_key,
                    locator,
                    source_root,
                    &pointer.revision,
                )? {
                    current.validators = remote;
                    return Ok(current);
                }
            }
            // The download would fail the same way.
            Err(error) if is_unreachable(&error) => return Err(error),
            _ => {}
        }
    }
    let downloaded = match (
        download_artifact(url, conditional_validators(stored.as_ref()).as_ref())?,
        &stored,
    ) {
        (Some(downloaded), _) => downloaded,
        (None, Some(pointer)) => {
            if let Some(mut current) = reuse_source_revision(
                cache_base,
                source_key,
                locator,
                source_root,
                &pointer.revision,
            )? {
                current.validators = pointer.validators();
                return Ok(current);
            }
            download_unconditionally(url)?
        }
        (None, None) => download_unconditionally(url)?,
    };
    if stored
        .as_ref()
        .is_some_and(|pointer| pointer.revision == downloaded.digest)
    {
        if let Some(mut current) = reuse_source_revision(
            cache_base,
            source_key,
            locator,
            source_root,
            &downloaded.digest,
        )? {
            current.validators = downloaded.validators;
            return Ok(current);
        }
    }
    stage_artifact_source(source_key, locator, downloaded, source_root)
}

/// The validators for a conditional download, when the cache has any.
fn conditional_validators(stored: Option<&CurrentPointer>) -> Option<ArtifactValidators> {
    stored
        .map(CurrentPointer::validators)
        .filter(|validators| validators.etag.is_some() || validators.last_modified.is_some())
}

/// A full download, for when the server answered 304 but the saved copy it
/// vouched for is unreadable.
fn download_unconditionally(url: &str) -> Result<DownloadedBytes, String> {
    download_artifact(url, None)?.ok_or_else(|| {
        "The server answered 304 Not Modified to a request that was not conditional.".to_string()
    })
}

fn reuse_source_revision(
    cache_base: &Path,
    source_key: &str,
    locator: &Locator,
    source_root: &Path,
    revision: &str,
) -> Result<Option<SourceCandidate>, String> {
    if read_current_pointer(source_root)?.is_none_or(|pointer| pointer.revision != revision) {
        return Ok(None);
    }
    let path = revision_path(cache_base, source_key, revision);
    let Ok(catalog) = read_manifest_catalog(&path, source_key) else {
        return Ok(None);
    };
    Ok(Some(SourceCandidate {
        definition: configured_from_catalog(
            source_key.to_string(),
            locator.clone(),
            None,
            &catalog,
        ),
        commit: revision.to_string(),
        path,
        catalog,
        staged: false,
        validators: ArtifactValidators::default(),
    }))
}

fn stage_artifact_source(
    source_key: &str,
    locator: &Locator,
    downloaded: DownloadedBytes,
    source_root: &Path,
) -> Result<SourceCandidate, String> {
    let staging = temporary_path(source_root, "source-preparing");
    let result = (|| {
        extract_source_archive(&downloaded.bytes, &staging)?;
        let catalog = read_manifest_catalog(&staging, source_key).map_err(|error| {
            format!("This artifact is not a valid Agent Plugins source: {error}")
        })?;
        let definition =
            configured_from_catalog(source_key.to_string(), locator.clone(), None, &catalog);
        Ok(SourceCandidate {
            definition,
            commit: downloaded.digest.clone(),
            path: staging.clone(),
            catalog,
            staged: true,
            validators: downloaded.validators,
        })
    })();
    if result.is_err() && staging.exists() {
        let _ = fs_retry::remove_dir_all(&staging);
    }
    result
}

fn prepare_repository_candidate(
    locator: &Locator,
    repository_key: &str,
    cache_base: &Path,
) -> Result<RepositoryCandidate, String> {
    let root = repository_cache_root(cache_base, repository_key);
    fs::create_dir_all(&root)
        .map_err(|error| format!("Could not create {}: {error}", root.display()))?;
    prepare_artifact_repository(locator, locator.url(), cache_base, repository_key, &root)
}

fn prepare_artifact_repository(
    locator: &Locator,
    url: &str,
    cache_base: &Path,
    repository_key: &str,
    root: &Path,
) -> Result<RepositoryCandidate, String> {
    let stored = read_current_pointer(root)?;
    if let Some(pointer) = &stored {
        match head_artifact(url) {
            Ok(remote) if validators_match(&pointer.validators(), &remote) => {
                if let Some(mut current) = load_repository_candidate(
                    cache_base,
                    locator,
                    repository_key,
                    &pointer.revision,
                    false,
                )? {
                    current.validators = remote;
                    return Ok(current);
                }
            }
            // The download would fail the same way.
            Err(error) if is_unreachable(&error) => return Err(error),
            _ => {}
        }
    }
    let downloaded = match (
        download_artifact(url, conditional_validators(stored.as_ref()).as_ref())?,
        &stored,
    ) {
        (Some(downloaded), _) => downloaded,
        (None, Some(pointer)) => {
            if let Some(mut current) = load_repository_candidate(
                cache_base,
                locator,
                repository_key,
                &pointer.revision,
                false,
            )? {
                current.validators = pointer.validators();
                return Ok(current);
            }
            download_unconditionally(url)?
        }
        (None, None) => download_unconditionally(url)?,
    };
    require_repository_json(&downloaded.bytes)?;
    if stored
        .as_ref()
        .is_some_and(|pointer| pointer.revision == downloaded.digest)
    {
        if let Some(mut current) = load_repository_candidate(
            cache_base,
            locator,
            repository_key,
            &downloaded.digest,
            false,
        )? {
            current.validators = downloaded.validators;
            return Ok(current);
        }
    }
    stage_artifact_repository(locator, downloaded, root)
}

fn load_repository_candidate(
    cache_base: &Path,
    locator: &Locator,
    repository_key: &str,
    revision: &str,
    staged: bool,
) -> Result<Option<RepositoryCandidate>, String> {
    let path = repository_revision_path(cache_base, repository_key, revision);
    let Ok(manifest) = RepositoryManifest::from_path_tolerant(&path) else {
        return Ok(None);
    };
    Ok(Some(RepositoryCandidate {
        definition: configured_from_repository_manifest(
            repository_key.to_string(),
            locator.clone(),
            &manifest,
        ),
        revision: revision.to_string(),
        path,
        manifest,
        staged,
        validators: ArtifactValidators::default(),
    }))
}

fn stage_artifact_repository(
    locator: &Locator,
    downloaded: DownloadedBytes,
    root: &Path,
) -> Result<RepositoryCandidate, String> {
    let staging = temporary_path(root, "repository-preparing");
    let result = (|| {
        fs::create_dir_all(&staging)
            .map_err(|error| format!("Could not create {}: {error}", staging.display()))?;
        fs::write(staging.join(REPOSITORY_MANIFEST_FILE), &downloaded.bytes).map_err(|error| {
            format!(
                "Could not write {}: {error}",
                staging.join(REPOSITORY_MANIFEST_FILE).display()
            )
        })?;
        let manifest = RepositoryManifest::from_path_tolerant(&staging)?;
        Ok(RepositoryCandidate {
            definition: configured_from_repository_manifest(
                locator.repository_key(),
                locator.clone(),
                &manifest,
            ),
            revision: downloaded.digest.clone(),
            path: staging.clone(),
            manifest,
            staged: true,
            validators: downloaded.validators,
        })
    })();
    if result.is_err() && staging.exists() {
        let _ = fs_retry::remove_dir_all(&staging);
    }
    result
}

pub(crate) fn activate_candidate(
    cache_base: &Path,
    candidate: SourceCandidate,
) -> Result<SourceSnapshot, String> {
    let source_root = source_cache_root(cache_base, &candidate.definition.source_key);
    let revision = revision_path(
        cache_base,
        &candidate.definition.source_key,
        &candidate.commit,
    );
    retain_revision(
        &revision,
        candidate.staged,
        &candidate.path,
        &candidate.commit,
    )?;
    write_current_pointer(&source_root, &candidate.commit, &candidate.validators)?;
    let catalog = read_manifest_catalog(&revision, &candidate.definition.source_key)?;
    Ok(SourceSnapshot {
        definition: candidate.definition,
        commit: candidate.commit,
        path: revision,
        catalog,
    })
}

pub(crate) fn activate_repository(
    cache_base: &Path,
    candidate: RepositoryCandidate,
) -> Result<RepositorySnapshot, String> {
    let root = repository_cache_root(cache_base, &candidate.definition.repository_key);
    let revision = repository_revision_path(
        cache_base,
        &candidate.definition.repository_key,
        &candidate.revision,
    );
    retain_revision(
        &revision,
        candidate.staged,
        &candidate.path,
        &candidate.revision,
    )?;
    write_current_pointer(&root, &candidate.revision, &candidate.validators)?;
    repository_snapshot(candidate.definition, candidate.revision, candidate.manifest)
}

pub(crate) fn discard_candidate(candidate: &SourceCandidate) {
    if candidate.staged && candidate.path.exists() {
        let _ = fs_retry::remove_dir_all(&candidate.path);
    }
}

pub(crate) fn discard_repository(candidate: &RepositoryCandidate) {
    if candidate.staged && candidate.path.exists() {
        let _ = fs_retry::remove_dir_all(&candidate.path);
    }
}

/// True when a refresh failed because the local cache for the source or
/// catalog is unreadable, so wiping the cache and refreshing again is the fix.
pub(crate) fn is_corrupt_cache_error(message: &str) -> bool {
    message.contains("revision pointer")
}

pub(crate) fn remove_source_cache(cache_base: &Path, source_key: &str) -> Result<(), String> {
    remove_cache_root(&source_cache_root(cache_base, source_key))
}

pub(crate) fn remove_repository_cache(
    cache_base: &Path,
    repository_key: &str,
) -> Result<(), String> {
    remove_cache_root(&repository_cache_root(cache_base, repository_key))
}

fn remove_cache_root(root: &Path) -> Result<(), String> {
    match fs_retry::remove_dir_all(root) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not remove {}: {error}", root.display())),
    }
}

fn retain_revision(
    revision: &Path,
    staged: bool,
    staged_path: &Path,
    token: &str,
) -> Result<(), String> {
    fs::create_dir_all(revision.parent().expect("revision parent"))
        .map_err(|error| format!("Could not create {}: {error}", revision.display()))?;
    if !staged {
        return Ok(());
    }
    if revision.exists() {
        fs_retry::remove_dir_all(staged_path)
            .map_err(|error| format!("Could not remove duplicate prepared snapshot: {error}"))?;
    } else {
        fs_retry::rename(staged_path, revision)
            .map_err(|error| format!("Could not retain revision {token}: {error}"))?;
    }
    Ok(())
}

fn configured_from_catalog(
    source_key: String,
    locator: Locator,
    repository_key: Option<String>,
    catalog: &ManifestCatalog,
) -> ConfiguredSource {
    ConfiguredSource {
        source_key,
        source_id: catalog.manifest.source().id.clone(),
        name: catalog.manifest.source().name.clone(),
        description: catalog.manifest.source().description.clone(),
        locator,
        repository_key,
    }
}

fn configured_from_repository_manifest(
    repository_key: String,
    locator: Locator,
    manifest: &RepositoryManifest,
) -> ConfiguredRepository {
    ConfiguredRepository {
        repository_key,
        repository_id: manifest.repository.id.clone(),
        name: manifest.repository.name.clone(),
        description: manifest.repository.description.clone(),
        locator,
    }
}

fn repository_snapshot(
    definition: ConfiguredRepository,
    revision: String,
    manifest: RepositoryManifest,
) -> Result<RepositorySnapshot, String> {
    let _ = manifest.canonical_sources()?;
    Ok(RepositorySnapshot {
        definition,
        revision,
        manifest,
    })
}

fn parse_sources_file(path: &Path, contents: &[u8]) -> Result<SourcesConfig, String> {
    let value = serde_json::from_slice::<serde_json::Value>(contents)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{} has no valid version.", path.display()))?;
    match version {
        version if version == u64::from(SOURCES_VERSION) => {
            let file = serde_json::from_value::<SourcesFile>(value)
                .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
            Ok(SourcesConfig {
                repositories: file.repositories,
                sources: file.sources,
            })
        }
        _ => Err(format!(
            "{} uses an unsupported source configuration version. Git sources are no longer supported; reset the development app data.",
            path.display()
        )),
    }
}

fn validate_sources_config(config: &SourcesConfig) -> Result<(), String> {
    validate_repositories(&config.repositories)?;
    validate_sources(&config.sources, &config.repositories)?;
    Ok(())
}

fn validate_repositories(repositories: &[ConfiguredRepository]) -> Result<(), String> {
    let mut keys = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut locators = BTreeSet::new();
    for repository in repositories {
        let locator = Locator::parse(repository.locator.url())?;
        if locator.repository_key() != repository.repository_key
            || locator.url() != repository.locator.url()
        {
            return Err(format!(
                "Source repository {} does not match its locator-derived repositoryKey.",
                repository.repository_id
            ));
        }
        validate_repository_id(&repository.repository_id)?;
        if repository.name.is_empty() || repository.description.is_empty() {
            return Err(format!(
                "Source repository {} has incomplete metadata.",
                repository.repository_id
            ));
        }
        if !keys.insert(repository.repository_key.as_str())
            || !ids.insert(repository.repository_id.as_str())
            || !locators.insert(locator.url().to_string())
        {
            return Err(
                "Source repository configuration contains a duplicate URL, repositoryKey, or repositoryId."
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn validate_sources(
    sources: &[ConfiguredSource],
    repositories: &[ConfiguredRepository],
) -> Result<(), String> {
    let repository_keys = repositories
        .iter()
        .map(|repository| repository.repository_key.as_str())
        .collect::<BTreeSet<_>>();
    let mut keys = BTreeSet::new();
    let mut ids = BTreeSet::new();
    let mut locators = BTreeSet::new();
    for source in sources {
        let locator = Locator::parse(source.locator.url())?;
        if locator.source_key() != source.source_key || locator.url() != source.locator.url() {
            return Err(format!(
                "Source {} does not match its locator-derived sourceKey.",
                source.source_id
            ));
        }
        if !valid_manifest_source_id(&source.source_id) {
            return Err(format!("Invalid configured sourceId: {}", source.source_id));
        }
        if source.name.is_empty() || source.description.is_empty() {
            return Err(format!(
                "Source {} has incomplete manifest metadata.",
                source.source_id
            ));
        }
        if let Some(repository_key) = &source.repository_key {
            if !repository_keys.contains(repository_key.as_str()) {
                // Provenance is display-only; a removed catalog must not invalidate the source.
            }
        }
        if !keys.insert(source.source_key.as_str())
            || !ids.insert(source.source_id.as_str())
            || !locators.insert(locator.url().to_string())
        {
            return Err(
                "Source configuration contains a duplicate URL, sourceKey, or sourceId."
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn validate_repository_id(value: &str) -> Result<(), String> {
    if validate_repository_id_shape(value) {
        Ok(())
    } else {
        Err(format!("Invalid configured repositoryId: {value}"))
    }
}

fn validate_repository_id_shape(value: &str) -> bool {
    (2..=32).contains(&value.len())
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_manifest_source_id(value: &str) -> bool {
    (2..=16).contains(&value.len())
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

impl CurrentPointer {
    fn validators(&self) -> ArtifactValidators {
        ArtifactValidators {
            etag: self.etag.clone(),
            last_modified: self.last_modified.clone(),
        }
    }
}

fn read_current_pointer(root: &Path) -> Result<Option<CurrentPointer>, String> {
    recover_current_pointer(root)?;
    let path = root.join(CURRENT_POINTER_FILE);
    let contents = match fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    };
    let value = serde_json::from_slice::<serde_json::Value>(&contents)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
    let version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            format!(
                "{} contains an invalid source revision pointer.",
                path.display()
            )
        })?;
    let pointer = if version == u64::from(CURRENT_POINTER_VERSION) {
        serde_json::from_value::<CurrentPointer>(value)
            .map_err(|error| format!("Could not parse {}: {error}", path.display()))?
    } else if version == u64::from(LEGACY_CURRENT_POINTER_VERSION) {
        let legacy = serde_json::from_value::<LegacyCurrentPointer>(value)
            .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
        if u64::from(legacy.version) != version {
            return Err(format!(
                "{} contains an invalid source revision pointer.",
                path.display()
            ));
        }
        CurrentPointer {
            version: CURRENT_POINTER_VERSION,
            revision: legacy.commit,
            etag: None,
            last_modified: None,
        }
    } else {
        return Err(format!(
            "{} contains an invalid source revision pointer.",
            path.display()
        ));
    };
    if !valid_commit_sha(&pointer.revision) {
        return Err(format!(
            "{} contains an invalid source revision pointer.",
            path.display()
        ));
    }
    Ok(Some(pointer))
}

fn write_current_pointer(
    root: &Path,
    revision: &str,
    validators: &ArtifactValidators,
) -> Result<(), String> {
    if !valid_commit_sha(revision) {
        return Err("Cannot activate an invalid source revision.".to_string());
    }
    let pointer = CurrentPointer {
        version: CURRENT_POINTER_VERSION,
        revision: revision.to_string(),
        etag: validators.etag.clone(),
        last_modified: validators.last_modified.clone(),
    };
    let mut contents = serde_json::to_vec_pretty(&pointer)
        .map_err(|error| format!("Could not serialize the source pointer: {error}"))?;
    contents.push(b'\n');
    fs::create_dir_all(root)
        .map_err(|error| format!("Could not create {}: {error}", root.display()))?;
    atomic_write_with_backup(
        root,
        &root.join(CURRENT_POINTER_FILE),
        &root.join(CURRENT_POINTER_BACKUP_FILE),
        "current-writing",
        &contents,
    )
}

fn recover_current_pointer(root: &Path) -> Result<(), String> {
    let path = root.join(CURRENT_POINTER_FILE);
    if path.exists() {
        return Ok(());
    }
    let backup = root.join(CURRENT_POINTER_BACKUP_FILE);
    match fs_retry::rename(&backup, &path) {
        Ok(()) => sync_directory(root),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not recover {}: {error}", path.display())),
    }
}

fn recover_sources_file(config_base: &Path) -> Result<(), String> {
    let path = sources_path(config_base);
    if path.exists() {
        return Ok(());
    }
    let backup = config_base.join(SOURCES_BACKUP_FILE);
    match fs_retry::rename(&backup, &path) {
        Ok(()) => sync_directory(config_base),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not recover {}: {error}", path.display())),
    }
}

fn atomic_write_with_backup(
    parent: &Path,
    path: &Path,
    backup: &Path,
    label: &str,
    contents: &[u8],
) -> Result<(), String> {
    let staging = temporary_path(parent, label);
    let result = write_new_file(&staging, contents)
        .and_then(|()| activate_with_backup(&staging, path, backup));
    if result.is_err() {
        let _ = fs_retry::remove_file(&staging);
    }
    result?;
    sync_directory(parent)
}

fn activate_with_backup(staging: &Path, path: &Path, backup: &Path) -> Result<(), String> {
    if path.exists() {
        if backup.exists() {
            fs_retry::remove_file(backup)
                .map_err(|error| format!("Could not remove {}: {error}", backup.display()))?;
        }
        fs_retry::rename(path, backup)
            .map_err(|error| format!("Could not stage {}: {error}", path.display()))?;
        if let Err(error) = fs_retry::rename(staging, path) {
            let restore = fs_retry::rename(backup, path);
            return match restore {
                Ok(()) => Err(format!("Could not activate {}: {error}", path.display())),
                Err(restore_error) => Err(format!(
                    "Could not activate {} ({error}) or restore it ({restore_error}).",
                    path.display()
                )),
            };
        }
        // The backup stays as the last good copy for a read that finds the
        // new file damaged.
    } else {
        fs_retry::rename(staging, path)
            .map_err(|error| format!("Could not activate {}: {error}", path.display()))?;
    }
    Ok(())
}

fn write_new_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
    file.write_all(contents)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceValidationError {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceValidationReport {
    pub source_id: String,
    pub valid_installs: usize,
    pub errors: Vec<SourceValidationError>,
}

pub fn validate_source(input: &str) -> Result<SourceValidationReport, String> {
    if input.starts_with("https://") {
        validate_remote_source(&Locator::parse(input)?)
    } else {
        require_strict_manifest(Path::new(input))?;
        Ok(report_catalog(&read_manifest_catalog(
            Path::new(input),
            "validation",
        )?))
    }
}

/// Publishing is strict even though the app reads fetched manifests
/// tolerantly: the manifest must parse with no unknown field or bad package.
fn require_strict_manifest(root: &Path) -> Result<(), String> {
    let path = root.join(SOURCE_MANIFEST_FILE);
    let bytes =
        fs::read(&path).map_err(|error| format!("Could not read {}: {error}", path.display()))?;
    SourceManifest::from_slice(&bytes).map(|_| ())
}

pub fn validate_source_locator(url: &str) -> Result<SourceValidationReport, String> {
    validate_remote_source(&Locator::parse(url)?)
}

pub fn validate_source_repository_locator(url: &str) -> Result<RepositoryValidationReport, String> {
    validate_remote_repository(&Locator::parse(url)?)
}

pub(crate) fn validate_remote_repository(
    locator: &Locator,
) -> Result<RepositoryValidationReport, String> {
    let cache = temporary_path(&std::env::temp_dir(), "agent-plugins-repository-validation");
    fs::create_dir(&cache)
        .map_err(|error| format!("Could not create {}: {error}", cache.display()))?;
    let result = prepare_new_repository(locator, &cache);
    let report = match result {
        Ok(candidate) => {
            let report = RepositoryManifest::from_path(&candidate.path)
                .and_then(|manifest| report_manifest(&manifest));
            discard_repository(&candidate);
            report
        }
        Err(error) => {
            let _ = fs_retry::remove_dir_all(&cache);
            return Err(error);
        }
    };
    let _ = fs_retry::remove_dir_all(&cache);
    report
}

fn validate_remote_source(locator: &Locator) -> Result<SourceValidationReport, String> {
    let cache = temporary_path(&std::env::temp_dir(), "agent-plugins-validation");
    fs::create_dir(&cache)
        .map_err(|error| format!("Could not create {}: {error}", cache.display()))?;
    let result = prepare_new_source(locator, &cache, None, None);
    let report = match result {
        Ok(candidate) => {
            let report = require_strict_manifest(&candidate.path)
                .map(|()| report_catalog(&candidate.catalog));
            discard_candidate(&candidate);
            report
        }
        Err(error) => {
            let _ = fs_retry::remove_dir_all(&cache);
            return Err(error);
        }
    };
    let _ = fs_retry::remove_dir_all(&cache);
    report
}

fn report_catalog(catalog: &ManifestCatalog) -> SourceValidationReport {
    SourceValidationReport {
        source_id: catalog.manifest.source().id.clone(),
        valid_installs: catalog.items.len(),
        errors: catalog
            .errors
            .iter()
            .map(|error| SourceValidationError {
                path: error.path.clone(),
                message: error.message.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_tree(source_id: &str) -> tempfile::TempDir {
        let tree = tempfile::tempdir().expect("source tree");
        let skill = tree.path().join("skills/review");
        fs::create_dir_all(&skill).expect("skill");
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: review\ndescription: Reviews code\n---\nBody\n",
        )
        .expect("skill");
        fs::write(
            tree.path().join("agent-plugins.json"),
            format!(
                r#"{{
                  "version": 2,
                  "source": {{ "id": "{source_id}", "name": "Test", "description": "Test source" }},
                  "packages": [{{
                    "id": "review",
                    "components": [{{"kind": "skill", "path": "skills/review"}}]
                  }}]
                }}"#
            ),
        )
        .expect("manifest");
        tree
    }

    #[test]
    fn validate_source_reports_a_local_catalog() {
        let tree = source_tree("acme");
        let report = validate_source(tree.path().to_str().expect("utf-8")).expect("report");
        assert_eq!(report.source_id, "acme");
        assert_eq!(report.valid_installs, 1);
        assert!(report.errors.is_empty());
    }

    #[test]
    fn candidate_activation_uses_immutable_revision_directories() {
        let tree = source_tree("acme");
        let cache = tempfile::tempdir().expect("cache");
        let locator =
            Locator::parse("https://nexus.example.com/repository/raw/sources/test-latest.zip")
                .expect("locator");
        let source_key = locator.source_key();
        let source_root = source_cache_root(cache.path(), &source_key);
        fs::create_dir_all(&source_root).expect("source root");
        let revision = "a".repeat(64);
        let copied = temporary_path(&source_root, "source-preparing");
        crate::sources::copy_directory(tree.path(), &copied).expect("copy source");
        let catalog = read_manifest_catalog(&copied, &source_key).expect("catalog");
        let definition = configured_from_catalog(source_key.clone(), locator, None, &catalog);
        let candidate = SourceCandidate {
            definition,
            commit: revision.clone(),
            path: copied,
            catalog,
            staged: true,
            validators: ArtifactValidators::default(),
        };
        let snapshot = activate_candidate(cache.path(), candidate).expect("activate");
        assert_eq!(snapshot.commit, revision);
        assert_eq!(
            snapshot.path,
            revision_path(cache.path(), &source_key, &revision)
        );
        assert!(snapshot.path.join("agent-plugins.json").is_file());
        assert_eq!(
            read_current_pointer(&source_root)
                .expect("pointer")
                .map(|pointer| pointer.revision),
            Some(revision)
        );
    }

    #[test]
    fn duplicate_manifest_namespaces_are_rejected_for_different_urls() {
        let sources = [
            ConfiguredSource::test_fixture(
                "acme",
                "https://nexus.example.com/repository/raw/sources/one-latest.zip",
            ),
            ConfiguredSource::test_fixture(
                "acme",
                "https://nexus.example.com/repository/raw/sources/two-latest.zip",
            ),
        ];
        assert!(validate_sources(&sources, &[])
            .expect_err("duplicate namespace")
            .contains("duplicate"));
    }

    #[test]
    fn source_id_changes_do_not_replace_the_current_revision() {
        let tree = source_tree("acme");
        let cache = tempfile::tempdir().expect("cache");
        let configured = ConfiguredSource::test_fixture(
            "different",
            "https://nexus.example.com/repository/raw/sources/acme-latest.zip",
        );
        let revision = "a".repeat(64);
        let copied = revision_path(cache.path(), &configured.source_key, &revision);
        fs::create_dir_all(copied.parent().expect("parent")).expect("revision parent");
        crate::sources::copy_directory(tree.path(), &copied).expect("copy");
        write_current_pointer(
            &source_cache_root(cache.path(), &configured.source_key),
            &revision,
            &ArtifactValidators::default(),
        )
        .expect("pointer");
        assert!(load_current(cache.path(), &configured)
            .expect_err("changed namespace")
            .contains("changed its manifest id"));
    }

    #[test]
    fn interrupted_current_pointer_replacement_recovers_the_previous_revision() {
        let cache = tempfile::tempdir().expect("cache");
        let source_root = source_cache_root(cache.path(), "source-test");
        let commit = "a".repeat(40);
        write_current_pointer(&source_root, &commit, &ArtifactValidators::default())
            .expect("pointer");
        fs_retry::rename(
            &source_root.join(CURRENT_POINTER_FILE),
            &source_root.join(CURRENT_POINTER_BACKUP_FILE),
        )
        .expect("simulate interrupted replacement");

        assert_eq!(
            read_current_pointer(&source_root)
                .expect("recovered pointer")
                .map(|pointer| pointer.revision),
            Some(commit)
        );
        assert!(source_root.join(CURRENT_POINTER_FILE).is_file());
    }

    #[test]
    fn a_damaged_sources_file_falls_back_to_the_previous_copy_then_a_rebuild() {
        let config = tempfile::tempdir().expect("config");
        let cache = tempfile::tempdir().expect("cache");
        let first = ConfiguredSource::test_fixture(
            "acme",
            "https://nexus.example.com/repository/raw/sources/acme-latest.zip",
        );
        let second = ConfiguredSource::test_fixture(
            "beta",
            "https://nexus.example.com/repository/raw/sources/beta-latest.zip",
        );
        for sources in [vec![first.clone()], vec![first.clone(), second]] {
            write_sources_config(
                config.path(),
                &SourcesConfig {
                    repositories: Vec::new(),
                    sources,
                },
            )
            .expect("write");
        }
        fs::write(sources_path(config.path()), b"{ damaged").expect("damage");
        let restored = read_sources_config_or_rebuild(config.path(), || unreachable!())
            .expect("previous copy");
        assert_eq!(restored.sources, vec![first.clone()]);

        fs::write(sources_path(config.path()), b"{ damaged").expect("damage");
        fs::write(config.path().join(SOURCES_BACKUP_FILE), b"also damaged").expect("damage");
        let installed = [("acme".to_string(), first.url().to_string())];
        let rebuilt = read_sources_config_or_rebuild(config.path(), || {
            rebuild_sources(cache.path(), None, &installed)
        })
        .expect("rebuilt");
        assert_eq!(rebuilt.sources.len(), 1);
        assert_eq!(rebuilt.sources[0].source_id, "acme");
        assert_eq!(rebuilt.sources[0].source_key, first.source_key);
        assert_eq!(read_sources_config(config.path()).expect("saved"), rebuilt);
        assert!(fs::read_dir(config.path())
            .expect("list")
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().to_string_lossy().contains(".corrupt-")));
    }

    #[test]
    fn legacy_git_sources_files_are_refused() {
        let config = tempfile::tempdir().expect("config");
        fs::write(
            sources_path(config.path()),
            r#"{
              "version": 4,
              "sources": [{
                "sourceKey": "source-41d130b3115ae73a",
                "sourceId": "skillbook",
                "name": "Skillbook",
                "description": "Jacob's canonical library of portable Agent Skills.",
                "url": "https://github.com/jacobragsdale/skillbook"
              }]
            }"#,
        )
        .expect("v4 file");
        assert!(read_sources_config(config.path())
            .expect_err("legacy")
            .contains("Git sources are no longer supported"));
    }

    #[test]
    fn removing_a_repository_leaves_opted_in_sources() {
        let config = tempfile::tempdir().expect("config");
        let cache = tempfile::tempdir().expect("cache");
        let locator = Locator::parse("https://nexus.example.com/repository/raw/catalogs/acme.json")
            .expect("locator");
        let mut source = ConfiguredSource::test_fixture(
            "review",
            "https://nexus.example.com/repository/raw/sources/review-latest.zip",
        );
        source.repository_key = Some(locator.repository_key());
        let repositories = vec![ConfiguredRepository {
            repository_key: locator.repository_key(),
            repository_id: "acme".to_string(),
            name: "Acme".to_string(),
            description: "Catalog".to_string(),
            locator,
        }];
        write_sources_config(
            config.path(),
            &SourcesConfig {
                repositories,
                sources: vec![source.clone()],
            },
        )
        .expect("write");
        let mut remaining = read_sources_config(config.path()).expect("read");
        remaining.repositories.clear();
        write_sources_config(config.path(), &remaining).expect("remove repo");
        remove_repository_cache(cache.path(), source.repository_key.as_deref().expect("key"))
            .expect("cache");
        let after = read_sources_config(config.path()).expect("after");
        assert!(after.repositories.is_empty());
        assert_eq!(after.sources.len(), 1);
        assert_eq!(after.sources[0].source_id, "review");
    }

    #[test]
    fn a_failed_write_leaves_no_staging_file() {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("sources.json");
        let backup = dir.path().join("sources.previous.json");
        fs::write(&path, "old").expect("current");
        fs::create_dir_all(backup.join("occupied")).expect("blocking backup");
        atomic_write_with_backup(dir.path(), &path, &backup, "sources-writing", b"new")
            .expect_err("backup cannot be replaced");
        assert_eq!(fs::read_to_string(&path).expect("current"), "old");
        let leftovers = fs::read_dir(dir.path())
            .expect("list")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("-writing-"))
            .count();
        assert_eq!(leftovers, 0);
    }
}
