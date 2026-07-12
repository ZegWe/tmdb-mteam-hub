use std::future::Future;
use std::pin::Pin;

use super::repository::{
    ApplyCompleteSnapshotCommand, ApplyCompleteSnapshotResult, BeginPollCommand, BeginPollResult,
    ClaimDueCommand, ClaimDueResult, ClaimOneCommand, ClaimOneResult, ExtendExecutionLeaseCommand,
    ExtendExecutionLeaseResult, FailExecutionCommand, FailExecutionResult, FinishExecutionCommand,
    FinishExecutionResult, ListSubscriptionsCommand, PollSchedule, RecordIncompleteSnapshotCommand,
    RecordIncompleteSnapshotResult, RecordPollFailureCommand, RecordPollFailureResult,
    ReleaseExecutionCommand, ReleaseExecutionResult, RepositoryResult, SubscriptionDetail,
    SubscriptionHead, SubscriptionKey, SubscriptionListPage, UpdateSubscriptionDetailCommand,
    UpdateSubscriptionDetailResult,
};

pub(crate) type RepoFuture<T> = Pin<Box<dyn Future<Output = RepositoryResult<T>> + Send + 'static>>;

pub(crate) trait SubscriptionReadRepository: Send + Sync {
    fn get(&self, key: SubscriptionKey) -> RepoFuture<SubscriptionHead>;

    fn list_summaries(&self, command: ListSubscriptionsCommand)
        -> RepoFuture<SubscriptionListPage>;

    fn load_detail(&self, key: SubscriptionKey) -> RepoFuture<SubscriptionDetail>;
}

/// Optimistic mutation capability for JSON-owned subscription detail.
///
/// This port intentionally cannot mutate activity, lifecycle, retry, due, or
/// claim/lease controls. Those columns need their own task-specific commands so
/// a partial in-memory view cannot overwrite scheduler state.
pub(crate) trait SubscriptionMutationRepository: Send + Sync {
    fn update_detail(
        &self,
        command: UpdateSubscriptionDetailCommand,
    ) -> RepoFuture<UpdateSubscriptionDetailResult>;
}

/// Poll persistence capability.
///
/// `begin_poll` must first persist one open [`PollAttemptToken`](super::repository::PollAttemptToken).
/// Each terminal method must conditionally update that exact token and consume it in the same
/// transaction. A mismatched token, or any terminal call after the token was consumed, returns
/// [`RepositoryError::StalePoll`](super::repository::RepositoryError::StalePoll); an already
/// consumed token has `current: None`. Complete resets the consecutive failure count, while failure
/// and incomplete terminals increment it before deriving `next_poll_at` from their retry policy.
/// Complete and incomplete snapshots both apply their observed records with insert-only
/// [`NewRecordPolicy`](super::repository::NewRecordPolicy) defaults and non-destructive source
/// enrichment. Only a complete snapshot may deactivate missing rows, update success/complete
/// metadata, or mark the account bootstrap complete; a partial snapshot leaves all three unchanged.
pub(crate) trait SubscriptionPollRepository: Send + Sync {
    fn load_poll_schedule(&self, account_key: String) -> RepoFuture<PollSchedule>;

    fn begin_poll(&self, command: BeginPollCommand) -> RepoFuture<BeginPollResult>;

    fn apply_complete_snapshot(
        &self,
        command: ApplyCompleteSnapshotCommand,
    ) -> RepoFuture<ApplyCompleteSnapshotResult>;

    fn record_incomplete_snapshot(
        &self,
        command: RecordIncompleteSnapshotCommand,
    ) -> RepoFuture<RecordIncompleteSnapshotResult>;

    fn record_poll_failure(
        &self,
        command: RecordPollFailureCommand,
    ) -> RepoFuture<RecordPollFailureResult>;
}

/// Atomic scheduler-claim capability, independent from read, detail mutation,
/// and source-poll persistence.
///
/// Both methods classify leases from a repository-owned injected clock. The
/// caller supplies only validated relative durations; it cannot provide `now`
/// or an observed lease timestamp. Every post-claim command fences on the exact
/// key + operation + attempt ID and a repository-clock-live lease. Row revision
/// is returned for freshness but is never part of attempt identity.
pub(crate) trait SubscriptionExecutionRepository: Send + Sync {
    fn claim_due(&self, command: ClaimDueCommand) -> RepoFuture<ClaimDueResult>;

