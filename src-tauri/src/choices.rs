//! The person's per-package choices beyond skill invocation: packages whose
//! updates they hold, and apps a component should stay out of.

use crate::fs_retry;
use crate::paths::SystemPaths;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;

const CHOICES_FILE: &str = "package-choices.json";

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
    let path = paths.app_data().join(CHOICES_FILE);
    match fs::read(&path) {
        Ok(contents) => serde_json::from_slice(&contents)
            .map_err(|error| format!("Could not parse {}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Choices::default()),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
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
    fs_retry::replace_file(&path, &contents)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

fn key(item_id: &str, component_id: &str) -> String {
    format!("{item_id}:{component_id}")
}
