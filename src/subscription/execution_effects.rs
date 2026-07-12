//! Production effects for latest-only movie execution.

use std::collections::{BTreeMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

mod link;
mod mteam;

use self::link::{error as hardlink_error, plans as link_plans};
use self::mteam::{
    append_candidates, candidates_from_response, match_candidates, search_body as mteam_search_body,
};

use super::effect_adapters::{
    HardlinkEffectAdapter, QbEffectAdapter, QbEffectAddSpec, QbTorrentInput,
};
use super::effects::{
    reconcile_qb_torrent, EnsureQbTorrentOutcome, LinkFileEffect, LinkFileOutcome,
    QbReconcileRequest, QbReconciliationDecision, QbTorrentObservation,
};
use super::execution::{
    ExecutionCategory, ExecutionEffectFuture, ExecutionEffectPolicy, ExecutionEffectResult,
    ExecutionQbServer, SubscriptionExecutionEffects,
};
use super::repository::payload::{
    stable_download_artifact_key, stable_resolved_link_artifact_key, CandidateMatchPayload,
    CandidatePayload, DownloadArtifactPayload, DownloadArtifactStatePayload, DownloadFilePayload,
    LinkArtifactPayload, LinkArtifactStatePayload, LinkDownloadRefPayload, LinkFilePayload,
};
use super::repository::{
    ClaimedSubscription, ExecutionOperation, ExecutionPayloadDelta, ExecutionScheduleDelay,
    FinishExecutionDisposition,
};
use crate::clients::mteam::MteamClient;
use crate::clients::qbittorrent::{self, QbTorrentFile, QbTorrentInfo};
use crate::config::QbServerEntry;

#[derive(Clone)]
pub(crate) struct LatestSubscriptionExecutionEffects {
    mteam: MteamClient,
    hardlinks: HardlinkEffectAdapter,
}

impl LatestSubscriptionExecutionEffects {
    pub(crate) fn try_production(
        mteam: MteamClient,
        filesystem_concurrency: usize,
    ) -> Result<Self, crate::storage::blocking::BlockingExecutorConfigError> {
        Ok(Self {
            mteam,
            hardlinks: HardlinkEffectAdapter::try_new(filesystem_concurrency)?,
        })
    }

    async fn execute_claimed(
        &self,
        claimed: &ClaimedSubscription,
        policy: &ExecutionEffectPolicy,
    ) -> ExecutionEffectResult {
        match claimed.attempt().token().operation() {
            ExecutionOperation::Meta => ExecutionEffectResult::Finished {
                disposition: FinishExecutionDisposition::MetaReady,
                payload_delta: ExecutionPayloadDelta::Meta,
            },
            ExecutionOperation::Search => self.execute_search(claimed, policy).await,
            ExecutionOperation::Progress => self.execute_progress(claimed, policy).await,
            ExecutionOperation::Link => self.execute_link(claimed, policy).await,
        }
    }

    async fn execute_search(
        &self,
        claimed: &ClaimedSubscription,
        policy: &ExecutionEffectPolicy,
    ) -> ExecutionEffectResult {
        let detail = claimed.detail();
        let key = &detail.summary().head.key;
        let payload = detail.payload();
        let retry_after = policy.search_interval_secs;
        let category = match category_for_source(&payload.source.tags, &policy.categories) {
            Ok(category) => category,
            Err(message) => {
                return failed(
                    ExecutionOperation::Search,
                    "configuration",
                    message,
                    retry_after,
                )
            }
        };
        let server = match qb_server(&policy.qb_servers, &category.qb_server_id) {
            Ok(server) => server,
            Err(message) => {
                return failed(
                    ExecutionOperation::Search,
                    "configuration",
                    message,
                    retry_after,
                )
            }
        };
        let server = client_qb_server(server);
        if policy.mteam_api_key.trim().is_empty() {
            return failed(
                ExecutionOperation::Search,
                "configuration",
                "M-Team API key is not configured",
                retry_after,
            );
        }

        let (matches, selected) = if let Some((matches, selected)) = retry_selection(payload) {
            (matches, Some(selected))
        } else {
            let candidates = match self
                .search_candidates(
                    policy.mteam_api_key.trim(),
                    &key.subject_id,
                    &payload.source.title,
                )
                .await
            {
                Ok(candidates) => candidates,
                Err(message) => {
                    return failed(
                        ExecutionOperation::Search,
                        "mteam_search",
                        message,
                        policy.system_retry_interval_secs,
                    )
                }
            };
            let matches = match_candidates(&candidates, &policy.torrent_match_rules);
            let selected = matches.iter().find(|candidate| candidate.selected).cloned();
            (matches, selected)
        };
        let Some(selected) = selected else {
            return ExecutionEffectResult::Finished {
                disposition: FinishExecutionDisposition::SearchWaiting {
                    retry_after: schedule_delay(retry_after),
                },
                payload_delta: ExecutionPayloadDelta::Search {
                    candidates: Some(matches),
                    download_updates: Vec::new(),
                },
            };
        };

        let torrent_id = selected.candidate.torrent_id.clone();
        let artifact_key =
            stable_download_artifact_key(&key.account_key, &key.subject_id, &torrent_id);
        let existing = payload
            .artifacts
            .downloads
            .iter()
            .find(|artifact| artifact.idempotency_key == artifact_key);
        let download_url = match self
            .mteam
            .fetch_download_url(policy.mteam_api_key.trim(), &torrent_id)
            .await
        {
            Ok(url) => url,
            Err(error) => {
                return failed_search_with_candidates(
                    matches,
                    Vec::new(),
                    "mteam_download_url",
                    error.to_string(),
                    policy.system_retry_interval_secs,
                )
            }
        };
        let request = match QbReconcileRequest::try_new(
            key.account_key.clone(),
            key.subject_id.clone(),
            torrent_id.clone(),
            existing.and_then(|artifact| artifact.qb_hash.as_deref()),
            None,
        ) {
            Ok(request) => request,
            Err(error) => {
                return failed_search_with_candidates(
                    matches,
                    Vec::new(),
                    "qb_identity",
                    error.to_string(),
                    policy.system_retry_interval_secs,
                )
            }
        };
        let input = match QbTorrentInput::from_url(download_url) {
            Ok(input) => input,
            Err(error) => {
                return failed_search_with_candidates(
                    matches,
                    Vec::new(),
                    "mteam_download_url",
                    error.to_string(),
                    policy.system_retry_interval_secs,
                )
            }
        };
        let add = QbEffectAddSpec::new(
            input,
            Some(category.qb_category.clone()),
            Some(category.qb_save_dir_name.clone()),
        );
        let mut adapter = QbEffectAdapter::for_server(server.clone());
        let now = system_now();
        let ensured = adapter.ensure(&request, &add).await;
        let (state, qb_hash, qb_name, error) = match ensured {
            Ok(EnsureQbTorrentOutcome::Added { .. }) => {
                (DownloadArtifactStatePayload::Pushed, None, None, None)
            }
            Ok(EnsureQbTorrentOutcome::Reconciled { torrent, .. }) => (
                DownloadArtifactStatePayload::Pushed,
                Some(torrent.hash().to_string()),
                Some(torrent.name().to_string()),
                None,
            ),
            Err(error) => (
                DownloadArtifactStatePayload::Failed,
                existing.and_then(|artifact| artifact.qb_hash.clone()),
                existing.and_then(|artifact| artifact.qb_name.clone()),
                Some(error.to_string()),
            ),
        };
        let artifact = DownloadArtifactPayload {
            idempotency_key: artifact_key,
            torrent_id: torrent_id.clone(),
            torrent_title: selected.candidate.title.clone(),
            qb_server_id: server.id.clone(),
            qb_server_name: non_empty(&server.name),
            qb_category: category.qb_category.clone(),
            qb_save_dir_name: category.qb_save_dir_name.clone(),
            qb_identifier: Some(request.idempotency_key().as_str().to_string()),
            qb_hash,
            qb_name,
            qb_state: None,
            torrent_download_url: None,
            mteam_torrent_url: Some(format!("https://kp.m-team.cc/detail/{torrent_id}")),
            state,
            progress: None,
            total_size: None,
            files: Vec::new(),
            pushed_at: Some(
                existing
                    .and_then(|artifact| artifact.pushed_at)
                    .unwrap_or(now),
            ),
            checked_at: Some(now),
            completed_at: None,
        };
        let delta = ExecutionPayloadDelta::Search {
            candidates: Some(matches),
            download_updates: vec![artifact],
        };
        if let Some(message) = error {
            ExecutionEffectResult::Failed {
                error_type: "qb_add".to_string(),
                message,
                retry_after_secs: policy.system_retry_interval_secs,
                payload_delta: delta,
            }
        } else {
            ExecutionEffectResult::Finished {
                disposition: FinishExecutionDisposition::SearchPushed,
                payload_delta: delta,
            }
        }
    }

    async fn execute_progress(
        &self,
        claimed: &ClaimedSubscription,
        policy: &ExecutionEffectPolicy,
    ) -> ExecutionEffectResult {
        let detail = claimed.detail();
        let key = &detail.summary().head.key;
        let Some(existing) = current_download(detail.payload()) else {
            return failed(
                ExecutionOperation::Progress,
                "missing_download_artifact",
                "subscription has no current download artifact",
                policy.system_retry_interval_secs,
            );
        };
        let server = match qb_server(&policy.qb_servers, &existing.qb_server_id) {
            Ok(server) => server,
            Err(message) => {
                return failed(
                    ExecutionOperation::Progress,
                    "configuration",
                    message,
                    policy.system_retry_interval_secs,
                )
            }
        };
        let server = client_qb_server(server);
        let torrent = match observe_download(&server, key, existing).await {
            Ok(Some(torrent)) => torrent,
            Ok(None) => {
                let mut missing = existing.clone();
                missing.state = DownloadArtifactStatePayload::Missing;
                missing.checked_at = Some(system_now());
                return ExecutionEffectResult::Failed {
                    error_type: "qb_missing".to_string(),
                    message: "qB task is missing for the stable subscription effect".to_string(),
                    retry_after_secs: policy.system_retry_interval_secs,
                    payload_delta: ExecutionPayloadDelta::Progress {
                        download_updates: vec![missing],
                    },
                };
            }
            Err(message) => {
                return failed(
                    ExecutionOperation::Progress,
                    "qb_inspect",
                    message,
                    policy.system_retry_interval_secs,
                )
            }
        };
        let files = match qbittorrent::torrent_files(&server, &torrent.hash).await {
            Ok(files) => files,
            Err(error) => {
                return failed(
                    ExecutionOperation::Progress,
                    "qb_files",
                    error.to_string(),
                    policy.system_retry_interval_secs,
                )
            }
        };
        let updated = updated_download(existing, &torrent, &files, system_now());
        let complete = torrent.is_complete();
        ExecutionEffectResult::Finished {
            disposition: if complete {
                FinishExecutionDisposition::ProgressDownloaded
            } else {
                FinishExecutionDisposition::ProgressPending {
                    retry_after: schedule_delay(policy.progress_interval_secs),
                }
            },
            payload_delta: ExecutionPayloadDelta::Progress {
                download_updates: vec![updated],
            },
        }
    }

    async fn execute_link(
        &self,
        claimed: &ClaimedSubscription,
        policy: &ExecutionEffectPolicy,
    ) -> ExecutionEffectResult {
        let detail = claimed.detail();
        let key = &detail.summary().head.key;
        let Some(existing_download) = current_download(detail.payload()) else {
            return failed(
                ExecutionOperation::Link,
                "missing_download_artifact",
                "subscription has no current download artifact",
                policy.system_retry_interval_secs,
            );
        };
        let category = match category_for_source(&detail.payload().source.tags, &policy.categories)
        {
            Ok(category) => category,
            Err(message) => {
                return failed(
                    ExecutionOperation::Link,
                    "configuration",
                    message,
                    policy.system_retry_interval_secs,
                )
            }
        };
        let server = match qb_server(&policy.qb_servers, &existing_download.qb_server_id) {
            Ok(server) => server,
            Err(message) => {
                return failed(
                    ExecutionOperation::Link,
                    "configuration",
                    message,
                    policy.system_retry_interval_secs,
                )
            }
        };
        let server = client_qb_server(server);
        let torrent = match observe_download(&server, key, existing_download).await {
            Ok(Some(torrent)) => torrent,
            Ok(None) => {
                return failed(
                    ExecutionOperation::Link,
                    "qb_missing",
                    "qB task is missing for the stable subscription effect",
                    policy.system_retry_interval_secs,
                )
            }
            Err(message) => {
                return failed(
                    ExecutionOperation::Link,
                    "qb_inspect",
                    message,
                    policy.system_retry_interval_secs,
                )
            }
        };
        let files = match qbittorrent::torrent_files(&server, &torrent.hash).await {
            Ok(files) => files,
            Err(error) => {
                return failed(
                    ExecutionOperation::Link,
                    "qb_files",
                    error.to_string(),
                    policy.system_retry_interval_secs,
                )
            }
        };
        let now = system_now();
        let updated_download = updated_download(existing_download, &torrent, &files, now);
        if !torrent.is_complete() {
            return ExecutionEffectResult::Finished {
                disposition: FinishExecutionDisposition::LinkPendingDownload {
                    retry_after: schedule_delay(policy.progress_interval_secs),
                },
                payload_delta: ExecutionPayloadDelta::Link {
                    download_updates: vec![updated_download],
                    link_updates: Vec::new(),
                },
            };
        }

        let plans = match link_plans(detail.payload(), category, existing_download, &files) {
            Ok(plans) => plans,
            Err(message) => {
                return ExecutionEffectResult::Failed {
                    error_type: "link_plan".to_string(),
                    message,
                    retry_after_secs: policy.link_retry_interval_secs,
                    payload_delta: ExecutionPayloadDelta::Link {
                        download_updates: vec![updated_download],
                        link_updates: Vec::new(),
                    },
                }
            }
        };
        let effects = match plans
            .iter()
            .map(|plan| {
                LinkFileEffect::try_new(
                    plan.source_path.clone(),
                    plan.target_path.clone(),
                    plan.persisted_outcome,
                )
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(effects) => effects,
            Err(error) => {
                return ExecutionEffectResult::Failed {
                    error_type: "link_plan".to_string(),
                    message: error.to_string(),
                    retry_after_secs: policy.link_retry_interval_secs,
                    payload_delta: ExecutionPayloadDelta::Link {
                        download_updates: vec![updated_download],
                        link_updates: Vec::new(),
                    },
                }
            }
        };
        let batch = match self.hardlinks.apply(effects).await {
            Ok(batch) => batch,
            Err(error) => {
                return ExecutionEffectResult::Failed {
                    error_type: "hardlink_executor".to_string(),
                    message: error.to_string(),
                    retry_after_secs: policy.link_retry_interval_secs,
                    payload_delta: ExecutionPayloadDelta::Link {
                        download_updates: vec![updated_download],
                        link_updates: Vec::new(),
                    },
                }
            }
        };
        let results = batch.into_files();
        let files_payload = plans
            .iter()
            .zip(results.iter())
            .map(|(plan, result)| LinkFilePayload {
                source_path: plan.source_path.display().to_string(),
                target_path: plan.target_path.display().to_string(),
                size: plan.size,
                outcome: result.outcome(),
                season_number: None,
                episode_number: None,
                episode_end_number: None,
                episode_label: None,
                error: hardlink_error(result.status()),
            })
            .collect::<Vec<_>>();
        let all_linked = files_payload
            .iter()
            .all(|file| file.outcome == LinkFileOutcome::Linked);
        let any_linked = files_payload
            .iter()
            .any(|file| file.outcome == LinkFileOutcome::Linked);
        let state = if all_linked {
            LinkArtifactStatePayload::Completed
        } else if any_linked {
            LinkArtifactStatePayload::Partial
        } else {
            LinkArtifactStatePayload::Failed
        };
        let link = LinkArtifactPayload {
            idempotency_key: stable_resolved_link_artifact_key(
                &key.account_key,
                &key.subject_id,
                &existing_download.idempotency_key,
            ),
            download: LinkDownloadRefPayload {
                artifact_id: existing_download.idempotency_key.clone(),
            },
            state,
            source_path: Some(category.download_dir.clone()),
            target_dir: plans
                .first()
                .and_then(|plan| plan.target_path.parent())
                .map(|path| path.display().to_string()),
            checked_at: now,
            completed_at: all_linked.then_some(now),
            files: files_payload,
        };
        ExecutionEffectResult::Finished {
            disposition: if all_linked {
                FinishExecutionDisposition::LinkCompleted
            } else {
                FinishExecutionDisposition::LinkPlanned {
                    retry_after: schedule_delay(policy.link_retry_interval_secs),
                }
            },
            payload_delta: ExecutionPayloadDelta::Link {
                download_updates: vec![updated_download],
                link_updates: vec![link],
            },
        }
    }

    async fn search_candidates(
        &self,
        api_key: &str,
        subject_id: &str,
        title: &str,
    ) -> Result<Vec<CandidatePayload>, String> {
        let mut candidates = Vec::new();
        let mut seen = HashSet::new();
        if !subject_id.trim().is_empty() {
            let query = format!("https://movie.douban.com/subject/{}/", subject_id.trim());
            let response = self
                .mteam
                .search(api_key, &mteam_search_body("douban", &query))
                .await
                .map_err(|error| error.to_string())?;
            append_candidates(
                &mut candidates,
                &mut seen,
                candidates_from_response(&response, "douban", subject_id),
            );
        }
        if !title.trim().is_empty() {
            let response = self
                .mteam
                .search(api_key, &mteam_search_body("keyword", title.trim()))
                .await
                .map_err(|error| error.to_string())?;
            append_candidates(
                &mut candidates,
                &mut seen,
                candidates_from_response(&response, "keyword", title),
            );
        }
        candidates.sort_by(|left, right| {
            right
                .seeders
                .unwrap_or_default()
                .cmp(&left.seeders.unwrap_or_default())
                .then_with(|| left.torrent_id.cmp(&right.torrent_id))
        });
        Ok(candidates)
    }
}

impl SubscriptionExecutionEffects for LatestSubscriptionExecutionEffects {
    fn execute(
        &self,
        claimed: ClaimedSubscription,
        policy: ExecutionEffectPolicy,
    ) -> ExecutionEffectFuture {
        let effects = self.clone();
        Box::pin(async move { effects.execute_claimed(&claimed, &policy).await })
    }
}

fn failed(
    operation: ExecutionOperation,
    error_type: impl Into<String>,
    message: impl Into<String>,
    retry_after_secs: u64,
) -> ExecutionEffectResult {
    ExecutionEffectResult::Failed {
        error_type: error_type.into(),
        message: message.into(),
        retry_after_secs,
        payload_delta: empty_delta(operation),
    }
}

fn failed_search_with_candidates(
    candidates: Vec<CandidateMatchPayload>,
    download_updates: Vec<DownloadArtifactPayload>,
    error_type: impl Into<String>,
    message: impl Into<String>,
    retry_after_secs: u64,
) -> ExecutionEffectResult {
    ExecutionEffectResult::Failed {
        error_type: error_type.into(),
        message: message.into(),
        retry_after_secs,
        payload_delta: ExecutionPayloadDelta::Search {
            candidates: Some(candidates),
            download_updates,
        },
    }
}

fn empty_delta(operation: ExecutionOperation) -> ExecutionPayloadDelta {
    match operation {
        ExecutionOperation::Meta => ExecutionPayloadDelta::Meta,
        ExecutionOperation::Search => ExecutionPayloadDelta::Search {
            candidates: None,
            download_updates: Vec::new(),
        },
        ExecutionOperation::Progress => ExecutionPayloadDelta::Progress {
            download_updates: Vec::new(),
        },
        ExecutionOperation::Link => ExecutionPayloadDelta::Link {
            download_updates: Vec::new(),
            link_updates: Vec::new(),
        },
    }
}

fn schedule_delay(seconds: u64) -> ExecutionScheduleDelay {
    ExecutionScheduleDelay::try_new(seconds)
        .expect("normalized watcher intervals must fit the SQLite integer range")
}

fn category_for_source<'a>(
    tags: &[String],
    categories: &'a [ExecutionCategory],
) -> Result<&'a ExecutionCategory, String> {
    let matches = categories
        .iter()
        .filter(|category| tags.iter().any(|tag| tag == &category.wanted_tag))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [category] => Ok(*category),
        [] => Err("subscription tags do not match a configured category".to_string()),
        _ => Err("subscription tags match more than one configured category".to_string()),
    }
}

