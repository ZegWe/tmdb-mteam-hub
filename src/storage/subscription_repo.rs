use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row, TransactionBehavior};

use super::sqlite::{create_fresh_v5_database, map_read_error, map_write_error, SqliteExecutor};
use crate::subscription::ports::{
    RepoFuture, SubscriptionMutationRepository, SubscriptionReadRepository,
};
use crate::subscription::repository::{
    BlockedReason, ListCursor, ListSubscriptionsCommand, RepositoryError, RepositoryResult,
    Revision, SnapshotId, SubscriptionDetail, SubscriptionHead, SubscriptionKey,
    SubscriptionListPage, SubscriptionPayload, SubscriptionProjection, SubscriptionSummary,
    UpdateSubscriptionDetailCommand, UpdateSubscriptionDetailResult,
};
use crate::subscription::{
    SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
    SubscriptionMediaKind,
};

mod claim;
mod execution_audit;
mod operation_logs;
mod poll;

#[cfg(test)]
mod evidence_support {
    use rusqlite::{params, Connection};

    const INSERT_GENERATED_CLONES_SQL: &str = r#"
WITH RECURSIVE generated(sequence) AS (
    SELECT 0
    UNION ALL
    SELECT sequence + 1
      FROM generated
     WHERE sequence + 1 < ?4
)
INSERT INTO wanted_subscription_records (
    account_key, subject_id, revision, active, inactive_at, last_seen_snapshot_id,
    media_kind, schedulable, blocked_reason, lifecycle_state, execution_state,
    next_attempt_at, retry_count, max_retries, retry_blocked, force_eligible_once,
    claimed_operation, attempt_id, lease_until, title, release_year, poster_url,
    category_text, douban_sort_time, attention_tags_json, updated_at, record_json
)
SELECT source.account_key,
       printf('%s-%05d', ?3, generated.sequence),
       source.revision,
       source.active,
       source.inactive_at,
       source.last_seen_snapshot_id,
       source.media_kind,
       source.schedulable,
       source.blocked_reason,
       source.lifecycle_state,
       source.execution_state,
       source.next_attempt_at,
       source.retry_count,
       source.max_retries,
       source.retry_blocked,
       source.force_eligible_once,
       source.claimed_operation,
       source.attempt_id,
       source.lease_until,
       source.title,
       source.release_year,
       source.poster_url,
       source.category_text,
       source.douban_sort_time,
       source.attention_tags_json,
       source.updated_at,
       source.record_json
  FROM wanted_subscription_records AS source
 CROSS JOIN generated
 WHERE source.account_key = ?1
   AND source.subject_id = ?2
"#;

    #[derive(Debug, PartialEq, Eq)]
    pub(super) struct UnrelatedStorageSnapshot {
        records: Vec<UnrelatedRecordSnapshot>,
        account_meta: Vec<u8>,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct UnrelatedRecordSnapshot {
        subject_id: String,
        revision: i64,
        attention_tags_json: Vec<u8>,
        record_json: Vec<u8>,
        scalar_values: Vec<u8>,
    }

    pub(super) fn insert_generated_clones(
        connection: &Connection,
        account_key: &str,
        source_subject_id: &str,
        prefix: &str,
        count: u32,
    ) {
        assert!(count > 0, "generated clone count must be positive");
        let changed = connection
            .execute(
                INSERT_GENERATED_CLONES_SQL,
                params![account_key, source_subject_id, prefix, i64::from(count)],
            )
            .expect("insert generated subscription clones");
        assert_eq!(
            changed,
            usize::try_from(count).expect("test clone count fits usize")
        );
    }

    pub(super) fn unrelated_storage_snapshot(
        connection: &Connection,
        account_key: &str,
        excluded_subject_id: &str,
    ) -> UnrelatedStorageSnapshot {
        let mut statement = connection
            .prepare(
                r#"SELECT subject_id,
                          revision,
                          CAST(attention_tags_json AS BLOB),
                          CAST(record_json AS BLOB),
                          CAST(json_array(
                              active, inactive_at, last_seen_snapshot_id, media_kind,
                              schedulable, blocked_reason, lifecycle_state, execution_state,
                              next_attempt_at, retry_count, max_retries, retry_blocked,
                              force_eligible_once, claimed_operation, attempt_id, lease_until,
                              title, release_year, poster_url, category_text, douban_sort_time,
                              updated_at
                          ) AS BLOB)
                     FROM wanted_subscription_records
                    WHERE account_key = ?1 AND subject_id != ?2
                    ORDER BY subject_id"#,
            )
            .expect("prepare unrelated subscription storage snapshot");
        let records = statement
            .query_map(params![account_key, excluded_subject_id], |row| {
                Ok(UnrelatedRecordSnapshot {
                    subject_id: row.get(0)?,
                    revision: row.get(1)?,
                    attention_tags_json: row.get(2)?,
                    record_json: row.get(3)?,
                    scalar_values: row.get(4)?,
                })
            })
            .expect("query unrelated subscription storage snapshot")
            .collect::<Result<Vec<_>, _>>()
            .expect("decode unrelated subscription storage snapshot");
        let account_meta = connection
            .query_row(
                r#"SELECT CAST(json_array(
                              state_version, bootstrap_completed, created_at, updated_at,
                              last_poll_attempt_at, last_poll_success_at, poll_failure_count,
                              next_poll_at, last_poll_error, poll_generation,
                              open_poll_generation, open_snapshot_id, last_incomplete_at,
                              last_incomplete_reason, last_incomplete_fetched_pages,
                              last_incomplete_truncated, last_incomplete_end_observed,
                              last_complete_snapshot_id
                          ) AS BLOB)
                     FROM subscription_meta
                    WHERE account_key = ?1"#,
                [account_key],
                |row| row.get(0),
            )
            .expect("snapshot exact account metadata values");
        UnrelatedStorageSnapshot {
            records,
            account_meta,
        }
    }
}

const GET_SQL: &str = r#"
SELECT account_key,
       subject_id,
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
       updated_at
  FROM wanted_subscription_records
 WHERE account_key = ?1 AND subject_id = ?2
"#;

const DETAIL_SQL: &str = r#"
SELECT account_key,
       subject_id,
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
       updated_at,
       title,
       release_year,
       poster_url,
       category_text,
       douban_sort_time,
       attention_tags_json,
       record_json
  FROM wanted_subscription_records
 WHERE account_key = ?1 AND subject_id = ?2
"#;

const UPDATE_DETAIL_SQL: &str = r#"
UPDATE wanted_subscription_records
   SET revision = revision + 1,
       title = ?4,
       release_year = ?5,
       poster_url = ?6,
       category_text = ?7,
       douban_sort_time = ?8,
       attention_tags_json = ?9,
       updated_at = MAX(updated_at, ?10),
       record_json = ?11
 WHERE account_key = ?1
   AND subject_id = ?2
   AND revision = ?3
"#;

const SUMMARY_COLUMNS: &str = r#"w.account_key,
       w.subject_id,
       w.revision,
       w.active,
       w.inactive_at,
       w.last_seen_snapshot_id,
       w.media_kind,
       w.schedulable,
       w.blocked_reason,
       w.lifecycle_state,
       w.execution_state,
       w.next_attempt_at,
       w.retry_count,
       w.max_retries,
       w.retry_blocked,
       w.force_eligible_once,
       w.updated_at,
       w.title,
       w.release_year,
       w.poster_url,
       w.category_text,
       w.douban_sort_time,
       w.attention_tags_json"#;

#[derive(Debug, Clone)]
pub(crate) struct SqliteSubscriptionRepository {
    executor: SqliteExecutor,
    claim_dependencies: claim::ClaimDependencies,
}

impl SqliteSubscriptionRepository {
    pub(crate) fn try_create_fresh(
        path: impl Into<PathBuf>,
        max_concurrency: usize,
        busy_timeout: Duration,
    ) -> RepositoryResult<Self> {
        let path = path.into();
        let executor = SqliteExecutor::try_new(path.clone(), max_concurrency, busy_timeout)?;
        create_fresh_v5_database(&path, busy_timeout)?;
        Ok(Self {
            executor,
            claim_dependencies: claim::ClaimDependencies::system(),
        })
    }

    pub(crate) fn try_new(
        path: impl Into<PathBuf>,
        max_concurrency: usize,
        busy_timeout: Duration,
    ) -> RepositoryResult<Self> {
        Ok(Self {
            executor: SqliteExecutor::try_new(path, max_concurrency, busy_timeout)?,
            claim_dependencies: claim::ClaimDependencies::system(),
        })
    }

    #[cfg(test)]
    fn try_new_with_claim_dependencies(
        path: impl Into<PathBuf>,
        max_concurrency: usize,
        busy_timeout: Duration,
        claim_dependencies: claim::ClaimDependencies,
    ) -> RepositoryResult<Self> {
        Ok(Self {
            executor: SqliteExecutor::try_new(path, max_concurrency, busy_timeout)?,
            claim_dependencies,
        })
    }

    pub(crate) fn preflight(&self) -> RepoFuture<()> {
        self.executor.preflight()
    }
}

impl SubscriptionReadRepository for SqliteSubscriptionRepository {
    fn get(&self, key: SubscriptionKey) -> RepoFuture<SubscriptionHead> {
        self.executor.run(move |connection| get(connection, key))
    }

    fn list_summaries(
        &self,
        command: ListSubscriptionsCommand,
    ) -> RepoFuture<SubscriptionListPage> {
        self.executor
            .run(move |connection| list_summaries(connection, command))
    }

    fn load_detail(&self, key: SubscriptionKey) -> RepoFuture<SubscriptionDetail> {
        self.executor
            .run(move |connection| load_detail(connection, key))
    }
}

impl SubscriptionMutationRepository for SqliteSubscriptionRepository {
    fn update_detail(
        &self,
        command: UpdateSubscriptionDetailCommand,
    ) -> RepoFuture<UpdateSubscriptionDetailResult> {
        self.executor
            .run(move |connection| update_detail(connection, command))
    }
}

fn get(connection: &Connection, key: SubscriptionKey) -> RepositoryResult<SubscriptionHead> {
    let raw = connection
        .query_row(
            GET_SQL,
            params![key.account_key.as_str(), key.subject_id.as_str()],
            RawHeadRow::read,
        )
        .optional()
        .map_err(|error| map_read_error("read subscription head by primary key", error))?
        .ok_or_else(|| RepositoryError::NotFound { key: key.clone() })?;
    raw.try_into_head()
}

fn load_detail(
    connection: &Connection,
    key: SubscriptionKey,
) -> RepositoryResult<SubscriptionDetail> {
    let raw = connection
        .query_row(
            DETAIL_SQL,
            params![key.account_key.as_str(), key.subject_id.as_str()],
            RawDetailRow::read,
        )
        .optional()
        .map_err(|error| map_read_error("read subscription detail by primary key", error))?
        .ok_or_else(|| RepositoryError::NotFound { key: key.clone() })?;
    raw.try_into_detail()
}

