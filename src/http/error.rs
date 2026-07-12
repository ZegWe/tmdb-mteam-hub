use axum::extract::rejection::{JsonRejection, PathRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::request::Parts;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::de::DeserializeOwned;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(crate) struct ApiErrorBody {
    code: &'static str,
    message: String,
}

#[derive(Debug)]
pub(crate) struct ApiError {
    status: StatusCode,
    body: ApiErrorBody,
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ApiErrorBody {
                code,
                message: message.into(),
            },
        }
    }

    pub(crate) fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::handler(StatusCode::BAD_REQUEST, message)
    }

    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::handler(StatusCode::CONFLICT, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::handler(StatusCode::INTERNAL_SERVER_ERROR, message)
    }

    pub(crate) fn douban(error: crate::douban::DoubanError) -> Self {
        if error.is_bad_request() {
            Self::bad_request(error.message)
        } else {
            Self::handler(StatusCode::BAD_GATEWAY, error.message)
        }
    }

    pub(crate) fn message(&self) -> &str {
        &self.body.message
    }

    pub(crate) fn handler(status: StatusCode, message: impl Into<String>) -> Self {
        let message = message.into();
        let public_message = match status {
            StatusCode::BAD_GATEWAY => "upstream service request failed".to_string(),
            StatusCode::SERVICE_UNAVAILABLE => "service is temporarily unavailable".to_string(),
            StatusCode::GATEWAY_TIMEOUT => "upstream service timed out".to_string(),
            status if status.is_server_error() => "internal server error".to_string(),
            _ => message,
        };
        Self::new(status, handler_error_code(status), public_message)
    }

    pub(crate) fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", "API endpoint not found")
    }

    pub(crate) fn method_not_allowed() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "HTTP method not allowed for this endpoint",
        )
    }
}

impl From<crate::clients::http::ClientError> for ApiError {
    fn from(error: crate::clients::http::ClientError) -> Self {
        let safe_message = error.to_string();
        match error {
            crate::clients::http::ClientError::InvalidRequest { message, .. } => {
                Self::bad_request(message)
            }
            crate::clients::http::ClientError::Timeout { .. } => {
                Self::handler(StatusCode::GATEWAY_TIMEOUT, safe_message)
            }
            _ => Self::handler(StatusCode::BAD_GATEWAY, safe_message),
        }
    }
}

impl From<crate::app::manual_qb::ManualQbError> for ApiError {
    fn from(error: crate::app::manual_qb::ManualQbError) -> Self {
        match error {
            crate::app::manual_qb::ManualQbError::Validation { message } => {
                Self::bad_request(message)
            }
            crate::app::manual_qb::ManualQbError::Upstream(error) => error.into(),
        }
    }
}

impl From<crate::app::mteam_search::MteamSearchError> for ApiError {
    fn from(error: crate::app::mteam_search::MteamSearchError) -> Self {
        match error {
            crate::app::mteam_search::MteamSearchError::Validation { message } => {
                Self::bad_request(message)
            }
            crate::app::mteam_search::MteamSearchError::Upstream(error) => error.into(),
        }
    }
}

impl From<crate::app::media_catalog::MediaCatalogError> for ApiError {
    fn from(error: crate::app::media_catalog::MediaCatalogError) -> Self {
        match error {
            crate::app::media_catalog::MediaCatalogError::Validation { message } => {
                Self::bad_request(message)
            }
            crate::app::media_catalog::MediaCatalogError::Upstream(error) => error.into(),
        }
    }
}

