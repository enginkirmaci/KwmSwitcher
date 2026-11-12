//! Autostart setup.
//!
//! Two mechanisms, kept in sync:
//! - a FreeDesktop `.desktop` file in the user's autostart dir (GNOME, KDE,
//!   or Hyprland sessions that run `dex`);
//! - on Omarchy, a managed block in `~/.config/hypr/autostart.lua`, because
//!   Omarchy's Hyprland session never processes XDG autostart (no `dex`).

use std::path::{Path, PathBuf};

const MARK_BEGIN: &str = "-- BEGIN KwmSwitcher (managed)";
const MARK_END: &str = "-- END KwmSwitcher (managed)";
const SUPERVISE_ARG: &str = "--supervise";

fn desktop_file_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("autostart")
        .join("KwmSwitcher.desktop")
}

/// Quotes a path for a Desktop Entry `Exec=` value: double quotes around the
/// argument, with the spec's reserved characters escaped, and `%` doubled so
/// it is not read as a field code.
fn desktop_exec_quote(path: &str) -> String {
    let mut quoted = String::with_capacity(path.len() + 2);
    quoted.push('"');
    for ch in path.chars() {
        match ch {
            '"' | '\\' | '$' | '`' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            '%' => quoted.push_str("%%"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

fn desktop_exec_line(exec_path: &str) -> String {
    format!("Exec={} {SUPERVISE_ARG}", desktop_exec_quote(exec_path))
}

fn desktop_file_content(exec_path: &str) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=KWM Switcher\n\
         {}\n\
         Icon=KwmSwitcher\n\
         Comment=USB KVM switcher for monitor input\n\
         Hidden=false\n\
         NoDisplay=false\n\
         X-GNOME-Autostart-enabled=true\n",
        desktop_exec_line(exec_path)
    )
}

/// `~/.config/hypr/autostart.lua` when running on Omarchy.
fn omarchy_autostart_lua() -> Option<PathBuf> {
    let omarchy_path = std::env::var_os("OMARCHY_PATH")?;
    if omarchy_path.is_empty() {
        return None;
    }
    Some(dirs::config_dir()?.join("hypr").join("autostart.lua"))
}

/// Single-quotes a value for a POSIX shell when it needs it.
fn shell_quote(value: &str) -> String {
    let plain = !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "-_./=:@+,%".contains(ch));
    if plain {
        return value.to_string();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(ch);
        }
    }
    quoted.push('\'');
    quoted
}

/// Escapes a value as a Lua double-quoted string literal.
fn lua_quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        match ch {
            '\\' | '"' => {
                quoted.push('\\');
                quoted.push(ch);
            }
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

fn hypr_autostart_block(exec_path: &str) -> String {
    let command = format!("{} {SUPERVISE_ARG}", shell_quote(exec_path));
    format!(
        "{MARK_BEGIN}\no.launch_on_start({})\n{MARK_END}\n",
        lua_quote(&command)
    )
}

/// Adds or removes the managed block, leaving all other lines untouched.
fn apply_block(content: &str, enable: bool, exec_path: &str) -> String {
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
    if enable {
        out.push_str(&hypr_autostart_block(exec_path));
    }
    out
}

fn write_file(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)
}

/// Adds or removes the managed block in the Omarchy autostart file.
fn update_omarchy_block(enable: bool) -> std::io::Result<()> {
    let Some(path) = omarchy_autostart_lua() else {
        return Ok(());
    };
    // Distinguish "missing" (start from empty) from "unreadable" (leave it
    // alone): treating an unreadable file as empty would replace the user's
    // whole autostart file with just our block.
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(err),
    };
    let updated = apply_block(&content, enable, &current_exe_path());
    if updated == content {
        return Ok(());
    }
    write_file(&path, &updated)
}

fn omarchy_block_present() -> bool {
    omarchy_autostart_lua()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .is_some_and(|content| content.contains(MARK_BEGIN))
}

/// True when an entry exists but its Exec line no longer matches the running
/// binary (e.g. after the checkout or home directory moved).
fn entry_is_stale() -> bool {
    let Ok(content) = std::fs::read_to_string(desktop_file_path()) else {
        return false;
    };
    let expected = desktop_exec_line(&current_exe_path());
    !content.lines().any(|line| line.trim() == expected)
}

