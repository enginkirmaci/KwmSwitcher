//! Autostart setup.
//!
//! Two mechanisms, kept in sync:
//! - a FreeDesktop `.desktop` file in the user's autostart dir (GNOME, KDE,
//!   or Hyprland sessions that run `dex`);
//! - on Omarchy, a managed block in `~/.config/hypr/autostart.lua`, because
//!   Omarchy's Hyprland session never processes XDG autostart (no `dex`).

use std::path::PathBuf;

const MARK_BEGIN: &str = "-- BEGIN KwmSwitcher (managed)";
const MARK_END: &str = "-- END KwmSwitcher (managed)";

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

/// `~/.config/hypr/autostart.lua` when running on Omarchy.
fn omarchy_autostart_lua() -> Option<PathBuf> {
    std::env::var_os("OMARCHY_PATH")?;
    Some(
        dirs::config_dir()?
            .join("hypr")
            .join("autostart.lua"),
    )
}

fn hypr_autostart_block(exec_path: &str) -> String {
    format!("{MARK_BEGIN}\no.launch_on_start(\"{exec_path} --supervise\")\n{MARK_END}\n")
}

/// Adds or removes the managed block, leaving all other lines untouched.
fn update_omarchy_block(enable: bool) {
    let Some(path) = omarchy_autostart_lua() else { return };
    let stripped = match std::fs::read_to_string(&path) {
        Ok(content) => {
            let mut out = String::with_capacity(content.len());
            let mut in_block = false;
            for line in content.lines() {
                match line.trim() {
                    MARK_BEGIN => in_block = true,
                    MARK_END => in_block = false,
                    _ if !in_block => {
                        out.push_str(line);
                        out.push('\n');
                    }
                    _ => {}
                }
            }
            Some(out)
        }
        // Missing file: enabling creates a fresh one; disabling is done.
        Err(_) if enable => Some(String::new()),
        Err(_) => return,
    };
    let mut content = stripped.unwrap_or_default();
    if enable {
        content.push_str(&hypr_autostart_block(&current_exe_path()));
    }
    let result = std::fs::create_dir_all(path.parent().unwrap())
        .and_then(|_| std::fs::write(&path, content));
    if let Err(err) = result {
        log::error!("Failed to update Omarchy autostart: {err}");
    }
}

fn omarchy_block_present() -> bool {
    omarchy_autostart_lua()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|c| c.contains(MARK_BEGIN))
}

/// True when an entry exists but its Exec path no longer matches the running
/// binary (e.g. after the checkout or home directory moved).
fn entry_is_stale() -> bool {
    let Ok(content) = std::fs::read_to_string(desktop_file_path()) else {
        return false;
    };
    !content.contains(&current_exe_path())
}

/// Canonical path of the running executable (for the `Exec=` line). The
/// checkout lives behind a symlinked Developments dir on this machine, so
/// resolve the prefix — otherwise entries flip-flop between path spellings.
pub fn current_exe_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::canonicalize(p).ok())
        .and_then(|p| p.to_str().map(str::to_string))
        .unwrap_or_else(|| "kwmswitcher".to_string())
}

pub fn is_enabled() -> bool {
    desktop_file_path().exists() || omarchy_block_present()
}

pub fn enable() {
    let result = std::fs::create_dir_all(desktop_file_path().parent().unwrap())
        .and_then(|_| std::fs::write(desktop_file_path(), desktop_file_content(&current_exe_path())));
    if let Err(err) = result {
        log::error!("Failed to enable autostart: {err}");
    }
    update_omarchy_block(true);
}

pub fn disable() {
    match std::fs::remove_file(desktop_file_path()) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => log::error!("Failed to disable autostart: {err}"),
    }
    update_omarchy_block(false);
}

pub fn set_enabled(enabled: bool) {
    if enabled {
        enable();
    } else {
        disable();
    }
}

/// Applies the cached flag at startup, rewriting entries that are missing,
/// stale, or predate the Omarchy mechanism.
pub fn reconcile(config: &crate::config::AppConfig) {
    if !config.auto_start {
        return;
    }
    let omarchy_entry_missing = omarchy_autostart_lua().is_some() && !omarchy_block_present();
    if !is_enabled() || entry_is_stale() || omarchy_entry_missing {
        enable();
    }
}
