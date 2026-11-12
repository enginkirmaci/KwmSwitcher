//! Runs the external `ddcutil` binary to talk DDC/CI to the monitor.
//!
//! Every invocation is serialized behind one lock (the i2c bus can't serve
//! concurrent requests and concurrent calls are the plausible cause of native
//! i2c faults mid-switch) and bounded by a hard timeout so a wedged process is
//! killed rather than awaited forever.

use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::config::{InputSwitchProtocol, SharedConfig};
use crate::input_source as isrc;
use crate::input_source::PipMode;

const DDCUTIL_BIN: &str = "ddcutil";
const DDCUTIL_TIMEOUT: Duration = Duration::from_secs(15);
/// Read-back is a check on a write, not a user action: cap it shorter so a
/// display that does not implement the read costs little.
const READ_BACK_TIMEOUT: Duration = Duration::from_secs(4);
/// How long to let an input change settle before reading it back.
const SETTLE_AFTER_WRITE: Duration = Duration::from_millis(500);
/// Consecutive failed read-backs before verification is dropped for the
/// session (one failure can simply mean the display was asleep).
const MAX_READ_BACK_FAILURES: u8 = 2;
/// How long the output readers get after the child exits. The readers are
/// abandoned (not joined) past this, so a grandchild holding the pipe open
/// cannot pin the engine thread.
const OUTPUT_GRACE: Duration = Duration::from_secs(2);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// What happened to an input switch, including what the monitor reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSwitchOutcome {
    /// The monitor reports the requested input after the write.
    Confirmed,
    /// The write succeeded, but the display does not report its input source,
    /// so the change could not be verified; treated as success.
    Unverified,
    /// The monitor still reports a different input, even after one retry.
    Mismatch { actual: u8 },
    /// The write itself failed.
    Failed,
}

/// The monitor operations the switch engine needs.
///
/// [`Ddc`] implements this with ddcutil; the engine's tests implement it with
/// an in-memory fake, so the state machine can be exercised without hardware.
pub(crate) trait Monitor {
    /// Switches the input and reports what the display says afterwards.
    fn set_input_source(&self, logical: u8) -> InputSwitchOutcome;
    /// The current input in MCCS logical codes, when the display reports it.
    fn get_input_source(&self) -> Option<u8>;
    /// The current PiP/PBP mode, or `None` when the display does not answer.
    fn get_pip_mode(&self) -> Option<PipMode>;
    /// Sets the PiP/PBP mode; `false` when the write failed or is unsupported.
    fn set_pip_mode(&self, mode: PipMode) -> bool;
    /// Whether PiP/PBP control is available for the configured protocol.
    fn pip_supported(&self) -> bool;
    /// Re-enables input read-back after the settings changed.
    fn reset_read_back(&self);
}

pub struct Ddc {
    config: SharedConfig,
    /// Serializes all ddcutil invocations.
    bus_lock: Mutex<()>,
    /// Consecutive reads that came back empty: after a few, this display does
    /// not answer input-source reads, so later writes skip the extra call.
    read_back_failures: AtomicU8,
}

struct Captured {
    success: bool,
    stdout: String,
    stderr: String,
}

/// The protocol plus the display the user pinned for this command.
#[derive(Debug, Clone, Copy)]
struct Target {
    protocol: InputSwitchProtocol,
    display: Option<u32>,
}

impl Target {
    /// `--display N` before the subcommand, so ddcutil addresses only the
    /// configured monitor instead of every attached one.
    fn with_display(self, mut args: Vec<String>) -> Vec<String> {
        let Some(display) = self.display else {
            return args;
        };
        let mut prefixed = Vec::with_capacity(args.len() + 2);
        prefixed.push("--display".to_string());
        prefixed.push(display.to_string());
        prefixed.append(&mut args);
        prefixed
    }
}

