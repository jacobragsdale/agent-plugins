//! Detected agent profiles. Detection is the configuration set.

use crate::fs_retry;
use crate::paths::SystemPaths;
use crate::process;
use crate::sources::{sync_directory, temporary_path};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const PROFILES_FILE: &str = "agent-profiles.json";
const PROFILES_BACKUP_FILE: &str = "agent-profiles.json.previous";
const PROFILES_VERSION: u8 = 1;
const DETECTION_TIMEOUT: Duration = Duration::from_secs(3);
const DETECTION_CACHE_TTL: Duration = Duration::from_secs(60);

/// MSIX package family of Claude Desktop on Windows.
pub(crate) const CLAUDE_DESKTOP_MSIX: &str = "Claude_pzs8sxrjxfjjc";
const CHATGPT_MSIX: [&str; 2] = [
    "OpenAI.ChatGPT-Desktop_2p2nqsd0c76g0",
    "OpenAI.Codex_2p2nqsd0c76g0",
];
/// What to do when no supported app is installed, primary apps first.
pub(crate) const INSTALL_AN_APP: &str = "Install GitHub Copilot, Cursor, or Claude (Claude Code or Claude Desktop), or another supported app such as OpenCode, pi, Codex, ChatGPT, or Grok Build, then refresh.";
const NOT_DETECTED: Detection = Detection {
    detected: false,
    version: None,
    message: None,
    inconclusive: false,
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TargetId {
    Cursor,
    ClaudeCode,
    Codex,
    #[serde(rename = "opencode")]
    OpenCode,
    GrokBuild,
    GithubCopilot,
    ClaudeDesktop,
    Chatgpt,
    Pi,
}

impl TargetId {
    /// The primary targets come first; the window lists apps in this order.
    pub(crate) const ALL: [Self; 9] = [
        Self::GithubCopilot,
        Self::Cursor,
        Self::ClaudeCode,
        Self::ClaudeDesktop,
        Self::OpenCode,
        Self::Pi,
        Self::Codex,
        Self::Chatgpt,
        Self::GrokBuild,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
            Self::GrokBuild => "grok-build",
            Self::GithubCopilot => "github-copilot",
            Self::ClaudeDesktop => "claude-desktop",
            Self::Chatgpt => "chatgpt",
            Self::Pi => "pi",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Cursor => "Cursor",
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::OpenCode => "OpenCode",
            Self::GrokBuild => "Grok Build",
            Self::GithubCopilot => "GitHub Copilot",
            Self::ClaudeDesktop => "Claude Desktop",
            // The app's skills and MCP servers live in its Codex mode.
            Self::Chatgpt => "ChatGPT (Codex)",
            Self::Pi => "pi",
        }
    }

    /// The CLI whose `--version` proves the agent is installed. Desktop apps
    /// have none and are detected from their installation instead.
    fn command(self) -> Option<&'static str> {
        match self {
            Self::Cursor => Some("cursor"),
            Self::ClaudeCode => Some("claude"),
            Self::Codex => Some("codex"),
            Self::OpenCode => Some("opencode"),
            Self::GrokBuild => Some("grok"),
            Self::GithubCopilot => Some("copilot"),
            Self::Pi => Some("pi"),
            Self::ClaudeDesktop | Self::Chatgpt => None,
        }
    }

    /// The dialect carries the month its configuration contract was verified.
    pub(crate) fn current_dialect(self) -> String {
        let verified = match self {
            Self::ClaudeDesktop | Self::Chatgpt | Self::Pi => "2026-09",
            _ => "2026-08",
        };
        format!("{}-{verified}", self.as_str())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(crate) struct AgentProfile {
    pub(crate) target_id: TargetId,
    pub(crate) enabled: bool,
    pub(crate) scopes: Vec<String>,
    pub(crate) dialect_id: String,
}

impl AgentProfile {
    fn disabled(target_id: TargetId) -> Self {
        Self {
            target_id,
            enabled: false,
            scopes: vec!["user".to_string()],
            dialect_id: target_id.current_dialect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentProfileState {
    pub(crate) target_id: TargetId,
    pub(crate) display_name: String,
    pub(crate) enabled: bool,
    pub(crate) scopes: Vec<String>,
    pub(crate) dialect_id: String,
    pub(crate) detected: bool,
    pub(crate) detected_version: Option<String>,
    pub(crate) detection_message: Option<String>,
    pub(crate) verification_guidance: String,
    pub(crate) reload_guidance: String,
    pub(crate) skill_directory: String,
    pub(crate) skill_directory_shared: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProfilesFile {
    version: u8,
    profiles: Vec<AgentProfile>,
}

/// Profiles are read one by one, so an app this version no longer supports
/// (Microsoft 365 Copilot) drops out instead of making the file unusable.
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct StoredProfilesFile {
    version: u8,
    profiles: Vec<serde_json::Value>,
}

/// Serializes the read-merge-write of the profiles file. Detection runs
/// outside it, so a slow probe never holds it.
static PROFILES_LOCK: Mutex<()> = Mutex::new(());

fn profiles_lock() -> std::sync::MutexGuard<'static, ()> {
    PROFILES_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Never fails: an unusable profiles file falls back to the previous copy,
/// then to nothing, which the next detection pass fills in again.
pub(crate) fn read(paths: &SystemPaths) -> Vec<AgentProfile> {
    let (configured, _) = read_configured(paths);
    materialize(&configured)
}

/// Enables what detection finds and disables what it no longer finds. A probe
/// that timed out or failed keeps the agent as it was. Never fails: a profile
/// file that cannot be written is logged and retried on the next pass.
pub(crate) fn apply_detected_defaults(paths: &SystemPaths) {
    apply_detections(paths, &detect_all());
}

/// Looks at the machine again: forgets cached detection, then applies what it
/// finds. Returns true when an agent was enabled or disabled.
pub(crate) fn refresh_detection() -> bool {
    clear_detection_cache();
    match SystemPaths::from_system() {
        Ok(paths) => apply_detections(&paths, &detect_all()).1,
        Err(error) => {
            eprintln!("Could not look for installed agents: {error}");
            false
        }
    }
}

fn apply_detections(
    paths: &SystemPaths,
    detections: &BTreeMap<TargetId, Detection>,
) -> (Vec<AgentProfile>, bool) {
    let detected = detections
        .iter()
        .filter(|(_, detection)| detection.detected)
        .map(|(target, _)| *target)
        .collect::<Vec<_>>();
    let inconclusive = detections
        .iter()
        .filter(|(_, detection)| detection.inconclusive)
        .map(|(target, _)| *target)
        .collect::<Vec<_>>();
    let _guard = profiles_lock();
    let (configured, damaged) = read_configured(paths);
    let enabled_before = enabled_targets(&configured);
    let (next, changed) = merge_detected_defaults(configured, &detected, &inconclusive);
    if changed || damaged {
        if let Err(error) = write(
            paths,
            &next.values().cloned().collect::<Vec<AgentProfile>>(),
        ) {
            eprintln!("Agent Plugins could not save the detected agents: {error}");
        }
    }
    let enabled_changed = enabled_targets(&next) != enabled_before;
    (materialize(&next), enabled_changed)
}

fn enabled_targets(configured: &BTreeMap<TargetId, AgentProfile>) -> Vec<TargetId> {
    configured
        .values()
        .filter(|profile| profile.enabled)
        .map(|profile| profile.target_id)
        .collect()
}

fn detect_all() -> BTreeMap<TargetId, Detection> {
    let detections = crate::parallel::map(&TargetId::ALL, |target| detect(*target));
    TargetId::ALL.into_iter().zip(detections).collect()
}

/// Only tests choose an agent by hand; the app enables what it detects.
#[cfg(test)]
pub(crate) fn set_enabled(
    paths: &SystemPaths,
    target_id: TargetId,
    enabled: bool,
) -> Result<Vec<AgentProfile>, String> {
    let _guard = profiles_lock();
    let (mut configured, _) = read_configured(paths);
    configured
        .entry(target_id)
        .or_insert_with(|| AgentProfile::disabled(target_id))
        .enabled = enabled;
    write(
        paths,
        &configured.values().cloned().collect::<Vec<AgentProfile>>(),
    )?;
    Ok(materialize(&configured))
}

pub(crate) fn states(paths: &SystemPaths) -> Result<Vec<AgentProfileState>, String> {
    let mut detections = detect_all();
    read(paths)
        .into_iter()
        .map(|profile| {
            let detection = detections
                .remove(&profile.target_id)
                .unwrap_or(NOT_DETECTED);
            Ok(AgentProfileState {
                target_id: profile.target_id,
                display_name: profile.target_id.display_name().to_string(),
                enabled: profile.enabled,
                scopes: profile.scopes,
                dialect_id: profile.dialect_id,
                detected: detection.detected,
                detected_version: detection.version,
                // A probe that timed out says nothing about an app that was
                // never set up here; only a configured one "couldn't be checked".
                detection_message: detection
                    .message
                    .filter(|_| profile.enabled || !detection.inconclusive),
                verification_guidance: verification_guidance(profile.target_id).to_string(),
                reload_guidance: reload_guidance(profile.target_id).to_string(),
                skill_directory: crate::adapters::skill_display_root(profile.target_id).to_string(),
                skill_directory_shared: crate::adapters::reads_shared_agents(profile.target_id),
            })
        })
        .collect()
}

fn verification_guidance(target: TargetId) -> &'static str {
    match target {
        TargetId::Cursor => "Inspect the Plugins and MCP settings surfaces.",
        TargetId::ClaudeCode => {
            "Run `claude mcp list` and inspect the effective user instructions."
        }
        TargetId::Codex => "Inspect configured MCP servers and the effective instruction chain.",
        TargetId::OpenCode => "Run `opencode mcp list` and inspect loaded skills and instructions.",
        TargetId::GrokBuild => "Inspect the configured skills and MCP servers in Grok Build.",
        TargetId::GithubCopilot => {
            "Inspect Copilot skills in VS Code, Visual Studio, JetBrains, or Copilot CLI."
        }
        TargetId::ClaudeDesktop => {
            "Open Claude Desktop Settings > Developer to see local MCP servers. Skills are managed at claude.ai under Customize > Skills."
        }
        TargetId::Chatgpt => "Open ChatGPT, switch to Codex, and check its skills and MCP servers.",
        TargetId::Pi => "Start pi and type `/skill:` to list the skills it found.",
    }
}

fn reload_guidance(target: TargetId) -> &'static str {
    match target {
        TargetId::Cursor => "Reload the Cursor window after configuration changes.",
        TargetId::GithubCopilot => {
            "Reload the IDE window or start a fresh Copilot session after configuration changes."
        }
        TargetId::ClaudeCode | TargetId::Codex | TargetId::OpenCode | TargetId::GrokBuild => {
            "Start a fresh client session after configuration changes."
        }
        TargetId::Pi => "Start a new pi session, or run /reload in the current one.",
        TargetId::ClaudeDesktop | TargetId::Chatgpt => {
            "Quit and reopen the app after configuration changes."
        }
    }
}

#[derive(Clone)]
struct Detection {
    detected: bool,
    version: Option<String>,
    message: Option<String>,
    /// The probe timed out or failed, which says nothing about whether the
    /// agent is installed.
    inconclusive: bool,
}

/// Detection runs `<agent> --version` for most targets, so a burst of state
/// reloads would spawn a process per agent per reload. Remember what it found
/// for a short while; a sync or an on-demand preflight starts over.
fn detection_cache() -> &'static Mutex<BTreeMap<TargetId, (Instant, Detection)>> {
    static CACHE: OnceLock<Mutex<BTreeMap<TargetId, (Instant, Detection)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Forgets what detection found, so the next read probes the machine again.
pub(crate) fn clear_detection_cache() {
    if let Ok(mut cache) = detection_cache().lock() {
        cache.clear();
    }
}

fn detect(target: TargetId) -> Detection {
    if let Ok(cache) = detection_cache().lock() {
        if let Some((found_at, detection)) = cache.get(&target) {
            if found_at.elapsed() < DETECTION_CACHE_TTL {
                return detection.clone();
            }
        }
    }
    let detection = detect_now(target);
    if let Ok(mut cache) = detection_cache().lock() {
        cache.insert(target, (Instant::now(), detection.clone()));
    }
    detection
}

fn detect_now(target: TargetId) -> Detection {
    if let Some(detection) = detect_application(target) {
        return detection;
    }
    detect_command(target)
}

fn detect_application(target: TargetId) -> Option<Detection> {
    match target {
        TargetId::Cursor => detect_cursor_application(),
        TargetId::GithubCopilot => detect_copilot_application(),
        TargetId::ClaudeDesktop => Some(detect_claude_desktop_application()),
        TargetId::Chatgpt => Some(detect_chatgpt_application()),
        TargetId::Pi => detect_pi_application(),
        _ => None,
    }
}

fn detect_claude_desktop_application() -> Detection {
    detect_msix_from(
        &msix_package_full_names(),
        &msix_packages_dir(),
        &[CLAUDE_DESKTOP_MSIX],
    )
    .or_else(|| {
        let local = dirs::data_local_dir()?;
        detect_squirrel_claude_from(&local.join("AnthropicClaude"))
    })
    .or_else(|| detect_app_bundle(Path::new("/Applications/Claude.app")))
    .unwrap_or(NOT_DETECTED)
}

fn detect_chatgpt_application() -> Detection {
    detect_msix_from(
        &msix_package_full_names(),
        &msix_packages_dir(),
        &CHATGPT_MSIX,
    )
    .or_else(|| detect_app_bundle(Path::new("/Applications/ChatGPT.app")))
    .unwrap_or(NOT_DETECTED)
}

/// `pi` is a common word, so a command alone is not proof: pi creates
/// `~/.pi/agent` on its first run. Without it, pi is not set up here; with it,
/// the command only supplies the version.
fn detect_pi_application() -> Option<Detection> {
    Some(detect_pi_from(dirs::home_dir().as_deref()))
}

fn detect_pi_from(home: Option<&Path>) -> Detection {
    if !home.is_some_and(|home| home.join(".pi").join("agent").is_dir()) {
        return NOT_DETECTED;
    }
    let detection = detect_command(TargetId::Pi);
    Detection {
        detected: true,
        inconclusive: false,
        message: None,
        ..detection
    }
}

/// MSIX packages registered for this user, as full names such as
/// `Claude_1.8555.2.0_x64__pzs8sxrjxfjjc`. Reading the repository key needs
/// no subprocess, unlike `Get-AppxPackage`.
#[cfg(windows)]
fn msix_package_full_names() -> Vec<String> {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(
            r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages",
        )
        .map(|key| key.enum_keys().flatten().collect())
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn msix_package_full_names() -> Vec<String> {
    Vec::new()
}

fn msix_packages_dir() -> PathBuf {
    dirs::data_local_dir()
        .map(|local| local.join("Packages"))
        .unwrap_or_default()
}

/// A family is `<name>_<publisher>`; a registered full name is
/// `<name>_<version>_<arch>__<publisher>`. The family folder under Packages
/// appears on first launch and counts without a version.
fn detect_msix_from(
    full_names: &[String],
    packages_dir: &Path,
    families: &[&str],
) -> Option<Detection> {
    families.iter().find_map(|family| {
        let (name, publisher) = family.rsplit_once('_')?;
        let version = full_names.iter().find_map(|full| {
            let rest = full.strip_prefix(name)?.strip_prefix('_')?;
            let (version, _) = rest.split_once('_')?;
            full.ends_with(&format!("__{publisher}"))
                .then(|| version.to_string())
        });
        (version.is_some() || packages_dir.join(family).is_dir()).then_some(Detection {
            detected: true,
            version,
            message: None,
            inconclusive: false,
        })
    })
}

/// The Squirrel installer Anthropic used before February 2026.
fn detect_squirrel_claude_from(root: &Path) -> Option<Detection> {
    root.join("claude.exe").is_file().then(|| Detection {
        detected: true,
        version: first_dir_with_prefix(root, "app-").and_then(|dir| {
            dir.file_name()?
                .to_str()
                .map(|name| name["app-".len()..].to_string())
        }),
        message: None,
        inconclusive: false,
    })
}

fn detect_app_bundle(app: &Path) -> Option<Detection> {
    app.is_dir().then_some(Detection {
        detected: true,
        version: None,
        message: None,
        inconclusive: false,
    })
}

fn detect_cursor_application() -> Option<Detection> {
    detect_cursor_application_from(&cursor_install_roots())
}

fn detect_cursor_application_from(roots: &[PathBuf]) -> Option<Detection> {
    roots.iter().find_map(|root| {
        let product = find_vscode_like_product_json(root)?;
        Some(Detection {
            detected: true,
            version: read_json_string_field(&product, "version"),
            message: None,
            inconclusive: false,
        })
    })
}

/// Folders Cursor may be installed in, most likely first.
pub(crate) fn cursor_install_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Applications/Cursor.app"));
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join("Applications/Cursor.app"));
        }
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = dirs::data_local_dir() {
            roots.push(local.join("Programs").join("cursor"));
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            roots.push(PathBuf::from(program_files).join("Cursor"));
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles(x86)") {
            roots.push(PathBuf::from(program_files).join("Cursor"));
        }
        roots.extend(registered_install_roots("Cursor", "cursor"));
        // The `cursor` command on PATH is `<install>\resources\app\bin\cursor.cmd`.
        if let Some(path) = std::env::var_os("PATH") {
            roots.extend(
                std::env::split_paths(&path)
                    .filter(|dir| dir.join("cursor.cmd").is_file())
                    .filter_map(|bin| bin.ancestors().nth(3).map(Path::to_path_buf)),
            );
        }
    }
    #[cfg(target_os = "linux")]
    {
        roots.push(PathBuf::from("/usr/share/cursor"));
        roots.push(PathBuf::from("/opt/Cursor"));
        if let Some(home) = dirs::home_dir() {
            roots.push(home.join(".local/share/cursor"));
        }
    }
    roots
}

