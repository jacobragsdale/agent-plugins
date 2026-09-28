//! Opening an AI app with a prompt: which detected apps can take one, the
//! skill tutorial (a sample skill that walks the person through making and
//! publishing their own, then a prompt that uses it), and **Create a
//! skill** (a prompt that has the app's agent write and publish one). No app
//! is closed first: every one of them picks up a new skill while it runs.

use crate::agent_profiles::{AgentProfileState, TargetId};
use crate::app_locations::App;
use crate::app_state::PromptApp;
use crate::fs_retry;
use crate::paths::SystemPaths;
use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Present once the tutorial has run, so it is offered only until then.
const SEEN_FILE: &str = "tutorial-seen";
const SKILL_NAME: &str = "agent-plugins-tutorial";
/// The description names only the skill itself: a broader one answered any
/// prompt that mentions Agent Plugins, **Create a skill**'s included. Marking
/// it manual does not work either, since Cursor then ignores the command when
/// it arrives as text from a link.
const SKILL: &str = "---
name: agent-plugins-tutorial
description: Answers a message that calls the agent-plugins-tutorial skill by name. Use only when the message names agent-plugins-tutorial; it is not a guide to Agent Plugins.
---

# Agent Plugins tutorial

Greet the person in one short sentence. Then explain, in two plain sentences, that
Agent Plugins installed this skill a moment ago and that skills teach the AI app how
to do a particular job. Say that this one walks them through making a skill of their
own and sharing it, then ask the first question of step 1 below.

From there, do what this request asks, as if the person had sent it:

";
/// Everything the agent needs to interview the person, write a skill, and
/// publish it; `create_prompt` fills in `{cli}` and `{skill}`. The app gets it
/// as a file, which no command line, shell, or link length limit can mangle.
const CREATE_PROMPT: &str = r#"Help me make a new skill for my AI apps and share it on the company marketplace. I'm not a developer: use plain words and ask one question at a time.

A skill is a folder with a SKILL.md file: frontmatter with a name and a description, then the instructions an AI follows when it uses the skill. The Agent Plugins command line is {cli}.

1. Interview me until you could write the skill yourself. Ask what job it does; for a real example of what I would give it and what a great result looks like; when it should be used, in the words I would say; and any steps, rules, tone, or format it must follow or avoid.
2. Run the command line with `whoami`. If its teams line names any teams, ask me whether the skill is just for me or for one of those teams. The namespace is the team's name if I pick a team, otherwise the namespace line. Just for me means only I can see it until I share it.
3. Pick a short lowercase hyphenated name, such as meeting-notes, or use mine. Write {skill}:
   - The folder and the frontmatter name are both <namespace>-<name>, even when I chose the name: the namespace from step 2 always comes first, as in <namespace>-meeting-notes.
   - The description says what the skill does and when to use it, in one or two sentences with the words I would use. Other people's AI apps read only this to decide when to use the skill.
   - The body has short numbered steps, my rules, and one worked example. Keep it under 150 lines, written for an assistant that is smart but new to my job.
   - Never include passwords, keys, tokens, customer data, or personal details.
4. Show me the skill and revise it until I'm happy. Suggest I try it; if it doesn't show up, start a new chat or session.
5. When I say it's ready, run the command line with: validate "<the skill folder>" --namespace <namespace>, and fix anything it reports. Propose up to five lowercase tags people would search for and a one-line changelog. Run: publish "<the skill folder>" --namespace <namespace> --version 1.0.0 --tags <a,b> --changelog "<text>" --dry-run, adding --private if it is just for me, and show me what it would publish. Only after I say yes, run the same command with --yes instead of --dry-run.
6. Give me the link it prints. If it's for a team, my teammates can find it and install it in Agent Plugins; if it's just for me, only I can, until I share it. I can keep using my copy.

If the version is taken, use the one it suggests. If it refuses because something looks like a secret or breaks a rule, explain why in plain words and fix the skill; never work around the check."#;
/// How long a launcher that passes the prompt on and exits, such as `open`, may take.
const HAND_OVER_TIMEOUT: Duration = Duration::from_secs(30);

