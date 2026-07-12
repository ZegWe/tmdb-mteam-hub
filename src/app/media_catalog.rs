use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::audit::{operation_log_entry, AuditLogPort, OperationLogEvent};
use crate::clients::http::ClientError;
use crate::clients::tmdb::TmdbClient;
use crate::config::{ConfigManager, FileConfig};
use crate::douban;
use crate::tmdb_cache::TmdbDiskCache;

const POSTER_BASE: &str = "https://image.tmdb.org/t/p/w500";
const STILL_BASE: &str = "https://image.tmdb.org/t/p/w185";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TmdbMediaKind {
    Movie,
    Tv,
}

impl TmdbMediaKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "tv",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MediaSearchItem {
    pub(crate) media_kind: TmdbMediaKind,
    pub(crate) id: i32,
    pub(crate) title: String,
    pub(crate) original_title: String,
    pub(crate) overview: String,
    pub(crate) poster_path: Option<String>,
    pub(crate) poster_url: Option<String>,
    pub(crate) release_date: Option<String>,
    pub(crate) first_air_date: Option<String>,
    pub(crate) vote_average: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MediaSearchOutcome {
    pub(crate) movies: Vec<MediaSearchItem>,
    pub(crate) tv: Vec<MediaSearchItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NamedMediaValue {
    pub(crate) id: Option<i32>,
    pub(crate) name: String,
    pub(crate) english_name: Option<String>,
    pub(crate) code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TmdbSeasonSummary {
    pub(crate) season_number: i32,
    pub(crate) name: String,
    pub(crate) episode_count: Option<u32>,
    pub(crate) air_date: Option<String>,
    pub(crate) poster_path: Option<String>,
    pub(crate) poster_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TmdbMediaDetail {
    pub(crate) media_kind: TmdbMediaKind,
    pub(crate) id: i32,
    pub(crate) title: String,
    pub(crate) original_title: String,
    pub(crate) overview: String,
    pub(crate) tagline: Option<String>,
    pub(crate) poster_path: Option<String>,
    pub(crate) poster_url: Option<String>,
    pub(crate) backdrop_path: Option<String>,
    pub(crate) backdrop_url: Option<String>,
    pub(crate) release_date: Option<String>,
    pub(crate) first_air_date: Option<String>,
    pub(crate) last_air_date: Option<String>,
    pub(crate) runtime: Option<u32>,
    pub(crate) status: Option<String>,
    pub(crate) vote_average: Option<f64>,
    pub(crate) vote_count: Option<u64>,
    pub(crate) genres: Vec<NamedMediaValue>,
    pub(crate) production_countries: Vec<NamedMediaValue>,
    pub(crate) spoken_languages: Vec<NamedMediaValue>,
    pub(crate) origin_country: Vec<String>,
    pub(crate) imdb_id: Option<String>,
    pub(crate) douban_id: Option<String>,
    pub(crate) douban_url: Option<String>,
    pub(crate) number_of_seasons: Option<u32>,
    pub(crate) number_of_episodes: Option<u32>,
    pub(crate) episode_run_time: Vec<u32>,
    pub(crate) networks: Vec<NamedMediaValue>,
    pub(crate) series_type: Option<String>,
    pub(crate) seasons: Vec<TmdbSeasonSummary>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TmdbEpisode {
    pub(crate) id: Option<i32>,
    pub(crate) episode_number: i32,
    pub(crate) name: String,
    pub(crate) overview: String,
    pub(crate) air_date: Option<String>,
    pub(crate) still_path: Option<String>,
    pub(crate) still_url: Option<String>,
    pub(crate) runtime: Option<u32>,
    pub(crate) vote_average: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TmdbSeasonDetail {
    pub(crate) id: Option<i32>,
    pub(crate) season_number: i32,
    pub(crate) name: String,
    pub(crate) overview: String,
    pub(crate) air_date: Option<String>,
    pub(crate) poster_path: Option<String>,
    pub(crate) poster_url: Option<String>,
    pub(crate) episodes: Vec<TmdbEpisode>,
}

#[derive(Debug)]
pub(crate) enum MediaCatalogError {
    Validation { message: String },
    Upstream(ClientError),
}

impl MediaCatalogError {
    fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }
}

type MediaProviderFuture =
    Pin<Box<dyn Future<Output = Result<Value, ClientError>> + Send + 'static>>;

trait MediaCatalogProvider: Send + Sync {
    fn get_json(
        &self,
        credential: String,
        path: String,
        query: Vec<(String, String)>,
    ) -> MediaProviderFuture;
}

#[derive(Clone)]
struct LiveMediaCatalogProvider {
    client: TmdbClient,
}

impl MediaCatalogProvider for LiveMediaCatalogProvider {
    fn get_json(
        &self,
        credential: String,
        path: String,
        query: Vec<(String, String)>,
    ) -> MediaProviderFuture {
        let client = self.client.clone();
        Box::pin(async move {
            let query = query
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect::<Vec<_>>();
            client.get_json(&credential, &path, &query).await
        })
    }
}

#[derive(Clone)]
pub(crate) struct MediaCatalogService {
    config: ConfigManager,
    provider: Arc<dyn MediaCatalogProvider>,
    cache: TmdbDiskCache,
    audit: Arc<dyn AuditLogPort>,
}

impl MediaCatalogService {
    pub(crate) fn new(
        config: ConfigManager,
        tmdb: TmdbClient,
        cache: TmdbDiskCache,
        audit: Arc<dyn AuditLogPort>,
    ) -> Self {
        Self {
            config,
            provider: Arc::new(LiveMediaCatalogProvider { client: tmdb }),
            cache,
            audit,
        }
    }

    #[cfg(test)]
    fn with_provider(
        config: ConfigManager,
        provider: Arc<dyn MediaCatalogProvider>,
        cache: TmdbDiskCache,
        audit: Arc<dyn AuditLogPort>,
    ) -> Self {
        Self {
            config,
            provider,
            cache,
            audit,
        }
    }

    pub(crate) async fn search(
        &self,
        query: String,
        language: Option<String>,
    ) -> Result<MediaSearchOutcome, MediaCatalogError> {
        let config = self.config.snapshot().await.value;
        let credential = match Self::credential(&config) {
            Ok(credential) => credential,
            Err(error) => {
                self.record_search_failure(
                    &config,
                    None,
                    "TMDB 搜索失败：缺少 API Key",
                    "请在设置中填写 TMDB API Key",
                )
                .await;
                return Err(error);
            }
        };
        let query = query.trim();
        if query.is_empty() {
            self.record_search_failure(
                &config,
                None,
                "TMDB 搜索失败：关键词为空",
                "搜索关键字不能为空",
            )
            .await;
            return Err(MediaCatalogError::validation("搜索关键字不能为空"));
        }
        let language = language
            .as_deref()
            .map(str::trim)
            .filter(|language| !language.is_empty())
            .unwrap_or("zh-CN");
        let movie_query = vec![
            ("query".to_string(), query.to_string()),
            ("language".to_string(), language.to_string()),
            ("page".to_string(), "1".to_string()),
            ("include_adult".to_string(), "false".to_string()),
        ];
        let tv_query = vec![
            ("query".to_string(), query.to_string()),
            ("language".to_string(), language.to_string()),
            ("page".to_string(), "1".to_string()),
        ];
        let movies = self.provider.get_json(
            credential.to_string(),
            "/search/movie".to_string(),
            movie_query,
        );
        let tv = self
            .provider
            .get_json(credential.to_string(), "/search/tv".to_string(), tv_query);
        let (movies, tv) = match tokio::try_join!(movies, tv) {
            Ok(result) => result,
            Err(error) => {
                self.record_search_failure(
                    &config,
                    Some(query),
                    "TMDB 搜索失败",
                    &error.to_string(),
                )
                .await;
                return Err(MediaCatalogError::Upstream(error));
            }
        };
        let outcome = MediaSearchOutcome {
            movies: parse_search_page(&movies, TmdbMediaKind::Movie),
            tv: parse_search_page(&tv, TmdbMediaKind::Tv),
        };
        self.record_search_success(&config, query, &outcome).await;
        Ok(outcome)
    }

    pub(crate) async fn movie_detail(
        &self,
        id: i32,
        force_refresh: bool,
    ) -> Result<TmdbMediaDetail, MediaCatalogError> {
        self.media_detail(TmdbMediaKind::Movie, id, force_refresh)
            .await
    }

    pub(crate) async fn tv_detail(
        &self,
        id: i32,
        force_refresh: bool,
    ) -> Result<TmdbMediaDetail, MediaCatalogError> {
        self.media_detail(TmdbMediaKind::Tv, id, force_refresh)
            .await
    }

    pub(crate) async fn tv_season_detail(
        &self,
        tv_id: i32,
        season_number: i32,
        force_refresh: bool,
    ) -> Result<TmdbSeasonDetail, MediaCatalogError> {
        let config = self.config.snapshot().await.value;
        let credential = Self::credential(&config)?;
        let cache_key = format!("tv_{tv_id}_s{season_number}");
        let value = if !force_refresh {
            self.cache.get(&cache_key).await
        } else {
            None
        };
        let value = match value {
            Some(value) => value,
            None => {
                let value = self
                    .provider
                    .get_json(
                        credential.to_string(),
                        format!("/tv/{tv_id}/season/{season_number}"),
                        vec![("language".to_string(), "zh-CN".to_string())],
                    )
                    .await
                    .map_err(MediaCatalogError::Upstream)?;
                if let Err(error) = self.cache.put(&cache_key, &value).await {
                    tracing::warn!("tmdb tv season cache write failed: {error}");
                }
                value
            }
        };
        parse_season_detail(&value).map_err(MediaCatalogError::Upstream)
    }

    async fn media_detail(
        &self,
        media_kind: TmdbMediaKind,
        id: i32,
        force_refresh: bool,
    ) -> Result<TmdbMediaDetail, MediaCatalogError> {
        let config = self.config.snapshot().await.value;
        let credential = Self::credential(&config)?;
        let cache_key = format!("{}_{id}", media_kind.as_str());
        let value = if !force_refresh {
            self.cache.get(&cache_key).await
        } else {
            None
        };
        let value = match value {
            Some(value) => value,
            None => {
                let value = self
                    .provider
                    .get_json(
                        credential.to_string(),
                        format!("/{}/{id}", media_kind.as_str()),
                        vec![
                            ("language".to_string(), "zh-CN".to_string()),
                            ("append_to_response".to_string(), "external_ids".to_string()),
                        ],
                    )
                    .await
                    .map_err(MediaCatalogError::Upstream)?;
                if let Err(error) = self.cache.put(&cache_key, &value).await {
                    tracing::warn!("tmdb {} cache write failed: {error}", media_kind.as_str());
                }
                value
            }
        };
        parse_media_detail(&value, media_kind).map_err(MediaCatalogError::Upstream)
    }

    fn credential(config: &FileConfig) -> Result<&str, MediaCatalogError> {
        let credential = config.tmdb_api_key.trim();
        if credential.is_empty() {
            Err(MediaCatalogError::validation("请在设置中填写 TMDB API Key"))
        } else {
            Ok(credential)
        }
    }

    async fn record_search_failure(
        &self,
        config: &FileConfig,
        query: Option<&str>,
        summary: &'static str,
        error: &str,
    ) {
        self.record_search(
            config,
            query,
            "failed",
            summary.to_string(),
            Some(error.to_string()),
            0,
            0,
        )
        .await;
    }

    async fn record_search_success(
        &self,
        config: &FileConfig,
        query: &str,
        outcome: &MediaSearchOutcome,
    ) {
        self.record_search(
            config,
            Some(query),
            "success",
            format!(
                "TMDB 搜索完成：电影 {}，剧集 {}",
                outcome.movies.len(),
                outcome.tv.len()
            ),
            None,
            outcome.movies.len(),
            outcome.tv.len(),
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_search(
        &self,
        config: &FileConfig,
        query: Option<&str>,
        status: &'static str,
        summary: String,
        error: Option<String>,
        movie_count: usize,
        tv_count: usize,
    ) {
        let account_key = douban::auth_cache_key_fragment(&config.douban_cookie)
            .unwrap_or_else(|_| "system".to_string());
        let entry = operation_log_entry(
            account_key,
            OperationLogEvent {
                category: "search",
                action: "search_media",
                target_type: "tmdb",
                target_id: None,
                target_title: query.map(str::to_string),
                status,
                summary,
                error,
                related: json!({
                    "source": "tmdb",
                    "movie_count": movie_count,
                    "tv_count": tv_count,
                }),
            },
        );
        if let Err(error) = self.audit.append(entry).await {
            tracing::warn!("operation log write failed: {error}");
        }
    }
}

fn parse_search_page(value: &Value, media_kind: TmdbMediaKind) -> Vec<MediaSearchItem> {
    value
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| parse_search_item(item, media_kind))
        .collect()
}

fn parse_search_item(value: &Value, media_kind: TmdbMediaKind) -> Option<MediaSearchItem> {
    let object = value.as_object()?;
    let id = i32_value(object, "id")?;
    let title = match media_kind {
        TmdbMediaKind::Movie => text(object, "title"),
        TmdbMediaKind::Tv => text(object, "name"),
    }?;
    let original_title = match media_kind {
        TmdbMediaKind::Movie => text(object, "original_title"),
        TmdbMediaKind::Tv => text(object, "original_name"),
    }
    .unwrap_or_else(|| title.clone());
    let poster_path = text(object, "poster_path");
    Some(MediaSearchItem {
        media_kind,
        id,
        title,
        original_title,
        overview: text(object, "overview").unwrap_or_default(),
        poster_url: image_url(POSTER_BASE, poster_path.as_deref()),
        poster_path,
        release_date: (media_kind == TmdbMediaKind::Movie)
            .then(|| text(object, "release_date"))
            .flatten(),
        first_air_date: (media_kind == TmdbMediaKind::Tv)
            .then(|| text(object, "first_air_date"))
            .flatten(),
        vote_average: f64_value(object, "vote_average"),
    })
}

fn parse_media_detail(
    value: &Value,
    media_kind: TmdbMediaKind,
) -> Result<TmdbMediaDetail, ClientError> {
    let object = value
        .as_object()
        .ok_or_else(|| ClientError::protocol("tmdb", "detail response must be an object"))?;
    let id = i32_value(object, "id")
        .ok_or_else(|| ClientError::protocol("tmdb", "detail response id is missing"))?;
    let title = match media_kind {
        TmdbMediaKind::Movie => text(object, "title"),
        TmdbMediaKind::Tv => text(object, "name"),
    }
    .ok_or_else(|| ClientError::protocol("tmdb", "detail response title is missing"))?;
    let original_title = match media_kind {
        TmdbMediaKind::Movie => text(object, "original_title"),
        TmdbMediaKind::Tv => text(object, "original_name"),
    }
    .unwrap_or_else(|| title.clone());
    let external_ids = object.get("external_ids").and_then(Value::as_object);
    let poster_path = text(object, "poster_path");
    let backdrop_path = text(object, "backdrop_path");
    Ok(TmdbMediaDetail {
        media_kind,
        id,
        title,
        original_title,
        overview: text(object, "overview").unwrap_or_default(),
        tagline: text(object, "tagline"),
        poster_url: image_url(POSTER_BASE, poster_path.as_deref()),
        poster_path,
        backdrop_url: image_url(POSTER_BASE, backdrop_path.as_deref()),
        backdrop_path,
        release_date: text(object, "release_date"),
        first_air_date: text(object, "first_air_date"),
        last_air_date: text(object, "last_air_date"),
        runtime: u32_value(object, "runtime"),
        status: text(object, "status"),
        vote_average: f64_value(object, "vote_average"),
        vote_count: u64_value(object, "vote_count"),
        genres: named_values(object, "genres"),
        production_countries: named_values(object, "production_countries"),
        spoken_languages: named_values(object, "spoken_languages"),
        origin_country: string_values(object, "origin_country"),
        imdb_id: text(object, "imdb_id")
            .or_else(|| external_ids.and_then(|ids| text(ids, "imdb_id"))),
        douban_id: text(object, "douban_id").or_else(|| {
            external_ids.and_then(|ids| text(ids, "douban_id").or_else(|| text(ids, "douban")))
        }),
        douban_url: text(object, "douban_url"),
        number_of_seasons: u32_value(object, "number_of_seasons"),
        number_of_episodes: u32_value(object, "number_of_episodes"),
        episode_run_time: u32_values(object, "episode_run_time"),
        networks: named_values(object, "networks"),
        series_type: text(object, "type"),
        seasons: season_summaries(object),
    })
}

fn parse_season_detail(value: &Value) -> Result<TmdbSeasonDetail, ClientError> {
    let object = value
        .as_object()
        .ok_or_else(|| ClientError::protocol("tmdb", "season response must be an object"))?;
    let season_number = i32_value(object, "season_number")
        .ok_or_else(|| ClientError::protocol("tmdb", "season number is missing"))?;
    let poster_path = text(object, "poster_path");
    let episodes = object
        .get("episodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(parse_episode)
        .collect();
    Ok(TmdbSeasonDetail {
        id: i32_value(object, "id"),
        season_number,
        name: text(object, "name").unwrap_or_else(|| format!("Season {season_number}")),
        overview: text(object, "overview").unwrap_or_default(),
        air_date: text(object, "air_date"),
        poster_url: image_url(POSTER_BASE, poster_path.as_deref()),
        poster_path,
        episodes,
    })
}

fn parse_episode(value: &Value) -> Option<TmdbEpisode> {
    let object = value.as_object()?;
    let episode_number = i32_value(object, "episode_number")?;
    let still_path = text(object, "still_path");
    Some(TmdbEpisode {
        id: i32_value(object, "id"),
        episode_number,
        name: text(object, "name").unwrap_or_else(|| format!("Episode {episode_number}")),
        overview: text(object, "overview").unwrap_or_default(),
        air_date: text(object, "air_date"),
        still_url: image_url(STILL_BASE, still_path.as_deref()),
        still_path,
        runtime: u32_value(object, "runtime"),
        vote_average: f64_value(object, "vote_average"),
    })
}

fn season_summaries(object: &Map<String, Value>) -> Vec<TmdbSeasonSummary> {
    object
        .get("seasons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let object = value.as_object()?;
            let season_number = i32_value(object, "season_number")?;
            let poster_path = text(object, "poster_path");
            Some(TmdbSeasonSummary {
                season_number,
                name: text(object, "name").unwrap_or_else(|| format!("Season {season_number}")),
                episode_count: u32_value(object, "episode_count"),
                air_date: text(object, "air_date"),
                poster_url: image_url(POSTER_BASE, poster_path.as_deref()),
                poster_path,
            })
        })
        .collect()
}

fn named_values(object: &Map<String, Value>, key: &str) -> Vec<NamedMediaValue> {
    object
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let object = value.as_object()?;
            let name = text(object, "name")?;
            Some(NamedMediaValue {
                id: i32_value(object, "id"),
                name,
                english_name: text(object, "english_name"),
                code: text(object, "iso_3166_1").or_else(|| text(object, "iso_639_1")),
            })
        })
        .collect()
}

fn string_values(object: &Map<String, Value>, key: &str) -> Vec<String> {
    object
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn u32_values(object: &Map<String, Value>, key: &str) -> Vec<u32> {
    object
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|value| u32::try_from(value).ok())
        .collect()
}

fn text(object: &Map<String, Value>, key: &str) -> Option<String> {
    object
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn i32_value(object: &Map<String, Value>, key: &str) -> Option<i32> {
    i32::try_from(object.get(key)?.as_i64()?).ok()
}

fn u32_value(object: &Map<String, Value>, key: &str) -> Option<u32> {
    u32::try_from(object.get(key)?.as_u64()?).ok()
}

fn u64_value(object: &Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key)?.as_u64()
}

fn f64_value(object: &Map<String, Value>, key: &str) -> Option<f64> {
    object.get(key)?.as_f64().filter(|value| value.is_finite())
}

fn image_url(base: &str, path: Option<&str>) -> Option<String> {
    path.filter(|path| path.starts_with('/'))
        .map(|path| format!("{base}{path}"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::app::audit::AuditLogFuture;
    use crate::subscription::NewOperationLogEntry;

    type RecordedRequest = (String, String, Vec<(String, String)>);

    struct FakeProvider {
        responses: Mutex<HashMap<String, Result<Value, ClientError>>>,
        requests: Mutex<Vec<RecordedRequest>>,
    }

    impl FakeProvider {
        fn new(
            responses: impl IntoIterator<Item = (&'static str, Result<Value, ClientError>)>,
        ) -> Self {
            Self {
                responses: Mutex::new(
                    responses
                        .into_iter()
                        .map(|(path, response)| (path.to_string(), response))
                        .collect(),
                ),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl MediaCatalogProvider for FakeProvider {
        fn get_json(
            &self,
            credential: String,
            path: String,
            query: Vec<(String, String)>,
        ) -> MediaProviderFuture {
            self.requests
                .lock()
                .unwrap()
                .push((credential, path.clone(), query));
            let response = self
                .responses
                .lock()
                .unwrap()
                .remove(&path)
                .unwrap_or_else(|| {
                    Err(ClientError::protocol(
                        "tmdb",
                        format!("unexpected fake request path: {path}"),
                    ))
                });
            Box::pin(async move { response })
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
    ) -> MediaCatalogService {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "media-catalog-{label}-{}-{nonce}",
            std::process::id()
        ));
        let config = FileConfig {
            tmdb_api_key: "tmdb-test-credential".to_string(),
            douban_cookie: "dbcl2=account-1:secret; ck=test".to_string(),
            ..FileConfig::default()
        };
        MediaCatalogService::with_provider(
            ConfigManager::new(root.join("config.toml"), config),
            provider,
            TmdbDiskCache::new(root.join("cache"), Duration::from_secs(60)),
            audit,
        )
    }

    #[tokio::test]
    async fn search_uses_the_provider_port_and_records_one_success() {
        let provider = Arc::new(FakeProvider::new([
            (
                "/search/movie",
                Ok(json!({
                    "results": [{
                        "id": 42,
                        "title": "电影",
                        "original_title": "Movie",
                        "poster_path": "/movie.jpg"
                    }]
                })),
            ),
            (
                "/search/tv",
                Ok(json!({
                    "results": [{
                        "id": 84,
                        "name": "剧集",
                        "original_name": "Series"
                    }]
                })),
            ),
        ]));
        let audit = Arc::new(RecordingAudit::default());
        let service = service("search-success", provider.clone(), audit.clone());

        let outcome = service
            .search("  keyword  ".to_string(), Some(" en-US ".to_string()))
            .await
            .unwrap();

        assert_eq!(outcome.movies[0].title, "电影");
        assert_eq!(outcome.tv[0].title, "剧集");
        let requests = provider.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests
            .iter()
            .all(|request| request.0 == "tmdb-test-credential"));
        assert!(requests.iter().all(|request| {
            request
                .2
                .contains(&("language".to_string(), "en-US".to_string()))
        }));
        let entries = audit.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "success");
        assert_eq!(entries[0].target_title.as_deref(), Some("keyword"));
        assert_eq!(entries[0].related["movie_count"], 1);
        assert_eq!(entries[0].related["tv_count"], 1);
    }

    #[tokio::test]
    async fn upstream_search_failure_is_returned_and_audited_once() {
        let provider = Arc::new(FakeProvider::new([
            ("/search/movie", Ok(json!({ "results": [] }))),
            (
                "/search/tv",
                Err(ClientError::unavailable("tmdb", "test upstream failure")),
            ),
        ]));
        let audit = Arc::new(RecordingAudit::default());
        let service = service("search-failure", provider, audit.clone());

        let error = service
            .search("keyword".to_string(), None)
            .await
            .unwrap_err();

        assert!(matches!(error, MediaCatalogError::Upstream(_)));
        let entries = audit.entries.lock().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, "failed");
        assert_eq!(entries[0].target_title.as_deref(), Some("keyword"));
        assert!(entries[0]
            .error
            .as_deref()
            .is_some_and(|message| message.contains("test upstream failure")));
    }

    #[test]
    fn movie_and_tv_provider_fields_map_to_one_detail_vocabulary() {
        let movie = parse_media_detail(
            &json!({
                "id": 42,
                "title": "电影",
                "original_title": "Movie",
                "poster_path": "/movie.jpg",
                "external_ids": { "imdb_id": "tt0042" },
                "runtime": 123
            }),
            TmdbMediaKind::Movie,
        )
        .unwrap();
        let tv = parse_media_detail(
            &json!({
                "id": 84,
                "name": "剧集",
                "original_name": "Series",
                "poster_path": "/tv.jpg",
                "type": "Scripted",
                "seasons": [{ "season_number": 1, "name": "第一季", "episode_count": 8 }]
            }),
            TmdbMediaKind::Tv,
        )
        .unwrap();

        assert_eq!(movie.title, "电影");
        assert_eq!(movie.original_title, "Movie");
        assert_eq!(movie.imdb_id.as_deref(), Some("tt0042"));
        assert_eq!(
            movie.poster_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w500/movie.jpg")
        );
        assert_eq!(tv.title, "剧集");
        assert_eq!(tv.original_title, "Series");
        assert_eq!(tv.series_type.as_deref(), Some("Scripted"));
        assert_eq!(tv.seasons[0].episode_count, Some(8));
    }

    #[test]
    fn season_provider_shape_maps_to_named_episode_contract() {
        let season = parse_season_detail(&json!({
            "id": 9,
            "season_number": 1,
            "name": "第一季",
            "episodes": [{
                "id": 10,
                "episode_number": 2,
                "name": "第二集",
                "still_path": "/episode.jpg"
            }]
        }))
        .unwrap();

        assert_eq!(season.season_number, 1);
        assert_eq!(season.episodes[0].episode_number, 2);
        assert_eq!(
            season.episodes[0].still_url.as_deref(),
            Some("https://image.tmdb.org/t/p/w185/episode.jpg")
        );
    }
}
