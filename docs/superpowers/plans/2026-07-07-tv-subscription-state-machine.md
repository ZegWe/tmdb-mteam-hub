# TV Subscription State Machine Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the wanted-subscription workflow with the lifecycle-only state model from `docs/superpowers/specs/2026-07-07-tv-subscription-state-machine-prd.md`, while keeping movie processing working and adding TV episode/task/lane scheduling.

**Architecture:** Keep `WantedSubscriptionRecord` as the API-facing aggregate, but make the new lifecycle fields authoritative and derive the legacy `status` string only for transition compatibility. Store TV episode records, download task records, lane state, failures, and skip state inside the subscription JSON blob for MVP, while adding SQLite index columns for eligible scans and list queries. Reuse the existing Douban, M-Team, qBittorrent, and hardlink integrations; wrap them behind movie operations and TV lane operations instead of changing external protocols.

**Tech Stack:** Rust, tokio, axum, rusqlite, serde, Vue 3, Node assertion tests.

---

## File Structure

- Modify: `src/subscription.rs`
  - Owns persistent state types, legacy migration, status derivation, TV episode/task/lane helpers, retry/failure bookkeeping, skip/manual retry mutations, and SQLite schema/index persistence.
- Modify: `src/main.rs`
  - Owns tick selection, movie operations, TV lane operations, API handlers, qB progress integration, hardlink execution integration, and operation logs.
- Modify: `src/config.rs`
  - Adds watcher interval settings for search/progress/link lanes and retry intervals while preserving existing defaults.
- Modify: `config.example.toml`
  - Documents the new interval settings.
- Modify: `frontend/src/App.vue`
  - Updates subscription status helpers, detail row helpers, manual action calls, and TV progress summaries.
- Modify: `frontend/src/components/SubscriptionDetailView.vue`
  - Renders TV episode matrix, download task list, lane failures, and scoped manual retry/skip actions.
- Modify: `frontend/src/styles.css`
  - Adds compact matrix/task/list styling without changing the existing poster-card layout.
- Create: `frontend/src/__tests__/subscription-state-machine-display.test.mjs`
  - Covers lifecycle labels, attention tag priority, TV summaries, scoped actions, and detail sections.

---

### Task 1: Define New Persistent State Model

**Files:**
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing state model tests**

Add tests in `src/subscription.rs`:

```rust
#[test]
fn lifecycle_statuses_exclude_execution_and_attention_states() {
    let labels = SubscriptionLifecycleState::ALL
        .iter()
        .map(|state| state.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        labels,
        vec!["queued", "meta", "searching", "downloading", "linking", "completed"]
    );
    assert!(!labels.contains(&"failed"));
    assert!(!labels.contains(&"skipped"));
    assert!(!labels.contains(&"running"));
    assert!(!labels.contains(&"idle"));
}

#[test]
fn record_defaults_to_new_queued_idle_model() {
    let item = test_douban_item("subject-1", "测试电影", "电影");
    let record = record_from_item(&item, 0, 3, 1_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Queued);
    assert_eq!(record.execution_state, SubscriptionExecutionState::Idle);
    assert!(record.attention_tags.is_empty());
    assert!(record.failure.is_none());
    assert_eq!(record.next_attempt_at, Some(1_000));
}
```

Run: `cargo test subscription::tests::lifecycle_statuses_exclude_execution_and_attention_states subscription::tests::record_defaults_to_new_queued_idle_model`

Expected: FAIL because these types and fields do not exist.

- [ ] **Step 2: Add state enums and aggregate fields**

Add these public types near `WantedSubscriptionStatus`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionLifecycleState {
    Queued,
    Meta,
    Searching,
    Downloading,
    Linking,
    Completed,
}

impl SubscriptionLifecycleState {
    pub const ALL: [Self; 6] = [
        Self::Queued,
        Self::Meta,
        Self::Searching,
        Self::Downloading,
        Self::Linking,
        Self::Completed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Meta => "meta",
            Self::Searching => "searching",
            Self::Downloading => "downloading",
            Self::Linking => "linking",
            Self::Completed => "completed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionExecutionState {
    Idle,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionAttentionTag {
    WaitingRelease,
    Failed,
    RetryBlocked,
    Skipped,
    NeedsReconciliation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionMediaKind {
    Movie,
    Tv,
}

impl SubscriptionMediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "tv",
        }
    }

