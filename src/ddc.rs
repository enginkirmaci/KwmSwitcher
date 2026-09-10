//! Runs the external `ddcutil` binary to talk DDC/CI to the monitor.
//!
//! Every invocation is serialized behind one lock (the i2c bus can't serve
//! concurrent requests and concurrent calls are the plausible cause of native
//! i2c faults mid-switch) and bounded by a hard timeout so a wedged process is
//! killed rather than awaited forever.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::{InputSwitchProtocol, SharedConfig};
use crate::input_source as isrc;

const DDCUTIL_TIMEOUT: Duration = Duration::from_secs(15);

pub struct Ddc {
    config: SharedConfig,
    /// Serializes all ddcutil invocations.
    bus_lock: Mutex<()>,
}

struct Captured {
    success: bool,
    stdout: String,
    stderr: String,
}

impl Ddc {
    pub fn new(config: SharedConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            bus_lock: Mutex::new(()),
        })
    }

    fn protocol(&self) -> InputSwitchProtocol {
        self.config.read().unwrap().input_protocol
    }

    pub fn set_input_source(&self, logical: u8) -> bool {
        let protocol = self.protocol();
        let args = build_set_args(
            protocol,
            isrc::input_vcp_code(protocol),
            isrc::encode_input(protocol, logical),
            isrc::input_i2c_source_addr(protocol),
        );
        self.set_vcp("input source", &args)
    }

    /// Kept for API parity with the C# version; unused until input-state
    /// polling is wired into the UI.
    #[allow(dead_code)]
    pub fn get_input_source(&self) -> Option<u8> {
        let protocol = self.protocol();
        let args = build_get_args(
            protocol,
            isrc::input_vcp_code(protocol),
            isrc::input_i2c_source_addr(protocol),
        );
        self.get_vcp("getvcp", &args)
            .map(|wire| isrc::decode_input(protocol, wire))
    }

    pub fn get_pip_mode(&self) -> Option<u8> {
        let protocol = self.protocol();
        let args = build_get_args(
            protocol,
            isrc::pip_vcp_code(protocol),
            isrc::pip_i2c_source_addr(protocol),
        );
        self.get_vcp("getvcp PiP", &args)
            .map(|wire| isrc::decode_pip(protocol, wire))
    }

    pub fn set_pip_mode(&self, mode: u8) -> bool {
        let protocol = self.protocol();
        let args = build_set_args(
            protocol,
            isrc::pip_vcp_code(protocol),
            isrc::encode_pip(protocol, mode),
            isrc::pip_i2c_source_addr(protocol),
        );
        self.set_vcp("setvcp PiP", &args)
    }

    /// Best-effort monitor enumeration via `ddcutil detect`.
    pub fn available_monitors(&self) -> Vec<String> {
        match self.run_capture("detect") {
            Some(captured) if captured.success => parse_detect(&captured.stdout),
            Some(captured) => {
                let trimmed = captured.stderr.trim();
                if !trimmed.is_empty() {
                    log::warn!("ddcutil detect failed: {trimmed}");
                }
                Vec::new()
            }
            None => Vec::new(),
        }
    }

    fn set_vcp(&self, label: &str, args: &str) -> bool {
        match self.run_capture(args) {
            Some(captured) => {
                if !captured.success && !captured.stderr.trim().is_empty() {
                    log::warn!("ddcutil {label} failed: {}", captured.stderr.trim());
                }
                captured.success
            }
            None => {
                log::error!("Failed to run ddcutil ({label})");
                false
            }
        }
    }

    /// Runs `ddcutil getvcp` and parses the `Incoming` value.
    fn get_vcp(&self, label: &str, args: &str) -> Option<u8> {
        let captured = self.run_capture(args)?;
        if !captured.success {
            let trimmed = captured.stderr.trim();
            if !trimmed.is_empty() {
                log::warn!("ddcutil {label} failed: {trimmed}");
            }
            return None;
        }
        match parse_incoming_value(&captured.stdout) {
            Some(value) => Some(value),
            None => {
                log::warn!("ddcutil {label} returned unparsable output: {}", captured.stdout.trim());
                None
            }
        }
    }

    fn run_capture(&self, arguments: &str) -> Option<Captured> {
        let _guard = self.bus_lock.lock().ok()?;
        let mut child = Command::new("ddcutil")
            .args(arguments.split_whitespace())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .inspect_err(|err| log::error!("Failed to spawn ddcutil: {err}"))
            .ok()?;

        // Drain both pipes on helper threads; waiting on the pipes directly
        // would deadlock once the OS pipe buffer fills.
        let mut stdout_pipe = child.stdout.take()?;
        let mut stderr_pipe = child.stderr.take()?;
        let stdout_reader = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stdout_pipe.read_to_string(&mut buf);
            buf
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut buf = String::new();
            let _ = stderr_pipe.read_to_string(&mut buf);
            buf
        });

        let deadline = Instant::now() + DDCUTIL_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let stdout = stdout_reader.join().unwrap_or_default();
                    let stderr = stderr_reader.join().unwrap_or_default();
                    return Some(Captured {
                        success: status.success(),
                        stdout,
                        stderr,
                    });
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(err) => {
                    log::error!("ddcutil wait failed: {err}");
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = stdout_reader.join();
                    let _ = stderr_reader.join();
                    return None;
                }
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        log::warn!(
            "ddcutil timed out after {}s and was killed: {arguments}",
            DDCUTIL_TIMEOUT.as_secs()
        );
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        None
    }
}

