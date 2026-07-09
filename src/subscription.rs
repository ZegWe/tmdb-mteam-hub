use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use rusqlite::{params, params_from_iter, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::config::SubscriptionWatcherConfig;
use crate::douban::{DoubanLibraryItem, DoubanSubjectDetail};

const STATE_VERSION: u32 = 1;
const DB_SCHEMA_VERSION: i64 = 3;
const DB_FILE_NAME: &str = "wanted.sqlite";

fn default_state_version() -> u32 {
    STATE_VERSION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WantedSubscriptionStatus {
    Unprocessed,
    Matching,
    Processing,
    Pushed,
    Downloading,
    Completed,
    Linked,
    Failed,
    Skipped,
}

impl Default for WantedSubscriptionStatus {
    fn default() -> Self {
        Self::Unprocessed
    }
}

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

impl Default for SubscriptionLifecycleState {
    fn default() -> Self {
        Self::Queued
    }
}

impl SubscriptionLifecycleState {
    #[allow(dead_code)]
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

impl Default for SubscriptionExecutionState {
    fn default() -> Self {
        Self::Idle
    }
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

impl Default for SubscriptionMediaKind {
    fn default() -> Self {
        Self::Movie
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OperationLaneState {
    pub next_attempt_at: Option<u64>,
    pub retry_count: u32,
    pub max_retries: u32,
    pub force_eligible_once: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<SubscriptionFailure>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TvTargetRange {
    pub season_number: u32,
    pub start_episode: u32,
    pub end_episode: u32,
    pub episode_total: u32,
}

impl TvTargetRange {
    #[allow(dead_code)]
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
    #[allow(dead_code)]
    pub fn single(season_number: u32, episode_number: u32) -> Self {
        Self {
            season_number,
            start_episode: episode_number,
            end_episode: episode_number,
        }
    }

    #[allow(dead_code)]
    pub fn range(season_number: u32, start_episode: u32, end_episode: u32) -> Self {
        Self {
            season_number,
            start_episode,
            end_episode,
        }
    }

    #[allow(dead_code)]
    pub fn contains_episode(self, episode_number: u32) -> bool {
        self.start_episode <= episode_number && episode_number <= self.end_episode
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageTrust {
    Tentative,
    Verified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeTargetState {
    Target,
    Skipped,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeAssignmentState {
    None,
    Active,
    Blocked,
    Released,
    Completed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeCoverageState {
    Uncovered,
    TentativeCovered,
    VerifiedCovered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadTaskState {
    Pushed,
    Downloading,
    Downloaded,
    Linking,
    Completed,
    Missing,
    Failed,
    Ignored,
    Superseded,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionDueOperation {
    MovieMeta,
    MovieSearch,
    MovieProgress,
    MovieLink,
    TvMeta,
    TvLane(TvLaneKind),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TvLaneKind {
    Link,
    Progress,
    Search,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovieOperationOutcome {
    Advanced(SubscriptionLifecycleState),
    StillDownloading,
    Waiting,
}

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentCandidateRecord {
    #[serde(default)]
    pub torrent_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub search_query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seeders: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub leechers: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uploaded_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateRuleEvaluation {
    pub rule_name: String,
    pub priority: i32,
    pub mode: String,
    pub matched: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_keywords: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentCandidateMatchRecord {
    pub candidate: TorrentCandidateRecord,
    pub selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_rule_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_priority: Option<i32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_keywords: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub excluded_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rule_evaluations: Vec<CandidateRuleEvaluation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentPushRecord {
    #[serde(default)]
    pub subscription_id: String,
    #[serde(default)]
    pub torrent_id: String,
    #[serde(default)]
    pub torrent_title: String,
    #[serde(default)]
    pub qb_server: String,
    #[serde(default)]
    pub qb_server_id: String,
    #[serde(default)]
    pub qb_category: String,
    #[serde(default)]
    pub qb_save_dir_name: String,
    #[serde(default)]
    pub qb_identifier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub torrent_download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mteam_torrent_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<u64>,
    #[serde(default)]
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qb_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qb_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_progress: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_file_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_file_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<TorrentFileProgressRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episodes: Vec<EpisodeProgressRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_files: Vec<HardlinkFileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentFileProgressRecord {
    pub name: String,
    pub size: u64,
    pub progress: f64,
    pub priority: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_end_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EpisodeProgressRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_number: Option<u32>,
    pub label: String,
    pub file_count: usize,
    pub completed_file_count: usize,
    pub linked_file_count: usize,
    pub failed_file_count: usize,
    pub progress: f64,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardlinkFileRecord {
    #[serde(default)]
    pub source_path: String,
    #[serde(default)]
    pub target_path: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_end_number: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardlinkCompletionRecord {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub checked_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qb_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qb_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_files: Vec<HardlinkFileRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub episodes: Vec<EpisodeProgressRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WantedSubscriptionRecord {
    #[serde(default)]
    pub subject_id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub release_year: Option<u16>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub poster_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cover_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_title: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aka: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub countries: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directors: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_published: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_value: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rating_count: Option<u64>,
    #[serde(default)]
    pub category_text: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub douban_date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub douban_sort_time: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub douban_return_order: Option<u32>,
    #[serde(default)]
    pub status: WantedSubscriptionStatus,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tv: Option<TvSubscriptionState>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub max_retries: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processing_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage_updated_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_matches: Vec<TorrentCandidateMatchRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_push: Option<TorrentPushRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_completion: Option<HardlinkCompletionRecord>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(default)]
    pub first_seen_at: u64,
    #[serde(default)]
    pub last_seen_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WantedSubscriptionState {
    #[serde(default = "default_state_version")]
    pub version: u32,
    #[serde(default)]
    pub account_key: String,
    #[serde(default)]
    pub bootstrap_completed: bool,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_poll_at: Option<u64>,
    pub records: BTreeMap<String, WantedSubscriptionRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WantedPollOutcome {
    pub account_key: String,
    pub total_wish_items: usize,
    pub created_unprocessed: usize,
    pub created_skipped: usize,
    pub updated_existing: usize,
    pub bootstrap_completed: bool,
    pub bootstrap_mode: bool,
    pub state_path: String,
    pub polled_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct WantedStatusUpdateOutcome {
    pub record: WantedSubscriptionRecord,
    pub retry_exhausted: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WantedStatusUpdate {
    pub status: WantedSubscriptionStatus,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub skip_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationLogEntry {
    pub id: u64,
    #[serde(default)]
    pub account_key: String,
    pub created_at: u64,
    pub category: String,
    pub action: String,
    pub target_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_title: Option<String>,
    pub status: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub related: Value,
}

#[derive(Debug, Clone)]
pub struct NewOperationLogEntry {
    pub account_key: String,
    pub created_at: u64,
    pub category: String,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub target_title: Option<String>,
    pub status: String,
    pub summary: String,
    pub error: Option<String>,
    pub related: Value,
}

#[derive(Debug, Clone, Default)]
pub struct OperationLogQuery {
    pub category: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OperationLogPage {
    pub items: Vec<OperationLogEntry>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub has_more: bool,
}

#[derive(Clone)]
pub struct WantedSubscriptionStore {
    root: PathBuf,
    db_path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl WantedSubscriptionStore {
    pub fn new(root: PathBuf) -> Self {
        let db_path = root.join(DB_FILE_NAME);
        Self {
            root,
            db_path,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn db_path(&self) -> &PathBuf {
        &self.db_path
    }

    pub async fn snapshot(
        &self,
        account_key: &str,
        now: u64,
    ) -> std::io::Result<WantedSubscriptionState> {
        let _guard = self.lock.lock().await;
        self.load_state_unlocked(account_key, now).await
    }

    #[allow(dead_code)]
    pub async fn apply_wish_items(
        &self,
        account_key: &str,
        items: &[DoubanLibraryItem],
        cfg: &SubscriptionWatcherConfig,
        now: u64,
    ) -> std::io::Result<WantedPollOutcome> {
        self.apply_wish_items_with_details(account_key, items, &BTreeMap::new(), cfg, now)
            .await
    }

    pub async fn apply_wish_items_with_details(
        &self,
        account_key: &str,
        items: &[DoubanLibraryItem],
        details: &BTreeMap<String, DoubanSubjectDetail>,
        cfg: &SubscriptionWatcherConfig,
        now: u64,
    ) -> std::io::Result<WantedPollOutcome> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state_unlocked(account_key, now).await?;
        let outcome = apply_wish_items_with_details_to_state(
            &mut state,
            items,
            details,
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            self.db_path.display().to_string(),
            now,
        );
        self.save_state_unlocked(account_key, &state)?;
        Ok(outcome)
    }

    pub async fn update_status(
        &self,
        account_key: &str,
        subject_id: &str,
        update: WantedStatusUpdate,
        max_retries: u32,
        now: u64,
    ) -> std::io::Result<Option<WantedStatusUpdateOutcome>> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state_unlocked(account_key, now).await?;
        let Some((record, retry_exhausted)) =
            state.records.get_mut(subject_id.trim()).map(|record| {
                let retry_exhausted = apply_status_update(record, update, max_retries, now);
                (record.clone(), retry_exhausted)
            })
        else {
            return Ok(None);
        };
        state.updated_at = now;
        self.save_state_unlocked(account_key, &state)?;
        Ok(Some(WantedStatusUpdateOutcome {
            record,
            retry_exhausted,
        }))
    }

    pub async fn update_candidate_matches(
        &self,
        account_key: &str,
        subject_id: &str,
        matches: Vec<TorrentCandidateMatchRecord>,
        now: u64,
    ) -> std::io::Result<Option<WantedSubscriptionRecord>> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state_unlocked(account_key, now).await?;
        let Some(record) = state.records.get_mut(subject_id.trim()) else {
            return Ok(None);
        };
        record.candidate_matches = matches;
        record.updated_at = now;
        apply_candidate_stage(record, now);
        state.updated_at = now;
        let record = record.clone();
        self.save_state_unlocked(account_key, &state)?;
        Ok(Some(record))
    }

    pub async fn update_sync_error(
        &self,
        account_key: &str,
        subject_id: &str,
        status: WantedSubscriptionStatus,
        error: String,
        now: u64,
    ) -> std::io::Result<Option<WantedSubscriptionRecord>> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state_unlocked(account_key, now).await?;
        let Some(record) = state.records.get_mut(subject_id.trim()) else {
            return Ok(None);
        };
        record.status = status;
        record.last_error = (!error.trim().is_empty()).then_some(error);
        record.updated_at = now;
        let message = record
            .last_error
            .clone()
            .unwrap_or_else(|| "订阅处理失败，等待检查配置或手动重试".to_string());
        set_stage(
            record,
            "error",
            &message,
            Some("检查错误后重新轮询或手动重试"),
            now,
        );
        state.updated_at = now;
        let record = record.clone();
        self.save_state_unlocked(account_key, &state)?;
        Ok(Some(record))
    }

    pub async fn update_next_attempt_at(
        &self,
        account_key: &str,
        subject_id: &str,
        next_attempt_at: Option<u64>,
        now: u64,
    ) -> std::io::Result<Option<WantedSubscriptionRecord>> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state_unlocked(account_key, now).await?;
        let Some(record) = state.records.get_mut(subject_id.trim()) else {
            return Ok(None);
        };
        record.next_attempt_at = next_attempt_at;
        record.force_eligible_once = false;
        record.updated_at = now;
        state.updated_at = now;
        let record = record.clone();
        self.save_state_unlocked(account_key, &state)?;
        Ok(Some(record))
    }

    pub async fn transition_movie_operation(
        &self,
        account_key: &str,
        subject_id: &str,
        outcome: MovieOperationOutcome,
        cfg: &SubscriptionWatcherConfig,
        now: u64,
    ) -> std::io::Result<Option<WantedSubscriptionRecord>> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state_unlocked(account_key, now).await?;
        let Some(record) = state.records.get_mut(subject_id.trim()) else {
            return Ok(None);
        };
        apply_movie_operation_outcome(record, outcome, cfg, now);
        state.updated_at = now;
        let record = record.clone();
        self.save_state_unlocked(account_key, &state)?;
        Ok(Some(record))
    }

    pub async fn update_push_record(
        &self,
        account_key: &str,
        subject_id: &str,
        push: TorrentPushRecord,
        status: WantedSubscriptionStatus,
        error: Option<String>,
        now: u64,
    ) -> std::io::Result<Option<WantedSubscriptionRecord>> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state_unlocked(account_key, now).await?;
        let Some(record) = state.records.get_mut(subject_id.trim()) else {
            return Ok(None);
        };
        record.status = status;
        record.last_push = Some(push);
        record.last_error = error.filter(|s| !s.trim().is_empty());
        record.updated_at = now;
        apply_push_stage(record, now);
        state.updated_at = now;
        let record = record.clone();
        self.save_state_unlocked(account_key, &state)?;
        Ok(Some(record))
    }

    pub async fn update_completion_record(
        &self,
        account_key: &str,
        subject_id: &str,
        push: TorrentPushRecord,
        completion: HardlinkCompletionRecord,
        status: WantedSubscriptionStatus,
        error: Option<String>,
        now: u64,
    ) -> std::io::Result<Option<WantedSubscriptionRecord>> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state_unlocked(account_key, now).await?;
        let Some(record) = state.records.get_mut(subject_id.trim()) else {
            return Ok(None);
        };
        record.status = status;
        record.last_push = Some(push);
        record.last_completion = Some(completion);
        record.last_error = error.filter(|s| !s.trim().is_empty());
        record.updated_at = now;
        apply_completion_stage(record, now);
        state.updated_at = now;
        let record = record.clone();
        self.save_state_unlocked(account_key, &state)?;
        Ok(Some(record))
    }

    pub async fn append_operation_log(
        &self,
        entry: NewOperationLogEntry,
    ) -> std::io::Result<OperationLogEntry> {
        let _guard = self.lock.lock().await;
        let conn = self.open_initialized_connection()?;
        append_operation_log_to_db(&conn, entry)
    }

    pub async fn query_operation_logs(
        &self,
        query: OperationLogQuery,
    ) -> std::io::Result<OperationLogPage> {
        let _guard = self.lock.lock().await;
        let conn = self.open_initialized_connection()?;
        query_operation_logs_from_db(&conn, query)
    }

    fn path_for(&self, account_key: &str) -> PathBuf {
        self.root
            .join(format!("wanted_{}.json", safe_account_key(account_key)))
    }

    fn connection_unlocked(&self) -> std::io::Result<Connection> {
        std::fs::create_dir_all(&self.root)?;
        match self.open_initialized_connection() {
            Ok(conn) => Ok(conn),
            Err(err) if is_sqlite_recoverable_corruption(&err) => {
                backup_corrupt_db(&self.db_path)?;
                self.open_initialized_connection()
            }
            Err(err) => Err(err),
        }
    }

    fn open_initialized_connection(&self) -> std::io::Result<Connection> {
        let conn = Connection::open(&self.db_path).map_err(sqlite_io)?;
        init_schema(&conn)?;
        Ok(conn)
    }

    async fn load_state_unlocked(
        &self,
        account_key: &str,
        now: u64,
    ) -> std::io::Result<WantedSubscriptionState> {
        {
            let conn = self.connection_unlocked()?;
            if let Some(mut state) = load_state_from_db(&conn, account_key)? {
                repair_state_defaults(&mut state, account_key, now);
                return Ok(state);
            }
        }

        if let Some(mut state) = self.load_legacy_json_state(account_key).await? {
            repair_state_defaults(&mut state, account_key, now);
            self.save_state_unlocked(account_key, &state)?;
            return Ok(state);
        }

        Ok(WantedSubscriptionState::new(account_key, now))
    }

    fn save_state_unlocked(
        &self,
        account_key: &str,
        state: &WantedSubscriptionState,
    ) -> std::io::Result<()> {
        let mut conn = self.connection_unlocked()?;
        save_state_to_db(&mut conn, account_key, state)
    }

    async fn load_legacy_json_state(
        &self,
        account_key: &str,
    ) -> std::io::Result<Option<WantedSubscriptionState>> {
        let path = self.path_for(account_key);
        match tokio::fs::read_to_string(&path).await {
            Ok(raw) => {
                let state = serde_json::from_str(&raw).map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("解析订阅状态文件失败 {}: {e}", path.display()),
                    )
                })?;
                Ok(Some(state))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
}

fn init_schema(conn: &Connection) -> std::io::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS subscription_schema_meta (
            key TEXT NOT NULL,
            value INTEGER NOT NULL
        )",
        [],
    )
    .map_err(sqlite_io)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS subscription_meta (
            account_key TEXT NOT NULL,
            version INTEGER NOT NULL,
            bootstrap_completed INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_poll_at INTEGER
        )",
        [],
    )
    .map_err(sqlite_io)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS wanted_subscription_records (
            account_key TEXT NOT NULL,
            subject_id TEXT NOT NULL,
            status TEXT NOT NULL,
            title TEXT NOT NULL,
            category_text TEXT,
            updated_at INTEGER NOT NULL,
            record_json TEXT NOT NULL
        )",
        [],
    )
    .map_err(sqlite_io)?;
    ensure_column(
        conn,
        "wanted_subscription_records",
        "lifecycle_state",
        "TEXT NOT NULL DEFAULT 'queued'",
    )?;
    ensure_column(
        conn,
        "wanted_subscription_records",
        "execution_state",
        "TEXT NOT NULL DEFAULT 'idle'",
    )?;
    ensure_column(
        conn,
        "wanted_subscription_records",
        "attention_tags_json",
        "TEXT NOT NULL DEFAULT '[]'",
    )?;
    ensure_column(
        conn,
        "wanted_subscription_records",
        "media_kind",
        "TEXT NOT NULL DEFAULT 'movie'",
    )?;
    ensure_column(
        conn,
        "wanted_subscription_records",
        "next_attempt_at",
        "INTEGER",
    )?;
    ensure_column(
        conn,
        "wanted_subscription_records",
        "search_next_attempt_at",
        "INTEGER",
    )?;
    ensure_column(
        conn,
        "wanted_subscription_records",
        "progress_next_attempt_at",
        "INTEGER",
    )?;
    ensure_column(
        conn,
        "wanted_subscription_records",
        "link_next_attempt_at",
        "INTEGER",
    )?;
    ensure_column(
        conn,
        "wanted_subscription_records",
        "retry_blocked_count",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS wanted_records_lifecycle_due_idx
            ON wanted_subscription_records (account_key, lifecycle_state, execution_state, next_attempt_at)",
        [],
    )
    .map_err(sqlite_io)?;
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS wanted_records_account_subject_uidx
            ON wanted_subscription_records (account_key, subject_id)",
        [],
    )
    .map_err(sqlite_io)?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS wanted_records_tv_lane_due_idx
            ON wanted_subscription_records (account_key, media_kind, search_next_attempt_at, progress_next_attempt_at, link_next_attempt_at)",
        [],
    )
    .map_err(sqlite_io)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS subscription_state_blobs (
            account_key TEXT NOT NULL,
            state_json TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(sqlite_io)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS operation_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_key TEXT NOT NULL DEFAULT '',
            created_at INTEGER NOT NULL,
            category TEXT NOT NULL,
            action TEXT NOT NULL,
            target_type TEXT NOT NULL,
            target_id TEXT,
            target_title TEXT,
            status TEXT NOT NULL,
            summary TEXT NOT NULL,
            error TEXT,
            related_json TEXT NOT NULL DEFAULT '{}'
        )",
        [],
    )
    .map_err(sqlite_io)?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS operation_logs_created_idx
            ON operation_logs (created_at DESC, id DESC)",
        [],
    )
    .map_err(sqlite_io)?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS operation_logs_category_status_idx
            ON operation_logs (category, status, created_at DESC)",
        [],
    )
    .map_err(sqlite_io)?;
    ensure_schema_version(conn)?;
    Ok(())
}

fn ensure_column(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> std::io::Result<()> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(sqlite_io)?;
    let mut rows = stmt.query([]).map_err(sqlite_io)?;
    while let Some(row) = rows.next().map_err(sqlite_io)? {
        let name: String = row.get(1).map_err(sqlite_io)?;
        if name == column {
            return Ok(());
        }
    }
    conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
        [],
    )
    .map_err(sqlite_io)?;
    Ok(())
}

fn ensure_schema_version(conn: &Connection) -> std::io::Result<()> {
    let current = read_schema_version(conn)?;
    if current > DB_SCHEMA_VERSION {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("订阅 SQLite schema 版本过新: {current} > {DB_SCHEMA_VERSION}"),
        ));
    }
    if current == DB_SCHEMA_VERSION {
        return Ok(());
    }
    conn.execute(
        "DELETE FROM subscription_schema_meta WHERE key = 'schema_version'",
        [],
    )
    .map_err(sqlite_io)?;
    conn.execute(
        "INSERT INTO subscription_schema_meta (key, value) VALUES ('schema_version', ?1)",
        params![DB_SCHEMA_VERSION],
    )
    .map_err(sqlite_io)?;
    Ok(())
}

fn read_schema_version(conn: &Connection) -> std::io::Result<i64> {
    conn.query_row(
        "SELECT value FROM subscription_schema_meta WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    )
    .optional()
    .map_err(sqlite_io)
    .map(|value| value.unwrap_or_default())
}

fn load_state_from_db(
    conn: &Connection,
    account_key: &str,
) -> std::io::Result<Option<WantedSubscriptionState>> {
    let Some((version, bootstrap_completed, created_at, updated_at, last_poll_at)) = conn
        .query_row(
            "SELECT version, bootstrap_completed, created_at, updated_at, last_poll_at
                FROM subscription_meta WHERE account_key = ?1",
            params![account_key],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)? != 0,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(sqlite_io)?
    else {
        return Ok(None);
    };

    if let Some(blob_state) = load_state_blob_from_db(conn, account_key)? {
        return Ok(Some(blob_state));
    }

    let mut state = WantedSubscriptionState {
        version: version as u32,
        account_key: account_key.to_string(),
        bootstrap_completed,
        created_at: i64_to_u64(created_at),
        updated_at: i64_to_u64(updated_at),
        last_poll_at: last_poll_at.map(i64_to_u64),
        records: BTreeMap::new(),
    };

    let mut stmt = conn
        .prepare(
            "SELECT subject_id, status, title, category_text, updated_at, record_json FROM wanted_subscription_records
                WHERE account_key = ?1 ORDER BY subject_id",
        )
        .map_err(sqlite_io)?;
    let mut rows = stmt.query(params![account_key]).map_err(sqlite_io)?;
    while let Some(row) = rows.next().map_err(sqlite_io)? {
        let mut record = parse_record_row(row)?;
        repair_record_defaults(&mut record, state.created_at, state.updated_at, 0);
        state.records.insert(record.subject_id.clone(), record);
    }

    Ok(Some(state))
}

fn load_state_blob_from_db(
    conn: &Connection,
    account_key: &str,
) -> std::io::Result<Option<WantedSubscriptionState>> {
    let raw = conn
        .query_row(
            "SELECT state_json FROM subscription_state_blobs WHERE account_key = ?1",
            params![account_key],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(sqlite_io)?;
    raw.map(|raw| {
        serde_json::from_str(&raw).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("解析订阅 SQLite 状态快照失败: {e}"),
            )
        })
    })
    .transpose()
}

fn parse_record_row(row: &Row<'_>) -> std::io::Result<WantedSubscriptionRecord> {
    let subject_id = row.get::<_, String>(0).map_err(sqlite_io)?;
    let status = row.get::<_, String>(1).map_err(sqlite_io)?;
    let title = row.get::<_, String>(2).map_err(sqlite_io)?;
    let category_text = row.get::<_, Option<String>>(3).map_err(sqlite_io)?;
    let updated_at = row
        .get::<_, Option<i64>>(4)
        .map_err(sqlite_io)?
        .map(i64_to_u64)
        .unwrap_or_default();
    let raw = row.get::<_, String>(5).map_err(sqlite_io)?;
    let mut record: WantedSubscriptionRecord =
        serde_json::from_str(&raw).unwrap_or_else(|_| WantedSubscriptionRecord {
            subject_id: subject_id.clone(),
            title: title.clone(),
            release_year: None,
            poster_url: String::new(),
            cover_url: String::new(),
            original_title: None,
            aka: Vec::new(),
            languages: Vec::new(),
            countries: Vec::new(),
            genres: Vec::new(),
            directors: Vec::new(),
            actors: Vec::new(),
            date_published: None,
            duration: None,
            summary: None,
            rating_value: None,
            rating_count: None,
            category_text: category_text.clone(),
            tags: Vec::new(),
            douban_date: None,
            douban_sort_time: None,
            douban_return_order: None,
            status: status_from_label(&status),
            lifecycle_state: SubscriptionLifecycleState::Queued,
            execution_state: SubscriptionExecutionState::Idle,
            attention_tags: Vec::new(),
            failure: None,
            next_attempt_at: Some(updated_at),
            force_eligible_once: false,
            media_kind: SubscriptionMediaKind::from_tags(&[]),
            tv: None,
            retry_count: 0,
            max_retries: 0,
            last_error: Some("原订阅记录 JSON 损坏，已按索引字段降级恢复".to_string()),
            skip_reason: None,
            processing_stage: None,
            stage_message: None,
            stage_updated_at: None,
            next_action: None,
            candidate_matches: Vec::new(),
            last_push: None,
            last_completion: None,
            created_at: updated_at,
            updated_at,
            first_seen_at: updated_at,
            last_seen_at: updated_at,
        });
    if record.subject_id.trim().is_empty() {
        record.subject_id = subject_id;
    }
    if record.title.trim().is_empty() {
        record.title = title;
    }
    if record.category_text.is_none() {
        record.category_text = category_text;
    }
    if matches!(record.status, WantedSubscriptionStatus::Unprocessed) && status != "unprocessed" {
        record.status = status_from_label(&status);
    }
    if record.updated_at == 0 {
        record.updated_at = updated_at;
    }
    repair_record_defaults(&mut record, updated_at, updated_at, updated_at);
    Ok(record)
}

fn save_state_to_db(
    conn: &mut Connection,
    account_key: &str,
    state: &WantedSubscriptionState,
) -> std::io::Result<()> {
    let tx = conn.transaction().map_err(sqlite_io)?;
    tx.execute(
        "DELETE FROM subscription_meta WHERE account_key = ?1",
        params![account_key],
    )
    .map_err(sqlite_io)?;
    tx.execute(
        "INSERT INTO subscription_meta
            (account_key, version, bootstrap_completed, created_at, updated_at, last_poll_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            account_key,
            state.version as i64,
            i64::from(state.bootstrap_completed),
            u64_to_i64(state.created_at),
            u64_to_i64(state.updated_at),
            state.last_poll_at.map(u64_to_i64),
        ],
    )
    .map_err(sqlite_io)?;

    let state_json = serde_json::to_string(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    tx.execute(
        "DELETE FROM subscription_state_blobs WHERE account_key = ?1",
        params![account_key],
    )
    .map_err(sqlite_io)?;
    tx.execute(
        "INSERT INTO subscription_state_blobs
            (account_key, state_json, updated_at)
            VALUES (?1, ?2, ?3)",
        params![account_key, state_json, u64_to_i64(state.updated_at)],
    )
    .map_err(sqlite_io)?;

    let state_subjects = state.records.keys().cloned().collect::<Vec<_>>();
    if state_subjects.is_empty() {
        tx.execute(
            "DELETE FROM wanted_subscription_records WHERE account_key = ?1",
            params![account_key],
        )
        .map_err(sqlite_io)?;
    } else {
        let placeholders = std::iter::repeat("?")
            .take(state_subjects.len())
            .collect::<Vec<_>>()
            .join(",");
        let mut params = vec![account_key.to_string()];
        params.extend(state_subjects);
        tx.execute(
            &format!(
                "DELETE FROM wanted_subscription_records
                    WHERE account_key = ? AND subject_id NOT IN ({placeholders})"
            ),
            params_from_iter(params.iter()),
        )
        .map_err(sqlite_io)?;
    }
    for record in state.records.values() {
        let record_json = serde_json::to_string(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let indexes = record_index_values(record);
        tx.execute(
            "INSERT INTO wanted_subscription_records
                (account_key, subject_id, status, title, category_text, updated_at, record_json,
                 lifecycle_state, execution_state, attention_tags_json, media_kind, next_attempt_at,
                 search_next_attempt_at, progress_next_attempt_at, link_next_attempt_at, retry_blocked_count)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                ON CONFLICT(account_key, subject_id) DO UPDATE SET
                    status = excluded.status,
                    title = excluded.title,
                    category_text = excluded.category_text,
                    updated_at = excluded.updated_at,
                    record_json = excluded.record_json,
                    lifecycle_state = excluded.lifecycle_state,
                    execution_state = excluded.execution_state,
                    attention_tags_json = excluded.attention_tags_json,
                    media_kind = excluded.media_kind,
                    next_attempt_at = excluded.next_attempt_at,
                    search_next_attempt_at = excluded.search_next_attempt_at,
                    progress_next_attempt_at = excluded.progress_next_attempt_at,
                    link_next_attempt_at = excluded.link_next_attempt_at,
                    retry_blocked_count = excluded.retry_blocked_count",
            params![
                account_key,
                record.subject_id,
                status_label(record.status),
                record.title,
                record.category_text,
                u64_to_i64(record.updated_at),
                record_json,
                indexes.lifecycle_state,
                indexes.execution_state,
                indexes.attention_tags_json,
                indexes.media_kind,
                indexes.next_attempt_at.map(u64_to_i64),
                indexes.search_next_attempt_at.map(u64_to_i64),
                indexes.progress_next_attempt_at.map(u64_to_i64),
                indexes.link_next_attempt_at.map(u64_to_i64),
                indexes.retry_blocked_count,
            ],
        )
        .map_err(sqlite_io)?;
    }
    tx.commit().map_err(sqlite_io)?;
    Ok(())
}

struct RecordIndexValues {
    lifecycle_state: String,
    execution_state: String,
    attention_tags_json: String,
    media_kind: String,
    next_attempt_at: Option<u64>,
    search_next_attempt_at: Option<u64>,
    progress_next_attempt_at: Option<u64>,
    link_next_attempt_at: Option<u64>,
    retry_blocked_count: i64,
}

fn record_index_values(record: &WantedSubscriptionRecord) -> RecordIndexValues {
    RecordIndexValues {
        lifecycle_state: record.lifecycle_state.as_str().to_string(),
        execution_state: execution_state_label(record.execution_state).to_string(),
        attention_tags_json: serde_json::to_string(&record.attention_tags)
            .unwrap_or_else(|_| "[]".to_string()),
        media_kind: record.media_kind.as_str().to_string(),
        next_attempt_at: record.next_attempt_at,
        search_next_attempt_at: record
            .tv
            .as_ref()
            .and_then(|tv| tv.lanes.search.next_attempt_at),
        progress_next_attempt_at: record
            .tv
            .as_ref()
            .and_then(|tv| tv.lanes.progress.next_attempt_at),
        link_next_attempt_at: record
            .tv
            .as_ref()
            .and_then(|tv| tv.lanes.link.next_attempt_at),
        retry_blocked_count: retry_blocked_count(record),
    }
}

fn execution_state_label(state: SubscriptionExecutionState) -> &'static str {
    match state {
        SubscriptionExecutionState::Idle => "idle",
        SubscriptionExecutionState::Running => "running",
    }
}

fn retry_blocked_count(record: &WantedSubscriptionRecord) -> i64 {
    let mut count = i64::from(
        record
            .failure
            .as_ref()
            .is_some_and(|failure| failure.retry_blocked),
    );
    if let Some(tv) = record.tv.as_ref() {
        count += i64::from(
            tv.lanes
                .search
                .failure
                .as_ref()
                .is_some_and(|failure| failure.retry_blocked),
        );
        count += i64::from(
            tv.lanes
                .progress
                .failure
                .as_ref()
                .is_some_and(|failure| failure.retry_blocked),
        );
        count += i64::from(
            tv.lanes
                .link
                .failure
                .as_ref()
                .is_some_and(|failure| failure.retry_blocked),
        );
        count += tv
            .episodes
            .iter()
            .filter(|ep| {
                ep.failure
                    .as_ref()
                    .is_some_and(|failure| failure.retry_blocked)
            })
            .count() as i64;
        count += tv
            .download_tasks
            .iter()
            .filter(|task| {
                task.failure
                    .as_ref()
                    .is_some_and(|failure| failure.retry_blocked)
            })
            .count() as i64;
    }
    count
}

fn append_operation_log_to_db(
    conn: &Connection,
    entry: NewOperationLogEntry,
) -> std::io::Result<OperationLogEntry> {
    let related_json = serde_json::to_string(&entry.related).unwrap_or_else(|_| "{}".to_string());
    conn.execute(
        "INSERT INTO operation_logs
            (account_key, created_at, category, action, target_type, target_id, target_title, status, summary, error, related_json)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            entry.account_key,
            u64_to_i64(entry.created_at),
            entry.category,
            entry.action,
            entry.target_type,
            entry.target_id,
            entry.target_title,
            entry.status,
            entry.summary,
            entry.error,
            related_json,
        ],
    )
    .map_err(sqlite_io)?;
    load_operation_log_by_id(conn, conn.last_insert_rowid())
}

fn query_operation_logs_from_db(
    conn: &Connection,
    query: OperationLogQuery,
) -> std::io::Result<OperationLogPage> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(30).clamp(1, 100);
    let offset = u64::from(page.saturating_sub(1)) * u64::from(page_size);
    let mut filters = String::new();
    let mut values = Vec::<String>::new();

    if let Some(category) = query.category.map(|value| value.trim().to_string()) {
        if !category.is_empty() && category != "all" {
            filters.push_str(" AND category = ?");
            values.push(category);
        }
    }
    if let Some(status) = query.status.map(|value| value.trim().to_string()) {
        if !status.is_empty() && status != "all" {
            filters.push_str(" AND status = ?");
            values.push(status);
        }
    }
    if let Some(q) = query.q.map(|value| value.trim().to_string()) {
        if !q.is_empty() {
            let pattern = format!("%{q}%");
            filters.push_str(
                " AND (
                    summary LIKE ?
                    OR action LIKE ?
                    OR target_id LIKE ?
                    OR target_title LIKE ?
                    OR error LIKE ?
                )",
            );
            values.extend([
                pattern.clone(),
                pattern.clone(),
                pattern.clone(),
                pattern.clone(),
                pattern,
            ]);
        }
    }

    let count_sql = format!("SELECT COUNT(*) FROM operation_logs WHERE 1=1{filters}");
    let total = conn
        .query_row(&count_sql, params_from_iter(values.iter()), |row| {
            row.get::<_, i64>(0)
        })
        .map_err(sqlite_io)
        .map(i64_to_u64)?;

    let list_sql = format!(
        "SELECT id, account_key, created_at, category, action, target_type, target_id, target_title, status, summary, error, related_json
            FROM operation_logs
            WHERE 1=1{filters}
            ORDER BY created_at DESC, id DESC
            LIMIT {page_size} OFFSET {offset}"
    );
    let mut stmt = conn.prepare(&list_sql).map_err(sqlite_io)?;
    let rows = stmt
        .query_map(params_from_iter(values.iter()), parse_operation_log_row)
        .map_err(sqlite_io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sqlite_io)?;
    let shown = offset.saturating_add(rows.len() as u64);

    Ok(OperationLogPage {
        items: rows,
        page,
        page_size,
        total,
        has_more: shown < total,
    })
}

fn load_operation_log_by_id(conn: &Connection, id: i64) -> std::io::Result<OperationLogEntry> {
    conn.query_row(
        "SELECT id, account_key, created_at, category, action, target_type, target_id, target_title, status, summary, error, related_json
            FROM operation_logs WHERE id = ?1",
        params![id],
        parse_operation_log_row,
    )
    .map_err(sqlite_io)
}

fn parse_operation_log_row(row: &Row<'_>) -> rusqlite::Result<OperationLogEntry> {
    let related_raw = row.get::<_, String>(11)?;
    let related = serde_json::from_str(&related_raw).unwrap_or_else(|_| json!({}));
    Ok(OperationLogEntry {
        id: i64_to_u64(row.get::<_, i64>(0)?),
        account_key: row.get(1)?,
        created_at: i64_to_u64(row.get::<_, i64>(2)?),
        category: row.get(3)?,
        action: row.get(4)?,
        target_type: row.get(5)?,
        target_id: row.get(6)?,
        target_title: row.get(7)?,
        status: row.get(8)?,
        summary: row.get(9)?,
        error: row.get(10)?,
        related,
    })
}

fn status_label(status: WantedSubscriptionStatus) -> &'static str {
    match status {
        WantedSubscriptionStatus::Unprocessed => "unprocessed",
        WantedSubscriptionStatus::Matching => "matching",
        WantedSubscriptionStatus::Processing => "processing",
        WantedSubscriptionStatus::Pushed => "pushed",
        WantedSubscriptionStatus::Downloading => "downloading",
        WantedSubscriptionStatus::Completed => "completed",
        WantedSubscriptionStatus::Linked => "linked",
        WantedSubscriptionStatus::Failed => "failed",
        WantedSubscriptionStatus::Skipped => "skipped",
    }
}

fn status_from_label(raw: &str) -> WantedSubscriptionStatus {
    match raw.trim().to_ascii_lowercase().as_str() {
        "matching" => WantedSubscriptionStatus::Matching,
        "processing" => WantedSubscriptionStatus::Processing,
        "pushed" => WantedSubscriptionStatus::Pushed,
        "downloading" => WantedSubscriptionStatus::Downloading,
        "completed" => WantedSubscriptionStatus::Completed,
        "linked" => WantedSubscriptionStatus::Linked,
        "failed" => WantedSubscriptionStatus::Failed,
        "skipped" => WantedSubscriptionStatus::Skipped,
        _ => WantedSubscriptionStatus::Unprocessed,
    }
}

fn u64_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn sqlite_io(error: rusqlite::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, error.to_string())
}

fn is_sqlite_recoverable_corruption(error: &std::io::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("malformed")
        || message.contains("not a database")
        || message.contains("invalid rootpage")
        || message.contains("database disk image is malformed")
}

fn backup_corrupt_db(path: &PathBuf) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("wanted.sqlite");
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let backup = path.with_file_name(format!("{file_name}.corrupt.{stamp}"));
    std::fs::rename(path, backup)
}

fn repair_state_defaults(state: &mut WantedSubscriptionState, account_key: &str, now: u64) {
    if state.version == 0 {
        state.version = STATE_VERSION;
    }
    if state.account_key.trim().is_empty() {
        state.account_key = account_key.to_string();
    }
    if state.created_at == 0 {
        state.created_at = now;
    }
    if state.updated_at == 0 {
        state.updated_at = now;
    }
    let state_created_at = state.created_at;
    let state_updated_at = state.updated_at;
    for record in state.records.values_mut() {
        repair_record_defaults(record, state_created_at, state_updated_at, now);
    }
}

fn repair_record_defaults(
    record: &mut WantedSubscriptionRecord,
    state_created_at: u64,
    state_updated_at: u64,
    now: u64,
) {
    if record.created_at == 0 {
        record.created_at = state_created_at.max(now);
    }
    if record.updated_at == 0 {
        record.updated_at = state_updated_at.max(record.created_at);
    }
    if record.first_seen_at == 0 {
        record.first_seen_at = record.created_at;
    }
    if record.last_seen_at == 0 {
        record.last_seen_at = record.updated_at;
    }
    if record.lifecycle_state == SubscriptionLifecycleState::Queued
        && (record.status != WantedSubscriptionStatus::Unprocessed
            || record
                .processing_stage
                .as_deref()
                .is_some_and(|stage| stage != "queued")
            || record.last_push.is_some()
            || record.last_completion.is_some()
            || !record.candidate_matches.is_empty())
    {
        migrate_legacy_status_fields(record, now);
    }
    normalize_existing_stage(record, now);
}

fn normalize_existing_stage(record: &mut WantedSubscriptionRecord, now: u64) {
    if matches!(record.status, WantedSubscriptionStatus::Skipped) {
        let stage = record.processing_stage.as_deref().unwrap_or_default();
        let message = record.stage_message.as_deref().unwrap_or_default().trim();
        let skip_reason = record.skip_reason.as_deref().unwrap_or_default().trim();
        if stage != "skipped"
            || message.is_empty()
            || (!skip_reason.is_empty() && message == skip_reason)
        {
            let stage_time = record
                .stage_updated_at
                .unwrap_or_else(|| record.updated_at.max(record.created_at).max(now));
            apply_status_stage(record, stage_time);
        }
        return;
    }

    if record.processing_stage.is_none() && record.stage_message.is_none() {
        hydrate_stage_from_record(record);
    }
}

pub fn migrate_legacy_status_fields(record: &mut WantedSubscriptionRecord, now: u64) {
    let (state, mut tags) = infer_lifecycle_from_legacy(record);
    record.lifecycle_state = state;
    record.execution_state = SubscriptionExecutionState::Idle;
    record.next_attempt_at.get_or_insert(now);
    if record.failure.is_none() {
        record.failure = failure_from_legacy_error(record, now);
    }
    preserve_retry_blocked_attention_tag(record, &mut tags);
    merge_attention_tags(record, tags);
    record.status = derive_legacy_status(record);
}

fn preserve_retry_blocked_attention_tag(
    record: &WantedSubscriptionRecord,
    tags: &mut Vec<SubscriptionAttentionTag>,
) {
    if record
        .failure
        .as_ref()
        .is_some_and(|failure| failure.retry_blocked)
        && !tags.contains(&SubscriptionAttentionTag::RetryBlocked)
    {
        tags.push(SubscriptionAttentionTag::RetryBlocked);
    }
}

fn infer_lifecycle_from_legacy(
    record: &WantedSubscriptionRecord,
) -> (SubscriptionLifecycleState, Vec<SubscriptionAttentionTag>) {
    if let Some(completion) = record.last_completion.as_ref() {
        match completion.status.as_str() {
            "completed" | "linked" => return (SubscriptionLifecycleState::Completed, vec![]),
            "failed" => {
                return (
                    SubscriptionLifecycleState::Linking,
                    vec![SubscriptionAttentionTag::Failed],
                )
            }
            "pending" | "dry_run" | "planned" => {
                return (SubscriptionLifecycleState::Downloading, vec![])
            }
            _ => {}
        }
    }

    if let Some(push) = record.last_push.as_ref() {
        match push.status.as_str() {
            "linked" | "completed" => {
                let complete = record
                    .last_completion
                    .as_ref()
                    .is_some_and(|completion| completion.status == "completed");
                return (
                    if complete {
                        SubscriptionLifecycleState::Completed
                    } else {
                        SubscriptionLifecycleState::Linking
                    },
                    vec![],
                );
            }
            "downloaded" => return (SubscriptionLifecycleState::Linking, vec![]),
            "downloading" | "pushed" => return (SubscriptionLifecycleState::Downloading, vec![]),
            "failed" => {
                let waiting = legacy_text_is_waiting_release(
                    record.processing_stage.as_deref(),
                    push.error.as_deref(),
                );
                return (
                    SubscriptionLifecycleState::Searching,
                    vec![if waiting {
                        SubscriptionAttentionTag::WaitingRelease
                    } else {
                        SubscriptionAttentionTag::Failed
                    }],
                );
            }
            _ => {}
        }
    }

    if let Some(stage) = record.processing_stage.as_deref() {
        match stage {
            "link_failed" | "download_complete" | "link_planned" => {
                return (SubscriptionLifecycleState::Linking, vec![])
            }
            "downloading" | "pushed" => return (SubscriptionLifecycleState::Downloading, vec![]),
            "no_candidates" | "no_match" => {
                return (
                    SubscriptionLifecycleState::Searching,
                    vec![SubscriptionAttentionTag::WaitingRelease],
                )
            }
            "searching" | "matched" | "pushing" => {
                return (SubscriptionLifecycleState::Searching, vec![])
            }
            "push_failed" => {
                return (
                    SubscriptionLifecycleState::Searching,
                    vec![SubscriptionAttentionTag::Failed],
                )
            }
            _ => {}
        }
    }

    if !record.candidate_matches.is_empty() {
        return (SubscriptionLifecycleState::Searching, vec![]);
    }

    match record.status {
        WantedSubscriptionStatus::Unprocessed => (SubscriptionLifecycleState::Queued, vec![]),
        WantedSubscriptionStatus::Matching | WantedSubscriptionStatus::Processing => {
            (SubscriptionLifecycleState::Searching, vec![])
        }
        WantedSubscriptionStatus::Pushed | WantedSubscriptionStatus::Downloading => {
            (SubscriptionLifecycleState::Downloading, vec![])
        }
        WantedSubscriptionStatus::Completed => (SubscriptionLifecycleState::Linking, vec![]),
        WantedSubscriptionStatus::Linked => (SubscriptionLifecycleState::Completed, vec![]),
        WantedSubscriptionStatus::Skipped => (
            SubscriptionLifecycleState::Queued,
            vec![SubscriptionAttentionTag::Skipped],
        ),
        WantedSubscriptionStatus::Failed => (
            SubscriptionLifecycleState::Queued,
            vec![
                SubscriptionAttentionTag::Failed,
                SubscriptionAttentionTag::NeedsReconciliation,
            ],
        ),
    }
}

fn legacy_text_is_waiting_release(stage: Option<&str>, error: Option<&str>) -> bool {
    matches!(stage, Some("no_candidates" | "no_match"))
        || error
            .unwrap_or_default()
            .to_ascii_lowercase()
            .contains("no candidate")
        || error.unwrap_or_default().contains("没有匹配")
        || error.unwrap_or_default().contains("无候选")
}

fn merge_attention_tags(
    record: &mut WantedSubscriptionRecord,
    tags: Vec<SubscriptionAttentionTag>,
) {
    record.attention_tags.retain(|tag| {
        !matches!(
            tag,
            SubscriptionAttentionTag::Failed
                | SubscriptionAttentionTag::WaitingRelease
                | SubscriptionAttentionTag::RetryBlocked
                | SubscriptionAttentionTag::NeedsReconciliation
        )
    });
    for tag in tags {
        if !record.attention_tags.contains(&tag) {
            record.attention_tags.push(tag);
        }
    }
    record.attention_tags.sort();
}

fn failure_from_legacy_error(
    record: &WantedSubscriptionRecord,
    now: u64,
) -> Option<SubscriptionFailure> {
    let message = record
        .last_error
        .as_deref()
        .or_else(|| {
            record
                .last_push
                .as_ref()
                .and_then(|push| push.error.as_deref())
        })
        .or_else(|| {
            record
                .last_completion
                .as_ref()
                .and_then(|completion| completion.error.as_deref())
        })?;
    Some(SubscriptionFailure {
        scope: FailureScope::Parent,
        owner_id: record.subject_id.clone(),
        operation: record
            .processing_stage
            .clone()
            .unwrap_or_else(|| "legacy".to_string()),
        error_type: "legacy".to_string(),
        message: message.to_string(),
        retry_count: record.retry_count,
        max_retries: record.max_retries,
        failed_at: record.stage_updated_at.unwrap_or(now),
        next_retry_at: record.next_attempt_at,
        retry_blocked: record.retry_count >= record.max_retries && record.max_retries > 0,
    })
}

pub fn derive_legacy_status(record: &WantedSubscriptionRecord) -> WantedSubscriptionStatus {
    if record
        .attention_tags
        .contains(&SubscriptionAttentionTag::Skipped)
    {
        return WantedSubscriptionStatus::Skipped;
    }
    if record
        .attention_tags
        .contains(&SubscriptionAttentionTag::Failed)
        || record
            .attention_tags
            .contains(&SubscriptionAttentionTag::RetryBlocked)
    {
        return WantedSubscriptionStatus::Failed;
    }
    match record.lifecycle_state {
        SubscriptionLifecycleState::Queued => WantedSubscriptionStatus::Unprocessed,
        SubscriptionLifecycleState::Meta | SubscriptionLifecycleState::Searching => {
            WantedSubscriptionStatus::Matching
        }
        SubscriptionLifecycleState::Downloading => WantedSubscriptionStatus::Downloading,
        SubscriptionLifecycleState::Linking => WantedSubscriptionStatus::Completed,
        SubscriptionLifecycleState::Completed => WantedSubscriptionStatus::Linked,
    }
}

#[allow(dead_code)]
pub fn initialize_tv_targets(
    record: &mut WantedSubscriptionRecord,
    target: TvTargetRange,
    now: u64,
) -> Result<(), String> {
    if target.start_episode == 0
        || target.end_episode < target.start_episode
        || target.episode_total < target.end_episode
    {
        return Err("invalid TV target episode range".to_string());
    }

    let episodes = (target.start_episode..=target.end_episode)
        .map(|episode| TvEpisodeRecord {
            season_number: target.season_number,
            episode_number: episode,
            label: format!("E{episode:02}"),
            target_state: EpisodeTargetState::Target,
            coverage_state: EpisodeCoverageState::Uncovered,
            assignment_state: EpisodeAssignmentState::None,
            selected_task_id: None,
            download_state: "idle".to_string(),
            link_state: "idle".to_string(),
            retry_count: 0,
            max_retries: record.max_retries,
            failure: None,
            updated_at: now,
        })
        .collect::<Vec<_>>();

    record.media_kind = SubscriptionMediaKind::Tv;
    record.tv = Some(TvSubscriptionState {
        schema_version: 1,
        metadata_ready: true,
        episode_records_initialized: true,
        target_episode_set_known: true,
        season_number: target.season_number,
        episode_total: target.episode_total,
        target_start_episode: target.start_episode,
        target_end_episode: target.end_episode,
        search_cursor_episode: Some(target.start_episode),
        lanes: TvLaneSet {
            search: OperationLaneState {
                next_attempt_at: Some(now),
                max_retries: record.max_retries,
                ..OperationLaneState::default()
            },
            progress: OperationLaneState {
                next_attempt_at: Some(now),
                max_retries: record.max_retries,
                ..OperationLaneState::default()
            },
            link: OperationLaneState {
                next_attempt_at: Some(now),
                max_retries: record.max_retries,
                ..OperationLaneState::default()
            },
        },
        episodes,
        download_tasks: Vec::new(),
    });
    Ok(())
}

#[allow(dead_code)]
pub fn bind_task_to_episodes(
    tv: &mut TvSubscriptionState,
    task_id: &str,
    coverage: CoverageRange,
    trust: CoverageTrust,
    now: u64,
) -> Result<(), String> {
    if task_id.trim().is_empty() {
        return Err("task id is required".to_string());
    }
    if coverage.start_episode == 0 || coverage.end_episode < coverage.start_episode {
        return Err("invalid coverage range".to_string());
    }

    for ep in tv.episodes.iter_mut().filter(|ep| {
        ep.season_number == coverage.season_number && coverage.contains_episode(ep.episode_number)
    }) {
        if matches!(
            ep.assignment_state,
            EpisodeAssignmentState::Active
                | EpisodeAssignmentState::Blocked
                | EpisodeAssignmentState::Completed
                | EpisodeAssignmentState::Skipped
        ) {
            continue;
        }
        if matches!(
            ep.target_state,
            EpisodeTargetState::Skipped | EpisodeTargetState::Completed
        ) {
            continue;
        }
        ep.selected_task_id = Some(task_id.to_string());
        ep.assignment_state = EpisodeAssignmentState::Active;
        ep.coverage_state = match trust {
            CoverageTrust::Tentative => EpisodeCoverageState::TentativeCovered,
            CoverageTrust::Verified => EpisodeCoverageState::VerifiedCovered,
        };
        ep.download_state = "pushed".to_string();
        ep.updated_at = now;
    }
    recalculate_search_cursor(tv);
    Ok(())
}

#[allow(dead_code)]
pub fn apply_verified_coverage(
    tv: &mut TvSubscriptionState,
    task_id: &str,
    coverage: CoverageRange,
    now: u64,
) -> Result<(), String> {
    let mut found = false;
    if let Some(task) = tv
        .download_tasks
        .iter_mut()
        .find(|task| task.task_id == task_id)
    {
        task.verified_coverage = vec![coverage];
        task.checked_at = Some(now);
        found = true;
    }

    let mut first_released = None;
    for ep in tv
        .episodes
        .iter_mut()
        .filter(|ep| ep.selected_task_id.as_deref() == Some(task_id))
    {
        found = true;
        if coverage.season_number == ep.season_number
            && coverage.contains_episode(ep.episode_number)
        {
            ep.coverage_state = EpisodeCoverageState::VerifiedCovered;
            ep.updated_at = now;
        } else if !matches!(
            ep.assignment_state,
            EpisodeAssignmentState::Completed | EpisodeAssignmentState::Skipped
        ) {
            ep.assignment_state = EpisodeAssignmentState::Released;
            ep.coverage_state = EpisodeCoverageState::Uncovered;
            ep.selected_task_id = None;
            ep.download_state = "idle".to_string();
            ep.updated_at = now;
            first_released = Some(
                first_released
                    .map(|current: u32| current.min(ep.episode_number))
                    .unwrap_or(ep.episode_number),
            );
        }
    }

    if !found {
        return Err(format!("task {task_id} not found"));
    }
    if let Some(episode) = first_released {
        tv.search_cursor_episode = Some(episode);
    } else {
        recalculate_search_cursor(tv);
    }
    Ok(())
}

#[allow(dead_code)]
pub fn recalculate_search_cursor(tv: &mut TvSubscriptionState) {
    tv.search_cursor_episode = tv
        .episodes
        .iter()
        .filter(|ep| ep.target_state == EpisodeTargetState::Target)
        .filter(|ep| {
            !matches!(
                ep.assignment_state,
                EpisodeAssignmentState::Active
                    | EpisodeAssignmentState::Blocked
                    | EpisodeAssignmentState::Completed
                    | EpisodeAssignmentState::Skipped
            )
        })
        .map(|ep| ep.episode_number)
        .min()
        .or_else(|| Some(tv.episode_total.saturating_add(1)));
}

#[allow(dead_code)]
pub fn block_episode_assignment(
    tv: &mut TvSubscriptionState,
    season: u32,
    episode: u32,
    failure: SubscriptionFailure,
) -> Result<(), String> {
    let ep = tv_episode_mut(tv, season, episode)?;
    ep.assignment_state = EpisodeAssignmentState::Blocked;
    ep.failure = Some(failure);
    ep.updated_at = ep.updated_at.max(1);
    recalculate_search_cursor(tv);
    Ok(())
}

#[allow(dead_code)]
pub fn skip_episode_range(
    tv: &mut TvSubscriptionState,
    season: u32,
    start: u32,
    end: u32,
    reason: &str,
    now: u64,
) -> Result<(), String> {
    if start == 0 || end < start {
        return Err("invalid episode range".to_string());
    }
    let mut changed = false;
    for ep in tv.episodes.iter_mut().filter(|ep| {
        ep.season_number == season && start <= ep.episode_number && ep.episode_number <= end
    }) {
        changed = true;
        ep.target_state = EpisodeTargetState::Skipped;
        ep.assignment_state = EpisodeAssignmentState::Skipped;
        ep.coverage_state = EpisodeCoverageState::Uncovered;
        ep.selected_task_id = None;
        ep.failure = None;
        ep.link_state = reason.trim().to_string();
        ep.updated_at = now;
    }
    if !changed {
        return Err("episode range not found".to_string());
    }
    recalculate_search_cursor(tv);
    Ok(())
}

#[allow(dead_code)]
pub fn unskip_episode_range(
    tv: &mut TvSubscriptionState,
    season: u32,
    start: u32,
    end: u32,
    now: u64,
) -> Result<(), String> {
    if start == 0 || end < start {
        return Err("invalid episode range".to_string());
    }
    let mut changed = false;
    for ep in tv.episodes.iter_mut().filter(|ep| {
        ep.season_number == season && start <= ep.episode_number && ep.episode_number <= end
    }) {
        changed = true;
        if ep.assignment_state == EpisodeAssignmentState::Skipped {
            ep.target_state = EpisodeTargetState::Target;
            ep.assignment_state = EpisodeAssignmentState::Released;
            ep.link_state = "idle".to_string();
            ep.updated_at = now;
        }
    }
    if !changed {
        return Err("episode range not found".to_string());
    }
    recalculate_search_cursor(tv);
    Ok(())
}

#[allow(dead_code)]
fn tv_episode_mut(
    tv: &mut TvSubscriptionState,
    season: u32,
    episode: u32,
) -> Result<&mut TvEpisodeRecord, String> {
    tv.episodes
        .iter_mut()
        .find(|ep| ep.season_number == season && ep.episode_number == episode)
        .ok_or_else(|| format!("episode S{season:02}E{episode:02} not found"))
}

#[allow(dead_code)]
pub fn derive_tv_parent_lifecycle(record: &mut WantedSubscriptionRecord) {
    let Some(tv) = record.tv.as_ref() else {
        return;
    };
    if !(tv.metadata_ready && tv.episode_records_initialized && tv.target_episode_set_known) {
        record.lifecycle_state = SubscriptionLifecycleState::Meta;
        record.status = derive_legacy_status(record);
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

#[allow(dead_code)]
pub fn tv_is_complete(tv: &TvSubscriptionState) -> bool {
    let cursor_done = tv.search_cursor_episode.unwrap_or(1) > tv.episode_total
        || tv.episodes.iter().all(|ep| {
            matches!(
                ep.assignment_state,
                EpisodeAssignmentState::Completed | EpisodeAssignmentState::Skipped
            )
        });
    cursor_done
        && !tv_has_active_downloads(tv)
        && !tv_has_linkable_or_linking_work(tv)
        && tv.episodes.iter().all(|ep| {
            matches!(
                ep.assignment_state,
                EpisodeAssignmentState::Completed | EpisodeAssignmentState::Skipped
            )
        })
}

#[allow(dead_code)]
pub fn tv_has_linkable_or_linking_work(tv: &TvSubscriptionState) -> bool {
    tv.download_tasks.iter().any(|task| {
        matches!(
            task.state,
            DownloadTaskState::Downloaded | DownloadTaskState::Linking
        )
    }) || tv.episodes.iter().any(|ep| {
        !matches!(
            ep.assignment_state,
            EpisodeAssignmentState::Completed | EpisodeAssignmentState::Skipped
        ) && (ep.download_state == "downloaded" || ep.link_state == "linking")
    })
}

#[allow(dead_code)]
pub fn tv_has_active_downloads(tv: &TvSubscriptionState) -> bool {
    tv.download_tasks.iter().any(|task| {
        matches!(
            task.state,
            DownloadTaskState::Pushed | DownloadTaskState::Downloading
        )
    }) || tv.episodes.iter().any(|ep| {
        ep.assignment_state == EpisodeAssignmentState::Active
            && matches!(ep.download_state.as_str(), "pushed" | "downloading")
    })
}

#[allow(dead_code)]
pub fn tv_has_uncovered_or_waiting_cursor(tv: &TvSubscriptionState) -> bool {
    tv.search_cursor_episode
        .is_some_and(|cursor| cursor <= tv.episode_total)
        || tv.episodes.iter().any(|ep| {
            ep.target_state == EpisodeTargetState::Target
                && matches!(
                    ep.assignment_state,
                    EpisodeAssignmentState::None | EpisodeAssignmentState::Released
                )
        })
}

#[allow(dead_code)]
pub fn select_due_operation(
    record: &WantedSubscriptionRecord,
    now: u64,
) -> Option<SubscriptionDueOperation> {
    if record.lifecycle_state == SubscriptionLifecycleState::Completed {
        return None;
    }
    if record
        .attention_tags
        .contains(&SubscriptionAttentionTag::Skipped)
        && !record.force_eligible_once
        && record.tv.as_ref().is_none_or(|tv| !tv_lane_forced(tv))
    {
        return None;
    }

    match record.media_kind {
        SubscriptionMediaKind::Tv => {
            let Some(tv) = record.tv.as_ref() else {
                return record_due(record, now).then_some(SubscriptionDueOperation::TvMeta);
            };
            if !(tv.metadata_ready && tv.episode_records_initialized && tv.target_episode_set_known)
            {
                return record_due(record, now).then_some(SubscriptionDueOperation::TvMeta);
            }
            select_due_tv_lane(record, now).map(SubscriptionDueOperation::TvLane)
        }
        SubscriptionMediaKind::Movie => {
            if !record_due(record, now) {
                return None;
            }
            match record.lifecycle_state {
                SubscriptionLifecycleState::Queued | SubscriptionLifecycleState::Meta => {
                    Some(SubscriptionDueOperation::MovieMeta)
                }
                SubscriptionLifecycleState::Searching => {
                    Some(SubscriptionDueOperation::MovieSearch)
                }
                SubscriptionLifecycleState::Downloading => {
                    Some(SubscriptionDueOperation::MovieProgress)
                }
                SubscriptionLifecycleState::Linking => Some(SubscriptionDueOperation::MovieLink),
                SubscriptionLifecycleState::Completed => None,
            }
        }
    }
}

#[allow(dead_code)]
pub fn select_due_tv_lane(record: &WantedSubscriptionRecord, now: u64) -> Option<TvLaneKind> {
    let tv = record.tv.as_ref()?;
    if lane_due(&tv.lanes.link, now) {
        return Some(TvLaneKind::Link);
    }
    if lane_due(&tv.lanes.progress, now) {
        return Some(TvLaneKind::Progress);
    }
    if tv
        .search_cursor_episode
        .is_some_and(|cursor| cursor <= tv.episode_total)
        && lane_due(&tv.lanes.search, now)
    {
        return Some(TvLaneKind::Search);
    }
    None
}

#[allow(dead_code)]
pub fn apply_movie_operation_outcome(
    record: &mut WantedSubscriptionRecord,
    outcome: MovieOperationOutcome,
    cfg: &SubscriptionWatcherConfig,
    now: u64,
) {
    match outcome {
        MovieOperationOutcome::Advanced(state) => {
            record.lifecycle_state = state;
            record.next_attempt_at = Some(now);
            record.force_eligible_once = false;
        }
        MovieOperationOutcome::StillDownloading => {
            record.lifecycle_state = SubscriptionLifecycleState::Downloading;
            record.next_attempt_at = Some(now + cfg.progress_interval_secs);
        }
        MovieOperationOutcome::Waiting => {
            record.next_attempt_at = Some(now + cfg.search_interval_secs);
        }
    }
    record.execution_state = SubscriptionExecutionState::Idle;
    record.failure = None;
    record.last_error = None;
    merge_attention_tags(record, Vec::new());
    record.updated_at = now;
}

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
        next_retry_at: (!retry_blocked)
            .then_some(now + retry_interval_for_operation(operation, cfg)),
        retry_blocked,
    });
    if retry_blocked {
        merge_attention_tags(
            record,
            vec![
                SubscriptionAttentionTag::Failed,
                SubscriptionAttentionTag::RetryBlocked,
            ],
        );
        record.next_attempt_at = None;
    } else {
        merge_attention_tags(record, vec![SubscriptionAttentionTag::Failed]);
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

#[allow(dead_code)]
fn record_due(record: &WantedSubscriptionRecord, now: u64) -> bool {
    record.force_eligible_once || record.next_attempt_at.is_some_and(|due| due <= now)
}

#[allow(dead_code)]
fn lane_due(lane: &OperationLaneState, now: u64) -> bool {
    lane.force_eligible_once || lane.next_attempt_at.is_some_and(|due| due <= now)
}

#[allow(dead_code)]
fn tv_lane_forced(tv: &TvSubscriptionState) -> bool {
    tv.lanes.search.force_eligible_once
        || tv.lanes.progress.force_eligible_once
        || tv.lanes.link.force_eligible_once
}

impl WantedSubscriptionState {
    fn new(account_key: &str, now: u64) -> Self {
        Self {
            version: STATE_VERSION,
            account_key: account_key.to_string(),
            bootstrap_completed: false,
            created_at: now,
            updated_at: now,
            last_poll_at: None,
            records: BTreeMap::new(),
        }
    }
}

#[allow(dead_code)]
fn apply_wish_items_to_state(
    state: &mut WantedSubscriptionState,
    items: &[DoubanLibraryItem],
    bootstrap_existing_as_skipped: bool,
    max_retries: u32,
    state_path: String,
    now: u64,
) -> WantedPollOutcome {
    apply_wish_items_with_details_to_state(
        state,
        items,
        &BTreeMap::new(),
        bootstrap_existing_as_skipped,
        max_retries,
        state_path,
        now,
    )
}

fn apply_wish_items_with_details_to_state(
    state: &mut WantedSubscriptionState,
    items: &[DoubanLibraryItem],
    details: &BTreeMap<String, DoubanSubjectDetail>,
    bootstrap_existing_as_skipped: bool,
    max_retries: u32,
    state_path: String,
    now: u64,
) -> WantedPollOutcome {
    let bootstrap_mode = !state.bootstrap_completed;
    let mut created_unprocessed = 0usize;
    let mut created_skipped = 0usize;
    let mut updated_existing = 0usize;

    for (idx, item) in items.iter().enumerate() {
        let subject_id = item.subject_id.trim();
        if subject_id.is_empty() {
            continue;
        }
        let detail = details.get(subject_id);
        if let Some(existing) = state.records.get_mut(subject_id) {
            refresh_record_from_item_with_detail(existing, item, detail, idx, now);
            updated_existing += 1;
            continue;
        }

        let mut record = record_from_item_with_detail(item, detail, idx, max_retries, now);
        if bootstrap_mode && bootstrap_existing_as_skipped {
            record.status = WantedSubscriptionStatus::Skipped;
            record.skip_reason = Some("initial_bootstrap_existing_wish".to_string());
            apply_status_stage(&mut record, now);
            created_skipped += 1;
        } else {
            record.status = WantedSubscriptionStatus::Unprocessed;
            created_unprocessed += 1;
        }
        state.records.insert(subject_id.to_string(), record);
    }

    state.bootstrap_completed = true;
    state.last_poll_at = Some(now);
    state.updated_at = now;

    WantedPollOutcome {
        account_key: state.account_key.clone(),
        total_wish_items: items.len(),
        created_unprocessed,
        created_skipped,
        updated_existing,
        bootstrap_completed: state.bootstrap_completed,
        bootstrap_mode,
        state_path,
        polled_at: now,
    }
}

#[allow(dead_code)]
fn record_from_item(
    item: &DoubanLibraryItem,
    return_order: usize,
    max_retries: u32,
    now: u64,
) -> WantedSubscriptionRecord {
    record_from_item_with_detail(item, None, return_order, max_retries, now)
}

fn record_from_item_with_detail(
    item: &DoubanLibraryItem,
    detail: Option<&DoubanSubjectDetail>,
    return_order: usize,
    max_retries: u32,
    now: u64,
) -> WantedSubscriptionRecord {
    let tags = normalized_tags(&item.tags);
    let media_kind = SubscriptionMediaKind::from_tags(&tags);
    let douban_date = normalized_douban_date(&item.date);
    let mut record = WantedSubscriptionRecord {
        subject_id: item.subject_id.trim().to_string(),
        title: item.title.trim().to_string(),
        release_year: release_year_from_item(item),
        poster_url: item.poster_url.trim().to_string(),
        cover_url: item.cover_url.trim().to_string(),
        original_title: None,
        aka: Vec::new(),
        languages: Vec::new(),
        countries: Vec::new(),
        genres: Vec::new(),
        directors: Vec::new(),
        actors: Vec::new(),
        date_published: None,
        duration: None,
        summary: None,
        rating_value: None,
        rating_count: None,
        category_text: tags.first().cloned(),
        tags,
        douban_sort_time: douban_date.as_deref().and_then(douban_date_sort_key),
        douban_date,
        douban_return_order: Some(return_order.min(u32::MAX as usize) as u32),
        status: WantedSubscriptionStatus::Unprocessed,
        lifecycle_state: SubscriptionLifecycleState::Queued,
        execution_state: SubscriptionExecutionState::Idle,
        attention_tags: Vec::new(),
        failure: None,
        next_attempt_at: Some(now),
        force_eligible_once: false,
        media_kind,
        tv: None,
        retry_count: 0,
        max_retries,
        last_error: None,
        skip_reason: None,
        processing_stage: Some("queued".to_string()),
        stage_message: Some("已进入订阅队列，等待下一轮自动处理".to_string()),
        stage_updated_at: Some(now),
        next_action: Some("自动搜索候选种子并推送 qB".to_string()),
        candidate_matches: Vec::new(),
        last_push: None,
        last_completion: None,
        created_at: now,
        updated_at: now,
        first_seen_at: now,
        last_seen_at: now,
    };
    if let Some(detail) = detail {
        apply_subject_detail_cache(&mut record, detail);
    }
    record
}

#[allow(dead_code)]
fn refresh_record_from_item(
    record: &mut WantedSubscriptionRecord,
    item: &DoubanLibraryItem,
    return_order: usize,
    now: u64,
) {
    refresh_record_from_item_with_detail(record, item, None, return_order, now)
}

fn refresh_record_from_item_with_detail(
    record: &mut WantedSubscriptionRecord,
    item: &DoubanLibraryItem,
    detail: Option<&DoubanSubjectDetail>,
    return_order: usize,
    now: u64,
) {
    let tags = normalized_tags(&item.tags);
    let douban_date = normalized_douban_date(&item.date);
    record.title = item.title.trim().to_string();
    record.release_year = release_year_from_item(item).or(record.release_year);
    let poster_url = item.poster_url.trim();
    if !poster_url.is_empty() {
        record.poster_url = poster_url.to_string();
    }
    let cover_url = item.cover_url.trim();
    if !cover_url.is_empty() {
        record.cover_url = cover_url.to_string();
    }
    record.category_text = tags
        .first()
        .cloned()
        .or_else(|| record.category_text.clone());
    record.tags = tags;
    record.douban_sort_time = douban_date
        .as_deref()
        .and_then(douban_date_sort_key)
        .or(record.douban_sort_time);
    record.douban_date = douban_date.or_else(|| record.douban_date.clone());
    record.douban_return_order = Some(return_order.min(u32::MAX as usize) as u32);
    record.last_seen_at = now;
    record.updated_at = now;
    if let Some(detail) = detail {
        apply_subject_detail_cache(record, detail);
    }
}

fn apply_subject_detail_cache(record: &mut WantedSubscriptionRecord, detail: &DoubanSubjectDetail) {
    let title = detail.title.trim();
    if !title.is_empty() {
        record.title = title.to_string();
    }
    let poster_url = detail.poster_url.trim();
    if !poster_url.is_empty() {
        record.poster_url = poster_url.to_string();
    }
    let image = detail.image.trim();
    if record.cover_url.trim().is_empty() && !image.is_empty() {
        record.cover_url = image.to_string();
    }
    record.original_title =
        non_empty_string(&detail.original_title).or(record.original_title.take());
    replace_vec_if_not_empty(&mut record.aka, &detail.aka);
    replace_vec_if_not_empty(&mut record.languages, &detail.languages);
    replace_vec_if_not_empty(&mut record.countries, &detail.countries);
    replace_vec_if_not_empty(&mut record.genres, &detail.genres);
    replace_vec_if_not_empty(&mut record.directors, &detail.directors);
    replace_vec_if_not_empty(&mut record.actors, &detail.actors);
    record.date_published =
        non_empty_string(&detail.date_published).or(record.date_published.take());
    record.duration = non_empty_string(&detail.duration).or(record.duration.take());
    record.summary = non_empty_string(&detail.summary).or(record.summary.take());
    record.rating_value = detail.rating.value.or(record.rating_value);
    record.rating_count = detail.rating.count.or(record.rating_count);
}

fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn replace_vec_if_not_empty(target: &mut Vec<String>, source: &[String]) {
    let values = source
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    if !values.is_empty() {
        *target = values;
    }
}

fn normalized_douban_date(raw: &str) -> Option<String> {
    let value = raw.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn douban_date_sort_key(raw: &str) -> Option<u64> {
    let digits = raw
        .chars()
        .filter(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.len() < 4 {
        return None;
    }
    let trimmed = if digits.len() > 14 {
        &digits[..14]
    } else {
        digits.as_str()
    };
    trimmed.parse::<u64>().ok()
}

fn apply_status_update(
    record: &mut WantedSubscriptionRecord,
    update: WantedStatusUpdate,
    max_retries: u32,
    now: u64,
) -> bool {
    let max_retries = max_retries.max(record.max_retries);
    record.max_retries = max_retries;
    record.updated_at = now;
    match update.status {
        WantedSubscriptionStatus::Failed => {
            record.retry_count = record.retry_count.saturating_add(1);
            record.last_error = update.error.filter(|s| !s.trim().is_empty());
            let exhausted = record.retry_count >= max_retries;
            record.status = if exhausted {
                WantedSubscriptionStatus::Failed
            } else {
                WantedSubscriptionStatus::Unprocessed
            };
            let message = record
                .last_error
                .clone()
                .unwrap_or_else(|| "本次处理失败，等待下次自动重试".to_string());
            if exhausted {
                set_stage(record, "error", &message, Some("检查错误后手动重试"), now);
            } else {
                set_stage(record, "queued", &message, Some("等待下一轮自动重试"), now);
            }
            exhausted
        }
        WantedSubscriptionStatus::Skipped => {
            record.status = WantedSubscriptionStatus::Skipped;
            record.skip_reason = update.skip_reason.filter(|s| !s.trim().is_empty());
            apply_status_stage(record, now);
            false
        }
        status => {
            record.status = status;
            if matches!(
                status,
                WantedSubscriptionStatus::Matching
                    | WantedSubscriptionStatus::Processing
                    | WantedSubscriptionStatus::Pushed
                    | WantedSubscriptionStatus::Downloading
                    | WantedSubscriptionStatus::Completed
                    | WantedSubscriptionStatus::Linked
            ) {
                record.last_error = None;
                record.skip_reason = None;
                record
                    .attention_tags
                    .retain(|tag| *tag != SubscriptionAttentionTag::Skipped);
            }
            apply_status_stage(record, now);
            false
        }
    }
}

fn hydrate_stage_from_record(record: &mut WantedSubscriptionRecord) {
    let now = record.updated_at.max(record.created_at);
    if record.last_completion.is_some() {
        apply_completion_stage(record, now);
    } else if record.last_push.is_some() {
        apply_push_stage(record, now);
    } else if !record.candidate_matches.is_empty() {
        apply_candidate_stage(record, now);
    } else {
        apply_status_stage(record, now);
    }
}

fn set_stage(
    record: &mut WantedSubscriptionRecord,
    stage: &str,
    message: &str,
    next_action: Option<&str>,
    now: u64,
) {
    record.processing_stage = Some(stage.to_string());
    record.stage_message = Some(message.to_string());
    record.stage_updated_at = Some(now.max(record.updated_at).max(record.created_at));
    record.next_action = next_action.map(str::to_string);
    sync_lifecycle_from_legacy_stage(record, now);
}

fn sync_lifecycle_from_legacy_stage(record: &mut WantedSubscriptionRecord, now: u64) {
    let (state, mut tags) = infer_lifecycle_from_legacy(record);
    record.lifecycle_state = state;
    record.execution_state = SubscriptionExecutionState::Idle;
    record.next_attempt_at.get_or_insert(now);
    if record.failure.is_none() {
        record.failure = failure_from_legacy_error(record, now);
    }
    preserve_retry_blocked_attention_tag(record, &mut tags);
    merge_attention_tags(record, tags);
}

fn apply_status_stage(record: &mut WantedSubscriptionRecord, now: u64) {
    match record.status {
        WantedSubscriptionStatus::Unprocessed => set_stage(
            record,
            "queued",
            "已进入订阅队列，等待下一轮自动处理",
            Some("自动搜索候选种子并推送 qB"),
            now,
        ),
        WantedSubscriptionStatus::Matching => set_stage(
            record,
            "searching",
            "正在搜索 M-Team 候选种子",
            Some("等待搜索结果并应用匹配规则"),
            now,
        ),
        WantedSubscriptionStatus::Processing => set_stage(
            record,
            "pushing",
            "正在获取下载链接并推送 qB",
            Some("等待 qB 接收任务"),
            now,
        ),
        WantedSubscriptionStatus::Pushed => set_stage(
            record,
            "pushed",
            "已推送到 qB，等待下载进度同步",
            Some("同步 qB 下载进度"),
            now,
        ),
        WantedSubscriptionStatus::Downloading => set_stage(
            record,
            "downloading",
            "等待 qB 下载完成",
            Some("下载完成后检查并硬链接"),
            now,
        ),
        WantedSubscriptionStatus::Completed => set_stage(
            record,
            "download_complete",
            "qB 下载已完成，等待硬链接",
            Some("执行完成检查并创建硬链接"),
            now,
        ),
        WantedSubscriptionStatus::Linked => set_stage(record, "linked", "硬链接已完成", None, now),
        WantedSubscriptionStatus::Failed => set_stage(
            record,
            "error",
            &record
                .last_error
                .clone()
                .unwrap_or_else(|| "订阅处理失败，等待检查配置或手动重试".to_string()),
            Some("检查错误后重新轮询或手动重试"),
            now,
        ),
        WantedSubscriptionStatus::Skipped => set_stage(
            record,
            "skipped",
            &subscription_skip_reason_message(record.skip_reason.as_deref()),
            None,
            now,
        ),
    }
}

fn subscription_skip_reason_message(reason: Option<&str>) -> String {
    match reason.map(str::trim).filter(|value| !value.is_empty()) {
        Some("initial_bootstrap_existing_wish") => "历史想看，首次同步跳过".to_string(),
        Some(value) => value.to_string(),
        None => "已跳过该订阅".to_string(),
    }
}

fn apply_candidate_stage(record: &mut WantedSubscriptionRecord, now: u64) {
    if record.candidate_matches.is_empty() {
        set_stage(
            record,
            "no_candidates",
            "未搜索到候选种子",
            Some("等待新种子或调整标题/配置后重试"),
            now,
        );
    } else if record.candidate_matches.iter().any(|item| item.selected) {
        set_stage(
            record,
            "matched",
            "已匹配到候选种子，等待推送 qB",
            Some("获取下载链接并推送 qB"),
            now,
        );
    } else {
        set_stage(
            record,
            "no_match",
            "候选种子未命中当前匹配规则",
            Some("调整匹配规则或等待新种子后重试"),
            now,
        );
    }
}

fn apply_push_stage(record: &mut WantedSubscriptionRecord, now: u64) {
    let Some(push) = record.last_push.as_ref() else {
        apply_status_stage(record, now);
        return;
    };
    match push.status.as_str() {
        "failed" => {
            let message = record
                .last_error
                .clone()
                .or_else(|| push.error.clone())
                .unwrap_or_else(|| "推送 qB 失败".to_string());
            let stage = if message.contains("未搜索到候选种子") {
                "no_candidates"
            } else if message.contains("没有候选种子匹配当前规则") {
                "no_match"
            } else {
                "push_failed"
            };
            let next_action = if stage == "no_candidates" {
                "等待新种子或调整标题/配置后重试"
            } else if stage == "no_match" {
                "调整匹配规则或等待新种子后重试"
            } else {
                "检查 qB/M-Team 配置后重试"
            };
            set_stage(record, stage, &message, Some(next_action), now);
        }
        "downloading" => set_stage(
            record,
            "downloading",
            "等待 qB 下载完成",
            Some("下载完成后检查并硬链接"),
            now,
        ),
        "downloaded" => set_stage(
            record,
            "download_complete",
            "qB 下载已完成，等待硬链接",
            Some("执行完成检查并创建硬链接"),
            now,
        ),
        "linked" | "completed" => set_stage(record, "linked", "硬链接已完成", None, now),
        _ => set_stage(
            record,
            "pushed",
            "已推送到 qB，等待下载进度同步",
            Some("同步 qB 下载进度"),
            now,
        ),
    }
}

fn apply_completion_stage(record: &mut WantedSubscriptionRecord, now: u64) {
    let Some(completion) = record.last_completion.as_ref() else {
        apply_push_stage(record, now);
        return;
    };
    match completion.status.as_str() {
        "pending" => set_stage(
            record,
            "downloading",
            "等待 qB 下载完成",
            Some("下载完成后检查并硬链接"),
            now,
        ),
        "failed" => {
            let message = record
                .last_error
                .clone()
                .or_else(|| completion.error.clone())
                .unwrap_or_else(|| "硬链接失败".to_string());
            set_stage(
                record,
                "link_failed",
                &message,
                Some("检查源目录/目标目录后重试"),
                now,
            );
        }
        "dry_run" => set_stage(
            record,
            "link_planned",
            "硬链接预演完成，等待执行",
            Some("执行完成检查并创建硬链接"),
            now,
        ),
        "completed" => set_stage(record, "linked", "硬链接已完成", None, now),
        _ => apply_push_stage(record, now),
    }
}

fn normalized_tags(tags: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for tag in tags {
        let t = tag.trim();
        if !t.is_empty() && !out.iter().any(|x| x == t) {
            out.push(t.to_string());
        }
    }
    out
}

fn release_year_from_item(item: &DoubanLibraryItem) -> Option<u16> {
    release_year_from_text(&item.abstract_text)
        .or_else(|| release_year_from_text(&item.abstract_2))
        .or_else(|| release_year_from_text(&item.date))
}

fn release_year_from_text(text: &str) -> Option<u16> {
    let bytes = text.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    for idx in 0..=bytes.len() - 4 {
        if bytes[idx..idx + 4].iter().all(|b| b.is_ascii_digit()) {
            let year: u16 = text[idx..idx + 4].parse().ok()?;
            if (1888..=2200).contains(&year) {
                return Some(year);
            }
        }
    }
    None
}

fn safe_account_key(raw: &str) -> String {
    let key = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect::<String>();
    if key.is_empty() {
        "current".to_string()
    } else {
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, title: &str, abstract_text: &str, tags: &[&str]) -> DoubanLibraryItem {
        DoubanLibraryItem {
            source: "douban",
            media_type: "douban",
            id: id.to_string(),
            subject_id: id.to_string(),
            title: title.to_string(),
            url: format!("https://movie.douban.com/subject/{id}/"),
            abstract_text: abstract_text.to_string(),
            abstract_2: String::new(),
            cover_url: String::new(),
            poster_url: String::new(),
            status: "wish",
            status_label: "想看",
            date: "2026-06-22".to_string(),
            comment: String::new(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            user_rating: None,
        }
    }

    fn item_with_date(id: &str, title: &str, date: &str) -> DoubanLibraryItem {
        let mut item = item(id, title, "2024 / 中国大陆 / 剧情", &["电影"]);
        item.date = date.to_string();
        item
    }

    fn subject_detail(id: &str, date_published: &str) -> crate::douban::DoubanSubjectDetail {
        crate::douban::DoubanSubjectDetail {
            source: "douban",
            media_type: "douban",
            id: id.to_string(),
            subject_id: id.to_string(),
            url: format!("https://movie.douban.com/subject/{id}/"),
            title: "中文片名".to_string(),
            original_title: "Original Title".to_string(),
            aka: vec!["别名一".to_string(), "别名二".to_string()],
            languages: vec!["汉语普通话".to_string()],
            countries: vec!["中国大陆".to_string()],
            image: "/api/douban/image?url=image".to_string(),
            poster_url: "/api/douban/image?url=poster".to_string(),
            directors: vec!["导演甲".to_string()],
            writers: vec!["编剧甲".to_string()],
            actors: vec!["主演甲".to_string(), "主演乙".to_string()],
            genres: vec!["剧情".to_string(), "犯罪".to_string()],
            date_published: date_published.to_string(),
            duration: "120分钟".to_string(),
            summary: "这是一段简介。".to_string(),
            rating: crate::douban::DoubanRating {
                value: Some(8.7),
                count: Some(12345),
                info: String::new(),
                star_count: Some(4.5),
            },
            user_interest: None,
            user_rating: None,
        }
    }

    #[test]
    fn wanted_records_preserve_douban_cover_urls() {
        let mut first = item("1", "电影一", "2024 / 中国大陆 / 剧情", &["电影"]);
        first.poster_url = "/api/douban/image?url=poster-one".to_string();
        first.cover_url = "/api/douban/image?url=cover-one".to_string();

        let created = record_from_item(&first, 0, 3, 100);

        assert_eq!(created.poster_url, "/api/douban/image?url=poster-one");
        assert_eq!(created.cover_url, "/api/douban/image?url=cover-one");

        let mut refreshed = record_from_item(&first, 0, 3, 100);
        let mut next = item("1", "电影一", "2024 / 中国大陆 / 剧情", &["电影"]);
        next.poster_url = "/api/douban/image?url=poster-two".to_string();
        next.cover_url = "/api/douban/image?url=cover-two".to_string();

        refresh_record_from_item(&mut refreshed, &next, 1, 200);

        assert_eq!(refreshed.poster_url, "/api/douban/image?url=poster-two");
        assert_eq!(refreshed.cover_url, "/api/douban/image?url=cover-two");
    }

    #[test]
    fn lifecycle_statuses_exclude_execution_and_attention_states() {
        let labels = SubscriptionLifecycleState::ALL
            .iter()
            .map(|state| state.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            labels,
            vec![
                "queued",
                "meta",
                "searching",
                "downloading",
                "linking",
                "completed"
            ]
        );
        assert!(!labels.contains(&"failed"));
        assert!(!labels.contains(&"skipped"));
        assert!(!labels.contains(&"running"));
        assert!(!labels.contains(&"idle"));
    }

    #[test]
    fn record_defaults_to_new_queued_idle_model() {
        let item = item("subject-1", "测试电影", "2026 / 中国大陆 / 电影", &["电影"]);
        let record = record_from_item(&item, 0, 3, 1_000);

        assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Queued);
        assert_eq!(record.execution_state, SubscriptionExecutionState::Idle);
        assert!(record.attention_tags.is_empty());
        assert!(record.failure.is_none());
        assert_eq!(record.next_attempt_at, Some(1_000));
    }

    fn bare_record_with_status(
        status: WantedSubscriptionStatus,
        stage: Option<&str>,
    ) -> WantedSubscriptionRecord {
        let mut record = record_from_item(
            &item("legacy", "旧订阅", "2026 / 中国大陆 / 电影", &["电影"]),
            0,
            3,
            1_000,
        );
        record.status = status;
        record.processing_stage = stage.map(str::to_string);
        record.lifecycle_state = SubscriptionLifecycleState::Queued;
        record.attention_tags.clear();
        record
    }

    fn test_push(status: &str, error: Option<&str>) -> TorrentPushRecord {
        TorrentPushRecord {
            subscription_id: "legacy".to_string(),
            torrent_id: "torrent-1".to_string(),
            torrent_title: "Legacy.Seed".to_string(),
            qb_server: "default".to_string(),
            qb_server_id: "default".to_string(),
            qb_category: "cat".to_string(),
            qb_save_dir_name: "save".to_string(),
            qb_identifier: String::new(),
            torrent_download_url: None,
            mteam_torrent_url: None,
            pushed_at: Some(1_000),
            status: status.to_string(),
            error: error.map(str::to_string),
            qb_hash: None,
            qb_name: None,
            checked_at: None,
            completed_at: None,
            download_progress: None,
            download_state: None,
            total_size: None,
            completed_file_count: None,
            total_file_count: None,
            files: Vec::new(),
            episodes: Vec::new(),
            source_path: None,
            target_dir: None,
            linked_files: Vec::new(),
        }
    }

    fn test_completion(status: &str) -> HardlinkCompletionRecord {
        HardlinkCompletionRecord {
            status: status.to_string(),
            checked_at: 1_000,
            completed_at: None,
            qb_hash: None,
            qb_name: None,
            source_path: None,
            target_dir: None,
            linked_files: Vec::new(),
            episodes: Vec::new(),
            error: None,
        }
    }

    fn bare_tv_record(
        subject_id: &str,
        season: u32,
        episode_total: u32,
    ) -> WantedSubscriptionRecord {
        let mut record = record_from_item(
            &item(
                subject_id,
                "测试剧集",
                "2026 / 中国大陆 / 剧情",
                &["电视剧"],
            ),
            0,
            3,
            1_000,
        );
        record.media_kind = SubscriptionMediaKind::Tv;
        record.lifecycle_state = SubscriptionLifecycleState::Meta;
        record.tv = Some(test_tv_state(season, episode_total));
        record
    }

    fn test_tv_state(season: u32, episode_total: u32) -> TvSubscriptionState {
        let mut record = record_from_item(
            &item("tv", "测试剧集", "2026 / 中国大陆 / 剧情", &["电视剧"]),
            0,
            3,
            1_000,
        );
        initialize_tv_targets(
            &mut record,
            TvTargetRange::episodes(season, 1, episode_total),
            1_000,
        )
        .unwrap();
        record.tv.unwrap()
    }

    fn episode(tv: &TvSubscriptionState, episode: u32) -> &TvEpisodeRecord {
        tv.episodes
            .iter()
            .find(|ep| ep.episode_number == episode)
            .expect("episode exists")
    }

    fn test_failure(operation: &str, retry_blocked: bool) -> SubscriptionFailure {
        SubscriptionFailure {
            scope: FailureScope::Episode,
            owner_id: "owner".to_string(),
            operation: operation.to_string(),
            error_type: "test".to_string(),
            message: "test failure".to_string(),
            retry_count: if retry_blocked { 3 } else { 1 },
            max_retries: 3,
            failed_at: 1_000,
            next_retry_at: None,
            retry_blocked,
        }
    }

    fn test_task(
        task_id: &str,
        state: DownloadTaskState,
        start: u32,
        end: u32,
    ) -> DownloadTaskRecord {
        DownloadTaskRecord {
            task_id: task_id.to_string(),
            torrent_id: task_id.to_string(),
            torrent_title: task_id.to_string(),
            qb_server_id: "default".to_string(),
            qb_category: "cat".to_string(),
            qb_save_dir_name: "save".to_string(),
            qb_hash: None,
            qb_name: None,
            state,
            tentative_coverage: vec![CoverageRange::range(1, start, end)],
            verified_coverage: Vec::new(),
            progress: None,
            pushed_at: 1_000,
            checked_at: None,
            completed_at: None,
            failure: None,
        }
    }

    fn tv_record_with_linkable_e01_and_uncovered_e05() -> WantedSubscriptionRecord {
        let mut record = bare_tv_record("subject-tv", 1, 8);
        let tv = record.tv.as_mut().unwrap();
        bind_task_to_episodes(
            tv,
            "task-e01",
            CoverageRange::single(1, 1),
            CoverageTrust::Verified,
            1_000,
        )
        .unwrap();
        episode_mut(tv, 1).download_state = "downloaded".to_string();
        tv.download_tasks
            .push(test_task("task-e01", DownloadTaskState::Downloaded, 1, 1));
        for ep in 2..=4 {
            let ep = episode_mut(tv, ep);
            ep.assignment_state = EpisodeAssignmentState::Completed;
            ep.target_state = EpisodeTargetState::Completed;
            ep.coverage_state = EpisodeCoverageState::VerifiedCovered;
        }
        tv.search_cursor_episode = Some(5);
        record
    }

    fn tv_record_with_cursor_past_end_but_active_download() -> WantedSubscriptionRecord {
        let mut record = bare_tv_record("subject-tv", 1, 3);
        let tv = record.tv.as_mut().unwrap();
        bind_task_to_episodes(
            tv,
            "task-e01-e03",
            CoverageRange::range(1, 1, 3),
            CoverageTrust::Tentative,
            1_000,
        )
        .unwrap();
        tv.search_cursor_episode = Some(4);
        tv.download_tasks.push(test_task(
            "task-e01-e03",
            DownloadTaskState::Downloading,
            1,
            3,
        ));
        record
    }

    fn tv_record_ready_for_search(subject_id: &str) -> WantedSubscriptionRecord {
        let mut record = bare_tv_record(subject_id, 1, 8);
        record.lifecycle_state = SubscriptionLifecycleState::Searching;
        record.status = derive_legacy_status(&record);
        record
    }

    fn test_watcher_cfg() -> SubscriptionWatcherConfig {
        SubscriptionWatcherConfig::default()
    }

    fn movie_record_in_state(
        lifecycle_state: SubscriptionLifecycleState,
        next_attempt_at: u64,
    ) -> WantedSubscriptionRecord {
        let mut record = record_from_item(
            &item("movie", "测试电影", "2026 / 中国大陆 / 电影", &["电影"]),
            0,
            3,
            1_000,
        );
        record.lifecycle_state = lifecycle_state;
        record.status = derive_legacy_status(&record);
        record.next_attempt_at = Some(next_attempt_at);
        record
    }

    fn tv_record_all_lanes_due(now: u64) -> WantedSubscriptionRecord {
        let mut record = tv_record_ready_for_search("subject-tv");
        let tv = record.tv.as_mut().unwrap();
        tv.lanes.link.next_attempt_at = Some(now);
        tv.lanes.progress.next_attempt_at = Some(now);
        tv.lanes.search.next_attempt_at = Some(now);
        record
    }

    fn complete_all_tv_episodes(tv: &mut TvSubscriptionState, now: u64) {
        for task in &mut tv.download_tasks {
            task.state = DownloadTaskState::Completed;
            task.completed_at = Some(now);
        }
        for ep in &mut tv.episodes {
            ep.target_state = EpisodeTargetState::Completed;
            ep.assignment_state = EpisodeAssignmentState::Completed;
            ep.coverage_state = EpisodeCoverageState::VerifiedCovered;
            ep.download_state = "downloaded".to_string();
            ep.link_state = "linked".to_string();
            ep.updated_at = now;
        }
        tv.search_cursor_episode = Some(tv.episode_total + 1);
    }

    fn episode_mut(tv: &mut TvSubscriptionState, episode: u32) -> &mut TvEpisodeRecord {
        tv.episodes
            .iter_mut()
            .find(|ep| ep.episode_number == episode)
            .expect("episode exists")
    }

    fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
        let mut stmt = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    struct IndexedRecordRow {
        lifecycle_state: String,
        media_kind: String,
        search_next_attempt_at: Option<i64>,
        record_json: String,
    }

    fn indexed_record_row(
        conn: &Connection,
        account_key: &str,
        subject_id: &str,
    ) -> IndexedRecordRow {
        conn.query_row(
            "SELECT lifecycle_state, media_kind, search_next_attempt_at, record_json
                FROM wanted_subscription_records
                WHERE account_key = ?1 AND subject_id = ?2",
            params![account_key, subject_id],
            |row| {
                Ok(IndexedRecordRow {
                    lifecycle_state: row.get(0)?,
                    media_kind: row.get(1)?,
                    search_next_attempt_at: row.get(2)?,
                    record_json: row.get(3)?,
                })
            },
        )
        .unwrap()
    }

    #[test]
    fn legacy_statuses_map_to_new_lifecycle_and_tags() {
        let cases = vec![
            (
                WantedSubscriptionStatus::Unprocessed,
                None,
                SubscriptionLifecycleState::Queued,
                vec![],
            ),
            (
                WantedSubscriptionStatus::Matching,
                None,
                SubscriptionLifecycleState::Searching,
                vec![],
            ),
            (
                WantedSubscriptionStatus::Processing,
                None,
                SubscriptionLifecycleState::Searching,
                vec![],
            ),
            (
                WantedSubscriptionStatus::Pushed,
                None,
                SubscriptionLifecycleState::Downloading,
                vec![],
            ),
            (
                WantedSubscriptionStatus::Downloading,
                None,
                SubscriptionLifecycleState::Downloading,
                vec![],
            ),
            (
                WantedSubscriptionStatus::Completed,
                None,
                SubscriptionLifecycleState::Linking,
                vec![],
            ),
            (
                WantedSubscriptionStatus::Linked,
                None,
                SubscriptionLifecycleState::Completed,
                vec![],
            ),
            (
                WantedSubscriptionStatus::Skipped,
                None,
                SubscriptionLifecycleState::Queued,
                vec![SubscriptionAttentionTag::Skipped],
            ),
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
        assert_eq!(
            completion_failed.lifecycle_state,
            SubscriptionLifecycleState::Linking
        );
        assert!(completion_failed
            .attention_tags
            .contains(&SubscriptionAttentionTag::Failed));

        let mut no_match =
            bare_record_with_status(WantedSubscriptionStatus::Failed, Some("no_match"));
        no_match.last_push = Some(test_push("failed", Some("没有匹配规则命中")));
        migrate_legacy_status_fields(&mut no_match, 2_000);
        assert_eq!(
            no_match.lifecycle_state,
            SubscriptionLifecycleState::Searching
        );
        assert!(no_match
            .attention_tags
            .contains(&SubscriptionAttentionTag::WaitingRelease));
    }

    #[test]
    fn legacy_migration_preserves_retry_blocked_tag_for_blocked_failure() {
        let mut record = bare_record_with_status(WantedSubscriptionStatus::Completed, None);
        record.attention_tags = vec![SubscriptionAttentionTag::RetryBlocked];
        record.failure = Some(SubscriptionFailure {
            scope: FailureScope::Parent,
            owner_id: record.subject_id.clone(),
            operation: "link".to_string(),
            error_type: "system".to_string(),
            message: "hardlink failed".to_string(),
            retry_count: 3,
            max_retries: 3,
            failed_at: 1_500,
            next_retry_at: None,
            retry_blocked: true,
        });

        migrate_legacy_status_fields(&mut record, 2_000);

        assert!(record.failure.as_ref().unwrap().retry_blocked);
        assert!(record
            .attention_tags
            .contains(&SubscriptionAttentionTag::RetryBlocked));
    }

    #[test]
    fn legacy_stage_sync_preserves_retry_blocked_tag_for_blocked_failure() {
        let mut record = bare_record_with_status(WantedSubscriptionStatus::Completed, None);
        record.attention_tags = vec![SubscriptionAttentionTag::RetryBlocked];
        record.failure = Some(SubscriptionFailure {
            scope: FailureScope::Parent,
            owner_id: record.subject_id.clone(),
            operation: "link".to_string(),
            error_type: "system".to_string(),
            message: "hardlink failed".to_string(),
            retry_count: 3,
            max_retries: 3,
            failed_at: 1_500,
            next_retry_at: None,
            retry_blocked: true,
        });

        set_stage(
            &mut record,
            "download_complete",
            "下载完成，等待硬链接",
            Some("执行硬链接"),
            2_000,
        );

        assert!(record.failure.as_ref().unwrap().retry_blocked);
        assert!(record
            .attention_tags
            .contains(&SubscriptionAttentionTag::RetryBlocked));
    }

    #[test]
    fn tv_meta_initializes_target_episodes_and_cursor() {
        let mut record = bare_tv_record("subject-tv", 1, 8);
        initialize_tv_targets(&mut record, TvTargetRange::episodes(1, 1, 8), 1_000).unwrap();

        assert_eq!(record.tv.as_ref().unwrap().episode_total, 8);
        assert_eq!(record.tv.as_ref().unwrap().search_cursor_episode, Some(1));
        assert_eq!(record.tv.as_ref().unwrap().episodes.len(), 8);
        assert_eq!(
            record.tv.as_ref().unwrap().lanes.search.next_attempt_at,
            Some(1_000)
        );
        assert_eq!(
            record.tv.as_ref().unwrap().lanes.progress.next_attempt_at,
            Some(1_000)
        );
        assert_eq!(
            record.tv.as_ref().unwrap().lanes.link.next_attempt_at,
            Some(1_000)
        );
    }

    #[test]
    fn cursor_advances_over_active_completed_blocked_and_skipped_assignments() {
        let mut tv = test_tv_state(1, 8);
        bind_task_to_episodes(
            &mut tv,
            "task-e01",
            CoverageRange::single(1, 1),
            CoverageTrust::Tentative,
            1_000,
        )
        .unwrap();
        block_episode_assignment(&mut tv, 1, 3, test_failure("link", true)).unwrap();
        skip_episode_range(&mut tv, 1, 4, 4, "user skipped", 1_000).unwrap();

        recalculate_search_cursor(&mut tv);

        assert_eq!(tv.search_cursor_episode, Some(2));
    }

    #[test]
    fn verified_coverage_loss_releases_unverified_episodes_and_rewinds_cursor() {
        let mut tv = test_tv_state(1, 8);
        bind_task_to_episodes(
            &mut tv,
            "task-e02-e04",
            CoverageRange::range(1, 2, 4),
            CoverageTrust::Tentative,
            1_000,
        )
        .unwrap();
        apply_verified_coverage(&mut tv, "task-e02-e04", CoverageRange::single(1, 2), 2_000)
            .unwrap();

        assert_eq!(
            episode(&tv, 3).assignment_state,
            EpisodeAssignmentState::Released
        );
        assert_eq!(
            episode(&tv, 4).assignment_state,
            EpisodeAssignmentState::Released
        );
        assert_eq!(tv.search_cursor_episode, Some(3));
    }

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
        assert_eq!(
            record.lifecycle_state,
            SubscriptionLifecycleState::Downloading
        );

        complete_all_tv_episodes(record.tv.as_mut().unwrap(), 2_000);
        derive_tv_parent_lifecycle(&mut record);
        assert_eq!(
            record.lifecycle_state,
            SubscriptionLifecycleState::Completed
        );
    }

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
        state.records.insert(
            "subject-tv".into(),
            tv_record_ready_for_search("subject-tv"),
        );

        save_state_to_db(&mut conn, "acct", &state).unwrap();

        let row = indexed_record_row(&conn, "acct", "subject-tv");
        assert_eq!(row.lifecycle_state, "searching");
        assert_eq!(row.media_kind, "tv");
        assert!(row.search_next_attempt_at.is_some());
        assert!(!row.record_json.is_empty());
    }

    #[test]
    fn movie_successful_transition_is_immediately_due_for_next_state() {
        let cfg = test_watcher_cfg();
        let mut record = movie_record_in_state(SubscriptionLifecycleState::Queued, 1_000);

        apply_movie_operation_outcome(
            &mut record,
            MovieOperationOutcome::Advanced(SubscriptionLifecycleState::Meta),
            &cfg,
            2_000,
        );

        assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Meta);
        assert_eq!(record.next_attempt_at, Some(2_000));
    }

    #[test]
    fn unchanged_movie_state_waits_for_state_interval() {
        let cfg = test_watcher_cfg();
        let mut record = movie_record_in_state(SubscriptionLifecycleState::Downloading, 1_000);

        apply_movie_operation_outcome(
            &mut record,
            MovieOperationOutcome::StillDownloading,
            &cfg,
            2_000,
        );

        assert_eq!(
            record.lifecycle_state,
            SubscriptionLifecycleState::Downloading
        );
        assert_eq!(
            record.next_attempt_at,
            Some(2_000 + cfg.progress_interval_secs)
        );
    }

    #[test]
    fn movie_search_waiting_release_keeps_searching_without_retry_churn() {
        let cfg = test_watcher_cfg();
        let mut record = movie_record_in_state(SubscriptionLifecycleState::Searching, 1_000);
        record.retry_count = 2;

        apply_movie_waiting_release(&mut record, "未搜索到候选种子", &cfg, 2_000);

        assert_eq!(
            record.lifecycle_state,
            SubscriptionLifecycleState::Searching
        );
        assert!(record
            .attention_tags
            .contains(&SubscriptionAttentionTag::WaitingRelease));
        assert_eq!(record.retry_count, 2);
        assert_eq!(
            record.next_attempt_at,
            Some(2_000 + cfg.search_interval_secs)
        );
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
    fn movie_advanced_outcome_clears_stale_failure_state() {
        let cfg = test_watcher_cfg();
        let mut record = movie_record_in_state(SubscriptionLifecycleState::Downloading, 1_000);
        record.execution_state = SubscriptionExecutionState::Running;
        record.attention_tags = vec![
            SubscriptionAttentionTag::Skipped,
            SubscriptionAttentionTag::Failed,
            SubscriptionAttentionTag::RetryBlocked,
        ];
        record.failure = Some(SubscriptionFailure {
            scope: FailureScope::Parent,
            owner_id: record.subject_id.clone(),
            operation: "link".to_string(),
            error_type: "system".to_string(),
            message: "hardlink failed".to_string(),
            retry_count: 3,
            max_retries: 3,
            failed_at: 1_500,
            next_retry_at: None,
            retry_blocked: true,
        });
        record.last_error = Some("hardlink failed".to_string());

        apply_movie_operation_outcome(
            &mut record,
            MovieOperationOutcome::Advanced(SubscriptionLifecycleState::Linking),
            &cfg,
            2_000,
        );

        assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Linking);
        assert_eq!(record.next_attempt_at, Some(2_000));
        assert_eq!(record.execution_state, SubscriptionExecutionState::Idle);
        assert_eq!(record.updated_at, 2_000);
        assert!(record.failure.is_none());
        assert!(record.last_error.is_none());
        assert!(record
            .attention_tags
            .contains(&SubscriptionAttentionTag::Skipped));
        assert!(!record
            .attention_tags
            .contains(&SubscriptionAttentionTag::Failed));
        assert!(!record
            .attention_tags
            .contains(&SubscriptionAttentionTag::RetryBlocked));
    }

    #[test]
    fn movie_link_failure_stays_linking_with_retry_due_time() {
        let cfg = test_watcher_cfg();
        let mut record = movie_record_in_state(SubscriptionLifecycleState::Linking, 1_000);

        apply_parent_operation_failure(&mut record, "link", "hardlink failed", &cfg, 2_000);

        assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Linking);
        assert!(record
            .attention_tags
            .contains(&SubscriptionAttentionTag::Failed));
        assert_eq!(record.failure.as_ref().unwrap().operation, "link");
        assert_eq!(
            record.next_attempt_at,
            Some(2_000 + cfg.link_retry_interval_secs)
        );
    }

    #[test]
    fn retry_blocked_parent_failure_retains_failed_and_retry_blocked_tags() {
        let cfg = test_watcher_cfg();
        let mut record = movie_record_in_state(SubscriptionLifecycleState::Linking, 1_000);
        record.retry_count = 2;
        record.max_retries = 3;

        apply_parent_operation_failure(&mut record, "link", "hardlink failed", &cfg, 2_000);

        assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Linking);
        assert!(record
            .attention_tags
            .contains(&SubscriptionAttentionTag::Failed));
        assert!(record
            .attention_tags
            .contains(&SubscriptionAttentionTag::RetryBlocked));
        assert!(record.failure.as_ref().unwrap().retry_blocked);
        assert_eq!(record.next_attempt_at, None);
    }

    #[test]
    fn non_blocked_parent_failure_clears_stale_retry_blocked_tag() {
        let cfg = test_watcher_cfg();
        let mut record = movie_record_in_state(SubscriptionLifecycleState::Linking, 1_000);
        record.retry_count = 1;
        record.max_retries = 3;
        record.attention_tags = vec![SubscriptionAttentionTag::RetryBlocked];

        apply_parent_operation_failure(&mut record, "link", "hardlink failed", &cfg, 2_000);

        assert_eq!(record.lifecycle_state, SubscriptionLifecycleState::Linking);
        assert!(record
            .attention_tags
            .contains(&SubscriptionAttentionTag::Failed));
        assert!(!record
            .attention_tags
            .contains(&SubscriptionAttentionTag::RetryBlocked));
        assert!(!record.failure.as_ref().unwrap().retry_blocked);
        assert_eq!(
            record.next_attempt_at,
            Some(2_000 + cfg.link_retry_interval_secs)
        );
    }

    #[test]
    fn movie_waiting_release_clears_stale_failed_and_retry_blocked_tags() {
        let cfg = test_watcher_cfg();
        let mut record = movie_record_in_state(SubscriptionLifecycleState::Searching, 1_000);
        record.attention_tags = vec![
            SubscriptionAttentionTag::Failed,
            SubscriptionAttentionTag::RetryBlocked,
        ];
        record.failure = Some(SubscriptionFailure {
            scope: FailureScope::Parent,
            owner_id: record.subject_id.clone(),
            operation: "link".to_string(),
            error_type: "system".to_string(),
            message: "hardlink failed".to_string(),
            retry_count: 3,
            max_retries: 3,
            failed_at: 1_500,
            next_retry_at: None,
            retry_blocked: true,
        });

        apply_movie_waiting_release(&mut record, "未搜索到候选种子", &cfg, 2_000);

        assert_eq!(
            record.lifecycle_state,
            SubscriptionLifecycleState::Searching
        );
        assert!(record
            .attention_tags
            .contains(&SubscriptionAttentionTag::WaitingRelease));
        assert!(!record
            .attention_tags
            .contains(&SubscriptionAttentionTag::Failed));
        assert!(!record
            .attention_tags
            .contains(&SubscriptionAttentionTag::RetryBlocked));
        assert!(record.failure.is_none());
    }

    #[test]
    fn tv_due_lanes_choose_link_then_progress_then_search_without_updating_others() {
        let record = tv_record_all_lanes_due(1_000);

        let selected = select_due_tv_lane(&record, 1_000).unwrap();

        assert_eq!(selected, TvLaneKind::Link);
        assert_eq!(
            record.tv.as_ref().unwrap().lanes.progress.next_attempt_at,
            Some(1_000)
        );
        assert_eq!(
            record.tv.as_ref().unwrap().lanes.search.next_attempt_at,
            Some(1_000)
        );
    }

    #[test]
    fn movie_due_operation_follows_lifecycle_only() {
        let cases = [
            (
                SubscriptionLifecycleState::Queued,
                Some(SubscriptionDueOperation::MovieMeta),
            ),
            (
                SubscriptionLifecycleState::Meta,
                Some(SubscriptionDueOperation::MovieMeta),
            ),
            (
                SubscriptionLifecycleState::Searching,
                Some(SubscriptionDueOperation::MovieSearch),
            ),
            (
                SubscriptionLifecycleState::Downloading,
                Some(SubscriptionDueOperation::MovieProgress),
            ),
            (
                SubscriptionLifecycleState::Linking,
                Some(SubscriptionDueOperation::MovieLink),
            ),
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

    #[test]
    fn wanted_records_cache_douban_subject_detail() {
        let mut state = WantedSubscriptionState::new("acct", 100);
        let cfg = SubscriptionWatcherConfig {
            bootstrap_existing_as_skipped: false,
            ..SubscriptionWatcherConfig::default()
        };
        let mut details = BTreeMap::new();
        details.insert("1".to_string(), subject_detail("1", "2026-07-01"));

        apply_wish_items_with_details_to_state(
            &mut state,
            &[item("1", "中文片名", "2026 / 中国大陆 / 剧情", &["电影"])],
            &details,
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            "state.json".to_string(),
            100,
        );

        let record = state.records.get("1").expect("created record");
        assert_eq!(record.title, "中文片名");
        assert_eq!(record.date_published.as_deref(), Some("2026-07-01"));
        assert_eq!(record.original_title.as_deref(), Some("Original Title"));
        assert_eq!(record.aka, vec!["别名一".to_string(), "别名二".to_string()]);
        assert_eq!(record.genres, vec!["剧情".to_string(), "犯罪".to_string()]);
        assert_eq!(record.countries, vec!["中国大陆".to_string()]);
        assert_eq!(record.languages, vec!["汉语普通话".to_string()]);
        assert_eq!(record.directors, vec!["导演甲".to_string()]);
        assert_eq!(
            record.actors,
            vec!["主演甲".to_string(), "主演乙".to_string()]
        );
        assert_eq!(record.duration.as_deref(), Some("120分钟"));
        assert_eq!(record.summary.as_deref(), Some("这是一段简介。"));
        assert_eq!(record.rating_value, Some(8.7));
        assert_eq!(record.rating_count, Some(12345));

        details.insert("1".to_string(), subject_detail("1", "2026-08-02"));
        apply_wish_items_with_details_to_state(
            &mut state,
            &[item("1", "中文片名", "2026 / 中国大陆 / 剧情", &["电影"])],
            &details,
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            "state.json".to_string(),
            200,
        );

        let refreshed = state.records.get("1").expect("refreshed record");
        assert_eq!(refreshed.date_published.as_deref(), Some("2026-08-02"));
    }

    #[test]
    fn bootstrap_existing_wish_items_are_skipped() {
        let mut state = WantedSubscriptionState::new("acct", 100);
        let cfg = SubscriptionWatcherConfig::default();
        let outcome = apply_wish_items_to_state(
            &mut state,
            &[item("1", "电影一", "2024 / 中国大陆 / 剧情", &["家庭"])],
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            "/tmp/state.json".to_string(),
            120,
        );

        assert!(outcome.bootstrap_mode);
        assert_eq!(outcome.created_skipped, 1);
        assert_eq!(outcome.created_unprocessed, 0);
        let rec = state.records.get("1").unwrap();
        assert_eq!(rec.status, WantedSubscriptionStatus::Skipped);
        assert_eq!(rec.release_year, Some(2024));
        assert_eq!(rec.category_text.as_deref(), Some("家庭"));
        assert_eq!(rec.processing_stage.as_deref(), Some("skipped"));
        assert_eq!(rec.stage_message.as_deref(), Some("历史想看，首次同步跳过"));
        assert_ne!(
            rec.stage_message.as_deref(),
            Some("initial_bootstrap_existing_wish")
        );
        assert_eq!(rec.next_action.as_deref(), None);
    }

    #[test]
    fn skipped_stage_hydration_does_not_leak_raw_skip_reason() {
        let mut rec = record_from_item(&item("1", "旧片", "2023 / 日本", &["日影"]), 0, 3, 100);
        rec.status = WantedSubscriptionStatus::Skipped;
        rec.skip_reason = Some("initial_bootstrap_existing_wish".to_string());
        rec.processing_stage = Some("skipped".to_string());
        rec.stage_message = Some("initial_bootstrap_existing_wish".to_string());
        rec.stage_updated_at = Some(150);
        rec.next_action = Some("raw".to_string());

        repair_record_defaults(&mut rec, 100, 120, 300);

        assert_eq!(rec.processing_stage.as_deref(), Some("skipped"));
        assert_eq!(rec.stage_message.as_deref(), Some("历史想看，首次同步跳过"));
        assert_eq!(rec.stage_updated_at, Some(150));
        assert_eq!(rec.next_action.as_deref(), None);
    }

    #[test]
    fn active_status_update_clears_historical_skip_reason() {
        let mut rec = record_from_item(&item("1", "旧片", "2023 / 日本", &["日影"]), 0, 3, 100);
        rec.status = WantedSubscriptionStatus::Skipped;
        rec.skip_reason = Some("initial_bootstrap_existing_wish".to_string());
        apply_status_stage(&mut rec, 120);
        assert!(rec
            .attention_tags
            .contains(&SubscriptionAttentionTag::Skipped));

        apply_status_update(
            &mut rec,
            WantedStatusUpdate {
                status: WantedSubscriptionStatus::Matching,
                error: None,
                skip_reason: None,
            },
            3,
            160,
        );

        assert_eq!(rec.status, WantedSubscriptionStatus::Matching);
        assert_eq!(rec.skip_reason, None);
        assert!(!rec
            .attention_tags
            .contains(&SubscriptionAttentionTag::Skipped));
        assert_eq!(rec.processing_stage.as_deref(), Some("searching"));
    }

    #[test]
    fn successful_progress_clears_historical_failed_attention() {
        let mut rec = record_from_item(&item("1", "旧片", "2023 / 日本", &["日影"]), 0, 3, 100);
        rec.status = WantedSubscriptionStatus::Failed;
        rec.last_error = Some("qB 中未找到已推送种子".to_string());
        rec.last_push = Some(TorrentPushRecord {
            subscription_id: "1".to_string(),
            torrent_id: "torrent-1".to_string(),
            torrent_title: "旧片.2023".to_string(),
            qb_server: "nas".to_string(),
            qb_server_id: "nas".to_string(),
            qb_category: "movie".to_string(),
            qb_save_dir_name: "movies".to_string(),
            qb_identifier: String::new(),
            torrent_download_url: None,
            mteam_torrent_url: None,
            pushed_at: Some(100),
            status: "failed".to_string(),
            error: Some("qB 中未找到已推送种子".to_string()),
            qb_hash: None,
            qb_name: None,
            checked_at: Some(120),
            completed_at: None,
            download_progress: Some(0.4),
            download_state: None,
            total_size: None,
            completed_file_count: None,
            total_file_count: None,
            files: Vec::new(),
            episodes: Vec::new(),
            source_path: None,
            target_dir: None,
            linked_files: Vec::new(),
        });
        apply_push_stage(&mut rec, 120);
        assert!(rec
            .attention_tags
            .contains(&SubscriptionAttentionTag::Failed));

        rec.status = WantedSubscriptionStatus::Downloading;
        rec.last_error = None;
        let push = rec.last_push.as_mut().unwrap();
        push.status = "downloading".to_string();
        push.error = None;
        push.download_progress = Some(0.5);
        apply_push_stage(&mut rec, 160);

        assert_eq!(rec.lifecycle_state, SubscriptionLifecycleState::Downloading);
        assert_eq!(rec.processing_stage.as_deref(), Some("downloading"));
        assert!(!rec
            .attention_tags
            .contains(&SubscriptionAttentionTag::Failed));
    }

    #[test]
    fn new_items_after_bootstrap_are_unprocessed() {
        let mut state = WantedSubscriptionState::new("acct", 100);
        let cfg = SubscriptionWatcherConfig::default();
        apply_wish_items_to_state(
            &mut state,
            &[item("1", "旧片", "2023 / 日本", &["日影"])],
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            "/tmp/state.json".to_string(),
            120,
        );
        let outcome = apply_wish_items_to_state(
            &mut state,
            &[
                item("1", "旧片", "2023 / 日本", &["日影"]),
                item("2", "新片", "2025 / 美国", &["新订阅"]),
            ],
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            "/tmp/state.json".to_string(),
            180,
        );

        assert!(!outcome.bootstrap_mode);
        assert_eq!(outcome.created_unprocessed, 1);
        assert_eq!(
            state.records.get("2").unwrap().status,
            WantedSubscriptionStatus::Unprocessed
        );
    }

    #[test]
    fn new_unprocessed_items_record_queue_stage() {
        let mut state = WantedSubscriptionState::new("acct", 100);
        state.bootstrap_completed = true;
        let cfg = SubscriptionWatcherConfig::default();
        apply_wish_items_to_state(
            &mut state,
            &[item("2", "新电影", "2025 / 中国大陆", &["电影"])],
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            "/tmp/state.json".to_string(),
            200,
        );

        let rec = state.records.get("2").unwrap();
        assert_eq!(rec.processing_stage.as_deref(), Some("queued"));
        assert_eq!(
            rec.stage_message.as_deref(),
            Some("已进入订阅队列，等待下一轮自动处理")
        );
        assert_eq!(rec.stage_updated_at, Some(200));
        assert_eq!(
            rec.next_action.as_deref(),
            Some("自动搜索候选种子并推送 qB")
        );
    }

    #[test]
    fn wish_items_store_douban_server_date_and_return_order() {
        let mut state = WantedSubscriptionState::new("acct", 100);
        state.bootstrap_completed = true;
        let cfg = SubscriptionWatcherConfig::default();
        apply_wish_items_to_state(
            &mut state,
            &[
                item_with_date("1", "服务器第一项", "2026-06-25"),
                item_with_date("2", "服务器第二项", "2026-06-24"),
            ],
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            "/tmp/state.json".to_string(),
            200,
        );

        let first = state.records.get("1").unwrap();
        let second = state.records.get("2").unwrap();
        assert_eq!(first.douban_date.as_deref(), Some("2026-06-25"));
        assert_eq!(first.douban_sort_time, Some(20260625));
        assert_eq!(first.douban_return_order, Some(0));
        assert_eq!(second.douban_return_order, Some(1));

        apply_wish_items_to_state(
            &mut state,
            &[item_with_date("1", "服务器第一项更新", "2026-06-26")],
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            "/tmp/state.json".to_string(),
            260,
        );
        let refreshed = state.records.get("1").unwrap();
        assert_eq!(refreshed.douban_date.as_deref(), Some("2026-06-26"));
        assert_eq!(refreshed.douban_sort_time, Some(20260626));
        assert_eq!(refreshed.douban_return_order, Some(0));
    }

    #[test]
    fn failure_policy_caps_retries() {
        let mut rec = record_from_item(&item("1", "失败片", "2026 / 中国大陆", &[]), 0, 2, 100);
        let first_exhausted = apply_status_update(
            &mut rec,
            WantedStatusUpdate {
                status: WantedSubscriptionStatus::Failed,
                error: Some("no torrent".to_string()),
                skip_reason: None,
            },
            2,
            120,
        );
        assert!(!first_exhausted);
        assert_eq!(rec.status, WantedSubscriptionStatus::Unprocessed);
        assert_eq!(rec.retry_count, 1);

        let second_exhausted = apply_status_update(
            &mut rec,
            WantedStatusUpdate {
                status: WantedSubscriptionStatus::Failed,
                error: Some("still no torrent".to_string()),
                skip_reason: None,
            },
            2,
            180,
        );
        assert!(second_exhausted);
        assert_eq!(rec.status, WantedSubscriptionStatus::Failed);
        assert_eq!(rec.retry_count, 2);
        assert_eq!(rec.last_error.as_deref(), Some("still no torrent"));
        assert_eq!(rec.processing_stage.as_deref(), Some("error"));
    }

    #[test]
    fn candidate_and_push_updates_record_actionable_stages() {
        let mut rec = record_from_item(
            &item("1", "电影一", "2024 / 中国大陆 / 剧情", &["电影"]),
            0,
            3,
            100,
        );
        rec.candidate_matches = vec![TorrentCandidateMatchRecord {
            candidate: TorrentCandidateRecord {
                torrent_id: "t1".to_string(),
                title: "电影一 720p".to_string(),
                subtitle: String::new(),
                source: "keyword".to_string(),
                search_query: "电影一".to_string(),
                size: None,
                seeders: None,
                leechers: None,
                uploaded_at: None,
            },
            selected: false,
            matched_rule_name: None,
            matched_priority: None,
            matched_keywords: Vec::new(),
            excluded_reason: Some("missing source:bluray".to_string()),
            rule_evaluations: Vec::new(),
        }];
        apply_candidate_stage(&mut rec, 120);
        assert_eq!(rec.processing_stage.as_deref(), Some("no_match"));
        assert_eq!(
            rec.stage_message.as_deref(),
            Some("候选种子未命中当前匹配规则")
        );

        rec.last_error = Some("没有候选种子匹配当前规则".to_string());
        rec.last_push = Some(TorrentPushRecord {
            subscription_id: "1".to_string(),
            torrent_id: String::new(),
            torrent_title: String::new(),
            qb_server: "nas".to_string(),
            qb_server_id: "nas".to_string(),
            qb_category: "movie".to_string(),
            qb_save_dir_name: "movies".to_string(),
            qb_identifier: String::new(),
            torrent_download_url: None,
            mteam_torrent_url: None,
            pushed_at: None,
            status: "failed".to_string(),
            error: rec.last_error.clone(),
            qb_hash: None,
            qb_name: None,
            checked_at: None,
            completed_at: None,
            download_progress: None,
            download_state: None,
            total_size: None,
            completed_file_count: None,
            total_file_count: None,
            files: Vec::new(),
            episodes: Vec::new(),
            source_path: None,
            target_dir: None,
            linked_files: Vec::new(),
        });
        apply_push_stage(&mut rec, 130);
        assert_eq!(rec.processing_stage.as_deref(), Some("no_match"));
        assert_eq!(
            rec.next_action.as_deref(),
            Some("调整匹配规则或等待新种子后重试")
        );
    }

    fn temp_state_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("tmdb_mteam_subscription_{name}_{nanos}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[tokio::test]
    async fn sqlite_store_persists_records_across_store_instances() {
        let root = temp_state_dir("sqlite");
        let cfg = SubscriptionWatcherConfig {
            bootstrap_existing_as_skipped: false,
            ..SubscriptionWatcherConfig::default()
        };

        let store = WantedSubscriptionStore::new(root.clone());
        let conn = store.connection_unlocked().unwrap();
        assert_eq!(read_schema_version(&conn).unwrap(), DB_SCHEMA_VERSION);
        init_schema(&conn).unwrap();
        assert_eq!(read_schema_version(&conn).unwrap(), DB_SCHEMA_VERSION);

        let outcome = store
            .apply_wish_items(
                "acct",
                &[item("2", "新剧", "2025 / 中国大陆 / 剧情", &["剧集"])],
                &cfg,
                200,
            )
            .await
            .unwrap();
        assert_eq!(outcome.created_unprocessed, 1);
        assert!(store.db_path().exists());

        let reopened = WantedSubscriptionStore::new(root.clone());
        let snapshot = reopened.snapshot("acct", 300).await.unwrap();
        let rec = snapshot.records.get("2").unwrap();
        assert_eq!(rec.title, "新剧");
        assert_eq!(rec.status, WantedSubscriptionStatus::Unprocessed);
        assert_eq!(rec.release_year, Some(2025));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn operation_logs_persist_filter_and_paginate() {
        let root = temp_state_dir("operation_logs");
        let store = WantedSubscriptionStore::new(root.clone());

        store
            .append_operation_log(NewOperationLogEntry {
                account_key: "acct".to_string(),
                created_at: 100,
                category: "subscription_sync".to_string(),
                action: "poll_wanted".to_string(),
                target_type: "subscription".to_string(),
                target_id: Some("sub-1".to_string()),
                target_title: Some("片一".to_string()),
                status: "success".to_string(),
                summary: "轮询想看完成".to_string(),
                error: None,
                related: json!({ "created_unprocessed": 1 }),
            })
            .await
            .unwrap();
        store
            .append_operation_log(NewOperationLogEntry {
                account_key: "acct".to_string(),
                created_at: 120,
                category: "qb_push".to_string(),
                action: "push_torrent".to_string(),
                target_type: "torrent".to_string(),
                target_id: Some("tid-2".to_string()),
                target_title: Some("片二 2160p".to_string()),
                status: "failed".to_string(),
                summary: "推送 qB 失败".to_string(),
                error: Some("qB 连接失败".to_string()),
                related: json!({ "qb_server": "nas" }),
            })
            .await
            .unwrap();

        let reopened = WantedSubscriptionStore::new(root.clone());
        let page = reopened
            .query_operation_logs(OperationLogQuery {
                page: Some(1),
                page_size: Some(1),
                ..OperationLogQuery::default()
            })
            .await
            .unwrap();
        assert_eq!(page.total, 2);
        assert!(page.has_more);
        assert_eq!(page.items[0].action, "push_torrent");

        let filtered = reopened
            .query_operation_logs(OperationLogQuery {
                category: Some("qb_push".to_string()),
                status: Some("failed".to_string()),
                q: Some("2160p".to_string()),
                page: Some(1),
                page_size: Some(20),
            })
            .await
            .unwrap();
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.items[0].error.as_deref(), Some("qB 连接失败"));
        assert_eq!(filtered.items[0].related["qb_server"], "nas");

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn sqlite_store_second_poll_adds_new_wish_as_unprocessed() {
        let root = temp_state_dir("sqlite_poll_twice");
        let cfg = SubscriptionWatcherConfig::default();
        let store = WantedSubscriptionStore::new(root.clone());

        let first = store
            .apply_wish_items(
                "acct",
                &[item("1", "旧片", "2023 / 日本", &["日影"])],
                &cfg,
                200,
            )
            .await
            .unwrap();
        assert!(first.bootstrap_mode);
        assert_eq!(first.created_skipped, 1);
        assert_eq!(first.created_unprocessed, 0);

        let second = store
            .apply_wish_items(
                "acct",
                &[
                    item("1", "旧片", "2023 / 日本", &["日影"]),
                    item("2", "新片", "2025 / 美国", &["新订阅"]),
                ],
                &cfg,
                260,
            )
            .await
            .unwrap();
        assert!(!second.bootstrap_mode);
        assert_eq!(second.created_unprocessed, 1);
        assert_eq!(second.created_skipped, 0);

        let snapshot = store.snapshot("acct", 300).await.unwrap();
        assert_eq!(
            snapshot.records.get("1").unwrap().status,
            WantedSubscriptionStatus::Skipped
        );
        assert_eq!(
            snapshot.records.get("2").unwrap().status,
            WantedSubscriptionStatus::Unprocessed
        );
        assert_eq!(snapshot.last_poll_at, Some(260));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn legacy_json_state_is_imported_into_sqlite() {
        let root = temp_state_dir("legacy");
        let legacy = r#"{
          "account_key": "acct",
          "bootstrap_completed": true,
          "records": {
            "42": {
              "subject_id": "42",
              "title": "旧状态",
              "status": "pushed",
              "last_push": {
                "subscription_id": "42",
                "torrent_id": "100",
                "torrent_title": "Old.Seed",
                "status": "pushed"
              }
            }
          }
        }"#;
        std::fs::write(root.join("wanted_acct.json"), legacy).unwrap();

        let store = WantedSubscriptionStore::new(root.clone());
        let snapshot = store.snapshot("acct", 500).await.unwrap();
        let rec = snapshot.records.get("42").unwrap();
        assert_eq!(rec.status, WantedSubscriptionStatus::Downloading);
        assert_eq!(rec.lifecycle_state, SubscriptionLifecycleState::Downloading);
        assert_eq!(rec.last_push.as_ref().unwrap().torrent_title, "Old.Seed");
        assert!(rec.created_at >= 500);
        assert!(store.db_path().exists());

        let reopened = WantedSubscriptionStore::new(root.clone());
        let snapshot = reopened.snapshot("acct", 600).await.unwrap();
        assert!(snapshot.records.contains_key("42"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn malformed_db_record_has_deterministic_fallback() {
        let root = temp_state_dir("malformed");
        let store = WantedSubscriptionStore::new(root.clone());
        let conn = store.connection_unlocked().unwrap();
        conn.execute(
            "INSERT INTO subscription_meta
                (account_key, version, bootstrap_completed, created_at, updated_at, last_poll_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["acct", STATE_VERSION as i64, 1i64, 100i64, 200i64, 150i64],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO wanted_subscription_records
                (account_key, subject_id, status, title, category_text, updated_at, record_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "acct",
                "bad",
                "failed",
                "损坏记录",
                Option::<String>::None,
                200i64,
                "{not valid json",
            ],
        )
        .unwrap();

        let snapshot = store.snapshot("acct", 300).await.unwrap();
        let rec = snapshot.records.get("bad").unwrap();
        assert_eq!(rec.title, "损坏记录");
        assert_eq!(rec.status, WantedSubscriptionStatus::Failed);
        assert_eq!(
            rec.last_error.as_deref(),
            Some("原订阅记录 JSON 损坏，已按索引字段降级恢复")
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn status_labels_cover_card_lifecycle_states() {
        let cases = [
            (WantedSubscriptionStatus::Skipped, "skipped"),
            (WantedSubscriptionStatus::Unprocessed, "unprocessed"),
            (WantedSubscriptionStatus::Matching, "matching"),
            (WantedSubscriptionStatus::Processing, "processing"),
            (WantedSubscriptionStatus::Pushed, "pushed"),
            (WantedSubscriptionStatus::Downloading, "downloading"),
            (WantedSubscriptionStatus::Completed, "completed"),
            (WantedSubscriptionStatus::Linked, "linked"),
            (WantedSubscriptionStatus::Failed, "failed"),
        ];
        for (status, label) in cases {
            assert_eq!(status_label(status), label);
            assert_eq!(status_from_label(label), status);
        }
    }
}
