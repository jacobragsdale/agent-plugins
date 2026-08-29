//! Command-line entry points in the application binary.
//!
//! `skill-manager validate|publish|search|install|whoami` run without the
//! window so an agent can drive them. They share the crate's validator,
//! locator, identity, and installer, so a CLI publish is the same operation as
//! one from the app and authenticates the same way (ADR 0004).

use crate::application::{self, RuntimeState};
use crate::host_identity;
use crate::marketplace::{self, IndexPackage};
use crate::sources::copy_directory;
use std::collections::BTreeMap;
use std::io::{self, BufRead as _, Write as _};
use std::path::{Path, PathBuf};

const COMMANDS: [&str; 7] = [
    "validate", "publish", "search", "install", "whoami", "help", "--help",
];
const MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024;
const SECRET_SCAN_LIMIT: u64 = 2 * 1024 * 1024;

/// Runs a CLI command when the first argument names one. Returns the exit code.
pub(crate) fn maybe_run() -> Option<i32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let command = args.first()?;
    if !COMMANDS.contains(&command.as_str()) {
        return None;
    }
    host_identity::attach_parent_console();
    let code = match dispatch(command, &args[1..]) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("error: {message}");
            1
        }
    };
    marketplace::flush_events();
    Some(code)
}

fn dispatch(command: &str, args: &[String]) -> Result<(), String> {
    match command {
        "help" | "--help" => {
            print!("{}", usage());
            Ok(())
        }
        "whoami" => whoami(),
        "validate" => validate(args),
        "search" => search(args),
        "publish" => publish(args),
        "install" => install(args),
        _ => Err(usage()),
    }
}

fn usage() -> String {
    format!(
        "Agent Plugins {}\n\n\
usage:\n  \
skill-manager whoami\n  \
skill-manager validate <path>\n  \
skill-manager search [query]\n  \
skill-manager publish <path> --version <semver> [--namespace <ns>] [--package-id <id>] [--tags a,b] [--changelog <text>] [--yes]\n  \
skill-manager install <namespace>/<package> [--approve-mcp]\n\n\
<path> for publish is a skill directory containing SKILL.md, an MCP document\n\
(mcp.json shape), or a source tree with skill-manager.json declaring one package.\n",
        marketplace::CLIENT_VERSION
    )
}

