use gpui::{
    AnyElement, App, ClickEvent, CursorStyle, ElementId, FontWeight, SharedString, Window, div,
    prelude::*, px, rgb,
};

use crate::theme::theme;

const TRACK_WIDTH: f32 = 38.0;
const TRACK_HEIGHT: f32 = 21.0;
const KNOB: f32 = 15.0;
const KNOB_INSET: f32 = 3.0;

pub(super) fn toggle_switch(
    id: impl Into<ElementId>,
    enabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .flex_none()
        .w(px(TRACK_WIDTH))
        .h(px(TRACK_HEIGHT))
        .rounded(px(TRACK_HEIGHT / 2.0))
        .bg(rgb(if enabled {
            theme().toggle_on
        } else {
            theme().toggle_off
        }))
        .relative()
        .cursor(CursorStyle::PointingHand)
        .focusable()
        .on_click(on_click)
        .child(
            div()
                .absolute()
                .top(px((TRACK_HEIGHT - KNOB) / 2.0))
                .left(px(if enabled {
                    TRACK_WIDTH - KNOB - KNOB_INSET
                } else {
                    KNOB_INSET
                }))
                .size(px(KNOB))
                .rounded_full()
                .bg(rgb(theme().toggle_knob)),
        )
        .into_any_element()
}

pub(super) fn setting_row(
    title: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: AnyElement,
) -> AnyElement {
    div()
        .w_full()
        .py(px(14.0))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(24.0))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(theme().text_primary))
                        .child(title.into()),
                )
                .child(
                    div()
                        .mt(px(3.0))
                        .text_size(px(11.5))
                        .line_height(px(16.0))
                        .text_color(rgb(theme().text_tertiary))
                        .child(description.into()),
                ),
        )
        .child(div().flex_none().child(control))
        .into_any_element()
}

pub(super) fn secondary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    width: f32,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .h(px(34.0))
        .w(px(width))
        .flex_none()
        .rounded(px(9.0))
        .border_1()
        .border_color(rgb(theme().control_border))
        .bg(rgb(theme().control_surface))
        .text_color(rgb(theme().control_text))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor(CursorStyle::PointingHand)
        .focusable()
        .hover(|style| {
            style
                .bg(rgb(theme().control_hover))
                .border_color(rgb(theme().control_hover_border))
        })
        .active(|style| style.bg(rgb(theme().control_pressed)))
        .flex()
        .items_center()
        .justify_center()
        .on_click(on_click)
        .child(label.into())
        .into_any_element()
}

pub(super) fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    width: f32,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .h(px(34.0))
        .w(px(width))
        .flex_none()
        .rounded(px(9.0))
        .bg(rgb(theme().accent_surface))
        .text_color(rgb(theme().accent_text))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor(CursorStyle::PointingHand)
        .focusable()
        .hover(|style| style.bg(rgb(theme().accent_hover)))
        .active(|style| style.bg(rgb(theme().accent_pressed)))
        .flex()
        .items_center()
        .justify_center()
        .on_click(on_click)
        .child(label.into())
        .into_any_element()
}

pub(super) fn section_note(text: impl Into<SharedString>) -> AnyElement {
    div()
        .mt(px(6.0))
        .text_size(px(11.0))
        .line_height(px(16.0))
        .text_color(rgb(theme().text_secondary))
        .child(text.into())
        .into_any_element()
}

pub(super) fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    selected: T,
    on_select: impl Fn(&T, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let on_select = std::rc::Rc::new(on_select);
    let mut row = div()
        .id(id)
        .flex_none()
        .p(px(2.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(rgb(theme().control_border))
        .bg(rgb(theme().control_surface))
        .flex()
        .items_center()
        .gap(px(2.0));

    for (index, (value, label)) in options.iter().enumerate() {
        let value = *value;
        let active = value == selected;
        let handler = std::rc::Rc::clone(&on_select);
        row = row.child(
            div()
                .id(("segment", index))
                .h(px(26.0))
                .px(px(14.0))
                .rounded(px(7.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .cursor(CursorStyle::PointingHand)
                .when(active, |segment| {
                    segment
                        .bg(rgb(theme().surface))
                        .text_color(rgb(theme().text_primary))
                })
                .when(!active, |segment| {
                    segment
                        .text_color(rgb(theme().text_tertiary))
                        .hover(|style| style.text_color(rgb(theme().text_secondary)))
                })
                .on_click(move |_, window, app| handler(&value, window, app))
                .child(*label),
        );
    }

    row.into_any_element()
}