    pub fn from_tags(tags: &[String]) -> Self {
        if tags.iter().any(|tag| {
            let tag = tag.trim().to_ascii_lowercase();
            matches!(tag.as_str(), "tv" | "剧集" | "电视剧" | "番剧")
        }) {
            Self::Tv
        } else {
            Self::Movie
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureScope {
    Parent,
    Lane,
    DownloadTask,
    Episode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionFailure {
    pub scope: FailureScope,
    pub owner_id: String,
    pub operation: String,
    pub error_type: String,
    pub message: String,
    pub retry_count: u32,
    pub max_retries: u32,
    pub failed_at: u64,
    pub next_retry_at: Option<u64>,
    pub retry_blocked: bool,
}
```

Add fields to `WantedSubscriptionRecord`:

```rust
#[serde(default)]
pub lifecycle_state: SubscriptionLifecycleState,
#[serde(default)]
pub execution_state: SubscriptionExecutionState,
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub attention_tags: Vec<SubscriptionAttentionTag>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub failure: Option<SubscriptionFailure>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub next_attempt_at: Option<u64>,
#[serde(default)]
pub force_eligible_once: bool,
#[serde(default)]
pub media_kind: SubscriptionMediaKind,
```

Use explicit defaults:

```rust
impl Default for SubscriptionLifecycleState {
    fn default() -> Self { Self::Queued }
}

impl Default for SubscriptionExecutionState {
    fn default() -> Self { Self::Idle }
}

impl Default for SubscriptionMediaKind {
    fn default() -> Self { Self::Movie }
}
```

- [ ] **Step 3: Initialize and repair defaults**

Update `record_from_item_with_detail()` to set:

```rust
lifecycle_state: SubscriptionLifecycleState::Queued,
execution_state: SubscriptionExecutionState::Idle,
attention_tags: Vec::new(),
failure: None,
next_attempt_at: Some(now),
force_eligible_once: false,
media_kind: SubscriptionMediaKind::from_tags(&tags),
```

Update fallback construction in `parse_record_row()` with the same fields. Update `repair_record_defaults()` so records deserialized from older JSON call `migrate_legacy_status_fields(record, now)` when `record.lifecycle_state` is still its default and old `status`/`processing_stage` carries more precise information.

- [ ] **Step 4: Run focused tests**

Run: `cargo test subscription::tests::lifecycle_statuses_exclude_execution_and_attention_states subscription::tests::record_defaults_to_new_queued_idle_model`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/subscription.rs
git commit -m "feat: add subscription lifecycle state fields"
```

### Task 2: Add Legacy Migration and Derived Compatibility Status

**Files:**
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing migration tests**

Add table-driven tests:

```rust
#[test]
fn legacy_statuses_map_to_new_lifecycle_and_tags() {
    let cases = vec![
        (WantedSubscriptionStatus::Unprocessed, None, SubscriptionLifecycleState::Queued, vec![]),
        (WantedSubscriptionStatus::Matching, None, SubscriptionLifecycleState::Searching, vec![]),
        (WantedSubscriptionStatus::Processing, None, SubscriptionLifecycleState::Searching, vec![]),
        (WantedSubscriptionStatus::Pushed, None, SubscriptionLifecycleState::Downloading, vec![]),
        (WantedSubscriptionStatus::Downloading, None, SubscriptionLifecycleState::Downloading, vec![]),
        (WantedSubscriptionStatus::Completed, None, SubscriptionLifecycleState::Linking, vec![]),
        (WantedSubscriptionStatus::Linked, None, SubscriptionLifecycleState::Completed, vec![]),
        (WantedSubscriptionStatus::Skipped, None, SubscriptionLifecycleState::Queued, vec![SubscriptionAttentionTag::Skipped]),
    ];

    for (old_status, stage, expected_state, expected_tags) in cases {
        let mut record = bare_record_with_status(old_status, stage);
        migrate_legacy_status_fields(&mut record, 2_000);
        assert_eq!(record.lifecycle_state, expected_state);
        assert_eq!(record.attention_tags, expected_tags);
    }
}

#[test]
fn legacy_failed_records_follow_prd_precedence() {
    let mut completion_failed = bare_record_with_status(WantedSubscriptionStatus::Failed, None);
    completion_failed.last_completion = Some(test_completion("failed"));
    migrate_legacy_status_fields(&mut completion_failed, 2_000);
    assert_eq!(completion_failed.lifecycle_state, SubscriptionLifecycleState::Linking);
    assert!(completion_failed.attention_tags.contains(&SubscriptionAttentionTag::Failed));

    let mut no_match = bare_record_with_status(WantedSubscriptionStatus::Failed, Some("no_match"));
    no_match.last_push = Some(test_push("failed", Some("没有匹配规则命中")));
    migrate_legacy_status_fields(&mut no_match, 2_000);
    assert_eq!(no_match.lifecycle_state, SubscriptionLifecycleState::Searching);
    assert!(no_match.attention_tags.contains(&SubscriptionAttentionTag::WaitingRelease));
}
```

Run: `cargo test subscription::tests::legacy_statuses_map_to_new_lifecycle_and_tags subscription::tests::legacy_failed_records_follow_prd_precedence`

Expected: FAIL until migration helpers exist.

- [ ] **Step 2: Implement deterministic migration**

Create:

```rust
pub fn migrate_legacy_status_fields(record: &mut WantedSubscriptionRecord, now: u64) {
    let (state, tags) = infer_lifecycle_from_legacy(record);
    record.lifecycle_state = state;
    record.execution_state = SubscriptionExecutionState::Idle;
    merge_attention_tags(record, tags);
    record.next_attempt_at.get_or_insert(now);
    if record.failure.is_none() {
        record.failure = failure_from_legacy_error(record, now);
    }
    record.status = derive_legacy_status(record);
}
```

Implement `infer_lifecycle_from_legacy()` in PRD order:

1. `last_completion.status = completed` => `completed`
2. `last_completion.status = failed` => `linking + failed`
3. `last_completion.status = pending` => `downloading`
4. `last_push.status = linked/completed` => `completed` when completion files are complete, otherwise `linking`
5. `last_push.status = downloaded` => `linking`
6. `last_push.status = downloading/pushed` => `downloading`
7. `last_push.status = failed` with no-candidate/no-match text or stage => `searching + waiting_release`
8. `last_push.status = failed` with qB/M-Team/API text => `searching + failed`
9. `processing_stage = link_failed/download_complete/link_planned` => `linking`
10. `processing_stage = downloading/pushed` => `downloading`
11. `processing_stage = no_candidates/no_match/searching/matched/pushing/push_failed` => `searching`, with `waiting_release` for no candidates/no match and `failed` for push failed
12. non-empty `candidate_matches` => `searching`
13. unknown => `queued + needs_reconciliation`

Keep old `status` as an API compatibility field only:

```rust
pub fn derive_legacy_status(record: &WantedSubscriptionRecord) -> WantedSubscriptionStatus {
    if record.attention_tags.contains(&SubscriptionAttentionTag::Skipped) {
        return WantedSubscriptionStatus::Skipped;
    }
    if record.attention_tags.contains(&SubscriptionAttentionTag::Failed)
        || record.attention_tags.contains(&SubscriptionAttentionTag::RetryBlocked)
    {
        return WantedSubscriptionStatus::Failed;
    }
    match record.lifecycle_state {
        SubscriptionLifecycleState::Queued => WantedSubscriptionStatus::Unprocessed,
        SubscriptionLifecycleState::Meta | SubscriptionLifecycleState::Searching => WantedSubscriptionStatus::Matching,
        SubscriptionLifecycleState::Downloading => WantedSubscriptionStatus::Downloading,
        SubscriptionLifecycleState::Linking => WantedSubscriptionStatus::Completed,
        SubscriptionLifecycleState::Completed => WantedSubscriptionStatus::Linked,
    }
}
```

- [ ] **Step 3: Run focused migration tests**

Run: `cargo test subscription::tests::legacy_statuses_map_to_new_lifecycle_and_tags subscription::tests::legacy_failed_records_follow_prd_precedence`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/subscription.rs
git commit -m "feat: migrate legacy subscription states"
```

### Task 3: Add TV Episode, Task, Coverage, and Lane Model

**Files:**
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing TV model tests**

Add tests:

```rust
#[test]
fn tv_meta_initializes_target_episodes_and_cursor() {
    let mut record = bare_tv_record("subject-tv", 1, 8);
    initialize_tv_targets(&mut record, TvTargetRange::episodes(1, 1, 8), 1_000).unwrap();

    assert_eq!(record.tv.as_ref().unwrap().episode_total, 8);
    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(1));
    assert_eq!(record.tv.as_ref().unwrap().episodes.len(), 8);
    assert_eq!(record.tv.as_ref().unwrap().lanes.search.next_attempt_at, Some(1_000));
    assert_eq!(record.tv.as_ref().unwrap().lanes.progress.next_attempt_at, Some(1_000));
    assert_eq!(record.tv.as_ref().unwrap().lanes.link.next_attempt_at, Some(1_000));
}

#[test]
fn cursor_advances_over_active_completed_blocked_and_skipped_assignments() {
    let mut tv = test_tv_state(1, 8);
    bind_task_to_episodes(&mut tv, "task-e01", CoverageRange::single(1, 1), CoverageTrust::Tentative, 1_000).unwrap();
    block_episode_assignment(&mut tv, 1, 3, test_failure("link", true)).unwrap();
    skip_episode_range(&mut tv, 1, 4, 4, "user skipped", 1_000).unwrap();

    recalculate_search_cursor(&mut tv);

    assert_eq!(tv.search_cursor_episode, Some(2));
}

#[test]
fn verified_coverage_loss_releases_unverified_episodes_and_rewinds_cursor() {
    let mut tv = test_tv_state(1, 8);
    bind_task_to_episodes(&mut tv, "task-e02-e04", CoverageRange::range(1, 2, 4), CoverageTrust::Tentative, 1_000).unwrap();
    apply_verified_coverage(&mut tv, "task-e02-e04", CoverageRange::single(1, 2), 2_000).unwrap();

    assert_eq!(episode(&tv, 3).assignment_state, EpisodeAssignmentState::Released);
    assert_eq!(episode(&tv, 4).assignment_state, EpisodeAssignmentState::Released);
    assert_eq!(tv.search_cursor_episode, Some(3));
}
```

Run: `cargo test subscription::tests::tv_meta_initializes_target_episodes_and_cursor subscription::tests::cursor_advances_over_active_completed_blocked_and_skipped_assignments subscription::tests::verified_coverage_loss_releases_unverified_episodes_and_rewinds_cursor`

Expected: FAIL until TV model exists.

- [ ] **Step 2: Add TV aggregate structs**

Add:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TvSubscriptionState {
    pub schema_version: u32,
    pub metadata_ready: bool,
    pub episode_records_initialized: bool,
    pub target_episode_set_known: bool,
    pub season_number: u32,
    pub episode_total: u32,
    pub target_start_episode: u32,
    pub target_end_episode: u32,
    pub search_cursor_episode: Option<u32>,
    pub lanes: TvLaneSet,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episodes: Vec<TvEpisodeRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub download_tasks: Vec<DownloadTaskRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TvLaneSet {
    pub search: OperationLaneState,
    pub progress: OperationLaneState,
    pub link: OperationLaneState,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OperationLaneState {
    pub next_attempt_at: Option<u64>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub force_eligible_once: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<SubscriptionFailure>,
}
```

Add episode/task enums and records:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TvTargetRange {
    pub season_number: u32,
    pub start_episode: u32,
    pub end_episode: u32,
    pub episode_total: u32,
}

impl TvTargetRange {
    pub fn episodes(season_number: u32, start_episode: u32, end_episode: u32) -> Self {
        Self {
            season_number,
            start_episode,
            end_episode,
            episode_total: end_episode,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageRange {
    pub season_number: u32,
    pub start_episode: u32,
    pub end_episode: u32,
}

impl CoverageRange {
    pub fn single(season_number: u32, episode_number: u32) -> Self {
        Self { season_number, start_episode: episode_number, end_episode: episode_number }
    }

    pub fn range(season_number: u32, start_episode: u32, end_episode: u32) -> Self {
        Self { season_number, start_episode, end_episode }
    }

    pub fn contains_episode(self, episode_number: u32) -> bool {
        self.start_episode <= episode_number && episode_number <= self.end_episode
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageTrust { Tentative, Verified }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeTargetState { Target, Skipped, Completed }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeAssignmentState { None, Active, Blocked, Released, Completed, Skipped }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeCoverageState { Uncovered, TentativeCovered, VerifiedCovered }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadTaskState { Pushed, Downloading, Downloaded, Linking, Completed, Missing, Failed, Ignored, Superseded }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TvEpisodeRecord {
    pub season_number: u32,
    pub episode_number: u32,
    pub label: String,
    pub target_state: EpisodeTargetState,
    pub coverage_state: EpisodeCoverageState,
    pub assignment_state: EpisodeAssignmentState,
    pub selected_task_id: Option<String>,
    pub download_state: String,
    pub link_state: String,
    pub retry_count: u32,
    pub max_retries: u32,
    pub failure: Option<SubscriptionFailure>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTaskRecord {
    pub task_id: String,
    pub torrent_id: String,
    pub torrent_title: String,
    pub qb_server_id: String,
    pub qb_category: String,
    pub qb_save_dir_name: String,
    pub qb_hash: Option<String>,
    pub qb_name: Option<String>,
    pub state: DownloadTaskState,
    pub tentative_coverage: Vec<CoverageRange>,
    pub verified_coverage: Vec<CoverageRange>,
    pub progress: Option<f64>,
    pub pushed_at: u64,
    pub checked_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub failure: Option<SubscriptionFailure>,
}
```

Add to `WantedSubscriptionRecord`:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub tv: Option<TvSubscriptionState>,
```

- [ ] **Step 3: Implement cursor, binding, release, and skip helpers**

Create helpers:

```rust
pub fn initialize_tv_targets(record: &mut WantedSubscriptionRecord, target: TvTargetRange, now: u64) -> Result<(), String>;
pub fn bind_task_to_episodes(tv: &mut TvSubscriptionState, task_id: &str, coverage: CoverageRange, trust: CoverageTrust, now: u64) -> Result<(), String>;
pub fn apply_verified_coverage(tv: &mut TvSubscriptionState, task_id: &str, coverage: CoverageRange, now: u64) -> Result<(), String>;
pub fn recalculate_search_cursor(tv: &mut TvSubscriptionState);
pub fn block_episode_assignment(tv: &mut TvSubscriptionState, season: u32, episode: u32, failure: SubscriptionFailure) -> Result<(), String>;
pub fn skip_episode_range(tv: &mut TvSubscriptionState, season: u32, start: u32, end: u32, reason: &str, now: u64) -> Result<(), String>;
pub fn unskip_episode_range(tv: &mut TvSubscriptionState, season: u32, start: u32, end: u32, now: u64) -> Result<(), String>;
```

Rules:

- `bind_task_to_episodes()` only binds episodes that are not completed, not skipped, and have no active/blocked selected task.
- Overlapping new tasks ignore already assigned or completed episodes.
- `apply_verified_coverage()` changes covered episodes to `VerifiedCovered`; tentative episodes not verified are released unless already completed/skipped.
- `recalculate_search_cursor()` returns the first target episode that is not completed/skipped and has no active or blocked selected task.
- If no such episode exists, `search_cursor_episode = episode_total + 1`.
- `block_episode_assignment()` sets only the requested episode to `Blocked`, preserves its selected task for auditability, stores the episode failure, and then recalculates the cursor so later uncovered episodes can still be searched.

- [ ] **Step 4: Run focused TV model tests**

Run: `cargo test subscription::tests::tv_meta_initializes_target_episodes_and_cursor subscription::tests::cursor_advances_over_active_completed_blocked_and_skipped_assignments subscription::tests::verified_coverage_loss_releases_unverified_episodes_and_rewinds_cursor`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/subscription.rs
git commit -m "feat: add tv episode task lane model"
```

### Task 4: Derive TV Parent State and Completion

**Files:**
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing derived-state tests**

Add tests:

```rust
#[test]
fn tv_parent_state_stays_meta_until_metadata_guards_are_ready() {
    let mut record = bare_tv_record("subject-tv", 1, 8);
    record.lifecycle_state = SubscriptionLifecycleState::Meta;
    record.tv = Some(TvSubscriptionState {
        metadata_ready: false,
        episode_records_initialized: true,
        target_episode_set_known: true,
        ..test_tv_state(1, 8)
    });

    derive_tv_parent_lifecycle(&mut record);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Meta);
}

#[test]
fn tv_linking_state_still_allows_search_work_to_be_due() {
    let mut record = tv_record_with_linkable_e01_and_uncovered_e05();
    derive_tv_parent_lifecycle(&mut record);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Linking);
    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(5));
}

#[test]
fn tv_completed_requires_all_unskipped_episodes_linked_and_no_active_work() {
    let mut record = tv_record_with_cursor_past_end_but_active_download();
    derive_tv_parent_lifecycle(&mut record);
    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Downloading);

    complete_all_tv_episodes(record.tv.as_mut().unwrap(), 2_000);
    derive_tv_parent_lifecycle(&mut record);
    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Completed);
}
```

Run: `cargo test subscription::tests::tv_parent_state_stays_meta_until_metadata_guards_are_ready subscription::tests::tv_linking_state_still_allows_search_work_to_be_due subscription::tests::tv_completed_requires_all_unskipped_episodes_linked_and_no_active_work`

Expected: FAIL until derivation exists.

- [ ] **Step 2: Implement parent derivation**

Add:

```rust
pub fn derive_tv_parent_lifecycle(record: &mut WantedSubscriptionRecord) {
    let Some(tv) = record.tv.as_ref() else { return; };
    if !(tv.metadata_ready && tv.episode_records_initialized && tv.target_episode_set_known) {
        record.lifecycle_state = SubscriptionLifecycleState::Meta;
        return;
    }
    record.lifecycle_state = if tv_is_complete(tv) {
        SubscriptionLifecycleState::Completed
    } else if tv_has_linkable_or_linking_work(tv) {
        SubscriptionLifecycleState::Linking
    } else if tv_has_active_downloads(tv) {
        SubscriptionLifecycleState::Downloading
    } else if tv_has_uncovered_or_waiting_cursor(tv) {
        SubscriptionLifecycleState::Searching
    } else {
        SubscriptionLifecycleState::Meta
    };
    record.status = derive_legacy_status(record);
}
```

Implement completion condition exactly:

```rust
pub fn tv_is_complete(tv: &TvSubscriptionState) -> bool {
    let cursor_done = tv.search_cursor_episode.unwrap_or(1) > tv.episode_total
        || tv.episodes.iter().all(|ep| matches!(ep.assignment_state, EpisodeAssignmentState::Completed | EpisodeAssignmentState::Skipped));
    cursor_done
        && !tv_has_active_downloads(tv)
        && !tv_has_linkable_or_linking_work(tv)
        && tv.episodes.iter().all(|ep| matches!(ep.assignment_state, EpisodeAssignmentState::Completed | EpisodeAssignmentState::Skipped))
}
```

- [ ] **Step 3: Run focused derived-state tests**

Run: `cargo test subscription::tests::tv_parent_state_stays_meta_until_metadata_guards_are_ready subscription::tests::tv_linking_state_still_allows_search_work_to_be_due subscription::tests::tv_completed_requires_all_unskipped_episodes_linked_and_no_active_work`

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/subscription.rs
git commit -m "feat: derive tv parent lifecycle state"
```

### Task 5: Add SQLite Index Columns and Rebuildable Migration

**Files:**
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing schema/index tests**

Add tests using an in-memory SQLite connection:

```rust
#[test]
fn schema_has_parent_index_fields_for_eligible_scans() {
    let conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    let columns = table_columns(&conn, "wanted_subscription_records");

    for name in [
        "lifecycle_state",
        "execution_state",
        "attention_tags_json",
        "media_kind",
        "next_attempt_at",
        "search_next_attempt_at",
        "progress_next_attempt_at",
        "link_next_attempt_at",
        "retry_blocked_count",
    ] {
        assert!(columns.contains(&name.to_string()), "missing column {name}");
    }
}

#[test]
fn save_state_rebuilds_record_indexes_without_deleting_json_blob() {
    let mut conn = Connection::open_in_memory().unwrap();
    init_schema(&conn).unwrap();
    let mut state = WantedSubscriptionState::new("acct", 1_000);
    state.records.insert("subject-tv".into(), tv_record_ready_for_search("subject-tv"));

    save_state_to_db(&mut conn, "acct", &state).unwrap();

    let row = indexed_record_row(&conn, "acct", "subject-tv");
    assert_eq!(row.lifecycle_state, "searching");
    assert_eq!(row.media_kind, "tv");
    assert!(row.search_next_attempt_at.is_some());
    assert!(!row.record_json.is_empty());
}
```

Run: `cargo test subscription::tests::schema_has_parent_index_fields_for_eligible_scans subscription::tests::save_state_rebuilds_record_indexes_without_deleting_json_blob`

Expected: FAIL because only old columns exist and `save_state_to_db()` deletes index rows.

- [ ] **Step 2: Bump schema and add columns**

Set:

```rust
const DB_SCHEMA_VERSION: i64 = 3;
```

Extend `init_schema()` with additive migration helpers:

```rust
ensure_column(conn, "wanted_subscription_records", "lifecycle_state", "TEXT NOT NULL DEFAULT 'queued'")?;
ensure_column(conn, "wanted_subscription_records", "execution_state", "TEXT NOT NULL DEFAULT 'idle'")?;
ensure_column(conn, "wanted_subscription_records", "attention_tags_json", "TEXT NOT NULL DEFAULT '[]'")?;
ensure_column(conn, "wanted_subscription_records", "media_kind", "TEXT NOT NULL DEFAULT 'movie'")?;
ensure_column(conn, "wanted_subscription_records", "next_attempt_at", "INTEGER")?;
ensure_column(conn, "wanted_subscription_records", "search_next_attempt_at", "INTEGER")?;
ensure_column(conn, "wanted_subscription_records", "progress_next_attempt_at", "INTEGER")?;
ensure_column(conn, "wanted_subscription_records", "link_next_attempt_at", "INTEGER")?;
ensure_column(conn, "wanted_subscription_records", "retry_blocked_count", "INTEGER NOT NULL DEFAULT 0")?;
```

Add indexes:

```sql
CREATE INDEX IF NOT EXISTS wanted_records_lifecycle_due_idx
ON wanted_subscription_records (account_key, lifecycle_state, execution_state, next_attempt_at);

CREATE INDEX IF NOT EXISTS wanted_records_tv_lane_due_idx
ON wanted_subscription_records (account_key, media_kind, search_next_attempt_at, progress_next_attempt_at, link_next_attempt_at);
```

- [ ] **Step 3: Persist rebuilt index rows**

Update `save_state_to_db()` so it writes `subscription_state_blobs` and also upserts rows in `wanted_subscription_records` with `record_json` plus redundant fields. Do not delete the JSON blob. Generate indexed fields from the record:

```rust
fn record_index_values(record: &WantedSubscriptionRecord) -> RecordIndexValues {
    RecordIndexValues {
        lifecycle_state: record.lifecycle_state.as_str().to_string(),
        execution_state: execution_state_label(record.execution_state).to_string(),
        attention_tags_json: serde_json::to_string(&record.attention_tags).unwrap_or_else(|_| "[]".to_string()),
        media_kind: record.media_kind.as_str().to_string(),
        next_attempt_at: record.next_attempt_at,
        search_next_attempt_at: record.tv.as_ref().and_then(|tv| tv.lanes.search.next_attempt_at),
        progress_next_attempt_at: record.tv.as_ref().and_then(|tv| tv.lanes.progress.next_attempt_at),
        link_next_attempt_at: record.tv.as_ref().and_then(|tv| tv.lanes.link.next_attempt_at),
        retry_blocked_count: retry_blocked_count(record),
    }
}
```

- [ ] **Step 4: Run schema/index tests**

Run: `cargo test subscription::tests::schema_has_parent_index_fields_for_eligible_scans subscription::tests::save_state_rebuilds_record_indexes_without_deleting_json_blob`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/subscription.rs
git commit -m "feat: index subscription lifecycle scan fields"
```

### Task 6: Add Configurable Lane and Retry Intervals

**Files:**
- Modify: `src/config.rs`
- Modify: `config.example.toml`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing config normalization test**

Add in `src/config.rs`:

```rust
#[test]
fn subscription_watcher_defaults_include_lane_intervals() {
    let cfg = super::SubscriptionWatcherConfig::default();

    assert_eq!(cfg.search_interval_secs, 1_800);
    assert_eq!(cfg.progress_interval_secs, 300);
    assert_eq!(cfg.link_retry_interval_secs, 900);
    assert_eq!(cfg.system_retry_interval_secs, 600);
}
```

Run: `cargo test config::tests::subscription_watcher_defaults_include_lane_intervals`

Expected: FAIL until fields exist.

- [ ] **Step 2: Add interval fields**

Add to `SubscriptionWatcherConfig`:

```rust
#[serde(default = "default_subscription_search_interval_secs")]
pub search_interval_secs: u64,
#[serde(default = "default_subscription_progress_interval_secs")]
pub progress_interval_secs: u64,
#[serde(default = "default_subscription_link_retry_interval_secs")]
pub link_retry_interval_secs: u64,
#[serde(default = "default_subscription_system_retry_interval_secs")]
pub system_retry_interval_secs: u64,
```

Add defaults:

```rust
fn default_subscription_search_interval_secs() -> u64 { 1_800 }
fn default_subscription_progress_interval_secs() -> u64 { 300 }
fn default_subscription_link_retry_interval_secs() -> u64 { 900 }
fn default_subscription_system_retry_interval_secs() -> u64 { 600 }
```

Update `normalize_subscription_watcher()` in `src/main.rs` to clamp each interval to at least 30 seconds.

- [ ] **Step 3: Document config example**

Under `[subscription_watcher]` in `config.example.toml`, add:

```toml
search_interval_secs = 1800
progress_interval_secs = 300
link_retry_interval_secs = 900
system_retry_interval_secs = 600
```

- [ ] **Step 4: Run config test**

Run: `cargo test config::tests::subscription_watcher_defaults_include_lane_intervals`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/config.rs src/main.rs config.example.toml
git commit -m "feat: configure subscription lane intervals"
```

### Task 7: Implement Unified Tick Eligibility and One Operation Per Tick

**Files:**
- Modify: `src/subscription.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing scheduling tests**

Add tests around pure helpers:

```rust
#[test]
fn movie_successful_transition_is_immediately_due_for_next_state() {
    let cfg = test_watcher_cfg();
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Queued, 1_000);

    apply_movie_operation_outcome(&mut record, MovieOperationOutcome::Advanced(SubscriptionLifecycleState::Meta), &cfg, 2_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Meta);
    assert_eq!(record.next_attempt_at, Some(2_000));
}

#[test]
fn unchanged_movie_state_waits_for_state_interval() {
    let cfg = test_watcher_cfg();
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Downloading, 1_000);

    apply_movie_operation_outcome(&mut record, MovieOperationOutcome::StillDownloading, &cfg, 2_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Downloading);
    assert_eq!(record.next_attempt_at, Some(2_000 + cfg.progress_interval_secs));
}

#[test]
fn tv_due_lanes_choose_link_then_progress_then_search_without_updating_others() {
    let mut record = tv_record_all_lanes_due(1_000);

    let selected = select_due_tv_lane(&record, 1_000).unwrap();

    assert_eq!(selected, TvLaneKind::Link);
    assert_eq!(record.tv.as_ref().unwrap().lanes.progress.next_attempt_at, Some(1_000));
    assert_eq!(record.tv.as_ref().unwrap().lanes.search.next_attempt_at, Some(1_000));
}
```

Run: `cargo test subscription::tests::movie_successful_transition_is_immediately_due_for_next_state subscription::tests::unchanged_movie_state_waits_for_state_interval subscription::tests::tv_due_lanes_choose_link_then_progress_then_search_without_updating_others`

Expected: FAIL until scheduling helpers exist.

- [ ] **Step 2: Add operation selection model**

Add:

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TvLaneKind { Link, Progress, Search }
```

Implement:

```rust
pub fn select_due_operation(record: &WantedSubscriptionRecord, now: u64) -> Option<SubscriptionDueOperation>;
pub fn select_due_tv_lane(record: &WantedSubscriptionRecord, now: u64) -> Option<TvLaneKind>;
```

Rules:

- Skip `completed`.
- Skip records tagged `skipped` unless `force_eligible_once` exists on parent/lane.
- Movie: select one operation for current lifecycle state when `next_attempt_at <= now` or forced.
- TV: run `TvMeta` while metadata guards are incomplete; otherwise select at most one lane by `link > progress > search`.
- Do not mutate lane timestamps during selection.

- [ ] **Step 3: Replace current queue logic**

Change `process_wanted_watch_queue()` to:

1. load snapshot,
2. iterate records,
3. call `select_due_operation(&record, now)`,
4. execute exactly one selected operation,
5. persist outcome and rederive status.

Remove the `for _ in 0..4` loop from automatic processing. Keep manual retry allowed to call one forced operation and then return the updated record.

- [ ] **Step 4: Run scheduling tests**

Run: `cargo test subscription::tests::movie_successful_transition_is_immediately_due_for_next_state subscription::tests::unchanged_movie_state_waits_for_state_interval subscription::tests::tv_due_lanes_choose_link_then_progress_then_search_without_updating_others`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/subscription.rs src/main.rs
git commit -m "feat: select unified subscription tick operations"
```

### Task 8: Implement Movie Lifecycle Operations

**Files:**
- Modify: `src/main.rs`
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing movie flow tests**

Add Rust tests around operation outcome helpers and existing API paths:

```rust
#[test]
fn movie_waiting_release_result_does_not_increment_retry() {
    let cfg = test_watcher_cfg();
    let mut record = movie_record_in_state(SubscriptionLifecycleState::Searching, 1_000);

    apply_movie_search_result(&mut record, SearchOperationResult::WaitingReleaseNoCandidates, &cfg, 2_000);

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Searching);
    assert!(record.attention_tags.contains(&SubscriptionAttentionTag::WaitingRelease));
    assert_eq!(record.failure, None);
    assert_eq!(record.retry_count, 0);
    assert_eq!(record.next_attempt_at, Some(2_000 + cfg.search_interval_secs));
}

#[test]
fn queued_skipped_manual_retry_preserves_cached_metadata_and_moves_to_meta() {
    let mut record = movie_record_with_cached_metadata();
    record.lifecycle_state = SubscriptionLifecycleState::Queued;
    record.attention_tags.push(SubscriptionAttentionTag::Skipped);

    apply_manual_retry(&mut record, RetryScope::Parent, 2_000).unwrap();

    assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Meta);
    assert!(!record.attention_tags.contains(&SubscriptionAttentionTag::Skipped));
    assert_eq!(record.title, "缓存标题");
    assert_eq!(record.date_published.as_deref(), Some("2026-07-01"));
}
```

Run: `cargo test subscription::tests::movie_waiting_release_result_does_not_increment_retry subscription::tests::queued_skipped_manual_retry_preserves_cached_metadata_and_moves_to_meta`

Expected: FAIL until movie outcomes and retry scope exist.

- [ ] **Step 2: Split movie operations from old retry actions**

Create operations in `src/main.rs`:

```rust
async fn process_movie_meta_operation(state: &AppState, account_key: &str, subject_id: &str) -> Result<(), ApiError>;
async fn process_movie_search_operation(state: &AppState, account_key: &str, subject_id: &str) -> Result<(), ApiError>;
async fn process_movie_progress_operation(state: &AppState, account_key: &str, subject_id: &str) -> Result<(), ApiError>;
async fn process_movie_link_operation(state: &AppState, account_key: &str, subject_id: &str) -> Result<(), ApiError>;
```

Implementation mapping:

- `queued -> meta`: mark metadata due immediately without external IO.
- `meta`: reuse cached rexxar detail if present; otherwise call existing rexxar fetch path; initialize media kind; advance to `searching`.
- `searching`: call existing candidate search/match/push logic. Return structured `pushed`, `waiting_release_no_candidates`, `waiting_release_no_match`, or `system_failed`.
- `downloading`: reuse qB lookup/progress logic. Keep `downloading` with progress interval when incomplete; advance to `linking` immediately when complete.
- `linking`: reuse hardlink plan/execution. Advance to `completed` on success; record scoped failure on system/link errors.

- [ ] **Step 3: Update failure and attention semantics**

Ensure:

- no-candidate/no-match clears `failed` and sets `waiting_release`,
- system errors set `failure.scope = Parent` and `failed`,
- retry exhaustion adds `retry_blocked`,
- success clears only parent failure and parent attention tags for that operation.

- [ ] **Step 4: Run movie flow tests**

Run: `cargo test subscription::tests::movie_waiting_release_result_does_not_increment_retry subscription::tests::queued_skipped_manual_retry_preserves_cached_metadata_and_moves_to_meta`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/subscription.rs
git commit -m "feat: run movie lifecycle operations"
```

