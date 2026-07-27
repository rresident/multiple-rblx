mod components;
mod game_picker;
mod launch_bar;
mod link_dialog;
mod remove_dialog;
mod settings_dialog;
mod titlebar;

use std::sync::Arc;

use gpui::{
    AppContext, Context, Entity, FocusHandle, Image, Render, SharedString, Window, div, prelude::*,
    px, rgb,
};

use crate::{
    accounts::{
        ConnectAccountRequested, ConnectedAccounts, LaunchRequested,
        ReauthenticateAccountRequested, RemoveAccountRequested, service::AccountServices,
    },
    games::{GamesGateway, RobloxGamesClient, browse_session_id},
    launcher::MultiInstanceGuard,
    linking::{LinkAttempt, LinkFailure, LinkRequest},
    settings::{Preferences, SavedGame, SharedSettings, system_settings},
    theme::theme,
};

use game_picker::GamePicker;
use settings_dialog::SettingsSection;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LaunchIntent {
    Account(u64),
    Pin,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LinkDialogState {
    Opening,
    OpeningSlow,
    Waiting,
    Checking,
    Saving,
    Failed(LinkFailure),
}

struct PendingLink {
    generation: u64,
    request: LinkRequest,
    state: LinkDialogState,
    attempt: Option<LinkAttempt>,
}

pub(crate) struct Dashboard {
    services: Arc<AccountServices>,
    accounts: Entity<ConnectedAccounts>,
    settings: SharedSettings,
    preferences: Preferences,
    games: Arc<dyn GamesGateway>,
    browse_session: String,
    selected_game: Option<SavedGame>,
    selected_game_icon: Option<Arc<Image>>,
    multi_instance: Option<MultiInstanceGuard>,
    multi_instance_notice: Option<SharedString>,
    catalogue: Vec<crate::games::GameSummary>,
    icon_cache: std::collections::HashMap<u64, Arc<Image>>,
    picker: Option<GamePicker>,
    settings_section: Option<SettingsSection>,
    confirming_reset: bool,
    picker_generation: u64,
    search_epoch: u64,
    focus_search_on_next_frame: bool,
    pending_removal: Option<RemoveAccountRequested>,
    pending_link: Option<PendingLink>,
    link_generation: u64,
    cancel_focus: FocusHandle,
    confirm_focus: FocusHandle,
    link_cancel_focus: FocusHandle,
    link_secondary_focus: FocusHandle,
    focus_modal_on_next_frame: bool,
    focus_link_on_next_frame: bool,
}

impl Dashboard {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let services = Arc::new(AccountServices::system());
        let accounts = cx.new(|cx| ConnectedAccounts::new(services.clone(), cx));
        cx.subscribe(
            &accounts,
            |dashboard, _accounts, request: &RemoveAccountRequested, cx| {
                if dashboard.pending_link.is_some() {
                    return;
                }
                dashboard.pending_removal = Some(request.clone());
                dashboard.focus_modal_on_next_frame = true;
                cx.notify();
            },
        )
        .detach();
        cx.subscribe(
            &accounts,
            |dashboard, _accounts, request: &ReauthenticateAccountRequested, cx| {
                if dashboard.pending_removal.is_none() && dashboard.pending_link.is_none() {
                    dashboard.begin_reauthentication(request.clone(), cx);
                }
            },
        )
        .detach();
        cx.subscribe(
            &accounts,
            |dashboard, _accounts, _: &ConnectAccountRequested, cx| {
                if dashboard.pending_removal.is_none() && dashboard.pending_link.is_none() {
                    dashboard.begin_linking(cx);
                }
            },
        )
        .detach();
        cx.subscribe(
            &accounts,
            |dashboard, _accounts, request: &LaunchRequested, cx| {
                if dashboard.pending_removal.is_some()
                    || dashboard.pending_link.is_some()
                    || dashboard.picker.is_some()
                {
                    return;
                }
                match dashboard.selected_game.clone() {
                    Some(game) => {
                        let account_id = request.account_id;
                        let place_id = game.root_place_id;
                        dashboard.remember_saved_launch(&game, cx);
                        dashboard.accounts.update(cx, |accounts, cx| {
                            accounts.begin_launch(account_id, place_id, cx);
                        });
                    }
                    None => {
                        dashboard.open_game_picker(LaunchIntent::Account(request.account_id), cx)
                    }
                }
            },
        )
        .detach();

        let settings = system_settings();
        let preferences = match settings.load() {
            Ok(preferences) => preferences,
            Err(error) => {
                tracing::warn!(reason = %error, "preferences could not be read; using defaults");
                Preferences::default()
            }
        };

        let mut multi_instance = None;
        let mut multi_instance_notice = None;
        let mut multiple_instances_enabled = preferences.multiple_instances_enabled;
        if multiple_instances_enabled {
            let (guard, notice) = launch_bar::arm_multi_instance();
            match guard {
                Some(guard) => {
                    tracing::info!("multi-instance guard re-armed at startup");
                    multi_instance = Some(guard);
                }
                None => {
                    multiple_instances_enabled = false;
                    multi_instance_notice = notice;
                }
            }
        }

        let mut preferences = preferences;
        preferences.multiple_instances_enabled = multiple_instances_enabled;
        crate::theme::set_theme(preferences.theme);
        crate::theme::set_reduce_motion(preferences.reduce_motion);
        let selected_game = preferences.selected_game.clone();

        cx.spawn(async move |this, cx| {
            let _ = this.update(cx, |this, cx| this.preload_catalogue(cx));
        })
        .detach();

        Self {
            services,
            accounts,
            settings,
            preferences,
            games: Arc::new(RobloxGamesClient),
            browse_session: browse_session_id(),
            selected_game,
            selected_game_icon: None,
            multi_instance,
            multi_instance_notice,
            catalogue: Vec::new(),
            icon_cache: std::collections::HashMap::new(),
            picker: None,
            settings_section: None,
            confirming_reset: false,
            picker_generation: 0,
            search_epoch: 0,
            focus_search_on_next_frame: false,
            pending_removal: None,
            pending_link: None,
            link_generation: 0,
            cancel_focus: cx.focus_handle().tab_stop(true),
            confirm_focus: cx.focus_handle().tab_stop(true),
            link_cancel_focus: cx.focus_handle().tab_stop(true),
            link_secondary_focus: cx.focus_handle().tab_stop(true),
            focus_modal_on_next_frame: false,
            focus_link_on_next_frame: false,
        }
    }

    pub(super) fn persist_preferences(&mut self, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        let snapshot = self.preferences.clone();
        cx.background_spawn(async move {
            if let Err(error) = settings.save(&snapshot) {
                tracing::warn!(reason = %error, "preferences could not be saved");
            }
        })
        .detach();
    }

    pub(super) fn remember_launch(
        &mut self,
        game: &crate::games::GameSummary,
        cx: &mut Context<Self>,
    ) {
        let icon_url = self
            .preferences
            .favorites
            .iter()
            .chain(self.preferences.recents.iter())
            .find(|saved| saved.universe_id == game.universe_id)
            .and_then(|saved| saved.icon_url.clone());
        self.preferences.record_launch(&game.to_saved(icon_url));
        self.persist_preferences(cx);
    }

    fn remember_saved_launch(&mut self, game: &SavedGame, cx: &mut Context<Self>) {
        self.preferences.record_launch(game);
        self.persist_preferences(cx);
    }

    pub(super) fn next_picker_generation(&mut self) -> u64 {
        self.picker_generation = self
            .picker_generation
            .checked_add(1)
            .expect("picker generation exhausted");
        self.picker_generation
    }

    pub(super) fn next_search_epoch(&mut self) -> u64 {
        self.search_epoch = self
            .search_epoch
            .checked_add(1)
            .expect("search epoch exhausted");
        self.search_epoch
    }
}