/// Detected apps that can be opened with a prompt, in picker order.
pub(crate) fn prompt_apps(profiles: &[AgentProfileState]) -> Vec<PromptApp> {
    App::ALL
        .into_iter()
        .filter(|app| {
            profiles
                .iter()
                .any(|profile| profile.detected && profile.target_id == app.target())
                && crate::app_locations::find(*app).is_some()
        })
        .map(|app| PromptApp {
            id: app,
            target_id: app.target(),
            label: label(app),
        })
        .collect()
}

/// Offers the tutorial until it has run once, while some app can take it.
pub(crate) fn offer(paths: &SystemPaths, apps: &[PromptApp]) -> bool {
    !apps.is_empty() && !paths.app_data().join(SEEN_FILE).exists()
}

fn label(app: App) -> &'static str {
    match app {
        App::Vscode => "GitHub Copilot in VS Code",
        App::VscodeInsiders => "GitHub Copilot in VS Code Insiders",
        App::CopilotCli => "GitHub Copilot CLI",
        App::Cursor => "Cursor",
        App::ClaudeCode => "Claude Code",
        App::ClaudeDesktop => "Claude Desktop",
        App::OpenCode => "OpenCode",
        App::Pi => "pi",
        App::Codex => "Codex CLI",
        App::Chatgpt => "ChatGPT (Codex)",
        App::GrokBuild => "Grok Build",
    }
}

/// Gives `app` the tutorial skill and opens it with a prompt that uses the
/// skill. Returns what the person does next.
pub(crate) fn run(paths: &SystemPaths, app: App) -> Result<String, String> {
    let program = program(app)?;
    let skills = skills_root(app, paths)?;
    let text = format!("{SKILL}{}", create_prompt(&skills, &command_line()?));
    write_skill(&skills, &text)?;
    open(app, &program, &tutorial_prompt(app), paths)?;
    dismiss(paths)?;
    Ok(format!(
        "{} It will show you how to make a skill of your own, one question at a time.",
        next_step(app)
    ))
}

/// Stops offering the tutorial, after it ran or when the person says not now.
/// **Reset** offers it again.
pub(crate) fn dismiss(paths: &SystemPaths) -> Result<(), String> {
    fs_retry::create_dir_all(&paths.app_data())
        .and_then(|()| fs_retry::write_synced(&paths.app_data().join(SEEN_FILE), b""))
        .map_err(|error| format!("Could not record that the tutorial was seen: {error}"))
}

/// Removes the sample skill from every folder the tutorial can put it in, so
/// **Reset** leaves nothing of Agent Plugins behind.
pub(crate) fn remove_skill(paths: &SystemPaths) -> Result<(), String> {
    let roots: BTreeSet<PathBuf> = App::ALL
        .into_iter()
        .filter_map(|app| skills_root(app, paths).ok())
        .collect();
    for root in roots {
        let skill = root.join(SKILL_NAME);
        remove_if_present(&skill)
            .map_err(|error| failed("remove the tutorial skill at", &skill, &error))?;
    }
    Ok(())
}

/// Writes instructions that have the app's agent interview the person, write
/// a skill where the app reads skills, and publish it, then opens `app` with
/// a prompt that points at them. Returns what the person does next.
pub(crate) fn create_skill(paths: &SystemPaths, app: App) -> Result<String, String> {
    let program = program(app)?;
    let skills = skills_root(app, paths)?;
    let instructions = paths.app_data().join("create-skill.md");
    let text = create_prompt(&skills, &command_line()?);
    // Claude Code only notices new skills in a folder that existed when it started.
    fs_retry::create_dir_all(&skills).map_err(|error| failed("create", &skills, &error))?;
    fs_retry::create_dir_all(&paths.app_data())
        .and_then(|()| fs_retry::replace_file(&instructions, text.as_bytes()))
        .map_err(|error| failed("write", &instructions, &error))?;
    let prompt = format!(
        "Read {} and follow the instructions in it.",
        instructions.display()
    );
    open(app, &program, &prompt, paths)?;
    Ok(format!(
        "{} It will ask about the skill you want to make, one question at a time.",
        next_step(app)
    ))
}

