use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue};
use axum::response::{IntoResponse, Json};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::app::audit::{operation_log_entry, write_operation_log, OperationLogEvent};
use crate::app::AppState;
use crate::config::{
    self, ConfigUpdateError, FileConfig, QbServerEntry, SubscriptionCategory,
    SubscriptionWatcherConfig, TorrentMatchRule,
};
use crate::douban;
use crate::http::error::{config_update_error, ApiError, ApiJson};

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/config", get(get_config).put(put_config))
}

#[derive(Debug, Serialize)]
struct ConfigResponse {
    revision: u64,
    listen_ip: String,
    listen_port: u16,
    has_tmdb_api_key: bool,
    has_mteam_api_key: bool,
    has_douban_cookie: bool,
    has_admin_token: bool,
    qb_servers: Vec<RedactedQbServerResponse>,
    subscription_categories: Vec<SubscriptionCategory>,
    subscription_watcher: SubscriptionWatcherConfig,
    torrent_match_rules: Vec<TorrentMatchRule>,
    allowed_origins: Vec<String>,
    secure_cookie: bool,
    restart_required: bool,
}

#[derive(Debug, Serialize)]
struct RedactedQbServerResponse {
    id: String,
    name: String,
    base_url: String,
    username: String,
    insecure_tls: bool,
    has_password: bool,
}

async fn get_config(State(state): State<AppState>) -> Json<ConfigResponse> {
    let snapshot = state.config.snapshot().await;
    Json(config_response(snapshot.revision, &snapshot.value, false))
}

