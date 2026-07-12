use std::fmt::Write as _;

use ring::digest::{Context, SHA256};
use rusqlite::{params, Connection, OptionalExtension, Row, Transaction, TransactionBehavior};
use serde_json::{json, Value};

use super::execution_audit::{append_execution_audit, ExecutionAuditEntry};
use super::{
    checked_bool, checked_optional_u64, checked_required_text, checked_u32, checked_u64,
    command_integer, corrupt, map_read_error, map_write_error, parse_attention_tags,
    parse_execution_state, parse_lifecycle_state, parse_media_kind, persisted_validation,
    SqliteSubscriptionRepository,
};
use crate::subscription::ports::{RepoFuture, SubscriptionPollRepository};
use crate::subscription::repository::payload::ObservationPayload;
use crate::subscription::repository::{
    ApplyCompleteSnapshotCommand, ApplyCompleteSnapshotResult, BeginPollCommand, BeginPollResult,
    BlockedReason, ExecutionAttemptId, ExecutionOperation, NewRecordPolicy, PollAttemptToken,
    PollGeneration, PollSchedule, RecordIncompleteSnapshotCommand, RecordIncompleteSnapshotResult,
    RecordPollFailureCommand, RecordPollFailureResult, RepositoryError, RepositoryResult, Revision,
    SnapshotId, SnapshotRecord, SubscriptionDetail, SubscriptionHead, SubscriptionPayload,
    SubscriptionProjection, SubscriptionSummary,
};
use crate::subscription::{
    SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
    SubscriptionMediaKind, INACTIVE_SUBSCRIPTION_REASON, TV_NOT_SUPPORTED_REASON,
};

const STATE_VERSION: i64 = 1;
const SNAPSHOT_ID_PREFIX: &str = "wanted-poll-v1-";
const SNAPSHOT_ID_DOMAIN: &[u8] = b"tmdb-mteam-hub/wanted-poll-snapshot/v1\0";
const INITIAL_BOOTSTRAP_SKIP_REASON: &str = "initial_bootstrap_existing_wish";
const SUPERSEDE_ATTEMPT_ACTION: &str = "supersede_attempt";
const SUPERSEDE_AUDIT_SCHEMA: &str = "subscription_attempt_superseded.v1";

const READ_EXISTING_SQL: &str = r#"
SELECT revision,
       active,
       inactive_at,
       last_seen_snapshot_id,
       media_kind,
       schedulable,
       blocked_reason,
       lifecycle_state,
       execution_state,
       next_attempt_at,
       retry_count,
       max_retries,
       retry_blocked,
       force_eligible_once,
       claimed_operation,
       attempt_id,
       lease_until,
       title,
       release_year,
       poster_url,
       category_text,
       douban_sort_time,
       attention_tags_json,
       updated_at,
       record_json
  FROM wanted_subscription_records
 WHERE account_key = ?1 AND subject_id = ?2
"#;

const READ_MISSING_EXISTING_SQL: &str = r#"
SELECT subject_id,
       revision,
       active,
       inactive_at,
       last_seen_snapshot_id,
       media_kind,
       schedulable,
       blocked_reason,
       lifecycle_state,
       execution_state,
       next_attempt_at,
       retry_count,
       max_retries,
       retry_blocked,
       force_eligible_once,
       claimed_operation,
       attempt_id,
       lease_until,
       title,
       release_year,
       poster_url,
       category_text,
       douban_sort_time,
       attention_tags_json,
       updated_at,
       record_json
  FROM wanted_subscription_records
 WHERE account_key = ?1
   AND active = 1
   AND (last_seen_snapshot_id IS NULL OR last_seen_snapshot_id <> ?2)
 ORDER BY subject_id
"#;

const UPDATE_SEEN_SQL: &str = r#"
UPDATE wanted_subscription_records
   SET revision = ?4,
       active = 1,
       inactive_at = NULL,
       last_seen_snapshot_id = ?5,
       media_kind = ?6,
       schedulable = ?7,
       blocked_reason = ?8,
       execution_state = ?9,
       next_attempt_at = ?10,
       force_eligible_once = ?11,
       claimed_operation = ?12,
       attempt_id = ?13,
       lease_until = ?14,
       title = ?15,
       release_year = ?16,
       poster_url = ?17,
       category_text = ?18,
       douban_sort_time = ?19,
       updated_at = ?20,
       record_json = ?21
 WHERE account_key = ?1
   AND subject_id = ?2
   AND revision = ?3
"#;

const INSERT_SEEN_SQL: &str = r#"
INSERT INTO wanted_subscription_records (
    account_key, subject_id, revision, active, inactive_at, last_seen_snapshot_id,
    media_kind, schedulable, blocked_reason, lifecycle_state, execution_state,
    next_attempt_at, retry_count, max_retries, retry_blocked, force_eligible_once,
    claimed_operation, attempt_id, lease_until, title, release_year, poster_url,
    category_text, douban_sort_time, attention_tags_json, updated_at, record_json
) VALUES (
    ?1, ?2, 1, 1, NULL, ?3, ?4, ?5, ?6, 'queued', 'idle',
    ?7, 0, ?8, 0, 0, NULL, NULL, NULL, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
)
"#;

const DEACTIVATE_MISSING_SQL: &str = r#"
UPDATE wanted_subscription_records
   SET revision = revision + 1,
       active = 0,
       inactive_at = ?3,
       schedulable = 0,
       blocked_reason = CASE
           WHEN media_kind = 'tv' THEN ?4
           ELSE ?5
       END,
       execution_state = 'idle',
       next_attempt_at = NULL,
       force_eligible_once = 0,
       claimed_operation = NULL,
       attempt_id = NULL,
       lease_until = NULL,
       updated_at = MAX(updated_at, ?3)
 WHERE account_key = ?1
   AND active = 1
   AND (last_seen_snapshot_id IS NULL OR last_seen_snapshot_id <> ?2)
"#;

impl SubscriptionPollRepository for SqliteSubscriptionRepository {
    fn load_poll_schedule(&self, account_key: String) -> RepoFuture<PollSchedule> {
        self.executor
            .run(move |connection| load_poll_schedule(connection, account_key))
    }

    fn begin_poll(&self, command: BeginPollCommand) -> RepoFuture<BeginPollResult> {
        self.executor
            .run(move |connection| begin_poll(connection, command))
    }

    fn apply_complete_snapshot(
        &self,
        command: ApplyCompleteSnapshotCommand,
    ) -> RepoFuture<ApplyCompleteSnapshotResult> {
        self.executor
            .run(move |connection| apply_complete_snapshot(connection, command))
    }

    fn record_incomplete_snapshot(
        &self,
        command: RecordIncompleteSnapshotCommand,
    ) -> RepoFuture<RecordIncompleteSnapshotResult> {
        self.executor
            .run(move |connection| record_incomplete_snapshot(connection, command))
    }

