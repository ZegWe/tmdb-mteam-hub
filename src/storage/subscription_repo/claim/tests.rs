use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier as ThreadBarrier, Mutex};
use std::time::Duration;

use rusqlite::types::Value as SqlValue;
use rusqlite::{params, params_from_iter, Connection, StatementStatus};
use serde_json::Value;
use tokio::sync::Barrier;

use super::super::{
    build_list_queries,
    evidence_support::{insert_generated_clones, unrelated_storage_snapshot},
};
use super::{
    ClaimDependencies, ExecutionAttemptIdSource, RepositoryClock, DUE_CANDIDATE_SQL,
    EXPIRED_CANDIDATE_SQL, FORCE_CANDIDATE_SQL,
};
use crate::storage::SqliteSubscriptionRepository;
use crate::subscription::ports::{
    SubscriptionExecutionRepository, SubscriptionMutationRepository, SubscriptionPollRepository,
    SubscriptionReadRepository,
};
use crate::subscription::repository::payload::{CandidateMatchPayload, CandidatePayload};
use crate::subscription::repository::{
    ApplyCompleteSnapshotCommand, BeginPollCommand, ClaimDueCommand, ClaimDueResult,
    ClaimOneCommand, ClaimOneResult, ClaimRejection, ExecutionAttemptId, ExecutionAttemptToken,
    ExecutionPayloadDelta, ExecutionScheduleDelay, ExtendExecutionLeaseCommand,
    FailExecutionCommand, FinishExecutionCommand, FinishExecutionDisposition,
    IncompleteSnapshotObservation, IncompleteSnapshotReason, ListSubscriptionsCommand,
    NewRecordPolicy, PollRetryPolicy, RecordIncompleteSnapshotCommand, ReleaseExecutionCommand,
    RepositoryError, RepositoryResult, SnapshotRecord, SubscriptionKey, SubscriptionListFilter,
    UpdateSubscriptionDetailCommand, WantedSourcePayload,
};
use crate::subscription::{
    SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
    SubscriptionMediaKind,
};

const ACCOUNT: &str = "fixture_rows_only";
const BASE_SUBJECT: &str = "rows-movie-001";
const NOW: u64 = 1_800_000_000;
const RACE_AT: u64 = NOW + 10;
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);

const CLONE_SUBJECT_SQL: &str = r#"
INSERT INTO wanted_subscription_records (
    account_key, subject_id, revision, active, inactive_at, last_seen_snapshot_id,
    media_kind, schedulable, blocked_reason, lifecycle_state, execution_state,
    next_attempt_at, retry_count, max_retries, retry_blocked, force_eligible_once,
    claimed_operation, attempt_id, lease_until, title, release_year, poster_url,
    category_text, douban_sort_time, attention_tags_json, updated_at, record_json
)
SELECT account_key, ?2, revision, active, inactive_at, last_seen_snapshot_id,
       media_kind, schedulable, blocked_reason, lifecycle_state, execution_state,
       next_attempt_at, retry_count, max_retries, retry_blocked, force_eligible_once,
       claimed_operation, attempt_id, lease_until, title, release_year, poster_url,
       category_text, douban_sort_time, attention_tags_json, updated_at, record_json
  FROM wanted_subscription_records
 WHERE account_key = ?1 AND subject_id = ?3
"#;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    path: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-v5-claim-repo-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create claim repository fixture directory");
        let path = root.join("subscriptions.sqlite");
        Self { root, path }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

async fn fresh_fixture(label: &str) -> Fixture {
    let fixture = Fixture::new(label);
    let path = fixture.path.clone();
    let repository = SqliteSubscriptionRepository::try_create_fresh(&path, 1, BUSY_TIMEOUT)
        .expect("create fresh latest-schema claim repository");
    let token = repository
        .begin_poll(BeginPollCommand::try_new(ACCOUNT, NOW).unwrap())
        .await
        .expect("begin fresh claim fixture snapshot")
        .token;
    repository
        .apply_complete_snapshot(
            ApplyCompleteSnapshotCommand::try_new(
                ACCOUNT,
                token,
                NOW,
                NOW + 60,
                NewRecordPolicy::try_new(3, false).unwrap(),
                vec![
                    movie_snapshot(BASE_SUBJECT, "Fixture Rows Queued Movie", NOW),
                    movie_snapshot("rows-movie-002", "Fixture Rows Completed Movie", NOW - 1),
                ],
            )
            .unwrap(),
        )
        .await
        .expect("seed fresh claim fixture snapshot");
    Connection::open(&path)
        .expect("open fresh claim fixture for completed seed")
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET lifecycle_state = 'completed', next_attempt_at = NULL
                WHERE account_key = ?1 AND subject_id = 'rows-movie-002'"#,
            [ACCOUNT],
        )
        .expect("seed completed fresh claim fixture row");
    fixture
}

fn movie_snapshot(subject_id: &str, title: &str, sort_time: u64) -> SnapshotRecord {
    SnapshotRecord::try_new(
        subject_id,
        SubscriptionMediaKind::Movie,
        true,
        None,
        WantedSourcePayload {
            title: title.to_string(),
            poster_url: format!("https://example.test/{subject_id}.jpg"),
            category_text: Some("fixture-movie".to_string()),
            douban_sort_time: Some(sort_time),
            tags: vec!["movie".to_string(), "fixture".to_string()],
            ..WantedSourcePayload::default()
        },
    )
    .expect("build fresh movie snapshot record")
}

#[derive(Debug)]
struct FixedClock {
    now: AtomicU64,
}

impl FixedClock {
    fn new(now: u64) -> Self {
        Self {
            now: AtomicU64::new(now),
        }
    }

    fn set(&self, now: u64) {
        self.now.store(now, Ordering::SeqCst);
    }
}

impl RepositoryClock for FixedClock {
    fn now_unix_seconds(&self) -> RepositoryResult<u64> {
        Ok(self.now.load(Ordering::SeqCst))
    }
}

#[derive(Debug)]
struct SequenceAttemptIds {
    values: Mutex<VecDeque<String>>,
    calls: AtomicUsize,
}

impl SequenceAttemptIds {
    fn new(values: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            values: Mutex::new(values.into_iter().map(Into::into).collect()),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl ExecutionAttemptIdSource for SequenceAttemptIds {
    fn next_attempt_id(&self) -> RepositoryResult<ExecutionAttemptId> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let value = self
            .values
            .lock()
            .map_err(|_| RepositoryError::Internal {
                message: "test attempt ID queue is poisoned".to_string(),
            })?
            .pop_front()
            .ok_or_else(|| RepositoryError::Unavailable {
                message: "test attempt ID queue is exhausted".to_string(),
            })?;
        ExecutionAttemptId::try_new(value)
    }
}

fn repository(
    path: &Path,
    clock: Arc<FixedClock>,
    attempt_ids: Arc<SequenceAttemptIds>,
) -> SqliteSubscriptionRepository {
    SqliteSubscriptionRepository::try_new_with_claim_dependencies(
        path,
        1,
        BUSY_TIMEOUT,
        ClaimDependencies::new(clock, attempt_ids),
    )
    .expect("construct injected staged claim repository")
}

fn clone_subject(connection: &Connection, subject_id: &str) {
    let changed = connection
        .execute(
            CLONE_SUBJECT_SQL,
            params![ACCOUNT, subject_id, BASE_SUBJECT],
        )
        .expect("clone claim fixture subject");
    assert_eq!(changed, 1);
}

#[derive(Debug)]
struct QueryWork {
    subjects: Vec<String>,
    fullscan_steps: i32,
    sort_operations: i32,
    automatic_indexes: i32,
    virtual_machine_steps: i32,
}

fn measured_subject_query(
    connection: &Connection,
    sql: &str,
    values: &[SqlValue],
    subject_column: usize,
) -> QueryWork {
    let mut statement = connection
        .prepare(sql)
        .expect("prepare measured subscription query");
    let subjects = {
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                row.get::<_, String>(subject_column)
            })
            .expect("execute measured subscription query");
        rows.collect::<Result<Vec<_>, _>>()
            .expect("decode measured subscription subjects")
    };
    QueryWork {
        subjects,
        fullscan_steps: statement.get_status(StatementStatus::FullscanStep),
        sort_operations: statement.get_status(StatementStatus::Sort),
        automatic_indexes: statement.get_status(StatementStatus::AutoIndex),
        virtual_machine_steps: statement.get_status(StatementStatus::VmStep),
    }
}

fn explain_query(connection: &Connection, sql: &str, values: &[SqlValue]) -> Vec<String> {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .expect("prepare subscription query plan");
    statement
        .query_map(params_from_iter(values.iter()), |row| {
            row.get::<_, String>(3)
        })
        .expect("execute subscription query plan")
        .collect::<Result<Vec<_>, _>>()
        .expect("decode subscription query plan")
}

fn assert_bounded_index_work(work: &QueryWork, maximum_rows: usize, label: &str) {
    assert!(
        work.subjects.len() <= maximum_rows,
        "{label} returned more than its SQL limit: {work:?}"
    );
    assert_eq!(
        work.fullscan_steps, 0,
        "{label} performed full-scan steps: {work:?}"
    );
    assert_eq!(
        work.sort_operations, 0,
        "{label} performed a SQLite sort: {work:?}"
    );
    assert_eq!(
        work.automatic_indexes, 0,
        "{label} fell back to a transient automatic index: {work:?}"
    );
    assert!(
        work.virtual_machine_steps < 2_000,
        "{label} exceeded the generous constant-work VM-step ceiling: {work:?}"
    );
}

fn key(subject_id: &str) -> SubscriptionKey {
    SubscriptionKey::try_new(ACCOUNT, subject_id).expect("valid fixture subscription key")
}

fn claim_due_command() -> ClaimDueCommand {
    ClaimDueCommand::try_new(ACCOUNT, 60, 1).expect("valid bounded due claim command")
}

fn claim_one_command(subject_id: &str) -> ClaimOneCommand {
    ClaimOneCommand::try_new(key(subject_id), 60).expect("valid manual claim command")
}

async fn claim_token(
    repository: &SqliteSubscriptionRepository,
    subject_id: &str,
) -> ExecutionAttemptToken {
    let claimed = repository
        .claim_one(claim_one_command(subject_id))
        .await
        .unwrap_or_else(|error| panic!("claim {subject_id}: {error}"));
    let ClaimOneResult::Claimed(claimed) = claimed else {
        panic!("{subject_id} must be claimable");
    };
    claimed.attempt().token().clone()
}

fn candidate(torrent_id: &str, title: &str) -> CandidateMatchPayload {
    CandidateMatchPayload {
        candidate: CandidatePayload {
            torrent_id: torrent_id.to_string(),
            title: title.to_string(),
            source: "mteam".to_string(),
            search_query: "fixture query".to_string(),
            ..CandidatePayload::default()
        },
        ..CandidateMatchPayload::default()
    }
}

