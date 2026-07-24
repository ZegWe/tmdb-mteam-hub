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
    TvDetailPayload, TvEpisodeDetailPayload, TvEpisodeIntentPayload, WantedSourcePayload,
};
use super::repository::{
    ClaimedSubscription, ExecutionOperation, ExecutionPayloadDelta, ExecutionScheduleDelay,
    FinishExecutionDisposition,
};
use crate::clients::douban::DoubanClient;
use crate::clients::mteam::MteamClient;
use crate::douban::DoubanSubjectDetail;
use crate::clients::qbittorrent::{self, QbTorrentFile, QbTorrentInfo};
use crate::clients::tmdb::TmdbClient;
use crate::config::QbServerEntry;
use crate::subscription::episode::recognize as recognize_episode;
use crate::subscription::SubscriptionMediaKind;

#[derive(Clone)]
pub(crate) struct LatestSubscriptionExecutionEffects {
    douban: DoubanClient,
    tmdb: TmdbClient,
    mteam: MteamClient,
    hardlinks: HardlinkEffectAdapter,
}

impl LatestSubscriptionExecutionEffects {
    pub(crate) fn try_production(
        douban: DoubanClient,
        tmdb: TmdbClient,
        mteam: MteamClient,
        filesystem_concurrency: usize,
    ) -> Result<Self, crate::storage::blocking::BlockingExecutorConfigError> {
        Ok(Self {
            douban,
            tmdb,
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
            ExecutionOperation::Meta => self.execute_meta(claimed, policy).await,
            ExecutionOperation::Search => self.execute_search(claimed, policy).await,
            ExecutionOperation::Progress => self.execute_progress(claimed, policy).await,
            ExecutionOperation::Link => self.execute_link(claimed, policy).await,
        }
    }

    async fn execute_meta(
        &self,
        claimed: &ClaimedSubscription,
        policy: &ExecutionEffectPolicy,
    ) -> ExecutionEffectResult {
        if claimed.detail().summary().head.media_kind == SubscriptionMediaKind::Movie {
            let key = &claimed.detail().summary().head.key;
            return match crate::douban::subject_detail(
                &self.douban,
                &policy.douban_cookie,
                &key.subject_id,
            )
            .await
            {
                Ok(detail) => ExecutionEffectResult::Finished {
                    disposition: FinishExecutionDisposition::MetaReady,
                    payload_delta: ExecutionPayloadDelta::MovieMeta {
                        source: Box::new(movie_source_from_detail(
                            &claimed.detail().payload().source,
                            detail,
                        )),
                    },
                },
                Err(error) => failed(
                    ExecutionOperation::Meta,
                    "movie_metadata",
                    error.to_string(),
                    policy.system_retry_interval_secs,
                ),
            };
        }
        let source = &claimed.detail().payload().source;
        let key = &claimed.detail().summary().head.key;
        let retry_after = policy.system_retry_interval_secs;

        let douban_result = crate::douban::subject_detail(
            &self.douban,
            &policy.douban_cookie,
            &key.subject_id,
        )
        .await;

        let enriched_source = match douban_result.as_ref().ok() {
            Some(detail) => Some(Box::new(tv_source_from_detail(source, detail))),
            None => None,
        };

        if let Ok(ref detail) = douban_result {
            if let Some(episodes_count) = detail.episodes_count.filter(|&n| n > 0) {
                let season_number = resolve_tv_season_number(
                    &self.douban,
                    &policy.douban_cookie,
                    &key.subject_id,
                    &source.title,
                )
                .await;
                let tv = tv_detail_from_douban(episodes_count, season_number);
                return ExecutionEffectResult::Finished {
                    disposition: FinishExecutionDisposition::MetaReady,
                    payload_delta: ExecutionPayloadDelta::TvMeta {
                        tv,
                        source: enriched_source,
                    },
                };
            }
        }

        if policy.tmdb_api_key.trim().is_empty() {
            return failed(
                ExecutionOperation::Meta,
                "configuration",
                "TMDB API credential is required when Douban TV data is insufficient",
                retry_after,
            );
        }
        let lookup_title = source
            .original_title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or(&source.title);
        let requested_season = season_number_from_title(&source.title)
            .or_else(|| season_number_from_title(lookup_title));
        match self
            .load_tv_detail(policy.tmdb_api_key.trim(), lookup_title, requested_season)
            .await
        {
            Ok(tv) => ExecutionEffectResult::Finished {
                disposition: FinishExecutionDisposition::MetaReady,
                payload_delta: ExecutionPayloadDelta::TvMeta {
                    tv,
                    source: enriched_source,
                },
            },
            Err(message) => failed(
                ExecutionOperation::Meta,
                "tv_metadata",
                message,
                retry_after,
            ),
        }
    }

    async fn load_tv_detail(
        &self,
        api_key: &str,
        title: &str,
        requested_season: Option<u32>,
    ) -> Result<TvDetailPayload, String> {
        let search_title = tv_series_search_title(title);
        let response = self
            .tmdb
            .get_json(
                api_key,
                "/search/tv",
                &[("query", &search_title), ("language", "zh-CN")],
            )
            .await
            .map_err(|error| error.to_string())?;
        let result = response
            .get("results")
            .and_then(serde_json::Value::as_array)
            .and_then(|results| results.first())
            .ok_or_else(|| format!("TMDB did not find TV metadata for {title}"))?;
        let id = result
            .get("id")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| "TMDB TV search result has no ID".to_string())?;
        let detail = self
            .tmdb
            .get_json(api_key, &format!("/tv/{id}"), &[("language", "zh-CN")])
            .await
            .map_err(|error| error.to_string())?;
        let seasons = detail
            .get("seasons")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| "TMDB TV detail has no seasons".to_string())?;
        let season = requested_season
            .and_then(|requested| {
                seasons.iter().find(|season| {
                    season
                        .get("season_number")
                        .and_then(serde_json::Value::as_u64)
                        == Some(u64::from(requested))
                })
            })
            .or_else(|| {
                seasons
                    .iter()
                    .filter(|season| {
                        season
                            .get("season_number")
                            .and_then(serde_json::Value::as_u64)
                            .is_some_and(|number| number > 0)
                            && season
                                .get("episode_count")
                                .and_then(serde_json::Value::as_u64)
                                .is_some_and(|count| count > 0)
                    })
                    .max_by_key(|season| {
                        season
                            .get("season_number")
                            .and_then(serde_json::Value::as_u64)
                    })
            })
            .ok_or_else(|| "TMDB TV detail has no regular season".to_string())?;
        let season_number = u32::try_from(
            season
                .get("season_number")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .map_err(|_| "TMDB season number is too large".to_string())?;
        let episode_total = u32::try_from(
            season
                .get("episode_count")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        )
        .map_err(|_| "TMDB episode count is too large".to_string())?;
        if season_number == 0 || episode_total == 0 {
            return Err("TMDB selected season has no episodes".to_string());
        }
        Ok(TvDetailPayload {
            season_number,
            episode_total,
            target_start_episode: 1,
            target_end_episode: episode_total,
            episodes: (1..=episode_total)
                .map(|episode_number| TvEpisodeDetailPayload {
                    season_number,
                    episode_number,
                    label: format!("S{season_number:02}E{episode_number:02}"),
                    intent: TvEpisodeIntentPayload::Target,
                })
                .collect(),
        })
    }

