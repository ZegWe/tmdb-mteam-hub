---
---

status: superseded
owner: tmdb-mteam-hub
last_verified: 2026-07-11
implementation_status: superseded
archived_at: 2026-07-11
authoritative: false
executable: false
spec: docs/archive/superpowers/specs/2026-07-08-subscription-state-convergence-prd.md
superseded_by:

- docs/superpowers/plans/2026-07-11-subscription-storage-scheduler.md
- docs/superpowers/plans/2026-07-11-backend-application-boundaries.md
  related_adr: docs/adr/0002-subscription-state-convergence.md
  tracking_note: historical checklist only; migration and legacy compatibility tasks are obsolete

---

# Subscription State Convergence Implementation Plan

> 历史归档：本文档的 migration、legacy compatibility、任务、命令和复选框均不可执行。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Converge wanted-subscription runtime, API, data shape, and frontend display onto `lifecycle_state`, `execution_state`, `attention_tags`, `failure`, `next_attempt_at`, and TV lanes as the only authoritative state model.

**Architecture:** Keep operation artifact statuses (`last_push.status`, `last_completion.status`, episode/file status, operation log status) as detail-only fields, but remove legacy subscription status/stage from runtime decisions and API/frontend primary state. Automatic ticks select exactly one due operation through `select_due_operation(record, now)` and execute semantic handlers for movie and TV operations. A one-time migration may read old `status`/`processing_stage`; after migration, production repair, tick, manual commands, serializers, and frontend helpers must not derive state from them.

**Tech Stack:** Rust 2021, Axum 0.8, Tokio, rusqlite, serde/serde_json, Vue 3, Vite Plus, Node test scripts, Cargo tests.

---

## File Structure

- Modify `src/subscription.rs`
  - Remove `WantedSubscriptionStatus`, `WantedStatusUpdate`, `WantedStatusUpdateOutcome`, `processing_stage`, and legacy stage/status derivation from the runtime record model.
  - Add explicit state transition helpers for movie operation outcomes, parent failures, waiting release, retry unblock, skipped records, rerun resets, and one-time legacy migration.
  - Keep `TorrentPushRecord.status`, `HardlinkCompletionRecord.status`, episode/file status fields, and `OperationLogEntry.status`.
  - Keep and harden `select_due_operation()`, `select_due_tv_lane()`, `derive_tv_parent_lifecycle()`, and index fields.
- Modify `src/main.rs`
  - Replace `SubscriptionRetryAction`/pipeline action selection with `SubscriptionDueOperation`.
  - Remove `/api/subscriptions/wanted/{id}/status`.
  - Make `retry-current` and `rerun` semantic commands over the new state model.
  - Route existing push/progress/completion external protocol code through new store transition methods.
- Modify `frontend/src/App.vue`
  - Rewrite primary display helpers to read only `lifecycle_state`, `attention_tags`, `failure`, and `execution_state`.
  - Remove `SUB_LEGACY_LIFECYCLE_BY_STATUS` and legacy progress/stage inference.
  - Remove manual “刷新下载进度” and “检查完成并硬链接” controls from the detail action surface.
- Modify `frontend/src/components/SubscriptionDetailView.vue`
  - Keep artifact detail rows for `last_push` and `last_completion`.
  - Ensure the primary state strip is driven by `subscriptionLifecycleNodes()`.
- Modify frontend tests under `frontend/src/__tests__/`
  - Add source-guard tests that fail if primary state helpers read `record.status` or `record.processing_stage`.
  - Update card/detail expectations to new state fields.
- Modify or add Rust tests in `src/subscription.rs` and `src/main.rs`
  - Cover movie state flow, waiting release, progress interval, link failure retry, TV lane priority/isolation, API shape, route removal, and migration.

---

### Task 1: Lock Down New-State Selection Semantics

**Files:**

- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing tests for due operation selection**

Add tests near existing subscription state tests:

```rust
#[test]
fn movie_due_operation_follows_lifecycle_only() {
    let cases = [
        (SubscriptionLifecycleState::Queued, Some(SubscriptionDueOperation::MovieMeta)),
        (SubscriptionLifecycleState::Meta, Some(SubscriptionDueOperation::MovieMeta)),
        (SubscriptionLifecycleState::Searching, Some(SubscriptionDueOperation::MovieSearch)),
        (SubscriptionLifecycleState::Downloading, Some(SubscriptionDueOperation::MovieProgress)),
        (SubscriptionLifecycleState::Linking, Some(SubscriptionDueOperation::MovieLink)),
        (SubscriptionLifecycleState::Completed, None),
    ];

    for (state, expected) in cases {
        let mut record = movie_record_in_state(state, 1_000);
        record.next_attempt_at = Some(1_000);
        assert_eq!(select_due_operation(&record, 1_000), expected);
    }
}

#[test]
fn skipped_tag_blocks_automatic_due_unless_forced() {
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Searching, 1_000);
    record.attention_tags = vec![SubscriptionAttentionTag::Skipped];
    record.next_attempt_at = Some(1_000);
    assert_eq!(select_due_operation(&record, 1_000), None);

    record.force_eligible_once = true;
    assert_eq!(
        select_due_operation(&record, 1_000),
        Some(SubscriptionDueOperation::MovieSearch)
    );
}

#[test]
fn tv_lane_failure_does_not_block_other_due_lanes() {
    let mut record = tv_record_all_lanes_due(1_000);
    let tv = record.tv.as_mut().unwrap();
    tv.lanes.link.next_attempt_at = Some(2_000);
    tv.lanes.progress.failure = Some(test_failure("progress", false));
    tv.lanes.progress.next_attempt_at = Some(1_000);
    tv.lanes.search.next_attempt_at = Some(1_000);

    assert_eq!(
        select_due_operation(&record, 1_000),
        Some(SubscriptionDueOperation::TvLane(TvLaneKind::Progress))
    );
}
```

