//! A durable prepare/cleanup boundary: never add the new torrent in the same
//! attempt that deletes the old one. A failed cleanup retains all old records.
use std::path::{Component, Path, PathBuf};

use super::*;

impl LatestSubscriptionExecutionEffects {
    pub(super) async fn execute_replacement(
        &self,
        claimed: &ClaimedSubscription,
        policy: &ExecutionEffectPolicy,
    ) -> ExecutionEffectResult {
        match replace(
            claimed.detail().payload(),
            &claimed.detail().summary().head.key,
            policy,
        )
        .await
        {
            Ok(payload_delta) => ExecutionEffectResult::Finished {
                disposition: FinishExecutionDisposition::SearchWaiting {
                    retry_after: schedule_delay(1),
                },
                payload_delta,
            },
            Err(message) => failed(
                ExecutionOperation::Search,
                "replacement_cleanup",
                message,
                policy.system_retry_interval_secs,
            ),
        }
    }
}

async fn replace(
    payload: &super::super::repository::SubscriptionPayload,
    key: &super::super::repository::SubscriptionKey,
    policy: &ExecutionEffectPolicy,
) -> Result<ExecutionPayloadDelta, String> {
    let category = category_for_source(&payload.source.tags, &policy.categories)?;
    let mut downloads = Vec::new();
    let mut torrents = Vec::new();
    // Inspect every old task before performing any destructive work.
    for download in &payload.artifacts.downloads {
        let server = client_qb_server(qb_server(&policy.qb_servers, &download.qb_server_id)?);
        let torrent = observe_download(&server, key, download).await?;
        if let Some(torrent) = &torrent {
            require_owned_torrent(torrent, &download.idempotency_key)?;
        }
        if !payload.replacement_prepared {
            let mut prepared = download.clone();
            if let Some(torrent) = &torrent {
                let files = qbittorrent::torrent_files(&server, &torrent.hash)
                    .await
                    .map_err(|error| error.to_string())?;
                prepared =
                    updated_download(download, torrent, &files, payload.tv.as_ref(), system_now());
            }
            downloads.push(prepared);
        }
        torrents.push((server, torrent));
    }
    if !payload.replacement_prepared {
        return Ok(ExecutionPayloadDelta::ReplacementPrepared {
            download_updates: downloads,
        });
    }

    let artifacts = payload.artifacts.clone();
    let category = category.clone();
    // Remove only recorded links verified to refer to their original source.
    // This happens before qB removes the source, so retries can still verify it.
    let paths = tokio::task::spawn_blocking(move || -> Result<Vec<PathBuf>, String> {
        let mut links = Vec::new();
        let mut sources = Vec::new();
        for download in &artifacts.downloads {
            for file in &download.files {
                let relative = Path::new(&file.name);
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
                {
                    return Err("下载文件路径不安全，已停止换种".to_string());
                }
                sources.push(checked_path(
                    Path::new(&category.download_dir),
                    &Path::new(&category.download_dir).join(relative),
                )?);
            }
        }
        for link in &artifacts.links {
            for file in &link.files {
                let target = checked_path(
                    Path::new(&category.link_target_dir),
                    Path::new(&file.target_path),
                )?;
                let source = checked_path(
                    Path::new(&category.download_dir),
                    Path::new(&file.source_path),
                )?;
                super::super::effect_adapters::verify_cleanup_link(&source, &target, file.outcome)?;
                links.push(target);
                sources.push(source);
            }
        }
        for target in links {
            remove_file_if_present(&target)?;
            remove_empty_parents(&target, Path::new(&category.link_target_dir));
        }
        Ok(sources)
    })
    .await
    .map_err(|error| error.to_string())??;

    for (server, torrent) in torrents {
        if let Some(torrent) = torrent {
            qbittorrent::delete_torrent_and_files(&server, &torrent.hash)
                .await
                .map_err(|error| error.to_string())?;
            if !qbittorrent::list_torrents_by_hashes(&server, &[torrent.hash])
                .await
                .map_err(|error| error.to_string())?
                .is_empty()
            {
                return Err("qB 正在删除旧任务，稍后继续清理".to_string());
            }
        }
    }
    // qB deletion can be asynchronous. Do not forget the manifest or start
    // another torrent while any old content (including .!qB partials) remains.
    tokio::task::spawn_blocking(move || {
        for path in paths {
            let partial = PathBuf::from(format!("{}.!qB", path.display()));
            for path in [path, partial] {
                match std::fs::symlink_metadata(&path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.to_string()),
                    Ok(_) => {
                        return Err(format!(
                            "旧下载文件尚未删除，请检查 qB 文件删除结果：{}",
                            path.display()
                        ))
                    }
                }
            }
        }
        Ok(())
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(ExecutionPayloadDelta::ReplacementCleaned)
}

fn require_owned_torrent(torrent: &QbTorrentInfo, key: &str) -> Result<(), String> {
    let tags = torrent.tags.split(',').map(str::trim).collect::<Vec<_>>();
    if !tags.contains(&key)
        || tags
            .iter()
            .any(|tag| tag.starts_with("download:v1:") && *tag != key)
    {
        return Err("旧 qB 任务归属不明确或被其他订阅共用，已停止换种".to_string());
    }
    Ok(())
}

fn checked_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    if !root.is_absolute() || !path.is_absolute() {
        return Err("清理目录必须使用绝对路径".to_string());
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "清理文件不在配置目录内".to_string())?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("清理文件路径不安全".to_string());
    }
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("清理路径包含符号链接".to_string())
            }
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                return Err(error.to_string())
            }
            _ => {}
        }
    }
    Ok(path.to_path_buf())
}

