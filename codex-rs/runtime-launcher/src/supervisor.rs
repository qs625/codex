use crate::LauncherError;
use crate::MutationRequest;
use crate::PrepareActivationRequest;
use crate::Result;
use crate::capsule::CapsuleRecord;
use crate::capsule::CapsuleTarget;
use crate::capsule::import_incoming;
use crate::capsule::load_and_verify_capsule;
use crate::capsule::remove_external_capsule;
use crate::capsule::verify_record_for_spawn;
use crate::control::ActivationOutcome;
use crate::control::ActivationPhase;
use crate::control::ActivationReceipt;
use crate::control::ActiveLaunchPhase;
use crate::control::ActiveLaunchRecord;
use crate::control::AttemptRecord;
use crate::control::BlockedRecord;
use crate::control::CapsuleRef;
use crate::control::CleanupRecord;
use crate::control::ControlPaths;
use crate::control::ControlState;
use crate::control::ExecutorEpoch;
use crate::control::FailureProjection;
use crate::control::SelectedRuntime;
use crate::control::WinnerRecord;
use crate::control::cas_update;
use crate::guard;
use crate::guard::GuardAcknowledgement;
use crate::guard::GuardExecReport;
use crate::guard::GuardExitReport;
use crate::guard::CleanupEvidence;
use crate::guard::GuardLaunchRequest;
use crate::guard::GuardRegistration;
use crate::guard::PayloadRegistration;
use crate::guard::ReadyBearer;
use crate::guard::StartAuthorization;
use crate::migration::migrate_legacy_state;
use crate::process::ProcessIdentity;
use crate::process::TerminationPolicy;
use crate::process::TerminationTarget;
use crate::process::terminate_and_observe_empty;
use crate::process::terminate_identity_and_observe_empty;
use crate::process::terminate_identity_tree_and_observe_empty;
use crate::seed::discover_seed;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;
use std::time::Instant;

pub const EXIT_COORDINATED_RESTART: i32 = 75;
const REQUEST_SCHEMA_VERSION: u32 = 1;
const OBSERVATION_WINDOW: Duration = Duration::from_secs(30);
const GUARD_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug)]
pub struct LauncherPaths {
    pub root: PathBuf,
    pub control: ControlPaths,
    pub executor_lock: PathBuf,
    pub attempts: PathBuf,
    pub failure_evidence: PathBuf,
}

impl LauncherPaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            control: ControlPaths::new(root.clone()),
            executor_lock: root.join("executor.lock"),
            attempts: root.join("attempts"),
            failure_evidence: root.join("failure-evidence.json"),
            root,
        }
    }

    fn ensure(&self) -> Result<()> {
        self.control.ensure()?;
        std::fs::create_dir_all(&self.attempts)
            .map_err(|error| crate::io_error(format!("create {}", self.attempts.display()), error))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    Exited(i32),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub control: ControlState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<FailureProjection>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrepareActivationDisposition {
    Prepared,
    AlreadyPrepared,
    AlreadyCommitted,
    TerminalFailed,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareActivationResult {
    pub disposition: PrepareActivationDisposition,
    pub activation_id: String,
    pub release_id: String,
    pub control: ControlState,
}

struct LaunchOutcome {
    status: ExitStatus,
    ready: bool,
    committed: bool,
}

pub fn prepare_activation(
    paths: &LauncherPaths,
    request: PrepareActivationRequest,
) -> Result<PrepareActivationResult> {
    validate_prepare(&request)?;
    let requested_activation_id = request.activation_id.clone();
    let requested_release_id = request.release_id.clone();
    let existing = ControlState::load(&paths.control)?.ok_or_else(|| {
        LauncherError::Conflict("run must initialize the trusted Seed before prepare".to_string())
    })?;
    paths.ensure()?;
    if let Some(attempt) = &existing.activation.attempt
        && attempt.attempt_id == request.activation_id
    {
        if attempt.candidate.release_id == request.release_id {
            let disposition = if existing.activation.phase == ActivationPhase::Blocked
                || existing.activation.receipt.as_ref().is_some_and(|receipt| {
                    matches!(
                        receipt.outcome,
                        ActivationOutcome::RolledBack | ActivationOutcome::Blocked
                    )
                })
            {
                PrepareActivationDisposition::TerminalFailed
            } else if matches!(
                existing.activation.phase,
                ActivationPhase::CommitDecided
                    | ActivationPhase::CommitRelaunch
                    | ActivationPhase::CommitCleanup
            ) || existing
                .activation
                .receipt
                .as_ref()
                .is_some_and(|receipt| receipt.outcome == ActivationOutcome::Committed)
            {
                PrepareActivationDisposition::AlreadyCommitted
            } else if existing.activation.phase == ActivationPhase::Prepared {
                PrepareActivationDisposition::AlreadyPrepared
            } else {
                return Err(LauncherError::Conflict(
                    "matching activation is already in progress".to_string(),
                ));
            };
            return Ok(PrepareActivationResult {
                disposition,
                activation_id: requested_activation_id,
                release_id: requested_release_id,
                control: existing,
            });
        }
        return Err(LauncherError::Conflict(
            "activationId is already bound to another release".to_string(),
        ));
    }
    if let Some(receipt) = &existing.activation.receipt
        && receipt.attempt_id == request.activation_id
    {
        if receipt.candidate_release_id == request.release_id {
            let disposition = if existing.activation.phase == ActivationPhase::Blocked
                || matches!(
                    receipt.outcome,
                    ActivationOutcome::RolledBack | ActivationOutcome::Blocked
                )
            {
                PrepareActivationDisposition::TerminalFailed
            } else {
                PrepareActivationDisposition::AlreadyCommitted
            };
            return Ok(PrepareActivationResult {
                disposition,
                activation_id: requested_activation_id,
                release_id: requested_release_id,
                control: existing,
            });
        }
        return Err(LauncherError::Conflict(
            "activationId terminal receipt is bound to another release".to_string(),
        ));
    }
    if existing.revision != request.expected_revision
        || existing.executor_epoch != request.expected_executor_epoch
    {
        return Err(LauncherError::Conflict(
            "prepare control revision or executor epoch is stale".to_string(),
        ));
    }
    if existing.activation.phase != ActivationPhase::Idle {
        return Err(LauncherError::Conflict(
            "prepare requires an idle activation".to_string(),
        ));
    }
    let candidate = import_incoming(&paths.root, &request.activation_id, &request.target)?;
    if candidate.release_id != request.release_id {
        return Err(LauncherError::InvalidRequest(
            "requested releaseId does not match imported capsule".to_string(),
        ));
    }
    let candidate_ref = capsule_ref(&candidate);
    let result = cas_update(
        &paths.control,
        request.expected_revision,
        request.expected_executor_epoch,
        |state| {
            if state.activation.phase != ActivationPhase::Idle {
                return Err(LauncherError::Conflict(
                    "prepare requires an idle or terminal activation".to_string(),
                ));
            }
            state.activation.phase = ActivationPhase::Prepared;
            state.activation.attempt = Some(AttemptRecord {
                attempt_id: requested_activation_id.clone(),
                candidate: candidate_ref,
                previous: state.selected.clone(),
                previous_external_current: state.external_current.clone(),
                previous_external_previous: state.external_previous.clone(),
                started_at_unix_ms: unix_time_ms(),
                launch_instance_id: None,
                spawn_attempt_id: None,
                guard_registration: None,
                payload_registration: None,
                start_authorization: None,
                ready_expectation: None,
                known_descendants: Vec::new(),
                observation_deadline_unix_ms: None,
                annotations: BTreeMap::from([("reason".to_string(), request.reason)]),
            });
            state.activation.winner = None;
            state.activation.cleanup = CleanupRecord::default();
            state.activation.receipt = None;
            state.activation.blocked = None;
            Ok(())
        },
    );
    // Published capsules are content addressed and may concurrently be about to
    // become referenced by another activation with the same digest. A failed
    // bind must retain the artifact for later safe garbage collection.
    result.map(|control| PrepareActivationResult {
        disposition: PrepareActivationDisposition::Prepared,
        activation_id: requested_activation_id,
        release_id: requested_release_id,
        control,
    })
}

pub fn cancel_activation(paths: &LauncherPaths, request: MutationRequest) -> Result<ControlState> {
    if let Some(state) = replay_rolled_back_mutation(paths, &request)? {
        return Ok(state);
    }
    let cancelled = mutate_attempt(paths, request, |state, attempt, reason| {
        if state.activation.phase != ActivationPhase::Prepared {
            return Err(LauncherError::Conflict(
                "only a prepared activation can be cancelled".to_string(),
            ));
        }
        state.activation.receipt = Some(ActivationReceipt {
            attempt_id: attempt.attempt_id.clone(),
            candidate_release_id: attempt.candidate.release_id.clone(),
            outcome: ActivationOutcome::RolledBack,
            selected: state.selected.clone(),
            completed_at_unix_ms: unix_time_ms(),
            reason: Some(if reason.is_empty() {
                "activation cancelled".to_string()
            } else {
                reason
            }),
        });
        state.activation.phase = ActivationPhase::Idle;
        state.activation.cleanup.pending =
            cleanup_candidates_if_unretained(state, [attempt.candidate.clone()]);
        state.activation.attempt = None;
        Ok(())
    })?;
    cleanup_pending_capsules(paths, &cancelled)?;
    cas_update(
        &paths.control,
        cancelled.revision,
        cancelled.executor_epoch,
        |state| {
            state.activation.cleanup.pending.clear();
            Ok(())
        },
    )
}

pub fn request_rollback(
    paths: &LauncherPaths,
    request: MutationRequest,
) -> Result<ControlState> {
    if let Some(state) = replay_rolled_back_mutation(paths, &request)? {
        return Ok(state);
    }
    mutate_attempt(paths, request, |state, attempt, reason| {
        if matches!(
            state.activation.phase,
            ActivationPhase::CommitDecided
                | ActivationPhase::CommitRelaunch
                | ActivationPhase::CommitCleanup
        ) || state
            .activation
            .receipt
            .as_ref()
            .is_some_and(|receipt| receipt.outcome == ActivationOutcome::Committed)
        {
            return Err(LauncherError::Conflict(
                "activation winner is already durably committed".to_string(),
            ));
        }
        if state.activation.phase == ActivationPhase::Blocked {
            return Err(LauncherError::Conflict(
                "blocked activation is already terminal".to_string(),
            ));
        }
        if matches!(
            state.activation.phase,
            ActivationPhase::EvidencePending
                | ActivationPhase::EvidenceCleanup
        ) {
            return Err(LauncherError::Conflict(
                "activation winner is already final".to_string(),
            ));
        }
        state.activation.phase = ActivationPhase::RollbackDecided;
        state.activation.receipt = None;
        state.activation.winner = Some(WinnerRecord {
            selected: attempt.previous.clone(),
            decided_at_unix_ms: unix_time_ms(),
            reason: if reason.is_empty() {
                "rollback requested".to_string()
            } else {
                reason
            },
        });
        Ok(())
    })
}

pub fn ack_failure(paths: &LauncherPaths, request: MutationRequest) -> Result<ControlState> {
    let mut before = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state is not initialized".to_string()))?;
    if before.revision != request.expected_revision
        || before.executor_epoch != request.expected_executor_epoch
    {
        return Err(LauncherError::Conflict(format!(
            "control CAS mismatch: expected revision {} epoch {}, found revision {} epoch {}",
            request.expected_revision,
            request.expected_executor_epoch,
            before.revision,
            before.executor_epoch
        )));
    }
    if before.activation.phase == ActivationPhase::Idle
        && before
            .activation
            .receipt
            .as_ref()
            .is_some_and(|receipt| receipt.attempt_id == request.activation_id)
        && !paths.failure_evidence.exists()
    {
        return Ok(before);
    }
    if !matches!(
        before.activation.phase,
        ActivationPhase::EvidencePending
            | ActivationPhase::EvidenceCleanup
            | ActivationPhase::Blocked
    ) {
        return Err(LauncherError::Conflict(
            "there is no failure evidence to acknowledge".to_string(),
        ));
    }
    let evidence_activation_id = before
        .activation
        .attempt
        .as_ref()
        .map(|attempt| attempt.attempt_id.as_str())
        .or_else(|| {
            before
                .activation
                .blocked
                .as_ref()
                .map(|blocked| blocked.failure.activation_id.as_str())
        });
    if evidence_activation_id != Some(request.activation_id.as_str()) {
        return Err(LauncherError::Conflict(
            "failure evidence belongs to another activation".to_string(),
        ));
    }
    if before.activation.phase == ActivationPhase::Blocked
        && before.active_launch.is_some()
    {
        return Err(LauncherError::Blocked(
            "blocked launch identities must be observed stopped before failure acknowledgement"
                .to_string(),
        ));
    }
    if before.activation.phase == ActivationPhase::Blocked {
        ensure_pending_failure_evidence(paths, &before)?;
    }
    if before.activation.phase == ActivationPhase::EvidencePending {
        before = cas_update(
            &paths.control,
            before.revision,
            before.executor_epoch,
            |state| {
                state.activation.phase = ActivationPhase::EvidenceCleanup;
                Ok(())
            },
        )?;
    }
    cleanup_pending_capsules(paths, &before)?;
    remove_failure_evidence_durably(&paths.failure_evidence)?;
    cas_update(
        &paths.control,
        before.revision,
        before.executor_epoch,
        |state| {
            if state.activation.receipt.is_none() {
                state.activation.receipt = Some(ActivationReceipt {
                    attempt_id: request.activation_id.clone(),
                    candidate_release_id: state
                        .activation
                        .attempt
                        .as_ref()
                        .map(|attempt| attempt.candidate.release_id.clone())
                        .or_else(|| {
                            state
                                .activation
                                .blocked
                                .as_ref()
                                .map(|blocked| blocked.failure.release_id.clone())
                        })
                        .unwrap_or_else(|| "unknown".to_string()),
                    outcome: ActivationOutcome::Blocked,
                    selected: state.selected.clone(),
                    completed_at_unix_ms: unix_time_ms(),
                    reason: Some("failure evidence acknowledged".to_string()),
                });
            }
            state.activation.phase = ActivationPhase::Idle;
            state.activation.blocked = None;
            state.activation.attempt = None;
            state.activation.cleanup.pending.clear();
            Ok(())
        },
    )
}

fn cleanup_pending_capsules(paths: &LauncherPaths, state: &ControlState) -> Result<()> {
    for capsule in &state.activation.cleanup.pending {
        match std::fs::symlink_metadata(&capsule.root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(crate::io_error(
                    format!("inspect {}", capsule.root.display()),
                    error,
                ));
            }
            Ok(_) => {}
        }
        let target = load_target(capsule)?;
        let record = record_from_ref(capsule, &target)?;
        remove_external_capsule(&paths.root, &record)?;
    }
    Ok(())
}

fn cleanup_candidates_if_unretained(
    state: &ControlState,
    candidates: impl IntoIterator<Item = CapsuleRef>,
) -> Vec<CapsuleRef> {
    let retained_release_ids = [
        Some(&state.trusted_seed.capsule),
        state.external_current.as_ref(),
        state.external_previous.as_ref(),
        Some(state.selected.capsule()),
    ]
    .into_iter()
    .flatten()
    .map(|capsule| capsule.release_id.as_str())
    .collect::<BTreeSet<_>>();
    candidates
        .into_iter()
        .filter(|capsule| !retained_release_ids.contains(capsule.release_id.as_str()))
        .collect()
}

fn remove_failure_evidence_durably(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(crate::io_error(
                format!("remove {}", path.display()),
                error,
            ));
        }
    }
    let parent = path.parent().ok_or_else(|| {
        LauncherError::InvalidRequest(format!("{} has no parent", path.display()))
    })?;
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| crate::io_error(format!("sync {}", parent.display()), error))
}

