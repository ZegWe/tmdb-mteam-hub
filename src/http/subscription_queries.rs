#[cfg(test)]
use std::sync::Arc;

use axum::extract::rejection::{PathRejection, QueryRejection};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use crate::app::redaction::SubscriptionDiagnosticRedactor;
use crate::app::AppState;
use crate::config::ConfigManager;
use crate::douban;
use crate::http::dto::subscriptions::{
    CursorCodecError, ListCursorScope, OpaqueListCursor, SubscriptionDetailDto,
    SubscriptionListResponse,
};
use crate::http::error::{subscription_query_error, ApiError, SubscriptionQueryTarget};
#[cfg(test)]
use crate::subscription::ports::SubscriptionReadRepository;
use crate::subscription::queries::{GetSubscription, ListSubscriptions, SubscriptionQueryService};
use crate::subscription::repository::SubscriptionListFilter;
use crate::subscription::{
    SubscriptionAttentionTag, SubscriptionLifecycleState, SubscriptionMediaKind,
};

const DEFAULT_LIST_LIMIT: u32 = 50;
const MAX_SUBSCRIPTION_ID_BYTES: usize = 256;

/// Test-only state for exercising the HTTP adapter against a recording port.
#[cfg(test)]
#[derive(Clone)]
pub(crate) struct SubscriptionQueryHttpState {
    config: ConfigManager,
    service: Arc<SubscriptionQueryService>,
}

#[cfg(test)]
impl SubscriptionQueryHttpState {
    pub(crate) fn new(
        config: ConfigManager,
        repository: Arc<dyn SubscriptionReadRepository>,
    ) -> Self {
        Self {
            config,
            service: Arc::new(SubscriptionQueryService::new(repository)),
        }
    }
}

struct SubscriptionRequestContext {
    account_key: String,
    diagnostic_redactor: SubscriptionDiagnosticRedactor,
}

async fn request_context(config: &ConfigManager) -> Result<SubscriptionRequestContext, ApiError> {
    let snapshot = config.snapshot().await;
    let account_key = douban::auth_cache_key_fragment(&snapshot.value.douban_cookie)
        .map_err(|_| subscription_store_unavailable())?;
    let diagnostic_redactor = SubscriptionDiagnosticRedactor::from_config(&snapshot.value);
    Ok(SubscriptionRequestContext {
        account_key,
        diagnostic_redactor,
    })
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/subscriptions/wanted", get(list_subscriptions))
        .route("/subscriptions/wanted/{id}", get(get_subscription))
}

