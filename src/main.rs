mod config;
mod douban;
mod qbittorrent;
mod subscription;
mod tmdb_cache;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path as PathParam, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use config::{FileConfig, QbServerEntry, SubscriptionCategory, TorrentMatchRule};
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
    wanted_store: subscription::WantedSubscriptionStore,
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

    let subscription_state_dir = resolve_subscription_state_dir();
    let wanted_store = subscription::WantedSubscriptionStore::new(subscription_state_dir.clone());
    tracing::info!(
        "subscription state: dir={}",
        subscription_state_dir.display()
    );

    let state = AppState {
        config_path,
        config: std::sync::Arc::new(RwLock::new(file_cfg)),
        tmdb_cache,
        douban_cache,
        douban_cache_ttl_secs,
        douban_qr_sessions: std::sync::Arc::new(RwLock::new(HashMap::new())),
        wanted_store,
    };
    spawn_wanted_watch_loop(state.clone());

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
        .route("/subscriptions/wanted", get(wanted_subscription_state))
        .route("/subscriptions/wanted/poll", post(wanted_subscription_poll))
        .route(
            "/subscriptions/wanted/{id}/candidates",
            post(wanted_subscription_candidates),
        )
        .route(
            "/subscriptions/wanted/{id}/push",
            post(wanted_subscription_push),
        )
        .route(
            "/subscriptions/wanted/{id}/completion",
            post(wanted_subscription_completion),
        )
        .route(
            "/subscriptions/wanted/{id}/status",
            post(wanted_subscription_status),
        )
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