/// Where Windows records an app outside its default folders: its uninstall
/// entries, per user and per machine (`Cursor (User)` for a per-user install),
/// and the program registered for its link scheme, which a portable copy sets
/// on first launch.
#[cfg(windows)]
fn registered_install_roots(display_name: &str, scheme: &str) -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let mut roots = Vec::new();
    for (hive, uninstall) in [
        (
            HKEY_CURRENT_USER,
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
        (
            HKEY_LOCAL_MACHINE,
            r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        ),
    ] {
        let Ok(uninstall) = RegKey::predef(hive).open_subkey(uninstall) else {
            continue;
        };
        for entry in uninstall
            .enum_keys()
            .flatten()
            .filter_map(|name| uninstall.open_subkey(name).ok())
        {
            let name: String = entry.get_value("DisplayName").unwrap_or_default();
            if name == display_name || name.starts_with(&format!("{display_name} (")) {
                if let Ok(location) = entry.get_value::<String, _>("InstallLocation") {
                    roots.push(PathBuf::from(location));
                }
            }
        }
    }
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        // Such as `"C:\...\Cursor.exe" --open-url -- "%1"`.
        let Ok(command) = RegKey::predef(hive)
            .open_subkey(format!(r"Software\Classes\{scheme}\shell\open\command"))
            .and_then(|key| key.get_value::<String, _>(""))
        else {
            continue;
        };
        let program = match command.strip_prefix('"') {
            Some(quoted) => quoted.split('"').next(),
            None => command.split_whitespace().next(),
        };
        if let Some(folder) = program.and_then(|program| Path::new(program).parent()) {
            roots.push(folder.to_path_buf());
        }
    }
    roots
}

