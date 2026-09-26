//! Command-line entry points in the application binary.
//!
//! `agent-plugins validate|publish|search|install|share|team|review|revoke|bundle|whoami`
//! run without the window so an agent can drive them. They share the crate's validator,
//! locator, identity, and installer, so a CLI publish is the same operation as
//! one from the app and authenticates the same way (ADR 0004).

use crate::app_state::BulkAction;
use crate::application::{self, RuntimeState};
use crate::host_identity;
use crate::marketplace::{self, IndexBundle, IndexPackage};
use crate::staging::{scan_for_secrets, stage_tree, zip_tree, StageRequest};
use reqwest::blocking::multipart::{Form, Part};
use reqwest::Method;
use serde::Deserialize;
use std::io::{self, BufRead as _, Write as _};
use std::path::PathBuf;

const COMMANDS: [&str; 14] = [
    "validate",
    "publish",
    "search",
    "install",
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

/// Runs a CLI command when the first argument names one. Returns the exit code.
pub(crate) fn maybe_run() -> Option<i32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let command = args.first()?;
    if !COMMANDS.contains(&command.as_str()) {
        // Options such as `--background` are the window's, and so is an
        // `agent-plugins://` link from the portal. Anything else is a mistyped
        // command, which should say so rather than open the window.
        if command.starts_with('-') || crate::deep_link::looks_like_link(command) {
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
            1
        }
    };
    marketplace::flush_events();
    Some(code)
}

fn dispatch(command: &str, args: &[String]) -> Result<(), String> {
    if matches!(command, "help" | "--help" | "-h")
        || args.iter().any(|arg| arg == "--help" || arg == "-h")
    {
        print!("{}", usage());
        return Ok(());
    }
    match command {
        "whoami" => whoami(),
        "validate" => validate(args),
        "search" => search(args),
        "publish" => publish(args),
        "install" => install(args),
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
agent-plugins whoami\n  \
agent-plugins validate <path>\n  \
agent-plugins search [query]\n  \
agent-plugins publish <path> --version <major.minor.patch> [--namespace <ns>] [--package-id <id>] [--tags a,b] [--changelog <text>] [--message <text>] [--yes]\n  \
agent-plugins install <ns>/<package> | <ns>/<package>/<skill> | <ns>/<bundle> | <link> [--approve-mcp]\n  \
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
<path> for publish is a skill directory containing SKILL.md, a folder of skill\n\
directories (a skill pack), an MCP document (mcp.json shape), or a source tree\n\
with agent-plugins.json declaring one package. Publishing to a package you don't\n\
own sends your change to its owners as a suggestion.\n\
<entry> for share is an account, team:<ns>, or group:<name>.\n",
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
    while let Some(arg) = iter.next() {
        let arg = if arg == "-y" { "--yes" } else { arg.as_str() };
        if let Some(name) = valued.iter().find(|name| **name == arg) {
            let value = iter.next().ok_or_else(|| format!("{arg} needs a value."))?;
            parsed.options.push((name, Some(value.clone())));
        } else if let Some(name) = switches.iter().find(|name| **name == arg) {
            parsed.options.push((name, None));
        } else if arg.starts_with("--") {
            return Err(format!("Unknown option {arg}."));
        } else {
            parsed.positional.push(arg.to_string());
        }
    }
    Ok(parsed)
}

fn whoami() -> Result<(), String> {
    let mode = marketplace::auth_mode();
    println!("host account: {}", host_identity::current().account);
    println!("identity: {}", mode.describe());
    let me = marketplace::fetch_me()?;
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
    Ok(())
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
    let bundles = index
        .bundles
        .iter()
        .filter(|bundle| query.is_empty() || bundle_matches(bundle, &query))
        .collect::<Vec<_>>();
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

fn bundle_matches(bundle: &IndexBundle, query: &str) -> bool {
    bundle.id.to_lowercase().contains(query)
        || bundle.name.to_lowercase().contains(query)
        || bundle.description.to_lowercase().contains(query)
        || bundle.publisher.display_name.to_lowercase().contains(query)
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
            "--message",
        ],
        &["--yes"],
    )?;
    let [path] = parsed.positional.as_slice() else {
        return Err(usage());
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
        changelog: parsed.value("--changelog").map(str::to_string),
        message: parsed.value("--message").map(str::to_string),
        yes: parsed.has("--yes"),
    })
}

/// What the server says about a version it accepted.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Published {
    #[serde(default)]
    waiting_for_public_review: bool,
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
    if !args.tags.is_empty() {
        println!("  tags      {}", args.tags.join(", "));
    }
    if let Some(changelog) = &args.changelog {
        println!("  changelog {changelog}");
    }
    if !args.yes && !confirm("Publish? [y/N] ")? {
        return Err("Cancelled.".to_string());
    }
    let mut form = archive_form(archive)?
        .text("version", args.version.clone())
        .text("tags", args.tags.join(","));
    if let Some(changelog) = &args.changelog {
        form = form.text("changelog", changelog.clone());
    }
    let (status, body) = upload(&format!("{base}/api/packages/{id}/versions"), form)?;
    if status != 201 {
        return Err(describe_problem(status, &body));
    }
    let waiting = serde_json::from_str::<Published>(&body)
        .is_ok_and(|published| published.waiting_for_public_review);
    if waiting {
        println!(
            "published {id} {}; everyone else sees it once an admin approves its MCP server",
            args.version
        );
    } else {
        println!("published {id} {}", args.version);
    }
    println!("  {base}/p/{id}");
    Ok(())
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
    format!("HTTP {status}: {message}")
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
    let parsed = parse_args(args, &[], &["--approve-mcp"])?;
    let [requested] = parsed.positional.as_slice() else {
        return Err(usage());
    };
    let approve_mcp = parsed.has("--approve-mcp");
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
                    if requires_approval && !approve_mcp {
                        return Err(approval_needed(&[item.name.as_str()]));
                    }
                    let outcome = application::install_item(
                        &state,
                        source_id,
                        local_id,
                        approve_mcp,
                        component,
                    )
                    .await?;
                    println!("installed {target} ({})", item.name);
                    for path in outcome.backup_paths {
                        println!("  backed up {path}");
                    }
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
        "{} includes a connector that runs a program on this computer. Run again with --approve-mcp to allow it.",
        names.join(", ")
    )
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
        let repeated = parse_args(
            &strings(&["--add", "bob", "--add", "team:platform"]),
            &["--add"],
            &[],
        )
        .expect("parse");
        assert_eq!(repeated.values("--add"), vec!["bob", "team:platform"]);
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
}
