//! Source manifests for portable packages.
//!
//! Publishing is strict: [`SourceManifest::from_slice`] refuses unknown fields
//! and any invalid package. The app reads fetched manifests with
//! [`SourceManifest::from_slice_tolerant`], which ignores fields a newer
//! publisher added and drops only the packages it cannot use.

use schemars::{generate::SchemaSettings, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const SOURCE_MANIFEST_FILE: &str = "agent-plugins.json";
pub const SOURCE_MANIFEST_VERSION: u8 = 2;
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(untagged)]
pub enum SourceManifest {
    V2(ManifestV2),
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct ManifestV2 {
    #[schemars(with = "i64", range(min = 2, max = 2))]
    pub version: u8,
    pub source: ManifestSource,
    #[schemars(length(min = 1))]
    pub packages: Vec<ManifestPackage>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct ManifestSource {
    #[schemars(
        length(min = 2, max = 16),
        regex(pattern = r"^[a-z](?:[a-z0-9]|-(?=[a-z0-9])){1,15}$")
    )]
    pub id: String,
    #[schemars(length(min = 1, max = 120))]
    pub name: String,
    #[schemars(length(min = 1, max = 1024))]
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct ManifestPackage {
    #[schemars(
        length(min = 1, max = 64),
        regex(pattern = r"^[a-z0-9](?:[a-z0-9]|-(?=[a-z0-9])){0,63}$")
    )]
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 120))]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 1024))]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ManifestComponent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conflicts_with: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
#[schemars(deny_unknown_fields)]
pub enum ManifestComponent {
    Skill {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        path: String,
    },
    McpServer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        path: String,
    },
}

impl ManifestComponent {
    pub(crate) fn id(&self) -> Option<&str> {
        match self {
            Self::Skill { id, .. } | Self::McpServer { id, .. } => id.as_deref(),
        }
    }

    pub(crate) fn path(&self) -> &str {
        match self {
            Self::Skill { path, .. } | Self::McpServer { path, .. } => path,
        }
    }
}

