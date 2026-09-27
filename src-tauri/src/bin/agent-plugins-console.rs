//! `agent-plugins.com`, the console twin installed beside `agent-plugins.exe`.
//!
//! Windows shells neither wait for a GUI program nor read its output, so
//! `agent-plugins whoami` from PowerShell or `cmd /c` printed nothing and
//! reported success at once. For a bare `agent-plugins`, shells try `.com`
//! before `.exe`, so commands land here instead: a console program they wait
//! for, which runs the app on this console and returns its exit code.

use std::process::{exit, Command};

fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    // The installer also ships this program as `agent-plugins-console.exe`, so
    // name the app itself: that name with `.exe` would start this program again.
    let app = match std::env::current_exe() {
        Ok(exe) => exe.with_file_name(format!("agent-plugins{}", std::env::consts::EXE_SUFFIX)),
        Err(error) => fail(&format!("Could not find Agent Plugins: {error}")),
    };
    let mut command = Command::new(&app);
    command.args(&args);
    // Opening the window, or starting in the tray, must not hold the terminal.
    let result = if args.iter().all(|arg| arg == "--background") {
        command.spawn().map(|_| 0)
    } else {
        command.status().map(|status| status.code().unwrap_or(1))
    };
    match result {
        Ok(code) => exit(code),
        Err(error) => fail(&format!("Could not start {}: {error}", app.display())),
    }
}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    exit(1)
}