### Task 9: Implement TV Search Lane

**Files:**
- Modify: `src/main.rs`
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing TV search tests**

Add tests:

```rust
#[test]
fn tv_search_requires_candidate_to_cover_cursor() {
    let mut record = tv_record_ready_for_search("subject-tv");
    record.tv.as_mut().unwrap().search_cursor_episode = Some(5);
    let candidates = vec![
        test_candidate("Show.S01E04.1080p"),
        test_candidate("Show.S01E05-E08.1080p"),
    ];

    let selected = select_tv_search_candidate(&record, &candidates, &test_match_rules()).unwrap();

    assert_eq!(selected.candidate.title, "Show.S01E05-E08.1080p");
}

#[test]
fn tv_search_push_range_advances_cursor_to_first_uncovered_episode() {
    let mut record = tv_record_ready_for_search("subject-tv");
    apply_tv_search_pushed(&mut record, test_task("task-e02-e04"), CoverageRange::range(1, 2, 4), 2_000).unwrap();

    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(5));
    assert_eq!(episode(record.tv.as_ref().unwrap(), 2).selected_task_id.as_deref(), Some("task-e02-e04"));
}

#[test]
fn tv_cursor_past_end_pauses_search_lane_only() {
    let mut record = tv_record_ready_for_search("subject-tv");
    record.tv.as_mut().unwrap().search_cursor_episode = Some(9);

    assert_eq!(select_due_tv_lane(&record, 2_000), None);
}
```

