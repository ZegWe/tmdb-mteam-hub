mod config;
mod douban;
mod qbittorrent;
mod tmdb_cache;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path as PathParam, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use config::{FileConfig, QbServerEntry, SubscriptionCategory};
use serde::Deserialize;
use serde_json::{json, Value};
use tmdb_cache::TmdbDiskCache;
use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    config_path: PathBuf,
    config: std::sync::Arc<RwLock<FileConfig>>,
    tmdb_cache: TmdbDiskCache,
    douban_cache: TmdbDiskCache,
    douban_cache_ttl_secs: u64,
    douban_qr_sessions: std::sync::Arc<RwLock<HashMap<String, douban::QrSession>>>,
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "tmdb_mteam_server=info,tower_http=info,axum=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config_path = resolve_config_path();
    tracing::info!("config path: {}", config_path.display());
    let file_cfg = FileConfig::load_or_create(&config_path)?;
    let listen_addr = file_cfg.listen_addr()?;

    let cache_dir = resolve_tmdb_cache_dir();
    let cache_ttl_secs: u64 = std::env::var("TMDB_CACHE_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(604_800);
    let tmdb_cache = TmdbDiskCache::new(cache_dir.clone(), Duration::from_secs(cache_ttl_secs));
    tmdb_cache.ensure_dir().await?;
    tracing::info!(
        "tmdb cache: dir={} ttl={}s",
        cache_dir.display(),
        cache_ttl_secs
    );

    let douban_cache_dir = resolve_douban_cache_dir();
    let douban_cache_ttl_secs: u64 = std::env::var("DOUBAN_CACHE_TTL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(86_400);
    let douban_cache = TmdbDiskCache::new(
        douban_cache_dir.clone(),
        Duration::from_secs(douban_cache_ttl_secs),
    );
    douban_cache.ensure_dir().await?;
    tracing::info!(
        "douban cache: dir={} ttl={}s",
        douban_cache_dir.display(),
        douban_cache_ttl_secs
    );

    let state = AppState {
        config_path,
        config: std::sync::Arc::new(RwLock::new(file_cfg)),
        tmdb_cache,
        douban_cache,
        douban_cache_ttl_secs,
        douban_qr_sessions: std::sync::Arc::new(RwLock::new(HashMap::new())),
    };

    let api = Router::new()
        .route("/config", get(get_config).put(put_config))
        .route("/search", get(search_tmdb))
        .route("/douban/search", get(douban_search))
        .route("/douban/library", get(douban_library))
        .route("/douban/tags", get(douban_tag_history))
        .route("/douban/subject/{id}", get(douban_subject_detail))
        .route("/douban/subject/{id}/interest", post(douban_mark_interest))
        .route("/douban/image", get(douban_image))
        .route("/douban/qr/start", post(douban_qr_start))
        .route("/douban/qr/poll", get(douban_qr_poll))
        .route("/douban/qr/image", get(douban_qr_image))
        .route("/tmdb/movie/{id}", get(tmdb_movie_detail))
        .route("/tmdb/tv/{id}/season/{season}", get(tmdb_tv_season_detail))
        .route("/tmdb/tv/{id}", get(tmdb_tv_detail))
        .route("/mteam/torrents", get(mteam_search))
        .route("/qb/test", post(qb_test))
        .route("/qb/push-mteam", post(qb_push_mteam))
        .with_state(state);

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let static_path = static_dir().unwrap_or_else(|_| PathBuf::from("static"));

    let static_svc = ServeDir::new(&static_path)
        .not_found_service(ServeFile::new(static_path.join("index.html")));

    let app = Router::new()
        .nest("/api", api)
        .fallback_service(static_svc)
        .layer(cors);

    let listener = tokio::net::TcpListener::bind(listen_addr).await?;
    tracing::info!("listen http://{listen_addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

/// 默认读写目录：进程的当前工作目录（`std::env::current_dir()`，失败则用 `.`）。
/// 需在固定目录读写时可设置环境变量 `CONFIG_PATH`、`TMDB_CACHE_DIR`。
fn cwd_or_dot() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn resolve_config_path() -> PathBuf {
    std::env::var("CONFIG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| cwd_or_dot().join("config.toml"))
}

fn resolve_tmdb_cache_dir() -> PathBuf {
    std::env::var("TMDB_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| cwd_or_dot().join("cache").join("tmdb"))
}

fn resolve_douban_cache_dir() -> PathBuf {
    std::env::var("DOUBAN_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| cwd_or_dot().join("cache").join("douban"))
}

fn static_dir() -> std::io::Result<PathBuf> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static");
    if manifest.is_dir() {
        return Ok(manifest);
    }
    let cwd = std::env::current_dir()?.join("static");
    if cwd.is_dir() {
        return Ok(cwd);
    }
    Ok(manifest)
}

async fn get_config(State(state): State<AppState>) -> impl IntoResponse {
    let cfg = state.config.read().await.clone();
    Json(json!({
        "listen_ip": cfg.listen_ip,
        "listen_port": cfg.listen_port,
        "tmdb_api_key": cfg.tmdb_api_key,
        "mteam_api_key": cfg.mteam_api_key,
        "douban_cookie": cfg.douban_cookie,
        "qb_servers": cfg.qb_servers,
        "subscription_categories": cfg.subscription_categories,
    }))
}

#[derive(Deserialize)]
struct PutConfigBody {
    #[serde(default)]
    listen_ip: Option<String>,
    #[serde(default)]
    listen_port: Option<u16>,
    tmdb_api_key: String,
    mteam_api_key: String,
    #[serde(default)]
    douban_cookie: String,
    #[serde(default)]
    qb_servers: Vec<QbServerEntry>,
    #[serde(default)]
    subscription_categories: Option<Vec<SubscriptionCategory>>,
}

async fn put_config(
    State(state): State<AppState>,
    Json(body): Json<PutConfigBody>,
) -> Result<impl IntoResponse, ApiError> {
    let mut new_cfg = state.config.read().await.clone();
    if let Some(listen_ip) = body.listen_ip {
        new_cfg.listen_ip = listen_ip;
    }
    if let Some(listen_port) = body.listen_port {
        new_cfg.listen_port = listen_port;
    }
    new_cfg.tmdb_api_key = body.tmdb_api_key;
    new_cfg.mteam_api_key = body.mteam_api_key;
    new_cfg.douban_cookie = douban::normalize_cookie_header(&body.douban_cookie);
    new_cfg.qb_servers = body.qb_servers;
    if let Some(subscription_categories) = body.subscription_categories {
        new_cfg.subscription_categories =
            normalize_subscription_categories(subscription_categories)?;
    }
    new_cfg
        .listen_addr()
        .map_err(|e| ApiError::bad_request(format!("监听地址配置无效: {e}")))?;
    new_cfg
        .save(&state.config_path)
        .map_err(|e| ApiError::internal(format!("写入配置失败: {e}")))?;
    *state.config.write().await = new_cfg;
    Ok(StatusCode::NO_CONTENT)
}

fn normalize_subscription_categories(
    categories: Vec<SubscriptionCategory>,
) -> Result<Vec<SubscriptionCategory>, ApiError> {
    let mut out = Vec::new();
    let mut names = HashSet::new();
    let mut wanted_tags = HashSet::new();

    for (idx, category) in categories.into_iter().enumerate() {
        let n = idx + 1;
        let normalized = SubscriptionCategory {
            name: category.name.trim().to_string(),
            wanted_tag: category.wanted_tag.trim().to_string(),
            qb_category: category.qb_category.trim().to_string(),
            qb_save_dir_name: category.qb_save_dir_name.trim().to_string(),
            download_dir: category.download_dir.trim().to_string(),
            link_target_dir: category.link_target_dir.trim().to_string(),
        };

        if normalized.name.is_empty() {
            return Err(ApiError::bad_request(format!("订阅分类 #{n} 缺少分类名")));
        }
        if normalized.wanted_tag.is_empty() {
            return Err(ApiError::bad_request(format!(
                "订阅分类 {} 缺少想看标签文本",
                normalized.name
            )));
        }
        if normalized.wanted_tag.split_whitespace().count() != 1 {
            return Err(ApiError::bad_request(format!(
                "订阅分类 {} 的想看标签不能包含空白字符",
                normalized.name
            )));
        }
        if normalized.qb_category.is_empty() {
            return Err(ApiError::bad_request(format!(
                "订阅分类 {} 缺少 qB 下载分类",
                normalized.name
            )));
        }
        if normalized.qb_save_dir_name.is_empty() {
            return Err(ApiError::bad_request(format!(
                "订阅分类 {} 缺少 qB 保存目录名",
                normalized.name
            )));
        }
        if normalized.download_dir.is_empty() {
            return Err(ApiError::bad_request(format!(
                "订阅分类 {} 缺少真实下载目录",
                normalized.name
            )));
        }
        if normalized.link_target_dir.is_empty() {
            return Err(ApiError::bad_request(format!(
                "订阅分类 {} 缺少硬链接目标目录",
                normalized.name
            )));
        }
        if !names.insert(normalized.name.clone()) {
            return Err(ApiError::bad_request(format!(
                "订阅分类名重复: {}",
                normalized.name
            )));
        }
        if !wanted_tags.insert(normalized.wanted_tag.clone()) {
            return Err(ApiError::bad_request(format!(
                "想看标签文本重复: {}",
                normalized.wanted_tag
            )));
        }
        out.push(normalized);
    }

    Ok(out)
}

fn normalize_wanted_tag_from_categories(
    raw: &str,
    categories: &[SubscriptionCategory],
) -> Result<String, ApiError> {
    if categories.is_empty() {
        return Err(ApiError::bad_request("请先在设置中配置订阅分类"));
    }
    let parts = raw
        .split_whitespace()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 1 {
        return Err(ApiError::bad_request("标记想看时必须选择一个订阅分类"));
    }
    let selected = parts[0];
    if categories
        .iter()
        .any(|category| category.wanted_tag.trim() == selected)
    {
        return Ok(selected.to_string());
    }
    Err(ApiError::bad_request(format!(
        "标记想看的标签必须来自订阅分类: {selected}"
    )))
}

async fn qb_test(Json(server): Json<QbServerEntry>) -> Result<Json<Value>, ApiError> {
    let version = qbittorrent::test_connection(&server).await?;
    Ok(Json(json!({ "ok": true, "version": version })))
}

#[derive(Deserialize)]
struct QbPushMteamBody {
    server: QbServerEntry,
    torrent_id: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    savepath: Option<String>,
}

async fn qb_push_mteam(
    State(state): State<AppState>,
    Json(body): Json<QbPushMteamBody>,
) -> Result<Json<Value>, ApiError> {
    let mteam_key = state.config.read().await.mteam_api_key.clone();
    if mteam_key.trim().is_empty() {
        return Err(ApiError::bad_request(
            "请先在设置中填写 M-Team OpenAPI Key（用于向 qB 换取可下载链接）",
        ));
    }
    let dl_url = mteam_fetch_gen_dl_url(mteam_key.trim(), &body.torrent_id).await?;
    qbittorrent::add_torrent_from_url(
        &body.server,
        &dl_url,
        body.category.as_deref(),
        body.savepath.as_deref(),
    )
    .await?;
    Ok(Json(json!({ "ok": true })))
}

async fn mteam_fetch_gen_dl_url(api_key: &str, torrent_id: &str) -> Result<String, ApiError> {
    let tid = torrent_id.trim();
    if tid.is_empty() {
        return Err(ApiError::bad_request("种子 id 为空"));
    }
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let form = reqwest::multipart::Form::new().text("id", tid.to_string());
    let resp = client
        .post("https://api.m-team.cc/api/torrent/genDlToken")
        .header("Accept", "application/json, text/plain, */*")
        .header("x-api-key", api_key.trim())
        .header("Origin", "https://kp.m-team.cc/")
        .header("Alt-Used", "api.m-team.cc")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) tmdb-mteam-hub/0.1",
        )
        .multipart(form)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("M-Team 取链请求失败: {e}")))?;
    let st = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !st.is_success() {
        return Err(ApiError::upstream(
            StatusCode::BAD_GATEWAY,
            format!("M-Team genDlToken HTTP {st}: {text}"),
        ));
    }
    let v: Value = serde_json::from_str(&text)
        .map_err(|e| ApiError::internal(format!("解析 M-Team 取链 JSON: {e}")))?;
    let code_ok = match v.get("code") {
        Some(Value::String(s)) => s == "0" || s == "200",
        Some(Value::Number(n)) => n.as_u64() == Some(0) || n.as_u64() == Some(200),
        _ => false,
    };
    if !code_ok {
        let msg = v
            .get("message")
            .or_else(|| v.get("msg"))
            .map(|x| x.to_string())
            .unwrap_or_else(|| text.clone());
        return Err(ApiError::bad_request(format!("M-Team 取链失败: {msg}")));
    }
    let Some(data) = v.get("data") else {
        return Err(ApiError::bad_request("M-Team 响应中缺少 data"));
    };
    let url = if let Some(s) = data.as_str() {
        s.trim().to_string()
    } else {
        data.to_string().trim_matches('"').to_string()
    };
    if url.is_empty() {
        return Err(ApiError::bad_request("M-Team 返回的下载地址为空"));
    }
    Ok(url)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Deserialize, Default)]
