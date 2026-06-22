use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::Value;
use tokio::fs;

/// TMDB 响应磁盘缓存：按文件修改时间判断是否过期。
#[derive(Clone)]
pub struct TmdbDiskCache {
    root: PathBuf,
    ttl: Duration,
}

impl TmdbDiskCache {
    pub fn new(root: PathBuf, ttl: Duration) -> Self {
        Self { root, ttl }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.root
            .join(format!("{}.json", Self::safe_key_fragment(key)))
    }

    fn safe_key_fragment(key: &str) -> String {
        key.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect()
    }

    pub async fn ensure_dir(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.root).await
    }

    pub async fn get(&self, key: &str) -> Option<Value> {
        let path = self.path_for(key);
        let meta = fs::metadata(&path).await.ok()?;
        let modified = meta.modified().ok()?;
        let age = SystemTime::now().duration_since(modified).ok()?;
        if age > self.ttl {
            return None;
        }
        let s = fs::read_to_string(&path).await.ok()?;
        serde_json::from_str(&s).ok()
    }

    pub async fn get_any(&self, key: &str) -> Option<Value> {
        let s = fs::read_to_string(self.path_for(key)).await.ok()?;
        serde_json::from_str(&s).ok()
    }

    pub async fn put(&self, key: &str, value: &Value) -> std::io::Result<()> {
        self.ensure_dir().await?;
        let path = self.path_for(key);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec(value)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        fs::write(&tmp, body).await?;
        fs::rename(&tmp, path).await?;
        Ok(())
    }

    pub async fn remove_prefix(&self, key_prefix: &str) -> std::io::Result<()> {
        let safe_prefix = Self::safe_key_fragment(key_prefix);
        let mut entries = match fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&safe_prefix) && name.ends_with(".json") {
                let _ = fs::remove_file(entry.path()).await;
            }
        }
        Ok(())
    }
}
