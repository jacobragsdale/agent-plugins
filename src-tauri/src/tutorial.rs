//! The skill tutorial: with the person's permission, close an AI app, give it a
//! sample skill, and reopen it with a prompt that uses the skill. Supporting
//! another app is one more row in `APPS`.

use crate::agent_profiles::{AgentProfileState, TargetId};
use crate::fs_retry;
use crate::paths::SystemPaths;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Present once the tutorial has run, so it is offered only until then.
const SEEN_FILE: &str = "tutorial-seen";
const SKILL_NAME: &str = "agent-plugins-tutorial";
const SKILL: &str = "---
name: agent-plugins-tutorial
description: Shows how an Agent Plugins skill works. Use when the Agent Plugins tutorial asks for it.
---

# Agent Plugins tutorial

Greet the person in one short sentence. Then explain, in two plain sentences, that
Agent Plugins installed this skill a moment ago and that skills teach the AI app how
to do a particular job.
";
const PROMPT: &str = "/agent-plugins-tutorial Show me what this skill does.";
/// How long the app gets to close its windows before the tutorial gives up.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(20);

struct App {
    target: TargetId,
    /// The executable's file name, such as `Cursor.exe`.
    executable: &'static str,
    /// Folders the app may be installed in, most likely first.
    install_roots: fn() -> Vec<PathBuf>,
    /// Arguments that start the app, or hand over to the running one, with
    /// `prompt` ready to send.
    prompt_args: fn(&str) -> Vec<String>,
}

const APPS: [App; 1] = [App {
    target: TargetId::Cursor,
    executable: "Cursor.exe",
    install_roots: crate::agent_profiles::cursor_install_roots,
    prompt_args: cursor_prompt_args,
}];

/// Cursor has no command-line option for a prompt; this is the command Windows
/// runs for a `cursor://` link, without needing the link registered. Cursor
/// fills in its chat and waits for the person to send it.
fn cursor_prompt_args(prompt: &str) -> Vec<String> {
    // Encode spaces as %20: `byte_serialize` writes `+`, which not every reader decodes.
    let text = url::form_urlencoded::byte_serialize(prompt.as_bytes())
        .collect::<String>()
        .replace('+', "%20");
    vec![
        "--open-url".to_string(),
        "--".to_string(),
        format!("cursor://anysphere.cursor-deeplink/prompt?text={text}"),
    ]
}

fn app(target: TargetId) -> Option<&'static App> {
    APPS.iter().find(|app| app.target == target)
}

/// The detected app the tutorial can show, until it has run once. Closing an
/// app is only implemented for Windows so far.
pub(crate) fn offer(paths: &SystemPaths, profiles: &[AgentProfileState]) -> Option<TargetId> {
    if !cfg!(windows) || paths.app_data().join(SEEN_FILE).exists() {
        return None;
    }
    profiles
        .iter()
        .find(|profile| profile.detected && app(profile.target_id).is_some())
        .map(|profile| profile.target_id)
}

/// Closes the app, installs the tutorial skill where it reads skills, and
/// reopens it with the tutorial prompt. Nothing is closed unless the app's
/// executable was found first.
pub(crate) fn run(paths: &SystemPaths, target: TargetId) -> Result<(), String> {
    let name = target.display_name();
    let app = app(target).ok_or_else(|| format!("The tutorial doesn't support {name} yet."))?;
    let root = crate::adapters::skill_root(target, paths)
        .ok_or_else(|| format!("{name} doesn't read skills from this computer."))?;
    let running = processes(app.executable)?;
    // The copy the person already runs is the one to reopen.
    let executable = running
        .iter()
        .find_map(|(_, path)| path.clone())
        .or_else(|| installed_executable(app))
        .ok_or_else(|| format!("Could not find {} on this computer.", app.executable))?;
    if !running.is_empty() {
        close(app, &running)?;
    }
    let skill = root.join(SKILL_NAME);
    fs_retry::create_dir_all(&skill)
        .and_then(|()| fs_retry::replace_file(&skill.join("SKILL.md"), SKILL.as_bytes()))
        .map_err(|error| {
            format!(
                "Could not add the tutorial skill to {}: {}",
                skill.display(),
                fs_retry::plain(&error)
            )
        })?;
    launch(&executable, &(app.prompt_args)(PROMPT)).map_err(|error| {
        format!(
            "Could not open {name} from {}: {error}",
            executable.display()
        )
    })?;
    dismiss(paths)
}

