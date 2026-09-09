use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use chrono::Utc;
use clap::Parser;
use clap::Subcommand;
use runtime_launcher::ActivationMode;
use runtime_launcher::ActivationRequest;
use runtime_launcher::BuildRecord;
use runtime_launcher::COORDINATED_RESTART_EXIT_CODE;
use runtime_launcher::CrashDecision;
use runtime_launcher::FailureEvidence;
use runtime_launcher::FailureEvidenceClaim;
use runtime_launcher::Layout;
use runtime_launcher::READY_TIMEOUT_SECS;
use runtime_launcher::RECOVERY_RESTART_EXIT_CODE;
use runtime_launcher::ReadyIdentity;
use runtime_launcher::SCHEMA_VERSION;
use runtime_launcher::SignatureRestorePhase;
use runtime_launcher::SlotRestorePhase;
use runtime_launcher::SlotRotationPhase;
use runtime_launcher::StateRootAuthority;
use runtime_launcher::SystemCommandRunner;
use runtime_launcher::Transaction;
use runtime_launcher::TransactionPhase;
use runtime_launcher::activate_slot;
use runtime_launcher::begin_post_ready_rollback;
use runtime_launcher::bind_transaction_paths;
use runtime_launcher::bootstrap_current_slot;
use runtime_launcher::claimed_failure_evidence_path;
use runtime_launcher::commit_activation_files;
use runtime_launcher::consumed_failure_evidence_path;
use runtime_launcher::failure_evidence_version;
use runtime_launcher::gc_staging;
use runtime_launcher::hash_file;
use runtime_launcher::load_and_validate_manifest;
use runtime_launcher::load_state;
use runtime_launcher::manifest_hash;
use runtime_launcher::pending_failure_evidence_path;
use runtime_launcher::persist_failure_evidence;
use runtime_launcher::read_json;
use runtime_launcher::ready_matches;
use runtime_launcher::record_crash;
use runtime_launcher::record_failure;
use runtime_launcher::recover_interrupted_transaction_with;
use runtime_launcher::remove_staging_child;
use runtime_launcher::request_failure;
use runtime_launcher::resolve_staging_child;
use runtime_launcher::resume_post_ready_rollback;
use runtime_launcher::rollback_activation_with;
use runtime_launcher::save_state;
use runtime_launcher::stage_candidate;
use runtime_launcher::validate_claim_derived_consumed;
use runtime_launcher::validate_failure_evidence_claim;
use runtime_launcher::validate_request;
use runtime_launcher::write_json_atomically;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::ExitStatus;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use uuid::Uuid;

// Identity-scoped consumed artifacts provide bounded ack idempotence without
// allowing acknowledged history to grow forever.
const MAX_CONSUMED_FAILURE_ARTIFACTS: usize = 32;
const RUNTIME_LAUNCHER_HOME_ENV: &str = "MORPHEUS_RUNTIME_LAUNCHER_HOME";

#[derive(Debug, Parser)]
#[command(name = "morpheus-runtime-launcher")]
struct Cli {
    #[arg(long, env = "MORPHEUS_RUNTIME_LAUNCHER_HOME")]
    state_root: Option<PathBuf>,
    #[arg(long)]
    app_bundle: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<LauncherCommand>,
}

#[derive(Debug, Subcommand)]
enum LauncherCommand {
    Run {
        #[arg(long)]
        app_bundle: PathBuf,
    },
    PrepareFull {
        #[arg(long)]
        request: PathBuf,
    },
    ActivateHot {
        #[arg(long)]
        request: PathBuf,
    },
    CommitHot {
        #[arg(long)]
        transaction: String,
    },
    RollbackHot {
        #[arg(long)]
        transaction: String,
    },
    AbortFull {
        #[arg(long)]
        transaction: String,
    },
    ClaimFailure {
        #[arg(long)]
        transaction: Option<String>,
        #[arg(long)]
        request: Option<String>,
        #[arg(long)]
        recovery_identity: String,
        #[arg(long)]
        expected_version: String,
    },
    FinalizeFailureClaim {
        #[arg(long)]
        claim_id: String,
        #[arg(long)]
        recovery_identity: String,
    },
    Status {
        #[arg(long)]
        app_bundle: Option<PathBuf>,
        #[arg(long)]
        recovery_identity: Option<String>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CommandResult<T: Serialize> {
    ok: bool,
    result: T,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivationResult {
    transaction_id: String,
    build_id: String,
    manifest_hash: String,
    artifact_content_hash: String,
    phase: TransactionPhase,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusResult {
    state: runtime_launcher::RuntimeState,
    transaction: Option<Transaction>,
    failure_evidence: Option<FailureEvidence>,
    pending_failure_evidence: Vec<FailureEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_evidence_match: Option<LocatedFailureEvidence>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LocatedFailureEvidence {
    recovery_identity: String,
    artifact_state: &'static str,
    transaction_id: Option<String>,
    request_id: Option<String>,
    configured_evidence_path: PathBuf,
    active_evidence_path: PathBuf,
    version_token: String,
    evidence: FailureEvidence,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct FailureClaimResult {
    ok: bool,
    claim_id: String,
    recovery_identity: String,
    transaction_id: Option<String>,
    request_id: Option<String>,
    version_token: String,
    source_version_token: String,
    configured_evidence_path: PathBuf,
    active_evidence_path: PathBuf,
    claim_state: &'static str,
    requested_version_matched: bool,
    evidence: FailureEvidence,
}

#[derive(Debug)]
struct HotActivationFailure {
    message: String,
    request: ActivationRequest,
    artifact_content_hash: String,
    rolled_back: bool,
}

#[derive(Debug)]
struct FailureEvidenceVersionChanged {
    expected: String,
    actual: String,
}

struct StateRootLock {
    file: File,
    authority: StateRootAuthority,
}

#[cfg(unix)]
mod file_lock {
    use std::os::raw::c_int;

    pub const EXCLUSIVE: c_int = 2;
    pub const UNLOCK: c_int = 8;

    unsafe extern "C" {
        pub fn flock(file_descriptor: c_int, operation: c_int) -> c_int;
    }
}

impl StateRootLock {
    fn acquire(authority: &StateRootAuthority) -> Result<Self> {
        let file = authority.open_lock_file()?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: `file` owns a valid descriptor for the duration of the call.
            if unsafe { file_lock::flock(file.as_raw_fd(), file_lock::EXCLUSIVE) } != 0 {
                return Err(std::io::Error::last_os_error())
                    .context("failed to lock trusted launcher state root");
            }
        }
        #[cfg(not(unix))]
        bail!("runtime launcher locking is unsupported on this platform");
        Ok(Self {
            file,
            authority: authority.clone(),
        })
    }

    fn layout(&self) -> Layout {
        self.authority.layout()
    }
}

impl Drop for StateRootLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: the lock guard still owns the descriptor being unlocked.
            let _ = unsafe { file_lock::flock(self.file.as_raw_fd(), file_lock::UNLOCK) };
        }
    }
}

impl std::fmt::Display for HotActivationFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HotActivationFailure {}

impl std::fmt::Display for FailureEvidenceVersionChanged {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "failure evidence version changed: expected {:?}, found {:?}",
            self.expected, self.actual
        )
    }
}

impl std::error::Error for FailureEvidenceVersionChanged {}

fn main() {
    if let Err(error) = run() {
        let response =
            if let Some(version_error) = error.downcast_ref::<FailureEvidenceVersionChanged>() {
                serde_json::json!({
                    "ok": false,
                    "error": format!("{error:#}"),
                    "errorCode": "failure-evidence-version-changed",
                    "expectedVersion": version_error.expected.as_str(),
                    "actualVersion": version_error.actual.as_str(),
                })
            } else {
                serde_json::json!({
                    "ok": false,
                    "error": format!("{error:#}"),
                })
            };
        println!(
            "{}",
            serde_json::to_string(&response).unwrap_or_else(|_| String::from(
                "{\"ok\":false,\"error\":\"serialization failed\"}"
            ))
        );
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    ensure_supported_platform()?;
    let cli = Cli::parse();
    let configured_layout = Layout::new(resolve_state_root(cli.state_root)?);
    let authority = StateRootAuthority::open(&configured_layout)?;
    let layout = authority.layout();
    match cli.command {
        Some(LauncherCommand::Run { app_bundle }) => {
            supervise(&authority, &layout, Some(app_bundle))
        }
        Some(LauncherCommand::PrepareFull { request }) => {
            let lock = StateRootLock::acquire(&authority)?;
            let layout = lock.layout();
            let request: ActivationRequest = read_json(&request)?;
            if request.mode != ActivationMode::Full {
                bail!("prepare-full requires mode=full");
            }
            let result = prepare_activation(&layout, request)?;
            print_json(&CommandResult { ok: true, result })
        }
        Some(LauncherCommand::ActivateHot { request }) => {
            let lock = StateRootLock::acquire(&authority)?;
            let layout = lock.layout();
            let request: ActivationRequest = read_json(&request)?;
            if request.mode != ActivationMode::Hot {
                bail!("activate-hot requires mode=hot");
            }
            match activate_hot(&layout, request) {
                Ok(result) => print_json(&CommandResult { ok: true, result }),
                Err(error) => print_hot_activation_failure_and_exit(&layout, error),
            }
        }
        Some(LauncherCommand::CommitHot { transaction }) => {
            let lock = StateRootLock::acquire(&authority)?;
            let layout = lock.layout();
            commit_hot(&layout, &transaction)
        }
        Some(LauncherCommand::RollbackHot { transaction }) => {
            let lock = StateRootLock::acquire(&authority)?;
            let layout = lock.layout();
            rollback_hot(&layout, &transaction)
        }
        Some(LauncherCommand::AbortFull { transaction }) => {
            let lock = StateRootLock::acquire(&authority)?;
            let layout = lock.layout();
            abort_full(&layout, &transaction)
        }
        Some(LauncherCommand::ClaimFailure {
            transaction,
            request,
            recovery_identity,
            expected_version,
        }) => {
            let lock = StateRootLock::acquire(&authority)?;
            let layout = lock.layout();
            let result = claim_failure(
                &layout,
                transaction.as_deref(),
                request.as_deref(),
                &recovery_identity,
                &expected_version,
            )?;
            print_json(&result)
        }
        Some(LauncherCommand::FinalizeFailureClaim {
            claim_id,
            recovery_identity,
        }) => {
            let lock = StateRootLock::acquire(&authority)?;
            let layout = lock.layout();
            finalize_failure_claim(&layout, &claim_id, &recovery_identity)
        }
        Some(LauncherCommand::Status {
            app_bundle,
            recovery_identity,
        }) => {
            let lock = StateRootLock::acquire(&authority)?;
            let layout = lock.layout();
            repair_failure_queue(&layout)?;
            let state = load_state(&layout)?;
            if let Some(app_bundle) = app_bundle {
                if let Some(current) = &state.current
                    && current.app_bundle_path != app_bundle
                {
                    bail!(
                        "state belongs to {}, not {}",
                        current.app_bundle_path.display(),
                        app_bundle.display()
                    );
                }
            }
            let claimed_failure_evidence = list_failure_claims(&layout)?
                .into_iter()
                .next()
                .map(|claim| claim.evidence);
            print_json(&CommandResult {
                ok: true,
                result: StatusResult {
                    failure_evidence_match: recovery_identity
                        .as_deref()
                        .map(|identity| locate_failure_evidence(&layout, identity))
                        .transpose()?,
                    state,
                    transaction: layout
                        .transaction()?
                        .exists()
                        .then(|| read_json(&layout.transaction()?))
                        .transpose()?,
                    failure_evidence: claimed_failure_evidence.or(layout
                        .failure_evidence()?
                        .exists()
                        .then(|| read_json(&layout.failure_evidence()?))
                        .transpose()?),
                    pending_failure_evidence: list_pending_failure_evidence(&layout)?,
                },
            })
        }
        None => supervise(&authority, &layout, cli.app_bundle),
    }
}

fn resolve_state_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    let home = std::env::var_os("MORPHEUS_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|path| PathBuf::from(path).join(".morpheus")))
        .context("MORPHEUS_HOME and HOME are both unset")?;
    Ok(home.join("runtime-launcher"))
}

fn prepare_activation(layout: &Layout, request: ActivationRequest) -> Result<ActivationResult> {
    reject_concurrent_transaction(layout)?;
    let (manifest, manifest_hash, artifact_content_hash) = load_and_validate_manifest(&request)?;
    let state = load_state(layout)?;
    reject_blocked_build(
        &state,
        &request.build_id,
        &manifest_hash,
        &artifact_content_hash,
    )?;
    let stage = stage_candidate(layout, &request, &manifest)?;
    let launcher_expected_hash = hash_file(
        &request
            .app_bundle_path
            .join("Contents/MacOS/MorpheusLauncher"),
    )
    .context("failed to capture stable MorpheusLauncher hash")?;
    let transaction = Transaction {
        schema_version: SCHEMA_VERSION,
        request: request.clone(),
        manifest_hash: manifest_hash.clone(),
        artifact_content_hash,
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
        resource_swap_phase: Default::default(),
        resource_restore_phase: Default::default(),
        signature_backup_path: None,
        signature_backup_ready: Some(false),
        signature_backup_had_code_signature: Some(false),
        signature_restore_phase: SignatureRestorePhase::NotStarted,
        runtime_retired_path: None,
        runtime_replacement_path: None,
        signature_retired_path: None,
        signature_replacement_path: None,
        signature_replacement_ready: Some(false),
        launcher_expected_hash: Some(launcher_expected_hash),
        rollback_failure_evidence: None,
        post_ready_rollback: None,
        started_at: Utc::now(),
    };
    write_json_atomically(&layout.transaction()?, &transaction)?;
    if !stage.exists() {
        bail!("staged candidate disappeared before activation");
    }
    Ok(ActivationResult {
        transaction_id: request.transaction_id,
        build_id: request.build_id,
        manifest_hash,
        artifact_content_hash: transaction.artifact_content_hash,
        phase: TransactionPhase::Prepared,
    })
}

fn activate_hot(layout: &Layout, request: ActivationRequest) -> Result<ActivationResult> {
    reject_concurrent_transaction(layout)?;
    let (manifest, manifest_hash, artifact_content_hash) = load_and_validate_manifest(&request)?;
    if manifest.changes.main || manifest.changes.preload {
        bail!("hot activation rejects main or preload changes");
    }
    let state = load_state(layout)?;
    reject_blocked_build(
        &state,
        &request.build_id,
        &manifest_hash,
        &artifact_content_hash,
    )?;
    stage_candidate(layout, &request, &manifest)?;
    let launcher_expected_hash = hash_file(
        &request
            .app_bundle_path
            .join("Contents/MacOS/MorpheusLauncher"),
    )
    .context("failed to capture stable MorpheusLauncher hash")?;
    let mut transaction = Transaction {
        schema_version: SCHEMA_VERSION,
        request: request.clone(),
        manifest_hash: manifest_hash.clone(),
        artifact_content_hash,
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
        resource_swap_phase: Default::default(),
        resource_restore_phase: Default::default(),
        signature_backup_path: None,
        signature_backup_ready: Some(false),
        signature_backup_had_code_signature: Some(false),
        signature_restore_phase: SignatureRestorePhase::NotStarted,
        runtime_retired_path: None,
        runtime_replacement_path: None,
        signature_retired_path: None,
        signature_replacement_path: None,
        signature_replacement_ready: Some(false),
        launcher_expected_hash: Some(launcher_expected_hash),
        rollback_failure_evidence: None,
        post_ready_rollback: None,
        started_at: Utc::now(),
    };
    write_json_atomically(&layout.transaction()?, &transaction)?;
    if let Err(error) = activate_slot(layout, &mut transaction, &SystemCommandRunner) {
        if transaction.phase == TransactionPhase::Prepared {
            abort_pre_activation_candidate(
                layout,
                &transaction,
                &format!("hot activation validation failed before slot mutation: {error:#}"),
            )?;
            return Err(error);
        }
        let artifact_content_hash = transaction.artifact_content_hash.clone();
        let summary = format!("hot activation switch or signing failed: {error:#}");
        let rollback = rollback_candidate(layout, &transaction, &summary, None);
        let (message, rolled_back) = match rollback {
            Ok(()) => (format!("{error:#}"), true),
            Err(rollback_error) => (
                format!("{error:#}; rollback verification also failed: {rollback_error:#}"),
                false,
            ),
        };
        return Err(anyhow::Error::new(HotActivationFailure {
            message,
            request,
            artifact_content_hash,
            rolled_back,
        }));
    }
    transaction.phase = TransactionPhase::CandidateInstalled;
    write_json_atomically(&layout.transaction()?, &transaction)?;
    Ok(ActivationResult {
        transaction_id: request.transaction_id,
        build_id: request.build_id,
        manifest_hash,
        artifact_content_hash: transaction.artifact_content_hash,
        phase: TransactionPhase::CandidateInstalled,
    })
}

fn print_hot_activation_failure_and_exit(layout: &Layout, error: anyhow::Error) -> Result<()> {
    if let Some(failure) = error.downcast_ref::<HotActivationFailure>() {
        print_json(&serde_json::json!({
            "ok": false,
            "error": failure.message.as_str(),
            "rolledBack": failure.rolled_back,
            "transactionId": failure.request.transaction_id,
            "requestId": failure.request.request_id,
            "buildId": failure.request.build_id,
            "artifactContentHash": failure.artifact_content_hash,
            "evidencePath": layout.configured_failure_evidence(),
            "transactionPath": layout.configured_transaction(),
            "phase": if failure.rolled_back { "rolled-back" } else { "rollback-pending" },
        }))?;
        std::process::exit(1);
    }
    Err(error)
}

fn reject_concurrent_transaction(layout: &Layout) -> Result<()> {
    if layout.transaction()?.exists() {
        let active: Transaction = read_json(&layout.transaction()?)?;
        bail!(
            "transaction {} is still active in phase {:?}",
            active.request.transaction_id,
            active.phase
        );
    }
    Ok(())
}

fn reject_blocked_build(
    state: &runtime_launcher::RuntimeState,
    build_id: &str,
    manifest_hash: &str,
    artifact_content_hash: &str,
) -> Result<()> {
    if state.blocked_build_ids.contains(build_id)
        || state.blocked_build_hashes.contains(manifest_hash)
        || state
            .blocked_artifact_hashes
            .contains(artifact_content_hash)
    {
        bail!("build {build_id} ({artifact_content_hash}) is blocked after an earlier rollback");
    }
    Ok(())
}

fn commit_hot(layout: &Layout, transaction_id: &str) -> Result<()> {
    let mut transaction: Transaction = read_json(&layout.transaction()?)?;
    require_hot_transaction(&transaction, transaction_id)?;
    if !matches!(
        transaction.phase,
        TransactionPhase::CandidateInstalled | TransactionPhase::Ready
    ) {
        bail!("hot transaction is not awaiting commit");
    }
    transaction.phase = TransactionPhase::Ready;
    write_json_atomically(&layout.transaction()?, &transaction)?;
    commit_ready_state(layout, &transaction)?;
    fs::remove_file(layout.transaction()?)?;
    print_json(&CommandResult {
        ok: true,
        result: ActivationResult {
            transaction_id: transaction.request.transaction_id,
            build_id: transaction.request.build_id,
            manifest_hash: transaction.manifest_hash,
            artifact_content_hash: transaction.artifact_content_hash,
            phase: TransactionPhase::Ready,
        },
    })
}

fn rollback_hot(layout: &Layout, transaction_id: &str) -> Result<()> {
    let transaction: Transaction = read_json(&layout.transaction()?)?;
    require_hot_transaction(&transaction, transaction_id)?;
    rollback_candidate(
        layout,
        &transaction,
        "hot activation was rejected by the host",
        None,
    )?;
    print_json(&serde_json::json!({
        "ok": true,
        "transactionId": transaction_id,
        "buildId": transaction.request.build_id,
        "artifactContentHash": transaction.artifact_content_hash,
        "evidencePath": layout.configured_failure_evidence(),
        "transactionPath": layout.configured_transaction(),
        "phase": "rolled-back",
    }))
}

fn require_hot_transaction(transaction: &Transaction, transaction_id: &str) -> Result<()> {
    if transaction.request.mode != ActivationMode::Hot
        || transaction.request.transaction_id != transaction_id
    {
        bail!("hot transaction identity does not match");
    }
    Ok(())
}

fn abort_full(layout: &Layout, transaction_id: &str) -> Result<()> {
    let transaction: Transaction = read_json(&layout.transaction()?)?;
    if transaction.request.mode != ActivationMode::Full
        || transaction.request.transaction_id != transaction_id
    {
        bail!("full transaction identity does not match");
    }
    if transaction.phase != TransactionPhase::Prepared {
        bail!("full transaction has already switched and cannot be aborted");
    }
    validate_request(&transaction.request)?;
    remove_staging_child(layout, transaction_id)?;
    fs::remove_file(layout.transaction()?)?;
    print_json(&serde_json::json!({
        "ok": true,
        "transactionId": transaction_id,
        "phase": "aborted",
    }))
}

fn locate_failure_evidence(
    layout: &Layout,
    expected_recovery_identity: &str,
) -> Result<LocatedFailureEvidence> {
    let recovery_identity = Uuid::parse_str(expected_recovery_identity)
        .with_context(|| {
            format!("invalid expected failure recovery identity {expected_recovery_identity:?}")
        })?
        .to_string();
    if recovery_identity != expected_recovery_identity {
        bail!("expected failure recovery identity is not canonical");
    }
    let candidates = [
        (
            "consumed",
            consumed_failure_evidence_path(layout, &recovery_identity)?,
            configured_consumed_failure_evidence_path(layout, &recovery_identity),
        ),
        (
            "claimed",
            claimed_failure_evidence_path(layout, &recovery_identity)?,
            configured_claimed_failure_evidence_path(layout, &recovery_identity),
        ),
        (
            "pending",
            pending_failure_evidence_path(layout, &recovery_identity)?,
            layout.root.join(format!(
                ".failure-evidence.pending-{recovery_identity}.json"
            )),
        ),
        (
            "current",
            layout.failure_evidence()?,
            layout.configured_failure_evidence(),
        ),
    ];
    for (artifact_state, active_path, configured_path) in candidates {
        if !regular_failure_artifact_exists(&active_path, "located failure evidence")? {
            continue;
        }
        let claim = (artifact_state == "claimed")
            .then(|| read_json::<FailureEvidenceClaim>(&active_path))
            .transpose()?;
        if let Some(claim) = &claim {
            validate_failure_evidence_claim(claim, &layout.root_owner_nonce()?)?;
        }
        let evidence = if let Some(claim) = claim {
            claim.evidence
        } else {
            read_json::<FailureEvidence>(&active_path)?
        };
        require_failure_artifact_owner(layout, &evidence)?;
        let identity = validated_recovery_identity(&evidence)?
            .context("located failure evidence has no recovery identity")?;
        if identity != recovery_identity {
            if artifact_state == "current" {
                continue;
            }
            bail!("identity-scoped failure evidence does not match requested identity");
        }
        return Ok(LocatedFailureEvidence {
            recovery_identity,
            artifact_state,
            transaction_id: evidence.transaction_id.clone(),
            request_id: evidence.request_id.clone(),
            configured_evidence_path: configured_path,
            active_evidence_path: active_path,
            version_token: failure_evidence_version(&evidence)?,
            evidence,
        });
    }
    bail!("no canonical failure evidence artifact matches recovery identity {recovery_identity}")
}

fn canonical_cli_uuid(value: &str, label: &str) -> Result<String> {
    let parsed = Uuid::parse_str(value)
        .with_context(|| format!("invalid {label} {value:?}"))?
        .to_string();
    if parsed != value {
        bail!("{label} is not canonical");
    }
    Ok(parsed)
}

fn read_failure_claim(
    layout: &Layout,
    recovery_identity: &str,
    claimed_path: &Path,
) -> Result<Option<FailureEvidenceClaim>> {
    if !regular_failure_artifact_exists(claimed_path, "claimed failure evidence")? {
        return Ok(None);
    }
    let claim: FailureEvidenceClaim = read_json(claimed_path)?;
    validate_failure_evidence_claim(&claim, &layout.root_owner_nonce()?)?;
    if claim.recovery_identity != recovery_identity {
        bail!("claimed failure evidence filename does not match recovery identity");
    }
    Ok(Some(claim))
}

fn failure_claim_result(
    layout: &Layout,
    claim: FailureEvidenceClaim,
    active_evidence_path: PathBuf,
    claim_state: &'static str,
    expected_version: &str,
) -> FailureClaimResult {
    let requested_version_matched = expected_version == claim.source_version_token.as_str()
        || expected_version == claim.version_token.as_str();
    FailureClaimResult {
        ok: true,
        claim_id: claim.claim_id,
        recovery_identity: claim.recovery_identity.clone(),
        transaction_id: claim.evidence.transaction_id.clone(),
        request_id: claim.evidence.request_id.clone(),
        version_token: claim.version_token,
        source_version_token: claim.source_version_token.clone(),
        configured_evidence_path: configured_claimed_failure_evidence_path(
            layout,
            &claim.recovery_identity,
        ),
        active_evidence_path,
        claim_state,
        requested_version_matched,
        evidence: claim.evidence,
    }
}

fn claim_failure(
    layout: &Layout,
    expected_transaction_id: Option<&str>,
    expected_request_id: Option<&str>,
    expected_recovery_identity: &str,
    expected_version: &str,
) -> Result<FailureClaimResult> {
    let recovery_identity = canonical_cli_uuid(
        expected_recovery_identity,
        "expected failure recovery identity",
    )?;
    let claimed_path = claimed_failure_evidence_path(layout, &recovery_identity)?;
    let consumed_path = consumed_failure_evidence_path(layout, &recovery_identity)?;
    if regular_failure_artifact_exists(&consumed_path, "consumed failure evidence")? {
        let consumed: FailureEvidence = read_json(&consumed_path)?;
        require_failure_artifact_owner(layout, &consumed)?;
        validate_expected_failure(
            &consumed,
            expected_transaction_id,
            expected_request_id,
            &recovery_identity,
        )?;
        bail!("failure evidence is already finalized");
    }
    if let Some(claim) = read_failure_claim(layout, &recovery_identity, &claimed_path)? {
        validate_expected_failure(
            &claim.evidence,
            expected_transaction_id,
            expected_request_id,
            &recovery_identity,
        )?;
        return Ok(failure_claim_result(
            layout,
            claim,
            claimed_path,
            "existing",
            expected_version,
        ));
    }

    let located = locate_failure_evidence(layout, &recovery_identity)?;
    validate_expected_failure(
        &located.evidence,
        expected_transaction_id,
        expected_request_id,
        &recovery_identity,
    )?;
    if located.version_token != expected_version {
        return Err(FailureEvidenceVersionChanged {
            expected: expected_version.to_string(),
            actual: located.version_token,
        }
        .into());
    }

    // All caller-controlled identity, raw pair, and version checks are
    // read-only. Queue-wide structural validation also completes before the
    // first claim mutation.
    let _ = preflight_failure_queue(layout)?;

    let owner_nonce = layout.root_owner_nonce()?;
    let claim_id = Uuid::new_v4().to_string();
    let mut frozen = located.evidence;
    let state = load_state(layout)?;
    if let Some(state_evidence) = state
        .failures
        .iter()
        .find(|evidence| evidence.recovery_identity.as_deref() == Some(recovery_identity.as_str()))
    {
        require_same_failure_provenance(state_evidence, &frozen)?;
        if frozen.failed_build_hash.is_none() {
            frozen.failed_build_hash = state_evidence.failed_build_hash.clone();
        }
    }
    frozen.claim_id = Some(claim_id.clone());
    frozen.acknowledged = false;
    let claim = FailureEvidenceClaim {
        schema_version: SCHEMA_VERSION,
        claim_id,
        recovery_identity: recovery_identity.clone(),
        launcher_owner_nonce: owner_nonce,
        source_version_token: expected_version.to_string(),
        version_token: failure_evidence_version(&frozen)?,
        evidence: frozen,
    };
    validate_failure_evidence_claim(&claim, &layout.root_owner_nonce()?)?;
    // The immutable claim is durable before any active source is removed.
    // Repair treats claimed+active as a recoverable duplicate.
    write_json_atomically(&claimed_path, &claim)?;
    for path in [
        pending_failure_evidence_path(layout, &recovery_identity)?,
        layout.failure_evidence()?,
    ] {
        if !regular_failure_artifact_exists(&path, "claimed active duplicate")? {
            continue;
        }
        let active: FailureEvidence = read_json(&path)?;
        if active.recovery_identity.as_deref() != Some(recovery_identity.as_str()) {
            continue;
        }
        require_failure_artifact_owner(layout, &active)?;
        require_same_failure_provenance(&active, &claim.evidence)?;
        fs::remove_file(&path)?;
        sync_directory(&layout.active_root()?)?;
    }
    promote_pending_failure_evidence(layout)?;
    Ok(failure_claim_result(
        layout,
        claim,
        claimed_path,
        "created",
        expected_version,
    ))
}

fn finalize_failure_claim(
    layout: &Layout,
    expected_claim_id: &str,
    expected_recovery_identity: &str,
) -> Result<()> {
    let claim_id = canonical_cli_uuid(expected_claim_id, "failure claim id")?;
    let recovery_identity =
        canonical_cli_uuid(expected_recovery_identity, "failure recovery identity")?;
    let claimed_path = claimed_failure_evidence_path(layout, &recovery_identity)?;
    let consumed_path = consumed_failure_evidence_path(layout, &recovery_identity)?;

    if let Some(claim) = read_failure_claim(layout, &recovery_identity, &claimed_path)? {
        if claim.claim_id != claim_id {
            bail!("failure claim id does not match recovery identity");
        }
        let _ = preflight_failure_queue(layout)?;
        let mut consumed = claim.evidence.clone();
        consumed.acknowledged = true;
        if regular_failure_artifact_exists(&consumed_path, "consumed failure evidence")? {
            let existing: FailureEvidence = read_json(&consumed_path)?;
            require_failure_artifact_owner(layout, &existing)?;
            require_consumed_matches_claim(&existing, &claim)?;
        } else {
            // Consumed is durable before claimed is removed. A crash leaves a
            // terminal duplicate that repair converges without payload drift.
            write_json_atomically(&consumed_path, &consumed)?;
        }
        if claimed_path.exists() {
            fs::remove_file(&claimed_path)?;
            sync_directory(&layout.active_root()?)?;
        }
        let mut state = load_state(layout)?;
        acknowledge_failure_in_state(&mut state, &consumed)?;
        save_state(layout, &state)?;
        promote_pending_failure_evidence(layout)?;
        return print_failure_ack_result(
            consumed.transaction_id.as_deref(),
            consumed.request_id.as_deref(),
            &recovery_identity,
            layout,
            Some(configured_consumed_failure_evidence_path(
                layout,
                &recovery_identity,
            )),
            true,
            Some(&claim_id),
            Some(&claim.version_token),
        );
    }

    if !regular_failure_artifact_exists(&consumed_path, "consumed failure evidence")? {
        bail!("no durable failure claim matches the expected claim id and recovery identity");
    }
    let consumed: FailureEvidence = read_json(&consumed_path)?;
    require_failure_artifact_owner(layout, &consumed)?;
    validate_expected_failure(
        &consumed,
        consumed.transaction_id.as_deref(),
        consumed.request_id.as_deref(),
        &recovery_identity,
    )?;
    if consumed.claim_id.as_deref() != Some(claim_id.as_str()) {
        bail!("consumed failure evidence belongs to another claim");
    }
    let mut frozen = consumed.clone();
    frozen.acknowledged = false;
    let version_token = failure_evidence_version(&frozen)?;
    print_failure_ack_result(
        consumed.transaction_id.as_deref(),
        consumed.request_id.as_deref(),
        &recovery_identity,
        layout,
        Some(configured_consumed_failure_evidence_path(
            layout,
            &recovery_identity,
        )),
        true,
        Some(&claim_id),
        Some(&version_token),
    )
}

fn require_consumed_matches_claim(
    consumed: &FailureEvidence,
    claim: &FailureEvidenceClaim,
) -> Result<()> {
    let mut expected = claim.evidence.clone();
    expected.acknowledged = true;
    if consumed != &expected {
        bail!("consumed failure evidence does not match its immutable claim");
    }
    Ok(())
}

#[cfg(test)]
fn acknowledge_failure_versioned(
    layout: &Layout,
    expected_transaction_id: Option<&str>,
    expected_request_id: Option<&str>,
    expected_recovery_identity: &str,
    expected_version: &str,
) -> Result<()> {
    let recovery_identity = Uuid::parse_str(expected_recovery_identity)
        .with_context(|| {
            format!("invalid expected failure recovery identity {expected_recovery_identity:?}")
        })?
        .to_string();
    if recovery_identity != expected_recovery_identity {
        bail!("expected failure recovery identity is not canonical");
    }
    let located = locate_failure_evidence(layout, &recovery_identity)?;
    validate_expected_failure(
        &located.evidence,
        expected_transaction_id,
        expected_request_id,
        &recovery_identity,
    )?;
    if located.version_token != expected_version {
        return Err(FailureEvidenceVersionChanged {
            expected: expected_version.to_string(),
            actual: located.version_token,
        }
        .into());
    }
    // Keep identity/raw-pair/version mismatch strictly read-only. Once those
    // target checks pass, validate the complete queue before the first ack
    // mutation so unrelated structural/provenance conflicts also fail closed.
    let _ = preflight_failure_queue(layout)?;
    let consumed_path = consumed_failure_evidence_path(layout, &recovery_identity)?;
    let configured_consumed_path =
        configured_consumed_failure_evidence_path(layout, &recovery_identity);
    let pending_path = pending_failure_evidence_path(layout, &recovery_identity)?;
    let current = layout
        .failure_evidence()?
        .exists()
        .then(|| {
            require_regular_failure_artifact(
                &layout.failure_evidence()?,
                "current failure evidence",
            )?;
            read_json::<FailureEvidence>(&layout.failure_evidence()?)
        })
        .transpose()?;

    if consumed_path.exists() {
        require_regular_failure_artifact(&consumed_path, "consumed failure evidence")?;
        let mut consumed: FailureEvidence = read_json(&consumed_path)?;
        validate_expected_failure(
            &consumed,
            expected_transaction_id,
            expected_request_id,
            &recovery_identity,
        )?;
        require_failure_artifact_owner(layout, &consumed)?;
        preflight_existing_failure_identity(layout, &consumed)?;
        let duplicate_current = if let Some(current) = &current
            && current.recovery_identity == consumed.recovery_identity
        {
            require_failure_artifact_owner(layout, current)?;
            require_same_failure_provenance(current, &consumed)?;
            true
        } else {
            false
        };
        let mut state = load_state(layout)?;
        acknowledge_failure_in_state(&mut state, &consumed)?;
        save_state(layout, &state)?;
        if !consumed.acknowledged {
            consumed.acknowledged = true;
            write_json_atomically(&consumed_path, &consumed)?;
        }
        if duplicate_current {
            fs::remove_file(layout.failure_evidence()?)?;
            promote_pending_failure_evidence(layout)?;
            sync_directory(&layout.active_root()?)?;
        }
        return print_failure_ack_result(
            expected_transaction_id,
            expected_request_id,
            &recovery_identity,
            layout,
            Some(configured_consumed_path),
            true,
            None,
            None,
        );
    }

    if pending_path.exists() {
        require_regular_failure_artifact(&pending_path, "pending failure evidence")?;
        let mut pending: FailureEvidence = read_json(&pending_path)?;
        validate_expected_failure(
            &pending,
            expected_transaction_id,
            expected_request_id,
            &recovery_identity,
        )?;
        require_failure_artifact_owner(layout, &pending)?;
        preflight_existing_failure_identity(layout, &pending)?;
        if current
            .as_ref()
            .is_some_and(|current| current.recovery_identity == pending.recovery_identity)
        {
            commit_newer_pending_failure_journal(layout)?;
            return acknowledge_failure_versioned(
                layout,
                expected_transaction_id,
                expected_request_id,
                &recovery_identity,
                expected_version,
            );
        }
        let mut state = load_state(layout)?;
        acknowledge_failure_in_state(&mut state, &pending)?;
        save_state(layout, &state)?;
        if !pending.acknowledged {
            pending.acknowledged = true;
            write_json_atomically(&pending_path, &pending)?;
        }
        fs::rename(&pending_path, &consumed_path)?;
        sync_directory(&layout.active_root()?)?;
        return print_failure_ack_result(
            expected_transaction_id,
            expected_request_id,
            &recovery_identity,
            layout,
            Some(configured_consumed_path),
            true,
            None,
            None,
        );
    }

    if let Some(evidence) = &current {
        validate_expected_failure(
            evidence,
            expected_transaction_id,
            expected_request_id,
            &recovery_identity,
        )?;
        require_failure_artifact_owner(layout, evidence)?;
        preflight_existing_failure_identity(layout, evidence)?;
        if pending_failure_evidence_path(layout, &recovery_identity)?.exists() {
            commit_newer_pending_failure_journal(layout)?;
            return acknowledge_failure_versioned(
                layout,
                expected_transaction_id,
                expected_request_id,
                &recovery_identity,
                expected_version,
            );
        }
        let state = load_state(layout)?;
        validate_state_failure_compatibility(&state, evidence)?;
        let mut evidence = evidence.clone();
        let mut state = load_state(layout)?;
        acknowledge_failure_in_state(&mut state, &evidence)?;
        save_state(layout, &state)?;
        if !evidence.acknowledged {
            evidence.acknowledged = true;
            persist_failure_evidence(layout, &evidence)?;
        }
        fs::rename(layout.failure_evidence()?, &consumed_path)?;
        promote_pending_failure_evidence(layout)?;
        sync_directory(&layout.active_root()?)?;
        return print_failure_ack_result(
            expected_transaction_id,
            expected_request_id,
            &recovery_identity,
            layout,
            Some(configured_consumed_path),
            true,
            None,
            None,
        );
    }

    let state = load_state(layout)?;
    let evidence = state
        .failures
        .iter()
        .find(|failure| failure.recovery_identity.as_deref() == Some(recovery_identity.as_str()))
        .cloned()
        .context("no durable or state failure evidence matches the expected recovery identity")?;
    validate_expected_failure(
        &evidence,
        expected_transaction_id,
        expected_request_id,
        &recovery_identity,
    )?;
    persist_failure_evidence(layout, &evidence)?;
    acknowledge_failure_versioned(
        layout,
        expected_transaction_id,
        expected_request_id,
        &recovery_identity,
        expected_version,
    )
}

fn print_failure_ack_result(
    transaction_id: Option<&str>,
    request_id: Option<&str>,
    recovery_identity: &str,
    layout: &Layout,
    consumed_evidence_path: Option<PathBuf>,
    consumed: bool,
    claim_id: Option<&str>,
    version_token: Option<&str>,
) -> Result<()> {
    repair_failure_queue(layout)?;
    let remaining_evidence = unacknowledged_failure_evidence_path_without_repair(layout)?.is_some();
    let consumed_artifact_exists =
        consumed_failure_evidence_path(layout, recovery_identity)?.exists();
    let consumed_evidence_path = consumed_evidence_path.filter(|_| consumed_artifact_exists);
    print_json(&serde_json::json!({
        "ok": true,
        "transactionId": transaction_id,
        "requestId": request_id,
        "recoveryIdentity": recovery_identity,
        "acknowledged": true,
        "consumed": consumed,
        "remainingEvidence": remaining_evidence,
        "evidencePath": layout.configured_failure_evidence(),
        "consumedEvidencePath": consumed_evidence_path,
        "claimId": claim_id,
        "versionToken": version_token,
    }))
}

fn configured_consumed_failure_evidence_path(layout: &Layout, recovery_identity: &str) -> PathBuf {
    layout.root.join(format!(
        ".failure-evidence.consumed-{recovery_identity}.json"
    ))
}

fn configured_claimed_failure_evidence_path(layout: &Layout, recovery_identity: &str) -> PathBuf {
    layout.root.join(format!(
        ".failure-evidence.claimed-{recovery_identity}.json"
    ))
}

fn require_failure_artifact_owner(layout: &Layout, evidence: &FailureEvidence) -> Result<()> {
    let expected = layout.root_owner_nonce()?;
    if evidence.launcher_owner_nonce.as_deref() != Some(expected.as_str()) {
        bail!("failure evidence artifact is not owned by this launcher state root");
    }
    Ok(())
}

fn require_regular_failure_artifact(path: &Path, label: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {label} {}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("{label} must be a regular file");
    }
    Ok(())
}

