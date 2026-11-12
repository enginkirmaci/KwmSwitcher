//! Persisted application settings, JSON-compatible with the previous Avalonia
//! version (`~/.config/KwmSwitcher/config.json`, PascalCase keys, protocol as
//! an integer), so existing installs migrate without touching their settings.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::paths;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InputSwitchProtocol {
    #[default]
    Standard,
    Lg,
}

impl InputSwitchProtocol {
    pub const fn as_u8(self) -> u8 {
        match self {
            InputSwitchProtocol::Standard => 0,
            InputSwitchProtocol::Lg => 1,
        }
    }

    pub const fn from_u8(v: u8) -> Self {
        match v {
            1 => InputSwitchProtocol::Lg,
            _ => InputSwitchProtocol::Standard,
        }
    }
}

// The C# app serialized the enum as a bare number; keep that representation.
impl Serialize for InputSwitchProtocol {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(self.as_u8())
    }
}

impl<'de> Deserialize<'de> for InputSwitchProtocol {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let v = u8::deserialize(deserializer)?;
        Ok(InputSwitchProtocol::from_u8(v))
    }
}

/// Accepts both a list and `null` (the C# version could persist a null list).
fn deserialize_keys_or_null<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct AppConfig {
    pub local_input_source: u8,
    pub remote_input_source: u8,
    pub input_protocol: InputSwitchProtocol,
    #[serde(default, deserialize_with = "deserialize_keys_or_null")]
    pub tracked_device_keys: Vec<String>,
    pub poll_interval_ms: u32,
    pub start_minimized: bool,
    /// Cached hint only; the OS (.desktop file) is the real source of truth.
    pub auto_start: bool,
    /// Target display as shown by `ddcutil detect` (`Display 2 — GBT3241`);
    /// `None` targets every detected monitor.
    pub target_monitor_name: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            local_input_source: crate::input_source::DISPLAY_PORT,
            remote_input_source: crate::input_source::HDMI1,
            input_protocol: InputSwitchProtocol::Standard,
            tracked_device_keys: Vec::new(),
            poll_interval_ms: 1000,
            start_minimized: true,
            auto_start: false,
            target_monitor_name: None,
        }
    }
}

/// Thread-safe handle to the live settings, shared by the UI, engine and DDC
/// thread. Cloning is cheap (one `Arc` bump) and every clone sees the same
/// settings.
#[derive(Clone, Debug)]
pub struct SharedConfig(Arc<RwLock<AppConfig>>);

impl SharedConfig {
    pub fn new(config: AppConfig) -> Self {
        Self(Arc::new(RwLock::new(config)))
    }

    /// Loads the settings from the default path.
    pub fn load() -> Self {
        Self::new(AppConfig::load())
    }

    /// # Panics
    /// Panics if a writer panicked while holding the lock. The panic hook
    /// terminates the process, so this is a "never happen" guard rather than
    /// a recoverable condition.
    pub fn read(&self) -> RwLockReadGuard<'_, AppConfig> {
        self.0.read().expect("config lock poisoned")
    }

    /// # Panics
    /// See [`SharedConfig::read`].
    pub fn write(&self) -> RwLockWriteGuard<'_, AppConfig> {
        self.0.write().expect("config lock poisoned")
    }

    /// Runs `f` on the settings under the write lock.
    pub fn update(&self, f: impl FnOnce(&mut AppConfig)) {
        f(&mut self.write());
    }

    /// A copy of the current settings, safe to hand to another thread.
    pub fn snapshot(&self) -> AppConfig {
        self.read().clone()
    }
}

impl AppConfig {
    pub fn load_from(path: &Path) -> Self {
        let json = match std::fs::read_to_string(path) {
            Ok(json) => json,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                log::warn!(
                    "Failed to read config at {}: {err}; using defaults",
                    path.display()
                );
                return Self::default();
            }
        };
        match serde_json::from_str::<AppConfig>(&json) {
            Ok(mut config) => {
                config.tracked_device_keys.retain(|k| !k.is_empty());
                config
            }
            Err(err) => {
                log::error!(
                    "Failed to parse config at {}: {err}; using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    pub fn load() -> Self {
        Self::load_from(&paths::config_file())
    }

    /// Writes the settings through a temporary file, so an interrupted save
    /// leaves the previous config intact instead of a truncated one.
    ///
    /// # Errors
    /// Returns the I/O error from creating, writing, syncing or renaming the
    /// file, or a `serde_json` encoding error.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let dir = path.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("config path has no parent: {}", path.display()),
            )
        })?;
        std::fs::create_dir_all(dir)?;

        let json = serde_json::to_string_pretty(self)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        let temp = path.with_extension("json.tmp");
        {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "kwmswitcher-{name}-{}-{unique}.json",
            std::process::id()
        ))
    }

    #[test]
    fn roundtrip_and_defaults() {
        let config = AppConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"LocalInputSource\":15"));
        assert!(json.contains("\"InputProtocol\":0"));
        assert!(json.contains("\"StartMinimized\":true"));
        let back: AppConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.local_input_source, 0x0F);
        assert_eq!(back.remote_input_source, 0x11);
    }

    #[test]
    fn reads_csharp_shape() {
        // Exactly what the Avalonia version wrote.
        let json = r#"{
            "LocalInputSource": 17,
            "RemoteInputSource": 15,
            "InputProtocol": 1,
            "TrackedDeviceKeys": ["046d:c52b"],
            "PollIntervalMs": 500,
            "StartMinimized": false,
            "AutoStart": true,
            "TargetMonitorName": "Display 1 — GBT3241"
        }"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.local_input_source, 17);
        assert_eq!(config.remote_input_source, 15);
        assert_eq!(config.input_protocol, InputSwitchProtocol::Lg);
        assert_eq!(config.tracked_device_keys, vec!["046d:c52b".to_string()]);
        assert_eq!(config.poll_interval_ms, 500);
        assert!(!config.start_minimized);
        assert_eq!(
            config.target_monitor_name.as_deref(),
            Some("Display 1 — GBT3241")
        );
    }

    #[test]
    fn tolerates_null_tracked_keys() {
        let json = r#"{"TrackedDeviceKeys": null}"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert!(config.tracked_device_keys.is_empty());
    }

    #[test]
    fn saves_and_loads_roundtrip() {
        let path = temp_path("save");
        let config = AppConfig {
            tracked_device_keys: vec!["046d:c52b".into()],
            ..AppConfig::default()
        };
        config.save_to(&path).expect("save config");
        assert!(
            !path.with_extension("json.tmp").exists(),
            "temp file removed"
        );
        let loaded = AppConfig::load_from(&path);
        assert_eq!(loaded.tracked_device_keys, config.tracked_device_keys);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn corrupt_file_falls_back_to_defaults() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ not json").expect("write corrupt config");
        let loaded = AppConfig::load_from(&path);
        assert_eq!(
            loaded.poll_interval_ms,
            AppConfig::default().poll_interval_ms
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn save_reports_io_errors() {
        // `/proc` cannot host new directories; the settings window relies on
        // this error to keep itself open and show the reason.
        let unwritable = std::path::Path::new("/proc/kwmswitcher/config.json");
        let error = AppConfig::default()
            .save_to(unwritable)
            .expect_err("writing under /proc must fail");
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn shared_config_is_shared() {
        let config = SharedConfig::new(AppConfig::default());
        let other = config.clone();
        config.update(|cfg| cfg.poll_interval_ms = 500);
        assert_eq!(other.read().poll_interval_ms, 500);
    }
}
