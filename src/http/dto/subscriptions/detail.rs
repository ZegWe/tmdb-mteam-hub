use serde::Serialize;

use super::artifacts::{DownloadArtifactDto, LinkArtifactDto};
use super::SubscriptionSummaryDto;
use crate::subscription::repository::payload;
use crate::subscription::repository::SubscriptionDetail;

/// Explicit detail view for the authenticated management API.
///
/// The source fields cover media presentation, while candidates, TV intent,
/// files, and artifacts are the heavy data intentionally excluded from list
/// summaries. Storage controls, internal fields, provider download
/// URLs, and rule-engine internals are not part of this contract. Construction
/// requires an explicit diagnostic redactor so a future route cannot expose
/// persisted provider/config secrets by using a raw `From` conversion.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct SubscriptionDetailDto {
    summary: SubscriptionSummaryDto,
    source: SubscriptionSourceDto,
    observation: SubscriptionObservationDto,
    issues: Vec<SubscriptionIssueDto>,
    skip_reason: Option<String>,
    candidates: Vec<SubscriptionCandidateDto>,
    tv: Option<SubscriptionTvDetailDto>,
    downloads: Vec<DownloadArtifactDto>,
    links: Vec<LinkArtifactDto>,
}

impl SubscriptionDetailDto {
    pub(crate) fn from_detail(
        detail: &SubscriptionDetail,
        redact_diagnostic: &dyn Fn(&str) -> String,
    ) -> Self {
        let payload = detail.payload();
        Self {
            summary: detail.summary().into(),
            source: (&payload.source).into(),
            observation: (&payload.observation).into(),
            issues: payload
                .issues
                .iter()
                .map(|issue| SubscriptionIssueDto::from_payload(issue, redact_diagnostic))
                .collect(),
            skip_reason: payload.skip_reason.as_deref().map(redact_diagnostic),
            candidates: payload
                .candidates
                .iter()
                .map(|candidate| {
                    SubscriptionCandidateDto::from_payload(candidate, redact_diagnostic)
                })
                .collect(),
            tv: payload.tv.as_ref().map(SubscriptionTvDetailDto::from),
            downloads: payload
                .artifacts
                .downloads
                .iter()
                .map(DownloadArtifactDto::from)
                .collect(),
            links: payload
                .artifacts
                .links
                .iter()
                .map(|link| LinkArtifactDto::from_payload(link, redact_diagnostic))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct SubscriptionSourceDto {
    cover_url: String,
    original_title: Option<String>,
    aka: Vec<String>,
    languages: Vec<String>,
    countries: Vec<String>,
    genres: Vec<String>,
    directors: Vec<String>,
    actors: Vec<String>,
    date_published: Option<String>,
    duration: Option<String>,
    synopsis: Option<String>,
    rating_value: Option<f64>,
    rating_count: Option<u64>,
    tags: Vec<String>,
    douban_date: Option<String>,
    douban_return_order: Option<u32>,
}

impl From<&payload::WantedSourcePayload> for SubscriptionSourceDto {
    fn from(value: &payload::WantedSourcePayload) -> Self {
        Self {
            cover_url: value.cover_url.clone(),
            original_title: value.original_title.clone(),
            aka: value.aka.clone(),
            languages: value.languages.clone(),
            countries: value.countries.clone(),
            genres: value.genres.clone(),
            directors: value.directors.clone(),
            actors: value.actors.clone(),
            date_published: value.date_published.clone(),
            duration: value.duration.clone(),
            synopsis: value.summary.clone(),
            rating_value: value.rating_value,
            rating_count: value.rating_count,
            tags: value.tags.clone(),
            douban_date: value.douban_date.clone(),
            douban_return_order: value.douban_return_order,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SubscriptionObservationDto {
    created_at: u64,
    first_seen_at: u64,
    last_seen_at: u64,
}

impl From<&payload::ObservationPayload> for SubscriptionObservationDto {
    fn from(value: &payload::ObservationPayload) -> Self {
        Self {
            created_at: value.created_at,
            first_seen_at: value.first_seen_at,
            last_seen_at: value.last_seen_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SubscriptionIssueDto {
    owner: &'static str,
    artifact_id: Option<String>,
    lane: Option<&'static str>,
    season_number: Option<u32>,
    episode_number: Option<u32>,
    operation: Option<String>,
    error_type: Option<String>,
    message: String,
    occurred_at: Option<u64>,
}

impl SubscriptionIssueDto {
    fn from_payload(
        value: &payload::IssuePayload,
        redact_diagnostic: &dyn Fn(&str) -> String,
    ) -> Self {
        let (owner, artifact_id, lane, season_number, episode_number) = match &value.owner {
            payload::IssueOwnerPayload::Parent => ("subscription", None, None, None, None),
            payload::IssueOwnerPayload::Artifact {
                artifact_kind,
                artifact_id,
            } => (
                artifact_owner_label(*artifact_kind),
                Some(artifact_id.clone()),
                None,
                None,
                None,
            ),
            payload::IssueOwnerPayload::TvLane { lane } => {
                ("tv_lane", None, Some(tv_lane_label(*lane)), None, None)
            }
            payload::IssueOwnerPayload::TvEpisode {
                season_number,
                episode_number,
            } => (
                "tv_episode",
                None,
                None,
                Some(*season_number),
                Some(*episode_number),
            ),
        };
        Self {
            owner,
            artifact_id,
            lane,
            season_number,
            episode_number,
            operation: value.operation.as_deref().map(redact_diagnostic),
            error_type: value.error_type.as_deref().map(redact_diagnostic),
            message: redact_diagnostic(&value.message),
            occurred_at: value.occurred_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SubscriptionCandidateDto {
    torrent_id: String,
    title: String,
    subtitle: String,
    source: String,
    size: Option<String>,
    seeders: Option<u64>,
    leechers: Option<u64>,
    uploaded_at: Option<String>,
    selected: bool,
    matched_rule_name: Option<String>,
    matched_priority: Option<i32>,
    matched_keywords: Vec<String>,
    excluded_reason: Option<String>,
}

impl SubscriptionCandidateDto {
    fn from_payload(
        value: &payload::CandidateMatchPayload,
        redact_diagnostic: &dyn Fn(&str) -> String,
    ) -> Self {
        Self {
            torrent_id: value.candidate.torrent_id.clone(),
            title: value.candidate.title.clone(),
            subtitle: value.candidate.subtitle.clone(),
            source: value.candidate.source.clone(),
            size: value.candidate.size.clone(),
            seeders: value.candidate.seeders,
            leechers: value.candidate.leechers,
            uploaded_at: value.candidate.uploaded_at.clone(),
            selected: value.selected,
            matched_rule_name: value.matched_rule_name.clone(),
            matched_priority: value.matched_priority,
            matched_keywords: value.matched_keywords.clone(),
            excluded_reason: value.excluded_reason.as_deref().map(redact_diagnostic),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SubscriptionTvDetailDto {
    season_number: u32,
    episode_total: u32,
    target_start_episode: u32,
    target_end_episode: u32,
    episodes: Vec<SubscriptionTvEpisodeDto>,
}

impl From<&payload::TvDetailPayload> for SubscriptionTvDetailDto {
    fn from(value: &payload::TvDetailPayload) -> Self {
        Self {
            season_number: value.season_number,
            episode_total: value.episode_total,
            target_start_episode: value.target_start_episode,
            target_end_episode: value.target_end_episode,
            episodes: value
                .episodes
                .iter()
                .map(SubscriptionTvEpisodeDto::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SubscriptionTvEpisodeDto {
    season_number: u32,
    episode_number: u32,
    label: String,
    intent: &'static str,
}

impl From<&payload::TvEpisodeDetailPayload> for SubscriptionTvEpisodeDto {
    fn from(value: &payload::TvEpisodeDetailPayload) -> Self {
        Self {
            season_number: value.season_number,
            episode_number: value.episode_number,
            label: value.label.clone(),
            intent: tv_episode_intent_label(value.intent),
        }
    }
}

const fn artifact_owner_label(value: payload::ArtifactKindPayload) -> &'static str {
    match value {
        payload::ArtifactKindPayload::Download => "download_artifact",
        payload::ArtifactKindPayload::Link => "link_artifact",
    }
}

const fn tv_lane_label(value: payload::TvLanePayload) -> &'static str {
    match value {
        payload::TvLanePayload::Search => "search",
        payload::TvLanePayload::Progress => "progress",
        payload::TvLanePayload::Link => "link",
    }
}

const fn tv_episode_intent_label(value: payload::TvEpisodeIntentPayload) -> &'static str {
    match value {
        payload::TvEpisodeIntentPayload::Target => "target",
        payload::TvEpisodeIntentPayload::Skipped => "skipped",
    }
}