fn regular_failure_artifact_exists(path: &Path, label: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                bail!("{label} must be a regular file");
            }
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("failed to inspect {label} {}", path.display()))
        }
    }
}

fn preflight_existing_failure_identity(layout: &Layout, target: &FailureEvidence) -> Result<()> {
    let identity = validated_recovery_identity(target)?
        .context("failure evidence has no recovery identity")?;
    let paths = [
        layout.failure_evidence()?,
        pending_failure_evidence_path(layout, identity)?,
        consumed_failure_evidence_path(layout, identity)?,
    ];
    for path in paths {
        if !path.exists() {
            continue;
        }
        require_regular_failure_artifact(&path, "failure evidence preflight target")?;
        let existing: FailureEvidence = read_json(&path)?;
        if existing.recovery_identity.as_deref() != Some(identity) {
            continue;
        }
        require_failure_artifact_owner(layout, &existing)?;
        require_same_failure_provenance(&existing, target)?;
    }
    Ok(())
}

fn commit_newer_pending_failure_journal(layout: &Layout) -> Result<()> {
    if !layout.failure_evidence()?.exists() {
        return Ok(());
    }
    require_regular_failure_artifact(&layout.failure_evidence()?, "current failure evidence")?;
    let current: FailureEvidence = read_json(&layout.failure_evidence()?)?;
    require_failure_artifact_owner(layout, &current)?;
    let identity = validated_recovery_identity(&current)?
        .context("current failure evidence has no recovery identity")?;
    let pending_path = pending_failure_evidence_path(layout, identity)?;
    if !pending_path.exists() {
        return Ok(());
    }
    require_regular_failure_artifact(&pending_path, "pending failure evidence journal")?;
    let mut pending: FailureEvidence = read_json(&pending_path)?;
    require_failure_artifact_owner(layout, &pending)?;
    if validated_recovery_identity(&pending)? != Some(identity) {
        bail!("pending failure evidence journal identity does not match current");
    }

    // Preflight every immutable field before the first mutation. The pending
    // payload is the newer, full journal record; current contributes only
    // fields whose contract is explicitly monotonic.
    require_same_failure_provenance(&current, &pending)?;
    if pending.failed_build_hash.is_none() {
        pending.failed_build_hash = current.failed_build_hash;
    }
    pending.acknowledged |= current.acknowledged;

    // write_json_atomically commits and syncs current before the journal is
    // removed. A crash at either boundary leaves repairable current+pending
    // or an already committed current.
    write_json_atomically(&layout.failure_evidence()?, &pending)?;
    fs::remove_file(&pending_path)?;
    sync_directory(&layout.active_root()?)?;
    Ok(())
}

fn promote_pending_failure_evidence(layout: &Layout) -> Result<Option<PathBuf>> {
    if layout.failure_evidence()?.exists() {
        return Ok(None);
    }
    let mut pending = Vec::new();
    for entry in fs::read_dir(layout.active_root()?)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(identity) = name
            .strip_prefix(".failure-evidence.pending-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        Uuid::parse_str(identity)
            .context("pending failure evidence filename has invalid identity")?;
        let metadata = entry.file_type()?;
        if !metadata.is_file() {
            bail!("pending failure evidence must be a regular file");
        }
        let evidence: FailureEvidence = read_json(&entry.path())?;
        require_failure_artifact_owner(layout, &evidence)?;
        if evidence.recovery_identity.as_deref() != Some(identity) {
            bail!("pending failure evidence filename does not match its identity");
        }
        if !evidence.acknowledged {
            pending.push((evidence.occurred_at, identity.to_string(), entry.path()));
        }
    }
    pending.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let Some((_, _, path)) = pending.into_iter().next() else {
        return Ok(None);
    };
    fs::rename(&path, layout.failure_evidence()?)?;
    sync_directory(&layout.active_root()?)?;
    Ok(Some(layout.configured_failure_evidence()))
}

fn list_pending_failure_evidence(layout: &Layout) -> Result<Vec<FailureEvidence>> {
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
        Uuid::parse_str(identity)
            .context("pending failure evidence filename has invalid identity")?;
        if !entry.file_type()?.is_file() {
            bail!("pending failure evidence must be a regular file");
        }
        let evidence: FailureEvidence = read_json(&entry.path())?;
        require_failure_artifact_owner(layout, &evidence)?;
        if evidence.recovery_identity.as_deref() != Some(identity) {
            bail!("pending failure evidence filename does not match recovery identity");
        }
        pending.push(evidence);
    }
    pending.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.recovery_identity.cmp(&right.recovery_identity))
    });
    Ok(pending)
}

fn list_failure_claims(layout: &Layout) -> Result<Vec<FailureEvidenceClaim>> {
    let owner_nonce = layout.root_owner_nonce()?;
    let mut claims = Vec::new();
    for entry in fs::read_dir(layout.active_root()?)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(identity) = name
            .strip_prefix(".failure-evidence.claimed-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        require_regular_failure_artifact(&entry.path(), "claimed failure evidence")?;
        let parsed = Uuid::parse_str(identity)
            .context("claimed failure evidence filename has invalid identity")?;
        if parsed.to_string() != identity {
            bail!("claimed failure evidence filename identity is not canonical");
        }
        let claim: FailureEvidenceClaim = read_json(&entry.path())?;
        validate_failure_evidence_claim(&claim, &owner_nonce)?;
        if claim.recovery_identity != identity {
            bail!("claimed failure evidence filename does not match recovery identity");
        }
        claims.push(claim);
    }
    claims.sort_by(|left, right| {
        left.evidence
            .occurred_at
            .cmp(&right.evidence.occurred_at)
            .then_with(|| left.recovery_identity.cmp(&right.recovery_identity))
    });
    Ok(claims)
}

fn retain_consumed_failure_evidence(
    layout: &Layout,
    state: &runtime_launcher::RuntimeState,
) -> Result<Vec<FailureEvidence>> {
    let mut consumed = Vec::new();
    for entry in fs::read_dir(layout.active_root()?)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Some(identity) = name
            .strip_prefix(".failure-evidence.consumed-")
            .and_then(|name| name.strip_suffix(".json"))
        else {
            continue;
        };
        require_regular_failure_artifact(&entry.path(), "consumed failure evidence")?;
        let evidence: FailureEvidence = read_json(&entry.path())?;
        require_failure_artifact_owner(layout, &evidence)?;
        validate_claim_derived_consumed(&evidence, &layout.root_owner_nonce()?)?;
        if validated_recovery_identity(&evidence)? != Some(identity) || !evidence.acknowledged {
            bail!("consumed failure evidence identity or acknowledgement is invalid");
        }
        if let Some(existing) = state
            .failures
            .iter()
            .find(|existing| existing.recovery_identity.as_deref() == Some(identity))
        {
            require_same_failure_provenance(existing, &evidence)?;
        }
        consumed.push((
            evidence.occurred_at,
            identity.to_string(),
            entry.path(),
            evidence,
        ));
    }
    consumed.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    let remove_count = consumed
        .len()
        .saturating_sub(MAX_CONSUMED_FAILURE_ARTIFACTS);
    for (_, _, path, _) in consumed.iter().take(remove_count) {
        fs::remove_file(path)?;
    }
    if remove_count > 0 {
        sync_directory(&layout.active_root()?)?;
    }
    Ok(consumed
        .into_iter()
        .skip(remove_count)
        .map(|(_, _, _, evidence)| evidence)
        .collect())
}

fn rebuild_failure_state_from_queue(
    layout: &Layout,
    state: &mut runtime_launcher::RuntimeState,
    consumed: &[FailureEvidence],
) -> Result<()> {
    let mut unacknowledged = Vec::new();
    if layout.failure_evidence()?.exists() {
        let evidence: FailureEvidence = read_json(&layout.failure_evidence()?)?;
        require_failure_artifact_owner(layout, &evidence)?;
        if !evidence.acknowledged {
            merge_failure_record(&mut unacknowledged, evidence)?;
        }
    }
    for evidence in list_pending_failure_evidence(layout)? {
        if !evidence.acknowledged {
            merge_failure_record(&mut unacknowledged, evidence)?;
        }
    }
    for claim in list_failure_claims(layout)? {
        merge_failure_record(&mut unacknowledged, claim.evidence)?;
    }
    unacknowledged.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.recovery_identity.cmp(&right.recovery_identity))
    });
    let consumed_capacity = MAX_CONSUMED_FAILURE_ARTIFACTS.saturating_sub(unacknowledged.len());
    let mut failures = unacknowledged;
    for evidence in consumed.iter().rev().take(consumed_capacity).rev().cloned() {
        merge_failure_record(&mut failures, evidence)?;
    }
    state.failures = failures;
    Ok(())
}

fn merge_failure_record(
    records: &mut Vec<FailureEvidence>,
    evidence: FailureEvidence,
) -> Result<()> {
    let identity = validated_recovery_identity(&evidence)?
        .context("failure evidence has no recovery identity")?;
    if let Some(existing) = records
        .iter_mut()
        .find(|existing| existing.recovery_identity.as_deref() == Some(identity))
    {
        require_same_failure_provenance(existing, &evidence)?;
        if existing.failed_build_hash.is_none() {
            existing.failed_build_hash = evidence.failed_build_hash;
        }
        existing.acknowledged |= evidence.acknowledged;
    } else {
        records.push(evidence);
    }
    Ok(())
}

fn validate_expected_failure(
    evidence: &FailureEvidence,
    expected_transaction_id: Option<&str>,
    expected_request_id: Option<&str>,
    expected_recovery_identity: &str,
) -> Result<()> {
    let actual_recovery_identity = validated_recovery_identity(evidence)?
        .context("failure evidence has no recovery identity")?;
    if actual_recovery_identity != expected_recovery_identity
        || evidence.transaction_id.as_deref() != expected_transaction_id
        || evidence.request_id.as_deref() != expected_request_id
    {
        bail!(
            "failure evidence identity mismatch: expected recovery={expected_recovery_identity:?} transaction={expected_transaction_id:?} request={expected_request_id:?}, found recovery={actual_recovery_identity:?} transaction={:?} request={:?}",
            evidence.transaction_id,
            evidence.request_id
        );
    }
    Ok(())
}

fn acknowledge_failure_in_state(
    state: &mut runtime_launcher::RuntimeState,
    evidence: &FailureEvidence,
) -> Result<()> {
    let recovery_identity = validated_recovery_identity(evidence)?
        .context("failure evidence has no recovery identity")?;
    if let Some(existing) = state
        .failures
        .iter_mut()
        .find(|failure| failure.recovery_identity.as_deref() == Some(recovery_identity))
    {
        require_same_failure_provenance(existing, evidence)?;
        if existing.failed_build_hash.is_none() {
            existing.failed_build_hash = evidence.failed_build_hash.clone();
        }
        if existing.launcher_owner_nonce.is_none() {
            existing.launcher_owner_nonce = evidence.launcher_owner_nonce.clone();
        }
        existing.acknowledged = true;
    } else {
        if let Some(existing) = state.failures.iter().find(|failure| {
            failure_identity_key(failure).is_ok_and(|key| {
                failure_identity_key(evidence).is_ok_and(|expected| key == expected)
            })
        }) {
            require_same_failure_provenance(existing, evidence)?;
            bail!("matching failure provenance has a different recovery identity");
        }
        let mut repaired = evidence.clone();
        repaired.acknowledged = true;
        record_failure(state, repaired);
    }
    Ok(())
}

fn validate_state_failure_compatibility(
    state: &runtime_launcher::RuntimeState,
    evidence: &FailureEvidence,
) -> Result<()> {
    let recovery_identity = validated_recovery_identity(evidence)?
        .context("failure evidence has no recovery identity")?;
    let evidence_key = failure_identity_key(evidence)?;
    for existing in &state.failures {
        if existing.recovery_identity.as_deref() == Some(recovery_identity) {
            require_same_failure_provenance(existing, evidence)?;
        } else if failure_identity_key(existing)? == evidence_key {
            require_same_failure_provenance(existing, evidence)?;
            bail!("matching failure provenance has a different recovery identity");
        }
    }
    Ok(())
}

