use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::app::audit::{AuditLogPort, SqliteAuditLog};
use crate::app::douban_catalog::DoubanCatalogService;
use crate::app::manual_qb::ManualQbService;
use crate::app::media_catalog::MediaCatalogService;
use crate::app::mteam_search::MteamSearchService;
use crate::config::{load_normalized_file_config, ConfigManager, FileConfig, ManagementConfig};
use crate::storage::blocking::BoundedBlockingExecutor;
use crate::storage::operation_log_retention::OperationLogRetention;
use crate::storage::service_lock::{acquire_storage_service_lock, StorageServiceLock};
use crate::storage::SqliteSubscriptionRepository;
use crate::subscription::execution::SubscriptionExecutionService;
use crate::subscription::execution_effects::LatestSubscriptionExecutionEffects;
use crate::subscription::queries::SubscriptionQueryService;
use crate::subscription::wanted_source::DoubanWantedSource;
use crate::subscription::worker::{
    SubscriptionPollService, SubscriptionWorkerHandle, SubscriptionWorkerOptions,
};
use crate::tmdb_cache::TmdbDiskCache;

pub(crate) mod audit;
pub(crate) mod auth_security;
pub(crate) mod douban_catalog;
pub(crate) mod manual_qb;
pub(crate) mod media_catalog;
pub(crate) mod mteam_search;
pub(crate) mod redaction;

pub(crate) const SUBSCRIPTION_DATABASE_FILE_NAME: &str = "subscriptions.sqlite";
const SUBSCRIPTION_SQLITE_MAX_CONCURRENCY: usize = 4;
const SUBSCRIPTION_SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_path: PathBuf,
    pub tmdb_cache_dir: PathBuf,
    pub douban_cache_dir: PathBuf,
    pub subscription_state_dir: PathBuf,
    pub static_dir: PathBuf,
}

impl AppPaths {
    pub fn from_env() -> Self {
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let manifest_static = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static");
        let cwd_static = cwd.join("static");
        let static_dir = if manifest_static.is_dir() {
            manifest_static
        } else if cwd_static.is_dir() {
            cwd_static
        } else {
            manifest_static
        };
        Self {
            config_path: env_path("CONFIG_PATH", cwd.join("config.toml")),
            tmdb_cache_dir: env_path("TMDB_CACHE_DIR", cwd.join("cache").join("tmdb")),
            douban_cache_dir: env_path("DOUBAN_CACHE_DIR", cwd.join("cache").join("douban")),
            subscription_state_dir: env_path(
                "SUBSCRIPTION_STATE_DIR",
                cwd.join("cache").join("subscriptions"),
            ),
            static_dir,
        }
    }

