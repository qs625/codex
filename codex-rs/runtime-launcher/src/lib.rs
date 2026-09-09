mod artifact;
mod failure;
mod platform;
mod state;
mod supervisor;
mod transaction;

pub use artifact::ArtifactManifest;
pub use artifact::ArtifactManifestEntry;
pub use artifact::PreparedArtifactRequest;
pub use failure::FailureEvidence;
pub use state::ArtifactIdentity;
pub use state::ArtifactRecord;
pub use state::LauncherPaths;
pub use state::LauncherState;
pub use supervisor::EXIT_COORDINATED_RESTART;
pub use supervisor::RunOutcome;
pub use transaction::ActivationChanges;
pub use transaction::HotActivationRequest;
pub use transaction::TransactionRecord;
pub use transaction::TransactionFailure;
pub use transaction::TransactionState;
pub use transaction::TransactionType;

use crate::artifact::install_prepared_artifact;
use crate::artifact::plan_prepared_artifact;
use crate::failure::ack_failure_evidence;
use crate::failure::require_no_failure_evidence;
use crate::state::OperationLock;
use crate::state::read_json_if_exists;
use crate::state::write_json_atomic;
use crate::transaction::load_transaction;
use crate::transaction::require_no_transaction;
use crate::transaction::activate_transaction;
use crate::transaction::ActivationOutcome;
use crate::transaction::RecoveredActivationFailure;
use crate::transaction::commit_transaction;
use crate::transaction::rollback_transaction;
use crate::transaction::rollback_failure;
use crate::transaction::recovered_transaction_failure;
use crate::transaction::reconcile_transaction;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, LauncherError>;

#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{context}: {source}")]
    Json {
        context: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("invalid artifact: {0}")]
    InvalidArtifact(String),
    #[error("launcher state conflict: {0}")]
    Conflict(String),
    #[error("runtime launch failed: {0}")]
    Launch(String),
}

pub(crate) fn io_error(context: impl Into<String>, source: std::io::Error) -> LauncherError {
    LauncherError::Io {
        context: context.into(),
        source,
    }
}

pub(crate) fn json_error(
    context: impl Into<String>,
    source: serde_json::Error,
) -> LauncherError {
    LauncherError::Json {
        context: context.into(),
        source,
    }
}

pub fn read_request<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes =
        std::fs::read(path).map_err(|err| io_error(format!("read {}", path.display()), err))?;
    serde_json::from_slice(&bytes)
        .map_err(|err| json_error(format!("parse {}", path.display()), err))
}

pub fn prepare_full(paths: &LauncherPaths, request_path: &Path) -> Result<TransactionRecord> {
    let _lock = OperationLock::acquire(paths)?;
    reconcile_and_record_failure(paths)?;
    require_no_failure_evidence(paths)?;
    require_no_transaction(paths)?;
    let request: PreparedArtifactRequest = read_request(request_path)?;
    request.validate_common()?;
    let planned = plan_prepared_artifact(paths, &request)?;
    let mut transaction = TransactionRecord::full_prepared(request.clone(), planned);
    write_json_atomic(&paths.transaction, &transaction)?;
    let artifact = match install_prepared_artifact(paths, &request) {
        Ok(artifact) => artifact,
        Err(error) => {
            state::remove_file_if_exists(&paths.transaction)?;
            return Err(error);
        }
    };
    transaction.candidate = artifact;
    write_json_atomic(&paths.transaction, &transaction)?;
    Ok(transaction)
}

pub fn activate_hot(paths: &LauncherPaths, request_path: &Path) -> Result<TransactionRecord> {
    let _lock = OperationLock::acquire(paths)?;
    reconcile_and_record_failure(paths)?;
    require_no_failure_evidence(paths)?;
    require_no_transaction(paths)?;
    let request: HotActivationRequest = read_request(request_path)?;
    request.validate()?;
    let mut state = LauncherState::load(paths)?;
    let previous = state.current.clone().ok_or_else(|| {
        LauncherError::Conflict("hot activation requires an existing current artifact".to_string())
    })?;
    let planned = plan_prepared_artifact(paths, &request.artifact)?;
    let mut transaction = TransactionRecord::hot_prepared(request.clone(), planned);
    write_json_atomic(&paths.transaction, &transaction)?;
    let artifact = match install_prepared_artifact(paths, &request.artifact) {
        Ok(artifact) => artifact,
        Err(error) => {
            state::remove_file_if_exists(&paths.transaction)?;
            return Err(error);
        }
    };
    transaction.candidate = artifact;
    write_json_atomic(&paths.transaction, &transaction)?;
    let (current, previous) = match activate_transaction(paths, &mut transaction, &previous)? {
        ActivationOutcome::Activated { current, previous } => (current, previous),
        ActivationOutcome::RecoveredFailure(failure) => {
            return record_recovered_activation_failure(paths, failure);
        }
    };
    state.previous = Some(previous);
    state.current = Some(current);
    state.save(paths)?;
    Ok(transaction)
}

pub fn abort_full(paths: &LauncherPaths, transaction_id: &str) -> Result<LauncherState> {
    let _lock = OperationLock::acquire(paths)?;
    reconcile_and_record_failure(paths)?;
    let transaction = load_transaction(paths)?.ok_or_else(|| {
        LauncherError::Conflict("there is no prepared full transaction".to_string())
    })?;
    transaction.require(TransactionType::Full, transaction_id)?;
    if transaction.state != TransactionState::Prepared {
        return Err(LauncherError::Conflict(format!(
            "full transaction {} is not prepared",
            transaction.transaction_id
        )));
    }
    transaction::abort_prepared_full(paths, &transaction)?;
    LauncherState::load(paths)
}

