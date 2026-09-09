//! Transactional runtime activation for the Morpheus desktop launcher.
//!
//! The launcher state root is a private trust boundary owned by the current
//! user. It is opened as a real, non-symlink directory, forced to mode 0700,
//! and mutated by cooperating launcher processes under the state-root lock.
//! A parent-anchored durable identity binds later CLI invocations to the same
//! state-root device and inode for the lifetime of launcher state.
//! Directory-FD-relative operations defend against persisted path corruption,
//! symlink traversal, pre-planted entries, and one-shot pathname replacement.
//! They do not claim to isolate the launcher from a continuously malicious
//! process running as the same user and modifying this private directory.

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::DateTime;
use chrono::Duration;
use chrono::Utc;
use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeSet;
#[cfg(target_os = "macos")]
use std::ffi::CStr;
#[cfg(unix)]
use std::ffi::CString;
use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::fd::FromRawFd;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "macos")]
use std::os::unix::fs::MetadataExt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitStatus;
use std::sync::Arc;
use uuid::Uuid;

pub const SCHEMA_VERSION: u32 = 1;
pub const COORDINATED_RESTART_EXIT_CODE: i32 = 75;
pub const RECOVERY_RESTART_EXIT_CODE: i32 = 76;
pub const READY_TIMEOUT_SECS: u64 = 45;
const MAX_FAILURE_EVIDENCE: usize = 32;
const OWNERSHIP_MARKER: &str = ".launcher-owned.json";
const ROOT_IDENTITY_PREFIX: &str = ".runtime-launcher-root-identity-";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RootIdentity {
    schema_version: u32,
    root_device: u64,
    root_inode: u64,
    owner_nonce: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct OwnedEntityMarker {
    schema_version: u32,
    transaction_id: String,
    kind: String,
    owner_nonce: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActivationMode {
    #[default]
    Full,
    Hot,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivationRequest {
    pub schema_version: u32,
    pub transaction_id: String,
    pub request_id: String,
    #[serde(default)]
    pub requested_by_thread_id: Option<String>,
    pub mode: ActivationMode,
    pub build_id: String,
    pub source_commit: String,
    pub prepared_root: PathBuf,
    pub app_bundle_path: PathBuf,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedManifest {
    pub schema_version: u32,
    pub build_id: String,
    pub source_commit: String,
    pub artifacts: Vec<PreparedArtifact>,
    pub changes: ManifestChanges,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparedArtifact {
    pub relative_path: PathBuf,
    pub sha256: String,
    pub kind: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestChanges {
    pub main: bool,
    pub preload: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyIdentity {
    pub schema_version: u32,
    pub transaction_id: String,
    pub build_id: String,
    pub instance_id: String,
    pub ready_at_ms: i64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeState {
    pub schema_version: u32,
    pub current: Option<BuildRecord>,
    pub previous: Option<BuildRecord>,
    pub blocked_build_hashes: BTreeSet<String>,
    #[serde(default)]
    pub blocked_build_ids: BTreeSet<String>,
    #[serde(default)]
    pub blocked_artifact_hashes: BTreeSet<String>,
    pub crash_history: Vec<CrashRecord>,
    pub failures: Vec<FailureEvidence>,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            current: None,
            previous: None,
            blocked_build_hashes: BTreeSet::new(),
            blocked_build_ids: BTreeSet::new(),
            blocked_artifact_hashes: BTreeSet::new(),
            crash_history: Vec::new(),
            failures: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildRecord {
    #[serde(default)]
    pub transaction_id: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub requested_by_thread_id: Option<String>,
    #[serde(default)]
    pub mode: ActivationMode,
    pub build_id: String,
    pub source_commit: String,
    pub manifest_hash: String,
    #[serde(default)]
    pub artifact_content_hash: String,
    pub app_bundle_path: PathBuf,
    pub activated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashRecord {
    pub build_hash: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureEvidence {
    pub schema_version: u32,
    #[serde(default)]
    pub recovery_identity: Option<String>,
    #[serde(default)]
    pub launcher_owner_nonce: Option<String>,
    pub occurred_at: DateTime<Utc>,
    pub transaction_id: Option<String>,
    pub request_id: Option<String>,
    pub requested_by_thread_id: Option<String>,
    pub mode: Option<ActivationMode>,
    pub build_id: Option<String>,
    pub source_commit: Option<String>,
    pub manifest_hash: Option<String>,
    #[serde(default)]
    pub failed_build_hash: Option<String>,
    pub failure_phase: String,
    pub summary: String,
    pub reason: Option<String>,
    pub app_bundle_path: Option<PathBuf>,
    pub exit_code: Option<i32>,
    pub signal: Option<String>,
    pub ready_timeout_ms: Option<u64>,
    pub log_path: Option<PathBuf>,
    pub transaction_path: Option<PathBuf>,
    pub recovered_build_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_id: Option<String>,
    #[serde(default)]
    pub acknowledged: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureEvidenceClaim {
    pub schema_version: u32,
    pub claim_id: String,
    pub recovery_identity: String,
    pub launcher_owner_nonce: String,
    pub source_version_token: String,
    pub version_token: String,
    pub evidence: FailureEvidence,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FailureEvidenceClaimWire {
    schema_version: u32,
    claim_id: String,
    recovery_identity: String,
    launcher_owner_nonce: String,
    source_version_token: String,
    version_token: String,
    evidence: ClaimedFailureEvidence,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClaimedFailureEvidence {
    schema_version: u32,
    #[serde(deserialize_with = "deserialize_required_option")]
    recovery_identity: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    launcher_owner_nonce: Option<String>,
    #[serde(deserialize_with = "deserialize_canonical_utc_datetime")]
    occurred_at: DateTime<Utc>,
    #[serde(deserialize_with = "deserialize_required_option")]
    transaction_id: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    request_id: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    requested_by_thread_id: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    mode: Option<ActivationMode>,
    #[serde(deserialize_with = "deserialize_required_option")]
    build_id: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    source_commit: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    manifest_hash: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    failed_build_hash: Option<String>,
    failure_phase: String,
    summary: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    reason: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    app_bundle_path: Option<PathBuf>,
    #[serde(deserialize_with = "deserialize_required_option")]
    exit_code: Option<i32>,
    #[serde(deserialize_with = "deserialize_required_option")]
    signal: Option<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    ready_timeout_ms: Option<u64>,
    #[serde(deserialize_with = "deserialize_required_option")]
    log_path: Option<PathBuf>,
    #[serde(deserialize_with = "deserialize_required_option")]
    transaction_path: Option<PathBuf>,
    #[serde(deserialize_with = "deserialize_required_option")]
    recovered_build_id: Option<String>,
    #[serde(default)]
    claim_id: Option<String>,
    acknowledged: bool,
}

fn deserialize_required_option<'de, D, T>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    Option<T>: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

fn deserialize_canonical_utc_datetime<'de, D>(
    deserializer: D,
) -> std::result::Result<DateTime<Utc>, D::Error>
where
    D: Deserializer<'de>,
{
    let input = String::deserialize(deserializer)?;
    let parsed = serde_json::from_value::<DateTime<Utc>>(serde_json::Value::String(input.clone()))
        .map_err(serde::de::Error::custom)?;
    let canonical = serde_json::to_value(&parsed)
        .map_err(serde::de::Error::custom)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            serde::de::Error::custom(
                "claimed failure evidence occurredAt did not serialize as a string",
            )
        })?;
    if canonical != input {
        return Err(serde::de::Error::custom(
            "claimed failure evidence occurredAt is not canonical",
        ));
    }
    Ok(parsed)
}

impl From<ClaimedFailureEvidence> for FailureEvidence {
    fn from(evidence: ClaimedFailureEvidence) -> Self {
        Self {
            schema_version: evidence.schema_version,
            recovery_identity: evidence.recovery_identity,
            launcher_owner_nonce: evidence.launcher_owner_nonce,
            occurred_at: evidence.occurred_at,
            transaction_id: evidence.transaction_id,
            request_id: evidence.request_id,
            requested_by_thread_id: evidence.requested_by_thread_id,
            mode: evidence.mode,
            build_id: evidence.build_id,
            source_commit: evidence.source_commit,
            manifest_hash: evidence.manifest_hash,
            failed_build_hash: evidence.failed_build_hash,
            failure_phase: evidence.failure_phase,
            summary: evidence.summary,
            reason: evidence.reason,
            app_bundle_path: evidence.app_bundle_path,
            exit_code: evidence.exit_code,
            signal: evidence.signal,
            ready_timeout_ms: evidence.ready_timeout_ms,
            log_path: evidence.log_path,
            transaction_path: evidence.transaction_path,
            recovered_build_id: evidence.recovered_build_id,
            claim_id: evidence.claim_id,
            acknowledged: evidence.acknowledged,
        }
    }
}

impl<'de> Deserialize<'de> for FailureEvidenceClaim {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = FailureEvidenceClaimWire::deserialize(deserializer)?;
        Ok(Self {
            schema_version: wire.schema_version,
            claim_id: wire.claim_id,
            recovery_identity: wire.recovery_identity,
            launcher_owner_nonce: wire.launcher_owner_nonce,
            source_version_token: wire.source_version_token,
            version_token: wire.version_token,
            evidence: wire.evidence.into(),
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TransactionPhase {
    Prepared,
    Activating,
    CandidateInstalled,
    CandidateStarted,
    Ready,
    RollingBack,
    RollbackComplete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PostReadyRollbackPhase {
    Planned,
    SlotRestored,
    ResourcesInstalled,
    Signed,
    Verified,
    StateCommitted,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceSwapPhase {
    #[default]
    NotStarted,
    ReplacementPrepared,
    OriginalBackedUp,
    CandidateInstalled,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceRestorePhase {
    #[default]
    NotStarted,
    DestinationRetired,
    BackupRestored,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SlotRotationPhase {
    #[default]
    NotStarted,
    PreviousRetired,
    CurrentPromoted,
    CandidateInstalled,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SlotRestorePhase {
    #[default]
    NotStarted,
    CandidateRetired,
    PreviousRestored,
    RetiredRestored,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SignatureRestorePhase {
    #[default]
    NotStarted,
    RuntimeRetired,
    RuntimeRestored,
    SignatureRetired,
    SignatureRestored,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SupersededTransaction {
    pub transaction_id: String,
    pub build_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostReadyRollback {
    pub failed_build: BuildRecord,
    pub fallback_build: BuildRecord,
    pub failure_evidence: FailureEvidence,
    pub phase: PostReadyRollbackPhase,
    pub failed_slot_path: PathBuf,
    pub resources_replacement_path: PathBuf,
    pub resources_backup_path: PathBuf,
    #[serde(default)]
    pub slot_restore_phase: SlotRestorePhase,
    #[serde(default)]
    pub superseded_transaction: Option<SupersededTransaction>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub schema_version: u32,
    pub request: ActivationRequest,
    pub manifest_hash: String,
    #[serde(default)]
    pub artifact_content_hash: String,
    pub phase: TransactionPhase,
    pub instance_id: Option<String>,
    #[serde(default)]
    pub slot_rotated: bool,
    #[serde(default)]
    pub slot_rotation_phase: SlotRotationPhase,
    #[serde(default)]
    pub slot_restore_phase: SlotRestorePhase,
    #[serde(default)]
    pub slot_retired_path: Option<PathBuf>,
    #[serde(default)]
    pub slot_failed_path: Option<PathBuf>,
    #[serde(default)]
    pub slot_had_previous: Option<bool>,
    #[serde(default)]
    pub resources_swapped: bool,
    pub resources_backup_path: Option<PathBuf>,
    #[serde(default)]
    pub resources_replacement_path: Option<PathBuf>,
    #[serde(default)]
    pub resources_retired_path: Option<PathBuf>,
    #[serde(default)]
    pub resource_swap_phase: ResourceSwapPhase,
    #[serde(default)]
    pub resource_restore_phase: ResourceRestorePhase,
    pub signature_backup_path: Option<PathBuf>,
    #[serde(default)]
    pub signature_backup_ready: Option<bool>,
    #[serde(default)]
    pub signature_backup_had_code_signature: Option<bool>,
    #[serde(default)]
    pub signature_restore_phase: SignatureRestorePhase,
    #[serde(default)]
    pub runtime_retired_path: Option<PathBuf>,
    #[serde(default)]
    pub runtime_replacement_path: Option<PathBuf>,
    #[serde(default)]
    pub signature_retired_path: Option<PathBuf>,
    #[serde(default)]
    pub signature_replacement_path: Option<PathBuf>,
    #[serde(default)]
    pub signature_replacement_ready: Option<bool>,
    #[serde(default)]
    pub launcher_expected_hash: Option<String>,
    #[serde(default)]
    pub rollback_failure_evidence: Option<FailureEvidence>,
    #[serde(default)]
    pub post_ready_rollback: Option<PostReadyRollback>,
    pub started_at: DateTime<Utc>,
}

pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&OsStr]) -> Result<ExitStatus>;
}

pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&OsStr]) -> Result<ExitStatus> {
        Command::new(program)
            .args(args)
            .status()
            .with_context(|| format!("failed to run {program}"))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CrashDecision {
    RestartCurrent,
    RollBack,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
    #[cfg(unix)]
    authority: Option<Arc<StateRootAuthorityInner>>,
}

impl Layout {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            #[cfg(unix)]
            authority: None,
        }
    }

    pub fn active_root(&self) -> Result<PathBuf> {
        #[cfg(unix)]
        if let Some(authority) = &self.authority {
            return directory_fd_path(&authority.root);
        }
        Ok(self.root.clone())
    }

    pub fn current(&self) -> Result<PathBuf> {
        Ok(self.active_root()?.join("current"))
    }

    pub fn previous(&self) -> Result<PathBuf> {
        Ok(self.active_root()?.join("previous"))
    }

    pub fn staging(&self) -> Result<PathBuf> {
        Ok(self.active_root()?.join("staging"))
    }

    pub fn transaction(&self) -> Result<PathBuf> {
        Ok(self.active_root()?.join("transaction.json"))
    }

    pub fn configured_transaction(&self) -> PathBuf {
        self.root.join("transaction.json")
    }

    pub fn state(&self) -> Result<PathBuf> {
        Ok(self.active_root()?.join("state.json"))
    }

    pub fn ready(&self) -> Result<PathBuf> {
        Ok(self.active_root()?.join("ready.json"))
    }

    pub fn configured_ready(&self) -> PathBuf {
        self.root.join("ready.json")
    }

    pub fn failure_evidence(&self) -> Result<PathBuf> {
        Ok(self.active_root()?.join("failure-evidence.json"))
    }

    pub fn configured_failure_evidence(&self) -> PathBuf {
        self.root.join("failure-evidence.json")
    }

    pub fn root_owner_nonce(&self) -> Result<String> {
        #[cfg(unix)]
        {
            if let Some(authority) = &self.authority {
                return Ok(authority.root_identity.owner_nonce.clone());
            }
            let (_parent, _root, identity) = open_trusted_state_root(self, true)?;
            Ok(identity.owner_nonce)
        }
        #[cfg(not(unix))]
        {
            bail!("trusted launcher state roots are unsupported on this platform")
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct StateRootAuthorityInner {
    _root_parent: File,
    root: File,
    root_identity: RootIdentity,
    configured_root: PathBuf,
}

#[derive(Clone, Debug)]
pub struct StateRootAuthority {
    #[cfg(unix)]
    inner: Arc<StateRootAuthorityInner>,
}

impl StateRootAuthority {
    pub fn open(layout: &Layout) -> Result<Self> {
        #[cfg(unix)]
        {
            if let Some(inner) = &layout.authority {
                return Ok(Self {
                    inner: Arc::clone(inner),
                });
            }
            let (root_parent, root, root_identity) = open_trusted_state_root(layout, true)?;
            Ok(Self {
                inner: Arc::new(StateRootAuthorityInner {
                    _root_parent: root_parent,
                    root,
                    root_identity,
                    configured_root: layout.root.clone(),
                }),
            })
        }
        #[cfg(not(unix))]
        {
            let _ = layout;
            bail!("trusted launcher state roots are unsupported on this platform")
        }
    }

    pub fn layout(&self) -> Layout {
        #[cfg(unix)]
        {
            Layout {
                root: self.inner.configured_root.clone(),
                authority: Some(Arc::clone(&self.inner)),
            }
        }
        #[cfg(not(unix))]
        {
            Layout::new(PathBuf::new())
        }
    }

    pub fn open_lock_file(&self) -> Result<File> {
        #[cfg(unix)]
        {
            open_or_create_regular_file_at(self.inner.root.as_raw_fd(), OsStr::new("launcher.lock"))
        }
        #[cfg(not(unix))]
        {
            bail!("runtime launcher locking is unsupported on this platform")
        }
    }

    pub fn duplicate_inheritable_root(&self) -> Result<(File, PathBuf)> {
        #[cfg(unix)]
        {
            let descriptor = unsafe { libc::fcntl(self.inner.root.as_raw_fd(), libc::F_DUPFD, 3) };
            if descriptor < 0 {
                return Err(std::io::Error::last_os_error())
                    .context("failed to duplicate state-root descriptor for child runtime");
            }
            let root = unsafe { File::from_raw_fd(descriptor) };
            let path = directory_fd_path(&root)?;
            Ok((root, path))
        }
        #[cfg(not(unix))]
        {
            bail!("inheritable launcher state roots are unsupported on this platform")
        }
    }
}

pub fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    let contents = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_slice(&contents)
        .with_context(|| format!("failed to parse JSON from {}", path.display()))
}

pub fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let temp_path = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(OsStr::to_str).unwrap_or("state"),
        Uuid::new_v4()
    ));
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = File::create(&temp_path)
        .with_context(|| format!("failed to create {}", temp_path.display()))?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temp_path, path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn pending_failure_evidence_path(layout: &Layout, recovery_identity: &str) -> Result<PathBuf> {
    Ok(layout.active_root()?.join(format!(
        ".failure-evidence.pending-{recovery_identity}.json"
    )))
}

pub fn claimed_failure_evidence_path(layout: &Layout, recovery_identity: &str) -> Result<PathBuf> {
    Ok(layout.active_root()?.join(format!(
        ".failure-evidence.claimed-{recovery_identity}.json"
    )))
}

pub fn consumed_failure_evidence_path(layout: &Layout, recovery_identity: &str) -> Result<PathBuf> {
    Ok(layout.active_root()?.join(format!(
        ".failure-evidence.consumed-{recovery_identity}.json"
    )))
}

pub fn failure_evidence_version(evidence: &FailureEvidence) -> Result<String> {
    let canonical_bytes = serde_json::to_vec(evidence)?;
    Ok(format!("sha256:{:x}", Sha256::digest(&canonical_bytes)))
}

/// Persists an active failure or returns the immutable claimed/consumed record
/// for the same recovery identity. Callers must use the returned record:
/// `claim_id=Some` freezes delivery, while `acknowledged=true` means the
/// identity was already completed. Neither state may be revived by a stale
/// producer payload.
pub fn persist_failure_evidence(
    layout: &Layout,
    evidence: &FailureEvidence,
) -> Result<FailureEvidence> {
    let mut evidence = evidence.clone();
    let owner_nonce = layout.root_owner_nonce()?;
    if let Some(existing) = evidence.launcher_owner_nonce.as_deref()
        && existing != owner_nonce
    {
        bail!("failure evidence belongs to a different launcher state root");
    }
    evidence.launcher_owner_nonce = Some(owner_nonce.clone());
    let identity = evidence
        .recovery_identity
        .as_deref()
        .context("failure evidence has no recovery identity")?
        .to_string();
    let parsed_identity =
        Uuid::parse_str(&identity).context("failure evidence recovery identity is invalid")?;
    if parsed_identity.to_string() != identity {
        bail!("failure evidence recovery identity is not canonical");
    }
    let preflight = preflight_failure_evidence_identity(layout, &evidence, &owner_nonce)?;
    if let Some((consumed_path, original_consumed, completed)) = preflight.completed {
        if original_consumed.claim_id.is_some() {
            // Claim-derived completion is an absolute freeze. A producer
            // cannot enrich or rewrite any field after finalize.
            return Ok(original_consumed);
        }
        // A consumed identity is terminal. Repeated producer persistence is
        // absorbed by that durable completion fact and cannot recreate
        // current/pending delivery. Only monotonic completion metadata may
        // enrich the consumed artifact.
        if completed != original_consumed {
            write_json_atomically(&consumed_path, &completed)?;
        }
        return Ok(completed);
    }
    if let Some(claimed) = preflight.claimed {
        // A claim freezes the complete payload before it is exposed to the
        // host. Stale producers for the same physical failure are absorbed by
        // that immutable record and cannot recreate or update active delivery.
        return Ok(claimed.evidence);
    }
    evidence.failed_build_hash = preflight.failed_build_hash;
    evidence.acknowledged |= preflight.acknowledged;

    let pending_path = pending_failure_evidence_path(layout, &identity)?;
    if regular_failure_artifact_exists(&pending_path, "pending failure evidence")? {
        let mut pending: FailureEvidence = read_json(&pending_path)?;
        if pending.launcher_owner_nonce.as_deref() != Some(owner_nonce.as_str()) {
            bail!("pending failure evidence belongs to another launcher state root");
        }
        require_failure_artifact_provenance(&pending, &evidence)?;
        if evidence.failed_build_hash.is_none() {
            evidence.failed_build_hash = pending.failed_build_hash.clone();
        }
        evidence.acknowledged |= pending.acknowledged;
        pending = evidence.clone();
        write_json_atomically(&pending_path, &pending)?;
    } else {
        // Journal-first: the identity-scoped pending artifact is the first
        // durable fact. A crash after this write cannot lose a newer failure.
        write_json_atomically(&pending_path, &evidence)?;
    }

    let current_path = layout.failure_evidence()?;
    if regular_failure_artifact_exists(&current_path, "current failure evidence")? {
        let mut current: FailureEvidence = read_json(&current_path)?;
        if let Some(existing) = current.launcher_owner_nonce.as_deref()
            && existing != owner_nonce
        {
            bail!("current failure evidence belongs to a different launcher state root");
        }
        current.launcher_owner_nonce = Some(owner_nonce.clone());
        let current_identity = current
            .recovery_identity
            .as_deref()
            .context("current failure evidence has no recovery identity")?;
        let parsed_current_identity = Uuid::parse_str(current_identity)
            .context("current failure evidence recovery identity is invalid")?;
        if parsed_current_identity.to_string() != current_identity {
            bail!("current failure evidence recovery identity is not canonical");
        }
        if current_identity == identity {
            require_failure_artifact_provenance(&current, &evidence)?;
            if evidence.failed_build_hash.is_none() {
                evidence.failed_build_hash = current.failed_build_hash.clone();
            }
            evidence.acknowledged |= current.acknowledged;
            write_json_atomically(&current_path, &evidence)?;
            fs::remove_file(&pending_path)?;
            sync_dir(&layout.active_root()?)?;
        }
    } else {
        promote_oldest_pending_failure_evidence(layout, &owner_nonce)?;
    }
    Ok(evidence)
}

struct FailureEvidencePreflight {
    acknowledged: bool,
    failed_build_hash: Option<String>,
    completed: Option<(PathBuf, FailureEvidence, FailureEvidence)>,
    claimed: Option<FailureEvidenceClaim>,
}

fn preflight_failure_evidence_identity(
    layout: &Layout,
    incoming: &FailureEvidence,
    owner_nonce: &str,
) -> Result<FailureEvidencePreflight> {
    let identity = incoming
        .recovery_identity
        .as_deref()
        .context("failure evidence has no recovery identity")?;
    let mut acknowledged = incoming.acknowledged;
    let mut failed_build_hash = incoming.failed_build_hash.clone();
    let mut consumed = None;
    let mut claimed = None;
    let mut paths = Vec::new();
    if regular_failure_artifact_exists(&layout.failure_evidence()?, "current failure evidence")? {
        paths.push(layout.failure_evidence()?);
    }
    for entry in fs::read_dir(layout.active_root()?)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with(".failure-evidence.pending-")
            || name.starts_with(".failure-evidence.claimed-")
            || name.starts_with(".failure-evidence.consumed-")
        {
            paths.push(entry.path());
        }
    }
    for path in paths {
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            bail!("failure evidence preflight target must be a regular file");
        }
        let is_claimed = path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.starts_with(".failure-evidence.claimed-"));
        let claim = is_claimed
            .then(|| read_json::<FailureEvidenceClaim>(&path))
            .transpose()?;
        let existing = if let Some(claim) = &claim {
            claim.evidence.clone()
        } else {
            read_json::<FailureEvidence>(&path)?
        };
        let existing_identity = canonical_failure_recovery_identity(&existing)?;
        let artifact_state =
            validate_scoped_failure_evidence_path(layout, &path, existing_identity)?;
        if existing.launcher_owner_nonce.as_deref() != Some(owner_nonce) {
            bail!("failure evidence preflight owner does not match state root");
        }
        if let Some(claim) = &claim {
            validate_failure_evidence_claim(claim, owner_nonce)?;
        }
        if artifact_state == "consumed" {
            validate_claim_derived_consumed(&existing, owner_nonce)?;
        }
        if existing_identity != identity {
            continue;
        }
        require_failure_artifact_provenance(&existing, incoming)?;
        acknowledged |= existing.acknowledged || artifact_state == "consumed";
        match (
            failed_build_hash.as_deref(),
            existing.failed_build_hash.as_deref(),
        ) {
            (Some(known), Some(existing)) if known != existing => {
                bail!("failure evidence identity has conflicting failed build hashes");
            }
            (None, Some(existing)) => {
                failed_build_hash = Some(existing.to_string());
            }
            _ => {}
        }
        if let Some(claim) = claim {
            claimed = Some(claim);
        } else if artifact_state == "consumed" {
            consumed = Some((path, existing));
        }
    }
    if let (Some((_, consumed)), Some(claim)) = (&consumed, &claimed) {
        let mut expected = claim.evidence.clone();
        expected.acknowledged = true;
        if consumed != &expected {
            bail!("claim-derived consumed evidence differs from its immutable claim");
        }
    }
    let completed = consumed.map(|(path, original)| {
        let mut completed = original.clone();
        if original.claim_id.is_none() {
            completed.launcher_owner_nonce = Some(owner_nonce.to_string());
            completed.failed_build_hash = failed_build_hash.clone();
            completed.acknowledged = true;
        }
        (path, original, completed)
    });
    Ok(FailureEvidencePreflight {
        acknowledged,
        failed_build_hash,
        completed,
        claimed,
    })
}

fn regular_failure_artifact_exists(path: &Path, label: &str) -> Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(false);
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect {label} {}", path.display()));
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("{label} must be a regular file");
    }
    Ok(true)
}

fn canonical_failure_recovery_identity(evidence: &FailureEvidence) -> Result<&str> {
    let identity = evidence
        .recovery_identity
        .as_deref()
        .context("failure evidence artifact has no recovery identity")?;
    let parsed = Uuid::parse_str(identity)
        .context("failure evidence artifact recovery identity is invalid")?;
    if parsed.to_string() != identity {
        bail!("failure evidence artifact recovery identity is not canonical");
    }
    Ok(identity)
}

fn validate_scoped_failure_evidence_path(
    layout: &Layout,
    path: &Path,
    payload_identity: &str,
) -> Result<&'static str> {
    if path == layout.failure_evidence()? {
        return Ok("current");
    }
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .context("failure evidence artifact filename is not valid UTF-8")?;
    let (scoped_identity, artifact_state) =
        if let Some(identity) = name.strip_prefix(".failure-evidence.pending-") {
            (
                identity
                    .strip_suffix(".json")
                    .context("pending failure evidence filename is malformed")?,
                "pending",
            )
        } else if let Some(identity) = name.strip_prefix(".failure-evidence.claimed-") {
            (
                identity
                    .strip_suffix(".json")
                    .context("claimed failure evidence filename is malformed")?,
                "claimed",
            )
        } else if let Some(identity) = name.strip_prefix(".failure-evidence.consumed-") {
            (
                identity
                    .strip_suffix(".json")
                    .context("consumed failure evidence filename is malformed")?,
                "consumed",
            )
        } else {
            bail!("failure evidence preflight found an unexpected artifact");
        };
    let parsed = Uuid::parse_str(scoped_identity)
        .context("scoped failure evidence filename has invalid identity")?;
    if parsed.to_string() != scoped_identity {
        bail!("scoped failure evidence filename identity is not canonical");
    }
    if scoped_identity != payload_identity {
        bail!("scoped failure evidence filename does not match payload identity");
    }
    Ok(artifact_state)
}

pub fn validate_failure_evidence_claim(
    claim: &FailureEvidenceClaim,
    owner_nonce: &str,
) -> Result<()> {
    if claim.schema_version != SCHEMA_VERSION {
        bail!("failure evidence claim schema version is unsupported");
    }
    if claim.evidence.schema_version != SCHEMA_VERSION {
        bail!("claimed failure evidence schema version is unsupported");
    }
    let claim_id =
        Uuid::parse_str(&claim.claim_id).context("failure evidence claim id is invalid")?;
    if claim_id.to_string() != claim.claim_id {
        bail!("failure evidence claim id is not canonical");
    }
    let identity = canonical_failure_recovery_identity(&claim.evidence)?;
    if claim.recovery_identity != identity
        || claim.launcher_owner_nonce != owner_nonce
        || claim.evidence.launcher_owner_nonce.as_deref() != Some(owner_nonce)
        || claim.evidence.claim_id.as_deref() != Some(claim.claim_id.as_str())
        || claim.evidence.acknowledged
    {
        bail!("failure evidence claim metadata does not match its frozen payload");
    }
    if failure_evidence_version(&claim.evidence)? != claim.version_token {
        bail!("failure evidence claim version token does not match its frozen payload");
    }
    for token in [&claim.source_version_token, &claim.version_token] {
        if token.len() != "sha256:".len() + 64
            || !token.starts_with("sha256:")
            || !token["sha256:".len()..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            bail!("failure evidence claim has an invalid version token");
        }
    }
    Ok(())
}

pub fn validate_claim_derived_consumed(
    evidence: &FailureEvidence,
    owner_nonce: &str,
) -> Result<()> {
    let Some(claim_id) = evidence.claim_id.as_deref() else {
        return Ok(());
    };
    let parsed = Uuid::parse_str(claim_id)
        .context("claim-derived consumed evidence has an invalid claim id")?;
    if parsed.to_string() != claim_id
        || evidence.launcher_owner_nonce.as_deref() != Some(owner_nonce)
        || canonical_failure_recovery_identity(evidence).is_err()
        || !evidence.acknowledged
    {
        bail!("claim-derived consumed evidence is not a canonical frozen completion");
    }
    Ok(())
}

fn promote_oldest_pending_failure_evidence(layout: &Layout, owner_nonce: &str) -> Result<()> {
    let mut pending = Vec::new();
    for entry in fs::read_dir(layout.active_root()?)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(identity) = name
            .strip_prefix(".failure-evidence.pending-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        let parsed = Uuid::parse_str(identity)
            .context("pending failure evidence filename has invalid identity")?;
        if parsed.to_string() != identity || !entry.file_type()?.is_file() {
            bail!("pending failure evidence must be a canonical regular file");
        }
        let evidence: FailureEvidence = read_json(&entry.path())?;
        if evidence.launcher_owner_nonce.as_deref() != Some(owner_nonce)
            || evidence.recovery_identity.as_deref() != Some(identity)
        {
            bail!("pending failure evidence identity or owner does not match");
        }
        if !evidence.acknowledged {
            pending.push((evidence.occurred_at, identity.to_string(), entry.path()));
        }
    }
    pending.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    if let Some((_, _, path)) = pending.into_iter().next() {
        fs::rename(path, layout.failure_evidence()?)?;
        sync_dir(&layout.active_root()?)?;
    }
    Ok(())
}

fn require_failure_artifact_provenance(
    existing: &FailureEvidence,
    evidence: &FailureEvidence,
) -> Result<()> {
    let failed_hash_compatible = match (
        existing.failed_build_hash.as_deref(),
        evidence.failed_build_hash.as_deref(),
    ) {
        (Some(existing), Some(evidence)) => existing == evidence,
        _ => true,
    };
    if existing.recovery_identity != evidence.recovery_identity
        || existing.occurred_at != evidence.occurred_at
        || existing.transaction_id != evidence.transaction_id
        || existing.request_id != evidence.request_id
        || existing.requested_by_thread_id != evidence.requested_by_thread_id
        || existing.mode != evidence.mode
        || existing.build_id != evidence.build_id
        || existing.source_commit != evidence.source_commit
        || existing.manifest_hash != evidence.manifest_hash
        || existing.app_bundle_path != evidence.app_bundle_path
        || !failed_hash_compatible
    {
        bail!("failure evidence identity has conflicting immutable provenance");
    }
    Ok(())
}

pub fn load_state(layout: &Layout) -> Result<RuntimeState> {
    let state_path = layout.state()?;
    if state_path.exists() {
        let mut state: RuntimeState = read_json(&state_path)?;
        for build in [&mut state.current, &mut state.previous]
            .into_iter()
            .flatten()
        {
            let fabricated_legacy_provenance = build.transaction_id.as_deref() == Some("legacy")
                && build.request_id.as_deref() == Some("legacy");
            let installed_baseline_provenance = build.build_id == "installed-baseline";
            if fabricated_legacy_provenance || installed_baseline_provenance {
                build.transaction_id = None;
                build.request_id = None;
                build.requested_by_thread_id = None;
            } else {
                if build.transaction_id.as_deref() == Some("") {
                    build.transaction_id = None;
                }
                if build.request_id.as_deref() == Some("") {
                    build.request_id = None;
                }
                if build.requested_by_thread_id.as_deref() == Some("") {
                    build.requested_by_thread_id = None;
                }
            }
            if build.artifact_content_hash.is_empty() {
                build.artifact_content_hash = build.manifest_hash.clone();
            }
        }
        Ok(state)
    } else {
        Ok(RuntimeState::default())
    }
}

pub fn save_state(layout: &Layout, state: &RuntimeState) -> Result<()> {
    write_json_atomically(&layout.state()?, state)
}

pub fn load_and_validate_manifest(
    request: &ActivationRequest,
) -> Result<(PreparedManifest, String, String)> {
    validate_request(request)?;
    let manifest_path = request.prepared_root.join("manifest.json");
    let manifest: PreparedManifest = read_json(&manifest_path)?;
    if manifest.schema_version != SCHEMA_VERSION {
        bail!("unsupported manifest schema {}", manifest.schema_version);
    }
    if manifest.build_id != request.build_id || manifest.source_commit != request.source_commit {
        bail!("request and manifest build identity differ");
    }
    if manifest.artifacts.is_empty() {
        bail!("prepared manifest contains no artifacts");
    }
    let manifest_hash = hex_digest(&serde_json::to_vec(&manifest)?);
    let resources = request.prepared_root.join("resources");
    let canonical_resources = resources
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", resources.display()))?;
    let mut artifact_paths = BTreeSet::new();
    for artifact in &manifest.artifacts {
        validate_relative_path(&artifact.relative_path)?;
        if artifact.kind == "launcher"
            || artifact
                .relative_path
                .file_name()
                .is_some_and(|name| name == OsStr::new("MorpheusLauncher"))
        {
            bail!("runtime updates must not replace Contents/MacOS/MorpheusLauncher");
        }
        if !artifact_paths.insert(artifact.relative_path.clone()) {
            bail!(
                "duplicate artifact path {}",
                artifact.relative_path.display()
            );
        }
        if artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("invalid SHA-256 for {}", artifact.relative_path.display());
        }
        let source = resources.join(&artifact.relative_path);
        let canonical_source = source
            .canonicalize()
            .with_context(|| format!("failed to resolve {}", source.display()))?;
        if !canonical_source.starts_with(&canonical_resources) || !canonical_source.is_file() {
            bail!("artifact escapes resources root: {}", source.display());
        }
        let actual = hash_file(&canonical_source)?;
        if !actual.eq_ignore_ascii_case(&artifact.sha256) {
            bail!(
                "SHA-256 mismatch for {}: expected {}, got {}",
                artifact.relative_path.display(),
                artifact.sha256,
                actual
            );
        }
    }
    let artifact_content_hash = artifact_content_hash(&manifest);
    Ok((manifest, manifest_hash, artifact_content_hash))
}

pub fn artifact_content_hash(manifest: &PreparedManifest) -> String {
    let mut artifacts = manifest.artifacts.iter().collect::<Vec<_>>();
    artifacts.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    let mut hasher = Sha256::new();
    for artifact in artifacts {
        hasher.update(artifact.relative_path.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(artifact.sha256.to_ascii_lowercase().as_bytes());
        hasher.update([0]);
        hasher.update([0xff]);
    }
    format!("{:x}", hasher.finalize())
}

pub fn manifest_hash(manifest: &PreparedManifest) -> Result<String> {
    Ok(hex_digest(&serde_json::to_vec(manifest)?))
}

pub fn validate_request(request: &ActivationRequest) -> Result<()> {
    if request.schema_version != SCHEMA_VERSION {
        bail!("unsupported request schema {}", request.schema_version);
    }
    for (name, value) in [
        ("transactionId", request.transaction_id.as_str()),
        ("requestId", request.request_id.as_str()),
        ("buildId", request.build_id.as_str()),
        ("sourceCommit", request.source_commit.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!("{name} must not be empty");
        }
    }
    if request
        .requested_by_thread_id
        .as_deref()
        .is_some_and(|value| value.trim().is_empty())
    {
        bail!("requestedByThreadId must be non-empty when provided");
    }
    if !request.prepared_root.is_absolute() || !request.app_bundle_path.is_absolute() {
        bail!("preparedRoot and appBundlePath must be absolute");
    }
    if request.app_bundle_path.parent().is_none() || request.app_bundle_path.file_name().is_none() {
        bail!("appBundlePath must identify an app bundle below a parent directory");
    }
    if !request
        .transaction_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        bail!("transactionId contains unsafe path characters");
    }
    validate_relative_path(Path::new(&request.transaction_id))
        .context("transactionId is not a safe staging child name")?;
    if Path::new(&request.transaction_id).components().count() != 1 {
        bail!("transactionId must be exactly one path component");
    }
    Ok(())
}

pub fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("artifact path must be non-empty and relative");
    }
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        bail!("artifact path contains traversal: {}", path.display());
    }
    Ok(())
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn stage_candidate(
    layout: &Layout,
    request: &ActivationRequest,
    manifest: &PreparedManifest,
) -> Result<PathBuf> {
    validate_request(request)?;
    #[cfg(unix)]
    {
        let staging = TrustedStaging::open(layout, true)?;
        let claim_name = claimed_stage_name(&request.transaction_id);
        if staging.child_exists(&request.transaction_id)?
            || staging.root_child_exists(&claim_name)?
        {
            bail!(
                "staging or claimed child already exists for transaction {}",
                request.transaction_id
            );
        }
        let temp_name = format!(".prepare-{}", Uuid::new_v4());
        let temp = staging.create_private_child(&temp_name)?;
        write_owned_marker(
            &temp.file,
            &request.transaction_id,
            "prepared",
            staging.root_owner_nonce(),
        )?;
        let installed_resources =
            open_directory_path_no_follow(&request.app_bundle_path.join("Contents/Resources"))?;
        let staged_resources =
            open_or_create_directory_at(temp.file.as_raw_fd(), OsStr::new("resources"))?;
        copy_directory_handles(&installed_resources, &staged_resources)?;
        let prepared_resources =
            open_directory_path_no_follow(&request.prepared_root.join("resources"))?;
        for artifact in &manifest.artifacts {
            let source =
                open_relative_file_no_follow(&prepared_resources, &artifact.relative_path)?;
            replace_relative_file_from_handle(
                &staged_resources,
                &artifact.relative_path,
                &source,
                artifact_is_executable(&artifact.kind),
            )?;
        }
        write_json_at(&temp.file, OsStr::new("manifest.json"), manifest)?;
        validate_staged_candidate_handle(
            &temp.file,
            &request.build_id,
            &request.source_commit,
            &manifest_hash(manifest)?,
            &artifact_content_hash(manifest),
        )?;
        staging.publish_child(&temp_name, &request.transaction_id, &temp.file)?;
        drop(temp);
        gc_staging(layout, Some(&request.transaction_id), 4)?;
        resolve_staging_child(layout, &request.transaction_id, true)
    }
    #[cfg(not(unix))]
    {
        let _ = (layout, manifest);
        bail!("transactional runtime staging is only supported on Unix")
    }
}

fn rotate_candidate_durably(
    layout: &Layout,
    transaction: &mut Transaction,
    stage: &Path,
    claimed_stage: Option<(&TrustedStaging, &TrustedChild, &str)>,
) -> Result<()> {
    let persisted_retired = transaction
        .slot_retired_path
        .clone()
        .context("activation transaction has no retired previous slot path")?;
    let retired = layout.active_root()?.join(
        persisted_retired
            .file_name()
            .context("retired previous slot path has no file name")?,
    );

    if transaction.slot_rotation_phase == SlotRotationPhase::NotStarted {
        let inferred_had_previous = layout.previous()?.exists() || retired.exists();
        let had_previous = transaction
            .slot_had_previous
            .unwrap_or(inferred_had_previous);
        if had_previous && layout.previous()?.exists() && !retired.exists() {
            fs::rename(layout.previous()?, &retired)?;
            sync_dir(&layout.active_root()?)?;
        }
        if layout.previous()?.exists() || (had_previous && !retired.exists()) {
            bail!("previous slot retirement is in an ambiguous filesystem state");
        }
        transaction.slot_rotation_phase = SlotRotationPhase::PreviousRetired;
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    if transaction.slot_rotation_phase == SlotRotationPhase::PreviousRetired {
        if layout.current()?.exists() && !layout.previous()?.exists() {
            fs::rename(layout.current()?, layout.previous()?)?;
            sync_dir(&layout.active_root()?)?;
        }
        if layout.current()?.exists() || !layout.previous()?.exists() {
            bail!("current slot promotion is in an ambiguous filesystem state");
        }
        transaction.slot_rotation_phase = SlotRotationPhase::CurrentPromoted;
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    if transaction.slot_rotation_phase == SlotRotationPhase::CurrentPromoted {
        if stage.exists() && !layout.current()?.exists() {
            if let Some((authority, claimed_stage, claimed_name)) = claimed_stage {
                install_claimed_stage_as_current(authority, claimed_name, claimed_stage)?;
            } else {
                fs::rename(stage, layout.current()?)?;
            }
            sync_dir(&layout.active_root()?)?;
            sync_dir(
                stage
                    .parent()
                    .context("candidate stage has no parent directory")?,
            )?;
        }
        let current_manifest: PreparedManifest =
            read_json(&layout.current()?.join("manifest.json"))
                .context("candidate slot installation is incomplete")?;
        if current_manifest.build_id != transaction.request.build_id || stage.exists() {
            bail!("candidate slot installation does not match the transaction");
        }
        transaction.slot_rotation_phase = SlotRotationPhase::CandidateInstalled;
        transaction.slot_rotated = true;
        write_json_atomically(&layout.transaction()?, transaction)?;
    }
    Ok(())
}

fn restore_activation_slots_durably(layout: &Layout, transaction: &mut Transaction) -> Result<()> {
    let retired_stored = transaction.slot_retired_path.clone().unwrap_or_else(|| {
        layout.root.join(format!(
            ".slot-previous-retired-{}",
            transaction.request.transaction_id
        ))
    });
    let failed_stored = transaction.slot_failed_path.clone().unwrap_or_else(|| {
        layout.root.join(format!(
            ".slot-candidate-failed-{}",
            transaction.request.transaction_id
        ))
    });
    let retired = layout.active_root()?.join(
        retired_stored
            .file_name()
            .context("retired previous slot path has no file name")?,
    );
    let failed = layout.active_root()?.join(
        failed_stored
            .file_name()
            .context("failed candidate slot path has no file name")?,
    );
    if transaction.slot_retired_path.is_none() || transaction.slot_failed_path.is_none() {
        transaction.slot_retired_path = Some(retired_stored);
        transaction.slot_failed_path = Some(failed_stored);
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    let current_is_candidate = layout.current()?.exists()
        && read_json::<PreparedManifest>(&layout.current()?.join("manifest.json"))
            .is_ok_and(|manifest| manifest.build_id == transaction.request.build_id);
    if current_is_candidate && !failed.exists() {
        fs::rename(layout.current()?, &failed)?;
        sync_dir(&layout.active_root()?)?;
    }
    if !current_is_candidate || failed.exists() {
        transaction.slot_restore_phase = SlotRestorePhase::CandidateRetired;
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    if !layout.current()?.exists() && layout.previous()?.exists() {
        fs::rename(layout.previous()?, layout.current()?)?;
        sync_dir(&layout.active_root()?)?;
    }
    if !layout.current()?.exists() {
        bail!("activation rollback could not restore the previous current slot");
    }
    transaction.slot_restore_phase = SlotRestorePhase::PreviousRestored;
    write_json_atomically(&layout.transaction()?, transaction)?;

    if !layout.previous()?.exists() && retired.exists() {
        fs::rename(&retired, layout.previous()?)?;
        sync_dir(&layout.active_root()?)?;
    }
    if retired.exists() {
        bail!("activation rollback could not restore the retired previous slot");
    }
    transaction.slot_restore_phase = SlotRestorePhase::RetiredRestored;
    write_json_atomically(&layout.transaction()?, transaction)?;
    Ok(())
}

pub fn capture_installed_slot(
    layout: &Layout,
    candidate_slot: &Path,
    app_bundle: &Path,
) -> Result<Option<BuildRecord>> {
    if layout.current()?.exists() {
        return Ok(None);
    }
    let candidate: PreparedManifest = read_json(&candidate_slot.join("manifest.json"))?;
    let canonical_app_bundle = app_bundle
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", app_bundle.display()))?;
    fs::create_dir_all(layout.current()?.join("resources"))?;
    let mut baseline_artifacts = Vec::with_capacity(candidate.artifacts.len());
    for artifact in candidate.artifacts {
        let installed = app_bundle
            .join("Contents/Resources")
            .join(&artifact.relative_path);
        let canonical_installed = installed
            .canonicalize()
            .with_context(|| format!("failed to resolve {}", installed.display()))?;
        if !canonical_installed.starts_with(&canonical_app_bundle) || !canonical_installed.is_file()
        {
            fs::remove_dir_all(layout.current()?)?;
            bail!(
                "cannot capture installed baseline; {} is missing or escapes the app bundle",
                installed.display()
            );
        }
        let destination = layout
            .current()?
            .join("resources")
            .join(&artifact.relative_path);
        copy_regular_file(
            &canonical_installed,
            &destination,
            artifact_is_executable(&artifact.kind),
        )?;
        baseline_artifacts.push(PreparedArtifact {
            relative_path: artifact.relative_path,
            sha256: hash_file(&canonical_installed)?,
            kind: artifact.kind,
        });
    }
    let baseline_manifest = PreparedManifest {
        schema_version: SCHEMA_VERSION,
        build_id: "installed-baseline".into(),
        source_commit: "unknown".into(),
        artifacts: baseline_artifacts,
        changes: ManifestChanges::default(),
    };
    write_json_atomically(&layout.current()?.join("manifest.json"), &baseline_manifest)?;
    let manifest_hash = hex_digest(&serde_json::to_vec(&baseline_manifest)?);
    let artifact_content_hash = artifact_content_hash(&baseline_manifest);
    Ok(Some(BuildRecord {
        transaction_id: None,
        request_id: None,
        requested_by_thread_id: None,
        mode: ActivationMode::Full,
        build_id: baseline_manifest.build_id,
        source_commit: baseline_manifest.source_commit,
        manifest_hash,
        artifact_content_hash,
        app_bundle_path: app_bundle.to_path_buf(),
        activated_at: Utc::now(),
    }))
}

pub fn bootstrap_current_slot(layout: &Layout, app_bundle: &Path) -> Result<Option<BuildRecord>> {
    if layout.current()?.exists() {
        return Ok(None);
    }
    let canonical_app_bundle = app_bundle
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", app_bundle.display()))?;
    let resources = canonical_app_bundle.join("Contents/Resources");
    if !resources.is_dir() {
        bail!(
            "app resources directory is missing: {}",
            resources.display()
        );
    }
    let mut files = Vec::new();
    collect_regular_files(&resources, &mut files)?;
    files.sort();

    fs::create_dir_all(layout.current()?.join("resources"))?;
    let mut artifacts = Vec::with_capacity(files.len());
    for source in files {
        let relative_path = source
            .strip_prefix(&resources)
            .context("installed baseline path escaped app bundle")?
            .to_path_buf();
        let kind = "file";
        let destination = layout.current()?.join("resources").join(&relative_path);
        copy_regular_file(&source, &destination, artifact_is_executable(kind))?;
        artifacts.push(PreparedArtifact {
            relative_path,
            sha256: hash_file(&source)?,
            kind: kind.into(),
        });
    }
    let manifest = PreparedManifest {
        schema_version: SCHEMA_VERSION,
        build_id: "installed-baseline".into(),
        source_commit: "unknown".into(),
        artifacts,
        changes: ManifestChanges::default(),
    };
    write_json_atomically(&layout.current()?.join("manifest.json"), &manifest)?;
    let manifest_hash = hex_digest(&serde_json::to_vec(&manifest)?);
    let artifact_content_hash = artifact_content_hash(&manifest);
    Ok(Some(BuildRecord {
        transaction_id: None,
        request_id: None,
        requested_by_thread_id: None,
        mode: ActivationMode::Full,
        build_id: manifest.build_id,
        source_commit: manifest.source_commit,
        manifest_hash,
        artifact_content_hash,
        app_bundle_path: canonical_app_bundle,
        activated_at: Utc::now(),
    }))
}

fn collect_regular_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            bail!(
                "installed resources contain unsupported symlink: {}",
                entry.path().display()
            );
        }
        if file_type.is_dir() {
            collect_regular_files(&entry.path(), files)?;
        } else if file_type.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_dir_all(destination)?;
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)
        .with_context(|| format!("failed to read directory {}", source.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_symlink() {
            bail!(
                "resource symlinks are unsupported: {}",
                entry.path().display()
            );
        }
        if file_type.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if file_type.is_file() {
            copy_regular_file(&entry.path(), &target, false)?;
        }
    }
    sync_dir(destination)?;
    Ok(())
}

pub fn sign_app_bundle_with(
    runner: &dyn CommandRunner,
    app_bundle: &Path,
    manifest: &PreparedManifest,
) -> Result<()> {
    sign_app_bundle_without_verify_with(runner, app_bundle, manifest)?;
    verify_app_bundle_with(runner, app_bundle)
}

pub fn sign_app_bundle_without_verify_with(
    runner: &dyn CommandRunner,
    app_bundle: &Path,
    manifest: &PreparedManifest,
) -> Result<()> {
    for artifact in &manifest.artifacts {
        if artifact_is_executable(&artifact.kind) {
            let executable = app_bundle
                .join("Contents/Resources")
                .join(&artifact.relative_path);
            let status = runner.run(
                "codesign",
                &[
                    OsStr::new("--force"),
                    OsStr::new("--sign"),
                    OsStr::new("-"),
                    executable.as_os_str(),
                ],
            )?;
            if !status.success() {
                bail!("codesign failed for {} with {status}", executable.display());
            }
        }
    }
    let status = runner.run(
        "codesign",
        &[
            OsStr::new("--force"),
            OsStr::new("--sign"),
            OsStr::new("-"),
            app_bundle.as_os_str(),
        ],
    )?;
    if !status.success() {
        bail!("top-level codesign failed with {status}");
    }
    Ok(())
}

pub fn verify_app_bundle_with(runner: &dyn CommandRunner, app_bundle: &Path) -> Result<()> {
    let verify_status = runner.run(
        "codesign",
        &[
            OsStr::new("--verify"),
            OsStr::new("--strict"),
            app_bundle.as_os_str(),
        ],
    )?;
    if !verify_status.success() {
        bail!("codesign verification failed with {verify_status}");
    }
    Ok(())
}

pub fn activate_slot(
    layout: &Layout,
    transaction: &mut Transaction,
    runner: &dyn CommandRunner,
) -> Result<()> {
    bind_transaction_paths(layout, transaction)?;
    let (authority, claimed) = claim_staging_child(layout, &transaction.request.transaction_id)?;
    validate_staged_candidate_handle(
        &claimed.file,
        &transaction.request.build_id,
        &transaction.request.source_commit,
        &transaction.manifest_hash,
        &transaction.artifact_content_hash,
    )?;
    let snapshot_name = format!(
        ".activation-snapshot-{}",
        transaction.request.transaction_id
    );
    if authority.root_child_exists(&snapshot_name)? {
        authority.remove_root_child(&snapshot_name)?;
    }
    let snapshot = authority.create_private_root_child(&snapshot_name)?;
    write_owned_marker(
        &snapshot.file,
        &transaction.request.transaction_id,
        "snapshot",
        authority.root_owner_nonce(),
    )?;
    copy_directory_handles(&claimed.file, &snapshot.file)?;
    validate_staged_candidate_handle(
        &snapshot.file,
        &transaction.request.build_id,
        &transaction.request.source_commit,
        &transaction.manifest_hash,
        &transaction.artifact_content_hash,
    )?;
    let active_layout = Layout::new(directory_fd_path(&authority.root)?);
    let launcher = transaction
        .request
        .app_bundle_path
        .join("Contents/MacOS/MorpheusLauncher");
    let launcher_hash = transaction
        .launcher_expected_hash
        .clone()
        .context("activation transaction has no fixed MorpheusLauncher hash")?;
    transaction.phase = TransactionPhase::Activating;
    transaction.slot_rotated = false;
    transaction.slot_rotation_phase = SlotRotationPhase::NotStarted;
    transaction.slot_restore_phase = SlotRestorePhase::NotStarted;
    transaction.slot_retired_path = Some(layout.root.join(format!(
        ".slot-previous-retired-{}",
        transaction.request.transaction_id
    )));
    transaction.slot_failed_path = Some(layout.root.join(format!(
        ".slot-candidate-failed-{}",
        transaction.request.transaction_id
    )));
    transaction.slot_had_previous = Some(active_layout.previous()?.exists());
    write_json_atomically(&active_layout.transaction()?, transaction)?;
    rotate_candidate_durably(
        &active_layout,
        transaction,
        &snapshot.path,
        Some((&authority, &snapshot, &snapshot_name)),
    )?;
    authority.remove_root_child(&claimed_stage_name(&transaction.request.transaction_id))?;

    let contents = transaction.request.app_bundle_path.join("Contents");
    let bundle_parent = transaction
        .request
        .app_bundle_path
        .parent()
        .with_context(|| {
            format!(
                "{} has no parent",
                transaction.request.app_bundle_path.display()
            )
        })?
        .to_path_buf();
    let resources = contents.join("Resources");
    let replacement = contents.join(format!(
        ".MorpheusResourcesCandidate-{}",
        transaction.request.transaction_id
    ));
    let resources_backup = bundle_parent.join(format!(
        ".MorpheusResourcesPrevious-{}",
        transaction.request.transaction_id
    ));
    let resources_retired = bundle_parent.join(format!(
        ".MorpheusResourcesFailed-{}",
        transaction.request.transaction_id
    ));
    let signature_backup = bundle_parent.join(format!(
        ".MorpheusSignaturePrevious-{}",
        transaction.request.transaction_id
    ));
    let code_signature = contents.join("_CodeSignature");
    transaction.resources_backup_path = Some(resources_backup.clone());
    transaction.resources_replacement_path = Some(replacement.clone());
    transaction.resources_retired_path = Some(resources_retired);
    transaction.resource_swap_phase = ResourceSwapPhase::NotStarted;
    transaction.resource_restore_phase = ResourceRestorePhase::NotStarted;
    transaction.signature_backup_path = Some(signature_backup.clone());
    transaction.signature_backup_ready = Some(false);
    transaction.signature_backup_had_code_signature = Some(code_signature.exists());
    transaction.signature_restore_phase = SignatureRestorePhase::NotStarted;
    transaction.runtime_retired_path = Some(bundle_parent.join(format!(
        ".MorpheusRuntimeFailed-{}",
        transaction.request.transaction_id
    )));
    transaction.runtime_replacement_path = Some(contents.join(format!(
        "MacOS/.Root Worker Runtime.restore-{}",
        transaction.request.transaction_id
    )));
    transaction.signature_retired_path = Some(bundle_parent.join(format!(
        ".MorpheusCodeSignatureFailed-{}",
        transaction.request.transaction_id
    )));
    transaction.signature_replacement_path = Some(contents.join(format!(
        ".MorpheusCodeSignatureRestore-{}",
        transaction.request.transaction_id
    )));
    transaction.signature_replacement_ready = Some(false);
    write_json_atomically(&active_layout.transaction()?, transaction)?;

    copy_directory(&active_layout.current()?.join("resources"), &replacement)?;
    transaction.resource_swap_phase = ResourceSwapPhase::ReplacementPrepared;
    write_json_atomically(&active_layout.transaction()?, transaction)?;
    if signature_backup.exists() {
        fs::remove_dir_all(&signature_backup)?;
    }
    fs::create_dir_all(&signature_backup)?;
    if code_signature.exists() {
        copy_directory(&code_signature, &signature_backup.join("_CodeSignature"))?;
    }
    let runtime_executable = contents.join("MacOS/Root Worker Runtime");
    copy_regular_file(
        &runtime_executable,
        &signature_backup.join("Root Worker Runtime"),
        true,
    )?;
    transaction.signature_backup_ready = Some(true);
    write_json_atomically(&active_layout.transaction()?, transaction)?;

    fs::rename(&resources, &resources_backup)?;
    sync_dir(&contents)?;
    sync_dir(&bundle_parent)?;
    transaction.resource_swap_phase = ResourceSwapPhase::OriginalBackedUp;
    write_json_atomically(&active_layout.transaction()?, transaction)?;
    fs::rename(&replacement, &resources)?;
    sync_dir(&contents)?;
    transaction.resource_swap_phase = ResourceSwapPhase::CandidateInstalled;
    transaction.resources_swapped = true;
    write_json_atomically(&active_layout.transaction()?, transaction)?;
    let manifest: PreparedManifest = read_json(&active_layout.current()?.join("manifest.json"))?;
    sign_app_bundle_with(runner, &transaction.request.app_bundle_path, &manifest)?;
    if hash_file(&launcher)? != launcher_hash {
        bail!("runtime activation changed the stable MorpheusLauncher executable");
    }
    transaction.phase = TransactionPhase::CandidateInstalled;
    write_json_atomically(&active_layout.transaction()?, transaction)?;
    Ok(())
}

pub fn rollback_activation(layout: &Layout, transaction: &mut Transaction) -> Result<()> {
    bind_transaction_paths(layout, transaction)?;
    transaction.phase = TransactionPhase::RollingBack;
    write_json_atomically(&layout.transaction()?, transaction)?;
    let contents = transaction.request.app_bundle_path.join("Contents");
    let resources = contents.join("Resources");
    restore_activation_resources_durably(layout, transaction, &resources)?;
    restore_signature_artifacts_durably(layout, transaction, &contents)?;
    restore_activation_slots_durably(layout, transaction)?;
    sync_dir(&contents)?;
    Ok(())
}

pub fn rollback_activation_with(
    layout: &Layout,
    transaction: &mut Transaction,
    runner: &dyn CommandRunner,
) -> Result<()> {
    bind_transaction_paths(layout, transaction)?;
    let launcher = transaction
        .request
        .app_bundle_path
        .join("Contents/MacOS/MorpheusLauncher");
    ensure_launcher_expected_hash(layout, transaction)?;
    let expected_launcher_hash = transaction
        .launcher_expected_hash
        .clone()
        .context("rollback transaction has no fixed MorpheusLauncher hash")?;
    rollback_activation(layout, transaction)?;
    verify_app_bundle_with(runner, &transaction.request.app_bundle_path)?;
    if hash_file(&launcher)? != expected_launcher_hash {
        bail!("rollback did not restore the stable MorpheusLauncher executable");
    }
    transaction.phase = TransactionPhase::RollbackComplete;
    write_json_atomically(&layout.transaction()?, transaction)?;
    commit_activation_files(layout, transaction)
}

fn ensure_launcher_expected_hash(layout: &Layout, transaction: &mut Transaction) -> Result<()> {
    if transaction.launcher_expected_hash.is_some() {
        return Ok(());
    }
    let backup_launcher = transaction
        .signature_backup_path
        .as_ref()
        .map(|backup| backup.join("MacOS/MorpheusLauncher"))
        .filter(|path| path.is_file())
        .context("legacy activation transaction has no complete trusted launcher backup")?;
    let backup_runtime = transaction
        .signature_backup_path
        .as_ref()
        .map(|backup| backup.join("MacOS/Root Worker Runtime"))
        .filter(|path| path.is_file())
        .context("legacy activation transaction has no complete trusted runtime backup")?;
    let _ = backup_runtime;
    transaction.launcher_expected_hash = Some(hash_file(&backup_launcher)?);
    write_json_atomically(&layout.transaction()?, transaction)
}

fn restore_signature_artifacts_durably(
    layout: &Layout,
    transaction: &mut Transaction,
    contents: &Path,
) -> Result<()> {
    let Some(backup) = transaction.signature_backup_path.clone() else {
        return Ok(());
    };
    let backup_runtime = if backup.join("Root Worker Runtime").is_file() {
        backup.join("Root Worker Runtime")
    } else {
        backup.join("MacOS/Root Worker Runtime")
    };
    let backup_ready = transaction.signature_backup_ready == Some(true)
        || (transaction.signature_backup_ready.is_none()
            && backup_runtime.is_file()
            && backup.join("MacOS/MorpheusLauncher").is_file());
    if !backup_ready {
        return Ok(());
    }
    if !backup_runtime.is_file() {
        bail!("runtime executable rollback backup is incomplete");
    }
    let runtime = contents.join("MacOS/Root Worker Runtime");
    let runtime_replacement = transaction
        .runtime_replacement_path
        .clone()
        .unwrap_or_else(|| {
            contents.join(format!(
                "MacOS/.Root Worker Runtime.restore-{}",
                transaction.request.transaction_id
            ))
        });
    let runtime_retired = transaction.runtime_retired_path.clone().unwrap_or_else(|| {
        transaction
            .request
            .app_bundle_path
            .parent()
            .unwrap_or(&layout.root)
            .join(format!(
                ".MorpheusRuntimeFailed-{}",
                transaction.request.transaction_id
            ))
    });
    let signature = contents.join("_CodeSignature");
    let signature_retired = transaction
        .signature_retired_path
        .clone()
        .unwrap_or_else(|| {
            transaction
                .request
                .app_bundle_path
                .parent()
                .unwrap_or(&layout.root)
                .join(format!(
                    ".MorpheusCodeSignatureFailed-{}",
                    transaction.request.transaction_id
                ))
        });
    if transaction.runtime_retired_path.is_none()
        || transaction.runtime_replacement_path.is_none()
        || transaction.signature_retired_path.is_none()
    {
        transaction.runtime_retired_path = Some(runtime_retired.clone());
        transaction.runtime_replacement_path = Some(runtime_replacement.clone());
        transaction.signature_retired_path = Some(signature_retired.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    if transaction.signature_restore_phase == SignatureRestorePhase::NotStarted {
        if runtime.exists() && !runtime_retired.exists() {
            fs::rename(&runtime, &runtime_retired)?;
            sync_dir(
                runtime
                    .parent()
                    .context("runtime executable has no parent")?,
            )?;
            sync_dir(
                runtime_retired
                    .parent()
                    .context("retired runtime executable has no parent")?,
            )?;
        }
        if runtime.exists() && runtime_retired.exists() {
            if hash_file(&runtime)? == hash_file(&backup_runtime)? {
                transaction.signature_restore_phase = SignatureRestorePhase::RuntimeRestored;
            } else {
                bail!("runtime executable retirement is ambiguous");
            }
        } else if !runtime.exists() && runtime_retired.exists() {
            transaction.signature_restore_phase = SignatureRestorePhase::RuntimeRetired;
        } else {
            bail!("runtime executable retirement is incomplete");
        }
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    if transaction.signature_restore_phase == SignatureRestorePhase::RuntimeRetired {
        if !runtime.exists() && backup_runtime.exists() {
            if runtime_replacement.exists() {
                fs::remove_file(&runtime_replacement)?;
            }
            fs::copy(&backup_runtime, &runtime_replacement)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&runtime_replacement, fs::Permissions::from_mode(0o755))?;
            }
            File::open(&runtime_replacement)?.sync_all()?;
            fs::rename(&runtime_replacement, &runtime)?;
            sync_dir(
                runtime
                    .parent()
                    .context("runtime executable has no parent")?,
            )?;
        }
        if !runtime.exists()
            || !backup_runtime.exists()
            || hash_file(&runtime)? != hash_file(&backup_runtime)?
        {
            bail!("runtime executable restoration is incomplete");
        }
        transaction.signature_restore_phase = SignatureRestorePhase::RuntimeRestored;
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    let backup_signature = backup.join("_CodeSignature");
    let signature_replacement = transaction
        .signature_replacement_path
        .clone()
        .unwrap_or_else(|| {
            contents.join(format!(
                ".MorpheusCodeSignatureRestore-{}",
                transaction.request.transaction_id
            ))
        });
    if transaction.signature_replacement_path.is_none() {
        transaction.signature_replacement_path = Some(signature_replacement.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }
    let had_signature = transaction
        .signature_backup_had_code_signature
        .unwrap_or_else(|| backup_signature.exists());
    if transaction.signature_restore_phase == SignatureRestorePhase::RuntimeRestored {
        if signature.exists() && !signature_retired.exists() {
            fs::rename(&signature, &signature_retired)?;
            sync_dir(contents)?;
            sync_dir(
                signature_retired
                    .parent()
                    .context("retired code signature has no parent")?,
            )?;
        }
        if signature.exists() && signature_retired.exists() && had_signature {
            transaction.signature_restore_phase = SignatureRestorePhase::SignatureRestored;
        } else if !signature.exists() && (signature_retired.exists() || !had_signature) {
            transaction.signature_restore_phase = SignatureRestorePhase::SignatureRetired;
        } else {
            bail!("code signature retirement is incomplete");
        }
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    if transaction.signature_restore_phase == SignatureRestorePhase::SignatureRetired {
        if had_signature && !signature.exists() {
            if transaction.signature_replacement_ready != Some(true) {
                if signature_replacement.exists() {
                    fs::remove_dir_all(&signature_replacement)?;
                }
                copy_directory(&backup_signature, &signature_replacement)?;
                transaction.signature_replacement_ready = Some(true);
                write_json_atomically(&layout.transaction()?, transaction)?;
            }
            fs::rename(&signature_replacement, &signature)?;
            sync_dir(contents)?;
        }
        if (had_signature && !signature.exists()) || (!had_signature && signature.exists()) {
            bail!("code signature restoration is incomplete");
        }
        transaction.signature_restore_phase = SignatureRestorePhase::SignatureRestored;
        write_json_atomically(&layout.transaction()?, transaction)?;
    }
    Ok(())
}

fn restore_activation_resources_durably(
    layout: &Layout,
    transaction: &mut Transaction,
    resources: &Path,
) -> Result<()> {
    let backup = match transaction.resources_backup_path.clone() {
        Some(path) => path,
        None => {
            if resources.exists() {
                return Ok(());
            }
            bail!("installed Resources is missing and no rollback backup was recorded");
        }
    };
    let retired = transaction
        .resources_retired_path
        .clone()
        .unwrap_or_else(|| {
            transaction
                .request
                .app_bundle_path
                .parent()
                .unwrap_or(&layout.root)
                .join(format!(
                    ".MorpheusResourcesFailed-{}",
                    transaction.request.transaction_id
                ))
        });
    if transaction.resources_retired_path.is_none() {
        transaction.resources_retired_path = Some(retired.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    if !backup.exists() {
        if resources.exists()
            && matches!(
                transaction.resource_swap_phase,
                ResourceSwapPhase::NotStarted | ResourceSwapPhase::ReplacementPrepared
            )
        {
            return Ok(());
        }
        if resources.exists()
            && matches!(
                transaction.resource_restore_phase,
                ResourceRestorePhase::DestinationRetired | ResourceRestorePhase::BackupRestored
            )
        {
            transaction.resource_restore_phase = ResourceRestorePhase::BackupRestored;
            write_json_atomically(&layout.transaction()?, transaction)?;
            return Ok(());
        }
        bail!(
            "rollback backup is missing while Resources restoration is incomplete: {}",
            backup.display()
        );
    }

    if resources.exists() && !retired.exists() {
        fs::rename(resources, &retired)?;
        sync_dir(
            resources
                .parent()
                .context("installed Resources has no Contents parent")?,
        )?;
        sync_dir(
            retired
                .parent()
                .context("retired Resources has no parent")?,
        )?;
        transaction.resource_restore_phase = ResourceRestorePhase::DestinationRetired;
        write_json_atomically(&layout.transaction()?, transaction)?;
    } else if !resources.exists() {
        transaction.resource_restore_phase = ResourceRestorePhase::DestinationRetired;
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    if !resources.exists() && backup.exists() {
        fs::rename(&backup, resources)?;
        sync_dir(
            resources
                .parent()
                .context("installed Resources has no Contents parent")?,
        )?;
        sync_dir(backup.parent().context("Resources backup has no parent")?)?;
    }
    if !resources.exists() || backup.exists() {
        bail!("Resources rollback did not reach a complete filesystem state");
    }
    transaction.resource_restore_phase = ResourceRestorePhase::BackupRestored;
    write_json_atomically(&layout.transaction()?, transaction)?;
    Ok(())
}

pub fn begin_post_ready_rollback(
    layout: &Layout,
    failed_build: BuildRecord,
    fallback_build: BuildRecord,
    mut failure_evidence: FailureEvidence,
) -> Result<Transaction> {
    #[cfg(unix)]
    let _trusted_root = TrustedStaging::open(layout, true)?;
    let superseded_transaction = if layout.transaction()?.exists() {
        let active: Transaction = read_json(&layout.transaction()?)?;
        if active.phase != TransactionPhase::Prepared {
            bail!(
                "cannot begin post-ready rollback while transaction {} is active in phase {:?}",
                active.request.transaction_id,
                active.phase
            );
        }
        Some(SupersededTransaction {
            transaction_id: active.request.transaction_id.clone(),
            build_id: active.request.build_id.clone(),
            reason: "superseded by automatic post-ready crash rollback".into(),
        })
    } else {
        None
    };
    if let Some(superseded) = &superseded_transaction {
        failure_evidence.summary = format!(
            "{}; superseded prepared transaction {} for build {}: {}",
            failure_evidence.summary,
            superseded.transaction_id,
            superseded.build_id,
            superseded.reason
        );
    }
    let app_bundle_path = failed_build.app_bundle_path.clone();
    let transaction_id = format!("post-ready-{}", Uuid::new_v4());
    let request_id = failed_build
        .request_id
        .clone()
        .unwrap_or_else(|| format!("post-ready-rollback-{transaction_id}"));
    let bundle_parent = app_bundle_path
        .parent()
        .map(Path::to_path_buf)
        .with_context(|| format!("{} has no parent", app_bundle_path.display()))?;
    let contents = app_bundle_path.join("Contents");
    let request = ActivationRequest {
        schema_version: SCHEMA_VERSION,
        transaction_id: transaction_id.clone(),
        request_id,
        requested_by_thread_id: failed_build.requested_by_thread_id.clone(),
        mode: failed_build.mode,
        build_id: failed_build.build_id.clone(),
        source_commit: failed_build.source_commit.clone(),
        prepared_root: layout.root.join("post-ready-recovery"),
        app_bundle_path,
        reason: "automatic post-ready crash rollback".into(),
    };
    let rollback = PostReadyRollback {
        failed_build: failed_build.clone(),
        fallback_build,
        failure_evidence,
        phase: PostReadyRollbackPhase::Planned,
        failed_slot_path: layout
            .root
            .join(format!(".failed-current-{transaction_id}")),
        resources_replacement_path: contents
            .join(format!(".MorpheusResourcesRollback-{transaction_id}")),
        resources_backup_path: bundle_parent
            .join(format!(".MorpheusResourcesFailed-{transaction_id}")),
        slot_restore_phase: SlotRestorePhase::NotStarted,
        superseded_transaction,
    };
    let transaction = Transaction {
        schema_version: SCHEMA_VERSION,
        request,
        manifest_hash: failed_build.manifest_hash.clone(),
        artifact_content_hash: failed_build.artifact_content_hash.clone(),
        phase: TransactionPhase::RollingBack,
        instance_id: None,
        slot_rotated: false,
        slot_rotation_phase: SlotRotationPhase::NotStarted,
        slot_restore_phase: SlotRestorePhase::NotStarted,
        slot_retired_path: None,
        slot_failed_path: None,
        slot_had_previous: None,
        resources_swapped: false,
        resources_backup_path: None,
        resources_replacement_path: None,
        resources_retired_path: None,
        resource_swap_phase: ResourceSwapPhase::NotStarted,
        resource_restore_phase: ResourceRestorePhase::NotStarted,
        signature_backup_path: None,
        signature_backup_ready: Some(false),
        signature_backup_had_code_signature: Some(false),
        signature_restore_phase: SignatureRestorePhase::NotStarted,
        runtime_retired_path: None,
        runtime_replacement_path: None,
        signature_retired_path: None,
        signature_replacement_path: None,
        signature_replacement_ready: Some(false),
        launcher_expected_hash: Some(hash_file(
            &failed_build
                .app_bundle_path
                .join("Contents/MacOS/MorpheusLauncher"),
        )?),
        rollback_failure_evidence: None,
        post_ready_rollback: Some(rollback),
        started_at: Utc::now(),
    };
    write_json_atomically(&layout.transaction()?, &transaction)?;
    if let Some(superseded) = transaction
        .post_ready_rollback
        .as_ref()
        .and_then(|rollback| rollback.superseded_transaction.as_ref())
    {
        remove_staging_child(layout, &superseded.transaction_id)?;
    }
    Ok(transaction)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PostReadyRollbackCheckpoint {
    RestoreSlot,
    InstallResources,
    Sign,
    Verify,
    SaveState,
}

pub fn resume_post_ready_rollback(
    layout: &Layout,
    transaction: &mut Transaction,
    runner: &dyn CommandRunner,
) -> Result<()> {
    resume_post_ready_rollback_with(layout, transaction, runner, |_| Ok(()))
}

pub fn resume_post_ready_rollback_with<F>(
    layout: &Layout,
    transaction: &mut Transaction,
    runner: &dyn CommandRunner,
    mut checkpoint: F,
) -> Result<()>
where
    F: FnMut(PostReadyRollbackCheckpoint) -> Result<()>,
{
    bind_transaction_paths(layout, transaction)?;
    if transaction.phase != TransactionPhase::RollingBack {
        bail!("transaction is not a post-ready rollback");
    }
    let mut rollback = transaction
        .post_ready_rollback
        .clone()
        .context("rolling-back transaction has no post-ready rollback context")?;
    let app_bundle = transaction.request.app_bundle_path.clone();
    let contents = app_bundle.join("Contents");
    let resources = contents.join("Resources");
    if let Some(superseded) = rollback.superseded_transaction.as_ref() {
        remove_staging_child(layout, &superseded.transaction_id)?;
    }

    if rollback.phase == PostReadyRollbackPhase::Planned {
        restore_previous_slot_durably(layout, transaction, &mut rollback)?;
        checkpoint(PostReadyRollbackCheckpoint::RestoreSlot)?;
        rollback.phase = PostReadyRollbackPhase::SlotRestored;
        transaction.post_ready_rollback = Some(rollback.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    if rollback.phase == PostReadyRollbackPhase::SlotRestored {
        install_rollback_resources_durably(
            &layout.current()?.join("resources"),
            &resources,
            &rollback.resources_replacement_path,
            &rollback.resources_backup_path,
        )?;
        checkpoint(PostReadyRollbackCheckpoint::InstallResources)?;
        rollback.phase = PostReadyRollbackPhase::ResourcesInstalled;
        transaction.post_ready_rollback = Some(rollback.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }

    let launcher = contents.join("MacOS/MorpheusLauncher");
    let launcher_hash = transaction
        .launcher_expected_hash
        .clone()
        .context("post-ready rollback has no fixed MorpheusLauncher hash")?;
    let manifest: PreparedManifest = read_json(&layout.current()?.join("manifest.json"))?;
    if rollback.phase == PostReadyRollbackPhase::ResourcesInstalled {
        sign_app_bundle_without_verify_with(runner, &app_bundle, &manifest)?;
        checkpoint(PostReadyRollbackCheckpoint::Sign)?;
        rollback.phase = PostReadyRollbackPhase::Signed;
        transaction.post_ready_rollback = Some(rollback.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }
    if rollback.phase == PostReadyRollbackPhase::Signed {
        verify_app_bundle_with(runner, &app_bundle)?;
        if hash_file(&launcher)? != launcher_hash {
            bail!("post-ready rollback changed the stable MorpheusLauncher executable");
        }
        checkpoint(PostReadyRollbackCheckpoint::Verify)?;
        rollback.phase = PostReadyRollbackPhase::Verified;
        transaction.post_ready_rollback = Some(rollback.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }
    if rollback.phase == PostReadyRollbackPhase::Verified {
        checkpoint(PostReadyRollbackCheckpoint::SaveState)?;
        let mut state = load_state(layout)?;
        state
            .blocked_build_hashes
            .insert(rollback.failed_build.manifest_hash.clone());
        state
            .blocked_build_ids
            .insert(rollback.failed_build.build_id.clone());
        state
            .blocked_artifact_hashes
            .insert(rollback.failed_build.artifact_content_hash.clone());
        state.current = Some(rollback.fallback_build.clone());
        state.previous = None;
        state.crash_history.clear();
        let persisted = persist_failure_evidence(layout, &rollback.failure_evidence)?;
        rollback.failure_evidence = persisted.clone();
        if let Some(existing) = state
            .failures
            .iter_mut()
            .find(|evidence| evidence.recovery_identity == persisted.recovery_identity)
        {
            *existing = persisted;
        } else {
            record_failure(&mut state, persisted);
        }
        save_state(layout, &state)?;
        rollback.phase = PostReadyRollbackPhase::StateCommitted;
        transaction.post_ready_rollback = Some(rollback.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }
    if rollback.phase == PostReadyRollbackPhase::StateCommitted {
        let failed_slot = layout.active_root()?.join(
            rollback
                .failed_slot_path
                .file_name()
                .context("post-ready failed slot path has no file name")?,
        );
        for path in [
            &failed_slot,
            &rollback.resources_replacement_path,
            &rollback.resources_backup_path,
        ] {
            if path.exists() {
                fs::remove_dir_all(path)?;
            }
        }
        fs::remove_file(layout.transaction()?)?;
    }
    Ok(())
}

#[cfg(unix)]
struct TrustedStaging {
    root: File,
    staging: File,
    root_path: PathBuf,
    root_owner_nonce: String,
}

#[cfg(unix)]
struct TrustedChild {
    file: File,
    path: PathBuf,
}

#[cfg(not(unix))]
struct TrustedStaging {
    root: File,
}

#[cfg(not(unix))]
struct TrustedChild {
    file: File,
    path: PathBuf,
}

#[cfg(all(unix, test))]
impl TrustedChild {
    fn anchored_path(&self) -> Result<PathBuf> {
        directory_fd_path(&self.file)
    }
}

#[cfg(all(not(unix), test))]
impl TrustedChild {
    fn anchored_path(&self) -> Result<PathBuf> {
        Ok(self.path.clone())
    }
}

#[cfg(unix)]
impl TrustedStaging {
    fn open(layout: &Layout, create: bool) -> Result<Self> {
        let (root, root_identity, root_path) = if let Some(authority) = &layout.authority {
            (
                authority.root.try_clone()?,
                authority.root_identity.clone(),
                directory_fd_path(&authority.root)?,
            )
        } else {
            let (_root_parent, root, root_identity) = open_trusted_state_root(layout, create)?;
            let root_path = directory_fd_path(&root)?;
            (root, root_identity, root_path)
        };
        if create {
            mkdirat_if_missing(root.as_raw_fd(), OsStr::new("staging"))?;
        }
        let staging = open_directory_at_no_follow(root.as_raw_fd(), OsStr::new("staging"))
            .context("launcher staging must be a real directory under the state root")?;
        set_directory_private(&staging)?;
        Ok(Self {
            root,
            staging,
            root_path,
            root_owner_nonce: root_identity.owner_nonce,
        })
    }

    fn root_owner_nonce(&self) -> &str {
        &self.root_owner_nonce
    }

    fn child_exists(&self, name: &str) -> Result<bool> {
        validate_staging_name(name)?;
        Ok(stat_at_no_follow(self.staging.as_raw_fd(), OsStr::new(name))?.is_some())
    }

    fn root_child_exists(&self, name: &str) -> Result<bool> {
        validate_staging_name(name)?;
        Ok(stat_at_no_follow(self.root.as_raw_fd(), OsStr::new(name))?.is_some())
    }

    fn create_private_child(&self, name: &str) -> Result<TrustedChild> {
        validate_staging_name(name)?;
        let name = OsStr::new(name);
        let c_name = c_string(name)?;
        let result = unsafe { libc::mkdirat(self.staging.as_raw_fd(), c_name.as_ptr(), 0o700) };
        if result != 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "failed to create private staging child {}",
                    name.to_string_lossy()
                )
            });
        }
        let file = open_directory_at_no_follow(self.staging.as_raw_fd(), name)?;
        Ok(TrustedChild {
            file,
            path: self.root_path.join("staging").join(name),
        })
    }

    fn create_private_root_child(&self, name: &str) -> Result<TrustedChild> {
        validate_staging_name(name)?;
        let name = OsStr::new(name);
        let c_name = c_string(name)?;
        let result = unsafe { libc::mkdirat(self.root.as_raw_fd(), c_name.as_ptr(), 0o700) };
        if result != 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "failed to create private root child {}",
                    name.to_string_lossy()
                )
            });
        }
        let file = open_directory_at_no_follow(self.root.as_raw_fd(), name)?;
        Ok(TrustedChild {
            file,
            path: directory_fd_path(&self.root)?.join(name),
        })
    }

    fn open_child(&self, name: &str) -> Result<TrustedChild> {
        validate_staging_name(name)?;
        let name = OsStr::new(name);
        let file = open_directory_at_no_follow(self.staging.as_raw_fd(), name)?;
        Ok(TrustedChild {
            file,
            path: self.root_path.join("staging").join(name),
        })
    }

    fn open_root_child(&self, name: &str) -> Result<TrustedChild> {
        validate_staging_name(name)?;
        let name = OsStr::new(name);
        let file = open_directory_at_no_follow(self.root.as_raw_fd(), name)?;
        Ok(TrustedChild {
            file,
            path: self.root_path.join(name),
        })
    }

    fn publish_child(&self, source: &str, destination: &str, source_handle: &File) -> Result<()> {
        validate_staging_name(source)?;
        validate_staging_name(destination)?;
        if stat_at_no_follow(self.staging.as_raw_fd(), OsStr::new(destination))?.is_some() {
            bail!("published staging child already exists: {destination}");
        }
        verify_entry_matches_handle(self.staging.as_raw_fd(), OsStr::new(source), source_handle)?;
        rename_at_no_replace(
            self.staging.as_raw_fd(),
            OsStr::new(source),
            self.staging.as_raw_fd(),
            OsStr::new(destination),
        )?;
        verify_entry_matches_handle(
            self.staging.as_raw_fd(),
            OsStr::new(destination),
            source_handle,
        )?;
        self.staging.sync_all()?;
        Ok(())
    }

    fn remove_child(&self, name: &str) -> Result<bool> {
        validate_staging_name(name)?;
        self.remove_entry(self.staging.as_raw_fd(), name)
    }

    fn remove_root_child(&self, name: &str) -> Result<bool> {
        validate_staging_name(name)?;
        self.remove_entry(self.root.as_raw_fd(), name)
    }

    fn entry_is_owned(&self, parent: libc::c_int, name: &str) -> bool {
        open_directory_at_no_follow(parent, OsStr::new(name))
            .and_then(|directory| read_owned_marker(&directory))
            .is_ok_and(|marker| self.marker_matches_entry(parent, name, &marker))
    }

    fn marker_matches_entry(
        &self,
        parent: libc::c_int,
        name: &str,
        marker: &OwnedEntityMarker,
    ) -> bool {
        if marker.owner_nonce != self.root_owner_nonce {
            return false;
        }
        if parent == self.staging.as_raw_fd() {
            if is_prepare_temp_name(name) {
                marker.kind == "prepared"
            } else {
                marker.kind == "prepared" && marker.transaction_id == name
            }
        } else if let Some(transaction_id) = name.strip_prefix(".activation-stage-") {
            marker.kind == "prepared" && marker.transaction_id == transaction_id
        } else if let Some(transaction_id) = name.strip_prefix(".activation-snapshot-") {
            marker.kind == "snapshot" && marker.transaction_id == transaction_id
        } else {
            is_quarantine_name(name) && matches!(marker.kind.as_str(), "prepared" | "snapshot")
        }
    }

    fn remove_entry(&self, parent: libc::c_int, name: &str) -> Result<bool> {
        let Some(metadata) = stat_at_no_follow(parent, OsStr::new(name))? else {
            return Ok(false);
        };
        if !stat_is_directory(&metadata) {
            bail!("staging child must be a real directory");
        }
        let owned = open_directory_at_no_follow(parent, OsStr::new(name))?;
        let marker = read_owned_marker(&owned)
            .context("refusing to remove an unowned launcher directory")?;
        if !self.marker_matches_entry(parent, name, &marker) {
            bail!("launcher ownership marker does not match the directory entry");
        }
        verify_entry_matches_handle(parent, OsStr::new(name), &owned)?;
        let quarantine = format!(".remove-{}", Uuid::new_v4());
        rename_at_no_replace(
            parent,
            OsStr::new(name),
            self.root.as_raw_fd(),
            OsStr::new(&quarantine),
        )?;
        verify_entry_matches_handle(self.root.as_raw_fd(), OsStr::new(&quarantine), &owned)?;
        if parent == self.staging.as_raw_fd() {
            self.staging.sync_all()?;
        }
        self.root.sync_all()?;
        let quarantine_path = directory_fd_path(&self.root)?.join(&quarantine);
        fs::remove_dir_all(&quarantine_path)?;
        self.root.sync_all()?;
        Ok(true)
    }
}

#[cfg(unix)]
fn open_trusted_state_root(layout: &Layout, create: bool) -> Result<(File, File, RootIdentity)> {
    if !layout.root.is_absolute() {
        bail!("launcher state root must be absolute");
    }
    let root_parent_path = layout
        .root
        .parent()
        .with_context(|| format!("{} has no parent", layout.root.display()))?;
    let root_name = layout
        .root
        .file_name()
        .context("launcher state root has no final path component")?;
    let root_parent = open_directory_path_no_follow(root_parent_path)
        .context("launcher state root parent must be a trusted real directory")?;
    if create {
        mkdirat_if_missing(root_parent.as_raw_fd(), root_name)?;
    }
    let root_metadata = stat_at_no_follow(root_parent.as_raw_fd(), root_name)?
        .context("launcher state root does not exist")?;
    if !stat_is_directory(&root_metadata) {
        bail!("launcher state root must be a real directory");
    }
    let root = open_directory_at_no_follow(root_parent.as_raw_fd(), root_name)?;
    verify_entry_matches_handle(root_parent.as_raw_fd(), root_name, &root)?;
    set_directory_private(&root)?;
    let root_identity = validate_or_create_root_identity(&root_parent, root_name, &root)?;
    verify_entry_matches_handle(root_parent.as_raw_fd(), root_name, &root)?;
    Ok((root_parent, root, root_identity))
}

#[cfg(not(unix))]
impl TrustedStaging {
    fn open(_layout: &Layout, _create: bool) -> Result<Self> {
        bail!("transactional runtime staging is only supported on Unix")
    }

    fn child_exists(&self, _name: &str) -> Result<bool> {
        bail!("transactional runtime staging is only supported on Unix")
    }

    fn root_child_exists(&self, _name: &str) -> Result<bool> {
        bail!("transactional runtime staging is only supported on Unix")
    }

    fn remove_root_child(&self, _name: &str) -> Result<bool> {
        bail!("transactional runtime staging is only supported on Unix")
    }

    fn create_private_child(&self, _name: &str) -> Result<TrustedChild> {
        bail!("transactional runtime staging is only supported on Unix")
    }

    fn create_private_root_child(&self, _name: &str) -> Result<TrustedChild> {
        bail!("transactional runtime staging is only supported on Unix")
    }

    fn publish_child(
        &self,
        _source: &str,
        _destination: &str,
        _source_handle: &File,
    ) -> Result<()> {
        bail!("transactional runtime staging is only supported on Unix")
    }
}

fn validate_staging_name(name: &str) -> Result<()> {
    validate_relative_path(Path::new(name))?;
    if Path::new(name).components().count() != 1 {
        bail!("staging name must be exactly one path component");
    }
    Ok(())
}

fn claimed_stage_name(transaction_id: &str) -> String {
    format!(".activation-stage-{transaction_id}")
}

fn is_prepare_temp_name(name: &str) -> bool {
    name.strip_prefix(".prepare-")
        .is_some_and(|suffix| Uuid::parse_str(suffix).is_ok())
}

fn is_quarantine_name(name: &str) -> bool {
    name.strip_prefix(".remove-")
        .is_some_and(|suffix| Uuid::parse_str(suffix).is_ok())
}

#[cfg(unix)]
fn c_string(value: &OsStr) -> Result<CString> {
    CString::new(value.as_bytes()).context("filesystem path contains an interior NUL")
}

#[cfg(unix)]
fn open_directory_path_no_follow(path: &Path) -> Result<File> {
    let path = c_string(path.as_os_str())?;
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to open trusted directory");
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_directory_at_no_follow(parent: libc::c_int, name: &OsStr) -> Result<File> {
    let name = c_string(name)?;
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to open trusted child directory");
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn mkdirat_if_missing(parent: libc::c_int, name: &OsStr) -> Result<()> {
    let name = c_string(name)?;
    let result = unsafe { libc::mkdirat(parent, name.as_ptr(), 0o700) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return Ok(());
    }
    Err(error).context("failed to create trusted child directory")
}

#[cfg(unix)]
fn stat_at_no_follow(parent: libc::c_int, name: &OsStr) -> Result<Option<libc::stat>> {
    let name = c_string(name)?;
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe {
        libc::fstatat(
            parent,
            name.as_ptr(),
            stat.as_mut_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        return Ok(Some(unsafe { stat.assume_init() }));
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(None)
    } else {
        Err(error).context("failed to inspect trusted directory entry")
    }
}

#[cfg(unix)]
fn stat_is_directory(stat: &libc::stat) -> bool {
    stat.st_mode & libc::S_IFMT == libc::S_IFDIR
}

#[cfg(unix)]
fn stat_is_regular_file(stat: &libc::stat) -> bool {
    stat.st_mode & libc::S_IFMT == libc::S_IFREG
}

#[cfg(unix)]
fn set_directory_private(directory: &File) -> Result<()> {
    let result = unsafe { libc::fchmod(directory.as_raw_fd(), 0o700) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error()).context("failed to make launcher directory private")
    }
}

#[cfg(unix)]
fn verify_entry_matches_handle(parent: libc::c_int, name: &OsStr, handle: &File) -> Result<()> {
    let entry = stat_at_no_follow(parent, name)?.context("trusted directory entry disappeared")?;
    let mut opened = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe { libc::fstat(handle.as_raw_fd(), opened.as_mut_ptr()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to inspect trusted open handle");
    }
    let opened = unsafe { opened.assume_init() };
    if entry.st_dev != opened.st_dev || entry.st_ino != opened.st_ino {
        bail!("trusted directory entry was replaced");
    }
    Ok(())
}

#[cfg(unix)]
fn open_file_at_no_follow(parent: libc::c_int, name: &OsStr) -> Result<File> {
    let name = c_string(name)?;
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to open trusted child file");
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn create_file_at_exclusive(parent: libc::c_int, name: &OsStr, executable: bool) -> Result<File> {
    let name = c_string(name)?;
    let mode = if executable { 0o700 } else { 0o600 };
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error()).context("failed to create trusted child file");
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(unix)]
fn open_or_create_regular_file_at(parent: libc::c_int, name: &OsStr) -> Result<File> {
    let name = c_string(name)?;
    let fd = unsafe {
        libc::openat(
            parent,
            name.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to open trusted launcher file");
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let (mode, _, _) = file_metadata_identity(&file)?;
    if mode & libc::S_IFMT != libc::S_IFREG {
        bail!("trusted launcher file must be a regular file");
    }
    Ok(file)
}

#[cfg(unix)]
fn open_or_create_directory_at(parent: libc::c_int, name: &OsStr) -> Result<File> {
    mkdirat_if_missing(parent, name)?;
    open_directory_at_no_follow(parent, name)
}

#[cfg(unix)]
fn open_relative_parent_directory(
    root: &File,
    relative: &Path,
    create: bool,
) -> Result<(File, std::ffi::OsString)> {
    validate_relative_path(relative)?;
    let mut components = relative.components().peekable();
    let mut current = root.try_clone()?;
    while let Some(component) = components.next() {
        let Component::Normal(component) = component else {
            bail!("artifact path contains traversal: {}", relative.display());
        };
        if components.peek().is_none() {
            return Ok((current, component.to_os_string()));
        }
        current = if create {
            open_or_create_directory_at(current.as_raw_fd(), component)?
        } else {
            open_directory_at_no_follow(current.as_raw_fd(), component)?
        };
    }
    bail!("artifact path must not be empty")
}

#[cfg(unix)]
fn open_relative_file_no_follow(root: &File, relative: &Path) -> Result<File> {
    let (parent, name) = open_relative_parent_directory(root, relative, false)?;
    let metadata = stat_at_no_follow(parent.as_raw_fd(), &name)?
        .context("trusted relative file does not exist")?;
    if !stat_is_regular_file(&metadata) {
        bail!("trusted relative path is not a regular file");
    }
    open_file_at_no_follow(parent.as_raw_fd(), &name)
}

#[cfg(unix)]
fn copy_file_handle(source: &File, destination: &mut File, executable: bool) -> Result<()> {
    let mut source = source.try_clone()?;
    std::io::copy(&mut source, destination)?;
    if executable {
        let result = unsafe { libc::fchmod(destination.as_raw_fd(), 0o700) };
        if result != 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to mark trusted artifact executable");
        }
    }
    destination.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn replace_relative_file_from_handle(
    destination_root: &File,
    relative: &Path,
    source: &File,
    executable: bool,
) -> Result<()> {
    let (parent, name) = open_relative_parent_directory(destination_root, relative, true)?;
    if let Some(existing) = stat_at_no_follow(parent.as_raw_fd(), &name)? {
        if !stat_is_regular_file(&existing) {
            bail!("artifact destination is not a replaceable regular file");
        }
        let name_c = c_string(&name)?;
        let result = unsafe { libc::unlinkat(parent.as_raw_fd(), name_c.as_ptr(), 0) };
        if result != 0 {
            return Err(std::io::Error::last_os_error())
                .context("failed to retire existing staged artifact");
        }
    }
    let mut destination = create_file_at_exclusive(parent.as_raw_fd(), &name, executable)?;
    copy_file_handle(source, &mut destination, executable)?;
    parent.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn copy_directory_handles(source: &File, destination: &File) -> Result<()> {
    for entry in fs::read_dir(directory_fd_path(source)?)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == OsStr::new(OWNERSHIP_MARKER) {
            continue;
        }
        let metadata = stat_at_no_follow(source.as_raw_fd(), &name)?
            .context("source directory entry disappeared during copy")?;
        if stat_is_directory(&metadata) {
            let source_child = open_directory_at_no_follow(source.as_raw_fd(), &name)?;
            let destination_child = open_or_create_directory_at(destination.as_raw_fd(), &name)?;
            copy_directory_handles(&source_child, &destination_child)?;
            destination_child.sync_all()?;
        } else if stat_is_regular_file(&metadata) {
            let source_file = open_file_at_no_follow(source.as_raw_fd(), &name)?;
            let executable = metadata.st_mode & 0o111 != 0;
            let mut destination_file =
                create_file_at_exclusive(destination.as_raw_fd(), &name, executable)?;
            copy_file_handle(&source_file, &mut destination_file, executable)?;
        } else {
            bail!(
                "source directory contains unsupported entry: {}",
                name.to_string_lossy()
            );
        }
    }
    destination.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn write_json_at<T: Serialize>(directory: &File, name: &OsStr, value: &T) -> Result<()> {
    let mut destination = create_file_at_exclusive(directory.as_raw_fd(), name, false)?;
    let bytes = serde_json::to_vec_pretty(value)?;
    destination.write_all(&bytes)?;
    destination.write_all(b"\n")?;
    destination.sync_all()?;
    directory.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn write_owned_marker(
    directory: &File,
    transaction_id: &str,
    kind: &str,
    root_owner_nonce: &str,
) -> Result<()> {
    write_json_at(
        directory,
        OsStr::new(OWNERSHIP_MARKER),
        &OwnedEntityMarker {
            schema_version: SCHEMA_VERSION,
            transaction_id: transaction_id.to_string(),
            kind: kind.to_string(),
            owner_nonce: root_owner_nonce.to_string(),
        },
    )
}

#[cfg(unix)]
fn read_owned_marker(directory: &File) -> Result<OwnedEntityMarker> {
    let marker = open_file_at_no_follow(directory.as_raw_fd(), OsStr::new(OWNERSHIP_MARKER))?;
    let mut marker = marker;
    let parsed: OwnedEntityMarker = serde_json::from_reader(&mut marker)?;
    if parsed.schema_version != SCHEMA_VERSION
        || Uuid::parse_str(&parsed.owner_nonce).is_err()
        || parsed.transaction_id.is_empty()
    {
        bail!("launcher ownership marker is invalid");
    }
    Ok(parsed)
}

#[cfg(unix)]
fn root_identity_name(root_name: &OsStr) -> String {
    let digest = Sha256::digest(root_name.as_bytes());
    format!("{ROOT_IDENTITY_PREFIX}{digest:x}.json")
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<(u64, u64)> {
    let (_, device, inode) = file_metadata_identity(file)?;
    Ok((device, inode))
}

#[cfg(unix)]
fn file_metadata_identity(file: &File) -> Result<(libc::mode_t, u64, u64)> {
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe { libc::fstat(file.as_raw_fd(), stat.as_mut_ptr()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to inspect trusted root handle");
    }
    let stat = unsafe { stat.assume_init() };
    Ok((stat.st_mode, stat.st_dev as u64, stat.st_ino as u64))
}

#[cfg(unix)]
fn validate_or_create_root_identity(
    root_parent: &File,
    root_name: &OsStr,
    root: &File,
) -> Result<RootIdentity> {
    let identity_name = root_identity_name(root_name);
    let identity_name = OsStr::new(&identity_name);
    let (root_device, root_inode) = file_identity(root)?;
    let identity = match stat_at_no_follow(root_parent.as_raw_fd(), identity_name)? {
        Some(metadata) => {
            if !stat_is_regular_file(&metadata) {
                bail!("launcher state root identity must be a regular file");
            }
            read_root_identity(root_parent, identity_name)?
        }
        None => {
            let identity = RootIdentity {
                schema_version: SCHEMA_VERSION,
                root_device,
                root_inode,
                owner_nonce: Uuid::new_v4().to_string(),
            };
            publish_root_identity(root_parent, identity_name, &identity)?
        }
    };
    if identity.schema_version != SCHEMA_VERSION
        || Uuid::parse_str(&identity.owner_nonce).is_err()
        || identity.root_device != root_device
        || identity.root_inode != root_inode
    {
        bail!("launcher state root identity does not match the trusted root");
    }
    root_parent.sync_all()?;
    Ok(identity)
}

#[cfg(unix)]
fn read_root_identity(root_parent: &File, name: &OsStr) -> Result<RootIdentity> {
    let mut file = open_file_at_no_follow(root_parent.as_raw_fd(), name)?;
    serde_json::from_reader::<_, RootIdentity>(&mut file)
        .context("failed to parse launcher state root identity")
}

#[cfg(unix)]
fn publish_root_identity(
    root_parent: &File,
    destination: &OsStr,
    identity: &RootIdentity,
) -> Result<RootIdentity> {
    let temp_name = format!("{ROOT_IDENTITY_PREFIX}{}.tmp", Uuid::new_v4());
    let temp_name = OsStr::new(&temp_name);
    write_json_at(root_parent, temp_name, identity)?;
    if let Err(error) = rename_at_no_replace(
        root_parent.as_raw_fd(),
        temp_name,
        root_parent.as_raw_fd(),
        destination,
    ) {
        remove_file_at_if_exists(root_parent.as_raw_fd(), temp_name)?;
        if stat_at_no_follow(root_parent.as_raw_fd(), destination)?.is_some() {
            return read_root_identity(root_parent, destination);
        }
        return Err(error).context("failed to publish launcher state root identity");
    }
    root_parent.sync_all()?;
    Ok(identity.clone())
}

#[cfg(unix)]
fn remove_file_at_if_exists(parent: libc::c_int, name: &OsStr) -> Result<()> {
    let name = c_string(name)?;
    let result = unsafe { libc::unlinkat(parent, name.as_ptr(), 0) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::NotFound {
        Ok(())
    } else {
        Err(error).context("failed to remove trusted temporary file")
    }
}

#[cfg(unix)]
fn hash_file_handle(file: &File) -> Result<String> {
    let mut file = file.try_clone()?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(target_os = "macos")]
fn rename_at_no_replace(
    source_parent: libc::c_int,
    source: &OsStr,
    destination_parent: libc::c_int,
    destination: &OsStr,
) -> Result<()> {
    let source = c_string(source)?;
    let destination = c_string(destination)?;
    let result = unsafe {
        libc::renameatx_np(
            source_parent,
            source.as_ptr(),
            destination_parent,
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
            .context("failed to atomically publish trusted directory entry")
    }
}

#[cfg(target_os = "linux")]
fn rename_at_no_replace(
    source_parent: libc::c_int,
    source: &OsStr,
    destination_parent: libc::c_int,
    destination: &OsStr,
) -> Result<()> {
    let source = c_string(source)?;
    let destination = c_string(destination)?;
    let result = unsafe {
        libc::renameat2(
            source_parent,
            source.as_ptr(),
            destination_parent,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
            .context("failed to atomically publish trusted directory entry")
    }
}

#[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
fn rename_at_no_replace(
    _source_parent: libc::c_int,
    _source: &OsStr,
    _destination_parent: libc::c_int,
    _destination: &OsStr,
) -> Result<()> {
    bail!("atomic no-replace staging publication is unsupported on this Unix platform")
}

#[cfg(target_os = "macos")]
fn directory_fd_path(file: &File) -> Result<PathBuf> {
    let mut path = [0 as libc::c_char; libc::PATH_MAX as usize];
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, path.as_mut_ptr()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to resolve trusted directory handle path");
    }
    let bytes = unsafe { std::slice::from_raw_parts(path.as_ptr().cast::<u8>(), path.len()) };
    let nul = bytes
        .iter()
        .position(|byte| *byte == 0)
        .context("trusted directory handle path is not NUL-terminated")?;
    let path = CStr::from_bytes_with_nul(&bytes[..=nul])
        .context("trusted directory handle path is invalid")?;
    let resolved = PathBuf::from(OsStr::from_bytes(path.to_bytes()));
    let metadata = fs::metadata(&resolved).with_context(|| {
        format!(
            "resolved trusted directory handle path is unavailable: {}",
            resolved.display()
        )
    })?;
    let mut opened = std::mem::MaybeUninit::<libc::stat>::uninit();
    let result = unsafe { libc::fstat(file.as_raw_fd(), opened.as_mut_ptr()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .context("failed to inspect trusted directory handle");
    }
    let opened = unsafe { opened.assume_init() };
    if !metadata.is_dir()
        || metadata.dev() != opened.st_dev as u64
        || metadata.ino() != opened.st_ino as u64
    {
        bail!("resolved trusted directory handle path no longer matches the open directory");
    }
    Ok(resolved)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn directory_fd_path(file: &File) -> Result<PathBuf> {
    Ok(PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd())))
}

#[cfg(not(unix))]
fn directory_fd_path(_file: &File) -> Result<PathBuf> {
    Ok(PathBuf::new())
}

#[cfg(not(unix))]
fn copy_directory_handles(_source: &File, _destination: &File) -> Result<()> {
    bail!("transactional runtime staging is only supported on Unix")
}

#[cfg(unix)]
fn claim_staging_child(
    layout: &Layout,
    transaction_id: &str,
) -> Result<(TrustedStaging, TrustedChild)> {
    validate_staging_name(transaction_id)?;
    let staging = TrustedStaging::open(layout, true)?;
    let claim_name = claimed_stage_name(transaction_id);
    let published = stat_at_no_follow(staging.staging.as_raw_fd(), OsStr::new(transaction_id))?;
    let claimed = stat_at_no_follow(staging.root.as_raw_fd(), OsStr::new(&claim_name))?;
    match (published, claimed) {
        (Some(published), None) if stat_is_directory(&published) => {
            let published = open_directory_at_no_follow(
                staging.staging.as_raw_fd(),
                OsStr::new(transaction_id),
            )?;
            let marker = read_owned_marker(&published)?;
            if !staging.marker_matches_entry(staging.staging.as_raw_fd(), transaction_id, &marker) {
                bail!("published staging directory ownership does not match the transaction");
            }
            verify_entry_matches_handle(
                staging.staging.as_raw_fd(),
                OsStr::new(transaction_id),
                &published,
            )?;
            rename_at_no_replace(
                staging.staging.as_raw_fd(),
                OsStr::new(transaction_id),
                staging.root.as_raw_fd(),
                OsStr::new(&claim_name),
            )?;
            verify_entry_matches_handle(
                staging.root.as_raw_fd(),
                OsStr::new(&claim_name),
                &published,
            )?;
            staging.staging.sync_all()?;
            staging.root.sync_all()?;
        }
        (None, Some(claimed)) if stat_is_directory(&claimed) => {}
        (Some(_), None) => bail!("published staging child must be a real directory"),
        (None, None) => bail!("published staging child does not exist"),
        _ => bail!("published and claimed staging children are in an ambiguous state"),
    }
    let file = open_directory_at_no_follow(staging.root.as_raw_fd(), OsStr::new(&claim_name))?;
    let marker = read_owned_marker(&file)?;
    if !staging.marker_matches_entry(staging.root.as_raw_fd(), &claim_name, &marker) {
        bail!("claimed staging directory ownership does not match the transaction");
    }
    let child = TrustedChild {
        file,
        path: directory_fd_path(&staging.root)?.join(claim_name),
    };
    Ok((staging, child))
}

#[cfg(unix)]
fn install_claimed_stage_as_current(
    staging: &TrustedStaging,
    claimed_name: &str,
    claimed: &TrustedChild,
) -> Result<()> {
    verify_entry_matches_handle(
        staging.root.as_raw_fd(),
        OsStr::new(claimed_name),
        &claimed.file,
    )?;
    rename_at_no_replace(
        staging.root.as_raw_fd(),
        OsStr::new(claimed_name),
        staging.root.as_raw_fd(),
        OsStr::new("current"),
    )?;
    verify_entry_matches_handle(
        staging.root.as_raw_fd(),
        OsStr::new("current"),
        &claimed.file,
    )?;
    staging.root.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn install_claimed_stage_as_current(
    _staging: &TrustedStaging,
    _claimed_name: &str,
    _claimed: &TrustedChild,
) -> Result<()> {
    bail!("transactional runtime staging is only supported on Unix")
}

#[cfg(not(unix))]
fn claim_staging_child(
    _layout: &Layout,
    _transaction_id: &str,
) -> Result<(TrustedStaging, TrustedChild)> {
    bail!("transactional runtime staging is only supported on Unix")
}

pub fn resolve_staging_child(
    layout: &Layout,
    transaction_id: &str,
    must_exist: bool,
) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        validate_staging_name(transaction_id)?;
        let staging = TrustedStaging::open(layout, true)?;
        let claim_name = claimed_stage_name(transaction_id);
        let published = stat_at_no_follow(staging.staging.as_raw_fd(), OsStr::new(transaction_id))?;
        let claimed = stat_at_no_follow(staging.root.as_raw_fd(), OsStr::new(&claim_name))?;
        match (published, claimed) {
            (Some(metadata), None) if stat_is_directory(&metadata) => {
                let child = staging.open_child(transaction_id)?;
                Ok(child.path)
            }
            (None, Some(metadata)) if must_exist && stat_is_directory(&metadata) => {
                let child = staging.open_root_child(&claim_name)?;
                Ok(child.path)
            }
            (Some(_), None) => bail!("staging child must be a real directory"),
            (None, Some(_)) if must_exist => {
                bail!("claimed staging child must be a real directory")
            }
            (None, None) if must_exist => {
                bail!("staging child does not exist for transaction {transaction_id}")
            }
            (None, None) => Ok(layout.staging()?.join(transaction_id)),
            _ => bail!("published and claimed staging children are in an ambiguous state"),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (layout, transaction_id, must_exist);
        bail!("transactional runtime staging is only supported on Unix")
    }
}

pub fn remove_staging_child(layout: &Layout, transaction_id: &str) -> Result<bool> {
    #[cfg(unix)]
    {
        validate_staging_name(transaction_id)?;
        let staging = TrustedStaging::open(layout, true)?;
        let claim_name = claimed_stage_name(transaction_id);
        let published = staging.child_exists(transaction_id)?;
        let claimed = staging.root_child_exists(&claim_name)?;
        match (published, claimed) {
            (true, false) => staging.remove_child(transaction_id),
            (false, true) => staging.remove_root_child(&claim_name),
            (false, false) => Ok(false),
            (true, true) => {
                bail!("published and claimed staging children are in an ambiguous state")
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (layout, transaction_id);
        bail!("transactional runtime staging is only supported on Unix")
    }
}

#[cfg(unix)]
fn validate_staged_candidate_handle(
    stage: &File,
    expected_build_id: &str,
    expected_source_commit: &str,
    expected_manifest_hash: &str,
    expected_artifact_content_hash: &str,
) -> Result<PreparedManifest> {
    let manifest_file = open_file_at_no_follow(stage.as_raw_fd(), OsStr::new("manifest.json"))?;
    let mut manifest_reader = manifest_file.try_clone()?;
    let manifest: PreparedManifest = serde_json::from_reader(&mut manifest_reader)?;
    if manifest.schema_version != SCHEMA_VERSION
        || manifest.artifacts.is_empty()
        || manifest.build_id != expected_build_id
        || manifest.source_commit != expected_source_commit
        || manifest_hash(&manifest)? != expected_manifest_hash
        || artifact_content_hash(&manifest) != expected_artifact_content_hash
    {
        bail!("staged candidate identity does not match its durable transaction");
    }
    let resources = open_directory_at_no_follow(stage.as_raw_fd(), OsStr::new("resources"))?;
    let mut artifact_paths = BTreeSet::new();
    for artifact in &manifest.artifacts {
        validate_relative_path(&artifact.relative_path)?;
        if !artifact_paths.insert(artifact.relative_path.clone()) {
            bail!(
                "duplicate staged artifact path {}",
                artifact.relative_path.display()
            );
        }
        let source = open_relative_file_no_follow(&resources, &artifact.relative_path)?;
        if hash_file_handle(&source)? != artifact.sha256.to_ascii_lowercase() {
            bail!(
                "staged artifact hash mismatch: {}",
                artifact.relative_path.display()
            );
        }
    }
    Ok(manifest)
}

#[cfg(not(unix))]
fn validate_staged_candidate_handle(
    _stage: &File,
    _expected_build_id: &str,
    _expected_source_commit: &str,
    _expected_manifest_hash: &str,
    _expected_artifact_content_hash: &str,
) -> Result<PreparedManifest> {
    bail!("transactional runtime staging is only supported on Unix")
}

fn restore_previous_slot_durably(
    layout: &Layout,
    transaction: &mut Transaction,
    rollback: &mut PostReadyRollback,
) -> Result<()> {
    let failed_slot = layout.active_root()?.join(
        rollback
            .failed_slot_path
            .file_name()
            .context("post-ready failed slot path has no file name")?,
    );
    if layout.current()?.exists() && layout.previous()?.exists() && !failed_slot.exists() {
        fs::rename(layout.current()?, &failed_slot)?;
        sync_dir(&layout.active_root()?)?;
    }
    if failed_slot.exists() {
        rollback.slot_restore_phase = SlotRestorePhase::CandidateRetired;
        transaction.post_ready_rollback = Some(rollback.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }
    if !layout.current()?.exists() && layout.previous()?.exists() {
        fs::rename(layout.previous()?, layout.current()?)?;
        sync_dir(&layout.active_root()?)?;
    }
    if layout.current()?.exists() && !layout.previous()?.exists() && failed_slot.exists() {
        rollback.slot_restore_phase = SlotRestorePhase::PreviousRestored;
        transaction.post_ready_rollback = Some(rollback.clone());
        write_json_atomically(&layout.transaction()?, transaction)?;
    }
    if !layout.current()?.exists() || !failed_slot.exists() || layout.previous()?.exists() {
        bail!("post-ready slot rollback is not in a recoverable state");
    }
    Ok(())
}

fn install_rollback_resources_durably(
    source: &Path,
    resources: &Path,
    replacement: &Path,
    backup: &Path,
) -> Result<()> {
    let contents = resources
        .parent()
        .with_context(|| format!("{} has no parent", resources.display()))?;
    if resources.exists() && !backup.exists() {
        if replacement.exists() {
            fs::remove_dir_all(replacement)?;
        }
        copy_directory(source, replacement)?;
        fs::rename(resources, backup)?;
        sync_dir(contents)?;
    }
    if !resources.exists() && backup.exists() {
        if !replacement.exists() {
            copy_directory(source, replacement)?;
        }
        fs::rename(replacement, resources)?;
        sync_dir(contents)?;
    }
    if !resources.exists() || !backup.exists() {
        bail!("post-ready resource rollback is not in a recoverable state");
    }
    Ok(())
}

pub fn commit_activation_files(layout: &Layout, transaction: &mut Transaction) -> Result<()> {
    bind_transaction_paths(layout, transaction)?;
    for path in [
        transaction.resources_backup_path.as_ref(),
        transaction.resources_replacement_path.as_ref(),
        transaction.resources_retired_path.as_ref(),
        transaction.signature_backup_path.as_ref(),
        transaction.runtime_retired_path.as_ref(),
        transaction.runtime_replacement_path.as_ref(),
        transaction.signature_retired_path.as_ref(),
        transaction.signature_replacement_path.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        if path.exists() {
            if path.is_dir() {
                fs::remove_dir_all(path)?;
            } else {
                fs::remove_file(path)?;
            }
        }
    }
    for stored in [
        transaction.slot_retired_path.as_ref(),
        transaction.slot_failed_path.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        let path = layout.active_root()?.join(
            stored
                .file_name()
                .context("transaction slot cleanup path has no file name")?,
        );
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
    }
    Ok(())
}

pub fn bind_transaction_paths(layout: &Layout, transaction: &mut Transaction) -> Result<()> {
    #[cfg(unix)]
    let _trusted_root = TrustedStaging::open(layout, true)?;
    validate_request(&transaction.request)?;
    let transaction_id = &transaction.request.transaction_id;
    let app_bundle = &transaction.request.app_bundle_path;
    let contents = app_bundle.join("Contents");
    let bundle_parent = app_bundle
        .parent()
        .with_context(|| format!("{} has no parent", app_bundle.display()))?;
    bind_optional_path(
        &mut transaction.slot_retired_path,
        layout
            .root
            .join(format!(".slot-previous-retired-{transaction_id}")),
        "slotRetiredPath",
    )?;
    bind_optional_path(
        &mut transaction.slot_failed_path,
        layout
            .root
            .join(format!(".slot-candidate-failed-{transaction_id}")),
        "slotFailedPath",
    )?;
    bind_optional_path(
        &mut transaction.resources_backup_path,
        bundle_parent.join(format!(".MorpheusResourcesPrevious-{transaction_id}")),
        "resourcesBackupPath",
    )?;
    bind_optional_path(
        &mut transaction.resources_replacement_path,
        contents.join(format!(".MorpheusResourcesCandidate-{transaction_id}")),
        "resourcesReplacementPath",
    )?;
    bind_optional_path(
        &mut transaction.resources_retired_path,
        bundle_parent.join(format!(".MorpheusResourcesFailed-{transaction_id}")),
        "resourcesRetiredPath",
    )?;
    bind_optional_path(
        &mut transaction.signature_backup_path,
        bundle_parent.join(format!(".MorpheusSignaturePrevious-{transaction_id}")),
        "signatureBackupPath",
    )?;
    bind_optional_path(
        &mut transaction.runtime_retired_path,
        bundle_parent.join(format!(".MorpheusRuntimeFailed-{transaction_id}")),
        "runtimeRetiredPath",
    )?;
    bind_optional_path(
        &mut transaction.runtime_replacement_path,
        contents.join(format!(
            "MacOS/.Root Worker Runtime.restore-{transaction_id}"
        )),
        "runtimeReplacementPath",
    )?;
    bind_optional_path(
        &mut transaction.signature_retired_path,
        bundle_parent.join(format!(".MorpheusCodeSignatureFailed-{transaction_id}")),
        "signatureRetiredPath",
    )?;
    bind_optional_path(
        &mut transaction.signature_replacement_path,
        contents.join(format!(".MorpheusCodeSignatureRestore-{transaction_id}")),
        "signatureReplacementPath",
    )?;
    if let Some(rollback) = &mut transaction.post_ready_rollback {
        bind_required_path(
            &mut rollback.failed_slot_path,
            layout
                .root
                .join(format!(".failed-current-{transaction_id}")),
            "postReadyRollback.failedSlotPath",
        )?;
        bind_required_path(
            &mut rollback.resources_replacement_path,
            contents.join(format!(".MorpheusResourcesRollback-{transaction_id}")),
            "postReadyRollback.resourcesReplacementPath",
        )?;
        bind_required_path(
            &mut rollback.resources_backup_path,
            bundle_parent.join(format!(".MorpheusResourcesFailed-{transaction_id}")),
            "postReadyRollback.resourcesBackupPath",
        )?;
    }
    Ok(())
}

fn bind_optional_path(stored: &mut Option<PathBuf>, derived: PathBuf, field: &str) -> Result<()> {
    if let Some(stored) = stored.as_ref()
        && stored != &derived
    {
        bail!("{field} does not match the transaction-owned path");
    }
    *stored = Some(derived);
    Ok(())
}

fn bind_required_path(stored: &mut PathBuf, derived: PathBuf, field: &str) -> Result<()> {
    if stored != &derived {
        bail!("{field} does not match the transaction-owned path");
    }
    Ok(())
}

fn artifact_is_executable(kind: &str) -> bool {
    matches!(
        kind,
        "executable" | "launcher" | "app-server" | "runtime-executable"
    )
}

fn copy_regular_file(source: &Path, destination: &Path, executable: bool) -> Result<()> {
    let parent = destination
        .parent()
        .with_context(|| format!("{} has no parent", destination.display()))?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".launcher-copy-{}", Uuid::new_v4()));
    fs::copy(source, &temp).with_context(|| {
        format!(
            "failed to copy {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    #[cfg(unix)]
    if executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o755))?;
    }
    File::open(&temp)?.sync_all()?;
    fs::rename(&temp, destination)?;
    sync_dir(parent)?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

pub fn ready_matches(ready: &ReadyIdentity, transaction: &Transaction, instance_id: &str) -> bool {
    ready.schema_version == SCHEMA_VERSION
        && ready.ready_at_ms > 0
        && ready.transaction_id == transaction.request.transaction_id
        && ready.build_id == transaction.request.build_id
        && ready.instance_id == instance_id
}

pub fn crash_decision(
    history: &[CrashRecord],
    build_hash: &str,
    now: DateTime<Utc>,
) -> CrashDecision {
    let recent_30 = history
        .iter()
        .filter(|record| {
            record.build_hash == build_hash && now - record.occurred_at <= Duration::seconds(30)
        })
        .count();
    let recent_10m = history
        .iter()
        .filter(|record| {
            record.build_hash == build_hash && now - record.occurred_at <= Duration::minutes(10)
        })
        .count();
    if recent_30 >= 2 || recent_10m >= 3 {
        CrashDecision::RollBack
    } else {
        CrashDecision::RestartCurrent
    }
}

pub fn record_crash(
    state: &mut RuntimeState,
    build_hash: &str,
    now: DateTime<Utc>,
) -> CrashDecision {
    state
        .crash_history
        .retain(|record| now - record.occurred_at <= Duration::minutes(10));
    state.crash_history.push(CrashRecord {
        build_hash: build_hash.to_string(),
        occurred_at: now,
    });
    crash_decision(&state.crash_history, build_hash, now)
}

pub fn record_failure(state: &mut RuntimeState, evidence: FailureEvidence) {
    state.failures.push(evidence);
    if state.failures.len() > MAX_FAILURE_EVIDENCE {
        state
            .failures
            .drain(0..state.failures.len() - MAX_FAILURE_EVIDENCE);
    }
}

pub fn request_failure(
    request: &ActivationRequest,
    manifest_hash: Option<String>,
    phase: impl Into<String>,
    summary: impl Into<String>,
) -> FailureEvidence {
    FailureEvidence {
        schema_version: SCHEMA_VERSION,
        recovery_identity: Some(Uuid::new_v4().to_string()),
        launcher_owner_nonce: None,
        occurred_at: Utc::now(),
        transaction_id: Some(request.transaction_id.clone()),
        request_id: Some(request.request_id.clone()),
        requested_by_thread_id: request.requested_by_thread_id.clone(),
        mode: Some(request.mode),
        build_id: Some(request.build_id.clone()),
        source_commit: Some(request.source_commit.clone()),
        manifest_hash,
        failed_build_hash: None,
        failure_phase: phase.into(),
        summary: summary.into(),
        reason: Some(request.reason.clone()),
        app_bundle_path: Some(request.app_bundle_path.clone()),
        exit_code: None,
        signal: None,
        ready_timeout_ms: None,
        log_path: None,
        transaction_path: None,
        recovered_build_id: None,
        claim_id: None,
        acknowledged: false,
    }
}

pub fn gc_staging(layout: &Layout, preserve: Option<&str>, keep: usize) -> Result<()> {
    #[cfg(unix)]
    {
        let staging = TrustedStaging::open(layout, true)?;
        let entries = fs::read_dir(directory_fd_path(&staging.staging)?)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter(|entry| preserve.is_none_or(|name| entry.file_name() != OsStr::new(name)))
            .collect::<Vec<_>>();
        for entry in entries
            .iter()
            .filter(|entry| entry.file_name().to_str().is_some_and(is_prepare_temp_name))
        {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("staging child name is not valid UTF-8"))?;
            if staging.entry_is_owned(staging.staging.as_raw_fd(), &name) {
                staging.remove_child(&name)?;
            }
        }
        let mut entries = entries
            .into_iter()
            .filter(|entry| !entry.file_name().to_str().is_some_and(is_prepare_temp_name))
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| staging.entry_is_owned(staging.staging.as_raw_fd(), name))
            })
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| {
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
        });
        let remove_count = entries.len().saturating_sub(keep);
        for entry in entries.into_iter().take(remove_count) {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("staging child name is not valid UTF-8"))?;
            if staging.entry_is_owned(staging.staging.as_raw_fd(), &name) {
                staging.remove_child(&name)?;
            }
        }
        let preserved_claim = preserve.map(claimed_stage_name);
        let preserved_snapshot =
            preserve.map(|transaction_id| format!(".activation-snapshot-{transaction_id}"));
        let root_entries = fs::read_dir(directory_fd_path(&staging.root)?)?
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| {
                name.starts_with(".activation-stage-")
                    || name.starts_with(".activation-snapshot-")
                    || is_quarantine_name(name)
            })
            .collect::<Vec<_>>();
        for name in root_entries {
            if preserved_claim.as_deref() == Some(name.as_str())
                || preserved_snapshot.as_deref() == Some(name.as_str())
            {
                continue;
            }
            if staging.entry_is_owned(staging.root.as_raw_fd(), &name) {
                staging.remove_root_child(&name)?;
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (layout, preserve, keep);
        bail!("transactional runtime staging is only supported on Unix")
    }
}

pub fn recover_interrupted_transaction(layout: &Layout) -> Result<Option<Transaction>> {
    recover_interrupted_transaction_impl(layout, None)
}

pub fn recover_interrupted_transaction_with(
    layout: &Layout,
    runner: &dyn CommandRunner,
) -> Result<Option<Transaction>> {
    recover_interrupted_transaction_impl(layout, Some(runner))
}

fn recover_interrupted_transaction_impl(
    layout: &Layout,
    runner: Option<&dyn CommandRunner>,
) -> Result<Option<Transaction>> {
    if !layout.transaction()?.exists() {
        return Ok(None);
    }
    let mut transaction: Transaction = read_json(&layout.transaction()?)?;
    persist_interrupted_rollback_evidence(layout, &mut transaction)?;
    if validate_staging_name(&transaction.request.transaction_id).is_ok() {
        gc_staging(layout, Some(&transaction.request.transaction_id), 4)?;
    }
    match transaction.phase {
        TransactionPhase::Prepared => {}
        TransactionPhase::Activating
        | TransactionPhase::CandidateInstalled
        | TransactionPhase::CandidateStarted => {
            if let Some(runner) = runner {
                rollback_activation_with(layout, &mut transaction, runner)?;
            } else {
                rollback_activation(layout, &mut transaction)?;
            }
        }
        TransactionPhase::RollingBack => {
            if transaction.post_ready_rollback.is_some() {
                let runner = runner
                    .context("post-ready rollback recovery requires a codesign command runner")?;
                let mut resumable = transaction.clone();
                resume_post_ready_rollback(layout, &mut resumable, runner)?;
            } else if let Some(runner) = runner {
                rollback_activation_with(layout, &mut transaction, runner)?;
            } else {
                rollback_activation(layout, &mut transaction)?;
            }
        }
        TransactionPhase::RollbackComplete => {
            commit_activation_files(layout, &mut transaction)?;
        }
        TransactionPhase::Ready => {}
    }
    if matches!(
        transaction.phase,
        TransactionPhase::Activating
            | TransactionPhase::CandidateInstalled
            | TransactionPhase::CandidateStarted
    ) {
        fs::remove_file(layout.transaction()?)?;
    } else if transaction.phase == TransactionPhase::RollingBack
        && transaction.post_ready_rollback.is_none()
    {
        fs::remove_file(layout.transaction()?)?;
    }
    Ok(Some(transaction))
}

fn persist_interrupted_rollback_evidence(
    layout: &Layout,
    transaction: &mut Transaction,
) -> Result<()> {
    let needs_evidence = matches!(
        transaction.phase,
        TransactionPhase::Activating
            | TransactionPhase::CandidateInstalled
            | TransactionPhase::CandidateStarted
            | TransactionPhase::RollingBack
    ) && transaction.post_ready_rollback.is_none()
        && transaction.rollback_failure_evidence.is_none();
    if !needs_evidence {
        return Ok(());
    }
    let interrupted_phase = transaction.phase;
    let mut evidence = request_failure(
        &transaction.request,
        Some(transaction.manifest_hash.clone()),
        "interrupted-recovery",
        format!(
            "launcher resumed an interrupted activation transaction from phase {interrupted_phase:?}"
        ),
    );
    evidence.failed_build_hash = Some(transaction.artifact_content_hash.clone());
    evidence.ready_timeout_ms = Some(READY_TIMEOUT_SECS * 1000);
    evidence.log_path = Some(layout.root.join("launcher.log"));
    evidence.transaction_path = Some(layout.configured_transaction());
    transaction.rollback_failure_evidence = Some(evidence);
    write_json_atomically(&layout.transaction()?, transaction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn artifact(path: &str, contents: &[u8]) -> PreparedArtifact {
        PreparedArtifact {
            relative_path: PathBuf::from(path),
            sha256: hex_digest(contents),
            kind: "file".into(),
        }
    }

    #[cfg(unix)]
    fn mark_owned(layout: &Layout, path: &Path, transaction_id: &str, kind: &str) -> Result<()> {
        let staging = TrustedStaging::open(layout, true)?;
        let directory = open_directory_path_no_follow(path)?;
        write_owned_marker(&directory, transaction_id, kind, staging.root_owner_nonce())
    }

    #[cfg(unix)]
    fn transaction_for_path_binding(layout: &Layout, app_bundle: &Path) -> Transaction {
        let failed = build_record("failed", app_bundle);
        let fallback = build_record("fallback", app_bundle);
        let transaction_id = "tx";
        Transaction {
            schema_version: SCHEMA_VERSION,
            request: ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: transaction_id.into(),
                request_id: "request".into(),
                requested_by_thread_id: None,
                mode: ActivationMode::Full,
                build_id: "failed".into(),
                source_commit: "commit-failed".into(),
                prepared_root: layout.root.join("prepared"),
                app_bundle_path: app_bundle.to_path_buf(),
                reason: "test".into(),
            },
            manifest_hash: "manifest-failed".into(),
            artifact_content_hash: "content-failed".into(),
            phase: TransactionPhase::RollbackComplete,
            instance_id: None,
            slot_rotated: false,
            slot_rotation_phase: SlotRotationPhase::NotStarted,
            slot_restore_phase: SlotRestorePhase::NotStarted,
            slot_retired_path: None,
            slot_failed_path: None,
            slot_had_previous: None,
            resources_swapped: false,
            resources_backup_path: None,
            resources_replacement_path: None,
            resources_retired_path: None,
            resource_swap_phase: ResourceSwapPhase::NotStarted,
            resource_restore_phase: ResourceRestorePhase::NotStarted,
            signature_backup_path: None,
            signature_backup_ready: Some(false),
            signature_backup_had_code_signature: Some(false),
            signature_restore_phase: SignatureRestorePhase::NotStarted,
            runtime_retired_path: None,
            runtime_replacement_path: None,
            signature_retired_path: None,
            signature_replacement_path: None,
            signature_replacement_ready: Some(false),
            launcher_expected_hash: None,
            rollback_failure_evidence: None,
            post_ready_rollback: Some(PostReadyRollback {
                failed_build: failed.clone(),
                fallback_build: fallback,
                failure_evidence: failure_for_build(&failed),
                phase: PostReadyRollbackPhase::StateCommitted,
                failed_slot_path: layout.root.join(".failed-current-tx"),
                resources_replacement_path: app_bundle
                    .join("Contents/.MorpheusResourcesRollback-tx"),
                resources_backup_path: app_bundle
                    .parent()
                    .expect("test app bundle has a parent")
                    .join(".MorpheusResourcesFailed-tx"),
                slot_restore_phase: SlotRestorePhase::NotStarted,
                superseded_transaction: None,
            }),
            started_at: Utc::now(),
        }
    }

    #[test]
    fn slot_rotation_recovers_each_rename_window() -> Result<()> {
        for window in 0..3 {
            let temp = TempDir::new()?;
            let layout = Layout::new(temp.path().join("launcher"));
            let stage = layout.staging()?.join("tx");
            for (slot, build_id) in [
                (layout.current()?, "old"),
                (layout.previous()?, "older"),
                (stage.clone(), "candidate"),
            ] {
                fs::create_dir_all(slot.join("resources"))?;
                write_json_atomically(
                    &slot.join("manifest.json"),
                    &PreparedManifest {
                        schema_version: SCHEMA_VERSION,
                        build_id: build_id.into(),
                        source_commit: build_id.into(),
                        artifacts: vec![artifact("value", build_id.as_bytes())],
                        changes: ManifestChanges::default(),
                    },
                )?;
            }
            let retired = layout.root.join(".slot-previous-retired-tx");
            fs::rename(layout.previous()?, &retired)?;
            let phase = match window {
                0 => SlotRotationPhase::NotStarted,
                1 => {
                    fs::rename(layout.current()?, layout.previous()?)?;
                    SlotRotationPhase::PreviousRetired
                }
                2 => {
                    fs::rename(layout.current()?, layout.previous()?)?;
                    fs::rename(&stage, layout.current()?)?;
                    SlotRotationPhase::CurrentPromoted
                }
                _ => unreachable!(),
            };
            let request = ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "candidate".into(),
                source_commit: "candidate".into(),
                prepared_root: temp.path().join("prepared"),
                app_bundle_path: temp.path().join("App.app"),
                reason: "test".into(),
            };
            let mut transaction = Transaction {
                schema_version: SCHEMA_VERSION,
                request,
                manifest_hash: "manifest".into(),
                artifact_content_hash: "content".into(),
                phase: TransactionPhase::Activating,
                instance_id: None,
                slot_rotated: false,
                slot_rotation_phase: phase,
                slot_restore_phase: SlotRestorePhase::NotStarted,
                slot_retired_path: Some(retired.clone()),
                slot_failed_path: Some(layout.root.join(".slot-candidate-failed-tx")),
                slot_had_previous: Some(true),
                resources_swapped: false,
                resources_backup_path: None,
                resources_replacement_path: None,
                resources_retired_path: None,
                resource_swap_phase: ResourceSwapPhase::NotStarted,
                resource_restore_phase: ResourceRestorePhase::NotStarted,
                signature_backup_path: None,
                signature_backup_ready: Some(false),
                signature_backup_had_code_signature: Some(false),
                signature_restore_phase: SignatureRestorePhase::NotStarted,
                runtime_retired_path: None,
                runtime_replacement_path: None,
                signature_retired_path: None,
                signature_replacement_path: None,
                signature_replacement_ready: Some(false),
                launcher_expected_hash: None,
                rollback_failure_evidence: None,
                post_ready_rollback: None,
                started_at: Utc::now(),
            };
            write_json_atomically(&layout.transaction()?, &transaction)?;
            rotate_candidate_durably(&layout, &mut transaction, &stage, None)?;
            assert_eq!(
                read_json::<PreparedManifest>(&layout.current()?.join("manifest.json"))?.build_id,
                "candidate"
            );
            assert_eq!(
                read_json::<PreparedManifest>(&layout.previous()?.join("manifest.json"))?.build_id,
                "old"
            );
            assert!(retired.exists());
            assert_eq!(
                transaction.slot_rotation_phase,
                SlotRotationPhase::CandidateInstalled
            );
        }
        Ok(())
    }

    fn build_record(build_id: &str, app_bundle: &Path) -> BuildRecord {
        BuildRecord {
            transaction_id: Some(format!("tx-{build_id}")),
            request_id: Some(format!("request-{build_id}")),
            requested_by_thread_id: Some("thread-id".into()),
            mode: ActivationMode::Full,
            build_id: build_id.into(),
            source_commit: format!("commit-{build_id}"),
            manifest_hash: format!("manifest-{build_id}"),
            artifact_content_hash: format!("content-{build_id}"),
            app_bundle_path: app_bundle.to_path_buf(),
            activated_at: Utc::now(),
        }
    }

    fn failure_for_build(build: &BuildRecord) -> FailureEvidence {
        FailureEvidence {
            schema_version: SCHEMA_VERSION,
            recovery_identity: Some(Uuid::new_v4().to_string()),
            launcher_owner_nonce: None,
            occurred_at: Utc::now(),
            transaction_id: build.transaction_id.clone(),
            request_id: build.request_id.clone(),
            requested_by_thread_id: build.requested_by_thread_id.clone(),
            mode: Some(build.mode),
            build_id: Some(build.build_id.clone()),
            source_commit: Some(build.source_commit.clone()),
            manifest_hash: Some(build.manifest_hash.clone()),
            failed_build_hash: Some(build.artifact_content_hash.clone()),
            failure_phase: "post-ready-crash".into(),
            summary: "crash threshold reached".into(),
            reason: None,
            app_bundle_path: Some(build.app_bundle_path.clone()),
            exit_code: Some(1),
            signal: None,
            ready_timeout_ms: Some(READY_TIMEOUT_SECS * 1000),
            log_path: None,
            transaction_path: None,
            recovered_build_id: None,
            claim_id: None,
            acknowledged: false,
        }
    }

    #[test]
    fn rejects_path_escape() {
        assert!(validate_relative_path(Path::new("../outside")).is_err());
        assert!(validate_relative_path(Path::new("/absolute")).is_err());
        assert!(validate_relative_path(Path::new("safe/file")).is_ok());
    }

    #[test]
    fn activation_request_preserves_optional_requester_provenance() -> Result<()> {
        let without_requester = br#"{
            "schemaVersion": 1,
            "transactionId": "tx",
            "requestId": "request",
            "mode": "full",
            "buildId": "build",
            "sourceCommit": "commit",
            "preparedRoot": "/prepared",
            "appBundlePath": "/App.app",
            "reason": "test"
        }"#;
        let null_requester = br#"{
            "schemaVersion": 1,
            "transactionId": "tx",
            "requestId": "request",
            "requestedByThreadId": null,
            "mode": "full",
            "buildId": "build",
            "sourceCommit": "commit",
            "preparedRoot": "/prepared",
            "appBundlePath": "/App.app",
            "reason": "test"
        }"#;
        let string_requester = br#"{
            "schemaVersion": 1,
            "transactionId": "tx",
            "requestId": "request",
            "requestedByThreadId": "thread",
            "mode": "full",
            "buildId": "build",
            "sourceCommit": "commit",
            "preparedRoot": "/prepared",
            "appBundlePath": "/App.app",
            "reason": "test"
        }"#;
        let missing: ActivationRequest = serde_json::from_slice(without_requester)?;
        let null: ActivationRequest = serde_json::from_slice(null_requester)?;
        let present: ActivationRequest = serde_json::from_slice(string_requester)?;
        assert_eq!(missing.requested_by_thread_id, None);
        assert_eq!(null.requested_by_thread_id, None);
        assert_eq!(present.requested_by_thread_id.as_deref(), Some("thread"));
        assert_eq!(
            request_failure(&missing, None, "prepare", "failed").requested_by_thread_id,
            None
        );

        let mut invalid = missing;
        invalid.requested_by_thread_id = Some("  ".into());
        assert!(validate_request(&invalid).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn persisted_cleanup_paths_cannot_escape_derived_transaction_layout() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app_bundle = temp.path().join("App.app");
        fs::create_dir_all(app_bundle.join("Contents"))?;
        let external = temp.path().join("external-sentinel");
        fs::create_dir_all(&external)?;
        fs::write(external.join("value"), b"keep")?;
        let mut base = transaction_for_path_binding(&layout, &app_bundle);
        bind_transaction_paths(&layout, &mut base)?;

        for field in [
            "slotRetiredPath",
            "slotFailedPath",
            "resourcesBackupPath",
            "resourcesReplacementPath",
            "resourcesRetiredPath",
            "signatureBackupPath",
            "runtimeRetiredPath",
            "runtimeReplacementPath",
            "signatureRetiredPath",
            "signatureReplacementPath",
            "postReadyRollback.failedSlotPath",
            "postReadyRollback.resourcesReplacementPath",
            "postReadyRollback.resourcesBackupPath",
        ] {
            let mut transaction = base.clone();
            match field {
                "slotRetiredPath" => transaction.slot_retired_path = Some(external.clone()),
                "slotFailedPath" => transaction.slot_failed_path = Some(external.clone()),
                "resourcesBackupPath" => transaction.resources_backup_path = Some(external.clone()),
                "resourcesReplacementPath" => {
                    transaction.resources_replacement_path = Some(external.clone())
                }
                "resourcesRetiredPath" => {
                    transaction.resources_retired_path = Some(external.clone())
                }
                "signatureBackupPath" => transaction.signature_backup_path = Some(external.clone()),
                "runtimeRetiredPath" => transaction.runtime_retired_path = Some(external.clone()),
                "runtimeReplacementPath" => {
                    transaction.runtime_replacement_path = Some(external.clone())
                }
                "signatureRetiredPath" => {
                    transaction.signature_retired_path = Some(external.clone())
                }
                "signatureReplacementPath" => {
                    transaction.signature_replacement_path = Some(external.clone())
                }
                "postReadyRollback.failedSlotPath" => {
                    transaction
                        .post_ready_rollback
                        .as_mut()
                        .expect("test rollback")
                        .failed_slot_path = external.clone();
                }
                "postReadyRollback.resourcesReplacementPath" => {
                    transaction
                        .post_ready_rollback
                        .as_mut()
                        .expect("test rollback")
                        .resources_replacement_path = external.clone();
                }
                "postReadyRollback.resourcesBackupPath" => {
                    transaction
                        .post_ready_rollback
                        .as_mut()
                        .expect("test rollback")
                        .resources_backup_path = external.clone();
                }
                _ => unreachable!(),
            }
            assert!(
                commit_activation_files(&layout, &mut transaction).is_err(),
                "{field} should be rejected"
            );
            assert_eq!(fs::read(external.join("value"))?, b"keep");
        }
        Ok(())
    }

    #[test]
    fn state_round_trips_atomically() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().to_path_buf());
        let mut state = RuntimeState::default();
        state.blocked_build_hashes.insert("blocked".into());
        save_state(&layout, &state)?;
        assert_eq!(load_state(&layout)?, state);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn staging_gc_is_bounded_and_preserves_active_transaction() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        fs::create_dir_all(layout.staging()?)?;
        for name in ["a", "b", "c", "active"] {
            fs::create_dir(layout.staging()?.join(name))?;
            mark_owned(&layout, &layout.staging()?.join(name), name, "prepared")?;
        }
        gc_staging(&layout, Some("active"), 2)?;
        let remaining = fs::read_dir(layout.staging()?)?.count();
        assert_eq!(remaining, 3);
        assert!(layout.staging()?.join("active").exists());
        Ok(())
    }

    #[test]
    fn validates_hash_and_manifest_identity() -> Result<()> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join("resources/Contents"))?;
        fs::write(temp.path().join("resources/Contents/value"), b"candidate")?;
        let manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "build-1".into(),
            source_commit: "abc".into(),
            artifacts: vec![artifact("Contents/value", b"candidate")],
            changes: ManifestChanges::default(),
        };
        write_json_atomically(&temp.path().join("manifest.json"), &manifest)?;
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Full,
            build_id: "build-1".into(),
            source_commit: "abc".into(),
            prepared_root: temp.path().to_path_buf(),
            app_bundle_path: temp.path().join("App.app"),
            reason: "test".into(),
        };
        assert!(load_and_validate_manifest(&request).is_ok());
        fs::write(temp.path().join("resources/Contents/value"), b"tampered")?;
        assert!(load_and_validate_manifest(&request).is_err());
        Ok(())
    }

    #[test]
    fn manifest_cannot_target_stable_launcher() -> Result<()> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join("resources/bin"))?;
        fs::write(
            temp.path().join("resources/bin/MorpheusLauncher"),
            b"replacement",
        )?;
        let manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "build-1".into(),
            source_commit: "abc".into(),
            artifacts: vec![artifact("bin/MorpheusLauncher", b"replacement")],
            changes: ManifestChanges::default(),
        };
        write_json_atomically(&temp.path().join("manifest.json"), &manifest)?;
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Full,
            build_id: "build-1".into(),
            source_commit: "abc".into(),
            prepared_root: temp.path().to_path_buf(),
            app_bundle_path: temp.path().join("App.app"),
            reason: "test".into(),
        };
        assert!(load_and_validate_manifest(&request).is_err());
        Ok(())
    }

    #[test]
    fn staging_overlays_resources_root_relative_artifacts() -> Result<()> {
        let temp = TempDir::new()?;
        let app = temp.path().join("App.app");
        fs::create_dir_all(app.join("Contents/Resources/bin"))?;
        fs::create_dir_all(app.join("Contents/Resources/default-config"))?;
        fs::write(app.join("Contents/Resources/app.asar"), b"old-asar")?;
        fs::write(app.join("Contents/Resources/bin/app-server"), b"old-server")?;
        fs::write(
            app.join("Contents/Resources/default-config/prompt.md"),
            b"old-prompt",
        )?;
        let prepared = temp.path().join("prepared");
        fs::create_dir_all(prepared.join("resources/bin"))?;
        fs::create_dir_all(prepared.join("resources/default-config"))?;
        fs::write(prepared.join("resources/app.asar"), b"new-asar")?;
        fs::write(prepared.join("resources/bin/app-server"), b"new-server")?;
        fs::write(
            prepared.join("resources/default-config/prompt.md"),
            b"new-prompt",
        )?;
        let manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "build".into(),
            source_commit: "commit".into(),
            artifacts: vec![
                artifact("app.asar", b"new-asar"),
                PreparedArtifact {
                    relative_path: "bin/app-server".into(),
                    sha256: hex_digest(b"new-server"),
                    kind: "executable".into(),
                },
                artifact("default-config/prompt.md", b"new-prompt"),
            ],
            changes: ManifestChanges::default(),
        };
        write_json_atomically(&prepared.join("manifest.json"), &manifest)?;
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Full,
            build_id: "build".into(),
            source_commit: "commit".into(),
            prepared_root: prepared,
            app_bundle_path: app,
            reason: "test".into(),
        };
        let layout = Layout::new(temp.path().join("launcher"));
        let stage = stage_candidate(&layout, &request, &manifest)?;
        assert_eq!(fs::read(stage.join("resources/app.asar"))?, b"new-asar");
        assert_eq!(
            fs::read(stage.join("resources/bin/app-server"))?,
            b"new-server"
        );
        assert_eq!(
            fs::read(stage.join("resources/default-config/prompt.md"))?,
            b"new-prompt"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn staging_rejects_symlinked_root_and_staging_without_touching_external_data() -> Result<()> {
        use std::os::unix::fs::symlink;

        for symlink_root in [true, false] {
            let temp = TempDir::new()?;
            let external = temp.path().join("external");
            fs::create_dir_all(&external)?;
            fs::write(external.join("sentinel"), b"keep")?;
            let layout = Layout::new(temp.path().join("launcher"));
            if symlink_root {
                symlink(&external, &layout.root)?;
            } else {
                fs::create_dir_all(&layout.root)?;
                symlink(&external, layout.staging()?)?;
            }
            let app = temp.path().join("App.app");
            fs::create_dir_all(app.join("Contents/Resources"))?;
            fs::write(app.join("Contents/Resources/value"), b"old")?;
            let prepared = temp.path().join("prepared");
            fs::create_dir_all(prepared.join("resources"))?;
            fs::write(prepared.join("resources/value"), b"new")?;
            let manifest = PreparedManifest {
                schema_version: SCHEMA_VERSION,
                build_id: "build".into(),
                source_commit: "commit".into(),
                artifacts: vec![artifact("value", b"new")],
                changes: ManifestChanges::default(),
            };
            let request = ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "build".into(),
                source_commit: "commit".into(),
                prepared_root: prepared,
                app_bundle_path: app,
                reason: "test".into(),
            };

            assert!(stage_candidate(&layout, &request, &manifest).is_err());
            assert_eq!(fs::read(external.join("sentinel"))?, b"keep");
            assert!(!external.join("tx").exists());
            assert!(!external.read_dir()?.any(|entry| entry.is_ok_and(|entry| {
                entry.file_name().to_string_lossy().starts_with(".prepare-")
            })));
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn staging_rejects_preplanted_transaction_entries() -> Result<()> {
        use std::os::unix::fs::symlink;

        for preplant_symlink in [false, true] {
            let temp = TempDir::new()?;
            let layout = Layout::new(temp.path().join("launcher"));
            fs::create_dir_all(layout.staging()?)?;
            let external = temp.path().join("external");
            fs::create_dir_all(&external)?;
            fs::write(external.join("sentinel"), b"keep")?;
            let stage = layout.staging()?.join("tx");
            if preplant_symlink {
                symlink(&external, &stage)?;
            } else {
                fs::create_dir_all(&stage)?;
                fs::write(stage.join("sentinel"), b"preexisting")?;
            }
            let app = temp.path().join("App.app");
            fs::create_dir_all(app.join("Contents/Resources"))?;
            fs::write(app.join("Contents/Resources/value"), b"old")?;
            let prepared = temp.path().join("prepared");
            fs::create_dir_all(prepared.join("resources"))?;
            fs::write(prepared.join("resources/value"), b"new")?;
            let manifest = PreparedManifest {
                schema_version: SCHEMA_VERSION,
                build_id: "build".into(),
                source_commit: "commit".into(),
                artifacts: vec![artifact("value", b"new")],
                changes: ManifestChanges::default(),
            };
            let request = ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "build".into(),
                source_commit: "commit".into(),
                prepared_root: prepared,
                app_bundle_path: app,
                reason: "test".into(),
            };

            assert!(stage_candidate(&layout, &request, &manifest).is_err());
            assert_eq!(fs::read(external.join("sentinel"))?, b"keep");
            if !preplant_symlink {
                assert_eq!(fs::read(stage.join("sentinel"))?, b"preexisting");
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn staging_prepare_operations_remain_anchored_after_path_replacement() -> Result<()> {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let staging = TrustedStaging::open(&layout, true)?;
        let temp_name = ".prepare-test";
        let child = staging.create_private_child(temp_name)?;
        let original_staging = layout.root.join("original-staging");
        fs::rename(layout.staging()?, &original_staging)?;
        let external = temp.path().join("external");
        fs::create_dir_all(&external)?;
        fs::write(external.join("sentinel"), b"keep")?;
        symlink(&external, layout.staging()?)?;

        fs::write(child.anchored_path()?.join("value"), b"candidate")?;
        staging.publish_child(temp_name, "tx", &child.file)?;
        assert_eq!(fs::read(original_staging.join("tx/value"))?, b"candidate");
        assert_eq!(fs::read(external.join("sentinel"))?, b"keep");
        assert!(!external.join("tx").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn trusted_root_fd_remains_authoritative_after_root_path_replacement() -> Result<()> {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let authority = TrustedStaging::open(&layout, true)?;
        let original_root = temp.path().join("original-launcher-root");
        fs::rename(&layout.root, &original_root)?;
        let external = temp.path().join("external-root");
        fs::create_dir_all(&external)?;
        fs::write(external.join("sentinel"), b"keep")?;
        symlink(&external, &layout.root)?;

        let child = authority.create_private_root_child(".activation-snapshot-tx")?;
        fs::write(child.anchored_path()?.join("value"), b"candidate")?;
        assert_eq!(
            fs::read(original_root.join(".activation-snapshot-tx/value"))?,
            b"candidate"
        );
        assert_eq!(fs::read(external.join("sentinel"))?, b"keep");
        assert!(!external.join(".activation-snapshot-tx").exists());
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn unlinked_directory_handle_replacement_fails_closed_before_copy_mutation() -> Result<()> {
        let temp = TempDir::new()?;
        let source_path = temp.path().join("source");
        let destination_path = temp.path().join("destination");
        fs::create_dir_all(&source_path)?;
        fs::create_dir_all(&destination_path)?;
        fs::write(source_path.join("value"), b"source")?;
        fs::write(destination_path.join("sentinel"), b"keep")?;
        let source = open_directory_path_no_follow(&source_path)?;
        let destination = open_directory_path_no_follow(&destination_path)?;
        fs::remove_dir_all(&source_path)?;
        fs::create_dir_all(&source_path)?;
        fs::write(source_path.join("value"), b"replacement")?;

        assert!(directory_fd_path(&source).is_err());
        assert!(copy_directory_handles(&source, &destination).is_err());
        assert_eq!(fs::read(destination_path.join("sentinel"))?, b"keep");
        assert!(!destination_path.join("value").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn trusted_root_identity_rejects_replacement_across_reopen() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        drop(TrustedStaging::open(&layout, true)?);
        let original_root = temp.path().join("original-launcher-root");
        fs::rename(&layout.root, &original_root)?;
        fs::create_dir_all(&layout.root)?;
        fs::write(layout.root.join("sentinel"), b"replacement")?;

        assert!(TrustedStaging::open(&layout, true).is_err());
        assert_eq!(fs::read(layout.root.join("sentinel"))?, b"replacement");
        assert!(!layout.staging()?.exists());
        assert!(original_root.join("staging").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn staging_copy_rejects_internal_resource_symlink_without_external_write() -> Result<()> {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        fs::create_dir_all(app.join("Contents/Resources"))?;
        let external = temp.path().join("external-resources");
        fs::create_dir_all(&external)?;
        fs::write(external.join("sentinel"), b"keep")?;
        symlink(&external, app.join("Contents/Resources/nested"))?;
        let prepared = temp.path().join("prepared");
        fs::create_dir_all(prepared.join("resources"))?;
        fs::write(prepared.join("resources/value"), b"new")?;
        let manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "build".into(),
            source_commit: "commit".into(),
            artifacts: vec![artifact("value", b"new")],
            changes: ManifestChanges::default(),
        };
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Full,
            build_id: "build".into(),
            source_commit: "commit".into(),
            prepared_root: prepared,
            app_bundle_path: app,
            reason: "test".into(),
        };

        assert!(stage_candidate(&layout, &request, &manifest).is_err());
        assert_eq!(fs::read(external.join("sentinel"))?, b"keep");
        assert_eq!(external.read_dir()?.count(), 1);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn claimed_stage_is_resumable_and_cleanup_is_idempotent() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        fs::create_dir_all(layout.staging()?.join("tx"))?;
        fs::write(layout.staging()?.join("tx/sentinel"), b"candidate")?;
        mark_owned(&layout, &layout.staging()?.join("tx"), "tx", "prepared")?;

        let (_authority, claimed) = claim_staging_child(&layout, "tx")?;
        assert_eq!(
            fs::read(claimed.anchored_path()?.join("sentinel"))?,
            b"candidate"
        );
        let resolved = resolve_staging_child(&layout, "tx", true)?;
        let expected = layout.root.join(claimed_stage_name("tx"));
        assert_eq!(resolved.canonicalize()?, expected.canonicalize()?);
        assert!(remove_staging_child(&layout, "tx")?);
        assert!(!remove_staging_child(&layout, "tx")?);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn claim_rejects_forged_published_owner_nonce_before_moving_entry() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        fs::create_dir_all(layout.staging()?.join("tx"))?;
        fs::write(layout.staging()?.join("tx/sentinel"), b"keep")?;
        let published = open_directory_path_no_follow(&layout.staging()?.join("tx"))?;
        write_owned_marker(&published, "tx", "prepared", &Uuid::new_v4().to_string())?;

        assert!(claim_staging_child(&layout, "tx").is_err());
        assert_eq!(fs::read(layout.staging()?.join("tx/sentinel"))?, b"keep");
        assert!(!layout.root.join(claimed_stage_name("tx")).exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn claim_rejects_forged_claimed_owner_nonce_without_removing_entry() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let claimed = layout.root.join(claimed_stage_name("tx"));
        fs::create_dir_all(&claimed)?;
        fs::write(claimed.join("sentinel"), b"keep")?;
        let claimed_directory = open_directory_path_no_follow(&claimed)?;
        write_owned_marker(
            &claimed_directory,
            "tx",
            "prepared",
            &Uuid::new_v4().to_string(),
        )?;

        assert!(claim_staging_child(&layout, "tx").is_err());
        assert_eq!(fs::read(claimed.join("sentinel"))?, b"keep");
        assert!(!layout.staging()?.join("tx").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn staging_gc_removes_owned_orphans_and_preserves_active_claims() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let prepare_orphan = format!(".prepare-{}", Uuid::new_v4());
        let quarantine_orphan = format!(".remove-{}", Uuid::new_v4());
        fs::create_dir_all(layout.staging()?.join(&prepare_orphan))?;
        fs::create_dir_all(layout.root.join(".activation-stage-orphan"))?;
        fs::create_dir_all(layout.root.join(".activation-snapshot-orphan"))?;
        fs::create_dir_all(layout.root.join(&quarantine_orphan))?;
        fs::create_dir_all(layout.root.join(".activation-stage-active"))?;
        fs::create_dir_all(layout.root.join(".activation-snapshot-active"))?;
        mark_owned(
            &layout,
            &layout.staging()?.join(&prepare_orphan),
            "orphan",
            "prepared",
        )?;
        mark_owned(
            &layout,
            &layout.root.join(".activation-stage-orphan"),
            "orphan",
            "prepared",
        )?;
        mark_owned(
            &layout,
            &layout.root.join(".activation-snapshot-orphan"),
            "orphan",
            "snapshot",
        )?;
        mark_owned(
            &layout,
            &layout.root.join(&quarantine_orphan),
            "orphan",
            "prepared",
        )?;
        mark_owned(
            &layout,
            &layout.root.join(".activation-stage-active"),
            "active",
            "prepared",
        )?;
        mark_owned(
            &layout,
            &layout.root.join(".activation-snapshot-active"),
            "active",
            "snapshot",
        )?;
        let unowned_prepare = layout
            .staging()?
            .join(format!(".prepare-{}", Uuid::new_v4()));
        let forged_prepare = layout
            .staging()?
            .join(format!(".prepare-{}", Uuid::new_v4()));
        let unowned_claim = layout.root.join(".activation-stage-preplanted");
        let unowned_snapshot = layout.root.join(".activation-snapshot-preplanted");
        let unowned_quarantine = layout.root.join(format!(".remove-{}", Uuid::new_v4()));
        for path in [
            &unowned_prepare,
            &forged_prepare,
            &unowned_claim,
            &unowned_snapshot,
            &unowned_quarantine,
        ] {
            fs::create_dir_all(path)?;
            fs::write(path.join("sentinel"), b"keep")?;
        }
        let forged_directory = open_directory_path_no_follow(&forged_prepare)?;
        write_owned_marker(
            &forged_directory,
            "forged",
            "prepared",
            &Uuid::new_v4().to_string(),
        )?;

        gc_staging(&layout, Some("active"), 4)?;
        assert!(!layout.staging()?.join(&prepare_orphan).exists());
        assert!(!layout.root.join(".activation-stage-orphan").exists());
        assert!(!layout.root.join(".activation-snapshot-orphan").exists());
        assert!(!layout.root.join(&quarantine_orphan).exists());
        assert!(layout.root.join(".activation-stage-active").exists());
        assert!(layout.root.join(".activation-snapshot-active").exists());
        for path in [
            &unowned_prepare,
            &forged_prepare,
            &unowned_claim,
            &unowned_snapshot,
            &unowned_quarantine,
        ] {
            assert_eq!(fs::read(path.join("sentinel"))?, b"keep");
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn claimed_stage_install_rejects_replaced_directory_inode() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        fs::create_dir_all(layout.staging()?.join("tx"))?;
        fs::write(layout.staging()?.join("tx/sentinel"), b"original")?;
        mark_owned(&layout, &layout.staging()?.join("tx"), "tx", "prepared")?;
        let (authority, claimed) = claim_staging_child(&layout, "tx")?;
        let displaced = layout.root.join(".displaced-claim");
        fs::rename(&claimed.path, &displaced)?;
        fs::create_dir_all(&claimed.path)?;
        fs::write(claimed.path.join("sentinel"), b"replacement")?;

        assert!(
            install_claimed_stage_as_current(&authority, &claimed_stage_name("tx"), &claimed,)
                .is_err()
        );
        assert!(!layout.current()?.exists());
        assert_eq!(fs::read(displaced.join("sentinel"))?, b"original");
        assert_eq!(fs::read(claimed.path.join("sentinel"))?, b"replacement");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rotation_keeps_one_previous_slot() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().to_path_buf());
        fs::create_dir_all(layout.current()?)?;
        fs::write(layout.current()?.join("id"), "old")?;
        let stage = layout.staging()?.join("new");
        fs::create_dir_all(&stage)?;
        fs::write(stage.join("id"), "new")?;
        write_json_atomically(
            &stage.join("manifest.json"),
            &PreparedManifest {
                schema_version: SCHEMA_VERSION,
                build_id: "new".into(),
                source_commit: "new".into(),
                artifacts: Vec::new(),
                changes: ManifestChanges::default(),
            },
        )?;
        let mut transaction = transaction_for_path_binding(&layout, &temp.path().join("App.app"));
        transaction.request.transaction_id = "rotation".into();
        transaction.request.build_id = "new".into();
        transaction.request.source_commit = "new".into();
        transaction.phase = TransactionPhase::Activating;
        transaction.slot_retired_path = Some(layout.root.join(".slot-previous-retired-rotation"));
        transaction.slot_had_previous = Some(false);
        transaction.post_ready_rollback = None;
        write_json_atomically(&layout.transaction()?, &transaction)?;

        rotate_candidate_durably(&layout, &mut transaction, &stage, None)?;

        assert_eq!(fs::read_to_string(layout.current()?.join("id"))?, "new");
        assert_eq!(fs::read_to_string(layout.previous()?.join("id"))?, "old");
        assert_eq!(
            transaction.slot_rotation_phase,
            SlotRotationPhase::CandidateInstalled
        );
        Ok(())
    }

    #[test]
    fn initial_bootstrap_snapshots_resources_and_runtime_executable() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        fs::create_dir_all(app.join("Contents/Resources/nested"))?;
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::write(app.join("Contents/Resources/nested/value"), b"resource")?;
        fs::write(
            app.join("Contents/MacOS/Root Worker Runtime"),
            b"executable",
        )?;
        let record = bootstrap_current_slot(&layout, &app)?
            .context("bootstrap should create a baseline record")?;
        assert_eq!(record.build_id, "installed-baseline");
        assert_eq!(
            fs::read(layout.current()?.join("resources/nested/value"))?,
            b"resource"
        );
        Ok(())
    }

    #[test]
    fn crash_policy_uses_both_windows() -> Result<()> {
        let now = Utc
            .with_ymd_and_hms(2026, 9, 8, 12, 0, 0)
            .single()
            .context("valid test time")?;
        let mut state = RuntimeState::default();
        assert_eq!(
            record_crash(&mut state, "hash", now),
            CrashDecision::RestartCurrent
        );
        assert_eq!(
            record_crash(&mut state, "hash", now + Duration::seconds(20)),
            CrashDecision::RollBack
        );
        state.crash_history.clear();
        assert_eq!(
            record_crash(&mut state, "hash", now),
            CrashDecision::RestartCurrent
        );
        assert_eq!(
            record_crash(&mut state, "hash", now + Duration::minutes(2)),
            CrashDecision::RestartCurrent
        );
        assert_eq!(
            record_crash(&mut state, "hash", now + Duration::minutes(4)),
            CrashDecision::RollBack
        );
        Ok(())
    }

    #[test]
    fn ready_identity_must_match_all_fields() {
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Full,
            build_id: "build".into(),
            source_commit: "commit".into(),
            prepared_root: PathBuf::from("/prepared"),
            app_bundle_path: PathBuf::from("/App.app"),
            reason: "test".into(),
        };
        let transaction = Transaction {
            schema_version: SCHEMA_VERSION,
            request,
            manifest_hash: "hash".into(),
            artifact_content_hash: "content-hash".into(),
            phase: TransactionPhase::CandidateStarted,
            instance_id: Some("instance".into()),
            slot_rotated: false,
            slot_rotation_phase: SlotRotationPhase::NotStarted,
            slot_restore_phase: SlotRestorePhase::NotStarted,
            slot_retired_path: None,
            slot_failed_path: None,
            slot_had_previous: None,
            resources_swapped: false,
            resources_backup_path: None,
            resources_replacement_path: None,
            resources_retired_path: None,
            resource_swap_phase: ResourceSwapPhase::NotStarted,
            resource_restore_phase: ResourceRestorePhase::NotStarted,
            signature_backup_path: None,
            signature_backup_ready: Some(false),
            signature_backup_had_code_signature: Some(false),
            signature_restore_phase: SignatureRestorePhase::NotStarted,
            runtime_retired_path: None,
            runtime_replacement_path: None,
            signature_retired_path: None,
            signature_replacement_path: None,
            signature_replacement_ready: Some(false),
            launcher_expected_hash: None,
            rollback_failure_evidence: None,
            post_ready_rollback: None,
            started_at: Utc::now(),
        };
        let ready = ReadyIdentity {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            build_id: "build".into(),
            instance_id: "instance".into(),
            ready_at_ms: 1,
        };
        assert!(ready_matches(&ready, &transaction, "instance"));
        assert!(!ready_matches(&ready, &transaction, "other"));
    }

    #[test]
    fn interrupted_installed_transaction_restores_previous() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        fs::create_dir_all(app.join("Contents/Resources"))?;
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::write(app.join("Contents/Resources/value"), b"bad")?;
        fs::write(app.join("Contents/MacOS/Root Worker Runtime"), b"runtime")?;
        let resources_backup = temp.path().join(".MorpheusResourcesPrevious-tx");
        fs::create_dir_all(&resources_backup)?;
        fs::write(resources_backup.join("value"), b"good")?;
        for (slot, value) in [
            (layout.current()?, &b"bad"[..]),
            (layout.previous()?, &b"good"[..]),
        ] {
            fs::create_dir_all(slot.join("resources"))?;
            fs::write(slot.join("resources/value"), value)?;
            let manifest = PreparedManifest {
                schema_version: SCHEMA_VERSION,
                build_id: String::from_utf8_lossy(value).into_owned(),
                source_commit: "commit".into(),
                artifacts: vec![artifact("value", value)],
                changes: ManifestChanges::default(),
            };
            write_json_atomically(&slot.join("manifest.json"), &manifest)?;
        }
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Full,
            build_id: "bad".into(),
            source_commit: "commit".into(),
            prepared_root: temp.path().join("prepared"),
            app_bundle_path: app.clone(),
            reason: "test".into(),
        };
        write_json_atomically(
            &layout.transaction()?,
            &Transaction {
                schema_version: SCHEMA_VERSION,
                request,
                manifest_hash: "bad-hash".into(),
                artifact_content_hash: "bad-content-hash".into(),
                phase: TransactionPhase::CandidateInstalled,
                instance_id: None,
                slot_rotated: true,
                slot_rotation_phase: SlotRotationPhase::CandidateInstalled,
                slot_restore_phase: SlotRestorePhase::NotStarted,
                slot_retired_path: None,
                slot_failed_path: None,
                slot_had_previous: None,
                resources_swapped: true,
                resources_backup_path: Some(resources_backup),
                resources_replacement_path: None,
                resources_retired_path: None,
                resource_swap_phase: ResourceSwapPhase::CandidateInstalled,
                resource_restore_phase: ResourceRestorePhase::NotStarted,
                signature_backup_path: None,
                signature_backup_ready: Some(false),
                signature_backup_had_code_signature: Some(false),
                signature_restore_phase: SignatureRestorePhase::NotStarted,
                runtime_retired_path: None,
                runtime_replacement_path: None,
                signature_retired_path: None,
                signature_replacement_path: None,
                signature_replacement_ready: Some(false),
                launcher_expected_hash: None,
                rollback_failure_evidence: None,
                post_ready_rollback: None,
                started_at: Utc::now(),
            },
        )?;
        recover_interrupted_transaction(&layout)?;
        assert_eq!(fs::read(app.join("Contents/Resources/value"))?, b"good");
        assert_eq!(
            fs::read(layout.current()?.join("resources/value"))?,
            b"good"
        );
        assert!(!layout.transaction()?.exists());
        Ok(())
    }

    #[test]
    fn interrupted_activating_before_slot_rotation_keeps_current() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        fs::create_dir_all(app.join("Contents/Resources"))?;
        fs::write(app.join("Contents/Resources/value"), b"good")?;
        fs::create_dir_all(layout.current()?.join("resources"))?;
        fs::write(layout.current()?.join("resources/value"), b"good")?;
        let manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "good".into(),
            source_commit: "commit".into(),
            artifacts: vec![artifact("value", b"good")],
            changes: ManifestChanges::default(),
        };
        write_json_atomically(&layout.current()?.join("manifest.json"), &manifest)?;
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Full,
            build_id: "candidate".into(),
            source_commit: "commit".into(),
            prepared_root: temp.path().join("prepared"),
            app_bundle_path: app,
            reason: "test".into(),
        };
        write_json_atomically(
            &layout.transaction()?,
            &Transaction {
                schema_version: SCHEMA_VERSION,
                request,
                manifest_hash: "hash".into(),
                artifact_content_hash: "content-hash".into(),
                phase: TransactionPhase::Activating,
                instance_id: None,
                slot_rotated: false,
                slot_rotation_phase: SlotRotationPhase::NotStarted,
                slot_restore_phase: SlotRestorePhase::NotStarted,
                slot_retired_path: None,
                slot_failed_path: None,
                slot_had_previous: None,
                resources_swapped: false,
                resources_backup_path: None,
                resources_replacement_path: None,
                resources_retired_path: None,
                resource_swap_phase: ResourceSwapPhase::NotStarted,
                resource_restore_phase: ResourceRestorePhase::NotStarted,
                signature_backup_path: None,
                signature_backup_ready: Some(false),
                signature_backup_had_code_signature: Some(false),
                signature_restore_phase: SignatureRestorePhase::NotStarted,
                runtime_retired_path: None,
                runtime_replacement_path: None,
                signature_retired_path: None,
                signature_replacement_path: None,
                signature_replacement_ready: Some(false),
                launcher_expected_hash: None,
                rollback_failure_evidence: None,
                post_ready_rollback: None,
                started_at: Utc::now(),
            },
        )?;
        recover_interrupted_transaction(&layout)?;
        assert_eq!(
            fs::read(layout.current()?.join("resources/value"))?,
            b"good"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn resource_swap_and_restore_rename_windows_are_recoverable() -> Result<()> {
        use std::os::unix::process::ExitStatusExt;

        struct SuccessfulRunner;
        impl CommandRunner for SuccessfulRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(0))
            }
        }

        for window in 0..4 {
            let temp = TempDir::new()?;
            let layout = Layout::new(temp.path().join("launcher"));
            let app = temp.path().join("App.app");
            let contents = app.join("Contents");
            let resources = contents.join("Resources");
            let replacement = contents.join(".MorpheusResourcesCandidate-tx");
            let backup = temp.path().join(".MorpheusResourcesPrevious-tx");
            let retired = temp.path().join(".MorpheusResourcesFailed-tx");
            fs::create_dir_all(&contents)?;
            fs::create_dir_all(contents.join("MacOS"))?;
            fs::write(contents.join("MacOS/MorpheusLauncher"), b"stable-launcher")?;
            let launcher_hash = hash_file(&contents.join("MacOS/MorpheusLauncher"))?;

            match window {
                0 => {
                    fs::create_dir_all(&backup)?;
                    fs::write(backup.join("value"), b"previous")?;
                    fs::create_dir_all(&replacement)?;
                    fs::write(replacement.join("value"), b"candidate")?;
                }
                1 => {
                    fs::create_dir_all(&resources)?;
                    fs::write(resources.join("value"), b"candidate")?;
                    fs::create_dir_all(&backup)?;
                    fs::write(backup.join("value"), b"previous")?;
                }
                2 => {
                    fs::create_dir_all(&backup)?;
                    fs::write(backup.join("value"), b"previous")?;
                    fs::create_dir_all(&retired)?;
                    fs::write(retired.join("value"), b"candidate")?;
                }
                3 => {
                    fs::create_dir_all(&resources)?;
                    fs::write(resources.join("value"), b"previous")?;
                    fs::create_dir_all(&retired)?;
                    fs::write(retired.join("value"), b"candidate")?;
                }
                _ => unreachable!(),
            }

            for (slot, build_id, value) in [
                (layout.current()?, "candidate", b"candidate".as_slice()),
                (layout.previous()?, "previous", b"previous".as_slice()),
            ] {
                fs::create_dir_all(slot.join("resources"))?;
                fs::write(slot.join("resources/value"), value)?;
                write_json_atomically(
                    &slot.join("manifest.json"),
                    &PreparedManifest {
                        schema_version: SCHEMA_VERSION,
                        build_id: build_id.into(),
                        source_commit: "commit".into(),
                        artifacts: vec![artifact("value", value)],
                        changes: ManifestChanges::default(),
                    },
                )?;
            }
            let request = ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "candidate".into(),
                source_commit: "commit".into(),
                prepared_root: temp.path().join("prepared"),
                app_bundle_path: app,
                reason: "test".into(),
            };
            let transaction = Transaction {
                schema_version: SCHEMA_VERSION,
                request,
                manifest_hash: "candidate-manifest".into(),
                artifact_content_hash: "candidate-content".into(),
                phase: if window < 2 {
                    TransactionPhase::Activating
                } else {
                    TransactionPhase::RollingBack
                },
                instance_id: None,
                slot_rotated: true,
                slot_rotation_phase: SlotRotationPhase::CandidateInstalled,
                slot_restore_phase: SlotRestorePhase::NotStarted,
                slot_retired_path: None,
                slot_failed_path: None,
                slot_had_previous: None,
                resources_swapped: window != 0,
                resources_backup_path: Some(backup),
                resources_replacement_path: Some(replacement),
                resources_retired_path: Some(retired),
                resource_swap_phase: if window == 0 {
                    ResourceSwapPhase::ReplacementPrepared
                } else {
                    ResourceSwapPhase::CandidateInstalled
                },
                resource_restore_phase: if window == 3 {
                    ResourceRestorePhase::DestinationRetired
                } else {
                    ResourceRestorePhase::NotStarted
                },
                signature_backup_path: None,
                signature_backup_ready: Some(false),
                signature_backup_had_code_signature: Some(false),
                signature_restore_phase: SignatureRestorePhase::NotStarted,
                runtime_retired_path: None,
                runtime_replacement_path: None,
                signature_retired_path: None,
                signature_replacement_path: None,
                signature_replacement_ready: Some(false),
                launcher_expected_hash: Some(launcher_hash),
                rollback_failure_evidence: None,
                post_ready_rollback: None,
                started_at: Utc::now(),
            };
            write_json_atomically(&layout.transaction()?, &transaction)?;
            let recovered = recover_interrupted_transaction_with(&layout, &SuccessfulRunner)?
                .context("resource rollback should retain its durable checkpoint")?;
            assert_eq!(fs::read(resources.join("value"))?, b"previous");
            assert_eq!(
                fs::read(layout.current()?.join("resources/value"))?,
                b"previous"
            );
            assert_eq!(recovered.phase, TransactionPhase::RollbackComplete);
            let recovered_identity = recovered
                .rollback_failure_evidence
                .as_ref()
                .and_then(|evidence| evidence.recovery_identity.as_deref())
                .context("recovered rollback should retain durable failure identity")?;
            let persisted: Transaction = read_json(&layout.transaction()?)?;
            assert_eq!(persisted.phase, TransactionPhase::RollbackComplete);
            assert_eq!(
                persisted
                    .rollback_failure_evidence
                    .as_ref()
                    .and_then(|evidence| evidence.recovery_identity.as_deref()),
                Some(recovered_identity)
            );
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn codesign_failure_is_reported_by_injected_runner() {
        use std::os::unix::process::ExitStatusExt;

        struct FailingRunner;
        impl CommandRunner for FailingRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(1 << 8))
            }
        }

        let manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "build".into(),
            source_commit: "commit".into(),
            artifacts: Vec::new(),
            changes: ManifestChanges::default(),
        };
        assert!(sign_app_bundle_with(&FailingRunner, Path::new("/App.app"), &manifest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn codesign_orders_resource_executables_before_bundle_and_verify() -> Result<()> {
        use std::cell::RefCell;
        use std::os::unix::process::ExitStatusExt;

        struct RecordingRunner {
            calls: RefCell<Vec<Vec<String>>>,
        }
        impl CommandRunner for RecordingRunner {
            fn run(&self, program: &str, args: &[&OsStr]) -> Result<ExitStatus> {
                let mut call = vec![program.to_string()];
                call.extend(args.iter().map(|arg| arg.to_string_lossy().into_owned()));
                self.calls.borrow_mut().push(call);
                Ok(ExitStatus::from_raw(0))
            }
        }

        let runner = RecordingRunner {
            calls: RefCell::new(Vec::new()),
        };
        let manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "build".into(),
            source_commit: "commit".into(),
            artifacts: vec![PreparedArtifact {
                relative_path: "bin/app-server".into(),
                sha256: "0".repeat(64),
                kind: "executable".into(),
            }],
            changes: ManifestChanges::default(),
        };
        sign_app_bundle_with(&runner, Path::new("/App.app"), &manifest)?;
        let calls = runner.calls.into_inner();
        assert_eq!(calls.len(), 3);
        assert!(
            calls[0]
                .last()
                .is_some_and(|arg| arg.ends_with("Contents/Resources/bin/app-server"))
        );
        assert_eq!(calls[1].last().map(String::as_str), Some("/App.app"));
        assert!(calls[2].iter().any(|arg| arg == "--verify"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn activation_preserves_stable_launcher_executable_contents() -> Result<()> {
        use std::os::unix::process::ExitStatusExt;

        struct SuccessfulRunner;
        impl CommandRunner for SuccessfulRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(0))
            }
        }

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        fs::create_dir_all(app.join("Contents/Resources"))?;
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::write(app.join("Contents/Resources/value"), b"old")?;
        fs::write(
            app.join("Contents/MacOS/MorpheusLauncher"),
            b"stable-launcher",
        )?;
        fs::write(app.join("Contents/MacOS/Root Worker Runtime"), b"runtime")?;
        let launcher_hash = hash_file(&app.join("Contents/MacOS/MorpheusLauncher"))?;

        fs::create_dir_all(layout.current()?.join("resources"))?;
        fs::write(layout.current()?.join("resources/value"), b"old")?;
        write_json_atomically(
            &layout.current()?.join("manifest.json"),
            &PreparedManifest {
                schema_version: SCHEMA_VERSION,
                build_id: "old".into(),
                source_commit: "old".into(),
                artifacts: vec![artifact("value", b"old")],
                changes: ManifestChanges::default(),
            },
        )?;
        let stage = layout.staging()?.join("tx");
        fs::create_dir_all(stage.join("resources"))?;
        fs::write(stage.join("resources/value"), b"new")?;
        let candidate_manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "new".into(),
            source_commit: "new".into(),
            artifacts: vec![artifact("value", b"new")],
            changes: ManifestChanges::default(),
        };
        write_json_atomically(&stage.join("manifest.json"), &candidate_manifest)?;
        mark_owned(&layout, &stage, "tx", "prepared")?;
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Full,
            build_id: "new".into(),
            source_commit: "new".into(),
            prepared_root: temp.path().join("prepared"),
            app_bundle_path: app.clone(),
            reason: "test".into(),
        };
        let mut transaction = Transaction {
            schema_version: SCHEMA_VERSION,
            request,
            manifest_hash: manifest_hash(&candidate_manifest)?,
            artifact_content_hash: artifact_content_hash(&candidate_manifest),
            phase: TransactionPhase::Prepared,
            instance_id: None,
            slot_rotated: false,
            slot_rotation_phase: SlotRotationPhase::NotStarted,
            slot_restore_phase: SlotRestorePhase::NotStarted,
            slot_retired_path: None,
            slot_failed_path: None,
            slot_had_previous: None,
            resources_swapped: false,
            resources_backup_path: None,
            resources_replacement_path: None,
            resources_retired_path: None,
            resource_swap_phase: ResourceSwapPhase::NotStarted,
            resource_restore_phase: ResourceRestorePhase::NotStarted,
            signature_backup_path: None,
            signature_backup_ready: Some(false),
            signature_backup_had_code_signature: Some(false),
            signature_restore_phase: SignatureRestorePhase::NotStarted,
            runtime_retired_path: None,
            runtime_replacement_path: None,
            signature_retired_path: None,
            signature_replacement_path: None,
            signature_replacement_ready: Some(false),
            launcher_expected_hash: Some(launcher_hash.clone()),
            rollback_failure_evidence: None,
            post_ready_rollback: None,
            started_at: Utc::now(),
        };
        activate_slot(&layout, &mut transaction, &SuccessfulRunner)?;
        assert_eq!(
            hash_file(&app.join("Contents/MacOS/MorpheusLauncher"))?,
            launcher_hash
        );
        fs::write(
            app.join("Contents/MacOS/Root Worker Runtime"),
            b"runtime-mutated-by-signing",
        )?;
        restore_signature_artifacts_durably(&layout, &mut transaction, &app.join("Contents"))?;
        restore_signature_artifacts_durably(&layout, &mut transaction, &app.join("Contents"))?;
        assert_eq!(
            fs::read(app.join("Contents/MacOS/Root Worker Runtime"))?,
            b"runtime"
        );
        assert_eq!(
            hash_file(&app.join("Contents/MacOS/MorpheusLauncher"))?,
            launcher_hash
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn activation_rejects_stage_replaced_by_symlink_before_slot_mutation() -> Result<()> {
        use std::os::unix::fs::symlink;
        use std::os::unix::process::ExitStatusExt;

        struct SuccessfulRunner;
        impl CommandRunner for SuccessfulRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(0))
            }
        }

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::write(
            app.join("Contents/MacOS/MorpheusLauncher"),
            b"stable-launcher",
        )?;
        fs::create_dir_all(&layout.root)?;
        fs::create_dir_all(layout.current()?)?;
        fs::create_dir_all(layout.previous()?)?;
        fs::write(layout.current()?.join("sentinel"), b"current")?;
        fs::write(layout.previous()?.join("sentinel"), b"previous")?;
        let stage = layout.staging()?.join("tx");
        fs::create_dir_all(&stage)?;
        let external = temp.path().join("external-stage");
        fs::create_dir_all(&external)?;
        fs::write(external.join("sentinel"), b"external")?;
        fs::remove_dir_all(&stage)?;
        symlink(&external, &stage)?;

        let mut transaction = Transaction {
            schema_version: SCHEMA_VERSION,
            request: ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "candidate".into(),
                source_commit: "commit".into(),
                prepared_root: temp.path().join("producer-owned-prepared"),
                app_bundle_path: app,
                reason: "test".into(),
            },
            manifest_hash: "manifest".into(),
            artifact_content_hash: "content".into(),
            phase: TransactionPhase::Prepared,
            instance_id: None,
            slot_rotated: false,
            slot_rotation_phase: SlotRotationPhase::NotStarted,
            slot_restore_phase: SlotRestorePhase::NotStarted,
            slot_retired_path: None,
            slot_failed_path: None,
            slot_had_previous: None,
            resources_swapped: false,
            resources_backup_path: None,
            resources_replacement_path: None,
            resources_retired_path: None,
            resource_swap_phase: ResourceSwapPhase::NotStarted,
            resource_restore_phase: ResourceRestorePhase::NotStarted,
            signature_backup_path: None,
            signature_backup_ready: Some(false),
            signature_backup_had_code_signature: Some(false),
            signature_restore_phase: SignatureRestorePhase::NotStarted,
            runtime_retired_path: None,
            runtime_replacement_path: None,
            signature_retired_path: None,
            signature_replacement_path: None,
            signature_replacement_ready: Some(false),
            launcher_expected_hash: Some(hash_file(
                &temp.path().join("App.app/Contents/MacOS/MorpheusLauncher"),
            )?),
            rollback_failure_evidence: None,
            post_ready_rollback: None,
            started_at: Utc::now(),
        };
        write_json_atomically(&layout.transaction()?, &transaction)?;

        assert!(activate_slot(&layout, &mut transaction, &SuccessfulRunner).is_err());
        let persisted: Transaction = read_json(&layout.transaction()?)?;
        assert_eq!(persisted.phase, TransactionPhase::Prepared);
        assert_eq!(fs::read(layout.current()?.join("sentinel"))?, b"current");
        assert_eq!(fs::read(layout.previous()?.join("sentinel"))?, b"previous");
        assert_eq!(fs::read(external.join("sentinel"))?, b"external");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn activation_claim_rejects_same_build_with_different_content_before_slot_mutation()
    -> Result<()> {
        use std::os::unix::process::ExitStatusExt;

        struct SuccessfulRunner;
        impl CommandRunner for SuccessfulRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(0))
            }
        }

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::write(
            app.join("Contents/MacOS/MorpheusLauncher"),
            b"stable-launcher",
        )?;
        fs::create_dir_all(layout.current()?)?;
        fs::create_dir_all(layout.previous()?)?;
        fs::write(layout.current()?.join("sentinel"), b"current")?;
        fs::write(layout.previous()?.join("sentinel"), b"previous")?;
        let expected_manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "candidate".into(),
            source_commit: "commit".into(),
            artifacts: vec![artifact("value", b"expected")],
            changes: ManifestChanges::default(),
        };
        let replacement_manifest = PreparedManifest {
            schema_version: SCHEMA_VERSION,
            build_id: "candidate".into(),
            source_commit: "commit".into(),
            artifacts: vec![artifact("value", b"replacement")],
            changes: ManifestChanges::default(),
        };
        let stage = layout.staging()?.join("tx");
        fs::create_dir_all(stage.join("resources"))?;
        fs::write(stage.join("resources/value"), b"replacement")?;
        write_json_atomically(&stage.join("manifest.json"), &replacement_manifest)?;
        mark_owned(&layout, &stage, "tx", "prepared")?;
        let mut transaction = Transaction {
            schema_version: SCHEMA_VERSION,
            request: ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "candidate".into(),
                source_commit: "commit".into(),
                prepared_root: temp.path().join("producer-owned-prepared"),
                app_bundle_path: app.clone(),
                reason: "test".into(),
            },
            manifest_hash: manifest_hash(&expected_manifest)?,
            artifact_content_hash: artifact_content_hash(&expected_manifest),
            phase: TransactionPhase::Prepared,
            instance_id: None,
            slot_rotated: false,
            slot_rotation_phase: SlotRotationPhase::NotStarted,
            slot_restore_phase: SlotRestorePhase::NotStarted,
            slot_retired_path: None,
            slot_failed_path: None,
            slot_had_previous: None,
            resources_swapped: false,
            resources_backup_path: None,
            resources_replacement_path: None,
            resources_retired_path: None,
            resource_swap_phase: ResourceSwapPhase::NotStarted,
            resource_restore_phase: ResourceRestorePhase::NotStarted,
            signature_backup_path: None,
            signature_backup_ready: Some(false),
            signature_backup_had_code_signature: Some(false),
            signature_restore_phase: SignatureRestorePhase::NotStarted,
            runtime_retired_path: None,
            runtime_replacement_path: None,
            signature_retired_path: None,
            signature_replacement_path: None,
            signature_replacement_ready: Some(false),
            launcher_expected_hash: Some(hash_file(&app.join("Contents/MacOS/MorpheusLauncher"))?),
            rollback_failure_evidence: None,
            post_ready_rollback: None,
            started_at: Utc::now(),
        };
        write_json_atomically(&layout.transaction()?, &transaction)?;

        assert!(activate_slot(&layout, &mut transaction, &SuccessfulRunner).is_err());
        let persisted: Transaction = read_json(&layout.transaction()?)?;
        assert_eq!(persisted.phase, TransactionPhase::Prepared);
        assert_eq!(fs::read(layout.current()?.join("sentinel"))?, b"current");
        assert_eq!(fs::read(layout.previous()?.join("sentinel"))?, b"previous");
        assert!(layout.root.join(claimed_stage_name("tx")).exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rollback_verify_failure_preserves_recoverable_retired_resources() -> Result<()> {
        use std::os::unix::process::ExitStatusExt;

        struct VerifyFailRunner;
        impl CommandRunner for VerifyFailRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(1 << 8))
            }
        }

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        let resources = app.join("Contents/Resources");
        let backup = temp.path().join(".MorpheusResourcesPrevious-tx");
        fs::create_dir_all(&resources)?;
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::create_dir_all(&backup)?;
        fs::write(resources.join("value"), b"candidate")?;
        fs::write(
            app.join("Contents/MacOS/MorpheusLauncher"),
            b"stable-launcher",
        )?;
        fs::write(backup.join("value"), b"previous")?;
        let launcher_hash = hash_file(&app.join("Contents/MacOS/MorpheusLauncher"))?;
        let mut transaction = Transaction {
            schema_version: SCHEMA_VERSION,
            request: ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "candidate".into(),
                source_commit: "commit".into(),
                prepared_root: temp.path().join("prepared"),
                app_bundle_path: app,
                reason: "test".into(),
            },
            manifest_hash: "manifest".into(),
            artifact_content_hash: "content".into(),
            phase: TransactionPhase::RollingBack,
            instance_id: None,
            slot_rotated: false,
            slot_rotation_phase: SlotRotationPhase::NotStarted,
            slot_restore_phase: SlotRestorePhase::NotStarted,
            slot_retired_path: None,
            slot_failed_path: None,
            slot_had_previous: None,
            resources_swapped: true,
            resources_backup_path: Some(backup.clone()),
            resources_replacement_path: None,
            resources_retired_path: None,
            resource_swap_phase: ResourceSwapPhase::CandidateInstalled,
            resource_restore_phase: ResourceRestorePhase::NotStarted,
            signature_backup_path: None,
            signature_backup_ready: Some(false),
            signature_backup_had_code_signature: Some(false),
            signature_restore_phase: SignatureRestorePhase::NotStarted,
            runtime_retired_path: None,
            runtime_replacement_path: None,
            signature_retired_path: None,
            signature_replacement_path: None,
            signature_replacement_ready: Some(false),
            launcher_expected_hash: Some(launcher_hash),
            rollback_failure_evidence: None,
            post_ready_rollback: None,
            started_at: Utc::now(),
        };
        assert!(rollback_activation_with(&layout, &mut transaction, &VerifyFailRunner).is_err());
        assert_eq!(fs::read(resources.join("value"))?, b"previous");
        assert!(!backup.exists());
        assert!(
            transaction
                .resources_retired_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
        Ok(())
    }

    #[test]
    fn interrupted_prepared_transaction_remains_resumable() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        fs::create_dir_all(&layout.root)?;
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Full,
            build_id: "candidate".into(),
            source_commit: "commit".into(),
            prepared_root: temp.path().join("prepared"),
            app_bundle_path: temp.path().join("App.app"),
            reason: "test".into(),
        };
        write_json_atomically(
            &layout.transaction()?,
            &Transaction {
                schema_version: SCHEMA_VERSION,
                request,
                manifest_hash: "hash".into(),
                artifact_content_hash: "content-hash".into(),
                phase: TransactionPhase::Prepared,
                instance_id: None,
                slot_rotated: false,
                slot_rotation_phase: SlotRotationPhase::NotStarted,
                slot_restore_phase: SlotRestorePhase::NotStarted,
                slot_retired_path: None,
                slot_failed_path: None,
                slot_had_previous: None,
                resources_swapped: false,
                resources_backup_path: None,
                resources_replacement_path: None,
                resources_retired_path: None,
                resource_swap_phase: ResourceSwapPhase::NotStarted,
                resource_restore_phase: ResourceRestorePhase::NotStarted,
                signature_backup_path: None,
                signature_backup_ready: Some(false),
                signature_backup_had_code_signature: Some(false),
                signature_restore_phase: SignatureRestorePhase::NotStarted,
                runtime_retired_path: None,
                runtime_replacement_path: None,
                signature_retired_path: None,
                signature_replacement_path: None,
                signature_replacement_ready: Some(false),
                launcher_expected_hash: None,
                rollback_failure_evidence: None,
                post_ready_rollback: None,
                started_at: Utc::now(),
            },
        )?;
        recover_interrupted_transaction(&layout)?;
        assert!(layout.transaction()?.exists());
        Ok(())
    }

    #[test]
    fn interrupted_legacy_rollback_persists_evidence_before_recovery_mutation() -> Result<()> {
        for phase in [
            TransactionPhase::Activating,
            TransactionPhase::CandidateInstalled,
            TransactionPhase::CandidateStarted,
            TransactionPhase::RollingBack,
        ] {
            let temp = TempDir::new()?;
            let layout = Layout::new(temp.path().join("launcher"));
            fs::create_dir_all(&layout.root)?;
            let transaction = Transaction {
                schema_version: SCHEMA_VERSION,
                request: ActivationRequest {
                    schema_version: SCHEMA_VERSION,
                    transaction_id: "tx".into(),
                    request_id: "request".into(),
                    requested_by_thread_id: Some("thread".into()),
                    mode: ActivationMode::Full,
                    build_id: "candidate".into(),
                    source_commit: "commit".into(),
                    prepared_root: temp.path().join("producer-owned-prepared"),
                    app_bundle_path: temp.path().join("missing-App.app"),
                    reason: "test".into(),
                },
                manifest_hash: "manifest".into(),
                artifact_content_hash: "content".into(),
                phase,
                instance_id: None,
                slot_rotated: false,
                slot_rotation_phase: SlotRotationPhase::NotStarted,
                slot_restore_phase: SlotRestorePhase::NotStarted,
                slot_retired_path: None,
                slot_failed_path: None,
                slot_had_previous: None,
                resources_swapped: false,
                resources_backup_path: None,
                resources_replacement_path: None,
                resources_retired_path: None,
                resource_swap_phase: ResourceSwapPhase::NotStarted,
                resource_restore_phase: ResourceRestorePhase::NotStarted,
                signature_backup_path: None,
                signature_backup_ready: Some(false),
                signature_backup_had_code_signature: Some(false),
                signature_restore_phase: SignatureRestorePhase::NotStarted,
                runtime_retired_path: None,
                runtime_replacement_path: None,
                signature_retired_path: None,
                signature_replacement_path: None,
                signature_replacement_ready: Some(false),
                launcher_expected_hash: None,
                rollback_failure_evidence: None,
                post_ready_rollback: None,
                started_at: Utc::now(),
            };
            write_json_atomically(&layout.transaction()?, &transaction)?;

            assert!(recover_interrupted_transaction(&layout).is_err());
            let persisted: Transaction = read_json(&layout.transaction()?)?;
            let evidence = persisted
                .rollback_failure_evidence
                .context("interrupted rollback must durably record failure evidence")?;
            assert_eq!(evidence.failure_phase, "interrupted-recovery");
            assert_eq!(evidence.transaction_id.as_deref(), Some("tx"));
            assert_eq!(evidence.manifest_hash.as_deref(), Some("manifest"));
            assert_eq!(evidence.failed_build_hash.as_deref(), Some("content"));
            assert_eq!(
                evidence.transaction_path.as_deref(),
                Some(layout.transaction()?.as_path())
            );
        }
        Ok(())
    }

    #[test]
    fn rollback_complete_cleanup_resumes_after_each_deleted_backup() -> Result<()> {
        for deleted_count in 0..5 {
            let temp = TempDir::new()?;
            let layout = Layout::new(temp.path().join("launcher"));
            fs::create_dir_all(&layout.root)?;
            let resources_backup = temp.path().join(".MorpheusResourcesPrevious-tx");
            let signature_backup = temp.path().join(".MorpheusSignaturePrevious-tx");
            let runtime_retired = temp.path().join(".MorpheusRuntimeFailed-tx");
            let signature_retired = temp.path().join(".MorpheusCodeSignatureFailed-tx");
            let failed_slot = layout.root.join(".slot-candidate-failed-tx");
            for path in [
                &resources_backup,
                &signature_backup,
                &signature_retired,
                &failed_slot,
            ] {
                fs::create_dir_all(path)?;
            }
            fs::write(&runtime_retired, b"failed-runtime")?;
            let cleanup_paths = [
                resources_backup.clone(),
                signature_backup.clone(),
                runtime_retired.clone(),
                signature_retired.clone(),
                failed_slot.clone(),
            ];
            for path in cleanup_paths.iter().take(deleted_count) {
                if path.is_dir() {
                    fs::remove_dir_all(path)?;
                } else {
                    fs::remove_file(path)?;
                }
            }
            let transaction = Transaction {
                schema_version: SCHEMA_VERSION,
                request: ActivationRequest {
                    schema_version: SCHEMA_VERSION,
                    transaction_id: "tx".into(),
                    request_id: "request".into(),
                    requested_by_thread_id: Some("thread".into()),
                    mode: ActivationMode::Full,
                    build_id: "candidate".into(),
                    source_commit: "commit".into(),
                    prepared_root: temp.path().join("prepared"),
                    app_bundle_path: temp.path().join("App.app"),
                    reason: "test".into(),
                },
                manifest_hash: "manifest".into(),
                artifact_content_hash: "content".into(),
                phase: TransactionPhase::RollbackComplete,
                instance_id: None,
                slot_rotated: true,
                slot_rotation_phase: SlotRotationPhase::CandidateInstalled,
                slot_restore_phase: SlotRestorePhase::RetiredRestored,
                slot_retired_path: None,
                slot_failed_path: Some(failed_slot),
                slot_had_previous: Some(false),
                resources_swapped: true,
                resources_backup_path: Some(resources_backup),
                resources_replacement_path: None,
                resources_retired_path: None,
                resource_swap_phase: ResourceSwapPhase::CandidateInstalled,
                resource_restore_phase: ResourceRestorePhase::BackupRestored,
                signature_backup_path: Some(signature_backup),
                signature_backup_ready: Some(true),
                signature_backup_had_code_signature: Some(true),
                signature_restore_phase: SignatureRestorePhase::SignatureRestored,
                runtime_retired_path: Some(runtime_retired),
                runtime_replacement_path: None,
                signature_retired_path: Some(signature_retired),
                signature_replacement_path: None,
                signature_replacement_ready: Some(true),
                launcher_expected_hash: Some("launcher-hash".into()),
                rollback_failure_evidence: None,
                post_ready_rollback: None,
                started_at: Utc::now(),
            };
            write_json_atomically(&layout.transaction()?, &transaction)?;
            let recovered = recover_interrupted_transaction(&layout)?
                .context("rollback-complete transaction should be recovered")?;
            assert_eq!(recovered.phase, TransactionPhase::RollbackComplete);
            assert!(cleanup_paths.iter().all(|path| !path.exists()));
            assert!(layout.transaction()?.exists());
        }
        Ok(())
    }

    #[test]
    fn legacy_build_provenance_remains_unknown() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().to_path_buf());
        fs::create_dir_all(&layout.root)?;
        fs::write(
            layout.state()?,
            br#"{
                "schemaVersion": 1,
                "current": {
                    "buildId": "legacy",
                    "sourceCommit": "unknown",
                    "manifestHash": "legacy-hash",
                    "appBundlePath": "/App.app",
                    "activatedAt": "2026-09-08T00:00:00Z"
                },
                "previous": null,
                "blockedBuildHashes": [],
                "crashHistory": [],
                "failures": []
            }"#,
        )?;
        let state = load_state(&layout)?;
        let current = state.current.context("legacy current should load")?;
        assert_eq!(current.transaction_id, None);
        assert_eq!(current.request_id, None);
        assert_eq!(current.requested_by_thread_id, None);
        assert_eq!(current.artifact_content_hash, "legacy-hash");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn post_ready_rollback_recovers_from_each_durable_checkpoint() -> Result<()> {
        use std::os::unix::process::ExitStatusExt;

        struct SuccessfulRunner;
        impl CommandRunner for SuccessfulRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(0))
            }
        }

        for failed_checkpoint in [
            None,
            Some(PostReadyRollbackCheckpoint::RestoreSlot),
            Some(PostReadyRollbackCheckpoint::InstallResources),
            Some(PostReadyRollbackCheckpoint::Sign),
            Some(PostReadyRollbackCheckpoint::Verify),
            Some(PostReadyRollbackCheckpoint::SaveState),
        ] {
            let temp = TempDir::new()?;
            let layout = Layout::new(temp.path().join("launcher"));
            let app = temp.path().join("App.app");
            fs::create_dir_all(app.join("Contents/Resources"))?;
            fs::create_dir_all(app.join("Contents/MacOS"))?;
            fs::write(app.join("Contents/Resources/value"), b"failed")?;
            fs::write(
                app.join("Contents/MacOS/MorpheusLauncher"),
                b"stable-launcher",
            )?;
            let launcher_hash = hash_file(&app.join("Contents/MacOS/MorpheusLauncher"))?;

            for (slot, build_id, value) in [
                (layout.current()?, "failed", b"failed".as_slice()),
                (layout.previous()?, "fallback", b"fallback".as_slice()),
            ] {
                fs::create_dir_all(slot.join("resources"))?;
                fs::write(slot.join("resources/value"), value)?;
                write_json_atomically(
                    &slot.join("manifest.json"),
                    &PreparedManifest {
                        schema_version: SCHEMA_VERSION,
                        build_id: build_id.into(),
                        source_commit: format!("commit-{build_id}"),
                        artifacts: vec![artifact("value", value)],
                        changes: ManifestChanges::default(),
                    },
                )?;
            }
            let failed = build_record("failed", &app);
            let fallback = build_record("fallback", &app);
            let mut state = RuntimeState {
                current: Some(failed.clone()),
                previous: Some(fallback.clone()),
                ..RuntimeState::default()
            };
            save_state(&layout, &state)?;
            let superseded_stage = layout.staging()?.join("superseded-prepared");
            fs::create_dir_all(&superseded_stage)?;
            mark_owned(
                &layout,
                &superseded_stage,
                "superseded-prepared",
                "prepared",
            )?;
            write_json_atomically(
                &layout.transaction()?,
                &Transaction {
                    schema_version: SCHEMA_VERSION,
                    request: ActivationRequest {
                        schema_version: SCHEMA_VERSION,
                        transaction_id: "superseded-prepared".into(),
                        request_id: "prepared-request".into(),
                        requested_by_thread_id: Some("thread-id".into()),
                        mode: ActivationMode::Full,
                        build_id: "prepared".into(),
                        source_commit: "prepared-commit".into(),
                        prepared_root: temp.path().join("prepared"),
                        app_bundle_path: app.clone(),
                        reason: "test".into(),
                    },
                    manifest_hash: "prepared-manifest".into(),
                    artifact_content_hash: "prepared-content".into(),
                    phase: TransactionPhase::Prepared,
                    instance_id: None,
                    slot_rotated: false,
                    slot_rotation_phase: SlotRotationPhase::NotStarted,
                    slot_restore_phase: SlotRestorePhase::NotStarted,
                    slot_retired_path: None,
                    slot_failed_path: None,
                    slot_had_previous: None,
                    resources_swapped: false,
                    resources_backup_path: None,
                    resources_replacement_path: None,
                    resources_retired_path: None,
                    resource_swap_phase: ResourceSwapPhase::NotStarted,
                    resource_restore_phase: ResourceRestorePhase::NotStarted,
                    signature_backup_path: None,
                    signature_backup_ready: Some(false),
                    signature_backup_had_code_signature: Some(false),
                    signature_restore_phase: SignatureRestorePhase::NotStarted,
                    runtime_retired_path: None,
                    runtime_replacement_path: None,
                    signature_retired_path: None,
                    signature_replacement_path: None,
                    signature_replacement_ready: Some(false),
                    launcher_expected_hash: None,
                    rollback_failure_evidence: None,
                    post_ready_rollback: None,
                    started_at: Utc::now(),
                },
            )?;
            let evidence = failure_for_build(&failed);
            let mut transaction =
                begin_post_ready_rollback(&layout, failed, fallback.clone(), evidence.clone())?;
            let external_stage = temp.path().join("external-stage");
            fs::create_dir_all(&external_stage)?;
            fs::write(external_stage.join("sentinel"), b"keep")?;
            let mut legacy_transaction = serde_json::to_value(&transaction)?;
            legacy_transaction["postReadyRollback"]["supersededTransaction"]["stagePath"] =
                serde_json::json!(external_stage);
            write_json_atomically(&layout.transaction()?, &legacy_transaction)?;
            transaction = read_json(&layout.transaction()?)?;
            assert_eq!(transaction.phase, TransactionPhase::RollingBack);
            let superseded = transaction
                .post_ready_rollback
                .as_ref()
                .and_then(|rollback| rollback.superseded_transaction.as_ref())
                .context("superseded prepared transaction should be audited")?;
            assert_eq!(superseded.transaction_id, "superseded-prepared");
            assert_eq!(superseded.build_id, "prepared");
            assert!(!superseded_stage.exists());
            assert_eq!(fs::read(external_stage.join("sentinel"))?, b"keep");
            if failed_checkpoint.is_some() {
                let mut injected = false;
                let result = resume_post_ready_rollback_with(
                    &layout,
                    &mut transaction,
                    &SuccessfulRunner,
                    |checkpoint| {
                        if Some(checkpoint) == failed_checkpoint && !injected {
                            injected = true;
                            bail!("injected checkpoint failure");
                        }
                        Ok(())
                    },
                );
                assert!(result.is_err());
            }
            assert!(layout.transaction()?.exists());

            transaction = read_json(&layout.transaction()?)?;
            resume_post_ready_rollback(&layout, &mut transaction, &SuccessfulRunner)?;
            assert_eq!(fs::read(external_stage.join("sentinel"))?, b"keep");
            state = load_state(&layout)?;
            assert_eq!(state.current, Some(fallback));
            assert_eq!(state.previous, None);
            let persisted_evidence: FailureEvidence = read_json(&layout.failure_evidence()?)?;
            assert!(state.failures.contains(&persisted_evidence));
            assert_eq!(
                persisted_evidence.recovery_identity,
                evidence.recovery_identity
            );
            assert_eq!(fs::read(app.join("Contents/Resources/value"))?, b"fallback");
            assert_eq!(
                hash_file(&app.join("Contents/MacOS/MorpheusLauncher"))?,
                launcher_hash
            );
            assert!(!layout.transaction()?.exists());
        }
        Ok(())
    }
}