fn program(app: App) -> Result<PathBuf, String> {
    crate::app_locations::find(app)
        .ok_or_else(|| format!("Could not find {} on this computer.", label(app)))
}

/// Where `app` reads personal skills. Claude Desktop's Code tab reads Claude
/// Code's; `adapters` installs to Claude Desktop by upload, for its chats.
fn skills_root(app: App, paths: &SystemPaths) -> Result<PathBuf, String> {
    let target = match app {
        App::ClaudeDesktop => TargetId::ClaudeCode,
        _ => app.target(),
    };
    crate::adapters::skill_root(target, paths)
        .ok_or_else(|| format!("{} doesn't read skills from this computer.", label(app)))
}

/// The one-line prompt that calls the sample skill, in the app's own syntax.
fn tutorial_prompt(app: App) -> String {
    let call = match app {
        App::Pi => "/skill:",
        App::Codex | App::Chatgpt => "$",
        _ => "/",
    };
    format!("{call}{SKILL_NAME} Show me what this skill does.")
}

/// Writes the sample skill into a sibling folder and renames it into `root`,
/// replacing any earlier copy: VS Code watches `root` alone, so it would miss
/// a SKILL.md written into a folder it has already seen appear.
fn write_skill(root: &Path, text: &str) -> Result<(), String> {
    let skill = root.join(SKILL_NAME);
    let staging = crate::sources::temporary_path(root, SKILL_NAME);
    let written = fs_retry::create_dir_all(&staging)
        .and_then(|()| fs_retry::write_synced(&staging.join("SKILL.md"), text.as_bytes()))
        .and_then(|()| remove_if_present(&skill))
        .and_then(|()| fs_retry::rename(&staging, &skill));
    if written.is_err() {
        let _ = fs_retry::remove_dir_all(&staging);
    }
    written.map_err(|error| failed("add the tutorial skill to", &skill, &error))
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs_retry::remove_dir_all(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn failed(action: &str, path: &Path, error: &io::Error) -> String {
    format!(
        "Could not {action} {}: {}",
        path.display(),
        fs_retry::plain(error)
    )
}

fn create_prompt(skills: &Path, cli: &Path) -> String {
    let cli = cli.display();
    let cli = if cfg!(windows) {
        format!("\"{cli}\"; in PowerShell, call it as & \"{cli}\"")
    } else {
        format!("'{cli}'")
    };
    let skill = skills.join("<namespace>-<name>").join("SKILL.md");
    CREATE_PROMPT
        .replace("{cli}", &cli)
        .replace("{skill}", &skill.display().to_string())
}

/// The Agent Plugins command line: on Windows the console twin the installer
/// puts beside the app, since the app itself prints nothing to a shell.
fn command_line() -> Result<PathBuf, String> {
    let app = std::env::current_exe()
        .map_err(|error| format!("Could not find Agent Plugins itself: {error}"))?;
    Ok(if cfg!(windows) {
        app.with_file_name("agent-plugins.com")
    } else {
        app
    })
}

/// What the person does once `app` opens with the message.
fn next_step(app: App) -> String {
    let name = label(app);
    match app {
        App::Cursor => "Cursor is opening. Choose Create Chat, then send the message.".to_string(),
        App::Vscode => {
            "VS Code is opening a new window and sending the message to GitHub Copilot.".to_string()
        }
        App::VscodeInsiders => "VS Code Insiders is opening a new window and sending the message to GitHub Copilot.".to_string(),
        App::ClaudeDesktop => "Claude Desktop is opening a new Code session with the message filled in. Press Enter to send it.".to_string(),
        App::Chatgpt => "ChatGPT is opening a new Codex chat with the message filled in. Press Enter to send it.".to_string(),
        App::ClaudeCode | App::CopilotCli => format!("A terminal window is opening with {name}. If it asks whether you trust the folder, choose Yes, and it sends the message."),
        App::Codex => format!("A terminal window is opening with {name}. If it asks whether you trust the folder, choose Yes; if the message still waits after that, press Enter."),
        App::OpenCode | App::Pi | App::GrokBuild => {
            format!("A terminal window is opening with {name}, which sends the message for you.")
        }
    }
}

fn open(app: App, program: &Path, prompt: &str, paths: &SystemPaths) -> Result<(), String> {
    launch(app, program, prompt, paths).map_err(|error| {
        format!(
            "Could not open {} from {}: {error}",
            label(app),
            program.display()
        )
    })
}

/// Opens `app` with `prompt` the way that app takes one, working in home.
fn launch(app: App, program: &Path, prompt: &str, paths: &SystemPaths) -> Result<(), String> {
    let home = &paths.home;
    let home_text = home.to_string_lossy();
    let text = encode(prompt);
    match app {
        // Cursor has no command-line option for a prompt; this is how it opens
        // its own links. It asks to create the chat, then waits for Send.
        App::Cursor => {
            let link = format!("cursor://anysphere.cursor-deeplink/prompt?text={text}");
            if cfg!(target_os = "macos") {
                let mut open = Command::new("/usr/bin/open");
                open.arg("-a").arg(program).arg(&link);
                hand_over(open, home)
            } else {
                let mut cursor = Command::new(program);
                cursor.args(["--open-url", "--", &link]);
                start(cursor, home)
            }
        }
        // `chat` sends the prompt at once, but only when VSCODE_CLI says the
        // code command started it; `-n` opens an empty window instead of this
        // process's folder. The prompt goes last: Electron refuses arguments
        // after one that looks like a link.
        App::Vscode | App::VscodeInsiders => {
            let mut code = Command::new(program);
            code.args(["chat", "-n", "-m", "agent", prompt])
                .env("VSCODE_CLI", "1")
                .env("ELECTRON_NO_ATTACH_CONSOLE", "1")
                .env_remove("VSCODE_DEV");
            start(code, home)
        }
        // Not through its own `--handle-uri` link launcher: started from a
        // window without a console, it falls back to a PowerShell that never
        // shows on Windows, and on macOS it asks to control Terminal.
        App::ClaudeCode => in_terminal(paths, program, &[prompt]),
        // A new session in the Code tab, which reads skills from disk. A
        // folder in the link would ask for trust every time.
        App::ClaudeDesktop => {
            let link = format!("claude://code/new?q={text}");
            if cfg!(target_os = "macos") {
                let mut open = Command::new("/usr/bin/open");
                open.args(["-b", "com.anthropic.claudefordesktop", &link]);
                hand_over(open, home)
            } else {
                let mut claude = Command::new(program);
                claude.arg(&link);
                start(claude, home).or_else(|_| open_link(&link))
            }
        }
        App::Chatgpt => open_link(&format!(
            "codex://threads/new?prompt={text}&path={}",
            encode(&home_text)
        )),
        App::CopilotCli => in_terminal(paths, program, &["-i", prompt]),
        App::Codex => in_terminal(paths, program, &["-C", &home_text, prompt]),
        App::OpenCode => in_terminal(paths, program, &["--prompt", prompt]),
        App::Pi => in_terminal(paths, program, &[prompt]),
        App::GrokBuild => in_terminal(paths, program, &["--cwd", &home_text, prompt]),
    }
}

/// Percent-encodes `text` for a link, with spaces as %20: `byte_serialize`
/// writes `+`, which not every app decodes.
fn encode(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

/// Starts `command` on its own, in home, with none of this app's input or
/// output, and without ELECTRON_RUN_AS_NODE, which makes an Electron app run
/// as plain Node.js. It is set when Agent Plugins itself was started from an
/// Electron app's terminal.
fn start(mut command: Command, home: &Path) -> Result<(), String> {
    command
        .current_dir(home)
        .env_remove("ELECTRON_RUN_AS_NODE")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    // Waited on in the background, so an app that quits leaves no zombie behind.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Runs a launcher that passes the prompt on and exits, such as `open`, and
/// reports what it printed when it fails.
fn hand_over(mut command: Command, home: &Path) -> Result<(), String> {
    command.current_dir(home).env_remove("ELECTRON_RUN_AS_NODE");
    let output = crate::process::run(command, "Opening the app", HAND_OVER_TIMEOUT)?;
    if output.status.success() {
        return Ok(());
    }
    let printed = String::from_utf8_lossy(&output.stderr);
    Err(printed
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("it stopped with an error")
        .to_string())
}

/// Opens `link` with the app registered for it.
#[cfg(feature = "app")]
fn open_link(link: &str) -> Result<(), String> {
    tauri_plugin_opener::open_url(link, None::<&str>).map_err(|error| error.to_string())
}

#[cfg(not(feature = "app"))]
fn open_link(_link: &str) -> Result<(), String> {
    Err("Only the Agent Plugins window opens links.".to_string())
}

/// Runs `program args` in a new console window: Windows Terminal when it is
/// the default terminal, a classic console otherwise. cmd.exe runs npm's
/// `.cmd` shims as well as programs, and keeps the window open on a failure
/// so the error can be read. Nothing is inherited: this app's stderr is its
/// log file, which would swallow the program's errors.
#[cfg(windows)]
fn in_terminal(paths: &SystemPaths, program: &Path, args: &[&str]) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, CREATE_NEW_CONSOLE, CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION,
        STARTUPINFOW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
    let wide = |text: &OsStr| text.encode_wide().chain([0]).collect::<Vec<u16>>();
    let system = std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    let cmd = wide(Path::new(&system).join(r"System32\cmd.exe").as_os_str());
    // `/s` takes off the outer quotes and runs the rest as typed.
    let line = format!(
        "cmd.exe /d /v:off /s /c \"{} || pause\"",
        windows_command(program, args)
    );
    let mut line = wide(OsStr::new(&line));
    // This app's environment, in Windows' order, without ELECTRON_RUN_AS_NODE.
    let mut environment = Vec::new();
    for (key, value) in std::env::vars_os() {
        if !key.eq_ignore_ascii_case("ELECTRON_RUN_AS_NODE") {
            environment.extend(
                key.encode_wide()
                    .chain([u16::from(b'=')])
                    .chain(value.encode_wide())
                    .chain([0]),
            );
        }
    }
    environment.push(0);
    let home = wide(paths.home.as_os_str());
    // SAFETY: plain C structs, valid when zeroed; `cb` is set below.
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    // SAFETY: as above.
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: plain call. The person just clicked here, so this app may let
    // the new window come to the front.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    // SAFETY: every pointer is to a live buffer owned above, NUL-terminated
    // where Windows expects it, and `line` is mutable as CreateProcessW
    // requires. With no handles inherited, the new console supplies all three
    // standard ones.
    let started = unsafe {
        CreateProcessW(
            cmd.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NEW_CONSOLE | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            home.as_ptr(),
            &startup,
            &mut process,
        )
    };
    if started == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    // SAFETY: handles CreateProcessW returned, owned here, each closed once.
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

/// `program args` as one line that cmd.exe and the Microsoft C runtime read
/// back unchanged. Everything is quoted, so cmd.exe leaves a `&` in a path
/// alone; no argument has a quote of its own, since Windows paths can't and
/// the prompts are Agent Plugins' own.
#[cfg(any(windows, test))]
fn windows_command(program: &Path, args: &[&str]) -> String {
    std::iter::once(program.to_string_lossy().as_ref())
        .chain(args.iter().copied())
        .map(quote_windows_arg)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quotes `arg` the way the Microsoft C runtime undoes: backslashes are
/// literal except right before a quote, where they are doubled.
#[cfg(any(windows, test))]
fn quote_windows_arg(arg: &str) -> String {
    let mut quoted = String::from('"');
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslashes + 1));
                backslashes = 0;
            }
            _ => backslashes = 0,
        }
        quoted.push(c);
    }
    quoted.extend(std::iter::repeat_n('\\', backslashes));
    quoted.push('"');
    quoted
}