- [ ] **Step 2: Run tests and verify they fail only where current behavior is incomplete**

Run: `cargo test subscription::tests::movie_due_operation_follows_lifecycle_only subscription::tests::skipped_tag_blocks_automatic_due_unless_forced subscription::tests::tv_lane_failure_does_not_block_other_due_lanes`

Expected before implementation: failures if helper types lack `Debug`/visibility or if forced/skipped/lane behavior is incomplete.

- [ ] **Step 3: Make due operation types testable and lifecycle-only**

In `src/subscription.rs`, keep `select_due_operation()` independent of `status` and `processing_stage`. Ensure:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionDueOperation {
    MovieMeta,
    MovieSearch,
    MovieProgress,
    MovieLink,
    TvMeta,
    TvLane(TvLaneKind),
}
```

Do not add any branch that checks `WantedSubscriptionStatus` or `processing_stage`.

- [ ] **Step 4: Run tests**

Run: `cargo test subscription::tests::movie_due_operation_follows_lifecycle_only subscription::tests::skipped_tag_blocks_automatic_due_unless_forced subscription::tests::tv_lane_failure_does_not_block_other_due_lanes`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/subscription.rs
git commit -m "test: lock lifecycle due operation selection"
```

### Task 2: Replace Movie Runtime Transitions

**Files:**

- Modify: `src/subscription.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing movie transition tests**

Add tests in `src/subscription.rs`:

```rust
#[test]
fn movie_search_waiting_release_keeps_searching_without_retry_churn() {
    let cfg = test_watcher_cfg();
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Searching, 1_000);
    record.retry_count = 2;

    apply_movie_waiting_release(&mut record, "未搜索到候选种子", &cfg, 2_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Searching);
    assert!(record.attention_tags.contains(&SubscriptionAttentionTag::WaitingRelease));
    assert_eq!(record.retry_count, 2);
    assert_eq!(record.next_attempt_at, Some(2_000 + cfg.search_interval_secs));
    assert!(record.failure.is_none());
}

#[test]
fn movie_download_complete_moves_to_linking_not_completed() {
    let cfg = test_watcher_cfg();
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Downloading, 1_000);

    apply_movie_operation_outcome(
        &mut record,
        MovieOperationOutcome::Advanced(SubscriptionLifecycleState::Linking),
        &cfg,
        2_000,
    );

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Linking);
    assert_eq!(record.next_attempt_at, Some(2_000));
}

#[test]
fn movie_link_failure_stays_linking_with_retry_due_time() {
    let cfg = test_watcher_cfg();
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Linking, 1_000);

    apply_parent_operation_failure(
        &mut record,
        "link",
        "hardlink failed",
        &cfg,
        2_000,
    );

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Linking);
    assert!(record.attention_tags.contains(&SubscriptionAttentionTag::Failed));
    assert_eq!(record.failure.as_ref().unwrap().operation, "link");
    assert_eq!(record.next_attempt_at, Some(2_000 + cfg.link_retry_interval_secs));
}
```

- [ ] **Step 2: Run tests and confirm missing helpers fail**

Run: `cargo test subscription::tests::movie_search_waiting_release_keeps_searching_without_retry_churn subscription::tests::movie_download_complete_moves_to_linking_not_completed subscription::tests::movie_link_failure_stays_linking_with_retry_due_time`

Expected: FAIL because `apply_movie_waiting_release()` and `apply_parent_operation_failure()` do not exist yet, and `apply_movie_operation_outcome()` still derives legacy status.

- [ ] **Step 3: Implement lifecycle transition helpers**

In `src/subscription.rs`, add helpers:

```rust
pub fn apply_movie_waiting_release(
    record: &mut WantedSubscriptionRecord,
    message: &str,
    cfg: &SubscriptionWatcherConfig,
    now: u64,
) {
    record.lifecycle_state = SubscriptionLifecycleState::Searching;
    record.execution_state = SubscriptionExecutionState::Idle;
    record.failure = None;
    record.last_error = Some(message.to_string());
    merge_attention_tags(record, vec![SubscriptionAttentionTag::WaitingRelease]);
    record.next_attempt_at = Some(now + cfg.search_interval_secs);
    record.force_eligible_once = false;
    record.updated_at = now;
}

