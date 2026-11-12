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

pub fn lock_file() -> PathBuf {
    state_dir().join("kwmswitcher.lock")
}

/// Unix socket a later launch uses to ask the running instance to show its
/// window. The runtime dir is per-session and user-private; the state dir is
/// only a fallback for sessions without one.
pub fn ipc_socket() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(state_dir)
        .join("kwmswitcher.sock")
}

pub fn stderr_log_file() -> PathBuf {
    state_dir().join("stderr.log")
}