fn remove_file_if_present(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn remove_empty_parents(path: &Path, root: &Path) {
    let mut parent = path.parent();
    while let Some(directory) = parent {
        if directory == root || !directory.starts_with(root) {
            break;
        }
        match std::fs::remove_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => break, // Preserve directories containing unrelated files.
        }
        parent = directory.parent();
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::repository::{SubscriptionKey, SubscriptionPayload};
    use super::*;
    use axum::{
        extract::Form,
        http::StatusCode,
        routing::{get, post},
        Json, Router,
    };
    use std::sync::{Arc, Mutex};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "replacement-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn cleanup_rejects_shared_torrents_and_paths_outside_the_root() {
        let mut torrent = QbTorrentInfo {
            tags: "download:v1:own, download:v1:other".to_string(),
            ..Default::default()
        };
        assert!(require_owned_torrent(&torrent, "download:v1:own").is_err());
        torrent.tags = "unrelated".to_string();
        assert!(require_owned_torrent(&torrent, "download:v1:own").is_err());
        torrent.tags = "download:v1:own, movies".to_string();
        assert!(require_owned_torrent(&torrent, "download:v1:own").is_ok());
        let fixture = Fixture::new();
        assert!(checked_path(&fixture.0, &fixture.0.join("../other.mkv")).is_err());
        assert!(checked_path(&fixture.0, &fixture.0).is_err());
        assert!(checked_path(&fixture.0, &fixture.0.join("movie.mkv")).is_ok());
    }

    #[tokio::test]
    async fn replacement_journals_files_before_delete_and_recovers_from_failure_and_missing_qb_task(
    ) {
        let fixture = Fixture::new();
        let source = fixture.0.join("movie.mkv");
        std::fs::write(&source, b"old").unwrap();
        let key = SubscriptionKey::try_new("account", "subject").unwrap();
        let effect_key = stable_download_artifact_key("account", "subject", "old");
        let torrent = QbTorrentInfo {
            hash: "a".repeat(40),
            tags: effect_key.clone(),
            ..Default::default()
        };
        // (present, fail deletion, deletion calls)
        let state = Arc::new(Mutex::new((true, true, 0usize)));
        let info_state = state.clone();
        let delete_state = state.clone();
        let app = Router::new()
            .route("/api/v2/auth/login", post(|| async { "Ok." }))
            .route(
                "/api/v2/torrents/info",
                get(move || {
                    let state = info_state.clone();
                    let torrent = torrent.clone();
                    async move {
                        Json(if state.lock().unwrap().0 {
                            vec![torrent]
                        } else {
                            vec![]
                        })
                    }
                }),
            )
            .route(
                "/api/v2/torrents/files",
                get(|| async {
                    Json(serde_json::json!([{ "name": "movie.mkv", "size": 3, "progress": 1.0 }]))
                }),
            )
            .route(
                "/api/v2/torrents/delete",
                post(
                    move |Form(form): Form<std::collections::HashMap<String, String>>| {
                        let state = delete_state.clone();
                        async move {
                            assert_eq!(form["hashes"], "a".repeat(40));
                            assert_eq!(form["deleteFiles"], "true");
                            let mut state = state.lock().unwrap();
                            state.2 += 1;
                            if state.1 {
                                return StatusCode::FORBIDDEN;
                            }
                            state.0 = false;
                            StatusCode::OK
                        }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let policy = ExecutionEffectPolicy {
            categories: vec![ExecutionCategory {
                name: "movies".into(),
                wanted_tag: "movie".into(),
                qb_server_id: "qb".into(),
                qb_category: "movies".into(),
                qb_save_dir_name: "/downloads".into(),
                download_dir: fixture.0.to_string_lossy().into_owned(),
                link_target_dir: fixture.0.join("library").to_string_lossy().into_owned(),
            }],
            qb_servers: vec![ExecutionQbServer {
                id: "qb".into(),
                name: "qB".into(),
                base_url,
                username: "user".into(),
                password: "password".into(),
                insecure_tls: false,
            }],
            ..Default::default()
        };
        let mut payload: SubscriptionPayload = serde_json::from_value(serde_json::json!({
            "source": { "title": "Movie", "tags": ["movie"] },
            "replacement_pending": true,
            "artifacts": { "downloads": [{ "idempotency_key": effect_key, "torrent_id": "old", "torrent_title": "Old", "qb_server_id": "qb", "qb_category": "movies", "qb_save_dir_name": "/downloads", "state": "pushed" }] }
        })).unwrap();
        let ExecutionPayloadDelta::ReplacementPrepared { download_updates } =
            replace(&payload, &key, &policy).await.unwrap()
        else {
            panic!("expected durable preparation")
        };
        assert_eq!(state.lock().unwrap().2, 0);
        assert_eq!(download_updates[0].files[0].name, "movie.mkv");
        payload.artifacts.downloads = download_updates;
        payload.replacement_prepared = true;
        assert!(replace(&payload, &key, &policy).await.is_err());
        assert!(source.exists());
        state.lock().unwrap().1 = false;
        // A successful qB response alone is insufficient while content remains.
        assert!(replace(&payload, &key, &policy)
            .await
            .unwrap_err()
            .contains("旧下载文件尚未删除"));
        std::fs::remove_file(&source).unwrap();
        assert!(matches!(
            replace(&payload, &key, &policy).await.unwrap(),
            ExecutionPayloadDelta::ReplacementCleaned
        ));
        assert_eq!(state.lock().unwrap().2, 2);
        server.abort();
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_verifies_hardlink_identity_and_rejects_symlinks() {
        let fixture = Fixture::new();
        let source = fixture.0.join("source");
        let link = fixture.0.join("link");
        std::fs::write(&source, "content").unwrap();
        std::fs::hard_link(&source, &link).unwrap();
        assert!(super::super::super::effect_adapters::verify_cleanup_link(
            &source,
            &link,
            LinkFileOutcome::Linked
        )
        .is_ok());
        remove_file_if_present(&link).unwrap();
        std::fs::write(&link, "unrelated").unwrap();
        assert!(super::super::super::effect_adapters::verify_cleanup_link(
            &source,
            &link,
            LinkFileOutcome::Linked
        )
        .is_err());
        remove_file_if_present(&link).unwrap();
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert!(checked_path(&fixture.0, &link).is_err());
    }
}
