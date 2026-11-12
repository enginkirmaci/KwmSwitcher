//! The switcher state machine: watches tracked USB devices, drives DDC/CI
//! input switches with a cooldown, and tracks PiP/PBP mode.
//!
//! Runs on its own thread; the UI talks to it through an mpsc command queue
//! and receives [`UiEvent`]s back. All ddcutil traffic (slow, seconds-long)
//! stays off the UI thread by construction.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::SharedConfig;
use crate::ddc::{Ddc, InputSwitchOutcome, Monitor};
use crate::input_source as isrc;
use crate::input_source::PipMode;
use crate::usb::UsbDevice;

#[derive(Clone)]
pub enum EngineCommand {
    DevicesChanged(Vec<UsbDevice>),
    /// Settings changed: re-read them and evaluate against a fresh device list.
    ConfigChanged,
    SwitchToLocal,
    SwitchToRemote,
    TogglePip,
    /// Set the PiP mode explicitly (used by the PiP / Single buttons).
    SetPip(PipMode),
    RefreshPip,
    Stop,
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    Status(String),
    LocalActive(bool),
    PipChanged {
        mode: PipMode,
        query_failed: bool,
    },
    /// Whether the configured protocol supports PiP/PBP control at all.
    PipSupported(bool),
    /// The configured tracked devices and whether each is currently
    /// attached, so the UI can show the list on hover.
    TrackedDevices(Vec<TrackedDeviceInfo>),
}

/// One tracked device as shown in the UI: display name and attach state.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackedDeviceInfo {
    pub name: String,
    pub present: bool,
}

const SWITCH_COOLDOWN: Duration = Duration::from_secs(3);

const STATUS_NO_TRACKED_DEVICES: &str =
    "No tracked devices configured. Open settings to select USB devices.";
const STATUS_LOCAL_ACTIVE: &str = "Local machine active, tracked USB devices detected";
const STATUS_REMOTE_ACTIVE: &str = "Remote machine active, no tracked USB devices";

#[derive(Clone)]
pub struct EngineHandle {
    pub(crate) tx: Sender<EngineCommand>,
}

impl EngineHandle {
    pub fn switch_to_local(&self) {
        let _ = self.tx.send(EngineCommand::SwitchToLocal);
    }

    pub fn switch_to_remote(&self) {
        let _ = self.tx.send(EngineCommand::SwitchToRemote);
    }

    pub fn set_pip(&self, mode: PipMode) {
        let _ = self.tx.send(EngineCommand::SetPip(mode));
    }

    pub fn refresh_pip(&self) {
        let _ = self.tx.send(EngineCommand::RefreshPip);
    }

    /// Tells the engine that the settings were saved, so it re-evaluates the
    /// current device set against the new tracked keys right away.
    pub fn config_changed(&self) {
        let _ = self.tx.send(EngineCommand::ConfigChanged);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(EngineCommand::Stop);
    }
}

struct EngineState {
    local_active: bool,
    pip_mode: PipMode,
    pip_supported: bool,
    pip_query_failed: bool,
    last_set_input: Option<u8>,
    last_switch_time: Option<Instant>,
}

impl EngineState {
    fn new() -> Self {
        Self {
            local_active: false,
            pip_mode: PipMode::Off,
            pip_supported: false,
            pip_query_failed: false,
            last_set_input: None,
            last_switch_time: None,
        }
    }

    fn is_pip_active(&self) -> bool {
        self.pip_mode.is_active()
    }

    fn within_cooldown(&self) -> bool {
        self.last_switch_time
            .is_some_and(|t| t.elapsed() < SWITCH_COOLDOWN)
    }
}

/// Spawns the engine thread; returns a command handle plus the UI event queue.
///
/// # Panics
/// Panics when the OS refuses to spawn the thread.
pub fn spawn(
    config: SharedConfig,
    ddc: Arc<Ddc>,
) -> (EngineHandle, Receiver<UiEvent>, Sender<EngineCommand>) {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<EngineCommand>();
    let (ui_tx, ui_rx) = std::sync::mpsc::channel::<UiEvent>();

    let handle = EngineHandle { tx: cmd_tx.clone() };

    let join = std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || run(config, ddc, cmd_rx, ui_tx))
        .expect("failed to spawn engine thread");
    // Detach: dropping a `JoinHandle` lets the thread run on. The loop exits
    // on `EngineCommand::Stop` or when every sender is dropped.
    drop(join);

    (handle, ui_rx, cmd_tx)
}