pub fn status(paths: &LauncherPaths) -> Result<Status> {
    let control = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state is not initialized".to_string()))?;
    ensure_pending_failure_evidence(paths, &control)?;
    let failure = crate::control::read_json_if_exists(&paths.failure_evidence)?
        .or_else(|| {
            control
                .activation
                .blocked
                .as_ref()
                .map(|blocked| blocked.failure.clone())
        });
    Ok(Status { control, failure })
}

pub fn run(
    paths: &LauncherPaths,
    outer_bundle: &Path,
    target: CapsuleTarget,
    launcher_path: &Path,
) -> Result<RunOutcome> {
    paths.ensure()?;
    let _executor = guard::ExecutorLease::acquire(&paths.executor_lock)
        .map_err(|error| LauncherError::Conflict(error.to_string()))?;
    let (seed, _) = discover_seed(outer_bundle, &target)?;
    migrate_legacy_state(&paths.control)?;
    ControlState::load_or_initialize(&paths.control, seed.clone())?;
    ExecutorEpoch::acquire_and_bump_epoch(&paths.control, seed.clone())?;
    let fenced = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
    let fenced = reconcile_active_launch(paths, fenced)?;
    let mut state = rebind_seed(paths, fenced.revision, fenced.executor_epoch, seed)?;
    reconcile_interrupted(paths, &target, &mut state)?;
    ensure_pending_failure_evidence(paths, &state)?;

    loop {
        state = ControlState::load(&paths.control)?.ok_or_else(|| {
            LauncherError::Conflict("control state disappeared".to_string())
        })?;
        if state.activation.phase == ActivationPhase::Blocked {
            return Err(LauncherError::Blocked(
                state
                    .activation
                    .blocked
                    .as_ref()
                    .map(|blocked| blocked.failure.message.clone())
                    .unwrap_or_else(|| "activation is blocked".to_string()),
            ));
        }
        if state.activation.phase == ActivationPhase::RollbackDecided {
            execute_rollback(paths, state)?;
            continue;
        }
        let selected = load_selected(&state, &target)?;
        let launching_candidate = state.activation.phase == ActivationPhase::SpawnPlanned
            && state
                .activation
                .attempt
                .as_ref()
                .is_some_and(|attempt| attempt.candidate.release_id == selected.release_id);
        let recovering_committed_candidate =
            state.activation.phase == ActivationPhase::CommitRelaunch;
        let outcome = match launch_selected(paths, &state, &selected, launcher_path) {
            Ok(outcome) => outcome,
            Err(error @ LauncherError::Blocked(_)) => {
                let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                    LauncherError::Conflict("control state disappeared".to_string())
                })?;
                reconcile_active_launch_with_block(paths, latest, &error.to_string())?;
                return Err(error);
            }
            Err(error) if launching_candidate => {
                let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                    LauncherError::Conflict("control state disappeared".to_string())
                })?;
                let latest = reconcile_active_launch(paths, latest)?;
                if latest.activation.phase == ActivationPhase::RollbackDecided {
                    execute_rollback(paths, latest)?;
                } else {
                    rollback_failed_candidate(paths, latest, error.to_string())?;
                }
                continue;
            }
            Err(error) if recovering_committed_candidate => {
                let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                    LauncherError::Conflict("control state disappeared".to_string())
                })?;
                reconcile_active_launch_with_block(
                    paths,
                    latest,
                    &format!("committed candidate failed recovery relaunch: {error}"),
                )?;
                return Err(LauncherError::Blocked(
                    "committed Runtime Capsule recovery relaunch is blocked".to_string(),
                ));
            }
            Err(error) => {
                let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                    LauncherError::Conflict("control state disappeared".to_string())
                })?;
                reconcile_active_launch(paths, latest)?;
                return Err(error);
            }
        };
        let code = outcome.status.code().unwrap_or(1);
        let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
            LauncherError::Conflict("control state disappeared".to_string())
        })?;
        if launching_candidate && (!outcome.ready || !outcome.committed) {
            rollback_failed_candidate(
                paths,
                latest,
                if outcome.ready {
                    "candidate exited during observation".to_string()
                } else {
                    format!("candidate exited before readiness with code {code}")
                },
            )?;
            continue;
        }
        if latest.activation.phase == ActivationPhase::Prepared
            && code == EXIT_COORDINATED_RESTART
        {
            authorize_candidate_after_old_stopped(paths, latest)?;
            continue;
        }
        return Ok(RunOutcome::Exited(code));
    }
}

fn reconcile_active_launch(
    paths: &LauncherPaths,
    state: ControlState,
) -> Result<ControlState> {
    reconcile_active_launch_inner(paths, state, None)
}

fn reconcile_active_launch_with_block(
    paths: &LauncherPaths,
    state: ControlState,
    message: &str,
) -> Result<ControlState> {
    reconcile_active_launch_inner(paths, state, Some(message))
}

fn reconcile_active_launch_inner(
    paths: &LauncherPaths,
    state: ControlState,
    pending_block_message: Option<&str>,
) -> Result<ControlState> {
    let Some(active) = state.active_launch.as_ref() else {
        if let Some(message) = pending_block_message {
            fence_blocked_launch(paths, &state, message)?;
            return Err(LauncherError::Blocked(message.to_string()));
        }
        return Ok(state);
    };
    let mut recovered_payload = active.payload_registration.clone();
    let cleanup_result = (|| -> Result<()> {
        match (
            active.guard_registration.as_ref(),
            active.payload_registration.as_ref(),
        ) {
            (Some(guard_registration), Some(payload_registration)) => {
                terminate_registered_launch(guard_registration, payload_registration)
            }
            (Some(guard_registration), None) => {
                let payload_path = active_payload_registration_path(paths, active);
                if let Some(payload_registration) =
                    guard::read_protocol_file_if_exists::<PayloadRegistration>(&payload_path)
                        .map_err(|error| LauncherError::Blocked(error.to_string()))?
                {
                    validate_recovered_payload_registration(
                        active,
                        guard_registration,
                        &payload_registration,
                    )?;
                    recovered_payload = Some(payload_registration.clone());
                    terminate_registered_launch(guard_registration, &payload_registration)
                } else if guard_registration
                    .guard
                    .is_alive()
                    .map_err(|error| LauncherError::Blocked(error.to_string()))?
                {
                    terminate_identity_tree_and_observe_empty(
                        guard_registration.guard,
                        TerminationPolicy::default(),
                    )
                    .map(|_| ())
                    .map_err(|error| LauncherError::Blocked(error.to_string()))
                } else {
                    Err(LauncherError::Blocked(
                        "guard exited before payload registration was durably recovered; cooperative cleanup cannot be confirmed"
                            .to_string(),
                    ))
                }
            }
            (None, None) => {
                let registration_path = paths
                    .attempts
                    .join(&active.spawn_attempt_id)
                    .with_extension("guard-registration.json");
                if let Some(registration) =
                    guard::read_protocol_file_if_exists::<GuardRegistration>(&registration_path)
                        .map_err(|error| LauncherError::Blocked(error.to_string()))?
                {
                    validate_recovered_guard_registration(active, &registration)?;
                    let payload_path = active_payload_registration_path(paths, active);
                    if let Some(payload_registration) =
                        guard::read_protocol_file_if_exists::<PayloadRegistration>(&payload_path)
                            .map_err(|error| LauncherError::Blocked(error.to_string()))?
                    {
                        validate_recovered_payload_registration(
                            active,
                            &registration,
                            &payload_registration,
                        )?;
                        recovered_payload = Some(payload_registration.clone());
                        terminate_registered_launch(&registration, &payload_registration)
                    } else if registration
                        .guard
                        .is_alive()
                        .map_err(|error| LauncherError::Blocked(error.to_string()))?
                    {
                        terminate_identity_tree_and_observe_empty(
                            registration.guard,
                            TerminationPolicy::default(),
                        )
                        .map(|_| ())
                        .map_err(|error| LauncherError::Blocked(error.to_string()))
                    } else {
                        Err(LauncherError::Blocked(
                            "recovered guard exited without a payload registration; cooperative cleanup cannot be confirmed"
                                .to_string(),
                        ))
                    }
                } else {
                    Err(LauncherError::Blocked(
                        "spawn was durably planned but no guard identity was recoverable"
                            .to_string(),
                    ))
                }
            }
            (None, Some(_)) => Err(LauncherError::Blocked(
                "active launch has payload identity without guard identity".to_string(),
            )),
        }
    })();
    let durable_violation = read_durable_guard_violation(
        paths,
        active,
        recovered_payload.as_ref(),
    );
    if let Err(error) = cleanup_result {
        let message = error.to_string();
        fence_blocked_launch(paths, &state, &message)?;
        return Err(LauncherError::Blocked(message));
    }
    let durable_violation = match durable_violation {
        Ok(violation) => violation,
        Err(error) => Some(error.to_string()),
    };
    let block_message = durable_violation.as_deref().or(pending_block_message);
    if let Some(message) = block_message {
        fence_blocked_launch_after_cleanup(
            paths,
            &state,
            &active.launch_instance_id,
            &active.spawn_attempt_id,
            message,
        )?;
        return Err(LauncherError::Blocked(message.to_string()));
    }
    let cleared = cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            next.active_launch = None;
            Ok(())
        },
    )?;
    ensure_pending_failure_evidence(paths, &cleared)?;
    Ok(cleared)
}

fn read_durable_guard_violation(
    paths: &LauncherPaths,
    active: &ActiveLaunchRecord,
    payload_registration: Option<&PayloadRegistration>,
) -> Result<Option<String>> {
    let exit_report_path = paths
        .attempts
        .join(&active.spawn_attempt_id)
        .with_extension("exit-report.json");
    let Some(report) = guard::read_protocol_file_if_exists::<GuardExitReport>(&exit_report_path)
        .map_err(|error| LauncherError::Blocked(error.to_string()))?
    else {
        return Ok(None);
    };
    let payload_registration = payload_registration.ok_or_else(|| {
        LauncherError::Blocked(
            "durable guard terminal report has no recoverable payload registration".to_string(),
        )
    })?;
    match validate_guard_exit_report(
        &report,
        active.selected.capsule().release_id.as_str(),
        &active.launch_instance_id,
        &active.spawn_attempt_id,
        payload_registration.payload,
    )? {
        GuardTerminalObservation::Exited(_) => Ok(None),
        GuardTerminalObservation::ContractViolated(message) => Ok(Some(message)),
    }
}

fn active_payload_registration_path(
    paths: &LauncherPaths,
    active: &ActiveLaunchRecord,
) -> PathBuf {
    paths
        .attempts
        .join(&active.spawn_attempt_id)
        .with_extension("payload-registration.json")
}

fn validate_recovered_guard_registration(
    active: &ActiveLaunchRecord,
    guard_registration: &GuardRegistration,
) -> Result<()> {
    guard_registration
        .validate()
        .map_err(|error| LauncherError::Blocked(error.to_string()))?;
    if guard_registration.release_id != active.selected.capsule().release_id
        || guard_registration.launch_instance_id != active.launch_instance_id
        || guard_registration.spawn_attempt_id != active.spawn_attempt_id
    {
        return Err(LauncherError::Blocked(
            "recovered guard registration does not belong to the active launch".to_string(),
        ));
    }
    Ok(())
}

fn validate_recovered_payload_registration(
    active: &ActiveLaunchRecord,
    guard_registration: &GuardRegistration,
    payload_registration: &PayloadRegistration,
) -> Result<()> {
    validate_recovered_guard_registration(active, guard_registration)?;
    if payload_registration.release_id != guard_registration.release_id
        || payload_registration.launch_instance_id != guard_registration.launch_instance_id
        || payload_registration.spawn_attempt_id != guard_registration.spawn_attempt_id
        || payload_registration.process_group.leader != payload_registration.payload
        || payload_registration.process_group.pgid != payload_registration.payload.pid
        || payload_registration.guard_registration_digest
            != guard_registration
                .digest()
                .map_err(|error| LauncherError::Blocked(error.to_string()))?
    {
        return Err(LauncherError::Blocked(
            "recovered payload registration does not belong to the active launch".to_string(),
        ));
    }
    Ok(())
}

fn fence_blocked_launch(
    paths: &LauncherPaths,
    observed: &ControlState,
    message: &str,
) -> Result<ControlState> {
    let latest = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
    if latest.executor_epoch != observed.executor_epoch {
        return Err(LauncherError::Conflict(
            "executor epoch changed before blocked launch could be fenced".to_string(),
        ));
    }
    if latest.activation.phase == ActivationPhase::Blocked {
        ensure_pending_failure_evidence(paths, &latest)?;
        return Ok(latest);
    }
    let active = latest.active_launch.as_ref();
    let attempt = latest.activation.attempt.as_ref();
    let activation_id = attempt
        .map(|attempt| attempt.attempt_id.clone())
        .or_else(|| active.map(|active| active.spawn_attempt_id.clone()))
        .unwrap_or_else(|| {
            format!(
                "steady-state-{}",
                latest.selected.capsule().release_id
            )
        });
    let failed = attempt
        .map(|attempt| attempt.candidate.clone())
        .or_else(|| active.map(|active| active.selected.capsule().clone()))
        .unwrap_or_else(|| latest.selected.capsule().clone());
    let durable_commit = matches!(
        latest.activation.phase,
        ActivationPhase::CommitDecided
            | ActivationPhase::CommitRelaunch
            | ActivationPhase::CommitCleanup
    ) || latest
        .activation
        .receipt
        .as_ref()
        .is_some_and(|receipt| receipt.outcome == ActivationOutcome::Committed);
    let fallback = if durable_commit {
        latest.selected.clone()
    } else {
        attempt
            .map(|attempt| attempt.previous.clone())
            .unwrap_or_else(|| latest.selected.clone())
    };
    let failure = FailureProjection {
        activation_id,
        release_id: failed.release_id.clone(),
        occurred_at: rfc3339_now(),
        fallback_release_id: Some(fallback.capsule().release_id.clone()),
        code: if durable_commit {
            "committed_relaunch_blocked".to_string()
        } else {
            "cooperative_cleanup_blocked".to_string()
        },
        message: message.to_string(),
        failed: Some(failed),
        fallback: Some(fallback),
        evidence_path: Some(paths.failure_evidence.clone()),
        details: active
            .map(|active| {
                BTreeMap::from([
                    (
                        "launchInstanceId".to_string(),
                        active.launch_instance_id.clone(),
                    ),
                    (
                        "spawnAttemptId".to_string(),
                        active.spawn_attempt_id.clone(),
                    ),
                ])
            })
            .unwrap_or_default(),
    };
    let blocked = cas_update(
        &paths.control,
        latest.revision,
        latest.executor_epoch,
        |state| {
            state.activation.phase = ActivationPhase::Blocked;
            state.activation.blocked = Some(BlockedRecord {
                since_unix_ms: unix_time_ms(),
                failure: failure.clone(),
            });
            Ok(())
        },
    )?;
    ensure_pending_failure_evidence(paths, &blocked)?;
    Ok(blocked)
}