pub fn apply_parent_operation_failure(
    record: &mut WantedSubscriptionRecord,
    operation: &str,
    message: &str,
    cfg: &SubscriptionWatcherConfig,
    now: u64,
) {
    record.execution_state = SubscriptionExecutionState::Idle;
    record.retry_count = record.retry_count.saturating_add(1);
    let retry_blocked = record.max_retries > 0 && record.retry_count >= record.max_retries;
    record.failure = Some(SubscriptionFailure {
        scope: FailureScope::Parent,
        owner_id: record.subject_id.clone(),
        operation: operation.to_string(),
        error_type: "system".to_string(),
        message: message.to_string(),
        retry_count: record.retry_count,
        max_retries: record.max_retries,
        failed_at: now,
        next_retry_at: (!retry_blocked).then_some(now + retry_interval_for_operation(operation, cfg)),
        retry_blocked,
    });
    merge_attention_tags(record, vec![SubscriptionAttentionTag::Failed]);
    if retry_blocked {
        merge_attention_tags(record, vec![SubscriptionAttentionTag::RetryBlocked]);
        record.next_attempt_at = None;
    } else {
        record.next_attempt_at = Some(now + retry_interval_for_operation(operation, cfg));
    }
    record.force_eligible_once = false;
    record.last_error = Some(message.to_string());
    record.updated_at = now;
}

fn retry_interval_for_operation(operation: &str, cfg: &SubscriptionWatcherConfig) -> u64 {
    match operation {
        "progress" => cfg.progress_interval_secs,
        "link" => cfg.link_retry_interval_secs,
        "search" => cfg.search_interval_secs,
        _ => 0,
    }
}
```

Update `apply_movie_operation_outcome()` to stop assigning `record.status = derive_legacy_status(record)`.

- [ ] **Step 4: Run tests**

Run: `cargo test subscription::tests::movie_search_waiting_release_keeps_searching_without_retry_churn subscription::tests::movie_download_complete_moves_to_linking_not_completed subscription::tests::movie_link_failure_stays_linking_with_retry_due_time`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/subscription.rs
git commit -m "feat: add lifecycle movie transition helpers"
```

### Task 3: Replace Automatic Tick Pipeline with Due Operations

**Files:**

- Modify: `src/main.rs`
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing backend tests for watcher selection**

Replace legacy tests around `automatic_pipeline_action_for_wanted_record()` with:

```rust
#[test]
fn watcher_uses_select_due_operation_for_movie_states() {
    let mut record = wanted_record("subject-1", "测试电影", Some(2024));
    record.lifecycle_state = subscription::SubscriptionLifecycleState::Downloading;
    record.next_attempt_at = Some(105);

    assert_eq!(select_watcher_due_operation(&record, 104), None);
    assert_eq!(
        select_watcher_due_operation(&record, 105),
        Some(subscription::SubscriptionDueOperation::MovieProgress)
    );
}

#[test]
fn watcher_ignores_legacy_status_for_action_selection() {
    let mut record = wanted_record("subject-1", "测试电影", Some(2024));
    record.lifecycle_state = subscription::SubscriptionLifecycleState::Linking;
    record.next_attempt_at = Some(100);
    record.status = subscription::WantedSubscriptionStatus::Unprocessed;
    record.processing_stage = Some("queued".to_string());

    assert_eq!(
        select_watcher_due_operation(&record, 100),
        Some(subscription::SubscriptionDueOperation::MovieLink)
    );
}
```

- [ ] **Step 2: Run tests and confirm legacy selector fails**

Run: `cargo test watcher_uses_select_due_operation_for_movie_states watcher_ignores_legacy_status_for_action_selection`

Expected: FAIL until `select_watcher_due_operation()` exists and pipeline selectors are removed.

- [ ] **Step 3: Replace watcher selection**

In `src/main.rs`:

```rust
fn select_watcher_due_operation(
    record: &subscription::WantedSubscriptionRecord,
    now: u64,
) -> Option<subscription::SubscriptionDueOperation> {
    subscription::select_due_operation(record, now)
}
```

Update `process_wanted_watch_queue()`:

```rust
for record in records {
    let Some(operation) = select_watcher_due_operation(&record, now) else {
        continue;
    };
    let subject_id = record.subject_id.clone();
    if let Err(err) = execute_due_subscription_operation(
        state,
        account_key,
        record.clone(),
        operation,
    )
    .await
    {
        tracing::warn!(
            subject_id = %record.subject_id,
            lifecycle_state = %record.lifecycle_state.as_str(),
            "wanted subscription operation failed: {}",
            err.message()
        );
    }
}
```

Add:

```rust
async fn execute_due_subscription_operation(
    state: &AppState,
    account_key: &str,
    record: subscription::WantedSubscriptionRecord,
    operation: subscription::SubscriptionDueOperation,
) -> Result<(), ApiError> {
    match operation {
        subscription::SubscriptionDueOperation::MovieMeta => {
            process_movie_meta_operation(state, account_key, &record).await
        }
        subscription::SubscriptionDueOperation::MovieSearch => {
            process_wanted_push_step(state, account_key, &record.subject_id, true).await
        }
        subscription::SubscriptionDueOperation::MovieProgress => {
            process_wanted_progress_step(state, account_key, &record.subject_id).await
        }
        subscription::SubscriptionDueOperation::MovieLink => {
            process_wanted_completion_step(state, account_key, &record.subject_id).await
        }
        subscription::SubscriptionDueOperation::TvMeta => {
            process_tv_meta_operation(state, account_key, &record).await
        }
        subscription::SubscriptionDueOperation::TvLane(lane) => {
            process_tv_lane_operation(state, account_key, &record, lane).await
        }
    }
}
```

Implement `process_movie_meta_operation()` as a minimal lifecycle transition for this PRD:

