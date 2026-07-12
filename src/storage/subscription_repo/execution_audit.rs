use rusqlite::{params, Transaction};
use serde_json::Value;

use super::{command_integer, map_write_error};
use crate::subscription::repository::{RepositoryError, RepositoryResult};

pub(super) const EXECUTION_AUDIT_CATEGORY: &str = "subscription_scheduler";
pub(super) const EXECUTION_AUDIT_STATUS: &str = "success";

pub(super) struct ExecutionAuditEntry {
    pub(super) account_key: String,
    pub(super) created_at: u64,
    pub(super) action: &'static str,
    pub(super) target_id: String,
    pub(super) target_title: String,
    pub(super) summary: &'static str,
    pub(super) related: Value,
}

pub(super) fn append_execution_audit(
    transaction: &Transaction<'_>,
    entry: ExecutionAuditEntry,
) -> RepositoryResult<()> {
    let created_at = command_integer("execution_audit.created_at", entry.created_at)?;
    let related_json =
        serde_json::to_string(&entry.related).map_err(|error| RepositoryError::Internal {
            message: format!("encode execution fencing audit: {error}"),
        })?;
    let changed = transaction
        .execute(
            r#"INSERT INTO operation_logs (
                   account_key, created_at, category, action, target_type, target_id,
                   target_title, status, summary, error, related_json
               ) VALUES (?1, ?2, ?3, ?4, 'subscription', ?5, ?6, ?7, ?8, NULL, ?9)"#,
            params![
                entry.account_key,
                created_at,
                EXECUTION_AUDIT_CATEGORY,
                entry.action,
                entry.target_id,
                entry.target_title,
                EXECUTION_AUDIT_STATUS,
                entry.summary,
                related_json,
            ],
        )
        .map_err(|error| map_write_error("append execution fencing audit", error))?;
    if changed != 1 {
        return Err(RepositoryError::Internal {
            message: format!("execution fencing audit inserted {changed} rows, expected one"),
        });
    }
    Ok(())
}
