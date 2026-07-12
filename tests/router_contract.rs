use std::fs;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::connect_info::ConnectInfo;
use axum::http::{header, Method, Request, StatusCode};
use http_body_util::BodyExt;
use tmdb_mteam_server::app::{AppPaths, AppState};
use tmdb_mteam_server::build_router;
use tower::ServiceExt;

fn temp_test_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "tmdb-mteam-router-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&root).expect("create router contract test root");
    root
}

async fn body_text(response: axum::response::Response) -> String {
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect response body")
        .to_bytes();
    String::from_utf8(bytes.to_vec()).expect("response body should be utf-8")
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_str(&body_text(response).await).expect("response body should be JSON")
}

fn loopback_peer() -> ConnectInfo<SocketAddr> {
    ConnectInfo(SocketAddr::from((Ipv4Addr::LOCALHOST, 42000)))
}

#[tokio::test]
async fn api_errors_use_flat_stable_envelopes_for_extractors_and_handlers() {
    let root = temp_test_root("error-envelope");
    let paths = AppPaths::for_test_root(&root);
    let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

    let malformed_json = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/qb/test")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(r#"{"server_id":"SECRET_JSON_VALUE","#))
                .expect("build malformed JSON request"),
        )
        .await
        .expect("call malformed JSON route");
    assert_eq!(malformed_json.status(), StatusCode::BAD_REQUEST);
    assert_eq!(malformed_json.headers()[header::CACHE_CONTROL], "no-store");
    let malformed_json = body_json(malformed_json).await;
    assert_eq!(malformed_json["code"], "invalid_json");
    assert_eq!(
        malformed_json["message"],
        "request body contains invalid JSON"
    );
    assert!(malformed_json.get("error").is_none());
    assert!(!malformed_json.to_string().contains("SECRET_JSON_VALUE"));

    let invalid_query = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/douban/search?q=test&page=SECRET_QUERY_VALUE")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build invalid query request"),
        )
        .await
        .expect("call invalid query route");
    assert_eq!(invalid_query.status(), StatusCode::BAD_REQUEST);
    let invalid_query = body_json(invalid_query).await;
    assert_eq!(invalid_query["code"], "invalid_query");
    assert_eq!(invalid_query["message"], "query parameters are invalid");
    assert!(invalid_query.get("error").is_none());
    assert!(!invalid_query.to_string().contains("SECRET_QUERY_VALUE"));

    let invalid_path = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/tmdb/movie/SECRET_PATH_VALUE")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build invalid path request"),
        )
        .await
        .expect("call invalid path route");
    assert_eq!(invalid_path.status(), StatusCode::BAD_REQUEST);
    let invalid_path = body_json(invalid_path).await;
    assert_eq!(invalid_path["code"], "invalid_path");
    assert_eq!(invalid_path["message"], "path parameters are invalid");
    assert!(invalid_path.get("error").is_none());
    assert!(!invalid_path.to_string().contains("SECRET_PATH_VALUE"));

    let semantic_error = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/qb/test")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(r#"{"server_id":"missing"}"#))
                .expect("build semantic error request"),
        )
        .await
        .expect("call semantic error route");
    assert_eq!(semantic_error.status(), StatusCode::BAD_REQUEST);
    assert_eq!(semantic_error.headers()[header::CACHE_CONTROL], "no-store");
    let semantic_error = body_json(semantic_error).await;
    assert_eq!(semantic_error["code"], "bad_request");
    assert!(semantic_error["message"]
        .as_str()
        .is_some_and(|message| message.contains("server_id")));
    assert!(semantic_error.get("error").is_none());

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn tmdb_detail_routes_return_closed_normalized_dtos_from_the_real_router() {
    let root = temp_test_root("tmdb-detail-dto");
    let paths = AppPaths::for_test_root(&root);
    fs::create_dir_all(&paths.tmdb_cache_dir).expect("create TMDB cache fixture directory");
    fs::write(
        paths.tmdb_cache_dir.join("movie_42.json"),
        serde_json::json!({
            "id": 42,
            "title": "电影",
            "original_title": "Movie",
            "overview": "Movie overview",
            "poster_path": "/movie.jpg",
            "external_ids": { "imdb_id": "tt0042" },
            "runtime": 123
        })
        .to_string(),
    )
    .expect("write movie cache fixture");
    fs::write(
        paths.tmdb_cache_dir.join("tv_84.json"),
        serde_json::json!({
            "id": 84,
            "name": "剧集",
            "original_name": "Series",
            "overview": "Series overview",
            "type": "Scripted",
            "seasons": [{ "season_number": 1, "name": "第一季", "episode_count": 8 }]
        })
        .to_string(),
    )
    .expect("write TV cache fixture");
    fs::write(
        paths.tmdb_cache_dir.join("tv_84_s1.json"),
        serde_json::json!({
            "id": 841,
            "season_number": 1,
            "name": "第一季",
            "episodes": [{
                "id": 84101,
                "episode_number": 1,
                "name": "第一集",
                "still_path": "/episode.jpg"
            }]
        })
        .to_string(),
    )
    .expect("write season cache fixture");
    let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

    let config = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": 1,
                        "tmdb_api_key": "test-only-tmdb-key"
                    })
                    .to_string(),
                ))
                .expect("build config update request"),
        )
        .await
        .expect("update test TMDB credential");
    assert_eq!(config.status(), StatusCode::OK);

    let movie = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/tmdb/movie/42")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build movie detail request"),
        )
        .await
        .expect("call movie detail route");
    assert_eq!(movie.status(), StatusCode::OK);
    let movie = body_json(movie).await;
    assert_eq!(movie["media_type"], "movie");
    assert_eq!(movie["title"], "电影");
    assert_eq!(movie["original_title"], "Movie");
    assert_eq!(movie["imdb_id"], "tt0042");
    assert_eq!(
        movie["poster_url"],
        "https://image.tmdb.org/t/p/w500/movie.jpg"
    );
    assert!(movie.get("external_ids").is_none());

    let tv = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/tmdb/tv/84")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build TV detail request"),
        )
        .await
        .expect("call TV detail route");
    assert_eq!(tv.status(), StatusCode::OK);
    let tv = body_json(tv).await;
    assert_eq!(tv["media_type"], "tv");
    assert_eq!(tv["title"], "剧集");
    assert_eq!(tv["original_title"], "Series");
    assert_eq!(tv["series_type"], "Scripted");
    assert!(tv.get("name").is_none());
    assert!(tv.get("original_name").is_none());
    assert!(tv.get("type").is_none());

    let season = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/tmdb/tv/84/season/1")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build season detail request"),
        )
        .await
        .expect("call season detail route");
    assert_eq!(season.status(), StatusCode::OK);
    let season = body_json(season).await;
    assert_eq!(season["season_number"], 1);
    assert_eq!(season["episodes"][0]["episode_number"], 1);
    assert_eq!(
        season["episodes"][0]["still_url"],
        "https://image.tmdb.org/t/p/w185/episode.jpg"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn removed_subscription_effect_routes_are_absent_from_production() {
    let root = temp_test_root("subscription-effects-absent");
    let paths = AppPaths::for_test_root(&root);
    let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

    for path in [
        "/api/subscriptions/wanted/example/candidates",
        "/api/subscriptions/wanted/example/push",
        "/api/subscriptions/wanted/example/retry-current",
        "/api/subscriptions/wanted/example/rerun",
        "/api/subscriptions/wanted/example/completion",
        "/api/subscriptions/wanted/example/progress",
    ] {
        for method in [Method::GET, Method::POST] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method.clone())
                        .uri(path)
                        .extension(loopback_peer())
                        .body(Body::empty())
                        .expect("build removed subscription effect request"),
                )
                .await
                .expect("call removed subscription effect route");
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "removed route must stay absent: {method} {path}"
            );
            assert_eq!(body_json(response).await["code"], "not_found");
        }
    }

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn production_router_contract_covers_api_405_and_spa_fallback() {
    let root = temp_test_root("baseline");
    let paths = AppPaths::for_test_root(&root);
    fs::create_dir_all(&paths.static_dir).expect("create isolated static directory");
    fs::write(
        paths.static_dir.join("index.html"),
        include_str!("fixtures/http/index.html"),
    )
    .expect("write isolated SPA fixture");
    let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

    assert!(!paths.config_path.exists());
    assert!(!paths.tmdb_cache_dir.exists());
    assert!(!paths.douban_cache_dir.exists());
    assert!(paths.subscription_state_dir.is_dir());
    assert!(paths
        .subscription_state_dir
        .join("subscriptions.sqlite")
        .is_file());
    assert!(!paths.subscription_state_dir.join("wanted.sqlite").exists());

    let api_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/config")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build API request"),
        )
        .await
        .expect("call API route");
    assert_eq!(api_response.status(), StatusCode::OK);
    let api_json: serde_json::Value =
        serde_json::from_str(&body_text(api_response).await).expect("parse config response JSON");
    assert_eq!(api_json["listen_ip"], "127.0.0.1");
    assert_eq!(api_json["listen_port"], 8787);
    assert_eq!(api_json["revision"], 1);

    let method_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/config")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build method-not-allowed request"),
        )
        .await
        .expect("call method-not-allowed route");
    assert_eq!(method_response.status(), StatusCode::METHOD_NOT_ALLOWED);

    let removed_status_route = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/subscriptions/wanted/example/status")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build removed status-route request"),
        )
        .await
        .expect("call removed status route");
    assert_eq!(removed_status_route.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(removed_status_route).await["code"], "not_found");

    let spa_response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/subscriptions/deep-link")
                .body(Body::empty())
                .expect("build SPA fallback request"),
        )
        .await
        .expect("call SPA fallback");
    assert_eq!(spa_response.status(), StatusCode::OK);
    assert!(body_text(spa_response)
        .await
        .contains("router-contract-spa"));

    assert!(!paths.config_path.exists());
    assert!(!paths.tmdb_cache_dir.exists());
    assert!(!paths.douban_cache_dir.exists());
    assert!(paths.subscription_state_dir.is_dir());
    assert!(paths
        .subscription_state_dir
        .join("subscriptions.sqlite")
        .is_file());
    assert!(!paths.subscription_state_dir.join("wanted.sqlite").exists());
}