fn fence_blocked_launch_after_cleanup(
    paths: &LauncherPaths,
    observed: &ControlState,
    launch_instance_id: &str,
    spawn_attempt_id: &str,
    message: &str,
) -> Result<ControlState> {
    let latest = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
    if latest.executor_epoch != observed.executor_epoch {
        return Err(LauncherError::Conflict(
            "executor epoch changed before cleaned launch could be fenced".to_string(),
        ));
    }
    let active = latest.active_launch.as_ref().ok_or_else(|| {
        LauncherError::Conflict("cleaned launch disappeared before durable fence".to_string())
    })?;
    if active.launch_instance_id != launch_instance_id
        || active.spawn_attempt_id != spawn_attempt_id
    {
        return Err(LauncherError::Conflict(
            "another active launch replaced the cleaned launch before durable fence".to_string(),
        ));
    }
    if latest.activation.phase == ActivationPhase::Blocked {
        let cleared = cas_update(
            &paths.control,
            latest.revision,
            latest.executor_epoch,
            |next| {
                next.active_launch = None;
                Ok(())
            },
        )?;
        ensure_pending_failure_evidence(paths, &cleared)?;
        return Ok(cleared);
    }
    let attempt = latest.activation.attempt.as_ref();
    let failed = attempt
        .map(|attempt| attempt.candidate.clone())
        .unwrap_or_else(|| active.selected.capsule().clone());
    let durable_commit = latest
        .activation
        .receipt
        .as_ref()
        .is_some_and(|receipt| receipt.outcome == ActivationOutcome::Committed);
    let fallback = if durable_commit {
        latest.selected.clone()
    } else {
        attempt
            .map(|attempt| attempt.previous.clone())
            .unwrap_or_else(|| latest.selected.clone())
    };
    let failure = FailureProjection {
        activation_id: attempt
            .map(|attempt| attempt.attempt_id.clone())
            .unwrap_or_else(|| active.spawn_attempt_id.clone()),
        release_id: failed.release_id.clone(),
        occurred_at: rfc3339_now(),
        fallback_release_id: Some(fallback.capsule().release_id.clone()),
        code: if durable_commit {
            "committed_relaunch_blocked".to_string()
        } else {
            "cooperative_cleanup_blocked".to_string()
        },
        message: message.to_string(),
        failed: Some(failed),
        fallback: Some(fallback),
        evidence_path: Some(paths.failure_evidence.clone()),
        details: BTreeMap::from([
            (
                "launchInstanceId".to_string(),
                active.launch_instance_id.clone(),
            ),
            (
                "spawnAttemptId".to_string(),
                active.spawn_attempt_id.clone(),
            ),
        ]),
    };
    let blocked = cas_update(
        &paths.control,
        latest.revision,
        latest.executor_epoch,
        |next| {
            next.active_launch = None;
            next.activation.phase = ActivationPhase::Blocked;
            next.activation.blocked = Some(BlockedRecord {
                since_unix_ms: unix_time_ms(),
                failure: failure.clone(),
            });
            Ok(())
        },
    )?;
    ensure_pending_failure_evidence(paths, &blocked)?;
    Ok(blocked)
}

fn fence_committed_relaunch_failure(
    paths: &LauncherPaths,
    state: &ControlState,
    message: &str,
) -> Result<ControlState> {
    if state.activation.phase == ActivationPhase::Blocked {
        ensure_pending_failure_evidence(paths, state)?;
        return Ok(state.clone());
    }
    if state.activation.phase != ActivationPhase::CommitRelaunch {
        return Err(LauncherError::Conflict(
            "committed relaunch failure can only fence CommitRelaunch".to_string(),
        ));
    }
    let attempt = state.activation.attempt.as_ref().ok_or_else(|| {
        LauncherError::Conflict("committed relaunch has no activation attempt".to_string())
    })?;
    let failure = FailureProjection {
        activation_id: attempt.attempt_id.clone(),
        release_id: attempt.candidate.release_id.clone(),
        occurred_at: rfc3339_now(),
        fallback_release_id: Some(state.selected.capsule().release_id.clone()),
        code: "committed_relaunch_blocked".to_string(),
        message: message.to_string(),
        failed: Some(attempt.candidate.clone()),
        fallback: Some(state.selected.clone()),
        evidence_path: Some(paths.failure_evidence.clone()),
        details: BTreeMap::new(),
    };
    let blocked = cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::Blocked;
            next.activation.blocked = Some(BlockedRecord {
                since_unix_ms: unix_time_ms(),
                failure: failure.clone(),
            });
            Ok(())
        },
    )?;
    ensure_pending_failure_evidence(paths, &blocked)?;
    Ok(blocked)
}

fn launch_selected(
    paths: &LauncherPaths,
    state: &ControlState,
    capsule: &CapsuleRecord,
    launcher_path: &Path,
) -> Result<LaunchOutcome> {
    let recovering_committed_candidate =
        state.activation.phase == ActivationPhase::CommitRelaunch;
    let verified = verify_record_for_spawn(capsule, &capsule.manifest.target)?;
    let attempt_id = state
        .activation
        .attempt
        .as_ref()
        .map(|attempt| attempt.attempt_id.clone())
        .unwrap_or_else(|| "steady-state".to_string());
    let token = guard::issue_ready_token()
        .map_err(|error| LauncherError::Launch(error.to_string()))?;
    let (is_candidate, launch_instance_id, spawn_attempt_id) = launch_protocol_identity(
        state,
        &verified.release_id,
        &attempt_id,
        &token,
    )?;
    let mut current = cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            if next.active_launch.is_some() {
                return Err(LauncherError::Conflict(
                    "another runtime launch is still active".to_string(),
                ));
            }
            next.active_launch = Some(ActiveLaunchRecord {
                selected: next.selected.clone(),
                launch_instance_id: launch_instance_id.clone(),
                spawn_attempt_id: spawn_attempt_id.clone(),
                phase: ActiveLaunchPhase::SpawnPlanned,
                guard_registration: None,
                payload_registration: None,
                start_authorization: None,
                ready_expectation: None,
            });
            Ok(())
        },
    )?;
    let protocol_base = paths.attempts.join(&spawn_attempt_id);
    let request_path = protocol_base.with_extension("guard-request.json");
    let guard_registration_path = protocol_base.with_extension("guard-registration.json");
    let guard_ack_path = protocol_base.with_extension("guard-ack.json");
    let payload_registration_path = protocol_base.with_extension("payload-registration.json");
    let start_authorization_path = protocol_base.with_extension("start-authorization.json");
    let exec_report_path = protocol_base.with_extension("exec-report.json");
    let exit_report_path = protocol_base.with_extension("exit-report.json");
    let ready_path = protocol_base.with_extension("ready.json");
    if !launcher_path.is_absolute() {
        return Err(LauncherError::InvalidRequest(
            "launcher executable path must be absolute".to_string(),
        ));
    }
    let parent = crate::process::current_process_identity()
        .map_err(|error| crate::io_error("observe launcher identity", error))?;
    let environment = build_payload_environment(
        paths,
        launcher_path,
        &ready_path,
        &token,
        &verified.release_id,
        &launch_instance_id,
        &spawn_attempt_id,
    );
    let request = GuardLaunchRequest {
        protocol_version: guard::GUARD_PROTOCOL_VERSION,
        release_id: verified.release_id.clone(),
        launch_instance_id: launch_instance_id.clone(),
        spawn_attempt_id: spawn_attempt_id.clone(),
        parent,
        executable: verified.executable.clone(),
        arguments: verified.manifest.launch.arguments.clone(),
        cwd: verified.cwd.clone(),
        environment,
        guard_registration_path: guard_registration_path.clone(),
        guard_ack_path: guard_ack_path.clone(),
        payload_registration_path: payload_registration_path.clone(),
        start_authorization_path: start_authorization_path.clone(),
        exec_report_path: exec_report_path.clone(),
        exit_report_path: exit_report_path.clone(),
    };
    request
        .validate()
        .map_err(|error| LauncherError::Launch(error.to_string()))?;
    guard::write_protocol_file(&request_path, &request)
        .map_err(|error| LauncherError::Launch(error.to_string()))?;
    let (parent_monitor, parent_keeper) = guard::parent_liveness_pipe()
        .map_err(|error| LauncherError::Launch(error.to_string()))?;
    guard::clear_cloexec(parent_monitor.as_raw_fd())
        .map_err(|error| LauncherError::Launch(error.to_string()))?;
    let mut guard_command = std::process::Command::new(launcher_path);
    guard_command
        .arg("--runtime-capsule-guard")
        .arg(&request_path)
        .env_clear()
        .env(
            guard::PARENT_LIVENESS_FD_ENV,
            parent_monitor.as_raw_fd().to_string(),
        );
    let mut guard_child = guard_command.spawn().map_err(|error| {
        crate::io_error(format!("launch guard {}", launcher_path.display()), error)
    })?;
    drop(parent_monitor);
    let guard_registration: GuardRegistration = wait_for_guard_file(
        &guard_registration_path,
        &mut guard_child,
        "guard registration",
        None,
    )?;
    guard_registration
        .validate_live()
        .map_err(|error| LauncherError::Blocked(error.to_string()))?;
    if guard_registration.release_id != verified.release_id
        || guard_registration.launch_instance_id != launch_instance_id
        || guard_registration.spawn_attempt_id != spawn_attempt_id
        || guard_registration.parent != parent
    {
        drop(parent_keeper);
        return Err(LauncherError::Blocked(
            "guard registration does not match its launch request".to_string(),
        ));
    }
    current = cas_update(
        &paths.control,
        current.revision,
        current.executor_epoch,
        |next| {
            let active = next.active_launch.as_mut().ok_or_else(|| {
                LauncherError::Conflict("active launch disappeared".to_string())
            })?;
            active.guard_registration = Some(guard_registration.clone());
            active.phase = ActiveLaunchPhase::GuardRegistered;
            if is_candidate {
                let attempt = next.activation.attempt.as_mut().ok_or_else(|| {
                    LauncherError::Conflict("activation attempt disappeared".to_string())
                })?;
                attempt.guard_registration = Some(guard_registration.clone());
                next.activation.phase = ActivationPhase::GuardRegistered;
            }
            Ok(())
        },
    )?;
    guard::write_protocol_file(
        &guard_ack_path,
        &GuardAcknowledgement {
            protocol_version: guard::GUARD_PROTOCOL_VERSION,
            guard_registration_digest: guard_registration
                .digest()
                .map_err(|error| LauncherError::Blocked(error.to_string()))?,
        },
    )
    .map_err(|error| LauncherError::Blocked(error.to_string()))?;
    let payload_registration: PayloadRegistration = wait_for_guard_file(
        &payload_registration_path,
        &mut guard_child,
        "payload registration",
        None,
    )?;
    payload_registration
        .validate(&guard_registration)
        .map_err(|error| LauncherError::Blocked(error.to_string()))?;
    let identity = payload_registration.payload;
    let bearer = ReadyBearer::bind(
        verified.release_id.clone(),
        launch_instance_id.clone(),
        spawn_attempt_id.clone(),
        identity,
        token,
    )
    .map_err(|error| LauncherError::Launch(error.to_string()))?;
    let authorization = StartAuthorization {
        protocol_version: guard::GUARD_PROTOCOL_VERSION,
        release_id: payload_registration.release_id.clone(),
        launch_instance_id: payload_registration.launch_instance_id.clone(),
        spawn_attempt_id: payload_registration.spawn_attempt_id.clone(),
        payload: identity,
        payload_registration_digest: payload_registration
            .digest()
            .map_err(|error| LauncherError::Blocked(error.to_string()))?,
    };
    authorization
        .validate(&payload_registration)
        .map_err(|error| LauncherError::Blocked(error.to_string()))?;
    current = cas_update(
        &paths.control,
        current.revision,
        current.executor_epoch,
        |next| {
            let active = next.active_launch.as_mut().ok_or_else(|| {
                LauncherError::Conflict("active launch disappeared".to_string())
            })?;
            active.payload_registration = Some(payload_registration.clone());
            active.start_authorization = Some(authorization.clone());
            active.ready_expectation = Some(bearer.expectation.clone());
            active.phase = ActiveLaunchPhase::StartAuthorized;
            if is_candidate {
                let attempt = next.activation.attempt.as_mut().ok_or_else(|| {
                    LauncherError::Conflict("activation attempt disappeared".to_string())
                })?;
                attempt.payload_registration = Some(payload_registration.clone());
                attempt.start_authorization = Some(authorization.clone());
                attempt.ready_expectation = Some(bearer.expectation.clone());
                attempt.known_descendants = vec![identity];
                next.activation.phase = ActivationPhase::StartAuthorized;
            }
            Ok(())
        },
    )?;
    guard::write_protocol_file(&start_authorization_path, &authorization)
        .map_err(|error| LauncherError::Blocked(error.to_string()))?;
    let exec_report: GuardExecReport =
        wait_for_guard_file(
            &exec_report_path,
            &mut guard_child,
            "exec report",
            Some((&guard_registration, &payload_registration)),
        )?;
    match exec_report {
        GuardExecReport::ExecSucceeded { payload } if payload == identity => {}
        GuardExecReport::ExecFailed { errno, .. } => {
            drop(parent_keeper);
            return Err(LauncherError::Launch(format!(
                "payload exec failed with errno {errno}"
            )));
        }
        _ => {
            drop(parent_keeper);
            return Err(LauncherError::Blocked(
                "guard exec report did not match registered payload".to_string(),
            ));
        }
    }
    persist_runtime_started(
        paths,
        current,
        is_candidate,
        bearer.expectation.clone(),
    )?;
    let deadline =
        Instant::now() + Duration::from_millis(verified.manifest.launch.readiness.timeout_ms);
    loop {
        if is_candidate
            && ControlState::load(&paths.control)?
                .is_some_and(|control| control.activation.phase == ActivationPhase::RollbackDecided)
        {
            terminate_registered_launch(&guard_registration, &payload_registration)?;
            clear_active_launch(paths, &launch_instance_id)?;
            drop(parent_keeper);
            return Err(LauncherError::Launch("rollback requested".to_string()));
        }
        if guard::consume_ready_marker(&ready_path, &bearer.expectation)
            .map_err(|error| LauncherError::Blocked(error.to_string()))?
        {
            if is_candidate {
                let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                    LauncherError::Conflict("control state disappeared".to_string())
                })?;
                let _observing = cas_update(
                    &paths.control,
                    latest.revision,
                    latest.executor_epoch,
                    |next| {
                        next.activation.phase = ActivationPhase::Observing;
                        if let Some(attempt) = next.activation.attempt.as_mut() {
                            attempt.observation_deadline_unix_ms =
                                Some(unix_time_ms() + OBSERVATION_WINDOW.as_millis() as u64);
                        }
                        Ok(())
                    },
                )?;
                let observation_deadline = Instant::now() + OBSERVATION_WINDOW;
                loop {
                    if let Some(status) =
                        read_guard_exit_status(
                            &exit_report_path,
                            &mut guard_child,
                            &guard_registration,
                            &payload_registration,
                        )?
                    {
                        drop(parent_keeper);
                        clear_active_launch(paths, &launch_instance_id)?;
                        return Ok(LaunchOutcome {
                            status,
                            ready: true,
                            committed: false,
                        });
                    }
                    let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                        LauncherError::Conflict("control state disappeared".to_string())
                    })?;
                    if latest.activation.phase == ActivationPhase::RollbackDecided {
                        terminate_registered_launch(
                            &guard_registration,
                            &payload_registration,
                        )?;
                        clear_active_launch(paths, &launch_instance_id)?;
                        drop(parent_keeper);
                        return Err(LauncherError::Launch(
                            "rollback requested during observation".to_string(),
                        ));
                    }
                    if Instant::now() >= observation_deadline {
                        commit_candidate(paths, latest)?;
                        break;
                    }
                    ensure_guard_alive(
                        &mut guard_child,
                        &guard_registration,
                        &payload_registration,
                    )?;
                    std::thread::sleep(Duration::from_millis(25));
                }
            } else if recovering_committed_candidate {
                let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                    LauncherError::Conflict("control state disappeared".to_string())
                })?;
                finalize_commit_after_ready(paths, latest)?;
            }
            let status = wait_for_guard_exit(
                &exit_report_path,
                &mut guard_child,
                &guard_registration,
                &payload_registration,
            )?;
            drop(parent_keeper);
            clear_active_launch(paths, &launch_instance_id)?;
            return Ok(LaunchOutcome {
                status,
                ready: true,
                committed: is_candidate,
            });
        }
        if let Some(status) = read_guard_exit_status(
            &exit_report_path,
            &mut guard_child,
            &guard_registration,
            &payload_registration,
        )? {
            drop(parent_keeper);
            clear_active_launch(paths, &launch_instance_id)?;
            return Ok(LaunchOutcome {
                status,
                ready: false,
                committed: false,
            });
        }
        if Instant::now() >= deadline {
            terminate_registered_launch(&guard_registration, &payload_registration)?;
            clear_active_launch(paths, &launch_instance_id)?;
            drop(parent_keeper);
            return Err(LauncherError::Launch("payload readiness timed out".to_string()));
        }
        ensure_guard_alive(
            &mut guard_child,
            &guard_registration,
            &payload_registration,
        )?;
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn launch_protocol_identity(
    state: &ControlState,
    release_id: &str,
    attempt_id: &str,
    entropy: &str,
) -> Result<(bool, String, String)> {
    let is_candidate = state.activation.phase == ActivationPhase::SpawnPlanned
        && state
            .activation
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.candidate.release_id == release_id);
    if is_candidate {
        let attempt = state.activation.attempt.as_ref().expect("checked above");
        let launch_instance_id = attempt.launch_instance_id.clone().ok_or_else(|| {
            LauncherError::Conflict("candidate launch has no launchInstanceId".to_string())
        })?;
        let spawn_attempt_id = attempt.spawn_attempt_id.clone().ok_or_else(|| {
            LauncherError::Conflict("candidate launch has no spawnAttemptId".to_string())
        })?;
        return Ok((true, launch_instance_id, spawn_attempt_id));
    }
    let nonce = entropy.get(..16).unwrap_or(entropy);
    let launch_instance_id = format!(
        "{}-{}-{nonce}",
        unsafe { libc::getpid() },
        unix_time_ms()
    );
    let spawn_attempt_id = format!("{attempt_id}-{launch_instance_id}");
    Ok((false, launch_instance_id, spawn_attempt_id))
}

