use std::collections::BTreeSet;
use std::fmt;

use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};

use crate::subscription::effects::stable_qb_idempotency_key;
pub(crate) use crate::subscription::effects::LinkFileOutcome as LinkFileOutcomePayload;

use super::{validated_text, RepositoryError, RepositoryResult};

pub(crate) const LINK_ARTIFACT_KEY_PREFIX: &str = "link:v1:";

/// Upstream-owned metadata refreshed by a wanted-list snapshot.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct WantedSourcePayload {
    pub(crate) title: String,
    pub(crate) release_year: Option<u16>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) poster_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) cover_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) original_title: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) aka: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) languages: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) countries: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) genres: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) directors: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) actors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) date_published: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rating_value: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) rating_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) category_text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) douban_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) douban_sort_time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) douban_return_order: Option<u32>,
}

impl WantedSourcePayload {
    pub(crate) fn validate(&self) -> RepositoryResult<()> {
        validated_text("source.title", self.title.clone())?;
        if self.rating_value.is_some_and(|rating| !rating.is_finite()) {
            return Err(RepositoryError::invalid(
                "source.rating_value",
                "rating must be finite",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ObservationPayload {
    pub(crate) created_at: u64,
    pub(crate) first_seen_at: u64,
    pub(crate) last_seen_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ArtifactKindPayload {
    Download,
    Link,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TvLanePayload {
    Search,
    Progress,
    Link,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum IssueOwnerPayload {
    Parent,
    Artifact {
        artifact_kind: ArtifactKindPayload,
        artifact_id: String,
    },
    TvLane {
        lane: TvLanePayload,
    },
    TvEpisode {
        season_number: u32,
        episode_number: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct IssuePayload {
    pub(crate) owner: IssueOwnerPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) operation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error_type: Option<String>,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) occurred_at: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CandidatePayload {
    pub(crate) torrent_id: String,
    pub(crate) title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) subtitle: String,
    pub(crate) source: String,
    pub(crate) search_query: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) seeders: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) leechers: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) uploaded_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CandidateRuleEvaluationPayload {
    pub(crate) rule_name: String,
    pub(crate) priority: i32,
    pub(crate) mode: String,
    pub(crate) matched: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) matched_keywords: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) missing_keywords: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) excluded_reason: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CandidateMatchPayload {
    pub(crate) candidate: CandidatePayload,
    pub(crate) selected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) matched_rule_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) matched_priority: Option<i32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) matched_keywords: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) excluded_reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) rule_evaluations: Vec<CandidateRuleEvaluationPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TvEpisodeIntentPayload {
    Target,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TvEpisodeDetailPayload {
    pub(crate) season_number: u32,
    pub(crate) episode_number: u32,
    pub(crate) label: String,
    pub(crate) intent: TvEpisodeIntentPayload,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct TvDetailPayload {
    pub(crate) season_number: u32,
    pub(crate) episode_total: u32,
    pub(crate) target_start_episode: u32,
    pub(crate) target_end_episode: u32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) episodes: Vec<TvEpisodeDetailPayload>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DownloadFilePayload {
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) progress: f64,
    pub(crate) priority: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) season_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) episode_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) episode_end_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) episode_label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DownloadArtifactStatePayload {
    Pushed,
    Downloading,
    Downloaded,
    Missing,
    Failed,
    Ignored,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DownloadArtifactPayload {
    pub(crate) idempotency_key: String,
    pub(crate) torrent_id: String,
    pub(crate) torrent_title: String,
    pub(crate) qb_server_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qb_server_name: Option<String>,
    pub(crate) qb_category: String,
    pub(crate) qb_save_dir_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qb_identifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qb_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qb_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) qb_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) torrent_download_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) mteam_torrent_url: Option<String>,
    pub(crate) state: DownloadArtifactStatePayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) progress: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) files: Vec<DownloadFilePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) pushed_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) checked_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkFilePayload {
    pub(crate) source_path: String,
    pub(crate) target_path: String,
    pub(crate) size: u64,
    pub(crate) outcome: LinkFileOutcomePayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) season_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) episode_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) episode_end_number: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) episode_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LinkArtifactStatePayload {
    Planned,
    Partial,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkDownloadRefPayload {
    pub(crate) artifact_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LinkArtifactPayload {
    pub(crate) idempotency_key: String,
    pub(crate) download: LinkDownloadRefPayload,
    pub(crate) state: LinkArtifactStatePayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target_dir: Option<String>,
    pub(crate) checked_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) completed_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) files: Vec<LinkFilePayload>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct ArtifactPayload {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) downloads: Vec<DownloadArtifactPayload>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) links: Vec<LinkArtifactPayload>,
}

