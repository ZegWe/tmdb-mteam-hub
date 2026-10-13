use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::post;
use axum::Router;

use crate::app::AppState;
use crate::http::dto::poll::SubscriptionPollOutcomeDto;
use crate::http::dto::subscriptions::SubscriptionSummaryDto;
use crate::http::error::ApiError;
use crate::subscription::queries::GetSubscription;
use crate::subscription::repository::SubscriptionKey;
use crate::subscription::worker::subscription_poll_policy;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/subscriptions/wanted/poll",
            post(poll_wanted_subscriptions),
        )
        .route("/subscriptions/wanted/{id}/retry", post(retry_subscription))
        .route(
            "/subscriptions/wanted/{id}/candidates/confirm",
            post(confirm_subscription_candidate),
        )
        .route(
            "/subscriptions/wanted/{id}/candidates/search",
            post(search_subscription_candidates),
        )
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchCandidatesRequest {
    #[serde(default)]
    source: CandidateSearchSource,
    keyword: String,
    expected_revision: u64,
}

#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum CandidateSearchSource {
    #[default]
    Keyword,
    Douban,
    Imdb,
}

impl CandidateSearchSource {
    fn provider(self) -> crate::app::mteam_search::TorrentSearchSource {
        use crate::app::mteam_search::TorrentSearchSource;
        match self {
            Self::Keyword => TorrentSearchSource::Keyword,
            Self::Douban => TorrentSearchSource::Douban,
            Self::Imdb => TorrentSearchSource::Imdb,
        }
    }

    fn candidate_source(self) -> &'static str {
        // Manually supplied IDs may belong to a different title. Preserve
        // provenance without granting automatic download authorization.
        match self {
            Self::Keyword => "keyword",
            Self::Douban => "manual_douban",
            Self::Imdb => "manual_imdb",
        }
    }

    fn valid_query(self, query: &str) -> bool {
        let id = match self {
            Self::Keyword => return true,
            Self::Douban => query,
            Self::Imdb => query.strip_prefix("tt").unwrap_or(query),
        };
        !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit())
    }
}

async fn search_subscription_candidates(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<SearchCandidatesRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<SubscriptionSummaryDto>, ApiError> {
    use crate::app::mteam_search::MteamSearchCommand;
    use crate::subscription::ports::SubscriptionReadRepository;
    use crate::subscription::repository::payload::{CandidateMatchPayload, CandidatePayload};
    let Path(subject_id) = path.map_err(|_| invalid_subscription_id())?;
    validate_subscription_id(&subject_id)?;
    let Json(body) = body.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_candidate_search",
            "keyword and expected_revision are required",
        )
    })?;
    let keyword = body.keyword.trim();
    if keyword.is_empty()
        || keyword.len() > 256
        || keyword.chars().any(char::is_control)
        || body.expected_revision == 0
        || !body.source.valid_query(keyword)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_candidate_search",
            "请输入有效的名称、豆瓣数字 ID 或 IMDb ID（tt 加数字）",
        ));
    }
    let config = state.config.snapshot().await;
    let account_key =
        crate::douban::auth_cache_key_fragment(&config.value.douban_cookie).map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "subscription_store_unavailable",
                "subscription store is temporarily unavailable",
            )
        })?;
    let key =
        SubscriptionKey::try_new(account_key, subject_id).map_err(|_| invalid_subscription_id())?;
    let current = state
        .subscription_repository
        .load_detail(key.clone())
        .await
        .map_err(retry_repository_error)?;
    if current.summary().head.revision.value() != body.expected_revision
        || current.payload().replacement_pending
    {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "subscription_candidate_conflict",
            "订阅已变化，请刷新后重试",
        ));
    }
    let result = state
        .mteam_search
        .search(MteamSearchCommand {
            source: body.source.provider(),
            query: keyword.to_string(),
            page: 1,
            page_size: 100,
        })
        .await?;
    let candidates = result
        .items
        .into_iter()
        .map(|item| CandidateMatchPayload {
            candidate: CandidatePayload {
                torrent_id: item.id,
                title: item.name,
                subtitle: item.small_description.unwrap_or_default(),
                source: body.source.candidate_source().to_string(),
                search_query: keyword.to_string(),
                size: item.size.map(|size| size.to_string()),
                seeders: item.seeders,
                leechers: item.leechers,
                uploaded_at: item.created_at,
            },
            excluded_reason: Some("手动搜索结果，请核对名称、年份和季集数后选择".to_string()),
            ..Default::default()
        })
        .collect();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let summary =
        state
            .subscription_repository
            .add_manual_candidates(key, body.expected_revision, candidates, now)
            .await
            .map_err(|error| match error {
                crate::subscription::repository::RepositoryError::InvalidInput {
                    field, ..
                } if field.starts_with("confirm_candidate.") => ApiError::new(
                    StatusCode::CONFLICT,
                    "subscription_candidate_conflict",
                    "订阅已变化，请刷新后重试",
                ),
                other => retry_repository_error(other),
            })?;
    Ok(Json(SubscriptionSummaryDto::from(&summary)))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfirmCandidateRequest {
    torrent_id: String,
    expected_revision: u64,
    #[serde(default)]
    replace_existing: bool,
}

