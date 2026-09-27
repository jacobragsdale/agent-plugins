//! Portable MCP configuration parser and validator.
//!
//! The document shape matches the Agent Plugins 1.0.0 `mcp.json` schema. That is
//! a closed transport contract, not a plugin install path.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use url::{Host, Url};

pub const MCP_SCHEMA_V1: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "kebab-case")]
pub enum McpServer {
    Stdio {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    StreamableHttp {
        url: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
    },
    Sse {
        url: String,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        headers: BTreeMap<String, String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct McpConfig {
    #[serde(rename = "$schema")]
    pub schema: String,
    pub mcp_servers: BTreeMap<String, McpServer>,
}

impl McpConfig {
    pub fn from_slice(contents: &[u8]) -> Result<Self, String> {
        let value = serde_json::from_slice::<serde_json::Value>(contents)
            .map_err(|e| format!("Could not parse mcp.json: {e}"))?;
        let config = serde_json::from_value::<McpConfig>(value)
            .map_err(|e| format!("Invalid mcp.json: {e}"))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != MCP_SCHEMA_V1 {
            return Err(format!(
                "mcp.json declared unsupported $schema {:?}; expected {MCP_SCHEMA_V1}",
                self.schema
            ));
        }
        if self.mcp_servers.is_empty() {
            return Err("mcp.json must declare at least one MCP server.".to_string());
        }
        for (server_name, server) in &self.mcp_servers {
            validate_server_name(server_name)?;
            server.validate(server_name)?;
        }
        Ok(())
    }
}

fn validate_server_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err(format!("Invalid MCP server name: {name:?}"));
    }
    Ok(())
}

impl McpServer {
    pub fn validate(&self, name: &str) -> Result<(), String> {
        match self {
            McpServer::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                if command.is_empty() {
                    return Err(format!("MCP server {name:?} command cannot be empty."));
                }
                if command.contains('\n') || command.contains('\r') {
                    return Err(format!(
                        "MCP server {name:?} command cannot contain newlines."
                    ));
                }
                if command.contains('/') || command.contains('\\') || command.starts_with('.') {
                    return Err(format!(
                        "MCP server {name:?} command must be a bare executable on PATH, not a package-relative path."
                    ));
                }
                if env.contains_key("PLUGIN_ROOT") || env.contains_key("PLUGIN_DATA") {
                    return Err(format!(
                        "MCP server {name:?} env cannot contain reserved PLUGIN_ROOT or PLUGIN_DATA keys."
                    ));
                }
                for arg in args {
                    reject_plugin_placeholder(name, "args", arg)?;
                }
                for value in env.values() {
                    reject_plugin_placeholder(name, "env", value)?;
                }
                if let Some(cwd_val) = cwd {
                    reject_plugin_placeholder(name, "cwd", cwd_val)?;
                    if cwd_val.starts_with("./")
                        || cwd_val == "${PLUGIN_ROOT}"
                        || cwd_val.starts_with("${PLUGIN_ROOT}/")
                        || cwd_val == "${PLUGIN_DATA}"
                        || cwd_val.starts_with("${PLUGIN_DATA}/")
                    {
                        return Err(format!(
                            "MCP server {name:?} cwd cannot be a plugin-relative path."
                        ));
                    }
                }
            }
            McpServer::StreamableHttp { url, headers } | McpServer::Sse { url, headers } => {
                let parsed = Url::parse(url).map_err(|error| {
                    format!("MCP server {name:?} has an invalid remote URL {url:?}: {error}")
                })?;
                let loopback_host = match parsed.host() {
                    Some(Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
                    Some(Host::Ipv4(address)) => address.is_loopback(),
                    Some(Host::Ipv6(address)) => address.is_loopback(),
                    None => false,
                };
                let loopback_http = parsed.scheme() == "http" && loopback_host;
                if parsed.scheme() != "https" && !loopback_http {
                    return Err(format!(
                        "MCP server {name:?} remote URL must use HTTPS or localhost: {url:?}"
                    ));
                }
                if parsed.host_str().is_none()
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                {
                    return Err(format!(
                        "MCP server {name:?} remote URL must have a host and cannot embed credentials: {url:?}"
                    ));
                }
                let mut seen_headers = BTreeSet::new();
                for header in headers.keys() {
                    let lower = header.to_ascii_lowercase();
                    if !seen_headers.insert(lower) {
                        return Err(format!(
                            "MCP server {name:?} has duplicate header (case-insensitive): {header:?}"
                        ));
                    }
                }
                for (header, value) in headers {
                    let sensitive = matches!(
                        header.to_ascii_lowercase().as_str(),
                        "authorization" | "proxy-authorization" | "x-api-key" | "api-key"
                    );
                    if sensitive && !is_environment_reference(value) {
                        return Err(format!(
                            "MCP server {name:?} sensitive header {header:?} must reference an environment variable instead of persisting a secret."
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

fn reject_plugin_placeholder(server: &str, field: &str, value: &str) -> Result<(), String> {
    if value.contains("${PLUGIN_ROOT}") || value.contains("${PLUGIN_DATA}") {
        return Err(format!(
            "MCP server {server:?} {field} cannot use ${{PLUGIN_ROOT}} or ${{PLUGIN_DATA}} placeholders."
        ));
    }
    Ok(())
}

/// A sensitive header is exactly `${NAME}`, or a scheme and `${NAME}` such
/// as `Bearer ${TOKEN}`, so the secret itself is never written down.
fn is_environment_reference(value: &str) -> bool {
    let value = value
        .split_once(' ')
        .filter(|(scheme, _)| {
            !scheme.is_empty() && scheme.bytes().all(|byte| byte.is_ascii_alphabetic())
        })
        .map_or(value, |(_, reference)| reference);
    matches!(environment_references(value).as_slice(), [(range, _)] if range.len() == value.len())
}

/// Each `${NAME}` in `text` with its byte range. `NAME` is upper-case
/// letters, digits, and `_`, the only references the portable document has.
pub(crate) fn environment_references(text: &str) -> Vec<(std::ops::Range<usize>, &str)> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(start) = text[from..].find("${").map(|offset| from + offset) {
        let name_start = start + 2;
        let name_end = text[name_start..]
            .find(|character: char| {
                !(character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_')
            })
            .map_or(text.len(), |offset| name_start + offset);
        if name_end > name_start && text[name_end..].starts_with('}') {
            found.push((start..name_end + 1, &text[name_start..name_end]));
            from = name_end + 1;
        } else {
            from = name_start;
        }
    }
    found
}

impl McpServer {
    /// Every environment variable the server reads through a `${NAME}`
    /// reference, which the person has to set before it works.
    pub(crate) fn environment_names(&self) -> BTreeSet<String> {
        let texts: Vec<&str> = match self {
            McpServer::Stdio {
                command,
                args,
                env,
                cwd,
            } => std::iter::once(command.as_str())
                .chain(args.iter().map(String::as_str))
                .chain(env.values().map(String::as_str))
                .chain(cwd.as_deref())
                .collect(),
            McpServer::StreamableHttp { url, headers } | McpServer::Sse { url, headers } => {
                std::iter::once(url.as_str())
                    .chain(headers.values().map(String::as_str))
                    .collect()
            }
        };
        texts
            .into_iter()
            .flat_map(environment_references)
            .map(|(_, name)| name.to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_mcp() {
        let mcp_json = r#"{
            "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": {
                "server": {
                    "type": "stdio",
                    "command": "npx",
                    "args": ["@acme/server"],
                    "env": { "MODE": "safe" }
                }
            }
        }"#;
        let mcp = McpConfig::from_slice(mcp_json.as_bytes()).expect("valid mcp");
        assert_eq!(mcp.mcp_servers.len(), 1);
    }

    #[test]
    fn plugin_relative_commands_and_placeholders_are_rejected() {
        let relative = r#"{
            "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": {
                "server": {
                    "type": "stdio",
                    "command": "./bin/server"
                }
            }
        }"#;
        assert!(McpConfig::from_slice(relative.as_bytes())
            .expect_err("relative command")
            .contains("bare executable"));

        let placeholder = r#"{
            "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
            "mcpServers": {
                "server": {
                    "type": "stdio",
                    "command": "npx",
                    "args": ["--data", "${PLUGIN_DATA}/db"]
                }
            }
        }"#;
        assert!(McpConfig::from_slice(placeholder.as_bytes())
            .expect_err("placeholder")
            .contains("PLUGIN_DATA"));
    }

    #[test]
    fn remote_mcp_urls_require_https_or_an_exact_loopback_host() {
        for url in [
            "http://localhost.evil.example/server",
            "http://example.com/server",
            "https://user:secret@example.com/server",
        ] {
            let server = McpServer::StreamableHttp {
                url: url.to_string(),
                headers: BTreeMap::new(),
            };
            assert!(server.validate("remote").is_err(), "accepted {url}");
        }
        for url in [
            "https://example.com/server",
            "http://localhost:3000/server",
            "http://127.0.0.1:3000/server",
            "http://[::1]:3000/server",
        ] {
            let server = McpServer::StreamableHttp {
                url: url.to_string(),
                headers: BTreeMap::new(),
            };
            server.validate("remote").expect("valid remote URL");
        }
    }

    #[test]
    fn sensitive_headers_may_carry_a_scheme_before_the_reference() {
        for (value, ok) in [
            ("${TOKEN}", true),
            ("Bearer ${ACME_TOKEN}", true),
            ("Basic ${CREDENTIALS}", true),
            ("Bearer abc123", false),
            ("${TOKEN} extra", false),
            ("Bearer ${lower}", false),
            ("${}", false),
        ] {
            let server = McpServer::StreamableHttp {
                url: "https://example.com/mcp".to_string(),
                headers: BTreeMap::from([("Authorization".to_string(), value.to_string())]),
            };
            assert_eq!(server.validate("remote").is_ok(), ok, "{value}");
        }
    }

    #[test]
    fn environment_names_come_from_every_field_that_can_hold_a_reference() {
        let server = McpServer::Stdio {
            command: "uvx".to_string(),
            args: vec!["--region=${REGION}".to_string(), "$HOME".to_string()],
            env: BTreeMap::from([("API_KEY".to_string(), "${API_KEY}".to_string())]),
            cwd: None,
        };
        assert_eq!(
            server.environment_names().into_iter().collect::<Vec<_>>(),
            ["API_KEY", "REGION"]
        );
        assert_eq!(
            environment_references("a ${X} b ${Y_2}c ${z} ${"),
            vec![(2..6, "X"), (9..15, "Y_2")]
        );
    }
}
