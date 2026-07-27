use std::{
    process,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result};
use serde::Deserialize;

use super::{GameIcon, GameSummary, GamesGateway};

const SORTS_URL: &str = "https://apis.roblox.com/explore-api/v1/get-sorts";
const SEARCH_URL: &str = "https://apis.roblox.com/search-api/omni-search";
const ICONS_URL: &str = "https://thumbnails.roblox.com/v1/games/icons";
const UNIVERSE_URL: &str = "https://apis.roblox.com/universes/v1/places";
const GAME_DETAILS_URL: &str = "https://games.roblox.com/v1/games";

const TOP_PLAYING_SORT: &str = "top-playing-now";

const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_ICON_BYTES: u64 = 4 * 1024 * 1024;

const ICON_PIXEL_SIZE: &str = "150x150";
const ICON_WIRE_FORMAT: &str = "Webp";

pub(crate) const ICON_BATCH: usize = 100;

const MAX_RESULTS: usize = 60;

pub(crate) struct RobloxGamesClient;

pub(crate) fn browse_session_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    format!("mrblx-{}-{nanos:x}", process::id())
}

impl GamesGateway for RobloxGamesClient {
    fn top_playing(&self, session_id: &str) -> Result<Vec<GameSummary>> {
        let agent = public_agent();
        let mut response = agent
            .get(SORTS_URL)
            .query("sessionId", session_id)
            .query("device", "computer")
            .query("country", "all")
            .call()
            .context("requesting Roblox discovery sorts")?;

        let payload: SortsResponse = response
            .body_mut()
            .read_json()
            .context("reading Roblox discovery sorts")?;

        let sort = payload
            .sorts
            .iter()
            .find(|sort| sort.sort_id.as_deref() == Some(TOP_PLAYING_SORT))
            .or_else(|| payload.sorts.iter().find(|sort| !sort.games.is_empty()));

        Ok(sort
            .map(|sort| {
                sort.games
                    .iter()
                    .filter_map(ExploreGame::to_summary)
                    .collect()
            })
            .unwrap_or_default())
    }

    fn search(&self, query: &str, session_id: &str) -> Result<Vec<GameSummary>> {
        let agent = public_agent();
        let mut response = agent
            .get(SEARCH_URL)
            .query("searchQuery", query)
            .query("pageType", "all")
            .query("sessionId", session_id)
            .call()
            .context("searching Roblox games")?;

        let payload: SearchResponse = response
            .body_mut()
            .read_json()
            .context("reading Roblox search results")?;

        Ok(payload
            .search_results
            .into_iter()
            .filter(|group| group.content_group_type.as_deref() == Some("Game"))
            .flat_map(|group| group.contents)
            .filter_map(SearchGame::into_summary)
            .take(MAX_RESULTS)
            .collect())
    }

    fn icons(&self, universe_ids: &[u64]) -> Result<Vec<GameIcon>> {
        if universe_ids.is_empty() {
            return Ok(Vec::new());
        }

        let agent = public_agent();
        let mut icons = Vec::with_capacity(universe_ids.len());

        for chunk in universe_ids.chunks(ICON_BATCH) {
            let ids = chunk
                .iter()
                .map(u64::to_string)
                .collect::<Vec<_>>()
                .join(",");

            let mut response = agent
                .get(ICONS_URL)
                .query("universeIds", &ids)
                .query("size", ICON_PIXEL_SIZE)
                .query("format", ICON_WIRE_FORMAT)
                .query("isCircular", "false")
                .call()
                .context("requesting Roblox game icons")?;

            let payload: ApiResponse<RobloxIcon> = response
                .body_mut()
                .read_json()
                .context("reading Roblox game icons")?;

            icons.extend(payload.data.into_iter().filter_map(|icon| {
                (icon.state == "Completed")
                    .then(|| {
                        Some(GameIcon {
                            universe_id: icon.target_id,
                            image_url: icon.image_url?,
                        })
                    })
                    .flatten()
            }));
        }

        Ok(icons)
    }

