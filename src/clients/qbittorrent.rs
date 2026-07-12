//! qBittorrent Web API v2 adapter.

use serde::{Deserialize, Serialize};

use crate::config::QbServerEntry;

use super::http::{ClientError, PolicyClient, QB_POLICY};

const QB_HTTP_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 tmdb-mteam-hub/0.1";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QbTorrentInfo {
    #[serde(default)]
    pub hash: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub tags: String,
    #[serde(default)]
    pub save_path: String,
    #[serde(default)]
    pub content_path: String,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub downloaded: u64,
    #[serde(default)]
    pub completion_on: i64,
    #[serde(default)]
    pub state: String,
}

impl QbTorrentInfo {
    pub fn is_complete(&self) -> bool {
        self.progress >= 0.999_999 || self.completion_on > 0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QbTorrentFile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub progress: f64,
    #[serde(default)]
    pub priority: i64,
}

fn session_client(insecure_tls: bool) -> Result<PolicyClient, ClientError> {
    PolicyClient::with_builder(QB_POLICY, |mut builder| {
        builder = builder.cookie_store(true).user_agent(QB_HTTP_UA);
        if insecure_tls {
            builder = builder.danger_accept_invalid_certs(true);
        }
        builder
    })
}

fn server_base(server: &QbServerEntry) -> Result<&str, ClientError> {
    let base = server.base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(ClientError::invalid_request(
            QB_POLICY.provider,
            "server Web UI address is empty",
        ));
    }
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(ClientError::invalid_request(
            QB_POLICY.provider,
            "server URL must use HTTP or HTTPS",
        ));
    }
    Ok(base)
}

fn join_url(base: &str, tail: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), tail)
}

async fn login(
    client: &PolicyClient,
    server: &QbServerEntry,
    base: &str,
) -> Result<(), ClientError> {
    let referer = join_url(base, "/");
    let response = client
        .execute(
            client
                .post(join_url(base, "/api/v2/auth/login"))
                .header("Referer", referer)
                .form(&[
                    ("username", server.username.trim()),
                    ("password", server.password.as_str()),
                ]),
        )
        .await?;
    if !response.is_success() {
        return Err(ClientError::for_status(QB_POLICY.provider, response.status));
    }
    let body = response.text(QB_POLICY.provider)?;
    if !is_qb_ok(body) {
        return Err(ClientError::Authentication {
            provider: QB_POLICY.provider,
        });
    }
    Ok(())
}

async fn session(server: &QbServerEntry) -> Result<(PolicyClient, String), ClientError> {
    let base = server_base(server)?.to_string();
    let client = session_client(server.insecure_tls)?;
    login(&client, server, &base).await?;
    Ok((client, base))
}

pub async fn test_connection(server: &QbServerEntry) -> Result<String, ClientError> {
    let (client, base) = session(server).await?;
    let response = client
        .execute(
            client
                .get(join_url(&base, "/api/v2/app/version"))
                .header("Referer", join_url(&base, "/")),
        )
        .await?;
    if !response.is_success() {
        return Err(ClientError::for_status(QB_POLICY.provider, response.status));
    }
    Ok(response.text(QB_POLICY.provider)?.trim().to_string())
}

pub async fn add_torrent_from_url(
    server: &QbServerEntry,
    url: &str,
    category: Option<&str>,
    savepath: Option<&str>,
) -> Result<(), ClientError> {
    add_torrent_from_url_with_tags(server, url, category, savepath, &[]).await
}

pub async fn add_torrent_from_url_with_tags(
    server: &QbServerEntry,
    url: &str,
    category: Option<&str>,
    savepath: Option<&str>,
    tags: &[String],
) -> Result<(), ClientError> {
    let url = url.trim();
    if url.is_empty() {
        return Err(ClientError::invalid_request(
            QB_POLICY.provider,
            "torrent URL is empty",
        ));
    }
    let (client, base) = session(server).await?;
    let form = add_torrent_options_form(
        reqwest::multipart::Form::new()
            .text("urls", url.to_string())
            .text("paused", "false".to_string()),
        category,
        savepath,
        tags,
    );
    submit_add_torrent_form(&client, &base, form).await
}

pub async fn add_torrent_bytes_with_tags(
    server: &QbServerEntry,
    filename: &str,
    bytes: Vec<u8>,
    category: Option<&str>,
    savepath: Option<&str>,
    tags: &[String],
) -> Result<(), ClientError> {
    if bytes.is_empty() {
        return Err(ClientError::invalid_request(
            QB_POLICY.provider,
            "torrent file is empty",
        ));
    }
    let (client, base) = session(server).await?;
    let filename = if filename.trim().is_empty() {
        "download.torrent"
    } else {
        filename.trim()
    };
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(filename.to_string())
        .mime_str("application/x-bittorrent")
        .map_err(|_| ClientError::protocol(QB_POLICY.provider, "invalid torrent upload form"))?;
    let form = add_torrent_options_form(
        reqwest::multipart::Form::new()
            .part("torrents", part)
            .text("paused", "false".to_string()),
        category,
        savepath,
        tags,
    );
    submit_add_torrent_form(&client, &base, form).await
}

fn add_torrent_options_form(
    mut form: reqwest::multipart::Form,
    category: Option<&str>,
    savepath: Option<&str>,
    tags: &[String],
) -> reqwest::multipart::Form {
    if let Some(category) = category.map(str::trim).filter(|value| !value.is_empty()) {
        form = form.text("category", category.to_string());
    }
    if let Some(savepath) = savepath.map(str::trim).filter(|value| !value.is_empty()) {
        form = form.text("savepath", savepath.to_string());
    }
    let tags = tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty() && !tag.contains(','))
        .collect::<Vec<_>>();
    if !tags.is_empty() {
        form = form.text("tags", tags.join(","));
    }
    form
}

