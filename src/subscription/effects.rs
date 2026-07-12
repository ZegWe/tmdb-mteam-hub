//! Domain contracts for retry-safe external subscription effects.
//!
//! This module deliberately knows nothing about HTTP, qBittorrent clients,
//! SQLite, or the host filesystem. Runtime adapters provide observations and
//! execute the returned actions; the rules here decide whether an effect may
//! run without duplicating or destroying an existing result.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::PathBuf;

use ring::digest::{Context, SHA256};
use serde::{Deserialize, Serialize};

const EFFECT_KEY_DOMAIN: &[u8] = b"tmdb-mteam-hub/external-effect/v1\0";
const DOWNLOAD_EFFECT_KEY_PREFIX: &str = "download:v1:";
const LINK_EFFECT_KEY_PREFIX: &str = "link:v1:";

/// The operation is part of external-effect identity; it is never inferred
/// from mutable lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ExternalEffectOperation {
    QbAddTorrent,
    HardlinkFiles,
}

impl ExternalEffectOperation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::QbAddTorrent => "qb_add_torrent",
            Self::HardlinkFiles => "hardlink_files",
        }
    }

    const fn key_prefix(self) -> &'static str {
        match self {
            Self::QbAddTorrent => DOWNLOAD_EFFECT_KEY_PREFIX,
            Self::HardlinkFiles => LINK_EFFECT_KEY_PREFIX,
        }
    }
}

/// Stable SHA-256 external-effect key.
///
/// Identity components are opaque original UTF-8 values. They are validated
/// as non-blank and NUL-free, but are not trimmed, case-folded, or Unicode-
/// normalized; doing so would alias distinct account, subject, or torrent
/// identifiers. Hash input is the domain separator followed by account key,
/// subject ID, selected torrent ID, and canonical operation label. Every
/// component is framed by an unsigned 64-bit big-endian byte length. Output is
/// the operation prefix plus lowercase SHA-256 hex and is safe as an exact qB
/// tag (it never contains a comma).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct EffectIdempotencyKey(String);