impl From<crate::app::douban_catalog::DoubanCatalogError> for ApiError {
    fn from(error: crate::app::douban_catalog::DoubanCatalogError) -> Self {
        match error {
            crate::app::douban_catalog::DoubanCatalogError::Validation { message } => {
                Self::bad_request(message)
            }
            crate::app::douban_catalog::DoubanCatalogError::Upstream { message } => {
                Self::handler(StatusCode::BAD_GATEWAY, message)
            }
            crate::app::douban_catalog::DoubanCatalogError::Internal { message } => {
                Self::internal(message)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubscriptionQueryTarget {
    List,
    Detail,
}

pub(crate) fn subscription_query_error(
    error: crate::subscription::queries::SubscriptionQueryError,
    target: SubscriptionQueryTarget,
) -> ApiError {
    use crate::subscription::queries::SubscriptionQueryErrorKind;

    match error.kind() {
        SubscriptionQueryErrorKind::Validation => match target {
            SubscriptionQueryTarget::List => ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_subscription_query",
                "subscription query parameters are invalid",
            ),
            SubscriptionQueryTarget::Detail => ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_subscription_id",
                "subscription id is invalid",
            ),
        },
        SubscriptionQueryErrorKind::NotFound => ApiError::new(
            StatusCode::NOT_FOUND,
            "subscription_not_found",
            "subscription not found",
        ),
        SubscriptionQueryErrorKind::Unavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "subscription_store_unavailable",
            "subscription store is temporarily unavailable",
        ),
        SubscriptionQueryErrorKind::Internal => ApiError::internal("subscription query failed"),
    }
}

impl From<crate::subscription::worker::SubscriptionPollError> for ApiError {
    fn from(error: crate::subscription::worker::SubscriptionPollError) -> Self {
        use crate::subscription::worker::SubscriptionPollErrorKind;

        match error.kind() {
            SubscriptionPollErrorKind::Validation => Self::new(
                StatusCode::BAD_REQUEST,
                "invalid_subscription_poll",
                error.message(),
            ),
            SubscriptionPollErrorKind::Upstream => Self::new(
                StatusCode::BAD_GATEWAY,
                "subscription_poll_upstream_error",
                error.message(),
            ),
            SubscriptionPollErrorKind::Conflict => Self::new(
                StatusCode::CONFLICT,
                "subscription_poll_superseded",
                "subscription Poll was superseded by a newer attempt",
            ),
            SubscriptionPollErrorKind::Unavailable => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "subscription_store_unavailable",
                "subscription store is temporarily unavailable",
            ),
            SubscriptionPollErrorKind::Internal => Self::internal("subscription Poll failed"),
        }
    }
}

pub(crate) fn config_update_error(error: crate::config::ConfigUpdateError) -> ApiError {
    match error {
        crate::config::ConfigUpdateError::Stale { expected, actual } => ApiError::conflict(
            format!("配置 revision 已过期: expected={expected}, current={actual}"),
        ),
        crate::config::ConfigUpdateError::Mutation(message) => ApiError::bad_request(message),
        crate::config::ConfigUpdateError::Persist(error) => {
            ApiError::internal(format!("写入配置失败: {error}"))
        }
    }
}

fn handler_error_code(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::METHOD_NOT_ALLOWED => "method_not_allowed",
        StatusCode::REQUEST_TIMEOUT => "request_timeout",
        StatusCode::CONFLICT => "conflict",
        StatusCode::PAYLOAD_TOO_LARGE => "payload_too_large",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "unsupported_media_type",
        StatusCode::UNPROCESSABLE_ENTITY => "invalid_request",
        StatusCode::TOO_MANY_REQUESTS => "rate_limited",
        StatusCode::BAD_GATEWAY => "upstream_error",
        StatusCode::SERVICE_UNAVAILABLE => "service_unavailable",
        StatusCode::GATEWAY_TIMEOUT => "upstream_timeout",
        status if status.is_server_error() => "internal_error",
        _ => "request_failed",
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body)).into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
    }
}

#[derive(Debug)]
pub(crate) struct ApiJson<T>(pub(crate) T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(json_rejection)
    }
}

