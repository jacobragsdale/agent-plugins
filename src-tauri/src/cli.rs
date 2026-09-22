//! Command-line entry points in the application binary.
//!
//! `agent-plugins validate|publish|search|install|access|whoami` run without the
//! window so an agent can drive them. They share the crate's validator,
//! locator, identity, and installer, so a CLI publish is the same operation as
//! one from the app and authenticates the same way (ADR 0004).

use crate::application::{self, RuntimeState};
use crate::host_identity;
use crate::marketplace::{self, IndexPackage};
use crate::staging::{scan_for_secrets, stage_tree, zip_tree, StageRequest};
use std::io::{self, BufRead as _, Write as _};
use std::path::PathBuf;

const COMMANDS: [&str; 8] = [
    "validate", "publish", "search", "install", "access", "whoami", "help", "--help",
];
const MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024;

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
        "access" => access(args),
        _ => Err(usage()),
    }
}

fn usage() -> String {
    format!(
        "Agent Plugins {}\n\n\
usage:\n  \
agent-plugins whoami\n  \
agent-plugins validate <path>\n  \
agent-plugins search [query]\n  \
agent-plugins publish <path> --version <semver> [--namespace <ns>] [--package-id <id>] [--tags a,b] [--changelog <text>] [--yes]\n  \
agent-plugins install <namespace>/<package> [--approve-mcp]\n  \
agent-plugins access <namespace>[/<package>] [--user <account>]... [--group <name>]... [--public]\n\n\
<path> for publish is a skill directory containing SKILL.md, an MCP document\n\
(mcp.json shape), or a source tree with agent-plugins.json declaring one package.\n",
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
            println!("groups: {}", me.groups.join(", "));
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
    let request = StageRequest {
        namespace: &namespace,
        package_id: args.package_id.as_deref(),
        name: None,
        description: None,
    };
    let staged = stage_tree(&args.path, &request, &staging)?;
    let findings = scan_for_secrets(&staged.root)?;
    if !findings.is_empty() {
        for finding in &findings {
            eprintln!("secret: {}: {}", finding.path, finding.reason);
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
        let pending = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|published| published["reviewState"].as_str().map(str::to_owned))
            .is_some_and(|state| state == "pending");
        if pending {
            println!(
                "submitted {namespace}/{} {} for review; it goes live when an admin approves it",
                staged.package_id, args.version
            );
        } else {
            println!(
                "published {namespace}/{} {}",
                staged.package_id, args.version
            );
        }
        println!("  {base}/p/{namespace}/{}", staged.package_id);
        return Ok(());
    }
    Err(describe_problem(status.as_u16(), &body))
}

/// The allowlist for a namespace or package, as `/api/access` returns it.
#[derive(serde::Deserialize)]
struct AccessDocument {
    target: String,
    #[serde(default)]
    users: Vec<String>,
    #[serde(default)]
    groups: Vec<String>,
}

struct AccessArgs {
    target: String,
    users: Vec<String>,
    groups: Vec<String>,
    public: bool,
}

fn parse_access_args(args: &[String]) -> Result<AccessArgs, String> {
    let mut parsed = AccessArgs {
        target: String::new(),
        users: Vec::new(),
        groups: Vec::new(),
        public: false,
    };
    let mut iter = args.iter();
    let mut target = None;
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value."))
        };
        match arg.as_str() {
            "--user" => parsed.users.push(value("--user")?),
            "--group" => parsed.groups.push(value("--group")?),
            "--public" => parsed.public = true,
            other if other.starts_with("--") => return Err(format!("Unknown option {other}.")),
            other if target.is_none() => target = Some(other.to_string()),
            other => return Err(format!("Unexpected argument {other}.")),
        }
    }
    parsed.target = target.ok_or_else(usage)?;
    if parsed.public && !(parsed.users.is_empty() && parsed.groups.is_empty()) {
        return Err("--public cannot be combined with --user or --group.".to_string());
    }
    Ok(parsed)
}

/// Shows or replaces who may see and install a namespace or package. Without
/// options it prints the current list; `--user`/`--group` replace it; `--public`
/// clears it.
fn access(args: &[String]) -> Result<(), String> {
    let args = parse_access_args(args)?;
    let url = format!("{}/api/access/{}", marketplace::base_url()?, args.target);
    let client = marketplace::client()?;
    let request = if args.public || !args.users.is_empty() || !args.groups.is_empty() {
        client
            .put(&url)
            .json(&serde_json::json!({ "users": args.users, "groups": args.groups }))
    } else {
        client.get(&url)
    };
    let response = marketplace::authorize(request, &url)?
        .send()
        .map_err(|error| marketplace::describe_error(&url, &error))?;
    let status = response.status();
    let body = response.text().unwrap_or_default();
    if !status.is_success() {
        return Err(describe_problem(status.as_u16(), &body));
    }
    let document = serde_json::from_str::<AccessDocument>(&body)
        .map_err(|error| format!("{url} returned an unreadable access document: {error}"))?;
    println!("access {}", document.target);
    if document.users.is_empty() && document.groups.is_empty() {
        println!("  public");
    } else {
        println!("  users:  {}", document.users.join(", "));
        println!("  groups: {}", document.groups.join(", "));
    }
    Ok(())
}

fn describe_problem(status: u16, body: &str) -> String {
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let title = parsed
        .as_ref()
        .and_then(|value| value.get("title"))
        .and_then(|value| value.as_str())
        .unwrap_or("The marketplace rejected the request.");
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
                format!("{id} is not in the catalog. Try `agent-plugins search`.")
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

    #[test]
    fn parses_access_arguments() {
        let args = [
            "jacob/review",
            "--group",
            "Platform Team",
            "--user",
            "CORP\\jane",
            "--user",
            "bob",
        ]
        .iter()
        .map(|arg| arg.to_string())
        .collect::<Vec<_>>();
        let parsed = parse_access_args(&args).expect("parse");
        assert_eq!(parsed.target, "jacob/review");
        assert_eq!(parsed.groups, vec!["Platform Team"]);
        assert_eq!(parsed.users, vec!["CORP\\jane", "bob"]);
        assert!(!parsed.public);
        let shown = parse_access_args(&["jacob".to_string()]).expect("parse");
        assert!(shown.users.is_empty() && shown.groups.is_empty() && !shown.public);
        assert!(parse_access_args(&[]).is_err());
        assert!(
            parse_access_args(&["jacob".to_string(), "--public".to_string()])
                .expect("parse")
                .public
        );
        assert!(parse_access_args(&[
            "jacob".to_string(),
            "--public".to_string(),
            "--user".to_string(),
            "bob".to_string()
        ])
        .is_err());
    }
}
