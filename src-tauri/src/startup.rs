//! Host environment preparation that runs once at process start.
//!
//! This is the only place that repairs the login environment so skill scripts
//! can find `uv`. Agents launch MCP servers themselves, so Node is not
//! discovered or installed here. GUI-launched apps inherit a PATH without `uv`
//! and miss proxy variables set in a shell profile. Corporate TLS interception
//! then breaks `uv` unless it uses the platform certificate store.
//!
//! Installed copies are preferred over a download. GUI apps do not inherit the
//! user's login PATH, so discovery also walks the user and machine Path and
//! Windows App Paths. `uv` is installed only when it is still missing.
//!
//! The app splits the work in two. [`prepare_process`] fixes this process's
//! environment during setup, before any command runs. [`finish_in_background`]
//! then publishes to the user session and downloads `uv` when it is missing,
//! so a slow or blocked download never holds up the window.
//!
//! Proxy values read from the system settings apply to this process only. The
//! user session gets only values the user supplied, because a value written
//! there comes back as "explicit" on the next launch and would pin a proxy the
//! user later changes.
//!
//! Add new startup host checks in [`prepare_process`] or [`finish_session`]. Do
//! not scatter PATH or proxy mutations through the rest of the crate.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

const TOOLS: [&str; 2] = ["uv", "uvx"];
const MAX_TOOL_ARCHIVE_BYTES: u64 = 80 * 1024 * 1024;
const TOOL_FETCH_TIMEOUT: Duration = Duration::from_secs(180);
const TOOL_FETCH_REDIRECTS: usize = 8;
const LOOPBACK_NO_PROXY: [&str; 3] = ["localhost", "127.0.0.1", "::1"];
const PROXY_PAIRS: [[&str; 2]; 4] = [
    ["HTTP_PROXY", "http_proxy"],
    ["HTTPS_PROXY", "https_proxy"],
    ["ALL_PROXY", "all_proxy"],
    ["NO_PROXY", "no_proxy"],
];
const UV_NATIVE_TLS: &str = "UV_NATIVE_TLS";
const SESSION_RECORD_FILE: &str = "session-environment.json";
/// Waits before each background `uv` install attempt at launch.
const TOOL_INSTALL_DELAYS: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_secs(30),
    Duration::from_secs(120),
];
/// Download attempts per session, counting the retries a preflight asks for.
const MAX_TOOL_INSTALL_ATTEMPTS: u32 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolPack {
    Uv,
}

