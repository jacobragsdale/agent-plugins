//! Command-line entry points in the application binary.
//!
//! `agent-plugins validate|publish|search|install|list|status|uninstall|sync|...`
//! run without the window so an agent or a script can drive them. They share the crate's validator,
//! locator, identity, and installer, so a CLI publish is the same operation as
//! one from the app and authenticates the same way (ADR 0004).

use crate::agent_profiles::{AgentProfile, TargetId};
use crate::app_state::{AppState, AutoUpdateReport, BulkAction};
use crate::application::{self, RuntimeState};
use crate::catalog::CatalogComponent;
use crate::host_identity;
use crate::install::ItemStatus;
use crate::ipc_error::{IpcError, IpcErrorKind};
use crate::marketplace;
use crate::staging::{scan_for_secrets, stage_tree, zip_tree, StageRequest};
use reqwest::blocking::multipart::{Form, Part};
use reqwest::Method;
use serde::Deserialize;
use serde_json::json;
use std::io::{self, BufRead as _, Write as _};
use std::path::{Path, PathBuf};

const COMMANDS: [&str; 21] = [
    "validate",
    "publish",
    "search",
    "install",
    "list",
    "status",
    "uninstall",
    "sync",
    "withdraw",
    "hold",
    "keep",
    "share",
    "team",
    "review",
    "revoke",
    "bundle",
    "whoami",
    "remove-from-path",
    "help",
    "--help",
    "-h",
];
const MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024;

/// Exit codes a script can act on; anything else that fails is 1.
const EXIT_USAGE: i32 = 2;
const EXIT_NEEDS_APPROVAL: i32 = 3;
const EXIT_NOT_FOUND: i32 = 4;
const EXIT_OFFLINE: i32 = 5;
/// The flag every "needs approval" message names, from the CLI or the installer.
const APPROVE_FLAG: &str = "--approve-mcp";
/// How the marketplace, the catalog, and the ledger say something isn't there.
const NOT_FOUND: [&str; 4] = [
    "was not found",
    "is not in the catalog",
    "is not installed",
    "Unknown source",
];

/// The exit code for a failure message.
fn exit_code(message: &str) -> i32 {
    if message == usage()
        || message.starts_with("Unknown option")
        || message.ends_with("needs a value.")
    {
        EXIT_USAGE
    } else if message.contains(APPROVE_FLAG) || message.contains("Tier 3 approval") {
        EXIT_NEEDS_APPROVAL
    } else if NOT_FOUND.iter().any(|marker| message.contains(marker)) {
        EXIT_NOT_FOUND
    } else if IpcError::from(message.to_string()).kind == IpcErrorKind::Offline {
        EXIT_OFFLINE
    } else {
        1
    }
}

/// Runs a CLI command when the first argument names one. Returns the exit code.
pub(crate) fn maybe_run() -> Option<i32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let command = args.first()?;
    if !COMMANDS.contains(&command.as_str()) {
        // `--background` (tray::BACKGROUND_ARG) is the window's only option, and
        // an `agent-plugins://` link from the portal is the window's too.
        // Anything else, `--version` included, is a mistyped command, which
        // should say so rather than open the window and hold the terminal.
        if opens_window(command) {
            return None;
        }
        host_identity::attach_parent_console();
        eprintln!("error: unknown command `{command}`\n\n{}", usage());
        return Some(2);
    }
    host_identity::attach_parent_console();
    let code = match dispatch(command, &args[1..]) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("error: {message}");
            exit_code(&message)
        }
    };
    marketplace::flush_events();
    Some(code)
}

/// Whether a first argument that names no command is the window's.
fn opens_window(argument: &str) -> bool {
    argument == "--background" || crate::deep_link::looks_like_link(argument)
}

fn dispatch(command: &str, args: &[String]) -> Result<(), String> {
    if matches!(command, "help" | "--help" | "-h")
        || args.iter().any(|arg| arg == "--help" || arg == "-h")
    {
        print!("{}", usage());
        return Ok(());
    }
    match command {
        "whoami" => whoami(args),
        "validate" => validate(args),
        "search" => search(args),
        "publish" => publish(args),
        "install" => install(args),
        "list" => list(args),
        "status" => status(args),
        "uninstall" => uninstall(args),
        "sync" => sync(args),
        "withdraw" => withdraw(args),
        "hold" => hold(args),
        "keep" => keep(args),
        "share" => share(args),
        "team" => team(args),
        "review" => review(args),
        "revoke" => revoke(args),
        "bundle" => bundle(args),
        // Run by the uninstaller, just before it deletes this folder.
        "remove-from-path" => crate::startup::remove_cli_dir_from_path().map(|removed| {
            if removed {
                println!("Removed this folder from the user PATH.");
            }
        }),
        _ => Err(usage()),
    }
}

fn usage() -> String {
    format!(
        "Agent Plugins {}\n\n\
usage:\n  \
agent-plugins whoami [--json]\n  \
agent-plugins validate <path> | <https archive url> [--namespace <ns>] [--package-id <id>]\n  \
agent-plugins search [words...] [--json]\n  \
agent-plugins publish <path> --version <major.minor.patch> [--namespace <ns>] [--package-id <id>] [--private | --visibility <inherit|public|private>] [--tags a,b] [--changelog <text> | --changelog-file <path>] [--message <text>] [--dry-run] [--yes]\n  \
agent-plugins install <ns>/<package> | <ns>/<package>/<skill> | <ns>/<bundle> | <link> [--approve-mcp] [--replace]\n  \
agent-plugins install --local <path> [--approve-mcp]\n  \
agent-plugins list [--json]\n  \
agent-plugins status <ns>/<package> [--json]\n  \
agent-plugins uninstall <ns>/<package>[/<component>] [--force]\n  \
agent-plugins sync\n  \
agent-plugins withdraw <ns>/<package> <version> [--undo]\n  \
agent-plugins hold <ns>/<package> [--undo]\n  \
agent-plugins keep <ns>/<package>\n  \
agent-plugins share <ns>[/<id>] [--public | --private | --inherit] [--add <entry>]... [--remove <entry>]...\n  \
agent-plugins share <ns>[/<id>] --link [--reset]\n  \
agent-plugins team [<ns>]\n  \
agent-plugins team create <ns> --name <display name> [--public]\n  \
agent-plugins team add <ns> <account> [--owner]\n  \
agent-plugins team remove|leave|delete <ns> [<account>]\n  \
agent-plugins team rename <ns> --name <display name>\n  \
agent-plugins team invite <ns> [--reset]\n  \
agent-plugins team join <link>\n  \
agent-plugins review [<number> --accept [--version <v>] | <number> --decline <note>]\n  \
agent-plugins revoke <ns>/<package> [--undo] [--yes]\n  \
agent-plugins bundle <ns>/<id>\n  \
agent-plugins bundle set <ns>/<id> --name <name> [--description <text>] <ns>/<package>...\n  \
agent-plugins bundle delete <ns>/<id> [--yes]\n\n\
<path> for validate, publish, and install --local is a skill directory containing\n\
SKILL.md, a folder of skill directories (a skill pack), an MCP document (mcp.json\n\
shape), or a source tree with agent-plugins.json (choose one of several packages\n\
with --package-id). Publishing to a package you don't own sends your change to\n\
its owners as a suggestion. Publish a first version with --private to try it\n\
yourself before anyone else can see it; --dry-run checks everything and shows\n\
what would change without publishing.\n\
<entry> for share is an account, team:<ns>, or group:<name>.\n\
uninstall --force also removes a copy you edited, after saving it to backups.\n\
hold stops background updates of a package; keep stops managing it and leaves\n\
its files as they are.\n\n\
exit codes: 0 done, 1 failed, 2 usage, 3 needs --approve-mcp, 4 not found,\n\
5 the marketplace could not be reached.\n",
        marketplace::CLIENT_VERSION
    )
}

/// Positional arguments and `--option` values, as given.
struct Parsed {
    positional: Vec<String>,
    options: Vec<(&'static str, Option<String>)>,
}

impl Parsed {
    fn has(&self, name: &str) -> bool {
        self.options.iter().any(|(option, _)| *option == name)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values(name).last().copied()
    }