```rust
async fn process_movie_meta_operation(
    state: &AppState,
    account_key: &str,
    record: &subscription::WantedSubscriptionRecord,
) -> Result<(), ApiError> {
    state
        .wanted_store
        .transition_movie_operation(
            account_key,
            &record.subject_id,
            subscription::MovieOperationOutcome::Advanced(
                subscription::SubscriptionLifecycleState::Searching,
            ),
            &state.config.read().await.subscription_watcher,
            unix_now_secs(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("更新订阅元数据状态失败: {e}")))?;
    Ok(())
}
```

Add TV stubs that update lane failure if unimplemented rather than falling back to legacy status. If TV lane execution is already implemented when this task runs, call that implementation instead.

Delete or stop using:

- `SubscriptionRetryAction`
- `retry_action_for_wanted_record()`
- `pipeline_action_for_wanted_record()`
- `automatic_pipeline_action_for_wanted_record()`
- `wanted_record_needs_automatic_pipeline()`
- `schedule_next_automatic_attempt()`
- `reschedule_automatic_pipeline_record()`
- `process_wanted_record_pipeline()`
- `pipeline_should_stop_after_action()`

- [ ] **Step 4: Run watcher tests**

Run: `cargo test watcher_uses_select_due_operation_for_movie_states watcher_ignores_legacy_status_for_action_selection`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/subscription.rs
git commit -m "feat: drive watcher from due operations"
```

### Task 4: Replace Store Write Paths with Semantic Transitions

**Files:**

- Modify: `src/subscription.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing tests for push/progress/completion writes**

Add Rust tests:

```rust
#[test]
fn push_success_sets_downloading_and_next_due_now() {
    let cfg = test_watcher_cfg();
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Searching, 1_000);
    let push = test_push("pushed", None);

    apply_movie_push_result(&mut record, push, None, &cfg, 2_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Downloading);
    assert_eq!(record.next_attempt_at, Some(2_000));
    assert!(record.failure.is_none());
    assert!(!record.attention_tags.contains(&SubscriptionAttentionTag::WaitingRelease));
}

#[test]
fn progress_unfinished_keeps_downloading_for_progress_interval() {
    let cfg = test_watcher_cfg();
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Downloading, 1_000);
    let push = test_push("downloading", None);

    apply_movie_progress_result(&mut record, push, false, &cfg, 2_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Downloading);
    assert_eq!(record.next_attempt_at, Some(2_000 + cfg.progress_interval_secs));
}

#[test]
fn completion_success_sets_completed() {
    let cfg = test_watcher_cfg();
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Linking, 1_000);
    let push = test_push("downloaded", None);
    let completion = test_completion("completed");

    apply_movie_completion_result(&mut record, push, completion, None, &cfg, 2_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Completed);
    assert_eq!(record.next_attempt_at, None);
    assert!(record.failure.is_none());
}
```

- [ ] **Step 2: Run tests and confirm helper gaps**

Run: `cargo test subscription::tests::push_success_sets_downloading_and_next_due_now subscription::tests::progress_unfinished_keeps_downloading_for_progress_interval subscription::tests::completion_success_sets_completed`

Expected: FAIL until semantic write helpers exist.

- [ ] **Step 3: Add semantic store methods**

In `WantedSubscriptionStore`, add methods that mutate lifecycle fields directly and no longer accept `WantedSubscriptionStatus`:

```rust
pub async fn apply_movie_push_result(
    &self,
    account_key: &str,
    subject_id: &str,
    push: TorrentPushRecord,
    error: Option<String>,
    cfg: &SubscriptionWatcherConfig,
    now: u64,
) -> std::io::Result<Option<WantedSubscriptionRecord>> { ... }

pub async fn apply_movie_progress_result(
    &self,
    account_key: &str,
    subject_id: &str,
    push: TorrentPushRecord,
    completed: bool,
    error: Option<String>,
    cfg: &SubscriptionWatcherConfig,
    now: u64,
) -> std::io::Result<Option<WantedSubscriptionRecord>> { ... }

pub async fn apply_movie_completion_result(
    &self,
    account_key: &str,
    subject_id: &str,
    push: TorrentPushRecord,
    completion: HardlinkCompletionRecord,
    error: Option<String>,
    cfg: &SubscriptionWatcherConfig,
    now: u64,
) -> std::io::Result<Option<WantedSubscriptionRecord>> { ... }
```

Each method loads the record, updates `last_push`/`last_completion` as artifact fields, then calls pure helpers:

- `apply_movie_push_result()`:
  - `push.status == "failed"` with no candidates/no match: `apply_movie_waiting_release()`.
  - other failed push: `apply_parent_operation_failure(record, "search", ...)`.
  - success: `lifecycle_state = Downloading`, `next_attempt_at = Some(now)`.
- `apply_movie_progress_result()`:
  - failed progress: keep `Downloading`, write parent failure with operation `progress`, next attempt by progress interval.
  - not completed: keep `Downloading`, `next_attempt_at = now + progress_interval_secs`.
  - completed: set `Linking`, `next_attempt_at = now`.
- `apply_movie_completion_result()`:
  - pending: keep `Downloading`, `next_attempt_at = now + progress_interval_secs`.
  - failed: keep `Linking`, write parent failure with operation `link`.
  - completed/linked: set `Completed`, clear failure/tags, `next_attempt_at = None`.

- [ ] **Step 4: Replace main.rs call sites**

Replace calls to:

- `wanted_store.update_status()`
- `wanted_store.update_push_record(..., WantedSubscriptionStatus, ...)`
- `wanted_store.update_completion_record(..., WantedSubscriptionStatus, ...)`
- `persist_subscription_sync_error()` with legacy status

