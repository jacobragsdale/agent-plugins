//! Marketplace server client: identity headers, health, index, and events.
//!
//! Every request to a marketplace URL carries the caller's identity. On a
//! domain-joined Windows host that is a Kerberos `Negotiate` token for
//! `HTTP/<server host>`; otherwise the `X-Dev-User` header, which only a
//! Development server trusts. See ADR 0004.

use crate::host_identity::{self, JoinState};
use crate::locator::{self, is_marketplace_url};
use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const DEV_USER_ENV: &str = "AGENT_PLUGINS_DEV_USER";
/// Comma-separated groups sent as `X-Dev-Groups` with the development header,
/// so team rules can be exercised where there is no domain.
pub(crate) const DEV_GROUPS_ENV: &str = "AGENT_PLUGINS_DEV_GROUPS";
const INDEX_CACHE_FILE: &str = "marketplace-index.json";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A failed request is tried at most this many more times.
const RETRIES: u64 = 2;
/// The longest a retry waits, even when the server's `Retry-After` asks for more.
const MAX_RETRY_WAIT: Duration = Duration::from_secs(10);
/// Opening words of the message for a request that never reached the server.
pub(crate) const CONNECT_FAILURE: &str = "Could not connect to";
/// Closing words of the message for a request the server did not answer in time.
pub(crate) const TIMED_OUT: &str = "timed out";
const OUTBOX_FILE: &str = "events-outbox.json";
const OUTBOX_MAX_EVENTS: usize = 1000;
const OUTBOX_MAX_AGE_SECONDS: u64 = 7 * 86_400;
/// The server accepts at most this many events in one request.
const EVENTS_PER_BATCH: usize = 500;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Health {
    pub(crate) server_version: String,
    pub(crate) minimum_client_version: String,
    pub(crate) latest_client_version: String,
    pub(crate) environment: String,
    #[serde(default)]
    pub(crate) auth_schemes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Me {
    pub(crate) account: String,
    pub(crate) namespace: String,
    pub(crate) display_name: String,
    #[serde(default)]
    pub(crate) namespaces: Vec<String>,
    #[serde(default)]
    pub(crate) admin: bool,
    #[serde(default)]
    pub(crate) groups: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexPublisher {
    pub(crate) account: String,
    pub(crate) display_name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexPackage {
    pub(crate) id: String,
    pub(crate) namespace: String,
    pub(crate) package_id: String,
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) version: String,
    pub(crate) publisher: IndexPublisher,
    pub(crate) lane: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) component_kinds: Vec<String>,
    pub(crate) published_at: String,
    #[serde(default)]
    pub(crate) installs: u64,
    #[serde(default)]
    pub(crate) installed_base: u64,
    /// The package or its namespace has an access list; the caller is on it.
    #[serde(default)]
    pub(crate) restricted: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Index {
    #[serde(default)]
    pub(crate) generated_at: String,
    #[serde(default)]
    pub(crate) packages: Vec<IndexPackage>,
}

impl Index {
    pub(crate) fn package(&self, canonical_id: &str) -> Option<&IndexPackage> {
        self.packages
            .iter()
            .find(|package| package.id == canonical_id)
    }
}

/// How the client identifies itself to the marketplace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AuthMode {
    /// Kerberos through SSPI; the value is the domain name.
    Negotiate(String),
    /// `X-Dev-User`, trusted only by a Development server.
    DevHeader(String),
}

impl AuthMode {
    pub(crate) fn describe(&self) -> String {
        match self {
            AuthMode::Negotiate(domain) => format!("Windows authentication ({domain})"),
            AuthMode::DevHeader(account) => format!("development header as {account}"),
        }
    }
}

/// Picks the identity mechanism for this host. The `AGENT_PLUGINS_DEV_USER`
/// environment variable forces the development header with that account.
pub(crate) fn auth_mode() -> AuthMode {
    if let Ok(account) = std::env::var(DEV_USER_ENV) {
        if !account.trim().is_empty() {
            return AuthMode::DevHeader(account.trim().to_string());
        }
    }
    let identity = host_identity::current();
    match identity.join {
        JoinState::Domain(domain) => AuthMode::Negotiate(domain),
        JoinState::Workgroup | JoinState::NotApplicable => AuthMode::DevHeader(identity.account),
    }
}

/// Headers that identify the caller, or none when `url` is not the marketplace.
pub(crate) fn auth_headers(url: &str) -> Result<Vec<(&'static str, String)>, String> {
    if !is_marketplace_url(url) {
        return Ok(Vec::new());
    }
    match auth_mode() {
        AuthMode::DevHeader(account) => {
            let mut headers = vec![("X-Dev-User", account)];
            if let Ok(groups) = std::env::var(DEV_GROUPS_ENV) {
                if !groups.trim().is_empty() {
                    headers.push(("X-Dev-Groups", groups.trim().to_string()));
                }
            }
            Ok(headers)
        }
        AuthMode::Negotiate(_) => {
            let host = url::Url::parse(url)
                .ok()
                .and_then(|parsed| parsed.host_str().map(str::to_string))
                .ok_or_else(|| format!("{url} has no host."))?;
            match host_identity::negotiate_token(&host)? {
                Some(token) => Ok(vec![("Authorization", format!("Negotiate {token}"))]),
                None => Ok(vec![("X-Dev-User", host_identity::current().account)]),
            }
        }
    }
}

/// Attaches identity headers to a request for `url`.
pub(crate) fn authorize(builder: RequestBuilder, url: &str) -> Result<RequestBuilder, String> {
    let mut builder = builder;
    for (name, value) in auth_headers(url)? {
        builder = builder.header(name, value);
    }
    Ok(builder)
}

pub(crate) fn client() -> Result<Client, String> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .user_agent(format!("agent-plugins/{CLIENT_VERSION}"))
        .build()
        .map_err(|error| format!("Could not create the HTTPS client: {error}"))
}

