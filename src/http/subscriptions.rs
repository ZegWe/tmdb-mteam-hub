use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use axum::Router;

use crate::app::AppState;
use crate::http::dto::poll::SubscriptionPollOutcomeDto;
use crate::http::error::ApiError;
use crate::subscription::worker::subscription_poll_policy;

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route(
        "/subscriptions/wanted/poll",
        post(poll_wanted_subscriptions),
    )
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
