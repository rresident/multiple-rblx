mod client;

use anyhow::Result;

pub(crate) use client::{RobloxGamesClient, browse_session_id};

pub(crate) const ICON_FORMAT: gpui::ImageFormat = gpui::ImageFormat::Webp;

use crate::settings::SavedGame;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GameSummary {
    pub(crate) universe_id: u64,
    pub(crate) root_place_id: u64,
    pub(crate) name: String,
    pub(crate) player_count: u64,
    pub(crate) up_votes: u64,
    pub(crate) down_votes: u64,
}

impl GameSummary {
    pub(crate) fn approval_percent(&self) -> Option<u32> {
        let up = u128::from(self.up_votes);
        let total = up + u128::from(self.down_votes);
        if total == 0 {
            return None;
        }
        let scaled = (up * 200) / total;
        u32::try_from(scaled.div_ceil(2)).ok()
    }

    pub(crate) fn to_saved(&self, icon_url: Option<String>) -> SavedGame {
        SavedGame {
            universe_id: self.universe_id,
            root_place_id: self.root_place_id,
            name: self.name.clone(),
            icon_url,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GameIcon {
    pub(crate) universe_id: u64,
    pub(crate) image_url: String,
}

pub(crate) trait GamesGateway: Send + Sync {
    fn top_playing(&self, session_id: &str) -> Result<Vec<GameSummary>>;
    fn search(&self, query: &str, session_id: &str) -> Result<Vec<GameSummary>>;
    fn icons(&self, universe_ids: &[u64]) -> Result<Vec<GameIcon>>;
    fn resolve_place(&self, place_id: u64) -> Result<Option<GameSummary>>;
    fn download_icon(&self, url: &str) -> Option<Vec<u8>>;
}

pub(crate) fn parse_place_reference(input: &str) -> Option<u64> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.bytes().all(|byte| byte.is_ascii_digit()) {
        if trimmed.len() < 5 || trimmed.len() > 19 {
            return None;
        }
        return trimmed.parse().ok().filter(|id| *id != 0);
    }

    let lowered = trimmed.to_ascii_lowercase();
    if !lowered.contains("roblox.com") {
        return None;
    }
    let after = lowered.split("/games/").nth(1)?;
    let digits = after
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    digits.parse().ok().filter(|id| *id != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn place_references_are_parsed_from_ids_and_urls() {
        assert_eq!(parse_place_reference("4924922222"), Some(4924922222));
        assert_eq!(
            parse_place_reference("https://www.roblox.com/games/4924922222/Brookhaven-RP"),
            Some(4924922222)
        );
        assert_eq!(
            parse_place_reference("roblox.com/games/142823291/Murder-Mystery-2?x=1"),
            Some(142823291)
        );
        assert_eq!(
            parse_place_reference("  https://ro.blox.com/games/1/x  "),
            None,
            "only roblox.com URLs resolve"
        );
    }

    #[test]
    fn ordinary_search_text_is_not_mistaken_for_an_id() {
        assert_eq!(parse_place_reference("obby"), None);
        assert_eq!(parse_place_reference("2"), None);
        assert_eq!(parse_place_reference("1234"), None);
        assert_eq!(parse_place_reference(""), None);
        assert_eq!(parse_place_reference("tower defense 2"), None);
        assert_eq!(parse_place_reference("00000"), None, "zero is not a place");
    }

    fn summary(up: u64, down: u64) -> GameSummary {
        GameSummary {
            universe_id: 1,
            root_place_id: 2,
            name: "Test".into(),
            player_count: 0,
            up_votes: up,
            down_votes: down,
        }
    }

    #[test]
    fn approval_is_absent_without_votes_and_rounded_otherwise() {
        assert_eq!(summary(0, 0).approval_percent(), None);
        assert_eq!(summary(1, 0).approval_percent(), Some(100));
        assert_eq!(summary(0, 1).approval_percent(), Some(0));
        assert_eq!(summary(1, 1).approval_percent(), Some(50));
        assert_eq!(summary(2, 1).approval_percent(), Some(67));
    }

    #[test]
    fn approval_cannot_overflow_on_absurd_vote_counts() {
        assert_eq!(summary(u64::MAX, u64::MAX).approval_percent(), Some(50));
        assert_eq!(summary(u64::MAX, 0).approval_percent(), Some(100));
        assert_eq!(summary(0, u64::MAX).approval_percent(), Some(0));
    }
}
