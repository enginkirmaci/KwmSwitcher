//! App bootstrap: the gpui application, the global UI bridge that pumps
//! engine events onto the UI thread, and the window-opening helpers.

mod assets;
mod main_window;
mod settings_window;

use std::sync::mpsc::Receiver;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use gpui_kit::component::theme::{Colorize, Theme};
use gpui_kit::component::{Root, TitleBar};
use gpui_kit::*;

pub use main_window::MainWindowView;
pub use settings_window::SettingsWindowView;

use crate::config::{AppConfig, SharedConfig};
use crate::ddc::Ddc;
use crate::engine::{self, EngineHandle, TrackedDeviceInfo, UiEvent};
use crate::input_source as isrc;
use crate::tray::{self, TrayHandle, UiCommand};

/// Everything the UI needs, installed as a gpui global.
pub struct Bridge {
    pub config: SharedConfig,
    pub ddc: Arc<Ddc>,
    pub engine: EngineHandle,
    /// Engine → UI events.
    pub ui_rx: Receiver<UiEvent>,
    /// Tray thread → UI commands.
    pub ui_cmd_rx: Receiver<UiCommand>,
    pub tray: Option<TrayHandle>,
    pub state: UiState,
    pub main_window: Option<AnyWindowHandle>,
    pub main_view: Option<WeakEntity<MainWindowView>>,
    pub settings_window: Option<AnyWindowHandle>,
}

impl Global for Bridge {}

/// Live UI state mirrored from the engine.
#[derive(Debug, Clone, PartialEq)]
pub struct UiState {
    pub status: String,
    pub local_active: bool,
    pub pip_mode: u8,
    pub pip_query_failed: bool,
    /// Tracked USB devices currently attached.
    pub tracked_present: usize,
    /// Every tracked device with its attach state, for the hover list.
    pub tracked_devices: Vec<TrackedDeviceInfo>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            status: "Initializing...".into(),
            local_active: false,
            pip_mode: isrc::PIP_OFF,
            pip_query_failed: false,
            tracked_present: 0,
            tracked_devices: Vec::new(),
        }
    }
}

impl UiState {
    pub fn is_pip_active(&self) -> bool {
        isrc::is_pip_active(self.pip_mode)
    }
}

/// Loads the logo for the X11 window icon.
pub fn logo_rgba() -> Option<Arc<image::RgbaImage>> {
    let png = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo.png"));
    image::load_from_memory(png)
        .ok()
        .map(|img| Arc::new(img.to_rgba8()))
}

/// Entry point for the GUI app.
pub fn run() {
    let config: SharedConfig = Arc::new(RwLock::new(AppConfig::load()));
    {
        let cfg = config.read().unwrap();
        crate::autostart::reconcile(&cfg);
    }
    tray::install_icon();

    let ddc = Ddc::new(config.clone());
    let (engine, ui_rx, _engine_tx) = engine::spawn(config.clone(), ddc.clone());

    // The poll thread runs for the process lifetime; it stops when the
    // process exits (or the engine's channel closes).
    let usb_monitor = crate::usb::UsbMonitor::spawn(config.clone(), engine.tx.clone());
    std::mem::forget(usb_monitor);

    // Tray before the gpui event loop: spawn() makes blocking DBus calls and
    // must not stall first paint. Window opening is requested via UiCommand.
    let (ui_cmd_tx, ui_cmd_rx) = std::sync::mpsc::channel::<UiCommand>();
    let tray_handle = tray::spawn(engine.tx.clone(), ui_cmd_tx);

    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            sync_theme(None, cx);

            let start_minimized = config.read().unwrap().start_minimized;
            cx.set_global(Bridge {
                config: config.clone(),
                ddc: ddc.clone(),
                engine,
                ui_rx,
                ui_cmd_rx,
                tray: tray_handle,
                state: UiState::default(),
                main_window: None,
                main_view: None,
                settings_window: None,
            });

            start_event_pump(cx);

            if !start_minimized {
                open_main_window(cx);
            }
        });
}