pub fn commit_hot(paths: &LauncherPaths, transaction_id: &str) -> Result<LauncherState> {
    let _lock = OperationLock::acquire(paths)?;
    reconcile_and_record_failure(paths)?;
    let transaction = load_transaction(paths)?.ok_or_else(|| {
        LauncherError::Conflict("there is no active hot transaction".to_string())
    })?;
    transaction.require(TransactionType::Hot, transaction_id)?;
    if transaction.state != TransactionState::Launching {
        return Err(LauncherError::Conflict(format!(
            "hot transaction {} is not launching",
            transaction.transaction_id
        )));
    }
    let state = LauncherState::load(paths)?;
    if state.current.as_ref().map(|item| &item.identity) != Some(&transaction.candidate.identity) {
        return Err(LauncherError::Conflict(
            "current artifact does not match the hot transaction candidate".to_string(),
        ));
    }
    commit_transaction(paths, &transaction, state)
}

pub fn rollback_hot(paths: &LauncherPaths, transaction_id: &str) -> Result<LauncherState> {
    let _lock = OperationLock::acquire(paths)?;
    reconcile_and_record_failure(paths)?;
    let mut transaction = load_transaction(paths)?.ok_or_else(|| {
        LauncherError::Conflict("there is no active hot transaction".to_string())
    })?;
    transaction.require(TransactionType::Hot, transaction_id)?;
    if transaction.state != TransactionState::Launching {
        return Err(LauncherError::Conflict(format!(
            "hot transaction {} is not launching",
            transaction.transaction_id
        )));
    }
    let state = LauncherState::load(paths)?;
    let failure = rollback_failure(&transaction, "hot activation was rolled back")?;
    let state = rollback_transaction(paths, &mut transaction, state, failure)?;
    let recovered = recovered_transaction_failure(&transaction)?;
    persist_recovered_activation_failure(paths, &recovered)?;
    Ok(state)
}

pub fn ack_failure(paths: &LauncherPaths, recovery_identity: &str) -> Result<bool> {
    let _lock = OperationLock::acquire(paths)?;
    ack_failure_evidence(paths, recovery_identity)
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: LauncherState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction: Option<TransactionRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_evidence: Option<FailureEvidence>,
}

pub fn status(paths: &LauncherPaths) -> Result<Status> {
    let _lock = OperationLock::acquire(paths)?;
    reconcile_and_record_failure(paths)?;
    Ok(Status {
        state: LauncherState::load(paths)?,
        transaction: load_transaction(paths)?,
        failure_evidence: read_json_if_exists(&paths.failure_evidence)?,
    })
}

pub fn run(paths: &LauncherPaths, app_bundle: &Path) -> Result<RunOutcome> {
    supervisor::run(paths, app_bundle)
}

pub(crate) fn reconcile_and_record_failure(paths: &LauncherPaths) -> Result<()> {
    if let Some(failure) = reconcile_transaction(paths)? {
        return record_recovered_activation_failure(paths, failure);
    }
    Ok(())
}

pub(crate) fn record_recovered_activation_failure<T>(
    paths: &LauncherPaths,
    failure: RecoveredActivationFailure,
) -> Result<T> {
    persist_recovered_activation_failure(paths, &failure)?;
    Err(failure.error)
}

pub(crate) fn persist_recovered_activation_failure(
    paths: &LauncherPaths,
    failure: &RecoveredActivationFailure,
) -> Result<()> {
    failure::record_failure(
        paths,
        &failure.failed,
        Some(&failure.fallback),
        failure.mode,
        &failure.reason,
    )?;
    state::remove_file_if_exists(&paths.transaction)?;
    Ok(())
}

pub fn state_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path);
    }
    std::env::var_os("MORPHEUS_RUNTIME_LAUNCHER_HOME")
        .map(PathBuf::from)
        .ok_or_else(|| {
            LauncherError::InvalidRequest(
                "--state-root or MORPHEUS_RUNTIME_LAUNCHER_HOME is required".to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovered_activation_failure_records_single_evidence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("state");
        let app_bundle = temp.path().join("Morpheus.app");
        let failed = artifact("tx", "new", &app_bundle, "candidate");
        let fallback = artifact("installed", "old", &app_bundle, "Resources");

        let error = record_recovered_activation_failure::<()>(
            &paths,
            RecoveredActivationFailure {
                failed: failed.clone(),
                fallback: fallback.clone(),
                mode: TransactionType::Full,
                reason: "candidate signature is invalid".to_string(),
                error: LauncherError::InvalidArtifact(
                    "candidate signature is invalid".to_string(),
                ),
            },
        )
        .expect_err("activation must remain failed");
        assert!(error.to_string().contains("candidate signature is invalid"));

        let evidence: FailureEvidence =
            read_json_if_exists(&paths.failure_evidence)
                .expect("read evidence")
                .expect("evidence");
        assert_eq!(evidence.mode, TransactionType::Full);
        assert_eq!(evidence.failed, failed.identity);
        assert_eq!(evidence.fallback, Some(fallback.identity));
    }

    fn artifact(
        transaction_id: &str,
        build_id: &str,
        app_bundle: &Path,
        root: &str,
    ) -> ArtifactRecord {
        let artifact_root = app_bundle.join("Contents").join(root);
        ArtifactRecord {
            identity: ArtifactIdentity {
                transaction_id: transaction_id.to_string(),
                build_id: build_id.to_string(),
                source_commit: "commit".to_string(),
            },
            entrypoint: artifact_root.join("app.asar"),
            artifact_root,
            app_bundle_path: app_bundle.to_path_buf(),
            installed_at_unix_ms: 1,
        }
    }
}