fn notify(ui_tx: &Sender<UiEvent>, event: UiEvent) {
    if ui_tx.send(event).is_err() {
        log::debug!("UI event queue closed; dropping event");
    }
}

fn set_status(ui_tx: &Sender<UiEvent>, status: impl Into<String>) {
    notify(ui_tx, UiEvent::Status(status.into()));
}

/// Status line shown whenever PiP/PBP is known to be active; the trailing
/// side tracks where the devices currently are, so it moves with them.
fn pip_status_text(state: &EngineState) -> String {
    format!(
        "PiP/PBP active ({}), auto-switch suspended · devices on {}",
        state.pip_mode.label(),
        if state.local_active {
            "local"
        } else {
            "remote"
        },
    )
}

fn run(
    config: SharedConfig,
    ddc: Arc<Ddc>,
    cmd_rx: Receiver<EngineCommand>,
    ui_tx: Sender<UiEvent>,
) {
    let monitor: &dyn Monitor = ddc.as_ref();
    let mut state = EngineState::new();

    // Seed the initial state from the current device set.
    let devices = crate::usb::list_devices();
    init_state(&config, &devices, &ui_tx);
    refresh_pip_state(monitor, &mut state, &ui_tx);
    // If the monitor was already in a split mode before launch, the PiP
    // status must win over init_state's plain side status.
    if state.is_pip_active() {
        set_status(&ui_tx, pip_status_text(&state));
    }

    while let Ok(command) = cmd_rx.recv() {
        if !handle_command(&config, monitor, &mut state, &ui_tx, command) {
            break;
        }
    }
    log::debug!("Engine thread exiting");
}

/// Applies one command; returns `false` when the engine should stop.
///
/// Split out from [`run`] so the command dispatch can be exercised by the
/// tests without a blocking channel or a real monitor.
fn handle_command(
    config: &SharedConfig,
    monitor: &dyn Monitor,
    state: &mut EngineState,
    ui_tx: &Sender<UiEvent>,
    command: EngineCommand,
) -> bool {
    match command {
        EngineCommand::Stop => return false,
        EngineCommand::DevicesChanged(devices) => {
            refresh_pip_state(monitor, state, ui_tx);
            evaluate_state(config, &devices, state, ui_tx, monitor);
        }
        EngineCommand::ConfigChanged => {
            // The tracked set and protocol may have changed, so re-read the
            // monitor state and take a fresh device snapshot rather than
            // waiting for the next USB event.
            monitor.reset_read_back();
            refresh_pip_state(monitor, state, ui_tx);
            let devices = crate::usb::list_devices();
            evaluate_state(config, &devices, state, ui_tx, monitor);
        }
        EngineCommand::SwitchToLocal => {
            let target = config.read().local_input_source;
            switch_input(monitor, state, ui_tx, target, true);
        }
        EngineCommand::SwitchToRemote => {
            let target = config.read().remote_input_source;
            switch_input(monitor, state, ui_tx, target, false);
        }
        EngineCommand::TogglePip => {
            let target = if state.is_pip_active() {
                PipMode::Off
            } else {
                PipMode::Pip
            };
            set_pip_mode(config, monitor, state, ui_tx, target);
        }
        EngineCommand::SetPip(mode) => set_pip_mode(config, monitor, state, ui_tx, mode),
        EngineCommand::RefreshPip => refresh_pip_state(monitor, state, ui_tx),
    }
    true
}

/// Mirrors the C# `InitState`: derive which side is active from device presence.
fn init_state(config: &SharedConfig, devices: &[UsbDevice], ui_tx: &Sender<UiEvent>) {
    let tracked = config.read().tracked_device_keys.clone();
    let device_list = tracked_device_list(devices, &tracked);
    let any_present = device_list.iter().any(|device| device.present);

    notify(ui_tx, UiEvent::TrackedDevices(device_list));
    if tracked.is_empty() {
        set_status(ui_tx, STATUS_NO_TRACKED_DEVICES);
        return;
    }

    if any_present {
        set_status(ui_tx, STATUS_LOCAL_ACTIVE);
        notify(ui_tx, UiEvent::LocalActive(true));
    } else {
        set_status(ui_tx, STATUS_REMOTE_ACTIVE);
        notify(ui_tx, UiEvent::LocalActive(false));
    }
}

