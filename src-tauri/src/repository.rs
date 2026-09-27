//! Source-repository catalog documents.
//!
//! Publishing is strict ([`RepositoryManifest::from_slice`]); the app reads a
//! fetched catalog tolerantly, ignoring unknown fields and dropping only the
//! listings it cannot use.

use crate::locator::Locator;
use crate::manifest::{reject_unknown_fields, validate_source_id, validate_text};
use schemars::{generate::SchemaSettings, JsonSchema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const REPOSITORY_MANIFEST_FILE: &str = "agent-plugins-repository.json";
pub const REPOSITORY_MANIFEST_VERSION: u8 = 1;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_LISTED_SOURCES: usize = 5000;

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct RepositoryManifest {
    #[schemars(with = "i64", range(min = 1, max = 1))]
    pub version: u8,
    pub repository: RepositoryIdentity,
    #[schemars(length(min = 0, max = 5000))]
    pub sources: Vec<ListedSource>,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct RepositoryIdentity {
    #[schemars(
        length(min = 2, max = 32),
        regex(pattern = r"^[a-z](?:[a-z0-9]|-(?=[a-z0-9])){1,31}$")
    )]
    pub id: String,
    #[schemars(length(min = 1, max = 120))]
    pub name: String,
    #[schemars(length(min = 1, max = 1024))]
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[schemars(deny_unknown_fields)]
pub struct ListedSource {
    #[schemars(length(min = 1, max = 120))]
    pub name: String,
    #[schemars(length(min = 1, max = 1024))]
    pub description: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 2, max = 16))]
    pub source_id: Option<String>,
    /// Marketplace listing: the publisher's display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 120))]
    pub publisher: Option<String>,
    /// Marketplace listing: how many packages the source currently publishes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_count: Option<u32>,
    /// Marketplace listing: when the source archive last changed (RFC 3339).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 64))]
    pub updated_at: Option<String>,
    /// Marketplace listing: the archive's current ETag for this caller, so an
    /// unchanged source is not asked for again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 128))]
    pub digest: Option<String>,
}

impl ListedSource {
    pub fn locator(&self) -> Result<Locator, String> {
        Locator::parse(&self.url)
    }
}