    fn values(&self, name: &str) -> Vec<&str> {
        self.options
            .iter()
            .filter(|(option, _)| *option == name)
            .filter_map(|(_, value)| value.as_deref())
            .collect()
    }
}

/// Reads `args`: each option in `valued` takes the next argument as its
/// value, each in `switches` stands alone, and `-y` means `--yes`.
fn parse_args(
    args: &[String],
    valued: &[&'static str],
    switches: &[&'static str],
) -> Result<Parsed, String> {
    let mut parsed = Parsed {
        positional: Vec::new(),
        options: Vec::new(),
    };
    let mut iter = args.iter();
    while let Some(given) = iter.next() {
        let arg = if given == "-y" {
            "--yes"
        } else {
            given.as_str()
        };
        if let Some(name) = valued.iter().find(|name| **name == arg) {
            let value = iter
                .next()
                .ok_or_else(|| format!("{given} needs a value."))?;
            parsed.options.push((name, Some(value.clone())));
        } else if let Some(name) = switches.iter().find(|name| **name == arg) {
            parsed.options.push((name, None));
        } else if arg.starts_with('-') && arg.len() > 1 {
            // A short option such as `-j` is not a positional either.
            return Err(format!("Unknown option {given}."));
        } else {
            parsed.positional.push(arg.to_string());
        }
    }
    Ok(parsed)
}

fn whoami(args: &[String]) -> Result<(), String> {
    let json = parse_args(args, &[], &["--json"])?.has("--json");
    let mode = marketplace::auth_mode();
    let host = host_identity::current().account;
    let me = marketplace::fetch_me()?;
    if json {
        return print_json(&json!({
            "hostAccount": host,
            "identity": mode.describe(),
            "account": me.account,
            "namespace": me.namespace,
            "namespaces": me.namespaces,
            "teams": me.teams,
            "groups": me.groups,
            "admin": me.admin,
            "suggestionsWaiting": me.suggestions_waiting,
            "reportsWaiting": me.reports_waiting,
            "unreadNotifications": me.unread_notifications,
        }));
    }
    println!("host account: {host}");
    println!("identity: {}", mode.describe());
    println!("marketplace account: {}", me.account);
    println!("namespace: {}", me.namespace);
    println!("publishes to: {}", me.namespaces.join(", "));
    let teams = me
        .teams
        .iter()
        .map(|team| {
            if team.owner {
                format!("{} (owner)", team.namespace)
            } else {
                team.namespace.clone()
            }
        })
        .collect::<Vec<_>>();
    println!("teams: {}", teams.join(", "));
    println!("groups: {}", me.groups.join(", "));
    println!("admin: {}", me.admin);
    println!("suggestions waiting: {}", me.suggestions_waiting);
    println!("reports waiting: {}", me.reports_waiting);
    println!("unread notifications: {}", me.unread_notifications);
    Ok(())
}

fn print_json(value: &serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    println!("{text}");
    Ok(())
}

/// Checks what `publish` would send, without the network: staging, the
/// manifest rules, the secret scan, the size limits, and which apps can use
/// each part.
fn validate(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &["--namespace", "--package-id"], &[])?;
    let [path] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    // An archive a source repository lists is checked as the app would fetch it.
    if path.starts_with("https://") {
        return validate_archive(path);
    }
    let path = PathBuf::from(path);
    let manifest = std::fs::read(path.join(crate::manifest::SOURCE_MANIFEST_FILE))
        .ok()
        .and_then(|bytes| crate::manifest::SourceManifest::from_slice(&bytes).ok())
        .map(|crate::manifest::SourceManifest::V2(manifest)| manifest);
    let namespace = parsed
        .value("--namespace")
        .map(str::to_string)
        .or_else(|| manifest.as_ref().map(|manifest| manifest.source.id.clone()))
        .unwrap_or_else(default_namespace);
    // Each package of a source tree is published on its own, so each is checked so.
    let package_ids = match (parsed.value("--package-id"), &manifest) {
        (Some(id), _) => vec![Some(id.to_string())],
        (None, Some(manifest)) if manifest.packages.len() > 1 => manifest
            .packages
            .iter()
            .map(|package| Some(package.id.clone()))
            .collect(),
        _ => vec![None],
    };
    let mut valid = true;
    for package_id in package_ids {
        let staging = tempfile_dir("validate")?;
        let checked = check_package(&path, &namespace, package_id.as_deref(), &staging);
        let _ = std::fs::remove_dir_all(&staging);
        valid &= checked?;
    }
    if valid {
        Ok(())
    } else {
        Err("validation failed".to_string())
    }
}

fn validate_archive(url: &str) -> Result<(), String> {
    let report = crate::source::validate_source(url)?;
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

/// Stages one package into `staging` and prints every problem with it, then
/// the apps that can use each part. Returns whether it would publish.
fn check_package(
    path: &Path,
    namespace: &str,
    package_id: Option<&str>,
    staging: &Path,
) -> Result<bool, String> {
    let request = StageRequest {
        namespace,
        package_id,
        name: None,
        description: None,
    };
    let staged = stage_tree(path, &request, staging)?;
    let root = staged.root.display().to_string();
    let report = crate::source::validate_source(&root)?;
    let secrets = scan_for_secrets(&staged.root)?;
    let archive_bytes = zip_tree(&staged.root)?.len() as u64;
    let limits = size_problems(staged.file_count, staged.total_bytes, archive_bytes);
    let valid = report.errors.is_empty()
        && report.valid_installs > 0
        && secrets.is_empty()
        && limits.is_empty();
    println!(
        "{namespace}/{}: {} file(s), {} KB ({} KB zipped){}",
        staged.package_id,
        staged.file_count,
        staged.total_bytes / 1024,
        archive_bytes / 1024,
        if valid { "" } else { ", not publishable" }
    );
    for error in &report.errors {
        println!("  error: {}: {}", error.path, error.message);
    }
    for finding in &secrets {
        println!("  secret: {}: {}", finding.path, finding.reason);
    }
    for limit in &limits {
        println!("  too big: {limit}");
    }
    let catalog = crate::catalog::read_manifest_catalog(&staged.root, "validate")?;
    let apps = tempfile_dir("validate-apps")?;
    for item in catalog.items.values() {
        for component in &item.components {
            println!("  {} {}", component_label(component), component.id);
            for (app, verdict) in compatibility(component, &staged.root, &apps) {
                match verdict {
                    Ok(()) => println!("    {app:<18} yes"),
                    Err(reason) => println!("    {app:<18} no: {reason}"),
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&apps);
    Ok(valid)
}

fn component_label(component: &CatalogComponent) -> &'static str {
    match component.kind {
        crate::catalog::CatalogComponentKind::Skill => "skill",
        crate::catalog::CatalogComponentKind::McpServer => "MCP server",
    }
}

/// Whether each app, primary ones first, can use `component`, planned against
/// an empty home under `home` so the answer doesn't depend on this computer.
fn compatibility(
    component: &CatalogComponent,
    source_root: &Path,
    home: &Path,
) -> Vec<(&'static str, Result<(), String>)> {
    let paths = crate::paths::SystemPaths {
        home: home.join("home"),
        config: home.join("config"),
        data: home.join("data"),
        local_data: home.join("local-data"),
        cache: home.join("cache"),
    };
    let context = crate::adapters::PlanningContext {
        paths: &paths,
        source_root,
    };
    TargetId::ALL
        .into_iter()
        .map(|target| {
            let profile = AgentProfile {
                target_id: target,
                enabled: true,
                scopes: vec!["user".to_string()],
                dialect_id: target.current_dialect(),
            };
            let verdict = crate::adapters::adapter(target)
                .plan(component, &profile, &context)
                .and_then(|plan| match plan.capability {
                    crate::resource::CapabilityResult::Unsupported { reason }
                    | crate::resource::CapabilityResult::Blocked { reason, .. } => Err(reason),
                    _ => Ok(()),
                });
            (target.display_name(), verdict)
        })
        .collect()
}

/// What the desktop app would refuse to download, in its own limits.
fn size_problems(files: usize, bytes: u64, archive_bytes: u64) -> Vec<String> {
    let mut problems = Vec::new();
    if files > crate::sources::MAX_SOURCE_FILES {
        problems.push(format!(
            "{files} files; a package holds at most {}",
            crate::sources::MAX_SOURCE_FILES
        ));
    }
    if bytes > crate::sources::MAX_SOURCE_BYTES {
        problems.push(format!(
            "{} MB unzipped; the limit is 50 MB",
            bytes / (1024 * 1024)
        ));
    }
    if archive_bytes > MAX_ARCHIVE_BYTES {
        problems.push(format!(
            "{} MB zipped; the limit is 50 MB",
            archive_bytes / (1024 * 1024)
        ));
    }
    problems
}

/// A namespace to validate under when none is named: the account the
/// marketplace would see (the Windows user, or a development user), the way
/// it derives a personal space.
fn default_namespace() -> String {
    let account = match marketplace::auth_mode() {
        marketplace::AuthMode::DevHeader(account) => account,
        marketplace::AuthMode::Negotiate(_) => host_identity::current().account,
    };
    let user = account.rsplit('\\').next().unwrap_or(&account);
    let slug = user
        .split('@')
        .next()
        .unwrap_or(user)
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    let slug = slug.trim_matches('-').chars().take(16).collect::<String>();
    if slug.len() < 2 {
        "me".to_string()
    } else {
        slug
    }
}

fn search(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &[], &["--json"])?;
    let query = parsed.positional.join(" ");
    let index = marketplace::fetch_index()?;
    let matches = index
        .packages
        .iter()
        .filter(|package| {
            let tags = package.tags.join(" ");
            matches_all_words(
                &[
                    &package.id,
                    &package.name,
                    &package.description,
                    &tags,
                    &package.publisher.display_name,
                ],
                &query,
            )
        })
        .collect::<Vec<_>>();
    let bundles = index
        .bundles
        .iter()
        .filter(|bundle| {
            matches_all_words(
                &[
                    &bundle.id,
                    &bundle.name,
                    &bundle.description,
                    &bundle.publisher.display_name,
                ],
                &query,
            )
        })
        .collect::<Vec<_>>();
    if parsed.has("--json") {
        return print_json(&json!({ "packages": matches, "bundles": bundles }));
    }
    if matches.is_empty() && bundles.is_empty() {
        println!("No packages match.");
        return Ok(());
    }
    if !matches.is_empty() {
        println!(
            "{:<32} {:<10} {:<18} {:>8} {:>6}  tags",
            "package", "version", "publisher", "installs", "users"
        );
    }
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
    if !bundles.is_empty() {
        println!("{:<32} {:<10} {:<18}", "bundle", "packages", "publisher");
    }
    for bundle in bundles {
        println!(
            "{:<32} {:<10} {:<18}",
            bundle.id,
            bundle.members.len(),
            truncate(&bundle.publisher.display_name, 18)
        );
        println!("    {}", truncate(&bundle.name, 100));
    }
    Ok(())
}

/// Every word of `query` appears somewhere in `fields`, ignoring case, the way
/// the portal and the window search.
fn matches_all_words(fields: &[&str], query: &str) -> bool {
    let haystack = fields.join("\n").to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| haystack.contains(word))
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
    message: Option<String>,
    /// `inherit`, `public`, or `private`; `None` leaves the package's setting.
    visibility: Option<String>,
    dry_run: bool,
    yes: bool,
}

fn parse_publish_args(args: &[String]) -> Result<PublishArgs, String> {
    let parsed = parse_args(
        args,
        &[
            "--version",
            "--namespace",
            "--package-id",
            "--tags",
            "--changelog",
            "--changelog-file",
            "--message",
            "--visibility",
        ],
        &["--yes", "--private", "--dry-run"],
    )?;
    let [path] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    let visibility = match (parsed.has("--private"), parsed.value("--visibility")) {
        (true, Some(_)) => return Err("Choose --private or --visibility, not both.".to_string()),
        (true, None) => Some("private".to_string()),
        (false, Some(value @ ("inherit" | "public" | "private"))) => Some(value.to_string()),
        (false, Some(value)) => {
            return Err(format!(
                "--visibility is inherit, public, or private, not {value}."
            ))
        }
        (false, None) => None,
    };
    let changelog = match (
        parsed.value("--changelog"),
        parsed.value("--changelog-file"),
    ) {
        (Some(_), Some(_)) => {
            return Err("Choose --changelog or --changelog-file, not both.".to_string())
        }
        (Some(text), None) => Some(text.to_string()),
        (None, Some(file)) => Some(
            std::fs::read_to_string(file)
                .map_err(|error| format!("Could not read {file}: {error}"))?
                .trim()
                .to_string(),
        ),
        (None, None) => None,
    };
    Ok(PublishArgs {
        path: PathBuf::from(path),
        version: parsed.value("--version").unwrap_or_default().to_string(),
        namespace: parsed.value("--namespace").map(str::to_string),
        package_id: parsed.value("--package-id").map(str::to_string),
        tags: parsed
            .value("--tags")
            .unwrap_or_default()
            .split(',')
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect(),
        changelog,
        message: parsed.value("--message").map(str::to_string),
        visibility,
        dry_run: parsed.has("--dry-run"),
        yes: parsed.has("--yes"),
    })
}

/// What the server says about a version it accepted.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Published {
    #[serde(default)]
    waiting_for_public_review: bool,
    #[serde(default)]
    warnings: Vec<String>,
}

/// What a dry run would change, compared with the live version.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DryRun {
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    files: Vec<DryRunFile>,
}

#[derive(Deserialize)]
struct DryRunFile {
    path: String,
    status: String,
}

fn publish(args: &[String]) -> Result<(), String> {
    let args = parse_publish_args(args)?;
    let me = marketplace::fetch_me()?;
    let namespace = args
        .namespace
        .clone()
        .unwrap_or_else(|| me.namespace.clone());
    let owned = me.admin
        || me
            .namespaces
            .iter()
            .any(|candidate| candidate == &namespace);
    if owned && args.version.is_empty() {
        return Err("--version is required (major.minor.patch).".to_string());
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
    let id = format!("{namespace}/{}", staged.package_id);
    let base = marketplace::base_url()?;
    let contents = format!(
        "{} file(s), {} KB ({} KB zipped)",
        staged.file_count,
        staged.total_bytes / 1024,
        archive.len() / 1024
    );

    if !owned {
        // Someone else's package: the change goes to its owners to decide.
        if marketplace::api(Method::GET, &format!("packages/{id}"), None).is_err() {
            return Err(format!(
                "{} may publish to {} but not to {namespace}.",
                me.account,
                me.namespaces.join(", ")
            ));
        }
        let message = args.message.or(args.changelog).ok_or_else(|| {
            "A suggestion needs --message \"<what you changed and why>\".".to_string()
        })?;
        println!("suggest a change to {id}");
        println!("  as        {}", me.account);
        println!("  contents  {contents}");
        println!("  message   {message}");
        if args.dry_run {
            println!("dry run: the checks passed and nothing was sent");
            return Ok(());
        }
        if !args.yes
            && !confirm(&format!(
                "You don't own {id}. Send this to its owners as a suggestion? [y/N] "
            ))?
        {
            return Err("Cancelled.".to_string());
        }
        let form = archive_form(archive)?.text("message", message);
        let (status, body) = upload(&format!("{base}/api/packages/{id}/suggestions"), form)?;
        if status != 201 {
            return Err(describe_problem(status, &body));
        }
        let suggestion = serde_json::from_str::<Suggestion>(&body).map_err(|error| {
            format!("The marketplace answered with an unreadable suggestion: {error}")
        })?;
        println!(
            "suggested a change to {id} (#{}); its owners decide whether to publish it",
            suggestion.id
        );
        println!("  {base}/p/{id}");
        return Ok(());
    }

    println!("publish {id} {}", args.version);
    println!("  as        {}", me.account);
    println!("  contents  {contents}");
    if let Some(visibility) = &args.visibility {
        println!("  who       {}", visibility_words(visibility));
    }
    if !args.tags.is_empty() {
        println!("  tags      {}", args.tags.join(", "));
    }
    if let Some(changelog) = &args.changelog {
        println!("  changelog {changelog}");
    }
    if !args.dry_run && !args.yes && !confirm("Publish? [y/N] ")? {
        return Err("Cancelled.".to_string());
    }
    let mut form = archive_form(archive)?
        .text("version", args.version.clone())
        .text("tags", args.tags.join(","));
    if let Some(changelog) = &args.changelog {
        form = form.text("changelog", changelog.clone());
    }
    if let Some(visibility) = &args.visibility {
        form = form.text("visibility", visibility.clone());
    }
    if args.dry_run {
        form = form.text("dryRun", "true");
    }
    let (status, body) = upload(&format!("{base}/api/packages/{id}/versions"), form)?;
    if !matches!(status, 200 | 201) {
        return Err(describe_problem(status, &body));
    }
    if args.dry_run {
        let dry_run = serde_json::from_str::<DryRun>(&body).map_err(|error| {
            format!("The marketplace answered with an unreadable dry run: {error}")
        })?;
        let unchanged = dry_run
            .files
            .iter()
            .filter(|file| file.status == "same")
            .count();
        for file in dry_run.files.iter().filter(|file| file.status != "same") {
            println!("  {:<8} {}", file.status, file.path);
        }
        if unchanged > 0 {
            println!("  {unchanged} file(s) unchanged");
        }
        for warning in &dry_run.warnings {
            println!("warning: {warning}");
        }
        println!("dry run: the checks passed and nothing was published");
        return Ok(());
    }
    let published = serde_json::from_str::<Published>(&body)
        .map_err(|error| format!("The marketplace answered with an unreadable version: {error}"))?;
    if status == 200 {
        println!(
            "{id} {} was already published with these files; nothing changed",
            args.version
        );
    } else if published.waiting_for_public_review {
        println!(
            "published {id} {}; everyone else sees it once an admin approves its MCP server",
            args.version
        );
    } else {
        println!("published {id} {}", args.version);
    }
    for warning in &published.warnings {
        println!("warning: {warning}");
    }
    println!("  {base}/p/{id}");
    Ok(())
}

fn visibility_words(visibility: &str) -> &'static str {
    match visibility {
        "private" => "only you, the space's owners, and people it is shared with",
        "public" => "everyone at the company",
        _ => "the same people as its space",
    }
}

fn archive_form(archive: Vec<u8>) -> Result<Form, String> {
    let part = Part::bytes(archive)
        .file_name("package.zip")
        .mime_str("application/zip")
        .map_err(|error| error.to_string())?;
    Ok(Form::new().part("archive", part))
}

/// Posts a package upload and returns the status and body. Uploads get a
/// longer timeout than other requests and are sent once.
fn upload(url: &str, form: Form) -> Result<(u16, String), String> {
    let client = reqwest::blocking::Client::builder()
        .use_preconfigured_tls(marketplace::tls()?)
        .timeout(std::time::Duration::from_secs(120))
        .user_agent(format!("agent-plugins/{}", marketplace::CLIENT_VERSION))
        .build()
        .map_err(|error| format!("Could not create the HTTPS client: {error}"))?;
    let response = marketplace::authorize(client.post(url), url)?
        .multipart(form)
        .send()
        .map_err(|error| marketplace::describe_error(url, &error))?;
    let status = response.status().as_u16();
    Ok((status, response.text().unwrap_or_default()))
}

fn describe_problem(status: u16, body: &str) -> String {
    let message = marketplace::problem_message(body)
        .unwrap_or_else(|| "The marketplace rejected the request.".to_string());
    let suggested = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|problem| problem["suggestedVersion"].as_str().map(str::to_string));
    // The server's own words usually name the version already.
    match suggested.filter(|version| !message.contains(version.as_str())) {
        Some(version) => format!("HTTP {status}: {message}\n  Publish {version} or higher."),
        None => format!("HTTP {status}: {message}"),
    }
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

/// What a marketplace link points at, as `/api/links/{code}` describes it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkInfo {
    kind: String,
    target: String,
    name: String,
    #[serde(default)]
    changed: bool,
}

