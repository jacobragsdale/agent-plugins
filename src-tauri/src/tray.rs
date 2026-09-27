use std::error::Error;
use std::ffi::OsStr;

#[cfg(windows)]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    App, AppHandle, Manager, Runtime,
};
use tauri_plugin_autostart::ManagerExt;

pub(crate) const BACKGROUND_ARG: &str = "--background";

const TRAY_ID: &str = "agent-plugins";
const OPEN_MENU_ID: &str = "open";
const CHECK_NOW_MENU_ID: &str = "check-now";
const LAUNCH_AT_LOGIN_MENU_ID: &str = "launch-at-login";
const QUIT_MENU_ID: &str = "quit";

fn has_background_arg<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .any(|argument| argument.as_ref() == OsStr::new(BACKGROUND_ARG))
}

pub(crate) fn is_background_launch() -> bool {
    has_background_arg(std::env::args_os())
}

fn show_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<bool> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(false);
    };

    window.show()?;
    window.unminimize()?;
    window.set_focus()?;
    Ok(true)
}

/// Shows the main window and logs, instead of failing, when it cannot.
pub(crate) fn open_main_window<R: Runtime>(app: &AppHandle<R>) {
    match show_main_window(app) {
        Ok(true) => {}
        Ok(false) => {
            eprintln!("Could not open Agent Plugins because its main window is unavailable.");
        }
        Err(error) => eprintln!("Could not open Agent Plugins: {error}"),
    }
}

/// The first time the window closes, says the app keeps running, since
/// updates and removals reach this PC only while it does.
pub(crate) fn explain_first_close<R: Runtime>(app: &AppHandle<R>) {
    let Ok(paths) = crate::paths::SystemPaths::from_system() else {
        return;
    };
    let marker = paths.app_data().join("explained-tray");
    if marker.exists() {
        return;
    }
    crate::notify::show(
        app,
        "Agent Plugins is still running in the system tray, so your skills stay up to date. Quit it from the tray icon.",
    );
    let _ = std::fs::create_dir_all(paths.app_data());
    let _ = std::fs::write(marker, b"");
}

/// Launch at Login starts on: updates and removals reach this PC only while
/// the app runs. The `LaunchAtLogin` policy decides for everyone when IT sets
/// it; otherwise the person's own choice in the tray menu stands after the
/// first run.
fn apply_launch_at_login_default<R: Runtime>(app: &App<R>) {
    if cfg!(debug_assertions) {
        return;
    }
    #[cfg(windows)]
    let policy = crate::locator::policy_value::<u32>("LaunchAtLogin").map(|value| value != 0);
    #[cfg(not(windows))]
    let policy: Option<bool> = None;
    let Ok(paths) = crate::paths::SystemPaths::from_system() else {
        return;
    };
    let marker = paths.app_data().join("launch-at-login-default");
    let wanted = policy.or_else(|| (!marker.exists()).then_some(true));
    let autolaunch = app.autolaunch();
    let result = match wanted {
        Some(true) if !autolaunch.is_enabled().unwrap_or(false) => autolaunch.enable(),
        Some(false) if autolaunch.is_enabled().unwrap_or(false) => autolaunch.disable(),
        _ => Ok(()),
    };
    if let Err(error) = result {
        // Not applied, so not remembered: the next start tries again.
        eprintln!("Could not set Launch at Login: {error}");
        return;
    }
    if policy.is_none() {
        let _ = std::fs::create_dir_all(paths.app_data());
        let _ = std::fs::write(marker, b"");
    }
}

fn toggle_launch_at_login<R: Runtime>(
    app: &AppHandle<R>,
    item: &CheckMenuItem<R>,
) -> Result<(), String> {
    let autolaunch = app.autolaunch();
    let was_enabled = autolaunch
        .is_enabled()
        .map_err(|error| format!("Could not read the launch-at-login setting: {error}"))?;
    let result = if was_enabled {
        autolaunch.disable()
    } else {
        autolaunch.enable()
    };

    match result {
        Ok(()) => {
            if !was_enabled {
                quote_launch_entry(app);
            }
            item.set_checked(!was_enabled).map_err(|error| {
                format!("Launch at login changed, but the tray menu could not be updated: {error}")
            })
        }
        Err(error) => {
            if let Err(menu_error) = item.set_checked(was_enabled) {
                eprintln!(
                    "Could not restore the Launch at Login menu state after an autostart error: {menu_error}"
                );
            }
            Err(format!(
                "Could not change the launch-at-login setting: {error}"
            ))
        }
    }
}