with the semantic methods above or parent/lane failure helpers.

Keep artifact construction code intact.

- [ ] **Step 5: Run targeted tests**

Run: `cargo test subscription::tests::push_success_sets_downloading_and_next_due_now subscription::tests::progress_unfinished_keeps_downloading_for_progress_interval subscription::tests::completion_success_sets_completed`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/subscription.rs src/main.rs
git commit -m "feat: persist subscription outcomes through lifecycle state"
```

### Task 5: Remove Legacy Status API and Direct Status Updates

**Files:**

- Modify: `src/main.rs`
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing route/source guard tests**

Add tests in `src/main.rs` test module:

```rust
#[test]
fn wanted_status_route_is_removed_from_router_source() {
    let source = include_str!("main.rs");
    assert!(!source.contains("\"/subscriptions/wanted/{id}/status\""));
    assert!(!source.contains("wanted_subscription_status("));
}
```

Add tests in `src/subscription.rs`:

```rust
#[test]
fn direct_legacy_status_update_type_is_not_available_in_runtime_source() {
    let source = include_str!("subscription.rs");
    assert!(!source.contains("pub struct WantedStatusUpdate"));
    assert!(!source.contains("pub async fn update_status"));
}
```

- [ ] **Step 2: Run tests and confirm they fail**

Run: `cargo test wanted_status_route_is_removed_from_router_source subscription::tests::direct_legacy_status_update_type_is_not_available_in_runtime_source`

Expected: FAIL.

- [ ] **Step 3: Delete legacy direct status API**

Remove:

- Router entry `.route("/subscriptions/wanted/{id}/status", post(wanted_subscription_status))`
- `wanted_subscription_status()`
- `WantedStatusUpdate`
- `WantedStatusUpdateOutcome`
- `WantedSubscriptionStore::update_status()`
- `apply_status_update()`

Update any internal callers to semantic operations from Task 4.

- [ ] **Step 4: Run route/source guard tests**

Run: `cargo test wanted_status_route_is_removed_from_router_source subscription::tests::direct_legacy_status_update_type_is_not_available_in_runtime_source`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/subscription.rs
git commit -m "refactor: remove direct wanted status api"
```

### Task 6: Make Retry Current and Rerun Semantic Commands

**Files:**

- Modify: `src/subscription.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing tests**

Add tests:

```rust
#[test]
fn retry_current_clears_parent_failure_and_makes_current_node_due() {
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Linking, 1_000);
    record.attention_tags = vec![SubscriptionAttentionTag::Failed, SubscriptionAttentionTag::RetryBlocked];
    record.failure = Some(test_failure("link", true));
    record.next_attempt_at = None;

    retry_current_node(&mut record, 2_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Linking);
    assert!(record.failure.is_none());
    assert!(!record.attention_tags.contains(&SubscriptionAttentionTag::Failed));
    assert!(!record.attention_tags.contains(&SubscriptionAttentionTag::RetryBlocked));
    assert_eq!(record.next_attempt_at, Some(2_000));
    assert!(record.force_eligible_once);
}

#[test]
fn rerun_movie_resets_to_searching_but_keeps_douban_metadata() {
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Completed, 1_000);
    record.title = "测试电影".to_string();
    record.last_push = Some(test_push("downloaded", None));
    record.last_completion = Some(test_completion("completed"));

    rerun_subscription_task(&mut record, 2_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Searching);
    assert_eq!(record.title, "测试电影");
    assert!(record.last_push.is_none());
    assert!(record.last_completion.is_none());
    assert_eq!(record.next_attempt_at, Some(2_000));
}
```

- [ ] **Step 2: Run tests and confirm missing helpers fail**

Run: `cargo test subscription::tests::retry_current_clears_parent_failure_and_makes_current_node_due subscription::tests::rerun_movie_resets_to_searching_but_keeps_douban_metadata`

Expected: FAIL.

- [ ] **Step 3: Implement semantic command helpers and store wrappers**

In `src/subscription.rs`:

```rust
pub fn retry_current_node(record: &mut WantedSubscriptionRecord, now: u64) {
    record.failure = None;
    record.attention_tags.retain(|tag| {
        !matches!(tag, SubscriptionAttentionTag::Failed | SubscriptionAttentionTag::RetryBlocked)
    });
    record.next_attempt_at = Some(now);
    record.force_eligible_once = true;
    record.updated_at = now;
}

pub fn rerun_subscription_task(record: &mut WantedSubscriptionRecord, now: u64) {
    record.lifecycle_state = if record.media_kind == SubscriptionMediaKind::Tv {
        SubscriptionLifecycleState::Meta
    } else {
        SubscriptionLifecycleState::Searching
    };
    record.execution_state = SubscriptionExecutionState::Idle;
    record.failure = None;
    record.attention_tags.retain(|tag| *tag == SubscriptionAttentionTag::Skipped);
    record.last_push = None;
    record.last_completion = None;
    record.candidate_matches.clear();
    record.next_attempt_at = Some(now);
    record.force_eligible_once = true;
    record.last_error = None;
    record.updated_at = now;
}
```

Add store methods `retry_current_node()` and `rerun_subscription_task()` that load, mutate, save, and return the record.

- [ ] **Step 4: Update handlers**

In `wanted_subscription_retry_current()`:

- Load record.
- Reject if `lifecycle_state == Completed`.
- Call store `retry_current_node()`.
- Optionally execute due operation immediately by calling `select_due_operation()` on the returned record.
- Return `{ ok: true, action: "<due operation>", record }`.

In `wanted_subscription_rerun()`:

- Reject if `subject_id` is empty.
- Call store `rerun_subscription_task()`.
- Execute due operation if desired.
- Do not read `processing_stage`.

- [ ] **Step 5: Run tests**

Run: `cargo test subscription::tests::retry_current_clears_parent_failure_and_makes_current_node_due subscription::tests::rerun_movie_resets_to_searching_but_keeps_douban_metadata`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/subscription.rs src/main.rs
git commit -m "feat: make subscription manual commands semantic"
```

