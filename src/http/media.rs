use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use crate::app::AppState;
use crate::http::dto::media::{MediaSearchResponseDto, TmdbMediaDetailDto, TmdbSeasonDetailDto};
use crate::http::error::{ApiError, ApiPath, ApiQuery};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/search", get(search))
        .route("/tmdb/movie/{id}", get(movie_detail))
        .route("/tmdb/tv/{id}/season/{season}", get(tv_season_detail))
        .route("/tmdb/tv/{id}", get(tv_detail))
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    language: Option<String>,
}

async fn search(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<SearchQuery>,
) -> Result<Json<MediaSearchResponseDto>, ApiError> {
    let outcome = state.media_catalog.search(query.q, query.language).await?;
    Ok(Json(outcome.into()))
}

#[derive(Debug, Deserialize, Default)]
struct DetailQuery {
    #[serde(default)]
    force_refresh: bool,
}

async fn movie_detail(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i32>,
    ApiQuery(query): ApiQuery<DetailQuery>,
) -> Result<Json<TmdbMediaDetailDto>, ApiError> {
    let detail = state
        .media_catalog
        .movie_detail(id, query.force_refresh)
        .await?;
    Ok(Json(detail.into()))
}

async fn tv_detail(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i32>,
    ApiQuery(query): ApiQuery<DetailQuery>,
) -> Result<Json<TmdbMediaDetailDto>, ApiError> {
    let detail = state
        .media_catalog
        .tv_detail(id, query.force_refresh)
        .await?;
    Ok(Json(detail.into()))
}

async fn tv_season_detail(
    State(state): State<AppState>,
    ApiPath((tv_id, season_number)): ApiPath<(i32, i32)>,
    ApiQuery(query): ApiQuery<DetailQuery>,
) -> Result<Json<TmdbSeasonDetailDto>, ApiError> {
    let detail = state
        .media_catalog
        .tv_season_detail(tv_id, season_number, query.force_refresh)
        .await?;
    Ok(Json(detail.into()))
}
