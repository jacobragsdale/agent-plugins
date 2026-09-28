//! Where AI apps are installed, and the program that opens each one.

use crate::agent_profiles::TargetId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// An installation Agent Plugins can open with a prompt. Several can belong
/// to one target: GitHub Copilot is VS Code, VS Code Insiders, and its CLI.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum App {
    Vscode,
    VscodeInsiders,
    CopilotCli,
    Cursor,
    ClaudeCode,
    ClaudeDesktop,
    #[serde(rename = "opencode")]
    OpenCode,
    Pi,
    Codex,
    Chatgpt,
    GrokBuild,
}

impl App {
    /// Primary apps first; the picker lists them in this order.
    pub(crate) const ALL: [Self; 11] = [
        Self::Vscode,
        Self::VscodeInsiders,
        Self::CopilotCli,
        Self::Cursor,
        Self::ClaudeCode,
        Self::ClaudeDesktop,
        Self::OpenCode,
        Self::Pi,
        Self::Codex,
        Self::Chatgpt,
        Self::GrokBuild,
    ];

    pub(crate) fn target(self) -> TargetId {
        match self {
            Self::Vscode | Self::VscodeInsiders | Self::CopilotCli => TargetId::GithubCopilot,
            Self::Cursor => TargetId::Cursor,
            Self::ClaudeCode => TargetId::ClaudeCode,
            Self::ClaudeDesktop => TargetId::ClaudeDesktop,
            Self::OpenCode => TargetId::OpenCode,
            Self::Pi => TargetId::Pi,
            Self::Codex => TargetId::Codex,
            Self::Chatgpt => TargetId::Chatgpt,
            Self::GrokBuild => TargetId::GrokBuild,
        }
    }

    /// The command a CLI runs as. Desktop apps have none.
    fn command(self) -> Option<&'static str> {
        match self {
            Self::CopilotCli => Some("copilot"),
            Self::ClaudeCode => Some("claude"),
            Self::OpenCode => Some("opencode"),
            Self::Pi => Some("pi"),
            Self::Codex => Some("codex"),
            Self::GrokBuild => Some("grok"),
            Self::Vscode
            | Self::VscodeInsiders
            | Self::Cursor
            | Self::ClaudeDesktop
            | Self::Chatgpt => None,
        }
    }
}

/// The program that opens `app`, or `None` when it isn't installed here.
pub(crate) fn find(app: App) -> Option<PathBuf> {
    if let Ok(cache) = FOUND.lock() {
        if let Some((_, found)) = cache.get(&app) {
            return found.clone();
        }
    }
    let found = locate(app);
    if let Ok(mut cache) = FOUND.lock() {
        cache.insert(app, (Instant::now(), found.clone()));
    }
    found
}

/// Looking reads the registry and the process list, or asks Spotlight, so
/// each answer is kept as detection's are: a state reload never looks again;
/// a focus or diagnostics clears them, and a sync forgets the stale ones.
static FOUND: Mutex<BTreeMap<App, (Instant, Option<PathBuf>)>> = Mutex::new(BTreeMap::new());

/// Forgets what `find` found, so the next call looks at the machine again.
pub(crate) fn clear_cache() {
    if let Ok(mut cache) = FOUND.lock() {
        cache.clear();
    }
}

/// Forgets answers older than `age`, as a sync does for detection.
pub(crate) fn expire_cache(age: Duration) {
    if let Ok(mut cache) = FOUND.lock() {
        cache.retain(|_, (found_at, _)| found_at.elapsed() < age);
    }
}

/// The folder of a VS Code-style app that holds `resources/app`, or its
/// `.app` bundle, for a program `find` returned.
pub(crate) fn editor_root(program: &Path) -> Option<&Path> {
    if program
        .extension()
        .is_some_and(|extension| extension == "app")
    {
        return Some(program);
    }
    let folder = program.parent()?;
    if folder.ends_with("Contents/Resources/app/bin") {
        folder.ancestors().nth(4)
    } else if folder.ends_with("bin") {
        folder.parent()
    } else {
        Some(folder)
    }
}

/// A CLI on the login PATH or where its installer puts it. On Windows an
/// MSIX alias counts too, and an npm `.cmd` shim gives way to the native
/// program it starts when that is known.
fn find_cli(app: App) -> Option<PathBuf> {
    let name = app.command()?;
    #[cfg(windows)]
    {
        crate::startup::find_program(name)
            .or_else(|| windows_apps_alias(name))
            .map(|program| native_beside_npm_shim(app, &program).unwrap_or(program))
    }
    #[cfg(not(windows))]
    {
        crate::startup::find_program(name)
    }
}

