use std::fmt;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

pub(crate) const SCHEMA_VERSION: u32 = 5;

/// Canonical operation-log indexes for a freshly initialized latest-schema database.
pub(crate) const ENSURE_OPERATION_LOG_INDEXES_SQL: &str = r#"
CREATE INDEX IF NOT EXISTS operation_logs_created_idx
ON operation_logs (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS operation_logs_category_status_idx
ON operation_logs (category, status, created_at DESC);
"#;

const LATEST_SCHEMA_SQL: &str = r#"
CREATE TABLE subscription_schema_meta (
    key TEXT NOT NULL,
    value INTEGER NOT NULL
);

CREATE TABLE subscription_meta (
    account_key TEXT NOT NULL PRIMARY KEY CHECK (length(trim(account_key)) > 0),
    state_version INTEGER NOT NULL CHECK (state_version > 0),
    bootstrap_completed INTEGER NOT NULL CHECK (bootstrap_completed IN (0, 1)),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    last_poll_attempt_at INTEGER CHECK (last_poll_attempt_at IS NULL OR last_poll_attempt_at >= 0),
    last_poll_success_at INTEGER CHECK (last_poll_success_at IS NULL OR last_poll_success_at >= 0),
    poll_failure_count INTEGER NOT NULL CHECK (poll_failure_count >= 0),
    next_poll_at INTEGER NOT NULL CHECK (next_poll_at >= 0),
    last_poll_error TEXT,
    poll_generation INTEGER NOT NULL CHECK (poll_generation >= 0),
    open_poll_generation INTEGER CHECK (open_poll_generation IS NULL OR open_poll_generation > 0),
    open_snapshot_id TEXT CHECK (open_snapshot_id IS NULL OR length(trim(open_snapshot_id)) > 0),
    last_incomplete_at INTEGER CHECK (last_incomplete_at IS NULL OR last_incomplete_at >= 0),
    last_incomplete_reason TEXT CHECK (
        last_incomplete_reason IS NULL OR length(trim(last_incomplete_reason)) > 0
    ),
    last_incomplete_fetched_pages INTEGER CHECK (
        last_incomplete_fetched_pages IS NULL OR last_incomplete_fetched_pages >= 0
    ),
    last_incomplete_truncated INTEGER CHECK (
        last_incomplete_truncated IS NULL OR last_incomplete_truncated IN (0, 1)
    ),
    last_incomplete_end_observed INTEGER CHECK (
        last_incomplete_end_observed IS NULL OR last_incomplete_end_observed IN (0, 1)
    ),
    last_complete_snapshot_id TEXT CHECK (
        last_complete_snapshot_id IS NULL OR length(trim(last_complete_snapshot_id)) > 0
    ),
    CHECK (updated_at >= created_at),
    CHECK (
        last_poll_attempt_at IS NULL
        OR last_poll_attempt_at BETWEEN created_at AND updated_at
    ),
    CHECK (
        last_poll_success_at IS NULL
        OR last_poll_success_at BETWEEN created_at AND updated_at
    ),
    CHECK (
        last_poll_attempt_at IS NULL
        OR last_poll_success_at IS NULL
        OR open_poll_generation IS NOT NULL
        OR (poll_failure_count = 0 AND last_poll_success_at >= last_poll_attempt_at)
        OR (poll_failure_count > 0 AND last_poll_success_at <= last_poll_attempt_at)
    ),
    CHECK ((open_poll_generation IS NULL) = (open_snapshot_id IS NULL)),
    CHECK (open_poll_generation IS NULL OR open_poll_generation = poll_generation),
    CHECK (
        (poll_failure_count = 0 AND last_poll_error IS NULL)
        OR (poll_failure_count > 0
            AND last_poll_error IS NOT NULL
            AND length(trim(last_poll_error)) > 0)
    ),
    CHECK (
        (last_incomplete_at IS NULL
            AND last_incomplete_reason IS NULL
            AND last_incomplete_fetched_pages IS NULL
            AND last_incomplete_truncated IS NULL
            AND last_incomplete_end_observed IS NULL)
        OR
        (last_incomplete_at IS NOT NULL
            AND last_incomplete_reason IS NOT NULL
            AND last_incomplete_fetched_pages IS NOT NULL
            AND last_incomplete_truncated IS NOT NULL
            AND last_incomplete_end_observed IS NOT NULL)
    )
) STRICT;

CREATE TABLE wanted_subscription_records (
    account_key TEXT NOT NULL CHECK (length(trim(account_key)) > 0),
    subject_id TEXT NOT NULL CHECK (length(trim(subject_id)) > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    active INTEGER NOT NULL CHECK (active IN (0, 1)),
    inactive_at INTEGER CHECK (inactive_at IS NULL OR inactive_at >= 0),
    last_seen_snapshot_id TEXT CHECK (
        last_seen_snapshot_id IS NULL OR length(trim(last_seen_snapshot_id)) > 0
    ),
    media_kind TEXT NOT NULL CHECK (media_kind IN ('movie', 'tv')),
    schedulable INTEGER NOT NULL CHECK (schedulable IN (0, 1)),
    blocked_reason TEXT CHECK (blocked_reason IS NULL OR length(trim(blocked_reason)) > 0),
    lifecycle_state TEXT NOT NULL CHECK (
        lifecycle_state IN ('queued', 'meta', 'searching', 'downloading', 'linking', 'completed')
    ),
    execution_state TEXT NOT NULL CHECK (execution_state IN ('idle', 'running')),
    next_attempt_at INTEGER CHECK (next_attempt_at IS NULL OR next_attempt_at >= 0),
    retry_count INTEGER NOT NULL CHECK (retry_count >= 0),
    max_retries INTEGER NOT NULL CHECK (max_retries >= 0),
    retry_blocked INTEGER NOT NULL CHECK (retry_blocked IN (0, 1)),
    force_eligible_once INTEGER NOT NULL CHECK (force_eligible_once IN (0, 1)),
    claimed_operation TEXT CHECK (
        claimed_operation IS NULL
        OR claimed_operation IN ('movie_meta', 'movie_search', 'movie_progress', 'movie_link')
    ),
    attempt_id TEXT CHECK (
        attempt_id IS NULL
        OR (length(trim(attempt_id)) > 0 AND length(CAST(attempt_id AS BLOB)) <= 256)
    ),
    lease_until INTEGER CHECK (lease_until IS NULL OR lease_until >= 0),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    release_year INTEGER CHECK (release_year IS NULL OR release_year BETWEEN 1 AND 9999),
    poster_url TEXT NOT NULL,
    category_text TEXT,
    douban_sort_time INTEGER CHECK (douban_sort_time IS NULL OR douban_sort_time >= 0),
    attention_tags_json TEXT NOT NULL CHECK (
        json_valid(attention_tags_json) AND json_type(attention_tags_json) = 'array'
    ),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    record_json TEXT NOT NULL CHECK (json_valid(record_json) AND json_type(record_json) = 'object'),
    PRIMARY KEY (account_key, subject_id),
    FOREIGN KEY (account_key) REFERENCES subscription_meta(account_key)
        ON UPDATE RESTRICT ON DELETE CASCADE,
    CHECK ((active = 1 AND inactive_at IS NULL) OR (active = 0 AND inactive_at IS NOT NULL)),
    CHECK (inactive_at IS NULL OR inactive_at <= updated_at),
    CHECK (
        (schedulable = 1 AND blocked_reason IS NULL)
        OR (schedulable = 0 AND blocked_reason IS NOT NULL)
    ),
    CHECK (
        (execution_state = 'idle'
            AND claimed_operation IS NULL
            AND attempt_id IS NULL
            AND lease_until IS NULL)
        OR
        (execution_state = 'running'
            AND claimed_operation IS NOT NULL
            AND attempt_id IS NOT NULL
            AND lease_until IS NOT NULL)
    ),
    CHECK (
        execution_state != 'running'
        OR (active = 1
            AND schedulable = 1
            AND media_kind = 'movie'
            AND lifecycle_state != 'completed')
    ),
    CHECK (
        execution_state != 'running'
        OR claimed_operation = CASE lifecycle_state
            WHEN 'queued' THEN 'movie_meta'
            WHEN 'meta' THEN 'movie_meta'
            WHEN 'searching' THEN 'movie_search'
            WHEN 'downloading' THEN 'movie_progress'
            WHEN 'linking' THEN 'movie_link'
            ELSE NULL
        END
    ),
    CHECK (active = 1 OR (execution_state = 'idle' AND next_attempt_at IS NULL)),
    CHECK (active = 1 OR schedulable = 0),
    CHECK (schedulable = 1 OR next_attempt_at IS NULL),
    CHECK (lifecycle_state != 'completed' OR next_attempt_at IS NULL),
    CHECK (
        retry_blocked = CASE
            WHEN max_retries > 0 AND retry_count >= max_retries THEN 1
            ELSE 0
        END
    ),
    CHECK (
        force_eligible_once = 0
        OR (active = 1
            AND schedulable = 1
            AND lifecycle_state != 'completed'
            AND next_attempt_at IS NOT NULL)
    ),
    CHECK (
        media_kind != 'tv'
        OR (schedulable = 0
            AND blocked_reason = 'tv_not_supported'
            AND execution_state = 'idle'
            AND next_attempt_at IS NULL)
    )
) STRICT;

CREATE TABLE operation_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_key TEXT NOT NULL DEFAULT '',
    created_at INTEGER NOT NULL,
    category TEXT NOT NULL,
    action TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT,
    target_title TEXT,
    status TEXT NOT NULL,
    summary TEXT NOT NULL,
    error TEXT,
    related_json TEXT NOT NULL DEFAULT '{}'
);

CREATE INDEX wanted_records_due_v5_idx
ON wanted_subscription_records
    (account_key, next_attempt_at, updated_at, subject_id)
WHERE active = 1
  AND schedulable = 1
  AND blocked_reason IS NULL
  AND media_kind = 'movie'
  AND lifecycle_state != 'completed'
  AND execution_state = 'idle'
  AND force_eligible_once = 0
  AND retry_blocked = 0
  AND next_attempt_at IS NOT NULL;

CREATE INDEX wanted_records_expired_lease_v5_idx
ON wanted_subscription_records
    (account_key, execution_state, lease_until, subject_id);

CREATE INDEX wanted_records_force_v5_idx
ON wanted_subscription_records
    (account_key, next_attempt_at, updated_at, subject_id)
WHERE active = 1
  AND schedulable = 1
  AND blocked_reason IS NULL
  AND media_kind = 'movie'
  AND lifecycle_state != 'completed'
  AND execution_state = 'idle'
  AND force_eligible_once = 1
  AND next_attempt_at IS NOT NULL;

CREATE INDEX wanted_records_list_v5_idx
ON wanted_subscription_records
    (account_key, active, douban_sort_time DESC, subject_id DESC);

CREATE UNIQUE INDEX wanted_records_attempt_v5_uidx
ON wanted_subscription_records (attempt_id)
WHERE attempt_id IS NOT NULL;
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaContractError {
    message: String,
}

impl SchemaContractError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for SchemaContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ColumnContract {
    name: &'static str,
    declared_type: &'static str,
    not_null: bool,
    primary_key_position: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActualColumn {
    position: i64,
    name: String,
    declared_type: Option<String>,
    not_null: bool,
    primary_key_position: i64,
    hidden: i64,
}

#[derive(Debug, Clone, Copy)]
struct ForeignKeyContract {
    id: i64,
    sequence: i64,
    parent_table: &'static str,
    from_column: &'static str,
    to_column: &'static str,
    on_update: &'static str,
    on_delete: &'static str,
    match_kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActualForeignKey {
    id: i64,
    sequence: i64,
    parent_table: String,
    from_column: String,
    to_column: Option<String>,
    on_update: String,
    on_delete: String,
    match_kind: String,
}

#[derive(Debug, Clone, Copy)]
struct TableContract {
    name: &'static str,
    strict: bool,
    columns: &'static [ColumnContract],
    foreign_keys: &'static [ForeignKeyContract],
    required_sql_fragments: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexColumnContract {
    name: &'static str,
    descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActualIndexColumn {
    sequence: i64,
    column_id: i64,
    name: Option<String>,
    descending: bool,
    collation: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct IndexContract {
    name: &'static str,
    table: &'static str,
    unique: bool,
    partial: bool,
    columns: &'static [IndexColumnContract],
    partial_predicate: Option<&'static str>,
}

macro_rules! column {
    ($name:literal, $declared_type:literal, $not_null:literal, $pk:literal) => {
        ColumnContract {
            name: $name,
            declared_type: $declared_type,
            not_null: $not_null,
            primary_key_position: $pk,
        }
    };
}

const SCHEMA_META_COLUMNS: &[ColumnContract] = &[
    column!("key", "TEXT", true, 0),
    column!("value", "INTEGER", true, 0),
];

const SUBSCRIPTION_META_COLUMNS: &[ColumnContract] = &[
    column!("account_key", "TEXT", true, 1),
    column!("state_version", "INTEGER", true, 0),
    column!("bootstrap_completed", "INTEGER", true, 0),
    column!("created_at", "INTEGER", true, 0),
    column!("updated_at", "INTEGER", true, 0),
    column!("last_poll_attempt_at", "INTEGER", false, 0),
    column!("last_poll_success_at", "INTEGER", false, 0),
    column!("poll_failure_count", "INTEGER", true, 0),
    column!("next_poll_at", "INTEGER", true, 0),
    column!("last_poll_error", "TEXT", false, 0),
    column!("poll_generation", "INTEGER", true, 0),
    column!("open_poll_generation", "INTEGER", false, 0),
    column!("open_snapshot_id", "TEXT", false, 0),
    column!("last_incomplete_at", "INTEGER", false, 0),
    column!("last_incomplete_reason", "TEXT", false, 0),
    column!("last_incomplete_fetched_pages", "INTEGER", false, 0),
    column!("last_incomplete_truncated", "INTEGER", false, 0),
    column!("last_incomplete_end_observed", "INTEGER", false, 0),
    column!("last_complete_snapshot_id", "TEXT", false, 0),
];

const WANTED_RECORD_COLUMNS: &[ColumnContract] = &[
    column!("account_key", "TEXT", true, 1),
    column!("subject_id", "TEXT", true, 2),
    column!("revision", "INTEGER", true, 0),
    column!("active", "INTEGER", true, 0),
    column!("inactive_at", "INTEGER", false, 0),
    column!("last_seen_snapshot_id", "TEXT", false, 0),
    column!("media_kind", "TEXT", true, 0),
    column!("schedulable", "INTEGER", true, 0),
    column!("blocked_reason", "TEXT", false, 0),
    column!("lifecycle_state", "TEXT", true, 0),
    column!("execution_state", "TEXT", true, 0),
    column!("next_attempt_at", "INTEGER", false, 0),
    column!("retry_count", "INTEGER", true, 0),
    column!("max_retries", "INTEGER", true, 0),
    column!("retry_blocked", "INTEGER", true, 0),
    column!("force_eligible_once", "INTEGER", true, 0),
    column!("claimed_operation", "TEXT", false, 0),
    column!("attempt_id", "TEXT", false, 0),
    column!("lease_until", "INTEGER", false, 0),
    column!("title", "TEXT", true, 0),
    column!("release_year", "INTEGER", false, 0),
    column!("poster_url", "TEXT", true, 0),
    column!("category_text", "TEXT", false, 0),
    column!("douban_sort_time", "INTEGER", false, 0),
    column!("attention_tags_json", "TEXT", true, 0),
    column!("updated_at", "INTEGER", true, 0),
    column!("record_json", "TEXT", true, 0),
];

const OPERATION_LOG_COLUMNS: &[ColumnContract] = &[
    column!("id", "INTEGER", false, 1),
    column!("account_key", "TEXT", true, 0),
    column!("created_at", "INTEGER", true, 0),
    column!("category", "TEXT", true, 0),
    column!("action", "TEXT", true, 0),
    column!("target_type", "TEXT", true, 0),
    column!("target_id", "TEXT", false, 0),
    column!("target_title", "TEXT", false, 0),
    column!("status", "TEXT", true, 0),
    column!("summary", "TEXT", true, 0),
    column!("error", "TEXT", false, 0),
    column!("related_json", "TEXT", true, 0),
];

const WANTED_RECORD_FOREIGN_KEYS: &[ForeignKeyContract] = &[ForeignKeyContract {
    id: 0,
    sequence: 0,
    parent_table: "subscription_meta",
    from_column: "account_key",
    to_column: "account_key",
    on_update: "RESTRICT",
    on_delete: "CASCADE",
    match_kind: "NONE",
}];

const SUBSCRIPTION_META_REQUIRED_SQL: &[&str] = &[
    r#"CHECK ((open_poll_generation IS NULL) = (open_snapshot_id IS NULL))"#,
    r#"CHECK (open_poll_generation IS NULL OR open_poll_generation = poll_generation)"#,
    r#"CHECK (
        last_poll_attempt_at IS NULL
        OR last_poll_success_at IS NULL
        OR open_poll_generation IS NOT NULL
        OR (poll_failure_count = 0 AND last_poll_success_at >= last_poll_attempt_at)
        OR (poll_failure_count > 0 AND last_poll_success_at <= last_poll_attempt_at)
    )"#,
    r#"CHECK (
        (poll_failure_count = 0 AND last_poll_error IS NULL)
        OR (poll_failure_count > 0
            AND last_poll_error IS NOT NULL
            AND length(trim(last_poll_error)) > 0)
    )"#,
];

const WANTED_RECORD_REQUIRED_SQL: &[&str] = &[
    r#"force_eligible_once INTEGER NOT NULL CHECK (force_eligible_once IN (0, 1))"#,
    r#"CHECK (
        attempt_id IS NULL
        OR (length(trim(attempt_id)) > 0 AND length(CAST(attempt_id AS BLOB)) <= 256)
    )"#,
    r#"CHECK (
        claimed_operation IS NULL
        OR claimed_operation IN ('movie_meta', 'movie_search', 'movie_progress', 'movie_link')
    )"#,
    r#"CHECK (
        (execution_state = 'idle'
            AND claimed_operation IS NULL
            AND attempt_id IS NULL
            AND lease_until IS NULL)
        OR
        (execution_state = 'running'
            AND claimed_operation IS NOT NULL
            AND attempt_id IS NOT NULL
            AND lease_until IS NOT NULL)
    )"#,
    r#"CHECK (
        (schedulable = 1 AND blocked_reason IS NULL)
        OR (schedulable = 0 AND blocked_reason IS NOT NULL)
    )"#,
    r#"CHECK (
        execution_state != 'running'
        OR (active = 1
            AND schedulable = 1
            AND media_kind = 'movie'
            AND lifecycle_state != 'completed')
    )"#,
    r#"CHECK (
        execution_state != 'running'
        OR claimed_operation = CASE lifecycle_state
            WHEN 'queued' THEN 'movie_meta'
            WHEN 'meta' THEN 'movie_meta'
            WHEN 'searching' THEN 'movie_search'
            WHEN 'downloading' THEN 'movie_progress'
            WHEN 'linking' THEN 'movie_link'
            ELSE NULL
        END
    )"#,
    r#"CHECK (active = 1 OR (execution_state = 'idle' AND next_attempt_at IS NULL))"#,
    r#"CHECK (active = 1 OR schedulable = 0)"#,
    r#"CHECK (schedulable = 1 OR next_attempt_at IS NULL)"#,
    r#"CHECK (lifecycle_state != 'completed' OR next_attempt_at IS NULL)"#,
    r#"CHECK (
        retry_blocked = CASE
            WHEN max_retries > 0 AND retry_count >= max_retries THEN 1
            ELSE 0
        END
    )"#,
    r#"CHECK (
        force_eligible_once = 0
        OR (active = 1
            AND schedulable = 1
            AND lifecycle_state != 'completed'
            AND next_attempt_at IS NOT NULL)
    )"#,
    r#"CHECK (
        media_kind != 'tv'
        OR (schedulable = 0
            AND blocked_reason = 'tv_not_supported'
            AND execution_state = 'idle'
            AND next_attempt_at IS NULL)
    )"#,
];

