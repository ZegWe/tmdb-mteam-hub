use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

use crate::storage::blocking::BoundedBlockingExecutor;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QbServerEntry {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub base_url: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub insecure_tls: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionCategory {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub wanted_tag: String,
    #[serde(default)]
    pub qb_server_id: String,
    #[serde(default)]
    pub qb_category: String,
    #[serde(default)]
    pub qb_save_dir_name: String,
    #[serde(default)]
    pub download_dir: String,
    #[serde(default)]
    pub link_target_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscriptionWatcherConfig {
    #[serde(default = "default_subscription_watcher_enabled")]
    pub enabled: bool,
    #[serde(default = "default_subscription_watcher_dry_run")]
    pub dry_run: bool,
    #[serde(default = "default_subscription_poll_interval_secs")]
    pub poll_interval_secs: u64,
    #[serde(default = "default_subscription_library_limit")]
    pub library_limit: usize,
    #[serde(default = "default_subscription_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_subscription_search_interval_secs")]
    pub search_interval_secs: u64,
    #[serde(default = "default_subscription_progress_interval_secs")]
    pub progress_interval_secs: u64,
    #[serde(default = "default_subscription_link_retry_interval_secs")]
    pub link_retry_interval_secs: u64,
    #[serde(default = "default_subscription_system_retry_interval_secs")]
    pub system_retry_interval_secs: u64,
    #[serde(default = "default_subscription_bootstrap_existing_as_skipped")]
    pub bootstrap_existing_as_skipped: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementConfig {
    #[serde(default)]
    pub admin_token: String,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub secure_cookie: bool,
}

impl ManagementConfig {
    pub fn validate(&self) -> io::Result<()> {
        let token = self.admin_token.trim();
        if !token.is_empty() && token.chars().count() < 24 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "management.admin_token 至少需要 24 个字符",
            ));
        }
        for origin in &self.allowed_origins {
            validate_management_origin(origin)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TorrentRuleMatchMode {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorrentMatchRule {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub mode: TorrentRuleMatchMode,
    #[serde(default)]
    pub title_keywords: Vec<String>,
    #[serde(default)]
    pub resolution_keywords: Vec<String>,
    #[serde(default)]
    pub source_keywords: Vec<String>,
}

impl Default for SubscriptionWatcherConfig {
    fn default() -> Self {
        Self {
            enabled: default_subscription_watcher_enabled(),
            dry_run: default_subscription_watcher_dry_run(),
            poll_interval_secs: default_subscription_poll_interval_secs(),
            library_limit: default_subscription_library_limit(),
            max_retries: default_subscription_max_retries(),
            search_interval_secs: default_subscription_search_interval_secs(),
            progress_interval_secs: default_subscription_progress_interval_secs(),
            link_retry_interval_secs: default_subscription_link_retry_interval_secs(),
            system_retry_interval_secs: default_subscription_system_retry_interval_secs(),
            bootstrap_existing_as_skipped: default_subscription_bootstrap_existing_as_skipped(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    #[serde(default = "default_listen_ip")]
    pub listen_ip: String,
    #[serde(default = "default_listen_port")]
    pub listen_port: u16,
    #[serde(default)]
    pub tmdb_api_key: String,
    #[serde(default)]
    pub mteam_api_key: String,
    #[serde(default)]
    pub douban_cookie: String,
    #[serde(default)]
    pub qb_servers: Vec<QbServerEntry>,
    #[serde(default)]
    pub subscription_categories: Vec<SubscriptionCategory>,
    #[serde(default)]
    pub subscription_watcher: SubscriptionWatcherConfig,
    #[serde(default)]
    pub torrent_match_rules: Vec<TorrentMatchRule>,
    #[serde(default)]
    pub management: ManagementConfig,
}

fn default_listen_ip() -> String {
    "127.0.0.1".to_string()
}

fn default_listen_port() -> u16 {
    8787
}

fn default_subscription_watcher_enabled() -> bool {
    false
}

fn default_subscription_watcher_dry_run() -> bool {
    true
}

fn default_subscription_poll_interval_secs() -> u64 {
    3600
}

fn default_subscription_library_limit() -> usize {
    200
}

fn default_subscription_max_retries() -> u32 {
    3
}

fn default_subscription_search_interval_secs() -> u64 {
    1_800
}

fn default_subscription_progress_interval_secs() -> u64 {
    5
}

fn default_subscription_link_retry_interval_secs() -> u64 {
    900
}

fn default_subscription_system_retry_interval_secs() -> u64 {
    600
}

fn default_subscription_bootstrap_existing_as_skipped() -> bool {
    true
}

fn validate_management_origin(origin: &str) -> io::Result<()> {
    let raw = origin.trim();
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "management.allowed_origins 只允许不含 userinfo、path、query 或 fragment 的完整 http(s) origin",
        )
    };
    if raw != origin
        || raw.is_empty()
        || !raw.is_ascii()
        || raw == "*"
        || raw.eq_ignore_ascii_case("null")
        || raw.contains('*')
        || raw.ends_with('/')
    {
        return Err(invalid());
    }
    let parsed = reqwest::Url::parse(raw).map_err(|_| invalid())?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(())
}

impl Default for FileConfig {
    fn default() -> Self {
        Self {
            listen_ip: default_listen_ip(),
            listen_port: default_listen_port(),
            tmdb_api_key: String::new(),
            mteam_api_key: String::new(),
            douban_cookie: String::new(),
            qb_servers: Vec::new(),
            subscription_categories: Vec::new(),
            subscription_watcher: SubscriptionWatcherConfig::default(),
            torrent_match_rules: Vec::new(),
            management: ManagementConfig::default(),
        }
    }
}

impl FileConfig {
    pub fn listen_addr(&self) -> io::Result<SocketAddr> {
        let ip_raw = self.listen_ip.trim();
        let ip_raw = if ip_raw.is_empty() {
            default_listen_ip()
        } else {
            ip_raw.to_string()
        };
        let ip: IpAddr = ip_raw.parse().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("listen_ip 必须是合法 IP 地址: {e}"),
            )
        })?;
        Ok(SocketAddr::new(ip, self.listen_port))
    }

    pub fn load_or_create(path: &Path) -> io::Result<Self> {
        if path.exists() {
            let raw = fs::read_to_string(path).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("读取配置文件 {} 失败: {error}", path.display()),
                )
            })?;
            let cfg: FileConfig = toml::from_str(&raw).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("解析配置文件 {} 失败: {error}", path.display()),
                )
            })?;
            cfg.validate()?;
            Ok(cfg)
        } else {
            let cfg = FileConfig::default();
            cfg.save(path)?;
            Ok(cfg)
        }
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        self.validate()?;
        let raw = serialize_config(self)?;
        atomic_write_secret(path, raw.as_bytes())
    }

    pub fn persist_normalized_if_changed(
        path: &Path,
        original: &Self,
        normalized: &Self,
    ) -> io::Result<Option<PathBuf>> {
        if original == normalized {
            return Ok(None);
        }

        normalized.validate()?;
        let raw = serialize_config(normalized)?;
        let backup_path = create_timestamped_backup(path)?;
        atomic_write_secret(path, raw.as_bytes()).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "写入规范化配置 {} 失败（原配置备份保留在 {}）: {error}",
                    path.display(),
                    backup_path.display()
                ),
            )
        })?;
        Ok(Some(backup_path))
    }

    fn validate(&self) -> io::Result<()> {
        let listen_addr = self.listen_addr()?;
        self.management.validate()?;
        if !listen_addr.ip().is_loopback() && self.management.admin_token.trim().is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "非 loopback 监听必须设置 management.admin_token；请改用 127.0.0.1，或配置至少 24 字符的高熵 admin_token 后再开放监听",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigValidationError {
    message: String,
}

impl ConfigValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConfigValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for ConfigValidationError {}

pub(crate) fn normalize_qb_servers(
    servers: Vec<QbServerEntry>,
) -> Result<Vec<QbServerEntry>, ConfigValidationError> {
    let mut used = HashSet::new();
    let mut out = Vec::new();

    for server in servers {
        let id = unique_qb_server_id(&server, &mut used);
        let base_url = server.base_url.trim().to_string();
        validate_qb_base_url(&base_url)?;
        let normalized = QbServerEntry {
            id,
            name: server.name.trim().to_string(),
            base_url,
            username: server.username.trim().to_string(),
            password: server.password,
            insecure_tls: server.insecure_tls,
        };
        out.push(normalized);
    }

    Ok(out)
}

fn validate_qb_base_url(base_url: &str) -> Result<(), ConfigValidationError> {
    if base_url.is_empty() {
        return Err(ConfigValidationError::new("qB base_url 不能为空"));
    }
    let url = reqwest::Url::parse(base_url)
        .map_err(|_| ConfigValidationError::new("qB base_url 必须是合法的 http/https URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(ConfigValidationError::new(
            "qB base_url 必须是合法的 http/https URL",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ConfigValidationError::new(
            "qB base_url 禁止包含 userinfo，请使用独立 username/password 字段",
        ));
    }
    Ok(())
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

pub(crate) fn normalize_subscription_categories(
    categories: Vec<SubscriptionCategory>,
    qb_servers: &[QbServerEntry],
) -> Result<Vec<SubscriptionCategory>, ConfigValidationError> {
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
                ConfigValidationError::new(format!(
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
            return Err(ConfigValidationError::new(format!(
                "订阅分类 #{n} 缺少分类名"
            )));
        }
        if normalized.wanted_tag.is_empty() {
            return Err(ConfigValidationError::new(format!(
                "订阅分类 {} 缺少想看标签文本",
                normalized.name
            )));
        }
        if normalized.wanted_tag.split_whitespace().count() != 1 {
            return Err(ConfigValidationError::new(format!(
                "订阅分类 {} 的想看标签不能包含空白字符",
                normalized.name
            )));
        }
        if normalized.qb_category.is_empty() {
            return Err(ConfigValidationError::new(format!(
                "订阅分类 {} 缺少 qB 下载分类",
                normalized.name
            )));
        }
        if normalized.qb_save_dir_name.is_empty() {
            return Err(ConfigValidationError::new(format!(
                "订阅分类 {} 缺少 qB 保存目录名",
                normalized.name
            )));
        }
        if !qb_server_ids.contains(&normalized.qb_server_id) {
            return Err(ConfigValidationError::new(format!(
                "订阅分类 {} 绑定的 qB 服务器不存在: {}",
                normalized.name, normalized.qb_server_id
            )));
        }
        if normalized.download_dir.is_empty() {
            return Err(ConfigValidationError::new(format!(
                "订阅分类 {} 缺少真实下载目录",
                normalized.name
            )));
        }
        if normalized.link_target_dir.is_empty() {
            return Err(ConfigValidationError::new(format!(
                "订阅分类 {} 缺少硬链接目标目录",
                normalized.name
            )));
        }
        if !names.insert(normalized.name.clone()) {
            return Err(ConfigValidationError::new(format!(
                "订阅分类名重复: {}",
                normalized.name
            )));
        }
        if !wanted_tags.insert(normalized.wanted_tag.clone()) {
            return Err(ConfigValidationError::new(format!(
                "想看标签文本重复: {}",
                normalized.wanted_tag
            )));
        }
        out.push(normalized);
    }

    Ok(out)
}

pub(crate) fn normalize_subscription_watcher(
    mut cfg: SubscriptionWatcherConfig,
) -> SubscriptionWatcherConfig {
    cfg.poll_interval_secs = cfg.poll_interval_secs.clamp(60, 86_400);
    cfg.library_limit = cfg.library_limit.clamp(1, 1200);
    cfg.max_retries = cfg.max_retries.clamp(1, 20);
    cfg.search_interval_secs = cfg.search_interval_secs.max(30);
    cfg.progress_interval_secs = cfg.progress_interval_secs.max(1);
    cfg.link_retry_interval_secs = cfg.link_retry_interval_secs.max(30);
    cfg.system_retry_interval_secs = cfg.system_retry_interval_secs.max(30);
    cfg
}

pub(crate) fn normalize_torrent_match_rules(
    rules: Vec<TorrentMatchRule>,
) -> Result<Vec<TorrentMatchRule>, ConfigValidationError> {
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
            return Err(ConfigValidationError::new(format!(
                "种子匹配规则 #{n} 缺少规则名"
            )));
        }
        if !names.insert(normalized.name.clone()) {
            return Err(ConfigValidationError::new(format!(
                "种子匹配规则名重复: {}",
                normalized.name
            )));
        }
        if normalized.title_keywords.is_empty()
            && normalized.resolution_keywords.is_empty()
            && normalized.source_keywords.is_empty()
        {
            return Err(ConfigValidationError::new(format!(
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

pub(crate) fn normalize_loaded_file_config(
    mut cfg: FileConfig,
) -> Result<FileConfig, ConfigValidationError> {
    cfg.qb_servers = normalize_qb_servers(cfg.qb_servers)?;
    cfg.subscription_categories =
        normalize_subscription_categories(cfg.subscription_categories, &cfg.qb_servers)?;
    cfg.subscription_watcher = normalize_subscription_watcher(cfg.subscription_watcher);
    cfg.torrent_match_rules = normalize_torrent_match_rules(cfg.torrent_match_rules)?;
    cfg.validate()
        .map_err(|error| ConfigValidationError::new(error.to_string()))?;
    Ok(cfg)
}

pub(crate) fn load_normalized_file_config(path: &Path) -> io::Result<FileConfig> {
    let loaded = FileConfig::load_or_create(path)?;
    let normalized = normalize_loaded_file_config(loaded.clone()).map_err(invalid_config_error)?;
    if let Some(backup_path) =
        FileConfig::persist_normalized_if_changed(path, &loaded, &normalized)?
    {
        tracing::info!(
            backup_path = %backup_path.display(),
            "normalized config persisted after validated backup"
        );
    }
    Ok(normalized)
}

fn invalid_config_error(error: ConfigValidationError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("配置校验失败: {error}"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigSnapshot {
    pub revision: u64,
    pub value: FileConfig,
}

#[derive(Debug)]
pub enum ConfigUpdateError {
    Stale { expected: u64, actual: u64 },
    Mutation(String),
    Persist(io::Error),
}

impl std::fmt::Display for ConfigUpdateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stale { expected, actual } => write!(
                formatter,
                "配置 revision 已过期: expected={expected}, current={actual}"
            ),
            Self::Mutation(message) => message.fmt(formatter),
            Self::Persist(error) => write!(formatter, "写入配置失败: {error}"),
        }
    }
}

impl std::error::Error for ConfigUpdateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Persist(error) => Some(error),
            Self::Stale { .. } | Self::Mutation(_) => None,
        }
    }
}

type ConfigPersist = dyn Fn(&Path, &FileConfig) -> io::Result<()> + Send + Sync;

#[derive(Clone)]
pub struct ConfigManager {
    path: PathBuf,
    state: Arc<Mutex<ConfigSnapshot>>,
    update_gate: Arc<Mutex<()>>,
    persist: Arc<ConfigPersist>,
    blocking: BoundedBlockingExecutor,
}

impl ConfigManager {
    pub fn new(path: PathBuf, value: FileConfig) -> Self {
        Self::with_persist(path, value, |path, config| config.save(path))
    }

    fn with_persist<F>(path: PathBuf, value: FileConfig, persist: F) -> Self
    where
        F: Fn(&Path, &FileConfig) -> io::Result<()> + Send + Sync + 'static,
    {
        Self {
            path,
            state: Arc::new(Mutex::new(ConfigSnapshot { revision: 1, value })),
            update_gate: Arc::new(Mutex::new(())),
            persist: Arc::new(persist),
            blocking: BoundedBlockingExecutor::try_new("config", 1)
                .expect("static config blocking concurrency must be valid"),
        }
    }

    pub async fn snapshot(&self) -> ConfigSnapshot {
        self.state.lock().await.clone()
    }

    pub async fn update<F>(
        &self,
        expected_revision: Option<u64>,
        mutate: F,
    ) -> Result<ConfigSnapshot, ConfigUpdateError>
    where
        F: FnOnce(&mut FileConfig) -> Result<(), String> + Send,
    {
        let _update = self.update_gate.lock().await;
        let current = self.state.lock().await.clone();
        if let Some(expected) = expected_revision {
            if expected != current.revision {
                return Err(ConfigUpdateError::Stale {
                    expected,
                    actual: current.revision,
                });
            }
        }

        let mut next_value = current.value.clone();
        mutate(&mut next_value).map_err(ConfigUpdateError::Mutation)?;
        next_value
            .validate()
            .map_err(|error| ConfigUpdateError::Mutation(error.to_string()))?;
        if next_value == current.value {
            return Ok(current);
        }

        let persist = self.persist.clone();
        let path = self.path.clone();
        let persisted_value = next_value.clone();
        self.blocking
            .run(move || persist(&path, &persisted_value))
            .await
            .map_err(|error| ConfigUpdateError::Persist(io::Error::other(error.to_string())))?
            .map_err(ConfigUpdateError::Persist)?;
        let next = ConfigSnapshot {
            revision: current.revision.saturating_add(1),
            value: next_value,
        };
        let mut state = self.state.lock().await;
        if state.revision != current.revision {
            return Err(ConfigUpdateError::Stale {
                expected: current.revision,
                actual: state.revision,
            });
        }
        *state = next.clone();
        Ok(next)
    }

    pub async fn patch_douban_cookie(
        &self,
        cookie: String,
    ) -> Result<ConfigSnapshot, ConfigUpdateError> {
        self.update(None, move |config| {
            config.douban_cookie = cookie;
            Ok(())
        })
        .await
    }
}

fn serialize_config(config: &FileConfig) -> io::Result<String> {
    toml::to_string_pretty(config)
        .map_err(|error| io::Error::other(format!("序列化配置失败: {error}")))
}

fn atomic_write_secret(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_secret_with_rename(path, bytes, |from, to| fs::rename(from, to))
}

fn atomic_write_secret_with_rename<F>(path: &Path, bytes: &[u8], rename: F) -> io::Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    atomic_write_secret_with_ops(path, bytes, rename, sync_parent_dir)
}

fn atomic_write_secret_with_ops<R, S>(
    path: &Path,
    bytes: &[u8],
    rename: R,
    sync_parent: S,
) -> io::Result<()>
where
    R: FnOnce(&Path, &Path) -> io::Result<()>,
    S: FnOnce(&Path) -> io::Result<()>,
{
    let parent = non_empty_parent(path);
    fs::create_dir_all(parent).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("创建配置目录 {} 失败: {error}", parent.display()),
        )
    })?;

    let (tmp_path, mut tmp_file) = create_secret_temp_file(path)?;
    let write_result = (|| -> io::Result<()> {
        tmp_file.write_all(bytes)?;
        tmp_file.sync_all()?;
        drop(tmp_file);
        // The temporary file already has its final restrictive permissions. Rename is the
        // commit point: after it succeeds, do not turn the committed write into an error.
        rename(&tmp_path, path)?;
        Ok(())
    })();

    if let Err(error) = write_result {
        let _ = fs::remove_file(&tmp_path);
        return Err(io::Error::new(
            error.kind(),
            format!("原子写入配置文件 {} 失败: {error}", path.display()),
        ));
    }
    if let Err(error) = sync_parent(parent) {
        tracing::warn!(
            path = %path.display(),
            "config rename committed but parent directory sync failed: {error}"
        );
    }
    Ok(())
}

