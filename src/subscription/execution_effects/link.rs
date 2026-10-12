use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use super::super::effect_adapters::HardlinkFileStatus;
use super::super::effects::LinkFileOutcome;
use super::super::execution::ExecutionCategory;
use super::super::repository::payload::{DownloadArtifactPayload, SubscriptionPayload};
use crate::clients::qbittorrent::QbTorrentFile;

#[derive(Debug)]
pub(super) struct LinkPlan {
    pub(super) source_path: PathBuf,
    pub(super) target_path: PathBuf,
    pub(super) size: u64,
    pub(super) persisted_outcome: LinkFileOutcome,
}

pub(super) fn plans(
    payload: &SubscriptionPayload,
    category: &ExecutionCategory,
    download: &DownloadArtifactPayload,
    files: &[QbTorrentFile],
) -> Result<Vec<LinkPlan>, String> {
    let directory_title = payload
        .tv
        .as_ref()
        .map(|tv| tv.localized_title.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(&payload.source.title);
    let title = safe_title_component(directory_title);
    if title.is_empty() {
        return Err("subscription title cannot form a safe link directory".to_string());
    }
    let year = payload
        .source
        .release_year
        .ok_or_else(|| "subscription release year is required for link layout".to_string())?;
    let source_root = PathBuf::from(category.download_dir.trim());
    let target_root = link_target_root(
        category,
        &title,
        year,
        payload.tv.as_ref().map(|tv| tv.season_number),
    );
    if source_root.as_os_str().is_empty() || target_root.as_os_str().is_empty() {
        return Err("subscription category link paths are empty".to_string());
    }
    let existing_link = payload
        .artifacts
        .links
        .iter()
        .find(|link| link.download.artifact_id == download.idempotency_key);
    let previous = existing_link
        .into_iter()
        .flat_map(|link| link.files.iter())
        .map(|file| (file.target_path.as_str(), file.outcome))
        .collect::<BTreeMap<_, _>>();
    let plans = files
        .iter()
        .filter_map(|file| {
            safe_relative_path(&file.name).map(|relative| {
                let source_path = source_root.join(&relative);
                let target_path = target_root.join(relative);
                let persisted_outcome = previous
                    .get(target_path.to_string_lossy().as_ref())
                    .copied()
                    .unwrap_or(LinkFileOutcome::Pending);
                LinkPlan {
                    source_path,
                    target_path,
                    size: file.size,
                    persisted_outcome,
                }
            })
        })
        .collect::<Vec<_>>();
    if plans.is_empty() {
        return Err("qB file list has no safe relative files to link".to_string());
    }
    Ok(plans)
}

fn link_target_root(
    category: &ExecutionCategory,
    title: &str,
    year: u16,
    season_number: Option<u32>,
) -> PathBuf {
    let root = PathBuf::from(category.link_target_dir.trim()).join(format!("{title}.{year}"));
    season_number
        .map(|season| root.join(format!("Season {season:02}")))
        .unwrap_or(root)
}

pub(super) fn error(status: &HardlinkFileStatus) -> Option<String> {
    match status {
        HardlinkFileStatus::Created
        | HardlinkFileStatus::SkippedVerified
        | HardlinkFileStatus::AcceptedExisting => None,
        HardlinkFileStatus::Missing => Some("source file is missing".to_string()),
        HardlinkFileStatus::Conflict { .. } => {
            Some("deterministic target belongs to another file".to_string())
        }
        HardlinkFileStatus::Failed(failure) => Some(format!(
            "filesystem {:?} failed: {:?}",
            failure.operation(),
            failure.kind()
        )),
    }
}

fn safe_relative_path(value: &str) -> Option<PathBuf> {
    let path = Path::new(value.trim());
    if path.as_os_str().is_empty() || path.is_absolute() {
        return None;
    }
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => safe.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    (!safe.as_os_str().is_empty()).then_some(safe)
}

fn safe_title_component(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter_map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => Some(' '),
            character if character.is_control() => None,
            character => Some(character),
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_rejects_traversal_and_sanitizes_title_components() {
        assert!(safe_relative_path("../escape.mkv").is_none());
        assert!(safe_relative_path("/absolute/movie.mkv").is_none());
        assert_eq!(
            safe_relative_path("Movie/feature.mkv").unwrap(),
            PathBuf::from("Movie/feature.mkv")
        );
        assert_eq!(safe_title_component("电影 / A:B?"), "电影 A B");
    }

    #[test]
    fn tv_layout_adds_a_stable_season_directory() {
        let category = ExecutionCategory {
            name: "TV".to_string(),
            wanted_tag: "电视剧".to_string(),
            qb_server_id: "qb".to_string(),
            qb_category: "tv".to_string(),
            qb_save_dir_name: "tv".to_string(),
            download_dir: "D:/downloads".to_string(),
            link_target_dir: "D:/media/TV".to_string(),
        };

        assert_eq!(
            link_target_root(&category, "Show", 2026, Some(2)),
            PathBuf::from("D:/media/TV/Show.2026/Season 02")
        );
        assert_eq!(
            link_target_root(&category, "Movie", 2026, None),
            PathBuf::from("D:/media/TV/Movie.2026")
        );
    }

    #[test]
    fn tv_link_directory_prefers_the_localized_api_title() {
        let category = ExecutionCategory {
            name: "TV".to_string(),
            wanted_tag: "电视剧".to_string(),
            qb_server_id: "qb".to_string(),
            qb_category: "tv".to_string(),
            qb_save_dir_name: "tv".to_string(),
            download_dir: "D:/downloads".to_string(),
            link_target_dir: "D:/media/TV".to_string(),
        };
        let payload = SubscriptionPayload {
            source: super::super::super::repository::payload::WantedSourcePayload {
                title: "豆瓣返回的超级长电视剧标题 Season 2".to_string(),
                release_year: Some(2026),
                ..Default::default()
            },
            tv: Some(super::super::super::repository::payload::TvDetailPayload {
                localized_title: "中文剧名".to_string(),
                season_number: 2,
                ..Default::default()
            }),
            ..Default::default()
        };
        let download = DownloadArtifactPayload {
            idempotency_key: "download:v1:test".to_string(),
            torrent_id: "torrent".to_string(),
            torrent_title: "fixture".to_string(),
            qb_server_id: "qb".to_string(),
            qb_server_name: None,
            qb_category: "tv".to_string(),
            qb_save_dir_name: "tv".to_string(),
            qb_identifier: None,
            qb_hash: None,
            qb_name: None,
            qb_state: None,
            torrent_download_url: None,
            mteam_torrent_url: None,
            state:
                super::super::super::repository::payload::DownloadArtifactStatePayload::Downloaded,
            progress: Some(1.0),
            total_size: Some(1),
            files: Vec::new(),
            pushed_at: Some(1),
            checked_at: Some(1),
            completed_at: Some(1),
        };
        let files = vec![QbTorrentFile {
            name: "S02E01.mkv".to_string(),
            size: 1,
            progress: 1.0,
            priority: 1,
        }];

        let plans = plans(&payload, &category, &download, &files).unwrap();

        assert_eq!(
            plans[0].target_path,
            PathBuf::from("D:/media/TV/中文剧名.2026/Season 02/S02E01.mkv")
        );
    }
}