async fn confirm_subscription_candidate(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
    body: Result<Json<ConfirmCandidateRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<SubscriptionSummaryDto>, ApiError> {
    let Path(subject_id) = path.map_err(|_| invalid_subscription_id())?;
    validate_subscription_id(&subject_id)?;
    let Json(body) = body.map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_candidate_selection",
            "torrent_id and expected_revision are required",
        )
    })?;
    if body.torrent_id.trim().is_empty()
        || body.torrent_id.trim() != body.torrent_id
        || body.torrent_id.len() > 256
        || body.torrent_id.chars().any(char::is_control)
        || body.expected_revision == 0
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid_candidate_selection",
            "candidate selection is invalid",
        ));
    }
    let config = state.config.snapshot().await;
    let account_key =
        crate::douban::auth_cache_key_fragment(&config.value.douban_cookie).map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "subscription_store_unavailable",
                "subscription store is temporarily unavailable",
            )
        })?;
    let key =
        SubscriptionKey::try_new(account_key, subject_id).map_err(|_| invalid_subscription_id())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let selection = if body.replace_existing {
        state.subscription_repository.replace_candidate(
            key,
            body.torrent_id,
            body.expected_revision,
            now,
        )
    } else {
        state.subscription_repository.confirm_candidate(
            key,
            body.torrent_id,
            body.expected_revision,
            now,
        )
    };
    let summary = selection.await.map_err(|error| match error {
        crate::subscription::repository::RepositoryError::InvalidInput { field, .. }
            if field.starts_with("confirm_candidate.") =>
        {
            ApiError::new(
                StatusCode::CONFLICT,
                "subscription_candidate_conflict",
                "候选已变化或当前不可选择，请刷新订阅详情后重试",
            )
        }
        other => retry_repository_error(other),
    })?;
    Ok(Json(SubscriptionSummaryDto::from(&summary)))
}

async fn poll_wanted_subscriptions(
    State(state): State<AppState>,
) -> Result<Json<SubscriptionPollOutcomeDto>, ApiError> {
    let config = state.config.snapshot().await;
    let policy = subscription_poll_policy(&config.value)?;
    state
        .subscription_poll
        .poll(&policy)
        .await
        .map(Into::into)
        .map(Json)
        .map_err(ApiError::from)
}

async fn retry_subscription(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<SubscriptionSummaryDto>, ApiError> {
    let config = state.config.snapshot().await;
    let account_key =
        crate::douban::auth_cache_key_fragment(&config.value.douban_cookie).map_err(|_| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "subscription_store_unavailable",
                "subscription store is temporarily unavailable",
            )
        })?;
    let Path(subject_id) = path.map_err(|_| invalid_subscription_id())?;
    validate_subscription_id(&subject_id)?;
    let key = SubscriptionKey::try_new(&account_key, &subject_id)
        .map_err(|_| invalid_subscription_id())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    state
        .subscription_repository
        .force_retry(key, now)
        .await
        .map_err(retry_repository_error)?;
    let command =
        GetSubscription::try_new(account_key, subject_id).map_err(|_| invalid_subscription_id())?;
    let detail = state
        .subscription_queries
        .get_subscription(command)
        .await
        .map_err(|error| {
            use crate::subscription::queries::SubscriptionQueryErrorKind;
            match error.kind() {
                SubscriptionQueryErrorKind::NotFound => ApiError::new(
                    StatusCode::NOT_FOUND,
                    "subscription_not_found",
                    "subscription not found",
                ),
                SubscriptionQueryErrorKind::Validation => ApiError::new(
                    StatusCode::BAD_REQUEST,
                    "invalid_subscription_id",
                    "subscription id is invalid",
                ),
                SubscriptionQueryErrorKind::Unavailable => ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "subscription_store_unavailable",
                    "subscription store is temporarily unavailable",
                ),
                SubscriptionQueryErrorKind::Internal => ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "internal server error",
                ),
            }
        })?;
    Ok(Json(SubscriptionSummaryDto::from(detail.summary())))
}