/// JSON-owned, non-control detail for one schema-v5 subscription row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct SubscriptionPayload {
    pub(crate) source: WantedSourcePayload,
    pub(crate) observation: ObservationPayload,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) issues: Vec<IssuePayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) skip_reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) candidates: Vec<CandidateMatchPayload>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tv: Option<TvDetailPayload>,
    pub(crate) artifacts: ArtifactPayload,
}

impl SubscriptionPayload {
    pub(crate) fn validate(&self) -> RepositoryResult<()> {
        self.source.validate()?;
        if self.observation.created_at == 0
            || self.observation.first_seen_at == 0
            || self.observation.last_seen_at == 0
            || self.observation.created_at > self.observation.first_seen_at
            || self.observation.first_seen_at > self.observation.last_seen_at
        {
            return Err(RepositoryError::invalid(
                "payload.observation",
                "observation timestamps must be non-zero and ordered created <= first_seen <= last_seen",
            ));
        }
        if let Some(reason) = &self.skip_reason {
            validated_text("payload.skip_reason", reason.clone())?;
        }
        for candidate in &self.candidates {
            validated_text(
                "payload.candidates.candidate.torrent_id",
                candidate.candidate.torrent_id.clone(),
            )?;
            validated_text(
                "payload.candidates.candidate.title",
                candidate.candidate.title.clone(),
            )?;
        }
        if let Some(tv) = &self.tv {
            validate_tv_detail(tv)?;
        }
        let (download_ids, link_ids) = validate_artifacts(&self.artifacts)?;
        validate_issues(&self.issues, self.tv.as_ref(), &download_ids, &link_ids)
    }

    pub(crate) fn validate_for(&self, account_key: &str, subject_id: &str) -> RepositoryResult<()> {
        validated_text("account_key", account_key.to_string())?;
        validated_text("subject_id", subject_id.to_string())?;
        self.validate()?;

        for download in &self.artifacts.downloads {
            let expected =
                stable_download_artifact_key(account_key, subject_id, &download.torrent_id);
            if download.idempotency_key != expected {
                return Err(RepositoryError::invalid(
                    "payload.artifacts.downloads.idempotency_key",
                    "download artifact idempotency key does not match its stable identity",
                ));
            }
        }
        for link in &self.artifacts.links {
            let expected = stable_resolved_link_artifact_key(
                account_key,
                subject_id,
                &link.download.artifact_id,
            );
            if link.idempotency_key != expected {
                return Err(RepositoryError::invalid(
                    "payload.artifacts.links.idempotency_key",
                    "link artifact idempotency key does not match its subscription identity",
                ));
            }
        }
        Ok(())
    }
}

fn validate_tv_detail(tv: &TvDetailPayload) -> RepositoryResult<()> {
    if tv.season_number == 0
        || tv.episode_total == 0
        || tv.target_start_episode == 0
        || tv.target_end_episode < tv.target_start_episode
        || tv.target_end_episode > tv.episode_total
    {
        return Err(RepositoryError::invalid(
            "payload.tv",
            "TV season and target range must be positive and ordered",
        ));
    }
    let mut episode_ids = BTreeSet::new();
    for episode in &tv.episodes {
        if episode.season_number != tv.season_number
            || episode.episode_number == 0
            || episode.episode_number > tv.episode_total
        {
            return Err(RepositoryError::invalid(
                "payload.tv.episodes",
                "TV episode identity must use the detail season and remain within episode_total",
            ));
        }
        if !episode_ids.insert((episode.season_number, episode.episode_number)) {
            return Err(RepositoryError::invalid(
                "payload.tv.episodes",
                "TV episode identities must be unique",
            ));
        }
        validated_text("payload.tv.episodes.label", episode.label.clone())?;
    }
    Ok(())
}