Run: `cargo test subscription::tests::tv_search_requires_candidate_to_cover_cursor subscription::tests::tv_search_push_range_advances_cursor_to_first_uncovered_episode subscription::tests::tv_cursor_past_end_pauses_search_lane_only`

Expected: FAIL until search lane exists.

- [ ] **Step 2: Implement coverage parsing and candidate filtering**

Reuse `episode_marker_for_file_name()` patterns by adding title-level helpers:

```rust
pub fn coverage_ranges_from_title(title: &str, season_hint: u32, episode_total: u32) -> Vec<CoverageRange>;
pub fn coverage_covers_cursor(coverage: &[CoverageRange], cursor: u32) -> bool;
pub fn select_tv_search_candidate(record: &WantedSubscriptionRecord, candidates: &[TorrentCandidateRecord], rules: &[TorrentMatchRule]) -> Option<TorrentCandidateMatchRecord>;
```

Rules:

- single episode, continuous range, and full-season titles are accepted.
- a candidate that does not cover `search_cursor_episode` is not selectable even if torrent rules match.
- when cursor is `episode_total + 1`, search lane is not due.

- [ ] **Step 3: Implement `process_tv_search_lane()`**

Create:

```rust
async fn process_tv_search_lane(state: &AppState, account_key: &str, subject_id: &str) -> Result<(), ApiError>;
```

