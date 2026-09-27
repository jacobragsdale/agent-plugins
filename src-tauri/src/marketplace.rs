//! Marketplace server client: identity headers, health, index, and events.
//!
//! Every request to a marketplace URL carries the caller's identity. On a
//! domain-joined Windows host that is a Kerberos `Negotiate` token for
//! `HTTP/<server host>`; otherwise the `X-Dev-User` header, which only a
//! Development server trusts. See ADR 0004.

use crate::host_identity::{self, JoinState};
use crate::locator::{self, is_marketplace_url};
use reqwest::blocking::{Client, ClientBuilder, RequestBuilder, Response};
use reqwest::header::{HeaderMap, RETRY_AFTER};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const DEV_USER_ENV: &str = "AGENT_PLUGINS_DEV_USER";
/// Comma-separated groups sent as `X-Dev-Groups` with the development header,
/// so team rules can be exercised where there is no domain.
pub(crate) const DEV_GROUPS_ENV: &str = "AGENT_PLUGINS_DEV_GROUPS";
const INDEX_CACHE_FILE: &str = "marketplace-index.json";
/// The index's ETag, so an unchanged index costs the server a 304.
const INDEX_ETAG_FILE: &str = "marketplace-index.etag";
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
    #[serde(default)]
    pub(crate) teams: Vec<TeamMembership>,
    /// Suggestions on the caller's packages that wait for their answer.
    #[serde(default)]
    pub(crate) suggestions_waiting: u64,
    /// Open reports and feedback on the caller's packages.
    #[serde(default)]
    pub(crate) reports_waiting: u64,
    #[serde(default)]
    pub(crate) unread_notifications: u64,
}

/// Something the marketplace tells a person: a suggestion waiting, a
/// connector approved, a report answered.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Notification {
    pub(crate) id: u64,
    #[serde(default)]
    pub(crate) kind: String,
    pub(crate) text: String,
    /// A portal path such as `/p/ns/id`.
    #[serde(default)]
    pub(crate) link: Option<String>,
}

#[derive(Deserialize)]
struct NotificationPage {
    #[serde(default)]
    items: Vec<Notification>,
}

const NOTIFICATIONS_SEEN_FILE: &str = "notifications-seen";

/// Marketplace news newer than what this PC already showed. The first look
/// only remembers where the news stands, so nobody gets a backlog at once.
pub(crate) fn new_notifications(cache_base: &Path) -> Result<Vec<Notification>, String> {
    let seen_file = cache_base.join(NOTIFICATIONS_SEEN_FILE);
    let seen = std::fs::read_to_string(&seen_file)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok());
    let path = match seen {
        Some(seen) => format!("notifications?after={seen}&limit=50"),
        None => "notifications?limit=1".to_string(),
    };
    let page = api_json::<NotificationPage>(reqwest::Method::GET, &path, None)?;
    let newest = page
        .items
        .iter()
        .map(|item| item.id)
        .max()
        .or(seen)
        .unwrap_or(0);
    let _ = std::fs::create_dir_all(cache_base);
    let _ = crate::fs_retry::replace_file(&seen_file, newest.to_string().as_bytes());
    Ok(if seen.is_some() {
        page.items
    } else {
        Vec::new()
    })
}

/// A team the caller belongs to, as `/api/me` lists it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamMembership {
    pub(crate) namespace: String,
    pub(crate) display_name: String,
    #[serde(default)]
    pub(crate) owner: bool,
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
    /// Only some people may see it: the package or its space is private.
    #[serde(default)]
    pub(crate) restricted: bool,
    /// The caller sees it only because it was shared with them.
    #[serde(default)]
    pub(crate) shared_with_you: bool,
    /// What the live version changed, in its publisher's words.
    #[serde(default)]
    pub(crate) changelog: Option<String>,
    /// Whether an admin let everyone see its MCP server; `None` without one.
    #[serde(default)]
    pub(crate) mcp_approved: Option<bool>,
}

