use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::json;

use super::audit::{operation_log_entry, AuditLogPort, OperationLogEvent};
use crate::clients::http::ClientError;
use crate::clients::mteam::MteamClient;
use crate::clients::qbittorrent;
use crate::config::{ConfigManager, FileConfig, QbServerEntry};
use crate::douban;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualQbPushCommand {
    pub(crate) server_id: String,
    pub(crate) torrent_id: String,
    pub(crate) category: Option<String>,
    pub(crate) savepath: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ManualQbPushOutcome {
    pub(crate) added: bool,
}

#[derive(Debug)]
pub(crate) enum ManualQbError {
    Validation { message: String },
    Upstream(ClientError),
}

impl ManualQbError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }
}

pub(crate) type ManualQbPortFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, ClientError>> + Send + 'static>>;

pub(crate) trait ManualQbPort: Send + Sync {
    fn test_connection(&self, server: QbServerEntry) -> ManualQbPortFuture<String>;

    fn fetch_mteam_download_url(
        &self,
        api_key: String,
        torrent_id: String,
    ) -> ManualQbPortFuture<String>;

    fn add_torrent_from_url(
        &self,
        server: QbServerEntry,
        download_url: String,
        category: Option<String>,
        savepath: Option<String>,
    ) -> ManualQbPortFuture<()>;
}

struct LiveManualQbPort {
    mteam: MteamClient,
}

impl LiveManualQbPort {
    fn new(mteam: MteamClient) -> Self {
        Self { mteam }
    }
}

impl ManualQbPort for LiveManualQbPort {
    fn test_connection(&self, server: QbServerEntry) -> ManualQbPortFuture<String> {
        Box::pin(async move { qbittorrent::test_connection(&server).await })
    }

    fn fetch_mteam_download_url(
        &self,
        api_key: String,
        torrent_id: String,
    ) -> ManualQbPortFuture<String> {
        let mteam = self.mteam.clone();
        Box::pin(async move { mteam.fetch_download_url(&api_key, &torrent_id).await })
    }

    fn add_torrent_from_url(
        &self,
        server: QbServerEntry,
        download_url: String,
        category: Option<String>,
        savepath: Option<String>,
    ) -> ManualQbPortFuture<()> {
        Box::pin(async move {
            qbittorrent::add_torrent_from_url(
                &server,
                &download_url,
                category.as_deref(),
                savepath.as_deref(),
            )
            .await
        })
    }
}

#[derive(Clone)]
pub(crate) struct ManualQbService {
    config: ConfigManager,
    port: Arc<dyn ManualQbPort>,
    audit: Arc<dyn AuditLogPort>,
}