/// Runs `program args` in a new terminal window through a script, which
/// keeps the window open on a failure so the error can be read. macOS opens
/// the script as a Terminal document, which needs no permission to control
/// Terminal; Linux hands it to the person's terminal.
#[cfg(unix)]
fn in_terminal(paths: &SystemPaths, program: &Path, args: &[&str]) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    let name = program.file_stem().unwrap_or_default().to_string_lossy();
    let script = paths.app_data().join(format!("open-{name}.command"));
    fs_retry::create_dir_all(&paths.app_data())
        .and_then(|()| {
            fs_retry::replace_file(
                &script,
                terminal_script(&paths.home, program, args).as_bytes(),
            )
        })
        .and_then(|()| std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)))
        .map_err(|error| failed("write", &script, &error))?;
    if cfg!(target_os = "macos") {
        let mut open = Command::new("/usr/bin/open");
        open.args(["-b", "com.apple.Terminal"]).arg(&script);
        return hand_over(open, &paths.home);
    }
    let (terminal, option) = linux_terminal().ok_or(
        "Could not find a terminal app. Set the TERMINAL environment variable to yours, then try again.",
    )?;
    let mut command = Command::new(terminal);
    command.arg(option).arg("/bin/sh").arg(&script);
    start(command, &paths.home)
}

