use std::time::{Duration, Instant};

use axum::response::Redirect;
use axum::routing::get;
use axum::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use tmdb_mteam_server::clients::douban::DoubanClient;
use tmdb_mteam_server::clients::http::{
    ClientError, HttpClientPolicy, PolicyClient, DOUBAN_POLICY, MTEAM_POLICY, QB_POLICY,
    TMDB_POLICY,
};
use tmdb_mteam_server::clients::mteam::MteamClient;
use tmdb_mteam_server::clients::tmdb::TmdbClient;

async fn local_listener() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    (listener, format!("http://{address}"))
}

async fn spawn_raw_response(response: Vec<u8>) -> String {
    let (listener, base_url) = local_listener().await;
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 16 * 1024];
        let _ = stream.read(&mut request).await;
        stream.write_all(&response).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    base_url
}

#[test]
fn every_production_provider_has_a_complete_named_policy() {
    let douban = DoubanClient::new().unwrap();
    let policies = [
        MTEAM_POLICY,
        QB_POLICY,
        TMDB_POLICY,
        DOUBAN_POLICY,
        douban.policies()[1],
    ];

    for policy in policies {
        assert!(!policy.provider.is_empty());
        assert!(!policy.connect_timeout.is_zero());
        assert!(!policy.request_timeout.is_zero());
        assert!(policy.request_timeout >= policy.connect_timeout);
        assert!(policy.redirect_limit > 0);
        assert!(policy.response_body_limit > 0);
    }
}

#[tokio::test]
async fn local_tls_stub_proves_connect_timeout_is_enforced() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let policy = HttpClientPolicy::new(
        "connect-timeout-stub",
        Duration::from_millis(40),
        Duration::from_secs(1),
        1,
        1024,
    );
    let client = PolicyClient::with_builder(policy, |builder| builder.no_proxy()).unwrap();
    let started = Instant::now();

    let error = match client
        .execute(client.get(format!("https://{address}/")))
        .await
    {
        Ok(_) => panic!("TLS handshake stall must time out"),
        Err(error) => error,
    };

    assert!(matches!(error, ClientError::Timeout { .. }));
    assert!(started.elapsed() < Duration::from_millis(750));
}

#[tokio::test]
async fn local_http_stub_proves_total_request_timeout_is_enforced() {
    let (listener, base_url) = local_listener().await;
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let policy = HttpClientPolicy::new(
        "request-timeout-stub",
        Duration::from_secs(1),
        Duration::from_millis(40),
        1,
        1024,
    );
    let client = PolicyClient::with_builder(policy, |builder| builder.no_proxy()).unwrap();
    let started = Instant::now();

    let error = match client.execute(client.get(format!("{base_url}/slow"))).await {
        Ok(_) => panic!("slow response must time out"),
        Err(error) => error,
    };

    assert!(matches!(error, ClientError::Timeout { .. }));
    assert!(started.elapsed() < Duration::from_millis(750));
}

