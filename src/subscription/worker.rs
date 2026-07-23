use std::error::Error;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::execution::{
    ExecutionBatchOutcome, ExecutionCategory, ExecutionEffectPolicy, ExecutionPolicy,
    ExecutionQbServer, ExecutionTorrentMatchRule, ExecutionTorrentRuleMatchMode,
    SubscriptionExecutionService,
};
use super::ports::SubscriptionPollRepository;
use super::repository::payload::WantedSourcePayload;
use super::repository::{
    ApplyCompleteSnapshotCommand, BeginPollCommand, IncompleteSnapshotObservation,
    IncompleteSnapshotReason, NewRecordPolicy, PollRetryPolicy, RecordIncompleteSnapshotCommand,
    RecordPollFailureCommand, RepositoryError, SnapshotRecord,
};
use super::SubscriptionMediaKind;
use crate::app::redaction::SubscriptionDiagnosticRedactor;
use crate::config::{ConfigManager, FileConfig, SubscriptionWatcherConfig, TorrentRuleMatchMode};
use crate::douban;

const DOUBAN_LIBRARY_MAX_PAGES: usize = 80;

pub(crate) type WantedSourceFuture =
    Pin<Box<dyn Future<Output = Result<WantedSnapshot, WantedSourceError>> + Send + 'static>>;

pub(crate) trait WantedSource: Send + Sync {
    fn fetch_wanted(&self, cookie_header: String, limit: usize) -> WantedSourceFuture;
}

#[derive(Debug)]
pub(crate) struct WantedSourceError {
    message: String,
}