    fn record_poll_failure(
        &self,
        command: RecordPollFailureCommand,
    ) -> RepoFuture<RecordPollFailureResult> {
        self.executor
            .run(move |connection| record_poll_failure(connection, command))
    }
}

fn load_poll_schedule(
    connection: &mut Connection,
    account_key: String,
) -> RepositoryResult<PollSchedule> {
    let next_poll_at = connection
        .query_row(
            "SELECT next_poll_at FROM subscription_meta WHERE account_key = ?1",
            [account_key],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(|error| map_read_error("read persisted Poll schedule", error))?
        .flatten()
        .map(|value| checked_u64("next_poll_at", value))
        .transpose()?;
    Ok(PollSchedule::new(next_poll_at))
}

fn begin_poll(
    connection: &mut Connection,
    command: BeginPollCommand,
) -> RepositoryResult<BeginPollResult> {
    let attempted_at = command_integer("attempted_at", command.attempted_at)?;
    let transaction = begin_immediate(connection, "begin poll attempt")?;
    let current = transaction
        .query_row(
            r#"SELECT state_version, created_at, updated_at, last_poll_attempt_at,
                      last_poll_success_at, poll_generation
                 FROM subscription_meta
                WHERE account_key = ?1"#,
            [command.account_key.as_str()],
            |row| {
                Ok(BeginMeta {
                    state_version: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    last_poll_attempt_at: row.get(3)?,
                    last_poll_success_at: row.get(4)?,
                    poll_generation: row.get(5)?,
                })
            },
        )
        .optional()
        .map_err(|error| map_read_error("read account metadata before poll begin", error))?;

    let generation = match current {
        Some(current) => {
            current.validate_command_time(command.attempted_at)?;
            let current_generation = checked_u64("poll_generation", current.poll_generation)?;
            if current_generation >= i64::MAX as u64 {
                return Err(RepositoryError::CorruptData {
                    message: "persisted poll generation is exhausted at SQLite INTEGER max"
                        .to_string(),
                });
            }
            let generation = current_generation + 1;
            let generation_sql = i64::try_from(generation).expect("bounded by SQLite INTEGER max");
            let snapshot_id = stable_snapshot_id(&command.account_key, generation)?;
            let changed = transaction
                .execute(
                    r#"UPDATE subscription_meta
                          SET updated_at = MAX(updated_at, ?2),
                              last_poll_attempt_at = ?2,
                              poll_generation = ?3,
                              open_poll_generation = ?3,
                              open_snapshot_id = ?4
                        WHERE account_key = ?1 AND poll_generation = ?5"#,
                    params![
                        command.account_key.as_str(),
                        attempted_at,
                        generation_sql,
                        snapshot_id.as_str(),
                        current.poll_generation,
                    ],
                )
                .map_err(|error| map_write_error("persist existing account poll begin", error))?;
            require_single_write(changed, "persist existing account poll begin")?;
            (generation, snapshot_id)
        }
        None => {
            let generation = 1_u64;
            let snapshot_id = stable_snapshot_id(&command.account_key, generation)?;
            let changed = transaction
                .execute(
                    r#"INSERT INTO subscription_meta (
                           account_key, state_version, bootstrap_completed, created_at, updated_at,
                           last_poll_attempt_at, last_poll_success_at, poll_failure_count,
                           next_poll_at, last_poll_error, poll_generation, open_poll_generation,
                           open_snapshot_id, last_incomplete_at, last_incomplete_reason,
                           last_incomplete_fetched_pages, last_incomplete_truncated,
                           last_incomplete_end_observed, last_complete_snapshot_id
                       ) VALUES (
                           ?1, ?2, 0, ?3, ?3, ?3, NULL, 0, ?3, NULL, 1, 1, ?4,
                           NULL, NULL, NULL, NULL, NULL, NULL
                       )"#,
                    params![
                        command.account_key.as_str(),
                        STATE_VERSION,
                        attempted_at,
                        snapshot_id.as_str(),
                    ],
                )
                .map_err(|error| map_write_error("persist new account poll begin", error))?;
            require_single_write(changed, "persist new account poll begin")?;
            (generation, snapshot_id)
        }
    };

    let token = PollAttemptToken::new(PollGeneration::try_new(generation.0)?, generation.1);
    transaction
        .commit()
        .map_err(|error| map_write_error("commit poll begin", error))?;
    Ok(BeginPollResult {
        token,
        attempted_at: command.attempted_at,
    })
}

fn record_poll_failure(
    connection: &mut Connection,
    command: RecordPollFailureCommand,
) -> RepositoryResult<RecordPollFailureResult> {
    let failed_at = command_integer("failed_at", command.failed_at)?;
    let token_sql = SqlToken::try_from(&command.token)?;
    let transaction = begin_immediate(connection, "record poll failure")?;
    let meta = require_current_poll(&transaction, &command.account_key, &command.token)?;
    meta.validate_terminal_time("failed_at", command.failed_at)?;
    let failure_count = meta.failure_count.saturating_add(1);
    let next_poll_at = command
        .retry_policy
        .next_poll_at(command.failed_at, failure_count);
    let next_poll_at_sql = command_integer("next_poll_at", next_poll_at)?;
    let changed = transaction
        .execute(
            r#"UPDATE subscription_meta
                  SET updated_at = MAX(updated_at, ?4),
                      poll_failure_count = ?5,
                      next_poll_at = ?6,
                      last_poll_error = ?7,
                      open_poll_generation = NULL,
                      open_snapshot_id = NULL
                WHERE account_key = ?1
                  AND open_poll_generation = ?2
                  AND open_snapshot_id = ?3"#,
            params![
                command.account_key.as_str(),
                token_sql.generation,
                token_sql.snapshot_id,
                failed_at,
                i64::from(failure_count),
                next_poll_at_sql,
                command.message.as_str(),
            ],
        )
        .map_err(|error| map_write_error("consume failed poll token", error))?;
    require_consumed(&transaction, changed, &command.account_key, &command.token)?;
    transaction
        .commit()
        .map_err(|error| map_write_error("commit poll failure", error))?;
    Ok(RecordPollFailureResult {
        token: command.token,
        failure_count,
        next_poll_at,
    })
}