#[cfg(test)]
pub(crate) fn staged_routes(state: SubscriptionQueryHttpState) -> Router {
    Router::new()
        .route("/subscriptions/wanted", get(staged_list_subscriptions))
        .route("/subscriptions/wanted/{id}", get(staged_get_subscription))
        .with_state(state)
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ListSubscriptionQuery {
    limit: Option<u32>,
    active: Option<bool>,
    media_kind: Option<SubscriptionMediaKind>,
    lifecycle_state: Option<SubscriptionLifecycleState>,
    attention_tag: Option<SubscriptionAttentionTag>,
    cursor: Option<String>,
}

struct ParsedListSubscriptionQuery {
    filter: SubscriptionListFilter,
    cursor: Option<crate::subscription::repository::ListCursor>,
    limit: u32,
    cursor_scope: ListCursorScope,
}

async fn list_subscriptions(
    State(state): State<AppState>,
    query: Result<Query<ListSubscriptionQuery>, QueryRejection>,
) -> Result<Json<SubscriptionListResponse>, ApiError> {
    list_subscriptions_with(&state.config, &state.subscription_queries, query).await
}

#[cfg(test)]
async fn staged_list_subscriptions(
    State(state): State<SubscriptionQueryHttpState>,
    query: Result<Query<ListSubscriptionQuery>, QueryRejection>,
) -> Result<Json<SubscriptionListResponse>, ApiError> {
    list_subscriptions_with(&state.config, state.service.as_ref(), query).await
}

async fn list_subscriptions_with(
    config: &ConfigManager,
    service: &SubscriptionQueryService,
    query: Result<Query<ListSubscriptionQuery>, QueryRejection>,
) -> Result<Json<SubscriptionListResponse>, ApiError> {
    let context = request_context(config).await?;
    let Query(query) = query.map_err(|_| invalid_subscription_query())?;
    let parsed = parse_list_query(query, &context.account_key)?;
    let command = ListSubscriptions::try_new(
        context.account_key,
        parsed.filter,
        parsed.cursor,
        parsed.limit,
    )
    .map_err(|error| subscription_query_error(error, SubscriptionQueryTarget::List))?;
    let page = service
        .list_subscriptions(command)
        .await
        .map_err(|error| subscription_query_error(error, SubscriptionQueryTarget::List))?;
    let response = SubscriptionListResponse::try_from_page_with_scope(&page, parsed.cursor_scope)
        .map_err(|_| internal_error())?;
    Ok(Json(response))
}

fn parse_list_query(
    query: ListSubscriptionQuery,
    account_key: &str,
) -> Result<ParsedListSubscriptionQuery, ApiError> {
    let limit = query.limit.unwrap_or(DEFAULT_LIST_LIMIT);
    if !(1..=100).contains(&limit) {
        return Err(invalid_subscription_query());
    }
    let filter = SubscriptionListFilter {
        active: query.active,
        media_kind: query.media_kind,
        lifecycle_state: query.lifecycle_state,
        attention_tag: query.attention_tag,
    };
    let cursor_scope = ListCursorScope::new(account_key, &filter);
    let cursor = query
        .cursor
        .map(|token| {
            OpaqueListCursor::try_from(token)
                .map_err(map_cursor_parse_error)?
                .decode(cursor_scope)
                .map_err(map_cursor_decode_error)
        })
        .transpose()?;
    Ok(ParsedListSubscriptionQuery {
        filter,
        cursor,
        limit,
        cursor_scope,
    })
}

fn map_cursor_parse_error(error: CursorCodecError) -> ApiError {
    match error {
        CursorCodecError::UnsupportedVersion { .. } => ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported_cursor_version",
            "subscription cursor version is not supported",
        ),
        CursorCodecError::ScopeMismatch => cursor_scope_mismatch(),
        CursorCodecError::InvalidEncoding
        | CursorCodecError::InvalidPayload
        | CursorCodecError::Oversized { .. } => invalid_cursor(),
    }
}

fn map_cursor_decode_error(error: CursorCodecError) -> ApiError {
    match error {
        CursorCodecError::ScopeMismatch => cursor_scope_mismatch(),
        other => map_cursor_parse_error(other),
    }
}

async fn get_subscription(
    State(state): State<AppState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<SubscriptionDetailDto>, ApiError> {
    get_subscription_with(&state.config, &state.subscription_queries, path).await
}

#[cfg(test)]
async fn staged_get_subscription(
    State(state): State<SubscriptionQueryHttpState>,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<SubscriptionDetailDto>, ApiError> {
    get_subscription_with(&state.config, state.service.as_ref(), path).await
}

async fn get_subscription_with(
    config: &ConfigManager,
    service: &SubscriptionQueryService,
    path: Result<Path<String>, PathRejection>,
) -> Result<Json<SubscriptionDetailDto>, ApiError> {
    let context = request_context(config).await?;
    let Path(subject_id) = path.map_err(|_| invalid_subscription_id())?;
    validate_subscription_id(&subject_id)?;
    let command = GetSubscription::try_new(context.account_key, subject_id)
        .map_err(|error| subscription_query_error(error, SubscriptionQueryTarget::Detail))?;
    let detail = service
        .get_subscription(command)
        .await
        .map_err(|error| subscription_query_error(error, SubscriptionQueryTarget::Detail))?;
    let dto = SubscriptionDetailDto::from_detail(&detail, &|value| {
        context.diagnostic_redactor.redact(value)
    });
    Ok(Json(dto))
}

fn validate_subscription_id(value: &str) -> Result<(), ApiError> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.len() > MAX_SUBSCRIPTION_ID_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
        || value.contains(['/', '\\'])
    {
        return Err(invalid_subscription_id());
    }
    Ok(())
}

fn invalid_subscription_query() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalid_subscription_query",
        "subscription query parameters are invalid",
    )
}