fn find_vscode_like_product_json(root: &Path) -> Option<PathBuf> {
    [
        root.join("Contents/Resources/app/product.json"),
        root.join("resources/app/product.json"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .or_else(|| {
        // Windows installs now keep the app in a versioned folder: <root>\<commit>\resources\app.
        fs::read_dir(root)
            .ok()?
            .flatten()
            .map(|entry| entry.path().join("resources/app/product.json"))
            .find(|path| path.is_file())
    })
}

fn read_json_string_field(path: &Path, field: &str) -> Option<String> {
    let value = serde_json::from_slice::<serde_json::Value>(&fs::read(path).ok()?).ok()?;
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn detect_copilot_application() -> Option<Detection> {
    let home = dirs::home_dir()?;
    detect_vscode_copilot_from(&vscode_editions(&home)).or_else(|| {
        detect_jetbrains_copilot_from(&jetbrains_app_roots(&home), &jetbrains_config_roots(&home))
    })
}

struct VscodeEdition {
    app_roots: Vec<PathBuf>,
    extensions: PathBuf,
}

fn vscode_editions(home: &Path) -> Vec<VscodeEdition> {
    vec![
        VscodeEdition {
            app_roots: vscode_stable_app_roots(home),
            extensions: home.join(".vscode/extensions"),
        },
        VscodeEdition {
            app_roots: vscode_insiders_app_roots(home),
            extensions: home.join(".vscode-insiders/extensions"),
        },
    ]
}

fn vscode_stable_app_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Applications/Visual Studio Code.app"));
        roots.push(home.join("Applications/Visual Studio Code.app"));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = dirs::data_local_dir() {
            roots.push(local.join("Programs").join("Microsoft VS Code"));
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            roots.push(PathBuf::from(program_files).join("Microsoft VS Code"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        roots.push(PathBuf::from("/usr/share/code"));
        roots.push(home.join(".local/share/code"));
    }
    let _ = home;
    roots
}

fn vscode_insiders_app_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from(
            "/Applications/Visual Studio Code - Insiders.app",
        ));
        roots.push(home.join("Applications/Visual Studio Code - Insiders.app"));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(local) = dirs::data_local_dir() {
            roots.push(local.join("Programs").join("Microsoft VS Code Insiders"));
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            roots.push(PathBuf::from(program_files).join("Microsoft VS Code Insiders"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        roots.push(PathBuf::from("/usr/share/code-insiders"));
        roots.push(home.join(".local/share/code-insiders"));
    }
    let _ = home;
    roots
}

fn detect_vscode_copilot_from(editions: &[VscodeEdition]) -> Option<Detection> {
    editions.iter().find_map(|edition| {
        let product = edition
            .app_roots
            .iter()
            .find_map(|root| find_vscode_like_product_json(root))?;
        // VS Code now ships Copilot Chat built in; older installs have it as a user extension.
        let builtin = product
            .parent()
            .map(|app| app.join("extensions/copilot"))
            .filter(|dir| dir.join("package.json").is_file());
        builtin
            .or_else(|| first_dir_with_prefix(&edition.extensions, "github.copilot"))
            .map(|dir| Detection {
                detected: true,
                version: read_json_string_field(&dir.join("package.json"), "version"),
                message: None,
                inconclusive: false,
            })
    })
}

fn detect_jetbrains_copilot_from(
    app_roots: &[PathBuf],
    config_roots: &[PathBuf],
) -> Option<Detection> {
    for app in app_roots {
        let Some(data_directory) = jetbrains_data_directory_name(app) else {
            continue;
        };
        for config_root in config_roots {
            let config_dir = config_root.join(&data_directory);
            if let Some(plugin) = jetbrains_copilot_plugin(&config_dir) {
                return Some(Detection {
                    detected: true,
                    version: read_json_string_field(&plugin.join("package.json"), "version"),
                    message: None,
                    inconclusive: false,
                });
            }
        }
    }
    None
}

fn jetbrains_data_directory_name(app_root: &Path) -> Option<String> {
    read_json_string_field(&find_jetbrains_product_info(app_root)?, "dataDirectoryName")
}

fn find_jetbrains_product_info(app_root: &Path) -> Option<PathBuf> {
    [
        app_root.join("Contents/Resources/product-info.json"),
        app_root.join("product-info.json"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn jetbrains_copilot_plugin(config_dir: &Path) -> Option<PathBuf> {
    let plugins = config_dir.join("plugins");
    first_dir_with_prefix(&plugins, "github-copilot")
        .or_else(|| first_dir_with_prefix(&plugins, "copilot-intellij"))
}

fn jetbrains_app_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for search in jetbrains_app_search_roots(home) {
        roots.extend(jetbrains_apps_in(&search));
    }
    roots.extend(toolbox_ide_roots(&home.join(toolbox_apps_relative())));
    roots.sort();
    roots.dedup();
    roots
}

fn jetbrains_app_search_roots(home: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Applications"));
        roots.push(home.join("Applications"));
    }
    #[cfg(target_os = "windows")]
    {
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            roots.push(PathBuf::from(program_files).join("JetBrains"));
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles(x86)") {
            roots.push(PathBuf::from(program_files).join("JetBrains"));
        }
        if let Some(local) = dirs::data_local_dir() {
            roots.push(local.join("Programs"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        roots.push(PathBuf::from("/opt"));
        roots.push(home.join(".local/share"));
    }
    let _ = home;
    roots
}

fn toolbox_apps_relative() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("Library/Application Support/JetBrains/Toolbox/apps")
    }
    #[cfg(target_os = "windows")]
    {
        PathBuf::from("AppData/Local/JetBrains/Toolbox/apps")
    }
    #[cfg(target_os = "linux")]
    {
        PathBuf::from(".local/share/JetBrains/Toolbox/apps")
    }
}

fn jetbrains_apps_in(search_root: &Path) -> Vec<PathBuf> {
    read_child_paths(search_root)
        .into_iter()
        .filter(|path| find_jetbrains_product_info(path).is_some())
        .collect()
}

fn toolbox_ide_roots(toolbox_apps: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for product in read_child_paths(toolbox_apps) {
        for channel in read_child_paths(&product) {
            for build in read_child_paths(&channel) {
                if find_jetbrains_product_info(&build).is_some() {
                    roots.push(build);
                    continue;
                }
                roots.extend(jetbrains_apps_in(&build));
            }
        }
    }
    roots
}

fn read_child_paths(parent: &Path) -> Vec<PathBuf> {
    fs::read_dir(parent)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .collect()
}

fn jetbrains_config_roots(home: &Path) -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![home.join("Library/Application Support/JetBrains")]
    }
    #[cfg(target_os = "windows")]
    {
        let _ = home;
        dirs::config_dir()
            .map(|config| vec![config.join("JetBrains")])
            .unwrap_or_default()
    }
    #[cfg(target_os = "linux")]
    {
        vec![
            home.join(".config/JetBrains"),
            home.join(".local/share/JetBrains"),
        ]
    }
}

fn first_dir_with_prefix(parent: &Path, prefix: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(parent).ok()?;
    let prefix = prefix.to_ascii_lowercase();
    entries.flatten().map(|entry| entry.path()).find(|path| {
        path.is_dir()
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().starts_with(&prefix))
    })
}

fn is_missing_program_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("no such file")
        || error.contains("not found")
        || error.contains("cannot find the file")
        || error.contains("cannot find the path")
        || error.contains("the system cannot find")
}

fn detect_command(target: TargetId) -> Detection {
    let Some(program) = target.command() else {
        return NOT_DETECTED;
    };
    // The PATH this process started with misses anything installed since, and
    // `Command` does not resolve `.cmd` shims on Windows, so look it up fresh.
    let Some(program) = crate::startup::find_program(program) else {
        return NOT_DETECTED;
    };
    let mut command = process::command(&program);
    command.arg("--version");
    match process::run(
        command,
        &format!("{} detection", target.display_name()),
        DETECTION_TIMEOUT,
    ) {
        Ok(output) if output.status.success() => {
            let version =
                first_nonempty_line(&output.stdout).or_else(|| first_nonempty_line(&output.stderr));
            Detection {
                detected: true,
                version,
                message: None,
                inconclusive: false,
            }
        }
        Ok(_) => NOT_DETECTED,
        Err(error) if is_missing_program_error(&error) => NOT_DETECTED,
        Err(error) => Detection {
            detected: false,
            version: None,
            message: Some(error),
            inconclusive: true,
        },
    }
}

/// The saved profiles, or the previous copy when the current file is missing
/// or unusable. Profiles only record what detection found, so when neither
/// copy is usable the app starts empty and detection rebuilds them. The flag
/// is true when the current file needs rewriting.
fn read_configured(paths: &SystemPaths) -> (BTreeMap<TargetId, AgentProfile>, bool) {
    let data_base = paths.app_data();
    let mut damaged = false;
    for name in [PROFILES_FILE, PROFILES_BACKUP_FILE] {
        match read_profiles_file(&data_base.join(name)) {
            Ok(Some(profiles)) => return (profiles, damaged),
            Ok(None) => {}
            Err(error) => {
                eprintln!("Agent Plugins ignored an unusable agent profile file: {error}")
            }
        }
        damaged = true;
    }
    (BTreeMap::new(), damaged)
}

/// `Ok(None)` when the file does not exist. A profile that fails validation is
/// dropped, and detection adds the agent back.
fn read_profiles_file(path: &Path) -> Result<Option<BTreeMap<TargetId, AgentProfile>>, String> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not read {}: {error}.", path.display())),
    };
    let file = serde_json::from_slice::<StoredProfilesFile>(&contents)
        .map_err(|error| format!("Could not parse {}: {error}.", path.display()))?;
    if file.version != PROFILES_VERSION {
        return Err(format!(
            "{} uses an unsupported profile version.",
            path.display()
        ));
    }
    let mut profiles = BTreeMap::new();
    for profile in file.profiles {
        let Ok(profile) = serde_json::from_value::<AgentProfile>(profile) else {
            continue;
        };
        if profile.scopes == ["user"] && !profile.dialect_id.is_empty() {
            profiles.entry(profile.target_id).or_insert(profile);
        }
    }
    Ok(Some(profiles))
}