fn require_same_failure_provenance(
    existing: &FailureEvidence,
    evidence: &FailureEvidence,
) -> Result<()> {
    // These fields identify one recovery fact and must not change as it moves
    // between transaction, state, current evidence, and consumed evidence.
    // failed_build_hash is populated after some requests are created, so None
    // may converge to Some; two concrete but different hashes still conflict.
    let failed_hash_compatible = match (
        existing.failed_build_hash.as_deref(),
        evidence.failed_build_hash.as_deref(),
    ) {
        (Some(existing), Some(evidence)) => existing == evidence,
        _ => true,
    };
    let owner_nonce_compatible = match (
        existing.launcher_owner_nonce.as_deref(),
        evidence.launcher_owner_nonce.as_deref(),
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
        || !owner_nonce_compatible
    {
        bail!("failure recovery identity has conflicting immutable provenance");
    }
    Ok(())
}

fn failure_identity_key(evidence: &FailureEvidence) -> Result<String> {
    // Legacy evidence has no recovery identity. Use only immutable provenance
    // here; failed_build_hash is deliberately excluded because it may be
    // filled after the initial FailureEvidence is created.
    Ok(serde_json::to_string(&(
        &evidence.occurred_at,
        &evidence.transaction_id,
        &evidence.request_id,
        &evidence.requested_by_thread_id,
        &evidence.mode,
        &evidence.build_id,
        &evidence.source_commit,
        &evidence.manifest_hash,
        &evidence.app_bundle_path,
    ))?)
}

fn validated_recovery_identity(evidence: &FailureEvidence) -> Result<Option<&str>> {
    let Some(identity) = evidence.recovery_identity.as_deref() else {
        return Ok(None);
    };
    let parsed = Uuid::parse_str(identity)
        .with_context(|| format!("invalid failure recovery identity {identity:?}"))?;
    if parsed.to_string() != identity {
        bail!("failure recovery identity is not canonical");
    }
    Ok(Some(identity))
}

fn register_failure_recovery_identity(
    identities_by_key: &mut BTreeMap<String, String>,
    keys_by_identity: &mut BTreeMap<String, String>,
    failed_hashes_by_identity: &mut BTreeMap<String, Option<String>>,
    evidence: &FailureEvidence,
) -> Result<()> {
    let Some(identity) = validated_recovery_identity(evidence)? else {
        return Ok(());
    };
    let key = failure_identity_key(evidence)?;
    if let Some(existing) = identities_by_key.insert(key.clone(), identity.to_string())
        && existing != identity
    {
        bail!("matching failure evidence records have conflicting recovery identities");
    }
    if let Some(existing) = keys_by_identity.insert(identity.to_string(), key.clone())
        && existing != key
    {
        bail!("failure recovery identity is reused by different evidence records");
    }
    register_failure_failed_hash(failed_hashes_by_identity, identity, evidence)?;
    Ok(())
}

fn register_failure_failed_hash(
    failed_hashes_by_identity: &mut BTreeMap<String, Option<String>>,
    identity: &str,
    evidence: &FailureEvidence,
) -> Result<()> {
    if let Some(current) = evidence.failed_build_hash.as_deref() {
        if let Some(existing) = failed_hashes_by_identity
            .get(identity)
            .and_then(|hash| hash.as_deref())
            && existing != current
        {
            bail!("failure recovery identity has conflicting failed build hashes");
        }
        failed_hashes_by_identity.insert(identity.to_string(), Some(current.to_string()));
    } else {
        failed_hashes_by_identity
            .entry(identity.to_string())
            .or_insert(None);
    }
    Ok(())
}

fn migrate_failure_recovery_identity(
    identities_by_key: &mut BTreeMap<String, String>,
    keys_by_identity: &mut BTreeMap<String, String>,
    failed_hashes_by_identity: &mut BTreeMap<String, Option<String>>,
    evidence: &mut FailureEvidence,
) -> Result<bool> {
    if validated_recovery_identity(evidence)?.is_some() {
        return Ok(false);
    }
    let key = failure_identity_key(evidence)?;
    let identity = if let Some(identity) = identities_by_key.get(&key) {
        identity.clone()
    } else {
        let identity = loop {
            let candidate = Uuid::new_v4().to_string();
            if !keys_by_identity.contains_key(&candidate) {
                break candidate;
            }
        };
        identities_by_key.insert(key.clone(), identity.clone());
        keys_by_identity.insert(identity.clone(), key);
        identity
    };
    register_failure_failed_hash(failed_hashes_by_identity, &identity, evidence)?;
    evidence.recovery_identity = Some(identity);
    Ok(true)
}

struct LegacyFailureArtifact {
    path: PathBuf,
    evidence: FailureEvidence,
    scoped_identity: Option<String>,
}

fn normalize_failure_evidence(
    identities_by_key: &mut BTreeMap<String, String>,
    keys_by_identity: &mut BTreeMap<String, String>,
    failed_hashes_by_identity: &mut BTreeMap<String, Option<String>>,
    evidence: &mut FailureEvidence,
    scoped_identity: Option<&str>,
    owner_nonce: &str,
) -> Result<bool> {
    let mut changed = false;
    if let Some(existing_owner) = evidence.launcher_owner_nonce.as_deref()
        && existing_owner != owner_nonce
    {
        bail!("failure evidence belongs to another launcher state root");
    }
    if evidence.launcher_owner_nonce.is_none() {
        evidence.launcher_owner_nonce = Some(owner_nonce.to_string());
        changed = true;
    }
    if let Some(identity) = validated_recovery_identity(evidence)? {
        if let Some(scoped_identity) = scoped_identity
            && identity != scoped_identity
        {
            bail!("failure queue artifact filename does not match recovery identity");
        }
        register_failure_recovery_identity(
            identities_by_key,
            keys_by_identity,
            failed_hashes_by_identity,
            evidence,
        )?;
        return Ok(changed);
    }
    if let Some(scoped_identity) = scoped_identity {
        evidence.recovery_identity = Some(scoped_identity.to_string());
        register_failure_recovery_identity(
            identities_by_key,
            keys_by_identity,
            failed_hashes_by_identity,
            evidence,
        )?;
        return Ok(true);
    }
    changed |= migrate_failure_recovery_identity(
        identities_by_key,
        keys_by_identity,
        failed_hashes_by_identity,
        evidence,
    )?;
    Ok(changed)
}

fn validate_failure_identity_migration_plan(
    artifacts: &[LegacyFailureArtifact],
    claims: &[FailureEvidenceClaim],
    state: &runtime_launcher::RuntimeState,
    transaction: Option<&Transaction>,
    owner_nonce: &str,
) -> Result<()> {
    let mut normalized_records = Vec::new();
    for artifact in artifacts {
        if artifact.evidence.launcher_owner_nonce.as_deref() != Some(owner_nonce) {
            bail!("normalized failure artifact owner does not match state root");
        }
        let identity = validated_recovery_identity(&artifact.evidence)?
            .context("normalized failure artifact has no recovery identity")?;
        if let Some(scoped_identity) = artifact.scoped_identity.as_deref()
            && scoped_identity != identity
        {
            bail!("normalized failure artifact filename does not match recovery identity");
        }
        normalized_records.push(&artifact.evidence);
    }
    for evidence in &state.failures {
        if evidence.launcher_owner_nonce.as_deref() != Some(owner_nonce)
            || validated_recovery_identity(evidence)?.is_none()
        {
            bail!("normalized failure state record is incomplete");
        }
        normalized_records.push(evidence);
    }
    if let Some(transaction) = transaction {
        if let Some(evidence) = &transaction.rollback_failure_evidence {
            if evidence.launcher_owner_nonce.as_deref() != Some(owner_nonce)
                || validated_recovery_identity(evidence)?.is_none()
            {
                bail!("normalized transaction failure evidence is incomplete");
            }
            normalized_records.push(evidence);
        }
        if let Some(rollback) = &transaction.post_ready_rollback {
            let evidence = &rollback.failure_evidence;
            if evidence.launcher_owner_nonce.as_deref() != Some(owner_nonce)
                || validated_recovery_identity(evidence)?.is_none()
            {
                bail!("normalized transaction failure evidence is incomplete");
            }
            normalized_records.push(evidence);
        }
    }

    for claim in claims {
        validate_failure_evidence_claim(claim, owner_nonce)?;
        let matching = normalized_records.iter().copied().filter(|evidence| {
            evidence.recovery_identity.as_deref() == Some(claim.recovery_identity.as_str())
        });
        for evidence in matching {
            require_same_failure_provenance(evidence, &claim.evidence)?;
            if claim.evidence.failed_build_hash.is_none() && evidence.failed_build_hash.is_some() {
                bail!("normalized failure record has a hash missing from its immutable claim");
            }
        }
        for consumed in artifacts.iter().filter(|artifact| {
            artifact
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".failure-evidence.consumed-"))
                && artifact.evidence.recovery_identity.as_deref()
                    == Some(claim.recovery_identity.as_str())
        }) {
            require_consumed_matches_claim(&consumed.evidence, claim)?;
        }
    }

    for consumed in artifacts
        .iter()
        .filter(|artifact| {
            artifact
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(".failure-evidence.consumed-"))
        })
        .map(|artifact| &artifact.evidence)
        .filter(|evidence| evidence.claim_id.is_some())
    {
        if !consumed.acknowledged {
            bail!("claim-derived consumed evidence is not acknowledged");
        }
        let identity = consumed
            .recovery_identity
            .as_deref()
            .context("claim-derived consumed evidence has no recovery identity")?;
        for evidence in normalized_records
            .iter()
            .copied()
            .filter(|evidence| evidence.recovery_identity.as_deref() == Some(identity))
        {
            require_same_failure_provenance(consumed, evidence)?;
            if consumed.failed_build_hash.is_none() && evidence.failed_build_hash.is_some() {
                bail!("normalized failure record has a hash missing from frozen consumed evidence");
            }
        }
    }
    Ok(())
}

fn repair_failure_recovery_identities(layout: &Layout) -> Result<()> {
    let owner_nonce = layout.root_owner_nonce()?;
    let mut state = load_state(layout)?;
    let mut artifacts = Vec::new();
    let mut claims = Vec::new();
    if regular_failure_artifact_exists(&layout.failure_evidence()?, "current failure evidence")? {
        artifacts.push(LegacyFailureArtifact {
            path: layout.failure_evidence()?,
            evidence: read_json(&layout.failure_evidence()?)?,
            scoped_identity: None,
        });
    }
    for entry in fs::read_dir(layout.active_root()?)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with(".failure-evidence.claimed-") {
            require_regular_failure_artifact(&entry.path(), "claimed failure evidence")?;
            let identity = name
                .strip_prefix(".failure-evidence.claimed-")
                .and_then(|name| name.strip_suffix(".json"))
                .context("claimed failure evidence filename is malformed")?;
            let parsed = Uuid::parse_str(identity)
                .context("claimed failure evidence filename has invalid identity")?;
            if parsed.to_string() != identity {
                bail!("claimed failure evidence filename identity is not canonical");
            }
            let claim: FailureEvidenceClaim = read_json(&entry.path())?;
            validate_failure_evidence_claim(&claim, &owner_nonce)?;
            if claim.recovery_identity != identity {
                bail!("claimed failure evidence filename does not match recovery identity");
            }
            claims.push(claim);
            continue;
        }
        let (prefix, label) = if name.starts_with(".failure-evidence.pending-") {
            (".failure-evidence.pending-", "pending failure evidence")
        } else if name.starts_with(".failure-evidence.consumed-") {
            (".failure-evidence.consumed-", "consumed failure evidence")
        } else {
            continue;
        };
        require_regular_failure_artifact(&entry.path(), label)?;
        let identity = name
            .strip_prefix(prefix)
            .and_then(|name| name.strip_suffix(".json"))
            .with_context(|| format!("{label} filename is malformed"))?;
        let parsed = Uuid::parse_str(identity)
            .with_context(|| format!("{label} filename has invalid identity"))?;
        if parsed.to_string() != identity {
            bail!("{label} filename identity is not canonical");
        }
        let evidence: FailureEvidence = read_json(&entry.path())?;
        if prefix == ".failure-evidence.consumed-" {
            validate_claim_derived_consumed(&evidence, &owner_nonce)?;
        }
        if let Some(payload_identity) = validated_recovery_identity(&evidence)?
            && payload_identity != identity
        {
            bail!("{label} filename does not match recovery identity");
        }
        artifacts.push(LegacyFailureArtifact {
            path: entry.path(),
            evidence,
            scoped_identity: Some(identity.to_string()),
        });
    }
    let mut transaction = layout
        .transaction()?
        .exists()
        .then(|| read_json::<Transaction>(&layout.transaction()?))
        .transpose()?;
    let mut identities_by_key = BTreeMap::new();
    let mut keys_by_identity = BTreeMap::new();
    let mut failed_hashes_by_identity = BTreeMap::new();

    // First register every already-known identity, including identities
    // supplied by scoped filenames for legacy payloads. No writes occur until
    // all sources have been normalized and cross-validated.
    for artifact in &artifacts {
        if let Some(existing_owner) = artifact.evidence.launcher_owner_nonce.as_deref()
            && existing_owner != owner_nonce
        {
            bail!("failure queue artifact belongs to another launcher state root");
        }
        let mut known = artifact.evidence.clone();
        if known.recovery_identity.is_none()
            && let Some(identity) = artifact.scoped_identity.as_deref()
        {
            known.recovery_identity = Some(identity.to_string());
        }
        register_failure_recovery_identity(
            &mut identities_by_key,
            &mut keys_by_identity,
            &mut failed_hashes_by_identity,
            &known,
        )?;
    }
    for claim in &claims {
        register_failure_recovery_identity(
            &mut identities_by_key,
            &mut keys_by_identity,
            &mut failed_hashes_by_identity,
            &claim.evidence,
        )?;
    }
    for evidence in &state.failures {
        if let Some(existing_owner) = evidence.launcher_owner_nonce.as_deref()
            && existing_owner != owner_nonce
        {
            bail!("failure state record belongs to another launcher state root");
        }
        register_failure_recovery_identity(
            &mut identities_by_key,
            &mut keys_by_identity,
            &mut failed_hashes_by_identity,
            evidence,
        )?;
    }
    if let Some(transaction) = &transaction {
        if let Some(evidence) = &transaction.rollback_failure_evidence {
            if let Some(existing_owner) = evidence.launcher_owner_nonce.as_deref()
                && existing_owner != owner_nonce
            {
                bail!("transaction failure evidence belongs to another launcher state root");
            }
            register_failure_recovery_identity(
                &mut identities_by_key,
                &mut keys_by_identity,
                &mut failed_hashes_by_identity,
                evidence,
            )?;
        }
        if let Some(rollback) = &transaction.post_ready_rollback {
            if let Some(existing_owner) = rollback.failure_evidence.launcher_owner_nonce.as_deref()
                && existing_owner != owner_nonce
            {
                bail!("transaction failure evidence belongs to another launcher state root");
            }
            register_failure_recovery_identity(
                &mut identities_by_key,
                &mut keys_by_identity,
                &mut failed_hashes_by_identity,
                &rollback.failure_evidence,
            )?;
        }
    }

    let mut artifact_changes = Vec::new();
    for artifact in &mut artifacts {
        let before = artifact.evidence.clone();
        normalize_failure_evidence(
            &mut identities_by_key,
            &mut keys_by_identity,
            &mut failed_hashes_by_identity,
            &mut artifact.evidence,
            artifact.scoped_identity.as_deref(),
            &owner_nonce,
        )?;
        if artifact.evidence != before {
            artifact_changes.push((artifact.path.clone(), artifact.evidence.clone()));
        }
    }
    let mut transaction_changed = false;
    if let Some(transaction) = &mut transaction {
        if let Some(evidence) = &mut transaction.rollback_failure_evidence {
            transaction_changed |= normalize_failure_evidence(
                &mut identities_by_key,
                &mut keys_by_identity,
                &mut failed_hashes_by_identity,
                evidence,
                None,
                &owner_nonce,
            )?;
        }
        if let Some(rollback) = &mut transaction.post_ready_rollback {
            transaction_changed |= normalize_failure_evidence(
                &mut identities_by_key,
                &mut keys_by_identity,
                &mut failed_hashes_by_identity,
                &mut rollback.failure_evidence,
                None,
                &owner_nonce,
            )?;
        }
    }
    let mut state_changed = false;
    for evidence in &mut state.failures {
        state_changed |= normalize_failure_evidence(
            &mut identities_by_key,
            &mut keys_by_identity,
            &mut failed_hashes_by_identity,
            evidence,
            None,
            &owner_nonce,
        )?;
    }

    // The normalized in-memory view is a migration plan. Validate the entire
    // queue/state/transaction graph, including immutable claim completion,
    // before committing any legacy owner or identity repair.
    validate_failure_identity_migration_plan(
        &artifacts,
        &claims,
        &state,
        transaction.as_ref(),
        &owner_nonce,
    )?;

    for (path, evidence) in artifact_changes {
        write_json_atomically(&path, &evidence)?;
    }
    if transaction_changed {
        write_json_atomically(
            &layout.transaction()?,
            transaction
                .as_ref()
                .context("missing activation transaction")?,
        )?;
    }
    if state_changed {
        save_state(layout, &state)?;
    }
    Ok(())
}

fn repair_failure_queue(layout: &Layout) -> Result<()> {
    repair_failure_recovery_identities(layout)?;
    let preflight = preflight_failure_queue(layout)?;
    collapse_terminal_failure_duplicates(layout, preflight)?;
    collapse_claimed_failure_duplicates(layout)?;
    commit_newer_pending_failure_journal(layout)?;
    let owner_nonce = layout.root_owner_nonce()?;
    let mut state = load_state(layout)?;
    let mut artifact_identities = BTreeSet::new();
    let mut state_changed = false;

    for claim in list_failure_claims(layout)? {
        artifact_identities.insert(claim.recovery_identity.clone());
        state_changed |= merge_failure_into_state(&mut state, &claim.evidence)?;
    }

    let mut artifact_paths = Vec::new();
    if layout.failure_evidence()?.exists() {
        artifact_paths.push(layout.failure_evidence()?);
    }
    for entry in fs::read_dir(layout.active_root()?)? {
        let entry = entry?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if (name.starts_with(".failure-evidence.pending-")
            || name.starts_with(".failure-evidence.consumed-"))
            && name.ends_with(".json")
        {
            if !entry.file_type()?.is_file() {
                bail!("failure queue artifact must be a regular file");
            }
            artifact_paths.push(entry.path());
        }
    }

    for path in artifact_paths {
        let mut evidence: FailureEvidence = read_json(&path)?;
        let mut artifact_changed = false;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let scoped_identity = name
            .strip_prefix(".failure-evidence.pending-")
            .or_else(|| name.strip_prefix(".failure-evidence.consumed-"))
            .and_then(|name| name.strip_suffix(".json"));
        if evidence.recovery_identity.is_none()
            && let Some(identity) = scoped_identity
        {
            Uuid::parse_str(identity)
                .context("failure queue artifact filename has invalid identity")?;
            evidence.recovery_identity = Some(identity.to_string());
            artifact_changed = true;
        }
        let identity = validated_recovery_identity(&evidence)?
            .context("failure queue artifact has no recovery identity")?
            .to_string();
        if let Some(existing) = evidence.launcher_owner_nonce.as_deref()
            && existing != owner_nonce
        {
            bail!("failure queue artifact belongs to another launcher state root");
        }
        if evidence.launcher_owner_nonce.is_none() {
            evidence.launcher_owner_nonce = Some(owner_nonce.clone());
            artifact_changed = true;
        }
        if artifact_changed {
            write_json_atomically(&path, &evidence)?;
        }
        let identity_scoped_name = format!(".failure-evidence.pending-{identity}.json");
        let consumed_name = format!(".failure-evidence.consumed-{identity}.json");
        if name.starts_with(".failure-evidence.pending-") && name != identity_scoped_name {
            bail!("pending failure evidence filename does not match recovery identity");
        }
        if name.starts_with(".failure-evidence.consumed-") && name != consumed_name {
            bail!("failure queue artifact filename does not match recovery identity");
        }
        if name.starts_with(".failure-evidence.consumed-") && !evidence.acknowledged {
            evidence.acknowledged = true;
            write_json_atomically(&path, &evidence)?;
        }
        if evidence.acknowledged
            && (path == layout.failure_evidence()?
                || name.starts_with(".failure-evidence.pending-"))
        {
            let consumed = consumed_failure_evidence_path(layout, &identity)?;
            if consumed.exists() {
                require_regular_failure_artifact(&consumed, "consumed failure evidence")?;
                let mut existing: FailureEvidence = read_json(&consumed)?;
                if let Some(existing_owner) = existing.launcher_owner_nonce.as_deref()
                    && existing_owner != owner_nonce
                {
                    bail!("consumed failure evidence belongs to another launcher state root");
                }
                let mut existing_changed = false;
                if existing.launcher_owner_nonce.is_none() {
                    existing.launcher_owner_nonce = Some(owner_nonce.clone());
                    existing_changed = true;
                }
                if !existing.acknowledged {
                    existing.acknowledged = true;
                    existing_changed = true;
                }
                if existing.failed_build_hash.is_none() && evidence.failed_build_hash.is_some() {
                    existing.failed_build_hash = evidence.failed_build_hash.clone();
                    existing_changed = true;
                }
                if validated_recovery_identity(&existing)? != Some(identity.as_str()) {
                    bail!("consumed failure evidence filename does not match recovery identity");
                }
                require_same_failure_provenance(&existing, &evidence)?;
                if existing_changed {
                    write_json_atomically(&consumed, &existing)?;
                }
                fs::remove_file(&path)?;
            } else {
                fs::rename(&path, &consumed)?;
            }
            sync_directory(&layout.active_root()?)?;
        }
        state_changed |= merge_failure_into_state(&mut state, &evidence)?;
        artifact_identities.insert(identity);
    }

    let missing = state
        .failures
        .iter()
        .filter(|evidence| !evidence.acknowledged)
        .filter_map(|evidence| {
            evidence
                .recovery_identity
                .as_ref()
                .filter(|identity| !artifact_identities.contains(*identity))
                .map(|_| evidence.clone())
        })
        .collect::<Vec<_>>();
    for evidence in missing {
        let persisted = persist_failure_evidence(layout, &evidence)?;
        artifact_identities.insert(
            persisted
                .recovery_identity
                .clone()
                .context("persisted failure evidence has no recovery identity")?,
        );
        state_changed |= merge_failure_into_state(&mut state, &persisted)?;
    }
    promote_pending_failure_evidence(layout)?;
    let consumed = retain_consumed_failure_evidence(layout, &state)?;
    let failures_before = state.failures.clone();
    rebuild_failure_state_from_queue(layout, &mut state, &consumed)?;
    if state_changed || state.failures != failures_before {
        save_state(layout, &state)?;
    }
    Ok(())
}

struct TerminalFailureRepair {
    consumed_path: PathBuf,
    original_consumed: FailureEvidence,
    completed: FailureEvidence,
    active_paths: Vec<PathBuf>,
}

fn preflight_failure_queue(layout: &Layout) -> Result<Vec<TerminalFailureRepair>> {
    let owner_nonce = layout.root_owner_nonce()?;
    let state = load_state(layout)?;
    preflight_failure_claims(layout, &state, &owner_nonce)?;
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
            || name.starts_with(".failure-evidence.consumed-")
        {
            require_regular_failure_artifact(&entry.path(), "failure queue artifact")?;
            paths.push(entry.path());
        }
    }

    let mut records_by_identity: BTreeMap<
        String,
        Vec<(PathBuf, FailureEvidence, FailureEvidence, bool, bool)>,
    > = BTreeMap::new();
    for path in paths {
        let original: FailureEvidence = read_json(&path)?;
        if original.launcher_owner_nonce.as_deref() != Some(owner_nonce.as_str()) {
            bail!("failure queue artifact belongs to another launcher state root");
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("failure queue artifact filename is not valid UTF-8")?;
        let (scoped_identity, is_pending, is_consumed) = if path == layout.failure_evidence()? {
            (None, false, false)
        } else if let Some(identity) = name.strip_prefix(".failure-evidence.pending-") {
            (
                Some(
                    identity
                        .strip_suffix(".json")
                        .context("pending failure evidence filename is malformed")?,
                ),
                true,
                false,
            )
        } else if let Some(identity) = name.strip_prefix(".failure-evidence.consumed-") {
            (
                Some(
                    identity
                        .strip_suffix(".json")
                        .context("consumed failure evidence filename is malformed")?,
                ),
                false,
                true,
            )
        } else {
            bail!("failure queue preflight found an unexpected artifact");
        };
        if is_consumed {
            validate_claim_derived_consumed(&original, &owner_nonce)?;
        }
        if let Some(scoped_identity) = scoped_identity {
            let parsed = Uuid::parse_str(scoped_identity)
                .context("failure queue artifact filename has invalid identity")?;
            if parsed.to_string() != scoped_identity {
                bail!("failure queue artifact filename identity is not canonical");
            }
        }
        let payload_identity = validated_recovery_identity(&original)?
            .context("failure queue artifact has no recovery identity")?;
        if let Some(scoped_identity) = scoped_identity
            && scoped_identity != payload_identity
        {
            bail!("failure queue artifact filename does not match recovery identity");
        }
        let identity = payload_identity;
        let mut normalized = original.clone();
        normalized.recovery_identity = Some(identity.to_string());
        records_by_identity
            .entry(identity.to_string())
            .or_default()
            .push((path, original, normalized, is_pending, is_consumed));
    }

    let mut repairs = Vec::new();
    for (identity, records) in records_by_identity {
        let canonical = &records[0].2;
        for (_, _, evidence, _, _) in records.iter().skip(1) {
            require_same_failure_provenance(canonical, evidence)?;
        }
        for state_evidence in state
            .failures
            .iter()
            .filter(|evidence| evidence.recovery_identity.as_deref() == Some(identity.as_str()))
        {
            let mut normalized_state = state_evidence.clone();
            if let Some(existing_owner) = normalized_state.launcher_owner_nonce.as_deref()
                && existing_owner != owner_nonce
            {
                bail!("failure state record belongs to another launcher state root");
            }
            normalized_state.launcher_owner_nonce = Some(owner_nonce.clone());
            require_same_failure_provenance(canonical, &normalized_state)?;
        }
        let Some((consumed_path, original_consumed, consumed, _, _)) = records
            .iter()
            .find(|(_, _, _, _, is_consumed)| *is_consumed)
        else {
            continue;
        };
        let mut completed = consumed.clone();
        completed.acknowledged = true;
        let claim_derived = consumed.claim_id.is_some();
        if claim_derived && !consumed.acknowledged {
            bail!("claim-derived consumed failure evidence is not acknowledged");
        }
        for (_, _, evidence, _, _) in &records {
            match (
                completed.failed_build_hash.as_deref(),
                evidence.failed_build_hash.as_deref(),
            ) {
                (Some(known), Some(existing)) if known != existing => {
                    bail!("terminal failure identity has conflicting failed build hashes");
                }
                (None, Some(existing)) => {
                    if claim_derived {
                        bail!("failure artifact has a hash missing from frozen consumed evidence");
                    }
                    completed.failed_build_hash = Some(existing.to_string());
                }
                _ => {}
            }
        }
        for state_evidence in state
            .failures
            .iter()
            .filter(|evidence| evidence.recovery_identity.as_deref() == Some(identity.as_str()))
        {
            match (
                completed.failed_build_hash.as_deref(),
                state_evidence.failed_build_hash.as_deref(),
            ) {
                (Some(known), Some(existing)) if known != existing => {
                    bail!("terminal failure identity has conflicting failed build hashes");
                }
                (None, Some(existing)) => {
                    if claim_derived {
                        bail!("failure state has a hash missing from frozen consumed evidence");
                    }
                    completed.failed_build_hash = Some(existing.to_string());
                }
                _ => {}
            }
        }
        let active_paths = records
            .iter()
            .filter(|(_, _, _, is_pending, is_consumed)| *is_pending || !*is_consumed)
            .map(|(path, _, _, _, _)| path.clone())
            .collect();
        repairs.push(TerminalFailureRepair {
            consumed_path: consumed_path.clone(),
            original_consumed: original_consumed.clone(),
            completed,
            active_paths,
        });
    }
    Ok(repairs)
}

