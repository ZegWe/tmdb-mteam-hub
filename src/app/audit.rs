use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::AppState;
use crate::storage::operation_log_retention::OperationLogRetention;
use crate::storage::SqliteSubscriptionRepository;
use crate::subscription::repository::RepositoryError;
use crate::subscription::NewOperationLogEntry;

pub(crate) type AuditLogFuture =
    Pin<Box<dyn Future<Output = Result<(), RepositoryError>> + Send + 'static>>;

pub(crate) trait AuditLogPort: Send + Sync {
    fn append(&self, entry: NewOperationLogEntry) -> AuditLogFuture;
}

pub(crate) struct SqliteAuditLog {
    repository: Arc<SqliteSubscriptionRepository>,
    retention: OperationLogRetention,
}

impl SqliteAuditLog {
    pub(crate) fn new(
        repository: Arc<SqliteSubscriptionRepository>,
        retention: OperationLogRetention,
    ) -> Self {
        Self {
            repository,
            retention,
        }
    }
}

impl AuditLogPort for SqliteAuditLog {
    fn append(&self, entry: NewOperationLogEntry) -> AuditLogFuture {
        let append = self.repository.append_operation_log(entry, self.retention);
        Box::pin(async move { append.await.map(|_| ()) })
    }
}

pub(crate) struct OperationLogEvent<'a, S> {
    pub(crate) category: &'a str,
    pub(crate) action: &'a str,
    pub(crate) target_type: &'a str,
    pub(crate) target_id: Option<String>,
    pub(crate) target_title: Option<String>,
    pub(crate) status: &'a str,
    pub(crate) summary: S,
    pub(crate) error: Option<String>,
    pub(crate) related: Value,
}

pub(crate) fn operation_log_entry<S>(
    account_key: impl Into<String>,
    event: OperationLogEvent<'_, S>,
) -> NewOperationLogEntry
where
    S: Into<String>,
{
    NewOperationLogEntry {
        account_key: account_key.into(),
        created_at: unix_now_secs(),
        category: event.category.to_string(),
        action: event.action.to_string(),
        target_type: event.target_type.to_string(),
        target_id: event.target_id,
        target_title: event.target_title,
        status: event.status.to_string(),
        summary: event.summary.into(),
        error: event.error.filter(|error| !error.trim().is_empty()),
        related: event.related,
    }
}

pub(crate) async fn write_operation_log(state: &AppState, entry: NewOperationLogEntry) {
    if let Err(error) = state.audit_log.append(entry).await {
        tracing::warn!("operation log write failed: {error}");
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}