fn audit_rows(path: &Path) -> Vec<(String, String, String, Value)> {
    let connection = Connection::open(path).expect("open claim audit fixture");
    let mut statement = connection
        .prepare(
            r#"SELECT action, status, target_id, related_json
                 FROM operation_logs
                WHERE category = 'subscription_scheduler'
                ORDER BY id"#,
        )
        .expect("prepare claim audit query");
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .expect("query claim audit rows")
        .map(|row| {
            let (action, status, target_id, related) = row.expect("decode claim audit row");
            (
                action,
                status,
                target_id,
                serde_json::from_str(&related).expect("decode claim audit related JSON"),
            )
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RowControls {
    revision: i64,
    execution_state: String,
    claimed_operation: Option<String>,
    attempt_id: Option<String>,
    lease_until: Option<i64>,
    force_eligible_once: i64,
    next_attempt_at: Option<i64>,
}

fn row_controls(path: &Path, subject_id: &str) -> RowControls {
    Connection::open(path)
        .expect("open claim controls fixture")
        .query_row(
            r#"SELECT revision, execution_state, claimed_operation, attempt_id, lease_until,
                      force_eligible_once, next_attempt_at
                 FROM wanted_subscription_records
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, subject_id],
            |row| {
                Ok(RowControls {
                    revision: row.get(0)?,
                    execution_state: row.get(1)?,
                    claimed_operation: row.get(2)?,
                    attempt_id: row.get(3)?,
                    lease_until: row.get(4)?,
                    force_eligible_once: row.get(5)?,
                    next_attempt_at: row.get(6)?,
                })
            },
        )
        .expect("read claim controls")
}

fn current_open_poll_token(path: &Path) -> Option<(i64, String)> {
    let (generation, snapshot_id) = Connection::open(path)
        .expect("open poll token fixture")
        .query_row(
            r#"SELECT open_poll_generation, open_snapshot_id
                 FROM subscription_meta
                WHERE account_key = ?1"#,
            [ACCOUNT],
            |row| {
                Ok((
                    row.get::<_, Option<i64>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                ))
            },
        )
        .expect("read open poll token");
    generation.zip(snapshot_id)
}

fn race_poll_terminal_and_claim<T, F>(
    path: &Path,
    claim_dependencies: ClaimDependencies,
    poll_terminal: F,
) -> (RepositoryResult<T>, RepositoryResult<ClaimDueResult>)
where
    T: Send + 'static,
    F: FnOnce(&mut Connection) -> RepositoryResult<T> + Send + 'static,
{
    let barrier = Arc::new(ThreadBarrier::new(3));
    let poll_path = path.to_path_buf();
    let poll_barrier = Arc::clone(&barrier);
    let poll = std::thread::spawn(move || {
        let mut connection = crate::storage::sqlite::open_v5_connection(&poll_path, BUSY_TIMEOUT)
            .expect("open physical Poll race connection");
        poll_barrier.wait();
        poll_terminal(&mut connection)
    });

    let claim_path = path.to_path_buf();
    let claim_barrier = Arc::clone(&barrier);
    let claim = std::thread::spawn(move || {
        let mut connection = crate::storage::sqlite::open_v5_connection(&claim_path, BUSY_TIMEOUT)
            .expect("open physical claim race connection");
        claim_barrier.wait();
        super::claim_due(&mut connection, claim_due_command(), claim_dependencies)
    });

    // Releasing this barrier proves that both independent physical connections are open before
    // either adapter enters its own BEGIN IMMEDIATE transaction.
    barrier.wait();
    (
        poll.join().expect("join physical Poll race thread"),
        claim.join().expect("join physical claim race thread"),
    )
}

#[tokio::test]
async fn claim_due_uses_expired_then_force_then_normal_priority_and_atomic_audits() {
    let fixture = fresh_fixture("priority").await;
    let connection = Connection::open(&fixture.path).expect("open priority fixture");
    clone_subject(&connection, "forced");
    clone_subject(&connection, "expired");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET next_attempt_at = ?3,
                      retry_count = 3,
                      retry_blocked = 1,
                      force_eligible_once = 1,
                      attention_tags_json = '["skipped"]',
                      record_json = json_set(record_json, '$.skip_reason', 'forced retry')
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "forced", NOW as i64 + 10_000],
        )
        .expect("seed forced candidate");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET lifecycle_state = 'searching',
                      execution_state = 'running',
                      claimed_operation = 'movie_search',
                      attempt_id = 'attempt-expired-old',
                      lease_until = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "expired", NOW as i64],
        )
        .expect("seed equality-expired candidate");
    drop(connection);

    let clock = Arc::new(FixedClock::new(NOW));
    let attempt_ids = Arc::new(SequenceAttemptIds::new([
        "attempt-expired-new",
        "attempt-forced",
        "attempt-normal",
    ]));
    let repository = repository(&fixture.path, clock, Arc::clone(&attempt_ids));

    let expired = repository
        .claim_due(claim_due_command())
        .await
        .expect("reclaim equality-expired attempt")
        .into_claim()
        .expect("expired branch must win");
    assert_eq!(expired.detail().summary().head.key.subject_id, "expired");
    let previous = expired
        .replaced_expired_attempt()
        .expect("reclaim must return old fencing token");
    assert_eq!(
        previous.token().attempt_id().as_str(),
        "attempt-expired-old"
    );
    assert_eq!(previous.lease_until(), NOW);
    assert_eq!(expired.attempt().lease_until(), NOW + 60);

    let forced = repository
        .claim_due(claim_due_command())
        .await
        .expect("claim forced candidate")
        .into_claim()
        .expect("force branch must beat normal due");
    assert_eq!(forced.detail().summary().head.key.subject_id, "forced");
    assert!(forced.detail().summary().head.force_eligible_once);
    assert!(forced.detail().summary().head.retry_blocked);
    assert!(forced
        .detail()
        .summary()
        .attention_tags
        .contains(&SubscriptionAttentionTag::Skipped));
    assert_eq!(
        forced.detail().payload().skip_reason.as_deref(),
        Some("forced retry")
    );
    assert_eq!(
        forced.detail().summary().head.next_attempt_at,
        Some(NOW + 10_000)
    );

    let normal = repository
        .claim_due(claim_due_command())
        .await
        .expect("claim normal candidate")
        .into_claim()
        .expect("normal candidate must remain claimable");
    assert_eq!(normal.detail().summary().head.key.subject_id, BASE_SUBJECT);
    assert_eq!(attempt_ids.calls(), 3);

    let forced_controls = row_controls(&fixture.path, "forced");
    assert_eq!(forced_controls.execution_state, "running");
    assert_eq!(
        forced_controls.claimed_operation.as_deref(),
        Some("movie_meta")
    );
    assert_eq!(
        forced_controls.attempt_id.as_deref(),
        Some("attempt-forced")
    );
    assert_eq!(forced_controls.lease_until, Some((NOW + 60) as i64));
    assert_eq!(forced_controls.force_eligible_once, 1);
    assert_eq!(forced_controls.next_attempt_at, Some((NOW + 10_000) as i64));

    let audits = audit_rows(&fixture.path);
    assert_eq!(audits.len(), 3);
    assert_eq!(audits[0].0, "reclaim_attempt");
    assert_eq!(audits[0].1, "success");
    assert_eq!(audits[0].2, "expired");
    assert_eq!(audits[0].3["trigger"], "claim_due");
    assert_eq!(audits[0].3["eligibility"], "expired_lease");
    assert_eq!(
        audits[0].3["previous_attempt"]["attempt_id"],
        "attempt-expired-old"
    );
    assert_eq!(audits[1].3["eligibility"], "force_once");
    assert_eq!(audits[2].3["eligibility"], "normal_due");
}

#[tokio::test]
async fn lease_greater_than_now_is_live_and_equality_is_reclaimable_by_claim_one() {
    let fixture = fresh_fixture("lease-boundary").await;
    let connection = Connection::open(&fixture.path).expect("open lease boundary fixture");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET execution_state = 'running',
                      claimed_operation = 'movie_meta',
                      attempt_id = 'attempt-live',
                      lease_until = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, BASE_SUBJECT, NOW as i64 + 1],
        )
        .expect("seed live attempt");
    drop(connection);

    let clock = Arc::new(FixedClock::new(NOW));
    let attempt_ids = Arc::new(SequenceAttemptIds::new(["attempt-reclaimed"]));
    let repository = repository(&fixture.path, Arc::clone(&clock), Arc::clone(&attempt_ids));
    let live = repository
        .claim_one(claim_one_command(BASE_SUBJECT))
        .await
        .expect("classify live attempt");
    let ClaimOneResult::Rejected(ClaimRejection::LiveAttempt { current }) = live else {
        panic!("lease greater than repository now must be live");
    };
    assert_eq!(current.token().attempt_id().as_str(), "attempt-live");
    assert_eq!(current.lease_until(), NOW + 1);
    assert_eq!(attempt_ids.calls(), 0);
    assert!(audit_rows(&fixture.path).is_empty());

    clock.set(NOW + 1);
    let reclaimed = repository
        .claim_one(claim_one_command(BASE_SUBJECT))
        .await
        .expect("reclaim at lease equality");
    let ClaimOneResult::Claimed(reclaimed) = reclaimed else {
        panic!("lease equality must be expired");
    };
    assert_eq!(
        reclaimed
            .replaced_expired_attempt()
            .expect("old attempt must be returned")
            .lease_until(),
        NOW + 1
    );
    assert_eq!(reclaimed.attempt().lease_until(), NOW + 61);
    assert_eq!(attempt_ids.calls(), 1);
}

