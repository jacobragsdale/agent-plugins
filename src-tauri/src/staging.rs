//! Turns what a publisher has into a one-package source the marketplace accepts: a skill
//! directory, a directory of skill directories (a skill pack), an MCP document, or an existing
//! source tree. The CLI and the server's browser uploads (`validate-source stage`) share it.

use crate::sources::copy_directory;
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const SECRET_SCAN_LIMIT: u64 = 2 * 1024 * 1024;

/// Where to publish and, optionally, how to label the package.
pub struct StageRequest<'a> {
    pub namespace: &'a str,
    pub package_id: Option<&'a str>,
    /// Package name; defaults to the title-cased package ID.
    pub name: Option<&'a str>,
    /// Package description; defaults to the skill's own description.
    pub description: Option<&'a str>,
}

/// A source tree staged for upload: `agent-plugins.json` at the root and one package.
pub struct StagedPackage {
    pub root: PathBuf,
    pub package_id: String,
    pub file_count: usize,
    pub total_bytes: u64,
}

/// Builds a one-package source tree under `staging/source`.
pub fn stage_tree(
    input: &Path,
    request: &StageRequest<'_>,
    staging: &Path,
) -> Result<StagedPackage, String> {
    let namespace = request.namespace;
    let input = input
        .canonicalize()
        .map_err(|error| format!("{}: {error}", input.display()))?;
    // Each branch creates `root` itself: copying a source tree requires that it not exist yet.
    let root = staging.join("source");
    std::fs::create_dir_all(staging).map_err(|error| error.to_string())?;
    let package_id = if input.join(crate::manifest::SOURCE_MANIFEST_FILE).is_file() {
        let bytes = std::fs::read(input.join(crate::manifest::SOURCE_MANIFEST_FILE))
            .map_err(|error| error.to_string())?;
        let crate::manifest::SourceManifest::V2(manifest) =
            crate::manifest::SourceManifest::from_slice(&bytes)?;
        if manifest.source.id != namespace {
            return Err(format!(
                "{} declares source.id {}; publish to the namespace {namespace} by setting source.id to it.",
                crate::manifest::SOURCE_MANIFEST_FILE,
                manifest.source.id
            ));
        }
        let [package] = manifest.packages.as_slice() else {
            return Err("A marketplace upload must declare exactly one package.".to_string());
        };
        if let Some(requested) = request.package_id {
            if requested != package.id {
                return Err(format!(
                    "The tree declares package {}, not {requested}.",
                    package.id
                ));
            }
        }
        copy_directory(&input, &root)?;
        package.id.clone()
    } else if input.join("SKILL.md").is_file() {
        let (name, description) = skill_frontmatter(&input.join("SKILL.md"))?;
        let id = request
            .package_id
            .map(str::to_string)
            .unwrap_or_else(|| skill_id(namespace, &name));
        copy_skill(&input, &root, &name, &id)?;
        write_manifest(
            &root,
            namespace,
            &id,
            &request.name.map_or_else(|| title_case(&id), str::to_string),
            request.description.unwrap_or(&description),
            &[serde_json::json!({ "kind": "skill", "path": format!("skills/{id}") })],
        )?;
        id
    } else if input.is_file() && input.extension().is_some_and(|ext| ext == "json") {
        let bytes = std::fs::read(&input).map_err(|error| error.to_string())?;
        let document: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("{} is not valid JSON: {error}", input.display()))?;
        if document.get("mcpServers").is_none() {
            return Err(format!("{} has no mcpServers object.", input.display()));
        }
        let stem = input
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("mcp")
            .to_lowercase();
        let id = request.package_id.map(str::to_string).unwrap_or(stem);
        std::fs::create_dir_all(root.join("mcp")).map_err(|error| error.to_string())?;
        std::fs::copy(&input, root.join("mcp").join(format!("{id}.json")))
            .map_err(|error| error.to_string())?;
        write_manifest(
            &root,
            namespace,
            &id,
            &request.name.map_or_else(|| title_case(&id), str::to_string),
            &request.description.map_or_else(
                || format!("{} MCP server.", title_case(&id)),
                str::to_string,
            ),
            &[serde_json::json!({ "kind": "mcpServer", "path": format!("mcp/{id}.json") })],
        )?;
        id
    } else if let Some(skills) = skill_directories(&input)? {
        stage_pack(&input, &skills, request, &root)?
    } else {
        return Err(format!(
            "{} is not a skill directory (SKILL.md), a folder of skill directories, an MCP document (.json), or a source tree (agent-plugins.json).",
            input.display()
        ));
    };
    let (file_count, total_bytes) = count_files(&root)?;
    Ok(StagedPackage {
        root,
        package_id,
        file_count,
        total_bytes,
    })
}