fn update_detail(
    connection: &mut Connection,
    command: UpdateSubscriptionDetailCommand,
) -> RepositoryResult<UpdateSubscriptionDetailResult> {
    let key = command.key().clone();
    let expected_revision = command.expected_revision();
    let expected_revision_sql = command_integer("expected_revision", expected_revision.value())?;
    if expected_revision_sql == i64::MAX {
        return Err(RepositoryError::InvalidInput {
            field: "expected_revision",
            message: "revision cannot be incremented beyond SQLite INTEGER range".to_string(),
        });
    }
    let updated_at = command_integer("updated_at", command.updated_at())?;
    let projection = SubscriptionProjection::from_source(&command.payload().source)?;
    let douban_sort_time = projection
        .douban_sort_time
        .map(|value| command_integer("payload.source.douban_sort_time", value))
        .transpose()?;
    let release_year = projection.release_year.map(i64::from);
    let attention_tags_json = serde_json::to_string(command.attention_tags()).map_err(|error| {
        RepositoryError::Internal {
            message: format!("encode attention tags for optimistic detail update: {error}"),
        }
    })?;
    let record_json =
        serde_json::to_string(command.payload()).map_err(|error| RepositoryError::Internal {
            message: format!("encode payload for optimistic detail update: {error}"),
        })?;

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| map_write_error("begin optimistic detail update", error))?;
    let current = load_detail(&transaction, key.clone())?;
    let actual_revision = current.summary().head.revision;
    if actual_revision != expected_revision {
        return Err(RepositoryError::RevisionConflict {
            key,
            expected: expected_revision,
            actual: actual_revision,
        });
    }
    command.validate_for_media_kind(current.summary().head.media_kind)?;
    if current.summary().head.execution_state == SubscriptionExecutionState::Running {
        let current_tags = current
            .summary()
            .attention_tags
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        let requested_tags = command
            .attention_tags()
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if current_tags != requested_tags
            || current.payload().skip_reason != command.payload().skip_reason
        {
            return Err(RepositoryError::ExecutionGateConflict { key });
        }
    }
    let changed = transaction
        .execute(
            UPDATE_DETAIL_SQL,
            params![
                key.account_key.as_str(),
                key.subject_id.as_str(),
                expected_revision_sql,
                projection.title,
                release_year,
                projection.poster_url,
                projection.category_text,
                douban_sort_time,
                attention_tags_json,
                updated_at,
                record_json,
            ],
        )
        .map_err(|error| map_write_error("optimistically update subscription detail", error))?;
    if changed == 0 {
        let actual = transaction
            .query_row(
                "SELECT revision
                   FROM wanted_subscription_records
                  WHERE account_key = ?1 AND subject_id = ?2",
                params![key.account_key.as_str(), key.subject_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| map_read_error("resolve optimistic detail update miss", error))?;
        return Err(match actual {
            None => RepositoryError::NotFound { key },
            Some(actual) => RepositoryError::RevisionConflict {
                key,
                expected: expected_revision,
                actual: checked_u64("revision", actual).and_then(|value| {
                    Revision::try_new(value)
                        .map_err(|error| persisted_validation("decode current revision", error))
                })?,
            },
        });
    }
    if changed != 1 {
        return Err(RepositoryError::Internal {
            message: format!(
                "optimistic detail update changed {changed} rows for a composite primary key"
            ),
        });
    }
    let detail = load_detail(&transaction, key)?;
    transaction
        .commit()
        .map_err(|error| map_write_error("commit optimistic detail update", error))?;
    Ok(UpdateSubscriptionDetailResult::new(detail))
}

fn list_summaries(
    connection: &Connection,
    command: ListSubscriptionsCommand,
) -> RepositoryResult<SubscriptionListPage> {
    if !(1..=100).contains(&command.limit) {
        return Err(RepositoryError::InvalidInput {
            field: "limit",
            message: "list limit must be between 1 and 100".to_string(),
        });
    }
    let limit = usize::try_from(command.limit).map_err(|_| RepositoryError::InvalidInput {
        field: "limit",
        message: "list limit does not fit this platform".to_string(),
    })?;
    let queries = build_list_queries(&command)?;
    let per_query_limit = limit.saturating_add(1);
    let mut items = Vec::with_capacity(per_query_limit.saturating_mul(queries.len()));
    for query in queries {
        read_list_segment(connection, query, &mut items)?;
    }
    // The frozen list index is partitioned by `active`, and NULL sort times form a second
    // ordered segment. Each SQL query is bounded to limit + 1 inside one index partition; this
    // bounded merge restores the contract's global NULLS LAST order without a database temp sort.
    items.sort_by(|left, right| {
        right
            .projection
            .douban_sort_time
            .cmp(&left.projection.douban_sort_time)
            .then_with(|| right.head.key.subject_id.cmp(&left.head.key.subject_id))
    });
    items.truncate(per_query_limit);
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let next_cursor = if has_more {
        let last = items
            .last()
            .expect("a positive validated list limit always retains one item");
        Some(
            ListCursor::try_new(
                last.projection.douban_sort_time,
                last.head.key.subject_id.clone(),
            )
            .map_err(|error| persisted_validation("build list continuation cursor", error))?,
        )
    } else {
        None
    };
    Ok(SubscriptionListPage { items, next_cursor })
}

fn read_list_segment(
    connection: &Connection,
    query: ListQuery,
    items: &mut Vec<SubscriptionSummary>,
) -> RepositoryResult<()> {
    let mut statement = connection
        .prepare(&query.sql)
        .map_err(|error| map_read_error("prepare subscription summary list segment", error))?;
    let rows = statement
        .query_map(params_from_iter(query.values.iter()), RawSummaryRow::read)
        .map_err(|error| map_read_error("query subscription summary list segment", error))?;
    for row in rows {
        let raw = row.map_err(|error| map_read_error("decode subscription summary row", error))?;
        items.push(raw.try_into_summary()?);
    }
    Ok(())
}

#[derive(Debug)]
struct ListQuery {
    sql: String,
    values: Vec<Value>,
}

#[derive(Debug, Clone, Copy)]
enum SortSegment<'a> {
    NonNull { cursor: Option<(i64, &'a str)> },
    Null { subject_before: Option<&'a str> },
}

fn build_list_queries(command: &ListSubscriptionsCommand) -> RepositoryResult<Vec<ListQuery>> {
    let cursor_sort_time = command
        .cursor
        .as_ref()
        .and_then(|cursor| cursor.douban_sort_time)
        .map(|sort_time| {
            i64::try_from(sort_time).map_err(|_| RepositoryError::InvalidInput {
                field: "cursor.douban_sort_time",
                message: "cursor sort time exceeds SQLite INTEGER range".to_string(),
            })
        })
        .transpose()?;
    let segments = match (&command.cursor, cursor_sort_time) {
        (None, None) => vec![
            SortSegment::NonNull { cursor: None },
            SortSegment::Null {
                subject_before: None,
            },
        ],
        (Some(cursor), Some(sort_time)) => vec![
            SortSegment::NonNull {
                cursor: Some((sort_time, cursor.subject_id.as_str())),
            },
            SortSegment::Null {
                subject_before: None,
            },
        ],
        (Some(cursor), None) => vec![SortSegment::Null {
            subject_before: Some(cursor.subject_id.as_str()),
        }],
        (None, Some(_)) => unreachable!("cursor sort time cannot exist without a cursor"),
    };
    let active_partitions = command
        .filter
        .active
        .map_or_else(|| vec![true, false], |active| vec![active]);
    let mut queries = Vec::with_capacity(active_partitions.len() * segments.len());
    for active in active_partitions {
        for segment in &segments {
            queries.push(build_list_segment(command, active, *segment));
        }
    }
    Ok(queries)
}

fn build_list_segment(
    command: &ListSubscriptionsCommand,
    active: bool,
    segment: SortSegment<'_>,
) -> ListQuery {
    let mut predicates = vec!["w.account_key = ?".to_string(), "w.active = ?".to_string()];
    let mut values = vec![
        Value::Text(command.account_key.clone()),
        Value::Integer(i64::from(active)),
    ];
    if let Some(media_kind) = command.filter.media_kind {
        predicates.push("w.media_kind = ?".to_string());
        values.push(Value::Text(media_kind.as_str().to_string()));
    }
    if let Some(lifecycle_state) = command.filter.lifecycle_state {
        predicates.push("w.lifecycle_state = ?".to_string());
        values.push(Value::Text(lifecycle_state.as_str().to_string()));
    }
    if let Some(attention_tag) = command.filter.attention_tag {
        predicates.push(
            "EXISTS (SELECT 1 FROM json_each(w.attention_tags_json) AS attention WHERE attention.value = ?)"
                .to_string(),
        );
        values.push(Value::Text(attention_tag_label(attention_tag).to_string()));
    }
    let order_by = match segment {
        SortSegment::NonNull { cursor } => {
            predicates.push("w.douban_sort_time IS NOT NULL".to_string());
            if let Some((sort_time, subject_id)) = cursor {
                predicates.push("(w.douban_sort_time, w.subject_id) < (?, ?)".to_string());
                values.push(Value::Integer(sort_time));
                values.push(Value::Text(subject_id.to_string()));
            }
            "w.douban_sort_time DESC, w.subject_id DESC"
        }
        SortSegment::Null { subject_before } => {
            predicates.push("w.douban_sort_time IS NULL".to_string());
            if let Some(subject_id) = subject_before {
                predicates.push("w.subject_id < ?".to_string());
                values.push(Value::Text(subject_id.to_string()));
            }
            "w.subject_id DESC"
        }
    };
    values.push(Value::Integer(i64::from(command.limit) + 1));
    let sql = format!(
        "SELECT {SUMMARY_COLUMNS}\n  FROM wanted_subscription_records AS w INDEXED BY wanted_records_list_v5_idx\n WHERE {}\n ORDER BY {order_by}\n LIMIT ?",
        predicates.join(" AND "),
    );
    ListQuery { sql, values }
}

#[derive(Debug)]
struct RawHeadRow {
    account_key: String,
    subject_id: String,
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
    updated_at: i64,
}

impl RawHeadRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            account_key: row.get(0)?,
            subject_id: row.get(1)?,
            revision: row.get(2)?,
            active: row.get(3)?,
            inactive_at: row.get(4)?,
            last_seen_snapshot_id: row.get(5)?,
            media_kind: row.get(6)?,
            schedulable: row.get(7)?,
            blocked_reason: row.get(8)?,
            lifecycle_state: row.get(9)?,
            execution_state: row.get(10)?,
            next_attempt_at: row.get(11)?,
            retry_count: row.get(12)?,
            max_retries: row.get(13)?,
            retry_blocked: row.get(14)?,
            force_eligible_once: row.get(15)?,
            updated_at: row.get(16)?,
        })
    }

    fn try_into_head(self) -> RepositoryResult<SubscriptionHead> {
        let key = SubscriptionKey::try_new(self.account_key, self.subject_id)
            .map_err(|error| persisted_validation("decode subscription key", error))?;
        let revision = checked_u64("revision", self.revision).and_then(|value| {
            Revision::try_new(value).map_err(|error| persisted_validation("decode revision", error))
        })?;
        let inactive_at = checked_optional_u64("inactive_at", self.inactive_at)?;
        let last_seen_snapshot_id = self
            .last_seen_snapshot_id
            .map(SnapshotId::try_new)
            .transpose()
            .map_err(|error| persisted_validation("decode last_seen_snapshot_id", error))?;
        let blocked_reason = self
            .blocked_reason
            .map(BlockedReason::try_new)
            .transpose()
            .map_err(|error| persisted_validation("decode blocked_reason", error))?;
        let head = SubscriptionHead {
            key,
            revision,
            active: checked_bool("active", self.active)?,
            inactive_at,
            last_seen_snapshot_id,
            media_kind: parse_media_kind(&self.media_kind)?,
            schedulable: checked_bool("schedulable", self.schedulable)?,
            blocked_reason,
            lifecycle_state: parse_lifecycle_state(&self.lifecycle_state)?,
            execution_state: parse_execution_state(&self.execution_state)?,
            next_attempt_at: checked_optional_u64("next_attempt_at", self.next_attempt_at)?,
            retry_count: checked_u32("retry_count", self.retry_count)?,
            max_retries: checked_u32("max_retries", self.max_retries)?,
            retry_blocked: checked_bool("retry_blocked", self.retry_blocked)?,
            force_eligible_once: checked_bool("force_eligible_once", self.force_eligible_once)?,
            updated_at: checked_u64("updated_at", self.updated_at)?,
        };
        head.validate()
            .map_err(|error| persisted_validation("validate subscription head", error))?;
        Ok(head)
    }
}

