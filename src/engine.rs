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
use crate::ddc::Ddc;
use crate::input_source as isrc;
use crate::usb::UsbDevice;

#[derive(Clone)]
pub enum EngineCommand {
    DevicesChanged(Vec<UsbDevice>),
    SwitchToLocal,
    SwitchToRemote,
    TogglePip,
    RefreshPip,
    Stop,
}

#[derive(Debug, Clone)]
pub enum UiEvent {
    Status(String),
    LocalActive(bool),
    PipChanged { mode: u8, query_failed: bool },
}

const SWITCH_COOLDOWN: Duration = Duration::from_secs(3);

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

    pub fn toggle_pip(&self) {
        let _ = self.tx.send(EngineCommand::TogglePip);
    }

    pub fn refresh_pip(&self) {
        let _ = self.tx.send(EngineCommand::RefreshPip);
    }

    pub fn stop(&self) {
        let _ = self.tx.send(EngineCommand::Stop);
    }
}

struct EngineState {
    local_active: bool,
    pip_mode: u8,
    pip_query_failed: bool,
    last_set_input: Option<u8>,
    last_switch_time: Option<Instant>,
}

impl EngineState {
    fn new() -> Self {
        Self {
            local_active: false,
            pip_mode: isrc::PIP_OFF,
            pip_query_failed: false,
            last_set_input: None,
            last_switch_time: None,
        }
    }

    fn is_pip_active(&self) -> bool {
        isrc::is_pip_active(self.pip_mode)
    }

    fn within_cooldown(&self) -> bool {
        self.last_switch_time
            .is_some_and(|t| t.elapsed() < SWITCH_COOLDOWN)
    }
}

/// Spawns the engine thread; returns a command handle plus the UI event queue.
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
    // Keep the thread alive independently; the loop exits on EngineCommand::Stop.
    std::mem::forget(join);

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

fn run(
    config: SharedConfig,
    ddc: Arc<Ddc>,
    cmd_rx: Receiver<EngineCommand>,
    ui_tx: Sender<UiEvent>,
) {
    let mut state = EngineState::new();

    // Seed the initial state from the current device set.
    let devices = crate::usb::list_devices();
    init_state(&config, &devices, &ui_tx);
    refresh_pip_state(&ddc, &mut state, &ui_tx);

    loop {
        let command = match cmd_rx.recv() {
            Ok(cmd) => cmd,
            Err(_) => break, // all senders dropped
        };
        match command {
            EngineCommand::Stop => break,
            EngineCommand::DevicesChanged(devices) => {
                refresh_pip_state(&ddc, &mut state, &ui_tx);
                evaluate_state(&config, &devices, &mut state, &ui_tx, &ddc);
            }
            EngineCommand::SwitchToLocal => {
                let target = config.read().unwrap().local_input_source;
                switch_input(&ddc, &mut state, &ui_tx, target, true);
            }
            EngineCommand::SwitchToRemote => {
                let target = config.read().unwrap().remote_input_source;
                switch_input(&ddc, &mut state, &ui_tx, target, false);
            }
            EngineCommand::TogglePip => {
                let target = if state.is_pip_active() {
                    isrc::PIP_OFF
                } else {
                    isrc::PIP_ON
                };
                set_pip_mode(&config, &ddc, &mut state, &ui_tx, target);
            }
            EngineCommand::RefreshPip => {
                refresh_pip_state(&ddc, &mut state, &ui_tx);
            }
        }
    }
    log::debug!("Engine thread exiting");
}

/// Mirrors the C# `InitState`: derive which side is active from device presence.
fn init_state(config: &SharedConfig, devices: &[UsbDevice], ui_tx: &Sender<UiEvent>) {
    let cfg = config.read().unwrap();
    let tracked: Vec<String> = cfg
        .tracked_device_keys
        .iter()
        .map(|k| k.to_lowercase())
        .collect();

    if tracked.is_empty() {
        set_status(ui_tx, "No tracked devices configured. Open settings to select USB devices.");
        return;
    }

    let any_present = devices
        .iter()
        .any(|d| tracked.contains(&d.key().to_lowercase()));
    if any_present {
        set_status(ui_tx, "Local machine active, tracked USB devices detected");
        notify(ui_tx, UiEvent::LocalActive(true));
    } else {
        set_status(ui_tx, "Remote machine active, no tracked USB devices");
        notify(ui_tx, UiEvent::LocalActive(false));
    }
}

