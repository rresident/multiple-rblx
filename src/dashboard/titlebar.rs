use gpui::{
    AnyElement, Context, CursorStyle, FontWeight, Window, WindowControlArea, div, prelude::*, px,
    rgb, svg,
};

use crate::theme::theme;

use super::Dashboard;

impl Dashboard {
    pub(super) fn render_titlebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let maximize_glyph = if window.is_maximized() {
            "\u{e923}"
        } else {
            "\u{e922}"
        };

        div()
            .h(px(32.0))
            .w_full()
            .flex_none()
            .flex()
            .items_center()
            .bg(rgb(theme().canvas))
            .child(
                div()
                    .h_full()
                    .flex_1()
                    .min_w(px(0.0))
                    .window_control_area(WindowControlArea::Drag)
                    .flex()
                    .items_center()
                    .child(
                        svg()
                            .ml(px(12.0))
                            .path("multiple.svg")
                            .size(px(16.0))
                            .text_color(rgb(theme().titlebar_icon)),
                    )
                    .child(
                        div()
                            .ml(px(8.0))
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme().titlebar_text))
                            .child("Multiple Roblox"),
                    ),
            )
            .child(
                div()
                    .id("open-settings")
                    .h(px(24.0))
                    .w(px(24.0))
                    .mr(px(8.0))
                    .flex_none()
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor(CursorStyle::PointingHand)
                    .hover(|style| style.bg(rgb(theme().caption_hover)))
                    .on_click(cx.listener(|this, _, _, cx| this.open_settings(cx)))
                    .child(
                        svg()
                            .path("settings.svg")
                            .size(px(14.0))
                            .text_color(rgb(theme().titlebar_text)),
                    ),
            )
            .child(
                div()
                    .h_full()
                    .w(px(138.0))
                    .flex_none()
                    .flex()
                    .child(window_control_button(
                        "window-minimize",
                        "\u{e921}",
                        CaptionAction::Minimize,
                        false,
                    ))
                    .child(window_control_button(
                        "window-maximize",
                        maximize_glyph,
                        CaptionAction::Maximize,
                        false,
                    ))
                    .child(window_control_button(
                        "window-close",
                        "\u{e8bb}",
                        CaptionAction::Close,
                        true,
                    )),
            )
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum CaptionAction {
    Minimize,
    Maximize,
    Close,
}

fn window_control_button(
    id: &'static str,
    glyph: &'static str,
    action: CaptionAction,
    destructive: bool,
) -> AnyElement {
    div()
        .id(id)
        .h_full()
        .w(px(46.0))
        .flex_none()
        .when(matches!(action, CaptionAction::Maximize), |button| {
            button.window_control_area(WindowControlArea::Max)
        })
        .when(matches!(action, CaptionAction::Minimize), |button| {
            button.window_control_area(WindowControlArea::Min)
        })
        .when(matches!(action, CaptionAction::Close), |button| {
            button.window_control_area(WindowControlArea::Close)
        })
        .text_color(rgb(theme().caption_glyph))
        .hover(move |style| {
            if destructive {
                style
                    .bg(rgb(theme().caption_close))
                    .text_color(rgb(theme().caption_close_text))
            } else {
                style
                    .bg(rgb(theme().caption_hover))
                    .text_color(rgb(theme().caption_hover_text))
            }
        })
        .active(move |style| {
            if destructive {
                style.bg(rgb(theme().caption_close_pressed))
            } else {
                style.bg(rgb(theme().caption_pressed))
            }
        })
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .font_family("Segoe Fluent Icons")
                .text_size(px(10.0))
                .child(glyph),
        )
        .into_any_element()
}
