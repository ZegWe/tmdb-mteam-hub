use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use axum::extract::connect_info::ConnectInfo;
use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::{Extension, Router};
use ring::digest::{digest, SHA256};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::app::auth_security::LoginRateLimitDecision;
use crate::app::AppState;
use crate::config::ManagementConfig;
use crate::http::error;
use crate::http::error::{ApiError, ApiJson};

const SESSION_COOKIE_NAME: &str = "tmdb_mteam_admin_session";
const SEC_FETCH_SITE: &str = "sec-fetch-site";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthenticationKind {
    Bearer,
    Cookie,
    Bootstrap,
    None,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    token: String,
}

#[derive(Debug, Serialize)]
struct AuthStatus {
    authenticated: bool,
    token_configured: bool,
    bootstrap_allowed: bool,
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/auth/status", get(status))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .method_not_allowed_fallback(error::method_not_allowed)
}

pub(crate) async fn require_management_auth(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|connect_info| connect_info.0);
    let management = state.config.snapshot().await.value.management;
    let kind = authentication_kind(&management, request.headers(), peer);
    match kind {
        AuthenticationKind::None => unauthorized_response(),
        AuthenticationKind::Cookie
            if !is_safe_method(request.method())
                && !cookie_mutation_is_same_origin(request.headers()) =>
        {
            csrf_rejected_response()
        }
        AuthenticationKind::Bootstrap
            if !is_safe_method(request.method())
                && bootstrap_mutation_is_cross_site(request.headers()) =>
        {
            csrf_rejected_response()
        }
        _ => next.run(request).await,
    }
}

async fn status(
    State(state): State<AppState>,
    headers: HeaderMap,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
) -> Response {
    let management = state.config.snapshot().await.value.management;
    no_store_json(authentication_status(
        &management,
        &headers,
        peer.map(|Extension(connect_info)| connect_info.0),
    ))
}

async fn login(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    ApiJson(body): ApiJson<LoginRequest>,
) -> Response {
    let management = state.config.snapshot().await.value.management;
    let configured_token = management.admin_token.trim();
    let peer_ip = peer
        .as_ref()
        .map(|Extension(connect_info)| connect_info.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    let bootstrap_allowed = configured_token.is_empty()
        && peer
            .map(|Extension(connect_info)| connect_info.0.ip().is_loopback())
            .unwrap_or(false);

    if configured_token.is_empty() {
        return if bootstrap_allowed {
            no_store_json(AuthStatus {
                authenticated: true,
                token_configured: false,
                bootstrap_allowed: true,
            })
        } else {
            unauthorized_response()
        };
    }
    if let LoginRateLimitDecision::RetryAfter(seconds) = state.login_rate_limiter.check(peer_ip) {
        return rate_limited_response(seconds);
    }
    if !token_matches(body.token.trim(), configured_token) {
        state.login_rate_limiter.record_failure(peer_ip);
        return unauthorized_response();
    }
    state.login_rate_limiter.record_success(peer_ip);

    let cookie = session_cookie(configured_token, management.secure_cookie);
    let mut response = no_store_json(AuthStatus {
        authenticated: true,
        token_configured: true,
        bootstrap_allowed: false,
    });
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&cookie).expect("digest-backed auth cookie is a valid header"),
    );
    response
}

async fn logout(
    State(state): State<AppState>,
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
) -> Response {
    let management = state.config.snapshot().await.value.management;
    let token_configured = !management.admin_token.trim().is_empty();
    let bootstrap_allowed = !token_configured
        && peer
            .map(|Extension(connect_info)| connect_info.0.ip().is_loopback())
            .unwrap_or(false);
    let mut response = no_store_json(AuthStatus {
        authenticated: bootstrap_allowed,
        token_configured,
        bootstrap_allowed,
    });
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&expired_session_cookie(management.secure_cookie))
            .expect("expired auth cookie is a valid header"),
    );
    response
}

