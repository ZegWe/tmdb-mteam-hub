use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use limbo::{Builder, Connection, Row, Value};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::config::SubscriptionWatcherConfig;
use crate::douban::DoubanLibraryItem;

const STATE_VERSION: u32 = 1;
const DB_SCHEMA_VERSION: i64 = 1;
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
    pub qb_category: String,
    #[serde(default)]
    pub qb_save_dir_name: String,
    #[serde(default)]
    pub qb_identifier: String,
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
    #[serde(default)]
    pub category_text: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub status: WantedSubscriptionStatus,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
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
            self.db_path.display().to_string(),
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

    async fn connection_unlocked(&self) -> std::io::Result<Connection> {
        tokio::fs::create_dir_all(&self.root).await?;
        let db_path = self.db_path.to_string_lossy().to_string();
        let db = Builder::new_local(&db_path)
            .build()
            .await
            .map_err(limbo_io)?;
        let conn = db.connect().map_err(limbo_io)?;
        init_schema(&conn).await?;
        Ok(conn)
    }

    async fn load_state_unlocked(
        &self,
        account_key: &str,
        now: u64,
    ) -> std::io::Result<WantedSubscriptionState> {
        let conn = self.connection_unlocked().await?;
        if let Some(mut state) = load_state_from_db(&conn, account_key).await? {
            repair_state_defaults(&mut state, account_key, now);
            return Ok(state);
        }

        if let Some(mut state) = self.load_legacy_json_state(account_key).await? {
            repair_state_defaults(&mut state, account_key, now);
            save_state_to_db(&conn, account_key, &state).await?;
            return Ok(state);
        }

        Ok(WantedSubscriptionState::new(account_key, now))
    }

    async fn save_state_unlocked(
        &self,
        account_key: &str,
        state: &WantedSubscriptionState,
    ) -> std::io::Result<()> {
        let conn = self.connection_unlocked().await?;
        save_state_to_db(&conn, account_key, state).await
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

async fn init_schema(conn: &Connection) -> std::io::Result<()> {
    conn.execute(
        "CREATE TABLE IF NOT EXISTS subscription_schema_meta (
            key TEXT NOT NULL,
            value INTEGER NOT NULL
        )",
        (),
    )
    .await
    .map_err(limbo_io)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS subscription_meta (
            account_key TEXT NOT NULL,
            version INTEGER NOT NULL,
            bootstrap_completed INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            last_poll_at INTEGER
        )",
        (),
    )
    .await
    .map_err(limbo_io)?;
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
        (),
    )
    .await
    .map_err(limbo_io)?;
    ensure_schema_version(conn).await?;
    Ok(())
}

async fn ensure_schema_version(conn: &Connection) -> std::io::Result<()> {
    let current = read_schema_version(conn).await?;
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
        (),
    )
    .await
    .map_err(limbo_io)?;
    conn.execute(
        "INSERT INTO subscription_schema_meta (key, value) VALUES ('schema_version', ?1)",
        [DB_SCHEMA_VERSION],
    )
    .await
    .map_err(limbo_io)?;
    Ok(())
}

async fn read_schema_version(conn: &Connection) -> std::io::Result<i64> {
    let mut rows = conn
        .query(
            "SELECT value FROM subscription_schema_meta WHERE key = 'schema_version'",
            (),
        )
        .await
        .map_err(limbo_io)?;
    match rows.next().await.map_err(limbo_io)? {
        Some(row) => row_i64(&row, 0),
        None => Ok(0),
    }
}

