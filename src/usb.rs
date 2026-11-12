//! USB device enumeration from sysfs and a poll thread that diffs the device
//! set and reports changes to the engine.

use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::Duration;

use crate::config::SharedConfig;
use crate::engine::EngineCommand;

const SYSFS_USB_DEVICES: &str = "/sys/bus/usb/devices";
/// Floor for the poll interval, so a bad config value cannot spin the thread.
const MIN_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbDevice {
    pub vendor_id: String,
    pub product_id: String,
    pub description: String,
}

impl UsbDevice {
    pub fn key(&self) -> String {
        format!("{}:{}", self.vendor_id, self.product_id)
    }

    /// True when `key` is this device's `vendor:product` id. The comparison is
    /// ASCII-case-insensitive because sysfs hex ids are lower case but the
    /// config may have been written by hand.
    pub fn matches_key(&self, key: &str) -> bool {
        key.split_once(':').is_some_and(|(vendor, product)| {
            self.vendor_id.eq_ignore_ascii_case(vendor)
                && self.product_id.eq_ignore_ascii_case(product)
        })
    }
}

/// One-shot enumeration of currently attached USB devices.
pub fn list_devices() -> Vec<UsbDevice> {
    list_devices_in(Path::new(SYSFS_USB_DEVICES))
}

fn list_devices_in(root: &Path) -> Vec<UsbDevice> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut devices = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let id_vendor_path = dir.join("idVendor");
        let id_product_path = dir.join("idProduct");
        if !id_vendor_path.is_file() || !id_product_path.is_file() {
            continue;
        }
        let (Ok(vendor_id), Ok(product_id)) = (
            std::fs::read_to_string(&id_vendor_path),
            std::fs::read_to_string(&id_product_path),
        ) else {
            continue;
        };
        let (vendor_id, product_id) = (vendor_id.trim().to_string(), product_id.trim().to_string());
        if vendor_id.is_empty() || product_id.is_empty() {
            continue;
        }
        let description = std::fs::read_to_string(dir.join("product"))
            .map(|s| s.trim().to_string())
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| format!("USB Device {vendor_id}:{product_id}"));
        devices.push(UsbDevice {
            vendor_id,
            product_id,
            description,
        });
    }
    // Total order: descriptions can repeat (two identical devices), so fall
    // back to the id pair to keep the UI list stable between polls.
    devices.sort_by(|a, b| {
        a.description
            .cmp(&b.description)
            .then_with(|| a.vendor_id.cmp(&b.vendor_id))
            .then_with(|| a.product_id.cmp(&b.product_id))
    });
    devices
}

fn device_keys(devices: &[UsbDevice]) -> HashSet<String> {
    devices.iter().map(UsbDevice::key).collect()
}

pub struct UsbMonitor {
    stop: Option<Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl UsbMonitor {
    /// Spawns the poll loop. On every device-set change it sends
    /// [`EngineCommand::DevicesChanged`] to the engine.
    ///
    /// # Panics
    /// Panics when the OS refuses to spawn the thread.
    pub fn spawn(config: SharedConfig, engine_tx: Sender<EngineCommand>) -> Self {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let thread = std::thread::Builder::new()
            .name("usb-monitor".into())
            .spawn(move || {
                let mut last_keys = device_keys(&list_devices());
                loop {
                    // Read the interval each tick so settings changes apply live.
                    let interval = Duration::from_millis(u64::from(config.read().poll_interval_ms))
                        .max(MIN_POLL_INTERVAL);
                    match stop_rx.recv_timeout(interval) {
                        Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    // One snapshot per tick, used both for the diff and for
                    // the message, so the engine never sees a half-updated set.
                    let devices = list_devices();
                    let keys = device_keys(&devices);
                    if keys != last_keys {
                        last_keys = keys;
                        if engine_tx
                            .send(EngineCommand::DevicesChanged(devices))
                            .is_err()
                        {
                            break; // engine gone
                        }
                    }
                }
            })
            .expect("failed to spawn usb monitor thread");

        Self {
            stop: Some(stop_tx),
            thread: Some(thread),
        }
    }

    pub fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for UsbMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_device(root: &Path, dir: &str, vendor: &str, product: &str, product_name: &str) {
        let path = root.join(dir);
        std::fs::create_dir_all(&path).expect("create fake usb device dir");
        std::fs::write(path.join("idVendor"), vendor).expect("write idVendor");
        std::fs::write(path.join("idProduct"), product).expect("write idProduct");
        std::fs::write(path.join("product"), product_name).expect("write product");
    }

    #[test]
    fn sysfs_is_readable_or_absent() {
        // On a dev machine without /sys/bus/usb/devices this still must not panic.
        let _ = list_devices();
    }

    #[test]
    fn key_format_matches_csharp() {
        let d = UsbDevice {
            vendor_id: "046d".into(),
            product_id: "c52b".into(),
            description: "USB Receiver".into(),
        };
        assert_eq!(d.key(), "046d:c52b");
        assert!(d.matches_key("046d:c52b"));
        assert!(d.matches_key("046D:C52B"));
        assert!(!d.matches_key("046d:c52c"));
        assert!(!d.matches_key("046d"));
    }

    #[test]
    fn enumerates_only_device_dirs() {
        let root = std::env::temp_dir().join(format!(
            "kwmswitcher-usb-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fake sysfs root");
        write_device(&root, "1-1", "046d", "c52b", "USB Receiver");
        write_device(&root, "2-1", "1234", "5678", "");
        // Interface dirs carry no ids and must be skipped.
        std::fs::create_dir_all(root.join("1-1:1.0")).expect("create fake interface dir");

        let devices = list_devices_in(&root);
        assert_eq!(devices.len(), 2, "devices: {devices:?}");
        assert!(devices.iter().any(|d| d.key() == "046d:c52b"));
        assert!(devices
            .iter()
            .any(|d| d.description == "USB Device 1234:5678"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
