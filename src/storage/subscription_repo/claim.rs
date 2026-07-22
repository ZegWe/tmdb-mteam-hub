use std::collections::BTreeSet;
use std::fmt::{self, Write as _};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use ring::rand::{SecureRandom, SystemRandom};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde_json::{json, Value};

use super::execution_audit::{append_execution_audit, ExecutionAuditEntry};
use super::{
    checked_optional_u64, checked_required_text, corrupt, load_detail, map_read_error,
    map_write_error, persisted_validation, SqliteSubscriptionRepository,
};
use crate::subscription::ports::{RepoFuture, SubscriptionExecutionRepository};
use crate::subscription::repository::{
    ClaimDueCommand, ClaimDueResult, ClaimOneCommand, ClaimOneResult, ClaimRejection,
    ClaimedSubscription, CurrentExecutionAttempt, ExecutionAttemptId, ExecutionAttemptToken,
    ExecutionOperation, ExpiredAttempt, ExtendExecutionLeaseCommand, ExtendExecutionLeaseResult,
    FailExecutionCommand, FailExecutionResult, FinishExecutionCommand, FinishExecutionResult,
    IssueOwnerPayload, IssuePayload, ReleaseExecutionCommand, ReleaseExecutionResult,
    RepositoryError, RepositoryResult, Revision, SubscriptionDetail, SubscriptionKey,
};
use crate::subscription::{
    SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
    SubscriptionMediaKind,
};

const MAX_ATTEMPT_ID_ATTEMPTS: usize = 8;
const CLAIM_ATTEMPT_ACTION: &str = "claim_attempt";
const RECLAIM_ATTEMPT_ACTION: &str = "reclaim_attempt";
const EXTEND_ATTEMPT_ACTION: &str = "extend_attempt";
const FINISH_ATTEMPT_ACTION: &str = "finish_attempt";
const FAIL_ATTEMPT_ACTION: &str = "fail_attempt";
const RELEASE_ATTEMPT_ACTION: &str = "release_attempt";

pub(super) const EXPIRED_CANDIDATE_SQL: &str = r#"
SELECT subject_id
  FROM wanted_subscription_records INDEXED BY wanted_records_expired_lease_v5_idx
 WHERE account_key = ?1
   AND execution_state = 'running'
   AND lease_until <= ?2
 ORDER BY lease_until, subject_id
 LIMIT 1
"#;

pub(super) const FORCE_CANDIDATE_SQL: &str = r#"
SELECT subject_id
  FROM wanted_subscription_records INDEXED BY wanted_records_force_v5_idx
 WHERE account_key = ?1
   AND active = 1
   AND schedulable = 1
   AND blocked_reason IS NULL
   AND media_kind = 'movie'
   AND lifecycle_state != 'completed'
   AND execution_state = 'idle'
   AND force_eligible_once = 1
   AND next_attempt_at IS NOT NULL
 ORDER BY next_attempt_at, updated_at, subject_id
 LIMIT 1
"#;

pub(super) const DUE_CANDIDATE_SQL: &str = r#"
SELECT subject_id
  FROM wanted_subscription_records INDEXED BY wanted_records_due_v5_idx
 WHERE account_key = ?1
   AND active = 1
   AND schedulable = 1
   AND blocked_reason IS NULL
   AND media_kind = 'movie'
   AND lifecycle_state != 'completed'
   AND execution_state = 'idle'
   AND force_eligible_once = 0
   AND retry_blocked = 0
   AND NOT EXISTS (
       SELECT 1
         FROM json_each(attention_tags_json) AS attention
        WHERE attention.value = 'skipped'
   )
   AND json_extract(record_json, '$.skip_reason') IS NULL
   AND next_attempt_at IS NOT NULL
   AND next_attempt_at <= ?2
 ORDER BY next_attempt_at, updated_at, subject_id
 LIMIT 1
"#;

const LOAD_EXECUTION_CONTROLS_SQL: &str = r#"
SELECT claimed_operation, attempt_id, lease_until
  FROM wanted_subscription_records
 WHERE account_key = ?1 AND subject_id = ?2
"#;

const CLAIM_IDLE_SQL: &str = r#"
UPDATE wanted_subscription_records
   SET revision = revision + 1,
       execution_state = 'running',
       claimed_operation = ?4,
       attempt_id = ?5,
       lease_until = ?6,
       updated_at = MAX(updated_at, ?7)
 WHERE account_key = ?1
   AND subject_id = ?2
   AND revision = ?3
   AND execution_state = 'idle'
   AND claimed_operation IS NULL
   AND attempt_id IS NULL
   AND lease_until IS NULL
"#;

const RECLAIM_EXPIRED_SQL: &str = r#"
UPDATE wanted_subscription_records
   SET revision = revision + 1,
       claimed_operation = ?4,
       attempt_id = ?5,
       lease_until = ?6,
       updated_at = MAX(updated_at, ?7)
 WHERE account_key = ?1
   AND subject_id = ?2
   AND revision = ?3
   AND execution_state = 'running'
   AND claimed_operation = ?8
   AND attempt_id = ?9
   AND lease_until = ?10
   AND lease_until <= ?11
"#;

const EXTEND_LEASE_SQL: &str = r#"
UPDATE wanted_subscription_records
   SET revision = revision + 1,
       lease_until = ?7,
       updated_at = MAX(updated_at, ?8)
 WHERE account_key = ?1
   AND subject_id = ?2
   AND execution_state = 'running'
   AND claimed_operation = ?3
   AND attempt_id = ?4
   AND lease_until = ?5
   AND lease_until > ?6
"#;

const CONSUME_ATTEMPT_SQL: &str = r#"
UPDATE wanted_subscription_records
   SET revision = revision + 1,
       lifecycle_state = ?7,
       execution_state = 'idle',
       next_attempt_at = ?8,
       retry_count = ?9,
       retry_blocked = ?10,
       force_eligible_once = 0,
       claimed_operation = NULL,
       attempt_id = NULL,
       lease_until = NULL,
       attention_tags_json = ?11,
       updated_at = MAX(updated_at, ?12),
       record_json = ?13
 WHERE account_key = ?1
   AND subject_id = ?2
   AND execution_state = 'running'
   AND claimed_operation = ?3
   AND attempt_id = ?4
   AND lease_until = ?5
   AND lease_until > ?6
"#;

const RELEASE_ATTEMPT_SQL: &str = r#"
UPDATE wanted_subscription_records
   SET revision = revision + 1,
       execution_state = 'idle',
       claimed_operation = NULL,
       attempt_id = NULL,
       lease_until = NULL,
       updated_at = MAX(updated_at, ?7)
 WHERE account_key = ?1
   AND subject_id = ?2
   AND execution_state = 'running'
   AND claimed_operation = ?3
   AND attempt_id = ?4
   AND lease_until = ?5
   AND lease_until > ?6