/// npm's `.cmd` shim for Claude Code or Copilot CLI starts a native program
/// in the package beside it, which takes arguments a batch file can't.
#[cfg(any(test, windows))]
fn native_beside_npm_shim(app: App, shim: &Path) -> Option<PathBuf> {
    if !shim
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cmd"))
    {
        return None;
    }
    let modules = shim.parent()?.join("node_modules");
    let native = if cfg!(target_arch = "aarch64") {
        "copilot-win32-arm64"
    } else {
        "copilot-win32-x64"
    };
    let under = |parts: &[&str]| {
        parts
            .iter()
            .fold(modules.clone(), |path, part| path.join(part))
    };
    let candidates = match app {
        App::ClaudeCode => vec![under(&[
            "@anthropic-ai",
            "claude-code",
            "bin",
            "claude.exe",
        ])],
        // Nested under the package, or hoisted beside it.
        App::CopilotCli => vec![
            under(&[
                "@github",
                "copilot",
                "node_modules",
                "@github",
                native,
                "copilot.exe",
            ]),
            under(&["@github", native, "copilot.exe"]),
        ],
        _ => Vec::new(),
    };
    candidates.into_iter().find(|program| program.is_file())
}

/// Registered MSIX packages for this user, as full names such as
/// `Claude_1.8555.2.0_x64__pzs8sxrjxfjjc`. Reading the repository key needs
/// no subprocess, unlike `Get-AppxPackage`. A folder under
/// `%LOCALAPPDATA%\Packages` is no proof: an uninstall can leave it behind.
#[cfg(windows)]
fn msix_package_full_names() -> Vec<String> {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(MSIX_REPOSITORY)
        .map(|key| key.enum_keys().flatten().collect())
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn msix_package_full_names() -> Vec<String> {
    Vec::new()
}

#[cfg(windows)]
const MSIX_REPOSITORY: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";

/// The registered package of an MSIX family, as its full name.
pub(crate) fn msix_package(family: &str) -> Option<String> {
    msix_full_name(&msix_package_full_names(), family).map(str::to_string)
}

/// A family is `<name>_<publisher>`; a registered full name is
/// `<name>_<version>_<arch>__<publisher>`.
fn msix_full_name<'a>(full_names: &'a [String], family: &str) -> Option<&'a str> {
    let (name, publisher) = family.rsplit_once('_')?;
    let suffix = format!("__{publisher}");
    full_names.iter().map(String::as_str).find(|full| {
        full.strip_prefix(name)
            .is_some_and(|rest| rest.starts_with('_'))
            && full.ends_with(&suffix)
    })
}

/// The version in an MSIX package's full name.
pub(crate) fn msix_version(full_name: &str) -> Option<String> {
    full_name.split('_').nth(1).map(str::to_string)
}