fn install(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &["--local"], &["--approve-mcp", "--replace"])?;
    let approve_mcp = parsed.has(APPROVE_FLAG);
    if let Some(path) = parsed.value("--local") {
        if !parsed.positional.is_empty() {
            return Err(usage());
        }
        return install_local(Path::new(path), approve_mcp);
    }
    let [requested] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    let replace = parsed.has("--replace");
    // A share link is redeemed first, so what it shares is in the catalog the
    // sync below fetches.
    let target = if requested.contains("/l/") {
        let code = marketplace::link_code(requested)?;
        let link = marketplace::api_json::<LinkInfo>(Method::POST, &format!("links/{code}"), None)?;
        if link.kind == "invite" {
            return Err(format!(
                "That link invites you to the team {}. Run `agent-plugins team join {requested}` to join it.",
                link.name
            ));
        }
        println!("{} is shared with you", link.name);
        link.target
    } else {
        requested.clone()
    };
    let parts = target.split('/').collect::<Vec<_>>();
    crate::prepare_host();
    let runtime = tokio::runtime::Runtime::new().map_err(|error| error.to_string())?;
    runtime.block_on(async {
        let state = RuntimeState::new();
        let app = application::sync_app_state(&state).await?;
        // The sync also updated or removed other packages; say so.
        print_report(&app.auto_update_report, &app);
        let not_found = || format!("{target} is not in the catalog. Try `agent-plugins search`.");
        let ids = match parts.as_slice() {
            [source_id, local_id] | [source_id, local_id, _] => {
                let id = format!("{source_id}/{local_id}");
                if let Some(item) = app.items.iter().find(|item| item.id == id) {
                    let component = parts.get(2).copied();
                    let requires_approval = match component {
                        Some(component) => {
                            item.components
                                .iter()
                                .find(|candidate| candidate.id == component)
                                .ok_or_else(not_found)?
                                .requires_approval
                        }
                        None => item.requires_approval,
                    };
                    // Following a link again, or asking twice, is not a failure.
                    let installed = match component {
                        Some(component) => item.components.iter().any(|candidate| {
                            candidate.id == component && candidate.status == ItemStatus::Installed
                        }),
                        None => item.status == ItemStatus::Installed,
                    };
                    if installed {
                        println!("{target} is already installed.");
                        return Ok(());
                    }
                    if requires_approval && !approve_mcp {
                        return Err(approval_needed(&[item.name.as_str()]));
                    }
                    if item.status == ItemStatus::Conflict && !replace {
                        return Err(format!(
                            "Files or settings that Agent Plugins didn't install are in the way of {target}. Run again with --replace to back them up and replace them."
                        ));
                    }
                    let outcome = if replace {
                        application::replace_item(&state, source_id, local_id, approve_mcp, component)
                            .await?
                    } else {
                        application::install_item(&state, source_id, local_id, approve_mcp, component)
                            .await?
                    };
                    println!("installed {target} ({})", item.name);
                    print_outcome(&outcome);
                    return Ok(());
                }
                if parts.len() == 3 {
                    return Err(not_found());
                }
                app.bundles
                    .iter()
                    .find(|bundle| bundle.id == id)
                    .ok_or_else(not_found)?
                    .members
                    .clone()
            }
            // A shared space installs everything in it.
            [source_id] => app
                .items
                .iter()
                .filter(|item| item.source_id == *source_id)
                .map(|item| item.id.clone())
                .collect(),
            _ => return Err(not_found()),
        };
        install_many(&state, &app, &ids, approve_mcp).await
    })
}