/// The tracked keys resolved against the currently attached devices:
/// present devices keep their description, absent ones fall back to the key.
fn tracked_device_list(devices: &[UsbDevice], tracked: &[String]) -> Vec<TrackedDeviceInfo> {
    tracked
        .iter()
        .map(
            |key| match devices.iter().find(|device| device.matches_key(key)) {
                Some(device) => TrackedDeviceInfo {
                    name: device.description.clone(),
                    present: true,
                },
                None => TrackedDeviceInfo {
                    name: key.clone(),
                    present: false,
                },
            },
        )
        .collect()
}

/// Number of tracked keys that have an attached device.
fn count_present(devices: &[UsbDevice], tracked: &[String]) -> usize {
    tracked
        .iter()
        .filter(|key| devices.iter().any(|device| device.matches_key(key)))
        .count()
}

/// The unified switch routine (C# `SwitchInputAsync`).
fn switch_input(
    monitor: &dyn Monitor,
    state: &mut EngineState,
    ui_tx: &Sender<UiEvent>,
    input_source: u8,
    is_local: bool,
) {
    let label = if is_local { "local" } else { "remote" };

    if state.last_set_input == Some(input_source) {
        // The app believes the monitor is already there. Confirm it when the
        // display reports its input source: the user may have changed it with
        // the monitor's own controls, and trusting memory would then swallow
        // every later request for this input.
        match monitor.get_input_source() {
            Some(actual) if actual != input_source => {
                log::info!(
                    "Monitor reports {} although {} was expected; re-applying",
                    isrc::input_name(actual),
                    isrc::input_name(input_source)
                );
                state.last_set_input = None;
            }
            _ => {
                state.local_active = is_local;
                notify(ui_tx, UiEvent::LocalActive(is_local));
                set_status(
                    ui_tx,
                    format!(
                        "Monitor already on {} ({label})",
                        isrc::input_name(input_source)
                    ),
                );
                return;
            }
        }
    }

    set_status(
        ui_tx,
        format!("Switching monitor to {}...", isrc::input_name(input_source)),
    );
    log::info!(
        "Switching monitor to {} ({label})",
        isrc::input_name(input_source)
    );

    match monitor.set_input_source(input_source) {
        InputSwitchOutcome::Confirmed | InputSwitchOutcome::Unverified => {
            state.last_set_input = Some(input_source);
            state.local_active = is_local;
            state.last_switch_time = Some(Instant::now());
            notify(ui_tx, UiEvent::LocalActive(is_local));
            set_status(
                ui_tx,
                format!(
                    "Monitor set to {} ({label})",
                    isrc::input_name(input_source)
                ),
            );
            log::info!(
                "Monitor switched to {} ({label})",
                isrc::input_name(input_source)
            );
        }
        InputSwitchOutcome::Mismatch { actual } => {
            // Keep `last_set_input` unset so the next event tries again, and
            // report what the monitor actually shows instead of pretending
            // the switch worked.
            set_status(
                ui_tx,
                format!(
                    "Monitor did not switch (still on {})",
                    isrc::input_name(actual)
                ),
            );
            log::warn!(
                "Monitor reports {} after switching to {} ({label}); input not applied",
                isrc::input_name(actual),
                isrc::input_name(input_source)
            );
        }
        InputSwitchOutcome::Failed => {
            set_status(ui_tx, "Failed to switch monitor input");
            log::warn!(
                "Failed to switch monitor to {} ({label})",
                isrc::input_name(input_source)
            );
        }
    }
}

/// Decides whether a USB change implies a switch (C# `EvaluateState`).
fn evaluate_state(
    config: &SharedConfig,
    devices: &[UsbDevice],
    state: &mut EngineState,
    ui_tx: &Sender<UiEvent>,
    monitor: &dyn Monitor,
) {
    // Snapshot the settings once and release the read lock before any slow
    // DDC call: holding it would block the settings window's save.
    let (tracked, local_input, remote_input) = {
        let cfg = config.read();
        (
            cfg.tracked_device_keys.clone(),
            cfg.local_input_source,
            cfg.remote_input_source,
        )
    };

    // Always report the tracked-device list, even on the early-return
    // paths below, so the UI badge stays current.
    notify(
        ui_tx,
        UiEvent::TrackedDevices(tracked_device_list(devices, &tracked)),
    );

    if tracked.is_empty() {
        set_status(ui_tx, STATUS_NO_TRACKED_DEVICES);
        return;
    }

    let any_present = count_present(devices, &tracked) > 0;

    if state.is_pip_active() {
        // Auto-switch is suspended, but the active side must still follow
        // the devices: the tray icon/menu and the window badges read it,
        // and freezing here left them on the pre-PiP side for the whole
        // split-mode session.
        if state.local_active != any_present {
            state.local_active = any_present;
            notify(ui_tx, UiEvent::LocalActive(any_present));
        }
        log::debug!(
            "PiP/PBP active ({}), skipping automatic input switch",
            state.pip_mode.label()
        );
        set_status(ui_tx, pip_status_text(state));
        return;
    }

    if state.within_cooldown() {
        log::debug!(
            "Ignoring switch-to-{} request, within cooldown period",
            if any_present { "local" } else { "remote" }
        );
        return;
    }

    if any_present && !state.local_active {
        switch_input(monitor, state, ui_tx, local_input, true);
    } else if !any_present && state.local_active {
        switch_input(monitor, state, ui_tx, remote_input, false);
    } else {
        set_status(
            ui_tx,
            if any_present {
                STATUS_LOCAL_ACTIVE
            } else {
                STATUS_REMOTE_ACTIVE
            },
        );
    }
}