    async fn execute_search(
        &self,
        claimed: &ClaimedSubscription,
        policy: &ExecutionEffectPolicy,
    ) -> ExecutionEffectResult {
        let detail = claimed.detail();
        let key = &detail.summary().head.key;
        let payload = detail.payload();
        let is_tv = detail.summary().head.media_kind == SubscriptionMediaKind::Tv;
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
        let tv_search = payload.tv.as_ref().and_then(|tv| {
            first_uncovered_episode(payload, tv).map(|episode| {
                let title = payload
                    .source
                    .original_title
                    .as_deref()
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or(&payload.source.title);
                format!("{title} S{:02}E{:02}", tv.season_number, episode)
            })
        });
        let search_subject_id = if is_tv { "" } else { key.subject_id.as_str() };
        let search_title = tv_search.as_deref().unwrap_or(&payload.source.title);

        let (mut matches, mut selected) =
            if let Some((matches, selected)) = retry_selection(payload) {
                (matches, Some(selected))
            } else {
                let candidates = match self
                    .search_candidates(
                        policy.mteam_api_key.trim(),
                        &policy.douban_cookie,
                        search_subject_id,
                        search_title,
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
        if is_tv {
            let Some(tv) = payload.tv.as_ref() else {
                return failed(
                    ExecutionOperation::Search,
                    "tv_metadata",
                    "TV episode metadata is missing",
                    policy.system_retry_interval_secs,
                );
            };
            let Some(cursor) = first_uncovered_episode(payload, tv) else {
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
            selected = select_tv_candidate(
                &mut matches,
                tv.season_number,
                cursor,
                &policy.torrent_match_rules,
                &payload.artifacts.downloads,
            );
        }
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
        let updated = updated_download(
            existing,
            &torrent,
            &files,
            detail.payload().tv.as_ref(),
            system_now(),
        );
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
        let mut updated_download = updated_download(
            existing_download,
            &torrent,
            &files,
            detail.payload().tv.as_ref(),
            now,
        );
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

        if let Some(tv) = detail.payload().tv.as_ref() {
            let maps_target_episode = updated_download.files.iter().any(|file| {
                file.season_number == Some(tv.season_number)
                    && file.episode_number.is_some_and(|start| {
                        let end = file.episode_end_number.unwrap_or(start);
                        start <= tv.target_end_episode && end >= tv.target_start_episode
                    })
            });
            if !maps_target_episode {
                updated_download.state = DownloadArtifactStatePayload::Ignored;
                return ExecutionEffectResult::Finished {
                    disposition: FinishExecutionDisposition::LinkMoreRequired,
                    payload_delta: ExecutionPayloadDelta::Link {
                        download_updates: vec![updated_download],
                        link_updates: Vec::new(),
                    },
                };
            }
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
            .map(|(plan, result)| {
                let marker = file_episode_marker(
                    &plan.source_path.to_string_lossy(),
                    detail.payload().tv.as_ref(),
                );
                LinkFilePayload {
                    source_path: plan.source_path.display().to_string(),
                    target_path: plan.target_path.display().to_string(),
                    size: plan.size,
                    outcome: result.outcome(),
                    season_number: marker.0,
                    episode_number: marker.1,
                    episode_end_number: marker.2,
                    episode_label: marker.3,
                    error: hardlink_error(result.status()),
                }
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
            files: files_payload.clone(),
        };
        ExecutionEffectResult::Finished {
            disposition: if all_linked {
                if detail.summary().head.media_kind == SubscriptionMediaKind::Tv
                    && detail.payload().tv.as_ref().is_some_and(|tv| {
                        !tv_complete_after_link(detail.payload(), tv, &files_payload)
                    })
                {
                    FinishExecutionDisposition::LinkMoreRequired
                } else {
                    FinishExecutionDisposition::LinkCompleted
                }
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
        douban_cookie: &str,
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
        if should_try_imdb_fallback(&candidates, subject_id) {
            if let Ok(detail) =
                crate::douban::subject_detail(&self.douban, douban_cookie, subject_id.trim()).await
            {
                if let Some(imdb_id) = detail.imdb_id {
                    let imdb_url = format!("https://www.imdb.com/title/{imdb_id}/");
                    let response = self
                        .mteam
                        .search(api_key, &mteam_search_body("imdb", &imdb_url))
                        .await
                        .map_err(|error| error.to_string())?;
                    append_candidates(
                        &mut candidates,
                        &mut seen,
                        candidates_from_response(&response, "imdb", &imdb_id),
                    );
                }
            }
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

fn should_try_imdb_fallback(candidates: &[CandidatePayload], subject_id: &str) -> bool {
    candidates.is_empty() && !subject_id.trim().is_empty()
}

fn movie_source_from_detail(
    current: &WantedSourcePayload,
    detail: crate::douban::DoubanSubjectDetail,
) -> WantedSourcePayload {
    let mut source = current.clone();
    if !detail.title.trim().is_empty() {
        source.title = detail.title.trim().to_string();
    }
    if !detail.poster_url.trim().is_empty() {
        source.poster_url = detail.poster_url.trim().to_string();
    }
    if !detail.image.trim().is_empty() {
        source.cover_url = detail.image.trim().to_string();
    }
    source.original_title = non_empty(&detail.original_title).or(source.original_title);
    for (target, observed) in [
        (&mut source.aka, detail.aka),
        (&mut source.languages, detail.languages),
        (&mut source.countries, detail.countries),
        (&mut source.genres, detail.genres),
        (&mut source.directors, detail.directors),
        (&mut source.actors, detail.actors),
    ] {
        if !observed.is_empty() {
            *target = observed;
        }
    }
    source.date_published = non_empty(&detail.date_published).or(source.date_published);
    source.duration = non_empty(&detail.duration).or(source.duration);
    source.summary = non_empty(&detail.summary).or(source.summary);
    source.rating_value = detail.rating.value.or(source.rating_value);
    source.rating_count = detail.rating.count.or(source.rating_count);
    source.release_year =
        release_year_from_metadata(source.date_published.as_deref()).or(source.release_year);
    source
}

fn tv_source_from_detail(
    current: &WantedSourcePayload,
    detail: &DoubanSubjectDetail,
) -> WantedSourcePayload {
    let mut source = current.clone();
    if !detail.title.trim().is_empty() {
        source.title = detail.title.trim().to_string();
    }
    if !detail.poster_url.trim().is_empty() {
        source.poster_url = detail.poster_url.trim().to_string();
    }
    if !detail.image.trim().is_empty() {
        source.cover_url = detail.image.trim().to_string();
    }
    source.original_title = non_empty(&detail.original_title).or(source.original_title);
    for (target, observed) in [
        (&mut source.aka, &detail.aka),
        (&mut source.languages, &detail.languages),
        (&mut source.countries, &detail.countries),
        (&mut source.genres, &detail.genres),
        (&mut source.directors, &detail.directors),
        (&mut source.actors, &detail.actors),
    ] {
        if !observed.is_empty() {
            target.clone_from(observed);
        }
    }
    source.date_published = non_empty(&detail.date_published).or(source.date_published);
    source.duration = non_empty(&detail.duration).or(source.duration);
    source.summary = non_empty(&detail.summary).or(source.summary);
    source.rating_value = detail.rating.value.or(source.rating_value);
    source.rating_count = detail.rating.count.or(source.rating_count);
    source.release_year =
        release_year_from_metadata(source.date_published.as_deref()).or(source.release_year);
    source
}

fn tv_detail_from_douban(episodes_count: u32, season_number: u32) -> TvDetailPayload {
    TvDetailPayload {
        season_number,
        episode_total: episodes_count,
        target_start_episode: 1,
        target_end_episode: episodes_count,
        episodes: (1..=episodes_count)
            .map(|episode_number| TvEpisodeDetailPayload {
                season_number,
                episode_number,
                label: format!("S{season_number:02}E{episode_number:02}"),
                intent: TvEpisodeIntentPayload::Target,
            })
            .collect(),
    }
}

async fn resolve_tv_season_number(
    douban: &DoubanClient,
    cookie_header: &str,
    subject_id: &str,
    source_title: &str,
) -> u32 {
    if let Ok(seasons) = crate::douban::tv_seasons(douban, cookie_header, subject_id).await {
        if !seasons.is_empty() {
            if let Some(season) = seasons.iter().find(|s| s.id == subject_id) {
                if let Some(parsed) = season_number_from_title(&season.title) {
                    return parsed;
                }
            }
            if seasons.len() > 1 {
                if let Some(requested) = season_number_from_title(source_title) {
                    if seasons
                        .iter()
                        .any(|s| season_number_from_title(&s.title) == Some(requested))
                    {
                        return requested;
                    }
                }
            }
        }
    }
    season_number_from_title(source_title).unwrap_or(1)
}

fn release_year_from_metadata(value: Option<&str>) -> Option<u16> {
    let value = value?;
    value.as_bytes().windows(4).find_map(|digits| {
        digits
            .iter()
            .all(u8::is_ascii_digit)
            .then(|| std::str::from_utf8(digits).ok()?.parse::<u16>().ok())
            .flatten()
            .filter(|year| (1..=9999).contains(year))
    })
}

fn season_number_from_title(title: &str) -> Option<u32> {
    let lower = title.to_ascii_lowercase();
    for marker in ["season ", "season.", "season_", "season-"] {
        if let Some(rest) = lower.split(marker).nth(1) {
            if let Some(value) = leading_number(rest) {
                return Some(value);
            }
        }
    }
    let chars = title.chars().collect::<Vec<_>>();
    for index in 0..chars.len() {
        if chars[index] != '第' {
            continue;
        }
        let Some(relative_end) = chars[index + 1..].iter().position(|value| *value == '季') else {
            continue;
        };
        let end = index + 1 + relative_end;
        let number = chars[index + 1..end].iter().collect::<String>();
        if let Ok(value) = number.parse() {
            return Some(value);
        }
        if let Some(value) = parse_chinese_number(&number) {
            return Some(value);
        }
    }
    None
}

fn parse_chinese_number(value: &str) -> Option<u32> {
    fn digit(character: char) -> Option<u32> {
        match character {
            '零' | '〇' => Some(0),
            '一' => Some(1),
            '二' | '两' => Some(2),
            '三' => Some(3),
            '四' => Some(4),
            '五' => Some(5),
            '六' => Some(6),
            '七' => Some(7),
            '八' => Some(8),
            '九' => Some(9),
            _ => None,
        }
    }

    let chars = value.trim().chars().collect::<Vec<_>>();
    match chars.as_slice() {
        ['十'] => Some(10),
        ['十', ones] => digit(*ones).map(|ones| 10 + ones),
        [tens, '十'] => digit(*tens).map(|tens| tens * 10),
        [tens, '十', ones] => Some(digit(*tens)? * 10 + digit(*ones)?),
        [single] => digit(*single),
        _ => None,
    }
    .filter(|value| *value > 0)
}

fn tv_series_search_title(title: &str) -> String {
    let trimmed = title.trim();
    let lower = trimmed.to_ascii_lowercase();
    let english = [" season ", " season.", " season_", " season-"]
        .iter()
        .filter_map(|marker| lower.find(marker))
        .min();
    let chinese = trimmed
        .char_indices()
        .find(|(index, character)| *character == '第' && trimmed[*index..].contains('季'))
        .map(|(index, _)| index);
    english
        .into_iter()
        .chain(chinese)
        .min()
        .map(|index| trimmed[..index].trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(trimmed)
        .to_string()
}

fn leading_number(value: &str) -> Option<u32> {
    value
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

fn first_uncovered_episode(
    payload: &super::repository::payload::SubscriptionPayload,
    tv: &TvDetailPayload,
) -> Option<u32> {
    (tv.target_start_episode..=tv.target_end_episode).find(|episode| {
        !payload
            .artifacts
            .links
            .iter()
            .flat_map(|link| &link.files)
            .any(|file| {
                file.outcome == LinkFileOutcome::Linked
                    && file.season_number == Some(tv.season_number)
                    && file.episode_number.is_some_and(|start| {
                        let end = file.episode_end_number.unwrap_or(start);
                        (start..=end).contains(episode)
                    })
            })
    })
}

fn select_tv_candidate(
    matches: &mut [CandidateMatchPayload],
    season: u32,
    episode: u32,
    rules: &[super::execution::ExecutionTorrentMatchRule],
    downloads: &[DownloadArtifactPayload],
) -> Option<CandidateMatchPayload> {
    let used = downloads
        .iter()
        .map(|download| download.torrent_id.as_str())
        .collect::<HashSet<_>>();
    let mut selected_index = None;
    let mut selected_priority = i32::MIN;
    for (index, candidate) in matches.iter().enumerate() {
        let eligible = !candidate.candidate.torrent_id.is_empty()
            && !used.contains(candidate.candidate.torrent_id.as_str())
            && (rules.is_empty() || candidate.matched_priority.is_some())
            && recognize_episode(&candidate.candidate.title)
                .is_some_and(|coverage| coverage.covers(season, episode));
        if !eligible {
            continue;
        }
        let priority = candidate.matched_priority.unwrap_or_default();
        if selected_index.is_none() || priority > selected_priority {
            selected_index = Some(index);
            selected_priority = priority;
        }
    }
    for (index, candidate) in matches.iter_mut().enumerate() {
        candidate.selected = Some(index) == selected_index;
        if candidate.selected {
            candidate.excluded_reason = None;
        } else if used.contains(candidate.candidate.torrent_id.as_str()) {
            candidate.excluded_reason =
                Some("torrent was already pushed for this TV subscription".to_string());
        } else if recognize_episode(&candidate.candidate.title)
            .is_none_or(|coverage| !coverage.covers(season, episode))
        {
            candidate.excluded_reason =
                Some(format!("torrent does not cover S{season:02}E{episode:02}"));
        }
    }
    selected_index.map(|index| matches[index].clone())
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
    if !matches!(artifact.state, DownloadArtifactStatePayload::Failed) {
        return None;
    }
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
    tv: Option<&TvDetailPayload>,
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
        .map(|file| {
            let marker = file_episode_marker(&file.name, tv);
            DownloadFilePayload {
                name: file.name.clone(),
                size: file.size,
                progress: file.progress.clamp(0.0, 1.0),
                priority: file.priority,
                season_number: marker.0,
                episode_number: marker.1,
                episode_end_number: marker.2,
                episode_label: marker.3,
            }
        })
        .collect();
    updated.checked_at = Some(now);
    updated.completed_at = complete.then_some(now).or(existing.completed_at);
    updated
}

fn file_episode_marker(
    name: &str,
    tv: Option<&TvDetailPayload>,
) -> (Option<u32>, Option<u32>, Option<u32>, Option<String>) {
    if !is_video_file(name) {
        return (None, None, None, None);
    }
    let Some(coverage) = recognize_episode(name) else {
        return (None, None, None, None);
    };
    let (mut season, episode, end) = coverage.parts();
    if season.is_none() {
        season = tv.map(|tv| tv.season_number);
    }
    (season, episode, end, Some(coverage.label()))
}

fn is_video_file(name: &str) -> bool {
    let extension = std::path::Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "mkv" | "mp4" | "avi" | "ts" | "m2ts" | "mov" | "wmv" | "webm" | "mpg" | "mpeg"
    )
}

fn tv_complete_after_link(
    payload: &super::repository::payload::SubscriptionPayload,
    tv: &TvDetailPayload,
    current: &[LinkFilePayload],
) -> bool {
    (tv.target_start_episode..=tv.target_end_episode).all(|episode| {
        payload
            .artifacts
            .links
            .iter()
            .flat_map(|link| &link.files)
            .chain(current)
            .any(|file| {
                file.outcome == LinkFileOutcome::Linked
                    && file.season_number == Some(tv.season_number)
                    && file.episode_number.is_some_and(|start| {
                        (start..=file.episode_end_number.unwrap_or(start)).contains(&episode)
                    })
            })
    })
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

    fn candidate_match(id: &str, title: &str) -> CandidateMatchPayload {
        CandidateMatchPayload {
            candidate: candidate(id, title, 10),
            selected: false,
            matched_rule_name: None,
            matched_priority: None,
            matched_keywords: Vec::new(),
            excluded_reason: None,
            rule_evaluations: Vec::new(),
        }
    }

    fn tv_detail() -> TvDetailPayload {
        TvDetailPayload {
            season_number: 2,
            episode_total: 6,
            target_start_episode: 1,
            target_end_episode: 6,
            episodes: Vec::new(),
        }
    }

    fn linked_file(start: u32, end: Option<u32>) -> LinkFilePayload {
        LinkFilePayload {
            source_path: format!("S02E{start:02}.mkv"),
            target_path: format!("Season 02/S02E{start:02}.mkv"),
            size: 1,
            outcome: LinkFileOutcome::Linked,
            season_number: Some(2),
            episode_number: Some(start),
            episode_end_number: end,
            episode_label: None,
            error: None,
        }
    }

    fn link_artifact(files: Vec<LinkFilePayload>) -> LinkArtifactPayload {
        LinkArtifactPayload {
            idempotency_key: "link:v1:test".to_string(),
            download: LinkDownloadRefPayload {
                artifact_id: "download:v1:test".to_string(),
            },
            state: LinkArtifactStatePayload::Completed,
            source_path: None,
            target_dir: None,
            checked_at: 1,
            completed_at: Some(1),
            files,
        }
    }

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

    #[test]
    fn imdb_fallback_runs_only_after_an_empty_douban_result() {
        assert!(should_try_imdb_fallback(&[], "1292052"));
        assert!(!should_try_imdb_fallback(
            &[candidate("1", "Douban result", 1)],
            "1292052"
        ));
        assert!(!should_try_imdb_fallback(&[], "  "));
    }

    #[test]
    fn tv_candidate_selection_requires_cursor_coverage_and_skips_used_torrents() {
        let mut matches = vec![
            candidate_match("wrong-season", "Show.S01E03.1080p"),
            candidate_match("used", "Show.S02E03.1080p"),
            candidate_match("range", "Show.S02E01-E04.1080p"),
            candidate_match("pack", "Show.S02.Complete.1080p"),
        ];
        let downloads = vec![DownloadArtifactPayload {
            idempotency_key: "download:v1:used".to_string(),
            torrent_id: "used".to_string(),
            torrent_title: "Show.S02E03.1080p".to_string(),
            qb_server_id: "qb".to_string(),
            qb_server_name: None,
            qb_category: "tv".to_string(),
            qb_save_dir_name: "/downloads".to_string(),
            qb_identifier: None,
            qb_hash: None,
            qb_name: None,
            qb_state: None,
            torrent_download_url: None,
            mteam_torrent_url: None,
            state: DownloadArtifactStatePayload::Downloaded,
            progress: Some(1.0),
            total_size: None,
            files: Vec::new(),
            pushed_at: Some(1),
            checked_at: Some(1),
            completed_at: Some(1),
        }];

        let selected = select_tv_candidate(&mut matches, 2, 3, &[], &downloads).unwrap();

        assert_eq!(selected.candidate.torrent_id, "range");
        assert_eq!(
            matches
                .iter()
                .filter(|candidate| candidate.selected)
                .count(),
            1
        );
        assert!(matches[0]
            .excluded_reason
            .as_deref()
            .unwrap()
            .contains("does not cover"));
        assert!(matches[1]
            .excluded_reason
            .as_deref()
            .unwrap()
            .contains("already pushed"));

        let mut pack_only = vec![candidate_match("pack", "Show.S02.Complete.1080p")];
        assert_eq!(
            select_tv_candidate(&mut pack_only, 2, 6, &[], &[])
                .unwrap()
                .candidate
                .torrent_id,
            "pack"
        );

        let mut prioritized = vec![
            candidate_match("low", "Show.S02E03.1080p"),
            candidate_match("high", "Show.S02E03.2160p"),
        ];
        prioritized[0].matched_priority = Some(1);
        prioritized[1].matched_priority = Some(10);
        let rules = vec![super::super::execution::ExecutionTorrentMatchRule {
            name: "quality".to_string(),
            priority: 10,
            mode: super::super::execution::ExecutionTorrentRuleMatchMode::Any,
            title_keywords: vec!["Show".to_string()],
            resolution_keywords: Vec::new(),
            source_keywords: Vec::new(),
        }];
        assert_eq!(
            select_tv_candidate(&mut prioritized, 2, 3, &rules, &[])
                .unwrap()
                .candidate
                .torrent_id,
            "high"
        );
    }

    #[test]
    fn tv_episode_progress_uses_linked_single_and_partial_coverage() {
        let tv = tv_detail();
        let mut payload = super::super::repository::payload::SubscriptionPayload::default();
        payload.artifacts.links = vec![link_artifact(vec![linked_file(1, Some(4))])];

        assert_eq!(first_uncovered_episode(&payload, &tv), Some(5));
        assert!(!tv_complete_after_link(&payload, &tv, &[]));
        assert!(tv_complete_after_link(
            &payload,
            &tv,
            &[linked_file(5, Some(6))]
        ));
    }

    #[test]
    fn file_episode_markers_apply_selected_season_to_seasonless_names() {
        let tv = tv_detail();
        assert_eq!(
            file_episode_marker("Show.[03-06].mkv", Some(&tv)),
            (Some(2), Some(3), Some(6), Some("E03-E06".to_string()))
        );
        assert_eq!(file_episode_marker("Show.S03E01.mkv", Some(&tv)).0, Some(3));
        assert_eq!(
            file_episode_marker("poster.jpg", Some(&tv)),
            (None, None, None, None)
        );
        assert_eq!(
            file_episode_marker("Show.S02E03.srt", Some(&tv)),
            (None, None, None, None)
        );
    }

    #[test]
    fn season_number_parser_handles_english_and_chinese_titles() {
        assert_eq!(season_number_from_title("Show Season 3"), Some(3));
        assert_eq!(season_number_from_title("剧名 第2季"), Some(2));
        assert_eq!(season_number_from_title("剧名 第一季"), Some(1));
        assert_eq!(season_number_from_title("剧名 第十二季"), Some(12));
        assert_eq!(tv_series_search_title("Show Season 3"), "Show");
        assert_eq!(tv_series_search_title("剧名 第一季"), "剧名");
        assert_eq!(season_number_from_title("Show"), None);
    }

    #[test]
    fn movie_metadata_enriches_source_without_losing_subscription_fields() {
        let current = WantedSourcePayload {
            title: "旧标题".to_string(),
            release_year: Some(1999),
            tags: vec!["电影".to_string()],
            douban_sort_time: Some(123),
            ..WantedSourcePayload::default()
        };
        let detail = crate::douban::DoubanSubjectDetail {
            source: "douban",
            media_type: "douban",
            id: "1292052".to_string(),
            subject_id: "1292052".to_string(),
            url: "https://movie.douban.com/subject/1292052/".to_string(),
            title: "肖申克的救赎".to_string(),
            imdb_id: Some("tt0111161".to_string()),
            original_title: "The Shawshank Redemption".to_string(),
            aka: vec!["月黑高飞".to_string()],
            languages: vec!["英语".to_string()],
            countries: vec!["美国".to_string()],
            image: "https://img.test/cover.jpg".to_string(),
            poster_url: "https://img.test/poster.jpg".to_string(),
            directors: vec!["弗兰克·德拉邦特".to_string()],
            writers: vec!["斯蒂芬·金".to_string()],
            actors: vec!["蒂姆·罗宾斯".to_string()],
            genres: vec!["剧情".to_string()],
            date_published: "1994-09-10".to_string(),
            duration: "142分钟".to_string(),
            summary: "希望让人自由。".to_string(),
            rating: crate::douban::DoubanRating {
                value: Some(9.7),
                count: Some(3_000_000),
                info: String::new(),
                star_count: None,
            },
            user_interest: None,
            user_rating: None,
            episodes_count: None,
        };

        let enriched = movie_source_from_detail(&current, detail);

        assert_eq!(enriched.title, "肖申克的救赎");
        assert_eq!(enriched.release_year, Some(1994));
        assert_eq!(
            enriched.original_title.as_deref(),
            Some("The Shawshank Redemption")
        );
        assert_eq!(enriched.summary.as_deref(), Some("希望让人自由。"));
        assert_eq!(enriched.rating_value, Some(9.7));
        assert_eq!(enriched.tags, vec!["电影"]);
        assert_eq!(enriched.douban_sort_time, Some(123));
    }

    #[test]
    fn tv_metadata_enriches_source_without_losing_subscription_fields() {
        let current = WantedSourcePayload {
            title: "旧标题".to_string(),
            release_year: None,
            tags: vec!["电视剧".to_string()],
            douban_sort_time: Some(456),
            ..WantedSourcePayload::default()
        };
        let detail = crate::douban::DoubanSubjectDetail {
            source: "douban",
            media_type: "douban",
            id: "35467152".to_string(),
            subject_id: "35467152".to_string(),
            url: "https://movie.douban.com/subject/35467152/".to_string(),
            title: "测试剧集 第一季".to_string(),
            imdb_id: None,
            original_title: String::new(),
            aka: Vec::new(),
            languages: vec!["汉语普通话".to_string()],
            countries: vec!["中国大陆".to_string()],
            image: "https://img.test/cover.jpg".to_string(),
            poster_url: "https://img.test/poster.jpg".to_string(),
            directors: vec!["导演".to_string()],
            writers: Vec::new(),
            actors: Vec::new(),
            genres: vec!["剧情".to_string()],
            date_published: "2024-01-01".to_string(),
            duration: "45分钟".to_string(),
            summary: "简介".to_string(),
            rating: crate::douban::DoubanRating {
                value: Some(8.1),
                count: Some(1000),
                info: String::new(),
                star_count: None,
            },
            user_interest: None,
            user_rating: None,
            episodes_count: Some(8),
        };

        let enriched = tv_source_from_detail(&current, &detail);

        assert_eq!(enriched.title, "测试剧集 第一季");
        assert_eq!(enriched.release_year, Some(2024));
        assert_eq!(enriched.genres, vec!["剧情"]);
        assert_eq!(enriched.rating_value, Some(8.1));
        assert_eq!(enriched.tags, vec!["电视剧"]);
        assert_eq!(enriched.douban_sort_time, Some(456));
    }

    #[test]
    fn tv_detail_from_douban_builds_correct_payload() {
        let tv = tv_detail_from_douban(8, 1);

        assert_eq!(tv.season_number, 1);
        assert_eq!(tv.episode_total, 8);
        assert_eq!(tv.target_start_episode, 1);
        assert_eq!(tv.target_end_episode, 8);
        assert_eq!(tv.episodes.len(), 8);
        assert_eq!(tv.episodes[0].label, "S01E01");
        assert_eq!(tv.episodes[0].season_number, 1);
        assert_eq!(tv.episodes[0].episode_number, 1);
        assert_eq!(tv.episodes[7].label, "S01E08");
    }

    #[test]
    fn tv_detail_from_douban_handles_different_season() {
        let tv = tv_detail_from_douban(6, 2);

        assert_eq!(tv.season_number, 2);
        assert_eq!(tv.episode_total, 6);
        assert_eq!(tv.episodes[0].label, "S02E01");
    }

    #[test]
    fn season_number_from_title_parses_tv_titles() {
        assert_eq!(season_number_from_title("测试剧集 第一季"), Some(1));
        assert_eq!(season_number_from_title("测试剧集 第二季"), Some(2));
        assert_eq!(season_number_from_title("测试剧集 第十季"), Some(10));
        assert_eq!(season_number_from_title("测试剧集 第十二季"), Some(12));
        assert_eq!(season_number_from_title("Show Season 1"), Some(1));
        assert_eq!(season_number_from_title("Show Season.3"), Some(3));
        assert_eq!(season_number_from_title("测试剧集"), None);
    }
}