#[derive(Debug)]
struct RawSummaryRow {
    head: RawHeadRow,
    title: String,
    release_year: Option<i64>,
    poster_url: String,
    category_text: Option<String>,
    douban_sort_time: Option<i64>,
    attention_tags_json: String,
}

impl RawSummaryRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            head: RawHeadRow::read(row)?,
            title: row.get(17)?,
            release_year: row.get(18)?,
            poster_url: row.get(19)?,
            category_text: row.get(20)?,
            douban_sort_time: row.get(21)?,
            attention_tags_json: row.get(22)?,
        })
    }

    fn try_into_summary(self) -> RepositoryResult<SubscriptionSummary> {
        let title = checked_required_text("title", self.title)?;
        let poster_url = checked_text("poster_url", self.poster_url)?;
        let category_text = self
            .category_text
            .map(|value| checked_text("category_text", value))
            .transpose()?;
        let release_year = self
            .release_year
            .map(|value| {
                let value = checked_u64("release_year", value)?;
                if !(1..=9999).contains(&value) {
                    return Err(corrupt("release_year must be between 1 and 9999"));
                }
                u16::try_from(value)
                    .map_err(|_| corrupt("release_year cannot be represented as u16"))
            })
            .transpose()?;
        let summary = SubscriptionSummary {
            head: self.head.try_into_head()?,
            projection: SubscriptionProjection {
                title,
                release_year,
                poster_url,
                category_text,
                douban_sort_time: checked_optional_u64("douban_sort_time", self.douban_sort_time)?,
            },
            attention_tags: parse_attention_tags(&self.attention_tags_json)?,
        };
        summary
            .validate()
            .map_err(|error| persisted_validation("validate subscription summary", error))?;
        Ok(summary)
    }
}

#[derive(Debug)]
struct RawDetailRow {
    summary: RawSummaryRow,
    record_json: String,
}

impl RawDetailRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            summary: RawSummaryRow::read(row)?,
            record_json: row.get(23)?,
        })
    }

    fn try_into_detail(self) -> RepositoryResult<SubscriptionDetail> {
        let summary = self.summary.try_into_summary()?;
        let payload =
            serde_json::from_str::<SubscriptionPayload>(&self.record_json).map_err(|error| {
                RepositoryError::CorruptData {
                    message: format!("decode record_json payload: {error}"),
                }
            })?;
        SubscriptionDetail::try_new(summary, payload)
            .map_err(|error| persisted_validation("validate subscription detail", error))
    }
}

fn checked_bool(field: &str, value: i64) -> RepositoryResult<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(corrupt(format!("{field} must be stored as integer 0 or 1"))),
    }
}

fn checked_u64(field: &str, value: i64) -> RepositoryResult<u64> {
    u64::try_from(value).map_err(|_| corrupt(format!("{field} must not be negative")))
}

fn checked_u32(field: &str, value: i64) -> RepositoryResult<u32> {
    let value = checked_u64(field, value)?;
    u32::try_from(value).map_err(|_| corrupt(format!("{field} exceeds u32 range")))
}

fn checked_optional_u64(field: &str, value: Option<i64>) -> RepositoryResult<Option<u64>> {
    value.map(|value| checked_u64(field, value)).transpose()
}

fn command_integer(field: &'static str, value: u64) -> RepositoryResult<i64> {
    i64::try_from(value).map_err(|_| RepositoryError::InvalidInput {
        field,
        message: "value exceeds SQLite INTEGER range".to_string(),
    })
}

fn checked_required_text(field: &str, value: String) -> RepositoryResult<String> {
    if value.trim().is_empty() {
        return Err(corrupt(format!("{field} must not be blank")));
    }
    checked_text(field, value)
}

fn checked_text(field: &str, value: String) -> RepositoryResult<String> {
    if value.contains('\0') {
        return Err(corrupt(format!("{field} must not contain a NUL byte")));
    }
    Ok(value)
}

fn parse_media_kind(value: &str) -> RepositoryResult<SubscriptionMediaKind> {
    match value {
        "movie" => Ok(SubscriptionMediaKind::Movie),
        "tv" => Ok(SubscriptionMediaKind::Tv),
        _ => Err(corrupt(format!("unsupported media_kind {value:?}"))),
    }
}

fn parse_lifecycle_state(value: &str) -> RepositoryResult<SubscriptionLifecycleState> {
    match value {
        "queued" => Ok(SubscriptionLifecycleState::Queued),
        "meta" => Ok(SubscriptionLifecycleState::Meta),
        "searching" => Ok(SubscriptionLifecycleState::Searching),
        "downloading" => Ok(SubscriptionLifecycleState::Downloading),
        "linking" => Ok(SubscriptionLifecycleState::Linking),
        "completed" => Ok(SubscriptionLifecycleState::Completed),
        _ => Err(corrupt(format!("unsupported lifecycle_state {value:?}"))),
    }
}

fn parse_execution_state(value: &str) -> RepositoryResult<SubscriptionExecutionState> {
    match value {
        "idle" => Ok(SubscriptionExecutionState::Idle),
        "running" => Ok(SubscriptionExecutionState::Running),
        _ => Err(corrupt(format!("unsupported execution_state {value:?}"))),
    }
}

fn parse_attention_tags(value: &str) -> RepositoryResult<Vec<SubscriptionAttentionTag>> {
    let tags = serde_json::from_str::<Vec<SubscriptionAttentionTag>>(value).map_err(|error| {
        RepositoryError::CorruptData {
            message: format!("decode attention_tags_json: {error}"),
        }
    })?;
    let unique = tags.iter().copied().collect::<BTreeSet<_>>();
    if unique.len() != tags.len() {
        return Err(corrupt("attention_tags_json contains duplicate tags"));
    }
    Ok(tags)
}

const fn attention_tag_label(tag: SubscriptionAttentionTag) -> &'static str {
    match tag {
        SubscriptionAttentionTag::WaitingRelease => "waiting_release",
        SubscriptionAttentionTag::Failed => "failed",
        SubscriptionAttentionTag::RetryBlocked => "retry_blocked",
        SubscriptionAttentionTag::Skipped => "skipped",
        SubscriptionAttentionTag::NeedsReconciliation => "needs_reconciliation",
    }
}

fn persisted_validation(context: &str, error: RepositoryError) -> RepositoryError {
    RepositoryError::CorruptData {
        message: format!("{context}: {error}"),
    }
}