/// The person's terminal and the option after which it takes a command:
/// TERMINAL when set, then the common ones.
#[cfg(unix)]
fn linux_terminal() -> Option<(PathBuf, &'static str)> {
    let chosen = std::env::var("TERMINAL")
        .ok()
        .and_then(|name| crate::startup::find_program(&name));
    if let Some(terminal) = chosen {
        return Some((terminal, "-e"));
    }
    [
        ("x-terminal-emulator", "-e"),
        ("gnome-terminal", "--"),
        ("ptyxis", "--"),
        ("konsole", "-e"),
        ("xfce4-terminal", "-x"),
        ("kitty", "-e"),
        ("alacritty", "-e"),
        ("xterm", "-e"),
    ]
    .into_iter()
    .find_map(|(name, option)| Some((crate::startup::find_program(name)?, option)))
}

#[cfg(any(unix, test))]
fn terminal_script(home: &Path, program: &Path, args: &[&str]) -> String {
    let command = std::iter::once(program.to_string_lossy().as_ref())
        .chain(args.iter().copied())
        .map(sh_quote)
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "#!/bin/sh\ncd {} || exit 1\n{command} || {{ printf '\\nPress Enter to close.'; read -r _; }}\n",
        sh_quote(&home.to_string_lossy())
    )
}