"#;

pub(super) trait RepositoryClock: Send + Sync {
    fn now_unix_seconds(&self) -> RepositoryResult<u64>;
}

pub(super) trait ExecutionAttemptIdSource: Send + Sync {
    fn next_attempt_id(&self) -> RepositoryResult<ExecutionAttemptId>;
}

#[derive(Clone)]
pub(super) struct ClaimDependencies {
    clock: Arc<dyn RepositoryClock>,
    attempt_ids: Arc<dyn ExecutionAttemptIdSource>,
}

impl fmt::Debug for ClaimDependencies {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimDependencies")
            .finish_non_exhaustive()
    }
}

impl ClaimDependencies {
    pub(super) fn system() -> Self {
        Self {
            clock: Arc::new(SystemRepositoryClock),
            attempt_ids: Arc::new(SystemExecutionAttemptIds::new()),
        }
    }

    #[cfg(test)]
    pub(super) fn new(
        clock: Arc<dyn RepositoryClock>,
        attempt_ids: Arc<dyn ExecutionAttemptIdSource>,
    ) -> Self {
        Self { clock, attempt_ids }
    }

    fn now_unix_seconds(&self) -> RepositoryResult<u64> {
        self.clock.now_unix_seconds()
    }

    fn next_attempt_id(&self) -> RepositoryResult<ExecutionAttemptId> {
        self.attempt_ids.next_attempt_id()
    }
}

#[derive(Debug)]
struct SystemRepositoryClock;

impl RepositoryClock for SystemRepositoryClock {
    fn now_unix_seconds(&self) -> RepositoryResult<u64> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .map_err(|error| RepositoryError::Unavailable {
                message: format!("read repository wall clock: {error}"),
            })
    }
}

#[derive(Debug)]
struct SystemExecutionAttemptIds {
    random: SystemRandom,
}

impl SystemExecutionAttemptIds {
    fn new() -> Self {
        Self {
            random: SystemRandom::new(),
        }
    }
}

impl ExecutionAttemptIdSource for SystemExecutionAttemptIds {
    fn next_attempt_id(&self) -> RepositoryResult<ExecutionAttemptId> {
        let mut nonce = [0_u8; 16];
        self.random
            .fill(&mut nonce)
            .map_err(|_| RepositoryError::Unavailable {
                message: "obtain operating-system randomness for execution attempt ID".to_string(),
            })?;
        let mut value = String::with_capacity("exec-v1-".len() + nonce.len() * 2);
        value.push_str("exec-v1-");
        for byte in nonce {
            write!(&mut value, "{byte:02x}")
                .expect("writing hexadecimal bytes to String cannot fail");
        }
        ExecutionAttemptId::try_new(value)
    }
}

impl SubscriptionExecutionRepository for SqliteSubscriptionRepository {
    fn claim_due(&self, command: ClaimDueCommand) -> RepoFuture<ClaimDueResult> {
        let dependencies = self.claim_dependencies.clone();
        self.executor
            .run(move |connection| claim_due(connection, command, dependencies))
    }

    fn claim_one(&self, command: ClaimOneCommand) -> RepoFuture<ClaimOneResult> {
        let dependencies = self.claim_dependencies.clone();
        self.executor
            .run(move |connection| claim_one(connection, command, dependencies))
    }

    fn extend_lease(
        &self,
        command: ExtendExecutionLeaseCommand,
    ) -> RepoFuture<ExtendExecutionLeaseResult> {
        let dependencies = self.claim_dependencies.clone();
        self.executor
            .run(move |connection| extend_lease(connection, command, dependencies))
    }

    fn finish(&self, command: FinishExecutionCommand) -> RepoFuture<FinishExecutionResult> {
        let dependencies = self.claim_dependencies.clone();
        self.executor
            .run(move |connection| finish(connection, command, dependencies))
    }

    fn fail(&self, command: FailExecutionCommand) -> RepoFuture<FailExecutionResult> {
        let dependencies = self.claim_dependencies.clone();
        self.executor
            .run(move |connection| fail(connection, command, dependencies))
    }

    fn release(&self, command: ReleaseExecutionCommand) -> RepoFuture<ReleaseExecutionResult> {
        let dependencies = self.claim_dependencies.clone();
        self.executor
            .run(move |connection| release(connection, command, dependencies))
    }
}

fn claim_due(
    connection: &mut Connection,
    command: ClaimDueCommand,
    dependencies: ClaimDependencies,
) -> RepositoryResult<ClaimDueResult> {
    debug_assert_eq!(command.limit(), 1);
    let transaction = begin_immediate(connection, "begin bounded due claim")?;
    let now = dependencies.now_unix_seconds()?;
    let now_sql = repository_integer("repository clock", now)?;
    let selected = select_due_candidate(&transaction, command.account_key(), now, now_sql)?;
    let Some((stored, eligibility)) = selected else {
        transaction
            .commit()
            .map_err(|error| map_write_error("commit empty bounded due claim", error))?;
        return Ok(ClaimDueResult::none_due());
    };
    let claimed = persist_claim(
        &transaction,
        stored,
        eligibility,
        ClaimTrigger::DueScan,
        now,
        command.lease_ttl().seconds(),
        &dependencies,
    )?;
    transaction
        .commit()
        .map_err(|error| map_write_error("commit bounded due claim", error))?;
    Ok(ClaimDueResult::claimed(claimed))
}

fn claim_one(
    connection: &mut Connection,
    command: ClaimOneCommand,
    dependencies: ClaimDependencies,
) -> RepositoryResult<ClaimOneResult> {
    let transaction = begin_immediate(connection, "begin manual subscription claim")?;
    let now = dependencies.now_unix_seconds()?;
    repository_integer("repository clock", now)?;
    let stored = load_stored_subscription(&transaction, command.key().clone())?;
    let eligibility = match classify(&stored, now)? {
        Classification::Eligible(eligibility) => eligibility,
        Classification::Rejected(rejection) => {
            transaction
                .commit()
                .map_err(|error| map_write_error("commit rejected manual claim", error))?;
            return Ok(ClaimOneResult::Rejected(rejection));
        }
    };
    let claimed = persist_claim(
        &transaction,
        stored,
        eligibility,
        ClaimTrigger::Manual,
        now,
        command.lease_ttl().seconds(),
        &dependencies,
    )?;
    transaction
        .commit()
        .map_err(|error| map_write_error("commit manual subscription claim", error))?;
    Ok(ClaimOneResult::Claimed(Box::new(claimed)))
}

