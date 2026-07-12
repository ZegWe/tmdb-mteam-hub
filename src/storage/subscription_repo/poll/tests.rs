use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier as ThreadBarrier};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use tokio::sync::Barrier;

use super::{DEACTIVATE_MISSING_SQL, SNAPSHOT_ID_PREFIX};
use crate::storage::SqliteSubscriptionRepository;
use crate::subscription::ports::{
    SubscriptionMutationRepository, SubscriptionPollRepository, SubscriptionReadRepository,
};
use crate::subscription::repository::payload::{
    stable_download_artifact_key, stable_resolved_link_artifact_key, CandidateMatchPayload,
    CandidatePayload,
};
use crate::subscription::repository::{
    ApplyCompleteSnapshotCommand, BeginPollCommand, BlockedReason, DownloadArtifactPayload,
    DownloadArtifactStatePayload, IncompleteSnapshotObservation, IncompleteSnapshotReason,
    IssueOwnerPayload, IssuePayload, LinkArtifactPayload, LinkArtifactStatePayload,
    LinkDownloadRefPayload, NewRecordPolicy, PollAttemptToken, PollRetryPolicy,
    RecordIncompleteSnapshotCommand, RecordPollFailureCommand, RepositoryError, SnapshotRecord,
    SubscriptionKey, TvDetailPayload, UpdateSubscriptionDetailCommand, WantedSourcePayload,
};
use crate::subscription::{
    SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
    SubscriptionMediaKind, INACTIVE_SUBSCRIPTION_REASON, TV_NOT_SUPPORTED_REASON,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FixtureSeed {
    RowsOnly,
    TvAndArtifacts,
}

const ROWS_ONLY: FixtureSeed = FixtureSeed::RowsOnly;
const TV_AND_ARTIFACTS: FixtureSeed = FixtureSeed::TvAndArtifacts;
const FIXTURE_ACCOUNT: &str = "fixture_rows_only";
const TV_FIXTURE_ACCOUNT: &str = "fixture_tv_and_artifacts";
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
const SEED_AT: u64 = 1_800_000_000;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-v5-poll-repo-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create poll repository fixture directory");
        let path = root.join("subscriptions.sqlite");
        Self { root, path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

async fn fresh_fixture(label: &str, seed: FixtureSeed) -> Fixture {
    let fixture = Fixture::new(label);
    let repository = SqliteSubscriptionRepository::try_create_fresh(&fixture.path, 4, BUSY_TIMEOUT)
        .expect("create fresh latest-schema Poll repository");
    let token = begin(&repository, FIXTURE_ACCOUNT, SEED_AT).await;
    complete(
        &repository,
        FIXTURE_ACCOUNT,
        token,
        SEED_AT,
        vec![
            movie("rows-movie-001", "Fixture Rows Queued Movie", SEED_AT),
            movie(
                "rows-movie-002",
                "Fixture Rows Completed Movie",
                SEED_AT - 1,
            ),
        ],
        default_policy(),
    )
    .await;
    let connection = Connection::open(&fixture.path).expect("open fresh Poll seed database");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET lifecycle_state = 'completed', next_attempt_at = NULL
                WHERE account_key = ?1 AND subject_id = 'rows-movie-002'"#,
            [FIXTURE_ACCOUNT],
        )
        .expect("seed completed fresh Poll row");
    reset_seed_poll_meta(&connection, FIXTURE_ACCOUNT);
    drop(connection);

    if seed == FixtureSeed::TvAndArtifacts {
        seed_tv_artifact_detail(&repository, &fixture.path).await;
    }
    fixture
}

fn reset_seed_poll_meta(connection: &Connection, account_key: &str) {
    let changed = connection
        .execute(
            r#"UPDATE subscription_meta
                  SET updated_at = ?2,
                      last_poll_attempt_at = NULL,
                      last_poll_success_at = NULL,
                      poll_failure_count = 0,
                      next_poll_at = ?2,
                      last_poll_error = NULL,
                      poll_generation = 0,
                      open_poll_generation = NULL,
                      open_snapshot_id = NULL,
                      last_incomplete_at = NULL,
                      last_incomplete_reason = NULL,
                      last_incomplete_fetched_pages = NULL,
                      last_incomplete_truncated = NULL,
                      last_incomplete_end_observed = NULL,
                      last_complete_snapshot_id = NULL
                WHERE account_key = ?1"#,
            params![account_key, SEED_AT as i64],
        )
        .expect("reset fresh fixture Poll metadata");
    assert_eq!(changed, 1);
}

async fn seed_tv_artifact_detail(repository: &SqliteSubscriptionRepository, path: &Path) {
    let token = begin(repository, TV_FIXTURE_ACCOUNT, SEED_AT).await;
    complete(
        repository,
        TV_FIXTURE_ACCOUNT,
        token,
        SEED_AT,
        vec![tv("tv-artifact-001", "Fixture TV Artifact Series", SEED_AT)],
        default_policy(),
    )
    .await;
    let key = key(TV_FIXTURE_ACCOUNT, "tv-artifact-001");
    let detail = repository
        .load_detail(key.clone())
        .await
        .expect("load fresh TV artifact seed");
    let mut payload = detail.payload().clone();
    payload.tv = Some(TvDetailPayload {
        season_number: 1,
        episode_total: 2,
        target_start_episode: 1,
        target_end_episode: 2,
        episodes: Vec::new(),
    });
    payload.candidates.push(CandidateMatchPayload {
        candidate: CandidatePayload {
            torrent_id: "fixture-torrent-tv-001".to_string(),
            title: "Fixture.TV.S01E01-E02.1080p".to_string(),
            source: "fixture".to_string(),
            search_query: "fixture tv season 1".to_string(),
            ..CandidatePayload::default()
        },
        selected: true,
        ..CandidateMatchPayload::default()
    });
    let download_id = stable_download_artifact_key(
        TV_FIXTURE_ACCOUNT,
        "tv-artifact-001",
        "fixture-torrent-tv-001",
    );
    payload.artifacts.downloads.push(DownloadArtifactPayload {
        idempotency_key: download_id.clone(),
        torrent_id: "fixture-torrent-tv-001".to_string(),
        torrent_title: "Fixture.TV.S01E01-E02.1080p".to_string(),
        qb_server_id: "fixture-qb".to_string(),
        qb_server_name: Some("Fixture qB".to_string()),
        qb_category: "fixture-tv".to_string(),
        qb_save_dir_name: "fixture-tv-artifact-series".to_string(),
        qb_identifier: Some("FIXTURETVHASH001".to_string()),
        qb_hash: Some("FIXTURETVHASH001".to_string()),
        qb_name: Some("Fixture.TV.S01E01-E02.1080p".to_string()),
        qb_state: Some("completed".to_string()),
        torrent_download_url: None,
        mteam_torrent_url: None,
        state: DownloadArtifactStatePayload::Downloaded,
        progress: Some(1.0),
        total_size: Some(2_000),
        files: Vec::new(),
        pushed_at: Some(SEED_AT - 30),
        checked_at: Some(SEED_AT - 10),
        completed_at: Some(SEED_AT - 10),
    });
    payload.artifacts.links.push(LinkArtifactPayload {
        idempotency_key: stable_resolved_link_artifact_key(
            TV_FIXTURE_ACCOUNT,
            "tv-artifact-001",
            &download_id,
        ),
        download: LinkDownloadRefPayload {
            artifact_id: download_id,
        },
        state: LinkArtifactStatePayload::Planned,
        source_path: Some("/fixture/downloads/Fixture.TV.S01".to_string()),
        target_dir: Some("/fixture/library/Fixture TV/Season 01".to_string()),
        checked_at: SEED_AT - 5,
        completed_at: None,
        files: Vec::new(),
    });
    payload.issues.push(IssuePayload {
        owner: IssueOwnerPayload::Parent,
        operation: Some("fixture_seed".to_string()),
        error_type: Some("fixture".to_string()),
        message: "fixture issue preserved across Poll enrichment".to_string(),
        occurred_at: Some(SEED_AT - 1),
    });
    repository
        .update_detail(
            UpdateSubscriptionDetailCommand::try_new(
                key,
                detail.summary().head.revision,
                SEED_AT + 1,
                vec![SubscriptionAttentionTag::NeedsReconciliation],
                payload,
            )
            .unwrap(),
        )
        .await
        .expect("persist fresh TV artifact seed detail");
    let connection = Connection::open(path).expect("open fresh TV fixture metadata reset");
    reset_seed_poll_meta(&connection, TV_FIXTURE_ACCOUNT);
}

fn repository(path: &Path) -> SqliteSubscriptionRepository {
    SqliteSubscriptionRepository::try_new(path, 4, BUSY_TIMEOUT)
        .expect("construct staged poll repository")
}

fn source(title: &str, sort_time: u64) -> WantedSourcePayload {
    WantedSourcePayload {
        title: title.to_string(),
        poster_url: format!("https://example.test/{sort_time}.jpg"),
        category_text: Some("fixture-movie".to_string()),
        douban_sort_time: Some(sort_time),
        tags: vec!["movie".to_string()],
        ..WantedSourcePayload::default()
    }
}

fn movie(subject_id: &str, title: &str, sort_time: u64) -> SnapshotRecord {
    SnapshotRecord::try_new(
        subject_id,
        SubscriptionMediaKind::Movie,
        true,
        None,
        source(title, sort_time),
    )
    .expect("build schedulable movie snapshot record")
}

fn blocked_movie(subject_id: &str, title: &str, sort_time: u64, reason: &str) -> SnapshotRecord {
    SnapshotRecord::try_new(
        subject_id,
        SubscriptionMediaKind::Movie,
        false,
        Some(BlockedReason::try_new(reason).expect("valid blocked reason")),
        source(title, sort_time),
    )
    .expect("build blocked movie snapshot record")
}

fn tv(subject_id: &str, title: &str, sort_time: u64) -> SnapshotRecord {
    let mut source = source(title, sort_time);
    source.tags = vec!["tv".to_string()];
    SnapshotRecord::try_new(
        subject_id,
        SubscriptionMediaKind::Tv,
        false,
        Some(BlockedReason::try_new(TV_NOT_SUPPORTED_REASON).unwrap()),
        source,
    )
    .expect("build parked TV snapshot record")
}

async fn begin(
    repository: &SqliteSubscriptionRepository,
    account_key: &str,
    attempted_at: u64,
) -> PollAttemptToken {
    repository
        .begin_poll(BeginPollCommand::try_new(account_key, attempted_at).unwrap())
        .await
        .expect("begin poll attempt")
        .token
}

async fn complete(
    repository: &SqliteSubscriptionRepository,
    account_key: &str,
    token: PollAttemptToken,
    completed_at: u64,
    records: Vec<SnapshotRecord>,
    policy: NewRecordPolicy,
) -> crate::subscription::repository::ApplyCompleteSnapshotResult {
    repository
        .apply_complete_snapshot(
            ApplyCompleteSnapshotCommand::try_new(
                account_key,
                token,
                completed_at,
                completed_at + 60,
                policy,
                records,
            )
            .unwrap(),
        )
        .await
        .expect("apply complete snapshot")
}

fn default_policy() -> NewRecordPolicy {
    NewRecordPolicy::try_new(3, false).unwrap()
}

fn retry_policy() -> PollRetryPolicy {
    PollRetryPolicy::try_new(5, 60).unwrap()
}

fn key(account_key: &str, subject_id: &str) -> SubscriptionKey {
    SubscriptionKey::try_new(account_key, subject_id).unwrap()
}

fn meta_json(path: &Path, account_key: &str) -> Option<String> {
    Connection::open(path)
        .unwrap()
        .query_row(
            r#"SELECT json_array(
                       state_version, bootstrap_completed, created_at, updated_at,
                       last_poll_attempt_at, last_poll_success_at, poll_failure_count,
                       next_poll_at, last_poll_error, poll_generation,
                       open_poll_generation, open_snapshot_id, last_incomplete_at,
                       last_incomplete_reason, last_incomplete_fetched_pages,
                       last_incomplete_truncated, last_incomplete_end_observed,
                       last_complete_snapshot_id
                   )
                 FROM subscription_meta
                WHERE account_key = ?1"#,
            [account_key],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

fn row_json(path: &Path, account_key: &str, subject_id: &str) -> Option<String> {
    Connection::open(path)
        .unwrap()
        .query_row(
            r#"SELECT json_array(
                       revision, active, inactive_at, last_seen_snapshot_id, media_kind,
                       schedulable, blocked_reason, lifecycle_state, execution_state,
                       next_attempt_at, retry_count, max_retries, retry_blocked,
                       force_eligible_once, claimed_operation, attempt_id, lease_until,
                       title, release_year, poster_url, category_text, douban_sort_time,
                       attention_tags_json, updated_at, record_json
                   )
                 FROM wanted_subscription_records
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![account_key, subject_id],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
}

fn operation_logs_json(path: &Path) -> String {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT json_group_array(json_array(id, account_key, created_at, category, action, target_type, target_id, target_title, status, summary, error, related_json)) FROM operation_logs",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn account_subjects(path: &Path, account_key: &str) -> Vec<String> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare(
            "SELECT subject_id FROM wanted_subscription_records WHERE account_key = ?1 ORDER BY subject_id",
        )
        .unwrap();
    statement
        .query_map([account_key], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn current_open_token(path: &Path, account_key: &str) -> Option<(i64, String)> {
    Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT open_poll_generation, open_snapshot_id FROM subscription_meta WHERE account_key = ?1",
            [account_key],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .unwrap()
        .and_then(|(generation, snapshot)| generation.zip(snapshot))
}

fn account_operation_logs(path: &Path, account_key: &str) -> Vec<Value> {
    let connection = Connection::open(path).unwrap();
    let mut statement = connection
        .prepare(
            r#"SELECT json_object(
                       'created_at', created_at,
                       'category', category,
                       'action', action,
                       'target_type', target_type,
                       'target_id', target_id,
                       'target_title', target_title,
                       'status', status,
                       'summary', summary,
                       'error', error,
                       'related', json(related_json)
                   )
                 FROM operation_logs
                WHERE account_key = ?1
                ORDER BY id"#,
        )
        .unwrap();
    statement
        .query_map([account_key], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect()
}

fn seed_running_attempt(
    path: &Path,
    account_key: &str,
    subject_id: &str,
    lifecycle_state: &str,
    claimed_operation: &str,
    attempt_id: &str,
    lease_until: u64,
) {
    let changed = Connection::open(path)
        .unwrap()
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET revision = revision + 1,
                      lifecycle_state = ?3,
                      execution_state = 'running',
                      claimed_operation = ?4,
                      attempt_id = ?5,
                      lease_until = ?6
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![
                account_key,
                subject_id,
                lifecycle_state,
                claimed_operation,
                attempt_id,
                i64::try_from(lease_until).unwrap(),
            ],
        )
        .unwrap();
    assert_eq!(changed, 1);
}

#[tokio::test]
async fn begin_persists_stable_opaque_tokens_across_reopen_and_serializes_concurrent_begins() {
    let fixture = fresh_fixture("begin", ROWS_ONLY).await;
    let first_repository = repository(&fixture.path);
    let first = begin(&first_repository, FIXTURE_ACCOUNT, 1_900_000_000).await;
    assert_eq!(first.generation.value(), 1);
    assert!(first.snapshot_id.as_str().starts_with(SNAPSHOT_ID_PREFIX));
    assert_eq!(
        first.snapshot_id.as_str().len(),
        SNAPSHOT_ID_PREFIX.len() + 64
    );
    assert!(!first.snapshot_id.as_str().contains(FIXTURE_ACCOUNT));
    drop(first_repository);

    let persisted = current_open_token(&fixture.path, FIXTURE_ACCOUNT).unwrap();
    assert_eq!(persisted.0, 1);
    assert_eq!(persisted.1, first.snapshot_id.as_str());
    let deterministic_fixture = fresh_fixture("begin-deterministic", ROWS_ONLY).await;
    let deterministic = begin(
        &repository(&deterministic_fixture.path),
        FIXTURE_ACCOUNT,
        1_900_000_000,
    )
    .await;
    assert_eq!(deterministic, first);

    let new_account = "new-poll-account";
    let new_token = begin(&repository(&fixture.path), new_account, 100).await;
    assert_eq!(new_token.generation.value(), 1);
    let new_meta: serde_json::Value =
        serde_json::from_str(&meta_json(&fixture.path, new_account).unwrap()).unwrap();
    assert_eq!(new_meta[0], 1);
    assert_eq!(new_meta[1], 0);
    assert_eq!(new_meta[2], 100);
    assert_eq!(new_meta[3], 100);
    assert_eq!(new_meta[4], 100);
    assert_eq!(new_meta[6], 0);
    assert_eq!(new_meta[7], 100);
    assert_eq!(new_meta[9], 1);

    let concurrent_account = "concurrent-begin-account";
    let barrier = Arc::new(Barrier::new(3));
    let first_repo = repository(&fixture.path);
    let second_repo = repository(&fixture.path);
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let first_task = tokio::spawn(async move {
        first_barrier.wait().await;
        first_repo
            .begin_poll(BeginPollCommand::try_new(concurrent_account, 200).unwrap())
            .await
    });
    let second_task = tokio::spawn(async move {
        second_barrier.wait().await;
        second_repo
            .begin_poll(BeginPollCommand::try_new(concurrent_account, 200).unwrap())
            .await
    });
    barrier.wait().await;
    let first_result = first_task.await.unwrap().unwrap();
    let second_result = second_task.await.unwrap().unwrap();
    let (old, current) = if first_result.token.generation < second_result.token.generation {
        (first_result.token, second_result.token)
    } else {
        (second_result.token, first_result.token)
    };
    assert_eq!(old.generation.value(), 1);
    assert_eq!(current.generation.value(), 2);
    assert_eq!(
        current_open_token(&fixture.path, concurrent_account),
        Some((2, current.snapshot_id.as_str().to_string()))
    );

    let stale = repository(&fixture.path)
        .record_poll_failure(
            RecordPollFailureCommand::try_new(
                concurrent_account,
                old.clone(),
                201,
                "old poll",
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        stale,
        RepositoryError::StalePoll {
            account_key: concurrent_account.to_string(),
            attempted: old,
            current: Some(current),
        }
    );
}

#[tokio::test]
async fn begin_rejects_generation_state_version_and_time_regressions_without_writes() {
    let fixture = fresh_fixture("begin-errors", ROWS_ONLY).await;
    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute(
            "UPDATE subscription_meta SET poll_generation = ?2 WHERE account_key = ?1",
            params![FIXTURE_ACCOUNT, i64::MAX],
        )
        .unwrap();
    drop(connection);
    let before = meta_json(&fixture.path, FIXTURE_ACCOUNT);
    let exhausted = repository(&fixture.path)
        .begin_poll(BeginPollCommand::try_new(FIXTURE_ACCOUNT, 1_900_000_000).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(exhausted, RepositoryError::CorruptData { .. }));
    assert_eq!(meta_json(&fixture.path, FIXTURE_ACCOUNT), before);

    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute(
            "UPDATE subscription_meta SET poll_generation = 0, state_version = 2 WHERE account_key = ?1",
            [FIXTURE_ACCOUNT],
        )
        .unwrap();
    drop(connection);
    let before = meta_json(&fixture.path, FIXTURE_ACCOUNT);
    let wrong_version = repository(&fixture.path)
        .begin_poll(BeginPollCommand::try_new(FIXTURE_ACCOUNT, 1_900_000_000).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(wrong_version, RepositoryError::CorruptData { .. }));
    assert_eq!(meta_json(&fixture.path, FIXTURE_ACCOUNT), before);

    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute(
            "UPDATE subscription_meta SET state_version = 1 WHERE account_key = ?1",
            [FIXTURE_ACCOUNT],
        )
        .unwrap();
    drop(connection);
    let before = meta_json(&fixture.path, FIXTURE_ACCOUNT);
    let backwards = repository(&fixture.path)
        .begin_poll(BeginPollCommand::try_new(FIXTURE_ACCOUNT, 1_700_000_000).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        backwards,
        RepositoryError::InvalidInput {
            field: "attempted_at",
            ..
        }
    ));
    assert_eq!(meta_json(&fixture.path, FIXTURE_ACCOUNT), before);

    let overflow_account = "overflow-begin";
    let overflow = repository(&fixture.path)
        .begin_poll(BeginPollCommand::try_new(overflow_account, u64::MAX).unwrap())
        .await
        .unwrap_err();
    assert!(matches!(
        overflow,
        RepositoryError::InvalidInput {
            field: "attempted_at",
            ..
        }
    ));
    assert!(meta_json(&fixture.path, overflow_account).is_none());
}

#[tokio::test]
async fn three_terminal_kinds_compete_on_one_token_and_exactly_one_commits() {
    let fixture = fresh_fixture("terminal-race", ROWS_ONLY).await;
    let account = "terminal-race-account";
    let token = begin(&repository(&fixture.path), account, 100).await;
    let barrier = Arc::new(Barrier::new(4));

    let failure_repo = repository(&fixture.path);
    let failure_barrier = Arc::clone(&barrier);
    let failure_token = token.clone();
    let failure = tokio::spawn(async move {
        failure_barrier.wait().await;
        failure_repo
            .record_poll_failure(
                RecordPollFailureCommand::try_new(
                    account,
                    failure_token,
                    101,
                    "network failed",
                    retry_policy(),
                )
                .unwrap(),
            )
            .await
    });

    let incomplete_repo = repository(&fixture.path);
    let incomplete_barrier = Arc::clone(&barrier);
    let incomplete_token = token.clone();
    let incomplete = tokio::spawn(async move {
        incomplete_barrier.wait().await;
        incomplete_repo
            .record_incomplete_snapshot(
                RecordIncompleteSnapshotCommand::try_new(
                    account,
                    incomplete_token,
                    101,
                    IncompleteSnapshotObservation::try_new(
                        1,
                        false,
                        false,
                        IncompleteSnapshotReason::EndNotObserved,
                    )
                    .unwrap(),
                    default_policy(),
                    vec![movie("partial-seen", "Partial Winner", 101)],
                    retry_policy(),
                )
                .unwrap(),
            )
            .await
    });

    let complete_repo = repository(&fixture.path);
    let complete_barrier = Arc::clone(&barrier);
    let complete_token = token.clone();
    let complete_task = tokio::spawn(async move {
        complete_barrier.wait().await;
        complete_repo
            .apply_complete_snapshot(
                ApplyCompleteSnapshotCommand::try_new(
                    account,
                    complete_token,
                    101,
                    161,
                    default_policy(),
                    vec![movie("complete-seen", "Complete Winner", 101)],
                )
                .unwrap(),
            )
            .await
    });

    barrier.wait().await;
    let failure = failure.await.unwrap();
    let incomplete = incomplete.await.unwrap();
    let complete_result = complete_task.await.unwrap();
    let successes = usize::from(failure.is_ok())
        + usize::from(incomplete.is_ok())
        + usize::from(complete_result.is_ok());
    assert_eq!(successes, 1);
    let stale_count = usize::from(matches!(failure, Err(RepositoryError::StalePoll { .. })))
        + usize::from(matches!(incomplete, Err(RepositoryError::StalePoll { .. })))
        + usize::from(matches!(
            complete_result,
            Err(RepositoryError::StalePoll { .. })
        ));
    assert_eq!(stale_count, 2);
    let expected_subjects = if incomplete.is_ok() {
        vec!["partial-seen".to_string()]
    } else if complete_result.is_ok() {
        vec!["complete-seen".to_string()]
    } else {
        Vec::new()
    };
    assert_eq!(account_subjects(&fixture.path, account), expected_subjects);
    assert!(current_open_token(&fixture.path, account).is_none());

    let repeated = repository(&fixture.path)
        .record_poll_failure(
            RecordPollFailureCommand::try_new(
                account,
                token.clone(),
                102,
                "repeat",
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        repeated,
        RepositoryError::StalePoll {
            account_key: account.to_string(),
            attempted: token,
            current: None,
        }
    );
}

#[tokio::test]
async fn two_open_physical_connections_contend_for_begin_immediate_and_only_one_consumes_token() {
    let fixture = fresh_fixture("physical-terminal-race", ROWS_ONLY).await;
    let account = "physical-terminal-race-account";
    let token = begin(&repository(&fixture.path), account, 100).await;
    let barrier = Arc::new(ThreadBarrier::new(3));
    let first_path = fixture.path.clone();
    let second_path = fixture.path.clone();
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let first_token = token.clone();
    let second_token = token.clone();
    let first = std::thread::spawn(move || {
        let mut connection =
            crate::storage::sqlite::open_v5_connection(&first_path, BUSY_TIMEOUT).unwrap();
        first_barrier.wait();
        super::record_poll_failure(
            &mut connection,
            RecordPollFailureCommand::try_new(
                account,
                first_token,
                101,
                "first contender",
                retry_policy(),
            )
            .unwrap(),
        )
    });
    let second = std::thread::spawn(move || {
        let mut connection =
            crate::storage::sqlite::open_v5_connection(&second_path, BUSY_TIMEOUT).unwrap();
        second_barrier.wait();
        super::record_poll_failure(
            &mut connection,
            RecordPollFailureCommand::try_new(
                account,
                second_token,
                101,
                "second contender",
                retry_policy(),
            )
            .unwrap(),
        )
    });
    barrier.wait();
    let first = first.join().unwrap();
    let second = second.join().unwrap();
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        usize::from(matches!(first, Err(RepositoryError::StalePoll { .. })))
            + usize::from(matches!(second, Err(RepositoryError::StalePoll { .. }))),
        1
    );
    assert!(current_open_token(&fixture.path, account).is_none());
}

#[tokio::test]
async fn failure_preserves_rows_and_success_metadata_and_backoff_overflow_keeps_token_open() {
    let fixture = fresh_fixture("failure", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let first_row = row_json(&fixture.path, FIXTURE_ACCOUNT, "rows-movie-001");
    let second_row = row_json(&fixture.path, FIXTURE_ACCOUNT, "rows-movie-002");
    let token = begin(&repository, FIXTURE_ACCOUNT, 1_900_000_000).await;
    let before_meta: serde_json::Value =
        serde_json::from_str(&meta_json(&fixture.path, FIXTURE_ACCOUNT).unwrap()).unwrap();
    let result = repository
        .record_poll_failure(
            RecordPollFailureCommand::try_new(
                FIXTURE_ACCOUNT,
                token.clone(),
                1_900_000_010,
                "upstream unavailable",
                PollRetryPolicy::try_new(5, 60).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.failure_count, 1);
    assert_eq!(result.next_poll_at, 1_900_000_015);
    assert_eq!(
        row_json(&fixture.path, FIXTURE_ACCOUNT, "rows-movie-001"),
        first_row
    );
    assert_eq!(
        row_json(&fixture.path, FIXTURE_ACCOUNT, "rows-movie-002"),
        second_row
    );
    let after_meta: serde_json::Value =
        serde_json::from_str(&meta_json(&fixture.path, FIXTURE_ACCOUNT).unwrap()).unwrap();
    assert_eq!(after_meta[1], before_meta[1], "bootstrap must be preserved");
    assert_eq!(
        after_meta[5], before_meta[5],
        "last success must be preserved"
    );
    assert_eq!(
        after_meta[17], before_meta[17],
        "last complete snapshot must be preserved"
    );
    assert_eq!(after_meta[6], 1);
    assert_eq!(after_meta[7], 1_900_000_015_i64);
    assert_eq!(after_meta[8], "upstream unavailable");
    assert!(after_meta[10].is_null());
    assert!(after_meta[11].is_null());

    let overflow_account = "backoff-overflow-account";
    let overflow_token = begin(&repository, overflow_account, i64::MAX as u64 - 1).await;
    let before = meta_json(&fixture.path, overflow_account);
    let overflow = repository
        .record_poll_failure(
            RecordPollFailureCommand::try_new(
                overflow_account,
                overflow_token.clone(),
                i64::MAX as u64 - 1,
                "overflow",
                PollRetryPolicy::try_new(5, 5).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        overflow,
        RepositoryError::InvalidInput {
            field: "next_poll_at",
            ..
        }
    ));
    assert_eq!(meta_json(&fixture.path, overflow_account), before);
    assert_eq!(
        current_open_token(&fixture.path, overflow_account),
        Some((1, overflow_token.snapshot_id.as_str().to_string()))
    );

    let backwards_account = "backwards-terminal-account";
    let backwards_token = begin(&repository, backwards_account, 500).await;
    let before = meta_json(&fixture.path, backwards_account);
    let backwards = repository
        .record_poll_failure(
            RecordPollFailureCommand::try_new(
                backwards_account,
                backwards_token.clone(),
                499,
                "time regression",
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        backwards,
        RepositoryError::InvalidInput {
            field: "failed_at",
            ..
        }
    ));
    assert_eq!(meta_json(&fixture.path, backwards_account), before);
    let overflow_time = repository
        .record_poll_failure(
            RecordPollFailureCommand::try_new(
                backwards_account,
                backwards_token.clone(),
                u64::MAX,
                "time overflow",
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        overflow_time,
        RepositoryError::InvalidInput {
            field: "failed_at",
            ..
        }
    ));
    assert_eq!(meta_json(&fixture.path, backwards_account), before);
    let recovered = repository
        .record_poll_failure(
            RecordPollFailureCommand::try_new(
                backwards_account,
                backwards_token,
                501,
                "valid terminal",
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(recovered.failure_count, 1);
}

#[tokio::test]
async fn incomplete_is_seen_only_reports_effects_and_reactivates_without_touching_missing_rows() {
    let fixture = fresh_fixture("incomplete", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "incomplete-account";
    let first_token = begin(&repository, account, 100).await;
    complete(
        &repository,
        account,
        first_token,
        101,
        vec![movie("a", "A", 10), movie("b", "B", 9)],
        default_policy(),
    )
    .await;
    let b_before = row_json(&fixture.path, account, "b");
    let before_meta: serde_json::Value =
        serde_json::from_str(&meta_json(&fixture.path, account).unwrap()).unwrap();

    let incomplete_token = begin(&repository, account, 200).await;
    let result = repository
        .record_incomplete_snapshot(
            RecordIncompleteSnapshotCommand::try_new(
                account,
                incomplete_token,
                201,
                IncompleteSnapshotObservation::try_new(
                    2,
                    true,
                    false,
                    IncompleteSnapshotReason::ItemLimitReached,
                )
                .unwrap(),
                NewRecordPolicy::try_new(9, true).unwrap(),
                vec![movie("a", "A Enriched", 20), movie("c", "C", 8)],
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.inserted, 1);
    assert_eq!(result.updated, 1);
    assert_eq!(result.unchanged, 0);
    assert_eq!(result.reactivated, 0);
    assert_eq!(result.inserted + result.updated + result.unchanged, 2);
    assert_eq!(row_json(&fixture.path, account, "b"), b_before);
    let after_meta: serde_json::Value =
        serde_json::from_str(&meta_json(&fixture.path, account).unwrap()).unwrap();
    assert_eq!(after_meta[1], before_meta[1]);
    assert_eq!(after_meta[5], before_meta[5]);
    assert_eq!(after_meta[17], before_meta[17]);
    assert_eq!(after_meta[6], 1);
    assert_eq!(after_meta[12], 201);
    assert_eq!(after_meta[13], "item_limit_reached");
    assert_eq!(after_meta[14], 2);
    assert_eq!(after_meta[15], 1);
    assert_eq!(after_meta[16], 0);
    let a = repository.load_detail(key(account, "a")).await.unwrap();
    assert_eq!(a.payload().source.title, "A Enriched");
    let c = repository.load_detail(key(account, "c")).await.unwrap();
    assert_eq!(c.summary().head.max_retries, 9);
    assert!(
        c.payload().skip_reason.is_none(),
        "policy flag is ignored after bootstrap"
    );

    let deactivate_token = begin(&repository, account, 300).await;
    let deactivated = complete(
        &repository,
        account,
        deactivate_token,
        301,
        vec![movie("a", "A Enriched", 20), movie("c", "C", 8)],
        default_policy(),
    )
    .await;
    assert_eq!(deactivated.deactivated, 1);
    let complete_meta: serde_json::Value =
        serde_json::from_str(&meta_json(&fixture.path, account).unwrap()).unwrap();
    assert_eq!(complete_meta[1], 1);
    assert_eq!(complete_meta[5], 301);
    assert_eq!(complete_meta[6], 0);
    assert_eq!(complete_meta[7], 361);
    assert!(complete_meta[8].is_null());
    assert_eq!(complete_meta[12], 201, "incomplete history is retained");
    assert_eq!(complete_meta[13], "item_limit_reached");
    assert_eq!(
        complete_meta[17],
        deactivated.token.snapshot_id.as_str(),
        "successful complete snapshot becomes authoritative"
    );
    let b_inactive = repository.load_detail(key(account, "b")).await.unwrap();
    assert!(!b_inactive.summary().head.active);
    assert_eq!(
        b_inactive
            .summary()
            .head
            .blocked_reason
            .as_ref()
            .map(BlockedReason::as_str),
        Some(INACTIVE_SUBSCRIPTION_REASON)
    );

    let reactivate_token = begin(&repository, account, 400).await;
    let result = repository
        .record_incomplete_snapshot(
            RecordIncompleteSnapshotCommand::try_new(
                account,
                reactivate_token,
                401,
                IncompleteSnapshotObservation::try_new(
                    1,
                    false,
                    false,
                    IncompleteSnapshotReason::EndNotObserved,
                )
                .unwrap(),
                NewRecordPolicy::try_new(99, true).unwrap(),
                vec![movie("b", "B Returns", 30)],
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.inserted, 0);
    assert_eq!(result.updated, 1);
    assert_eq!(result.reactivated, 1);
    let b = repository.load_detail(key(account, "b")).await.unwrap();
    assert!(b.summary().head.active);
    assert!(b.summary().head.schedulable);
    assert!(b.summary().head.blocked_reason.is_none());
    assert_eq!(b.summary().head.next_attempt_at, Some(401));
    assert_eq!(b.summary().head.max_retries, 3);
    assert!(
        repository
            .load_detail(key(account, "a"))
            .await
            .unwrap()
            .summary()
            .head
            .active
    );
    assert!(
        repository
            .load_detail(key(account, "c"))
            .await
            .unwrap()
            .summary()
            .head
            .active
    );
}

#[tokio::test]
async fn incomplete_parks_only_movie_to_tv_running_attempt_and_preserves_seen_movie_attempt() {
    let fixture = fresh_fixture("incomplete-supersede", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "incomplete-supersede-account";
    let initial_token = begin(&repository, account, 100).await;
    complete(
        &repository,
        account,
        initial_token,
        101,
        vec![
            movie("park", "Park Old", 10),
            movie("keep", "Keep Old", 9),
            movie("idle", "Idle Old", 8),
        ],
        default_policy(),
    )
    .await;
    seed_running_attempt(
        &fixture.path,
        account,
        "park",
        "queued",
        "movie_meta",
        "attempt-park",
        150,
    );
    seed_running_attempt(
        &fixture.path,
        account,
        "keep",
        "searching",
        "movie_search",
        "attempt-keep",
        250,
    );

    let token = begin(&repository, account, 200).await;
    let result = repository
        .record_incomplete_snapshot(
            RecordIncompleteSnapshotCommand::try_new(
                account,
                token.clone(),
                201,
                IncompleteSnapshotObservation::try_new(
                    1,
                    false,
                    false,
                    IncompleteSnapshotReason::EndNotObserved,
                )
                .unwrap(),
                default_policy(),
                vec![
                    tv("park", "Park New TV", 20),
                    movie("keep", "Keep New Movie", 19),
                    tv("idle", "Idle New TV", 18),
                ],
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.updated, 3);
    assert_eq!(result.inserted, 0);
    assert_eq!(result.unchanged, 0);

    let keep = repository.load_detail(key(account, "keep")).await.unwrap();
    assert_eq!(keep.payload().source.title, "Keep New Movie");
    assert_eq!(
        keep.summary().head.execution_state,
        SubscriptionExecutionState::Running
    );
    let keep_controls: (String, String, i64) = Connection::open(&fixture.path)
        .unwrap()
        .query_row(
            "SELECT claimed_operation, attempt_id, lease_until FROM wanted_subscription_records WHERE account_key = ?1 AND subject_id = 'keep'",
            [account],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        keep_controls,
        ("movie_search".to_string(), "attempt-keep".to_string(), 250)
    );
    let idle = repository.load_detail(key(account, "idle")).await.unwrap();
    assert_eq!(idle.summary().head.media_kind, SubscriptionMediaKind::Tv);
    assert_eq!(
        idle.summary().head.execution_state,
        SubscriptionExecutionState::Idle
    );

    assert_eq!(
        account_operation_logs(&fixture.path, account),
        vec![json!({
            "created_at": 201,
            "category": "subscription_scheduler",
            "action": "supersede_attempt",
            "target_type": "subscription",
            "target_id": "park",
            "target_title": "Park New TV",
            "status": "success",
            "summary": "superseded an execution attempt during wanted poll persistence",
            "error": null,
            "related": {
                "schema": "subscription_attempt_superseded.v1",
                "disposition": "superseded",
                "reason": "parked_as_tv_not_supported",
                "attempt_id": "attempt-park",
                "claimed_operation": "movie_meta",
                "lease_until": 150,
                "lease_state_at_fence": "expired",
                "fenced_at": 201,
                "fenced_by": "wanted_poll",
                "poll_generation": token.generation.value(),
                "poll_snapshot_id": token.snapshot_id.as_str(),
                "poll_snapshot_kind": "incomplete",
                "revision_before": 2,
                "revision_after": 3,
                "execution_state_before": "running",
                "execution_state_after": "idle",
                "active_before": true,
                "active_after": true,
                "media_kind_before": "movie",
                "media_kind_after": "tv",
                "blocked_reason_before": null,
                "blocked_reason_after": TV_NOT_SUPPORTED_REASON,
                "replacement_attempt_id": null,
            },
        })]
    );
}

#[tokio::test]
async fn complete_snapshot_preserves_active_controls_parks_tv_and_deactivates_missing_once() {
    let fixture = fresh_fixture("complete-controls", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "complete-controls-account";
    let first_token = begin(&repository, account, 100).await;
    let initial = complete(
        &repository,
        account,
        first_token,
        101,
        vec![
            movie("a", "A", 10),
            movie("b", "B", 9),
            movie("d", "D", 8),
            tv("t", "T", 7),
        ],
        NewRecordPolicy::try_new(4, false).unwrap(),
    )
    .await;
    assert_eq!(initial.inserted, 4);
    assert_eq!(initial.updated, 0);
    assert_eq!(initial.deactivated, 0);

    let connection = Connection::open(&fixture.path).unwrap();
    for (subject_id, attempt_id) in [("a", "attempt-a"), ("d", "attempt-d")] {
        connection
            .execute(
                r#"UPDATE wanted_subscription_records
                      SET revision = revision + 1,
                          lifecycle_state = 'searching', execution_state = 'running',
                          retry_count = 1, force_eligible_once = 1,
                          claimed_operation = 'movie_search', attempt_id = ?3, lease_until = 250
                    WHERE account_key = ?1 AND subject_id = ?2"#,
                params![account, subject_id, attempt_id],
            )
            .unwrap();
    }
    drop(connection);

    let token = begin(&repository, account, 200).await;
    let result = complete(
        &repository,
        account,
        token,
        201,
        vec![
            movie("a", "A Enriched", 20),
            tv("d", "D Is TV", 19),
            movie("t", "T Still TV", 18),
            movie("c", "C New", 17),
        ],
        NewRecordPolicy::try_new(9, true).unwrap(),
    )
    .await;
    assert_eq!(result.inserted, 1);
    assert_eq!(result.updated, 3);
    assert_eq!(result.unchanged, 0);
    assert_eq!(result.reactivated, 0);
    assert_eq!(result.deactivated, 1);
    assert_eq!(result.inserted + result.updated + result.unchanged, 4);

    let a = repository.load_detail(key(account, "a")).await.unwrap();
    assert_eq!(a.summary().head.revision.value(), 3);
    assert_eq!(
        a.summary().head.lifecycle_state,
        SubscriptionLifecycleState::Searching
    );
    assert_eq!(
        a.summary().head.execution_state,
        SubscriptionExecutionState::Running
    );
    assert_eq!(a.summary().head.retry_count, 1);
    assert_eq!(a.summary().head.max_retries, 4);
    assert!(a.summary().head.force_eligible_once);
    assert_eq!(a.summary().head.next_attempt_at, Some(101));
    assert_eq!(a.payload().source.title, "A Enriched");
    let a_claim: (String, String, i64) = Connection::open(&fixture.path)
        .unwrap()
        .query_row(
            "SELECT claimed_operation, attempt_id, lease_until FROM wanted_subscription_records WHERE account_key = ?1 AND subject_id = 'a'",
            [account],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        a_claim,
        ("movie_search".to_string(), "attempt-a".to_string(), 250)
    );

    let d = repository.load_detail(key(account, "d")).await.unwrap();
    assert_eq!(d.summary().head.revision.value(), 3);
    assert_eq!(d.summary().head.media_kind, SubscriptionMediaKind::Tv);
    assert_eq!(
        d.summary().head.lifecycle_state,
        SubscriptionLifecycleState::Searching
    );
    assert_eq!(d.summary().head.retry_count, 1);
    assert_eq!(d.summary().head.max_retries, 4);
    assert_eq!(
        d.summary().head.execution_state,
        SubscriptionExecutionState::Idle
    );
    assert!(!d.summary().head.schedulable);
    assert_eq!(
        d.summary()
            .head
            .blocked_reason
            .as_ref()
            .map(BlockedReason::as_str),
        Some(TV_NOT_SUPPORTED_REASON)
    );
    assert!(d.summary().head.next_attempt_at.is_none());
    assert!(!d.summary().head.force_eligible_once);
    let d_claims: (Option<String>, Option<String>, Option<i64>) = Connection::open(&fixture.path)
        .unwrap()
        .query_row(
            "SELECT claimed_operation, attempt_id, lease_until FROM wanted_subscription_records WHERE account_key = ?1 AND subject_id = 'd'",
            [account],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(d_claims, (None, None, None));

    let parking_audits = account_operation_logs(&fixture.path, account);
    assert_eq!(
        parking_audits,
        vec![json!({
            "created_at": 201,
            "category": "subscription_scheduler",
            "action": "supersede_attempt",
            "target_type": "subscription",
            "target_id": "d",
            "target_title": "D Is TV",
            "status": "success",
            "summary": "superseded an execution attempt during wanted poll persistence",
            "error": null,
            "related": {
                "schema": "subscription_attempt_superseded.v1",
                "disposition": "superseded",
                "reason": "parked_as_tv_not_supported",
                "attempt_id": "attempt-d",
                "claimed_operation": "movie_search",
                "lease_until": 250,
                "lease_state_at_fence": "live",
                "fenced_at": 201,
                "fenced_by": "wanted_poll",
                "poll_generation": result.token.generation.value(),
                "poll_snapshot_id": result.token.snapshot_id.as_str(),
                "poll_snapshot_kind": "complete",
                "revision_before": 2,
                "revision_after": 3,
                "execution_state_before": "running",
                "execution_state_after": "idle",
                "active_before": true,
                "active_after": true,
                "media_kind_before": "movie",
                "media_kind_after": "tv",
                "blocked_reason_before": null,
                "blocked_reason_after": TV_NOT_SUPPORTED_REASON,
                "replacement_attempt_id": null,
            },
        })]
    );
    let stale = repository
        .apply_complete_snapshot(
            ApplyCompleteSnapshotCommand::try_new(
                account,
                result.token.clone(),
                202,
                262,
                default_policy(),
                vec![tv("d", "Must Not Reapply", 21)],
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(stale, RepositoryError::StalePoll { .. }));
    assert_eq!(
        account_operation_logs(&fixture.path, account),
        parking_audits
    );

    let t = repository.load_detail(key(account, "t")).await.unwrap();
    assert_eq!(t.summary().head.media_kind, SubscriptionMediaKind::Tv);
    assert!(!t.summary().head.schedulable);
    assert_eq!(
        t.summary()
            .head
            .blocked_reason
            .as_ref()
            .map(BlockedReason::as_str),
        Some(TV_NOT_SUPPORTED_REASON)
    );

    let b = repository.load_detail(key(account, "b")).await.unwrap();
    assert_eq!(b.summary().head.revision.value(), 2);
    assert!(!b.summary().head.active);
    assert!(!b.summary().head.schedulable);
    assert_eq!(
        b.summary()
            .head
            .blocked_reason
            .as_ref()
            .map(BlockedReason::as_str),
        Some(INACTIVE_SUBSCRIPTION_REASON)
    );
    assert_eq!(
        b.summary().head.lifecycle_state,
        SubscriptionLifecycleState::Queued
    );
    assert!(b.summary().head.next_attempt_at.is_none());

    let c = repository.load_detail(key(account, "c")).await.unwrap();
    assert_eq!(c.summary().head.revision.value(), 1);
    assert_eq!(c.summary().head.max_retries, 9);
    assert!(c.payload().skip_reason.is_none());
    assert!(!c
        .summary()
        .attention_tags
        .contains(&SubscriptionAttentionTag::Skipped));

    let empty_token = begin(&repository, account, 300).await;
    let empty = complete(
        &repository,
        account,
        empty_token,
        301,
        Vec::new(),
        default_policy(),
    )
    .await;
    assert_eq!(empty.deactivated, 4);
    let audits_after_empty = account_operation_logs(&fixture.path, account);
    assert_eq!(audits_after_empty.len(), 2);
    assert_eq!(audits_after_empty[1]["target_id"], "a");
    assert_eq!(audits_after_empty[1]["target_title"], "A Enriched");
    assert_eq!(
        audits_after_empty[1]["related"]["reason"],
        "missing_from_complete_snapshot"
    );
    assert_eq!(
        audits_after_empty[1]["related"]["lease_state_at_fence"],
        "expired"
    );
    let rows_after_empty = ["a", "b", "c", "d", "t"]
        .into_iter()
        .map(|subject| (subject, row_json(&fixture.path, account, subject).unwrap()))
        .collect::<Vec<_>>();
    let repeated_token = begin(&repository, account, 400).await;
    let repeated = complete(
        &repository,
        account,
        repeated_token,
        401,
        Vec::new(),
        default_policy(),
    )
    .await;
    assert_eq!(repeated.deactivated, 0);
    assert_eq!(
        account_operation_logs(&fixture.path, account),
        audits_after_empty
    );
    let rows_after_repeat = ["a", "b", "c", "d", "t"]
        .into_iter()
        .map(|subject| (subject, row_json(&fixture.path, account, subject).unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(rows_after_repeat, rows_after_empty);
    let parked_tv = repository.load_detail(key(account, "t")).await.unwrap();
    assert_eq!(
        parked_tv
            .summary()
            .head
            .blocked_reason
            .as_ref()
            .map(BlockedReason::as_str),
        Some(TV_NOT_SUPPORTED_REASON)
    );
}

#[tokio::test]
async fn complete_missing_running_attempts_audit_live_and_expired_leases_exactly_once() {
    let fixture = fresh_fixture("missing-supersede", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "missing-supersede-account";
    let initial_token = begin(&repository, account, 100).await;
    complete(
        &repository,
        account,
        initial_token,
        101,
        vec![
            movie("expired", "Expired Existing", 10),
            movie("live", "Live Existing", 9),
        ],
        default_policy(),
    )
    .await;
    seed_running_attempt(
        &fixture.path,
        account,
        "expired",
        "searching",
        "movie_search",
        "attempt-expired",
        201,
    );
    seed_running_attempt(
        &fixture.path,
        account,
        "live",
        "queued",
        "movie_meta",
        "attempt-live",
        202,
    );

    let token = begin(&repository, account, 200).await;
    let result = complete(
        &repository,
        account,
        token.clone(),
        201,
        Vec::new(),
        default_policy(),
    )
    .await;
    assert_eq!(result.deactivated, 2);
    let audits = account_operation_logs(&fixture.path, account);
    assert_eq!(audits.len(), 2);
    assert_eq!(
        audits[0],
        json!({
            "created_at": 201,
            "category": "subscription_scheduler",
            "action": "supersede_attempt",
            "target_type": "subscription",
            "target_id": "expired",
            "target_title": "Expired Existing",
            "status": "success",
            "summary": "superseded an execution attempt during wanted poll persistence",
            "error": null,
            "related": {
                "schema": "subscription_attempt_superseded.v1",
                "disposition": "superseded",
                "reason": "missing_from_complete_snapshot",
                "attempt_id": "attempt-expired",
                "claimed_operation": "movie_search",
                "lease_until": 201,
                "lease_state_at_fence": "expired",
                "fenced_at": 201,
                "fenced_by": "wanted_poll",
                "poll_generation": token.generation.value(),
                "poll_snapshot_id": token.snapshot_id.as_str(),
                "poll_snapshot_kind": "complete",
                "revision_before": 2,
                "revision_after": 3,
                "execution_state_before": "running",
                "execution_state_after": "idle",
                "active_before": true,
                "active_after": false,
                "media_kind_before": "movie",
                "media_kind_after": "movie",
                "blocked_reason_before": null,
                "blocked_reason_after": INACTIVE_SUBSCRIPTION_REASON,
                "replacement_attempt_id": null,
            },
        })
    );
    assert_eq!(audits[1]["target_id"], "live");
    assert_eq!(audits[1]["target_title"], "Live Existing");
    assert_eq!(
        audits[1]["related"]["reason"],
        "missing_from_complete_snapshot"
    );
    assert_eq!(audits[1]["related"]["attempt_id"], "attempt-live");
    assert_eq!(audits[1]["related"]["claimed_operation"], "movie_meta");
    assert_eq!(audits[1]["related"]["lease_until"], 202);
    assert_eq!(audits[1]["related"]["lease_state_at_fence"], "live");
    assert_eq!(audits[1]["related"]["active_after"], false);
    assert_eq!(
        audits[1]["related"]["blocked_reason_after"],
        INACTIVE_SUBSCRIPTION_REASON
    );

    let repeated_token = begin(&repository, account, 300).await;
    let repeated = complete(
        &repository,
        account,
        repeated_token,
        301,
        Vec::new(),
        default_policy(),
    )
    .await;
    assert_eq!(repeated.deactivated, 0);
    assert_eq!(account_operation_logs(&fixture.path, account), audits);
}

#[tokio::test]
async fn source_enrichment_preserves_tv_detail_artifacts_issues_candidates_and_attention() {
    let fixture = fresh_fixture("artifact-enrichment", TV_AND_ARTIFACTS).await;
    let repository = repository(&fixture.path);
    let key = key(TV_FIXTURE_ACCOUNT, "tv-artifact-001");
    let before = repository.load_detail(key.clone()).await.unwrap();
    assert!(!before.payload().artifacts.downloads.is_empty());
    assert!(!before.payload().artifacts.links.is_empty());
    let before_payload = before.payload().clone();
    let before_attention = before.summary().attention_tags.clone();
    let before_lifecycle = before.summary().head.lifecycle_state;
    let before_retry = (
        before.summary().head.retry_count,
        before.summary().head.max_retries,
        before.summary().head.retry_blocked,
    );

    let token = begin(&repository, TV_FIXTURE_ACCOUNT, 1_900_000_000).await;
    let result = complete(
        &repository,
        TV_FIXTURE_ACCOUNT,
        token,
        1_900_000_001,
        vec![movie(
            "tv-artifact-001",
            "Enriched Without Downgrade",
            1_900_000_001,
        )],
        NewRecordPolicy::try_new(99, true).unwrap(),
    )
    .await;
    assert_eq!(result.updated, 1);
    assert_eq!(result.reactivated, 0);
    assert_eq!(result.deactivated, 0);

    let after = repository.load_detail(key).await.unwrap();
    assert_eq!(after.summary().head.media_kind, SubscriptionMediaKind::Tv);
    assert_eq!(after.summary().head.lifecycle_state, before_lifecycle);
    assert_eq!(
        (
            after.summary().head.retry_count,
            after.summary().head.max_retries,
            after.summary().head.retry_blocked,
        ),
        before_retry
    );
    assert_eq!(after.summary().attention_tags, before_attention);
    assert_eq!(after.payload().issues, before_payload.issues);
    assert_eq!(after.payload().skip_reason, before_payload.skip_reason);
    assert_eq!(after.payload().candidates, before_payload.candidates);
    assert_eq!(after.payload().tv, before_payload.tv);
    assert_eq!(after.payload().artifacts, before_payload.artifacts);
    assert_eq!(after.payload().source.title, "Enriched Without Downgrade");
    assert_eq!(
        after.payload().observation.created_at,
        before_payload.observation.created_at
    );
    assert_eq!(
        after.payload().observation.first_seen_at,
        before_payload.observation.first_seen_at
    );
    assert_eq!(after.payload().observation.last_seen_at, 1_900_000_001);
}

#[tokio::test]
async fn reappearance_keeps_skipped_retry_blocked_completed_and_unschedulable_rows_undue() {
    let fixture = fresh_fixture("reactivation-due", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "reactivation-due-account";
    let token = begin(&repository, account, 100).await;
    complete(
        &repository,
        account,
        token,
        101,
        vec![
            movie("skipped", "Skipped", 10),
            movie("retry", "Retry", 9),
            movie("completed", "Completed", 8),
            movie("blocked", "Blocked", 7),
        ],
        default_policy(),
    )
    .await;
    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET attention_tags_json = '["skipped"]', next_attempt_at = NULL,
                      record_json = json_set(record_json, '$.skip_reason', 'manual_skip')
                WHERE account_key = ?1 AND subject_id = 'skipped'"#,
            [account],
        )
        .unwrap();
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET retry_count = max_retries, retry_blocked = 1, next_attempt_at = NULL,
                      attention_tags_json = '["retry_blocked"]'
                WHERE account_key = ?1 AND subject_id = 'retry'"#,
            [account],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE wanted_subscription_records SET lifecycle_state = 'completed', next_attempt_at = NULL WHERE account_key = ?1 AND subject_id = 'completed'",
            [account],
        )
        .unwrap();
    drop(connection);

    let deactivate_token = begin(&repository, account, 200).await;
    let deactivated = complete(
        &repository,
        account,
        deactivate_token,
        201,
        Vec::new(),
        default_policy(),
    )
    .await;
    assert_eq!(deactivated.deactivated, 4);

    let reactivate_token = begin(&repository, account, 300).await;
    let reactivated = complete(
        &repository,
        account,
        reactivate_token,
        301,
        vec![
            movie("skipped", "Skipped Returns", 20),
            movie("retry", "Retry Returns", 19),
            movie("completed", "Completed Returns", 18),
            blocked_movie("blocked", "Blocked Returns", 17, "manual_block"),
        ],
        NewRecordPolicy::try_new(99, false).unwrap(),
    )
    .await;
    assert_eq!(reactivated.inserted, 0);
    assert_eq!(reactivated.updated, 4);
    assert_eq!(reactivated.reactivated, 4);
    assert_eq!(reactivated.unchanged, 0);
    for subject_id in ["skipped", "retry", "completed", "blocked"] {
        let detail = repository
            .load_detail(key(account, subject_id))
            .await
            .unwrap();
        assert!(detail.summary().head.active, "{subject_id} must reactivate");
        assert_eq!(detail.summary().head.revision.value(), 3);
        assert!(
            detail.summary().head.next_attempt_at.is_none(),
            "{subject_id} has no next valid operation"
        );
        assert_eq!(detail.summary().head.max_retries, 3);
    }
    let blocked = repository
        .load_detail(key(account, "blocked"))
        .await
        .unwrap();
    assert!(!blocked.summary().head.schedulable);
    assert_eq!(
        blocked
            .summary()
            .head
            .blocked_reason
            .as_ref()
            .map(BlockedReason::as_str),
        Some("manual_block")
    );
}

#[tokio::test]
async fn bootstrap_skip_is_insert_only_and_incomplete_does_not_complete_bootstrap() {
    let fixture = fresh_fixture("bootstrap-partial", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "bootstrap-partial-account";
    let partial_token = begin(&repository, account, 100).await;
    let partial = repository
        .record_incomplete_snapshot(
            RecordIncompleteSnapshotCommand::try_new(
                account,
                partial_token,
                101,
                IncompleteSnapshotObservation::try_new(
                    1,
                    true,
                    false,
                    IncompleteSnapshotReason::ItemLimitReached,
                )
                .unwrap(),
                NewRecordPolicy::try_new(3, true).unwrap(),
                vec![movie("partial", "Partial Bootstrap", 10)],
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(partial.inserted, 1);
    let meta: serde_json::Value =
        serde_json::from_str(&meta_json(&fixture.path, account).unwrap()).unwrap();
    assert_eq!(meta[1], 0);
    assert!(meta[5].is_null());
    assert!(meta[17].is_null());
    let partial_row = repository
        .load_detail(key(account, "partial"))
        .await
        .unwrap();
    assert_eq!(partial_row.summary().head.max_retries, 3);
    assert_eq!(
        partial_row.payload().skip_reason.as_deref(),
        Some("initial_bootstrap_existing_wish")
    );
    assert!(partial_row
        .summary()
        .attention_tags
        .contains(&SubscriptionAttentionTag::Skipped));
    assert_eq!(
        partial_row.summary().head.next_attempt_at,
        Some(101),
        "bootstrap skip remains insert metadata; claim policy owns execution gating"
    );

    let complete_token = begin(&repository, account, 200).await;
    let completed = complete(
        &repository,
        account,
        complete_token,
        201,
        vec![
            movie("partial", "Partial Refreshed", 20),
            movie("complete-new", "Complete Bootstrap New", 19),
        ],
        NewRecordPolicy::try_new(9, true).unwrap(),
    )
    .await;
    assert_eq!(completed.inserted, 1);
    assert_eq!(completed.updated, 1);
    let refreshed = repository
        .load_detail(key(account, "partial"))
        .await
        .unwrap();
    assert_eq!(refreshed.summary().head.max_retries, 3);
    assert_eq!(
        refreshed.payload().skip_reason.as_deref(),
        Some("initial_bootstrap_existing_wish")
    );
    let complete_new = repository
        .load_detail(key(account, "complete-new"))
        .await
        .unwrap();
    assert_eq!(complete_new.summary().head.max_retries, 9);
    assert_eq!(
        complete_new.payload().skip_reason.as_deref(),
        Some("initial_bootstrap_existing_wish")
    );
    let meta: serde_json::Value =
        serde_json::from_str(&meta_json(&fixture.path, account).unwrap()).unwrap();
    assert_eq!(meta[1], 1);
    assert_eq!(meta[5], 201);

    let later_token = begin(&repository, account, 300).await;
    complete(
        &repository,
        account,
        later_token,
        301,
        vec![
            movie("partial", "Partial Refreshed", 20),
            movie("complete-new", "Complete Bootstrap New", 19),
            movie("post-bootstrap", "Post Bootstrap", 18),
        ],
        NewRecordPolicy::try_new(99, true).unwrap(),
    )
    .await;
    let post = repository
        .load_detail(key(account, "post-bootstrap"))
        .await
        .unwrap();
    assert_eq!(post.summary().head.max_retries, 99);
    assert!(post.payload().skip_reason.is_none());
    assert!(!post
        .summary()
        .attention_tags
        .contains(&SubscriptionAttentionTag::Skipped));
}

#[tokio::test]
async fn corrupt_second_row_and_revision_exhaustion_roll_back_all_seen_writes_and_allow_retry() {
    let fixture = fresh_fixture("terminal-rollback", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "terminal-rollback-account";
    let initial_token = begin(&repository, account, 100).await;
    complete(
        &repository,
        account,
        initial_token,
        101,
        vec![movie("a", "A", 10), movie("b", "B", 9)],
        default_policy(),
    )
    .await;
    let valid_b_payload: String = Connection::open(&fixture.path)
        .unwrap()
        .query_row(
            "SELECT record_json FROM wanted_subscription_records WHERE account_key = ?1 AND subject_id = 'b'",
            [account],
            |row| row.get(0),
        )
        .unwrap();
    let token = begin(&repository, account, 200).await;
    Connection::open(&fixture.path)
        .unwrap()
        .execute(
            "UPDATE wanted_subscription_records SET record_json = '{}' WHERE account_key = ?1 AND subject_id = 'b'",
            [account],
        )
        .unwrap();
    let a_before = row_json(&fixture.path, account, "a");
    let b_before = row_json(&fixture.path, account, "b");
    let meta_before = meta_json(&fixture.path, account);
    let isolation_before = operation_logs_json(&fixture.path);
    let command = ApplyCompleteSnapshotCommand::try_new(
        account,
        token.clone(),
        201,
        261,
        default_policy(),
        vec![movie("a", "A Changed", 20), movie("b", "B Changed", 19)],
    )
    .unwrap();
    let corrupt_error = repository
        .apply_complete_snapshot(command.clone())
        .await
        .unwrap_err();
    assert!(matches!(corrupt_error, RepositoryError::CorruptData { .. }));
    assert_eq!(row_json(&fixture.path, account, "a"), a_before);
    assert_eq!(row_json(&fixture.path, account, "b"), b_before);
    assert_eq!(meta_json(&fixture.path, account), meta_before);
    assert_eq!(operation_logs_json(&fixture.path), isolation_before);
    assert_eq!(
        current_open_token(&fixture.path, account),
        Some((
            token.generation.value() as i64,
            token.snapshot_id.as_str().to_string()
        ))
    );

    Connection::open(&fixture.path)
        .unwrap()
        .execute(
            "UPDATE wanted_subscription_records SET record_json = ?3 WHERE account_key = ?1 AND subject_id = ?2",
            params![account, "b", valid_b_payload],
        )
        .unwrap();
    let retry = repository
        .apply_complete_snapshot(command)
        .await
        .expect("same open token succeeds after persisted corruption is repaired");
    assert_eq!(retry.updated, 2);
    assert_eq!(
        repository
            .load_detail(key(account, "a"))
            .await
            .unwrap()
            .payload()
            .source
            .title,
        "A Changed"
    );

    let revision_token = begin(&repository, account, 300).await;
    let original_b_revision: i64 = Connection::open(&fixture.path)
        .unwrap()
        .query_row(
            "SELECT revision FROM wanted_subscription_records WHERE account_key = ?1 AND subject_id = 'b'",
            [account],
            |row| row.get(0),
        )
        .unwrap();
    Connection::open(&fixture.path)
        .unwrap()
        .execute(
            "UPDATE wanted_subscription_records SET revision = ?2 WHERE account_key = ?1 AND subject_id = 'b'",
            params![account, i64::MAX],
        )
        .unwrap();
    let a_before = row_json(&fixture.path, account, "a");
    let meta_before = meta_json(&fixture.path, account);
    let isolation_before = operation_logs_json(&fixture.path);
    let revision_command = ApplyCompleteSnapshotCommand::try_new(
        account,
        revision_token.clone(),
        301,
        361,
        default_policy(),
        vec![
            movie("a", "A Revision Rollback", 30),
            movie("b", "B Revision Exhausted", 29),
        ],
    )
    .unwrap();
    let revision_error = repository
        .apply_complete_snapshot(revision_command.clone())
        .await
        .unwrap_err();
    assert!(matches!(
        revision_error,
        RepositoryError::CorruptData { .. }
    ));
    assert_eq!(row_json(&fixture.path, account, "a"), a_before);
    assert_eq!(meta_json(&fixture.path, account), meta_before);
    assert_eq!(operation_logs_json(&fixture.path), isolation_before);
    assert_eq!(
        current_open_token(&fixture.path, account),
        Some((
            revision_token.generation.value() as i64,
            revision_token.snapshot_id.as_str().to_string()
        ))
    );
    Connection::open(&fixture.path)
        .unwrap()
        .execute(
            "UPDATE wanted_subscription_records SET revision = ?2 WHERE account_key = ?1 AND subject_id = 'b'",
            params![account, original_b_revision],
        )
        .unwrap();
    let retry = repository
        .apply_complete_snapshot(revision_command)
        .await
        .expect("same token retries after revision repair");
    assert_eq!(retry.updated, 2);
}

#[tokio::test]
async fn supersede_audit_and_post_audit_meta_failures_roll_back_and_keep_token_retryable() {
    let audit_fixture = fresh_fixture("poll-audit-rollback", ROWS_ONLY).await;
    let audit_repository = repository(&audit_fixture.path);
    let audit_account = "audit-row-rollback-account";
    let initial_token = begin(&audit_repository, audit_account, 100).await;
    complete(
        &audit_repository,
        audit_account,
        initial_token,
        101,
        vec![movie("park", "Park Before", 10)],
        default_policy(),
    )
    .await;
    seed_running_attempt(
        &audit_fixture.path,
        audit_account,
        "park",
        "queued",
        "movie_meta",
        "attempt-audit-rollback",
        250,
    );
    let token = begin(&audit_repository, audit_account, 200).await;
    Connection::open(&audit_fixture.path)
        .unwrap()
        .execute_batch(
            r#"CREATE TRIGGER reject_poll_supersede_audit
               BEFORE INSERT ON operation_logs
               WHEN NEW.account_key = 'audit-row-rollback-account'
                AND NEW.action = 'supersede_attempt'
               BEGIN
                   SELECT RAISE(ABORT, 'reject poll supersede audit');
               END;"#,
        )
        .unwrap();
    let row_before = row_json(&audit_fixture.path, audit_account, "park");
    let meta_before = meta_json(&audit_fixture.path, audit_account);
    let logs_before = account_operation_logs(&audit_fixture.path, audit_account);
    let command = ApplyCompleteSnapshotCommand::try_new(
        audit_account,
        token.clone(),
        201,
        261,
        default_policy(),
        vec![tv("park", "Park After", 20)],
    )
    .unwrap();
    let error = audit_repository
        .apply_complete_snapshot(command.clone())
        .await
        .unwrap_err();
    assert!(matches!(error, RepositoryError::CorruptData { .. }));
    assert_eq!(
        row_json(&audit_fixture.path, audit_account, "park"),
        row_before
    );
    assert_eq!(meta_json(&audit_fixture.path, audit_account), meta_before);
    assert_eq!(
        account_operation_logs(&audit_fixture.path, audit_account),
        logs_before
    );
    assert_eq!(
        current_open_token(&audit_fixture.path, audit_account),
        Some((
            token.generation.value() as i64,
            token.snapshot_id.as_str().to_string()
        ))
    );
    Connection::open(&audit_fixture.path)
        .unwrap()
        .execute_batch("DROP TRIGGER reject_poll_supersede_audit;")
        .unwrap();
    audit_repository
        .apply_complete_snapshot(command)
        .await
        .expect("same token succeeds after audit trigger is removed");
    assert_eq!(
        account_operation_logs(&audit_fixture.path, audit_account).len(),
        1
    );

    let meta_fixture = fresh_fixture("poll-meta-after-audit-rollback", ROWS_ONLY).await;
    let meta_repository = repository(&meta_fixture.path);
    let meta_account = "meta-after-audit-account";
    let initial_token = begin(&meta_repository, meta_account, 100).await;
    complete(
        &meta_repository,
        meta_account,
        initial_token,
        101,
        vec![movie("missing", "Missing", 10)],
        default_policy(),
    )
    .await;
    seed_running_attempt(
        &meta_fixture.path,
        meta_account,
        "missing",
        "queued",
        "movie_meta",
        "attempt-meta-rollback",
        250,
    );
    let token = begin(&meta_repository, meta_account, 200).await;
    Connection::open(&meta_fixture.path)
        .unwrap()
        .execute_batch(
            r#"CREATE TRIGGER reject_poll_meta_after_audit
               BEFORE UPDATE ON subscription_meta
               WHEN NEW.account_key = 'meta-after-audit-account'
                AND OLD.open_poll_generation IS NOT NULL
                AND NEW.open_poll_generation IS NULL
                AND NEW.last_complete_snapshot_id IS NOT OLD.last_complete_snapshot_id
               BEGIN
                   SELECT RAISE(ABORT, 'reject poll meta after audit');
               END;"#,
        )
        .unwrap();
    let row_before = row_json(&meta_fixture.path, meta_account, "missing");
    let meta_before = meta_json(&meta_fixture.path, meta_account);
    let command = ApplyCompleteSnapshotCommand::try_new(
        meta_account,
        token.clone(),
        201,
        261,
        default_policy(),
        Vec::new(),
    )
    .unwrap();
    let error = meta_repository
        .apply_complete_snapshot(command.clone())
        .await
        .unwrap_err();
    assert!(matches!(error, RepositoryError::CorruptData { .. }));
    assert_eq!(
        row_json(&meta_fixture.path, meta_account, "missing"),
        row_before
    );
    assert_eq!(meta_json(&meta_fixture.path, meta_account), meta_before);
    assert!(account_operation_logs(&meta_fixture.path, meta_account).is_empty());
    assert_eq!(
        current_open_token(&meta_fixture.path, meta_account),
        Some((
            token.generation.value() as i64,
            token.snapshot_id.as_str().to_string()
        ))
    );
    Connection::open(&meta_fixture.path)
        .unwrap()
        .execute_batch("DROP TRIGGER reject_poll_meta_after_audit;")
        .unwrap();
    let retry = meta_repository
        .apply_complete_snapshot(command)
        .await
        .expect("same token succeeds after post-audit meta trigger is removed");
    assert_eq!(retry.deactivated, 1);
    assert_eq!(
        account_operation_logs(&meta_fixture.path, meta_account).len(),
        1
    );
}

#[tokio::test]
async fn second_missing_audit_failure_rolls_back_all_rows_and_prior_audit() {
    let fixture = fresh_fixture("poll-second-audit-rollback", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "second-audit-rollback-account";
    let initial_token = begin(&repository, account, 100).await;
    complete(
        &repository,
        account,
        initial_token,
        101,
        vec![movie("a", "A", 10), movie("b", "B", 9)],
        default_policy(),
    )
    .await;
    seed_running_attempt(
        &fixture.path,
        account,
        "a",
        "queued",
        "movie_meta",
        "attempt-a",
        250,
    );
    seed_running_attempt(
        &fixture.path,
        account,
        "b",
        "searching",
        "movie_search",
        "attempt-b",
        250,
    );
    let token = begin(&repository, account, 200).await;
    Connection::open(&fixture.path)
        .unwrap()
        .execute_batch(
            r#"CREATE TRIGGER reject_second_poll_supersede_audit
               BEFORE INSERT ON operation_logs
               WHEN NEW.account_key = 'second-audit-rollback-account'
                AND NEW.action = 'supersede_attempt'
                AND NEW.target_id = 'b'
               BEGIN
                   SELECT RAISE(ABORT, 'reject second poll supersede audit');
               END;"#,
        )
        .unwrap();
    let a_before = row_json(&fixture.path, account, "a");
    let b_before = row_json(&fixture.path, account, "b");
    let meta_before = meta_json(&fixture.path, account);
    let command = ApplyCompleteSnapshotCommand::try_new(
        account,
        token.clone(),
        201,
        261,
        default_policy(),
        Vec::new(),
    )
    .unwrap();
    let error = repository
        .apply_complete_snapshot(command.clone())
        .await
        .unwrap_err();
    assert!(matches!(error, RepositoryError::CorruptData { .. }));
    assert_eq!(row_json(&fixture.path, account, "a"), a_before);
    assert_eq!(row_json(&fixture.path, account, "b"), b_before);
    assert_eq!(meta_json(&fixture.path, account), meta_before);
    assert!(account_operation_logs(&fixture.path, account).is_empty());
    assert_eq!(
        current_open_token(&fixture.path, account),
        Some((
            token.generation.value() as i64,
            token.snapshot_id.as_str().to_string()
        ))
    );
    Connection::open(&fixture.path)
        .unwrap()
        .execute_batch("DROP TRIGGER reject_second_poll_supersede_audit;")
        .unwrap();
    let retry = repository
        .apply_complete_snapshot(command)
        .await
        .expect("same token succeeds after second-audit trigger is removed");
    assert_eq!(retry.deactivated, 2);
    let audits = account_operation_logs(&fixture.path, account);
    assert_eq!(audits.len(), 2);
    assert_eq!(audits[0]["target_id"], "a");
    assert_eq!(audits[1]["target_id"], "b");
}

#[tokio::test]
async fn deactivation_count_mismatch_aborts_before_meta_consumption() {
    let fixture = fresh_fixture("poll-deactivation-count", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "deactivation-count-account";
    let initial_token = begin(&repository, account, 100).await;
    complete(
        &repository,
        account,
        initial_token,
        101,
        vec![movie("a", "A", 10), movie("b", "B", 9)],
        default_policy(),
    )
    .await;
    let token = begin(&repository, account, 200).await;
    Connection::open(&fixture.path)
        .unwrap()
        .execute_batch(
            r#"CREATE TRIGGER ignore_one_poll_deactivation
               BEFORE UPDATE OF active ON wanted_subscription_records
               WHEN OLD.account_key = 'deactivation-count-account'
                AND OLD.subject_id = 'b'
                AND OLD.active = 1
                AND NEW.active = 0
               BEGIN
                   SELECT RAISE(IGNORE);
               END;"#,
        )
        .unwrap();
    let a_before = row_json(&fixture.path, account, "a");
    let b_before = row_json(&fixture.path, account, "b");
    let meta_before = meta_json(&fixture.path, account);
    let command = ApplyCompleteSnapshotCommand::try_new(
        account,
        token.clone(),
        201,
        261,
        default_policy(),
        Vec::new(),
    )
    .unwrap();
    let error = repository
        .apply_complete_snapshot(command.clone())
        .await
        .unwrap_err();
    assert!(matches!(error, RepositoryError::Internal { .. }));
    assert_eq!(row_json(&fixture.path, account, "a"), a_before);
    assert_eq!(row_json(&fixture.path, account, "b"), b_before);
    assert_eq!(meta_json(&fixture.path, account), meta_before);
    assert!(account_operation_logs(&fixture.path, account).is_empty());
    assert_eq!(
        current_open_token(&fixture.path, account),
        Some((
            token.generation.value() as i64,
            token.snapshot_id.as_str().to_string()
        ))
    );
    Connection::open(&fixture.path)
        .unwrap()
        .execute_batch("DROP TRIGGER ignore_one_poll_deactivation;")
        .unwrap();
    let retry = repository
        .apply_complete_snapshot(command)
        .await
        .expect("same token succeeds after count-skew trigger is removed");
    assert_eq!(retry.deactivated, 2);
}

#[tokio::test]
async fn complete_rejects_missing_row_time_regression_and_terminal_rechecks_state_version() {
    let fixture = fresh_fixture("terminal-validation", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "terminal-validation-account";
    let initial_token = begin(&repository, account, 100).await;
    complete(
        &repository,
        account,
        initial_token,
        101,
        vec![movie("future", "Future Observation", 10)],
        default_policy(),
    )
    .await;
    let token = begin(&repository, account, 200).await;
    Connection::open(&fixture.path)
        .unwrap()
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET updated_at = 300,
                      record_json = json_set(record_json, '$.observation.last_seen_at', 300)
                WHERE account_key = ?1 AND subject_id = 'future'"#,
            [account],
        )
        .unwrap();
    let row_before = row_json(&fixture.path, account, "future");
    let meta_before = meta_json(&fixture.path, account);
    let backwards = repository
        .apply_complete_snapshot(
            ApplyCompleteSnapshotCommand::try_new(
                account,
                token.clone(),
                201,
                261,
                default_policy(),
                Vec::new(),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(
        backwards,
        RepositoryError::InvalidInput {
            field: "completed_at",
            ..
        }
    ));
    assert_eq!(row_json(&fixture.path, account, "future"), row_before);
    assert_eq!(meta_json(&fixture.path, account), meta_before);
    assert_eq!(
        current_open_token(&fixture.path, account),
        Some((
            token.generation.value() as i64,
            token.snapshot_id.as_str().to_string()
        ))
    );
    let recovered = repository
        .apply_complete_snapshot(
            ApplyCompleteSnapshotCommand::try_new(
                account,
                token,
                300,
                360,
                default_policy(),
                Vec::new(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(recovered.deactivated, 1);

    let version_account = "terminal-version-account";
    let version_token = begin(&repository, version_account, 400).await;
    Connection::open(&fixture.path)
        .unwrap()
        .execute(
            "UPDATE subscription_meta SET state_version = 2 WHERE account_key = ?1",
            [version_account],
        )
        .unwrap();
    let before = meta_json(&fixture.path, version_account);
    let version_error = repository
        .record_poll_failure(
            RecordPollFailureCommand::try_new(
                version_account,
                version_token.clone(),
                401,
                "failure",
                retry_policy(),
            )
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert!(matches!(version_error, RepositoryError::CorruptData { .. }));
    assert_eq!(meta_json(&fixture.path, version_account), before);
    assert_eq!(
        current_open_token(&fixture.path, version_account),
        Some((
            version_token.generation.value() as i64,
            version_token.snapshot_id.as_str().to_string()
        ))
    );
}

#[tokio::test]
async fn failure_count_saturates_at_u32_max_without_sql_overflow() {
    let fixture = fresh_fixture("failure-saturation", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let account = "failure-saturation-account";
    let token = begin(&repository, account, 100).await;
    Connection::open(&fixture.path)
        .unwrap()
        .execute(
            "UPDATE subscription_meta SET poll_failure_count = ?2, last_poll_error = 'previous failure' WHERE account_key = ?1",
            params![account, i64::from(u32::MAX)],
        )
        .unwrap();
    let result = repository
        .record_poll_failure(
            RecordPollFailureCommand::try_new(
                account,
                token,
                101,
                "still failing",
                PollRetryPolicy::try_new(1, 1).unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.failure_count, u32::MAX);
    assert_eq!(result.next_poll_at, 102);
    let stored: i64 = Connection::open(&fixture.path)
        .unwrap()
        .query_row(
            "SELECT poll_failure_count FROM subscription_meta WHERE account_key = ?1",
            [account],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, i64::from(u32::MAX));
}

#[tokio::test]
async fn poll_writes_leave_operation_logs_and_adjacent_accounts_untouched_and_use_bounded_scope() {
    let fixture = fresh_fixture("poll-isolation", ROWS_ONLY).await;
    let repository = repository(&fixture.path);
    let connection = Connection::open(&fixture.path).unwrap();
    connection
        .execute(
            r#"INSERT INTO operation_logs (
                   account_key, created_at, category, action, target_type, target_id,
                   target_title, status, summary, error, related_json
               ) VALUES (?1, 100, 'fixture', 'seed', 'subscription', 'seed-id',
                         'Seed', 'success', 'must remain unchanged', NULL, '{}')"#,
            [FIXTURE_ACCOUNT],
        )
        .unwrap();
    drop(connection);
    let account = "isolated-poll-account";
    let token = begin(&repository, account, 100).await;
    let isolation_snapshot = || {
        let connection = Connection::open(&fixture.path).unwrap();
        let logs: String = connection
            .query_row(
                "SELECT json_group_array(json_array(id, account_key, created_at, category, action, target_type, target_id, target_title, status, summary, error, related_json)) FROM operation_logs",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let adjacent: String = connection
            .query_row(
                "SELECT json_group_array(json_array(subject_id, revision, active, record_json)) FROM wanted_subscription_records WHERE account_key = ?1 ORDER BY subject_id",
                [FIXTURE_ACCOUNT],
                |row| row.get(0),
            )
            .unwrap();
        (logs, adjacent)
    };
    let before = isolation_snapshot();
    complete(
        &repository,
        account,
        token,
        101,
        vec![
            movie("isolated", "Isolated", 10),
            tv("isolated-tv", "TV", 9),
        ],
        default_policy(),
    )
    .await;
    assert_eq!(isolation_snapshot(), before);

    assert!(DEACTIVATE_MISSING_SQL.contains("WHERE account_key = ?1"));
    assert!(DEACTIVATE_MISSING_SQL.contains("active = 1"));
    assert!(DEACTIVATE_MISSING_SQL.contains("last_seen_snapshot_id IS NULL"));
    let connection = Connection::open(&fixture.path).unwrap();
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {DEACTIVATE_MISSING_SQL}"))
        .unwrap();
    let plan = statement
        .query_map(
            params![
                account,
                "snapshot",
                200_i64,
                TV_NOT_SUPPORTED_REASON,
                INACTIVE_SUBSCRIPTION_REASON,
            ],
            |row| row.get::<_, String>(3),
        )
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert!(plan.iter().any(|detail| {
        detail.contains("SEARCH wanted_subscription_records")
            && detail.contains("account_key=? AND active=?")
    }));
    assert!(plan.iter().all(|detail| !detail.contains("TEMP B-TREE")));
}
