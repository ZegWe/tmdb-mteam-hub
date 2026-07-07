use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionWatcherConfig {
    #[serde(default)]
    pub enabled: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TorrentRuleMatchMode {
    All,
    Any,
}

impl Default for TorrentRuleMatchMode {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
            enabled: false,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

fn default_listen_ip() -> String {
    "0.0.0.0".to_string()
}

fn default_listen_port() -> u16 {
    8787
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
    300
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
        }
    }
}

impl FileConfig {
    pub fn listen_addr(&self) -> std::io::Result<SocketAddr> {
        let ip_raw = self.listen_ip.trim();
        let ip_raw = if ip_raw.is_empty() {
            default_listen_ip()
        } else {
            ip_raw.to_string()
        };
        let ip: IpAddr = ip_raw.parse().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("listen_ip 必须是合法 IP 地址: {e}"),
            )
        })?;
        Ok(SocketAddr::new(ip, self.listen_port))
    }

    pub fn load_or_create(path: &PathBuf) -> std::io::Result<Self> {
        if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            let cfg: FileConfig = toml::from_str(&raw).unwrap_or_else(|_| FileConfig::default());
            Ok(cfg)
        } else {
            let cfg = FileConfig::default();
            cfg.save(path)?;
            Ok(cfg)
        }
    }

    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, raw)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{FileConfig, SubscriptionWatcherConfig};

    #[test]
    fn default_listen_addr_binds_all_interfaces() {
        let addr = FileConfig::default()
            .listen_addr()
            .expect("default listen address should parse");

        assert_eq!(addr.to_string(), "0.0.0.0:8787");
    }

    #[test]
    fn subscription_watcher_defaults_include_lane_intervals() {
        let cfg = SubscriptionWatcherConfig::default();

        assert_eq!(cfg.search_interval_secs, 1_800);
        assert_eq!(cfg.progress_interval_secs, 300);
        assert_eq!(cfg.link_retry_interval_secs, 900);
        assert_eq!(cfg.system_retry_interval_secs, 600);
    }
}