impl SourceManifest {
    /// Strict parse for publishing and the validators: unknown fields and any
    /// invalid package fail the whole manifest.
    pub fn from_slice(contents: &[u8]) -> Result<Self, String> {
        let (manifest, errors) = Self::parse(contents, true)?;
        match errors.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(manifest),
        }
    }

    /// Tolerant parse for a fetched manifest: unknown fields are ignored and
    /// an invalid package is dropped with one message per package.
    pub(crate) fn from_slice_tolerant(contents: &[u8]) -> Result<(Self, Vec<String>), String> {
        Self::parse(contents, false)
    }

    fn parse(contents: &[u8], strict: bool) -> Result<(Self, Vec<String>), String> {
        if contents.len() > MAX_MANIFEST_BYTES {
            return Err("agent-plugins.json is larger than the 1 MB limit.".to_string());
        }
        let value = serde_json::from_slice::<serde_json::Value>(contents)
            .map_err(|error| format!("Could not parse agent-plugins.json: {error}"))?;
        match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(1) => {
                return Err(
                    "agent-plugins.json version 1 generic file installs are no longer supported. Publish version 2 packages."
                        .to_string(),
                );
            }
            Some(2) => {}
            Some(version) => {
                return Err(format!(
                    "agent-plugins.json uses unsupported version {version}."
                ));
            }
            None => return Err("agent-plugins.json has no valid version.".to_string()),
        }
        let source = serde_json::from_value::<ManifestSource>(
            value.get("source").cloned().unwrap_or_default(),
        )
        .map_err(|error| format!("Could not parse agent-plugins.json: source: {error}"))?;
        validate_source_id(&source.id)?;
        validate_text(&source.name, "source.name", 1, 120)?;
        validate_text(&source.description, "source.description", 1, 1024)?;
        let entries = value
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .filter(|entries| !entries.is_empty())
            .ok_or_else(|| "agent-plugins.json does not publish any packages.".to_string())?;
        let mut packages = Vec::new();
        let mut errors = Vec::new();
        let mut ids = BTreeSet::new();
        for (index, entry) in entries.iter().enumerate() {
            let parsed = serde_json::from_value::<ManifestPackage>(entry.clone())
                .map_err(|error| format!("Package {}: {error}", index + 1))
                .and_then(|package| {
                    if strict {
                        reject_unknown_fields(entry, &package, &format!("packages[{index}]"))
                            .map_err(|error| {
                                format!("Could not parse agent-plugins.json: {error}")
                            })?;
                    }
                    validate_package(&package)?;
                    if !ids.insert(package.id.clone()) {
                        return Err(format!("Duplicate package id: {}", package.id));
                    }
                    Ok(package)
                });
            match parsed {
                Ok(package) => packages.push(package),
                Err(error) => errors.push(error),
            }
        }
        let manifest = Self::V2(ManifestV2 {
            version: SOURCE_MANIFEST_VERSION,
            source,
            packages,
        });
        if strict && errors.is_empty() {
            reject_unknown_fields(&value, &manifest, "")
                .map_err(|error| format!("Could not parse agent-plugins.json: {error}"))?;
        }
        Ok((manifest, errors))
    }

    pub fn validate(&self) -> Result<(), String> {
        let source = self.source();
        validate_source_id(&source.id)?;
        validate_text(&source.name, "source.name", 1, 120)?;
        validate_text(&source.description, "source.description", 1, 1024)?;
        let Self::V2(manifest) = self;
        if manifest.version != SOURCE_MANIFEST_VERSION || manifest.packages.is_empty() {
            return Err("agent-plugins.json does not publish any packages.".to_string());
        }
        let mut ids = BTreeSet::new();
        for package in &manifest.packages {
            validate_package(package)?;
            if !ids.insert(&package.id) {
                return Err(format!("Duplicate package id: {}", package.id));
            }
        }
        Ok(())
    }

    pub fn source(&self) -> &ManifestSource {
        let Self::V2(manifest) = self;
        &manifest.source
    }

    pub(crate) fn packages(&self) -> &[ManifestPackage] {
        let Self::V2(manifest) = self;
        &manifest.packages
    }
}

fn validate_package(package: &ManifestPackage) -> Result<(), String> {
    validate_package_id(&package.id, "package")?;
    if let Some(name) = &package.name {
        validate_text(name, "package.name", 1, 120)?;
    }
    if let Some(description) = &package.description {
        validate_text(description, "package.description", 1, 1024)?;
    }
    if package.components.is_empty() {
        return Err(format!(
            "Package {} must declare one or more skill or mcpServer components.",
            package.id
        ));
    }
    let mut component_ids = BTreeSet::new();
    for (index, component) in package.components.iter().enumerate() {
        let id = component.id().unwrap_or_else(|| {
            if package.components.len() == 1 {
                &package.id
            } else {
                ""
            }
        });
        if id.is_empty() {
            return Err(format!(
                "Package {} component {} needs an id because the package has several components.",
                package.id,
                index + 1
            ));
        }
        validate_package_id(id, "component")?;
        if !component_ids.insert(id) {
            return Err(format!(
                "Package {} has duplicate component id {id}.",
                package.id
            ));
        }
        if component.path().is_empty() {
            return Err(format!(
                "Package {} has an empty component path.",
                package.id
            ));
        }
    }
    for conflict in &package.conflicts_with {
        let valid = conflict
            .split_once('/')
            .filter(|(_, package_id)| !package_id.contains('/'))
            .is_some_and(|(source_id, package_id)| {
                validate_source_id(source_id).is_ok()
                    && validate_package_id(package_id, "conflicting package").is_ok()
            });
        if !valid {
            return Err(format!(
                "Package {} has invalid conflictsWith id {conflict:?}; use source-id/package-id.",
                package.id
            ));
        }
    }
    Ok(())
}

