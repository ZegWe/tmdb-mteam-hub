use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use rusqlite::ffi::ErrorCode;
use rusqlite::{Connection, DatabaseName, OpenFlags};

use super::blocking::{BlockingTaskError, BoundedBlockingExecutor};
use crate::storage::schema_v5::{
    initialize_latest_schema, migrate_previous_schema, validate_schema_contract, SCHEMA_VERSION,
};
use crate::subscription::ports::RepoFuture;
use crate::subscription::repository::{RepositoryError, RepositoryResult};

const SQLITE_HEADER_LEN: usize = 100;
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";
const SQLITE_WRITE_VERSION_OFFSET: usize = 18;
const SQLITE_READ_VERSION_OFFSET: usize = 19;
const SQLITE_ROLLBACK_JOURNAL_VERSION: u8 = 1;
const SQLITE_WAL_VERSION: u8 = 2;
const SQLITE_SIDECAR_SUFFIXES: &[&str] = &["-journal", "-wal", "-shm"];
const FRESH_TEMP_ATTEMPTS: usize = 64;

static NEXT_FRESH_TEMP: AtomicU64 = AtomicU64::new(0);

pub(crate) fn migrate_subscription_schema(
    path: &Path,
    busy_timeout: Duration,
) -> RepositoryResult<bool> {
    let mut connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| map_connection_error("open subscription SQLite for migration", error))?;
    connection
        .busy_timeout(busy_timeout)
        .map_err(|error| map_connection_error("configure migration busy timeout", error))?;
    migrate_previous_schema(&mut connection).map_err(|error| RepositoryError::Unavailable {
        message: format!("migrate subscription SQLite to schema {SCHEMA_VERSION}: {error}"),
    })
}

#[derive(Debug)]
struct FreshTempFile {
    path: PathBuf,
    owner: Option<File>,
    armed: bool,
}

impl FreshTempFile {
    fn create(target: &Path) -> RepositoryResult<Self> {
        let file_name = target
            .file_name()
            .ok_or_else(|| RepositoryError::InvalidInput {
                field: "database_path",
                message: "database path must end with a file name".to_string(),
            })?
            .to_string_lossy();
        for _ in 0..FRESH_TEMP_ATTEMPTS {
            let sequence = NEXT_FRESH_TEMP.fetch_add(1, Ordering::Relaxed);
            let candidate = target.with_file_name(format!(
                ".{file_name}.init-{}-{sequence}",
                std::process::id()
            ));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&candidate)
            {
                Ok(owner) => {
                    return Ok(Self {
                        path: candidate,
                        owner: Some(owner),
                        armed: true,
                    });
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(RepositoryError::Unavailable {
                        message: format!(
                            "create temporary latest-schema SQLite beside {}: {error}",
                            target.display()
                        ),
                    });
                }
            }
        }
        Err(RepositoryError::Unavailable {
            message: format!(
                "could not allocate a unique latest-schema SQLite temporary file after {FRESH_TEMP_ATTEMPTS} attempts beside {}",
                target.display()
            ),
        })
    }

    fn verify_identity(&self) -> RepositoryResult<()> {
        let owner = self
            .owner
            .as_ref()
            .ok_or_else(|| RepositoryError::Internal {
                message: "latest-schema SQLite temporary owner was closed before publication"
                    .to_string(),
            })?;
        if same_file_identity(owner, &self.path)? {
            return Ok(());
        }
        Err(RepositoryError::Unavailable {
            message: format!(
                "latest-schema SQLite temporary file identity changed before publication: {}",
                self.path.display()
            ),
        })
    }

    fn cleanup(&mut self) {
        if !self.armed {
            return;
        }
        let owned = self
            .owner
            .as_ref()
            .is_some_and(|owner| same_file_identity(owner, &self.path).unwrap_or(false));
        self.owner.take();
        if owned {
            for suffix in SQLITE_SIDECAR_SUFFIXES {
                let _ = fs::remove_file(sqlite_sidecar_path(&self.path, suffix));
            }
            if fs::remove_file(&self.path).is_ok() {
                self.armed = false;
            }
        }
    }
}