#[tokio::test]
async fn claim_one_returns_typed_rejections_without_nonce_or_audit() {
    let fixture = fresh_fixture("typed-rejections").await;
    let connection = Connection::open(&fixture.path).expect("open rejection fixture");
    for subject in [
        "inactive",
        "blocked",
        "tv",
        "not-due",
        "retry",
        "tag-skip",
        "payload-skip",
    ] {
        clone_subject(&connection, subject);
    }
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET active = 0, inactive_at = updated_at, schedulable = 0,
                      blocked_reason = 'subscription_inactive', next_attempt_at = NULL
                WHERE account_key = ?1 AND subject_id = 'inactive'"#,
            [ACCOUNT],
        )
        .expect("seed inactive rejection");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET schedulable = 0, blocked_reason = 'maintenance', next_attempt_at = NULL
                WHERE account_key = ?1 AND subject_id = 'blocked'"#,
            [ACCOUNT],
        )
        .expect("seed blocked rejection");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET media_kind = 'tv', schedulable = 0,
                      blocked_reason = 'tv_not_supported', next_attempt_at = NULL
                WHERE account_key = ?1 AND subject_id = 'tv'"#,
            [ACCOUNT],
        )
        .expect("seed TV rejection");
    connection
        .execute(
            "UPDATE wanted_subscription_records SET next_attempt_at = ?2 WHERE account_key = ?1 AND subject_id = 'not-due'",
            params![ACCOUNT, NOW as i64 + 100],
        )
        .expect("seed not-due rejection");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET retry_count = 3, retry_blocked = 1
                WHERE account_key = ?1 AND subject_id = 'retry'"#,
            [ACCOUNT],
        )
        .expect("seed retry-blocked rejection");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET attention_tags_json = '["skipped"]', next_attempt_at = NULL
                WHERE account_key = ?1 AND subject_id = 'tag-skip'"#,
            [ACCOUNT],
        )
        .expect("seed tag-only skip rejection");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET record_json = json_set(record_json, '$.skip_reason', 'manual skip'),
                      next_attempt_at = NULL
                WHERE account_key = ?1 AND subject_id = 'payload-skip'"#,
            [ACCOUNT],
        )
        .expect("seed payload-only skip rejection");
    drop(connection);

    let clock = Arc::new(FixedClock::new(NOW));
    let attempt_ids = Arc::new(SequenceAttemptIds::new(Vec::<String>::new()));
    let repository = repository(&fixture.path, clock, Arc::clone(&attempt_ids));
    for (subject, expected) in [
        ("inactive", "inactive"),
        ("blocked", "blocked"),
        ("tv", "tv"),
        ("not-due", "not-due"),
        ("retry", "retry"),
        ("tag-skip", "skipped"),
        ("payload-skip", "skipped"),
        ("rows-movie-002", "completed"),
    ] {
        let result = repository
            .claim_one(claim_one_command(subject))
            .await
            .unwrap_or_else(|error| panic!("classify {subject}: {error}"));
        let matched = matches!(
            (expected, result),
            (
                "inactive",
                ClaimOneResult::Rejected(ClaimRejection::Inactive)
            ) | (
                "blocked",
                ClaimOneResult::Rejected(ClaimRejection::Unschedulable { .. })
            ) | (
                "tv",
                ClaimOneResult::Rejected(ClaimRejection::Unschedulable { .. })
            ) | (
                "not-due",
                ClaimOneResult::Rejected(ClaimRejection::NotDue { .. })
            ) | (
                "retry",
                ClaimOneResult::Rejected(ClaimRejection::RetryBlocked)
            ) | ("skipped", ClaimOneResult::Rejected(ClaimRejection::Skipped))
                | (
                    "completed",
                    ClaimOneResult::Rejected(ClaimRejection::Completed)
                )
        );
        assert!(matched, "unexpected rejection for {subject}");
    }
    let missing = repository
        .claim_one(claim_one_command("missing"))
        .await
        .expect_err("missing manual claim must remain a repository error");
    assert!(matches!(missing, RepositoryError::NotFound { .. }));
    assert_eq!(attempt_ids.calls(), 0);
    assert!(audit_rows(&fixture.path).is_empty());
}

#[tokio::test]
async fn attempt_id_collisions_retry_same_candidate_and_old_reclaim_id_is_not_reused() {
    let fixture = fresh_fixture("collision-retry").await;
    let connection = Connection::open(&fixture.path).expect("open collision fixture");
    clone_subject(&connection, "owner");
    clone_subject(&connection, "expired");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET execution_state = 'running', claimed_operation = 'movie_meta',
                      attempt_id = 'attempt-taken', lease_until = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "owner", NOW as i64 + 100],
        )
        .expect("seed globally taken attempt ID");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET execution_state = 'running', claimed_operation = 'movie_meta',
                      attempt_id = 'attempt-expired-old', lease_until = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "expired", NOW as i64],
        )
        .expect("seed expired old attempt ID");
    drop(connection);

    let clock = Arc::new(FixedClock::new(NOW));
    let attempt_ids = Arc::new(SequenceAttemptIds::new([
        "attempt-taken",
        "attempt-fresh",
        "attempt-expired-old",
        "attempt-reclaim-fresh",
    ]));
    let repository = repository(&fixture.path, clock, Arc::clone(&attempt_ids));
    let before_revision = row_controls(&fixture.path, BASE_SUBJECT).revision;
    let claimed = repository
        .claim_one(claim_one_command(BASE_SUBJECT))
        .await
        .expect("retry unique collision");
    let ClaimOneResult::Claimed(claimed) = claimed else {
        panic!("idle target must be claimed after collision retry");
    };
    assert_eq!(
        claimed.attempt().token().attempt_id().as_str(),
        "attempt-fresh"
    );
    assert_eq!(
        row_controls(&fixture.path, BASE_SUBJECT).revision,
        before_revision + 1,
        "failed collision statements must not increment revision"
    );

    let reclaimed = repository
        .claim_one(claim_one_command("expired"))
        .await
        .expect("retry old-ID reclaim collision");
    let ClaimOneResult::Claimed(reclaimed) = reclaimed else {
        panic!("expired target must be reclaimed");
    };
    assert_eq!(
        reclaimed.attempt().token().attempt_id().as_str(),
        "attempt-reclaim-fresh"
    );
    assert_eq!(attempt_ids.calls(), 4);
    assert_eq!(audit_rows(&fixture.path).len(), 2);
}

#[tokio::test]
async fn collision_exhaustion_and_audit_failure_roll_back_every_claim_write() {
    let collision_fixture = fresh_fixture("collision-exhaustion").await;
    let connection =
        Connection::open(&collision_fixture.path).expect("open collision exhaustion fixture");
    clone_subject(&connection, "owner");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET execution_state = 'running', claimed_operation = 'movie_meta',
                      attempt_id = 'attempt-always-taken', lease_until = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "owner", NOW as i64 + 100],
        )
        .expect("seed collision exhaustion owner");
    drop(connection);
    let before = row_controls(&collision_fixture.path, BASE_SUBJECT);
    let collision_ids = Arc::new(SequenceAttemptIds::new(std::iter::repeat_n(
        "attempt-always-taken",
        8,
    )));
    let collision_repository = repository(
        &collision_fixture.path,
        Arc::new(FixedClock::new(NOW)),
        Arc::clone(&collision_ids),
    );
    let error = collision_repository
        .claim_one(claim_one_command(BASE_SUBJECT))
        .await
        .expect_err("bounded collision exhaustion must fail");
    assert!(matches!(error, RepositoryError::Unavailable { .. }));
    assert_eq!(collision_ids.calls(), 8);
    assert_eq!(row_controls(&collision_fixture.path, BASE_SUBJECT), before);
    assert!(audit_rows(&collision_fixture.path).is_empty());

    let audit_fixture = fresh_fixture("audit-rollback").await;
    let connection = Connection::open(&audit_fixture.path).expect("open audit rollback fixture");
    connection
        .execute_batch(
            r#"CREATE TRIGGER reject_execution_audit
               BEFORE INSERT ON operation_logs
               WHEN NEW.category = 'subscription_scheduler'
               BEGIN
                   SELECT RAISE(ABORT, 'reject execution audit');
               END;"#,
        )
        .expect("install audit rejection trigger");
    drop(connection);
    let before = row_controls(&audit_fixture.path, BASE_SUBJECT);
    let audit_ids = Arc::new(SequenceAttemptIds::new(["attempt-audit-rollback"]));
    let audit_repository = repository(
        &audit_fixture.path,
        Arc::new(FixedClock::new(NOW)),
        Arc::clone(&audit_ids),
    );
    let error = audit_repository
        .claim_one(claim_one_command(BASE_SUBJECT))
        .await
        .expect_err("audit failure must abort claim");
    assert!(matches!(error, RepositoryError::CorruptData { .. }));
    assert_eq!(row_controls(&audit_fixture.path, BASE_SUBJECT), before);
    assert!(audit_rows(&audit_fixture.path).is_empty());
}

#[tokio::test]
async fn due_collision_exhaustion_keeps_first_and_second_candidates_for_later_bounded_calls() {
    let fixture = fresh_fixture("due-collision-contract").await;
    let connection = Connection::open(&fixture.path).expect("open due collision fixture");
    clone_subject(&connection, "later-valid");
    clone_subject(&connection, "attempt-owner");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET next_attempt_at = CASE subject_id
                      WHEN 'rows-movie-001' THEN ?2
                      WHEN 'later-valid' THEN ?3
                  END
                WHERE account_key = ?1
                  AND subject_id IN ('rows-movie-001', 'later-valid')"#,
            params![ACCOUNT, NOW as i64 - 1, NOW as i64],
        )
        .expect("order two eligible due candidates");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET execution_state = 'running', claimed_operation = 'movie_meta',
                      attempt_id = 'attempt-always-taken', lease_until = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "attempt-owner", NOW as i64 + 100],
        )
        .expect("seed globally occupied attempt ID");
    drop(connection);

    let first_before = row_controls(&fixture.path, BASE_SUBJECT);
    let second_before = row_controls(&fixture.path, "later-valid");
    let exhausted_ids = Arc::new(SequenceAttemptIds::new(std::iter::repeat_n(
        "attempt-always-taken",
        8,
    )));
    let exhausted_repository = repository(
        &fixture.path,
        Arc::new(FixedClock::new(NOW)),
        Arc::clone(&exhausted_ids),
    );
    let error = exhausted_repository
        .claim_due(claim_due_command())
        .await
        .expect_err("bounded due claim must report attempt-ID exhaustion");
    assert!(matches!(error, RepositoryError::Unavailable { .. }));
    assert_eq!(exhausted_ids.calls(), 8);
    assert_eq!(row_controls(&fixture.path, BASE_SUBJECT), first_before);
    assert_eq!(
        row_controls(&fixture.path, "later-valid"),
        second_before,
        "limit-one claim must not skip to the second eligible row after allocation failure"
    );
    assert!(audit_rows(&fixture.path).is_empty());

    let recovery_ids = Arc::new(SequenceAttemptIds::new([
        "attempt-recovery-first",
        "attempt-recovery-second",
    ]));
    let recovery_repository = repository(
        &fixture.path,
        Arc::new(FixedClock::new(NOW)),
        Arc::clone(&recovery_ids),
    );
    let first = recovery_repository
        .claim_due(claim_due_command())
        .await
        .expect("retry first bounded due claim")
        .into_claim()
        .expect("first candidate remains eligible after exhaustion rollback");
    assert_eq!(first.detail().summary().head.key.subject_id, BASE_SUBJECT);
    let second = recovery_repository
        .claim_due(claim_due_command())
        .await
        .expect("run second bounded due claim")
        .into_claim()
        .expect("second candidate remains eligible for the next call");
    assert_eq!(second.detail().summary().head.key.subject_id, "later-valid");
    assert_eq!(recovery_ids.calls(), 2);
    assert_eq!(audit_rows(&fixture.path).len(), 2);
}

