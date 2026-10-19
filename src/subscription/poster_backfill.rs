//! Backfill poster files without restarting subscription workflows.
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::ports::{SubscriptionMutationRepository, SubscriptionReadRepository};
use super::posters::{SubscriptionPosters, POSTER_PREFIX};
use super::repository::{
    ListSubscriptionsCommand, RepositoryError, RepositoryResult, SubscriptionKey,
    SubscriptionListFilter, UpdateSubscriptionDetailCommand,
};
use crate::clients::douban::DoubanClient;
use crate::config::ConfigManager;
use crate::storage::SqliteSubscriptionRepository;

#[derive(Clone)]
pub(crate) struct PosterBackfill {
    repository: Arc<SqliteSubscriptionRepository>,
    client: DoubanClient,
    posters: SubscriptionPosters,
}

impl PosterBackfill {
    pub(crate) fn new(
        repository: Arc<SqliteSubscriptionRepository>,
        client: DoubanClient,
        posters: SubscriptionPosters,
    ) -> Self {
        Self {
            repository,
            client,
            posters,
        }
    }

    async fn run_pass(
        &self,
        account_key: &str,
        cookie: &str,
        cancelled: &watch::Receiver<bool>,
    ) -> RepositoryResult<usize> {
        let mut cursor = None;
        let mut repaired = 0;
        loop {
            if *cancelled.borrow() {
                return Ok(repaired);
            }
            // No lifecycle/activity filter: completed and inactive subscriptions need posters too.
            let page = self
                .repository
                .list_summaries(ListSubscriptionsCommand::try_new(
                    account_key,
                    SubscriptionListFilter::default(),
                    cursor,
                    100,
                )?)
                .await?;
            for summary in page.items {
                if *cancelled.borrow() {
                    return Ok(repaired);
                }
                if summary.projection.poster_url.starts_with(POSTER_PREFIX) {
                    continue;
                }
                let key = summary.head.key;
                match self.backfill_one(key.clone(), cookie).await {
                    Ok(true) => repaired += 1,
                    Ok(false) => {}
                    Err(_) => {
                        // Do not log raw URLs, signed queries, cookies, or upstream error bodies.
                        tracing::warn!(subject_id = %key.subject_id, "subscription poster backfill failed; will retry");
                    }
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                return Ok(repaired);
            }
        }
    }

    async fn backfill_one(&self, key: SubscriptionKey, cookie: &str) -> RepositoryResult<bool> {
        let detail = self.repository.load_detail(key.clone()).await?;
        if detail
            .payload()
            .source
            .poster_url
            .starts_with(POSTER_PREFIX)
        {
            return Ok(false);
        }
        let source = &detail.payload().source;
        let url = if source.poster_url.is_empty() {
            &source.cover_url
        } else {
            &source.poster_url
        };
        let local = match self.posters.cache(&self.client, &key.subject_id, url).await {
            Ok(local) => local,
            Err(_) => {
                // Legacy signed URLs can have expired. Refresh only enough metadata to obtain
                // a current poster, preserving all saved workflow and metadata fields.
                let refreshed =
                    crate::douban::subject_detail(&self.client, cookie, &key.subject_id)
                        .await
                        .map_err(|_| unavailable())?;
                let url = if refreshed.poster_url.is_empty() {
                    &refreshed.image
                } else {
                    &refreshed.poster_url
                };
                self.posters
                    .cache(&self.client, &key.subject_id, url)
                    .await
                    .map_err(|_| unavailable())?
            }
        };
        self.save_poster(key, local).await
    }

    async fn save_poster(&self, key: SubscriptionKey, local: String) -> RepositoryResult<bool> {
        // Reload after network I/O, then fence on this revision. Concurrent execution must win.
        let detail = self.repository.load_detail(key.clone()).await?;
        if detail
            .payload()
            .source
            .poster_url
            .starts_with(POSTER_PREFIX)
        {
            return Ok(false);
        }
        let mut payload = detail.payload().clone();
        payload.source.poster_url = local.clone();
        payload.source.cover_url = local;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .max(detail.summary().head.updated_at)
            .max(payload.observation.last_seen_at);
        self.repository
            .update_detail(UpdateSubscriptionDetailCommand::try_new(
                key,
                detail.summary().head.revision,
                now,
                detail.summary().attention_tags.clone(),
                payload,
            )?)
            .await?;
        Ok(true)
    }

    pub(crate) fn spawn(
        self,
        config: ConfigManager,
        mut cancelled: watch::Receiver<bool>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                if *cancelled.borrow() {
                    break;
                }
                let snapshot = config.snapshot().await;
                if let Ok(account_key) =
                    crate::douban::auth_cache_key_fragment(&snapshot.value.douban_cookie)
                {
                    match self
                        .run_pass(&account_key, &snapshot.value.douban_cookie, &cancelled)
                        .await
                    {
                        Ok(repaired) if repaired > 0 => {
                            tracing::info!(repaired, "backfilled subscription posters")
                        }
                        Ok(_) => {}
                        Err(_) => {
                            tracing::warn!("subscription poster backfill scan failed; will retry")
                        }
                    }
                }
                // Independent of automatic-download enable/dry-run switches. Failures must
                // retry even when the historical subscription itself is already complete.
                let retry = snapshot
                    .value
                    .subscription_watcher
                    .system_retry_interval_secs
                    .max(60);
                tokio::select! {
                    _ = cancelled.changed() => break,
                    _ = tokio::time::sleep(Duration::from_secs(retry)) => {},
                }
            }
        })
    }
}