impl Drop for FreshTempFile {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(unix)]
fn same_file_identity(owner: &File, path: &Path) -> RepositoryResult<bool> {
    use std::os::unix::fs::MetadataExt;

    let owner = owner
        .metadata()
        .map_err(|error| RepositoryError::Unavailable {
            message: format!("inspect created SQLite temporary file descriptor: {error}"),
        })?;
    let current = fs::symlink_metadata(path).map_err(|error| RepositoryError::Unavailable {
        message: format!(
            "inspect created SQLite temporary path {}: {error}",
            path.display()
        ),
    })?;
    Ok(current.file_type().is_file()
        && !current.file_type().is_symlink()
        && owner.dev() == current.dev()
        && owner.ino() == current.ino())
}

#[cfg(not(unix))]
fn same_file_identity(owner: &File, path: &Path) -> RepositoryResult<bool> {
    let owner = owner
        .metadata()
        .map_err(|error| RepositoryError::Unavailable {
            message: format!("inspect created SQLite temporary file descriptor: {error}"),
        })?;
    let current = fs::symlink_metadata(path).map_err(|error| RepositoryError::Unavailable {
        message: format!(
            "inspect created SQLite temporary path {}: {error}",
            path.display()
        ),
    })?;
    Ok(owner.is_file() && current.file_type().is_file() && !current.file_type().is_symlink())
}

#[derive(Debug, Clone)]
pub(super) struct SqliteExecutor {
    path: Arc<PathBuf>,
    busy_timeout: Duration,
    blocking: BoundedBlockingExecutor,
}

impl SqliteExecutor {
    pub(super) fn try_new(
        path: impl Into<PathBuf>,
        max_concurrency: usize,
        busy_timeout: Duration,
    ) -> RepositoryResult<Self> {
        let path = path.into();
        if path.as_os_str().is_empty() {
            return Err(RepositoryError::InvalidInput {
                field: "database_path",
                message: "database path must not be empty".to_string(),
            });
        }
        if max_concurrency == 0 {
            return Err(RepositoryError::InvalidInput {
                field: "max_concurrency",
                message: "SQLite concurrency must be greater than zero".to_string(),
            });
        }
        if busy_timeout.is_zero() {
            return Err(RepositoryError::InvalidInput {
                field: "busy_timeout",
                message: "SQLite busy timeout must be greater than zero".to_string(),
            });
        }
        Ok(Self {
            path: Arc::new(path),
            busy_timeout,
            blocking: BoundedBlockingExecutor::try_new("SQLite", max_concurrency).map_err(
                |error| RepositoryError::InvalidInput {
                    field: "max_concurrency",
                    message: error.to_string(),
                },
            )?,
        })
    }

    pub(super) fn run<T, F>(&self, operation: F) -> RepoFuture<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> RepositoryResult<T> + Send + 'static,
    {
        let path = Arc::clone(&self.path);
        let busy_timeout = self.busy_timeout;
        let blocking = self.blocking.clone();
        Box::pin(async move {
            blocking
                .run(move || {
                    let mut connection = open_v5_connection(path.as_path(), busy_timeout)?;
                    operation(&mut connection)
                })
                .await
                .map_err(map_blocking_task_error)?
        })
    }

    pub(super) fn preflight(&self) -> RepoFuture<()> {
        let path = Arc::clone(&self.path);
        let busy_timeout = self.busy_timeout;
        let blocking = self.blocking.clone();
        Box::pin(async move {
            blocking
                .run(move || {
                    // Preflight uses a separate fail-closed read-only open path so bootstrap cannot
                    // recover a hot journal/WAL or create a sidecar as an accidental write.
                    let connection = open_v5_preflight_connection(path.as_path(), busy_timeout)?;
                    validate_runtime_schema(&connection)
                })
                .await
                .map_err(map_blocking_task_error)?
        })
    }
}