    pub fn for_test_root(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref();
        Self {
            config_path: root.join("config").join("config.toml"),
            tmdb_cache_dir: root.join("cache").join("tmdb"),
            douban_cache_dir: root.join("cache").join("douban"),
            subscription_state_dir: root.join("state").join("subscriptions"),
            static_dir: root.join("static"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BootstrapOptions {
    pub paths: AppPaths,
    pub tmdb_cache_ttl_secs: u64,
    pub douban_cache_ttl_secs: u64,
    pub cache_cleanup_interval_secs: u64,
    pub operation_log_retention_days: u64,
    pub operation_log_max_rows_per_account: u64,
    pub execution_lease_ttl_secs: u64,
    pub execution_batch_size: usize,
    pub execution_concurrency: usize,
    pub execution_idle_interval_secs: u64,
    pub execution_jitter_secs: u64,
    pub filesystem_effect_concurrency: usize,
}

impl BootstrapOptions {
    pub fn from_env() -> Self {
        Self {
            paths: AppPaths::from_env(),
            tmdb_cache_ttl_secs: env_u64("TMDB_CACHE_TTL_SECS", 604_800),
            douban_cache_ttl_secs: env_u64("DOUBAN_CACHE_TTL_SECS", 86_400),
            cache_cleanup_interval_secs: env_u64("CACHE_CLEANUP_INTERVAL_SECS", 21_600),
            operation_log_retention_days: env_u64("OPERATION_LOG_RETENTION_DAYS", 90),
            operation_log_max_rows_per_account: env_u64(
                "OPERATION_LOG_MAX_ROWS_PER_ACCOUNT",
                10_000,
            ),
            execution_lease_ttl_secs: env_u64("EXECUTION_LEASE_TTL_SECS", 900),
            execution_batch_size: env_usize("EXECUTION_BATCH_SIZE", 4),
            execution_concurrency: env_usize("EXECUTION_CONCURRENCY", 2),
            execution_idle_interval_secs: env_u64("EXECUTION_IDLE_INTERVAL_SECS", 15),
            execution_jitter_secs: env_u64("EXECUTION_JITTER_SECS", 5),
            filesystem_effect_concurrency: env_usize("FILESYSTEM_EFFECT_CONCURRENCY", 2),
        }
    }
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) config: ConfigManager,
    pub(crate) startup_management: ManagementConfig,
    pub(crate) upstream_clients: crate::clients::UpstreamClients,
    pub(crate) tmdb_cache: TmdbDiskCache,
    pub(crate) douban_cache: TmdbDiskCache,
    pub(crate) login_rate_limiter: auth_security::LoginRateLimiter,
    pub(crate) subscription_repository: Arc<SqliteSubscriptionRepository>,
    pub(crate) subscription_queries: SubscriptionQueryService,
    pub(crate) subscription_poll: SubscriptionPollService,
    pub(crate) subscription_execution: SubscriptionExecutionService,
    pub(crate) audit_log: Arc<dyn AuditLogPort>,
    pub(crate) douban_catalog: DoubanCatalogService,
    pub(crate) manual_qb: ManualQbService,
    pub(crate) media_catalog: MediaCatalogService,
    pub(crate) mteam_search: MteamSearchService,
    pub(crate) subscription_database_path: PathBuf,
    pub(crate) _storage_service_lock: Option<Arc<StorageServiceLock>>,
}

#[derive(Debug, Clone, Copy)]
struct AppRuntimeOptions {
    tmdb_cache_ttl_secs: u64,
    douban_cache_ttl_secs: u64,
    operation_log_retention: OperationLogRetention,
    execution_lease_ttl_secs: u64,
    filesystem_effect_concurrency: usize,
}

impl AppState {
    fn new(
        paths: &AppPaths,
        config: FileConfig,
        subscription_repository: Arc<SqliteSubscriptionRepository>,
        runtime: AppRuntimeOptions,
        storage_service_lock: Option<Arc<StorageServiceLock>>,
    ) -> Self {
        let startup_management = config.management.clone();
        let config = ConfigManager::new(paths.config_path.clone(), config);
        let upstream_clients = crate::clients::UpstreamClients::new()
            .expect("static upstream client policies must be valid");
        let audit_log: Arc<dyn AuditLogPort> = Arc::new(SqliteAuditLog::new(
            subscription_repository.clone(),
            runtime.operation_log_retention,
        ));
        let tmdb_cache = TmdbDiskCache::new(
            paths.tmdb_cache_dir.clone(),
            Duration::from_secs(runtime.tmdb_cache_ttl_secs),
        );
        let douban_cache = TmdbDiskCache::new(
            paths.douban_cache_dir.clone(),
            Duration::from_secs(runtime.douban_cache_ttl_secs),
        );
        let manual_qb = ManualQbService::new(
            config.clone(),
            upstream_clients.mteam.clone(),
            audit_log.clone(),
        );
        let douban_catalog = DoubanCatalogService::new(
            config.clone(),
            upstream_clients.douban.clone(),
            douban_cache.clone(),
            runtime.douban_cache_ttl_secs,
            audit_log.clone(),
        );
        let media_catalog = MediaCatalogService::new(
            config.clone(),
            upstream_clients.tmdb.clone(),
            tmdb_cache.clone(),
            audit_log.clone(),
        );
        let mteam_search = MteamSearchService::new(
            config.clone(),
            upstream_clients.mteam.clone(),
            audit_log.clone(),
        );
        let subscription_queries = SubscriptionQueryService::new(subscription_repository.clone());
        let subscription_poll = SubscriptionPollService::new(
            subscription_repository.clone(),
            Arc::new(DoubanWantedSource::new(upstream_clients.douban.clone())),
        );
        let execution_effects = LatestSubscriptionExecutionEffects::try_production(
            upstream_clients.mteam.clone(),
            runtime.filesystem_effect_concurrency,
        )
        .expect("filesystem effect concurrency must be positive");
        let subscription_execution = SubscriptionExecutionService::try_new(
            subscription_repository.clone(),
            Arc::new(execution_effects),
            runtime.execution_lease_ttl_secs,
        )
        .expect("execution lease TTL must be valid");
        Self {
            config,
            startup_management,
            upstream_clients,
            tmdb_cache,
            douban_cache,
            login_rate_limiter: auth_security::LoginRateLimiter::default(),
            subscription_repository,
            subscription_queries,
            subscription_poll,
            subscription_execution,
            audit_log,
            douban_catalog,
            manual_qb,
            media_catalog,
            mteam_search,
            subscription_database_path: paths
                .subscription_state_dir
                .join(SUBSCRIPTION_DATABASE_FILE_NAME),
            _storage_service_lock: storage_service_lock,
        }
    }

    pub fn for_test(paths: AppPaths) -> Self {
        let subscription_repository = open_latest_subscription_repository_for_test(&paths);
        Self::new(
            &paths,
            FileConfig::default(),
            subscription_repository,
            AppRuntimeOptions {
                tmdb_cache_ttl_secs: 60,
                douban_cache_ttl_secs: 60,
                operation_log_retention: OperationLogRetention::default(),
                execution_lease_ttl_secs: 900,
                filesystem_effect_concurrency: 2,
            },
            None,
        )
    }

    pub fn for_test_with_config(paths: AppPaths, config: FileConfig) -> Self {
        let subscription_repository = open_latest_subscription_repository_for_test(&paths);
        Self::new(
            &paths,
            config,
            subscription_repository,
            AppRuntimeOptions {
                tmdb_cache_ttl_secs: 60,
                douban_cache_ttl_secs: 60,
                operation_log_retention: OperationLogRetention::default(),
                execution_lease_ttl_secs: 900,
                filesystem_effect_concurrency: 2,
            },
            None,
        )
    }
}

pub struct BootstrappedApp {
    pub state: AppState,
    pub listen_addr: SocketAddr,
    pub static_dir: PathBuf,
    pub(crate) _subscription_worker: SubscriptionWorkerHandle,
}

pub async fn bootstrap(
    options: BootstrapOptions,
) -> Result<BootstrappedApp, Box<dyn std::error::Error + Send + Sync>> {
    let paths = options.paths;
    tracing::info!("config path: {}", paths.config_path.display());
    ensure_cache_roots_are_isolated(&paths).await?;
    let bootstrap_blocking = BoundedBlockingExecutor::try_new("bootstrap", 1)?;
    let config_path = paths.config_path.clone();
    let (storage_service_lock, config, listen_addr) = bootstrap_blocking
        .run(move || -> std::io::Result<_> {
            let storage_service_lock = Arc::new(acquire_storage_service_lock(&config_path)?);
            let config = load_normalized_file_config(&config_path)?;
            let listen_addr = config.listen_addr()?;
            Ok((storage_service_lock, config, listen_addr))
        })
        .await??;
    let subscription_repository = open_latest_subscription_repository(&paths).await?;
    let worker_options = SubscriptionWorkerOptions::try_new(
        options.execution_batch_size,
        options.execution_concurrency,
        options.execution_idle_interval_secs,
        options.execution_jitter_secs,
    )
    .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidInput, message))?;
    let state = AppState::new(
        &paths,
        config,
        subscription_repository,
        AppRuntimeOptions {
            tmdb_cache_ttl_secs: options.tmdb_cache_ttl_secs,
            douban_cache_ttl_secs: options.douban_cache_ttl_secs,
            operation_log_retention: OperationLogRetention::from_limits(
                options.operation_log_retention_days.saturating_mul(86_400),
                options.operation_log_max_rows_per_account,
            ),
            execution_lease_ttl_secs: options.execution_lease_ttl_secs,
            filesystem_effect_concurrency: options.filesystem_effect_concurrency,
        },
        Some(storage_service_lock),
    );

    state.tmdb_cache.ensure_dir().await?;
    log_cache_cleanup("tmdb", state.tmdb_cache.cleanup_expired().await);
    tracing::info!(
        "tmdb cache: dir={} ttl={}s",
        paths.tmdb_cache_dir.display(),
        options.tmdb_cache_ttl_secs
    );
    state.douban_cache.ensure_dir().await?;
    log_cache_cleanup("douban", state.douban_cache.cleanup_expired().await);
    tracing::info!(
        "douban cache: dir={} ttl={}s",
        paths.douban_cache_dir.display(),
        options.douban_cache_ttl_secs
    );
    tracing::info!(
        "subscription state: dir={} db={}",
        paths.subscription_state_dir.display(),
        state.subscription_database_path.display()
    );
    tracing::info!(
        retention_days = options.operation_log_retention_days,
        max_rows_per_account = options.operation_log_max_rows_per_account,
        "operation log retention configured (zero disables the corresponding limit)"
    );

    spawn_cache_cleanup_loop(
        state.tmdb_cache.clone(),
        state.douban_cache.clone(),
        options.cache_cleanup_interval_secs,
    );
    let subscription_worker = crate::subscription::worker::spawn_subscription_worker(
        state.config.clone(),
        state.subscription_poll.clone(),
        state.subscription_execution.clone(),
        worker_options,
    );
    Ok(BootstrappedApp {
        state,
        listen_addr,
        static_dir: paths.static_dir,
        _subscription_worker: subscription_worker,
    })
}

async fn ensure_cache_roots_are_isolated(paths: &AppPaths) -> std::io::Result<()> {
    tokio::fs::create_dir_all(&paths.subscription_state_dir).await?;
    let state_root = tokio::fs::canonicalize(&paths.subscription_state_dir).await?;
    for (name, cache_root) in [
        ("TMDB_CACHE_DIR", &paths.tmdb_cache_dir),
        ("DOUBAN_CACHE_DIR", &paths.douban_cache_dir),
    ] {
        tokio::fs::create_dir_all(cache_root).await?;
        let cache_root = tokio::fs::canonicalize(cache_root).await?;
        if cache_root.starts_with(&state_root) || state_root.starts_with(&cache_root) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "{name} must not overlap SUBSCRIPTION_STATE_DIR ({})",
                    state_root.display()
                ),
            ));
        }
    }
    Ok(())
}

