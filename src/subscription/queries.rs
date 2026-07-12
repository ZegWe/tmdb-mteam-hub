use std::error::Error;
use std::fmt;
use std::sync::Arc;

use super::ports::SubscriptionReadRepository;
use super::repository::{
    ListCursor, ListSubscriptionsCommand, RepositoryError, SubscriptionDetail, SubscriptionKey,
    SubscriptionListFilter, SubscriptionListPage,
};

/// Owned application-query capability for schema-v5 subscription reads.
///
/// Runtime assembly deliberately remains separate until the v5 cutover. Keeping
/// the repository behind this service lets HTTP and worker adapters share one
/// query contract without coupling application code to SQLite.
#[derive(Clone)]
pub(crate) struct SubscriptionQueryService {
    read_repository: Arc<dyn SubscriptionReadRepository>,
}

impl SubscriptionQueryService {
    pub(crate) fn new(read_repository: Arc<dyn SubscriptionReadRepository>) -> Self {
        Self { read_repository }
    }

    pub(crate) async fn list_subscriptions(
        &self,
        command: ListSubscriptions,
    ) -> Result<SubscriptionListPage, SubscriptionQueryError> {
        self.read_repository
            .list_summaries(command.into_repository_command())
            .await
            .map_err(SubscriptionQueryError::from)
    }

    pub(crate) async fn get_subscription(
        &self,
        command: GetSubscription,
    ) -> Result<SubscriptionDetail, SubscriptionQueryError> {
        self.read_repository
            .load_detail(command.into_key())
            .await
            .map_err(SubscriptionQueryError::from)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ListSubscriptions {
    repository_command: ListSubscriptionsCommand,
}

impl ListSubscriptions {
    pub(crate) fn try_new(
        account_key: impl Into<String>,
        filter: SubscriptionListFilter,
        cursor: Option<ListCursor>,
        limit: u32,
    ) -> Result<Self, SubscriptionQueryError> {
        ListSubscriptionsCommand::try_new(account_key, filter, cursor, limit)
            .map(|repository_command| Self { repository_command })
            .map_err(SubscriptionQueryError::from)
    }

    fn into_repository_command(self) -> ListSubscriptionsCommand {
        self.repository_command
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GetSubscription {
    key: SubscriptionKey,
}

impl GetSubscription {
    pub(crate) fn try_new(
        account_key: impl Into<String>,
        subject_id: impl Into<String>,
    ) -> Result<Self, SubscriptionQueryError> {
        SubscriptionKey::try_new(account_key, subject_id)
            .map(|key| Self { key })
            .map_err(SubscriptionQueryError::from)
    }

    fn into_key(self) -> SubscriptionKey {
        self.key
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubscriptionQueryErrorKind {
    Validation,
    NotFound,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubscriptionQueryError {
    kind: SubscriptionQueryErrorKind,
    source: RepositoryError,
}

impl SubscriptionQueryError {
    pub(crate) const fn kind(&self) -> SubscriptionQueryErrorKind {
        self.kind
    }

    #[cfg(test)]
    pub(crate) const fn repository_error(&self) -> &RepositoryError {
        &self.source
    }
}

impl From<RepositoryError> for SubscriptionQueryError {
    fn from(source: RepositoryError) -> Self {
        let kind = match &source {
            RepositoryError::InvalidInput { .. } => SubscriptionQueryErrorKind::Validation,
            RepositoryError::NotFound { .. } => SubscriptionQueryErrorKind::NotFound,
            RepositoryError::UnsupportedSchema { .. } | RepositoryError::Unavailable { .. } => {
                SubscriptionQueryErrorKind::Unavailable
            }
            RepositoryError::RevisionConflict { .. }
            | RepositoryError::ExecutionGateConflict { .. }
            | RepositoryError::StaleAttempt { .. }
            | RepositoryError::LeaseExpired { .. }
            | RepositoryError::LeaseNotExtended { .. }
            | RepositoryError::StalePoll { .. }
            | RepositoryError::CorruptData { .. }
            | RepositoryError::Internal { .. } => SubscriptionQueryErrorKind::Internal,
        };
        Self { kind, source }
    }
}

impl fmt::Display for SubscriptionQueryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "subscription query failed: {}", self.source)
    }
}

impl Error for SubscriptionQueryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::{
        GetSubscription, ListSubscriptions, SubscriptionQueryError, SubscriptionQueryErrorKind,
        SubscriptionQueryService,
    };
    use crate::subscription::ports::{RepoFuture, SubscriptionReadRepository};
    use crate::subscription::repository::payload::{
        ObservationPayload, SubscriptionPayload, WantedSourcePayload,
    };
    use crate::subscription::repository::{
        ListCursor, ListSubscriptionsCommand, RepositoryError, RepositoryResult, Revision,
        SubscriptionDetail, SubscriptionHead, SubscriptionKey, SubscriptionListFilter,
        SubscriptionListPage, SubscriptionProjection, SubscriptionSummary,
    };
    use crate::subscription::{
        SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
        SubscriptionMediaKind,
    };

    const ACCOUNT: &str = "account-1";
    const SUBJECT: &str = "subject-1";

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum ReadCall {
        Get(SubscriptionKey),
        List(ListSubscriptionsCommand),
        LoadDetail(SubscriptionKey),
    }

    struct RecordingRepository {
        calls: Mutex<Vec<ReadCall>>,
        head_result: Mutex<Option<RepositoryResult<SubscriptionHead>>>,
        list_result: Mutex<Option<RepositoryResult<SubscriptionListPage>>>,
        detail_result: Mutex<Option<RepositoryResult<SubscriptionDetail>>>,
    }

    impl RecordingRepository {
        fn with_list(result: RepositoryResult<SubscriptionListPage>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                head_result: Mutex::new(Some(unexpected("get"))),
                list_result: Mutex::new(Some(result)),
                detail_result: Mutex::new(Some(unexpected("load_detail"))),
            }
        }

        fn with_detail(result: RepositoryResult<SubscriptionDetail>) -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                head_result: Mutex::new(Some(unexpected("get"))),
                list_result: Mutex::new(Some(unexpected("list_summaries"))),
                detail_result: Mutex::new(Some(result)),
            }
        }

        fn calls(&self) -> Vec<ReadCall> {
            self.calls.lock().expect("lock recorded calls").clone()
        }
    }