### Task 7: Isolate and Remove Legacy Runtime Migration Paths

**Files:**

- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing migration/source tests**

Add tests:

```rust
#[test]
fn migration_is_the_only_code_allowed_to_infer_from_legacy_status() {
    let source = include_str!("subscription.rs");
    let infer_refs = source.matches("infer_lifecycle_from_legacy(").count();
    assert_eq!(infer_refs, 1, "only the migration function may call legacy inference");
    assert!(!source.contains("sync_lifecycle_from_legacy_stage"));
    assert!(!source.contains("derive_legacy_status(record)"));
}

#[test]
fn repair_defaults_does_not_migrate_legacy_on_every_load() {
    let source = include_str!("subscription.rs");
    let start = source.find("fn repair_record_defaults").unwrap();
    let end = source[start..].find("\nfn ").map(|offset| start + offset).unwrap_or(source.len());
    let body = &source[start..end];
    assert!(!body.contains("migrate_legacy_status_fields"));
    assert!(!body.contains("normalize_existing_stage"));
}
```

- [ ] **Step 2: Run tests and confirm they fail**

Run: `cargo test subscription::tests::migration_is_the_only_code_allowed_to_infer_from_legacy_status subscription::tests::repair_defaults_does_not_migrate_legacy_on_every_load`

Expected: FAIL.

- [ ] **Step 3: Make legacy migration explicit**

Rename `migrate_legacy_status_fields()` to `migrate_legacy_record_to_lifecycle_once()` and use it only in a schema migration path:

```rust
fn migrate_legacy_record_to_lifecycle_once(record: &mut WantedSubscriptionRecord, now: u64) {
    let (state, tags) = infer_lifecycle_from_legacy(record);
    record.lifecycle_state = state;
    record.execution_state = SubscriptionExecutionState::Idle;
    merge_attention_tags(record, tags);
    record.next_attempt_at.get_or_insert(now);
    if record.failure.is_none() {
        record.failure = failure_from_legacy_error(record, now);
    }
}
```

Remove `record.status = derive_legacy_status(record)` from migration.

Delete runtime stage normalization:

- `normalize_existing_stage()`
- `hydrate_stage_from_record()`
- `set_stage()`
- `sync_lifecycle_from_legacy_stage()`
- `apply_status_stage()`
- `apply_candidate_stage()` as a stage writer
- `apply_push_stage()`
- `apply_completion_stage()`
- `derive_legacy_status()`

If a display message is still needed, represent it through `failure.message`, `last_error`, operation logs, or artifact errors.

- [ ] **Step 4: Move migration to schema/version upgrade**

Increment `DB_SCHEMA_VERSION`.

Add an upgrade helper:

```rust
fn migrate_subscription_schema(conn: &Connection, from_version: i64, now: u64) -> std::io::Result<()> {
    if from_version < 4 {
        migrate_legacy_status_columns_to_lifecycle(conn, now)?;
    }
    Ok(())
}
```

The migration may read old `record_json`, `status`, and `processing_stage`, then save new JSON/index fields without serializing legacy subscription fields after Task 8.

- [ ] **Step 5: Run migration/source tests**

Run: `cargo test subscription::tests::migration_is_the_only_code_allowed_to_infer_from_legacy_status subscription::tests::repair_defaults_does_not_migrate_legacy_on_every_load`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/subscription.rs
git commit -m "refactor: isolate legacy status migration"
```

### Task 8: Remove Legacy Fields from API Record and SQLite Runtime Indexing

**Files:**

- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing serialization and schema tests**

Add tests:

```rust
#[test]
fn serialized_record_does_not_expose_legacy_subscription_status_or_stage() {
    let record = movie_record_in_state(SubscriptionLifecycleState::Downloading, 1_000);
    let value = serde_json::to_value(&record).unwrap();

    assert!(value.get("lifecycle_state").is_some());
    assert!(value.get("execution_state").is_some());
    assert!(value.get("attention_tags").is_none() || value.get("attention_tags").unwrap().is_array());
    assert!(value.get("status").is_none());
    assert!(value.get("processing_stage").is_none());
}