const OPERATION_LOG_REQUIRED_SQL: &[&str] = &["id INTEGER PRIMARY KEY AUTOINCREMENT"];

const TABLES: &[TableContract] = &[
    TableContract {
        name: "subscription_schema_meta",
        strict: false,
        columns: SCHEMA_META_COLUMNS,
        foreign_keys: &[],
        required_sql_fragments: &[],
    },
    TableContract {
        name: "subscription_meta",
        strict: true,
        columns: SUBSCRIPTION_META_COLUMNS,
        foreign_keys: &[],
        required_sql_fragments: SUBSCRIPTION_META_REQUIRED_SQL,
    },
    TableContract {
        name: "wanted_subscription_records",
        strict: true,
        columns: WANTED_RECORD_COLUMNS,
        foreign_keys: WANTED_RECORD_FOREIGN_KEYS,
        required_sql_fragments: WANTED_RECORD_REQUIRED_SQL,
    },
    TableContract {
        name: "operation_logs",
        strict: false,
        columns: OPERATION_LOG_COLUMNS,
        foreign_keys: &[],
        required_sql_fragments: OPERATION_LOG_REQUIRED_SQL,
    },
];

const INDEXES: &[IndexContract] = &[
    IndexContract {
        name: "wanted_records_due_v5_idx",
        table: "wanted_subscription_records",
        unique: false,
        partial: true,
        columns: &[
            IndexColumnContract {
                name: "account_key",
                descending: false,
            },
            IndexColumnContract {
                name: "next_attempt_at",
                descending: false,
            },
            IndexColumnContract {
                name: "updated_at",
                descending: false,
            },
            IndexColumnContract {
                name: "subject_id",
                descending: false,
            },
        ],
        partial_predicate: Some(
            "active = 1 and schedulable = 1 and blocked_reason is null and media_kind = 'movie' and lifecycle_state != 'completed' and execution_state = 'idle' and force_eligible_once = 0 and retry_blocked = 0 and next_attempt_at is not null",
        ),
    },
    IndexContract {
        name: "wanted_records_expired_lease_v5_idx",
        table: "wanted_subscription_records",
        unique: false,
        partial: false,
        columns: &[
            IndexColumnContract {
                name: "account_key",
                descending: false,
            },
            IndexColumnContract {
                name: "execution_state",
                descending: false,
            },
            IndexColumnContract {
                name: "lease_until",
                descending: false,
            },
            IndexColumnContract {
                name: "subject_id",
                descending: false,
            },
        ],
        partial_predicate: None,
    },
    IndexContract {
        name: "wanted_records_force_v5_idx",
        table: "wanted_subscription_records",
        unique: false,
        partial: true,
        columns: &[
            IndexColumnContract {
                name: "account_key",
                descending: false,
            },
            IndexColumnContract {
                name: "next_attempt_at",
                descending: false,
            },
            IndexColumnContract {
                name: "updated_at",
                descending: false,
            },
            IndexColumnContract {
                name: "subject_id",
                descending: false,
            },
        ],
        partial_predicate: Some(
            "active = 1 and schedulable = 1 and blocked_reason is null and media_kind = 'movie' and lifecycle_state != 'completed' and execution_state = 'idle' and force_eligible_once = 1 and next_attempt_at is not null",
        ),
    },
    IndexContract {
        name: "wanted_records_list_v5_idx",
        table: "wanted_subscription_records",
        unique: false,
        partial: false,
        columns: &[
            IndexColumnContract {
                name: "account_key",
                descending: false,
            },
            IndexColumnContract {
                name: "active",
                descending: false,
            },
            IndexColumnContract {
                name: "douban_sort_time",
                descending: true,
            },
            IndexColumnContract {
                name: "subject_id",
                descending: true,
            },
        ],
        partial_predicate: None,
    },
    IndexContract {
        name: "wanted_records_attempt_v5_uidx",
        table: "wanted_subscription_records",
        unique: true,
        partial: true,
        columns: &[IndexColumnContract {
            name: "attempt_id",
            descending: false,
        }],
        partial_predicate: Some("attempt_id is not null"),
    },
    IndexContract {
        name: "operation_logs_created_idx",
        table: "operation_logs",
        unique: false,
        partial: false,
        columns: &[
            IndexColumnContract {
                name: "created_at",
                descending: true,
            },
            IndexColumnContract {
                name: "id",
                descending: true,
            },
        ],
        partial_predicate: None,
    },
    IndexContract {
        name: "operation_logs_category_status_idx",
        table: "operation_logs",
        unique: false,
        partial: false,
        columns: &[
            IndexColumnContract {
                name: "category",
                descending: false,
            },
            IndexColumnContract {
                name: "status",
                descending: false,
            },
            IndexColumnContract {
                name: "created_at",
                descending: true,
            },
        ],
        partial_predicate: None,
    },
];