    impl SubscriptionReadRepository for RecordingRepository {
        fn get(&self, key: SubscriptionKey) -> RepoFuture<SubscriptionHead> {
            self.calls
                .lock()
                .expect("record get call")
                .push(ReadCall::Get(key));
            let result = self
                .head_result
                .lock()
                .expect("lock get result")
                .take()
                .expect("one configured get result");
            Box::pin(async move { result })
        }

        fn list_summaries(
            &self,
            command: ListSubscriptionsCommand,
        ) -> RepoFuture<SubscriptionListPage> {
            self.calls
                .lock()
                .expect("record list call")
                .push(ReadCall::List(command));
            let result = self
                .list_result
                .lock()
                .expect("lock list result")
                .take()
                .expect("one configured list result");
            Box::pin(async move { result })
        }

        fn load_detail(&self, key: SubscriptionKey) -> RepoFuture<SubscriptionDetail> {
            self.calls
                .lock()
                .expect("record detail call")
                .push(ReadCall::LoadDetail(key));
            let result = self
                .detail_result
                .lock()
                .expect("lock detail result")
                .take()
                .expect("one configured detail result");
            Box::pin(async move { result })
        }
    }

    #[tokio::test]
    async fn list_query_forwards_the_exact_repository_command_and_returns_its_page() {
        let cursor = ListCursor::try_new(Some(123), "subject-before").unwrap();
        let filter = SubscriptionListFilter {
            active: Some(true),
            media_kind: Some(SubscriptionMediaKind::Movie),
            lifecycle_state: Some(SubscriptionLifecycleState::Downloading),
            attention_tag: Some(SubscriptionAttentionTag::Failed),
        };
        let expected_command =
            ListSubscriptionsCommand::try_new(ACCOUNT, filter.clone(), Some(cursor.clone()), 25)
                .unwrap();
        let expected_page = list_page();
        let repository = Arc::new(RecordingRepository::with_list(Ok(expected_page.clone())));
        let service = SubscriptionQueryService::new(repository.clone());

        let result = service
            .list_subscriptions(
                ListSubscriptions::try_new(ACCOUNT, filter, Some(cursor), 25).unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(result, expected_page);
        assert_eq!(repository.calls(), vec![ReadCall::List(expected_command)]);
    }

    #[tokio::test]
    async fn detail_query_uses_only_load_detail_and_forwards_the_exact_key() {
        let expected_key = SubscriptionKey::try_new(ACCOUNT, SUBJECT).unwrap();
        let expected_detail = detail();
        let repository = Arc::new(RecordingRepository::with_detail(
            Ok(expected_detail.clone()),
        ));
        let service = SubscriptionQueryService::new(repository.clone());

        let result = service
            .get_subscription(GetSubscription::try_new(ACCOUNT, SUBJECT).unwrap())
            .await
            .unwrap();

        assert_eq!(result, expected_detail);
        assert_eq!(repository.calls(), vec![ReadCall::LoadDetail(expected_key)]);
    }

    #[test]
    fn application_commands_map_validation_failures_before_repository_access() {
        let list_error =
            ListSubscriptions::try_new(ACCOUNT, SubscriptionListFilter::default(), None, 0)
                .unwrap_err();
        assert_query_error(
            &list_error,
            SubscriptionQueryErrorKind::Validation,
            &RepositoryError::InvalidInput {
                field: "limit",
                message: "list limit must be between 1 and 100".to_string(),
            },
        );

        let get_error = GetSubscription::try_new(ACCOUNT, " ").unwrap_err();
        assert_query_error(
            &get_error,
            SubscriptionQueryErrorKind::Validation,
            &RepositoryError::InvalidInput {
                field: "subject_id",
                message: "value must not be blank".to_string(),
            },
        );
    }

    #[tokio::test]
    async fn missing_detail_maps_to_typed_not_found() {
        let key = SubscriptionKey::try_new(ACCOUNT, SUBJECT).unwrap();
        let source = RepositoryError::NotFound { key: key.clone() };
        let repository = Arc::new(RecordingRepository::with_detail(Err(source.clone())));
        let service = SubscriptionQueryService::new(repository);

        let error = service
            .get_subscription(GetSubscription::try_new(ACCOUNT, SUBJECT).unwrap())
            .await
            .unwrap_err();

        assert_query_error(&error, SubscriptionQueryErrorKind::NotFound, &source);
    }

    #[tokio::test]
    async fn unavailable_and_unsupported_schema_errors_share_the_unavailable_class() {
        for source in [
            RepositoryError::Unavailable {
                message: "database busy".to_string(),
            },
            RepositoryError::UnsupportedSchema {
                found: 4,
                maximum_supported: 5,
            },
        ] {
            let error = run_list_error(source.clone()).await;
            assert_query_error(&error, SubscriptionQueryErrorKind::Unavailable, &source);
        }
    }

    #[tokio::test]
    async fn corruption_and_internal_repository_failures_share_the_internal_class() {
        for source in [
            RepositoryError::CorruptData {
                message: "projection drift".to_string(),
            },
            RepositoryError::Internal {
                message: "unexpected adapter failure".to_string(),
            },
        ] {
            let error = run_list_error(source.clone()).await;
            assert_query_error(&error, SubscriptionQueryErrorKind::Internal, &source);
        }
    }

    async fn run_list_error(source: RepositoryError) -> SubscriptionQueryError {
        let repository = Arc::new(RecordingRepository::with_list(Err(source)));
        SubscriptionQueryService::new(repository)
            .list_subscriptions(
                ListSubscriptions::try_new(ACCOUNT, SubscriptionListFilter::default(), None, 10)
                    .unwrap(),
            )
            .await
            .unwrap_err()
    }

    fn assert_query_error(
        actual: &SubscriptionQueryError,
        kind: SubscriptionQueryErrorKind,
        source: &RepositoryError,
    ) {
        assert_eq!(actual.kind(), kind);
        assert_eq!(actual.repository_error(), source);
    }

    fn unexpected<T>(operation: &str) -> RepositoryResult<T> {
        Err(RepositoryError::Internal {
            message: format!("unexpected fake repository call to {operation}"),
        })
    }

    fn list_page() -> SubscriptionListPage {
        let detail = detail();
        SubscriptionListPage {
            items: vec![detail.summary().clone()],
            next_cursor: Some(ListCursor::try_new(Some(90), "subject-next").unwrap()),
        }
    }

    fn detail() -> SubscriptionDetail {
        let source = WantedSourcePayload {
            title: "Fixture Movie".to_string(),
            release_year: Some(2026),
            poster_url: "https://images.test/poster.jpg".to_string(),
            category_text: Some("movie".to_string()),
            douban_sort_time: Some(100),
            ..WantedSourcePayload::default()
        };
        let summary = SubscriptionSummary {
            head: SubscriptionHead {
                key: SubscriptionKey::try_new(ACCOUNT, SUBJECT).unwrap(),
                revision: Revision::try_new(3).unwrap(),
                active: true,
                inactive_at: None,
                last_seen_snapshot_id: None,
                media_kind: SubscriptionMediaKind::Movie,
                schedulable: true,
                blocked_reason: None,
                lifecycle_state: SubscriptionLifecycleState::Downloading,
                execution_state: SubscriptionExecutionState::Idle,
                next_attempt_at: Some(50),
                retry_count: 1,
                max_retries: 5,
                retry_blocked: false,
                force_eligible_once: false,
                updated_at: 40,
            },
            projection: SubscriptionProjection::from_source(&source).unwrap(),
            attention_tags: vec![SubscriptionAttentionTag::Failed],
        };
        let payload = SubscriptionPayload {
            source,
            observation: ObservationPayload {
                created_at: 10,
                first_seen_at: 20,
                last_seen_at: 30,
            },
            ..SubscriptionPayload::default()
        };
        SubscriptionDetail::try_new(summary, payload).unwrap()
    }
}