#[tokio::test]
async fn local_redirect_loop_is_rejected_at_the_configured_limit() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/loop", get(|| async { Redirect::temporary("/loop") })),
        )
        .await
        .unwrap();
    });
    let policy = HttpClientPolicy::new(
        "redirect-stub",
        Duration::from_secs(1),
        Duration::from_secs(1),
        2,
        1024,
    );
    let client = PolicyClient::with_builder(policy, |builder| builder.no_proxy()).unwrap();

    let error = match client
        .execute(client.get(format!("http://{address}/loop")))
        .await
    {
        Ok(_) => panic!("redirect loop must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(error, ClientError::Protocol { .. }));
    assert!(error.to_string().contains("redirect policy"));
    server.abort();
}

#[tokio::test]
async fn content_length_larger_than_the_cap_is_rejected_before_body_read() {
    let base_url = spawn_raw_response(
        b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\n".to_vec(),
    )
    .await;
    let policy = HttpClientPolicy::new(
        "length-stub",
        Duration::from_secs(1),
        Duration::from_secs(1),
        1,
        8,
    );
    let client = PolicyClient::with_builder(policy, |builder| builder.no_proxy()).unwrap();

    let error = match client
        .execute(client.get(format!("{base_url}/oversized")))
        .await
    {
        Ok(_) => panic!("oversized response must be rejected"),
        Err(error) => error,
    };

    assert_eq!(
        error,
        ClientError::BodyTooLarge {
            provider: "length-stub",
            limit: 8,
        }
    );
}

#[tokio::test]
async fn chunked_body_without_content_length_is_bounded_while_streaming() {
    let base_url = spawn_raw_response(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nabcd\r\n4\r\nefgh\r\n0\r\n\r\n"
            .to_vec(),
    )
    .await;
    let policy = HttpClientPolicy::new(
        "chunked-stub",
        Duration::from_secs(1),
        Duration::from_secs(1),
        1,
        7,
    );
    let client = PolicyClient::with_builder(policy, |builder| builder.no_proxy()).unwrap();

    let error = match client
        .execute(client.get(format!("{base_url}/chunked")))
        .await
    {
        Ok(_) => panic!("oversized chunked response must be rejected"),
        Err(error) => error,
    };

    assert_eq!(
        error,
        ClientError::BodyTooLarge {
            provider: "chunked-stub",
            limit: 7,
        }
    );
}

#[tokio::test]
async fn tmdb_adapter_uses_the_shared_body_cap_for_api_key_requests() {
    let oversized = TMDB_POLICY.response_body_limit + 1;
    let response =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {oversized}\r\nConnection: close\r\n\r\n")
            .into_bytes();
    let base_url = spawn_raw_response(response).await;
    let http = PolicyClient::with_builder(TMDB_POLICY, |builder| builder.no_proxy()).unwrap();
    let client = TmdbClient::with_http_client(base_url, http);

    let error = client
        .get_json("SUPER_SECRET_TMDB_KEY", "/movie/1", &[])
        .await
        .unwrap_err();

    assert_eq!(
        error,
        ClientError::BodyTooLarge {
            provider: "TMDB",
            limit: TMDB_POLICY.response_body_limit,
        }
    );
    assert!(!error.to_string().contains("SUPER_SECRET_TMDB_KEY"));
}

#[tokio::test]
async fn tmdb_adapter_enforces_its_injected_total_request_timeout() {
    let (listener, base_url) = local_listener().await;
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        let _ = stream.read(&mut request).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
    });
    let policy = HttpClientPolicy::new(
        "TMDB",
        Duration::from_secs(1),
        Duration::from_millis(40),
        TMDB_POLICY.redirect_limit,
        TMDB_POLICY.response_body_limit,
    );
    let http = PolicyClient::with_builder(policy, |builder| builder.no_proxy()).unwrap();
    let client = TmdbClient::with_http_client(base_url, http);

    let error = client
        .get_json("SUPER_SECRET_TMDB_KEY", "/movie/1", &[])
        .await
        .unwrap_err();

    assert_eq!(error, ClientError::Timeout { provider: "TMDB" });
    assert!(!error.to_string().contains("SUPER_SECRET_TMDB_KEY"));
}

#[tokio::test]
async fn provider_errors_do_not_echo_credentials_urls_or_response_bodies() {
    const KEY: &str = "SUPER_SECRET_MTEAM_KEY";
    const RESPONSE_SECRET: &str = "SUPER_SECRET_RESPONSE_BODY";
    let response = format!(
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{RESPONSE_SECRET}",
        RESPONSE_SECRET.len()
    )
    .into_bytes();
    let base_url = spawn_raw_response(response).await;
    let http = PolicyClient::with_builder(MTEAM_POLICY, |builder| builder.no_proxy()).unwrap();
    let client = MteamClient::with_http_client(base_url, http);

    let error = client
        .search(KEY, &serde_json::json!({ "keyword": "movie" }))
        .await
        .unwrap_err();
    let message = error.to_string();

    assert!(matches!(error, ClientError::Authentication { .. }));
    assert!(!message.contains(KEY));
    assert!(!message.contains(RESPONSE_SECRET));
    assert!(!message.contains("127.0.0.1"));
}