/// `--i2c-source-addr` only applies to the LG path; standard monitors want
/// plain `setvcp`/`getvcp` and `--noverify` only on LG writes.
fn build_set_args(protocol: InputSwitchProtocol, vcp_code: u8, value: u8, i2c_addr: u8) -> String {
    if protocol == InputSwitchProtocol::Lg && i2c_addr != 0 {
        format!("--i2c-source-addr=0x{i2c_addr:02X} setvcp 0x{vcp_code:02X} 0x{value:02X} --noverify")
    } else {
        format!("setvcp 0x{vcp_code:02X} 0x{value:02X}")
    }
}

fn build_get_args(protocol: InputSwitchProtocol, vcp_code: u8, i2c_addr: u8) -> String {
    if protocol == InputSwitchProtocol::Lg && i2c_addr != 0 {
        format!("--i2c-source-addr=0x{i2c_addr:02X} getvcp 0x{vcp_code:02X}")
    } else {
        format!("getvcp 0x{vcp_code:02X}")
    }
}

/// Equivalent of the C# regex `Incoming\s*[=:]\s*(0x[0-9A-Fa-f]+)`, with a
/// fallback for newer ddcutil versions that report the current value as
/// `(sl=0x..)` instead (seen on LG displays for the PiP code).
fn parse_incoming_value(output: &str) -> Option<u8> {
    for line in output.lines() {
        let Some(pos) = line.find("Incoming") else { continue };
        if let Some(value) = parse_hex_after_separator(&line[pos + "Incoming".len()..]) {
            return Some(value);
        }
    }
    for line in output.lines() {
        if let Some(pos) = line.find("sl=0x") {
            let hex = &line[pos + "sl=0x".len()..];
            let end = hex
                .char_indices()
                .find(|(_, c)| !c.is_ascii_hexdigit())
                .map(|(i, _)| i)
                .unwrap_or(hex.len());
            if end > 0 {
                if let Ok(value) = u8::from_str_radix(&hex[..end], 16) {
                    return Some(value);
                }
            }
        }
    }
    None
}

fn parse_hex_after_separator(after: &str) -> Option<u8> {
    let after = after.trim_start();
    let after = after
        .strip_prefix('=')
        .or_else(|| after.strip_prefix(':'))?
        .trim_start();
    let hex = after
        .strip_prefix("0x")
        .or_else(|| after.strip_prefix("0X"))?;
    let end = hex
        .char_indices()
        .find(|(_, c)| !c.is_ascii_hexdigit())
        .map(|(i, _)| i)
        .unwrap_or(hex.len());
    if end == 0 {
        return None;
    }
    u8::from_str_radix(&hex[..end], 16).ok()
}

/// `ddcutil detect` output:
/// ```text
/// Display 1
///    I2C bus:  /dev/i2c-5
///    Monitor:  GBT3241
/// ```
fn parse_detect(output: &str) -> Vec<String> {
    let mut descriptions = Vec::new();
    let mut display_index = 0u32;
    let mut model: Option<String> = None;

    let mut flush = |index: u32, model: &Option<String>| {
        if index > 0 {
            if let Some(model) = model.as_deref().filter(|m| !m.trim().is_empty()) {
                descriptions.push(format!("Display {index} — {}", model.trim()));
            }
        }
    };

    for raw in output.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Display ") {
            flush(display_index, &model);
            display_index = rest.trim().parse().unwrap_or(display_index + 1);
            model = None;
            continue;
        }
        if let Some(value) = line.strip_prefix("Monitor:") {
            let value = value.trim();
            if !value.is_empty() {
                model = Some(value.to_string());
            }
        }
    }
    flush(display_index, &model);
    descriptions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_incoming_value() {
        assert_eq!(parse_incoming_value("VCP code 0x60: Incoming: 0x11"), Some(0x11));
        assert_eq!(parse_incoming_value("Incoming = 0x0f"), Some(0x0f));
        assert_eq!(parse_incoming_value("Incoming: 15"), None);
        assert_eq!(parse_incoming_value(""), None);
        // Newer ddcutil format seen on LG displays.
        assert_eq!(
            parse_incoming_value(
                "VCP code 0xd7 (Auxiliary power output        ): Disable auxiliary power (sl=0x01)"
            ),
            Some(0x01)
        );
    }

    #[test]
    fn parses_detect_output() {
        let out = "Display 1\n   I2C bus:  /dev/i2c-5\n   Monitor:  GBT3241\n\nDisplay 2\n   Monitor:  LG Ultra HD\n";
        assert_eq!(
            parse_detect(out),
            vec!["Display 1 — GBT3241".to_string(), "Display 2 — LG Ultra HD".to_string()]
        );
        assert!(parse_detect("no monitors").is_empty());
    }

    #[test]
    fn arg_building() {
        assert_eq!(
            build_set_args(InputSwitchProtocol::Lg, 0xF4, 0xD0, 0x50),
            "--i2c-source-addr=0x50 setvcp 0xF4 0xD0 --noverify"
        );
        assert_eq!(
            build_set_args(InputSwitchProtocol::Standard, 0x60, 0x11, 0),
            "setvcp 0x60 0x11"
        );
        assert_eq!(
            build_get_args(InputSwitchProtocol::Standard, 0x60, 0),
            "getvcp 0x60"
        );
    }
}