pub(super) fn record_incomplete_snapshot(
    connection: &mut Connection,
    command: RecordIncompleteSnapshotCommand,
) -> RepositoryResult<RecordIncompleteSnapshotResult> {
    let incomplete_at = command_integer("incomplete_at", command.incomplete_at)?;
    let token_sql = SqlToken::try_from(&command.token)?;
    let transaction = begin_immediate(connection, "record incomplete poll snapshot")?;
    let meta = require_current_poll(&transaction, &command.account_key, &command.token)?;
    meta.validate_terminal_time("incomplete_at", command.incomplete_at)?;
    let effects = apply_seen_records(
        &transaction,
        &command.account_key,
        PollFenceContext {
            token: &command.token,
            snapshot_kind: PollSnapshotKind::Incomplete,
            fenced_at: command.incomplete_at,
        },
        command.incomplete_at,
        incomplete_at,
        meta.bootstrap_completed,
        command.new_record_policy,
        &command.records,
    )?;
    let failure_count = meta.failure_count.saturating_add(1);
    let next_poll_at = command
        .retry_policy
        .next_poll_at(command.incomplete_at, failure_count);
    let next_poll_at_sql = command_integer("next_poll_at", next_poll_at)?;
    let reason = incomplete_reason_label(command.reason);
    let poll_error = format!("incomplete snapshot: {reason}");
    let changed = transaction
        .execute(
            r#"UPDATE subscription_meta
                  SET updated_at = MAX(updated_at, ?4),
                      poll_failure_count = ?5,
                      next_poll_at = ?6,
                      last_poll_error = ?7,
                      last_incomplete_at = ?4,
                      last_incomplete_reason = ?8,
                      last_incomplete_fetched_pages = ?9,
                      last_incomplete_truncated = ?10,
                      last_incomplete_end_observed = ?11,
                      open_poll_generation = NULL,
                      open_snapshot_id = NULL
                WHERE account_key = ?1
                  AND open_poll_generation = ?2
                  AND open_snapshot_id = ?3"#,
            params![
                command.account_key.as_str(),
                token_sql.generation,
                token_sql.snapshot_id,
                incomplete_at,
                i64::from(failure_count),
                next_poll_at_sql,
                poll_error,
                reason,
                i64::from(command.fetched_pages),
                i64::from(command.truncated_by_limit),
                i64::from(command.end_observed),
            ],
        )
        .map_err(|error| map_write_error("consume incomplete poll token", error))?;
    require_consumed(&transaction, changed, &command.account_key, &command.token)?;
    transaction
        .commit()
        .map_err(|error| map_write_error("commit incomplete poll snapshot", error))?;
    Ok(RecordIncompleteSnapshotResult {
        token: command.token,
        inserted: effects.inserted,
        updated: effects.updated,
        unchanged: effects.unchanged,
        reactivated: effects.reactivated,
        incomplete_at: command.incomplete_at,
        failure_count,
        next_poll_at,
    })
}

pub(super) fn apply_complete_snapshot(
    connection: &mut Connection,
    command: ApplyCompleteSnapshotCommand,
) -> RepositoryResult<ApplyCompleteSnapshotResult> {
    let completed_at = command_integer("completed_at", command.completed_at)?;
    let next_poll_at = command_integer("next_poll_at", command.next_poll_at)?;
    if command.next_poll_at < command.completed_at {
        return Err(RepositoryError::InvalidInput {
            field: "next_poll_at",
            message: "next poll time cannot precede completed_at".to_string(),
        });
    }
    let token_sql = SqlToken::try_from(&command.token)?;
    let transaction = begin_immediate(connection, "apply complete poll snapshot")?;
    let meta = require_current_poll(&transaction, &command.account_key, &command.token)?;
    meta.validate_terminal_time("completed_at", command.completed_at)?;
    let effects = apply_seen_records(
        &transaction,
        &command.account_key,
        PollFenceContext {
            token: &command.token,
            snapshot_kind: PollSnapshotKind::Complete,
            fenced_at: command.completed_at,
        },
        command.completed_at,
        completed_at,
        meta.bootstrap_completed,
        command.new_record_policy,
        &command.records,
    )?;
    let missing = load_missing_for_deactivation(
        &transaction,
        &command.account_key,
        &command.token.snapshot_id,
        command.completed_at,
    )?;
    let deactivated = transaction
        .execute(
            DEACTIVATE_MISSING_SQL,
            params![
                command.account_key.as_str(),
                command.token.snapshot_id.as_str(),
                completed_at,
                TV_NOT_SUPPORTED_REASON,
                INACTIVE_SUBSCRIPTION_REASON,
            ],
        )
        .map_err(|error| {
            map_write_error("deactivate subscriptions missing from snapshot", error)
        })?;
    if deactivated != missing.len() {
        return Err(RepositoryError::Internal {
            message: format!(
                "missing-row deactivation changed {deactivated} rows after loading {}",
                missing.len()
            ),
        });
    }
    let missing_fence = PollFenceContext {
        token: &command.token,
        snapshot_kind: PollSnapshotKind::Complete,
        fenced_at: command.completed_at,
    };
    for existing in &missing {
        if let Some(attempt) = existing.running_attempt()? {
            let summary = existing.detail.summary();
            let head = &summary.head;
            append_poll_supersede_audit(
                &transaction,
                existing,
                &attempt,
                missing_fence,
                PollSupersedeReason::MissingFromCompleteSnapshot,
                SupersedeAfter {
                    revision: increment_revision(head.revision)?,
                    active: false,
                    execution_state: SubscriptionExecutionState::Idle,
                    media_kind: head.media_kind,
                    blocked_reason: Some(if head.media_kind == SubscriptionMediaKind::Tv {
                        TV_NOT_SUPPORTED_REASON
                    } else {
                        INACTIVE_SUBSCRIPTION_REASON
                    }),
                    target_title: summary.projection.title.as_str(),
                },
            )?;
        }
    }
    let changed = transaction
        .execute(
            r#"UPDATE subscription_meta
                  SET bootstrap_completed = 1,
                      updated_at = MAX(updated_at, ?4),
                      last_poll_success_at = ?4,
                      poll_failure_count = 0,
                      next_poll_at = ?5,
                      last_poll_error = NULL,
                      last_complete_snapshot_id = ?3,
                      open_poll_generation = NULL,
                      open_snapshot_id = NULL
                WHERE account_key = ?1
                  AND open_poll_generation = ?2
                  AND open_snapshot_id = ?3"#,
            params![
                command.account_key.as_str(),
                token_sql.generation,
                token_sql.snapshot_id,
                completed_at,
                next_poll_at,
            ],
        )
        .map_err(|error| map_write_error("consume completed poll token", error))?;
    require_consumed(&transaction, changed, &command.account_key, &command.token)?;
    transaction
        .commit()
        .map_err(|error| map_write_error("commit complete poll snapshot", error))?;
    Ok(ApplyCompleteSnapshotResult {
        token: command.token,
        inserted: effects.inserted,
        updated: effects.updated,
        unchanged: effects.unchanged,
        reactivated: effects.reactivated,
        deactivated,
        completed_at: command.completed_at,
        next_poll_at: command.next_poll_at,
    })
}

fn begin_immediate<'connection>(
    connection: &'connection mut Connection,
    context: &str,
) -> RepositoryResult<Transaction<'connection>> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| map_write_error(context, error))
}

#[derive(Debug)]
struct BeginMeta {
    state_version: i64,
    created_at: i64,
    updated_at: i64,
    last_poll_attempt_at: Option<i64>,
    last_poll_success_at: Option<i64>,
    poll_generation: i64,
}

