use serde::Serialize;

use crate::subscription::repository::payload;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(super) struct DownloadArtifactDto {
    id: String,
    torrent_id: String,
    torrent_title: String,
    qb_server_id: String,
    qb_server_name: Option<String>,
    qb_category: String,
    qb_save_dir_name: String,
    qb_hash: Option<String>,
    qb_name: Option<String>,
    qb_state: Option<String>,
    state: &'static str,
    progress: Option<f64>,
    total_size: Option<u64>,
    files: Vec<DownloadFileDto>,
    pushed_at: Option<u64>,
    checked_at: Option<u64>,
    completed_at: Option<u64>,
}

impl From<&payload::DownloadArtifactPayload> for DownloadArtifactDto {
    fn from(value: &payload::DownloadArtifactPayload) -> Self {
        Self {
            id: value.idempotency_key.clone(),
            torrent_id: value.torrent_id.clone(),
            torrent_title: value.torrent_title.clone(),
            qb_server_id: value.qb_server_id.clone(),
            qb_server_name: value.qb_server_name.clone(),
            qb_category: value.qb_category.clone(),
            qb_save_dir_name: value.qb_save_dir_name.clone(),
            qb_hash: value.qb_hash.clone(),
            qb_name: value.qb_name.clone(),
            qb_state: value.qb_state.clone(),
            state: download_state_label(value.state),
            progress: value.progress,
            total_size: value.total_size,
            files: value.files.iter().map(DownloadFileDto::from).collect(),
            pushed_at: value.pushed_at,
            checked_at: value.checked_at,
            completed_at: value.completed_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct DownloadFileDto {
    name: String,
    size: u64,
    progress: f64,
    season_number: Option<u32>,
    episode_number: Option<u32>,
    episode_end_number: Option<u32>,
    episode_label: Option<String>,
}

impl From<&payload::DownloadFilePayload> for DownloadFileDto {
    fn from(value: &payload::DownloadFilePayload) -> Self {
        Self {
            name: value.name.clone(),
            size: value.size,
            progress: value.progress,
            season_number: value.season_number,
            episode_number: value.episode_number,
            episode_end_number: value.episode_end_number,
            episode_label: value.episode_label.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct LinkArtifactDto {
    id: String,
    download_artifact_id: String,
    state: &'static str,
    source_path: Option<String>,
    target_dir: Option<String>,
    checked_at: u64,
    completed_at: Option<u64>,
    files: Vec<LinkFileDto>,
}

impl LinkArtifactDto {
    pub(super) fn from_payload(
        value: &payload::LinkArtifactPayload,
        redact_diagnostic: &dyn Fn(&str) -> String,
    ) -> Self {
        Self {
            id: value.idempotency_key.clone(),
            download_artifact_id: value.download.artifact_id.clone(),
            state: link_state_label(value.state),
            source_path: value.source_path.clone(),
            target_dir: value.target_dir.clone(),
            checked_at: value.checked_at,
            completed_at: value.completed_at,
            files: value
                .files
                .iter()
                .map(|file| LinkFileDto::from_payload(file, redact_diagnostic))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct LinkFileDto {
    source_path: String,
    target_path: String,
    size: u64,
    outcome: &'static str,
    season_number: Option<u32>,
    episode_number: Option<u32>,
    episode_end_number: Option<u32>,
    episode_label: Option<String>,
    error: Option<String>,
}

impl LinkFileDto {
    fn from_payload(
        value: &payload::LinkFilePayload,
        redact_diagnostic: &dyn Fn(&str) -> String,
    ) -> Self {
        Self {
            source_path: value.source_path.clone(),
            target_path: value.target_path.clone(),
            size: value.size,
            outcome: link_file_outcome_label(value.outcome),
            season_number: value.season_number,
            episode_number: value.episode_number,
            episode_end_number: value.episode_end_number,
            episode_label: value.episode_label.clone(),
            error: value.error.as_deref().map(redact_diagnostic),
        }
    }
}

const fn download_state_label(value: payload::DownloadArtifactStatePayload) -> &'static str {
    match value {
        payload::DownloadArtifactStatePayload::Pushed => "pushed",
        payload::DownloadArtifactStatePayload::Downloading => "downloading",
        payload::DownloadArtifactStatePayload::Downloaded => "downloaded",
        payload::DownloadArtifactStatePayload::Missing => "missing",
        payload::DownloadArtifactStatePayload::Failed => "failed",
        payload::DownloadArtifactStatePayload::Ignored => "ignored",
        payload::DownloadArtifactStatePayload::Superseded => "superseded",
    }
}

const fn link_state_label(value: payload::LinkArtifactStatePayload) -> &'static str {
    match value {
        payload::LinkArtifactStatePayload::Planned => "planned",
        payload::LinkArtifactStatePayload::Partial => "partial",
        payload::LinkArtifactStatePayload::Completed => "completed",
        payload::LinkArtifactStatePayload::Failed => "failed",
    }
}

const fn link_file_outcome_label(value: payload::LinkFileOutcomePayload) -> &'static str {
    match value {
        payload::LinkFileOutcomePayload::Pending => "pending",
        payload::LinkFileOutcomePayload::Linked => "linked",
        payload::LinkFileOutcomePayload::Failed => "failed",
        payload::LinkFileOutcomePayload::Missing => "missing",
        payload::LinkFileOutcomePayload::Conflict => "conflict",
    }
}