const FORBIDDEN_OBJECTS: &[&str] = &[
    "subscription_meta_v5",
    "wanted_subscription_records_v5",
    "subscription_state_blobs",
    "subscription_state_blobs_legacy_v4",
    "subscription_state_blobs_legacy_v4_no_insert",
    "subscription_state_blobs_legacy_v4_no_update",
    "subscription_state_blobs_legacy_v4_no_delete",
];

/// Initialize an empty SQLite connection with the one supported subscription schema.
///
/// This entrypoint is intentionally fresh-only: it refuses every pre-existing user schema object,
/// never reads or transforms an older database, and installs no compatibility table or trigger.
pub(crate) fn initialize_latest_schema(
    connection: &mut Connection,
) -> Result<(), SchemaContractError> {
    let foreign_keys: i64 = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|error| {
            SchemaContractError::new(format!(
                "read foreign_keys before latest-schema initialization: {error}"
            ))
        })?;
    if foreign_keys != 1 {
        return Err(SchemaContractError::new(format!(
            "latest-schema initialization requires foreign_keys=1, found {foreign_keys}"
        )));
    }

    let existing_objects: i64 = connection
        .query_row(
            "SELECT COUNT(*)
               FROM sqlite_schema
              WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| {
            SchemaContractError::new(format!(
                "inspect empty database before latest-schema initialization: {error}"
            ))
        })?;
    if existing_objects != 0 {
        return Err(SchemaContractError::new(format!(
            "latest-schema initialization requires an empty database, found {existing_objects} user objects"
        )));
    }

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| {
            SchemaContractError::new(format!(
                "begin latest-schema initialization transaction: {error}"
            ))
        })?;
    transaction
        .execute_batch(LATEST_SCHEMA_SQL)
        .map_err(|error| {
            SchemaContractError::new(format!("create latest subscription schema: {error}"))
        })?;
    transaction
        .execute(
            "INSERT INTO subscription_schema_meta (key, value) VALUES ('schema_version', ?1)",
            [i64::from(SCHEMA_VERSION)],
        )
        .map_err(|error| {
            SchemaContractError::new(format!("write latest schema-version marker: {error}"))
        })?;
    transaction
        .execute_batch(ENSURE_OPERATION_LOG_INDEXES_SQL)
        .map_err(|error| {
            SchemaContractError::new(format!("create latest operation-log indexes: {error}"))
        })?;
    validate_schema_contract(&transaction)?;
    let marker_count: i64 = transaction
        .query_row(
            "SELECT COUNT(*)
               FROM subscription_schema_meta
              WHERE key = 'schema_version' AND value = ?1",
            [i64::from(SCHEMA_VERSION)],
            |row| row.get(0),
        )
        .map_err(|error| {
            SchemaContractError::new(format!(
                "verify latest schema-version marker before commit: {error}"
            ))
        })?;
    if marker_count != 1 {
        return Err(SchemaContractError::new(format!(
            "latest schema must contain one version-{SCHEMA_VERSION} marker, found {marker_count}"
        )));
    }
    transaction.commit().map_err(|error| {
        SchemaContractError::new(format!("commit latest-schema initialization: {error}"))
    })
}

