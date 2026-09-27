//! HTTPS artifact locators for sources and source repositories.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Marketplace server base URL when no fleet policy names one. The catalog
/// document lives at `/api/catalog` and namespace archives under
/// `/api/sources/{namespace}/archive`. Empty disables the marketplace.
pub(crate) const MARKETPLACE_URL: &str = "https://marketplace.ragsdale.dev";

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Locator {
    pub url: String,
}

impl Locator {
    pub(crate) fn parse(url: &str) -> Result<Self, String> {
        Ok(Self {
            url: canonicalize_artifact_url(url)?,
        })
    }

    pub(crate) fn display_url(url: String) -> Self {
        Self { url }
    }

    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    pub(crate) fn source_key(&self) -> String {
        prefixed_key("source-", format!("artifact:{}", self.url).as_bytes())
    }

    pub(crate) fn repository_key(&self) -> String {
        prefixed_key("repo-", format!("artifact\0{}", self.url).as_bytes())
    }

    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.url == other.url
    }
}

/// A fleet setting IT sets under `Software\Policies\AgentPlugins`, the
/// machine's value first, then the user's, so one build serves any company.
#[cfg(windows)]
pub(crate) fn policy_value<T: winreg::types::FromRegValue>(name: &str) -> Option<T> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
        .into_iter()
        .find_map(|root| {
            winreg::RegKey::predef(root)
                .open_subkey(r"Software\Policies\AgentPlugins")
                .ok()?
                .get_value::<T, _>(name)
                .ok()
        })
}

fn policy_url(name: &str) -> Option<String> {
    #[cfg(windows)]
    let value = policy_value::<String>(name);
    #[cfg(not(windows))]
    let value: Option<String> = {
        let _ = name;
        None
    };
    let value = value?.trim().trim_end_matches('/').to_string();
    match url::Url::parse(&value) {
        Ok(url) if url.scheme() == "https" && url.host_str().is_some() => Some(value),
        _ => {
            eprintln!("Ignored the {name} policy: {value:?} is not an https address.");
            None
        }
    }
}

/// A development build can point at a local marketplace with
/// `AGENT_PLUGINS_MARKETPLACE_URL`; a release build ignores it.
fn development_url() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("AGENT_PLUGINS_MARKETPLACE_URL")
        .ok()
        .map(|url| url.trim().trim_end_matches('/').to_string())
        .filter(|url| !url.is_empty())
}

/// The marketplace base URL without a trailing slash, or `None` when disabled:
/// the `MarketplaceUrl` policy, else the built-in address.
pub(crate) fn marketplace_base_url() -> Option<&'static str> {
    static URL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    URL.get_or_init(|| {
        development_url()
            .or_else(|| policy_url("MarketplaceUrl"))
            .or_else(|| {
                let url = MARKETPLACE_URL.trim().trim_end_matches('/');
                (!url.is_empty()).then(|| url.to_string())
            })
    })
    .as_deref()
}

/// Where a person downloads a newer client: the `DownloadUrl` policy, else the
/// marketplace's own download section.
pub(crate) fn download_url() -> Option<&'static str> {
    static URL: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    URL.get_or_init(|| {
        policy_url("DownloadUrl")
            .or_else(|| marketplace_base_url().map(|base| format!("{base}/#download")))
    })
    .as_deref()
}

pub(crate) fn default_catalog_locator() -> Result<Option<Locator>, String> {
    match marketplace_base_url() {
        None => Ok(None),
        Some(base) => Ok(Some(Locator::parse(&format!("{base}/api/catalog"))?)),
    }
}

/// True when `url` is served by the marketplace server (same scheme, host, and port).
pub(crate) fn is_marketplace_url(url: &str) -> bool {
    let Some(base) = marketplace_base_url() else {
        return false;
    };
    let (Ok(base), Ok(candidate)) = (url::Url::parse(base), url::Url::parse(url)) else {
        return false;
    };
    base.scheme() == candidate.scheme()
        && base.host_str().map(str::to_ascii_lowercase)
            == candidate.host_str().map(str::to_ascii_lowercase)
        && base.port_or_known_default() == candidate.port_or_known_default()
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn prefixed_key(prefix: &str, material: &[u8]) -> String {
    let digest = Sha256::digest(material);
    let mut key = prefix.to_string();
    for byte in &digest[..8] {
        write!(&mut key, "{byte:02x}").expect("writing to a String cannot fail");
    }
    key
}

pub(crate) fn canonicalize_artifact_url(input: &str) -> Result<String, String> {
    let input = input.trim();
    let (scheme, remainder) = input
        .split_once("://")
        .ok_or_else(|| artifact_url_error("Use an https:// URL."))?;
    // A development build pointed at a local marketplace reads it over HTTP.
    let development = scheme.eq_ignore_ascii_case("http")
        && development_url().is_some_and(|base| {
            input
                .to_ascii_lowercase()
                .starts_with(&format!("{}/", base.to_ascii_lowercase()))
        });
    if !scheme.eq_ignore_ascii_case("https") && !development {
        return Err(artifact_url_error(
            "Only https:// URLs are supported. HTTP, including LAN Nexus, is not accepted.",
        ));
    }
    if remainder.is_empty()
        || remainder
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        || remainder.contains('\\')
    {
        return Err(artifact_url_error("The URL contains an invalid character."));
    }
    let remainder = remainder
        .split_once('#')
        .map_or(remainder, |(without_fragment, _)| without_fragment);
    let (authority, path_and_query) = remainder
        .split_once('/')
        .ok_or_else(|| artifact_url_error("The URL must include a path."))?;
    if authority.is_empty() || authority.contains('@') {
        return Err(artifact_url_error(
            "Artifact URLs may not contain credentials or an empty host.",
        ));
    }
    if path_and_query.is_empty() {
        return Err(artifact_url_error("The URL must include a path."));
    }
    let (path, query) = path_and_query
        .split_once('?')
        .map_or((path_and_query, None), |(path, query)| (path, Some(query)));
    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return Err(artifact_url_error("The URL must include a path."));
    }
    let (host, port) = canonical_host_and_port(authority)?;
    let mut canonical = format!("{}://{host}", if development { "http" } else { "https" });
    if let Some(port) = port {
        canonical.push(':');
        canonical.push_str(port);
    }
    canonical.push('/');
    canonical.push_str(path);
    if let Some(query) = query {
        if query.is_empty() {
            return Err(artifact_url_error("The URL has an empty query string."));
        }
        canonical.push('?');
        canonical.push_str(query);
    }
    Ok(canonical)
}

