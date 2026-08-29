//! Startup preflight: declarative checks for the Windows host, authentication,
//! the marketplace server, detected agents, and dependencies.
//!
//! Every check has a stable ID, a status, a remediation, and a blocking flag.
//! The report is shown in the app and summarized in the heartbeat. See
//! `docs/preflight-reference.md`.

use crate::agent_profiles::AgentProfileState;
use crate::app_state::CatalogItemState;
use crate::host_identity::{self, JoinState};
use crate::install::ItemStatus;
use crate::locator;
use crate::marketplace::{self, AuthMode};
use crate::paths::SystemPaths;
use crate::startup::StartupReport;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::net::ToSocketAddrs as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

const PREFLIGHT_CACHE_FILE: &str = "preflight.json";
const CLOCK_WARN_SECONDS: u64 = 60;
const CLOCK_FAIL_SECONDS: u64 = 300;
const DISK_WARN_BYTES: u64 = 1024 * 1024 * 1024;
const DISK_FAIL_BYTES: u64 = 200 * 1024 * 1024;
const CATALOG_STALE_SECONDS: u64 = 24 * 60 * 60;
const PUBLISH_SKILL_NAME: &str = "official-publish";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CheckStatus {
    Ok,
    Warn,
    Fail,
    Skipped,
}