/// Queries the monitor's current PiP mode; failures only set the failed flag
/// (many displays don't implement the PiP VCP code).
fn refresh_pip_state(monitor: &dyn Monitor, state: &mut EngineState, ui_tx: &Sender<UiEvent>) {
    let supported = monitor.pip_supported();
    if supported != state.pip_supported {
        state.pip_supported = supported;
        notify(ui_tx, UiEvent::PipSupported(supported));
    }
    if !supported {
        // Standard DDC/CI has no usable PiP code (0xCC is OSD Language), so
        // there is nothing to read and nothing to suspend switching for.
        state.pip_query_failed = false;
        if state.pip_mode != PipMode::Off {
            state.pip_mode = PipMode::Off;
            notify(
                ui_tx,
                UiEvent::PipChanged {
                    mode: PipMode::Off,
                    query_failed: false,
                },
            );
        }
        return;
    }
    if let Some(mode) = monitor.get_pip_mode() {
        state.pip_query_failed = false;
        if mode != state.pip_mode {
            state.pip_mode = mode;
            match mode {
                PipMode::Unknown(wire) => log::warn!(
                    "Unrecognized PiP value 0x{wire:02X}; treating it as not active \
                     (auto-switch stays enabled)"
                ),
                _ => log::info!("PiP mode detected: {}", mode.label()),
            }
            notify(
                ui_tx,
                UiEvent::PipChanged {
                    mode,
                    query_failed: false,
                },
            );
        }
    } else {
        state.pip_query_failed = true;
        log::debug!("PiP mode query not supported on this display");
    }
}