fn validate_artifacts(
    artifacts: &ArtifactPayload,
) -> RepositoryResult<(BTreeSet<&str>, BTreeSet<&str>)> {
    let mut download_ids = BTreeSet::new();
    for download in &artifacts.downloads {
        validated_text(
            "payload.artifacts.downloads.idempotency_key",
            download.idempotency_key.clone(),
        )?;
        validated_text(
            "payload.artifacts.downloads.torrent_id",
            download.torrent_id.clone(),
        )?;
        validated_text(
            "payload.artifacts.downloads.qb_server_id",
            download.qb_server_id.clone(),
        )?;
        validated_text(
            "payload.artifacts.downloads.qb_category",
            download.qb_category.clone(),
        )?;
        validated_text(
            "payload.artifacts.downloads.qb_save_dir_name",
            download.qb_save_dir_name.clone(),
        )?;
        if !download_ids.insert(download.idempotency_key.as_str()) {
            return Err(RepositoryError::invalid(
                "payload.artifacts.downloads",
                "download artifact idempotency keys must be unique",
            ));
        }
        validate_progress("payload.artifacts.downloads.progress", download.progress)?;
        for file in &download.files {
            validated_text("payload.artifacts.downloads.files.name", file.name.clone())?;
            validate_progress(
                "payload.artifacts.downloads.files.progress",
                Some(file.progress),
            )?;
            validate_file_episode_range(
                "payload.artifacts.downloads.files",
                file.season_number,
                file.episode_number,
                file.episode_end_number,
                file.episode_label.as_deref(),
            )?;
        }
    }
    let mut link_ids = BTreeSet::new();
    for link in &artifacts.links {
        validated_text(
            "payload.artifacts.links.idempotency_key",
            link.idempotency_key.clone(),
        )?;
        if !link_ids.insert(link.idempotency_key.as_str()) {
            return Err(RepositoryError::invalid(
                "payload.artifacts.links",
                "link artifact idempotency keys must be unique",
            ));
        }
        validated_text(
            "payload.artifacts.links.download.artifact_id",
            link.download.artifact_id.clone(),
        )?;
        if !download_ids.contains(link.download.artifact_id.as_str()) {
            return Err(RepositoryError::invalid(
                "payload.artifacts.links.download.artifact_id",
                "link artifact must reference an existing download artifact",
            ));
        }
        for file in &link.files {
            validated_text(
                "payload.artifacts.links.files.source_path",
                file.source_path.clone(),
            )?;
            validated_text(
                "payload.artifacts.links.files.target_path",
                file.target_path.clone(),
            )?;
            validate_file_episode_range(
                "payload.artifacts.links.files",
                file.season_number,
                file.episode_number,
                file.episode_end_number,
                file.episode_label.as_deref(),
            )?;
        }
        if !link.files.is_empty() {
            let expected = derive_link_state(&link.files);
            if link.state != expected {
                return Err(RepositoryError::invalid(
                    "payload.artifacts.links.state",
                    "link artifact state must be derived from its per-file outcomes",
                ));
            }
        }
    }
    Ok((download_ids, link_ids))
}