impl ToolPack {
    fn id(self) -> &'static str {
        "uv"
    }

    fn tools(self) -> &'static [&'static str] {
        &["uv", "uvx"]
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ToolStatus {
    pub(crate) name: &'static str,
    pub(crate) path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum ProxyStatus {
    #[default]
    Unset,
    FromEnvironment {
        http: String,
        https: String,
    },
    FromSystem {
        http: String,
        https: String,
    },
    Socks {
        url: String,
    },
    PacOnly {
        url: String,
    },
}

impl ProxyStatus {
    /// One line for the preflight report.
    pub(crate) fn describe(&self) -> String {
        match self {
            ProxyStatus::Unset => "No proxy is configured.".to_string(),
            ProxyStatus::FromEnvironment { http, https } => format!(
                "Proxy from the environment: HTTP {}, HTTPS {}.",
                display_proxy(http),
                display_proxy(https)
            ),
            ProxyStatus::FromSystem { http, https } => format!(
                "Proxy from the system settings: HTTP {}, HTTPS {}.",
                display_proxy(http),
                display_proxy(https)
            ),
            ProxyStatus::Socks { url } => format!("SOCKS proxy {}.", display_proxy(url)),
            ProxyStatus::PacOnly { url } => format!(
                "Only a PAC script ({}) is configured; direct connections are used.",
                display_proxy(url)
            ),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct StartupReport {
    pub(crate) tools: Vec<ToolStatus>,
    pub(crate) proxy: ProxyStatus,
    pub(crate) notes: Vec<String>,
    /// Directories this run added to the user's PATH, not ones already there.
    pub(crate) published_path_dirs: Vec<PathBuf>,
    /// The app's own directory, once it is on the user's PATH.
    pub(crate) cli_dir_on_path: Option<PathBuf>,
    /// Proxy variables the user set before launch; only these are published.
    user_proxy_values: Vec<(String, OsString)>,
}

impl StartupReport {
    pub(crate) fn log(&self) {
        for tool in &self.tools {
            match &tool.path {
                Some(path) => eprintln!(
                    "Agent Plugins startup: found {} at {}.",
                    tool.name,
                    path.display()
                ),
                None => eprintln!(
                    "Agent Plugins startup: {} was not found. Skill scripts and MCP servers that invoke it will fail until it is installed.",
                    tool.name
                ),
            }
        }
        if !self.published_path_dirs.is_empty() {
            eprintln!(
                "Agent Plugins startup: added {} to the user PATH.",
                self.published_path_dirs
                    .iter()
                    .map(|dir| dir.display().to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        match &self.proxy {
            ProxyStatus::Unset => {
                eprintln!("Agent Plugins startup: no HTTP proxy is configured.");
            }
            ProxyStatus::FromEnvironment { http, https } => eprintln!(
                "Agent Plugins startup: using proxy from the environment (http {}, https {}).",
                display_proxy(http),
                display_proxy(https)
            ),
            ProxyStatus::FromSystem { http, https } => eprintln!(
                "Agent Plugins startup: using the system proxy (http {}, https {}).",
                display_proxy(http),
                display_proxy(https)
            ),
            ProxyStatus::Socks { url } => eprintln!(
                "Agent Plugins startup: using the system SOCKS proxy ({}).",
                display_proxy(url)
            ),
            ProxyStatus::PacOnly { url } => eprintln!(
                "Agent Plugins startup: a proxy auto-config URL is set ({url}); HTTP_PROXY was left unset because PAC files are not evaluated."
            ),
        }
        for note in &self.notes {
            eprintln!("Agent Plugins startup: {note}");
        }
    }
}

pub(crate) trait Host {
    fn extra_search_roots(&self) -> Vec<PathBuf>;
    fn additional_search_dirs(&self) -> Vec<PathBuf>;
    fn env(&self, key: &str) -> Option<OsString>;
    fn set_env(&mut self, key: &str, value: &OsStr);
    fn remove_env(&mut self, key: &str);
    fn is_executable(&self, path: &Path) -> bool;
    fn executable_extensions(&self) -> Vec<String>;
    fn path_separator(&self) -> char;
    fn env_keys_are_case_insensitive(&self) -> bool;
    fn system_proxy(&self) -> Option<SystemProxy>;
    /// What new processes in the user session see for `key`. `Ok(None)` means
    /// it is not set; `Err` means it could not be read. For PATH it is the
    /// user's own part only.
    fn session_env(&self, key: &str) -> Result<Option<OsString>, String>;
    fn session_path_defaults_to_gui(&self) -> bool;
    fn persist_enabled(&self) -> bool;
    fn persist_session(&mut self, key: &str, value: &OsStr) -> Result<(), String>;
    fn remove_session_env(&mut self, key: &str) -> Result<(), String>;
    /// Where the app records what it wrote to the user session.
    fn session_record_path(&self) -> Option<PathBuf>;
    /// The app's own directory, published so agents can run `agent-plugins`.
    fn cli_dir(&self) -> Option<PathBuf>;
    /// The app's executable, which `agent-plugins://` links open.
    fn app_exe(&self) -> Option<PathBuf>;
    /// The command Windows runs for an `agent-plugins://` link, if any.
    fn link_handler(&self) -> Result<Option<String>, String>;
    /// Points `agent-plugins://` links at `exe`, or forgets them for `None`.
    fn set_link_handler(&mut self, exe: Option<&Path>) -> Result<(), String>;
    fn managed_tools_root(&self) -> Option<PathBuf>;
    fn install_tool_pack(&mut self, pack: ToolPack) -> Result<PathBuf, String>;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SystemProxy {
    pub(crate) http: Option<String>,
    pub(crate) https: Option<String>,
    pub(crate) socks: Option<String>,
    pub(crate) no_proxy: Option<String>,
    pub(crate) pac_url: Option<String>,
}

impl SystemProxy {
    fn has_explicit_proxy(&self) -> bool {
        self.http.is_some() || self.https.is_some() || self.socks.is_some()
    }
}

struct LiveHost;

impl Host for LiveHost {
    fn extra_search_roots(&self) -> Vec<PathBuf> {
        live_search_roots()
    }

    fn additional_search_dirs(&self) -> Vec<PathBuf> {
        live_additional_search_dirs()
    }

    fn env(&self, key: &str) -> Option<OsString> {
        std::env::var_os(key)
    }

    fn set_env(&mut self, key: &str, value: &OsStr) {
        // SAFETY: the app calls `prepare_process` on the main thread during
        // setup, before the event loop dispatches any command or the scheduler
        // starts; the command line calls it before its runtime exists. The
        // background half never changes this process's environment.
        unsafe {
            std::env::set_var(key, value);
        }
    }

    fn remove_env(&mut self, key: &str) {
        // SAFETY: as for `set_env`.
        unsafe {
            std::env::remove_var(key);
        }
    }

    fn is_executable(&self, path: &Path) -> bool {
        is_executable_file(path)
    }

    fn executable_extensions(&self) -> Vec<String> {
        live_executable_extensions()
    }

    fn path_separator(&self) -> char {
        if cfg!(windows) {
            ';'
        } else {
            ':'
        }
    }

    fn env_keys_are_case_insensitive(&self) -> bool {
        cfg!(windows)
    }

    fn system_proxy(&self) -> Option<SystemProxy> {
        live_system_proxy()
    }

    fn session_env(&self, key: &str) -> Result<Option<OsString>, String> {
        live_session_env(key)
    }

    fn session_path_defaults_to_gui(&self) -> bool {
        cfg!(target_os = "macos")
    }

    fn persist_enabled(&self) -> bool {
        crate::qa_paths::root().ok().flatten().is_none()
    }

    fn persist_session(&mut self, key: &str, value: &OsStr) -> Result<(), String> {
        persist_session_env(key, value)
    }

    fn remove_session_env(&mut self, key: &str) -> Result<(), String> {
        remove_live_session_env(key)
    }

    fn session_record_path(&self) -> Option<PathBuf> {
        crate::paths::SystemPaths::from_system()
            .ok()
            .map(|paths| paths.app_data().join(SESSION_RECORD_FILE))
    }

    fn cli_dir(&self) -> Option<PathBuf> {
        // A development build lives in `target/`, which does not belong on
        // anyone's PATH.
        if cfg!(debug_assertions) {
            return None;
        }
        self.app_exe()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
    }

    fn app_exe(&self) -> Option<PathBuf> {
        // A development build in `target/` must not take over the links.
        if cfg!(debug_assertions) {
            return None;
        }
        std::env::current_exe().ok()
    }

    fn link_handler(&self) -> Result<Option<String>, String> {
        live_link_handler()
    }

    fn set_link_handler(&mut self, exe: Option<&Path>) -> Result<(), String> {
        live_set_link_handler(exe)
    }

    fn managed_tools_root(&self) -> Option<PathBuf> {
        live_managed_tools_root()
    }

    fn install_tool_pack(&mut self, pack: ToolPack) -> Result<PathBuf, String> {
        install_official_tool_pack(self, pack)
    }
}

/// Repairs PATH, proxy variables, and `uv` for the command line, installing
/// `uv` in the foreground when it is missing. Missing tools are reported and do
/// not stop it.
pub(crate) fn prepare() -> StartupReport {
    prepare_with(&mut LiveHost)
}

/// The launch half for the app: fixes this process's environment without
/// touching the network. [`finish_in_background`] does the rest.
pub(crate) fn prepare_process() -> StartupReport {
    prepare_process_with(&mut LiveHost)
}

/// Finds a command the way a freshly opened terminal would: this process's
/// PATH plus the user and machine Path as they are now, trying each PATHEXT
/// extension so `.cmd` shims such as npm's resolve.
/// The app's log file: what it reports on stderr, which a windowed app would
/// otherwise throw away.
pub(crate) fn log_path(paths: &crate::paths::SystemPaths) -> PathBuf {
    paths
        .local_data
        .join("agent-plugins")
        .join("agent-plugins.log")
}

/// Sends stderr to the log file when the window runs without a console. The
/// previous log is kept once as `.old` after it grows past 5 MB.
#[cfg(windows)]
pub(crate) fn log_to_file() {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{SetStdHandle, STD_ERROR_HANDLE};
    if cfg!(debug_assertions) {
        return;
    }
    let Ok(paths) = crate::paths::SystemPaths::from_system() else {
        return;
    };
    let path = log_path(&paths);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() > 5 * 1024 * 1024) {
        let _ = std::fs::rename(&path, path.with_extension("log.old"));
    }
    let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    // SAFETY: the handle is a file this process opened and never closes;
    // Rust's stderr looks the handle up again on every write.
    unsafe { SetStdHandle(STD_ERROR_HANDLE, file.into_raw_handle()) };
    eprintln!(
        "Agent Plugins {} started at {}",
        crate::marketplace::CLIENT_VERSION,
        crate::marketplace::rfc3339_now()
    );
}

#[cfg(not(windows))]
pub(crate) fn log_to_file() {}

/// Whether apps started from now on would see `name`: this process has it,
/// or the person's saved environment does.
pub(crate) fn environment_has(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|value| !value.is_empty())
        || live_session_env(name).ok().flatten().is_some()
}

/// Saves a connector setting, such as an API key, where the person's apps
/// read environment variables, and in this process so the window sees it at
/// once. The value is never logged or recorded anywhere else.
pub(crate) fn save_user_variable(name: &str, value: &str) -> Result<(), String> {
    let valid_name = !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
    if !valid_name || matches!(name, "PATH" | "HOME" | "USERPROFILE" | "APPDATA") {
        return Err(format!("{name} is not a setting Agent Plugins can save."));
    }
    if value.is_empty() || value.len() > 8192 || value.contains('\0') {
        return Err(format!("The value for {name} is empty or too long."));
    }
    if !cfg!(any(windows, target_os = "macos")) {
        return Err(format!(
            "Set {name} in your shell profile; Agent Plugins saves settings only on Windows and macOS."
        ));
    }
    persist_session_env(name, OsStr::new(value))?;
    std::env::set_var(name, value);
    Ok(())
}

pub(crate) fn find_program(name: &str) -> Option<PathBuf> {
    find_tool(&LiveHost, name)
}

pub(crate) fn prepare_with(host: &mut impl Host) -> StartupReport {
    let mut report = prepare_process_with(host);
    match install_missing_tools(host, &mut report) {
        Ok(()) => prepend_dirs(
            host,
            &tool_dirs(host, &report.tools),
            &current_path_dirs(host),
        ),
        Err(error) => report.notes.push(error),
    }
    finish_session(host, &mut report);
    report
}

fn prepare_process_with(host: &mut impl Host) -> StartupReport {
    let mut report = StartupReport::default();
    forget_legacy_system_proxy(host);
    report.user_proxy_values = user_proxy_values(host);
    ensure_proxy(host, &mut report);
    ensure_uv_trust(host, &mut report);
    scan_tools(host, &mut report);
    prepend_dirs(
        host,
        &tool_dirs(host, &report.tools),
        &current_path_dirs(host),
    );
    report
}

fn install_missing_tools(host: &mut impl Host, report: &mut StartupReport) -> Result<(), String> {
    let packs = missing_packs(&report.tools);
    if packs.is_empty() {
        return Ok(());
    }
    for pack in packs {
        if host.managed_tools_root().is_none() {
            return Err(format!(
                "Could not find a user-writable directory to install {}.",
                pack.id()
            ));
        }
        let dir = host
            .install_tool_pack(pack)
            .map_err(|error| format!("Could not install {}: {error}", pack.id()))?;
        report
            .notes
            .push(format!("Installed {} into {}.", pack.id(), dir.display()));
    }
    report.tools.clear();
    scan_tools(host, report);
    Ok(())
}

fn tool_dirs(host: &impl Host, tools: &[ToolStatus]) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for tool in tools {
        if let Some(parent) = tool.path.as_ref().and_then(|path| path.parent()) {
            push_unique_dir(host, &mut dirs, parent);
        }
    }
    dirs
}

/// The host preparation report. The background half updates it once `uv` is
/// installed.
pub(crate) struct SharedReport(Mutex<Option<StartupReport>>);

impl SharedReport {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(None))
    }

    pub(crate) fn get(&self) -> Option<StartupReport> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn set(&self, report: StartupReport) {
        *self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(report);
    }
}

/// Where the background `uv` install stands, for the preflight.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ToolInstallStatus {
    Idle,
    Running,
    /// Every attempt this session failed; carries the last error.
    GaveUp(Option<String>),
}

struct ToolInstall {
    running: bool,
    attempts: u32,
    last_error: Option<String>,
}

static TOOL_INSTALL: Mutex<ToolInstall> = Mutex::new(ToolInstall {
    running: false,
    attempts: 0,
    last_error: None,
});

fn tool_install() -> std::sync::MutexGuard<'static, ToolInstall> {
    TOOL_INSTALL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Publishes to the user session and installs a missing `uv` on a background
/// thread, retrying the download a few times this session.
pub(crate) fn finish_in_background() {
    start_background(&TOOL_INSTALL_DELAYS);
}

/// Tries the download once more when `uv` is still missing and the session
/// has attempts left, then reports where the install stands.
pub(crate) fn retry_tool_install() -> ToolInstallStatus {
    start_background(&TOOL_INSTALL_DELAYS[..1]);
    let state = tool_install();
    if state.running {
        ToolInstallStatus::Running
    } else if state.attempts >= MAX_TOOL_INSTALL_ATTEMPTS {
        ToolInstallStatus::GaveUp(state.last_error.clone())
    } else {
        ToolInstallStatus::Idle
    }
}

fn start_background(delays: &'static [Duration]) {
    if crate::STARTUP_REPORT.get().is_none() {
        return;
    }
    {
        let mut state = tool_install();
        if state.running {
            return;
        }
        state.running = true;
    }
    let spawned = std::thread::Builder::new()
        .name("agent-plugins-host".to_string())
        .spawn(move || {
            for (index, delay) in delays.iter().enumerate() {
                std::thread::sleep(*delay);
                let Some(mut report) = crate::STARTUP_REPORT.get() else {
                    break;
                };
                let missing = !missing_packs(&report.tools).is_empty();
                if index > 0 && !missing {
                    break;
                }
                if missing && take_install_attempt() {
                    if let Err(error) = install_missing_tools(&mut LiveHost, &mut report) {
                        eprintln!("Agent Plugins startup: {error}");
                        tool_install().last_error = Some(error);
                    }
                }
                // This process's PATH stays as it was: nothing here runs `uv`,
                // and changing the environment of a running app is unsafe.
                finish_session(&mut LiveHost, &mut report);
                let done = missing_packs(&report.tools).is_empty();
                crate::STARTUP_REPORT.set(report);
                if done {
                    break;
                }
            }
            tool_install().running = false;
        });
    if let Err(error) = spawned {
        eprintln!("Agent Plugins startup: could not start host preparation: {error}");
        tool_install().running = false;
    }
}

fn take_install_attempt() -> bool {
    let mut state = tool_install();
    if state.attempts >= MAX_TOOL_INSTALL_ATTEMPTS {
        return false;
    }
    state.attempts += 1;
    true
}

fn scan_tools(host: &impl Host, report: &mut StartupReport) {
    for name in TOOLS {
        report.tools.push(ToolStatus {
            name,
            path: find_tool(host, name),
        });
    }
    complete_tool_pairs(host, &mut report.tools);
}

fn missing_packs(tools: &[ToolStatus]) -> Vec<ToolPack> {
    let missing_primary = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .is_none_or(|tool| tool.path.is_none())
    };
    let mut packs = Vec::new();
    if missing_primary("uv") {
        packs.push(ToolPack::Uv);
    }
    packs
}

fn complete_tool_pairs(host: &impl Host, tools: &mut [ToolStatus]) {
    complete_companion(host, tools, "uv", "uvx");
}

fn complete_companion(host: &impl Host, tools: &mut [ToolStatus], primary: &str, companion: &str) {
    let Some(dir) = tools
        .iter()
        .find(|tool| tool.name == primary)
        .and_then(|tool| tool.path.as_ref())
        .and_then(|path| path.parent())
        .map(Path::to_path_buf)
    else {
        return;
    };
    let Some(status) = tools.iter_mut().find(|tool| tool.name == companion) else {
        return;
    };
    if status.path.is_some() {
        return;
    }
    status.path = find_tool_in_dir(host, &dir, companion);
}

fn find_tool(host: &impl Host, name: &str) -> Option<PathBuf> {
    candidate_search_dirs(host)
        .into_iter()
        .find_map(|dir| find_tool_in_dir(host, &dir, name))
}

fn candidate_search_dirs(host: &impl Host) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let login_dirs = host
        .session_env("PATH")
        .ok()
        .flatten()
        .map(|path| split_and_expand_paths(host, &path))
        .unwrap_or_default();
    for dir in current_path_dirs(host)
        .into_iter()
        .chain(login_dirs)
        .chain(host.additional_search_dirs())
        .chain(host.extra_search_roots())
    {
        if !dir.as_os_str().is_empty() {
            push_unique_dir(host, &mut dirs, &dir);
        }
    }
    dirs
}

fn find_tool_in_dir(host: &impl Host, dir: &Path, name: &str) -> Option<PathBuf> {
    tool_file_names(name, &host.executable_extensions())
        .into_iter()
        .map(|file_name| dir.join(file_name))
        .find(|candidate| host.is_executable(candidate))
}

fn ensure_proxy(host: &mut impl Host, report: &mut StartupReport) {
    mirror_proxy_cases(host);
    if let Some((http, https)) = explicit_http_proxies(host) {
        apply_proxy_var(host, "HTTP_PROXY", &http);
        apply_proxy_var(host, "HTTPS_PROXY", &https);
        report.proxy = ProxyStatus::FromEnvironment { http, https };
    } else if let Some(system) = host.system_proxy() {
        apply_system_proxy(host, system, report);
    } else {
        report.proxy = ProxyStatus::Unset;
    }
    ensure_loopback_no_proxy(host);
}

fn apply_system_proxy(host: &mut impl Host, system: SystemProxy, report: &mut StartupReport) {
    if let Some(no_proxy) = system.no_proxy.as_deref() {
        if env_utf8(host, "NO_PROXY").is_none() {
            apply_proxy_var(host, "NO_PROXY", no_proxy);
        }
    }
    let http = system.http.clone().or_else(|| system.https.clone());
    let https = system.https.clone().or_else(|| system.http.clone());
    let has_explicit_proxy = system.has_explicit_proxy();
    match (http, https, system.socks, system.pac_url) {
        (Some(http), Some(https), _, _) => {
            apply_proxy_var(host, "HTTP_PROXY", &http);
            apply_proxy_var(host, "HTTPS_PROXY", &https);
            report.proxy = ProxyStatus::FromSystem { http, https };
        }
        (_, _, Some(url), _) => {
            apply_proxy_var(host, "ALL_PROXY", &url);
            report.proxy = ProxyStatus::Socks { url };
        }
        (_, _, _, Some(url)) => {
            report.proxy = ProxyStatus::PacOnly { url };
        }
        _ => {
            report.proxy = ProxyStatus::Unset;
            if !has_explicit_proxy {
                report.notes.push(
                    "The system proxy settings did not include an explicit HTTP or SOCKS proxy."
                        .to_string(),
                );
            }
        }
    }
}

fn ensure_uv_trust(host: &mut impl Host, report: &mut StartupReport) {
    if env_utf8(host, UV_NATIVE_TLS).is_some() {
        return;
    }
    host.set_env(UV_NATIVE_TLS, OsStr::new("1"));
    report.notes.push(
        "Set UV_NATIVE_TLS=1 so uv uses the platform certificate store behind a corporate proxy."
            .to_string(),
    );
}

/// Publishes the tool directories, the app's own directory, `UV_NATIVE_TLS`,
/// and the proxy values the user supplied. Only values that change are
/// written, and an unreadable user Path is left alone.
fn finish_session(host: &mut impl Host, report: &mut StartupReport) {
    if !host.persist_enabled() {
        return;
    }
    let mut dirs = tool_dirs(host, &report.tools);
    let cli_dir = host.cli_dir();
    if let Some(dir) = &cli_dir {
        push_unique_dir(host, &mut dirs, dir);
    }
    match persist_session_path(host, &dirs) {
        Ok(added) => {
            for dir in added {
                push_unique_dir(host, &mut report.published_path_dirs, &dir);
            }
            report.cli_dir_on_path = cli_dir;
        }
        Err(error) => report.notes.push(format!(
            "Could not publish PATH to the user session: {error}"
        )),
    }
    if let Err(error) = register_link_handler(host) {
        report.notes.push(error);
    }
    let mut values = report.user_proxy_values.clone();
    if let Some(value) = host.env(UV_NATIVE_TLS) {
        values.push((UV_NATIVE_TLS.to_string(), value));
    }
    for (key, value) in values {
        if let Err(error) = persist_if_changed(host, &key, &value) {
            report.notes.push(format!(
                "Could not publish {key} to the user session: {error}"
            ));
        }
    }
}

fn persist_if_changed(host: &mut impl Host, key: &str, value: &OsStr) -> Result<(), String> {
    let text = value.to_string_lossy();
    let current = host.session_env(key)?;
    if current.is_some_and(|current| current.to_string_lossy() == text.trim()) {
        return Ok(());
    }
    host.persist_session(key, value)?;
    if let Some(path) = host.session_record_path() {
        let mut record = read_session_record(&path);
        record.written.insert(key.to_string(), text.into_owned());
        write_session_record(&path, &record);
    }
    Ok(())
}

/// Returns the directories it added. A user Path that could not be read is
/// an error, so a registry hiccup never replaces the user's Path with the
/// tool directories alone.
fn persist_session_path(host: &mut impl Host, dirs: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    if dirs.is_empty() {
        return Ok(Vec::new());
    }
    let sep = host.path_separator();
    let base = match host.session_env("PATH")? {
        Some(path) => path,
        None if host.session_path_defaults_to_gui() => OsString::from(default_gui_path(sep)),
        None => OsString::new(),
    };
    let existing = split_and_expand_paths(host, &base);
    let added = dirs
        .iter()
        .filter(|dir| !existing.iter().any(|entry| paths_match(host, entry, dir)))
        .cloned()
        .collect::<Vec<_>>();
    if added.is_empty() {
        return Ok(added);
    }
    let mut published = added.clone();
    published.extend(split_paths(&base, sep));
    host.persist_session("PATH", &join_paths(&published, sep))?;
    Ok(added)
}

/// Takes this copy's folder back off the user PATH, where the app put it so
/// shells find `agent-plugins`. The uninstaller runs it just before it deletes
/// the folder. Every other entry stays, unexpanded and in its place. True when
/// the folder was there.
pub(crate) fn remove_cli_dir_from_path() -> Result<bool, String> {
    remove_cli_dir_from_path_with(&mut LiveHost)
}

fn remove_cli_dir_from_path_with(host: &mut impl Host) -> Result<bool, String> {
    if !host.persist_enabled() {
        return Ok(false);
    }
    let removed = remove_cli_dir(host);
    unregister_link_handler(host)?;
    removed
}

fn remove_cli_dir(host: &mut impl Host) -> Result<bool, String> {
    let (Some(dir), Some(path)) = (host.cli_dir(), host.session_env("PATH")?) else {
        return Ok(false);
    };
    let sep = host.path_separator();
    let entries = split_paths(&path, sep);
    let kept = entries
        .iter()
        .filter(|entry| !paths_match(host, &expand_path_vars(host, (*entry).clone()), &dir))
        .cloned()
        .collect::<Vec<_>>();
    if kept.len() == entries.len() {
        return Ok(false);
    }
    host.persist_session("PATH", &join_paths(&kept, sep))?;
    Ok(true)
}

/// What Windows runs for an `agent-plugins://` link: this copy, with the link
/// as its one argument.
fn link_command(exe: &Path) -> String {
    format!("\"{}\" \"%1\"", exe.display())
}

/// Points `agent-plugins://` links at this copy, so the marketplace portal's
/// Install buttons open it. Writes only when the handler points elsewhere.
fn register_link_handler(host: &mut impl Host) -> Result<(), String> {
    let Some(exe) = host.app_exe() else {
        return Ok(());
    };
    if host.link_handler()? == Some(link_command(&exe)) {
        return Ok(());
    }
    host.set_link_handler(Some(&exe))
}

/// Forgets the link handler, but only one that still points at this copy.
fn unregister_link_handler(host: &mut impl Host) -> Result<(), String> {
    let Some(exe) = host.app_exe() else {
        return Ok(());
    };
    if host.link_handler()? == Some(link_command(&exe)) {
        host.set_link_handler(None)?;
    }
    Ok(())
}

#[cfg(windows)]
const LINK_KEY: &str = r"Software\Classes\agent-plugins";

#[cfg(windows)]
fn live_link_handler() -> Result<Option<String>, String> {
    let key = format!(r"{LINK_KEY}\shell\open\command");
    match winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER).open_subkey(key) {
        Ok(command) => Ok(command.get_value::<String, _>("").ok()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!(
            "Could not read the agent-plugins:// link handler: {error}"
        )),
    }
}