/// A skill pack: every subdirectory is one skill, published together as one package.
fn stage_pack(
    input: &Path,
    skills: &[PathBuf],
    request: &StageRequest<'_>,
    root: &Path,
) -> Result<String, String> {
    let namespace = request.namespace;
    let package_id = match request.package_id {
        Some(id) => id.to_string(),
        None => input
            .file_name()
            .and_then(|name| name.to_str())
            .map(slug)
            .filter(|id| !id.is_empty())
            .ok_or_else(|| "Name the skill pack with --package-id.".to_string())?,
    };
    let mut components = Vec::with_capacity(skills.len());
    let mut descriptions = Vec::with_capacity(skills.len());
    for skill in skills {
        let (name, _) = skill_frontmatter(&skill.join("SKILL.md"))?;
        let id = skill_id(namespace, &name);
        if components
            .iter()
            .any(|component: &serde_json::Value| component["id"] == id.as_str())
        {
            return Err(format!("Two skills in the pack are both named {id}."));
        }
        copy_skill(skill, root, &name, &id)?;
        components
            .push(serde_json::json!({ "kind": "skill", "id": id, "path": format!("skills/{id}") }));
        descriptions.push(title_case(&id));
    }
    let description = request.description.map_or_else(
        || format!("Skill pack: {}.", descriptions.join(", ")),
        str::to_string,
    );
    write_manifest(
        root,
        namespace,
        &package_id,
        &request
            .name
            .map_or_else(|| title_case(&package_id), str::to_string),
        &description,
        &components,
    )?;
    Ok(package_id)
}

/// The subdirectories of `input` when every one of them holds a SKILL.md. Loose files beside
/// them (a README, `.DS_Store`) are left out of the pack.
fn skill_directories(input: &Path) -> Result<Option<Vec<PathBuf>>, String> {
    if !input.is_dir() {
        return Ok(None);
    }
    let mut directories = Vec::new();
    for entry in
        std::fs::read_dir(input).map_err(|error| format!("{}: {error}", input.display()))?
    {
        let entry = entry.map_err(|error| error.to_string())?;
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            directories.push(entry.path());
        }
    }
    directories.sort();
    if directories.is_empty() || !directories.iter().any(|dir| dir.join("SKILL.md").is_file()) {
        return Ok(None);
    }
    if let Some(stray) = directories
        .iter()
        .find(|dir| !dir.join("SKILL.md").is_file())
    {
        return Err(format!(
            "{} has no SKILL.md; every folder in a skill pack must be a skill.",
            stray.display()
        ));
    }
    Ok(Some(directories))
}

/// Copies a skill to `skills/<id>` and makes its frontmatter name match the component ID.
fn copy_skill(skill: &Path, root: &Path, name: &str, id: &str) -> Result<(), String> {
    let skill_dir = root.join("skills").join(id);
    std::fs::create_dir_all(skill_dir.parent().unwrap_or(root))
        .map_err(|error| error.to_string())?;
    copy_directory(skill, &skill_dir)?;
    if name != id {
        rewrite_skill_name(&skill_dir.join("SKILL.md"), id)?;
    }
    Ok(())
}

/// Installed skills are named `<namespace>-<id>`, so a name that already carries the prefix drops it.
fn skill_id(namespace: &str, name: &str) -> String {
    name.strip_prefix(&format!("{namespace}-"))
        .unwrap_or(name)
        .to_string()
}