fn resolve_subscription_state_dir() -> PathBuf {
    std::env::var("SUBSCRIPTION_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| cwd_or_dot().join("cache").join("subscriptions"))
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
        "subscription_watcher": cfg.subscription_watcher,
        "torrent_match_rules": cfg.torrent_match_rules,
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
    #[serde(default)]
    subscription_watcher: Option<config::SubscriptionWatcherConfig>,
    #[serde(default)]
    torrent_match_rules: Option<Vec<TorrentMatchRule>>,
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
    if let Some(subscription_watcher) = body.subscription_watcher {
        new_cfg.subscription_watcher = normalize_subscription_watcher(subscription_watcher);
    }
    if let Some(torrent_match_rules) = body.torrent_match_rules {
        new_cfg.torrent_match_rules = normalize_torrent_match_rules(torrent_match_rules)?;
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

fn normalize_subscription_watcher(
    mut cfg: config::SubscriptionWatcherConfig,
) -> config::SubscriptionWatcherConfig {
    cfg.poll_interval_secs = cfg.poll_interval_secs.clamp(60, 86_400);
    cfg.library_limit = cfg.library_limit.clamp(1, 1200);
    cfg.max_retries = cfg.max_retries.clamp(1, 20);
    cfg
}

fn normalize_torrent_match_rules(
    rules: Vec<TorrentMatchRule>,
) -> Result<Vec<TorrentMatchRule>, ApiError> {
    let mut out = Vec::new();
    let mut names = HashSet::new();
    for (idx, rule) in rules.into_iter().enumerate() {
        let n = idx + 1;
        let normalized = TorrentMatchRule {
            name: rule.name.trim().to_string(),
            priority: rule.priority,
            mode: rule.mode,
            title_keywords: normalize_keyword_list(rule.title_keywords),
            resolution_keywords: normalize_keyword_list(rule.resolution_keywords),
            source_keywords: normalize_keyword_list(rule.source_keywords),
        };
        if normalized.name.is_empty() {
            return Err(ApiError::bad_request(format!(
                "种子匹配规则 #{n} 缺少规则名"
            )));
        }
        if !names.insert(normalized.name.clone()) {
            return Err(ApiError::bad_request(format!(
                "种子匹配规则名重复: {}",
                normalized.name
            )));
        }
        if normalized.title_keywords.is_empty()
            && normalized.resolution_keywords.is_empty()
            && normalized.source_keywords.is_empty()
        {
            return Err(ApiError::bad_request(format!(
                "种子匹配规则 {} 至少需要一个关键词",
                normalized.name
            )));
        }
        out.push(normalized);
    }
    out.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

fn normalize_keyword_list(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let item = value.trim();
        if !item.is_empty() && !out.iter().any(|existing| existing == item) {
            out.push(item.to_string());
        }
    }
    out
}

async fn wanted_subscription_state(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let cookie = state.config.read().await.douban_cookie.clone();
    let account_key = douban::auth_cache_key_fragment(&cookie).map_err(ApiError::douban)?;
    let snapshot = state
        .wanted_store
        .snapshot(&account_key, unix_now_secs())
        .await
        .map_err(|e| ApiError::internal(format!("读取想看订阅状态失败: {e}")))?;
    Ok(Json(
        serde_json::to_value(snapshot).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn wanted_subscription_poll(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let outcome = run_wanted_watch_poll(&state).await?;
    Ok(Json(
        serde_json::to_value(outcome).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn wanted_subscription_status(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    Json(body): Json<subscription::WantedStatusUpdate>,
) -> Result<Json<Value>, ApiError> {
    let cfg = state.config.read().await.clone();
    let account_key =
        douban::auth_cache_key_fragment(&cfg.douban_cookie).map_err(ApiError::douban)?;
    let outcome = state
        .wanted_store
        .update_status(
            &account_key,
            &id,
            body,
            cfg.subscription_watcher.max_retries,
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("更新想看订阅状态失败: {e}")))?;
    let Some(outcome) = outcome else {
        return Err(ApiError::bad_request("订阅记录不存在"));
    };
    Ok(Json(
        serde_json::to_value(outcome).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

#[derive(Deserialize, Default)]
struct WantedCandidateBody {
    #[serde(default)]
    page_size: Option<u32>,
}

#[derive(Deserialize, Default)]
struct WantedPushBody {
    #[serde(default)]
    qb_server_name: Option<String>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    page_size: Option<u32>,
}

#[derive(Deserialize, Default)]
struct WantedCompletionBody {
    #[serde(default)]
    qb_server_name: Option<String>,
    #[serde(default)]
    qb_hash: Option<String>,
    #[serde(default)]
    force: bool,
    #[serde(default)]
    dry_run: bool,
}

async fn wanted_subscription_candidates(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    Json(body): Json<WantedCandidateBody>,
) -> Result<Json<Value>, ApiError> {
    let (cfg, account_key, record) = load_wanted_record_context(&state, &id).await?;
    let _category = category_for_wanted_record(&record, &cfg.subscription_categories)?;
    let candidates =
        search_mteam_candidates_for_record(&cfg.mteam_api_key, &record, body.page_size).await?;
    let matches = match_torrent_candidates(&candidates, &cfg.torrent_match_rules);
    let record = state
        .wanted_store
        .update_candidate_matches(
            &account_key,
            &record.subject_id,
            matches.clone(),
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入候选种子记录失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;

    Ok(Json(json!({
        "subscription_id": record.subject_id,
        "candidate_count": candidates.len(),
        "selected": matches.iter().find(|item| item.selected),
        "matches": matches,
        "record": record,
    })))
}

async fn wanted_subscription_push(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    Json(body): Json<WantedPushBody>,
) -> Result<Json<Value>, ApiError> {
    let (cfg, account_key, record) = load_wanted_record_context(&state, &id).await?;
    if !body.force
        && matches!(
            record.status,
            subscription::WantedSubscriptionStatus::Pushed
                | subscription::WantedSubscriptionStatus::Completed
                | subscription::WantedSubscriptionStatus::Processing
        )
    {
        return Err(ApiError::bad_request(format!(
            "订阅 {} 当前状态为 {:?}，不会重复推送；需要重试请传 force=true",
            record.subject_id, record.status
        )));
    }
    let category = category_for_wanted_record(&record, &cfg.subscription_categories)?.clone();
    let qb_server = select_qb_server(&cfg.qb_servers, body.qb_server_name.as_deref())?;
    let candidates =
        search_mteam_candidates_for_record(&cfg.mteam_api_key, &record, body.page_size).await?;
    let matches = match_torrent_candidates(&candidates, &cfg.torrent_match_rules);
    let selected = matches.iter().find(|item| item.selected).cloned();
    let now = unix_now_secs();
    state
        .wanted_store
        .update_candidate_matches(&account_key, &record.subject_id, matches.clone(), now)
        .await
        .map_err(|e| ApiError::internal(format!("写入候选种子记录失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;

    let Some(selected) = selected else {
        let error = if candidates.is_empty() {
            "未搜索到候选种子".to_string()
        } else {
            "没有候选种子匹配当前规则".to_string()
        };
        record_push_failure(
            &state,
            &account_key,
            &record.subject_id,
            &qb_server,
            &category,
            None,
            error.clone(),
        )
        .await?;
        return Err(ApiError::bad_request(error));
    };

    let processing = subscription::WantedStatusUpdate {
        status: subscription::WantedSubscriptionStatus::Processing,
        error: None,
        skip_reason: None,
    };
    state
        .wanted_store
        .update_status(
            &account_key,
            &record.subject_id,
            processing,
            cfg.subscription_watcher.max_retries,
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("更新订阅处理状态失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;

    let dl_url =
        match mteam_fetch_gen_dl_url(&cfg.mteam_api_key, &selected.candidate.torrent_id).await {
            Ok(url) => url,
            Err(e) => {
                record_push_failure(
                    &state,
                    &account_key,
                    &record.subject_id,
                    &qb_server,
                    &category,
                    Some(&selected),
                    e.message().to_string(),
                )
                .await?;
                return Err(e);
            }
        };

    if let Err(e) = qbittorrent::add_torrent_from_url(
        &qb_server,
        &dl_url,
        Some(&category.qb_category),
        Some(&category.qb_save_dir_name),
    )
    .await
    {
        record_push_failure(
            &state,
            &account_key,
            &record.subject_id,
            &qb_server,
            &category,
            Some(&selected),
            e.message().to_string(),
        )
        .await?;
        return Err(e);
    }

    let push = torrent_push_record(
        &record.subject_id,
        &qb_server,
        &category,
        &selected.candidate,
        "pushed",
        Some(unix_now_secs()),
        None,
    );
    let record = state
        .wanted_store
        .update_push_record(
            &account_key,
            &record.subject_id,
            push.clone(),
            subscription::WantedSubscriptionStatus::Pushed,
            None,
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入 qB 推送记录失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;

    Ok(Json(json!({
        "ok": true,
        "subscription_id": record.subject_id,
        "selected": selected,
        "push": push,
        "record": record,
    })))
}

async fn wanted_subscription_completion(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    Json(body): Json<WantedCompletionBody>,
) -> Result<Json<Value>, ApiError> {
    let (cfg, account_key, record) = load_wanted_record_context(&state, &id).await?;
    if matches!(
        record.status,
        subscription::WantedSubscriptionStatus::Completed
    ) && !body.force
    {
        return Ok(Json(json!({
            "ok": true,
            "already_completed": true,
            "record": record,
        })));
    }

    let category = category_for_wanted_record(&record, &cfg.subscription_categories)?.clone();
    let mut push = record
        .last_push
        .clone()
        .ok_or_else(|| ApiError::bad_request("订阅记录缺少 qB pushed record"))?;
    if push.status == "failed" && !body.force {
        return Err(ApiError::bad_request(
            "最后一次 qB push 已失败；需要重新检查请传 force=true",
        ));
    }
    if push.torrent_id.trim().is_empty() && body.qb_hash.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err(ApiError::bad_request(
            "pushed record 缺少种子 id，且未提供 qb_hash",
        ));
    }

    let qb_server = select_qb_server(
        &cfg.qb_servers,
        body.qb_server_name
            .as_deref()
            .or(Some(push.qb_server.as_str())),
    )?;
    let torrents = qbittorrent::list_torrents(&qb_server, Some(&push.qb_category)).await?;
    let qb_torrent = select_qb_torrent(&torrents, &push, body.qb_hash.as_deref())?;
    let now = unix_now_secs();

    push.checked_at = Some(now);
    push.qb_hash = Some(qb_torrent.hash.clone());
    push.qb_name = Some(qb_torrent.name.clone());

    if !qb_torrent.is_complete() {
        let completion = subscription::HardlinkCompletionRecord {
            status: "pending".to_string(),
            checked_at: now,
            completed_at: None,
            qb_hash: Some(qb_torrent.hash.clone()),
            qb_name: Some(qb_torrent.name.clone()),
            source_path: None,
            target_dir: None,
            linked_files: Vec::new(),
            error: None,
        };
        if !body.dry_run {
            let record = state
                .wanted_store
                .update_completion_record(
                    &account_key,
                    &record.subject_id,
                    push.clone(),
                    completion.clone(),
                    subscription::WantedSubscriptionStatus::Pushed,
                    None,
                    now,
                )
                .await
                .map_err(|e| ApiError::internal(format!("写入 qB 完成检查记录失败: {e}")))?
                .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
            return Ok(Json(json!({
                "ok": true,
                "completed": false,
                "dry_run": false,
                "completion": completion,
                "record": record,
            })));
        }
        return Ok(Json(json!({
            "ok": true,
            "completed": false,
            "dry_run": true,
            "completion": completion,
        })));
    }

    let qb_files = qbittorrent::torrent_files(&qb_server, &qb_torrent.hash).await?;
    let plan = build_hardlink_plan(&record, &category, &push, &qb_torrent, &qb_files, now)?;
    let completion = if body.dry_run {
        dry_run_hardlink_plan(&plan, now)
    } else {
        execute_hardlink_plan(&plan, now)
    };
    let completed = completion.status == "completed";
    push.status = completion.status.clone();
    push.error = completion.error.clone();
    push.completed_at = completion.completed_at;
    push.source_path = completion.source_path.clone();
    push.target_dir = completion.target_dir.clone();
    push.linked_files = completion.linked_files.clone();

    if body.dry_run {
        return Ok(Json(json!({
            "ok": true,
            "completed": completed,
            "dry_run": true,
            "completion": completion,
            "plan_file_count": plan.files.len(),
        })));
    }

    let status = if completed {
        subscription::WantedSubscriptionStatus::Completed
    } else {
        subscription::WantedSubscriptionStatus::Failed
    };
    let record = state
        .wanted_store
        .update_completion_record(
            &account_key,
            &record.subject_id,
            push.clone(),
            completion.clone(),
            status,
            completion.error.clone(),
            now,
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入硬链接结果失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;

    Ok(Json(json!({
        "ok": completed,
        "completed": completed,
        "dry_run": false,
        "completion": completion,
        "push": push,
        "record": record,
    })))
}

async fn load_wanted_record_context(
    state: &AppState,
    id: &str,
) -> Result<(FileConfig, String, subscription::WantedSubscriptionRecord), ApiError> {
    let cfg = state.config.read().await.clone();
    let account_key =
        douban::auth_cache_key_fragment(&cfg.douban_cookie).map_err(ApiError::douban)?;
    let snapshot = state
        .wanted_store
        .snapshot(&account_key, unix_now_secs())
        .await
        .map_err(|e| ApiError::internal(format!("读取想看订阅状态失败: {e}")))?;
    let record = snapshot
        .records
        .get(id.trim())
        .cloned()
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
    Ok((cfg, account_key, record))
}

fn category_for_wanted_record<'a>(
    record: &subscription::WantedSubscriptionRecord,
    categories: &'a [SubscriptionCategory],
) -> Result<&'a SubscriptionCategory, ApiError> {
    let wanted_text = record
        .category_text
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| record.tags.first().map(String::as_str))
        .ok_or_else(|| ApiError::bad_request("订阅记录缺少想看分类标签"))?;
    categories
        .iter()
        .find(|category| category.wanted_tag.trim() == wanted_text.trim())
        .ok_or_else(|| ApiError::bad_request(format!("订阅分类不存在或已被修改: {wanted_text}")))
}

fn select_qb_server(
    servers: &[QbServerEntry],
    requested_name: Option<&str>,
) -> Result<QbServerEntry, ApiError> {
    let requested = requested_name.map(str::trim).filter(|s| !s.is_empty());
    let server = if let Some(name) = requested {
        servers
            .iter()
            .find(|server| server.name.trim() == name || server.base_url.trim() == name)
    } else {
        servers.first()
    };
    server
        .cloned()
        .ok_or_else(|| ApiError::bad_request("请先在设置中配置 qB 服务器"))
}

async fn search_mteam_candidates_for_record(
    api_key: &str,
    record: &subscription::WantedSubscriptionRecord,
    page_size: Option<u32>,
) -> Result<Vec<subscription::TorrentCandidateRecord>, ApiError> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err(ApiError::bad_request("请先在设置中填写 M-Team API Key"));
    }
    let page_size = page_size.unwrap_or(default_page_size()).clamp(1, 100);
    let client = reqwest::Client::builder()
        .use_rustls_tls()
        .build()
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    let subject_id = record.subject_id.trim();
    if !subject_id.is_empty() {
        let douban = normalize_douban_url(subject_id);
        let body = json!({
            "pageNumber": 1,
            "pageSize": page_size,
            "douban": douban,
        });
        let response = mteam_search_post(&client, key, &body).await?;
        append_unique_candidates(
            &mut candidates,
            &mut seen,
            mteam_candidates_from_response(&response, "douban", subject_id),
        );
    }

    let title = record.title.trim();
    if !title.is_empty() {
        let body = json!({
            "pageNumber": 1,
            "pageSize": page_size,
            "keyword": title,
        });
        let response = mteam_search_post(&client, key, &body).await?;
        append_unique_candidates(
            &mut candidates,
            &mut seen,
            mteam_candidates_from_response(&response, "keyword", title),
        );
    }

    Ok(candidates)
}

fn append_unique_candidates(
    out: &mut Vec<subscription::TorrentCandidateRecord>,
    seen: &mut HashSet<String>,
    candidates: Vec<subscription::TorrentCandidateRecord>,
) {
    for candidate in candidates {
        let key = if candidate.torrent_id.trim().is_empty() {
            format!("title:{}", candidate.title)
        } else {
            format!("id:{}", candidate.torrent_id)
        };
        if seen.insert(key) {
            out.push(candidate);
        }
    }
}

fn mteam_candidates_from_response(
    response: &Value,
    source: &str,
    search_query: &str,
) -> Vec<subscription::TorrentCandidateRecord> {
    let mut values = Vec::new();
    collect_mteam_candidate_objects(response, &mut values);
    values
        .into_iter()
        .filter_map(|value| mteam_candidate_from_value(value, source, search_query))
        .collect()
}

fn collect_mteam_candidate_objects<'a>(value: &'a Value, out: &mut Vec<&'a Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_mteam_candidate_objects(item, out);
            }
        }
        Value::Object(map) => {
            let looks_like_torrent = ["id", "torrentId", "torrent_id", "tid"]
                .iter()
                .any(|key| map.contains_key(*key))
                && ["name", "title", "smallDescr", "small_descr"]
                    .iter()
                    .any(|key| map.contains_key(*key));
            if looks_like_torrent {
                out.push(value);
            } else {
                for item in map.values() {
                    collect_mteam_candidate_objects(item, out);
                }
            }
        }
        _ => {}
    }
}

fn mteam_candidate_from_value(
    value: &Value,
    source: &str,
    search_query: &str,
) -> Option<subscription::TorrentCandidateRecord> {
    let title = first_string_field(value, &["name", "title", "smallDescr", "small_descr"])?;
    let torrent_id =
        first_string_field(value, &["id", "torrentId", "torrent_id", "tid"]).unwrap_or_default();
    let subtitle = first_string_field(
        value,
        &[
            "smallDescr",
            "small_descr",
            "description",
            "descr",
            "subTitle",
            "subtitle",
        ],
    )
    .unwrap_or_default();
    Some(subscription::TorrentCandidateRecord {
        torrent_id,
        title,
        subtitle,
        source: source.to_string(),
        search_query: search_query.to_string(),
        size: first_string_field(value, &["size", "sizeStr", "size_str"]),
        seeders: first_u64_field(value, &["seeders", "seeder", "seed", "status.seeders"]),
        leechers: first_u64_field(value, &["leechers", "leecher", "leech", "status.leechers"]),
        uploaded_at: first_string_field(value, &["createdDate", "created_date", "added", "date"]),
    })
}

fn first_string_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value_at_path(value, key))
        .find_map(value_to_trimmed_string)
}

fn first_u64_field(value: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .filter_map(|key| value_at_path(value, key))
        .find_map(|value| match value {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        })
}

fn value_at_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn value_to_trimmed_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            let trimmed = s.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn match_torrent_candidates(
    candidates: &[subscription::TorrentCandidateRecord],
    rules: &[TorrentMatchRule],
) -> Vec<subscription::TorrentCandidateMatchRecord> {
    if rules.is_empty() {
        return match_torrent_candidates_without_rules(candidates);
    }

    let mut sorted_rules = rules.to_vec();
    sorted_rules.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.name.cmp(&b.name))
    });
    let mut rows = candidates
        .iter()
        .map(|candidate| {
            let evaluations = sorted_rules
                .iter()
                .map(|rule| evaluate_candidate_rule(candidate, rule))
                .collect::<Vec<_>>();
            let best = evaluations.iter().find(|item| item.matched);
            subscription::TorrentCandidateMatchRecord {
                candidate: candidate.clone(),
                selected: false,
                matched_rule_name: best.map(|item| item.rule_name.clone()),
                matched_priority: best.map(|item| item.priority),
                matched_keywords: best
                    .map(|item| item.matched_keywords.clone())
                    .unwrap_or_default(),
                excluded_reason: if candidate.torrent_id.trim().is_empty() {
                    Some("缺少种子 ID，无法推送".to_string())
                } else {
                    best.and_then(|item| item.excluded_reason.clone())
                },
                rule_evaluations: evaluations,
            }
        })
        .collect::<Vec<_>>();

    let selected_priority = rows
        .iter()
        .filter(|row| row.candidate.torrent_id.trim().len() > 0)
        .filter_map(|row| row.matched_priority)
        .max();
    if let Some(priority) = selected_priority {
        if let Some(selected) = rows.iter_mut().find(|row| {
            row.candidate.torrent_id.trim().len() > 0 && row.matched_priority == Some(priority)
        }) {
            selected.selected = true;
            selected.excluded_reason = None;
        }
        for row in rows.iter_mut().filter(|row| !row.selected) {
            if row.matched_priority.is_none() && row.excluded_reason.is_none() {
                row.excluded_reason = Some("未命中任何规则".to_string());
            } else if row.matched_priority.is_some() && row.excluded_reason.is_none() {
                row.excluded_reason = Some("已有更高优先级或更靠前候选被选中".to_string());
            }
        }
    } else {
        for row in &mut rows {
            if row.excluded_reason.is_none() {
                row.excluded_reason = Some("未命中任何规则".to_string());
            }
        }
    }
    rows
}

fn match_torrent_candidates_without_rules(
    candidates: &[subscription::TorrentCandidateRecord],
) -> Vec<subscription::TorrentCandidateMatchRecord> {
    let mut selected = false;
    candidates
        .iter()
        .map(|candidate| {
            let can_push = !candidate.torrent_id.trim().is_empty();
            let row_selected = can_push && !selected;
            if row_selected {
                selected = true;
            }
            subscription::TorrentCandidateMatchRecord {
                candidate: candidate.clone(),
                selected: row_selected,
                matched_rule_name: row_selected.then(|| "默认首个候选".to_string()),
                matched_priority: row_selected.then_some(0),
                matched_keywords: Vec::new(),
                excluded_reason: if row_selected {
                    None
                } else if can_push {
                    Some("未配置规则，已有更靠前候选被选中".to_string())
                } else {
                    Some("缺少种子 ID，无法推送".to_string())
                },
                rule_evaluations: Vec::new(),
            }
        })
        .collect()
}

fn evaluate_candidate_rule(
    candidate: &subscription::TorrentCandidateRecord,
    rule: &TorrentMatchRule,
) -> subscription::CandidateRuleEvaluation {
    let checks = rule_keyword_checks(rule);
    if checks.is_empty() {
        return subscription::CandidateRuleEvaluation {
            rule_name: rule.name.clone(),
            priority: rule.priority,
            mode: rule_mode_label(rule).to_string(),
            matched: false,
            matched_keywords: Vec::new(),
            missing_keywords: Vec::new(),
            excluded_reason: Some("规则没有关键词".to_string()),
        };
    }
    let searchable = format!(
        "{}\n{}\n{}\n{}",
        candidate.title, candidate.subtitle, candidate.source, candidate.search_query
    )
    .to_lowercase();
    let mut matched_keywords = Vec::new();
    let mut missing_keywords = Vec::new();
    for (label, needle) in checks {
        if searchable.contains(&needle.to_lowercase()) {
            matched_keywords.push(label);
        } else {
            missing_keywords.push(label);
        }
    }
    let matched = match rule.mode {
        config::TorrentRuleMatchMode::All => missing_keywords.is_empty(),
        config::TorrentRuleMatchMode::Any => !matched_keywords.is_empty(),
    };
    let excluded_reason = (!matched).then(|| {
        if matched_keywords.is_empty() {
            "规则关键词均未命中".to_string()
        } else {
            format!("缺少关键词: {}", missing_keywords.join(", "))
        }
    });
    subscription::CandidateRuleEvaluation {
        rule_name: rule.name.clone(),
        priority: rule.priority,
        mode: rule_mode_label(rule).to_string(),
        matched,
        matched_keywords,
        missing_keywords,
        excluded_reason,
    }
}

fn rule_keyword_checks(rule: &TorrentMatchRule) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (prefix, values) in [
        ("title", &rule.title_keywords),
        ("resolution", &rule.resolution_keywords),
        ("source", &rule.source_keywords),
    ] {
        for value in values {
            let keyword = value.trim();
            if !keyword.is_empty() {
                out.push((format!("{prefix}:{keyword}"), keyword.to_string()));
            }
        }
    }
    out
}

fn rule_mode_label(rule: &TorrentMatchRule) -> &'static str {
    match rule.mode {
        config::TorrentRuleMatchMode::All => "all",
        config::TorrentRuleMatchMode::Any => "any",
    }
}

async fn record_push_failure(
    state: &AppState,
    account_key: &str,
    subscription_id: &str,
    qb_server: &QbServerEntry,
    category: &SubscriptionCategory,
    selected: Option<&subscription::TorrentCandidateMatchRecord>,
    error: String,
) -> Result<(), ApiError> {
    let fallback = subscription::TorrentCandidateRecord {
        torrent_id: String::new(),
        title: String::new(),
        subtitle: String::new(),
        source: String::new(),
        search_query: String::new(),
        size: None,
        seeders: None,
        leechers: None,
        uploaded_at: None,
    };
    let candidate = selected.map(|item| &item.candidate).unwrap_or(&fallback);
    let push = torrent_push_record(
        subscription_id,
        qb_server,
        category,
        candidate,
        "failed",
        None,
        Some(error.clone()),
    );
    state
        .wanted_store
        .update_push_record(
            account_key,
            subscription_id,
            push,
            subscription::WantedSubscriptionStatus::Failed,
            Some(error),
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入 qB 推送失败记录失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
    Ok(())
}

#[derive(Debug, Clone)]
struct HardlinkPlan {
    source_root: PathBuf,
    target_dir: PathBuf,
    qb_hash: String,
    qb_name: String,
    files: Vec<HardlinkFilePlan>,
}

#[derive(Debug, Clone)]
struct HardlinkFilePlan {
    source_path: PathBuf,
    target_path: PathBuf,
    size: u64,
}

fn select_qb_torrent(
    torrents: &[qbittorrent::QbTorrentInfo],
    push: &subscription::TorrentPushRecord,
    requested_hash: Option<&str>,
) -> Result<qbittorrent::QbTorrentInfo, ApiError> {
    if let Some(hash) = requested_hash.map(str::trim).filter(|s| !s.is_empty()) {
        return torrents
            .iter()
            .find(|torrent| torrent.hash.eq_ignore_ascii_case(hash))
            .cloned()
            .ok_or_else(|| ApiError::bad_request(format!("qB 中未找到指定 hash: {hash}")));
    }
    let title = normalize_match_text(&push.torrent_title);
    if title.is_empty() {
        return Err(ApiError::bad_request(
            "pushed record 缺少种子标题，无法匹配 qB 任务",
        ));
    }
    torrents
        .iter()
        .find(|torrent| {
            let name = normalize_match_text(&torrent.name);
            !name.is_empty() && (name == title || name.contains(&title) || title.contains(&name))
        })
        .cloned()
        .ok_or_else(|| {
            ApiError::bad_request(format!("qB 中未找到已推送种子: {}", push.torrent_title))
        })
}

fn normalize_match_text(raw: &str) -> String {
    raw.trim().to_lowercase()
}

fn build_hardlink_plan(
    record: &subscription::WantedSubscriptionRecord,
    category: &SubscriptionCategory,
    push: &subscription::TorrentPushRecord,
    torrent: &qbittorrent::QbTorrentInfo,
    files: &[qbittorrent::QbTorrentFile],
    _now: u64,
) -> Result<HardlinkPlan, ApiError> {
    let release_year = record.release_year.ok_or_else(|| {
        ApiError::bad_request("订阅记录缺少上映年份，无法创建 中文名.上映年份 目录")
    })?;
    let title = sanitize_output_component(&record.title);
    if title.is_empty() {
        return Err(ApiError::bad_request(
            "订阅记录缺少中文名，无法创建硬链接目录",
        ));
    }
    let source_root = PathBuf::from(category.download_dir.trim());
    if source_root.as_os_str().is_empty() {
        return Err(ApiError::bad_request("订阅分类缺少真实下载目录"));
    }
    let target_root = PathBuf::from(category.link_target_dir.trim());
    if target_root.as_os_str().is_empty() {
        return Err(ApiError::bad_request("订阅分类缺少硬链接目标目录"));
    }
    let target_dir = target_root.join(format!("{title}.{release_year}"));

    let selected = files
        .iter()
        .filter(|file| should_link_media_file(&file.name))
        .filter_map(|file| safe_relative_path(&file.name).map(|relative| (file, relative)))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(ApiError::bad_request("qB 文件列表中没有可硬链接的媒体文件"));
    }

    let mut plan_files = Vec::new();
    for (file, relative) in selected {
        let source_path = source_path_for_qb_file(&source_root, push, torrent, &relative);
        let target_path = target_dir.join(relative);
        plan_files.push(HardlinkFilePlan {
            source_path,
            target_path,
            size: file.size,
        });
    }

    Ok(HardlinkPlan {
        source_root,
        target_dir,
        qb_hash: torrent.hash.clone(),
        qb_name: torrent.name.clone(),
        files: plan_files,
    })
}

fn source_path_for_qb_file(
    source_root: &Path,
    push: &subscription::TorrentPushRecord,
    torrent: &qbittorrent::QbTorrentInfo,
    relative: &Path,
) -> PathBuf {
    let direct = source_root.join(relative);
    if direct.exists() {
        return direct;
    }

    let content_path = map_qb_path_to_local(
        Path::new(torrent.content_path.trim()),
        push.qb_save_dir_name.trim(),
        source_root,
    );
    if let Some(path) = content_path {
        if path.is_file() {
            return path;
        }
        let joined = path.join(relative);
        if joined.exists() {
            return joined;
        }
    }
    direct
}

fn map_qb_path_to_local(
    qb_path: &Path,
    qb_save_dir_name: &str,
    source_root: &Path,
) -> Option<PathBuf> {
    if qb_path.as_os_str().is_empty() {
        return None;
    }
    let save_dir = Path::new(qb_save_dir_name);
    if !qb_save_dir_name.trim().is_empty() && qb_path.starts_with(save_dir) {
        let suffix = qb_path.strip_prefix(save_dir).ok()?;
        return Some(source_root.join(suffix));
    }
    None
}

fn dry_run_hardlink_plan(plan: &HardlinkPlan, now: u64) -> subscription::HardlinkCompletionRecord {
    subscription::HardlinkCompletionRecord {
        status: "dry_run".to_string(),
        checked_at: now,
        completed_at: None,
        qb_hash: Some(plan.qb_hash.clone()),
        qb_name: Some(plan.qb_name.clone()),
        source_path: Some(plan.source_root.display().to_string()),
        target_dir: Some(plan.target_dir.display().to_string()),
        linked_files: plan
            .files
            .iter()
            .map(|file| subscription::HardlinkFileRecord {
                source_path: file.source_path.display().to_string(),
                target_path: file.target_path.display().to_string(),
                size: file.size,
                status: "planned".to_string(),
                error: None,
            })
            .collect(),
        error: None,
    }
}

fn execute_hardlink_plan(plan: &HardlinkPlan, now: u64) -> subscription::HardlinkCompletionRecord {
    let mut records = Vec::new();
    let mut errors = Vec::new();

    for file in &plan.files {
        let record = hardlink_one_file(file);
        if let Some(error) = record.error.clone() {
            errors.push(error);
        }
        records.push(record);
    }

    let status = if errors.is_empty() {
        "completed"
    } else {
        "failed"
    };
    subscription::HardlinkCompletionRecord {
        status: status.to_string(),
        checked_at: now,
        completed_at: errors.is_empty().then_some(now),
        qb_hash: Some(plan.qb_hash.clone()),
        qb_name: Some(plan.qb_name.clone()),
        source_path: Some(plan.source_root.display().to_string()),
        target_dir: Some(plan.target_dir.display().to_string()),
        linked_files: records,
        error: (!errors.is_empty()).then(|| errors.join("; ")),
    }
}

fn hardlink_one_file(file: &HardlinkFilePlan) -> subscription::HardlinkFileRecord {
    let source_display = file.source_path.display().to_string();
    let target_display = file.target_path.display().to_string();
    if !file.source_path.exists() {
        return subscription::HardlinkFileRecord {
            source_path: source_display,
            target_path: target_display,
            size: file.size,
            status: "failed".to_string(),
            error: Some("源文件不存在".to_string()),
        };
    }
    if file.target_path.exists() {
        if same_file(&file.source_path, &file.target_path).unwrap_or(false) {
            return subscription::HardlinkFileRecord {
                source_path: source_display,
                target_path: target_display,
                size: file.size,
                status: "already_linked".to_string(),
                error: None,
            };
        }
        return subscription::HardlinkFileRecord {
            source_path: source_display,
            target_path: target_display,
            size: file.size,
            status: "failed".to_string(),
            error: Some("目标文件已存在且不是同一硬链接".to_string()),
        };
    }
    if let Some(parent) = file.target_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return subscription::HardlinkFileRecord {
                source_path: source_display,
                target_path: target_display,
                size: file.size,
                status: "failed".to_string(),
                error: Some(format!("创建目标目录失败: {e}")),
            };
        }
    }
    match std::fs::hard_link(&file.source_path, &file.target_path) {
        Ok(()) => subscription::HardlinkFileRecord {
            source_path: source_display,
            target_path: target_display,
            size: file.size,
            status: "linked".to_string(),
            error: None,
        },
        Err(e) => subscription::HardlinkFileRecord {
            source_path: source_display,
            target_path: target_display,
            size: file.size,
            status: "failed".to_string(),
            error: Some(hardlink_error_message(&e)),
        },
    }
}