Flow:

1. load record and TV state,
2. if cursor is past end, set search lane `next_attempt_at = None` and return,
3. search M-Team with existing `search_mteam_candidates_for_record()`,
4. filter by cursor coverage and torrent rules,
5. if no candidates: set lane result `waiting_release_no_candidates`, add cursor-level waiting release, `next_attempt_at = now + search_interval_secs`, no retry increment,
6. if candidates but no match: set `waiting_release_no_match`, no retry increment,
7. push selected torrent using existing qB push logic,
8. create `DownloadTaskRecord` with tentative coverage,
9. bind task to episodes and recalculate cursor,
10. set search lane `next_attempt_at = now` after push so next tick can continue if another cursor remains.

- [ ] **Step 4: Run TV search tests**

Run: `cargo test subscription::tests::tv_search_requires_candidate_to_cover_cursor subscription::tests::tv_search_push_range_advances_cursor_to_first_uncovered_episode subscription::tests::tv_cursor_past_end_pauses_search_lane_only`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/subscription.rs
git commit -m "feat: process tv search lane"
```

### Task 10: Implement TV Progress Lane

**Files:**
- Modify: `src/main.rs`
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing TV progress tests**

Add tests:

```rust
#[test]
fn tv_progress_verified_coverage_smaller_than_tentative_releases_missing_episodes() {
    let mut record = tv_record_with_task_range("task-e02-e04", 2, 4);

    apply_tv_task_progress(
        &mut record,
        "task-e02-e04",
        TaskProgressSnapshot {
            complete: false,
            verified_coverage: vec![CoverageRange::single(1, 2)],
            progress: Some(0.5),
            missing: false,
        },
        2_000,
    ).unwrap();

    assert_eq!(episode(record.tv.as_ref().unwrap(), 3).assignment_state, EpisodeAssignmentState::Released);
    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(3));
}

