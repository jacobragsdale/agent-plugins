//! Turns what a publisher has into a one-package source the marketplace accepts: a skill
//! directory, a directory of skill directories (a skill pack), an MCP document, or an existing
//! source tree. The CLI and the server's browser uploads (`validate-source stage`) share it.

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

/// Builds a one-package source tree under `staging/source`. Errors name
/// files relative to `input`, so a server's temporary folder never shows.
pub fn stage_tree(
    input: &Path,
    request: &StageRequest<'_>,
    staging: &Path,
) -> Result<StagedPackage, String> {
    let canonical = input
        .canonicalize()
        .map_err(|error| format!("{}: {error}", input.display()))?;
    let whole = if canonical.is_dir() {
        "This folder".to_string()
    } else {
        canonical
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
    };
    // Only absolute paths are replaced: a relative one such as `.` could
    // match text anywhere in the message.
    stage_in(&canonical, request, staging).map_err(|error| {
        [canonical.as_path(), input]
            .iter()
            .filter(|root| root.is_absolute())
            .fold(error, |error, root| {
                let root = root.display().to_string();
                error
                    .replace(&format!("{root}/"), "")
                    .replace(&format!("{root}\\"), "")
                    .replace(&root, &whole)
            })
    })
}

fn stage_in(
    input: &Path,
    request: &StageRequest<'_>,
    staging: &Path,
) -> Result<StagedPackage, String> {
    let namespace = request.namespace;
    let input = input.to_path_buf();
    // Each branch creates `root` itself: copying a source tree requires that it not exist yet.
    let root = staging.join("source");
    std::fs::create_dir_all(staging).map_err(|error| error.to_string())?;
    let package_id = if input.join(crate::manifest::SOURCE_MANIFEST_FILE).is_file() {
        let bytes = std::fs::read(input.join(crate::manifest::SOURCE_MANIFEST_FILE))
            .map_err(|error| error.to_string())?;
        let crate::manifest::SourceManifest::V2(mut manifest) =
            crate::manifest::SourceManifest::from_slice(&bytes)?;
        let package = match (manifest.packages.as_slice(), request.package_id) {
            ([package], None) => package.clone(),
            ([package], Some(requested)) if requested == package.id => package.clone(),
            (packages, Some(requested)) => packages
                .iter()
                .find(|package| package.id == requested)
                .cloned()
                .ok_or_else(|| {
                    format!(
                        "{} has no package {requested}. It declares: {}.",
                        crate::manifest::SOURCE_MANIFEST_FILE,
                        packages
                            .iter()
                            .map(|package| package.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?,
            (packages, None) => {
                return Err(format!(
                    "{} declares {} packages; choose one with --package-id ({}).",
                    crate::manifest::SOURCE_MANIFEST_FILE,
                    packages.len(),
                    packages
                        .iter()
                        .map(|package| package.id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        };
        // Only the manifest and what the package declares are published, so a
        // repository's .git, tests, and tooling never ship. The source takes
        // the namespace it is published to.
        std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
        for component in &package.components {
            // An upload is untrusted: a path must stay inside it, links included.
            let relative = crate::catalog::validate_relative_path(component.path(), "component")?;
            let from = input
                .join(&relative)
                .canonicalize()
                .map_err(|error| format!("{}: {error}", component.path()))?;
            if !from.starts_with(&input) {
                return Err(format!(
                    "{} points outside the folder being published.",
                    component.path()
                ));
            }
            let to = root.join(&relative);
            if from.is_dir() {
                std::fs::create_dir_all(to.parent().unwrap_or(&root))
                    .map_err(|error| error.to_string())?;
                copy_publishable(&from, &to)?;
            } else {
                std::fs::create_dir_all(to.parent().unwrap_or(&root))
                    .map_err(|error| error.to_string())?;
                std::fs::copy(&from, &to)
                    .map_err(|error| format!("{}: {error}", component.path()))?;
            }
        }
        manifest.source.id = namespace.to_string();
        manifest.packages = vec![package.clone()];
        let text = serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?;
        std::fs::write(
            root.join(crate::manifest::SOURCE_MANIFEST_FILE),
            text + "\n",
        )
        .map_err(|error| error.to_string())?;
        package.id
    } else if let Some(skill_file) = skill_file(&input) {
        let (name, description) = skill_frontmatter(&skill_file)?;
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
            "{} is not a skill: a skill is a folder with a SKILL.md file in it. A folder of skill folders, an MCP document (.json), or a source tree (agent-plugins.json) works too.",
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
        let skill_md =
            skill_file(skill).ok_or_else(|| format!("{} has no SKILL.md.", skill.display()))?;
        let (name, _) = skill_frontmatter(&skill_md)?;
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
    if directories.is_empty() || !directories.iter().any(|dir| skill_file(dir).is_some()) {
        return Ok(None);
    }
    if let Some(stray) = directories.iter().find(|dir| skill_file(dir).is_none()) {
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
    copy_publishable(skill, &skill_dir)?;
    // `skill.md` or Notepad's `SKILL.md.txt` is the same file under the name agents look for.
    if let Some(found) =
        skill_file(&skill_dir).filter(|found| found.file_name() != Some("SKILL.md".as_ref()))
    {
        std::fs::rename(&found, skill_dir.join("SKILL.md")).map_err(|error| error.to_string())?;
    }
    if name != id {
        rewrite_skill_name(&skill_dir.join("SKILL.md"), id)?;
    }
    Ok(())
}

/// The skill's instructions file: `SKILL.md`, or the same name in another
/// case or with the `.txt` Notepad adds.
/// The name on disk is read back rather than assumed, because Windows and
/// macOS also find `skill.md` when asked for `SKILL.md`.
fn skill_file(dir: &Path) -> Option<PathBuf> {
    let exact = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.file_name() == Some("SKILL.md".as_ref()) && path.is_file());
    if exact.is_some() {
        return exact;
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| {
                        matches!(
                            name.to_ascii_lowercase().as_str(),
                            "skill.md" | "skill.md.txt"
                        )
                    })
        })
}

/// Copies a folder, leaving tool leftovers out.
fn copy_publishable(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to)
        .map_err(|error| format!("Could not create {}: {error}", to.display()))?;
    let mut entries = std::fs::read_dir(from)
        .map_err(|error| format!("{}: {error}", from.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let name = entry.file_name();
        if name.to_str().is_some_and(crate::digest::is_tool_leftover) {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_dir() {
            copy_publishable(&entry.path(), &to.join(&name))?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), to.join(&name))
                .map_err(|error| format!("Could not copy {}: {error}", entry.path().display()))?;
        } else {
            return Err(format!(
                "{} is a link or special file; a package can hold only files and folders.",
                entry.path().display()
            ));
        }
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
            "{} must start with a header: a line with ---, then name: and description: lines, then another ---.",
            path.display()
        ));
    }
    let frontmatter = lines
        .by_ref()
        .take_while(|line| line.trim() != "---")
        .collect::<Vec<_>>()
        .join("\n");
    let mapping: BTreeMap<String, serde_yaml_ng::Value> = serde_yaml_ng::from_str(&frontmatter)
        .map_err(|error| {
            let hint = if error.to_string().contains("mapping values are not allowed") {
                "A value there contains a colon followed by a space; put that value in double quotes, like description: \"Use it when: ...\"."
            } else {
                "A value that contains a colon, #, or starts with a quote or bracket needs double quotes around it."
            };
            format!("The header between the --- lines at the top of {} isn't valid. {hint} ({error})", path.display())
        })?;
    let name = mapping
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("The header of {} needs a name: line.", path.display()))?
        .to_string();
    let description = mapping
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("The header of {} needs a description: line saying when the assistant should use the skill.", path.display()))?
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

const SECRET_FILE_NAMES: [&str; 5] = [".env", "id_rsa", "id_ed25519", "id_ecdsa", ".netrc"];
/// `.env.example` and its kin hold placeholders for people to fill in.
const ENV_TEMPLATES: [&str; 4] = [".env.example", ".env.sample", ".env.template", ".env.dist"];
/// Key stores. A `.pem` is often only a public certificate, so its content decides.
const SECRET_EXTENSIONS: [&str; 5] = ["key", "p12", "pfx", "keytab", "jks"];
/// A marker, how many token characters must follow it, and what it is.
const SECRET_MARKERS: [(&str, usize, &str); 10] = [
    ("-----BEGIN OPENSSH", 0, "a private key"),
    ("PRIVATE KEY-----", 0, "a private key"),
    ("ghp_", 36, "a GitHub token"),
    ("gho_", 36, "a GitHub token"),
    ("github_pat_", 22, "a GitHub token"),
    ("xoxb-", 10, "a Slack token"),
    ("xoxp-", 10, "a Slack token"),
    ("sk-ant-", 20, "an Anthropic API key"),
    ("sk-proj-", 20, "an OpenAI API key"),
    ("AKIA", 16, "an AWS access key"),
];
/// AWS's documented example key, which appears in docs and tests.
const AWS_EXAMPLE_KEY: &str = "AKIAIOSFODNN7EXAMPLE";

/// A file that looks like a credential, by name, extension, or content marker.
pub struct SecretFinding {
    /// Path relative to the scanned root, with forward slashes.
    pub path: String,
    pub reason: String,
}

/// Refuses obvious credentials: by file name, extension, a token in the
/// content, or an MCP server that writes a secret down instead of reading it
/// from an environment variable.
pub fn scan_for_secrets(root: &Path) -> Result<Vec<SecretFinding>, String> {
    let mut findings = Vec::new();
    for path in walk(root)? {
        let relative = relative_path(root, &path)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let finding = |reason: String| SecretFinding {
            path: relative.clone(),
            reason,
        };
        if SECRET_FILE_NAMES.contains(&file_name)
            || (file_name.starts_with(".env.") && !ENV_TEMPLATES.contains(&file_name))
        {
            findings.push(finding("credential file name".to_string()));
            continue;
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| SECRET_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        {
            findings.push(finding("credential file extension".to_string()));
            continue;
        }
        let metadata = std::fs::metadata(&path).map_err(|error| error.to_string())?;
        if metadata.len() > SECRET_SCAN_LIMIT {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        let text = String::from_utf8_lossy(&bytes);
        if file_name == ".npmrc"
            && ["_authToken", "_auth=", "_password"]
                .iter()
                .any(|key| text.contains(key))
        {
            findings.push(finding(
                "an .npmrc with a registry password or token".to_string(),
            ));
            continue;
        }
        if let Some((line, what)) = token_in(&text) {
            findings.push(finding(format!("line {line} looks like {what}")));
            continue;
        }
        if file_name.ends_with(".json") {
            if let Some(reason) = literal_mcp_secret(&text) {
                findings.push(finding(reason));
            }
        }
    }
    Ok(findings)
}

/// The first line holding a token, and what kind it looks like.
fn token_in(text: &str) -> Option<(usize, &'static str)> {
    for (index, line) in text.lines().enumerate() {
        for (marker, length, what) in SECRET_MARKERS {
            for (position, _) in line.match_indices(marker) {
                let tail = &line[position + marker.len()..];
                let token = if marker == "AKIA" {
                    tail.bytes()
                        .take_while(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
                        .count()
                } else {
                    tail.bytes()
                        .take_while(|byte| {
                            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
                        })
                        .count()
                };
                if token >= length && !line[position..].starts_with(AWS_EXAMPLE_KEY) {
                    return Some((index + 1, what));
                }
            }
        }
    }
    None
}

/// An MCP document whose environment or headers hold a secret itself rather
/// than a `${NAME}` reference each person fills in.
fn literal_mcp_secret(text: &str) -> Option<String> {
    let document = serde_json::from_str::<serde_json::Value>(text).ok()?;
    let servers = document.get("mcpServers")?.as_object()?;
    for (server, definition) in servers {
        for field in ["env", "headers"] {
            let Some(values) = definition.get(field).and_then(|values| values.as_object()) else {
                continue;
            };
            for (key, value) in values {
                let Some(value) = value.as_str() else {
                    continue;
                };
                if is_secret_name(key)
                    && !value.trim().is_empty()
                    && crate::mcp::environment_references(value).is_empty()
                {
                    let variable = key.to_ascii_uppercase().replace('-', "_");
                    return Some(format!(
                        "MCP server {server} sets {key} to a fixed value; write \"${{{variable}}}\" instead so each person uses their own"
                    ));
                }
            }
        }
    }
    None
}

fn is_secret_name(key: &str) -> bool {
    key.split(['_', '-']).any(|part| {
        matches!(
            part.to_ascii_lowercase().as_str(),
            "token"
                | "secret"
                | "password"
                | "passwd"
                | "pwd"
                | "key"
                | "apikey"
                | "credential"
                | "credentials"
                | "auth"
                | "authorization"
                | "cookie"
        )
    })
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
    fn stages_only_what_the_chosen_package_declares() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tree = temp.path().join("tree");
        write_skill(&tree.join("skills/lint"), "lint", "Checks style.");
        write_skill(&tree.join("skills/docs"), "docs", "Writes docs.");
        std::fs::create_dir_all(tree.join(".git")).expect("git");
        std::fs::write(
            tree.join(".git/config"),
            "extraheader = AUTHORIZATION: basic x\n",
        )
        .expect("git config");
        std::fs::create_dir_all(tree.join("skills/lint/__pycache__")).expect("cache");
        std::fs::write(tree.join("skills/lint/__pycache__/x.pyc"), "x").expect("pyc");
        std::fs::write(tree.join("README.md"), "repo readme\n").expect("readme");
        std::fs::write(
            tree.join("agent-plugins.json"),
            r#"{"version":2,"source":{"id":"jacob","name":"j","description":"j"},"packages":[{"id":"lint","components":[{"kind":"skill","path":"skills/lint"}]},{"id":"docs","components":[{"kind":"skill","path":"skills/docs"}]}]}"#,
        )
        .expect("manifest");
        let error = stage_tree(&tree, &request("jacob"), &temp.path().join("none"))
            .err()
            .expect("two packages");
        assert!(error.contains("lint, docs"), "{error}");
        let lint = StageRequest {
            namespace: "team",
            package_id: Some("lint"),
            name: None,
            description: None,
        };
        let staged = stage_tree(&tree, &lint, &temp.path().join("staging")).expect("stage");
        assert_eq!(staged.package_id, "lint");
        assert_eq!(staged.file_count, 2);
        assert!(!staged.root.join(".git").exists());
        assert!(!staged.root.join("skills/docs").exists());
        let manifest: serde_json::Value = serde_json::from_slice(
            &std::fs::read(staged.root.join("agent-plugins.json")).expect("manifest"),
        )
        .expect("json");
        assert_eq!(manifest["source"]["id"], "team");
        assert_eq!(manifest["packages"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn component_paths_cannot_leave_the_upload() {
        let temp = tempfile::tempdir().expect("tempdir");
        let tree = temp.path().join("tree");
        write_skill(&tree.join("skills/lint"), "lint", "Checks style.");
        for path in ["/etc", "../outside", "skills/../../outside"] {
            std::fs::write(
                tree.join("agent-plugins.json"),
                format!(r#"{{"version":2,"source":{{"id":"jacob","name":"j","description":"j"}},"packages":[{{"id":"lint","components":[{{"kind":"skill","path":"{path}"}}]}}]}}"#),
            )
            .expect("manifest");
            assert!(
                stage_tree(
                    &tree,
                    &request("jacob"),
                    &temp.path().join(format!("s-{}", path.len()))
                )
                .is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn a_lowercase_or_notepad_skill_file_counts_and_errors_name_no_server_paths() {
        let temp = tempfile::tempdir().expect("tempdir");
        let skill = temp.path().join("upload");
        std::fs::create_dir_all(&skill).expect("dir");
        std::fs::write(
            skill.join("SKILL.md.txt"),
            "---\nname: notes\ndescription: Takes notes.\n---\nBody\n",
        )
        .expect("skill");
        let staged =
            stage_tree(&skill, &request("jacob"), &temp.path().join("staging")).expect("stage");
        assert!(staged.root.join("skills/notes/SKILL.md").is_file());
        std::fs::write(
            skill.join("SKILL.md.txt"),
            "---\nname: notes\ndescription: Use it when: always\n---\n",
        )
        .expect("skill");
        let error = stage_tree(&skill, &request("jacob"), &temp.path().join("again"))
            .err()
            .expect("bad yaml");
        assert!(error.contains("double quotes"), "{error}");
        assert!(
            !error.contains(&temp.path().display().to_string()),
            "{error}"
        );
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
        let root = temp.path();
        std::fs::write(
            root.join("notes.txt"),
            "docs\ntoken ghp_abcdefghijklmnopqrstuvwxyz0123456789\n",
        )
        .expect("write");
        std::fs::write(
            root.join("ok.txt"),
            "AKIAnotakey AKIAIOSFODNN7EXAMPLE ghp_ prefix\n",
        )
        .expect("write");
        std::fs::write(root.join(".env.example"), "API_KEY=\n").expect("write");
        std::fs::write(root.join("ca.pem"), "-----BEGIN CERTIFICATE-----\n").expect("write");
        std::fs::create_dir_all(root.join("config")).expect("dir");
        std::fs::write(root.join("config/.env"), "A=1\n").expect("write");
        std::fs::write(
            root.join("mcp.json"),
            r#"{"mcpServers":{"db":{"command":"uvx","env":{"DB_PASSWORD":"hunter2","MODE":"safe","API_KEY":"${API_KEY}"}}}}"#,
        )
        .expect("write");
        let findings = scan_for_secrets(root).expect("scan");
        let found = findings
            .iter()
            .map(|finding| (finding.path.as_str(), finding.reason.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[0].0, "config/.env");
        assert!(
            found[1].0 == "mcp.json" && found[1].1.contains("${DB_PASSWORD}"),
            "{found:?}"
        );
        assert_eq!(found[2], ("notes.txt", "line 2 looks like a GitHub token"));
    }
}