/// Sends a request, trying twice more when the server was not reached, timed
/// out, failed with 5xx, or answered 429. The wait doubles from one second and
/// honours `Retry-After` up to ten seconds. `request` builds each attempt anew,
/// so every attempt carries a fresh identity token.
pub(crate) fn send(
    url: &str,
    request: impl Fn() -> Result<RequestBuilder, String>,
) -> Result<Response, String> {
    let mut attempt = 0;
    loop {
        let outcome = request()?.send();
        let retry = match &outcome {
            Ok(response) => {
                let status = response.status();
                (status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS)
                    .then(|| retry_after(response.headers()))
            }
            Err(error) => (error.is_connect() || error.is_timeout()).then_some(None),
        };
        match retry {
            Some(wait) if attempt < RETRIES => {
                attempt += 1;
                let backoff = Duration::from_secs(1 << (attempt - 1));
                std::thread::sleep(wait.unwrap_or(backoff).min(MAX_RETRY_WAIT));
            }
            _ => return outcome.map_err(|error| describe_error(url, &error)),
        }
    }
}

/// `Retry-After` as either delay seconds or an HTTP date.
fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    let value = headers.get(RETRY_AFTER)?.to_str().ok()?.trim();
    value
        .parse::<u64>()
        .ok()
        .or_else(|| parse_http_date(value).map(|at| at.saturating_sub(epoch_seconds_now())))
        .map(Duration::from_secs)
}

pub(crate) fn base_url() -> Result<&'static str, String> {
    locator::marketplace_base_url().ok_or_else(|| "No marketplace is configured.".to_string())
}

/// Server health plus the server's clock, for the preflight.
#[derive(Clone, Debug)]
pub(crate) struct HealthProbe {
    pub(crate) health: Health,
    pub(crate) server_epoch_seconds: Option<u64>,
}

