use std::{sync::Arc, time::Duration};

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, CursorStyle, Entity, Focusable as _,
    FontWeight, Image, KeyDownEvent, ObjectFit, ScrollHandle, SharedString, StyledImage,
    Subscription, Transformation, deferred, div, img, percentage, prelude::*, px, rgb, rgba, svg,
};

use crate::{games::GameSummary, settings::SavedGame, theme::theme};

use super::{
    Dashboard, LaunchIntent,
    components::{primary_button, secondary_button},
    text_input::{TextInput, TextInputEvent},
};

const DIALOG_WIDTH: f32 = 800.0;
const DIALOG_HEIGHT: f32 = 496.0;
const CARD_WIDTH: f32 = 112.0;
const CARD_GAP: f32 = 16.0;
const CARDS_PER_ROW: usize = 6;
const THUMBNAIL: f32 = 112.0;

const ICON_CHUNK: usize = 6;
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(320);
const MAX_QUERY_LEN: usize = 96;
const MAX_LOOKUP_LEN: usize = 220;

pub(super) struct GamePicker {
    pub(super) generation: u64,
    pub(super) intent: LaunchIntent,
    pub(super) query: String,
    pub(super) results: Option<Vec<GameSummary>>,
    pub(super) selected: Option<GameSummary>,
    pub(super) hovered: Option<u64>,
    pub(super) hovered_star: Option<u64>,
    pub(super) favorites_snapshot: Vec<SavedGame>,
    pub(super) recents_snapshot: Vec<SavedGame>,
    pub(super) loading: bool,
    pub(super) search: Entity<TextInput>,
    pub(super) scroll: ScrollHandle,
    pub(super) lookup: Option<Lookup>,
    _search_subscription: Subscription,
}

pub(super) struct Lookup {
    input: Entity<TextInput>,
    status: LookupStatus,
    _subscription: Subscription,
}

enum LookupStatus {
    Idle,
    Resolving,
    Found(GameSummary),
    Failed(SharedString),
}

impl GamePicker {
    fn sections<'a>(
        &'a self,
        favorites: &'a [SavedGame],
        recents: &'a [SavedGame],
        catalogue: &'a [GameSummary],
    ) -> Vec<Section<'a>> {
        if let Some(results) = &self.results {
            return vec![Section {
                title: "Results",
                games: SectionGames::Live(results),
            }];
        }

        let mut sections = Vec::with_capacity(3);
        if !favorites.is_empty() {
            sections.push(Section {
                title: "Favorited",
                games: SectionGames::Saved(favorites),
            });
        }
        if !recents.is_empty() {
            sections.push(Section {
                title: "Recently launched",
                games: SectionGames::Saved(recents),
            });
        }
        sections.push(Section {
            title: "Games",
            games: SectionGames::Live(catalogue),
        });
        sections
    }
}

struct Section<'a> {
    title: &'static str,
    games: SectionGames<'a>,
}