impl ManualQbService {
    pub(crate) fn new(
        config: ConfigManager,
        mteam: MteamClient,
        audit: Arc<dyn AuditLogPort>,
    ) -> Self {
        Self {
            config,
            port: Arc::new(LiveManualQbPort::new(mteam)),
            audit,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_port(
        config: ConfigManager,
        port: Arc<dyn ManualQbPort>,
        audit: Arc<dyn AuditLogPort>,
    ) -> Self {
        Self {
            config,
            port,
            audit,
        }
    }

    pub(crate) async fn test_connection(&self, server_id: String) -> Result<String, ManualQbError> {
        let config = self.config.snapshot().await.value;
        let server = resolve_configured_server(&config.qb_servers, &server_id)?;
        self.port
            .test_connection(server)
            .await
            .map_err(ManualQbError::Upstream)
    }

    pub(crate) async fn push_mteam(
        &self,
        command: ManualQbPushCommand,
    ) -> Result<ManualQbPushOutcome, ManualQbError> {
        let config = self.config.snapshot().await.value;
        let server = resolve_configured_server(&config.qb_servers, &command.server_id)?;
        let torrent_id = command.torrent_id.trim();
        if torrent_id.is_empty() {
            return Err(ManualQbError::validation("M-Team torrent_id 不能为空"));
        }
        let mteam_key = config.mteam_api_key.trim();
        if mteam_key.is_empty() {
            self.record(
                &config,
                &server,
                &command,
                "failed",
                "手动推送种子到 qB 失败：缺少 M-Team API Key",
                Some("请先在设置中填写 M-Team OpenAPI Key".to_string()),
            )
            .await;
            return Err(ManualQbError::validation(
                "请先在设置中填写 M-Team OpenAPI Key（用于向 qB 换取可下载链接）",
            ));
        }

        let download_url = match self
            .port
            .fetch_mteam_download_url(mteam_key.to_string(), torrent_id.to_string())
            .await
        {
            Ok(url) => url,
            Err(error) => {
                self.record(
                    &config,
                    &server,
                    &command,
                    "failed",
                    "手动推送种子到 qB 失败：M-Team 取链失败",
                    Some(error.to_string()),
                )
                .await;
                return Err(ManualQbError::Upstream(error));
            }
        };

        if let Err(error) = self
            .port
            .add_torrent_from_url(
                server.clone(),
                download_url,
                command.category.clone(),
                command.savepath.clone(),
            )
            .await
        {
            self.record(
                &config,
                &server,
                &command,
                "failed",
                "手动推送种子到 qB 失败：qB 添加种子失败",
                Some(error.to_string()),
            )
            .await;
            return Err(ManualQbError::Upstream(error));
        }

        self.record(
            &config,
            &server,
            &command,
            "success",
            "已手动推送种子到 qB",
            None,
        )
        .await;
        Ok(ManualQbPushOutcome { added: true })
    }

    async fn record(
        &self,
        config: &FileConfig,
        server: &QbServerEntry,
        command: &ManualQbPushCommand,
        status: &'static str,
        summary: &'static str,
        error: Option<String>,
    ) {
        let account_key = douban::auth_cache_key_fragment(&config.douban_cookie)
            .unwrap_or_else(|_| "system".to_string());
        let entry = operation_log_entry(
            account_key,
            OperationLogEvent {
                category: "qb_push",
                action: "manual_push_torrent",
                target_type: "torrent",
                target_id: Some(command.torrent_id.clone()),
                target_title: None,
                status,
                summary,
                error,
                related: json!({
                    "torrent_id": command.torrent_id,
                    "qb_server_id": server.id,
                    "qb_server": server.name,
                    "qb_category": command.category,
                    "savepath": command.savepath,
                }),
            },
        );
        if let Err(error) = self.audit.append(entry).await {
            tracing::warn!("operation log write failed: {error}");
        }
    }
}

fn resolve_configured_server(
    servers: &[QbServerEntry],
    requested_id: &str,
) -> Result<QbServerEntry, ManualQbError> {
    let id = requested_id.trim();
    if id.is_empty() {
        return Err(ManualQbError::validation("qB server_id 不能为空"));
    }
    let mut matches = servers.iter().filter(|server| server.id.trim() == id);
    let server = matches
        .next()
        .ok_or_else(|| ManualQbError::validation("qB server_id 未配置或已删除"))?;
    if matches.next().is_some() {
        return Err(ManualQbError::validation("配置中存在重复的 qB server_id"));
    }
    Ok(server.clone())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::super::audit::AuditLogFuture;
    use crate::subscription::NewOperationLogEntry;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct AddCall {
        server_id: String,
        download_url: String,
        category: Option<String>,
        savepath: Option<String>,
    }

    struct FakePortState {
        test_result: Result<String, ClientError>,
        fetch_result: Result<String, ClientError>,
        add_result: Result<(), ClientError>,
        tested_servers: Vec<String>,
        fetch_calls: Vec<(String, String)>,
        add_calls: Vec<AddCall>,
    }

    struct FakeManualQbPort {
        state: Mutex<FakePortState>,
    }

    impl FakeManualQbPort {
        fn new(
            test_result: Result<String, ClientError>,
            fetch_result: Result<String, ClientError>,
            add_result: Result<(), ClientError>,
        ) -> Self {
            Self {
                state: Mutex::new(FakePortState {
                    test_result,
                    fetch_result,
                    add_result,
                    tested_servers: Vec::new(),
                    fetch_calls: Vec::new(),
                    add_calls: Vec::new(),
                }),
            }
        }
    }

    impl ManualQbPort for FakeManualQbPort {
        fn test_connection(&self, server: QbServerEntry) -> ManualQbPortFuture<String> {
            let result = {
                let mut state = self.state.lock().expect("fake port lock");
                state.tested_servers.push(server.id);
                state.test_result.clone()
            };
            Box::pin(async move { result })
        }

        fn fetch_mteam_download_url(
            &self,
            api_key: String,
            torrent_id: String,
        ) -> ManualQbPortFuture<String> {
            let result = {
                let mut state = self.state.lock().expect("fake port lock");
                state.fetch_calls.push((api_key, torrent_id));
                state.fetch_result.clone()
            };
            Box::pin(async move { result })
        }

        fn add_torrent_from_url(
            &self,
            server: QbServerEntry,
            download_url: String,
            category: Option<String>,
            savepath: Option<String>,
        ) -> ManualQbPortFuture<()> {
            let result = {
                let mut state = self.state.lock().expect("fake port lock");
                state.add_calls.push(AddCall {
                    server_id: server.id,
                    download_url,
                    category,
                    savepath,
                });
                state.add_result.clone()
            };
            Box::pin(async move { result })
        }
    }

    #[derive(Default)]
    struct RecordingAudit {
        entries: Mutex<Vec<NewOperationLogEntry>>,
    }

    impl AuditLogPort for RecordingAudit {
        fn append(&self, entry: NewOperationLogEntry) -> AuditLogFuture {
            self.entries
                .lock()
                .expect("recording audit lock")
                .push(entry);
            Box::pin(async { Ok(()) })
        }
    }

    fn server(id: &str) -> QbServerEntry {
        QbServerEntry {
            id: id.to_string(),
            name: id.to_string(),
            base_url: "http://127.0.0.1:8080".to_string(),
            username: "admin".to_string(),
            password: "test-only-password".to_string(),
            insecure_tls: false,
        }
    }

    fn test_config() -> ConfigManager {
        ConfigManager::new(
            PathBuf::from("/tmp/manual-qb-service-test-config.toml"),
            FileConfig {
                mteam_api_key: "mteam-test-key".to_string(),
                douban_cookie: "dbcl2=manual-qb-test:secret; ck=test".to_string(),
                qb_servers: vec![server("nas")],
                ..FileConfig::default()
            },
        )
    }

    fn push_command() -> ManualQbPushCommand {
        ManualQbPushCommand {
            server_id: "nas".to_string(),
            torrent_id: "42".to_string(),
            category: Some("movies".to_string()),
            savepath: Some("/downloads/movies".to_string()),
        }
    }

    #[test]
    fn configured_server_resolution_is_exact_and_rejects_ambiguity() {
        let servers = [server("nas-a"), server("nas-b")];
        assert_eq!(
            resolve_configured_server(&servers, " nas-b ").unwrap().id,
            "nas-b"
        );
        assert!(resolve_configured_server(&servers, "missing").is_err());
        assert!(resolve_configured_server(&[server("nas"), server("nas")], "nas").is_err());
    }

    #[tokio::test]
    async fn fake_port_drives_connection_and_one_successful_push_with_one_audit() {
        let port = Arc::new(FakeManualQbPort::new(
            Ok("5.0.4".to_string()),
            Ok("https://download.test/torrent/42".to_string()),
            Ok(()),
        ));
        let audit = Arc::new(RecordingAudit::default());
        let service = ManualQbService::with_port(test_config(), port.clone(), audit.clone());

        assert_eq!(
            service.test_connection("nas".to_string()).await.unwrap(),
            "5.0.4"
        );
        assert_eq!(
            service.push_mteam(push_command()).await.unwrap(),
            ManualQbPushOutcome { added: true }
        );

        let state = port.state.lock().expect("fake port lock");
        assert_eq!(state.tested_servers, ["nas"]);
        assert_eq!(
            state.fetch_calls,
            [("mteam-test-key".to_string(), "42".to_string())]
        );
        assert_eq!(
            state.add_calls,
            [AddCall {
                server_id: "nas".to_string(),
                download_url: "https://download.test/torrent/42".to_string(),
                category: Some("movies".to_string()),
                savepath: Some("/downloads/movies".to_string()),
            }]
        );
        drop(state);

        let entries = audit.entries.lock().expect("recording audit lock");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "success");
        assert_eq!(entries[0].action, "manual_push_torrent");
        assert_eq!(entries[0].target_id.as_deref(), Some("42"));
        assert!(entries[0].error.is_none());
    }

    #[tokio::test]
    async fn fetch_failure_records_one_failure_without_running_add_effect() {
        let port = Arc::new(FakeManualQbPort::new(
            Ok("unused".to_string()),
            Err(ClientError::unavailable("M-Team", "fixture unavailable")),
            Ok(()),
        ));
        let audit = Arc::new(RecordingAudit::default());
        let service = ManualQbService::with_port(test_config(), port.clone(), audit.clone());

        assert!(matches!(
            service.push_mteam(push_command()).await,
            Err(ManualQbError::Upstream(ClientError::Unavailable {
                provider: "M-Team",
                ..
            }))
        ));

        let state = port.state.lock().expect("fake port lock");
        assert_eq!(state.fetch_calls.len(), 1);
        assert!(state.add_calls.is_empty());
        drop(state);

        let entries = audit.entries.lock().expect("recording audit lock");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "failed");
        assert_eq!(
            entries[0].summary,
            "手动推送种子到 qB 失败：M-Team 取链失败"
        );
        assert!(entries[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("fixture unavailable")));
    }

    #[tokio::test]
    async fn add_failure_records_one_failure_after_exactly_one_add_effect() {
        let port = Arc::new(FakeManualQbPort::new(
            Ok("unused".to_string()),
            Ok("https://download.test/torrent/42".to_string()),
            Err(ClientError::unavailable(
                "qBittorrent",
                "fixture unavailable",
            )),
        ));
        let audit = Arc::new(RecordingAudit::default());
        let service = ManualQbService::with_port(test_config(), port.clone(), audit.clone());

        assert!(matches!(
            service.push_mteam(push_command()).await,
            Err(ManualQbError::Upstream(ClientError::Unavailable {
                provider: "qBittorrent",
                ..
            }))
        ));

        let state = port.state.lock().expect("fake port lock");
        assert_eq!(state.fetch_calls.len(), 1);
        assert_eq!(state.add_calls.len(), 1);
        drop(state);

        let entries = audit.entries.lock().expect("recording audit lock");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "failed");
        assert_eq!(
            entries[0].summary,
            "手动推送种子到 qB 失败：qB 添加种子失败"
        );
        assert!(entries[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("fixture unavailable")));
    }
}