fn build_payload_environment(
    paths: &LauncherPaths,
    launcher_path: &Path,
    ready_path: &Path,
    token: &str,
    release_id: &str,
    launch_instance_id: &str,
    spawn_attempt_id: &str,
) -> BTreeMap<String, String> {
    const INHERITED_ENVIRONMENT_ALLOWLIST: [&str; 17] = [
        "HOME",
        "USER",
        "LOGNAME",
        "TMPDIR",
        "PATH",
        "SHELL",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "__CF_USER_TEXT_ENCODING",
        "DISPLAY",
        "WAYLAND_DISPLAY",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
        "MORPHEUS_HOME",
        "ROOT_WORKER_WORKSPACE",
        "ROOT_WORKER_SOURCE_WORKSPACE",
    ];
    let inherited = std::env::vars().collect::<BTreeMap<_, _>>();
    let mut environment = INHERITED_ENVIRONMENT_ALLOWLIST
        .into_iter()
        .filter_map(|name| inherited.get(name).cloned().map(|value| (name.to_string(), value)))
        .collect::<BTreeMap<_, _>>();
    environment.insert(
        "RUNTIME_CAPSULE_READY_PROTOCOL".to_string(),
        guard::GUARD_PROTOCOL_VERSION.to_string(),
    );
    environment.insert(
        "RUNTIME_CAPSULE_READY_PATH".to_string(),
        ready_path.display().to_string(),
    );
    environment.insert("RUNTIME_CAPSULE_READY_TOKEN".to_string(), token.to_string());
    environment.insert(
        "RUNTIME_CAPSULE_RELEASE_ID".to_string(),
        release_id.to_string(),
    );
    environment.insert(
        "RUNTIME_CAPSULE_LAUNCH_INSTANCE_ID".to_string(),
        launch_instance_id.to_string(),
    );
    environment.insert(
        "RUNTIME_CAPSULE_SPAWN_ATTEMPT_ID".to_string(),
        spawn_attempt_id.to_string(),
    );
    environment.insert(
        "RUNTIME_CAPSULE_FAILURE_EVIDENCE_PATH".to_string(),
        paths.failure_evidence.display().to_string(),
    );
    environment.insert(
        "RUNTIME_CAPSULE_LAUNCHER_PATH".to_string(),
        launcher_path.display().to_string(),
    );
    environment.insert(
        "RUNTIME_CAPSULE_LAUNCHER_HOME".to_string(),
        paths.root.display().to_string(),
    );
    environment
}

fn persist_runtime_started(
    paths: &LauncherPaths,
    state: ControlState,
    is_candidate: bool,
    expectation: guard::ReadyExpectation,
) -> Result<ControlState> {
    let started = cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            let active = next.active_launch.as_mut().ok_or_else(|| {
                LauncherError::Conflict("active launch disappeared".to_string())
            })?;
            active.phase = ActiveLaunchPhase::Running;
            if is_candidate {
                next.activation.phase = ActivationPhase::RuntimeStarted;
            }
            Ok(())
        },
    )?;
    if !is_candidate {
        return Ok(started);
    }
    cas_update(
        &paths.control,
        started.revision,
        started.executor_epoch,
        |next| {
            let attempt = next.activation.attempt.as_mut().ok_or_else(|| {
                LauncherError::Conflict("activation attempt disappeared".to_string())
            })?;
            attempt.ready_expectation = Some(expectation);
            next.activation.phase = ActivationPhase::AwaitingReady;
            Ok(())
        },
    )
}

fn wait_for_guard_file<T: serde::de::DeserializeOwned>(
    path: &Path,
    guard_child: &mut std::process::Child,
    label: &str,
    registered: Option<(&GuardRegistration, &PayloadRegistration)>,
) -> Result<T> {
    let deadline = Instant::now() + GUARD_HANDSHAKE_TIMEOUT;
    loop {
        if let Some(value) = guard::read_protocol_file_if_exists(path)
            .map_err(|error| LauncherError::Blocked(error.to_string()))?
        {
            return Ok(value);
        }
        if let Some(status) = guard_child
            .try_wait()
            .map_err(|error| crate::io_error(format!("observe {label} guard"), error))?
        {
            return Err(LauncherError::Launch(format!(
                "guard exited with {status} before durable {label}"
            )));
        }
        if Instant::now() >= deadline {
            if let Some((guard_registration, payload_registration)) = registered {
                terminate_registered_launch(guard_registration, payload_registration)?;
            } else {
                let guard_pid = i32::try_from(guard_child.id()).map_err(|_| {
                    LauncherError::Blocked("guard pid does not fit platform pid type".to_string())
                })?;
                if let Some(identity) = ProcessIdentity::observe(guard_pid)
                    .map_err(|error| crate::io_error("observe timed-out guard", error))?
                {
                    terminate_identity_tree_and_observe_empty(
                        identity,
                        TerminationPolicy::default(),
                    )
                        .map_err(|error| LauncherError::Blocked(error.to_string()))?;
                }
            }
            let _ = guard_child.wait();
            return Err(LauncherError::Blocked(format!(
                "guard timed out before durable {label}"
            )));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read_guard_exit_status(
    path: &Path,
    guard_child: &mut std::process::Child,
    guard_registration: &GuardRegistration,
    payload_registration: &PayloadRegistration,
) -> Result<Option<ExitStatus>> {
    let Some(report) = guard::read_protocol_file_if_exists::<GuardExitReport>(path)
        .map_err(|error| LauncherError::Blocked(error.to_string()))?
    else {
        return Ok(None);
    };
    let observation = validate_guard_exit_report(
        &report,
        &guard_registration.release_id,
        &guard_registration.launch_instance_id,
        &guard_registration.spawn_attempt_id,
        payload_registration.payload,
    )?;
    match observation {
        GuardTerminalObservation::Exited(raw_wait_status) => {
            let deadline = Instant::now() + GUARD_HANDSHAKE_TIMEOUT;
            loop {
                if guard_child
                    .try_wait()
                    .map_err(|error| crate::io_error("observe exited hidden guard", error))?
                    .is_some()
                {
                    return Ok(Some(ExitStatus::from_raw(raw_wait_status)));
                }
                if Instant::now() >= deadline {
                    return Err(LauncherError::Blocked(
                        "hidden guard did not exit after publishing its terminal report"
                            .to_string(),
                    ));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        GuardTerminalObservation::ContractViolated(message) => {
            Err(LauncherError::Blocked(message))
        }
    }
}

#[derive(Debug)]
enum GuardTerminalObservation {
    Exited(i32),
    ContractViolated(String),
}

fn validate_guard_exit_report(
    report: &GuardExitReport,
    expected_release_id: &str,
    expected_launch_instance_id: &str,
    expected_spawn_attempt_id: &str,
    expected_payload: ProcessIdentity,
) -> Result<GuardTerminalObservation> {
    let (release_id, launch_instance_id, spawn_attempt_id, payload) = match report {
        GuardExitReport::Exited {
            release_id,
            launch_instance_id,
            spawn_attempt_id,
            payload,
            ..
        }
        | GuardExitReport::ContractViolated {
            release_id,
            launch_instance_id,
            spawn_attempt_id,
            payload,
            ..
        } => (
            release_id,
            launch_instance_id,
            spawn_attempt_id,
            payload,
        ),
    };
    if release_id != expected_release_id
        || launch_instance_id != expected_launch_instance_id
        || spawn_attempt_id != expected_spawn_attempt_id
        || *payload != expected_payload
    {
        return Err(LauncherError::Blocked(
            "guard terminal report does not match the registered launch identity".to_string(),
        ));
    }
    match report {
        GuardExitReport::Exited {
            raw_wait_status,
            cleanup_evidence: CleanupEvidence::CooperativeObservedEmpty,
            ..
        } => Ok(GuardTerminalObservation::Exited(*raw_wait_status)),
        GuardExitReport::ContractViolated { message, .. } => {
            Ok(GuardTerminalObservation::ContractViolated(message.clone()))
        }
    }
}

fn wait_for_guard_exit(
    path: &Path,
    guard_child: &mut std::process::Child,
    guard_registration: &GuardRegistration,
    payload_registration: &PayloadRegistration,
) -> Result<ExitStatus> {
    loop {
        if let Some(status) = read_guard_exit_status(
            path,
            guard_child,
            guard_registration,
            payload_registration,
        )? {
            return Ok(status);
        }
        ensure_guard_alive(guard_child, guard_registration, payload_registration)?;
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn ensure_guard_alive(
    guard_child: &mut std::process::Child,
    guard_registration: &GuardRegistration,
    payload_registration: &PayloadRegistration,
) -> Result<()> {
    if let Some(status) = guard_child
        .try_wait()
        .map_err(|error| crate::io_error("observe hidden guard", error))?
    {
        terminate_registered_launch(guard_registration, payload_registration)?;
        return Err(LauncherError::Blocked(format!(
            "hidden guard exited with {status} before the payload exit report"
        )));
    }
    Ok(())
}

fn terminate_registered_launch(
    guard_registration: &GuardRegistration,
    payload_registration: &PayloadRegistration,
) -> Result<()> {
    terminate_and_observe_empty(
        &TerminationTarget {
            root: payload_registration.payload,
            process_group: payload_registration.process_group,
            guard: Some(guard_registration.guard),
            known_descendants: BTreeSet::from([payload_registration.payload]),
        },
        TerminationPolicy::default(),
    )
    .map_err(|error| LauncherError::Blocked(error.to_string()))?;
    Ok(())
}

fn clear_active_launch(paths: &LauncherPaths, launch_instance_id: &str) -> Result<ControlState> {
    let state = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
    if state
        .active_launch
        .as_ref()
        .map_or(true, |active| active.launch_instance_id != launch_instance_id)
    {
        return Ok(state);
    }
    cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            if next
                .active_launch
                .as_ref()
                .is_some_and(|active| active.launch_instance_id == launch_instance_id)
            {
                next.active_launch = None;
            }
            Ok(())
        },
    )
}

fn authorize_candidate_after_old_stopped(
    paths: &LauncherPaths,
    state: ControlState,
) -> Result<ControlState> {
    let stopping = cas_update(&paths.control, state.revision, state.executor_epoch, |next| {
        next.activation.phase = ActivationPhase::StoppingOld;
        Ok(())
    })?;
    let stopped = cas_update(
        &paths.control,
        stopping.revision,
        stopping.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::OldStopped;
            Ok(())
        },
    )?;
    cas_update(&paths.control, stopped.revision, stopped.executor_epoch, |next| {
        let revision = next.revision;
        let candidate = {
            let attempt = next.activation.attempt.as_mut().ok_or_else(|| {
                LauncherError::Conflict("prepared activation has no attempt".to_string())
            })?;
            attempt.launch_instance_id = Some(format!(
                "{}-{}",
                unsafe { libc::getpid() },
                unix_time_ms()
            ));
            attempt.spawn_attempt_id = Some(format!(
                "{}-{revision}",
                attempt.attempt_id
            ));
            attempt.candidate.clone()
        };
        let current = next.external_current.clone();
        let previous = next.external_previous.clone();
        let mut cleanup_candidates = Vec::new();
        match (
            current.as_ref().map(|capsule| &capsule.release_id),
            previous.as_ref().map(|capsule| &capsule.release_id),
        ) {
            (Some(current_id), _) if current_id == &candidate.release_id => {
                // Relaunching the current content-addressed runtime does not rotate generations.
            }
            (_, Some(previous_id)) if previous_id == &candidate.release_id => {
                next.external_current = Some(candidate.clone());
                next.external_previous = current;
            }
            _ => {
                next.external_current = Some(candidate.clone());
                next.external_previous = current;
                if let Some(displaced) = previous
                    && displaced.release_id != candidate.release_id
                    && next
                        .external_previous
                        .as_ref()
                        .is_none_or(|retained| retained.release_id != displaced.release_id)
                {
                    cleanup_candidates.push(displaced);
                }
            }
        }
        next.selected = SelectedRuntime::external(candidate);
        next.activation.cleanup.pending =
            cleanup_candidates_if_unretained(next, cleanup_candidates);
        next.activation.phase = ActivationPhase::SpawnPlanned;
        Ok(())
    })
}

fn commit_candidate(paths: &LauncherPaths, state: ControlState) -> Result<ControlState> {
    let decided = cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            let attempt = next.activation.attempt.clone().ok_or_else(|| {
                LauncherError::Conflict("candidate commit has no attempt".to_string())
            })?;
            next.activation.phase = ActivationPhase::CommitDecided;
            next.activation.winner = Some(WinnerRecord {
                selected: next.selected.clone(),
                decided_at_unix_ms: unix_time_ms(),
                reason: "candidate reported readiness".to_string(),
            });
            next.activation.receipt = Some(ActivationReceipt {
                attempt_id: attempt.attempt_id,
                candidate_release_id: attempt.candidate.release_id,
                outcome: ActivationOutcome::Committed,
                selected: next.selected.clone(),
                completed_at_unix_ms: unix_time_ms(),
                reason: None,
            });
            Ok(())
        },
    )?;
    let relaunch = cas_update(
        &paths.control,
        decided.revision,
        decided.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::CommitRelaunch;
            Ok(())
        },
    )?;
    finalize_commit_after_ready(paths, relaunch)
}

fn finalize_commit_after_ready(
    paths: &LauncherPaths,
    state: ControlState,
) -> Result<ControlState> {
    let cleanup = cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::CommitCleanup;
            Ok(())
        },
    )?;
    cleanup_and_idle(paths, cleanup)
}

