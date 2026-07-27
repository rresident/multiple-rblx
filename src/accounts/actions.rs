use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, CursorStyle, FontWeight, Transformation,
    deferred, div, percentage, prelude::*, px, rgb, svg,
};

use crate::theme::theme;

use super::{
    AccountAction, ConnectedAccounts, HoveredAction, instance::InstancePhase, model::SessionHealth,
};

const ACTION_SLOT_WIDTH: f32 = 124.0;
const ACTION_CONTROL_WIDTH: f32 = 116.0;
const ACTION_SEGMENT_WIDTH: f32 = 57.0;
const ACTION_SEGMENT_HEIGHT: f32 = 32.0;
const ACTION_HEIGHT: f32 = 34.0;
const STATE_TEXT_SIZE: f32 = 11.0;
const RUNNING_CONTENT_OFFSET: f32 = 9.0;

const SIGN_IN_SEGMENT_WIDTH: f32 = 78.0;
const SIGN_IN_REMOVE_WIDTH: f32 = 36.0;

impl ConnectedAccounts {
    pub(super) fn render_actions(
        &self,
        user_id: u64,
        phase: InstancePhase,
        session_health: SessionHealth,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match phase {
            InstancePhase::Idle if session_health == SessionHealth::LoginRequired => {
                self.render_sign_in_control(user_id, cx)
            }
            InstancePhase::Idle => self.render_idle_actions(user_id, session_health, cx),
            InstancePhase::Starting => render_transition_control(user_id, phase, "Starting"),
            InstancePhase::Running => self.render_running_control(user_id, cx),
            InstancePhase::Stopping => render_transition_control(user_id, phase, "Stopping"),
        }
    }