fn extend_lease(
    connection: &mut Connection,
    command: ExtendExecutionLeaseCommand,
    dependencies: ClaimDependencies,
) -> RepositoryResult<ExtendExecutionLeaseResult> {
    let transaction = begin_immediate(connection, "begin execution lease extension")?;
    let now = dependencies.now_unix_seconds()?;
    let now_sql = repository_integer("repository clock", now)?;
    let stored = load_stored_subscription(&transaction, command.token().key().clone())?;
    let current = require_live_attempt(&stored, command.token(), now)?;
    let requested_lease_until = now
        .checked_add(command.lease_ttl().seconds())
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| RepositoryError::Unavailable {
            message: "repository clock plus execution lease TTL exceeds SQLite INTEGER range"
                .to_string(),
        })?;
    if requested_lease_until <= current.lease_until {
        return Err(RepositoryError::LeaseNotExtended {
            attempt: command.token().clone(),
            current_lease_until: current.lease_until,
            requested_lease_until,
        });
    }
    let expected_revision = next_revision(stored.detail.summary().head.revision)?;
    let expected_updated_at = stored.detail.summary().head.updated_at.max(now);
    let changed = transaction
        .execute(
            EXTEND_LEASE_SQL,
            params![
                command.token().key().account_key.as_str(),
                command.token().key().subject_id.as_str(),
                command.token().operation().as_str(),
                command.token().attempt_id().as_str(),
                repository_integer("current execution lease", current.lease_until)?,
                now_sql,
                repository_integer("extended execution lease", requested_lease_until)?,
                now_sql,
            ],
        )
        .map_err(|error| map_write_error("extend execution lease", error))?;
    expect_exact_attempt_update(changed, "execution lease extension")?;

    let post = load_stored_subscription(&transaction, command.token().key().clone())?;
    verify_preserved_detail(&stored.detail, &post.detail)?;
    verify_immutable_subscription_fields(&stored.detail, &post.detail)?;
    let persisted = match &post.controls {
        ExecutionControls::Running(persisted) => persisted,
        ExecutionControls::Idle => {
            return Err(corrupt(
                "execution lease extension unexpectedly consumed the attempt",
            ));
        }
    };
    if persisted.token != *command.token()
        || persisted.lease_until != requested_lease_until
        || post.detail.summary().head.revision != expected_revision
        || post.detail.summary().head.updated_at != expected_updated_at
        || post.detail.summary().head.lifecycle_state
            != stored.detail.summary().head.lifecycle_state
        || post.detail.summary().head.next_attempt_at
            != stored.detail.summary().head.next_attempt_at
        || post.detail.summary().head.retry_count != stored.detail.summary().head.retry_count
        || post.detail.summary().head.retry_blocked != stored.detail.summary().head.retry_blocked
        || post.detail.summary().head.force_eligible_once
            != stored.detail.summary().head.force_eligible_once
    {
        return Err(corrupt(
            "execution lease extension post-write controls do not match the exact attempt",
        ));
    }
    let attempt = CurrentExecutionAttempt::classified_by_repository_clock(
        persisted.token.clone(),
        persisted.lease_until,
    );
    append_extend_audit(
        &transaction,
        &post.detail,
        &attempt,
        current.lease_until,
        now,
    )?;
    transaction
        .commit()
        .map_err(|error| map_write_error("commit execution lease extension", error))?;
    Ok(ExtendExecutionLeaseResult::new(expected_revision, attempt))
}

fn finish(
    connection: &mut Connection,
    command: FinishExecutionCommand,
    dependencies: ClaimDependencies,
) -> RepositoryResult<FinishExecutionResult> {
    let transaction = begin_immediate(connection, "begin exact execution finish")?;
    let now = dependencies.now_unix_seconds()?;
    let now_sql = repository_integer("repository clock", now)?;
    let stored = load_stored_subscription(&transaction, command.token().key().clone())?;
    let current = require_live_attempt(&stored, command.token(), now)?;
    let expected_revision = next_revision(stored.detail.summary().head.revision)?;
    let expected_updated_at = stored.detail.summary().head.updated_at.max(now);
    let previous_force = stored.detail.summary().head.force_eligible_once;

    let mut payload = stored.detail.payload().clone();
    command
        .payload_delta()
        .apply_to_latest(command.token().key(), &mut payload)?;
    clear_execution_issue(&mut payload, command.token().operation());
    payload.validate_for(
        &command.token().key().account_key,
        &command.token().key().subject_id,
    )?;
    let attention_tags = terminal_attention_tags(
        &stored.detail.summary().attention_tags,
        command.disposition().waits_for_release(),
        false,
        false,
    );
    let next_attempt_at = schedule_from_repository_now(now, command.disposition().next_delay())?;
    let next_attempt_at_sql = next_attempt_at
        .map(|value| repository_integer("next execution attempt", value))
        .transpose()?;
    let attention_tags_json = encode_json("encode terminal attention tags", &attention_tags)?;
    let record_json = encode_json("encode terminal execution payload", &payload)?;
    let changed = transaction
        .execute(
            CONSUME_ATTEMPT_SQL,
            params![
                command.token().key().account_key.as_str(),
                command.token().key().subject_id.as_str(),
                command.token().operation().as_str(),
                command.token().attempt_id().as_str(),
                repository_integer("current execution lease", current.lease_until)?,
                now_sql,
                command.disposition().next_lifecycle().as_str(),
                next_attempt_at_sql,
                0_i64,
                0_i64,
                attention_tags_json,
                now_sql,
                record_json,
            ],
        )
        .map_err(|error| map_write_error("consume exact successful execution attempt", error))?;
    expect_exact_attempt_update(changed, "successful execution finish")?;

    let post = load_stored_subscription(&transaction, command.token().key().clone())?;
    verify_terminal_post_write(
        &post,
        &stored.detail,
        expected_revision,
        expected_updated_at,
        command.disposition().next_lifecycle(),
        next_attempt_at,
        0,
        false,
        &attention_tags,
        &payload,
    )?;
    append_finish_audit(
        &transaction,
        &post.detail,
        command.token(),
        current.lease_until,
        command.disposition().as_str(),
        previous_force,
        now,
    )?;
    let result = FinishExecutionResult::new(post.detail, command.token().clone(), now);
    transaction
        .commit()
        .map_err(|error| map_write_error("commit exact execution finish", error))?;
    Ok(result)
}

