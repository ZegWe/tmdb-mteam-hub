use serde::Serialize;

use crate::subscription::worker::SubscriptionPollOutcome;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SubscriptionPollOutcomeDto {
    snapshot_id: String,
    snapshot_complete: bool,
    fetched_items: usize,
    inserted: usize,
    updated: usize,
    unchanged: usize,
    reactivated: usize,
    deactivated: usize,
    failure_count: u32,
    next_poll_at: u64,
    polled_at: u64,
}

impl From<SubscriptionPollOutcome> for SubscriptionPollOutcomeDto {
    fn from(value: SubscriptionPollOutcome) -> Self {
        Self {
            snapshot_id: value.snapshot_id,
            snapshot_complete: value.snapshot_complete,
            fetched_items: value.fetched_items,
            inserted: value.inserted,
            updated: value.updated,
            unchanged: value.unchanged,
            reactivated: value.reactivated,
            deactivated: value.deactivated,
            failure_count: value.failure_count,
            next_poll_at: value.next_poll_at,
            polled_at: value.polled_at,
        }
    }
}
