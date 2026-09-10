//! Persisted application settings, JSON-compatible with the previous Avalonia
//! version (`~/.config/KwmSwitcher/config.json`, PascalCase keys, protocol as
//! an integer), so existing installs migrate without touching their settings.

use std::path::Path;
use std::sync::{Arc, RwLock};

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

pub type SharedConfig = Arc<RwLock<AppConfig>>;

impl AppConfig {
    pub fn load_from(path: &Path) -> Self {
        let json = match std::fs::read_to_string(path) {
            Ok(json) => json,
            Err(_) => return Self::default(),
        };
        match serde_json::from_str::<AppConfig>(&json) {
            Ok(mut config) => {
                config.tracked_device_keys.retain(|k| !k.is_empty());
                config
            }
            Err(err) => {
                log::error!("Failed to parse config at {}: {err}; using defaults", path.display());
                Self::default()
            }
        }
    }

    pub fn load() -> Self {
        Self::load_from(&paths::config_file())
    }

    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".into());
        std::fs::write(path, json)
    }

    pub fn save(&self) {
        if let Err(err) = self.save_to(&paths::config_file()) {
            log::error!("Failed to save config: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(config.target_monitor_name.as_deref(), Some("Display 1 — GBT3241"));
    }

    #[test]
    fn tolerates_null_tracked_keys() {
        let json = r#"{"TrackedDeviceKeys": null}"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert!(config.tracked_device_keys.is_empty());
    }
}
