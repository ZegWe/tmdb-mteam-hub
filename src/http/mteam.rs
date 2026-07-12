use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::app::mteam_search::{
    MteamSearchCommand, MteamSearchOutcome, MteamTorrent, TorrentSearchSource,
};
use crate::app::AppState;
use crate::http::error::{ApiError, ApiQuery};

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/mteam/torrents", get(search))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum MteamSource {
    Imdb,
    Douban,
    Keyword,
}

impl From<MteamSource> for TorrentSearchSource {
    fn from(source: MteamSource) -> Self {
        match source {
            MteamSource::Imdb => Self::Imdb,
            MteamSource::Douban => Self::Douban,
            MteamSource::Keyword => Self::Keyword,
        }
    }
}

#[derive(Debug, Deserialize)]
struct MteamQuery {
    source: MteamSource,
    #[serde(default)]
    imdb_id: Option<String>,
    #[serde(default)]
    douban_id: Option<String>,
    #[serde(default)]
    keyword: Option<String>,
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default = "default_page_size")]
    page_size: u32,
}

impl MteamQuery {
    fn into_command(self) -> MteamSearchCommand {
        let query = match self.source {
            MteamSource::Imdb => self.imdb_id.unwrap_or_default(),
            MteamSource::Douban => self.douban_id.unwrap_or_default(),
            MteamSource::Keyword => self.keyword.unwrap_or_default(),
        };
        MteamSearchCommand {
            source: self.source.into(),
            query,
            page: self.page,
            page_size: self.page_size,
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct MteamSearchResponse {
    items: Vec<MteamTorrentDto>,
    page: u32,
    page_size: u32,
}

impl From<MteamSearchOutcome> for MteamSearchResponse {
    fn from(outcome: MteamSearchOutcome) -> Self {
        Self {
            items: outcome.items.into_iter().map(Into::into).collect(),
            page: outcome.page,
            page_size: outcome.page_size,
        }
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct MteamTorrentDto {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    small_description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seeders: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    leechers: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
}

impl From<MteamTorrent> for MteamTorrentDto {
    fn from(torrent: MteamTorrent) -> Self {
        Self {
            id: torrent.id,
            name: torrent.name,
            small_description: torrent.small_description,
            size: torrent.size,
            seeders: torrent.seeders,
            leechers: torrent.leechers,
            created_at: torrent.created_at,
        }
    }
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    50
}

async fn search(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<MteamQuery>,
) -> Result<Json<MteamSearchResponse>, ApiError> {
    let outcome = state.mteam_search.search(query.into_command()).await?;
    Ok(Json(outcome.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_selects_exactly_one_source_value() {
        let command = MteamQuery {
            source: MteamSource::Douban,
            imdb_id: Some("tt-ignored".to_string()),
            douban_id: Some("1295644".to_string()),
            keyword: Some("ignored".to_string()),
            page: 2,
            page_size: 25,
        }
        .into_command();

        assert_eq!(command.source, TorrentSearchSource::Douban);
        assert_eq!(command.query, "1295644");
        assert_eq!(command.page, 2);
        assert_eq!(command.page_size, 25);
    }

    #[test]
    fn response_has_one_named_candidate_shape() {
        let response = MteamSearchResponse::from(MteamSearchOutcome {
            items: vec![MteamTorrent {
                id: "42".to_string(),
                name: "Movie".to_string(),
                small_description: Some("UHD".to_string()),
                size: Some(4096),
                seeders: Some(8),
                leechers: Some(2),
                created_at: Some("2026-07-12".to_string()),
            }],
            page: 1,
            page_size: 50,
        });

        assert_eq!(response.items[0].id, "42");
        assert_eq!(response.items[0].small_description.as_deref(), Some("UHD"));
        assert_eq!(response.page, 1);
        assert_eq!(response.page_size, 50);
    }
}
