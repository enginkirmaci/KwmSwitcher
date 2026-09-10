//! The settings window: tracked USB devices, input sources/protocol, target
//! monitor, and startup options.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::group_box::GroupBox;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::searchable_list::{
    SearchableListDelegate, SearchableListItem, SearchableVec,
};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Icon, IndexPath, Sizable, TitleBar};
use gpui_kit::*;

use std::sync::Arc;

use crate::config::{InputSwitchProtocol, SharedConfig};
use crate::ddc::Ddc;
use crate::input_source as isrc;
use crate::usb;

// ---------------------------------------------------------------- options

#[derive(Clone)]
struct InputOption {
    code: u8,
    name: &'static str,
}

impl SearchableListItem for InputOption {
    type Value = u8;
    fn title(&self) -> SharedString {
        self.name.into()
    }
    fn value(&self) -> &Self::Value {
        &self.code
    }
}

#[derive(Clone)]
struct ProtocolOption {
    protocol: InputSwitchProtocol,
    value: u8,
    name: &'static str,
}

impl SearchableListItem for ProtocolOption {
    type Value = u8;
    fn title(&self) -> SharedString {
        self.name.into()
    }
    fn value(&self) -> &Self::Value {
        &self.value
    }
}

#[derive(Clone)]
struct MonitorOption(String);

impl SearchableListItem for MonitorOption {
    type Value = String;
    fn title(&self) -> SharedString {
        self.0.clone().into()
    }
    fn value(&self) -> &Self::Value {
        &self.0
    }
}

struct DeviceRow {
    key: String,
    description: String,
    is_tracked: bool,
}

fn protocol_options() -> Vec<ProtocolOption> {
    vec![
        ProtocolOption {
            protocol: InputSwitchProtocol::Standard,
            value: InputSwitchProtocol::Standard.as_u8(),
            name: "Standard DDC/CI (0x60)",
        },
        ProtocolOption {
            protocol: InputSwitchProtocol::Lg,
            value: InputSwitchProtocol::Lg.as_u8(),
            name: "LG (0xF4)",
        },
    ]
}

fn input_options(protocol: InputSwitchProtocol) -> Vec<InputOption> {
    isrc::input_options(protocol)
        .into_iter()
        .map(|(code, name)| InputOption { code, name })
        .collect()
}

// ---------------------------------------------------------------- view

pub struct SettingsWindowView {
    config: SharedConfig,
    devices: Vec<DeviceRow>,
    protocol: Entity<SelectState<SearchableVec<ProtocolOption>>>,
    local_input: Entity<SelectState<SearchableVec<InputOption>>>,
    remote_input: Entity<SelectState<SearchableVec<InputOption>>>,
    target_monitor: Entity<SelectState<SearchableVec<MonitorOption>>>,
    start_minimized: bool,
    autostart: bool,
}

