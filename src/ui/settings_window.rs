//! The settings window: tracked USB devices, input sources/protocol, target
//! monitor, and startup options.
//!
//! Layout mirrors the main window's design language: a 44 px bare title bar
//! with our own close control for server-side decorations, a two-column body
//! of tinted cards (devices on the left; monitor + startup on the right),
//! and a footer bar with pill actions.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::searchable_list::{
    SearchableListDelegate, SearchableListItem, SearchableVec,
};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Icon, IndexPath, Sizable, TitleBar};
use gpui_kit::prelude::*;
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();

        // Same decoration handling as the main window: the toolkit's built-in
        // controls only render client-side, so we draw our own close control
        // when the session reports server-side decorations.
        let toolkit_controls = matches!(window.window_decorations(), Decorations::Client { .. });

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(Self::title_bar(&theme, !toolkit_controls))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .gap_4()
                    .px_8()
                    .py_5()
                    .child(self.devices_card(&theme, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_4()
                            .child(self.monitor_card(&theme, cx))
                            .child(self.startup_card(&theme, cx)),
                    ),
            )
            // Footer: main-window styling — right-aligned soft pill actions.
            // Cancel discards changes and closes the window; Save persists
            // and closes.
            .child(
                h_flex()
                    .items_center()
                    .justify_end()
                    .gap_2()
                    .px_8()
                    .py_3()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("cancel")
                            .ghost()
                            .rounded(px(9999.))
                            .label("Cancel")
                            .on_click(|_, window, _| window.remove_window()),
                    )
                    .child(
                        Button::new("save")
                            .primary()
                            .rounded(px(9999.))
                            .label("Save")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.save(window, cx);
                            })),
                    ),
            )
    }
}

