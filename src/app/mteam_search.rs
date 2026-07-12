use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::audit::{operation_log_entry, AuditLogPort, OperationLogEvent};
use crate::clients::http::ClientError;
use crate::clients::mteam::MteamClient;
use crate::config::{ConfigManager, FileConfig};
use crate::douban;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TorrentSearchSource {
    Imdb,
    Douban,
    Keyword,
}

impl TorrentSearchSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Imdb => "imdb",
            Self::Douban => "douban",
            Self::Keyword => "keyword",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MteamSearchCommand {
    pub(crate) source: TorrentSearchSource,
    pub(crate) query: String,
    pub(crate) page: u32,
    pub(crate) page_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MteamTorrent {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) small_description: Option<String>,
    pub(crate) size: Option<u64>,
    pub(crate) seeders: Option<u64>,
    pub(crate) leechers: Option<u64>,
    pub(crate) created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MteamSearchOutcome {
    pub(crate) items: Vec<MteamTorrent>,
    pub(crate) page: u32,
    pub(crate) page_size: u32,
}

#[derive(Debug)]
pub(crate) enum MteamSearchError {
    Validation { message: String },
    Upstream(ClientError),
}

impl MteamSearchError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }
}

pub(crate) type ProviderFuture =
    Pin<Box<dyn Future<Output = Result<Value, ClientError>> + Send + 'static>>;

pub(crate) trait MteamSearchProvider: Send + Sync {
    fn search(&self, api_key: String, body: Value) -> ProviderFuture;
}

#[derive(Clone)]
struct LiveMteamSearchProvider {
    client: MteamClient,
}

impl MteamSearchProvider for LiveMteamSearchProvider {
    fn search(&self, api_key: String, body: Value) -> ProviderFuture {
        let client = self.client.clone();
        Box::pin(async move { client.search(&api_key, &body).await })
    }
}

#[derive(Clone)]
pub(crate) struct MteamSearchService {
    config: ConfigManager,
    provider: Arc<dyn MteamSearchProvider>,
    audit: Arc<dyn AuditLogPort>,
}