fn create_secret_temp_file(path: &Path) -> io::Result<(PathBuf, File)> {
    let parent = non_empty_parent(path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config.toml");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for attempt in 0..100_u32 {
        let tmp_path = parent.join(format!(
            ".{file_name}.tmp.{}.{nonce}.{attempt}",
            std::process::id()
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&tmp_path) {
            Ok(file) => {
                if let Err(error) = set_secret_permissions(&tmp_path) {
                    drop(file);
                    let _ = fs::remove_file(&tmp_path);
                    return Err(error);
                }
                return Ok((tmp_path, file));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!("创建配置临时文件 {} 失败: {error}", tmp_path.display()),
                ));
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("无法为配置文件 {} 创建唯一临时文件", path.display()),
    ))
}

fn create_timestamped_backup(path: &Path) -> io::Result<PathBuf> {
    let bytes = fs::read(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("读取待备份配置 {} 失败: {error}", path.display()),
        )
    })?;
    let parent = non_empty_parent(path);
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config.toml");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    for suffix in 0..100_u32 {
        let backup_name = if suffix == 0 {
            format!("{file_name}.bak.{timestamp}")
        } else {
            format!("{file_name}.bak.{timestamp}.{suffix}")
        };
        let backup_path = parent.join(backup_name);
        match write_new_secret_file(&backup_path, &bytes) {
            Ok(()) => {
                let persisted = fs::read(&backup_path)?;
                if persisted != bytes {
                    let _ = fs::remove_file(&backup_path);
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("配置备份校验失败: {}", backup_path.display()),
                    ));
                }
                sync_parent_dir(parent)?;
                return Ok(backup_path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("无法为配置文件 {} 创建唯一备份", path.display()),
    ))
}

