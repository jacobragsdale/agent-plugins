//! The person's choice of whether a skill runs only when asked. The source's
//! `disable-model-invocation` is the default; only choices that differ from
//! it are kept, keyed `<item id>:<component id>`.

use crate::catalog::CatalogComponent;
use crate::fs_retry;
use crate::paths::SystemPaths;
use std::collections::BTreeMap;
use std::fs;
use std::io;

const OVERRIDES_FILE: &str = "invocation-overrides.json";

pub(crate) type Overrides = BTreeMap<String, bool>;

pub(crate) fn read(paths: &SystemPaths) -> Result<Overrides, String> {
    let path = paths.app_data().join(OVERRIDES_FILE);
    match fs::read(&path) {
        Ok(contents) => serde_json::from_slice(&contents)
            .map_err(|error| format!("Could not parse {}: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Overrides::new()),
        Err(error) => Err(format!("Could not read {}: {error}", path.display())),
    }
}

/// For planning and display: an unusable file falls back to the source's values.
pub(crate) fn read_or_default(paths: &SystemPaths) -> Overrides {
    read(paths).unwrap_or_else(|error| {
        eprintln!("Agent Plugins ignored its skill invocation choices: {error}");
        Overrides::new()
    })
}

pub(crate) fn write(paths: &SystemPaths, overrides: &Overrides) -> Result<(), String> {
    let directory = paths.app_data();
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;
    let path = directory.join(OVERRIDES_FILE);
    let contents = serde_json::to_vec_pretty(overrides)
        .map_err(|error| format!("Could not serialize skill invocation choices: {error}"))?;
    fs_retry::replace_file(&path, &contents)
        .map_err(|error| format!("Could not write {}: {error}", path.display()))
}

/// Records `manual` for a component, or forgets the choice when it matches the source.
pub(crate) fn set(
    overrides: &mut Overrides,
    item_id: &str,
    component: &CatalogComponent,
    manual: bool,
) {
    let key = key(item_id, &component.id);
    if manual == component.disable_model_invocation {
        overrides.remove(&key);
    } else {
        overrides.insert(key, manual);
    }
}

/// The value to write into SKILL.md, only when it differs from the source's.
pub(crate) fn override_for(
    overrides: &Overrides,
    item_id: &str,
    component: &CatalogComponent,
) -> Option<bool> {
    overrides
        .get(&key(item_id, &component.id))
        .copied()
        .filter(|manual| *manual != component.disable_model_invocation)
}

pub(crate) fn effective(
    overrides: &Overrides,
    item_id: &str,
    component: &CatalogComponent,
) -> bool {
    override_for(overrides, item_id, component).unwrap_or(component.disable_model_invocation)
}

fn key(item_id: &str, component_id: &str) -> String {
    format!("{item_id}:{component_id}")
}
