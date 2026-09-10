//! The main window: a status hero (Local → Monitor → Remote) with the active
//! path highlighted, quick switch actions, PiP toggle and a status line.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::Theme;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme, Icon, Sizable, TitleBar};
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let local_active = self.state.local_active;
        let pip_active = self.state.is_pip_active();
        let pip_label = if pip_active {
            isrc::pip_mode_name(self.state.pip_mode)
        } else {
            "PiP".to_string()
        };

        let (status_color, status_text) = if self.state.local_active {
            (theme.success, "LOCAL")
        } else {
            (theme.info, "REMOTE")
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                TitleBar::new()
                    .child(
                        h_flex()
                            .items_center()
                            .gap_2()
                            .pl_2()
                            .child(
                                Icon::new(IconName::MonitorSmartphone)
                                    .small()
                                    .text_color(theme.accent),
                            )
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("KWM Switcher"),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .pr_2()
                            .child(
                                Button::new("open-settings")
                                    .icon(IconName::Settings)
                                    .ghost()
                                    .small()
                                    .tooltip("Settings")
                                    .on_click(|_, _, cx| {
                                        crate::ui::request_open_settings(cx);
                                    }),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .overflow_hidden()
                    .px_6()
                    .py_5()
                    .gap_4()
                    .justify_center()
                    .max_w(px(640.))
                    // Hero: Local — Monitor — Remote with the active path lit.
                    .child(
                        div()
                            .rounded(theme.radius_lg)
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.secondary)
                            .p_5()
                            .child(
                                h_flex()
                                    .w_full()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        self.node("Local", "This machine", IconName::Laptop, local_active, &theme),
                                    )
                                    .child(Self::connector(local_active, &theme))
                                    .child({
                                        let mut monitor = self.node(
                                            "Monitor",
                                            "DDC/CI target",
                                            IconName::Monitor,
                                            true,
                                            &theme,
                                        );
                                        if pip_active {
                                            monitor = monitor.child(
                                                div()
                                                    .absolute()
                                                    .bottom(px(18.))
                                                    .rounded_full()
                                                    .px_2()
                                                    .py(px(1.))
                                                    .bg(theme.accent)
                                                    .text_color(theme.accent_foreground)
                                                    .text_size(px(10.))
                                                    .font_weight(FontWeight::BOLD)
                                                    .child(pip_label.clone()),
                                            );
                                        }
                                        monitor.relative()
                                    })
                                    .child(Self::connector(!local_active, &theme))
                                    .child(
                                        self.node("Remote", "Other machine", IconName::Server, !local_active, &theme),
                                    ),
                            ),
                    )
                    // Status line.
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                div()
                                    .rounded_full()
                                    .px_3()
                                    .py_1()
                                    .bg(status_color.opacity(0.15))
                                    .text_color(status_color)
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::BOLD)
                                    .child(status_text),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(13.))
                                    .text_color(theme.muted_foreground)
                                    .child(self.state.status.clone()),
                            )
                            .child(
                                Button::new("refresh-pip")
                                    .icon(IconName::RefreshCw)
                                    .ghost()
                                    .xsmall()
                                    .tooltip("Re-read PiP mode from the monitor")
                                    .on_click(cx.listener(|this, _, _, _| {
                                        this.engine.refresh_pip();
                                    })),
                            ),
                    )
                    // Actions.
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                Button::new("switch-local")
                                    .primary()
                                    .icon(IconName::ArrowLeft)
                                    .label("Switch to Local")
                                    .on_click(cx.listener(|this, _, _, _| {
                                        this.engine.switch_to_local();
                                    })),
                            )
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap_3()
                                    .child(
                                        Button::new("switch-remote")
                                            .secondary()
                                            .icon(IconName::ArrowRight)
                                            .label("Switch to Remote")
                                            .on_click(cx.listener(|this, _, _, _| {
                                                this.engine.switch_to_remote();
                                            })),
                                    )
                                    .child(
                                        Button::new("toggle-pip")
                                            .outline()
                                            .icon(IconName::PictureInPicture2)
                                            .label(if pip_active {
                                                "Exit PiP/PBP"
                                            } else {
                                                "PiP / PBP"
                                            })
                                            .on_click(cx.listener(|this, _, _, _| {
                                                this.engine.toggle_pip();
                                            })),
                                    ),
                            ),
                    ),
            )
    }
}

impl MainWindowView {
    /// A device node: icon tile + name + subtitle, accent-tinted when active.
    fn node(
        &self,
        name: &'static str,
        subtitle: &'static str,
        icon: IconName,
        active: bool,
        theme: &Theme,
    ) -> Div {
        let accent = theme.accent;
        v_flex()
            .w(px(110.))
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(64.))
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(if active { accent } else { theme.border })
                    .bg(if active {
                        accent.opacity(0.12)
                    } else {
                        theme.muted
                    })
                    .child(
                        Icon::new(icon).large().text_color(if active {
                            accent
                        } else {
                            theme.muted_foreground
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(name),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(theme.muted_foreground)
                    .child(subtitle),
            )
    }

    /// The wire between two nodes; lit in accent when this path is active.
    fn connector(active: bool, theme: &Theme) -> Div {
        let color = if active { theme.accent } else { theme.border };
        div()
            .flex_1()
            .mx_2()
            .h(px(3.))
            .rounded_full()
            .bg(color.opacity(if active { 0.9 } else { 0.5 }))
    }
}