fn qb_server<'a>(
    servers: &'a [ExecutionQbServer],
    id: &str,
) -> Result<&'a ExecutionQbServer, String> {
    let matches = servers
        .iter()
        .filter(|server| server.id == id)
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [server] => Ok(*server),
        [] => Err(format!("configured qB server does not exist: {id}")),
        _ => Err(format!("configured qB server ID is ambiguous: {id}")),
    }
}

fn client_qb_server(server: &ExecutionQbServer) -> QbServerEntry {
    QbServerEntry {
        id: server.id.clone(),
        name: server.name.clone(),
        base_url: server.base_url.clone(),
        username: server.username.clone(),
        password: server.password.clone(),
        insecure_tls: server.insecure_tls,
    }
}

fn current_download(
    payload: &super::repository::payload::SubscriptionPayload,
) -> Option<&DownloadArtifactPayload> {
    payload.artifacts.downloads.iter().rev().find(|artifact| {
        !matches!(
            artifact.state,
            DownloadArtifactStatePayload::Ignored | DownloadArtifactStatePayload::Superseded
        )
    })
}

fn retry_selection(
    payload: &super::repository::payload::SubscriptionPayload,
) -> Option<(Vec<CandidateMatchPayload>, CandidateMatchPayload)> {
    let artifact = current_download(payload)?;
    let selected = payload
        .candidates
        .iter()
        .find(|candidate| candidate.candidate.torrent_id == artifact.torrent_id)?
        .clone();
    let mut candidates = payload.candidates.clone();
    for candidate in &mut candidates {
        candidate.selected = candidate.candidate.torrent_id == artifact.torrent_id;
        if candidate.selected {
            candidate.excluded_reason = None;
        }
    }
    Some((candidates, selected))
}