impl BeginMeta {
    fn validate_command_time(&self, attempted_at: u64) -> RepositoryResult<()> {
        validate_state_version(self.state_version)?;
        let created_at = checked_u64("created_at", self.created_at)?;
        let updated_at = checked_u64("updated_at", self.updated_at)?;
        let last_attempt = checked_optional_u64("last_poll_attempt_at", self.last_poll_attempt_at)?;
        let last_success = checked_optional_u64("last_poll_success_at", self.last_poll_success_at)?;
        let lower_bound = [
            Some(created_at),
            Some(updated_at),
            last_attempt,
            last_success,
        ]
        .into_iter()
        .flatten()
        .max()
        .expect("created_at is always present");
        if attempted_at < lower_bound {
            return Err(RepositoryError::InvalidInput {
                field: "attempted_at",
                message: format!(
                    "poll begin time {attempted_at} precedes persisted account time {lower_bound}"
                ),
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
struct TerminalMeta {
    created_at: u64,
    updated_at: u64,
    last_poll_attempt_at: u64,
    failure_count: u32,
    bootstrap_completed: bool,
}

impl TerminalMeta {
    fn validate_terminal_time(
        &self,
        field: &'static str,
        terminal_at: u64,
    ) -> RepositoryResult<()> {
        let lower_bound = self
            .created_at
            .max(self.updated_at)
            .max(self.last_poll_attempt_at);
        if terminal_at < lower_bound {
            return Err(RepositoryError::InvalidInput {
                field,
                message: format!(
                    "poll terminal time {terminal_at} precedes persisted open-attempt time {lower_bound}"
                ),
            });
        }
        Ok(())
    }
}

fn require_current_poll(
    transaction: &Transaction<'_>,
    account_key: &str,
    attempted: &PollAttemptToken,
) -> RepositoryResult<TerminalMeta> {
    let row = transaction
        .query_row(
            r#"SELECT state_version, created_at, updated_at, last_poll_attempt_at,
                      poll_failure_count, bootstrap_completed, poll_generation,
                      open_poll_generation, open_snapshot_id
                 FROM subscription_meta
                WHERE account_key = ?1"#,
            [account_key],
            |row| {
                Ok(RawTerminalMeta {
                    state_version: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    last_poll_attempt_at: row.get(3)?,
                    poll_failure_count: row.get(4)?,
                    bootstrap_completed: row.get(5)?,
                    poll_generation: row.get(6)?,
                    open_poll_generation: row.get(7)?,
                    open_snapshot_id: row.get(8)?,
                })
            },
        )
        .optional()
        .map_err(|error| map_read_error("read current poll token", error))?;
    let Some(row) = row else {
        return Err(stale_poll(account_key, attempted, None));
    };
    validate_state_version(row.state_version)?;
    let current = row.current_token()?;
    if current.as_ref() != Some(attempted) {
        return Err(stale_poll(account_key, attempted, current));
    }
    let last_poll_attempt_at = row
        .last_poll_attempt_at
        .ok_or_else(|| corrupt("an open poll token has no last_poll_attempt_at"))?;
    Ok(TerminalMeta {
        created_at: checked_u64("created_at", row.created_at)?,
        updated_at: checked_u64("updated_at", row.updated_at)?,
        last_poll_attempt_at: checked_u64("last_poll_attempt_at", last_poll_attempt_at)?,
        failure_count: checked_u32("poll_failure_count", row.poll_failure_count)?,
        bootstrap_completed: checked_bool("bootstrap_completed", row.bootstrap_completed)?,
    })
}

#[derive(Debug)]
struct RawTerminalMeta {
    state_version: i64,
    created_at: i64,
    updated_at: i64,
    last_poll_attempt_at: Option<i64>,
    poll_failure_count: i64,
    bootstrap_completed: i64,
    poll_generation: i64,
    open_poll_generation: Option<i64>,
    open_snapshot_id: Option<String>,
}

impl RawTerminalMeta {
    fn current_token(&self) -> RepositoryResult<Option<PollAttemptToken>> {
        let poll_generation = checked_u64("poll_generation", self.poll_generation)?;
        match (self.open_poll_generation, self.open_snapshot_id.as_deref()) {
            (None, None) => Ok(None),
            (Some(open_generation), Some(snapshot_id)) => {
                let open_generation = checked_u64("open_poll_generation", open_generation)?;
                if open_generation != poll_generation {
                    return Err(corrupt(
                        "open_poll_generation does not equal poll_generation",
                    ));
                }
                Ok(Some(PollAttemptToken::new(
                    PollGeneration::try_new(open_generation).map_err(|error| {
                        persisted_validation("decode open poll generation", error)
                    })?,
                    SnapshotId::try_new(snapshot_id).map_err(|error| {
                        persisted_validation("decode open poll snapshot ID", error)
                    })?,
                )))
            }
            _ => Err(corrupt(
                "open_poll_generation and open_snapshot_id must both be NULL or both be present",
            )),
        }
    }
}

#[derive(Debug)]
struct SqlToken<'token> {
    generation: i64,
    snapshot_id: &'token str,
}

impl<'token> TryFrom<&'token PollAttemptToken> for SqlToken<'token> {
    type Error = RepositoryError;

    fn try_from(token: &'token PollAttemptToken) -> Result<Self, Self::Error> {
        Ok(Self {
            generation: command_integer("poll_generation", token.generation.value())?,
            snapshot_id: token.snapshot_id.as_str(),
        })
    }
}

fn require_consumed(
    transaction: &Transaction<'_>,
    changed: usize,
    account_key: &str,
    attempted: &PollAttemptToken,
) -> RepositoryResult<()> {
    if changed == 1 {
        return Ok(());
    }
    if changed > 1 {
        return Err(RepositoryError::Internal {
            message: format!("poll terminal metadata update changed {changed} primary-key rows"),
        });
    }
    let current = read_current_token(transaction, account_key)?;
    if current.as_ref() == Some(attempted) {
        return Err(RepositoryError::Internal {
            message:
                "exact poll token update matched no row even though the token remained current"
                    .to_string(),
        });
    }
    Err(stale_poll(account_key, attempted, current))
}

fn read_current_token(
    transaction: &Transaction<'_>,
    account_key: &str,
) -> RepositoryResult<Option<PollAttemptToken>> {
    transaction
        .query_row(
            r#"SELECT poll_generation, open_poll_generation, open_snapshot_id
                 FROM subscription_meta
                WHERE account_key = ?1"#,
            [account_key],
            |row| {
                Ok(RawTerminalMeta {
                    state_version: STATE_VERSION,
                    created_at: 0,
                    updated_at: 0,
                    last_poll_attempt_at: None,
                    poll_failure_count: 0,
                    bootstrap_completed: 0,
                    poll_generation: row.get(0)?,
                    open_poll_generation: row.get(1)?,
                    open_snapshot_id: row.get(2)?,
                })
            },
        )
        .optional()
        .map_err(|error| map_read_error("resolve current poll token", error))?
        .map_or(Ok(None), |row| row.current_token())
}

fn stale_poll(
    account_key: &str,
    attempted: &PollAttemptToken,
    current: Option<PollAttemptToken>,
) -> RepositoryError {
    RepositoryError::StalePoll {
        account_key: account_key.to_string(),
        attempted: attempted.clone(),
        current,
    }
}

fn require_single_write(changed: usize, context: &str) -> RepositoryResult<()> {
    if changed == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Internal {
            message: format!("{context} changed {changed} rows, expected exactly one"),
        })
    }
}

fn stable_snapshot_id(account_key: &str, generation: u64) -> RepositoryResult<SnapshotId> {
    let mut context = Context::new(&SHA256);
    context.update(SNAPSHOT_ID_DOMAIN);
    context.update(&(account_key.len() as u64).to_be_bytes());
    context.update(account_key.as_bytes());
    context.update(&generation.to_be_bytes());
    let digest = context.finish();
    let mut value = String::with_capacity(SNAPSHOT_ID_PREFIX.len() + digest.as_ref().len() * 2);
    value.push_str(SNAPSHOT_ID_PREFIX);
    for byte in digest.as_ref() {
        write!(&mut value, "{byte:02x}").expect("writing to String cannot fail");
    }
    SnapshotId::try_new(value)
}

fn incomplete_reason_label(
    reason: crate::subscription::repository::IncompleteSnapshotReason,
) -> &'static str {
    use crate::subscription::repository::IncompleteSnapshotReason;
    match reason {
        IncompleteSnapshotReason::ItemLimitReached => "item_limit_reached",
        IncompleteSnapshotReason::MaximumPageCountReached => "maximum_page_count_reached",
        IncompleteSnapshotReason::RepeatedPage => "repeated_page",
        IncompleteSnapshotReason::EndNotObserved => "end_not_observed",
    }
}