/// Fails when `input` has an object key that `parsed` (the typed document
/// serialized again) does not carry: a field this version does not know.
/// Explicit nulls and empty arrays count as known, because the typed document
/// omits them when it serializes.
pub(crate) fn reject_unknown_fields(
    input: &serde_json::Value,
    parsed: &impl Serialize,
    path: &str,
) -> Result<(), String> {
    fn walk(input: &serde_json::Value, known: &serde_json::Value, path: &str) -> Option<String> {
        use serde_json::Value;
        match (input, known) {
            (Value::Object(input), Value::Object(known)) => {
                input.iter().find_map(|(key, value)| {
                    let child = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    match known.get(key) {
                        Some(known) => walk(value, known, &child),
                        None if value.is_null() || value.as_array().is_some_and(Vec::is_empty) => {
                            None
                        }
                        None => Some(child),
                    }
                })
            }
            (Value::Array(input), Value::Array(known)) if input.len() == known.len() => input
                .iter()
                .zip(known)
                .enumerate()
                .find_map(|(index, (input, known))| {
                    walk(input, known, &format!("{path}[{index}]"))
                }),
            _ => None,
        }
    }
    let known = serde_json::to_value(parsed)
        .map_err(|error| format!("Could not check the document's fields: {error}"))?;
    match walk(input, &known, path) {
        Some(field) => Err(format!("unknown field `{field}`")),
        None => Ok(()),
    }
}

pub(crate) fn validate_package_id(value: &str, label: &str) -> Result<(), String> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('-')
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    valid
        .then_some(())
        .ok_or_else(|| format!("Invalid {label} id: {value}"))
}

pub fn source_manifest_schema_json() -> Result<String, String> {
    let generator = SchemaSettings::draft2020_12().into_generator();
    let schema = generator.into_root_schema_for::<SourceManifest>();
    let mut schema = serde_json::to_value(schema)
        .map_err(|error| format!("Could not prepare the source manifest schema: {error}"))?;
    if let Some(version) = schema.pointer_mut("/properties/version") {
        version
            .as_object_mut()
            .expect("the generated version schema is an object")
            .remove("format");
    }
    let mut output = serde_json::to_string_pretty(&schema)
        .map_err(|error| format!("Could not serialize the source manifest schema: {error}"))?;
    output.push('\n');
    Ok(output)
}

pub(crate) fn validate_source_id(value: &str) -> Result<(), String> {
    let valid = (2..=16).contains(&value.len())
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && !value.ends_with('-')
        && !value.contains("--")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(())
    } else {
        Err(format!(
            "Invalid source.id {value:?}; use 2-16 lowercase ASCII letters, digits, and single hyphens, beginning with a letter."
        ))
    }
}