fn authentication_status(
    management: &ManagementConfig,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> AuthStatus {
    let configured_token = management.admin_token.trim();
    let token_configured = !configured_token.is_empty();
    let bootstrap_allowed =
        !token_configured && peer.map(|peer| peer.ip().is_loopback()).unwrap_or(false);
    let authenticated = authentication_kind(management, headers, peer) != AuthenticationKind::None;
    AuthStatus {
        authenticated,
        token_configured,
        bootstrap_allowed,
    }
}

fn authentication_kind(
    management: &ManagementConfig,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) -> AuthenticationKind {
    let configured_token = management.admin_token.trim();
    if configured_token.is_empty() {
        return if peer.is_some_and(|peer| peer.ip().is_loopback()) {
            AuthenticationKind::Bootstrap
        } else {
            AuthenticationKind::None
        };
    }
    if bearer_token(headers).is_some_and(|candidate| token_matches(candidate, configured_token)) {
        return AuthenticationKind::Bearer;
    }
    if session_cookie_value(headers)
        .is_some_and(|candidate| session_matches(candidate, configured_token))
    {
        return AuthenticationKind::Cookie;
    }
    AuthenticationKind::None
}

fn is_safe_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

fn fetch_site(headers: &HeaderMap) -> Option<&str> {
    headers.get(SEC_FETCH_SITE)?.to_str().ok()
}

fn cookie_mutation_is_same_origin(headers: &HeaderMap) -> bool {
    match fetch_site(headers) {
        Some(value) => value.eq_ignore_ascii_case("same-origin"),
        None => !headers.contains_key(header::ORIGIN),
    }
}

fn bootstrap_mutation_is_cross_site(headers: &HeaderMap) -> bool {
    match fetch_site(headers) {
        Some(value) => {
            value.eq_ignore_ascii_case("same-site") || value.eq_ignore_ascii_case("cross-site")
        }
        None => headers.contains_key(header::ORIGIN),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty()).then(|| token.trim())
}

fn session_cookie_value(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(name, value)| (name == SESSION_COOKIE_NAME).then_some(value))
}

fn token_matches(candidate: &str, configured: &str) -> bool {
    token_digest(candidate)
        .ct_eq(&token_digest(configured))
        .into()
}

fn session_matches(candidate: &str, configured: &str) -> bool {
    let expected = digest_hex(&token_digest(configured));
    candidate.len() == expected.len() && bool::from(candidate.as_bytes().ct_eq(expected.as_bytes()))
}

fn token_digest(token: &str) -> [u8; 32] {
    let digest = digest(&SHA256, token.as_bytes());
    let mut out = [0_u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

fn digest_hex(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

fn session_cookie(configured_token: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE_NAME}={}; Path=/api; HttpOnly; SameSite=Strict{secure}",
        digest_hex(&token_digest(configured_token))
    )
}

fn expired_session_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE_NAME}=; Path=/api; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
}

fn unauthorized_response() -> Response {
    let mut response = ApiError::unauthorized("management authentication required").into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Bearer realm=\"tmdb-mteam-hub\""),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn csrf_rejected_response() -> Response {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "csrf_rejected",
        "cookie-authenticated mutations require a same-origin browser request",
    )
    .into_response()
}

fn rate_limited_response(retry_after_secs: u64) -> Response {
    let mut response = ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "rate_limited",
        "too many failed login attempts; try again later",
    )
    .into_response();
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from_str(&retry_after_secs.to_string())
            .expect("retry-after seconds are a valid header"),
    );
    response
}

