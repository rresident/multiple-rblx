use gpui::{
    AnyElement, Context, CursorStyle, FontWeight, KeyDownEvent, ObjectFit, StyledImage, Window,
    deferred, div, img, prelude::*, px, rgb, rgba,
};

use crate::{accounts::RemoveAccountRequested, theme::theme};

use super::Dashboard;

impl Dashboard {
    pub(super) fn render_remove_dialog(
        &self,
        request: RemoveAccountRequested,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let account_id = request.account_id;
        let account_id_for_confirm = account_id;
        let username = request.username.clone();
        let body = format!(
            "{} will be removed from Multiple Roblox on this device. This does not delete or modify the Roblox account.",
            request.username
        );

        deferred(
            div()
                .id("remove-account-modal")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgba(theme().scrim))
                .occlude()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            this.cancel_removal(window, cx);
                            cx.stop_propagation();
                        }
                        "tab" => {
                            this.cycle_modal_focus(event.keystroke.modifiers.shift, window);
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                }))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(408.0))
                        .h(px(258.0))
                        .p(px(24.0))
                        .rounded(px(16.0))
                        .border_1()
                        .border_color(rgb(theme().strong_border))
                        .bg(rgb(theme().dialog_surface))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(17.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme().text_primary))
                                .child("Remove account?"),
                        )
                        .child(
                            div()
                                .mt(px(8.0))
                                .text_size(px(12.5))
                                .line_height(px(18.0))
                                .text_color(rgb(theme().text_secondary))
                                .child(body),
                        )
                        .child(
                            div()
                                .mt(px(18.0))
                                .h(px(64.0))
                                .w_full()
                                .px(px(12.0))
                                .rounded(px(10.0))
                                .border_1()
                                .border_color(rgb(theme().border))
                                .bg(rgb(theme().inset_surface))
                                .flex()
                                .items_center()
                                .child(dialog_avatar(&request))
                                .child(
                                    div()
                                        .ml(px(11.0))
                                        .flex()
                                        .flex_col()
                                        .justify_center()
                                        .child(
                                            div()
                                                .text_size(px(13.5))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(rgb(theme().text_primary))
                                                .child(username),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .text_color(rgb(theme().text_metadata))
                                                .child(account_id.to_string()),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .mt(px(24.0))
                                .h(px(38.0))
                                .w_full()
                                .flex()
                                .items_center()
                                .justify_end()
                                .child(
                                    div()
                                        .id("cancel-account-removal")
                                        .track_focus(&self.cancel_focus)
                                        .h(px(38.0))
                                        .w(px(88.0))
                                        .rounded(px(9.0))
                                        .border_1()
                                        .border_color(rgb(theme().control_border))
                                        .bg(rgb(theme().control_surface))
                                        .text_color(rgb(theme().control_text))
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .cursor(CursorStyle::PointingHand)
                                        .focus(|style| {
                                            style.border_color(rgb(theme().focus_border))
                                        })
                                        .hover(|style| {
                                            style
                                                .bg(rgb(theme().dialog_button_hover))
                                                .border_color(rgb(
                                                    theme().dialog_button_hover_border
                                                ))
                                                .text_color(rgb(theme().text_primary))
                                        })
                                        .active(|style| {
                                            style.bg(rgb(theme().dialog_button_pressed))
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.cancel_removal(window, cx);
                                        }))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child("Cancel"),
                                )
                                .child(
                                    div()
                                        .id("confirm-account-removal")
                                        .track_focus(&self.confirm_focus)
                                        .ml(px(8.0))
                                        .h(px(38.0))
                                        .w(px(104.0))
                                        .rounded(px(9.0))
                                        .border_1()
                                        .border_color(rgb(theme().danger_border))
                                        .bg(rgb(theme().danger_surface))
                                        .text_color(rgb(theme().danger_text))
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .cursor(CursorStyle::PointingHand)
                                        .focus(|style| {
                                            style.border_color(rgb(theme().danger_focus_border))
                                        })
                                        .hover(|style| style.bg(rgb(theme().danger_hover)))
                                        .active(|style| style.bg(rgb(theme().danger_pressed)))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.confirm_removal(
                                                account_id_for_confirm,
                                                window,
                                                cx,
                                            );
                                        }))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child("Remove account"),
                                ),
                        ),
                ),
        )
        .with_priority(100)
        .into_any_element()
    }

    fn cancel_removal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let return_focus = self
            .pending_removal
            .take()
            .and_then(|request| request.return_focus);

        if let Some(return_focus) = return_focus {
            window.focus(&return_focus);
        } else {
            window.focus(&self.accounts.read(cx).section_focus());
        }
        cx.notify();
    }

    fn confirm_removal(&mut self, account_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.accounts
            .update(cx, |accounts, cx| accounts.remove_account(account_id, cx));
        self.pending_removal = None;
        window.focus(&self.accounts.read(cx).section_focus());
        cx.notify();
    }

    fn cycle_modal_focus(&self, backwards: bool, window: &mut Window) {
        let cancel_is_focused = self.cancel_focus.is_focused(window);
        let target = if backwards {
            if cancel_is_focused {
                &self.confirm_focus
            } else {
                &self.cancel_focus
            }
        } else if cancel_is_focused {
            &self.confirm_focus
        } else {
            &self.cancel_focus
        };
        window.focus(target);
    }
}

fn dialog_avatar(request: &RemoveAccountRequested) -> AnyElement {
    if let Some(image) = request.avatar.clone() {
        return div()
            .size(px(40.0))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded_full()
            .overflow_hidden()
            .border_1()
            .border_color(rgb(theme().strong_border))
            .bg(rgb(theme().border))
            .child(
                img(image)
                    .size(px(36.0))
                    .rounded_full()
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element();
    }

    let initial = request
        .username
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();

    div()
        .size(px(40.0))
        .flex_none()
        .rounded_full()
        .border_1()
        .border_color(rgb(theme().strong_border))
        .bg(rgb(theme().border))
        .text_color(rgb(theme().text_secondary))
        .text_size(px(13.0))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(initial)
        .into_any_element()
}
