use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, CursorStyle, FontWeight, KeyDownEvent,
    Transformation, Window, deferred, div, percentage, prelude::*, px, rgb, rgba, svg,
};

use crate::{
    accounts::ReauthenticateAccountRequested,
    linking::{LinkAttempt, LinkCancelStatus, LinkEvent, LinkFailure, LinkRequest, LinkResult},
    theme::theme,
};

use super::{Dashboard, LinkDialogState, PendingLink};

const SLOW_OPEN_DELAY: Duration = Duration::from_secs(4);
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

impl Dashboard {
    pub(super) fn begin_linking(&mut self, cx: &mut Context<Self>) {
        self.begin_link(LinkRequest::Add, cx);
    }

    pub(super) fn begin_reauthentication(
        &mut self,
        request: ReauthenticateAccountRequested,
        cx: &mut Context<Self>,
    ) {
        self.begin_link(
            LinkRequest::Reauthenticate {
                token: request.token,
                account: request.account,
            },
            cx,
        );
    }

    fn begin_link(&mut self, request: LinkRequest, cx: &mut Context<Self>) {
        let generation = self.next_link_generation();
        match LinkAttempt::start(self.services.clone(), request.clone()) {
            Ok(attempt) => {
                let events = attempt.events();
                let ready_events = attempt.ready_events();
                self.pending_link = Some(PendingLink {
                    generation,
                    request,
                    state: LinkDialogState::Opening,
                    attempt: Some(attempt),
                });
                self.focus_link_on_next_frame = true;
                cx.notify();

                cx.spawn(async move |this, cx| {
                    gpui::Timer::after(SLOW_OPEN_DELAY).await;
                    let _ = this.update(cx, |this, cx| {
                        let Some(pending) = this.pending_link.as_mut() else {
                            return;
                        };
                        if pending.generation != generation {
                            return;
                        }

                        if pending.state == LinkDialogState::Opening {
                            pending.state = LinkDialogState::OpeningSlow;
                            cx.notify();
                        }
                    });
                })
                .detach();

                cx.spawn(async move |this, cx| {
                    gpui::Timer::after(OPEN_TIMEOUT).await;
                    let _ = this.update(cx, |this, cx| {
                        let Some(pending) = this.pending_link.as_mut() else {
                            return;
                        };
                        if pending.generation != generation
                            || !matches!(
                                pending.state,
                                LinkDialogState::Opening | LinkDialogState::OpeningSlow
                            )
                        {
                            return;
                        }

                        if let Some(attempt) = pending.attempt.as_ref() {
                            let _ = attempt.cancel();
                        }
                        pending.state = LinkDialogState::Failed(LinkFailure::BrowserUnavailable);
                        this.focus_link_on_next_frame = true;
                        cx.notify();
                    });
                })
                .detach();

                cx.spawn(async move |this, cx| {
                    if ready_events.recv().await.is_ok() {
                        let _ = this.update(cx, |this, cx| {
                            this.handle_link_event(generation, LinkEvent::BrowserReady, cx);
                        });
                    }
                })
                .detach();

                cx.spawn(async move |this, cx| {
                    while let Ok(event) = events.recv().await {
                        let terminal = matches!(event, LinkEvent::Finished(_));
                        let _ = this.update(cx, |this, cx| {
                            this.handle_link_event(generation, event, cx);
                        });
                        if terminal {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(failure) => {
                self.pending_link = Some(PendingLink {
                    generation,
                    request,
                    state: LinkDialogState::Failed(failure),
                    attempt: None,
                });
                self.focus_link_on_next_frame = true;
                cx.notify();
            }
        }
    }

    pub(super) fn render_link_dialog(
        &self,
        state: LinkDialogState,
        request: &LinkRequest,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let copy = DialogCopy::for_state(&state, request);
        let is_progress = !matches!(state, LinkDialogState::Failed(_));
        let is_saving = state == LinkDialogState::Saving;

        deferred(
            div()
                .id("connect-account-modal")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgba(theme().scrim))
                .occlude()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    match event.keystroke.key.as_str() {
                        "escape" => {
                            this.close_linking(window, cx);
                            cx.stop_propagation();
                        }
                        "tab" => {
                            this.cycle_link_focus(event.keystroke.modifiers.shift, window);
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
                        .h(px(238.0))
                        .p(px(24.0))
                        .rounded(px(16.0))
                        .border_1()
                        .border_color(rgb(theme().strong_border))
                        .bg(rgb(theme().dialog_surface))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .h(px(21.0))
                                .text_size(px(17.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme().text_primary))
                                .child(copy.title),
                        )
                        .child(
                            div()
                                .mt(px(8.0))
                                .h(px(36.0))
                                .text_size(px(12.5))
                                .line_height(px(18.0))
                                .text_color(rgb(theme().text_secondary))
                                .child(copy.body),
                        )
                        .child(status_band(&state, copy.status))
                        .child(div().flex_1())
                        .child(
                            div()
                                .h(px(38.0))
                                .w_full()
                                .flex()
                                .items_center()
                                .justify_end()
                                .when(!is_saving, |actions| {
                                    actions.child(
                                        div()
                                            .id("close-account-linking")
                                            .track_focus(&self.link_cancel_focus)
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
                                                this.close_linking(window, cx);
                                            }))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child(if is_progress { "Cancel" } else { "Close" }),
                                    )
                                })
                                .when(!is_progress, |actions| {
                                    actions.child(
                                        div()
                                            .id("account-linking-secondary")
                                            .track_focus(&self.link_secondary_focus)
                                            .ml(px(8.0))
                                            .h(px(38.0))
                                            .w(px(104.0))
                                            .rounded(px(9.0))
                                            .border_1()
                                            .border_color(rgb(theme().emphasis_border))
                                            .bg(rgb(theme().emphasis_surface))
                                            .text_color(rgb(theme().emphasis_text))
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::MEDIUM)
                                            .cursor(CursorStyle::PointingHand)
                                            .focus(|style| {
                                                style.border_color(rgb(
                                                    theme().emphasis_focus_border
                                                ))
                                            })
                                            .hover(|style| {
                                                style
                                                    .bg(rgb(theme().emphasis_hover))
                                                    .border_color(
                                                        rgb(theme().emphasis_hover_border),
                                                    )
                                                    .text_color(rgb(theme().text_primary))
                                            })
                                            .active(|style| style.bg(rgb(theme().emphasis_pressed)))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.retry_linking(window, cx);
                                            }))
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .child("Try again"),
                                    )
                                }),
                        ),
                ),
        )
        .with_priority(100)
        .into_any_element()
    }

    fn handle_link_event(&mut self, generation: u64, event: LinkEvent, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_link.as_mut() else {
            return;
        };
        if pending.generation != generation {
            return;
        }

        match event {
            LinkEvent::BrowserReady => {
                if matches!(
                    pending.state,
                    LinkDialogState::Opening | LinkDialogState::OpeningSlow
                ) {
                    pending.state = LinkDialogState::Waiting;
                    cx.notify();
                }
            }
            LinkEvent::CheckingAccount => {
                pending.state = LinkDialogState::Checking;
                cx.notify();
            }
            LinkEvent::SavingAccount => {
                pending.state = LinkDialogState::Saving;
                cx.notify();
            }
            LinkEvent::Finished(Ok(result)) => {
                let account_id = result.account_id();
                let accepted = pending
                    .attempt
                    .as_ref()
                    .is_some_and(|attempt| attempt.accept(account_id));
                if !accepted {
                    return;
                }

                match result {
                    LinkResult::Added(account) => self
                        .accounts
                        .update(cx, |accounts, cx| accounts.add_linked_account(account, cx)),
                    LinkResult::Reauthenticated { token, account } => {
                        let applied = self.accounts.update(cx, |accounts, cx| {
                            accounts.apply_reauthentication(token, account, cx)
                        });
                        if !applied {
                            tracing::warn!(
                                account_id,
                                "ignored reauthentication for a stale account row"
                            );
                        }
                    }
                }
                self.pending_link = None;
                self.next_link_generation();
                cx.notify();
            }
            LinkEvent::Finished(Err(LinkFailure::Cancelled)) => {
                if matches!(pending.state, LinkDialogState::Failed(_)) {
                    return;
                }
                self.pending_link = None;
                self.next_link_generation();
                cx.notify();
            }
            LinkEvent::Finished(Err(failure)) => {
                if pending.state == LinkDialogState::Saving
                    && let LinkRequest::Reauthenticate { token, .. } = &pending.request
                {
                    self.accounts.update(cx, |accounts, cx| {
                        accounts.recheck_session(token.clone(), cx)
                    });
                }
                pending.state = LinkDialogState::Failed(failure);
                self.focus_link_on_next_frame = true;
                cx.notify();
            }
        }
    }

    fn close_linking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut pending) = self.pending_link.take() else {
            return;
        };
        if let Some(attempt) = pending.attempt.as_ref() {
            match attempt.cancel() {
                Ok(LinkCancelStatus::Cancelled) => {}
                Ok(LinkCancelStatus::Finishing) => {
                    pending.state = LinkDialogState::Saving;
                    self.pending_link = Some(pending);
                    cx.notify();
                    return;
                }
                Err(failure) => {
                    pending.state = LinkDialogState::Failed(failure);
                    self.pending_link = Some(pending);
                    self.focus_link_on_next_frame = true;
                    cx.notify();
                    return;
                }
            }
        }

        self.next_link_generation();
        window.focus(&self.accounts.read(cx).section_focus());
        cx.notify();
    }

    fn retry_linking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let request = self
            .pending_link
            .as_ref()
            .map(|pending| pending.request.clone());
        self.close_linking(window, cx);
        if self.pending_link.is_none()
            && let Some(request) = request
        {
            self.begin_link(request, cx);
        }
    }

    fn cycle_link_focus(&self, backwards: bool, window: &mut Window) {
        if self
            .pending_link
            .as_ref()
            .is_some_and(|pending| pending.state == LinkDialogState::Saving)
        {
            return;
        }
        let has_secondary_action = self
            .pending_link
            .as_ref()
            .is_some_and(|pending| matches!(pending.state, LinkDialogState::Failed(_)));
        if !has_secondary_action {
            window.focus(&self.link_cancel_focus);
            return;
        }

        let cancel_is_focused = self.link_cancel_focus.is_focused(window);
        let target = if backwards {
            if cancel_is_focused {
                &self.link_secondary_focus
            } else {
                &self.link_cancel_focus
            }
        } else if cancel_is_focused {
            &self.link_secondary_focus
        } else {
            &self.link_cancel_focus
        };
        window.focus(target);
    }

    fn next_link_generation(&mut self) -> u64 {
        self.link_generation = self
            .link_generation
            .checked_add(1)
            .expect("link generation exhausted");
        self.link_generation
    }
}