enum SectionGames<'a> {
    Live(&'a [GameSummary]),
    Saved(&'a [SavedGame]),
}

#[derive(Clone)]
struct Card {
    universe_id: u64,
    root_place_id: u64,
    name: SharedString,
    player_count: Option<u64>,
    approval: Option<u32>,
}

impl Card {
    fn from_live(game: &GameSummary) -> Self {
        Self {
            universe_id: game.universe_id,
            root_place_id: game.root_place_id,
            name: game.name.clone().into(),
            player_count: Some(game.player_count),
            approval: game.approval_percent(),
        }
    }

    fn from_saved(game: &SavedGame) -> Self {
        Self {
            universe_id: game.universe_id,
            root_place_id: game.root_place_id,
            name: game.name.clone().into(),
            player_count: None,
            approval: None,
        }
    }

    fn to_summary(&self) -> GameSummary {
        GameSummary {
            universe_id: self.universe_id,
            root_place_id: self.root_place_id,
            name: self.name.to_string(),
            player_count: self.player_count.unwrap_or_default(),
            up_votes: 0,
            down_votes: 0,
        }
    }
}

impl Dashboard {
    pub(super) fn open_game_picker(&mut self, intent: LaunchIntent, cx: &mut Context<Self>) {
        let generation = self.next_picker_generation();
        let cached = !self.catalogue.is_empty();
        let search = cx.new(|cx| TextInput::new("Search games", MAX_QUERY_LEN, cx));
        let subscription = cx.subscribe(&search, |this, input, _: &TextInputEvent, cx| {
            let text = input.read(cx).text().to_owned();
            let Some(picker) = this.picker.as_mut() else {
                return;
            };
            if picker.query == text {
                return;
            }
            picker.query = text;
            this.schedule_search(cx);
            cx.notify();
        });
        let picker = GamePicker {
            generation,
            intent,
            query: String::new(),
            results: None,
            selected: None,
            hovered: None,
            hovered_star: None,
            favorites_snapshot: self.preferences.favorites.clone(),
            recents_snapshot: self.preferences.recents.clone(),
            loading: !cached,
            search,
            scroll: ScrollHandle::new(),
            lookup: None,
            _search_subscription: subscription,
        };
        self.picker = Some(picker);
        self.focus_search_on_next_frame = true;
        cx.notify();

        if cached {
            let ids = self.visible_universe_ids();
            self.load_icons(ids, cx);
            return;
        }

        let games = self.games.clone();
        let session = self.browse_session.clone();
        let task = cx.background_spawn(async move { games.top_playing(&session) });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(picker) = this.picker.as_mut() {
                    if picker.generation != generation {
                        return;
                    }
                    picker.loading = false;
                }
                match result {
                    Ok(games) => this.catalogue = games,
                    Err(error) => {
                        tracing::warn!(reason = %error, "Roblox discovery could not be loaded");
                    }
                }
                cx.notify();
                let ids = this.visible_universe_ids();
                this.load_icons(ids, cx);
            });
        })
        .detach();

        let ids = self.visible_universe_ids();
        self.load_icons(ids, cx);
    }

    pub(super) fn preload_catalogue(&mut self, cx: &mut Context<Self>) {
        if !self.catalogue.is_empty() {
            return;
        }

        let games = self.games.clone();
        let session = self.browse_session.clone();
        let task = cx.background_spawn(async move { games.top_playing(&session) });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(games) => {
                        tracing::debug!(count = games.len(), "discovery preloaded");
                        this.catalogue = games;
                    }
                    Err(error) => {
                        tracing::warn!(reason = %error, "discovery could not be preloaded");
                    }
                }
                if let Some(picker) = this.picker.as_mut() {
                    picker.loading = false;
                }
                cx.notify();
                let ids = this.visible_universe_ids();
                this.load_icons(ids, cx);
            });
        })
        .detach();
    }

    fn visible_universe_ids(&self) -> Vec<u64> {
        self.preferences
            .favorites
            .iter()
            .chain(self.preferences.recents.iter())
            .chain(self.preferences.selected_game.iter())
            .map(|game| game.universe_id)
            .chain(self.catalogue.iter().map(|game| game.universe_id))
            .collect()
    }

    fn refresh_selected_game_icon(&mut self) {
        if self.selected_game_icon.is_some() {
            return;
        }
        if let Some(selected) = self.selected_game.as_ref() {
            self.selected_game_icon = self.icon_cache.get(&selected.universe_id).cloned();
        }
    }

    fn load_icons(&mut self, universe_ids: Vec<u64>, cx: &mut Context<Self>) {
        let mut seen = std::collections::HashSet::new();
        let wanted = universe_ids
            .into_iter()
            .filter(|universe_id| {
                !self.icon_cache.contains_key(universe_id) && seen.insert(*universe_id)
            })
            .collect::<Vec<_>>();
        if wanted.is_empty() {
            return;
        }

        let games = self.games.clone();
        let url_task = cx.background_spawn(async move { games.icons(&wanted).unwrap_or_default() });

        cx.spawn(async move |this, cx| {
            let icons = url_task.await;
            if icons.is_empty() {
                return;
            }

            let Ok(games) = this.update(cx, |this, _| this.games.clone()) else {
                return;
            };

            let downloads = icons
                .chunks(ICON_CHUNK)
                .map(|chunk| {
                    let chunk = chunk.to_vec();
                    let games = games.clone();
                    cx.background_spawn(async move {
                        chunk
                            .into_iter()
                            .filter_map(|icon| {
                                let bytes = games.download_icon(&icon.image_url)?;
                                Some((icon.universe_id, bytes))
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<Vec<_>>();

            for download in downloads {
                let decoded = download.await;
                let alive = this
                    .update(cx, |this, cx| {
                        for (universe_id, bytes) in decoded {
                            this.icon_cache.entry(universe_id).or_insert_with(|| {
                                Arc::new(Image::from_bytes(crate::games::ICON_FORMAT, bytes))
                            });
                        }
                        this.refresh_selected_game_icon();
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    return;
                }
            }
        })
        .detach();
    }

    pub(super) fn close_game_picker(&mut self, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            self.next_picker_generation();
            cx.notify();
        }
    }

    fn picker_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) -> bool {
        let awaiting_lookup = self
            .picker
            .as_ref()
            .and_then(|picker| picker.lookup.as_ref())
            .is_some_and(|lookup| !matches!(lookup.status, LookupStatus::Found(_)));

        match event.keystroke.key.as_str() {
            "escape" => self.close_game_picker(cx),
            "enter" if awaiting_lookup => self.resolve_lookup(cx),
            "enter" => self.launch_selected(cx),
            _ => return false,
        }
        cx.notify();
        true
    }

    fn open_lookup(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| TextInput::new("Paste a game link or ID", MAX_LOOKUP_LEN, cx));
        let subscription = cx.subscribe(&input, |this, _, _: &TextInputEvent, cx| {
            let Some(lookup) = this
                .picker
                .as_mut()
                .and_then(|picker| picker.lookup.as_mut())
            else {
                return;
            };
            if !matches!(lookup.status, LookupStatus::Idle) {
                lookup.status = LookupStatus::Idle;
                cx.notify();
            }
        });

        let handle = input.read(cx).focus_handle(cx);
        if let Some(picker) = self.picker.as_mut() {
            picker.selected = None;
            picker.lookup = Some(Lookup {
                input,
                status: LookupStatus::Idle,
                _subscription: subscription,
            });
        }
        window.focus(&handle);
        cx.notify();
    }

    fn close_lookup(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        picker.lookup = None;
        picker.selected = None;
        let handle = picker.search.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    fn resolve_lookup(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let Some(lookup) = picker.lookup.as_ref() else {
            return;
        };
        if matches!(lookup.status, LookupStatus::Resolving) {
            return;
        }

        let generation = picker.generation;
        let raw = lookup.input.read(cx).text().trim().to_owned();
        if raw.is_empty() {
            return;
        }

        let Some(place_id) = crate::games::parse_place_reference(&raw) else {
            self.set_lookup_status(
                LookupStatus::Failed("That is not a Roblox game link or ID".into()),
                cx,
            );
            return;
        };

        let epoch = self.next_search_epoch();
        self.set_lookup_status(LookupStatus::Resolving, cx);

        let games = self.games.clone();
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_spawn(async move { games.resolve_place(place_id) })
                .await;

            let _ = this.update(cx, |this, cx| {
                if this.search_epoch != epoch {
                    return;
                }
                let Some(picker) = this.picker.as_mut() else {
                    return;
                };
                if picker.generation != generation || picker.lookup.is_none() {
                    return;
                }

                match found {
                    Ok(Some(game)) => {
                        let universe_id = game.universe_id;
                        picker.selected = Some(game.clone());
                        if let Some(lookup) = picker.lookup.as_mut() {
                            lookup.status = LookupStatus::Found(game);
                        }
                        cx.notify();
                        this.load_icons(vec![universe_id], cx);
                    }
                    Ok(None) => this.set_lookup_status(
                        LookupStatus::Failed("No game exists for that link or ID".into()),
                        cx,
                    ),
                    Err(error) => {
                        tracing::warn!(reason = %error, "place lookup failed");
                        this.set_lookup_status(
                            LookupStatus::Failed("Roblox could not be reached".into()),
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
    }

    fn set_lookup_status(&mut self, status: LookupStatus, cx: &mut Context<Self>) {
        if let Some(lookup) = self
            .picker
            .as_mut()
            .and_then(|picker| picker.lookup.as_mut())
        {
            lookup.status = status;
        }
        cx.notify();
    }

    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let generation = picker.generation;
        let query = picker.query.trim().to_owned();
        let search_epoch = self.next_search_epoch();

        if query.is_empty() {
            if let Some(picker) = self.picker.as_mut() {
                picker.results = None;
            }
            return;
        }

        let games = self.games.clone();
        let session = self.browse_session.clone();

        cx.spawn(async move |this, cx| {
            gpui::Timer::after(SEARCH_DEBOUNCE).await;
            let superseded = this
                .update(cx, |this, _| this.search_epoch != search_epoch)
                .unwrap_or(true);
            if superseded {
                return;
            }

            let found = match crate::games::parse_place_reference(&query) {
                Some(place_id) => {
                    cx.background_spawn(async move {
                        games
                            .resolve_place(place_id)
                            .map(|game| game.into_iter().collect::<Vec<_>>())
                    })
                    .await
                }
                None => {
                    cx.background_spawn(async move { games.search(&query, &session) })
                        .await
                }
            };

            let _ = this.update(cx, |this, cx| {
                if this.search_epoch != search_epoch {
                    return;
                }
                let Some(picker) = this.picker.as_mut() else {
                    return;
                };
                if picker.generation != generation {
                    return;
                }
                match found {
                    Ok(games) => {
                        let ids = games
                            .iter()
                            .map(|game| game.universe_id)
                            .collect::<Vec<_>>();
                        picker.results = Some(games);
                        cx.notify();
                        this.load_icons(ids, cx);
                    }
                    Err(error) => {
                        tracing::warn!(reason = %error, "Roblox game search failed");
                        picker.results = Some(Vec::new());
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    fn select_card(&mut self, card: &Card, cx: &mut Context<Self>) {
        if let Some(picker) = self.picker.as_mut() {
            let summary = card.to_summary();
            picker.selected = if picker
                .selected
                .as_ref()
                .is_some_and(|selected| selected.universe_id == summary.universe_id)
            {
                None
            } else {
                Some(summary)
            };
            cx.notify();
        }
    }

    fn toggle_favorite(&mut self, card: &Card, cx: &mut Context<Self>) {
        let icon_url = self
            .preferences
            .favorites
            .iter()
            .chain(self.preferences.recents.iter())
            .find(|saved| saved.universe_id == card.universe_id)
            .and_then(|saved| saved.icon_url.clone());

        let saved = SavedGame {
            universe_id: card.universe_id,
            root_place_id: card.root_place_id,
            name: card.name.to_string(),
            icon_url,
        };

        if self.preferences.toggle_favorite(&saved) {
            self.persist_preferences(cx);
            cx.notify();
        }
    }

    fn launch_selected(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let Some(selected) = picker.selected.clone() else {
            return;
        };
        let intent = picker.intent;

        self.remember_launch(&selected, cx);
        self.close_game_picker(cx);

        if let LaunchIntent::Account(account_id) = intent {
            let place_id = selected.root_place_id;
            self.accounts.update(cx, |accounts, cx| {
                accounts.begin_launch(account_id, place_id, cx);
            });
        }
    }

    fn pin_selected(&mut self, cx: &mut Context<Self>) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        let Some(selected) = picker.selected.clone() else {
            return;
        };

        let icon_url = self
            .preferences
            .favorites
            .iter()
            .chain(self.preferences.recents.iter())
            .find(|saved| saved.universe_id == selected.universe_id)
            .and_then(|saved| saved.icon_url.clone());

        let saved = selected.to_saved(icon_url);
        self.selected_game_icon = self.icon_cache.get(&selected.universe_id).cloned();
        self.selected_game = Some(saved.clone());
        self.preferences.selected_game = Some(saved);
        self.persist_preferences(cx);
        self.close_game_picker(cx);
        cx.notify();
    }

    pub(super) fn render_game_picker(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(picker) = self.picker.as_ref() else {
            return div().into_any_element();
        };

        let favorites = picker.favorites_snapshot.clone();
        let recents = picker.recents_snapshot.clone();
        let catalogue = self.catalogue.clone();
        let sections = picker.sections(&favorites, &recents, &catalogue);
        let selected_id = picker.selected.as_ref().map(|game| game.universe_id);
        let has_selection = picker.selected.is_some();
        let is_empty = sections
            .iter()
            .all(|section| section_len(&section.games) == 0);
        let loading = picker.loading;
        let query = picker.query.clone();
        let searching = !query.trim().is_empty();

        let body = sections
            .iter()
            .map(|section| self.render_section(section, selected_id, cx))
            .collect::<Vec<_>>();

        deferred(
            div()
                .id("game-picker-modal")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgba(theme().scrim))
                .occlude()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if this.picker_key(event, cx) {
                        cx.stop_propagation();
                    }
                }))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(DIALOG_WIDTH))
                        .h(px(DIALOG_HEIGHT))
                        .rounded(px(16.0))
                        .border_1()
                        .border_color(rgb(theme().strong_border))
                        .bg(rgb(theme().surface))
                        .overflow_hidden()
                        .flex()
                        .flex_col()
                        .child(self.render_picker_header(&query, window, cx))
                        .child(if picker.lookup.is_some() {
                            self.render_lookup(cx)
                        } else {
                            div()
                                .id("game-picker-body")
                                .flex_1()
                                .w_full()
                                .overflow_y_scroll()
                                .track_scroll(&picker.scroll)
                                .px(px(24.0))
                                .py(px(18.0))
                                .flex()
                                .flex_col()
                                .when(loading && is_empty, |body| {
                                    body.child(picker_placeholder("Loading games"))
                                })
                                .when(!loading && is_empty && searching, |body| {
                                    body.child(picker_placeholder("No games matched that search"))
                                })
                                .when(!loading && is_empty && !searching, |body| {
                                    body.child(picker_placeholder("Games could not be loaded"))
                                })
                                .children(body)
                                .into_any_element()
                        })
                        .child(self.render_picker_footer(has_selection, cx)),
                ),
        )
        .with_priority(100)
        .into_any_element()
    }

    fn render_picker_header(
        &self,
        query: &str,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(picker) = self.picker.as_ref() else {
            return div().into_any_element();
        };
        let is_focused = picker.search.read(cx).focus_handle(cx).is_focused(window);

        div()
            .h(px(72.0))
            .w_full()
            .flex_none()
            .px(px(24.0))
            .border_b_1()
            .border_color(rgb(theme().divider))
            .bg(rgb(theme().inset_surface))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Choose a game"),
                    )
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(px(11.5))
                            .text_color(rgb(theme().text_tertiary))
                            .child(if picker.lookup.is_some() {
                                "Paste the address of a game on roblox.com, or its ID."
                            } else {
                                "Pick a game to launch, or star one to keep it handy."
                            }),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .when(picker.lookup.is_none(), |actions| {
                        actions.child(
                            div()
                                .id("game-search")
                                .w(px(268.0))
                                .h(px(34.0))
                                .flex_none()
                                .rounded(px(9.0))
                                .border_1()
                                .border_color(rgb(theme().border))
                                .bg(rgb(theme().surface))
                                .when(is_focused, |field| {
                                    field.border_color(rgb(theme().search_focus_border))
                                })
                                .cursor(CursorStyle::IBeam)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    if let Some(picker) = this.picker.as_ref() {
                                        window.focus(&picker.search.read(cx).focus_handle(cx));
                                        cx.notify();
                                    }
                                }))
                                .flex()
                                .items_center()
                                .px(px(11.0))
                                .gap(px(8.0))
                                .child(
                                    svg()
                                        .path("search.svg")
                                        .size(px(14.0))
                                        .flex_none()
                                        .text_color(rgb(theme().text_tertiary)),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .overflow_hidden()
                                        .flex()
                                        .items_center()
                                        .text_size(px(12.0))
                                        .child(picker.search.clone()),
                                )
                                .when(!query.is_empty(), |bar| {
                                    bar.child(
                                        div()
                                            .id("clear-search")
                                            .flex_none()
                                            .size(px(16.0))
                                            .rounded_full()
                                            .flex()
                                            .items_center()
                                            .justify_center()
                                            .text_color(rgb(theme().text_tertiary))
                                            .cursor(CursorStyle::PointingHand)
                                            .hover(|style| {
                                                style.text_color(rgb(theme().text_primary))
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                if let Some(picker) = this.picker.as_ref() {
                                                    let search = picker.search.clone();
                                                    let handle = search.read(cx).focus_handle(cx);
                                                    window.focus(&handle);
                                                    search.update(cx, |input, cx| input.clear(cx));
                                                }
                                            }))
                                            .child(
                                                svg()
                                                    .path("close.svg")
                                                    .size(px(11.0))
                                                    .text_color(rgb(theme().text_tertiary)),
                                            ),
                                    )
                                }),
                        )
                    })
                    .child(if picker.lookup.is_some() {
                        secondary_button(
                            "picker-lookup-back",
                            "Back to browsing",
                            146.0,
                            cx.listener(|this, _, window, cx| this.close_lookup(window, cx)),
                        )
                    } else {
                        secondary_button(
                            "picker-lookup-open",
                            "Look up by ID",
                            128.0,
                            cx.listener(|this, _, window, cx| this.open_lookup(window, cx)),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_lookup(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(lookup) = self
            .picker
            .as_ref()
            .and_then(|picker| picker.lookup.as_ref())
        else {
            return div().into_any_element();
        };

        let resolving = matches!(lookup.status, LookupStatus::Resolving);

        div()
            .flex_1()
            .w_full()
            .px(px(24.0))
            .py(px(20.0))
            .flex()
            .flex_col()
            .gap(px(14.0))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(theme().text_tertiary))
                    .child("Open the game on roblox.com and copy the address"),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(
                        div()
                            .id("lookup-field")
                            .flex_1()
                            .min_w(px(0.0))
                            .h(px(36.0))
                            .rounded(px(9.0))
                            .border_1()
                            .border_color(rgb(theme().border))
                            .bg(rgb(theme().surface))
                            .cursor(CursorStyle::IBeam)
                            .flex()
                            .items_center()
                            .px(px(12.0))
                            .text_size(px(12.5))
                            .child(lookup.input.clone()),
                    )
                    .child(primary_button(
                        "lookup-resolve",
                        if resolving { "Looking up" } else { "Look up" },
                        104.0,
                        cx.listener(|this, _, _, cx| this.resolve_lookup(cx)),
                    )),
            )
            .child(self.render_lookup_status(cx))
            .into_any_element()
    }

    fn render_lookup_status(&self, _cx: &mut Context<Self>) -> AnyElement {
        let Some(lookup) = self
            .picker
            .as_ref()
            .and_then(|picker| picker.lookup.as_ref())
        else {
            return div().into_any_element();
        };

        match &lookup.status {
            LookupStatus::Idle => div().into_any_element(),
            LookupStatus::Resolving => div()
                .mt(px(6.0))
                .text_size(px(12.0))
                .text_color(rgb(theme().text_tertiary))
                .child("Asking Roblox about that game")
                .into_any_element(),
            LookupStatus::Failed(reason) => div()
                .mt(px(6.0))
                .text_size(px(12.0))
                .text_color(rgb(theme().danger_text))
                .child(reason.clone())
                .into_any_element(),
            LookupStatus::Found(game) => {
                let image = self.icon_cache.get(&game.universe_id).cloned();
                let players = game.player_count;

                div()
                    .mt(px(4.0))
                    .w_full()
                    .p(px(14.0))
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(rgb(theme().card_selected_border))
                    .bg(rgb(theme().inset_surface))
                    .flex()
                    .items_center()
                    .gap(px(14.0))
                    .child(
                        div()
                            .size(px(72.0))
                            .flex_none()
                            .rounded(px(9.0))
                            .overflow_hidden()
                            .bg(rgb(theme().surface))
                            .when_some(image, |slot, image| {
                                slot.child(
                                    img(image)
                                        .size_full()
                                        .object_fit(ObjectFit::Cover)
                                        .rounded(px(9.0)),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(14.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .line_clamp(2)
                                    .child(SharedString::from(game.name.clone())),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(theme().text_tertiary))
                                    .child(SharedString::from(format!("{players} playing now"))),
                            ),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_picker_footer(&self, has_selection: bool, cx: &mut Context<Self>) -> AnyElement {
        let selected_name = self
            .picker
            .as_ref()
            .and_then(|picker| picker.selected.as_ref())
            .map(|game| SharedString::from(game.name.clone()));

        div()
            .h(px(64.0))
            .w_full()
            .flex_none()
            .px(px(24.0))
            .border_t_1()
            .border_color(rgb(theme().divider))
            .bg(rgb(theme().inset_surface))
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.0))
                    .text_color(rgb(if has_selection {
                        theme().text_secondary
                    } else {
                        theme().text_tertiary
                    }))
                    .child(match selected_name {
                        Some(name) => SharedString::from(format!("Selected: {name}")),
                        None => SharedString::from("Select a game to continue"),
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(10.0))
                    .child(secondary_button(
                        "picker-cancel",
                        "Cancel",
                        88.0,
                        cx.listener(|this, _, _, cx| this.close_game_picker(cx)),
                    ))
                    .when(has_selection, |actions| {
                        actions
                            .child(secondary_button(
                                "picker-pin",
                                "Use for all launches",
                                156.0,
                                cx.listener(|this, _, _, cx| this.pin_selected(cx)),
                            ))
                            .child(primary_button(
                                "picker-launch",
                                "Launch",
                                96.0,
                                cx.listener(|this, _, _, cx| this.launch_selected(cx)),
                            ))
                    }),
            )
            .into_any_element()
    }

    fn render_section(
        &self,
        section: &Section<'_>,
        selected_id: Option<u64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cards: Vec<Card> = match &section.games {
            SectionGames::Live(games) => games.iter().map(Card::from_live).collect(),
            SectionGames::Saved(games) => games.iter().map(Card::from_saved).collect(),
        };
        if cards.is_empty() {
            return div().into_any_element();
        }

        let rows = cards
            .chunks(CARDS_PER_ROW)
            .map(|row| {
                div()
                    .flex()
                    .gap(px(CARD_GAP))
                    .mb(px(CARD_GAP))
                    .children(
                        row.iter()
                            .map(|card| self.render_card(card, selected_id, cx))
                            .collect::<Vec<_>>(),
                    )
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .w_full()
            .flex()
            .flex_col()
            .mb(px(6.0))
            .child(
                div()
                    .mb(px(12.0))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme().text_secondary))
                    .child(section.title),
            )
            .children(rows)
            .into_any_element()
    }

    fn render_card(
        &self,
        card: &Card,
        selected_id: Option<u64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let universe_id = card.universe_id;
        let is_selected = selected_id == Some(universe_id);
        let is_favorite = self.preferences.is_favorite(universe_id);
        let is_hovered =
            self.picker.as_ref().and_then(|picker| picker.hovered) == Some(universe_id);
        let image = self.icon_cache.get(&universe_id).cloned();

        let for_select = card.clone();
        let for_star = card.clone();

        div()
            .id(("game-card", universe_id))
            .w(px(CARD_WIDTH))
            .flex_none()
            .flex()
            .flex_col()
            .cursor(CursorStyle::PointingHand)
            .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                if let Some(picker) = this.picker.as_mut()
                    && (*hovered || picker.hovered == Some(universe_id))
                {
                    picker.hovered = hovered.then_some(universe_id);
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.select_card(&for_select, cx)))
            .child(
                div()
                    .relative()
                    .size(px(THUMBNAIL))
                    .rounded(px(11.0))
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(if is_selected {
                        theme().card_selected_border
                    } else {
                        theme().border
                    }))
                    .bg(rgb(theme().card_backdrop))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when_some(image, |slot, image| {
                        slot.child(
                            img(image)
                                .size(px(THUMBNAIL - 2.0))
                                .rounded(px(10.0))
                                .object_fit(ObjectFit::Cover),
                        )
                    })
                    .when(is_hovered || is_favorite, |slot| {
                        let star_hovered =
                            self.picker.as_ref().and_then(|picker| picker.hovered_star)
                                == Some(universe_id);
                        let filled = is_favorite || star_hovered;

                        slot.child(
                            div()
                                .id(("game-star", universe_id))
                                .absolute()
                                .top(px(6.0))
                                .right(px(6.0))
                                .size(px(22.0))
                                .rounded_full()
                                .bg(rgba(if star_hovered {
                                    theme().star_backdrop_hover
                                } else {
                                    theme().star_backdrop
                                }))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor(CursorStyle::PointingHand)
                                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                    if let Some(picker) = this.picker.as_mut()
                                        && (*hovered || picker.hovered_star == Some(universe_id))
                                    {
                                        picker.hovered_star = hovered.then_some(universe_id);
                                        cx.notify();
                                    }
                                }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_favorite(&for_star, cx);
                                    cx.stop_propagation();
                                }))
                                .child(
                                    svg()
                                        .path(if filled {
                                            "star-filled.svg"
                                        } else {
                                            "star.svg"
                                        })
                                        .size(px(13.0))
                                        .text_color(rgb(if filled {
                                            theme().star_active
                                        } else {
                                            theme().star_idle
                                        })),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .mt(px(8.0))
                    .w_full()
                    .h(px(32.0))
                    .line_clamp(2)
                    .text_size(px(11.5))
                    .line_height(px(15.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(theme().text_primary))
                    .child(card.name.clone()),
            )
            .child(
                div()
                    .mt(px(3.0))
                    .w_full()
                    .h(px(14.0))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .text_size(px(10.5))
                    .text_color(rgb(theme().text_tertiary))
                    .when_some(card.player_count, |stats, count| {
                        stats.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(3.0))
                                .child(
                                    svg()
                                        .path("people.svg")
                                        .size(px(11.0))
                                        .flex_none()
                                        .text_color(rgb(theme().text_tertiary)),
                                )
                                .child(compact_count(count)),
                        )
                    })
                    .when_some(card.approval, |stats, approval| {
                        stats.child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(3.0))
                                .child(
                                    svg()
                                        .path("star-filled.svg")
                                        .size(px(10.0))
                                        .flex_none()
                                        .text_color(rgb(theme().text_tertiary)),
                                )
                                .child(SharedString::from(format!("{approval}%"))),
                        )
                    }),
            )
            .into_any_element()
    }
}

fn section_len(games: &SectionGames<'_>) -> usize {
    match games {
        SectionGames::Live(games) => games.len(),
        SectionGames::Saved(games) => games.len(),
    }
}

fn picker_placeholder(message: &'static str) -> AnyElement {
    div()
        .w_full()
        .h(px(240.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.0))
        .child(
            svg()
                .path("spinner.svg")
                .size(px(18.0))
                .text_color(rgb(theme().text_tertiary))
                .with_animation(
                    "picker-placeholder-spinner",
                    Animation::new(Duration::from_millis(760)).repeat(),
                    |svg, delta| svg.with_transformation(Transformation::rotate(percentage(delta))),
                ),
        )
        .child(
            div()
                .text_size(px(12.0))
                .text_color(rgb(theme().text_tertiary))
                .child(message),
        )
        .into_any_element()
}

pub(super) fn compact_count(value: u64) -> SharedString {
    const MILLION: u64 = 1_000_000;
    const THOUSAND: u64 = 1_000;

    if value >= MILLION {
        let whole = value / MILLION;
        let tenth = (value % MILLION) / (MILLION / 10);
        return SharedString::from(format!("{whole}.{tenth}M"));
    }
    if value >= 10 * THOUSAND {
        let whole = value / THOUSAND;
        let tenth = (value % THOUSAND) / (THOUSAND / 10);
        return SharedString::from(format!("{whole}.{tenth}K"));
    }
    SharedString::from(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_abbreviate_the_way_roblox_shows_them() {
        assert_eq!(compact_count(0).to_string(), "0");
        assert_eq!(compact_count(999).to_string(), "999");
        assert_eq!(compact_count(9_999).to_string(), "9999");
        assert_eq!(compact_count(10_000).to_string(), "10.0K");
        assert_eq!(compact_count(855_707).to_string(), "855.7K");
        assert_eq!(compact_count(1_250_000).to_string(), "1.2M");
    }
}