#[test]
fn sqlite_record_indexes_do_not_require_legacy_status_column_for_due_scans() {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    let columns = table_columns(&conn, "wanted_subscription_records");

    assert!(columns.contains(&"lifecycle_state".to_string()));
    assert!(columns.contains(&"next_attempt_at".to_string()));
    assert!(!columns.contains(&"processing_stage".to_string()));
}
```

- [ ] **Step 2: Run tests and confirm failure**

Run: `cargo test subscription::tests::serialized_record_does_not_expose_legacy_subscription_status_or_stage subscription::tests::sqlite_record_indexes_do_not_require_legacy_status_column_for_due_scans`

Expected: FAIL while `status`/`processing_stage` remain serialized.

- [ ] **Step 3: Remove legacy subscription fields from record**

Delete from `WantedSubscriptionRecord`:

- `pub status: WantedSubscriptionStatus`
- `pub processing_stage: Option<String>`
- `stage_message` and `next_action` if they only describe legacy stages. If the UI still needs notes, replace with new-model `failure.message`, `last_error`, and artifact rows.

Delete `WantedSubscriptionStatus` entirely unless a migration-only `LegacyWantedSubscriptionStatus` is required inside a private migration module. If kept for migration, it must be private and not part of `WantedSubscriptionRecord`.

- [ ] **Step 4: Update SQLite save/load**

Stop writing legacy `status` into `record_json`. If SQLite cannot drop the physical `status` column in place, keep a constant placeholder in the table only:

```rust
const LEGACY_STATUS_PLACEHOLDER: &str = "migrated";
```

The `wanted_subscription_records.status` column must not be read for runtime decisions or API serialization. Prefer a schema rebuild migration that creates a new table without `status` if practical:

```sql
CREATE TABLE wanted_subscription_records_new (
    account_key TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    title TEXT NOT NULL,
    category_text TEXT,
    updated_at INTEGER NOT NULL,
    record_json TEXT NOT NULL,
    lifecycle_state TEXT NOT NULL DEFAULT 'queued',
    execution_state TEXT NOT NULL DEFAULT 'idle',
    attention_tags_json TEXT NOT NULL DEFAULT '[]',
    media_kind TEXT NOT NULL DEFAULT 'movie',
    next_attempt_at INTEGER,
    search_next_attempt_at INTEGER,
    progress_next_attempt_at INTEGER,
    link_next_attempt_at INTEGER,
    retry_blocked_count INTEGER NOT NULL DEFAULT 0
);
```

- [ ] **Step 5: Update tests/fixtures and compile errors**

Replace tests assigning `record.status` or `record.processing_stage` with lifecycle fields and attention tags.

- [ ] **Step 6: Run tests**

Run: `cargo test`

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/subscription.rs src/main.rs
git commit -m "refactor: remove legacy subscription fields from api model"
```

### Task 9: Update Frontend Primary State Display

**Files:**

- Modify: `frontend/src/App.vue`
- Modify: `frontend/src/components/SubscriptionDetailView.vue`
- Modify: `frontend/src/__tests__/subscription-card-display.test.mjs`

- [ ] **Step 1: Write failing frontend source guard tests**

In `frontend/src/__tests__/subscription-card-display.test.mjs`, add:

```js
for (const [name, body] of [
  [
    "subscriptionDisplayStatus",
    sourceBetween(
      "function subscriptionDisplayStatus",
      "\nfunction subscriptionProgress",
      "display status helper",
    ),
  ],
  [
    "subscriptionLifecycleKey",
    sourceBetween(
      "function subscriptionLifecycleKey",
      "\nfunction subscriptionAttentionKey",
      "lifecycle key helper",
    ),
  ],
  [
    "subscriptionAttentionKey",
    sourceBetween(
      "function subscriptionAttentionKey",
      "\nfunction subscriptionStageTrackLabel",
      "attention helper",
    ),
  ],
  [
    "canRetrySubscription",
    sourceBetween(
      "function canRetrySubscription",
      "\nfunction canRerunSubscription",
      "retry helper",
    ),
  ],
]) {
  assert.doesNotMatch(
    body,
    /record\?*\.status|record\?*\.processing_stage/,
    `${name} must not read legacy subscription state`,
  );
}

assert.equal(appSource.includes("SUB_LEGACY_LIFECYCLE_BY_STATUS"), false);
```

Add behavior cases:

```js
assert.deepEqual(
  plain(
    helpers
      .subscriptionLifecycleNodes({
        lifecycle_state: "linking",
        attention_tags: ["failed"],
        failure: { message: "硬链接失败" },
      })
      .find((node) => node.key === "linking"),
  ),
  { key: "linking", label: "硬链接中", state: "current", attention: "failed" },
);
```

- [ ] **Step 2: Run frontend test and confirm failure**

Run: `node frontend/src/__tests__/subscription-card-display.test.mjs`

Expected: FAIL.

- [ ] **Step 3: Rewrite display helpers**

Use only new fields:

```js
function subscriptionDisplayStatus(record) {
  const lifecycle = subscriptionLifecycleKey(record);
  const attention = subscriptionAttentionKey(record);
  if (attention) return { key: attention, text: SUB_ATTENTION_LABELS[attention] || attention };
  return { key: lifecycle, text: SUB_LIFECYCLE_LABELS[lifecycle] || "待处理" };
}

function subscriptionLifecycleKey(record) {
  const lifecycle = normalizedStatus(record?.lifecycle_state);
  return SUB_LIFECYCLE_STEPS.some((step) => step.key === lifecycle) ? lifecycle : "queued";
}

function subscriptionAttentionKey(record) {
  const tags = Array.isArray(record?.attention_tags)
    ? record.attention_tags.map((tag) => normalizedStatus(tag))
    : [];
  if (record?.failure && !tags.includes("failed")) tags.push("failed");
  return SUB_ATTENTION_PRIORITY.find((tag) => tags.includes(tag)) || "";
}

function canRetrySubscription(record) {
  return !!record?.subject_id && subscriptionLifecycleKey(record) !== "completed";
}
```