struct DialogCopy {
    title: String,
    body: String,
    status: &'static str,
}

impl DialogCopy {
    fn for_state(state: &LinkDialogState, request: &LinkRequest) -> Self {
        let title = match request {
            LinkRequest::Add => "Connect Roblox account".into(),
            LinkRequest::Reauthenticate { account, .. } => {
                format!("Reconnect {}", account.username)
            }
        };
        match state {
            LinkDialogState::Opening => Self {
                title,
                body: "Roblox sign-in is opening in a separate window.".into(),
                status: "Opening sign-in",
            },
            LinkDialogState::OpeningSlow => Self {
                title,
                body: "Roblox sign-in is opening in a separate window.".into(),
                status: "Still opening sign-in",
            },
            LinkDialogState::Waiting => Self {
                title,
                body: match request {
                    LinkRequest::Add => "Sign in through the Roblox window.".into(),
                    LinkRequest::Reauthenticate { account, .. } => {
                        format!("Sign in as {} through the Roblox window.", account.username)
                    }
                },
                status: "Waiting for sign-in",
            },
            LinkDialogState::Checking => Self {
                title,
                body: "Sign-in complete.".into(),
                status: "Checking account",
            },
            LinkDialogState::Saving => Self {
                title,
                body: "The account matched.".into(),
                status: "Saving sign-in",
            },
            LinkDialogState::Failed(LinkFailure::Cancelled) => Self {
                title: "Sign-in cancelled".into(),
                body: "No account was added.".into(),
                status: "Cancelled",
            },
            LinkDialogState::Failed(LinkFailure::Duplicate { username }) => Self {
                title: "Account already connected".into(),
                body: format!("{username} is already in your connected accounts."),
                status: "Use a different account",
            },
            LinkDialogState::Failed(LinkFailure::WrongAccount { username }) => Self {
                title: "Different Roblox account".into(),
                body: format!("Sign in as {username} to reconnect this account."),
                status: "Wrong account",
            },
            LinkDialogState::Failed(LinkFailure::Conflict) => Self {
                title: "Account changed".into(),
                body: "The saved sign-in changed. Try again.".into(),
                status: "Not replaced",
            },
            LinkDialogState::Failed(LinkFailure::SignInRejected) => Self {
                title: "Couldn’t connect account".into(),
                body: "Roblox did not accept the signed-in session. Try signing in again.".into(),
                status: "Sign-in rejected",
            },
            LinkDialogState::Failed(LinkFailure::Network) => Self {
                title: "Couldn’t reach Roblox".into(),
                body: "Check your connection, then try again.".into(),
                status: "Connection unavailable",
            },
            LinkDialogState::Failed(LinkFailure::SecureStorage) => Self {
                title: "Couldn’t save account".into(),
                body: "Windows Credential Manager is unavailable.".into(),
                status: "Credential Manager unavailable",
            },
            LinkDialogState::Failed(LinkFailure::BrowserUnavailable) => Self {
                title: "Couldn’t open Roblox".into(),
                body: "Multiple Roblox needs Microsoft WebView2 for sign-in. Install or repair it, then try again."
                    .into(),
                status: "WebView2 is required",
            },
            LinkDialogState::Failed(LinkFailure::Browser) => Self {
                title: "Sign-in window closed".into(),
                body: "The Roblox sign-in window stopped unexpectedly. Try again.".into(),
                status: "Window closed",
            },
        }
    }
}