fn write_manifest(
    root: &Path,
    namespace: &str,
    package_id: &str,
    name: &str,
    description: &str,
    components: &[serde_json::Value],
) -> Result<(), String> {
    let manifest = serde_json::json!({
        "version": 2,
        "source": {
            "id": namespace,
            "name": namespace,
            "description": format!("Packages published by {namespace}."),
        },
        "packages": [{
            "id": package_id,
            "name": name,
            "description": description,
            "components": components,
        }],
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?;
    std::fs::write(
        root.join(crate::manifest::SOURCE_MANIFEST_FILE),
        text + "\n",
    )
    .map_err(|error| error.to_string())
}

/// Reads `name` and `description` from SKILL.md frontmatter.
fn skill_frontmatter(path: &Path) -> Result<(String, String), String> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Err(format!(
            "{} does not start with YAML frontmatter.",
            path.display()
        ));
    }
    let frontmatter = lines
        .by_ref()
        .take_while(|line| line.trim() != "---")
        .collect::<Vec<_>>()
        .join("\n");
    let mapping: BTreeMap<String, serde_yaml_ng::Value> = serde_yaml_ng::from_str(&frontmatter)
        .map_err(|error| format!("{} frontmatter is not valid YAML: {error}", path.display()))?;
    let name = mapping
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} frontmatter has no name.", path.display()))?
        .to_string();
    let description = mapping
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{} frontmatter has no description.", path.display()))?
        .to_string();
    Ok((name, description))
}

