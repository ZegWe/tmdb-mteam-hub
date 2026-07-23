#[allow(
    dead_code,
    reason = "external-effect adapters are staged before execution service injection"
)]
pub(crate) mod effect_adapters;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "external-effect domain and port contracts precede runtime adapter injection"
    )
)]
pub(crate) mod effects;
pub(crate) mod episode;
pub(crate) mod execution;
pub(crate) mod execution_effects;
mod model;
mod operation_logs;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "execution capabilities remain staged after the latest read/Poll runtime cutover"
    )
)]
pub(crate) mod ports;
pub(crate) mod queries;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "execution commands remain staged after the latest read/Poll runtime cutover"
    )
)]
pub(crate) mod repository;
pub(crate) mod wanted_source;
pub(crate) mod worker;

pub use model::{
    SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
    SubscriptionMediaKind,
};
pub use operation_logs::{
    NewOperationLogEntry, OperationLogEntry, OperationLogPage, OperationLogQuery,
};

pub(crate) const INACTIVE_SUBSCRIPTION_REASON: &str = "subscription_inactive";