#[cfg(windows)]
fn live_set_link_handler(exe: Option<&Path>) -> Result<(), String> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let Some(exe) = exe else {
        return match hkcu.delete_subkey_all(LINK_KEY) {
            Err(error) if error.kind() != io::ErrorKind::NotFound => Err(format!(
                "Could not remove the agent-plugins:// link handler: {error}"
            )),
            _ => Ok(()),
        };
    };
    let write = || -> io::Result<()> {
        let (root, _) = hkcu.create_subkey(LINK_KEY)?;
        root.set_value("", &"URL:Agent Plugins".to_string())?;
        root.set_value("URL Protocol", &String::new())?;
        let (icon, _) = root.create_subkey("DefaultIcon")?;
        icon.set_value("", &format!("\"{}\",0", exe.display()))?;
        let (command, _) = root.create_subkey(r"shell\open\command")?;
        command.set_value("", &link_command(exe))
    };
    write().map_err(|error| format!("Could not register agent-plugins:// links: {error}"))
}

#[cfg(not(windows))]
fn live_link_handler() -> Result<Option<String>, String> {
    Ok(None)
}

#[cfg(not(windows))]
fn live_set_link_handler(_exe: Option<&Path>) -> Result<(), String> {
    Ok(())
}

fn user_proxy_values(host: &impl Host) -> Vec<(String, OsString)> {
    proxy_keys(host)
        .into_iter()
        .filter_map(|key| {
            let value = host.env(&key)?;
            (!value.to_string_lossy().trim().is_empty()).then_some((key, value))
        })
        .collect()
}

fn proxy_keys(host: &impl Host) -> Vec<String> {
    let mut keys = PROXY_PAIRS
        .iter()
        .map(|pair| pair[0].to_string())
        .collect::<Vec<_>>();
    if !host.env_keys_are_case_insensitive() {
        keys.extend(PROXY_PAIRS.iter().map(|pair| pair[1].to_string()));
    }
    keys
}

/// What the app keeps about its writes to the user session.
#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
#[serde(default, rename_all = "camelCase")]
struct SessionRecord {
    legacy_proxy_cleanup_done: bool,
    /// Every value the app wrote, so a later version can tell its own writes
    /// from the user's.
    written: BTreeMap<String, String>,
}

fn read_session_record(path: &Path) -> SessionRecord {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_session_record(path: &Path, record: &SessionRecord) {
    let written = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| fs::write(path, serde_json::to_vec_pretty(record).unwrap_or_default()));
    if let Err(error) = written {
        eprintln!(
            "Agent Plugins startup: could not record the user session changes in {}: {error}",
            path.display()
        );
    }
}

/// Builds before this one copied the system proxy into the user session. The
/// next launch read the copy back as the user's own setting and kept using it
/// after the system proxy changed. Once per machine, remove every session
/// value that still matches what those builds wrote from the current system
/// proxy. A copy of an older system proxy cannot be told from the user's own
/// setting and stays.
fn forget_legacy_system_proxy(host: &mut impl Host) {
    if !host.persist_enabled() {
        return;
    }
    let Some(record_path) = host.session_record_path() else {
        return;
    };
    let mut record = read_session_record(&record_path);
    if record.legacy_proxy_cleanup_done {
        return;
    }
    let system = host.system_proxy().unwrap_or_default();
    for (key, value) in legacy_session_proxy_values(host, &system) {
        let Ok(Some(current)) = host.session_env(&key) else {
            continue;
        };
        if current.to_string_lossy() != value.as_str() {
            continue;
        }
        if let Err(error) = host.remove_session_env(&key) {
            // Try again on the next launch.
            eprintln!("Agent Plugins startup: could not remove the copied {key}: {error}");
            return;
        }
        eprintln!(
            "Agent Plugins startup: removed {key} from the user session; an earlier version copied it from the system proxy."
        );
        if env_utf8(host, &key).as_deref() == Some(value.as_str()) {
            host.remove_env(&key);
        }
    }
    record.legacy_proxy_cleanup_done = true;
    write_session_record(&record_path, &record);
}

/// The proxy values earlier builds wrote to the user session from `system`.
fn legacy_session_proxy_values(host: &impl Host, system: &SystemProxy) -> Vec<(String, String)> {
    let mut values = Vec::new();
    let http = system.http.clone().or_else(|| system.https.clone());
    let https = system.https.clone().or_else(|| system.http.clone());
    match (http, https, &system.socks) {
        (Some(http), Some(https), _) => {
            values.push(("HTTP_PROXY".to_string(), http));
            values.push(("HTTPS_PROXY".to_string(), https));
        }
        (_, _, Some(socks)) => values.push(("ALL_PROXY".to_string(), socks.clone())),
        _ => {}
    }
    let (no_proxy, _) = with_loopback_hosts(system.no_proxy.as_deref().unwrap_or_default());
    values.push(("NO_PROXY".to_string(), no_proxy));
    if !host.env_keys_are_case_insensitive() {
        let lower = values
            .iter()
            .map(|(key, value)| (key.to_ascii_lowercase(), value.clone()))
            .collect::<Vec<_>>();
        values.extend(lower);
    }
    values
}

fn prepend_dirs(host: &mut impl Host, dirs: &[PathBuf], existing: &[PathBuf]) {
    if dirs.is_empty() {
        return;
    }
    let published = prepended_path(host, existing, dirs);
    if published != existing {
        host.set_env("PATH", &join_paths(&published, host.path_separator()));
    }
}

fn prepended_path(host: &impl Host, existing: &[PathBuf], dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut entries = existing.to_vec();
    for dir in dirs.iter().rev() {
        if !entries.iter().any(|entry| paths_match(host, entry, dir)) {
            entries.insert(0, dir.clone());
        }
    }
    entries
}

fn mirror_proxy_cases(host: &mut impl Host) {
    for pair in PROXY_PAIRS {
        let upper = env_utf8(host, pair[0]);
        let lower = env_utf8(host, pair[1]);
        match (upper, lower) {
            (Some(value), None) => apply_proxy_var(host, pair[0], &value),
            (None, Some(value)) => apply_proxy_var(host, pair[0], &value),
            (Some(_), Some(_)) | (None, None) => {}
        }
    }
}

fn explicit_http_proxies(host: &impl Host) -> Option<(String, String)> {
    let http = env_utf8(host, "HTTP_PROXY").or_else(|| env_utf8(host, "http_proxy"))?;
    let https = env_utf8(host, "HTTPS_PROXY")
        .or_else(|| env_utf8(host, "https_proxy"))
        .unwrap_or_else(|| http.clone());
    Some((http, https))
}

fn apply_proxy_var(host: &mut impl Host, canonical: &str, value: &str) {
    host.set_env(canonical, OsStr::new(value));
    if !host.env_keys_are_case_insensitive() {
        host.set_env(&canonical.to_ascii_lowercase(), OsStr::new(value));
    }
}