/// Follow the system light/dark setting, then apply our accent + radius.
///
/// On Linux the color scheme arrives asynchronously from the XDG desktop
/// portal, so this must be re-run when window appearance changes.
pub(crate) fn sync_theme(window: Option<&mut Window>, cx: &mut App) {
    Theme::sync_system_appearance(window, cx);
    apply_theme_overrides(cx);
}

/// Our look on top of the system theme. Always run after a theme sync, which
/// resets colors to the registry defaults.
pub(crate) fn apply_theme_overrides(cx: &mut App) {
    let theme = Theme::global_mut(cx);
    let accent: gpui::Hsla = gpui::rgb(0x6E56CF).into();
    let accent_hover = accent.lightness(0.62);
    let accent_active = accent.lightness(0.46);
    theme.accent = accent;
    theme.primary = accent;
    theme.primary_foreground = gpui::rgb(0xFFFFFF).into();
    theme.primary_hover = accent_hover;
    theme.primary_active = accent_active;
    // Button variants read their own `button_*` colors (not `primary`), and
    // render from the tokens snapshot — set both.
    theme.button_primary = accent;
    theme.button_primary_hover = accent_hover;
    theme.button_primary_active = accent_active;
    theme.button_primary_foreground = gpui::rgb(0xFFFFFF).into();
    theme.tokens = gpui_kit::component::theme::ThemeTokens::from(&theme.colors);
    theme.radius = gpui::px(8.);
    theme.radius_lg = gpui::px(12.);
    Theme::sync_base(cx);
}

/// Opens the settings window after prefetching the monitor list.
pub(crate) fn request_open_settings(cx: &mut App) {
    let (ddc, config) = {
        let bridge = cx.global::<Bridge>();
        (bridge.ddc.clone(), bridge.config.clone())
    };
    spawn_settings_prefetch(cx, ddc, config);
}

/// Drains engine events + tray commands onto the UI thread.
fn start_event_pump(cx: &mut App) {
    cx.spawn(async move |cx| {
        // Propagation is guarded by this: cx.notify() re-renders the window
        // and a ksni update re-flattens the whole tray menu, both far too
        // costly to repeat at a 60 ms tick when nothing has changed.
        let mut last_seen: Option<UiState> = None;
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(60))
                .await;

            let mut quit_requested = false;
            // AsyncApp::update panics if the app is gone, which is fine: the
            // executor drops this task during shutdown.
            cx.update(|cx| {
                // 1. Fold engine events into the bridge state.
                {
                    let bridge = cx.global_mut::<Bridge>();
                    while let Ok(event) = bridge.ui_rx.try_recv() {
                        match event {
                            UiEvent::Status(status) => bridge.state.status = status,
                            UiEvent::LocalActive(active) => bridge.state.local_active = active,
                            UiEvent::PipChanged { mode, query_failed } => {
                                bridge.state.pip_mode = mode;
                                bridge.state.pip_query_failed = query_failed;
                            }
                            UiEvent::TrackedDevices(devices) => {
                                bridge.state.tracked_devices = devices.clone();
                                bridge.state.tracked_present =
                                    devices.iter().filter(|d| d.present).count();
                            }
                        }
                    }
                }

                // 2. Snapshot and propagate to the view + tray, but only when
                // the state actually changed.
                let (state, tray, main_view) = {
                    let bridge = cx.global::<Bridge>();
                    (
                        bridge.state.clone(),
                        bridge.tray.clone(),
                        bridge.main_view.clone(),
                    )
                };
                if last_seen.as_ref() != Some(&state) {
                    if let Some(view) = main_view.and_then(|v| v.upgrade()) {
                        let _ = view.update(cx, |view, cx| {
                            view.sync_state(&state);
                            cx.notify();
                        });
                    }
                    if let Some(tray) = tray {
                        tray.update_state(|t| {
                            t.status = state.status.clone();
                            t.local_active = state.local_active;
                            t.has_tracked = !state.tracked_devices.is_empty();
                            t.pip_active = state.is_pip_active();
                            t.pip_label = isrc::pip_mode_name(state.pip_mode);
                        });
                    }
                    last_seen = Some(state);
                }

                // 3. Drain tray commands (they may open windows).
                let commands = {
                    let bridge = cx.global_mut::<Bridge>();
                    let mut commands = Vec::new();
                    while let Ok(cmd) = bridge.ui_cmd_rx.try_recv() {
                        commands.push(cmd);
                    }
                    commands
                };
                for cmd in commands {
                    match cmd {
                        UiCommand::OpenMainWindow => open_main_window(cx),
                        UiCommand::OpenSettings => {
                            let (ddc, config) = {
                                let bridge = cx.global::<Bridge>();
                                (bridge.ddc.clone(), bridge.config.clone())
                            };
                            spawn_settings_prefetch(cx, ddc, config);
                        }
                        UiCommand::Quit => {
                            let engine = cx.global::<Bridge>().engine.clone();
                            engine.stop();
                            quit_requested = true;
                        }
                    }
                }
            });
            if quit_requested {
                log::info!("Quit requested from tray");
                cx.update(|cx| cx.quit());
                break;
            }
        }
    })
    .detach();
}

