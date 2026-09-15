//! System tray icon via the FreeDesktop StatusNotifierItem protocol (ksni).
//!
//! The tray lives on its own thread with a DBus connection; it talks to the
//! engine through the command queue and to the gpui UI through a small
//! [`UiCommand`] queue (opening windows must happen on the UI thread).

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use ksni::blocking::TrayMethods;
use ksni::menu::StandardItem;
use ksni::{Category, Icon, MenuItem, Status, ToolTip, Tray};

use crate::engine::EngineCommand;

/// Commands the tray posts to the gpui UI thread.
#[derive(Debug, Clone)]
pub enum UiCommand {
    OpenMainWindow,
    OpenSettings,
    Quit,
}

/// What the menu displays; updated by the UI event pump.
#[derive(Debug, Clone, Default)]
pub struct TrayState {
    pub status: String,
    pub local_active: bool,
    /// False while no tracked devices are configured: the app has never
    /// switched, so the active side is unknown.
    pub has_tracked: bool,
    pub pip_active: bool,
    pub pip_label: String,
}

/// One RGBA variant per connection side; `icon_pixmap` picks by state.
struct TrayIcons {
    /// Brand logo, shown before the first switch (side unknown).
    unknown: Vec<u8>,
    local: Vec<u8>,
    remote: Vec<u8>,
    size: u32,
}

struct KwmTray {
    engine_tx: Sender<EngineCommand>,
    ui_tx: Sender<UiCommand>,
    state: Arc<Mutex<TrayState>>,
    icons: TrayIcons,
}

/// Local = white, remote = green (readable on dark panels).
const LOCAL_TINT: [u8; 3] = [0xF9, 0xFA, 0xFB];
const REMOTE_TINT: [u8; 3] = [0x22, 0xC5, 0x5E];

fn menu_item(
    label: &str,
    engine_tx: &Sender<EngineCommand>,
    ui_tx: &Sender<UiCommand>,
    on_click: MenuAction,
) -> StandardItem<KwmTray> {
    let engine_tx = engine_tx.clone();
    let ui_tx = ui_tx.clone();
    StandardItem {
        label: label.to_string(),
        activate: Box::new(move |_| match on_click.clone() {
            MenuAction::Engine(cmd) => {
                let _ = engine_tx.send(cmd);
            }
            MenuAction::Ui(cmd) => {
                let _ = ui_tx.send(cmd);
            }
        }),
        ..Default::default()
    }
}

#[derive(Clone)]
enum MenuAction {
    Engine(EngineCommand),
    Ui(UiCommand),
}

impl Tray for KwmTray {
    fn id(&self) -> String {
        "kwmswitcher".into()
    }

    fn icon_theme_path(&self) -> String {
        String::new()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let state = self.state.lock().map(|s| s.clone()).unwrap_or_default();
        let icon_rgba = match state.has_tracked {
            true if state.local_active => &self.icons.local,
            true => &self.icons.remote,
            false => &self.icons.unknown,
        };
        // ARGB32, network byte order.
        let mut argb = Vec::with_capacity(icon_rgba.len());
        for px in icon_rgba.chunks_exact(4) {
            argb.push(px[3]); // A
            argb.push(px[0]); // R
            argb.push(px[1]); // G
            argb.push(px[2]); // B
        }
        vec![Icon {
            width: self.icons.size as i32,
            height: self.icons.size as i32,
            data: argb,
        }]
    }

    fn title(&self) -> String {
        "KWM Switcher".into()
    }

    fn status(&self) -> Status {
        // Active, not Passive: some hosts (e.g. Omarchy's Quickshell shell)
        // hide passive items entirely, which made the icon vanish.
        Status::Active
    }