#[tokio::test]
async fn skipped_due_drift_is_ignored_without_blocking_a_later_valid_candidate() {
    for (label, mutation) in [
        (
            "tag-skip-drift",
            r#"UPDATE wanted_subscription_records
                  SET attention_tags_json = '["skipped"]'
                WHERE account_key = 'fixture_rows_only' AND subject_id = 'rows-movie-001'"#,
        ),
        (
            "payload-skip-drift",
            r#"UPDATE wanted_subscription_records
                  SET record_json = json_set(record_json, '$.skip_reason', 'manual skip')
                WHERE account_key = 'fixture_rows_only' AND subject_id = 'rows-movie-001'"#,
        ),
    ] {
        let fixture = fresh_fixture(label).await;
        let connection = Connection::open(&fixture.path).expect("open poison fixture");
        clone_subject(&connection, "later-valid");
        connection
            .execute(
                r#"UPDATE wanted_subscription_records
                      SET next_attempt_at = CASE subject_id
                          WHEN 'rows-movie-001' THEN ?2
                          WHEN 'later-valid' THEN ?3
                      END
                    WHERE account_key = ?1
                      AND subject_id IN ('rows-movie-001', 'later-valid')"#,
                params![ACCOUNT, NOW as i64 - 1, NOW as i64],
            )
            .expect("order poison before a still-due valid candidate");
        connection
            .execute_batch(mutation)
            .expect("seed first poison candidate");
        drop(connection);
        let attempt_ids = Arc::new(SequenceAttemptIds::new(["must-not-be-used"]));
        let repository = repository(
            &fixture.path,
            Arc::new(FixedClock::new(NOW)),
            Arc::clone(&attempt_ids),
        );
        let claimed = repository
            .claim_due(claim_due_command())
            .await
            .expect("skipped legacy drift must not poison the due scan")
            .into_claim()
            .expect("the later valid candidate must remain claimable");
        assert_eq!(
            claimed.detail().summary().head.key.subject_id,
            "later-valid"
        );
        assert_eq!(attempt_ids.calls(), 1);
        assert_eq!(
            row_controls(&fixture.path, BASE_SUBJECT).execution_state,
            "idle",
            "the skipped legacy row must remain unclaimed"
        );
        assert_eq!(audit_rows(&fixture.path).len(), 1);
    }
}

#[tokio::test]
async fn malformed_forced_and_expired_candidates_fail_closed_before_later_due_rows() {
    for (label, expired) in [("forced-poison", false), ("expired-poison", true)] {
        let fixture = fresh_fixture(label).await;
        let connection = Connection::open(&fixture.path).expect("open branch poison fixture");
        clone_subject(&connection, "later-valid");
        connection
            .execute(
                r#"UPDATE wanted_subscription_records
                      SET next_attempt_at = ?3
                    WHERE account_key = ?1 AND subject_id = ?2"#,
                params![ACCOUNT, "later-valid", NOW as i64],
            )
            .expect("keep a valid due row behind branch poison");
        if expired {
            connection
                .execute(
                    r#"UPDATE wanted_subscription_records
                          SET execution_state = 'running', claimed_operation = 'movie_meta',
                              attempt_id = 'attempt-expired-poison', lease_until = ?3,
                              record_json = '{"unknown":true}'
                        WHERE account_key = ?1 AND subject_id = ?2"#,
                    params![ACCOUNT, BASE_SUBJECT, NOW as i64],
                )
                .expect("seed malformed expired-lease candidate");
        } else {
            connection
                .execute(
                    r#"UPDATE wanted_subscription_records
                          SET force_eligible_once = 1, next_attempt_at = ?3,
                              attention_tags_json = '["unknown_attention"]'
                        WHERE account_key = ?1 AND subject_id = ?2"#,
                    params![ACCOUNT, BASE_SUBJECT, NOW as i64 + 10_000],
                )
                .expect("seed malformed force-once candidate");
        }
        drop(connection);

        let poison_before = row_controls(&fixture.path, BASE_SUBJECT);
        let later_before = row_controls(&fixture.path, "later-valid");
        let attempt_ids = Arc::new(SequenceAttemptIds::new(["must-not-be-used"]));
        let repository = repository(
            &fixture.path,
            Arc::new(FixedClock::new(NOW)),
            Arc::clone(&attempt_ids),
        );
        let error = repository
            .claim_due(claim_due_command())
            .await
            .expect_err("malformed priority candidate must stop the bounded scan");
        assert!(matches!(error, RepositoryError::CorruptData { .. }));
        assert_eq!(attempt_ids.calls(), 0);
        assert_eq!(row_controls(&fixture.path, BASE_SUBJECT), poison_before);
        assert_eq!(
            row_controls(&fixture.path, "later-valid"),
            later_before,
            "a later valid row must not be silently substituted for poison"
        );
        assert!(audit_rows(&fixture.path).is_empty());
    }
}

#[tokio::test]
async fn claim_preserves_detail_and_revision_changes_do_not_replace_attempt_identity() {
    let fixture = fresh_fixture("detail-preservation").await;
    let clock = Arc::new(FixedClock::new(NOW));
    let attempt_ids = Arc::new(SequenceAttemptIds::new(["attempt-detail"]));
    let repository = repository(&fixture.path, Arc::clone(&clock), Arc::clone(&attempt_ids));
    let before = repository
        .load_detail(key(BASE_SUBJECT))
        .await
        .expect("load detail before claim");
    let claimed = repository
        .claim_one(claim_one_command(BASE_SUBJECT))
        .await
        .expect("claim detail fixture");
    let ClaimOneResult::Claimed(claimed) = claimed else {
        panic!("due detail fixture must be claimed");
    };
    assert_eq!(claimed.detail().payload(), before.payload());
    assert_eq!(
        claimed.detail().summary().projection,
        before.summary().projection
    );
    assert_eq!(
        claimed.detail().summary().attention_tags,
        before.summary().attention_tags
    );
    assert_eq!(
        claimed.detail().summary().head.revision.value(),
        before.summary().head.revision.value() + 1
    );
    assert_eq!(
        claimed.detail().summary().head.next_attempt_at,
        before.summary().head.next_attempt_at
    );
    let token = claimed.attempt().token().clone();

    let mut payload = claimed.detail().payload().clone();
    payload.source.title = "Detail changed while attempt stays live".to_string();
    payload.observation.last_seen_at = NOW + 1;
    let command = UpdateSubscriptionDetailCommand::try_new(
        claimed.detail().summary().head.key.clone(),
        claimed.detail().summary().head.revision,
        NOW + 1,
        claimed.detail().summary().attention_tags.clone(),
        payload,
    )
    .expect("build non-gate live detail update");
    let updated = repository
        .update_detail(command)
        .await
        .expect("update non-gate detail while attempt is live");
    assert_eq!(
        updated.detail().summary().head.revision.value(),
        claimed.detail().summary().head.revision.value() + 1
    );
    let collision = repository
        .claim_one(claim_one_command(BASE_SUBJECT))
        .await
        .expect("classify live attempt after unrelated revision change");
    let ClaimOneResult::Rejected(ClaimRejection::LiveAttempt { current }) = collision else {
        panic!("revision change must not replace live attempt identity");
    };
    assert_eq!(current.token(), &token);
    assert_eq!(attempt_ids.calls(), 1);
}

#[tokio::test]
async fn two_repositories_claim_disjoint_rows_and_manual_worker_collision_has_one_owner() {
    let disjoint_fixture = fresh_fixture("two-repository-disjoint").await;
    let connection = Connection::open(&disjoint_fixture.path).expect("open disjoint fixture");
    clone_subject(&connection, "second-due");
    drop(connection);
    let barrier = Arc::new(Barrier::new(2));
    let first = repository(
        &disjoint_fixture.path,
        Arc::new(FixedClock::new(NOW)),
        Arc::new(SequenceAttemptIds::new(["attempt-concurrent-a"])),
    );
    let second = repository(
        &disjoint_fixture.path,
        Arc::new(FixedClock::new(NOW)),
        Arc::new(SequenceAttemptIds::new(["attempt-concurrent-b"])),
    );
    let first_barrier = Arc::clone(&barrier);
    let first_task = tokio::spawn(async move {
        first_barrier.wait().await;
        first.claim_due(claim_due_command()).await
    });
    let second_barrier = Arc::clone(&barrier);
    let second_task = tokio::spawn(async move {
        second_barrier.wait().await;
        second.claim_due(claim_due_command()).await
    });
    let first = first_task
        .await
        .expect("join first concurrent claim")
        .expect("first concurrent claim");
    let second = second_task
        .await
        .expect("join second concurrent claim")
        .expect("second concurrent claim");
    let subjects = [first, second]
        .into_iter()
        .map(|result| {
            result
                .into_claim()
                .expect("two due rows must produce two claims")
                .detail()
                .summary()
                .head
                .key
                .subject_id
                .clone()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        subjects,
        BTreeSet::from([BASE_SUBJECT.to_string(), "second-due".to_string()])
    );
    assert_eq!(audit_rows(&disjoint_fixture.path).len(), 2);

    let collision_fixture = fresh_fixture("manual-worker-collision").await;
    let barrier = Arc::new(Barrier::new(2));
    let worker = repository(
        &collision_fixture.path,
        Arc::new(FixedClock::new(NOW)),
        Arc::new(SequenceAttemptIds::new(["attempt-worker"])),
    );
    let manual = repository(
        &collision_fixture.path,
        Arc::new(FixedClock::new(NOW)),
        Arc::new(SequenceAttemptIds::new(["attempt-manual"])),
    );
    let worker_barrier = Arc::clone(&barrier);
    let worker_task = tokio::spawn(async move {
        worker_barrier.wait().await;
        worker.claim_due(claim_due_command()).await
    });
    let manual_barrier = Arc::clone(&barrier);
    let manual_task = tokio::spawn(async move {
        manual_barrier.wait().await;
        manual.claim_one(claim_one_command(BASE_SUBJECT)).await
    });
    let worker = worker_task
        .await
        .expect("join worker collision")
        .expect("worker collision result");
    let manual = manual_task
        .await
        .expect("join manual collision")
        .expect("manual collision result");
    let worker_owned = worker.claim().is_some();
    let manual_owned = matches!(manual, ClaimOneResult::Claimed(_));
    assert_ne!(
        worker_owned, manual_owned,
        "exactly one caller may own the live attempt"
    );
    if !manual_owned {
        assert!(matches!(
            manual,
            ClaimOneResult::Rejected(ClaimRejection::LiveAttempt { .. })
        ));
    }
    assert_eq!(audit_rows(&collision_fixture.path).len(), 1);
}

#[tokio::test]
async fn simultaneous_incomplete_movie_poll_and_claim_serialize_without_losing_attempt_or_source() {
    let fixture = fresh_fixture("race-incomplete-movie-claim").await;
    let poll_repository = repository(
        &fixture.path,
        Arc::new(FixedClock::new(RACE_AT)),
        Arc::new(SequenceAttemptIds::new(Vec::<String>::new())),
    );
    let poll_token = poll_repository
        .begin_poll(BeginPollCommand::try_new(ACCOUNT, RACE_AT - 1).unwrap())
        .await
        .expect("begin incomplete race poll")
        .token;
    let poll_command = RecordIncompleteSnapshotCommand::try_new(
        ACCOUNT,
        poll_token.clone(),
        RACE_AT,
        IncompleteSnapshotObservation::try_new(
            1,
            false,
            false,
            IncompleteSnapshotReason::EndNotObserved,
        )
        .unwrap(),
        NewRecordPolicy::try_new(3, false).unwrap(),
        vec![movie_snapshot(
            BASE_SUBJECT,
            "Race Refreshed Incomplete Movie",
            RACE_AT,
        )],
        PollRetryPolicy::try_new(5, 60).unwrap(),
    )
    .unwrap();
    let attempt_ids = Arc::new(SequenceAttemptIds::new(["attempt-race-incomplete"]));
    let attempt_source: Arc<dyn ExecutionAttemptIdSource> = attempt_ids.clone();
    let dependencies = ClaimDependencies::new(Arc::new(FixedClock::new(RACE_AT)), attempt_source);

    let (poll, claim) =
        race_poll_terminal_and_claim(&fixture.path, dependencies, move |connection| {
            super::super::poll::record_incomplete_snapshot(connection, poll_command)
        });
    let poll = poll.expect("incomplete Poll contender must commit");
    assert_eq!(poll.updated, 1);
    let claimed = claim
        .expect("claim contender must serialize")
        .into_claim()
        .expect("seen movie remains due in either serialization order");
    assert_eq!(
        claimed.attempt().token().attempt_id().as_str(),
        "attempt-race-incomplete"
    );
    assert!(claimed.replaced_expired_attempt().is_none());

    let detail = poll_repository
        .load_detail(key(BASE_SUBJECT))
        .await
        .expect("load incomplete-race result");
    assert_eq!(
        detail.summary().head.execution_state,
        SubscriptionExecutionState::Running
    );
    assert_eq!(
        detail.summary().projection.title,
        "Race Refreshed Incomplete Movie"
    );
    let controls = row_controls(&fixture.path, BASE_SUBJECT);
    assert_eq!(
        controls.attempt_id.as_deref(),
        Some("attempt-race-incomplete")
    );
    assert_eq!(controls.lease_until, Some((RACE_AT + 60) as i64));
    assert!(current_open_poll_token(&fixture.path).is_none());
    assert_eq!(attempt_ids.calls(), 1);
    let audits = audit_rows(&fixture.path);
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].0, "claim_attempt");
    assert_eq!(
        audits[0].3["new_attempt"]["attempt_id"],
        "attempt-race-incomplete"
    );
}