fn rollback_failed_candidate(
    paths: &LauncherPaths,
    state: ControlState,
    reason: String,
) -> Result<ControlState> {
    let decided = cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            let attempt = next.activation.attempt.clone().ok_or_else(|| {
                LauncherError::Conflict("candidate rollback has no attempt".to_string())
            })?;
            next.activation.phase = ActivationPhase::RollbackDecided;
            next.activation.winner = Some(WinnerRecord {
                selected: attempt.previous,
                decided_at_unix_ms: unix_time_ms(),
                reason: reason.clone(),
            });
            next.activation.receipt = None;
            Ok(())
        },
    )?;
    execute_rollback(paths, decided)
}

fn execute_rollback(paths: &LauncherPaths, state: ControlState) -> Result<ControlState> {
    let stopping = cas_update(&paths.control, state.revision, state.executor_epoch, |next| {
        next.activation.phase = ActivationPhase::StoppingCandidate;
        Ok(())
    })?;
    let attempt = stopping.activation.attempt.as_ref().ok_or_else(|| {
        LauncherError::Conflict("candidate stop has no activation attempt".to_string())
    })?;
    observe_attempt_tree_stopped(attempt)?;
    let stopped = cas_update(
        &paths.control,
        stopping.revision,
        stopping.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::CandidateStopped;
            Ok(())
        },
    )?;
    let restored = cas_update(&paths.control, stopped.revision, stopped.executor_epoch, |next| {
        let attempt = next.activation.attempt.clone().ok_or_else(|| {
            LauncherError::Conflict("rollback has no activation attempt".to_string())
        })?;
        next.activation.phase = ActivationPhase::Restoring;
        next.external_current = attempt.previous_external_current.clone();
        next.external_previous = attempt.previous_external_previous.clone();
        next.selected = attempt.previous.clone();
        let candidate_is_retained = [
            next.external_current.as_ref(),
            next.external_previous.as_ref(),
            Some(next.selected.capsule()),
            Some(&next.trusted_seed.capsule),
        ]
        .into_iter()
        .flatten()
        .any(|capsule| capsule.release_id == attempt.candidate.release_id);
        next.activation.cleanup.pending = if candidate_is_retained {
            Vec::new()
        } else {
            vec![attempt.candidate.clone()]
        };
        next.activation.receipt = Some(ActivationReceipt {
            attempt_id: attempt.attempt_id,
            candidate_release_id: attempt.candidate.release_id,
            outcome: ActivationOutcome::RolledBack,
            selected: next.selected.clone(),
            completed_at_unix_ms: unix_time_ms(),
            reason: next
                .activation
                .winner
                .as_ref()
                .map(|winner| winner.reason.clone()),
        });
        Ok(())
    })?;
    let evidence = cas_update(
        &paths.control,
        restored.revision,
        restored.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::EvidencePending;
            if let Some(attempt) = next.activation.attempt.as_mut() {
                attempt
                    .annotations
                    .entry("failureCode".to_string())
                    .or_insert_with(|| "activation_rolled_back".to_string());
            }
            Ok(())
        },
    )?;
    let failure_code = evidence
        .activation
        .attempt
        .as_ref()
        .and_then(|attempt| attempt.annotations.get("failureCode"))
        .map(String::as_str)
        .unwrap_or("activation_rolled_back");
    write_failure_projection_with_code(paths, &evidence, failure_code)?;
    Ok(evidence)
}

fn cleanup_and_idle(paths: &LauncherPaths, state: ControlState) -> Result<ControlState> {
    cleanup_pending_capsules(paths, &state)?;
    cas_update(&paths.control, state.revision, state.executor_epoch, |next| {
        next.activation.cleanup.pending.clear();
        next.activation.phase = ActivationPhase::Idle;
        next.activation.attempt = None;
        Ok(())
    })
}

fn reconcile_interrupted(
    paths: &LauncherPaths,
    _target: &CapsuleTarget,
    state: &mut ControlState,
) -> Result<()> {
    if state.activation.phase == ActivationPhase::Idle
        && !state.activation.cleanup.pending.is_empty()
    {
        cleanup_pending_capsules(paths, state)?;
        *state = cas_update(&paths.control, state.revision, state.executor_epoch, |next| {
            next.activation.cleanup.pending.clear();
            Ok(())
        })?;
        return Ok(());
    }
    if matches!(
        state.activation.phase,
        ActivationPhase::CommitDecided | ActivationPhase::CommitCleanup
    ) {
        *state = cas_update(&paths.control, state.revision, state.executor_epoch, |next| {
            next.activation.phase = ActivationPhase::CommitRelaunch;
            Ok(())
        })?;
        return Ok(());
    }
    if matches!(
        state.activation.phase,
        ActivationPhase::StoppingOld
            | ActivationPhase::OldStopped
            | ActivationPhase::SpawnPlanned
            | ActivationPhase::GuardRegistered
            | ActivationPhase::StartAuthorized
            | ActivationPhase::RuntimeStarted
            | ActivationPhase::AwaitingReady
            | ActivationPhase::Observing
            | ActivationPhase::StoppingCandidate
            | ActivationPhase::CandidateStopped
            | ActivationPhase::Restoring
    ) {
        let attempt = state.activation.attempt.as_ref().ok_or_else(|| {
            LauncherError::Conflict("interrupted activation has no attempt".to_string())
        })?;
        observe_attempt_tree_stopped(attempt)?;
        *state = cas_update(&paths.control, state.revision, state.executor_epoch, |next| {
            let previous = next
                .activation
                .attempt
                .as_ref()
                .map(|attempt| attempt.previous.clone())
                .ok_or_else(|| {
                    LauncherError::Conflict("interrupted activation has no attempt".to_string())
                })?;
            next.activation.phase = ActivationPhase::RollbackDecided;
            next.activation.winner = Some(WinnerRecord {
                selected: previous,
                decided_at_unix_ms: unix_time_ms(),
                reason: "recovered interrupted activation".to_string(),
            });
            next.activation.receipt = None;
            Ok(())
        })?;
    }
    Ok(())
}

fn rebind_seed(
    paths: &LauncherPaths,
    revision: u64,
    epoch: u64,
    seed: crate::TrustedSeed,
) -> Result<ControlState> {
    let before = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state is not initialized".to_string()))?;
    if before.revision != revision || before.executor_epoch != epoch {
        return Err(LauncherError::Conflict(
            "Seed rebind state changed before reconciliation".to_string(),
        ));
    }
    if before.trusted_seed == seed {
        return Ok(before);
    }

    if before.activation.phase == ActivationPhase::Prepared {
        if let Some(attempt) = &before.activation.attempt {
            archive_seed_replacement(paths, &before, attempt)?;
        }
        let rebound = cas_update(&paths.control, revision, epoch, |state| {
            let selected_seed = matches!(state.selected, SelectedRuntime::Seed { .. });
            let candidate = state
                .activation
                .attempt
                .as_ref()
                .map(|attempt| attempt.candidate.clone());
            let receipt = state.activation.attempt.as_ref().map(|attempt| ActivationReceipt {
                attempt_id: attempt.attempt_id.clone(),
                candidate_release_id: attempt.candidate.release_id.clone(),
                outcome: ActivationOutcome::RolledBack,
                selected: if selected_seed {
                    SelectedRuntime::seed(seed.capsule.clone())
                } else {
                    state.selected.clone()
                },
                completed_at_unix_ms: unix_time_ms(),
                reason: Some("Seed was replaced before activation".to_string()),
            });
            state.activation = Default::default();
            state.activation.receipt = receipt;
            state.trusted_seed = seed.clone();
            if selected_seed {
                state.selected = SelectedRuntime::seed(seed.capsule.clone());
            }
            let pending = cleanup_candidates_if_unretained(state, candidate);
            state.activation.cleanup.pending = pending;
            Ok(())
        })?;
        return cleanup_and_idle(paths, rebound);
    }

    if matches!(
        before.activation.phase,
        ActivationPhase::CommitDecided
            | ActivationPhase::CommitRelaunch
            | ActivationPhase::CommitCleanup
    ) {
        return cas_update(&paths.control, revision, epoch, |state| {
            let old_seed_release = state.trusted_seed.capsule.release_id.clone();
            state.trusted_seed = seed.clone();
            if let Some(attempt) = state.activation.attempt.as_mut()
                && matches!(
                    &attempt.previous,
                    SelectedRuntime::Seed { capsule }
                        if capsule.release_id == old_seed_release
                )
            {
                attempt.previous = SelectedRuntime::seed(seed.capsule.clone());
            }
            Ok(())
        });
    }

    let active_old_seed = before.activation.phase != ActivationPhase::Idle
        && matches!(
            before.activation.attempt.as_ref().map(|attempt| &attempt.previous),
            Some(SelectedRuntime::Seed { capsule })
                if capsule.release_id == before.trusted_seed.capsule.release_id
        );
    if active_old_seed {
        let attempt = before.activation.attempt.as_ref().ok_or_else(|| {
            LauncherError::Conflict("active Seed replacement has no attempt".to_string())
        })?;
        observe_attempt_tree_stopped(attempt)?;
        archive_seed_replacement(paths, &before, attempt)?;
        let rollback = cas_update(&paths.control, revision, epoch, |state| {
            let fallback = {
                let attempt = state.activation.attempt.as_mut().ok_or_else(|| {
                    LauncherError::Conflict("active Seed replacement has no attempt".to_string())
                })?;
                attempt.previous = SelectedRuntime::seed(seed.capsule.clone());
                attempt
                    .annotations
                    .insert("failureCode".to_string(), "seed_replaced".to_string());
                attempt.previous.clone()
            };
            state.trusted_seed = seed.clone();
            state.activation.phase = ActivationPhase::RollbackDecided;
            state.activation.blocked = None;
            state.activation.winner = Some(WinnerRecord {
                selected: fallback,
                decided_at_unix_ms: unix_time_ms(),
                reason: "trusted Seed was replaced during activation recovery".to_string(),
            });
            state.activation.receipt = None;
            Ok(())
        })?;
        return execute_rollback(paths, rollback);
    }

    cas_update(&paths.control, revision, epoch, |state| {
        let selected_seed = matches!(state.selected, SelectedRuntime::Seed { .. });
        state.trusted_seed = seed.clone();
        if selected_seed {
            state.selected = SelectedRuntime::seed(seed.capsule.clone());
        }
        Ok(())
    })
}

fn observe_attempt_tree_stopped(attempt: &AttemptRecord) -> Result<()> {
    match (
        attempt.guard_registration.as_ref(),
        attempt.payload_registration.as_ref(),
    ) {
        (Some(guard_registration), Some(payload_registration)) => {
            terminate_registered_launch(guard_registration, payload_registration)
        }
        (Some(guard_registration), None) => {
            terminate_identity_and_observe_empty(
                guard_registration.guard,
                TerminationPolicy::default(),
            )
            .map_err(|error| LauncherError::Blocked(error.to_string()))?;
            Ok(())
        }
        (None, None) => Ok(()),
        (None, Some(_)) => Err(LauncherError::Blocked(
            "payload registration exists without a guard identity".to_string(),
        )),
    }
}

