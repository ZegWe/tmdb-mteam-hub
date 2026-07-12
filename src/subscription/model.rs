use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionLifecycleState {
    #[default]
    Queued,
    Meta,
    Searching,
    Downloading,
    Linking,
    Completed,
}

impl SubscriptionLifecycleState {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionExecutionState {
    #[default]
    Idle,
    Running,
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionMediaKind {
    #[default]
    Movie,
    Tv,
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

#[cfg(test)]
mod tests {
    use super::{
        SubscriptionAttentionTag, SubscriptionExecutionState, SubscriptionLifecycleState,
        SubscriptionMediaKind,
    };

    #[test]
    fn serialized_labels_are_stable() {
        let cases = [
            serde_json::to_value(SubscriptionLifecycleState::Downloading).unwrap(),
            serde_json::to_value(SubscriptionExecutionState::Running).unwrap(),
            serde_json::to_value(SubscriptionAttentionTag::NeedsReconciliation).unwrap(),
            serde_json::to_value(SubscriptionMediaKind::Tv).unwrap(),
        ];
        assert_eq!(
            cases,
            ["downloading", "running", "needs_reconciliation", "tv"]
        );
    }

    #[test]
    fn media_kind_is_derived_from_current_tags() {
        assert_eq!(
            SubscriptionMediaKind::from_tags(&[" TV ".to_string()]),
            SubscriptionMediaKind::Tv
        );
        assert_eq!(
            SubscriptionMediaKind::from_tags(&["电影".to_string()]),
            SubscriptionMediaKind::Movie
        );
    }
}