pub(crate) fn fetch_health() -> Result<HealthProbe, String> {
    let url = format!("{}/api/health", base_url()?);
    let client = client()?;
    let response = send(&url, || Ok(client.get(&url)))?;
    if !response.status().is_success() {
        return Err(failure(&url, response));
    }
    let server_epoch_seconds = response
        .headers()
        .get(reqwest::header::DATE)
        .and_then(|value| value.to_str().ok())
        .and_then(parse_http_date);
    let health = response
        .json::<Health>()
        .map_err(|error| format!("{url} returned an unreadable health document: {error}"))?;
    Ok(HealthProbe {
        health,
        server_epoch_seconds,
    })
}

pub(crate) fn fetch_me() -> Result<Me, String> {
    let url = format!("{}/api/me", base_url()?);
    let client = client()?;
    let response = send(&url, || authorize(client.get(&url), &url))?;
    let status = response.status();
    if status.as_u16() == 401 {
        return Err("The marketplace rejected this machine's identity (HTTP 401). On a domain-joined machine that means the service principal name or keytab does not match; on a workgroup machine the server must allow the development header.".to_string());
    }
    if status.as_u16() == 403 {
        return Err(
            "The marketplace recognized this account but refused it (HTTP 403).".to_string(),
        );
    }
    if !status.is_success() {
        return Err(failure(&url, response));
    }
    response
        .json::<Me>()
        .map_err(|error| format!("{url} returned an unreadable identity document: {error}"))
}

pub(crate) fn fetch_index() -> Result<Index, String> {
    let url = format!("{}/api/index", base_url()?);
    let client = client()?;
    let response = send(&url, || authorize(client.get(&url), &url))?;
    if !response.status().is_success() {
        return Err(failure(&url, response));
    }
    response
        .json::<Index>()
        .map_err(|error| format!("{url} returned an unreadable index: {error}"))
}

/// Fetches the index and caches it; falls back to the cached copy when the
/// server is unreachable. Returns `None` when neither is available.
pub(crate) fn index_with_cache(cache_base: &Path) -> Option<Index> {
    match fetch_index() {
        Ok(index) => {
            if let Ok(json) = serde_json::to_vec(&index) {
                let _ = std::fs::create_dir_all(cache_base);
                let _ = std::fs::write(cache_base.join(INDEX_CACHE_FILE), json);
            }
            Some(index)
        }
        Err(error) => {
            eprintln!("Marketplace index unavailable, using the cached copy: {error}");
            read_cached_index(cache_base)
        }
    }
}

pub(crate) fn read_cached_index(cache_base: &Path) -> Option<Index> {
    let bytes = std::fs::read(cache_base.join(INDEX_CACHE_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClientEvent {
    pub(crate) kind: String,
    pub(crate) occurred_at: String,
    pub(crate) client_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) os_build: Option<String>,
    pub(crate) agents: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) installed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) checks: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) package_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) from_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) to_version: Option<String>,
}

impl ClientEvent {
    fn base(kind: &'static str, agents: Vec<String>) -> Self {
        Self {
            kind: kind.to_string(),
            occurred_at: rfc3339_now(),
            client_version: CLIENT_VERSION.to_string(),
            os_build: None,
            agents,
            installed: None,
            checks: None,
            package_id: None,
            version: None,
            from_version: None,
            to_version: None,
        }
    }

    pub(crate) fn heartbeat(
        agents: Vec<String>,
        installed: Vec<String>,
        checks: BTreeMap<String, String>,
    ) -> Self {
        let mut event = Self::base("heartbeat", agents);
        event.os_build = Some(os_build());
        event.installed = Some(installed);
        event.checks = Some(checks);
        event
    }

    pub(crate) fn install(package_id: &str, version: Option<String>, agents: Vec<String>) -> Self {
        let mut event = Self::base("install", agents);
        event.package_id = Some(package_id.to_string());
        event.version = version;
        event
    }

