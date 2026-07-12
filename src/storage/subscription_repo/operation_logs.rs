use rusqlite::{params, params_from_iter, Connection, Row, TransactionBehavior};

use super::{checked_u64, command_integer, SqliteSubscriptionRepository};
use crate::storage::operation_log_retention::{
    cleanup_operation_logs_for_account, OperationLogRetention,
};
use crate::storage::sqlite::{map_read_error, map_write_error};
use crate::subscription::ports::RepoFuture;
use crate::subscription::repository::{RepositoryError, RepositoryResult};
use crate::subscription::{
    NewOperationLogEntry, OperationLogEntry, OperationLogPage, OperationLogQuery,
};

impl SqliteSubscriptionRepository {
    pub(crate) fn append_operation_log(
        &self,
        entry: NewOperationLogEntry,
        retention: OperationLogRetention,
    ) -> RepoFuture<OperationLogEntry> {
        self.executor
            .run(move |connection| append_operation_log(connection, entry, retention))
    }

    pub(crate) fn query_operation_logs(
        &self,
        query: OperationLogQuery,
    ) -> RepoFuture<OperationLogPage> {
        self.executor
            .run(move |connection| query_operation_logs(connection, query))
    }
}

fn append_operation_log(
    connection: &mut Connection,
    entry: NewOperationLogEntry,
    retention: OperationLogRetention,
) -> RepositoryResult<OperationLogEntry> {
    let related_json =
        serde_json::to_string(&entry.related).map_err(|error| RepositoryError::Internal {
            message: format!("encode operation log related payload: {error}"),
        })?;
    let created_at = command_integer("operation_log.created_at", entry.created_at)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| map_write_error("begin operation log insert", error))?;
    transaction
        .execute(
            r#"INSERT INTO operation_logs (
                   account_key, created_at, category, action, target_type, target_id,
                   target_title, status, summary, error, related_json
               ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"#,
            params![
                entry.account_key,
                created_at,
                entry.category,
                entry.action,
                entry.target_type,
                entry.target_id,
                entry.target_title,
                entry.status,
                entry.summary,
                entry.error,
                related_json,
            ],
        )
        .map_err(|error| map_write_error("insert operation log", error))?;
    let inserted = load_operation_log_by_id(&transaction, transaction.last_insert_rowid())?;
    let cleanup = cleanup_operation_logs_for_account(
        &transaction,
        &inserted.account_key,
        inserted.created_at,
        retention,
    )
    .map_err(|error| map_write_error("apply operation log retention", error))?;
    transaction
        .commit()
        .map_err(|error| map_write_error("commit operation log insert", error))?;
    if cleanup.deleted_by_age > 0 || cleanup.deleted_by_count > 0 {
        tracing::info!(
            deleted_by_age = cleanup.deleted_by_age,
            deleted_by_count = cleanup.deleted_by_count,
            "operation log retention cleanup completed"
        );
    }
    Ok(inserted)
}

