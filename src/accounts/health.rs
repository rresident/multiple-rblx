use std::sync::Arc;

use gpui::{AppContext as _, Context};

use super::{ConnectedAccounts, model::SessionCheckToken, service::AccountServices};

impl ConnectedAccounts {
    pub(super) fn schedule_session_checks(
        services: Arc<AccountServices>,
        checks: impl IntoIterator<Item = SessionCheckToken>,
        cx: &mut Context<Self>,
    ) {
        for token in checks {
            let user_id = token.account_id;
            let services = services.clone();
            let task = cx.background_spawn(async move { services.check_session(user_id) });
            cx.spawn(async move |this, cx| {
                let health = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.store.apply_session_health(&token, health) {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }
}