    pub(crate) fn update(
        package_id: &str,
        from_version: Option<String>,
        to_version: Option<String>,
        agents: Vec<String>,
    ) -> Self {
        let mut event = Self::base("update", agents);
        event.package_id = Some(package_id.to_string());
        event.from_version = from_version;
        event.to_version = to_version;
        event
    }

    pub(crate) fn uninstall(package_id: &str, agents: Vec<String>) -> Self {
        let mut event = Self::base("uninstall", agents);
        event.package_id = Some(package_id.to_string());
        event
    }
}

#[derive(Serialize)]
struct EventsBatch<'a> {
    events: &'a [ClientEvent],
}

pub(crate) fn post_events(events: &[ClientEvent]) -> Result<(), String> {
    if events.is_empty() {
        return Ok(());
    }
    let url = format!("{}/api/events", base_url()?);
    let response = authorize(client()?.post(&url), &url)?
        .json(&EventsBatch { events })
        .send()
        .map_err(|error| describe_error(&url, &error))?;
    if !response.status().is_success() {
        return Err(failure(&url, response));
    }
    Ok(())
}

static PENDING_REPORTS: std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> =
    std::sync::Mutex::new(Vec::new());
/// One reporter at a time reads and rewrites the outbox.
static OUTBOX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct QueuedEvent {
    queued_at: u64,
    event: ClientEvent,
}

/// Sends events from a background thread. Usage reporting never blocks or
/// fails an operation. Events that cannot be delivered wait in an outbox on
/// disk and go out with the next report. A short-lived process (the CLI) calls
/// [`flush_events`] before exiting so the report is not lost with the process.
pub(crate) fn send_events_background(events: Vec<ClientEvent>) {
    if events.is_empty() || locator::marketplace_base_url().is_none() {
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("marketplace-events".to_string())
        .spawn(move || {
            let delivered = crate::sources::cache_base_dir().and_then(|cache| {
                deliver_with_outbox(&cache.join(OUTBOX_FILE), events, post_events)
            });
            if let Err(error) = delivered {
                eprintln!("Could not report usage to the marketplace; it will be retried: {error}");
            }
        });
    match spawned {
        Ok(handle) => {
            if let Ok(mut pending) = PENDING_REPORTS.lock() {
                pending.retain(|existing| !existing.is_finished());
                pending.push(handle);
            }
        }
        Err(error) => eprintln!("Could not start the usage reporter: {error}"),
    }
}

/// Posts `events` after everything still queued in `outbox`, oldest first, in
/// batches the server accepts. What is not delivered stays queued, capped at
/// the newest 1,000 events from the last 7 days. Only the newest heartbeat is
/// worth keeping, so a new one replaces any queued one.
fn deliver_with_outbox(
    outbox: &Path,
    events: Vec<ClientEvent>,
    post: impl Fn(&[ClientEvent]) -> Result<(), String>,
) -> Result<(), String> {
    let _guard = OUTBOX.lock().unwrap_or_else(|error| error.into_inner());
    let now = epoch_seconds_now();
    let mut queue = std::fs::read(outbox)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Vec<QueuedEvent>>(&bytes).ok())
        .unwrap_or_default();
    if events.iter().any(|event| event.kind == "heartbeat") {
        queue.retain(|queued| queued.event.kind != "heartbeat");
    }
    queue.extend(events.into_iter().map(|event| QueuedEvent {
        queued_at: now,
        event,
    }));
    queue.retain(|queued| now.saturating_sub(queued.queued_at) <= OUTBOX_MAX_AGE_SECONDS);
    let excess = queue.len().saturating_sub(OUTBOX_MAX_EVENTS);
    queue.drain(..excess);
    let mut result = Ok(());
    while !queue.is_empty() {
        let batch = queue
            .iter()
            .take(EVENTS_PER_BATCH)
            .map(|queued| queued.event.clone())
            .collect::<Vec<_>>();
        if let Err(error) = post(&batch) {
            result = Err(error);
            break;
        }
        queue.drain(..batch.len());
    }
    if queue.is_empty() {
        let _ = std::fs::remove_file(outbox);
    } else if let Ok(json) = serde_json::to_vec(&queue) {
        if let Some(parent) = outbox.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(outbox, json);
    }
    result
}