fn ensure_loopback_no_proxy(host: &mut impl Host) {
    let (value, changed) = with_loopback_hosts(&env_utf8(host, "NO_PROXY").unwrap_or_default());
    if changed {
        apply_proxy_var(host, "NO_PROXY", &value);
    }
}

fn with_loopback_hosts(existing: &str) -> (String, bool) {
    let mut entries = split_no_proxy(existing);
    let mut changed = existing.is_empty();
    for host_name in LOOPBACK_NO_PROXY {
        if !entries
            .iter()
            .any(|entry| entry.eq_ignore_ascii_case(host_name))
        {
            entries.push(host_name.to_string());
            changed = true;
        }
    }
    (entries.join(","), changed)
}

fn split_no_proxy(value: &str) -> Vec<String> {
    value
        .split([',', ';'])
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

fn current_path_dirs(host: &impl Host) -> Vec<PathBuf> {
    host.env("PATH")
        .map(|path| split_and_expand_paths(host, &path))
        .unwrap_or_default()
}

fn split_and_expand_paths(host: &impl Host, path: &OsStr) -> Vec<PathBuf> {
    split_paths(path, host.path_separator())
        .into_iter()
        .map(|entry| expand_path_vars(host, entry))
        .collect()
}

fn split_paths(path: &OsStr, sep: char) -> Vec<PathBuf> {
    path.to_string_lossy()
        .split(sep)
        .filter(|entry| !entry.is_empty())
        .map(PathBuf::from)
        .collect()
}

fn expand_path_vars(host: &impl Host, path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if !text.contains('%') {
        return path;
    }
    PathBuf::from(expand_percent_vars(host, &text))
}

fn expand_percent_vars(host: &impl Host, input: &str) -> String {
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let Some(end) = rest.find('%') else {
            out.push('%');
            out.push_str(rest);
            return out;
        };
        let name = &rest[..end];
        rest = &rest[end + 1..];
        if name.is_empty() {
            out.push('%');
            continue;
        }
        if let Some(value) = env_utf8(host, name) {
            out.push_str(&value);
        } else {
            out.push('%');
            out.push_str(name);
            out.push('%');
        }
    }
    out.push_str(rest);
    out
}

fn join_paths(entries: &[PathBuf], sep: char) -> OsString {
    let mut joined = OsString::new();
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            joined.push(sep.to_string());
        }
        joined.push(entry);
    }
    joined
}

fn push_unique_dir(host: &impl Host, dirs: &mut Vec<PathBuf>, dir: &Path) {
    if !dirs.iter().any(|existing| paths_match(host, existing, dir)) {
        dirs.push(dir.to_path_buf());
    }
}

fn paths_match(host: &impl Host, left: &Path, right: &Path) -> bool {
    if host.env_keys_are_case_insensitive() {
        left.as_os_str().eq_ignore_ascii_case(right.as_os_str())
    } else {
        left == right
    }
}

/// Windows runs a bare name only through one of its PATHEXT extensions, and npm
/// puts an extensionless shell script beside each `.cmd` shim, so the bare
/// name is a candidate only where there are no extensions.
fn tool_file_names(name: &str, extensions: &[String]) -> Vec<String> {
    if extensions.is_empty() {
        return vec![name.to_string()];
    }
    let mut names = Vec::new();
    for extension in extensions {
        let extension = extension.trim().trim_start_matches('.');
        if extension.is_empty() {
            continue;
        }
        names.push(format!("{name}.{extension}"));
    }
    names
}

fn env_utf8(host: &impl Host, key: &str) -> Option<String> {
    let value = host.env(key)?;
    let value = value.to_string_lossy();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn default_gui_path(sep: char) -> String {
    if sep == ';' {
        r"C:\Windows\system32;C:\Windows".to_string()
    } else {
        "/usr/bin:/bin:/usr/sbin:/sbin".to_string()
    }
}

fn display_proxy(value: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(value) else {
        return value.to_string();
    };
    if parsed.username().is_empty() && parsed.password().is_none() {
        return value.to_string();
    }
    let _ = parsed.set_username("***");
    let _ = parsed.set_password(None);
    parsed.to_string()
}

fn live_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(managed) = live_managed_tools_root() {
        roots.push(managed.join("uv"));
    }
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local").join("bin"));
        roots.push(home.join(".cargo").join("bin"));
        roots.push(home.join(".asdf").join("shims"));
        roots.push(home.join(".local").join("share").join("mise").join("shims"));
        roots.push(home.join("scoop").join("shims"));
    }
    #[cfg(unix)]
    {
        roots.push(PathBuf::from("/opt/homebrew/bin"));
        roots.push(PathBuf::from("/usr/local/bin"));
    }
    #[cfg(windows)]
    {
        roots.push(PathBuf::from(r"C:\ProgramData\chocolatey\bin"));
        if let Some(local) = dirs::data_local_dir() {
            roots.push(local.join("Microsoft").join("WinGet").join("Links"));
            push_python_script_dirs(&mut roots, &local.join("Programs").join("Python"));
        }
        if let Some(roaming) = dirs::data_dir() {
            push_python_script_dirs(&mut roots, &roaming.join("Python"));
        }
    }
    roots
}

fn live_additional_search_dirs() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let mut dirs = Vec::new();
        if let Some(path) = windows_env_value(
            winreg::enums::HKEY_LOCAL_MACHINE,
            MACHINE_ENVIRONMENT,
            "Path",
        )
        .ok()
        .flatten()
        {
            dirs.extend(
                split_paths(&path, ';')
                    .into_iter()
                    .map(|entry| expand_live_percent_vars(&entry.to_string_lossy())),
            );
        }
        for name in ["uv", "uvx"] {
            if let Some(exe) = windows_app_path(name) {
                if let Some(parent) = exe.parent() {
                    dirs.push(parent.to_path_buf());
                }
            }
        }
        dirs
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn expand_live_percent_vars(path: &str) -> PathBuf {
    if !path.contains('%') {
        return PathBuf::from(path);
    }
    PathBuf::from(expand_percent_vars(&LiveHost, path))
}

#[cfg(windows)]
const MACHINE_ENVIRONMENT: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";

/// A registry environment value: `Ok(None)` when the key or value is absent,
/// `Err` when it exists but could not be read.
#[cfg(windows)]
fn windows_env_value(
    hive: winreg::HKEY,
    subkey: &str,
    name: &str,
) -> Result<Option<OsString>, String> {
    let key = match winreg::RegKey::predef(hive).open_subkey(subkey) {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Could not open {subkey} in the registry: {error}")),
    };
    match key.get_value::<String, _>(name) {
        Ok(value) => Ok(Some(value.trim().to_string())
            .filter(|value| !value.is_empty())
            .map(OsString::from)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Could not read {name} from {subkey}: {error}")),
    }
}

#[cfg(windows)]
fn windows_app_path(name: &str) -> Option<PathBuf> {
    let file = format!("{name}.exe");
    let relative = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{file}");
    for hive in [
        winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER),
        winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE),
    ] {
        let Ok(key) = hive.open_subkey(&relative) else {
            continue;
        };
        let Ok(value) = key.get_value::<String, _>("") else {
            continue;
        };
        let path = PathBuf::from(value.trim().trim_matches('"'));
        if is_executable_file(&path) {
            return Some(path);
        }
    }
    None
}

#[cfg(windows)]
fn push_python_script_dirs(roots: &mut Vec<PathBuf>, python_root: &Path) {
    let Ok(entries) = fs::read_dir(python_root) else {
        return;
    };
    for entry in entries.flatten() {
        let scripts = entry.path().join("Scripts");
        if scripts.is_dir() {
            roots.push(scripts);
        }
    }
}

fn live_managed_tools_root() -> Option<PathBuf> {
    crate::paths::SystemPaths::from_system()
        .ok()
        .map(|paths| paths.local_data.join("agent-plugins").join("tools"))
}

