//! The person's per-package choices beyond skill invocation: packages whose
//! updates they hold, and apps a component should stay out of.

use crate::fs_retry;
use crate::paths::SystemPaths;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;

const CHOICES_FILE: &str = "package-choices.json";
/// The choices the last save replaced, for a damaged file to fall back to.
const CHOICES_BACKUP_FILE: &str = "package-choices.json.previous";

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Choices {
    /// Canonical package IDs that do not update in the background.
    #[serde(default)]
    pub(crate) held: BTreeSet<String>,
    /// Target IDs a component stays out of, keyed `<item id>:<component id>`.
    #[serde(default)]
    pub(crate) excluded_apps: BTreeMap<String, BTreeSet<String>>,
}

impl Choices {
    pub(crate) fn excluded(&self, item_id: &str, component_id: &str) -> Vec<String> {
        self.excluded_apps
            .get(&key(item_id, component_id))
            .map(|apps| apps.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn is_excluded(&self, item_id: &str, component_id: &str, target_id: &str) -> bool {
        self.excluded_apps
            .get(&key(item_id, component_id))
            .is_some_and(|apps| apps.contains(target_id))
    }

    pub(crate) fn set_excluded(
        &mut self,
        item_id: &str,
        component_id: &str,
        apps: BTreeSet<String>,
    ) {
        if apps.is_empty() {
            self.excluded_apps.remove(&key(item_id, component_id));
        } else {
            self.excluded_apps.insert(key(item_id, component_id), apps);
        }
    }
}

pub(crate) fn read(paths: &SystemPaths) -> Result<Choices, String> {
    let directory = paths.app_data();
    let path = directory.join(CHOICES_FILE);
    let contents = match fs::read(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Choices::default()),
        Err(error) => return Err(format!("Could not read {}: {error}", path.display())),
    };
    let error = match serde_json::from_slice::<Choices>(&contents) {
        Ok(choices) => return Ok(choices),
        Err(error) => error,
    };
    // Like the ledger: keep the damaged file for support and go back to the
    // copy the last save replaced, so holds and exclusions survive.
    let aside = crate::ledger::quarantine_path(&directory, &format!("{CHOICES_FILE}.corrupt-"));
    fs_retry::rename(&path, &aside).map_err(|rename_error| {
        format!(
            "Could not parse {} ({error}), and could not move it aside: {}",
            path.display(),
            fs_retry::plain(&rename_error)
        )
    })?;
    let previous = fs::read(directory.join(CHOICES_BACKUP_FILE))
        .ok()
        .and_then(|contents| serde_json::from_slice::<Choices>(&contents).ok())
        .unwrap_or_default();
    eprintln!(
        "Agent Plugins set aside its damaged package choices as {} and went back to the last saved ones.",
        aside.display()
    );
    write(paths, &previous)?;
    Ok(previous)
}

/// For planning and display: an unusable file means no choices.
pub(crate) fn read_or_default(paths: &SystemPaths) -> Choices {
    read(paths).unwrap_or_else(|error| {
        eprintln!("Agent Plugins ignored its saved package choices: {error}");
        Choices::default()
    })
}

pub(crate) fn write(paths: &SystemPaths, choices: &Choices) -> Result<(), String> {
    let directory = paths.app_data();
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    let path = directory.join(CHOICES_FILE);
    let contents = serde_json::to_vec_pretty(choices)
        .map_err(|error| format!("Could not serialize package choices: {error}"))?;
    if path.exists() {
        let _ = fs_retry::copy(&path, &directory.join(CHOICES_BACKUP_FILE));
    }
    fs_retry::replace_file(&path, &contents)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

fn key(item_id: &str, component_id: &str) -> String {
    format!("{item_id}:{component_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_damaged_choices_file_is_set_aside_and_the_last_saved_choices_kept() {
        let root = tempfile::tempdir().expect("root");
        let paths = SystemPaths {
            home: root.path().join("home"),
            config: root.path().join("config"),
            data: root.path().join("data"),
            local_data: root.path().join("local-data"),
            cache: root.path().join("cache"),
        };
        let mut choices = Choices::default();
        choices.held.insert("acme/one".to_string());
        write(&paths, &choices).expect("first");
        choices.held.insert("acme/two".to_string());
        write(&paths, &choices).expect("second");
        fs::write(paths.app_data().join(CHOICES_FILE), "{\"held\":[").expect("damage");

        let read_back = read(&paths).expect("read");

        assert!(
            read_back.held.contains("acme/one"),
            "the copy the last save replaced"
        );
        assert_eq!(
            read(&paths).expect("again"),
            read_back,
            "restored on disk, so later reads agree"
        );
        assert!(fs::read_dir(paths.app_data())
            .expect("dir")
            .flatten()
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("package-choices.json.corrupt-")));
    }
}