async fn load_state_from_db(
    conn: &Connection,
    account_key: &str,
) -> std::io::Result<Option<WantedSubscriptionState>> {
    let mut meta_rows = conn
        .query(
            "SELECT version, bootstrap_completed, created_at, updated_at, last_poll_at
                FROM subscription_meta WHERE account_key = ?1",
            [account_key],
        )
        .await
        .map_err(limbo_io)?;
    let Some(meta) = meta_rows.next().await.map_err(limbo_io)? else {
        return Ok(None);
    };

    let mut state = WantedSubscriptionState {
        version: row_i64(&meta, 0)? as u32,
        account_key: account_key.to_string(),
        bootstrap_completed: row_i64(&meta, 1)? != 0,
        created_at: i64_to_u64(row_i64(&meta, 2)?),
        updated_at: i64_to_u64(row_i64(&meta, 3)?),
        last_poll_at: row_opt_i64(&meta, 4)?.map(i64_to_u64),
        records: BTreeMap::new(),
    };

    let mut record_rows = conn
        .query(
            "SELECT subject_id, status, title, category_text, updated_at, record_json FROM wanted_subscription_records
                WHERE account_key = ?1 ORDER BY subject_id",
            [account_key],
        )
        .await
        .map_err(limbo_io)?;
    while let Some(row) = record_rows.next().await.map_err(limbo_io)? {
        let mut record = parse_record_row(&row)?;
        repair_record_defaults(&mut record, state.created_at, state.updated_at, 0);
        state.records.insert(record.subject_id.clone(), record);
    }

    Ok(Some(state))
}

fn parse_record_row(row: &Row) -> std::io::Result<WantedSubscriptionRecord> {
    let subject_id = row_text(row, 0)?;
    let status = row_text(row, 1)?;
    let title = row_text(row, 2)?;
    let category_text = row_optional_text(row, 3)?;
    let updated_at = row_opt_i64(row, 4)?.map(i64_to_u64).unwrap_or_default();
    let raw = row_text(row, 5)?;
    let mut record: WantedSubscriptionRecord =
        serde_json::from_str(&raw).unwrap_or_else(|_| WantedSubscriptionRecord {
            subject_id: subject_id.clone(),
            title: title.clone(),
            release_year: None,
            category_text: category_text.clone(),
            tags: Vec::new(),
            status: status_from_label(&status),
            retry_count: 0,
            max_retries: 0,
            last_error: Some("原订阅记录 JSON 损坏，已按索引字段降级恢复".to_string()),
            skip_reason: None,
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
    Ok(record)
}

async fn save_state_to_db(
    conn: &Connection,
    account_key: &str,
    state: &WantedSubscriptionState,
) -> std::io::Result<()> {
    conn.execute("BEGIN IMMEDIATE", ())
        .await
        .map_err(limbo_io)?;
    let result = save_state_to_db_inner(conn, account_key, state).await;
    match result {
        Ok(()) => {
            conn.execute("COMMIT", ()).await.map_err(limbo_io)?;
            Ok(())
        }
        Err(e) => {
            let _ = conn.execute("ROLLBACK", ()).await;
            Err(e)
        }
    }
}

async fn save_state_to_db_inner(
    conn: &Connection,
    account_key: &str,
    state: &WantedSubscriptionState,
) -> std::io::Result<()> {
    conn.execute(
        "DELETE FROM subscription_meta WHERE account_key = ?1",
        [account_key],
    )
    .await
    .map_err(limbo_io)?;
    conn.execute(
        "INSERT INTO subscription_meta
            (account_key, version, bootstrap_completed, created_at, updated_at, last_poll_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (
            account_key,
            state.version as i64,
            i64::from(state.bootstrap_completed),
            u64_to_i64(state.created_at),
            u64_to_i64(state.updated_at),
            opt_u64_value(state.last_poll_at),
        ),
    )
    .await
    .map_err(limbo_io)?;

    conn.execute(
        "DELETE FROM wanted_subscription_records WHERE account_key = ?1",
        [account_key],
    )
    .await
    .map_err(limbo_io)?;

    for record in state.records.values() {
        let record_json = serde_json::to_string(record)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        conn.execute(
            "INSERT INTO wanted_subscription_records
                (account_key, subject_id, status, title, category_text, updated_at, record_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                account_key,
                record.subject_id.as_str(),
                status_label(record.status),
                record.title.as_str(),
                opt_str_value(record.category_text.as_deref()),
                u64_to_i64(record.updated_at),
                record_json,
            ),
        )
        .await
        .map_err(limbo_io)?;
    }

    Ok(())
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

fn opt_u64_value(value: Option<u64>) -> Value {
    value
        .map(|value| Value::Integer(u64_to_i64(value)))
        .unwrap_or(Value::Null)
}

fn opt_str_value(value: Option<&str>) -> Value {
    value
        .map(|value| Value::Text(value.to_string()))
        .unwrap_or(Value::Null)
}

fn row_text(row: &Row, index: usize) -> std::io::Result<String> {
    match row.get_value(index).map_err(limbo_io)? {
        Value::Text(value) => Ok(value),
        Value::Integer(value) => Ok(value.to_string()),
        Value::Real(value) => Ok(value.to_string()),
        Value::Null => Ok(String::new()),
        Value::Blob(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "SQLite 字段是 blob，无法作为文本读取",
        )),
    }
}

fn row_optional_text(row: &Row, index: usize) -> std::io::Result<Option<String>> {
    match row.get_value(index).map_err(limbo_io)? {
        Value::Null => Ok(None),
        Value::Text(value) => {
            let trimmed = value.trim();
            Ok((!trimmed.is_empty()).then(|| trimmed.to_string()))
        }
        Value::Integer(value) => Ok(Some(value.to_string())),
        Value::Real(value) => Ok(Some(value.to_string())),
        Value::Blob(_) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "SQLite 字段是 blob，无法作为文本读取",
        )),
    }
}

fn row_i64(row: &Row, index: usize) -> std::io::Result<i64> {
    match row.get_value(index).map_err(limbo_io)? {
        Value::Integer(value) => Ok(value),
        Value::Text(value) => value.parse().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("SQLite 整数字段解析失败: {e}"),
            )
        }),
        value => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("SQLite 字段不是整数: {value:?}"),
        )),
    }
}