fn map_blocking_task_error(error: BlockingTaskError) -> RepositoryError {
    if error.is_closed() {
        RepositoryError::Unavailable {
            message: error.to_string(),
        }
    } else {
        RepositoryError::Internal {
            message: error.to_string(),
        }
    }
}

pub(super) fn open_v5_connection(
    path: &Path,
    busy_timeout: Duration,
) -> RepositoryResult<Connection> {
    let connection = open_unvalidated_writable_connection(path, busy_timeout)?;
    validate_schema_marker(&connection)?;
    Ok(connection)
}

/// Create, initialize, and close one brand-new latest-schema database.
///
/// A same-directory temporary inode is fully initialized, validated, and closed before an atomic
/// no-clobber hard-link publishes the target. An existing path passed accidentally by a caller is
/// never opened, inspected, truncated, or replaced, and an initialization failure never leaves a
/// partial target database behind.
pub(super) fn create_fresh_v5_database(
    path: &Path,
    busy_timeout: Duration,
) -> RepositoryResult<()> {
    create_fresh_v5_database_with(path, busy_timeout, |connection| {
        initialize_latest_schema(connection).map_err(|error| RepositoryError::CorruptData {
            message: format!(
                "initialize new latest-schema SQLite {}: {error}",
                path.display()
            ),
        })?;
        validate_schema_marker(connection)?;
        validate_runtime_schema(connection)?;
        let journal_mode: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(|error| map_connection_error("read fresh SQLite journal mode", error))?;
        validate_preflight_journal_mode(&journal_mode)
    })
}

fn create_fresh_v5_database_with<F>(
    path: &Path,
    busy_timeout: Duration,
    initialize: F,
) -> RepositoryResult<()>
where
    F: FnOnce(&mut Connection) -> RepositoryResult<()>,
{
    if path.as_os_str().is_empty() {
        return Err(RepositoryError::InvalidInput {
            field: "database_path",
            message: "database path must not be empty".to_string(),
        });
    }
    if busy_timeout.is_zero() {
        return Err(RepositoryError::InvalidInput {
            field: "busy_timeout",
            message: "SQLite busy timeout must be greater than zero".to_string(),
        });
    }
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(RepositoryError::Unavailable {
                message: format!(
                    "create new latest-schema SQLite {} without clobber: target already exists",
                    path.display()
                ),
            });
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RepositoryError::Unavailable {
                message: format!(
                    "inspect latest-schema SQLite target {} before create: {error}",
                    path.display()
                ),
            });
        }
    }

    let mut temporary = FreshTempFile::create(path)?;
    let mut connection = open_unvalidated_writable_connection(&temporary.path, busy_timeout)?;
    initialize(&mut connection)?;
    connection
        .close()
        .map_err(|(_, error)| map_connection_error("close fresh latest-schema SQLite", error))?;
    temporary.verify_identity()?;
    fs::hard_link(&temporary.path, path).map_err(|error| RepositoryError::Unavailable {
        message: format!(
            "publish initialized latest-schema SQLite {} without clobber: {error}",
            path.display()
        ),
    })?;
    temporary.cleanup();
    Ok(())
}

fn open_unvalidated_writable_connection(
    path: &Path,
    busy_timeout: Duration,
) -> RepositoryResult<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE,
    )
    .map_err(|error| RepositoryError::Unavailable {
        message: format!(
            "open existing schema-v5 SQLite {} read-write without create: {error}",
            path.display()
        ),
    })?;
    connection
        .busy_timeout(busy_timeout)
        .map_err(|error| RepositoryError::Unavailable {
            message: format!("configure SQLite busy timeout: {error}"),
        })?;
    let readonly = connection
        .is_readonly(DatabaseName::Main)
        .map_err(|error| RepositoryError::Unavailable {
            message: format!("verify SQLite read-write access: {error}"),
        })?;
    if readonly {
        return Err(RepositoryError::Unavailable {
            message: "schema-v5 SQLite unexpectedly opened read-only".to_string(),
        });
    }
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| map_connection_error("enable SQLite foreign keys", error))?;
    let foreign_keys: i64 = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|error| map_connection_error("read SQLite foreign_keys pragma", error))?;
    if foreign_keys != 1 {
        return Err(RepositoryError::Unavailable {
            message: format!("SQLite foreign_keys pragma returned {foreign_keys}, expected 1"),
        });
    }
    Ok(connection)
}