#[derive(Debug, Deserialize)]
struct PutConfigBody {
    expected_revision: u64,
    #[serde(default)]
    confirm_enable_automation: bool,
    #[serde(flatten)]
    patch: ConfigPatch,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigPatch {
    #[serde(default)]
    listen_ip: Option<String>,
    #[serde(default)]
    listen_port: Option<u16>,
    #[serde(default)]
    tmdb_api_key: Option<String>,
    #[serde(default)]
    clear_tmdb_api_key: bool,
    #[serde(default)]
    mteam_api_key: Option<String>,
    #[serde(default)]
    clear_mteam_api_key: bool,
    #[serde(default)]
    douban_cookie: Option<String>,
    #[serde(default)]
    clear_douban_cookie: bool,
    #[serde(default)]
    admin_token: Option<String>,
    #[serde(default)]
    clear_admin_token: bool,
    #[serde(default)]
    qb_servers: Option<Vec<QbServerPatch>>,
    #[serde(default)]
    subscription_categories: Option<Vec<SubscriptionCategory>>,
    #[serde(default)]
    subscription_watcher: Option<SubscriptionWatcherConfig>,
    #[serde(default)]
    torrent_match_rules: Option<Vec<TorrentMatchRule>>,
    #[serde(default)]
    allowed_origins: Option<Vec<String>>,
    #[serde(default)]
    secure_cookie: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QbServerPatch {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    base_url: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    insecure_tls: bool,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    clear_password: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SecretPatch {
    Keep,
    Set(String),
    Clear,
}

async fn put_config(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<PutConfigBody>,
) -> Result<impl IntoResponse, ApiError> {
    let expected_revision = body.expected_revision;
    let confirm_enable_automation = body.confirm_enable_automation;
    let patch = body.patch;
    let before = state.config.snapshot().await;
    let update = state
        .config
        .update(Some(expected_revision), move |cfg| {
            let requested = merge_config_patch(cfg, patch)?;
            require_automation_enable_confirmation(cfg, &requested, confirm_enable_automation)?;
            *cfg = requested;
            Ok(())
        })
        .await;

    let snapshot = match update {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let summary = match &error {
                ConfigUpdateError::Stale { .. } => "配置保存失败：revision 已过期",
                ConfigUpdateError::Mutation(_) => "配置保存失败：配置无效",
                ConfigUpdateError::Persist(_) => "配置保存失败：写入配置文件失败",
            };
            let api_error = config_update_error(error);
            let cfg = state.config.snapshot().await.value;
            write_operation_log(
                &state,
                operation_log_entry(
                    "system",
                    OperationLogEvent {
                        category: "configuration",
                        action: "save_config",
                        target_type: "config",
                        target_id: None,
                        target_title: None,
                        status: "failed",
                        summary,
                        error: Some(api_error.message().to_string()),
                        related: json!({ "qb_server_count": cfg.qb_servers.len() }),
                    },
                ),
            )
            .await;
            return Err(api_error);
        }
    };
    let cfg = &snapshot.value;
    write_operation_log(
        &state,
        operation_log_entry(
            "system",
            OperationLogEvent {
                category: "configuration",
                action: "save_config",
                target_type: "config",
                target_id: None,
                target_title: None,
                status: "success",
                summary: "配置已保存",
                error: None,
                related: json!({
                    "qb_server_count": cfg.qb_servers.len(),
                    "subscription_category_count": cfg.subscription_categories.len(),
                    "torrent_rule_count": cfg.torrent_match_rules.len(),
                }),
            },
        ),
    )
    .await;
    let restart_required = config_restart_required(&before.value, cfg);
    let response = config_response(snapshot.revision, cfg, restart_required);
    let revision = HeaderValue::from_str(&snapshot.revision.to_string())
        .map_err(|error| ApiError::internal(format!("生成配置 revision 响应头失败: {error}")))?;
    let mut headers = HeaderMap::new();
    headers.insert(HeaderName::from_static("x-config-revision"), revision);
    Ok((headers, Json(response)))
}

fn config_response(revision: u64, cfg: &FileConfig, restart_required: bool) -> ConfigResponse {
    ConfigResponse {
        revision,
        listen_ip: cfg.listen_ip.clone(),
        listen_port: cfg.listen_port,
        has_tmdb_api_key: !cfg.tmdb_api_key.trim().is_empty(),
        has_mteam_api_key: !cfg.mteam_api_key.trim().is_empty(),
        has_douban_cookie: !cfg.douban_cookie.trim().is_empty(),
        has_admin_token: !cfg.management.admin_token.trim().is_empty(),
        qb_servers: cfg
            .qb_servers
            .iter()
            .map(|server| RedactedQbServerResponse {
                id: server.id.clone(),
                name: server.name.clone(),
                base_url: server.base_url.clone(),
                username: server.username.clone(),
                insecure_tls: server.insecure_tls,
                has_password: !server.password.is_empty(),
            })
            .collect(),
        subscription_categories: cfg.subscription_categories.clone(),
        subscription_watcher: cfg.subscription_watcher.clone(),
        torrent_match_rules: cfg.torrent_match_rules.clone(),
        allowed_origins: cfg.management.allowed_origins.clone(),
        secure_cookie: cfg.management.secure_cookie,
        restart_required,
    }
}

fn merge_config_patch(current: &FileConfig, patch: ConfigPatch) -> Result<FileConfig, String> {
    let ConfigPatch {
        listen_ip,
        listen_port,
        tmdb_api_key,
        clear_tmdb_api_key,
        mteam_api_key,
        clear_mteam_api_key,
        douban_cookie,
        clear_douban_cookie,
        admin_token,
        clear_admin_token,
        qb_servers,
        subscription_categories,
        subscription_watcher,
        torrent_match_rules,
        allowed_origins,
        secure_cookie,
    } = patch;
    let mut next = current.clone();
    if let Some(listen_ip) = listen_ip {
        next.listen_ip = listen_ip;
    }
    if let Some(listen_port) = listen_port {
        next.listen_port = listen_port;
    }
    next.tmdb_api_key = merge_secret_value(
        &current.tmdb_api_key,
        SecretPatch::from_fields(tmdb_api_key, clear_tmdb_api_key, "clear_tmdb_api_key")?,
        "clear_tmdb_api_key",
        str::to_string,
    )?;
    next.mteam_api_key = merge_secret_value(
        &current.mteam_api_key,
        SecretPatch::from_fields(mteam_api_key, clear_mteam_api_key, "clear_mteam_api_key")?,
        "clear_mteam_api_key",
        str::to_string,
    )?;
    next.douban_cookie = merge_secret_value(
        &current.douban_cookie,
        SecretPatch::from_fields(douban_cookie, clear_douban_cookie, "clear_douban_cookie")?,
        "clear_douban_cookie",
        douban::normalize_cookie_header,
    )?;
    next.management.admin_token = merge_secret_value(
        &current.management.admin_token,
        SecretPatch::from_fields(admin_token, clear_admin_token, "clear_admin_token")?,
        "clear_admin_token",
        str::to_string,
    )?;
    if let Some(qb_servers) = qb_servers {
        next.qb_servers = merge_qb_server_patches(&current.qb_servers, qb_servers)?;
    }
    if let Some(subscription_categories) = subscription_categories {
        next.subscription_categories = subscription_categories;
    }
    if let Some(subscription_watcher) = subscription_watcher {
        next.subscription_watcher = subscription_watcher;
    }
    if let Some(torrent_match_rules) = torrent_match_rules {
        next.torrent_match_rules = torrent_match_rules;
    }
    if let Some(allowed_origins) = allowed_origins {
        next.management.allowed_origins = allowed_origins;
    }
    if let Some(secure_cookie) = secure_cookie {
        next.management.secure_cookie = secure_cookie;
    }
    config::normalize_loaded_file_config(next).map_err(|error| error.to_string())
}

impl SecretPatch {
    fn from_fields(value: Option<String>, clear: bool, clear_field: &str) -> Result<Self, String> {
        match (value, clear) {
            (Some(_), true) => Err(format!("secret set 与 {clear_field}=true 不能同时提交")),
            (Some(value), false) if value.trim().is_empty() => Err(format!(
                "secret 不能为空；如需清空请显式提交 {clear_field}=true"
            )),
            (Some(value), false) => Ok(Self::Set(value)),
            (None, true) => Ok(Self::Clear),
            (None, false) => Ok(Self::Keep),
        }
    }
}

fn merge_secret_value(
    current: &str,
    patch: SecretPatch,
    clear_field: &str,
    normalize: impl FnOnce(&str) -> String,
) -> Result<String, String> {
    match patch {
        SecretPatch::Keep => Ok(current.to_string()),
        SecretPatch::Clear => Ok(String::new()),
        SecretPatch::Set(value) => {
            let normalized = normalize(&value);
            if normalized.trim().is_empty() {
                Err(format!(
                    "secret 规范化后为空；如需清空请显式提交 {clear_field}=true"
                ))
            } else {
                Ok(normalized)
            }
        }
    }
}

fn merge_qb_server_patches(
    current: &[QbServerEntry],
    patches: Vec<QbServerPatch>,
) -> Result<Vec<QbServerEntry>, String> {
    let mut password_patches = Vec::with_capacity(patches.len());
    let mut servers = Vec::with_capacity(patches.len());
    for patch in patches {
        let password_patch =
            SecretPatch::from_fields(patch.password, patch.clear_password, "clear_password")?;
        password_patches.push(password_patch);
        servers.push(QbServerEntry {
            id: patch.id,
            name: patch.name,
            base_url: patch.base_url,
            username: patch.username,
            password: String::new(),
            insecure_tls: patch.insecure_tls,
        });
    }
    let mut servers = config::normalize_qb_servers(servers).map_err(|error| error.to_string())?;
    for (server, password_patch) in servers.iter_mut().zip(password_patches) {
        let current_password = current
            .iter()
            .find(|existing| existing.id == server.id)
            .map(|existing| existing.password.as_str())
            .unwrap_or("");
        server.password = merge_secret_value(
            current_password,
            password_patch,
            "clear_password",
            str::to_string,
        )?;
    }
    Ok(servers)
}

fn config_restart_required(current: &FileConfig, next: &FileConfig) -> bool {
    current.listen_ip != next.listen_ip
        || current.listen_port != next.listen_port
        || current.management.allowed_origins != next.management.allowed_origins
}

fn require_automation_enable_confirmation(
    current: &FileConfig,
    requested: &FileConfig,
    confirmed: bool,
) -> Result<(), String> {
    if !current.subscription_watcher.enabled && requested.subscription_watcher.enabled && !confirmed
    {
        return Err("启用订阅自动化必须显式提交 confirm_enable_automation=true".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    const TMDB_SECRET: &str = "SECRET_MUST_NOT_LEAK_TMDB";
    const MTEAM_SECRET: &str = "SECRET_MUST_NOT_LEAK_MTEAM";
    const DOUBAN_SECRET: &str = "dbcl2=SECRET_MUST_NOT_LEAK_DOUBAN:token; ck=test";
    const ADMIN_SECRET: &str = "SECRET_MUST_NOT_LEAK_ADMIN_TOKEN_123456";
    const QB_SECRET: &str = "SECRET_MUST_NOT_LEAK_QB_PASSWORD";

    fn secret_config() -> FileConfig {
        FileConfig {
            tmdb_api_key: TMDB_SECRET.to_string(),
            mteam_api_key: MTEAM_SECRET.to_string(),
            douban_cookie: DOUBAN_SECRET.to_string(),
            qb_servers: vec![QbServerEntry {
                id: "nas".to_string(),
                name: "NAS".to_string(),
                base_url: "http://127.0.0.1:8080".to_string(),
                username: "admin".to_string(),
                password: QB_SECRET.to_string(),
                insecure_tls: false,
            }],
            management: config::ManagementConfig {
                admin_token: ADMIN_SECRET.to_string(),
                allowed_origins: vec!["https://admin.example".to_string()],
                secure_cookie: true,
            },
            ..FileConfig::default()
        }
    }

    #[test]
    fn config_response_never_serializes_any_secret_value() {
        let response = config_response(7, &secret_config(), false);
        let serialized = serde_json::to_string(&response).expect("serialize config response");

        for secret in [
            TMDB_SECRET,
            MTEAM_SECRET,
            DOUBAN_SECRET,
            ADMIN_SECRET,
            QB_SECRET,
        ] {
            assert!(!serialized.contains(secret), "secret leaked: {secret}");
        }
    }

    #[test]
    fn config_response_uses_has_flags_and_redacted_qb_servers() {
        let response = config_response(7, &secret_config(), false);
        let value = serde_json::to_value(response).expect("serialize config response value");

        assert_eq!(value["revision"], 7);
        assert_eq!(value["has_tmdb_api_key"], true);
        assert_eq!(value["has_mteam_api_key"], true);
        assert_eq!(value["has_douban_cookie"], true);
        assert_eq!(value["has_admin_token"], true);
        assert_eq!(value["qb_servers"][0]["has_password"], true);
        assert_eq!(value["subscription_watcher"]["enabled"], false);
        assert_eq!(value["subscription_watcher"]["dry_run"], true);
        assert!(value["qb_servers"][0].get("password").is_none());
        assert!(value.get("tmdb_api_key").is_none());
        assert!(value.get("mteam_api_key").is_none());
        assert!(value.get("douban_cookie").is_none());
        assert!(value.get("admin_token").is_none());
    }

    #[test]
    fn enabling_automation_requires_explicit_confirmation() {
        let current = FileConfig::default();
        let mut enabled = current.clone();
        enabled.subscription_watcher.enabled = true;

        let error = require_automation_enable_confirmation(&current, &enabled, false)
            .expect_err("false to true must require confirmation");
        assert!(error.contains("confirm_enable_automation=true"));
        require_automation_enable_confirmation(&current, &enabled, true)
            .expect("explicit confirmation should allow enabling");

        let mut currently_enabled = current.clone();
        currently_enabled.subscription_watcher.enabled = true;
        let mut disabled = currently_enabled.clone();
        disabled.subscription_watcher.enabled = false;
        require_automation_enable_confirmation(&currently_enabled, &disabled, false)
            .expect("disabling must not require confirmation");
        require_automation_enable_confirmation(&current, &current, false)
            .expect("unrelated updates must not require confirmation");
    }

    #[test]
    fn config_patch_omitted_secrets_are_kept() {
        let current = secret_config();
        let merged = merge_config_patch(
            &current,
            ConfigPatch {
                listen_port: Some(9898),
                ..ConfigPatch::default()
            },
        )
        .expect("merge patch with omitted secrets");

        assert_eq!(merged.tmdb_api_key, TMDB_SECRET);
        assert_eq!(merged.mteam_api_key, MTEAM_SECRET);
        assert_eq!(merged.douban_cookie, DOUBAN_SECRET);
        assert_eq!(merged.management.admin_token, ADMIN_SECRET);
        assert_eq!(merged.qb_servers[0].password, QB_SECRET);
    }

    #[test]
    fn config_patch_clear_secret_is_explicit() {
        let current = secret_config();
        let merged = merge_config_patch(
            &current,
            ConfigPatch {
                clear_tmdb_api_key: true,
                clear_mteam_api_key: true,
                clear_douban_cookie: true,
                clear_admin_token: true,
                ..ConfigPatch::default()
            },
        )
        .expect("merge explicit secret clears");

        assert!(merged.tmdb_api_key.is_empty());
        assert!(merged.mteam_api_key.is_empty());
        assert!(merged.douban_cookie.is_empty());
        assert!(merged.management.admin_token.is_empty());

        let error = merge_config_patch(
            &current,
            ConfigPatch {
                tmdb_api_key: Some(String::new()),
                ..ConfigPatch::default()
            },
        )
        .expect_err("empty set must not silently clear a secret");
        assert!(error.contains("clear_tmdb_api_key"));
    }

    #[test]
    fn config_patch_preserves_existing_qb_password_by_id() {
        let current = secret_config();
        let merged = merge_config_patch(
            &current,
            ConfigPatch {
                qb_servers: Some(vec![
                    QbServerPatch {
                        id: "nas".to_string(),
                        name: "Renamed NAS".to_string(),
                        base_url: "http://127.0.0.1:9090".to_string(),
                        username: "operator".to_string(),
                        insecure_tls: true,
                        password: None,
                        clear_password: false,
                    },
                    QbServerPatch {
                        id: "ssd".to_string(),
                        name: "New SSD".to_string(),
                        base_url: "http://127.0.0.1:8081".to_string(),
                        username: "operator".to_string(),
                        insecure_tls: false,
                        password: None,
                        clear_password: false,
                    },
                ]),
                ..ConfigPatch::default()
            },
        )
        .expect("merge qB server patch");

        assert_eq!(merged.qb_servers[0].id, "nas");
        assert_eq!(merged.qb_servers[0].password, QB_SECRET);
        assert_eq!(merged.qb_servers[0].name, "Renamed NAS");
        assert_eq!(merged.qb_servers[1].id, "ssd");
        assert!(merged.qb_servers[1].password.is_empty());

        let cleared = merge_config_patch(
            &current,
            ConfigPatch {
                qb_servers: Some(vec![QbServerPatch {
                    id: "nas".to_string(),
                    name: "NAS".to_string(),
                    base_url: "http://127.0.0.1:8080".to_string(),
                    username: "admin".to_string(),
                    insecure_tls: false,
                    password: None,
                    clear_password: true,
                }]),
                ..ConfigPatch::default()
            },
        )
        .expect("merge explicit qB password clear");
        assert!(cleared.qb_servers[0].password.is_empty());
    }

    #[test]
    fn config_patch_rejects_qb_url_userinfo() {
        let error = merge_config_patch(
            &FileConfig::default(),
            ConfigPatch {
                qb_servers: Some(vec![QbServerPatch {
                    id: "nas".to_string(),
                    name: "NAS".to_string(),
                    base_url: "http://user:SECRET_MUST_NOT_LEAK@127.0.0.1:8080".to_string(),
                    username: "admin".to_string(),
                    insecure_tls: false,
                    password: None,
                    clear_password: false,
                }]),
                ..ConfigPatch::default()
            },
        )
        .expect_err("qB URL userinfo must be rejected");

        assert!(error.contains("userinfo"));
        assert!(!error.contains("SECRET_MUST_NOT_LEAK"));
    }

    #[test]
    fn config_patch_revalidates_existing_categories_when_only_qb_servers_change() {
        let mut current = secret_config();
        current.subscription_categories = vec![SubscriptionCategory {
            name: "电影".to_string(),
            wanted_tag: "电影".to_string(),
            qb_server_id: "nas".to_string(),
            qb_category: "movie".to_string(),
            qb_save_dir_name: "movies".to_string(),
            download_dir: "/downloads/movies".to_string(),
            link_target_dir: "/media/movies".to_string(),
        }];

        let error = merge_config_patch(
            &current,
            ConfigPatch {
                qb_servers: Some(vec![QbServerPatch {
                    id: "ssd".to_string(),
                    name: "SSD".to_string(),
                    base_url: "https://qb.example".to_string(),
                    username: "operator".to_string(),
                    insecure_tls: false,
                    password: None,
                    clear_password: false,
                }]),
                ..ConfigPatch::default()
            },
        )
        .expect_err("qB-only patch must reject an orphaned category reference");

        assert!(error.contains("qB 服务器不存在"));
        assert!(error.contains("nas"));
    }

    #[tokio::test]
    async fn config_patch_rejects_stale_revision_without_writing() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-config-patch-stale-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create stale patch test directory");
        let path = root.join("config.toml");
        let original = secret_config();
        original
            .save(&path)
            .expect("write stale patch source config");
        let manager = config::ConfigManager::new(path.clone(), original);

        manager
            .update(Some(1), |cfg| {
                *cfg = merge_config_patch(
                    cfg,
                    ConfigPatch {
                        listen_port: Some(9898),
                        ..ConfigPatch::default()
                    },
                )?;
                Ok(())
            })
            .await
            .expect("commit first config patch");
        let committed_file = fs::read(&path).expect("read committed config patch");

        let error = manager
            .update(Some(1), |cfg| {
                *cfg = merge_config_patch(
                    cfg,
                    ConfigPatch {
                        tmdb_api_key: Some("SECRET_MUST_NOT_COMMIT_STALE".to_string()),
                        ..ConfigPatch::default()
                    },
                )?;
                Ok(())
            })
            .await
            .expect_err("stale config patch must fail");

        assert!(matches!(
            error,
            ConfigUpdateError::Stale {
                expected: 1,
                actual: 2
            }
        ));
        assert_eq!(
            fs::read(&path).expect("read config after stale patch"),
            committed_file
        );
        assert_eq!(manager.snapshot().await.revision, 2);
        assert!(!manager
            .snapshot()
            .await
            .value
            .tmdb_api_key
            .contains("SECRET_MUST_NOT_COMMIT_STALE"));

        let _ = fs::remove_dir_all(root);
    }
}
