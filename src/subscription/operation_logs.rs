use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationLogEntry {
    pub id: u64,
    #[serde(default)]
    pub account_key: String,
    pub created_at: u64,
    pub category: String,
    pub action: String,
    pub target_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_title: Option<String>,
    pub status: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub related: Value,
}

#[derive(Debug, Clone)]
pub struct NewOperationLogEntry {
    pub account_key: String,
    pub created_at: u64,
    pub category: String,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub target_title: Option<String>,
    pub status: String,
    pub summary: String,
    pub error: Option<String>,
    pub related: Value,
}

#[derive(Debug, Clone, Default)]
pub struct OperationLogQuery {
    pub account_key: Option<String>,
    pub category: Option<String>,
    pub status: Option<String>,
    pub q: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OperationLogPage {
    pub items: Vec<OperationLogEntry>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub has_more: bool,
}
