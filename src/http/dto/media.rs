use serde::Serialize;

use crate::app::media_catalog::{
    MediaSearchItem, MediaSearchOutcome, NamedMediaValue, TmdbEpisode, TmdbMediaDetail,
    TmdbSeasonDetail, TmdbSeasonSummary,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct MediaSearchResponseDto {
    movies: Vec<MediaSearchItemDto>,
    tv: Vec<MediaSearchItemDto>,
}

impl From<MediaSearchOutcome> for MediaSearchResponseDto {
    fn from(outcome: MediaSearchOutcome) -> Self {
        Self {
            movies: outcome.movies.into_iter().map(Into::into).collect(),
            tv: outcome.tv.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct MediaSearchItemDto {
    media_type: &'static str,
    id: i32,
    title: String,
    original_title: String,
    overview: String,
    poster_path: Option<String>,
    poster_url: Option<String>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    vote_average: Option<f64>,
}

impl From<MediaSearchItem> for MediaSearchItemDto {
    fn from(item: MediaSearchItem) -> Self {
        Self {
            media_type: item.media_kind.as_str(),
            id: item.id,
            title: item.title,
            original_title: item.original_title,
            overview: item.overview,
            poster_path: item.poster_path,
            poster_url: item.poster_url,
            release_date: item.release_date,
            first_air_date: item.first_air_date,
            vote_average: item.vote_average,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct TmdbMediaDetailDto {
    media_type: &'static str,
    id: i32,
    title: String,
    original_title: String,
    overview: String,
    tagline: Option<String>,
    poster_path: Option<String>,
    poster_url: Option<String>,
    backdrop_path: Option<String>,
    backdrop_url: Option<String>,
    release_date: Option<String>,
    first_air_date: Option<String>,
    last_air_date: Option<String>,
    runtime: Option<u32>,
    status: Option<String>,
    vote_average: Option<f64>,
    vote_count: Option<u64>,
    genres: Vec<NamedMediaValueDto>,
    production_countries: Vec<NamedMediaValueDto>,
    spoken_languages: Vec<NamedMediaValueDto>,
    origin_country: Vec<String>,
    imdb_id: Option<String>,
    douban_id: Option<String>,
    douban_url: Option<String>,
    number_of_seasons: Option<u32>,
    number_of_episodes: Option<u32>,
    episode_run_time: Vec<u32>,
    networks: Vec<NamedMediaValueDto>,
    series_type: Option<String>,
    seasons: Vec<TmdbSeasonSummaryDto>,
}

impl From<TmdbMediaDetail> for TmdbMediaDetailDto {
    fn from(detail: TmdbMediaDetail) -> Self {
        Self {
            media_type: detail.media_kind.as_str(),
            id: detail.id,
            title: detail.title,
            original_title: detail.original_title,
            overview: detail.overview,
            tagline: detail.tagline,
            poster_path: detail.poster_path,
            poster_url: detail.poster_url,
            backdrop_path: detail.backdrop_path,
            backdrop_url: detail.backdrop_url,
            release_date: detail.release_date,
            first_air_date: detail.first_air_date,
            last_air_date: detail.last_air_date,
            runtime: detail.runtime,
            status: detail.status,
            vote_average: detail.vote_average,
            vote_count: detail.vote_count,
            genres: detail.genres.into_iter().map(Into::into).collect(),
            production_countries: detail
                .production_countries
                .into_iter()
                .map(Into::into)
                .collect(),
            spoken_languages: detail
                .spoken_languages
                .into_iter()
                .map(Into::into)
                .collect(),
            origin_country: detail.origin_country,
            imdb_id: detail.imdb_id,
            douban_id: detail.douban_id,
            douban_url: detail.douban_url,
            number_of_seasons: detail.number_of_seasons,
            number_of_episodes: detail.number_of_episodes,
            episode_run_time: detail.episode_run_time,
            networks: detail.networks.into_iter().map(Into::into).collect(),
            series_type: detail.series_type,
            seasons: detail.seasons.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NamedMediaValueDto {
    id: Option<i32>,
    name: String,
    english_name: Option<String>,
    code: Option<String>,
}

impl From<NamedMediaValue> for NamedMediaValueDto {
    fn from(value: NamedMediaValue) -> Self {
        Self {
            id: value.id,
            name: value.name,
            english_name: value.english_name,
            code: value.code,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct TmdbSeasonSummaryDto {
    season_number: i32,
    name: String,
    episode_count: Option<u32>,
    air_date: Option<String>,
    poster_path: Option<String>,
    poster_url: Option<String>,
}

impl From<TmdbSeasonSummary> for TmdbSeasonSummaryDto {
    fn from(season: TmdbSeasonSummary) -> Self {
        Self {
            season_number: season.season_number,
            name: season.name,
            episode_count: season.episode_count,
            air_date: season.air_date,
            poster_path: season.poster_path,
            poster_url: season.poster_url,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct TmdbSeasonDetailDto {
    id: Option<i32>,
    season_number: i32,
    name: String,
    overview: String,
    air_date: Option<String>,
    poster_path: Option<String>,
    poster_url: Option<String>,
    episodes: Vec<TmdbEpisodeDto>,
}

impl From<TmdbSeasonDetail> for TmdbSeasonDetailDto {
    fn from(season: TmdbSeasonDetail) -> Self {
        Self {
            id: season.id,
            season_number: season.season_number,
            name: season.name,
            overview: season.overview,
            air_date: season.air_date,
            poster_path: season.poster_path,
            poster_url: season.poster_url,
            episodes: season.episodes.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct TmdbEpisodeDto {
    id: Option<i32>,
    episode_number: i32,
    name: String,
    overview: String,
    air_date: Option<String>,
    still_path: Option<String>,
    still_url: Option<String>,
    runtime: Option<u32>,
    vote_average: Option<f64>,
}

impl From<TmdbEpisode> for TmdbEpisodeDto {
    fn from(episode: TmdbEpisode) -> Self {
        Self {
            id: episode.id,
            episode_number: episode.episode_number,
            name: episode.name,
            overview: episode.overview,
            air_date: episode.air_date,
            still_path: episode.still_path,
            still_url: episode.still_url,
            runtime: episode.runtime,
            vote_average: episode.vote_average,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::app::media_catalog::TmdbMediaKind;

    #[test]
    fn detail_dto_has_closed_unified_names_without_provider_aliases() {
        let dto = TmdbMediaDetailDto::from(TmdbMediaDetail {
            media_kind: TmdbMediaKind::Tv,
            id: 84,
            title: "剧集".to_string(),
            original_title: "Series".to_string(),
            overview: String::new(),
            tagline: None,
            poster_path: None,
            poster_url: None,
            backdrop_path: None,
            backdrop_url: None,
            release_date: None,
            first_air_date: None,
            last_air_date: None,
            runtime: None,
            status: None,
            vote_average: None,
            vote_count: None,
            genres: Vec::new(),
            production_countries: Vec::new(),
            spoken_languages: Vec::new(),
            origin_country: Vec::new(),
            imdb_id: Some("tt0084".to_string()),
            douban_id: None,
            douban_url: None,
            number_of_seasons: Some(1),
            number_of_episodes: Some(8),
            episode_run_time: vec![45],
            networks: Vec::new(),
            series_type: Some("Scripted".to_string()),
            seasons: Vec::new(),
        });
        let value = serde_json::to_value(dto).unwrap();

        assert_eq!(value["media_type"], "tv");
        assert_eq!(value["title"], "剧集");
        assert_eq!(value["original_title"], "Series");
        assert_eq!(value["series_type"], "Scripted");
        for provider_alias in ["name", "original_name", "external_ids", "type"] {
            assert_eq!(
                value.get(provider_alias),
                None,
                "provider alias {provider_alias}"
            );
        }
        assert_eq!(
            value,
            json!({
                "media_type": "tv", "id": 84, "title": "剧集", "original_title": "Series",
                "overview": "", "tagline": null, "poster_path": null, "poster_url": null,
                "backdrop_path": null, "backdrop_url": null, "release_date": null,
                "first_air_date": null, "last_air_date": null, "runtime": null, "status": null,
                "vote_average": null, "vote_count": null, "genres": [],
                "production_countries": [], "spoken_languages": [], "origin_country": [],
                "imdb_id": "tt0084", "douban_id": null, "douban_url": null,
                "number_of_seasons": 1, "number_of_episodes": 8, "episode_run_time": [45],
                "networks": [], "series_type": "Scripted", "seasons": []
            })
        );
    }
}