    fn claim_one(&self, command: ClaimOneCommand) -> RepoFuture<ClaimOneResult>;

    fn extend_lease(
        &self,
        command: ExtendExecutionLeaseCommand,
    ) -> RepoFuture<ExtendExecutionLeaseResult>;

    fn finish(&self, command: FinishExecutionCommand) -> RepoFuture<FinishExecutionResult>;

    fn fail(&self, command: FailExecutionCommand) -> RepoFuture<FailExecutionResult>;

    fn release(&self, command: ReleaseExecutionCommand) -> RepoFuture<ReleaseExecutionResult>;
}

#[cfg(test)]
mod tests {
    use super::{
        RepoFuture, SubscriptionExecutionRepository, SubscriptionMutationRepository,
        SubscriptionPollRepository, SubscriptionReadRepository,
    };
    use crate::subscription::repository::{
        ApplyCompleteSnapshotCommand, ApplyCompleteSnapshotResult, BeginPollCommand,
        BeginPollResult, ClaimDueCommand, ClaimDueResult, ClaimOneCommand, ClaimOneResult,
        ClaimRejection, ExecutionAttemptId, ExecutionAttemptToken, ExecutionOperation,
        ExecutionPayloadDelta, ExtendExecutionLeaseCommand, ExtendExecutionLeaseResult,
        FailExecutionCommand, FailExecutionResult, FinishExecutionCommand,
        FinishExecutionDisposition, FinishExecutionResult, IncompleteSnapshotObservation,
        IncompleteSnapshotReason, ListSubscriptionsCommand, NewRecordPolicy, PollAttemptToken,
        PollGeneration, PollRetryPolicy, PollSchedule, RecordIncompleteSnapshotCommand,
        RecordIncompleteSnapshotResult, RecordPollFailureCommand, RecordPollFailureResult,
        ReleaseExecutionCommand, ReleaseExecutionResult, RepositoryError, SnapshotId,
        SnapshotRecord, SubscriptionDetail, SubscriptionHead, SubscriptionKey,
        SubscriptionListPage, UpdateSubscriptionDetailCommand, UpdateSubscriptionDetailResult,
        WantedSourcePayload,
    };
    use crate::subscription::SubscriptionMediaKind;

    struct FakeRepository;

    impl SubscriptionReadRepository for FakeRepository {
        fn get(&self, key: SubscriptionKey) -> RepoFuture<SubscriptionHead> {
            Box::pin(async move { Err(RepositoryError::NotFound { key }) })
        }

        fn list_summaries(
            &self,
            _command: ListSubscriptionsCommand,
        ) -> RepoFuture<SubscriptionListPage> {
            unsupported()
        }

        fn load_detail(&self, _key: SubscriptionKey) -> RepoFuture<SubscriptionDetail> {
            unsupported()
        }
    }

    impl SubscriptionPollRepository for FakeRepository {
        fn load_poll_schedule(&self, _account_key: String) -> RepoFuture<PollSchedule> {
            Box::pin(async { Ok(PollSchedule::new(None)) })
        }

        fn begin_poll(&self, command: BeginPollCommand) -> RepoFuture<BeginPollResult> {
            Box::pin(async move {
                Ok(BeginPollResult {
                    token: PollAttemptToken::new(
                        PollGeneration::try_new(1)?,
                        SnapshotId::try_new("snapshot-1")?,
                    ),
                    attempted_at: command.attempted_at,
                })
            })
        }

        fn apply_complete_snapshot(
            &self,
            _command: ApplyCompleteSnapshotCommand,
        ) -> RepoFuture<ApplyCompleteSnapshotResult> {
            unsupported()
        }

        fn record_incomplete_snapshot(
            &self,
            command: RecordIncompleteSnapshotCommand,
        ) -> RepoFuture<RecordIncompleteSnapshotResult> {
            Box::pin(async move {
                let inserted = command.records.len();
                Ok(RecordIncompleteSnapshotResult {
                    token: command.token,
                    inserted,
                    updated: 0,
                    unchanged: 0,
                    reactivated: 0,
                    incomplete_at: command.incomplete_at,
                    failure_count: 1,
                    next_poll_at: command.retry_policy.next_poll_at(command.incomplete_at, 1),
                })
            })
        }

        fn record_poll_failure(
            &self,
            _command: RecordPollFailureCommand,
        ) -> RepoFuture<RecordPollFailureResult> {
            unsupported()
        }
    }

