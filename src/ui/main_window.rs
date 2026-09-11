//! The main window: a minimal header (logo and title on the left; the
//! settings button next to the close control on the right), three device
//! cards (Local / Monitor / Remote) with the active one highlighted —
//! Local and Remote are clickable to switch — the status strip floating as
//! the last element of the centered content group, and a footer bar with
//! the monitor mode actions.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Icon, Sizable, TitleBar};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::ui::UiState;
use crate::input_source as isrc;

pub struct MainWindowView {
    engine: crate::engine::EngineHandle,
    state: UiState,
}

impl MainWindowView {
    pub fn new(
        engine: crate::engine::EngineHandle,
        state: UiState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The color scheme arrives asynchronously from the XDG portal on
        // Linux; re-sync whenever the platform reports a change.
        cx.observe_window_appearance(window, |_, window, cx| {
            crate::ui::sync_theme(Some(window), cx);
            cx.notify();
        })
        .detach();
        Self { engine, state }
    }

    pub fn sync_state(&mut self, state: &UiState) {
        self.state = state.clone();
    }
}

impl Render for MainWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();

        // The toolkit's built-in min/max/close only render for client-side
        // decorations; this KDE session reports server-side, so we draw our
        // own controls in exactly that case (never both).
        let toolkit_controls = matches!(window.window_decorations(), Decorations::Client { .. });

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.header(&theme, !toolkit_controls))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .justify_center()
                    .px_8()
                    .pb_6()
                    .gap_6()
                    .child(self.cards_row(&theme, cx))
                    .child(self.status_strip(&theme, cx)),
            )
            .child(self.footer(&theme, cx))
    }
}

