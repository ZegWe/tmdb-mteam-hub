mod config;
mod douban;
mod qbittorrent;
mod subscription;
mod tmdb_cache;
mod torrent_hash;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path as PathParam, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post},
    Router,
};
use config::{
    FileConfig, QbServerEntry, SubscriptionCategory, SubscriptionWatcherConfig, TorrentMatchRule,
};
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
    let file_cfg = normalize_loaded_file_config(FileConfig::load_or_create(&config_path)?);
    if let Err(err) = file_cfg.save(&config_path) {
        tracing::warn!("failed to persist normalized config: {err}");
    }
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
        "subscription state: dir={} db={}",
        subscription_state_dir.display(),
        wanted_store.db_path().display()
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
        .route("/operation-logs", get(operation_logs))
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
            "/subscriptions/wanted/{id}/progress",
            post(wanted_subscription_progress),
        )
        .route(
            "/subscriptions/wanted/{id}/retry-current",
            post(wanted_subscription_retry_current),
        )
        .route(
            "/subscriptions/wanted/{id}/rerun",
            post(wanted_subscription_rerun),
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
    new_cfg.qb_servers = normalize_qb_servers(body.qb_servers)?;
    if let Some(subscription_categories) = body.subscription_categories {
        new_cfg.subscription_categories =
            normalize_subscription_categories(subscription_categories, &new_cfg.qb_servers)?;
    }
    if let Some(subscription_watcher) = body.subscription_watcher {
        new_cfg.subscription_watcher = normalize_subscription_watcher(subscription_watcher);
    }
    if let Some(torrent_match_rules) = body.torrent_match_rules {
        new_cfg.torrent_match_rules = normalize_torrent_match_rules(torrent_match_rules)?;
    }
    let account_key = config_account_key(&new_cfg);
    if let Err(e) = new_cfg.listen_addr() {
        let error = format!("监听地址配置无效: {e}");
        write_operation_log(
            &state,
            operation_log_entry(
                account_key,
                "configuration",
                "save_config",
                "config",
                None,
                None,
                "failed",
                "配置保存失败：监听地址无效",
                Some(error.clone()),
                json!({ "qb_server_count": new_cfg.qb_servers.len() }),
            ),
        )
        .await;
        return Err(ApiError::bad_request(error));
    }
    if let Err(e) = new_cfg.save(&state.config_path) {
        let error = format!("写入配置失败: {e}");
        write_operation_log(
            &state,
            operation_log_entry(
                account_key,
                "configuration",
                "save_config",
                "config",
                None,
                None,
                "failed",
                "配置保存失败：写入配置文件失败",
                Some(error.clone()),
                json!({ "qb_server_count": new_cfg.qb_servers.len() }),
            ),
        )
        .await;
        return Err(ApiError::internal(error));
    }
    *state.config.write().await = new_cfg;
    let cfg = state.config.read().await.clone();
    write_operation_log(
        &state,
        operation_log_entry(
            config_account_key(&cfg),
            "configuration",
            "save_config",
            "config",
            None,
            None,
            "success",
            "配置已保存",
            None,
            json!({
                "qb_server_count": cfg.qb_servers.len(),
                "subscription_category_count": cfg.subscription_categories.len(),
                "torrent_rule_count": cfg.torrent_match_rules.len(),
            }),
        ),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

fn normalize_qb_servers(servers: Vec<QbServerEntry>) -> Result<Vec<QbServerEntry>, ApiError> {
    let mut used = HashSet::new();
    let mut out = Vec::new();

    for server in servers {
        let id = unique_qb_server_id(&server, &mut used);
        let normalized = QbServerEntry {
            id,
            name: server.name.trim().to_string(),
            base_url: server.base_url.trim().to_string(),
            username: server.username.trim().to_string(),
            password: server.password,
            insecure_tls: server.insecure_tls,
        };
        out.push(normalized);
    }

    Ok(out)
}

fn normalize_loaded_file_config(mut cfg: FileConfig) -> FileConfig {
    if let Ok(servers) = normalize_qb_servers(cfg.qb_servers.clone()) {
        cfg.qb_servers = servers;
    }
    if let Ok(categories) =
        normalize_subscription_categories(cfg.subscription_categories.clone(), &cfg.qb_servers)
    {
        cfg.subscription_categories = categories;
    }
    cfg.subscription_watcher = normalize_subscription_watcher(cfg.subscription_watcher);
    if let Ok(rules) = normalize_torrent_match_rules(cfg.torrent_match_rules.clone()) {
        cfg.torrent_match_rules = rules;
    }
    cfg
}

fn unique_qb_server_id(server: &QbServerEntry, used: &mut HashSet<String>) -> String {
    let base = qb_server_id_base(server);
    if used.insert(base.clone()) {
        return base;
    }

    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    unreachable!("unbounded suffix search must find a unique qB server id")
}

fn qb_server_id_base(server: &QbServerEntry) -> String {
    for raw in [&server.id, &server.name, &server.base_url] {
        let sanitized = sanitize_qb_server_id(raw);
        if !sanitized.is_empty() {
            return sanitized;
        }
    }
    format!("qb-{:x}", stable_qb_server_hash(server))
}

fn sanitize_qb_server_id(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if ch == '_' {
            if !out.is_empty() {
                out.push('_');
                last_dash = false;
            }
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

fn stable_qb_server_hash(server: &QbServerEntry) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in format!(
        "{}\n{}\n{}",
        server.name.trim(),
        server.base_url.trim(),
        server.username.trim()
    )
    .as_bytes()
    {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn normalize_subscription_categories(
    categories: Vec<SubscriptionCategory>,
    qb_servers: &[QbServerEntry],
) -> Result<Vec<SubscriptionCategory>, ApiError> {
    let mut out = Vec::new();
    let mut names = HashSet::new();
    let mut wanted_tags = HashSet::new();
    let qb_server_ids = qb_servers
        .iter()
        .map(|server| server.id.trim().to_string())
        .filter(|id| !id.is_empty())
        .collect::<HashSet<_>>();
    let default_qb_server_id = qb_servers
        .first()
        .map(|server| server.id.trim().to_string())
        .filter(|id| !id.is_empty());

    for (idx, category) in categories.into_iter().enumerate() {
        let n = idx + 1;
        let qb_server_id = category.qb_server_id.trim().to_string();
        let qb_server_id = if qb_server_id.is_empty() {
            default_qb_server_id.clone().ok_or_else(|| {
                ApiError::bad_request(format!(
                    "订阅分类 {} 缺少 qB 服务器，请先配置 qB 服务器",
                    category.name.trim()
                ))
            })?
        } else {
            qb_server_id
        };
        let normalized = SubscriptionCategory {
            name: category.name.trim().to_string(),
            wanted_tag: category.wanted_tag.trim().to_string(),
            qb_server_id,
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
        if !qb_server_ids.contains(&normalized.qb_server_id) {
            return Err(ApiError::bad_request(format!(
                "订阅分类 {} 绑定的 qB 服务器不存在: {}",
                normalized.name, normalized.qb_server_id
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
    cfg.search_interval_secs = cfg.search_interval_secs.max(30);
    cfg.progress_interval_secs = cfg.progress_interval_secs.max(1);
    cfg.link_retry_interval_secs = cfg.link_retry_interval_secs.max(30);
    cfg.system_retry_interval_secs = cfg.system_retry_interval_secs.max(30);
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

#[derive(Deserialize, Default)]
struct OperationLogsQuery {
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
}

async fn operation_logs(
    State(state): State<AppState>,
    Query(q): Query<OperationLogsQuery>,
) -> Result<Json<Value>, ApiError> {
    let page = state
        .wanted_store
        .query_operation_logs(subscription::OperationLogQuery {
            category: q.category,
            status: q.status,
            q: q.q,
            page: q.page,
            page_size: q.page_size,
        })
        .await
        .map_err(|e| ApiError::internal(format!("读取操作日志失败: {e}")))?;
    Ok(Json(
        serde_json::to_value(page).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

#[derive(Deserialize, Default)]
struct WantedStateQuery {
    #[serde(default)]
    log: bool,
}

async fn wanted_subscription_state(
    State(state): State<AppState>,
    Query(q): Query<WantedStateQuery>,
) -> Result<Json<Value>, ApiError> {
    let cookie = state.config.read().await.douban_cookie.clone();
    let account_key = douban::auth_cache_key_fragment(&cookie).map_err(ApiError::douban)?;
    let snapshot = state
        .wanted_store
        .snapshot(&account_key, unix_now_secs())
        .await
        .map_err(|e| ApiError::internal(format!("读取想看订阅状态失败: {e}")))?;
    if q.log {
        write_operation_log(
            &state,
            operation_log_entry(
                account_key.clone(),
                "subscription_sync",
                "refresh_local",
                "subscription_state",
                None,
                None,
                "success",
                format!("刷新本地订阅列表：{} 条记录", snapshot.records.len()),
                None,
                json!({ "record_count": snapshot.records.len() }),
            ),
        )
        .await;
    }
    Ok(Json(
        serde_json::to_value(snapshot).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn wanted_subscription_poll(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let outcome = match run_wanted_watch_poll(&state).await {
        Ok(outcome) => outcome,
        Err(err) => {
            write_operation_log(
                &state,
                operation_log_entry(
                    "system",
                    "subscription_sync",
                    "poll_wanted",
                    "subscription_state",
                    None,
                    None,
                    "failed",
                    "轮询想看失败",
                    Some(err.message().to_string()),
                    json!({ "trigger": "manual" }),
                ),
            )
            .await;
            return Err(err);
        }
    };
    write_operation_log(
        &state,
        operation_log_entry(
            outcome.account_key.clone(),
            "subscription_sync",
            "poll_wanted",
            "subscription_state",
            None,
            None,
            "success",
            format!(
                "轮询想看完成：新增待处理 {}，跳过旧想看 {}，更新已有 {}",
                outcome.created_unprocessed, outcome.created_skipped, outcome.updated_existing
            ),
            None,
            json!({
                "trigger": "manual",
                "total_wish_items": outcome.total_wish_items,
                "created_unprocessed": outcome.created_unprocessed,
                "created_skipped": outcome.created_skipped,
                "updated_existing": outcome.updated_existing,
                "bootstrap_mode": outcome.bootstrap_mode,
            }),
        ),
    )
    .await;
    Ok(Json(
        serde_json::to_value(outcome).map_err(|e| ApiError::internal(e.to_string()))?,
    ))
}

async fn wanted_subscription_retry_current(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
) -> Result<Json<Value>, ApiError> {
    let (_cfg, account_key, record) = load_wanted_record_context(&state, &id).await?;
    if record.lifecycle_state == subscription::SubscriptionLifecycleState::Completed {
        return Err(ApiError::bad_request("当前订阅状态不需要重试"));
    }
    let now = unix_now_secs();
    let (record, operation) = state
        .wanted_store
        .retry_current_node(&account_key, &record.subject_id, now)
        .await
        .map_err(|e| ApiError::internal(format!("更新订阅重试状态失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("当前订阅状态不需要重试"))?;
    let action = operation.as_str();
    Ok(Json(json!({
        "ok": true,
        "action": action,
        "record": record,
    })))
}

async fn wanted_subscription_rerun(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
) -> Result<Json<Value>, ApiError> {
    let (_cfg, account_key, record) = load_wanted_record_context(&state, &id).await?;
    if record.subject_id.trim().is_empty() {
        return Err(ApiError::bad_request("订阅记录缺少 subject_id"));
    }
    let now = unix_now_secs();
    let (record, operation) = state
        .wanted_store
        .rerun_subscription_task(&account_key, &record.subject_id, now)
        .await
        .map_err(|e| ApiError::internal(format!("更新订阅重跑状态失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("当前订阅状态不需要重跑"))?;
    let action = operation.as_str();
    Ok(Json(json!({
        "ok": true,
        "action": action,
        "record": record,
    })))
}

#[derive(Deserialize, Default)]
struct WantedCandidateBody {
    #[serde(default)]
    page_size: Option<u32>,
}

#[derive(Deserialize, Default)]
struct WantedPushBody {
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

#[derive(Deserialize, Default)]
struct WantedProgressBody {
    #[serde(default)]
    qb_server_name: Option<String>,
    #[serde(default)]
    qb_hash: Option<String>,
}

async fn wanted_subscription_candidates(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    Json(body): Json<WantedCandidateBody>,
) -> Result<Json<Value>, ApiError> {
    let (cfg, account_key, record) = load_wanted_record_context(&state, &id).await?;
    let _category = category_for_wanted_record(&record, &cfg.subscription_categories)?;
    let candidates =
        match search_mteam_candidates_for_record(&cfg.mteam_api_key, &record, body.page_size).await
        {
            Ok(candidates) => candidates,
            Err(err) => {
                write_operation_log(
                    &state,
                    operation_log_entry(
                        account_key.clone(),
                        "torrent_search",
                        "match_candidates",
                        "subscription",
                        Some(record.subject_id.clone()),
                        Some(record.title.clone()),
                        "failed",
                        "搜索订阅候选种子失败",
                        Some(err.message().to_string()),
                        json!({ "page_size": body.page_size }),
                    ),
                )
                .await;
                return Err(err);
            }
        };
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

    let selected = matches.iter().find(|item| item.selected).cloned();
    write_operation_log(
        &state,
        operation_log_entry(
            account_key.clone(),
            "torrent_search",
            "match_candidates",
            "subscription",
            Some(record.subject_id.clone()),
            Some(record.title.clone()),
            "success",
            format!(
                "搜索订阅候选种子完成：{} 个候选，{} 个匹配",
                candidates.len(),
                matches.iter().filter(|item| item.selected).count()
            ),
            None,
            json!({
                "candidate_count": candidates.len(),
                "match_count": matches.iter().filter(|item| item.selected).count(),
                "selected_torrent_id": selected.as_ref().map(|item| item.candidate.torrent_id.clone()),
                "torrent_matches": torrent_match_log_entries(&matches),
            }),
        ),
    )
    .await;

    Ok(Json(json!({
        "subscription_id": record.subject_id,
        "candidate_count": candidates.len(),
        "selected": selected,
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
    if !body.force && record.lifecycle_state != subscription::SubscriptionLifecycleState::Searching
    {
        return Err(ApiError::bad_request(format!(
            "订阅 {} 当前生命周期为 {:?}，不会重复推送；需要重试请传 force=true",
            record.subject_id, record.lifecycle_state
        )));
    }
    let category = category_for_wanted_record(&record, &cfg.subscription_categories)?.clone();
    let qb_server = select_qb_server_for_category(&cfg.qb_servers, &category)?;
    let candidates =
        match search_mteam_candidates_for_record(&cfg.mteam_api_key, &record, body.page_size).await
        {
            Ok(candidates) => candidates,
            Err(err) => {
                write_operation_log(
                    &state,
                    operation_log_entry(
                        account_key.clone(),
                        "torrent_search",
                        "match_candidates",
                        "subscription",
                        Some(record.subject_id.clone()),
                        Some(record.title.clone()),
                        "failed",
                        "订阅推送前搜索候选种子失败",
                        Some(err.message().to_string()),
                        json!({
                            "page_size": body.page_size,
                            "qb_server": qb_server_label(&qb_server),
                            "qb_category": category.qb_category,
                        }),
                    ),
                )
                .await;
                return Err(err);
            }
        };
    let matches = match_torrent_candidates(&candidates, &cfg.torrent_match_rules);
    let selected = matches.iter().find(|item| item.selected).cloned();
    let now = unix_now_secs();
    state
        .wanted_store
        .update_candidate_matches(&account_key, &record.subject_id, matches.clone(), now)
        .await
        .map_err(|e| ApiError::internal(format!("写入候选种子记录失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
    write_operation_log(
        &state,
        operation_log_entry(
            account_key.clone(),
            "torrent_search",
            "match_candidates",
            "subscription",
            Some(record.subject_id.clone()),
            Some(record.title.clone()),
            if selected.is_some() { "success" } else { "failed" },
            format!(
                "订阅推送前匹配候选种子完成：{} 个候选，{} 个选中",
                candidates.len(),
                matches.iter().filter(|item| item.selected).count()
            ),
            None,
            json!({
                "candidate_count": candidates.len(),
                "match_count": matches.iter().filter(|item| item.selected).count(),
                "selected_torrent_id": selected.as_ref().map(|item| item.candidate.torrent_id.clone()),
                "qb_server": qb_server_label(&qb_server),
                "qb_category": category.qb_category,
                "torrent_matches": torrent_match_log_entries(&matches),
            }),
        ),
    )
    .await;

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
            &cfg.subscription_watcher,
        )
        .await?;
        write_operation_log(
            &state,
            operation_log_entry(
                account_key.clone(),
                "qb_push",
                "push_torrent",
                "subscription",
                Some(record.subject_id.clone()),
                Some(record.title.clone()),
                "failed",
                "订阅推送 qB 失败：无匹配候选种子",
                Some(error.clone()),
                json!({
                    "candidate_count": candidates.len(),
                    "qb_category": category.qb_category,
                    "torrent_matches": torrent_match_log_entries(&matches),
                }),
            ),
        )
        .await;
        return Err(ApiError::bad_request(error));
    };

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
                    &cfg.subscription_watcher,
                )
                .await?;
                write_operation_log(
                    &state,
                    operation_log_entry(
                        account_key.clone(),
                        "qb_push",
                        "push_torrent",
                        "subscription",
                        Some(record.subject_id.clone()),
                        Some(record.title.clone()),
                        "failed",
                        "订阅推送 qB 失败：M-Team 取链失败",
                        Some(e.message().to_string()),
                        json!({
                            "torrent_id": selected.candidate.torrent_id,
                            "qb_category": category.qb_category,
                        }),
                    ),
                )
                .await;
                return Err(e);
            }
        };

    let mut push = torrent_push_record_with_download_url(
        &record.subject_id,
        &qb_server,
        &category,
        &selected.candidate,
        "pushed",
        Some(unix_now_secs()),
        None,
        Some(&dl_url),
    );
    inherit_existing_qb_lookup(&mut push, record.last_push.as_ref());
    let download_info = match torrent_hash::torrent_download_info_from_url(&dl_url).await {
        Ok(info) => info,
        Err(err) => {
            tracing::warn!(
                torrent_id = %selected.candidate.torrent_id,
                error = %err,
                "calculate torrent infohash before qB add failed"
            );
            torrent_hash::TorrentDownloadInfo {
                info_hashes: Vec::new(),
                torrent_bytes: None,
            }
        }
    };
    let precomputed_hash = download_info.info_hashes.first().cloned();
    if let Some(hash) = precomputed_hash.as_deref() {
        if let Ok(lookup) = find_qb_torrent_for_push(&qb_server, &push, Some(hash)).await {
            apply_existing_qb_lookup_to_push(&mut push, &lookup.torrent);
            let record = state
                .wanted_store
                .apply_movie_push_result(
                    &account_key,
                    &record.subject_id,
                    push.clone(),
                    false,
                    None,
                    &cfg.subscription_watcher,
                    unix_now_secs(),
                )
                .await
                .map_err(|e| ApiError::internal(format!("写入 qB 推送记录失败: {e}")))?
                .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;

            write_operation_log(
                &state,
                operation_log_entry(
                    account_key.clone(),
                    "qb_push",
                    "push_torrent",
                    "subscription",
                    Some(record.subject_id.clone()),
                    Some(record.title.clone()),
                    "success",
                    format!("订阅种子已存在于 qB，跳过添加：{}", push.torrent_title),
                    None,
                    json!({
                        "torrent_id": push.torrent_id.clone(),
                        "torrent_title": push.torrent_title.clone(),
                        "precomputed_info_hash": hash,
                        "qb_hash": push.qb_hash.clone(),
                        "qb_name": push.qb_name.clone(),
                        "qb_server": push.qb_server.clone(),
                        "qb_category": push.qb_category.clone(),
                        "qb_save_dir_name": push.qb_save_dir_name.clone(),
                    }),
                ),
            )
            .await;

            return Ok(Json(json!({
                "ok": true,
                "already_in_qb": true,
                "subscription_id": record.subject_id,
                "selected": selected,
                "push": push,
                "record": record,
            })));
        }
    }

    let qb_tags = qb_tags_for_torrent_id(&selected.candidate.torrent_id);
    let add_result = if let Some(bytes) = download_info.torrent_bytes {
        qbittorrent::add_torrent_bytes_with_tags(
            &qb_server,
            &format!("mteam-{}.torrent", selected.candidate.torrent_id.trim()),
            bytes,
            Some(&category.qb_category),
            Some(&category.qb_save_dir_name),
            &qb_tags,
        )
        .await
    } else {
        qbittorrent::add_torrent_from_url_with_tags(
            &qb_server,
            &dl_url,
            Some(&category.qb_category),
            Some(&category.qb_save_dir_name),
            &qb_tags,
        )
        .await
    };
    if let Err(e) = add_result {
        record_push_failure(
            &state,
            &account_key,
            &record.subject_id,
            &qb_server,
            &category,
            Some(&selected),
            e.message().to_string(),
            &cfg.subscription_watcher,
        )
        .await?;
        write_operation_log(
            &state,
            operation_log_entry(
                account_key.clone(),
                "qb_push",
                "push_torrent",
                "subscription",
                Some(record.subject_id.clone()),
                Some(record.title.clone()),
                "failed",
                "订阅推送 qB 失败：qB 添加种子失败",
                Some(e.message().to_string()),
                json!({
                    "torrent_id": selected.candidate.torrent_id,
                    "qb_category": category.qb_category,
                }),
            ),
        )
        .await;
        return Err(e);
    }

    if let Ok(lookup) =
        find_qb_torrent_for_push(&qb_server, &push, precomputed_hash.as_deref()).await
    {
        apply_existing_qb_lookup_to_push(&mut push, &lookup.torrent);
    }
    let record = state
        .wanted_store
        .apply_movie_push_result(
            &account_key,
            &record.subject_id,
            push.clone(),
            false,
            None,
            &cfg.subscription_watcher,
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入 qB 推送记录失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;

    write_operation_log(
        &state,
        operation_log_entry(
            account_key.clone(),
            "qb_push",
            "push_torrent",
            "subscription",
            Some(record.subject_id.clone()),
            Some(record.title.clone()),
            "success",
            format!("已推送订阅种子到 qB：{}", push.torrent_title),
            None,
            json!({
                "torrent_id": push.torrent_id.clone(),
                "torrent_title": push.torrent_title.clone(),
                "qb_server": push.qb_server.clone(),
                "qb_category": push.qb_category.clone(),
                "qb_save_dir_name": push.qb_save_dir_name.clone(),
            }),
        ),
    )
    .await;

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
    let now = unix_now_secs();
    if subscription_completion_already_linked(&record) && !body.force {
        return Ok(Json(json!({
            "ok": true,
            "already_completed": true,
            "record": record,
        })));
    }

    let category = category_for_wanted_record(&record, &cfg.subscription_categories)?.clone();
    let mut push = match record.last_push.clone() {
        Some(push) => push,
        None => {
            let error = "订阅记录缺少 qB pushed record".to_string();
            let record = state
                .wanted_store
                .apply_parent_operation_failure_result(
                    &account_key,
                    &record.subject_id,
                    "link",
                    &error,
                    &cfg.subscription_watcher,
                    now,
                )
                .await
                .map_err(|e| ApiError::internal(format!("写入硬链接前置错误失败: {e}")))?
                .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
            write_operation_log(
                &state,
                operation_log_entry(
                    account_key.clone(),
                    "hardlink",
                    "link_result",
                    "subscription",
                    Some(record.subject_id.clone()),
                    Some(record.title.clone()),
                    "failed",
                    "硬链接失败：订阅记录缺少 qB pushed record",
                    Some(error.clone()),
                    json!({
                        "operation": "link",
                        "reason": "missing_last_push",
                    }),
                ),
            )
            .await;
            return Err(ApiError::bad_request(error));
        }
    };
    if record
        .failure
        .as_ref()
        .is_some_and(|failure| failure.operation == "search")
        && !body.force
    {
        let error = "最后一次 qB push 已失败；需要重新检查请传 force=true".to_string();
        persist_completion_sync_error(&state, &account_key, &record.subject_id, push, &error, now)
            .await?;
        return Err(ApiError::bad_request(error));
    }
    if push.torrent_id.trim().is_empty() && body.qb_hash.as_deref().unwrap_or("").trim().is_empty()
    {
        let error = "pushed record 缺少种子 id，且未提供 qb_hash".to_string();
        persist_completion_sync_error(&state, &account_key, &record.subject_id, push, &error, now)
            .await?;
        return Err(ApiError::bad_request(error));
    }

    let qb_server =
        match select_qb_server_for_push(&cfg.qb_servers, &push, body.qb_server_name.as_deref()) {
            Ok(qb_server) => qb_server,
            Err(err) => {
                let error = err.message().to_string();
                persist_completion_sync_error(
                    &state,
                    &account_key,
                    &record.subject_id,
                    push,
                    &error,
                    now,
                )
                .await?;
                return Err(err);
            }
        };
    let qb_torrent =
        match find_qb_torrent_for_push(&qb_server, &push, body.qb_hash.as_deref()).await {
            Ok(lookup) => lookup.torrent,
            Err(err) => {
                let error = err.message().to_string();
                persist_completion_sync_error(
                    &state,
                    &account_key,
                    &record.subject_id,
                    push,
                    &error,
                    now,
                )
                .await?;
                return Err(err);
            }
        };
    if !qb_torrent.is_complete() {
        push.checked_at = Some(now);
        push.qb_hash = Some(qb_torrent.hash.clone());
        push.qb_name = Some(qb_torrent.name.clone());
        apply_qb_progress_to_push(&mut push, &qb_torrent, &[]);
        mark_push_progress_success(&mut push, false);

        let completion = subscription::HardlinkCompletionRecord {
            status: "pending".to_string(),
            checked_at: now,
            completed_at: None,
            qb_hash: Some(qb_torrent.hash.clone()),
            qb_name: Some(qb_torrent.name.clone()),
            source_path: None,
            target_dir: None,
            linked_files: Vec::new(),
            episodes: push.episodes.clone(),
            error: None,
        };
        let record = state
            .wanted_store
            .apply_movie_completion_result(
                &account_key,
                &record.subject_id,
                push.clone(),
                completion.clone(),
                subscription::MovieCompletionOutcome::PendingDownload,
                None,
                &cfg.subscription_watcher,
                now,
            )
            .await
            .map_err(|e| ApiError::internal(format!("写入 qB 完成检查记录失败: {e}")))?
            .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
        write_operation_log(
            &state,
            operation_log_entry(
                account_key.clone(),
                "download_progress",
                "check_completion",
                "subscription",
                Some(record.subject_id.clone()),
                Some(record.title.clone()),
                "processing",
                "完成检测：qB 种子仍在下载中",
                None,
                json!({
                    "dry_run": body.dry_run,
                    "qb_hash": qb_torrent.hash,
                    "qb_name": qb_torrent.name,
                    "download_progress": push.download_progress,
                }),
            ),
        )
        .await;
        return Ok(Json(json!({
            "ok": true,
            "completed": false,
            "dry_run": body.dry_run,
            "completion": completion,
            "progress": push,
            "record": record,
        })));
    }
    let qb_files = match qbittorrent::torrent_files(&qb_server, &qb_torrent.hash).await {
        Ok(qb_files) => qb_files,
        Err(err) => {
            let error = err.message().to_string();
            push.qb_hash = Some(qb_torrent.hash.clone());
            push.qb_name = Some(qb_torrent.name.clone());
            persist_completion_sync_error(
                &state,
                &account_key,
                &record.subject_id,
                push,
                &error,
                now,
            )
            .await?;
            return Err(err);
        }
    };

    push.checked_at = Some(now);
    push.qb_hash = Some(qb_torrent.hash.clone());
    push.qb_name = Some(qb_torrent.name.clone());
    apply_qb_progress_to_push(&mut push, &qb_torrent, &qb_files);
    mark_push_progress_success(&mut push, qb_torrent.is_complete());

    let plan = match build_hardlink_plan(&record, &category, &push, &qb_torrent, &qb_files, now) {
        Ok(plan) => plan,
        Err(err) => {
            let error = err.message().to_string();
            persist_hardlink_sync_error(
                &state,
                &account_key,
                &record.subject_id,
                push,
                &error,
                now,
            )
            .await?;
            return Err(err);
        }
    };
    let completion = if body.dry_run {
        dry_run_hardlink_plan(&plan, now)
    } else {
        execute_hardlink_plan(&plan, now)
    };
    let completed = completion.error.is_none() && completion.completed_at.is_some();
    push.status = if completed {
        "linked".to_string()
    } else {
        completion.status.clone()
    };
    push.error = completion.error.clone();
    push.completed_at = completion.completed_at;
    push.source_path = completion.source_path.clone();
    push.target_dir = completion.target_dir.clone();
    push.linked_files = completion.linked_files.clone();

    let record = state
        .wanted_store
        .apply_movie_completion_result(
            &account_key,
            &record.subject_id,
            push.clone(),
            completion.clone(),
            if body.dry_run {
                subscription::MovieCompletionOutcome::LinkPlanned
            } else if completed {
                subscription::MovieCompletionOutcome::Completed
            } else {
                subscription::MovieCompletionOutcome::LinkFailed
            },
            if body.dry_run {
                None
            } else {
                completion.error.clone()
            },
            &cfg.subscription_watcher,
            now,
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入硬链接结果失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;

    write_operation_log(
        &state,
        operation_log_entry(
            account_key.clone(),
            "hardlink",
            "link_result",
            "subscription",
            Some(record.subject_id.clone()),
            Some(record.title.clone()),
            if completed || body.dry_run {
                "success"
            } else {
                "failed"
            },
            if body.dry_run {
                format!("硬链接 dry-run 完成：计划 {} 个文件", plan.files.len())
            } else if completed {
                format!("硬链接完成：{} 个文件", completion.linked_files.len())
            } else {
                "硬链接失败".to_string()
            },
            completion.error.clone(),
            json!({
                "dry_run": body.dry_run,
                "completed": completed,
                "status": completion.status.clone(),
                "file_count": completion.linked_files.len(),
                "plan_file_count": plan.files.len(),
                "qb_hash": completion.qb_hash.clone(),
                "qb_name": completion.qb_name.clone(),
                "target_dir": completion.target_dir.clone(),
            }),
        ),
    )
    .await;

    Ok(Json(json!({
        "ok": completed || body.dry_run,
        "completed": completed,
        "dry_run": body.dry_run,
        "completion": completion,
        "push": push,
        "plan_file_count": plan.files.len(),
        "record": record,
    })))
}

fn subscription_completion_already_linked(record: &subscription::WantedSubscriptionRecord) -> bool {
    record.lifecycle_state == subscription::SubscriptionLifecycleState::Completed
}

async fn wanted_subscription_progress(
    State(state): State<AppState>,
    PathParam(id): PathParam<String>,
    Json(body): Json<WantedProgressBody>,
) -> Result<Json<Value>, ApiError> {
    let (cfg, account_key, record) = load_wanted_record_context(&state, &id).await?;
    let now = unix_now_secs();
    let mut push = match record.last_push.clone() {
        Some(push) => push,
        None => {
            let error = "订阅记录缺少 qB pushed record".to_string();
            let record = state
                .wanted_store
                .apply_parent_operation_failure_result(
                    &account_key,
                    &record.subject_id,
                    "progress",
                    &error,
                    &cfg.subscription_watcher,
                    now,
                )
                .await
                .map_err(|e| ApiError::internal(format!("写入下载进度前置错误失败: {e}")))?
                .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
            write_operation_log(
                &state,
                operation_log_entry(
                    account_key.clone(),
                    "download_progress",
                    "sync_progress",
                    "subscription",
                    Some(record.subject_id.clone()),
                    Some(record.title.clone()),
                    "failed",
                    "同步 qB 下载进度失败：订阅记录缺少 qB pushed record",
                    Some(error.clone()),
                    json!({
                        "operation": "progress",
                        "reason": "missing_last_push",
                    }),
                ),
            )
            .await;
            return Err(ApiError::bad_request(error));
        }
    };
    let qb_server =
        match select_qb_server_for_push(&cfg.qb_servers, &push, body.qb_server_name.as_deref()) {
            Ok(qb_server) => qb_server,
            Err(err) => {
                let error = err.message().to_string();
                persist_progress_sync_error(
                    &state,
                    &account_key,
                    &record.subject_id,
                    push,
                    &error,
                    now,
                )
                .await?;
                return Err(err);
            }
        };
    let qb_torrent =
        match find_qb_torrent_for_push(&qb_server, &push, body.qb_hash.as_deref()).await {
            Ok(lookup) => lookup.torrent,
            Err(err) => {
                let error = err.message().to_string();
                persist_progress_sync_error(
                    &state,
                    &account_key,
                    &record.subject_id,
                    push,
                    &error,
                    now,
                )
                .await?;
                return Err(err);
            }
        };
    let mut file_list_error = None;
    let qb_files = if qb_torrent.is_complete() {
        match qbittorrent::torrent_files(&qb_server, &qb_torrent.hash).await {
            Ok(qb_files) => qb_files,
            Err(err) => {
                let error = err.message().to_string();
                tracing::warn!(
                    subject_id = %record.subject_id,
                    qb_hash = %qb_torrent.hash,
                    "qB file list unavailable while syncing progress: {error}"
                );
                file_list_error = Some(error);
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    push.checked_at = Some(now);
    push.qb_hash = Some(qb_torrent.hash.clone());
    push.qb_name = Some(qb_torrent.name.clone());
    apply_qb_progress_to_push(&mut push, &qb_torrent, &qb_files);
    mark_push_progress_success(&mut push, qb_torrent.is_complete());

    let record = state
        .wanted_store
        .apply_movie_progress_result(
            &account_key,
            &record.subject_id,
            push.clone(),
            qb_torrent.is_complete(),
            None,
            &cfg.subscription_watcher,
            now,
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入 qB 下载进度失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;

    write_operation_log(
        &state,
        operation_log_entry(
            account_key.clone(),
            "download_progress",
            "sync_progress",
            "subscription",
            Some(record.subject_id.clone()),
            Some(record.title.clone()),
            "success",
            format!(
                "同步 qB 下载进度：{}",
                format!("{:.0}%", push.download_progress.unwrap_or_default() * 100.0)
            ),
            None,
            json!({
                "completed": qb_torrent.is_complete(),
                "download_progress": push.download_progress,
                "download_state": push.download_state.clone(),
                "qb_hash": push.qb_hash.clone(),
                "qb_name": push.qb_name.clone(),
                "file_list_error": file_list_error,
            }),
        ),
    )
    .await;

    Ok(Json(json!({
        "ok": true,
        "completed": qb_torrent.is_complete(),
        "progress": push,
        "record": record,
    })))
}

async fn persist_progress_sync_error(
    state: &AppState,
    account_key: &str,
    subject_id: &str,
    mut push: subscription::TorrentPushRecord,
    error: &str,
    now: u64,
) -> Result<subscription::WantedSubscriptionRecord, ApiError> {
    let cfg = state.config.read().await.clone();
    push.checked_at = Some(now);
    push.status = "failed".to_string();
    push.error = Some(error.to_string());
    let record = state
        .wanted_store
        .apply_movie_progress_result(
            account_key,
            subject_id,
            push.clone(),
            false,
            Some(error.to_string()),
            &cfg.subscription_watcher,
            now,
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入 qB 下载进度错误失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
    write_operation_log(
        state,
        operation_log_entry(
            account_key.to_string(),
            "download_progress",
            "sync_progress",
            "subscription",
            Some(record.subject_id.clone()),
            Some(record.title.clone()),
            "failed",
            "同步 qB 下载进度失败",
            Some(error.to_string()),
            json!({
                "qb_hash": push.qb_hash,
                "qb_name": push.qb_name,
                "qb_category": push.qb_category,
            }),
        ),
    )
    .await;
    Ok(record)
}

async fn persist_completion_sync_error(
    state: &AppState,
    account_key: &str,
    subject_id: &str,
    mut push: subscription::TorrentPushRecord,
    error: &str,
    now: u64,
) -> Result<subscription::WantedSubscriptionRecord, ApiError> {
    let cfg = state.config.read().await.clone();
    push.checked_at = Some(now);
    push.status = "failed".to_string();
    push.error = Some(error.to_string());
    let completion = subscription::HardlinkCompletionRecord {
        status: "failed".to_string(),
        checked_at: now,
        completed_at: None,
        qb_hash: push.qb_hash.clone(),
        qb_name: push.qb_name.clone(),
        source_path: push.source_path.clone(),
        target_dir: push.target_dir.clone(),
        linked_files: push.linked_files.clone(),
        episodes: push.episodes.clone(),
        error: Some(error.to_string()),
    };
    let record = state
        .wanted_store
        .apply_movie_completion_result(
            account_key,
            subject_id,
            push.clone(),
            completion.clone(),
            subscription::MovieCompletionOutcome::LinkFailed,
            Some(error.to_string()),
            &cfg.subscription_watcher,
            now,
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入 qB 完成检查错误失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
    write_operation_log(
        state,
        operation_log_entry(
            account_key.to_string(),
            "completion",
            "check_completion",
            "subscription",
            Some(record.subject_id.clone()),
            Some(record.title.clone()),
            "failed",
            "完成检测失败",
            Some(error.to_string()),
            json!({
                "qb_hash": completion.qb_hash,
                "qb_name": completion.qb_name,
                "qb_category": push.qb_category,
            }),
        ),
    )
    .await;
    Ok(record)
}

async fn persist_hardlink_sync_error(
    state: &AppState,
    account_key: &str,
    subject_id: &str,
    mut push: subscription::TorrentPushRecord,
    error: &str,
    now: u64,
) -> Result<subscription::WantedSubscriptionRecord, ApiError> {
    let cfg = state.config.read().await.clone();
    push.checked_at = Some(now);
    push.status = "failed".to_string();
    push.error = Some(error.to_string());
    let completion = subscription::HardlinkCompletionRecord {
        status: "failed".to_string(),
        checked_at: now,
        completed_at: None,
        qb_hash: push.qb_hash.clone(),
        qb_name: push.qb_name.clone(),
        source_path: push.source_path.clone(),
        target_dir: push.target_dir.clone(),
        linked_files: push.linked_files.clone(),
        episodes: push.episodes.clone(),
        error: Some(error.to_string()),
    };
    let record = state
        .wanted_store
        .apply_movie_completion_result(
            account_key,
            subject_id,
            push.clone(),
            completion.clone(),
            subscription::MovieCompletionOutcome::LinkFailed,
            Some(error.to_string()),
            &cfg.subscription_watcher,
            now,
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入硬链接错误失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
    write_operation_log(
        state,
        operation_log_entry(
            account_key.to_string(),
            "hardlink",
            "link_result",
            "subscription",
            Some(record.subject_id.clone()),
            Some(record.title.clone()),
            "failed",
            "硬链接失败",
            Some(error.to_string()),
            json!({
                "qb_hash": completion.qb_hash,
                "qb_name": completion.qb_name,
                "qb_category": push.qb_category,
            }),
        ),
    )
    .await;
    Ok(record)
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
        servers.iter().find(|server| {
            server.id.trim() == name || server.name.trim() == name || server.base_url.trim() == name
        })
    } else {
        servers.first()
    };
    server
        .cloned()
        .ok_or_else(|| ApiError::bad_request("请先在设置中配置 qB 服务器"))
}

fn select_qb_server_for_category(
    servers: &[QbServerEntry],
    category: &SubscriptionCategory,
) -> Result<QbServerEntry, ApiError> {
    let id = category.qb_server_id.trim();
    if id.is_empty() {
        return servers
            .first()
            .cloned()
            .ok_or_else(|| ApiError::bad_request("请先在设置中配置 qB 服务器"));
    }
    servers
        .iter()
        .find(|server| server.id.trim() == id)
        .cloned()
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "订阅分类 {} 绑定的 qB 服务器不存在: {}",
                category.name, id
            ))
        })
}

fn select_qb_server_for_push(
    servers: &[QbServerEntry],
    push: &subscription::TorrentPushRecord,
    requested: Option<&str>,
) -> Result<QbServerEntry, ApiError> {
    let selector = requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            let id = push.qb_server_id.trim();
            (!id.is_empty()).then_some(id)
        })
        .or_else(|| {
            let name = push.qb_server.trim();
            (!name.is_empty()).then_some(name)
        });
    select_qb_server(servers, selector)
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
        let body = mteam_search_body(1, page_size, "douban", &douban);
        let response = mteam_search_post(&client, key, &body).await?;
        append_unique_candidates(
            &mut candidates,
            &mut seen,
            mteam_candidates_from_response(&response, "douban", subject_id),
        );
    }

    let title = record.title.trim();
    if !title.is_empty() {
        let body = mteam_search_body(1, page_size, "keyword", title);
        let response = mteam_search_post(&client, key, &body).await?;
        append_unique_candidates(
            &mut candidates,
            &mut seen,
            mteam_candidates_from_response(&response, "keyword", title),
        );
    }

    sort_torrent_candidates_by_seeders(&mut candidates);
    Ok(candidates)
}

fn sort_torrent_candidates_by_seeders(candidates: &mut [subscription::TorrentCandidateRecord]) {
    candidates.sort_by(|a, b| b.seeders.unwrap_or(0).cmp(&a.seeders.unwrap_or(0)));
}

fn mteam_search_body(page: u32, page_size: u32, query_field: &str, query_value: &str) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("pageNumber".to_string(), json!(page));
    body.insert("pageSize".to_string(), json!(page_size));
    body.insert("sortField".to_string(), json!("SEEDERS"));
    body.insert("sortDirection".to_string(), json!("DESC"));
    body.insert(query_field.to_string(), json!(query_value));
    Value::Object(body)
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

fn torrent_match_log_entries(matches: &[subscription::TorrentCandidateMatchRecord]) -> Value {
    Value::Array(
        matches
            .iter()
            .map(|item| {
                json!({
                    "torrent_id": item.candidate.torrent_id,
                    "title": item.candidate.title,
                    "subtitle": item.candidate.subtitle,
                    "source": item.candidate.source,
                    "search_query": item.candidate.search_query,
                    "size": item.candidate.size,
                    "seeders": item.candidate.seeders,
                    "leechers": item.candidate.leechers,
                    "uploaded_at": item.candidate.uploaded_at,
                    "selected": item.selected,
                    "matched_rule_name": item.matched_rule_name,
                    "matched_priority": item.matched_priority,
                    "matched_keywords": item.matched_keywords,
                    "excluded_reason": item.excluded_reason,
                    "rule_evaluations": item.rule_evaluations.iter().map(|evaluation| {
                        json!({
                            "rule_name": evaluation.rule_name,
                            "priority": evaluation.priority,
                            "mode": evaluation.mode,
                            "matched": evaluation.matched,
                            "matched_keywords": evaluation.matched_keywords,
                            "missing_keywords": evaluation.missing_keywords,
                            "excluded_reason": evaluation.excluded_reason,
                        })
                    }).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
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
    watcher_cfg: &SubscriptionWatcherConfig,
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
        .apply_movie_push_result(
            account_key,
            subscription_id,
            push,
            true,
            Some(error),
            watcher_cfg,
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入 qB 推送失败记录失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
struct EpisodeMarker {
    season_number: Option<u32>,
    episode_number: Option<u32>,
    episode_end_number: Option<u32>,
    label: Option<String>,
}

#[derive(Debug, Clone)]
struct EpisodeGroupKey {
    season_number: Option<u32>,
    episode_number: Option<u32>,
    label: String,
}

fn apply_qb_progress_to_push(
    push: &mut subscription::TorrentPushRecord,
    torrent: &qbittorrent::QbTorrentInfo,
    files: &[qbittorrent::QbTorrentFile],
) {
    let file_records = torrent_file_progress_records(files);
    let total_size = if torrent.size > 0 {
        torrent.size
    } else {
        file_records.iter().map(|file| file.size).sum()
    };

    push.download_progress = Some(torrent.progress.clamp(0.0, 1.0));
    push.download_state = (!torrent.state.trim().is_empty()).then(|| torrent.state.clone());
    if total_size > 0 {
        push.total_size = Some(total_size);
    }
    if !files.is_empty() {
        let total_file_count = file_records.len();
        let completed_file_count = file_records
            .iter()
            .filter(|file| file.progress >= 0.999_999)
            .count();
        push.total_file_count = Some(total_file_count);
        push.completed_file_count = Some(completed_file_count);
        push.files = file_records;
        push.episodes = episode_records_from_file_progress(&push.files);
    }
}

fn mark_push_progress_success(push: &mut subscription::TorrentPushRecord, completed: bool) {
    push.status = if completed {
        "downloaded".to_string()
    } else {
        "downloading".to_string()
    };
    push.error = None;
}

fn torrent_file_progress_records(
    files: &[qbittorrent::QbTorrentFile],
) -> Vec<subscription::TorrentFileProgressRecord> {
    let media_files = files
        .iter()
        .filter(|file| should_link_media_file(&file.name))
        .collect::<Vec<_>>();
    let media_file_count = media_files.len();
    media_files
        .into_iter()
        .map(|file| {
            let episode = episode_marker_for_file_name(&file.name, media_file_count);
            subscription::TorrentFileProgressRecord {
                name: file.name.clone(),
                size: file.size,
                progress: file.progress.clamp(0.0, 1.0),
                priority: file.priority,
                season_number: episode.season_number,
                episode_number: episode.episode_number,
                episode_end_number: episode.episode_end_number,
                episode_label: episode.label,
            }
        })
        .collect()
}

fn episode_records_from_file_progress(
    files: &[subscription::TorrentFileProgressRecord],
) -> Vec<subscription::EpisodeProgressRecord> {
    let mut grouped: BTreeMap<String, Vec<&subscription::TorrentFileProgressRecord>> =
        BTreeMap::new();
    let mut metadata: BTreeMap<String, (Option<u32>, Option<u32>)> = BTreeMap::new();
    for file in files {
        for key in episode_group_keys_for_progress_file(file) {
            metadata
                .entry(key.label.clone())
                .or_insert((key.season_number, key.episode_number));
            grouped.entry(key.label).or_default().push(file);
        }
    }
    add_missing_episode_groups(&mut grouped, &mut metadata);
    let conflict_seasons = conflicted_episode_seasons(&metadata);
    grouped
        .into_iter()
        .map(|(label, rows)| {
            let (season_number, episode_number) =
                metadata.get(&label).copied().unwrap_or((None, None));
            let file_count = rows.len();
            let completed_file_count = rows
                .iter()
                .filter(|file| file.progress >= 0.999_999)
                .count();
            let progress = if file_count == 0 {
                0.0
            } else {
                rows.iter().map(|file| file.progress).sum::<f64>() / file_count as f64
            };
            subscription::EpisodeProgressRecord {
                season_number,
                episode_number,
                label,
                file_count,
                completed_file_count,
                linked_file_count: 0,
                failed_file_count: 0,
                progress,
                status: progress_episode_status(
                    season_number,
                    episode_number,
                    &rows,
                    completed_file_count,
                    progress,
                    &conflict_seasons,
                ),
            }
        })
        .collect()
}

fn episode_records_from_hardlink_files(
    files: &[subscription::HardlinkFileRecord],
) -> Vec<subscription::EpisodeProgressRecord> {
    let mut grouped: BTreeMap<String, Vec<&subscription::HardlinkFileRecord>> = BTreeMap::new();
    let mut metadata: BTreeMap<String, (Option<u32>, Option<u32>)> = BTreeMap::new();
    for file in files {
        for key in episode_group_keys_for_hardlink_file(file) {
            metadata
                .entry(key.label.clone())
                .or_insert((key.season_number, key.episode_number));
            grouped.entry(key.label).or_default().push(file);
        }
    }
    add_missing_episode_groups(&mut grouped, &mut metadata);
    let conflict_seasons = conflicted_episode_seasons(&metadata);
    grouped
        .into_iter()
        .map(|(label, rows)| {
            let (season_number, episode_number) =
                metadata.get(&label).copied().unwrap_or((None, None));
            let file_count = rows.len();
            let planned_file_count = rows
                .iter()
                .filter(|file| file.status.as_str() == "planned")
                .count();
            let linked_file_count = rows
                .iter()
                .filter(|file| {
                    matches!(
                        file.status.as_str(),
                        "linked" | "already_linked" | "planned"
                    )
                })
                .count();
            let failed_file_count = rows
                .iter()
                .filter(|file| file.status.as_str() == "failed")
                .count();
            subscription::EpisodeProgressRecord {
                season_number,
                episode_number,
                label,
                file_count,
                completed_file_count: linked_file_count,
                linked_file_count,
                failed_file_count,
                progress: if file_count == 0 {
                    0.0
                } else {
                    linked_file_count as f64 / file_count as f64
                },
                status: hardlink_episode_status(
                    season_number,
                    episode_number,
                    &rows,
                    planned_file_count,
                    linked_file_count,
                    failed_file_count,
                    &conflict_seasons,
                ),
            }
        })
        .collect()
}

const UNKNOWN_EPISODE_LABEL: &str = "未识别分集";

fn episode_group_keys_for_progress_file(
    file: &subscription::TorrentFileProgressRecord,
) -> Vec<EpisodeGroupKey> {
    episode_group_keys(
        file.season_number,
        file.episode_number,
        file.episode_end_number,
        file.episode_label.as_deref(),
    )
}

fn episode_group_keys_for_hardlink_file(
    file: &subscription::HardlinkFileRecord,
) -> Vec<EpisodeGroupKey> {
    episode_group_keys(
        file.season_number,
        file.episode_number,
        file.episode_end_number,
        file.episode_label.as_deref(),
    )
}

fn episode_group_keys(
    season_number: Option<u32>,
    episode_number: Option<u32>,
    episode_end_number: Option<u32>,
    episode_label: Option<&str>,
) -> Vec<EpisodeGroupKey> {
    if let Some(start) = episode_number {
        let end = episode_end_number
            .filter(|end| *end >= start)
            .map(|end| end.min(start.saturating_add(200)))
            .unwrap_or(start);
        return (start..=end)
            .map(|episode| EpisodeGroupKey {
                season_number,
                episode_number: Some(episode),
                label: episode_number_label(season_number, episode),
            })
            .collect();
    }
    episode_label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(|label| {
            vec![EpisodeGroupKey {
                season_number,
                episode_number,
                label: label.to_string(),
            }]
        })
        .unwrap_or_default()
}

fn episode_number_label(season_number: Option<u32>, episode_number: u32) -> String {
    if let Some(season) = season_number {
        format!("S{season:02}E{episode_number:02}")
    } else {
        format!("E{episode_number:02}")
    }
}

fn add_missing_episode_groups<T>(
    grouped: &mut BTreeMap<String, Vec<T>>,
    metadata: &mut BTreeMap<String, (Option<u32>, Option<u32>)>,
) {
    let mut by_season: BTreeMap<Option<u32>, BTreeSet<u32>> = BTreeMap::new();
    for (season, episode) in metadata.values().copied() {
        if let Some(episode) = episode {
            by_season.entry(season).or_default().insert(episode);
        }
    }
    for (season, episodes) in by_season {
        if episodes.len() < 2 {
            continue;
        }
        let Some(first) = episodes.iter().next().copied() else {
            continue;
        };
        let Some(last) = episodes.iter().next_back().copied() else {
            continue;
        };
        for episode in first..=last {
            if episodes.contains(&episode) {
                continue;
            }
            let label = episode_number_label(season, episode);
            metadata
                .entry(label.clone())
                .or_insert((season, Some(episode)));
            grouped.entry(label).or_default();
        }
    }
}

fn conflicted_episode_seasons(
    metadata: &BTreeMap<String, (Option<u32>, Option<u32>)>,
) -> BTreeSet<u32> {
    let mut season_packs = BTreeSet::new();
    let mut numbered = BTreeSet::new();
    for (label, (season, episode)) in metadata {
        if let Some(season) = season {
            if episode.is_some() {
                numbered.insert(*season);
            } else if label.contains("全季") {
                season_packs.insert(*season);
            }
        }
    }
    season_packs.intersection(&numbered).copied().collect()
}

fn progress_episode_status(
    season_number: Option<u32>,
    episode_number: Option<u32>,
    rows: &[&subscription::TorrentFileProgressRecord],
    completed_file_count: usize,
    progress: f64,
    conflict_seasons: &BTreeSet<u32>,
) -> String {
    if rows.is_empty() {
        return "missing".to_string();
    }
    if rows
        .iter()
        .any(|file| file.episode_label.as_deref() == Some(UNKNOWN_EPISODE_LABEL))
    {
        return "needs_review".to_string();
    }
    if season_number.is_some_and(|season| conflict_seasons.contains(&season)) {
        return "conflict".to_string();
    }
    if episode_number.is_some() && rows.len() > 1 {
        return "duplicate".to_string();
    }
    if completed_file_count == rows.len() {
        "downloaded".to_string()
    } else if progress > 0.0 {
        "downloading".to_string()
    } else {
        "pending".to_string()
    }
}

fn hardlink_episode_status(
    season_number: Option<u32>,
    episode_number: Option<u32>,
    rows: &[&subscription::HardlinkFileRecord],
    planned_file_count: usize,
    linked_file_count: usize,
    failed_file_count: usize,
    conflict_seasons: &BTreeSet<u32>,
) -> String {
    if rows.is_empty() {
        return "missing".to_string();
    }
    if rows
        .iter()
        .any(|file| file.episode_label.as_deref() == Some(UNKNOWN_EPISODE_LABEL))
    {
        return "needs_review".to_string();
    }
    if season_number.is_some_and(|season| conflict_seasons.contains(&season)) {
        return "conflict".to_string();
    }
    if episode_number.is_some() && rows.len() > 1 {
        return "duplicate".to_string();
    }
    if failed_file_count > 0 {
        "failed".to_string()
    } else if planned_file_count == rows.len() {
        "planned".to_string()
    } else if linked_file_count == rows.len() {
        "linked".to_string()
    } else {
        "pending".to_string()
    }
}

fn episode_marker_for_file_name(name: &str, media_file_count: usize) -> EpisodeMarker {
    let mut marker = episode_marker_from_name(name);
    if marker.label.is_none() && should_flag_unknown_episode(name, media_file_count) {
        marker.label = Some(UNKNOWN_EPISODE_LABEL.to_string());
    }
    marker
}

fn episode_marker_from_name(name: &str) -> EpisodeMarker {
    let lower = name.to_ascii_lowercase();
    if let Some((season, episode, last_episode)) = find_season_episode_marker(&lower) {
        let label = if let Some(last) = last_episode.filter(|last| *last > episode) {
            format!("S{season:02}E{episode:02}-E{last:02}")
        } else {
            format!("S{season:02}E{episode:02}")
        };
        return EpisodeMarker {
            season_number: Some(season),
            episode_number: Some(episode),
            episode_end_number: last_episode.filter(|last| *last > episode),
            label: Some(label),
        };
    }
    if has_season_pack_keyword(&lower) {
        if let Some(season) = find_season_pack_marker(&lower) {
            return EpisodeMarker {
                season_number: Some(season),
                episode_number: None,
                episode_end_number: None,
                label: Some(format!("S{season:02} 全季")),
            };
        }
    }
    if let Some((episode, last_episode)) = find_bare_episode_marker(&lower)
        .map(|(episode, last)| (episode, last))
        .or_else(|| find_chinese_episode_marker(name).map(|episode| (episode, None)))
        .or_else(|| find_plain_episode_marker(&lower))
    {
        let label = if let Some(last) = last_episode.filter(|last| *last > episode) {
            format!("E{episode:02}-E{last:02}")
        } else {
            format!("E{episode:02}")
        };
        return EpisodeMarker {
            season_number: None,
            episode_number: Some(episode),
            episode_end_number: last_episode.filter(|last| *last > episode),
            label: Some(label),
        };
    }
    if let Some(season) = find_season_pack_marker(&lower) {
        return EpisodeMarker {
            season_number: Some(season),
            episode_number: None,
            episode_end_number: None,
            label: Some(format!("S{season:02} 全季")),
        };
    }
    EpisodeMarker::default()
}

fn has_season_pack_keyword(text: &str) -> bool {
    text.contains("season")
        || text.contains("complete")
        || text.contains("全季")
        || text.contains("全集")
}

fn should_flag_unknown_episode(name: &str, media_file_count: usize) -> bool {
    if media_file_count > 1 {
        return true;
    }
    let lower = name.to_ascii_lowercase();
    has_season_pack_keyword(&lower) || lower.contains("episode")
}

fn find_season_episode_marker(text: &str) -> Option<(u32, u32, Option<u32>)> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b's' {
            i += 1;
            continue;
        }
        if let Some((season, mut pos)) = read_ascii_number(bytes, i + 1) {
            pos = skip_episode_separators(bytes, pos);
            if pos < bytes.len() && bytes[pos] == b'e' {
                if let Some((episode, end)) = read_ascii_number(bytes, pos + 1) {
                    if episode > 0 {
                        return Some((season, episode, read_episode_range_end(bytes, end)));
                    }
                }
            }
        }
        i += 1;
    }
    None
}

fn find_bare_episode_marker(text: &str) -> Option<(u32, Option<u32>)> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'e'
            && (i == 0 || !bytes[i - 1].is_ascii_alphabetic())
            && i + 1 < bytes.len()
            && bytes[i + 1].is_ascii_digit()
        {
            if let Some((episode, end)) = read_ascii_number(bytes, i + 1) {
                if episode > 0 {
                    return Some((episode, read_episode_range_end(bytes, end)));
                }
            }
        }
        i += 1;
    }
    None
}

fn find_plain_episode_marker(text: &str) -> Option<(u32, Option<u32>)> {
    let stem = file_stem_text(text);
    let bytes = stem.as_bytes();
    let mut i = 0usize;
    let mut found = None;
    while i < bytes.len() {
        if !bytes[i].is_ascii_digit() || !is_episode_token_left_boundary(bytes, i) {
            i += 1;
            continue;
        }
        let Some((episode, end)) = read_ascii_number(bytes, i) else {
            i += 1;
            continue;
        };
        let digit_count = end - i;
        if !valid_plain_episode_number(episode, digit_count) {
            i = end;
            continue;
        }
        if let Some((last_episode, range_end)) = read_plain_episode_range_end(bytes, end) {
            if last_episode > episode {
                found = unique_plain_episode_match(found, (episode, Some(last_episode)))?;
            }
            i = range_end;
            continue;
        }
        if is_episode_token_right_boundary(bytes, end) {
            found = unique_plain_episode_match(found, (episode, None))?;
        }
        i = end;
    }
    found
}

fn file_stem_text(text: &str) -> &str {
    let leaf = text
        .rsplit(|ch| ch == '/' || ch == '\\')
        .next()
        .unwrap_or(text);
    leaf.rsplit_once('.')
        .map(|(stem, _extension)| stem)
        .unwrap_or(leaf)
}

fn unique_plain_episode_match(
    current: Option<(u32, Option<u32>)>,
    next: (u32, Option<u32>),
) -> Option<Option<(u32, Option<u32>)>> {
    match current {
        None => Some(Some(next)),
        Some(existing) if existing == next => Some(Some(existing)),
        Some(_) => None,
    }
}

fn valid_plain_episode_number(value: u32, digit_count: usize) -> bool {
    value > 0 && value <= 200 && digit_count <= 3
}

fn read_plain_episode_range_end(bytes: &[u8], pos: usize) -> Option<(u32, usize)> {
    if pos >= bytes.len() || !matches!(bytes[pos], b'-' | b'_' | b'~') {
        return None;
    }
    let next = skip_episode_range_separators(bytes, pos);
    let (episode, end) = read_ascii_number(bytes, next)?;
    let digit_count = end - next;
    if valid_plain_episode_number(episode, digit_count)
        && is_episode_token_right_boundary(bytes, end)
    {
        Some((episode, end))
    } else {
        None
    }
}

fn is_episode_token_left_boundary(bytes: &[u8], pos: usize) -> bool {
    pos == 0 || !bytes[pos - 1].is_ascii_alphanumeric()
}

fn is_episode_token_right_boundary(bytes: &[u8], pos: usize) -> bool {
    pos >= bytes.len() || !bytes[pos].is_ascii_alphanumeric()
}

fn read_episode_range_end(bytes: &[u8], pos: usize) -> Option<u32> {
    if pos >= bytes.len() {
        return None;
    }
    let mut next = if bytes[pos] == b'e' {
        pos
    } else if matches!(bytes[pos], b'-' | b'_' | b'~' | b' ') {
        skip_episode_range_separators(bytes, pos)
    } else {
        return None;
    };
    if next < bytes.len() && bytes[next] == b'e' {
        next += 1;
    }
    let (episode, _) = read_ascii_number(bytes, next)?;
    (episode > 0).then_some(episode)
}

fn skip_episode_range_separators(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && matches!(bytes[pos], b'-' | b'_' | b'~' | b' ') {
        pos += 1;
    }
    pos
}

fn find_season_pack_marker(text: &str) -> Option<u32> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b's' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            if let Some((season, pos)) = read_ascii_number(bytes, i + 1) {
                if pos >= bytes.len() || bytes[pos] != b'e' {
                    return Some(season);
                }
            }
        }
        i += 1;
    }
    find_season_word_pack_marker(text)
}

fn find_season_word_pack_marker(text: &str) -> Option<u32> {
    let words = ascii_words(text);
    for (idx, word) in words.iter().enumerate() {
        if *word != "season" {
            continue;
        }
        if idx > 0 {
            if let Ok(season) = words[idx - 1].parse::<u32>() {
                return Some(season);
            }
        }
        for next in words.iter().skip(idx + 1).take(2) {
            if let Ok(season) = next.parse::<u32>() {
                return Some(season);
            }
        }
    }
    None
}

fn ascii_words(text: &str) -> Vec<&str> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect()
}

fn find_chinese_episode_marker(text: &str) -> Option<u32> {
    let chars = text.chars().collect::<Vec<_>>();
    for (idx, ch) in chars.iter().enumerate() {
        if *ch != '第' {
            continue;
        }
        let mut digits = String::new();
        for next in chars.iter().skip(idx + 1) {
            if next.is_ascii_digit() {
                digits.push(*next);
            } else if *next == '集' && !digits.is_empty() {
                return digits.parse().ok();
            } else if !digits.is_empty() {
                break;
            }
        }
    }
    None
}

fn read_ascii_number(bytes: &[u8], start: usize) -> Option<(u32, usize)> {
    let mut pos = start;
    let mut value = 0u32;
    let mut seen = false;
    while pos < bytes.len() && bytes[pos].is_ascii_digit() {
        seen = true;
        value = value
            .saturating_mul(10)
            .saturating_add((bytes[pos] - b'0') as u32);
        pos += 1;
    }
    seen.then_some((value, pos))
}

fn skip_episode_separators(bytes: &[u8], mut pos: usize) -> usize {
    while pos < bytes.len() && matches!(bytes[pos], b'.' | b'_' | b'-' | b' ' | b'[' | b']') {
        pos += 1;
    }
    pos
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
    season_number: Option<u32>,
    episode_number: Option<u32>,
    episode_end_number: Option<u32>,
    episode_label: Option<String>,
}

#[derive(Debug, Clone)]
struct QbTorrentLookup {
    torrent: qbittorrent::QbTorrentInfo,
    #[allow(dead_code)]
    matched_by: String,
}

async fn find_qb_torrent_for_push(
    qb_server: &QbServerEntry,
    push: &subscription::TorrentPushRecord,
    requested_hash: Option<&str>,
) -> Result<QbTorrentLookup, ApiError> {
    let hash_candidates = qb_hash_lookup_candidates(push, requested_hash);
    if hash_candidates.is_empty() {
        return Err(ApiError::bad_request(qb_lookup_error_message(
            qb_server,
            push,
            requested_hash,
            None,
        )));
    }
    let torrents = qbittorrent::list_torrents_by_hashes(qb_server, &hash_candidates).await?;
    if let Some(found) = select_qb_torrent_by_hash(&torrents, &hash_candidates) {
        return Ok(QbTorrentLookup {
            torrent: found,
            matched_by: "qB hashes 精确查询".to_string(),
        });
    }

    Err(ApiError::bad_request(qb_lookup_error_message(
        qb_server,
        push,
        requested_hash,
        Some(torrents.len()),
    )))
}

fn qb_hash_lookup_candidates(
    push: &subscription::TorrentPushRecord,
    requested_hash: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(hash) = requested_hash.map(str::trim).filter(|s| !s.is_empty()) {
        out.push(hash.to_string());
    }
    if let Some(hash) = push
        .qb_hash
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push(hash.to_string());
    }
    out.sort();
    out.dedup();
    if let Some(hash) = requested_hash.map(str::trim).filter(|s| !s.is_empty()) {
        out.sort_by_key(|candidate| {
            if candidate.eq_ignore_ascii_case(hash) {
                0
            } else {
                1
            }
        });
    }
    out
}

fn select_qb_torrent_by_hash(
    torrents: &[qbittorrent::QbTorrentInfo],
    hashes: &[String],
) -> Option<qbittorrent::QbTorrentInfo> {
    hashes
        .iter()
        .find_map(|hash| find_qb_torrent_by_hash(torrents, hash))
}

fn find_qb_torrent_by_hash(
    torrents: &[qbittorrent::QbTorrentInfo],
    hash: &str,
) -> Option<qbittorrent::QbTorrentInfo> {
    torrents
        .iter()
        .find(|torrent| torrent.hash.eq_ignore_ascii_case(hash))
        .cloned()
}

fn qb_tags_for_torrent_id(torrent_id: &str) -> Vec<String> {
    let torrent_id = torrent_id.trim();
    if torrent_id.is_empty() {
        Vec::new()
    } else {
        vec![format!("mteam:{torrent_id}")]
    }
}

fn qb_lookup_error_message(
    qb_server: &QbServerEntry,
    push: &subscription::TorrentPushRecord,
    requested_hash: Option<&str>,
    hash_result_count: Option<usize>,
) -> String {
    let identifiers = qb_lookup_identifiers(push, requested_hash);
    let hash_summary = hash_result_count
        .map(|count| count.to_string())
        .unwrap_or_else(|| "未查询".to_string());
    format!(
        "qB 中未找到已推送种子：server={}，hash候选返回={}，查找标识={}。请确认任务未被删除，或在刷新时提供 qB hash。",
        qb_server_label(qb_server),
        hash_summary,
        if identifiers.is_empty() {
            "无".to_string()
        } else {
            identifiers.join(" / ")
        }
    )
}

fn qb_lookup_identifiers(
    push: &subscription::TorrentPushRecord,
    requested_hash: Option<&str>,
) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(hash) = requested_hash.map(str::trim).filter(|s| !s.is_empty()) {
        out.push(format!("requested_hash={hash}"));
    }
    if let Some(hash) = push
        .qb_hash
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.push(format!("stored_hash={hash}"));
    }
    out
}

fn qb_server_label(server: &QbServerEntry) -> String {
    if !server.name.trim().is_empty() {
        server.name.trim().to_string()
    } else if !server.base_url.trim().is_empty() {
        server.base_url.trim().to_string()
    } else {
        "未命名 qB".to_string()
    }
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
    let title = hardlink_title_component(record);
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
        .filter_map(|file| safe_relative_path(&file.name).map(|relative| (file, relative)))
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(ApiError::bad_request("qB 文件列表中没有可硬链接的文件"));
    }

    let mut plan_files = Vec::new();
    let selected_count = selected.len();
    for (file, relative) in selected {
        let source_path = source_path_for_qb_file(&source_root, push, torrent, &relative);
        let target_path = target_dir.join(relative);
        let episode = episode_marker_for_file_name(&file.name, selected_count);
        plan_files.push(HardlinkFilePlan {
            source_path,
            target_path,
            size: file.size,
            season_number: episode.season_number,
            episode_number: episode.episode_number,
            episode_end_number: episode.episode_end_number,
            episode_label: episode.label,
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
                season_number: file.season_number,
                episode_number: file.episode_number,
                episode_end_number: file.episode_end_number,
                episode_label: file.episode_label.clone(),
                error: None,
            })
            .collect(),
        episodes: episode_records_from_hardlink_files(
            &plan
                .files
                .iter()
                .map(|file| subscription::HardlinkFileRecord {
                    source_path: file.source_path.display().to_string(),
                    target_path: file.target_path.display().to_string(),
                    size: file.size,
                    status: "planned".to_string(),
                    season_number: file.season_number,
                    episode_number: file.episode_number,
                    episode_end_number: file.episode_end_number,
                    episode_label: file.episode_label.clone(),
                    error: None,
                })
                .collect::<Vec<_>>(),
        ),
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
        episodes: episode_records_from_hardlink_files(&records),
        linked_files: records,
        error: (!errors.is_empty()).then(|| errors.join("; ")),
    }
}

fn hardlink_one_file(file: &HardlinkFilePlan) -> subscription::HardlinkFileRecord {
    let source_display = file.source_path.display().to_string();
    if !file.source_path.exists() {
        return subscription::HardlinkFileRecord {
            source_path: source_display,
            target_path: file.target_path.display().to_string(),
            size: file.size,
            status: "failed".to_string(),
            season_number: file.season_number,
            episode_number: file.episode_number,
            episode_end_number: file.episode_end_number,
            episode_label: file.episode_label.clone(),
            error: Some("源文件不存在".to_string()),
        };
    }
    let (target_path, already_linked) =
        match resolve_hardlink_target_path(&file.source_path, &file.target_path) {
            Ok(resolved) => resolved,
            Err(error) => {
                return subscription::HardlinkFileRecord {
                    source_path: source_display,
                    target_path: file.target_path.display().to_string(),
                    size: file.size,
                    status: "failed".to_string(),
                    season_number: file.season_number,
                    episode_number: file.episode_number,
                    episode_end_number: file.episode_end_number,
                    episode_label: file.episode_label.clone(),
                    error: Some(error),
                };
            }
        };
    let target_display = target_path.display().to_string();
    if already_linked {
        return subscription::HardlinkFileRecord {
            source_path: source_display,
            target_path: target_display,
            size: file.size,
            status: "already_linked".to_string(),
            season_number: file.season_number,
            episode_number: file.episode_number,
            episode_end_number: file.episode_end_number,
            episode_label: file.episode_label.clone(),
            error: None,
        };
    }
    if let Some(parent) = target_path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return subscription::HardlinkFileRecord {
                source_path: source_display,
                target_path: target_display,
                size: file.size,
                status: "failed".to_string(),
                season_number: file.season_number,
                episode_number: file.episode_number,
                episode_end_number: file.episode_end_number,
                episode_label: file.episode_label.clone(),
                error: Some(format!("创建目标目录失败: {e}")),
            };
        }
    }
    match std::fs::hard_link(&file.source_path, &target_path) {
        Ok(()) => subscription::HardlinkFileRecord {
            source_path: source_display,
            target_path: target_display,
            size: file.size,
            status: "linked".to_string(),
            season_number: file.season_number,
            episode_number: file.episode_number,
            episode_end_number: file.episode_end_number,
            episode_label: file.episode_label.clone(),
            error: None,
        },
        Err(e) => subscription::HardlinkFileRecord {
            source_path: source_display,
            target_path: target_display,
            size: file.size,
            status: "failed".to_string(),
            season_number: file.season_number,
            episode_number: file.episode_number,
            episode_end_number: file.episode_end_number,
            episode_label: file.episode_label.clone(),
            error: Some(hardlink_error_message(&e)),
        },
    }
}

fn resolve_hardlink_target_path(source: &Path, target: &Path) -> Result<(PathBuf, bool), String> {
    if !target.exists() {
        return Ok((target.to_path_buf(), false));
    }
    if same_file(source, target).unwrap_or(false) {
        return Ok((target.to_path_buf(), true));
    }
    for suffix in 1..=999 {
        let candidate = conflict_renamed_path(target, suffix);
        if !candidate.exists() {
            return Ok((candidate, false));
        }
        if same_file(source, &candidate).unwrap_or(false) {
            return Ok((candidate, true));
        }
    }
    Err("目标文件已存在且自动改名超过 999 次".to_string())
}

fn conflict_renamed_path(path: &Path, suffix: u32) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|value| value.to_string_lossy())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| path.file_name().unwrap_or_default().to_string_lossy());
    let file_name = match path.extension().map(|value| value.to_string_lossy()) {
        Some(ext) if !ext.is_empty() => format!("{stem}.{suffix}.{ext}"),
        _ => format!("{stem}.{suffix}"),
    };
    path.with_file_name(file_name)
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

fn hardlink_title_component(record: &subscription::WantedSubscriptionRecord) -> String {
    let title = record.title.trim();
    let primary_chinese = title
        .split(['/', '／'])
        .map(str::trim)
        .find(|part| !part.is_empty() && contains_cjk(part));
    sanitize_output_component(primary_chinese.unwrap_or(title))
}

fn contains_cjk(value: &str) -> bool {
    value.chars().any(|ch| {
        matches!(
            ch,
            '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}' | '\u{f900}'..='\u{faff}'
        )
    })
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
    torrent_push_record_with_download_url(
        subscription_id,
        qb_server,
        category,
        candidate,
        status,
        pushed_at,
        error,
        None,
    )
}

fn torrent_push_record_with_download_url(
    subscription_id: &str,
    qb_server: &QbServerEntry,
    category: &SubscriptionCategory,
    candidate: &subscription::TorrentCandidateRecord,
    status: &str,
    pushed_at: Option<u64>,
    error: Option<String>,
    torrent_download_url: Option<&str>,
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
        qb_server_id: qb_server.id.trim().to_string(),
        qb_category: category.qb_category.trim().to_string(),
        qb_save_dir_name: category.qb_save_dir_name.trim().to_string(),
        qb_identifier: if candidate.torrent_id.trim().is_empty() {
            String::new()
        } else {
            format!("mteam:{}", candidate.torrent_id.trim())
        },
        torrent_download_url: torrent_download_url
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_string),
        mteam_torrent_url: mteam_torrent_web_url(&candidate.torrent_id),
        pushed_at,
        status: status.to_string(),
        error,
        qb_hash: None,
        qb_name: None,
        checked_at: None,
        completed_at: None,
        download_progress: None,
        download_state: None,
        total_size: None,
        completed_file_count: None,
        total_file_count: None,
        files: Vec::new(),
        episodes: Vec::new(),
        source_path: None,
        target_dir: None,
        linked_files: Vec::new(),
    }
}

fn mteam_torrent_web_url(torrent_id: &str) -> Option<String> {
    let id = torrent_id.trim();
    (!id.is_empty()).then(|| format!("https://kp.m-team.cc/detail/{id}"))
}

fn inherit_existing_qb_lookup(
    next: &mut subscription::TorrentPushRecord,
    existing: Option<&subscription::TorrentPushRecord>,
) {
    let Some(existing) = existing else {
        return;
    };
    if existing.torrent_id.trim() != next.torrent_id.trim()
        || existing.qb_server_id.trim() != next.qb_server_id.trim()
        || existing.qb_category.trim() != next.qb_category.trim()
    {
        return;
    }
    if next.qb_hash.as_deref().unwrap_or("").trim().is_empty() {
        next.qb_hash = existing.qb_hash.clone();
    }
    if next.qb_name.as_deref().unwrap_or("").trim().is_empty() {
        next.qb_name = existing.qb_name.clone();
    }
}

fn apply_existing_qb_lookup_to_push(
    push: &mut subscription::TorrentPushRecord,
    torrent: &qbittorrent::QbTorrentInfo,
) {
    push.qb_hash = Some(torrent.hash.clone());
    push.qb_name = Some(torrent.name.clone());
}

const WANTED_PIPELINE_TICK_SECS: u64 = 1;

fn wanted_watch_interval() -> tokio::time::Interval {
    let mut interval = tokio::time::interval(Duration::from_secs(WANTED_PIPELINE_TICK_SECS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WantedWatchTickAction {
    PollWanted,
    ProcessPipeline,
}

fn wanted_watch_tick_action(
    has_account_key: bool,
    now_secs: u64,
    last_wanted_poll_at: Option<u64>,
    poll_interval_secs: u64,
) -> Option<WantedWatchTickAction> {
    if !has_account_key {
        return None;
    }
    let poll_interval_secs = poll_interval_secs.clamp(60, 86_400);
    let should_poll = last_wanted_poll_at
        .map(|last| now_secs.saturating_sub(last) >= poll_interval_secs)
        .unwrap_or(true);
    Some(if should_poll {
        WantedWatchTickAction::PollWanted
    } else {
        WantedWatchTickAction::ProcessPipeline
    })
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
    let existing_state = state
        .wanted_store
        .snapshot(&account_key, unix_now_secs())
        .await
        .ok();
    let details =
        fetch_wanted_subject_detail_cache(&cfg.douban_cookie, &wish.items, existing_state.as_ref())
            .await;
    let outcome = state
        .wanted_store
        .apply_wish_items_with_details(
            &account_key,
            &wish.items,
            &details,
            &cfg.subscription_watcher,
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("写入想看订阅状态失败: {e}")))?;
    if let Err(err) = process_wanted_watch_queue(state, &account_key).await {
        tracing::warn!(
            account_key = %account_key,
            "wanted subscription processing failed after poll: {}",
            err.message()
        );
    }
    Ok(outcome)
}

async fn fetch_wanted_subject_detail_cache(
    douban_cookie: &str,
    items: &[douban::DoubanLibraryItem],
    existing_state: Option<&subscription::WantedSubscriptionState>,
) -> BTreeMap<String, douban::DoubanSubjectDetail> {
    let mut out = BTreeMap::new();
    for item in items {
        let subject_id = item.subject_id.trim();
        if subject_id.is_empty() {
            continue;
        }
        if !wanted_item_needs_rexxar_detail_cache(existing_state, item) {
            continue;
        }
        match douban::subject_detail_rexxar(douban_cookie, subject_id).await {
            Ok(detail) => {
                out.insert(subject_id.to_string(), detail);
            }
            Err(err) => {
                let message = err.to_string();
                if douban_rexxar_detail_cache_should_back_off(&message) {
                    tracing::info!(
                        subject_id = %subject_id,
                        "douban rexxar detail cache paused for this wanted poll: {}",
                        message
                    );
                    break;
                } else {
                    tracing::debug!(
                        subject_id = %subject_id,
                        "douban rexxar detail cache fetch skipped during wanted poll: {}",
                        message
                    );
                }
            }
        }
    }
    out
}

fn wanted_item_needs_rexxar_detail_cache(
    existing_state: Option<&subscription::WantedSubscriptionState>,
    item: &douban::DoubanLibraryItem,
) -> bool {
    let subject_id = item.subject_id.trim();
    if subject_id.is_empty() {
        return false;
    }
    let Some(record) = existing_state.and_then(|state| state.records.get(subject_id)) else {
        return true;
    };
    record.date_published.is_none()
        || record.summary.is_none()
        || record.genres.is_empty()
        || record.directors.is_empty()
        || record.actors.is_empty()
}

fn douban_rexxar_detail_cache_should_back_off(message: &str) -> bool {
    message.contains("网络存在异常") || message.contains("登录后重试")
}

async fn run_wanted_pipeline_tick(state: &AppState) -> Result<String, ApiError> {
    let cfg = state.config.read().await.clone();
    let account_key =
        douban::auth_cache_key_fragment(&cfg.douban_cookie).map_err(ApiError::douban)?;
    process_wanted_watch_queue(state, &account_key).await?;
    Ok(account_key)
}

async fn process_wanted_watch_queue(state: &AppState, account_key: &str) -> Result<(), ApiError> {
    let now = unix_now_secs();
    let snapshot = state
        .wanted_store
        .snapshot(account_key, now)
        .await
        .map_err(|e| ApiError::internal(format!("读取想看订阅处理队列失败: {e}")))?;
    let records = snapshot.records.values().cloned().collect::<Vec<_>>();
    for record in records {
        let Some(operation) = select_watcher_due_operation(&record, now) else {
            continue;
        };
        if let Err(err) =
            execute_due_subscription_operation(state, account_key, record.clone(), operation).await
        {
            tracing::warn!(
                subject_id = %record.subject_id,
                lifecycle_state = %record.lifecycle_state.as_str(),
                operation = ?operation,
                "wanted subscription due operation failed: {}",
                err.message()
            );
        }
    }
    Ok(())
}

fn select_watcher_due_operation(
    record: &subscription::WantedSubscriptionRecord,
    now: u64,
) -> Option<subscription::SubscriptionDueOperation> {
    subscription::select_due_operation(record, now)
}

async fn execute_due_subscription_operation(
    state: &AppState,
    account_key: &str,
    record: subscription::WantedSubscriptionRecord,
    operation: subscription::SubscriptionDueOperation,
) -> Result<(), ApiError> {
    match operation {
        subscription::SubscriptionDueOperation::MovieMeta => {
            process_movie_meta_operation(state, account_key, &record).await
        }
        subscription::SubscriptionDueOperation::MovieSearch => {
            process_wanted_push_step(state, account_key, &record.subject_id, true).await
        }
        subscription::SubscriptionDueOperation::MovieProgress => {
            process_wanted_progress_step(state, account_key, &record.subject_id).await
        }
        subscription::SubscriptionDueOperation::MovieLink => {
            process_wanted_completion_step(state, account_key, &record.subject_id).await
        }
        subscription::SubscriptionDueOperation::TvMeta => {
            process_tv_meta_operation(state, account_key, &record).await
        }
        subscription::SubscriptionDueOperation::TvLane(lane) => {
            process_tv_lane_operation(state, account_key, &record, lane).await
        }
    }
}

async fn process_movie_meta_operation(
    state: &AppState,
    account_key: &str,
    record: &subscription::WantedSubscriptionRecord,
) -> Result<(), ApiError> {
    let cfg = state.config.read().await.clone();
    state
        .wanted_store
        .transition_movie_meta_operation(
            account_key,
            &record.subject_id,
            &cfg.subscription_watcher,
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("更新电影订阅元数据阶段失败: {e}")))?
        .ok_or_else(|| ApiError::bad_request("订阅记录不存在"))?;
    Ok(())
}

async fn process_tv_meta_operation(
    _state: &AppState,
    _account_key: &str,
    _record: &subscription::WantedSubscriptionRecord,
) -> Result<(), ApiError> {
    Err(ApiError::bad_request(
        "TV meta operation is not implemented yet",
    ))
}

async fn process_tv_lane_operation(
    _state: &AppState,
    _account_key: &str,
    _record: &subscription::WantedSubscriptionRecord,
    lane: subscription::TvLaneKind,
) -> Result<(), ApiError> {
    Err(ApiError::bad_request(format!(
        "TV lane operation is not implemented yet: {lane:?}"
    )))
}

async fn process_wanted_push_step(
    state: &AppState,
    account_key: &str,
    subject_id: &str,
    force: bool,
) -> Result<(), ApiError> {
    let original_record = state
        .wanted_store
        .snapshot(account_key, unix_now_secs())
        .await
        .ok()
        .and_then(|snapshot| snapshot.records.get(subject_id).cloned());
    match wanted_subscription_push(
        State(state.clone()),
        PathParam(subject_id.to_string()),
        Json(WantedPushBody {
            force,
            ..WantedPushBody::default()
        }),
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(err) => {
            if should_persist_search_step_failure_fallback(
                state,
                account_key,
                subject_id,
                original_record.as_ref(),
            )
            .await
            {
                let cfg = state.config.read().await.clone();
                let now = unix_now_secs();
                if let Err(persist_err) = state
                    .wanted_store
                    .apply_parent_operation_failure_result(
                        account_key,
                        subject_id,
                        "search",
                        err.message(),
                        &cfg.subscription_watcher,
                        now,
                    )
                    .await
                {
                    tracing::warn!(
                        subject_id = %subject_id,
                        error = %persist_err,
                        "persist search step failure failed"
                    );
                }
            }
            Err(err)
        }
    }
}

async fn should_persist_search_step_failure_fallback(
    state: &AppState,
    account_key: &str,
    subject_id: &str,
    original: Option<&subscription::WantedSubscriptionRecord>,
) -> bool {
    let Ok(snapshot) = state
        .wanted_store
        .snapshot(account_key, unix_now_secs())
        .await
    else {
        return true;
    };
    let Some(record) = snapshot.records.get(subject_id) else {
        return false;
    };
    let Some(original) = original else {
        return record.failure.is_none();
    };
    !search_step_failure_was_persisted_during_attempt(original, record)
}

fn search_step_failure_was_persisted_during_attempt(
    original: &subscription::WantedSubscriptionRecord,
    current: &subscription::WantedSubscriptionRecord,
) -> bool {
    if current.retry_count > original.retry_count {
        return true;
    }
    if current.updated_at <= original.updated_at {
        return false;
    }
    failed_push_marker_changed(original.last_push.as_ref(), current.last_push.as_ref())
}

fn failed_push_marker_changed(
    original: Option<&subscription::TorrentPushRecord>,
    current: Option<&subscription::TorrentPushRecord>,
) -> bool {
    let Some(current) = current else {
        return false;
    };
    if current.status != "failed" {
        return false;
    }
    let Some(original) = original else {
        return true;
    };
    original.status != current.status
        || original.error != current.error
        || original.pushed_at != current.pushed_at
        || original.checked_at != current.checked_at
        || original.torrent_id != current.torrent_id
}

async fn process_wanted_progress_step(
    state: &AppState,
    account_key: &str,
    subject_id: &str,
) -> Result<(), ApiError> {
    match wanted_subscription_progress(
        State(state.clone()),
        PathParam(subject_id.to_string()),
        Json(WantedProgressBody::default()),
    )
    .await
    {
        Ok(Json(value)) => {
            if value
                .get("completed")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                process_wanted_completion_step(state, account_key, subject_id).await?;
            }
            Ok(())
        }
        Err(err) => Err(err),
    }
}

async fn process_wanted_completion_step(
    state: &AppState,
    _account_key: &str,
    subject_id: &str,
) -> Result<(), ApiError> {
    match wanted_subscription_completion(
        State(state.clone()),
        PathParam(subject_id.to_string()),
        Json(WantedCompletionBody::default()),
    )
    .await
    {
        Ok(_) => Ok(()),
        Err(err) => Err(err),
    }
}

fn spawn_wanted_watch_loop(state: AppState) {
    tokio::spawn(async move {
        let mut last_wanted_poll_at = None;
        let mut tick = wanted_watch_interval();
        loop {
            tick.tick().await;
            let cfg = state.config.read().await.clone();
            let now_secs = unix_now_secs();
            let action = wanted_watch_tick_action(
                douban::auth_cache_key_fragment(&cfg.douban_cookie).is_ok(),
                now_secs,
                last_wanted_poll_at,
                cfg.subscription_watcher.poll_interval_secs,
            );
            match action {
                Some(WantedWatchTickAction::PollWanted) => {
                    last_wanted_poll_at = Some(now_secs);
                    match run_wanted_watch_poll(&state).await {
                        Ok(outcome) => {
                            tracing::info!(
                                account_key = %outcome.account_key,
                                total = outcome.total_wish_items,
                                created_unprocessed = outcome.created_unprocessed,
                                created_skipped = outcome.created_skipped,
                                updated_existing = outcome.updated_existing,
                                "wanted subscription poll completed"
                            );
                            write_operation_log(
                                &state,
                                operation_log_entry(
                                    outcome.account_key.clone(),
                                    "subscription_sync",
                                    "poll_wanted",
                                    "subscription_state",
                                    None,
                                    None,
                                    "success",
                                    format!(
                                        "后台轮询想看完成：新增待处理 {}，跳过旧想看 {}，更新已有 {}",
                                        outcome.created_unprocessed,
                                        outcome.created_skipped,
                                        outcome.updated_existing
                                    ),
                                    None,
                                    json!({
                                        "trigger": "watcher",
                                        "total_wish_items": outcome.total_wish_items,
                                        "created_unprocessed": outcome.created_unprocessed,
                                        "created_skipped": outcome.created_skipped,
                                        "updated_existing": outcome.updated_existing,
                                        "bootstrap_mode": outcome.bootstrap_mode,
                                    }),
                                ),
                            )
                            .await;
                        }
                        Err(e) => {
                            tracing::warn!("wanted subscription poll failed: {}", e.message());
                            write_operation_log(
                                &state,
                                operation_log_entry(
                                    "system",
                                    "subscription_sync",
                                    "poll_wanted",
                                    "subscription_state",
                                    None,
                                    None,
                                    "failed",
                                    "后台轮询想看失败",
                                    Some(e.message().to_string()),
                                    json!({ "trigger": "watcher" }),
                                ),
                            )
                            .await;
                        }
                    }
                }
                Some(WantedWatchTickAction::ProcessPipeline) => {
                    if let Err(err) = run_wanted_pipeline_tick(&state).await {
                        tracing::warn!(
                            "wanted subscription pipeline tick failed: {}",
                            err.message()
                        );
                    }
                }
                None => {}
            }
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
    let cfg = state.config.read().await.clone();
    let mteam_key = cfg.mteam_api_key.clone();
    if mteam_key.trim().is_empty() {
        write_operation_log(
            &state,
            operation_log_entry(
                config_account_key(&cfg),
                "qb_push",
                "manual_push_torrent",
                "torrent",
                Some(body.torrent_id.clone()),
                None,
                "failed",
                "手动推送种子到 qB 失败：缺少 M-Team API Key",
                Some("请先在设置中填写 M-Team OpenAPI Key".to_string()),
                json!({ "torrent_id": body.torrent_id }),
            ),
        )
        .await;
        return Err(ApiError::bad_request(
            "请先在设置中填写 M-Team OpenAPI Key（用于向 qB 换取可下载链接）",
        ));
    }
    let dl_url = match mteam_fetch_gen_dl_url(mteam_key.trim(), &body.torrent_id).await {
        Ok(url) => url,
        Err(err) => {
            write_operation_log(
                &state,
                operation_log_entry(
                    config_account_key(&cfg),
                    "qb_push",
                    "manual_push_torrent",
                    "torrent",
                    Some(body.torrent_id.clone()),
                    None,
                    "failed",
                    "手动推送种子到 qB 失败：M-Team 取链失败",
                    Some(err.message().to_string()),
                    json!({ "torrent_id": body.torrent_id }),
                ),
            )
            .await;
            return Err(err);
        }
    };
    if let Err(err) = qbittorrent::add_torrent_from_url(
        &body.server,
        &dl_url,
        body.category.as_deref(),
        body.savepath.as_deref(),
    )
    .await
    {
        write_operation_log(
            &state,
            operation_log_entry(
                config_account_key(&cfg),
                "qb_push",
                "manual_push_torrent",
                "torrent",
                Some(body.torrent_id.clone()),
                None,
                "failed",
                "手动推送种子到 qB 失败：qB 添加种子失败",
                Some(err.message().to_string()),
                json!({
                    "torrent_id": body.torrent_id,
                    "qb_server": body.server.name,
                    "qb_category": body.category,
                    "savepath": body.savepath,
                }),
            ),
        )
        .await;
        return Err(err);
    }
    write_operation_log(
        &state,
        operation_log_entry(
            config_account_key(&cfg),
            "qb_push",
            "manual_push_torrent",
            "torrent",
            Some(body.torrent_id.clone()),
            None,
            "success",
            "已手动推送种子到 qB",
            None,
            json!({
                "torrent_id": body.torrent_id,
                "qb_server": body.server.name,
                "qb_category": body.category,
                "savepath": body.savepath,
            }),
        ),
    )
    .await;
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
    let cfg = state.config.read().await.clone();
    let key = cfg.tmdb_api_key.clone();
    if key.trim().is_empty() {
        write_operation_log(
            &state,
            operation_log_entry(
                config_account_key(&cfg),
                "search",
                "search_media",
                "tmdb",
                None,
                Some(q.q.trim().to_string()),
                "failed",
                "TMDB 搜索失败：缺少 API Key",
                Some("请在设置中填写 TMDB API Key".to_string()),
                json!({ "source": "tmdb" }),
            ),
        )
        .await;
        return Err(ApiError::bad_request("请在设置中填写 TMDB API Key"));
    }
    if q.q.trim().is_empty() {
        write_operation_log(
            &state,
            operation_log_entry(
                config_account_key(&cfg),
                "search",
                "search_media",
                "tmdb",
                None,
                None,
                "failed",
                "TMDB 搜索失败：关键词为空",
                Some("搜索关键字不能为空".to_string()),
                json!({ "source": "tmdb" }),
            ),
        )
        .await;
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

    write_operation_log(
        &state,
        operation_log_entry(
            config_account_key(&cfg),
            "search",
            "search_media",
            "tmdb",
            None,
            Some(q.q.trim().to_string()),
            "success",
            format!(
                "TMDB 搜索完成：电影 {}，剧集 {}",
                movie_items.len(),
                tv_items.len()
            ),
            None,
            json!({
                "source": "tmdb",
                "movie_count": movie_items.len(),
                "tv_count": tv_items.len(),
            }),
        ),
    )
    .await;
    Ok(Json(json!({ "movies": movie_items, "tv": tv_items })))
}

#[derive(Deserialize)]
struct DoubanSearchQuery {
    q: String,
    #[serde(default = "default_page_usize")]
    page: usize,
    #[serde(default = "default_douban_page_size")]
    page_size: usize,
}

fn default_page_usize() -> usize {
    1
}

fn default_douban_page_size() -> usize {
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
    let cfg = state.config.read().await.clone();
    let cookie = cfg.douban_cookie.clone();
    let page = q.page.max(1);
    let page_size = q.page_size.clamp(1, 20);
    let result = match douban::search(&cookie, &q.q, page, page_size).await {
        Ok(result) => result,
        Err(err) => {
            write_operation_log(
                &state,
                operation_log_entry(
                    config_account_key(&cfg),
                    "search",
                    "search_media",
                    "douban",
                    None,
                    Some(q.q.trim().to_string()),
                    "failed",
                    "豆瓣搜索失败",
                    Some(err.to_string()),
                    json!({ "source": "douban", "page": page, "page_size": page_size }),
                ),
            )
            .await;
            return Err(ApiError::douban(err));
        }
    };
    let items_value =
        serde_json::to_value(&result.items).map_err(|e| ApiError::internal(e.to_string()))?;
    write_operation_log(
        &state,
        operation_log_entry(
            config_account_key(&cfg),
            "search",
            "search_media",
            "douban",
            None,
            Some(q.q.trim().to_string()),
            "success",
            format!("豆瓣搜索完成：{} 条结果", result.items.len()),
            None,
            json!({
                "source": "douban",
                "result_count": result.items.len(),
                "page": result.page,
                "page_size": result.page_size,
                "has_more": result.has_more,
            }),
        ),
    )
    .await;
    Ok(Json(json!({
        "items": items_value.clone(),
        "movies": items_value,
        "tv": [],
        "page": result.page,
        "page_size": result.page_size,
        "has_more": result.has_more,
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

fn operation_log_entry(
    account_key: impl Into<String>,
    category: &str,
    action: &str,
    target_type: &str,
    target_id: Option<String>,
    target_title: Option<String>,
    status: &str,
    summary: impl Into<String>,
    error: Option<String>,
    related: Value,
) -> subscription::NewOperationLogEntry {
    subscription::NewOperationLogEntry {
        account_key: account_key.into(),
        created_at: unix_now_secs(),
        category: category.to_string(),
        action: action.to_string(),
        target_type: target_type.to_string(),
        target_id,
        target_title,
        status: status.to_string(),
        summary: summary.into(),
        error: error.filter(|s| !s.trim().is_empty()),
        related,
    }
}

async fn write_operation_log(state: &AppState, entry: subscription::NewOperationLogEntry) {
    if let Err(e) = state.wanted_store.append_operation_log(entry).await {
        tracing::warn!("operation log write failed: {e}");
    }
}

fn config_account_key(cfg: &FileConfig) -> String {
    douban::auth_cache_key_fragment(&cfg.douban_cookie).unwrap_or_else(|_| "system".to_string())
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
    let result = match douban::mark_interest(&cookie, &id, body.interest, body.rating, &tags).await
    {
        Ok(result) => result,
        Err(err) => {
            write_operation_log(
                &state,
                operation_log_entry(
                    account_key.clone(),
                    "subscription_sync",
                    "mark_interest",
                    "douban_subject",
                    Some(id.clone()),
                    None,
                    "failed",
                    "豆瓣标记失败",
                    Some(err.to_string()),
                    json!({
                        "interest": format!("{:?}", body.interest),
                        "has_rating": body.rating.is_some(),
                    }),
                ),
            )
            .await;
            return Err(ApiError::douban(err));
        }
    };
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
    write_operation_log(
        &state,
        operation_log_entry(
            account_key.clone(),
            "subscription_sync",
            "mark_interest",
            "douban_subject",
            Some(id.clone()),
            None,
            "success",
            if matches!(body.interest, douban::DoubanInterest::Wish) {
                "已标记豆瓣想看"
            } else {
                "已标记豆瓣看过"
            },
            None,
            json!({
                "interest": format!("{:?}", body.interest),
                "tag_count": result.tags.split_whitespace().count(),
            }),
        ),
    )
    .await;
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
    let cfg = state.config.read().await.clone();
    let key = cfg.mteam_api_key.clone();
    let source_label = match &q.source {
        MteamSource::Imdb => "imdb",
        MteamSource::Douban => "douban",
        MteamSource::Keyword => "keyword",
    };
    if key.trim().is_empty() {
        write_operation_log(
            &state,
            operation_log_entry(
                config_account_key(&cfg),
                "torrent_search",
                "search_torrents",
                "mteam",
                None,
                None,
                "failed",
                "M-Team 种子搜索失败：缺少 API Key",
                Some("请在设置中填写 M-Team API Key".to_string()),
                json!({ "source": source_label }),
            ),
        )
        .await;
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

    let query_label = match source_label {
        "imdb" => imdb_raw.unwrap_or("").to_string(),
        "douban" => douban_raw.unwrap_or("").to_string(),
        _ => keyword_raw.unwrap_or("").to_string(),
    };
    let out_result: Result<Value, ApiError> = match q.source {
        MteamSource::Imdb => {
            if let Some(s) = imdb_raw {
                let imdb = normalize_imdb_url(s);
                let body = mteam_search_body(q.page, q.page_size, "imdb", &imdb);
                mteam_search_post(&client, &key, &body).await
            } else {
                Err(ApiError::bad_request(
                    "使用 IMDb 路径时请提供有效的 imdb_id",
                ))
            }
        }
        MteamSource::Douban => {
            if let Some(s) = douban_raw {
                let douban = normalize_douban_url(s);
                let body = mteam_search_body(q.page, q.page_size, "douban", &douban);
                mteam_search_post(&client, &key, &body).await
            } else {
                Err(ApiError::bad_request(
                    "使用豆瓣路径时请提供有效的 douban_id",
                ))
            }
        }
        MteamSource::Keyword => {
            if let Some(k) = keyword_raw {
                let body = mteam_search_body(q.page, q.page_size, "keyword", k);
                mteam_search_post(&client, &key, &body).await
            } else {
                Err(ApiError::bad_request("使用关键字路径时请提供 keyword"))
            }
        }
    };

    let out = match out_result {
        Ok(out) => out,
        Err(err) => {
            write_operation_log(
                &state,
                operation_log_entry(
                    config_account_key(&cfg),
                    "torrent_search",
                    "search_torrents",
                    "mteam",
                    None,
                    (!query_label.is_empty()).then_some(query_label.clone()),
                    "failed",
                    "M-Team 种子搜索失败",
                    Some(err.message().to_string()),
                    json!({ "source": source_label, "page": q.page, "page_size": q.page_size }),
                ),
            )
            .await;
            return Err(err);
        }
    };

    let mut result_values = Vec::new();
    collect_mteam_candidate_objects(&out, &mut result_values);
    write_operation_log(
        &state,
        operation_log_entry(
            config_account_key(&cfg),
            "torrent_search",
            "search_torrents",
            "mteam",
            None,
            (!query_label.is_empty()).then_some(query_label),
            "success",
            format!("M-Team 种子搜索完成：{} 条候选", result_values.len()),
            None,
            json!({
                "source": source_label,
                "candidate_count": result_values.len(),
                "page": q.page,
                "page_size": q.page_size,
            }),
        ),
    )
    .await;

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

    #[test]
    fn wanted_status_route_is_removed_from_router_source() {
        let source = include_str!("main.rs");
        assert!(!source.contains("\"/subscriptions/wanted/{id}/status\""));
        assert!(!source.contains(concat!("wanted_subscription_", "status(")));
    }

    fn category(name: &str, wanted_tag: &str) -> SubscriptionCategory {
        SubscriptionCategory {
            name: name.to_string(),
            wanted_tag: wanted_tag.to_string(),
            qb_server_id: String::new(),
            qb_category: format!("qb-{name}"),
            qb_save_dir_name: format!("save-{name}"),
            download_dir: format!("/downloads/{name}"),
            link_target_dir: format!("/media/{name}"),
        }
    }

    fn qb_server(id: &str, name: &str, base_url: &str) -> QbServerEntry {
        QbServerEntry {
            id: id.to_string(),
            name: name.to_string(),
            base_url: base_url.to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        }
    }

    #[test]
    fn qb_servers_get_stable_unique_ids() {
        let servers = match normalize_qb_servers(vec![
            qb_server("", "NAS", "http://127.0.0.1:8080"),
            qb_server("", "NAS", "http://127.0.0.1:8081"),
            qb_server("custom-id", "下载机", "http://127.0.0.1:8082"),
        ]) {
            Ok(servers) => servers,
            Err(err) => panic!("qB servers should normalize: {}", err.message()),
        };

        assert_eq!(servers[0].id, "nas");
        assert_eq!(servers[1].id, "nas-2");
        assert_eq!(servers[2].id, "custom-id");
    }

    #[test]
    fn wanted_detail_cache_skips_records_that_already_have_rexxar_fields() {
        let mut state = subscription::WantedSubscriptionState {
            version: 1,
            account_key: "acct".to_string(),
            bootstrap_completed: true,
            created_at: 100,
            updated_at: 100,
            last_poll_at: Some(100),
            records: BTreeMap::new(),
        };
        let mut cached = wanted_record("1", "已有缓存", Some(2024));
        cached.date_published = Some("2024-01-01".to_string());
        cached.summary = Some("已有简介".to_string());
        cached.genres = vec!["剧情".to_string()];
        cached.directors = vec!["导演".to_string()];
        cached.actors = vec!["主演".to_string()];
        state.records.insert("1".to_string(), cached);

        assert!(!wanted_item_needs_rexxar_detail_cache(
            Some(&state),
            &library_item("1", "已有缓存"),
        ));
        assert!(wanted_item_needs_rexxar_detail_cache(
            Some(&state),
            &library_item("2", "新条目"),
        ));
    }

    #[test]
    fn douban_rexxar_detail_cache_backoff_recognizes_anti_abuse_response() {
        assert!(douban_rexxar_detail_cache_should_back_off(
            "豆瓣 rexxar 接口 HTTP 400 Bad Request: 您所在的网络存在异常，请登录后重试。"
        ));
        assert!(!douban_rexxar_detail_cache_should_back_off(
            "豆瓣 rexxar 请求失败: dns error"
        ));
    }

    #[test]
    fn subscription_categories_bind_to_qb_server_ids() {
        let servers = vec![qb_server("nas", "NAS", "http://127.0.0.1:8080")];
        let mut movie = category("电影", "电影");
        let normalized = match normalize_subscription_categories(vec![movie.clone()], &servers) {
            Ok(categories) => categories,
            Err(err) => panic!("category should bind to first qB server: {}", err.message()),
        };
        assert_eq!(normalized[0].qb_server_id, "nas");

        movie.qb_server_id = "missing".to_string();
        assert!(matches!(
            normalize_subscription_categories(vec![movie], &servers),
            Err(ApiError::BadRequest { .. })
        ));
    }

    #[test]
    fn category_qb_server_id_selects_push_target() {
        let servers = vec![
            qb_server("nas", "NAS", "http://127.0.0.1:8080"),
            qb_server("ssd", "SSD", "http://127.0.0.1:8081"),
        ];
        let mut movie = category("电影", "电影");
        movie.qb_server_id = "ssd".to_string();

        let selected = match select_qb_server_for_category(&servers, &movie) {
            Ok(server) => server,
            Err(err) => panic!(
                "category should select configured qB server: {}",
                err.message()
            ),
        };
        assert_eq!(selected.id, "ssd");
        assert_eq!(selected.name, "SSD");
    }

    #[test]
    fn subscription_categories_reject_duplicate_wanted_tags() {
        let servers = vec![qb_server("nas", "NAS", "http://127.0.0.1:8080")];
        let res = normalize_subscription_categories(
            vec![category("电影", "影视"), category("剧集", "影视")],
            &servers,
        );
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
    fn torrent_match_log_entries_include_every_candidate_result() {
        let candidates = vec![
            torrent("1", "测试电影 2160p WEB-DL"),
            torrent("2", "测试电影 1080p HDTV"),
        ];
        let rules = vec![torrent_rule(
            "wanted 4k",
            100,
            config::TorrentRuleMatchMode::All,
            &["2160p"],
            &[],
            &["web-dl"],
        )];
        let matches = match_torrent_candidates(&candidates, &rules);

        let value = torrent_match_log_entries(&matches);
        let rows = value.as_array().expect("match log entries should be array");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("torrent_id").and_then(Value::as_str), Some("1"));
        assert_eq!(rows[0].get("selected").and_then(Value::as_bool), Some(true));
        assert_eq!(
            rows[0].get("matched_rule_name").and_then(Value::as_str),
            Some("wanted 4k")
        );
        assert!(rows[0]
            .get("rule_evaluations")
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty()));
        assert_eq!(rows[1].get("torrent_id").and_then(Value::as_str), Some("2"));
        assert_eq!(
            rows[1].get("excluded_reason").and_then(Value::as_str),
            Some("未命中任何规则")
        );
    }

    #[test]
    fn failed_push_record_keeps_qb_and_candidate_context() {
        let candidate = torrent("12345", "测试电影 2160p");
        let qb = QbServerEntry {
            id: "nas".to_string(),
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

    #[test]
    fn push_record_keeps_mteam_page_and_download_url() {
        let candidate = torrent("12345", "测试电影 2160p");
        let qb = qb_server("nas", "nas", "http://127.0.0.1:8080");
        let category = category("电影", "电影");
        let push = torrent_push_record_with_download_url(
            "subject-1",
            &qb,
            &category,
            &candidate,
            "pushed",
            Some(100),
            None,
            Some("https://api.m-team.cc/download/token"),
        );

        assert_eq!(
            push.mteam_torrent_url.as_deref(),
            Some("https://kp.m-team.cc/detail/12345")
        );
        assert_eq!(
            push.torrent_download_url.as_deref(),
            Some("https://api.m-team.cc/download/token")
        );
    }

    #[test]
    fn qb_torrent_lookup_matches_stored_hash_first() {
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let mut push = torrent_push_record(
            "subject-1",
            &qb,
            &category("电影", "电影"),
            &torrent("12345", "显示标题不一定等于 qB 名称"),
            "pushed",
            Some(100),
            None,
        );
        push.qb_hash = Some("hash-b".to_string());

        let mut first = qb_torrent("Other.Name");
        first.hash = "hash-a".to_string();
        let mut second = qb_torrent("Different.Internal.Name");
        second.hash = "hash-b".to_string();

        let candidates = qb_hash_lookup_candidates(&push, None);
        let found = select_qb_torrent_by_hash(&[first, second], &candidates).unwrap();
        assert_eq!(found.hash, "hash-b");
    }

    #[test]
    fn qb_hash_lookup_candidates_prefer_requested_then_stored_hash() {
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let mut push = torrent_push_record(
            "subject-1",
            &qb,
            &category("电影", "电影"),
            &torrent("12345", "测试电影 2160p"),
            "pushed",
            Some(100),
            None,
        );
        push.qb_hash = Some("stored-hash".to_string());

        assert_eq!(
            qb_hash_lookup_candidates(&push, Some("requested-hash")),
            vec!["requested-hash".to_string(), "stored-hash".to_string()]
        );
    }

    #[test]
    fn qb_lookup_identifiers_only_include_hashes() {
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let mut push = torrent_push_record(
            "subject-1",
            &qb,
            &category("TV", "TV"),
            &torrent("943109", "What If S01 2025 1080p WEB-DL H.264 AAC-ADWeb"),
            "pushed",
            Some(100),
            None,
        );
        push.qb_hash = Some("stored-hash".to_string());
        push.qb_name = Some("What.If.S01.2025.1080p.WEB-DL.H264.AAC-ADWeb".to_string());

        assert_eq!(
            qb_lookup_identifiers(&push, Some("requested-hash")),
            vec![
                "requested_hash=requested-hash".to_string(),
                "stored_hash=stored-hash".to_string(),
            ]
        );
    }

    #[test]
    fn existing_qb_lookup_is_applied_to_push_record() {
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let mut push = torrent_push_record(
            "subject-1",
            &qb,
            &category("电影", "电影"),
            &torrent("12345", "测试电影 2160p"),
            "pushed",
            Some(100),
            None,
        );
        let mut torrent = qb_torrent("qB 内已有任务");
        torrent.hash = "aaebc806fdbf63111b1f7dde3a89bc17d2988686".to_string();

        apply_existing_qb_lookup_to_push(&mut push, &torrent);

        assert_eq!(
            push.qb_hash.as_deref(),
            Some("aaebc806fdbf63111b1f7dde3a89bc17d2988686")
        );
        assert_eq!(push.qb_name.as_deref(), Some("qB 内已有任务"));
    }

    #[test]
    fn successful_progress_update_clears_previous_push_error() {
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let mut push = torrent_push_record(
            "subject-1",
            &qb,
            &category("电影", "电影"),
            &torrent("12345", "测试电影 2160p"),
            "failed",
            Some(100),
            Some("previous lookup failed".to_string()),
        );

        mark_push_progress_success(&mut push, false);

        assert_eq!(push.status, "downloading");
        assert_eq!(push.error, None);
    }

    #[test]
    fn watcher_uses_select_due_operation_for_movie_states() {
        let mut record = wanted_record("subject-1", "测试电影", Some(2024));
        record.lifecycle_state = subscription::SubscriptionLifecycleState::Downloading;
        record.next_attempt_at = Some(105);

        assert_eq!(select_watcher_due_operation(&record, 104), None);
        assert_eq!(
            select_watcher_due_operation(&record, 105),
            Some(subscription::SubscriptionDueOperation::MovieProgress)
        );
    }

    #[test]
    fn watcher_movie_progress_dispatches_to_progress_sync() {
        let source = include_str!("main.rs");
        let body = function_body(source, "execute_due_subscription_operation");
        let progress_branch = body
            .split("subscription::SubscriptionDueOperation::MovieProgress =>")
            .nth(1)
            .and_then(|part| {
                part.split("subscription::SubscriptionDueOperation::MovieLink =>")
                    .next()
            })
            .unwrap_or_else(|| panic!("MovieProgress dispatch branch should exist"));

        assert!(
            progress_branch.contains("process_wanted_progress_step"),
            "MovieProgress must sync qB download progress during watcher ticks"
        );
        assert!(
            !progress_branch.contains("process_wanted_completion_step"),
            "MovieProgress must not skip straight to completion checks"
        );
    }

    #[test]
    fn watcher_ignores_legacy_status_for_action_selection() {
        let mut record = wanted_record("subject-1", "测试电影", Some(2024));
        record.lifecycle_state = subscription::SubscriptionLifecycleState::Linking;
        record.next_attempt_at = Some(100);

        assert_eq!(
            select_watcher_due_operation(&record, 100),
            Some(subscription::SubscriptionDueOperation::MovieLink)
        );
    }

    #[test]
    fn due_operation_action_labels_are_stable() {
        assert_eq!(
            subscription::SubscriptionDueOperation::MovieSearch.as_str(),
            "movie_search"
        );
        assert_eq!(
            subscription::SubscriptionDueOperation::MovieLink.as_str(),
            "movie_link"
        );
        assert_eq!(
            subscription::SubscriptionDueOperation::TvLane(subscription::TvLaneKind::Progress)
                .as_str(),
            "tv_progress"
        );
    }

    #[test]
    fn rerun_preserves_existing_qb_lookup_for_same_torrent() {
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let category = category("电影", "电影");
        let mut existing = torrent_push_record(
            "subject-1",
            &qb,
            &category,
            &torrent(
                "1081744",
                "Love Undercover 2002 2160p WEB-DL H265 AAC2.0-CSWEB",
            ),
            "downloaded",
            Some(100),
            None,
        );
        existing.qb_hash = Some("fd7094".to_string());
        existing.qb_name = Some("[国语].新扎师妹.Love.Undercover.2002.2160p".to_string());
        let mut next = torrent_push_record(
            "subject-1",
            &qb,
            &category,
            &torrent(
                "1081744",
                "Love Undercover 2002 2160p WEB-DL H265 AAC2.0-CSWEB",
            ),
            "pushed",
            Some(200),
            None,
        );

        inherit_existing_qb_lookup(&mut next, Some(&existing));

        assert_eq!(next.qb_hash.as_deref(), Some("fd7094"));
        assert_eq!(
            next.qb_name.as_deref(),
            Some("[国语].新扎师妹.Love.Undercover.2002.2160p")
        );
    }

    #[tokio::test]
    async fn watcher_tick_runs_pipeline_between_douban_polls() {
        assert_eq!(WANTED_PIPELINE_TICK_SECS, 1);
        let interval = wanted_watch_interval();
        assert_eq!(
            interval.missed_tick_behavior(),
            tokio::time::MissedTickBehavior::Skip
        );
        assert_eq!(
            wanted_watch_tick_action(true, 1_000, None, 3_600),
            Some(WantedWatchTickAction::PollWanted)
        );
        assert_eq!(
            wanted_watch_tick_action(true, 1_060, Some(1_000), 3_600),
            Some(WantedWatchTickAction::ProcessPipeline)
        );
        assert_eq!(
            wanted_watch_tick_action(true, 4_600, Some(1_000), 3_600),
            Some(WantedWatchTickAction::PollWanted)
        );
        assert_eq!(
            wanted_watch_tick_action(false, 4_600, Some(1_000), 3_600),
            None
        );
    }

    #[test]
    fn completion_short_circuit_distinguishes_downloaded_from_linked() {
        let mut record = wanted_record("subject-1", "测试电影", Some(2024));
        record.last_push = Some(torrent_push_record(
            "subject-1",
            &QbServerEntry {
                id: "nas".to_string(),
                name: "nas".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "admin".to_string(),
                password: String::new(),
                insecure_tls: false,
            },
            &category("电影", "电影"),
            &torrent("12345", "测试电影 2160p"),
            "downloaded",
            Some(100),
            None,
        ));

        assert!(!subscription_completion_already_linked(&record));

        record.lifecycle_state = subscription::SubscriptionLifecycleState::Completed;
        assert!(subscription_completion_already_linked(&record));
    }

    #[test]
    fn torrent_candidates_are_sorted_by_seeders_descending() {
        let mut candidates = vec![
            torrent("1", "低做种 2160p"),
            torrent("2", "未知做种 2160p"),
            torrent("3", "高做种 2160p"),
        ];
        candidates[0].seeders = Some(8);
        candidates[1].seeders = None;
        candidates[2].seeders = Some(21);

        sort_torrent_candidates_by_seeders(&mut candidates);

        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.torrent_id.as_str())
                .collect::<Vec<_>>(),
            vec!["3", "1", "2"]
        );
    }

    #[test]
    fn mteam_search_body_requests_seeders_descending() {
        let body = mteam_search_body(2, 50, "keyword", "测试电影");

        assert_eq!(body.get("pageNumber").and_then(Value::as_u64), Some(2));
        assert_eq!(body.get("pageSize").and_then(Value::as_u64), Some(50));
        assert_eq!(
            body.get("keyword").and_then(Value::as_str),
            Some("测试电影")
        );
        assert_eq!(
            body.get("sortField").and_then(Value::as_str),
            Some("SEEDERS")
        );
        assert_eq!(
            body.get("sortDirection").and_then(Value::as_str),
            Some("DESC")
        );
    }

    #[test]
    fn qb_torrent_lookup_ignores_mteam_tag_when_hash_is_absent() {
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let push = torrent_push_record(
            "subject-1",
            &qb,
            &category("电影", "电影"),
            &torrent("12345", "M-Team 显示标题 2160p"),
            "pushed",
            Some(100),
            None,
        );

        let mut candidate = qb_torrent("qB 内部种子名完全不同");
        candidate.tags = "manual, mteam:12345".to_string();

        assert!(
            select_qb_torrent_by_hash(&[candidate], &qb_hash_lookup_candidates(&push, None))
                .is_none()
        );
    }

    #[test]
    fn qb_lookup_error_mentions_only_hash_counts_and_identifiers() {
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let mut push = torrent_push_record(
            "subject-1",
            &qb,
            &category("电影", "电影"),
            &torrent("12345", "测试电影 2160p"),
            "pushed",
            Some(100),
            None,
        );
        push.qb_hash = Some("abcdef".to_string());

        let message = qb_lookup_error_message(&qb, &push, Some("requested"), Some(7));

        assert!(message.contains("server=nas"));
        assert!(message.contains("hash候选返回=7"));
        assert!(message.contains("requested_hash=requested"));
        assert!(message.contains("stored_hash=abcdef"));
        assert!(!message.contains("category="));
        assert!(!message.contains("tag=mteam:12345"));
        assert!(!message.contains("title="));
        assert!(!message.contains("save_dir="));
    }

    fn test_app_state(root: &Path) -> AppState {
        AppState {
            config_path: root.join("config.toml"),
            config: std::sync::Arc::new(tokio::sync::RwLock::new(FileConfig::default())),
            tmdb_cache: TmdbDiskCache::new(root.join("tmdb"), Duration::from_secs(60)),
            douban_cache: TmdbDiskCache::new(root.join("douban"), Duration::from_secs(60)),
            douban_cache_ttl_secs: 60,
            douban_qr_sessions: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            wanted_store: subscription::WantedSubscriptionStore::new(root.join("subscriptions")),
        }
    }

    fn library_item(subject_id: &str, title: &str) -> douban::DoubanLibraryItem {
        douban::DoubanLibraryItem {
            source: "douban",
            media_type: "movie",
            id: subject_id.to_string(),
            subject_id: subject_id.to_string(),
            title: title.to_string(),
            url: format!("https://movie.douban.com/subject/{subject_id}/"),
            abstract_text: "2024 / 中国大陆".to_string(),
            abstract_2: String::new(),
            cover_url: String::new(),
            poster_url: String::new(),
            status: "wish",
            status_label: "想看",
            date: String::new(),
            comment: String::new(),
            tags: vec!["电影".to_string()],
            user_rating: None,
        }
    }

    fn function_body<'a>(source: &'a str, name: &str) -> &'a str {
        let signature = format!("fn {name}");
        let start = source.find(&signature).expect("function exists");
        let open_offset = source[start..].find('{').expect("function has body");
        let open = start + open_offset;
        let mut depth = 0usize;
        for (offset, ch) in source[open..].char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &source[open + 1..open + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("function body closes");
    }

    async fn seed_wanted_record(
        state: &AppState,
        account_key: &str,
        subject_id: &str,
    ) -> subscription::WantedSubscriptionRecord {
        let cfg = config::SubscriptionWatcherConfig {
            bootstrap_existing_as_skipped: false,
            ..config::SubscriptionWatcherConfig::default()
        };
        state
            .wanted_store
            .apply_wish_items(
                account_key,
                &[library_item(subject_id, "测试电影")],
                &cfg,
                100,
            )
            .await
            .unwrap();
        state
            .wanted_store
            .snapshot(account_key, 110)
            .await
            .unwrap()
            .records
            .get(subject_id)
            .unwrap()
            .clone()
    }

    #[test]
    fn automatic_progress_and_completion_steps_do_not_use_legacy_status_fallbacks() {
        let source = include_str!("main.rs");
        for name in [
            "process_wanted_progress_step",
            "process_wanted_completion_step",
        ] {
            let body = function_body(source, name);
            assert!(!body.contains("persist_if_status_unchanged"));
            assert!(!body.contains("persist_subscription_sync_error"));
        }
    }

    #[tokio::test]
    async fn watcher_queue_advances_pending_record_to_movie_meta() {
        let root = temp_test_dir("watcher_movie_meta");
        let state = test_app_state(&root);
        let account_key = "acct";
        let subject_id = "subject-stage";
        seed_wanted_record(&state, account_key, subject_id).await;
        {
            let mut cfg = state.config.write().await;
            cfg.douban_cookie = "dbcl2=acct:token; ck=test".to_string();
            cfg.subscription_categories = vec![category("电影", "电影")];
            cfg.qb_servers = vec![QbServerEntry {
                id: "nas".to_string(),
                name: "nas".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "admin".to_string(),
                password: String::new(),
                insecure_tls: false,
            }];
        }

        process_wanted_watch_queue(&state, account_key)
            .await
            .unwrap_or_else(|err| panic!("{}", err.message()));
        let snapshot = state.wanted_store.snapshot(account_key, 220).await.unwrap();
        let record = snapshot.records.get(subject_id).unwrap();
        assert_eq!(
            record.lifecycle_state,
            subscription::SubscriptionLifecycleState::Meta
        );
        assert_eq!(record.last_error, None);
        assert!(record.failure.is_none());
        assert_eq!(record.next_attempt_at, Some(record.updated_at));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn manual_sync_error_persists_as_semantic_failure() {
        let root = temp_test_dir("manual_sync_error");
        let state = test_app_state(&root);
        let account_key = "acct";
        let subject_id = "subject-sync";
        seed_wanted_record(&state, account_key, subject_id).await;

        let watcher_cfg = state.config.read().await.subscription_watcher.clone();
        let record = state
            .wanted_store
            .apply_parent_operation_failure_result(
                account_key,
                subject_id,
                "progress",
                "qB 服务器不存在",
                &watcher_cfg,
                200,
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(record.retry_count, 1);
        assert_eq!(record.last_error.as_deref(), Some("qB 服务器不存在"));
        assert_eq!(record.failure.as_ref().unwrap().operation, "progress");
        let snapshot = state.wanted_store.snapshot(account_key, 210).await.unwrap();
        let persisted = snapshot.records.get(subject_id).unwrap();
        assert_eq!(persisted.retry_count, 1);
        assert_eq!(persisted.last_error.as_deref(), Some("qB 服务器不存在"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn movie_search_step_failure_persists_semantic_retry_state() {
        let root = temp_test_dir("movie_search_step_failure");
        let state = test_app_state(&root);
        let account_key = "acct";
        let subject_id = "subject-search-failure";
        seed_wanted_record(&state, account_key, subject_id).await;
        {
            let mut cfg = state.config.write().await;
            cfg.douban_cookie = "dbcl2=acct:token; ck=test".to_string();
            cfg.mteam_api_key = String::new();
            cfg.subscription_watcher.search_interval_secs = 5;
            cfg.subscription_categories = vec![category("电影", "电影")];
            cfg.qb_servers = vec![QbServerEntry {
                id: "nas".to_string(),
                name: "nas".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "admin".to_string(),
                password: String::new(),
                insecure_tls: false,
            }];
        }
        state
            .wanted_store
            .transition_movie_operation(
                account_key,
                subject_id,
                subscription::MovieOperationOutcome::Advanced(
                    subscription::SubscriptionLifecycleState::Searching,
                ),
                &state.config.read().await.subscription_watcher,
                200,
            )
            .await
            .unwrap()
            .unwrap();
        let before = state.wanted_store.snapshot(account_key, 210).await.unwrap();
        let before_record = before.records.get(subject_id).unwrap();
        assert_eq!(
            before_record.lifecycle_state,
            subscription::SubscriptionLifecycleState::Searching
        );

        let err = process_wanted_push_step(&state, account_key, subject_id, true)
            .await
            .unwrap_err();

        assert!(err.message().contains("M-Team API Key"));
        let snapshot = state.wanted_store.snapshot(account_key, 220).await.unwrap();
        let record = snapshot.records.get(subject_id).unwrap();
        assert_eq!(
            record.lifecycle_state,
            subscription::SubscriptionLifecycleState::Searching
        );
        let failure = record.failure.as_ref().unwrap();
        assert_eq!(failure.operation, "search");
        assert_eq!(failure.retry_count, 1);
        assert!(!failure.retry_blocked);
        assert_eq!(record.retry_count, 1);
        assert!(record.next_attempt_at.is_some());
        assert!(record
            .attention_tags
            .contains(&subscription::SubscriptionAttentionTag::Failed));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn stale_search_failure_does_not_suppress_new_push_step_fallback() {
        let root = temp_test_dir("push_step_stale_failure");
        let state = test_app_state(&root);
        let account_key = "acct";
        let subject_id = "subject-stale-failure";
        seed_wanted_record(&state, account_key, subject_id).await;
        {
            let mut cfg = state.config.write().await;
            cfg.douban_cookie = "dbcl2=acct:token; ck=test".to_string();
            cfg.mteam_api_key = String::new();
            cfg.subscription_watcher.search_interval_secs = 5;
            cfg.subscription_categories = vec![category("电影", "电影")];
            cfg.qb_servers = vec![QbServerEntry {
                id: "nas".to_string(),
                name: "nas".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "admin".to_string(),
                password: String::new(),
                insecure_tls: false,
            }];
        }

        let category = category("电影", "电影");
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let selected = subscription::TorrentCandidateMatchRecord {
            candidate: torrent("12345", "测试电影 2160p"),
            selected: true,
            matched_rule_name: Some("default".to_string()),
            matched_priority: Some(1),
            matched_keywords: Vec::new(),
            excluded_reason: None,
            rule_evaluations: Vec::new(),
        };
        let watcher_cfg = state.config.read().await.subscription_watcher.clone();
        state
            .wanted_store
            .update_candidate_matches(account_key, subject_id, vec![selected.clone()], 205)
            .await
            .unwrap()
            .unwrap();
        let old_push = torrent_push_record(
            subject_id,
            &qb,
            &category,
            &selected.candidate,
            "failed",
            None,
            Some("qB 添加种子失败".to_string()),
        );
        state
            .wanted_store
            .apply_movie_push_result(
                account_key,
                subject_id,
                old_push,
                true,
                Some("qB 添加种子失败".to_string()),
                &watcher_cfg,
                210,
            )
            .await
            .unwrap()
            .unwrap();
        let before = state.wanted_store.snapshot(account_key, 215).await.unwrap();
        let before_record = before.records.get(subject_id).unwrap();
        let old_failed_at = before_record.failure.as_ref().unwrap().failed_at;
        let old_updated_at = before_record.updated_at;
        assert_eq!(before_record.retry_count, 1);
        assert_eq!(before_record.failure.as_ref().unwrap().operation, "search");
        assert_eq!(
            before_record.last_push.as_ref().unwrap().status.as_str(),
            "failed"
        );

        let err = process_wanted_push_step(&state, account_key, subject_id, true)
            .await
            .unwrap_err();

        assert!(err.message().contains("M-Team API Key"));
        let snapshot = state.wanted_store.snapshot(account_key, 220).await.unwrap();
        let record = snapshot.records.get(subject_id).unwrap();
        assert_eq!(record.retry_count, 2);
        assert_eq!(record.failure.as_ref().unwrap().operation, "search");
        assert!(record.failure.as_ref().unwrap().failed_at > old_failed_at);
        assert!(record.updated_at > old_updated_at);
        assert!(record
            .failure
            .as_ref()
            .unwrap()
            .message
            .contains("M-Team API Key"));
        assert_eq!(record.last_push.as_ref().unwrap().status.as_str(), "failed");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn same_attempt_push_failure_suppresses_search_step_fallback() {
        let root = temp_test_dir("push_step_same_attempt_failure");
        let state = test_app_state(&root);
        let account_key = "acct";
        let subject_id = "subject-push-same-attempt";
        seed_wanted_record(&state, account_key, subject_id).await;
        {
            let mut cfg = state.config.write().await;
            cfg.douban_cookie = "dbcl2=acct:token; ck=test".to_string();
            cfg.subscription_watcher.search_interval_secs = 5;
            cfg.subscription_categories = vec![category("电影", "电影")];
            cfg.qb_servers = vec![QbServerEntry {
                id: "nas".to_string(),
                name: "nas".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "admin".to_string(),
                password: String::new(),
                insecure_tls: false,
            }];
        }

        let category = category("电影", "电影");
        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let selected = subscription::TorrentCandidateMatchRecord {
            candidate: torrent("12345", "测试电影 2160p"),
            selected: true,
            matched_rule_name: Some("default".to_string()),
            matched_priority: Some(1),
            matched_keywords: Vec::new(),
            excluded_reason: None,
            rule_evaluations: Vec::new(),
        };
        state
            .wanted_store
            .update_candidate_matches(account_key, subject_id, vec![selected.clone()], 205)
            .await
            .unwrap()
            .unwrap();
        let original = state
            .wanted_store
            .snapshot(account_key, 210)
            .await
            .unwrap()
            .records
            .get(subject_id)
            .unwrap()
            .clone();
        record_push_failure(
            &state,
            account_key,
            subject_id,
            &qb,
            &category,
            Some(&selected),
            "qB 添加种子失败".to_string(),
            &state.config.read().await.subscription_watcher,
        )
        .await
        .unwrap_or_else(|err| panic!("{}", err.message()));

        let should_persist = should_persist_search_step_failure_fallback(
            &state,
            account_key,
            subject_id,
            Some(&original),
        )
        .await;

        assert!(!should_persist);
        let snapshot = state.wanted_store.snapshot(account_key, 220).await.unwrap();
        let record = snapshot.records.get(subject_id).unwrap();
        assert_eq!(record.retry_count, 1);
        assert_eq!(record.failure.as_ref().unwrap().operation, "search");
        assert_eq!(record.last_push.as_ref().unwrap().status.as_str(), "failed");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn missing_push_progress_logs_failed_operation_and_keeps_semantic_state() {
        let root = temp_test_dir("missing_push_progress_log");
        let state = test_app_state(&root);
        let account_key = "acct";
        let subject_id = "subject-progress-missing-push";
        seed_wanted_record(&state, account_key, subject_id).await;
        {
            let mut cfg = state.config.write().await;
            cfg.douban_cookie = "dbcl2=acct:token; ck=test".to_string();
            cfg.subscription_categories = vec![category("电影", "电影")];
        }
        state
            .wanted_store
            .transition_movie_operation(
                account_key,
                subject_id,
                subscription::MovieOperationOutcome::Advanced(
                    subscription::SubscriptionLifecycleState::Downloading,
                ),
                &state.config.read().await.subscription_watcher,
                200,
            )
            .await
            .unwrap()
            .unwrap();

        let err = wanted_subscription_progress(
            State(state.clone()),
            PathParam(subject_id.to_string()),
            Json(WantedProgressBody::default()),
        )
        .await
        .unwrap_err();

        assert!(err.message().contains("缺少 qB pushed record"));
        let snapshot = state.wanted_store.snapshot(account_key, 210).await.unwrap();
        let record = snapshot.records.get(subject_id).unwrap();
        assert_eq!(
            record.lifecycle_state,
            subscription::SubscriptionLifecycleState::Downloading
        );
        assert_eq!(record.failure.as_ref().unwrap().operation, "progress");

        let logs = state
            .wanted_store
            .query_operation_logs(subscription::OperationLogQuery {
                status: Some("failed".to_string()),
                ..subscription::OperationLogQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(logs.items.len(), 1);
        let log = &logs.items[0];
        assert_eq!(log.category, "download_progress");
        assert_eq!(log.action, "sync_progress");
        assert_eq!(log.target_id.as_deref(), Some(subject_id));
        assert!(log.summary.contains("缺少 qB pushed record"));
        assert!(log
            .error
            .as_deref()
            .unwrap()
            .contains("缺少 qB pushed record"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn missing_push_completion_logs_failed_operation_and_keeps_semantic_state() {
        let root = temp_test_dir("missing_push_completion_log");
        let state = test_app_state(&root);
        let account_key = "acct";
        let subject_id = "subject-completion-missing-push";
        seed_wanted_record(&state, account_key, subject_id).await;
        {
            let mut cfg = state.config.write().await;
            cfg.douban_cookie = "dbcl2=acct:token; ck=test".to_string();
            cfg.subscription_categories = vec![category("电影", "电影")];
        }
        state
            .wanted_store
            .transition_movie_operation(
                account_key,
                subject_id,
                subscription::MovieOperationOutcome::Advanced(
                    subscription::SubscriptionLifecycleState::Linking,
                ),
                &state.config.read().await.subscription_watcher,
                200,
            )
            .await
            .unwrap()
            .unwrap();

        let err = wanted_subscription_completion(
            State(state.clone()),
            PathParam(subject_id.to_string()),
            Json(WantedCompletionBody::default()),
        )
        .await
        .unwrap_err();

        assert!(err.message().contains("缺少 qB pushed record"));
        let snapshot = state.wanted_store.snapshot(account_key, 210).await.unwrap();
        let record = snapshot.records.get(subject_id).unwrap();
        assert_eq!(
            record.lifecycle_state,
            subscription::SubscriptionLifecycleState::Linking
        );
        assert_eq!(record.failure.as_ref().unwrap().operation, "link");

        let logs = state
            .wanted_store
            .query_operation_logs(subscription::OperationLogQuery {
                status: Some("failed".to_string()),
                ..subscription::OperationLogQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(logs.items.len(), 1);
        let log = &logs.items[0];
        assert_eq!(log.category, "hardlink");
        assert_eq!(log.action, "link_result");
        assert_eq!(log.target_id.as_deref(), Some(subject_id));
        assert!(log.summary.contains("缺少 qB pushed record"));
        assert!(log
            .error
            .as_deref()
            .unwrap()
            .contains("缺少 qB pushed record"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn progress_and_completion_sync_errors_persist_snapshots() {
        let root = temp_test_dir("sync_error_snapshots");
        let state = test_app_state(&root);
        let account_key = "acct";
        let subject_id = "subject-progress";
        seed_wanted_record(&state, account_key, subject_id).await;

        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let mut push = torrent_push_record(
            subject_id,
            &qb,
            &category("电影", "电影"),
            &torrent("12345", "测试电影 2160p"),
            "downloading",
            Some(120),
            None,
        );
        push.download_progress = Some(0.4);
        push.files = vec![subscription::TorrentFileProgressRecord {
            name: "Movie.2024.mkv".to_string(),
            size: 10,
            progress: 0.4,
            priority: 1,
            season_number: None,
            episode_number: None,
            episode_end_number: None,
            episode_label: None,
        }];

        let progress_record = persist_progress_sync_error(
            &state,
            account_key,
            subject_id,
            push.clone(),
            "qB 中未找到已推送种子",
            220,
        )
        .await
        .unwrap_or_else(|err| panic!("{}", err.message()));
        let progress_push = progress_record.last_push.as_ref().unwrap();
        assert_eq!(
            progress_record.lifecycle_state,
            subscription::SubscriptionLifecycleState::Downloading
        );
        assert_eq!(
            progress_record.failure.as_ref().unwrap().operation,
            "progress"
        );
        assert_eq!(progress_record.next_attempt_at, Some(820));
        assert_eq!(progress_push.status, "failed");
        assert_eq!(progress_push.checked_at, Some(220));
        assert_eq!(progress_push.download_progress, Some(0.4));
        assert_eq!(
            progress_push.error.as_deref(),
            Some("qB 中未找到已推送种子")
        );

        let completion_record = persist_completion_sync_error(
            &state,
            account_key,
            subject_id,
            push,
            "qB 文件列表读取失败",
            240,
        )
        .await
        .unwrap_or_else(|err| panic!("{}", err.message()));
        let completion = completion_record.last_completion.as_ref().unwrap();
        assert_eq!(completion.status, "failed");
        assert_eq!(completion.checked_at, 240);
        assert_eq!(completion.error.as_deref(), Some("qB 文件列表读取失败"));
        assert_eq!(
            completion_record.lifecycle_state,
            subscription::SubscriptionLifecycleState::Linking
        );
        assert_eq!(
            completion_record.failure.as_ref().unwrap().operation,
            "link"
        );
        assert_eq!(
            completion_record.last_error.as_deref(),
            Some("qB 文件列表读取失败")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn dry_run_completion_result_persists_planned_files_and_episodes() {
        let root = temp_test_dir("dry_run_completion");
        let state = test_app_state(&root);
        let account_key = "acct";
        let subject_id = "subject-dry-run";
        seed_wanted_record(&state, account_key, subject_id).await;

        let qb = QbServerEntry {
            id: "nas".to_string(),
            name: "nas".to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: String::new(),
            insecure_tls: false,
        };
        let mut push = torrent_push_record(
            subject_id,
            &qb,
            &category("剧集", "剧集"),
            &torrent("67890", "Show.S01E01-E02"),
            "downloaded",
            Some(120),
            None,
        );
        push.qb_hash = Some("hash-dry".to_string());
        push.qb_name = Some("Show.S01E01-E02".to_string());

        let plan = HardlinkPlan {
            source_root: root.join("downloads"),
            target_dir: root.join("media/测试剧.2024"),
            qb_hash: "hash-dry".to_string(),
            qb_name: "Show.S01E01-E02".to_string(),
            files: vec![HardlinkFilePlan {
                source_path: root.join("downloads/Show.S01E01-E02.mkv"),
                target_path: root.join("media/测试剧.2024/Show.S01E01-E02.mkv"),
                size: 10,
                season_number: Some(1),
                episode_number: Some(1),
                episode_end_number: Some(2),
                episode_label: Some("S01E01-E02".to_string()),
            }],
        };
        let completion = dry_run_hardlink_plan(&plan, 260);
        let record = state
            .wanted_store
            .apply_movie_completion_result(
                account_key,
                subject_id,
                push,
                completion,
                subscription::MovieCompletionOutcome::LinkPlanned,
                None,
                &state.config.read().await.subscription_watcher,
                260,
            )
            .await
            .unwrap()
            .unwrap();

        let persisted = record.last_completion.as_ref().unwrap();
        assert_eq!(persisted.status, "dry_run");
        assert_eq!(persisted.linked_files[0].status, "planned");
        let episode_rows = persisted
            .episodes
            .iter()
            .map(|episode| (episode.label.as_str(), episode.status.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            episode_rows,
            vec![("S01E01", "planned"), ("S01E02", "planned")]
        );
        assert!(record.last_error.is_none());

        let reloaded = state.wanted_store.snapshot(account_key, 270).await.unwrap();
        let reloaded_completion = reloaded
            .records
            .get(subject_id)
            .unwrap()
            .last_completion
            .as_ref()
            .unwrap();
        assert_eq!(reloaded_completion.status, "dry_run");
        assert_eq!(reloaded_completion.linked_files[0].status, "planned");

        let _ = std::fs::remove_dir_all(root);
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
            poster_url: String::new(),
            cover_url: String::new(),
            original_title: None,
            aka: Vec::new(),
            languages: Vec::new(),
            countries: Vec::new(),
            genres: Vec::new(),
            directors: Vec::new(),
            actors: Vec::new(),
            date_published: None,
            duration: None,
            summary: None,
            rating_value: None,
            rating_count: None,
            category_text: Some("电影".to_string()),
            tags: vec!["电影".to_string()],
            douban_date: None,
            douban_sort_time: None,
            douban_return_order: None,
            lifecycle_state: subscription::SubscriptionLifecycleState::Downloading,
            execution_state: subscription::SubscriptionExecutionState::Idle,
            attention_tags: Vec::new(),
            failure: None,
            next_attempt_at: Some(100),
            force_eligible_once: false,
            media_kind: subscription::SubscriptionMediaKind::Movie,
            tv: None,
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
            tags: String::new(),
            save_path: "/downloads/movie".to_string(),
            content_path: String::new(),
            progress: 1.0,
            size: 5,
            downloaded: 5,
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
                id: "qb".to_string(),
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
            &wanted_record("subject-1", "测试电影 / Test Movie", Some(2024)),
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
        assert_eq!(plan.files.len(), 2);
        assert_eq!(plan.target_dir, target.join("测试电影.2024"));
        assert_eq!(
            plan.files[0].source_path,
            source.join("TorrentRoot/movie.mkv")
        );
        assert_eq!(
            plan.files[0].target_path,
            target.join("测试电影.2024/TorrentRoot/movie.mkv")
        );
        assert_eq!(
            plan.files[1].target_path,
            target.join("测试电影.2024/TorrentRoot/Sample/sample.mkv")
        );
    }

    #[test]
    fn hardlink_title_uses_first_chinese_alias_segment() {
        assert_eq!(
            hardlink_title_component(&wanted_record(
                "subject-1",
                "The Furious / 火遮眼 / 狂怒",
                Some(2025),
            )),
            "火遮眼"
        );
        assert_eq!(
            hardlink_title_component(&wanted_record(
                "subject-1",
                "火遮眼 / 狂怒 / The Furious",
                Some(2025),
            )),
            "火遮眼"
        );
    }

    #[test]
    fn hardlink_plan_preserves_all_safe_qb_files() {
        let root = temp_test_dir("plan_all_files");
        let source = root.join("downloads");
        let target = root.join("links");
        std::fs::create_dir_all(source.join("TorrentRoot/Screenshots")).unwrap();
        std::fs::write(source.join("TorrentRoot/movie.mkv"), b"movie").unwrap();
        std::fs::write(source.join("TorrentRoot/readme.txt"), b"readme").unwrap();
        std::fs::write(source.join("TorrentRoot/Screenshots/shot01.png"), b"shot").unwrap();

        let mut category = category("电影", "电影");
        category.download_dir = source.display().to_string();
        category.link_target_dir = target.display().to_string();
        let push = torrent_push_record(
            "subject-1",
            &qb_server("nas", "NAS", "http://127.0.0.1:8080"),
            &category,
            &torrent("123", "Movie 2160p"),
            "pushed",
            Some(100),
            None,
        );
        let plan = match build_hardlink_plan(
            &wanted_record("subject-1", "测试电影", Some(2024)),
            &category,
            &push,
            &qb_torrent("Movie 2160p"),
            &[
                qb_file("TorrentRoot/movie.mkv", 5),
                qb_file("TorrentRoot/readme.txt", 2),
                qb_file("TorrentRoot/Screenshots/shot01.png", 1),
                qb_file("../escape.txt", 1),
            ],
            200,
        ) {
            Ok(plan) => plan,
            Err(err) => panic!(
                "valid completed torrent should produce a hardlink plan: {}",
                err.message()
            ),
        };

        let targets = plan
            .files
            .iter()
            .map(|file| {
                file.target_path
                    .strip_prefix(&plan.target_dir)
                    .unwrap()
                    .to_path_buf()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            targets,
            vec![
                PathBuf::from("TorrentRoot/movie.mkv"),
                PathBuf::from("TorrentRoot/readme.txt"),
                PathBuf::from("TorrentRoot/Screenshots/shot01.png"),
            ]
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
                season_number: None,
                episode_number: None,
                episode_end_number: None,
                episode_label: None,
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
                season_number: None,
                episode_number: None,
                episode_end_number: None,
                episode_label: None,
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
    fn hardlink_execution_renames_target_conflicts() {
        let root = temp_test_dir("target_conflict");
        let source = root.join("source/movie.mkv");
        let target = root.join("target/movie.mkv");
        let renamed = root.join("target/movie.1.mkv");
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
                source_path: source.clone(),
                target_path: target.clone(),
                size: 6,
                season_number: None,
                episode_number: None,
                episode_end_number: None,
                episode_label: None,
            }],
        };
        let result = execute_hardlink_plan(&plan, 300);
        assert_eq!(result.status, "completed");
        assert_eq!(result.linked_files[0].status, "linked");
        assert_eq!(
            result.linked_files[0].target_path,
            renamed.display().to_string()
        );
        assert!(same_file(&source, &renamed).unwrap());
        assert!(!same_file(&source, &target).unwrap());
    }

    #[test]
    fn hardlink_error_message_calls_out_cross_device() {
        let err = std::io::Error::from_raw_os_error(18);
        assert!(hardlink_error_message(&err).contains("跨设备硬链接失败"));
    }

    #[test]
    fn episode_progress_groups_common_episode_file_names() {
        let files = vec![
            qbittorrent::QbTorrentFile {
                name: "Show.S01E01.2160p.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.S01E02.2160p.mkv".to_string(),
                size: 10,
                progress: 0.5,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show/第03集.mkv".to_string(),
                size: 10,
                progress: 0.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.S01E04-E05.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.S02.Complete.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.Season.3.Complete.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.S00E01.Special.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
        ];
        let progress = torrent_file_progress_records(&files);
        let episodes = episode_records_from_file_progress(&progress);
        let labels = episodes
            .iter()
            .map(|episode| episode.label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            labels,
            vec![
                "E03",
                "S00E01",
                "S01E01",
                "S01E02",
                "S01E03",
                "S01E04",
                "S01E05",
                "S02 全季",
                "S03 全季"
            ]
        );
        assert_eq!(episodes[1].status, "downloaded");
        assert_eq!(episodes[2].status, "downloaded");
        assert_eq!(episodes[3].status, "downloading");
        assert_eq!(episodes[4].status, "missing");
    }

    #[test]
    fn episode_progress_expands_ranges_and_plain_episode_tokens() {
        let files = vec![
            qbittorrent::QbTorrentFile {
                name: "Show/01.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show/1-3.mkv".to_string(),
                size: 10,
                progress: 0.5,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.S01E04-E05.mkv".to_string(),
                size: 10,
                progress: 0.0,
                priority: 1,
            },
        ];

        let progress = torrent_file_progress_records(&files);
        let labels = episode_records_from_file_progress(&progress)
            .into_iter()
            .map(|episode| (episode.label, episode.status))
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec![
                ("E01".to_string(), "duplicate".to_string()),
                ("E02".to_string(), "downloading".to_string()),
                ("E03".to_string(), "downloading".to_string()),
                ("S01E04".to_string(), "pending".to_string()),
                ("S01E05".to_string(), "pending".to_string()),
            ]
        );
    }

    #[test]
    fn episode_progress_marks_unknown_missing_and_season_conflicts() {
        let files = vec![
            qbittorrent::QbTorrentFile {
                name: "Show/ambiguous-part-a.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.S01E01.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.S01E03.mkv".to_string(),
                size: 10,
                progress: 0.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.S01.Complete.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
        ];

        let progress = torrent_file_progress_records(&files);
        let by_label = episode_records_from_file_progress(&progress)
            .into_iter()
            .map(|episode| (episode.label, episode.status))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            by_label.get(UNKNOWN_EPISODE_LABEL),
            Some(&"needs_review".to_string())
        );
        assert_eq!(by_label.get("S01 全季"), Some(&"conflict".to_string()));
        assert_eq!(by_label.get("S01E01"), Some(&"conflict".to_string()));
        assert_eq!(by_label.get("S01E02"), Some(&"missing".to_string()));
        assert_eq!(by_label.get("S01E03"), Some(&"conflict".to_string()));
    }

    #[test]
    fn episode_progress_keeps_single_movie_file_without_episode_records() {
        let files = vec![qbittorrent::QbTorrentFile {
            name: "Movie.2024.2160p.mkv".to_string(),
            size: 10,
            progress: 1.0,
            priority: 1,
        }];

        let progress = torrent_file_progress_records(&files);

        assert_eq!(progress.len(), 1);
        assert!(progress[0].episode_label.is_none());
        assert!(episode_records_from_file_progress(&progress).is_empty());
    }

    #[test]
    fn hardlink_episode_records_expand_range_and_preserve_link_status() {
        let files = vec![subscription::HardlinkFileRecord {
            source_path: "source/Show.S01E01-E02.mkv".to_string(),
            target_path: "target/Show.S01E01-E02.mkv".to_string(),
            size: 10,
            status: "linked".to_string(),
            season_number: Some(1),
            episode_number: Some(1),
            episode_end_number: Some(2),
            episode_label: Some("S01E01-E02".to_string()),
            error: None,
        }];

        let episodes = episode_records_from_hardlink_files(&files)
            .into_iter()
            .map(|episode| (episode.label, episode.status, episode.linked_file_count))
            .collect::<Vec<_>>();

        assert_eq!(
            episodes,
            vec![
                ("S01E01".to_string(), "linked".to_string(), 1),
                ("S01E02".to_string(), "linked".to_string(), 1),
            ]
        );
    }

    #[test]
    fn qb_progress_snapshot_updates_push_file_and_episode_records() {
        let mut category = category("剧集", "剧集");
        category.qb_category = "tv".to_string();
        let mut push = torrent_push_record(
            "subject-tv",
            &QbServerEntry {
                id: "nas".to_string(),
                name: "nas".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "u".to_string(),
                password: "p".to_string(),
                insecure_tls: false,
            },
            &category,
            &torrent("456", "Show.S01"),
            "pushed",
            Some(100),
            None,
        );
        let torrent = qbittorrent::QbTorrentInfo {
            hash: "hash-tv".to_string(),
            name: "Show.S01".to_string(),
            category: "tv".to_string(),
            tags: String::new(),
            save_path: "/downloads/tv".to_string(),
            content_path: "tv/Show.S01".to_string(),
            progress: 0.5,
            size: 20,
            downloaded: 10,
            completion_on: -1,
            state: "downloading".to_string(),
        };
        let files = vec![
            qbittorrent::QbTorrentFile {
                name: "Show.S01E01.mkv".to_string(),
                size: 10,
                progress: 1.0,
                priority: 1,
            },
            qbittorrent::QbTorrentFile {
                name: "Show.S01E02.mkv".to_string(),
                size: 10,
                progress: 0.0,
                priority: 1,
            },
        ];

        apply_qb_progress_to_push(&mut push, &torrent, &files);

        assert_eq!(push.download_progress, Some(0.5));
        assert_eq!(push.download_state.as_deref(), Some("downloading"));
        assert_eq!(push.total_size, Some(20));
        assert_eq!(push.completed_file_count, Some(1));
        assert_eq!(push.total_file_count, Some(2));
        assert_eq!(push.files.len(), 2);
        assert_eq!(push.episodes.len(), 2);
        assert_eq!(push.episodes[0].label, "S01E01");
        assert_eq!(push.episodes[0].status, "downloaded");
        assert_eq!(push.episodes[1].label, "S01E02");
        assert_eq!(push.episodes[1].status, "pending");
    }

    #[test]
    fn qb_progress_without_files_preserves_existing_file_and_episode_records() {
        let mut category = category("剧集", "剧集");
        category.qb_category = "tv".to_string();
        let mut push = torrent_push_record(
            "subject-tv",
            &QbServerEntry {
                id: "nas".to_string(),
                name: "nas".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "u".to_string(),
                password: "p".to_string(),
                insecure_tls: false,
            },
            &category,
            &torrent("456", "Show.S01"),
            "pushed",
            Some(100),
            None,
        );
        push.files = vec![subscription::TorrentFileProgressRecord {
            name: "Show.S01E01.mkv".to_string(),
            size: 10,
            progress: 0.5,
            priority: 1,
            season_number: Some(1),
            episode_number: Some(1),
            episode_end_number: None,
            episode_label: Some("S01E01".to_string()),
        }];
        push.episodes = episode_records_from_file_progress(&push.files);
        push.total_file_count = Some(1);
        push.completed_file_count = Some(0);
        let torrent = qbittorrent::QbTorrentInfo {
            hash: "hash-tv".to_string(),
            name: "Show.S01".to_string(),
            category: "tv".to_string(),
            tags: String::new(),
            save_path: "/downloads/tv".to_string(),
            content_path: "tv/Show.S01".to_string(),
            progress: 0.7,
            size: 20,
            downloaded: 14,
            completion_on: -1,
            state: "downloading".to_string(),
        };

        apply_qb_progress_to_push(&mut push, &torrent, &[]);

        assert_eq!(push.download_progress, Some(0.7));
        assert_eq!(push.download_state.as_deref(), Some("downloading"));
        assert_eq!(push.total_size, Some(20));
        assert_eq!(push.completed_file_count, Some(0));
        assert_eq!(push.total_file_count, Some(1));
        assert_eq!(push.files.len(), 1);
        assert_eq!(push.episodes.len(), 1);
        assert_eq!(push.episodes[0].label, "S01E01");
        assert_eq!(push.episodes[0].status, "downloading");
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