/// Validate the exact latest-schema shape used by fresh `subscriptions.sqlite` databases.
///
/// This intentionally fingerprints table layout, strictness, key topology, foreign keys, runtime
/// indexes, key runtime CHECK fragments, and the absence of forbidden/staging objects. CHECK expressions
/// that are not listed in the manifest remain enforced by SQLite but are not fully reparsed here.
pub(crate) fn validate_schema_contract(connection: &Connection) -> Result<(), SchemaContractError> {
    for table in TABLES {
        validate_table(connection, table)?;
    }
    for index in INDEXES {
        validate_index(connection, index)?;
    }
    for name in FORBIDDEN_OBJECTS {
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_schema WHERE name = ?1",
                [name],
                |row| row.get(0),
            )
            .map_err(|error| {
                SchemaContractError::new(format!("inspect forbidden schema object {name}: {error}"))
            })?;
        if count != 0 {
            return Err(SchemaContractError::new(format!(
                "obsolete schema object {name} remains in schema-v5 runtime"
            )));
        }
    }
    Ok(())
}

fn validate_table(
    connection: &Connection,
    contract: &TableContract,
) -> Result<(), SchemaContractError> {
    let object_count: i64 = connection
        .query_row(
            "SELECT COUNT(*)
               FROM sqlite_schema
              WHERE type = 'table' AND name = ?1 AND tbl_name = ?1",
            [contract.name],
            |row| row.get(0),
        )
        .map_err(|error| {
            SchemaContractError::new(format!("inspect table {}: {error}", contract.name))
        })?;
    if object_count != 1 {
        return Err(SchemaContractError::new(format!(
            "schema-v5 requires exactly one table named {}, found {object_count}",
            contract.name
        )));
    }

    let table_sql: String = connection
        .query_row(
            "SELECT sql
               FROM sqlite_schema
              WHERE type = 'table' AND name = ?1 AND tbl_name = ?1",
            [contract.name],
            |row| row.get(0),
        )
        .map_err(|error| {
            SchemaContractError::new(format!(
                "read canonical CREATE SQL for table {}: {error}",
                contract.name
            ))
        })?;
    let normalized_table_sql = normalize_sql(&table_sql).to_ascii_lowercase();
    for fragment in contract.required_sql_fragments {
        let normalized_fragment = normalize_sql(fragment).to_ascii_lowercase();
        if !normalized_table_sql.contains(&normalized_fragment) {
            return Err(SchemaContractError::new(format!(
                "table {} is missing required canonical SQL fragment: {}",
                contract.name, normalized_fragment
            )));
        }
    }

    let table_flags = connection
        .query_row(
            "SELECT wr, strict
               FROM pragma_table_list
              WHERE schema = 'main' AND name = ?1 AND type = 'table'",
            [contract.name],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|error| {
            SchemaContractError::new(format!(
                "inspect table flags for {}: {error}",
                contract.name
            ))
        })?
        .ok_or_else(|| {
            SchemaContractError::new(format!(
                "table {} is absent from pragma_table_list",
                contract.name
            ))
        })?;
    let expected_flags = (0, i64::from(contract.strict));
    if table_flags != expected_flags {
        return Err(SchemaContractError::new(format!(
            "table {} flags differ: expected wr/strict {expected_flags:?}, found {table_flags:?}",
            contract.name
        )));
    }

    let columns = load_columns(connection, contract.name)?;
    if columns.len() != contract.columns.len() {
        return Err(SchemaContractError::new(format!(
            "table {} column count differs: expected {}, found {} ({columns:?})",
            contract.name,
            contract.columns.len(),
            columns.len()
        )));
    }
    for (position, (actual, expected)) in columns.iter().zip(contract.columns).enumerate() {
        let expected_position = i64::try_from(position).expect("schema contract fits i64");
        let expected_type = Some(expected.declared_type);
        if actual.position != expected_position
            || actual.name != expected.name
            || actual.declared_type.as_deref() != expected_type
            || actual.not_null != expected.not_null
            || actual.primary_key_position != expected.primary_key_position
            || actual.hidden != 0
        {
            return Err(SchemaContractError::new(format!(
                "table {} column {position} differs: expected name={} type={} not_null={} pk={} hidden=0, found {actual:?}",
                contract.name,
                expected.name,
                expected.declared_type,
                expected.not_null,
                expected.primary_key_position
            )));
        }
    }

    let foreign_keys = load_foreign_keys(connection, contract.name)?;
    if foreign_keys.len() != contract.foreign_keys.len() {
        return Err(SchemaContractError::new(format!(
            "table {} foreign-key count differs: expected {}, found {} ({foreign_keys:?})",
            contract.name,
            contract.foreign_keys.len(),
            foreign_keys.len()
        )));
    }
    for (actual, expected) in foreign_keys.iter().zip(contract.foreign_keys) {
        if actual.id != expected.id
            || actual.sequence != expected.sequence
            || actual.parent_table != expected.parent_table
            || actual.from_column != expected.from_column
            || actual.to_column.as_deref() != Some(expected.to_column)
            || !actual.on_update.eq_ignore_ascii_case(expected.on_update)
            || !actual.on_delete.eq_ignore_ascii_case(expected.on_delete)
            || !actual.match_kind.eq_ignore_ascii_case(expected.match_kind)
        {
            return Err(SchemaContractError::new(format!(
                "table {} foreign-key topology differs: expected id={} seq={} {}({}) -> {} on_update={} on_delete={} match={}, found {actual:?}",
                contract.name,
                expected.id,
                expected.sequence,
                expected.from_column,
                expected.to_column,
                expected.parent_table,
                expected.on_update,
                expected.on_delete,
                expected.match_kind
            )));
        }
    }
    Ok(())
}

