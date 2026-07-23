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
        .map_err(|error| {
            use crate::subscription::repository::RepositoryError;
            match error {
                RepositoryError::NotFound { .. } => ApiError::new(
                    StatusCode::NOT_FOUND,
                    "subscription_not_found",
                    "subscription not found",
                ),
                _ => ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "internal server error",
                ),
            }
        })?;
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