impl MainWindowView {
    /// Minimal header: logo and title left-aligned in the left section;
    /// the settings button next to the close control at the far right.
    fn header(&self, theme: &Theme, own_controls: bool) -> TitleBar {
        TitleBar::new()
            .h(px(44.))
            .bg(theme.background)
            .border_color(theme.transparent)
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .pl_3()
                    .child(
                        Icon::new(IconName::MonitorSmartphone)
                            .with_size(px(22.))
                            .text_color(theme.accent),
                    )
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("KWM Switcher"),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(Self::settings_button())
                    .when(!own_controls, |bar| bar.pr_2())
                    .when(own_controls, |bar| {
                        bar.child(Self::window_control(
                            "close",
                            IconName::WindowClose,
                            true,
                            theme,
                            |_, window, _| window.remove_window(),
                        ))
                    }),
            )
    }

    /// Title-bar settings button next to the close control: soft pill with
    /// icon + label, matching the footer's button language.
    fn settings_button() -> Button {
        Button::new("open-settings")
            .icon(IconName::Settings)
            .label("Settings")
            .ghost()
            .small()
            .rounded(px(9999.))
            .on_click(|_, _, cx| {
                crate::ui::request_open_settings(cx);
            })
    }

    /// A title-bar control button (minimize / maximize / close) in the
    /// native Linux style: a small circular hit target centered in the bar,
    /// tinted on hover (red fill for close) with a darker pressed state.
    /// The toolkit's own controls only render under client-side
    /// decorations, so these replace them for server-side sessions.
    fn window_control(
        id: &'static str,
        icon: IconName,
        is_close: bool,
        theme: &Theme,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Div {
        let (hover_bg, hover_fg, active_bg) = if is_close {
            (
                theme.danger,
                theme.danger_foreground,
                theme.danger_active,
            )
        } else {
            (
                theme.secondary_hover,
                theme.secondary_foreground,
                theme.secondary_active,
            )
        };
        div()
            .w(px(44.))
            .h_full()
            .flex_shrink_0()
            .justify_center()
            .items_center()
            .child(
                div()
                    .id(id)
                    .flex()
                    .w(px(28.))
                    .h(px(28.))
                    .rounded_full()
                    .justify_center()
                    .items_center()
                    .text_color(theme.foreground)
                    .hover(move |s| s.bg(hover_bg).text_color(hover_fg))
                    .active(move |s| s.bg(active_bg).text_color(hover_fg))
                    .on_click(on_click)
                    .child(Icon::new(icon).small()),
            )
    }

    /// Local — Monitor — Remote cards; the active side gets the accent
    /// border and the "Active" badge. Local and Remote are clickable and
    /// switch to that side.
    fn cards_row(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let local_active = self.state.local_active;
        // With nothing configured the app has never switched, so neither
        // side can claim the "Active" badge.
        let remote_active = !self.state.tracked_devices.is_empty() && !local_active;
        let cards = [
            CardSpec {
                name: "Local",
                subtitle: "This machine",
                icon: IconName::Laptop,
                icon_color: display_blue(),
                active: local_active,
                action: Some(SwitchAction::Local),
            },
            CardSpec {
                name: "Monitor",
                subtitle: "DDC/CI target",
                icon: IconName::Monitor,
                icon_color: display_blue(),
                active: false,
                action: None,
            },
            CardSpec {
                name: "Remote",
                subtitle: "Other machine",
                icon: IconName::Server,
                icon_color: theme.muted_foreground,
                active: remote_active,
                action: Some(SwitchAction::Remote),
            },
        ];

        let mut row = h_flex().items_stretch().gap_6();
        for spec in &cards {
            let card = Self::device_card(spec, theme);
            match spec.action {
                Some(action) => {
                    // Hover affordance for clickable cards: active cards
                    // deepen their accent tint, inactive ones lighten and
                    // gain an accent-tinted border.
                    let (hover_bg, hover_border) = if spec.active {
                        (theme.accent.opacity(0.12), theme.accent)
                    } else {
                        (
                            theme.secondary_hover,
                            theme.muted_foreground.opacity(0.4),
                        )
                    };
                    row = row.child(
                        card.id(spec.name)
                            .cursor(CursorStyle::PointingHand)
                            .hover(move |s| s.bg(hover_bg).border_color(hover_border))
                            .on_click(cx.listener(move |this, _, _, _| match action {
                                SwitchAction::Local => this.engine.switch_to_local(),
                                SwitchAction::Remote => this.engine.switch_to_remote(),
                            })),
                    );
                }
                None => row = row.child(card),
            }
        }
        row
    }

    fn device_card(spec: &CardSpec, theme: &Theme) -> Div {
        let mut card = v_flex()
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(176.))
            .gap_0()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(if spec.active { theme.accent } else { theme.border })
            .bg(if spec.active {
                theme.accent.opacity(0.06)
            } else {
                theme.secondary
            })
            .p_5()
            .child(Icon::new(spec.icon).with_size(px(42.)).text_color(spec.icon_color))
            .child(
                div()
                    .mt_4()
                    .text_size(px(19.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(spec.name),
            )
            .child(
                div()
                    .mt_1()
                    .text_size(px(13.))
                    .text_color(theme.muted_foreground)
                    .child(spec.subtitle),
            );

        if spec.active {
            card = card.child(
                h_flex()
                    .absolute()
                    .top_4()
                    .right_4()
                    .items_center()
                    .gap_1p5()
                    .rounded_full()
                    .px_2p5()
                    .py_1()
                    .bg(theme.success.opacity(0.12))
                    .child(div().size(px(7.)).rounded_full().bg(theme.success))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.success)
                            .child("Active"),
                    ),
            );
        }

        card
    }

    /// Status strip: a full-width container whose tint follows the active
    /// side (green for local, blue for remote). The status text part is
    /// left-aligned; the divider and the devices chip sit at the right edge.
    fn status_strip(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        // Neutral grey while nothing is configured: the tint otherwise
        // claims an active side (green local / blue remote).
        let has_tracked = !self.state.tracked_devices.is_empty();
        let tint = if !has_tracked {
            theme.muted_foreground
        } else if self.state.local_active {
            theme.success
        } else {
            display_blue()
        };

        let count = self.state.tracked_present;
        let devices_label = match count {
            0 => "No devices".to_string(),
            1 => "1 device".to_string(),
            n => format!("{n} devices"),
        };

        let mut container = h_flex()
            .items_center()
            .gap_2()
            .rounded(theme.radius_lg)
            .px_3()
            .py_1()
            .bg(tint.opacity(0.10))
            .border_1()
            .border_color(tint.opacity(0.30))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(div().size(px(8.)).rounded_full().bg(tint))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(tint)
                            .child(self.state.status.clone()),
                    ),
            )
            .child(div().w(px(1.)).h(px(14.)).bg(tint.opacity(0.25)));

        if self.state.pip_query_failed {
            container = container.child(
                Button::new("refresh-pip")
                    .icon(IconName::RefreshCw)
                    .ghost()
                    .xsmall()
                    .tooltip("Re-read PiP mode from the monitor")
                    .on_click(cx.listener(|this, _, _, _| {
                        this.engine.refresh_pip();
                    })),
            );
        }

        // Hovering the chip reveals the tracked-device list: one row per
        // device, green dot when attached, grey when absent.
        let hover_devices = self.state.tracked_devices.clone();
        container = container.child(
            div().id("strip-devices").flex_shrink_0().tooltip(move |window, cx| {
                let devices = hover_devices.clone();
                Tooltip::element(move |_, cx| {
                    let theme = cx.theme().clone();
                    let mut list = v_flex().gap_1p5();
                    if devices.is_empty() {
                        list = list.child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.muted_foreground)
                                .child("No tracked devices configured."),
                        );
                    }
                    for device in &devices {
                        let dot = if device.present {
                            theme.success
                        } else {
                            theme.border
                        };
                        list = list.child(
                            h_flex()
                                .items_center()
                                .gap_1p5()
                                .child(div().size(px(6.)).rounded_full().bg(dot))
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(if device.present {
                                            theme.foreground
                                        } else {
                                            theme.muted_foreground
                                        })
                                        .child(device.name.clone()),
                                ),
                        );
                    }
                    list
                })
                .build(window, cx)
            })
            .child(
                Button::new("strip-devices")
                    .ghost()
                    .small()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Usb).with_size(px(14.)))
                    .child(div().text_size(px(12.)).child(devices_label))
                    .child(Icon::new(IconName::ChevronRight).with_size(px(14.)))
                    .on_click(|_, _, cx| {
                        crate::ui::request_open_settings(cx);
                    }),
            ),
        );

        container
    }

    /// Footer bar: the monitor mode actions on the right. Switching happens
    /// by clicking the Local / Remote cards.
    fn footer(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let pip_label = if self.state.is_pip_active() {
            isrc::pip_mode_name(self.state.pip_mode)
        } else {
            "PiP / PBP".to_string()
        };

        h_flex()
            .items_center()
            .justify_end()
            .gap_2()
            .px_8()
            .py_3()
            .border_t_1()
            .border_color(theme.border)
            // Soft, pill-shaped buttons matching the tinted cards and the
            // status pill.
            .child(
                Button::new("pip-mode")
                    .secondary()
                    .outline()
                    .rounded(px(9999.))
                    .icon(IconName::PictureInPicture2)
                    .label(pip_label)
                    .on_click(cx.listener(|this, _, _, _| {
                        this.engine.set_pip(isrc::PIP_ON);
                    })),
            )
            .child(
                Button::new("single-mode")
                    .secondary()
                    .outline()
                    .rounded(px(9999.))
                    .icon(IconName::Columns2)
                    .label("Single")
                    .on_click(cx.listener(|this, _, _, _| {
                        this.engine.set_pip(isrc::PIP_OFF);
                    })),
            )
    }
}

/// The display-path blue used for the Local/Monitor icons and the REMOTE
/// state, matching the mockup (the theme's `info` skews teal on this palette).
fn display_blue() -> Hsla {
    gpui::rgb(0x3B82F6).into()
}

struct CardSpec {
    name: &'static str,
    subtitle: &'static str,
    icon: IconName,
    icon_color: Hsla,
    active: bool,
    /// Set on the cards that switch sides when clicked.
    action: Option<SwitchAction>,
}

/// Which side a clickable card switches to.
#[derive(Clone, Copy)]
enum SwitchAction {
    Local,
    Remote,
}
