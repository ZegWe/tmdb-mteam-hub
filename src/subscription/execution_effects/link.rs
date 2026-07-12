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
    let title = safe_title_component(&payload.source.title);
    if title.is_empty() {
        return Err("subscription title cannot form a safe link directory".to_string());
    }
    let year = payload
        .source
        .release_year
        .ok_or_else(|| "subscription release year is required for link layout".to_string())?;
    let source_root = PathBuf::from(category.download_dir.trim());
    let target_root =
        PathBuf::from(category.link_target_dir.trim()).join(format!("{title}.{year}"));
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
}
