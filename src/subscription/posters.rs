//! Persistent subscription posters. Only metadata execution may fetch upstream.
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use sha1::{Digest, Sha1};
use tokio::sync::Mutex;

use crate::clients::douban::DoubanClient;

pub(crate) const POSTER_PREFIX: &str = "/api/subscriptions/posters/";

#[derive(Clone)]
pub(crate) struct SubscriptionPosters {
    root: PathBuf,
    gate: Arc<Mutex<()>>,
}

impl SubscriptionPosters {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            gate: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) async fn cache(
        &self,
        client: &DoubanClient,
        subject_id: &str,
        url: &str,
    ) -> Result<String, String> {
        self.ensure(subject_id, || async {
            // Older metadata contains proxy URLs; preserve signed query strings.
            let upstream = if url.starts_with("/api/douban/image?") {
                reqwest::Url::parse(&format!("http://localhost{url}"))
                    .ok()
                    .and_then(|parsed| {
                        parsed
                            .query_pairs()
                            .find(|(key, _)| key == "url")
                            .map(|(_, value)| value.into_owned())
                    })
                    .ok_or_else(|| "invalid poster URL".to_string())?
            } else {
                url.to_string()
            };
            crate::douban::fetch_image(client, &upstream)
                .await
                .map_err(|error| error.to_string())
        })
        .await
    }

    async fn ensure<F, Fut>(&self, subject_id: &str, fetch: F) -> Result<String, String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<(String, Vec<u8>), String>>,
    {
        let key = format!("{:x}", Sha1::digest(subject_id.as_bytes()));
        let local_url = format!("{POSTER_PREFIX}{key}");
        let _guard = self.gate.lock().await;
        match self.read(&key).await {
            Ok(_) => return Ok(local_url),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
        let (content_type, bytes) = fetch().await?;
        if !content_type.starts_with("image/")
            || content_type.contains(['\r', '\n'])
            || bytes.is_empty()
        {
            return Err("invalid poster response".to_string());
        }
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|error| error.to_string())?;
        let mut stored = content_type.into_bytes();
        stored.push(b'\n');
        stored.extend(bytes);
        let path = self.root.join(&key);
        let temporary = path.with_extension("tmp");
        let result = async {
            tokio::fs::write(&temporary, stored).await?;
            tokio::fs::rename(&temporary, &path).await
        }
        .await;
        if result.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        result.map_err(|error: std::io::Error| error.to_string())?;
        Ok(local_url)
    }

    pub(crate) async fn read(&self, key: &str) -> std::io::Result<(String, Vec<u8>)> {
        if key.len() != 40 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        let stored = tokio::fs::read(self.root.join(key)).await?;
        let invalid = || std::io::Error::from(std::io::ErrorKind::InvalidData);
        let separator = stored
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(invalid)?;
        let content_type = std::str::from_utf8(&stored[..separator]).map_err(|_| invalid())?;
        if !content_type.starts_with("image/")
            || content_type.contains('\r')
            || separator + 1 == stored.len()
        {
            return Err(invalid());
        }
        Ok((content_type.to_string(), stored[separator + 1..].to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn posters_survive_restart_and_never_fetch_again() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(".workspaces")
            .join(format!("poster-test-{}", std::process::id()));
        let cache = SubscriptionPosters::new(root.clone());
        let url = cache
            .ensure("123", || async { Ok(("image/webp".into(), vec![1, 2, 3])) })
            .await
            .unwrap();
        let cache = SubscriptionPosters::new(root.clone());
        let again = cache
            .ensure("123", || async { panic!("cached poster must not fetch") })
            .await
            .unwrap();
        assert_eq!(url, again);
        assert_eq!(
            cache
                .read(url.strip_prefix(POSTER_PREFIX).unwrap())
                .await
                .unwrap(),
            ("image/webp".into(), vec![1, 2, 3])
        );
        assert!(cache.read("../config.toml").await.is_err());
        assert!(cache.read(&"0".repeat(40)).await.is_err());
        assert!(cache
            .ensure("456", || async { Err("download failed".into()) })
            .await
            .is_err());
        assert!(cache
            .ensure("456", || async { Ok(("image/jpeg".into(), vec![4])) })
            .await
            .is_ok());
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