#[tokio::test]
async fn simultaneous_complete_missing_poll_and_claim_have_only_two_legal_audit_outcomes() {
    let fixture = fresh_fixture("race-complete-missing-claim").await;
    let poll_repository = repository(
        &fixture.path,
        Arc::new(FixedClock::new(RACE_AT)),
        Arc::new(SequenceAttemptIds::new(Vec::<String>::new())),
    );
    let poll_token = poll_repository
        .begin_poll(BeginPollCommand::try_new(ACCOUNT, RACE_AT - 1).unwrap())
        .await
        .expect("begin complete-missing race poll")
        .token;
    let poll_command = ApplyCompleteSnapshotCommand::try_new(
        ACCOUNT,
        poll_token,
        RACE_AT,
        RACE_AT + 60,
        NewRecordPolicy::try_new(3, false).unwrap(),
        Vec::new(),
    )
    .unwrap();
    let attempt_ids = Arc::new(SequenceAttemptIds::new(["attempt-race-missing"]));
    let attempt_source: Arc<dyn ExecutionAttemptIdSource> = attempt_ids.clone();
    let dependencies = ClaimDependencies::new(Arc::new(FixedClock::new(RACE_AT)), attempt_source);

    let (poll, claim) =
        race_poll_terminal_and_claim(&fixture.path, dependencies, move |connection| {
            super::super::poll::apply_complete_snapshot(connection, poll_command)
        });
    assert_eq!(
        poll.expect("complete-missing Poll contender must commit")
            .deactivated,
        2
    );
    let claim = claim.expect("claim contender must serialize");
    let claim_won_first = claim.claim().is_some();

    let detail = poll_repository
        .load_detail(key(BASE_SUBJECT))
        .await
        .expect("load complete-missing race result");
    assert!(!detail.summary().head.active);
    assert_eq!(
        detail.summary().head.execution_state,
        SubscriptionExecutionState::Idle
    );
    assert_eq!(detail.summary().head.next_attempt_at, None);
    let controls = row_controls(&fixture.path, BASE_SUBJECT);
    assert_eq!(controls.attempt_id, None);
    assert_eq!(controls.lease_until, None);
    assert!(current_open_poll_token(&fixture.path).is_none());

    let audits = audit_rows(&fixture.path);
    if claim_won_first {
        assert_eq!(attempt_ids.calls(), 1);
        assert_eq!(audits.len(), 2);
        assert_eq!(audits[0].0, "claim_attempt");
        assert_eq!(audits[1].0, "supersede_attempt");
        assert_eq!(audits[1].3["reason"], "missing_from_complete_snapshot");
        assert_eq!(audits[1].3["attempt_id"], "attempt-race-missing");
        assert_eq!(audits[1].3["lease_state_at_fence"], "live");
    } else {
        assert_eq!(attempt_ids.calls(), 0);
        assert!(audits.is_empty());
    }
}

#[tokio::test]
async fn simultaneous_complete_movie_poll_and_expired_reclaim_preserve_the_new_fence() {
    let fixture = fresh_fixture("race-complete-movie-reclaim").await;
    Connection::open(&fixture.path)
        .expect("open expired reclaim race fixture")
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET execution_state = 'running',
                      claimed_operation = 'movie_meta',
                      attempt_id = 'attempt-race-expired-old',
                      lease_until = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, BASE_SUBJECT, RACE_AT as i64],
        )
        .expect("seed expired attempt before complete/reclaim race");
    let poll_repository = repository(
        &fixture.path,
        Arc::new(FixedClock::new(RACE_AT)),
        Arc::new(SequenceAttemptIds::new(Vec::<String>::new())),
    );
    let poll_token = poll_repository
        .begin_poll(BeginPollCommand::try_new(ACCOUNT, RACE_AT - 1).unwrap())
        .await
        .expect("begin complete/reclaim race poll")
        .token;
    let poll_command = ApplyCompleteSnapshotCommand::try_new(
        ACCOUNT,
        poll_token,
        RACE_AT,
        RACE_AT + 60,
        NewRecordPolicy::try_new(3, false).unwrap(),
        vec![
            movie_snapshot(BASE_SUBJECT, "Race Refreshed Complete Movie", RACE_AT),
            movie_snapshot(
                "rows-movie-002",
                "Fixture Rows Completed Movie",
                RACE_AT - 1,
            ),
        ],
    )
    .unwrap();
    let attempt_ids = Arc::new(SequenceAttemptIds::new(["attempt-race-reclaimed"]));
    let attempt_source: Arc<dyn ExecutionAttemptIdSource> = attempt_ids.clone();
    let dependencies = ClaimDependencies::new(Arc::new(FixedClock::new(RACE_AT)), attempt_source);

    let (poll, claim) =
        race_poll_terminal_and_claim(&fixture.path, dependencies, move |connection| {
            super::super::poll::apply_complete_snapshot(connection, poll_command)
        });
    poll.expect("complete movie Poll contender must commit");
    let reclaimed = claim
        .expect("expired reclaim contender must serialize")
        .into_claim()
        .expect("expired movie remains reclaimable in either serialization order");
    assert_eq!(
        reclaimed
            .replaced_expired_attempt()
            .expect("old expired fence must be returned")
            .token()
            .attempt_id()
            .as_str(),
        "attempt-race-expired-old"
    );
    assert_eq!(
        reclaimed.attempt().token().attempt_id().as_str(),
        "attempt-race-reclaimed"
    );

    let detail = poll_repository
        .load_detail(key(BASE_SUBJECT))
        .await
        .expect("load complete/reclaim race result");
    assert_eq!(
        detail.summary().head.execution_state,
        SubscriptionExecutionState::Running
    );
    assert_eq!(
        detail.summary().projection.title,
        "Race Refreshed Complete Movie"
    );
    let controls = row_controls(&fixture.path, BASE_SUBJECT);
    assert_eq!(
        controls.attempt_id.as_deref(),
        Some("attempt-race-reclaimed")
    );
    assert_eq!(controls.lease_until, Some((RACE_AT + 60) as i64));
    assert!(current_open_poll_token(&fixture.path).is_none());
    assert_eq!(attempt_ids.calls(), 1);
    let audits = audit_rows(&fixture.path);
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].0, "reclaim_attempt");
    assert_eq!(
        audits[0].3["previous_attempt"]["attempt_id"],
        "attempt-race-expired-old"
    );
    assert_eq!(
        audits[0].3["new_attempt"]["attempt_id"],
        "attempt-race-reclaimed"
    );
}