fn corrupt(message: impl Into<String>) -> RepositoryError {
    RepositoryError::CorruptData {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::ffi::OsString;
    use std::fs;
    use std::io::{Seek, SeekFrom, Write};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use rusqlite::{params, params_from_iter, Connection};

    use super::{
        build_list_queries,
        evidence_support::{insert_generated_clones, unrelated_storage_snapshot},
        DETAIL_SQL, GET_SQL, UPDATE_DETAIL_SQL,
    };
    use crate::storage::SqliteSubscriptionRepository;
    use crate::subscription::ports::{
        SubscriptionMutationRepository, SubscriptionPollRepository, SubscriptionReadRepository,
    };
    use crate::subscription::repository::{
        ApplyCompleteSnapshotCommand, BeginPollCommand, IssueOwnerPayload, IssuePayload,
        ListCursor, ListSubscriptionsCommand, NewRecordPolicy, RepositoryError, Revision,
        SnapshotRecord, SubscriptionDetail, SubscriptionKey, SubscriptionListFilter,
        TvDetailPayload, UpdateSubscriptionDetailCommand, WantedSourcePayload,
    };
    use crate::subscription::{
        SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
        SubscriptionMediaKind,
    };

    const ACCOUNT: &str = "fixture_rows_only";
    const BUSY_TIMEOUT: Duration = Duration::from_millis(250);
    const SEED_AT: u64 = 1_800_000_000;
    const INSERT_CLONE_SQL: &str = r#"
INSERT INTO wanted_subscription_records (
    account_key, subject_id, revision, active, inactive_at, last_seen_snapshot_id,
    media_kind, schedulable, blocked_reason, lifecycle_state, execution_state,
    next_attempt_at, retry_count, max_retries, retry_blocked, force_eligible_once,
    claimed_operation, attempt_id, lease_until, title, release_year, poster_url,
    category_text, douban_sort_time, attention_tags_json, updated_at, record_json
)
SELECT account_key, ?3, revision, active, inactive_at, last_seen_snapshot_id,
       media_kind, schedulable, blocked_reason, lifecycle_state, execution_state,
       next_attempt_at, retry_count, max_retries, retry_blocked, force_eligible_once,
       claimed_operation, attempt_id, lease_until, ?4, release_year, poster_url,
       category_text, ?5, attention_tags_json, updated_at, record_json
  FROM wanted_subscription_records
 WHERE account_key = ?1 AND subject_id = ?2
"#;

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        path: PathBuf,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum NamespaceEntry {
        File(Vec<u8>),
        Directory,
        Symlink(PathBuf),
        Other,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct NamespaceSnapshot(Vec<(OsString, NamespaceEntry)>);

    impl Fixture {
        fn new(label: &str) -> Self {
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "tmdb-mteam-v5-read-repo-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("create storage test directory");
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
        let repository =
            SqliteSubscriptionRepository::try_create_fresh(&fixture.path, 2, BUSY_TIMEOUT)
                .expect("create fresh latest-schema read repository");
        let token = repository
            .begin_poll(BeginPollCommand::try_new(ACCOUNT, SEED_AT).unwrap())
            .await
            .expect("begin fresh read fixture snapshot")
            .token;
        repository
            .apply_complete_snapshot(
                ApplyCompleteSnapshotCommand::try_new(
                    ACCOUNT,
                    token,
                    SEED_AT,
                    SEED_AT + 60,
                    NewRecordPolicy::try_new(3, false).unwrap(),
                    vec![
                        seed_movie("rows-movie-001", "Fixture Rows Queued Movie"),
                        seed_movie("rows-movie-002", "Fixture Rows Completed Movie"),
                    ],
                )
                .unwrap(),
            )
            .await
            .expect("seed fresh read fixture snapshot");
        let connection = Connection::open(&fixture.path).expect("open fresh read fixture seed");
        connection
            .execute(
                r#"UPDATE wanted_subscription_records
                      SET lifecycle_state = 'completed', next_attempt_at = NULL
                    WHERE account_key = ?1 AND subject_id = 'rows-movie-002'"#,
                [ACCOUNT],
            )
            .expect("seed completed fresh read row");
        connection
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
                          last_complete_snapshot_id = NULL
                    WHERE account_key = ?1"#,
                params![ACCOUNT, SEED_AT as i64],
            )
            .expect("reset fresh read fixture Poll metadata");
        fixture
    }

    fn seed_movie(subject_id: &str, title: &str) -> SnapshotRecord {
        SnapshotRecord::try_new(
            subject_id,
            SubscriptionMediaKind::Movie,
            true,
            None,
            WantedSourcePayload {
                title: title.to_string(),
                poster_url: format!("https://example.test/{subject_id}.jpg"),
                category_text: Some("fixture-movie".to_string()),
                douban_sort_time: None,
                tags: vec!["movie".to_string(), "fixture".to_string()],
                ..WantedSourcePayload::default()
            },
        )
        .expect("build fresh read movie snapshot")
    }

    fn make_repository(path: &Path) -> SqliteSubscriptionRepository {
        SqliteSubscriptionRepository::try_new(path, 2, BUSY_TIMEOUT)
            .expect("construct staged v5 repository")
    }

    fn namespace_snapshot(fixture: &Fixture) -> NamespaceSnapshot {
        let mut entries = fs::read_dir(&fixture.root)
            .expect("read fixture namespace")
            .map(|entry| {
                let entry = entry.expect("read fixture namespace entry");
                let file_name = entry.file_name();
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).expect("inspect namespace entry");
                let value = if metadata.file_type().is_file() {
                    NamespaceEntry::File(fs::read(&path).expect("read namespace file"))
                } else if metadata.file_type().is_dir() {
                    NamespaceEntry::Directory
                } else if metadata.file_type().is_symlink() {
                    NamespaceEntry::Symlink(
                        fs::read_link(&path).expect("read namespace symlink target"),
                    )
                } else {
                    NamespaceEntry::Other
                };
                (file_name, value)
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        NamespaceSnapshot(entries)
    }

    fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
        let mut value = path.as_os_str().to_os_string();
        value.push(suffix);
        PathBuf::from(value)
    }

    async fn convert_fixture_to_clean_wal_header(path: &Path) {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let connection = Connection::open(&path).expect("open fixture to enable WAL mode");
            let mode: String = connection
                .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
                .expect("enable WAL mode");
            assert_eq!(mode.to_ascii_lowercase(), "wal");
            let checkpoint: (i64, i64, i64) = connection
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
                .expect("checkpoint WAL fixture");
            assert_eq!(checkpoint.0, 0, "WAL checkpoint must not remain busy");
            connection.close().expect("close WAL fixture");
            for suffix in ["-wal", "-shm"] {
                let sidecar = sidecar_path(&path, suffix);
                match fs::remove_file(&sidecar) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => panic!("remove checkpointed WAL sidecar {sidecar:?}: {error}"),
                }
            }
            let header = fs::read(&path).expect("read WAL-header fixture");
            assert_eq!(
                header[18], 2,
                "fixture must retain WAL write-version header"
            );
            assert_eq!(header[19], 2, "fixture must retain WAL read-version header");
        })
        .await
        .expect("join WAL fixture conversion");
    }

    async fn execute_fixture_sql(path: &Path, sql: &'static str) {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let connection = Connection::open(path).expect("open fixture for schema mutation");
            connection
                .execute_batch(sql)
                .expect("apply fixture schema mutation");
            connection.close().expect("close mutated fixture");
        })
        .await
        .expect("join fixture schema mutation");
    }

    async fn corrupt_index_root_page(path: &Path, index: &'static str) {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let connection = Connection::open(&path).expect("open fixture to locate index page");
            let page_size: u64 = connection
                .pragma_query_value(None, "page_size", |row| row.get(0))
                .expect("read fixture page size");
            let root_page: u64 = connection
                .query_row(
                    "SELECT rootpage FROM sqlite_schema WHERE type = 'index' AND name = ?1",
                    [index],
                    |row| row.get(0),
                )
                .expect("read fixture index root page");
            connection.close().expect("close fixture before corruption");

            let offset = root_page
                .checked_sub(1)
                .and_then(|page| page.checked_mul(page_size))
                .expect("valid index page offset");
            let bytes = fs::read(&path).expect("read fixture before page corruption");
            let offset_index = usize::try_from(offset).expect("index page offset fits usize");
            assert_ne!(
                bytes[offset_index], 0,
                "index b-tree page type must be non-zero"
            );

            let mut file = fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .expect("open fixture to corrupt index page");
            file.seek(SeekFrom::Start(offset))
                .expect("seek to index root page");
            file.write_all(&[0])
                .expect("invalidate index b-tree page type");
            file.sync_all().expect("sync corrupted fixture");
        })
        .await
        .expect("join fixture page corruption");
    }

    fn key(subject_id: &str) -> SubscriptionKey {
        SubscriptionKey::try_new(ACCOUNT, subject_id).expect("valid fixture key")
    }

    fn list_command(
        filter: SubscriptionListFilter,
        cursor: Option<ListCursor>,
        limit: u32,
    ) -> ListSubscriptionsCommand {
        ListSubscriptionsCommand::try_new(ACCOUNT, filter, cursor, limit)
            .expect("valid fixture list command")
    }

    fn detail_update_command(
        detail: &SubscriptionDetail,
        title: &str,
        updated_at: u64,
        attention_tags: Vec<SubscriptionAttentionTag>,
    ) -> UpdateSubscriptionDetailCommand {
        let mut payload = detail.payload().clone();
        payload.source.title = title.to_string();
        payload.source.release_year = Some(2030);
        payload.source.poster_url = "https://example.test/updated-poster.jpg".to_string();
        payload.source.category_text = Some("updated-category".to_string());
        payload.source.douban_sort_time = Some(1_900_000_000);
        payload.observation.last_seen_at = updated_at;
        UpdateSubscriptionDetailCommand::try_new(
            detail.summary().head.key.clone(),
            detail.summary().head.revision,
            updated_at,
            attention_tags,
            payload,
        )
        .expect("build valid optimistic detail update")
    }

    async fn seed_running_attempt(
        repository: &SqliteSubscriptionRepository,
        attention_tags_json: &'static str,
        skip_reason: Option<&'static str>,
        force_eligible_once: bool,
    ) {
        repository
            .executor
            .run(move |connection| {
                let changed = connection
                    .execute(
                        r#"UPDATE wanted_subscription_records
                              SET revision = revision + 1,
                                  lifecycle_state = 'searching',
                                  execution_state = 'running',
                                  force_eligible_once = ?2,
                                  claimed_operation = 'movie_search',
                                  attempt_id = 'detail-cas-running-attempt',
                                  lease_until = 2000000000,
                                  attention_tags_json = ?3,
                                  record_json = CASE
                                      WHEN ?4 IS NULL
                                      THEN json_remove(record_json, '$.skip_reason')
                                      ELSE json_set(record_json, '$.skip_reason', ?4)
                                  END
                            WHERE account_key = ?1 AND subject_id = 'rows-movie-001'"#,
                        params![
                            ACCOUNT,
                            i64::from(force_eligible_once),
                            attention_tags_json,
                            skip_reason,
                        ],
                    )
                    .map_err(|error| super::map_write_error("seed running detail CAS", error))?;
                assert_eq!(changed, 1);
                Ok(())
            })
            .await
            .expect("seed running detail CAS fixture");
    }

    async fn isolation_snapshot(
        repository: &SqliteSubscriptionRepository,
    ) -> (String, String, String) {
        repository
            .executor
            .run(|connection| {
                let target_controls = connection
                    .query_row(
                        r#"SELECT json_array(
                                   active, inactive_at, last_seen_snapshot_id, media_kind,
                                   schedulable, blocked_reason, lifecycle_state, execution_state,
                                   next_attempt_at, retry_count, max_retries, retry_blocked,
                                   force_eligible_once, claimed_operation, attempt_id, lease_until
                               )
                              FROM wanted_subscription_records
                             WHERE account_key = ?1 AND subject_id = 'rows-movie-001'"#,
                        [ACCOUNT],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| super::map_read_error("snapshot target controls", error))?;
                let adjacent_row = connection
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
                             WHERE account_key = ?1 AND subject_id = 'rows-movie-002'"#,
                        [ACCOUNT],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| super::map_read_error("snapshot adjacent row", error))?;
                let account_meta = connection
                    .query_row(
                        r#"SELECT json_array(
                                   state_version, bootstrap_completed, created_at, updated_at,
                                   last_poll_attempt_at, last_poll_success_at, next_poll_at,
                                   poll_failure_count, last_poll_error, poll_generation,
                                   open_poll_generation, open_snapshot_id, last_incomplete_at,
                                   last_incomplete_reason, last_incomplete_fetched_pages,
                                   last_incomplete_truncated, last_incomplete_end_observed,
                                   last_complete_snapshot_id
                               )
                              FROM subscription_meta
                             WHERE account_key = ?1"#,
                        [ACCOUNT],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|error| super::map_read_error("snapshot account metadata", error))?;
                Ok((target_controls, adjacent_row, account_meta))
            })
            .await
            .expect("snapshot staged repository isolation boundaries")
    }

    #[tokio::test]
    async fn fresh_latest_rows_are_available_through_get_detail_and_list() {
        let fixture = fresh_fixture("basic").await;
        let repository = make_repository(&fixture.path);

        let pending_head = repository.get(key("rows-movie-001"));
        drop(repository);
        let head = pending_head.await.expect("read fresh head");
        assert_eq!(head.key.subject_id, "rows-movie-001");
        assert_eq!(head.media_kind, SubscriptionMediaKind::Movie);
        assert_eq!(head.lifecycle_state, SubscriptionLifecycleState::Queued);
        assert_eq!(head.retry_count, 0);
        assert_eq!(head.max_retries, 3);
        assert!(!head.retry_blocked);
        assert!(!head.force_eligible_once);

        let repository = make_repository(&fixture.path);
        let detail = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("read fresh detail");
        assert_eq!(
            detail.summary().projection.title,
            "Fixture Rows Queued Movie"
        );
        assert_eq!(detail.payload().source.title, "Fixture Rows Queued Movie");

        let page = repository
            .list_summaries(list_command(SubscriptionListFilter::default(), None, 100))
            .await
            .expect("list fresh summaries");
        assert_eq!(page.items.len(), 2);
        assert!(page.next_cursor.is_none());
    }

    #[tokio::test]
    async fn optimistic_detail_update_is_atomic_bounded_and_preserves_control_state() {
        let fixture = fresh_fixture("optimistic-update").await;
        let repository = make_repository(&fixture.path);
        let before = isolation_snapshot(&repository).await;
        let current = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load detail before optimistic update");
        let expected_revision = current.summary().head.revision;
        let command = detail_update_command(
            &current,
            "Optimistically Updated Title",
            1_900_000_000,
            vec![
                SubscriptionAttentionTag::Failed,
                SubscriptionAttentionTag::NeedsReconciliation,
            ],
        );

        let result = repository
            .update_detail(command)
            .await
            .expect("optimistically update one detail row");
        let detail = result.detail();
        assert_eq!(
            detail.summary().head.revision.value(),
            expected_revision.value() + 1
        );
        assert_eq!(detail.summary().head.updated_at, 1_900_000_000);
        assert_eq!(
            detail.summary().projection.title,
            "Optimistically Updated Title"
        );
        assert_eq!(detail.summary().projection.release_year, Some(2030));
        assert_eq!(
            detail.summary().projection.poster_url,
            "https://example.test/updated-poster.jpg"
        );
        assert_eq!(
            detail.summary().projection.category_text.as_deref(),
            Some("updated-category")
        );
        assert_eq!(
            detail.summary().projection.douban_sort_time,
            Some(1_900_000_000)
        );
        assert_eq!(
            detail.payload().source.title,
            detail.summary().projection.title
        );
        assert_eq!(
            detail.summary().attention_tags,
            [
                SubscriptionAttentionTag::Failed,
                SubscriptionAttentionTag::NeedsReconciliation,
            ]
        );

        let reloaded = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("reload committed optimistic detail update");
        assert_eq!(&reloaded, detail);
        let after = isolation_snapshot(&repository).await;
        assert_eq!(
            before, after,
            "detail CAS must not rewrite target controls, adjacent rows, or account metadata"
        );
    }

    #[tokio::test]
    async fn thousand_neighbor_detail_update_has_one_row_write_and_preserves_exact_bytes() {
        let fixture = fresh_fixture("detail-write-amplification").await;
        let mut connection =
            crate::storage::sqlite::open_v5_connection(&fixture.path, BUSY_TIMEOUT)
                .expect("open direct detail write-amplification connection");
        insert_generated_clones(
            &connection,
            ACCOUNT,
            "rows-movie-001",
            "detail-neighbor",
            1_000,
        );
        let record_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM wanted_subscription_records WHERE account_key = ?1",
                [ACCOUNT],
                |row| row.get(0),
            )
            .expect("count detail write-amplification records");
        assert_eq!(record_count, 1_002);

        let current = super::load_detail(&connection, key("rows-movie-001"))
            .expect("load direct detail update target");
        let target_payload_before: Vec<u8> = connection
            .query_row(
                r#"SELECT CAST(record_json AS BLOB)
                     FROM wanted_subscription_records
                    WHERE account_key = ?1 AND subject_id = ?2"#,
                params![ACCOUNT, "rows-movie-001"],
                |row| row.get(0),
            )
            .expect("read target payload bytes before detail update");
        let unrelated_before = unrelated_storage_snapshot(&connection, ACCOUNT, "rows-movie-001");
        let changes_before = connection.total_changes();

        let result = super::update_detail(
            &mut connection,
            detail_update_command(
                &current,
                "Constant Write Set",
                1_900_000_100,
                vec![SubscriptionAttentionTag::Failed],
            ),
        )
        .expect("update one detail among one thousand neighbors");

        assert_eq!(
            connection.total_changes() - changes_before,
            1,
            "detail CAS must update exactly one row independent of account cardinality"
        );
        assert_eq!(
            result.detail().summary().head.revision.value(),
            current.summary().head.revision.value() + 1
        );
        assert_eq!(
            unrelated_storage_snapshot(&connection, ACCOUNT, "rows-movie-001"),
            unrelated_before,
            "all unrelated revisions, scalar controls, JSON bytes, and account metadata must stay exact"
        );
        let target_payload_after: Vec<u8> = connection
            .query_row(
                r#"SELECT CAST(record_json AS BLOB)
                     FROM wanted_subscription_records
                    WHERE account_key = ?1 AND subject_id = ?2"#,
                params![ACCOUNT, "rows-movie-001"],
                |row| row.get(0),
            )
            .expect("read target payload bytes after detail update");
        assert_ne!(target_payload_after, target_payload_before);
    }

    #[tokio::test]
    async fn running_detail_cas_allows_non_gate_changes_tag_reordering_and_revision_freshness() {
        let fixture = fresh_fixture("running-detail-safe").await;
        let repository = make_repository(&fixture.path);
        seed_running_attempt(
            &repository,
            r#"["failed","needs_reconciliation"]"#,
            None,
            false,
        )
        .await;
        let current = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load running detail before safe CAS");
        let before_controls = isolation_snapshot(&repository).await;
        let updated_at = 1_900_000_010;
        let mut payload = current.payload().clone();
        payload.source.title = "Running Safe Detail Update".to_string();
        payload.source.poster_url = "https://example.test/running-safe.jpg".to_string();
        payload.observation.last_seen_at = updated_at;
        payload.issues.push(IssuePayload {
            owner: IssueOwnerPayload::Parent,
            operation: Some("detail_refresh".to_string()),
            error_type: None,
            message: "non-gate detail changed during execution".to_string(),
            occurred_at: Some(updated_at),
        });
        let command = UpdateSubscriptionDetailCommand::try_new(
            current.summary().head.key.clone(),
            current.summary().head.revision,
            updated_at,
            vec![
                SubscriptionAttentionTag::NeedsReconciliation,
                SubscriptionAttentionTag::Failed,
            ],
            payload,
        )
        .expect("build safe running detail command");

        let result = repository
            .update_detail(command)
            .await
            .expect("safe running detail CAS must succeed");
        assert_eq!(
            result.detail().summary().head.revision.value(),
            current.summary().head.revision.value() + 1
        );
        assert_eq!(
            result.detail().summary().head.execution_state,
            SubscriptionExecutionState::Running
        );
        assert_eq!(
            result.detail().payload().source.title,
            "Running Safe Detail Update"
        );
        assert_eq!(result.detail().payload().issues.len(), 1);
        assert_eq!(
            result.detail().summary().attention_tags,
            [
                SubscriptionAttentionTag::NeedsReconciliation,
                SubscriptionAttentionTag::Failed,
            ]
        );
        assert_eq!(
            isolation_snapshot(&repository).await,
            before_controls,
            "safe running CAS must preserve attempt, lease, force, and every adjacent scope"
        );
    }

    #[tokio::test]
    async fn running_detail_cas_rejects_attention_tag_add_and_remove_without_writes() {
        let fixture = fresh_fixture("running-detail-tags").await;
        let repository = make_repository(&fixture.path);
        seed_running_attempt(&repository, r#"["failed"]"#, None, false).await;
        let current = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load running tag fixture");

        for requested_tags in [
            vec![
                SubscriptionAttentionTag::Failed,
                SubscriptionAttentionTag::NeedsReconciliation,
            ],
            Vec::new(),
        ] {
            let command = UpdateSubscriptionDetailCommand::try_new(
                current.summary().head.key.clone(),
                current.summary().head.revision,
                1_900_000_011,
                requested_tags,
                current.payload().clone(),
            )
            .expect("build running tag mutation");
            let error = repository
                .update_detail(command)
                .await
                .expect_err("running attention tag mutation must conflict");
            assert_eq!(
                error,
                RepositoryError::ExecutionGateConflict {
                    key: key("rows-movie-001"),
                }
            );
        }

        assert_eq!(
            repository
                .load_detail(key("rows-movie-001"))
                .await
                .expect("reload rejected tag mutations"),
            current,
            "tag conflicts must leave the entire row unchanged"
        );
    }

    #[tokio::test]
    async fn running_detail_cas_rejects_skip_reason_add_remove_and_text_change() {
        let fixture = fresh_fixture("running-detail-skip").await;
        let repository = make_repository(&fixture.path);
        seed_running_attempt(&repository, "[]", None, false).await;
        let unskipped = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load unskipped running detail");
        let mut added_skip = unskipped.payload().clone();
        added_skip.skip_reason = Some("manual_skip".to_string());
        let add_error = repository
            .update_detail(
                UpdateSubscriptionDetailCommand::try_new(
                    unskipped.summary().head.key.clone(),
                    unskipped.summary().head.revision,
                    1_900_000_012,
                    Vec::new(),
                    added_skip,
                )
                .unwrap(),
            )
            .await
            .expect_err("running skip reason add must conflict");
        assert!(matches!(
            add_error,
            RepositoryError::ExecutionGateConflict { .. }
        ));
        assert_eq!(
            repository.load_detail(key("rows-movie-001")).await.unwrap(),
            unskipped
        );

        let forced_fixture = fresh_fixture("running-detail-forced-skip").await;
        let forced_repository = make_repository(&forced_fixture.path);
        seed_running_attempt(
            &forced_repository,
            r#"["skipped"]"#,
            Some("manual_skip"),
            true,
        )
        .await;
        let forced = forced_repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load forced skipped running detail");
        for requested_reason in [None, Some("changed_skip_reason")] {
            let mut payload = forced.payload().clone();
            payload.skip_reason = requested_reason.map(ToString::to_string);
            let error = forced_repository
                .update_detail(
                    UpdateSubscriptionDetailCommand::try_new(
                        forced.summary().head.key.clone(),
                        forced.summary().head.revision,
                        1_900_000_013,
                        vec![SubscriptionAttentionTag::Skipped],
                        payload,
                    )
                    .unwrap(),
                )
                .await
                .expect_err("running skip reason mutation must conflict");
            assert!(matches!(
                error,
                RepositoryError::ExecutionGateConflict { .. }
            ));
        }

        let before_controls = isolation_snapshot(&forced_repository).await;
        let safe = detail_update_command(
            &forced,
            "Forced Skipped Safe Refresh",
            1_900_000_014,
            vec![SubscriptionAttentionTag::Skipped],
        );
        let safe_result = forced_repository
            .update_detail(safe)
            .await
            .expect("forced skipped source refresh must preserve its running gate");
        assert!(safe_result.detail().summary().head.force_eligible_once);
        assert_eq!(
            safe_result.detail().payload().skip_reason.as_deref(),
            Some("manual_skip")
        );
        assert_eq!(
            isolation_snapshot(&forced_repository).await,
            before_controls,
            "forced skipped refresh must preserve attempt controls and force"
        );
    }

    #[tokio::test]
    async fn stale_revision_wins_over_running_gate_conflict() {
        let fixture = fresh_fixture("running-detail-stale-first").await;
        let repository = make_repository(&fixture.path);
        seed_running_attempt(&repository, r#"["failed"]"#, None, false).await;
        let stale = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load soon-stale running detail");
        repository
            .executor
            .run(|connection| {
                connection
                    .execute(
                        "UPDATE wanted_subscription_records
                            SET revision = revision + 1
                          WHERE account_key = ?1 AND subject_id = 'rows-movie-001'",
                        [ACCOUNT],
                    )
                    .map_err(|error| super::map_write_error("advance running revision", error))?;
                Ok(())
            })
            .await
            .expect("advance running detail revision");

        let command = UpdateSubscriptionDetailCommand::try_new(
            stale.summary().head.key.clone(),
            stale.summary().head.revision,
            1_900_000_015,
            Vec::new(),
            stale.payload().clone(),
        )
        .expect("build stale gate-changing command");
        let error = repository
            .update_detail(command)
            .await
            .expect_err("stale revision must win over gate comparison");
        assert_eq!(
            error,
            RepositoryError::RevisionConflict {
                key: key("rows-movie-001"),
                expected: stale.summary().head.revision,
                actual: Revision::try_new(stale.summary().head.revision.value() + 1).unwrap(),
            }
        );
    }

    #[tokio::test]
    async fn idle_detail_cas_keeps_compatible_gate_mutation_behavior() {
        let fixture = fresh_fixture("idle-detail-gates").await;
        let repository = make_repository(&fixture.path);
        let current = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load idle detail gate fixture");
        let mut payload = current.payload().clone();
        payload.skip_reason = Some("manual_skip".to_string());
        let result = repository
            .update_detail(
                UpdateSubscriptionDetailCommand::try_new(
                    current.summary().head.key.clone(),
                    current.summary().head.revision,
                    1_900_000_016,
                    vec![SubscriptionAttentionTag::Skipped],
                    payload,
                )
                .unwrap(),
            )
            .await
            .expect("idle detail gate mutation remains compatible");
        assert_eq!(
            result.detail().summary().head.execution_state,
            SubscriptionExecutionState::Idle
        );
        assert_eq!(
            result.detail().summary().attention_tags,
            [SubscriptionAttentionTag::Skipped]
        );
        assert_eq!(
            result.detail().payload().skip_reason.as_deref(),
            Some("manual_skip")
        );
    }

    #[tokio::test]
    async fn stale_and_missing_optimistic_updates_are_typed_and_leave_data_unchanged() {
        let fixture = fresh_fixture("optimistic-conflicts").await;
        let repository = make_repository(&fixture.path);
        let current = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load initial detail");
        let expected = current.summary().head.revision;
        let command = detail_update_command(
            &current,
            "First Winning Update",
            1_900_000_001,
            vec![SubscriptionAttentionTag::Failed],
        );
        repository
            .update_detail(command.clone())
            .await
            .expect("first compare-and-swap wins");

        let stale = repository
            .update_detail(command)
            .await
            .expect_err("reusing the consumed revision must conflict");
        assert_eq!(
            stale,
            RepositoryError::RevisionConflict {
                key: key("rows-movie-001"),
                expected,
                actual: Revision::try_new(expected.value() + 1).unwrap(),
            }
        );
        let after_stale = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load detail after stale update");
        assert_eq!(after_stale.payload().source.title, "First Winning Update");
        assert_eq!(
            after_stale.summary().head.revision.value(),
            expected.value() + 1
        );

        let mut missing_payload = after_stale.payload().clone();
        missing_payload.source.title = "Missing Row".to_string();
        let missing_key = key("missing-subject");
        let missing_command = UpdateSubscriptionDetailCommand::try_new(
            missing_key.clone(),
            after_stale.summary().head.revision,
            1_900_000_002,
            Vec::new(),
            missing_payload,
        )
        .expect("build valid missing-row update");
        let missing = repository
            .update_detail(missing_command)
            .await
            .expect_err("missing optimistic target must remain distinct from conflict");
        assert_eq!(missing, RepositoryError::NotFound { key: missing_key });
    }

    #[tokio::test]
    async fn concurrent_compare_and_swap_allows_exactly_one_winner() {
        let fixture = fresh_fixture("optimistic-concurrent").await;
        let first_repository = make_repository(&fixture.path);
        let second_repository = make_repository(&fixture.path);
        let current = first_repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load shared revision");
        let first = detail_update_command(
            &current,
            "Concurrent First",
            1_900_000_003,
            vec![SubscriptionAttentionTag::Failed],
        );
        let second = detail_update_command(
            &current,
            "Concurrent Second",
            1_900_000_004,
            vec![SubscriptionAttentionTag::NeedsReconciliation],
        );

        let (first_result, second_result) = tokio::join!(
            first_repository.update_detail(first),
            second_repository.update_detail(second)
        );
        let results = [first_result, second_result];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(RepositoryError::RevisionConflict { .. })))
                .count(),
            1
        );
        let committed = make_repository(&fixture.path)
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load concurrent winner");
        assert_eq!(
            committed.summary().head.revision.value(),
            current.summary().head.revision.value() + 1
        );
        assert!(matches!(
            committed.payload().source.title.as_str(),
            "Concurrent First" | "Concurrent Second"
        ));
    }

    #[tokio::test]
    async fn invalid_detail_updates_fail_before_sql_execution() {
        let fixture = fresh_fixture("optimistic-validation").await;
        let repository = make_repository(&fixture.path);
        let current = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("load validation fixture");
        let original_revision = current.summary().head.revision;

        let duplicate_tags = UpdateSubscriptionDetailCommand::try_new(
            current.summary().head.key.clone(),
            original_revision,
            1_900_000_005,
            vec![
                SubscriptionAttentionTag::Failed,
                SubscriptionAttentionTag::Failed,
            ],
            current.payload().clone(),
        )
        .expect_err("duplicate attention tags must be rejected by the command contract");
        assert!(matches!(
            duplicate_tags,
            RepositoryError::InvalidInput {
                field: "attention_tags",
                ..
            }
        ));

        let mut invalid_payload = current.payload().clone();
        invalid_payload.source.poster_url = "invalid\0poster".to_string();
        let invalid_projection = UpdateSubscriptionDetailCommand::try_new(
            current.summary().head.key.clone(),
            original_revision,
            1_900_000_005,
            Vec::new(),
            invalid_payload,
        )
        .expect_err("projection text that cannot round-trip must be rejected");
        assert!(matches!(
            invalid_projection,
            RepositoryError::InvalidInput {
                field: "source.poster_url",
                ..
            }
        ));

        let stale_timestamp = UpdateSubscriptionDetailCommand::try_new(
            current.summary().head.key.clone(),
            original_revision,
            current.payload().observation.last_seen_at - 1,
            Vec::new(),
            current.payload().clone(),
        )
        .expect_err("updated_at must not precede the payload observation");
        assert!(matches!(
            stale_timestamp,
            RepositoryError::InvalidInput {
                field: "updated_at",
                ..
            }
        ));

        let mut movie_with_tv_payload = current.payload().clone();
        movie_with_tv_payload.tv = Some(TvDetailPayload {
            season_number: 1,
            episode_total: 1,
            target_start_episode: 1,
            target_end_episode: 1,
            episodes: Vec::new(),
        });
        let cross_kind_command = UpdateSubscriptionDetailCommand::try_new(
            current.summary().head.key.clone(),
            original_revision,
            1_900_000_006,
            Vec::new(),
            movie_with_tv_payload,
        )
        .expect("payload shape is validated against persisted media kind inside the transaction");
        let cross_kind_error = repository
            .update_detail(cross_kind_command)
            .await
            .expect_err("movie rows must reject TV detail without committing");
        assert!(matches!(
            cross_kind_error,
            RepositoryError::InvalidInput {
                field: "payload.tv",
                ..
            }
        ));

        let unchanged = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect("invalid commands must not change the row");
        assert_eq!(unchanged.summary().head.revision, original_revision);
    }

    #[tokio::test]
    async fn every_runtime_connection_enables_foreign_keys() {
        let fixture = fresh_fixture("foreign-keys").await;
        let repository = make_repository(&fixture.path);
        let (foreign_keys, insert_error) = repository
            .executor
            .run(|connection| {
                let foreign_keys: i64 = connection
                    .pragma_query_value(None, "foreign_keys", |row| row.get(0))
                    .map_err(|error| super::map_read_error("read test foreign_keys", error))?;
                let insert_error = connection
                    .execute(
                        r#"INSERT INTO wanted_subscription_records (
                               account_key, subject_id, revision, active, inactive_at,
                               last_seen_snapshot_id, media_kind, schedulable, blocked_reason,
                               lifecycle_state, execution_state, next_attempt_at, retry_count,
                               max_retries, retry_blocked, force_eligible_once, claimed_operation,
                               attempt_id, lease_until, title, release_year, poster_url,
                               category_text, douban_sort_time, attention_tags_json, updated_at,
                               record_json
                           )
                           SELECT 'missing-parent', 'orphan', revision, active, inactive_at,
                                  last_seen_snapshot_id, media_kind, schedulable, blocked_reason,
                                  lifecycle_state, execution_state, next_attempt_at, retry_count,
                                  max_retries, retry_blocked, force_eligible_once,
                                  claimed_operation, attempt_id, lease_until, title, release_year,
                                  poster_url, category_text, douban_sort_time,
                                  attention_tags_json, updated_at, record_json
                             FROM wanted_subscription_records
                            LIMIT 1"#,
                        [],
                    )
                    .expect_err("orphan row must violate the foreign key");
                Ok((foreign_keys, insert_error.to_string()))
            })
            .await
            .expect("inspect fresh repository connection");
        assert_eq!(foreign_keys, 1);
        assert!(insert_error.contains("FOREIGN KEY constraint failed"));
    }

    #[tokio::test]
    async fn existing_open_never_creates_missing_paths_and_fresh_create_never_clobbers() {
        let missing = Fixture::new("missing");
        assert!(!missing.path.exists());
        let missing_error = make_repository(&missing.path)
            .get(key("rows-movie-001"))
            .await
            .expect_err("missing database must be unavailable");
        assert!(matches!(missing_error, RepositoryError::Unavailable { .. }));
        assert!(
            !missing.path.exists(),
            "runtime adapter must not create a database"
        );

        let old_path = missing.root.join("wanted.sqlite");
        fs::write(&old_path, b"old database must remain ignored").unwrap();
        SqliteSubscriptionRepository::try_create_fresh(&missing.path, 2, BUSY_TIMEOUT)
            .expect("create independent fresh subscriptions.sqlite");
        let latest_before = fs::read(&missing.path).expect("read fresh latest database");
        let duplicate =
            SqliteSubscriptionRepository::try_create_fresh(&missing.path, 2, BUSY_TIMEOUT)
                .expect_err("fresh create must fail when subscriptions.sqlite already exists");
        assert!(matches!(duplicate, RepositoryError::Unavailable { .. }));
        assert_eq!(fs::read(&missing.path).unwrap(), latest_before);
        assert_eq!(
            fs::read(&old_path).unwrap(),
            b"old database must remain ignored"
        );
    }

    #[tokio::test]
    async fn preflight_is_eager_and_zero_write_for_valid_missing_unsupported_and_future_databases()
    {
        let valid = fresh_fixture("preflight-valid").await;
        let valid_before = namespace_snapshot(&valid);
        make_repository(&valid.path)
            .preflight()
            .await
            .expect("valid schema-v5 database must pass preflight");
        assert_eq!(
            namespace_snapshot(&valid),
            valid_before,
            "successful preflight must not change the main database or create a sidecar"
        );

        let missing = Fixture::new("preflight-missing");
        let missing_before = namespace_snapshot(&missing);
        let missing_error = make_repository(&missing.path)
            .preflight()
            .await
            .expect_err("preflight must reject a missing database");
        assert!(matches!(missing_error, RepositoryError::Unavailable { .. }));
        assert_eq!(
            namespace_snapshot(&missing),
            missing_before,
            "preflight must not create a missing database or sidecar"
        );

        let unsupported = fresh_fixture("preflight-unsupported").await;
        execute_fixture_sql(
            &unsupported.path,
            "UPDATE subscription_schema_meta SET value = 4 WHERE key = 'schema_version'",
        )
        .await;
        let unsupported_before = namespace_snapshot(&unsupported);
        let unsupported_error = make_repository(&unsupported.path)
            .preflight()
            .await
            .expect_err("preflight must reject any non-latest schema marker");
        assert_eq!(
            unsupported_error,
            RepositoryError::UnsupportedSchema {
                found: 4,
                maximum_supported: 5,
            }
        );
        assert_eq!(
            namespace_snapshot(&unsupported),
            unsupported_before,
            "unsupported-schema rejection must not change the SQLite namespace"
        );

        let future = fresh_fixture("preflight-future").await;
        execute_fixture_sql(
            &future.path,
            "UPDATE subscription_schema_meta SET value = 6 WHERE key = 'schema_version'",
        )
        .await;
        let future_before = namespace_snapshot(&future);
        let future_error = make_repository(&future.path)
            .preflight()
            .await
            .expect_err("preflight must reject a future schema");
        assert_eq!(
            future_error,
            RepositoryError::UnsupportedSchema {
                found: 6,
                maximum_supported: 5,
            }
        );
        assert_eq!(
            namespace_snapshot(&future),
            future_before,
            "future-schema rejection must not change the SQLite namespace"
        );
    }

    #[tokio::test]
    async fn preflight_rejects_existing_journal_wal_and_shm_entries_before_sqlite_open() {
        for suffix in ["-journal", "-wal", "-shm"] {
            let fixture = fresh_fixture(&format!(
                "preflight-sidecar-{}",
                suffix.trim_start_matches('-')
            ))
            .await;
            let sidecar = sidecar_path(&fixture.path, suffix);
            fs::write(&sidecar, format!("sentinel sidecar {suffix}"))
                .expect("create pre-existing SQLite sidecar sentinel");
            let before = namespace_snapshot(&fixture);

            let error = make_repository(&fixture.path)
                .preflight()
                .await
                .expect_err("preflight must reject every existing SQLite sidecar");
            let RepositoryError::Unavailable { message } = error else {
                panic!("existing sidecar must be unavailable: {error}");
            };
            assert!(
                message.contains(suffix),
                "unexpected sidecar error: {message}"
            );
            assert_eq!(
                namespace_snapshot(&fixture),
                before,
                "sidecar rejection must not create, modify, or delete any namespace entry"
            );
        }
    }

    #[tokio::test]
    async fn preflight_rejects_clean_wal_header_without_creating_sidecars() {
        let fixture = fresh_fixture("preflight-clean-wal-header").await;
        convert_fixture_to_clean_wal_header(&fixture.path).await;
        let before = namespace_snapshot(&fixture);

        let error = make_repository(&fixture.path)
            .preflight()
            .await
            .expect_err("preflight must reject persistent WAL mode before SQLite open");
        let RepositoryError::Unavailable { message } = error else {
            panic!("WAL header must be unavailable: {error}");
        };
        assert!(
            message.contains("WAL-format"),
            "unexpected WAL error: {message}"
        );
        assert_eq!(
            namespace_snapshot(&fixture),
            before,
            "WAL-header rejection must not create -wal/-shm or modify the main database"
        );
    }

    #[tokio::test]
    async fn preflight_requires_complete_runtime_tables_and_indexes_without_repairing_them() {
        for (label, mutation, missing_object) in [
            (
                "missing-runtime-table",
                "DROP TABLE operation_logs",
                "operation_logs",
            ),
            (
                "missing-runtime-index",
                "DROP INDEX wanted_records_list_v5_idx",
                "wanted_records_list_v5_idx",
            ),
            (
                "missing-expired-lease-index",
                "DROP INDEX wanted_records_expired_lease_v5_idx",
                "wanted_records_expired_lease_v5_idx",
            ),
            (
                "missing-force-index",
                "DROP INDEX wanted_records_force_v5_idx",
                "wanted_records_force_v5_idx",
            ),
        ] {
            let fixture = fresh_fixture(label).await;
            execute_fixture_sql(&fixture.path, mutation).await;
            let before = namespace_snapshot(&fixture);

            let error = make_repository(&fixture.path)
                .preflight()
                .await
                .expect_err("preflight must reject an incomplete schema-v5 database");
            let RepositoryError::CorruptData { message } = error else {
                panic!("incomplete schema must be corrupt data: {error}");
            };
            assert!(
                message.contains(missing_object),
                "unexpected error: {message}"
            );
            assert_eq!(
                namespace_snapshot(&fixture),
                before,
                "preflight must not repair or otherwise change an incomplete schema namespace"
            );
        }
    }

    #[tokio::test]
    async fn preflight_rejects_same_name_schema_shells_and_column_drift_without_writes() {
        for (label, mutation, expected_message) in [
            (
                "wrong-same-name-index",
                r#"
                DROP INDEX wanted_records_list_v5_idx;
                CREATE INDEX wanted_records_list_v5_idx
                    ON wanted_subscription_records (account_key, subject_id);
                "#,
                "wanted_records_list_v5_idx",
            ),
            (
                "wrong-force-index-predicate",
                r#"
                DROP INDEX wanted_records_force_v5_idx;
                CREATE INDEX wanted_records_force_v5_idx
                    ON wanted_subscription_records
                       (account_key, next_attempt_at, updated_at, subject_id);
                "#,
                "wanted_records_force_v5_idx",
            ),
            (
                "forbidden-legacy-table",
                r#"
                CREATE TABLE subscription_state_blobs_legacy_v4 (
                    account_key TEXT NOT NULL,
                    state_json TEXT NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                "#,
                "subscription_state_blobs_legacy_v4",
            ),
            (
                "unexpected-runtime-column",
                "ALTER TABLE wanted_subscription_records ADD COLUMN unexpected_runtime_column TEXT;",
                "column count",
            ),
            (
                "wrong-marker-column-type",
                r#"
                ALTER TABLE subscription_schema_meta RENAME TO subscription_schema_meta_old;
                CREATE TABLE subscription_schema_meta (
                    key BLOB NOT NULL,
                    value INTEGER NOT NULL
                );
                INSERT INTO subscription_schema_meta (key, value)
                    SELECT key, value FROM subscription_schema_meta_old;
                DROP TABLE subscription_schema_meta_old;
                "#,
                "column 0 differs",
            ),
            (
                "obsolete-poll-timestamp-check",
                r#"
                PRAGMA writable_schema = ON;
                UPDATE sqlite_schema
                   SET sql = replace(
                       sql,
                       'poll_failure_count = 0 AND last_poll_success_at >= last_poll_attempt_at',
                       'poll_failure_count = 0 AND last_poll_success_at <= last_poll_attempt_at'
                   )
                 WHERE type = 'table' AND name = 'subscription_meta';
                PRAGMA writable_schema = OFF;
                PRAGMA schema_version = 12345;
                "#,
                "required canonical SQL fragment",
            ),
            (
                "weakened-execution-attempt-byte-check",
                r#"
                PRAGMA writable_schema = ON;
                UPDATE sqlite_schema
                   SET sql = replace(
                       sql,
                       'length(CAST(attempt_id AS BLOB)) <= 256',
                       'length(CAST(attempt_id AS BLOB)) <= 512'
                   )
                 WHERE type = 'table' AND name = 'wanted_subscription_records';
                PRAGMA writable_schema = OFF;
                PRAGMA schema_version = 12346;
                "#,
                "required canonical SQL fragment",
            ),
            (
                "weakened-running-safety-check",
                r#"
                PRAGMA writable_schema = ON;
                UPDATE sqlite_schema
                   SET sql = replace(
                       sql,
                       'AND schedulable = 1
            AND media_kind = ''movie''
            AND lifecycle_state != ''completed''',
                       'AND schedulable IN (0, 1)
            AND media_kind = ''movie''
            AND lifecycle_state != ''completed'''
                   )
                 WHERE type = 'table' AND name = 'wanted_subscription_records';
                PRAGMA writable_schema = OFF;
                PRAGMA schema_version = 12347;
                "#,
                "required canonical SQL fragment",
            ),
        ] {
            let fixture = fresh_fixture(label).await;
            execute_fixture_sql(&fixture.path, mutation).await;
            let before = namespace_snapshot(&fixture);

            let error = make_repository(&fixture.path)
                .preflight()
                .await
                .expect_err("preflight must reject a same-name schema shell or column drift");
            let RepositoryError::CorruptData { message } = error else {
                panic!("schema contract drift must be corrupt data: {error}");
            };
            assert!(
                message.contains(expected_message),
                "unexpected schema-contract error for {label}: {message}"
            );
            assert_eq!(
                namespace_snapshot(&fixture),
                before,
                "schema-contract rejection must not change any SQLite namespace entry"
            );
        }
    }

    #[tokio::test]
    async fn preflight_rejects_foreign_key_violations_without_changing_the_database() {
        let fixture = fresh_fixture("preflight-foreign-key").await;
        execute_fixture_sql(
            &fixture.path,
            r#"
            PRAGMA foreign_keys = OFF;
            INSERT INTO wanted_subscription_records (
                account_key, subject_id, revision, active, inactive_at, last_seen_snapshot_id,
                media_kind, schedulable, blocked_reason, lifecycle_state, execution_state,
                next_attempt_at, retry_count, max_retries, retry_blocked, force_eligible_once,
                claimed_operation, attempt_id, lease_until, title, release_year, poster_url,
                category_text, douban_sort_time, attention_tags_json, updated_at, record_json
            )
            SELECT 'missing-parent', 'orphan', revision, active, inactive_at, last_seen_snapshot_id,
                   media_kind, schedulable, blocked_reason, lifecycle_state, execution_state,
                   next_attempt_at, retry_count, max_retries, retry_blocked, force_eligible_once,
                   claimed_operation, attempt_id, lease_until, title, release_year, poster_url,
                   category_text, douban_sort_time, attention_tags_json, updated_at, record_json
              FROM wanted_subscription_records
             LIMIT 1;
            "#,
        )
        .await;
        let before = namespace_snapshot(&fixture);

        let error = make_repository(&fixture.path)
            .preflight()
            .await
            .expect_err("preflight must reject foreign-key violations");
        let RepositoryError::CorruptData { message } = error else {
            panic!("foreign-key violation must be corrupt data: {error}");
        };
        assert!(message.contains("foreign_key_check"));
        assert_eq!(
            namespace_snapshot(&fixture),
            before,
            "foreign-key preflight failure must not change the SQLite namespace"
        );
    }

    #[tokio::test]
    async fn preflight_runs_integrity_check_and_leaves_a_corrupt_database_untouched() {
        let fixture = fresh_fixture("preflight-integrity-check").await;
        corrupt_index_root_page(&fixture.path, "wanted_records_list_v5_idx").await;
        let before = namespace_snapshot(&fixture);

        let error = make_repository(&fixture.path)
            .preflight()
            .await
            .expect_err("preflight must reject physical SQLite corruption");
        assert!(matches!(error, RepositoryError::CorruptData { .. }));
        assert_eq!(
            namespace_snapshot(&fixture),
            before,
            "integrity-check failure must not change the corrupt SQLite namespace"
        );
    }

    #[tokio::test]
    async fn missing_subscription_key_returns_not_found() {
        let fixture = fresh_fixture("not-found").await;
        let repository = make_repository(&fixture.path);
        let missing_key = key("missing-subject");
        let error = repository
            .get(missing_key.clone())
            .await
            .expect_err("missing subscription must return NotFound");
        assert_eq!(
            error,
            RepositoryError::NotFound {
                key: missing_key.clone(),
            }
        );
        let detail_error = repository
            .load_detail(missing_key.clone())
            .await
            .expect_err("missing detail must return NotFound");
        assert_eq!(detail_error, RepositoryError::NotFound { key: missing_key });
    }

    #[tokio::test]
    async fn list_ignores_record_json_but_detail_rejects_projection_drift() {
        let fixture = fresh_fixture("projection-drift").await;
        let repository = make_repository(&fixture.path);
        repository
            .executor
            .run(|connection| {
                connection
                    .execute(
                        "UPDATE wanted_subscription_records
                            SET record_json = json_set(record_json, '$.source.title', 'Drifted Payload')
                          WHERE account_key = ?1 AND subject_id = ?2",
                        params![ACCOUNT, "rows-movie-001"],
                    )
                    .map_err(|error| super::map_read_error("inject projection drift", error))?;
                Ok(())
            })
            .await
            .expect("inject valid JSON projection drift");

        let page = repository
            .list_summaries(list_command(SubscriptionListFilter::default(), None, 100))
            .await
            .expect("list must not parse record_json");
        assert_eq!(page.items.len(), 2);
        let error = repository
            .load_detail(key("rows-movie-001"))
            .await
            .expect_err("detail must detect projection drift");
        assert!(matches!(error, RepositoryError::CorruptData { .. }));
    }

    #[tokio::test]
    async fn keyset_pagination_crosses_non_null_and_null_sort_times_without_duplicates() {
        let fixture = fresh_fixture("pagination").await;
        let repository = make_repository(&fixture.path);
        repository
            .executor
            .run(|connection| {
                connection
                    .execute(
                        "UPDATE wanted_subscription_records
                            SET douban_sort_time = CASE subject_id
                                WHEN 'rows-movie-001' THEN 300
                                WHEN 'rows-movie-002' THEN 200
                            END
                          WHERE account_key = ?1",
                        [ACCOUNT],
                    )
                    .map_err(|error| super::map_read_error("seed sort times", error))?;
                for (subject_id, title, sort_time) in [
                    ("rows-sort-150", "Sort 150", Some(150_i64)),
                    ("rows-null-z", "Null Z", None),
                    ("rows-null-a", "Null A", None),
                ] {
                    connection
                        .execute(
                            INSERT_CLONE_SQL,
                            params![ACCOUNT, "rows-movie-001", subject_id, title, sort_time],
                        )
                        .map_err(|error| super::map_read_error("insert pagination row", error))?;
                }
                Ok(())
            })
            .await
            .expect("seed pagination fixture");

        let mut cursor = None;
        let mut subjects = Vec::new();
        let mut cursor_kinds = Vec::new();
        loop {
            let page = repository
                .list_summaries(list_command(SubscriptionListFilter::default(), cursor, 2))
                .await
                .expect("read keyset page");
            subjects.extend(
                page.items
                    .iter()
                    .map(|summary| summary.head.key.subject_id.clone()),
            );
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor_kinds.push(next.douban_sort_time.is_some());
            cursor = Some(next);
        }
        assert_eq!(
            subjects,
            [
                "rows-movie-001",
                "rows-movie-002",
                "rows-sort-150",
                "rows-null-z",
                "rows-null-a",
            ]
        );
        assert_eq!(
            subjects.iter().collect::<HashSet<_>>().len(),
            subjects.len()
        );
        assert!(
            cursor_kinds.contains(&true),
            "must continue from non-NULL sort key"
        );
        assert!(
            cursor_kinds.contains(&false),
            "must continue from NULL sort key"
        );
    }

    #[tokio::test]
    async fn list_supports_active_media_lifecycle_and_attention_filters() {
        let fixture = fresh_fixture("filters").await;
        let repository = make_repository(&fixture.path);
        repository
            .executor
            .run(|connection| {
                connection
                    .execute(
                        "UPDATE wanted_subscription_records
                            SET attention_tags_json = '[\"failed\"]'
                          WHERE account_key = ?1 AND subject_id = 'rows-movie-001'",
                        [ACCOUNT],
                    )
                    .map_err(|error| super::map_read_error("seed failed attention", error))?;
                connection
                    .execute(
                        INSERT_CLONE_SQL,
                        params![ACCOUNT, "rows-movie-002", "rows-tv", "Rows TV", 100_i64],
                    )
                    .map_err(|error| super::map_read_error("insert TV filter row", error))?;
                connection
                    .execute(
                        "UPDATE wanted_subscription_records
                            SET media_kind = 'tv', schedulable = 0,
                                blocked_reason = 'tv_not_supported',
                                attention_tags_json = '[\"needs_reconciliation\"]'
                          WHERE account_key = ?1 AND subject_id = 'rows-tv'",
                        [ACCOUNT],
                    )
                    .map_err(|error| super::map_read_error("park TV filter row", error))?;
                connection
                    .execute(
                        INSERT_CLONE_SQL,
                        params![
                            ACCOUNT,
                            "rows-movie-001",
                            "rows-inactive",
                            "Rows Inactive",
                            50_i64
                        ],
                    )
                    .map_err(|error| super::map_read_error("insert inactive filter row", error))?;
                connection
                    .execute(
                        "UPDATE wanted_subscription_records
                            SET active = 0, inactive_at = updated_at, schedulable = 0,
                                blocked_reason = 'subscription_inactive', next_attempt_at = NULL,
                                attention_tags_json = '[\"skipped\"]'
                          WHERE account_key = ?1 AND subject_id = 'rows-inactive'",
                        [ACCOUNT],
                    )
                    .map_err(|error| super::map_read_error("deactivate filter row", error))?;
                Ok(())
            })
            .await
            .expect("seed filter fixture");

        for (filter, expected) in [
            (
                SubscriptionListFilter {
                    active: Some(false),
                    ..SubscriptionListFilter::default()
                },
                vec!["rows-inactive"],
            ),
            (
                SubscriptionListFilter {
                    media_kind: Some(SubscriptionMediaKind::Tv),
                    ..SubscriptionListFilter::default()
                },
                vec!["rows-tv"],
            ),
            (
                SubscriptionListFilter {
                    lifecycle_state: Some(SubscriptionLifecycleState::Completed),
                    ..SubscriptionListFilter::default()
                },
                vec!["rows-tv", "rows-movie-002"],
            ),
            (
                SubscriptionListFilter {
                    attention_tag: Some(SubscriptionAttentionTag::Failed),
                    ..SubscriptionListFilter::default()
                },
                vec!["rows-movie-001"],
            ),
        ] {
            let page = repository
                .list_summaries(list_command(filter, None, 100))
                .await
                .expect("read filtered list");
            assert_eq!(
                page.items
                    .iter()
                    .map(|summary| summary.head.key.subject_id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[tokio::test]
    async fn query_plans_use_frozen_list_and_primary_key_indexes_without_legacy_blob_access() {
        let fixture = fresh_fixture("query-plan").await;
        let repository = make_repository(&fixture.path);
        let command = list_command(SubscriptionListFilter::default(), None, 10);
        let list_queries = build_list_queries(&command).expect("build default list SQL");
        assert_eq!(
            list_queries.len(),
            4,
            "default list has two activity and two NULL partitions"
        );
        for query in &list_queries {
            assert!(!query.sql.contains("record_json"));
            assert_eq!(
                query.values.last(),
                Some(&rusqlite::types::Value::Integer(11)),
                "each list segment must fetch limit + 1 rows"
            );
            assert!(!query.sql.contains("subscription_state_blobs_legacy_v4"));
        }
        for sql in [GET_SQL, DETAIL_SQL] {
            assert!(!sql.contains("subscription_state_blobs_legacy_v4"));
        }
        assert!(!UPDATE_DETAIL_SQL.contains("subscription_state_blobs_legacy_v4"));
        for untouched_control in [
            "active =",
            "lifecycle_state =",
            "execution_state =",
            "next_attempt_at =",
            "retry_count =",
            "max_retries =",
            "retry_blocked =",
            "force_eligible_once =",
            "claimed_operation =",
            "attempt_id =",
            "lease_until =",
        ] {
            assert!(
                !UPDATE_DETAIL_SQL.contains(untouched_control),
                "detail update must not own scheduler control {untouched_control}"
            );
        }
        let seek_command = list_command(
            SubscriptionListFilter {
                active: Some(true),
                ..SubscriptionListFilter::default()
            },
            Some(ListCursor::try_new(Some(250), "seek-subject").unwrap()),
            10,
        );
        let seek_query = build_list_queries(&seek_command)
            .expect("build non-NULL seek SQL")
            .into_iter()
            .find(|query| query.sql.contains("IS NOT NULL"))
            .expect("non-NULL seek segment");
        let null_seek_command = list_command(
            SubscriptionListFilter {
                active: Some(true),
                ..SubscriptionListFilter::default()
            },
            Some(ListCursor::try_new(None, "seek-subject").unwrap()),
            10,
        );
        let null_seek_query = build_list_queries(&null_seek_command)
            .expect("build NULL seek SQL")
            .into_iter()
            .next()
            .expect("NULL seek segment");

        let (list_plans, seek_plan, null_seek_plan, get_plan, detail_plan, update_plan) =
            repository
                .executor
                .run(move |connection| {
                    let list_plans = list_queries
                        .into_iter()
                        .map(|query| explain(connection, &query.sql, query.values))
                        .collect::<Result<Vec<_>, _>>()?;
                    let seek_plan = explain(connection, &seek_query.sql, seek_query.values)?;
                    let null_seek_plan =
                        explain(connection, &null_seek_query.sql, null_seek_query.values)?;
                    let get_plan = explain(
                        connection,
                        GET_SQL,
                        vec![
                            rusqlite::types::Value::Text(ACCOUNT.to_string()),
                            rusqlite::types::Value::Text("rows-movie-001".to_string()),
                        ],
                    )?;
                    let detail_plan = explain(
                        connection,
                        DETAIL_SQL,
                        vec![
                            rusqlite::types::Value::Text(ACCOUNT.to_string()),
                            rusqlite::types::Value::Text("rows-movie-001".to_string()),
                        ],
                    )?;
                    let update_plan = explain(
                        connection,
                        UPDATE_DETAIL_SQL,
                        vec![
                            rusqlite::types::Value::Text(ACCOUNT.to_string()),
                            rusqlite::types::Value::Text("rows-movie-001".to_string()),
                            rusqlite::types::Value::Integer(1),
                            rusqlite::types::Value::Text("Updated".to_string()),
                            rusqlite::types::Value::Integer(2030),
                            rusqlite::types::Value::Text(String::new()),
                            rusqlite::types::Value::Null,
                            rusqlite::types::Value::Integer(1_900_000_000),
                            rusqlite::types::Value::Text("[]".to_string()),
                            rusqlite::types::Value::Integer(1_900_000_000),
                            rusqlite::types::Value::Text("{}".to_string()),
                        ],
                    )?;
                    Ok((
                        list_plans,
                        seek_plan,
                        null_seek_plan,
                        get_plan,
                        detail_plan,
                        update_plan,
                    ))
                })
                .await
                .expect("inspect repository query plans");

        for plan in &list_plans {
            assert!(
                plan.iter()
                    .any(|detail| detail.contains("wanted_records_list_v5_idx")),
                "default list segment did not use frozen list index: {plan:?}"
            );
            assert!(
                plan.iter().all(|detail| !detail.contains("TEMP B-TREE")),
                "default list segment used an unbounded temp sort: {plan:?}"
            );
        }
        assert!(
            seek_plan.iter().any(|detail| {
                detail.contains("wanted_records_list_v5_idx")
                    && detail.contains("(douban_sort_time,subject_id)<(?,?)")
            }),
            "non-NULL cursor did not seek on the sort key: {seek_plan:?}"
        );
        assert!(seek_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE")));
        assert!(
            null_seek_plan.iter().any(|detail| {
                detail.contains("wanted_records_list_v5_idx")
                    && detail.contains("douban_sort_time=? AND subject_id<?")
            }),
            "NULL cursor did not seek on sort partition and subject key: {null_seek_plan:?}"
        );
        assert!(null_seek_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE")));
        for (label, plan) in [("get", get_plan), ("detail", detail_plan)] {
            assert!(
                plan.iter().any(|detail| {
                    detail.contains("sqlite_autoindex_wanted_subscription_records_1")
                        && detail.contains("account_key=? AND subject_id=?")
                }),
                "{label} plan did not use the composite primary-key index: {plan:?}"
            );
            assert!(plan
                .iter()
                .all(|detail| !detail.contains("subscription_state_blobs_legacy_v4")));
        }
        assert!(
            update_plan.iter().any(|detail| {
                detail.contains("sqlite_autoindex_wanted_subscription_records_1")
                    && detail.contains("account_key=? AND subject_id=?")
            }),
            "optimistic update plan did not use the composite primary-key index: {update_plan:?}"
        );
        assert!(update_plan
            .iter()
            .all(|detail| !detail.contains("TEMP B-TREE")));
    }

    fn explain(
        connection: &Connection,
        sql: &str,
        values: Vec<rusqlite::types::Value>,
    ) -> Result<Vec<String>, RepositoryError> {
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .map_err(|error| super::map_read_error("prepare query plan", error))?;
        let rows = statement
            .query_map(params_from_iter(values.iter()), |row| {
                row.get::<_, String>(3)
            })
            .map_err(|error| super::map_read_error("query plan", error))?;
        rows.map(|row| row.map_err(|error| super::map_read_error("decode query plan", error)))
            .collect()
    }
}