async fn open_latest_subscription_repository(
    paths: &AppPaths,
) -> Result<Arc<SqliteSubscriptionRepository>, Box<dyn std::error::Error + Send + Sync>> {
    let blocking = BoundedBlockingExecutor::try_new("subscription-bootstrap", 1)?;
    let subscription_state_dir = paths.subscription_state_dir.clone();
    let repository = blocking
        .run(
            move || -> Result<SqliteSubscriptionRepository, Box<dyn std::error::Error + Send + Sync>> {
                std::fs::create_dir_all(&subscription_state_dir)?;
                let database_path = subscription_state_dir.join(SUBSCRIPTION_DATABASE_FILE_NAME);
                let repository = if database_path.try_exists()? {
                    SqliteSubscriptionRepository::try_new(
                        database_path,
                        SUBSCRIPTION_SQLITE_MAX_CONCURRENCY,
                        SUBSCRIPTION_SQLITE_BUSY_TIMEOUT,
                    )?
                } else {
                    SqliteSubscriptionRepository::try_create_fresh(
                        database_path,
                        SUBSCRIPTION_SQLITE_MAX_CONCURRENCY,
                        SUBSCRIPTION_SQLITE_BUSY_TIMEOUT,
                    )?
                };
                Ok(repository)
            },
        )
        .await??;
    repository.preflight().await?;
    Ok(Arc::new(repository))
}

