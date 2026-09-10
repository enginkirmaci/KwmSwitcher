//! XDG paths for config and log/state files.

use std::path::PathBuf;

fn config_dir() -> PathBuf {
    // Same location the Avalonia version used (.NET maps SpecialFolder.ApplicationData
    // to $XDG_CONFIG_HOME), so existing installs keep their settings.
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("KwmSwitcher")
}

fn state_dir() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(config_dir)
        .join("KwmSwitcher")
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

pub fn log_file() -> PathBuf {
    state_dir().join("kwmswitcher.log")
}

pub fn supervisor_log_file() -> PathBuf {
    state_dir().join("supervisor.log")
}

pub fn crash_log_file() -> PathBuf {
    state_dir().join("crash.log")
}

pub fn stderr_log_file() -> PathBuf {
    state_dir().join("stderr.log")
}