/// Canonical path of the running executable (for the `Exec=` line). The
/// checkout lives behind a symlinked Developments dir on this machine, so
/// resolve the prefix — otherwise entries flip-flop between path spellings.
pub fn current_exe_path() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::canonicalize(path).ok())
        .and_then(|path| path.to_str().map(str::to_string))
        .unwrap_or_else(|| "kwmswitcher".to_string())
}

pub fn is_enabled() -> bool {
    desktop_file_path().exists() || omarchy_block_present()
}

/// Writes the autostart entry (and the Omarchy block) and reports the first
/// I/O failure instead of only logging it, so the settings window can show it.
pub fn enable() -> std::io::Result<()> {
    write_file(
        &desktop_file_path(),
        &desktop_file_content(&current_exe_path()),
    )?;
    update_omarchy_block(true)
}

pub fn disable() -> std::io::Result<()> {
    match std::fs::remove_file(desktop_file_path()) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    update_omarchy_block(false)
}

pub fn set_enabled(enabled: bool) -> std::io::Result<()> {
    if enabled {
        enable()
    } else {
        disable()
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
        if let Err(err) = enable() {
            log::error!("Failed to reconcile autostart: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_exec_quotes_and_escapes() {
        assert_eq!(
            desktop_exec_quote("/usr/bin/kwmswitcher"),
            "\"/usr/bin/kwmswitcher\""
        );
        assert_eq!(
            desktop_exec_quote("/home/me/My Apps/kwmswitcher"),
            "\"/home/me/My Apps/kwmswitcher\""
        );
        // `%` is a field code in Exec values and must be doubled.
        assert_eq!(desktop_exec_quote("/tmp/100%/app"), "\"/tmp/100%%/app\"");
        assert_eq!(desktop_exec_quote("/tmp/a\"b"), "\"/tmp/a\\\"b\"");
    }

    #[test]
    fn desktop_entry_uses_the_escaped_exec_line() {
        let content = desktop_file_content("/home/me/My Apps/kwmswitcher");
        assert!(content.contains("Exec=\"/home/me/My Apps/kwmswitcher\" --supervise"));
        assert!(content.starts_with("[Desktop Entry]\n"));
    }

    #[test]
    fn shell_quote_covers_spaces_and_quotes() {
        assert_eq!(shell_quote("/usr/bin/app"), "/usr/bin/app");
        assert_eq!(
            shell_quote("/home/me/My Apps/app"),
            "'/home/me/My Apps/app'"
        );
        assert_eq!(shell_quote("/tmp/it's"), "'/tmp/it'\\''s'");
    }

    #[test]
    fn lua_block_quotes_the_command() {
        let block = hypr_autostart_block("/home/me/My Apps/kwmswitcher");
        assert!(block.starts_with(MARK_BEGIN));
        assert!(block.ends_with(&format!("{MARK_END}\n")));
        assert!(block.contains("o.launch_on_start(\"'/home/me/My Apps/kwmswitcher' --supervise\")"));
    }

    #[test]
    fn apply_block_adds_removes_and_keeps_other_lines() {
        let original = "-- header\no.launch_on_start(\"other\")\n";
        let added = apply_block(original, true, "/usr/bin/kwmswitcher");
        assert!(added.contains("o.launch_on_start(\"other\")"));
        assert!(added.contains(MARK_BEGIN));
        assert!(added.contains("o.launch_on_start(\"/usr/bin/kwmswitcher --supervise\")"));

        let removed = apply_block(&added, false, "/usr/bin/kwmswitcher");
        assert!(!removed.contains(MARK_BEGIN));
        assert!(!removed.contains(MARK_END));
        assert!(removed.contains("o.launch_on_start(\"other\")"));

        // Idempotent: enabling twice keeps one block.
        let added_twice = apply_block(&added, true, "/usr/bin/kwmswitcher");
        assert_eq!(added_twice.matches(MARK_BEGIN).count(), 1);
        assert_eq!(added, added_twice);
    }
}
