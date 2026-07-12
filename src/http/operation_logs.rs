use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use crate::app::AppState;
use crate::http::dto::operation_logs::OperationLogPageDto;
use crate::http::error::ApiError;
use crate::http::error::ApiQuery;
use crate::subscription;

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/operation-logs", get(operation_logs))
}

#[derive(Deserialize, Default)]
struct OperationLogsQuery {
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
}

async fn operation_logs(
    State(state): State<AppState>,
    ApiQuery(query): ApiQuery<OperationLogsQuery>,
) -> Result<Json<OperationLogPageDto>, ApiError> {
    let page = state
        .subscription_repository
        .query_operation_logs(subscription::OperationLogQuery {
            account_key: None,
            category: query.category,
            status: query.status,
            q: query.q,
            page: query.page,
            page_size: query.page_size,
        })
        .await
        .map_err(|error| ApiError::internal(format!("读取操作日志失败: {error}")))?;
    Ok(Json(page.into()))
}