impl EffectIdempotencyKey {
    pub(crate) fn try_new(
        account_key: impl Into<String>,
        subject_id: impl Into<String>,
        selected_torrent_id: impl Into<String>,
        operation: ExternalEffectOperation,
    ) -> Result<Self, EffectIdentityError> {
        let account_key = validated_identity_component("account_key", account_key.into())?;
        let subject_id = validated_identity_component("subject_id", subject_id.into())?;
        let selected_torrent_id =
            validated_identity_component("selected_torrent_id", selected_torrent_id.into())?;
        Ok(stable_effect_key(
            &account_key,
            &subject_id,
            &selected_torrent_id,
            operation,
        ))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

/// Infallible counterpart used after repository validation has already
/// established non-blank, NUL-free identity components.
pub(crate) fn stable_qb_idempotency_key(
    account_key: &str,
    subject_id: &str,
    selected_torrent_id: &str,
) -> String {
    stable_effect_key(
        account_key,
        subject_id,
        selected_torrent_id,
        ExternalEffectOperation::QbAddTorrent,
    )
    .into_string()
}

fn stable_effect_key(
    account_key: &str,
    subject_id: &str,
    selected_torrent_id: &str,
    operation: ExternalEffectOperation,
) -> EffectIdempotencyKey {
    let mut context = Context::new(&SHA256);
    context.update(EFFECT_KEY_DOMAIN);
    for component in [
        account_key,
        subject_id,
        selected_torrent_id,
        operation.as_str(),
    ] {
        let bytes = component.as_bytes();
        let length = u64::try_from(bytes.len()).expect("UTF-8 component length must fit u64");
        context.update(&length.to_be_bytes());
        context.update(bytes);
    }
    let digest = context.finish();
    let mut encoded = String::with_capacity(
        operation.key_prefix().len() + digest.as_ref().len().saturating_mul(2),
    );
    encoded.push_str(operation.key_prefix());
    for byte in digest.as_ref() {
        use fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    EffectIdempotencyKey(encoded)
}

fn validated_identity_component(
    field: &'static str,
    value: String,
) -> Result<String, EffectIdentityError> {
    if value.trim().is_empty() {
        return Err(EffectIdentityError::Blank { field });
    }
    if value.contains('\0') {
        return Err(EffectIdentityError::ContainsNul { field });
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EffectIdentityError {
    Blank { field: &'static str },
    ContainsNul { field: &'static str },
    InvalidQbHash { field: &'static str },
    ConflictingQbHashes { stored: String, expected: String },
}

impl fmt::Display for EffectIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Blank { field } => write!(formatter, "{field} must not be blank"),
            Self::ContainsNul { field } => {
                write!(formatter, "{field} must not contain a NUL byte")
            }
            Self::InvalidQbHash { field } => write!(
                formatter,
                "{field} must be a 40- or 64-character hexadecimal qB hash"
            ),
            Self::ConflictingQbHashes { stored, expected } => write!(
                formatter,
                "stored qB hash {stored} disagrees with selected torrent hash {expected}"
            ),
        }
    }
}

impl Error for EffectIdentityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QbHashSource {
    StoredArtifact,
    SelectedTorrent,
}

/// Inputs required to reconcile one qB add effect before it is attempted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QbReconcileRequest {
    idempotency_key: EffectIdempotencyKey,
    selected_torrent_id: String,
    authoritative_hash: Option<String>,
    hash_source: Option<QbHashSource>,
}

impl QbReconcileRequest {
    pub(crate) fn try_new(
        account_key: impl Into<String>,
        subject_id: impl Into<String>,
        selected_torrent_id: impl Into<String>,
        stored_hash: Option<&str>,
        selected_torrent_hash: Option<&str>,
    ) -> Result<Self, EffectIdentityError> {
        let account_key = account_key.into();
        let subject_id = subject_id.into();
        let selected_torrent_id = selected_torrent_id.into();
        let idempotency_key = EffectIdempotencyKey::try_new(
            account_key,
            subject_id,
            selected_torrent_id.clone(),
            ExternalEffectOperation::QbAddTorrent,
        )?;
        let stored_hash = normalized_qb_hash("stored_hash", stored_hash)?;
        let selected_torrent_hash =
            normalized_qb_hash("selected_torrent_hash", selected_torrent_hash)?;
        if let (Some(stored), Some(expected)) = (&stored_hash, &selected_torrent_hash) {
            if stored != expected {
                return Err(EffectIdentityError::ConflictingQbHashes {
                    stored: stored.clone(),
                    expected: expected.clone(),
                });
            }
        }
        let (authoritative_hash, hash_source) = if let Some(hash) = stored_hash {
            (Some(hash), Some(QbHashSource::StoredArtifact))
        } else if let Some(hash) = selected_torrent_hash {
            (Some(hash), Some(QbHashSource::SelectedTorrent))
        } else {
            (None, None)
        };
        Ok(Self {
            idempotency_key,
            selected_torrent_id,
            authoritative_hash,
            hash_source,
        })
    }

    pub(crate) fn idempotency_key(&self) -> &EffectIdempotencyKey {
        &self.idempotency_key
    }

    pub(crate) fn authoritative_hash(&self) -> Option<&str> {
        self.authoritative_hash.as_deref()
    }

    fn add_request(&self) -> QbAddRequest {
        QbAddRequest {
            idempotency_key: self.idempotency_key.clone(),
            selected_torrent_id: self.selected_torrent_id.clone(),
            selected_torrent_hash: self.authoritative_hash.clone(),
        }
    }
}

fn normalized_qb_hash(
    field: &'static str,
    value: Option<&str>,
) -> Result<Option<String>, EffectIdentityError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(EffectIdentityError::InvalidQbHash { field });
    }
    Ok(Some(value.to_ascii_lowercase()))
}

/// Minimal qB observation needed by the reconciliation rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QbTorrentObservation {
    hash: String,
    name: String,
    tags: BTreeSet<String>,
}

impl QbTorrentObservation {
    pub(crate) fn try_new(
        hash: impl Into<String>,
        name: impl Into<String>,
        tags: impl IntoIterator<Item = String>,
    ) -> Result<Self, EffectIdentityError> {
        let hash_value = hash.into();
        let hash = normalized_qb_hash("observed_hash", Some(&hash_value))?
            .expect("an explicitly supplied observed hash cannot normalize to None");
        let tags = tags
            .into_iter()
            .map(|tag| tag.trim().to_string())
            .filter(|tag| !tag.is_empty())
            .collect();
        Ok(Self {
            hash,
            name: name.into(),
            tags,
        })
    }

