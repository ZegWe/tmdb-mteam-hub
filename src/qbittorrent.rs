//! qBittorrent Web API v2 — 连接自检与添加任务。

use crate::config::QbServerEntry;
use crate::ApiError;
use axum::http::StatusCode;

const QB_HTTP_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 tmdb-mteam-hub/0.1";

async fn http_client_tls(insecure: bool) -> Result<reqwest::Client, ApiError> {
    let mut b = reqwest::Client::builder()
        .cookie_store(true)
        .user_agent(QB_HTTP_UA)
        .redirect(reqwest::redirect::Policy::limited(15));
    if insecure {
        b = b.danger_accept_invalid_certs(true);
    }
    b.build()
        .map_err(|e| ApiError::internal(format!("HTTP 客户端: {e}")))
}

fn join_url(base: &str, tail: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), tail)
}

pub async fn qb_login_session(
    client: &reqwest::Client,
    server: &QbServerEntry,
) -> Result<(), ApiError> {
    let base = server.base_url.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(ApiError::bad_request("qB 服务器 Web UI 地址为空"));
    }
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(ApiError::bad_request(
            "qB base_url 需以 http:// 或 https:// 开头",
        ));
    }

    let referer = join_url(base, "/");
    let login_url = join_url(base, "/api/v2/auth/login");
    let login = client
        .post(login_url)
        .header("Referer", &referer)
        .form(&[
            ("username", server.username.trim()),
            ("password", server.password.as_str()),
        ])
        .send()
        .await
        .map_err(|e| {
            ApiError::upstream(StatusCode::BAD_GATEWAY, format!("qB 登录请求失败: {e}"))
        })?;
    let lst = login.status();
    let ltext = login
        .text()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !lst.is_success() || !is_qb_ok(&ltext) {
        return Err(ApiError::upstream(
            StatusCode::BAD_GATEWAY,
            format!("qB 登录失败 {lst}: {ltext}"),
        ));
    }
    Ok(())
}

pub async fn test_connection(server: &QbServerEntry) -> Result<String, ApiError> {
    let base = server.base_url.trim().trim_end_matches('/');
    let client = http_client_tls(server.insecure_tls).await?;
    qb_login_session(&client, server).await?;

    let referer = join_url(base, "/");
    let ver_url = join_url(base, "/api/v2/app/version");
    let ver = client
        .get(ver_url)
        .header("Referer", &referer)
        .send()
        .await
        .map_err(|e| {
            ApiError::upstream(StatusCode::BAD_GATEWAY, format!("qB 版本请求失败: {e}"))
        })?;
    let vst = ver.status();
    let vtext = ver
        .text()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !vst.is_success() {
        return Err(ApiError::upstream(
            StatusCode::BAD_GATEWAY,
            format!("qB Web API 返回 {vst}: {vtext}"),
        ));
    }
    Ok(vtext.trim().to_string())
}

/// 通过 Web API 添加网络种子（`urls` 为 qB 能直接访问的 HTTP/HTTPS 下载地址）。
pub async fn add_torrent_from_url(
    server: &QbServerEntry,
    url: &str,
    category: Option<&str>,
    savepath: Option<&str>,
) -> Result<(), ApiError> {
    let u = url.trim();
    if u.is_empty() {
        return Err(ApiError::bad_request("下载地址为空"));
    }

    let base = server.base_url.trim().trim_end_matches('/');
    let client = http_client_tls(server.insecure_tls).await?;
    qb_login_session(&client, server).await?;

    let referer = join_url(base, "/");
    let add_url = join_url(base, "/api/v2/torrents/add");

    let mut form = reqwest::multipart::Form::new()
        .text("urls", u.to_string())
        .text("paused", "false".to_string());
    if let Some(c) = category {
        let t = c.trim();
        if !t.is_empty() {
            form = form.text("category", t.to_string());
        }
    }
    if let Some(s) = savepath {
        let t = s.trim();
        if !t.is_empty() {
            form = form.text("savepath", t.to_string());
        }
    }

    let add = client
        .post(&add_url)
        .header("Referer", &referer)
        .multipart(form)
        .send()
        .await
        .map_err(|e| {
            ApiError::upstream(StatusCode::BAD_GATEWAY, format!("qB 添加任务请求失败: {e}"))
        })?;
    let st = add.status();
    let body = add
        .text()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !st.is_success() {
        return Err(ApiError::upstream(
            StatusCode::BAD_GATEWAY,
            format!("qB 添加任务返回 {st}: {body}"),
        ));
    }
    let b = body.trim();
    if b.is_empty() || b.eq_ignore_ascii_case("ok.") {
        return Ok(());
    }
    if b.to_ascii_lowercase().contains("fail") {
        return Err(ApiError::upstream(
            StatusCode::BAD_GATEWAY,
            format!("qB 添加失败: {b}"),
        ));
    }
    Ok(())
}

fn is_qb_ok(body: &str) -> bool {
    let s = body.trim();
    if s.is_empty() {
        return true;
    }
    s.eq_ignore_ascii_case("ok.")
}
