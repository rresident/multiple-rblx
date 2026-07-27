mod actions;
mod health;
mod instance;
mod model;
mod roblox;
mod row;
pub(crate) mod service;
mod view;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use gpui::{
    AppContext, Context, EventEmitter, FocusHandle, Image, ScrollHandle, SharedString, Timer,
};

use crate::{
    accounts::service::LaunchFailure, launcher::LaunchTarget, security::StoredSessionAccount,
};

use instance::{CLIENT_POLL_INTERVAL, InstancePhase, TRANSITION_DELAY};
use model::AccountStore;
pub(crate) use model::{AccountRowToken, LinkedAccountData};
use service::AccountServices;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SyncState {
    Loading,
    Live,
    Unavailable,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AccountAction {
    Launch,
    Remove,
    SessionInfo,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct HoveredAction {
    account_id: u64,
    action: AccountAction,
}

#[derive(Clone, Copy)]
pub(crate) struct ConnectAccountRequested;

#[derive(Clone, Copy)]
pub(crate) struct LaunchRequested {
    pub(crate) account_id: u64,
}

#[derive(Clone)]
pub(crate) struct ReauthenticateAccountRequested {
    pub(crate) token: AccountRowToken,
    pub(crate) account: StoredSessionAccount,
}

#[derive(Clone)]
pub(crate) struct RemoveAccountRequested {
    pub(crate) account_id: u64,
    pub(crate) username: SharedString,
    pub(crate) avatar: Option<Arc<Image>>,
    pub(crate) return_focus: Option<FocusHandle>,
}

pub(crate) struct ConnectedAccounts {
    services: Arc<AccountServices>,
    store: AccountStore,
    sync_state: SyncState,
    status_error: Option<SharedString>,
    pending_removals: HashSet<u64>,
    hovered_action: Option<HoveredAction>,
    visible_tooltip: Option<HoveredAction>,
    tooltip_generation: u64,
    newly_linked: Option<u64>,
    section_focus: FocusHandle,
    launch_focus: HashMap<u64, FocusHandle>,
    focus_launch_on_next_frame: Option<u64>,
    row_scroll: ScrollHandle,
    watching_clients: bool,
}

impl EventEmitter<ConnectAccountRequested> for ConnectedAccounts {}
impl EventEmitter<LaunchRequested> for ConnectedAccounts {}
impl EventEmitter<ReauthenticateAccountRequested> for ConnectedAccounts {}
impl EventEmitter<RemoveAccountRequested> for ConnectedAccounts {}

impl ConnectedAccounts {
    pub(crate) fn new(services: Arc<AccountServices>, cx: &mut Context<Self>) -> Self {
        let (stored_accounts, status_error) = match services.load_accounts() {
            Ok(accounts) => {
                tracing::info!(account_count = accounts.len(), "loaded connected accounts");
                (accounts, None)
            }
            Err(error) => {
                tracing::error!(reason = %error, "secure account storage could not be loaded");
                (
                    Vec::new(),
                    Some(SharedString::from("Couldn’t open connected accounts")),
                )
            }
        };
        let has_accounts = !stored_accounts.is_empty();
        let user_ids = stored_accounts
            .iter()
            .map(|account| account.user_id)
            .collect::<Vec<_>>();
        let launch_focus = stored_accounts
            .iter()
            .map(|account| (account.user_id, cx.focus_handle().tab_stop(true)))
            .collect();
        let store = AccountStore::new(stored_accounts);

        if has_accounts {
            let checks = user_ids
                .iter()
                .filter_map(|user_id| store.session_check_token(*user_id))
                .collect::<Vec<_>>();
            Self::schedule_session_checks(services.clone(), checks, cx);
            let services_for_refresh = services.clone();
            let profile_user_ids = user_ids.clone();
            let fetch_task = cx.background_spawn(async move {
                services_for_refresh.refresh_profiles(&profile_user_ids)
            });

            cx.spawn(async move |this, cx| {
                let result = fetch_task.await;
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(profiles) => {
                            this.store.apply_profiles(profiles);
                            this.sync_state = SyncState::Live;
                        }
                        Err(error) => {
                            tracing::warn!(
                                reason = %error,
                                "public Roblox profile refresh failed"
                            );
                            this.sync_state = SyncState::Unavailable;
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }

        Self {
            services,
            store,
            sync_state: if has_accounts {
                SyncState::Loading
            } else {
                SyncState::Live
            },
            status_error,
            pending_removals: HashSet::new(),
            hovered_action: None,
            visible_tooltip: None,
            tooltip_generation: 0,
            newly_linked: None,
            section_focus: cx.focus_handle().tab_stop(true),
            launch_focus,
            focus_launch_on_next_frame: None,
            row_scroll: ScrollHandle::new(),
            watching_clients: false,
        }
    }

    fn begin_instance_transition(&mut self, account_id: u64, cx: &mut Context<Self>) {
        if self.pending_removals.contains(&account_id) {
            return;
        }

        match self.store.instance_phase(account_id) {
            Some(InstancePhase::Idle) => {
                if !self
                    .store
                    .session_health(account_id)
                    .is_some_and(|health| health.can_launch())
                {
                    return;
                }
                self.clear_action_interaction();
                cx.emit(LaunchRequested { account_id });
                cx.notify();
            }
            Some(InstancePhase::Running) => self.begin_stop(account_id, cx),
            _ => {}
        }
    }

    fn begin_stop(&mut self, account_id: u64, cx: &mut Context<Self>) {
        let Some(token) = self.store.begin_instance_transition(account_id) else {
            return;
        };

        self.clear_action_interaction();
        self.store.terminate_client(account_id);
        cx.notify();

        cx.spawn(async move |this, cx| {
            Timer::after(TRANSITION_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if this.store.complete_instance_transition(token) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn begin_launch(&mut self, account_id: u64, place_id: u64, cx: &mut Context<Self>) {
        if self.pending_removals.contains(&account_id)
            || self.store.instance_phase(account_id) != Some(InstancePhase::Idle)
            || !self
                .store
                .session_health(account_id)
                .is_some_and(|health| health.can_launch())
        {
            return;
        }
        let Some(token) = self.store.begin_instance_transition(account_id) else {
            return;
        };

        self.clear_action_interaction();
        self.status_error = None;
        cx.notify();

        let services = self.services.clone();
        let task = cx.background_spawn(async move {
            let prepared = services.prepare_launch(account_id)?;
            crate::launcher::launch_client(&prepared, LaunchTarget { place_id }).map_err(|error| {
                tracing::error!(account_id, reason = %error, "Roblox client failed to start");
                LaunchFailure::Unavailable
            })
        });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(client) => {
                        tracing::info!(account_id, pid = client.pid(), "Roblox client started");
                        this.store.attach_client(account_id, client);
                        this.store.complete_instance_transition(token);
                        this.watch_clients(cx);
                    }
                    Err(failure) => {
                        this.store.force_idle(account_id);
                        this.status_error = Some(format!("Couldn't launch: {failure}").into());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn watch_clients(&mut self, cx: &mut Context<Self>) {
        if self.watching_clients {
            return;
        }
        self.watching_clients = true;

        cx.spawn(async move |this, cx| {
            loop {
                Timer::after(CLIENT_POLL_INTERVAL).await;
                let keep_going = this.update(cx, |this, cx| {
                    let exited = this.store.reap_exited_clients();
                    let mut changed = false;
                    for account_id in exited {
                        changed |= this.store.force_idle(account_id);
                    }
                    if changed {
                        cx.notify();
                    }

                    let remaining = this
                        .store
                        .visible()
                        .iter()
                        .any(|account| this.store.has_running_client(account.id));
                    if !remaining {
                        this.watching_clients = false;
                    }
                    remaining
                });

                match keep_going {
                    Ok(true) => {}
                    _ => break,
                }
            }
        })
        .detach();
    }

    fn request_linking(&mut self, cx: &mut Context<Self>) {
        tracing::debug!("account linking requested");
        self.clear_action_interaction();
        cx.emit(ConnectAccountRequested);
        cx.notify();
    }

    fn request_reauthentication(&mut self, account_id: u64, cx: &mut Context<Self>) {
        if self.pending_removals.contains(&account_id) {
            return;
        }
        let Some((token, account)) = self.store.reauthentication_request(account_id) else {
            return;
        };

        self.clear_action_interaction();
        cx.emit(ReauthenticateAccountRequested { token, account });
        cx.notify();
    }

    fn request_removal(
        &mut self,
        account_id: u64,
        return_focus: Option<FocusHandle>,
        cx: &mut Context<Self>,
    ) {
        if self.pending_removals.contains(&account_id)
            || self.store.instance_phase(account_id) != Some(InstancePhase::Idle)
        {
            return;
        }

        let Some(account) = self
            .store
            .visible()
            .iter()
            .find(|account| account.id == account_id)
        else {
            return;
        };
        let request = RemoveAccountRequested {
            account_id,
            username: account.username.clone(),
            avatar: account.avatar.clone(),
            return_focus,
        };

        self.clear_action_interaction();
        cx.emit(request);
        cx.notify();
    }

    pub(crate) fn add_linked_account(
        &mut self,
        account: LinkedAccountData,
        cx: &mut Context<Self>,
    ) {
        let account_id = account.record.user_id;
        if self.store.add(account) {
            let row_index = self.store.visible().len().saturating_sub(1);
            self.row_scroll.scroll_to_item(row_index);
            self.newly_linked = Some(account_id);
            self.launch_focus
                .insert(account_id, cx.focus_handle().tab_stop(true));
            self.focus_launch_on_next_frame = Some(account_id);
            tracing::info!(account_id, "connected account added to dashboard");
            self.sync_state = SyncState::Live;
            self.status_error = None;
            self.clear_action_interaction();
            cx.notify();

            let services = self.services.clone();
            let fetch_task =
                cx.background_spawn(async move { services.refresh_profiles(&[account_id]) });
            cx.spawn(async move |this, cx| {
                let result = fetch_task.await;
                let _ = this.update(cx, |this, cx| {
                    if let Ok(profiles) = result {
                        this.store.apply_profiles(profiles);
                        cx.notify();
                    }
                });
            })
            .detach();

            cx.spawn(async move |this, cx| {
                Timer::after(Duration::from_millis(1_060)).await;
                let _ = this.update(cx, |this, cx| {
                    if this.newly_linked == Some(account_id) {
                        this.newly_linked = None;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    pub(crate) fn apply_reauthentication(
        &mut self,
        token: AccountRowToken,
        account: LinkedAccountData,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.store.apply_reauthentication(&token, account) {
            return false;
        }

        self.status_error = None;
        cx.notify();
        true
    }

    pub(crate) fn recheck_session(&mut self, token: AccountRowToken, cx: &mut Context<Self>) {
        if self
            .store
            .apply_session_health(&token, model::SessionHealth::Checking)
        {
            Self::schedule_session_checks(self.services.clone(), [token], cx);
            cx.notify();
        }
    }

    pub(crate) fn remove_account(&mut self, account_id: u64, cx: &mut Context<Self>) {
        if self.pending_removals.contains(&account_id) || !self.store.begin_removal(account_id) {
            return;
        }
        self.pending_removals.insert(account_id);
        tracing::info!(account_id, "removing connected account");
        let services = self.services.clone();
        let task = cx.background_spawn(async move { services.remove(account_id) });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.pending_removals.remove(&account_id);
                match result {
                    Ok(()) => {
                        tracing::info!(account_id, "connected account removed");
                        this.store.remove(account_id);
                        this.launch_focus.remove(&account_id);
                        if this.focus_launch_on_next_frame == Some(account_id) {
                            this.focus_launch_on_next_frame = None;
                        }
                        this.status_error = None;
                    }
                    Err(error) => {
                        this.store.cancel_removal(account_id);
                        tracing::error!(
                            account_id,
                            reason = %error,
                            "connected account removal failed"
                        );
                        this.status_error = Some("Couldn’t remove account".into());
                    }
                }
                this.clear_action_interaction();
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn section_focus(&self) -> FocusHandle {
        self.section_focus.clone()
    }

    fn set_action_hover(&mut self, action: HoveredAction, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            let generation = self.next_tooltip_generation();
            self.hovered_action = Some(action);
            self.visible_tooltip = None;

            cx.spawn(async move |this, cx| {
                Timer::after(Duration::from_millis(350)).await;
                let _ = this.update(cx, |this, cx| {
                    if this.tooltip_generation == generation && this.hovered_action == Some(action)
                    {
                        this.visible_tooltip = Some(action);
                        cx.notify();
                    }
                });
            })
            .detach();
        } else {
            let was_current = self.hovered_action == Some(action);
            let was_visible = self.visible_tooltip == Some(action);
            if was_current {
                self.hovered_action = None;
            }
            if was_visible {
                self.visible_tooltip = None;
            }
            if was_current || was_visible {
                self.next_tooltip_generation();
            }
        }

        cx.notify();
    }

    fn toggle_session_info(&mut self, account_id: u64, cx: &mut Context<Self>) {
        let action = HoveredAction {
            account_id,
            action: AccountAction::SessionInfo,
        };
        self.next_tooltip_generation();
        self.visible_tooltip = if self.visible_tooltip == Some(action) {
            None
        } else {
            Some(action)
        };
        cx.notify();
    }

    fn clear_action_interaction(&mut self) {
        self.hovered_action = None;
        self.visible_tooltip = None;
        self.next_tooltip_generation();
    }

    fn next_tooltip_generation(&mut self) -> u64 {
        self.tooltip_generation = self
            .tooltip_generation
            .checked_add(1)
            .expect("tooltip generation exhausted");
        self.tooltip_generation
    }
}