    fn resolve_place(&self, id: u64) -> Result<Option<GameSummary>> {
        if id == 0 {
            return Ok(None);
        }
        let agent = public_agent();

        let mut universe_response = agent
            .get(&format!("{UNIVERSE_URL}/{id}/universe"))
            .call()
            .context("resolving the pasted id as a place")?;
        let universe: UniverseResponse = universe_response
            .body_mut()
            .read_json()
            .context("reading the resolved universe")?;

        match universe.universe_id {
            Some(universe_id) => game_details(&agent, universe_id, Some(id)),
            None => game_details(&agent, id, None),
        }
    }

    fn download_icon(&self, url: &str) -> Option<Vec<u8>> {
        if !url.starts_with("https://") {
            return None;
        }

        public_agent()
            .get(url)
            .header("Accept", "image/webp,image/png")
            .call()
            .ok()?
            .body_mut()
            .with_config()
            .limit(MAX_ICON_BYTES)
            .read_to_vec()
            .ok()
    }
}

fn game_details(
    agent: &ureq::Agent,
    universe_id: u64,
    fallback_place: Option<u64>,
) -> Result<Option<GameSummary>> {
    let mut response = agent
        .get(GAME_DETAILS_URL)
        .query("universeIds", universe_id.to_string())
        .call()
        .context("requesting game details")?;
    let details: ApiResponse<GameDetails> = response
        .body_mut()
        .read_json()
        .context("reading game details")?;

    Ok(details.data.into_iter().next().and_then(|game| {
        let root_place_id = match (game.root_place_id, fallback_place) {
            (0, Some(place_id)) => place_id,
            (0, None) => return None,
            (root, _) => root,
        };
        Some(GameSummary {
            universe_id: game.id,
            root_place_id,
            name: game.name,
            player_count: game.playing,
            up_votes: 0,
            down_votes: 0,
        })
    }))
}