fn archive_seed_replacement(
    paths: &LauncherPaths,
    state: &ControlState,
    attempt: &AttemptRecord,
) -> Result<()> {
    let transaction_archive = paths
        .attempts
        .join(format!("{}.seed-replaced.transaction.json", attempt.attempt_id));
    if !transaction_archive.exists() {
        crate::control::write_json_atomic(&transaction_archive, state)?;
    }
    let evidence_archive = paths
        .attempts
        .join(format!("{}.seed-replaced.failure.json", attempt.attempt_id));
    if paths.failure_evidence.exists() && !evidence_archive.exists() {
        std::fs::rename(&paths.failure_evidence, &evidence_archive).map_err(|error| {
            crate::io_error(
                format!(
                    "archive {} as {}",
                    paths.failure_evidence.display(),
                    evidence_archive.display()
                ),
                error,
            )
        })?;
        let parent = paths.failure_evidence.parent().ok_or_else(|| {
            LauncherError::InvalidRequest("failure evidence has no parent".to_string())
        })?;
        std::fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| crate::io_error(format!("sync {}", parent.display()), error))?;
    }
    Ok(())
}

fn mutate_attempt<F>(
    paths: &LauncherPaths,
    request: MutationRequest,
    update: F,
) -> Result<ControlState>
where
    F: FnOnce(&mut ControlState, AttemptRecord, String) -> Result<()>,
{
    cas_update(
        &paths.control,
        request.expected_revision,
        request.expected_executor_epoch,
        |state| {
            let attempt = state.activation.attempt.clone().ok_or_else(|| {
                LauncherError::Conflict("there is no active activation".to_string())
            })?;
            if attempt.attempt_id != request.activation_id {
                return Err(LauncherError::Conflict(
                    "activationId does not match active activation".to_string(),
                ));
            }
            update(state, attempt, request.reason)
        },
    )
}

fn replay_rolled_back_mutation(
    paths: &LauncherPaths,
    request: &MutationRequest,
) -> Result<Option<ControlState>> {
    let Some(state) = ControlState::load(&paths.control)? else {
        return Ok(None);
    };
    let replay = state
        .activation
        .receipt
        .as_ref()
        .is_some_and(|receipt| {
            receipt.attempt_id == request.activation_id
                && receipt.outcome == ActivationOutcome::RolledBack
        });
    Ok(replay.then_some(state))
}

fn load_selected(state: &ControlState, target: &CapsuleTarget) -> Result<CapsuleRecord> {
    record_from_ref(state.selected.capsule(), target)
}

fn record_from_ref(capsule: &CapsuleRef, target: &CapsuleTarget) -> Result<CapsuleRecord> {
    let record = load_and_verify_capsule(&capsule.root, target)?;
    if record.release_id != capsule.release_id || record.executable != capsule.entrypoint {
        return Err(LauncherError::Conflict(
            "control capsule reference does not match verified capsule".to_string(),
        ));
    }
    Ok(record)
}

fn capsule_ref(record: &CapsuleRecord) -> CapsuleRef {
    CapsuleRef {
        release_id: record.release_id.clone(),
        root: record.root.clone(),
        entrypoint: record.executable.clone(),
        metadata: record.manifest.metadata.clone(),
    }
}

fn load_target(capsule: &CapsuleRef) -> Result<CapsuleTarget> {
    let bytes = std::fs::read(capsule.root.join("capsule.json"))
        .map_err(|error| crate::io_error("read capsule target", error))?;
    let manifest: crate::CapsuleManifest = serde_json::from_slice(&bytes)
        .map_err(|error| crate::json_error("parse capsule target", error))?;
    Ok(manifest.target)
}

fn write_failure_projection(paths: &LauncherPaths, state: &ControlState) -> Result<()> {
    write_failure_projection_with_code(paths, state, "activation_rolled_back")
}

fn ensure_pending_failure_evidence(
    paths: &LauncherPaths,
    state: &ControlState,
) -> Result<()> {
    let existing =
        crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)?;
    match state.activation.phase {
        ActivationPhase::EvidencePending => {
            if existing.is_some() {
                return Ok(());
            }
            let code = state
                .activation
                .attempt
                .as_ref()
                .and_then(|attempt| attempt.annotations.get("failureCode"))
                .map(String::as_str)
                .unwrap_or("activation_rolled_back");
            write_failure_projection_with_code(paths, state, code)
        }
        ActivationPhase::Blocked => {
            let failure = state
                .activation
                .blocked
                .as_ref()
                .ok_or_else(|| {
                    LauncherError::Conflict(
                        "blocked activation has no durable failure projection".to_string(),
                    )
                })?
                .failure
                .clone();
            if existing.as_ref() == Some(&failure) {
                Ok(())
            } else {
                crate::control::write_json_atomic(&paths.failure_evidence, &failure)
            }
        }
        _ => Ok(()),
    }
}

fn write_failure_projection_with_code(
    paths: &LauncherPaths,
    state: &ControlState,
    code: &str,
) -> Result<()> {
    let failure = build_failure_projection(paths, state, code)?;
    crate::control::write_json_atomic(&paths.failure_evidence, &failure)
}

fn build_failure_projection(
    paths: &LauncherPaths,
    state: &ControlState,
    code: &str,
) -> Result<FailureProjection> {
    let attempt = state.activation.attempt.as_ref().ok_or_else(|| {
        LauncherError::Conflict("failure projection has no activation attempt".to_string())
    })?;
    Ok(FailureProjection {
        activation_id: attempt.attempt_id.clone(),
        release_id: attempt.candidate.release_id.clone(),
        occurred_at: rfc3339_now(),
        fallback_release_id: Some(state.selected.capsule().release_id.clone()),
        code: code.to_string(),
        message: state
            .activation
            .winner
            .as_ref()
            .map(|winner| winner.reason.clone())
            .unwrap_or_else(|| "activation rolled back".to_string()),
        failed: Some(attempt.candidate.clone()),
        fallback: Some(state.selected.clone()),
        evidence_path: Some(paths.failure_evidence.clone()),
        details: BTreeMap::from([
            ("entrypoint".to_string(), attempt.candidate.entrypoint.display().to_string()),
            ("releaseId".to_string(), attempt.candidate.release_id.clone()),
        ]),
    })
}

fn validate_prepare(request: &PrepareActivationRequest) -> Result<()> {
    if request.schema_version != REQUEST_SCHEMA_VERSION {
        return Err(LauncherError::InvalidRequest(format!(
            "unsupported request schema {}",
            request.schema_version
        )));
    }
    if request.activation_id.is_empty() || request.release_id.is_empty() {
        return Err(LauncherError::InvalidRequest(
            "activationId and releaseId are required".to_string(),
        ));
    }
    Ok(())
}