fn open_v5_preflight_connection(
    path: &Path,
    busy_timeout: Duration,
) -> RepositoryResult<Connection> {
    validate_preflight_file_namespace(path)?;
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NOFOLLOW
            | OpenFlags::SQLITE_OPEN_PRIVATE_CACHE,
    )
    .map_err(|error| RepositoryError::Unavailable {
        message: format!(
            "open existing schema-v5 SQLite {} read-only for preflight: {error}",
            path.display()
        ),
    })?;
    connection
        .busy_timeout(busy_timeout)
        .map_err(|error| RepositoryError::Unavailable {
            message: format!("configure SQLite preflight busy timeout: {error}"),
        })?;
    let readonly = connection
        .is_readonly(DatabaseName::Main)
        .map_err(|error| RepositoryError::Unavailable {
            message: format!("verify SQLite preflight read-only access: {error}"),
        })?;
    if !readonly {
        return Err(RepositoryError::Unavailable {
            message: "schema-v5 preflight unexpectedly opened a writable connection".to_string(),
        });
    }
    connection
        .pragma_update(None, "query_only", "ON")
        .map_err(|error| map_connection_error("enable SQLite preflight query_only", error))?;
    let query_only: i64 = connection
        .pragma_query_value(None, "query_only", |row| row.get(0))
        .map_err(|error| map_connection_error("read SQLite preflight query_only", error))?;
    if query_only != 1 {
        return Err(RepositoryError::Unavailable {
            message: format!(
                "SQLite preflight query_only pragma returned {query_only}, expected 1"
            ),
        });
    }
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| map_connection_error("enable SQLite preflight foreign keys", error))?;
    let journal_mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|error| map_connection_error("read SQLite preflight journal mode", error))?;
    validate_preflight_journal_mode(&journal_mode)?;
    validate_schema_marker(&connection)?;
    // A second check catches a sidecar that appeared during open. On a clean rollback-journal
    // database the read-only connection itself must not create any of these entries.
    validate_preflight_file_namespace(path)?;
    Ok(connection)
}

fn validate_preflight_journal_mode(journal_mode: &str) -> RepositoryResult<()> {
    if journal_mode.eq_ignore_ascii_case("delete") {
        return Ok(());
    }
    Err(RepositoryError::Unavailable {
        message: format!(
            "schema-v5 preflight requires canonical DELETE journal mode, found {journal_mode}"
        ),
    })
}