fn public_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .into()
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RobloxIcon {
    target_id: u64,
    state: String,
    image_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UniverseResponse {
    #[serde(default)]
    universe_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameDetails {
    id: u64,
    #[serde(default)]
    root_place_id: u64,
    name: String,
    #[serde(default)]
    playing: u64,
}

#[derive(Debug, Deserialize)]
struct SortsResponse {
    #[serde(default)]
    sorts: Vec<ExploreSort>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExploreSort {
    #[serde(default)]
    sort_id: Option<String>,
    #[serde(default)]
    games: Vec<ExploreGame>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExploreGame {
    universe_id: u64,
    root_place_id: u64,
    name: String,
    #[serde(default)]
    player_count: u64,
    #[serde(default)]
    total_up_votes: u64,
    #[serde(default)]
    total_down_votes: u64,
}

impl ExploreGame {
    fn to_summary(&self) -> Option<GameSummary> {
        if self.universe_id == 0 || self.root_place_id == 0 || self.name.is_empty() {
            return None;
        }
        Some(GameSummary {
            universe_id: self.universe_id,
            root_place_id: self.root_place_id,
            name: self.name.clone(),
            player_count: self.player_count,
            up_votes: self.total_up_votes,
            down_votes: self.total_down_votes,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchResponse {
    #[serde(default)]
    search_results: Vec<SearchGroup>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchGroup {
    #[serde(default)]
    content_group_type: Option<String>,
    #[serde(default)]
    contents: Vec<SearchGame>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SearchGame {
    universe_id: u64,
    #[serde(default)]
    root_place_id: u64,
    name: String,
    #[serde(default)]
    player_count: u64,
    #[serde(default)]
    total_up_votes: u64,
    #[serde(default)]
    total_down_votes: u64,
}

impl SearchGame {
    fn into_summary(self) -> Option<GameSummary> {
        if self.universe_id == 0 || self.root_place_id == 0 || self.name.is_empty() {
            return None;
        }
        Some(GameSummary {
            universe_id: self.universe_id,
            root_place_id: self.root_place_id,
            name: self.name,
            player_count: self.player_count,
            up_votes: self.total_up_votes,
            down_votes: self.total_down_votes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_session_ids_are_non_empty_and_carry_no_user_data() {
        let id = browse_session_id();
        assert!(id.starts_with("mrblx-"));
        assert!(id.len() > 10);
    }

    #[test]
    fn explore_sort_parsing_matches_the_live_wire_shape() {
        let raw = r#"{
            "sorts": [
                { "contentType": "Filters", "games": [] },
                { "contentType": "Games", "sortId": "top-playing-now", "games": [
                    { "universeId": 66654135, "rootPlaceId": 142823291, "name": "Murder Mystery 2",
                      "playerCount": 855707, "totalUpVotes": 10111376, "totalDownVotes": 1017018 }
                ]}
            ]
        }"#;

        let payload: SortsResponse = serde_json::from_str(raw).expect("should parse");
        let sort = payload
            .sorts
            .iter()
            .find(|sort| sort.sort_id.as_deref() == Some(TOP_PLAYING_SORT))
            .expect("top playing sort should be found");
        let games = sort
            .games
            .iter()
            .filter_map(ExploreGame::to_summary)
            .collect::<Vec<_>>();

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].universe_id, 66654135);
        assert_eq!(games[0].root_place_id, 142823291);
        assert_eq!(games[0].player_count, 855707);
        assert_eq!(games[0].approval_percent(), Some(91));
    }

    #[test]
    fn search_results_flatten_single_game_groups_and_drop_non_games() {
        let raw = r#"{
            "searchResults": [
                { "contentGroupType": "Game", "contents": [
                    { "universeId": 6255392043, "rootPlaceId": 18458348062, "name": "Tower Defense RNG",
                      "playerCount": 137, "totalUpVotes": 172450, "totalDownVotes": 7835 }
                ]},
                { "contentGroupType": "Creator", "contents": [] }
            ]
        }"#;

        let payload: SearchResponse = serde_json::from_str(raw).expect("should parse");
        let games = payload
            .search_results
            .into_iter()
            .filter(|group| group.content_group_type.as_deref() == Some("Game"))
            .flat_map(|group| group.contents)
            .filter_map(SearchGame::into_summary)
            .collect::<Vec<_>>();

        assert_eq!(games.len(), 1);
        assert_eq!(games[0].name, "Tower Defense RNG");
    }

    #[test]
    #[ignore = "hits the live Roblox API"]
    fn benchmark_cold_picker_load() {
        use std::time::Instant;

        let client = RobloxGamesClient;
        let session = browse_session_id();

        let started = Instant::now();
        let games = client.top_playing(&session).expect("discovery");
        let discovery = started.elapsed();

        let ids = games
            .iter()
            .map(|game| game.universe_id)
            .collect::<Vec<_>>();

        let started = Instant::now();
        let icons = client.icons(&ids).expect("icons");
        let icon_urls = started.elapsed();

        let started = Instant::now();
        let mut first_row_bytes = 0;
        for icon in icons.iter().take(6) {
            first_row_bytes += client.download_icon(&icon.image_url).map_or(0, |b| b.len());
        }
        let first_row = started.elapsed();

        let started = Instant::now();
        let mut total_bytes = first_row_bytes;
        for icon in icons.iter().skip(6) {
            total_bytes += client.download_icon(&icon.image_url).map_or(0, |b| b.len());
        }
        let remainder = started.elapsed();

        println!(
            "games discovered      : {:>4} in {discovery:?}",
            games.len()
        );
        println!(
            "icon urls resolved    : {:>4} in {icon_urls:?}",
            icons.len()
        );
        println!("first row (6 icons)   : {first_row_bytes:>7} bytes in {first_row:?}");
        println!(
            "remaining {:>3} icons   : {:>7} bytes in {remainder:?}",
            icons.len().saturating_sub(6),
            total_bytes - first_row_bytes
        );
        println!("total payload         : {total_bytes} bytes");
        println!(
            "NOTE: the app downloads in parallel row-sized chunks, so wall clock in the \
             app is well under the serial figure above."
        );

        assert!(!games.is_empty());
        assert!(!icons.is_empty());
    }

    #[test]
    #[ignore = "hits the live Roblox API"]
    fn live_place_and_universe_ids_both_resolve_to_the_same_game() {
        let client = RobloxGamesClient;
        const MURDER_MYSTERY_PLACE: u64 = 142823291;
        const MURDER_MYSTERY_UNIVERSE: u64 = 66654135;

        let by_place = client
            .resolve_place(MURDER_MYSTERY_PLACE)
            .expect("place lookup should succeed")
            .expect("place should resolve");
        let by_universe = client
            .resolve_place(MURDER_MYSTERY_UNIVERSE)
            .expect("universe lookup should succeed")
            .expect("universe should resolve");

        println!("by place:    {} ({})", by_place.name, by_place.universe_id);
        println!(
            "by universe: {} ({})",
            by_universe.name, by_universe.universe_id
        );

        assert_eq!(by_place.universe_id, MURDER_MYSTERY_UNIVERSE);
        assert_eq!(by_place.root_place_id, MURDER_MYSTERY_PLACE);
        assert_eq!(by_universe.universe_id, by_place.universe_id);
        assert_eq!(by_universe.root_place_id, by_place.root_place_id);
        assert_eq!(by_universe.name, by_place.name);
    }

    #[test]
    #[ignore = "hits the live Roblox API"]
    fn live_discovery_search_and_icons_return_usable_data() {
        let client = RobloxGamesClient;

        let top = client
            .top_playing(&browse_session_id())
            .expect("top playing should load");
        assert!(!top.is_empty(), "discovery returned no games");
        assert!(top.iter().all(|game| game.root_place_id != 0));
        println!("top_playing: {} games, first = {}", top.len(), top[0].name);

        let found = client
            .search("obby", &browse_session_id())
            .expect("search should load");
        assert!(
            !found.is_empty(),
            "search returned no games; sessionId is required or the shape changed"
        );
        println!("search: {} games, first = {}", found.len(), found[0].name);

        let ids = top
            .iter()
            .take(8)
            .map(|game| game.universe_id)
            .collect::<Vec<_>>();
        let icons = client.icons(&ids).expect("icons should load");
        assert!(!icons.is_empty(), "no icons resolved");
        println!("icons: {} of {} resolved", icons.len(), ids.len());

        let bytes = client
            .download_icon(&icons[0].image_url)
            .expect("icon bytes should download");
        assert!(bytes.len() > 512, "icon looks truncated");
        assert_eq!(&bytes[0..4], b"RIFF", "icon should be a WebP");
        println!("icon bytes: {}", bytes.len());
    }

    #[test]
    fn incomplete_icons_and_malformed_games_are_discarded() {
        let raw = r#"{"data":[
            {"targetId":1,"state":"Completed","imageUrl":"https://t0.rbxcdn.com/a"},
            {"targetId":2,"state":"Blocked","imageUrl":"https://t0.rbxcdn.com/b"},
            {"targetId":3,"state":"Completed","imageUrl":null}
        ]}"#;
        let payload: ApiResponse<RobloxIcon> = serde_json::from_str(raw).expect("should parse");
        let kept = payload
            .data
            .into_iter()
            .filter_map(|icon| {
                (icon.state == "Completed")
                    .then(|| {
                        Some(GameIcon {
                            universe_id: icon.target_id,
                            image_url: icon.image_url?,
                        })
                    })
                    .flatten()
            })
            .collect::<Vec<_>>();

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].universe_id, 1);

        let malformed = ExploreGame {
            universe_id: 0,
            root_place_id: 5,
            name: "X".into(),
            player_count: 0,
            total_up_votes: 0,
            total_down_votes: 0,
        };
        assert!(malformed.to_summary().is_none());
    }
}