/// Stops offering the tutorial, after it ran or when the person says not now.
/// **Reset** offers it again.
pub(crate) fn dismiss(paths: &SystemPaths) -> Result<(), String> {
    fs_retry::create_dir_all(&paths.app_data())
        .and_then(|()| fs_retry::write_synced(&paths.app_data().join(SEEN_FILE), b""))
        .map_err(|error| format!("Could not record that the tutorial was seen: {error}"))
}

fn installed_executable(app: &App) -> Option<PathBuf> {
    (app.install_roots)()
        .into_iter()
        .map(|root| root.join(app.executable))
        .find(|path| path.is_file())
}

/// Asks the app to close its windows, the same as clicking X, so it can save
/// its work or ask about it. Never forces it: a window left asking is reported.
fn close(app: &App, running: &[(u32, Option<PathBuf>)]) -> Result<(), String> {
    close_windows(&running.iter().map(|(id, _)| *id).collect::<Vec<_>>());
    let started = Instant::now();
    while !processes(app.executable)?.is_empty() {
        if started.elapsed() > CLOSE_TIMEOUT {
            return Err(format!(
                "{} is still open. Save your work, close it, and try again.",
                app.target.display_name()
            ));
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(())
}

fn launch(executable: &Path, args: &[String]) -> std::io::Result<()> {
    Command::new(executable)
        .args(args)
        // Set when Agent Plugins itself was started from an Electron app's
        // terminal; it would make the app run as plain Node.js.
        .env_remove("ELECTRON_RUN_AS_NODE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
}

/// This session's processes named `executable`, with their paths. Other
/// sessions' copies, on a shared machine, are not this person's to close.
/// Native calls rather than tasklist, which needs WMI.
#[cfg(windows)]
fn processes(executable: &str) -> Result<Vec<(u32, Option<PathBuf>)>, String> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    let session = session_of(std::process::id());
    // SAFETY: plain call; the handle is checked, then closed below.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(format!(
            "Could not list running apps: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: PROCESSENTRY32W is plain data, valid when zeroed.
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut found = Vec::new();
    // SAFETY: `entry` is a live PROCESSENTRY32W with `dwSize` set.
    let mut more = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
    while more {
        let name = &entry.szExeFile;
        let name = String::from_utf16_lossy(
            &name[..name.iter().position(|&c| c == 0).unwrap_or(name.len())],
        );
        let id = entry.th32ProcessID;
        if name.eq_ignore_ascii_case(executable) && session.is_some() && session_of(id) == session {
            found.push((id, image_path(id)));
        }
        // SAFETY: as above.
        more = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
    }
    // SAFETY: `snapshot` is a valid handle owned here.
    unsafe { CloseHandle(snapshot) };
    Ok(found)
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

/// Posts WM_CLOSE to the visible top-level windows of `processes`.
#[cfg(windows)]
fn close_windows(processes: &[u32]) {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
    };
    unsafe extern "system" fn visit(window: HWND, processes: LPARAM) -> BOOL {
        // SAFETY: `processes` is the slice reference passed to EnumWindows
        // below, alive for the whole enumeration.
        let processes = unsafe { &*(processes as *const &[u32]) };
        let mut owner = 0;
        // SAFETY: `window` comes from EnumWindows; `owner` is a live u32.
        unsafe {
            GetWindowThreadProcessId(window, &mut owner);
            if processes.contains(&owner) && IsWindowVisible(window) != 0 {
                PostMessageW(window, WM_CLOSE, 0, 0);
            }
        }
        1
    }
    // SAFETY: `visit` only reads `processes` during this call.
    unsafe { EnumWindows(Some(visit), &processes as *const &[u32] as LPARAM) };
}

#[cfg(not(windows))]
fn processes(_executable: &str) -> Result<Vec<(u32, Option<PathBuf>)>, String> {
    Err("The tutorial only runs on Windows so far.".to_string())
}

#[cfg(not(windows))]
fn close_windows(_processes: &[u32]) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_prompt_args_percent_encode_the_prompt() {
        assert_eq!(
            cursor_prompt_args("/skill a+b & c"),
            [
                "--open-url",
                "--",
                "cursor://anysphere.cursor-deeplink/prompt?text=%2Fskill%20a%2Bb%20%26%20c"
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn processes_finds_this_process_and_its_path() {
        let me = std::env::current_exe().expect("current exe");
        let name = me.file_name().and_then(|name| name.to_str()).expect("name");
        let found = processes(name).expect("processes");
        assert!(found
            .iter()
            .any(|(id, path)| *id == std::process::id() && path.as_deref() == Some(me.as_path())));
    }
}