#[test]
fn tv_task_retry_blocked_does_not_block_other_tasks_or_search_cursor() {
    let mut record = tv_record_with_task_range("task-e03", 3, 3);

    block_tv_task(&mut record, "task-e03", test_failure("progress", true), 2_000).unwrap();

    assert_eq!(episode(record.tv.as_ref().unwrap(), 3).assignment_state, EpisodeAssignmentState::Blocked);
    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(1));
    assert!(record.attention_tags.contains(&SubscriptionAttentionTag::RetryBlocked));
}
```

Run: `cargo test subscription::tests::tv_progress_verified_coverage_smaller_than_tentative_releases_missing_episodes subscription::tests::tv_task_retry_blocked_does_not_block_other_tasks_or_search_cursor`

Expected: FAIL until progress helpers exist.

- [ ] **Step 2: Implement progress selection**

Add:

```rust
pub fn due_tv_progress_tasks(tv: &TvSubscriptionState) -> Vec<String>;
pub fn apply_tv_task_progress(record: &mut WantedSubscriptionRecord, task_id: &str, snapshot: TaskProgressSnapshot, now: u64) -> Result<(), String>;
pub fn block_tv_task(record: &mut WantedSubscriptionRecord, task_id: &str, failure: SubscriptionFailure, now: u64) -> Result<(), String>;
```

Rules:

- Progress lane scans active tasks in `Pushed` or `Downloading`.
- If qB task missing, mark task `Missing`, release unverified unfinished episodes, and recalculate cursor.
- If file list verifies less coverage than tentative, release unverified unfinished episodes.
- If qB reports complete, mark task `Downloaded` and make link lane immediately due.
- If still downloading, keep lane in same stage and set `progress_next_attempt_at = now + progress_interval_secs`.
- System failure increments retry on that task only; retry-blocked task creates blocked assignments but does not block other task sync.

- [ ] **Step 3: Wire `process_tv_progress_lane()`**

Create:

```rust
async fn process_tv_progress_lane(state: &AppState, account_key: &str, subject_id: &str) -> Result<(), ApiError>;
```

Use existing `find_qb_torrent_for_push()`, `qbittorrent::torrent_files()`, and `apply_qb_progress_to_push()` logic by constructing temporary `TorrentPushRecord` from `DownloadTaskRecord`, then mapping qB output back into the task and episode model.

- [ ] **Step 4: Run TV progress tests**

Run: `cargo test subscription::tests::tv_progress_verified_coverage_smaller_than_tentative_releases_missing_episodes subscription::tests::tv_task_retry_blocked_does_not_block_other_tasks_or_search_cursor`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/subscription.rs
git commit -m "feat: process tv progress lane"
```