fn approval_needed(names: &[&str]) -> String {
    format!(
        "{} includes a connector that runs a program on this computer. Run again with {APPROVE_FLAG} to allow it.",
        names.join(", ")
    )
}

fn print_outcome(outcome: &crate::install::OperationOutcome) {
    for path in &outcome.backup_paths {
        println!("  backed up {path}");
    }
    for warning in &outcome.warnings {
        println!("warning: {warning}");
    }
}

/// What a sync did to installed packages, one line each.
fn print_report(report: &AutoUpdateReport, app: &AppState) {
    let name = |id: &str| {
        app.items
            .iter()
            .find(|item| item.id == id)
            .map_or_else(|| id.to_string(), |item| item.name.clone())
    };
    for updated in &report.updated_items {
        let versions = match (&updated.from_version, &updated.to_version) {
            (Some(from), Some(to)) => format!(" {from} -> {to}"),
            (None, Some(to)) => format!(" to {to}"),
            _ => String::new(),
        };
        println!("updated {} ({}){versions}", updated.id, name(&updated.id));
    }
    for removed in &report.removed_items {
        println!("removed {removed}: its publisher or an admin pulled it");
    }
    for repaired in &report.repaired_items {
        println!("repaired {repaired}: put back files that were missing");
    }
    for extended in &report.extended_items {
        println!("added {extended} to newly found apps");
    }
    for failure in &report.failed_items {
        eprintln!("could not update {}: {}", failure.id, failure.message);
    }
}

/// Where a local test install's source lives: its own source key, named
/// after the folder, so it never takes a published package's place.
const LOCAL_NAMESPACE: &str = "local";

/// Installs a folder on this computer as the `local` space's package, the way
/// a published one would install, without publishing anything. Background
/// syncs leave it alone because no configured source lists it; `uninstall
/// local/<id>` removes it.
fn install_local(path: &Path, approve_mcp: bool) -> Result<(), String> {
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let staging = tempfile_dir("local")?;
    let installed = install_staged_locally(&canonical, &staging, approve_mcp);
    let _ = std::fs::remove_dir_all(&staging);
    installed
}

fn install_staged_locally(path: &Path, staging: &Path, approve_mcp: bool) -> Result<(), String> {
    crate::prepare_host();
    let paths = crate::paths::SystemPaths::from_system()?;
    crate::agent_profiles::apply_detected_defaults(&paths);
    let (item, outcome) = install_folder(&paths, path, staging, approve_mcp)?;
    println!(
        "installed {} ({}) from {}",
        item.id,
        item.name,
        path.display()
    );
    print_outcome(&outcome);
    println!("  remove it with: agent-plugins uninstall {}", item.id);
    Ok(())
}

/// The source a local folder installs from. Its key comes from the folder's
/// path, so installing the same folder again updates that install.
fn local_source(path: &Path) -> crate::source::ConfiguredSource {
    let locator = crate::locator::Locator::display_url(format!("file://{}", path.display()));
    crate::source::ConfiguredSource {
        source_key: locator.source_key(),
        source_id: LOCAL_NAMESPACE.to_string(),
        name: "This computer".to_string(),
        description: format!("Installed for testing from {}.", path.display()),
        locator,
        repository_key: None,
    }
}

fn install_folder(
    paths: &crate::paths::SystemPaths,
    path: &Path,
    staging: &Path,
    approve_mcp: bool,
) -> Result<
    (
        crate::catalog::CatalogItem,
        crate::install::OperationOutcome,
    ),
    String,
> {
    let request = StageRequest {
        namespace: LOCAL_NAMESPACE,
        package_id: None,
        name: None,
        description: None,
    };
    let staged = stage_tree(path, &request, staging)?;
    let source = local_source(path);
    let catalog = crate::catalog::read_manifest_catalog(&staged.root, &source.source_key)?;
    let item = catalog
        .items
        .values()
        .next()
        .cloned()
        .ok_or_else(|| "The folder has no package that can be installed.".to_string())?;
    let snapshot = crate::source::SourceSnapshot {
        definition: source.clone(),
        commit: "local".to_string(),
        path: staged.root.clone(),
        catalog,
    };
    let outcome = crate::install::install_item_components_approved(
        paths,
        &source,
        &snapshot,
        &item,
        approve_mcp,
        None,
    )
    .map_err(|error| {
        if error.contains("Tier 3 approval") {
            approval_needed(&[item.name.as_str()])
        } else {
            error
        }
    })?;
    Ok((item, outcome))
}

fn run<T>(work: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
    tokio::runtime::Runtime::new()
        .map_err(|error| error.to_string())?
        .block_on(work)
}

/// `<ns>/<package>` or `<ns>/<package>/<component>`, checked.
fn package_parts(target: &str) -> Result<(&str, &str, Option<&str>), String> {
    let parts = target.split('/').collect::<Vec<_>>();
    match parts.as_slice() {
        [namespace, package] => {
            marketplace::target_path(target)?;
            Ok((namespace, package, None))
        }
        [namespace, package, component] if !component.is_empty() => {
            marketplace::target_path(&format!("{namespace}/{package}"))?;
            Ok((namespace, package, Some(component)))
        }
        _ => Err(format!("Name it as <namespace>/<package>, not {target}.")),
    }
}

