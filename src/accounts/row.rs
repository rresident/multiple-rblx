use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, CursorStyle, FontWeight, ObjectFit,
    StyledImage, deferred, div, img, prelude::*, px, rgb, svg,
};

use crate::theme::{reduce_motion, theme};

use super::{
    AccountAction, ConnectedAccounts, HoveredAction,
    model::{ConnectedAccount, SessionHealth},
};

const LINK_HIGHLIGHT_DURATION: Duration = Duration::from_millis(1_060);
const LINK_HOLD_RATIO: f32 = 900.0 / 1_060.0;

const EXPLAINER_WIDTH: f32 = 254.0;

impl ConnectedAccounts {
    pub(super) fn render_row(
        &self,
        account: ConnectedAccount,
        index: usize,
        account_count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let user_id = account.id;
        let phase = self.store.instance_phase(user_id).unwrap_or_default();
        let added_at = account.formatted_added_at();
        let avatar = render_avatar(&account);
        let metadata = self.render_account_metadata(&account, cx);
        let actions = self.render_actions(user_id, phase, account.session_health, cx);
        let newly_linked = self.newly_linked == Some(user_id);

        let row = div()
            .id(("account-row", user_id))
            .h(px(76.0))
            .w_full()
            .px(px(18.0))
            .flex()
            .items_center()
            .border_b_1()
            .border_color(if index + 1 < account_count {
                rgb(theme().divider)
            } else {
                rgb(theme().surface)
            })
            .when(index + 1 == account_count, |row| {
                row.rounded_bl(px(13.0)).rounded_br(px(13.0))
            })
            .hover(|style| style.bg(rgb(theme().row_hover)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(320.0))
                    .flex()
                    .items_center()
                    .child(avatar)
                    .child(
                        div()
                            .ml(px(12.0))
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(theme().text_primary))
                                    .child(account.username),
                            )
                            .child(metadata),
                    ),
            )
            .child(
                div()
                    .w(px(156.0))
                    .flex_none()
                    .text_size(px(12.5))
                    .text_color(rgb(theme().text_secondary))
                    .child(added_at),
            )
            .child(actions);

        if newly_linked && !reduce_motion() {
            row.with_animation(
                ("linked-account-highlight", user_id),
                Animation::new(LINK_HIGHLIGHT_DURATION),
                |row, delta| {
                    let fade =
                        ((delta - LINK_HOLD_RATIO) / (1.0 - LINK_HOLD_RATIO)).clamp(0.0, 1.0);
                    row.bg(rgb(blend_rgb(
                        theme().link_highlight,
                        theme().surface,
                        fade,
                    )))
                },
            )
            .into_any_element()
        } else {
            row.into_any_element()
        }
    }

    fn render_account_metadata(
        &self,
        account: &ConnectedAccount,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let metadata = div()
            .min_w(px(0.0))
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(px(11.5))
            .text_color(rgb(theme().text_metadata))
            .flex()
            .items_center()
            .child(account.id.to_string());

        let status = match account.session_health {
            SessionHealth::Checking => Some(("Checking sign-in", theme().text_tertiary)),
            SessionHealth::Valid => None,
            SessionHealth::CheckUnavailable => {
                Some(("Sign-in check unavailable", theme().text_metadata))
            }
            SessionHealth::CredentialUnavailable => {
                Some(("Sign-in unavailable", theme().warning_text))
            }
            SessionHealth::LoginRequired => Some(("Sign-in expired", theme().warning_text)),
        };

        let metadata = metadata.when_some(status, |metadata, (label, color)| {
            metadata.child(div().mx(px(4.0)).child("·")).child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(color))
                    .child(label),
            )
        });

        if account.session_health == SessionHealth::LoginRequired {
            return div()
                .flex()
                .items_center()
                .min_w(px(0.0))
                .child(metadata)
                .child(self.render_session_explainer(account.id, cx))
                .into_any_element();
        }

        metadata.into_any_element()
    }

    fn render_session_explainer(
        &self,
        account_id: u64,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let info_action = HoveredAction {
            account_id,
            action: AccountAction::SessionInfo,
        };
        let hovered = self.hovered_action == Some(info_action);
        let open = self.visible_tooltip == Some(info_action);

        div()
            .id(("session-explainer", account_id))
            .ml(px(5.0))
            .flex_none()
            .relative()
            .flex()
            .items_center()
            .justify_center()
            .size(px(15.0))
            .cursor(CursorStyle::PointingHand)
            .on_hover(cx.listener(move |this, hovered, _, cx| {
                this.set_action_hover(info_action, *hovered, cx);
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_session_info(account_id, cx);
            }))
            .child(
                svg()
                    .path("info.svg")
                    .size(px(13.0))
                    .text_color(rgb(if hovered || open {
                        theme().info_icon_active
                    } else {
                        theme().info_icon
                    })),
            )
            .when(open, |icon| {
                icon.child(deferred(session_explainer_popover()).with_priority(10))
            })
    }
}

fn session_explainer_popover() -> AnyElement {
    div()
        .absolute()
        .top(px(22.0))
        .left(px(-9.0))
        .w(px(EXPLAINER_WIDTH))
        .px(px(12.0))
        .py(px(10.0))
        .rounded(px(9.0))
        .border_1()
        .border_color(rgb(theme().explainer_border))
        .bg(rgb(theme().explainer_surface))
        .flex()
        .flex_col()
        .child(
            div()
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme().explainer_title))
                .child("Sign-in expired"),
        )
        .child(
            div()
                .mt(px(3.0))
                .text_size(px(11.0))
                .line_height(px(16.0))
                .text_color(rgb(theme().explainer_body))
                .child(
                    "Roblox no longer accepts this account's saved sign-in. Choose Sign in to reconnect it.",
                ),
        )
        .into_any_element()
}

fn blend_rgb(from: u32, to: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let from = ((from >> shift) & 0xff_u32) as f32;
        let to = ((to >> shift) & 0xff_u32) as f32;
        (from + (to - from) * amount).round() as u32
    };

    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

fn render_avatar(account: &ConnectedAccount) -> AnyElement {
    if let Some(image) = account.avatar.clone() {
        return div()
            .size(px(44.0))
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
                    .size(px(40.0))
                    .rounded_full()
                    .object_fit(ObjectFit::Cover),
            )
            .into_any_element();
    }

    let initial = account
        .username
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();

    div()
        .size(px(44.0))
        .flex_none()
        .rounded_full()
        .border_1()
        .border_color(rgb(theme().strong_border))
        .bg(rgb(theme().border))
        .text_color(rgb(theme().text_secondary))
        .text_size(px(14.0))
        .font_weight(FontWeight::SEMIBOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(initial)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linked_row_highlight_blends_to_the_normal_surface() {
        assert_eq!(
            blend_rgb(theme().link_highlight, theme().surface, 0.0),
            theme().link_highlight
        );
        assert_eq!(
            blend_rgb(theme().link_highlight, theme().surface, 1.0),
            theme().surface
        );
    }
}