/// A named list of packages that install together, from the index.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IndexBundle {
    pub(crate) id: String,
    pub(crate) namespace: String,
    pub(crate) bundle_id: String,
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: String,
    pub(crate) publisher: IndexPublisher,
    pub(crate) lane: String,
    /// Canonical ids of the members the caller may see.
    #[serde(default)]
    pub(crate) members: Vec<String>,
    #[serde(default)]
    pub(crate) updated_at: String,
    #[serde(default)]
    pub(crate) restricted: bool,
    #[serde(default)]
    pub(crate) shared_with_you: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Index {
    #[serde(default)]
    pub(crate) generated_at: String,
    #[serde(default)]
    pub(crate) packages: Vec<IndexPackage>,
    #[serde(default)]
    pub(crate) bundles: Vec<IndexBundle>,
    /// Packages their publisher or an admin pulled from every PC.
    #[serde(default)]
    pub(crate) revoked: Vec<String>,
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
        JoinState::Cloud => AuthMode::Negotiate("Microsoft Entra ID".to_string()),
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

static CLIENT: Mutex<Option<Client>> = Mutex::new(None);

pub(crate) fn client() -> Result<Client, String> {
    cached_client(&CLIENT, |builder| {
        builder
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(format!("agent-plugins/{CLIENT_VERSION}"))
            .build()
    })
}

/// A clone of the client in `slot`, built on first use, so requests share one
/// connection pool, certificate checks, and proxy lookup until
/// `reset_http_clients`.
pub(crate) fn cached_client(
    slot: &Mutex<Option<Client>>,
    build: impl FnOnce(ClientBuilder) -> reqwest::Result<Client>,
) -> Result<Client, String> {
    let mut slot = slot.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(client) = slot.as_ref() {
        return Ok(client.clone());
    }
    let client = build(Client::builder().use_preconfigured_tls(tls()?))
        .map_err(|error| format!("Could not create the HTTPS client: {error}"))?;
    *slot = Some(client.clone());
    Ok(client)
}

/// Certificate checks for every HTTPS client, done by the operating system as
/// a browser does them. A fresh Windows PC holds only a few root authorities
/// and fetches the rest the first time a chain needs one; reading the store
/// alone rejected such a server as untrusted until something else had fetched
/// its root.
pub(crate) fn tls() -> Result<rustls::ClientConfig, String> {
    use rustls_platform_verifier::BuilderVerifierExt;
    rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .and_then(BuilderVerifierExt::with_platform_verifier)
    .map(|builder| builder.with_no_client_auth())
    .map_err(|error| format!("Could not set up HTTPS certificate checks: {error}"))
}

/// Drops the cached clients, so the next request picks up proxy and
/// certificate changes. Each sync pass starts with this.
pub(crate) fn reset_http_clients() {
    for slot in [&CLIENT, &crate::artifact::CLIENT] {
        *slot.lock().unwrap_or_else(|error| error.into_inner()) = None;
    }
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
            // A refused certificate stays refused, so it is not worth a retry.
            Err(error) => ((error.is_connect() && refused_certificate(error).is_none())
                || error.is_timeout())
            .then_some(None),
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
        return Err(
            "The marketplace didn't accept this computer's Windows sign-in (HTTP 401).".to_string(),
        );
    }
    if status.as_u16() == 403 {
        return Err(
            "The marketplace recognized your account but doesn't allow it (HTTP 403).".to_string(),
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
    fetch_index_since(None)?
        .map(|(index, _)| index)
        .ok_or_else(|| "The marketplace index did not change.".to_string())
}

/// The index, or `None` when it still has the ETag `since`, with its new ETag.
fn fetch_index_since(since: Option<&str>) -> Result<Option<(Index, Option<String>)>, String> {
    let url = format!("{}/api/index", base_url()?);
    let client = client()?;
    let response = send(&url, || {
        let request = authorize(client.get(&url), &url)?;
        Ok(match since {
            Some(etag) => request.header(reqwest::header::IF_NONE_MATCH, etag),
            None => request,
        })
    })?;
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Err(failure(&url, response));
    }
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let index = response
        .json::<Index>()
        .map_err(|error| format!("{url} returned an unreadable index: {error}"))?;
    Ok(Some((index, etag)))
}

/// Fetches the index and caches it; falls back to the cached copy when the
/// server is unreachable or says it has not changed. Returns `None` when
/// neither is available.
pub(crate) fn index_with_cache(cache_base: &Path) -> Option<Index> {
    let cached = read_cached_index(cache_base);
    let etag = cached
        .as_ref()
        .and_then(|_| std::fs::read_to_string(cache_base.join(INDEX_ETAG_FILE)).ok());
    match fetch_index_since(etag.as_deref().map(str::trim)) {
        Ok(Some((index, etag))) => {
            if let Ok(json) = serde_json::to_vec(&index) {
                let _ = std::fs::create_dir_all(cache_base);
                let _ = crate::fs_retry::replace_file(&cache_base.join(INDEX_CACHE_FILE), &json);
                let etag_file = cache_base.join(INDEX_ETAG_FILE);
                match etag {
                    Some(etag) => {
                        let _ = crate::fs_retry::replace_file(&etag_file, etag.as_bytes());
                    }
                    None => {
                        let _ = std::fs::remove_file(etag_file);
                    }
                }
            }
            Some(index)
        }
        Ok(None) => cached,
        Err(error) => {
            eprintln!("Marketplace index unavailable, using the cached copy: {error}");
            cached
        }
    }
}

pub(crate) fn read_cached_index(cache_base: &Path) -> Option<Index> {
    let bytes = std::fs::read(cache_base.join(INDEX_CACHE_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Calls `/api/{path}` as the caller and returns the body of a successful
/// answer. A read is retried like every other request; a change is sent once,
/// so it never happens twice. A failed answer becomes its problem title.
pub(crate) fn api(
    method: reqwest::Method,
    path: &str,
    body: Option<&serde_json::Value>,
) -> Result<String, String> {
    let url = format!("{}/api/{path}", base_url()?);
    let client = client()?;
    let request = || {
        let mut request = client.request(method.clone(), &url);
        if let Some(body) = body {
            request = request.json(body);
        }
        authorize(request, &url)
    };
    let response = if method == reqwest::Method::GET {
        send(&url, request)?
    } else {
        request()?
            .send()
            .map_err(|error| describe_error(&url, &error))?
    };
    if !response.status().is_success() {
        return Err(failure(&url, response));
    }
    response
        .text()
        .map_err(|error| describe_error(&url, &error))
}

/// [`api`], read as JSON.
pub(crate) fn api_json<T: serde::de::DeserializeOwned>(
    method: reqwest::Method,
    path: &str,
    body: Option<&serde_json::Value>,
) -> Result<T, String> {
    let text = api(method, path, body)?;
    serde_json::from_str(&text).map_err(|error| {
        format!(
            "The marketplace answered /api/{path} with something Agent Plugins can't read: {error}"
        )
    })
}

/// A space's name, checked so it can go into a URL path.
pub(crate) fn namespace_path(namespace: &str) -> Result<&str, String> {
    crate::manifest::validate_source_id(namespace)
        .map(|()| namespace)
        .map_err(|_| format!("{namespace} is not a space name such as data-team."))
}

/// `ns` or `ns/id`, checked as marketplace names so it can go into a URL path.
pub(crate) fn target_path(target: &str) -> Result<&str, String> {
    let (namespace, id) = match target.split_once('/') {
        Some((namespace, id)) => (namespace, Some(id)),
        None => (target, None),
    };
    let valid = crate::manifest::validate_source_id(namespace).is_ok()
        && id.is_none_or(|id| crate::manifest::validate_package_id(id, "package").is_ok());
    if valid {
        Ok(target)
    } else {
        Err(format!(
            "{target} is not a marketplace name such as data-team or data-team/review."
        ))
    }
}

/// `value` percent-encoded for a query string.
pub(crate) fn query(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// The code of a marketplace link (`https://<marketplace>/l/<code>`), or the
/// bare code itself.
pub(crate) fn link_code(link: &str) -> Result<String, String> {
    let link = link.trim();
    let code = link
        .rsplit_once("/l/")
        .map_or(link, |(_, code)| code)
        .trim_end_matches('/');
    let valid = (8..=64).contains(&code.len())
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if valid {
        Ok(code.to_string())
    } else {
        Err(format!(
            "{link} is not a marketplace link. Copy the whole link, which ends in /l/ and a code."
        ))
    }
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
    /// This computer's name, so one person's laptop and desktop count apart.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) device: Option<String>,
    /// The version of each installed package that is up to date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) installed_versions: Option<BTreeMap<String, String>>,
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
            device: None,
            installed_versions: None,
        }
    }

    pub(crate) fn heartbeat(
        agents: Vec<String>,
        installed: Vec<String>,
        installed_versions: BTreeMap<String, String>,
        checks: BTreeMap<String, String>,
    ) -> Self {
        let mut event = Self::base("heartbeat", agents);
        event.os_build = Some(os_build());
        event.installed = Some(installed);
        event.installed_versions = Some(installed_versions);
        event.checks = Some(checks);
        event.device = device_name();
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
        let _ = crate::fs_retry::replace_file(outbox, &json);
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
    // The server answered, so this is not being offline, and waiting fixes
    // nothing: someone has to correct the clock or trust the certificate.
    if let Some(reason) = refused_certificate(error) {
        let host = url::Url::parse(url)
            .ok()
            .and_then(|parsed| parsed.host_str().map(str::to_string))
            .unwrap_or_else(|| url.to_string());
        return format!("The security certificate of {host} isn't trusted by this computer ({reason}). Check that the date and time on this computer are right. If they are, ask IT to add your network's certificate authority to this computer.");
    }
    let detail = causes(error);
    // A connect timeout is a connect failure first: the server was never reached.
    if error.is_connect() {
        format!("{CONNECT_FAILURE} {url}: {detail}")
    } else if error.is_timeout() {
        format!("{url} {TIMED_OUT}.")
    } else {
        format!("Request to {url} failed: {detail}")
    }
}

/// Each distinct message in the error's chain, outermost first. The outer
/// layers are generic ("client error (Connect)"); the cause a person can act
/// on, such as a refused connection or an unknown host, comes last.
fn causes(error: &reqwest::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut next: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = next {
        let text = current.to_string();
        if !parts.iter().any(|part| part.contains(&text)) {
            parts.push(text);
        }
        next = current.source();
    }
    parts.join(": ")
}

/// Why the server's certificate was refused, when that is what failed.
fn refused_certificate(error: &reqwest::Error) -> Option<String> {
    match tls_error(error)? {
        rustls::Error::InvalidCertificate(reason) => Some(certificate_reason(reason)),
        _ => None,
    }
}

/// The TLS error in the chain. It arrives inside `io::Error`s, which may nest,
/// and whose `source()` skips the error they wrap, so those are unwrapped too.
fn tls_error<'a>(error: &'a (dyn std::error::Error + 'static)) -> Option<&'a rustls::Error> {
    if let Some(tls) = error.downcast_ref::<rustls::Error>() {
        return Some(tls);
    }
    if let Some(inner) = error
        .downcast_ref::<std::io::Error>()
        .and_then(std::io::Error::get_ref)
    {
        return tls_error(inner);
    }
    error.source().and_then(tls_error)
}

fn certificate_reason(reason: &rustls::CertificateError) -> String {
    use rustls::CertificateError::*;
    match reason {
        UnknownIssuer => "it comes from an authority this computer doesn't recognize".to_string(),
        Expired | ExpiredContext { .. } => "it has expired".to_string(),
        NotValidYet | NotValidYetContext { .. } => "it isn't valid yet".to_string(),
        NotValidForName | NotValidForNameContext { .. } => {
            "it was issued for a different server".to_string()
        }
        Revoked => "it has been revoked".to_string(),
        other => format!("it could not be verified: {other}"),
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

/// This computer's name, as Windows or the host names it.
fn device_name() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
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

    /// A self-signed certificate for localhost, trusted by nothing.
    const UNTRUSTED_CERT: &str = "MIIBmzCCAUGgAwIBAgIUIZ9hVrKfJCOuFKdb1Zd33iDfPtQwCgYIKoZIzj0EAwIwFDESMBAGA1UEAwwJbG9jYWxob3N0MCAXDTI2MDkyNTIzMjQzMVoYDzIxMjYwOTAxMjMyNDMxWjAUMRIwEAYDVQQDDAlsb2NhbGhvc3QwWTATBgcqhkjOPQIBBggqhkjOPQMBBwNCAARE7SSKUxTn0A8dvGU1K/qf/T9G72d0wm/uJv87lytJL0gPpV6hPx0tU5GDQCV0Oz2EHwYx1HQbYoODY/6ar6nbo28wbTAdBgNVHQ4EFgQUHk3lyxGNYjjzyehPdmOg7A4Oz3cwHwYDVR0jBBgwFoAUHk3lyxGNYjjzyehPdmOg7A4Oz3cwDwYDVR0TAQH/BAUwAwEB/zAaBgNVHREEEzARgglsb2NhbGhvc3SHBH8AAAEwCgYIKoZIzj0EAwIDSAAwRQIgemn/lCbAzkY29XruQGhqv6GODMc4LNqOyXtsi+IiyccCIQCmxyNRDAdG7pZ6zEhfzGvSlACSHd1Wh2ww6KZqY1HG4A==";
    const UNTRUSTED_KEY: &str = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgVn0geI0c5K0ur9wiOznRIE+iSyRmDlk4mwQFBtIv7kShRANCAARE7SSKUxTn0A8dvGU1K/qf/T9G72d0wm/uJv87lytJL0gPpV6hPx0tU5GDQCV0Oz2EHwYx1HQbYoODY/6ar6nb";

    #[test]
    fn an_untrusted_certificate_is_explained_not_reported_as_offline() {
        use base64::Engine as _;
        use std::io::{Read, Write};
        let decode = |text| {
            base64::engine::general_purpose::STANDARD
                .decode(text)
                .expect("der")
        };
        let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![decode(UNTRUSTED_CERT).into()],
            rustls::pki_types::PrivateKeyDer::Pkcs8(decode(UNTRUSTED_KEY).into()),
        )
        .expect("server config");
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!(
            "https://localhost:{}/api/health",
            listener.local_addr().expect("addr").port()
        );
        let accepted = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().expect("accept");
            let connection =
                rustls::ServerConnection::new(std::sync::Arc::new(config)).expect("tls");
            let mut stream = rustls::StreamOwned::new(connection, tcp);
            let _ = stream.read(&mut [0; 1]);
            let _ = stream.flush();
        });
        let client = Client::builder()
            .use_preconfigured_tls(tls().expect("tls"))
            .build()
            .expect("client");
        let error = client.get(&url).send().expect_err("untrusted");
        accepted.join().expect("server");
        let message = describe_error(&url, &error);
        assert!(
            message.starts_with(
                "The security certificate of localhost isn't trusted by this computer ("
            ),
            "{message}"
        );
        assert_eq!(
            crate::ipc_error::IpcError::from(message).kind,
            crate::ipc_error::IpcErrorKind::NeedsUser
        );
    }

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
        let heartbeat =
            || ClientEvent::heartbeat(Vec::new(), Vec::new(), BTreeMap::new(), BTreeMap::new());
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
    fn reads_link_codes_and_checks_names() {
        assert_eq!(
            link_code("https://marketplace.example.com/l/Xk3qZ9_-abcdEFGH12345w/"),
            Ok("Xk3qZ9_-abcdEFGH12345w".to_string())
        );
        assert_eq!(
            link_code(" Xk3qZ9_-abcdEFGH12345w "),
            Ok("Xk3qZ9_-abcdEFGH12345w".to_string())
        );
        assert!(link_code("https://marketplace.example.com/l/../admin").is_err());
        assert!(link_code("short").is_err());
        assert_eq!(target_path("data-team/review"), Ok("data-team/review"));
        assert!(target_path("data-team/review/extra").is_err());
        assert!(target_path("../admin").is_err());
        assert!(namespace_path("data-team/review").is_err());
        assert_eq!(query("CORP\\jane doe"), "CORP%5Cjane+doe");
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