/// Statuses that mean the package is on this computer.
fn is_installed(status: ItemStatus) -> bool {
    !matches!(status, ItemStatus::Available | ItemStatus::Conflict)
}

fn status_word(status: ItemStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// The version on this computer, when the marketplace's current one is it.
fn installed_version(item: &crate::app_state::CatalogItemState) -> Option<&str> {
    matches!(
        item.status,
        ItemStatus::Installed | ItemStatus::Modified | ItemStatus::PartiallyInstalled
    )
    .then(|| item.marketplace.as_ref().map(|meta| meta.version.as_str()))
    .flatten()
}

fn cached_state() -> Result<AppState, String> {
    let state = RuntimeState::new();
    run(application::load_cached_app_state(&state))?.ok_or_else(|| {
        "Agent Plugins has no saved state yet. Run `agent-plugins sync`.".to_string()
    })
}

fn list(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &[], &["--json"])?;
    if !parsed.positional.is_empty() {
        return Err(usage());
    }
    let json = parsed.has("--json");
    let app = cached_state()?;
    let installed = app
        .items
        .iter()
        .filter(|item| is_installed(item.status))
        .collect::<Vec<_>>();
    if json {
        return print_json(&serde_json::Value::Array(
            installed
                .iter()
                .map(|item| {
                    json!({
                        "id": item.id,
                        "name": item.name,
                        "status": status_word(item.status),
                        "installedVersion": installed_version(item),
                        "latestVersion": item.marketplace.as_ref().map(|meta| &meta.version),
                        "held": item.held,
                    })
                })
                .collect(),
        ));
    }
    if installed.is_empty() {
        println!("Nothing is installed.");
        return Ok(());
    }
    println!("{:<32} {:<20} {:<10} name", "package", "status", "version");
    for item in installed {
        let status = if item.held {
            format!("{} (held)", status_word(item.status))
        } else {
            status_word(item.status)
        };
        println!(
            "{:<32} {:<20} {:<10} {}",
            item.id,
            status,
            installed_version(item).unwrap_or("-"),
            item.name
        );
    }
    Ok(())
}

fn status(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &[], &["--json"])?;
    let [target] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    let (namespace, package, None) = package_parts(target)? else {
        return Err(usage());
    };
    let id = format!("{namespace}/{package}");
    let view =
        marketplace::api_json::<serde_json::Value>(Method::GET, &format!("packages/{id}"), None)?;
    // `local` is this computer's copy, so only a package installed here has one.
    let local = cached_state()
        .ok()
        .and_then(|app| app.items.into_iter().find(|item| item.id == id))
        .filter(|item| is_installed(item.status));
    let versions = view["versions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|version| {
            json!({
                "version": version["version"],
                "publishedAt": version["publishedAt"],
                "publishedBy": version["publishedBy"],
                "changelog": version["changelog"],
                "withdrawn": version["yanked"].as_bool().unwrap_or(false),
                "purged": version["purged"].as_bool().unwrap_or(false),
            })
        })
        .collect::<Vec<_>>();
    let summary = json!({
        "id": id,
        "name": view["name"],
        "liveVersion": view["liveVersion"],
        "visibility": view["effective"],
        "revoked": view["revoked"].as_bool().unwrap_or(false),
        "revokedByAdmin": view["revokedByAdmin"].as_bool().unwrap_or(false),
        "review": view["publicReview"]["state"],
        "reviewNote": view["publicReview"]["note"],
        "installs": view["installs"],
        "installedBase": view["installedBase"],
        "versions": versions,
        "local": local.as_ref().map(|item| json!({
            "status": status_word(item.status),
            "installedVersion": installed_version(item),
            "held": item.held,
        })),
    });
    if parsed.has("--json") {
        return print_json(&summary);
    }
    let text = |value: &serde_json::Value| match value {
        serde_json::Value::Null => "-".to_string(),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    println!("{} ({id})", text(&summary["name"]));
    println!("  live version  {}", text(&summary["liveVersion"]));
    println!("  visible to    {}", text(&summary["visibility"]));
    if summary["revoked"] == true {
        let by = if summary["revokedByAdmin"] == true {
            "an admin"
        } else {
            "its owners"
        };
        println!("  removed       from every PC by {by}");
    }
    if !summary["review"].is_null() {
        println!("  MCP review    {}", text(&summary["review"]));
        if !summary["reviewNote"].is_null() {
            println!("  review note   {}", text(&summary["reviewNote"]));
        }
    }
    println!(
        "  installs      {} ({} using it in the last 30 days)",
        text(&summary["installs"]),
        text(&summary["installedBase"])
    );
    match &local {
        Some(item) => println!(
            "  this computer {}{}",
            status_word(item.status),
            if item.held { ", updates held" } else { "" }
        ),
        None => println!("  this computer not installed"),
    }
    for version in &versions {
        let date = text(&version["publishedAt"])
            .chars()
            .take(10)
            .collect::<String>();
        let state = if version["purged"] == true {
            "  purged"
        } else if version["withdrawn"] == true {
            "  withdrawn"
        } else {
            ""
        };
        println!("  {:<12} {date}{state}", text(&version["version"]));
    }
    Ok(())
}

fn uninstall(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &[], &["--force"])?;
    let [target] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    let (namespace, package, component) = package_parts(target)?;
    let state = RuntimeState::new();
    let outcome = run(application::uninstall_item(
        &state,
        namespace,
        package,
        component,
        parsed.has("--force"),
    ))
    .map_err(|error| {
        if error.contains(crate::executor::LOCAL_CHANGES) {
            format!("{error} Run again with --force to remove it anyway; your copy is saved to backups first.")
        } else {
            error
        }
    })?;
    println!("uninstalled {target}");
    print_outcome(&outcome);
    Ok(())
}

fn sync(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err(usage());
    }
    crate::prepare_host();
    let state = RuntimeState::new();
    let app = run(application::sync_app_state(&state))?;
    let report = &app.auto_update_report;
    print_report(report, &app);
    if let Some(message) = &app.catalog_message {
        eprintln!("{message}");
    }
    if report.updated_items.is_empty()
        && report.removed_items.is_empty()
        && report.repaired_items.is_empty()
        && report.extended_items.is_empty()
        && report.failed_items.is_empty()
    {
        println!("Everything is up to date.");
    }
    if report.failed_items.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} package(s) could not be updated.",
            report.failed_items.len()
        ))
    }
}

fn withdraw(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &[], &["--undo"])?;
    let [target, version] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    let id = item_target(target, "package")?;
    if version.is_empty()
        || !version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+'))
    {
        return Err(format!("{version} is not a version number."));
    }
    let path = format!("packages/{id}/versions/{version}/yank");
    if parsed.has("--undo") {
        marketplace::api(Method::DELETE, &path, None)?;
        println!("restored {id} {version}");
    } else {
        marketplace::api(Method::PUT, &path, None)?;
        println!("withdrew {id} {version}: PCs that have it move to the newest version left at their next check");
    }
    Ok(())
}

fn hold(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &[], &["--undo"])?;
    let [target] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    let (namespace, package, None) = package_parts(target)? else {
        return Err(usage());
    };
    let held = !parsed.has("--undo");
    let state = RuntimeState::new();
    run(application::set_held(&state, namespace, package, held))?;
    if held {
        println!("holding {target}: it no longer updates on its own; `agent-plugins install {target}` updates it when you choose");
    } else {
        println!("{target} updates on its own again");
    }
    Ok(())
}

fn keep(args: &[String]) -> Result<(), String> {
    let [target] = args else {
        return Err(usage());
    };
    let (namespace, package, None) = package_parts(target)? else {
        return Err(usage());
    };
    let state = RuntimeState::new();
    run(application::keep_my_version(&state, namespace, package))?;
    println!(
        "Agent Plugins no longer manages {target}; its files stay where they are and are yours now"
    );
    Ok(())
}

