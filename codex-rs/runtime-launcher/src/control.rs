use crate::LauncherError;
use crate::Result;
use crate::io_error;
use crate::json_error;
use crate::process::ProcessGroupRecord;
use crate::process::ProcessIdentity;
use crate::readiness::ReadyExpectation;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub const CONTROL_SCHEMA_VERSION: u32 = 2;

#[derive(Clone, Debug)]
pub struct ControlPaths {
    pub root: PathBuf,
    pub control: PathBuf,
    pub state_lock: PathBuf,
    pub legacy: PathBuf,
}

impl ControlPaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            control: root.join("control.json"),
            state_lock: root.join("state.lock"),
            legacy: root.join("legacy"),
            root,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)
            .map_err(|error| io_error(format!("create {}", self.root.display()), error))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapsuleRef {
    pub release_id: String,
    pub root: PathBuf,
    pub entrypoint: PathBuf,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustedSeed {
    pub capsule: CapsuleRef,
    pub trust_anchor: PathBuf,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum SelectedRuntime {
    Seed { capsule: CapsuleRef },
    External { capsule: CapsuleRef },
}

impl SelectedRuntime {
    pub fn seed(capsule: CapsuleRef) -> Self {
        Self::Seed { capsule }
    }

    pub fn external(capsule: CapsuleRef) -> Self {
        Self::External { capsule }
    }

    pub fn capsule(&self) -> &CapsuleRef {
        match self {
            Self::Seed { capsule } | Self::External { capsule } => capsule,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationPhase {
    Prepared,
    StoppingOld,
    OldStopped,
    SpawnPlanned,
    RuntimeStarted,
    AwaitingReady,
    Observing,
    CommitDecided,
    CommitRelaunch,
    CommitCleanup,
    RollbackDecided,
    StoppingCandidate,
    CandidateStopped,
    Restoring,
    EvidencePending,
    EvidenceCleanup,
    #[default]
    Idle,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PayloadRegistration {
    pub release_id: String,
    pub launch_instance_id: String,
    pub spawn_attempt_id: String,
    pub payload: ProcessIdentity,
    pub process_group: ProcessGroupRecord,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptRecord {
    pub attempt_id: String,
    pub candidate: CapsuleRef,
    pub previous: SelectedRuntime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_external_current: Option<CapsuleRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_external_previous: Option<CapsuleRef>,
    pub started_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_registration: Option<PayloadRegistration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_expectation: Option<ReadyExpectation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_descendants: Vec<ProcessIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_deadline_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub annotations: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WinnerRecord {
    pub selected: SelectedRuntime,
    pub decided_at_unix_ms: u64,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CleanupRecord {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<CapsuleRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_release_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationOutcome {
    Committed,
    RolledBack,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationReceipt {
    pub attempt_id: String,
    pub candidate_release_id: String,
    pub outcome: ActivationOutcome,
    pub selected: SelectedRuntime,
    pub completed_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FailureProjection {
    pub activation_id: String,
    pub release_id: String,
    pub occurred_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_release_id: Option<String>,
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<CapsuleRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<SelectedRuntime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub details: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveLaunchPhase {
    SpawnPlanned,
    Running,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveLaunchRecord {
    pub selected: SelectedRuntime,
    pub launch_instance_id: String,
    pub spawn_attempt_id: String,
    pub phase: ActiveLaunchPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_registration: Option<PayloadRegistration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_expectation: Option<ReadyExpectation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_descendants: Vec<ProcessIdentity>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActivationRecord {
    pub phase: ActivationPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<AttemptRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub winner: Option<WinnerRecord>,
    #[serde(default)]
    pub cleanup: CleanupRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ActivationReceipt>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlState {
    pub schema_version: u32,
    pub revision: u64,
    pub executor_epoch: u64,
    pub trusted_seed: TrustedSeed,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_current: Option<CapsuleRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_previous: Option<CapsuleRef>,
    pub selected: SelectedRuntime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_launch: Option<ActiveLaunchRecord>,
    #[serde(default)]
    pub activation: ActivationRecord,
}

impl ControlState {
    pub fn initialize(trusted_seed: TrustedSeed) -> Self {
        Self {
            schema_version: CONTROL_SCHEMA_VERSION,
            revision: 0,
            executor_epoch: 0,
            selected: SelectedRuntime::seed(trusted_seed.capsule.clone()),
            trusted_seed,
            external_current: None,
            external_previous: None,
            active_launch: None,
            activation: ActivationRecord::default(),
        }
    }

    pub fn load(paths: &ControlPaths) -> Result<Option<Self>> {
        load_unlocked(paths)
    }

    pub fn load_or_initialize(paths: &ControlPaths, trusted_seed: TrustedSeed) -> Result<Self> {
        let _lock = StateLock::acquire(paths)?;
        if let Some(state) = load_unlocked(paths)? {
            return Ok(state);
        }
        let state = Self::initialize(trusted_seed);
        state.validate()?;
        write_json_atomic(&paths.control, &state)?;
        Ok(state)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CONTROL_SCHEMA_VERSION {
            return Err(LauncherError::Conflict(format!(
                "unsupported control schema {}",
                self.schema_version
            )));
        }
        validate_capsule("trusted Seed", &self.trusted_seed.capsule)?;
        if !self.trusted_seed.trust_anchor.is_absolute() {
            return Err(LauncherError::Conflict(
                "trusted Seed trust anchor must be absolute".to_string(),
            ));
        }
        if let Some(current) = &self.external_current {
            validate_capsule("external current", current)?;
        }
        if let Some(previous) = &self.external_previous {
            validate_capsule("external previous", previous)?;
        }
        if self
            .external_current
            .as_ref()
            .map(|capsule| &capsule.release_id)
            == self
                .external_previous
                .as_ref()
                .map(|capsule| &capsule.release_id)
            && self.external_current.is_some()
        {
            return Err(LauncherError::Conflict(
                "external current and previous capsules must differ".to_string(),
            ));
        }
        if self.external_current.is_none() && self.external_previous.is_some() {
            return Err(LauncherError::Conflict(
                "external previous requires an external current capsule".to_string(),
            ));
        }
        match &self.selected {
            SelectedRuntime::Seed { capsule } if capsule != &self.trusted_seed.capsule => {
                return Err(LauncherError::Conflict(
                    "selected Seed does not match trusted Seed".to_string(),
                ));
            }
            SelectedRuntime::External { capsule }
                if self.external_current.as_ref() != Some(capsule) =>
            {
                return Err(LauncherError::Conflict(
                    "selected external runtime does not match external current".to_string(),
                ));
            }
            _ => {}
        }
        if let Some(active_launch) = &self.active_launch {
            validate_active_launch(active_launch)?;
        }
        self.validate_activation()?;
        Ok(())
    }

    fn validate_activation(&self) -> Result<()> {
        let activation = &self.activation;
        let attempt_required = activation.phase != ActivationPhase::Idle;
        if attempt_required && activation.attempt.is_none() {
            return Err(LauncherError::Conflict(format!(
                "activation phase {:?} requires an activation attempt",
                activation.phase
            )));
        }
        if activation.phase == ActivationPhase::Idle && activation.attempt.is_some() {
            return Err(LauncherError::Conflict(
                "idle activation must not retain an activation attempt".to_string(),
            ));
        }

        let winner_required = matches!(
            activation.phase,
            ActivationPhase::CommitDecided
                | ActivationPhase::CommitRelaunch
                | ActivationPhase::CommitCleanup
                | ActivationPhase::RollbackDecided
                | ActivationPhase::StoppingCandidate
                | ActivationPhase::CandidateStopped
                | ActivationPhase::Restoring
                | ActivationPhase::EvidencePending
                | ActivationPhase::EvidenceCleanup
        );
        let winner_forbidden = matches!(
            activation.phase,
            ActivationPhase::Prepared
                | ActivationPhase::StoppingOld
                | ActivationPhase::OldStopped
                | ActivationPhase::SpawnPlanned
                | ActivationPhase::RuntimeStarted
                | ActivationPhase::AwaitingReady
                | ActivationPhase::Observing
        );
        if winner_required && activation.winner.is_none() {
            return Err(LauncherError::Conflict(format!(
                "activation phase {:?} requires a winner",
                activation.phase
            )));
        }
        if winner_forbidden && activation.winner.is_some() {
            return Err(LauncherError::Conflict(format!(
                "activation phase {:?} must not have a winner",
                activation.phase
            )));
        }

        let receipt_required = matches!(
            activation.phase,
            ActivationPhase::CommitDecided
                | ActivationPhase::CommitRelaunch
                | ActivationPhase::CommitCleanup
                | ActivationPhase::Restoring
                | ActivationPhase::EvidencePending
                | ActivationPhase::EvidenceCleanup
        );
        let receipt_forbidden = matches!(
            activation.phase,
            ActivationPhase::Prepared
                | ActivationPhase::StoppingOld
                | ActivationPhase::OldStopped
                | ActivationPhase::SpawnPlanned
                | ActivationPhase::RuntimeStarted
                | ActivationPhase::AwaitingReady
                | ActivationPhase::Observing
                | ActivationPhase::RollbackDecided
                | ActivationPhase::StoppingCandidate
                | ActivationPhase::CandidateStopped
        );
        if receipt_required && activation.receipt.is_none() {
            return Err(LauncherError::Conflict(format!(
                "activation phase {:?} requires a receipt",
                activation.phase
            )));
        }
        if receipt_forbidden && activation.receipt.is_some() {
            return Err(LauncherError::Conflict(format!(
                "activation phase {:?} must not have a receipt",
                activation.phase
            )));
        }

        validate_cleanup_disjoint(self)?;
        let Some(attempt) = &activation.attempt else {
            return Ok(());
        };
        validate_capsule("activation candidate", &attempt.candidate)?;
        validate_attempt_facts(attempt)?;

        match activation.phase {
            ActivationPhase::Prepared
            | ActivationPhase::StoppingOld
            | ActivationPhase::OldStopped => {
                require_attempt_fact_level(attempt, AttemptFactLevel::Prepared)?;
            }
            ActivationPhase::SpawnPlanned => {
                require_attempt_fact_level(attempt, AttemptFactLevel::SpawnPlanned)?;
            }
            ActivationPhase::RuntimeStarted | ActivationPhase::AwaitingReady => {
                require_attempt_fact_level(attempt, AttemptFactLevel::PayloadRegistered)?;
            }
            ActivationPhase::Observing
            | ActivationPhase::CommitDecided
            | ActivationPhase::CommitRelaunch
            | ActivationPhase::CommitCleanup => {
                require_attempt_fact_level(attempt, AttemptFactLevel::Observing)?;
            }
            ActivationPhase::RollbackDecided
            | ActivationPhase::StoppingCandidate
            | ActivationPhase::CandidateStopped
            | ActivationPhase::Restoring
            | ActivationPhase::EvidencePending
            | ActivationPhase::EvidenceCleanup => {}
            ActivationPhase::Idle => unreachable!(),
        }

        if let Some(winner) = &activation.winner {
            let expected = match activation.phase {
                ActivationPhase::CommitDecided
                | ActivationPhase::CommitRelaunch
                | ActivationPhase::CommitCleanup => {
                    SelectedRuntime::external(attempt.candidate.clone())
                }
                ActivationPhase::RollbackDecided
                | ActivationPhase::StoppingCandidate
                | ActivationPhase::CandidateStopped
                | ActivationPhase::Restoring
                | ActivationPhase::EvidencePending
                | ActivationPhase::EvidenceCleanup => attempt.previous.clone(),
                _ => winner.selected.clone(),
            };
            if winner.selected != expected {
                return Err(LauncherError::Conflict(
                    "activation winner does not match the phase decision".to_string(),
                ));
            }
        }
        if let Some(receipt) = &activation.receipt {
            if receipt.attempt_id != attempt.attempt_id {
                return Err(LauncherError::Conflict(
                    "activation receipt belongs to another attempt".to_string(),
                ));
            }
            if receipt.candidate_release_id != attempt.candidate.release_id {
                return Err(LauncherError::Conflict(
                    "activation receipt candidate release does not match its attempt".to_string(),
                ));
            }
            let expected_outcome = match activation.phase {
                ActivationPhase::CommitDecided
                | ActivationPhase::CommitRelaunch
                | ActivationPhase::CommitCleanup => ActivationOutcome::Committed,
                ActivationPhase::Restoring
                | ActivationPhase::EvidencePending
                | ActivationPhase::EvidenceCleanup => ActivationOutcome::RolledBack,
                _ => receipt.outcome,
            };
            if receipt.outcome != expected_outcome {
                return Err(LauncherError::Conflict(
                    "activation receipt outcome does not match the phase".to_string(),
                ));
            }
            if let Some(winner) = &activation.winner
                && receipt.selected != winner.selected
            {
                return Err(LauncherError::Conflict(
                    "activation receipt does not select the decided winner".to_string(),
                ));
            }
        }
        Ok(())
    }
}

fn validate_cleanup_disjoint(state: &ControlState) -> Result<()> {
    let retained = [
        Some(&state.trusted_seed.capsule),
        state.external_current.as_ref(),
        state.external_previous.as_ref(),
        Some(state.selected.capsule()),
    ];
    let mut pending_release_ids = std::collections::BTreeSet::new();
    for pending in &state.activation.cleanup.pending {
        if !pending_release_ids.insert(&pending.release_id) {
            return Err(LauncherError::Conflict(format!(
                "cleanup capsule {} is listed more than once",
                pending.release_id
            )));
        }
        if retained
            .iter()
            .flatten()
            .any(|capsule| capsule.release_id == pending.release_id)
        {
            return Err(LauncherError::Conflict(format!(
                "cleanup capsule {} is still retained by control state",
                pending.release_id
            )));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttemptFactLevel {
    Prepared,
    SpawnPlanned,
    PayloadRegistered,
    Observing,
}

fn validate_attempt_facts(attempt: &AttemptRecord) -> Result<()> {
    if attempt.attempt_id.trim().is_empty() {
        return Err(LauncherError::Conflict(
            "activation attempt id must not be empty".to_string(),
        ));
    }
    if attempt.launch_instance_id.is_some() != attempt.spawn_attempt_id.is_some() {
        return Err(LauncherError::Conflict(
            "launchInstanceId and spawnAttemptId must appear together".to_string(),
        ));
    }
    if attempt.payload_registration.is_some()
        && (attempt.launch_instance_id.is_none() || attempt.spawn_attempt_id.is_none())
    {
        return Err(LauncherError::Conflict(
            "payload registration requires launch and spawn identities".to_string(),
        ));
    }
    if attempt.observation_deadline_unix_ms.is_some() && attempt.ready_expectation.is_none() {
        return Err(LauncherError::Conflict(
            "observation deadline requires a ready expectation".to_string(),
        ));
    }
    if !attempt.known_descendants.is_empty() && attempt.payload_registration.is_none() {
        return Err(LauncherError::Conflict(
            "known descendants require a payload registration".to_string(),
        ));
    }
    if attempt
        .previous_external_current
        .as_ref()
        .map(|capsule| &capsule.release_id)
        == attempt
            .previous_external_previous
            .as_ref()
            .map(|capsule| &capsule.release_id)
        && attempt.previous_external_current.is_some()
    {
        return Err(LauncherError::Conflict(
            "activation generation snapshot contains duplicate releases".to_string(),
        ));
    }
    if let SelectedRuntime::External { capsule } = &attempt.previous
        && attempt.previous_external_current.as_ref() != Some(capsule)
    {
        return Err(LauncherError::Conflict(
            "activation previous external selection does not match its generation snapshot"
                .to_string(),
        ));
    }

    let launch_instance_id = attempt.launch_instance_id.as_deref();
    let spawn_attempt_id = attempt.spawn_attempt_id.as_deref();
    if let Some(payload) = &attempt.payload_registration {
        if payload.release_id != attempt.candidate.release_id
            || Some(payload.launch_instance_id.as_str()) != launch_instance_id
            || Some(payload.spawn_attempt_id.as_str()) != spawn_attempt_id
            || payload.process_group.leader != payload.payload
            || payload.process_group.pgid != payload.payload.pid
        {
            return Err(LauncherError::Conflict(
                "payload registration does not match its activation attempt".to_string(),
            ));
        }
    }
    if let Some(expectation) = &attempt.ready_expectation {
        let payload = attempt
            .payload_registration
            .as_ref()
            .expect("checked above");
        if expectation.release_id != attempt.candidate.release_id
            || Some(expectation.launch_instance_id.as_str()) != launch_instance_id
            || Some(expectation.spawn_attempt_id.as_str()) != spawn_attempt_id
            || expectation.payload != payload.payload
            || expectation.token_verifier.trim().is_empty()
        {
            return Err(LauncherError::Conflict(
                "ready expectation does not match its activation attempt".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_failure_projection(
    failure: &FailureProjection,
    attempt: Option<&AttemptRecord>,
) -> Result<()> {
    if failure.activation_id.trim().is_empty()
        || failure.release_id.trim().is_empty()
        || failure.occurred_at.trim().is_empty()
        || failure.code.trim().is_empty()
        || failure.message.trim().is_empty()
    {
        return Err(LauncherError::Conflict(
            "failure projection requires activationId, releaseId, occurredAt, code, and message"
                .to_string(),
        ));
    }
    if let Some(attempt) = attempt
        && (failure.activation_id != attempt.attempt_id
            || failure.release_id != attempt.candidate.release_id)
    {
        return Err(LauncherError::Conflict(
            "failure projection does not match its activation attempt".to_string(),
        ));
    }
    if let Some(failed) = &failure.failed
        && failed.release_id != failure.release_id
    {
        return Err(LauncherError::Conflict(
            "failure projection releaseId does not match failed capsule".to_string(),
        ));
    }
    if let (Some(fallback_release_id), Some(fallback)) =
        (&failure.fallback_release_id, &failure.fallback)
        && fallback_release_id != &fallback.capsule().release_id
    {
        return Err(LauncherError::Conflict(
            "failure projection fallbackReleaseId does not match fallback runtime".to_string(),
        ));
    }
    Ok(())
}

fn validate_active_launch(active: &ActiveLaunchRecord) -> Result<()> {
    if active.launch_instance_id.trim().is_empty() || active.spawn_attempt_id.trim().is_empty() {
        return Err(LauncherError::Conflict(
            "active launch requires launch and spawn identities".to_string(),
        ));
    }
    let facts = (
        active.payload_registration.is_some(),
        active.ready_expectation.is_some(),
    );
    let expected = match active.phase {
        ActiveLaunchPhase::SpawnPlanned => (false, false),
        ActiveLaunchPhase::Running => (true, true),
    };
    if facts != expected {
        return Err(LauncherError::Conflict(format!(
            "active launch facts do not match {:?}: expected {:?}, found {:?}",
            active.phase, expected, facts
        )));
    }
    if active.phase == ActiveLaunchPhase::SpawnPlanned && !active.known_descendants.is_empty() {
        return Err(LauncherError::Conflict(
            "a spawn-planned launch must not retain observed descendants".to_string(),
        ));
    }
    if let Some(payload) = &active.payload_registration {
        if payload.release_id != active.selected.capsule().release_id
            || payload.launch_instance_id != active.launch_instance_id
            || payload.spawn_attempt_id != active.spawn_attempt_id
            || payload.process_group.leader != payload.payload
            || payload.process_group.pgid != payload.payload.pid
            || active
                .known_descendants
                .iter()
                .any(|identity| *identity == payload.payload)
        {
            return Err(LauncherError::Conflict(
                "active launch payload registration does not match selection".to_string(),
            ));
        }
    }
    if let Some(expectation) = &active.ready_expectation {
        let payload = active
            .payload_registration
            .as_ref()
            .expect("fact prefix checked");
        if expectation.release_id != active.selected.capsule().release_id
            || expectation.launch_instance_id != active.launch_instance_id
            || expectation.spawn_attempt_id != active.spawn_attempt_id
            || expectation.payload != payload.payload
        {
            return Err(LauncherError::Conflict(
                "active launch readiness does not match selection".to_string(),
            ));
        }
    }
    Ok(())
}

fn require_attempt_fact_level(attempt: &AttemptRecord, level: AttemptFactLevel) -> Result<()> {
    let expected = match level {
        AttemptFactLevel::Prepared => (false, false, false, false),
        AttemptFactLevel::SpawnPlanned => (true, false, false, false),
        AttemptFactLevel::PayloadRegistered => (true, true, true, false),
        AttemptFactLevel::Observing => (true, true, true, true),
    };
    let actual = (
        attempt.launch_instance_id.is_some(),
        attempt.payload_registration.is_some(),
        attempt.ready_expectation.is_some(),
        attempt.observation_deadline_unix_ms.is_some(),
    );
    if actual != expected {
        return Err(LauncherError::Conflict(format!(
            "activation attempt facts do not match {:?}: expected {:?}, found {:?}",
            level, expected, actual
        )));
    }
    Ok(())
}

pub struct StateLock {
    #[cfg(unix)]
    _file: File,
    #[cfg(not(unix))]
    path: PathBuf,
}

impl StateLock {
    pub fn acquire(paths: &ControlPaths) -> Result<Self> {
        paths.ensure()?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&paths.state_lock)
                .map_err(|error| io_error(format!("open {}", paths.state_lock.display()), error))?;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result != 0 {
                return Err(io_error(
                    format!("lock {}", paths.state_lock.display()),
                    std::io::Error::last_os_error(),
                ));
            }
            Ok(Self { _file: file })
        }
        #[cfg(not(unix))]
        {
            match std::fs::create_dir(&paths.state_lock) {
                Ok(()) => Ok(Self {
                    path: paths.state_lock.clone(),
                }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(
                    LauncherError::Conflict("another control state mutation is active".to_string()),
                ),
                Err(error) => Err(io_error(
                    format!("create {}", paths.state_lock.display()),
                    error,
                )),
            }
        }
    }
}

#[cfg(not(unix))]
impl Drop for StateLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutorEpoch {
    pub revision: u64,
    pub executor_epoch: u64,
}

impl ExecutorEpoch {
    pub fn acquire_and_bump_epoch(paths: &ControlPaths, trusted_seed: TrustedSeed) -> Result<Self> {
        let _lock = StateLock::acquire(paths)?;
        let mut state =
            load_unlocked(paths)?.unwrap_or_else(|| ControlState::initialize(trusted_seed));
        state.executor_epoch = state
            .executor_epoch
            .checked_add(1)
            .ok_or_else(|| LauncherError::Conflict("executor epoch exhausted".to_string()))?;
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| LauncherError::Conflict("control revision exhausted".to_string()))?;
        state.validate()?;
        write_json_atomic(&paths.control, &state)?;
        Ok(Self {
            revision: state.revision,
            executor_epoch: state.executor_epoch,
        })
    }
}

pub fn cas_update<F>(
    paths: &ControlPaths,
    expected_revision: u64,
    expected_epoch: u64,
    update: F,
) -> Result<ControlState>
where
    F: FnOnce(&mut ControlState) -> Result<()>,
{
    let _lock = StateLock::acquire(paths)?;
    let mut state = load_unlocked(paths)?
        .ok_or_else(|| LauncherError::Conflict("control state is not initialized".to_string()))?;
    if state.revision != expected_revision || state.executor_epoch != expected_epoch {
        return Err(LauncherError::Conflict(format!(
            "control CAS mismatch: expected revision {expected_revision} epoch \
             {expected_epoch}, found revision {} epoch {}",
            state.revision, state.executor_epoch
        )));
    }
    update(&mut state)?;
    if state.executor_epoch != expected_epoch {
        return Err(LauncherError::Conflict(
            "control CAS update must not mutate executor epoch".to_string(),
        ));
    }
    if state.revision != expected_revision {
        return Err(LauncherError::Conflict(
            "control CAS update must not mutate revision".to_string(),
        ));
    }
    state.revision = state
        .revision
        .checked_add(1)
        .ok_or_else(|| LauncherError::Conflict("control revision exhausted".to_string()))?;
    state.validate()?;
    write_json_atomic(&paths.control, &state)?;
    Ok(state)
}

fn load_unlocked(paths: &ControlPaths) -> Result<Option<ControlState>> {
    let Some(mut value) = read_json_if_exists::<serde_json::Value>(&paths.control)? else {
        return Ok(None);
    };
    let schema_version = value
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| {
            LauncherError::Conflict("control state has no valid schemaVersion".to_string())
        })?;
    if schema_version == 1 {
        migrate_guard_control_shape(&mut value);
    } else if schema_version != CONTROL_SCHEMA_VERSION {
        return Err(LauncherError::Conflict(format!(
            "unsupported control schema {schema_version}"
        )));
    }
    let state = serde_json::from_value::<ControlState>(value)
        .map_err(|error| json_error(format!("parse {}", paths.control.display()), error))?;
    state.validate()?;
    Ok(Some(state))
}

fn migrate_guard_control_shape(value: &mut serde_json::Value) {
    let Some(root) = value.as_object_mut() else {
        return;
    };
    root.insert(
        "schemaVersion".to_string(),
        serde_json::Value::from(CONTROL_SCHEMA_VERSION),
    );
    if let Some(active) = root
        .get_mut("activeLaunch")
        .and_then(serde_json::Value::as_object_mut)
    {
        active.remove("guardRegistration");
        active.remove("startAuthorization");
        if active.get("phase").and_then(serde_json::Value::as_str) == Some("guard_registered") {
            active.insert(
                "phase".to_string(),
                serde_json::Value::from("spawn_planned"),
            );
        } else if active.get("phase").and_then(serde_json::Value::as_str)
            == Some("start_authorized")
        {
            active.insert("phase".to_string(), serde_json::Value::from("running"));
        }
        if let Some(payload) = active
            .get_mut("payloadRegistration")
            .and_then(serde_json::Value::as_object_mut)
        {
            payload.remove("protocolVersion");
            payload.remove("guardRegistrationDigest");
        }
    }
    if let Some(activation) = root
        .get_mut("activation")
        .and_then(serde_json::Value::as_object_mut)
    {
        activation.remove("blocked");
        if activation.get("phase").and_then(serde_json::Value::as_str) == Some("blocked") {
            activation.insert("phase".to_string(), serde_json::Value::from("idle"));
            activation.remove("attempt");
            if let Some(receipt) = activation
                .get_mut("receipt")
                .and_then(serde_json::Value::as_object_mut)
                && receipt.get("outcome").and_then(serde_json::Value::as_str) == Some("blocked")
            {
                receipt.insert(
                    "outcome".to_string(),
                    serde_json::Value::from("rolled_back"),
                );
            }
        }
        if let Some(attempt) = activation
            .get_mut("attempt")
            .and_then(serde_json::Value::as_object_mut)
        {
            attempt.remove("guardRegistration");
            attempt.remove("startAuthorization");
            if let Some(payload) = attempt
                .get_mut("payloadRegistration")
                .and_then(serde_json::Value::as_object_mut)
            {
                payload.remove("protocolVersion");
                payload.remove("guardRegistrationDigest");
            }
        }
        if activation.get("phase").and_then(serde_json::Value::as_str) == Some("guard_registered") {
            activation.insert(
                "phase".to_string(),
                serde_json::Value::from("spawn_planned"),
            );
        } else if activation.get("phase").and_then(serde_json::Value::as_str)
            == Some("start_authorized")
        {
            activation.insert(
                "phase".to_string(),
                serde_json::Value::from("runtime_started"),
            );
        }
    }
}

fn validate_capsule(label: &str, capsule: &CapsuleRef) -> Result<()> {
    if capsule.release_id.trim().is_empty() {
        return Err(LauncherError::Conflict(format!(
            "{label} release id must not be empty"
        )));
    }
    if !capsule.root.is_absolute() || !capsule.entrypoint.is_absolute() {
        return Err(LauncherError::Conflict(format!(
            "{label} paths must be absolute"
        )));
    }
    if !capsule.entrypoint.starts_with(&capsule.root) {
        return Err(LauncherError::Conflict(format!(
            "{label} entrypoint must be inside its capsule root"
        )));
    }
    Ok(())
}

pub(crate) fn read_json_if_exists<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(LauncherError::Conflict(format!(
                "control file must not be a symlink: {}",
                path.display()
            )));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(LauncherError::Conflict(format!(
                "control path is not a regular file: {}",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(format!("inspect {}", path.display()), error)),
    }
    let bytes =
        std::fs::read(path).map_err(|error| io_error(format!("read {}", path.display()), error))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| json_error(format!("parse {}", path.display()), error))
}

pub(crate) fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        LauncherError::InvalidRequest(format!("{} has no parent directory", path.display()))
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|error| io_error(format!("create {}", parent.display()), error))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            LauncherError::InvalidRequest(format!("invalid control path {}", path.display()))
        })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nonce));
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| json_error("serialize control state", error))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| io_error(format!("create {}", temporary.display()), error))?;
    let result = (|| {
        file.write_all(&bytes)
            .map_err(|error| io_error(format!("write {}", temporary.display()), error))?;
        file.write_all(b"\n")
            .map_err(|error| io_error(format!("write {}", temporary.display()), error))?;
        file.sync_all()
            .map_err(|error| io_error(format!("sync {}", temporary.display()), error))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| io_error(format!("replace {}", path.display()), error))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| io_error(format!("sync {}", parent.display()), error))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capsule(id: &str) -> CapsuleRef {
        let root = std::env::temp_dir().join("runtime-control-tests").join(id);
        CapsuleRef {
            release_id: format!("release-{id}"),
            entrypoint: root.join("bin/toy-runtime"),
            root,
            metadata: serde_json::Value::Null,
        }
    }

    fn trusted_seed(id: &str) -> TrustedSeed {
        TrustedSeed {
            capsule: capsule(id),
            trust_anchor: std::env::temp_dir().join("runtime-control-tests/anchor"),
            metadata: serde_json::Value::Null,
        }
    }

    fn prepared_state() -> ControlState {
        let seed = trusted_seed("seed");
        let mut state = ControlState::initialize(seed);
        state.activation.phase = ActivationPhase::Prepared;
        state.activation.attempt = Some(AttemptRecord {
            attempt_id: "activation-1".to_string(),
            candidate: capsule("candidate"),
            previous: state.selected.clone(),
            previous_external_current: None,
            previous_external_previous: None,
            started_at_unix_ms: 1,
            launch_instance_id: None,
            spawn_attempt_id: None,
            payload_registration: None,
            ready_expectation: None,
            known_descendants: Vec::new(),
            observation_deadline_unix_ms: None,
            annotations: BTreeMap::new(),
        });
        state
    }

    #[test]
    fn failure_projection_matches_electron_recovery_shape() {
        let failed = capsule("failed");
        let fallback = capsule("fallback");
        let projection = FailureProjection {
            activation_id: "activation-123".to_string(),
            release_id: failed.release_id.clone(),
            occurred_at: "2026-09-09T08:07:06.005Z".to_string(),
            fallback_release_id: Some(fallback.release_id.clone()),
            code: "activation_rolled_back".to_string(),
            message: "candidate exited before readiness".to_string(),
            failed: Some(failed.clone()),
            fallback: Some(SelectedRuntime::external(fallback.clone())),
            evidence_path: None,
            details: BTreeMap::new(),
        };

        let value = serde_json::to_value(&projection).expect("serialize failure projection");
        assert_eq!(value["activationId"], "activation-123");
        assert_eq!(value["releaseId"], failed.release_id);
        assert_eq!(value["occurredAt"], "2026-09-09T08:07:06.005Z");
        assert_eq!(value["fallbackReleaseId"], fallback.release_id);
        assert_eq!(value["code"], "activation_rolled_back");
        assert_eq!(value["message"], "candidate exited before readiness");
        assert_eq!(value["failed"]["releaseId"], "release-failed");
        assert_eq!(value["fallback"]["kind"], "external");
        assert!(value.get("evidencePath").is_none());
        assert!(value.get("details").is_none());

        let without_fallback = FailureProjection {
            fallback_release_id: None,
            fallback: None,
            ..projection
        };
        let value = serde_json::to_value(&without_fallback).expect("serialize without fallback");
        assert!(value.get("fallbackReleaseId").is_none());
        assert!(value.get("fallback").is_none());
    }

    #[test]
    fn activation_phase_requires_payload_identity_before_runtime_started() {
        let mut state = prepared_state();
        state.activation.phase = ActivationPhase::RuntimeStarted;
        let error = state
            .validate()
            .expect_err("runtime start needs payload facts");
        assert!(error.to_string().contains("PayloadRegistered"));

        let attempt = state.activation.attempt.as_mut().expect("attempt");
        attempt.launch_instance_id = Some("launch-1".to_string());
        attempt.spawn_attempt_id = Some("spawn-1".to_string());
        let payload = ProcessIdentity {
            pid: 101,
            start_identity: 1001,
        };
        attempt.payload_registration = Some(PayloadRegistration {
            release_id: attempt.candidate.release_id.clone(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            payload,
            process_group: crate::process::ProcessGroupRecord {
                leader: payload,
                pgid: payload.pid,
            },
        });
        attempt.ready_expectation = Some(ReadyExpectation {
            protocol_version: crate::readiness::READY_PROTOCOL_VERSION,
            release_id: attempt.candidate.release_id.clone(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            payload,
            token_verifier: "a".repeat(64),
        });
        state.validate().expect("direct payload facts are complete");
    }

    #[test]
    fn activation_attempt_rejects_payload_with_wrong_group() {
        let mut state = prepared_state();
        state.activation.phase = ActivationPhase::RollbackDecided;
        let attempt = state.activation.attempt.as_mut().expect("attempt");
        attempt.launch_instance_id = Some("launch-1".to_string());
        attempt.spawn_attempt_id = Some("spawn-1".to_string());
        let payload = ProcessIdentity {
            pid: 102,
            start_identity: 1002,
        };
        attempt.payload_registration = Some(PayloadRegistration {
            release_id: attempt.candidate.release_id.clone(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            payload,
            process_group: crate::process::ProcessGroupRecord {
                leader: payload,
                pgid: payload.pid + 1,
            },
        });
        state.activation.winner = Some(WinnerRecord {
            selected: attempt.previous.clone(),
            decided_at_unix_ms: 2,
            reason: "test rollback".to_string(),
        });

        let error = state
            .validate()
            .expect_err("payload needs a dedicated process group");
        assert!(
            error
                .to_string()
                .contains("payload registration does not match")
        );
    }

    #[test]
    fn cleanup_cannot_overlap_any_retained_capsule() {
        let mut state = ControlState::initialize(trusted_seed("seed"));
        let current = capsule("current");
        let previous = capsule("previous");
        state.external_current = Some(current.clone());
        state.external_previous = Some(previous);
        state.selected = SelectedRuntime::external(current.clone());
        state.activation.cleanup.pending = vec![current];

        let error = state.validate().expect_err("retained cleanup rejected");
        assert!(error.to_string().contains("still retained"));
    }

    #[test]
    fn cas_rejects_stale_revision_and_epoch_without_writing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ControlPaths::new(temp.path().join("state"));
        let lease =
            ExecutorEpoch::acquire_and_bump_epoch(&paths, trusted_seed("seed")).expect("lease");
        let updated = cas_update(&paths, lease.revision, lease.executor_epoch, |state| {
            let external = capsule("external");
            state.external_current = Some(external.clone());
            state.selected = SelectedRuntime::external(external);
            Ok(())
        })
        .expect("CAS update");
        let stale_revision = cas_update(&paths, lease.revision, lease.executor_epoch, |_| Ok(()))
            .expect_err("stale revision");
        assert!(stale_revision.to_string().contains("CAS mismatch"));
        let stale_epoch = cas_update(&paths, updated.revision, lease.executor_epoch + 1, |_| {
            Ok(())
        })
        .expect_err("stale epoch");
        assert!(stale_epoch.to_string().contains("CAS mismatch"));
        assert_eq!(
            ControlState::load(&paths).expect("load").expect("state"),
            updated
        );
    }

    #[test]
    fn acquiring_a_new_executor_fences_the_old_epoch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ControlPaths::new(temp.path().join("state"));
        let first =
            ExecutorEpoch::acquire_and_bump_epoch(&paths, trusted_seed("seed")).expect("first");
        let second =
            ExecutorEpoch::acquire_and_bump_epoch(&paths, trusted_seed("ignored")).expect("second");
        assert_eq!(second.executor_epoch, first.executor_epoch + 1);
        let error = cas_update(&paths, second.revision, first.executor_epoch, |_| Ok(()))
            .expect_err("old executor fenced");
        assert!(error.to_string().contains("CAS mismatch"));
        let state = ControlState::load(&paths)
            .expect("load")
            .expect("control state");
        assert_eq!(state.trusted_seed, trusted_seed("seed"));
    }
}
