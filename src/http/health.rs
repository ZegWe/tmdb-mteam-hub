use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::State;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::app::AppState;
use crate::http::error::ApiError;
const READINESS_CACHE_TTL: Duration = Duration::from_secs(10);
const READINESS_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Serialize)]
struct HealthStatus {
    status: &'static str,
}

#[derive(Clone)]
struct HealthState {
    app: AppState,
    readiness: Arc<ReadinessCoordinator>,
}

impl HealthState {
    fn new(app: AppState) -> Self {
        Self {
            app,
            readiness: Arc::new(ReadinessCoordinator::default()),
        }
    }
}

#[derive(Default)]
struct ReadinessCoordinator {
    cache: StdMutex<Option<CachedReadiness>>,
    #[cfg(test)]
    completed_checks: AtomicU64,
}

impl ReadinessCoordinator {
    fn cached(&self) -> Option<Result<(), String>> {
        let cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        cache
            .as_ref()
            .filter(|entry| entry.checked_at.elapsed() < READINESS_CACHE_TTL)
            .map(|entry| entry.result.clone())
    }

    fn remember(&self, result: Result<(), String>) {
        let mut cache = self
            .cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *cache = Some(CachedReadiness {
            checked_at: Instant::now(),
            result,
        });
    }

    #[cfg(test)]
    fn record_completed_check(&self) {
        self.completed_checks.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn completed_checks(&self) -> u64 {
        self.completed_checks.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
struct CachedReadiness {
    checked_at: Instant,
    result: Result<(), String>,
}

pub(crate) fn routes(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .method_not_allowed_fallback(crate::http::error::method_not_allowed)
        .with_state(HealthState::new(state))
}

async fn healthz() -> Response {
    healthy_response()
}

async fn readyz(State(state): State<HealthState>) -> Response {
    if let Some(result) = state.readiness.cached() {
        return readiness_response(result);
    }

    let result = match tokio::time::timeout(
        READINESS_CHECK_TIMEOUT,
        state.app.subscription_repository.preflight(),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "readiness check failed");
            Err(error.to_string())
        }
        Err(_) => {
            tracing::warn!("readiness check timed out");
            Err("subscription readiness check timed out".to_string())
        }
    };
    #[cfg(test)]
    state.readiness.record_completed_check();
    state.readiness.remember(result.clone());
    readiness_response(result)
}

fn readiness_response(result: Result<(), String>) -> Response {
    match result {
        Ok(()) => healthy_response(),
        Err(_) => not_ready_response(),
    }
}

fn not_ready_response() -> Response {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "not_ready",
        "service not ready",
    )
    .into_response()
}

fn healthy_response() -> Response {
    let mut response = Json(HealthStatus { status: "ok" }).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::Body;
    use axum::extract::State;
    use axum::http::{header, Request, StatusCode};
    use http_body_util::BodyExt;
    use rusqlite::Connection;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::{readyz, HealthState};
    use crate::app::{AppPaths, AppState};
    use crate::http::router::build_router;

    fn temp_test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-health-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create health test root");
        root
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect health response")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("parse health response JSON")
    }

    #[tokio::test]
    async fn health_and_latest_readiness_are_unauthenticated_and_minimal() {
        let root = temp_test_root("router");
        let paths = AppPaths::for_test_root(&root);
        fs::create_dir_all(&paths.static_dir).expect("create static directory");
        fs::write(paths.static_dir.join("index.html"), "health fixture")
            .expect("write static index");
        let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());
        let database = paths.subscription_state_dir.join("subscriptions.sqlite");
        let before = fs::read(&database).expect("read latest database before readiness");

        for path in ["/healthz", "/readyz"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("build health request"),
                )
                .await
                .expect("call health route");
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(
                body_json(response).await,
                serde_json::json!({ "status": "ok" })
            );
        }
        assert_eq!(
            fs::read(&database).expect("read latest database after readiness"),
            before,
            "readiness must not repair or rewrite the latest database"
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn readiness_ignores_old_wanted_sqlite_without_mutating_it() {
        let root = temp_test_root("ignore-old-database");
        let paths = AppPaths::for_test_root(&root);
        fs::create_dir_all(&paths.subscription_state_dir).expect("create state directory");
        let old_database = paths.subscription_state_dir.join("wanted.sqlite");
        let sentinel = b"OLD_DATABASE_MUST_STAY_BYTE_IDENTICAL";
        fs::write(&old_database, sentinel).expect("write old database sentinel");
        let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .expect("build readiness request"),
            )
            .await
            .expect("call readiness route");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(fs::read(&old_database).unwrap(), sentinel);
        assert!(paths
            .subscription_state_dir
            .join("subscriptions.sqlite")
            .is_file());

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn readiness_rejects_latest_manifest_drift_and_is_cached() {
        let root = temp_test_root("manifest-drift");
        let paths = AppPaths::for_test_root(&root);
        let app_state = AppState::for_test(paths.clone());
        let cached_state = HealthState::new(app_state.clone());

        assert_eq!(
            readyz(State(cached_state.clone())).await.status(),
            StatusCode::OK
        );
        assert_eq!(
            readyz(State(cached_state.clone())).await.status(),
            StatusCode::OK
        );
        assert_eq!(cached_state.readiness.completed_checks(), 1);

        let database = paths.subscription_state_dir.join("subscriptions.sqlite");
        let connection = Connection::open(&database).expect("open latest readiness fixture");
        connection
            .execute("DROP INDEX wanted_records_list_v5_idx", [])
            .expect("drift latest schema manifest");
        drop(connection);
        let drifted_state = HealthState::new(app_state);
        let response = readyz(State(drifted_state.clone())).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body_json(response).await["code"], "not_ready");
        assert_eq!(drifted_state.readiness.completed_checks(), 1);

        let _ = fs::remove_dir_all(root);
    }
}