impl Ddc {
    pub fn new(config: SharedConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            bus_lock: Mutex::new(()),
            read_back_failures: AtomicU8::new(0),
        })
    }

    fn lock_bus(&self) -> MutexGuard<'_, ()> {
        self.bus_lock.lock().expect("ddc bus lock poisoned")
    }

    fn target(&self) -> Target {
        let config = self.config.read();
        Target {
            protocol: config.input_protocol,
            display: config
                .target_monitor_name
                .as_deref()
                .and_then(display_number),
        }
    }

    /// Whether PiP/PBP control is available for the configured protocol.
    pub fn pip_supported(&self) -> bool {
        isrc::pip_supported(self.config.read().input_protocol)
    }

    /// Re-enables read-back verification; call after the settings changed,
    /// since the protocol or the target display may be different now.
    pub fn reset_read_back(&self) {
        self.read_back_failures.store(0, Ordering::Relaxed);
    }

    fn read_back_enabled(&self) -> bool {
        self.read_back_failures.load(Ordering::Relaxed) < MAX_READ_BACK_FAILURES
    }

    fn note_read_back_result(&self, reported: bool) {
        if reported {
            self.read_back_failures.store(0, Ordering::Relaxed);
            return;
        }
        let failures = self.read_back_failures.fetch_add(1, Ordering::Relaxed) + 1;
        if failures == MAX_READ_BACK_FAILURES {
            log::info!(
                "Monitor does not report its input source; skipping read-back verification \
                 for this session"
            );
        }
    }

    /// Switches the input and verifies what the monitor reports back.
    ///
    /// The read-back uses the standard VCP `0x60` (see [`Ddc::read_input_locked`]).
    /// The write is retried once when the display keeps reporting the old
    /// input; a display that does not answer the read at all is reported as
    /// [`InputSwitchOutcome::Unverified`], and read-back is skipped after a
    /// few failures for the rest of the session.
    pub fn set_input_source(&self, logical: u8) -> InputSwitchOutcome {
        let _guard = self.lock_bus();
        let target = self.target();
        let protocol = target.protocol;
        let args = target.with_display(build_set_args(
            protocol,
            isrc::input_vcp_code(protocol),
            isrc::encode_input(protocol, logical),
            isrc::input_i2c_source_addr(protocol),
        ));

        if !self.write_vcp_locked("input source", &args) {
            return InputSwitchOutcome::Failed;
        }
        if !self.read_back_enabled() {
            return InputSwitchOutcome::Unverified;
        }
        // An input change is not always visible over I2C immediately.
        std::thread::sleep(SETTLE_AFTER_WRITE);

        match self.read_input_locked(target) {
            None => {
                self.note_read_back_result(false);
                InputSwitchOutcome::Unverified
            }
            Some(actual) if actual == logical => {
                self.note_read_back_result(true);
                InputSwitchOutcome::Confirmed
            }
            Some(actual) => {
                self.note_read_back_result(true);
                log::warn!(
                    "Monitor still reports {} after switching to {}; retrying once",
                    isrc::input_name(actual),
                    isrc::input_name(logical)
                );
                if !self.write_vcp_locked("input source (retry)", &args) {
                    return InputSwitchOutcome::Failed;
                }
                std::thread::sleep(SETTLE_AFTER_WRITE);
                match self.read_input_locked(target) {
                    Some(retry) if retry == logical => {
                        self.note_read_back_result(true);
                        InputSwitchOutcome::Confirmed
                    }
                    Some(retry) => {
                        self.note_read_back_result(true);
                        InputSwitchOutcome::Mismatch { actual: retry }
                    }
                    None => {
                        self.note_read_back_result(false);
                        InputSwitchOutcome::Unverified
                    }
                }
            }
        }
    }

    /// Reads the monitor's current input source in logical (MCCS) codes, when
    /// the display reports it. Returns `None` when the display does not answer
    /// or when read-back was disabled after repeated failures.
    pub fn get_input_source(&self) -> Option<u8> {
        if !self.read_back_enabled() {
            return None;
        }
        let _guard = self.lock_bus();
        let target = self.target();
        let input = self.read_input_locked(target);
        self.note_read_back_result(input.is_some());
        input
    }

    pub fn get_pip_mode(&self) -> Option<PipMode> {
        let _guard = self.lock_bus();
        let target = self.target();
        if !isrc::pip_supported(target.protocol) {
            return None;
        }
        let protocol = target.protocol;
        let args = target.with_display(build_get_args(
            protocol,
            isrc::pip_vcp_code(protocol),
            isrc::pip_i2c_source_addr(protocol),
        ));
        self.read_vcp_locked("getvcp PiP", &args, DDCUTIL_TIMEOUT)
            .map(|wire| isrc::decode_pip(protocol, wire))
    }

    pub fn set_pip_mode(&self, mode: PipMode) -> bool {
        let _guard = self.lock_bus();
        let target = self.target();
        if !isrc::pip_supported(target.protocol) {
            log::info!("PiP/PBP control is only implemented for the LG protocol");
            return false;
        }
        let protocol = target.protocol;
        let Some(wire) = isrc::encode_pip(protocol, mode) else {
            log::warn!(
                "Refusing to write an unrecognized PiP mode: {}",
                mode.label()
            );
            return false;
        };
        let args = target.with_display(build_set_args(
            protocol,
            isrc::pip_vcp_code(protocol),
            wire,
            isrc::pip_i2c_source_addr(protocol),
        ));
        self.write_vcp_locked("setvcp PiP", &args)
    }

    /// Best-effort monitor enumeration via `ddcutil detect`.
    pub fn available_monitors(&self) -> Vec<String> {
        let _guard = self.lock_bus();
        let args = vec!["detect".to_string()];
        match self.run_capture_locked(&args, DDCUTIL_TIMEOUT) {
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

    /// Reads the input source through the **standard** VCP `0x60`, for every
    /// protocol. LG's `0xF4` register does not echo the value that was written
    /// (ddcutil's own write verification fails on it, which is why the LG path
    /// uses `--noverify`), while `0x60` reports the MCCS logical code on LG
    /// panels too. The display target still applies.
    fn read_input_locked(&self, target: Target) -> Option<u8> {
        let args = target.with_display(build_get_args(
            InputSwitchProtocol::Standard,
            isrc::INPUT_VCP_CODE,
            0,
        ));
        self.read_vcp_locked("input source read-back", &args, READ_BACK_TIMEOUT)
            .map(|wire| isrc::decode_input(InputSwitchProtocol::Standard, wire))
    }

    /// Both helpers below assume the bus lock is held by the caller.
    fn write_vcp_locked(&self, label: &str, args: &[String]) -> bool {
        let Some(captured) = self.run_capture_locked(args, DDCUTIL_TIMEOUT) else {
            log::error!("Failed to run ddcutil ({label})");
            return false;
        };
        if !captured.success && !captured.stderr.trim().is_empty() {
            log::warn!("ddcutil {label} failed: {}", captured.stderr.trim());
        }
        captured.success
    }

    /// Runs `ddcutil getvcp` and parses the current value.
    fn read_vcp_locked(&self, label: &str, args: &[String], timeout: Duration) -> Option<u8> {
        let captured = self.run_capture_locked(args, timeout)?;
        if !captured.success {
            let trimmed = captured.stderr.trim();
            if !trimmed.is_empty() {
                log::warn!("ddcutil {label} failed: {trimmed}");
            }
            return None;
        }
        let Some(value) = parse_incoming_value(&captured.stdout) else {
            log::warn!(
                "ddcutil {label} returned unparsable output: {}",
                captured.stdout.trim()
            );
            return None;
        };
        Some(value)
    }

    fn run_capture_locked(&self, args: &[String], timeout: Duration) -> Option<Captured> {
        let mut child = Command::new(DDCUTIL_BIN)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .inspect_err(|err| log::error!("Failed to spawn ddcutil: {err}"))
            .ok()?;

        // Drain both pipes on helper threads; waiting on the pipes directly
        // would deadlock once the OS pipe buffer fills.
        let stdout_pipe = child.stdout.take()?;
        let stderr_pipe = child.stderr.take()?;
        let (stdout_tx, stdout_rx) = mpsc::channel();
        let (stderr_tx, stderr_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = stdout_tx.send(read_pipe(stdout_pipe));
        });
        std::thread::spawn(move || {
            let _ = stderr_tx.send(read_pipe(stderr_pipe));
        });

        let deadline = Instant::now() + timeout;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(CHILD_POLL_INTERVAL);
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    log::warn!(
                        "ddcutil timed out after {}s and was killed: {}",
                        timeout.as_secs(),
                        args.join(" ")
                    );
                    return None;
                }
                Err(err) => {
                    log::error!("ddcutil wait failed: {err}");
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        };

        // The child is gone, so the readers hit EOF; bound the wait anyway.
        let stdout = stdout_rx.recv_timeout(OUTPUT_GRACE).unwrap_or_default();
        let stderr = stderr_rx.recv_timeout(OUTPUT_GRACE).unwrap_or_default();
        Some(Captured {
            success: status.success(),
            stdout,
            stderr,
        })
    }
}

