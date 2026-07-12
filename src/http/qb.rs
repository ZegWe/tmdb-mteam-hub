use axum::extract::State;
use axum::response::Json;
use axum::routing::post;
use axum::Router;
use serde::{Deserialize, Serialize};

use crate::app::manual_qb::ManualQbPushCommand;
use crate::app::AppState;
use crate::http::error::{ApiError, ApiJson};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/qb/test", post(qb_test))
        .route("/qb/push-mteam", post(qb_push_mteam))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QbTestRequest {
    server_id: String,
}

#[derive(Debug, Serialize)]
struct QbTestResponse {
    ok: bool,
    version: String,
}

async fn qb_test(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<QbTestRequest>,
) -> Result<Json<QbTestResponse>, ApiError> {
    let version = state.manual_qb.test_connection(body.server_id).await?;
    Ok(Json(QbTestResponse { ok: true, version }))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QbPushMteamRequest {
    server_id: String,
    torrent_id: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    savepath: Option<String>,
}

#[derive(Debug, Serialize)]
struct QbPushMteamResponse {
    ok: bool,
}

async fn qb_push_mteam(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<QbPushMteamRequest>,
) -> Result<Json<QbPushMteamResponse>, ApiError> {
    let outcome = state
        .manual_qb
        .push_mteam(ManualQbPushCommand {
            server_id: body.server_id,
            torrent_id: body.torrent_id,
            category: body.category,
            savepath: body.savepath,
        })
        .await?;
    Ok(Json(QbPushMteamResponse { ok: outcome.added }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_reject_inline_server_credentials() {
        use serde_json::json;

        let test_error = serde_json::from_value::<QbTestRequest>(json!({
            "server_id": "nas",
            "base_url": "http://127.0.0.1:8080",
            "username": "admin",
            "password": "SECRET_MUST_NOT_BE_ACCEPTED"
        }))
        .unwrap_err();
        let push_error = serde_json::from_value::<QbPushMteamRequest>(json!({
            "server_id": "nas",
            "torrent_id": "42",
            "server": { "password": "SECRET_MUST_NOT_BE_ACCEPTED" }
        }))
        .unwrap_err();
        assert!(test_error.to_string().contains("unknown field"));
        assert!(push_error.to_string().contains("unknown field"));
        assert!(!test_error
            .to_string()
            .contains("SECRET_MUST_NOT_BE_ACCEPTED"));
        assert!(!push_error
            .to_string()
            .contains("SECRET_MUST_NOT_BE_ACCEPTED"));
    }
}