impl WantedSourceError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for WantedSourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for WantedSourceError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WantedSnapshot {
    pub(crate) items: Vec<WantedItem>,
    pub(crate) metadata: WantedSnapshotMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WantedItem {
    pub(crate) subject_id: String,
    pub(crate) title: String,
    pub(crate) abstract_text: String,
    pub(crate) abstract_2: String,
    pub(crate) cover_url: String,
    pub(crate) poster_url: String,
    pub(crate) date: String,
    pub(crate) tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WantedSnapshotMetadata {
    pub(crate) completeness: WantedSnapshotCompleteness,
    pub(crate) fetched_pages: usize,
    pub(crate) truncated_by_limit: bool,
    pub(crate) end_observed: bool,
}

impl WantedSnapshotMetadata {
    fn is_complete(self) -> bool {
        self.completeness == WantedSnapshotCompleteness::Complete && self.end_observed
    }
}

#[cfg(test)]
impl WantedSnapshotMetadata {
    fn complete(fetched_pages: usize) -> Self {
        Self {
            completeness: WantedSnapshotCompleteness::Complete,
            fetched_pages,
            truncated_by_limit: false,
            end_observed: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WantedSnapshotCompleteness {
    Complete,
    Partial,
}

trait PollClock: Send + Sync {
    fn now(&self) -> u64;
}

struct SystemPollClock;

impl PollClock for SystemPollClock {
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default()
    }
}

/// Shared application service used by both the manual HTTP command and the worker.
///
/// The service owns the complete Poll transaction protocol: it persists the open
/// token before touching the network, translates one source snapshot, and consumes
/// the exact token with one complete, incomplete, or failure terminal.
#[derive(Clone)]
pub(crate) struct SubscriptionPollService {
    repository: Arc<dyn SubscriptionPollRepository>,
    source: Arc<dyn WantedSource>,
    clock: Arc<dyn PollClock>,
}

#[derive(Debug, Clone)]
pub(crate) struct SubscriptionPollPolicy {
    account_key: String,
    cookie_header: String,
    library_limit: usize,
    poll_interval_secs: u64,
    new_record_policy: NewRecordPolicy,
    retry_policy: PollRetryPolicy,
    redactor: SubscriptionDiagnosticRedactor,
}

impl SubscriptionPollService {
    pub(crate) fn new(
        repository: Arc<dyn SubscriptionPollRepository>,
        source: Arc<dyn WantedSource>,
    ) -> Self {
        Self {
            repository,
            source,
            clock: Arc::new(SystemPollClock),
        }
    }

    #[cfg(test)]
    fn with_dependencies(
        repository: Arc<dyn SubscriptionPollRepository>,
        source: Arc<dyn WantedSource>,
        clock: Arc<dyn PollClock>,
    ) -> Self {
        Self {
            repository,
            source,
            clock,
        }
    }

    pub(crate) async fn poll(
        &self,
        policy: &SubscriptionPollPolicy,
    ) -> Result<SubscriptionPollOutcome, SubscriptionPollError> {
        let account_key = policy.account_key.clone();
        let retry_policy = policy.retry_policy;
        let attempted_at = self.clock.now();
        let begin_command = BeginPollCommand::try_new(account_key.clone(), attempted_at)
            .map_err(SubscriptionPollError::repository)?;
        let begin = self
            .repository
            .begin_poll(begin_command)
            .await
            .map_err(SubscriptionPollError::repository)?;

        let source = match self
            .source
            .fetch_wanted(policy.cookie_header.clone(), policy.library_limit)
            .await
        {
            Ok(source) => source,
            Err(error) => {
                let safe_message = valid_poll_failure_message(
                    policy
                        .redactor
                        .redact_or(&error.to_string(), "wanted subscription Poll failed"),
                    "wanted subscription Poll failed",
                );
                let failed_at = self.clock.now().max(attempted_at);
                return self
                    .fail_open_poll(
                        &account_key,
                        begin.token,
                        failed_at,
                        SubscriptionPollErrorKind::Upstream,
                        safe_message,
                        retry_policy,
                    )
                    .await;
            }
        };

        let snapshot_terminal = match validated_snapshot_terminal(source.metadata) {
            Ok(terminal) => terminal,
            Err(error) => {
                let safe_message =
                    "wanted subscription source returned inconsistent snapshot metadata"
                        .to_string();
                let failed_at = self.clock.now().max(attempted_at);
                tracing::warn!(error = %error, "invalid wanted subscription snapshot metadata");
                return self
                    .fail_open_poll(
                        &account_key,
                        begin.token,
                        failed_at,
                        SubscriptionPollErrorKind::Internal,
                        safe_message,
                        retry_policy,
                    )
                    .await;
            }
        };

        let records = match snapshot_records(&source.items) {
            Ok(records) => records,
            Err(error) => {
                let safe_message =
                    "wanted subscription source returned invalid records".to_string();
                let failed_at = self.clock.now().max(attempted_at);
                tracing::warn!(error = %error, "invalid wanted subscription source snapshot");
                return self
                    .fail_open_poll(
                        &account_key,
                        begin.token,
                        failed_at,
                        SubscriptionPollErrorKind::Internal,
                        safe_message,
                        retry_policy,
                    )
                    .await;
            }
        };

        let terminal_at = self.clock.now().max(attempted_at);
        let snapshot_id = begin.token.snapshot_id.as_str().to_string();
        let fetched_items = records.len();

        match snapshot_terminal {
            SnapshotTerminal::Complete => {
                let next_poll_at = terminal_at.saturating_add(policy.poll_interval_secs);
                let command = match ApplyCompleteSnapshotCommand::try_new(
                    account_key.clone(),
                    begin.token.clone(),
                    terminal_at,
                    next_poll_at,
                    policy.new_record_policy,
                    records,
                ) {
                    Ok(command) => command,
                    Err(error) => {
                        let safe_message =
                            "wanted subscription source returned invalid complete snapshot records"
                                .to_string();
                        tracing::warn!(error = %error, "invalid complete wanted subscription snapshot command");
                        return self
                            .fail_open_poll(
                                &account_key,
                                begin.token,
                                terminal_at,
                                SubscriptionPollErrorKind::Internal,
                                safe_message,
                                retry_policy,
                            )
                            .await;
                    }
                };
                let result = self
                    .repository
                    .apply_complete_snapshot(command)
                    .await
                    .map_err(SubscriptionPollError::repository)?;
                Ok(SubscriptionPollOutcome {
                    snapshot_id,
                    snapshot_complete: true,
                    fetched_items,
                    inserted: result.inserted,
                    updated: result.updated,
                    unchanged: result.unchanged,
                    reactivated: result.reactivated,
                    deactivated: result.deactivated,
                    failure_count: 0,
                    next_poll_at: result.next_poll_at,
                    polled_at: result.completed_at,
                })
            }
            SnapshotTerminal::Incomplete(observation) => {
                let command = match RecordIncompleteSnapshotCommand::try_new(
                    account_key.clone(),
                    begin.token.clone(),
                    terminal_at,
                    observation,
                    policy.new_record_policy,
                    records,
                    retry_policy,
                ) {
                    Ok(command) => command,
                    Err(error) => {
                        let safe_message = "wanted subscription source returned invalid incomplete snapshot records".to_string();
                        tracing::warn!(error = %error, "invalid incomplete wanted subscription snapshot command");
                        return self
                            .fail_open_poll(
                                &account_key,
                                begin.token,
                                terminal_at,
                                SubscriptionPollErrorKind::Internal,
                                safe_message,
                                retry_policy,
                            )
                            .await;
                    }
                };
                let result = self
                    .repository
                    .record_incomplete_snapshot(command)
                    .await
                    .map_err(SubscriptionPollError::repository)?;
                Ok(SubscriptionPollOutcome {
                    snapshot_id,
                    snapshot_complete: false,
                    fetched_items,
                    inserted: result.inserted,
                    updated: result.updated,
                    unchanged: result.unchanged,
                    reactivated: result.reactivated,
                    deactivated: 0,
                    failure_count: result.failure_count,
                    next_poll_at: result.next_poll_at,
                    polled_at: result.incomplete_at,
                })
            }
        }
    }

    pub(crate) async fn next_poll_at(
        &self,
        policy: &SubscriptionPollPolicy,
    ) -> Result<Option<u64>, SubscriptionPollError> {
        self.repository
            .load_poll_schedule(policy.account_key.clone())
            .await
            .map(|schedule| schedule.next_poll_at())
            .map_err(SubscriptionPollError::repository)
    }

    async fn fail_open_poll<T>(
        &self,
        account_key: &str,
        token: super::repository::PollAttemptToken,
        failed_at: u64,
        kind: SubscriptionPollErrorKind,
        message: String,
        retry_policy: PollRetryPolicy,
    ) -> Result<T, SubscriptionPollError> {
        let next_poll_at = self
            .persist_failure(account_key, token, failed_at, message.clone(), retry_policy)
            .await?;
        Err(SubscriptionPollError::new(kind, message).with_next_poll_at(next_poll_at))
    }

    async fn persist_failure(
        &self,
        account_key: &str,
        token: super::repository::PollAttemptToken,
        failed_at: u64,
        message: String,
        retry_policy: PollRetryPolicy,
    ) -> Result<u64, SubscriptionPollError> {
        self.repository
            .record_poll_failure(
                RecordPollFailureCommand::try_new(
                    account_key,
                    token,
                    failed_at,
                    message,
                    retry_policy,
                )
                .map_err(SubscriptionPollError::repository)?,
            )
            .await
            .map(|result| result.next_poll_at)
            .map_err(SubscriptionPollError::repository)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SubscriptionPollOutcome {
    pub(crate) snapshot_id: String,
    pub(crate) snapshot_complete: bool,
    pub(crate) fetched_items: usize,
    pub(crate) inserted: usize,
    pub(crate) updated: usize,
    pub(crate) unchanged: usize,
    pub(crate) reactivated: usize,
    pub(crate) deactivated: usize,
    pub(crate) failure_count: u32,
    pub(crate) next_poll_at: u64,
    pub(crate) polled_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubscriptionPollErrorKind {
    Validation,
    Upstream,
    Conflict,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubscriptionPollError {
    kind: SubscriptionPollErrorKind,
    message: String,
    next_poll_at: Option<u64>,
}

impl SubscriptionPollError {
    fn new(kind: SubscriptionPollErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            next_poll_at: None,
        }
    }

    fn with_next_poll_at(mut self, next_poll_at: u64) -> Self {
        self.next_poll_at = Some(next_poll_at);
        self
    }

    fn repository(error: RepositoryError) -> Self {
        let kind = match error {
            RepositoryError::InvalidInput { .. } => SubscriptionPollErrorKind::Internal,
            RepositoryError::StalePoll { .. } => SubscriptionPollErrorKind::Conflict,
            RepositoryError::UnsupportedSchema { .. } | RepositoryError::Unavailable { .. } => {
                SubscriptionPollErrorKind::Unavailable
            }
            RepositoryError::NotFound { .. }
            | RepositoryError::RevisionConflict { .. }
            | RepositoryError::ExecutionGateConflict { .. }
            | RepositoryError::StaleAttempt { .. }
            | RepositoryError::LeaseExpired { .. }
            | RepositoryError::LeaseNotExtended { .. }
            | RepositoryError::CorruptData { .. }
            | RepositoryError::Internal { .. } => SubscriptionPollErrorKind::Internal,
        };
        Self::new(kind, error.to_string())
    }

    pub(crate) const fn kind(&self) -> SubscriptionPollErrorKind {
        self.kind
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for SubscriptionPollError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for SubscriptionPollError {}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub(crate) enum SubscriptionWorkerTick {
    Disabled,
    Polled(SubscriptionPollOutcome),
}

#[cfg(test)]
pub(crate) async fn run_worker_tick(
    config: &ConfigManager,
    poll_service: &SubscriptionPollService,
) -> Result<SubscriptionWorkerTick, SubscriptionPollError> {
    let snapshot = config.snapshot().await;
    if !snapshot.value.subscription_watcher.enabled {
        return Ok(SubscriptionWorkerTick::Disabled);
    }
    let policy = subscription_poll_policy(&snapshot.value)?;
    poll_service
        .poll(&policy)
        .await
        .map(SubscriptionWorkerTick::Polled)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SubscriptionWorkerOptions {
    pub(crate) execution_batch_size: usize,
    pub(crate) execution_concurrency: usize,
    pub(crate) idle_execution_interval_secs: u64,
    pub(crate) execution_jitter_secs: u64,
}

impl SubscriptionWorkerOptions {
    pub(crate) fn try_new(
        execution_batch_size: usize,
        execution_concurrency: usize,
        idle_execution_interval_secs: u64,
        execution_jitter_secs: u64,
    ) -> Result<Self, &'static str> {
        if execution_batch_size == 0 {
            return Err("execution batch size must be greater than zero");
        }
        if execution_concurrency == 0 || execution_concurrency > execution_batch_size {
            return Err("execution concurrency must be between one and the batch size");
        }
        if idle_execution_interval_secs == 0 {
            return Err("idle execution interval must be greater than zero");
        }
        Ok(Self {
            execution_batch_size,
            execution_concurrency,
            idle_execution_interval_secs,
            execution_jitter_secs,
        })
    }
}

pub(crate) struct SubscriptionWorkerHandle {
    cancel: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl SubscriptionWorkerHandle {
    pub(crate) fn cancel(&self) {
        let _ = self.cancel.send(true);
    }

    pub(crate) async fn shutdown(mut self) {
        self.cancel();
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for SubscriptionWorkerHandle {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

pub(crate) fn spawn_subscription_worker(
    config: ConfigManager,
    poll_service: SubscriptionPollService,
    execution_service: SubscriptionExecutionService,
    options: SubscriptionWorkerOptions,
) -> SubscriptionWorkerHandle {
    let (cancel, mut cancelled) = watch::channel(false);
    let task = tokio::spawn(async move {
        let mut next_poll_at = None;
        loop {
            if *cancelled.borrow() {
                break;
            }
            let snapshot = config.snapshot().await;
            let watcher = snapshot.value.subscription_watcher.clone();
            if !watcher.enabled {
                next_poll_at = None;
                if wait_or_cancel(&mut cancelled, 60).await {
                    break;
                }
                continue;
            }

            let poll_policy = match subscription_poll_policy(&snapshot.value) {
                Ok(policy) => policy,
                Err(error) => {
                    tracing::warn!(error = %error, "subscription Poll configuration is invalid");
                    if wait_or_cancel(&mut cancelled, watcher.system_retry_interval_secs.max(1))
                        .await
                    {
                        break;
                    }
                    continue;
                }
            };

            if next_poll_at.is_none() {
                match poll_service.next_poll_at(&poll_policy).await {
                    Ok(persisted) => next_poll_at = persisted,
                    Err(error) => {
                        tracing::warn!(error = %error, "load persisted subscription Poll schedule failed");
                        if wait_or_cancel(&mut cancelled, watcher.system_retry_interval_secs.max(1))
                            .await
                        {
                            break;
                        }
                        continue;
                    }
                }
            }

            let now = system_now();
            if next_poll_at.is_none_or(|due| due <= now) {
                next_poll_at = Some(match poll_service.poll(&poll_policy).await {
                    Ok(outcome) => {
                        tracing::info!(
                            snapshot_id = %outcome.snapshot_id,
                            fetched_items = outcome.fetched_items,
                            snapshot_complete = outcome.snapshot_complete,
                            dry_run = watcher.dry_run,
                            next_poll_at = outcome.next_poll_at,
                            "subscription Poll worker completed"
                        );
                        outcome.next_poll_at
                    }
                    Err(error) => {
                        tracing::warn!(
                            error = %error,
                            dry_run = watcher.dry_run,
                            "subscription Poll worker failed"
                        );
                        error.next_poll_at.unwrap_or_else(|| {
                            system_now().saturating_add(watcher.system_retry_interval_secs.max(1))
                        })
                    }
                });
            }

            let execution = if watcher.dry_run {
                None
            } else {
                match execution_service
                    .run_due_batch(
                        &execution_policy(&snapshot.value),
                        options.execution_batch_size,
                        options.execution_concurrency,
                    )
                    .await
                {
                    Ok(outcome) => {
                        log_execution_batch(&outcome);
                        Some(outcome)
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "subscription execution batch failed");
                        None
                    }
                }
            };

            let now = system_now();
            let until_poll = next_poll_at
                .map(|due| due.saturating_sub(now).max(1))
                .unwrap_or(watcher.poll_interval_secs.max(1));
            let queue_busy = execution
                .as_ref()
                .is_some_and(|outcome| outcome.claimed == options.execution_batch_size);
            let execution_delay = if queue_busy {
                1
            } else {
                options.idle_execution_interval_secs
            };
            let jitter = bounded_jitter(options.execution_jitter_secs);
            let delay = until_poll
                .min(execution_delay.saturating_add(jitter))
                .max(1);
            if wait_or_cancel(&mut cancelled, delay).await {
                break;
            }
        }
    });
    SubscriptionWorkerHandle {
        cancel,
        task: Some(task),
    }
}

fn log_execution_batch(outcome: &ExecutionBatchOutcome) {
    if outcome.claimed == 0 && outcome.terminal_errors == 0 {
        return;
    }
    tracing::info!(
        claimed = outcome.claimed,
        finished = outcome.finished,
        failed = outcome.failed,
        terminal_errors = outcome.terminal_errors,
        exhausted = outcome.exhausted,
        "subscription execution batch completed"
    );
}

async fn wait_or_cancel(cancelled: &mut watch::Receiver<bool>, seconds: u64) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(seconds.max(1))) => false,
        changed = cancelled.changed() => changed.is_err() || *cancelled.borrow(),
    }
}

fn bounded_jitter(maximum: u64) -> u64 {
    if maximum == 0 {
        return 0;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.subsec_nanos() as u64) % maximum.saturating_add(1))
        .unwrap_or_default()
}

fn system_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub(crate) fn subscription_poll_policy(
    config: &FileConfig,
) -> Result<SubscriptionPollPolicy, SubscriptionPollError> {
    let redactor = SubscriptionDiagnosticRedactor::from_config(config);
    let account_key = douban::auth_cache_key_fragment(&config.douban_cookie).map_err(|error| {
        SubscriptionPollError::new(
            SubscriptionPollErrorKind::Validation,
            redactor.redact_or(
                &error.to_string(),
                "Douban authentication is not configured",
            ),
        )
    })?;
    Ok(SubscriptionPollPolicy {
        account_key,
        cookie_header: config.douban_cookie.clone(),
        library_limit: config.subscription_watcher.library_limit,
        poll_interval_secs: config.subscription_watcher.poll_interval_secs,
        new_record_policy: new_record_policy(&config.subscription_watcher)?,
        retry_policy: retry_policy(&config.subscription_watcher)?,
        redactor,
    })
}

fn execution_policy(config: &FileConfig) -> ExecutionPolicy {
    let watcher = &config.subscription_watcher;
    ExecutionPolicy {
        enabled: watcher.enabled,
        dry_run: watcher.dry_run,
        account_key: douban::auth_cache_key_fragment(&config.douban_cookie).unwrap_or_default(),
        effects: ExecutionEffectPolicy {
            douban_cookie: config.douban_cookie.clone(),
            tmdb_api_key: config.tmdb_api_key.clone(),
            mteam_api_key: config.mteam_api_key.clone(),
            qb_servers: config
                .qb_servers
                .iter()
                .map(|server| ExecutionQbServer {
                    id: server.id.clone(),
                    name: server.name.clone(),
                    base_url: server.base_url.clone(),
                    username: server.username.clone(),
                    password: server.password.clone(),
                    insecure_tls: server.insecure_tls,
                })
                .collect(),
            categories: config
                .subscription_categories
                .iter()
                .map(|category| ExecutionCategory {
                    name: category.name.clone(),
                    wanted_tag: category.wanted_tag.clone(),
                    qb_server_id: category.qb_server_id.clone(),
                    qb_category: category.qb_category.clone(),
                    qb_save_dir_name: category.qb_save_dir_name.clone(),
                    download_dir: category.download_dir.clone(),
                    link_target_dir: category.link_target_dir.clone(),
                })
                .collect(),
            torrent_match_rules: config
                .torrent_match_rules
                .iter()
                .map(|rule| ExecutionTorrentMatchRule {
                    name: rule.name.clone(),
                    priority: rule.priority,
                    mode: match rule.mode {
                        TorrentRuleMatchMode::All => ExecutionTorrentRuleMatchMode::All,
                        TorrentRuleMatchMode::Any => ExecutionTorrentRuleMatchMode::Any,
                    },
                    title_keywords: rule.title_keywords.clone(),
                    resolution_keywords: rule.resolution_keywords.clone(),
                    source_keywords: rule.source_keywords.clone(),
                })
                .collect(),
            search_interval_secs: watcher.search_interval_secs,
            progress_interval_secs: watcher.progress_interval_secs,
            link_retry_interval_secs: watcher.link_retry_interval_secs,
            system_retry_interval_secs: watcher.system_retry_interval_secs,
        },
    }
}

fn new_record_policy(
    watcher: &SubscriptionWatcherConfig,
) -> Result<NewRecordPolicy, SubscriptionPollError> {
    NewRecordPolicy::try_new(watcher.max_retries, watcher.bootstrap_existing_as_skipped)
        .map_err(SubscriptionPollError::repository)
}

fn retry_policy(
    watcher: &SubscriptionWatcherConfig,
) -> Result<PollRetryPolicy, SubscriptionPollError> {
    let maximum = watcher.poll_interval_secs.max(1);
    let initial = watcher.system_retry_interval_secs.clamp(1, maximum);
    PollRetryPolicy::try_new(initial, maximum).map_err(SubscriptionPollError::repository)
}

enum SnapshotTerminal {
    Complete,
    Incomplete(IncompleteSnapshotObservation),
}

fn validated_snapshot_terminal(
    snapshot: WantedSnapshotMetadata,
) -> Result<SnapshotTerminal, String> {
    match snapshot.completeness {
        WantedSnapshotCompleteness::Complete => {
            if !snapshot.is_complete() {
                return Err(
                    "a complete source snapshot must prove that its end was observed".to_string(),
                );
            }
            if snapshot.truncated_by_limit {
                return Err(
                    "a complete source snapshot cannot also be truncated by the item limit"
                        .to_string(),
                );
            }
            Ok(SnapshotTerminal::Complete)
        }
        WantedSnapshotCompleteness::Partial => {
            incomplete_observation(snapshot).map(SnapshotTerminal::Incomplete)
        }
    }
}

fn incomplete_observation(
    snapshot: WantedSnapshotMetadata,
) -> Result<IncompleteSnapshotObservation, String> {
    if snapshot.end_observed {
        return Err(
            "an incomplete source snapshot cannot claim that its end was observed".to_string(),
        );
    }
    let reason = if snapshot.truncated_by_limit {
        IncompleteSnapshotReason::ItemLimitReached
    } else if snapshot.fetched_pages >= DOUBAN_LIBRARY_MAX_PAGES {
        IncompleteSnapshotReason::MaximumPageCountReached
    } else {
        IncompleteSnapshotReason::EndNotObserved
    };
    let fetched_pages = u32::try_from(snapshot.fetched_pages).map_err(|_| {
        "incomplete source snapshot page count exceeds the persisted range".to_string()
    })?;
    IncompleteSnapshotObservation::try_new(
        fetched_pages,
        snapshot.truncated_by_limit,
        snapshot.end_observed,
        reason,
    )
    .map_err(|error| error.to_string())
}

fn valid_poll_failure_message(message: String, fallback: &str) -> String {
    let message = message.replace('\0', "");
    if message.trim().is_empty() {
        fallback.to_string()
    } else {
        message
    }
}

fn snapshot_records(items: &[WantedItem]) -> Result<Vec<SnapshotRecord>, RepositoryError> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| snapshot_record(item, index))
        .collect()
}

fn snapshot_record(
    item: &WantedItem,
    return_order: usize,
) -> Result<SnapshotRecord, RepositoryError> {
    let tags = normalized_tags(&item.tags);
    let media_kind = SubscriptionMediaKind::from_tags(&tags);
    let douban_date = non_empty(&item.date);
    let source = WantedSourcePayload {
        title: item.title.trim().to_string(),
        release_year: release_year_from_item(item),
        poster_url: item.poster_url.trim().to_string(),
        cover_url: item.cover_url.trim().to_string(),
        category_text: tags.first().cloned(),
        tags,
        douban_sort_time: douban_date.as_deref().and_then(douban_date_sort_key),
        douban_date,
        douban_return_order: Some(return_order.min(u32::MAX as usize) as u32),
        ..WantedSourcePayload::default()
    };
    SnapshotRecord::try_new(&item.subject_id, media_kind, true, None, source)
}

fn normalized_tags(tags: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for tag in tags {
        let value = tag.trim();
        if !value.is_empty() && !out.iter().any(|existing| existing == value) {
            out.push(value.to_string());
        }
    }
    out
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn douban_date_sort_key(raw: &str) -> Option<u64> {
    let digits = raw.chars().filter(char::is_ascii_digit).collect::<String>();
    if digits.len() < 4 {
        return None;
    }
    digits[..digits.len().min(14)].parse().ok()
}

fn release_year_from_item(item: &WantedItem) -> Option<u16> {
    [&item.abstract_text, &item.abstract_2, &item.date]
        .into_iter()
        .find_map(|value| release_year_from_text(value))
}

fn release_year_from_text(text: &str) -> Option<u16> {
    text.as_bytes().windows(4).find_map(|digits| {
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        let year = std::str::from_utf8(digits).ok()?.parse::<u16>().ok()?;
        (1888..=2200).contains(&year).then_some(year)
    })
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use rusqlite::Connection;

    use super::*;
    use crate::storage::SqliteSubscriptionRepository;
    use crate::subscription::execution::{
        ExecutionEffectFuture, ExecutionEffectResult, SubscriptionExecutionEffects,
        SubscriptionExecutionService,
    };
    use crate::subscription::ports::SubscriptionReadRepository;
    use crate::subscription::repository::{
        ClaimedSubscription, ExecutionPayloadDelta, FinishExecutionDisposition,
        ListSubscriptionsCommand, SubscriptionListFilter,
    };

    struct NoopExecutionEffects;

    impl SubscriptionExecutionEffects for NoopExecutionEffects {
        fn execute(
            &self,
            _claimed: ClaimedSubscription,
            _policy: ExecutionEffectPolicy,
        ) -> ExecutionEffectFuture {
            Box::pin(async {
                ExecutionEffectResult::Finished {
                    disposition: FinishExecutionDisposition::MetaReady,
                    payload_delta: ExecutionPayloadDelta::Meta,
                }
            })
        }
    }

    struct FakeSource {
        calls: AtomicUsize,
        results: Mutex<VecDeque<Result<WantedSnapshot, WantedSourceError>>>,
    }

    impl FakeSource {
        fn new(
            results: impl IntoIterator<Item = Result<WantedSnapshot, WantedSourceError>>,
        ) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                results: Mutex::new(results.into_iter().collect()),
            }
        }
    }

    impl WantedSource for FakeSource {
        fn fetch_wanted(&self, _cookie_header: String, _limit: usize) -> WantedSourceFuture {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let result = self
                .results
                .lock()
                .expect("lock fake source")
                .pop_front()
                .expect("one configured fake source result");
            Box::pin(async move { result })
        }
    }

    struct SequenceClock(Mutex<VecDeque<u64>>);

    impl PollClock for SequenceClock {
        fn now(&self) -> u64 {
            self.0
                .lock()
                .expect("lock clock")
                .pop_front()
                .expect("one configured clock value")
        }
    }

    fn temp_database(label: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-worker-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root.join("subscriptions.sqlite")
    }

    fn config(enabled: bool, dry_run: bool) -> FileConfig {
        let mut config = FileConfig {
            douban_cookie: "dbcl2=account-1:secret; ck=test".to_string(),
            ..FileConfig::default()
        };
        config.subscription_watcher.enabled = enabled;
        config.subscription_watcher.dry_run = dry_run;
        config.subscription_watcher.bootstrap_existing_as_skipped = false;
        config
    }

    fn movie(subject_id: &str) -> WantedItem {
        WantedItem {
            subject_id: subject_id.to_string(),
            title: "Fixture Movie".to_string(),
            abstract_text: "2026 / 中国大陆".to_string(),
            abstract_2: String::new(),
            cover_url: "https://images.test/cover.jpg".to_string(),
            poster_url: "https://images.test/poster.jpg".to_string(),
            date: "2026-07-11".to_string(),
            tags: vec!["电影".to_string()],
        }
    }

    fn complete_list(items: Vec<WantedItem>) -> WantedSnapshot {
        WantedSnapshot {
            items,
            metadata: WantedSnapshotMetadata::complete(1),
        }
    }

    fn list_with_snapshot(
        items: Vec<WantedItem>,
        snapshot: WantedSnapshotMetadata,
    ) -> WantedSnapshot {
        WantedSnapshot {
            items,
            metadata: snapshot,
        }
    }

    fn poll_state(path: &std::path::Path) -> (Option<i64>, Option<String>, i64, i64) {
        Connection::open(path)
            .unwrap()
            .query_row(
                r#"SELECT open_poll_generation, open_snapshot_id,
                          poll_failure_count, next_poll_at
                     FROM subscription_meta
                    WHERE account_key = ?1"#,
                ["account-1"],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap()
    }

    fn assert_closed_failure(
        path: &std::path::Path,
        error: &SubscriptionPollError,
        expected_failure_count: i64,
    ) {
        let (open_generation, open_snapshot_id, failure_count, next_poll_at) = poll_state(path);
        assert_eq!(open_generation, None);
        assert_eq!(open_snapshot_id, None);
        assert_eq!(failure_count, expected_failure_count);
        assert_eq!(error.next_poll_at, Some(next_poll_at as u64));
    }

    #[tokio::test]
    async fn poll_persists_before_fetch_and_makes_the_result_readable() {
        let path = temp_database("poll-write");
        let repository = Arc::new(
            SqliteSubscriptionRepository::try_create_fresh(&path, 2, Duration::from_secs(1))
                .unwrap(),
        );
        let source = Arc::new(FakeSource::new([Ok(complete_list(vec![movie(
            "subject-1",
        )]))]));
        let service = SubscriptionPollService::with_dependencies(
            repository.clone(),
            source,
            Arc::new(SequenceClock(Mutex::new(VecDeque::from([100, 101])))),
        );

        let config = config(false, true);
        let policy = subscription_poll_policy(&config).unwrap();
        let outcome = service.poll(&policy).await.unwrap();

        assert!(outcome.snapshot_complete);
        assert_eq!(outcome.inserted, 1);
        assert_eq!(
            service.next_poll_at(&policy).await.unwrap(),
            Some(outcome.next_poll_at)
        );
        let page = repository
            .list_summaries(
                ListSubscriptionsCommand::try_new(
                    "account-1",
                    SubscriptionListFilter::default(),
                    None,
                    10,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].head.key.subject_id, "subject-1");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn manual_poll_and_worker_tick_share_the_exact_service_graph() {
        let path = temp_database("manual-worker-parity");
        let repository = Arc::new(
            SqliteSubscriptionRepository::try_create_fresh(&path, 2, Duration::from_secs(1))
                .unwrap(),
        );
        let source = Arc::new(FakeSource::new([
            Ok(complete_list(vec![movie("subject-1")])),
            Ok(complete_list(vec![movie("subject-1"), movie("subject-2")])),
        ]));
        let service = SubscriptionPollService::with_dependencies(
            repository.clone(),
            source.clone(),
            Arc::new(SequenceClock(Mutex::new(VecDeque::from([
                100, 101, 200, 201,
            ])))),
        );
        let config = config(true, true);
        let manager = ConfigManager::new(path.with_extension("toml"), config.clone());

        let manual = service
            .poll(&subscription_poll_policy(&config).unwrap())
            .await
            .unwrap();
        let worker = run_worker_tick(&manager, &service).await.unwrap();

        assert_eq!(manual.inserted, 1);
        let SubscriptionWorkerTick::Polled(worker) = worker else {
            panic!("enabled worker must use the shared Poll service");
        };
        assert_eq!(worker.inserted, 1);
        assert_eq!(source.calls.load(Ordering::Relaxed), 2);
        let page = repository
            .list_summaries(
                ListSubscriptionsCommand::try_new(
                    "account-1",
                    SubscriptionListFilter::default(),
                    None,
                    10,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(page.items.len(), 2);

        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let _ = std::fs::remove_file(path.with_extension("toml"));
    }

    #[tokio::test]
    async fn invalid_config_policy_is_rejected_before_opening_a_poll() {
        let path = temp_database("invalid-policy");
        let repository = Arc::new(
            SqliteSubscriptionRepository::try_create_fresh(&path, 1, Duration::from_secs(1))
                .unwrap(),
        );
        let source = Arc::new(FakeSource::new([]));
        let _service = SubscriptionPollService::with_dependencies(
            repository,
            source.clone(),
            Arc::new(SequenceClock(Mutex::new(VecDeque::new()))),
        );
        let mut invalid_config = config(false, true);
        invalid_config.subscription_watcher.max_retries = 0;

        let error = subscription_poll_policy(&invalid_config).unwrap_err();

        assert_eq!(error.kind(), SubscriptionPollErrorKind::Internal);
        assert_eq!(source.calls.load(Ordering::Relaxed), 0);
        let account_count: i64 = Connection::open(&path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM subscription_meta", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(account_count, 0);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn invalid_snapshots_consume_the_exact_token_and_allow_an_immediate_retry() {
        let path = temp_database("invalid-snapshot-terminal");
        let repository = Arc::new(
            SqliteSubscriptionRepository::try_create_fresh(&path, 2, Duration::from_secs(1))
                .unwrap(),
        );
        let duplicate_complete = complete_list(vec![movie("duplicate"), movie("duplicate")]);
        let duplicate_incomplete = list_with_snapshot(
            vec![movie("partial-duplicate"), movie("partial-duplicate")],
            WantedSnapshotMetadata {
                completeness: WantedSnapshotCompleteness::Partial,
                fetched_pages: 1,
                truncated_by_limit: true,
                end_observed: false,
            },
        );
        let source = Arc::new(FakeSource::new([
            Ok(list_with_snapshot(
                vec![movie("inconsistent")],
                WantedSnapshotMetadata {
                    completeness: WantedSnapshotCompleteness::Complete,
                    fetched_pages: 1,
                    truncated_by_limit: false,
                    end_observed: false,
                },
            )),
            Ok(duplicate_complete),
            Ok(duplicate_incomplete),
            Ok(complete_list(vec![movie("recovered")])),
        ]));
        let service = SubscriptionPollService::with_dependencies(
            repository.clone(),
            source.clone(),
            Arc::new(SequenceClock(Mutex::new(VecDeque::from([
                100, 101, 102, 103, 104, 105, 106, 107,
            ])))),
        );
        let config = config(false, true);
        let policy = subscription_poll_policy(&config).unwrap();

        let inconsistent = service.poll(&policy).await.unwrap_err();
        assert_eq!(inconsistent.kind(), SubscriptionPollErrorKind::Internal);
        assert_closed_failure(&path, &inconsistent, 1);

        let duplicate_complete = service.poll(&policy).await.unwrap_err();
        assert_eq!(
            duplicate_complete.kind(),
            SubscriptionPollErrorKind::Internal
        );
        assert_closed_failure(&path, &duplicate_complete, 2);

        let duplicate_incomplete = service.poll(&policy).await.unwrap_err();
        assert_eq!(
            duplicate_incomplete.kind(),
            SubscriptionPollErrorKind::Internal
        );
        assert_closed_failure(&path, &duplicate_incomplete, 3);

        let recovered = service.poll(&policy).await.unwrap();
        assert!(recovered.snapshot_complete);
        assert_eq!(recovered.inserted, 1);
        assert_eq!(source.calls.load(Ordering::Relaxed), 4);
        assert_eq!(
            poll_state(&path),
            (None, None, 0, recovered.next_poll_at as i64)
        );
        let page = repository
            .list_summaries(
                ListSubscriptionsCommand::try_new(
                    "account-1",
                    SubscriptionListFilter::default(),
                    None,
                    10,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].head.key.subject_id, "recovered");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[tokio::test]
    async fn disabled_worker_tick_never_calls_the_upstream_source() {
        let path = temp_database("disabled");
        let repository = Arc::new(
            SqliteSubscriptionRepository::try_create_fresh(&path, 1, Duration::from_secs(1))
                .unwrap(),
        );
        let source = Arc::new(FakeSource::new([]));
        let service = SubscriptionPollService::with_dependencies(
            repository,
            source.clone(),
            Arc::new(SequenceClock(Mutex::new(VecDeque::new()))),
        );
        let root = path.parent().unwrap();
        let manager = ConfigManager::new(root.join("config.toml"), config(false, true));

        assert_eq!(
            run_worker_tick(&manager, &service).await.unwrap(),
            SubscriptionWorkerTick::Disabled
        );
        assert_eq!(source.calls.load(Ordering::Relaxed), 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn worker_options_reject_unbounded_or_zero_execution_settings() {
        assert!(SubscriptionWorkerOptions::try_new(0, 1, 1, 0).is_err());
        assert!(SubscriptionWorkerOptions::try_new(2, 0, 1, 0).is_err());
        assert!(SubscriptionWorkerOptions::try_new(2, 3, 1, 0).is_err());
        assert!(SubscriptionWorkerOptions::try_new(2, 1, 0, 0).is_err());
        assert_eq!(
            SubscriptionWorkerOptions::try_new(4, 2, 15, 5).unwrap(),
            SubscriptionWorkerOptions {
                execution_batch_size: 4,
                execution_concurrency: 2,
                idle_execution_interval_secs: 15,
                execution_jitter_secs: 5,
            }
        );
    }

    #[tokio::test]
    async fn worker_handle_cancels_a_disabled_wait_without_poll_or_claim() {
        let path = temp_database("cancel");
        let repository = Arc::new(
            SqliteSubscriptionRepository::try_create_fresh(&path, 2, Duration::from_secs(1))
                .unwrap(),
        );
        let source = Arc::new(FakeSource::new([]));
        let poll = SubscriptionPollService::with_dependencies(
            repository.clone(),
            source.clone(),
            Arc::new(SequenceClock(Mutex::new(VecDeque::new()))),
        );
        let execution =
            SubscriptionExecutionService::try_new(repository, Arc::new(NoopExecutionEffects), 60)
                .unwrap();
        let root = path.parent().unwrap();
        let handle = spawn_subscription_worker(
            ConfigManager::new(root.join("config.toml"), config(false, false)),
            poll,
            execution,
            SubscriptionWorkerOptions::try_new(2, 1, 15, 0).unwrap(),
        );

        tokio::time::timeout(Duration::from_secs(1), handle.shutdown())
            .await
            .expect("worker shutdown must not wait for the disabled sleep");

        assert_eq!(source.calls.load(Ordering::Relaxed), 0);
        let _ = std::fs::remove_dir_all(root);
    }
}