/// `ddcutil detect` is slow; the monitor list is prefetched on the thread
/// pool inside [`request_open_settings`] before the modal opens.

/// Opens (or activates) the main window.
pub fn open_main_window(cx: &mut App) {
    let existing = cx.global::<Bridge>().main_window;
    if let Some(handle) = existing {
        // A failed update means the window was closed; fall through to reopen.
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }

    let (engine, state) = {
        let bridge = cx.global::<Bridge>();
        (bridge.engine.clone(), bridge.state.clone())
    };
    let logo = logo_rgba();

    let bounds = WindowBounds::Windowed(Bounds::centered(None, size(px(820.), px(460.)), cx));
    let options = WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(size(px(740.), px(420.))),
        app_id: Some("kwmswitcher".into()),
        icon: logo,
        titlebar: Some(TitlebarOptions {
            title: Some("KWM Switcher".into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    };

    let mut view_slot: Option<WeakEntity<MainWindowView>> = None;
    let handle = cx
        .open_window(options, |window, cx| {
            sync_theme(Some(window), cx);
            let view = cx.new(|cx| MainWindowView::new(engine.clone(), state.clone(), window, cx));
            view_slot = Some(view.downgrade());
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open main window");
    let _ = handle.update(cx, |_, window, _| window.activate_window());

    let bridge = cx.global_mut::<Bridge>();
    bridge.main_window = Some(handle.into());
    bridge.main_view = view_slot;
}

/// `ddcutil detect` is slow; run it on the thread pool, then open settings.
fn spawn_settings_prefetch(cx: &mut App, ddc: Arc<Ddc>, config: SharedConfig) {
    cx.spawn(async move |cx| {
        let monitors = cx
            .background_executor()
            .spawn(async move { ddc.available_monitors() })
            .await;
        let _ = cx.update(|cx| open_settings_window(cx, monitors, config));
    })
    .detach();
}

/// Opens the settings window (single instance). `monitors` comes from the
/// background `ddcutil detect` prefetch.
pub fn open_settings_window(cx: &mut App, monitors: Vec<String>, config: SharedConfig) {
    let existing = cx.global::<Bridge>().settings_window;
    if let Some(handle) = existing {
        if handle.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }

    let ddc = cx.global::<Bridge>().ddc.clone();
    let bounds = WindowBounds::Windowed(Bounds::centered(None, size(px(720.), px(640.)), cx));
    let options = WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(size(px(640.), px(520.))),
        app_id: Some("kwmswitcher".into()),
        icon: logo_rgba(),
        titlebar: Some(TitlebarOptions {
            // Must stay exactly "KWM Switcher": the session's Hyprland float
            // rule matches this title, otherwise the settings window gets
            // tiled to screen size.
            title: Some("KWM Switcher".into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    };

    let handle = cx
        .open_window(options, |window, cx| {
            sync_theme(Some(window), cx);
            let view = cx.new(|cx| SettingsWindowView::new(config, ddc, monitors, window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open settings window");

    cx.global_mut::<Bridge>().settings_window = Some(handle.into());
}