fn fail(
    connection: &mut Connection,
    command: FailExecutionCommand,
    dependencies: ClaimDependencies,
) -> RepositoryResult<FailExecutionResult> {
    let transaction = begin_immediate(connection, "begin exact execution failure")?;
    let now = dependencies.now_unix_seconds()?;
    let now_sql = repository_integer("repository clock", now)?;
    let stored = load_stored_subscription(&transaction, command.token().key().clone())?;
    let current = require_live_attempt(&stored, command.token(), now)?;
    let expected_revision = next_revision(stored.detail.summary().head.revision)?;
    let expected_updated_at = stored.detail.summary().head.updated_at.max(now);
    let previous_force = stored.detail.summary().head.force_eligible_once;
    let retry_count = stored
        .detail
        .summary()
        .head
        .retry_count
        .checked_add(1)
        .ok_or_else(|| corrupt("execution retry_count overflowed u32"))?;
    let retry_blocked = stored.detail.summary().head.max_retries > 0
        && retry_count >= stored.detail.summary().head.max_retries;
    let next_attempt_at = if retry_blocked {
        None
    } else {
        schedule_from_repository_now(now, Some(command.retry_after()))?
    };

    let mut payload = stored.detail.payload().clone();
    command
        .payload_delta()
        .apply_to_latest(command.token().key(), &mut payload)?;
    replace_execution_issue(
        &mut payload,
        command.token().operation(),
        command.error_type(),
        command.message(),
        now,
    );
    payload.validate_for(
        &command.token().key().account_key,
        &command.token().key().subject_id,
    )?;
    let attention_tags = terminal_attention_tags(
        &stored.detail.summary().attention_tags,
        false,
        true,
        retry_blocked,
    );
    let next_attempt_at_sql = next_attempt_at
        .map(|value| repository_integer("next execution retry", value))
        .transpose()?;
    let attention_tags_json = encode_json("encode failure attention tags", &attention_tags)?;
    let record_json = encode_json("encode failed execution payload", &payload)?;
    let changed = transaction
        .execute(
            CONSUME_ATTEMPT_SQL,
            params![
                command.token().key().account_key.as_str(),
                command.token().key().subject_id.as_str(),
                command.token().operation().as_str(),
                command.token().attempt_id().as_str(),
                repository_integer("current execution lease", current.lease_until)?,
                now_sql,
                stored.detail.summary().head.lifecycle_state.as_str(),
                next_attempt_at_sql,
                i64::from(retry_count),
                i64::from(retry_blocked),
                attention_tags_json,
                now_sql,
                record_json,
            ],
        )
        .map_err(|error| map_write_error("consume exact failed execution attempt", error))?;
    expect_exact_attempt_update(changed, "failed execution finish")?;

    let post = load_stored_subscription(&transaction, command.token().key().clone())?;
    verify_terminal_post_write(
        &post,
        &stored.detail,
        expected_revision,
        expected_updated_at,
        stored.detail.summary().head.lifecycle_state,
        next_attempt_at,
        retry_count,
        retry_blocked,
        &attention_tags,
        &payload,
    )?;
    append_fail_audit(
        &transaction,
        &post.detail,
        command.token(),
        current.lease_until,
        command.error_type(),
        previous_force,
        now,
    )?;
    let result = FailExecutionResult::new(post.detail, command.token().clone(), now);
    transaction
        .commit()
        .map_err(|error| map_write_error("commit exact execution failure", error))?;
    Ok(result)
}

fn release(
    connection: &mut Connection,
    command: ReleaseExecutionCommand,
    dependencies: ClaimDependencies,
) -> RepositoryResult<ReleaseExecutionResult> {
    let transaction = begin_immediate(connection, "begin exact execution release")?;
    let now = dependencies.now_unix_seconds()?;
    let now_sql = repository_integer("repository clock", now)?;
    let stored = load_stored_subscription(&transaction, command.token().key().clone())?;
    let current = require_live_attempt(&stored, command.token(), now)?;
    let expected_revision = next_revision(stored.detail.summary().head.revision)?;
    let expected_updated_at = stored.detail.summary().head.updated_at.max(now);
    let changed = transaction
        .execute(
            RELEASE_ATTEMPT_SQL,
            params![
                command.token().key().account_key.as_str(),
                command.token().key().subject_id.as_str(),
                command.token().operation().as_str(),
                command.token().attempt_id().as_str(),
                repository_integer("current execution lease", current.lease_until)?,
                now_sql,
                now_sql,
            ],
        )
        .map_err(|error| map_write_error("release exact execution attempt", error))?;
    expect_exact_attempt_update(changed, "execution release")?;

    let post = load_stored_subscription(&transaction, command.token().key().clone())?;
    if !matches!(&post.controls, ExecutionControls::Idle)
        || post.detail.summary().head.revision != expected_revision
        || post.detail.summary().head.updated_at != expected_updated_at
        || post.detail.summary().head.lifecycle_state
            != stored.detail.summary().head.lifecycle_state
        || post.detail.summary().head.next_attempt_at
            != stored.detail.summary().head.next_attempt_at
        || post.detail.summary().head.retry_count != stored.detail.summary().head.retry_count
        || post.detail.summary().head.max_retries != stored.detail.summary().head.max_retries
        || post.detail.summary().head.retry_blocked != stored.detail.summary().head.retry_blocked
        || post.detail.summary().head.force_eligible_once
            != stored.detail.summary().head.force_eligible_once
    {
        return Err(corrupt(
            "execution release changed controls beyond attempt ownership",
        ));
    }
    verify_preserved_detail(&stored.detail, &post.detail)?;
    verify_immutable_subscription_fields(&stored.detail, &post.detail)?;
    append_release_audit(
        &transaction,
        &post.detail,
        command.token(),
        current.lease_until,
        now,
    )?;
    let result = ReleaseExecutionResult::new(post.detail, command.token().clone(), now);
    transaction
        .commit()
        .map_err(|error| map_write_error("commit exact execution release", error))?;
    Ok(result)
}

fn begin_immediate<'connection>(
    connection: &'connection mut Connection,
    context: &str,
) -> RepositoryResult<Transaction<'connection>> {
    connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| map_write_error(context, error))
}

fn select_due_candidate(
    transaction: &Transaction<'_>,
    account_key: &str,
    now: u64,
    now_sql: i64,
) -> RepositoryResult<Option<(StoredSubscription, ClaimEligibility)>> {
    for (branch, sql) in [
        (CandidateBranch::Expired, EXPIRED_CANDIDATE_SQL),
        (CandidateBranch::Forced, FORCE_CANDIDATE_SQL),
        (CandidateBranch::Normal, DUE_CANDIDATE_SQL),
    ] {
        let subject_id = match branch {
            CandidateBranch::Forced => {
                transaction.query_row(sql, [account_key], |row| row.get::<_, String>(0))
            }
            CandidateBranch::Expired | CandidateBranch::Normal => {
                transaction.query_row(sql, params![account_key, now_sql], |row| {
                    row.get::<_, String>(0)
                })
            }
        }
        .optional()
        .map_err(|error| map_read_error(branch.read_context(), error))?;
        let Some(subject_id) = subject_id else {
            continue;
        };
        let key = SubscriptionKey::try_new(account_key, subject_id)
            .map_err(|error| persisted_validation("decode due candidate key", error))?;
        let stored = load_stored_subscription(transaction, key)?;
        let classification = classify(&stored, now)?;
        let Classification::Eligible(eligibility) = classification else {
            return Err(corrupt(format!(
                "{} index selected a subscription rejected by the typed claim contract",
                branch.label()
            )));
        };
        if !branch.matches(&eligibility) {
            return Err(corrupt(format!(
                "{} index selected a subscription classified as {}",
                branch.label(),
                eligibility.label()
            )));
        }
        return Ok(Some((stored, eligibility)));
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy)]
enum CandidateBranch {
    Expired,
    Forced,
    Normal,
}