#[derive(Debug, Default)]
struct SeenEffects {
    inserted: usize,
    updated: usize,
    unchanged: usize,
    reactivated: usize,
}

#[derive(Debug, Clone, Copy)]
enum PollSnapshotKind {
    Complete,
    Incomplete,
}

impl PollSnapshotKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PollFenceContext<'a> {
    token: &'a PollAttemptToken,
    snapshot_kind: PollSnapshotKind,
    fenced_at: u64,
}

#[allow(clippy::too_many_arguments)]
fn apply_seen_records(
    transaction: &Transaction<'_>,
    account_key: &str,
    fence: PollFenceContext<'_>,
    observed_at: u64,
    observed_at_sql: i64,
    bootstrap_completed: bool,
    policy: NewRecordPolicy,
    records: &[SnapshotRecord],
) -> RepositoryResult<SeenEffects> {
    let mut effects = SeenEffects::default();
    for record in records {
        let existing = read_existing(transaction, account_key, &record.subject_id)?;
        match existing {
            Some(existing) => {
                let was_inactive = !existing.detail.summary().head.active;
                if update_existing_seen(
                    transaction,
                    account_key,
                    fence,
                    observed_at,
                    record,
                    existing,
                )? {
                    effects.updated += 1;
                    effects.reactivated += usize::from(was_inactive);
                } else {
                    effects.unchanged += 1;
                }
            }
            None => {
                insert_new_seen(
                    transaction,
                    account_key,
                    &fence.token.snapshot_id,
                    observed_at,
                    observed_at_sql,
                    !bootstrap_completed,
                    policy,
                    record,
                )?;
                effects.inserted += 1;
            }
        }
    }
    Ok(effects)
}

fn read_existing(
    transaction: &Transaction<'_>,
    account_key: &str,
    subject_id: &str,
) -> RepositoryResult<Option<ExistingRecord>> {
    transaction
        .query_row(
            READ_EXISTING_SQL,
            params![account_key, subject_id],
            RawExisting::read,
        )
        .optional()
        .map_err(|error| map_read_error("read subscription before snapshot merge", error))?
        .map(|raw| raw.try_into_existing(account_key, subject_id))
        .transpose()
}

#[derive(Debug)]
struct ExistingRecord {
    detail: SubscriptionDetail,
    claimed_operation: Option<ExecutionOperation>,
    attempt_id: Option<ExecutionAttemptId>,
    lease_until: Option<u64>,
}

#[derive(Debug, Clone)]
struct StoredExecutionAttempt {
    operation: ExecutionOperation,
    attempt_id: ExecutionAttemptId,
    lease_until: u64,
}

impl ExistingRecord {
    fn running_attempt(&self) -> RepositoryResult<Option<StoredExecutionAttempt>> {
        match self.detail.summary().head.execution_state {
            SubscriptionExecutionState::Idle => Ok(None),
            SubscriptionExecutionState::Running => Ok(Some(StoredExecutionAttempt {
                operation: self
                    .claimed_operation
                    .ok_or_else(|| corrupt("running row has no typed claimed operation"))?,
                attempt_id: self
                    .attempt_id
                    .clone()
                    .ok_or_else(|| corrupt("running row has no typed attempt ID"))?,
                lease_until: self
                    .lease_until
                    .ok_or_else(|| corrupt("running row has no typed lease"))?,
            })),
        }
    }
}

#[derive(Debug)]
struct RawExisting {
    revision: i64,
    active: i64,
    inactive_at: Option<i64>,
    last_seen_snapshot_id: Option<String>,
    media_kind: String,
    schedulable: i64,
    blocked_reason: Option<String>,
    lifecycle_state: String,
    execution_state: String,
    next_attempt_at: Option<i64>,
    retry_count: i64,
    max_retries: i64,
    retry_blocked: i64,
    force_eligible_once: i64,
    claimed_operation: Option<String>,
    attempt_id: Option<String>,
    lease_until: Option<i64>,
    title: String,
    release_year: Option<i64>,
    poster_url: String,
    category_text: Option<String>,
    douban_sort_time: Option<i64>,
    attention_tags_json: String,
    updated_at: i64,
    record_json: String,
}