fn status_band(state: &LinkDialogState, status: &'static str) -> AnyElement {
    let is_progress = !matches!(state, LinkDialogState::Failed(_));

    div()
        .mt(px(16.0))
        .h(px(48.0))
        .w_full()
        .px(px(14.0))
        .rounded(px(11.0))
        .border_1()
        .border_color(rgb(theme().border))
        .bg(rgb(theme().inset_surface))
        .flex()
        .items_center()
        .child(if is_progress {
            div()
                .size(px(28.0))
                .flex_none()
                .rounded_full()
                .border_1()
                .border_color(rgb(theme().status_border))
                .bg(rgb(theme().status_surface))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path("spinner.svg")
                        .size(px(14.0))
                        .text_color(rgb(theme().status_text))
                        .with_animation(
                            "account-linking-spinner",
                            Animation::new(Duration::from_millis(760)).repeat(),
                            |svg, delta| {
                                svg.with_transformation(Transformation::rotate(percentage(delta)))
                            },
                        ),
                )
                .into_any_element()
        } else {
            div()
                .size(px(28.0))
                .flex_none()
                .rounded_full()
                .border_1()
                .border_color(rgb(theme().status_error_border))
                .bg(rgb(theme().status_error_surface))
                .text_color(rgb(theme().warning_text))
                .text_size(px(12.0))
                .font_weight(FontWeight::SEMIBOLD)
                .flex()
                .items_center()
                .justify_center()
                .child("!")
                .into_any_element()
        })
        .child(
            div()
                .ml(px(12.0))
                .min_w(px(0.0))
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme().text_primary))
                .child(status),
        )
        .into_any_element()
}
