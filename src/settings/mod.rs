mod data;
mod startup;
mod store;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::theme::ThemeMode;

pub(crate) use data::{
    clear_cached_files, clearable_bytes, delete_everything, format_bytes, stored_account_count,
};
pub(crate) use startup::{
    is_enabled as starts_with_windows, launched_hidden, set_enabled as set_start_with_windows,
};
pub(crate) use store::{SharedSettings, system_settings};

pub(crate) const SCHEMA_VERSION: u32 = 1;

const MAX_FAVORITES: usize = 64;
const MAX_RECENTS: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedGame {
    pub(crate) universe_id: u64,
    pub(crate) root_place_id: u64,
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) icon_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Preferences {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) multiple_instances_enabled: bool,
    #[serde(default)]
    pub(crate) favorites: Vec<SavedGame>,
    #[serde(default)]
    pub(crate) recents: Vec<SavedGame>,
    #[serde(default)]
    pub(crate) start_with_windows: bool,
    #[serde(default)]
    pub(crate) start_hidden: bool,
    #[serde(default)]
    pub(crate) theme: ThemeMode,
    #[serde(default)]
    pub(crate) reduce_motion: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) selected_game: Option<SavedGame>,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            multiple_instances_enabled: false,
            start_with_windows: false,
            start_hidden: false,
            reduce_motion: false,
            theme: ThemeMode::Dark,
            favorites: Vec::new(),
            recents: Vec::new(),
            selected_game: None,
        }
    }
}

impl Preferences {
    pub(crate) fn is_favorite(&self, universe_id: u64) -> bool {
        self.favorites
            .iter()
            .any(|game| game.universe_id == universe_id)
    }

    pub(crate) fn toggle_favorite(&mut self, game: &SavedGame) -> bool {
        if let Some(index) = self
            .favorites
            .iter()
            .position(|saved| saved.universe_id == game.universe_id)
        {
            self.favorites.remove(index);
            return true;
        }

        if self.favorites.len() >= MAX_FAVORITES {
            return false;
        }
        self.favorites.push(game.clone());
        true
    }

    pub(crate) fn record_launch(&mut self, game: &SavedGame) {
        self.recents
            .retain(|saved| saved.universe_id != game.universe_id);
        self.recents.insert(0, game.clone());
        self.recents.truncate(MAX_RECENTS);
    }
}

pub(crate) trait SettingsStore: Send + Sync {
    fn load(&self) -> Result<Preferences>;
    fn save(&self, preferences: &Preferences) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(universe_id: u64) -> SavedGame {
        SavedGame {
            universe_id,
            root_place_id: universe_id * 10,
            name: format!("Game {universe_id}"),
            icon_url: None,
        }
    }

    #[test]
    fn toggling_a_favorite_adds_then_removes_it() {
        let mut preferences = Preferences::default();
        assert!(!preferences.is_favorite(1));

        assert!(preferences.toggle_favorite(&game(1)));
        assert!(preferences.is_favorite(1));

        assert!(preferences.toggle_favorite(&game(1)));
        assert!(!preferences.is_favorite(1));
    }

    #[test]
    fn favorites_are_capped_and_the_cap_rejects_rather_than_evicts() {
        let mut preferences = Preferences::default();
        for id in 0..MAX_FAVORITES as u64 {
            assert!(preferences.toggle_favorite(&game(id)));
        }

        assert!(!preferences.toggle_favorite(&game(9_999)));
        assert_eq!(preferences.favorites.len(), MAX_FAVORITES);
        assert!(preferences.is_favorite(0), "existing entries survive");
    }

    #[test]
    fn recents_move_to_front_without_duplicating() {
        let mut preferences = Preferences::default();
        preferences.record_launch(&game(1));
        preferences.record_launch(&game(2));
        preferences.record_launch(&game(1));

        let ids = preferences
            .recents
            .iter()
            .map(|saved| saved.universe_id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![1, 2]);
    }

    #[test]
    fn recents_are_capped_to_the_most_recent_entries() {
        let mut preferences = Preferences::default();
        for id in 0..(MAX_RECENTS as u64 + 5) {
            preferences.record_launch(&game(id));
        }

        assert_eq!(preferences.recents.len(), MAX_RECENTS);
        assert_eq!(preferences.recents[0].universe_id, MAX_RECENTS as u64 + 4);
    }

    #[test]
    fn a_pinned_game_survives_a_serialisation_round_trip() {
        let mut preferences = Preferences::default();
        assert!(preferences.selected_game.is_none());
        preferences.selected_game = Some(game(7));

        let encoded = serde_json::to_string(&preferences).expect("encode");
        let decoded: Preferences = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded.selected_game, Some(game(7)));

        let legacy: Preferences = serde_json::from_str(r#"{"version":1}"#).expect("legacy");
        assert!(legacy.selected_game.is_none());
    }

    #[test]
    fn unknown_fields_and_missing_fields_both_survive_a_round_trip() {
        let raw = r#"{"version":1,"favorites":[{"universeId":7,"rootPlaceId":70,"name":"X"}],"somethingNew":true}"#;
        let preferences: Preferences =
            serde_json::from_str(raw).expect("forward-compatible parse should succeed");

        assert_eq!(preferences.favorites.len(), 1);
        assert_eq!(preferences.favorites[0].universe_id, 7);
        assert!(preferences.recents.is_empty());
        assert!(!preferences.multiple_instances_enabled);
    }
}
