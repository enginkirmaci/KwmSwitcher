//! Autostart via a FreeDesktop `.desktop` file in the user's autostart dir.

use std::path::PathBuf;

fn desktop_file_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("autostart")
        .join("KwmSwitcher.desktop")
}

fn desktop_file_content(exec_path: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=KWM Switcher\n\
         Exec={exec_path} --supervise\n\
         Icon=KwmSwitcher\n\
         Comment=USB KVM switcher for monitor input\n\
         Hidden=false\n\
         NoDisplay=false\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// Path of the running executable (for the `Exec=` line).
pub fn current_exe_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_string))
        .unwrap_or_else(|| "kwmswitcher".to_string())
}

pub fn is_enabled() -> bool {
    desktop_file_path().exists()
}

pub fn enable() {
    let result = std::fs::create_dir_all(desktop_file_path().parent().unwrap())
        .and_then(|_| std::fs::write(desktop_file_path(), desktop_file_content(&current_exe_path())));
    if let Err(err) = result {
        log::error!("Failed to enable autostart: {err}");
    }
}

pub fn disable() {
    match std::fs::remove_file(desktop_file_path()) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => log::error!("Failed to disable autostart: {err}"),
    }
}

pub fn set_enabled(enabled: bool) {
    if enabled {
        enable();
    } else {
        disable();
    }
}

/// Applies the cached flag at startup if the .desktop file is missing but the
/// config claims autostart (e.g. after the executable moved).
pub fn reconcile(config: &crate::config::AppConfig) {
    if config.auto_start && !is_enabled() {
        enable();
    }
}