fn live_executable_extensions() -> Vec<String> {
    #[cfg(windows)]
    {
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        // Only what `std::process::Command` can start; a `.js` or `.vbs`
        // needs a script host.
        pathext
            .split(';')
            .map(|part| part.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|part| matches!(part.as_str(), "com" | "exe" | "bat" | "cmd"))
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn live_system_proxy() -> Option<SystemProxy> {
    #[cfg(target_os = "macos")]
    {
        macos_system_proxy()
    }
    #[cfg(windows)]
    {
        windows_system_proxy()
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
fn macos_system_proxy() -> Option<SystemProxy> {
    let mut command = crate::process::command(Path::new("/usr/sbin/scutil"));
    command.arg("--proxy");
    let output =
        crate::process::run(command, "system proxy lookup", Duration::from_secs(3)).ok()?;
    if !output.status.success() {
        return None;
    }
    let parsed = parse_scutil_proxy(&String::from_utf8_lossy(&output.stdout));
    if parsed.has_explicit_proxy() || parsed.pac_url.is_some() {
        Some(parsed)
    } else {
        None
    }
}

#[cfg(windows)]
fn windows_system_proxy() -> Option<SystemProxy> {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let settings = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .ok()?;
    let pac_url = settings
        .get_value::<String, _>("AutoConfigURL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let enabled = settings.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 1;
    let server = settings
        .get_value::<String, _>("ProxyServer")
        .ok()
        .unwrap_or_default();
    let override_list = settings
        .get_value::<String, _>("ProxyOverride")
        .ok()
        .unwrap_or_default();
    let no_proxy_entries = parse_windows_proxy_override(&override_list);
    let mut proxy = SystemProxy {
        pac_url,
        no_proxy: (!no_proxy_entries.is_empty()).then(|| no_proxy_entries.join(",")),
        ..SystemProxy::default()
    };
    if enabled {
        let (http, https) = parse_windows_proxy_server(&server);
        proxy.http = http;
        proxy.https = https;
    }
    if proxy.has_explicit_proxy() || proxy.pac_url.is_some() {
        Some(proxy)
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn live_session_env(key: &str) -> Result<Option<OsString>, String> {
    let mut command = crate::process::command(Path::new("/bin/launchctl"));
    command.args(["getenv", key]);
    let output = crate::process::run(command, "session environment", Duration::from_secs(2))?;
    if !output.status.success() {
        return Ok(None);
    }
    let value = String::from_utf8_lossy(&output.stdout);
    let trimmed = value.trim();
    Ok((!trimmed.is_empty()).then(|| OsString::from(trimmed)))
}

/// New processes see the user's value, or the machine's when the user has
/// none. Path is the exception: Windows joins the two, and only the user's
/// part is ours to rewrite.
#[cfg(windows)]
fn live_session_env(key: &str) -> Result<Option<OsString>, String> {
    let user = windows_env_value(winreg::enums::HKEY_CURRENT_USER, "Environment", key)?;
    if user.is_some() || key.eq_ignore_ascii_case("PATH") {
        return Ok(user);
    }
    windows_env_value(winreg::enums::HKEY_LOCAL_MACHINE, MACHINE_ENVIRONMENT, key)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn live_session_env(_key: &str) -> Result<Option<OsString>, String> {
    Ok(None)
}

#[cfg(target_os = "macos")]
fn remove_live_session_env(key: &str) -> Result<(), String> {
    let mut command = crate::process::command(Path::new("/bin/launchctl"));
    command.args(["unsetenv", key]);
    let output = crate::process::run(command, "session environment", Duration::from_secs(2))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "launchctl unsetenv {key} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(windows)]
fn remove_live_session_env(key: &str) -> Result<(), String> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    let environment = match winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags("Environment", KEY_SET_VALUE)
    {
        Ok(key) => key,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("Could not open the user environment: {error}")),
    };
    match environment.delete_value(key) {
        Ok(()) => {
            broadcast_environment_change();
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not remove user environment {key}: {error}")),
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn remove_live_session_env(_key: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn persist_session_env(key: &str, value: &OsStr) -> Result<(), String> {
    let mut command = crate::process::command(Path::new("/bin/launchctl"));
    command.arg("setenv").arg(key).arg(value);
    let output = crate::process::run(command, "session environment", Duration::from_secs(2))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "launchctl setenv {key} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(windows)]
fn persist_session_env(key: &str, value: &OsStr) -> Result<(), String> {
    persist_windows_user_env(key, value)
}

#[cfg(not(any(target_os = "macos", windows)))]
fn persist_session_env(_key: &str, _value: &OsStr) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
fn persist_windows_user_env(key: &str, value: &OsStr) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_EXPAND_SZ};
    use winreg::RegValue;

    let hkcu = winreg::RegKey::predef(HKEY_CURRENT_USER);
    let environment = hkcu
        .open_subkey_with_flags("Environment", KEY_READ | KEY_SET_VALUE)
        .or_else(|_| hkcu.create_subkey("Environment").map(|(key, _)| key))
        .map_err(|error| format!("Could not open the user environment: {error}"))?;
    let name = if key.eq_ignore_ascii_case("PATH") {
        "Path"
    } else {
        key
    };
    if name == "Path" {
        let mut wide: Vec<u16> = value.encode_wide().collect();
        wide.push(0);
        let bytes = wide.iter().flat_map(|unit| unit.to_le_bytes()).collect();
        environment
            .set_raw_value(
                name,
                &RegValue {
                    bytes,
                    vtype: REG_EXPAND_SZ,
                },
            )
            .map_err(|error| format!("Could not write the user Path: {error}"))?;
    } else {
        let text = value.to_string_lossy();
        environment
            .set_value(name, &text.as_ref())
            .map_err(|error| format!("Could not write user environment {name}: {error}"))?;
    }
    broadcast_environment_change();
    Ok(())
}

#[cfg(windows)]
fn broadcast_environment_change() {
    const HWND_BROADCAST: isize = 0xffff;
    const WM_SETTINGCHANGE: u32 = 0x001A;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    let mut name: Vec<u16> = "Environment"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            name.as_mut_ptr() as isize,
            SMTO_ABORTIFHUNG,
            2_000,
            std::ptr::null_mut(),
        );
    }
}

#[cfg(windows)]
#[link(name = "user32")]
extern "system" {
    fn SendMessageTimeoutW(
        hwnd: isize,
        msg: u32,
        wparam: usize,
        lparam: isize,
        flags: u32,
        timeout_ms: u32,
        result: *mut usize,
    ) -> isize;
}

#[cfg(any(test, target_os = "macos"))]
pub(crate) fn parse_scutil_proxy(text: &str) -> SystemProxy {
    let mut values = BTreeMap::new();
    let mut exceptions = Vec::new();
    let mut in_exceptions = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if in_exceptions {
            if trimmed == "}" {
                in_exceptions = false;
                continue;
            }
            if let Some((_, value)) = split_scutil_field(trimmed) {
                if !value.is_empty() {
                    exceptions.push(value.to_string());
                }
            }
            continue;
        }
        let Some((key, value)) = split_scutil_field(trimmed) else {
            continue;
        };
        if key == "ExceptionsList" {
            in_exceptions = value.contains("<array>");
            continue;
        }
        values.insert(key.to_string(), value.to_string());
    }
    let http = enabled_proxy_url(&values, "HTTPEnable", "HTTPProxy", "HTTPPort");
    let https = enabled_proxy_url(&values, "HTTPSEnable", "HTTPSProxy", "HTTPSPort");
    let socks = if values.get("SOCKSEnable").is_some_and(|value| value == "1") {
        socks_proxy_url(values.get("SOCKSProxy"), values.get("SOCKSPort"))
    } else {
        None
    };
    let pac_url = values
        .get("ProxyAutoConfigEnable")
        .is_some_and(|value| value == "1")
        .then(|| values.get("ProxyAutoConfigURLString").cloned())
        .flatten()
        .filter(|value| !value.is_empty());
    SystemProxy {
        http,
        https,
        socks,
        no_proxy: (!exceptions.is_empty()).then(|| exceptions.join(",")),
        pac_url,
    }
}

#[cfg(any(test, target_os = "macos"))]
fn split_scutil_field(line: &str) -> Option<(&str, &str)> {
    line.split_once(" : ")
        .map(|(key, value)| (key.trim(), value.trim()))
}

#[cfg(any(test, target_os = "macos"))]
fn enabled_proxy_url(
    values: &BTreeMap<String, String>,
    enable_key: &str,
    host_key: &str,
    port_key: &str,
) -> Option<String> {
    if values.get(enable_key).map(String::as_str) != Some("1") {
        return None;
    }
    http_proxy_url(values.get(host_key), values.get(port_key))
}

#[cfg(any(test, target_os = "macos"))]
fn http_proxy_url(host: Option<&String>, port: Option<&String>) -> Option<String> {
    proxy_url("http", host.map(String::as_str), port.map(String::as_str))
}

#[cfg(any(test, target_os = "macos"))]
fn socks_proxy_url(host: Option<&String>, port: Option<&String>) -> Option<String> {
    proxy_url("socks5", host.map(String::as_str), port.map(String::as_str))
}

#[cfg(any(test, target_os = "macos"))]
fn proxy_url(scheme: &str, host: Option<&str>, port: Option<&str>) -> Option<String> {
    let host = host.map(str::trim).filter(|value| !value.is_empty())?;
    if host.contains("://") {
        return Some(host.to_string());
    }
    match port
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "0")
    {
        Some(port) => Some(format!("{scheme}://{host}:{port}")),
        None => Some(format!("{scheme}://{host}")),
    }
}

#[cfg(any(test, windows))]
fn parse_windows_proxy_server(server: &str) -> (Option<String>, Option<String>) {
    let server = server.trim();
    if server.is_empty() {
        return (None, None);
    }
    if !server.contains('=') {
        let url = normalize_proxy_url(server);
        return (url.clone(), url);
    }
    let mut http = None;
    let mut https = None;
    for part in server.split(';') {
        let Some((scheme, rest)) = part.split_once('=') else {
            continue;
        };
        let url = normalize_proxy_url(rest);
        match scheme.trim().to_ascii_lowercase().as_str() {
            "http" => http = url,
            "https" => https = url,
            _ => {}
        }
    }
    (http, https)
}

#[cfg(any(test, windows))]
fn parse_windows_proxy_override(override_list: &str) -> Vec<String> {
    let mut entries = Vec::new();
    for entry in override_list.split([';', ',']) {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if entry.eq_ignore_ascii_case("<local>") {
            entries.push("localhost".to_string());
            entries.push("127.0.0.1".to_string());
        } else {
            entries.push(entry.to_string());
        }
    }
    entries
}

#[cfg(any(test, windows))]
fn normalize_proxy_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.contains("://") {
        Some(value.to_string())
    } else {
        Some(format!("http://{value}"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ToolArchiveKind {
    Zip,
    TarGz,
}

struct ToolDownload {
    url: String,
    kind: ToolArchiveKind,
}

fn pack_download(pack: ToolPack) -> Result<ToolDownload, String> {
    pack_download_for(pack, std::env::consts::OS, std::env::consts::ARCH)
}

fn pack_download_for(pack: ToolPack, os: &str, arch: &str) -> Result<ToolDownload, String> {
    let url = match (pack, os, arch) {
        (ToolPack::Uv, "windows", "x86_64") => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-x86_64-pc-windows-msvc.zip"
        }
        (ToolPack::Uv, "windows", "aarch64") => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-aarch64-pc-windows-msvc.zip"
        }
        (ToolPack::Uv, "macos", "x86_64") => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-x86_64-apple-darwin.tar.gz"
        }
        (ToolPack::Uv, "macos", "aarch64") => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-aarch64-apple-darwin.tar.gz"
        }
        (ToolPack::Uv, "linux", "x86_64") => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-x86_64-unknown-linux-gnu.tar.gz"
        }
        (ToolPack::Uv, "linux", "aarch64") => {
            "https://github.com/astral-sh/uv/releases/latest/download/uv-aarch64-unknown-linux-gnu.tar.gz"
        }
        _ => {
            return Err(format!(
                "No user-level {} build is published for {os}/{arch}.",
                pack.id()
            ));
        }
    };
    let kind = if url.ends_with(".zip") {
        ToolArchiveKind::Zip
    } else {
        ToolArchiveKind::TarGz
    };
    Ok(ToolDownload {
        url: url.to_string(),
        kind,
    })
}

fn install_official_tool_pack(host: &impl Host, pack: ToolPack) -> Result<PathBuf, String> {
    let root = host
        .managed_tools_root()
        .ok_or_else(|| "Could not find a user-writable tools directory.".to_string())?;
    let dest = root.join(pack.id());
    if pack_is_present(host, &dest, pack) {
        return Ok(dest);
    }
    let download = pack_download(pack)?;
    eprintln!(
        "Agent Plugins startup: downloading {} from {}.",
        pack.id(),
        download.url
    );
    let bytes = download_https(&download.url)?;
    verify_published_checksum(&download.url, &bytes)?;
    fs::create_dir_all(&root)
        .map_err(|error| format!("Could not create {}: {error}", root.display()))?;
    let staging = crate::sources::temporary_path(&root, pack.id());
    if let Err(error) = extract_tool_archive(&bytes, download.kind, &staging) {
        let _ = crate::fs_retry::remove_dir_all(&staging);
        return Err(error);
    }
    make_extracted_files_executable(&staging);
    let bin_dir = find_pack_bin_dir(host, &staging, pack).ok_or_else(|| {
        let _ = crate::fs_retry::remove_dir_all(&staging);
        format!(
            "The {} archive did not contain {}.",
            pack.id(),
            pack.tools().join(" and ")
        )
    })?;
    if dest.exists() {
        crate::fs_retry::remove_dir_all(&dest)
            .map_err(|error| format!("Could not replace {}: {error}", dest.display()))?;
    }
    let rename_from = if bin_dir == staging {
        staging.clone()
    } else {
        bin_dir
    };
    crate::fs_retry::rename(&rename_from, &dest).map_err(|error| {
        format!(
            "Could not install {} to {}: {error}",
            pack.id(),
            dest.display()
        )
    })?;
    if staging.exists() {
        let _ = crate::fs_retry::remove_dir_all(&staging);
    }
    Ok(dest)
}

/// uv publishes `<archive>.sha256` beside every release archive. Both come
/// from GitHub, so this catches a truncated, corrupted, or swapped download on
/// the way, not a compromised release.
fn verify_published_checksum(url: &str, bytes: &[u8]) -> Result<(), String> {
    let checksum_url = format!("{url}.sha256");
    let expected = parse_sha256_file(&download_https(&checksum_url)?)
        .ok_or_else(|| format!("{checksum_url} is not a SHA-256 checksum file."))?;
    if crate::locator::sha256_hex(bytes) == expected {
        Ok(())
    } else {
        Err("The downloaded archive does not match its published SHA-256 checksum, so it was discarded.".to_string())
    }
}

/// Reads `<hex digest>  <file name>`, the format `sha256sum` writes.
fn parse_sha256_file(bytes: &[u8]) -> Option<String> {
    let digest = std::str::from_utf8(bytes)
        .ok()?
        .split_whitespace()
        .next()?
        .to_ascii_lowercase();
    (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(digest)
}

fn pack_is_present(host: &impl Host, dir: &Path, pack: ToolPack) -> bool {
    pack.tools()
        .iter()
        .all(|name| find_tool_in_dir(host, dir, name).is_some())
}

fn find_pack_bin_dir(host: &impl Host, root: &Path, pack: ToolPack) -> Option<PathBuf> {
    let mut current = vec![root.to_path_buf()];
    for _ in 0..3 {
        let mut next = Vec::new();
        for dir in current {
            if pack_is_present(host, &dir, pack) {
                return Some(dir);
            }
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
                        next.push(entry.path());
                    }
                }
            }
        }
        current = next;
    }
    None
}

fn make_extracted_files_executable(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if file_type.is_dir() {
                    stack.push(path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let Ok(metadata) = path.metadata() else {
                    continue;
                };
                let mut permissions = metadata.permissions();
                permissions.set_mode(permissions.mode() | 0o755);
                let _ = fs::set_permissions(&path, permissions);
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = root;
    }
}

fn download_https(url: &str) -> Result<Vec<u8>, String> {
    let parsed = url::Url::parse(url).map_err(|error| format!("Invalid download URL: {error}"))?;
    if parsed.scheme() != "https" {
        return Err("Tool downloads must use https://.".to_string());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("Tool download URLs may not contain credentials.".to_string());
    }
    let client = reqwest::blocking::Client::builder()
        .use_preconfigured_tls(crate::marketplace::tls()?)
        .timeout(TOOL_FETCH_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= TOOL_FETCH_REDIRECTS {
                return attempt.error("The download redirected too many times.");
            }
            if attempt.url().scheme() != "https" {
                return attempt.error("The download redirected to a non-HTTPS URL.");
            }
            attempt.follow()
        }))
        .build()
        .map_err(|error| format!("Could not create the download client: {error}"))?;
    let response = client
        .get(url)
        .send()
        .map_err(|error| crate::marketplace::describe_error(url, &error))?;
    if !response.status().is_success() {
        return Err(format!(
            "Could not download {url}: HTTP {}.",
            response.status()
        ));
    }
    if let Some(length) = response.content_length() {
        if length > MAX_TOOL_ARCHIVE_BYTES {
            return Err("The tool archive is larger than the 80 MB download limit.".to_string());
        }
    }
    let mut bytes = Vec::new();
    let mut reader = response;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("Could not download {url}: {error}"))?;
        if read == 0 {
            break;
        }
        let next = bytes.len().checked_add(read).ok_or_else(|| {
            "The tool archive is larger than the 80 MB download limit.".to_string()
        })?;
        if next as u64 > MAX_TOOL_ARCHIVE_BYTES {
            return Err("The tool archive is larger than the 80 MB download limit.".to_string());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    Ok(bytes)
}

fn extract_tool_archive(
    bytes: &[u8],
    kind: ToolArchiveKind,
    destination: &Path,
) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create {}: {error}", destination.display()))?;
    match kind {
        ToolArchiveKind::Zip => extract_tool_zip(bytes, destination),
        ToolArchiveKind::TarGz => extract_tool_tar(
            flate2::read::GzDecoder::new(Cursor::new(bytes)),
            destination,
        ),
    }
}