fn materialize(configured: &BTreeMap<TargetId, AgentProfile>) -> Vec<AgentProfile> {
    TargetId::ALL
        .into_iter()
        .map(|target| {
            configured
                .get(&target)
                .cloned()
                .unwrap_or_else(|| AgentProfile::disabled(target))
        })
        .collect()
}

fn merge_detected_defaults(
    mut configured: BTreeMap<TargetId, AgentProfile>,
    detected: &[TargetId],
    inconclusive: &[TargetId],
) -> (BTreeMap<TargetId, AgentProfile>, bool) {
    let mut changed = false;
    for target in TargetId::ALL {
        let should_enable = detected.contains(&target);
        match configured.entry(target) {
            std::collections::btree_map::Entry::Vacant(entry) if should_enable => {
                entry.insert(AgentProfile {
                    target_id: target,
                    enabled: true,
                    scopes: vec!["user".to_string()],
                    dialect_id: target.current_dialect(),
                });
                changed = true;
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let profile = entry.get_mut();
                // The adapters block a dialect they do not recognize, and a
                // stored one only ever lags behind this build.
                if profile.dialect_id != target.current_dialect() {
                    profile.dialect_id = target.current_dialect();
                    changed = true;
                }
                if profile.enabled != should_enable && !inconclusive.contains(&target) {
                    profile.enabled = should_enable;
                    changed = true;
                }
            }
            _ => {}
        }
    }
    (configured, changed)
}

