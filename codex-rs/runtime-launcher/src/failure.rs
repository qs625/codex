use crate::ArtifactIdentity;
use crate::ArtifactRecord;
use crate::LauncherError;
use crate::Result;
use crate::state::LauncherPaths;
use crate::state::read_json_if_exists;
use crate::state::remove_file_if_exists;
use crate::state::unix_time_ms;
use crate::state::write_json_atomic;
use crate::transaction::TransactionType;
use serde::Deserialize;
use serde::Serialize;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureEvidence {
    pub schema_version: u32,
    pub recovery_identity: String,
    pub mode: TransactionType,
    pub failed: ArtifactIdentity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<ArtifactIdentity>,
    pub reason: String,
    pub observed_at_unix_ms: u64,
}

pub(crate) fn record_failure(
    paths: &LauncherPaths,
    failed: &ArtifactRecord,
    fallback: Option<&ArtifactRecord>,
    mode: TransactionType,
    reason: impl Into<String>,
) -> Result<FailureEvidence> {
    let evidence = FailureEvidence {
        schema_version: crate::state::SCHEMA_VERSION,
        recovery_identity: failed.identity.recovery_identity(),
        mode,
        failed: failed.identity.clone(),
        fallback: fallback.map(|item| item.identity.clone()),
        reason: reason.into(),
        observed_at_unix_ms: unix_time_ms(),
    };
    if let Some(existing) = read_json_if_exists::<FailureEvidence>(&paths.failure_evidence)? {
        if existing.recovery_identity == evidence.recovery_identity {
            return Ok(existing);
        }
        return Err(LauncherError::Conflict(format!(
            "unacknowledged failure evidence {} prevents recording {}",
            existing.recovery_identity, evidence.recovery_identity
        )));
    }
    write_json_atomic(&paths.failure_evidence, &evidence)?;
    Ok(evidence)
}

pub(crate) fn ack_failure_evidence(
    paths: &LauncherPaths,
    recovery_identity: &str,
) -> Result<bool> {
    let Some(evidence) = read_json_if_exists::<FailureEvidence>(&paths.failure_evidence)? else {
        return Ok(false);
    };
    if evidence.recovery_identity != recovery_identity {
        return Err(LauncherError::Conflict(format!(
            "failure evidence belongs to {}, not {}",
            evidence.recovery_identity, recovery_identity
        )));
    }
    remove_file_if_exists(&paths.failure_evidence)?;
    Ok(true)
}

pub(crate) fn require_no_failure_evidence(paths: &LauncherPaths) -> Result<()> {
    if let Some(evidence) = read_json_if_exists::<FailureEvidence>(&paths.failure_evidence)? {
        return Err(LauncherError::Conflict(format!(
            "failure evidence {} must be acknowledged before another activation",
            evidence.recovery_identity
        )));
    }
    Ok(())
}
