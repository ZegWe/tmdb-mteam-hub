use serde::Serialize;
use serde_json::Value;

use crate::subscription::{OperationLogEntry, OperationLogPage};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct OperationLogPageDto {
    items: Vec<OperationLogEntryDto>,
    page: u32,
    page_size: u32,
    total: u64,
    has_more: bool,
}

impl From<OperationLogPage> for OperationLogPageDto {
    fn from(value: OperationLogPage) -> Self {
        Self {
            items: value.items.into_iter().map(Into::into).collect(),
            page: value.page,
            page_size: value.page_size,
            total: value.total,
            has_more: value.has_more,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct OperationLogEntryDto {
    id: u64,
    created_at: u64,
    category: String,
    action: String,
    target_type: String,
    target_id: Option<String>,
    target_title: Option<String>,
    status: String,
    summary: String,
    error: Option<String>,
    related: OperationLogRelatedDto,
}

impl From<OperationLogEntry> for OperationLogEntryDto {
    fn from(value: OperationLogEntry) -> Self {
        Self {
            id: value.id,
            created_at: value.created_at,
            category: value.category,
            action: value.action,
            target_type: value.target_type,
            target_id: value.target_id,
            target_title: value.target_title,
            status: value.status,
            summary: value.summary,
            error: value.error,
            related: OperationLogRelatedDto::from_value(value.related),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
struct OperationLogRelatedDto {
    fields: Vec<OperationLogRelatedFieldDto>,
    torrent_matches: Vec<OperationLogTorrentMatchDto>,
}

impl OperationLogRelatedDto {
    fn from_value(value: Value) -> Self {
        let Some(object) = value.as_object() else {
            return Self::default();
        };
        let fields = object
            .iter()
            .filter(|(key, _)| key.as_str() != "torrent_matches")
            .filter_map(|(key, value)| scalar_text(value).map(|value| (key, value)))
            .map(|(key, value)| OperationLogRelatedFieldDto {
                key: key.clone(),
                value,
            })
            .collect();
        let torrent_matches = object
            .get("torrent_matches")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(OperationLogTorrentMatchDto::from_value)
            .collect();
        Self {
            fields,
            torrent_matches,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct OperationLogRelatedFieldDto {
    key: String,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct OperationLogTorrentMatchDto {
    torrent_id: String,
    seeders: Option<u64>,
    leechers: Option<u64>,
    size: Option<String>,
    uploaded_at: Option<String>,
    matched_keywords: Vec<String>,
    rule_evaluations: Vec<OperationLogRuleEvaluationDto>,
}

impl OperationLogTorrentMatchDto {
    fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let torrent_id = object.get("torrent_id")?.as_str()?.trim();
        if torrent_id.is_empty() {
            return None;
        }
        Some(Self {
            torrent_id: torrent_id.to_string(),
            seeders: object.get("seeders").and_then(Value::as_u64),
            leechers: object.get("leechers").and_then(Value::as_u64),
            size: optional_string(object.get("size")),
            uploaded_at: optional_string(object.get("uploaded_at")),
            matched_keywords: string_array(object.get("matched_keywords")),
            rule_evaluations: object
                .get("rule_evaluations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(OperationLogRuleEvaluationDto::from_value)
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct OperationLogRuleEvaluationDto {
    rule_name: String,
    matched: bool,
    matched_keywords: Vec<String>,
    missing_keywords: Vec<String>,
    excluded_reason: Option<String>,
}

impl OperationLogRuleEvaluationDto {
    fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        Some(Self {
            rule_name: object
                .get("rule_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            matched: object
                .get("matched")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            matched_keywords: string_array(object.get("matched_keywords")),
            missing_keywords: string_array(object.get("missing_keywords")),
            excluded_reason: optional_string(object.get("excluded_reason")),
        })
    }
}

fn scalar_text(value: &Value) -> Option<String> {
    match value {
        Value::Null | Value::Array(_) | Value::Object(_) => None,
        Value::String(value) if value.trim().is_empty() => None,
        Value::String(value) => Some(value.clone()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
    }
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn log_dto_removes_account_identity_and_closes_related_payload() {
        let value = serde_json::to_value(OperationLogPageDto::from(OperationLogPage {
            items: vec![OperationLogEntry {
                id: 7,
                account_key: "account-secret".to_string(),
                created_at: 10,
                category: "torrent_search".to_string(),
                action: "match_candidates".to_string(),
                target_type: "subscription".to_string(),
                target_id: Some("subject-1".to_string()),
                target_title: None,
                status: "success".to_string(),
                summary: "matched".to_string(),
                error: None,
                related: json!({
                    "candidate_count": 3,
                    "nested_internal": { "secret": "must-not-cross" },
                    "torrent_matches": [{
                        "torrent_id": "torrent-1",
                        "seeders": 8,
                        "unknown_provider_field": "ignored",
                        "matched_keywords": ["1080p"]
                    }]
                }),
            }],
            page: 1,
            page_size: 30,
            total: 1,
            has_more: false,
        }))
        .unwrap();

        let entry = &value["items"][0];
        assert!(entry.get("account_key").is_none());
        assert_eq!(entry["related"]["fields"][0]["key"], "candidate_count");
        assert_eq!(entry["related"]["fields"][0]["value"], "3");
        assert_eq!(
            entry["related"]["torrent_matches"][0]["torrent_id"],
            "torrent-1"
        );
        assert!(!value.to_string().contains("account-secret"));
        assert!(!value.to_string().contains("must-not-cross"));
        assert!(!value.to_string().contains("unknown_provider_field"));
    }
}
