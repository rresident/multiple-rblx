use gpui::{
    AnyElement, Context, CursorStyle, FontWeight, ObjectFit, SharedString, StyledImage, div, img,
    prelude::*, px, rgb, svg,
};

use crate::{
    launcher::{ArmOutcome, MultiInstanceGuard},
    theme::theme,
};

use super::{Dashboard, LaunchIntent, components::toggle_switch};

pub(super) fn arm_multi_instance() -> (Option<MultiInstanceGuard>, Option<SharedString>) {
    match crate::launcher::arm() {
        Ok((ArmOutcome::Armed, guard)) => (Some(guard), None),
        Ok((ArmOutcome::ClientAlreadyRunning, _)) => {
            if crate::launcher::client_is_playing() {
                return (
                    None,
                    Some(SharedString::from(
                        "Roblox is already running. Close it, then turn this on.",
                    )),
                );
            }

            match crate::launcher::close_running_clients() {
                Ok(0) => {}
                Ok(closed) => tracing::info!(closed, "cleared stale Roblox background processes"),
                Err(error) => {
                    tracing::warn!(reason = %error, "stale Roblox processes could not be cleared");
                }
            }

            match crate::launcher::arm() {
                Ok((ArmOutcome::Armed, guard)) => (Some(guard), None),
                Ok((ArmOutcome::ClientAlreadyRunning, _)) => (
                    None,
                    Some(SharedString::from(
                        "Something else is holding Roblox open. Restart your PC and try again.",
                    )),
                ),
                Err(error) => {
                    tracing::error!(reason = %error, "multi-instance guard failed to arm");
                    (
                        None,
                        Some(SharedString::from("Multiple Roblox could not be enabled.")),
                    )
                }
            }
        }
        Err(error) => {
            tracing::error!(reason = %error, "multi-instance guard failed to arm");
            (
                None,
                Some(SharedString::from("Multiple Roblox could not be enabled.")),
            )
        }
    }
}