#[tokio::test]
async fn supersede_failure_rolls_back_simultaneous_missing_poll_while_reclaim_remains_retryable() {
    let fixture = fresh_fixture("race-missing-reclaim-rollback").await;
    let connection = Connection::open(&fixture.path).expect("open rollback race fixture");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET execution_state = 'running',
                      claimed_operation = 'movie_meta',
                      attempt_id = 'attempt-race-rollback-old',
                      lease_until = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, BASE_SUBJECT, RACE_AT as i64],
        )
        .expect("seed expired rollback-race attempt");
    connection
        .execute_batch(
            r#"CREATE TRIGGER reject_race_supersede_audit
               BEFORE INSERT ON operation_logs
               WHEN NEW.category = 'subscription_scheduler'
                AND NEW.action = 'supersede_attempt'
                AND NEW.target_id = 'rows-movie-001'
               BEGIN
                   SELECT RAISE(ABORT, 'reject race supersede audit');
               END;"#,
        )
        .expect("install rollback-race audit rejection trigger");
    drop(connection);
    let poll_repository = repository(
        &fixture.path,
        Arc::new(FixedClock::new(RACE_AT)),
        Arc::new(SequenceAttemptIds::new(Vec::<String>::new())),
    );
    let poll_token = poll_repository
        .begin_poll(BeginPollCommand::try_new(ACCOUNT, RACE_AT - 1).unwrap())
        .await
        .expect("begin rollback race poll")
        .token;
    let expected_open = Some((
        poll_token.generation.value() as i64,
        poll_token.snapshot_id.as_str().to_string(),
    ));
    let poll_command = ApplyCompleteSnapshotCommand::try_new(
        ACCOUNT,
        poll_token,
        RACE_AT,
        RACE_AT + 60,
        NewRecordPolicy::try_new(3, false).unwrap(),
        Vec::new(),
    )
    .unwrap();
    let retry_command = poll_command.clone();
    let attempt_ids = Arc::new(SequenceAttemptIds::new(["attempt-race-rollback-new"]));
    let attempt_source: Arc<dyn ExecutionAttemptIdSource> = attempt_ids.clone();
    let dependencies = ClaimDependencies::new(Arc::new(FixedClock::new(RACE_AT)), attempt_source);

    let (poll, claim) =
        race_poll_terminal_and_claim(&fixture.path, dependencies, move |connection| {
            super::super::poll::apply_complete_snapshot(connection, poll_command)
        });
    assert!(matches!(poll, Err(RepositoryError::CorruptData { .. })));
    let reclaimed = claim
        .expect("reclaim must commit around rolled-back Poll contender")
        .into_claim()
        .expect("expired attempt must be reclaimed");
    assert_eq!(
        reclaimed.attempt().token().attempt_id().as_str(),
        "attempt-race-rollback-new"
    );
    assert_eq!(current_open_poll_token(&fixture.path), expected_open);
    let before_retry = poll_repository
        .load_detail(key(BASE_SUBJECT))
        .await
        .expect("load row after rolled-back Poll");
    assert!(before_retry.summary().head.active);
    assert_eq!(
        before_retry.summary().head.execution_state,
        SubscriptionExecutionState::Running
    );
    assert_eq!(
        row_controls(&fixture.path, BASE_SUBJECT)
            .attempt_id
            .as_deref(),
        Some("attempt-race-rollback-new")
    );
    let audits = audit_rows(&fixture.path);
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].0, "reclaim_attempt");

    Connection::open(&fixture.path)
        .expect("open rollback race trigger cleanup")
        .execute_batch("DROP TRIGGER reject_race_supersede_audit;")
        .expect("drop rollback-race rejection trigger");
    assert_eq!(
        poll_repository
            .apply_complete_snapshot(retry_command)
            .await
            .expect("same Poll token must retry after transactional rollback")
            .deactivated,
        2
    );
    let final_detail = poll_repository
        .load_detail(key(BASE_SUBJECT))
        .await
        .expect("load retried missing Poll result");
    assert!(!final_detail.summary().head.active);
    assert_eq!(
        final_detail.summary().head.execution_state,
        SubscriptionExecutionState::Idle
    );
    assert!(current_open_poll_token(&fixture.path).is_none());
    let audits = audit_rows(&fixture.path);
    assert_eq!(audits.len(), 2);
    assert_eq!(audits[0].0, "reclaim_attempt");
    assert_eq!(audits[1].0, "supersede_attempt");
    assert_eq!(audits[1].3["attempt_id"], "attempt-race-rollback-new");
    assert_eq!(audits[1].3["reason"], "missing_from_complete_snapshot");
    assert_eq!(attempt_ids.calls(), 1);
}

#[tokio::test]
async fn repository_clock_overflow_rolls_back_before_nonce_generation() {
    let fixture = fresh_fixture("clock-overflow").await;
    let before = row_controls(&fixture.path, BASE_SUBJECT);
    let attempt_ids = Arc::new(SequenceAttemptIds::new(["attempt-overflow"]));
    let clock = Arc::new(FixedClock::new(i64::MAX as u64 + 1));
    let repository = repository(&fixture.path, Arc::clone(&clock), Arc::clone(&attempt_ids));
    let rejection_error = repository
        .claim_one(claim_one_command("rows-movie-002"))
        .await
        .expect_err("out-of-range repository clock must fail before typed rejection");
    assert!(matches!(
        rejection_error,
        RepositoryError::Unavailable { .. }
    ));

    clock.set(i64::MAX as u64);
    let error = repository
        .claim_one(claim_one_command(BASE_SUBJECT))
        .await
        .expect_err("clock plus TTL overflow must fail");
    assert!(matches!(error, RepositoryError::Unavailable { .. }));
    assert_eq!(attempt_ids.calls(), 0);
    assert_eq!(row_controls(&fixture.path, BASE_SUBJECT), before);
    assert!(audit_rows(&fixture.path).is_empty());
}

#[tokio::test]
async fn ten_thousand_row_list_due_and_reclaim_queries_are_indexed_and_bounded() {
    let fixture = fresh_fixture("large-query-evidence").await;
    let connection = Connection::open(&fixture.path).expect("open large query fixture");
    insert_generated_clones(&connection, ACCOUNT, BASE_SUBJECT, "scale", 10_000);
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET next_attempt_at = ?2, force_eligible_once = 0
                WHERE account_key = ?1 AND lifecycle_state != 'completed'"#,
            params![ACCOUNT, NOW as i64 + 10_000],
        )
        .expect("park large fixture rows in the future");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET execution_state = 'running', claimed_operation = 'movie_meta',
                      attempt_id = 'attempt-scale-expired', lease_until = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "scale-00000", NOW as i64],
        )
        .expect("seed one expired lease in large fixture");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET force_eligible_once = 1
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "scale-00001"],
        )
        .expect("seed one forced row in large fixture");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET next_attempt_at = ?3
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "scale-00002", NOW as i64 - 1],
        )
        .expect("seed one normally due row in large fixture");
    let record_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM wanted_subscription_records WHERE account_key = ?1",
            [ACCOUNT],
            |row| row.get(0),
        )
        .expect("count large query fixture rows");
    assert_eq!(record_count, 10_002);

    let list_command =
        ListSubscriptionsCommand::try_new(ACCOUNT, SubscriptionListFilter::default(), None, 10)
            .expect("build large list command");
    let list_queries = build_list_queries(&list_command).expect("build large list segments");
    assert_eq!(list_queries.len(), 4);
    let mut largest_list_segment = 0;
    for query in &list_queries {
        let plan = explain_query(&connection, &query.sql, &query.values);
        let search = plan
            .iter()
            .find(|detail| detail.contains("wanted_records_list_v5_idx"))
            .unwrap_or_else(|| panic!("list did not use frozen index: {plan:?}"));
        assert!(
            search.contains("SEARCH") && !search.contains("SCAN"),
            "list scanned instead of seeking the frozen index: {plan:?}"
        );
        assert!(
            plan.iter().all(|detail| !detail.contains("TEMP B-TREE")),
            "list used a temporary sort: {plan:?}"
        );
        let work = measured_subject_query(&connection, &query.sql, &query.values, 1);
        assert_bounded_index_work(&work, 11, "summary list segment");
        largest_list_segment = largest_list_segment.max(work.subjects.len());
    }
    assert_eq!(
        largest_list_segment, 11,
        "a populated list segment must stop at limit + 1"
    );

    for (label, sql, expected_index, expected_subject, values) in [
        (
            "expired lease",
            EXPIRED_CANDIDATE_SQL,
            "wanted_records_expired_lease_v5_idx",
            "scale-00000",
            vec![
                SqlValue::Text(ACCOUNT.to_string()),
                SqlValue::Integer(NOW as i64),
            ],
        ),
        (
            "forced",
            FORCE_CANDIDATE_SQL,
            "wanted_records_force_v5_idx",
            "scale-00001",
            vec![SqlValue::Text(ACCOUNT.to_string())],
        ),
        (
            "normally due",
            DUE_CANDIDATE_SQL,
            "wanted_records_due_v5_idx",
            "scale-00002",
            vec![
                SqlValue::Text(ACCOUNT.to_string()),
                SqlValue::Integer(NOW as i64),
            ],
        ),
    ] {
        assert!(sql.contains("LIMIT 1"));
        assert!(!sql.contains(" OR "));
        let plan = explain_query(&connection, sql, &values);
        let search = plan
            .iter()
            .find(|detail| detail.contains(expected_index))
            .unwrap_or_else(|| panic!("{label} did not use {expected_index}: {plan:?}"));
        assert!(
            search.contains("SEARCH") && !search.contains("SCAN"),
            "{label} scanned rather than searched {expected_index}: {plan:?}"
        );
        assert!(
            plan.iter().all(|detail| !detail.contains("TEMP B-TREE")),
            "{label} used a temporary sort: {plan:?}"
        );
        let work = measured_subject_query(&connection, sql, &values, 0);
        assert_bounded_index_work(&work, 1, label);
        assert_eq!(work.subjects, [expected_subject]);
    }
    drop(connection);

    let attempt_ids = Arc::new(SequenceAttemptIds::new([
        "attempt-scale-expired-new",
        "attempt-scale-forced",
        "attempt-scale-due",
    ]));
    let repository = repository(
        &fixture.path,
        Arc::new(FixedClock::new(NOW)),
        Arc::clone(&attempt_ids),
    );
    let page = repository
        .list_summaries(list_command)
        .await
        .expect("list ten-thousand-row fixture through repository");
    assert_eq!(page.items.len(), 10);
    assert!(page.next_cursor.is_some());

    let mut claimed_subjects = Vec::new();
    for _ in 0..3 {
        let claimed = repository
            .claim_due(claim_due_command())
            .await
            .expect("claim bounded large-fixture candidate")
            .into_claim()
            .expect("large fixture must retain the seeded candidate");
        claimed_subjects.push(claimed.detail().summary().head.key.subject_id.clone());
    }
    assert_eq!(
        claimed_subjects,
        ["scale-00000", "scale-00001", "scale-00002"]
    );
    assert_eq!(attempt_ids.calls(), 3);
}