/// Waits for every in-flight usage report. Each report has its own request
/// timeout, so this returns within that bound.
pub(crate) fn flush_events() {
    let handles = match PENDING_REPORTS.lock() {
        Ok(mut pending) => std::mem::take(&mut *pending),
        Err(_) => return,
    };
    for handle in handles {
        let _ = handle.join();
    }
}

/// The message for a failed marketplace response: its problem document when
/// it sent one, otherwise the bare status.
pub(crate) fn failure(url: &str, response: Response) -> String {
    let status = response.status();
    problem_message(&response.text().unwrap_or_default())
        .unwrap_or_else(|| format!("{url} answered HTTP {status}."))
}

/// Reads an RFC 9457 problem document: its `title`, then its `detail` and each
/// validation error on their own lines. `None` when `body` has no title.
pub(crate) fn problem_message(body: &str) -> Option<String> {
    let problem = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let mut message = problem.get("title")?.as_str()?.to_string();
    if let Some(detail) = problem.get("detail").and_then(|value| value.as_str()) {
        message.push_str(&format!("\n  {detail}"));
    }
    for error in problem
        .get("errors")
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
    {
        let path = error.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let text = error.get("message").and_then(|v| v.as_str()).unwrap_or("");
        message.push_str(&format!("\n  {path}: {text}"));
    }
    Some(message)
}

pub(crate) fn describe_error(url: &str, error: &reqwest::Error) -> String {
    let text = error.to_string();
    let source = std::error::Error::source(error)
        .map(|inner| inner.to_string())
        .unwrap_or_default();
    let detail = if source.is_empty() {
        text
    } else {
        format!("{text} ({source})")
    };
    // A connect timeout is a connect failure first: the server was never reached.
    if error.is_connect() {
        format!("{CONNECT_FAILURE} {url}: {detail}")
    } else if error.is_timeout() {
        format!("{url} {TIMED_OUT}.")
    } else {
        format!("Request to {url} failed: {detail}")
    }
}

pub(crate) fn os_build() -> String {
    #[cfg(windows)]
    {
        if let Ok(key) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
            .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        {
            let build: String = key.get_value("CurrentBuildNumber").unwrap_or_default();
            let ubr: u32 = key.get_value("UBR").unwrap_or_default();
            if !build.is_empty() {
                return format!("windows/10.0.{build}.{ubr}");
            }
        }
        "windows".to_string()
    }
    #[cfg(not(windows))]
    {
        format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH)
    }
}

pub(crate) fn epoch_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// RFC 3339 UTC timestamp with second precision.
pub(crate) fn rfc3339_now() -> String {
    rfc3339_from_epoch(epoch_seconds_now())
}

pub(crate) fn rfc3339_from_epoch(seconds: u64) -> String {
    let days = seconds / 86_400;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        remainder / 3600,
        (remainder % 3600) / 60,
        remainder % 60
    )
}