fn no_store_json(status: AuthStatus) -> Response {
    let mut response = Json(status).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::Body;
    use axum::extract::connect_info::ConnectInfo;
    use axum::http::{header, Method, Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::{digest_hex, token_digest, SEC_FETCH_SITE};
    use crate::app::{AppPaths, AppState};
    use crate::config::{FileConfig, ManagementConfig};
    use crate::http::router::build_api_router;

    const ADMIN_TOKEN: &str = "test-management-token-123456789";

    fn temp_test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-auth-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create auth test root");
        root
    }

    fn test_router(label: &str, admin_token: &str, secure_cookie: bool) -> Router {
        let root = temp_test_root(label);
        let paths = AppPaths::for_test_root(root);
        let config = FileConfig {
            management: ManagementConfig {
                admin_token: admin_token.to_string(),
                secure_cookie,
                ..ManagementConfig::default()
            },
            ..FileConfig::default()
        };
        build_api_router(AppState::for_test_with_config(paths, config))
    }

    fn peer(ip: IpAddr) -> ConnectInfo<SocketAddr> {
        ConnectInfo(SocketAddr::new(ip, 42000))
    }

    fn loopback_peer() -> ConnectInfo<SocketAddr> {
        peer(IpAddr::V4(Ipv4Addr::LOCALHOST))
    }

    fn remote_peer() -> ConnectInfo<SocketAddr> {
        peer(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)))
    }

    async fn body_text(response: axum::response::Response) -> String {
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect auth response body")
            .to_bytes();
        String::from_utf8(bytes.to_vec()).expect("auth response should be utf-8")
    }

    async fn login(app: &Router, token: &str) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(loopback_peer())
                    .body(Body::from(
                        serde_json::json!({ "token": token }).to_string(),
                    ))
                    .expect("build login request"),
            )
            .await
            .expect("call login route")
    }

    #[tokio::test]
    async fn management_api_rejects_missing_cookie_when_token_configured() {
        let app = test_router("missing-cookie", ADMIN_TOKEN, false);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config")
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build protected request"),
            )
            .await
            .expect("call protected route");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            response.headers()[header::WWW_AUTHENTICATE],
            "Bearer realm=\"tmdb-mteam-hub\""
        );
        let body = body_text(response).await;
        let error: serde_json::Value =
            serde_json::from_str(&body).expect("parse unauthorized API error");
        assert_eq!(error["code"], "unauthorized");
        assert_eq!(error["message"], "management authentication required");
        assert!(error.get("details").is_none());
        assert!(error.get("error").is_none());
        assert!(!body.contains(ADMIN_TOKEN));
    }

    #[tokio::test]
    async fn malformed_login_json_uses_shared_error_envelope() {
        let app = test_router("malformed-login", ADMIN_TOKEN, false);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(loopback_peer())
                    .body(Body::from("{"))
                    .expect("build malformed login request"),
            )
            .await
            .expect("call malformed login route");

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value =
            serde_json::from_str(&body_text(response).await).expect("parse malformed login error");
        assert_eq!(body["code"], "invalid_json");
        assert!(body["message"].is_string());
        assert!(body.get("error").is_none());
    }

    #[tokio::test]
    async fn management_api_accepts_valid_login_cookie() {
        let app = test_router("login-cookie", ADMIN_TOKEN, false);
        let login_response = login(&app, ADMIN_TOKEN).await;
        assert_eq!(login_response.status(), StatusCode::OK);
        let cookie = login_response.headers()[header::SET_COOKIE]
            .to_str()
            .expect("login cookie should be ASCII")
            .split(';')
            .next()
            .expect("login cookie should contain a pair")
            .to_string();

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config")
                    .header(header::COOKIE, cookie)
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build cookie-authenticated request"),
            )
            .await
            .expect("call cookie-authenticated route");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn management_api_accepts_valid_bearer_token() {
        let app = test_router("bearer", ADMIN_TOKEN, false);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config")
                    .header(header::AUTHORIZATION, format!("Bearer {ADMIN_TOKEN}"))
                    .extension(remote_peer())
                    .body(Body::empty())
                    .expect("build bearer-authenticated request"),
            )
            .await
            .expect("call bearer-authenticated route");

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn cookie_authenticated_mutations_require_same_origin_fetch_metadata() {
        let app = test_router("cookie-csrf", ADMIN_TOKEN, false);
        let login_response = login(&app, ADMIN_TOKEN).await;
        let cookie = login_response.headers()[header::SET_COOKIE]
            .to_str()
            .expect("login cookie should be ASCII")
            .split(';')
            .next()
            .expect("login cookie should contain a pair")
            .to_string();

        for (fetch_site, origin) in [
            (Some("same-site"), None),
            (Some("cross-site"), None),
            (None, Some("https://untrusted.example")),
        ] {
            let mut request = Request::builder()
                .method(Method::PUT)
                .uri("/config")
                .header(header::COOKIE, &cookie)
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer());
            if let Some(fetch_site) = fetch_site {
                request = request.header(SEC_FETCH_SITE, fetch_site);
            }
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            let response = app
                .clone()
                .oneshot(
                    request
                        .body(Body::from("{}"))
                        .expect("build mutation request"),
                )
                .await
                .expect("call cookie-authenticated mutation");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let body = body_text(response).await;
            assert!(body.contains("csrf_rejected"));
            assert!(!body.contains(ADMIN_TOKEN));
        }

        let same_origin = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/config")
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(SEC_FETCH_SITE, "same-origin")
                    .extension(loopback_peer())
                    .body(Body::from("{}"))
                    .expect("build same-origin mutation"),
            )
            .await
            .expect("call same-origin mutation");
        assert_eq!(same_origin.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let no_fetch_metadata = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/config")
                    .header(header::COOKIE, &cookie)
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(loopback_peer())
                    .body(Body::from("{}"))
                    .expect("build no-fetch-metadata mutation"),
            )
            .await
            .expect("call no-fetch-metadata mutation");
        assert_eq!(no_fetch_metadata.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn bearer_mutations_do_not_depend_on_browser_fetch_metadata() {
        let app = test_router("bearer-csrf", ADMIN_TOKEN, false);
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/config")
                    .header(header::AUTHORIZATION, format!("Bearer {ADMIN_TOKEN}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .extension(remote_peer())
                    .body(Body::from("{}"))
                    .expect("build bearer mutation"),
            )
            .await
            .expect("call bearer mutation");

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn cross_site_browser_requests_cannot_mutate_loopback_bootstrap() {
        let app = test_router("bootstrap-csrf", "", false);
        for (fetch_site, origin) in [
            (Some("cross-site"), None),
            (Some("same-site"), None),
            (None, Some("https://untrusted.example")),
        ] {
            let mut request = Request::builder()
                .method(Method::PUT)
                .uri("/config")
                .header(header::CONTENT_TYPE, "application/json")
                .extension(loopback_peer());
            if let Some(fetch_site) = fetch_site {
                request = request.header(SEC_FETCH_SITE, fetch_site);
            }
            if let Some(origin) = origin {
                request = request.header(header::ORIGIN, origin);
            }
            let response = app
                .clone()
                .oneshot(
                    request
                        .body(Body::from("{}"))
                        .expect("build cross-site bootstrap mutation"),
                )
                .await
                .expect("call cross-site bootstrap mutation");
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
    }

    #[tokio::test]
    async fn management_api_rejects_wrong_token_without_echoing_it() {
        let app = test_router("wrong-token", ADMIN_TOKEN, false);
        let wrong_token = "wrong-management-token-987654321";

        let response = login(&app, wrong_token).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = body_text(response).await;
        assert!(!body.contains(wrong_token));
        assert!(!body.contains(ADMIN_TOKEN));
    }

    #[tokio::test]
    async fn repeated_login_failures_are_rate_limited_per_peer_without_echoing_tokens() {
        let app = test_router("login-rate-limit", ADMIN_TOKEN, false);
        let wrong_token = "wrong-management-token-987654321";

        for _ in 0..5 {
            let response = login(&app, wrong_token).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let response = login(&app, ADMIN_TOKEN).await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().contains_key(header::RETRY_AFTER));
        let body = body_text(response).await;
        assert!(body.contains("rate_limited"));
        assert!(!body.contains(wrong_token));
        assert!(!body.contains(ADMIN_TOKEN));
    }

    #[tokio::test]
    async fn successful_login_clears_prior_failure_budget() {
        let app = test_router("login-rate-limit-reset", ADMIN_TOKEN, false);
        let wrong_token = "wrong-management-token-987654321";

        for _ in 0..4 {
            assert_eq!(
                login(&app, wrong_token).await.status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(login(&app, ADMIN_TOKEN).await.status(), StatusCode::OK);
        for _ in 0..4 {
            assert_eq!(
                login(&app, wrong_token).await.status(),
                StatusCode::UNAUTHORIZED
            );
        }
    }

    #[tokio::test]
    async fn local_bootstrap_requires_loopback_peer() {
        let app = test_router("bootstrap", "", false);

        let local = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/config")
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build local bootstrap request"),
            )
            .await
            .expect("call local bootstrap route");
        assert_eq!(local.status(), StatusCode::OK);

        let remote = app
            .oneshot(
                Request::builder()
                    .uri("/config")
                    .extension(remote_peer())
                    .body(Body::empty())
                    .expect("build remote bootstrap request"),
            )
            .await
            .expect("call remote bootstrap route");
        assert_eq!(remote.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn local_bootstrap_rejects_missing_connect_info() {
        let app = test_router("bootstrap-missing-peer", "", false);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config")
                    .body(Body::empty())
                    .expect("build bootstrap request without peer info"),
            )
            .await
            .expect("call bootstrap route without peer info");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn changing_admin_token_invalidates_existing_cookie_immediately() {
        let root = temp_test_root("token-rotation");
        let paths = AppPaths::for_test_root(root);
        let config = FileConfig {
            management: ManagementConfig {
                admin_token: ADMIN_TOKEN.to_string(),
                ..ManagementConfig::default()
            },
            ..FileConfig::default()
        };
        let state = AppState::for_test_with_config(paths, config);
        let app = build_api_router(state.clone());
        let login_response = login(&app, ADMIN_TOKEN).await;
        let cookie = login_response.headers()[header::SET_COOKIE]
            .to_str()
            .expect("login cookie should be ASCII")
            .split(';')
            .next()
            .expect("login cookie should contain a pair")
            .to_string();

        state
            .config
            .update(None, |config| {
                config.management.admin_token = "rotated-management-token-987654321".to_string();
                Ok(())
            })
            .await
            .expect("rotate management token");

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/config")
                    .header(header::COOKIE, cookie)
                    .extension(loopback_peer())
                    .body(Body::empty())
                    .expect("build request with stale cookie"),
            )
            .await
            .expect("call protected route with stale cookie");

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn token_digest_matches_sha256_test_vector() {
        assert_eq!(
            digest_hex(&token_digest("abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn login_cookie_has_httponly_samesite_and_api_path() {
        let app = test_router("cookie-attributes", ADMIN_TOKEN, true);

        let response = login(&app, ADMIN_TOKEN).await;
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .expect("login cookie should be ASCII");
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(cookie.contains("Path=/api"));
        assert!(cookie.contains("Secure"));
        assert!(!cookie.contains("Domain="));
        assert!(!cookie.contains(ADMIN_TOKEN));
    }
}