fn extract_tool_zip(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Could not read the tool zip: {error}"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Could not read a zip entry: {error}"))?;
        if entry.is_symlink() {
            return Err("Tool archives may not contain symbolic links.".to_string());
        }
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| "Tool archives may not contain unsafe paths.".to_string())?;
        let relative = sanitize_tool_archive_path(&enclosed)?;
        let path = destination.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&path)
                .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        let mut file = File::create(&path)
            .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
        io::copy(&mut entry, &mut file)
            .map_err(|error| format!("Could not extract {}: {error}", path.display()))?;
    }
    Ok(())
}

fn extract_tool_tar<R: Read>(reader: R, destination: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader);
    for entry in archive
        .entries()
        .map_err(|error| format!("Could not read the tool archive: {error}"))?
    {
        let mut entry = entry.map_err(|error| format!("Could not read a tar entry: {error}"))?;
        let header = entry.header();
        if header.entry_type().is_symlink() || header.entry_type().is_hard_link() {
            return Err("Tool archives may not contain symbolic links.".to_string());
        }
        let enclosed = entry
            .path()
            .map_err(|error| format!("Could not read a tar path: {error}"))?;
        let relative = sanitize_tool_archive_path(&enclosed)?;
        let path = destination.join(&relative);
        if header.entry_type().is_dir() {
            fs::create_dir_all(&path)
                .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
            continue;
        }
        if !header.entry_type().is_file() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        let mut file = File::create(&path)
            .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
        io::copy(&mut entry, &mut file)
            .map_err(|error| format!("Could not extract {}: {error}", path.display()))?;
    }
    Ok(())
}