impl CheckStatus {
    fn as_str(self) -> &'static str {
        match self {
            CheckStatus::Ok => "ok",
            CheckStatus::Warn => "warn",
            CheckStatus::Fail => "fail",
            CheckStatus::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum Remediation {
    AutoFixed,
    Action { action: String },
    Manual { text: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreflightCheck {
    pub(crate) id: String,
    pub(crate) group: String,
    pub(crate) title: String,
    pub(crate) status: CheckStatus,
    pub(crate) detail: String,
    pub(crate) remediation: Option<Remediation>,
    pub(crate) blocking: bool,
    pub(crate) duration_millis: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreflightReport {
    pub(crate) started_at_epoch_seconds: u64,
    pub(crate) duration_millis: u64,
    pub(crate) blocked: bool,
    pub(crate) auth_mode: String,
    pub(crate) checks: Vec<PreflightCheck>,
}

impl PreflightReport {
    /// `{ id: status }` for the heartbeat; detail text stays on the machine.
    pub(crate) fn status_map(&self) -> BTreeMap<String, String> {
        self.checks
            .iter()
            .map(|check| (check.id.clone(), check.status.as_str().to_string()))
            .collect()
    }

    pub(crate) fn write_cache(&self, cache_base: &Path) {
        if let Ok(json) = serde_json::to_vec(self) {
            let _ = std::fs::create_dir_all(cache_base);
            let _ = std::fs::write(cache_base.join(PREFLIGHT_CACHE_FILE), json);
        }
    }

    pub(crate) fn read_cache(cache_base: &Path) -> Option<Self> {
        let bytes = std::fs::read(cache_base.join(PREFLIGHT_CACHE_FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

/// What the preflight needs from the rest of the app.
pub(crate) struct PreflightInput<'a> {
    pub(crate) paths: &'a SystemPaths,
    pub(crate) startup: Option<&'a StartupReport>,
    pub(crate) profiles: &'a [AgentProfileState],
    pub(crate) items: &'a [CatalogItemState],
    /// Age of the marketplace catalog snapshot, when one exists.
    pub(crate) catalog_age_seconds: Option<u64>,
    pub(crate) ledger_error: Option<String>,
}

/// What the preflight learned that the rest of the sync wants to reuse.
#[derive(Clone, Debug, Default)]
pub(crate) struct PreflightFindings {
    pub(crate) identity: Option<marketplace::Me>,
    pub(crate) health: Option<marketplace::Health>,
}

struct Collector {
    checks: Vec<PreflightCheck>,
}

impl Collector {
    #[allow(clippy::too_many_arguments)]
    fn push(
        &mut self,
        started: Instant,
        id: &str,
        title: &str,
        status: CheckStatus,
        detail: impl Into<String>,
        remediation: Option<Remediation>,
        blocking: bool,
    ) {
        let group = id.split('.').next().unwrap_or("host").to_string();
        self.checks.push(PreflightCheck {
            id: id.to_string(),
            group,
            title: title.to_string(),
            status,
            detail: detail.into(),
            remediation,
            blocking: blocking && status == CheckStatus::Fail,
            duration_millis: started.elapsed().as_millis() as u64,
        });
    }
}

fn manual(text: &str) -> Option<Remediation> {
    Some(Remediation::Manual {
        text: text.to_string(),
    })
}

fn action(name: &str) -> Option<Remediation> {
    Some(Remediation::Action {
        action: name.to_string(),
    })
}

pub(crate) fn run(input: &PreflightInput<'_>) -> (PreflightReport, PreflightFindings) {
    let overall = Instant::now();
    let started_at = marketplace::epoch_seconds_now();
    let mut out = Collector { checks: Vec::new() };
    let mut findings = PreflightFindings::default();
    let identity = host_identity::current();
    let auth_mode = marketplace::auth_mode();
    let base_url = locator::marketplace_base_url();

    host_checks(&mut out, input, &identity, base_url);
    let health = server_checks(&mut out, base_url);
    findings.health = health.as_ref().map(|probe| probe.health.clone());
    auth_checks(
        &mut out,
        &identity,
        &auth_mode,
        base_url,
        health.as_ref(),
        &mut findings,
    );
    catalog_check(&mut out, input.catalog_age_seconds);
    agent_checks(&mut out, input);
    dependency_checks(&mut out, input);

    let blocked = out.checks.iter().any(|check| check.blocking);
    let report = PreflightReport {
        started_at_epoch_seconds: started_at,
        duration_millis: overall.elapsed().as_millis() as u64,
        blocked,
        auth_mode: auth_mode.describe(),
        checks: out.checks,
    };
    (report, findings)
}

fn host_checks(
    out: &mut Collector,
    input: &PreflightInput<'_>,
    identity: &host_identity::HostIdentity,
    base_url: Option<&str>,
) {
    let started = Instant::now();
    let (status, detail) = if cfg!(windows) {
        let build = marketplace::os_build();
        if std::env::consts::ARCH == "x86_64" && build.contains("10.0.2") {
            (
                CheckStatus::Ok,
                format!("{build} {}", std::env::consts::ARCH),
            )
        } else {
            (
                CheckStatus::Warn,
                format!(
                    "{build} {}; Windows 11 x64 is the supported platform.",
                    std::env::consts::ARCH
                ),
            )
        }
    } else {
        (
            CheckStatus::Skipped,
            format!(
                "{} {} is a development host.",
                std::env::consts::OS,
                std::env::consts::ARCH
            ),
        )
    };
    out.push(
        started,
        "host.platform",
        "Operating system",
        status,
        detail,
        None,
        false,
    );

    let started = Instant::now();
    match &identity.join {
        JoinState::Domain(domain) => out.push(started, "host.domain", "Domain membership", CheckStatus::Ok, format!("{} on {domain}", identity.account), None, false),
        JoinState::Workgroup => out.push(
            started,
            "host.domain",
            "Domain membership",
            CheckStatus::Warn,
            format!("{} is not domain-joined; the marketplace accepts this only from a development server.", identity.account),
            manual("Join the machine to the corporate domain."),
            false,
        ),
        JoinState::NotApplicable => out.push(started, "host.domain", "Domain membership", CheckStatus::Skipped, format!("{} on a non-Windows host.", identity.account), None, false),
    }

    let started = Instant::now();
    match base_url {
        None => out.push(
            started,
            "host.serverUrl",
            "Marketplace URL",
            CheckStatus::Fail,
            "No marketplace URL is compiled into this build.",
            manual("Install a build configured for the company marketplace."),
            false,
        ),
        Some(url) => {
            let parsed = url::Url::parse(url).ok();
            let host = parsed
                .as_ref()
                .and_then(|parsed| parsed.host_str())
                .unwrap_or_default()
                .to_string();
            let https = parsed
                .as_ref()
                .is_some_and(|parsed| parsed.scheme() == "https");
            let fqdn = host.contains('.') && host.parse::<std::net::IpAddr>().is_err();
            if https && fqdn {
                out.push(
                    started,
                    "host.serverUrl",
                    "Marketplace URL",
                    CheckStatus::Ok,
                    url,
                    None,
                    false,
                );
            } else {
                out.push(
                    started,
                    "host.serverUrl",
                    "Marketplace URL",
                    CheckStatus::Fail,
                    format!("{url} must be an HTTPS URL with a fully qualified host name; Kerberos falls back to NTLM otherwise."),
                    manual("Point the build at the server's fully qualified name."),
                    false,
                );
            }

            let started = Instant::now();
            if host.is_empty() {
                out.push(
                    started,
                    "host.dns",
                    "Name resolution",
                    CheckStatus::Skipped,
                    "No host to resolve.",
                    None,
                    false,
                );
            } else {
                match (host.as_str(), 443).to_socket_addrs() {
                    Ok(mut addresses) => match addresses.next() {
                        Some(address) => out.push(
                            started,
                            "host.dns",
                            "Name resolution",
                            CheckStatus::Ok,
                            format!("{host} resolves to {}", address.ip()),
                            None,
                            false,
                        ),
                        None => out.push(
                            started,
                            "host.dns",
                            "Name resolution",
                            CheckStatus::Fail,
                            format!("{host} resolved to no addresses."),
                            manual("Check the machine's DNS configuration."),
                            false,
                        ),
                    },
                    Err(error) => out.push(
                        started,
                        "host.dns",
                        "Name resolution",
                        CheckStatus::Fail,
                        format!("{host} does not resolve: {error}"),
                        manual("Check the machine's DNS configuration."),
                        false,
                    ),
                }
            }
        }
    }

    let started = Instant::now();
    match input.startup {
        Some(report) => {
            let notes = report
                .notes
                .iter()
                .filter(|note| note.contains("proxy"))
                .cloned()
                .collect::<Vec<_>>();
            if notes.is_empty() {
                out.push(
                    started,
                    "host.proxy",
                    "Proxy configuration",
                    CheckStatus::Ok,
                    report.proxy.describe(),
                    Some(Remediation::AutoFixed),
                    false,
                );
            } else {
                out.push(
                    started,
                    "host.proxy",
                    "Proxy configuration",
                    CheckStatus::Warn,
                    notes.join(" "),
                    manual("Set HTTPS_PROXY and NO_PROXY consistently."),
                    false,
                );
            }
        }
        None => out.push(
            started,
            "host.proxy",
            "Proxy configuration",
            CheckStatus::Skipped,
            "Host preparation has not run in this process.",
            None,
            false,
        ),
    }

    let started = Instant::now();
    let mut unwritable = Vec::new();
    let mut created = false;
    for dir in home_directories(input.paths) {
        match ensure_writable(&dir) {
            Ok(true) => created = true,
            Ok(false) => {}
            Err(error) => unwritable.push(format!("{}: {error}", dir.display())),
        }
    }
    if unwritable.is_empty() {
        out.push(
            started,
            "host.homeDirs",
            "Home directories",
            CheckStatus::Ok,
            "Skill and data directories are writable.",
            created.then_some(Remediation::AutoFixed),
            false,
        );
    } else {
        out.push(
            started,
            "host.homeDirs",
            "Home directories",
            CheckStatus::Fail,
            unwritable.join("; "),
            manual("Fix the permissions on the listed directories."),
            true,
        );
    }

    let started = Instant::now();
    match host_identity::free_disk_bytes(&input.paths.home) {
        Some(free) if free < DISK_FAIL_BYTES => out.push(
            started,
            "host.disk",
            "Free disk space",
            CheckStatus::Fail,
            format!("{} MB free on the home volume.", free / (1024 * 1024)),
            manual("Free at least 200 MB."),
            false,
        ),
        Some(free) if free < DISK_WARN_BYTES => out.push(
            started,
            "host.disk",
            "Free disk space",
            CheckStatus::Warn,
            format!("{} MB free on the home volume.", free / (1024 * 1024)),
            manual("Free space on the home volume."),
            false,
        ),
        Some(free) => out.push(
            started,
            "host.disk",
            "Free disk space",
            CheckStatus::Ok,
            format!(
                "{} GB free on the home volume.",
                free / (1024 * 1024 * 1024)
            ),
            None,
            false,
        ),
        None => out.push(
            started,
            "host.disk",
            "Free disk space",
            CheckStatus::Skipped,
            "Not measured on this platform.",
            None,
            false,
        ),
    }

    let started = Instant::now();
    match host_identity::long_paths_enabled() {
        Some(true) => out.push(
            started,
            "host.longPaths",
            "Long path support",
            CheckStatus::Ok,
            "LongPathsEnabled is set.",
            None,
            false,
        ),
        Some(false) => out.push(
            started,
            "host.longPaths",
            "Long path support",
            CheckStatus::Warn,
            "LongPathsEnabled is off; deeply nested skills may fail to install.",
            manual("Ask IT to enable Win32 long paths (LongPathsEnabled=1)."),
            false,
        ),
        None => out.push(
            started,
            "host.longPaths",
            "Long path support",
            CheckStatus::Skipped,
            "Windows only.",
            None,
            false,
        ),
    }

    let started = Instant::now();
    match input.startup {
        Some(report) => {
            let notes = report
                .notes
                .iter()
                .filter(|note| note.contains("PATH"))
                .cloned()
                .collect::<Vec<_>>();
            if notes.is_empty() {
                let detail = if report.prepended_path_dirs.is_empty() {
                    "The user PATH already lists every tool directory.".to_string()
                } else {
                    format!(
                        "Published {} director{} to the user PATH.",
                        report.prepended_path_dirs.len(),
                        if report.prepended_path_dirs.len() == 1 {
                            "y"
                        } else {
                            "ies"
                        }
                    )
                };
                out.push(
                    started,
                    "host.path",
                    "User PATH",
                    CheckStatus::Ok,
                    detail,
                    (!report.prepended_path_dirs.is_empty()).then_some(Remediation::AutoFixed),
                    false,
                );
            } else {
                out.push(
                    started,
                    "host.path",
                    "User PATH",
                    CheckStatus::Warn,
                    notes.join(" "),
                    manual("Add the tool directories to the user PATH."),
                    false,
                );
            }
        }
        None => out.push(
            started,
            "host.path",
            "User PATH",
            CheckStatus::Skipped,
            "Host preparation has not run in this process.",
            None,
            false,
        ),
    }
}

fn server_checks(out: &mut Collector, base_url: Option<&str>) -> Option<marketplace::HealthProbe> {
    let started = Instant::now();
    let Some(url) = base_url else {
        out.push(
            started,
            "server.health",
            "Marketplace server",
            CheckStatus::Skipped,
            "No marketplace is configured.",
            None,
            false,
        );
        out.push(
            started,
            "host.tls",
            "TLS trust",
            CheckStatus::Skipped,
            "No marketplace is configured.",
            None,
            false,
        );
        out.push(
            started,
            "host.clock",
            "Clock",
            CheckStatus::Skipped,
            "No marketplace is configured.",
            None,
            false,
        );
        out.push(
            started,
            "server.clientVersion",
            "Client version",
            CheckStatus::Skipped,
            "No marketplace is configured.",
            None,
            false,
        );
        return None;
    };
    match marketplace::fetch_health() {
        Ok(probe) => {
            out.push(
                started,
                "server.health",
                "Marketplace server",
                CheckStatus::Ok,
                format!(
                    "{url} is serving version {} ({}).",
                    probe.health.server_version, probe.health.environment
                ),
                None,
                false,
            );
            out.push(
                started,
                "host.tls",
                "TLS trust",
                CheckStatus::Ok,
                "The server certificate chains to a trusted authority.",
                None,
                false,
            );

            let started = Instant::now();
            match probe.server_epoch_seconds {
                Some(server) => {
                    let local = marketplace::epoch_seconds_now();
                    let skew = server.abs_diff(local);
                    if skew > CLOCK_FAIL_SECONDS {
                        out.push(started, "host.clock", "Clock", CheckStatus::Fail, format!("This machine's clock differs from the server by {skew} seconds; Kerberos tolerates 300."), manual("Synchronize the clock with the domain time source."), false);
                    } else if skew > CLOCK_WARN_SECONDS {
                        out.push(
                            started,
                            "host.clock",
                            "Clock",
                            CheckStatus::Warn,
                            format!(
                                "This machine's clock differs from the server by {skew} seconds."
                            ),
                            manual("Synchronize the clock with the domain time source."),
                            false,
                        );
                    } else {
                        out.push(
                            started,
                            "host.clock",
                            "Clock",
                            CheckStatus::Ok,
                            format!("Within {skew} seconds of the server."),
                            None,
                            false,
                        );
                    }
                }
                None => out.push(
                    started,
                    "host.clock",
                    "Clock",
                    CheckStatus::Skipped,
                    "The server sent no Date header.",
                    None,
                    false,
                ),
            }

            let started = Instant::now();
            let client = marketplace::CLIENT_VERSION;
            if marketplace::version_less_than(client, &probe.health.minimum_client_version) {
                out.push(
                    started,
                    "server.clientVersion",
                    "Client version",
                    CheckStatus::Fail,
                    format!(
                        "Agent Plugins {client} is below the minimum {} the server accepts.",
                        probe.health.minimum_client_version
                    ),
                    action("update"),
                    true,
                );
            } else if marketplace::version_less_than(client, &probe.health.latest_client_version) {
                out.push(
                    started,
                    "server.clientVersion",
                    "Client version",
                    CheckStatus::Warn,
                    format!(
                        "Agent Plugins {client}; {} is available.",
                        probe.health.latest_client_version
                    ),
                    action("update"),
                    false,
                );
            } else {
                out.push(
                    started,
                    "server.clientVersion",
                    "Client version",
                    CheckStatus::Ok,
                    format!("Agent Plugins {client} is current."),
                    None,
                    false,
                );
            }
            Some(probe)
        }
        Err(error) => {
            let tls_problem =
                error.contains("certificate") || error.contains("tls") || error.contains("TLS");
            out.push(started, "server.health", "Marketplace server", CheckStatus::Fail, format!("{error} The app continues with the cached catalog; publishing and statistics are unavailable."), manual("Check the network connection and the marketplace URL."), false);
            if tls_problem {
                out.push(
                    started,
                    "host.tls",
                    "TLS trust",
                    CheckStatus::Fail,
                    error,
                    manual(
                        "Install the corporate certificate authority in the Windows trust store.",
                    ),
                    false,
                );
            } else {
                out.push(
                    started,
                    "host.tls",
                    "TLS trust",
                    CheckStatus::Skipped,
                    "The server was not reached.",
                    None,
                    false,
                );
            }
            out.push(
                started,
                "host.clock",
                "Clock",
                CheckStatus::Skipped,
                "The server was not reached.",
                None,
                false,
            );
            out.push(
                started,
                "server.clientVersion",
                "Client version",
                CheckStatus::Skipped,
                "The server was not reached.",
                None,
                false,
            );
            None
        }
    }
}

fn auth_checks(
    out: &mut Collector,
    identity: &host_identity::HostIdentity,
    auth_mode: &AuthMode,
    base_url: Option<&str>,
    health: Option<&marketplace::HealthProbe>,
    findings: &mut PreflightFindings,
) {
    let started = Instant::now();
    match (auth_mode, base_url) {
        (AuthMode::Negotiate(domain), Some(url)) => {
            let host = url::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_string))
                .unwrap_or_default();
            match host_identity::negotiate_token(&host) {
                Ok(Some(_)) => out.push(started, "auth.ticket", "Kerberos ticket", CheckStatus::Ok, format!("Obtained a service ticket for HTTP/{host} from {domain}."), None, false),
                Ok(None) => out.push(started, "auth.ticket", "Kerberos ticket", CheckStatus::Skipped, "Kerberos is not available on this platform.", None, false),
                Err(error) => out.push(started, "auth.ticket", "Kerberos ticket", CheckStatus::Fail, format!("{error} Either the domain controller is unreachable or HTTP/{host} is not registered as a service principal name."), manual("Ask the marketplace administrator to verify the SPN and keytab."), false),
            }
        }
        (AuthMode::DevHeader(account), _) => out.push(
            started,
            "auth.ticket",
            "Kerberos ticket",
            CheckStatus::Skipped,
            format!("Not domain-joined; identifying as {account} with the development header."),
            None,
            false,
        ),
        (AuthMode::Negotiate(_), None) => out.push(
            started,
            "auth.ticket",
            "Kerberos ticket",
            CheckStatus::Skipped,
            "No marketplace is configured.",
            None,
            false,
        ),
    }

    let started = Instant::now();
    if base_url.is_none() || health.is_none() {
        out.push(
            started,
            "auth.identity",
            "Signed in",
            CheckStatus::Skipped,
            "The server was not reached.",
            None,
            false,
        );
        return;
    }
    let dev_header_offered = health.is_some_and(|probe| {
        probe
            .health
            .auth_schemes
            .iter()
            .any(|scheme| scheme == "DevHeader")
    });
    match marketplace::fetch_me() {
        Ok(me) => {
            let detail = format!("{} (namespace {})", me.account, me.namespace);
            let status = match auth_mode {
                AuthMode::DevHeader(_) if !dev_header_offered => CheckStatus::Warn,
                _ => CheckStatus::Ok,
            };
            out.push(
                started,
                "auth.identity",
                "Signed in",
                status,
                detail,
                None,
                false,
            );
            findings.identity = Some(me);
        }
        Err(error) => {
            let remediation = match auth_mode {
                AuthMode::DevHeader(_) if !dev_header_offered => manual("This server accepts Windows authentication only; use a domain-joined machine."),
                _ => manual("Ask the marketplace administrator to check the account and the server's Kerberos configuration."),
            };
            out.push(
                started,
                "auth.identity",
                "Signed in",
                CheckStatus::Fail,
                format!("{error} (host account {}).", identity.account),
                remediation,
                false,
            );
        }
    }
}

fn catalog_check(out: &mut Collector, catalog_age_seconds: Option<u64>) {
    let started = Instant::now();
    match catalog_age_seconds {
        None => out.push(
            started,
            "server.catalog",
            "Marketplace catalog",
            CheckStatus::Warn,
            "No catalog snapshot has been fetched yet.",
            action("sync"),
            false,
        ),
        Some(age) if age > CATALOG_STALE_SECONDS => out.push(
            started,
            "server.catalog",
            "Marketplace catalog",
            CheckStatus::Warn,
            format!("The cached catalog is {} hours old.", age / 3600),
            action("sync"),
            false,
        ),
        Some(age) => out.push(
            started,
            "server.catalog",
            "Marketplace catalog",
            CheckStatus::Ok,
            format!("Refreshed {} minutes ago.", age / 60),
            None,
            false,
        ),
    }
}

fn agent_checks(out: &mut Collector, input: &PreflightInput<'_>) {
    let started = Instant::now();
    let detected = input
        .profiles
        .iter()
        .filter(|profile| profile.detected)
        .map(|profile| profile.display_name.clone())
        .collect::<Vec<_>>();
    if detected.is_empty() {
        out.push(
            started,
            "agents.detected",
            "Detected agents",
            CheckStatus::Warn,
            "No supported coding agent was found on this machine.",
            manual("Install Cursor, Claude Code, Codex, OpenCode, Grok Build, or GitHub Copilot, then refresh."),
            false,
        );
    } else {
        out.push(
            started,
            "agents.detected",
            "Detected agents",
            CheckStatus::Ok,
            detected.join(", "),
            None,
            false,
        );
    }

    let started = Instant::now();
    match &input.ledger_error {
        None => out.push(
            started,
            "agents.ledger",
            "Installation ledger",
            CheckStatus::Ok,
            "The ledger is readable.",
            None,
            false,
        ),
        Some(error) => out.push(
            started,
            "agents.ledger",
            "Installation ledger",
            CheckStatus::Fail,
            error.clone(),
            manual("Reset the app from the header, or restore the ledger from a backup."),
            true,
        ),
    }

    let started = Instant::now();
    let journal = input
        .paths
        .data
        .join("skill-manager")
        .join("resource-transaction.json");
    if journal.exists() {
        out.push(
            started,
            "agents.journal",
            "Recovery journal",
            CheckStatus::Warn,
            format!(
                "A transaction journal is present at {}; the next operation will recover it.",
                journal.display()
            ),
            None,
            false,
        );
    } else {
        out.push(
            started,
            "agents.journal",
            "Recovery journal",
            CheckStatus::Ok,
            "No interrupted transaction.",
            None,
            false,
        );
    }

    let started = Instant::now();
    let drifted = input
        .items
        .iter()
        .filter(|item| item.status == ItemStatus::Modified)
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    if drifted.is_empty() {
        out.push(
            started,
            "agents.drift",
            "Owned files",
            CheckStatus::Ok,
            "Every installed file matches its ledger digest.",
            None,
            false,
        );
    } else {
        out.push(
            started,
            "agents.drift",
            "Owned files",
            CheckStatus::Warn,
            format!(
                "{} package(s) were modified outside Agent Plugins: {}",
                drifted.len(),
                drifted.join(", ")
            ),
            action("showDrift"),
            false,
        );
    }
}

fn dependency_checks(out: &mut Collector, input: &PreflightInput<'_>) {
    let started = Instant::now();
    match input.startup {
        Some(report) => {
            let missing = report
                .tools
                .iter()
                .filter(|tool| tool.path.is_none())
                .map(|tool| tool.name.to_string())
                .collect::<Vec<_>>();
            if missing.is_empty() {
                let auto = report.notes.iter().any(|note| note.contains("Installed"));
                out.push(
                    started,
                    "dependencies.uv",
                    "uv and uvx",
                    CheckStatus::Ok,
                    report
                        .tools
                        .iter()
                        .filter_map(|tool| {
                            tool.path
                                .as_ref()
                                .map(|path| format!("{} at {}", tool.name, path.display()))
                        })
                        .collect::<Vec<_>>()
                        .join("; "),
                    auto.then_some(Remediation::AutoFixed),
                    false,
                );
            } else {
                out.push(
                    started,
                    "dependencies.uv",
                    "uv and uvx",
                    CheckStatus::Warn,
                    format!(
                        "Missing: {}. Skills and MCP servers that invoke them will fail.",
                        missing.join(", ")
                    ),
                    manual("Install uv from https://astral.sh/uv or ask IT for the package."),
                    false,
                );
            }
        }
        None => out.push(
            started,
            "dependencies.uv",
            "uv and uvx",
            CheckStatus::Skipped,
            "Host preparation has not run in this process.",
            None,
            false,
        ),
    }

    let started = Instant::now();
    let mcp_installed = input.items.iter().any(|item| {
        matches!(
            item.status,
            ItemStatus::Installed
                | ItemStatus::UpdateAvailable
                | ItemStatus::PartiallyInstalled
                | ItemStatus::Modified
        ) && item
            .components
            .iter()
            .any(|component| component.kind == "mcpServer")
    });
    if !mcp_installed {
        out.push(
            started,
            "dependencies.node",
            "Node.js (npx)",
            CheckStatus::Skipped,
            "No installed MCP server needs it.",
            None,
            false,
        );
    } else if which("npx").is_some() {
        out.push(
            started,
            "dependencies.node",
            "Node.js (npx)",
            CheckStatus::Ok,
            "npx resolves on PATH.",
            None,
            false,
        );
    } else {
        out.push(
            started,
            "dependencies.node",
            "Node.js (npx)",
            CheckStatus::Warn,
            "An installed MCP server may need npx, which is not on PATH.",
            manual("Install Node.js so npx is available."),
            false,
        );
    }

    let started = Instant::now();
    match std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        Some(dir) => {
            let on_path = std::env::var_os("PATH").is_some_and(|path| {
                std::env::split_paths(&path).any(|entry| same_dir(&entry, &dir))
            });
            if on_path {
                out.push(
                    started,
                    "dependencies.cli",
                    "Command line",
                    CheckStatus::Ok,
                    format!(
                        "{} is on PATH; agents can run the publish command.",
                        dir.display()
                    ),
                    None,
                    false,
                );
            } else {
                out.push(started, "dependencies.cli", "Command line", CheckStatus::Warn, format!("{} is not on PATH; the publish skill falls back to the full executable path.", dir.display()), manual("Add the Agent Plugins directory to the user PATH."), false);
            }
        }
        None => out.push(
            started,
            "dependencies.cli",
            "Command line",
            CheckStatus::Skipped,
            "The executable location is unknown.",
            None,
            false,
        ),
    }

    let started = Instant::now();
    let skill_dirs = [
        input.paths.home.join(".agents").join("skills"),
        input.paths.home.join(".claude").join("skills"),
    ];
    if skill_dirs
        .iter()
        .any(|dir| dir.join(PUBLISH_SKILL_NAME).join("SKILL.md").exists())
    {
        out.push(
            started,
            "dependencies.publishSkill",
            "Publish skill",
            CheckStatus::Ok,
            "The official publish skill is installed.",
            None,
            false,
        );
    } else {
        out.push(
            started,
            "dependencies.publishSkill",
            "Publish skill",
            CheckStatus::Warn,
            "Install the official publish skill so agents can publish to the marketplace.",
            action("installPublishSkill"),
            false,
        );
    }
}

fn home_directories(paths: &SystemPaths) -> Vec<PathBuf> {
    vec![
        paths.home.join(".agents").join("skills"),
        paths.home.join(".claude").join("skills"),
        paths.data.join("skill-manager"),
        paths.cache.join("skill-manager"),
    ]
}

/// Creates the directory when missing and proves it is writable. Returns true when it was created.
fn ensure_writable(dir: &Path) -> Result<bool, String> {
    let created = !dir.exists();
    std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    let probe = dir.join(format!(".agent-plugins-write-test-{}", std::process::id()));
    std::fs::write(&probe, b"ok").map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(&probe);
    Ok(created)
}

fn same_dir(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left == right,
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.CMD;.BAT".to_string())
            .split(';')
            .map(|ext| ext.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    for dir in std::env::split_paths(&path) {
        for extension in &extensions {
            let candidate = dir.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_status_map_uses_stable_ids() {
        let report = PreflightReport {
            started_at_epoch_seconds: 0,
            duration_millis: 0,
            blocked: false,
            auth_mode: "test".to_string(),
            checks: vec![PreflightCheck {
                id: "auth.identity".to_string(),
                group: "auth".to_string(),
                title: "Signed in".to_string(),
                status: CheckStatus::Warn,
                detail: String::new(),
                remediation: None,
                blocking: false,
                duration_millis: 1,
            }],
        };
        assert_eq!(
            report.status_map().get("auth.identity").map(String::as_str),
            Some("warn")
        );
    }

    #[test]
    fn ensure_writable_creates_and_probes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let dir = temp.path().join("nested").join("skills");
        assert!(ensure_writable(&dir).expect("writable"));
        assert!(!ensure_writable(&dir).expect("writable again"));
        assert!(std::fs::read_dir(&dir).expect("read").next().is_none());
    }
}
