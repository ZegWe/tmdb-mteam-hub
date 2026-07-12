//! Runtime adapters for retry-safe subscription effects.
//!
//! The domain rules remain in [`super::effects`]. This module is the boundary
//! that is allowed to depend on qBittorrent HTTP and the host filesystem.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::clients::http::ClientError;
use crate::clients::qbittorrent::{self, QbTorrentInfo};
use crate::config::QbServerEntry;
use crate::storage::blocking::{
    BlockingExecutorConfigError, BlockingTaskError, BoundedBlockingExecutor,
};

use super::effects::{
    plan_link_retry, reconcile_qb_torrent, EffectIdentityError, EnsureQbTorrentOutcome,
    FileIdentity, LinkFileAction, LinkFileEffect, LinkFileFailure, LinkFileOutcome, LinkFileProbe,
    LinkPlanError, QbReconcileRequest, QbReconciliationConflict, QbReconciliationDecision,
    QbTorrentObservation,
};

pub(crate) type QbTransportFuture<'a, T, E> =
    Pin<Box<dyn Future<Output = Result<T, E>> + Send + 'a>>;

/// Transport seam used by the async qB effect orchestration. The production
/// implementation below delegates to the policy-enforced qB client; tests use
/// an in-memory transport and never touch the network.
pub(crate) trait QbEffectTransport: Send {
    type Error: Send;

    fn list_by_exact_tag<'a>(
        &'a mut self,
        stable_tag: &'a str,
    ) -> QbTransportFuture<'a, Vec<QbTorrentInfo>, Self::Error>;

    fn list_by_hash<'a>(
        &'a mut self,
        authoritative_hash: &'a str,
    ) -> QbTransportFuture<'a, Vec<QbTorrentInfo>, Self::Error>;

    fn add<'a>(
        &'a mut self,
        command: &'a QbAddCommand<'a>,
    ) -> QbTransportFuture<'a, (), Self::Error>;
}

/// Real qB Web API transport scoped to one configured server/account.
///
/// Deliberately does not implement `Debug`: the server contains credentials.
pub(crate) struct QbHttpEffectTransport {
    server: QbServerEntry,
}

impl QbHttpEffectTransport {
    pub(crate) fn new(server: QbServerEntry) -> Self {
        Self { server }
    }
}

impl QbEffectTransport for QbHttpEffectTransport {
    type Error = ClientError;

    fn list_by_exact_tag<'a>(
        &'a mut self,
        stable_tag: &'a str,
    ) -> QbTransportFuture<'a, Vec<QbTorrentInfo>, Self::Error> {
        Box::pin(
            async move { qbittorrent::list_torrents_by_exact_tag(&self.server, stable_tag).await },
        )
    }

    fn list_by_hash<'a>(
        &'a mut self,
        authoritative_hash: &'a str,
    ) -> QbTransportFuture<'a, Vec<QbTorrentInfo>, Self::Error> {
        Box::pin(async move {
            let hashes = [authoritative_hash.to_string()];
            qbittorrent::list_torrents_by_hashes(&self.server, &hashes).await
        })
    }

    fn add<'a>(
        &'a mut self,
        command: &'a QbAddCommand<'a>,
    ) -> QbTransportFuture<'a, (), Self::Error> {
        Box::pin(async move {
            let tags = [command.stable_tag.to_string()];
            match command.input {
                QbTorrentInput::Url(url) => {
                    qbittorrent::add_torrent_from_url_with_tags(
                        &self.server,
                        url,
                        command.category,
                        command.save_path,
                        &tags,
                    )
                    .await
                }
                QbTorrentInput::Bytes { filename, bytes } => {
                    qbittorrent::add_torrent_bytes_with_tags(
                        &self.server,
                        filename,
                        bytes.clone(),
                        command.category,
                        command.save_path,
                        &tags,
                    )
                    .await
                }
            }
        })
    }
}

/// Torrent material accepted by the qB add endpoint.
///
/// This type intentionally has no `Debug` implementation so a URL query or
/// uploaded metainfo cannot accidentally enter logs through the adapter.
pub(crate) enum QbTorrentInput {
    Url(String),
    Bytes { filename: String, bytes: Vec<u8> },
}

impl QbTorrentInput {
    pub(crate) fn from_url(url: impl Into<String>) -> Result<Self, QbAddInputError> {
        let url = url.into();
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(QbAddInputError::EmptyUrl);
        }
        Ok(Self::Url(trimmed.to_string()))
    }

    pub(crate) fn from_bytes(
        filename: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<Self, QbAddInputError> {
        if bytes.is_empty() {
            return Err(QbAddInputError::EmptyTorrentFile);
        }
        let filename = filename.into();
        let filename = if filename.trim().is_empty() {
            "download.torrent".to_string()
        } else {
            filename.trim().to_string()
        };
        Ok(Self::Bytes { filename, bytes })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QbAddInputError {
    EmptyUrl,
    EmptyTorrentFile,
}

impl fmt::Display for QbAddInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyUrl => formatter.write_str("qB torrent URL must not be empty"),
            Self::EmptyTorrentFile => formatter.write_str("qB torrent file must not be empty"),
        }
    }
}

impl Error for QbAddInputError {}

/// Minimal non-identity qB add configuration. Tags are intentionally absent:
/// the adapter always supplies the stable effect key as the only add tag.
pub(crate) struct QbEffectAddSpec {
    input: QbTorrentInput,
    category: Option<String>,
    save_path: Option<String>,
}