impl RepositoryManifest {
    /// Strict parse for publishing and the validators.
    pub fn from_slice(contents: &[u8]) -> Result<Self, String> {
        let (manifest, errors) = Self::parse(contents, true)?;
        match errors.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(manifest),
        }
    }

    /// Tolerant parse for a fetched catalog: unknown fields are ignored and an
    /// unusable listing is dropped with one message per listing.
    pub(crate) fn from_slice_tolerant(contents: &[u8]) -> Result<(Self, Vec<String>), String> {
        Self::parse(contents, false)
    }

    fn parse(contents: &[u8], strict: bool) -> Result<(Self, Vec<String>), String> {
        if contents.len() > MAX_MANIFEST_BYTES {
            return Err(format!(
                "{REPOSITORY_MANIFEST_FILE} is larger than the 1 MB limit."
            ));
        }
        let value = serde_json::from_slice::<serde_json::Value>(contents)
            .map_err(|error| format!("Could not parse {REPOSITORY_MANIFEST_FILE}: {error}"))?;
        match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(1) => {}
            Some(version) => {
                return Err(format!(
                    "{REPOSITORY_MANIFEST_FILE} uses unsupported version {version}."
                ));
            }
            None => {
                return Err(format!("{REPOSITORY_MANIFEST_FILE} has no valid version."));
            }
        }
        let repository = serde_json::from_value::<RepositoryIdentity>(
            value.get("repository").cloned().unwrap_or_default(),
        )
        .map_err(|error| {
            format!("Could not parse {REPOSITORY_MANIFEST_FILE}: repository: {error}")
        })?;
        validate_identity(&repository)?;
        let entries = value
            .get("sources")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| format!("{REPOSITORY_MANIFEST_FILE} has no sources list."))?;
        if entries.len() > MAX_LISTED_SOURCES {
            return Err(format!(
                "{REPOSITORY_MANIFEST_FILE} lists more than {MAX_LISTED_SOURCES} sources."
            ));
        }
        let mut sources = Vec::new();
        let mut errors = Vec::new();
        let mut urls = BTreeSet::new();
        for (index, entry) in entries.iter().enumerate() {
            let parsed = serde_json::from_value::<ListedSource>(entry.clone())
                .map_err(|error| format!("Listed source {}: {error}", index + 1))
                .and_then(|source| {
                    if strict {
                        reject_unknown_fields(entry, &source, &format!("sources[{index}]"))
                            .map_err(|error| {
                                format!("Could not parse {REPOSITORY_MANIFEST_FILE}: {error}")
                            })?;
                    }
                    validate_listing(index, &source, &mut urls)?;
                    Ok(source)
                });
            match parsed {
                Ok(source) => sources.push(source),
                Err(error) => errors.push(error),
            }
        }
        let manifest = Self {
            version: REPOSITORY_MANIFEST_VERSION,
            repository,
            sources,
        };
        if strict && errors.is_empty() {
            reject_unknown_fields(&value, &manifest, "")
                .map_err(|error| format!("Could not parse {REPOSITORY_MANIFEST_FILE}: {error}"))?;
        }
        Ok((manifest, errors))
    }

    pub fn from_path(path: &Path) -> Result<Self, String> {
        Self::from_slice(&read_manifest_file(path)?)
    }

    /// Tolerant read of a fetched catalog; dropped listings are logged.
    pub(crate) fn from_path_tolerant(path: &Path) -> Result<Self, String> {
        let (manifest, errors) = Self::from_slice_tolerant(&read_manifest_file(path)?)?;
        for error in errors {
            eprintln!(
                "Skipped a listing in the catalog {}: {error}",
                manifest.repository.name
            );
        }
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != REPOSITORY_MANIFEST_VERSION {
            return Err(format!(
                "{REPOSITORY_MANIFEST_FILE} uses unsupported version {}.",
                self.version
            ));
        }
        validate_identity(&self.repository)?;
        if self.sources.len() > MAX_LISTED_SOURCES {
            return Err(format!(
                "{REPOSITORY_MANIFEST_FILE} lists more than {MAX_LISTED_SOURCES} sources."
            ));
        }
        let mut urls = BTreeSet::new();
        for (index, source) in self.sources.iter().enumerate() {
            validate_listing(index, source, &mut urls)?;
        }
        Ok(())
    }

    pub fn canonical_sources(&self) -> Result<Vec<ListedSource>, String> {
        self.sources
            .iter()
            .map(|source| {
                let locator = source.locator()?;
                Ok(ListedSource {
                    name: source.name.clone(),
                    description: source.description.clone(),
                    url: locator.url().to_string(),
                    source_id: source.source_id.clone(),
                    publisher: source.publisher.clone(),
                    package_count: source.package_count,
                    updated_at: source.updated_at.clone(),
                    digest: source.digest.clone(),
                })
            })
            .collect()
    }
}

fn read_manifest_file(path: &Path) -> Result<Vec<u8>, String> {
    let file = if path.is_dir() {
        path.join(REPOSITORY_MANIFEST_FILE)
    } else {
        path.to_path_buf()
    };
    std::fs::read(&file).map_err(|error| format!("Could not read {}: {error}", file.display()))
}

fn validate_identity(repository: &RepositoryIdentity) -> Result<(), String> {
    validate_repository_id(&repository.id)?;
    validate_text(&repository.name, "repository.name", 1, 120)?;
    validate_text(&repository.description, "repository.description", 1, 1024)
}

/// Checks one listing; `urls` collects canonical URLs to catch duplicates.
fn validate_listing(
    index: usize,
    source: &ListedSource,
    urls: &mut BTreeSet<String>,
) -> Result<(), String> {
    validate_text(&source.name, "sources[].name", 1, 120)?;
    validate_text(&source.description, "sources[].description", 1, 1024)?;
    let locator = source
        .locator()
        .map_err(|error| format!("Listed source {} has an invalid URL: {error}", index + 1))?;
    if !urls.insert(locator.url().to_string()) {
        return Err(
            "Source repository listings contain a duplicate URL after canonicalization."
                .to_string(),
        );
    }
    if let Some(source_id) = &source.source_id {
        validate_source_id(source_id).map_err(|error| {
            format!(
                "Listed source {} has an invalid sourceId: {error}",
                index + 1
            )
        })?;
    }
    if let Some(publisher) = &source.publisher {
        validate_text(publisher, "sources[].publisher", 1, 120)?;
    }
    if let Some(updated_at) = &source.updated_at {
        validate_text(updated_at, "sources[].updatedAt", 1, 64)?;
    }
    Ok(())
}

fn validate_repository_id(value: &str) -> Result<(), String> {
    let valid = (2..=32).contains(&value.len())
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
            "Invalid repository.id {value:?}; use 2-32 lowercase ASCII letters, digits, and single hyphens, beginning with a letter."
        ))
    }
}