impl MteamSearchService {
    pub(crate) fn new(
        config: ConfigManager,
        mteam: MteamClient,
        audit: Arc<dyn AuditLogPort>,
    ) -> Self {
        Self {
            config,
            provider: Arc::new(LiveMteamSearchProvider { client: mteam }),
            audit,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_provider(
        config: ConfigManager,
        provider: Arc<dyn MteamSearchProvider>,
        audit: Arc<dyn AuditLogPort>,
    ) -> Self {
        Self {
            config,
            provider,
            audit,
        }
    }

    pub(crate) async fn search(
        &self,
        command: MteamSearchCommand,
    ) -> Result<MteamSearchOutcome, MteamSearchError> {
        let config = self.config.snapshot().await.value;
        let source = command.source.as_str();
        let page = command.page.max(1);
        let page_size = command.page_size.clamp(1, 100);
        let query = command.query.trim();
        let api_key = config.mteam_api_key.trim();

        if api_key.is_empty() {
            self.record(
                &config,
                source,
                query,
                page,
                page_size,
                "failed",
                "M-Team 种子搜索失败：缺少 API Key",
                Some("请在设置中填写 M-Team API Key".to_string()),
                None,
            )
            .await;
            return Err(MteamSearchError::validation(
                "请在设置中填写 M-Team API Key（控制面板中的 OpenAPI 密钥）",
            ));
        }
        if query.is_empty() {
            let message = match command.source {
                TorrentSearchSource::Imdb => "使用 IMDb 路径时请提供有效的 imdb_id",
                TorrentSearchSource::Douban => "使用豆瓣路径时请提供有效的 douban_id",
                TorrentSearchSource::Keyword => "使用关键字路径时请提供 keyword",
            };
            self.record(
                &config,
                source,
                "",
                page,
                page_size,
                "failed",
                "M-Team 种子搜索失败：查询条件为空",
                Some(message.to_string()),
                None,
            )
            .await;
            return Err(MteamSearchError::validation(message));
        }

        let provider_query = match command.source {
            TorrentSearchSource::Imdb => normalize_imdb_url(query),
            TorrentSearchSource::Douban => normalize_douban_url(query),
            TorrentSearchSource::Keyword => query.to_string(),
        };
        let response = match self
            .provider
            .search(
                api_key.to_string(),
                search_body(page, page_size, source, &provider_query),
            )
            .await
        {
            Ok(response) => response,
            Err(error) => {
                self.record(
                    &config,
                    source,
                    query,
                    page,
                    page_size,
                    "failed",
                    "M-Team 种子搜索失败",
                    Some(error.to_string()),
                    None,
                )
                .await;
                return Err(MteamSearchError::Upstream(error));
            }
        };

        let items = extract_torrents(&response);
        self.record(
            &config,
            source,
            query,
            page,
            page_size,
            "success",
            "M-Team 种子搜索完成",
            None,
            Some(items.len()),
        )
        .await;
        Ok(MteamSearchOutcome {
            items,
            page,
            page_size,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn record(
        &self,
        config: &FileConfig,
        source: &str,
        query: &str,
        page: u32,
        page_size: u32,
        status: &'static str,
        summary: &'static str,
        error: Option<String>,
        candidate_count: Option<usize>,
    ) {
        let account_key = douban::auth_cache_key_fragment(&config.douban_cookie)
            .unwrap_or_else(|_| "system".to_string());
        let summary = candidate_count
            .map(|count| format!("{summary}：{count} 条候选"))
            .unwrap_or_else(|| summary.to_string());
        let entry = operation_log_entry(
            account_key,
            OperationLogEvent {
                category: "torrent_search",
                action: "search_torrents",
                target_type: "mteam",
                target_id: None,
                target_title: (!query.is_empty()).then(|| query.to_string()),
                status,
                summary,
                error,
                related: json!({
                    "source": source,
                    "candidate_count": candidate_count,
                    "page": page,
                    "page_size": page_size,
                }),
            },
        );
        if let Err(error) = self.audit.append(entry).await {
            tracing::warn!("operation log write failed: {error}");
        }
    }
}

fn search_body(page: u32, page_size: u32, field: &str, value: &str) -> Value {
    let mut body = Map::new();
    body.insert("pageNumber".to_string(), json!(page));
    body.insert("pageSize".to_string(), json!(page_size));
    body.insert("sortField".to_string(), json!("SEEDERS"));
    body.insert("sortDirection".to_string(), json!("DESC"));
    body.insert(field.to_string(), json!(value));
    Value::Object(body)
}

fn extract_torrents(value: &Value) -> Vec<MteamTorrent> {
    let mut candidates = Vec::new();
    collect_candidate_objects(value, &mut candidates);
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter_map(parse_torrent)
        .filter(|torrent| seen.insert(torrent.id.clone()))
        .collect()
}

fn collect_candidate_objects<'a>(value: &'a Value, output: &mut Vec<&'a Map<String, Value>>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_candidate_objects(item, output);
            }
        }
        Value::Object(map) => {
            let has_id = ["id", "torrentId", "torrent_id", "tid"]
                .iter()
                .any(|key| map.contains_key(*key));
            let has_title = ["name", "title", "smallDescr", "small_descr"]
                .iter()
                .any(|key| map.contains_key(*key));
            if has_id && has_title {
                output.push(map);
            } else {
                for nested in map.values() {
                    collect_candidate_objects(nested, output);
                }
            }
        }
        _ => {}
    }
}

fn parse_torrent(map: &Map<String, Value>) -> Option<MteamTorrent> {
    let id = string_alias(map, &["id", "torrentId", "torrent_id", "tid"])?;
    let name = string_alias(map, &["name", "title", "smallDescr", "small_descr"])?;
    let status = map.get("status").and_then(Value::as_object);
    Some(MteamTorrent {
        id,
        name,
        small_description: string_alias(map, &["smallDescr", "small_descr", "description"]),
        size: u64_alias(map, &["size", "totalSize", "total_size"]),
        seeders: status
            .and_then(|status| u64_alias(status, &["seeders"]))
            .or_else(|| u64_alias(map, &["seeders"])),
        leechers: status
            .and_then(|status| u64_alias(status, &["leechers"]))
            .or_else(|| u64_alias(map, &["leechers"])),
        created_at: string_alias(map, &["createdDate", "created_date", "createdAt"]),
    })
}

fn string_alias(map: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = map.get(*key)?;
        let value = match value {
            Value::String(value) => value.trim().to_string(),
            Value::Number(value) => value.to_string(),
            _ => return None,
        };
        (!value.is_empty()).then_some(value)
    })
}