fn preflight_failure_claims(
    layout: &Layout,
    state: &runtime_launcher::RuntimeState,
    owner_nonce: &str,
) -> Result<()> {
    for claim in list_failure_claims(layout)? {
        validate_failure_evidence_claim(&claim, owner_nonce)?;
        for state_evidence in state.failures.iter().filter(|evidence| {
            evidence.recovery_identity.as_deref() == Some(claim.recovery_identity.as_str())
        }) {
            if let Some(existing_owner) = state_evidence.launcher_owner_nonce.as_deref()
                && existing_owner != owner_nonce
            {
                bail!("failure state record belongs to another launcher state root");
            }
            require_same_failure_provenance(state_evidence, &claim.evidence)?;
            if claim.evidence.failed_build_hash.is_none()
                && state_evidence.failed_build_hash.is_some()
            {
                bail!("failure state has a hash missing from its immutable claim");
            }
        }
        for path in [
            layout.failure_evidence()?,
            pending_failure_evidence_path(layout, &claim.recovery_identity)?,
            consumed_failure_evidence_path(layout, &claim.recovery_identity)?,
        ] {
            if !regular_failure_artifact_exists(&path, "claimed duplicate evidence")? {
                continue;
            }
            let evidence: FailureEvidence = read_json(&path)?;
            require_failure_artifact_owner(layout, &evidence)?;
            if evidence.recovery_identity.as_deref() != Some(claim.recovery_identity.as_str()) {
                if path == layout.failure_evidence()? {
                    continue;
                }
                bail!("identity-scoped failure evidence does not match claim");
            }
            require_same_failure_provenance(&evidence, &claim.evidence)?;
            if claim.evidence.failed_build_hash.is_none() && evidence.failed_build_hash.is_some() {
                bail!("failure artifact has a hash missing from its immutable claim");
            }
            if path == consumed_failure_evidence_path(layout, &claim.recovery_identity)? {
                require_consumed_matches_claim(&evidence, &claim)?;
            }
        }
    }
    Ok(())
}

fn collapse_claimed_failure_duplicates(layout: &Layout) -> Result<()> {
    for claim in list_failure_claims(layout)? {
        let claimed_path = claimed_failure_evidence_path(layout, &claim.recovery_identity)?;
        let consumed_path = consumed_failure_evidence_path(layout, &claim.recovery_identity)?;
        if regular_failure_artifact_exists(&consumed_path, "consumed failure evidence")? {
            fs::remove_file(&claimed_path)?;
            sync_directory(&layout.active_root()?)?;
            continue;
        }
        for path in [
            layout.failure_evidence()?,
            pending_failure_evidence_path(layout, &claim.recovery_identity)?,
        ] {
            if !regular_failure_artifact_exists(&path, "claimed active duplicate")? {
                continue;
            }
            let evidence: FailureEvidence = read_json(&path)?;
            if evidence.recovery_identity.as_deref() != Some(claim.recovery_identity.as_str()) {
                continue;
            }
            require_same_failure_provenance(&evidence, &claim.evidence)?;
            fs::remove_file(&path)?;
            sync_directory(&layout.active_root()?)?;
        }
    }
    Ok(())
}

fn collapse_terminal_failure_duplicates(
    layout: &Layout,
    repairs: Vec<TerminalFailureRepair>,
) -> Result<()> {
    for repair in repairs {
        if repair.completed != repair.original_consumed {
            write_json_atomically(&repair.consumed_path, &repair.completed)?;
        }
        for path in repair.active_paths {
            match fs::remove_file(&path) {
                Ok(()) => sync_directory(&layout.active_root()?)?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("failed to remove terminal duplicate {}", path.display())
                    });
                }
            }
        }
    }
    Ok(())
}

fn merge_failure_into_state(
    state: &mut runtime_launcher::RuntimeState,
    evidence: &FailureEvidence,
) -> Result<bool> {
    let identity = validated_recovery_identity(evidence)?
        .context("failure evidence has no recovery identity")?;
    if let Some(existing) = state
        .failures
        .iter_mut()
        .find(|failure| failure.recovery_identity.as_deref() == Some(identity))
    {
        require_same_failure_provenance(existing, evidence)?;
        let before = existing.clone();
        if existing.failed_build_hash.is_none() {
            existing.failed_build_hash = evidence.failed_build_hash.clone();
        }
        if existing.launcher_owner_nonce.is_none() {
            existing.launcher_owner_nonce = evidence.launcher_owner_nonce.clone();
        }
        existing.acknowledged |= evidence.acknowledged;
        Ok(*existing != before)
    } else {
        let evidence_key = failure_identity_key(evidence)?;
        for existing in &state.failures {
            if failure_identity_key(existing)? == evidence_key {
                require_same_failure_provenance(existing, evidence)?;
                bail!("matching failure provenance has a different recovery identity");
            }
        }
        record_failure(state, evidence.clone());
        Ok(true)
    }
}

fn supervise(
    authority: &StateRootAuthority,
    layout: &Layout,
    app_bundle_override: Option<PathBuf>,
) -> Result<()> {
    let app_bundle = match app_bundle_override {
        Some(app_bundle) => app_bundle,
        None => infer_default_app_bundle()?,
    }
    .canonicalize()
    .context("failed to resolve supervised app bundle")?;
    let mut pending = {
        let _lock = StateRootLock::acquire(authority)?;
        reconcile_launcher_state(layout, &app_bundle)?;
        load_valid_prepared_transaction_on_startup(layout, &app_bundle)?
    };
    let mut stop_on_abnormal_current = false;

    loop {
        let status = if let Some(mut transaction) = pending.take() {
            if transaction.phase != TransactionPhase::Prepared {
                bail!("unexpected pending transaction phase");
            }
            {
                let _lock = StateRootLock::acquire(authority)?;
                if let Err(error) = activate_slot(layout, &mut transaction, &SystemCommandRunner) {
                    if transaction.phase == TransactionPhase::Prepared {
                        abort_pre_activation_candidate(
                            layout,
                            &transaction,
                            &format!(
                                "full activation validation failed before slot mutation: {error:#}"
                            ),
                        )?;
                        continue;
                    }
                    rollback_candidate(
                        layout,
                        &transaction,
                        &format!("activation or codesign failed: {error:#}"),
                        None,
                    )?;
                    continue;
                }
            }
            match supervise_candidate(authority, layout, &mut transaction) {
                Ok(CandidateOutcome::ReadyExit(status)) => {
                    stop_on_abnormal_current = false;
                    status
                }
                Ok(CandidateOutcome::FailedBeforeReady { summary, status }) => {
                    let authoritative = {
                        let _lock = StateRootLock::acquire(authority)?;
                        reload_transaction_for_rollback(layout, &transaction)?
                    };
                    let _lock = StateRootLock::acquire(authority)?;
                    rollback_candidate(layout, &authoritative, &summary, status.as_ref())?;
                    continue;
                }
                Err(error) => {
                    let authoritative = {
                        let _lock = StateRootLock::acquire(authority)?;
                        reload_transaction_for_rollback(layout, &transaction)?
                    };
                    let _lock = StateRootLock::acquire(authority)?;
                    rollback_candidate(
                        layout,
                        &authoritative,
                        &format!("candidate launch or readiness supervision failed: {error:#}"),
                        None,
                    )?;
                    continue;
                }
            }
        } else {
            let failure_path = {
                let _lock = StateRootLock::acquire(authority)?;
                unacknowledged_failure_evidence_path(layout)?
            };
            launch_current(authority, &app_bundle, None, failure_path.as_deref())?
        };

        let _lock = StateRootLock::acquire(authority)?;
        let mut state = reconcile_launcher_state(layout, &app_bundle)?;
        if status.code() == Some(RECOVERY_RESTART_EXIT_CODE) {
            append_launcher_log(layout, "runtime requested supervised recovery restart")?;
            continue;
        }
        if status.success() {
            return Ok(());
        }
        if status.code() == Some(COORDINATED_RESTART_EXIT_CODE) {
            pending = Some(load_prepared_full_transaction(layout, &app_bundle)?);
            stop_on_abnormal_current = false;
            continue;
        }

        if stop_on_abnormal_current {
            let mut evidence = generic_failure(
                state.current.clone(),
                "previous-failed",
                "previous runtime also exited abnormally; supervisor stopped to avoid ping-pong",
            );
            apply_exit_status(&mut evidence, Some(&status));
            evidence.log_path = Some(layout.root.join("launcher.log"));
            let evidence = persist_failure_evidence(layout, &evidence)?;
            record_failure(&mut state, evidence);
            save_state(layout, &state)?;
            bail!("previous runtime failed; refusing rollback ping-pong");
        }

        let current = state
            .current
            .clone()
            .context("runtime exited abnormally without current build state")?;
        match record_crash(&mut state, &current.artifact_content_hash, Utc::now()) {
            CrashDecision::RestartCurrent => save_state(layout, &state)?,
            CrashDecision::RollBack => {
                if state.previous.is_none() || !layout.previous()?.exists() {
                    append_launcher_log(
                        layout,
                        "crash threshold reached without a previous runtime",
                    )?;
                    let mut evidence = generic_failure(
                        Some(current),
                        "post-ready-crash",
                        "rollback threshold reached but previous runtime is unavailable",
                    );
                    apply_exit_status(&mut evidence, Some(&status));
                    evidence.ready_timeout_ms = Some(READY_TIMEOUT_SECS * 1000);
                    evidence.log_path = Some(layout.root.join("launcher.log"));
                    evidence.transaction_path = Some(layout.configured_transaction());
                    let evidence = persist_failure_evidence(layout, &evidence)?;
                    record_failure(&mut state, evidence);
                    save_state(layout, &state)?;
                    bail!("current runtime repeatedly crashed and previous is unavailable");
                }
                let mut evidence = generic_failure(
                    Some(current.clone()),
                    "post-ready-crash",
                    "crash threshold reached; rolling back to previous runtime",
                );
                apply_exit_status(&mut evidence, Some(&status));
                evidence.ready_timeout_ms = Some(READY_TIMEOUT_SECS * 1000);
                evidence.log_path = Some(layout.root.join("launcher.log"));
                evidence.transaction_path = Some(layout.configured_transaction());
                evidence.recovered_build_id =
                    state.previous.as_ref().map(|build| build.build_id.clone());
                let fallback = state
                    .previous
                    .clone()
                    .context("previous build disappeared before rollback")?;
                let mut rollback = begin_post_ready_rollback(layout, current, fallback, evidence)?;
                append_launcher_log(layout, "post-ready crash threshold reached")?;
                resume_post_ready_rollback(layout, &mut rollback, &SystemCommandRunner)?;
                stop_on_abnormal_current = true;
            }
        }
    }
}

fn abort_pre_activation_candidate(
    layout: &Layout,
    transaction: &Transaction,
    summary: &str,
) -> Result<()> {
    let mut state = load_state(layout)?;
    state
        .blocked_build_hashes
        .insert(transaction.manifest_hash.clone());
    state
        .blocked_build_ids
        .insert(transaction.request.build_id.clone());
    state
        .blocked_artifact_hashes
        .insert(transaction.artifact_content_hash.clone());
    let mut evidence = request_failure(
        &transaction.request,
        Some(transaction.manifest_hash.clone()),
        "activation-validation",
        summary,
    );
    evidence.failed_build_hash = Some(transaction.artifact_content_hash.clone());
    evidence.log_path = Some(layout.root.join("launcher.log"));
    evidence.transaction_path = Some(layout.configured_transaction());
    let evidence = persist_failure_evidence(layout, &evidence)?;
    record_failure(&mut state, evidence);
    save_state(layout, &state)?;
    append_launcher_log(layout, summary)?;
    remove_staging_child(layout, &transaction.request.transaction_id)?;
    if layout.transaction()?.exists() {
        fs::remove_file(layout.transaction()?)?;
        sync_directory(&layout.active_root()?)?;
    }
    gc_staging(layout, None, 4)?;
    Ok(())
}

fn reload_transaction_for_rollback(layout: &Layout, expected: &Transaction) -> Result<Transaction> {
    let authoritative: Transaction = read_json(&layout.transaction()?)?;
    if authoritative.request.transaction_id != expected.request.transaction_id
        || authoritative.request.build_id != expected.request.build_id
        || authoritative.request.app_bundle_path != expected.request.app_bundle_path
        || !matches!(
            authoritative.phase,
            TransactionPhase::Activating
                | TransactionPhase::CandidateInstalled
                | TransactionPhase::CandidateStarted
                | TransactionPhase::RollingBack
                | TransactionPhase::RollbackComplete
        )
    {
        bail!("candidate rollback transaction changed while child was supervised");
    }
    Ok(authoritative)
}

fn load_prepared_full_transaction(layout: &Layout, app_bundle: &Path) -> Result<Transaction> {
    if !layout.transaction()?.exists() {
        bail!("exit 75 did not have a prepared full activation transaction");
    }
    let transaction: Transaction = read_json(&layout.transaction()?)?;
    if transaction.request.mode != ActivationMode::Full
        || transaction.phase != TransactionPhase::Prepared
        || transaction.request.app_bundle_path != app_bundle
    {
        bail!("exit 75 did not have a matching prepared full activation transaction");
    }
    Ok(transaction)
}

fn load_valid_prepared_transaction_on_startup(
    layout: &Layout,
    app_bundle: &Path,
) -> Result<Option<Transaction>> {
    if !layout.transaction()?.exists() {
        return Ok(None);
    }
    let mut transaction: Transaction = read_json(&layout.transaction()?)?;
    if transaction.phase != TransactionPhase::Prepared {
        return Ok(None);
    }
    let validation = (|| -> Result<()> {
        validate_request(&transaction.request)?;
        if transaction.request.mode != ActivationMode::Full {
            bail!("startup recovery only resumes prepared full activations");
        }
        if transaction.request.app_bundle_path != app_bundle {
            bail!("prepared activation belongs to a different app bundle");
        }
        let stage = resolve_staging_child(layout, &transaction.request.transaction_id, true)?;
        let manifest: runtime_launcher::PreparedManifest = read_json(&stage.join("manifest.json"))?;
        if manifest.build_id != transaction.request.build_id
            || manifest.source_commit != transaction.request.source_commit
            || manifest_hash(&manifest)? != transaction.manifest_hash
            || runtime_launcher::artifact_content_hash(&manifest)
                != transaction.artifact_content_hash
        {
            bail!("prepared stage identity does not match its durable transaction");
        }
        for artifact in &manifest.artifacts {
            let staged = stage.join("resources").join(&artifact.relative_path);
            if hash_file(&staged)? != artifact.sha256.to_ascii_lowercase() {
                bail!(
                    "prepared stage artifact hash mismatch: {}",
                    staged.display()
                );
            }
        }
        Ok(())
    })()
    .and_then(|()| {
        if transaction.launcher_expected_hash.is_none() {
            let launcher = transaction
                .request
                .app_bundle_path
                .join("Contents/MacOS/MorpheusLauncher");
            transaction.launcher_expected_hash = Some(hash_file(&launcher).with_context(|| {
                format!(
                    "failed to capture launcher hash for legacy prepared transaction: {}",
                    launcher.display()
                )
            })?);
            write_json_atomically(&layout.transaction()?, &transaction)?;
        }
        Ok(())
    });
    if let Err(error) = validation {
        let summary = format!("invalid prepared activation was durably aborted: {error:#}");
        let mut state = load_state(layout)?;
        let mut evidence = request_failure(
            &transaction.request,
            Some(transaction.manifest_hash.clone()),
            "prepared-startup-validation",
            &summary,
        );
        evidence.failed_build_hash = Some(transaction.artifact_content_hash.clone());
        evidence.log_path = Some(layout.root.join("launcher.log"));
        evidence.transaction_path = Some(layout.configured_transaction());
        let evidence = persist_failure_evidence(layout, &evidence)?;
        record_failure(&mut state, evidence);
        save_state(layout, &state)?;
        append_launcher_log(layout, &summary)?;
        fs::remove_file(layout.transaction()?)?;
        sync_directory(&layout.active_root()?)?;
        if let Err(cleanup_error) =
            remove_staging_child(layout, &transaction.request.transaction_id)
        {
            append_launcher_log(
                layout,
                &format!(
                    "invalid prepared stage cleanup deferred for transaction {}: {cleanup_error}",
                    transaction.request.transaction_id
                ),
            )?;
        }
        return Ok(None);
    }
    Ok(Some(transaction))
}

fn reconcile_launcher_state(
    layout: &Layout,
    app_bundle: &Path,
) -> Result<runtime_launcher::RuntimeState> {
    reconcile_launcher_state_with(layout, app_bundle, &SystemCommandRunner)
}

fn reconcile_launcher_state_with(
    layout: &Layout,
    app_bundle: &Path,
    runner: &dyn runtime_launcher::CommandRunner,
) -> Result<runtime_launcher::RuntimeState> {
    repair_failure_queue(layout)?;
    if !layout.transaction()?.exists() {
        gc_staging(layout, None, 4)?;
    }
    migrate_transaction_app_bundle_path(layout, &app_bundle)?;
    let recovered = match recover_interrupted_transaction_with(layout, runner) {
        Ok(recovered) => recovered,
        Err(error) => {
            if layout.transaction()?.exists() {
                let mut transaction: Transaction = read_json(&layout.transaction()?)?;
                if let Some(durable_evidence) = transaction
                    .post_ready_rollback
                    .as_ref()
                    .map(|rollback| rollback.failure_evidence.clone())
                {
                    let updated_evidence =
                        persist_post_ready_recovery_failure(layout, &durable_evidence, &error)?;
                    if let Some(rollback) = &mut transaction.post_ready_rollback {
                        rollback.failure_evidence = updated_evidence;
                    }
                    write_json_atomically(&layout.transaction()?, &transaction)?;
                } else {
                    persist_interrupted_activation_failure(layout, &mut transaction, &error)?;
                }
            }
            return Err(error);
        }
    };
    let mut state = load_state(layout)?;
    if let Some(transaction) = recovered.as_ref()
        && matches!(
            transaction.phase,
            TransactionPhase::Activating
                | TransactionPhase::CandidateInstalled
                | TransactionPhase::CandidateStarted
                | TransactionPhase::RollingBack
                | TransactionPhase::RollbackComplete
        )
        && transaction.post_ready_rollback.is_none()
    {
        if !layout.previous()?.exists() {
            state.previous = None;
        }
        state
            .blocked_build_hashes
            .insert(transaction.manifest_hash.clone());
        state
            .blocked_build_ids
            .insert(transaction.request.build_id.clone());
        state
            .blocked_artifact_hashes
            .insert(transaction.artifact_content_hash.clone());
        append_launcher_log(layout, "recovered interrupted activation")?;
        let mut evidence = transaction
            .rollback_failure_evidence
            .clone()
            .context("recovered interrupted activation has no durable rollback evidence")?;
        evidence.failed_build_hash = Some(transaction.artifact_content_hash.clone());
        evidence.ready_timeout_ms = Some(READY_TIMEOUT_SECS * 1000);
        evidence.log_path = Some(layout.root.join("launcher.log"));
        evidence.transaction_path = Some(layout.configured_transaction());
        evidence.recovered_build_id = state.current.as_ref().map(|build| build.build_id.clone());
        let evidence = persist_failure_evidence(layout, &evidence)?;
        if let Some(existing) = state
            .failures
            .iter_mut()
            .find(|existing| existing.recovery_identity == evidence.recovery_identity)
        {
            *existing = evidence;
        } else {
            record_failure(&mut state, evidence);
        }
        save_state(layout, &state)?;
        if transaction.phase == TransactionPhase::RollbackComplete && layout.transaction()?.exists()
        {
            fs::remove_file(layout.transaction()?)?;
            sync_directory(&layout.active_root()?)?;
        }
    }
    migrate_app_bundle_path(layout, &mut state, &app_bundle)?;
    if layout.transaction()?.exists() {
        let transaction = read_json::<Transaction>(&layout.transaction()?)?;
        if transaction.phase == TransactionPhase::Ready {
            commit_ready_state(layout, &transaction)?;
            fs::remove_file(layout.transaction()?)?;
            state = load_state(layout)?;
        }
    }
    if let Some(baseline) = bootstrap_current_slot(layout, &app_bundle)? {
        state.current = Some(baseline);
        save_state(layout, &state)?;
    }
    Ok(state)
}

enum CandidateOutcome {
    ReadyExit(ExitStatus),
    FailedBeforeReady {
        summary: String,
        status: Option<ExitStatus>,
    },
}