struct TmdbDetailQuery {
    #[serde(default)]
    force_refresh: bool,
}

/// TMDB 控制台里的「API 读访问令牌」是 JWT，需走 `Authorization: Bearer`；
/// 「API 密钥」则走查询参数 `api_key`（`tmdb_client` 所用方式）。
fn tmdb_uses_bearer_token(credential: &str) -> bool {
    let s = credential.trim();
    s.starts_with("eyJ") && s.contains('.')
}

async fn tmdb_v3_get(
    credential: &str,
    path: &str,
    query: &[(&str, &str)],
) -> Result<Value, ApiError> {
    const BASE: &str = "https://api.themoviedb.org/3";
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let url = format!("{BASE}{path}");
    let mut req = client.get(url).header("Accept", "application/json");
    if tmdb_uses_bearer_token(credential) {
        req = req.bearer_auth(credential.trim());
    } else {
        req = req.query(&[("api_key", credential.trim())]);
    }
    for &(k, v) in query {
        req = req.query(&[(k, v)]);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| ApiError::tmdb(format!("TMDB 网络错误: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    if !status.is_success() {
        return Err(ApiError::tmdb(format!("TMDB HTTP {status}: {text}")));
    }
    serde_json::from_str(&text).map_err(|e| ApiError::tmdb(format!("解析 TMDB JSON: {e}")))
}

fn search_results_from_tmdb_value(page: &Value, media: &str) -> Vec<Value> {
    let Some(arr) = page.get("results").and_then(|x| x.as_array()) else {
        return vec![];
    };
    let mut out = Vec::new();
    for item in arr {
        let Some(id) = item.get("id").and_then(|x| x.as_i64()) else {
            continue;
        };
        let id = id as i32;
        if media == "movie" {
            out.push(json!({
                "media_type": "movie",
                "id": id,
                "title": item.get("title"),
                "original_title": item.get("original_title"),
                "overview": item.get("overview"),
                "poster_path": item.get("poster_path").and_then(|x| x.as_str()),
                "release_date": item.get("release_date").and_then(|x| x.as_str()),
                "vote_average": item.get("vote_average"),
            }));
        } else {
            out.push(json!({
                "media_type": "tv",
                "id": id,
                "title": item.get("name"),
                "original_title": item.get("original_name"),
                "overview": item.get("overview"),
                "poster_path": item.get("poster_path").and_then(|x| x.as_str()),
                "first_air_date": item.get("first_air_date").and_then(|x| x.as_str()),
                "vote_average": item.get("vote_average"),
            }));
        }
    }
    out
}

async fn search_tmdb(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let key = state.config.read().await.tmdb_api_key.clone();
    if key.trim().is_empty() {
        return Err(ApiError::bad_request("请在设置中填写 TMDB API Key"));
    }
    if q.q.trim().is_empty() {
        return Err(ApiError::bad_request("搜索关键字不能为空"));
    }

    let lang = q.language.unwrap_or_else(|| "zh-CN".to_string());
    let query = q.q.clone();

    let (movie_items, tv_items) = if tmdb_uses_bearer_token(&key) {
        let cred = key.trim();
        let movies_page = tmdb_v3_get(
            cred,
            "/search/movie",
            &[
                ("query", query.as_str()),
                ("language", lang.as_str()),
                ("page", "1"),
                ("include_adult", "false"),
            ],
        )
        .await?;
        let tv_page = tmdb_v3_get(
            cred,
            "/search/tv",
            &[
                ("query", query.as_str()),
                ("language", lang.as_str()),
                ("page", "1"),
            ],
        )
        .await?;
        (
            search_results_from_tmdb_value(&movies_page, "movie"),
            search_results_from_tmdb_value(&tv_page, "tv"),
        )
    } else {
        let key_clone = key.clone();
        let (movies, tv) = tokio::task::spawn_blocking(move || {
            use tmdb_client::apis::client::APIClient;
            let client = APIClient::new_with_api_key(key_clone);
            let movies = client.search_api().get_search_movie_paginated(
                &query,
                None,
                None,
                Some(lang.as_str()),
                Some(1),
                Some(false),
                None,
            );
            let tv = client.search_api().get_search_tv_paginated(
                &query,
                None,
                Some(lang.as_str()),
                Some(1),
            );
            (movies, tv)
        })
        .await
        .map_err(|e| ApiError::internal(format!("搜索任务失败: {e}")))?;

        let movies = movies.map_err(|e| ApiError::tmdb(e.to_string()))?;
        let tv = tv.map_err(|e| ApiError::tmdb(e.to_string()))?;

        let movie_items: Vec<Value> = movies
            .results
            .unwrap_or_default()
            .into_iter()
            .filter_map(|m| {
                let id = m.id?;
                Some(json!({
                    "media_type": "movie",
                    "id": id,
                    "title": m.title,
                    "original_title": m.original_title,
                    "overview": m.overview,
                    "poster_path": m.poster_path,
                    "release_date": m.release_date,
                    "vote_average": m.vote_average,
                }))
            })
            .collect();

        let tv_items: Vec<Value> = tv
            .results
            .unwrap_or_default()
            .into_iter()
            .filter_map(|t| {
                let id = t.id?;
                Some(json!({
                    "media_type": "tv",
                    "id": id,
                    "title": t.name,
                    "original_title": t.original_name,
                    "overview": t.overview,
                    "poster_path": t.poster_path,
                    "first_air_date": t.first_air_date,
                    "vote_average": t.vote_average,
                }))
            })
            .collect();
        (movie_items, tv_items)
    };

    Ok(Json(json!({ "movies": movie_items, "tv": tv_items })))
}

#[derive(Deserialize)]
struct DoubanSearchQuery {
    q: String,
    #[serde(default = "default_douban_limit")]
    limit: usize,
}

fn default_douban_limit() -> usize {
    20
}

#[derive(Deserialize)]
struct DoubanLibraryQuery {
    #[serde(default)]
    force_refresh: bool,
    #[serde(default = "default_douban_library_limit")]
    limit: usize,
}

fn default_douban_library_limit() -> usize {
    200
}

#[derive(Deserialize)]
struct DoubanTagHistoryQuery {
    #[serde(default = "default_douban_tag_history_limit")]
    limit: usize,
}

fn default_douban_tag_history_limit() -> usize {
    80
}

async fn douban_search(
    State(state): State<AppState>,
    Query(q): Query<DoubanSearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let cookie = state.config.read().await.douban_cookie.clone();
    let limit = q.limit.clamp(1, 50);
    let items = douban::search(&cookie, &q.q, limit)
        .await
        .map_err(ApiError::douban)?;
    let items_value =
        serde_json::to_value(&items).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(json!({
        "items": items_value.clone(),
        "movies": items_value,
        "tv": [],
    })))
}

async fn douban_library(
    State(state): State<AppState>,
    Query(q): Query<DoubanLibraryQuery>,
) -> Result<Json<Value>, ApiError> {
    let cookie = state.config.read().await.douban_cookie.clone();
    let account_key = douban::auth_cache_key_fragment(&cookie).map_err(ApiError::douban)?;
    let limit = q.limit.clamp(1, 1200);
    let cache_key = format!("library_{account_key}_limit_{limit}");

    if !q.force_refresh {
        if let Some(mut cached) = state.douban_cache.get(&cache_key).await {
            mark_cache_hit(&mut cached);
            return Ok(Json(cached));
        }
    }

    let (wish, collect) = tokio::try_join!(
        douban::library(&cookie, douban::DoubanLibraryStatus::Wish, limit),
        douban::library(&cookie, douban::DoubanLibraryStatus::Collect, limit),
    )
    .map_err(ApiError::douban)?;

    let value = json!({
        "source": "douban",
        "cached": false,
        "fetched_at": unix_now_secs(),
        "ttl_seconds": state.douban_cache_ttl_secs,
        "limit": limit,
        "wish": wish,
        "collect": collect,
    });
    if let Err(e) = state.douban_cache.put(&cache_key, &value).await {
        tracing::warn!("douban library cache write failed: {e}");
    }
    Ok(Json(value))
}

async fn douban_tag_history(
    State(state): State<AppState>,
    Query(q): Query<DoubanTagHistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    let cfg = state.config.read().await.clone();
    let cookie = cfg.douban_cookie.clone();
    let account_key = douban::auth_cache_key_fragment(&cookie).map_err(ApiError::douban)?;
    let limit = q.limit.clamp(1, 1200);
    let mut value = load_douban_tag_history_value(&state, &account_key).await;
    constrain_douban_tag_history(&mut value, &cfg.subscription_categories, limit);
    Ok(Json(value))
}

fn mark_cache_hit(value: &mut Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert("cached".to_string(), Value::Bool(true));
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

async fn douban_subject_detail(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
) -> Result<Json<Value>, ApiError> {
    let cookie = state.config.read().await.douban_cookie.clone();
    let detail = douban::subject_detail(&cookie, &id)
        .await
        .map_err(ApiError::douban)?;
    Ok(Json(
        serde_json::to_value(detail).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

#[derive(Deserialize)]
struct DoubanMarkInterestBody {
    interest: douban::DoubanInterest,
    #[serde(default)]
    rating: Option<u8>,
    #[serde(default)]
    tags: String,
}

async fn douban_mark_interest(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    Json(body): Json<DoubanMarkInterestBody>,
) -> Result<Json<Value>, ApiError> {
    let cfg = state.config.read().await.clone();
    let cookie = cfg.douban_cookie.clone();
    let account_key = douban::auth_cache_key_fragment(&cookie).map_err(ApiError::douban)?;
    let tags = if matches!(body.interest, douban::DoubanInterest::Wish) {
        normalize_wanted_tag_from_categories(&body.tags, &cfg.subscription_categories)?
    } else {
        body.tags
    };
    let result = douban::mark_interest(&cookie, &id, body.interest, body.rating, &tags)
        .await
        .map_err(ApiError::douban)?;
    if let Err(e) = state
        .douban_cache
        .remove_prefix(&format!("library_{account_key}_"))
        .await
    {
        tracing::warn!("douban library cache invalidation failed: {e}");
    }
    if let Err(e) = update_douban_tag_history(&state, &account_key, &result.tags).await {
        tracing::warn!("douban tag history update failed: {e}");
    }
    Ok(Json(
        serde_json::to_value(result).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

fn douban_tag_history_cache_key(account_key: &str) -> String {
    format!("tag_history_manual_{account_key}")
}

async fn load_douban_tag_history_value(state: &AppState, account_key: &str) -> Value {
    let key = douban_tag_history_cache_key(account_key);
    state.douban_cache.get_any(&key).await.unwrap_or_else(|| {
        json!({
            "source": "local-cache",
            "cached": true,
            "updated_at": null,
            "tags": [],
            "tag_counts": [],
        })
    })
}

fn truncate_douban_tag_history(value: &mut Value, limit: usize) {
    if let Some(tags) = value.get_mut("tags").and_then(|v| v.as_array_mut()) {
        tags.truncate(limit);
    }
    if let Some(tag_counts) = value.get_mut("tag_counts").and_then(|v| v.as_array_mut()) {
        tag_counts.truncate(limit);
    }
    if let Some(obj) = value.as_object_mut() {
        obj.insert("cached".to_string(), Value::Bool(true));
    }
}

fn constrain_douban_tag_history(
    value: &mut Value,
    categories: &[SubscriptionCategory],
    limit: usize,
) {
    if categories.is_empty() {
        truncate_douban_tag_history(value, 0);
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "subscription_categories".to_string(),
                Value::Array(Vec::new()),
            );
        }
        return;
    }

    let mut counts = HashMap::<String, u64>::new();
    if let Some(tag_counts) = value.get("tag_counts").and_then(|v| v.as_array()) {
        for item in tag_counts {
            let Some(tag) = item.get("tag").and_then(|v| v.as_str()) else {
                continue;
            };
            let count = item.get("count").and_then(|v| v.as_u64()).unwrap_or(1);
            counts.insert(tag.trim().to_string(), count.max(1));
        }
    }

    let mut rows = categories
        .iter()
        .filter_map(|category| {
            let tag = category.wanted_tag.trim();
            if tag.is_empty() {
                return None;
            }
            Some((
                tag.to_string(),
                counts.get(tag).copied().unwrap_or(0),
                category.name.trim().to_string(),
            ))
        })
        .collect::<Vec<_>>();
    rows.sort_by(|(tag_a, count_a, name_a), (tag_b, count_b, name_b)| {
        count_b
            .cmp(count_a)
            .then_with(|| name_a.cmp(name_b))
            .then_with(|| tag_a.cmp(tag_b))
    });
    rows.truncate(limit);

    let tags = rows
        .iter()
        .map(|(tag, _, _)| Value::String(tag.clone()))
        .collect::<Vec<_>>();
    let tag_counts = rows
        .iter()
        .map(|(tag, count, name)| json!({ "tag": tag, "count": count, "category": name }))
        .collect::<Vec<_>>();
    let categories_value =
        serde_json::to_value(categories).unwrap_or_else(|_| Value::Array(vec![]));

    if let Some(obj) = value.as_object_mut() {
        obj.insert("cached".to_string(), Value::Bool(true));
        obj.insert("tags".to_string(), Value::Array(tags));
        obj.insert("tag_counts".to_string(), Value::Array(tag_counts));
        obj.insert("subscription_categories".to_string(), categories_value);
    }
}

async fn update_douban_tag_history(
    state: &AppState,
    account_key: &str,
    tags_text: &str,
) -> std::io::Result<()> {
    let tags = tags_text
        .split_whitespace()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    if tags.is_empty() {
        return Ok(());
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    let current = load_douban_tag_history_value(state, account_key).await;
    if let Some(items) = current.get("tag_counts").and_then(|v| v.as_array()) {
        for item in items {
            let Some(tag) = item.get("tag").and_then(|v| v.as_str()) else {
                continue;
            };
            let count = item.get("count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if !tag.trim().is_empty() {
                counts.insert(tag.trim().to_string(), count.max(1));
            }
        }
    } else if let Some(items) = current.get("tags").and_then(|v| v.as_array()) {
        for tag in items.iter().filter_map(|v| v.as_str()) {
            if !tag.trim().is_empty() {
                counts.entry(tag.trim().to_string()).or_insert(1);
            }
        }
    }

    for tag in tags {
        *counts.entry(tag.to_string()).or_default() += 1;
    }

    let mut ranked = counts.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|(tag_a, count_a), (tag_b, count_b)| {
        count_b.cmp(count_a).then_with(|| tag_a.cmp(tag_b))
    });

    let tags = ranked
        .iter()
        .map(|(tag, _)| tag.clone())
        .collect::<Vec<_>>();
    let tag_counts = ranked
        .iter()
        .map(|(tag, count)| json!({ "tag": tag, "count": count }))
        .collect::<Vec<_>>();

    let value = json!({
        "source": "local-cache",
        "cached": true,
        "updated_at": unix_now_secs(),
        "tags": tags,
        "tag_counts": tag_counts,
    });
    state
        .douban_cache
        .put(&douban_tag_history_cache_key(account_key), &value)
        .await
}

#[derive(Deserialize)]
struct DoubanImageQuery {
    url: String,
}

async fn douban_image(Query(q): Query<DoubanImageQuery>) -> Result<Response, ApiError> {
    let (content_type, bytes) = douban::fetch_image(&q.url)
        .await
        .map_err(ApiError::douban)?;
    let mut headers = HeaderMap::new();
    let content_type = HeaderValue::from_str(&content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("image/jpeg"));
    headers.insert(header::CONTENT_TYPE, content_type);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=86400"),
    );
    Ok((headers, axum::body::Body::from(bytes)).into_response())
}

async fn douban_qr_start(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let (session_id, session, result) = douban::qr_start().await.map_err(ApiError::douban)?;
    state
        .douban_qr_sessions
        .write()
        .await
        .insert(session_id, session);
    Ok(Json(
        serde_json::to_value(result).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

#[derive(Deserialize)]
struct DoubanQrQuery {
    session_id: String,
}

async fn douban_qr_image(
    State(state): State<AppState>,
    Query(q): Query<DoubanQrQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let image = state
        .douban_qr_sessions
        .read()
        .await
        .get(q.session_id.trim())
        .map(|s| s.image.clone())
        .ok_or_else(|| ApiError::bad_request("豆瓣 QR 登录会话不存在或已过期"))?;
    Ok((
        [(header::CONTENT_TYPE, "image/png")],
        image.as_ref().clone(),
    ))
}

async fn douban_qr_poll(
    State(state): State<AppState>,
    Query(q): Query<DoubanQrQuery>,
) -> Result<Json<Value>, ApiError> {
    let session_id = q.session_id.trim().to_string();
    let session = state
        .douban_qr_sessions
        .read()
        .await
        .get(&session_id)
        .cloned()
        .ok_or_else(|| ApiError::bad_request("豆瓣 QR 登录会话不存在或已过期"))?;

    let result = douban::qr_poll(&session).await.map_err(ApiError::douban)?;
    if let Some(cookie_header) = result.cookie_header.clone() {
        let mut new_cfg = state.config.read().await.clone();
        new_cfg.douban_cookie = douban::normalize_cookie_header(&cookie_header);
        new_cfg
            .save(&state.config_path)
            .map_err(|e| ApiError::internal(format!("写入豆瓣 Cookie 失败: {e}")))?;
        *state.config.write().await = new_cfg;
        state.douban_qr_sessions.write().await.remove(&session_id);
    }

    Ok(Json(
        serde_json::to_value(result).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn tmdb_movie_detail(
    State(state): State<AppState>,
    PathParam(id): PathParam<i32>,
    Query(dq): Query<TmdbDetailQuery>,
) -> Result<Json<Value>, ApiError> {
    let key = state.config.read().await.tmdb_api_key.clone();
    if key.trim().is_empty() {
        return Err(ApiError::bad_request("请在设置中填写 TMDB API Key"));
    }

    let cache_key = format!("movie_{id}");
    if !dq.force_refresh {
        if let Some(mut v) = state.tmdb_cache.get(&cache_key).await {
            enrich_posters(&mut v);
            return Ok(Json(v));
        }
    }

    let mut v = if tmdb_uses_bearer_token(&key) {
        tmdb_v3_get(
            key.trim(),
            &format!("/movie/{id}"),
            &[
                ("language", "zh-CN"),
                ("append_to_response", "external_ids"),
            ],
        )
        .await?
    } else {
        let detail = tokio::task::spawn_blocking(move || {
            use tmdb_client::apis::client::APIClient;
            let client = APIClient::new_with_api_key(key);
            client
                .movies_api()
                .get_movie_details(id, Some("zh-CN"), None, Some("external_ids"))
        })
        .await
        .map_err(|e| ApiError::internal(format!("TMDB 请求失败: {e}")))?
        .map_err(|e| ApiError::tmdb(e.to_string()))?;
        serde_json::to_value(&detail).map_err(|e| ApiError::internal(e.to_string()))?
    };
    enrich_posters(&mut v);
    if let Err(e) = state.tmdb_cache.put(&cache_key, &v).await {
        tracing::warn!("tmdb movie cache write failed: {e}");
    }
    Ok(Json(v))
}

async fn tmdb_tv_detail(
    State(state): State<AppState>,
    PathParam(id): PathParam<i32>,
    Query(dq): Query<TmdbDetailQuery>,
) -> Result<Json<Value>, ApiError> {
    let key = state.config.read().await.tmdb_api_key.clone();
    if key.trim().is_empty() {
        return Err(ApiError::bad_request("请在设置中填写 TMDB API Key"));
    }

    let cache_key = format!("tv_{id}");
    if !dq.force_refresh {
        if let Some(mut v) = state.tmdb_cache.get(&cache_key).await {
            enrich_posters(&mut v);
            return Ok(Json(v));
        }
    }

    let mut v = if tmdb_uses_bearer_token(&key) {
        tmdb_v3_get(
            key.trim(),
            &format!("/tv/{id}"),
            &[
                ("language", "zh-CN"),
                ("append_to_response", "external_ids"),
            ],
        )
        .await?
    } else {
        let detail = tokio::task::spawn_blocking(move || {
            use tmdb_client::apis::client::APIClient;
            let client = APIClient::new_with_api_key(key);
            client
                .tv_api()
                .get_tv_details(id, Some("zh-CN"), None, Some("external_ids"))
        })
        .await
        .map_err(|e| ApiError::internal(format!("TMDB 请求失败: {e}")))?
        .map_err(|e| ApiError::tmdb(e.to_string()))?;
        serde_json::to_value(&detail).map_err(|e| ApiError::internal(e.to_string()))?
    };
    enrich_posters(&mut v);
    if let Err(e) = state.tmdb_cache.put(&cache_key, &v).await {
        tracing::warn!("tmdb tv cache write failed: {e}");
    }
    Ok(Json(v))
}

async fn tmdb_tv_season_detail(
    State(state): State<AppState>,
    PathParam((tv_id, season_number)): PathParam<(i32, i32)>,
    Query(dq): Query<TmdbDetailQuery>,
) -> Result<Json<Value>, ApiError> {
    let key = state.config.read().await.tmdb_api_key.clone();
    if key.trim().is_empty() {
        return Err(ApiError::bad_request("请在设置中填写 TMDB API Key"));
    }

    let cache_key = format!("tv_{tv_id}_s{season_number}");
    if !dq.force_refresh {
        if let Some(v) = state.tmdb_cache.get(&cache_key).await {
            return Ok(Json(v));
        }
    }

    let mut v = if tmdb_uses_bearer_token(&key) {
        tmdb_v3_get(
            key.trim(),
            &format!("/tv/{tv_id}/season/{season_number}"),
            &[("language", "zh-CN")],
        )
        .await?
    } else {
        let detail = tokio::task::spawn_blocking(move || {
            use tmdb_client::apis::client::APIClient;
            let client = APIClient::new_with_api_key(key);
            client.tv_seasons_api().get_tv_season_details(
                tv_id,
                season_number,
                Some("zh-CN"),
                None,
                None,
            )
        })
        .await
        .map_err(|e| ApiError::internal(format!("TMDB 请求失败: {e}")))?
        .map_err(|e| ApiError::tmdb(e.to_string()))?;
        serde_json::to_value(&detail).map_err(|e| ApiError::internal(e.to_string()))?
    };
    enrich_season_episode_stills(&mut v);
    if let Err(e) = state.tmdb_cache.put(&cache_key, &v).await {
        tracing::warn!("tmdb tv season cache write failed: {e}");
    }
    Ok(Json(v))
}

fn enrich_season_episode_stills(v: &mut Value) {
    const STILL: &str = "https://image.tmdb.org/t/p/w185";
    let Some(eps) = v.get_mut("episodes").and_then(|x| x.as_array_mut()) else {
        return;
    };
    for ep in eps {
        let Some(obj) = ep.as_object_mut() else {
            continue;
        };
        if let Some(p) = obj
            .get("still_path")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
        {
            obj.insert(
                "still_url".to_string(),
                Value::String(format!("{STILL}{p}")),
            );
        }
    }
}

fn enrich_posters(v: &mut Value) {
    const BASE: &str = "https://image.tmdb.org/t/p/w500";
    let poster = v
        .get("poster_path")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("{BASE}{s}"));
    let backdrop = v
        .get("backdrop_path")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("{BASE}{s}"));
    if let Some(obj) = v.as_object_mut() {
        if let Some(u) = poster {
            obj.insert("poster_url".to_string(), Value::String(u));
        }
        if let Some(u) = backdrop {
            obj.insert("backdrop_url".to_string(), Value::String(u));
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum MteamSource {
    Imdb,
    Douban,
    Keyword,
}

#[derive(Deserialize)]
struct MteamQuery {
    /// 只查询单一路径，与前端标签对应：imdb / douban / keyword。
    source: MteamSource,
    #[serde(default)]
    imdb_id: Option<String>,
    #[serde(default)]
    douban_id: Option<String>,
    /// source=keyword 时使用（建议 TMDB 英文 / 原标题）。
    #[serde(default)]
    keyword: Option<String>,
    #[serde(default = "default_page")]
    page: u32,
    #[serde(default = "default_page_size")]
    page_size: u32,
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    50
}

async fn mteam_search(
    State(state): State<AppState>,
    Query(q): Query<MteamQuery>,
) -> Result<Json<Value>, ApiError> {
    let key = state.config.read().await.mteam_api_key.clone();
    if key.trim().is_empty() {
        return Err(ApiError::bad_request(
            "请在设置中填写 M-Team API Key（控制面板中的 OpenAPI 密钥）",
        ));
    }

    let imdb_raw = q
        .imdb_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let douban_raw = q
        .douban_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let keyword_raw = q
        .keyword
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let out = match q.source {
        MteamSource::Imdb => {
            let s = imdb_raw
                .ok_or_else(|| ApiError::bad_request("使用 IMDb 路径时请提供有效的 imdb_id"))?;
            let body = json!({
                "pageNumber": q.page,
                "pageSize": q.page_size,
                "imdb": normalize_imdb_url(s),
            });
            mteam_search_post(&client, &key, &body).await?
        }
        MteamSource::Douban => {
            let s = douban_raw
                .ok_or_else(|| ApiError::bad_request("使用豆瓣路径时请提供有效的 douban_id"))?;
            let body = json!({
                "pageNumber": q.page,
                "pageSize": q.page_size,
                "douban": normalize_douban_url(s),
            });
            mteam_search_post(&client, &key, &body).await?
        }
        MteamSource::Keyword => {
            let k = keyword_raw
                .ok_or_else(|| ApiError::bad_request("使用关键字路径时请提供 keyword"))?;
            let body = json!({
                "pageNumber": q.page,
                "pageSize": q.page_size,
                "keyword": k,
            });
            mteam_search_post(&client, &key, &body).await?
        }
    };

    Ok(Json(out))
}

async fn mteam_search_post(
    client: &reqwest::Client,
    api_key: &str,
    body: &Value,
) -> Result<Value, ApiError> {
    let resp = client
        .post("https://api.m-team.cc/api/torrent/search")
        .header("Accept", "application/json, text/plain, */*")
        .header("Content-Type", "application/json")
        .header("x-api-key", api_key)
        .header("Origin", "https://kp.m-team.cc/")
        .header("Alt-Used", "api.m-team.cc")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) tmdb-mteam-hub/0.1",
        )
        .json(body)
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("M-Team 请求失败: {e}")))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if !status.is_success() {
        return Err(ApiError::Other {
            status: StatusCode::BAD_GATEWAY,
            message: format!("M-Team HTTP {status}: {text}"),
        });
    }

    serde_json::from_str(&text).map_err(|e| ApiError::internal(format!("解析 M-Team 响应: {e}")))
}

fn normalize_imdb_url(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        return s.to_string();
    }
    let id = if s.starts_with("tt") {
        s.to_string()
    } else {
        format!("tt{s}")
    };
    format!("https://www.imdb.com/title/{id}/")
}

fn normalize_douban_url(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("http://") || s.starts_with("https://") {
        let u = s.trim_end_matches('/').trim();
        return format!("{u}/");
    }
    let tail = s
        .rsplit('/')
        .next()
        .unwrap_or(s)
        .trim()
        .trim_start_matches("subject/");
    format!("https://movie.douban.com/subject/{tail}/")
}

#[cfg(test)]
mod subscription_category_tests {
    use super::*;

    fn category(name: &str, wanted_tag: &str) -> SubscriptionCategory {
        SubscriptionCategory {
            name: name.to_string(),
            wanted_tag: wanted_tag.to_string(),
            qb_category: format!("qb-{name}"),
            qb_save_dir_name: format!("save-{name}"),
            download_dir: format!("/downloads/{name}"),
            link_target_dir: format!("/media/{name}"),
        }
    }

    #[test]
    fn subscription_categories_reject_duplicate_wanted_tags() {
        let res = normalize_subscription_categories(vec![
            category("电影", "影视"),
            category("剧集", "影视"),
        ]);
        assert!(matches!(res, Err(ApiError::BadRequest { .. })));
    }

    #[test]
    fn wanted_tag_must_match_one_configured_category() {
        let categories = vec![category("电影", "电影"), category("剧集", "剧集")];
        let Ok(selected) = normalize_wanted_tag_from_categories("电影", &categories) else {
            panic!("configured wanted tag should be accepted");
        };
        assert_eq!(selected, "电影");
        assert!(matches!(
            normalize_wanted_tag_from_categories("电影 剧集", &categories),
            Err(ApiError::BadRequest { .. })
        ));
        assert!(matches!(
            normalize_wanted_tag_from_categories("纪录片", &categories),
            Err(ApiError::BadRequest { .. })
        ));
    }

    #[test]
    fn douban_tag_history_is_constrained_to_subscription_categories() {
        let mut value = json!({
            "source": "local-cache",
            "cached": true,
            "tags": ["外部"],
            "tag_counts": [
                { "tag": "剧集", "count": 5 },
                { "tag": "外部", "count": 99 },
                { "tag": "电影", "count": 2 }
            ],
        });
        constrain_douban_tag_history(
            &mut value,
            &[category("电影", "电影"), category("剧集", "剧集")],
            10,
        );
        let tags = value
            .get("tags")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(tags, vec!["剧集", "电影"]);
    }
}

pub(crate) enum ApiError {
    BadRequest { message: String },
    Other { status: StatusCode, message: String },
}

impl ApiError {
    fn bad_request(msg: impl Into<String>) -> Self {
        Self::BadRequest {
            message: msg.into(),
        }
    }

    fn upstream(status: StatusCode, msg: impl Into<String>) -> Self {
        Self::Other {
            status,
            message: msg.into(),
        }
    }

    fn internal(msg: impl Into<String>) -> Self {
        Self::Other {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: msg.into(),
        }
    }

    fn tmdb(msg: impl Into<String>) -> Self {
        Self::Other {
            status: StatusCode::BAD_GATEWAY,
            message: msg.into(),
        }
    }

    fn douban(err: douban::DoubanError) -> Self {
        Self::Other {
            status: err.status,
            message: err.message,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, msg) = match self {
            ApiError::BadRequest { message } => (StatusCode::BAD_REQUEST, message),
            ApiError::Other { status, message } => (status, message),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}
