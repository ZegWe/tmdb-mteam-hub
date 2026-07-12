use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::Value;
use tokio::fs;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct CacheCleanupReport {
    pub(crate) scanned: u64,
    pub(crate) removed: u64,
    pub(crate) errors: u64,
}

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
            match fs::remove_file(&path).await {
                Ok(()) => tracing::debug!("removed expired JSON cache entry"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::warn!("failed to remove expired JSON cache entry: {error}"),
            }
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
        let body = serde_json::to_vec(value).map_err(|e| std::io::Error::other(e.to_string()))?;
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

    pub(crate) async fn cleanup_expired(&self) -> CacheCleanupReport {
        self.cleanup_expired_at(SystemTime::now()).await
    }

    async fn cleanup_expired_at(&self, now: SystemTime) -> CacheCleanupReport {
        let mut report = CacheCleanupReport::default();
        let mut entries = match fs::read_dir(&self.root).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return report,
            Err(_) => {
                report.errors = 1;
                return report;
            }
        };

        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(entry)) => entry,
                Ok(None) => break,
                Err(_) => {
                    report.errors += 1;
                    break;
                }
            };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !(name.ends_with(".json") || name.ends_with(".json.tmp")) {
                continue;
            }
            let file_type = match entry.file_type().await {
                Ok(file_type) => file_type,
                Err(_) => {
                    report.errors += 1;
                    continue;
                }
            };
            if !file_type.is_file() {
                continue;
            }
            report.scanned += 1;
            let modified = match entry
                .metadata()
                .await
                .and_then(|metadata| metadata.modified())
            {
                Ok(modified) => modified,
                Err(_) => {
                    report.errors += 1;
                    continue;
                }
            };
            let expired = now.duration_since(modified).is_ok_and(|age| age > self.ttl);
            if !expired {
                continue;
            }
            match fs::remove_file(entry.path()).await {
                Ok(()) => report.removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => report.errors += 1,
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{File, FileTimes};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tmdb-mteam-cache-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn set_modified(path: &std::path::Path, modified: SystemTime) {
        File::options()
            .write(true)
            .open(path)
            .expect("open cache fixture")
            .set_times(FileTimes::new().set_modified(modified))
            .expect("set cache fixture mtime");
    }

    #[tokio::test]
    async fn cache_cleanup_removes_only_expired_cache_files() {
        let root = temp_root("cleanup");
        std::fs::create_dir_all(root.join("nested.json")).expect("create nested fixture");
        let cache = TmdbDiskCache::new(root.clone(), Duration::from_secs(60));
        cache.ensure_dir().await.expect("create cache root");
        cache.put("fresh", &json!({ "fresh": true })).await.unwrap();
        cache
            .put("expired", &json!({ "expired": true }))
            .await
            .unwrap();
        std::fs::write(root.join("orphan.json.tmp"), b"temporary").unwrap();
        std::fs::write(root.join("config.toml"), b"secret = 'keep'").unwrap();
        std::fs::write(root.join("wanted.sqlite"), b"state-must-stay").unwrap();

        let now = SystemTime::now();
        set_modified(&root.join("expired.json"), now - Duration::from_secs(61));
        set_modified(&root.join("orphan.json.tmp"), now - Duration::from_secs(61));
        let report = cache.cleanup_expired_at(now).await;

        assert_eq!(report.scanned, 3);
        assert_eq!(report.removed, 2);
        assert_eq!(report.errors, 0);
        assert!(root.join("fresh.json").exists());
        assert!(!root.join("expired.json").exists());
        assert!(!root.join("orphan.json.tmp").exists());
        assert!(root.join("nested.json").is_dir());
        assert_eq!(
            std::fs::read(root.join("config.toml")).unwrap(),
            b"secret = 'keep'"
        );
        assert_eq!(
            std::fs::read(root.join("wanted.sqlite")).unwrap(),
            b"state-must-stay"
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn cache_get_deletes_an_expired_entry_instead_of_leaving_it_on_disk() {
        let root = temp_root("get-expired");
        let cache = TmdbDiskCache::new(root.clone(), Duration::from_secs(60));
        cache
            .put("expired", &json!({ "expired": true }))
            .await
            .unwrap();
        set_modified(
            &root.join("expired.json"),
            SystemTime::now() - Duration::from_secs(61),
        );

        assert!(cache.get("expired").await.is_none());
        assert!(!root.join("expired.json").exists());

        std::fs::remove_dir_all(root).unwrap();
    }
}