impl Monitor for Ddc {
    fn set_input_source(&self, logical: u8) -> InputSwitchOutcome {
        Ddc::set_input_source(self, logical)
    }

    fn get_input_source(&self) -> Option<u8> {
        Ddc::get_input_source(self)
    }

    fn get_pip_mode(&self) -> Option<PipMode> {
        Ddc::get_pip_mode(self)
    }

    fn set_pip_mode(&self, mode: PipMode) -> bool {
        Ddc::set_pip_mode(self, mode)
    }

    fn pip_supported(&self) -> bool {
        Ddc::pip_supported(self)
    }

    fn reset_read_back(&self) {
        Ddc::reset_read_back(self);
    }
}

fn read_pipe(mut pipe: impl Read) -> String {
    let mut buf = Vec::new();
    let _ = pipe.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

/// `--i2c-source-addr` only applies to the LG path; standard monitors want
/// plain `setvcp`/`getvcp` and `--noverify` only on LG writes.
fn build_set_args(
    protocol: InputSwitchProtocol,
    vcp_code: u8,
    value: u8,
    i2c_addr: u8,
) -> Vec<String> {
    if protocol == InputSwitchProtocol::Lg && i2c_addr != 0 {
        vec![
            format!("--i2c-source-addr=0x{i2c_addr:02X}"),
            "setvcp".to_string(),
            format!("0x{vcp_code:02X}"),
            format!("0x{value:02X}"),
            "--noverify".to_string(),
        ]
    } else {
        vec![
            "setvcp".to_string(),
            format!("0x{vcp_code:02X}"),
            format!("0x{value:02X}"),
        ]
    }
}

fn build_get_args(protocol: InputSwitchProtocol, vcp_code: u8, i2c_addr: u8) -> Vec<String> {
    if protocol == InputSwitchProtocol::Lg && i2c_addr != 0 {
        vec![
            format!("--i2c-source-addr=0x{i2c_addr:02X}"),
            "getvcp".to_string(),
            format!("0x{vcp_code:02X}"),
        ]
    } else {
        vec!["getvcp".to_string(), format!("0x{vcp_code:02X}")]
    }
}

/// The display number out of a `ddcutil detect` label (`Display 2 — GBT3241`).
/// Returns `None` for an empty target (switch every monitor) or a stale label.
fn display_number(name: &str) -> Option<u32> {
    let rest = name.trim().strip_prefix("Display ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok().filter(|&number| number > 0)
}

/// Equivalent of the C# regex `Incoming\s*[=:]\s*(0x[0-9A-Fa-f]+)`, with a
/// fallback for newer ddcutil versions that report the current value as
/// `(sl=0x..)` instead (seen on LG displays for the PiP code).
fn parse_incoming_value(output: &str) -> Option<u8> {
    for line in output.lines() {
        let Some(pos) = line.find("Incoming") else {
            continue;
        };
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
                .map_or(hex.len(), |(i, _)| i);
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
        .map_or(hex.len(), |(i, _)| i);
    if end == 0 {
        return None;
    }
    u8::from_str_radix(&hex[..end], 16).ok()
}

/// `ddcutil detect` output. Current versions (ddcutil 3.x) label the model
/// under an EDID synopsis; older ones print `Monitor:`:
/// ```text
/// Display 1
///    I2C bus:  /dev/i2c-5
///    EDID synopsis:
///       Model:   LG ULTRAGEAR+
/// ```
/// Two `Display` blocks may report the same model, so the display number is
/// kept as the stable part of the label.
fn parse_detect(output: &str) -> Vec<String> {
    let mut descriptions = Vec::new();
    let mut current: Option<(u32, Option<String>)> = None;

    let flush = |current: Option<(u32, Option<String>)>, out: &mut Vec<String>| {
        if let Some((index, model)) = current {
            if index > 0 {
                let suffix = model.map(|m| format!(" — {m}")).unwrap_or_default();
                out.push(format!("Display {index}{suffix}"));
            }
        }
    };

    for raw in output.lines() {
        let line = raw.trim();
        if let Some(rest) = line.strip_prefix("Display ") {
            flush(current.take(), &mut descriptions);
            if let Ok(index) = rest.trim().parse::<u32>() {
                current = Some((index, None));
            }
            continue;
        }
        let model = line
            .strip_prefix("Model:")
            .or_else(|| line.strip_prefix("Monitor:"));
        if let (Some(model), Some((_, slot))) = (model, current.as_mut()) {
            let model = model.trim();
            if !model.is_empty() && slot.is_none() {
                *slot = Some(model.to_string());
            }
        }
    }
    flush(current, &mut descriptions);
    descriptions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_incoming_value() {
        assert_eq!(
            parse_incoming_value("VCP code 0x60: Incoming: 0x11"),
            Some(0x11)
        );
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
        // ddcutil 3.0.2 reports the current input as `(sl=0x0f)`.
        assert_eq!(
            parse_incoming_value("VCP code 0x60 (Input Source): DisplayPort-1 (sl=0x0f)"),
            Some(0x0f)
        );
    }

    #[test]
    fn parses_detect_output() {
        // Old ddcutil format.
        let out = "Display 1\n   I2C bus:  /dev/i2c-5\n   Monitor:  GBT3241\n\nDisplay 2\n   Monitor:  LG Ultra HD\n";
        assert_eq!(
            parse_detect(out),
            vec![
                "Display 1 — GBT3241".to_string(),
                "Display 2 — LG Ultra HD".to_string()
            ]
        );
        assert!(parse_detect("no monitors").is_empty());
    }

    #[test]
    fn parses_ddcutil_3_detect_output() {
        let out = "\
Display 1
   I2C bus:  /dev/i2c-14
   DRM_connector:           card2-DP-3
   EDID synopsis:
      Mfg id:               GSM - LG Electronics
      Model:                LG ULTRAGEAR+
      Product code:         40605  (0x9e9d)
   VCP version:         2.1

Display 2
   I2C bus:  /dev/i2c-16
   EDID synopsis:
      Model:                GBT3241
";
        assert_eq!(
            parse_detect(out),
            vec![
                "Display 1 — LG ULTRAGEAR+".to_string(),
                "Display 2 — GBT3241".to_string()
            ]
        );
    }

    #[test]
    fn detect_labels_without_a_model_are_still_listed() {
        assert_eq!(
            parse_detect("Display 3\n   I2C bus:  /dev/i2c-7\n"),
            vec!["Display 3".to_string()]
        );
    }

    #[test]
    fn parses_display_number_from_label() {
        assert_eq!(display_number("Display 1 — LG ULTRAGEAR+"), Some(1));
        assert_eq!(display_number("Display 12"), Some(12));
        assert_eq!(display_number("Display 0 — nope"), None);
        assert_eq!(display_number("Display — nope"), None);
        assert_eq!(display_number("monitor"), None);
        assert_eq!(display_number(""), None);
    }

    #[test]
    fn display_prefix_targets_one_monitor() {
        let target = Target {
            protocol: InputSwitchProtocol::Standard,
            display: Some(2),
        };
        assert_eq!(
            target.with_display(build_set_args(InputSwitchProtocol::Standard, 0x60, 0x11, 0)),
            vec!["--display", "2", "setvcp", "0x60", "0x11"]
        );
        let all = Target {
            protocol: InputSwitchProtocol::Standard,
            display: None,
        };
        assert_eq!(
            all.with_display(build_get_args(InputSwitchProtocol::Standard, 0x60, 0)),
            vec!["getvcp", "0x60"]
        );
    }

    #[test]
    fn arg_building() {
        assert_eq!(
            build_set_args(InputSwitchProtocol::Lg, 0xF4, 0xD0, 0x50),
            vec![
                "--i2c-source-addr=0x50",
                "setvcp",
                "0xF4",
                "0xD0",
                "--noverify"
            ]
        );
        assert_eq!(
            build_set_args(InputSwitchProtocol::Standard, 0x60, 0x11, 0),
            vec!["setvcp", "0x60", "0x11"]
        );
        assert_eq!(
            build_get_args(InputSwitchProtocol::Standard, 0x60, 0),
            vec!["getvcp", "0x60"]
        );
        assert_eq!(
            build_get_args(InputSwitchProtocol::Lg, 0xD7, 0x51),
            vec!["--i2c-source-addr=0x51", "getvcp", "0xD7"]
        );
    }
}