fn validate_issues(
    issues: &[IssuePayload],
    tv: Option<&TvDetailPayload>,
    download_ids: &BTreeSet<&str>,
    link_ids: &BTreeSet<&str>,
) -> RepositoryResult<()> {
    for issue in issues {
        validated_text("payload.issues.message", issue.message.clone())?;
        if let Some(operation) = &issue.operation {
            validated_text("payload.issues.operation", operation.clone())?;
        }
        if let Some(error_type) = &issue.error_type {
            validated_text("payload.issues.error_type", error_type.clone())?;
        }
        match &issue.owner {
            IssueOwnerPayload::Parent => {}
            IssueOwnerPayload::Artifact {
                artifact_kind,
                artifact_id,
            } => {
                let ids = match artifact_kind {
                    ArtifactKindPayload::Download => download_ids,
                    ArtifactKindPayload::Link => link_ids,
                };
                if !ids.contains(artifact_id.as_str()) {
                    return Err(RepositoryError::invalid(
                        "payload.issues.owner.artifact_id",
                        "artifact issue must reference an artifact in the same payload",
                    ));
                }
            }
            IssueOwnerPayload::TvLane { .. } => {
                if tv.is_none() {
                    return Err(RepositoryError::invalid(
                        "payload.issues.owner",
                        "TV lane issue requires TV detail",
                    ));
                }
            }
            IssueOwnerPayload::TvEpisode {
                season_number,
                episode_number,
            } => {
                let episode_exists = tv.is_some_and(|tv| {
                    tv.episodes.iter().any(|episode| {
                        episode.season_number == *season_number
                            && episode.episode_number == *episode_number
                    })
                });
                if !episode_exists {
                    return Err(RepositoryError::invalid(
                        "payload.issues.owner",
                        "TV episode issue must reference an episode in the same payload",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_file_episode_range(
    field: &'static str,
    season_number: Option<u32>,
    episode_number: Option<u32>,
    episode_end_number: Option<u32>,
    episode_label: Option<&str>,
) -> RepositoryResult<()> {
    if season_number == Some(0)
        || episode_number == Some(0)
        || episode_end_number == Some(0)
        || episode_end_number.is_some() && episode_number.is_none()
        || episode_number
            .zip(episode_end_number)
            .is_some_and(|(start, end)| end < start)
    {
        return Err(RepositoryError::invalid(
            field,
            "file episode identity must be positive and its end must not precede its start",
        ));
    }
    if let Some(label) = episode_label {
        validated_text(field, label.to_string())?;
    }
    Ok(())
}

fn derive_link_state(files: &[LinkFilePayload]) -> LinkArtifactStatePayload {
    let linked = files
        .iter()
        .filter(|file| file.outcome == LinkFileOutcomePayload::Linked)
        .count();
    let failed = files.iter().any(|file| {
        matches!(
            file.outcome,
            LinkFileOutcomePayload::Failed
                | LinkFileOutcomePayload::Missing
                | LinkFileOutcomePayload::Conflict
        )
    });
    if linked == files.len() {
        LinkArtifactStatePayload::Completed
    } else if linked > 0 {
        LinkArtifactStatePayload::Partial
    } else if failed {
        LinkArtifactStatePayload::Failed
    } else {
        LinkArtifactStatePayload::Planned
    }
}

fn validate_progress(field: &'static str, progress: Option<f64>) -> RepositoryResult<()> {
    if progress.is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value)) {
        return Err(RepositoryError::invalid(
            field,
            "progress must be finite and between zero and one",
        ));
    }
    Ok(())
}

pub(crate) fn stable_download_artifact_key(
    account_key: &str,
    subject_id: &str,
    torrent_id: &str,
) -> String {
    stable_qb_idempotency_key(account_key, subject_id, torrent_id)
}

pub(crate) fn stable_resolved_link_artifact_key(
    account_key: &str,
    subject_id: &str,
    download_artifact_id: &str,
) -> String {
    stable_artifact_key(
        b"tmdb-mteam-hub/link-artifact/v1\0",
        LINK_ARTIFACT_KEY_PREFIX,
        &[account_key, subject_id, "resolved", download_artifact_id],
    )
}

fn stable_artifact_key(domain: &[u8], prefix: &str, components: &[&str]) -> String {
    let mut context = Context::new(&SHA256);
    context.update(domain);
    for component in components {
        context.update(&(component.len() as u64).to_be_bytes());
        context.update(component.as_bytes());
    }
    let digest = context.finish();
    let mut encoded = String::with_capacity(prefix.len() + digest.as_ref().len() * 2);
    encoded.push_str(prefix);
    for byte in digest.as_ref() {
        use fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        derive_link_state, ArtifactPayload, CandidateMatchPayload, CandidatePayload,
        CandidateRuleEvaluationPayload, LinkArtifactStatePayload, LinkFileOutcomePayload,
        LinkFilePayload, ObservationPayload, SubscriptionPayload, WantedSourcePayload,
    };

    fn source() -> WantedSourcePayload {
        WantedSourcePayload {
            title: "Fixture Blob Movie".to_string(),
            ..WantedSourcePayload::default()
        }
    }

    #[test]
    fn full_candidate_payload_round_trips_without_losing_optional_fields() {
        let candidate = CandidateMatchPayload {
            candidate: CandidatePayload {
                torrent_id: "fixture-torrent-blob-001".to_string(),
                title: "Fixture.Blob.Movie.1080p".to_string(),
                subtitle: "Fixture subtitle".to_string(),
                source: "fixture".to_string(),
                search_query: "fixture blob movie".to_string(),
                size: Some("8 GiB".to_string()),
                seeders: Some(42),
                leechers: Some(3),
                uploaded_at: Some("2026-07-11T10:00:00Z".to_string()),
            },
            selected: true,
            matched_rule_name: Some("fixture-rule".to_string()),
            matched_priority: Some(10),
            matched_keywords: vec!["1080p".to_string()],
            excluded_reason: None,
            rule_evaluations: vec![CandidateRuleEvaluationPayload {
                rule_name: "fixture-rule".to_string(),
                priority: 10,
                mode: "include".to_string(),
                matched: true,
                matched_keywords: vec!["1080p".to_string()],
                missing_keywords: Vec::new(),
                excluded_reason: None,
            }],
        };
        let payload = SubscriptionPayload {
            source: source(),
            observation: ObservationPayload {
                created_at: 1,
                first_seen_at: 1,
                last_seen_at: 2,
            },
            candidates: vec![candidate],
            artifacts: ArtifactPayload::default(),
            ..SubscriptionPayload::default()
        };

        let encoded = serde_json::to_value(&payload).unwrap();
        let decoded: SubscriptionPayload = serde_json::from_value(encoded).unwrap();

        assert_eq!(decoded, payload);
        assert!(decoded.validate().is_ok());
    }

    #[test]
    fn payload_and_source_reject_unknown_control_fields() {
        assert!(serde_json::from_value::<WantedSourcePayload>(json!({
            "title": "Fixture",
            "active": true
        }))
        .is_err());
        assert!(serde_json::from_value::<SubscriptionPayload>(json!({
            "source": { "title": "Fixture" },
            "execution_state": "running"
        }))
        .is_err());
    }

    #[test]
    fn missing_observation_defaults_are_rejected_by_validation() {
        let payload: SubscriptionPayload = serde_json::from_value(json!({
            "source": { "title": "Fixture" }
        }))
        .unwrap();

        assert!(matches!(
            payload.validate(),
            Err(super::RepositoryError::InvalidInput {
                field: "payload.observation",
                ..
            })
        ));
    }

    #[test]
    fn explicit_target_conflict_derives_a_failed_link_artifact() {
        let files = vec![LinkFilePayload {
            source_path: "/downloads/movie.mkv".to_string(),
            target_path: "/library/movie.mkv".to_string(),
            size: 1024,
            outcome: LinkFileOutcomePayload::Conflict,
            season_number: None,
            episode_number: None,
            episode_end_number: None,
            episode_label: None,
            error: Some("deterministic target belongs to another inode".to_string()),
        }];

        assert_eq!(derive_link_state(&files), LinkArtifactStatePayload::Failed);
    }
}