fn u64_alias(map: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| match map.get(*key)? {
        Value::Number(value) => value.as_u64(),
        Value::String(value) => value.trim().parse().ok(),
        _ => None,
    })
}

fn normalize_imdb_url(value: &str) -> String {
    let value = value.trim();
    if value.starts_with("http://") || value.starts_with("https://") {
        return value.to_string();
    }
    let id = if value.starts_with("tt") {
        value.to_string()
    } else {
        format!("tt{value}")
    };
    format!("https://www.imdb.com/title/{id}/")
}

fn normalize_douban_url(value: &str) -> String {
    let value = value.trim();
    if value.starts_with("http://") || value.starts_with("https://") {
        return format!("{}/", value.trim_end_matches('/').trim());
    }
    let tail = value
        .rsplit('/')
        .next()
        .unwrap_or(value)
        .trim()
        .trim_start_matches("subject/");
    format!("https://movie.douban.com/subject/{tail}/")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::app::audit::AuditLogFuture;
    use crate::subscription::NewOperationLogEntry;

    #[derive(Default)]
    struct FakeProvider {
        calls: Mutex<Vec<(String, Value)>>,
        result: Mutex<Option<Result<Value, ClientError>>>,
    }

    impl MteamSearchProvider for FakeProvider {
        fn search(&self, api_key: String, body: Value) -> ProviderFuture {
            self.calls.lock().unwrap().push((api_key, body));
            let result = self.result.lock().unwrap().take().unwrap();
            Box::pin(async move { result })
        }
    }

    #[derive(Default)]
    struct RecordingAudit {
        entries: Mutex<Vec<NewOperationLogEntry>>,
    }

    impl AuditLogPort for RecordingAudit {
        fn append(&self, entry: NewOperationLogEntry) -> AuditLogFuture {
            self.entries.lock().unwrap().push(entry);
            Box::pin(async { Ok(()) })
        }
    }

    fn service(
        label: &str,
        provider: Arc<FakeProvider>,
        audit: Arc<RecordingAudit>,
    ) -> MteamSearchService {
        let root =
            std::env::temp_dir().join(format!("mteam-search-{label}-{}", std::process::id()));
        let config = FileConfig {
            mteam_api_key: "fixture-api-key".to_string(),
            douban_cookie: "dbcl2=fixture-account:secret; ck=test".to_string(),
            ..FileConfig::default()
        };
        MteamSearchService::with_provider(
            ConfigManager::new(root.join("config.toml"), config),
            provider,
            audit,
        )
    }

    #[test]
    fn provider_shapes_are_normalized_to_one_stable_candidate_contract() {
        let response = json!({
            "data": { "items": [
                {
                    "torrentId": 42,
                    "title": "one",
                    "smallDescr": "UHD",
                    "size": "4096",
                    "status": { "seeders": "8", "leechers": 2 },
                    "createdDate": "2026-07-12"
                },
                { "id": "42", "name": "duplicate" },
                { "message": "not a torrent" }
            ]}
        });

        assert_eq!(
            extract_torrents(&response),
            vec![MteamTorrent {
                id: "42".to_string(),
                name: "one".to_string(),
                small_description: Some("UHD".to_string()),
                size: Some(4096),
                seeders: Some(8),
                leechers: Some(2),
                created_at: Some("2026-07-12".to_string()),
            }]
        );
    }

    #[test]
    fn provider_request_normalization_is_owned_by_the_service() {
        let body = search_body(2, 50, "keyword", "测试电影");
        assert_eq!(body["pageNumber"], 2);
        assert_eq!(body["pageSize"], 50);
        assert_eq!(body["sortField"], "SEEDERS");
        assert_eq!(body["sortDirection"], "DESC");
        assert_eq!(body["keyword"], "测试电影");
        assert_eq!(
            normalize_imdb_url("123"),
            "https://www.imdb.com/title/tt123/"
        );
        assert_eq!(
            normalize_douban_url("123"),
            "https://movie.douban.com/subject/123/"
        );
    }

    #[tokio::test]
    async fn fake_provider_success_is_normalized_and_audited() {
        let provider = Arc::new(FakeProvider::default());
        *provider.result.lock().unwrap() = Some(Ok(json!({
            "data": { "items": [
                {
                    "torrentId": 42,
                    "title": "Fixture.Release.2160p",
                    "smallDescr": "UHD",
                    "size": "4096",
                    "status": { "seeders": "8", "leechers": 2 },
                    "createdDate": "2026-07-12"
                },
                { "id": "42", "name": "duplicate" }
            ]}
        })));
        let audit = Arc::new(RecordingAudit::default());
        let service = service("success", provider.clone(), audit.clone());

        let outcome = service
            .search(MteamSearchCommand {
                source: TorrentSearchSource::Imdb,
                query: " 123 ".to_string(),
                page: 0,
                page_size: 999,
            })
            .await
            .unwrap();

        assert_eq!(outcome.page, 1);
        assert_eq!(outcome.page_size, 100);
        assert_eq!(
            outcome.items,
            vec![MteamTorrent {
                id: "42".to_string(),
                name: "Fixture.Release.2160p".to_string(),
                small_description: Some("UHD".to_string()),
                size: Some(4096),
                seeders: Some(8),
                leechers: Some(2),
                created_at: Some("2026-07-12".to_string()),
            }]
        );

        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "fixture-api-key");
        assert_eq!(calls[0].1["pageNumber"], 1);
        assert_eq!(calls[0].1["pageSize"], 100);
        assert_eq!(calls[0].1["imdb"], "https://www.imdb.com/title/tt123/");
        drop(calls);

        let entries = audit.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "success");
        assert_eq!(entries[0].target_title.as_deref(), Some("123"));
        assert_eq!(entries[0].related["candidate_count"], 1);
        assert_eq!(entries[0].related["page"], 1);
        assert_eq!(entries[0].related["page_size"], 100);
    }

    #[tokio::test]
    async fn fake_provider_failure_is_returned_and_audited() {
        let provider = Arc::new(FakeProvider::default());
        *provider.result.lock().unwrap() = Some(Err(ClientError::unavailable(
            "M-Team",
            "fixture provider unavailable",
        )));
        let audit = Arc::new(RecordingAudit::default());
        let service = service("failure", provider.clone(), audit.clone());

        let error = service
            .search(MteamSearchCommand {
                source: TorrentSearchSource::Keyword,
                query: " fixture movie ".to_string(),
                page: 2,
                page_size: 50,
            })
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            MteamSearchError::Upstream(ClientError::Unavailable {
                provider: "M-Team",
                ..
            })
        ));
        let calls = provider.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].1["keyword"], "fixture movie");
        drop(calls);

        let entries = audit.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "failed");
        assert_eq!(entries[0].target_title.as_deref(), Some("fixture movie"));
        assert!(entries[0]
            .error
            .as_deref()
            .is_some_and(|message| message.contains("fixture provider unavailable")));
        assert_eq!(entries[0].related["source"], "keyword");
        assert_eq!(entries[0].related["candidate_count"], Value::Null);
    }
}