/// The unified switch routine (C# `SwitchInputAsync`).
fn switch_input(
    ddc: &Arc<Ddc>,
    state: &mut EngineState,
    ui_tx: &Sender<UiEvent>,
    input_source: u8,
    is_local: bool,
) {
    let label = if is_local { "local" } else { "remote" };

    if state.last_set_input == Some(input_source) {
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

    set_status(
        ui_tx,
        format!("Switching monitor to {}...", isrc::input_name(input_source)),
    );
    log::info!(
        "Switching monitor to {} ({label})",
        isrc::input_name(input_source)
    );

    let success = ddc.set_input_source(input_source);
    if success {
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
    } else {
        set_status(ui_tx, "Failed to switch monitor input");
        log::warn!(
            "Failed to switch monitor to {} ({label})",
            isrc::input_name(input_source)
        );
    }
}

/// Decides whether a USB change implies a switch (C# `EvaluateState`).
fn evaluate_state(
    config: &SharedConfig,
    devices: &[UsbDevice],
    state: &mut EngineState,
    ui_tx: &Sender<UiEvent>,
    ddc: &Arc<Ddc>,
) {
    let cfg = config.read().unwrap();
    let tracked: Vec<String> = cfg
        .tracked_device_keys
        .iter()
        .map(|k| k.to_lowercase())
        .collect();

    if tracked.is_empty() {
        set_status(ui_tx, "No tracked devices configured. Open settings to select USB devices.");
        return;
    }

    if state.is_pip_active() {
        log::debug!(
            "PiP/PBP active ({}), skipping automatic input switch",
            isrc::pip_mode_name(state.pip_mode)
        );
        set_status(
            ui_tx,
            format!(
                "PiP/PBP active ({}), auto-switch suspended",
                isrc::pip_mode_name(state.pip_mode)
            ),
        );
        return;
    }
    drop(cfg);

    let any_present = devices
        .iter()
        .any(|d| tracked.contains(&d.key().to_lowercase()));

    if state.within_cooldown() {
        log::debug!(
            "Ignoring switch-to-{} request, within cooldown period",
            if any_present { "local" } else { "remote" }
        );
        return;
    }

    if any_present && !state.local_active {
        let target = config.read().unwrap().local_input_source;
        switch_input(ddc, state, ui_tx, target, true);
    } else if !any_present && state.local_active {
        let target = config.read().unwrap().remote_input_source;
        switch_input(ddc, state, ui_tx, target, false);
    } else {
        set_status(
            ui_tx,
            if any_present {
                "Local machine active, tracked USB devices detected"
            } else {
                "Remote machine active, no tracked USB devices"
            },
        );
    }
}

/// Queries the monitor's current PiP mode; failures only set the failed flag
/// (many displays don't implement the PiP VCP code).
fn refresh_pip_state(
    ddc: &Arc<Ddc>,
    state: &mut EngineState,
    ui_tx: &Sender<UiEvent>,
) {
    match ddc.get_pip_mode() {
        Some(mode) => {
            state.pip_query_failed = false;
            if mode != state.pip_mode {
                state.pip_mode = mode;
                log::info!("PiP mode detected: {}", isrc::pip_mode_name(mode));
                notify(
                    ui_tx,
                    UiEvent::PipChanged {
                        mode,
                        query_failed: false,
                    },
                );
            }
        }
        None => {
            state.pip_query_failed = true;
            log::debug!("PiP mode query not supported on this display");
        }
    }
}

fn set_pip_mode(
    config: &SharedConfig,
    ddc: &Arc<Ddc>,
    state: &mut EngineState,
    ui_tx: &Sender<UiEvent>,
    mode: u8,
) {
    set_status(
        ui_tx,
        format!("Setting PiP mode to {}...", isrc::pip_mode_name(mode)),
    );
    log::info!("Setting PiP mode to {}", isrc::pip_mode_name(mode));

    let success = ddc.set_pip_mode(mode);
    if success {
        state.pip_mode = isrc::canonicalize_pip(
            config.read().unwrap().input_protocol,
            mode,
        );
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
            if isrc::is_pip_active(state.pip_mode) {
                format!(
                    "PiP/PBP active ({}), auto-switch suspended",
                    isrc::pip_mode_name(state.pip_mode)
                )
            } else {
                "PiP off, auto-switch resumed".to_string()
            },
        );
        log::info!("PiP mode set to {}", isrc::pip_mode_name(state.pip_mode));
    } else {
        state.pip_query_failed = true;
        set_status(
            ui_tx,
            "Failed to set PiP mode (display may not support PiP over DDC/CI)",
        );
        log::warn!("Failed to set PiP mode to {}", isrc::pip_mode_name(mode));
    }
}