fn hardlink_error_message(error: &std::io::Error) -> String {
    if error.raw_os_error() == Some(18) {
        return "跨设备硬链接失败: 源目录和目标目录不在同一文件系统".to_string();
    }
    format!("硬链接失败: {error}")
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let ma = std::fs::metadata(a)?;
    let mb = std::fs::metadata(b)?;
    Ok(ma.dev() == mb.dev() && ma.ino() == mb.ino())
}

#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> std::io::Result<bool> {
    Ok(std::fs::canonicalize(a)? == std::fs::canonicalize(b)?)
}

fn should_link_media_file(path: &str) -> bool {
    let Some(relative) = safe_relative_path(path) else {
        return false;
    };
    if relative.components().any(|component| {
        let text = component.as_os_str().to_string_lossy().to_lowercase();
        matches!(
            text.as_str(),
            "sample"
                | "samples"
                | "screenshot"
                | "screenshots"
                | "screen"
                | "screens"
                | "proof"
                | "cover"
                | "covers"
                | "poster"
                | "posters"
                | "extra"
                | "extras"
        )
    }) {
        return false;
    }
    let file_name = relative
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if file_name.contains("sample") || file_name.contains("screenshot") {
        return false;
    }
    let ext = relative
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "mkv"
            | "mp4"
            | "m4v"
            | "avi"
            | "mov"
            | "wmv"
            | "flv"
            | "webm"
            | "ts"
            | "m2ts"
            | "iso"
            | "srt"
            | "ass"
            | "ssa"
            | "sup"
            | "sub"
            | "idx"
    )
}