impl Dashboard {
    pub(super) fn render_launch_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w_full()
            .flex_none()
            .flex()
            .flex_col()
            .mb(px(18.0))
            .child(self.render_multi_instance_row(cx))
            .when(self.selected_game.is_some(), |bar| {
                bar.child(self.render_selected_game_card(cx))
            })
            .into_any_element()
    }

    fn render_multi_instance_row(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let enabled = self
            .multi_instance
            .as_ref()
            .is_some_and(MultiInstanceGuard::is_armed);
        let notice = self.multi_instance_notice.clone();

        div()
            .w_full()
            .h(px(58.0))
            .px(px(18.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(rgb(theme().border))
            .bg(rgb(theme().surface))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme().text_primary))
                            .child("Enable multiple Roblox"),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(px(11.5))
                            .text_color(rgb(match notice {
                                Some(_) => theme().warning_text,
                                None => theme().text_tertiary,
                            }))
                            .child(notice.unwrap_or_else(|| {
                                SharedString::from(
                                    "Lets several accounts run side by side. Turn on before starting Roblox.",
                                )
                            })),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .when(self.multi_instance_notice.is_some() && !enabled, |row| {
                        row.child(
                            div()
                                .id("close-roblox")
                                .h(px(30.0))
                                .px(px(12.0))
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(rgb(theme().remove_border))
                                .bg(rgb(theme().warning_surface))
                                .text_size(px(11.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(theme().warning_strong_text))
                                .cursor(CursorStyle::PointingHand)
                                .focusable()
                                .hover(|style| {
                                    style.bg(rgb(theme().warning_hover)).border_color(rgb(theme().warning_hover_border))
                                })
                                .active(|style| style.bg(rgb(theme().warning_pressed)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .on_click(cx.listener(|this, _, _, cx| this.close_roblox(cx)))
                                .child("Close Roblox"),
                        )
                    })
                    .child(toggle_switch(
                        "multi-instance-toggle",
                        enabled,
                        cx.listener(|this, _, _, cx| this.toggle_multiple_instances(cx)),
                    )),
            )
            .into_any_element()
    }

    fn close_roblox(&mut self, cx: &mut Context<Self>) {
        match crate::launcher::close_running_clients() {
            Ok(0) => {
                self.multi_instance_notice =
                    Some(SharedString::from("No running Roblox client was found."));
            }
            Ok(closed) => {
                tracing::info!(closed, "closed Roblox clients at the user's request");
                self.multi_instance_notice = None;
                self.toggle_multiple_instances(cx);
                return;
            }
            Err(error) => {
                tracing::error!(reason = %error, "Roblox clients could not be closed");
                self.multi_instance_notice =
                    Some(SharedString::from("Roblox could not be closed."));
            }
        }
        cx.notify();
    }

    fn render_selected_game_card(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(game) = self.selected_game.clone() else {
            return div().into_any_element();
        };
        let icon = self.selected_game_icon.clone();

        div()
            .w_full()
            .mt(px(10.0))
            .h(px(66.0))
            .px(px(14.0))
            .rounded(px(12.0))
            .border_1()
            .border_color(rgb(theme().divider))
            .bg(rgb(theme().surface))
            .flex()
            .items_center()
            .child(
                div()
                    .size(px(42.0))
                    .flex_none()
                    .rounded(px(9.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(theme().border))
                    .bg(rgb(theme().card_backdrop))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when_some(icon, |slot, image| {
                        slot.child(img(image).size(px(40.0)).object_fit(ObjectFit::Cover))
                    }),
            )
            .child(
                div()
                    .ml(px(12.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme().text_primary))
                            .child(SharedString::from(format!(
                                "{} is selected for launches",
                                game.name
                            ))),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(px(11.0))
                            .text_color(rgb(theme().text_tertiary))
                            .child("Launch on any account starts this game without asking."),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("change-selected-game")
                            .h(px(30.0))
                            .px(px(12.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(theme().control_border))
                            .bg(rgb(theme().control_surface))
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme().text_secondary))
                            .cursor(CursorStyle::PointingHand)
                            .focusable()
                            .hover(|style| style.bg(rgb(theme().control_hover)))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_game_picker(LaunchIntent::Pin, cx);
                            }))
                            .child("Change"),
                    )
                    .child(
                        div()
                            .id("clear-selected-game")
                            .size(px(30.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(theme().control_border))
                            .bg(rgb(theme().control_surface))
                            .cursor(CursorStyle::PointingHand)
                            .focusable()
                            .hover(|style| {
                                style
                                    .bg(rgb(theme().remove_hover))
                                    .border_color(rgb(theme().remove_border))
                            })
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_click(cx.listener(|this, _, _, cx| this.clear_selected_game(cx)))
                            .child(
                                svg()
                                    .path("close.svg")
                                    .size(px(15.0))
                                    .text_color(rgb(theme().text_secondary)),
                            ),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn clear_selected_game(&mut self, cx: &mut Context<Self>) {
        self.selected_game = None;
        self.selected_game_icon = None;
        self.preferences.selected_game = None;
        self.persist_preferences(cx);
        cx.notify();
    }

    pub(super) fn toggle_multiple_instances(&mut self, cx: &mut Context<Self>) {
        if self.preferences.multiple_instances_enabled {
            self.multi_instance = None;
            self.preferences.multiple_instances_enabled = false;
            self.multi_instance_notice = None;
            self.persist_preferences(cx);
            tracing::info!("multi-instance guard released");
            cx.notify();
            return;
        }

        let (guard, notice) = arm_multi_instance();
        match guard {
            Some(guard) => {
                self.multi_instance = Some(guard);
                self.preferences.multiple_instances_enabled = true;
                self.multi_instance_notice = None;
                self.persist_preferences(cx);
                tracing::info!("multi-instance guard armed");
            }
            None => {
                self.multi_instance_notice = notice;
                tracing::info!("multi-instance guard could not be armed");
            }
        }
        cx.notify();
    }
}