fn write_new_secret_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(error) = (|| -> io::Result<()> {
        set_secret_permissions(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })() {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

fn non_empty_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(unix)]
fn set_secret_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_secret_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        atomic_write_secret_with_ops, atomic_write_secret_with_rename, load_normalized_file_config,
        normalize_qb_servers, normalize_subscription_categories, normalize_subscription_watcher,
        normalize_torrent_match_rules, ConfigManager, ConfigUpdateError, FileConfig, QbServerEntry,
        SubscriptionCategory, SubscriptionWatcherConfig, TorrentMatchRule, TorrentRuleMatchMode,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_dir(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "tmdb-mteam-config-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create config test directory");
        path
    }

    fn write_fixture(path: &Path, fixture: &str) {
        fs::write(path, fixture).expect("write config fixture");
    }

    fn backup_count(root: &Path) -> usize {
        fs::read_dir(root)
            .expect("list config directory")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("config.toml.bak."))
            })
            .count()
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

    #[test]
    fn qb_servers_get_stable_unique_ids() {
        let servers = normalize_qb_servers(vec![
            qb_server("", "NAS", "http://127.0.0.1:8080"),
            qb_server("", "NAS", "http://127.0.0.1:8081"),
            qb_server("custom-id", "下载机", "https://qb.example"),
        ])
        .expect("qB servers should normalize");

        assert_eq!(servers[0].id, "nas");
        assert_eq!(servers[1].id, "nas-2");
        assert_eq!(servers[2].id, "custom-id");
    }

    #[test]
    fn qb_base_url_requires_http_or_https_without_userinfo() {
        for (label, base_url, expected) in [
            ("empty", "", "不能为空"),
            ("malformed", "://not-a-url", "http/https"),
            ("unsupported-scheme", "ftp://qb.example", "http/https"),
            (
                "userinfo",
                "http://user:SECRET_MUST_NOT_LEAK@127.0.0.1:8080",
                "userinfo",
            ),
        ] {
            let error =
                normalize_qb_servers(vec![qb_server("nas", "NAS", base_url)]).expect_err(label);
            let message = error.to_string();
            assert!(message.contains(expected), "{label}: {message}");
            assert!(!message.contains("SECRET_MUST_NOT_LEAK"));
        }
    }

    #[test]
    fn subscription_categories_bind_to_existing_qb_server_ids() {
        let servers = vec![qb_server("nas", "NAS", "http://127.0.0.1:8080")];
        let mut movie = category("电影", "电影");
        let normalized = normalize_subscription_categories(vec![movie.clone()], &servers)
            .expect("category should bind to first qB server");
        assert_eq!(normalized[0].qb_server_id, "nas");

        movie.qb_server_id = "missing".to_string();
        let error = normalize_subscription_categories(vec![movie], &servers)
            .expect_err("missing qB server reference must fail");
        assert!(error.to_string().contains("不存在"));
    }

    #[test]
    fn subscription_categories_reject_duplicate_wanted_tags() {
        let servers = vec![qb_server("nas", "NAS", "http://127.0.0.1:8080")];
        let error = normalize_subscription_categories(
            vec![category("电影", "影视"), category("剧集", "影视")],
            &servers,
        )
        .expect_err("duplicate wanted tags must fail");

        assert!(error.to_string().contains("想看标签文本重复"));
    }

    #[test]
    fn watcher_and_torrent_rules_normalize_in_config_core() {
        let watcher = normalize_subscription_watcher(SubscriptionWatcherConfig {
            poll_interval_secs: 1,
            library_limit: 0,
            max_retries: 99,
            search_interval_secs: 1,
            progress_interval_secs: 0,
            link_retry_interval_secs: 1,
            system_retry_interval_secs: 1,
            ..SubscriptionWatcherConfig::default()
        });
        assert_eq!(watcher.poll_interval_secs, 60);
        assert_eq!(watcher.library_limit, 1);
        assert_eq!(watcher.max_retries, 20);
        assert_eq!(watcher.search_interval_secs, 30);
        assert_eq!(watcher.progress_interval_secs, 1);
        assert_eq!(watcher.link_retry_interval_secs, 30);
        assert_eq!(watcher.system_retry_interval_secs, 30);

        let rules = normalize_torrent_match_rules(vec![
            TorrentMatchRule {
                name: " lower ".to_string(),
                priority: 1,
                mode: TorrentRuleMatchMode::All,
                title_keywords: vec![" WEB ".to_string(), "WEB".to_string(), String::new()],
                resolution_keywords: Vec::new(),
                source_keywords: Vec::new(),
            },
            TorrentMatchRule {
                name: "higher".to_string(),
                priority: 2,
                mode: TorrentRuleMatchMode::Any,
                title_keywords: Vec::new(),
                resolution_keywords: vec!["2160p".to_string()],
                source_keywords: Vec::new(),
            },
        ])
        .expect("valid torrent rules should normalize");
        assert_eq!(rules[0].name, "higher");
        assert_eq!(rules[1].name, "lower");
        assert_eq!(rules[1].title_keywords, ["WEB"]);
    }

    #[test]
    fn startup_normalizes_valid_legacy_config_once_with_backup() {
        let root = temp_test_dir("startup-legacy");
        let path = root.join("config.toml");
        fs::write(
            &path,
            include_str!("../tests/fixtures/config/valid-pre-safety.toml"),
        )
        .expect("write legacy config fixture");

        let first = load_normalized_file_config(&path).expect("normalize valid legacy config");

        assert_eq!(first.qb_servers[0].id, "legacy-nas");
        assert_eq!(backup_count(&root), 1);
        let first_bytes = fs::read(&path).expect("read first normalized config");

        let second = load_normalized_file_config(&path).expect("reload normalized config");

        assert_eq!(second, first);
        assert_eq!(backup_count(&root), 1);
        assert_eq!(
            fs::read(&path).expect("read second normalized config"),
            first_bytes
        );
    }

    #[test]
    fn startup_validation_failure_does_not_write_or_backup() {
        let root = temp_test_dir("startup-invalid-normalization");
        let path = root.join("config.toml");
        let raw = concat!(
            "listen_ip = \"127.0.0.1\"\n",
            "[[subscription_categories]]\n",
            "name = \"电影\"\n",
            "wanted_tag = \"电影\"\n",
            "qb_category = \"movie\"\n",
            "qb_save_dir_name = \"movies\"\n",
            "download_dir = \"/downloads/movies\"\n",
            "link_target_dir = \"/media/movies\"\n",
        );
        fs::write(&path, raw).expect("write invalid normalized config");
        let before = fs::read(&path).expect("read config before validation");

        let error = load_normalized_file_config(&path)
            .expect_err("category without qB server must fail validation");

        assert!(error.to_string().contains("配置校验失败"));
        assert_eq!(
            fs::read(&path).expect("read config after validation"),
            before
        );
        assert_eq!(backup_count(&root), 0);
    }

    #[test]
    fn default_listen_addr_is_loopback() {
        let addr = FileConfig::default()
            .listen_addr()
            .expect("default listen address should parse");

        assert_eq!(addr.to_string(), "127.0.0.1:8787");
    }

    #[test]
    fn checked_in_config_example_parses_and_validates() {
        let config: FileConfig = toml::from_str(include_str!("../config.example.toml"))
            .expect("config.example.toml should parse as FileConfig");

        config
            .validate()
            .expect("config.example.toml should satisfy configuration validation");
    }

    #[test]
    fn subscription_watcher_defaults_include_lane_intervals() {
        let cfg = SubscriptionWatcherConfig::default();

        assert_eq!(cfg.search_interval_secs, 1_800);
        assert_eq!(cfg.progress_interval_secs, 5);
        assert_eq!(cfg.link_retry_interval_secs, 900);
        assert_eq!(cfg.system_retry_interval_secs, 600);
    }

    #[test]
    fn subscription_watcher_defaults_to_disabled_dry_run() {
        let cfg = SubscriptionWatcherConfig::default();

        assert!(!cfg.enabled);
        assert!(cfg.dry_run);
    }

    #[test]
    fn legacy_subscription_watcher_toml_uses_safe_runtime_defaults() {
        let config: FileConfig = toml::from_str(concat!(
            "listen_ip = \"127.0.0.1\"\n",
            "[subscription_watcher]\n",
            "poll_interval_secs = 7200\n",
            "library_limit = 50\n",
        ))
        .expect("legacy watcher config without safety flags should parse");

        assert!(!config.subscription_watcher.enabled);
        assert!(config.subscription_watcher.dry_run);
        assert_eq!(config.subscription_watcher.poll_interval_secs, 7_200);
        assert_eq!(config.subscription_watcher.library_limit, 50);
    }

    #[test]
    fn config_store_reports_malformed_toml_and_never_replaces_source() {
        let root = temp_test_dir("malformed");
        let path = root.join("config.toml");
        let raw = include_str!("../tests/fixtures/config/malformed.toml");
        write_fixture(&path, raw);
        let before = fs::read(&path).expect("read malformed config before load");
        let before_modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .expect("read malformed config mtime before load");
        #[cfg(unix)]
        let before_inode = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&path).expect("metadata before load").ino()
        };

        let error = FileConfig::load_or_create(&path).expect_err("malformed TOML must fail");
        let message = error.to_string();

        assert!(message.contains(&path.display().to_string()));
        assert!(message.contains("line"));
        assert!(message.contains("column"));
        assert_eq!(
            fs::read(&path).expect("read malformed config after load"),
            before
        );
        assert_eq!(
            fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .expect("read malformed config mtime after load"),
            before_modified
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(&path).expect("metadata after load").ino(),
                before_inode
            );
        }
    }

    #[test]
    fn config_store_rejects_unknown_top_level_and_nested_fields() {
        for (label, raw) in [
            (
                "top-level",
                include_str!("../tests/fixtures/config/unknown-field.toml"),
            ),
            (
                "nested",
                concat!(
                    "listen_ip = \"127.0.0.1\"\n",
                    "[[qb_servers]]\n",
                    "name = \"nas\"\n",
                    "base_url = \"http://127.0.0.1:8080\"\n",
                    "username = \"admin\"\n",
                    "password = \"test-only\"\n",
                    "unexpected_nested = true\n",
                ),
            ),
        ] {
            let root = temp_test_dir(label);
            let path = root.join("config.toml");
            write_fixture(&path, raw);
            let before = fs::read(&path).expect("read unknown-field config before load");

            let error = FileConfig::load_or_create(&path)
                .expect_err("unknown config fields must be rejected");

            assert!(error.to_string().contains("unknown field"));
            assert_eq!(
                fs::read(&path).expect("read unknown-field config after load"),
                before
            );
        }
    }

    #[test]
    fn config_store_loads_valid_pre_safety_config_without_rewriting_it() {
        let root = temp_test_dir("valid-pre-safety");
        let path = root.join("config.toml");
        let raw = include_str!("../tests/fixtures/config/valid-pre-safety.toml");
        write_fixture(&path, raw);
        let before = fs::read(&path).expect("read legacy config before load");

        let config = FileConfig::load_or_create(&path).expect("legacy config should remain valid");

        assert_eq!(config.listen_ip, "127.0.0.1");
        assert_eq!(config.qb_servers.len(), 1);
        assert!(config.qb_servers[0].id.is_empty());
        assert_eq!(
            fs::read(&path).expect("read legacy config after load"),
            before
        );
    }

    #[test]
    fn config_store_skips_write_and_backup_when_normalized_value_is_unchanged() {
        let root = temp_test_dir("unchanged");
        let path = root.join("config.toml");
        let config = FileConfig::default();
        config.save(&path).expect("write initial config");
        let before = fs::read(&path).expect("read config before unchanged persist");
        #[cfg(unix)]
        let before_inode = {
            use std::os::unix::fs::MetadataExt;
            fs::metadata(&path).expect("metadata before persist").ino()
        };

        let backup = FileConfig::persist_normalized_if_changed(&path, &config, &config)
            .expect("unchanged config should not fail");

        assert!(backup.is_none());
        assert_eq!(fs::read(&path).expect("read config after persist"), before);
        assert_eq!(
            fs::read_dir(&root)
                .expect("list config directory")
                .filter_map(Result::ok)
                .count(),
            1
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(
                fs::metadata(&path).expect("metadata after persist").ino(),
                before_inode
            );
        }
    }

    #[test]
    fn config_store_backs_up_before_persisting_normalized_change() {
        let root = temp_test_dir("normalized");
        let path = root.join("config.toml");
        let original = FileConfig::default();
        original.save(&path).expect("write original config");
        let original_bytes = fs::read(&path).expect("read original config");
        let mut normalized = original.clone();
        normalized.listen_ip = "127.0.0.2".to_string();

        let backup = FileConfig::persist_normalized_if_changed(&path, &original, &normalized)
            .expect("persist normalized config")
            .expect("changed config must create backup");

        assert!(backup
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.starts_with("config.toml.bak.")));
        assert_eq!(fs::read(&backup).expect("read backup"), original_bytes);
        let reloaded = FileConfig::load_or_create(&path).expect("load normalized config");
        assert_eq!(reloaded.listen_ip, "127.0.0.2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&backup)
                    .expect("backup metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&path)
                    .expect("normalized config metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn config_store_atomic_replace_failure_preserves_old_file() {
        let root = temp_test_dir("replace-failure");
        let path = root.join("config.toml");
        let original = FileConfig::default();
        original.save(&path).expect("write original config");
        let before = fs::read(&path).expect("read original config before failed replace");

        let error = atomic_write_secret_with_rename(&path, b"replacement", |_from, _to| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected rename failure",
            ))
        })
        .expect_err("injected rename failure must be returned");

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            fs::read(&path).expect("read config after failed replace"),
            before
        );
        assert_eq!(
            fs::read_dir(&root)
                .expect("list directory after failed replace")
                .filter_map(Result::ok)
                .count(),
            1,
            "failed atomic replace must clean its temporary file"
        );
    }

    #[test]
    fn config_store_directory_sync_failure_is_post_commit_warning() {
        let root = temp_test_dir("directory-sync-failure");
        let path = root.join("config.toml");
        FileConfig::default()
            .save(&path)
            .expect("write original config");

        atomic_write_secret_with_ops(
            &path,
            b"committed replacement",
            |from, to| fs::rename(from, to),
            |_parent| Err(std::io::Error::other("injected directory sync failure")),
        )
        .expect("rename success is the commit point");

        assert_eq!(
            fs::read(&path).expect("read committed replacement"),
            b"committed replacement"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path)
                    .expect("committed config metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn config_store_creates_new_secret_file_with_mode_0600() {
        use std::os::unix::fs::PermissionsExt;

        let root = temp_test_dir("permissions");
        let path = root.join("nested").join("config.toml");

        FileConfig::load_or_create(&path).expect("create default config");

        assert_eq!(
            fs::metadata(&path)
                .expect("config metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[tokio::test]
    async fn config_manager_failed_persist_keeps_memory_file_and_revision() {
        let root = temp_test_dir("manager-persist-failure");
        let path = root.join("config.toml");
        let original = FileConfig::default();
        original.save(&path).expect("write manager source config");
        let before_file = fs::read(&path).expect("read manager source config");
        let manager = ConfigManager::with_persist(path.clone(), original.clone(), |_path, _cfg| {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected persist failure",
            ))
        });

        let error = manager
            .update(Some(1), |cfg| {
                cfg.tmdb_api_key = "must-not-commit".to_string();
                Ok(())
            })
            .await
            .expect_err("persist failure must be returned");

        assert!(matches!(error, ConfigUpdateError::Persist(_)));
        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.value, original);
        assert_eq!(
            fs::read(&path).expect("read config after failure"),
            before_file
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn config_manager_persist_does_not_block_a_single_thread_runtime() {
        use std::sync::{mpsc, Arc, Mutex as StdMutex};
        use std::time::Duration;

        use tokio::sync::oneshot;

        let root = temp_test_dir("manager-runtime-responsive");
        let path = root.join("config.toml");
        let original = FileConfig::default();
        original.save(&path).expect("write manager source config");

        let (started_tx, started_rx) = oneshot::channel();
        let started_tx = Arc::new(StdMutex::new(Some(started_tx)));
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(StdMutex::new(release_rx));
        let manager = ConfigManager::with_persist(path, original, move |_path, _cfg| {
            if let Some(started_tx) = started_tx.lock().unwrap().take() {
                let _ = started_tx.send(());
            }
            release_rx
                .lock()
                .unwrap()
                .recv()
                .expect("release config persist fixture");
            Ok(())
        });

        let update_manager = manager.clone();
        let update = tokio::spawn(async move {
            update_manager
                .update(Some(1), |cfg| {
                    cfg.tmdb_api_key = "runtime-remains-responsive".to_string();
                    Ok(())
                })
                .await
        });
        started_rx.await.expect("config persist fixture started");

        tokio::time::timeout(Duration::from_millis(100), async {
            tokio::time::sleep(Duration::from_millis(10)).await;
        })
        .await
        .expect("Tokio timer must progress while config persistence is blocked");

        release_tx.send(()).expect("release config persist fixture");
        update
            .await
            .expect("join config update")
            .expect("config update should commit");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelled_config_update_keeps_blocking_capacity_until_persist_returns() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{mpsc, Arc, Mutex as StdMutex};
        use std::time::Duration;

        use tokio::sync::oneshot;

        let root = temp_test_dir("manager-cancelled-persist");
        let path = root.join("config.toml");
        let original = FileConfig::default();
        original.save(&path).expect("write manager source config");

        let calls = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = oneshot::channel();
        let started_tx = Arc::new(StdMutex::new(Some(started_tx)));
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Arc::new(StdMutex::new(release_rx));
        let manager = ConfigManager::with_persist(path.clone(), original, move |path, cfg| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                if let Some(started_tx) = started_tx.lock().unwrap().take() {
                    let _ = started_tx.send(());
                }
                release_rx
                    .lock()
                    .unwrap()
                    .recv()
                    .expect("release cancelled config persist fixture");
            }
            cfg.save(path)
        });

        let first_manager = manager.clone();
        let first = tokio::spawn(async move {
            first_manager
                .update(Some(1), |cfg| {
                    cfg.tmdb_api_key = "cancelled-update".to_string();
                    Ok(())
                })
                .await
        });
        started_rx.await.expect("cancelled config persist started");
        first.abort();

        let second_manager = manager.clone();
        let mut second = tokio::spawn(async move {
            second_manager
                .update(Some(1), |cfg| {
                    cfg.mteam_api_key = "committed-after-cancel".to_string();
                    Ok(())
                })
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut second)
                .await
                .is_err(),
            "the second persist must wait for detached blocking work"
        );

        release_tx
            .send(())
            .expect("release cancelled config persist fixture");
        let committed = tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .expect("second config update should finish")
            .expect("join second config update")
            .expect("second config update should commit");
        assert_eq!(committed.revision, 2);
        assert_eq!(committed.value.tmdb_api_key, "");
        assert_eq!(committed.value.mteam_api_key, "committed-after-cancel");
        assert_eq!(
            FileConfig::load_or_create(&path).expect("reload config after cancelled update"),
            committed.value
        );
    }

    #[tokio::test]
    async fn config_manager_rejects_stale_revision_without_writing() {
        let root = temp_test_dir("manager-stale");
        let path = root.join("config.toml");
        let original = FileConfig::default();
        original.save(&path).expect("write manager source config");
        let manager = ConfigManager::new(path.clone(), original);

        let committed = manager
            .update(Some(1), |cfg| {
                cfg.tmdb_api_key = "first".to_string();
                Ok(())
            })
            .await
            .expect("first revision update should commit");
        assert_eq!(committed.revision, 2);
        let committed_file = fs::read(&path).expect("read committed config");

        let error = manager
            .update(Some(1), |cfg| {
                cfg.mteam_api_key = "stale".to_string();
                Ok(())
            })
            .await
            .expect_err("stale revision must fail");

        assert!(matches!(
            error,
            ConfigUpdateError::Stale {
                expected: 1,
                actual: 2
            }
        ));
        assert_eq!(manager.snapshot().await, committed);
        assert_eq!(
            fs::read(&path).expect("read config after stale update"),
            committed_file
        );
    }

    #[tokio::test]
    async fn config_manager_qr_cookie_patch_preserves_concurrent_settings_change() {
        let root = temp_test_dir("manager-qr-concurrent");
        let path = root.join("config.toml");
        let original = FileConfig::default();
        original.save(&path).expect("write manager source config");
        let manager = ConfigManager::new(path.clone(), original);

        let settings_manager = manager.clone();
        let qr_manager = manager.clone();
        let (settings_result, qr_result) = tokio::join!(
            settings_manager.update(None, |cfg| {
                cfg.mteam_api_key = "new-mteam-key".to_string();
                Ok(())
            }),
            qr_manager.patch_douban_cookie("dbcl2=new-cookie; ck=test".to_string()),
        );
        settings_result.expect("settings patch should commit");
        qr_result.expect("QR cookie patch should commit");

        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.revision, 3);
        assert_eq!(snapshot.value.mteam_api_key, "new-mteam-key");
        assert_eq!(snapshot.value.douban_cookie, "dbcl2=new-cookie; ck=test");
        assert_eq!(
            FileConfig::load_or_create(&path).expect("reload concurrent config"),
            snapshot.value
        );
    }

    #[tokio::test]
    async fn config_manager_settings_patch_preserves_cookie_when_omitted() {
        let root = temp_test_dir("manager-keep-cookie");
        let path = root.join("config.toml");
        let original = FileConfig {
            douban_cookie: "dbcl2=existing; ck=test".to_string(),
            ..FileConfig::default()
        };
        original.save(&path).expect("write manager source config");
        let manager = ConfigManager::new(path.clone(), original);

        manager
            .update(Some(1), |cfg| {
                cfg.tmdb_api_key = "new-tmdb-key".to_string();
                Ok(())
            })
            .await
            .expect("settings patch should commit");

        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.value.tmdb_api_key, "new-tmdb-key");
        assert_eq!(snapshot.value.douban_cookie, "dbcl2=existing; ck=test");
        assert_eq!(
            FileConfig::load_or_create(&path).expect("reload kept-cookie config"),
            snapshot.value
        );
    }

    #[tokio::test]
    async fn config_manager_serializes_two_disjoint_patches() {
        let root = temp_test_dir("manager-disjoint");
        let path = root.join("config.toml");
        let original = FileConfig::default();
        original.save(&path).expect("write manager source config");
        let manager = ConfigManager::new(path.clone(), original);

        let first_manager = manager.clone();
        let second_manager = manager.clone();
        let (first, second) = tokio::join!(
            first_manager.update(None, |cfg| {
                cfg.tmdb_api_key = "tmdb-disjoint".to_string();
                Ok(())
            }),
            second_manager.update(None, |cfg| {
                cfg.mteam_api_key = "mteam-disjoint".to_string();
                Ok(())
            }),
        );
        first.expect("first disjoint patch should commit");
        second.expect("second disjoint patch should commit");

        let snapshot = manager.snapshot().await;
        assert_eq!(snapshot.revision, 3);
        assert_eq!(snapshot.value.tmdb_api_key, "tmdb-disjoint");
        assert_eq!(snapshot.value.mteam_api_key, "mteam-disjoint");
        assert_eq!(
            FileConfig::load_or_create(&path).expect("reload disjoint config"),
            snapshot.value
        );
    }

    #[test]
    fn non_loopback_listener_requires_admin_token() {
        let mut config = FileConfig {
            listen_ip: "0.0.0.0".to_string(),
            ..FileConfig::default()
        };

        let error = config
            .validate()
            .expect_err("non-loopback listener without token must fail");
        assert!(error.to_string().contains("127.0.0.1"));
        assert!(error.to_string().contains("admin_token"));

        config.management.admin_token = "short-token".to_string();
        let error = config
            .validate()
            .expect_err("short management token must fail");
        assert!(error.to_string().contains("24"));

        config.management.admin_token = "valid-management-token-123456".to_string();
        config
            .validate()
            .expect("non-loopback listener with strong token should validate");
    }

    #[test]
    fn existing_non_loopback_config_without_token_fails_without_rewrite() {
        let root = temp_test_dir("non-loopback-without-token");
        let path = root.join("config.toml");
        let raw = concat!(
            "listen_ip = \"0.0.0.0\"\n",
            "listen_port = 8787\n",
            "tmdb_api_key = \"test-only\"\n",
        );
        write_fixture(&path, raw);
        let before = fs::read(&path).expect("read non-loopback config before load");

        let error = FileConfig::load_or_create(&path)
            .expect_err("non-loopback config without token must fail closed");

        assert!(error.to_string().contains("admin_token"));
        assert_eq!(
            fs::read(&path).expect("read non-loopback config after failed load"),
            before
        );
        assert_eq!(
            fs::read_dir(&root)
                .expect("list non-loopback config directory")
                .filter_map(Result::ok)
                .count(),
            1,
            "validation failure must not create a backup or temporary file"
        );
    }
}
