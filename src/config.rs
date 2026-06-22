use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QbServerEntry {
    #[serde(default)]
    pub name: String,
    pub base_url: String,
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub insecure_tls: bool,
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
}

fn default_listen_ip() -> String {
    "127.0.0.1".to_string()
}

fn default_listen_port() -> u16 {
    8787
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