#[tokio::test]
async fn thousand_neighbor_finish_has_two_row_writes_and_preserves_exact_unrelated_bytes() {
    let fixture = fresh_fixture("finish-write-amplification").await;
    let mut connection = crate::storage::sqlite::open_v5_connection(&fixture.path, BUSY_TIMEOUT)
        .expect("open direct finish write-amplification connection");
    insert_generated_clones(&connection, ACCOUNT, BASE_SUBJECT, "finish-neighbor", 1_000);
    let record_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM wanted_subscription_records WHERE account_key = ?1",
            [ACCOUNT],
            |row| row.get(0),
        )
        .expect("count finish write-amplification records");
    assert_eq!(record_count, 1_002);

    let clock = Arc::new(FixedClock::new(NOW));
    let attempt_ids = Arc::new(SequenceAttemptIds::new(["attempt-finish-write-set"]));
    let dependencies = ClaimDependencies::new(clock.clone(), attempt_ids);
    let claimed = super::claim_one(
        &mut connection,
        claim_one_command(BASE_SUBJECT),
        dependencies.clone(),
    )
    .expect("claim direct finish write-amplification target");
    let ClaimOneResult::Claimed(claimed) = claimed else {
        panic!("finish write-amplification target must be claimed");
    };
    let token = claimed.attempt().token().clone();
    let target_payload_before: Vec<u8> = connection
        .query_row(
            r#"SELECT CAST(record_json AS BLOB)
                 FROM wanted_subscription_records
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, BASE_SUBJECT],
            |row| row.get(0),
        )
        .expect("read target payload bytes before finish");
    let unrelated_before = unrelated_storage_snapshot(&connection, ACCOUNT, BASE_SUBJECT);
    let changes_before = connection.total_changes();

    clock.set(NOW + 1);
    let result = super::finish(
        &mut connection,
        FinishExecutionCommand::try_new(
            token,
            FinishExecutionDisposition::MetaReady,
            ExecutionPayloadDelta::Meta,
        )
        .expect("build direct finish command"),
        dependencies,
    )
    .expect("finish one target among one thousand neighbors");

    assert_eq!(
        connection.total_changes() - changes_before,
        2,
        "finish must update one subscription row and append one audit row independent of account cardinality"
    );
    assert_eq!(
        result.detail().summary().head.revision.value(),
        claimed.detail().summary().head.revision.value() + 1
    );
    assert_eq!(result.detail().payload(), claimed.detail().payload());
    assert_eq!(
        unrelated_storage_snapshot(&connection, ACCOUNT, BASE_SUBJECT),
        unrelated_before,
        "finish must preserve every unrelated revision, scalar control, JSON byte string, and account metadata value"
    );
    let target_payload_after: Vec<u8> = connection
        .query_row(
            r#"SELECT CAST(record_json AS BLOB)
                 FROM wanted_subscription_records
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, BASE_SUBJECT],
            |row| row.get(0),
        )
        .expect("read target payload bytes after finish");
    assert_eq!(
        target_payload_after, target_payload_before,
        "a metadata-only finish must preserve the target payload bytes as well as its semantics"
    );
}

#[tokio::test]
async fn exact_finish_merges_search_delta_into_latest_poll_and_detail_revision() {
    let fixture = fresh_fixture("finish-interleaving").await;
    let connection = Connection::open(&fixture.path).expect("open finish interleaving fixture");
    clone_subject(&connection, "link-complete");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET lifecycle_state = 'searching',
                      next_attempt_at = ?3,
                      retry_count = 3,
                      max_retries = 3,
                      retry_blocked = 1,
                      force_eligible_once = 1,
                      attention_tags_json = '["skipped","failed","retry_blocked"]',
                      record_json = json_set(record_json, '$.skip_reason', 'operator skip')
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, BASE_SUBJECT, NOW as i64 + 10_000],
        )
        .expect("seed forced search finish row");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET lifecycle_state = 'linking'
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "link-complete"],
        )
        .expect("seed exact link completion row");
    drop(connection);

    let clock = Arc::new(FixedClock::new(NOW));
    let attempt_ids = Arc::new(SequenceAttemptIds::new([
        "attempt-finish",
        "attempt-link-complete",
    ]));
    let repository = repository(&fixture.path, Arc::clone(&clock), attempt_ids);
    let token = claim_token(&repository, BASE_SUBJECT).await;

    let claimed_detail = repository
        .load_detail(key(BASE_SUBJECT))
        .await
        .expect("load claimed detail");
    let mut refreshed_source = claimed_detail.payload().source.clone();
    refreshed_source.title = "Poll-refreshed title".to_string();
    let poll = repository
        .begin_poll(BeginPollCommand::try_new(ACCOUNT, NOW + 1).unwrap())
        .await
        .expect("begin interleaved poll");
    repository
        .record_incomplete_snapshot(
            RecordIncompleteSnapshotCommand::try_new(
                ACCOUNT,
                poll.token,
                NOW + 2,
                IncompleteSnapshotObservation::try_new(
                    1,
                    false,
                    false,
                    IncompleteSnapshotReason::EndNotObserved,
                )
                .unwrap(),
                NewRecordPolicy::try_new(3, false).unwrap(),
                vec![SnapshotRecord::try_new(
                    BASE_SUBJECT,
                    SubscriptionMediaKind::Movie,
                    true,
                    None,
                    refreshed_source,
                )
                .unwrap()],
                PollRetryPolicy::try_new(5, 60).unwrap(),
            )
            .unwrap(),
        )
        .await
        .expect("apply interleaved partial poll");

    let after_poll = repository
        .load_detail(key(BASE_SUBJECT))
        .await
        .expect("load poll-refreshed running detail");
    assert_eq!(
        after_poll.summary().head.execution_state,
        SubscriptionExecutionState::Running
    );
    let mut detail_payload = after_poll.payload().clone();
    detail_payload.source.summary = Some("fresh detail field".to_string());
    let mut reordered_tags = after_poll.summary().attention_tags.clone();
    reordered_tags.reverse();
    let updated = repository
        .update_detail(
            UpdateSubscriptionDetailCommand::try_new(
                key(BASE_SUBJECT),
                after_poll.summary().head.revision,
                NOW + 5,
                reordered_tags,
                detail_payload,
            )
            .unwrap(),
        )
        .await
        .expect("advance detail revision while attempt remains live");
    assert_eq!(
        updated.detail().summary().head.execution_state,
        SubscriptionExecutionState::Running
    );

    clock.set(NOW + 10);
    let result = repository
        .finish(
            FinishExecutionCommand::try_new(
                token.clone(),
                FinishExecutionDisposition::SearchWaiting {
                    retry_after: ExecutionScheduleDelay::try_new(90).unwrap(),
                },
                ExecutionPayloadDelta::Search {
                    candidates: Some(vec![candidate("torrent-fresh", "Fresh candidate")]),
                    download_updates: Vec::new(),
                },
            )
            .unwrap(),
        )
        .await
        .expect("finish exact live attempt after Poll and detail revisions");

    let detail = result.detail();
    assert_eq!(result.attempt(), &token);
    assert_eq!(result.finished_at(), NOW + 10);
    assert_eq!(
        detail.summary().projection.title,
        "Poll-refreshed title",
        "finish must retain the latest Poll-owned source projection"
    );
    assert_eq!(
        detail.payload().source.summary.as_deref(),
        Some("fresh detail field"),
        "finish must retain the latest generic detail change"
    );
    assert_eq!(detail.payload().candidates.len(), 1);
    assert_eq!(
        detail.payload().candidates[0].candidate.torrent_id,
        "torrent-fresh"
    );
    assert_eq!(
        detail.payload().skip_reason.as_deref(),
        Some("operator skip")
    );
    assert_eq!(
        detail.summary().head.lifecycle_state,
        SubscriptionLifecycleState::Searching
    );
    assert_eq!(
        detail.summary().head.execution_state,
        SubscriptionExecutionState::Idle
    );
    assert_eq!(detail.summary().head.next_attempt_at, Some(NOW + 100));
    assert_eq!(detail.summary().head.retry_count, 0);
    assert!(!detail.summary().head.retry_blocked);
    assert!(!detail.summary().head.force_eligible_once);
    assert_eq!(
        detail.summary().attention_tags,
        vec![
            SubscriptionAttentionTag::WaitingRelease,
            SubscriptionAttentionTag::Skipped,
        ]
    );

    let link_token = claim_token(&repository, "link-complete").await;
    let completed = repository
        .finish(
            FinishExecutionCommand::try_new(
                link_token,
                FinishExecutionDisposition::LinkCompleted,
                ExecutionPayloadDelta::Link {
                    download_updates: Vec::new(),
                    link_updates: Vec::new(),
                },
            )
            .unwrap(),
        )
        .await
        .expect("finish exact live link attempt as completed");
    assert_eq!(
        completed.detail().summary().head.lifecycle_state,
        SubscriptionLifecycleState::Completed
    );
    assert_eq!(completed.detail().summary().head.next_attempt_at, None);
    assert_eq!(
        completed.detail().summary().head.execution_state,
        SubscriptionExecutionState::Idle
    );
    assert_eq!(
        audit_rows(&fixture.path)
            .into_iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        vec![
            "claim_attempt",
            "finish_attempt",
            "claim_attempt",
            "finish_attempt",
        ]
    );
}

#[tokio::test]
async fn exact_failure_consumes_attempt_force_and_persists_retry_issue_atomically() {
    let fixture = fresh_fixture("exact-failure").await;
    let connection = Connection::open(&fixture.path).expect("open exact failure fixture");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET lifecycle_state = 'downloading',
                      next_attempt_at = ?3,
                      retry_count = 1,
                      max_retries = 1,
                      retry_blocked = 1,
                      force_eligible_once = 1,
                      attention_tags_json = '["failed","retry_blocked"]'
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, BASE_SUBJECT, NOW as i64 + 5_000],
        )
        .expect("seed progress failure row");
    drop(connection);

    let clock = Arc::new(FixedClock::new(NOW));
    let repository = repository(
        &fixture.path,
        Arc::clone(&clock),
        Arc::new(SequenceAttemptIds::new(["attempt-fail"])),
    );
    let token = claim_token(&repository, BASE_SUBJECT).await;
    clock.set(NOW + 1);
    let result = repository
        .fail(
            FailExecutionCommand::try_new(
                token.clone(),
                "upstream",
                "qB progress unavailable",
                30,
                ExecutionPayloadDelta::Progress {
                    download_updates: Vec::new(),
                },
            )
            .unwrap(),
        )
        .await
        .expect("persist exact failed attempt");

    let detail = result.detail();
    assert_eq!(result.attempt(), &token);
    assert_eq!(result.failed_at(), NOW + 1);
    assert_eq!(
        detail.summary().head.execution_state,
        SubscriptionExecutionState::Idle
    );
    assert_eq!(
        detail.summary().head.lifecycle_state,
        SubscriptionLifecycleState::Downloading
    );
    assert_eq!(detail.summary().head.retry_count, 2);
    assert!(detail.summary().head.retry_blocked);
    assert_eq!(detail.summary().head.next_attempt_at, None);
    assert!(!detail.summary().head.force_eligible_once);
    assert_eq!(
        detail.summary().attention_tags,
        vec![
            SubscriptionAttentionTag::Failed,
            SubscriptionAttentionTag::RetryBlocked,
        ]
    );
    let issue = detail.payload().issues.last().expect("failure issue");
    assert_eq!(issue.operation.as_deref(), Some("movie_progress"));
    assert_eq!(issue.error_type.as_deref(), Some("upstream"));
    assert_eq!(issue.message, "qB progress unavailable");
    assert_eq!(issue.occurred_at, Some(NOW + 1));

    let duplicate = repository
        .fail(
            FailExecutionCommand::try_new(
                token,
                "upstream",
                "duplicate",
                30,
                ExecutionPayloadDelta::Progress {
                    download_updates: Vec::new(),
                },
            )
            .unwrap(),
        )
        .await
        .expect_err("consumed failure attempt must be stale");
    assert!(matches!(
        duplicate,
        RepositoryError::StaleAttempt { current: None, .. }
    ));
    assert_eq!(
        audit_rows(&fixture.path)
            .into_iter()
            .map(|row| (row.0, row.3))
            .collect::<Vec<_>>(),
        vec![
            (
                "claim_attempt".to_string(),
                serde_json::json!({
                    "eligibility": "force_once",
                    "force_eligible_once": true,
                    "new_attempt": {
                        "attempt_id": "attempt-fail",
                        "lease_until": NOW + 60,
                        "operation": "movie_progress",
                    },
                    "previous_attempt": null,
                    "row_revision": 2,
                    "schema_version": 1,
                    "trigger": "claim_one",
                }),
            ),
            (
                "fail_attempt".to_string(),
                serde_json::json!({
                    "attempt_id": "attempt-fail",
                    "error_type": "upstream",
                    "force_eligible_once_consumed": true,
                    "lease_until": NOW + 60,
                    "next_attempt_at": null,
                    "operation": "movie_progress",
                    "retry_blocked": true,
                    "retry_count": 2,
                    "row_revision": 3,
                    "schema_version": 1,
                }),
            ),
        ]
    );
}