Remove `subscriptionProgressIndex()` if only used for the old status bar, or rewrite it as:

```js
function subscriptionProgressIndex(record) {
  return Math.max(
    0,
    SUB_LIFECYCLE_STEPS.findIndex((step) => step.key === subscriptionLifecycleKey(record)),
  );
}
```

- [ ] **Step 4: Remove legacy/manual detail actions**

Ensure the detail action area exposes only:

- `重试当前节点`
- `重跑任务`

Remove “刷新下载进度” and “检查完成并硬链接” action buttons. Keep artifact display rows for `last_push` and `last_completion`.

- [ ] **Step 5: Run frontend tests and build check**

Run: `node frontend/src/__tests__/subscription-card-display.test.mjs`

Run: `npm run check`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/App.vue frontend/src/components/SubscriptionDetailView.vue frontend/src/__tests__/subscription-card-display.test.mjs
git commit -m "refactor: display subscriptions from lifecycle state"
```

### Task 10: Final Legacy Reference Audit and Full Verification

**Files:**

- Modify as needed: `src/subscription.rs`, `src/main.rs`, `frontend/src/App.vue`, tests

- [ ] **Step 1: Run source audits**

Run:

```bash
rg -n "WantedSubscriptionStatus|WantedStatusUpdate|pipeline_action_for_wanted_record|automatic_pipeline_action_for_wanted_record|wanted_record_needs_automatic_pipeline|retry_action_for_wanted_record|sync_lifecycle_from_legacy_stage|apply_status_stage|apply_push_stage|apply_completion_stage|SUB_LEGACY_LIFECYCLE_BY_STATUS|record\\?*\\.status|record\\?*\\.processing_stage" src frontend/src
```

Expected: no runtime/frontend primary-state hits. Acceptable hits only:

- migration-only legacy parser/helper, with comments saying it is one-time migration only
- operation artifact `push.status`, `completion.status`, episode/file `status`
- operation log status
- tests asserting absence

- [ ] **Step 2: Run backend tests**

Run: `cargo test`

Expected: PASS.

- [ ] **Step 3: Run frontend tests**

Run:

```bash
node frontend/src/__tests__/subscription-card-display.test.mjs
npm run check
```

Expected: PASS.

- [ ] **Step 4: Manual API shape check**

Start server if needed with the project’s normal command. Fetch wanted subscriptions and confirm a record contains:

```json
{
  "lifecycle_state": "downloading",
  "execution_state": "idle",
  "attention_tags": [],
  "failure": null,
  "next_attempt_at": 1234567890
}
```

and does not contain top-level `status` or `processing_stage`.

- [ ] **Step 5: Commit**

```bash
git add src frontend docs/archive/superpowers/plans/2026-07-09-subscription-state-convergence.md
git commit -m "test: verify subscription state convergence"
```

---

## Self-Review

**Spec coverage:**

- Main state uses only `lifecycle_state`: covered by Tasks 1, 3, 4, 8, 9.
- `failed`, `skipped`, `waiting_release`, `retry_blocked` are attention/failure only: covered by Tasks 2, 6, 8, 9.
- Automatic tick uses `select_due_operation()` and TV lane due logic: covered by Tasks 1 and 3.
- Movie flow `queued -> meta -> searching -> downloading -> linking -> completed`: covered by Tasks 1, 2, 3, 4.
- TV parent state and lane priority `link > progress > search`: covered by Tasks 1, 3, 10; existing TV parent helpers remain, with legacy status write removed in Task 7.
- Manual operations reduced to retry/rerun and no legacy stage dependency: covered by Task 6 and frontend Task 9.
- Delete `/status` API and direct legacy status writes: covered by Task 5.
- One-time migration allowed, runtime inference removed: covered by Task 7 and Task 10 source audit.
- API returns new fields and omits legacy top-level fields: covered by Task 8 and Task 10.
- Frontend no longer infers primary state from `status`, `processing_stage`, `last_push.status`, `last_completion.status`: covered by Task 9 source guards. Artifact status rows remain allowed.
- SQLite due scans by lifecycle/lane fields: covered by Task 8 and existing index tests.

**Placeholder scan:** No task uses placeholder instructions or unspecified tests. Each implementation task includes concrete file targets, snippets, commands, and expected results.

**Type consistency:** The plan consistently uses existing public types where present: `SubscriptionLifecycleState`, `SubscriptionExecutionState`, `SubscriptionAttentionTag`, `SubscriptionFailure`, `FailureScope`, `SubscriptionDueOperation`, `TvLaneKind`, `MovieOperationOutcome`, `WantedSubscriptionRecord`, `TorrentPushRecord`, and `HardlinkCompletionRecord`. Newly named helpers are introduced before later tasks reference them.

**Risk notes:**

- `status` is currently a physical SQLite column in `wanted_subscription_records`. If SQLite table rebuild is too risky in one step, keeping a placeholder column is acceptable only as storage compatibility; it must not be serialized, written meaningfully, or read for runtime decisions.
- Existing external protocol handlers are large. Task 4 intentionally preserves their qB/M-Team/hardlink logic and changes only the persistence boundary.
- Some tests in `src/main.rs` currently assert legacy behavior. Task execution must rewrite those tests rather than carrying compatibility forward.