fn sanitize_tool_archive_path(path: &Path) -> Result<PathBuf, String> {
    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => sanitized.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("Tool archives may not contain parent-directory paths.".to_string());
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err("Tool archives may not contain absolute paths.".to_string());
            }
        }
    }
    if sanitized.as_os_str().is_empty() {
        return Err("Tool archives may not contain empty paths.".to_string());
    }
    Ok(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    struct FakeHost {
        extra_roots: Vec<PathBuf>,
        additional_dirs: Vec<PathBuf>,
        vars: BTreeMap<String, OsString>,
        executables: BTreeSet<PathBuf>,
        executable_extensions: Vec<String>,
        path_separator: char,
        case_insensitive: bool,
        system_proxy: Option<SystemProxy>,
        session_unreadable: bool,
        session_path_defaults_to_gui: bool,
        persist_enabled: bool,
        persisted: BTreeMap<String, OsString>,
        persist_calls: Vec<String>,
        record_path: Option<PathBuf>,
        cli_dir: Option<PathBuf>,
        app_exe: Option<PathBuf>,
        link_handler: Option<String>,
        managed_root: Option<PathBuf>,
        install_error: Option<String>,
        installed: Vec<ToolPack>,
        https_proxy_at_install: Option<String>,
    }

    impl FakeHost {
        fn new() -> Self {
            Self {
                extra_roots: Vec::new(),
                additional_dirs: Vec::new(),
                vars: BTreeMap::new(),
                executables: BTreeSet::new(),
                executable_extensions: default_test_extensions(),
                path_separator: ':',
                case_insensitive: false,
                system_proxy: None,
                session_unreadable: false,
                session_path_defaults_to_gui: false,
                persist_enabled: true,
                persisted: BTreeMap::new(),
                persist_calls: Vec::new(),
                record_path: None,
                cli_dir: None,
                app_exe: None,
                link_handler: None,
                managed_root: None,
                install_error: None,
                installed: Vec::new(),
                https_proxy_at_install: None,
            }
        }

        fn with_path(mut self, path: &str) -> Self {
            self.vars.insert("PATH".to_string(), OsString::from(path));
            self
        }

        fn with_executable(mut self, path: &str) -> Self {
            self.executables.insert(PathBuf::from(path));
            self
        }

        fn with_root(mut self, path: &str) -> Self {
            self.extra_roots.push(PathBuf::from(path));
            self
        }

        fn with_additional_dir(mut self, path: &str) -> Self {
            self.additional_dirs.push(PathBuf::from(path));
            self
        }

        fn with_env(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.to_string(), OsString::from(value));
            self
        }

        fn with_system_proxy(mut self, proxy: SystemProxy) -> Self {
            self.system_proxy = Some(proxy);
            self
        }

        fn with_session(mut self, key: &str, value: &str) -> Self {
            self.persisted
                .insert(key.to_string(), OsString::from(value));
            self
        }

        fn persisted_str(&self, key: &str) -> Option<String> {
            self.persisted
                .get(key)
                .map(|value| value.to_string_lossy().into_owned())
        }

        fn env_str(&self, key: &str) -> Option<String> {
            self.env(key)
                .map(|value| value.to_string_lossy().into_owned())
        }

        fn with_managed_root(mut self, path: &str) -> Self {
            self.managed_root = Some(PathBuf::from(path));
            self
        }

        fn with_extensions(mut self, extensions: &[&str]) -> Self {
            self.executable_extensions = extensions.iter().map(|ext| (*ext).to_string()).collect();
            self
        }

        fn with_install_error(mut self, error: &str) -> Self {
            self.install_error = Some(error.to_string());
            self
        }
    }

    fn default_test_extensions() -> Vec<String> {
        if cfg!(windows) {
            vec!["exe".to_string()]
        } else {
            Vec::new()
        }
    }

    fn tool_file_name(name: &str) -> String {
        format!("{name}{}", std::env::consts::EXE_SUFFIX)
    }

    impl Host for FakeHost {
        fn extra_search_roots(&self) -> Vec<PathBuf> {
            self.extra_roots.clone()
        }

        fn additional_search_dirs(&self) -> Vec<PathBuf> {
            self.additional_dirs.clone()
        }

        fn env(&self, key: &str) -> Option<OsString> {
            if self.case_insensitive {
                let needle = key.to_ascii_lowercase();
                return self.vars.iter().find_map(|(existing, value)| {
                    existing
                        .eq_ignore_ascii_case(&needle)
                        .then(|| value.clone())
                });
            }
            self.vars.get(key).cloned()
        }

        fn set_env(&mut self, key: &str, value: &OsStr) {
            if self.case_insensitive {
                if let Some(existing) = self
                    .vars
                    .keys()
                    .find(|existing| existing.eq_ignore_ascii_case(key))
                    .cloned()
                {
                    self.vars.insert(existing, value.to_os_string());
                    return;
                }
            }
            self.vars.insert(key.to_string(), value.to_os_string());
        }

        fn remove_env(&mut self, key: &str) {
            self.vars.retain(|existing, _| {
                !(existing == key || (self.case_insensitive && existing.eq_ignore_ascii_case(key)))
            });
        }

        fn is_executable(&self, path: &Path) -> bool {
            self.executables.contains(path)
        }

        fn executable_extensions(&self) -> Vec<String> {
            self.executable_extensions.clone()
        }

        fn path_separator(&self) -> char {
            self.path_separator
        }

        fn env_keys_are_case_insensitive(&self) -> bool {
            self.case_insensitive
        }

        fn system_proxy(&self) -> Option<SystemProxy> {
            self.system_proxy.clone()
        }

        fn session_env(&self, key: &str) -> Result<Option<OsString>, String> {
            if self.session_unreadable {
                return Err("access denied".to_string());
            }
            Ok(self.persisted.get(key).cloned())
        }

        fn session_path_defaults_to_gui(&self) -> bool {
            self.session_path_defaults_to_gui
        }

        fn persist_enabled(&self) -> bool {
            self.persist_enabled
        }

        fn persist_session(&mut self, key: &str, value: &OsStr) -> Result<(), String> {
            self.persist_calls.push(key.to_string());
            self.persisted.insert(key.to_string(), value.to_os_string());
            Ok(())
        }

        fn remove_session_env(&mut self, key: &str) -> Result<(), String> {
            self.persisted.remove(key);
            Ok(())
        }

        fn session_record_path(&self) -> Option<PathBuf> {
            self.record_path.clone()
        }

        fn cli_dir(&self) -> Option<PathBuf> {
            self.cli_dir.clone()
        }

        fn app_exe(&self) -> Option<PathBuf> {
            self.app_exe.clone()
        }

        fn link_handler(&self) -> Result<Option<String>, String> {
            Ok(self.link_handler.clone())
        }

        fn set_link_handler(&mut self, exe: Option<&Path>) -> Result<(), String> {
            self.persist_calls.push("link".to_string());
            self.link_handler = exe.map(link_command);
            Ok(())
        }

        fn managed_tools_root(&self) -> Option<PathBuf> {
            self.managed_root.clone()
        }

        fn install_tool_pack(&mut self, pack: ToolPack) -> Result<PathBuf, String> {
            self.https_proxy_at_install = self.env_str("HTTPS_PROXY");
            if let Some(error) = &self.install_error {
                return Err(error.clone());
            }
            let root = self
                .managed_root
                .clone()
                .ok_or_else(|| "No user-writable tools directory is configured.".to_string())?;
            let dir = root.join(pack.id());
            for name in pack.tools() {
                for file_name in tool_file_names(name, &self.executable_extensions) {
                    self.executables.insert(dir.join(file_name));
                }
            }
            if !self.extra_roots.iter().any(|existing| existing == &dir) {
                self.extra_roots.insert(0, dir.clone());
            }
            self.installed.push(pack);
            Ok(dir)
        }
    }

    fn uv_path() -> PathBuf {
        PathBuf::from("/home/user/.local/bin").join(tool_file_name("uv"))
    }

    #[test]
    fn uninstalling_takes_only_this_folder_off_the_user_path() {
        let mut host = FakeHost::new();
        host.path_separator = ';';
        host.case_insensitive = true;
        host.cli_dir = Some(PathBuf::from(r"C:\Users\sam\AppData\Local\Agent Plugins"));
        host.persisted.insert(
            "PATH".to_string(),
            OsString::from(r"C:\Users\sam\AppData\Local\agent plugins;%USERPROFILE%\bin;C:\Tools"),
        );
        assert!(remove_cli_dir_from_path_with(&mut host).expect("removed"));
        assert_eq!(
            host.persisted["PATH"],
            OsString::from(r"%USERPROFILE%\bin;C:\Tools")
        );
        assert!(!remove_cli_dir_from_path_with(&mut host).expect("already gone"));
    }

    #[test]
    fn links_open_this_copy_until_it_is_uninstalled() {
        let exe = PathBuf::from(r"C:\Users\sam\AppData\Local\Agent Plugins\agent-plugins.exe");
        let command = format!("\"{}\" \"%1\"", exe.display());
        let mut host = FakeHost::new();
        host.app_exe = Some(exe);
        register_link_handler(&mut host).expect("register");
        assert_eq!(host.link_handler.as_deref(), Some(command.as_str()));
        register_link_handler(&mut host).expect("unchanged");
        assert_eq!(
            host.persist_calls,
            vec!["link"],
            "an unchanged handler is not rewritten"
        );

        remove_cli_dir_from_path_with(&mut host).expect("uninstall");
        assert_eq!(host.link_handler, None);

        // Another copy's handler is not ours to remove.
        host.link_handler = Some("\"D:\\Other\\agent-plugins.exe\" \"%1\"".to_string());
        remove_cli_dir_from_path_with(&mut host).expect("uninstall");
        assert!(host.link_handler.is_some());
    }

    #[test]
    fn finds_uv_outside_path_and_prepends_its_directory() {
        let mut host = FakeHost::new()
            .with_path("/usr/bin:/bin")
            .with_root("/home/user/.local/bin")
            .with_executable(uv_path().to_str().expect("utf8"));
        let report = prepare_with(&mut host);
        assert_eq!(
            report
                .tools
                .iter()
                .find(|tool| tool.name == "uv")
                .and_then(|tool| tool.path.as_ref()),
            Some(&uv_path())
        );
        assert_eq!(
            host.env_str("PATH").as_deref(),
            Some("/home/user/.local/bin:/usr/bin:/bin")
        );
    }

    #[test]
    fn does_not_duplicate_a_directory_already_on_path() {
        let mut host = FakeHost::new()
            .with_path("/home/user/.local/bin:/usr/bin")
            .with_root("/home/user/.local/bin")
            .with_executable(uv_path().to_str().expect("utf8"));
        prepare_with(&mut host);
        assert_eq!(
            host.env_str("PATH").as_deref(),
            Some("/home/user/.local/bin:/usr/bin")
        );
    }

    #[test]
    fn reports_missing_tools_without_changing_path() {
        let mut host = FakeHost::new().with_path("/usr/bin");
        let report = prepare_with(&mut host);
        assert!(report.tools.iter().all(|tool| tool.path.is_none()));
        assert_eq!(host.env_str("PATH").as_deref(), Some("/usr/bin"));
        assert!(report.published_path_dirs.is_empty());
    }

    #[test]
    fn copies_lowercase_proxy_to_uppercase_and_fills_https() {
        let mut host = FakeHost::new().with_env("http_proxy", "http://proxy.corp:8080");
        let report = prepare_with(&mut host);
        assert_eq!(
            host.env_str("HTTP_PROXY").as_deref(),
            Some("http://proxy.corp:8080")
        );
        assert_eq!(
            host.env_str("HTTPS_PROXY").as_deref(),
            Some("http://proxy.corp:8080")
        );
        assert_eq!(
            host.env_str("https_proxy").as_deref(),
            Some("http://proxy.corp:8080")
        );
        assert_eq!(
            report.proxy,
            ProxyStatus::FromEnvironment {
                http: "http://proxy.corp:8080".to_string(),
                https: "http://proxy.corp:8080".to_string(),
            }
        );
    }

    #[test]
    fn does_not_overwrite_an_existing_https_proxy() {
        let mut host = FakeHost::new()
            .with_env("HTTP_PROXY", "http://http-proxy:8080")
            .with_env("HTTPS_PROXY", "http://https-proxy:8443");
        prepare_with(&mut host);
        assert_eq!(
            host.env_str("HTTPS_PROXY").as_deref(),
            Some("http://https-proxy:8443")
        );
    }

    #[test]
    fn applies_system_proxy_when_environment_is_empty() {
        let mut host = FakeHost::new().with_system_proxy(SystemProxy {
            http: Some("http://proxy.corp:8080".to_string()),
            https: Some("http://proxy.corp:8443".to_string()),
            no_proxy: Some("*.corp".to_string()),
            ..SystemProxy::default()
        });
        let report = prepare_with(&mut host);
        assert_eq!(
            host.env_str("HTTPS_PROXY").as_deref(),
            Some("http://proxy.corp:8443")
        );
        assert!(host
            .env_str("NO_PROXY")
            .is_some_and(|value| value.contains("*.corp") && value.contains("localhost")));
        assert_eq!(
            report.proxy,
            ProxyStatus::FromSystem {
                http: "http://proxy.corp:8080".to_string(),
                https: "http://proxy.corp:8443".to_string(),
            }
        );
    }

    #[test]
    fn reports_pac_only_without_inventing_a_proxy() {
        let mut host = FakeHost::new().with_system_proxy(SystemProxy {
            pac_url: Some("http://pac.corp/proxy.pac".to_string()),
            ..SystemProxy::default()
        });
        let report = prepare_with(&mut host);
        assert!(host.env_str("HTTP_PROXY").is_none());
        assert_eq!(
            report.proxy,
            ProxyStatus::PacOnly {
                url: "http://pac.corp/proxy.pac".to_string(),
            }
        );
    }

    #[test]
    fn socks_only_system_proxy_becomes_all_proxy() {
        let mut host = FakeHost::new().with_system_proxy(SystemProxy {
            socks: Some("socks5://proxy.corp:1080".to_string()),
            ..SystemProxy::default()
        });
        let report = prepare_with(&mut host);
        assert_eq!(
            host.env_str("ALL_PROXY").as_deref(),
            Some("socks5://proxy.corp:1080")
        );
        assert_eq!(
            report.proxy,
            ProxyStatus::Socks {
                url: "socks5://proxy.corp:1080".to_string(),
            }
        );
    }

    #[test]
    fn sets_uv_native_tls_when_absent_and_preserves_an_existing_value() {
        let mut host = FakeHost::new();
        prepare_with(&mut host);
        assert_eq!(host.env_str(UV_NATIVE_TLS).as_deref(), Some("1"));

        let mut host = FakeHost::new().with_env(UV_NATIVE_TLS, "false");
        prepare_with(&mut host);
        assert_eq!(host.env_str(UV_NATIVE_TLS).as_deref(), Some("false"));
    }

    #[test]
    fn adds_loopback_hosts_to_no_proxy_without_duplicating() {
        let mut host = FakeHost::new().with_env("NO_PROXY", "localhost,*.corp");
        prepare_with(&mut host);
        let value = host.env_str("NO_PROXY").expect("NO_PROXY");
        assert!(value.contains("localhost"));
        assert!(value.contains("127.0.0.1"));
        assert!(value.contains("::1"));
        assert_eq!(value.matches("localhost").count(), 1);
    }

    #[test]
    fn publishes_tool_dirs_to_the_session_path_without_using_process_path() {
        let mut host = FakeHost::new()
            .with_path("/opt/conda/bin:/usr/bin")
            .with_root("/home/user/.local/bin")
            .with_executable(uv_path().to_str().expect("utf8"));
        host.persisted
            .insert("PATH".to_string(), OsString::from("/usr/bin:/bin"));
        prepare_with(&mut host);
        assert_eq!(
            host.persisted
                .get("PATH")
                .map(|value| value.to_string_lossy().into_owned())
                .as_deref(),
            Some("/home/user/.local/bin:/usr/bin:/bin")
        );
        assert_eq!(
            host.persisted
                .get(UV_NATIVE_TLS)
                .map(|value| value.to_string_lossy().into_owned())
                .as_deref(),
            Some("1")
        );
    }

    #[test]
    fn case_insensitive_hosts_only_write_uppercase_proxy_keys() {
        let mut host = FakeHost::new()
            .with_env("http_proxy", "http://proxy.corp:8080")
            .with_system_proxy(SystemProxy::default());
        host.case_insensitive = true;
        prepare_with(&mut host);
        assert!(host.vars.keys().all(|key| {
            !key.chars().any(char::is_lowercase) || key == "http_proxy" || key == "PATH"
        }));
        assert_eq!(
            host.env_str("HTTP_PROXY").as_deref(),
            Some("http://proxy.corp:8080")
        );
    }

    #[test]
    fn second_prepare_is_idempotent() {
        let mut host = FakeHost::new()
            .with_path("/usr/bin")
            .with_root("/home/user/.local/bin")
            .with_executable(uv_path().to_str().expect("utf8"))
            .with_env("NO_PROXY", "localhost");
        prepare_with(&mut host);
        let first_path = host.env_str("PATH");
        let first_no_proxy = host.env_str("NO_PROXY");
        prepare_with(&mut host);
        assert_eq!(host.env_str("PATH"), first_path);
        assert_eq!(host.env_str("NO_PROXY"), first_no_proxy);
    }

    #[test]
    fn parse_scutil_proxy_reads_explicit_proxy_exceptions_and_pac() {
        let parsed = parse_scutil_proxy(
            r#"
<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
  }
  HTTPEnable : 1
  HTTPPort : 8080
  HTTPProxy : proxy.corp.example
  HTTPSEnable : 1
  HTTPSPort : 8443
  HTTPSProxy : proxy.corp.example
  ProxyAutoConfigEnable : 0
  SOCKSEnable : 0
}
"#,
        );
        assert_eq!(
            parsed.http.as_deref(),
            Some("http://proxy.corp.example:8080")
        );
        assert_eq!(
            parsed.https.as_deref(),
            Some("http://proxy.corp.example:8443")
        );
        assert_eq!(parsed.no_proxy.as_deref(), Some("*.local,169.254/16"));
        assert!(parsed.pac_url.is_none());

        let pac = parse_scutil_proxy(
            r#"
  HTTPEnable : 0
  ProxyAutoConfigEnable : 1
  ProxyAutoConfigURLString : http://pac.corp/proxy.pac
"#,
        );
        assert_eq!(pac.pac_url.as_deref(), Some("http://pac.corp/proxy.pac"));
        assert!(pac.http.is_none());
    }

    #[test]
    fn parse_windows_proxy_server_handles_shared_and_per_scheme_hosts() {
        assert_eq!(
            parse_windows_proxy_server("proxy.corp:8080"),
            (
                Some("http://proxy.corp:8080".to_string()),
                Some("http://proxy.corp:8080".to_string())
            )
        );
        assert_eq!(
            parse_windows_proxy_server("http=http-proxy:8080;https=https-proxy:8443"),
            (
                Some("http://http-proxy:8080".to_string()),
                Some("http://https-proxy:8443".to_string())
            )
        );
        assert_eq!(
            parse_windows_proxy_override("localhost;<local>;*.corp"),
            ["localhost", "localhost", "127.0.0.1", "*.corp"]
        );
    }

    #[test]
    fn display_proxy_redacts_userinfo() {
        assert_eq!(
            display_proxy("http://user:secret@proxy.corp:8080"),
            "http://***@proxy.corp:8080/"
        );
    }

    #[test]
    fn installs_missing_uv_into_the_user_tools_directory() {
        let uv_dir = PathBuf::from("/home/user/.local/share/agent-plugins/tools").join("uv");
        let mut host = FakeHost::new()
            .with_path("/usr/bin")
            .with_managed_root(uv_dir.parent().expect("tools root").to_str().expect("utf8"));
        let report = prepare_with(&mut host);
        assert_eq!(host.installed, [ToolPack::Uv]);
        assert!(report.tools.iter().all(|tool| tool.path.is_some()));
        assert!(host.env_str("PATH").is_some_and(|path| {
            split_paths(OsStr::new(&path), host.path_separator).contains(&uv_dir)
        }));
        assert_eq!(report.published_path_dirs, [uv_dir]);
    }

    #[test]
    fn does_not_install_tools_that_are_already_on_path() {
        let mut host = FakeHost::new()
            .with_path("/usr/bin")
            .with_root("/home/user/.local/bin")
            .with_executable(uv_path().to_str().expect("utf8"))
            .with_executable(
                PathBuf::from("/home/user/.local/bin")
                    .join(tool_file_name("uvx"))
                    .to_str()
                    .expect("utf8"),
            )
            .with_managed_root("/tmp/tools");
        prepare_with(&mut host);
        assert!(host.installed.is_empty());
    }

    #[test]
    fn failed_user_level_install_is_reported_and_does_not_stop_startup() {
        let mut host = FakeHost::new()
            .with_path("/usr/bin")
            .with_managed_root("/tmp/tools")
            .with_install_error("download blocked");
        let report = prepare_with(&mut host);
        assert!(report.tools.iter().all(|tool| tool.path.is_none()));
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("uv") && note.contains("download blocked")));
        assert_eq!(host.env_str("PATH").as_deref(), Some("/usr/bin"));
    }

    #[test]
    fn applies_proxy_before_installing_tools() {
        let mut host = FakeHost::new()
            .with_path("/usr/bin")
            .with_managed_root("/tmp/tools")
            .with_env("https_proxy", "http://proxy.corp:8080");
        prepare_with(&mut host);
        assert_eq!(
            host.https_proxy_at_install.as_deref(),
            Some("http://proxy.corp:8080")
        );
    }

    #[test]
    fn windows_session_path_persist_keeps_the_user_path_only() {
        let uv = PathBuf::from(r"C:\Users\me\.local\bin").join("uv.exe");
        let mut host = FakeHost::new()
            .with_extensions(&["exe"])
            .with_path(r"C:\Windows\system32;C:\Windows;C:\Users\me\.local\bin")
            .with_root(r"C:\Users\me\.local\bin")
            .with_executable(uv.to_str().expect("utf8"));
        host.path_separator = ';';
        host.case_insensitive = true;
        host.persisted
            .insert("PATH".to_string(), OsString::from(r"%USERPROFILE%\bin"));
        prepare_with(&mut host);
        assert_eq!(
            host.persisted
                .get("PATH")
                .map(|value| value.to_string_lossy().into_owned())
                .as_deref(),
            Some(r"C:\Users\me\.local\bin;%USERPROFILE%\bin")
        );
    }

    #[test]
    fn empty_windows_user_path_persists_only_tool_directories() {
        let uv = PathBuf::from(r"C:\Users\me\.local\bin").join("uv.exe");
        let mut host = FakeHost::new()
            .with_extensions(&["exe"])
            .with_path(r"C:\Windows\system32;C:\Users\me\.local\bin")
            .with_root(r"C:\Users\me\.local\bin")
            .with_executable(uv.to_str().expect("utf8"));
        host.path_separator = ';';
        host.session_path_defaults_to_gui = false;
        prepare_with(&mut host);
        assert_eq!(
            host.persisted
                .get("PATH")
                .map(|value| value.to_string_lossy().into_owned())
                .as_deref(),
            Some(r"C:\Users\me\.local\bin")
        );
    }

    #[test]
    fn windows_tool_download_urls_are_user_level_zips() {
        let uv = pack_download_for(ToolPack::Uv, "windows", "x86_64").expect("uv");
        assert!(uv.url.contains("uv-x86_64-pc-windows-msvc.zip"));
        assert_eq!(uv.kind, ToolArchiveKind::Zip);
    }

    #[test]
    fn finds_uv_on_the_login_path_without_installing() {
        let uv = PathBuf::from("/users/me/.local/bin").join(tool_file_name("uv"));
        let mut host = FakeHost::new()
            .with_path("/windows/system32")
            .with_executable(uv.to_str().expect("utf8"));
        host.persisted
            .insert("PATH".to_string(), OsString::from("/users/me/.local/bin"));
        let report = prepare_with(&mut host);
        assert_eq!(
            report
                .tools
                .iter()
                .find(|tool| tool.name == "uv")
                .and_then(|tool| tool.path.as_ref()),
            Some(&uv)
        );
        assert!(host.installed.is_empty());
    }

    #[test]
    fn expands_percent_variables_in_the_login_path() {
        let uv = PathBuf::from("/users/me/bin").join(tool_file_name("uv"));
        let mut host = FakeHost::new()
            .with_path("/windows/system32")
            .with_env("USERPROFILE", "/users/me")
            .with_executable(uv.to_str().expect("utf8"));
        host.persisted
            .insert("PATH".to_string(), OsString::from("%USERPROFILE%/bin"));
        let report = prepare_with(&mut host);
        assert_eq!(
            report
                .tools
                .iter()
                .find(|tool| tool.name == "uv")
                .and_then(|tool| tool.path.as_ref()),
            Some(&uv)
        );
    }

    #[test]
    fn finds_uv_from_an_additional_search_dir() {
        let uv_dir = PathBuf::from("/users/me/.local/bin");
        let uv = uv_dir.join(tool_file_name("uv"));
        let mut host = FakeHost::new()
            .with_path("/windows/system32")
            .with_additional_dir(uv_dir.to_str().expect("utf8"))
            .with_executable(uv.to_str().expect("utf8"));
        let report = prepare_with(&mut host);
        assert_eq!(
            report
                .tools
                .iter()
                .find(|tool| tool.name == "uv")
                .and_then(|tool| tool.path.as_ref()),
            Some(&uv)
        );
        assert!(host.installed.is_empty());
    }

    #[test]
    fn does_not_download_uv_when_only_uvx_is_missing() {
        let mut host = FakeHost::new()
            .with_path("/usr/bin")
            .with_root("/home/user/.local/bin")
            .with_executable(uv_path().to_str().expect("utf8"))
            .with_managed_root("/tmp/tools");
        prepare_with(&mut host);
        assert!(!host.installed.contains(&ToolPack::Uv));
    }

    #[test]
    fn expand_percent_vars_replaces_known_names() {
        let host = FakeHost::new().with_env("LOCALAPPDATA", r"C:\Users\me\AppData\Local");
        assert_eq!(
            expand_percent_vars(&host, r"%LOCALAPPDATA%\Microsoft\WinGet\Links"),
            r"C:\Users\me\AppData\Local\Microsoft\WinGet\Links"
        );
        assert_eq!(
            expand_percent_vars(&host, r"%MISSING%\bin"),
            r"%MISSING%\bin"
        );
    }

    #[test]
    fn finds_uv_binaries_in_the_official_windows_zip_layout() {
        let root = tempfile::tempdir().expect("temp");
        let nested = root.path().join("uv-x86_64-pc-windows-msvc");
        fs::create_dir_all(&nested).expect("uv dir");
        for name in ["uv.exe", "uvx.exe"] {
            fs::write(nested.join(name), []).expect("tool file");
        }
        let mut host = FakeHost::new().with_extensions(&["exe"]);
        for name in ["uv.exe", "uvx.exe"] {
            host.executables.insert(nested.join(name));
        }
        assert_eq!(
            find_pack_bin_dir(&host, root.path(), ToolPack::Uv).as_deref(),
            Some(nested.as_path())
        );
    }

    fn corp_system_proxy() -> SystemProxy {
        SystemProxy {
            http: Some("http://proxy.corp:8080".to_string()),
            https: Some("http://proxy.corp:8080".to_string()),
            ..SystemProxy::default()
        }
    }

    #[test]
    fn system_proxy_applies_to_the_process_only() {
        let mut host = FakeHost::new().with_system_proxy(corp_system_proxy());
        prepare_with(&mut host);
        assert_eq!(
            host.env_str("HTTPS_PROXY").as_deref(),
            Some("http://proxy.corp:8080")
        );
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "NO_PROXY",
            "http_proxy",
            "no_proxy",
        ] {
            assert!(host.persisted_str(key).is_none(), "{key} was published");
        }
    }

    #[test]
    fn publishes_proxy_values_exactly_as_the_user_supplied_them() {
        let mut host = FakeHost::new()
            .with_env("http_proxy", "http://proxy.corp:8080")
            .with_env("NO_PROXY", "*.corp");
        prepare_with(&mut host);
        assert_eq!(
            host.persisted_str("http_proxy").as_deref(),
            Some("http://proxy.corp:8080")
        );
        assert_eq!(host.persisted_str("NO_PROXY").as_deref(), Some("*.corp"));
        assert!(host.persisted_str("HTTPS_PROXY").is_none());
    }

    #[test]
    fn removes_a_system_proxy_copy_left_by_an_earlier_version_once() {
        let temp = tempfile::tempdir().expect("temp");
        let record = temp.path().join(SESSION_RECORD_FILE);
        let loopback = "localhost,127.0.0.1,::1";
        let legacy = |host: FakeHost| {
            let mut host = host
                .with_system_proxy(corp_system_proxy())
                .with_session("HTTP_PROXY", "http://proxy.corp:8080")
                .with_session("HTTPS_PROXY", "http://proxy.corp:8080")
                .with_session("NO_PROXY", loopback)
                .with_env("HTTP_PROXY", "http://proxy.corp:8080")
                .with_env("HTTPS_PROXY", "http://proxy.corp:8080")
                .with_env("NO_PROXY", loopback);
            host.case_insensitive = true;
            host.record_path = Some(record.clone());
            host
        };
        let mut host = legacy(FakeHost::new().with_session("ALL_PROXY", "socks5://mine:1080"));
        let report = prepare_with(&mut host);
        assert_eq!(
            report.proxy,
            ProxyStatus::FromSystem {
                http: "http://proxy.corp:8080".to_string(),
                https: "http://proxy.corp:8080".to_string(),
            }
        );
        assert!(host.persisted_str("HTTP_PROXY").is_none());
        assert!(host.persisted_str("HTTPS_PROXY").is_none());
        assert!(host.persisted_str("NO_PROXY").is_none());
        assert_eq!(
            host.persisted_str("ALL_PROXY").as_deref(),
            Some("socks5://mine:1080")
        );
        assert!(read_session_record(&record).legacy_proxy_cleanup_done);

        // The same value set again later is the user's and stays.
        let mut host = legacy(FakeHost::new());
        prepare_with(&mut host);
        assert_eq!(
            host.persisted_str("HTTP_PROXY").as_deref(),
            Some("http://proxy.corp:8080")
        );
    }

    #[test]
    fn an_unreadable_user_path_is_not_overwritten() {
        let mut host = FakeHost::new()
            .with_path("/usr/bin")
            .with_root("/home/user/.local/bin")
            .with_executable(uv_path().to_str().expect("utf8"));
        host.session_unreadable = true;
        let report = prepare_with(&mut host);
        assert!(host.persist_calls.is_empty());
        assert!(report
            .notes
            .iter()
            .any(|note| note.contains("Could not publish PATH")));
    }

    #[test]
    fn writes_only_values_that_change_and_counts_only_added_dirs() {
        let cli = PathBuf::from("/opt/agent-plugins");
        let mut host = FakeHost::new()
            .with_path("/usr/bin")
            .with_root("/home/user/.local/bin")
            .with_executable(uv_path().to_str().expect("utf8"))
            .with_session("PATH", "/home/user/.local/bin:/usr/bin")
            .with_session(UV_NATIVE_TLS, "1");
        host.cli_dir = Some(cli.clone());
        let report = prepare_with(&mut host);
        assert_eq!(host.persist_calls, ["PATH"]);
        assert_eq!(report.published_path_dirs, std::slice::from_ref(&cli));
        assert_eq!(report.cli_dir_on_path, Some(cli));

        // A relaunch starts from the session, not from this run's process.
        host.vars.retain(|key, _| key == "PATH");
        let report = prepare_with(&mut host);
        assert_eq!(host.persist_calls, ["PATH"]);
        assert!(report.published_path_dirs.is_empty());
    }

    #[test]
    fn describe_redacts_proxy_credentials() {
        let status = ProxyStatus::FromSystem {
            http: "http://user:secret@proxy.corp:8080".to_string(),
            https: "http://user:secret@proxy.corp:8443".to_string(),
        };
        assert!(!status.describe().contains("secret"));
    }

    #[test]
    fn windows_names_need_an_extension() {
        assert_eq!(
            tool_file_names("claude", &["cmd".to_string(), "exe".to_string()]),
            ["claude.cmd", "claude.exe"]
        );
        assert_eq!(tool_file_names("claude", &[]), ["claude"]);
    }

    #[test]
    fn parses_the_published_uv_checksum_file() {
        let digest = "a252121d5b59398fcb137c6ea448176459a44010f33f67e0072305a637119ca7";
        assert_eq!(
            parse_sha256_file(format!("{digest}  uv-x86_64-pc-windows-msvc.zip\n").as_bytes())
                .as_deref(),
            Some(digest)
        );
        assert_eq!(parse_sha256_file(b"<html>Not Found</html>"), None);
    }
}