async fn submit_add_torrent_form(
    client: &PolicyClient,
    base: &str,
    form: reqwest::multipart::Form,
) -> Result<(), ClientError> {
    let response = client
        .execute(
            client
                .post(join_url(base, "/api/v2/torrents/add"))
                .header("Referer", join_url(base, "/"))
                .multipart(form),
        )
        .await?;
    if !response.is_success() {
        return Err(ClientError::for_status(QB_POLICY.provider, response.status));
    }
    let body = response.text(QB_POLICY.provider)?.trim();
    if body.is_empty() || body.eq_ignore_ascii_case("ok.") {
        return Ok(());
    }
    if body.to_ascii_lowercase().contains("fail") {
        return Err(ClientError::protocol(
            QB_POLICY.provider,
            "torrent add request was rejected",
        ));
    }
    Ok(())
}

pub async fn list_torrents_by_hashes(
    server: &QbServerEntry,
    hashes: &[String],
) -> Result<Vec<QbTorrentInfo>, ClientError> {
    let hashes = hashes
        .iter()
        .map(|hash| hash.trim())
        .filter(|hash| !hash.is_empty())
        .collect::<Vec<_>>();
    if hashes.is_empty() {
        return Ok(Vec::new());
    }
    list_torrents_query(server, Some(&hashes.join("|")), None).await
}

/// Returns torrents carrying the exact qB tag supplied through the Web API's
/// `tag` filter. Callers must still validate the returned tag set because the
/// upstream server, rather than this client, owns filtering semantics.
pub async fn list_torrents_by_exact_tag(
    server: &QbServerEntry,
    tag: &str,
) -> Result<Vec<QbTorrentInfo>, ClientError> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Err(ClientError::invalid_request(
            QB_POLICY.provider,
            "torrent tag is empty",
        ));
    }
    if tag.contains(',') {
        return Err(ClientError::invalid_request(
            QB_POLICY.provider,
            "exact torrent tag must not contain a comma",
        ));
    }
    list_torrents_query(server, None, Some(tag)).await
}

async fn list_torrents_query(
    server: &QbServerEntry,
    hashes: Option<&str>,
    tag: Option<&str>,
) -> Result<Vec<QbTorrentInfo>, ClientError> {
    let (client, base) = session(server).await?;
    let request = torrents_info_request(&client, &base, hashes, tag);
    let response = client.execute(request).await?;
    if !response.is_success() {
        return Err(ClientError::for_status(QB_POLICY.provider, response.status));
    }
    serde_json::from_slice(&response.body)
        .map_err(|_| ClientError::protocol(QB_POLICY.provider, "invalid torrent list JSON"))
}

fn torrents_info_request(
    client: &PolicyClient,
    base: &str,
    hashes: Option<&str>,
    tag: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut request = client
        .get(join_url(base, "/api/v2/torrents/info"))
        .header("Referer", join_url(base, "/"));
    if let Some(hashes) = hashes.map(str::trim).filter(|value| !value.is_empty()) {
        request = request.query(&[("hashes", hashes)]);
    }
    if let Some(tag) = tag.map(str::trim).filter(|value| !value.is_empty()) {
        request = request.query(&[("tag", tag)]);
    }
    request
}

pub async fn torrent_files(
    server: &QbServerEntry,
    hash: &str,
) -> Result<Vec<QbTorrentFile>, ClientError> {
    let hash = hash.trim();
    if hash.is_empty() {
        return Err(ClientError::invalid_request(
            QB_POLICY.provider,
            "torrent hash is empty",
        ));
    }
    let (client, base) = session(server).await?;
    let response = client
        .execute(
            client
                .get(join_url(&base, "/api/v2/torrents/files"))
                .header("Referer", join_url(&base, "/"))
                .query(&[("hash", hash)]),
        )
        .await?;
    if !response.is_success() {
        return Err(ClientError::for_status(QB_POLICY.provider, response.status));
    }
    serde_json::from_slice(&response.body)
        .map_err(|_| ClientError::protocol(QB_POLICY.provider, "invalid torrent file JSON"))
}

fn is_qb_ok(body: &str) -> bool {
    let body = body.trim();
    body.is_empty() || body.eq_ignore_ascii_case("ok.")
}

pub fn policy() -> super::http::HttpClientPolicy {
    QB_POLICY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_success_body_keeps_qb_compatibility() {
        assert!(is_qb_ok(""));
        assert!(is_qb_ok("Ok."));
        assert!(!is_qb_ok("Fails."));
    }

    #[test]
    fn invalid_server_urls_are_typed_client_validation_errors() {
        let server = QbServerEntry {
            id: "qb".to_string(),
            name: "qB".to_string(),
            base_url: "file:///tmp/qb".to_string(),
            username: "user".to_string(),
            password: "secret".to_string(),
            insecure_tls: false,
        };
        assert!(matches!(
            server_base(&server),
            Err(ClientError::InvalidRequest { .. })
        ));
    }

    #[test]
    fn exact_tag_lookup_uses_qb_torrents_info_tag_parameter() {
        let client = session_client(false).unwrap();
        let request = torrents_info_request(
            &client,
            "http://127.0.0.1:8080",
            None,
            Some("download:v1:abc123"),
        )
        .build()
        .unwrap();
        let query = request.url().query_pairs().collect::<Vec<_>>();

        assert_eq!(query, [("tag".into(), "download:v1:abc123".into())]);
        assert_eq!(request.url().path(), "/api/v2/torrents/info");
    }
}