/// Installs a bundle's or a space's packages in one batch, each in its own
/// transaction, with one approval for every connector among them.
async fn install_many(
    state: &RuntimeState,
    app: &crate::app_state::AppState,
    ids: &[String],
    approve_mcp: bool,
) -> Result<(), String> {
    let plan = application::plan_items(state, ids, BulkAction::Install).await?;
    let needs_approval = plan
        .entries
        .iter()
        .filter(|entry| entry.will_run)
        .filter_map(|entry| app.items.iter().find(|item| item.id == entry.id))
        .filter(|item| item.requires_approval)
        .map(|item| item.name.as_str())
        .collect::<Vec<_>>();
    if !needs_approval.is_empty() && !approve_mcp {
        return Err(approval_needed(&needs_approval));
    }
    let result = application::run_items(state, ids, BulkAction::Install, approve_mcp).await?;
    for id in &result.completed {
        let name = app
            .items
            .iter()
            .find(|item| &item.id == id)
            .map_or(id.as_str(), |item| item.name.as_str());
        println!("installed {id} ({name})");
    }
    for path in &result.backup_paths {
        println!("  backed up {path}");
    }
    if result.completed.is_empty() && result.failures.is_empty() {
        println!("Everything in it is already installed.");
    }
    for failure in &result.failures {
        eprintln!("could not install {}: {}", failure.id, failure.message);
    }
    if result.failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {} package(s) could not be installed.",
            result.failures.len(),
            result.failures.len() + result.completed.len()
        ))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamSummary {
    namespace: String,
    display_name: String,
    visibility: String,
    role: String,
    member_count: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Team {
    namespace: String,
    display_name: String,
    visibility: String,
    role: String,
    members: Vec<TeamMember>,
    invite: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamMember {
    account: String,
    display_name: String,
    owner: bool,
}

fn team(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(
        args,
        &["--name"],
        &["--public", "--owner", "--reset", "--yes"],
    )?;
    let positional = parsed
        .positional
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let name = || {
        parsed
            .value("--name")
            .ok_or_else(|| "--name is required: the team's name as people see it.".to_string())
    };
    match positional.as_slice() {
        [] => {
            let teams = marketplace::api_json::<Vec<TeamSummary>>(Method::GET, "teams", None)?;
            if teams.is_empty() {
                println!("You are not in a team. Create one with `agent-plugins team create <name> --name \"<display name>\"`.");
            }
            for team in teams {
                println!(
                    "{:<18} {:<28} {:<7} {:<8} {} member(s)",
                    team.namespace,
                    truncate(&team.display_name, 28),
                    team.role,
                    team.visibility,
                    team.member_count
                );
            }
            Ok(())
        }
        ["create", namespace] => {
            let visibility = if parsed.has("--public") {
                "public"
            } else {
                "private"
            };
            let body = serde_json::json!({ "namespace": namespace, "displayName": name()?, "visibility": visibility });
            print_team(&marketplace::api_json(Method::POST, "teams", Some(&body))?);
            Ok(())
        }
        ["add", namespace, account] => {
            let path = format!("teams/{}/members", marketplace::namespace_path(namespace)?);
            let body = serde_json::json!({ "account": account, "owner": parsed.has("--owner") });
            print_team(&marketplace::api_json(Method::POST, &path, Some(&body))?);
            Ok(())
        }
        ["remove", namespace, account] => {
            remove_member(namespace, account)?;
            println!("removed {account} from {namespace}");
            Ok(())
        }
        ["leave", namespace] => {
            remove_member(namespace, &marketplace::fetch_me()?.account)?;
            println!("left {namespace}");
            Ok(())
        }
        ["rename", namespace] => {
            let path = format!("teams/{}", marketplace::namespace_path(namespace)?);
            let body = serde_json::json!({ "displayName": name()? });
            print_team(&marketplace::api_json(Method::PUT, &path, Some(&body))?);
            Ok(())
        }
        ["invite", namespace] => {
            let path = format!("teams/{}/invite", marketplace::namespace_path(namespace)?);
            let body = serde_json::json!({ "reset": parsed.has("--reset") });
            let answer =
                marketplace::api_json::<serde_json::Value>(Method::POST, &path, Some(&body))?;
            println!("{}", answer["invite"].as_str().unwrap_or_default());
            println!(
                "Anyone at the company who opens this link can join {namespace} and publish there."
            );
            Ok(())
        }
        ["join", link] => {
            let code = marketplace::link_code(link)?;
            let preview =
                marketplace::api_json::<LinkInfo>(Method::GET, &format!("links/{code}"), None)?;
            if preview.kind != "invite" {
                return Err(format!(
                    "That link shares {} with you; it doesn't invite you to a team. Run `agent-plugins install {link}` to install it.",
                    preview.name
                ));
            }
            let joined =
                marketplace::api_json::<LinkInfo>(Method::POST, &format!("links/{code}"), None)?;
            if joined.changed {
                println!(
                    "joined {} ({}); you can publish there now",
                    joined.name, joined.target
                );
            } else {
                println!("you are already in {} ({})", joined.name, joined.target);
            }
            Ok(())
        }
        ["delete", namespace] => {
            let path = format!("teams/{}", marketplace::namespace_path(namespace)?);
            if !parsed.has("--yes") && !confirm(&format!("Delete the team {namespace}? [y/N] "))? {
                return Err("Cancelled.".to_string());
            }
            marketplace::api(Method::DELETE, &path, None)?;
            println!("deleted the team {namespace}");
            Ok(())
        }
        [namespace] => {
            let path = format!("teams/{}", marketplace::namespace_path(namespace)?);
            print_team(&marketplace::api_json(Method::GET, &path, None)?);
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn remove_member(namespace: &str, account: &str) -> Result<(), String> {
    let path = format!(
        "teams/{}/members?account={}",
        marketplace::namespace_path(namespace)?,
        marketplace::query(account)
    );
    marketplace::api(Method::DELETE, &path, None).map(drop)
}

fn print_team(team: &Team) {
    println!(
        "{} ({}) · {} · you are {}",
        team.display_name, team.namespace, team.visibility, team.role
    );
    for member in &team.members {
        println!(
            "  {:<28} {:<28} {}",
            truncate(&member.display_name, 28),
            member.account,
            if member.owner { "owner" } else { "" }
        );
    }
    match &team.invite {
        Some(invite) => println!("  invite link: {invite}"),
        None => println!(
            "  no invite link yet: `agent-plugins team invite {}` makes one",
            team.namespace
        ),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Share {
    target: String,
    visibility: String,
    effective: String,
    #[serde(default)]
    users: Vec<SharedPerson>,
    #[serde(default)]
    teams: Vec<SharedTeam>,
    #[serde(default)]
    groups: Vec<String>,
    link: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedPerson {
    account: String,
    display_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedTeam {
    namespace: String,
    display_name: String,
}

/// A share-list entry: `team:<ns>`, `group:<name>`, or an account.
#[derive(Debug, Eq, PartialEq)]
enum ShareEntry<'a> {
    Person(&'a str),
    Team(&'a str),
    Group(&'a str),
}

fn share_entry(entry: &str) -> ShareEntry<'_> {
    if let Some(team) = entry.strip_prefix("team:") {
        ShareEntry::Team(team)
    } else if let Some(group) = entry.strip_prefix("group:") {
        ShareEntry::Group(group)
    } else {
        ShareEntry::Person(entry)
    }
}

/// The share list's three parts after adding and removing entries. Removing
/// an entry that is not there is an error, so a typo is not taken for success.
fn edit_share_lists(
    share: &Share,
    add: &[&str],
    remove: &[&str],
) -> Result<[Vec<String>; 3], String> {
    let mut lists = [
        share
            .users
            .iter()
            .map(|user| user.account.clone())
            .collect::<Vec<_>>(),
        share
            .teams
            .iter()
            .map(|team| team.namespace.clone())
            .collect(),
        share.groups.clone(),
    ];
    let slot = |entry: &str| match share_entry(entry) {
        ShareEntry::Person(value) => (0, value.to_string()),
        ShareEntry::Team(value) => (1, value.to_string()),
        ShareEntry::Group(value) => (2, value.to_string()),
    };
    for entry in add {
        let (list, value) = slot(entry);
        if !lists[list]
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&value))
        {
            lists[list].push(value);
        }
    }
    for entry in remove {
        let (list, value) = slot(entry);
        let before = lists[list].len();
        lists[list].retain(|existing| !existing.eq_ignore_ascii_case(&value));
        if lists[list].len() == before {
            return Err(format!(
                "{entry} is not on the share list of {}.",
                share.target
            ));
        }
    }
    Ok(lists)
}

fn share(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(
        args,
        &["--add", "--remove"],
        &["--public", "--private", "--inherit", "--link", "--reset"],
    )?;
    let [target] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    let path = format!("access/{}", marketplace::target_path(target)?);
    // `--link` hands out the link and changes nothing else, so a list change beside it would be dropped.
    let changes = ["--public", "--private", "--inherit", "--add", "--remove"];
    if parsed.has("--link") && changes.iter().any(|flag| parsed.has(flag)) {
        return Err(usage());
    }
    if parsed.has("--link") {
        let body = serde_json::json!({ "reset": parsed.has("--reset") });
        let answer = marketplace::api_json::<serde_json::Value>(
            Method::POST,
            &format!("{path}/link"),
            Some(&body),
        )?;
        println!("{}", answer["link"].as_str().unwrap_or_default());
        println!("Anyone at the company who opens this link can see and install {target}.");
        return Ok(());
    }
    let visibility = ["public", "private", "inherit"]
        .into_iter()
        .filter(|value| parsed.has(&format!("--{value}")))
        .collect::<Vec<_>>();
    if visibility.len() > 1 {
        return Err("Choose one of --public, --private, and --inherit.".to_string());
    }
    let add = parsed.values("--add");
    let remove = parsed.values("--remove");
    let current = marketplace::api_json::<Share>(Method::GET, &path, None)?;
    if visibility.is_empty() && add.is_empty() && remove.is_empty() {
        print_share(&current);
        return Ok(());
    }
    let [users, teams, groups] = edit_share_lists(&current, &add, &remove)?;
    let body = serde_json::json!({
        "visibility": visibility.first().copied().unwrap_or(current.visibility.as_str()),
        "users": users,
        "teams": teams,
        "groups": groups,
    });
    print_share(&marketplace::api_json(Method::PUT, &path, Some(&body))?);
    Ok(())
}

fn print_share(share: &Share) {
    println!("share {}", share.target);
    let visibility = if share.visibility == "inherit" {
        format!("same as its space ({})", share.effective)
    } else {
        share.visibility.clone()
    };
    println!("  visibility  {visibility}");
    let people = share
        .users
        .iter()
        .map(|user| format!("{} ({})", user.display_name, user.account))
        .collect::<Vec<_>>();
    println!("  people      {}", listed(&people));
    let teams = share
        .teams
        .iter()
        .map(|team| format!("{} (team:{})", team.display_name, team.namespace))
        .collect::<Vec<_>>();
    println!("  teams       {}", listed(&teams));
    if !share.groups.is_empty() {
        println!("  groups      {}", share.groups.join(", "));
    }
    match &share.link {
        Some(link) => println!("  link        {link}"),
        None => println!("  link        none; --link makes one"),
    }
}

fn listed(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_string()
    } else {
        values.join(", ")
    }
}

/// A suggested change to a package, as the marketplace reports it.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Suggestion {
    id: u64,
    target: String,
    #[serde(default)]
    base_version: Option<String>,
    #[serde(default)]
    live_version: Option<String>,
    #[serde(default)]
    message: String,
    #[serde(default)]
    suggested_by_name: String,
    #[serde(default)]
    suggested_at: String,
    #[serde(default)]
    accepted_version: Option<String>,
}

#[derive(Deserialize)]
struct Mine {
    #[serde(default)]
    suggestions: MineSuggestions,
}

#[derive(Default, Deserialize)]
struct MineSuggestions {
    #[serde(default)]
    waiting: Vec<Suggestion>,
}

fn review(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &["--version", "--decline"], &["--accept"])?;
    match parsed.positional.as_slice() {
        [] if parsed.options.is_empty() => {
            let mine = marketplace::api_json::<Mine>(Method::GET, "mine", None)?;
            if mine.suggestions.waiting.is_empty() {
                println!("No suggestions are waiting for you.");
                return Ok(());
            }
            for suggestion in &mine.suggestions.waiting {
                let date = suggestion.suggested_at.get(..10).unwrap_or_default();
                println!(
                    "#{:<5} {}  from {}, {date}",
                    suggestion.id, suggestion.target, suggestion.suggested_by_name
                );
                println!("       {}", suggestion.message);
                if suggestion.base_version != suggestion.live_version {
                    println!(
                        "       based on {}; the package has changed since (now {})",
                        suggestion.base_version.as_deref().unwrap_or("nothing"),
                        suggestion.live_version.as_deref().unwrap_or("nothing")
                    );
                }
            }
            println!(
                "Answer with `agent-plugins review <number> --accept` or `--decline \"<note>\"`."
            );
            Ok(())
        }
        [number] => {
            let id = number.trim_start_matches('#').parse::<u64>().map_err(|_| {
                format!("{number} is not a suggestion number; `agent-plugins review` lists them.")
            })?;
            let body = match (parsed.has("--accept"), parsed.value("--decline")) {
                (true, None) => {
                    serde_json::json!({ "decision": "accept", "version": parsed.value("--version") })
                }
                (false, Some(note)) => serde_json::json!({ "decision": "decline", "note": note }),
                _ => return Err("Choose --accept or --decline \"<note>\".".to_string()),
            };
            let decided = marketplace::api_json::<Suggestion>(
                Method::POST,
                &format!("suggestions/{id}"),
                Some(&body),
            )?;
            match &decided.accepted_version {
                Some(version) => println!(
                    "accepted #{id}: published {} {version}, credited to {}",
                    decided.target, decided.suggested_by_name
                ),
                None => println!(
                    "declined #{id}; {} sees your note",
                    decided.suggested_by_name
                ),
            }
            Ok(())
        }
        _ => Err(usage()),
    }
}

/// `ns/id`, checked; `what` names it in the error.
fn item_target<'a>(target: &'a str, what: &str) -> Result<&'a str, String> {
    if target.contains('/') {
        marketplace::target_path(target)
    } else {
        Err(format!("Name the {what} as <namespace>/<id>."))
    }
}

fn revoke(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &[], &["--undo", "--yes"])?;
    let [target] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    let path = format!("packages/{}/revoke", item_target(target, "package")?);
    if parsed.has("--undo") {
        marketplace::api(Method::DELETE, &path, None)?;
        println!("restored {target}: it is offered again, but PCs that removed it don't reinstall it on their own");
        return Ok(());
    }
    if !parsed.has("--yes")
        && !confirm(&format!(
            "Remove {target} from every PC that has it? [y/N] "
        ))?
    {
        return Err("Cancelled.".to_string());
    }
    marketplace::api(Method::PUT, &path, None)?;
    println!("revoked {target}: every PC that has it removes it at its next check");
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Bundle {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    hidden_members: u64,
    publisher: marketplace::IndexPublisher,
}

fn bundle(args: &[String]) -> Result<(), String> {
    let parsed = parse_args(args, &["--name", "--description"], &["--yes"])?;
    let positional = parsed
        .positional
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    match positional.as_slice() {
        ["set", target, members @ ..] => {
            let path = format!("bundles/{}", item_target(target, "bundle")?);
            let name = parsed.value("--name").ok_or_else(|| {
                "--name is required: the bundle's name as people see it.".to_string()
            })?;
            if members.is_empty() {
                return Err(
                    "List the packages in the bundle, as <namespace>/<package>.".to_string()
                );
            }
            let body = serde_json::json!({
                "name": name,
                "description": parsed.value("--description").unwrap_or_default(),
                "members": members,
            });
            print_bundle(&marketplace::api_json(Method::PUT, &path, Some(&body))?);
            Ok(())
        }
        ["delete", target] => {
            let path = format!("bundles/{}", item_target(target, "bundle")?);
            if !parsed.has("--yes") && !confirm(&format!("Delete the bundle {target}? [y/N] "))? {
                return Err("Cancelled.".to_string());
            }
            marketplace::api(Method::DELETE, &path, None)?;
            println!("deleted the bundle {target}; its packages stay installed where they are");
            Ok(())
        }
        [target] => {
            let path = format!("bundles/{}", item_target(target, "bundle")?);
            print_bundle(&marketplace::api_json(Method::GET, &path, None)?);
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn print_bundle(bundle: &Bundle) {
    println!(
        "{} ({}) by {}",
        bundle.name, bundle.id, bundle.publisher.display_name
    );
    if !bundle.description.is_empty() {
        println!("  {}", bundle.description);
    }
    println!("  packages  {}", listed(&bundle.members));
    if bundle.hidden_members > 0 {
        println!("  and {} you can't see", bundle.hidden_members);
    }
    println!("  install   agent-plugins install {}", bundle.id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_publish_arguments() {
        let args = strings(&["./skill", "--version", "1.2.0", "--tags", "a, b", "--yes"]);
        let parsed = parse_publish_args(&args).expect("parse");
        assert_eq!(parsed.version, "1.2.0");
        assert_eq!(parsed.tags, vec!["a", "b"]);
        assert!(parsed.yes);
        let suggestion =
            parse_publish_args(&strings(&["./skill", "--message", "Fix step 2", "-y"]))
                .expect("a suggestion needs no version");
        assert_eq!(suggestion.message.as_deref(), Some("Fix step 2"));
        assert!(suggestion.version.is_empty() && suggestion.yes);
        assert!(parse_publish_args(&[]).is_err());
        assert!(parse_publish_args(&strings(&["./skill", "--version"])).is_err());
        assert!(parse_publish_args(&strings(&["./skill", "--bogus"])).is_err());
    }

    #[test]
    fn help_flags_print_usage_for_every_command() {
        for command in COMMANDS {
            for flag in ["--help", "-h"] {
                assert!(
                    dispatch(command, &[flag.to_string()]).is_ok(),
                    "{command} {flag}"
                );
            }
        }
        assert!(dispatch("-h", &[]).is_ok());
    }

    #[test]
    fn parses_options_and_positionals() {
        let parsed = parse_args(
            &strings(&[
                "add",
                "data-team",
                "--owner",
                "CORP\\jane",
                "--name",
                "Data",
                "-y",
            ]),
            &["--name"],
            &["--owner", "--yes"],
        )
        .expect("parse");
        assert_eq!(
            parsed.positional,
            strings(&["add", "data-team", "CORP\\jane"])
        );
        assert!(parsed.has("--owner") && parsed.has("--yes"));
        assert_eq!(parsed.value("--name"), Some("Data"));
        assert!(parse_args(&strings(&["--name"]), &["--name"], &[]).is_err());
        assert!(parse_args(&strings(&["--nope"]), &[], &[]).is_err());
        assert_eq!(
            parse_args(&strings(&["-j"]), &[], &["--json"])
                .err()
                .as_deref(),
            Some("Unknown option -j.")
        );
        assert_eq!(
            parse_args(&strings(&["x", "-y"]), &[], &[])
                .err()
                .as_deref(),
            Some("Unknown option -y.")
        );
        let repeated = parse_args(
            &strings(&["--add", "bob", "--add", "team:platform"]),
            &["--add"],
            &[],
        )
        .expect("parse");
        assert_eq!(repeated.values("--add"), vec!["bob", "team:platform"]);
    }

    #[test]
    fn only_the_background_flag_and_links_open_the_window() {
        assert!(opens_window("--background"));
        assert!(opens_window("agent-plugins://open/jacob/review"));
        for argument in ["--version", "-v", "--json", "--background-task"] {
            assert!(!opens_window(argument), "{argument}");
        }
    }

    #[test]
    fn share_entries_add_and_remove_by_kind() {
        let share = Share {
            target: "jacob/review".to_string(),
            visibility: "private".to_string(),
            effective: "private".to_string(),
            users: vec![SharedPerson {
                account: "CORP\\alice".to_string(),
                display_name: "Alice".to_string(),
            }],
            teams: Vec::new(),
            groups: Vec::new(),
            link: None,
        };
        assert_eq!(share_entry("team:platform"), ShareEntry::Team("platform"));
        assert_eq!(share_entry("group:Finance"), ShareEntry::Group("Finance"));
        assert_eq!(share_entry("CORP\\bob"), ShareEntry::Person("CORP\\bob"));
        let [users, teams, groups] = edit_share_lists(
            &share,
            &["corp\\ALICE", "bob", "team:platform", "group:Finance"],
            &["CORP\\alice"],
        )
        .expect("edit");
        assert_eq!(users, strings(&["bob"]));
        assert_eq!(teams, strings(&["platform"]));
        assert_eq!(groups, strings(&["Finance"]));
        assert!(edit_share_lists(&share, &[], &["team:nobody"]).is_err());
    }

    #[test]
    fn bundles_and_packages_need_a_namespace() {
        assert_eq!(
            item_target("data-team/starter", "bundle"),
            Ok("data-team/starter")
        );
        assert!(item_target("starter", "bundle").is_err());
        assert!(item_target("data-team/Bad Name", "bundle").is_err());
    }

    #[test]
    fn failures_map_to_exit_codes_a_script_can_act_on() {
        assert_eq!(exit_code(&usage()), EXIT_USAGE);
        assert_eq!(exit_code("Unknown option --bogus."), EXIT_USAGE);
        assert_eq!(
            exit_code(&approval_needed(&["Weather"])),
            EXIT_NEEDS_APPROVAL
        );
        assert_eq!(
            exit_code("acme/weather contains an MCP server and requires explicit Tier 3 approval."),
            EXIT_NEEDS_APPROVAL
        );
        assert_eq!(
            exit_code("The package acme/nope was not found, or you do not have access to it."),
            EXIT_NOT_FOUND
        );
        assert_eq!(exit_code("acme/nope is not installed."), EXIT_NOT_FOUND);
        assert_eq!(
            exit_code("Could not connect to https://marketplace.test/api/index: dns error"),
            EXIT_OFFLINE
        );
        assert_eq!(
            exit_code("HTTP 409: Version 1.0.0 is already published."),
            1
        );
    }

    #[test]
    fn search_needs_every_word_somewhere() {
        let fields = [
            "acme/report-pdf",
            "Report builder",
            "Makes PDF reports.",
            "docs",
        ];
        assert!(matches_all_words(&fields, "report pdf"));
        assert!(matches_all_words(&fields, "PDF  Builder"));
        assert!(matches_all_words(&fields, ""));
        assert!(!matches_all_words(&fields, "report excel"));
    }

    #[test]
    fn publish_takes_visibility_dry_run_and_a_changelog_file() {
        let root = tempfile::tempdir().expect("root");
        let changelog = root.path().join("CHANGELOG.txt");
        std::fs::write(&changelog, "Fixed step 2.\n").expect("changelog");
        let file = changelog.display().to_string();
        let parsed = parse_publish_args(&strings(&[
            "./skill",
            "--version",
            "1.0.0",
            "--private",
            "--dry-run",
            "--changelog-file",
            &file,
        ]))
        .expect("parse");
        assert_eq!(parsed.visibility.as_deref(), Some("private"));
        assert!(parsed.dry_run);
        assert_eq!(parsed.changelog.as_deref(), Some("Fixed step 2."));
        let public =
            parse_publish_args(&strings(&["./skill", "--visibility", "public"])).expect("parse");
        assert_eq!(public.visibility.as_deref(), Some("public"));
        assert!(parse_publish_args(&strings(&["./skill", "--visibility", "team"])).is_err());
        assert!(parse_publish_args(&strings(&[
            "./skill",
            "--private",
            "--visibility",
            "public"
        ]))
        .is_err());
        assert!(parse_publish_args(&strings(&[
            "./skill",
            "--changelog",
            "a",
            "--changelog-file",
            &file
        ]))
        .is_err());
    }

    #[test]
    fn a_version_conflict_says_which_version_to_publish() {
        let body = r#"{"title":"Version 1.0.1 is lower than the live version 1.1.0.","status":409,"suggestedVersion":"1.1.1"}"#;
        let message = describe_problem(409, body);
        assert!(message.contains("lower than the live version"), "{message}");
        assert!(message.ends_with("Publish 1.1.1 or higher."), "{message}");
        let named = r#"{"title":"1.0.1 is lower than 1.1.0. Publish 1.1.1 or later.","status":409,"suggestedVersion":"1.1.1"}"#;
        assert_eq!(describe_problem(409, named).matches("1.1.1").count(), 1);
    }

    #[test]
    fn package_targets_name_a_namespace_a_package_and_maybe_a_component() {
        assert_eq!(package_parts("acme/tools"), Ok(("acme", "tools", None)));
        assert_eq!(
            package_parts("acme/tools/review"),
            Ok(("acme", "tools", Some("review")))
        );
        assert!(package_parts("tools").is_err());
        assert!(package_parts("acme/tools/").is_err());
        assert!(package_parts("acme/Bad Name").is_err());
    }

    #[test]
    fn compatibility_puts_the_primary_apps_first_and_says_why_not() {
        let root = tempfile::tempdir().expect("root");
        let skill = CatalogComponent {
            id: "notes".to_string(),
            kind: crate::catalog::CatalogComponentKind::Skill,
            source: "skills/notes".to_string(),
            source_is_directory: true,
            digest: "digest".to_string(),
            effective_name: "jacob-notes".to_string(),
            description: "Takes notes.".to_string(),
            disable_model_invocation: false,
            mcp_server: None,
        };
        let rows = compatibility(&skill, root.path(), root.path());
        assert_eq!(rows[0].0, "GitHub Copilot");
        assert!(rows
            .iter()
            .find(|(app, _)| *app == "Cursor")
            .is_some_and(|(_, verdict)| verdict.is_ok()));
        let desktop = rows
            .iter()
            .find(|(app, _)| *app == "Claude Desktop")
            .expect("row");
        assert!(desktop
            .1
            .as_ref()
            .is_err_and(|reason| reason.contains("claude.ai")));
        let server = CatalogComponent {
            kind: crate::catalog::CatalogComponentKind::McpServer,
            mcp_server: Some(crate::mcp::McpServer::Stdio {
                command: "uvx".to_string(),
                args: Vec::new(),
                env: std::collections::BTreeMap::new(),
                cwd: None,
            }),
            ..skill
        };
        let rows = compatibility(&server, root.path(), root.path());
        assert!(rows
            .iter()
            .find(|(app, _)| *app == "pi")
            .is_some_and(|(_, verdict)| verdict.is_err()));
        assert!(rows
            .iter()
            .find(|(app, _)| *app == "Claude Code")
            .is_some_and(|(_, verdict)| verdict.is_ok()));
    }

    #[test]
    fn size_limits_match_what_the_app_downloads() {
        assert!(size_problems(10, 1024, 512).is_empty());
        assert_eq!(size_problems(2_001, 1024, 512).len(), 1);
        assert_eq!(
            size_problems(10, 51 * 1024 * 1024, 51 * 1024 * 1024).len(),
            2
        );
    }

    #[test]
    fn a_local_folder_installs_under_its_own_source_and_comes_off_again() {
        use crate::agent_profiles::{set_enabled, TargetId};
        let root = tempfile::tempdir().expect("root");
        let paths = crate::paths::SystemPaths {
            home: root.path().join("home"),
            config: root.path().join("config"),
            data: root.path().join("data"),
            local_data: root.path().join("local-data"),
            cache: root.path().join("cache"),
        };
        set_enabled(&paths, TargetId::Cursor, true).expect("cursor");
        let skill = root.path().join("notes");
        std::fs::create_dir_all(&skill).expect("skill");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: notes\ndescription: Takes meeting notes.\n---\nBody\n",
        )
        .expect("skill");
        let staging = root.path().join("staging");
        let (item, _) = install_folder(&paths, &skill, &staging, false).expect("install");
        assert_eq!(item.id, "local/notes");
        let installed = paths.home.join(".agents/skills/local-notes/SKILL.md");
        assert!(installed.is_file());
        // Installing the same folder again, changed, updates that install.
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: notes\ndescription: Takes meeting notes.\n---\nBetter body\n",
        )
        .expect("edit");
        install_folder(&paths, &skill, &root.path().join("again"), false).expect("reinstall");
        assert!(std::fs::read_to_string(&installed)
            .expect("skill")
            .contains("Better body"));
        crate::install::uninstall_item_components(
            &paths,
            &local_source(&skill),
            &item.id,
            None,
            false,
        )
        .expect("uninstall");
        assert!(!installed.exists());
    }
}