impl SettingsWindowView {
    /// Bare 44 px title bar matching the main window: gear + title on the
    /// left, our own circular close control on the right for server-side
    /// decoration sessions.
    fn title_bar(theme: &Theme, own_controls: bool) -> TitleBar {
        TitleBar::new()
            .h(px(44.))
            .bg(theme.background)
            .border_color(theme.transparent)
            .child(
                h_flex()
                    .items_center()
                    .gap_2p5()
                    .pl_3()
                    .child(
                        Icon::new(IconName::Settings)
                            .with_size(px(18.))
                            .text_color(theme.accent),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Settings"),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .when(!own_controls, |bar| bar.pr_2())
                    .when(own_controls, |bar| {
                        bar.child(crate::ui::window_control(
                            "settings-close",
                            IconName::WindowClose,
                            true,
                            theme,
                            |_, window, _| window.remove_window(),
                        ))
                    }),
            )
    }

    /// A card header: small tinted icon chip + section title, mirroring the
    /// main window's card iconography at settings density.
    fn card_header(
        icon: IconName,
        icon_color: Hsla,
        title: &'static str,
        theme: &Theme,
    ) -> Div {
        h_flex()
            .items_center()
            .gap_2p5()
            .child(
                h_flex()
                    .items_center()
                    .justify_center()
                    .size(px(26.))
                    .rounded(theme.radius)
                    .bg(icon_color.opacity(0.12))
                    .child(Icon::new(icon).with_size(px(15.)).text_color(icon_color)),
            )
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
    }

    /// Tracked USB devices — the trigger list. Scrolls internally so the
    /// card keeps a fixed footprint; the footer strip shows how many devices
    /// are tracked and holds the Refresh action.
    fn devices_card(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let total = self.devices.len();
        let tracked = self.devices.iter().filter(|d| d.is_tracked).count();

        let rows: Vec<AnyElement> = if self.devices.is_empty() {
            vec![div()
                .px_2()
                .py_2()
                .text_size(px(12.))
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
                    // Untracked rows dim out: the tracked set reads at a
                    // glance, like the main window's device tooltip.
                    h_flex()
                        .w_full()
                        .items_start()
                        .gap_2p5()
                        .px_2()
                        .py_1p5()
                        .rounded(theme.radius)
                        .hover(|s| s.bg(theme.secondary_hover))
                        .child(
                            Checkbox::new(("device", i))
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
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_px()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(13.))
                                        .text_color(if device.is_tracked {
                                            theme.foreground
                                        } else {
                                            theme.muted_foreground
                                        })
                                        .child(device.description.clone()),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(theme.muted_foreground)
                                        .child(key_label),
                                ),
                        )
                        .into_any_element()
                })
                .collect()
        };

        v_flex()
            .w(px(280.))
            .flex_shrink_0()
            .gap_3()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .p_4()
            .child(Self::card_header(
                IconName::Usb,
                theme.accent,
                "Tracked USB Devices",
                theme,
            ))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.muted_foreground)
                    .child(
                        "Devices connected through the USB switch. When any of \
                         these appear, the monitor switches to the local input.",
                    ),
            )
            .child(
                v_flex()
                    .id("devices-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .gap_0p5()
                    .children(rows),
            )
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(theme.muted_foreground)
                            .child(format!("{tracked} of {total} tracked")),
                    )
                    .child(
                        Button::new("refresh-devices")
                            .icon(IconName::RefreshCw)
                            .label("Refresh")
                            .secondary()
                            .outline()
                            .rounded(px(9999.))
                            .xsmall()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh_devices();
                                cx.notify();
                            })),
                    ),
            )
    }

    /// Re-lists USB devices, keeping the user's unsaved toggles for devices
    /// that are still present.
    fn refresh_devices(&mut self) {
        let unsaved: Vec<(String, bool)> = self
            .devices
            .iter()
            .map(|d| (d.key.clone(), d.is_tracked))
            .collect();
        let tracked_cfg: Vec<String> =
            self.config.read().unwrap().tracked_device_keys.clone();
        self.devices = usb::list_devices()
            .into_iter()
            .map(|d| {
                let key = d.key();
                let is_tracked = unsaved
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, tracked)| *tracked)
                    .unwrap_or_else(|| {
                        tracked_cfg.iter().any(|k| k.eq_ignore_ascii_case(&key))
                    });
                DeviceRow {
                    key,
                    description: d.description,
                    is_tracked,
                }
            })
            .collect();
    }

    /// Monitor card: input sources and protocol configure the same path, so
    /// they share a card; the target display sits below a divider.
    fn monitor_card(&self, theme: &Theme, _cx: &mut Context<Self>) -> Div {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .p_4()
            .child(Self::card_header(
                IconName::Monitor,
                crate::ui::display_blue(),
                "Monitor",
                theme,
            ))
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
                         standard DDC/CI switching.",
                    ),
            )
            .child(div().h(px(1.)).w_full().bg(theme.border))
            .child(self.select_row(
                "Target monitor",
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
                        "Empty = all monitors. On laptops, pick the \
                         external display.",
                    ),
            )
    }

    /// Startup card: switches with a one-line explanation each.
    fn startup_card(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        v_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .p_4()
            .child(Self::card_header(
                IconName::Power,
                theme.foreground,
                "Startup",
                theme,
            ))
            .child(self.toggle_row(
                "start-minimized",
                "Start minimized to tray",
                "Show only the tray icon until the window is opened.",
                self.start_minimized,
                theme,
                cx.listener(|this, value: &bool, _, cx| {
                    this.start_minimized = *value;
                    cx.notify();
                }),
            ))
            .child(self.toggle_row(
                "autostart",
                "Start automatically on login",
                "Launch KwmSwitcher with the desktop session.",
                self.autostart,
                theme,
                cx.listener(|this, value: &bool, _, cx| {
                    this.autostart = *value;
                    cx.notify();
                }),
            ))
    }

    /// A labeled select row: label on the left, control pinned to the right
    /// edge of the card. The row gets an explicit `w_full` and the select a
    /// fixed-width wrapper — the select's internals are percentage-sized, so
    /// a shrink-to-fit row would otherwise collapse the label to min-content.
    fn select_row<D>(&self, label: &'static str, select: Select<D>) -> Div
    where
        D: SearchableListDelegate,
    {
        h_flex()
            .w_full()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .child(label),
            )
            .child(
                div()
                    .w(px(220.))
                    .flex_shrink_0()
                    .child(select.small()),
            )
    }

    /// A labeled switch row: label + description on the left, switch on the
    /// right.
    fn toggle_row(
        &self,
        id: &'static str,
        label: &'static str,
        description: &'static str,
        checked: bool,
        theme: &Theme,
        on_change: impl Fn(&bool, &mut Window, &mut App) + 'static,
    ) -> Div {
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                v_flex()
                    .min_w_0()
                    .gap_px()
                    .child(div().text_size(px(13.)).child(label))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(theme.muted_foreground)
                            .child(description),
                    ),
            )
            .child(Switch::new(id).checked(checked).on_change(on_change))
    }

    /// Persists the form and closes the settings window.
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