fn set_pip_mode(
    config: &SharedConfig,
    monitor: &dyn Monitor,
    state: &mut EngineState,
    ui_tx: &Sender<UiEvent>,
    mode: PipMode,
) {
    if !monitor.pip_supported() {
        set_status(
            ui_tx,
            "PiP/PBP control is only available with the LG protocol",
        );
        log::info!("PiP mode change ignored: only the LG protocol implements PiP/PBP");
        return;
    }
    set_status(ui_tx, format!("Setting PiP mode to {}...", mode.label()));
    log::info!("Setting PiP mode to {}", mode.label());

    let success = monitor.set_pip_mode(mode);
    if success {
        let protocol = config.read().input_protocol;
        state.pip_mode = isrc::canonicalize_pip(protocol, mode);
        state.pip_query_failed = false;
        notify(
            ui_tx,
            UiEvent::PipChanged {
                mode: state.pip_mode,
                query_failed: false,
            },
        );
        set_status(
            ui_tx,
            if state.pip_mode.is_active() {
                pip_status_text(state)
            } else {
                "PiP off, auto-switch resumed".to_string()
            },
        );
        log::info!("PiP mode set to {}", state.pip_mode.label());
    } else {
        state.pip_query_failed = true;
        set_status(
            ui_tx,
            "Failed to set PiP mode (display may not support PiP over DDC/CI)",
        );
        log::warn!("Failed to set PiP mode to {}", mode.label());
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::mpsc::channel;

    use super::*;
    use crate::config::{AppConfig, InputSwitchProtocol};

    /// An in-memory [`Monitor`]: records writes and returns whatever the test
    /// asked for, so the engine's decisions can be checked without ddcutil or
    /// a real display.
    struct FakeMonitor {
        input: Cell<Option<u8>>,
        outcome: Cell<InputSwitchOutcome>,
        writes: RefCell<Vec<u8>>,
        pip: Cell<Option<PipMode>>,
        pip_reads: Cell<u32>,
        pip_supported: bool,
        accept_pip: bool,
        pip_writes: RefCell<Vec<PipMode>>,
        read_back_resets: Cell<u32>,
    }

    impl FakeMonitor {
        fn new() -> Self {
            Self {
                input: Cell::new(None),
                outcome: Cell::new(InputSwitchOutcome::Confirmed),
                writes: RefCell::new(Vec::new()),
                pip: Cell::new(None),
                pip_reads: Cell::new(0),
                pip_supported: true,
                accept_pip: true,
                pip_writes: RefCell::new(Vec::new()),
                read_back_resets: Cell::new(0),
            }
        }

        fn with_input(self, input: Option<u8>) -> Self {
            self.input.set(input);
            self
        }

        fn with_outcome(self, outcome: InputSwitchOutcome) -> Self {
            self.outcome.set(outcome);
            self
        }

        fn with_pip(self, mode: Option<PipMode>) -> Self {
            self.pip.set(mode);
            self
        }

        fn without_pip_support(self) -> Self {
            Self {
                pip_supported: false,
                ..self
            }
        }

        fn rejecting_pip_support(self) -> Self {
            Self {
                accept_pip: false,
                ..self
            }
        }

        fn writes(&self) -> Vec<u8> {
            self.writes.borrow().clone()
        }

        fn pip_writes(&self) -> Vec<PipMode> {
            self.pip_writes.borrow().clone()
        }
    }

    impl Monitor for FakeMonitor {
        fn set_input_source(&self, logical: u8) -> InputSwitchOutcome {
            self.writes.borrow_mut().push(logical);
            self.outcome.get()
        }

        fn get_input_source(&self) -> Option<u8> {
            self.input.get()
        }

        fn get_pip_mode(&self) -> Option<PipMode> {
            self.pip_reads.set(self.pip_reads.get() + 1);
            self.pip.get()
        }

        fn set_pip_mode(&self, mode: PipMode) -> bool {
            self.pip_writes.borrow_mut().push(mode);
            self.accept_pip
        }

        fn pip_supported(&self) -> bool {
            self.pip_supported
        }

        fn reset_read_back(&self) {
            self.read_back_resets.set(self.read_back_resets.get() + 1);
        }
    }

    fn config(protocol: InputSwitchProtocol, tracked: &[&str]) -> SharedConfig {
        SharedConfig::new(AppConfig {
            input_protocol: protocol,
            tracked_device_keys: tracked.iter().map(|key| (*key).to_string()).collect(),
            ..AppConfig::default()
        })
    }

    fn device(vendor: &str, product: &str, description: &str) -> UsbDevice {
        UsbDevice {
            vendor_id: vendor.into(),
            product_id: product.into(),
            description: description.into(),
        }
    }

    fn drain(rx: &Receiver<UiEvent>) -> Vec<UiEvent> {
        rx.try_iter().collect()
    }

    fn statuses(events: &[UiEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                UiEvent::Status(status) => Some(status.clone()),
                _ => None,
            })
            .collect()
    }

    fn last_status(events: &[UiEvent]) -> String {
        statuses(events).pop().unwrap_or_default()
    }

    fn local_active_events(events: &[UiEvent]) -> Vec<bool> {
        events
            .iter()
            .filter_map(|event| match event {
                UiEvent::LocalActive(active) => Some(*active),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn tracked_list_marks_presence_and_names() {
        let devices = vec![device("046d", "c52b", "USB Receiver")];
        let tracked = vec!["046D:C52B".to_string(), "1234:5678".to_string()];
        let list = tracked_device_list(&devices, &tracked);
        assert_eq!(
            list,
            vec![
                TrackedDeviceInfo {
                    name: "USB Receiver".into(),
                    present: true
                },
                TrackedDeviceInfo {
                    name: "1234:5678".into(),
                    present: false
                },
            ]
        );
        assert_eq!(count_present(&devices, &tracked), 1);
    }

    #[test]
    fn cooldown_expires() {
        let mut state = EngineState::new();
        assert!(!state.within_cooldown(), "fresh state is not cooling down");
        state.last_switch_time = Some(Instant::now());
        assert!(state.within_cooldown());
        state.last_switch_time =
            Instant::now().checked_sub(SWITCH_COOLDOWN + Duration::from_secs(1));
        assert!(!state.within_cooldown());
    }

    #[test]
    fn evaluate_switches_to_local_when_tracked_devices_appear() {
        let config = config(InputSwitchProtocol::Standard, &["046d:c52b"]);
        let monitor = FakeMonitor::new();
        let (tx, rx) = channel();
        let mut state = EngineState::new();
        state.local_active = false;

        let devices = vec![device("046d", "c52b", "USB Receiver")];
        evaluate_state(&config, &devices, &mut state, &tx, &monitor);

        assert_eq!(monitor.writes(), vec![isrc::DISPLAY_PORT]);
        assert!(state.local_active);
        assert_eq!(state.last_set_input, Some(isrc::DISPLAY_PORT));
        assert_eq!(local_active_events(&drain(&rx)), vec![true]);
    }

    #[test]
    fn evaluate_switches_to_remote_when_devices_disappear() {
        let config = config(InputSwitchProtocol::Standard, &["046d:c52b"]);
        let monitor = FakeMonitor::new();
        let (tx, rx) = channel();
        let mut state = EngineState::new();
        state.local_active = true;
        state.last_set_input = Some(isrc::DISPLAY_PORT);

        evaluate_state(&config, &[], &mut state, &tx, &monitor);

        assert_eq!(monitor.writes(), vec![isrc::HDMI1]);
        assert!(!state.local_active);
        assert_eq!(local_active_events(&drain(&rx)), vec![false]);
    }

    #[test]
    fn cooldown_blocks_a_switch() {
        let config = config(InputSwitchProtocol::Standard, &["046d:c52b"]);
        let monitor = FakeMonitor::new();
        let (tx, _rx) = channel();
        let mut state = EngineState::new();
        state.last_switch_time = Some(Instant::now());

        evaluate_state(
            &config,
            &[device("046d", "c52b", "USB Receiver")],
            &mut state,
            &tx,
            &monitor,
        );

        assert!(
            monitor.writes().is_empty(),
            "cooldown must suppress the write"
        );
        assert!(!state.local_active);
    }

    #[test]
    fn pip_mode_suspends_switching_but_follows_devices() {
        let config = config(InputSwitchProtocol::Lg, &["046d:c52b"]);
        let monitor = FakeMonitor::new().with_pip(Some(PipMode::Pbp));
        let (tx, rx) = channel();
        let mut state = EngineState::new();
        state.pip_mode = PipMode::Pbp;
        state.local_active = false;

        evaluate_state(
            &config,
            &[device("046d", "c52b", "USB Receiver")],
            &mut state,
            &tx,
            &monitor,
        );

        assert!(monitor.writes().is_empty(), "PiP must suspend switching");
        assert!(
            state.local_active,
            "the active side still follows the devices"
        );
        assert!(last_status(&drain(&rx)).contains("PiP/PBP active"));
    }

    #[test]
    fn unknown_pip_value_does_not_suspend_switching() {
        let config = config(InputSwitchProtocol::Lg, &["046d:c52b"]);
        let monitor = FakeMonitor::new().with_pip(Some(PipMode::Unknown(0x03)));
        let (tx, _rx) = channel();
        let mut state = EngineState::new();
        state.pip_mode = PipMode::Unknown(0x03);
        state.local_active = false;

        evaluate_state(
            &config,
            &[device("046d", "c52b", "USB Receiver")],
            &mut state,
            &tx,
            &monitor,
        );

        assert_eq!(monitor.writes(), vec![isrc::DISPLAY_PORT]);
        assert!(state.local_active);
    }

    #[test]
    fn no_tracked_devices_reports_configuration() {
        let config = config(InputSwitchProtocol::Standard, &[]);
        let monitor = FakeMonitor::new();
        let (tx, rx) = channel();
        let mut state = EngineState::new();

        evaluate_state(&config, &[], &mut state, &tx, &monitor);

        assert!(monitor.writes().is_empty());
        assert!(last_status(&drain(&rx)).contains("No tracked devices configured"));
    }

    #[test]
    fn already_on_input_is_not_rewritten() {
        let monitor = FakeMonitor::new().with_input(Some(isrc::DISPLAY_PORT));
        let (tx, rx) = channel();
        let mut state = EngineState::new();
        state.last_set_input = Some(isrc::DISPLAY_PORT);

        switch_input(&monitor, &mut state, &tx, isrc::DISPLAY_PORT, true);

        assert!(monitor.writes().is_empty());
        assert!(state.local_active);
        assert!(last_status(&drain(&rx)).contains("already on"));
    }

    #[test]
    fn manual_input_change_is_detected_and_reapplied() {
        // The monitor reports a different input than the app remembers: the
        // user changed it with the monitor's own controls.
        let monitor = FakeMonitor::new().with_input(Some(isrc::HDMI1));
        let (tx, _rx) = channel();
        let mut state = EngineState::new();
        state.last_set_input = Some(isrc::DISPLAY_PORT);

        switch_input(&monitor, &mut state, &tx, isrc::DISPLAY_PORT, true);

        assert_eq!(monitor.writes(), vec![isrc::DISPLAY_PORT]);
        assert_eq!(state.last_set_input, Some(isrc::DISPLAY_PORT));
    }

    #[test]
    fn unreadable_input_falls_back_to_memory() {
        // Displays without a readable 0x60 must not be re-switched on every call.
        let monitor = FakeMonitor::new().with_input(None);
        let (tx, _rx) = channel();
        let mut state = EngineState::new();
        state.last_set_input = Some(isrc::DISPLAY_PORT);

        switch_input(&monitor, &mut state, &tx, isrc::DISPLAY_PORT, true);

        assert!(monitor.writes().is_empty());
    }

    #[test]
    fn mismatch_is_reported_and_not_recorded() {
        let monitor = FakeMonitor::new().with_outcome(InputSwitchOutcome::Mismatch {
            actual: isrc::HDMI1,
        });
        let (tx, rx) = channel();
        let mut state = EngineState::new();

        switch_input(&monitor, &mut state, &tx, isrc::DISPLAY_PORT, true);

        assert_eq!(state.last_set_input, None);
        assert!(state.last_switch_time.is_none());
        assert!(last_status(&drain(&rx)).contains("did not switch"));
    }

    #[test]
    fn unverified_switch_counts_as_success() {
        let monitor = FakeMonitor::new().with_outcome(InputSwitchOutcome::Unverified);
        let (tx, _rx) = channel();
        let mut state = EngineState::new();

        switch_input(&monitor, &mut state, &tx, isrc::DISPLAY_PORT, true);

        assert_eq!(state.last_set_input, Some(isrc::DISPLAY_PORT));
        assert!(state.last_switch_time.is_some());
        assert!(state.local_active);
    }

    #[test]
    fn failed_write_is_reported() {
        let monitor = FakeMonitor::new().with_outcome(InputSwitchOutcome::Failed);
        let (tx, rx) = channel();
        let mut state = EngineState::new();

        switch_input(&monitor, &mut state, &tx, isrc::DISPLAY_PORT, true);

        assert_eq!(state.last_set_input, None);
        assert!(last_status(&drain(&rx)).contains("Failed to switch"));
    }

    #[test]
    fn refresh_pip_state_reports_unknown_value() {
        let monitor = FakeMonitor::new().with_pip(Some(PipMode::Unknown(0x03)));
        let (tx, rx) = channel();
        let mut state = EngineState::new();

        refresh_pip_state(&monitor, &mut state, &tx);

        assert_eq!(state.pip_mode, PipMode::Unknown(0x03));
        assert!(!state.is_pip_active());
        let events = drain(&rx);
        assert!(events.iter().any(|event| matches!(
            event,
            UiEvent::PipChanged {
                mode: PipMode::Unknown(0x03),
                query_failed: false
            }
        )));
    }

    #[test]
    fn unsupported_protocol_skips_pip_reads() {
        let monitor = FakeMonitor::new()
            .without_pip_support()
            .with_pip(Some(PipMode::Pbp));
        let (tx, rx) = channel();
        let mut state = EngineState::new();
        state.pip_mode = PipMode::Pbp;

        refresh_pip_state(&monitor, &mut state, &tx);

        assert_eq!(
            monitor.pip_reads.get(),
            0,
            "no PiP register on standard DDC/CI"
        );
        assert_eq!(state.pip_mode, PipMode::Off);
        assert!(!state.pip_query_failed);
        // `EngineState` starts unsupported, so an unsupported protocol emits
        // no support event at all; the UI is initialized from the config.
        assert!(drain(&rx)
            .iter()
            .all(|event| !matches!(event, UiEvent::PipSupported(true))));
    }

    #[test]
    fn pip_query_failure_sets_the_flag() {
        let monitor = FakeMonitor::new().with_pip(None);
        let (tx, _rx) = channel();
        let mut state = EngineState::new();

        refresh_pip_state(&monitor, &mut state, &tx);

        assert!(state.pip_query_failed);
    }

    #[test]
    fn set_pip_mode_canonicalizes_for_lg() {
        let config = config(InputSwitchProtocol::Lg, &[]);
        let monitor = FakeMonitor::new();
        let (tx, _rx) = channel();
        let mut state = EngineState::new();

        set_pip_mode(&config, &monitor, &mut state, &tx, PipMode::Pip);

        assert_eq!(monitor.pip_writes(), vec![PipMode::Pip]);
        // LG reports PiP and PBP with the same wire value.
        assert_eq!(state.pip_mode, PipMode::Pbp);
    }

    #[test]
    fn set_pip_mode_refuses_standard_protocol() {
        let config = config(InputSwitchProtocol::Standard, &[]);
        let monitor = FakeMonitor::new().without_pip_support();
        let (tx, rx) = channel();
        let mut state = EngineState::new();

        set_pip_mode(&config, &monitor, &mut state, &tx, PipMode::Pip);

        assert!(monitor.pip_writes().is_empty());
        assert!(last_status(&drain(&rx)).contains("only available with the LG protocol"));
    }

    #[test]
    fn set_pip_mode_failure_is_reported() {
        let config = config(InputSwitchProtocol::Lg, &[]);
        let monitor = FakeMonitor::new().rejecting_pip_support();
        let (tx, rx) = channel();
        let mut state = EngineState::new();

        set_pip_mode(&config, &monitor, &mut state, &tx, PipMode::Pip);

        assert!(state.pip_query_failed);
        assert!(last_status(&drain(&rx)).contains("Failed to set PiP mode"));
    }

    #[test]
    fn init_state_reports_tracked_presence() {
        let config = config(InputSwitchProtocol::Standard, &["046d:c52b"]);
        let (tx, rx) = channel();

        init_state(&config, &[device("046d", "c52b", "USB Receiver")], &tx);

        let events = drain(&rx);
        assert!(matches!(events.first(), Some(UiEvent::TrackedDevices(_))));
        assert_eq!(local_active_events(&events), vec![true]);
        assert!(statuses(&events)
            .iter()
            .any(|status| status.contains("Local machine active")));
    }

    #[test]
    fn stop_command_stops_the_engine() {
        let config = config(InputSwitchProtocol::Standard, &[]);
        let monitor = FakeMonitor::new();
        let (tx, _rx) = channel();
        let mut state = EngineState::new();

        assert!(!handle_command(
            &config,
            &monitor,
            &mut state,
            &tx,
            EngineCommand::Stop
        ));
    }

    #[test]
    fn config_change_resets_read_back_and_reports() {
        let config = config(InputSwitchProtocol::Lg, &[]);
        let monitor = FakeMonitor::new();
        let (tx, rx) = channel();
        let mut state = EngineState::new();

        assert!(handle_command(
            &config,
            &monitor,
            &mut state,
            &tx,
            EngineCommand::ConfigChanged
        ));

        assert_eq!(monitor.read_back_resets.get(), 1);
        assert!(last_status(&drain(&rx)).contains("No tracked devices"));
    }

    #[test]
    fn manual_switch_commands_use_the_configured_inputs() {
        let config = config(InputSwitchProtocol::Standard, &[]);
        let monitor = FakeMonitor::new();
        let (tx, _rx) = channel();
        let mut state = EngineState::new();

        handle_command(
            &config,
            &monitor,
            &mut state,
            &tx,
            EngineCommand::SwitchToLocal,
        );
        handle_command(
            &config,
            &monitor,
            &mut state,
            &tx,
            EngineCommand::SwitchToRemote,
        );

        assert_eq!(monitor.writes(), vec![isrc::DISPLAY_PORT, isrc::HDMI1]);
        assert!(!state.local_active);
    }

    #[test]
    fn toggle_pip_alternates_the_mode() {
        let config = config(InputSwitchProtocol::Lg, &[]);
        let monitor = FakeMonitor::new();
        let (tx, _rx) = channel();
        let mut state = EngineState::new();

        handle_command(&config, &monitor, &mut state, &tx, EngineCommand::TogglePip);
        assert_eq!(monitor.pip_writes(), vec![PipMode::Pip]);
        // LG reports PiP and PBP with the same wire value.
        assert_eq!(state.pip_mode, PipMode::Pbp);

        handle_command(&config, &monitor, &mut state, &tx, EngineCommand::TogglePip);
        assert_eq!(monitor.pip_writes(), vec![PipMode::Pip, PipMode::Off]);
        assert_eq!(state.pip_mode, PipMode::Off);
    }
}