    impl SubscriptionMutationRepository for FakeRepository {
        fn update_detail(
            &self,
            command: UpdateSubscriptionDetailCommand,
        ) -> RepoFuture<UpdateSubscriptionDetailResult> {
            let key = command.key().clone();
            Box::pin(async move { Err(RepositoryError::NotFound { key }) })
        }
    }

    impl SubscriptionExecutionRepository for FakeRepository {
        fn claim_due(&self, _command: ClaimDueCommand) -> RepoFuture<ClaimDueResult> {
            Box::pin(async { Ok(ClaimDueResult::none_due()) })
        }

        fn claim_one(&self, _command: ClaimOneCommand) -> RepoFuture<ClaimOneResult> {
            Box::pin(async { Ok(ClaimOneResult::Rejected(ClaimRejection::RetryBlocked)) })
        }

        fn extend_lease(
            &self,
            _command: ExtendExecutionLeaseCommand,
        ) -> RepoFuture<ExtendExecutionLeaseResult> {
            unsupported()
        }

        fn finish(&self, _command: FinishExecutionCommand) -> RepoFuture<FinishExecutionResult> {
            unsupported()
        }

        fn fail(&self, _command: FailExecutionCommand) -> RepoFuture<FailExecutionResult> {
            unsupported()
        }

        fn release(&self, _command: ReleaseExecutionCommand) -> RepoFuture<ReleaseExecutionResult> {
            unsupported()
        }
    }

    fn unsupported<T>() -> RepoFuture<T> {
        Box::pin(async {
            Err(RepositoryError::Internal {
                message: "not implemented by fake".to_string(),
            })
        })
    }

    #[tokio::test]
    async fn read_capability_is_object_safe_and_returns_static_futures() {
        let repository: Box<dyn SubscriptionReadRepository> = Box::new(FakeRepository);
        let key = SubscriptionKey::try_new("account", "subject").unwrap();

        let pending = repository.get(key.clone());
        drop(repository);
        let error = pending.await.unwrap_err();

        assert_eq!(error, RepositoryError::NotFound { key });
    }

    #[tokio::test]
    async fn poll_capability_is_independently_object_safe() {
        let repository: Box<dyn SubscriptionPollRepository> = Box::new(FakeRepository);
        let command = BeginPollCommand::try_new("account", 42).unwrap();

        let pending = repository.begin_poll(command);
        drop(repository);
        let result = pending.await.unwrap();

        assert_eq!(result.attempted_at, 42);
        assert_eq!(result.token.generation.value(), 1);
        assert_eq!(result.token.snapshot_id.as_str(), "snapshot-1");
    }

    #[tokio::test]
    async fn poll_terminal_commands_with_new_record_policy_remain_object_safe() {
        let token = PollAttemptToken::new(
            PollGeneration::try_new(2).unwrap(),
            SnapshotId::try_new("snapshot-2").unwrap(),
        );
        let insert_policy = NewRecordPolicy::try_new(3, true).unwrap();

        let complete_repository: Box<dyn SubscriptionPollRepository> = Box::new(FakeRepository);
        let complete = ApplyCompleteSnapshotCommand::try_new(
            "account",
            token.clone(),
            42,
            102,
            insert_policy,
            Vec::new(),
        )
        .unwrap();
        let pending = complete_repository.apply_complete_snapshot(complete);
        drop(complete_repository);
        assert!(matches!(
            pending.await,
            Err(RepositoryError::Internal { .. })
        ));

        let incomplete_repository: Box<dyn SubscriptionPollRepository> = Box::new(FakeRepository);
        let incomplete = RecordIncompleteSnapshotCommand::try_new(
            "account",
            token,
            42,
            IncompleteSnapshotObservation::try_new(
                2,
                false,
                false,
                IncompleteSnapshotReason::RepeatedPage,
            )
            .unwrap(),
            insert_policy,
            vec![SnapshotRecord::try_new(
                "partial-subject",
                SubscriptionMediaKind::Movie,
                true,
                None,
                WantedSourcePayload {
                    title: "Partial subject".to_string(),
                    ..WantedSourcePayload::default()
                },
            )
            .unwrap()],
            PollRetryPolicy::try_new(5, 60).unwrap(),
        )
        .unwrap();
        let pending = incomplete_repository.record_incomplete_snapshot(incomplete);
        drop(incomplete_repository);
        let result = pending.await.unwrap();
        assert_eq!(result.inserted, 1);
        assert_eq!(result.updated, 0);
        assert_eq!(result.unchanged, 0);
        assert_eq!(result.reactivated, 0);
        assert_eq!(result.incomplete_at, 42);
        assert_eq!(result.failure_count, 1);
        assert_eq!(result.next_poll_at, 47);
    }