impl Render for Dashboard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_modal_on_next_frame {
            let cancel_focus = self.cancel_focus.clone();
            window.on_next_frame(move |window, _| window.focus(&cancel_focus));
            self.focus_modal_on_next_frame = false;
        }
        if self.focus_link_on_next_frame {
            let cancel_focus = self.link_cancel_focus.clone();
            window.on_next_frame(move |window, _| window.focus(&cancel_focus));
            self.focus_link_on_next_frame = false;
        }

        if self.focus_search_on_next_frame
            && let Some(picker) = self.picker.as_ref()
        {
            let search_focus = picker.search_focus.clone();
            window.on_next_frame(move |window, _| window.focus(&search_focus));
            self.focus_search_on_next_frame = false;
        }

        let launch_bar = self.render_launch_bar(cx);
        let content = div()
            .relative()
            .flex_1()
            .w_full()
            .px(px(32.0))
            .pt(px(28.0))
            .pb(px(42.0))
            .flex()
            .flex_col()
            .child(launch_bar)
            .child(self.accounts.clone())
            .when(self.picker.is_some(), |content| {
                content.child(self.render_game_picker(window, cx))
            })
            .when(self.settings_section.is_some(), |content| {
                content.child(self.render_settings_dialog(cx))
            })
            .when_some(self.pending_removal.clone(), |content, request| {
                content.child(self.render_remove_dialog(request, cx))
            })
            .when_some(
                self.pending_link
                    .as_ref()
                    .map(|pending| (pending.state.clone(), pending.request.clone())),
                |content, (state, request)| {
                    content.child(self.render_link_dialog(state, &request, cx))
                },
            );

        div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(rgb(theme().canvas))
            .text_color(rgb(theme().text_primary))
            .font_family("Segoe UI")
            .child(self.render_titlebar(window, cx))
            .child(content)
    }
}
