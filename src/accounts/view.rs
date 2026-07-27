use gpui::{
    AnyElement, Context, CursorStyle, FontWeight, Render, SharedString, Window, div, prelude::*,
    px, rgb, svg,
};

use crate::theme::theme;

use super::{ConnectedAccounts, SyncState};

impl Render for ConnectedAccounts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(account_id) = self.focus_launch_on_next_frame.take()
            && let Some(focus) = self.launch_focus.get(&account_id).cloned()
        {
            window.on_next_frame(move |window, _| window.focus(&focus));
        }

        let accounts = self.store.visible().to_vec();
        let account_count = accounts.len();
        let summary = account_summary(self.sync_state, account_count, self.status_error.as_ref());
        let rows = accounts
            .into_iter()
            .enumerate()
            .map(|(index, account)| self.render_row(account, index, account_count, cx))
            .collect::<Vec<AnyElement>>();

        div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(32.0))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .id("connected-accounts-heading")
                            .track_focus(&self.section_focus)
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Connected accounts"),
                    )
                    .child(
                        div()
                            .h_full()
                            .flex()
                            .items_center()
                            .when_some(summary, |right, summary| {
                                right.child(
                                    div()
                                        .mr(px(16.0))
                                        .text_size(px(12.0))
                                        .text_color(rgb(theme().text_tertiary))
                                        .child(summary),
                                )
                            })
                            .child(add_account_button(cx)),
                    ),
            )
            .child(
                div()
                    .mt(px(12.0))
                    .w_full()
                    .rounded(px(14.0))
                    .border_1()
                    .border_color(rgb(theme().border))
                    .bg(rgb(theme().surface))
                    .overflow_hidden()
                    .child(table_header())
                    .when(account_count == 0, |table| table.child(empty_state()))
                    .when(account_count > 0, |table| {
                        table.child(
                            div()
                                .id("connected-account-rows")
                                .max_h(px(380.0))
                                .overflow_y_scroll()
                                .track_scroll(&self.row_scroll)
                                .children(rows),
                        )
                    }),
            )
    }
}

fn add_account_button(cx: &mut Context<ConnectedAccounts>) -> AnyElement {
    div()
        .id("add-account")
        .h(px(32.0))
        .w(px(104.0))
        .flex_none()
        .rounded(px(8.0))
        .bg(rgb(theme().accent_surface))
        .text_color(rgb(theme().accent_text))
        .text_size(px(12.0))
        .font_weight(FontWeight::MEDIUM)
        .cursor(CursorStyle::PointingHand)
        .focusable()
        .focus(|style| style.border_2().border_color(rgb(theme().focus_border)))
        .hover(|style| style.bg(rgb(theme().accent_hover)))
        .active(|style| style.bg(rgb(theme().accent_pressed)))
        .on_click(cx.listener(|this, _, _, cx| this.request_linking(cx)))
        .flex()
        .items_center()
        .justify_center()
        .child("Add account")
        .into_any_element()
}

fn table_header() -> AnyElement {
    div()
        .h(px(38.0))
        .w_full()
        .px(px(18.0))
        .rounded_tl(px(13.0))
        .rounded_tr(px(13.0))
        .border_b_1()
        .border_color(rgb(theme().border))
        .bg(rgb(theme().inset_surface))
        .text_size(px(11.0))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(theme().text_tertiary))
        .flex()
        .items_center()
        .child(div().flex_1().min_w(px(320.0)).child("Account"))
        .child(div().w(px(156.0)).flex_none().child("Added at"))
        .child(
            div()
                .w(px(124.0))
                .flex_none()
                .text_align(gpui::TextAlign::Center)
                .child("Actions"),
        )
        .into_any_element()
}

fn empty_state() -> AnyElement {
    div()
        .h(px(168.0))
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .child(
            svg()
                .path("multiple.svg")
                .size(px(24.0))
                .text_color(rgb(theme().text_tertiary)),
        )
        .child(
            div()
                .mt(px(10.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(theme().text_primary))
                .child("No connected accounts"),
        )
        .child(
            div()
                .mt(px(4.0))
                .text_size(px(11.5))
                .text_color(rgb(theme().text_metadata))
                .child(
                    "Run separate Roblox instances side by side, each signed in to a different account.",
                ),
        )
        .into_any_element()
}

fn account_summary(
    sync_state: SyncState,
    account_count: usize,
    status_error: Option<&SharedString>,
) -> Option<SharedString> {
    if let Some(error) = status_error {
        return Some(error.clone());
    }
    if account_count == 0 {
        return None;
    }

    Some(match sync_state {
        SyncState::Loading => "Refreshing profiles".into(),
        SyncState::Live => format!(
            "{} {}",
            account_count,
            if account_count == 1 {
                "account"
            } else {
                "accounts"
            }
        )
        .into(),
        SyncState::Unavailable => format!(
            "{} {} · profiles unavailable",
            account_count,
            if account_count == 1 {
                "account"
            } else {
                "accounts"
            }
        )
        .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_store_hides_the_summary() {
        assert_eq!(account_summary(SyncState::Live, 0, None), None);
    }

    #[test]
    fn account_summary_handles_loading_plurality_and_sync_failure() {
        assert_eq!(
            account_summary(SyncState::Loading, 3, None),
            Some("Refreshing profiles".into())
        );
        assert_eq!(
            account_summary(SyncState::Live, 1, None),
            Some("1 account".into())
        );
        assert_eq!(
            account_summary(SyncState::Live, 3, None),
            Some("3 accounts".into())
        );
        assert_eq!(
            account_summary(SyncState::Unavailable, 3, None),
            Some("3 accounts · profiles unavailable".into())
        );
    }

    #[test]
    fn secure_storage_error_takes_priority_even_when_empty() {
        let error: SharedString = "Couldn’t open connected accounts".into();
        assert_eq!(
            account_summary(SyncState::Live, 0, Some(&error)),
            Some(error)
        );
    }
}
