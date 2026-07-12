use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;

use crate::app::AppState;
use crate::douban;
use crate::http::dto::douban::{
    DoubanInterestResponseDto, DoubanLibraryQuery, DoubanLibraryResponseDto,
    DoubanQrPollResponseDto, DoubanQrQuery, DoubanQrStartResponseDto, DoubanSearchQuery,
    DoubanSearchResponseDto, DoubanSubjectDetailDto, DoubanTagHistoryQuery,
    DoubanTagHistoryResponseDto, MarkDoubanInterestRequestDto,
};
use crate::http::error::{ApiError, ApiJson, ApiPath, ApiQuery};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/douban/search", get(douban_search))
        .route("/douban/library", get(douban_library))
        .route("/douban/tags", get(douban_tag_history))
        .route("/douban/subject/{id}", get(douban_subject_detail))
        .route("/douban/subject/{id}/interest", post(douban_mark_interest))
        .route("/douban/image", get(douban_image))
        .route("/douban/qr/start", post(douban_qr_start))
        .route("/douban/qr/poll", get(douban_qr_poll))
        .route("/douban/qr/image", get(douban_qr_image))
}

async fn douban_search(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<DoubanSearchQuery>,
) -> Result<Json<DoubanSearchResponseDto>, ApiError> {
    let outcome = state.douban_catalog.search(q.into()).await?;
    Ok(Json(outcome.into()))
}

async fn douban_library(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<DoubanLibraryQuery>,
) -> Result<Json<DoubanLibraryResponseDto>, ApiError> {
    let outcome = state.douban_catalog.library(q.into()).await?;
    Ok(Json(outcome.into()))
}

async fn douban_tag_history(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<DoubanTagHistoryQuery>,
) -> Result<Json<DoubanTagHistoryResponseDto>, ApiError> {
    let outcome = state.douban_catalog.tag_history(q.into()).await?;
    Ok(Json(outcome.into()))
}

async fn douban_subject_detail(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<DoubanSubjectDetailDto>, ApiError> {
    let detail = state.douban_catalog.subject_detail(id).await?;
    Ok(Json(detail.into()))
}

async fn douban_mark_interest(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    ApiJson(body): ApiJson<MarkDoubanInterestRequestDto>,
) -> Result<Json<DoubanInterestResponseDto>, ApiError> {
    let outcome = state
        .douban_catalog
        .mark_interest(body.into_command(id))
        .await?;
    Ok(Json(outcome.into()))
}

#[derive(Deserialize)]
struct DoubanImageQuery {
    url: String,
}

async fn douban_image(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<DoubanImageQuery>,
) -> Result<Response, ApiError> {
    let (content_type, bytes) = douban::fetch_image(&state.upstream_clients.douban, &q.url)
        .await
        .map_err(ApiError::douban)?;
    let mut headers = HeaderMap::new();
    let content_type = HeaderValue::from_str(&content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("image/jpeg"));
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    Ok((headers, axum::body::Body::from(bytes)).into_response())
}

async fn douban_qr_start(
    State(state): State<AppState>,
) -> Result<Json<DoubanQrStartResponseDto>, ApiError> {
    let outcome = state.douban_catalog.start_qr().await?;
    Ok(Json(outcome.into()))
}

async fn douban_qr_image(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<DoubanQrQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let image = state.douban_catalog.qr_image(q.into_session_id()).await?;
    Ok((
        [(header::CONTENT_TYPE, "image/png")],
        image.as_ref().clone(),
    ))
}

async fn douban_qr_poll(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<DoubanQrQuery>,
) -> Result<Json<DoubanQrPollResponseDto>, ApiError> {
    let outcome = state.douban_catalog.poll_qr(q.into_session_id()).await?;
    Ok(Json(outcome.into()))
}