fn load_columns(
    connection: &Connection,
    table: &str,
) -> Result<Vec<ActualColumn>, SchemaContractError> {
    let mut statement = connection
        .prepare(
            "SELECT cid, name, type, \"notnull\", pk, hidden
               FROM pragma_table_xinfo(?1)
              ORDER BY cid",
        )
        .map_err(|error| {
            SchemaContractError::new(format!("prepare column manifest for {table}: {error}"))
        })?;
    let rows = statement
        .query_map([table], |row| {
            Ok(ActualColumn {
                position: row.get(0)?,
                name: row.get(1)?,
                declared_type: row.get(2)?,
                not_null: row.get::<_, i64>(3)? != 0,
                primary_key_position: row.get(4)?,
                hidden: row.get(5)?,
            })
        })
        .map_err(|error| {
            SchemaContractError::new(format!("query column manifest for {table}: {error}"))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            SchemaContractError::new(format!("read column manifest for {table}: {error}"))
        })?;
    Ok(rows)
}

fn load_foreign_keys(
    connection: &Connection,
    table: &str,
) -> Result<Vec<ActualForeignKey>, SchemaContractError> {
    let mut statement = connection
        .prepare(
            "SELECT id, seq, \"table\", \"from\", \"to\", on_update, on_delete, match
               FROM pragma_foreign_key_list(?1)
              ORDER BY id, seq",
        )
        .map_err(|error| {
            SchemaContractError::new(format!("prepare foreign-key manifest for {table}: {error}"))
        })?;
    let rows = statement
        .query_map([table], |row| {
            Ok(ActualForeignKey {
                id: row.get(0)?,
                sequence: row.get(1)?,
                parent_table: row.get(2)?,
                from_column: row.get(3)?,
                to_column: row.get(4)?,
                on_update: row.get(5)?,
                on_delete: row.get(6)?,
                match_kind: row.get(7)?,
            })
        })
        .map_err(|error| {
            SchemaContractError::new(format!("query foreign-key manifest for {table}: {error}"))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            SchemaContractError::new(format!("read foreign-key manifest for {table}: {error}"))
        })?;
    Ok(rows)
}