fn first_nonempty_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

fn write(paths: &SystemPaths, profiles: &[AgentProfile]) -> Result<(), String> {
    let data_base = paths.app_data();
    fs::create_dir_all(&data_base)
        .map_err(|error| format!("Could not create {}: {error}", data_base.display()))?;
    let file = ProfilesFile {
        version: PROFILES_VERSION,
        profiles: profiles.to_vec(),
    };
    let mut contents = serde_json::to_vec_pretty(&file)
        .map_err(|error| format!("Could not serialize agent profiles: {error}"))?;
    contents.push(b'\n');
    atomic_write(
        &data_base,
        &data_base.join(PROFILES_FILE),
        &data_base.join(PROFILES_BACKUP_FILE),
        &contents,
    )
}

/// Replaces `path` and keeps the file it replaces as `backup`, the last good
/// copy. A current file that does not parse is not worth keeping, so the
/// backup stays as it was. Reads fall back to the backup while `path` is
/// briefly missing.
fn atomic_write(
    directory: &Path,
    path: &Path,
    backup: &Path,
    contents: &[u8],
) -> Result<(), String> {
    let staging = temporary_path(directory, "agent-profiles-writing");
    let staged = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging)
        .and_then(|mut file| file.write_all(contents).and_then(|()| file.sync_all()));
    if let Err(error) = staged {
        let _ = fs_retry::remove_file(&staging);
        return Err(format!("Could not write {}: {error}", staging.display()));
    }
    if matches!(read_profiles_file(path), Ok(Some(_))) {
        if let Err(error) = fs_retry::rename(path, backup) {
            let _ = fs_retry::remove_file(&staging);
            return Err(format!(
                "Could not keep a copy of {}: {error}",
                path.display()
            ));
        }
    }
    if let Err(error) = fs_retry::rename(&staging, path) {
        let _ = fs_retry::remove_file(&staging);
        return Err(format!("Could not activate {}: {error}", path.display()));
    }
    sync_directory(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::fs;

    fn paths(root: &Path) -> SystemPaths {
        SystemPaths {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            local_data: root.join("local-data"),
            cache: root.join("cache"),
        }
    }

    #[test]
    fn profiles_default_disabled_and_persist_explicit_selection() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        let initial = read(&paths);
        assert_eq!(initial.len(), TargetId::ALL.len());
        assert!(initial.iter().all(|profile| !profile.enabled));
        set_enabled(&paths, TargetId::Codex, true).expect("enable");
        let reloaded = read(&paths);
        assert!(reloaded
            .iter()
            .any(|profile| profile.target_id == TargetId::Codex && profile.enabled));
        assert!(reloaded
            .iter()
            .filter(|profile| profile.target_id != TargetId::Codex)
            .all(|profile| !profile.enabled));
    }

    #[test]
    fn detected_agents_are_configured_and_undetected_agents_are_not() {
        let configured = BTreeMap::from([(
            TargetId::Codex,
            AgentProfile {
                target_id: TargetId::Codex,
                enabled: false,
                scopes: vec!["user".to_string()],
                dialect_id: TargetId::Codex.current_dialect(),
            },
        )]);
        let (next, changed) =
            merge_detected_defaults(configured, &[TargetId::Cursor, TargetId::Codex], &[]);
        assert!(changed);
        assert!(next[&TargetId::Cursor].enabled);
        assert!(next[&TargetId::Codex].enabled);
        let (after_loss, lost) = merge_detected_defaults(next, &[TargetId::Cursor], &[]);
        assert!(lost);
        assert!(after_loss[&TargetId::Cursor].enabled);
        assert!(!after_loss[&TargetId::Codex].enabled);
    }

    fn profile(target_id: TargetId, enabled: bool, dialect_id: &str) -> AgentProfile {
        AgentProfile {
            target_id,
            enabled,
            scopes: vec!["user".to_string()],
            dialect_id: dialect_id.to_string(),
        }
    }

    #[test]
    fn a_failed_probe_keeps_the_agent_as_it_was() {
        let configured = BTreeMap::from([(
            TargetId::Codex,
            profile(TargetId::Codex, true, &TargetId::Codex.current_dialect()),
        )]);
        let (kept, changed) = merge_detected_defaults(configured, &[], &[TargetId::Codex]);
        assert!(!changed);
        assert!(kept[&TargetId::Codex].enabled);
        let (lost, changed) = merge_detected_defaults(kept, &[], &[]);
        assert!(changed);
        assert!(!lost[&TargetId::Codex].enabled);
    }

    #[test]
    fn merging_refreshes_a_stale_dialect() {
        let configured = BTreeMap::from([(
            TargetId::ClaudeDesktop,
            profile(TargetId::ClaudeDesktop, true, "claude-desktop-2026-08"),
        )]);
        let (next, changed) = merge_detected_defaults(configured, &[TargetId::ClaudeDesktop], &[]);
        assert!(changed);
        assert_eq!(
            next[&TargetId::ClaudeDesktop].dialect_id,
            TargetId::ClaudeDesktop.current_dialect()
        );
    }

    #[test]
    fn damaged_profiles_fall_back_to_the_previous_copy_then_to_detection() {
        let root = tempfile::tempdir().expect("root");
        let paths = paths(root.path());
        set_enabled(&paths, TargetId::Codex, true).expect("first write");
        set_enabled(&paths, TargetId::Cursor, true).expect("second write");
        let current = paths.app_data().join(PROFILES_FILE);
        let previous = paths.app_data().join(PROFILES_BACKUP_FILE);
        let enabled = |paths: &SystemPaths| {
            read(paths)
                .into_iter()
                .filter(|profile| profile.enabled)
                .map(|profile| profile.target_id)
                .collect::<Vec<_>>()
        };

        fs::write(&current, b"{ not json").expect("corrupt current");
        assert_eq!(enabled(&paths), [TargetId::Codex]);

        fs::write(&previous, b"").expect("corrupt previous");
        assert!(enabled(&paths).is_empty());

        apply_detections(&paths, &BTreeMap::new());
        assert!(matches!(read_profiles_file(&current), Ok(Some(_))));
    }

    fn write_vscode_like_product_json(root: &Path, version: &str) {
        let path = root.join("Contents/Resources/app/product.json");
        fs::create_dir_all(path.parent().expect("parent")).expect("product dir");
        fs::write(&path, format!(r#"{{"version":"{version}"}}"#)).expect("product");
    }

    fn write_jetbrains_product_info(root: &Path, data_directory: &str) {
        let path = root.join("Contents/Resources/product-info.json");
        fs::create_dir_all(path.parent().expect("parent")).expect("product info dir");
        fs::write(
            &path,
            format!(r#"{{"dataDirectoryName":"{data_directory}","version":"2026.1"}}"#),
        )
        .expect("product info");
    }

    #[test]
    fn cursor_detection_requires_product_json() {
        let root = tempfile::tempdir().expect("root");
        let empty = root.path().join("Cursor.app");
        fs::create_dir_all(&empty).expect("empty app");
        assert!(detect_cursor_application_from(std::slice::from_ref(&empty)).is_none());
        write_vscode_like_product_json(&empty, "3.15.6");
        let detection =
            detect_cursor_application_from(std::slice::from_ref(&empty)).expect("detected");
        assert!(detection.detected);
        assert_eq!(detection.version.as_deref(), Some("3.15.6"));
    }

    #[test]
    fn vscode_copilot_ignores_leftover_extensions_without_the_editor() {
        let root = tempfile::tempdir().expect("root");
        let home = root.path().join("home");
        let extensions = home.join(".vscode/extensions/github.copilot-1.372.0");
        fs::create_dir_all(&extensions).expect("extension");
        fs::write(extensions.join("package.json"), r#"{"version":"1.372.0"}"#).expect("manifest");
        let missing_app = root.path().join("Visual Studio Code.app");
        assert!(detect_vscode_copilot_from(&[VscodeEdition {
            app_roots: vec![missing_app.clone()],
            extensions: home.join(".vscode/extensions"),
        }])
        .is_none());
        write_vscode_like_product_json(&missing_app, "1.128.0");
        let detection = detect_vscode_copilot_from(&[VscodeEdition {
            app_roots: vec![missing_app],
            extensions: home.join(".vscode/extensions"),
        }])
        .expect("detected");
        assert!(detection.detected);
        assert_eq!(detection.version.as_deref(), Some("1.372.0"));
    }

    #[test]
    fn vscode_copilot_built_in_to_a_versioned_windows_install_is_detected() {
        // The layout VS Code 1.1xx installs on Windows: <root>\<commit>\resources\app, Copilot Chat built in.
        let root = tempfile::tempdir().expect("root");
        let app = root
            .path()
            .join("Microsoft VS Code/04c0d99f4f/resources/app");
        fs::create_dir_all(app.join("extensions/copilot")).expect("built-in extension");
        fs::write(app.join("product.json"), r#"{"version":"1.130.0"}"#).expect("product");
        fs::write(
            app.join("extensions/copilot/package.json"),
            r#"{"name":"copilot-chat","publisher":"GitHub","version":"0.67.0"}"#,
        )
        .expect("manifest");
        let detection = detect_vscode_copilot_from(&[VscodeEdition {
            app_roots: vec![root.path().join("Microsoft VS Code")],
            extensions: root.path().join("home/.vscode/extensions"),
        }])
        .expect("detected");
        assert_eq!(detection.version.as_deref(), Some("0.67.0"));
    }

    #[test]
    fn jetbrains_copilot_ignores_plugins_from_older_ide_versions() {
        let root = tempfile::tempdir().expect("root");
        let app = root.path().join("CLion.app");
        write_jetbrains_product_info(&app, "CLion2026.1");
        let config = root.path().join("JetBrains");
        fs::create_dir_all(config.join("CLion2024.3/plugins/github-copilot-intellij"))
            .expect("leftover plugin");
        fs::create_dir_all(config.join("CLion2026.1/plugins/idea-vim")).expect("current plugins");
        assert!(detect_jetbrains_copilot_from(
            std::slice::from_ref(&app),
            std::slice::from_ref(&config)
        )
        .is_none());
        fs::create_dir_all(config.join("CLion2026.1/plugins/github-copilot-intellij"))
            .expect("current plugin");
        let detection = detect_jetbrains_copilot_from(
            std::slice::from_ref(&app),
            std::slice::from_ref(&config),
        )
        .expect("detected current plugin");
        assert!(detection.detected);
    }

    #[test]
    fn first_dir_with_prefix_matches_vscode_style_extension_folders() {
        let root = tempfile::tempdir().expect("root");
        fs::create_dir_all(root.path().join("github.copilot-1.372.0")).expect("extension");
        fs::create_dir_all(root.path().join("other.ext-1.0.0")).expect("other");
        assert_eq!(
            first_dir_with_prefix(root.path(), "github.copilot")
                .expect("found")
                .file_name()
                .and_then(|name| name.to_str()),
            Some("github.copilot-1.372.0")
        );
        assert_eq!(first_dir_with_prefix(root.path(), "missing"), None);
    }

    #[test]
    fn product_json_version_is_read_from_the_version_field() {
        let root = tempfile::tempdir().expect("root");
        let path = root.path().join("product.json");
        fs::write(&path, r#"{"nameShort":"Cursor","version":"3.15.6"}"#).expect("product");
        assert_eq!(
            read_json_string_field(&path, "version").as_deref(),
            Some("3.15.6")
        );
        fs::write(&path, r#"{"nameShort":"Cursor"}"#).expect("product");
        assert_eq!(read_json_string_field(&path, "version"), None);
    }

    #[test]
    fn version_output_uses_the_first_nonempty_line() {
        assert_eq!(
            first_nonempty_line(b"\n  3.15.6\na1f686\narm64\n").as_deref(),
            Some("3.15.6")
        );
        assert_eq!(first_nonempty_line(b"\n\n"), None);
    }

    #[test]
    fn target_ids_serialize_to_the_stable_wire_contract() {
        let serialized = TargetId::ALL
            .into_iter()
            .map(|target| serde_json::to_value(target).expect("serialize target"))
            .collect::<Vec<_>>();
        assert_eq!(
            serialized,
            [
                "github-copilot",
                "cursor",
                "claude-code",
                "claude-desktop",
                "opencode",
                "pi",
                "codex",
                "chatgpt",
                "grok-build"
            ]
            .into_iter()
            .map(serde_json::Value::from)
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn windows_missing_program_errors_are_not_surfaced_as_detection_failures() {
        assert!(is_missing_program_error(
            "codex detection: could not start the process: The system cannot find the file specified. (os error 2)"
        ));
        assert!(is_missing_program_error("No such file or directory"));
        assert!(!is_missing_program_error("timed out after 3 seconds"));
    }

    #[test]
    fn msix_detection_reads_the_version_from_the_package_repository() {
        let root = tempfile::tempdir().expect("root");
        let names = [
            "Other_1.0.0.0_x64__zzz".to_string(),
            "Claude_1.8555.2.0_x64__pzs8sxrjxfjjc".to_string(),
        ];
        let detection =
            detect_msix_from(&names, root.path(), &[CLAUDE_DESKTOP_MSIX]).expect("detected");
        assert!(detection.detected);
        assert_eq!(detection.version.as_deref(), Some("1.8555.2.0"));
        assert!(detect_msix_from(&names, root.path(), &["Nope_abc"]).is_none());
        assert!(detect_msix_from(&names, root.path(), &["Claude_otherpublisher"]).is_none());
    }

    #[test]
    fn msix_detection_accepts_the_packages_folder_without_a_version() {
        let root = tempfile::tempdir().expect("root");
        assert!(detect_msix_from(&[], root.path(), &CHATGPT_MSIX).is_none());
        fs::create_dir_all(root.path().join(CHATGPT_MSIX[1])).expect("packages dir");
        let detection = detect_msix_from(&[], root.path(), &CHATGPT_MSIX).expect("detected");
        assert!(detection.detected);
        assert_eq!(detection.version, None);
    }

    #[test]
    fn squirrel_claude_detection_needs_the_executable() {
        let root = tempfile::tempdir().expect("root");
        fs::create_dir_all(root.path().join("app-1.2.3")).expect("app dir");
        assert!(detect_squirrel_claude_from(root.path()).is_none());
        fs::write(root.path().join("claude.exe"), b"").expect("exe");
        let detection = detect_squirrel_claude_from(root.path()).expect("detected");
        assert_eq!(detection.version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn pi_counts_only_once_its_agent_folder_exists() {
        let root = tempfile::tempdir().expect("root");
        assert!(!detect_pi_from(None).detected);
        assert!(!detect_pi_from(Some(root.path())).detected);
        fs::create_dir_all(root.path().join(".pi/agent")).expect("pi folder");
        assert!(detect_pi_from(Some(root.path())).detected);
    }

    #[test]
    fn a_retired_app_in_the_profiles_file_is_dropped_not_fatal() {
        let root = tempfile::tempdir().expect("root");
        let path = root.path().join(PROFILES_FILE);
        fs::write(
            &path,
            r#"{"version":1,"profiles":[
                {"targetId":"m365-copilot","enabled":true,"scopes":["user"],"dialectId":"m365-copilot-2026-09"},
                {"targetId":"cursor","enabled":true,"scopes":["user"],"dialectId":"cursor-2026-08"}]}"#,
        )
        .expect("profiles");
        let profiles = read_profiles_file(&path).expect("read").expect("present");
        assert_eq!(
            profiles.keys().copied().collect::<Vec<_>>(),
            [TargetId::Cursor]
        );
    }

    #[test]
    fn desktop_targets_pin_the_september_dialect() {
        assert_eq!(TargetId::Chatgpt.current_dialect(), "chatgpt-2026-09");
        assert_eq!(
            TargetId::ClaudeDesktop.current_dialect(),
            "claude-desktop-2026-09"
        );
        assert_eq!(TargetId::Pi.current_dialect(), "pi-2026-09");
        assert_eq!(TargetId::Codex.current_dialect(), "codex-2026-08");
    }
}