/// The version a macOS app bundle declares.
#[cfg(target_os = "macos")]
pub(crate) fn bundle_version(bundle: &Path) -> Option<String> {
    bundle_string(bundle, "CFBundleShortVersionString")
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn bundle_version(_bundle: &Path) -> Option<String> {
    None
}

#[cfg(any(test, target_os = "macos"))]
fn bundle_string(bundle: &Path, key: &str) -> Option<String> {
    plist::Value::from_file(bundle.join("Contents/Info.plist"))
        .ok()?
        .as_dictionary()?
        .get(key)?
        .as_string()
        .map(str::to_string)
}

/// The first of `names` in `folders` whose bundle id is `id`. The id tells
/// the ChatGPT app with Codex from ChatGPT Classic, which has the same name.
#[cfg(any(test, target_os = "macos"))]
fn app_bundle(folders: &[PathBuf], names: &[&str], id: &str) -> Option<PathBuf> {
    for folder in folders {
        for name in names {
            let bundle = folder.join(format!("{name}.app"));
            if bundle_string(&bundle, "CFBundleIdentifier").as_deref() == Some(id) {
                return Some(bundle);
            }
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn locate(app: App) -> Option<PathBuf> {
    let (names, id): (&[&str], &str) = match app {
        App::Vscode => (&["Visual Studio Code"], "com.microsoft.VSCode"),
        App::VscodeInsiders => (
            &["Visual Studio Code - Insiders"],
            "com.microsoft.VSCodeInsiders",
        ),
        App::Cursor => (&["Cursor"], "com.todesktop.230313mzl4w4u92"),
        App::ClaudeDesktop => (&["Claude"], "com.anthropic.claudefordesktop"),
        // Named ChatGPT since the Codex app became it, or Codex before that.
        App::Chatgpt => (&["ChatGPT", "Codex"], "com.openai.codex"),
        _ => return find_cli(app),
    };
    let mut folders = vec![PathBuf::from("/Applications")];
    folders.extend(dirs::home_dir().map(|home| home.join("Applications")));
    let bundle = app_bundle(&folders, names, id).or_else(|| spotlight(id))?;
    match app {
        // Both editions name their command-line script `code` inside the bundle.
        App::Vscode | App::VscodeInsiders => {
            Some(bundle.join("Contents/Resources/app/bin/code")).filter(|code| code.is_file())
        }
        _ => Some(bundle),
    }
}

/// Where Spotlight has seen the app with bundle id `id`, such as a copy
/// moved out of Applications. Nothing when Spotlight is off.
#[cfg(target_os = "macos")]
fn spotlight(id: &str) -> Option<PathBuf> {
    let mut command = crate::process::command(Path::new("/usr/bin/mdfind"));
    command.arg(format!("kMDItemCFBundleIdentifier == '{id}'"));
    let output = crate::process::run(
        command,
        "Spotlight search",
        std::time::Duration::from_secs(2),
    )
    .ok()?;
    let paths = String::from_utf8_lossy(&output.stdout);
    paths
        .lines()
        .map(PathBuf::from)
        .find(|bundle| bundle.is_dir() && !bundle.to_string_lossy().contains("/.Trash/"))
}

/// Linux gets the default install paths only.
#[cfg(not(any(windows, target_os = "macos")))]
fn locate(app: App) -> Option<PathBuf> {
    let home = dirs::home_dir();
    let at_home = |relative: &str| home.as_ref().map(|home| home.join(relative));
    let candidates = match app {
        App::Vscode => vec![
            Some(PathBuf::from("/usr/share/code/bin/code")),
            at_home(".local/share/code/bin/code"),
        ],
        App::VscodeInsiders => vec![
            Some(PathBuf::from("/usr/share/code-insiders/bin/code-insiders")),
            at_home(".local/share/code-insiders/bin/code-insiders"),
        ],
        App::Cursor => vec![
            Some(PathBuf::from("/usr/share/cursor/cursor")),
            Some(PathBuf::from("/opt/Cursor/cursor")),
            at_home(".local/share/cursor/cursor"),
        ],
        App::ClaudeDesktop => vec![Some(PathBuf::from("/usr/bin/claude-desktop"))],
        App::Chatgpt => return None,
        _ => return find_cli(app),
    };
    candidates
        .into_iter()
        .flatten()
        .find(|program| program.is_file())
}

#[cfg(windows)]
const CHATGPT_MSIX: &str = "OpenAI.Codex_2p2nqsd0c76g0";

#[cfg(windows)]
fn locate(app: App) -> Option<PathBuf> {
    match app {
        App::Vscode => editor(&VSCODE),
        App::VscodeInsiders => editor(&VSCODE_INSIDERS),
        App::Cursor => editor(&CURSOR),
        // The Microsoft Store build answers to an alias; the Squirrel build
        // winget installs keeps a stub that starts its newest version.
        App::ClaudeDesktop => windows_apps_alias("claude-desktop").or_else(|| {
            Some(dirs::data_local_dir()?.join(r"AnthropicClaude\claude.exe"))
                .filter(|stub| stub.is_file())
        }),
        // Opened by its link, so the package only has to be registered. Its
        // name stays `OpenAI.Codex` now that the app is called ChatGPT;
        // ChatGPT Classic, which has no Codex, is another package.
        App::Chatgpt => msix_package(CHATGPT_MSIX).map(|full_name| package_root(&full_name)),
        _ => find_cli(app),
    }
}

/// An MSIX app execution alias, a reparse point that `metadata` can't read.
#[cfg(windows)]
fn windows_apps_alias(name: &str) -> Option<PathBuf> {
    let alias = dirs::data_local_dir()?
        .join(r"Microsoft\WindowsApps")
        .join(format!("{name}.exe"));
    alias.symlink_metadata().is_ok().then_some(alias)
}

#[cfg(windows)]
fn package_root(full_name: &str) -> PathBuf {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(format!(r"{MSIX_REPOSITORY}\{full_name}"))
        .and_then(|key| key.get_value::<String, _>("PackageRootFolder"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env_path("ProgramFiles")
                .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
                .join("WindowsApps")
                .join(full_name)
        })
}

#[cfg(windows)]
fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// A VS Code-style editor on Windows, and the places that record its folder.
#[cfg(windows)]
struct Editor {
    /// The program in the install folder.
    exe: &'static str,
    /// Its folder under `%LOCALAPPDATA%\Programs` and `%ProgramFiles%`.
    folder: &'static str,
    /// Its Scoop app name.
    scoop: &'static str,
    /// Its uninstall entry, less the ` (User)` a per-user install adds.
    display_name: &'static str,
    /// The link scheme it registers on first launch.
    scheme: &'static str,
    /// Its `App Paths` entry, when the installer writes one.
    app_path: Option<&'static str>,
    /// Its command on PATH, and how many levels above that file the install
    /// folder is.
    command: (&'static str, usize),
}

#[cfg(windows)]
const VSCODE: Editor = Editor {
    exe: "Code.exe",
    folder: "Microsoft VS Code",
    scoop: "vscode",
    display_name: "Microsoft Visual Studio Code",
    scheme: "vscode",
    app_path: Some("code"),
    // <root>\bin\code.cmd
    command: ("code", 2),
};

#[cfg(windows)]
const VSCODE_INSIDERS: Editor = Editor {
    exe: "Code - Insiders.exe",
    folder: "Microsoft VS Code Insiders",
    scoop: "vscode-insiders",
    display_name: "Microsoft Visual Studio Code Insiders",
    scheme: "vscode-insiders",
    app_path: Some("code-insiders"),
    command: ("code-insiders", 2),
};

#[cfg(windows)]
const CURSOR: Editor = Editor {
    exe: "Cursor.exe",
    folder: "cursor",
    scoop: "cursor",
    display_name: "Cursor",
    scheme: "cursor",
    app_path: None,
    // <root>\resources\app\bin\cursor.cmd
    command: ("cursor", 4),
};

/// The copy the person runs, or else the first one installed, looking in
/// the cheap places first.
#[cfg(windows)]
fn editor(editor: &Editor) -> Option<PathBuf> {
    use std::iter::once_with;
    let running = once_with(|| {
        running(editor.exe).and_then(|program| program.parent().map(Path::to_path_buf))
    })
    .flatten();
    let defaults = [
        dirs::data_local_dir().map(|local| local.join("Programs")),
        env_path("ProgramFiles"),
        env_path("ProgramFiles(x86)"),
    ]
    .into_iter()
    .flatten()
    .map(|programs| programs.join(editor.folder));
    let registered = once_with(|| registered_roots(editor)).flatten();
    let scoop = scoop_roots(editor.scoop);
    let command = once_with(|| {
        let (name, depth) = editor.command;
        crate::startup::find_program(name)
            .and_then(|shim| shim.ancestors().nth(depth).map(Path::to_path_buf))
    })
    .flatten();
    running
        .chain(defaults)
        .chain(registered)
        .chain(scoop)
        .chain(command)
        // An empty InstallLocation would otherwise look in the working folder.
        .filter(|root| root.is_absolute())
        .map(|root| root.join(editor.exe))
        .find(|program| program.is_file())
}

/// Where Windows records an editor outside its default folders: its
/// uninstall entries, per user and per machine; its `App Paths` entry; and
/// the program its link scheme opens, which a portable copy sets on first
/// launch.
#[cfg(windows)]
fn registered_roots(editor: &Editor) -> Vec<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let mut roots = Vec::new();
    let variant = format!("{} (", editor.display_name);
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
            if name != editor.display_name && !name.starts_with(&variant) {
                continue;
            }
            if let Ok(location) = entry.get_value::<String, _>("InstallLocation") {
                roots.push(PathBuf::from(location.trim()));
            }
            if let Some(program) = entry
                .get_value::<String, _>("DisplayIcon")
                .ok()
                .and_then(|icon| display_icon_exe(&icon))
            {
                roots.extend(program.parent().map(Path::to_path_buf));
            }
        }
    }
    if let Some(program) = editor.app_path.and_then(crate::startup::windows_app_path) {
        roots.extend(program.parent().map(Path::to_path_buf));
    }
    for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        // Such as `"C:\...\Cursor.exe" --open-url -- "%1"`.
        let Ok(command) = RegKey::predef(hive)
            .open_subkey(format!(
                r"Software\Classes\{}\shell\open\command",
                editor.scheme
            ))
            .and_then(|key| key.get_value::<String, _>(""))
        else {
            continue;
        };
        if let Some(folder) =
            command_program(&command).and_then(|program| Path::new(program).parent())
        {
            roots.push(folder.to_path_buf());
        }
    }
    roots
}

/// Scoop's `current` folder for `app`, per user and global.
#[cfg(windows)]
fn scoop_roots(app: &str) -> Vec<PathBuf> {
    [
        env_path("SCOOP"),
        dirs::home_dir().map(|home| home.join("scoop")),
        env_path("SCOOP_GLOBAL"),
        env_path("ProgramData").map(|data| data.join("scoop")),
    ]
    .into_iter()
    .flatten()
    .map(|scoop| scoop.join("apps").join(app).join("current"))
    .collect()
}

/// The program a registered command line starts, such as `C:\...\Cursor.exe`
/// in `"C:\...\Cursor.exe" --open-url -- "%1"`.
#[cfg(any(test, windows))]
fn command_program(command: &str) -> Option<&str> {
    let command = command.trim();
    match command.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next(),
        None => command.split_whitespace().next(),
    }
}