impl QbEffectAddSpec {
    pub(crate) fn new(
        input: QbTorrentInput,
        category: Option<String>,
        save_path: Option<String>,
    ) -> Self {
        Self {
            input,
            category: normalized_optional(category),
            save_path: normalized_optional(save_path),
        }
    }
}

fn normalized_optional(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) struct QbAddCommand<'a> {
    input: &'a QbTorrentInput,
    category: Option<&'a str>,
    save_path: Option<&'a str>,
    stable_tag: &'a str,
}

impl QbAddCommand<'_> {
    pub(crate) fn input(&self) -> &QbTorrentInput {
        self.input
    }

    pub(crate) const fn category(&self) -> Option<&str> {
        self.category
    }

    pub(crate) const fn save_path(&self) -> Option<&str> {
        self.save_path
    }

    pub(crate) const fn stable_tag(&self) -> &str {
        self.stable_tag
    }
}

pub(crate) struct QbEffectAdapter<T> {
    transport: T,
}

impl QbEffectAdapter<QbHttpEffectTransport> {
    pub(crate) fn for_server(server: QbServerEntry) -> Self {
        Self::new(QbHttpEffectTransport::new(server))
    }
}

impl<T> QbEffectAdapter<T>
where
    T: QbEffectTransport,
{
    pub(crate) fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Observes by exact stable tag and, when available, authoritative hash;
    /// merges both result sets before the domain decides whether add is safe.
    pub(crate) async fn ensure(
        &mut self,
        request: &QbReconcileRequest,
        add: &QbEffectAddSpec,
    ) -> Result<EnsureQbTorrentOutcome, QbEffectAdapterError<T::Error>> {
        let tagged = self
            .transport
            .list_by_exact_tag(request.idempotency_key().as_str())
            .await
            .map_err(|source| QbEffectAdapterError::Transport {
                stage: QbEffectStage::InspectTag,
                source,
            })?;
        let hashed = if let Some(hash) = request.authoritative_hash() {
            self.transport.list_by_hash(hash).await.map_err(|source| {
                QbEffectAdapterError::Transport {
                    stage: QbEffectStage::InspectHash,
                    source,
                }
            })?
        } else {
            Vec::new()
        };
        let observations = merge_qb_observations(tagged, hashed)
            .map_err(QbEffectAdapterError::InvalidObservation)?;

        match reconcile_qb_torrent(request, &observations) {
            QbReconciliationDecision::UseExisting {
                torrent,
                matched_by,
            } => Ok(EnsureQbTorrentOutcome::Reconciled {
                torrent,
                matched_by,
            }),
            QbReconciliationDecision::Conflict(conflict) => {
                Err(QbEffectAdapterError::Conflict(conflict))
            }
            QbReconciliationDecision::Add => {
                let command = QbAddCommand {
                    input: &add.input,
                    category: add.category.as_deref(),
                    save_path: add.save_path.as_deref(),
                    stable_tag: request.idempotency_key().as_str(),
                };
                self.transport.add(&command).await.map_err(|source| {
                    QbEffectAdapterError::Transport {
                        stage: QbEffectStage::Add,
                        source,
                    }
                })?;
                Ok(EnsureQbTorrentOutcome::Added {
                    idempotency_key: request.idempotency_key().clone(),
                })
            }
        }
    }

    #[cfg(test)]
    fn transport(&self) -> &T {
        &self.transport
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QbEffectStage {
    InspectTag,
    InspectHash,
    Add,
}

impl fmt::Display for QbEffectStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InspectTag => formatter.write_str("inspect exact tag"),
            Self::InspectHash => formatter.write_str("inspect authoritative hash"),
            Self::Add => formatter.write_str("add torrent"),
        }
    }
}

#[derive(Debug)]
pub(crate) enum QbEffectAdapterError<E> {
    Transport { stage: QbEffectStage, source: E },
    InvalidObservation(EffectIdentityError),
    Conflict(QbReconciliationConflict),
}

impl<E: fmt::Display> fmt::Display for QbEffectAdapterError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport { stage, source } => {
                write!(formatter, "qB effect failed during {stage}: {source}")
            }
            Self::InvalidObservation(source) => {
                write!(
                    formatter,
                    "qB returned an invalid torrent observation: {source}"
                )
            }
            Self::Conflict(conflict) => {
                write!(formatter, "qB reconciliation conflict: {conflict:?}")
            }
        }
    }
}

impl<E: Error + 'static> Error for QbEffectAdapterError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transport { source, .. } => Some(source),
            Self::InvalidObservation(source) => Some(source),
            Self::Conflict(_) => None,
        }
    }
}

#[derive(Default)]
struct MergedQbTorrent {
    name: String,
    tags: BTreeSet<String>,
}

fn merge_qb_observations(
    tagged: Vec<QbTorrentInfo>,
    hashed: Vec<QbTorrentInfo>,
) -> Result<Vec<QbTorrentObservation>, EffectIdentityError> {
    let mut merged = BTreeMap::<String, MergedQbTorrent>::new();
    for info in tagged.into_iter().chain(hashed) {
        let tags = parse_qb_tags(&info.tags);
        let observation = QbTorrentObservation::try_new(&info.hash, &info.name, tags.clone())?;
        let entry = merged.entry(observation.hash().to_string()).or_default();
        if entry.name.is_empty() && !observation.name().is_empty() {
            entry.name = observation.name().to_string();
        }
        entry.tags.extend(tags);
    }
    merged
        .into_iter()
        .map(|(hash, torrent)| QbTorrentObservation::try_new(hash, torrent.name, torrent.tags))
        .collect()
}