fn validate_preflight_file_namespace(path: &Path) -> RepositoryResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| RepositoryError::Unavailable {
        message: format!(
            "inspect existing schema-v5 SQLite {} before read-only preflight: {error}",
            path.display()
        ),
    })?;
    if !metadata.file_type().is_file() {
        return Err(RepositoryError::Unavailable {
            message: format!(
                "schema-v5 SQLite {} must be a regular non-symlink file for preflight",
                path.display()
            ),
        });
    }

    for suffix in SQLITE_SIDECAR_SUFFIXES {
        let sidecar = sqlite_sidecar_path(path, suffix);
        match fs::symlink_metadata(&sidecar) {
            Ok(_) => {
                return Err(RepositoryError::Unavailable {
                    message: format!(
                        "schema-v5 preflight rejects existing SQLite sidecar {}",
                        sidecar.display()
                    ),
                });
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(RepositoryError::Unavailable {
                    message: format!(
                        "inspect SQLite sidecar {} before preflight: {error}",
                        sidecar.display()
                    ),
                });
            }
        }
    }

    let mut header = [0_u8; SQLITE_HEADER_LEN];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| RepositoryError::CorruptData {
            message: format!(
                "read complete SQLite header from {} before preflight: {error}",
                path.display()
            ),
        })?;
    if &header[..SQLITE_MAGIC.len()] != SQLITE_MAGIC {
        return Err(RepositoryError::CorruptData {
            message: format!(
                "{} does not contain a SQLite format-3 header",
                path.display()
            ),
        });
    }
    let write_version = header[SQLITE_WRITE_VERSION_OFFSET];
    let read_version = header[SQLITE_READ_VERSION_OFFSET];
    if write_version == SQLITE_WAL_VERSION || read_version == SQLITE_WAL_VERSION {
        return Err(RepositoryError::Unavailable {
            message: format!(
                "schema-v5 preflight rejects WAL-format SQLite header in {} (write={write_version}, read={read_version})",
                path.display()
            ),
        });
    }
    if write_version != SQLITE_ROLLBACK_JOURNAL_VERSION
        || read_version != SQLITE_ROLLBACK_JOURNAL_VERSION
    {
        return Err(RepositoryError::CorruptData {
            message: format!(
                "SQLite header in {} has unsupported write/read versions {write_version}/{read_version}",
                path.display()
            ),
        });
    }
    Ok(())
}

fn sqlite_sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar = path.as_os_str().to_os_string();
    sidecar.push(suffix);
    PathBuf::from(sidecar)
}