/// The skill name must equal the component ID; rewrite only the `name:` line.
fn rewrite_skill_name(path: &Path, id: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let text = text.trim_start_matches('\u{feff}');
    let mut out = String::with_capacity(text.len());
    let mut in_frontmatter = false;
    let mut replaced = false;
    for (index, line) in text.lines().enumerate() {
        if index == 0 && line.trim() == "---" {
            in_frontmatter = true;
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if in_frontmatter && line.trim() == "---" {
            in_frontmatter = false;
        }
        if in_frontmatter && !replaced && line.starts_with("name:") {
            out.push_str(&format!("name: {id}\n"));
            replaced = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    std::fs::write(path, out).map_err(|error| error.to_string())
}

fn title_case(id: &str) -> String {
    id.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lowercase letters, digits, and single hyphens, as package IDs require.
fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').chars().take(64).collect()
}

fn count_files(root: &Path) -> Result<(usize, u64), String> {
    let mut count = 0;
    let mut bytes = 0;
    for path in walk(root)? {
        let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
        count += 1;
        bytes += metadata.len();
    }
    Ok((count, bytes))
}

fn walk(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = std::fs::read_dir(&dir)
            .map_err(|error| format!("{}: {error}", dir.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            if file_type.is_symlink() {
                return Err(format!(
                    "{} is a symbolic link; packages cannot contain links.",
                    path.display()
                ));
            }
            if file_type.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

const SECRET_FILE_NAMES: [&str; 6] = [
    ".env",
    "id_rsa",
    "id_ed25519",
    "id_ecdsa",
    ".npmrc",
    ".netrc",
];
const SECRET_EXTENSIONS: [&str; 6] = ["pem", "key", "p12", "pfx", "keytab", "jks"];
const SECRET_MARKERS: [&str; 8] = [
    "PRIVATE KEY-----",
    "ghp_",
    "github_pat_",
    "xoxb-",
    "xoxp-",
    "sk-ant-",
    "AKIA",
    "-----BEGIN OPENSSH",
];

/// A file that looks like a credential, by name, extension, or content marker.
pub struct SecretFinding {
    /// Path relative to the scanned root, with forward slashes.
    pub path: String,
    pub reason: String,
}

/// Refuses obvious credentials: by file name, extension, or content marker.
pub fn scan_for_secrets(root: &Path) -> Result<Vec<SecretFinding>, String> {
    let mut findings = Vec::new();
    for path in walk(root)? {
        let relative = relative_path(root, &path)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if SECRET_FILE_NAMES.contains(&file_name) || file_name.starts_with(".env.") {
            findings.push(SecretFinding {
                path: relative,
                reason: "credential file name".to_string(),
            });
            continue;
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| SECRET_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        {
            findings.push(SecretFinding {
                path: relative,
                reason: "credential file extension".to_string(),
            });
            continue;
        }
        let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
        if metadata.len() > SECRET_SCAN_LIMIT {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        let text = String::from_utf8_lossy(&bytes);
        for marker in SECRET_MARKERS {
            if let Some(position) = text.find(marker) {
                if marker == "AKIA" {
                    let tail = text[position + 4..].chars().take(16).collect::<String>();
                    if tail.len() < 16
                        || !tail
                            .chars()
                            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
                    {
                        continue;
                    }
                }
                findings.push(SecretFinding {
                    path: relative,
                    reason: format!("contains {marker}"),
                });
                break;
            }
        }
    }
    Ok(findings)
}

/// Deterministic zip of the tree: sorted entries, forward slashes, fixed timestamps.
pub fn zip_tree(root: &Path) -> Result<Vec<u8>, String> {
    use std::io::Cursor;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let base_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for path in walk(root)? {
        let options = unix_permissions(&path, base_options)?;
        writer
            .start_file(relative_path(root, &path)?, options)
            .map_err(|error| error.to_string())?;
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        writer
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
    }
    let cursor = writer.finish().map_err(|error| error.to_string())?;
    Ok(cursor.into_inner())
}

fn relative_path(root: &Path, path: &Path) -> Result<String, String> {
    Ok(path
        .strip_prefix(root)
        .map_err(|error| error.to_string())?
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}

#[cfg(unix)]
fn unix_permissions(
    path: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<zip::write::SimpleFileOptions, String> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = std::fs::metadata(path)
        .map_err(|error| error.to_string())?
        .permissions()
        .mode();
    Ok(options.unix_permissions(mode & 0o777))
}

#[cfg(not(unix))]
fn unix_permissions(
    _path: &Path,
    options: zip::write::SimpleFileOptions,
) -> Result<zip::write::SimpleFileOptions, String> {
    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(namespace: &str) -> StageRequest<'_> {
        StageRequest {
            namespace,
            package_id: None,
            name: None,
            description: None,
        }
    }

    fn write_skill(dir: &Path, name: &str, description: &str) {
        std::fs::create_dir_all(dir).expect("skill dir");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
        )
        .expect("write");
    }

    #[test]
    fn stages_a_bare_skill_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let skill = temp.path().join("review");
        write_skill(&skill, "jacob-review", "Reviews a change.");
        let staging = temp.path().join("staging");
        let staged = stage_tree(&skill, &request("jacob"), &staging).expect("stage");
        assert_eq!(staged.package_id, "review");
        let manifest =
            std::fs::read_to_string(staged.root.join("agent-plugins.json")).expect("manifest");
        assert!(manifest.contains("\"id\": \"jacob\""));
        assert!(manifest.contains("skills/review"));
        let rewritten =
            std::fs::read_to_string(staged.root.join("skills/review/SKILL.md")).expect("skill");
        assert!(rewritten.starts_with("---\nname: review\n"));
        let report =
            crate::source::validate_source(&staged.root.display().to_string()).expect("validate");
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.valid_installs, 1);
        let archive = zip_tree(&staged.root).expect("zip");
        assert!(archive.len() > 100);
    }

    #[test]
    fn rewrites_the_name_after_a_bom_and_ignores_nested_names() {
        let temp = tempfile::tempdir().expect("tempdir");
        for (label, text) in [
            (
                "bom",
                "\u{feff}---\nname: acme-review\ndescription: d\n---\n",
            ),
            (
                "nested",
                "---\nmetadata:\n  name: x\nname: acme-review\ndescription: d\n---\n",
            ),
        ] {
            let skill = temp.path().join(label).join("review");
            std::fs::create_dir_all(&skill).expect("skill dir");
            std::fs::write(skill.join("SKILL.md"), text).expect("write");
            let staged = stage_tree(
                &skill,
                &request("acme"),
                &temp.path().join(label).join("out"),
            )
            .expect("stage");
            let rewritten =
                std::fs::read_to_string(staged.root.join("skills/review/SKILL.md")).expect("skill");
            assert!(
                rewritten.contains("\nname: review\n"),
                "{label}: {rewritten}"
            );
            let report = crate::source::validate_source(&staged.root.display().to_string())
                .expect("validate");
            assert!(report.errors.is_empty(), "{label}: {:?}", report.errors);
            assert_eq!(report.valid_installs, 1, "{label}");
        }
    }

    #[test]
    fn stages_an_existing_source_tree() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tree = temp.path().join("tree");
        write_skill(&tree.join("skills/lint"), "lint", "Checks style.");
        std::fs::write(
            tree.join("agent-plugins.json"),
            r#"{"version":2,"source":{"id":"jacob","name":"j","description":"j"},"packages":[{"id":"lint","components":[{"kind":"skill","path":"skills/lint"}]}]}"#,
        )
        .expect("manifest");
        let staged =
            stage_tree(&tree, &request("jacob"), &temp.path().join("staging")).expect("stage");
        assert_eq!(staged.package_id, "lint");
        assert_eq!(staged.file_count, 2);
        let error = stage_tree(&tree, &request("other"), &temp.path().join("again"))
            .err()
            .expect("namespace mismatch");
        assert!(error.contains("source.id jacob"), "{error}");
    }

    #[test]
    fn stages_a_folder_of_skills_as_one_pack() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pack = temp.path().join("Writing Helpers");
        write_skill(&pack.join("a"), "jacob-tone", "Matches our house tone.");
        write_skill(&pack.join("b"), "summarize", "Summarizes meeting notes.");
        std::fs::write(pack.join("README.md"), "left out\n").expect("readme");
        let staged =
            stage_tree(&pack, &request("jacob"), &temp.path().join("staging")).expect("stage");
        assert_eq!(staged.package_id, "writing-helpers");
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(staged.root.join("agent-plugins.json")).expect("manifest"),
        )
        .expect("json");
        let package = &manifest["packages"][0];
        assert_eq!(package["name"], "Writing Helpers");
        assert_eq!(package["description"], "Skill pack: Tone, Summarize.");
        assert_eq!(package["components"][0]["id"], "tone");
        assert_eq!(package["components"][1]["path"], "skills/summarize");
        assert!(!staged.root.join("README.md").exists());
        let report =
            crate::source::validate_source(&staged.root.display().to_string()).expect("validate");
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.valid_installs, 1);

        let labelled = StageRequest {
            namespace: "jacob",
            package_id: Some("helpers"),
            name: Some("Helpers"),
            description: Some("Writing helpers for the team."),
        };
        let relabelled = stage_tree(&pack, &labelled, &temp.path().join("again")).expect("stage");
        assert_eq!(relabelled.package_id, "helpers");
    }

    #[test]
    fn refuses_a_pack_folder_without_a_skill() {
        let temp = tempfile::tempdir().expect("tempdir");
        let pack = temp.path().join("pack");
        write_skill(&pack.join("one"), "one", "First.");
        std::fs::create_dir_all(pack.join("notes")).expect("stray");
        let error = stage_tree(&pack, &request("jacob"), &temp.path().join("staging"))
            .err()
            .expect("error");
        assert!(error.contains("notes has no SKILL.md"), "{error}");
    }

    #[test]
    fn refuses_secrets() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("notes.txt"), "token ghp_abcdef\n").expect("write");
        std::fs::write(temp.path().join("ok.txt"), "AKIAnotakey\n").expect("write");
        std::fs::create_dir_all(temp.path().join("config")).expect("dir");
        std::fs::write(temp.path().join("config/.env"), "A=1\n").expect("write");
        let findings = scan_for_secrets(temp.path()).expect("scan");
        let paths = findings
            .iter()
            .map(|finding| finding.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, ["config/.env", "notes.txt"]);
    }
}