fn query_operation_logs(
    connection: &mut Connection,
    query: OperationLogQuery,
) -> RepositoryResult<OperationLogPage> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(30).clamp(1, 100);
    let offset = u64::from(page.saturating_sub(1)) * u64::from(page_size);
    let mut filters = String::new();
    let mut values = Vec::<String>::new();

    append_exact_filter(
        &mut filters,
        &mut values,
        "account_key",
        query.account_key,
        false,
    );
    append_exact_filter(&mut filters, &mut values, "category", query.category, true);
    append_exact_filter(&mut filters, &mut values, "status", query.status, true);
    if let Some(query) = query.q.map(|value| value.trim().to_string()) {
        if !query.is_empty() {
            let pattern = format!("%{query}%");
            filters.push_str(
                " AND (summary LIKE ? OR action LIKE ? OR target_id LIKE ? OR target_title LIKE ? OR error LIKE ?)",
            );
            values.extend([
                pattern.clone(),
                pattern.clone(),
                pattern.clone(),
                pattern.clone(),
                pattern,
            ]);
        }
    }

    let count_sql = format!("SELECT COUNT(*) FROM operation_logs WHERE 1=1{filters}");
    let total = connection
        .query_row(&count_sql, params_from_iter(values.iter()), |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|error| map_read_error("count operation logs", error))
        .and_then(|value| checked_u64("operation_logs.total", value))?;

    let list_sql = format!(
        "SELECT id, account_key, created_at, category, action, target_type, target_id, \
                target_title, status, summary, error, related_json \
           FROM operation_logs \
          WHERE 1=1{filters} \
          ORDER BY created_at DESC, id DESC \
          LIMIT {page_size} OFFSET {offset}"
    );
    let mut statement = connection
        .prepare(&list_sql)
        .map_err(|error| map_read_error("prepare operation log list", error))?;
    let items = statement
        .query_map(params_from_iter(values.iter()), read_operation_log)
        .map_err(|error| map_read_error("query operation log list", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| map_read_error("decode operation log list", error))?
        .into_iter()
        .map(RawOperationLog::try_into_entry)
        .collect::<RepositoryResult<Vec<_>>>()?;
    let shown = offset.saturating_add(items.len() as u64);
    Ok(OperationLogPage {
        items,
        page,
        page_size,
        total,
        has_more: shown < total,
    })
}

fn append_exact_filter(
    filters: &mut String,
    values: &mut Vec<String>,
    column: &str,
    value: Option<String>,
    accepts_all: bool,
) {
    let Some(value) = value.map(|value| value.trim().to_string()) else {
        return;
    };
    if value.is_empty() || (accepts_all && value == "all") {
        return;
    }
    filters.push_str(" AND ");
    filters.push_str(column);
    filters.push_str(" = ?");
    values.push(value);
}

fn load_operation_log_by_id(
    connection: &Connection,
    id: i64,
) -> RepositoryResult<OperationLogEntry> {
    let raw = connection
        .query_row(
            r#"SELECT id, account_key, created_at, category, action, target_type, target_id,
                      target_title, status, summary, error, related_json
                 FROM operation_logs WHERE id = ?1"#,
            [id],
            read_operation_log,
        )
        .map_err(|error| map_read_error("reload inserted operation log", error))?;
    raw.try_into_entry()
}

struct RawOperationLog {
    id: i64,
    account_key: String,
    created_at: i64,
    category: String,
    action: String,
    target_type: String,
    target_id: Option<String>,
    target_title: Option<String>,
    status: String,
    summary: String,
    error: Option<String>,
    related_json: String,
}

impl RawOperationLog {
    fn try_into_entry(self) -> RepositoryResult<OperationLogEntry> {
        let related = serde_json::from_str(&self.related_json).map_err(|error| {
            RepositoryError::CorruptData {
                message: format!("decode operation log related_json: {error}"),
            }
        })?;
        Ok(OperationLogEntry {
            id: checked_u64("operation_log.id", self.id)?,
            account_key: self.account_key,
            created_at: checked_u64("operation_log.created_at", self.created_at)?,
            category: self.category,
            action: self.action,
            target_type: self.target_type,
            target_id: self.target_id,
            target_title: self.target_title,
            status: self.status,
            summary: self.summary,
            error: self.error,
            related,
        })
    }
}

fn read_operation_log(row: &Row<'_>) -> rusqlite::Result<RawOperationLog> {
    Ok(RawOperationLog {
        id: row.get(0)?,
        account_key: row.get(1)?,
        created_at: row.get(2)?,
        category: row.get(3)?,
        action: row.get(4)?,
        target_type: row.get(5)?,
        target_id: row.get(6)?,
        target_title: row.get(7)?,
        status: row.get(8)?,
        summary: row.get(9)?,
        error: row.get(10)?,
        related_json: row.get(11)?,
    })
}