    /// A left click on the icon opens the main window; the context menu
    /// stays on right click (`MENU_ON_ACTIVATE` is false, so activation is
    /// not hijacked into opening the menu).
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.ui_tx.send(UiCommand::OpenMainWindow);
    }

    fn category(&self) -> Category {
        Category::Hardware
    }

    fn tool_tip(&self) -> ToolTip {
        let state = self.state.lock().map(|s| s.clone()).unwrap_or_default();
        ToolTip {
            icon_name: String::new(),
            icon_pixmap: Vec::new(),
            title: "KWM Switcher".into(),
            description: state.status,
        }
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let state = self.state.lock().map(|s| s.clone()).unwrap_or_default();
        let side = match state.has_tracked {
            true if state.local_active => "Local",
            true => "Remote",
            // Nothing configured: the app has not switched yet.
            false => "Unknown",
        };

        vec![
            MenuItem::Standard(menu_item("Main Window", &self.engine_tx, &self.ui_tx, MenuAction::Ui(UiCommand::OpenMainWindow))),
            MenuItem::Standard(menu_item("Settings", &self.engine_tx, &self.ui_tx, MenuAction::Ui(UiCommand::OpenSettings))),
            MenuItem::Separator,
            MenuItem::Standard(menu_item("Switch to Local", &self.engine_tx, &self.ui_tx, MenuAction::Engine(EngineCommand::SwitchToLocal))),
            MenuItem::Standard(menu_item("Switch to Remote", &self.engine_tx, &self.ui_tx, MenuAction::Engine(EngineCommand::SwitchToRemote))),
            MenuItem::Standard(menu_item("Toggle PiP/PBP", &self.engine_tx, &self.ui_tx, MenuAction::Engine(EngineCommand::TogglePip))),
            MenuItem::Separator,
            MenuItem::Standard(StandardItem {
                label: format!("Active: {side}"),
                enabled: false,
                ..Default::default()
            }),
            MenuItem::Standard(StandardItem {
                label: if state.pip_active {
                    format!("PiP: {}", state.pip_label)
                } else {
                    "PiP: Off".to_string()
                },
                enabled: false,
                ..Default::default()
            }),
            MenuItem::Separator,
            MenuItem::Standard(menu_item("Quit", &self.engine_tx, &self.ui_tx, MenuAction::Ui(UiCommand::Quit))),
        ]
    }

    /// Keep running even when no SNI host (e.g. bare GNOME) is present yet;
    /// the tray may appear once one comes online.
    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        log::warn!("No StatusNotifierItem host available; tray icon not shown yet");
        true
    }
}

/// Live handle used by the UI to refresh menu labels.
#[derive(Clone)]
pub struct TrayHandle {
    handle: ksni::blocking::Handle<KwmTray>,
    state: Arc<Mutex<TrayState>>,
}

impl TrayHandle {
    pub fn update_state(&self, f: impl FnOnce(&mut TrayState)) {
        if let Ok(mut state) = self.state.lock() {
            f(&mut state);
        }
        let _ = self.handle.update(|_| {});
    }
}

/// Decodes the bundled logo into RGBA, scales it to a sane tray size and
/// recolors it per connection side (alpha is kept, so the silhouette is
/// unchanged; only the fill color differs).
fn load_tray_icons() -> Option<TrayIcons> {
    let png = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo.png"));
    let img = image::load_from_memory(png).ok()?;
    let size = 64;
    let rgba = img
        .resize(size, size, image::imageops::FilterType::Lanczos3)
        .into_rgba8()
        .into_raw();
    let tinted = |rgba: &[u8], rgb: [u8; 3]| {
        rgba.chunks_exact(4)
            .flat_map(|px| [rgb[0], rgb[1], rgb[2], px[3]])
            .collect()
    };
    Some(TrayIcons {
        unknown: rgba.clone(),
        local: tinted(&rgba, LOCAL_TINT),
        remote: tinted(&rgba, REMOTE_TINT),
        size,
    })
}

/// Spawns the tray service. Returns `None` when DBus/SNI is unavailable —
/// the app keeps running without a tray.
pub fn spawn(
    engine_tx: Sender<EngineCommand>,
    ui_tx: Sender<UiCommand>,
) -> Option<TrayHandle> {
    let icons = match load_tray_icons() {
        Some(icons) => icons,
        None => {
            log::warn!("Failed to decode bundled logo; tray icon will be blank");
            TrayIcons {
                unknown: Vec::new(),
                local: Vec::new(),
                remote: Vec::new(),
                size: 64,
            }
        }
    };

    let state = Arc::new(Mutex::new(TrayState {
        status: "Starting...".into(),
        local_active: false,
        has_tracked: false,
        pip_active: false,
        pip_label: "PiP".into(),
    }));

    let tray = KwmTray {
        engine_tx,
        ui_tx,
        state: state.clone(),
        icons,
    };

    match tray.assume_sni_available(true).spawn() {
        Ok(handle) => {
            log::info!("Tray icon registered");
            Some(TrayHandle { handle, state })
        }
        Err(err) => {
            log::warn!("Tray unavailable: {err}; continuing without tray icon");
            None
        }
    }
}

/// Writes the tray icon to the user's icon dir so the .desktop autostart entry
/// and window lists can resolve `Icon=KwmSwitcher`.
pub fn install_icon() {
    let Some(dirs) = dirs::data_dir() else { return };
    let icon_path = dirs
        .join("icons")
        .join("hicolor")
        .join("64x64")
        .join("apps")
        .join("kwmswitcher.png");
    if icon_path.exists() {
        return;
    }
    let png = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo.png"));
    if let Some(parent) = icon_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&icon_path, png);
}
