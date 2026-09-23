//! Versioned ownership ledger for logical installs, bindings, and physical resources.

use crate::digest::directory_digest;
use crate::fs_retry;
use crate::resource::{stable_id, CapabilityResult, StructuredFormat};
use crate::sources::{sync_directory, temporary_path};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path};

const LEDGER_FILE: &str = "installations.json";
const LEDGER_BACKUP_FILE: &str = "installations.json.previous";
const LEDGER_VERSION: u8 = 4;
const CORRUPT_PREFIX: &str = "installations.json.corrupt-";
/// Present after the ledger was restored from its one-transaction-old backup.
/// Files on disk are then the better record, so the next repair pass forgets
/// packages whose files are gone instead of putting them back.
const RESTORED_MARKER_FILE: &str = "installations.json.restored";
pub(crate) const NEWER_LEDGER_MESSAGE: &str = "A newer version of Agent Plugins manages the packages on this computer. Update Agent Plugins to install, update, or remove packages.";

pub(crate) struct LegacyPathRoots<'a> {
    pub(crate) home: &'a Path,
    pub(crate) config: &'a Path,
    pub(crate) data: &'a Path,
    pub(crate) local_data: &'a Path,
    pub(crate) cache: &'a Path,
}

// Record structs accept unknown fields so a ledger from a newer version still
// shows its packages, read-only.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OwnedPathKind {
    File,
    Directory,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OwnedPath {
    pub(crate) path: String,
    pub(crate) kind: OwnedPathKind,
    pub(crate) installed_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OwnedStructuredEntry {
    pub(crate) document_path: String,
    pub(crate) format: StructuredFormat,
    pub(crate) key_path: Vec<String>,
    pub(crate) value_digest: String,
    pub(crate) document_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OwnedTextBlock {
    pub(crate) document_path: String,
    pub(crate) marker_id: String,
    pub(crate) body_digest: String,
    pub(crate) document_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "resourceType")]
pub(crate) enum OwnedResource {
    Path(OwnedPath),
    StructuredEntry(OwnedStructuredEntry),
    TextBlock(OwnedTextBlock),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ResourceRecord {
    pub(crate) id: String,
    pub(crate) identity: String,
    pub(crate) desired_digest: String,
    pub(crate) owned: OwnedResource,
    pub(crate) consumer_binding_ids: Vec<String>,
    pub(crate) adapter_id: String,
    pub(crate) dialect_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BindingRecord {
    pub(crate) id: String,
    pub(crate) installation_id: String,
    pub(crate) component_id: String,
    pub(crate) target_id: String,
    pub(crate) dialect_id: String,
    pub(crate) scope: String,
    pub(crate) capability: CapabilityResult,
    pub(crate) resource_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InstallationRecord {
    pub(crate) source_key: String,
    pub(crate) source_url: String,
    pub(crate) source_id: String,
    pub(crate) local_id: String,
    pub(crate) commit: String,
    pub(crate) item_digest: String,
    pub(crate) name: String,
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) disable_model_invocation: bool,
    pub(crate) source: String,
    pub(crate) destination: OwnedPath,
    #[serde(default = "manifest_v1")]
    pub(crate) manifest_version: u8,
    #[serde(default = "legacy_component_kind")]
    pub(crate) component_kind: String,
    #[serde(default)]
    pub(crate) binding_ids: Vec<String>,
    #[serde(default)]
    pub(crate) selected_component_ids: Vec<String>,
    #[serde(default)]
    pub(crate) conflicts_with: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InstallationLedger {
    pub(crate) items: BTreeMap<String, InstallationRecord>,
    pub(crate) bindings: BTreeMap<String, BindingRecord>,
    pub(crate) resources: BTreeMap<String, ResourceRecord>,
    pub(crate) last_transaction_id: Option<String>,
    /// A newer app version wrote this ledger. It is shown but never changed.
    pub(crate) read_only: bool,
}

impl InstallationLedger {
    pub(crate) fn resource_by_identity(&self, identity: &str) -> Option<&ResourceRecord> {
        self.resources
            .values()
            .find(|resource| resource.identity == identity)
    }

    pub(crate) fn resource_by_identity_mut(
        &mut self,
        identity: &str,
    ) -> Option<&mut ResourceRecord> {
        self.resources
            .values_mut()
            .find(|resource| resource.identity == identity)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LedgerFileV4 {
    version: u8,
    items: BTreeMap<String, InstallationRecord>,
    bindings: BTreeMap<String, BindingRecord>,
    resources: BTreeMap<String, ResourceRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_transaction_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LedgerFileV3 {
    version: u8,
    items: BTreeMap<String, LegacyInstallationRecord>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LegacyInstallationRecord {
    source_key: String,
    source_url: String,
    source_id: String,
    local_id: String,
    commit: String,
    item_digest: String,
    name: String,
    description: String,
    #[serde(default)]
    disable_model_invocation: bool,
    source: String,
    destination: OwnedPath,
}

pub(crate) fn read(
    data_base: &Path,
    legacy_roots: LegacyPathRoots<'_>,
) -> Result<InstallationLedger, String> {
    recover(data_base)?;
    let path = data_base.join(LEDGER_FILE);
    // The live file first, then the last good copy it is replaced with when
    // it cannot be parsed.
    for _ in 0..2 {
        let contents = match fs::read(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(InstallationLedger::default());
            }
            Err(error) => {
                return Err(format!(
                    "Could not read the list of installed packages at {}: {}",
                    path.display(),
                    fs_retry::plain(&error)
                ))
            }
        };
        match parse(&path, &contents, &legacy_roots) {
            Ok((ledger, migrated)) => {
                if migrated {
                    persist_migration(data_base, &path, &ledger)?;
                }
                return Ok(ledger);
            }
            Err(error) => {
                let quarantine = quarantine(data_base, &path)?;
                eprintln!(
                    "Moved the unreadable {} aside to {}: {error}",
                    path.display(),
                    quarantine.display()
                );
                recover(data_base)?;
            }
        }
    }
    Ok(InstallationLedger::default())
}

fn parse(
    path: &Path,
    contents: &[u8],
    legacy_roots: &LegacyPathRoots<'_>,
) -> Result<(InstallationLedger, bool), String> {
    let mut value = serde_json::from_slice::<serde_json::Value>(contents)
        .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
    let mut version = value
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| format!("{} has no valid ledger version.", path.display()))?;
    if version == 2 {
        migrate_legacy_destinations(&mut value, legacy_roots)?;
        version = 3;
    }
    let (mut ledger, migrated) = match version {
        3 => {
            let legacy = serde_json::from_value::<LedgerFileV3>(value)
                .map_err(|error| format!("Could not parse {}: {error}", path.display()))?;
            (migrate_v3(legacy, legacy_roots)?, true)
        }
        version if version >= u64::from(LEDGER_VERSION) => {
            let mut ledger = InstallationLedger {
                items: records(path, &value, "items")?,
                bindings: records(path, &value, "bindings")?,
                resources: records(path, &value, "resources")?,
                last_transaction_id: value
                    .get("lastTransactionId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                read_only: false,
            };
            ledger.read_only = version > u64::from(LEDGER_VERSION);
            (ledger, false)
        }
        _ => {
            return Err(format!(
                "{} uses an unsupported ledger version.",
                path.display()
            ));
        }
    };
    prune(path, &mut ledger);
    Ok((ledger, migrated))
}

/// Reads one record map, skipping the records that do not parse so one bad
/// entry costs only that package.
fn records<T: serde::de::DeserializeOwned>(
    path: &Path,
    value: &serde_json::Value,
    key: &str,
) -> Result<BTreeMap<String, T>, String> {
    let map = value
        .get(key)
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| format!("{} has no {key} object.", path.display()))?;
    let mut records = BTreeMap::new();
    for (id, record) in map {
        match serde_json::from_value::<T>(record.clone()) {
            Ok(record) => {
                records.insert(id.clone(), record);
            }
            Err(error) => eprintln!(
                "Ignored the unreadable {key} entry {id} in {}: {error}",
                path.display()
            ),
        }
    }
    Ok(records)
}

fn persist_migration(
    data_base: &Path,
    path: &Path,
    ledger: &InstallationLedger,
) -> Result<(), String> {
    let migration_backup = data_base.join("installations.v3.json");
    if !migration_backup.exists() {
        fs_retry::copy(path, &migration_backup).map_err(|error| {
            format!(
                "Could not preserve the v3 ledger at {}: {}",
                migration_backup.display(),
                fs_retry::plain(&error)
            )
        })?;
    }
    write(data_base, ledger)?;
    let reread = fs::read(data_base.join(LEDGER_FILE))
        .map_err(|error| format!("Could not reread the migrated ledger: {error}"))?;
    let migrated_file = serde_json::from_slice::<LedgerFileV4>(&reread)
        .map_err(|error| format!("Could not verify the migrated ledger: {error}"))?;
    if migrated_file.version != LEDGER_VERSION {
        return Err("The migrated ledger did not retain version 4.".to_string());
    }
    Ok(())
}

/// Moves an unparsable ledger aside, keeping it for support instead of
/// deleting what may be the only record of an install.
fn quarantine(data_base: &Path, path: &Path) -> Result<std::path::PathBuf, String> {
    let target = quarantine_path(data_base, CORRUPT_PREFIX);
    fs_retry::rename(path, &target).map_err(|error| {
        format!(
            "The list of installed packages at {} is damaged and could not be moved aside: {}",
            path.display(),
            fs_retry::plain(&error)
        )
    })?;
    sync_directory(data_base)?;
    Ok(target)
}

/// `<directory>/<prefix><seconds>`, with a `-<n>` suffix when that name is
/// taken, so a second quarantine in the same second never replaces the first.
pub(crate) fn quarantine_path(directory: &Path, prefix: &str) -> std::path::PathBuf {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut target = directory.join(format!("{prefix}{seconds}"));
    let mut suffix = 1;
    while target.exists() {
        target = directory.join(format!("{prefix}{seconds}-{suffix}"));
        suffix += 1;
    }
    target
}

/// True once, after a read restored the ledger from its backup; clears it.
pub(crate) fn take_restored_marker(data_base: &Path) -> bool {
    fs_retry::remove_file(&data_base.join(RESTORED_MARKER_FILE)).is_ok()
}

/// Removes the ledger and its backup, backup first, so an interrupted wipe
/// cannot leave only the older copy for the next read to restore.
pub(crate) fn remove_files(data_base: &Path) -> Result<(), String> {
    for name in [LEDGER_BACKUP_FILE, LEDGER_FILE] {
        match fs_retry::remove_file(&data_base.join(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "Could not remove {}: {}",
                    data_base.join(name).display(),
                    fs_retry::plain(&error)
                ))
            }
        }
    }
    Ok(())
}

fn migrate_v3(
    file: LedgerFileV3,
    roots: &LegacyPathRoots<'_>,
) -> Result<InstallationLedger, String> {
    if file.version != 3 {
        return Err("The legacy ledger is not version 3.".to_string());
    }
    let mut ledger = InstallationLedger::default();
    for (installation_id, legacy) in file.items {
        let proven_plugin = legacy.destination.kind == OwnedPathKind::Directory
            && Path::new(&legacy.destination.path)
                .join("plugin.json")
                .is_file();
        let binding_id = stable_id("binding", &format!("{installation_id}:legacy-v1"));
        let identity = format!(
            "path:{}",
            normalize_path(Path::new(&legacy.destination.path))
        );
        let resource_id = stable_id("resource", &identity);
        ledger.resources.insert(
            resource_id.clone(),
            ResourceRecord {
                id: resource_id.clone(),
                identity,
                desired_digest: legacy.destination.installed_digest.clone(),
                owned: OwnedResource::Path(legacy.destination.clone()),
                consumer_binding_ids: vec![binding_id.clone()],
                adapter_id: "legacy-v1".to_string(),
                dialect_id: "manifest-v1".to_string(),
            },
        );
        ledger.bindings.insert(
            binding_id.clone(),
            BindingRecord {
                id: binding_id.clone(),
                installation_id: installation_id.clone(),
                component_id: legacy.local_id.clone(),
                target_id: "legacy-v1".to_string(),
                dialect_id: "manifest-v1".to_string(),
                scope: "explicit".to_string(),
                capability: CapabilityResult::Native,
                resource_ids: vec![resource_id],
            },
        );
        let mut binding_ids = vec![binding_id];
        if proven_plugin {
            for (target_id, path, dialect_id) in [
                (
                    "cursor",
                    roots.home.join(".cursor/plugins/local").join(&legacy.name),
                    "cursor-local-plugin-2026-08",
                ),
                (
                    "github-copilot",
                    roots
                        .home
                        .join(".copilot/installed-plugins/_direct")
                        .join(&legacy.name),
                    "copilot-direct-plugin-2026-08",
                ),
            ] {
                if !path.is_dir()
                    || path_digest(&path, OwnedPathKind::Directory).ok().as_deref()
                        != Some(&legacy.destination.installed_digest)
                {
                    continue;
                }
                let binding_id = stable_id(
                    "binding",
                    &format!("{installation_id}:{target_id}:legacy-plugin"),
                );
                let identity = format!("path:{}", normalize_path(&path));
                let resource_id = stable_id("resource", &identity);
                ledger.resources.insert(
                    resource_id.clone(),
                    ResourceRecord {
                        id: resource_id.clone(),
                        identity,
                        desired_digest: legacy.destination.installed_digest.clone(),
                        owned: OwnedResource::Path(OwnedPath {
                            path: path.display().to_string(),
                            kind: OwnedPathKind::Directory,
                            installed_digest: legacy.destination.installed_digest.clone(),
                        }),
                        consumer_binding_ids: vec![binding_id.clone()],
                        adapter_id: target_id.to_string(),
                        dialect_id: dialect_id.to_string(),
                    },
                );
                ledger.bindings.insert(
                    binding_id.clone(),
                    BindingRecord {
                        id: binding_id.clone(),
                        installation_id: installation_id.clone(),
                        component_id: legacy.local_id.clone(),
                        target_id: target_id.to_string(),
                        dialect_id: dialect_id.to_string(),
                        scope: "user".to_string(),
                        capability: CapabilityResult::Native,
                        resource_ids: vec![resource_id],
                    },
                );
                binding_ids.push(binding_id);
            }
        }
        ledger.items.insert(
            installation_id,
            InstallationRecord {
                source_key: legacy.source_key,
                source_url: legacy.source_url,
                source_id: legacy.source_id,
                local_id: legacy.local_id,
                commit: legacy.commit,
                item_digest: legacy.item_digest,
                name: legacy.name,
                description: legacy.description,
                disable_model_invocation: legacy.disable_model_invocation,
                source: legacy.source,
                destination: legacy.destination,
                manifest_version: 1,
                component_kind: if proven_plugin {
                    "agentPlugin".to_string()
                } else {
                    legacy_component_kind()
                },
                binding_ids,
                selected_component_ids: Vec::new(),
                conflicts_with: Vec::new(),
            },
        );
    }
    Ok(ledger)
}

fn valid_item(id: &str, record: &InstallationRecord) -> bool {
    id == format!("{}/{}", record.source_id, record.local_id)
        && !record.source_key.is_empty()
        && !record.source_url.is_empty()
        && !record.commit.is_empty()
        && !record.name.is_empty()
        && !record.description.is_empty()
        && !record.source.is_empty()
        && !record.destination.path.is_empty()
        && Path::new(&record.destination.path).is_absolute()
        && valid_digest(&record.item_digest)
        && valid_digest(&record.destination.installed_digest)
        && record.manifest_version != 0
}

/// Drops the records that are invalid or point at nothing, keeping the rest.
/// References to a dropped record are removed rather than dropping their
/// owner, so one bad resource does not take its whole package with it.
fn prune(path: &Path, ledger: &mut InstallationLedger) {
    let mut dropped = Vec::new();
    ledger.items.retain(|id, record| {
        let keep = valid_item(id, record);
        if !keep {
            dropped.push(format!("package {id}"));
        }
        keep
    });
    let mut identities = BTreeSet::new();
    ledger.resources.retain(|id, resource| {
        let keep = id == &resource.id
            && !resource.identity.is_empty()
            && valid_digest(&resource.desired_digest)
            && identities.insert(resource.identity.clone());
        if !keep {
            dropped.push(format!("resource {id}"));
        }
        keep
    });
    loop {
        let before = (ledger.bindings.len(), ledger.resources.len());
        let items = &ledger.items;
        ledger.bindings.retain(|id, binding| {
            let keep = id == &binding.id && items.contains_key(&binding.installation_id);
            if !keep {
                dropped.push(format!("binding {id}"));
            }
            keep
        });
        let bindings = &ledger.bindings;
        ledger.resources.retain(|id, resource| {
            resource
                .consumer_binding_ids
                .retain(|binding_id| bindings.contains_key(binding_id));
            let keep = !resource.consumer_binding_ids.is_empty();
            if !keep {
                dropped.push(format!("resource {id}"));
            }
            keep
        });
        let resources = &ledger.resources;
        for binding in ledger.bindings.values_mut() {
            binding
                .resource_ids
                .retain(|resource_id| resources.contains_key(resource_id));
        }
        if before == (ledger.bindings.len(), ledger.resources.len()) {
            break;
        }
    }
    let bindings = &ledger.bindings;
    for record in ledger.items.values_mut() {
        record
            .binding_ids
            .retain(|binding_id| bindings.contains_key(binding_id));
    }
    if !dropped.is_empty() {
        eprintln!(
            "Ignored invalid entries in {}: {}",
            path.display(),
            dropped.join(", ")
        );
    }
}

fn migrate_legacy_destinations(
    value: &mut serde_json::Value,
    roots: &LegacyPathRoots<'_>,
) -> Result<(), String> {
    let items = value
        .get_mut("items")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or_else(|| "The legacy installation ledger has no valid items object.".to_string())?;
    for (id, record) in items {
        let destination = record
            .get_mut("destination")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or_else(|| {
                format!("The legacy installation record for {id} has no destination.")
            })?;
        let anchor = destination
            .remove("anchor")
            .and_then(|value| value.as_str().map(str::to_string))
            .ok_or_else(|| format!("The legacy installation record for {id} has no anchor."))?;
        let relative = destination
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("The legacy installation record for {id} has no path."))?;
        let relative = Path::new(relative);
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(format!(
                "The legacy installation record for {id} has an invalid path."
            ));
        }
        let root = match anchor.as_str() {
            "home" => roots.home,
            "config" => roots.config,
            "data" => roots.data,
            "localData" => roots.local_data,
            "cache" => roots.cache,
            _ => {
                return Err(format!(
                    "The legacy installation record for {id} has an unknown anchor."
                ));
            }
        };
        destination.insert(
            "path".to_string(),
            serde_json::Value::String(root.join(relative).display().to_string()),
        );
    }
    value["version"] = serde_json::Value::from(3);
    Ok(())
}

pub(crate) fn write(data_base: &Path, ledger: &InstallationLedger) -> Result<(), String> {
    if ledger.read_only {
        return Err(NEWER_LEDGER_MESSAGE.to_string());
    }
    fs_retry::create_dir_all(data_base).map_err(|error| {
        format!(
            "Could not create {}: {}",
            data_base.display(),
            fs_retry::plain(&error)
        )
    })?;
    recover(data_base)?;
    let file = LedgerFileV4 {
        version: LEDGER_VERSION,
        items: ledger.items.clone(),
        bindings: ledger.bindings.clone(),
        resources: ledger.resources.clone(),
        last_transaction_id: ledger.last_transaction_id.clone(),
    };
    let mut contents = serde_json::to_vec_pretty(&file)
        .map_err(|error| format!("Could not serialize the installation ledger: {error}"))?;
    contents.push(b'\n');
    atomic_write(
        data_base,
        &data_base.join(LEDGER_FILE),
        &data_base.join(LEDGER_BACKUP_FILE),
        &contents,
    )
}

pub(crate) fn path_digest(path: &Path, kind: OwnedPathKind) -> Result<String, String> {
    match kind {
        OwnedPathKind::Directory => directory_digest(path),
        OwnedPathKind::File => {
            let bytes = fs::read(path)
                .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
            Ok(hex_digest(Sha256::digest(bytes)))
        }
    }
}

pub(crate) fn bytes_digest(bytes: &[u8]) -> String {
    hex_digest(Sha256::digest(bytes))
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn atomic_write(
    directory: &Path,
    path: &Path,
    backup: &Path,
    contents: &[u8],
) -> Result<(), String> {
    let staging = temporary_path(directory, "installations-writing");
    let staged = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .and_then(|mut file| file.write_all(contents).and_then(|()| file.sync_all()));
    if let Err(error) = staged {
        let _ = fs::remove_file(&staging);
        return Err(format!(
            "Could not save the list of installed packages in {}: {}",
            directory.display(),
            fs_retry::plain(&error)
        ));
    }
    if path.exists() {
        // The replaced file stays behind as the last good copy that a damaged
        // ledger falls back to.
        if let Err(error) = fs_retry::rename(path, backup) {
            let _ = fs::remove_file(&staging);
            return Err(format!(
                "Could not stage {}: {}",
                path.display(),
                fs_retry::plain(&error)
            ));
        }
        if let Err(error) = sync_directory(directory) {
            eprintln!("{error}");
        }
        if let Err(error) = fs_retry::rename(&staging, path) {
            let _ = fs::remove_file(&staging);
            let restore = fs_retry::rename(backup, path);
            return match restore {
                Ok(()) => Err(format!(
                    "Could not save {}: {}",
                    path.display(),
                    fs_retry::plain(&error)
                )),
                Err(restore_error) => Err(format!(
                    "Could not save {} ({}) or restore it ({restore_error}).",
                    path.display(),
                    fs_retry::plain(&error)
                )),
            };
        }
    } else if let Err(error) = fs_retry::rename(&staging, path) {
        let _ = fs::remove_file(&staging);
        return Err(format!(
            "Could not save {}: {}",
            path.display(),
            fs_retry::plain(&error)
        ));
    }
    // The ledger is committed once the rename succeeds. A failed flush now is
    // a warning, never an error that would roll back files the ledger owns.
    if let Err(error) = sync_directory(directory) {
        eprintln!("The ledger was saved, but {error}");
    }
    Ok(())
}

fn recover(data_base: &Path) -> Result<(), String> {
    let path = data_base.join(LEDGER_FILE);
    if path.exists() {
        return Ok(());
    }
    let backup = data_base.join(LEDGER_BACKUP_FILE);
    match fs_retry::rename(&backup, &path) {
        Ok(()) => {
            let _ = fs::write(data_base.join(RESTORED_MARKER_FILE), b"");
            sync_directory(data_base)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not recover {}: {error}", path.display())),
    }
}

fn manifest_v1() -> u8 {
    1
}

fn legacy_component_kind() -> String {
    "legacyFileTree".to_string()
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
        .to_lowercase()
}

fn hex_digest(digest: impl AsRef<[u8]>) -> String {
    let mut output = String::with_capacity(64);
    for byte in digest.as_ref() {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_plugin(path: &Path, divergent: bool) {
        fs::create_dir_all(path).expect("plugin directory");
        fs::write(
            path.join("plugin.json"),
            r#"{
              "$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
              "name":"acme-tools",
              "version":"1.0.0",
              "description":"Acme tools"
            }"#,
        )
        .expect("plugin manifest");
        if divergent {
            fs::write(path.join("local-change.txt"), "changed").expect("local change");
        }
    }

    fn record(path: &Path) -> InstallationRecord {
        let binding_id = stable_id("binding", "acme/review:legacy-v1");
        InstallationRecord {
            source_key: "source-key".to_string(),
            source_url: "https://example.com/source".to_string(),
            source_id: "acme".to_string(),
            local_id: "review".to_string(),
            commit: "a".repeat(40),
            item_digest: "b".repeat(64),
            name: "acme-review".to_string(),
            description: "Review code.".to_string(),
            disable_model_invocation: false,
            source: "skills/review".to_string(),
            destination: OwnedPath {
                path: path.display().to_string(),
                kind: OwnedPathKind::Directory,
                installed_digest: "c".repeat(64),
            },
            manifest_version: 1,
            component_kind: "legacyFileTree".to_string(),
            binding_ids: vec![binding_id],
            selected_component_ids: Vec::new(),
            conflicts_with: Vec::new(),
        }
    }

    fn roots(root: &Path) -> LegacyPathRoots<'_> {
        LegacyPathRoots {
            home: root,
            config: root,
            data: root,
            local_data: root,
            cache: root,
        }
    }

    fn one_item_ledger(root: &Path, local_id: &str) -> InstallationLedger {
        let mut ledger = InstallationLedger::default();
        let mut item = record(&root.join(local_id));
        item.local_id = local_id.to_string();
        item.binding_ids.clear();
        ledger.items.insert(format!("acme/{local_id}"), item);
        ledger
    }

    #[test]
    fn write_keeps_the_previous_ledger_and_a_damaged_one_falls_back_to_it() {
        let root = tempfile::tempdir().expect("tempdir");
        write(root.path(), &one_item_ledger(root.path(), "first")).expect("first");
        write(root.path(), &one_item_ledger(root.path(), "second")).expect("second");
        assert!(root.path().join(LEDGER_BACKUP_FILE).exists());

        fs::write(root.path().join(LEDGER_FILE), "{\"version\":4,").expect("damage");
        let recovered = read(root.path(), roots(root.path())).expect("read");
        assert!(recovered.items.contains_key("acme/first"));
        assert!(fs::read_dir(root.path())
            .expect("dir")
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with(CORRUPT_PREFIX)));
    }

    #[test]
    fn invalid_records_are_dropped_and_the_rest_are_kept() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut ledger = one_item_ledger(root.path(), "good");
        let mut bad = record(&root.path().join("bad"));
        bad.local_id = "bad".to_string();
        bad.description.clear();
        bad.binding_ids.clear();
        ledger.items.insert("acme/bad".to_string(), bad);
        write(root.path(), &ledger).expect("write");
        let reread = read(root.path(), roots(root.path())).expect("read");
        assert_eq!(reread.items.keys().collect::<Vec<_>>(), vec!["acme/good"]);
    }

    #[test]
    fn a_newer_ledger_is_read_only() {
        let root = tempfile::tempdir().expect("tempdir");
        write(root.path(), &one_item_ledger(root.path(), "first")).expect("write");
        let path = root.path().join(LEDGER_FILE);
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path).expect("read"))
                .expect("json");
        value["version"] = serde_json::json!(LEDGER_VERSION + 1);
        value["items"]["acme/first"]["futureField"] = serde_json::json!(true);
        fs::write(&path, serde_json::to_vec(&value).expect("json")).expect("newer");
        let ledger = read(root.path(), roots(root.path())).expect("read");
        assert!(ledger.read_only);
        assert!(ledger.items.contains_key("acme/first"));
        assert_eq!(
            write(root.path(), &ledger).expect_err("refused"),
            NEWER_LEDGER_MESSAGE
        );
    }

    #[test]
    fn ledger_round_trips_atomically() {
        let root = tempfile::tempdir().expect("tempdir");
        let mut ledger = InstallationLedger::default();
        let installation_id = "acme/review".to_string();
        let record = record(&root.path().join("acme-review"));
        let binding_id = record.binding_ids[0].clone();
        let identity = format!(
            "path:{}",
            normalize_path(Path::new(&record.destination.path))
        );
        let resource_id = stable_id("resource", &identity);
        ledger.items.insert(installation_id.clone(), record.clone());
        ledger.bindings.insert(
            binding_id.clone(),
            BindingRecord {
                id: binding_id.clone(),
                installation_id,
                component_id: "review".to_string(),
                target_id: "legacy-v1".to_string(),
                dialect_id: "manifest-v1".to_string(),
                scope: "explicit".to_string(),
                capability: CapabilityResult::Native,
                resource_ids: vec![resource_id.clone()],
            },
        );
        ledger.resources.insert(
            resource_id.clone(),
            ResourceRecord {
                id: resource_id,
                identity,
                desired_digest: record.destination.installed_digest.clone(),
                owned: OwnedResource::Path(record.destination),
                consumer_binding_ids: vec![binding_id],
                adapter_id: "legacy-v1".to_string(),
                dialect_id: "manifest-v1".to_string(),
            },
        );
        write(root.path(), &ledger).expect("write");
        let roots = LegacyPathRoots {
            home: root.path(),
            config: root.path(),
            data: root.path(),
            local_data: root.path(),
            cache: root.path(),
        };
        let reloaded = read(root.path(), roots).expect("read");
        assert_eq!(reloaded.items, ledger.items);
        assert_eq!(reloaded.bindings, ledger.bindings);
        assert_eq!(reloaded.resources, ledger.resources);
    }

    #[test]
    fn v3_migration_is_repeatable_and_adopts_only_proven_identical_plugin_copies() {
        let root = tempfile::tempdir().expect("tempdir");
        let data_base = root.path().join("data");
        let home = root.path().join("home");
        let primary = home.join(".agents/plugins/acme-tools");
        let cursor = home.join(".cursor/plugins/local/acme-tools");
        let copilot = home.join(".copilot/installed-plugins/_direct/acme-tools");
        write_plugin(&primary, false);
        write_plugin(&cursor, false);
        write_plugin(&copilot, true);
        let digest = directory_digest(&primary).expect("digest");
        fs::create_dir_all(&data_base).expect("data");
        let original = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 3,
            "items": {
                "acme/tools": {
                    "sourceKey": "source-key",
                    "sourceUrl": "https://example.com/acme.git",
                    "sourceId": "acme",
                    "localId": "tools",
                    "commit": "a".repeat(40),
                    "itemDigest": "b".repeat(64),
                    "name": "acme-tools",
                    "description": "Acme tools.",
                    "disableModelInvocation": false,
                    "source": "plugins/tools",
                    "destination": {
                        "path": primary.display().to_string(),
                        "kind": "directory",
                        "installedDigest": digest
                    }
                }
            }
        }))
        .expect("legacy ledger");
        fs::write(data_base.join(LEDGER_FILE), &original).expect("write legacy ledger");
        let roots = LegacyPathRoots {
            home: &home,
            config: root.path(),
            data: root.path(),
            local_data: root.path(),
            cache: root.path(),
        };

        let migrated = read(&data_base, roots).expect("migrate");
        assert_eq!(migrated.items.len(), 1);
        assert_eq!(migrated.bindings.len(), 2);
        assert_eq!(migrated.resources.len(), 2);
        assert!(migrated
            .resources
            .values()
            .any(|resource| resource.identity.contains(".cursor/plugins/local")));
        assert!(!migrated
            .resources
            .values()
            .any(|resource| resource.identity.contains(".copilot/installed-plugins")));
        assert_eq!(
            fs::read(data_base.join("installations.v3.json")).expect("migration backup"),
            original
        );
        let live = fs::read(data_base.join(LEDGER_FILE)).expect("v4 ledger");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&live).expect("v4 JSON")["version"],
            4
        );

        let roots = LegacyPathRoots {
            home: &home,
            config: root.path(),
            data: root.path(),
            local_data: root.path(),
            cache: root.path(),
        };
        let repeated = read(&data_base, roots).expect("repeat read");
        assert_eq!(repeated.items, migrated.items);
        assert_eq!(repeated.bindings, migrated.bindings);
        assert_eq!(repeated.resources, migrated.resources);

        fs::rename(
            data_base.join(LEDGER_FILE),
            data_base.join(LEDGER_BACKUP_FILE),
        )
        .expect("simulate interrupted activation");
        let roots = LegacyPathRoots {
            home: &home,
            config: root.path(),
            data: root.path(),
            local_data: root.path(),
            cache: root.path(),
        };
        let recovered = read(&data_base, roots).expect("recover backup");
        assert_eq!(recovered.items, migrated.items);
    }

    #[test]
    fn quarantine_names_never_collide_and_a_wipe_removes_the_backup_first() {
        let dir = tempfile::tempdir().expect("dir");
        let first = quarantine_path(dir.path(), "x.corrupt-");
        fs::write(&first, "a").expect("first");
        let second = quarantine_path(dir.path(), "x.corrupt-");
        assert_ne!(first, second);

        fs::write(dir.path().join(LEDGER_BACKUP_FILE), "{}").expect("backup");
        remove_files(dir.path()).expect("remove");
        assert!(!dir.path().join(LEDGER_BACKUP_FILE).exists());
        assert!(!take_restored_marker(dir.path()));
    }
}