/// Howard Hinnant's days-to-civil algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parses an RFC 7231 `Date` header (`Sat, 29 Aug 2026 03:06:17 GMT`) to epoch seconds.
pub(crate) fn parse_http_date(value: &str) -> Option<u64> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 6 {
        return None;
    }
    let day = parts[1].parse::<i64>().ok()?;
    let month = match parts[2] {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = parts[3].parse::<i64>().ok()?;
    let mut clock = parts[4].split(':');
    let hour = clock.next()?.parse::<i64>().ok()?;
    let minute = clock.next()?.parse::<i64>().ok()?;
    let second = clock.next()?.parse::<i64>().ok()?;
    let days = days_from_civil(year, month, day);
    let total = days * 86_400 + hour * 3600 + minute * 60 + second;
    u64::try_from(total).ok()
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Compares dotted numeric versions such as `0.1.0` and `0.2.1`.
pub(crate) fn version_less_than(left: &str, right: &str) -> bool {
    let parse = |value: &str| {
        value
            .split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let (left, right) = (parse(left), parse(right));
    let length = left.len().max(right.len());
    for index in 0..length {
        let l = left.get(index).copied().unwrap_or(0);
        let r = right.get(index).copied().unwrap_or(0);
        if l != r {
            return l < r;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_parses_dates() {
        assert_eq!(rfc3339_from_epoch(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_from_epoch(1_787_972_777), "2026-08-29T03:06:17Z");
        assert_eq!(
            parse_http_date("Sat, 29 Aug 2026 03:06:17 GMT"),
            Some(1_787_972_777)
        );
        assert_eq!(parse_http_date("nonsense"), None);
    }

    #[test]
    fn reads_problem_documents() {
        let body = r#"{"title":"The package failed validation.","detail":"Path: $.version","errors":[{"path":"version","message":"1.0.0-beta.1 is a pre-release."}]}"#;
        assert_eq!(
            problem_message(body).as_deref(),
            Some("The package failed validation.\n  Path: $.version\n  version: 1.0.0-beta.1 is a pre-release.")
        );
        assert_eq!(problem_message("<html>Bad gateway</html>"), None);
        assert_eq!(problem_message(r#"{"status":503}"#), None);
    }

    #[test]
    fn undelivered_events_wait_in_the_outbox_for_the_next_report() {
        let dir = tempfile::tempdir().expect("dir");
        let outbox = dir.path().join(OUTBOX_FILE);
        let offline = |_: &[ClientEvent]| Err("offline".to_string());
        let heartbeat = || ClientEvent::heartbeat(Vec::new(), Vec::new(), BTreeMap::new());
        deliver_with_outbox(
            &outbox,
            vec![
                heartbeat(),
                ClientEvent::uninstall("acme/review", Vec::new()),
            ],
            offline,
        )
        .expect_err("offline");
        deliver_with_outbox(&outbox, vec![heartbeat()], offline).expect_err("still offline");

        let sent = std::cell::RefCell::new(Vec::new());
        deliver_with_outbox(
            &outbox,
            vec![ClientEvent::install("acme/tools", None, Vec::new())],
            |events| {
                sent.borrow_mut()
                    .extend(events.iter().map(|event| event.kind.clone()));
                Ok(())
            },
        )
        .expect("delivered");

        assert_eq!(
            sent.into_inner(),
            ["uninstall", "heartbeat", "install"],
            "queued events go first, and only the newest heartbeat is kept"
        );
        assert!(!outbox.exists());
    }

    #[test]
    fn compares_versions() {
        assert!(version_less_than("0.1.0", "0.2.0"));
        assert!(version_less_than("0.1.0", "0.1.1"));
        assert!(!version_less_than("1.0.0", "0.9.9"));
        assert!(!version_less_than("1.0.0", "1.0.0"));
        assert!(version_less_than("1.0", "1.0.1"));
    }

    #[test]
    fn dev_user_env_forces_the_development_header() {
        let headers = {
            std::env::set_var(DEV_USER_ENV, "TEST\\someone");
            std::env::set_var(DEV_GROUPS_ENV, " Platform Team,Ops ");
            let headers = auth_headers(&format!("{}/api/me", locator::MARKETPLACE_URL));
            std::env::remove_var(DEV_USER_ENV);
            std::env::remove_var(DEV_GROUPS_ENV);
            headers
        }
        .expect("headers");
        assert_eq!(
            headers,
            vec![
                ("X-Dev-User", "TEST\\someone".to_string()),
                ("X-Dev-Groups", "Platform Team,Ops".to_string()),
            ]
        );
        assert!(auth_headers("https://example.com/other.zip")
            .expect("headers")
            .is_empty());
    }
}
