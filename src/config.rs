use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileConfig {
    #[serde(default)]
    pub tmdb_api_key: String,
    #[serde(default)]
    pub mteam_api_key: String,
    #[serde(default)]
    pub qb_servers: Vec<QbServerEntry>,
}

impl FileConfig {
    pub fn load_or_create(path: &PathBuf) -> std::io::Result<Self> {
        if path.exists() {
            let raw = std::fs::read_to_string(path)?;
            let cfg: FileConfig =
                toml::from_str(&raw).unwrap_or_else(|_| FileConfig::default());
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
