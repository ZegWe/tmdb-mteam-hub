use std::path::PathBuf;

use axum::middleware;
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

use crate::app::AppState;
use crate::http::{
    auth, config, douban, error, health, media, mteam, operation_logs, qb, security,
    subscription_queries, subscriptions,
};

pub fn build_api_router(state: AppState) -> Router {
    let protected = Router::new()
        .merge(config::routes())
        .merge(douban::routes())
        .merge(media::routes())
        .merge(mteam::routes())
        .merge(operation_logs::routes())
        .merge(qb::routes())
        .merge(subscription_queries::routes())
        .merge(subscriptions::routes())
        .method_not_allowed_fallback(error::method_not_allowed)
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_management_auth,
        ));

    Router::new()
        .merge(auth::routes())
        .merge(protected)
        .fallback(error::not_found)
        .with_state(state)
}

pub fn build_router(state: AppState, static_dir: PathBuf) -> Router {
    let management = state.startup_management.clone();
    let api = security::apply_cors(build_api_router(state.clone()), &management);
    let health = health::routes(state);
    let static_service =
        ServeDir::new(&static_dir).fallback(ServeFile::new(static_dir.join("index.html")));

    Router::new()
        .merge(health)
        .nest("/api", api)
        .fallback_service(static_service)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::Body;
    use axum::extract::connect_info::ConnectInfo;
    use axum::http::{header, Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use super::{build_api_router, build_router};
    use crate::app::douban_catalog::{
        DoubanCatalogError, DoubanCatalogProvider, DoubanCatalogService, DoubanInterestResult,
        DoubanLibraryList, DoubanQrPollProviderOutcome, DoubanQrStartProviderOutcome, DoubanRating,
        DoubanSearchItem, DoubanSearchOutcome, DoubanSubjectDetail, MarkDoubanInterestCommand,
        ProviderFuture as DoubanProviderFuture,
    };
    use crate::app::manual_qb::{ManualQbPort, ManualQbPortFuture, ManualQbService};
    use crate::app::mteam_search::{
        MteamSearchProvider, MteamSearchService, ProviderFuture as MteamProviderFuture,
    };
    use crate::app::{AppPaths, AppState};
    use crate::clients::http::ClientError;
    use crate::config::{FileConfig, QbServerEntry};
    use crate::subscription::ports::SubscriptionPollRepository;
    use crate::subscription::repository::payload::WantedSourcePayload;
    use crate::subscription::repository::{
        ApplyCompleteSnapshotCommand, BeginPollCommand, NewRecordPolicy, SnapshotRecord,
    };
    use crate::subscription::SubscriptionMediaKind;

    struct FakeDoubanSearchProvider {
        results: Mutex<Vec<Result<DoubanSearchOutcome, DoubanCatalogError>>>,
    }

    impl FakeDoubanSearchProvider {
        fn new(results: Vec<Result<DoubanSearchOutcome, DoubanCatalogError>>) -> Self {
            Self {
                results: Mutex::new(results),
            }
        }

        fn unused<T>() -> DoubanProviderFuture<T> {
            Box::pin(async {
                Err(DoubanCatalogError::Upstream {
                    message: "unused fake Douban provider method".to_string(),
                })
            })
        }
    }

    impl DoubanCatalogProvider for FakeDoubanSearchProvider {
        fn search(
            &self,
            _cookie: String,
            _query: String,
            _page: usize,
            _page_size: usize,
        ) -> DoubanProviderFuture<DoubanSearchOutcome> {
            let result = self.results.lock().expect("fake Douban lock").remove(0);
            Box::pin(async move { result })
        }

        fn subject_detail(
            &self,
            _cookie: String,
            _subject_id: String,
        ) -> DoubanProviderFuture<DoubanSubjectDetail> {
            Self::unused()
        }

        fn mark_interest(
            &self,
            _cookie: String,
            _command: MarkDoubanInterestCommand,
        ) -> DoubanProviderFuture<DoubanInterestResult> {
            Self::unused()
        }

        fn library(
            &self,
            _cookie: String,
            _status: crate::douban::DoubanLibraryStatus,
            _limit: usize,
        ) -> DoubanProviderFuture<DoubanLibraryList> {
            Self::unused()
        }

        fn qr_start(&self) -> DoubanProviderFuture<DoubanQrStartProviderOutcome> {
            Self::unused()
        }

        fn qr_poll(
            &self,
            _session: crate::douban::QrSession,
        ) -> DoubanProviderFuture<DoubanQrPollProviderOutcome> {
            Self::unused()
        }
    }

    struct FakeMteamSearchProvider {
        results: Mutex<Vec<Result<Value, ClientError>>>,
    }

    impl FakeMteamSearchProvider {
        fn new(results: Vec<Result<Value, ClientError>>) -> Self {
            Self {
                results: Mutex::new(results),
            }
        }
    }

    impl MteamSearchProvider for FakeMteamSearchProvider {
        fn search(&self, _api_key: String, _body: Value) -> MteamProviderFuture {
            let result = self.results.lock().expect("fake M-Team lock").remove(0);
            Box::pin(async move { result })
        }
    }

    struct FakeManualQbPort {
        test_results: Mutex<Vec<Result<String, ClientError>>>,
    }

    impl FakeManualQbPort {
        fn new(test_results: Vec<Result<String, ClientError>>) -> Self {
            Self {
                test_results: Mutex::new(test_results),
            }
        }
    }

    impl ManualQbPort for FakeManualQbPort {
        fn test_connection(&self, _server: QbServerEntry) -> ManualQbPortFuture<String> {
            let result = self.test_results.lock().expect("fake qB lock").remove(0);
            Box::pin(async move { result })
        }

        fn fetch_mteam_download_url(
            &self,
            _api_key: String,
            _torrent_id: String,
        ) -> ManualQbPortFuture<String> {
            Box::pin(async {
                Err(ClientError::unavailable(
                    "M-Team",
                    "unused fake qB port method",
                ))
            })
        }

        fn add_torrent_from_url(
            &self,
            _server: QbServerEntry,
            _download_url: String,
            _category: Option<String>,
            _savepath: Option<String>,
        ) -> ManualQbPortFuture<()> {
            Box::pin(async {
                Err(ClientError::unavailable(
                    "qBittorrent",
                    "unused fake qB port method",
                ))
            })
        }
    }

    fn temp_test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-router-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create router test root");
        root
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect router response body")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("parse router response JSON")
    }

    fn loopback_peer() -> ConnectInfo<SocketAddr> {
        ConnectInfo(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 42000))
    }

    #[tokio::test]
    async fn production_douban_router_has_stable_success_and_failure_json_shapes() {
        let root = temp_test_root("douban-provider-contract");
        let paths = AppPaths::for_test_root(&root);
        let config = FileConfig {
            douban_cookie: "dbcl2=router-fixture:secret; ck=test".to_string(),
            ..FileConfig::default()
        };
        let mut state = AppState::for_test_with_config(paths, config);
        let provider = Arc::new(FakeDoubanSearchProvider::new(vec![
            Ok(DoubanSearchOutcome {
                items: vec![DoubanSearchItem {
                    source: "douban".to_string(),
                    media_type: "movie".to_string(),
                    id: "1295644".to_string(),
                    subject_id: "1295644".to_string(),
                    title: "Fixture Movie".to_string(),
                    url: "https://movie.douban.com/subject/1295644/".to_string(),
                    abstract_text: "2026 / China".to_string(),
                    abstract_2: "Fixture abstract".to_string(),
                    cover_url: "https://images.test/fixture.jpg".to_string(),
                    poster_url: "/douban/image?fixture=1".to_string(),
                    rating: DoubanRating {
                        value: Some(8.8),
                        count: Some(1234),
                        info: "8.8".to_string(),
                        star_count: Some(4.5),
                    },
                    vote_average: Some(8.8),
                }],
                page: 2,
                page_size: 10,
                has_more: true,
            }),
            Err(DoubanCatalogError::Upstream {
                message: "SECRET_DOUBAN_UPSTREAM_DETAIL".to_string(),
            }),
        ]));
        state.douban_catalog = DoubanCatalogService::with_provider(
            state.config.clone(),
            provider,
            state.douban_cache.clone(),
            60,
            state.audit_log.clone(),
        );
        let app = build_api_router(state);

        let success = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/douban/search?q=fixture&page=2&page_size=10")
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build Douban success request"),
            )
            .await
            .expect("call Douban success route");
        assert_eq!(success.status(), StatusCode::OK);
        assert_eq!(
            body_json(success).await,
            json!({
                "items": [{
                    "source": "douban",
                    "media_type": "movie",
                    "id": "1295644",
                    "subject_id": "1295644",
                    "title": "Fixture Movie",
                    "url": "https://movie.douban.com/subject/1295644/",
                    "abstract_text": "2026 / China",
                    "abstract_2": "Fixture abstract",
                    "cover_url": "https://images.test/fixture.jpg",
                    "poster_url": "/douban/image?fixture=1",
                    "rating": {
                        "value": 8.8,
                        "count": 1234,
                        "info": "8.8",
                        "star_count": 4.5
                    },
                    "vote_average": 8.8
                }],
                "page": 2,
                "page_size": 10,
                "has_more": true
            })
        );

        let failure = app
            .oneshot(
                Request::builder()
                    .uri("/douban/search?q=failure")
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build Douban failure request"),
            )
            .await
            .expect("call Douban failure route");
        assert_eq!(failure.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            body_json(failure).await,
            json!({
                "code": "upstream_error",
                "message": "upstream service request failed"
            })
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn production_mteam_router_has_stable_success_and_failure_json_shapes() {
        let root = temp_test_root("mteam-provider-contract");
        let paths = AppPaths::for_test_root(&root);
        let config = FileConfig {
            mteam_api_key: "fixture-key".to_string(),
            ..FileConfig::default()
        };
        let mut state = AppState::for_test_with_config(paths, config);
        let provider = Arc::new(FakeMteamSearchProvider::new(vec![
            Ok(json!({
                "data": {
                    "items": [{
                        "torrentId": 42,
                        "title": "Fixture.Release.2160p",
                        "smallDescr": "UHD",
                        "size": "4096",
                        "status": { "seeders": "8", "leechers": 2 },
                        "createdDate": "2026-07-12"
                    }]
                }
            })),
            Err(ClientError::unavailable(
                "M-Team",
                "SECRET_MTEAM_UPSTREAM_DETAIL",
            )),
        ]));
        state.mteam_search = MteamSearchService::with_provider(
            state.config.clone(),
            provider,
            state.audit_log.clone(),
        );
        let app = build_api_router(state);

        let success = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/mteam/torrents?source=keyword&keyword=fixture&page=2&page_size=10")
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build M-Team success request"),
            )
            .await
            .expect("call M-Team success route");
        assert_eq!(success.status(), StatusCode::OK);
        assert_eq!(
            body_json(success).await,
            json!({
                "items": [{
                    "id": "42",
                    "name": "Fixture.Release.2160p",
                    "small_description": "UHD",
                    "size": 4096,
                    "seeders": 8,
                    "leechers": 2,
                    "created_at": "2026-07-12"
                }],
                "page": 2,
                "page_size": 10
            })
        );

        let failure = app
            .oneshot(
                Request::builder()
                    .uri("/mteam/torrents?source=keyword&keyword=failure")
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build M-Team failure request"),
            )
            .await
            .expect("call M-Team failure route");
        assert_eq!(failure.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            body_json(failure).await,
            json!({
                "code": "upstream_error",
                "message": "upstream service request failed"
            })
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn production_qb_router_has_stable_success_and_failure_json_shapes() {
        let root = temp_test_root("qb-provider-contract");
        let paths = AppPaths::for_test_root(&root);
        let config = FileConfig {
            qb_servers: vec![QbServerEntry {
                id: "nas".to_string(),
                name: "NAS".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "admin".to_string(),
                password: "test-only".to_string(),
                insecure_tls: false,
            }],
            ..FileConfig::default()
        };
        let mut state = AppState::for_test_with_config(paths, config);
        let port = Arc::new(FakeManualQbPort::new(vec![
            Ok("5.0.4".to_string()),
            Err(ClientError::unavailable(
                "qBittorrent",
                "SECRET_QB_UPSTREAM_DETAIL",
            )),
        ]));
        state.manual_qb =
            ManualQbService::with_port(state.config.clone(), port, state.audit_log.clone());
        let app = build_api_router(state);

        let success = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/qb/test")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(loopback_peer())
                    .body(Body::from(r#"{"server_id":"nas"}"#))
                    .expect("build qB success request"),
            )
            .await
            .expect("call qB success route");
        assert_eq!(success.status(), StatusCode::OK);
        assert_eq!(
            body_json(success).await,
            json!({ "ok": true, "version": "5.0.4" })
        );

        let failure = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/qb/test")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(loopback_peer())
                    .body(Body::from(r#"{"server_id":"nas"}"#))
                    .expect("build qB failure request"),
            )
            .await
            .expect("call qB failure route");
        assert_eq!(failure.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(
            body_json(failure).await,
            json!({
                "code": "upstream_error",
                "message": "upstream service request failed"
            })
        );

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn unknown_api_path_returns_json_not_spa_fallback() {
        let root = temp_test_root("api-not-found");
        let paths = AppPaths::for_test_root(&root);
        fs::create_dir_all(&paths.static_dir).expect("create static test directory");
        fs::write(paths.static_dir.join("index.html"), "SPA_INDEX_SENTINEL")
            .expect("write static test index");
        let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/does-not-exist")
                    .body(Body::empty())
                    .expect("build unknown API request"),
            )
            .await
            .expect("call unknown API route");

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body = body_json(response).await;
        assert_eq!(body["code"], "not_found");
        assert_eq!(body["message"], "API endpoint not found");
        assert!(body.get("error").is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn known_api_path_with_wrong_method_returns_json_405() {
        let root = temp_test_root("method-not-allowed");
        let paths = AppPaths::for_test_root(&root);
        let app = build_api_router(AppState::for_test(paths));

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/auth/status")
                    .body(Body::empty())
                    .expect("build wrong-method request"),
            )
            .await
            .expect("call wrong-method API route");

        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        let body = body_json(response).await;
        assert_eq!(body["code"], "method_not_allowed");
        assert_eq!(body["message"], "HTTP method not allowed for this endpoint");
        assert!(body.get("error").is_none());

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn protected_wrong_method_still_requires_management_auth() {
        let root = temp_test_root("protected-method-auth");
        let paths = AppPaths::for_test_root(&root);
        let app = build_api_router(AppState::for_test(paths));

        let unauthenticated = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/config")
                    .body(Body::empty())
                    .expect("build unauthenticated wrong-method request"),
            )
            .await
            .expect("call unauthenticated wrong-method route");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(body_json(unauthenticated).await["code"], "unauthorized");

        let authenticated = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/config")
                    .extension(ConnectInfo(SocketAddr::new(
                        IpAddr::V4(Ipv4Addr::LOCALHOST),
                        42000,
                    )))
                    .body(Body::empty())
                    .expect("build authenticated wrong-method request"),
            )
            .await
            .expect("call authenticated wrong-method route");
        assert_eq!(authenticated.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body_json(authenticated).await["code"], "method_not_allowed");

        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn production_subscription_list_and_detail_read_the_latest_repository() {
        let root = temp_test_root("latest-subscription-query");
        let paths = AppPaths::for_test_root(&root);
        fs::create_dir_all(&paths.subscription_state_dir).unwrap();
        let old_database = paths.subscription_state_dir.join("wanted.sqlite");
        let old_sentinel = b"OLD_QUERY_DATABASE_MUST_NOT_CHANGE";
        fs::write(&old_database, old_sentinel).unwrap();
        let config = FileConfig {
            douban_cookie: "dbcl2=account-1:secret; ck=test".to_string(),
            ..FileConfig::default()
        };
        let state = AppState::for_test_with_config(paths.clone(), config.clone());
        let account_key = crate::douban::auth_cache_key_fragment(&config.douban_cookie).unwrap();
        let begin = state
            .subscription_repository
            .begin_poll(BeginPollCommand::try_new(&account_key, 100).unwrap())
            .await
            .unwrap();
        state
            .subscription_repository
            .apply_complete_snapshot(
                ApplyCompleteSnapshotCommand::try_new(
                    &account_key,
                    begin.token,
                    101,
                    3_701,
                    NewRecordPolicy::try_new(3, false).unwrap(),
                    vec![SnapshotRecord::try_new(
                        "subject-1",
                        SubscriptionMediaKind::Movie,
                        true,
                        None,
                        WantedSourcePayload {
                            title: "Latest Fixture".to_string(),
                            release_year: Some(2026),
                            poster_url: "https://images.test/poster.jpg".to_string(),
                            category_text: Some("电影".to_string()),
                            douban_sort_time: Some(20260711),
                            ..WantedSourcePayload::default()
                        },
                    )
                    .unwrap()],
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let app = build_api_router(state);
        let peer = ConnectInfo(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 42000));

        let list = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/subscriptions/wanted")
                    .extension(peer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list.status(), StatusCode::OK);
        let list = body_json(list).await;
        assert_eq!(list["items"][0]["subject_id"], "subject-1");
        assert_eq!(list["items"][0]["title"], "Latest Fixture");

        let detail = app
            .oneshot(
                Request::builder()
                    .uri("/subscriptions/wanted/subject-1")
                    .extension(peer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(detail.status(), StatusCode::OK);
        let detail = body_json(detail).await;
        assert_eq!(detail["summary"]["subject_id"], "subject-1");
        assert_eq!(detail["summary"]["title"], "Latest Fixture");
        assert_eq!(detail["source"]["cover_url"], "");
        assert_eq!(fs::read(&old_database).unwrap(), old_sentinel);

        let _ = fs::remove_dir_all(root);
    }
}