fn unavailable() -> RepositoryError {
    RepositoryError::Unavailable {
        message: "subscription poster could not be cached".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppPaths, AppState};
    use crate::subscription::ports::{SubscriptionExecutionRepository, SubscriptionPollRepository};
    use crate::subscription::repository::{
        payload::WantedSourcePayload, ApplyCompleteSnapshotCommand, BeginPollCommand,
        ClaimOneCommand, ClaimOneResult, NewRecordPolicy, SnapshotRecord,
    };
    use crate::subscription::SubscriptionMediaKind;
    use sha1::{Digest, Sha1};

    #[tokio::test]
    async fn backfill_pages_through_history_and_preserves_completed_inactive_payloads() {
        let root = std::env::temp_dir().join(format!("poster-backfill-{}", std::process::id()));
        let paths = AppPaths::for_test_root(&root);
        let state = AppState::for_test(paths.clone());
        let repository = &state.subscription_repository;
        let poster_dir = paths.subscription_state_dir.join("posters");
        std::fs::create_dir_all(&poster_dir).unwrap();
        let mut records = Vec::new();
        for id in 1..=103 {
            let subject = id.to_string();
            records.push(
                SnapshotRecord::try_new(
                    &subject,
                    SubscriptionMediaKind::Movie,
                    true,
                    None,
                    WantedSourcePayload {
                        title: format!("Movie {id}"),
                        // The seeded persistent file must be reused, even though this URL is invalid.
                        poster_url: "legacy-unavailable-url".into(),
                        summary: Some("Keep historical metadata".into()),
                        ..Default::default()
                    },
                )
                .unwrap(),
            );
            let hash = format!("{:x}", Sha1::digest(subject.as_bytes()));
            std::fs::write(poster_dir.join(hash), b"image/webp\nposter").unwrap();
        }
        let begin = repository
            .begin_poll(BeginPollCommand::try_new("account", 100).unwrap())
            .await
            .unwrap();
        repository
            .apply_complete_snapshot(
                ApplyCompleteSnapshotCommand::try_new(
                    "account",
                    begin.token,
                    101,
                    3701,
                    NewRecordPolicy::try_new(3, true).unwrap(),
                    records,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let connection = rusqlite::Connection::open(&state.subscription_database_path).unwrap();
        connection.execute("UPDATE wanted_subscription_records SET lifecycle_state = 'completed', active = 0, inactive_at = 102, updated_at = 102, schedulable = 0, blocked_reason = 'inactive', next_attempt_at = NULL WHERE subject_id = '1'", []).unwrap();
        drop(connection);
        let key = SubscriptionKey::try_new("account", "1").unwrap();
        let before = repository.load_detail(key.clone()).await.unwrap();
        let backfill = PosterBackfill::new(
            repository.clone(),
            state.upstream_clients.douban.clone(),
            state.subscription_posters.clone(),
        );
        let (cancel, cancelled) = watch::channel(false);
        assert_eq!(
            backfill.run_pass("account", "", &cancelled).await.unwrap(),
            103
        );
        let after = repository.load_detail(key.clone()).await.unwrap();
        let mut expected = before.payload().clone();
        let local = format!("{POSTER_PREFIX}{:x}", Sha1::digest(b"1"));
        expected.source.poster_url = local.clone();
        expected.source.cover_url = local.clone();
        assert_eq!(after.payload(), &expected);
        let mut expected_head = before.summary().head.clone();
        expected_head.revision = after.summary().head.revision;
        expected_head.updated_at = after.summary().head.updated_at;
        assert_eq!(after.summary().head, expected_head);
        assert_eq!(
            after.summary().attention_tags,
            before.summary().attention_tags
        );
        assert_eq!(after.summary().projection.poster_url, local);
        assert_eq!(
            backfill.run_pass("account", "", &cancelled).await.unwrap(),
            0
        );
        assert_eq!(repository.load_detail(key).await.unwrap(), after);
        cancel.send(true).unwrap();
        assert_eq!(
            backfill.run_pass("account", "", &cancelled).await.unwrap(),
            0
        );
        drop(backfill);
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[tokio::test]
    async fn saving_a_poster_preserves_concurrent_metadata_and_running_attempt() {
        let root = std::env::temp_dir().join(format!("poster-save-{}", std::process::id()));
        let state = AppState::for_test(AppPaths::for_test_root(&root));
        let repository = &state.subscription_repository;
        let begin = repository
            .begin_poll(BeginPollCommand::try_new("account", 100).unwrap())
            .await
            .unwrap();
        repository
            .apply_complete_snapshot(
                ApplyCompleteSnapshotCommand::try_new(
                    "account",
                    begin.token,
                    101,
                    3701,
                    NewRecordPolicy::try_new(3, false).unwrap(),
                    vec![SnapshotRecord::try_new(
                        "1",
                        SubscriptionMediaKind::Movie,
                        true,
                        None,
                        WantedSourcePayload {
                            title: "Original".into(),
                            ..Default::default()
                        },
                    )
                    .unwrap()],
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let key = SubscriptionKey::try_new("account", "1").unwrap();
        let detail = repository.load_detail(key.clone()).await.unwrap();
        let mut payload = detail.payload().clone();
        payload.source.title = "Changed while downloading".into();
        repository
            .update_detail(
                UpdateSubscriptionDetailCommand::try_new(
                    key.clone(),
                    detail.summary().head.revision,
                    102,
                    detail.summary().attention_tags.clone(),
                    payload,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        let claim = repository
            .claim_one(ClaimOneCommand::try_new(key.clone(), 120).unwrap())
            .await
            .unwrap();
        let ClaimOneResult::Claimed(claimed) = claim else {
            panic!("expected metadata claim")
        };
        let before = claimed.detail().summary().head.clone();
        let backfill = PosterBackfill::new(
            repository.clone(),
            state.upstream_clients.douban.clone(),
            state.subscription_posters.clone(),
        );
        assert!(backfill
            .save_poster(key.clone(), format!("{POSTER_PREFIX}{}", "a".repeat(40)))
            .await
            .unwrap());
        let after = repository.load_detail(key).await.unwrap();
        assert_eq!(after.payload().source.title, "Changed while downloading");
        let mut expected = before;
        expected.revision = after.summary().head.revision;
        expected.updated_at = after.summary().head.updated_at;
        assert_eq!(after.summary().head, expected);
        drop(backfill);
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
    }
}
