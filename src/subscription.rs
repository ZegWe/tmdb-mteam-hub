use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::SubscriptionWatcherConfig;
use crate::douban::DoubanLibraryItem;

const STATE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WantedSubscriptionStatus {
    Unprocessed,
    Processing,
    Pushed,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TorrentCandidateRecord {
    pub torrent_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subtitle: String,
    pub source: String,
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
    pub subscription_id: String,
    pub torrent_id: String,
    pub torrent_title: String,
    pub qb_server: String,
    pub qb_category: String,
    pub qb_save_dir_name: String,
    pub qb_identifier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<u64>,
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
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_files: Vec<HardlinkFileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardlinkFileRecord {
    pub source_path: String,
    pub target_path: String,
    pub size: u64,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardlinkCompletionRecord {
    pub status: String,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WantedSubscriptionRecord {
    pub subject_id: String,
    pub title: String,
    pub release_year: Option<u16>,
    pub category_text: Option<String>,
    pub tags: Vec<String>,
    pub status: WantedSubscriptionStatus,
    pub retry_count: u32,
    pub max_retries: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_matches: Vec<TorrentCandidateMatchRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_push: Option<TorrentPushRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_completion: Option<HardlinkCompletionRecord>,
    pub created_at: u64,
    pub updated_at: u64,
    pub first_seen_at: u64,
    pub last_seen_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WantedSubscriptionState {
    pub version: u32,
    pub account_key: String,
    pub bootstrap_completed: bool,
    pub created_at: u64,
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

#[derive(Clone)]
pub struct WantedSubscriptionStore {
    root: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl WantedSubscriptionStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn snapshot(
        &self,
        account_key: &str,
        now: u64,
    ) -> std::io::Result<WantedSubscriptionState> {
        let _guard = self.lock.lock().await;
        self.load_state_unlocked(account_key, now).await
    }

    pub async fn apply_wish_items(
        &self,
        account_key: &str,
        items: &[DoubanLibraryItem],
        cfg: &SubscriptionWatcherConfig,
        now: u64,
    ) -> std::io::Result<WantedPollOutcome> {
        let _guard = self.lock.lock().await;
        let mut state = self.load_state_unlocked(account_key, now).await?;
        let outcome = apply_wish_items_to_state(
            &mut state,
            items,
            cfg.bootstrap_existing_as_skipped,
            cfg.max_retries,
            self.path_for(account_key).display().to_string(),
            now,
        );
        self.save_state_unlocked(account_key, &state).await?;
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
        self.save_state_unlocked(account_key, &state).await?;
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
        state.updated_at = now;
        let record = record.clone();
        self.save_state_unlocked(account_key, &state).await?;
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
        state.updated_at = now;
        let record = record.clone();
        self.save_state_unlocked(account_key, &state).await?;
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
        state.updated_at = now;
        let record = record.clone();
        self.save_state_unlocked(account_key, &state).await?;
        Ok(Some(record))
    }

    fn path_for(&self, account_key: &str) -> PathBuf {
        self.root
            .join(format!("wanted_{}.json", safe_account_key(account_key)))
    }

    async fn load_state_unlocked(
        &self,
        account_key: &str,
        now: u64,
    ) -> std::io::Result<WantedSubscriptionState> {
        let path = self.path_for(account_key);
        match tokio::fs::read_to_string(&path).await {
            Ok(raw) => {
                let mut state: WantedSubscriptionState =
                    serde_json::from_str(&raw).map_err(|e| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("解析订阅状态文件失败 {}: {e}", path.display()),
                        )
                    })?;
                if state.account_key.trim().is_empty() {
                    state.account_key = account_key.to_string();
                }
                Ok(state)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(WantedSubscriptionState::new(account_key, now))
            }
            Err(e) => Err(e),
        }
    }

    async fn save_state_unlocked(
        &self,
        account_key: &str,
        state: &WantedSubscriptionState,
    ) -> std::io::Result<()> {
        tokio::fs::create_dir_all(&self.root).await?;
        let path = self.path_for(account_key);
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_vec_pretty(state)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        tokio::fs::write(&tmp, body).await?;
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }
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

fn apply_wish_items_to_state(
    state: &mut WantedSubscriptionState,
    items: &[DoubanLibraryItem],
    bootstrap_existing_as_skipped: bool,
    max_retries: u32,
    state_path: String,
    now: u64,
) -> WantedPollOutcome {
    let bootstrap_mode = !state.bootstrap_completed;
    let mut created_unprocessed = 0usize;
    let mut created_skipped = 0usize;
    let mut updated_existing = 0usize;

    for item in items {
        let subject_id = item.subject_id.trim();
        if subject_id.is_empty() {
            continue;
        }
        if let Some(existing) = state.records.get_mut(subject_id) {
            refresh_record_from_item(existing, item, now);
            updated_existing += 1;
            continue;
        }

        let mut record = record_from_item(item, max_retries, now);
        if bootstrap_mode && bootstrap_existing_as_skipped {
            record.status = WantedSubscriptionStatus::Skipped;
            record.skip_reason = Some("initial_bootstrap_existing_wish".to_string());
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

fn record_from_item(
    item: &DoubanLibraryItem,
    max_retries: u32,
    now: u64,
) -> WantedSubscriptionRecord {
    let tags = normalized_tags(&item.tags);
    WantedSubscriptionRecord {
        subject_id: item.subject_id.trim().to_string(),
        title: item.title.trim().to_string(),
        release_year: release_year_from_item(item),
        category_text: tags.first().cloned(),
        tags,
        status: WantedSubscriptionStatus::Unprocessed,
        retry_count: 0,
        max_retries,
        last_error: None,
        skip_reason: None,
        candidate_matches: Vec::new(),
        last_push: None,
        last_completion: None,
        created_at: now,
        updated_at: now,
        first_seen_at: now,
        last_seen_at: now,
    }
}

fn refresh_record_from_item(
    record: &mut WantedSubscriptionRecord,
    item: &DoubanLibraryItem,
    now: u64,
) {
    let tags = normalized_tags(&item.tags);
    record.title = item.title.trim().to_string();
    record.release_year = release_year_from_item(item).or(record.release_year);
    record.category_text = tags
        .first()
        .cloned()
        .or_else(|| record.category_text.clone());
    record.tags = tags;
    record.last_seen_at = now;
    record.updated_at = now;
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
            exhausted
        }
        WantedSubscriptionStatus::Skipped => {
            record.status = WantedSubscriptionStatus::Skipped;
            record.skip_reason = update.skip_reason.filter(|s| !s.trim().is_empty());
            false
        }
        status => {
            record.status = status;
            if matches!(
                status,
                WantedSubscriptionStatus::Processing
                    | WantedSubscriptionStatus::Pushed
                    | WantedSubscriptionStatus::Completed
            ) {
                record.last_error = None;
            }
            false
        }
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
    fn failure_policy_caps_retries() {
        let mut rec = record_from_item(&item("1", "失败片", "2026 / 中国大陆", &[]), 2, 100);
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
    }
}