impl CandidateBranch {
    const fn label(self) -> &'static str {
        match self {
            Self::Expired => "expired-lease",
            Self::Forced => "force-once",
            Self::Normal => "normal-due",
        }
    }

    const fn read_context(self) -> &'static str {
        match self {
            Self::Expired => "select one expired execution lease",
            Self::Forced => "select one forced idle subscription",
            Self::Normal => "select one normally due subscription",
        }
    }

    fn matches(self, eligibility: &ClaimEligibility) -> bool {
        matches!(
            (self, eligibility),
            (Self::Expired, ClaimEligibility::Expired(_))
                | (Self::Forced, ClaimEligibility::Forced { .. })
                | (Self::Normal, ClaimEligibility::Normal { .. })
        )
    }
}

#[derive(Debug)]
struct StoredSubscription {
    detail: SubscriptionDetail,
    controls: ExecutionControls,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecutionControls {
    Idle,
    Running(PersistedAttempt),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PersistedAttempt {
    token: ExecutionAttemptToken,
    lease_until: u64,
}

fn load_stored_subscription(
    transaction: &Transaction<'_>,
    key: SubscriptionKey,
) -> RepositoryResult<StoredSubscription> {
    let detail = load_detail(transaction, key.clone())?;
    let raw = transaction
        .query_row(
            LOAD_EXECUTION_CONTROLS_SQL,
            params![key.account_key.as_str(), key.subject_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| map_read_error("read execution attempt controls", error))?
        .ok_or_else(|| RepositoryError::Internal {
            message: format!(
                "subscription {}/{} disappeared inside an immediate transaction",
                key.account_key, key.subject_id
            ),
        })?;
    let execution_state = detail.summary().head.execution_state;
    let controls = match (execution_state, raw.0, raw.1, raw.2) {
        (SubscriptionExecutionState::Idle, None, None, None) => ExecutionControls::Idle,
        (SubscriptionExecutionState::Running, Some(operation), Some(attempt_id), Some(lease)) => {
            let operation = ExecutionOperation::try_from_persisted(&checked_required_text(
                "claimed_operation",
                operation,
            )?)?;
            let attempt_id =
                ExecutionAttemptId::try_new(checked_required_text("attempt_id", attempt_id)?)
                    .map_err(|error| persisted_validation("decode execution attempt ID", error))?;
            let lease_until = checked_optional_u64("lease_until", Some(lease))?
                .expect("the running match arm always provides a lease");
            let expected = ExecutionOperation::for_lifecycle(detail.summary().head.lifecycle_state)
                .ok_or_else(|| corrupt("completed subscription retained a running attempt"))?;
            if operation != expected {
                return Err(corrupt(format!(
                    "persisted operation {} does not match lifecycle {}",
                    operation.as_str(),
                    detail.summary().head.lifecycle_state.as_str()
                )));
            }
            if detail.summary().head.next_attempt_at.is_none() {
                return Err(corrupt(
                    "running execution attempt lost its preserved next_attempt_at",
                ));
            }
            ExecutionControls::Running(PersistedAttempt {
                token: ExecutionAttemptToken::new(key, attempt_id, operation),
                lease_until,
            })
        }
        (SubscriptionExecutionState::Idle, _, _, _) => {
            return Err(corrupt(
                "idle subscription retained execution attempt controls",
            ));
        }
        (SubscriptionExecutionState::Running, _, _, _) => {
            return Err(corrupt(
                "running subscription has incomplete execution attempt controls",
            ));
        }
    };
    Ok(StoredSubscription { detail, controls })
}

#[derive(Debug)]
enum Classification {
    Eligible(ClaimEligibility),
    Rejected(ClaimRejection),
}

#[derive(Debug, Clone)]
enum ClaimEligibility {
    Expired(PersistedAttempt),
    Forced { operation: ExecutionOperation },
    Normal { operation: ExecutionOperation },
}

impl ClaimEligibility {
    fn operation(&self) -> ExecutionOperation {
        match self {
            Self::Expired(previous) => previous.token.operation(),
            Self::Forced { operation } | Self::Normal { operation } => *operation,
        }
    }

    fn previous(&self) -> Option<&PersistedAttempt> {
        match self {
            Self::Expired(previous) => Some(previous),
            Self::Forced { .. } | Self::Normal { .. } => None,
        }
    }

    const fn label(&self) -> &'static str {
        match self {
            Self::Expired(_) => "expired_lease",
            Self::Forced { .. } => "force_once",
            Self::Normal { .. } => "normal_due",
        }
    }
}

fn classify(stored: &StoredSubscription, now: u64) -> RepositoryResult<Classification> {
    let summary = stored.detail.summary();
    let head = &summary.head;
    if !head.active {
        return Ok(Classification::Rejected(ClaimRejection::Inactive));
    }
    if head.media_kind != SubscriptionMediaKind::Movie {
        return Ok(Classification::Rejected(
            ClaimRejection::UnsupportedMediaKind {
                media_kind: head.media_kind,
            },
        ));
    }
    if head.lifecycle_state == SubscriptionLifecycleState::Completed {
        return Ok(Classification::Rejected(ClaimRejection::Completed));
    }
    if !head.schedulable {
        let blocked_reason = head
            .blocked_reason
            .clone()
            .ok_or_else(|| corrupt("unschedulable subscription has no blocked reason"))?;
        return Ok(Classification::Rejected(ClaimRejection::Unschedulable {
            blocked_reason,
        }));
    }
    let operation = ExecutionOperation::for_lifecycle(head.lifecycle_state)
        .ok_or_else(|| corrupt("claimable lifecycle has no execution operation"))?;
    match &stored.controls {
        ExecutionControls::Running(previous) => {
            if previous.token.operation() != operation {
                return Err(corrupt(
                    "running attempt operation changed during classification",
                ));
            }
            if previous.lease_until > now {
                return Ok(Classification::Rejected(ClaimRejection::LiveAttempt {
                    current: CurrentExecutionAttempt::classified_by_repository_clock(
                        previous.token.clone(),
                        previous.lease_until,
                    ),
                }));
            }
            Ok(Classification::Eligible(ClaimEligibility::Expired(
                previous.clone(),
            )))
        }
        ExecutionControls::Idle => {
            if head.force_eligible_once {
                return Ok(Classification::Eligible(ClaimEligibility::Forced {
                    operation,
                }));
            }
            if head.retry_blocked {
                return Ok(Classification::Rejected(ClaimRejection::RetryBlocked));
            }
            if summary
                .attention_tags
                .contains(&SubscriptionAttentionTag::Skipped)
                || stored.detail.payload().skip_reason.is_some()
            {
                return Ok(Classification::Rejected(ClaimRejection::Skipped));
            }
            if head.next_attempt_at.is_none_or(|due| due > now) {
                return Ok(Classification::Rejected(ClaimRejection::NotDue {
                    next_attempt_at: head.next_attempt_at,
                }));
            }
            Ok(Classification::Eligible(ClaimEligibility::Normal {
                operation,
            }))
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum ClaimTrigger {
    DueScan,
    Manual,
}

impl ClaimTrigger {
    const fn as_str(self) -> &'static str {
        match self {
            Self::DueScan => "claim_due",
            Self::Manual => "claim_one",
        }
    }
}

fn persist_claim(
    transaction: &Transaction<'_>,
    stored: StoredSubscription,
    eligibility: ClaimEligibility,
    trigger: ClaimTrigger,
    now: u64,
    lease_ttl: u64,
    dependencies: &ClaimDependencies,
) -> RepositoryResult<ClaimedSubscription> {
    let head = &stored.detail.summary().head;
    let key = head.key.clone();
    let expected_revision = head.revision;
    let expected_revision_sql =
        repository_integer("persisted revision", expected_revision.value())?;
    if expected_revision_sql == i64::MAX {
        return Err(corrupt(format!(
            "subscription {}/{} revision is exhausted at SQLite INTEGER max",
            key.account_key, key.subject_id
        )));
    }
    let expected_next_attempt_at = head.next_attempt_at;
    let expected_force = head.force_eligible_once;
    let expected_updated_at = head.updated_at.max(now);
    let operation = eligibility.operation();
    let previous = eligibility.previous().cloned();
    let lease_until = now
        .checked_add(lease_ttl)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| RepositoryError::Unavailable {
            message: "repository clock plus execution lease TTL exceeds SQLite INTEGER range"
                .to_string(),
        })?;
    let lease_until_sql = repository_integer("execution lease", lease_until)?;
    let now_sql = repository_integer("repository clock", now)?;

    let mut selected_attempt_id = None;
    for attempt_number in 0..MAX_ATTEMPT_ID_ATTEMPTS {
        let attempt_id = dependencies.next_attempt_id()?;
        if previous
            .as_ref()
            .is_some_and(|attempt| attempt.token.attempt_id() == &attempt_id)
        {
            if attempt_number + 1 == MAX_ATTEMPT_ID_ATTEMPTS {
                break;
            }
            continue;
        }
        let result = match &previous {
            None => transaction.execute(
                CLAIM_IDLE_SQL,
                params![
                    key.account_key.as_str(),
                    key.subject_id.as_str(),
                    expected_revision_sql,
                    operation.as_str(),
                    attempt_id.as_str(),
                    lease_until_sql,
                    now_sql,
                ],
            ),
            Some(previous) => transaction.execute(
                RECLAIM_EXPIRED_SQL,
                params![
                    key.account_key.as_str(),
                    key.subject_id.as_str(),
                    expected_revision_sql,
                    operation.as_str(),
                    attempt_id.as_str(),
                    lease_until_sql,
                    now_sql,
                    previous.token.operation().as_str(),
                    previous.token.attempt_id().as_str(),
                    repository_integer("expired execution lease", previous.lease_until)?,
                    now_sql,
                ],
            ),
        };
        match result {
            Ok(1) => {
                selected_attempt_id = Some(attempt_id);
                break;
            }
            Ok(changed) => {
                return Err(RepositoryError::Internal {
                    message: format!(
                        "execution claim changed {changed} rows for a selected composite primary key"
                    ),
                });
            }
            Err(error) if is_unique_constraint(&error) => {
                if attempt_number + 1 == MAX_ATTEMPT_ID_ATTEMPTS {
                    break;
                }
            }
            Err(error) => return Err(map_write_error("persist execution claim", error)),
        }
    }
    let attempt_id = selected_attempt_id.ok_or_else(|| RepositoryError::Unavailable {
        message: format!(
            "could not allocate a unique execution attempt ID after {MAX_ATTEMPT_ID_ATTEMPTS} attempts"
        ),
    })?;
    let expected_post_revision = Revision::try_new(expected_revision.value() + 1)
        .map_err(|error| persisted_validation("build post-claim revision", error))?;
    let post = load_stored_subscription(transaction, key.clone())?;
    if post.detail.summary().head.revision != expected_post_revision {
        return Err(corrupt(
            "execution claim did not increment revision exactly once",
        ));
    }
    if post.detail.summary().head.next_attempt_at != expected_next_attempt_at
        || post.detail.summary().head.force_eligible_once != expected_force
        || post.detail.summary().head.updated_at != expected_updated_at
    {
        return Err(corrupt(
            "execution claim changed due, force, or updated-time controls unexpectedly",
        ));
    }
    if post.detail.payload() != stored.detail.payload()
        || post.detail.summary().projection != stored.detail.summary().projection
        || post.detail.summary().attention_tags != stored.detail.summary().attention_tags
    {
        return Err(corrupt(
            "execution claim changed JSON-owned subscription detail",
        ));
    }
    let persisted = match &post.controls {
        ExecutionControls::Running(persisted) => persisted,
        ExecutionControls::Idle => {
            return Err(corrupt(
                "execution claim committed without running controls",
            ));
        }
    };
    if persisted.token.attempt_id() != &attempt_id
        || persisted.token.operation() != operation
        || persisted.lease_until != lease_until
    {
        return Err(corrupt(
            "execution claim controls do not match the allocated attempt",
        ));
    }
    let current = CurrentExecutionAttempt::classified_by_repository_clock(
        persisted.token.clone(),
        persisted.lease_until,
    );
    let replaced_expired_attempt = previous.as_ref().map(|attempt| {
        ExpiredAttempt::classified_by_repository_clock(attempt.token.clone(), attempt.lease_until)
    });
    let claimed = ClaimedSubscription::try_new(post.detail, current, replaced_expired_attempt)?;
    append_claim_audit(
        transaction,
        &claimed,
        previous.as_ref(),
        trigger,
        eligibility.label(),
        now,
    )?;
    Ok(claimed)
}

fn append_claim_audit(
    transaction: &Transaction<'_>,
    claimed: &ClaimedSubscription,
    previous: Option<&PersistedAttempt>,
    trigger: ClaimTrigger,
    eligibility: &'static str,
    now: u64,
) -> RepositoryResult<()> {
    let detail = claimed.detail();
    let head = &detail.summary().head;
    let attempt = claimed.attempt();
    let previous_json = previous.map_or(Value::Null, |previous| {
        json!({
            "attempt_id": previous.token.attempt_id().as_str(),
            "operation": previous.token.operation().as_str(),
            "lease_until": previous.lease_until,
        })
    });
    let (action, summary) = if previous.is_some() {
        (
            RECLAIM_ATTEMPT_ACTION,
            "reclaimed an expired subscription execution attempt",
        )
    } else {
        (
            CLAIM_ATTEMPT_ACTION,
            "claimed a subscription execution attempt",
        )
    };
    append_execution_audit(
        transaction,
        ExecutionAuditEntry {
            account_key: head.key.account_key.clone(),
            created_at: now,
            action,
            target_id: head.key.subject_id.clone(),
            target_title: detail.summary().projection.title.clone(),
            summary,
            related: json!({
                "schema_version": 1,
                "trigger": trigger.as_str(),
                "eligibility": eligibility,
                "new_attempt": {
                    "attempt_id": attempt.token().attempt_id().as_str(),
                    "operation": attempt.token().operation().as_str(),
                    "lease_until": attempt.lease_until(),
                },
                "previous_attempt": previous_json,
                "row_revision": head.revision.value(),
                "force_eligible_once": head.force_eligible_once,
            }),
        },
    )
}

fn require_live_attempt(
    stored: &StoredSubscription,
    attempted: &ExecutionAttemptToken,
    now: u64,
) -> RepositoryResult<PersistedAttempt> {
    match &stored.controls {
        ExecutionControls::Idle => Err(RepositoryError::StaleAttempt {
            attempted: Box::new(attempted.clone()),
            current: None,
        }),
        ExecutionControls::Running(current) if current.token != *attempted => {
            Err(RepositoryError::StaleAttempt {
                attempted: Box::new(attempted.clone()),
                current: Some(Box::new(current.token.clone())),
            })
        }
        ExecutionControls::Running(current) if current.lease_until <= now => {
            Err(RepositoryError::LeaseExpired {
                attempt: attempted.clone(),
                lease_until: current.lease_until,
            })
        }
        ExecutionControls::Running(current) => Ok(current.clone()),
    }
}

fn next_revision(current: Revision) -> RepositoryResult<Revision> {
    let value = current
        .value()
        .checked_add(1)
        .filter(|value| *value <= i64::MAX as u64)
        .ok_or_else(|| corrupt("subscription revision is exhausted at SQLite INTEGER max"))?;
    Revision::try_new(value)
        .map_err(|error| persisted_validation("build post-execution revision", error))
}

fn schedule_from_repository_now(
    now: u64,
    delay: Option<crate::subscription::repository::ExecutionScheduleDelay>,
) -> RepositoryResult<Option<u64>> {
    delay
        .map(|delay| {
            now.checked_add(delay.seconds())
                .filter(|value| *value <= i64::MAX as u64)
                .ok_or_else(|| RepositoryError::Unavailable {
                    message:
                        "repository clock plus execution schedule delay exceeds SQLite INTEGER range"
                            .to_string(),
                })
        })
        .transpose()
}

fn terminal_attention_tags(
    current: &[SubscriptionAttentionTag],
    waiting_release: bool,
    failed: bool,
    retry_blocked: bool,
) -> Vec<SubscriptionAttentionTag> {
    let mut tags = current
        .iter()
        .copied()
        .filter(|tag| {
            !matches!(
                tag,
                SubscriptionAttentionTag::WaitingRelease
                    | SubscriptionAttentionTag::Failed
                    | SubscriptionAttentionTag::RetryBlocked
                    | SubscriptionAttentionTag::NeedsReconciliation
            )
        })
        .collect::<BTreeSet<_>>();
    if waiting_release {
        tags.insert(SubscriptionAttentionTag::WaitingRelease);
    }
    if failed {
        tags.insert(SubscriptionAttentionTag::Failed);
    }
    if retry_blocked {
        tags.insert(SubscriptionAttentionTag::RetryBlocked);
    }
    tags.into_iter().collect()
}

fn clear_execution_issue(
    payload: &mut crate::subscription::repository::SubscriptionPayload,
    operation: ExecutionOperation,
) {
    payload.issues.retain(|issue| {
        !matches!(issue.owner, IssueOwnerPayload::Parent)
            || !issue
                .operation
                .as_deref()
                .is_some_and(|value| execution_issue_matches(operation, value))
    });
}

fn replace_execution_issue(
    payload: &mut crate::subscription::repository::SubscriptionPayload,
    operation: ExecutionOperation,
    error_type: &str,
    message: &str,
    occurred_at: u64,
) {
    clear_execution_issue(payload, operation);
    payload.issues.push(IssuePayload {
        owner: IssueOwnerPayload::Parent,
        operation: Some(operation.as_str().to_string()),
        error_type: Some(error_type.to_string()),
        message: message.to_string(),
        occurred_at: Some(occurred_at),
    });
}

fn execution_issue_matches(operation: ExecutionOperation, value: &str) -> bool {
    value == operation.as_str()
        || matches!(
            (operation, value),
            (ExecutionOperation::Meta, "meta")
                | (ExecutionOperation::Search, "search")
                | (ExecutionOperation::Progress, "progress")
                | (ExecutionOperation::Link, "link")
        )
}

fn encode_json<T: serde::Serialize>(context: &str, value: &T) -> RepositoryResult<String> {
    serde_json::to_string(value).map_err(|error| RepositoryError::Internal {
        message: format!("{context}: {error}"),
    })
}

fn expect_exact_attempt_update(changed: usize, context: &str) -> RepositoryResult<()> {
    if changed != 1 {
        return Err(RepositoryError::Internal {
            message: format!("{context} changed {changed} rows, expected one exact attempt"),
        });
    }
    Ok(())
}

fn verify_preserved_detail(
    before: &SubscriptionDetail,
    after: &SubscriptionDetail,
) -> RepositoryResult<()> {
    if before.payload() != after.payload()
        || before.summary().projection != after.summary().projection
        || before.summary().attention_tags != after.summary().attention_tags
    {
        return Err(corrupt(
            "execution control update changed JSON-owned subscription detail",
        ));
    }
    Ok(())
}

fn verify_immutable_subscription_fields(
    before: &SubscriptionDetail,
    after: &SubscriptionDetail,
) -> RepositoryResult<()> {
    let before_head = &before.summary().head;
    let after_head = &after.summary().head;
    if before_head.key != after_head.key
        || before_head.active != after_head.active
        || before_head.inactive_at != after_head.inactive_at
        || before_head.last_seen_snapshot_id != after_head.last_seen_snapshot_id
        || before_head.media_kind != after_head.media_kind
        || before_head.schedulable != after_head.schedulable
        || before_head.blocked_reason != after_head.blocked_reason
        || before_head.max_retries != after_head.max_retries
        || before.summary().projection != after.summary().projection
    {
        return Err(corrupt(
            "execution update changed subscription fields outside its ownership",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn verify_terminal_post_write(
    post: &StoredSubscription,
    before: &SubscriptionDetail,
    expected_revision: Revision,
    expected_updated_at: u64,
    expected_lifecycle: SubscriptionLifecycleState,
    expected_next_attempt_at: Option<u64>,
    expected_retry_count: u32,
    expected_retry_blocked: bool,
    expected_attention_tags: &[SubscriptionAttentionTag],
    expected_payload: &crate::subscription::repository::SubscriptionPayload,
) -> RepositoryResult<()> {
    verify_immutable_subscription_fields(before, &post.detail)?;
    let head = &post.detail.summary().head;
    if !matches!(&post.controls, ExecutionControls::Idle)
        || head.execution_state != SubscriptionExecutionState::Idle
        || head.revision != expected_revision
        || head.updated_at != expected_updated_at
        || head.lifecycle_state != expected_lifecycle
        || head.next_attempt_at != expected_next_attempt_at
        || head.retry_count != expected_retry_count
        || head.retry_blocked != expected_retry_blocked
        || head.force_eligible_once
        || post.detail.summary().attention_tags != expected_attention_tags
        || post.detail.payload() != expected_payload
    {
        return Err(corrupt(
            "execution terminal post-write state does not match the exact result",
        ));
    }
    Ok(())
}

fn append_extend_audit(
    transaction: &Transaction<'_>,
    detail: &SubscriptionDetail,
    attempt: &CurrentExecutionAttempt,
    previous_lease_until: u64,
    now: u64,
) -> RepositoryResult<()> {
    append_execution_audit(
        transaction,
        ExecutionAuditEntry {
            account_key: detail.summary().head.key.account_key.clone(),
            created_at: now,
            action: EXTEND_ATTEMPT_ACTION,
            target_id: detail.summary().head.key.subject_id.clone(),
            target_title: detail.summary().projection.title.clone(),
            summary: "extended a live subscription execution lease",
            related: json!({
                "schema_version": 1,
                "attempt_id": attempt.token().attempt_id().as_str(),
                "operation": attempt.token().operation().as_str(),
                "previous_lease_until": previous_lease_until,
                "lease_until": attempt.lease_until(),
                "row_revision": detail.summary().head.revision.value(),
            }),
        },
    )
}

fn append_finish_audit(
    transaction: &Transaction<'_>,
    detail: &SubscriptionDetail,
    attempt: &ExecutionAttemptToken,
    lease_until: u64,
    disposition: &'static str,
    previous_force: bool,
    now: u64,
) -> RepositoryResult<()> {
    append_execution_audit(
        transaction,
        ExecutionAuditEntry {
            account_key: detail.summary().head.key.account_key.clone(),
            created_at: now,
            action: FINISH_ATTEMPT_ACTION,
            target_id: detail.summary().head.key.subject_id.clone(),
            target_title: detail.summary().projection.title.clone(),
            summary: "finished a live subscription execution attempt",
            related: json!({
                "schema_version": 1,
                "attempt_id": attempt.attempt_id().as_str(),
                "operation": attempt.operation().as_str(),
                "lease_until": lease_until,
                "disposition": disposition,
                "next_lifecycle_state": detail.summary().head.lifecycle_state.as_str(),
                "next_attempt_at": detail.summary().head.next_attempt_at,
                "row_revision": detail.summary().head.revision.value(),
                "force_eligible_once_consumed": previous_force,
            }),
        },
    )
}

fn append_fail_audit(
    transaction: &Transaction<'_>,
    detail: &SubscriptionDetail,
    attempt: &ExecutionAttemptToken,
    lease_until: u64,
    error_type: &str,
    previous_force: bool,
    now: u64,
) -> RepositoryResult<()> {
    append_execution_audit(
        transaction,
        ExecutionAuditEntry {
            account_key: detail.summary().head.key.account_key.clone(),
            created_at: now,
            action: FAIL_ATTEMPT_ACTION,
            target_id: detail.summary().head.key.subject_id.clone(),
            target_title: detail.summary().projection.title.clone(),
            summary: "recorded a failed live subscription execution attempt",
            related: json!({
                "schema_version": 1,
                "attempt_id": attempt.attempt_id().as_str(),
                "operation": attempt.operation().as_str(),
                "lease_until": lease_until,
                "error_type": error_type,
                "retry_count": detail.summary().head.retry_count,
                "retry_blocked": detail.summary().head.retry_blocked,
                "next_attempt_at": detail.summary().head.next_attempt_at,
                "row_revision": detail.summary().head.revision.value(),
                "force_eligible_once_consumed": previous_force,
            }),
        },
    )
}

fn append_release_audit(
    transaction: &Transaction<'_>,
    detail: &SubscriptionDetail,
    attempt: &ExecutionAttemptToken,
    lease_until: u64,
    now: u64,
) -> RepositoryResult<()> {
    append_execution_audit(
        transaction,
        ExecutionAuditEntry {
            account_key: detail.summary().head.key.account_key.clone(),
            created_at: now,
            action: RELEASE_ATTEMPT_ACTION,
            target_id: detail.summary().head.key.subject_id.clone(),
            target_title: detail.summary().projection.title.clone(),
            summary: "released a live subscription execution attempt before side effects",
            related: json!({
                "schema_version": 1,
                "attempt_id": attempt.attempt_id().as_str(),
                "operation": attempt.operation().as_str(),
                "lease_until": lease_until,
                "external_effect": "not_started",
                "row_revision": detail.summary().head.revision.value(),
                "force_eligible_once_preserved": detail.summary().head.force_eligible_once,
                "next_attempt_at": detail.summary().head.next_attempt_at,
            }),
        },
    )
}

fn repository_integer(context: &str, value: u64) -> RepositoryResult<i64> {
    i64::try_from(value).map_err(|_| RepositoryError::Unavailable {
        message: format!("{context} exceeds SQLite INTEGER range"),
    })
}

fn is_unique_constraint(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if inner.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

#[cfg(test)]
mod tests;