fn row_opt_i64(row: &Row, index: usize) -> std::io::Result<Option<i64>> {
    match row.get_value(index).map_err(limbo_io)? {
        Value::Null => Ok(None),
        Value::Integer(value) => Ok(Some(value)),
        Value::Text(value) if value.trim().is_empty() => Ok(None),
        Value::Text(value) => value.parse().map(Some).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("SQLite 可空整数字段解析失败: {e}"),
            )
        }),
        value => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("SQLite 字段不是可空整数: {value:?}"),
        )),
    }
}

fn u64_to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

fn limbo_io(error: limbo::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, error.to_string())
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
                WantedSubscriptionStatus::Matching
                    | WantedSubscriptionStatus::Processing
                    | WantedSubscriptionStatus::Pushed
                    | WantedSubscriptionStatus::Downloading
                    | WantedSubscriptionStatus::Completed
                    | WantedSubscriptionStatus::Linked
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
        let conn = store.connection_unlocked().await.unwrap();
        assert_eq!(read_schema_version(&conn).await.unwrap(), DB_SCHEMA_VERSION);
        init_schema(&conn).await.unwrap();
        assert_eq!(read_schema_version(&conn).await.unwrap(), DB_SCHEMA_VERSION);

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
        assert_eq!(rec.status, WantedSubscriptionStatus::Pushed);
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
        let conn = store.connection_unlocked().await.unwrap();
        conn.execute(
            "INSERT INTO subscription_meta
                (account_key, version, bootstrap_completed, created_at, updated_at, last_poll_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            ("acct", STATE_VERSION as i64, 1i64, 100i64, 200i64, 150i64),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO wanted_subscription_records
                (account_key, subject_id, status, title, category_text, updated_at, record_json)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "acct",
                "bad",
                "failed",
                "损坏记录",
                Value::Null,
                200i64,
                "{not valid json",
            ),
        )
        .await
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