/// An uninstall entry's `DisplayIcon` is a path, quoted or not, maybe with
/// `,<index>`. Only a program says where the app is: some point at an icon.
#[cfg(any(test, windows))]
fn display_icon_exe(icon: &str) -> Option<PathBuf> {
    let icon = icon.trim();
    let path = match icon.strip_prefix('"') {
        Some(quoted) => quoted.split('"').next()?,
        None => icon
            .rsplit_once(',')
            .filter(|(_, index)| index.trim().parse::<i32>().is_ok())
            .map_or(icon, |(path, _)| path),
    };
    path.to_ascii_lowercase()
        .ends_with(".exe")
        .then(|| PathBuf::from(path))
}

/// Where this session's running `executable` started from: the copy the
/// person uses, wherever it is. Other sessions' copies, on a shared machine,
/// are not this person's. Native calls rather than tasklist, which needs WMI.
#[cfg(windows)]
fn running(executable: &str) -> Option<PathBuf> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let session = session_of(std::process::id())?;
    // SAFETY: plain call; the handle is checked, then closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return None;
    }
    // SAFETY: PROCESSENTRY32W is plain data, valid when zeroed.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = None;
    // SAFETY: `entry` is a live PROCESSENTRY32W with `dwSize` set.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more && found.is_none() {
        let name = &entry.szExeFile;
        let name = String::from_utf16_lossy(
            &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())],
        );
        let id = entry.th32ProcessID;
        if name.eq_ignore_ascii_case(executable) && session_of(id) == Some(session) {
            found = image_path(id);
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: `snapshot` is a valid handle owned here.
    unsafe { CloseHandle(snapshot) };
    found
}

