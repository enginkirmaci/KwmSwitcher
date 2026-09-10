//! USB device enumeration from sysfs and a poll thread that diffs the device
//! set and reports changes to the engine.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use crate::config::SharedConfig;
use crate::engine::EngineCommand;

const SYSFS_USB_DEVICES: &str = "/sys/bus/usb/devices";

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
}

/// One-shot enumeration of currently attached USB devices.
pub fn list_devices() -> Vec<UsbDevice> {
    let Ok(entries) = std::fs::read_dir(SYSFS_USB_DEVICES) else {
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
    devices.sort_by(|a, b| a.description.cmp(&b.description));
    devices
}

pub struct UsbMonitor {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl UsbMonitor {
    /// Spawns the poll loop. On every device-set change it sends
    /// [`EngineCommand::DevicesChanged`] to the engine.
    pub fn spawn(config: SharedConfig, engine_tx: Sender<EngineCommand>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = stop.clone();
        let config = config.clone();

        let poll_interval = move || {
            Duration::from_millis(
                config.read().map(|c| c.poll_interval_ms).unwrap_or(1000).max(100) as u64,
            )
        };

        let thread = std::thread::Builder::new()
            .name("usb-monitor".into())
            .spawn(move || {
                let mut last_keys: HashSet<String> =
                    list_devices().iter().map(UsbDevice::key).collect();
                loop {
                    // Read the interval each tick so settings changes apply live.
                    std::thread::sleep(poll_interval());
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }
                    let current: HashSet<String> =
                        list_devices().iter().map(UsbDevice::key).collect();
                    if current != last_keys {
                        last_keys = current;
                        if engine_tx
                            .send(EngineCommand::DevicesChanged(list_devices()))
                            .is_err()
                        {
                            break; // engine gone
                        }
                    }
                }
            })
            .expect("failed to spawn usb monitor thread");

        Self {
            stop,
            thread: Some(thread),
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
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
    }
}