fn invalid_cursor() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalid_cursor",
        "subscription cursor is invalid",
    )
}

fn cursor_scope_mismatch() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "cursor_scope_mismatch",
        "subscription cursor does not match the requested list scope",
    )
}

fn subscription_store_unavailable() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "subscription_store_unavailable",
        "subscription store is temporarily unavailable",
    )
}

fn invalid_subscription_id() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "invalid_subscription_id",
        "subscription id is invalid",
    )
}

fn internal_error() -> ApiError {
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "internal server error",
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::{staged_routes, validate_subscription_id, SubscriptionQueryHttpState};
    use crate::config::{ConfigManager, FileConfig, ManagementConfig, QbServerEntry};
    use crate::http::dto::subscriptions::{ListCursorScope, OpaqueListCursor};
    use crate::subscription::ports::{RepoFuture, SubscriptionReadRepository};
    use crate::subscription::repository::payload::{
        ArtifactPayload, IssueOwnerPayload, IssuePayload, ObservationPayload, SubscriptionPayload,
        WantedSourcePayload,
    };
    use crate::subscription::repository::{
        ListCursor, ListSubscriptionsCommand, RepositoryError, RepositoryResult, Revision,
        SubscriptionDetail, SubscriptionHead, SubscriptionKey, SubscriptionListFilter,
        SubscriptionListPage, SubscriptionProjection, SubscriptionSummary,
    };
    use crate::subscription::{
        SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
        SubscriptionMediaKind,
    };

    const ACCOUNT: &str = "trusted-account-must-not-leak";
    const SUBJECT: &str = "subject-1";
    const CONFIG_SECRET: &str = "MTEAM_SECRET_MUST_NOT_LEAK";
    const REPOSITORY_SECRET: &str = "REPOSITORY_SECRET_MUST_NOT_LEAK";

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ReadCall {
        Get(SubscriptionKey),
        List(ListSubscriptionsCommand),
        LoadDetail(SubscriptionKey),
    }

    struct RecordingRepository {
        calls: Mutex<Vec<ReadCall>>,
        list_results: Mutex<VecDeque<RepositoryResult<SubscriptionListPage>>>,
        detail_results: Mutex<VecDeque<RepositoryResult<SubscriptionDetail>>>,
    }

    impl RecordingRepository {
        fn new(
            list_results: impl IntoIterator<Item = RepositoryResult<SubscriptionListPage>>,
            detail_results: impl IntoIterator<Item = RepositoryResult<SubscriptionDetail>>,
        ) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                list_results: Mutex::new(list_results.into_iter().collect()),
                detail_results: Mutex::new(detail_results.into_iter().collect()),
            }
        }

        fn calls(&self) -> Vec<ReadCall> {
            self.calls.lock().expect("lock recorded calls").clone()
        }
    }

    impl SubscriptionReadRepository for RecordingRepository {
        fn get(&self, key: SubscriptionKey) -> RepoFuture<SubscriptionHead> {
            self.calls
                .lock()
                .expect("record get call")
                .push(ReadCall::Get(key));
            Box::pin(async {
                Err(RepositoryError::Internal {
                    message: "unexpected get call".to_string(),
                })
            })
        }

        fn list_summaries(
            &self,
            command: ListSubscriptionsCommand,
        ) -> RepoFuture<SubscriptionListPage> {
            self.calls
                .lock()
                .expect("record list call")
                .push(ReadCall::List(command));
            let result = self
                .list_results
                .lock()
                .expect("lock list results")
                .pop_front()
                .expect("one configured list result");
            Box::pin(async move { result })
        }

        fn load_detail(&self, key: SubscriptionKey) -> RepoFuture<SubscriptionDetail> {
            self.calls
                .lock()
                .expect("record detail call")
                .push(ReadCall::LoadDetail(key));
            let result = self
                .detail_results
                .lock()
                .expect("lock detail results")
                .pop_front()
                .expect("one configured detail result");
            Box::pin(async move { result })
        }
    }

    fn router(repository: Arc<RecordingRepository>, config: ConfigManager) -> axum::Router {
        axum::Router::new().nest(
            "/api",
            staged_routes(SubscriptionQueryHttpState::new(config, repository)),
        )
    }

    fn temp_test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tmdb-mteam-subscription-query-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn config_manager(config: FileConfig) -> ConfigManager {
        ConfigManager::new(temp_test_root("snapshot").join("config.toml"), config)
    }

    async fn send(app: axum::Router, uri: &str) -> (StatusCode, Value) {
        let response = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .expect("build subscription query request"),
            )
            .await
            .expect("call subscription query route");
        let status = response.status();
        let body = response
            .into_body()
            .collect()
            .await
            .expect("collect subscription query response")
            .to_bytes();
        let value = serde_json::from_slice(&body).expect("parse subscription query response");
        (status, value)
    }

    fn source() -> WantedSourcePayload {
        WantedSourcePayload {
            title: "Fixture Movie".to_string(),
            release_year: Some(2026),
            poster_url: "https://images.test/poster.jpg".to_string(),
            category_text: Some("movie".to_string()),
            douban_sort_time: Some(123),
            ..WantedSourcePayload::default()
        }
    }

    fn summary_for(account_key: &str) -> SubscriptionSummary {
        let source = source();
        SubscriptionSummary {
            head: SubscriptionHead {
                key: SubscriptionKey::try_new(account_key, SUBJECT).unwrap(),
                revision: Revision::try_new(1).unwrap(),
                active: true,
                inactive_at: None,
                last_seen_snapshot_id: None,
                media_kind: SubscriptionMediaKind::Movie,
                schedulable: true,
                blocked_reason: None,
                lifecycle_state: SubscriptionLifecycleState::Queued,
                execution_state: SubscriptionExecutionState::Idle,
                next_attempt_at: Some(10),
                retry_count: 0,
                max_retries: 3,
                retry_blocked: false,
                force_eligible_once: false,
                updated_at: 3,
            },
            projection: SubscriptionProjection::from_source(&source).unwrap(),
            attention_tags: vec![SubscriptionAttentionTag::WaitingRelease],
        }
    }

    fn summary() -> SubscriptionSummary {
        summary_for(ACCOUNT)
    }

    fn detail_with_diagnostic(account_key: &str, diagnostic: String) -> SubscriptionDetail {
        let payload = SubscriptionPayload {
            source: source(),
            observation: ObservationPayload {
                created_at: 1,
                first_seen_at: 2,
                last_seen_at: 3,
            },
            issues: vec![IssuePayload {
                owner: IssueOwnerPayload::Parent,
                operation: Some(format!("fetch:{diagnostic}")),
                error_type: Some("upstream".to_string()),
                message: diagnostic.clone(),
                occurred_at: Some(3),
            }],
            skip_reason: Some(format!("skip:{diagnostic}")),
            artifacts: ArtifactPayload::default(),
            ..SubscriptionPayload::default()
        };
        SubscriptionDetail::try_new(summary_for(account_key), payload).unwrap()
    }

    fn detail_with_sensitive_diagnostic() -> SubscriptionDetail {
        detail_with_diagnostic(
            ACCOUNT,
            format!(
                "failed with {CONFIG_SECRET} at https://user:password@example.test/file?passkey=url-secret"
            ),
        )
    }

    fn empty_page() -> SubscriptionListPage {
        SubscriptionListPage {
            items: Vec::new(),
            next_cursor: None,
        }
    }

    fn base_config() -> FileConfig {
        FileConfig {
            douban_cookie: format!("dbcl2={ACCOUNT}:COOKIE_SECRET; ck=COOKIE_CK"),
            ..FileConfig::default()
        }
    }

    fn redaction_config() -> FileConfig {
        FileConfig {
            mteam_api_key: CONFIG_SECRET.to_string(),
            douban_cookie: format!("dbcl2={ACCOUNT}:COOKIE_SECRET; ck=COOKIE_CK"),
            management: ManagementConfig {
                admin_token: "MANAGEMENT_SECRET_123456789".to_string(),
                ..ManagementConfig::default()
            },
            qb_servers: vec![QbServerEntry {
                id: "qb".to_string(),
                name: "qB".to_string(),
                base_url: "http://qb.test".to_string(),
                username: "admin".to_string(),
                password: "QB_SECRET".to_string(),
                insecure_tls: false,
            }],
            ..FileConfig::default()
        }
    }

    #[tokio::test]
    async fn list_defaults_limit_and_returns_only_items_and_scoped_next_cursor() {
        let page = SubscriptionListPage {
            items: vec![summary()],
            next_cursor: Some(ListCursor::try_new(Some(123), SUBJECT).unwrap()),
        };
        let repository = Arc::new(RecordingRepository::new([Ok(page)], []));
        let app = router(repository.clone(), config_manager(base_config()));

        let (status, body) = send(app, "/api/subscriptions/wanted").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_object().unwrap().len(), 2);
        assert!(body["items"].is_array());
        assert_eq!(body["items"][0]["subject_id"], SUBJECT);
        assert!(!body.to_string().contains(ACCOUNT));
        let cursor = OpaqueListCursor::try_from(body["next_cursor"].as_str().unwrap()).unwrap();
        assert_eq!(
            cursor
                .decode(ListCursorScope::new(
                    ACCOUNT,
                    &SubscriptionListFilter::default()
                ))
                .unwrap(),
            ListCursor::try_new(Some(123), SUBJECT).unwrap()
        );
        assert_eq!(
            repository.calls(),
            vec![ReadCall::List(
                ListSubscriptionsCommand::try_new(
                    ACCOUNT,
                    SubscriptionListFilter::default(),
                    None,
                    50,
                )
                .unwrap()
            )]
        );
    }

    #[tokio::test]
    async fn list_parses_all_filters_boundary_limit_cursor_and_ignores_account_header() {
        let filter = SubscriptionListFilter {
            active: Some(true),
            media_kind: Some(SubscriptionMediaKind::Movie),
            lifecycle_state: Some(SubscriptionLifecycleState::Downloading),
            attention_tag: Some(SubscriptionAttentionTag::Failed),
        };
        let list_cursor = ListCursor::try_new(None, "subject-before").unwrap();
        let cursor =
            OpaqueListCursor::encode(&list_cursor, ListCursorScope::new(ACCOUNT, &filter)).unwrap();
        let repository = Arc::new(RecordingRepository::new([Ok(empty_page())], []));
        let app = router(repository.clone(), config_manager(base_config()));
        let uri = format!(
            "/api/subscriptions/wanted?limit=100&active=true&media_kind=movie&lifecycle_state=downloading&attention_tag=failed&cursor={}",
            cursor.as_str()
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("x-account-key", "attacker-account")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            repository.calls(),
            vec![ReadCall::List(
                ListSubscriptionsCommand::try_new(ACCOUNT, filter, Some(list_cursor), 100,)
                    .unwrap()
            )]
        );
    }

    #[tokio::test]
    async fn list_accepts_the_lower_limit_boundary() {
        let repository = Arc::new(RecordingRepository::new([Ok(empty_page())], []));

        let (status, _) = send(
            router(repository.clone(), config_manager(base_config())),
            "/api/subscriptions/wanted?limit=1",
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            repository.calls(),
            vec![ReadCall::List(
                ListSubscriptionsCommand::try_new(
                    ACCOUNT,
                    SubscriptionListFilter::default(),
                    None,
                    1,
                )
                .unwrap()
            )]
        );
    }

    #[tokio::test]
    async fn list_rejects_invalid_or_account_bearing_queries_before_repository_access() {
        let cases = [
            "/api/subscriptions/wanted?limit=0",
            "/api/subscriptions/wanted?limit=101",
            "/api/subscriptions/wanted?limit=abc",
            "/api/subscriptions/wanted?active=1",
            "/api/subscriptions/wanted?media_kind=series",
            "/api/subscriptions/wanted?lifecycle_state=unknown",
            "/api/subscriptions/wanted?attention_tag=unknown",
            "/api/subscriptions/wanted?account_key=attacker-account",
        ];
        for uri in cases {
            let repository = Arc::new(RecordingRepository::new([], []));
            let (status, body) = send(
                router(repository.clone(), config_manager(base_config())),
                uri,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "URI {uri}");
            assert_eq!(body["code"], "invalid_subscription_query", "URI {uri}");
            assert!(repository.calls().is_empty(), "URI {uri}");
        }
    }

    #[tokio::test]
    async fn cursor_errors_keep_version_token_and_scope_failures_distinct() {
        let foreign_cursor = OpaqueListCursor::encode(
            &ListCursor::try_new(Some(1), "before").unwrap(),
            ListCursorScope::new("foreign-account", &SubscriptionListFilter::default()),
        )
        .unwrap();
        let cases = [
            (
                "/api/subscriptions/wanted?cursor=v1.00".to_string(),
                "unsupported_cursor_version",
            ),
            (
                "/api/subscriptions/wanted?cursor=v2.gg".to_string(),
                "invalid_cursor",
            ),
            (
                format!(
                    "/api/subscriptions/wanted?cursor={}",
                    foreign_cursor.as_str()
                ),
                "cursor_scope_mismatch",
            ),
        ];
        for (uri, expected_code) in cases {
            let repository = Arc::new(RecordingRepository::new([], []));
            let (status, body) = send(
                router(repository.clone(), config_manager(base_config())),
                &uri,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(body["code"], expected_code);
            assert!(repository.calls().is_empty());
        }
    }

    #[tokio::test]
    async fn detail_forwards_exact_trusted_key_and_uses_config_aware_redaction() {
        let repository = Arc::new(RecordingRepository::new(
            [],
            [Ok(detail_with_sensitive_diagnostic())],
        ));
        let app = router(repository.clone(), config_manager(redaction_config()));

        let (status, body) = send(app, "/api/subscriptions/wanted/subject-1").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["summary"]["subject_id"], SUBJECT);
        assert!(body["issues"][0]["message"]
            .as_str()
            .unwrap()
            .contains("[REDACTED]"));
        let raw = body.to_string();
        for secret in [
            ACCOUNT,
            CONFIG_SECRET,
            "user:password",
            "url-secret",
            "MANAGEMENT_SECRET_123456789",
            "QB_SECRET",
        ] {
            assert!(!raw.contains(secret), "detail leaked {secret}");
        }
        assert_eq!(
            repository.calls(),
            vec![ReadCall::LoadDetail(
                SubscriptionKey::try_new(ACCOUNT, SUBJECT).unwrap()
            )]
        );
    }

    #[tokio::test]
    async fn one_router_uses_one_fresh_config_snapshot_per_request_after_hot_update() {
        const ACCOUNT_A: &str = "account-a";
        const ACCOUNT_B: &str = "account-b";
        const COOKIE_A: &str = "dbcl2=account-a:COOKIE_A_SECRET; ck=COOKIE_A_CK";
        const COOKIE_B: &str = "dbcl2=account-b:COOKIE_B_SECRET; ck=COOKIE_B_CK";
        const MTEAM_B: &str = "MTEAM_B_SECRET";
        const QB_B: &str = "QB_B_SECRET";
        const URL_USER_B: &str = "dynamic-user-b";
        const URL_PASSWORD_B: &str = "dynamic-password-b";
        const URL_PASSKEY_B: &str = "dynamic-passkey-b";

        let first_page = SubscriptionListPage {
            items: vec![summary_for(ACCOUNT_A)],
            next_cursor: Some(ListCursor::try_new(Some(123), SUBJECT).unwrap()),
        };
        let diagnostic_b = format!(
            "{COOKIE_B} | {MTEAM_B} | {QB_B} | https://{URL_USER_B}:{URL_PASSWORD_B}@example.test/file?passkey={URL_PASSKEY_B}"
        );
        let repository = Arc::new(RecordingRepository::new(
            [Ok(first_page)],
            [Ok(detail_with_diagnostic(ACCOUNT_B, diagnostic_b))],
        ));
        let root = temp_test_root("hot-update");
        let manager = ConfigManager::new(
            root.join("config.toml"),
            FileConfig {
                douban_cookie: COOKIE_A.to_string(),
                ..FileConfig::default()
            },
        );
        let app = router(repository.clone(), manager.clone());

        let (first_status, first_body) = send(app.clone(), "/api/subscriptions/wanted").await;
        assert_eq!(first_status, StatusCode::OK);
        let old_cursor = first_body["next_cursor"].as_str().unwrap().to_string();

        let revision = manager.snapshot().await.revision;
        manager
            .update(Some(revision), |config| {
                config.douban_cookie = COOKIE_B.to_string();
                config.mteam_api_key = MTEAM_B.to_string();
                config.qb_servers = vec![QbServerEntry {
                    id: "qb-b".to_string(),
                    name: "qB B".to_string(),
                    base_url: "http://qb-b.test".to_string(),
                    username: "admin".to_string(),
                    password: QB_B.to_string(),
                    insecure_tls: false,
                }];
                Ok(())
            })
            .await
            .unwrap();

        let (detail_status, detail_body) =
            send(app.clone(), "/api/subscriptions/wanted/subject-1").await;
        assert_eq!(detail_status, StatusCode::OK);
        let raw_detail = detail_body.to_string();
        assert!(raw_detail.contains("[REDACTED]"));
        for secret in [
            ACCOUNT_B,
            COOKIE_B,
            "COOKIE_B_SECRET",
            "COOKIE_B_CK",
            MTEAM_B,
            QB_B,
            URL_USER_B,
            URL_PASSWORD_B,
            URL_PASSKEY_B,
        ] {
            assert!(
                !raw_detail.contains(secret),
                "updated detail leaked {secret}"
            );
        }

        let (cursor_status, cursor_body) = send(
            app,
            &format!("/api/subscriptions/wanted?cursor={old_cursor}"),
        )
        .await;
        assert_eq!(cursor_status, StatusCode::BAD_REQUEST);
        assert_eq!(cursor_body["code"], "cursor_scope_mismatch");
        assert_eq!(
            repository.calls(),
            vec![
                ReadCall::List(
                    ListSubscriptionsCommand::try_new(
                        ACCOUNT_A,
                        SubscriptionListFilter::default(),
                        None,
                        50,
                    )
                    .unwrap()
                ),
                ReadCall::LoadDetail(SubscriptionKey::try_new(ACCOUNT_B, SUBJECT).unwrap()),
            ]
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn invalid_account_configuration_is_a_generic_server_error() {
        const INVALID_COOKIE: &str = "ck=SERVER_CONFIG_SECRET_MUST_NOT_LEAK";
        let repository = Arc::new(RecordingRepository::new([], []));
        let app = router(
            repository.clone(),
            config_manager(FileConfig {
                douban_cookie: INVALID_COOKIE.to_string(),
                ..FileConfig::default()
            }),
        );

        for uri in [
            "/api/subscriptions/wanted",
            "/api/subscriptions/wanted/subject-1",
        ] {
            let (status, body) = send(app.clone(), uri).await;
            assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(body["code"], "subscription_store_unavailable");
            let raw = body.to_string();
            assert!(!raw.contains(INVALID_COOKIE));
            assert!(!raw.contains("SERVER_CONFIG_SECRET_MUST_NOT_LEAK"));
        }
        assert!(repository.calls().is_empty());
    }

    #[test]
    fn detail_id_validation_is_byte_bounded_and_rejects_unsafe_segments() {
        for invalid in [
            "",
            " subject",
            "subject ",
            "subject\n",
            "subject\0id",
            "subject/id",
            "subject\\id",
            ".",
            "..",
            &"a".repeat(257),
            &"界".repeat(86),
        ] {
            assert!(
                validate_subscription_id(invalid).is_err(),
                "accepted invalid ID {invalid:?}"
            );
        }
        assert!(validate_subscription_id("界".repeat(85).as_str()).is_ok());
        assert!(validate_subscription_id("a".repeat(256).as_str()).is_ok());
        assert!(validate_subscription_id("subject-1").is_ok());
    }

    #[tokio::test]
    async fn detail_router_rejects_encoded_unsafe_ids_before_repository_access() {
        for uri in [
            "/api/subscriptions/wanted/%20subject",
            "/api/subscriptions/wanted/subject%20",
            "/api/subscriptions/wanted/subject%00id",
            "/api/subscriptions/wanted/subject%0Aid",
            "/api/subscriptions/wanted/subject%2Fid",
            "/api/subscriptions/wanted/subject%5Cid",
            "/api/subscriptions/wanted/%2E",
            "/api/subscriptions/wanted/%2E%2E",
        ] {
            let repository = Arc::new(RecordingRepository::new([], []));
            let (status, body) = send(
                router(repository.clone(), config_manager(base_config())),
                uri,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "URI {uri}");
            assert_eq!(body["code"], "invalid_subscription_id", "URI {uri}");
            assert!(repository.calls().is_empty(), "URI {uri}");
        }
    }

    #[tokio::test]
    async fn query_error_mapping_is_generic_and_never_exposes_repository_details() {
        let errors = [
            RepositoryError::NotFound {
                key: SubscriptionKey::try_new(ACCOUNT, "missing").unwrap(),
            },
            RepositoryError::UnsupportedSchema {
                found: 99,
                maximum_supported: 5,
            },
            RepositoryError::Unavailable {
                message: REPOSITORY_SECRET.to_string(),
            },
            RepositoryError::CorruptData {
                message: REPOSITORY_SECRET.to_string(),
            },
            RepositoryError::Internal {
                message: REPOSITORY_SECRET.to_string(),
            },
        ];
        let repository = Arc::new(RecordingRepository::new([], errors.into_iter().map(Err)));
        let app = router(repository, config_manager(base_config()));
        let expectations = [
            ("missing", StatusCode::NOT_FOUND, "subscription_not_found"),
            (
                "schema",
                StatusCode::SERVICE_UNAVAILABLE,
                "subscription_store_unavailable",
            ),
            (
                "unavailable",
                StatusCode::SERVICE_UNAVAILABLE,
                "subscription_store_unavailable",
            ),
            (
                "corrupt",
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
            ),
            (
                "internal",
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
            ),
        ];
        for (subject, expected_status, expected_code) in expectations {
            let (status, body) =
                send(app.clone(), &format!("/api/subscriptions/wanted/{subject}")).await;
            assert_eq!(status, expected_status);
            assert_eq!(body["code"], expected_code);
            let raw = body.to_string();
            assert!(!raw.contains(ACCOUNT));
            assert!(!raw.contains(REPOSITORY_SECRET));
            assert!(!raw.contains("99"));
        }
    }

    #[tokio::test]
    async fn list_repository_failures_use_stable_public_codes_without_diagnostics() {
        let repository = Arc::new(RecordingRepository::new(
            [
                Err(RepositoryError::InvalidInput {
                    field: "filter",
                    message: REPOSITORY_SECRET.to_string(),
                }),
                Err(RepositoryError::Unavailable {
                    message: REPOSITORY_SECRET.to_string(),
                }),
                Err(RepositoryError::Internal {
                    message: REPOSITORY_SECRET.to_string(),
                }),
            ],
            [],
        ));
        let app = router(repository, config_manager(base_config()));
        for (status, code) in [
            (StatusCode::BAD_REQUEST, "invalid_subscription_query"),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "subscription_store_unavailable",
            ),
            (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        ] {
            let (actual_status, body) = send(app.clone(), "/api/subscriptions/wanted").await;
            assert_eq!(actual_status, status);
            assert_eq!(body["code"], code);
            assert!(!body.to_string().contains(REPOSITORY_SECRET));
        }
    }

    #[tokio::test]
    async fn error_responses_are_no_store() {
        let repository = Arc::new(RecordingRepository::new([], []));
        let response = router(repository, config_manager(base_config()))
            .oneshot(
                Request::builder()
                    .uri("/api/subscriptions/wanted?limit=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({
                "code": "invalid_subscription_query",
                "message": "subscription query parameters are invalid"
            })
        );
    }
}