    #[tokio::test]
    async fn mutation_capability_is_object_safe_and_returns_static_futures() {
        let repository: Box<dyn SubscriptionMutationRepository> = Box::new(FakeRepository);
        let key = SubscriptionKey::try_new("account", "subject").unwrap();
        let command = UpdateSubscriptionDetailCommand::try_new(
            key.clone(),
            crate::subscription::repository::Revision::try_new(1).unwrap(),
            42,
            Vec::new(),
            valid_payload(),
        )
        .unwrap();

        let pending = repository.update_detail(command);
        drop(repository);
        let error = pending.await.unwrap_err();

        assert_eq!(error, RepositoryError::NotFound { key });
    }

    #[tokio::test]
    async fn execution_capability_is_independently_object_safe_with_owned_commands() {
        let due_repository: Box<dyn SubscriptionExecutionRepository> = Box::new(FakeRepository);
        let due = ClaimDueCommand::try_new("account", 60, 1).unwrap();
        let pending = due_repository.claim_due(due);
        drop(due_repository);
        assert!(pending.await.unwrap().claim().is_none());

        let one_repository: Box<dyn SubscriptionExecutionRepository> = Box::new(FakeRepository);
        let one =
            ClaimOneCommand::try_new(SubscriptionKey::try_new("account", "subject").unwrap(), 60)
                .unwrap();
        let pending = one_repository.claim_one(one);
        drop(one_repository);
        assert_eq!(
            pending.await.unwrap(),
            ClaimOneResult::Rejected(ClaimRejection::RetryBlocked)
        );

        let token = ExecutionAttemptToken::new(
            SubscriptionKey::try_new("account", "subject").unwrap(),
            ExecutionAttemptId::try_new("attempt-owned").unwrap(),
            ExecutionOperation::Meta,
        );

        let extend_repository: Box<dyn SubscriptionExecutionRepository> = Box::new(FakeRepository);
        let pending = extend_repository
            .extend_lease(ExtendExecutionLeaseCommand::try_new(token.clone(), 60).unwrap());
        drop(extend_repository);
        assert!(matches!(
            pending.await,
            Err(RepositoryError::Internal { .. })
        ));

        let finish_repository: Box<dyn SubscriptionExecutionRepository> = Box::new(FakeRepository);
        let pending = finish_repository.finish(
            FinishExecutionCommand::try_new(
                token.clone(),
                FinishExecutionDisposition::MetaReady,
                ExecutionPayloadDelta::Meta,
            )
            .unwrap(),
        );
        drop(finish_repository);
        assert!(matches!(
            pending.await,
            Err(RepositoryError::Internal { .. })
        ));

        let fail_repository: Box<dyn SubscriptionExecutionRepository> = Box::new(FakeRepository);
        let pending = fail_repository.fail(
            FailExecutionCommand::try_new(
                token.clone(),
                "system",
                "failure",
                5,
                ExecutionPayloadDelta::Meta,
            )
            .unwrap(),
        );
        drop(fail_repository);
        assert!(matches!(
            pending.await,
            Err(RepositoryError::Internal { .. })
        ));

        let release_repository: Box<dyn SubscriptionExecutionRepository> = Box::new(FakeRepository);
        let pending =
            release_repository.release(ReleaseExecutionCommand::before_external_effect(token));
        drop(release_repository);
        assert!(matches!(
            pending.await,
            Err(RepositoryError::Internal { .. })
        ));
    }

    fn valid_payload() -> crate::subscription::repository::SubscriptionPayload {
        use crate::subscription::repository::{SubscriptionPayload, WantedSourcePayload};

        SubscriptionPayload {
            source: WantedSourcePayload {
                title: "Title".to_string(),
                ..WantedSourcePayload::default()
            },
            observation: crate::subscription::repository::payload::ObservationPayload {
                created_at: 1,
                first_seen_at: 1,
                last_seen_at: 1,
            },
            ..SubscriptionPayload::default()
        }
    }
}