fn validate_index(
    connection: &Connection,
    contract: &IndexContract,
) -> Result<(), SchemaContractError> {
    let schema_row = connection
        .query_row(
            "SELECT tbl_name, sql
               FROM sqlite_schema
              WHERE type = 'index' AND name = ?1",
            [contract.name],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|error| {
            SchemaContractError::new(format!("inspect index {}: {error}", contract.name))
        })?
        .ok_or_else(|| {
            SchemaContractError::new(format!("schema-v5 requires index {}", contract.name))
        })?;
    if schema_row.0 != contract.table {
        return Err(SchemaContractError::new(format!(
            "index {} belongs to {}, expected {}",
            contract.name, schema_row.0, contract.table
        )));
    }
    let sql = schema_row.1.ok_or_else(|| {
        SchemaContractError::new(format!("runtime index {} has no CREATE SQL", contract.name))
    })?;

    let attributes = connection
        .query_row(
            "SELECT \"unique\", origin, partial
               FROM pragma_index_list(?1)
              WHERE name = ?2",
            params![contract.table, contract.name],
            |row| {
                Ok((
                    row.get::<_, i64>(0)? != 0,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? != 0,
                ))
            },
        )
        .optional()
        .map_err(|error| {
            SchemaContractError::new(format!(
                "inspect index attributes for {}: {error}",
                contract.name
            ))
        })?
        .ok_or_else(|| {
            SchemaContractError::new(format!(
                "index {} is absent from pragma_index_list({})",
                contract.name, contract.table
            ))
        })?;
    let expected_attributes = (contract.unique, "c".to_string(), contract.partial);
    if attributes != expected_attributes {
        return Err(SchemaContractError::new(format!(
            "index {} attributes differ: expected unique/origin/partial {expected_attributes:?}, found {attributes:?}",
            contract.name
        )));
    }

    let columns = load_index_columns(connection, contract.name)?;
    if columns.len() != contract.columns.len() {
        return Err(SchemaContractError::new(format!(
            "index {} key-column count differs: expected {}, found {} ({columns:?})",
            contract.name,
            contract.columns.len(),
            columns.len()
        )));
    }
    for (position, (actual, expected)) in columns.iter().zip(contract.columns).enumerate() {
        let expected_position = i64::try_from(position).expect("schema contract fits i64");
        if actual.sequence != expected_position
            || actual.column_id < 0
            || actual.name.as_deref() != Some(expected.name)
            || actual.descending != expected.descending
            || actual.collation.as_deref() != Some("BINARY")
        {
            return Err(SchemaContractError::new(format!(
                "index {} key column {position} differs: expected {} descending={} BINARY, found {actual:?}",
                contract.name, expected.name, expected.descending
            )));
        }
    }

    let actual_predicate = normalized_where_clause(&sql);
    if actual_predicate.as_deref() != contract.partial_predicate {
        return Err(SchemaContractError::new(format!(
            "index {} partial predicate differs: expected {:?}, found {:?}",
            contract.name, contract.partial_predicate, actual_predicate
        )));
    }
    Ok(())
}