/// Single quotes keep every character as it is; a quote itself becomes `'\''`.
#[cfg(any(unix, test))]
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_paths(root: &Path) -> SystemPaths {
        SystemPaths {
            home: root.join("home"),
            config: root.join("config"),
            data: root.join("data"),
            local_data: root.join("local-data"),
            cache: root.join("cache"),
        }
    }

    #[test]
    fn tutorial_prompt_uses_each_apps_skill_syntax() {
        assert_eq!(
            tutorial_prompt(App::Cursor),
            "/agent-plugins-tutorial Show me what this skill does."
        );
        assert_eq!(
            tutorial_prompt(App::Pi),
            "/skill:agent-plugins-tutorial Show me what this skill does."
        );
        assert_eq!(
            tutorial_prompt(App::Codex),
            "$agent-plugins-tutorial Show me what this skill does."
        );
        assert_eq!(tutorial_prompt(App::Chatgpt), tutorial_prompt(App::Codex));
    }

    #[test]
    fn encode_writes_spaces_as_percent_20() {
        assert_eq!(encode("/skill a+b & c"), "%2Fskill%20a%2Bb%20%26%20c");
    }

    #[test]
    fn create_prompt_fills_in_every_placeholder() {
        let skills = Path::new("home").join(".agents").join("skills");
        let prompt = create_prompt(&skills, Path::new("agent-plugins"));
        assert!(!prompt.contains('{') && !prompt.contains('}'), "{prompt}");
        let skill = skills.join("<namespace>-<name>").join("SKILL.md");
        assert!(prompt.contains(&skill.display().to_string()));
        assert!(!prompt.contains("Cursor"));
    }

    /// How the Microsoft C runtime splits a command line.
    fn split_windows_command(line: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut chars = line.chars().peekable();
        while chars.peek().is_some() {
            let (mut arg, mut quoted) = (String::new(), false);
            while let Some(c) = chars.next() {
                match c {
                    '\\' => {
                        let mut count = 1;
                        while chars.next_if_eq(&'\\').is_some() {
                            count += 1;
                        }
                        if chars.peek() == Some(&'"') {
                            arg.extend(std::iter::repeat_n('\\', count / 2));
                            if count % 2 == 1 {
                                arg.push(chars.next().unwrap_or('"'));
                            }
                        } else {
                            arg.extend(std::iter::repeat_n('\\', count));
                        }
                    }
                    '"' => quoted = !quoted,
                    ' ' if !quoted => break,
                    _ => arg.push(c),
                }
            }
            args.push(arg);
        }
        args
    }

    #[test]
    fn windows_command_round_trips_through_the_c_runtime() {
        let args = [
            "-C",
            r"C:\Users\Sam Lee",
            r"ends in a backslash\",
            r#"says "hi" \"there\""#,
            "",
            "$agent-plugins-tutorial Show me what this skill does.",
        ];
        let program = Path::new(r"C:\Program Files\A & B\codex.cmd");
        let line = windows_command(program, &args);
        let mut expected = vec![program.to_string_lossy().into_owned()];
        expected.extend(args.iter().map(|arg| arg.to_string()));
        assert_eq!(split_windows_command(&line), expected, "{line}");
    }

    #[test]
    fn terminal_script_quotes_for_sh() {
        let script = terminal_script(
            Path::new("/home/o'brien"),
            Path::new("/usr/bin/pi"),
            &["/skill:agent-plugins-tutorial Show me what this skill does."],
        );
        assert!(
            script.contains(r"cd '/home/o'\''brien' || exit 1"),
            "{script}"
        );
        assert!(script.contains(
            "'/usr/bin/pi' '/skill:agent-plugins-tutorial Show me what this skill does.' ||"
        ));
    }

    #[test]
    fn write_skill_replaces_an_earlier_copy_whole() {
        let root = tempfile::tempdir().expect("tempdir");
        let skills = root.path().join("skills");
        let old = skills.join(SKILL_NAME);
        std::fs::create_dir_all(&old).expect("old skill");
        std::fs::write(old.join("notes.txt"), "old").expect("old file");
        write_skill(&skills, SKILL).expect("write");
        assert_eq!(
            std::fs::read_to_string(old.join("SKILL.md")).expect("skill"),
            SKILL
        );
        assert!(!old.join("notes.txt").exists());
        assert_eq!(std::fs::read_dir(&skills).expect("skills").count(), 1);
    }

    #[test]
    fn remove_skill_deletes_the_sample_skill_and_tolerates_its_absence() {
        let root = tempfile::tempdir().expect("tempdir");
        let paths = temp_paths(root.path());
        let skills = [
            paths.home.join(".agents").join("skills").join(SKILL_NAME),
            paths.home.join(".claude").join("skills").join(SKILL_NAME),
        ];
        for skill in &skills {
            std::fs::create_dir_all(skill).expect("skill dir");
            std::fs::write(skill.join("SKILL.md"), SKILL).expect("skill");
        }
        remove_skill(&paths).expect("remove");
        assert!(skills.iter().all(|skill| !skill.exists()));
        remove_skill(&paths).expect("already gone");
    }
}