#[cfg(windows)]
fn session_of(process: u32) -> Option<u32> {
    let mut session = 0;
    // SAFETY: `session` is a live u32. Fails for processes this user may not query.
    (unsafe {
        windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(process, &mut session)
    } != 0)
        .then_some(session)
}

#[cfg(windows)]
fn image_path(process: u32) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt as _;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    // SAFETY: plain call; a null handle means no access.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process) };
    if handle.is_null() {
        return None;
    }
    let mut buffer = [0u16; 32_768];
    let mut length = buffer.len() as u32;
    // SAFETY: `buffer` holds `length` u16s; the handle is closed right after.
    let ok = unsafe {
        QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut length)
    } != 0;
    // SAFETY: `handle` is a valid handle owned here.
    unsafe { CloseHandle(handle) };
    ok.then(|| PathBuf::from(std::ffi::OsString::from_wide(&buffer[..length as usize])))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn apps_serialize_to_the_wire_ids_and_belong_to_their_targets() {
        let ids = App::ALL
            .into_iter()
            .map(|app| serde_json::to_value(app).expect("serialize app"))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "vscode",
                "vscode-insiders",
                "copilot-cli",
                "cursor",
                "claude-code",
                "claude-desktop",
                "opencode",
                "pi",
                "codex",
                "chatgpt",
                "grok-build"
            ]
            .map(serde_json::Value::from)
        );
        assert_eq!(App::CopilotCli.target(), TargetId::GithubCopilot);
        assert_eq!(App::Chatgpt.target(), TargetId::Chatgpt);
    }

    #[test]
    fn editor_root_undoes_where_find_points_on_each_system() {
        for (program, root) in [
            (
                "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
                "/Applications/Visual Studio Code.app",
            ),
            ("/Applications/Cursor.app", "/Applications/Cursor.app"),
            ("/usr/share/code/bin/code", "/usr/share/code"),
            ("/usr/share/cursor/cursor", "/usr/share/cursor"),
        ] {
            assert_eq!(editor_root(Path::new(program)), Some(Path::new(root)));
        }
    }

    #[test]
    fn msix_packages_match_the_whole_name_and_publisher() {
        let names = [
            "OpenAI.ChatGPT-Desktop_1.2024.345.0_x64__2p2nqsd0c76g0".to_string(),
            "Claude_1.8555.2.0_x64__pzs8sxrjxfjjc".to_string(),
        ];
        let claude = msix_full_name(&names, "Claude_pzs8sxrjxfjjc").expect("registered");
        assert_eq!(msix_version(claude).as_deref(), Some("1.8555.2.0"));
        assert_eq!(msix_full_name(&names, "Claude_otherpublisher"), None);
        // ChatGPT Classic is not the Codex app.
        assert_eq!(msix_full_name(&names, "OpenAI.Codex_2p2nqsd0c76g0"), None);
        assert_eq!(msix_full_name(&names, "OpenAI.ChatGPT_2p2nqsd0c76g0"), None);
    }

    #[test]
    fn registered_command_lines_and_icons_name_the_program() {
        assert_eq!(
            command_program(r#""C:\Apps\Cursor.exe" --open-url -- "%1""#),
            Some(r"C:\Apps\Cursor.exe")
        );
        assert_eq!(
            command_program(r"C:\Apps\Code.exe --open-url"),
            Some(r"C:\Apps\Code.exe")
        );
        for (icon, program) in [
            (
                r#""C:\Program Files\Code\Code.exe",0"#,
                Some(r"C:\Program Files\Code\Code.exe"),
            ),
            (
                r"C:\Program Files\Code\Code.exe,0",
                Some(r"C:\Program Files\Code\Code.exe"),
            ),
            (
                r"C:\Program Files\Code\CODE.EXE",
                Some(r"C:\Program Files\Code\CODE.EXE"),
            ),
            (r"C:\Users\me\AppData\Local\AnthropicClaude\app.ico", None),
        ] {
            assert_eq!(display_icon_exe(icon), program.map(PathBuf::from), "{icon}");
        }
    }

    #[test]
    fn npm_shims_give_way_to_the_native_program_beside_them() {
        let npm = tempfile::tempdir().expect("npm");
        let claude = npm
            .path()
            .join("node_modules/@anthropic-ai/claude-code/bin/claude.exe");
        fs::create_dir_all(claude.parent().expect("bin")).expect("package");
        fs::write(&claude, b"").expect("claude.exe");
        let shim = npm.path().join("claude.cmd");
        assert_eq!(native_beside_npm_shim(App::ClaudeCode, &shim), Some(claude));
        assert_eq!(
            native_beside_npm_shim(App::CopilotCli, &npm.path().join("copilot.cmd")),
            None
        );
        assert_eq!(
            native_beside_npm_shim(App::ClaudeCode, &npm.path().join("claude.exe")),
            None
        );
    }

    #[test]
    fn a_mac_app_counts_only_with_the_expected_bundle_id() {
        let applications = tempfile::tempdir().expect("applications");
        let bundle = |name: &str, id: &str| {
            let contents = applications.path().join(format!("{name}.app/Contents"));
            fs::create_dir_all(&contents).expect("bundle");
            fs::write(
                contents.join("Info.plist"),
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{id}</string>
<key>CFBundleShortVersionString</key><string>26.715.1</string>
</dict></plist>"#
                ),
            )
            .expect("Info.plist");
        };
        let folders = [applications.path().to_path_buf()];
        let names: &[&str] = &["ChatGPT", "Codex"];
        bundle("ChatGPT", "com.openai.chat");
        assert_eq!(app_bundle(&folders, names, "com.openai.codex"), None);
        bundle("Codex", "com.openai.codex");
        let codex = app_bundle(&folders, names, "com.openai.codex").expect("Codex app");
        assert_eq!(codex, applications.path().join("Codex.app"));
        assert_eq!(
            bundle_string(&codex, "CFBundleShortVersionString").as_deref(),
            Some("26.715.1")
        );
    }

    #[cfg(windows)]
    #[test]
    fn running_finds_this_process() {
        let me = std::env::current_exe().expect("current exe");
        let name = me.file_name().and_then(|name| name.to_str()).expect("name");
        assert_eq!(running(name), Some(me));
    }
}