async fn observe_download(
    server: &QbServerEntry,
    key: &super::repository::SubscriptionKey,
    artifact: &DownloadArtifactPayload,
) -> Result<Option<QbTorrentInfo>, String> {
    let request = QbReconcileRequest::try_new(
        key.account_key.clone(),
        key.subject_id.clone(),
        artifact.torrent_id.clone(),
        artifact.qb_hash.as_deref(),
        None,
    )
    .map_err(|error| error.to_string())?;
    let tagged =
        qbittorrent::list_torrents_by_exact_tag(server, request.idempotency_key().as_str())
            .await
            .map_err(|error| error.to_string())?;
    let hashed = if let Some(hash) = request.authoritative_hash() {
        qbittorrent::list_torrents_by_hashes(server, &[hash.to_string()])
            .await
            .map_err(|error| error.to_string())?
    } else {
        Vec::new()
    };
    let rows = merge_qb_rows(tagged, hashed);
    let observations = rows
        .values()
        .map(|torrent| {
            QbTorrentObservation::try_new(
                torrent.hash.clone(),
                torrent.name.clone(),
                torrent
                    .tags
                    .split(',')
                    .map(str::trim)
                    .filter(|tag| !tag.is_empty())
                    .map(str::to_string),
            )
            .map_err(|error| error.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    match reconcile_qb_torrent(&request, &observations) {
        QbReconciliationDecision::UseExisting { torrent, .. } => {
            Ok(rows.get(torrent.hash()).cloned())
        }
        QbReconciliationDecision::Add => Ok(None),
        QbReconciliationDecision::Conflict(conflict) => {
            Err(format!("qB reconciliation conflict: {conflict:?}"))
        }
    }
}

fn merge_qb_rows(
    tagged: Vec<QbTorrentInfo>,
    hashed: Vec<QbTorrentInfo>,
) -> BTreeMap<String, QbTorrentInfo> {
    let mut rows = BTreeMap::new();
    for torrent in tagged.into_iter().chain(hashed) {
        rows.entry(torrent.hash.to_ascii_lowercase())
            .or_insert(torrent);
    }
    rows
}

fn updated_download(
    existing: &DownloadArtifactPayload,
    torrent: &QbTorrentInfo,
    files: &[QbTorrentFile],
    now: u64,
) -> DownloadArtifactPayload {
    let complete = torrent.is_complete();
    let mut updated = existing.clone();
    updated.qb_hash = Some(torrent.hash.to_ascii_lowercase());
    updated.qb_name = non_empty(&torrent.name);
    updated.qb_state = non_empty(&torrent.state);
    updated.state = if complete {
        DownloadArtifactStatePayload::Downloaded
    } else {
        DownloadArtifactStatePayload::Downloading
    };
    updated.progress = Some(torrent.progress.clamp(0.0, 1.0));
    updated.total_size = Some(torrent.size);
    updated.files = files
        .iter()
        .map(|file| DownloadFilePayload {
            name: file.name.clone(),
            size: file.size,
            progress: file.progress.clamp(0.0, 1.0),
            priority: file.priority,
            season_number: None,
            episode_number: None,
            episode_end_number: None,
            episode_label: None,
        })
        .collect();
    updated.checked_at = Some(now);
    updated.completed_at = complete.then_some(now).or(existing.completed_at);
    updated
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn system_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: &str, title: &str, seeders: u64) -> CandidatePayload {
        CandidatePayload {
            torrent_id: id.to_string(),
            title: title.to_string(),
            subtitle: String::new(),
            source: "fixture".to_string(),
            search_query: "fixture".to_string(),
            size: None,
            seeders: Some(seeders),
            leechers: None,
            uploaded_at: None,
        }
    }

    #[test]
    fn response_loss_retry_keeps_the_persisted_torrent_identity() {
        let first = CandidateMatchPayload {
            candidate: candidate("1", "Movie 1080p", 10),
            selected: false,
            matched_rule_name: None,
            matched_priority: None,
            matched_keywords: Vec::new(),
            excluded_reason: Some("old ordering".to_string()),
            rule_evaluations: Vec::new(),
        };
        let second = CandidateMatchPayload {
            candidate: candidate("2", "Movie 2160p", 20),
            selected: true,
            matched_rule_name: None,
            matched_priority: None,
            matched_keywords: Vec::new(),
            excluded_reason: None,
            rule_evaluations: Vec::new(),
        };
        let payload = super::super::repository::payload::SubscriptionPayload {
            candidates: vec![first, second],
            artifacts: super::super::repository::payload::ArtifactPayload {
                downloads: vec![DownloadArtifactPayload {
                    idempotency_key: "download:v1:fixture".to_string(),
                    torrent_id: "1".to_string(),
                    torrent_title: "Movie 1080p".to_string(),
                    qb_server_id: "qb".to_string(),
                    qb_server_name: None,
                    qb_category: "movie".to_string(),
                    qb_save_dir_name: "/downloads".to_string(),
                    qb_identifier: None,
                    qb_hash: None,
                    qb_name: None,
                    qb_state: None,
                    torrent_download_url: None,
                    mteam_torrent_url: None,
                    state: DownloadArtifactStatePayload::Failed,
                    progress: None,
                    total_size: None,
                    files: Vec::new(),
                    pushed_at: Some(1),
                    checked_at: Some(1),
                    completed_at: None,
                }],
                links: Vec::new(),
            },
            ..super::super::repository::payload::SubscriptionPayload::default()
        };

        let (candidates, selected) = retry_selection(&payload).unwrap();

        assert_eq!(selected.candidate.torrent_id, "1");
        assert!(candidates[0].selected);
        assert!(!candidates[1].selected);
    }
}