/// `auto-launch` writes the Run entry as `<exe> --background` without quotes,
/// and the per-user install folder is `Agent Plugins`, with a space. Windows
/// then has to guess where the program name ends, and a stray `Agent.exe`
/// would win. Rewrites the entry quoted, pointing at this copy.
#[cfg(windows)]
fn quote_launch_entry<R: Runtime>(app: &AppHandle<R>) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    let name = &app.package_info().name;
    let Ok(run) = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        KEY_READ | KEY_SET_VALUE,
    ) else {
        return;
    };
    let (Ok(current), Ok(exe)) = (run.get_value::<String, _>(name), std::env::current_exe()) else {
        return;
    };
    let quoted = format!("\"{}\" {BACKGROUND_ARG}", exe.display());
    if current != quoted {
        if let Err(error) = run.set_value(name, &quoted) {
            eprintln!("Could not quote the launch-at-login entry: {error}");
        }
    }
}

#[cfg(not(windows))]
fn quote_launch_entry<R: Runtime>(_app: &AppHandle<R>) {}

pub(crate) fn setup<R: Runtime>(app: &mut App<R>) -> Result<(), Box<dyn Error>> {
    let open_item = MenuItem::with_id(app, OPEN_MENU_ID, "Open Agent Plugins", true, None::<&str>)?;
    let check_now_item = MenuItem::with_id(
        app,
        CHECK_NOW_MENU_ID,
        "Check for Updates Now",
        true,
        None::<&str>,
    )?;
    apply_launch_at_login_default(app);
    let launch_at_login_enabled = match app.autolaunch().is_enabled() {
        Ok(enabled) => {
            if enabled {
                quote_launch_entry(app.handle());
            }
            enabled
        }
        Err(error) => {
            eprintln!("Could not read the launch-at-login setting: {error}");
            false
        }
    };
    let launch_at_login_item = CheckMenuItem::with_id(
        app,
        LAUNCH_AT_LOGIN_MENU_ID,
        "Launch at Login",
        true,
        launch_at_login_enabled,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(app, QUIT_MENU_ID, "Quit Agent Plugins", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &open_item,
            &check_now_item,
            &launch_at_login_item,
            &separator,
            &quit_item,
        ],
    )?;
    // macOS renders template images from their alpha channel alone, so the
    // full-bleed app icon would show up as a solid rounded square. Use the
    // dedicated silhouette instead. Other platforms draw the icon in colour.
    #[cfg(target_os = "macos")]
    let icon = tauri::include_image!("icons/tray.png");
    #[cfg(not(target_os = "macos"))]
    let icon = app.default_window_icon().cloned().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Agent Plugins has no application icon for the system tray.",
        )
    })?;
    let launch_item_for_handler = launch_at_login_item.clone();

    let tray = TrayIconBuilder::<R>::with_id(TRAY_ID)
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Agent Plugins")
        .menu(&menu)
        // The macOS menu bar opens the menu from either button. The Windows
        // notification area expects the left button to open the application
        // and reserves the menu for the right button.
        .show_menu_on_left_click(cfg!(not(windows)))
        .on_menu_event(move |app, event| match event.id().as_ref() {
            OPEN_MENU_ID => open_main_window(app),
            // The window shows the check running and what it found; with it
            // hidden, the click would look like it did nothing.
            CHECK_NOW_MENU_ID => {
                open_main_window(app);
                crate::application::spawn_app_sync(app.clone());
            }
            LAUNCH_AT_LOGIN_MENU_ID => {
                if let Err(error) = toggle_launch_at_login(app, &launch_item_for_handler) {
                    eprintln!("{error}");
                }
            }
            QUIT_MENU_ID => app.exit(0),
            _ => {}
        });

    #[cfg(windows)]
    let tray = tray.on_tray_icon_event(|tray, event| {
        let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        else {
            return;
        };
        open_main_window(tray.app_handle());
    });

    tray.build(app)?;

    // The tray icon is up, so a window that cannot be shown now can still be
    // opened from it; failing setup here would end the app.
    if !is_background_launch() {
        open_main_window(app.handle());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_background_launch_argument() {
        assert!(has_background_arg(["agent-plugins", BACKGROUND_ARG]));
        assert!(has_background_arg([
            "agent-plugins",
            "--other",
            BACKGROUND_ARG
        ]));
    }

    #[test]
    fn ignores_other_launch_arguments() {
        assert!(!has_background_arg(["agent-plugins"]));
        assert!(!has_background_arg(["agent-plugins", "--background-task"]));
    }
}