fn safe_relative_path(path: &str) -> Option<PathBuf> {
    let raw = Path::new(path.trim());
    if raw.as_os_str().is_empty() || raw.is_absolute() {
        return None;
    }
    let mut out = PathBuf::new();
    for component in raw.components() {
        match component {
            std::path::Component::Normal(part) => out.push(part),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

fn sanitize_output_component(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.trim().chars() {
        let safe = match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            _ => ch,
        };
        if safe.is_control() {
            continue;
        }
        out.push(safe);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn torrent_push_record(
    subscription_id: &str,
    qb_server: &QbServerEntry,
    category: &SubscriptionCategory,
    candidate: &subscription::TorrentCandidateRecord,
    status: &str,
    pushed_at: Option<u64>,
    error: Option<String>,
) -> subscription::TorrentPushRecord {
    let qb_server_name = if qb_server.name.trim().is_empty() {
        qb_server.base_url.trim().to_string()
    } else {
        qb_server.name.trim().to_string()
    };
    subscription::TorrentPushRecord {
        subscription_id: subscription_id.to_string(),
        torrent_id: candidate.torrent_id.clone(),
        torrent_title: candidate.title.clone(),
        qb_server: qb_server_name,
        qb_category: category.qb_category.trim().to_string(),
        qb_save_dir_name: category.qb_save_dir_name.trim().to_string(),
        qb_identifier: if candidate.torrent_id.trim().is_empty() {
            String::new()
        } else {
            format!("mteam:{}", candidate.torrent_id.trim())
        },
        pushed_at,
        status: status.to_string(),
        error,
        qb_hash: None,
        qb_name: None,
        checked_at: None,
        completed_at: None,
        source_path: None,
        target_dir: None,
        linked_files: Vec::new(),
    }
}

async fn run_wanted_watch_poll(
    state: &AppState,
) -> Result<subscription::WantedPollOutcome, ApiError> {
    let cfg = state.config.read().await.clone();
    let account_key =
        douban::auth_cache_key_fragment(&cfg.douban_cookie).map_err(ApiError::douban)?;
    let limit = cfg.subscription_watcher.library_limit.clamp(1, 1200);
    let wish = douban::library(&cfg.douban_cookie, douban::DoubanLibraryStatus::Wish, limit)
        .await
        .map_err(ApiError::douban)?;
    state
        .wanted_store
        .apply_wish_items(
            &account_key,
            &wish.items,
            &cfg.subscription_watcher,
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入想看订阅状态失败: {e}")))
}

fn spawn_wanted_watch_loop(state: AppState) {
    tokio::spawn(async move {
        loop {
            let cfg = state.config.read().await.subscription_watcher.clone();
            let interval = cfg.poll_interval_secs.clamp(60, 86_400);
            if cfg.enabled {
                match run_wanted_watch_poll(&state).await {
                    Ok(outcome) => tracing::info!(
                        account_key = %outcome.account_key,
                        total = outcome.total_wish_items,
                        created_unprocessed = outcome.created_unprocessed,
                        created_skipped = outcome.created_skipped,
                        updated_existing = outcome.updated_existing,
                        "wanted subscription poll completed"
                    ),
                    Err(e) => tracing::warn!("wanted subscription poll failed: {}", e.message()),
                }
            }
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    });
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

    fn torrent(id: &str, title: &str) -> subscription::TorrentCandidateRecord {
        subscription::TorrentCandidateRecord {
            torrent_id: id.to_string(),
            title: title.to_string(),
            subtitle: String::new(),
            source: "keyword".to_string(),
            search_query: "测试电影".to_string(),
            size: None,
            seeders: None,
            leechers: None,
            uploaded_at: None,
        }
    }

    fn torrent_rule(
        name: &str,
        priority: i32,
        mode: config::TorrentRuleMatchMode,
        title_keywords: &[&str],
        resolution_keywords: &[&str],
        source_keywords: &[&str],
    ) -> TorrentMatchRule {
        TorrentMatchRule {
            name: name.to_string(),
            priority,
            mode,
            title_keywords: title_keywords.iter().map(|s| s.to_string()).collect(),
            resolution_keywords: resolution_keywords.iter().map(|s| s.to_string()).collect(),
            source_keywords: source_keywords.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn torrent_matching_prefers_high_priority_rule() {
        let candidates = vec![
            torrent("1", "测试电影 1080p WEB-DL"),
            torrent("2", "测试电影 2160p BluRay REMUX"),
        ];
        let rules = vec![
            torrent_rule(
                "low 1080p",
                10,
                config::TorrentRuleMatchMode::All,
                &["1080p"],
                &[],
                &[],
            ),
            torrent_rule(
                "high 2160p",
                100,
                config::TorrentRuleMatchMode::All,
                &["2160p"],
                &[],
                &["remux"],
            ),
        ];
        let matches = match_torrent_candidates(&candidates, &rules);
        let selected = matches.iter().find(|item| item.selected).unwrap();
        assert_eq!(selected.candidate.torrent_id, "2");
        assert_eq!(selected.matched_rule_name.as_deref(), Some("high 2160p"));
        assert_eq!(selected.matched_priority, Some(100));
    }

    #[test]
    fn torrent_matching_falls_back_to_lower_priority_when_high_misses() {
        let candidates = vec![torrent("1", "测试电影 1080p WEB-DL")];
        let rules = vec![
            torrent_rule(
                "high 2160p",
                100,
                config::TorrentRuleMatchMode::All,
                &["2160p"],
                &[],
                &[],
            ),
            torrent_rule(
                "low 1080p",
                10,
                config::TorrentRuleMatchMode::All,
                &["1080p"],
                &[],
                &[],
            ),
        ];
        let matches = match_torrent_candidates(&candidates, &rules);
        let selected = matches.iter().find(|item| item.selected).unwrap();
        assert_eq!(selected.candidate.torrent_id, "1");
        assert_eq!(selected.matched_rule_name.as_deref(), Some("low 1080p"));
        assert!(selected
            .rule_evaluations
            .iter()
            .any(|item| item.rule_name == "high 2160p" && !item.matched));
    }

    #[test]
    fn torrent_matching_supports_and_and_or_modes() {
        let candidates = vec![
            torrent("1", "测试电影 2160p WEB-DL"),
            torrent("2", "测试电影 1080p BluRay"),
        ];
        let rules = vec![
            torrent_rule(
                "all remux",
                100,
                config::TorrentRuleMatchMode::All,
                &["2160p"],
                &[],
                &["remux"],
            ),
            torrent_rule(
                "any web or bluray",
                50,
                config::TorrentRuleMatchMode::Any,
                &[],
                &[],
                &["web-dl", "bluray"],
            ),
        ];
        let matches = match_torrent_candidates(&candidates, &rules);
        let selected = matches.iter().find(|item| item.selected).unwrap();
        assert_eq!(selected.candidate.torrent_id, "1");
        assert_eq!(
            selected.matched_rule_name.as_deref(),
            Some("any web or bluray")
        );
        assert!(selected
            .matched_keywords
            .iter()
            .any(|keyword| keyword == "source:web-dl"));
    }

    #[test]
    fn torrent_matching_records_no_match_explanations() {
        let candidates = vec![torrent("1", "测试电影 720p HDTV")];
        let rules = vec![torrent_rule(
            "wanted",
            100,
            config::TorrentRuleMatchMode::All,
            &["2160p"],
            &[],
            &["remux"],
        )];
        let matches = match_torrent_candidates(&candidates, &rules);
        assert!(!matches.iter().any(|item| item.selected));
        assert_eq!(
            matches[0].excluded_reason.as_deref(),
            Some("未命中任何规则")
        );
        assert!(matches[0].rule_evaluations[0]
            .missing_keywords
            .contains(&"title:2160p".to_string()));
    }

    #[test]
    fn failed_push_record_keeps_qb_and_candidate_context() {
        let candidate = torrent("12345", "测试电影 2160p");
        let qb = QbServerEntry {
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let category = category("电影", "电影");
        let push = torrent_push_record(
            "subject-1",
            &qb,
            &category,
            &candidate,
            "failed",
            None,
            Some("qB 添加失败".to_string()),
        );
        assert_eq!(push.subscription_id, "subject-1");
        assert_eq!(push.torrent_id, "12345");
        assert_eq!(push.qb_server, "nas");
        assert_eq!(push.qb_category, "qb-电影");
        assert_eq!(push.qb_save_dir_name, "save-电影");
        assert_eq!(push.qb_identifier, "mteam:12345");
        assert_eq!(push.status, "failed");
        assert_eq!(push.error.as_deref(), Some("qB 添加失败"));
    }

    fn wanted_record(
        subject_id: &str,
        title: &str,
        release_year: Option<u16>,
    ) -> subscription::WantedSubscriptionRecord {
        subscription::WantedSubscriptionRecord {
            subject_id: subject_id.to_string(),
            title: title.to_string(),
            release_year,
            category_text: Some("电影".to_string()),
            tags: vec!["电影".to_string()],
            status: subscription::WantedSubscriptionStatus::Pushed,
            retry_count: 0,
            max_retries: 3,
            last_error: None,
            skip_reason: None,
            candidate_matches: Vec::new(),
            last_push: None,
            last_completion: None,
            created_at: 100,
            updated_at: 100,
            first_seen_at: 100,
            last_seen_at: 100,
        }
    }

    fn qb_torrent(name: &str) -> qbittorrent::QbTorrentInfo {
        qbittorrent::QbTorrentInfo {
            hash: "abcdef".to_string(),
            name: name.to_string(),
            category: "qb-电影".to_string(),
            save_path: "/downloads/movie".to_string(),
            content_path: String::new(),
            progress: 1.0,
            completion_on: 200,
            state: "uploading".to_string(),
        }
    }

    fn qb_file(name: &str, size: u64) -> qbittorrent::QbTorrentFile {
        qbittorrent::QbTorrentFile {
            name: name.to_string(),
            size,
            progress: 1.0,
            priority: 1,
        }
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tmdb_mteam_{name}_{nanos}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn media_file_filter_keeps_media_and_skips_noise() {
        assert!(should_link_media_file("Movie/Movie.2024.2160p.mkv"));
        assert!(should_link_media_file("Movie/Subs/Movie.zh.ass"));
        assert!(!should_link_media_file("Movie/Sample/sample.mkv"));
        assert!(!should_link_media_file("Movie/Screenshots/shot01.png"));
        assert!(!should_link_media_file("Movie/readme.txt"));
        assert!(!should_link_media_file("../escape.mkv"));
    }

    #[test]
    fn hardlink_plan_creates_chinese_title_year_outer_dir() {
        let root = temp_test_dir("plan");
        let source = root.join("downloads");
        let target = root.join("links");
        std::fs::create_dir_all(source.join("TorrentRoot")).unwrap();
        std::fs::write(source.join("TorrentRoot/movie.mkv"), b"movie").unwrap();

        let mut category = category("电影", "电影");
        category.download_dir = source.display().to_string();
        category.link_target_dir = target.display().to_string();
        let push = torrent_push_record(
            "subject-1",
            &QbServerEntry {
                name: "qb".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "u".to_string(),
                password: "p".to_string(),
                insecure_tls: false,
            },
            &category,
            &torrent("123", "Movie 2160p"),
            "pushed",
            Some(100),
            None,
        );
        let Ok(plan) = build_hardlink_plan(
            &wanted_record("subject-1", "测试电影", Some(2024)),
            &category,
            &push,
            &qb_torrent("Movie 2160p"),
            &[
                qb_file("TorrentRoot/movie.mkv", 5),
                qb_file("TorrentRoot/Sample/sample.mkv", 1),
            ],
            200,
        ) else {
            panic!("valid completed torrent should produce a hardlink plan");
        };
        assert_eq!(plan.files.len(), 1);
        assert_eq!(plan.target_dir, target.join("测试电影.2024"));
        assert_eq!(
            plan.files[0].source_path,
            source.join("TorrentRoot/movie.mkv")
        );
        assert_eq!(
            plan.files[0].target_path,
            target.join("测试电影.2024/TorrentRoot/movie.mkv")
        );
    }

    #[test]
    fn hardlink_execution_is_idempotent_for_existing_hardlink() {
        let root = temp_test_dir("idempotent");
        let source = root.join("source/movie.mkv");
        let target = root.join("target/movie.mkv");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"movie").unwrap();
        let plan = HardlinkPlan {
            source_root: root.join("source"),
            target_dir: root.join("target"),
            qb_hash: "hash".to_string(),
            qb_name: "movie".to_string(),
            files: vec![HardlinkFilePlan {
                source_path: source,
                target_path: target,
                size: 5,
            }],
        };

        let first = execute_hardlink_plan(&plan, 300);
        assert_eq!(first.status, "completed");
        assert_eq!(first.linked_files[0].status, "linked");
        let second = execute_hardlink_plan(&plan, 400);
        assert_eq!(second.status, "completed");
        assert_eq!(second.linked_files[0].status, "already_linked");
    }

    #[test]
    fn hardlink_execution_records_missing_source() {
        let root = temp_test_dir("missing");
        let plan = HardlinkPlan {
            source_root: root.join("source"),
            target_dir: root.join("target"),
            qb_hash: "hash".to_string(),
            qb_name: "movie".to_string(),
            files: vec![HardlinkFilePlan {
                source_path: root.join("source/missing.mkv"),
                target_path: root.join("target/missing.mkv"),
                size: 5,
            }],
        };
        let result = execute_hardlink_plan(&plan, 300);
        assert_eq!(result.status, "failed");
        assert_eq!(
            result.linked_files[0].error.as_deref(),
            Some("源文件不存在")
        );
    }

    #[test]
    fn hardlink_execution_records_target_conflict() {
        let root = temp_test_dir("target_conflict");
        let source = root.join("source/movie.mkv");
        let target = root.join("target/movie.mkv");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&target, b"target").unwrap();
        let plan = HardlinkPlan {
            source_root: root.join("source"),
            target_dir: root.join("target"),
            qb_hash: "hash".to_string(),
            qb_name: "movie".to_string(),
            files: vec![HardlinkFilePlan {
                source_path: source,
                target_path: target,
                size: 6,
            }],
        };
        let result = execute_hardlink_plan(&plan, 300);
        assert_eq!(result.status, "failed");
        assert_eq!(
            result.linked_files[0].error.as_deref(),
            Some("目标文件已存在且不是同一硬链接")
        );
    }

    #[test]
    fn hardlink_error_message_calls_out_cross_device() {
        let err = std::io::Error::from_raw_os_error(18);
        assert!(hardlink_error_message(&err).contains("跨设备硬链接失败"));
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

    fn message(&self) -> &str {
        match self {
            Self::BadRequest { message } | Self::Other { message, .. } => message,
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