fn whoami() -> Result<(), String> {
    let mode = marketplace::auth_mode();
    println!("host account: {}", host_identity::current().account);
    println!("identity: {}", mode.describe());
    match marketplace::fetch_me() {
        Ok(me) => {
            println!("marketplace account: {}", me.account);
            println!("namespace: {}", me.namespace);
            println!("publishes to: {}", me.namespaces.join(", "));
            println!("admin: {}", me.admin);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn validate(args: &[String]) -> Result<(), String> {
    let [path] = args else {
        return Err(usage());
    };
    let report = crate::source::validate_source(path)?;
    println!(
        "{}: {} valid install(s), {} catalog error(s)",
        report.source_id,
        report.valid_installs,
        report.errors.len()
    );
    for error in &report.errors {
        println!("{}: {}", error.path, error.message);
    }
    if report.errors.is_empty() && report.valid_installs > 0 {
        Ok(())
    } else {
        Err("validation failed".to_string())
    }
}

fn search(args: &[String]) -> Result<(), String> {
    let query = args.join(" ").to_lowercase();
    let index = marketplace::fetch_index()?;
    let matches = index
        .packages
        .iter()
        .filter(|package| query.is_empty() || matches_query(package, &query))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        println!("No packages match.");
        return Ok(());
    }
    println!(
        "{:<32} {:<10} {:<18} {:>8} {:>6}  tags",
        "package", "version", "publisher", "installs", "users"
    );
    for package in matches {
        println!(
            "{:<32} {:<10} {:<18} {:>8} {:>6}  {}",
            package.id,
            package.version,
            truncate(&package.publisher.display_name, 18),
            package.installs,
            package.installed_base,
            package.tags.join(",")
        );
        println!("    {}", truncate(&package.description, 100));
    }
    Ok(())
}

fn matches_query(package: &IndexPackage, query: &str) -> bool {
    package.id.to_lowercase().contains(query)
        || package.name.to_lowercase().contains(query)
        || package.description.to_lowercase().contains(query)
        || package.tags.iter().any(|tag| tag.contains(query))
        || package
            .publisher
            .display_name
            .to_lowercase()
            .contains(query)
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        value.to_string()
    } else {
        format!("{}…", value.chars().take(max - 1).collect::<String>())
    }
}

struct PublishArgs {
    path: PathBuf,
    version: String,
    namespace: Option<String>,
    package_id: Option<String>,
    tags: Vec<String>,
    changelog: Option<String>,
    yes: bool,
}

fn parse_publish_args(args: &[String]) -> Result<PublishArgs, String> {
    let mut parsed = PublishArgs {
        path: PathBuf::new(),
        version: String::new(),
        namespace: None,
        package_id: None,
        tags: Vec::new(),
        changelog: None,
        yes: false,
    };
    let mut iter = args.iter();
    let mut path = None;
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value."))
        };
        match arg.as_str() {
            "--version" => parsed.version = value("--version")?,
            "--namespace" => parsed.namespace = Some(value("--namespace")?),
            "--package-id" => parsed.package_id = Some(value("--package-id")?),
            "--tags" => {
                parsed.tags = value("--tags")?
                    .split(',')
                    .map(|tag| tag.trim().to_string())
                    .filter(|tag| !tag.is_empty())
                    .collect();
            }
            "--changelog" => parsed.changelog = Some(value("--changelog")?),
            "--yes" | "-y" => parsed.yes = true,
            other if other.starts_with("--") => return Err(format!("Unknown option {other}.")),
            other if path.is_none() => path = Some(PathBuf::from(other)),
            other => return Err(format!("Unexpected argument {other}.")),
        }
    }
    parsed.path = path.ok_or_else(usage)?;
    if parsed.version.is_empty() {
        return Err("--version is required (major.minor.patch).".to_string());
    }
    Ok(parsed)
}

/// A source tree staged for upload: `skill-manager.json` at the root and one package.
struct StagedPackage {
    root: PathBuf,
    package_id: String,
    file_count: usize,
    total_bytes: u64,
}

fn publish(args: &[String]) -> Result<(), String> {
    let args = parse_publish_args(args)?;
    let me = marketplace::fetch_me()?;
    let namespace = args
        .namespace
        .clone()
        .unwrap_or_else(|| me.namespace.clone());
    if !me
        .namespaces
        .iter()
        .any(|candidate| candidate == &namespace)
        && !me.admin
    {
        return Err(format!(
            "{} may publish to {} but not to {namespace}.",
            me.account,
            me.namespaces.join(", ")
        ));
    }
    let staging = tempfile_dir("publish")?;
    let staged = stage_tree(&args.path, &namespace, args.package_id.as_deref(), &staging)?;
    let findings = scan_for_secrets(&staged.root)?;
    if !findings.is_empty() {
        for finding in &findings {
            eprintln!("secret: {finding}");
        }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(
            "The package contains files that look like secrets. Remove them and publish again."
                .to_string(),
        );
    }
    let report = crate::source::validate_source(&staged.root.display().to_string())?;
    if !report.errors.is_empty() || report.valid_installs == 0 {
        for error in &report.errors {
            eprintln!("{}: {}", error.path, error.message);
        }
        let _ = std::fs::remove_dir_all(&staging);
        return Err("The package failed validation.".to_string());
    }
    let archive = zip_tree(&staged.root)?;
    let _ = std::fs::remove_dir_all(&staging);
    if archive.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err("The package archive is larger than the 50 MB limit.".to_string());
    }

    println!("publish {namespace}/{} {}", staged.package_id, args.version);
    println!("  as        {}", me.account);
    println!(
        "  contents  {} file(s), {} KB ({} KB zipped)",
        staged.file_count,
        staged.total_bytes / 1024,
        archive.len() / 1024
    );
    if !args.tags.is_empty() {
        println!("  tags      {}", args.tags.join(", "));
    }
    if let Some(changelog) = &args.changelog {
        println!("  changelog {changelog}");
    }
    if !args.yes && !confirm("Publish? [y/N] ")? {
        return Err("Cancelled.".to_string());
    }

    let base = crate::locator::marketplace_base_url()
        .ok_or_else(|| "No marketplace is configured.".to_string())?;
    let url = format!(
        "{base}/api/packages/{namespace}/{}/versions",
        staged.package_id
    );
    let part = reqwest::blocking::multipart::Part::bytes(archive)
        .file_name("package.zip")
        .mime_str("application/zip")
        .map_err(|error| error.to_string())?;
    let mut form = reqwest::blocking::multipart::Form::new()
        .part("archive", part)
        .text("version", args.version.clone())
        .text("tags", args.tags.join(","));
    if let Some(changelog) = &args.changelog {
        form = form.text("changelog", changelog.clone());
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .user_agent(format!("agent-plugins/{}", marketplace::CLIENT_VERSION))
        .build()
        .map_err(|error| format!("Could not create the HTTPS client: {error}"))?;
    let response = marketplace::authorize(client.post(&url), &url)?
        .multipart(form)
        .send()
        .map_err(|error| marketplace::describe_error(&url, &error))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if status.as_u16() == 201 {
        println!(
            "published {namespace}/{} {}",
            staged.package_id, args.version
        );
        println!("  {base}/api/packages/{namespace}/{}", staged.package_id);
        return Ok(());
    }
    Err(describe_problem(status.as_u16(), &body))
}

