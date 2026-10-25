use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::clients::http::{ClientError, PolicyClient, MTEAM_POLICY};
use crate::clients::mteam::MteamClient;
use crate::clients::qbittorrent;
use crate::config::QbServerEntry;
use axum::body::Bytes;
use axum::extract::Query;
use axum::http::HeaderMap;
use axum::response::Redirect;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;
use sha1::{Digest, Sha1};

const INFO: &[u8] =
    b"d6:lengthi1e4:name4:test12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae";

fn metainfo() -> Vec<u8> {
    [b"d4:info".as_slice(), INFO, b"e"].concat()
}

async fn mteam_stub(body: Vec<u8>, limit: usize) -> (MteamClient, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let download = format!("{base}/redirect?sign=SECRET_SIGN");
    let app = Router::new()
        .route(
            "/api/torrent/genDlToken",
            post(move |headers: HeaderMap| {
                let download = download.clone();
                async move {
                    assert_eq!(headers["x-api-key"], "SECRET_KEY");
                    Json(json!({"code": "0", "data": download}))
                }
            }),
        )
        .route("/redirect", get(|| async { Redirect::temporary("/file") }))
        .route(
            "/file",
            get(move |headers: HeaderMap| {
                let body = body.clone();
                async move {
                    assert!(!headers.contains_key("x-api-key"));
                    assert!(headers["user-agent"]
                        .to_str()
                        .unwrap()
                        .starts_with("Mozilla/"));
                    assert!(headers.contains_key("referer"));
                    // Some download servers use a generic content type for valid metainfo.
                    ([("content-type", "application/octet-stream")], body)
                }
            }),
        );
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut policy = MTEAM_POLICY;
    policy.response_body_limit = limit;
    let http = PolicyClient::with_builder(policy, |builder| builder.no_proxy()).unwrap();
    (MteamClient::with_http_client(base, http), handle)
}

#[tokio::test]
async fn download_follows_redirects_validates_metainfo_and_does_not_forward_api_key() {
    let (client, server) = mteam_stub(metainfo(), 4096).await;
    let torrent = client.fetch_torrent("SECRET_KEY", "42").await.unwrap();
    assert_eq!(torrent.bytes, metainfo());
    assert_eq!(torrent.info_hash, format!("{:x}", Sha1::digest(INFO)));
    server.abort();
}

#[tokio::test]
async fn download_rejects_site_error_pages_and_truncated_or_oversized_files() {
    for body in [
        b"<html>SECRET_RESPONSE</html>".to_vec(),
        br#"{"code":"1","message":"SECRET_RESPONSE"}"#.to_vec(),
        metainfo()[..metainfo().len() - 1].to_vec(),
    ] {
        let (client, server) = mteam_stub(body, 4096).await;
        let error = client
            .fetch_torrent("SECRET_KEY", "42")
            .await
            .err()
            .unwrap();
        assert!(matches!(error, ClientError::Protocol { .. }));
        for secret in ["SECRET_KEY", "SECRET_SIGN", "SECRET_RESPONSE"] {
            assert!(!error.to_string().contains(secret));
        }
        server.abort();
    }
    let (client, server) = mteam_stub(vec![b'x'; 8192], 4096).await;
    assert!(matches!(
        client.fetch_torrent("SECRET_KEY", "42").await,
        Err(ClientError::BodyTooLarge { .. })
    ));
    server.abort();
}

async fn qb_stub(
    visible_after: usize,
) -> (QbServerEntry, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let observations = Arc::new(AtomicUsize::new(0));
    let calls = observations.clone();
    let app = Router::new()
        .route("/api/v2/auth/login", post(|| async { "Ok." }))
        .route(
            "/api/v2/torrents/add",
            post(|headers: HeaderMap, body: Bytes| async move {
                assert!(headers["content-type"]
                    .to_str()
                    .unwrap()
                    .starts_with("multipart/form-data;"));
                let text = String::from_utf8_lossy(&body);
                assert!(text.contains("name=\"torrents\"; filename=\"42.torrent\""));
                assert!(!text.contains("name=\"urls\""));
                assert!(body
                    .windows(metainfo().len())
                    .any(|window| window == metainfo()));
                for option in ["Movie", "/downloads/Movies", "download:v1:test"] {
                    assert!(text.contains(option));
                }
                "Ok."
            }),
        )
        .route(
            "/api/v2/torrents/info",
            get(
                move |Query(query): Query<std::collections::HashMap<String, String>>| {
                    let calls = calls.clone();
                    async move {
                        let hash = format!("{:x}", Sha1::digest(INFO));
                        assert_eq!(query.get("hashes"), Some(&hash));
                        if calls.fetch_add(1, Ordering::SeqCst) >= visible_after {
                            Json(json!([{"hash": hash, "name": "test"}]))
                        } else {
                            Json(json!([]))
                        }
                    }
                },
            ),
        );
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        QbServerEntry {
            id: "qb".into(),
            name: "qb".into(),
            base_url,
            username: "test".into(),
            password: "secret".into(),
            insecure_tls: false,
        },
        observations,
        handle,
    )
}

#[tokio::test]
async fn upload_sends_file_and_options_and_waits_for_the_actual_qb_task() {
    let (mteam, mteam_handle) = mteam_stub(metainfo(), 4096).await;
    let torrent = mteam.fetch_torrent("SECRET_KEY", "42").await.unwrap();
    let (server, calls, handle) = qb_stub(1).await;
    qbittorrent::add_torrent_bytes_with_tags(
        &server,
        "42.torrent",
        torrent.bytes,
        Some("Movie"),
        Some("/downloads/Movies"),
        &["download:v1:test".into()],
    )
    .await
    .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    handle.abort();
    mteam_handle.abort();
}

#[tokio::test]
async fn upload_does_not_report_success_when_qb_never_imports_the_file() {
    let (server, calls, handle) = qb_stub(usize::MAX).await;
    let error = qbittorrent::add_torrent_bytes_with_tags(
        &server,
        "42.torrent",
        metainfo(),
        Some("Movie"),
        Some("/downloads/Movies"),
        &["download:v1:test".into()],
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("task was not found"));
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    handle.abort();
}