pub(crate) fn validate_text(
    value: &str,
    label: &str,
    min: usize,
    max: usize,
) -> Result<(), String> {
    if (min..=max).contains(&value.chars().count()) {
        Ok(())
    } else {
        Err(format!("{label} must contain {min}-{max} characters."))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"{
      "version": 2,
      "source": {
        "id": "fiqit",
        "name": "Fiqit",
        "description": "Shared agent configuration."
      },
      "packages": [{
        "id": "review",
        "components": [{"kind": "skill", "path": "skills/review"}]
      }]
    }"#;

    #[test]
    fn generic_v1_installs_are_rejected() {
        let v1 = br#"{
          "version": 1,
          "source": {"id":"fiqit","name":"Fiqit","description":"Shared agent configuration."},
          "installs": [{"id":"review","source":"skills/review","destination":"~/.agents/skills/fiqit-review"}]
        }"#;
        assert!(SourceManifest::from_slice(v1)
            .expect_err("v1")
            .contains("version 1"));
    }

    #[test]
    fn portable_package_contract_is_accepted() {
        let manifest = SourceManifest::from_slice(
            br#"{
              "version": 2,
              "source": {"id":"acme","name":"Acme","description":"Shared configuration."},
              "packages": [{
                "id":"review",
                "components":[
                  {"kind":"skill","id":"review-skill","path":"skills/review"},
                  {"kind":"mcpServer","id":"review-db","path":"mcp/database.json"}
                ]
              }]
            }"#,
        )
        .expect("valid v2 manifest");
        assert_eq!(manifest.source().id, "acme");
        assert_eq!(manifest.packages().len(), 1);
    }

    #[test]
    fn instruction_sets_and_plugin_packages_are_rejected() {
        let instruction = br#"{
          "version": 2,
          "source": {"id":"acme","name":"Acme","description":"Shared configuration."},
          "packages": [{
            "id":"review",
            "components":[
              {"kind":"instructionSet","id":"review-rules","path":"rules/review.md","activation":"always"}
            ]
          }]
        }"#;
        let plugin = br#"{
          "version": 2,
          "source": {"id":"acme","name":"Acme","description":"Shared configuration."},
          "packages": [{
            "id":"data-tools",
            "format":"agent-plugin@1.0.0",
            "path":"plugins/data-tools"
          }]
        }"#;
        assert!(SourceManifest::from_slice(instruction)
            .expect_err("instruction set")
            .contains("unknown variant"));
        assert!(SourceManifest::from_slice(plugin)
            .expect_err("plugin package")
            .contains("unknown field"));
    }

    #[test]
    fn portable_package_conflicts_require_canonical_ids() {
        for conflict in ["missing-slash", "Bad/tools", "acme/", "acme/tools/extra"] {
            let manifest = format!(
                r#"{{
                  "version":2,
                  "source":{{"id":"acme","name":"Acme","description":"Test"}},
                  "packages":[{{
                    "id":"review",
                    "components":[{{"kind":"skill","path":"skills/review"}}],
                    "conflictsWith":["{conflict}"]
                  }}]
                }}"#
            );
            assert!(SourceManifest::from_slice(manifest.as_bytes())
                .expect_err("invalid conflict")
                .contains("invalid conflictsWith"));
        }
    }

    #[test]
    fn source_ids_unknown_fields_and_empty_packages_are_rejected() {
        let invalid_id = EXAMPLE.replace("\"fiqit\"", "\"Fiqit\"");
        assert!(SourceManifest::from_slice(invalid_id.as_bytes())
            .expect_err("invalid source id")
            .contains("Invalid source.id"));
        let unknown = EXAMPLE.replace("\"version\": 2,", "\"version\": 2, \"extra\": true,");
        assert!(SourceManifest::from_slice(unknown.as_bytes())
            .expect_err("unknown field")
            .contains("unknown field"));
        let empty = EXAMPLE.replace(
            r#"[{
        "id": "review",
        "components": [{"kind": "skill", "path": "skills/review"}]
      }]"#,
            "[]",
        );
        assert!(SourceManifest::from_slice(empty.as_bytes())
            .expect_err("empty packages")
            .contains("does not publish"));
    }

    #[test]
    fn fetched_manifests_skip_unknown_fields_and_bad_packages() {
        let manifest = br#"{
          "version": 2,
          "future": true,
          "source": {"id":"acme","name":"Acme","description":"Shared configuration.","logo":"x.png"},
          "packages": [
            {"id":"review","components":[{"kind":"skill","path":"skills/review","pinned":true}]},
            {"id":"rules","components":[{"kind":"instructionSet","path":"rules/review.md"}]}
          ]
        }"#;
        let (parsed, errors) = SourceManifest::from_slice_tolerant(manifest).expect("tolerant");
        assert_eq!(parsed.packages().len(), 1);
        assert_eq!(parsed.packages()[0].id, "review");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("Package 2"), "{errors:?}");
        let strict = SourceManifest::from_slice(manifest).expect_err("strict");
        assert!(strict.contains("unknown field"), "{strict}");
    }

    #[test]
    fn schema_matches_the_published_golden_file() {
        let generated = source_manifest_schema_json().expect("generated schema");
        let published = include_str!("../../schemas/v2/source-manifest.schema.json");
        let generated =
            serde_json::from_str::<serde_json::Value>(&generated).expect("generated schema JSON");
        let published =
            serde_json::from_str::<serde_json::Value>(published).expect("published schema JSON");
        assert_eq!(
            generated, published,
            "regenerate the source manifest schema"
        );
    }
}