    fn render_idle_actions(
        &self,
        user_id: u64,
        session_health: SessionHealth,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let launch_action = HoveredAction {
            account_id: user_id,
            action: AccountAction::Launch,
        };
        let remove_action = HoveredAction {
            account_id: user_id,
            action: AccountAction::Remove,
        };
        let launch_hovered = self.hovered_action == Some(launch_action);
        let remove_hovered = self.hovered_action == Some(remove_action);
        let launch_tooltip_visible = self.visible_tooltip == Some(launch_action);
        let remove_tooltip_visible = self.visible_tooltip == Some(remove_action);
        let launch_enabled = session_health.can_launch();
        let launch_checking = session_health == SessionHealth::Checking;
        let (launch_tooltip, launch_tooltip_width, launch_tooltip_warm) =
            launch_tooltip_copy(session_health);
        let launch_icon = if launch_checking {
            svg()
                .path("spinner.svg")
                .size(px(14.0))
                .text_color(rgb(theme().spinner_muted))
                .with_animation(
                    ("session-check-spinner", user_id),
                    Animation::new(Duration::from_millis(760)).repeat(),
                    |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                )
                .into_any_element()
        } else {
            svg()
                .path("launch.svg")
                .size(px(16.0))
                .text_color(rgb(if launch_enabled {
                    if launch_hovered {
                        theme().launch_icon_hover
                    } else {
                        theme().action_icon
                    }
                } else {
                    theme().disabled_icon
                }))
                .into_any_element()
        };

        div()
            .w(px(ACTION_SLOT_WIDTH))
            .h(px(ACTION_HEIGHT))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .justify_end()
            .child(
                div()
                    .h(px(ACTION_HEIGHT))
                    .w(px(ACTION_CONTROL_WIDTH))
                    .relative()
                    .rounded(px(9.0))
                    .border_1()
                    .border_color(rgb(if launch_enabled && launch_hovered {
                        theme().launch_border
                    } else if remove_hovered {
                        theme().remove_border
                    } else {
                        theme().divider
                    }))
                    .bg(rgb(theme().action_idle))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .id(("launch-account", user_id))
                            .h(px(ACTION_SEGMENT_HEIGHT))
                            .w(px(ACTION_SEGMENT_WIDTH))
                            .flex_none()
                            .rounded_tl(px(8.0))
                            .rounded_bl(px(8.0))
                            .bg(rgb(if launch_enabled && launch_hovered {
                                theme().launch_hover
                            } else {
                                theme().action_idle
                            }))
                            .cursor(if launch_enabled {
                                CursorStyle::PointingHand
                            } else {
                                CursorStyle::Arrow
                            })
                            .when(launch_enabled, |button| {
                                button
                                    .when_some(self.launch_focus.get(&user_id), |button, focus| {
                                        button.track_focus(focus)
                                    })
                                    .focus(|style| style.bg(rgb(theme().launch_focus)))
                                    .active(|style| style.bg(rgb(theme().launch_pressed)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.begin_instance_transition(user_id, cx);
                                    }))
                            })
                            .on_hover(cx.listener(move |this, hovered, _, cx| {
                                this.set_action_hover(launch_action, *hovered, cx);
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(launch_icon),
                    )
                    .child(
                        div()
                            .id(("remove-account", user_id))
                            .h(px(ACTION_SEGMENT_HEIGHT))
                            .w(px(ACTION_SEGMENT_WIDTH))
                            .flex_none()
                            .rounded_tr(px(8.0))
                            .rounded_br(px(8.0))
                            .bg(rgb(if remove_hovered {
                                theme().remove_hover
                            } else {
                                theme().action_idle
                            }))
                            .cursor(CursorStyle::PointingHand)
                            .focusable()
                            .focus(|style| style.bg(rgb(theme().remove_focus)))
                            .active(|style| style.bg(rgb(theme().remove_pressed)))
                            .on_hover(cx.listener(move |this, hovered, _, cx| {
                                this.set_action_hover(remove_action, *hovered, cx);
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.request_removal(user_id, window.focused(cx), cx);
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(svg().path("remove.svg").size(px(16.0)).text_color(rgb(
                                if remove_hovered {
                                    theme().remove_icon_hover
                                } else {
                                    theme().action_icon
                                },
                            ))),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(9.0))
                            .left(px(58.0))
                            .h(px(16.0))
                            .w(px(1.0))
                            .bg(rgb(if launch_enabled && launch_hovered {
                                theme().launch_hover
                            } else if remove_hovered {
                                theme().remove_hover
                            } else {
                                theme().divider
                            })),
                    ),
            )
            .when(launch_tooltip_visible, |actions| {
                actions.child(
                    deferred(
                        div()
                            .absolute()
                            .top(px(42.0))
                            .left(px(36.5 - launch_tooltip_width / 2.0))
                            .h(px(28.0))
                            .w(px(launch_tooltip_width))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(if launch_tooltip_warm {
                                theme().explainer_border
                            } else {
                                theme().tooltip_border
                            }))
                            .bg(rgb(if launch_tooltip_warm {
                                theme().explainer_surface
                            } else {
                                theme().tooltip_surface
                            }))
                            .text_color(rgb(if launch_tooltip_warm {
                                theme().tooltip_warm_text
                            } else {
                                theme().tooltip_text
                            }))
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(launch_tooltip),
                    )
                    .with_priority(10),
                )
            })
            .when(remove_tooltip_visible, |actions| {
                actions.child(
                    deferred(
                        div()
                            .absolute()
                            .top(px(42.0))
                            .left(px(41.0))
                            .h(px(28.0))
                            .w(px(108.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(theme().explainer_border))
                            .bg(rgb(theme().explainer_surface))
                            .text_color(rgb(theme().tooltip_warm_text))
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("Remove account"),
                    )
                    .with_priority(10),
                )
            })
            .into_any_element()
    }

    fn render_sign_in_control(&self, user_id: u64, cx: &mut Context<Self>) -> AnyElement {
        let remove_action = HoveredAction {
            account_id: user_id,
            action: AccountAction::Remove,
        };
        let remove_hovered = self.hovered_action == Some(remove_action);
        let remove_tooltip_visible = self.visible_tooltip == Some(remove_action);

        div()
            .w(px(ACTION_SLOT_WIDTH))
            .h(px(ACTION_HEIGHT))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .justify_end()
            .child(
                div()
                    .h(px(ACTION_HEIGHT))
                    .w(px(ACTION_CONTROL_WIDTH))
                    .relative()
                    .rounded(px(9.0))
                    .border_1()
                    .border_color(rgb(if remove_hovered {
                        theme().warning_remove_border
                    } else {
                        theme().warning_border
                    }))
                    .bg(rgb(theme().warning_surface))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .id(("sign-in-account", user_id))
                            .h(px(ACTION_SEGMENT_HEIGHT))
                            .w(px(SIGN_IN_SEGMENT_WIDTH))
                            .flex_none()
                            .rounded_tl(px(8.0))
                            .rounded_bl(px(8.0))
                            .bg(rgb(theme().warning_surface))
                            .text_color(rgb(theme().warning_strong_text))
                            .text_size(px(STATE_TEXT_SIZE))
                            .font_weight(FontWeight::MEDIUM)
                            .cursor(CursorStyle::PointingHand)
                            .when_some(self.launch_focus.get(&user_id), |button, focus| {
                                button.track_focus(focus)
                            })
                            .focus(|style| style.bg(rgb(theme().warning_hover)))
                            .hover(|style| style.bg(rgb(theme().warning_hover)))
                            .active(|style| style.bg(rgb(theme().warning_pressed)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request_reauthentication(user_id, cx);
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .gap(px(6.0))
                            .child(
                                svg()
                                    .path("signin.svg")
                                    .size(px(14.0))
                                    .text_color(rgb(theme().warning_strong_text)),
                            )
                            .child("Sign in"),
                    )
                    .child(
                        div()
                            .id(("remove-expired-account", user_id))
                            .h(px(ACTION_SEGMENT_HEIGHT))
                            .w(px(SIGN_IN_REMOVE_WIDTH))
                            .flex_none()
                            .rounded_tr(px(8.0))
                            .rounded_br(px(8.0))
                            .bg(rgb(if remove_hovered {
                                theme().warning_remove_hover
                            } else {
                                theme().warning_surface
                            }))
                            .cursor(CursorStyle::PointingHand)
                            .focusable()
                            .focus(|style| style.bg(rgb(theme().warning_remove_hover)))
                            .active(|style| style.bg(rgb(theme().warning_remove_pressed)))
                            .on_hover(cx.listener(move |this, hovered, _, cx| {
                                this.set_action_hover(remove_action, *hovered, cx);
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.request_removal(user_id, window.focused(cx), cx);
                            }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(svg().path("remove.svg").size(px(16.0)).text_color(rgb(
                                if remove_hovered {
                                    theme().remove_icon_hover
                                } else {
                                    theme().warning_icon
                                },
                            ))),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(9.0))
                            .left(px(SIGN_IN_SEGMENT_WIDTH + 1.0))
                            .h(px(16.0))
                            .w(px(1.0))
                            .bg(rgb(if remove_hovered {
                                theme().warning_remove_hover
                            } else {
                                theme().warning_border
                            })),
                    ),
            )
            .when(remove_tooltip_visible, |actions| {
                actions.child(
                    deferred(
                        div()
                            .absolute()
                            .top(px(42.0))
                            .left(px(51.0))
                            .h(px(28.0))
                            .w(px(108.0))
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(rgb(theme().explainer_border))
                            .bg(rgb(theme().explainer_surface))
                            .text_color(rgb(theme().tooltip_warm_text))
                            .text_size(px(11.0))
                            .font_weight(FontWeight::MEDIUM)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("Remove account"),
                    )
                    .with_priority(10),
                )
            })
            .into_any_element()
    }

    fn render_running_control(&self, user_id: u64, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w(px(ACTION_SLOT_WIDTH))
            .h(px(ACTION_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .justify_end()
            .child(
                div()
                    .id(("stop-instance", user_id))
                    .h(px(ACTION_HEIGHT))
                    .w(px(ACTION_CONTROL_WIDTH))
                    .rounded(px(9.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(theme().running_border))
                    .bg(rgb(theme().running_surface))
                    .text_color(rgb(theme().running_text))
                    .text_size(px(STATE_TEXT_SIZE))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor(CursorStyle::PointingHand)
                    .focusable()
                    .focus(|style| style.border_color(rgb(theme().focus_border)))
                    .hover(|style| {
                        style
                            .bg(rgb(theme().running_hover))
                            .border_color(rgb(theme().running_hover_border))
                    })
                    .active(|style| style.bg(rgb(theme().running_pressed)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.begin_instance_transition(user_id, cx);
                    }))
                    .child(render_state_content(
                        svg()
                            .path("stop.svg")
                            .size(px(14.0))
                            .text_color(rgb(theme().running_text))
                            .into_any_element(),
                        "Stop",
                        RUNNING_CONTENT_OFFSET,
                    )),
            )
            .into_any_element()
    }
}

fn launch_tooltip_copy(session_health: SessionHealth) -> (&'static str, f32, bool) {
    match session_health {
        SessionHealth::Checking => ("Checking sign-in", 112.0, false),
        SessionHealth::Valid | SessionHealth::CheckUnavailable => ("Launch account", 104.0, false),
        SessionHealth::LoginRequired => ("Sign in again to launch", 144.0, true),
        SessionHealth::CredentialUnavailable => ("Sign-in unavailable", 128.0, true),
    }
}

fn render_transition_control(
    user_id: u64,
    phase: InstancePhase,
    label: &'static str,
) -> AnyElement {
    let animation_id = match phase {
        InstancePhase::Starting => "starting-instance-spinner",
        InstancePhase::Stopping => "stopping-instance-spinner",
        _ => unreachable!("transition control requires a transitional phase"),
    };

    div()
        .w(px(ACTION_SLOT_WIDTH))
        .h(px(ACTION_HEIGHT))
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .child(
            div()
                .h(px(ACTION_HEIGHT))
                .w(px(ACTION_CONTROL_WIDTH))
                .rounded(px(9.0))
                .overflow_hidden()
                .border_1()
                .border_color(rgb(theme().transition_border))
                .bg(rgb(theme().transition_surface))
                .text_color(rgb(theme().transition_text))
                .text_size(px(STATE_TEXT_SIZE))
                .font_weight(FontWeight::MEDIUM)
                .child(render_state_content(
                    svg()
                        .path("spinner.svg")
                        .size(px(14.0))
                        .text_color(rgb(theme().transition_icon))
                        .with_animation(
                            (animation_id, user_id),
                            Animation::new(Duration::from_millis(700)).repeat(),
                            |svg, delta| {
                                svg.with_transformation(Transformation::rotate(percentage(delta)))
                            },
                        )
                        .into_any_element(),
                    label,
                    0.0,
                )),
        )
        .into_any_element()
}

fn render_state_content(
    icon: AnyElement,
    label: &'static str,
    horizontal_offset: f32,
) -> AnyElement {
    div()
        .size_full()
        .relative()
        .child(
            div()
                .absolute()
                .top(px(10.0))
                .left(px(22.5 + horizontal_offset))
                .size(px(14.0))
                .child(icon),
        )
        .child(
            div()
                .absolute()
                .top_0()
                .left(px(42.0 + horizontal_offset))
                .h_full()
                .w(px(74.0 - horizontal_offset))
                .overflow_hidden()
                .flex()
                .items_center()
                .child(label),
        )
        .into_any_element()
}