fn validate_subscription_id(value: &str) -> Result<(), ApiError> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.len() > 256
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.contains(['/', '\\'])
    {
        return Err(invalid_subscription_id());
    }
    Ok(())
}

fn invalid_subscription_id() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalid_subscription_id",
        "subscription id is invalid",
    )
}

fn retry_repository_error(error: crate::subscription::repository::RepositoryError) -> ApiError {
    use crate::subscription::repository::RepositoryError;

    match error {
        RepositoryError::NotFound { .. } => ApiError::new(
            StatusCode::NOT_FOUND,
            "subscription_not_found",
            "subscription not found",
        ),
        RepositoryError::ExecutionGateConflict { .. } => ApiError::new(
            StatusCode::CONFLICT,
            "subscription_execution_running",
            "subscription is running; retry after the current attempt finishes",
        ),
        RepositoryError::InvalidInput {
            field: "force_retry.completed_tv",
            ..
        } => ApiError::new(
            StatusCode::CONFLICT,
            "subscription_rerun_unsupported",
            "completed TV subscriptions cannot be rerun",
        ),
        RepositoryError::InvalidInput {
            field: "force_retry",
            ..
        } => ApiError::new(
            StatusCode::CONFLICT,
            "subscription_not_schedulable",
            "subscription is inactive or blocked and cannot be rerun",
        ),
        RepositoryError::Unavailable { .. } => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "subscription_store_unavailable",
            "subscription store is temporarily unavailable",
        ),
        error => {
            tracing::error!(error = %error, "failed to rerun subscription");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal server error",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    use super::retry_repository_error;
    use crate::subscription::repository::{RepositoryError, SubscriptionKey};

    #[test]
    fn manual_search_supports_ids_without_granting_automatic_download_permission() {
        use super::SearchCandidatesRequest;
        use crate::subscription::repository::payload::{CandidateMatchPayload, CandidatePayload};
        for (source, query, valid) in [
            ("keyword", "Movie 2026", true),
            ("douban", "1292052", true),
            ("douban", "tt0111161", false),
            ("imdb", "tt0111161", true),
            ("imdb", "0111161", true),
            ("imdb", "tt", false),
            ("imdb", "movie", false),
        ] {
            let body: SearchCandidatesRequest = serde_json::from_value(
                serde_json::json!({ "source": source, "keyword": query, "expected_revision": 1 }),
            )
            .unwrap();
            assert_eq!(body.source.provider().as_str(), source);
            assert_eq!(body.source.valid_query(query), valid);
            let mut candidate = CandidateMatchPayload {
                candidate: CandidatePayload {
                    source: body.source.candidate_source().to_string(),
                    ..Default::default()
                },
                ..Default::default()
            };
            assert!(!candidate.download_authorized());
            candidate.user_confirmed = true;
            assert!(candidate.download_authorized());
        }
        let legacy: SearchCandidatesRequest = serde_json::from_value(
            serde_json::json!({ "keyword": "Movie", "expected_revision": 1 }),
        )
        .unwrap();
        assert_eq!(legacy.source.provider().as_str(), "keyword");
        assert!(serde_json::from_value::<SearchCandidatesRequest>(
            serde_json::json!({ "source": "unknown", "keyword": "Movie", "expected_revision": 1 })
        )
        .is_err());
    }

    #[test]
    fn retry_errors_distinguish_busy_and_blocked_subscriptions_from_storage_failures() {
        let key = SubscriptionKey::try_new("account", "subject").unwrap();
        for (error, status) in [
            (
                RepositoryError::NotFound { key: key.clone() },
                StatusCode::NOT_FOUND,
            ),
            (
                RepositoryError::ExecutionGateConflict { key },
                StatusCode::CONFLICT,
            ),
            (
                RepositoryError::InvalidInput {
                    field: "force_retry",
                    message: "blocked".to_string(),
                },
                StatusCode::CONFLICT,
            ),
            (
                RepositoryError::InvalidInput {
                    field: "force_retry.completed_tv",
                    message: "completed TV".to_string(),
                },
                StatusCode::CONFLICT,
            ),
            (
                RepositoryError::Unavailable {
                    message: "busy database".to_string(),
                },
                StatusCode::SERVICE_UNAVAILABLE,
            ),
        ] {
            assert_eq!(
                retry_repository_error(error).into_response().status(),
                status
            );
        }
    }
}
