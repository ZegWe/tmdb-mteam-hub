//! Latest-only execution application service.
//!
//! This module owns the claim/effect/terminal protocol. It never opens a
//! database path itself and never knows about obsolete subscription storage.
//! External providers and filesystem work live behind [`SubscriptionExecutionEffects`].

use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::task::JoinSet;

use super::ports::SubscriptionExecutionRepository;
use super::repository::{
    ClaimDueCommand, ClaimedSubscription, ExecutionPayloadDelta, FailExecutionCommand,
    FinishExecutionCommand, FinishExecutionDisposition, RepositoryError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionPolicy {
    pub(crate) enabled: bool,
    pub(crate) dry_run: bool,
    pub(crate) account_key: String,
    pub(crate) effects: ExecutionEffectPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct ExecutionEffectPolicy {
    pub(crate) douban_cookie: String,
    pub(crate) tmdb_api_key: String,
    pub(crate) mteam_api_key: String,
    pub(crate) qb_servers: Vec<ExecutionQbServer>,
    pub(crate) categories: Vec<ExecutionCategory>,
    pub(crate) torrent_match_rules: Vec<ExecutionTorrentMatchRule>,
    pub(crate) search_interval_secs: u64,
    pub(crate) progress_interval_secs: u64,
    pub(crate) link_retry_interval_secs: u64,
    pub(crate) system_retry_interval_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionQbServer {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) base_url: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) insecure_tls: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionCategory {
    pub(crate) name: String,
    pub(crate) wanted_tag: String,
    pub(crate) qb_server_id: String,
    pub(crate) qb_category: String,
    pub(crate) qb_save_dir_name: String,
    pub(crate) download_dir: String,
    pub(crate) link_target_dir: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionTorrentRuleMatchMode {
    All,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionTorrentMatchRule {
    pub(crate) name: String,
    pub(crate) priority: i32,
    pub(crate) mode: ExecutionTorrentRuleMatchMode,
    pub(crate) title_keywords: Vec<String>,
    pub(crate) resolution_keywords: Vec<String>,
    pub(crate) source_keywords: Vec<String>,
}

pub(crate) type ExecutionEffectFuture =
    Pin<Box<dyn Future<Output = ExecutionEffectResult> + Send + 'static>>;

/// Runtime boundary for one already-claimed operation.
///
/// Implementations must report whether work stopped before any external effect
/// could happen. Once a network mutation or filesystem mutation may have
/// happened, the only legal outcomes are `Finished` or `Failed`; the service
/// will never release that attempt.
pub(crate) trait SubscriptionExecutionEffects: Send + Sync {
    fn execute(
        &self,
        claimed: ClaimedSubscription,
        policy: ExecutionEffectPolicy,
    ) -> ExecutionEffectFuture;
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ExecutionEffectResult {
    Finished {
        disposition: FinishExecutionDisposition,
        payload_delta: ExecutionPayloadDelta,
    },
    Failed {
        error_type: String,
        message: String,
        retry_after_secs: u64,
        payload_delta: ExecutionPayloadDelta,
    },
}

#[derive(Clone)]
pub(crate) struct SubscriptionExecutionService {
    repository: Arc<dyn SubscriptionExecutionRepository>,
    effects: Arc<dyn SubscriptionExecutionEffects>,
    lease_ttl_secs: u64,
}

impl SubscriptionExecutionService {
    pub(crate) fn try_new(
        repository: Arc<dyn SubscriptionExecutionRepository>,
        effects: Arc<dyn SubscriptionExecutionEffects>,
        lease_ttl_secs: u64,
    ) -> Result<Self, SubscriptionExecutionError> {
        // Validate once at composition time. The repository command repeats
        // validation at the trust boundary before every claim.
        super::repository::ExecutionLeaseTtl::try_new(lease_ttl_secs)
            .map_err(SubscriptionExecutionError::repository)?;
        Ok(Self {
            repository,
            effects,
            lease_ttl_secs,
        })
    }

    /// Execute a bounded batch for one immutable configuration snapshot.
    ///
    /// Claims are issued only while an execution slot is available, so rows do
    /// not sit leased behind an in-memory queue. `batch_size` bounds claims for
    /// this call and `max_concurrency` bounds live effects.
    pub(crate) async fn run_due_batch(
        &self,
        policy: &ExecutionPolicy,
        batch_size: usize,
        max_concurrency: usize,
    ) -> Result<ExecutionBatchOutcome, SubscriptionExecutionError> {
        if !policy.enabled {
            return Ok(ExecutionBatchOutcome::disabled());
        }
        if policy.dry_run {
            return Ok(ExecutionBatchOutcome::dry_run());
        }
        if batch_size == 0 {
            return Err(SubscriptionExecutionError::invalid(
                "execution batch size must be greater than zero",
            ));
        }
        if max_concurrency == 0 || max_concurrency > batch_size {
            return Err(SubscriptionExecutionError::invalid(
                "execution concurrency must be between one and the batch size",
            ));
        }

        if policy.account_key.trim().is_empty() {
            return Err(SubscriptionExecutionError::invalid(
                "Douban authentication is not configured",
            ));
        }
        let account_key = policy.account_key.clone();
        let mut outcome = ExecutionBatchOutcome::active();
        let mut tasks = JoinSet::new();
        let mut no_more_due = false;

        while outcome.claimed < batch_size {
            while !no_more_due && tasks.len() < max_concurrency && outcome.claimed < batch_size {
                let command = ClaimDueCommand::try_new(account_key.clone(), self.lease_ttl_secs, 1)
                    .map_err(SubscriptionExecutionError::repository)?;
                let claim = self
                    .repository
                    .claim_due(command)
                    .await
                    .map_err(SubscriptionExecutionError::repository)?
                    .into_claim();
                let Some(claimed) = claim else {
                    no_more_due = true;
                    break;
                };
                outcome.claimed += 1;
                let service = self.clone();
                let effects = policy.effects.clone();
                tasks.spawn(async move { service.execute_claimed(claimed, effects).await });
            }

            let Some(joined) = tasks.join_next().await else {
                break;
            };
            match joined {
                Ok(Ok(ExecutionTerminal::Finished)) => outcome.finished += 1,
                Ok(Ok(ExecutionTerminal::Failed)) => outcome.failed += 1,
                Ok(Err(error)) => {
                    outcome.terminal_errors += 1;
                    tracing::warn!(error = %error, "subscription execution terminal failed");
                }
                Err(error) => {
                    outcome.terminal_errors += 1;
                    tracing::warn!(error = %error, "subscription execution task terminated unexpectedly");
                }
            }
        }

        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok(ExecutionTerminal::Finished)) => outcome.finished += 1,
                Ok(Ok(ExecutionTerminal::Failed)) => outcome.failed += 1,
                Ok(Err(error)) => {
                    outcome.terminal_errors += 1;
                    tracing::warn!(error = %error, "subscription execution terminal failed");
                }
                Err(error) => {
                    outcome.terminal_errors += 1;
                    tracing::warn!(error = %error, "subscription execution task terminated unexpectedly");
                }
            }
        }
        outcome.exhausted = no_more_due;
        Ok(outcome)
    }

    async fn execute_claimed(
        &self,
        claimed: ClaimedSubscription,
        policy: ExecutionEffectPolicy,
    ) -> Result<ExecutionTerminal, SubscriptionExecutionError> {
        let token = claimed.attempt().token().clone();
        match self.effects.execute(claimed, policy).await {
            ExecutionEffectResult::Finished {
                disposition,
                payload_delta,
            } => {
                let command = FinishExecutionCommand::try_new(token, disposition, payload_delta)
                    .map_err(SubscriptionExecutionError::repository)?;
                self.repository
                    .finish(command)
                    .await
                    .map_err(SubscriptionExecutionError::repository)?;
                Ok(ExecutionTerminal::Finished)
            }
            ExecutionEffectResult::Failed {
                error_type,
                message,
                retry_after_secs,
                payload_delta,
            } => {
                let command = FailExecutionCommand::try_new(
                    token,
                    safe_terminal_text(error_type, "execution_error"),
                    safe_terminal_text(message, "subscription execution failed"),
                    retry_after_secs,
                    payload_delta,
                )
                .map_err(SubscriptionExecutionError::repository)?;
                self.repository
                    .fail(command)
                    .await
                    .map_err(SubscriptionExecutionError::repository)?;
                Ok(ExecutionTerminal::Failed)
            }
        }
    }
}

fn safe_terminal_text(value: String, fallback: &str) -> String {
    let value = value.replace('\0', "");
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExecutionTerminal {
    Finished,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ExecutionBatchMode {
    Active,
    Disabled,
    DryRun,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExecutionBatchOutcome {
    pub(crate) mode: ExecutionBatchMode,
    pub(crate) claimed: usize,
    pub(crate) finished: usize,
    pub(crate) failed: usize,
    pub(crate) terminal_errors: usize,
    pub(crate) exhausted: bool,
}

impl ExecutionBatchOutcome {
    const fn active() -> Self {
        Self {
            mode: ExecutionBatchMode::Active,
            claimed: 0,
            finished: 0,
            failed: 0,
            terminal_errors: 0,
            exhausted: false,
        }
    }

    const fn disabled() -> Self {
        Self {
            mode: ExecutionBatchMode::Disabled,
            ..Self::active()
        }
    }

    const fn dry_run() -> Self {
        Self {
            mode: ExecutionBatchMode::DryRun,
            ..Self::active()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SubscriptionExecutionError {
    InvalidConfiguration { message: String },
    Repository(RepositoryError),
}

impl SubscriptionExecutionError {
    fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidConfiguration {
            message: message.into(),
        }
    }

    fn repository(error: RepositoryError) -> Self {
        Self::Repository(error)
    }
}

impl fmt::Display for SubscriptionExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfiguration { message } => formatter.write_str(message),
            Self::Repository(error) => write!(formatter, "subscription repository: {error}"),
        }
    }
}

impl Error for SubscriptionExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidConfiguration { .. } => None,
            Self::Repository(error) => Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use rusqlite::{params, Connection};

    use super::*;
    use crate::storage::SqliteSubscriptionRepository;
    use crate::subscription::effects::stable_qb_idempotency_key;
    use crate::subscription::ports::{
        RepoFuture, SubscriptionPollRepository, SubscriptionReadRepository,
    };
    use crate::subscription::repository::{
        ApplyCompleteSnapshotCommand, BeginPollCommand, ClaimDueResult, ClaimOneCommand,
        ClaimOneResult, ExecutionOperation, ExtendExecutionLeaseCommand,
        ExtendExecutionLeaseResult, FailExecutionResult, FinishExecutionResult, NewRecordPolicy,
        ReleaseExecutionCommand, ReleaseExecutionResult, SnapshotRecord, SubscriptionKey,
        WantedSourcePayload,
    };
    use crate::subscription::{
        SubscriptionExecutionState, SubscriptionLifecycleState, SubscriptionMediaKind,
    };

    #[derive(Clone, Copy)]
    enum FakeEffectMode {
        Finish,
        Fail,
    }

    struct FakeEffects {
        mode: FakeEffectMode,
        calls: AtomicUsize,
        current: Arc<AtomicUsize>,
        maximum: Arc<AtomicUsize>,
        delay: Duration,
    }

    impl FakeEffects {
        fn new(mode: FakeEffectMode) -> Self {
            Self {
                mode,
                calls: AtomicUsize::new(0),
                current: Arc::new(AtomicUsize::new(0)),
                maximum: Arc::new(AtomicUsize::new(0)),
                delay: Duration::ZERO,
            }
        }

        fn delayed(delay: Duration) -> Self {
            Self {
                delay,
                ..Self::new(FakeEffectMode::Finish)
            }
        }
    }

    impl SubscriptionExecutionEffects for FakeEffects {
        fn execute(
            &self,
            claimed: ClaimedSubscription,
            _policy: ExecutionEffectPolicy,
        ) -> ExecutionEffectFuture {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(current, Ordering::SeqCst);
            let mode = self.mode;
            let delay = self.delay;
            let current_counter = self.current.clone();
            Box::pin(async move {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                current_counter.fetch_sub(1, Ordering::SeqCst);
                let operation = claimed.attempt().token().operation();
                match mode {
                    FakeEffectMode::Finish => ExecutionEffectResult::Finished {
                        disposition: match operation {
                            super::super::repository::ExecutionOperation::Meta => {
                                FinishExecutionDisposition::MetaReady
                            }
                            super::super::repository::ExecutionOperation::Search => {
                                FinishExecutionDisposition::SearchWaiting {
                                    retry_after:
                                        super::super::repository::ExecutionScheduleDelay::try_new(
                                            30,
                                        )
                                        .unwrap(),
                                }
                            }
                            _ => panic!("fixture handles only Meta and Search operations"),
                        },
                        payload_delta: match operation {
                            super::super::repository::ExecutionOperation::Meta => {
                                ExecutionPayloadDelta::Meta
                            }
                            super::super::repository::ExecutionOperation::Search => {
                                ExecutionPayloadDelta::Search {
                                    candidates: None,
                                    download_updates: Vec::new(),
                                }
                            }
                            _ => unreachable!(),
                        },
                    },
                    FakeEffectMode::Fail => ExecutionEffectResult::Failed {
                        error_type: "fixture_failure".to_string(),
                        message: "fixture failed\0".to_string(),
                        retry_after_secs: 30,
                        payload_delta: ExecutionPayloadDelta::Meta,
                    },
                }
            })
        }
    }

    struct StaleFinishRepository {
        inner: Arc<SqliteSubscriptionRepository>,
        attempted: Arc<Mutex<Option<super::super::repository::ExecutionAttemptToken>>>,
    }

    impl SubscriptionExecutionRepository for StaleFinishRepository {
        fn claim_due(&self, command: ClaimDueCommand) -> RepoFuture<ClaimDueResult> {
            let inner = self.inner.clone();
            let attempted = self.attempted.clone();
            Box::pin(async move {
                let result = inner.claim_due(command).await?;
                if let Some(claimed) = result.claim() {
                    *attempted.lock().expect("lock attempted token") =
                        Some(claimed.attempt().token().clone());
                }
                Ok(result)
            })
        }

        fn claim_one(&self, command: ClaimOneCommand) -> RepoFuture<ClaimOneResult> {
            self.inner.claim_one(command)
        }

        fn extend_lease(
            &self,
            command: ExtendExecutionLeaseCommand,
        ) -> RepoFuture<ExtendExecutionLeaseResult> {
            self.inner.extend_lease(command)
        }

        fn finish(&self, command: FinishExecutionCommand) -> RepoFuture<FinishExecutionResult> {
            let token = command.token().clone();
            Box::pin(async move {
                Err(RepositoryError::StaleAttempt {
                    attempted: Box::new(token),
                    current: None,
                })
            })
        }

        fn fail(&self, command: FailExecutionCommand) -> RepoFuture<FailExecutionResult> {
            self.inner.fail(command)
        }

        fn release(&self, command: ReleaseExecutionCommand) -> RepoFuture<ReleaseExecutionResult> {
            self.inner.release(command)
        }
    }

    struct FailOnceFinishAuditRepository {
        inner: Arc<SqliteSubscriptionRepository>,
        database: PathBuf,
        fail_once: AtomicBool,
        finish_calls: AtomicUsize,
        claimed_attempt_ids: Arc<Mutex<Vec<String>>>,
    }

    impl FailOnceFinishAuditRepository {
        fn new(inner: Arc<SqliteSubscriptionRepository>, database: PathBuf) -> Self {
            Self {
                inner,
                database,
                fail_once: AtomicBool::new(true),
                finish_calls: AtomicUsize::new(0),
                claimed_attempt_ids: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl SubscriptionExecutionRepository for FailOnceFinishAuditRepository {
        fn claim_due(&self, command: ClaimDueCommand) -> RepoFuture<ClaimDueResult> {
            let inner = self.inner.clone();
            let claimed_attempt_ids = self.claimed_attempt_ids.clone();
            Box::pin(async move {
                let result = inner.claim_due(command).await?;
                if let Some(claimed) = result.claim() {
                    claimed_attempt_ids
                        .lock()
                        .expect("lock claimed attempt IDs")
                        .push(claimed.attempt().token().attempt_id().as_str().to_string());
                }
                Ok(result)
            })
        }

        fn claim_one(&self, command: ClaimOneCommand) -> RepoFuture<ClaimOneResult> {
            self.inner.claim_one(command)
        }

        fn extend_lease(
            &self,
            command: ExtendExecutionLeaseCommand,
        ) -> RepoFuture<ExtendExecutionLeaseResult> {
            self.inner.extend_lease(command)
        }

        fn finish(&self, command: FinishExecutionCommand) -> RepoFuture<FinishExecutionResult> {
            self.finish_calls.fetch_add(1, Ordering::SeqCst);
            let fail_once = self.fail_once.swap(false, Ordering::SeqCst);
            let inner = self.inner.clone();
            let database = self.database.clone();
            let token = command.token().clone();
            Box::pin(async move {
                let result = inner.finish(command).await;
                if fail_once {
                    let rejected_by_audit =
                        matches!(&result, Err(RepositoryError::CorruptData { .. }));
                    let connection = Connection::open(database)
                        .expect("open fail-once finish audit fixture database");
                    connection
                        .execute_batch("DROP TRIGGER reject_finish_audit_once;")
                        .expect("remove fail-once finish audit trigger");
                    let changed = connection
                        .execute(
                            r#"UPDATE wanted_subscription_records
                                  SET lease_until = 0
                                WHERE account_key = ?1
                                  AND subject_id = ?2
                                  AND attempt_id = ?3
                                  AND execution_state = 'running'"#,
                            params![
                                token.key().account_key,
                                token.key().subject_id,
                                token.attempt_id().as_str(),
                            ],
                        )
                        .expect("expire rolled-back execution lease for deterministic retry");
                    assert!(
                        rejected_by_audit,
                        "the injected finish audit failure must reject finish as corrupt data"
                    );
                    assert_eq!(
                        changed, 1,
                        "failed finish must leave the original attempt running"
                    );
                }
                result
            })
        }

        fn fail(&self, command: FailExecutionCommand) -> RepoFuture<FailExecutionResult> {
            self.inner.fail(command)
        }

        fn release(&self, command: ReleaseExecutionCommand) -> RepoFuture<ReleaseExecutionResult> {
            self.inner.release(command)
        }
    }

    #[derive(Default)]
    struct IdempotentQbEffects {
        execute_calls: AtomicUsize,
        physical_adds: AtomicUsize,
        observed_keys: Mutex<BTreeSet<String>>,
    }

    impl SubscriptionExecutionEffects for IdempotentQbEffects {
        fn execute(
            &self,
            claimed: ClaimedSubscription,
            _policy: ExecutionEffectPolicy,
        ) -> ExecutionEffectFuture {
            self.execute_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(
                claimed.attempt().token().operation(),
                ExecutionOperation::Search,
                "fixture models the qB add search effect"
            );
            let key = claimed.attempt().token().key();
            let effect_key =
                stable_qb_idempotency_key(&key.account_key, &key.subject_id, "fixture-torrent-42");
            let inserted = self
                .observed_keys
                .lock()
                .expect("lock idempotent effect keys")
                .insert(effect_key);
            if inserted {
                self.physical_adds.fetch_add(1, Ordering::SeqCst);
            }
            Box::pin(async {
                ExecutionEffectResult::Finished {
                    disposition: FinishExecutionDisposition::SearchPushed,
                    payload_delta: ExecutionPayloadDelta::Search {
                        candidates: None,
                        download_updates: Vec::new(),
                    },
                }
            })
        }
    }

    fn temp_database(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-execution-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create execution fixture directory");
        root.join("subscriptions.sqlite")
    }

    async fn repository_with_movies_and_path(
        label: &str,
        subjects: &[&str],
    ) -> (Arc<SqliteSubscriptionRepository>, PathBuf) {
        let database = temp_database(label);
        let repository = Arc::new(
            SqliteSubscriptionRepository::try_create_fresh(
                database.clone(),
                4,
                Duration::from_secs(1),
            )
            .expect("create latest execution fixture repository"),
        );
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_secs();
        let begin = repository
            .begin_poll(BeginPollCommand::try_new("account-1", now).unwrap())
            .await
            .unwrap();
        let records = subjects
            .iter()
            .map(|subject| {
                SnapshotRecord::try_new(
                    *subject,
                    SubscriptionMediaKind::Movie,
                    true,
                    None,
                    WantedSourcePayload {
                        title: format!("Fixture {subject}"),
                        release_year: Some(2026),
                        tags: vec!["电影".to_string()],
                        ..WantedSourcePayload::default()
                    },
                )
                .unwrap()
            })
            .collect();
        repository
            .apply_complete_snapshot(
                ApplyCompleteSnapshotCommand::try_new(
                    "account-1",
                    begin.token,
                    now,
                    now.saturating_add(3600),
                    NewRecordPolicy::try_new(3, false).unwrap(),
                    records,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        (repository, database)
    }

    async fn repository_with_movies(
        label: &str,
        subjects: &[&str],
    ) -> Arc<SqliteSubscriptionRepository> {
        repository_with_movies_and_path(label, subjects).await.0
    }

    fn install_finish_audit_rejection(database: &Path) {
        let connection = Connection::open(database).expect("open finish audit fixture database");
        connection
            .execute_batch(
                r#"CREATE TRIGGER reject_finish_audit_once
                   BEFORE INSERT ON operation_logs
                   WHEN NEW.category = 'subscription_scheduler'
                    AND NEW.action = 'finish_attempt'
                   BEGIN
                       SELECT RAISE(ABORT, 'reject finish audit once');
                   END;"#,
            )
            .expect("install fail-once finish audit trigger");
    }

    fn finish_audit_count(database: &Path) -> u64 {
        Connection::open(database)
            .expect("open finish audit count database")
            .query_row(
                r#"SELECT count(*)
                     FROM operation_logs
                    WHERE category = 'subscription_scheduler'
                      AND action = 'finish_attempt'"#,
                [],
                |row| row.get(0),
            )
            .expect("count finish audit rows")
    }

    fn live_policy() -> ExecutionPolicy {
        ExecutionPolicy {
            enabled: true,
            dry_run: false,
            account_key: "account-1".to_string(),
            effects: ExecutionEffectPolicy::default(),
        }
    }

    #[tokio::test]
    async fn disabled_and_dry_run_modes_never_claim() {
        let repository = repository_with_movies("disabled-dry", &["subject-1"]).await;
        let effects = Arc::new(FakeEffects::new(FakeEffectMode::Finish));
        let service =
            SubscriptionExecutionService::try_new(repository.clone(), effects.clone(), 60).unwrap();
        let mut disabled = live_policy();
        disabled.enabled = false;
        let mut dry_run = live_policy();
        dry_run.dry_run = true;

        assert_eq!(
            service.run_due_batch(&disabled, 1, 1).await.unwrap().mode,
            ExecutionBatchMode::Disabled
        );
        assert_eq!(
            service.run_due_batch(&dry_run, 1, 1).await.unwrap().mode,
            ExecutionBatchMode::DryRun
        );
        assert_eq!(effects.calls.load(Ordering::SeqCst), 0);
        let detail = repository
            .load_detail(SubscriptionKey::try_new("account-1", "subject-1").unwrap())
            .await
            .unwrap();
        assert_eq!(
            detail.summary().head.execution_state,
            SubscriptionExecutionState::Idle
        );
        assert_eq!(
            detail.summary().head.lifecycle_state,
            SubscriptionLifecycleState::Queued
        );
    }

    #[tokio::test]
    async fn exact_finish_and_failure_are_persisted_by_the_application_service() {
        let finished_repository = repository_with_movies("finish", &["subject-1"]).await;
        let finish_service = SubscriptionExecutionService::try_new(
            finished_repository.clone(),
            Arc::new(FakeEffects::new(FakeEffectMode::Finish)),
            60,
        )
        .unwrap();
        let finish = finish_service
            .run_due_batch(&live_policy(), 1, 1)
            .await
            .unwrap();
        assert_eq!((finish.claimed, finish.finished, finish.failed), (1, 1, 0));
        let detail = finished_repository
            .load_detail(SubscriptionKey::try_new("account-1", "subject-1").unwrap())
            .await
            .unwrap();
        assert_eq!(
            detail.summary().head.lifecycle_state,
            SubscriptionLifecycleState::Searching
        );

        let failed_repository = repository_with_movies("fail", &["subject-2"]).await;
        let fail_service = SubscriptionExecutionService::try_new(
            failed_repository.clone(),
            Arc::new(FakeEffects::new(FakeEffectMode::Fail)),
            60,
        )
        .unwrap();
        let failure = fail_service
            .run_due_batch(&live_policy(), 1, 1)
            .await
            .unwrap();
        assert_eq!(
            (failure.claimed, failure.finished, failure.failed),
            (1, 0, 1)
        );
        let detail = failed_repository
            .load_detail(SubscriptionKey::try_new("account-1", "subject-2").unwrap())
            .await
            .unwrap();
        assert_eq!(
            detail.summary().head.lifecycle_state,
            SubscriptionLifecycleState::Meta
        );
        assert_eq!(detail.summary().head.retry_count, 1);
        assert_eq!(detail.payload().issues[0].message, "fixture failed");
    }

    #[tokio::test]
    async fn batch_size_and_concurrency_bound_live_effects() {
        let repository = repository_with_movies(
            "bounded",
            &["subject-1", "subject-2", "subject-3", "subject-4"],
        )
        .await;
        let effects = Arc::new(FakeEffects::delayed(Duration::from_millis(20)));
        let service =
            SubscriptionExecutionService::try_new(repository, effects.clone(), 60).unwrap();

        let outcome = service.run_due_batch(&live_policy(), 4, 2).await.unwrap();

        assert_eq!(outcome.claimed, 4);
        assert_eq!(outcome.finished, 4);
        assert!(effects.maximum.load(Ordering::SeqCst) <= 2);
    }

    #[tokio::test]
    async fn stale_attempt_finish_is_rejected_and_never_reported_as_success() {
        let inner = repository_with_movies("stale", &["subject-1"]).await;
        let wrapper = Arc::new(StaleFinishRepository {
            inner: inner.clone(),
            attempted: Arc::new(Mutex::new(None)),
        });
        let service = SubscriptionExecutionService::try_new(
            wrapper.clone(),
            Arc::new(FakeEffects::new(FakeEffectMode::Finish)),
            60,
        )
        .unwrap();

        let outcome = service.run_due_batch(&live_policy(), 1, 1).await.unwrap();

        assert_eq!(outcome.finished, 0);
        assert_eq!(outcome.terminal_errors, 1);
        assert!(wrapper
            .attempted
            .lock()
            .expect("lock attempted token")
            .is_some());
        let detail = inner
            .load_detail(SubscriptionKey::try_new("account-1", "subject-1").unwrap())
            .await
            .unwrap();
        assert_eq!(
            detail.summary().head.execution_state,
            SubscriptionExecutionState::Running
        );
    }

    #[tokio::test]
    async fn finish_audit_failure_after_qb_effect_retries_without_a_second_physical_add() {
        let (inner, database) =
            repository_with_movies_and_path("finish-audit-effect-retry", &["subject-1"]).await;

        let meta_service = SubscriptionExecutionService::try_new(
            inner.clone(),
            Arc::new(FakeEffects::new(FakeEffectMode::Finish)),
            60,
        )
        .unwrap();
        let meta = meta_service
            .run_due_batch(&live_policy(), 1, 1)
            .await
            .unwrap();
        assert_eq!((meta.claimed, meta.finished), (1, 1));
        let audits_before_failure = finish_audit_count(&database);

        install_finish_audit_rejection(&database);
        let repository = Arc::new(FailOnceFinishAuditRepository::new(
            inner.clone(),
            database.clone(),
        ));
        let effects = Arc::new(IdempotentQbEffects::default());
        let service =
            SubscriptionExecutionService::try_new(repository.clone(), effects.clone(), 60).unwrap();

        let failed_terminal = service.run_due_batch(&live_policy(), 1, 1).await.unwrap();
        assert_eq!(failed_terminal.claimed, 1);
        assert_eq!(failed_terminal.finished, 0);
        assert_eq!(failed_terminal.terminal_errors, 1);
        assert_eq!(effects.execute_calls.load(Ordering::SeqCst), 1);
        assert_eq!(effects.physical_adds.load(Ordering::SeqCst), 1);
        assert_eq!(finish_audit_count(&database), audits_before_failure);
        let rolled_back = inner
            .load_detail(SubscriptionKey::try_new("account-1", "subject-1").unwrap())
            .await
            .unwrap();
        assert_eq!(
            rolled_back.summary().head.lifecycle_state,
            SubscriptionLifecycleState::Searching
        );
        assert_eq!(
            rolled_back.summary().head.execution_state,
            SubscriptionExecutionState::Running
        );

        let recovered = service.run_due_batch(&live_policy(), 1, 1).await.unwrap();
        assert_eq!((recovered.claimed, recovered.finished), (1, 1));
        assert_eq!(recovered.terminal_errors, 0);
        assert_eq!(effects.execute_calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            effects.physical_adds.load(Ordering::SeqCst),
            1,
            "retry must reconcile the stable effect instead of adding again"
        );
        assert_eq!(effects.observed_keys.lock().unwrap().len(), 1);
        assert_eq!(repository.finish_calls.load(Ordering::SeqCst), 2);
        {
            let attempt_ids = repository.claimed_attempt_ids.lock().unwrap();
            assert_eq!(attempt_ids.len(), 2);
            assert_ne!(
                attempt_ids[0], attempt_ids[1],
                "expired retry must use a new fenced attempt"
            );
        }
        assert_eq!(finish_audit_count(&database), audits_before_failure + 1);
        let detail = inner
            .load_detail(SubscriptionKey::try_new("account-1", "subject-1").unwrap())
            .await
            .unwrap();
        assert_eq!(
            detail.summary().head.lifecycle_state,
            SubscriptionLifecycleState::Downloading
        );
        assert_eq!(
            detail.summary().head.execution_state,
            SubscriptionExecutionState::Idle
        );
    }
}