fn json_rejection(rejection: JsonRejection) -> ApiError {
    match rejection {
        JsonRejection::JsonDataError(error) => ApiError::new(
            error.status(),
            "invalid_request_body",
            "request body does not match the expected schema",
        ),
        JsonRejection::JsonSyntaxError(error) => ApiError::new(
            error.status(),
            "invalid_json",
            "request body contains invalid JSON",
        ),
        JsonRejection::MissingJsonContentType(error) => ApiError::new(
            error.status(),
            "unsupported_media_type",
            "request body must use application/json",
        ),
        JsonRejection::BytesRejection(error) => match error.status() {
            StatusCode::PAYLOAD_TOO_LARGE => ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "request body exceeds the allowed size",
            ),
            status if status.is_server_error() => {
                ApiError::handler(status, "failed to read request body")
            }
            status => ApiError::new(
                status,
                "invalid_request_body",
                "request body could not be read",
            ),
        },
        _ => {
            let status = rejection.status();
            if status.is_server_error() {
                ApiError::handler(status, "failed to extract request body")
            } else {
                ApiError::new(status, "invalid_request_body", "request body is invalid")
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct ApiQuery<T>(pub(crate) T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(query_rejection)
    }
}

fn query_rejection(rejection: QueryRejection) -> ApiError {
    let status = rejection.status();
    if status.is_server_error() {
        ApiError::handler(status, "failed to extract query parameters")
    } else {
        ApiError::new(status, "invalid_query", "query parameters are invalid")
    }
}

#[derive(Debug)]
pub(crate) struct ApiPath<T>(pub(crate) T);

impl<S, T> FromRequestParts<S> for ApiPath<T>
where
    S: Send + Sync,
    T: DeserializeOwned + Send,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(path_rejection)
    }
}

fn path_rejection(rejection: PathRejection) -> ApiError {
    let status = rejection.status();
    if status.is_server_error() {
        ApiError::handler(status, "failed to extract path parameters")
    } else {
        ApiError::new(status, "invalid_path", "path parameters are invalid")
    }
}

pub(crate) async fn not_found() -> ApiError {
    ApiError::not_found()
}

pub(crate) async fn method_not_allowed() -> ApiError {
    ApiError::method_not_allowed()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Request};
    use http_body_util::BodyExt;
    use serde_json::Value;

    use super::*;

    async fn response_json(error: ApiError) -> (StatusCode, Value, Option<String>) {
        let response = error.into_response();
        let status = response.status();
        let cache_control = response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect API error body")
            .to_bytes();
        let value = serde_json::from_slice(&body).expect("parse API error body");
        (status, value, cache_control)
    }

    #[tokio::test]
    async fn error_envelope_is_flat_and_omits_absent_details() {
        let (status, body, cache_control) = response_json(ApiError::not_found()).await;

        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["code"], "not_found");
        assert_eq!(body["message"], "API endpoint not found");
        assert!(body.get("details").is_none());
        assert_eq!(cache_control.as_deref(), Some("no-store"));
    }

    #[tokio::test]
    async fn handler_errors_hide_internal_and_upstream_details() {
        const SECRET: &str = "SECRET_URL_OR_UPSTREAM_BODY_MUST_NOT_LEAK";

        for (status, code, message) in [
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "internal server error",
            ),
            (
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "upstream service request failed",
            ),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "service is temporarily unavailable",
            ),
        ] {
            let (_, body, _) = response_json(ApiError::handler(status, SECRET)).await;

            assert_eq!(body["code"], code);
            assert_eq!(body["message"], message);
            assert!(!body.to_string().contains(SECRET));
            assert!(body.get("error").is_none());
        }
    }

    #[tokio::test]
    async fn api_json_maps_axum_rejections_to_the_shared_envelope() {
        #[derive(Debug, serde::Deserialize)]
        struct Payload {
            _name: String,
        }

        let request = Request::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{"))
            .expect("build malformed JSON request");
        let error = ApiJson::<Payload>::from_request(request, &())
            .await
            .expect_err("malformed JSON should fail");
        let (status, body, _) = response_json(error).await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["code"], "invalid_json");
        assert!(body["message"].is_string());
    }
}