fn open_latest_subscription_repository_for_test(
    paths: &AppPaths,
) -> Arc<SqliteSubscriptionRepository> {
    std::fs::create_dir_all(&paths.subscription_state_dir)
        .expect("create latest subscription test state directory");
    let database_path = paths
        .subscription_state_dir
        .join(SUBSCRIPTION_DATABASE_FILE_NAME);
    let repository = if database_path
        .try_exists()
        .expect("inspect latest subscription test database")
    {
        SqliteSubscriptionRepository::try_new(
            database_path,
            SUBSCRIPTION_SQLITE_MAX_CONCURRENCY,
            SUBSCRIPTION_SQLITE_BUSY_TIMEOUT,
        )
    } else {
        SqliteSubscriptionRepository::try_create_fresh(
            database_path,
            SUBSCRIPTION_SQLITE_MAX_CONCURRENCY,
            SUBSCRIPTION_SQLITE_BUSY_TIMEOUT,
        )
    }
    .expect("open latest subscription test database");
    Arc::new(repository)
}

fn spawn_cache_cleanup_loop(
    tmdb_cache: TmdbDiskCache,
    douban_cache: TmdbDiskCache,
    interval_secs: u64,
) {
    if interval_secs == 0 {
        tracing::info!("periodic JSON cache cleanup disabled");
        return;
    }
    tokio::spawn(async move {
        let period = Duration::from_secs(interval_secs);
        let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            log_cache_cleanup("tmdb", tmdb_cache.cleanup_expired().await);
            log_cache_cleanup("douban", douban_cache.cleanup_expired().await);
        }
    });
}

