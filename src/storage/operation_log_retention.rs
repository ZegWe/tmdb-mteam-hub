use rusqlite::{params, Connection};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OperationLogRetention {
    pub(crate) max_age_secs: Option<u64>,
    pub(crate) max_rows_per_account: Option<u64>,
}

impl OperationLogRetention {
    pub(crate) fn from_limits(max_age_secs: u64, max_rows_per_account: u64) -> Self {
        Self {
            max_age_secs: (max_age_secs > 0).then_some(max_age_secs),
            max_rows_per_account: (max_rows_per_account > 0).then_some(max_rows_per_account),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct OperationLogCleanupReport {
    pub(crate) deleted_by_age: u64,
    pub(crate) deleted_by_count: u64,
}

pub(crate) fn cleanup_operation_logs_for_account(
    connection: &Connection,
    account_key: &str,
    now: u64,
    retention: OperationLogRetention,
) -> rusqlite::Result<OperationLogCleanupReport> {
    let mut report = OperationLogCleanupReport::default();
    if let Some(max_age_secs) = retention.max_age_secs {
        let cutoff = now.saturating_sub(max_age_secs);
        report.deleted_by_age = connection.execute(
            "DELETE FROM operation_logs
             WHERE account_key = ?1 AND created_at < ?2",
            params![account_key, sqlite_integer(cutoff)],
        )? as u64;
    }
    if let Some(max_rows_per_account) = retention.max_rows_per_account {
        report.deleted_by_count = connection.execute(
            "DELETE FROM operation_logs
             WHERE account_key = ?1
               AND id NOT IN (
                   SELECT id
                   FROM operation_logs
                   WHERE account_key = ?1
                   ORDER BY created_at DESC, id DESC
                   LIMIT ?2
               )",
            params![account_key, sqlite_integer(max_rows_per_account)],
        )? as u64;
    }
    Ok(report)
}

fn sqlite_integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_limits_disable_each_retention_dimension_independently() {
        assert_eq!(
            OperationLogRetention::from_limits(0, 0),
            OperationLogRetention::default()
        );
        assert_eq!(
            OperationLogRetention::from_limits(60, 0),
            OperationLogRetention {
                max_age_secs: Some(60),
                max_rows_per_account: None,
            }
        );
        assert_eq!(
            OperationLogRetention::from_limits(0, 100),
            OperationLogRetention {
                max_age_secs: None,
                max_rows_per_account: Some(100),
            }
        );
    }
}