#[tokio::test]
async fn config_api_redacts_secrets_and_requires_revision() {
    const TMDB_SECRET: &str = "SECRET_MUST_NOT_LEAK_TMDB";
    const MTEAM_SECRET: &str = "SECRET_MUST_NOT_LEAK_MTEAM";
    const DOUBAN_SECRET: &str = "dbcl2=SECRET_MUST_NOT_LEAK_DOUBAN:token; ck=test";
    const ADMIN_SECRET: &str = "SECRET_MUST_NOT_LEAK_ADMIN_TOKEN_123456";
    const QB_SECRET: &str = "SECRET_MUST_NOT_LEAK_QB_PASSWORD";
    const STALE_SECRET: &str = "SECRET_MUST_NOT_COMMIT_STALE";

    let root = temp_test_root("config-revision");
    let paths = AppPaths::for_test_root(&root);
    fs::create_dir_all(&paths.static_dir).expect("create isolated static directory");
    fs::write(
        paths.static_dir.join("index.html"),
        include_str!("fixtures/http/index.html"),
    )
    .expect("write isolated SPA fixture");
    fs::create_dir_all(&paths.subscription_state_dir)
        .expect("create isolated subscription state directory");
    let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

    let initial = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/config")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build initial config request"),
        )
        .await
        .expect("call initial config request");
    assert_eq!(initial.status(), StatusCode::OK);
    let initial_json: serde_json::Value =
        serde_json::from_str(&body_text(initial).await).expect("parse initial config JSON");
    assert_eq!(initial_json["revision"], 1);
    assert_eq!(initial_json["has_tmdb_api_key"], false);
    assert_eq!(initial_json["has_admin_token"], false);
    assert_eq!(initial_json["restart_required"], false);

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": 1,
                        "tmdb_api_key": TMDB_SECRET,
                        "mteam_api_key": MTEAM_SECRET,
                        "douban_cookie": DOUBAN_SECRET,
                        "admin_token": ADMIN_SECRET,
                        "allowed_origins": ["https://admin.example"],
                        "qb_servers": [{
                            "id": "nas",
                            "name": "NAS",
                            "base_url": "http://127.0.0.1:8080",
                            "username": "admin",
                            "password": QB_SECRET,
                            "insecure_tls": false
                        }]
                    })
                    .to_string(),
                ))
                .expect("build first config update"),
        )
        .await
        .expect("call first config update");
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["x-config-revision"], "2");
    let first_text = body_text(first).await;
    for secret in [
        TMDB_SECRET,
        MTEAM_SECRET,
        DOUBAN_SECRET,
        ADMIN_SECRET,
        QB_SECRET,
    ] {
        assert!(!first_text.contains(secret), "PUT response leaked {secret}");
    }
    let first_json: serde_json::Value =
        serde_json::from_str(&first_text).expect("parse first config update JSON");
    assert_eq!(first_json["revision"], 2);
    assert_eq!(first_json["has_tmdb_api_key"], true);
    assert_eq!(first_json["has_mteam_api_key"], true);
    assert_eq!(first_json["has_douban_cookie"], true);
    assert_eq!(first_json["has_admin_token"], true);
    assert_eq!(first_json["qb_servers"][0]["has_password"], true);
    assert_eq!(first_json["restart_required"], true);
    assert!(first_json["qb_servers"][0].get("password").is_none());

    let compatible = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {ADMIN_SECRET}"))
                .extension(loopback_peer())
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": 2,
                        "listen_port": 9898,
                        "qb_servers": [{
                            "id": "nas",
                            "name": "Renamed NAS",
                            "base_url": "http://127.0.0.1:9090",
                            "username": "operator",
                            "insecure_tls": true
                        }]
                    })
                    .to_string(),
                ))
                .expect("build compatible config update"),
        )
        .await
        .expect("call compatible config update");
    assert_eq!(compatible.status(), StatusCode::OK);
    assert_eq!(compatible.headers()["x-config-revision"], "3");
    let compatible_json: serde_json::Value = serde_json::from_str(&body_text(compatible).await)
        .expect("parse compatible config update JSON");
    assert_eq!(compatible_json["revision"], 3);
    assert_eq!(compatible_json["has_tmdb_api_key"], true);
    assert_eq!(compatible_json["has_douban_cookie"], true);
    assert_eq!(compatible_json["qb_servers"][0]["has_password"], true);
    assert_eq!(compatible_json["restart_required"], true);

    let missing_revision = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {ADMIN_SECRET}"))
                .extension(loopback_peer())
                .body(Body::from("{}"))
                .expect("build missing-revision config update"),
        )
        .await
        .expect("call missing-revision config update");
    assert_eq!(missing_revision.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let stale = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {ADMIN_SECRET}"))
                .extension(loopback_peer())
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": 2,
                        "tmdb_api_key": STALE_SECRET
                    })
                    .to_string(),
                ))
                .expect("build stale config update"),
        )
        .await
        .expect("call stale config update");
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let stale_body = body_text(stale).await;
    assert!(stale_body.contains("expected=2"));
    assert!(stale_body.contains("current=3"));
    assert!(!stale_body.contains(STALE_SECRET));

    let userinfo_secret = "SECRET_MUST_NOT_LEAK_URL_USERINFO";
    let userinfo = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {ADMIN_SECRET}"))
                .extension(loopback_peer())
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": 3,
                        "qb_servers": [{
                            "id": "nas",
                            "name": "NAS",
                            "base_url": format!("http://user:{userinfo_secret}@127.0.0.1:8080"),
                            "username": "admin"
                        }]
                    })
                    .to_string(),
                ))
                .expect("build userinfo config update"),
        )
        .await
        .expect("call userinfo config update");
    assert_eq!(userinfo.status(), StatusCode::BAD_REQUEST);
    let userinfo_body = body_text(userinfo).await;
    assert!(userinfo_body.contains("userinfo"));
    assert!(!userinfo_body.contains(userinfo_secret));

    let current = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/config")
                .header(header::AUTHORIZATION, format!("Bearer {ADMIN_SECRET}"))
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build current config request"),
        )
        .await
        .expect("call current config request");
    assert_eq!(current.status(), StatusCode::OK);
    let current_json: serde_json::Value =
        serde_json::from_str(&body_text(current).await).expect("parse current config JSON");
    assert_eq!(current_json["revision"], 3);
    assert_eq!(current_json["has_tmdb_api_key"], true);
    assert_eq!(current_json["has_mteam_api_key"], true);
    assert_eq!(current_json["has_douban_cookie"], true);
    assert_eq!(current_json["has_admin_token"], true);
    assert_eq!(current_json["qb_servers"][0]["has_password"], true);
    assert_eq!(current_json["restart_required"], false);
    assert!(current_json.get("tmdb_api_key").is_none());
    assert!(current_json.get("mteam_api_key").is_none());
    assert!(current_json.get("douban_cookie").is_none());
    assert!(current_json.get("admin_token").is_none());
    assert!(current_json["qb_servers"][0].get("password").is_none());

    let logs = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/operation-logs")
                .header(header::AUTHORIZATION, format!("Bearer {ADMIN_SECRET}"))
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build operation log request"),
        )
        .await
        .expect("call operation log route");
    assert_eq!(logs.status(), StatusCode::OK);
    let logs_text = body_text(logs).await;
    for secret in [
        TMDB_SECRET,
        MTEAM_SECRET,
        DOUBAN_SECRET,
        ADMIN_SECRET,
        QB_SECRET,
        STALE_SECRET,
        userinfo_secret,
    ] {
        assert!(!logs_text.contains(secret), "operation log leaked {secret}");
    }

    let persisted = fs::read_to_string(&paths.config_path).expect("read persisted config TOML");
    assert!(persisted.contains(TMDB_SECRET));
    assert!(persisted.contains(QB_SECRET));
    assert!(!persisted.contains(STALE_SECRET));

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn config_api_requires_confirmation_only_when_enabling_automation() {
    let root = temp_test_root("automation-confirmation");
    let paths = AppPaths::for_test_root(&root);
    fs::create_dir_all(&paths.static_dir).expect("create isolated static directory");
    fs::write(
        paths.static_dir.join("index.html"),
        include_str!("fixtures/http/index.html"),
    )
    .expect("write isolated SPA fixture");
    fs::create_dir_all(&paths.subscription_state_dir)
        .expect("create isolated subscription state directory");
    let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

    let initial = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/config")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build initial config request"),
        )
        .await
        .expect("call initial config request");
    assert_eq!(initial.status(), StatusCode::OK);
    let initial_json: serde_json::Value =
        serde_json::from_str(&body_text(initial).await).expect("parse initial config JSON");
    assert_eq!(initial_json["revision"], 1);
    assert_eq!(initial_json["subscription_watcher"]["enabled"], false);
    assert_eq!(initial_json["subscription_watcher"]["dry_run"], true);
    assert!(!paths.config_path.exists());

    let rejected = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": 1,
                        "subscription_watcher": {
                            "enabled": true
                        }
                    })
                    .to_string(),
                ))
                .expect("build unconfirmed enable request"),
        )
        .await
        .expect("call unconfirmed enable request");
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);
    assert!(body_text(rejected)
        .await
        .contains("confirm_enable_automation=true"));
    assert!(!paths.config_path.exists());

    let unchanged = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/config")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build unchanged config request"),
        )
        .await
        .expect("call unchanged config request");
    let unchanged_json: serde_json::Value =
        serde_json::from_str(&body_text(unchanged).await).expect("parse unchanged config JSON");
    assert_eq!(unchanged_json["revision"], 1);
    assert_eq!(unchanged_json["subscription_watcher"]["enabled"], false);

    let enabled = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": 1,
                        "confirm_enable_automation": true,
                        "subscription_watcher": {
                            "enabled": true
                        }
                    })
                    .to_string(),
                ))
                .expect("build confirmed enable request"),
        )
        .await
        .expect("call confirmed enable request");
    assert_eq!(enabled.status(), StatusCode::OK);
    assert_eq!(enabled.headers()["x-config-revision"], "2");
    let enabled_json: serde_json::Value =
        serde_json::from_str(&body_text(enabled).await).expect("parse enabled config JSON");
    assert_eq!(enabled_json["subscription_watcher"]["enabled"], true);
    assert_eq!(enabled_json["subscription_watcher"]["dry_run"], true);

    let persisted = fs::read_to_string(&paths.config_path).expect("read enabled config TOML");
    assert!(persisted.contains("enabled = true"));
    assert!(persisted.contains("dry_run = true"));
    assert!(!persisted.contains("confirm_enable_automation"));

    let still_enabled = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": 2,
                        "subscription_watcher": {
                            "enabled": true,
                            "dry_run": false
                        }
                    })
                    .to_string(),
                ))
                .expect("build already-enabled update request"),
        )
        .await
        .expect("call already-enabled update request");
    assert_eq!(still_enabled.status(), StatusCode::OK);
    assert_eq!(still_enabled.headers()["x-config-revision"], "3");
    let still_enabled_json: serde_json::Value =
        serde_json::from_str(&body_text(still_enabled).await)
            .expect("parse already-enabled config JSON");
    assert_eq!(still_enabled_json["subscription_watcher"]["enabled"], true);
    assert_eq!(still_enabled_json["subscription_watcher"]["dry_run"], false);

    let disabled = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/api/config")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": 3,
                        "subscription_watcher": {
                            "enabled": false,
                            "dry_run": false
                        }
                    })
                    .to_string(),
                ))
                .expect("build disable request"),
        )
        .await
        .expect("call disable request");
    assert_eq!(disabled.status(), StatusCode::OK);
    assert_eq!(disabled.headers()["x-config-revision"], "4");
    let disabled_json: serde_json::Value =
        serde_json::from_str(&body_text(disabled).await).expect("parse disabled config JSON");
    assert_eq!(disabled_json["subscription_watcher"]["enabled"], false);
    assert_eq!(disabled_json["subscription_watcher"]["dry_run"], false);

    let persisted = fs::read_to_string(&paths.config_path).expect("read disabled config TOML");
    assert!(persisted.contains("enabled = false"));
    assert!(persisted.contains("dry_run = false"));
    assert!(!persisted.contains("confirm_enable_automation"));

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn operation_logs_route_filters_paginates_and_uses_stable_envelopes() {
    let root = temp_test_root("operation-logs-route");
    let paths = AppPaths::for_test_root(&root);
    fs::create_dir_all(&paths.subscription_state_dir)
        .expect("create isolated operation-log state directory");
    let old_database = paths.subscription_state_dir.join("wanted.sqlite");
    let old_sentinel = b"OLD_OPERATION_LOG_DATABASE_MUST_NOT_CHANGE";
    fs::write(&old_database, old_sentinel).expect("write old operation-log database sentinel");
    let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

    for (expected_revision, listen_port) in [(1, 9_001), (2, 9_002)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(loopback_peer())
                    .body(Body::from(
                        serde_json::json!({
                            "expected_revision": expected_revision,
                            "listen_port": listen_port,
                        })
                        .to_string(),
                    ))
                    .expect("build config update that seeds an operation log"),
            )
            .await
            .expect("call config update that seeds an operation log");
        assert_eq!(response.status(), StatusCode::OK);
    }

    let filtered = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(
                    "/api/operation-logs?category=configuration&status=success&q=save_config&page=2&page_size=1",
                )
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build filtered operation log request"),
        )
        .await
        .expect("call filtered operation log route");
    let filtered_status = filtered.status();
    let filtered_body = body_text(filtered).await;
    assert_eq!(filtered_status, StatusCode::OK, "{filtered_body}");
    let filtered: serde_json::Value =
        serde_json::from_str(&filtered_body).expect("parse filtered operation log JSON");
    assert_eq!(filtered["page"], 2);
    assert_eq!(filtered["page_size"], 1);
    assert_eq!(filtered["total"], 2);
    assert_eq!(filtered["has_more"], false);
    assert_eq!(filtered["items"].as_array().map(Vec::len), Some(1));
    assert_eq!(filtered["items"][0]["category"], "configuration");
    assert_eq!(filtered["items"][0]["action"], "save_config");
    assert_eq!(filtered["items"][0]["status"], "success");
    assert_eq!(filtered["items"][0]["summary"], "配置已保存");
    assert!(filtered["items"][0].get("related").is_some());

    let defaults = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/operation-logs?category=all&status=all")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build default operation log request"),
        )
        .await
        .expect("call default operation log route");
    assert_eq!(defaults.status(), StatusCode::OK);
    let defaults = body_json(defaults).await;
    assert_eq!(defaults["page"], 1);
    assert_eq!(defaults["page_size"], 30);
    assert_eq!(defaults["total"], 2);
    assert_eq!(defaults["has_more"], false);
    assert_eq!(defaults["items"].as_array().map(Vec::len), Some(2));

    let invalid = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/operation-logs?page=SECRET_INVALID_PAGE")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build invalid operation log request"),
        )
        .await
        .expect("call invalid operation log route");
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    let invalid = body_json(invalid).await;
    assert_eq!(invalid["code"], "invalid_query");
    assert_eq!(invalid["message"], "query parameters are invalid");
    assert!(invalid.get("error").is_none());
    assert!(!invalid.to_string().contains("SECRET_INVALID_PAGE"));

    let wrong_method = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/operation-logs")
                .extension(loopback_peer())
                .body(Body::empty())
                .expect("build operation log wrong-method request"),
        )
        .await
        .expect("call operation log wrong-method route");
    assert_eq!(wrong_method.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(body_json(wrong_method).await["code"], "method_not_allowed");
    assert_eq!(
        fs::read(&old_database).expect("read old operation-log database sentinel"),
        old_sentinel,
        "config audit and operation-log reads must use subscriptions.sqlite only"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn qb_actions_accept_only_configured_server_ids() {
    const INLINE_SECRET: &str = "SECRET_MUST_NOT_BE_ACCEPTED_INLINE";

    let root = temp_test_root("qb-server-id-boundary");
    let paths = AppPaths::for_test_root(&root);
    fs::create_dir_all(&paths.static_dir).expect("create isolated static directory");
    fs::write(
        paths.static_dir.join("index.html"),
        include_str!("fixtures/http/index.html"),
    )
    .expect("write isolated SPA fixture");
    let app = build_router(AppState::for_test(paths), root.join("static"));

    for (path, body) in [
        (
            "/api/qb/test",
            serde_json::json!({
                "server_id": "nas",
                "base_url": "http://127.0.0.1:8080",
                "username": "admin",
                "password": INLINE_SECRET
            }),
        ),
        (
            "/api/qb/push-mteam",
            serde_json::json!({
                "server_id": "nas",
                "torrent_id": "42",
                "server": {
                    "base_url": "http://127.0.0.1:8080",
                    "username": "admin",
                    "password": INLINE_SECRET
                }
            }),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(loopback_peer())
                    .body(Body::from(body.to_string()))
                    .expect("build inline qB credential request"),
            )
            .await
            .expect("call inline qB credential request");
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!body_text(response).await.contains(INLINE_SECRET));
    }

    let unknown_test = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/qb/test")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(r#"{"server_id":"missing"}"#))
                .expect("build unknown qB test request"),
        )
        .await
        .expect("call unknown qB test request");
    assert_eq!(unknown_test.status(), StatusCode::BAD_REQUEST);
    assert!(body_text(unknown_test).await.contains("server_id"));

    let unknown_push = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/qb/push-mteam")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer())
                .body(Body::from(r#"{"server_id":"missing","torrent_id":"42"}"#))
                .expect("build unknown qB push request"),
        )
        .await
        .expect("call unknown qB push request");
    assert_eq!(unknown_push.status(), StatusCode::BAD_REQUEST);
    let unknown_push_body = body_text(unknown_push).await;
    assert!(unknown_push_body.contains("server_id"));
    assert!(!unknown_push_body.contains("M-Team"));

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn every_production_api_route_has_an_explicit_method_and_auth_contract() {
    let root = temp_test_root("all-api-routes");
    let paths = AppPaths::for_test_root(&root);
    fs::create_dir_all(&paths.static_dir).expect("create isolated static directory");
    fs::write(
        paths.static_dir.join("index.html"),
        include_str!("fixtures/http/index.html"),
    )
    .expect("write isolated SPA fixture");
    let app = build_router(AppState::for_test(paths.clone()), paths.static_dir.clone());

    let public_routes = [
        (Method::GET, "/api/auth/status", Body::empty()),
        (
            Method::POST,
            "/api/auth/login",
            Body::from(r#"{"token":""}"#),
        ),
        (Method::POST, "/api/auth/logout", Body::empty()),
    ];
    for (method, path, body) in public_routes {
        let mut request = Request::builder().method(method).uri(path);
        if path.ends_with("/login") {
            request = request.header(header::CONTENT_TYPE, "application/json");
        }
        let response = app
            .clone()
            .oneshot(
                request
                    .extension(loopback_peer())
                    .body(body)
                    .expect("build public API contract request"),
            )
            .await
            .expect("call public API route");
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "unexpected success status for {path}"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert!(body_json(response).await.get("authenticated").is_some());

        let wrong_method = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(path)
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build public wrong-method request"),
            )
            .await
            .expect("call public API route with wrong method");
        assert_eq!(
            wrong_method.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "wrong method did not produce 405 for {path}"
        );
        assert_eq!(body_json(wrong_method).await["code"], "method_not_allowed");
    }

    let protected_routes = [
        (Method::GET, "/api/config"),
        (Method::PUT, "/api/config"),
        (Method::GET, "/api/operation-logs"),
        (Method::GET, "/api/search"),
        (Method::GET, "/api/douban/search"),
        (Method::GET, "/api/douban/library"),
        (Method::GET, "/api/douban/tags"),
        (Method::GET, "/api/douban/subject/1"),
        (Method::POST, "/api/douban/subject/1/interest"),
        (Method::GET, "/api/douban/image"),
        (Method::POST, "/api/douban/qr/start"),
        (Method::GET, "/api/douban/qr/poll"),
        (Method::GET, "/api/douban/qr/image"),
        (Method::GET, "/api/tmdb/movie/1"),
        (Method::GET, "/api/tmdb/tv/1"),
        (Method::GET, "/api/tmdb/tv/1/season/1"),
        (Method::GET, "/api/mteam/torrents"),
        (Method::POST, "/api/qb/test"),
        (Method::POST, "/api/qb/push-mteam"),
        (Method::GET, "/api/subscriptions/wanted"),
        (Method::GET, "/api/subscriptions/wanted/example"),
        (Method::POST, "/api/subscriptions/wanted/poll"),
    ];

    for (method, path) in protected_routes {
        let unauthenticated = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .expect("build protected API contract request"),
            )
            .await
            .expect("call protected API route without authentication");
        assert_eq!(
            unauthenticated.status(),
            StatusCode::UNAUTHORIZED,
            "registered route/method was not protected: {path}"
        );
        assert_eq!(unauthenticated.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(body_json(unauthenticated).await["code"], "unauthorized");

        let wrong_method = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(path)
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build protected wrong-method request"),
            )
            .await
            .expect("call protected API route with wrong method");
        assert_eq!(
            wrong_method.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "wrong method did not produce 405 for {path}"
        );
        assert_eq!(body_json(wrong_method).await["code"], "method_not_allowed");
    }

    let unknown = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/api/not-a-production-route")
                .body(Body::empty())
                .expect("build unknown API request"),
        )
        .await
        .expect("call unknown API route");
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(unknown).await["code"], "not_found");

    let _ = fs::remove_dir_all(root);
}