fn log_cache_cleanup(cache: &str, report: crate::tmdb_cache::CacheCleanupReport) {
    if report.scanned == 0 && report.errors == 0 {
        return;
    }
    tracing::info!(
        cache,
        scanned = report.scanned,
        removed = report.removed,
        errors = report.errors,
        "JSON cache cleanup completed"
    );
}

fn env_path(name: &str, default: PathBuf) -> PathBuf {
    std::env::var(name).map(PathBuf::from).unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{
        ensure_cache_roots_are_isolated, open_latest_subscription_repository, AppPaths,
        SUBSCRIPTION_DATABASE_FILE_NAME,
    };
    use crate::storage::operation_log_retention::OperationLogRetention;
    use crate::subscription::ports::{SubscriptionPollRepository, SubscriptionReadRepository};
    use crate::subscription::repository::{
        ApplyCompleteSnapshotCommand, BeginPollCommand, NewRecordPolicy, SnapshotRecord,
        SubscriptionKey, WantedSourcePayload,
    };
    use crate::subscription::{NewOperationLogEntry, OperationLogQuery, SubscriptionMediaKind};

    const BACKUP_ACCOUNT: &str = "backup-account";
    const BACKUP_SUBJECT: &str = "backup-subject";
    const BACKUP_AT: u64 = 1_900_000_000;

    fn temp_test_root(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-app-{label}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create app test root");
        root
    }

    fn current_database_path(paths: &AppPaths) -> PathBuf {
        paths
            .subscription_state_dir
            .join(SUBSCRIPTION_DATABASE_FILE_NAME)
    }

    fn require_non_empty_regular_file(path: &Path) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_file() || metadata.len() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("required backup input is not a non-empty regular file: {path:?}"),
            ));
        }
        Ok(())
    }

    fn copy_current_operational_pair(source: &AppPaths, restore: &AppPaths) -> io::Result<()> {
        let source_database = current_database_path(source);
        require_non_empty_regular_file(&source.config_path)?;
        require_non_empty_regular_file(&source_database)?;

        let restore_config_dir = restore.config_path.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "restore config path has no parent directory",
            )
        })?;
        fs::create_dir_all(restore_config_dir)?;
        fs::create_dir_all(&restore.subscription_state_dir)?;
        fs::copy(&source.config_path, &restore.config_path)?;
        fs::copy(source_database, current_database_path(restore))?;
        Ok(())
    }

    fn backup_operation_log_query() -> OperationLogQuery {
        OperationLogQuery {
            account_key: Some(BACKUP_ACCOUNT.to_string()),
            page: Some(1),
            page_size: Some(100),
            ..OperationLogQuery::default()
        }
    }

    #[tokio::test]
    async fn latest_repository_open_creates_only_current_database_and_ignores_old_files() {
        let root = temp_test_root("fresh-ignore-old");
        let paths = AppPaths::for_test_root(&root);
        fs::create_dir_all(&paths.subscription_state_dir).unwrap();
        let old_database = paths.subscription_state_dir.join("wanted.sqlite");
        let old_json = paths.subscription_state_dir.join("wanted_account.json");
        let database_sentinel = b"OLD_SQLITE_SENTINEL";
        let json_sentinel = b"OLD_JSON_SENTINEL";
        fs::write(&old_database, database_sentinel).unwrap();
        fs::write(&old_json, json_sentinel).unwrap();

        let repository = open_latest_subscription_repository(&paths).await.unwrap();
        repository.preflight().await.unwrap();

        assert!(paths
            .subscription_state_dir
            .join(SUBSCRIPTION_DATABASE_FILE_NAME)
            .is_file());
        assert_eq!(fs::read(&old_database).unwrap(), database_sentinel);
        assert_eq!(fs::read(&old_json).unwrap(), json_sentinel);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn cache_roots_cannot_overlap_latest_subscription_state() {
        let root = temp_test_root("cache-state-isolation");
        let mut paths = AppPaths::for_test_root(&root);
        fs::create_dir_all(&paths.subscription_state_dir).unwrap();
        let old_database = paths.subscription_state_dir.join("wanted.sqlite");
        let old_json = paths.subscription_state_dir.join("wanted_account.json");
        fs::write(&old_database, b"OLD_SQLITE_SENTINEL").unwrap();
        fs::write(&old_json, b"OLD_JSON_SENTINEL").unwrap();

        let state_parent = paths
            .subscription_state_dir
            .parent()
            .expect("test state directory has a parent")
            .to_path_buf();
        for cache_root in [
            paths.subscription_state_dir.clone(),
            paths.subscription_state_dir.join("nested-cache"),
            state_parent,
        ] {
            paths.tmdb_cache_dir = cache_root;
            let error = ensure_cache_roots_are_isolated(&paths).await.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
            assert!(error.to_string().contains("TMDB_CACHE_DIR"));
        }
        assert_eq!(fs::read(&old_database).unwrap(), b"OLD_SQLITE_SENTINEL");
        assert_eq!(fs::read(&old_json).unwrap(), b"OLD_JSON_SENTINEL");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn stopped_operational_copy_restores_latest_detail_and_log_without_old_database() {
        let source_root = temp_test_root("backup-source");
        let source_paths = AppPaths::for_test_root(&source_root);
        fs::create_dir_all(
            source_paths
                .config_path
                .parent()
                .expect("test config path has a parent"),
        )
        .unwrap();
        let config = b"listen_ip = \"127.0.0.1\"\nlisten_port = 8787\n";
        fs::write(&source_paths.config_path, config).unwrap();
        fs::create_dir_all(&source_paths.subscription_state_dir).unwrap();
        let old_database = source_paths.subscription_state_dir.join("wanted.sqlite");
        let old_database_sentinel = b"ARCHIVED_OLD_SQLITE_MUST_STAY_UNTOUCHED";
        fs::write(&old_database, old_database_sentinel).unwrap();

        let repository = open_latest_subscription_repository(&source_paths)
            .await
            .unwrap();
        let poll = repository
            .begin_poll(BeginPollCommand::try_new(BACKUP_ACCOUNT, BACKUP_AT).unwrap())
            .await
            .unwrap();
        repository
            .apply_complete_snapshot(
                ApplyCompleteSnapshotCommand::try_new(
                    BACKUP_ACCOUNT,
                    poll.token,
                    BACKUP_AT,
                    BACKUP_AT + 3_600,
                    NewRecordPolicy::try_new(3, false).unwrap(),
                    vec![SnapshotRecord::try_new(
                        BACKUP_SUBJECT,
                        SubscriptionMediaKind::Movie,
                        true,
                        None,
                        WantedSourcePayload {
                            title: "Backup Evidence Movie".to_string(),
                            release_year: Some(2026),
                            poster_url: "https://example.test/backup-evidence.jpg".to_string(),
                            category_text: Some("backup-evidence".to_string()),
                            tags: vec!["movie".to_string(), "backup".to_string()],
                            douban_sort_time: Some(BACKUP_AT),
                            ..WantedSourcePayload::default()
                        },
                    )
                    .unwrap()],
                )
                .unwrap(),
            )
            .await
            .unwrap();
        repository
            .append_operation_log(
                NewOperationLogEntry {
                    account_key: BACKUP_ACCOUNT.to_string(),
                    created_at: BACKUP_AT + 1,
                    category: "subscription".to_string(),
                    action: "backup_evidence".to_string(),
                    target_type: "subscription".to_string(),
                    target_id: Some(BACKUP_SUBJECT.to_string()),
                    target_title: Some("Backup Evidence Movie".to_string()),
                    status: "success".to_string(),
                    summary: "persisted before stopped backup".to_string(),
                    error: None,
                    related: json!({
                        "account_key": BACKUP_ACCOUNT,
                        "subject_id": BACKUP_SUBJECT,
                        "schema": "latest-only"
                    }),
                },
                OperationLogRetention::default(),
            )
            .await
            .unwrap();

        let key = SubscriptionKey::try_new(BACKUP_ACCOUNT, BACKUP_SUBJECT).unwrap();
        let expected_detail = repository.load_detail(key.clone()).await.unwrap();
        let expected_logs = repository
            .query_operation_logs(backup_operation_log_query())
            .await
            .unwrap();
        assert_eq!(expected_logs.total, 1);
        let expected_logs_json = serde_json::to_value(&expected_logs.items).unwrap();
        drop(repository);

        let source_database = current_database_path(&source_paths);
        let source_database_bytes = fs::read(&source_database).unwrap();
        let restore_root = temp_test_root("backup-restore");
        let restore_paths = AppPaths::for_test_root(&restore_root);
        copy_current_operational_pair(&source_paths, &restore_paths).unwrap();

        assert_eq!(fs::read(&restore_paths.config_path).unwrap(), config);
        assert_eq!(
            fs::read(current_database_path(&restore_paths)).unwrap(),
            source_database_bytes
        );
        assert!(!restore_paths
            .subscription_state_dir
            .join("wanted.sqlite")
            .try_exists()
            .unwrap());

        let restored_repository = open_latest_subscription_repository(&restore_paths)
            .await
            .unwrap();
        restored_repository.preflight().await.unwrap();
        let restored_detail = restored_repository.load_detail(key).await.unwrap();
        let restored_logs = restored_repository
            .query_operation_logs(backup_operation_log_query())
            .await
            .unwrap();
        assert_eq!(restored_detail, expected_detail);
        assert_eq!(restored_logs.total, 1);
        assert_eq!(
            serde_json::to_value(&restored_logs.items).unwrap(),
            expected_logs_json
        );
        drop(restored_repository);

        assert_eq!(fs::read(&source_paths.config_path).unwrap(), config);
        assert_eq!(fs::read(&source_database).unwrap(), source_database_bytes);
        assert_eq!(fs::read(&old_database).unwrap(), old_database_sentinel);
        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(restore_root);
    }

    #[test]
    fn operational_pair_copy_rejects_a_missing_current_database_before_writing_restore() {
        let source_root = temp_test_root("backup-missing-current");
        let source_paths = AppPaths::for_test_root(&source_root);
        fs::create_dir_all(
            source_paths
                .config_path
                .parent()
                .expect("test config path has a parent"),
        )
        .unwrap();
        fs::write(&source_paths.config_path, b"listen_port = 8787\n").unwrap();
        fs::create_dir_all(&source_paths.subscription_state_dir).unwrap();
        let old_database = source_paths.subscription_state_dir.join("wanted.sqlite");
        let old_database_sentinel = b"OLD_DATABASE_IS_NOT_A_CURRENT_BACKUP";
        fs::write(&old_database, old_database_sentinel).unwrap();

        let restore_root = temp_test_root("restore-missing-current");
        let restore_paths = AppPaths::for_test_root(&restore_root);
        let error = copy_current_operational_pair(&source_paths, &restore_paths).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(!restore_paths.config_path.try_exists().unwrap());
        assert!(!current_database_path(&restore_paths).try_exists().unwrap());
        assert!(!restore_paths
            .subscription_state_dir
            .join("wanted.sqlite")
            .try_exists()
            .unwrap());
        assert_eq!(fs::read(&old_database).unwrap(), old_database_sentinel);
        let _ = fs::remove_dir_all(source_root);
        let _ = fs::remove_dir_all(restore_root);
    }

    #[tokio::test]
    async fn existing_latest_database_is_preflighted_without_reinitialization() {
        let root = temp_test_root("existing-current");
        let paths = AppPaths::for_test_root(&root);
        let first = open_latest_subscription_repository(&paths).await.unwrap();
        drop(first);
        let database = paths
            .subscription_state_dir
            .join(SUBSCRIPTION_DATABASE_FILE_NAME);
        let before = fs::read(&database).unwrap();

        let reopened = open_latest_subscription_repository(&paths).await.unwrap();
        reopened.preflight().await.unwrap();

        assert_eq!(fs::read(&database).unwrap(), before);
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn corrupt_restored_latest_database_fails_closed_without_replacement_or_old_file_access()
    {
        let root = temp_test_root("corrupt-restore");
        let paths = AppPaths::for_test_root(&root);
        fs::create_dir_all(
            paths
                .config_path
                .parent()
                .expect("test config path has a parent"),
        )
        .unwrap();
        let config = b"listen_port = 8787\n";
        fs::write(&paths.config_path, config).unwrap();
        fs::create_dir_all(&paths.subscription_state_dir).unwrap();
        let database = current_database_path(&paths);
        let corrupt = b"NOT_A_CURRENT_SQLITE_DATABASE";
        fs::write(&database, corrupt).unwrap();
        let old_database = paths.subscription_state_dir.join("wanted.sqlite");
        let old_database_sentinel = b"OLD_DATABASE_MUST_NOT_BE_USED_AS_A_FALLBACK";
        fs::write(&old_database, old_database_sentinel).unwrap();

        assert!(open_latest_subscription_repository(&paths).await.is_err());
        assert_eq!(fs::read(&paths.config_path).unwrap(), config);
        assert_eq!(fs::read(&database).unwrap(), corrupt);
        assert_eq!(fs::read(&old_database).unwrap(), old_database_sentinel);
        let _ = fs::remove_dir_all(root);
    }
}