impl SettingsWindowView {
    /// Called inside the window's `cx.new` with a live `Window`.
    pub fn new(
        config: SharedConfig,
        ddc: Arc<Ddc>,
        monitors: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let _ = &ddc;
        let cfg = config.read().unwrap().clone();

        let protocols = protocol_options();
        let protocol_index = protocols
            .iter()
            .position(|p| p.protocol == cfg.input_protocol)
            .unwrap_or(0);
        let protocol = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(protocols),
                Some(IndexPath::new(protocol_index)),
                window,
                cx,
            )
        });

        let local_opts = input_options(cfg.input_protocol);
        let remote_opts = local_opts.clone();
        let local_index = local_opts.iter().position(|o| o.code == cfg.local_input_source);
        let remote_index = remote_opts
            .iter()
            .position(|o| o.code == cfg.remote_input_source);
        let local_input = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(local_opts),
                local_index.map(IndexPath::new),
                window,
                cx,
            )
        });
        let remote_input = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(remote_opts),
                remote_index.map(IndexPath::new),
                window,
                cx,
            )
        });

        let monitor_options: Vec<MonitorOption> =
            monitors.into_iter().map(MonitorOption).collect();
        let monitor_index = cfg
            .target_monitor_name
            .as_ref()
            .and_then(|name| monitor_options.iter().position(|m| &m.0 == name));
        let target_monitor = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(monitor_options),
                monitor_index.map(IndexPath::new),
                window,
                cx,
            )
            .searchable(true)
        });

        // Switching the protocol swaps the available input sources.
        cx.subscribe_in(
            &protocol,
            window,
            |this, _, event: &SelectEvent<SearchableVec<ProtocolOption>>, window, cx| {
                if matches!(event, SelectEvent::Confirm(_)) {
                    this.sync_input_lists(window, cx);
                }
            },
        )
        .detach();

        Self {
            devices: Self::collect_devices(&config),
            config,
            protocol,
            local_input,
            remote_input,
            target_monitor,
            start_minimized: cfg.start_minimized,
            autostart: crate::autostart::is_enabled(),
        }
    }

    fn collect_devices(config: &SharedConfig) -> Vec<DeviceRow> {
        let tracked: Vec<String> = config.read().unwrap().tracked_device_keys.clone();
        usb::list_devices()
            .into_iter()
            .map(|d| DeviceRow {
                is_tracked: tracked.iter().any(|k| k.eq_ignore_ascii_case(&d.key())),
                key: d.key(),
                description: d.description,
            })
            .collect()
    }

    /// Rebuilds the local/remote input options after a protocol change,
    /// preserving the current selection when still valid.
    fn sync_input_lists(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(protocol) = self.protocol.read(cx).selected_value().copied() else {
            return;
        };
        let protocol = InputSwitchProtocol::from_u8(protocol);
        let (local_target, remote_target) = {
            let cfg = self.config.read().unwrap();
            (
                self.local_input
                    .read(cx)
                    .selected_value()
                    .copied()
                    .unwrap_or(cfg.local_input_source),
                self.remote_input
                    .read(cx)
                    .selected_value()
                    .copied()
                    .unwrap_or(cfg.remote_input_source),
            )
        };

        let opts = input_options(protocol);
        let pick = |code: u8, opts: &[InputOption]| -> Option<IndexPath> {
            opts.iter()
                .position(|o| o.code == code)
                .map(IndexPath::new)
                .or_else(|| (!opts.is_empty()).then(|| IndexPath::new(0)))
        };
        let local_index = pick(local_target, &opts);
        let remote_index = pick(remote_target, &opts);

        self.local_input.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(opts.clone()), window, cx);
            if let Some(index) = local_index {
                state.set_selected_index(Some(index), window, cx);
            }
        });
        self.remote_input.update(cx, |state, cx| {
            state.set_items(SearchableVec::new(opts), window, cx);
            if let Some(index) = remote_index {
                state.set_selected_index(Some(index), window, cx);
            }
        });
        cx.notify();
    }
}