#[tokio::test]
async fn lease_extension_is_strict_and_release_preserves_due_and_force() {
    let fixture = fresh_fixture("extend-release").await;
    let connection = Connection::open(&fixture.path).expect("open extend/release fixture");
    clone_subject(&connection, "release");
    connection
        .execute(
            r#"UPDATE wanted_subscription_records
                  SET next_attempt_at = ?3,
                      retry_count = 3,
                      retry_blocked = 1,
                      force_eligible_once = 1
                WHERE account_key = ?1 AND subject_id = ?2"#,
            params![ACCOUNT, "release", NOW as i64 + 5_000],
        )
        .expect("seed forced release row");
    drop(connection);

    let clock = Arc::new(FixedClock::new(NOW));
    let repository = repository(
        &fixture.path,
        Arc::clone(&clock),
        Arc::new(SequenceAttemptIds::new([
            "attempt-extend",
            "attempt-release",
        ])),
    );
    let extend_token = claim_token(&repository, BASE_SUBJECT).await;
    clock.set(NOW + 20);
    let extended = repository
        .extend_lease(ExtendExecutionLeaseCommand::try_new(extend_token.clone(), 60).unwrap())
        .await
        .expect("extend exact live lease forward");
    assert_eq!(extended.attempt().token(), &extend_token);
    assert_eq!(extended.attempt().lease_until(), NOW + 80);
    assert_eq!(
        row_controls(&fixture.path, BASE_SUBJECT).revision,
        i64::try_from(extended.revision().value()).unwrap()
    );

    let not_forward = repository
        .extend_lease(ExtendExecutionLeaseCommand::try_new(extend_token, 30).unwrap())
        .await
        .expect_err("shorter absolute lease boundary must be rejected");
    assert!(matches!(
        not_forward,
        RepositoryError::LeaseNotExtended {
            current_lease_until,
            requested_lease_until,
            ..
        } if current_lease_until == NOW + 80 && requested_lease_until == NOW + 50
    ));

    let release_token = claim_token(&repository, "release").await;
    let before_release = row_controls(&fixture.path, "release");
    let released = repository
        .release(ReleaseExecutionCommand::before_external_effect(
            release_token.clone(),
        ))
        .await
        .expect("release exact live attempt before effect");
    assert_eq!(released.attempt(), &release_token);
    assert_eq!(released.released_at(), NOW + 20);
    assert_eq!(
        released.detail().summary().head.execution_state,
        SubscriptionExecutionState::Idle
    );
    assert_eq!(
        released.detail().summary().head.next_attempt_at,
        Some(NOW + 5_000)
    );
    assert!(released.detail().summary().head.force_eligible_once);
    let after_release = row_controls(&fixture.path, "release");
    assert_eq!(
        after_release.next_attempt_at,
        before_release.next_attempt_at
    );
    assert_eq!(
        after_release.force_eligible_once,
        before_release.force_eligible_once
    );
    assert_eq!(after_release.execution_state, "idle");
    assert_eq!(after_release.claimed_operation, None);
    assert_eq!(after_release.attempt_id, None);
    assert_eq!(after_release.lease_until, None);
    assert_eq!(
        audit_rows(&fixture.path)
            .into_iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        vec![
            "claim_attempt",
            "extend_attempt",
            "claim_attempt",
            "release_attempt",
        ]
    );
}

#[tokio::test]
async fn equality_expiry_and_reclaim_fence_every_old_attempt_terminal() {
    let fixture = fresh_fixture("terminal-fencing").await;
    let clock = Arc::new(FixedClock::new(NOW));
    let repository = repository(
        &fixture.path,
        Arc::clone(&clock),
        Arc::new(SequenceAttemptIds::new(["attempt-old", "attempt-new"])),
    );
    let old = claim_token(&repository, BASE_SUBJECT).await;
    clock.set(NOW + 60);

    let expired_finish = repository
        .finish(
            FinishExecutionCommand::try_new(
                old.clone(),
                FinishExecutionDisposition::MetaReady,
                ExecutionPayloadDelta::Meta,
            )
            .unwrap(),
        )
        .await
        .expect_err("lease equality must reject finish");
    let expired_fail = repository
        .fail(
            FailExecutionCommand::try_new(
                old.clone(),
                "system",
                "expired",
                0,
                ExecutionPayloadDelta::Meta,
            )
            .unwrap(),
        )
        .await
        .expect_err("lease equality must reject failure");
    let expired_release = repository
        .release(ReleaseExecutionCommand::before_external_effect(old.clone()))
        .await
        .expect_err("lease equality must reject release");
    let expired_extend = repository
        .extend_lease(ExtendExecutionLeaseCommand::try_new(old.clone(), 60).unwrap())
        .await
        .expect_err("lease equality must reject extension");
    for error in [
        expired_finish,
        expired_fail,
        expired_release,
        expired_extend,
    ] {
        assert!(matches!(
            error,
            RepositoryError::LeaseExpired {
                lease_until,
                ..
            } if lease_until == NOW + 60
        ));
    }
    assert_eq!(audit_rows(&fixture.path).len(), 1);

    let new = claim_token(&repository, BASE_SUBJECT).await;
    assert_ne!(old.attempt_id(), new.attempt_id());
    for error in [
        repository
            .finish(
                FinishExecutionCommand::try_new(
                    old.clone(),
                    FinishExecutionDisposition::MetaReady,
                    ExecutionPayloadDelta::Meta,
                )
                .unwrap(),
            )
            .await
            .expect_err("old finish must be stale after reclaim"),
        repository
            .fail(
                FailExecutionCommand::try_new(
                    old.clone(),
                    "system",
                    "stale",
                    0,
                    ExecutionPayloadDelta::Meta,
                )
                .unwrap(),
            )
            .await
            .expect_err("old failure must be stale after reclaim"),
        repository
            .release(ReleaseExecutionCommand::before_external_effect(old.clone()))
            .await
            .expect_err("old release must be stale after reclaim"),
        repository
            .extend_lease(ExtendExecutionLeaseCommand::try_new(old, 60).unwrap())
            .await
            .expect_err("old extension must be stale after reclaim"),
    ] {
        assert!(matches!(
            error,
            RepositoryError::StaleAttempt {
                current: Some(ref current),
                ..
            } if current.as_ref() == &new
        ));
    }

    repository
        .release(ReleaseExecutionCommand::before_external_effect(new.clone()))
        .await
        .expect("current live attempt may release");
    let repeated = repository
        .release(ReleaseExecutionCommand::before_external_effect(new))
        .await
        .expect_err("consumed release must be stale");
    assert!(matches!(
        repeated,
        RepositoryError::StaleAttempt { current: None, .. }
    ));
    assert_eq!(
        audit_rows(&fixture.path)
            .into_iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        vec!["claim_attempt", "reclaim_attempt", "release_attempt"]
    );
}

#[tokio::test]
async fn every_post_claim_audit_failure_rolls_back_row_and_payload() {
    let fixture = fresh_fixture("terminal-audit-rollback").await;
    let clock = Arc::new(FixedClock::new(NOW));
    let repository = repository(
        &fixture.path,
        Arc::clone(&clock),
        Arc::new(SequenceAttemptIds::new(["attempt-audit-terminal"])),
    );
    let token = claim_token(&repository, BASE_SUBJECT).await;
    let before_controls = row_controls(&fixture.path, BASE_SUBJECT);
    let before_detail = repository
        .load_detail(key(BASE_SUBJECT))
        .await
        .expect("load pre-failure detail");
    let connection = Connection::open(&fixture.path).expect("open terminal audit trigger fixture");
    connection
        .execute_batch(
            r#"CREATE TRIGGER reject_post_claim_execution_audit
               BEFORE INSERT ON operation_logs
               WHEN NEW.category = 'subscription_scheduler'
                AND NEW.action IN (
                    'extend_attempt', 'finish_attempt', 'fail_attempt', 'release_attempt'
                )
               BEGIN
                   SELECT RAISE(ABORT, 'reject post-claim execution audit');
               END;"#,
        )
        .expect("install terminal audit rejection trigger");
    drop(connection);
    clock.set(NOW + 1);

    let errors = [
        repository
            .extend_lease(ExtendExecutionLeaseCommand::try_new(token.clone(), 60).unwrap())
            .await
            .expect_err("extend audit failure must roll back"),
        repository
            .finish(
                FinishExecutionCommand::try_new(
                    token.clone(),
                    FinishExecutionDisposition::MetaReady,
                    ExecutionPayloadDelta::Meta,
                )
                .unwrap(),
            )
            .await
            .expect_err("finish audit failure must roll back"),
        repository
            .fail(
                FailExecutionCommand::try_new(
                    token.clone(),
                    "system",
                    "rollback failure",
                    5,
                    ExecutionPayloadDelta::Meta,
                )
                .unwrap(),
            )
            .await
            .expect_err("failure audit failure must roll back"),
        repository
            .release(ReleaseExecutionCommand::before_external_effect(token))
            .await
            .expect_err("release audit failure must roll back"),
    ];
    assert!(errors
        .iter()
        .all(|error| matches!(error, RepositoryError::CorruptData { .. })));
    assert_eq!(
        row_controls(&fixture.path, BASE_SUBJECT),
        before_controls,
        "every terminal row mutation must roll back with its audit"
    );
    assert_eq!(
        repository
            .load_detail(key(BASE_SUBJECT))
            .await
            .expect("reload rolled-back detail"),
        before_detail
    );
    assert_eq!(
        audit_rows(&fixture.path)
            .into_iter()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        vec!["claim_attempt"]
    );
}
