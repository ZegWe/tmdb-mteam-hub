use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use super::audit::{operation_log_entry, AuditLogPort, OperationLogEvent};
use crate::clients::douban::DoubanClient;
use crate::config::{ConfigManager, FileConfig, SubscriptionCategory};
use crate::douban;
use crate::tmdb_cache::TmdbDiskCache;

const SEARCH_PAGE_SIZE: usize = 20;
const MAX_LIBRARY_LIMIT: usize = 1_200;
const MAX_TAG_HISTORY_LIMIT: usize = 1_200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DoubanLibraryCommand {
    pub(crate) force_refresh: bool,
    pub(crate) limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DoubanLibraryItem {
    pub(crate) source: String,
    pub(crate) media_type: String,
    pub(crate) id: String,
    pub(crate) subject_id: String,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) abstract_text: String,
    pub(crate) abstract_2: String,
    pub(crate) cover_url: String,
    pub(crate) poster_url: String,
    pub(crate) status: String,
    pub(crate) status_label: String,
    pub(crate) date: String,
    pub(crate) comment: String,
    pub(crate) tags: Vec<String>,
    pub(crate) user_rating: Option<u8>,
}

impl From<douban::DoubanLibraryItem> for DoubanLibraryItem {
    fn from(value: douban::DoubanLibraryItem) -> Self {
        Self {
            source: value.source.to_string(),
            media_type: value.media_type.to_string(),
            id: value.id,
            subject_id: value.subject_id,
            title: value.title,
            url: value.url,
            abstract_text: value.abstract_text,
            abstract_2: value.abstract_2,
            cover_url: value.cover_url,
            poster_url: value.poster_url,
            status: value.status.to_string(),
            status_label: value.status_label.to_string(),
            date: value.date,
            comment: value.comment,
            tags: value.tags,
            user_rating: value.user_rating,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DoubanSnapshotCompleteness {
    Complete,
    Partial,
}

impl From<douban::SnapshotCompleteness> for DoubanSnapshotCompleteness {
    fn from(value: douban::SnapshotCompleteness) -> Self {
        match value {
            douban::SnapshotCompleteness::Complete => Self::Complete,
            douban::SnapshotCompleteness::Partial => Self::Partial,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DoubanLibraryList {
    pub(crate) status: String,
    pub(crate) label: String,
    pub(crate) items: Vec<DoubanLibraryItem>,
    pub(crate) completeness: DoubanSnapshotCompleteness,
    pub(crate) fetched_pages: usize,
    pub(crate) truncated_by_limit: bool,
    pub(crate) end_observed: bool,
}

impl From<douban::DoubanLibraryList> for DoubanLibraryList {
    fn from(value: douban::DoubanLibraryList) -> Self {
        Self {
            status: value.status.to_string(),
            label: value.label.to_string(),
            items: value.items.into_iter().map(Into::into).collect(),
            completeness: value.snapshot.completeness.into(),
            fetched_pages: value.snapshot.fetched_pages,
            truncated_by_limit: value.snapshot.truncated_by_limit,
            end_observed: value.snapshot.end_observed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DoubanLibraryOutcome {
    pub(crate) source: String,
    pub(crate) cached: bool,
    pub(crate) fetched_at: u64,
    pub(crate) ttl_seconds: u64,
    pub(crate) limit: usize,
    pub(crate) wish: DoubanLibraryList,
    pub(crate) collect: DoubanLibraryList,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DoubanTagHistoryCommand {
    pub(crate) limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DoubanTagCount {
    pub(crate) tag: String,
    pub(crate) count: u64,
    pub(crate) category: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DoubanTagCategory {
    pub(crate) name: String,
    pub(crate) wanted_tag: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DoubanTagHistoryOutcome {
    pub(crate) source: String,
    pub(crate) cached: bool,
    pub(crate) updated_at: Option<u64>,
    pub(crate) tags: Vec<String>,
    pub(crate) tag_counts: Vec<DoubanTagCount>,
    pub(crate) subscription_categories: Vec<DoubanTagCategory>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DoubanQrStartOutcome {
    pub(crate) session_id: String,
    pub(crate) image_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DoubanQrPollOutcome {
    pub(crate) done: bool,
    pub(crate) login_status: String,
    pub(crate) message: String,
    pub(crate) description: String,
    pub(crate) cookie_saved: bool,
}

pub(crate) struct DoubanQrStartProviderOutcome {
    session_id: String,
    session: douban::QrSession,
    image_url: String,
}

pub(crate) struct DoubanQrPollProviderOutcome {
    done: bool,
    login_status: String,
    message: String,
    description: String,
    cookie_header: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DoubanInterest {
    Wish,
    Collect,
}

impl DoubanInterest {
    fn audit_label(self) -> &'static str {
        match self {
            Self::Wish => "Wish",
            Self::Collect => "Collect",
        }
    }

    fn success_summary(self) -> &'static str {
        match self {
            Self::Wish => "已标记豆瓣想看",
            Self::Collect => "已标记豆瓣看过",
        }
    }
}

impl From<DoubanInterest> for douban::DoubanInterest {
    fn from(value: DoubanInterest) -> Self {
        match value {
            DoubanInterest::Wish => Self::Wish,
            DoubanInterest::Collect => Self::Collect,
        }
    }
}

impl From<douban::DoubanInterest> for DoubanInterest {
    fn from(value: douban::DoubanInterest) -> Self {
        match value {
            douban::DoubanInterest::Wish => Self::Wish,
            douban::DoubanInterest::Collect => Self::Collect,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DoubanRating {
    pub(crate) value: Option<f64>,
    pub(crate) count: Option<u64>,
    pub(crate) info: String,
    pub(crate) star_count: Option<f64>,
}

impl From<douban::DoubanRating> for DoubanRating {
    fn from(value: douban::DoubanRating) -> Self {
        Self {
            value: value.value,
            count: value.count,
            info: value.info,
            star_count: value.star_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DoubanSearchItem {
    pub(crate) source: String,
    pub(crate) media_type: String,
    pub(crate) id: String,
    pub(crate) subject_id: String,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) abstract_text: String,
    pub(crate) abstract_2: String,
    pub(crate) cover_url: String,
    pub(crate) poster_url: String,
    pub(crate) rating: DoubanRating,
    pub(crate) vote_average: Option<f64>,
}

impl From<douban::DoubanSearchResult> for DoubanSearchItem {
    fn from(value: douban::DoubanSearchResult) -> Self {
        Self {
            source: value.source.to_string(),
            media_type: value.media_type.to_string(),
            id: value.id,
            subject_id: value.subject_id,
            title: value.title,
            url: value.url,
            abstract_text: value.abstract_text,
            abstract_2: value.abstract_2,
            cover_url: value.cover_url,
            poster_url: value.poster_url,
            rating: value.rating.into(),
            vote_average: value.vote_average,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DoubanSearchOutcome {
    pub(crate) items: Vec<DoubanSearchItem>,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
    pub(crate) has_more: bool,
}

impl From<douban::DoubanSearchPage> for DoubanSearchOutcome {
    fn from(value: douban::DoubanSearchPage) -> Self {
        Self {
            items: value.items.into_iter().map(Into::into).collect(),
            page: value.page,
            page_size: value.page_size,
            has_more: value.has_more,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DoubanSubjectDetail {
    pub(crate) source: String,
    pub(crate) media_type: String,
    pub(crate) id: String,
    pub(crate) subject_id: String,
    pub(crate) url: String,
    pub(crate) title: String,
    pub(crate) original_title: String,
    pub(crate) aka: Vec<String>,
    pub(crate) languages: Vec<String>,
    pub(crate) countries: Vec<String>,
    pub(crate) image: String,
    pub(crate) poster_url: String,
    pub(crate) directors: Vec<String>,
    pub(crate) writers: Vec<String>,
    pub(crate) actors: Vec<String>,
    pub(crate) genres: Vec<String>,
    pub(crate) date_published: String,
    pub(crate) duration: String,
    pub(crate) summary: String,
    pub(crate) rating: DoubanRating,
    pub(crate) user_interest: Option<DoubanInterest>,
    pub(crate) user_rating: Option<u8>,
}

impl From<douban::DoubanSubjectDetail> for DoubanSubjectDetail {
    fn from(value: douban::DoubanSubjectDetail) -> Self {
        Self {
            source: value.source.to_string(),
            media_type: value.media_type.to_string(),
            id: value.id,
            subject_id: value.subject_id,
            url: value.url,
            title: value.title,
            original_title: value.original_title,
            aka: value.aka,
            languages: value.languages,
            countries: value.countries,
            image: value.image,
            poster_url: value.poster_url,
            directors: value.directors,
            writers: value.writers,
            actors: value.actors,
            genres: value.genres,
            date_published: value.date_published,
            duration: value.duration,
            summary: value.summary,
            rating: value.rating.into(),
            user_interest: value.user_interest.map(Into::into),
            user_rating: value.user_rating,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DoubanInterestResult {
    pub(crate) ok: bool,
    pub(crate) interest: DoubanInterest,
    pub(crate) rating: Option<u8>,
    pub(crate) tags: String,
}

impl From<douban::DoubanInterestResult> for DoubanInterestResult {
    fn from(value: douban::DoubanInterestResult) -> Self {
        Self {
            ok: value.ok,
            interest: match value.interest {
                "wish" => DoubanInterest::Wish,
                _ => DoubanInterest::Collect,
            },
            rating: value.rating,
            tags: value.tags,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DoubanSearchCommand {
    pub(crate) query: String,
    pub(crate) page: usize,
    pub(crate) page_size: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarkDoubanInterestCommand {
    pub(crate) subject_id: String,
    pub(crate) interest: DoubanInterest,
    pub(crate) rating: Option<u8>,
    pub(crate) tags: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DoubanCatalogError {
    Validation { message: String },
    Upstream { message: String },
    Internal { message: String },
}

impl DoubanCatalogError {
    pub(crate) fn message(&self) -> &str {
        match self {
            Self::Validation { message }
            | Self::Upstream { message }
            | Self::Internal { message } => message,
        }
    }
}

impl From<douban::DoubanError> for DoubanCatalogError {
    fn from(value: douban::DoubanError) -> Self {
        if value.is_bad_request() {
            Self::Validation {
                message: value.message,
            }
        } else {
            Self::Upstream {
                message: value.message,
            }
        }
    }
}

pub(crate) type ProviderFuture<T> =
    Pin<Box<dyn Future<Output = Result<T, DoubanCatalogError>> + Send + 'static>>;

pub(crate) trait DoubanCatalogProvider: Send + Sync {
    fn search(
        &self,
        cookie: String,
        query: String,
        page: usize,
        page_size: usize,
    ) -> ProviderFuture<DoubanSearchOutcome>;

    fn subject_detail(
        &self,
        cookie: String,
        subject_id: String,
    ) -> ProviderFuture<DoubanSubjectDetail>;

    fn mark_interest(
        &self,
        cookie: String,
        command: MarkDoubanInterestCommand,
    ) -> ProviderFuture<DoubanInterestResult>;

    fn library(
        &self,
        cookie: String,
        status: douban::DoubanLibraryStatus,
        limit: usize,
    ) -> ProviderFuture<DoubanLibraryList>;

    fn qr_start(&self) -> ProviderFuture<DoubanQrStartProviderOutcome>;

    fn qr_poll(&self, session: douban::QrSession) -> ProviderFuture<DoubanQrPollProviderOutcome>;
}

#[derive(Clone)]
struct LiveDoubanCatalogProvider {
    client: DoubanClient,
}

impl DoubanCatalogProvider for LiveDoubanCatalogProvider {
    fn search(
        &self,
        cookie: String,
        query: String,
        page: usize,
        page_size: usize,
    ) -> ProviderFuture<DoubanSearchOutcome> {
        let client = self.client.clone();
        Box::pin(async move {
            douban::search(&client, &cookie, &query, page, page_size)
                .await
                .map(Into::into)
                .map_err(Into::into)
        })
    }

    fn subject_detail(
        &self,
        cookie: String,
        subject_id: String,
    ) -> ProviderFuture<DoubanSubjectDetail> {
        let client = self.client.clone();
        Box::pin(async move {
            douban::subject_detail(&client, &cookie, &subject_id)
                .await
                .map(Into::into)
                .map_err(Into::into)
        })
    }

    fn mark_interest(
        &self,
        cookie: String,
        command: MarkDoubanInterestCommand,
    ) -> ProviderFuture<DoubanInterestResult> {
        let client = self.client.clone();
        Box::pin(async move {
            douban::mark_interest(
                &client,
                &cookie,
                &command.subject_id,
                command.interest.into(),
                command.rating,
                &command.tags,
            )
            .await
            .map(Into::into)
            .map_err(Into::into)
        })
    }

    fn library(
        &self,
        cookie: String,
        status: douban::DoubanLibraryStatus,
        limit: usize,
    ) -> ProviderFuture<DoubanLibraryList> {
        let client = self.client.clone();
        Box::pin(async move {
            douban::library(&client, &cookie, status, limit)
                .await
                .map(Into::into)
                .map_err(Into::into)
        })
    }

    fn qr_start(&self) -> ProviderFuture<DoubanQrStartProviderOutcome> {
        let client = self.client.clone();
        Box::pin(async move {
            let (session_id, session, result) = douban::qr_start(&client).await?;
            Ok(DoubanQrStartProviderOutcome {
                session_id,
                session,
                image_url: result.image_url,
            })
        })
    }

    fn qr_poll(&self, session: douban::QrSession) -> ProviderFuture<DoubanQrPollProviderOutcome> {
        Box::pin(async move {
            let result = douban::qr_poll(&session).await?;
            Ok(DoubanQrPollProviderOutcome {
                done: result.done,
                login_status: result.login_status,
                message: result.message,
                description: result.description,
                cookie_header: result.cookie_header,
            })
        })
    }
}

#[derive(Clone)]
pub(crate) struct DoubanCatalogService {
    config: ConfigManager,
    provider: Arc<dyn DoubanCatalogProvider>,
    cache: TmdbDiskCache,
    cache_ttl_secs: u64,
    audit: Arc<dyn AuditLogPort>,
    qr_sessions: Arc<RwLock<HashMap<String, douban::QrSession>>>,
}

impl DoubanCatalogService {
    pub(crate) fn new(
        config: ConfigManager,
        client: DoubanClient,
        cache: TmdbDiskCache,
        cache_ttl_secs: u64,
        audit: Arc<dyn AuditLogPort>,
    ) -> Self {
        Self {
            config,
            provider: Arc::new(LiveDoubanCatalogProvider { client }),
            cache,
            cache_ttl_secs,
            audit,
            qr_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_provider(
        config: ConfigManager,
        provider: Arc<dyn DoubanCatalogProvider>,
        cache: TmdbDiskCache,
        cache_ttl_secs: u64,
        audit: Arc<dyn AuditLogPort>,
    ) -> Self {
        Self {
            config,
            provider,
            cache,
            cache_ttl_secs,
            audit,
            qr_sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub(crate) async fn search(
        &self,
        command: DoubanSearchCommand,
    ) -> Result<DoubanSearchOutcome, DoubanCatalogError> {
        let config = self.config.snapshot().await.value;
        let page = command.page.max(1);
        let page_size = command.page_size.clamp(1, SEARCH_PAGE_SIZE);
        let query = command.query;
        let result = self
            .provider
            .search(config.douban_cookie.clone(), query.clone(), page, page_size)
            .await;
        match result {
            Ok(outcome) => {
                self.record_search_success(&config, &query, &outcome).await;
                Ok(outcome)
            }
            Err(error) => {
                self.record_search_failure(&config, &query, page, page_size, &error)
                    .await;
                Err(error)
            }
        }
    }

    pub(crate) async fn library(
        &self,
        command: DoubanLibraryCommand,
    ) -> Result<DoubanLibraryOutcome, DoubanCatalogError> {
        let cookie = self.config.snapshot().await.value.douban_cookie;
        let account_key = douban::auth_cache_key_fragment(&cookie)?;
        let limit = command.limit.clamp(1, MAX_LIBRARY_LIMIT);
        let cache_key = format!("library_{account_key}_limit_{limit}");

        if !command.force_refresh {
            if let Some(value) = self.cache.get(&cache_key).await {
                if let Ok(mut cached) = serde_json::from_value::<DoubanLibraryOutcome>(value) {
                    cached.cached = true;
                    return Ok(cached);
                }
            }
        }

        let (wish, collect) = tokio::try_join!(
            self.provider
                .library(cookie.clone(), douban::DoubanLibraryStatus::Wish, limit),
            self.provider
                .library(cookie, douban::DoubanLibraryStatus::Collect, limit),
        )?;
        let outcome = DoubanLibraryOutcome {
            source: "douban".to_string(),
            cached: false,
            fetched_at: unix_now_secs(),
            ttl_seconds: self.cache_ttl_secs,
            limit,
            wish,
            collect,
        };
        match serde_json::to_value(&outcome) {
            Ok(value) => {
                if let Err(error) = self.cache.put(&cache_key, &value).await {
                    tracing::warn!("douban library cache write failed: {error}");
                }
            }
            Err(error) => tracing::warn!("douban library cache serialization failed: {error}"),
        }
        Ok(outcome)
    }

    pub(crate) async fn tag_history(
        &self,
        command: DoubanTagHistoryCommand,
    ) -> Result<DoubanTagHistoryOutcome, DoubanCatalogError> {
        let config = self.config.snapshot().await.value;
        let account_key = douban::auth_cache_key_fragment(&config.douban_cookie)?;
        let limit = command.limit.clamp(1, MAX_TAG_HISTORY_LIMIT);
        let key = douban_tag_history_cache_key(&account_key);
        let cached = self
            .cache
            .get_any(&key)
            .await
            .and_then(|value| serde_json::from_value::<CachedTagHistory>(value).ok())
            .unwrap_or_default();
        Ok(constrain_tag_history(
            cached,
            &config.subscription_categories,
            limit,
        ))
    }

    pub(crate) async fn start_qr(&self) -> Result<DoubanQrStartOutcome, DoubanCatalogError> {
        let result = self.provider.qr_start().await?;
        let outcome = DoubanQrStartOutcome {
            session_id: result.session_id.clone(),
            image_url: result.image_url,
        };
        self.qr_sessions
            .write()
            .await
            .insert(result.session_id, result.session);
        Ok(outcome)
    }

    pub(crate) async fn qr_image(
        &self,
        session_id: String,
    ) -> Result<Arc<Vec<u8>>, DoubanCatalogError> {
        let session_id = normalized_session_id(session_id)?;
        self.qr_sessions
            .read()
            .await
            .get(&session_id)
            .map(|session| session.image.clone())
            .ok_or_else(|| DoubanCatalogError::Validation {
                message: "豆瓣 QR 登录会话不存在或已过期".to_string(),
            })
    }

    pub(crate) async fn poll_qr(
        &self,
        session_id: String,
    ) -> Result<DoubanQrPollOutcome, DoubanCatalogError> {
        let session_id = normalized_session_id(session_id)?;
        let session = self
            .qr_sessions
            .read()
            .await
            .get(&session_id)
            .cloned()
            .ok_or_else(|| DoubanCatalogError::Validation {
                message: "豆瓣 QR 登录会话不存在或已过期".to_string(),
            })?;
        let result = self.provider.qr_poll(session).await?;
        let cookie_saved = if let Some(cookie_header) = result.cookie_header.as_deref() {
            let cookie = douban::normalize_cookie_header(cookie_header);
            self.config
                .patch_douban_cookie(cookie)
                .await
                .map_err(|error| DoubanCatalogError::Internal {
                    message: format!("写入豆瓣 Cookie 失败: {error}"),
                })?;
            self.qr_sessions.write().await.remove(&session_id);
            true
        } else {
            false
        };
        Ok(DoubanQrPollOutcome {
            done: result.done,
            login_status: result.login_status,
            message: result.message,
            description: result.description,
            cookie_saved,
        })
    }

    pub(crate) async fn subject_detail(
        &self,
        subject_id: String,
    ) -> Result<DoubanSubjectDetail, DoubanCatalogError> {
        let cookie = self.config.snapshot().await.value.douban_cookie;
        self.provider.subject_detail(cookie, subject_id).await
    }

    pub(crate) async fn mark_interest(
        &self,
        mut command: MarkDoubanInterestCommand,
    ) -> Result<DoubanInterestResult, DoubanCatalogError> {
        let config = self.config.snapshot().await.value;
        let account_key = douban::auth_cache_key_fragment(&config.douban_cookie)
            .map_err(DoubanCatalogError::from)?;
        if command.interest == DoubanInterest::Wish {
            command.tags = normalize_wanted_tag(&command.tags, &config.subscription_categories)?;
        }
        let result = self
            .provider
            .mark_interest(config.douban_cookie.clone(), command.clone())
            .await;
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                self.record_interest(
                    &account_key,
                    &command,
                    "failed",
                    "豆瓣标记失败",
                    Some(error.message().to_string()),
                    json!({
                        "interest": command.interest.audit_label(),
                        "has_rating": command.rating.is_some(),
                    }),
                )
                .await;
                return Err(error);
            }
        };

        if let Err(error) = self
            .cache
            .remove_prefix(&format!("library_{account_key}_"))
            .await
        {
            tracing::warn!("douban library cache invalidation failed: {error}");
        }
        if let Err(error) = self.update_tag_history(&account_key, &outcome.tags).await {
            tracing::warn!("douban tag history update failed: {error}");
        }
        self.record_interest(
            &account_key,
            &command,
            "success",
            command.interest.success_summary(),
            None,
            json!({
                "interest": command.interest.audit_label(),
                "tag_count": outcome.tags.split_whitespace().count(),
            }),
        )
        .await;
        Ok(outcome)
    }

    async fn record_search_success(
        &self,
        config: &FileConfig,
        query: &str,
        outcome: &DoubanSearchOutcome,
    ) {
        self.append_audit(operation_log_entry(
            config_account_key(config),
            OperationLogEvent {
                category: "search",
                action: "search_media",
                target_type: "douban",
                target_id: None,
                target_title: Some(query.trim().to_string()),
                status: "success",
                summary: format!("豆瓣搜索完成：{} 条结果", outcome.items.len()),
                error: None,
                related: json!({
                    "source": "douban",
                    "result_count": outcome.items.len(),
                    "page": outcome.page,
                    "page_size": outcome.page_size,
                    "has_more": outcome.has_more,
                }),
            },
        ))
        .await;
    }

    async fn record_search_failure(
        &self,
        config: &FileConfig,
        query: &str,
        page: usize,
        page_size: usize,
        error: &DoubanCatalogError,
    ) {
        self.append_audit(operation_log_entry(
            config_account_key(config),
            OperationLogEvent {
                category: "search",
                action: "search_media",
                target_type: "douban",
                target_id: None,
                target_title: Some(query.trim().to_string()),
                status: "failed",
                summary: "豆瓣搜索失败",
                error: Some(error.message().to_string()),
                related: json!({
                    "source": "douban",
                    "page": page,
                    "page_size": page_size,
                }),
            },
        ))
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_interest(
        &self,
        account_key: &str,
        command: &MarkDoubanInterestCommand,
        status: &'static str,
        summary: &'static str,
        error: Option<String>,
        related: Value,
    ) {
        self.append_audit(operation_log_entry(
            account_key,
            OperationLogEvent {
                category: "subscription_sync",
                action: "mark_interest",
                target_type: "douban_subject",
                target_id: Some(command.subject_id.clone()),
                target_title: None,
                status,
                summary,
                error,
                related,
            },
        ))
        .await;
    }

    async fn append_audit(&self, entry: crate::subscription::NewOperationLogEntry) {
        if let Err(error) = self.audit.append(entry).await {
            tracing::warn!("operation log write failed: {error}");
        }
    }

    async fn update_tag_history(&self, account_key: &str, tags_text: &str) -> std::io::Result<()> {
        let tags = tags_text
            .split_whitespace()
            .map(str::trim)
            .filter(|tag| !tag.is_empty())
            .collect::<Vec<_>>();
        if tags.is_empty() {
            return Ok(());
        }

        let mut counts: HashMap<String, u64> = HashMap::new();
        let current = self
            .cache
            .get_any(&douban_tag_history_cache_key(account_key))
            .await
            .and_then(|value| serde_json::from_value::<CachedTagHistory>(value).ok())
            .unwrap_or_default();
        for item in current.tag_counts {
            if !item.tag.trim().is_empty() {
                counts.insert(item.tag.trim().to_string(), item.count.max(1));
            }
        }
        if counts.is_empty() {
            for tag in current.tags {
                if !tag.trim().is_empty() {
                    counts.entry(tag.trim().to_string()).or_insert(1);
                }
            }
        }
        for tag in tags {
            *counts.entry(tag.to_string()).or_default() += 1;
        }

        let mut ranked = counts.into_iter().collect::<Vec<_>>();
        ranked.sort_by(|(tag_a, count_a), (tag_b, count_b)| {
            count_b.cmp(count_a).then_with(|| tag_a.cmp(tag_b))
        });
        let value = serde_json::to_value(CachedTagHistory {
            source: "local-cache".to_string(),
            cached: true,
            updated_at: Some(unix_now_secs()),
            tags: ranked.iter().map(|(tag, _)| tag.clone()).collect(),
            tag_counts: ranked
                .into_iter()
                .map(|(tag, count)| CachedTagCount { tag, count })
                .collect(),
        })
        .map_err(std::io::Error::other)?;
        self.cache
            .put(&douban_tag_history_cache_key(account_key), &value)
            .await
    }
}

pub(crate) fn douban_tag_history_cache_key(account_key: &str) -> String {
    format!("tag_history_manual_{account_key}")
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct CachedTagHistory {
    #[serde(default = "local_cache_source")]
    source: String,
    #[serde(default = "default_true")]
    cached: bool,
    #[serde(default)]
    updated_at: Option<u64>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    tag_counts: Vec<CachedTagCount>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedTagCount {
    tag: String,
    #[serde(default)]
    count: u64,
}

fn local_cache_source() -> String {
    "local-cache".to_string()
}

fn default_true() -> bool {
    true
}

fn constrain_tag_history(
    cached: CachedTagHistory,
    categories: &[SubscriptionCategory],
    limit: usize,
) -> DoubanTagHistoryOutcome {
    let counts = cached
        .tag_counts
        .into_iter()
        .map(|item| (item.tag, item.count.max(1)))
        .collect::<HashMap<_, _>>();
    let mut rows = categories
        .iter()
        .filter_map(|category| {
            let tag = category.wanted_tag.trim();
            if tag.is_empty() {
                return None;
            }
            Some((
                tag.to_string(),
                counts.get(tag).copied().unwrap_or(0),
                category.name.trim().to_string(),
            ))
        })
        .collect::<Vec<_>>();
    rows.sort_by(|(tag_a, count_a, name_a), (tag_b, count_b, name_b)| {
        count_b
            .cmp(count_a)
            .then_with(|| name_a.cmp(name_b))
            .then_with(|| tag_a.cmp(tag_b))
    });
    rows.truncate(limit);

    DoubanTagHistoryOutcome {
        source: cached.source,
        cached: true,
        updated_at: cached.updated_at,
        tags: rows.iter().map(|(tag, _, _)| tag.clone()).collect(),
        tag_counts: rows
            .iter()
            .map(|(tag, count, category)| DoubanTagCount {
                tag: tag.clone(),
                count: *count,
                category: category.clone(),
            })
            .collect(),
        subscription_categories: categories
            .iter()
            .map(|category| DoubanTagCategory {
                name: category.name.trim().to_string(),
                wanted_tag: category.wanted_tag.trim().to_string(),
            })
            .collect(),
    }
}

fn normalized_session_id(session_id: String) -> Result<String, DoubanCatalogError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Err(DoubanCatalogError::Validation {
            message: "豆瓣 QR 登录会话 ID 不能为空".to_string(),
        });
    }
    Ok(session_id.to_string())
}

fn normalize_wanted_tag(
    raw: &str,
    categories: &[SubscriptionCategory],
) -> Result<String, DoubanCatalogError> {
    if categories.is_empty() {
        return Err(DoubanCatalogError::Validation {
            message: "请先在设置中配置订阅分类".to_string(),
        });
    }
    let parts = raw
        .split_whitespace()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 1 {
        return Err(DoubanCatalogError::Validation {
            message: "标记想看时必须选择一个订阅分类".to_string(),
        });
    }
    let selected = parts[0];
    if categories
        .iter()
        .any(|category| category.wanted_tag.trim() == selected)
    {
        return Ok(selected.to_string());
    }
    Err(DoubanCatalogError::Validation {
        message: format!("标记想看的标签必须来自订阅分类: {selected}"),
    })
}

fn config_account_key(config: &FileConfig) -> String {
    douban::auth_cache_key_fragment(&config.douban_cookie).unwrap_or_else(|_| "system".to_string())
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::time::Duration;

    use super::*;
    use crate::app::audit::AuditLogFuture;
    use crate::config::SubscriptionCategory;
    use crate::subscription::NewOperationLogEntry;

    #[derive(Default)]
    struct FakeProvider {
        searches: Mutex<Vec<(String, String, usize, usize)>>,
        marks: Mutex<Vec<(String, MarkDoubanInterestCommand)>>,
        libraries: Mutex<Vec<(String, String, usize)>>,
        search_result: Mutex<Option<Result<DoubanSearchOutcome, DoubanCatalogError>>>,
        mark_result: Mutex<Option<Result<DoubanInterestResult, DoubanCatalogError>>>,
        library_results: Mutex<Vec<Result<DoubanLibraryList, DoubanCatalogError>>>,
    }

    impl DoubanCatalogProvider for FakeProvider {
        fn search(
            &self,
            cookie: String,
            query: String,
            page: usize,
            page_size: usize,
        ) -> ProviderFuture<DoubanSearchOutcome> {
            self.searches
                .lock()
                .unwrap()
                .push((cookie, query, page, page_size));
            let result = self.search_result.lock().unwrap().take().unwrap();
            Box::pin(async move { result })
        }

        fn subject_detail(
            &self,
            _cookie: String,
            _subject_id: String,
        ) -> ProviderFuture<DoubanSubjectDetail> {
            Box::pin(async {
                Err(DoubanCatalogError::Upstream {
                    message: "unused".to_string(),
                })
            })
        }

        fn mark_interest(
            &self,
            cookie: String,
            command: MarkDoubanInterestCommand,
        ) -> ProviderFuture<DoubanInterestResult> {
            self.marks.lock().unwrap().push((cookie, command));
            let result = self.mark_result.lock().unwrap().take().unwrap();
            Box::pin(async move { result })
        }

        fn library(
            &self,
            cookie: String,
            status: douban::DoubanLibraryStatus,
            limit: usize,
        ) -> ProviderFuture<DoubanLibraryList> {
            self.libraries
                .lock()
                .unwrap()
                .push((cookie, status.as_str().to_string(), limit));
            let result = self.library_results.lock().unwrap().remove(0);
            Box::pin(async move { result })
        }

        fn qr_start(&self) -> ProviderFuture<DoubanQrStartProviderOutcome> {
            Box::pin(async {
                Err(DoubanCatalogError::Upstream {
                    message: "unused".to_string(),
                })
            })
        }

        fn qr_poll(
            &self,
            _session: douban::QrSession,
        ) -> ProviderFuture<DoubanQrPollProviderOutcome> {
            Box::pin(async {
                Err(DoubanCatalogError::Upstream {
                    message: "unused".to_string(),
                })
            })
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

    fn category(name: &str, wanted_tag: &str) -> SubscriptionCategory {
        SubscriptionCategory {
            name: name.to_string(),
            wanted_tag: wanted_tag.to_string(),
            qb_server_id: String::new(),
            qb_category: String::new(),
            qb_save_dir_name: String::new(),
            download_dir: String::new(),
            link_target_dir: String::new(),
        }
    }

    fn config() -> FileConfig {
        FileConfig {
            douban_cookie: "dbcl2=account-1:secret; ck=test".to_string(),
            subscription_categories: vec![category("电影", "电影"), category("剧集", "剧集")],
            ..FileConfig::default()
        }
    }

    fn service(
        label: &str,
        provider: Arc<FakeProvider>,
        audit: Arc<RecordingAudit>,
    ) -> (DoubanCatalogService, TmdbDiskCache) {
        let root = std::env::temp_dir().join(format!(
            "douban-catalog-{label}-{}-{}",
            std::process::id(),
            unix_now_secs()
        ));
        let cache = TmdbDiskCache::new(root.clone(), Duration::from_secs(60));
        let manager = ConfigManager::new(root.join("config.toml"), config());
        (
            DoubanCatalogService::with_provider(manager, provider, cache.clone(), 60, audit),
            cache,
        )
    }

    #[tokio::test]
    async fn search_clamps_transport_values_and_records_one_success() {
        let provider = Arc::new(FakeProvider::default());
        *provider.search_result.lock().unwrap() = Some(Ok(DoubanSearchOutcome {
            items: Vec::new(),
            page: 1,
            page_size: 20,
            has_more: false,
        }));
        let audit = Arc::new(RecordingAudit::default());
        let (service, _) = service("search", provider.clone(), audit.clone());

        service
            .search(DoubanSearchCommand {
                query: "  movie  ".to_string(),
                page: 0,
                page_size: 100,
            })
            .await
            .unwrap();

        assert_eq!(provider.searches.lock().unwrap()[0].2, 1);
        assert_eq!(provider.searches.lock().unwrap()[0].3, 20);
        let entries = audit.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "success");
        assert_eq!(entries[0].target_title.as_deref(), Some("movie"));
    }

    fn library_list(status: &str) -> DoubanLibraryList {
        DoubanLibraryList {
            status: status.to_string(),
            label: if status == "wish" { "想看" } else { "看过" }.to_string(),
            items: Vec::new(),
            completeness: DoubanSnapshotCompleteness::Complete,
            fetched_pages: 1,
            truncated_by_limit: false,
            end_observed: true,
        }
    }

    #[tokio::test]
    async fn library_owns_provider_fanout_and_typed_cache_hits() {
        let provider = Arc::new(FakeProvider::default());
        provider
            .library_results
            .lock()
            .unwrap()
            .extend([Ok(library_list("wish")), Ok(library_list("collect"))]);
        let audit = Arc::new(RecordingAudit::default());
        let (service, _) = service("library", provider.clone(), audit);

        let first = service
            .library(DoubanLibraryCommand {
                force_refresh: false,
                limit: 5_000,
            })
            .await
            .unwrap();
        assert!(!first.cached);
        assert_eq!(first.limit, MAX_LIBRARY_LIMIT);
        assert_eq!(first.ttl_seconds, 60);
        assert_eq!(provider.libraries.lock().unwrap().len(), 2);

        let cached = service
            .library(DoubanLibraryCommand {
                force_refresh: false,
                limit: 5_000,
            })
            .await
            .unwrap();
        assert!(cached.cached);
        assert_eq!(cached.wish.status, "wish");
        assert_eq!(cached.collect.status, "collect");
        assert_eq!(provider.libraries.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn tag_history_returns_only_configured_categories_in_ranked_closed_shape() {
        let provider = Arc::new(FakeProvider::default());
        let audit = Arc::new(RecordingAudit::default());
        let (service, cache) = service("tags", provider, audit);
        cache
            .put(
                &douban_tag_history_cache_key("account-1"),
                &json!({
                    "source": "local-cache",
                    "cached": true,
                    "updated_at": 123,
                    "tags": ["未配置", "电影", "剧集"],
                    "tag_counts": [
                        { "tag": "未配置", "count": 99 },
                        { "tag": "电影", "count": 2 },
                        { "tag": "剧集", "count": 5 }
                    ]
                }),
            )
            .await
            .unwrap();

        let outcome = service
            .tag_history(DoubanTagHistoryCommand { limit: 1 })
            .await
            .unwrap();
        assert_eq!(outcome.updated_at, Some(123));
        assert_eq!(outcome.tags, vec!["剧集"]);
        assert_eq!(outcome.tag_counts[0].count, 5);
        assert_eq!(outcome.tag_counts[0].category, "剧集");
        assert_eq!(outcome.subscription_categories.len(), 2);
    }

    #[tokio::test]
    async fn qr_session_lookup_is_owned_by_service_and_rejects_unknown_ids() {
        let provider = Arc::new(FakeProvider::default());
        let audit = Arc::new(RecordingAudit::default());
        let (service, _) = service("qr-session", provider, audit);

        assert!(matches!(
            service.qr_image(" ".to_string()).await.unwrap_err(),
            DoubanCatalogError::Validation { .. }
        ));
        assert!(matches!(
            service.poll_qr("missing".to_string()).await.unwrap_err(),
            DoubanCatalogError::Validation { .. }
        ));
    }

    #[tokio::test]
    async fn wish_validation_and_successful_interest_own_cache_and_history_side_effects() {
        let provider = Arc::new(FakeProvider::default());
        *provider.mark_result.lock().unwrap() = Some(Ok(DoubanInterestResult {
            ok: true,
            interest: DoubanInterest::Wish,
            rating: None,
            tags: "电影".to_string(),
        }));
        let audit = Arc::new(RecordingAudit::default());
        let (service, cache) = service("interest", provider.clone(), audit.clone());
        cache
            .put("library_account-1_limit_200", &json!({ "cached": true }))
            .await
            .unwrap();
        cache
            .put(
                &douban_tag_history_cache_key("account-1"),
                &json!({
                    "tags": ["电影"],
                    "tag_counts": [{ "tag": "电影", "count": 2 }]
                }),
            )
            .await
            .unwrap();

        let outcome = service
            .mark_interest(MarkDoubanInterestCommand {
                subject_id: "1292052".to_string(),
                interest: DoubanInterest::Wish,
                rating: None,
                tags: " 电影 ".to_string(),
            })
            .await
            .unwrap();

        assert_eq!(outcome.tags, "电影");
        assert_eq!(provider.marks.lock().unwrap()[0].1.tags, "电影");
        assert!(cache.get_any("library_account-1_limit_200").await.is_none());
        let history = cache
            .get_any(&douban_tag_history_cache_key("account-1"))
            .await
            .unwrap();
        assert_eq!(history["tag_counts"][0]["count"], 3);
        let entries = audit.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "success");
    }

    #[tokio::test]
    async fn invalid_wish_category_stops_before_provider_and_audit() {
        let provider = Arc::new(FakeProvider::default());
        let audit = Arc::new(RecordingAudit::default());
        let (service, _) = service("invalid-tag", provider.clone(), audit.clone());

        let error = service
            .mark_interest(MarkDoubanInterestCommand {
                subject_id: "1292052".to_string(),
                interest: DoubanInterest::Wish,
                rating: None,
                tags: "纪录片".to_string(),
            })
            .await
            .unwrap_err();

        assert!(matches!(error, DoubanCatalogError::Validation { .. }));
        assert!(provider.marks.lock().unwrap().is_empty());
        assert!(audit.entries.lock().unwrap().is_empty());
    }
}