fn canonical_host_and_port(host_port: &str) -> Result<(String, Option<&str>), String> {
    let (host, port) = if let Some(bracketed) = host_port.strip_prefix('[') {
        let closing = bracketed
            .find(']')
            .ok_or_else(|| artifact_url_error("The URL has an invalid IPv6 host."))?;
        let host_end = closing + 1;
        let host = &host_port[..=host_end];
        let suffix = &host_port[host_end + 1..];
        let port = if suffix.is_empty() {
            None
        } else {
            Some(
                suffix
                    .strip_prefix(':')
                    .ok_or_else(|| artifact_url_error("The URL has an invalid host."))?,
            )
        };
        (host, port)
    } else {
        if host_port.matches(':').count() > 1 {
            return Err(artifact_url_error(
                "IPv6 hosts must be enclosed in brackets.",
            ));
        }
        host_port
            .rsplit_once(':')
            .map_or((host_port, None), |(host, port)| (host, Some(port)))
    };
    if host.is_empty()
        || host == "[]"
        || host
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        || host.contains(['/', '@', '%'])
    {
        return Err(artifact_url_error("The URL has an invalid host."));
    }
    let port = match port {
        Some(port) => {
            let parsed = port
                .parse::<u16>()
                .map_err(|_| artifact_url_error("The URL has an invalid port."))?;
            if parsed == 0 {
                return Err(artifact_url_error("The URL has an invalid port."));
            }
            (parsed != 443).then_some(port)
        }
        None => None,
    };
    Ok((host.to_ascii_lowercase(), port))
}

fn artifact_url_error(detail: &str) -> String {
    format!("Invalid artifact URL. {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_source_key_uses_prefixed_canonical_url() {
        let locator = Locator::parse(
            "HTTPS://Nexus.Example.com:443/repository/raw/sources/data-latest.zip?download=1#ignored",
        )
        .expect("locator");
        assert_eq!(
            locator.url(),
            "https://nexus.example.com/repository/raw/sources/data-latest.zip?download=1"
        );
        assert_eq!(
            locator.source_key(),
            prefixed_key(
                "source-",
                b"artifact:https://nexus.example.com/repository/raw/sources/data-latest.zip?download=1"
            )
        );
    }

    #[test]
    fn artifact_urls_reject_credentials_and_http() {
        assert!(
            Locator::parse("http://nexus.example.com/repository/raw/latest.zip")
                .expect_err("http")
                .contains("https://")
        );
        assert!(
            Locator::parse("https://user:token@nexus.example.com/repository/raw/latest.zip")
                .expect_err("userinfo")
                .contains("credentials")
        );
    }

    #[test]
    fn repository_key_is_stable_for_canonical_url() {
        let locator = Locator::parse("https://nexus.example.com/repository/raw/catalogs/acme.json")
            .expect("locator");
        assert!(locator.repository_key().starts_with("repo-"));
        assert_ne!(locator.repository_key(), locator.source_key());
        assert_eq!(
            locator.repository_key(),
            Locator::parse("HTTPS://Nexus.Example.com:443/repository/raw/catalogs/acme.json")
                .expect("canonical")
                .repository_key()
        );
    }

    #[test]
    fn default_catalog_is_the_marketplace_catalog_endpoint() {
        let locator = default_catalog_locator()
            .expect("default")
            .expect("configured");
        assert_eq!(locator.url(), format!("{MARKETPLACE_URL}/api/catalog"));
        assert!(is_marketplace_url(locator.url()));
        assert!(is_marketplace_url(&format!(
            "{MARKETPLACE_URL}/api/sources/jacob/archive"
        )));
        assert!(!is_marketplace_url(
            "https://repo.ragsdale.dev/api/v1/repositories/files/download/x.zip"
        ));
    }
}