impl RawExisting {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Self::read_at(row, 0)
    }

    fn read_at(row: &Row<'_>, offset: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            revision: row.get(offset)?,
            active: row.get(offset + 1)?,
            inactive_at: row.get(offset + 2)?,
            last_seen_snapshot_id: row.get(offset + 3)?,
            media_kind: row.get(offset + 4)?,
            schedulable: row.get(offset + 5)?,
            blocked_reason: row.get(offset + 6)?,
            lifecycle_state: row.get(offset + 7)?,
            execution_state: row.get(offset + 8)?,
            next_attempt_at: row.get(offset + 9)?,
            retry_count: row.get(offset + 10)?,
            max_retries: row.get(offset + 11)?,
            retry_blocked: row.get(offset + 12)?,
            force_eligible_once: row.get(offset + 13)?,
            claimed_operation: row.get(offset + 14)?,
            attempt_id: row.get(offset + 15)?,
            lease_until: row.get(offset + 16)?,
            title: row.get(offset + 17)?,
            release_year: row.get(offset + 18)?,
            poster_url: row.get(offset + 19)?,
            category_text: row.get(offset + 20)?,
            douban_sort_time: row.get(offset + 21)?,
            attention_tags_json: row.get(offset + 22)?,
            updated_at: row.get(offset + 23)?,
            record_json: row.get(offset + 24)?,
        })
    }

    fn try_into_existing(
        self,
        account_key: &str,
        subject_id: &str,
    ) -> RepositoryResult<ExistingRecord> {
        let key =
            crate::subscription::repository::SubscriptionKey::try_new(account_key, subject_id)
                .map_err(|error| persisted_validation("decode snapshot merge key", error))?;
        let revision = Revision::try_new(checked_u64("revision", self.revision)?)
            .map_err(|error| persisted_validation("decode snapshot merge revision", error))?;
        let media_kind = parse_media_kind(&self.media_kind)?;
        let lifecycle_state = parse_lifecycle_state(&self.lifecycle_state)?;
        let execution_state = parse_execution_state(&self.execution_state)?;
        let claimed_operation = self
            .claimed_operation
            .map(|value| {
                let value = checked_required_text("claimed_operation", value)?;
                ExecutionOperation::try_from_persisted(&value)
            })
            .transpose()?;
        let attempt_id = self
            .attempt_id
            .map(|value| {
                let value = checked_required_text("attempt_id", value)?;
                ExecutionAttemptId::try_new(value).map_err(|error| {
                    persisted_validation("decode poll execution attempt ID", error)
                })
            })
            .transpose()?;
        let lease_until = checked_optional_u64("lease_until", self.lease_until)?;
        match execution_state {
            SubscriptionExecutionState::Idle
                if claimed_operation.is_some() || attempt_id.is_some() || lease_until.is_some() =>
            {
                return Err(corrupt("idle row retains claim or lease controls"));
            }
            SubscriptionExecutionState::Running
                if claimed_operation.is_none() || attempt_id.is_none() || lease_until.is_none() =>
            {
                return Err(corrupt(
                    "running row has incomplete claim or lease controls",
                ));
            }
            _ => {}
        }
        if execution_state == SubscriptionExecutionState::Running {
            let expected = ExecutionOperation::for_lifecycle(lifecycle_state)
                .ok_or_else(|| corrupt("completed row retained a running execution attempt"))?;
            if claimed_operation != Some(expected) {
                return Err(corrupt(format!(
                    "persisted claimed operation does not match lifecycle {}",
                    lifecycle_state.as_str()
                )));
            }
        }
        let release_year = self
            .release_year
            .map(|value| {
                let value = checked_u64("release_year", value)?;
                u16::try_from(value).map_err(|_| corrupt("release_year exceeds u16 range"))
            })
            .transpose()?;
        let projection = SubscriptionProjection {
            title: self.title,
            release_year,
            poster_url: self.poster_url,
            category_text: self.category_text,
            douban_sort_time: checked_optional_u64("douban_sort_time", self.douban_sort_time)?,
        };
        let head = SubscriptionHead {
            key,
            revision,
            active: checked_bool("active", self.active)?,
            inactive_at: checked_optional_u64("inactive_at", self.inactive_at)?,
            last_seen_snapshot_id: self
                .last_seen_snapshot_id
                .map(SnapshotId::try_new)
                .transpose()
                .map_err(|error| persisted_validation("decode last seen snapshot ID", error))?,
            media_kind,
            schedulable: checked_bool("schedulable", self.schedulable)?,
            blocked_reason: self
                .blocked_reason
                .map(BlockedReason::try_new)
                .transpose()
                .map_err(|error| persisted_validation("decode blocked reason", error))?,
            lifecycle_state,
            execution_state,
            next_attempt_at: checked_optional_u64("next_attempt_at", self.next_attempt_at)?,
            retry_count: checked_u32("retry_count", self.retry_count)?,
            max_retries: checked_u32("max_retries", self.max_retries)?,
            retry_blocked: checked_bool("retry_blocked", self.retry_blocked)?,
            force_eligible_once: checked_bool("force_eligible_once", self.force_eligible_once)?,
            updated_at: checked_u64("updated_at", self.updated_at)?,
        };
        let summary = SubscriptionSummary {
            head,
            projection,
            attention_tags: parse_attention_tags(&self.attention_tags_json)?,
        };
        let payload =
            serde_json::from_str::<SubscriptionPayload>(&self.record_json).map_err(|error| {
                RepositoryError::CorruptData {
                    message: format!("decode snapshot merge record_json payload: {error}"),
                }
            })?;
        let detail = SubscriptionDetail::try_new(summary, payload)
            .map_err(|error| persisted_validation("validate snapshot merge detail", error))?;
        Ok(ExistingRecord {
            detail,
            claimed_operation,
            attempt_id,
            lease_until,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn update_existing_seen(
    transaction: &Transaction<'_>,
    account_key: &str,
    fence: PollFenceContext<'_>,
    observed_at: u64,
    observed: &SnapshotRecord,
    existing: ExistingRecord,
) -> RepositoryResult<bool> {
    let snapshot_id = &fence.token.snapshot_id;
    let summary = existing.detail.summary();
    let head = &summary.head;
    let was_inactive = !head.active;
    let media_kind = if head.media_kind == SubscriptionMediaKind::Tv
        || observed.media_kind == SubscriptionMediaKind::Tv
    {
        SubscriptionMediaKind::Tv
    } else {
        SubscriptionMediaKind::Movie
    };
    let superseded_attempt = if head.media_kind == SubscriptionMediaKind::Movie
        && media_kind == SubscriptionMediaKind::Tv
    {
        existing.running_attempt()?
    } else {
        None
    };
    let mut payload = existing.detail.payload().clone();
    payload.merge_snapshot_observation(&observed.source, observed_at)?;
    payload.validate_for(account_key, &observed.subject_id)?;
    let projection = SubscriptionProjection::from_source(&payload.source)?;

    let (schedulable, blocked_reason, execution_state, next_attempt_at, force_eligible_once) =
        if media_kind == SubscriptionMediaKind::Tv {
            (
                false,
                Some(TV_NOT_SUPPORTED_REASON.to_string()),
                SubscriptionExecutionState::Idle,
                None,
                false,
            )
        } else if was_inactive {
            let blocked_reason = observed
                .blocked_reason
                .as_ref()
                .map(BlockedReason::as_str)
                .map(str::to_string);
            let skipped = payload.skip_reason.is_some()
                || summary
                    .attention_tags
                    .contains(&SubscriptionAttentionTag::Skipped);
            let due = (observed.schedulable
                && head.lifecycle_state != SubscriptionLifecycleState::Completed
                && !head.retry_blocked
                && !skipped)
                .then_some(observed_at);
            (
                observed.schedulable,
                blocked_reason,
                SubscriptionExecutionState::Idle,
                due,
                false,
            )
        } else {
            (
                head.schedulable,
                head.blocked_reason
                    .as_ref()
                    .map(BlockedReason::as_str)
                    .map(str::to_string),
                head.execution_state,
                head.next_attempt_at,
                head.force_eligible_once,
            )
        };
    let (claimed_operation, attempt_id, lease_until) =
        if media_kind == SubscriptionMediaKind::Tv || was_inactive {
            (None, None, None)
        } else {
            (
                existing.claimed_operation,
                existing.attempt_id.clone(),
                existing.lease_until,
            )
        };
    let blocked_reason_typed = blocked_reason
        .as_deref()
        .map(BlockedReason::try_new)
        .transpose()?;
    let changed = !head.active
        || head.inactive_at.is_some()
        || head.last_seen_snapshot_id.as_ref() != Some(snapshot_id)
        || head.media_kind != media_kind
        || head.schedulable != schedulable
        || head.blocked_reason != blocked_reason_typed
        || head.execution_state != execution_state
        || head.next_attempt_at != next_attempt_at
        || head.force_eligible_once != force_eligible_once
        || existing.claimed_operation != claimed_operation
        || existing.attempt_id != attempt_id
        || existing.lease_until != lease_until
        || summary.projection != projection
        || existing.detail.payload() != &payload;
    if !changed {
        return Ok(false);
    }
    let revision = increment_revision(head.revision)?;
    let updated_at = head
        .updated_at
        .saturating_add(1)
        .min(i64::MAX as u64)
        .max(observed_at);
    let final_head = SubscriptionHead {
        key: head.key.clone(),
        revision,
        active: true,
        inactive_at: None,
        last_seen_snapshot_id: Some(snapshot_id.clone()),
        media_kind,
        schedulable,
        blocked_reason: blocked_reason_typed,
        lifecycle_state: head.lifecycle_state,
        execution_state,
        next_attempt_at,
        retry_count: head.retry_count,
        max_retries: head.max_retries,
        retry_blocked: head.retry_blocked,
        force_eligible_once,
        updated_at,
    };
    SubscriptionDetail::try_new(
        SubscriptionSummary {
            head: final_head,
            projection: projection.clone(),
            attention_tags: summary.attention_tags.clone(),
        },
        payload.clone(),
    )?;
    let record_json = encode_payload(&payload, "encode refreshed snapshot payload")?;
    let douban_sort_time = projection
        .douban_sort_time
        .map(|value| command_integer("payload.source.douban_sort_time", value))
        .transpose()?;
    let changed_rows = transaction
        .execute(
            UPDATE_SEEN_SQL,
            params![
                account_key,
                observed.subject_id.as_str(),
                command_integer("expected_revision", head.revision.value())?,
                command_integer("revision", revision.value())?,
                snapshot_id.as_str(),
                media_kind.as_str(),
                i64::from(schedulable),
                blocked_reason,
                execution_state_label(execution_state),
                next_attempt_at
                    .map(|value| command_integer("next_attempt_at", value))
                    .transpose()?,
                i64::from(force_eligible_once),
                claimed_operation.map(ExecutionOperation::as_str),
                attempt_id.as_ref().map(ExecutionAttemptId::as_str),
                lease_until
                    .map(|value| command_integer("lease_until", value))
                    .transpose()?,
                projection.title,
                projection.release_year.map(i64::from),
                projection.poster_url,
                projection.category_text,
                douban_sort_time,
                command_integer("updated_at", updated_at)?,
                record_json,
            ],
        )
        .map_err(|error| map_write_error("update seen subscription record", error))?;
    require_single_write(changed_rows, "update seen subscription record")?;
    if let Some(attempt) = superseded_attempt.as_ref() {
        append_poll_supersede_audit(
            transaction,
            &existing,
            attempt,
            fence,
            PollSupersedeReason::ParkedAsTvNotSupported,
            SupersedeAfter {
                revision,
                active: true,
                execution_state: SubscriptionExecutionState::Idle,
                media_kind: SubscriptionMediaKind::Tv,
                blocked_reason: Some(TV_NOT_SUPPORTED_REASON),
                target_title: projection.title.as_str(),
            },
        )?;
    }
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn insert_new_seen(
    transaction: &Transaction<'_>,
    account_key: &str,
    snapshot_id: &SnapshotId,
    observed_at: u64,
    observed_at_sql: i64,
    bootstrap_mode: bool,
    policy: NewRecordPolicy,
    observed: &SnapshotRecord,
) -> RepositoryResult<()> {
    let mut payload = SubscriptionPayload {
        source: observed.source.clone(),
        observation: ObservationPayload {
            created_at: observed_at,
            first_seen_at: observed_at,
            last_seen_at: observed_at,
        },
        ..SubscriptionPayload::default()
    };
    let bootstrap_skipped = bootstrap_mode && policy.bootstrap_existing_as_skipped();
    let attention_tags = if bootstrap_skipped {
        payload.skip_reason = Some(INITIAL_BOOTSTRAP_SKIP_REASON.to_string());
        vec![SubscriptionAttentionTag::Skipped]
    } else {
        Vec::new()
    };
    payload.validate_for(account_key, &observed.subject_id)?;
    let projection = SubscriptionProjection::from_source(&payload.source)?;
    let media_kind = observed.media_kind;
    let blocked_reason = observed
        .blocked_reason
        .as_ref()
        .map(BlockedReason::as_str)
        .map(str::to_string);
    let next_attempt_at =
        (media_kind == SubscriptionMediaKind::Movie && observed.schedulable).then_some(observed_at);
    let head = SubscriptionHead {
        key: crate::subscription::repository::SubscriptionKey::try_new(
            account_key,
            &observed.subject_id,
        )?,
        revision: Revision::try_new(1)?,
        active: true,
        inactive_at: None,
        last_seen_snapshot_id: Some(snapshot_id.clone()),
        media_kind,
        schedulable: observed.schedulable,
        blocked_reason: blocked_reason
            .as_deref()
            .map(BlockedReason::try_new)
            .transpose()?,
        lifecycle_state: SubscriptionLifecycleState::Queued,
        execution_state: SubscriptionExecutionState::Idle,
        next_attempt_at,
        retry_count: 0,
        max_retries: policy.max_retries(),
        retry_blocked: false,
        force_eligible_once: false,
        updated_at: observed_at,
    };
    SubscriptionDetail::try_new(
        SubscriptionSummary {
            head,
            projection: projection.clone(),
            attention_tags: attention_tags.clone(),
        },
        payload.clone(),
    )?;
    let record_json = encode_payload(&payload, "encode inserted snapshot payload")?;
    let attention_tags_json =
        serde_json::to_string(&attention_tags).map_err(|error| RepositoryError::Internal {
            message: format!("encode inserted snapshot attention tags: {error}"),
        })?;
    let douban_sort_time = projection
        .douban_sort_time
        .map(|value| command_integer("payload.source.douban_sort_time", value))
        .transpose()?;
    let changed = transaction
        .execute(
            INSERT_SEEN_SQL,
            params![
                account_key,
                observed.subject_id.as_str(),
                snapshot_id.as_str(),
                media_kind.as_str(),
                i64::from(observed.schedulable),
                blocked_reason,
                next_attempt_at
                    .map(|value| command_integer("next_attempt_at", value))
                    .transpose()?,
                i64::from(policy.max_retries()),
                projection.title,
                projection.release_year.map(i64::from),
                projection.poster_url,
                projection.category_text,
                douban_sort_time,
                attention_tags_json,
                observed_at_sql,
                record_json,
            ],
        )
        .map_err(|error| map_write_error("insert seen subscription record", error))?;
    require_single_write(changed, "insert seen subscription record")
}

fn increment_revision(current: Revision) -> RepositoryResult<Revision> {
    let value = current.value();
    if value >= i64::MAX as u64 {
        return Err(RepositoryError::CorruptData {
            message: "persisted subscription revision is exhausted at SQLite INTEGER max"
                .to_string(),
        });
    }
    Revision::try_new(value + 1)
}

#[derive(Debug, Clone, Copy)]
enum PollSupersedeReason {
    MissingFromCompleteSnapshot,
    ParkedAsTvNotSupported,
}

impl PollSupersedeReason {
    const fn as_str(self) -> &'static str {
        match self {
            Self::MissingFromCompleteSnapshot => "missing_from_complete_snapshot",
            Self::ParkedAsTvNotSupported => "parked_as_tv_not_supported",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SupersedeAfter<'a> {
    revision: Revision,
    active: bool,
    execution_state: SubscriptionExecutionState,
    media_kind: SubscriptionMediaKind,
    blocked_reason: Option<&'a str>,
    target_title: &'a str,
}

fn append_poll_supersede_audit(
    transaction: &Transaction<'_>,
    existing: &ExistingRecord,
    attempt: &StoredExecutionAttempt,
    fence: PollFenceContext<'_>,
    reason: PollSupersedeReason,
    after: SupersedeAfter<'_>,
) -> RepositoryResult<()> {
    let head = &existing.detail.summary().head;
    if head.execution_state != SubscriptionExecutionState::Running {
        return Err(corrupt(
            "poll supersede audit requires a previously running subscription",
        ));
    }
    if increment_revision(head.revision)? != after.revision {
        return Err(corrupt(
            "poll supersede audit revision does not match the row update",
        ));
    }
    if after.execution_state != SubscriptionExecutionState::Idle {
        return Err(corrupt(
            "poll supersede audit replacement must clear execution to idle",
        ));
    }
    let lease_state = if attempt.lease_until > fence.fenced_at {
        "live"
    } else {
        "expired"
    };
    append_execution_audit(
        transaction,
        ExecutionAuditEntry {
            account_key: head.key.account_key.clone(),
            created_at: fence.fenced_at,
            action: SUPERSEDE_ATTEMPT_ACTION,
            target_id: head.key.subject_id.clone(),
            target_title: after.target_title.to_string(),
            summary: "superseded an execution attempt during wanted poll persistence",
            related: json!({
                "schema": SUPERSEDE_AUDIT_SCHEMA,
                "disposition": "superseded",
                "reason": reason.as_str(),
                "attempt_id": attempt.attempt_id.as_str(),
                "claimed_operation": attempt.operation.as_str(),
                "lease_until": attempt.lease_until,
                "lease_state_at_fence": lease_state,
                "fenced_at": fence.fenced_at,
                "fenced_by": "wanted_poll",
                "poll_generation": fence.token.generation.value(),
                "poll_snapshot_id": fence.token.snapshot_id.as_str(),
                "poll_snapshot_kind": fence.snapshot_kind.as_str(),
                "revision_before": head.revision.value(),
                "revision_after": after.revision.value(),
                "execution_state_before": execution_state_label(head.execution_state),
                "execution_state_after": execution_state_label(after.execution_state),
                "active_before": head.active,
                "active_after": after.active,
                "media_kind_before": head.media_kind.as_str(),
                "media_kind_after": after.media_kind.as_str(),
                "blocked_reason_before": head.blocked_reason.as_ref().map(BlockedReason::as_str),
                "blocked_reason_after": after.blocked_reason,
                "replacement_attempt_id": Value::Null,
            }),
        },
    )
}

fn load_missing_for_deactivation(
    transaction: &Transaction<'_>,
    account_key: &str,
    snapshot_id: &SnapshotId,
    completed_at: u64,
) -> RepositoryResult<Vec<ExistingRecord>> {
    let mut statement = transaction
        .prepare(READ_MISSING_EXISTING_SQL)
        .map_err(|error| map_read_error("prepare missing-row payload validation", error))?;
    let rows = statement
        .query_map(params![account_key, snapshot_id.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, RawExisting::read_at(row, 1)?))
        })
        .map_err(|error| map_read_error("query missing-row payload validation", error))?;
    let mut missing = Vec::new();
    for row in rows {
        let (subject_id, raw) =
            row.map_err(|error| map_read_error("decode missing-row detail", error))?;
        let existing = raw.try_into_existing(account_key, &subject_id)?;
        let head = &existing.detail.summary().head;
        if head.revision.value() >= i64::MAX as u64 {
            return Err(RepositoryError::CorruptData {
                message: format!(
                    "persisted missing subject {subject_id} revision is exhausted at SQLite INTEGER max"
                ),
            });
        }
        let payload = existing.detail.payload();
        if completed_at < payload.observation.last_seen_at {
            return Err(RepositoryError::InvalidInput {
                field: "completed_at",
                message: format!(
                    "complete snapshot time {completed_at} precedes last observation {} for missing subject {subject_id}",
                    payload.observation.last_seen_at
                ),
            });
        }
        missing.push(existing);
    }
    Ok(missing)
}

fn validate_state_version(value: i64) -> RepositoryResult<()> {
    if value == STATE_VERSION {
        Ok(())
    } else {
        Err(corrupt(format!(
            "subscription state_version is {value}, expected {STATE_VERSION}"
        )))
    }
}

fn encode_payload(payload: &SubscriptionPayload, context: &str) -> RepositoryResult<String> {
    serde_json::to_string(payload).map_err(|error| RepositoryError::Internal {
        message: format!("{context}: {error}"),
    })
}

const fn execution_state_label(state: SubscriptionExecutionState) -> &'static str {
    match state {
        SubscriptionExecutionState::Idle => "idle",
        SubscriptionExecutionState::Running => "running",
    }
}

#[cfg(test)]
mod tests;
