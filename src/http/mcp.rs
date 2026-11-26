use std::path::{Component, Path};
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{routing::any, Router};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::CallToolResult;
use rmcp::transport::streamable_http_server::{
    session::never::NeverSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{schemars, tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::auth::{bearer_token, token_matches};
use super::dto::{douban::DoubanSearchResponseDto, media::MediaSearchResponseDto};
use super::error::ApiError;
use super::mteam::MteamSearchResponse;
use super::subscription_queries::{query_detail, query_list, ListSubscriptionQuery};
use crate::app::douban_catalog::{DoubanInterest, DoubanSearchCommand, MarkDoubanInterestCommand};
use crate::app::manual_qb::ManualQbPushCommand;
use crate::app::mteam_search::{MteamSearchCommand, TorrentSearchSource};
use crate::app::AppState;
use crate::config::{FileConfig, SubscriptionCategory};
use crate::subscription::effect_adapters::{
    HardlinkEffectAdapter, HardlinkFileStatus, HardlinkFilesystemFailureKind,
};

pub(super) fn routes(state: AppState) -> Router {
    Router::new()
        .route("/mcp", any(handle_request))
        .with_state(HubTools {
            state,
            hardlinks: HardlinkEffectAdapter::try_new(2).expect("positive concurrency"),
        })
}

async fn handle_request(State(hub): State<HubTools>, request: Request) -> Response {
    let management = hub.state.config.snapshot().await.value.management;
    let token = management.admin_token.trim();
    if token.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !bearer_token(request.headers()).is_some_and(|candidate| token_matches(candidate, token)) {
        return (
            StatusCode::UNAUTHORIZED,
            [(header::WWW_AUTHENTICATE, "Bearer realm=\"hub-mcp\"")],
        )
            .into_response();
    }
    // This transport is stateless. Apply the saved host policy to this request
    // so rotating credentials and removing a host take effect together.
    let mut config = StreamableHttpServerConfig::default().enforce_origin_validation();
    config.legacy_session_mode = false;
    config.json_response = true;
    config.allowed_origins = management.allowed_origins;
    if !management.mcp_host.is_empty() {
        config.allowed_hosts.push(management.mcp_host);
    }
    let service = StreamableHttpService::new(
        move || Ok(hub.clone()),
        Arc::new(NeverSessionManager::default()),
        config,
    );
    service.handle(request).await.into_response()
}

#[derive(Clone)]
struct HubTools {
    state: AppState,
    hardlinks: HardlinkEffectAdapter,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Catalog {
    Douban,
    Tmdb,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchMedia {
    query: String,
    /// Defaults to Douban. Use Douban when adding a subscription.
    catalog: Option<Catalog>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
enum ResourceSource {
    Douban,
    Imdb,
    Keyword,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchResources {
    source: ResourceSource,
    query: String,
    page: Option<u32>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct PushDownload {
    torrent_id: String,
    /// ID from list_download_targets.
    server_id: String,
    /// Optional configured category name. Uses its qB save path.
    category: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SubscriptionPage {
    limit: Option<u32>,
    cursor: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Subject {
    douban_id: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddSubscription {
    douban_id: String,
    category: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ManualLink {
    category: String,
    /// Existing completed file, relative to the category download directory.
    source: String,
    /// Desired filename (including subdirectories), relative to the category library directory.
    target: String,
}

fn category<'a>(config: &'a FileConfig, name: &str) -> Result<&'a SubscriptionCategory, String> {
    config
        .subscription_categories
        .iter()
        .find(|c| c.name == name)
        .ok_or_else(|| "未知分类，请先调用 list_download_targets".to_owned())
}

fn relative_file(value: &str) -> Result<&Path, String> {
    let path = Path::new(value);
    if value.trim().is_empty()
        || value.ends_with('/')
        || value.contains(['\\', ':'])
        || value.chars().any(char::is_control)
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return Err("请使用分类目录下的相对文件路径，以 / 分隔".to_owned());
    }
    Ok(path)
}

fn tool_error(error: impl Into<ApiError>) -> String {
    error.into().message().to_owned()
}

fn result(value: impl Serialize) -> Result<CallToolResult, String> {
    let value = serde_json::to_value(value).map_err(|_| "无法编码工具结果".to_owned())?;
    Ok(CallToolResult::structured(value))
}

#[tool_router(server_handler)]
impl HubTools {
    #[tool(
        annotations(read_only_hint = true),
        description = "Search movies and TV shows by title. Douban results provide the subject ID needed for add_subscription."
    )]
    async fn search_media(
        &self,
        Parameters(args): Parameters<SearchMedia>,
    ) -> Result<CallToolResult, String> {
        match args.catalog.unwrap_or(Catalog::Douban) {
            Catalog::Douban => {
                let found = self
                    .state
                    .douban_catalog
                    .search(DoubanSearchCommand {
                        query: args.query,
                        page: 1,
                        page_size: 20,
                    })
                    .await
                    .map_err(tool_error)?;
                result(DoubanSearchResponseDto::from(found))
            }
            Catalog::Tmdb => {
                let found = self
                    .state
                    .media_catalog
                    .search(args.query, None)
                    .await
                    .map_err(tool_error)?;
                result(MediaSearchResponseDto::from(found))
            }
        }
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "Search M-Team by Douban subject ID, IMDb ID or title keyword. Returns torrent IDs for push_download."
    )]
    async fn search_resources(
        &self,
        Parameters(args): Parameters<SearchResources>,
    ) -> Result<CallToolResult, String> {
        let source = match args.source {
            ResourceSource::Douban => TorrentSearchSource::Douban,
            ResourceSource::Imdb => TorrentSearchSource::Imdb,
            ResourceSource::Keyword => TorrentSearchSource::Keyword,
        };
        let found = self
            .state
            .mteam_search
            .search(MteamSearchCommand {
                source,
                query: args.query,
                page: args.page.unwrap_or(1),
                page_size: 50,
            })
            .await
            .map_err(tool_error)?;
        result(MteamSearchResponse::from(found))
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "List qB server IDs and configured subscription categories with download and library directories."
    )]
    async fn list_download_targets(&self) -> Result<CallToolResult, String> {
        let config = self.state.config.snapshot().await.value;
        let servers: Vec<_> = config
            .qb_servers
            .iter()
            .map(|s| json!({"id": s.id, "name": s.name}))
            .collect();
        result(json!({"servers": servers, "categories": config.subscription_categories}))
    }

    #[tool(
        description = "Push one M-Team torrent to the specified qB server for a manual download."
    )]
    async fn push_download(
        &self,
        Parameters(args): Parameters<PushDownload>,
    ) -> Result<CallToolResult, String> {
        let config = self.state.config.snapshot().await.value;
        let target = args
            .category
            .as_deref()
            .map(|name| category(&config, name))
            .transpose()?;
        if target.is_some_and(|c| c.qb_server_id != args.server_id) {
            return Err("所选分类与 qB 服务器不匹配".to_owned());
        }
        let outcome = self
            .state
            .manual_qb
            .push_mteam(ManualQbPushCommand {
                server_id: args.server_id,
                torrent_id: args.torrent_id,
                category: target.map(|c| c.qb_category.clone()),
                savepath: target.map(|c| c.qb_save_dir_name.clone()),
            })
            .await
            .map_err(tool_error)?;
        result(json!({"added": outcome.added}))
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "List Hub subscriptions. Pass next_cursor as cursor to read the next page."
    )]
    async fn list_subscriptions(
        &self,
        Parameters(args): Parameters<SubscriptionPage>,
    ) -> Result<CallToolResult, String> {
        let page = query_list(
            &self.state.config,
            &self.state.subscription_queries,
            ListSubscriptionQuery::page(args.limit, args.cursor),
        )
        .await
        .map_err(tool_error)?;
        result(page)
    }

    #[tool(
        annotations(read_only_hint = true),
        description = "Read a subscription, including progress, candidates and linked files."
    )]
    async fn get_subscription(
        &self,
        Parameters(args): Parameters<Subject>,
    ) -> Result<CallToolResult, String> {
        result(
            query_detail(
                &self.state.config,
                &self.state.subscription_queries,
                args.douban_id,
            )
            .await
            .map_err(tool_error)?,
        )
    }

    #[tool(
        description = "Add a subscription by marking a Douban subject as wanted with a configured category. The watcher imports it on a subsequent poll."
    )]
    async fn add_subscription(
        &self,
        Parameters(args): Parameters<AddSubscription>,
    ) -> Result<CallToolResult, String> {
        if args.douban_id.is_empty() || !args.douban_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err("请提供豆瓣数字条目 ID".to_owned());
        }
        let config = self.state.config.snapshot().await.value;
        let target = category(&config, &args.category)?;
        let outcome = self
            .state
            .douban_catalog
            .mark_interest(MarkDoubanInterestCommand {
                subject_id: args.douban_id.clone(),
                interest: DoubanInterest::Wish,
                rating: None,
                tags: target.wanted_tag.clone(),
            })
            .await
            .map_err(tool_error)?;
        result(
            json!({"wish_saved": outcome.ok, "douban_id": args.douban_id, "category": target.name,
            "watcher_enabled": config.subscription_watcher.enabled, "dry_run": config.subscription_watcher.dry_run}),
        )
    }

    #[tool(
        annotations(destructive_hint = false, idempotent_hint = true),
        description = "Hard-link one completed file from the category download directory to its library directory on the same filesystem. Both paths are relative. Keeps the original for seeding and never overwrites another file."
    )]
    async fn create_hardlink(
        &self,
        Parameters(args): Parameters<ManualLink>,
    ) -> Result<CallToolResult, String> {
        let config = self.state.config.snapshot().await.value;
        let c = category(&config, &args.category)?;
        let source = Path::new(&c.download_dir).join(relative_file(&args.source)?);
        let target = Path::new(&c.link_target_dir).join(relative_file(&args.target)?);
        if source == target {
            return Err("源文件和目标文件相同".to_owned());
        }
        let status = self
            .hardlinks
            .link_one(source, target)
            .await
            .map_err(|e| e.to_string())?;
        let status = match status {
            HardlinkFileStatus::Created => "created",
            HardlinkFileStatus::SkippedVerified | HardlinkFileStatus::AcceptedExisting => {
                "already_linked"
            }
            HardlinkFileStatus::Conflict { .. } => {
                return Err("媒体库目标已存在且指向其他文件".to_owned())
            }
            HardlinkFileStatus::Missing => return Err("下载目录下找不到源文件".to_owned()),
            HardlinkFileStatus::Failed(failure) if failure.kind() == HardlinkFilesystemFailureKind::Io(std::io::ErrorKind::CrossesDevices) => {
                return Err("下载目录与媒体库不在同一文件系统或挂载点；请检查分类路径与容器的共同父目录挂载".to_owned())
            }
            HardlinkFileStatus::Failed(failure) if failure.kind() == HardlinkFilesystemFailureKind::Io(std::io::ErrorKind::Unsupported) => {
                return Err("当前平台不支持此硬链接操作，请在 Linux/NAS 服务中使用".to_owned())
            }
            HardlinkFileStatus::Failed(failure) => {
                return Err(format!(
                    "硬链接失败：{:?} ({:?})",
                    failure.operation(),
                    failure.kind()
                ))
            }
        };
        result(
            json!({"status": status, "category": c.name, "source": args.source, "target": args.target}),
        )
    }
}
