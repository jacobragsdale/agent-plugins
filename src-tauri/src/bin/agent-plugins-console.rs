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
        command.spawn().and_then(|mut child| {
            end_with_this_process(&child);
            child.wait().map(|status| status.code().unwrap_or(1))
        })
    };
    match result {
        Ok(code) => exit(code),
        Err(error) => fail(&format!("Could not start {}: {error}", app.display())),
    }
}

/// A command stopped with Ctrl+Break or by closing its terminal kills this
/// program; the app it started goes with it instead of finishing a change
/// nobody is waiting for. Its journal is rolled back at the next start.
#[cfg(windows)]
fn end_with_this_process(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    // SAFETY: plain Win32 calls on a job handle this process owns and never
    // closes; Windows closes it, killing the job, when this process ends.
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return;
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let set = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if set != 0 {
            AssignProcessToJobObject(job, child.as_raw_handle());
        }
    }
}

#[cfg(not(windows))]
fn end_with_this_process(_child: &std::process::Child) {}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}");
    exit(1)
}
