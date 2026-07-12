use axum::http::{header, HeaderValue, Method};
use axum::Router;
use tower_http::cors::CorsLayer;

use crate::config::ManagementConfig;

pub(crate) fn apply_cors(router: Router, management: &ManagementConfig) -> Router {
    if management.allowed_origins.is_empty() {
        return router;
    }
    let origins = management
        .allowed_origins
        .iter()
        .map(|origin| {
            HeaderValue::from_str(origin).expect("validated management origin is a valid header")
        })
        .collect::<Vec<_>>();
    router.layer(
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::GET, Method::POST, Method::PUT])
            .allow_headers([header::ACCEPT, header::AUTHORIZATION, header::CONTENT_TYPE])
            .allow_credentials(true),
    )
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{header, Method, Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    use super::apply_cors;
    use crate::config::ManagementConfig;

    fn test_router(management: &ManagementConfig) -> Router {
        apply_cors(
            Router::new().route("/ping", get(|| async { StatusCode::OK })),
            management,
        )
    }

    #[tokio::test]
    async fn same_origin_mode_emits_no_cors_headers() {
        let response = test_router(&ManagementConfig::default())
            .oneshot(
                Request::builder()
                    .uri("/ping")
                    .header(header::ORIGIN, "https://other.example")
                    .body(Body::empty())
                    .expect("build same-origin-mode request"),
            )
            .await
            .expect("call same-origin-mode route");

        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));
    }

    #[tokio::test]
    async fn cors_allows_only_configured_exact_origin() {
        let management = ManagementConfig {
            allowed_origins: vec!["https://admin.example:8443".to_string()],
            ..ManagementConfig::default()
        };
        management.validate().expect("exact origin should validate");
        let app = test_router(&management);

        let allowed = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ping")
                    .header(header::ORIGIN, "https://admin.example:8443")
                    .body(Body::empty())
                    .expect("build allowed-origin request"),
            )
            .await
            .expect("call allowed-origin route");
        assert_eq!(
            allowed.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://admin.example:8443"
        );
        assert_eq!(
            allowed.headers()[header::ACCESS_CONTROL_ALLOW_CREDENTIALS],
            "true"
        );

        let denied = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/ping")
                    .header(header::ORIGIN, "https://evil.example")
                    .body(Body::empty())
                    .expect("build denied-origin request"),
            )
            .await
            .expect("call denied-origin route");
        assert!(!denied
            .headers()
            .contains_key(header::ACCESS_CONTROL_ALLOW_ORIGIN));

        let preflight = app
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri("/ping")
                    .header(header::ORIGIN, "https://admin.example:8443")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "PUT")
                    .header(
                        header::ACCESS_CONTROL_REQUEST_HEADERS,
                        "authorization,content-type",
                    )
                    .body(Body::empty())
                    .expect("build allowed preflight request"),
            )
            .await
            .expect("call allowed preflight route");
        assert_eq!(
            preflight.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            "https://admin.example:8443"
        );
    }

    #[test]
    fn cors_rejects_wildcard_and_null_origin() {
        for origin in [
            "*",
            "null",
            "https://*.example.com",
            "https://user@example.com",
            "https://example.com/admin",
        ] {
            let management = ManagementConfig {
                allowed_origins: vec![origin.to_string()],
                ..ManagementConfig::default()
            };
            assert!(
                management.validate().is_err(),
                "origin should be rejected: {origin}"
            );
        }
    }
}