fn validate_schema_marker(connection: &Connection) -> RepositoryResult<()> {
    let mut statement = connection
        .prepare(
            "SELECT value
               FROM subscription_schema_meta
              WHERE key = 'schema_version'",
        )
        .map_err(|error| map_read_error("prepare schema marker query", error))?;
    let rows = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|error| map_read_error("query schema marker", error))?;
    let mut markers = Vec::new();
    for row in rows {
        markers.push(row.map_err(|error| map_read_error("decode schema marker", error))?);
    }
    if markers.is_empty() {
        return Err(RepositoryError::UnsupportedSchema {
            found: 0,
            maximum_supported: SCHEMA_VERSION,
        });
    }
    if markers.len() != 1 {
        return Err(RepositoryError::CorruptData {
            message: format!(
                "schema-v5 database must contain exactly one schema_version marker, found {}",
                markers.len()
            ),
        });
    }
    let found = u32::try_from(markers[0]).map_err(|_| RepositoryError::CorruptData {
        message: format!(
            "schema_version marker {} cannot be represented as a non-negative version",
            markers[0]
        ),
    })?;
    if found != SCHEMA_VERSION {
        return Err(RepositoryError::UnsupportedSchema {
            found,
            maximum_supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

fn validate_runtime_schema(connection: &Connection) -> RepositoryResult<()> {
    validate_foreign_keys_enabled(connection)?;
    validate_integrity_check(connection)?;
    validate_foreign_key_check(connection)?;
    validate_schema_contract(connection).map_err(|error| RepositoryError::CorruptData {
        message: format!("schema-v5 structural contract failed: {error}"),
    })
}

fn validate_foreign_keys_enabled(connection: &Connection) -> RepositoryResult<()> {
    let foreign_keys: i64 = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|error| map_connection_error("read SQLite foreign_keys pragma", error))?;
    if foreign_keys != 1 {
        return Err(RepositoryError::Unavailable {
            message: format!("SQLite foreign_keys pragma returned {foreign_keys}, expected 1"),
        });
    }
    Ok(())
}

fn validate_integrity_check(connection: &Connection) -> RepositoryResult<()> {
    let mut statement = connection
        .prepare("PRAGMA integrity_check(1)")
        .map_err(|error| map_read_error("prepare SQLite integrity_check", error))?;
    let results = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| map_read_error("execute SQLite integrity_check", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| map_read_error("read SQLite integrity_check", error))?;
    if results.as_slice() != ["ok"] {
        return Err(RepositoryError::CorruptData {
            message: format!("SQLite integrity_check returned {results:?}"),
        });
    }
    Ok(())
}

fn validate_foreign_key_check(connection: &Connection) -> RepositoryResult<()> {
    let mut statement = connection
        .prepare("PRAGMA foreign_key_check")
        .map_err(|error| map_read_error("prepare SQLite foreign_key_check", error))?;
    let mut rows = statement
        .query([])
        .map_err(|error| map_read_error("execute SQLite foreign_key_check", error))?;
    if let Some(row) = rows
        .next()
        .map_err(|error| map_read_error("read SQLite foreign_key_check", error))?
    {
        let table = row
            .get::<_, String>(0)
            .map_err(|error| map_read_error("decode SQLite foreign_key_check table", error))?;
        return Err(RepositoryError::CorruptData {
            message: format!("SQLite foreign_key_check reported a violation in table {table}"),
        });
    }
    Ok(())
}

fn map_connection_error(context: &str, error: rusqlite::Error) -> RepositoryError {
    RepositoryError::Unavailable {
        message: format!("{context}: {error}"),
    }
}

pub(super) fn map_read_error(context: &str, error: rusqlite::Error) -> RepositoryError {
    let message = format!("{context}: {error}");
    match classify_read_error(&error) {
        ReadErrorKind::Unavailable => RepositoryError::Unavailable { message },
        ReadErrorKind::CorruptData => RepositoryError::CorruptData { message },
        ReadErrorKind::Internal => RepositoryError::Internal { message },
    }
}

pub(super) fn map_write_error(context: &str, error: rusqlite::Error) -> RepositoryError {
    let message = format!("{context}: {error}");
    match &error {
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                ErrorCode::ConstraintViolation | ErrorCode::TypeMismatch | ErrorCode::TooBig
            ) =>
        {
            RepositoryError::CorruptData { message }
        }
        _ => match classify_read_error(&error) {
            ReadErrorKind::Unavailable => RepositoryError::Unavailable { message },
            ReadErrorKind::CorruptData => RepositoryError::CorruptData { message },
            ReadErrorKind::Internal => RepositoryError::Internal { message },
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadErrorKind {
    Unavailable,
    CorruptData,
    Internal,
}

fn classify_read_error(error: &rusqlite::Error) -> ReadErrorKind {
    match error {
        rusqlite::Error::SqliteFailure(inner, _) => classify_sqlite_code(inner.code),
        rusqlite::Error::SqliteSingleThreadedMode | rusqlite::Error::InvalidPath(_) => {
            ReadErrorKind::Unavailable
        }
        rusqlite::Error::FromSqlConversionFailure(..)
        | rusqlite::Error::IntegralValueOutOfRange(..)
        | rusqlite::Error::Utf8Error(_)
        | rusqlite::Error::InvalidColumnType(..) => ReadErrorKind::CorruptData,
        _ => ReadErrorKind::Internal,
    }
}

fn classify_sqlite_code(code: ErrorCode) -> ReadErrorKind {
    match code {
        ErrorCode::PermissionDenied
        | ErrorCode::OperationAborted
        | ErrorCode::DatabaseBusy
        | ErrorCode::DatabaseLocked
        | ErrorCode::OutOfMemory
        | ErrorCode::ReadOnly
        | ErrorCode::OperationInterrupted
        | ErrorCode::SystemIoFailure
        | ErrorCode::DiskFull
        | ErrorCode::CannotOpen
        | ErrorCode::FileLockingProtocolFailed
        | ErrorCode::SchemaChanged
        | ErrorCode::NoLargeFileSupport
        | ErrorCode::AuthorizationForStatementDenied => ReadErrorKind::Unavailable,
        ErrorCode::DatabaseCorrupt
        | ErrorCode::NotADatabase
        | ErrorCode::TooBig
        | ErrorCode::TypeMismatch
        | ErrorCode::Unknown => ReadErrorKind::CorruptData,
        ErrorCode::InternalMalfunction
        | ErrorCode::NotFound
        | ErrorCode::ConstraintViolation
        | ErrorCode::ApiMisuse
        | ErrorCode::ParameterOutOfRange => ReadErrorKind::Internal,
        _ => ReadErrorKind::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use rusqlite::ffi;

    use super::{
        create_fresh_v5_database, create_fresh_v5_database_with, map_read_error, map_write_error,
        open_v5_connection, validate_preflight_journal_mode,
    };
    use crate::subscription::repository::RepositoryError;

    static NEXT_FRESH_FAILURE_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn read_error_mapping_distinguishes_availability_corruption_and_programming_failures() {
        for code in [
            ffi::SQLITE_BUSY,
            ffi::SQLITE_IOERR,
            ffi::SQLITE_FULL,
            ffi::SQLITE_CANTOPEN,
            ffi::SQLITE_READONLY,
            ffi::SQLITE_NOMEM,
            ffi::SQLITE_INTERRUPT,
        ] {
            let error = map_read_error("read fixture", sqlite_failure(code));
            assert!(matches!(error, RepositoryError::Unavailable { .. }));
        }
        for code in [
            ffi::SQLITE_CORRUPT,
            ffi::SQLITE_NOTADB,
            ffi::SQLITE_MISMATCH,
        ] {
            let error = map_read_error("read fixture", sqlite_failure(code));
            assert!(matches!(error, RepositoryError::CorruptData { .. }));
        }
        let programming =
            map_read_error("read fixture", rusqlite::Error::InvalidParameterCount(1, 2));
        assert!(matches!(programming, RepositoryError::Internal { .. }));
    }

    #[test]
    fn write_constraint_failures_are_treated_as_persisted_invariant_violations() {
        let error = map_write_error(
            "write fixture",
            sqlite_failure(ffi::SQLITE_CONSTRAINT_CHECK),
        );
        assert!(matches!(error, RepositoryError::CorruptData { .. }));
    }

    #[test]
    fn preflight_accepts_only_canonical_delete_journal_mode() {
        validate_preflight_journal_mode("delete").expect("DELETE is the canonical runtime mode");
        validate_preflight_journal_mode("DELETE")
            .expect("journal mode comparison is case-insensitive");
        for mode in ["truncate", "persist", "memory", "off", "wal"] {
            let error = validate_preflight_journal_mode(mode)
                .expect_err("non-DELETE journal mode must fail closed");
            let RepositoryError::Unavailable { message } = error else {
                panic!("noncanonical journal mode must be unavailable: {error}");
            };
            assert!(message.contains(mode));
        }
    }

    #[test]
    fn failed_fresh_initialization_never_publishes_or_blocks_the_target() {
        let sequence = NEXT_FRESH_FAILURE_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "tmdb-mteam-fresh-init-failure-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fresh initialization failure fixture");
        let target = root.join("subscriptions.sqlite");
        let failure =
            create_fresh_v5_database_with(&target, Duration::from_secs(1), |connection| {
                connection
                    .execute_batch("CREATE TABLE partial_schema (id INTEGER PRIMARY KEY);")
                    .map_err(|error| map_write_error("seed partial fresh schema", error))?;
                Err(RepositoryError::Internal {
                    message: "injected latest-schema initialization failure".to_string(),
                })
            })
            .expect_err("injected fresh initialization must fail");
        assert!(matches!(failure, RepositoryError::Internal { .. }));
        assert!(
            !target.exists(),
            "a failed initializer must never publish subscriptions.sqlite"
        );
        assert_eq!(
            fs::read_dir(&root).unwrap().count(),
            0,
            "the owned temporary inode and SQLite sidecars must be cleaned"
        );

        create_fresh_v5_database(&target, Duration::from_secs(1))
            .expect("a clean retry must create the latest database");
        open_v5_connection(&target, Duration::from_secs(1))
            .expect("the retried latest database must be immediately usable");
        fs::remove_dir_all(root).expect("remove fresh initialization failure fixture");
    }

    fn sqlite_failure(code: i32) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(ffi::Error::new(code), None)
    }
}