fn unix_time_ms() -> u64 {
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn rfc3339_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ActivationRecord;
    use crate::control::TrustedSeed;

    fn capsule(id: &str, root: &Path) -> CapsuleRef {
        let capsule_root = root.join(id);
        CapsuleRef {
            release_id: format!("sha256:{id:0>64}"),
            root: capsule_root.clone(),
            entrypoint: capsule_root.join("bin/runtime"),
            metadata: serde_json::Value::Null,
        }
    }

    fn evidence_state(root: &Path, phase: ActivationPhase) -> ControlState {
        let seed = capsule("1", root);
        let candidate = capsule("2", root);
        let selected = SelectedRuntime::seed(seed.clone());
        ControlState {
            schema_version: crate::control::CONTROL_SCHEMA_VERSION,
            revision: 7,
            executor_epoch: 3,
            trusted_seed: TrustedSeed {
                capsule: seed.clone(),
                trust_anchor: root.join("trust-anchor"),
                metadata: serde_json::Value::Null,
            },
            external_current: None,
            external_previous: None,
            selected: selected.clone(),
            active_launch: None,
            activation: ActivationRecord {
                phase,
                attempt: Some(AttemptRecord {
                    attempt_id: "activation-1".to_string(),
                    candidate: candidate.clone(),
                    previous: selected.clone(),
                    previous_external_current: None,
                    previous_external_previous: None,
                    started_at_unix_ms: 1,
                    launch_instance_id: None,
                    spawn_attempt_id: None,
                    guard_registration: None,
                    payload_registration: None,
                    start_authorization: None,
                    ready_expectation: None,
                    known_descendants: Vec::new(),
                    observation_deadline_unix_ms: None,
                    annotations: BTreeMap::from([(
                        "failureCode".to_string(),
                        "activation_rolled_back".to_string(),
                    )]),
                }),
                winner: Some(WinnerRecord {
                    selected: selected.clone(),
                    decided_at_unix_ms: 2,
                    reason: "candidate failed".to_string(),
                }),
                cleanup: CleanupRecord::default(),
                receipt: Some(ActivationReceipt {
                    attempt_id: "activation-1".to_string(),
                    candidate_release_id: candidate.release_id.clone(),
                    outcome: ActivationOutcome::RolledBack,
                    selected,
                    completed_at_unix_ms: 3,
                    reason: Some("candidate failed".to_string()),
                }),
                blocked: None,
            },
        }
    }

    fn committed_candidate_state(root: &Path, phase: ActivationPhase) -> ControlState {
        let mut state = evidence_state(root, phase);
        let candidate = state
            .activation
            .attempt
            .as_ref()
            .expect("attempt")
            .candidate
            .clone();
        let previous = state.selected.clone();
        let guard = ProcessIdentity {
            pid: i32::MAX - 2,
            start_identity: 1001,
        };
        let parent = ProcessIdentity {
            pid: i32::MAX - 3,
            start_identity: 1000,
        };
        let payload = ProcessIdentity {
            pid: i32::MAX - 1,
            start_identity: 1002,
        };
        let guard_registration = GuardRegistration {
            protocol_version: guard::GUARD_PROTOCOL_VERSION,
            release_id: candidate.release_id.clone(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            guard,
            parent,
        };
        let payload_registration = PayloadRegistration {
            protocol_version: guard::GUARD_PROTOCOL_VERSION,
            release_id: candidate.release_id.clone(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            payload,
            process_group: crate::process::ProcessGroupRecord {
                leader: payload,
                pgid: payload.pid,
            },
            guard_registration_digest: guard_registration.digest().expect("guard digest"),
        };
        let authorization = StartAuthorization {
            protocol_version: guard::GUARD_PROTOCOL_VERSION,
            release_id: candidate.release_id.clone(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            payload,
            payload_registration_digest: payload_registration
                .digest()
                .expect("payload digest"),
        };
        let expectation = ReadyBearer::bind(
            candidate.release_id.clone(),
            "launch-1".to_string(),
            "spawn-1".to_string(),
            payload,
            "token".to_string(),
        )
        .expect("ready bearer")
        .expectation;
        let attempt_id = {
            let attempt = state.activation.attempt.as_mut().expect("attempt");
            attempt.previous = previous;
            attempt.launch_instance_id = Some("launch-1".to_string());
            attempt.spawn_attempt_id = Some("spawn-1".to_string());
            attempt.guard_registration = Some(guard_registration);
            attempt.payload_registration = Some(payload_registration);
            attempt.start_authorization = Some(authorization);
            attempt.ready_expectation = Some(expectation);
            attempt.known_descendants = vec![payload];
            attempt.observation_deadline_unix_ms = Some(10);
            attempt.attempt_id.clone()
        };
        state.external_current = Some(candidate.clone());
        state.selected = SelectedRuntime::external(candidate.clone());
        state.activation.winner = Some(WinnerRecord {
            selected: state.selected.clone(),
            decided_at_unix_ms: 2,
            reason: "candidate observed".to_string(),
        });
        state.activation.receipt = Some(ActivationReceipt {
            attempt_id,
            candidate_release_id: candidate.release_id,
            outcome: ActivationOutcome::Committed,
            selected: state.selected.clone(),
            completed_at_unix_ms: 3,
            reason: None,
        });
        state
    }

    fn attach_active_launch(
        mut state: ControlState,
    ) -> (ControlState, GuardRegistration, PayloadRegistration) {
        let attempt = state.activation.attempt.as_ref().expect("attempt");
        let guard_registration = attempt.guard_registration.clone().expect("guard");
        let payload_registration = attempt.payload_registration.clone().expect("payload");
        let start_authorization = attempt.start_authorization.clone();
        let ready_expectation = attempt.ready_expectation.clone();
        state.active_launch = Some(ActiveLaunchRecord {
            selected: state.selected.clone(),
            launch_instance_id: guard_registration.launch_instance_id.clone(),
            spawn_attempt_id: guard_registration.spawn_attempt_id.clone(),
            phase: ActiveLaunchPhase::Running,
            guard_registration: Some(guard_registration.clone()),
            payload_registration: Some(payload_registration.clone()),
            start_authorization,
            ready_expectation,
        });
        (state, guard_registration, payload_registration)
    }

    #[test]
    fn production_failure_writer_matches_recovery_contract() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = evidence_state(temp.path(), ActivationPhase::EvidencePending);
        write_failure_projection(&paths, &state).expect("write projection");
        let value: serde_json::Value =
            crate::control::read_json_if_exists(&paths.failure_evidence)
                .expect("read evidence")
                .expect("evidence");
        assert_eq!(value["activationId"], "activation-1");
        assert_eq!(value["releaseId"], capsule("2", temp.path()).release_id);
        assert_eq!(value["fallbackReleaseId"], capsule("1", temp.path()).release_id);
        let occurred_at = value["occurredAt"].as_str().expect("occurredAt");
        assert!(occurred_at.ends_with('Z'));
        assert_eq!(occurred_at.len(), "2026-09-09T00:00:00.000Z".len());
        chrono::DateTime::parse_from_rfc3339(occurred_at).expect("RFC3339");
    }

    #[test]
    fn evidence_cleanup_phase_recovers_after_file_deletion() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = evidence_state(temp.path(), ActivationPhase::EvidenceCleanup);
        state.validate().expect("valid cleanup state");
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write control");
        let idle = ack_failure(
            &paths,
            MutationRequest {
                activation_id: "activation-1".to_string(),
                expected_revision: state.revision,
                expected_executor_epoch: state.executor_epoch,
                reason: String::new(),
            },
        )
        .expect("resume ack");
        assert_eq!(idle.activation.phase, ActivationPhase::Idle);
        let repeated = ack_failure(
            &paths,
            MutationRequest {
                activation_id: "activation-1".to_string(),
                expected_revision: idle.revision,
                expected_executor_epoch: idle.executor_epoch,
                reason: String::new(),
            },
        )
        .expect("idempotent ack");
        assert_eq!(repeated, idle);
    }

    #[test]
    fn every_payload_receives_launcher_and_recovery_environment() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        let launcher = temp.path().join("Runtime.app/Contents/MacOS/launcher");
        let ready = temp.path().join("ready.json");
        let environment = build_payload_environment(
            &paths,
            &launcher,
            &ready,
            "token",
            "release",
            "launch",
            "spawn",
        );
        assert_eq!(environment["RUNTIME_CAPSULE_READY_PROTOCOL"], "1");
        assert_eq!(
            environment["RUNTIME_CAPSULE_FAILURE_EVIDENCE_PATH"],
            paths.failure_evidence.display().to_string()
        );
        assert_eq!(
            environment["RUNTIME_CAPSULE_LAUNCHER_PATH"],
            launcher.display().to_string()
        );
        assert_eq!(
            environment["RUNTIME_CAPSULE_LAUNCHER_HOME"],
            paths.root.display().to_string()
        );
        assert!(!environment.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!environment.contains_key(guard::PARENT_LIVENESS_FD_ENV));
    }

    #[test]
    fn fallback_launch_does_not_reuse_candidate_protocol_namespace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut state = evidence_state(temp.path(), ActivationPhase::EvidencePending);
        let attempt = state.activation.attempt.as_mut().expect("attempt");
        attempt.launch_instance_id = Some("candidate-launch".to_string());
        attempt.spawn_attempt_id = Some("candidate-spawn".to_string());
        let fallback_release = state.selected.capsule().release_id.clone();
        let (is_candidate, launch, spawn) = launch_protocol_identity(
            &state,
            &fallback_release,
            "activation-1",
            "0123456789abcdef0123456789abcdef",
        )
        .expect("fallback identity");
        assert!(!is_candidate);
        assert_ne!(launch, "candidate-launch");
        assert_ne!(spawn, "candidate-spawn");
    }

    #[test]
    fn same_release_activation_relaunches_without_rotating_generations() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let seed = capsule("1", temp.path());
        let current = capsule("2", temp.path());
        let previous = capsule("3", temp.path());
        let mut state = evidence_state(temp.path(), ActivationPhase::Prepared);
        state.selected = SelectedRuntime::external(current.clone());
        state.external_current = Some(current.clone());
        state.external_previous = Some(previous.clone());
        state.activation.winner = None;
        state.activation.receipt = None;
        let selected = state.selected.clone();
        let attempt = state.activation.attempt.as_mut().expect("attempt");
        attempt.candidate = current.clone();
        attempt.previous = selected;
        attempt.previous_external_current = Some(current.clone());
        attempt.previous_external_previous = Some(previous.clone());
        state.trusted_seed.capsule = seed;
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");

        let launched =
            authorize_candidate_after_old_stopped(&paths, state).expect("authorize relaunch");
        assert_eq!(launched.external_current, Some(current.clone()));
        assert_eq!(launched.external_previous, Some(previous));
        assert_eq!(launched.selected, SelectedRuntime::external(current));
        assert!(launched.activation.cleanup.pending.is_empty());
        assert_eq!(launched.activation.phase, ActivationPhase::SpawnPlanned);
    }

    #[test]
    fn restoring_replay_uses_the_original_generation_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let current = capsule("3", temp.path());
        let previous = capsule("4", temp.path());
        let candidate = capsule("2", temp.path());
        let mut state = evidence_state(temp.path(), ActivationPhase::Restoring);
        state.selected = SelectedRuntime::external(current.clone());
        state.external_current = Some(current.clone());
        state.external_previous = Some(previous.clone());
        let selected = state.selected.clone();
        let attempt_id = {
            let attempt = state.activation.attempt.as_mut().expect("attempt");
            attempt.candidate = candidate.clone();
            attempt.previous = selected.clone();
            attempt.previous_external_current = Some(current.clone());
            attempt.previous_external_previous = Some(previous.clone());
            attempt.attempt_id.clone()
        };
        state.activation.winner = Some(WinnerRecord {
            selected: selected.clone(),
            decided_at_unix_ms: 2,
            reason: "rollback".to_string(),
        });
        state.activation.receipt = Some(ActivationReceipt {
            attempt_id,
            candidate_release_id: candidate.release_id.clone(),
            outcome: ActivationOutcome::RolledBack,
            selected,
            completed_at_unix_ms: 3,
            reason: Some("rollback".to_string()),
        });
        state.activation.cleanup.pending = vec![candidate.clone()];
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");

        reconcile_interrupted(
            &paths,
            &CapsuleTarget {
                os: "toy-os".to_string(),
                arch: "toy-arch".to_string(),
            },
            &mut state,
        )
        .expect("reconcile restoring");
        let restored = execute_rollback(&paths, state).expect("replay rollback");
        assert_eq!(restored.external_current, Some(current.clone()));
        assert_eq!(restored.external_previous, Some(previous));
        assert_eq!(restored.selected, SelectedRuntime::external(current));
        assert_eq!(restored.activation.cleanup.pending, vec![candidate]);
    }

    #[test]
    fn terminal_receipt_replays_prepare_and_cancel_without_restaging() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let candidate = capsule("2", temp.path());
        let mut state = evidence_state(temp.path(), ActivationPhase::Idle);
        state.activation.attempt = None;
        state.activation.winner = None;
        state.activation.cleanup.pending.clear();
        state.activation.receipt = Some(ActivationReceipt {
            attempt_id: "activation-1".to_string(),
            candidate_release_id: candidate.release_id.clone(),
            outcome: ActivationOutcome::RolledBack,
            selected: state.selected.clone(),
            completed_at_unix_ms: 3,
            reason: Some("cancelled".to_string()),
        });
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");

        let prepared = prepare_activation(
            &paths,
            PrepareActivationRequest {
                schema_version: REQUEST_SCHEMA_VERSION,
                activation_id: "activation-1".to_string(),
                release_id: candidate.release_id,
                expected_revision: 0,
                expected_executor_epoch: 0,
                target: CapsuleTarget {
                    os: "toy-os".to_string(),
                    arch: "toy-arch".to_string(),
                },
                reason: "retry".to_string(),
            },
        )
        .expect("prepare replay");
        assert_eq!(
            prepared.disposition,
            PrepareActivationDisposition::TerminalFailed
        );
        assert_eq!(prepared.control, state);

        let cancelled = cancel_activation(
            &paths,
            MutationRequest {
                activation_id: "activation-1".to_string(),
                expected_revision: 0,
                expected_executor_epoch: 0,
                reason: "retry".to_string(),
            },
        )
        .expect("cancel replay");
        assert_eq!(cancelled, state);
    }

    #[test]
    fn cancelling_same_release_prepare_keeps_retained_external_capsule() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let current = capsule("2", temp.path());
        let mut state = evidence_state(temp.path(), ActivationPhase::Prepared);
        state.selected = SelectedRuntime::external(current.clone());
        state.external_current = Some(current.clone());
        state.activation.winner = None;
        state.activation.receipt = None;
        let selected = state.selected.clone();
        let attempt = state.activation.attempt.as_mut().expect("attempt");
        attempt.candidate = current.clone();
        attempt.previous = selected;
        attempt.previous_external_current = Some(current.clone());
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");

        let cancelled = cancel_activation(
            &paths,
            MutationRequest {
                activation_id: "activation-1".to_string(),
                expected_revision: state.revision,
                expected_executor_epoch: state.executor_epoch,
                reason: "cancel".to_string(),
            },
        )
        .expect("cancel retained capsule");
        assert_eq!(cancelled.external_current, Some(current.clone()));
        assert_eq!(
            cancelled.selected,
            SelectedRuntime::external(current)
        );
        assert!(cancelled.activation.cleanup.pending.is_empty());
        assert_eq!(cancelled.activation.phase, ActivationPhase::Idle);
        assert_eq!(
            cancelled
                .activation
                .receipt
                .as_ref()
                .map(|receipt| receipt.outcome),
            Some(ActivationOutcome::RolledBack)
        );
    }

    #[test]
    fn commit_cleanup_crash_requires_ready_relaunch_before_cleanup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let mut state =
            committed_candidate_state(temp.path(), ActivationPhase::CommitCleanup);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");

        reconcile_interrupted(
            &paths,
            &CapsuleTarget {
                os: "toy-os".to_string(),
                arch: "toy-arch".to_string(),
            },
            &mut state,
        )
        .expect("recover commit");
        assert_eq!(state.activation.phase, ActivationPhase::CommitRelaunch);
        assert!(state.activation.receipt.is_some());
    }

    #[test]
    fn seed_replacement_preserves_a_durable_external_winner() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = committed_candidate_state(
            temp.path(),
            ActivationPhase::CommitRelaunch,
        );
        let selected = state.selected.clone();
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");
        let new_seed_capsule = capsule("9", temp.path());
        let new_seed = TrustedSeed {
            capsule: new_seed_capsule.clone(),
            trust_anchor: temp.path().join("new-trust-anchor"),
            metadata: serde_json::Value::Null,
        };

        let rebound = rebind_seed(
            &paths,
            state.revision,
            state.executor_epoch,
            new_seed,
        )
        .expect("rebind Seed");
        assert_eq!(rebound.selected, selected);
        assert_eq!(rebound.external_current, Some(selected.capsule().clone()));
        assert!(matches!(
            &rebound
                .activation
                .attempt
                .as_ref()
                .expect("attempt")
                .previous,
            SelectedRuntime::Seed { capsule } if capsule == &new_seed_capsule
        ));
        assert_eq!(
            rebound.activation.phase,
            ActivationPhase::CommitRelaunch
        );
    }

    #[test]
    fn seed_replacement_during_activation_restores_external_generation_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let old_seed = capsule("1", temp.path());
        let candidate = capsule("2", temp.path());
        let old_current = capsule("3", temp.path());
        let old_previous = capsule("4", temp.path());
        let mut state = evidence_state(temp.path(), ActivationPhase::SpawnPlanned);
        state.trusted_seed.capsule = old_seed.clone();
        state.selected = SelectedRuntime::external(candidate.clone());
        state.external_current = Some(candidate.clone());
        state.external_previous = Some(old_current.clone());
        state.activation.winner = None;
        state.activation.receipt = None;
        state.activation.cleanup.pending = vec![old_previous.clone()];
        let attempt = state.activation.attempt.as_mut().expect("attempt");
        attempt.candidate = candidate.clone();
        attempt.previous = SelectedRuntime::seed(old_seed);
        attempt.previous_external_current = Some(old_current.clone());
        attempt.previous_external_previous = Some(old_previous.clone());
        attempt.launch_instance_id = Some("launch-1".to_string());
        attempt.spawn_attempt_id = Some("spawn-1".to_string());
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");
        let new_seed_capsule = capsule("9", temp.path());
        let new_seed = TrustedSeed {
            capsule: new_seed_capsule.clone(),
            trust_anchor: temp.path().join("new-trust-anchor"),
            metadata: serde_json::Value::Null,
        };

        let rebound = rebind_seed(
            &paths,
            state.revision,
            state.executor_epoch,
            new_seed,
        )
        .expect("rebind Seed");
        assert_eq!(
            rebound.selected,
            SelectedRuntime::seed(new_seed_capsule)
        );
        assert_eq!(rebound.external_current, Some(old_current));
        assert_eq!(rebound.external_previous, Some(old_previous));
        assert_eq!(rebound.activation.cleanup.pending, vec![candidate]);
        assert_eq!(
            rebound
                .activation
                .attempt
                .as_ref()
                .and_then(|attempt| attempt.annotations.get("failureCode"))
                .map(String::as_str),
            Some("seed_replaced")
        );
        assert_eq!(
            rebound.activation.phase,
            ActivationPhase::EvidencePending
        );
    }

    #[test]
    fn request_rollback_cannot_reverse_a_durable_commit() {
        for phase in [
            ActivationPhase::CommitDecided,
            ActivationPhase::CommitRelaunch,
            ActivationPhase::CommitCleanup,
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let paths = LauncherPaths::new(temp.path().join("state"));
            paths.ensure().expect("paths");
            let state = committed_candidate_state(temp.path(), phase);
            let winner = state.activation.winner.clone();
            let receipt = state.activation.receipt.clone();
            crate::control::write_json_atomic(&paths.control.control, &state)
                .expect("control");

            let error = request_rollback(
                &paths,
                MutationRequest {
                    activation_id: "activation-1".to_string(),
                    expected_revision: state.revision,
                    expected_executor_epoch: state.executor_epoch,
                    reason: "late rollback".to_string(),
                },
            )
            .expect_err("durable commit must reject rollback");
            assert!(error.to_string().contains("durably committed"));
            let unchanged = ControlState::load(&paths.control)
                .expect("load")
                .expect("control");
            assert_eq!(unchanged.activation.phase, phase);
            assert_eq!(unchanged.activation.winner, winner);
            assert_eq!(unchanged.activation.receipt, receipt);
        }
    }

    #[test]
    fn repeated_rollback_replays_terminal_receipt_before_ack() {
        for phase in [
            ActivationPhase::EvidencePending,
            ActivationPhase::EvidenceCleanup,
        ] {
            let temp = tempfile::tempdir().expect("tempdir");
            let paths = LauncherPaths::new(temp.path().join("state"));
            paths.ensure().expect("paths");
            let state = evidence_state(temp.path(), phase);
            crate::control::write_json_atomic(&paths.control.control, &state)
                .expect("control");

            let replayed = request_rollback(
                &paths,
                MutationRequest {
                    activation_id: "activation-1".to_string(),
                    expected_revision: 0,
                    expected_executor_epoch: 0,
                    reason: "retry".to_string(),
                },
            )
            .expect("terminal replay");
            assert_eq!(replayed, state);
        }
    }

    #[test]
    fn committed_relaunch_failure_blocks_without_changing_winner() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state =
            committed_candidate_state(temp.path(), ActivationPhase::CommitRelaunch);
        let winner = state.activation.winner.clone();
        let receipt = state.activation.receipt.clone();
        let selected = state.selected.clone();
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");

        let blocked = fence_committed_relaunch_failure(
            &paths,
            &state,
            "readiness failed",
        )
        .expect("block relaunch");
        assert_eq!(blocked.activation.phase, ActivationPhase::Blocked);
        assert_eq!(blocked.activation.winner, winner);
        assert_eq!(blocked.activation.receipt, receipt);
        assert_eq!(blocked.selected, selected);
        assert_eq!(
            blocked
                .activation
                .blocked
                .as_ref()
                .map(|blocked| blocked.failure.code.as_str()),
            Some("committed_relaunch_blocked")
        );

        let error = request_rollback(
            &paths,
            MutationRequest {
                activation_id: "activation-1".to_string(),
                expected_revision: blocked.revision,
                expected_executor_epoch: blocked.executor_epoch,
                reason: "late rollback".to_string(),
            },
        )
        .expect_err("blocked committed winner must reject rollback");
        assert!(error.to_string().contains("durably committed"));
        let unchanged = ControlState::load(&paths.control)
            .expect("load")
            .expect("control");
        assert_eq!(unchanged, blocked);
    }

    #[test]
    fn durable_guard_violation_reconciles_to_acknowledgeable_blocked_state() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let (state, guard_registration, payload_registration) = attach_active_launch(
            committed_candidate_state(temp.path(), ActivationPhase::CommitRelaunch),
        );
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");
        let report_path = paths
            .attempts
            .join(&guard_registration.spawn_attempt_id)
            .with_extension("exit-report.json");
        guard::write_protocol_file(
            &report_path,
            &GuardExitReport::ContractViolated {
                release_id: guard_registration.release_id.clone(),
                launch_instance_id: guard_registration.launch_instance_id.clone(),
                spawn_attempt_id: guard_registration.spawn_attempt_id.clone(),
                payload: payload_registration.payload,
                observed_process: ProcessIdentity {
                    pid: 103,
                    start_identity: 1003,
                },
                observed_pgid: 103,
                message: "contract violated".to_string(),
            },
        )
        .expect("write violation");

        let error = reconcile_active_launch(&paths, state)
            .expect_err("durable violation must block");
        assert!(error.to_string().contains("contract violated"));
        let blocked = ControlState::load(&paths.control)
            .expect("load")
            .expect("control");
        assert_eq!(blocked.activation.phase, ActivationPhase::Blocked);
        assert!(blocked.active_launch.is_none());
        assert!(paths.failure_evidence.exists());

        let idle = ack_failure(
            &paths,
            MutationRequest {
                activation_id: "activation-1".to_string(),
                expected_revision: blocked.revision,
                expected_executor_epoch: blocked.executor_epoch,
                reason: "ack".to_string(),
            },
        )
        .expect("acknowledge");
        assert_eq!(idle.activation.phase, ActivationPhase::Idle);
        assert!(idle.active_launch.is_none());
    }

    #[test]
    fn active_committed_launch_error_clears_and_blocks_in_one_reconcile() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let (state, _, _) = attach_active_launch(
            committed_candidate_state(temp.path(), ActivationPhase::CommitRelaunch),
        );
        let initial_revision = state.revision;
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");
        let error = reconcile_active_launch_with_block(
            &paths,
            state,
            "committed relaunch failed",
        )
        .expect_err("pending failure must stop the launcher");
        assert!(error.to_string().contains("committed relaunch failed"));
        let blocked = ControlState::load(&paths.control)
            .expect("load")
            .expect("control");
        assert_eq!(blocked.revision, initial_revision + 1);
        assert_eq!(blocked.activation.phase, ActivationPhase::Blocked);
        assert!(blocked.active_launch.is_none());
        assert_eq!(
            blocked
                .activation
                .blocked
                .as_ref()
                .map(|blocked| blocked.failure.message.as_str()),
            Some("committed relaunch failed")
        );
    }

    #[test]
    fn recovery_restores_payload_registration_before_validating_terminal_report() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let (mut state, guard_registration, payload_registration) = attach_active_launch(
            committed_candidate_state(temp.path(), ActivationPhase::CommitRelaunch),
        );
        let active = state.active_launch.as_mut().expect("active");
        active.phase = ActiveLaunchPhase::GuardRegistered;
        active.payload_registration = None;
        active.start_authorization = None;
        active.ready_expectation = None;
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");
        let payload_path = paths
            .attempts
            .join(&guard_registration.spawn_attempt_id)
            .with_extension("payload-registration.json");
        guard::write_protocol_file(&payload_path, &payload_registration)
            .expect("write payload registration");
        let report_path = paths
            .attempts
            .join(&guard_registration.spawn_attempt_id)
            .with_extension("exit-report.json");
        guard::write_protocol_file(
            &report_path,
            &GuardExitReport::ContractViolated {
                release_id: guard_registration.release_id.clone(),
                launch_instance_id: guard_registration.launch_instance_id.clone(),
                spawn_attempt_id: guard_registration.spawn_attempt_id.clone(),
                payload: payload_registration.payload,
                observed_process: ProcessIdentity {
                    pid: i32::MAX - 4,
                    start_identity: 1004,
                },
                observed_pgid: i32::MAX - 4,
                message: "recovered violation".to_string(),
            },
        )
        .expect("write violation");

        let error = reconcile_active_launch(&paths, state)
            .expect_err("recovered violation must block");
        assert!(error.to_string().contains("recovered violation"));
        let blocked = ControlState::load(&paths.control)
            .expect("load")
            .expect("control");
        assert_eq!(blocked.activation.phase, ActivationPhase::Blocked);
        assert!(blocked.active_launch.is_none());
    }

    #[test]
    fn malformed_terminal_report_is_fenced_after_cleanup() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let (state, guard_registration, payload_registration) = attach_active_launch(
            committed_candidate_state(temp.path(), ActivationPhase::CommitRelaunch),
        );
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");
        let report_path = paths
            .attempts
            .join(&guard_registration.spawn_attempt_id)
            .with_extension("exit-report.json");
        guard::write_protocol_file(
            &report_path,
            &GuardExitReport::Exited {
                release_id: guard_registration.release_id,
                launch_instance_id: guard_registration.launch_instance_id,
                spawn_attempt_id: guard_registration.spawn_attempt_id,
                payload: payload_registration.payload,
                raw_wait_status: 0,
                parent_liveness_lost: false,
                cleanup_evidence: CleanupEvidence::CooperativeObservedEmpty,
            },
        )
        .expect("write report");
        std::fs::write(&report_path, b"{not-json").expect("corrupt report");

        let error = reconcile_active_launch(&paths, state)
            .expect_err("malformed report must block");
        assert!(error.to_string().contains("parse"));
        let blocked = ControlState::load(&paths.control)
            .expect("load")
            .expect("control");
        assert_eq!(blocked.activation.phase, ActivationPhase::Blocked);
        assert!(blocked.active_launch.is_none());
        assert!(paths.failure_evidence.exists());
    }

    #[test]
    fn guard_terminal_report_identity_mismatch_is_blocked() {
        let payload = ProcessIdentity {
            pid: 100,
            start_identity: 1000,
        };
        let report = GuardExitReport::Exited {
            release_id: "wrong-release".to_string(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            payload,
            raw_wait_status: 0,
            parent_liveness_lost: false,
            cleanup_evidence: CleanupEvidence::CooperativeObservedEmpty,
        };
        let error = validate_guard_exit_report(
            &report,
            "expected-release",
            "launch-1",
            "spawn-1",
            payload,
        )
        .expect_err("identity mismatch");
        assert!(error.to_string().contains("does not match"));

        let violation = GuardExitReport::ContractViolated {
            release_id: "expected-release".to_string(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "wrong-spawn".to_string(),
            payload,
            observed_process: ProcessIdentity {
                pid: 101,
                start_identity: 1001,
            },
            observed_pgid: 101,
            message: "violation".to_string(),
        };
        let error = validate_guard_exit_report(
            &violation,
            "expected-release",
            "launch-1",
            "spawn-1",
            payload,
        )
        .expect_err("violation identity mismatch");
        assert!(error.to_string().contains("does not match"));
    }

    #[test]
    fn live_guard_reader_rejects_identity_mismatch_without_waiting_for_guard_exit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let report_path = temp.path().join("exit-report.json");
        let mut guard_child = std::process::Command::new("sleep")
            .arg("2")
            .spawn()
            .expect("spawn guard stand-in");
        let guard_pid = i32::try_from(guard_child.id()).expect("guard pid");
        let guard_identity = ProcessIdentity::observe(guard_pid)
            .expect("observe guard")
            .expect("live guard");
        let parent_pid = i32::try_from(std::process::id()).expect("parent pid");
        let parent_identity = ProcessIdentity::observe(parent_pid)
            .expect("observe parent")
            .expect("live parent");
        let payload = ProcessIdentity {
            pid: i32::MAX - 1,
            start_identity: 1002,
        };
        let guard_registration = GuardRegistration {
            protocol_version: guard::GUARD_PROTOCOL_VERSION,
            release_id: "expected-release".to_string(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            guard: guard_identity,
            parent: parent_identity,
        };
        let payload_registration = PayloadRegistration {
            protocol_version: guard::GUARD_PROTOCOL_VERSION,
            release_id: guard_registration.release_id.clone(),
            launch_instance_id: guard_registration.launch_instance_id.clone(),
            spawn_attempt_id: guard_registration.spawn_attempt_id.clone(),
            payload,
            process_group: crate::process::ProcessGroupRecord {
                leader: payload,
                pgid: payload.pid,
            },
            guard_registration_digest: guard_registration.digest().expect("guard digest"),
        };
        guard::write_protocol_file(
            &report_path,
            &GuardExitReport::Exited {
                release_id: "wrong-release".to_string(),
                launch_instance_id: guard_registration.launch_instance_id.clone(),
                spawn_attempt_id: guard_registration.spawn_attempt_id.clone(),
                payload,
                raw_wait_status: 0,
                parent_liveness_lost: false,
                cleanup_evidence: CleanupEvidence::CooperativeObservedEmpty,
            },
        )
        .expect("write report");

        let result = read_guard_exit_status(
            &report_path,
            &mut guard_child,
            &guard_registration,
            &payload_registration,
        );
        let guard_was_running = guard_child.try_wait().expect("observe child").is_none();
        if guard_was_running {
            guard_child.kill().expect("kill guard stand-in");
        }
        guard_child.wait().expect("reap guard stand-in");

        let error = result.expect_err("identity mismatch must fail before guard exit");
        assert!(error.to_string().contains("does not match"));
        assert!(guard_was_running);
    }

    #[test]
    fn live_guard_reader_waits_for_guard_after_valid_exited_report() {
        let temp = tempfile::tempdir().expect("tempdir");
        let report_path = temp.path().join("exit-report.json");
        let mut guard_child = std::process::Command::new("sleep")
            .arg("0.2")
            .spawn()
            .expect("spawn guard stand-in");
        let guard_pid = i32::try_from(guard_child.id()).expect("guard pid");
        let guard_identity = ProcessIdentity::observe(guard_pid)
            .expect("observe guard")
            .expect("live guard");
        let parent_pid = i32::try_from(std::process::id()).expect("parent pid");
        let parent_identity = ProcessIdentity::observe(parent_pid)
            .expect("observe parent")
            .expect("live parent");
        let payload = ProcessIdentity {
            pid: i32::MAX - 1,
            start_identity: 1002,
        };
        let guard_registration = GuardRegistration {
            protocol_version: guard::GUARD_PROTOCOL_VERSION,
            release_id: "release-1".to_string(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            guard: guard_identity,
            parent: parent_identity,
        };
        let payload_registration = PayloadRegistration {
            protocol_version: guard::GUARD_PROTOCOL_VERSION,
            release_id: guard_registration.release_id.clone(),
            launch_instance_id: guard_registration.launch_instance_id.clone(),
            spawn_attempt_id: guard_registration.spawn_attempt_id.clone(),
            payload,
            process_group: crate::process::ProcessGroupRecord {
                leader: payload,
                pgid: payload.pid,
            },
            guard_registration_digest: guard_registration.digest().expect("guard digest"),
        };
        guard::write_protocol_file(
            &report_path,
            &GuardExitReport::Exited {
                release_id: guard_registration.release_id.clone(),
                launch_instance_id: guard_registration.launch_instance_id.clone(),
                spawn_attempt_id: guard_registration.spawn_attempt_id.clone(),
                payload,
                raw_wait_status: 0,
                parent_liveness_lost: false,
                cleanup_evidence: CleanupEvidence::CooperativeObservedEmpty,
            },
        )
        .expect("write report");

        let status = read_guard_exit_status(
            &report_path,
            &mut guard_child,
            &guard_registration,
            &payload_registration,
        )
        .expect("read valid report")
        .expect("guard exit");
        assert!(status.success());
        assert!(guard_child.try_wait().expect("reaped guard").is_some());
    }

    #[test]
    fn blocked_control_recreates_missing_typed_failure_evidence() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let old_state = evidence_state(temp.path(), ActivationPhase::EvidencePending);
        write_failure_projection(&paths, &old_state).expect("write old projection");
        let old_projection =
            crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
                .expect("read old")
                .expect("old projection");
        let state =
            committed_candidate_state(temp.path(), ActivationPhase::CommitRelaunch);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");
        let blocked = fence_committed_relaunch_failure(&paths, &state, "failed")
            .expect("block");
        let replaced =
            crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
                .expect("read replaced")
                .expect("replaced projection");
        assert_ne!(replaced, old_projection);
        assert_eq!(
            replaced,
            blocked.activation.blocked.as_ref().expect("blocked").failure
        );
        std::fs::remove_file(&paths.failure_evidence).expect("remove projection");

        ensure_pending_failure_evidence(&paths, &blocked)
            .expect("recreate projection");
        let projection =
            crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
                .expect("read")
                .expect("projection");
        assert_eq!(
            projection,
            blocked.activation.blocked.expect("blocked").failure
        );
    }

    #[test]
    fn generation_rotation_does_not_cleanup_a_release_retained_by_seed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let seed = capsule("1", temp.path());
        let current = capsule("3", temp.path());
        let candidate = capsule("4", temp.path());
        let mut state = evidence_state(temp.path(), ActivationPhase::Prepared);
        state.trusted_seed.capsule = seed.clone();
        state.selected = SelectedRuntime::external(current.clone());
        state.external_current = Some(current.clone());
        state.external_previous = Some(seed.clone());
        state.activation.winner = None;
        state.activation.receipt = None;
        let selected = state.selected.clone();
        let attempt = state.activation.attempt.as_mut().expect("attempt");
        attempt.candidate = candidate.clone();
        attempt.previous = selected;
        attempt.previous_external_current = Some(current.clone());
        attempt.previous_external_previous = Some(seed);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");

        let launched =
            authorize_candidate_after_old_stopped(&paths, state).expect("authorize");
        assert_eq!(launched.external_current, Some(candidate.clone()));
        assert_eq!(launched.external_previous, Some(current));
        assert_eq!(
            launched.selected,
            SelectedRuntime::external(candidate)
        );
        assert!(launched.activation.cleanup.pending.is_empty());
    }

    #[test]
    fn prepared_candidate_matching_new_seed_is_not_cleaned() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let mut state = evidence_state(temp.path(), ActivationPhase::Prepared);
        state.activation.winner = None;
        state.activation.receipt = None;
        let candidate = state
            .activation
            .attempt
            .as_ref()
            .expect("attempt")
            .candidate
            .clone();
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("control");
        let new_seed = TrustedSeed {
            capsule: candidate.clone(),
            trust_anchor: temp.path().join("new-trust-anchor"),
            metadata: serde_json::Value::Null,
        };

        let rebound = rebind_seed(
            &paths,
            state.revision,
            state.executor_epoch,
            new_seed,
        )
        .expect("rebind Seed");
        assert_eq!(rebound.selected, SelectedRuntime::seed(candidate));
        assert_eq!(rebound.activation.phase, ActivationPhase::Idle);
        assert!(rebound.activation.cleanup.pending.is_empty());
        assert_eq!(
            rebound
                .activation
                .receipt
                .as_ref()
                .map(|receipt| receipt.outcome),
            Some(ActivationOutcome::RolledBack)
        );
    }
}