fn describe_problem(status: u16, body: &str) -> String {
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let title = parsed
        .as_ref()
        .and_then(|value| value.get("title"))
        .and_then(|value| value.as_str())
        .unwrap_or("The marketplace rejected the publish.");
    let mut message = format!("HTTP {status}: {title}");
    if let Some(errors) = parsed
        .as_ref()
        .and_then(|value| value.get("errors"))
        .and_then(|value| value.as_array())
    {
        for error in errors {
            let path = error.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let text = error.get("message").and_then(|v| v.as_str()).unwrap_or("");
            message.push_str(&format!("\n  {path}: {text}"));
        }
    }
    message
}

fn confirm(prompt: &str) -> Result<bool, String> {
    print!("{prompt}");
    io::stdout().flush().map_err(|error| error.to_string())?;
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|error| error.to_string())?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes"))
}

fn tempfile_dir(label: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!(
        "agent-plugins-{label}-{}-{}",
        std::process::id(),
        marketplace::epoch_seconds_now()
    ));
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Could not create {}: {error}", dir.display()))?;
    Ok(dir)
}

/// Builds a one-package source tree from a skill directory, an MCP document, or
/// an existing source tree.
fn stage_tree(
    input: &Path,
    namespace: &str,
    package_id: Option<&str>,
    staging: &Path,
) -> Result<StagedPackage, String> {
    let input = input
        .canonicalize()
        .map_err(|error| format!("{}: {error}", input.display()))?;
    let root = staging.join("source");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
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
        if let Some(requested) = package_id {
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
        let id = package_id.map(str::to_string).unwrap_or_else(|| {
            name.strip_prefix(&format!("{namespace}-"))
                .unwrap_or(&name)
                .to_string()
        });
        let skill_dir = root.join("skills").join(&id);
        std::fs::create_dir_all(skill_dir.parent().unwrap_or(&root))
            .map_err(|error| error.to_string())?;
        copy_directory(&input, &skill_dir)?;
        if name != id {
            rewrite_skill_name(&skill_dir.join("SKILL.md"), &id)?;
        }
        write_manifest(
            &root,
            namespace,
            &id,
            &title_case(&id),
            &description,
            "skill",
            &format!("skills/{id}"),
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
        let id = package_id.map(str::to_string).unwrap_or(stem);
        std::fs::create_dir_all(root.join("mcp")).map_err(|error| error.to_string())?;
        std::fs::copy(&input, root.join("mcp").join(format!("{id}.json")))
            .map_err(|error| error.to_string())?;
        write_manifest(
            &root,
            namespace,
            &id,
            &title_case(&id),
            &format!("{} MCP server.", title_case(&id)),
            "mcpServer",
            &format!("mcp/{id}.json"),
        )?;
        id
    } else {
        return Err(format!(
            "{} is not a skill directory (SKILL.md), an MCP document (.json), or a source tree (skill-manager.json).",
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

fn write_manifest(
    root: &Path,
    namespace: &str,
    package_id: &str,
    name: &str,
    description: &str,
    kind: &str,
    component_path: &str,
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
            "components": [{ "kind": kind, "path": component_path }],
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
        if in_frontmatter && !replaced && line.trim_start().starts_with("name:") {
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

/// Refuses obvious credentials: by file name, extension, or content marker.
fn scan_for_secrets(root: &Path) -> Result<Vec<String>, String> {
    let mut findings = Vec::new();
    for path in walk(root)? {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .display()
            .to_string();
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if SECRET_FILE_NAMES.contains(&file_name) || file_name.starts_with(".env.") {
            findings.push(format!("{relative}: credential file name"));
            continue;
        }
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| SECRET_EXTENSIONS.contains(&ext.to_lowercase().as_str()))
        {
            findings.push(format!("{relative}: credential file extension"));
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
                findings.push(format!("{relative}: contains {marker}"));
                break;
            }
        }
    }
    Ok(findings)
}

/// Deterministic zip of the tree: sorted entries, forward slashes, fixed timestamps.
fn zip_tree(root: &Path) -> Result<Vec<u8>, String> {
    use std::io::Cursor;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let base_options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    for path in walk(root)? {
        let relative = path
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let options = unix_permissions(&path, base_options)?;
        writer
            .start_file(relative, options)
            .map_err(|error| error.to_string())?;
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        writer
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
    }
    let cursor = writer.finish().map_err(|error| error.to_string())?;
    Ok(cursor.into_inner())
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

fn install(args: &[String]) -> Result<(), String> {
    let mut id = None;
    let mut approve_mcp = false;
    for arg in args {
        match arg.as_str() {
            "--approve-mcp" => approve_mcp = true,
            other if other.starts_with("--") => return Err(format!("Unknown option {other}.")),
            other if id.is_none() => id = Some(other.to_string()),
            other => return Err(format!("Unexpected argument {other}.")),
        }
    }
    let id = id.ok_or_else(usage)?;
    let (source_id, local_id) = id
        .split_once('/')
        .ok_or_else(|| "Name the package as <namespace>/<package>.".to_string())?;
    crate::prepare_host();
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let state = RuntimeState::new()?;
        let app = application::sync_app_state(&state).await?;
        let item =
            app.items.iter().find(|item| item.id == id).ok_or_else(|| {
                format!("{id} is not in the catalog. Try `skill-manager search`.")
            })?;
        let outcome =
            application::install_item(&state, source_id, local_id, approve_mcp, None).await?;
        println!("installed {} ({})", item.id, item.name);
        for path in outcome.backup_paths {
            println!("  backed up {path}");
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_a_bare_skill_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let skill = temp.path().join("review");
        std::fs::create_dir_all(&skill).expect("skill dir");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: jacob-review\ndescription: Reviews a change.\n---\n\n# Review\n",
        )
        .expect("write");
        let staging = temp.path().join("staging");
        let staged = stage_tree(&skill, "jacob", None, &staging).expect("stage");
        assert_eq!(staged.package_id, "review");
        let manifest =
            std::fs::read_to_string(staged.root.join("skill-manager.json")).expect("manifest");
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
    fn refuses_secrets() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("notes.txt"), "token ghp_abcdef\n").expect("write");
        std::fs::write(temp.path().join("ok.txt"), "AKIAnotakey\n").expect("write");
        let findings = scan_for_secrets(temp.path()).expect("scan");
        assert_eq!(findings.len(), 1);
        assert!(findings[0].starts_with("notes.txt"));
    }

    #[test]
    fn parses_publish_arguments() {
        let args = ["./skill", "--version", "1.2.0", "--tags", "a, b", "--yes"]
            .iter()
            .map(|arg| arg.to_string())
            .collect::<Vec<_>>();
        let parsed = parse_publish_args(&args).expect("parse");
        assert_eq!(parsed.version, "1.2.0");
        assert_eq!(parsed.tags, vec!["a", "b"]);
        assert!(parsed.yes);
        assert!(parse_publish_args(&["./skill".to_string()]).is_err());
    }
}