fn load_index_columns(
    connection: &Connection,
    index: &str,
) -> Result<Vec<ActualIndexColumn>, SchemaContractError> {
    let mut statement = connection
        .prepare(
            "SELECT seqno, cid, name, \"desc\", coll
               FROM pragma_index_xinfo(?1)
              WHERE key = 1
              ORDER BY seqno",
        )
        .map_err(|error| {
            SchemaContractError::new(format!("prepare index manifest for {index}: {error}"))
        })?;
    let rows = statement
        .query_map([index], |row| {
            Ok(ActualIndexColumn {
                sequence: row.get(0)?,
                column_id: row.get(1)?,
                name: row.get(2)?,
                descending: row.get::<_, i64>(3)? != 0,
                collation: row.get(4)?,
            })
        })
        .map_err(|error| {
            SchemaContractError::new(format!("query index manifest for {index}: {error}"))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            SchemaContractError::new(format!("read index manifest for {index}: {error}"))
        })?;
    Ok(rows)
}

fn normalized_where_clause(sql: &str) -> Option<String> {
    let normalized = normalize_sql(sql).to_ascii_lowercase();
    normalized
        .find(" where ")
        .map(|offset| normalized[offset + " where ".len()..].to_string())
}

fn normalize_sql(sql: &str) -> String {
    sql.trim()
        .trim_end_matches(';')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