fn parse_qb_tags(tags: &str) -> BTreeSet<String> {
    tags.split(',')
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

/// Filesystem adapter with a concurrency gate independent from SQLite work.
#[derive(Clone, Debug)]
pub(crate) struct HardlinkEffectAdapter {
    blocking: BoundedBlockingExecutor,
}

impl HardlinkEffectAdapter {
    pub(crate) fn try_new(max_concurrency: usize) -> Result<Self, BlockingExecutorConfigError> {
        Ok(Self {
            blocking: BoundedBlockingExecutor::try_new("filesystem", max_concurrency)?,
        })
    }

    pub(crate) async fn apply(
        &self,
        files: Vec<LinkFileEffect>,
    ) -> Result<HardlinkBatchOutcome, HardlinkEffectAdapterError> {
        self.blocking
            .run(move || apply_hardlinks(files))
            .await
            .map_err(HardlinkEffectAdapterError::Blocking)?
            .map_err(HardlinkEffectAdapterError::Plan)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HardlinkBatchOutcome {
    files: Vec<HardlinkFileResult>,
}

impl HardlinkBatchOutcome {
    pub(crate) fn files(&self) -> &[HardlinkFileResult] {
        &self.files
    }

    pub(crate) fn into_files(self) -> Vec<HardlinkFileResult> {
        self.files
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HardlinkFileResult {
    source_path: PathBuf,
    target_path: PathBuf,
    outcome: LinkFileOutcome,
    status: HardlinkFileStatus,
}

impl HardlinkFileResult {
    pub(crate) fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub(crate) fn target_path(&self) -> &Path {
        &self.target_path
    }

    pub(crate) const fn outcome(&self) -> LinkFileOutcome {
        self.outcome
    }

    pub(crate) const fn status(&self) -> &HardlinkFileStatus {
        &self.status
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HardlinkFileStatus {
    Created,
    SkippedVerified,
    AcceptedExisting,
    Missing,
    Conflict {
        source: FileIdentity,
        target: FileIdentity,
    },
    Failed(HardlinkFilesystemFailure),
}

impl HardlinkFileStatus {
    const fn outcome(&self) -> LinkFileOutcome {
        match self {
            Self::Created | Self::SkippedVerified | Self::AcceptedExisting => {
                LinkFileOutcome::Linked
            }
            Self::Missing => LinkFileOutcome::Missing,
            Self::Conflict { .. } => LinkFileOutcome::Conflict,
            Self::Failed(_) => LinkFileOutcome::Failed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HardlinkFilesystemOperation {
    ProbeSource,
    ProbeTarget,
    CreateParent,
    CreateLink,
    VerifyCreatedLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HardlinkFilesystemFailureKind {
    Io(io::ErrorKind),
    Symlink,
    NonRegularFile,
    UnsafePath,
    SourceChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HardlinkFilesystemFailure {
    operation: HardlinkFilesystemOperation,
    kind: HardlinkFilesystemFailureKind,
}

impl HardlinkFilesystemFailure {
    pub(crate) const fn operation(&self) -> HardlinkFilesystemOperation {
        self.operation
    }

    pub(crate) const fn kind(&self) -> HardlinkFilesystemFailureKind {
        self.kind
    }

    const fn io(operation: HardlinkFilesystemOperation, kind: io::ErrorKind) -> Self {
        Self {
            operation,
            kind: HardlinkFilesystemFailureKind::Io(kind),
        }
    }

    const fn unsafe_path(operation: HardlinkFilesystemOperation) -> Self {
        Self {
            operation,
            kind: HardlinkFilesystemFailureKind::UnsafePath,
        }
    }
}

#[derive(Debug)]
pub(crate) enum HardlinkEffectAdapterError {
    Blocking(BlockingTaskError),
    Plan(LinkPlanError),
}

impl fmt::Display for HardlinkEffectAdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blocking(source) => write!(formatter, "hardlink blocking task failed: {source}"),
            Self::Plan(source) => write!(formatter, "hardlink retry plan is invalid: {source}"),
        }
    }
}

impl Error for HardlinkEffectAdapterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Blocking(source) => Some(source),
            Self::Plan(source) => Some(source),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PathProbe {
    identity: Option<FileIdentity>,
    failure: Option<HardlinkFilesystemFailure>,
}

#[derive(Debug, Clone, Copy)]
struct LinkProbe {
    source: PathProbe,
    target: PathProbe,
}

fn apply_hardlinks(files: Vec<LinkFileEffect>) -> Result<HardlinkBatchOutcome, LinkPlanError> {
    let observed = files
        .iter()
        .map(|file| LinkProbe {
            source: probe_regular_path(
                file.source_path(),
                HardlinkFilesystemOperation::ProbeSource,
            ),
            target: probe_regular_path(
                file.target_path(),
                HardlinkFilesystemOperation::ProbeTarget,
            ),
        })
        .collect::<Vec<_>>();
    let domain_probes = observed
        .iter()
        .map(|probe| LinkFileProbe {
            source: probe.source.identity,
            target: probe.target.identity,
        })
        .collect::<Vec<_>>();
    let planned = plan_link_retry(&files, &domain_probes)?;

    let files = planned
        .into_iter()
        .zip(observed)
        .map(|(planned, observed)| {
            let status = if let Some(failure) = observed.source.failure {
                HardlinkFileStatus::Failed(failure)
            } else if let Some(failure) = observed.target.failure {
                HardlinkFileStatus::Failed(failure)
            } else {
                execute_link_action(&planned, observed)
            };
            HardlinkFileResult {
                source_path: planned.source_path().to_path_buf(),
                target_path: planned.target_path().to_path_buf(),
                outcome: status.outcome(),
                status,
            }
        })
        .collect();
    Ok(HardlinkBatchOutcome { files })
}

fn execute_link_action(
    planned: &super::effects::PlannedLinkFile,
    observed: LinkProbe,
) -> HardlinkFileStatus {
    match planned.action() {
        LinkFileAction::SkipVerified => HardlinkFileStatus::SkippedVerified,
        LinkFileAction::AcceptExisting => HardlinkFileStatus::AcceptedExisting,
        LinkFileAction::Fail(LinkFileFailure::SourceMissing) => HardlinkFileStatus::Missing,
        LinkFileAction::Fail(LinkFileFailure::TargetConflict { source, target }) => {
            HardlinkFileStatus::Conflict { source, target }
        }
        LinkFileAction::Create => create_hardlink(
            planned.source_path(),
            planned.target_path(),
            observed
                .source
                .identity
                .expect("Create requires a successfully probed source"),
        ),
    }
}

fn probe_regular_path(path: &Path, operation: HardlinkFilesystemOperation) -> PathProbe {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            let file_type = metadata.file_type();
            if file_type.is_symlink() {
                PathProbe {
                    identity: None,
                    failure: Some(HardlinkFilesystemFailure {
                        operation,
                        kind: HardlinkFilesystemFailureKind::Symlink,
                    }),
                }
            } else if !file_type.is_file() {
                PathProbe {
                    identity: None,
                    failure: Some(HardlinkFilesystemFailure {
                        operation,
                        kind: HardlinkFilesystemFailureKind::NonRegularFile,
                    }),
                }
            } else {
                match metadata_identity(&metadata) {
                    Ok(identity) => PathProbe {
                        identity: Some(identity),
                        failure: None,
                    },
                    Err(kind) => PathProbe {
                        identity: None,
                        failure: Some(HardlinkFilesystemFailure::io(operation, kind)),
                    },
                }
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => PathProbe {
            identity: None,
            failure: None,
        },
        Err(error) => PathProbe {
            identity: None,
            failure: Some(HardlinkFilesystemFailure::io(operation, error.kind())),
        },
    }
}

#[cfg(unix)]
fn metadata_identity(metadata: &std::fs::Metadata) -> Result<FileIdentity, io::ErrorKind> {
    use std::os::unix::fs::MetadataExt;

    Ok(FileIdentity::new(metadata.dev(), metadata.ino()))
}

#[cfg(not(unix))]
fn metadata_identity(_metadata: &std::fs::Metadata) -> Result<FileIdentity, io::ErrorKind> {
    Err(io::ErrorKind::Unsupported)
}

#[cfg(target_os = "linux")]
fn create_hardlink(
    source: &Path,
    target: &Path,
    expected_source: FileIdentity,
) -> HardlinkFileStatus {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;

    fn c_string(value: &std::ffi::OsStr) -> Result<CString, HardlinkFilesystemFailure> {
        CString::new(value.as_bytes()).map_err(|_| {
            HardlinkFilesystemFailure::unsafe_path(HardlinkFilesystemOperation::CreateLink)
        })
    }

    fn open_dir_at(parent: &OwnedFd, component: &CString) -> Result<OwnedFd, io::Error> {
        // SAFETY: `parent` is a live directory descriptor and `component` is a
        // NUL-terminated string retained for the duration of the call.
        let descriptor = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            Err(io::Error::last_os_error())
        } else {
            // SAFETY: a successful `openat` returns a new owned descriptor.
            Ok(unsafe { OwnedFd::from_raw_fd(descriptor) })
        }
    }

    fn mkdir_at(parent: &OwnedFd, component: &CString) -> Result<(), io::Error> {
        // SAFETY: arguments are valid for the duration of the call.
        let result = unsafe {
            libc::mkdirat(
                parent.as_raw_fd(),
                component.as_ptr(),
                libc::S_IRWXU | libc::S_IRGRP | libc::S_IXGRP | libc::S_IROTH | libc::S_IXOTH,
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn safe_parent(target: &Path) -> Result<(OwnedFd, CString), HardlinkFilesystemFailure> {
        let filename = target
            .file_name()
            .ok_or_else(|| {
                HardlinkFilesystemFailure::unsafe_path(HardlinkFilesystemOperation::CreateParent)
            })
            .and_then(c_string)?;
        let start = if target.is_absolute() { "/" } else { "." };
        let mut directory: OwnedFd =
            std::fs::File::open(start)
                .map(Into::into)
                .map_err(|error| {
                    HardlinkFilesystemFailure::io(
                        HardlinkFilesystemOperation::CreateParent,
                        error.kind(),
                    )
                })?;
        if let Some(parent) = target.parent() {
            for component in parent.components() {
                let Component::Normal(component) = component else {
                    if matches!(component, Component::RootDir | Component::CurDir) {
                        continue;
                    }
                    return Err(HardlinkFilesystemFailure::unsafe_path(
                        HardlinkFilesystemOperation::CreateParent,
                    ));
                };
                let component = c_string(component).map_err(|_| {
                    HardlinkFilesystemFailure::unsafe_path(
                        HardlinkFilesystemOperation::CreateParent,
                    )
                })?;
                directory = match open_dir_at(&directory, &component) {
                    Ok(next) => next,
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        if let Err(error) = mkdir_at(&directory, &component) {
                            if error.kind() != io::ErrorKind::AlreadyExists {
                                return Err(HardlinkFilesystemFailure::io(
                                    HardlinkFilesystemOperation::CreateParent,
                                    error.kind(),
                                ));
                            }
                        }
                        open_dir_at(&directory, &component).map_err(|error| {
                            HardlinkFilesystemFailure::io(
                                HardlinkFilesystemOperation::CreateParent,
                                error.kind(),
                            )
                        })?
                    }
                    Err(error) => {
                        return Err(HardlinkFilesystemFailure::io(
                            HardlinkFilesystemOperation::CreateParent,
                            error.kind(),
                        ));
                    }
                };
            }
        }
        Ok((directory, filename))
    }

    enum TargetProbe {
        Missing,
        Regular(FileIdentity),
        Unsafe(HardlinkFilesystemFailureKind),
    }

    fn probe_target_at(
        directory: &OwnedFd,
        filename: &CString,
    ) -> Result<TargetProbe, HardlinkFilesystemFailure> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: the descriptor and filename are live and `stat` points to
        // sufficient writable memory. It is read only after a successful call.
        let result = unsafe {
            libc::fstatat(
                directory.as_raw_fd(),
                filename.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Ok(TargetProbe::Missing)
            } else {
                Err(HardlinkFilesystemFailure::io(
                    HardlinkFilesystemOperation::VerifyCreatedLink,
                    error.kind(),
                ))
            };
        }
        // SAFETY: `fstatat` initialized the value on success.
        let stat = unsafe { stat.assume_init() };
        let file_kind = stat.st_mode & libc::S_IFMT;
        if file_kind == libc::S_IFLNK {
            return Ok(TargetProbe::Unsafe(HardlinkFilesystemFailureKind::Symlink));
        }
        if file_kind != libc::S_IFREG {
            return Ok(TargetProbe::Unsafe(
                HardlinkFilesystemFailureKind::NonRegularFile,
            ));
        }
        Ok(TargetProbe::Regular(FileIdentity::new(
            stat.st_dev,
            stat.st_ino,
        )))
    }

    fn open_stable_source(
        source: &Path,
        expected: FileIdentity,
    ) -> Result<OwnedFd, HardlinkFileStatus> {
        let source = c_string(source.as_os_str()).map_err(HardlinkFileStatus::Failed)?;
        // SAFETY: `source` is NUL-terminated and live for the call. O_PATH plus
        // O_NOFOLLOW pins the final inode without following a raced symlink.
        let descriptor = unsafe {
            libc::open(
                source.as_ptr(),
                libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            let error = io::Error::last_os_error();
            return if error.kind() == io::ErrorKind::NotFound {
                Err(HardlinkFileStatus::Missing)
            } else {
                Err(HardlinkFileStatus::Failed(HardlinkFilesystemFailure::io(
                    HardlinkFilesystemOperation::ProbeSource,
                    error.kind(),
                )))
            };
        }
        // SAFETY: a successful `open` returns a new owned descriptor.
        let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: the descriptor is live and `stat` has sufficient writable
        // memory. It is read only after a successful call.
        if unsafe { libc::fstat(descriptor.as_raw_fd(), stat.as_mut_ptr()) } != 0 {
            return Err(HardlinkFileStatus::Failed(HardlinkFilesystemFailure::io(
                HardlinkFilesystemOperation::ProbeSource,
                io::Error::last_os_error().kind(),
            )));
        }
        // SAFETY: `fstat` initialized the value on success.
        let stat = unsafe { stat.assume_init() };
        let file_kind = stat.st_mode & libc::S_IFMT;
        if file_kind == libc::S_IFLNK {
            return Err(HardlinkFileStatus::Failed(HardlinkFilesystemFailure {
                operation: HardlinkFilesystemOperation::ProbeSource,
                kind: HardlinkFilesystemFailureKind::Symlink,
            }));
        }
        if file_kind != libc::S_IFREG {
            return Err(HardlinkFileStatus::Failed(HardlinkFilesystemFailure {
                operation: HardlinkFilesystemOperation::ProbeSource,
                kind: HardlinkFilesystemFailureKind::NonRegularFile,
            }));
        }
        let actual = FileIdentity::new(stat.st_dev, stat.st_ino);
        if actual != expected {
            return Err(HardlinkFileStatus::Failed(HardlinkFilesystemFailure {
                operation: HardlinkFilesystemOperation::ProbeSource,
                kind: HardlinkFilesystemFailureKind::SourceChanged,
            }));
        }
        Ok(descriptor)
    }

    let source_descriptor = match open_stable_source(source, expected_source) {
        Ok(descriptor) => descriptor,
        Err(status) => return status,
    };
    let (target_directory, target_filename) = match safe_parent(target) {
        Ok(value) => value,
        Err(failure) => return HardlinkFileStatus::Failed(failure),
    };
    let pinned_source =
        match CString::new(format!("/proc/self/fd/{}", source_descriptor.as_raw_fd())) {
            Ok(source) => source,
            Err(_) => {
                return HardlinkFileStatus::Failed(HardlinkFilesystemFailure::unsafe_path(
                    HardlinkFilesystemOperation::CreateLink,
                ));
            }
        };
    // SAFETY: the procfs path names the pinned source descriptor, the target
    // directory descriptor is live, and both C strings outlive the call.
    // AT_SYMLINK_FOLLOW follows only procfs's descriptor link, never a mutable
    // caller-supplied source pathname.
    let result = unsafe {
        libc::linkat(
            libc::AT_FDCWD,
            pinned_source.as_ptr(),
            target_directory.as_raw_fd(),
            target_filename.as_ptr(),
            libc::AT_SYMLINK_FOLLOW,
        )
    };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::AlreadyExists {
            return HardlinkFileStatus::Failed(HardlinkFilesystemFailure::io(
                HardlinkFilesystemOperation::CreateLink,
                error.kind(),
            ));
        }
        return match probe_target_at(&target_directory, &target_filename) {
            Ok(TargetProbe::Regular(target)) if target == expected_source => {
                HardlinkFileStatus::AcceptedExisting
            }
            Ok(TargetProbe::Regular(target)) => HardlinkFileStatus::Conflict {
                source: expected_source,
                target,
            },
            Ok(TargetProbe::Unsafe(kind)) => {
                HardlinkFileStatus::Failed(HardlinkFilesystemFailure {
                    operation: HardlinkFilesystemOperation::VerifyCreatedLink,
                    kind,
                })
            }
            Ok(TargetProbe::Missing) => HardlinkFileStatus::Failed(HardlinkFilesystemFailure::io(
                HardlinkFilesystemOperation::CreateLink,
                io::ErrorKind::AlreadyExists,
            )),
            Err(failure) => HardlinkFileStatus::Failed(failure),
        };
    }

    match probe_target_at(&target_directory, &target_filename) {
        Ok(TargetProbe::Regular(target)) if target == expected_source => {
            HardlinkFileStatus::Created
        }
        Ok(TargetProbe::Regular(target)) => HardlinkFileStatus::Conflict {
            source: expected_source,
            target,
        },
        Ok(TargetProbe::Unsafe(kind)) => HardlinkFileStatus::Failed(HardlinkFilesystemFailure {
            operation: HardlinkFilesystemOperation::VerifyCreatedLink,
            kind,
        }),
        Ok(TargetProbe::Missing) => HardlinkFileStatus::Failed(HardlinkFilesystemFailure::io(
            HardlinkFilesystemOperation::VerifyCreatedLink,
            io::ErrorKind::NotFound,
        )),
        Err(failure) => HardlinkFileStatus::Failed(failure),
    }
}

#[cfg(not(target_os = "linux"))]
fn create_hardlink(
    _source: &Path,
    _target: &Path,
    _expected_source: FileIdentity,
) -> HardlinkFileStatus {
    HardlinkFileStatus::Failed(HardlinkFilesystemFailure::io(
        HardlinkFilesystemOperation::CreateLink,
        io::ErrorKind::Unsupported,
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::subscription::effects::QbReconciledBy;

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FakeQbError {
        ResponseLost,
    }

    impl fmt::Display for FakeQbError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("response lost after qB accepted the add")
        }
    }

    impl Error for FakeQbError {}

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum AddedInputKind {
        Url,
        Bytes,
    }

    struct FakeQbTransport {
        torrents: Vec<QbTorrentInfo>,
        add_hash: String,
        tag_queries: Vec<String>,
        hash_queries: Vec<String>,
        add_calls: usize,
        added_tag: Option<String>,
        added_input: Option<AddedInputKind>,
        lose_next_add_response: bool,
    }

    impl Default for FakeQbTransport {
        fn default() -> Self {
            Self {
                torrents: Vec::new(),
                add_hash: HASH_A.to_string(),
                tag_queries: Vec::new(),
                hash_queries: Vec::new(),
                add_calls: 0,
                added_tag: None,
                added_input: None,
                lose_next_add_response: false,
            }
        }
    }

    impl QbEffectTransport for FakeQbTransport {
        type Error = FakeQbError;

        fn list_by_exact_tag<'a>(
            &'a mut self,
            stable_tag: &'a str,
        ) -> QbTransportFuture<'a, Vec<QbTorrentInfo>, Self::Error> {
            Box::pin(async move {
                self.tag_queries.push(stable_tag.to_string());
                Ok(self
                    .torrents
                    .iter()
                    .filter(|torrent| parse_qb_tags(&torrent.tags).contains(stable_tag))
                    .cloned()
                    .collect())
            })
        }

        fn list_by_hash<'a>(
            &'a mut self,
            authoritative_hash: &'a str,
        ) -> QbTransportFuture<'a, Vec<QbTorrentInfo>, Self::Error> {
            Box::pin(async move {
                self.hash_queries.push(authoritative_hash.to_string());
                Ok(self
                    .torrents
                    .iter()
                    .filter(|torrent| torrent.hash.eq_ignore_ascii_case(authoritative_hash))
                    .cloned()
                    .collect())
            })
        }

        fn add<'a>(
            &'a mut self,
            command: &'a QbAddCommand<'a>,
        ) -> QbTransportFuture<'a, (), Self::Error> {
            Box::pin(async move {
                self.add_calls += 1;
                self.added_tag = Some(command.stable_tag().to_string());
                self.added_input = Some(match command.input() {
                    QbTorrentInput::Url(_) => AddedInputKind::Url,
                    QbTorrentInput::Bytes { .. } => AddedInputKind::Bytes,
                });
                self.torrents.push(QbTorrentInfo {
                    hash: self.add_hash.clone(),
                    name: "added-by-effect".to_string(),
                    tags: command.stable_tag().to_string(),
                    ..QbTorrentInfo::default()
                });
                if std::mem::take(&mut self.lose_next_add_response) {
                    Err(FakeQbError::ResponseLost)
                } else {
                    Ok(())
                }
            })
        }
    }

    fn qb_info(hash: &str, name: &str, tags: &str) -> QbTorrentInfo {
        QbTorrentInfo {
            hash: hash.to_string(),
            name: name.to_string(),
            tags: tags.to_string(),
            ..QbTorrentInfo::default()
        }
    }

    fn qb_request(hash: Option<&str>) -> QbReconcileRequest {
        QbReconcileRequest::try_new("account", "subject", "torrent", None, hash).unwrap()
    }

    fn url_add() -> QbEffectAddSpec {
        QbEffectAddSpec::new(
            QbTorrentInput::from_url("https://tracker.invalid/download?id=SECRET_QUERY").unwrap(),
            Some("movie".to_string()),
            Some("/downloads".to_string()),
        )
    }

    #[tokio::test]
    async fn hash_and_tag_queries_are_deduplicated_before_reconciliation() {
        let request = qb_request(Some(HASH_A));
        let stable_tag = request.idempotency_key().as_str().to_string();
        let transport = FakeQbTransport {
            torrents: vec![qb_info(HASH_A, "existing", &stable_tag)],
            ..FakeQbTransport::default()
        };
        let mut adapter = QbEffectAdapter::new(transport);

        let outcome = adapter.ensure(&request, &url_add()).await.unwrap();

        assert!(matches!(
            outcome,
            EnsureQbTorrentOutcome::Reconciled {
                matched_by: QbReconciledBy::HashAndStableTag,
                ..
            }
        ));
        assert_eq!(adapter.transport().tag_queries, [stable_tag]);
        assert_eq!(adapter.transport().hash_queries, [HASH_A]);
        assert_eq!(adapter.transport().add_calls, 0);
    }

    #[tokio::test]
    async fn authoritative_hash_and_exact_tag_disagreement_fails_closed() {
        let request = qb_request(Some(HASH_A));
        let stable_tag = request.idempotency_key().as_str().to_string();
        let transport = FakeQbTransport {
            torrents: vec![
                qb_info(HASH_A, "hash-match", ""),
                qb_info(HASH_B, "tag-match", &stable_tag),
            ],
            ..FakeQbTransport::default()
        };
        let mut adapter = QbEffectAdapter::new(transport);

        let error = adapter.ensure(&request, &url_add()).await.unwrap_err();

        assert!(matches!(
            error,
            QbEffectAdapterError::Conflict(QbReconciliationConflict::HashAndTagDisagree { .. })
        ));
        assert_eq!(adapter.transport().add_calls, 0);
    }

    #[tokio::test]
    async fn response_lost_retry_observes_stable_tag_without_a_second_add() {
        let request = qb_request(Some(HASH_A));
        let transport = FakeQbTransport {
            lose_next_add_response: true,
            ..FakeQbTransport::default()
        };
        let mut adapter = QbEffectAdapter::new(transport);
        let add = url_add();

        let first = adapter.ensure(&request, &add).await.unwrap_err();
        let message = first.to_string();
        assert!(matches!(
            first,
            QbEffectAdapterError::Transport {
                stage: QbEffectStage::Add,
                source: FakeQbError::ResponseLost
            }
        ));
        assert!(!message.contains("SECRET_QUERY"));
        assert_eq!(adapter.transport().add_calls, 1);

        let retry = adapter.ensure(&request, &add).await.unwrap();
        assert!(matches!(retry, EnsureQbTorrentOutcome::Reconciled { .. }));
        assert_eq!(adapter.transport().add_calls, 1);
        assert_eq!(
            adapter.transport().added_tag.as_deref(),
            Some(request.idempotency_key().as_str())
        );
        assert_eq!(adapter.transport().added_input, Some(AddedInputKind::Url));
    }

    #[tokio::test]
    async fn bytes_add_also_receives_the_forced_stable_tag() {
        let request = qb_request(None);
        let mut adapter = QbEffectAdapter::new(FakeQbTransport::default());
        let add = QbEffectAddSpec::new(
            QbTorrentInput::from_bytes("movie.torrent", vec![1, 2, 3]).unwrap(),
            None,
            None,
        );

        let outcome = adapter.ensure(&request, &add).await.unwrap();

        assert!(matches!(outcome, EnsureQbTorrentOutcome::Added { .. }));
        assert_eq!(adapter.transport().added_input, Some(AddedInputKind::Bytes));
        assert_eq!(
            adapter.transport().added_tag.as_deref(),
            Some(request.idempotency_key().as_str())
        );
    }

    struct TempTree {
        root: PathBuf,
    }

    impl TempTree {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "tmdb-mteam-effect-{label}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.root.join(relative)
        }
    }

    impl Drop for TempTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn link_effect(source: &Path, target: &Path, outcome: LinkFileOutcome) -> LinkFileEffect {
        LinkFileEffect::try_new(source, target, outcome).unwrap()
    }

    #[cfg(unix)]
    fn inode(path: &Path) -> (u64, u64) {
        use std::os::unix::fs::MetadataExt;

        let metadata = fs::symlink_metadata(path).unwrap();
        (metadata.dev(), metadata.ino())
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn created_link_is_accepted_on_crash_style_retry_without_rewriting() {
        let tree = TempTree::new("retry");
        let source = tree.path("downloads/movie.mkv");
        let target = tree.path("library/movie.mkv");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"movie").unwrap();
        let adapter = HardlinkEffectAdapter::try_new(1).unwrap();

        let first = adapter
            .apply(vec![link_effect(
                &source,
                &target,
                LinkFileOutcome::Pending,
            )])
            .await
            .unwrap();
        assert_eq!(first.files()[0].status(), &HardlinkFileStatus::Created);
        let first_inode = inode(&target);

        let retry = adapter
            .apply(vec![link_effect(
                &source,
                &target,
                LinkFileOutcome::Pending,
            )])
            .await
            .unwrap();
        assert_eq!(
            retry.files()[0].status(),
            &HardlinkFileStatus::AcceptedExisting
        );
        assert_eq!(inode(&source), first_inode);
        assert_eq!(inode(&target), first_inode);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn conflicting_target_is_never_overwritten_or_renamed() {
        let tree = TempTree::new("conflict");
        let source = tree.path("source.mkv");
        let target = tree.path("target.mkv");
        fs::write(&source, b"source").unwrap();
        fs::write(&target, b"keep-me").unwrap();
        let target_inode = inode(&target);
        let adapter = HardlinkEffectAdapter::try_new(1).unwrap();

        let outcome = adapter
            .apply(vec![link_effect(
                &source,
                &target,
                LinkFileOutcome::Pending,
            )])
            .await
            .unwrap();

        assert!(matches!(
            outcome.files()[0].status(),
            HardlinkFileStatus::Conflict { .. }
        ));
        assert_eq!(fs::read(&target).unwrap(), b"keep-me");
        assert_eq!(inode(&target), target_inode);
        assert_eq!(fs::read_dir(&tree.root).unwrap().count(), 2);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn missing_and_probe_failures_are_reported_per_file() {
        let tree = TempTree::new("per-file-failure");
        let present = tree.path("present.mkv");
        let not_directory = tree.path("not-a-directory");
        fs::write(&present, b"present").unwrap();
        fs::write(&not_directory, b"file").unwrap();
        let adapter = HardlinkEffectAdapter::try_new(1).unwrap();

        let outcome = adapter
            .apply(vec![
                link_effect(
                    &tree.path("missing.mkv"),
                    &tree.path("library/missing.mkv"),
                    LinkFileOutcome::Missing,
                ),
                link_effect(
                    &present,
                    &not_directory.join("target.mkv"),
                    LinkFileOutcome::Failed,
                ),
            ])
            .await
            .unwrap();

        assert_eq!(outcome.files()[0].status(), &HardlinkFileStatus::Missing);
        assert!(matches!(
            outcome.files()[1].status(),
            HardlinkFileStatus::Failed(HardlinkFilesystemFailure {
                operation: HardlinkFilesystemOperation::ProbeTarget,
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn verified_sibling_is_skipped_while_unresolved_file_is_created() {
        let tree = TempTree::new("siblings");
        let source_a = tree.path("downloads/a.mkv");
        let source_b = tree.path("downloads/b.mkv");
        let target_a = tree.path("library/a.mkv");
        let target_b = tree.path("library/b.mkv");
        fs::create_dir_all(source_a.parent().unwrap()).unwrap();
        fs::create_dir_all(target_a.parent().unwrap()).unwrap();
        fs::write(&source_a, b"a").unwrap();
        fs::write(&source_b, b"b").unwrap();
        fs::hard_link(&source_a, &target_a).unwrap();
        let target_a_inode = inode(&target_a);
        let adapter = HardlinkEffectAdapter::try_new(1).unwrap();

        let outcome = adapter
            .apply(vec![
                link_effect(&source_a, &target_a, LinkFileOutcome::Linked),
                link_effect(&source_b, &target_b, LinkFileOutcome::Failed),
            ])
            .await
            .unwrap();

        assert_eq!(
            outcome.files()[0].status(),
            &HardlinkFileStatus::SkippedVerified
        );
        assert_eq!(outcome.files()[1].status(), &HardlinkFileStatus::Created);
        assert_eq!(inode(&target_a), target_a_inode);
        assert_eq!(inode(&source_b), inode(&target_b));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "current_thread")]
    async fn symlink_sources_and_symlink_parent_components_fail_closed() {
        use std::os::unix::fs::symlink;

        let tree = TempTree::new("symlink");
        let source = tree.path("source.mkv");
        let source_link = tree.path("source-link.mkv");
        let direct_target = tree.path("library/direct.mkv");
        let redirected_directory = tree.path("redirected");
        let parent_link = tree.path("library-link");
        let redirected_target = redirected_directory.join("escaped.mkv");
        fs::write(&source, b"source").unwrap();
        fs::create_dir_all(&redirected_directory).unwrap();
        symlink(&source, &source_link).unwrap();
        symlink(&redirected_directory, &parent_link).unwrap();
        let adapter = HardlinkEffectAdapter::try_new(1).unwrap();

        let outcome = adapter
            .apply(vec![
                link_effect(&source_link, &direct_target, LinkFileOutcome::Pending),
                link_effect(
                    &source,
                    &parent_link.join("escaped.mkv"),
                    LinkFileOutcome::Pending,
                ),
            ])
            .await
            .unwrap();

        assert!(matches!(
            outcome.files()[0].status(),
            HardlinkFileStatus::Failed(HardlinkFilesystemFailure {
                kind: HardlinkFilesystemFailureKind::Symlink,
                ..
            })
        ));
        assert!(matches!(
            outcome.files()[1].status(),
            HardlinkFileStatus::Failed(HardlinkFilesystemFailure {
                operation: HardlinkFilesystemOperation::CreateParent,
                ..
            })
        ));
        assert!(!direct_target.exists());
        assert!(!redirected_target.exists());
    }
}