fn supervise_candidate(
    authority: &StateRootAuthority,
    layout: &Layout,
    transaction: &mut Transaction,
) -> Result<CandidateOutcome> {
    let instance_id = Uuid::new_v4().to_string();
    {
        let _lock = StateRootLock::acquire(authority)?;
        if layout.ready()?.exists() {
            fs::remove_file(layout.ready()?)?;
        }
        transaction.instance_id = Some(instance_id.clone());
        transaction.phase = TransactionPhase::CandidateStarted;
        write_json_atomically(&layout.transaction()?, transaction)?;
    }
    let mut child = spawn_app(
        authority,
        &transaction.request.app_bundle_path,
        Some((transaction, &instance_id)),
        None,
    )?;
    let deadline = Instant::now() + Duration::from_secs(READY_TIMEOUT_SECS);
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(CandidateOutcome::FailedBeforeReady {
                    summary: "candidate exited cleanly before publishing ready identity".into(),
                    status: Some(status),
                });
            }
            return Ok(CandidateOutcome::FailedBeforeReady {
                summary: format!("candidate exited before ready with {status}"),
                status: Some(status),
            });
        }
        if layout.ready()?.exists() {
            match read_json::<ReadyIdentity>(&layout.ready()?) {
                Ok(ready) if ready_matches(&ready, transaction, &instance_id) => {
                    let _lock = StateRootLock::acquire(authority)?;
                    let authoritative = reload_transaction_for_rollback(layout, transaction)?;
                    if authoritative.instance_id.as_deref() != Some(instance_id.as_str()) {
                        bail!("candidate ready identity no longer owns the active transaction");
                    }
                    transaction.phase = TransactionPhase::Ready;
                    write_json_atomically(&layout.transaction()?, transaction)?;
                    commit_ready_state(layout, transaction)?;
                    fs::remove_file(layout.transaction()?)?;
                    drop(_lock);
                    return Ok(CandidateOutcome::ReadyExit(child.wait()?));
                }
                Ok(_) => {}
                Err(_) => {}
            }
        }
        if Instant::now() >= deadline {
            terminate_child(&mut child);
            return Ok(CandidateOutcome::FailedBeforeReady {
                summary: format!(
                    "candidate did not publish matching ready identity within {READY_TIMEOUT_SECS}s"
                ),
                status: None,
            });
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn rollback_candidate(
    layout: &Layout,
    transaction: &Transaction,
    summary: &str,
    exit_status: Option<&ExitStatus>,
) -> Result<()> {
    let mut persisted = transaction.clone();
    match persisted.phase {
        TransactionPhase::Activating
        | TransactionPhase::CandidateInstalled
        | TransactionPhase::CandidateStarted
        | TransactionPhase::RollingBack => {
            if persisted.rollback_failure_evidence.is_none() {
                let failure_phase = if persisted.phase == TransactionPhase::Activating {
                    "activation"
                } else {
                    "pre-ready"
                };
                let mut evidence = request_failure(
                    &persisted.request,
                    Some(persisted.manifest_hash.clone()),
                    failure_phase,
                    summary,
                );
                evidence.failed_build_hash = Some(persisted.artifact_content_hash.clone());
                apply_exit_status(&mut evidence, exit_status);
                evidence.ready_timeout_ms = Some(READY_TIMEOUT_SECS * 1000);
                evidence.log_path = Some(layout.root.join("launcher.log"));
                evidence.transaction_path = Some(layout.configured_transaction());
                persisted.rollback_failure_evidence = Some(evidence);
            }
            persisted.phase = TransactionPhase::RollingBack;
            write_json_atomically(&layout.transaction()?, &persisted)?;
        }
        TransactionPhase::RollbackComplete => {}
        TransactionPhase::Prepared | TransactionPhase::Ready => {
            bail!(
                "transaction {} cannot be rolled back from phase {:?}",
                persisted.request.transaction_id,
                persisted.phase
            );
        }
    }
    let durable_summary = persisted
        .rollback_failure_evidence
        .as_ref()
        .map(|evidence| evidence.summary.as_str())
        .unwrap_or(summary);
    append_launcher_log(layout, durable_summary)?;
    let rollback_error = if persisted.phase == TransactionPhase::RollbackComplete {
        commit_activation_files(layout, &mut persisted).err()
    } else {
        rollback_activation_with(layout, &mut persisted, &SystemCommandRunner).err()
    };
    let mut state = load_state(layout)?;
    if !layout.previous()?.exists() {
        state.previous = None;
    }
    state
        .blocked_build_hashes
        .insert(transaction.manifest_hash.clone());
    state
        .blocked_build_ids
        .insert(transaction.request.build_id.clone());
    state
        .blocked_artifact_hashes
        .insert(transaction.artifact_content_hash.clone());
    let recovered_build_id = state.current.as_ref().map(|build| build.build_id.clone());
    let mut evidence = persisted
        .rollback_failure_evidence
        .clone()
        .unwrap_or_else(|| {
            request_failure(
                &persisted.request,
                Some(persisted.manifest_hash.clone()),
                "interrupted-recovery",
                summary,
            )
        });
    evidence.failed_build_hash = Some(persisted.artifact_content_hash.clone());
    evidence.ready_timeout_ms = Some(READY_TIMEOUT_SECS * 1000);
    evidence.log_path = Some(layout.root.join("launcher.log"));
    evidence.transaction_path = Some(layout.configured_transaction());
    evidence.recovered_build_id = recovered_build_id;
    if let Some(error) = &rollback_error {
        evidence.summary = format!(
            "{}; rollback verification failed: {error:#}",
            evidence.summary
        );
    }
    let evidence = persist_failure_evidence(layout, &evidence)?;
    if let Some(existing) = state
        .failures
        .iter_mut()
        .find(|existing| existing.recovery_identity == evidence.recovery_identity)
    {
        *existing = evidence;
    } else {
        record_failure(&mut state, evidence);
    }
    state.crash_history.clear();
    save_state(layout, &state)?;
    if let Some(error) = rollback_error {
        return Err(error);
    }
    fs::remove_file(layout.transaction()?)?;
    sync_directory(&layout.active_root()?)?;
    Ok(())
}

fn commit_ready_state(layout: &Layout, transaction: &Transaction) -> Result<()> {
    let mut state = load_state(layout)?;
    if state
        .current
        .as_ref()
        .map(|build| build.manifest_hash.as_str())
        != Some(transaction.manifest_hash.as_str())
    {
        state.previous = state.current.take();
        state.current = Some(BuildRecord {
            build_id: transaction.request.build_id.clone(),
            transaction_id: Some(transaction.request.transaction_id.clone()),
            request_id: Some(transaction.request.request_id.clone()),
            requested_by_thread_id: transaction.request.requested_by_thread_id.clone(),
            mode: transaction.request.mode,
            source_commit: transaction.request.source_commit.clone(),
            manifest_hash: transaction.manifest_hash.clone(),
            artifact_content_hash: transaction.artifact_content_hash.clone(),
            app_bundle_path: transaction.request.app_bundle_path.clone(),
            activated_at: Utc::now(),
        });
    }
    state.crash_history.clear();
    save_state(layout, &state)?;
    let mut transaction = transaction.clone();
    commit_activation_files(layout, &mut transaction)
}

fn persist_interrupted_activation_failure(
    layout: &Layout,
    transaction: &mut Transaction,
    error: &anyhow::Error,
) -> Result<()> {
    append_launcher_log(layout, &format!("interrupted-recovery-verify: {error:#}"))?;
    let mut state = load_state(layout)?;
    let recovered_build_id = state.current.as_ref().map(|build| build.build_id.clone());
    let mut evidence = transaction
        .rollback_failure_evidence
        .clone()
        .context("interrupted recovery failed without durable rollback evidence")?;
    evidence.failure_phase = "interrupted-recovery-verify".into();
    evidence.summary = format!("interrupted activation recovery remains pending: {error:#}");
    evidence.failed_build_hash = Some(transaction.artifact_content_hash.clone());
    evidence.ready_timeout_ms = Some(READY_TIMEOUT_SECS * 1000);
    evidence.log_path = Some(layout.root.join("launcher.log"));
    evidence.transaction_path = Some(layout.configured_transaction());
    evidence.recovered_build_id = recovered_build_id;
    let evidence = persist_failure_evidence(layout, &evidence)?;
    merge_failure_into_state(&mut state, &evidence)?;
    transaction.rollback_failure_evidence = Some(evidence);
    write_json_atomically(&layout.transaction()?, transaction)?;
    save_state(layout, &state)
}

fn persist_post_ready_recovery_failure(
    layout: &Layout,
    durable_evidence: &FailureEvidence,
    error: &anyhow::Error,
) -> Result<FailureEvidence> {
    append_launcher_log(
        layout,
        &format!("post-ready rollback recovery remains pending: {error:#}"),
    )?;
    let mut state = load_state(layout)?;
    let mut evidence = durable_evidence.clone();
    evidence.failure_phase = "post-ready-rollback-recovery".into();
    evidence.summary = format!("post-ready rollback recovery remains pending: {error:#}");
    evidence.transaction_path = Some(layout.configured_transaction());
    evidence.log_path = Some(layout.root.join("launcher.log"));
    evidence.acknowledged = false;
    // A stale rollback transaction may retain this identity after it has
    // already been consumed. Persistence returns that terminal record instead
    // of reviving delivery, and state merging preserves the returned
    // acknowledged/hash facts monotonically.
    let evidence = persist_failure_evidence(layout, &evidence)?;
    merge_failure_into_state(&mut state, &evidence)?;
    save_state(layout, &state)?;
    Ok(evidence)
}

fn generic_failure(build: Option<BuildRecord>, phase: &str, summary: &str) -> FailureEvidence {
    FailureEvidence {
        schema_version: SCHEMA_VERSION,
        recovery_identity: Some(Uuid::new_v4().to_string()),
        launcher_owner_nonce: None,
        occurred_at: Utc::now(),
        transaction_id: build
            .as_ref()
            .and_then(|value| value.transaction_id.clone()),
        request_id: build.as_ref().and_then(|value| value.request_id.clone()),
        requested_by_thread_id: build
            .as_ref()
            .and_then(|value| value.requested_by_thread_id.clone()),
        mode: build.as_ref().map(|value| value.mode),
        build_id: build.as_ref().map(|value| value.build_id.clone()),
        source_commit: build.as_ref().map(|value| value.source_commit.clone()),
        manifest_hash: build.as_ref().map(|value| value.manifest_hash.clone()),
        failed_build_hash: build
            .as_ref()
            .map(|value| value.artifact_content_hash.clone()),
        failure_phase: phase.into(),
        summary: summary.into(),
        reason: None,
        app_bundle_path: build.map(|value| value.app_bundle_path),
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

fn unacknowledged_failure_evidence_path(layout: &Layout) -> Result<Option<PathBuf>> {
    repair_failure_queue(layout)?;
    unacknowledged_failure_evidence_path_without_repair(layout)
}

fn unacknowledged_failure_evidence_path_without_repair(layout: &Layout) -> Result<Option<PathBuf>> {
    if let Some(claim) = list_failure_claims(layout)?.into_iter().next() {
        return Ok(Some(layout.root.join(format!(
            ".failure-evidence.claimed-{}.json",
            claim.recovery_identity
        ))));
    }
    let mut candidates = Vec::new();
    if layout.failure_evidence()?.exists() {
        let evidence: FailureEvidence = read_json(&layout.failure_evidence()?)?;
        require_failure_artifact_owner(layout, &evidence)?;
        if !evidence.acknowledged {
            candidates.push((
                evidence.occurred_at,
                evidence.recovery_identity.clone(),
                PathBuf::from("failure-evidence.json"),
            ));
        }
    }
    for evidence in list_pending_failure_evidence(layout)? {
        if !evidence.acknowledged {
            let identity = evidence
                .recovery_identity
                .clone()
                .context("pending failure evidence has no recovery identity")?;
            candidates.push((
                evidence.occurred_at,
                Some(identity.clone()),
                PathBuf::from(format!(".failure-evidence.pending-{identity}.json")),
            ));
        }
    }
    candidates.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    Ok(candidates
        .into_iter()
        .next()
        .map(|(_, _, name)| layout.root.join(name)))
}

fn apply_exit_status(evidence: &mut FailureEvidence, status: Option<&ExitStatus>) {
    evidence.exit_code = status.and_then(ExitStatus::code);
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        evidence.signal = status
            .and_then(ExitStatusExt::signal)
            .map(|signal| signal.to_string());
    }
}

fn append_launcher_log(layout: &Layout, message: &str) -> Result<()> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(layout.active_root()?.join("launcher.log"))?;
    writeln!(file, "{} {}", Utc::now().to_rfc3339(), message)?;
    file.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

fn infer_default_app_bundle() -> Result<PathBuf> {
    let launcher = std::env::current_exe().context("failed to resolve launcher executable")?;
    if launcher.file_name().and_then(|name| name.to_str()) != Some("MorpheusLauncher") {
        bail!("launcher executable is not Contents/MacOS/MorpheusLauncher");
    }
    let macos = launcher
        .parent()
        .context("launcher executable has no parent")?;
    if macos.file_name().and_then(|name| name.to_str()) != Some("MacOS") {
        bail!("launcher executable is not under Contents/MacOS");
    }
    let contents = macos.parent().context("MacOS has no parent")?;
    if contents.file_name().and_then(|name| name.to_str()) != Some("Contents") {
        bail!("launcher executable is not under an app Contents directory");
    }
    let app_bundle = contents
        .parent()
        .context("Contents has no app bundle parent")?;
    if app_bundle
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("app")
    {
        bail!("launcher parent is not a .app bundle");
    }
    Ok(app_bundle.to_path_buf())
}

fn migrate_app_bundle_path(
    layout: &Layout,
    state: &mut runtime_launcher::RuntimeState,
    app_bundle: &Path,
) -> Result<()> {
    validate_state_slot("current", state.current.as_ref(), &layout.current()?)?;
    validate_state_slot("previous", state.previous.as_ref(), &layout.previous()?)?;
    let canonical = app_bundle
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", app_bundle.display()))?;
    let mut changed = false;
    for build in [&mut state.current, &mut state.previous]
        .into_iter()
        .flatten()
    {
        if build.app_bundle_path != canonical {
            build.app_bundle_path = canonical.clone();
            changed = true;
        }
    }
    if changed {
        save_state(layout, state)?;
    }
    Ok(())
}

fn validate_state_slot(name: &str, build: Option<&BuildRecord>, slot: &Path) -> Result<()> {
    match (build, slot.exists()) {
        (Some(build), true) => {
            let manifest: runtime_launcher::PreparedManifest =
                read_json(&slot.join("manifest.json")).with_context(|| {
                    format!("persisted {name} build has no readable slot manifest")
                })?;
            if manifest.build_id != build.build_id {
                bail!(
                    "persisted {name} build {} does not match slot build {}",
                    build.build_id,
                    manifest.build_id
                );
            }
        }
        (Some(_), false) => bail!("persisted {name} build has no slot"),
        (None, true) => bail!("{name} slot exists without persisted build"),
        (None, false) => {}
    }
    Ok(())
}

fn normalize_system_var_alias(path: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Ok(relative) = path.strip_prefix("/private/var") {
        return Path::new("/var").join(relative);
    }
    path.to_path_buf()
}

fn migrate_transaction_app_bundle_path(layout: &Layout, app_bundle: &Path) -> Result<()> {
    if !layout.transaction()?.exists() {
        return Ok(());
    }
    let canonical = app_bundle
        .canonicalize()
        .with_context(|| format!("failed to resolve {}", app_bundle.display()))?;
    let mut transaction: Transaction = read_json(&layout.transaction()?)?;
    if transaction.phase == TransactionPhase::Prepared {
        if validate_request(&transaction.request).is_err() {
            return Ok(());
        }
        if transaction.request.app_bundle_path != canonical {
            transaction.request.app_bundle_path = canonical;
            write_json_atomically(&layout.transaction()?, &transaction)?;
        }
        return Ok(());
    }
    validate_request(&transaction.request)?;
    if normalize_system_var_alias(&transaction.request.app_bundle_path)
        != normalize_system_var_alias(&canonical)
    {
        bail!(
            "active transaction {} belongs to {}, not trusted current bundle {}; refusing to use persisted recovery paths as relocation authority",
            transaction.request.transaction_id,
            transaction.request.app_bundle_path.display(),
            canonical.display()
        );
    }
    bind_transaction_paths(layout, &mut transaction)?;
    Ok(())
}

fn launch_current(
    authority: &StateRootAuthority,
    app_bundle: &Path,
    ready: Option<(&Transaction, &str)>,
    failure_evidence: Option<&Path>,
) -> Result<ExitStatus> {
    Ok(spawn_app(authority, app_bundle, ready, failure_evidence)?.wait()?)
}

fn configure_host_state_root(command: &mut Command, authority: &StateRootAuthority) {
    // The configured path is absolute and remains stable across exec. Never
    // expose the launcher-internal active root path as Host configuration.
    command.env(RUNTIME_LAUNCHER_HOME_ENV, authority.layout().root);
}

#[cfg(target_os = "macos")]
fn spawn_app(
    authority: &StateRootAuthority,
    app_bundle: &Path,
    ready: Option<(&Transaction, &str)>,
    failure_evidence: Option<&Path>,
) -> Result<Child> {
    let executable = app_bundle.join("Contents/MacOS/Root Worker Runtime");
    if !executable.is_file() {
        bail!("runtime executable is missing: {}", executable.display());
    }
    let mut command = Command::new(executable);
    configure_host_state_root(&mut command, authority);
    command.env(
        "MORPHEUS_LAUNCHER_PATH",
        std::env::current_exe().context("failed to resolve launcher executable")?,
    );
    let inherited_root = if ready.is_some() || failure_evidence.is_some() {
        Some(authority.duplicate_inheritable_root()?)
    } else {
        None
    };
    if let Some((transaction, instance_id)) = ready {
        let ready_path = inherited_root
            .as_ref()
            .context("ready publication requires an inherited state root")?
            .1
            .join("ready.json");
        command
            .env("MORPHEUS_LAUNCH_READY_PATH", ready_path)
            .env(
                "MORPHEUS_LAUNCH_TRANSACTION_ID",
                &transaction.request.transaction_id,
            )
            .env("MORPHEUS_LAUNCH_BUILD_ID", &transaction.request.build_id)
            .env("MORPHEUS_LAUNCH_INSTANCE_ID", instance_id);
    }
    if let Some(failure_evidence) = failure_evidence {
        let name = failure_evidence
            .file_name()
            .and_then(|name| name.to_str())
            .context("failure evidence path has no valid file name")?;
        if name != "failure-evidence.json" {
            let identity = name
                .strip_prefix(".failure-evidence.pending-")
                .or_else(|| name.strip_prefix(".failure-evidence.claimed-"))
                .and_then(|name| name.strip_suffix(".json"))
                .context("failure evidence path is not a trusted queue artifact")?;
            let parsed = Uuid::parse_str(identity)
                .context("failure evidence queue path has invalid identity")?;
            if parsed.to_string() != identity {
                bail!("failure evidence queue path identity is not canonical");
            }
        }
        let path = inherited_root
            .as_ref()
            .context("failure evidence requires an inherited state root")?
            .1
            .join(name);
        command.env("MORPHEUS_LAUNCH_FAILURE_EVIDENCE_PATH", path);
    }
    let child = command
        .spawn()
        .with_context(|| format!("failed to launch {}", app_bundle.display()))?;
    drop(inherited_root);
    Ok(child)
}

#[cfg(not(target_os = "macos"))]
fn spawn_app(
    _authority: &StateRootAuthority,
    _app_bundle: &Path,
    _ready: Option<(&Transaction, &str)>,
    _failure_evidence: Option<&Path>,
) -> Result<Child> {
    bail!("runtime launcher process adapter is unsupported on this platform")
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn print_json<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}

#[cfg(target_os = "macos")]
fn ensure_supported_platform() -> Result<()> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn ensure_supported_platform() -> Result<()> {
    bail!("runtime launcher is unsupported on this platform")
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_launcher::ManifestChanges;
    use runtime_launcher::PreparedArtifact;
    use runtime_launcher::PreparedManifest;
    use runtime_launcher::ResourceRestorePhase;
    use runtime_launcher::ResourceSwapPhase;
    use sha2::Digest;
    use sha2::Sha256;
    use tempfile::TempDir;

    fn artifact(path: &str, contents: &[u8]) -> PreparedArtifact {
        PreparedArtifact {
            relative_path: path.into(),
            sha256: format!("{:x}", Sha256::digest(contents)),
            kind: "file".into(),
        }
    }

    fn directory_file_snapshot(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut snapshot = BTreeMap::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("snapshot filename is not UTF-8"))?;
            let metadata = fs::symlink_metadata(entry.path())?;
            let bytes = if metadata.file_type().is_file() {
                fs::read(entry.path())?
            } else if metadata.file_type().is_symlink() {
                fs::read_link(entry.path())?
                    .as_os_str()
                    .as_encoded_bytes()
                    .to_vec()
            } else {
                b"<directory>".to_vec()
            };
            snapshot.insert(name, bytes);
        }
        Ok(snapshot)
    }

    fn acknowledge_failure(
        layout: &Layout,
        transaction_id: Option<&str>,
        request_id: Option<&str>,
        recovery_identity: &str,
    ) -> Result<()> {
        let located = locate_failure_evidence(layout, recovery_identity)?;
        super::acknowledge_failure_versioned(
            layout,
            transaction_id,
            request_id,
            recovery_identity,
            &located.version_token,
        )
    }

    #[cfg(unix)]
    fn assert_terminal_duplicate_repair(use_pending: bool) -> Result<()> {
        // Model the durable boundaries of terminal cleanup: before the
        // consumed update, after the consumed update but before active removal,
        // and after active removal but before/after its directory sync.
        for checkpoint in 0..3 {
            let temp = TempDir::new()?;
            let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
            let layout = authority.layout();
            let app = temp.path().join("App.app");
            let mut active = generic_failure(Some(build("terminal", &app)), "active", "active");
            active.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
            active.failed_build_hash = Some("known-terminal-hash".into());
            active.acknowledged = false;
            let identity = active
                .recovery_identity
                .as_deref()
                .context("terminal recovery identity")?
                .to_string();
            let mut consumed = active.clone();
            consumed.failure_phase = "consumed".into();
            consumed.summary = "terminal".into();
            consumed.failed_build_hash =
                (checkpoint > 0).then(|| "known-terminal-hash".to_string());
            consumed.acknowledged = checkpoint > 0;
            write_json_atomically(
                &consumed_failure_evidence_path(&layout, &identity)?,
                &consumed,
            )?;
            let active_path = if use_pending {
                pending_failure_evidence_path(&layout, &identity)?
            } else {
                layout.failure_evidence()?
            };
            if checkpoint < 2 {
                write_json_atomically(&active_path, &active)?;
            }
            save_state(
                &layout,
                &runtime_launcher::RuntimeState {
                    failures: vec![active],
                    ..runtime_launcher::RuntimeState::default()
                },
            )?;

            repair_failure_queue(&layout)?;
            repair_failure_queue(&layout)?;

            let completed: FailureEvidence =
                read_json(&consumed_failure_evidence_path(&layout, &identity)?)?;
            assert!(completed.acknowledged);
            assert_eq!(
                completed.failed_build_hash.as_deref(),
                Some("known-terminal-hash")
            );
            assert!(!layout.failure_evidence()?.exists());
            assert!(!pending_failure_evidence_path(&layout, &identity)?.exists());
            assert!(unacknowledged_failure_evidence_path_without_repair(&layout)?.is_none());
            let state = load_state(&layout)?;
            assert_eq!(state.failures.len(), 1);
            assert!(state.failures[0].acknowledged);
            assert_eq!(
                state.failures[0].failed_build_hash.as_deref(),
                Some("known-terminal-hash")
            );
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn state_root_lock_serializes_writers() -> Result<()> {
        use std::sync::mpsc;

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let authority = StateRootAuthority::open(&layout)?;
        let first = StateRootLock::acquire(&authority)?;
        let second_authority = authority.clone();
        let (sender, receiver) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            let lock = StateRootLock::acquire(&second_authority);
            let acquired = lock.is_ok();
            sender.send(acquired).ok();
        });
        assert!(receiver.recv_timeout(Duration::from_millis(100)).is_err());
        drop(first);
        assert!(receiver.recv_timeout(Duration::from_secs(2))?);
        waiter
            .join()
            .map_err(|_| anyhow::anyhow!("lock waiter panicked"))?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn host_spawn_configuration_overrides_and_propagates_custom_state_root() -> Result<()> {
        let temp = TempDir::new()?;
        let configured_root = temp.path().join("custom-launcher-root");
        let authority = StateRootAuthority::open(&Layout::new(configured_root.clone()))?;
        let mut command = Command::new("/bin/true");
        command.env(RUNTIME_LAUNCHER_HOME_ENV, "/wrong/caller/root");

        configure_host_state_root(&mut command, &authority);

        let propagated = command
            .get_envs()
            .find(|(name, _)| *name == std::ffi::OsStr::new(RUNTIME_LAUNCHER_HOME_ENV))
            .and_then(|(_, value)| value);
        assert_eq!(propagated, Some(configured_root.as_os_str()));
        assert!(configured_root.is_absolute());
        assert!(!configured_root.to_string_lossy().starts_with("/dev/fd/"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn state_root_lock_rejects_replaced_root_before_touching_it() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let authority = StateRootAuthority::open(&layout)?;
        let lock = StateRootLock::acquire(&authority)?;
        let anchored_layout = lock.layout();
        let original_root = temp.path().join("original-launcher");
        fs::rename(&layout.root, &original_root)?;
        fs::create_dir_all(&layout.root)?;
        fs::write(layout.root.join("sentinel"), b"replacement")?;

        save_state(&anchored_layout, &runtime_launcher::RuntimeState::default())?;
        assert!(original_root.join("state.json").exists());
        assert!(!layout.root.join("state.json").exists());
        assert!(StateRootAuthority::open(&layout).is_err());
        assert_eq!(fs::read(layout.root.join("sentinel"))?, b"replacement");
        assert!(!layout.root.join("launcher.lock").exists());
        assert!(original_root.join("launcher.lock").exists());
        drop(lock);
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn authority_layout_paths_follow_renamed_root_without_touching_replacement() -> Result<()> {
        let temp = TempDir::new()?;
        let configured_root = temp.path().join("launcher");
        let authority = StateRootAuthority::open(&Layout::new(configured_root.clone()))?;
        let layout = authority.layout();
        let renamed_root = temp.path().join("renamed-launcher");
        fs::rename(&configured_root, &renamed_root)?;
        fs::create_dir_all(&configured_root)?;
        for (name, contents) in [
            ("sentinel", b"replacement-sentinel".as_slice()),
            ("transaction.json", b"replacement-transaction".as_slice()),
            (
                "failure-evidence.json",
                b"replacement-failure-evidence".as_slice(),
            ),
            ("ready.json", b"replacement-ready".as_slice()),
            ("current", b"replacement-current".as_slice()),
        ] {
            fs::write(configured_root.join(name), contents)?;
        }
        let replacement_before = directory_file_snapshot(&configured_root)?;

        write_json_atomically(
            &layout.transaction()?,
            &serde_json::json!({"transaction": "original"}),
        )?;
        let app = temp.path().join("App.app");
        let evidence = generic_failure(Some(build("failed", &app)), "phase", "original");
        let persisted = persist_failure_evidence(&layout, &evidence)?;
        let queued_evidence = generic_failure(Some(build("queued", &app)), "phase", "queued");
        let queued_identity = queued_evidence
            .recovery_identity
            .as_deref()
            .context("queued recovery identity")?;
        let queued = persist_failure_evidence(&layout, &queued_evidence)?;
        write_json_atomically(&layout.ready()?, &serde_json::json!({"ready": "original"}))?;
        fs::create_dir_all(layout.current()?)?;
        fs::write(layout.current()?.join("value"), b"original-slot")?;

        assert_eq!(
            read_json::<serde_json::Value>(&renamed_root.join("transaction.json"))?,
            serde_json::json!({"transaction": "original"})
        );
        assert_eq!(
            read_json::<FailureEvidence>(&renamed_root.join("failure-evidence.json"))?,
            persisted
        );
        assert_eq!(
            read_json::<FailureEvidence>(&pending_failure_evidence_path(
                &layout,
                queued_identity,
            )?)?,
            queued
        );
        assert_eq!(
            read_json::<serde_json::Value>(&renamed_root.join("ready.json"))?,
            serde_json::json!({"ready": "original"})
        );
        assert_eq!(
            fs::read(renamed_root.join("current/value"))?,
            b"original-slot"
        );
        assert_eq!(
            directory_file_snapshot(&configured_root)?,
            replacement_before
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn child_state_root_is_inheritable_but_durable_paths_remain_configured() -> Result<()> {
        use std::os::fd::AsRawFd;

        let temp = TempDir::new()?;
        let configured = temp.path().join("launcher");
        let authority = StateRootAuthority::open(&Layout::new(configured.clone()))?;
        let layout = authority.layout();
        let (inherited_root, inherited_path) = authority.duplicate_inheritable_root()?;
        let descriptor_flags = unsafe { libc::fcntl(inherited_root.as_raw_fd(), libc::F_GETFD) };

        assert!(descriptor_flags >= 0);
        assert_eq!(descriptor_flags & libc::FD_CLOEXEC, 0);
        assert_eq!(layout.active_root()?, inherited_path);
        assert_eq!(
            layout.configured_transaction(),
            configured.join("transaction.json")
        );
        assert_eq!(
            layout.configured_failure_evidence(),
            configured.join("failure-evidence.json")
        );
        assert_eq!(layout.configured_ready(), configured.join("ready.json"));
        assert_eq!(
            inherited_path.join("ready.json").file_name(),
            Some(std::ffi::OsStr::new("ready.json"))
        );
        let ready_path = inherited_path.join("ready.json");
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg("printf inherited-ready > \"$READY_PATH\"")
            .env("READY_PATH", &ready_path)
            .spawn()?;
        drop(inherited_root);
        assert!(child.wait()?.success());
        assert_eq!(fs::read(layout.ready()?)?, b"inherited-ready");

        let durable_paths = serde_json::to_string(&serde_json::json!({
            "readyPath": layout.configured_ready(),
            "evidencePath": layout.configured_failure_evidence(),
            "transactionPath": layout.configured_transaction(),
        }))?;
        assert!(!durable_paths.contains("/dev/fd/"));
        assert!(!durable_paths.contains("/proc/self/fd/"));
        Ok(())
    }

    fn build(build_id: &str, app: &Path) -> BuildRecord {
        BuildRecord {
            transaction_id: Some(format!("tx-{build_id}")),
            request_id: Some(format!("request-{build_id}")),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Hot,
            build_id: build_id.into(),
            source_commit: format!("commit-{build_id}"),
            manifest_hash: format!("manifest-{build_id}"),
            artifact_content_hash: format!("content-{build_id}"),
            app_bundle_path: app.to_path_buf(),
            activated_at: Utc::now(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn new_failure_evidence_has_unique_persistent_recovery_identity() -> Result<()> {
        let temp = TempDir::new()?;
        let app = temp.path().join("App.app");
        let first = generic_failure(Some(build("failed", &app)), "phase", "first");
        let second = generic_failure(Some(build("failed", &app)), "phase", "second");
        let first_identity =
            validated_recovery_identity(&first)?.context("first recovery identity")?;
        let second_identity =
            validated_recovery_identity(&second)?.context("second recovery identity")?;

        assert_ne!(first_identity, second_identity);
        let persisted: FailureEvidence = serde_json::from_slice(&serde_json::to_vec(&first)?)?;
        assert_eq!(persisted.recovery_identity, first.recovery_identity);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn legacy_failure_recovery_identity_migrates_once_and_survives_reopen() -> Result<()> {
        let temp = TempDir::new()?;
        let configured = Layout::new(temp.path().join("launcher"));
        let authority = StateRootAuthority::open(&configured)?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let mut evidence = generic_failure(Some(build("failed", &app)), "phase", "legacy");
        evidence.recovery_identity = None;
        let mut durable_json = serde_json::to_value(&evidence)?;
        durable_json
            .as_object_mut()
            .context("failure evidence JSON object")?
            .remove("recoveryIdentity");
        write_json_atomically(&layout.failure_evidence()?, &durable_json)?;
        let mut state_json = serde_json::to_value(runtime_launcher::RuntimeState {
            failures: vec![evidence],
            ..runtime_launcher::RuntimeState::default()
        })?;
        state_json["failures"][0]
            .as_object_mut()
            .context("state failure JSON object")?
            .remove("recoveryIdentity");
        write_json_atomically(&layout.state()?, &state_json)?;

        repair_failure_recovery_identities(&layout)?;
        let migrated: FailureEvidence = read_json(&layout.failure_evidence()?)?;
        let identity = validated_recovery_identity(&migrated)?
            .context("migrated recovery identity")?
            .to_string();
        let state = load_state(&layout)?;
        assert_eq!(
            state.failures[0].recovery_identity.as_deref(),
            Some(identity.as_str())
        );
        drop(layout);
        drop(authority);

        let reopened = StateRootAuthority::open(&configured)?;
        let reopened_layout = reopened.layout();
        repair_failure_recovery_identities(&reopened_layout)?;
        let after_reopen: FailureEvidence = read_json(&reopened_layout.failure_evidence()?)?;
        assert_eq!(
            after_reopen.recovery_identity.as_deref(),
            Some(identity.as_str())
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn repair_queue_migrates_legacy_current_pending_and_consumed_artifacts() -> Result<()> {
        for artifact_kind in ["current", "pending", "consumed"] {
            let temp = TempDir::new()?;
            let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
            let layout = authority.layout();
            let app = temp.path().join("App.app");
            let mut evidence =
                generic_failure(Some(build(artifact_kind, &app)), "legacy", artifact_kind);
            evidence.recovery_identity = None;
            evidence.launcher_owner_nonce = None;
            evidence.acknowledged = artifact_kind == "consumed";
            let scoped_identity = Uuid::new_v4().to_string();
            let path = match artifact_kind {
                "current" => layout.failure_evidence()?,
                "pending" => pending_failure_evidence_path(&layout, &scoped_identity)?,
                "consumed" => consumed_failure_evidence_path(&layout, &scoped_identity)?,
                _ => unreachable!(),
            };
            write_json_atomically(&path, &evidence)?;
            save_state(
                &layout,
                &runtime_launcher::RuntimeState {
                    failures: vec![evidence],
                    ..runtime_launcher::RuntimeState::default()
                },
            )?;

            repair_failure_queue(&layout)?;
            repair_failure_queue(&layout)?;

            let state = load_state(&layout)?;
            let owner_nonce = layout.root_owner_nonce()?;
            assert_eq!(state.failures.len(), 1);
            let identity = validated_recovery_identity(&state.failures[0])?
                .context("migrated recovery identity")?;
            assert_eq!(
                state.failures[0].launcher_owner_nonce.as_deref(),
                Some(owner_nonce.as_str())
            );
            if artifact_kind != "current" {
                assert_eq!(identity, scoped_identity);
            }
            let located = locate_failure_evidence(&layout, identity)?;
            assert_eq!(
                located.evidence.launcher_owner_nonce.as_deref(),
                Some(owner_nonce.as_str())
            );
            if artifact_kind == "consumed" {
                assert_eq!(located.artifact_state, "consumed");
                assert!(located.evidence.acknowledged);
            } else {
                assert_eq!(located.artifact_state, "current");
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn repair_queue_rejects_conflicting_legacy_identity_or_owner_without_writes() -> Result<()> {
        for conflict in ["identity", "owner"] {
            let temp = TempDir::new()?;
            let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
            let layout = authority.layout();
            let app = temp.path().join("App.app");
            let mut current =
                generic_failure(Some(build("legacy-conflict", &app)), "legacy", "current");
            current.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
            write_json_atomically(&layout.failure_evidence()?, &current)?;
            if conflict == "identity" {
                let mut duplicate = current.clone();
                let duplicate_identity = Uuid::new_v4().to_string();
                duplicate.recovery_identity = Some(duplicate_identity.clone());
                write_json_atomically(
                    &pending_failure_evidence_path(&layout, &duplicate_identity)?,
                    &duplicate,
                )?;
            } else {
                current.launcher_owner_nonce = Some("different-owner".into());
                write_json_atomically(&layout.failure_evidence()?, &current)?;
            }
            let before = directory_file_snapshot(&layout.active_root()?)?;

            assert!(repair_failure_queue(&layout).is_err());

            assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn evidence_only_failure_can_be_acknowledged_idempotently() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let evidence = generic_failure(Some(build("failed", &app)), "phase", "evidence-only");
        let recovery_identity = evidence
            .recovery_identity
            .clone()
            .context("recovery identity")?;
        persist_failure_evidence(&layout, &evidence)?;

        acknowledge_failure(
            &layout,
            Some("tx-failed"),
            Some("request-failed"),
            &recovery_identity,
        )?;
        acknowledge_failure(
            &layout,
            Some("tx-failed"),
            Some("request-failed"),
            &recovery_identity,
        )?;

        let state = load_state(&layout)?;
        assert_eq!(state.failures.len(), 1);
        assert!(state.failures[0].acknowledged);
        assert_eq!(
            state.failures[0].recovery_identity.as_deref(),
            Some(recovery_identity.as_str())
        );
        assert!(!layout.failure_evidence()?.exists());
        let consumed: FailureEvidence = read_json(&consumed_failure_evidence_path(
            &layout,
            &recovery_identity,
        )?)?;
        assert!(consumed.acknowledged);
        assert_eq!(
            consumed.recovery_identity.as_deref(),
            Some(recovery_identity.as_str())
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn acknowledge_rejects_recovery_identity_request_mismatch() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let durable = generic_failure(Some(build("failed", &app)), "phase", "durable");
        let mut conflicting = durable.clone();
        conflicting.request_id = Some("different-request".into());
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![conflicting],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        persist_failure_evidence(&layout, &durable)?;
        let recovery_identity = durable
            .recovery_identity
            .as_deref()
            .context("recovery identity")?
            .to_string();

        assert!(
            acknowledge_failure(
                &layout,
                Some("tx-failed"),
                Some("request-failed"),
                &recovery_identity,
            )
            .is_err()
        );
        assert!(!load_state(&layout)?.failures[0].acknowledged);
        assert!(!read_json::<FailureEvidence>(&layout.failure_evidence()?)?.acknowledged);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn stale_ack_cannot_consume_new_identity_for_same_transaction() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let stale = generic_failure(Some(build("failed", &app)), "phase", "snapshot-a");
        let current = generic_failure(Some(build("failed", &app)), "phase", "snapshot-b");
        let stale_identity = stale
            .recovery_identity
            .as_deref()
            .context("stale recovery identity")?
            .to_string();
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![current.clone()],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        persist_failure_evidence(&layout, &current)?;
        let state_before = fs::read(layout.state()?)?;
        let evidence_before = fs::read(layout.failure_evidence()?)?;

        assert!(
            acknowledge_failure(
                &layout,
                Some("tx-failed"),
                Some("request-failed"),
                &stale_identity,
            )
            .is_err()
        );
        assert_eq!(fs::read(layout.state()?)?, state_before);
        assert_eq!(fs::read(layout.failure_evidence()?)?, evidence_before);
        assert!(!consumed_failure_evidence_path(&layout, &stale_identity)?.exists());
        assert!(
            !consumed_failure_evidence_path(
                &layout,
                current
                    .recovery_identity
                    .as_deref()
                    .context("current recovery identity")?,
            )?
            .exists()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn consumed_ack_is_idempotent_while_new_current_remains_untouched() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let first = generic_failure(Some(build("failed", &app)), "phase", "first");
        let first_identity = first
            .recovery_identity
            .as_deref()
            .context("first recovery identity")?
            .to_string();
        persist_failure_evidence(&layout, &first)?;
        acknowledge_failure(
            &layout,
            Some("tx-failed"),
            Some("request-failed"),
            &first_identity,
        )?;

        let second = generic_failure(Some(build("failed", &app)), "phase", "second");
        persist_failure_evidence(&layout, &second)?;
        let current_before = fs::read(layout.failure_evidence()?)?;
        acknowledge_failure(
            &layout,
            Some("tx-failed"),
            Some("request-failed"),
            &first_identity,
        )?;

        assert_eq!(fs::read(layout.failure_evidence()?)?, current_before);
        assert!(consumed_failure_evidence_path(&layout, &first_identity)?.exists());
        let second_consumed = second
            .recovery_identity
            .as_deref()
            .map(|identity| consumed_failure_evidence_path(&layout, identity))
            .transpose()?
            .is_some_and(|path| path.exists());
        assert!(!second_consumed);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn state_only_failure_materializes_and_consumes_artifact() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let mut evidence = generic_failure(None, "legacy-state-only", "state-only");
        let recovery_identity = evidence
            .recovery_identity
            .as_deref()
            .context("recovery identity")?
            .to_string();
        evidence.acknowledged = false;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![evidence],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;

        repair_failure_queue(&layout)?;
        let materialized: FailureEvidence = read_json(&layout.failure_evidence()?)?;
        assert_eq!(
            materialized.recovery_identity.as_deref(),
            Some(recovery_identity.as_str())
        );
        acknowledge_failure(&layout, None, None, &recovery_identity)?;

        assert!(load_state(&layout)?.failures[0].acknowledged);
        assert!(!layout.failure_evidence()?.exists());
        assert!(consumed_failure_evidence_path(&layout, &recovery_identity)?.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn repair_recovers_journal_first_pending_checkpoints() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let mut first = generic_failure(Some(build("first", &app)), "checkpoint", "first");
        first.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
        let first_identity = first
            .recovery_identity
            .as_deref()
            .context("first recovery identity")?;
        write_json_atomically(
            &pending_failure_evidence_path(&layout, first_identity)?,
            &first,
        )?;

        repair_failure_queue(&layout)?;
        assert_eq!(
            read_json::<FailureEvidence>(&layout.failure_evidence()?)?.recovery_identity,
            first.recovery_identity
        );

        let mut second = generic_failure(Some(build("second", &app)), "checkpoint", "second");
        second.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
        let second_identity = second
            .recovery_identity
            .as_deref()
            .context("second recovery identity")?;
        write_json_atomically(
            &pending_failure_evidence_path(&layout, second_identity)?,
            &second,
        )?;

        repair_failure_queue(&layout)?;
        assert_eq!(
            read_json::<FailureEvidence>(&layout.failure_evidence()?)?.recovery_identity,
            first.recovery_identity
        );
        assert!(pending_failure_evidence_path(&layout, second_identity)?.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn same_identity_pending_journal_commits_across_all_checkpoints() -> Result<()> {
        for checkpoint in 0..4 {
            let temp = TempDir::new()?;
            let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
            let layout = authority.layout();
            let app = temp.path().join("App.app");
            let mut original =
                generic_failure(Some(build("journal", &app)), "old-phase", "old summary");
            original.failed_build_hash = None;
            let current = persist_failure_evidence(&layout, &original)?;
            let identity = current
                .recovery_identity
                .as_deref()
                .context("recovery identity")?;
            let pending_path = pending_failure_evidence_path(&layout, identity)?;
            let mut newer = current.clone();
            newer.failure_phase = "new-phase".into();
            newer.summary = "new full journal payload".into();
            newer.reason = Some("new reason".into());
            newer.log_path = Some(layout.root.join("new.log"));
            newer.failed_build_hash = Some("newly-known-build-hash".into());
            write_json_atomically(&pending_path, &newer)?;
            if checkpoint >= 1 {
                write_json_atomically(&layout.failure_evidence()?, &newer)?;
            }
            if checkpoint >= 2 {
                fs::remove_file(&pending_path)?;
            }
            if checkpoint >= 3 {
                sync_directory(&layout.active_root()?)?;
            }

            repair_failure_queue(&layout)?;
            assert_eq!(
                read_json::<FailureEvidence>(&layout.failure_evidence()?)?,
                newer
            );
            assert_eq!(load_state(&layout)?.failures, vec![newer.clone()]);

            acknowledge_failure(
                &layout,
                Some("tx-journal"),
                Some("request-journal"),
                identity,
            )?;
            let mut consumed_expected = newer;
            consumed_expected.acknowledged = true;
            assert_eq!(
                read_json::<FailureEvidence>(&consumed_failure_evidence_path(&layout, identity,)?)?,
                consumed_expected
            );
            assert_eq!(load_state(&layout)?.failures, vec![consumed_expected]);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn conflicting_same_identity_journal_fails_without_side_effects() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let current = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("conflict", &app)), "old-phase", "old summary"),
        )?;
        let identity = current
            .recovery_identity
            .as_deref()
            .context("recovery identity")?;
        let pending_path = pending_failure_evidence_path(&layout, identity)?;
        let mut conflicting = current.clone();
        conflicting.source_commit = Some("conflicting-source".into());
        write_json_atomically(&pending_path, &conflicting)?;
        let current_before = fs::read(layout.failure_evidence()?)?;
        let pending_before = fs::read(&pending_path)?;

        assert!(repair_failure_queue(&layout).is_err());

        assert_eq!(fs::read(layout.failure_evidence()?)?, current_before);
        assert_eq!(fs::read(&pending_path)?, pending_before);
        assert!(!layout.state()?.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn persist_preflight_rejects_conflicting_current_before_creating_pending() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let current = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("current", &app)), "phase", "current"),
        )?;
        let identity = current
            .recovery_identity
            .as_deref()
            .context("recovery identity")?;
        let mut conflicting = current.clone();
        conflicting.source_commit = Some("conflicting-source".into());
        let current_before = fs::read(layout.failure_evidence()?)?;

        assert!(persist_failure_evidence(&layout, &conflicting).is_err());

        assert_eq!(fs::read(layout.failure_evidence()?)?, current_before);
        assert!(!pending_failure_evidence_path(&layout, identity)?.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn persist_preflight_rejects_conflicting_existing_pending_without_writes() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let current = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("pending", &app)), "phase", "current"),
        )?;
        let identity = current
            .recovery_identity
            .as_deref()
            .context("recovery identity")?;
        let pending_path = pending_failure_evidence_path(&layout, identity)?;
        let mut conflicting = current.clone();
        conflicting.source_commit = Some("conflicting-source".into());
        write_json_atomically(&pending_path, &conflicting)?;
        let current_before = fs::read(layout.failure_evidence()?)?;
        let pending_before = fs::read(&pending_path)?;

        assert!(persist_failure_evidence(&layout, &current).is_err());

        assert_eq!(fs::read(layout.failure_evidence()?)?, current_before);
        assert_eq!(fs::read(&pending_path)?, pending_before);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn persist_absorbs_consumed_identity_without_reviving_delivery() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let mut consumed = generic_failure(Some(build("consumed", &app)), "done", "consumed");
        consumed.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
        consumed.failed_build_hash = Some("known-failed-build-hash".into());
        consumed.acknowledged = true;
        let identity = consumed
            .recovery_identity
            .as_deref()
            .context("consumed recovery identity")?;
        let consumed_path = consumed_failure_evidence_path(&layout, identity)?;
        write_json_atomically(&consumed_path, &consumed)?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![consumed.clone()],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        let consumed_before = fs::read(&consumed_path)?;

        let mut stale_producer = consumed.clone();
        stale_producer.failed_build_hash = None;
        stale_producer.acknowledged = false;
        let persisted = persist_post_ready_recovery_failure(
            &layout,
            &stale_producer,
            &anyhow::anyhow!("late recovery retry"),
        )?;

        assert!(persisted.acknowledged);
        assert_eq!(
            persisted.failed_build_hash.as_deref(),
            Some("known-failed-build-hash")
        );
        assert_eq!(persisted, consumed);
        assert_eq!(fs::read(&consumed_path)?, consumed_before);
        assert!(!layout.failure_evidence()?.exists());
        assert!(!pending_failure_evidence_path(&layout, identity)?.exists());
        assert!(unacknowledged_failure_evidence_path_without_repair(&layout)?.is_none());
        assert!(load_state(&layout)?.failures[0].acknowledged);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn persist_preflight_rejects_noncanonical_scoped_identity_without_writes() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let mut malformed = generic_failure(Some(build("malformed", &app)), "phase", "malformed");
        malformed.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
        let uppercase_identity = malformed
            .recovery_identity
            .as_deref()
            .context("malformed recovery identity")?
            .to_uppercase();
        malformed.recovery_identity = Some(uppercase_identity.clone());
        write_json_atomically(
            &pending_failure_evidence_path(&layout, &uppercase_identity)?,
            &malformed,
        )?;
        let before = directory_file_snapshot(&layout.active_root()?)?;
        let incoming = generic_failure(Some(build("incoming", &app)), "phase", "incoming");

        assert!(persist_failure_evidence(&layout, &incoming).is_err());

        assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn persist_preflight_rejects_scoped_filename_payload_mismatch_without_writes() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let mut mismatched = generic_failure(Some(build("mismatch", &app)), "phase", "mismatch");
        mismatched.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
        let filename_identity = Uuid::new_v4().to_string();
        write_json_atomically(
            &consumed_failure_evidence_path(&layout, &filename_identity)?,
            &mismatched,
        )?;
        let before = directory_file_snapshot(&layout.active_root()?)?;
        let incoming = generic_failure(Some(build("incoming", &app)), "phase", "incoming");

        assert!(persist_failure_evidence(&layout, &incoming).is_err());

        assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn persist_rejects_dangling_current_symlink_before_journal_write() -> Result<()> {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let dangling_target = temp.path().join("missing-current.json");
        symlink(&dangling_target, layout.failure_evidence()?)?;
        let symlink_target_before = fs::read_link(layout.failure_evidence()?)?;
        let before = directory_file_snapshot(&layout.active_root()?)?;
        let app = temp.path().join("App.app");
        let evidence = generic_failure(Some(build("dangling", &app)), "phase", "dangling");
        let identity = evidence
            .recovery_identity
            .as_deref()
            .context("recovery identity")?;

        assert!(persist_failure_evidence(&layout, &evidence).is_err());

        assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        assert_eq!(
            fs::read_link(layout.failure_evidence()?)?,
            symlink_target_before
        );
        assert!(!dangling_target.exists());
        assert!(!pending_failure_evidence_path(&layout, identity)?.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn repair_consumed_identity_removes_unacknowledged_current() -> Result<()> {
        assert_terminal_duplicate_repair(false)
    }

    #[cfg(unix)]
    #[test]
    fn repair_consumed_identity_removes_unacknowledged_pending() -> Result<()> {
        assert_terminal_duplicate_repair(true)
    }

    #[cfg(unix)]
    #[test]
    fn repair_terminal_hash_conflict_fails_before_mutation() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let mut current = generic_failure(Some(build("conflict", &app)), "active", "active");
        current.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
        current.failed_build_hash = Some("active-hash".into());
        let identity = current
            .recovery_identity
            .as_deref()
            .context("recovery identity")?;
        let mut consumed = current.clone();
        consumed.failed_build_hash = Some("terminal-hash".into());
        consumed.acknowledged = true;
        write_json_atomically(&layout.failure_evidence()?, &current)?;
        write_json_atomically(
            &consumed_failure_evidence_path(&layout, identity)?,
            &consumed,
        )?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![current],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        let before = directory_file_snapshot(&layout.active_root()?)?;

        assert!(repair_failure_queue(&layout).is_err());

        assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn repair_terminal_consumed_inherits_known_state_hash_monotonically() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let mut active = generic_failure(Some(build("state-hash", &app)), "active", "active");
        active.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
        active.failed_build_hash = None;
        active.acknowledged = false;
        let identity = active
            .recovery_identity
            .as_deref()
            .context("recovery identity")?;
        let mut consumed = active.clone();
        consumed.failure_phase = "consumed".into();
        consumed.summary = "terminal".into();
        consumed.acknowledged = true;
        let mut state_evidence = active.clone();
        state_evidence.failed_build_hash = Some("state-known-hash".into());
        write_json_atomically(&layout.failure_evidence()?, &active)?;
        write_json_atomically(
            &consumed_failure_evidence_path(&layout, identity)?,
            &consumed,
        )?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![state_evidence],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;

        repair_failure_queue(&layout)?;
        repair_failure_queue(&layout)?;

        let completed: FailureEvidence =
            read_json(&consumed_failure_evidence_path(&layout, identity)?)?;
        assert!(completed.acknowledged);
        assert_eq!(
            completed.failed_build_hash.as_deref(),
            Some("state-known-hash")
        );
        assert!(!layout.failure_evidence()?.exists());
        assert!(unacknowledged_failure_evidence_path_without_repair(&layout)?.is_none());
        assert_eq!(load_state(&layout)?.failures, vec![completed]);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn repair_terminal_state_hash_conflict_fails_without_writes() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let mut consumed =
            generic_failure(Some(build("state-conflict", &app)), "consumed", "terminal");
        consumed.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
        consumed.failed_build_hash = Some("consumed-hash".into());
        consumed.acknowledged = true;
        let identity = consumed
            .recovery_identity
            .as_deref()
            .context("recovery identity")?;
        let mut state_evidence = consumed.clone();
        state_evidence.failed_build_hash = Some("state-hash".into());
        state_evidence.acknowledged = false;
        write_json_atomically(
            &consumed_failure_evidence_path(&layout, identity)?,
            &consumed,
        )?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![state_evidence],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        let before = directory_file_snapshot(&layout.active_root()?)?;

        assert!(repair_failure_queue(&layout).is_err());

        assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn locator_selects_expected_pending_identity_instead_of_current() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let current = generic_failure(Some(build("locator-a", &app)), "current", "A");
        let pending = generic_failure(Some(build("locator-b", &app)), "pending", "B");
        persist_failure_evidence(&layout, &current)?;
        let pending = persist_failure_evidence(&layout, &pending)?;
        let pending_identity = pending
            .recovery_identity
            .as_deref()
            .context("pending recovery identity")?;

        repair_failure_queue(&layout)?;
        let located = locate_failure_evidence(&layout, pending_identity)?;

        assert_eq!(located.artifact_state, "pending");
        assert_eq!(located.evidence, pending);
        assert_eq!(
            located.configured_evidence_path,
            layout
                .root
                .join(format!(".failure-evidence.pending-{pending_identity}.json"))
        );
        assert_eq!(
            located.active_evidence_path,
            pending_failure_evidence_path(&layout, pending_identity)?
        );
        assert_eq!(located.transaction_id.as_deref(), Some("tx-locator-b"));
        assert_eq!(located.request_id.as_deref(), Some("request-locator-b"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn locator_missing_identity_does_not_fall_back_to_current() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let current = generic_failure(Some(build("locator-current", &app)), "current", "A");
        persist_failure_evidence(&layout, &current)?;
        repair_failure_queue(&layout)?;
        let before = directory_file_snapshot(&layout.active_root()?)?;

        assert!(locate_failure_evidence(&layout, &Uuid::new_v4().to_string()).is_err());

        assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn claim_freezes_expected_pending_identity_and_is_idempotent() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let current = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("claim-a", &app)), "current", "A"),
        )?;
        let pending = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("claim-b", &app)), "pending", "B"),
        )?;
        let current_identity = current.recovery_identity.as_deref().context("current id")?;
        let pending_identity = pending.recovery_identity.as_deref().context("pending id")?;
        let located = locate_failure_evidence(&layout, pending_identity)?;
        let before_mismatch = directory_file_snapshot(&layout.active_root()?)?;
        let mismatch = claim_failure(
            &layout,
            pending.transaction_id.as_deref(),
            pending.request_id.as_deref(),
            pending_identity,
            "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        )
        .expect_err("active version mismatch must fail");
        assert!(
            mismatch
                .downcast_ref::<FailureEvidenceVersionChanged>()
                .is_some()
        );
        assert_eq!(
            directory_file_snapshot(&layout.active_root()?)?,
            before_mismatch
        );

        let claimed = claim_failure(
            &layout,
            pending.transaction_id.as_deref(),
            pending.request_id.as_deref(),
            pending_identity,
            &located.version_token,
        )?;
        let repeated = claim_failure(
            &layout,
            pending.transaction_id.as_deref(),
            pending.request_id.as_deref(),
            pending_identity,
            &located.version_token,
        )?;
        let stale_retry = claim_failure(
            &layout,
            pending.transaction_id.as_deref(),
            pending.request_id.as_deref(),
            pending_identity,
            "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        )?;

        assert_eq!(claimed.claim_state, "created");
        assert_eq!(repeated.claim_state, "existing");
        assert_eq!(claimed.claim_id, repeated.claim_id);
        assert_eq!(claimed.claim_id, stale_retry.claim_id);
        assert!(!stale_retry.requested_version_matched);
        assert_eq!(claimed.evidence, repeated.evidence);
        assert_eq!(
            read_json::<FailureEvidence>(&layout.failure_evidence()?)?
                .recovery_identity
                .as_deref(),
            Some(current_identity)
        );
        assert!(!pending_failure_evidence_path(&layout, pending_identity)?.exists());
        let located_claim = locate_failure_evidence(&layout, pending_identity)?;
        assert_eq!(located_claim.artifact_state, "claimed");
        assert_eq!(located_claim.evidence, claimed.evidence);
        assert_eq!(
            unacknowledged_failure_evidence_path_without_repair(&layout)?,
            Some(configured_claimed_failure_evidence_path(
                &layout,
                pending_identity,
            ))
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn claimed_identity_absorbs_later_producer_without_payload_drift() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let original = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("claimed", &app)), "old", "frozen"),
        )?;
        let identity = original.recovery_identity.clone().context("recovery id")?;
        let located = locate_failure_evidence(&layout, &identity)?;
        let claim = claim_failure(
            &layout,
            original.transaction_id.as_deref(),
            original.request_id.as_deref(),
            &identity,
            &located.version_token,
        )?;
        let claim_path = claimed_failure_evidence_path(&layout, &identity)?;
        let before = fs::read(&claim_path)?;
        let mut stale_producer = original;
        stale_producer.failure_phase = "newer".into();
        stale_producer.summary = "must not replace frozen payload".into();
        stale_producer.reason = Some("late producer".into());

        let absorbed = persist_failure_evidence(&layout, &stale_producer)?;

        assert_eq!(absorbed, claim.evidence);
        assert_eq!(fs::read(&claim_path)?, before);
        assert!(!layout.failure_evidence()?.exists());
        assert!(!pending_failure_evidence_path(&layout, &identity)?.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn claim_derived_consumed_absorbs_stale_producers_at_finalize_checkpoints() -> Result<()> {
        for retain_claim in [true, false] {
            let temp = TempDir::new()?;
            let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
            let layout = authority.layout();
            let app = temp.path().join("App.app");
            let original = persist_failure_evidence(
                &layout,
                &generic_failure(Some(build("frozen-consumed", &app)), "old", "frozen"),
            )?;
            let identity = original.recovery_identity.clone().context("recovery id")?;
            let located = locate_failure_evidence(&layout, &identity)?;
            let claim = claim_failure(
                &layout,
                original.transaction_id.as_deref(),
                original.request_id.as_deref(),
                &identity,
                &located.version_token,
            )?;
            let claim_path = claimed_failure_evidence_path(&layout, &identity)?;
            let consumed_path = consumed_failure_evidence_path(&layout, &identity)?;
            let mut consumed = claim.evidence.clone();
            consumed.acknowledged = true;
            write_json_atomically(&consumed_path, &consumed)?;
            if !retain_claim {
                fs::remove_file(&claim_path)?;
                sync_directory(&layout.active_root()?)?;
            }
            let consumed_before = fs::read(&consumed_path)?;
            let claim_before = retain_claim.then(|| fs::read(&claim_path)).transpose()?;
            let version_before = failure_evidence_version(&consumed)?;
            let mut stale = original;
            stale.failure_phase = "late".into();
            stale.summary = "must not update consumed".into();
            stale.reason = Some("stale producer".into());

            let absorbed = persist_failure_evidence(&layout, &stale)?;

            assert_eq!(absorbed, consumed);
            assert_eq!(fs::read(&consumed_path)?, consumed_before);
            assert_eq!(
                retain_claim.then(|| fs::read(&claim_path)).transpose()?,
                claim_before
            );
            assert_eq!(
                failure_evidence_version(&read_json::<FailureEvidence>(&consumed_path)?)?,
                version_before
            );
            assert!(!layout.failure_evidence()?.exists());
            assert!(!pending_failure_evidence_path(&layout, &identity)?.exists());
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn legacy_migration_plan_rejects_claim_completion_drift_without_any_write() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let target = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("claimed-drift", &app)), "phase", "target"),
        )?;
        let target_identity = target.recovery_identity.as_deref().context("target id")?;
        let located = locate_failure_evidence(&layout, target_identity)?;
        let claim = claim_failure(
            &layout,
            target.transaction_id.as_deref(),
            target.request_id.as_deref(),
            target_identity,
            &located.version_token,
        )?;
        let mut drifted_consumed = claim.evidence.clone();
        drifted_consumed.acknowledged = true;
        drifted_consumed.summary = "drifted after claim".into();
        write_json_atomically(
            &consumed_failure_evidence_path(&layout, target_identity)?,
            &drifted_consumed,
        )?;

        let mut legacy = generic_failure(Some(build("legacy-unrelated", &app)), "legacy", "legacy");
        legacy.recovery_identity = None;
        legacy.launcher_owner_nonce = None;
        write_json_atomically(&layout.failure_evidence()?, &legacy)?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![legacy],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        let before = directory_file_snapshot(&layout.active_root()?)?;

        assert!(repair_failure_queue(&layout).is_err());

        assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn finalize_claim_retries_all_durable_boundaries_without_payload_drift() -> Result<()> {
        for checkpoint in 0..3 {
            let temp = TempDir::new()?;
            let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
            let layout = authority.layout();
            let app = temp.path().join("App.app");
            let unrelated = persist_failure_evidence(
                &layout,
                &generic_failure(Some(build("remaining", &app)), "current", "A"),
            )?;
            let target = persist_failure_evidence(
                &layout,
                &generic_failure(Some(build("finalize", &app)), "pending", "B"),
            )?;
            let identity = target.recovery_identity.as_deref().context("target id")?;
            let located = locate_failure_evidence(&layout, identity)?;
            let claim = claim_failure(
                &layout,
                target.transaction_id.as_deref(),
                target.request_id.as_deref(),
                identity,
                &located.version_token,
            )?;
            let claim_path = claimed_failure_evidence_path(&layout, identity)?;
            let consumed_path = consumed_failure_evidence_path(&layout, identity)?;
            let mut expected_consumed = claim.evidence.clone();
            expected_consumed.acknowledged = true;
            if checkpoint >= 1 {
                write_json_atomically(&consumed_path, &expected_consumed)?;
            }
            if checkpoint >= 2 {
                fs::remove_file(&claim_path)?;
                sync_directory(&layout.active_root()?)?;
            }

            finalize_failure_claim(&layout, &claim.claim_id, identity)?;
            finalize_failure_claim(&layout, &claim.claim_id, identity)?;

            assert_eq!(
                read_json::<FailureEvidence>(&consumed_path)?,
                expected_consumed
            );
            assert!(!claim_path.exists());
            assert_eq!(
                read_json::<FailureEvidence>(&layout.failure_evidence()?)?.recovery_identity,
                unrelated.recovery_identity
            );
            assert!(unacknowledged_failure_evidence_path_without_repair(&layout)?.is_some());
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn repair_converges_claimed_duplicates_and_rejects_corruption_before_writes() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let evidence = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("repair-claim", &app)), "phase", "claim"),
        )?;
        let identity = evidence
            .recovery_identity
            .as_deref()
            .context("recovery id")?;
        let located = locate_failure_evidence(&layout, identity)?;
        let claim = claim_failure(
            &layout,
            evidence.transaction_id.as_deref(),
            evidence.request_id.as_deref(),
            identity,
            &located.version_token,
        )?;
        let claim_path = claimed_failure_evidence_path(&layout, identity)?;
        let mut active_duplicate = claim.evidence.clone();
        active_duplicate.claim_id = None;
        write_json_atomically(
            &pending_failure_evidence_path(&layout, identity)?,
            &active_duplicate,
        )?;

        repair_failure_queue(&layout)?;

        assert!(claim_path.exists());
        assert!(!pending_failure_evidence_path(&layout, identity)?.exists());
        let mut consumed = claim.evidence.clone();
        consumed.acknowledged = true;
        write_json_atomically(
            &consumed_failure_evidence_path(&layout, identity)?,
            &consumed,
        )?;
        repair_failure_queue(&layout)?;
        assert!(!claim_path.exists());

        let second = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("corrupt-claim", &app)), "phase", "bad"),
        )?;
        let second_identity = second.recovery_identity.as_deref().context("second id")?;
        let second_located = locate_failure_evidence(&layout, second_identity)?;
        let second_claim = claim_failure(
            &layout,
            second.transaction_id.as_deref(),
            second.request_id.as_deref(),
            second_identity,
            &second_located.version_token,
        )?;
        let second_path = claimed_failure_evidence_path(&layout, second_identity)?;
        let mut corrupt: FailureEvidenceClaim = read_json(&second_path)?;
        corrupt.claim_id = Uuid::new_v4().to_string();
        write_json_atomically(&second_path, &corrupt)?;
        let before = directory_file_snapshot(&layout.active_root()?)?;

        assert!(repair_failure_queue(&layout).is_err());

        assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        assert_ne!(corrupt.claim_id, second_claim.claim_id);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn claimed_schema_rejects_noncanonical_payloads_without_writes() -> Result<()> {
        for corruption in [
            "outer-unknown",
            "outer-version",
            "nested-unknown",
            "nested-version",
            "occurred-at-offset",
        ] {
            let temp = TempDir::new()?;
            let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
            let layout = authority.layout();
            let app = temp.path().join("App.app");
            let evidence = persist_failure_evidence(
                &layout,
                &generic_failure(
                    Some(build(corruption, &app)),
                    "phase",
                    "closed claim schema",
                ),
            )?;
            let identity = evidence
                .recovery_identity
                .as_deref()
                .context("recovery id")?;
            let located = locate_failure_evidence(&layout, identity)?;
            claim_failure(
                &layout,
                evidence.transaction_id.as_deref(),
                evidence.request_id.as_deref(),
                identity,
                &located.version_token,
            )?;
            let claim_path = claimed_failure_evidence_path(&layout, identity)?;
            let mut claim_json: serde_json::Value =
                serde_json::from_slice(&fs::read(&claim_path)?)?;
            let original_version_token = claim_json["versionToken"].clone();
            match corruption {
                "outer-unknown" => {
                    claim_json["unexpectedOuterField"] = serde_json::json!(true);
                }
                "outer-version" => {
                    claim_json["schemaVersion"] = serde_json::json!(2);
                }
                "nested-unknown" => {
                    claim_json["evidence"]["unexpectedNestedField"] = serde_json::json!(true);
                }
                "nested-version" => {
                    claim_json["evidence"]["schemaVersion"] = serde_json::json!(2);
                }
                "occurred-at-offset" => {
                    let occurred_at = claim_json["evidence"]["occurredAt"]
                        .as_str()
                        .context("generated occurredAt should be a string")?
                        .to_owned();
                    let without_utc_suffix = occurred_at
                        .strip_suffix('Z')
                        .context("generated occurredAt should use canonical Z")?;
                    claim_json["evidence"]["occurredAt"] =
                        serde_json::json!(format!("{without_utc_suffix}+00:00"));
                }
                _ => unreachable!(),
            }
            if corruption == "occurred-at-offset" {
                assert_eq!(claim_json["versionToken"], original_version_token);
            }
            write_json_atomically(&claim_path, &claim_json)?;
            let before = directory_file_snapshot(&layout.active_root()?)?;

            assert!(repair_failure_queue(&layout).is_err());

            assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn valid_v1_claim_remains_repairable_locatable_and_finalizable() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let evidence = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("valid-v1-claim", &app)), "phase", "valid claim"),
        )?;
        let identity = evidence
            .recovery_identity
            .as_deref()
            .context("recovery id")?;
        let located = locate_failure_evidence(&layout, identity)?;
        let claimed = claim_failure(
            &layout,
            evidence.transaction_id.as_deref(),
            evidence.request_id.as_deref(),
            identity,
            &located.version_token,
        )?;

        repair_failure_queue(&layout)?;
        let status_claim = list_failure_claims(&layout)?
            .into_iter()
            .next()
            .context("status should expose the valid claim")?;
        let located_claim = locate_failure_evidence(&layout, identity)?;
        let claim_json = serde_json::to_value(&status_claim)?;
        let outer_fields = claim_json
            .as_object()
            .context("serialized claim should be an object")?
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            outer_fields,
            [
                "claimId",
                "evidence",
                "launcherOwnerNonce",
                "recoveryIdentity",
                "schemaVersion",
                "sourceVersionToken",
                "versionToken",
            ]
            .into_iter()
            .collect()
        );
        let nested_fields = claim_json["evidence"]
            .as_object()
            .context("serialized claim evidence should be an object")?
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            nested_fields,
            [
                "acknowledged",
                "appBundlePath",
                "buildId",
                "claimId",
                "exitCode",
                "failedBuildHash",
                "failurePhase",
                "launcherOwnerNonce",
                "logPath",
                "manifestHash",
                "mode",
                "occurredAt",
                "readyTimeoutMs",
                "reason",
                "recoveredBuildId",
                "recoveryIdentity",
                "requestId",
                "requestedByThreadId",
                "schemaVersion",
                "signal",
                "sourceCommit",
                "summary",
                "transactionId",
                "transactionPath",
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(status_claim.claim_id, claimed.claim_id);
        assert_eq!(status_claim.evidence, claimed.evidence);
        assert_eq!(located_claim.artifact_state, "claimed");
        assert_eq!(located_claim.evidence, claimed.evidence);

        finalize_failure_claim(&layout, &claimed.claim_id, identity)?;

        let consumed: FailureEvidence =
            read_json(&consumed_failure_evidence_path(&layout, identity)?)?;
        assert!(consumed.acknowledged);
        assert_eq!(
            consumed.claim_id.as_deref(),
            Some(claimed.claim_id.as_str())
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn stale_version_ack_fails_without_committing_newer_journal() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let current = generic_failure(Some(build("versioned", &app)), "old", "old");
        let current = persist_failure_evidence(&layout, &current)?;
        repair_failure_queue(&layout)?;
        let identity = current
            .recovery_identity
            .as_deref()
            .context("recovery identity")?;
        let old_location = locate_failure_evidence(&layout, identity)?;
        let mut newer = current.clone();
        newer.failure_phase = "new".into();
        newer.summary = "complete newer payload".into();
        newer.reason = Some("new reason".into());
        newer.log_path = Some(layout.root.join("new.log"));
        newer.transaction_path = Some(layout.configured_transaction());
        write_json_atomically(&pending_failure_evidence_path(&layout, identity)?, &newer)?;
        let before = directory_file_snapshot(&layout.active_root()?)?;

        let error = acknowledge_failure_versioned(
            &layout,
            newer.transaction_id.as_deref(),
            newer.request_id.as_deref(),
            identity,
            &old_location.version_token,
        )
        .expect_err("stale version must fail");
        let version_error = error
            .downcast_ref::<FailureEvidenceVersionChanged>()
            .context("version mismatch should retain its typed error")?;
        assert_eq!(version_error.expected, old_location.version_token);
        assert_eq!(version_error.actual, failure_evidence_version(&newer)?);

        assert_eq!(directory_file_snapshot(&layout.active_root()?)?, before);
        repair_failure_queue(&layout)?;
        let new_location = locate_failure_evidence(&layout, identity)?;
        assert_ne!(new_location.version_token, old_location.version_token);
        assert_eq!(new_location.evidence, newer);
        acknowledge_failure_versioned(
            &layout,
            newer.transaction_id.as_deref(),
            newer.request_id.as_deref(),
            identity,
            &new_location.version_token,
        )?;
        let mut expected_consumed = newer;
        expected_consumed.acknowledged = true;
        assert_eq!(
            read_json::<FailureEvidence>(&consumed_failure_evidence_path(&layout, identity,)?)?,
            expected_consumed
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn stale_ack_does_not_commit_unrelated_current_journal() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let current = persist_failure_evidence(
            &layout,
            &generic_failure(Some(build("b", &app)), "old", "current B"),
        )?;
        let current_identity = current
            .recovery_identity
            .as_deref()
            .context("current recovery identity")?;
        let pending_path = pending_failure_evidence_path(&layout, current_identity)?;
        let mut pending = current.clone();
        pending.failure_phase = "new".into();
        pending.summary = "pending B".into();
        write_json_atomically(&pending_path, &pending)?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![current.clone()],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        let mut consumed = generic_failure(Some(build("consumed", &app)), "done", "consumed");
        consumed.launcher_owner_nonce = Some(layout.root_owner_nonce()?);
        consumed.acknowledged = true;
        let consumed_identity = consumed
            .recovery_identity
            .as_deref()
            .context("consumed recovery identity")?;
        let consumed_path = consumed_failure_evidence_path(&layout, consumed_identity)?;
        write_json_atomically(&consumed_path, &consumed)?;
        let stale = Uuid::new_v4().to_string();
        let current_before = fs::read(layout.failure_evidence()?)?;
        let pending_before = fs::read(&pending_path)?;
        let state_before = fs::read(layout.state()?)?;
        let consumed_before = fs::read(&consumed_path)?;

        assert!(
            acknowledge_failure(&layout, Some("tx-stale"), Some("request-stale"), &stale,).is_err()
        );

        assert_eq!(fs::read(layout.failure_evidence()?)?, current_before);
        assert_eq!(fs::read(&pending_path)?, pending_before);
        assert_eq!(fs::read(layout.state()?)?, state_before);
        assert_eq!(fs::read(&consumed_path)?, consumed_before);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unacknowledged_failures_queue_and_are_discovered_without_loss() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let first = generic_failure(Some(build("first", &app)), "phase", "first");
        let mut second = generic_failure(Some(build("second", &app)), "phase", "second");
        second.occurred_at = first.occurred_at + chrono::Duration::seconds(1);
        let first_identity = first
            .recovery_identity
            .as_deref()
            .context("first recovery identity")?
            .to_string();
        let second_identity = second
            .recovery_identity
            .as_deref()
            .context("second recovery identity")?
            .to_string();

        persist_failure_evidence(&layout, &first)?;
        persist_failure_evidence(&layout, &second)?;
        assert!(pending_failure_evidence_path(&layout, &second_identity)?.exists());
        assert_eq!(
            read_json::<FailureEvidence>(&layout.failure_evidence()?)?
                .recovery_identity
                .as_deref(),
            Some(first_identity.as_str())
        );
        repair_failure_queue(&layout)?;
        assert_eq!(load_state(&layout)?.failures.len(), 2);
        assert_eq!(
            unacknowledged_failure_evidence_path(&layout)?,
            Some(layout.configured_failure_evidence())
        );

        acknowledge_failure(
            &layout,
            Some("tx-first"),
            Some("request-first"),
            &first_identity,
        )?;
        assert_eq!(
            read_json::<FailureEvidence>(&layout.failure_evidence()?)?
                .recovery_identity
                .as_deref(),
            Some(second_identity.as_str())
        );
        assert_eq!(
            unacknowledged_failure_evidence_path(&layout)?,
            Some(layout.configured_failure_evidence())
        );
        assert!(unacknowledged_failure_evidence_path_without_repair(&layout)?.is_some());
        acknowledge_failure(
            &layout,
            Some("tx-second"),
            Some("request-second"),
            &second_identity,
        )?;

        assert!(!layout.failure_evidence()?.exists());
        assert!(unacknowledged_failure_evidence_path_without_repair(&layout)?.is_none());
        assert!(consumed_failure_evidence_path(&layout, &first_identity)?.exists());
        assert!(consumed_failure_evidence_path(&layout, &second_identity)?.exists());
        let state = load_state(&layout)?;
        assert_eq!(state.failures.len(), 2);
        assert!(state.failures.iter().all(|failure| failure.acknowledged));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn post_ready_recovery_preserves_unrelated_current_failure() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let durable = generic_failure(Some(build("rollback", &app)), "post-ready", "rollback");
        let unrelated = generic_failure(Some(build("unrelated", &app)), "other", "unrelated");
        let unrelated = persist_failure_evidence(&layout, &unrelated)?;
        let unrelated_identity = unrelated
            .recovery_identity
            .as_deref()
            .context("unrelated recovery identity")?
            .to_string();
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                failures: vec![unrelated.clone()],
                ..runtime_launcher::RuntimeState::default()
            },
        )?;

        let persisted = persist_post_ready_recovery_failure(
            &layout,
            &durable,
            &anyhow::anyhow!("rollback still pending"),
        )?;

        assert_eq!(persisted.recovery_identity, durable.recovery_identity);
        assert_eq!(
            read_json::<FailureEvidence>(&layout.failure_evidence()?)?.recovery_identity,
            unrelated.recovery_identity
        );
        let queued: FailureEvidence = read_json(&pending_failure_evidence_path(
            &layout,
            durable
                .recovery_identity
                .as_deref()
                .context("durable recovery identity")?,
        )?)?;
        require_same_failure_provenance(&queued, &persisted)?;
        let state = load_state(&layout)?;
        assert_eq!(state.failures.len(), 2);
        assert!(
            state
                .failures
                .iter()
                .any(|failure| { failure.recovery_identity == durable.recovery_identity })
        );
        assert!(state.failures.iter().any(|failure| {
            failure.recovery_identity.as_deref() == Some(unrelated_identity.as_str())
        }));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn consumed_retention_keeps_latest_thirty_two_identities() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        let base = Utc::now();
        let mut identities = Vec::new();
        for index in 0..40 {
            let mut evidence = generic_failure(
                Some(build(&format!("consumed-{index}"), &app)),
                "retention",
                "consumed",
            );
            evidence.occurred_at = base + chrono::Duration::seconds(index);
            let mut evidence = persist_failure_evidence(&layout, &evidence)?;
            let identity = evidence
                .recovery_identity
                .clone()
                .context("recovery identity")?;
            evidence.acknowledged = true;
            persist_failure_evidence(&layout, &evidence)?;
            fs::rename(
                layout.failure_evidence()?,
                consumed_failure_evidence_path(&layout, &identity)?,
            )?;
            identities.push(identity);
        }

        repair_failure_queue(&layout)?;

        for identity in identities.iter().take(8) {
            assert!(!consumed_failure_evidence_path(&layout, identity)?.exists());
        }
        for identity in identities.iter().skip(8) {
            assert!(consumed_failure_evidence_path(&layout, identity)?.exists());
        }
        assert_eq!(load_state(&layout)?.failures.len(), 32);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unacknowledged_queue_is_not_evicted_by_consumed_history() -> Result<()> {
        let temp = TempDir::new()?;
        let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
        let layout = authority.layout();
        let app = temp.path().join("App.app");
        for index in 0..32 {
            let mut evidence = generic_failure(
                Some(build(&format!("old-{index}"), &app)),
                "history",
                "consumed",
            );
            evidence = persist_failure_evidence(&layout, &evidence)?;
            let identity = evidence
                .recovery_identity
                .clone()
                .context("recovery identity")?;
            evidence.acknowledged = true;
            persist_failure_evidence(&layout, &evidence)?;
            fs::rename(
                layout.failure_evidence()?,
                consumed_failure_evidence_path(&layout, &identity)?,
            )?;
        }
        for index in 0..40 {
            let evidence = generic_failure(
                Some(build(&format!("pending-{index}"), &app)),
                "queue",
                "unacknowledged",
            );
            persist_failure_evidence(&layout, &evidence)?;
        }

        repair_failure_queue(&layout)?;

        let state = load_state(&layout)?;
        assert_eq!(state.failures.len(), 40);
        assert!(state.failures.iter().all(|evidence| !evidence.acknowledged));
        assert_eq!(list_pending_failure_evidence(&layout)?.len(), 39);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn failure_ack_retries_each_durable_breakpoint() -> Result<()> {
        for breakpoint in 0..3 {
            let temp = TempDir::new()?;
            let authority = StateRootAuthority::open(&Layout::new(temp.path().join("launcher")))?;
            let layout = authority.layout();
            let app = temp.path().join("App.app");
            let mut evidence = generic_failure(Some(build("failed", &app)), "phase", "breakpoint");
            let recovery_identity = evidence
                .recovery_identity
                .as_deref()
                .context("recovery identity")?
                .to_string();
            let mut state_evidence = evidence.clone();
            state_evidence.acknowledged = true;
            save_state(
                &layout,
                &runtime_launcher::RuntimeState {
                    failures: vec![state_evidence],
                    ..runtime_launcher::RuntimeState::default()
                },
            )?;
            match breakpoint {
                0 => {
                    evidence = persist_failure_evidence(&layout, &evidence)?;
                }
                1 => {
                    evidence = persist_failure_evidence(&layout, &evidence)?;
                    evidence.acknowledged = true;
                    evidence = persist_failure_evidence(&layout, &evidence)?;
                }
                2 => {
                    evidence = persist_failure_evidence(&layout, &evidence)?;
                    evidence.acknowledged = true;
                    write_json_atomically(
                        &consumed_failure_evidence_path(&layout, &recovery_identity)?,
                        &evidence,
                    )?;
                    fs::remove_file(layout.failure_evidence()?)?;
                }
                _ => unreachable!(),
            }

            acknowledge_failure(
                &layout,
                Some("tx-failed"),
                Some("request-failed"),
                &recovery_identity,
            )?;

            assert!(load_state(&layout)?.failures[0].acknowledged);
            assert!(!layout.failure_evidence()?.exists());
            let consumed: FailureEvidence = read_json(&consumed_failure_evidence_path(
                &layout,
                &recovery_identity,
            )?)?;
            assert!(consumed.acknowledged);
        }
        Ok(())
    }

    #[test]
    fn immutable_failure_provenance_mismatches_fail_closed() -> Result<()> {
        let temp = TempDir::new()?;
        let app = temp.path().join("App.app");
        let evidence = generic_failure(Some(build("failed", &app)), "phase", "baseline");
        let mut mismatches = Vec::new();

        let mut changed = evidence.clone();
        changed.recovery_identity = Some(Uuid::new_v4().to_string());
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.occurred_at = changed.occurred_at + chrono::Duration::seconds(1);
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.transaction_id = Some("other-transaction".into());
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.request_id = Some("other-request".into());
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.requested_by_thread_id = Some("other-thread".into());
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.mode = Some(ActivationMode::Full);
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.build_id = Some("other-build".into());
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.source_commit = Some("other-commit".into());
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.manifest_hash = Some("other-manifest".into());
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.app_bundle_path = Some(temp.path().join("Other.app"));
        mismatches.push(changed);
        let mut changed = evidence.clone();
        changed.failed_build_hash = Some("other-content".into());
        mismatches.push(changed);

        for mismatch in mismatches {
            assert!(require_same_failure_provenance(&evidence, &mismatch).is_err());
        }
        let mut compatible = evidence.clone();
        compatible.failed_build_hash = None;
        assert!(require_same_failure_provenance(&evidence, &compatible).is_ok());
        Ok(())
    }

    fn write_slot(slot: &Path, build_id: &str, contents: &[u8]) -> Result<()> {
        fs::create_dir_all(slot.join("resources"))?;
        fs::write(slot.join("resources/value"), contents)?;
        write_json_atomically(
            &slot.join("manifest.json"),
            &PreparedManifest {
                schema_version: SCHEMA_VERSION,
                build_id: build_id.into(),
                source_commit: format!("commit-{build_id}"),
                artifacts: vec![artifact("value", contents)],
                changes: ManifestChanges::default(),
            },
        )
    }

    #[test]
    fn authoritative_state_reload_attributes_crash_after_concurrent_hot_commit() -> Result<()> {
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
        let old = build("old", &app);
        let new = build("new", &app);
        write_slot(&layout.current()?, "old", b"old")?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                current: Some(old.clone()),
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        let cached_before_child_exit = load_state(&layout)?;
        assert_eq!(
            cached_before_child_exit
                .current
                .as_ref()
                .map(|build| build.build_id.as_str()),
            Some("old")
        );

        fs::rename(layout.current()?, layout.previous()?)?;
        write_slot(&layout.current()?, "new", b"new")?;
        fs::write(app.join("Contents/Resources/value"), b"new")?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                current: Some(new.clone()),
                previous: Some(old),
                ..runtime_launcher::RuntimeState::default()
            },
        )?;

        let mut authoritative = reconcile_launcher_state_with(&layout, &app, &SystemCommandRunner)?;
        let current = authoritative.current.clone().context("current build")?;
        record_crash(
            &mut authoritative,
            &current.artifact_content_hash,
            Utc::now(),
        );
        assert_eq!(current.build_id, new.build_id);
        assert!(
            authoritative
                .crash_history
                .iter()
                .all(|record| record.build_hash == new.artifact_content_hash)
        );
        Ok(())
    }

    #[test]
    fn producer_owned_prepared_root_is_preserved() -> Result<()> {
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
        let prepared = temp.path().join("producer-owned-prepared");
        fs::create_dir_all(prepared.join("resources"))?;
        fs::write(prepared.join("resources/value"), b"new")?;
        fs::write(prepared.join("producer-sentinel"), b"keep")?;
        write_json_atomically(
            &prepared.join("manifest.json"),
            &PreparedManifest {
                schema_version: SCHEMA_VERSION,
                build_id: "new".into(),
                source_commit: "commit-new".into(),
                artifacts: vec![artifact("value", b"new")],
                changes: ManifestChanges::default(),
            },
        )?;
        let result = prepare_activation(
            &layout,
            ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "new".into(),
                source_commit: "commit-new".into(),
                prepared_root: prepared.clone(),
                app_bundle_path: app.clone(),
                reason: "test".into(),
            },
        )?;
        assert_eq!(result.phase, TransactionPhase::Prepared);
        assert!(layout.transaction()?.exists());
        assert!(prepared.exists());
        assert_eq!(fs::read(prepared.join("producer-sentinel"))?, b"keep");
        let mut legacy_prepared: Transaction = read_json(&layout.transaction()?)?;
        legacy_prepared.launcher_expected_hash = None;
        write_json_atomically(&layout.transaction()?, &legacy_prepared)?;
        let pending = load_valid_prepared_transaction_on_startup(&layout, &app)?;
        assert_eq!(
            pending
                .as_ref()
                .map(|transaction| transaction.request.transaction_id.as_str()),
            Some("tx")
        );
        assert!(
            pending
                .as_ref()
                .and_then(|transaction| transaction.launcher_expected_hash.as_ref())
                .is_some()
        );
        fs::write(layout.staging()?.join("tx/resources/value"), b"corrupt")?;
        assert!(load_valid_prepared_transaction_on_startup(&layout, &app)?.is_none());
        assert!(!layout.transaction()?.exists());
        assert!(layout.failure_evidence()?.exists());
        assert_eq!(fs::read(prepared.join("producer-sentinel"))?, b"keep");
        Ok(())
    }

    #[test]
    fn legacy_prepared_without_readable_launcher_is_durably_aborted() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        fs::create_dir_all(app.join("Contents/Resources"))?;
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::write(app.join("Contents/Resources/value"), b"old")?;
        let launcher = app.join("Contents/MacOS/MorpheusLauncher");
        fs::write(&launcher, b"stable-launcher")?;
        let prepared = temp.path().join("prepared");
        fs::create_dir_all(prepared.join("resources"))?;
        fs::write(prepared.join("resources/value"), b"new")?;
        write_json_atomically(
            &prepared.join("manifest.json"),
            &PreparedManifest {
                schema_version: SCHEMA_VERSION,
                build_id: "new".into(),
                source_commit: "commit-new".into(),
                artifacts: vec![artifact("value", b"new")],
                changes: ManifestChanges::default(),
            },
        )?;
        prepare_activation(
            &layout,
            ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "legacy-tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "new".into(),
                source_commit: "commit-new".into(),
                prepared_root: prepared,
                app_bundle_path: app.clone(),
                reason: "test".into(),
            },
        )?;
        let mut transaction: Transaction = read_json(&layout.transaction()?)?;
        transaction.launcher_expected_hash = None;
        write_json_atomically(&layout.transaction()?, &transaction)?;
        fs::remove_file(launcher)?;

        assert!(load_valid_prepared_transaction_on_startup(&layout, &app)?.is_none());
        assert!(!layout.transaction()?.exists());
        let evidence: FailureEvidence = read_json(&layout.failure_evidence()?)?;
        assert_eq!(evidence.failure_phase, "prepared-startup-validation");
        Ok(())
    }

    #[test]
    fn invalid_persisted_transaction_ids_never_escape_staging() -> Result<()> {
        for invalid_id in ["", "..", "../x", "a/b", "/absolute"] {
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
            let prepared = temp.path().join("prepared");
            fs::create_dir_all(prepared.join("resources"))?;
            fs::write(prepared.join("resources/value"), b"new")?;
            write_json_atomically(
                &prepared.join("manifest.json"),
                &PreparedManifest {
                    schema_version: SCHEMA_VERSION,
                    build_id: "new".into(),
                    source_commit: "commit-new".into(),
                    artifacts: vec![artifact("value", b"new")],
                    changes: ManifestChanges::default(),
                },
            )?;
            prepare_activation(
                &layout,
                ActivationRequest {
                    schema_version: SCHEMA_VERSION,
                    transaction_id: "safe-tx".into(),
                    request_id: "request".into(),
                    requested_by_thread_id: Some("thread".into()),
                    mode: ActivationMode::Full,
                    build_id: "new".into(),
                    source_commit: "commit-new".into(),
                    prepared_root: prepared,
                    app_bundle_path: app.clone(),
                    reason: "test".into(),
                },
            )?;
            let mut transaction: Transaction = read_json(&layout.transaction()?)?;
            transaction.request.transaction_id = invalid_id.into();
            write_json_atomically(&layout.transaction()?, &transaction)?;
            let root_sentinel = layout.root.join("root-sentinel");
            let lock_sentinel = layout.root.join("launcher.lock");
            fs::write(&root_sentinel, b"keep")?;
            fs::write(&lock_sentinel, b"keep")?;

            assert!(load_valid_prepared_transaction_on_startup(&layout, &app)?.is_none());
            assert!(layout.root.exists());
            assert_eq!(fs::read(&root_sentinel)?, b"keep");
            assert_eq!(fs::read(&lock_sentinel)?, b"keep");
            assert!(layout.state()?.exists());
            assert!(layout.failure_evidence()?.exists());
            assert!(layout.staging()?.join("safe-tx").exists());
            assert!(!layout.transaction()?.exists());
        }
        Ok(())
    }

    #[test]
    fn ready_runtime_can_prepare_another_full_transaction_for_exit_75() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        fs::create_dir_all(&app)?;
        let transaction = Transaction {
            schema_version: SCHEMA_VERSION,
            request: ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "next-full".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Full,
                build_id: "next".into(),
                source_commit: "commit-next".into(),
                prepared_root: temp.path().join("prepared"),
                app_bundle_path: app.clone(),
                reason: "test".into(),
            },
            manifest_hash: "manifest-next".into(),
            artifact_content_hash: "content-next".into(),
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
        };
        write_json_atomically(&layout.transaction()?, &transaction)?;
        let loaded = load_prepared_full_transaction(&layout, &app)?;
        assert_eq!(loaded.request.transaction_id, "next-full");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn recovery_restart_reconciles_rolling_back_transaction_before_respawn() -> Result<()> {
        use std::ffi::OsStr;
        use std::os::unix::process::ExitStatusExt;
        use std::process::ExitStatus;

        struct SuccessfulRunner;
        impl runtime_launcher::CommandRunner for SuccessfulRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(0))
            }
        }

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        let resources = app.join("Contents/Resources");
        let backup = temp.path().join(".MorpheusResourcesPrevious-tx");
        let retired = temp.path().join(".MorpheusResourcesFailed-tx");
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::write(
            app.join("Contents/MacOS/MorpheusLauncher"),
            b"stable-launcher",
        )?;
        let launcher_hash = hash_file(&app.join("Contents/MacOS/MorpheusLauncher"))?;
        fs::create_dir_all(&backup)?;
        fs::write(backup.join("value"), b"old")?;
        fs::create_dir_all(&retired)?;
        fs::write(retired.join("value"), b"candidate")?;
        write_slot(&layout.current()?, "candidate", b"candidate")?;
        write_slot(&layout.previous()?, "old", b"old")?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                current: Some(build("old", &app)),
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        let transaction = Transaction {
            schema_version: SCHEMA_VERSION,
            request: ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Hot,
                build_id: "candidate".into(),
                source_commit: "commit-candidate".into(),
                prepared_root: temp.path().join("prepared"),
                app_bundle_path: app.clone(),
                reason: "test".into(),
            },
            manifest_hash: "manifest-candidate".into(),
            artifact_content_hash: "content-candidate".into(),
            phase: TransactionPhase::RollingBack,
            instance_id: None,
            slot_rotated: true,
            slot_rotation_phase: SlotRotationPhase::CandidateInstalled,
            slot_restore_phase: SlotRestorePhase::NotStarted,
            slot_retired_path: None,
            slot_failed_path: None,
            slot_had_previous: None,
            resources_swapped: true,
            resources_backup_path: Some(backup),
            resources_replacement_path: None,
            resources_retired_path: Some(retired),
            resource_swap_phase: ResourceSwapPhase::CandidateInstalled,
            resource_restore_phase: ResourceRestorePhase::DestinationRetired,
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

        let state = reconcile_launcher_state_with(&layout, &app, &SuccessfulRunner)?;
        assert_eq!(
            state.current.as_ref().map(|build| build.build_id.as_str()),
            Some("old")
        );
        assert_eq!(fs::read(resources.join("value"))?, b"old");
        assert!(!layout.transaction()?.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn interrupted_rollback_retries_reuse_one_durable_failure_identity() -> Result<()> {
        use std::ffi::OsStr;
        use std::os::unix::process::ExitStatusExt;
        use std::process::ExitStatus;

        struct VerifyFailRunner;
        impl runtime_launcher::CommandRunner for VerifyFailRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(1 << 8))
            }
        }

        struct SuccessfulRunner;
        impl runtime_launcher::CommandRunner for SuccessfulRunner {
            fn run(&self, _program: &str, _args: &[&OsStr]) -> Result<ExitStatus> {
                Ok(ExitStatus::from_raw(0))
            }
        }

        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let app = temp.path().join("App.app");
        let resources = app.join("Contents/Resources");
        let backup = temp.path().join(".MorpheusResourcesPrevious-tx");
        let retired = temp.path().join(".MorpheusResourcesFailed-tx");
        fs::create_dir_all(app.join("Contents/MacOS"))?;
        fs::write(
            app.join("Contents/MacOS/MorpheusLauncher"),
            b"stable-launcher",
        )?;
        let launcher_hash = hash_file(&app.join("Contents/MacOS/MorpheusLauncher"))?;
        fs::create_dir_all(&backup)?;
        fs::write(backup.join("value"), b"old")?;
        fs::create_dir_all(&retired)?;
        fs::write(retired.join("value"), b"candidate")?;
        write_slot(&layout.current()?, "candidate", b"candidate")?;
        write_slot(&layout.previous()?, "old", b"old")?;
        save_state(
            &layout,
            &runtime_launcher::RuntimeState {
                current: Some(build("old", &app)),
                ..runtime_launcher::RuntimeState::default()
            },
        )?;
        write_json_atomically(
            &layout.transaction()?,
            &Transaction {
                schema_version: SCHEMA_VERSION,
                request: ActivationRequest {
                    schema_version: SCHEMA_VERSION,
                    transaction_id: "tx".into(),
                    request_id: "request".into(),
                    requested_by_thread_id: Some("thread".into()),
                    mode: ActivationMode::Hot,
                    build_id: "candidate".into(),
                    source_commit: "commit-candidate".into(),
                    prepared_root: temp.path().join("prepared"),
                    app_bundle_path: app.clone(),
                    reason: "test".into(),
                },
                manifest_hash: "manifest-candidate".into(),
                artifact_content_hash: "content-candidate".into(),
                phase: TransactionPhase::RollingBack,
                instance_id: None,
                slot_rotated: true,
                slot_rotation_phase: SlotRotationPhase::CandidateInstalled,
                slot_restore_phase: SlotRestorePhase::NotStarted,
                slot_retired_path: None,
                slot_failed_path: None,
                slot_had_previous: None,
                resources_swapped: true,
                resources_backup_path: Some(backup),
                resources_replacement_path: None,
                resources_retired_path: Some(retired),
                resource_swap_phase: ResourceSwapPhase::CandidateInstalled,
                resource_restore_phase: ResourceRestorePhase::DestinationRetired,
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
            },
        )?;

        assert!(reconcile_launcher_state_with(&layout, &app, &VerifyFailRunner).is_err());
        let first_transaction: Transaction = read_json(&layout.transaction()?)?;
        let first_identity = first_transaction
            .rollback_failure_evidence
            .as_ref()
            .and_then(|evidence| evidence.recovery_identity.clone())
            .context("first failed recovery should retain durable identity")?;
        assert_eq!(load_state(&layout)?.failures.len(), 1);

        assert!(reconcile_launcher_state_with(&layout, &app, &VerifyFailRunner).is_err());
        let second_transaction: Transaction = read_json(&layout.transaction()?)?;
        assert_eq!(
            second_transaction
                .rollback_failure_evidence
                .as_ref()
                .and_then(|evidence| evidence.recovery_identity.as_deref()),
            Some(first_identity.as_str())
        );
        assert_eq!(load_state(&layout)?.failures.len(), 1);
        assert_eq!(
            read_json::<FailureEvidence>(&layout.failure_evidence()?)?
                .recovery_identity
                .as_deref(),
            Some(first_identity.as_str())
        );
        assert!(!pending_failure_evidence_path(&layout, &first_identity)?.exists());

        let state = reconcile_launcher_state_with(&layout, &app, &SuccessfulRunner)?;
        assert!(!layout.transaction()?.exists());
        assert_eq!(state.failures.len(), 1);
        assert_eq!(
            state.failures[0].recovery_identity.as_deref(),
            Some(first_identity.as_str())
        );
        assert_eq!(fs::read(resources.join("value"))?, b"old");
        assert!(!pending_failure_evidence_path(&layout, &first_identity)?.exists());
        Ok(())
    }

    #[test]
    fn repeated_rollback_hot_finalizes_rollback_complete_without_phase_regression() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        fs::create_dir_all(&layout.root)?;
        let request = ActivationRequest {
            schema_version: SCHEMA_VERSION,
            transaction_id: "tx".into(),
            request_id: "request".into(),
            requested_by_thread_id: Some("thread".into()),
            mode: ActivationMode::Hot,
            build_id: "candidate".into(),
            source_commit: "commit".into(),
            prepared_root: temp.path().join("prepared"),
            app_bundle_path: temp.path().join("App.app"),
            reason: "test".into(),
        };
        let mut evidence = request_failure(
            &request,
            Some("manifest".into()),
            "pre-ready",
            "host rejected candidate",
        );
        evidence.failed_build_hash = Some("content".into());
        let remaining_backup = temp.path().join(".MorpheusSignaturePrevious-tx");
        fs::create_dir_all(&remaining_backup)?;
        let transaction = Transaction {
            schema_version: SCHEMA_VERSION,
            request,
            manifest_hash: "manifest".into(),
            artifact_content_hash: "content".into(),
            phase: TransactionPhase::RollbackComplete,
            instance_id: None,
            slot_rotated: true,
            slot_rotation_phase: SlotRotationPhase::CandidateInstalled,
            slot_restore_phase: SlotRestorePhase::RetiredRestored,
            slot_retired_path: None,
            slot_failed_path: None,
            slot_had_previous: Some(false),
            resources_swapped: true,
            resources_backup_path: None,
            resources_replacement_path: None,
            resources_retired_path: None,
            resource_swap_phase: ResourceSwapPhase::CandidateInstalled,
            resource_restore_phase: ResourceRestorePhase::BackupRestored,
            signature_backup_path: Some(remaining_backup),
            signature_backup_ready: Some(true),
            signature_backup_had_code_signature: Some(true),
            signature_restore_phase: SignatureRestorePhase::SignatureRestored,
            runtime_retired_path: None,
            runtime_replacement_path: None,
            signature_retired_path: None,
            signature_replacement_path: None,
            signature_replacement_ready: Some(true),
            launcher_expected_hash: Some("launcher".into()),
            rollback_failure_evidence: Some(evidence),
            post_ready_rollback: None,
            started_at: Utc::now(),
        };
        write_json_atomically(&layout.transaction()?, &transaction)?;
        rollback_hot(&layout, "tx")?;
        assert!(!layout.transaction()?.exists());
        assert_eq!(load_state(&layout)?.failures.len(), 1);

        write_json_atomically(&layout.transaction()?, &transaction)?;
        rollback_hot(&layout, "tx")?;
        let state = load_state(&layout)?;
        assert_eq!(state.failures.len(), 1);
        assert_eq!(state.failures[0].summary, "host rejected candidate");
        Ok(())
    }

    #[test]
    fn polluted_persisted_bundle_path_cannot_import_external_recovery_artifacts() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let old_parent = temp.path().join("old");
        let new_parent = temp.path().join("new");
        let old_app = old_parent.join("App.app");
        let new_app = new_parent.join("Moved.app");
        fs::create_dir_all(old_app.join("Contents/MacOS"))?;
        fs::write(
            old_app.join("Contents/MacOS/MorpheusLauncher"),
            b"stable-launcher",
        )?;
        fs::create_dir_all(new_app.join("Contents/MacOS"))?;
        fs::write(
            new_app.join("Contents/MacOS/MorpheusLauncher"),
            b"trusted-launcher",
        )?;
        let replacement = old_app.join("Contents/.MorpheusResourcesCandidate-tx");
        let backup = old_parent.join(".MorpheusResourcesPrevious-tx");
        let retired = old_parent.join(".MorpheusResourcesFailed-tx");
        let signature = old_parent.join(".MorpheusSignaturePrevious-tx");
        for path in [&replacement, &backup, &retired, &signature] {
            fs::create_dir_all(path)?;
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
                app_bundle_path: old_app.clone(),
                reason: "test".into(),
            },
            manifest_hash: "manifest".into(),
            artifact_content_hash: "content".into(),
            phase: TransactionPhase::Activating,
            instance_id: None,
            slot_rotated: true,
            slot_rotation_phase: SlotRotationPhase::CandidateInstalled,
            slot_restore_phase: SlotRestorePhase::NotStarted,
            slot_retired_path: None,
            slot_failed_path: None,
            slot_had_previous: None,
            resources_swapped: false,
            resources_backup_path: Some(backup),
            resources_replacement_path: Some(replacement),
            resources_retired_path: Some(retired),
            resource_swap_phase: ResourceSwapPhase::ReplacementPrepared,
            resource_restore_phase: ResourceRestorePhase::NotStarted,
            signature_backup_path: Some(signature),
            signature_backup_ready: Some(true),
            signature_backup_had_code_signature: Some(true),
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

        assert!(migrate_transaction_app_bundle_path(&layout, &new_app).is_err());
        let persisted: Transaction = read_json(&layout.transaction()?)?;
        assert_eq!(persisted.request.app_bundle_path, old_app);
        for path in [
            persisted.resources_backup_path,
            persisted.resources_retired_path,
            persisted.signature_backup_path,
        ]
        .into_iter()
        .flatten()
        {
            assert!(path.exists());
            assert!(path.starts_with(&old_parent));
        }
        assert!(!new_parent.join(".MorpheusResourcesPrevious-tx").exists());
        assert!(!new_parent.join(".MorpheusResourcesFailed-tx").exists());
        assert!(!new_parent.join(".MorpheusSignaturePrevious-tx").exists());
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn active_transaction_accepts_var_alias_for_same_bundle_and_parent() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let canonical_app = temp.path().join("App.app");
        fs::create_dir_all(canonical_app.join("Contents/MacOS"))?;
        fs::write(
            canonical_app.join("Contents/MacOS/MorpheusLauncher"),
            b"stable-launcher",
        )?;
        let canonical_app = canonical_app.canonicalize()?;
        let alias_app = Path::new("/").join(
            canonical_app
                .strip_prefix("/private")
                .context("macOS temporary directory should resolve below /private")?,
        );
        assert_ne!(alias_app, canonical_app);
        assert_eq!(alias_app.canonicalize()?, canonical_app);
        assert_eq!(
            alias_app
                .parent()
                .context("alias bundle parent")?
                .canonicalize()?,
            canonical_app
                .parent()
                .context("canonical bundle parent")?
                .canonicalize()?
        );

        let transaction = Transaction {
            schema_version: SCHEMA_VERSION,
            request: ActivationRequest {
                schema_version: SCHEMA_VERSION,
                transaction_id: "tx".into(),
                request_id: "request".into(),
                requested_by_thread_id: Some("thread".into()),
                mode: ActivationMode::Hot,
                build_id: "candidate".into(),
                source_commit: "commit".into(),
                prepared_root: temp.path().join("prepared"),
                app_bundle_path: alias_app.clone(),
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

        migrate_transaction_app_bundle_path(&layout, &canonical_app)?;

        let persisted: Transaction = read_json(&layout.transaction()?)?;
        assert_eq!(persisted.request.app_bundle_path, alias_app);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn active_transaction_rejects_mutable_bundle_and_parent_symlink_aliases() -> Result<()> {
        use std::os::unix::fs::symlink;

        fn assert_alias_rejected(parent_alias: bool) -> Result<()> {
            let temp = TempDir::new()?;
            let layout = Layout::new(temp.path().join("launcher"));
            let real_parent = temp.path().join("real");
            let app = real_parent.join("App.app");
            fs::create_dir_all(app.join("Contents/MacOS"))?;
            fs::write(
                app.join("Contents/MacOS/MorpheusLauncher"),
                b"stable-launcher",
            )?;
            fs::write(real_parent.join("parent-sentinel"), b"parent")?;
            fs::write(app.join("bundle-sentinel"), b"bundle")?;
            let persisted_app = if parent_alias {
                let alias = temp.path().join("alias-parent");
                symlink(&real_parent, &alias)?;
                alias.join("App.app")
            } else {
                let alias = real_parent.join("Alias.app");
                symlink(&app, &alias)?;
                alias
            };
            let derived_artifacts = [
                real_parent.join(".MorpheusResourcesPrevious-tx"),
                real_parent.join(".MorpheusResourcesFailed-tx"),
                real_parent.join(".MorpheusSignaturePrevious-tx"),
                app.join("Contents/.MorpheusResourcesCandidate-tx"),
            ];
            let transaction = Transaction {
                schema_version: SCHEMA_VERSION,
                request: ActivationRequest {
                    schema_version: SCHEMA_VERSION,
                    transaction_id: "tx".into(),
                    request_id: "request".into(),
                    requested_by_thread_id: Some("thread".into()),
                    mode: ActivationMode::Hot,
                    build_id: "candidate".into(),
                    source_commit: "commit".into(),
                    prepared_root: temp.path().join("prepared"),
                    app_bundle_path: persisted_app.clone(),
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
            let transaction_path = layout.transaction()?;
            write_json_atomically(&transaction_path, &transaction)?;
            let transaction_before = fs::read(&transaction_path)?;
            let state_root_before = directory_file_snapshot(&layout.active_root()?)?;
            assert!(!layout.staging()?.exists());

            assert!(migrate_transaction_app_bundle_path(&layout, &app).is_err());

            assert_eq!(fs::read(&transaction_path)?, transaction_before);
            assert_eq!(
                directory_file_snapshot(&layout.active_root()?)?,
                state_root_before
            );
            assert!(!layout.staging()?.exists());
            let persisted: Transaction = read_json(&transaction_path)?;
            assert_eq!(persisted.request.app_bundle_path, persisted_app);
            assert_eq!(fs::read(real_parent.join("parent-sentinel"))?, b"parent");
            assert_eq!(fs::read(app.join("bundle-sentinel"))?, b"bundle");
            for artifact in &derived_artifacts {
                assert!(!artifact.exists());
            }
            Ok(())
        }

        assert_alias_rejected(false)?;
        assert_alias_rejected(true)?;
        Ok(())
    }

    #[test]
    fn post_ready_bundle_relocation_fails_closed_without_path_rewrite() -> Result<()> {
        let temp = TempDir::new()?;
        let layout = Layout::new(temp.path().join("launcher"));
        let old_parent = temp.path().join("old");
        let new_parent = temp.path().join("new");
        let old_app = old_parent.join("App.app");
        let new_app = new_parent.join("Moved.app");
        fs::create_dir_all(old_app.join("Contents/MacOS"))?;
        fs::write(
            old_app.join("Contents/MacOS/MorpheusLauncher"),
            b"stable-launcher",
        )?;
        fs::create_dir_all(&new_parent)?;
        let failed = build("failed", &old_app);
        let fallback = build("fallback", &old_app);
        let evidence = generic_failure(Some(failed.clone()), "post-ready-crash", "rollback");
        let transaction = begin_post_ready_rollback(&layout, failed, fallback, evidence)?;
        let rollback = transaction
            .post_ready_rollback
            .as_ref()
            .context("post-ready rollback context")?;
        fs::create_dir_all(&rollback.resources_replacement_path)?;
        fs::create_dir_all(&rollback.resources_backup_path)?;
        let failed_slot_path = rollback.failed_slot_path.clone();
        fs::rename(&old_app, &new_app)?;

        assert!(migrate_transaction_app_bundle_path(&layout, &new_app).is_err());
        let persisted: Transaction = read_json(&layout.transaction()?)?;
        assert_eq!(persisted.request.app_bundle_path, old_app);
        let rollback = persisted
            .post_ready_rollback
            .context("persisted post-ready rollback context")?;
        assert_eq!(rollback.failed_slot_path, failed_slot_path);
        assert!(rollback.resources_backup_path.exists());
        assert!(rollback.resources_backup_path.starts_with(&old_parent));
        assert_eq!(rollback.failure_evidence.app_bundle_path, Some(old_app));
        let backup_name = rollback
            .resources_backup_path
            .file_name()
            .context("rollback backup has no file name")?;
        assert!(!new_parent.join(backup_name).exists());
        Ok(())
    }
}