impl Render for SettingsWindowView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();

        let device_rows: Vec<AnyElement> = if self.devices.is_empty() {
            vec![div()
                .py_2()
                .text_size(px(13.))
                .text_color(theme.muted_foreground)
                .child("No USB devices found. Plug in a device or press Refresh.")
                .into_any_element()]
        } else {
            self.devices
                .iter()
                .enumerate()
                .map(|(i, device)| {
                    let key = device.key.clone();
                    let key_label = key.clone();
                    v_flex()
                        .py_1()
                        .child(
                            Checkbox::new(("device", i))
                                .label(device.description.clone())
                                .checked(device.is_tracked)
                                .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                                    // `key` is cloned per invocation: the
                                    // listener may fire many times.
                                    let key = key.clone();
                                    if let Some(row) =
                                        this.devices.iter_mut().find(|r| r.key == key)
                                    {
                                        row.is_tracked = *checked;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .pl_6()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(key_label),
                        )
                        .into_any_element()
                })
                .collect()
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .pl_2()
                        .child(
                            Icon::new(IconName::Settings)
                                .small()
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Settings"),
                        ),
                ),
            )
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .overflow_y_scrollbar()
                    .px_5()
                    .py_4()
                    .child(
                        v_flex()
                            .gap_4()
                            // Tracked USB devices.
                            .child(
                                GroupBox::new().title("Tracked USB Devices").child(
                                    v_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(theme.muted_foreground)
                                                .child(
                                                    "Devices connected through the USB switch. \
                                                     When any of these appear, the monitor switches \
                                                     to the local input.",
                                                ),
                                        )
                                        .child(
                                            h_flex()
                                                .justify_end()
                                                .child(
                                                    Button::new("refresh-devices")
                                                        .icon(IconName::RefreshCw)
                                                        .label("Refresh")
                                                        .ghost()
                                                        .small()
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.devices =
                                                                Self::collect_devices(&this.config);
                                                            cx.notify();
                                                        })),
                                                ),
                                        )
                                        .children(device_rows),
                                ),
                            )
                            // Input sources.
                            .child(
                                GroupBox::new().title("Monitor Input").child(
                                    v_flex()
                                        .gap_3()
                                        .child(self.select_row(
                                            "Local input (this machine)",
                                            Select::new(&self.local_input).id("local-input"),
                                        ))
                                        .child(self.select_row(
                                            "Remote input (other machine)",
                                            Select::new(&self.remote_input).id("remote-input"),
                                        ))
                                        .child(self.select_row(
                                            "Input protocol",
                                            Select::new(&self.protocol).id("protocol"),
                                        ))
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(theme.muted_foreground)
                                                .child(
                                                    "Use the LG protocol if your monitor ignores \
                                                     standard DDC/CI input switching.",
                                                ),
                                        ),
                                ),
                            )
                            // Target monitor.
                            .child(
                                GroupBox::new().title("Target Monitor").child(
                                    v_flex()
                                        .gap_2()
                                        .child(self.select_row(
                                            "Monitor",
                                            Select::new(&self.target_monitor)
                                                .id("target-monitor")
                                                .placeholder("All monitors")
                                                .cleanable(true),
                                        ))
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .text_color(theme.muted_foreground)
                                                .child(
                                                    "Leave empty to try all monitors. Pick your \
                                                     external display on laptops to avoid switching \
                                                     the built-in panel.",
                                                ),
                                        ),
                                ),
                            )
                            // Startup.
                            .child(
                                GroupBox::new().title("Startup").child(
                                    v_flex()
                                        .gap_3()
                                        .child(
                                            Switch::new("start-minimized")
                                                .label("Start minimized to tray")
                                                .checked(self.start_minimized)
                                                .on_change(cx.listener(
                                                    |this, value: &bool, _, cx| {
                                                        this.start_minimized = *value;
                                                        cx.notify();
                                                    },
                                                )),
                                        )
                                        .child(
                                            Switch::new("autostart")
                                                .label("Start automatically on login")
                                                .checked(self.autostart)
                                                .on_change(cx.listener(
                                                    |this, value: &bool, _, cx| {
                                                        this.autostart = *value;
                                                        cx.notify();
                                                    },
                                                )),
                                        ),
                                ),
                            ),
                    ),
            )
            // Footer.
            .child(
                h_flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .px_5()
                    .py_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("cancel")
                            .ghost()
                            .label("Cancel")
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .child(
                        Button::new("save")
                            .primary()
                            .label("Save")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save(window, cx);
                            })),
                    ),
            )
    }
}

impl SettingsWindowView {
    fn select_row<D>(&self, label: &'static str, select: Select<D>) -> Div
    where
        D: SearchableListDelegate,
    {
        h_flex()
            .items_center()
            .justify_between()
            .child(div().text_size(px(13.)).child(label))
            .child(select.w(px(260.)))
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let protocol = self
            .protocol
            .read(cx)
            .selected_value()
            .copied()
            .map(InputSwitchProtocol::from_u8)
            .unwrap_or(InputSwitchProtocol::Standard);
        let local = self.local_input.read(cx).selected_value().copied();
        let remote = self.remote_input.read(cx).selected_value().copied();
        let target = self.target_monitor.read(cx).selected_value().cloned();
        let tracked: Vec<String> = self
            .devices
            .iter()
            .filter(|d| d.is_tracked)
            .map(|d| d.key.clone())
            .collect();

        {
            let mut cfg = self.config.write().unwrap();
            cfg.input_protocol = protocol;
            if let Some(local) = local {
                cfg.local_input_source = local;
            }
            if let Some(remote) = remote {
                cfg.remote_input_source = remote;
            }
            cfg.target_monitor_name = target;
            cfg.start_minimized = self.start_minimized;
            cfg.auto_start = self.autostart;
            cfg.tracked_device_keys = tracked;
            cfg.save();
        }
        crate::autostart::set_enabled(self.autostart);
        log::info!("Settings saved");

        window.remove_window();
    }
}