    pub(crate) fn hash(&self) -> &str {
        &self.hash
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    fn has_exact_tag(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QbReconciledBy {
    StoredHash,
    SelectedTorrentHash,
    StableTag,
    HashAndStableTag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QbReconciliationConflict {
    DuplicateStableTag {
        hashes: Vec<String>,
    },
    HashAndTagDisagree {
        authoritative_hash: String,
        tagged_hash: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QbReconciliationDecision {
    UseExisting {
        torrent: QbTorrentObservation,
        matched_by: QbReconciledBy,
    },
    Add,
    Conflict(QbReconciliationConflict),
}

/// Reconciles a stored/precomputed hash and the stable effect tag before add.
/// Any disagreement fails closed; it never guesses and never authorizes a
/// second add while an ambiguous tagged task exists.
pub(crate) fn reconcile_qb_torrent(
    request: &QbReconcileRequest,
    observations: &[QbTorrentObservation],
) -> QbReconciliationDecision {
    let stable_tag = request.idempotency_key.as_str();
    let mut observations_by_hash = BTreeMap::<&str, &QbTorrentObservation>::new();
    let mut tagged_hashes = BTreeSet::<&str>::new();
    for observation in observations {
        observations_by_hash
            .entry(observation.hash())
            .or_insert(observation);
        if observation.has_exact_tag(stable_tag) {
            tagged_hashes.insert(observation.hash());
        }
    }

    if tagged_hashes.len() > 1 {
        return QbReconciliationDecision::Conflict(QbReconciliationConflict::DuplicateStableTag {
            hashes: tagged_hashes.into_iter().map(str::to_string).collect(),
        });
    }

    let tagged_hash = tagged_hashes.first().copied();
    if let Some(authoritative_hash) = request.authoritative_hash() {
        if let Some(tagged_hash) = tagged_hash {
            if tagged_hash != authoritative_hash {
                return QbReconciliationDecision::Conflict(
                    QbReconciliationConflict::HashAndTagDisagree {
                        authoritative_hash: authoritative_hash.to_string(),
                        tagged_hash: tagged_hash.to_string(),
                    },
                );
            }
        }
        if let Some(torrent) = observations_by_hash.get(authoritative_hash).copied() {
            return QbReconciliationDecision::UseExisting {
                torrent: torrent.clone(),
                matched_by: if tagged_hash == Some(authoritative_hash) {
                    QbReconciledBy::HashAndStableTag
                } else {
                    match request.hash_source {
                        Some(QbHashSource::StoredArtifact) => QbReconciledBy::StoredHash,
                        Some(QbHashSource::SelectedTorrent) => QbReconciledBy::SelectedTorrentHash,
                        None => unreachable!("authoritative hash always records its source"),
                    }
                },
            };
        }
        return QbReconciliationDecision::Add;
    }

    if let Some(tagged_hash) = tagged_hash {
        let torrent = observations_by_hash
            .get(tagged_hash)
            .expect("tagged hash came from the same observation set");
        return QbReconciliationDecision::UseExisting {
            torrent: (*torrent).clone(),
            matched_by: QbReconciledBy::StableTag,
        };
    }
    QbReconciliationDecision::Add
}

/// Port scoped to one configured qB account/server. Runtime implementations
/// may issue separate hash and tag queries, but must return their union before
/// `add` is called.
pub(crate) trait QbEffectPort {
    type Error;

    fn inspect(
        &mut self,
        stable_tag: &str,
        authoritative_hash: Option<&str>,
    ) -> Result<Vec<QbTorrentObservation>, Self::Error>;

    fn add(&mut self, request: &QbAddRequest) -> Result<(), Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct QbAddRequest {
    idempotency_key: EffectIdempotencyKey,
    selected_torrent_id: String,
    selected_torrent_hash: Option<String>,
}

impl QbAddRequest {
    pub(crate) fn stable_tag(&self) -> &str {
        self.idempotency_key.as_str()
    }

    pub(crate) fn selected_torrent_id(&self) -> &str {
        &self.selected_torrent_id
    }

    pub(crate) fn selected_torrent_hash(&self) -> Option<&str> {
        self.selected_torrent_hash.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum EnsureQbTorrentOutcome {
    Added {
        idempotency_key: EffectIdempotencyKey,
    },
    Reconciled {
        torrent: QbTorrentObservation,
        matched_by: QbReconciledBy,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EnsureQbTorrentError<E> {
    Port(E),
    Conflict(QbReconciliationConflict),
}

impl<E: fmt::Display> fmt::Display for EnsureQbTorrentError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Port(error) => write!(formatter, "qB effect adapter failed: {error}"),
            Self::Conflict(conflict) => {
                write!(formatter, "qB reconciliation conflict: {conflict:?}")
            }
        }
    }
}

impl<E: Error + 'static> Error for EnsureQbTorrentError<E> {}

/// Executes at most one add after reconciliation. If qB accepts the add but
/// its response is lost, this call returns the adapter error; the next call
/// observes the stable tag/hash and returns `Reconciled` without adding again.
pub(crate) fn ensure_qb_torrent<P: QbEffectPort>(
    port: &mut P,
    request: &QbReconcileRequest,
) -> Result<EnsureQbTorrentOutcome, EnsureQbTorrentError<P::Error>> {
    let observations = port
        .inspect(
            request.idempotency_key().as_str(),
            request.authoritative_hash(),
        )
        .map_err(EnsureQbTorrentError::Port)?;
    match reconcile_qb_torrent(request, &observations) {
        QbReconciliationDecision::UseExisting {
            torrent,
            matched_by,
        } => Ok(EnsureQbTorrentOutcome::Reconciled {
            torrent,
            matched_by,
        }),
        QbReconciliationDecision::Add => {
            port.add(&request.add_request())
                .map_err(EnsureQbTorrentError::Port)?;
            Ok(EnsureQbTorrentOutcome::Added {
                idempotency_key: request.idempotency_key.clone(),
            })
        }
        QbReconciliationDecision::Conflict(conflict) => {
            Err(EnsureQbTorrentError::Conflict(conflict))
        }
    }
}

/// Canonical persisted per-file hardlink outcome used by the latest artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LinkFileOutcome {
    Pending,
    Linked,
    Failed,
    Missing,
    Conflict,
}

/// Filesystem-neutral identity. Unix adapters normally map this to
/// `(st_dev, st_ino)`; other adapters may provide an equivalent stable pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FileIdentity {
    namespace: u64,
    object: u64,
}

impl FileIdentity {
    pub(crate) const fn new(namespace: u64, object: u64) -> Self {
        Self { namespace, object }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkFileEffect {
    source_path: PathBuf,
    target_path: PathBuf,
    persisted_outcome: LinkFileOutcome,
}

impl LinkFileEffect {
    pub(crate) fn try_new(
        source_path: impl Into<PathBuf>,
        target_path: impl Into<PathBuf>,
        persisted_outcome: LinkFileOutcome,
    ) -> Result<Self, LinkPlanError> {
        let source_path = source_path.into();
        let target_path = target_path.into();
        if source_path.as_os_str().is_empty() {
            return Err(LinkPlanError::EmptySourcePath);
        }
        if target_path.as_os_str().is_empty() {
            return Err(LinkPlanError::EmptyTargetPath);
        }
        if source_path == target_path {
            return Err(LinkPlanError::SourceEqualsTarget);
        }
        Ok(Self {
            source_path,
            target_path,
            persisted_outcome,
        })
    }

    pub(crate) fn source_path(&self) -> &std::path::Path {
        &self.source_path
    }

    pub(crate) fn target_path(&self) -> &std::path::Path {
        &self.target_path
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LinkFileProbe {
    pub(crate) source: Option<FileIdentity>,
    pub(crate) target: Option<FileIdentity>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkFileFailure {
    SourceMissing,
    TargetConflict {
        source: FileIdentity,
        target: FileIdentity,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkFileAction {
    /// Persisted success was verified again by equal file identity.
    SkipVerified,
    /// The target already exists as the same inode and is a successful result.
    AcceptExisting,
    /// Create exactly the deterministic target; no renamed alternative exists.
    Create,
    /// Record a non-destructive per-file failure.
    Fail(LinkFileFailure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlannedLinkFile {
    source_path: PathBuf,
    target_path: PathBuf,
    action: LinkFileAction,
}

impl PlannedLinkFile {
    pub(crate) fn source_path(&self) -> &std::path::Path {
        &self.source_path
    }

    pub(crate) fn target_path(&self) -> &std::path::Path {
        &self.target_path
    }

    pub(crate) const fn action(&self) -> LinkFileAction {
        self.action
    }

    pub(crate) const fn resulting_outcome(&self) -> LinkFileOutcome {
        match self.action {
            LinkFileAction::SkipVerified | LinkFileAction::AcceptExisting => {
                LinkFileOutcome::Linked
            }
            LinkFileAction::Create => LinkFileOutcome::Pending,
            LinkFileAction::Fail(LinkFileFailure::SourceMissing) => LinkFileOutcome::Missing,
            LinkFileAction::Fail(LinkFileFailure::TargetConflict { .. }) => {
                LinkFileOutcome::Conflict
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkPlanError {
    EmptySourcePath,
    EmptyTargetPath,
    SourceEqualsTarget,
    ObservationCountMismatch { files: usize, probes: usize },
    DuplicateTarget { target: PathBuf },
}

impl fmt::Display for LinkPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySourcePath => formatter.write_str("hardlink source path must not be empty"),
            Self::EmptyTargetPath => formatter.write_str("hardlink target path must not be empty"),
            Self::SourceEqualsTarget => {
                formatter.write_str("hardlink source and target paths must differ")
            }
            Self::ObservationCountMismatch { files, probes } => write!(
                formatter,
                "hardlink retry requires one filesystem probe per file ({files} files, {probes} probes)"
            ),
            Self::DuplicateTarget { target } => write!(
                formatter,
                "hardlink artifact contains duplicate deterministic target {}",
                target.display()
            ),
        }
    }
}

impl Error for LinkPlanError {}

/// Plans a per-file retry. Verified siblings are skipped, failed/missing
/// siblings may be retried, and a different inode at the deterministic target
/// is an explicit failure. No action can choose or mutate an alternate path.
pub(crate) fn plan_link_retry(
    files: &[LinkFileEffect],
    probes: &[LinkFileProbe],
) -> Result<Vec<PlannedLinkFile>, LinkPlanError> {
    if files.len() != probes.len() {
        return Err(LinkPlanError::ObservationCountMismatch {
            files: files.len(),
            probes: probes.len(),
        });
    }
    let mut targets = BTreeSet::new();
    let mut planned = Vec::with_capacity(files.len());
    for (file, probe) in files.iter().zip(probes) {
        if !targets.insert(file.target_path.clone()) {
            return Err(LinkPlanError::DuplicateTarget {
                target: file.target_path.clone(),
            });
        }
        let action = match (probe.source, probe.target) {
            (None, _) => LinkFileAction::Fail(LinkFileFailure::SourceMissing),
            (Some(_), None) => LinkFileAction::Create,
            (Some(source), Some(target)) if source == target => {
                if file.persisted_outcome == LinkFileOutcome::Linked {
                    LinkFileAction::SkipVerified
                } else {
                    LinkFileAction::AcceptExisting
                }
            }
            (Some(source), Some(target)) => {
                LinkFileAction::Fail(LinkFileFailure::TargetConflict { source, target })
            }
        };
        planned.push(PlannedLinkFile {
            source_path: file.source_path.clone(),
            target_path: file.target_path.clone(),
            action,
        });
    }
    Ok(planned)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum FakeQbError {
        ResponseLost,
    }

    impl fmt::Display for FakeQbError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("response lost after qB accepted the add")
        }
    }

    impl Error for FakeQbError {}

    #[derive(Default)]
    struct FakeQb {
        torrents: Vec<QbTorrentObservation>,
        add_calls: usize,
        lose_next_add_response: bool,
    }

    impl QbEffectPort for FakeQb {
        type Error = FakeQbError;

        fn inspect(
            &mut self,
            stable_tag: &str,
            authoritative_hash: Option<&str>,
        ) -> Result<Vec<QbTorrentObservation>, Self::Error> {
            Ok(self
                .torrents
                .iter()
                .filter(|torrent| {
                    authoritative_hash == Some(torrent.hash()) || torrent.has_exact_tag(stable_tag)
                })
                .cloned()
                .collect())
        }

        fn add(&mut self, request: &QbAddRequest) -> Result<(), Self::Error> {
            self.add_calls += 1;
            let hash = request
                .selected_torrent_hash()
                .unwrap_or(HASH_A)
                .to_string();
            self.torrents.push(
                QbTorrentObservation::try_new(
                    hash,
                    format!("torrent-{}", request.selected_torrent_id()),
                    [request.stable_tag().to_string()],
                )
                .unwrap(),
            );
            if std::mem::take(&mut self.lose_next_add_response) {
                return Err(FakeQbError::ResponseLost);
            }
            Ok(())
        }
    }

    fn request(expected_hash: Option<&str>) -> QbReconcileRequest {
        QbReconcileRequest::try_new(
            "account-original",
            "subject-original",
            "943109",
            None,
            expected_hash,
        )
        .unwrap()
    }

    #[test]
    fn effect_key_contract_is_length_framed_and_operation_scoped() {
        let key = EffectIdempotencyKey::try_new(
            "account-original",
            "subject-original",
            "943109",
            ExternalEffectOperation::QbAddTorrent,
        )
        .unwrap();
        assert_eq!(
            key.as_str(),
            "download:v1:bc318060939bc69006442a50cd4cc97b9031d5101ca83b77be900f90ced93870"
        );
        assert_eq!(key.as_str().len(), DOWNLOAD_EFFECT_KEY_PREFIX.len() + 64);
        assert!(!key.as_str().contains(','));

        let hardlink = EffectIdempotencyKey::try_new(
            "account-original",
            "subject-original",
            "943109",
            ExternalEffectOperation::HardlinkFiles,
        )
        .unwrap();
        assert_ne!(key, hardlink);
        assert!(hardlink.as_str().starts_with(LINK_EFFECT_KEY_PREFIX));
    }

    #[test]
    fn effect_identity_preserves_opaque_original_values_without_aliasing() {
        let exact = EffectIdempotencyKey::try_new(
            "account",
            "subject",
            "torrent",
            ExternalEffectOperation::QbAddTorrent,
        )
        .unwrap();
        let spaced = EffectIdempotencyKey::try_new(
            " account ",
            "subject",
            "torrent",
            ExternalEffectOperation::QbAddTorrent,
        )
        .unwrap();
        let cased = EffectIdempotencyKey::try_new(
            "Account",
            "subject",
            "torrent",
            ExternalEffectOperation::QbAddTorrent,
        )
        .unwrap();

        assert_ne!(exact, spaced);
        assert_ne!(exact, cased);
        assert!(matches!(
            EffectIdempotencyKey::try_new(
                "   ",
                "subject",
                "torrent",
                ExternalEffectOperation::QbAddTorrent
            ),
            Err(EffectIdentityError::Blank {
                field: "account_key"
            })
        ));
    }

    #[test]
    fn qb_response_loss_is_reconciled_without_a_second_add() {
        let request = request(Some(HASH_A));
        let mut qb = FakeQb {
            lose_next_add_response: true,
            ..FakeQb::default()
        };

        assert_eq!(
            ensure_qb_torrent(&mut qb, &request),
            Err(EnsureQbTorrentError::Port(FakeQbError::ResponseLost))
        );
        assert_eq!(qb.add_calls, 1);
        assert_eq!(qb.torrents.len(), 1, "qB accepted before response loss");
        assert_eq!(qb.torrents[0].name(), "torrent-943109");

        let retry = ensure_qb_torrent(&mut qb, &request).unwrap();
        assert!(matches!(
            retry,
            EnsureQbTorrentOutcome::Reconciled {
                matched_by: QbReconciledBy::HashAndStableTag,
                ..
            }
        ));
        assert_eq!(qb.add_calls, 1, "retry must not duplicate the qB task");
    }

    #[test]
    fn crash_after_effect_before_database_finish_does_not_repeat_add() {
        let request = request(Some(HASH_A));
        let mut qb = FakeQb::default();

        let accepted_but_not_persisted = ensure_qb_torrent(&mut qb, &request).unwrap();
        assert!(matches!(
            accepted_but_not_persisted,
            EnsureQbTorrentOutcome::Added { .. }
        ));
        assert_eq!(qb.add_calls, 1);

        let after_restart = ensure_qb_torrent(&mut qb, &request).unwrap();
        assert!(matches!(
            after_restart,
            EnsureQbTorrentOutcome::Reconciled { .. }
        ));
        assert_eq!(qb.add_calls, 1);
    }

    #[test]
    fn stable_tag_recovers_response_loss_when_no_hash_was_available() {
        let request = request(None);
        let mut qb = FakeQb {
            torrents: vec![QbTorrentObservation::try_new(
                HASH_A,
                "existing",
                [request.idempotency_key().as_str().to_string()],
            )
            .unwrap()],
            ..FakeQb::default()
        };

        let outcome = ensure_qb_torrent(&mut qb, &request).unwrap();

        assert!(matches!(
            outcome,
            EnsureQbTorrentOutcome::Reconciled {
                matched_by: QbReconciledBy::StableTag,
                ..
            }
        ));
        assert_eq!(qb.add_calls, 0);
    }

    #[test]
    fn mismatched_hash_and_stable_tag_fail_closed_without_add() {
        let request = request(Some(HASH_A));
        let mut qb = FakeQb {
            torrents: vec![
                QbTorrentObservation::try_new(HASH_A, "hash-match", Vec::new()).unwrap(),
                QbTorrentObservation::try_new(
                    HASH_B,
                    "wrong-tag-match",
                    [request.idempotency_key().as_str().to_string()],
                )
                .unwrap(),
            ],
            ..FakeQb::default()
        };

        let error = ensure_qb_torrent(&mut qb, &request).unwrap_err();

        assert!(matches!(
            error,
            EnsureQbTorrentError::Conflict(QbReconciliationConflict::HashAndTagDisagree { .. })
        ));
        assert_eq!(qb.add_calls, 0);
        assert_eq!(qb.torrents.len(), 2);
    }

    #[test]
    fn stored_and_selected_hash_disagreement_is_rejected_before_qb() {
        assert!(matches!(
            QbReconcileRequest::try_new(
                "account",
                "subject",
                "torrent",
                Some(HASH_A),
                Some(HASH_B)
            ),
            Err(EffectIdentityError::ConflictingQbHashes { .. })
        ));
    }

    #[test]
    fn same_inode_is_success_and_only_verified_files_are_skipped() {
        let identity = FileIdentity::new(7, 11);
        let files = vec![
            LinkFileEffect::try_new(
                "/downloads/already.mkv",
                "/library/already.mkv",
                LinkFileOutcome::Linked,
            )
            .unwrap(),
            LinkFileEffect::try_new(
                "/downloads/reconciled.mkv",
                "/library/reconciled.mkv",
                LinkFileOutcome::Failed,
            )
            .unwrap(),
        ];
        let probes = vec![
            LinkFileProbe {
                source: Some(identity),
                target: Some(identity),
            },
            LinkFileProbe {
                source: Some(identity),
                target: Some(identity),
            },
        ];

        let planned = plan_link_retry(&files, &probes).unwrap();

        assert_eq!(planned[0].action(), LinkFileAction::SkipVerified);
        assert_eq!(planned[1].action(), LinkFileAction::AcceptExisting);
        assert_eq!(planned[0].resulting_outcome(), LinkFileOutcome::Linked);
        assert_eq!(planned[1].resulting_outcome(), LinkFileOutcome::Linked);
    }

    #[test]
    fn target_conflict_is_non_destructive_and_never_renamed() {
        let target = PathBuf::from("/library/movie.mkv");
        let file = LinkFileEffect::try_new(
            "/downloads/movie.mkv",
            target.clone(),
            LinkFileOutcome::Pending,
        )
        .unwrap();
        let planned = plan_link_retry(
            &[file],
            &[LinkFileProbe {
                source: Some(FileIdentity::new(1, 10)),
                target: Some(FileIdentity::new(1, 20)),
            }],
        )
        .unwrap();

        assert!(matches!(
            planned[0].action(),
            LinkFileAction::Fail(LinkFileFailure::TargetConflict { .. })
        ));
        assert_eq!(planned[0].target_path(), target);
        assert_eq!(planned[0].resulting_outcome(), LinkFileOutcome::Conflict);
    }

    #[test]
    fn per_file_retry_skips_verified_sibling_and_retries_only_unresolved_files() {
        let files = vec![
            LinkFileEffect::try_new(
                "/downloads/verified.mkv",
                "/library/verified.mkv",
                LinkFileOutcome::Linked,
            )
            .unwrap(),
            LinkFileEffect::try_new(
                "/downloads/failed.mkv",
                "/library/failed.mkv",
                LinkFileOutcome::Failed,
            )
            .unwrap(),
            LinkFileEffect::try_new(
                "/downloads/was-missing.mkv",
                "/library/was-missing.mkv",
                LinkFileOutcome::Missing,
            )
            .unwrap(),
        ];
        let verified = FileIdentity::new(3, 30);
        let planned = plan_link_retry(
            &files,
            &[
                LinkFileProbe {
                    source: Some(verified),
                    target: Some(verified),
                },
                LinkFileProbe {
                    source: Some(FileIdentity::new(3, 31)),
                    target: None,
                },
                LinkFileProbe {
                    source: Some(FileIdentity::new(3, 32)),
                    target: None,
                },
            ],
        )
        .unwrap();

        assert_eq!(planned[0].action(), LinkFileAction::SkipVerified);
        assert_eq!(planned[1].action(), LinkFileAction::Create);
        assert_eq!(planned[2].action(), LinkFileAction::Create);
        assert_eq!(
            planned
                .iter()
                .filter(|file| file.action() == LinkFileAction::Create)
                .count(),
            2
        );
        assert_eq!(
            planned[1].source_path(),
            std::path::Path::new("/downloads/failed.mkv")
        );
    }

    #[test]
    fn link_file_outcome_json_includes_explicit_conflict() {
        assert_eq!(
            serde_json::to_string(&LinkFileOutcome::Conflict).unwrap(),
            r#""conflict""#
        );
        assert_eq!(
            serde_json::from_str::<LinkFileOutcome>(r#""linked""#).unwrap(),
            LinkFileOutcome::Linked
        );
    }
}
