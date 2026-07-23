use std::io;
use std::path::Path;

#[cfg(unix)]
use std::fs::{self, File, OpenOptions};

#[cfg(unix)]
const SERVICE_LOCK_FILE_NAME: &str = "subscription-storage.service.lock";

#[derive(Debug)]
pub(crate) struct StorageServiceLock {
    #[cfg(unix)]
    _file: File,
}

#[cfg(unix)]
pub(crate) fn acquire_storage_service_lock(config_path: &Path) -> io::Result<StorageServiceLock> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

    let raw_parent = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(raw_parent).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "create configuration directory {} for subscription storage lock: {error}",
                raw_parent.display()
            ),
        )
    })?;
    let canonical_parent = fs::canonicalize(raw_parent).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "canonicalize configuration directory {} for subscription storage lock: {error}",
                raw_parent.display()
            ),
        )
    })?;
    let lock_path = canonical_parent.join(SERVICE_LOCK_FILE_NAME);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&lock_path)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "open subscription storage lock {} without following symlinks: {error}",
                    lock_path.display()
                ),
            )
        })?;

    let metadata = file.metadata().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("inspect subscription storage lock: {error}"),
        )
    })?;
    if !metadata.is_file() || metadata.nlink() != 1 || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "subscription storage lock must be a private regular file with one hard link",
        ));
    }

    // SAFETY: `file` owns a valid descriptor for the duration of the call. `flock` retains no Rust
    // pointer, and the operating system releases the lock when `StorageServiceLock` drops the file.
    let lock_result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if lock_result != 0 {
        let error = io::Error::last_os_error();
        let raw_error = error.raw_os_error();
        let kind = if raw_error == Some(libc::EWOULDBLOCK) || raw_error == Some(libc::EAGAIN) {
            io::ErrorKind::WouldBlock
        } else {
            error.kind()
        };
        return Err(io::Error::new(
            kind,
            "subscription storage lock is already held or unavailable; stop the other service or maintenance process",
        ));
    }

    let path_metadata = fs::symlink_metadata(&lock_path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("verify subscription storage lock path: {error}"),
        )
    })?;
    if !path_metadata.is_file()
        || path_metadata.nlink() != 1
        || path_metadata.dev() != metadata.dev()
        || path_metadata.ino() != metadata.ino()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "subscription storage lock path identity changed during acquisition",
        ));
    }

    Ok(StorageServiceLock { _file: file })
}

#[cfg(not(unix))]
pub(crate) fn acquire_storage_service_lock(_config_path: &Path) -> io::Result<StorageServiceLock> {
    Ok(StorageServiceLock {})
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn test_root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "tmdb-mteam-storage-lock-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn lock_is_private_and_rejects_a_second_live_owner() {
        let root = test_root();
        let config_path = root.join("config").join("config.toml");
        let first = acquire_storage_service_lock(&config_path).expect("acquire first storage lock");
        let lock_path = root
            .join("config")
            .join("subscription-storage.service.lock");
        let mode = fs::metadata(&lock_path)
            .expect("inspect storage lock")
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);

        let second = acquire_storage_service_lock(&config_path)
            .expect_err("a second live storage owner must fail");
        assert_eq!(second.kind(), ErrorKind::WouldBlock);

        drop(first);
        acquire_storage_service_lock(&config_path).expect("lock becomes available after drop");
        fs::remove_dir_all(root).expect("remove storage lock fixture");
    }
}