### Task 11: Implement TV Link Lane

**Files:**
- Modify: `src/main.rs`
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing TV link tests**

Add tests:

```rust
#[test]
fn tv_link_success_completes_only_verified_episode_assignments() {
    let mut record = tv_record_with_downloaded_task("task-e01-e02", 1, 2);

    apply_tv_link_result(&mut record, "task-e01-e02", test_link_result_for_episodes(&[1, 2]), 2_000).unwrap();

    assert_eq!(episode(record.tv.as_ref().unwrap(), 1).assignment_state, EpisodeAssignmentState::Completed);
    assert_eq!(episode(record.tv.as_ref().unwrap(), 2).link_state, "linked");
}

#[test]
fn skipping_episode_bound_to_task_releases_only_that_episode() {
    let mut record = tv_record_with_downloaded_task("task-e01-e02", 1, 2);

    skip_episode_range(record.tv.as_mut().unwrap(), 1, 1, 1, "user skipped", 2_000).unwrap();

    assert_eq!(episode(record.tv.as_ref().unwrap(), 1).assignment_state, EpisodeAssignmentState::Skipped);
    assert_eq!(episode(record.tv.as_ref().unwrap(), 1).selected_task_id, None);
    assert_eq!(episode(record.tv.as_ref().unwrap(), 2).selected_task_id.as_deref(), Some("task-e01-e02"));
}
```

Run: `cargo test subscription::tests::tv_link_success_completes_only_verified_episode_assignments subscription::tests::skipping_episode_bound_to_task_releases_only_that_episode`

Expected: FAIL until link helpers exist.

- [ ] **Step 2: Extend hardlink planning for TV scoped episodes**

Add a TV-specific builder:

```rust
fn build_tv_hardlink_plan(
    record: &subscription::WantedSubscriptionRecord,
    category: &SubscriptionCategory,
    task: &subscription::DownloadTaskRecord,
    torrent: &qbittorrent::QbTorrentInfo,
    files: &[qbittorrent::QbTorrentFile],
    now: u64,
) -> Result<HardlinkPlan, ApiError>;
```

Rules:

- Only include files for non-skipped episodes selected by the task.
- Lock target path by episode label before execution to avoid duplicate writes.
- Do not replace already completed episode files.
- If the task only has skipped episodes remaining, mark it as historical and do not enter link lane.

- [ ] **Step 3: Wire `process_tv_link_lane()`**

Create:

```rust
async fn process_tv_link_lane(state: &AppState, account_key: &str, subject_id: &str) -> Result<(), ApiError>;
```

Flow:

1. choose downloaded/linkable task or episode,
2. get qB files,
3. build scoped hardlink plan,
4. execute or dry-run according to existing handler behavior,
5. mark linked episodes `Completed`,
6. mark task `Completed` when all non-skipped assigned episodes are complete,
7. on link failure, attach failure to episode/task only; retry-blocked episode becomes blocked assignment and does not prevent other episodes linking.

- [ ] **Step 4: Run TV link tests**

Run: `cargo test subscription::tests::tv_link_success_completes_only_verified_episode_assignments subscription::tests::skipping_episode_bound_to_task_releases_only_that_episode`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/subscription.rs
git commit -m "feat: process tv link lane"
```

### Task 12: Add Scoped Manual Retry and Skip APIs

**Files:**
- Modify: `src/main.rs`
- Modify: `src/subscription.rs`

- [ ] **Step 1: Write failing manual operation tests**

Add tests:

```rust
#[test]
fn retry_blocked_tv_episode_becomes_immediately_eligible_without_clearing_other_failures() {
    let mut record = tv_record_with_blocked_episode(3);
    let other_failure = test_failure("progress", true);
    record.tv.as_mut().unwrap().lanes.progress.failure = Some(other_failure.clone());

    apply_manual_retry(&mut record, RetryScope::Episode { season: 1, episode: 3 }, 2_000).unwrap();

    assert_eq!(episode(record.tv.as_ref().unwrap(), 3).assignment_state, EpisodeAssignmentState::Released);
    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(3));
    assert_eq!(record.tv.as_ref().unwrap().lanes.progress.failure, Some(other_failure));
}

#[test]
fn unskip_episode_recomputes_cursor_to_uncovered_episode() {
    let mut record = tv_record_all_complete_except_skipped(5);

    apply_unskip_episode_range(&mut record, 1, 5, 5, 2_000).unwrap();

    assert_eq!(episode(record.tv.as_ref().unwrap(), 5).assignment_state, EpisodeAssignmentState::Released);
    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(5));
}
```

Run: `cargo test subscription::tests::retry_blocked_tv_episode_becomes_immediately_eligible_without_clearing_other_failures subscription::tests::unskip_episode_recomputes_cursor_to_uncovered_episode`

Expected: FAIL until scoped retry/skip helpers exist.

- [ ] **Step 2: Add request/handler types**

Add endpoint bodies:

```rust
#[derive(Deserialize)]
struct SubscriptionRetryScopeBody {
    scope: String,
    lane: Option<String>,
    task_id: Option<String>,
    season: Option<u32>,
    episode: Option<u32>,
}

#[derive(Deserialize)]
struct SubscriptionEpisodeRangeBody {
    season: u32,
    start_episode: u32,
    end_episode: u32,
    reason: Option<String>,
}
```

Add routes:

```rust
.route("/subscriptions/wanted/{id}/retry", post(wanted_subscription_retry_scope))
.route("/subscriptions/wanted/{id}/episodes/skip", post(wanted_subscription_skip_episodes))
.route("/subscriptions/wanted/{id}/episodes/unskip", post(wanted_subscription_unskip_episodes))
```

Keep existing `/retry-current` and `/rerun` as compatibility wrappers that call the new parent retry or movie rerun path.

- [ ] **Step 3: Implement retry semantics**

Implement `apply_manual_retry(record, scope, now)`:

- parent `queued(skipped)`: clear skipped, set `meta`, preserve cached metadata.
- lane `search`: clear search lane blocked/failure, set `force_eligible_once = true`, `next_attempt_at = now`.
- task: clear task failure, allow immediate progress.
- episode: clear episode failure, release blocked assignment when needed, recalculate cursor or make link lane due.
- completed: return bad request.

- [ ] **Step 4: Run manual operation tests**

Run: `cargo test subscription::tests::retry_blocked_tv_episode_becomes_immediately_eligible_without_clearing_other_failures subscription::tests::unskip_episode_recomputes_cursor_to_uncovered_episode`

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/subscription.rs
git commit -m "feat: add scoped subscription manual operations"
```

### Task 13: Update API Response Shape and Frontend Display

**Files:**
- Modify: `frontend/src/App.vue`
- Modify: `frontend/src/components/SubscriptionDetailView.vue`
- Modify: `frontend/src/styles.css`
- Create: `frontend/src/__tests__/subscription-state-machine-display.test.mjs`

- [ ] **Step 1: Write failing frontend display tests**

Create `frontend/src/__tests__/subscription-state-machine-display.test.mjs` with assertions:

```javascript
assert.equal(
  helpers.subscriptionDisplayStatus({ lifecycle_state: "linking", execution_state: "idle" }).text,
  "硬链接中",
);

assert.deepEqual(
  plain(helpers.subscriptionCardNotices({
    lifecycle_state: "downloading",
    attention_tags: ["waiting_release"],
    tv: { search_cursor_episode: 5 },
  })),
  [{ key: "waiting-release", kind: "stage", text: "等待 E05 发布" }],
);

assert.equal(
  helpers.subscriptionDisplayStatus({
    lifecycle_state: "searching",
    attention_tags: ["retry_blocked", "failed", "waiting_release"],
  }).key,
  "retry_blocked",
);

assert.match(
  subscriptionDetailSource,
  /subscription-episode-matrix/,
  "TV detail should render an episode matrix",
);

assert.match(
  subscriptionDetailSource,
  /retrySubscriptionScope/,
  "manual retry buttons should call scoped retry",
);
```

Run: `node frontend/src/__tests__/subscription-state-machine-display.test.mjs`

Expected: FAIL until frontend helpers/template are updated.

- [ ] **Step 2: Update display constants and helpers**

In `frontend/src/App.vue`, replace old status-first display with lifecycle/tag display:

```javascript
const SUB_LIFECYCLE_LABELS = {
  queued: "已入队",
  meta: "准备元数据",
  searching: "搜索中",
  downloading: "下载中",
  linking: "硬链接中",
  completed: "已完成",
};

const ATTENTION_PRIORITY = ["skipped", "retry_blocked", "failed", "waiting_release"];
```

`subscriptionDisplayStatus(record)` should:

1. choose the first tag in priority,
2. otherwise return `running` when `execution_state === "running"`,
3. otherwise return the lifecycle label.

`subscriptionCardNotices(record)` should include:

- `waiting_release` with cursor label such as `等待 E05 发布`,
- failed child count such as `2 个分集失败`,
- lane next attempt such as `下次搜索 30 分钟后`.

- [ ] **Step 3: Update detail component**

In `SubscriptionDetailView.vue`:

- Render parent lifecycle and tags in the header.
- Render TV summary counts: target total, covered, downloading, linked, failed, skipped.
- Add `<section class="subscription-episode-matrix">` for episodes with states: uncovered, waiting release, downloading, downloaded, linking, linked, failed, skipped.
- Add download task list with title, coverage, qB progress, state, and retry action.
- Replace broad retry/rerun buttons with scoped buttons:
  - parent retry,
  - search lane retry,
  - task progress retry,
  - episode link retry,
  - skip/unskip episode.

- [ ] **Step 4: Add styles**

In `frontend/src/styles.css`, add stable compact layouts:

```css
.subscription-episode-matrix {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(48px, 1fr));
  gap: 6px;
}

.subscription-episode-cell {
  min-height: 44px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.subscription-task-list {
  display: grid;
  gap: 8px;
}
```

- [ ] **Step 5: Run frontend display test**

Run: `node frontend/src/__tests__/subscription-state-machine-display.test.mjs`

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/App.vue frontend/src/components/SubscriptionDetailView.vue frontend/src/styles.css frontend/src/__tests__/subscription-state-machine-display.test.mjs
git commit -m "feat: display subscription lifecycle state machine"
```

### Task 14: End-to-End Regression and Acceptance Coverage

**Files:**
- Modify: `src/subscription.rs`
- Modify: `src/main.rs`
- Modify: `frontend/src/__tests__/subscription-state-machine-display.test.mjs`

- [ ] **Step 1: Add acceptance regression tests**

Add Rust tests covering the PRD acceptance examples:

```rust
#[test]
fn tv_acceptance_cursor_examples_match_prd() {
    let mut record = tv_record_ready_for_search("subject-tv");

    apply_tv_search_pushed(&mut record, test_task("e01"), CoverageRange::single(1, 1), 1_000).unwrap();
    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(2));

    apply_tv_search_pushed(&mut record, test_task("e02-e04"), CoverageRange::range(1, 2, 4), 2_000).unwrap();
    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(5));

    apply_tv_search_pushed(&mut record, test_task("e05-e08"), CoverageRange::range(1, 5, 8), 3_000).unwrap();
    assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(9));
}

#[test]
fn link_progress_search_due_same_tick_executes_only_link() {
    let record = tv_record_all_lanes_due(5_000);
    assert_eq!(select_due_operation(&record, 5_000), Some(SubscriptionDueOperation::TvLane(TvLaneKind::Link)));
}

#[test]
fn failed_child_blocks_completion_but_not_later_search() {
    let mut record = tv_record_ready_for_search("subject-tv");
    block_episode_assignment(record.tv.as_mut().unwrap(), 1, 3, test_failure("link", true)).unwrap();
    bind_task_to_episodes(record.tv.as_mut().unwrap(), "e04-e08", CoverageRange::range(1, 4, 8), CoverageTrust::Tentative, 2_000).unwrap();
    complete_all_except(record.tv.as_mut().unwrap(), &[3]);
    derive_tv_parent_lifecycle(&mut record);

    assert_ne!(record.lifecycle_state, SubscriptionLifecycleState::Completed);
    assert!(record.attention_tags.contains(&SubscriptionAttentionTag::RetryBlocked));
}
```

Run: `cargo test subscription::tests::tv_acceptance_cursor_examples_match_prd subscription::tests::link_progress_search_due_same_tick_executes_only_link subscription::tests::failed_child_blocks_completion_but_not_later_search`

Expected: PASS after earlier tasks.

- [ ] **Step 2: Run focused backend test groups**

Run:

```bash
cargo test subscription::tests::lifecycle_statuses_exclude_execution_and_attention_states
cargo test subscription::tests::legacy_failed_records_follow_prd_precedence
cargo test subscription::tests::tv_acceptance_cursor_examples_match_prd
cargo test subscription::tests::link_progress_search_due_same_tick_executes_only_link
cargo test subscription::tests::failed_child_blocks_completion_but_not_later_search
cargo test config::tests::subscription_watcher_defaults_include_lane_intervals
```

Expected: all commands exit 0.

- [ ] **Step 3: Run frontend focused tests**

Run:

```bash
node frontend/src/__tests__/subscription-card-display.test.mjs
node frontend/src/__tests__/subscription-state-machine-display.test.mjs
```

Expected: both commands exit 0.

- [ ] **Step 4: Run broad verification**

Run:

```bash
cargo test
npm run build
```

Expected: both commands exit 0.

- [ ] **Step 5: Commit**

```bash
git add src/subscription.rs src/main.rs frontend/src/__tests__/subscription-state-machine-display.test.mjs
git commit -m "test: cover subscription state machine acceptance cases"
```

---

## Self-Review Checklist

- [ ] Main lifecycle state is limited to `queued`, `meta`, `searching`, `downloading`, `linking`, `completed`.
- [ ] `failed`, `skipped`, `running`, and `idle` are not authoritative lifecycle states.
- [ ] Movie flow remains linear and one operation per tick.
- [ ] Successful state transition sets next attempt to `now`; unchanged state/lane applies interval.
- [ ] TV scheduling is lane-based and runs at most one due lane per tick with `link > progress > search`.
- [ ] TV cursor requires selected torrents to cover the cursor.
- [ ] Tentative coverage can advance the cursor, but verified coverage loss releases unverified episodes.
- [ ] Blocked assignments do not block later search, but they prevent parent completion.
- [ ] Parent TV completion requires all unskipped target episodes linked and no active work.
- [ ] Metadata guards prevent TV from deriving `searching` or `completed` before metadata and episode targets are ready.
- [ ] Waiting release is structured and does not increment retry.
- [ ] Manual retry clears only the requested scope and makes that scope immediately eligible.
- [ ] Migration follows the PRD precedence for old `failed` records and preserves old JSON for rollback.
- [ ] API/frontend expose lifecycle, tags, lane failures, episode records, task records, and scoped actions.