pub fn source_repository_schema_json() -> Result<String, String> {
    let generator = SchemaSettings::draft2020_12().into_generator();
    let schema = generator.into_root_schema_for::<RepositoryManifest>();
    let mut schema = serde_json::to_value(schema)
        .map_err(|error| format!("Could not prepare the source-repository schema: {error}"))?;
    if let Some(version) = schema.pointer_mut("/properties/version") {
        version
            .as_object_mut()
            .expect("the generated version schema is an object")
            .remove("format");
    }
    let mut output = serde_json::to_string_pretty(&schema)
        .map_err(|error| format!("Could not serialize the source-repository schema: {error}"))?;
    output.push('\n');
    Ok(output)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryValidationError {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryValidationReport {
    pub repository_id: String,
    pub listed_sources: usize,
    pub errors: Vec<RepositoryValidationError>,
}

pub fn validate_source_repository(input: &str) -> Result<RepositoryValidationReport, String> {
    report_manifest(&RepositoryManifest::from_path(Path::new(input))?)
}

pub(crate) fn report_manifest(
    manifest: &RepositoryManifest,
) -> Result<RepositoryValidationReport, String> {
    Ok(RepositoryValidationReport {
        repository_id: manifest.repository.id.clone(),
        listed_sources: manifest.sources.len(),
        errors: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"{
      "version": 1,
      "repository": {
        "id": "acme",
        "name": "Acme sources",
        "description": "Official portable sources."
      },
      "sources": [
        {
          "name": "Review workflows",
          "description": "Review skill and database MCP server.",
          "url": "https://nexus.example.com/repository/raw/sources/review-latest.zip"
        },
        {
          "name": "Data tools",
          "description": "Published from Nexus as a zip.",
          "url": "https://nexus.example.com/repository/raw/sources/data-latest.zip"
        }
      ]
    }"#;

    #[test]
    fn valid_catalog_is_accepted() {
        let manifest = RepositoryManifest::from_slice(EXAMPLE.as_bytes()).expect("manifest");
        assert_eq!(manifest.repository.id, "acme");
        assert_eq!(manifest.canonical_sources().expect("canonical").len(), 2);
    }

    #[test]
    fn duplicate_locators_are_rejected() {
        let duplicate = r#"{
          "version": 1,
          "repository": {"id":"acme","name":"Acme","description":"Sources"},
          "sources": [
            {"name":"One","description":"One","url":"https://nexus.example.com/repository/raw/sources/one-latest.zip"},
            {"name":"Two","description":"Two","url":"HTTPS://Nexus.Example.com:443/repository/raw/sources/one-latest.zip"}
          ]
        }"#;
        assert!(RepositoryManifest::from_slice(duplicate.as_bytes())
            .expect_err("duplicate")
            .contains("duplicate URL"));
    }

    #[test]
    fn unknown_fields_and_bad_ids_are_rejected() {
        let unknown = EXAMPLE.replace("\"version\": 1,", "\"version\": 1, \"extra\": true,");
        assert!(RepositoryManifest::from_slice(unknown.as_bytes())
            .expect_err("unknown")
            .contains("unknown field"));
        let bad_id = EXAMPLE.replace("\"acme\"", "\"Acme\"");
        assert!(RepositoryManifest::from_slice(bad_id.as_bytes())
            .expect_err("id")
            .contains("repository.id"));
    }

    #[test]
    fn fetched_catalogs_keep_the_listings_they_can_use() {
        let tolerant = EXAMPLE
            .replace("\"version\": 1,", "\"version\": 1, \"extra\": true,")
            .replace(
                "https://nexus.example.com/repository/raw/sources/data-latest.zip",
                "ftp://nexus.example.com/data.zip",
            );
        let (manifest, errors) =
            RepositoryManifest::from_slice_tolerant(tolerant.as_bytes()).expect("tolerant");
        assert_eq!(manifest.sources.len(), 1);
        assert_eq!(manifest.sources[0].name, "Review workflows");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("Listed source 2"), "{errors:?}");
        assert!(RepositoryManifest::from_slice(tolerant.as_bytes()).is_err());
    }

    #[test]
    fn schema_matches_the_published_golden_file() {
        let generated = source_repository_schema_json().expect("generated schema");
        let published = include_str!("../../schemas/v1/source-repository.schema.json");
        let generated =
            serde_json::from_str::<serde_json::Value>(&generated).expect("generated schema JSON");
        let published =
            serde_json::from_str::<serde_json::Value>(published).expect("published schema JSON");
        assert_eq!(
            generated, published,
            "regenerate the source-repository schema"
        );
    }
}
