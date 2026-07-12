use serde::Serialize;

use super::{CursorCodecError, ListCursorScope, OpaqueListCursor};
#[cfg(test)]
use crate::subscription::repository::ListSubscriptionsCommand;
use crate::subscription::repository::{SubscriptionListPage, SubscriptionSummary};
use crate::subscription::{
    SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
    SubscriptionMediaKind,
};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SubscriptionListResponse {
    items: Vec<SubscriptionSummaryDto>,
    next_cursor: Option<OpaqueListCursor>,
}

impl SubscriptionListResponse {
    #[cfg(test)]
    pub(crate) fn try_from_page(
        page: &SubscriptionListPage,
        command: &ListSubscriptionsCommand,
    ) -> Result<Self, CursorCodecError> {
        Self::try_from_page_with_scope(page, ListCursorScope::from_command(command))
    }

    pub(crate) fn try_from_page_with_scope(
        page: &SubscriptionListPage,
        scope: ListCursorScope,
    ) -> Result<Self, CursorCodecError> {
        Ok(Self {
            items: page
                .items
                .iter()
                .map(SubscriptionSummaryDto::from)
                .collect(),
            next_cursor: page
                .next_cursor
                .as_ref()
                .map(|cursor| OpaqueListCursor::encode(cursor, scope))
                .transpose()?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SubscriptionSummaryDto {
    subject_id: String,
    revision: u64,
    active: bool,
    inactive_at: Option<u64>,
    last_seen_snapshot_id: Option<String>,
    media_kind: &'static str,
    schedulable: bool,
    blocked_reason: Option<String>,
    lifecycle_state: &'static str,
    execution_state: &'static str,
    next_attempt_at: Option<u64>,
    retry_count: u32,
    max_retries: u32,
    retry_blocked: bool,
    force_eligible_once: bool,
    updated_at: u64,
    title: String,
    release_year: Option<u16>,
    poster_url: String,
    category_text: Option<String>,
    douban_sort_time: Option<u64>,
    attention_tags: Vec<&'static str>,
}

impl From<&SubscriptionSummary> for SubscriptionSummaryDto {
    fn from(summary: &SubscriptionSummary) -> Self {
        Self {
            subject_id: summary.head.key.subject_id.clone(),
            revision: summary.head.revision.value(),
            active: summary.head.active,
            inactive_at: summary.head.inactive_at,
            last_seen_snapshot_id: summary
                .head
                .last_seen_snapshot_id
                .as_ref()
                .map(|snapshot| snapshot.as_str().to_string()),
            media_kind: media_kind_label(summary.head.media_kind),
            schedulable: summary.head.schedulable,
            blocked_reason: summary
                .head
                .blocked_reason
                .as_ref()
                .map(|reason| reason.as_str().to_string()),
            lifecycle_state: lifecycle_label(summary.head.lifecycle_state),
            execution_state: execution_label(summary.head.execution_state),
            next_attempt_at: summary.head.next_attempt_at,
            retry_count: summary.head.retry_count,
            max_retries: summary.head.max_retries,
            retry_blocked: summary.head.retry_blocked,
            force_eligible_once: summary.head.force_eligible_once,
            updated_at: summary.head.updated_at,
            title: summary.projection.title.clone(),
            release_year: summary.projection.release_year,
            poster_url: summary.projection.poster_url.clone(),
            category_text: summary.projection.category_text.clone(),
            douban_sort_time: summary.projection.douban_sort_time,
            attention_tags: summary
                .attention_tags
                .iter()
                .copied()
                .map(attention_tag_label)
                .collect(),
        }
    }
}

const fn media_kind_label(value: SubscriptionMediaKind) -> &'static str {
    match value {
        SubscriptionMediaKind::Movie => "movie",
        SubscriptionMediaKind::Tv => "tv",
    }
}

const fn lifecycle_label(value: SubscriptionLifecycleState) -> &'static str {
    match value {
        SubscriptionLifecycleState::Queued => "queued",
        SubscriptionLifecycleState::Meta => "meta",
        SubscriptionLifecycleState::Searching => "searching",
        SubscriptionLifecycleState::Downloading => "downloading",
        SubscriptionLifecycleState::Linking => "linking",
        SubscriptionLifecycleState::Completed => "completed",
    }
}

const fn execution_label(value: SubscriptionExecutionState) -> &'static str {
    match value {
        SubscriptionExecutionState::Idle => "idle",
        SubscriptionExecutionState::Running => "running",
    }
}

const fn attention_tag_label(value: SubscriptionAttentionTag) -> &'static str {
    match value {
        SubscriptionAttentionTag::WaitingRelease => "waiting_release",
        SubscriptionAttentionTag::Failed => "failed",
        SubscriptionAttentionTag::RetryBlocked => "retry_blocked",
        SubscriptionAttentionTag::Skipped => "skipped",
        SubscriptionAttentionTag::NeedsReconciliation => "needs_reconciliation",
    }
}
