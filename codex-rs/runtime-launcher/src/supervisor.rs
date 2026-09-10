use crate::LauncherError;
use crate::MutationRequest;
use crate::PrepareActivationRequest;
use crate::Result;
use crate::capsule::CapsuleRecord;
use crate::capsule::CapsuleTarget;
use crate::capsule::import_incoming;
use crate::capsule::load_and_verify_capsule;
use crate::capsule::verify_record_for_spawn;
use crate::control::ActivationOutcome;
use crate::control::ActivationPhase;
use crate::control::ActivationReceipt;
use crate::control::ActiveLaunchPhase;
use crate::control::ActiveLaunchRecord;
use crate::control::AttemptRecord;
use crate::control::CapsuleRef;
use crate::control::CleanupRecord;
use crate::control::ControlPaths;
use crate::control::ControlState;
use crate::control::ExecutorEpoch;
use crate::control::FailureProjection;
use crate::control::PayloadRegistration;
use crate::control::SelectedRuntime;
use crate::control::WinnerRecord;
use crate::control::cas_update;
use crate::migration::migrate_legacy_state;
use crate::process::ProcessCleanupError;
use crate::process::ProcessGroupRecord;
use crate::process::ProcessIdentity;
use crate::process::TerminationPolicy;
use crate::process::TerminationTarget;
use crate::process::signal_identity_if_exact;
use crate::process::terminate_and_observe_empty_with_reporter;
use crate::readiness;
use crate::readiness::ReadyBearer;
use crate::seed::discover_seed;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

pub const EXIT_COORDINATED_RESTART: i32 = 75;
const REQUEST_SCHEMA_VERSION: u32 = 1;
const OBSERVATION_WINDOW: Duration = Duration::from_secs(30);

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

struct ExecutorLease {
    _file: File,
}

impl ExecutorLease {
    fn acquire(path: &Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| crate::io_error(format!("open {}", path.display()), error))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            return if matches!(
                error.raw_os_error(),
                Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
            ) {
                Err(LauncherError::Conflict(
                    "another Runtime Capsule Launcher is active".to_string(),
                ))
            } else {
                Err(crate::io_error(format!("lock {}", path.display()), error))
            };
        }
        Ok(Self { _file: file })
    }
}

pub fn prepare_activation(
    paths: &LauncherPaths,
    request: PrepareActivationRequest,
) -> Result<PrepareActivationResult> {
    validate_prepare(&request)?;
    let existing = ControlState::load(&paths.control)?.ok_or_else(|| {
        LauncherError::Conflict("run must initialize the trusted Seed before prepare".to_string())
    })?;
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
    let candidate = capsule_ref(&candidate);
    let control = cas_update(
        &paths.control,
        request.expected_revision,
        request.expected_executor_epoch,
        |state| {
            state.activation.phase = ActivationPhase::Prepared;
            state.activation.attempt = Some(AttemptRecord {
                attempt_id: request.activation_id.clone(),
                candidate,
                previous: state.selected.clone(),
                previous_external_current: state.external_current.clone(),
                previous_external_previous: state.external_previous.clone(),
                started_at_unix_ms: unix_time_ms(),
                launch_instance_id: None,
                spawn_attempt_id: None,
                payload_registration: None,
                ready_expectation: None,
                known_descendants: Vec::new(),
                observation_deadline_unix_ms: None,
                annotations: BTreeMap::from([("reason".to_string(), request.reason.clone())]),
            });
            state.activation.winner = None;
            state.activation.cleanup = CleanupRecord::default();
            state.activation.receipt = None;
            Ok(())
        },
    )?;
    Ok(PrepareActivationResult {
        disposition: PrepareActivationDisposition::Prepared,
        activation_id: request.activation_id,
        release_id: request.release_id,
        control,
    })
}

pub fn cancel_activation(paths: &LauncherPaths, request: MutationRequest) -> Result<ControlState> {
    mutate_attempt(paths, request, |state, attempt, reason| {
        if state.activation.phase != ActivationPhase::Prepared {
            return Err(LauncherError::Conflict(
                "only a prepared activation can be cancelled".to_string(),
            ));
        }
        state.activation.receipt = Some(ActivationReceipt {
            attempt_id: attempt.attempt_id,
            candidate_release_id: attempt.candidate.release_id,
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
        state.activation.attempt = None;
        Ok(())
    })
}

pub fn request_rollback(paths: &LauncherPaths, request: MutationRequest) -> Result<ControlState> {
    mutate_attempt(paths, request, |state, attempt, reason| {
        if matches!(
            state.activation.phase,
            ActivationPhase::CommitDecided
                | ActivationPhase::CommitRelaunch
                | ActivationPhase::CommitCleanup
        ) {
            return Err(LauncherError::Conflict(
                "activation winner is already committed".to_string(),
            ));
        }
        state.activation.phase = ActivationPhase::RollbackDecided;
        state.activation.winner = Some(WinnerRecord {
            selected: attempt.previous,
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

pub fn status(paths: &LauncherPaths) -> Result<Status> {
    let control = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state is not initialized".to_string()))?;
    let failure = crate::control::read_json_if_exists(&paths.failure_evidence)?;
    Ok(Status { control, failure })
}

pub fn run(
    paths: &LauncherPaths,
    outer_bundle: &Path,
    target: CapsuleTarget,
    launcher_path: &Path,
) -> Result<RunOutcome> {
    paths.ensure()?;
    let _executor = ExecutorLease::acquire(&paths.executor_lock)?;
    let (seed, _) = discover_seed(outer_bundle, &target)?;
    migrate_legacy_state(&paths.control)?;
    ControlState::load_or_initialize(&paths.control, seed.clone())?;
    ExecutorEpoch::acquire_and_bump_epoch(&paths.control, seed.clone())?;
    let state = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
    let mut state = reconcile_active_launch(paths, state)?;
    state = rebind_seed(paths, state.revision, state.executor_epoch, seed)?;
    reconcile_interrupted(paths, &mut state)?;

    loop {
        let state = ControlState::load(&paths.control)?
            .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
        if state.activation.phase == ActivationPhase::RollbackDecided {
            execute_rollback(paths, state)?;
            continue;
        }
        let selected = load_selected(&state, &target)?;
        let candidate = state.activation.phase == ActivationPhase::SpawnPlanned
            && state
                .activation
                .attempt
                .as_ref()
                .is_some_and(|attempt| attempt.candidate.release_id == selected.release_id);
        let outcome = match launch_selected(paths, &state, &selected, &target, launcher_path) {
            Ok(outcome) => outcome,
            Err(error) if candidate => {
                let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                    LauncherError::Conflict("control state disappeared".to_string())
                })?;
                if latest.active_launch.is_some() {
                    return Err(error);
                }
                if !has_uncommitted_candidate_for(&latest, &selected.release_id) {
                    return Err(error);
                }
                rollback_failed_candidate(paths, latest, error.to_string())?;
                continue;
            }
            Err(error) => return Err(error),
        };
        let code = outcome.status.code().unwrap_or(1);
        if candidate && (!outcome.ready || !outcome.committed) {
            let latest = ControlState::load(&paths.control)?
                .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
            rollback_failed_candidate(
                paths,
                latest,
                format!("candidate exited before it became active (code {code})"),
            )?;
            continue;
        }
        let latest = ControlState::load(&paths.control)?
            .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
        if latest.activation.phase == ActivationPhase::Prepared && code == EXIT_COORDINATED_RESTART
        {
            authorize_candidate_after_old_stopped(paths, latest)?;
            continue;
        }
        if code != 0 {
            write_runtime_diagnostic(paths, &selected.release_id, code, "payload exited")?;
        }
        return Ok(RunOutcome::Exited(code));
    }
}

fn reconcile_active_launch(paths: &LauncherPaths, state: ControlState) -> Result<ControlState> {
    let Some(active) = state.active_launch.as_ref() else {
        return Ok(state);
    };
    let manual_cleanup_warning = if let Some(payload) = &active.payload_registration {
        match ProcessIdentity::observe(payload.payload.pid)
            .map_err(|error| crate::io_error("observe stale payload", error))?
        {
            Some(identity) if identity == payload.payload => {
                match cleanup_payload(
                    paths,
                    &active.launch_instance_id,
                    payload,
                    active.known_descendants.iter().copied().collect(),
                ) {
                    Ok(_) => None,
                    Err(ProcessCleanupError::Blocked(error)) => Some(format!(
                        "prior payload cannot be safely cleaned up automatically: {error}; \
                         no signal was sent to untracked processes and manual cleanup may be required"
                    )),
                    Err(error) => {
                        return Err(LauncherError::Launch(format!(
                            "best-effort cleanup of prior payload failed: {error}"
                        )));
                    }
                }
            }
            Some(_) => Some(format!(
                "prior payload pid {} has a different start identity; no signal was sent and \
                 the stale launch record was discarded",
                payload.payload.pid
            )),
            None => match cleanup_payload(
                paths,
                &active.launch_instance_id,
                payload,
                active.known_descendants.iter().copied().collect(),
            ) {
                Ok(_) => None,
                Err(ProcessCleanupError::Blocked(error)) => Some(format!(
                    "prior payload root is gone and remaining processes cannot be safely \
                     attributed: {error}; no signal was sent to untracked processes and \
                     manual cleanup may be required"
                )),
                Err(error) => {
                    return Err(LauncherError::Launch(format!(
                        "best-effort cleanup of prior payload descendants failed: {error}"
                    )));
                }
            },
        }
    } else {
        None
    };
    if let Some(warning) = manual_cleanup_warning {
        write_runtime_diagnostic(paths, &active.selected.capsule().release_id, 1, &warning)?;
    }
    clear_active_launch(paths, &active.launch_instance_id)
}

fn launch_selected(
    paths: &LauncherPaths,
    state: &ControlState,
    selected: &CapsuleRecord,
    target: &CapsuleTarget,
    launcher_path: &Path,
) -> Result<LaunchOutcome> {
    let verified = verify_record_for_spawn(selected, target)?;
    let (candidate, launch_instance_id, spawn_attempt_id) =
        launch_identity(state, &verified.release_id)?;
    let _reserved = reserve_active_launch(paths, state, &launch_instance_id, &spawn_attempt_id)?;
    let ready_path = paths
        .attempts
        .join(&spawn_attempt_id)
        .with_extension("ready.json");
    let token = readiness::issue_ready_token()?;
    let mut command = std::process::Command::new(&verified.executable);
    command
        .args(&verified.manifest.launch.arguments)
        .env_clear()
        .envs(build_payload_environment(
            paths,
            launcher_path,
            &ready_path,
            &token,
            &verified.release_id,
            &launch_instance_id,
            &spawn_attempt_id,
        ))
        .stdin(Stdio::null());
    if let Some(cwd) = &verified.cwd {
        command.current_dir(cwd);
    }
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            clear_active_launch(paths, &launch_instance_id)?;
            return Err(crate::io_error(
                format!("launch {}", verified.executable.display()),
                error,
            ));
        }
    };
    let pid = i32::try_from(child.id())
        .map_err(|_| LauncherError::Launch("spawned payload pid does not fit i32".to_string()))?;
    let Some(process_group) = ProcessGroupRecord::observe(pid)
        .map_err(|error| crate::io_error("observe spawned payload group", error))?
    else {
        let status = child
            .wait()
            .map_err(|error| crate::io_error("wait for early payload exit", error))?;
        clear_active_launch(paths, &launch_instance_id)?;
        return Ok(LaunchOutcome {
            status,
            ready: false,
            committed: false,
        });
    };
    let process_group = match process_group.require_dedicated() {
        Ok(process_group) => process_group,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(LauncherError::Launch(format!(
                "spawned payload did not lead a dedicated group: {error}"
            )));
        }
    };
    let payload = PayloadRegistration {
        release_id: verified.release_id.clone(),
        launch_instance_id: launch_instance_id.clone(),
        spawn_attempt_id: spawn_attempt_id.clone(),
        payload: process_group.leader,
        process_group,
    };
    let bearer = ReadyBearer::bind(
        verified.release_id.clone(),
        launch_instance_id.clone(),
        spawn_attempt_id.clone(),
        payload.payload,
        token,
    )?;
    if let Err(error) = persist_active_launch(
        paths,
        candidate,
        &launch_instance_id,
        &spawn_attempt_id,
        payload.clone(),
        bearer.expectation.clone(),
    ) {
        let durable_target = record_cleanup_target(
            paths,
            &launch_instance_id,
            payload.clone(),
            bearer.expectation.clone(),
        );
        return match (
            durable_target,
            terminate_direct_child(paths, &launch_instance_id, &mut child, &payload),
        ) {
            (Ok(()), Ok(())) => {
                clear_active_launch(paths, &launch_instance_id)?;
                Err(error)
            }
            (Ok(()), Err(cleanup_error)) => {
                let cleanup_error = handle_direct_cleanup_failure(
                    paths,
                    &selected.release_id,
                    &launch_instance_id,
                    "direct payload cleanup after registration failure",
                    cleanup_error,
                )?;
                Err(LauncherError::Launch(format!(
                    "{error}; direct payload cleanup failed: {cleanup_error}"
                )))
            }
            (Err(record_error), Ok(())) => {
                clear_active_launch(paths, &launch_instance_id)?;
                Err(LauncherError::Launch(format!(
                    "{error}; could not record cleanup target: {record_error}"
                )))
            }
            (Err(record_error), Err(cleanup_error)) => {
                let cleanup_error = handle_direct_cleanup_failure(
                    paths,
                    &selected.release_id,
                    &launch_instance_id,
                    "direct payload cleanup after registration failure",
                    cleanup_error,
                )?;
                Err(LauncherError::Launch(format!(
                    "{error}; could not record cleanup target: {record_error}; direct payload cleanup failed: {cleanup_error}"
                )))
            }
        };
    }

    let deadline =
        Instant::now() + Duration::from_millis(verified.manifest.launch.readiness.timeout_ms);
    let mut ready = false;
    let mut committed = false;
    let mut observed_descendants = BTreeSet::new();
    loop {
        let snapshot = crate::process::snapshot_processes().map_err(|error| {
            LauncherError::Launch(format!("snapshot payload descendants: {error}"))
        })?;
        observed_descendants.extend(crate::process::descendants_of(payload.payload, &snapshot));
        record_observed_descendants(paths, &launch_instance_id, &observed_descendants)?;
        if candidate
            && ControlState::load(&paths.control)?
                .is_some_and(|control| control.activation.phase == ActivationPhase::RollbackDecided)
        {
            let cleanup = terminate_direct_child(paths, &launch_instance_id, &mut child, &payload);
            return match cleanup {
                Ok(()) => {
                    clear_active_launch(paths, &launch_instance_id)?;
                    Err(LauncherError::Launch("rollback requested".to_string()))
                }
                Err(error) => Err(handle_direct_cleanup_failure(
                    paths,
                    &selected.release_id,
                    &launch_instance_id,
                    "candidate rollback cleanup",
                    error,
                )?),
            };
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| crate::io_error("observe payload exit", error))?
        {
            if let Err(error) =
                cleanup_payload(paths, &launch_instance_id, &payload, observed_descendants)
            {
                let message = match error {
                    ProcessCleanupError::Blocked(error) => format!(
                        "payload exited but remaining processes cannot be safely attributed: \
                         {error}; no signal was sent to untracked processes and manual cleanup \
                         may be required"
                    ),
                    error => {
                        return Err(LauncherError::Launch(format!(
                            "clean up payload after its exit: {error}"
                        )));
                    }
                };
                write_runtime_diagnostic(
                    paths,
                    &selected.release_id,
                    status.code().unwrap_or(1),
                    &message,
                )?;
                clear_active_launch(paths, &launch_instance_id)?;
                return Err(LauncherError::Launch(message));
            }
            clear_active_launch(paths, &launch_instance_id)?;
            return Ok(LaunchOutcome {
                status,
                ready,
                committed,
            });
        }
        if !ready && readiness::consume_ready_marker(&ready_path, &bearer.expectation)? {
            ready = true;
            if candidate {
                let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                    LauncherError::Conflict("control state disappeared".to_string())
                })?;
                commit_candidate(paths, mark_candidate_observing(paths, latest)?)?;
                committed = true;
            }
        }
        if !ready && Instant::now() >= deadline {
            let cleanup = terminate_direct_child(paths, &launch_instance_id, &mut child, &payload);
            return match cleanup {
                Ok(()) => {
                    clear_active_launch(paths, &launch_instance_id)?;
                    Err(LauncherError::Launch(
                        "payload readiness timed out".to_string(),
                    ))
                }
                Err(error) => Err(handle_direct_cleanup_failure(
                    paths,
                    &selected.release_id,
                    &launch_instance_id,
                    "payload readiness timeout cleanup",
                    error,
                )?),
            };
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn terminate_direct_child(
    paths: &LauncherPaths,
    launch_instance_id: &str,
    child: &mut std::process::Child,
    payload: &PayloadRegistration,
) -> std::result::Result<(), ProcessCleanupError> {
    let mut known_descendants =
        snapshot_and_record_descendants(paths, launch_instance_id, payload)?;
    signal_identity_if_exact(payload.payload, libc::SIGTERM)?;
    let term_deadline = Instant::now() + TerminationPolicy::default().term_timeout;
    while Instant::now() < term_deadline {
        known_descendants.extend(snapshot_and_record_descendants(
            paths,
            launch_instance_id,
            payload,
        )?);
        if child.try_wait().map_err(ProcessCleanupError::Io)?.is_some() {
            return cleanup_payload(paths, launch_instance_id, payload, known_descendants);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    child.kill().map_err(ProcessCleanupError::Io)?;
    child.wait().map_err(ProcessCleanupError::Io)?;
    cleanup_payload(paths, launch_instance_id, payload, known_descendants)
}

fn snapshot_and_record_descendants(
    paths: &LauncherPaths,
    launch_instance_id: &str,
    payload: &PayloadRegistration,
) -> std::result::Result<BTreeSet<ProcessIdentity>, ProcessCleanupError> {
    let snapshot = crate::process::snapshot_processes()?;
    let descendants = crate::process::descendants_of(payload.payload, &snapshot);
    record_observed_descendants(paths, launch_instance_id, &descendants)
        .map_err(|error| ProcessCleanupError::Io(std::io::Error::other(error.to_string())))?;
    Ok(descendants)
}

fn cleanup_payload(
    paths: &LauncherPaths,
    launch_instance_id: &str,
    payload: &PayloadRegistration,
    known_descendants: BTreeSet<ProcessIdentity>,
) -> std::result::Result<(), ProcessCleanupError> {
    terminate_and_observe_empty_with_reporter(
        &TerminationTarget {
            root: payload.payload,
            process_group: payload.process_group,
            additional_root: None,
            known_descendants,
        },
        TerminationPolicy::default(),
        |tracked| {
            let descendants = tracked
                .iter()
                .copied()
                .filter(|identity| *identity != payload.payload)
                .collect::<BTreeSet<_>>();
            record_observed_descendants(paths, launch_instance_id, &descendants)
                .map_err(|error| ProcessCleanupError::Io(std::io::Error::other(error.to_string())))
        },
    )?;
    Ok(())
}

fn handle_direct_cleanup_failure(
    paths: &LauncherPaths,
    release_id: &str,
    launch_instance_id: &str,
    context: &str,
    error: ProcessCleanupError,
) -> Result<LauncherError> {
    match error {
        ProcessCleanupError::Blocked(error) => {
            let message = format!(
                "{context} cannot safely attribute remaining processes: {error}; \
                 no signal was sent to untracked processes and manual cleanup may be required"
            );
            write_runtime_diagnostic(paths, release_id, 1, &message)?;
            clear_active_launch(paths, launch_instance_id)?;
            Ok(LauncherError::Launch(message))
        }
        error => Ok(LauncherError::Launch(error.to_string())),
    }
}

fn reserve_active_launch(
    paths: &LauncherPaths,
    state: &ControlState,
    launch_instance_id: &str,
    spawn_attempt_id: &str,
) -> Result<ControlState> {
    cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            if next.active_launch.is_some() {
                return Err(LauncherError::Conflict(
                    "an earlier payload launch is still recorded".to_string(),
                ));
            }
            next.active_launch = Some(ActiveLaunchRecord {
                selected: next.selected.clone(),
                launch_instance_id: launch_instance_id.to_string(),
                spawn_attempt_id: spawn_attempt_id.to_string(),
                phase: ActiveLaunchPhase::SpawnPlanned,
                payload_registration: None,
                ready_expectation: None,
                known_descendants: Vec::new(),
            });
            Ok(())
        },
    )
}

fn persist_active_launch(
    paths: &LauncherPaths,
    candidate: bool,
    launch_instance_id: &str,
    spawn_attempt_id: &str,
    payload: PayloadRegistration,
    expectation: readiness::ReadyExpectation,
) -> Result<()> {
    let current = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
    cas_update(
        &paths.control,
        current.revision,
        current.executor_epoch,
        |next| {
            if next
                .active_launch
                .as_ref()
                .is_none_or(|active| active.launch_instance_id != launch_instance_id)
            {
                return Err(LauncherError::Conflict(
                    "payload launch reservation disappeared".to_string(),
                ));
            }
            next.active_launch = Some(ActiveLaunchRecord {
                selected: next.selected.clone(),
                launch_instance_id: launch_instance_id.to_string(),
                spawn_attempt_id: spawn_attempt_id.to_string(),
                phase: ActiveLaunchPhase::Running,
                payload_registration: Some(payload.clone()),
                ready_expectation: Some(expectation.clone()),
                known_descendants: Vec::new(),
            });
            if candidate {
                let attempt = next.activation.attempt.as_mut().ok_or_else(|| {
                    LauncherError::Conflict("candidate activation disappeared".to_string())
                })?;
                attempt.payload_registration = Some(payload);
                attempt.ready_expectation = Some(expectation);
                attempt.known_descendants.clear();
                next.activation.phase = ActivationPhase::RuntimeStarted;
            }
            Ok(())
        },
    )?;
    Ok(())
}

fn record_cleanup_target(
    paths: &LauncherPaths,
    launch_instance_id: &str,
    payload: PayloadRegistration,
    expectation: readiness::ReadyExpectation,
) -> Result<()> {
    loop {
        let current = ControlState::load(&paths.control)?
            .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
        let active = current.active_launch.as_ref().ok_or_else(|| {
            LauncherError::Conflict("payload launch reservation disappeared".to_string())
        })?;
        if active.launch_instance_id != launch_instance_id {
            return Err(LauncherError::Conflict(
                "payload launch reservation was replaced".to_string(),
            ));
        }
        if active.payload_registration.as_ref() == Some(&payload) {
            return Ok(());
        }
        match cas_update(
            &paths.control,
            current.revision,
            current.executor_epoch,
            |next| {
                let active = next.active_launch.as_mut().ok_or_else(|| {
                    LauncherError::Conflict("payload launch reservation disappeared".to_string())
                })?;
                if active.launch_instance_id != launch_instance_id {
                    return Err(LauncherError::Conflict(
                        "payload launch reservation was replaced".to_string(),
                    ));
                }
                active.phase = ActiveLaunchPhase::Running;
                active.payload_registration = Some(payload.clone());
                active.ready_expectation = Some(expectation.clone());
                active.known_descendants.clear();
                Ok(())
            },
        ) {
            Ok(_) => return Ok(()),
            Err(LauncherError::Conflict(_)) => continue,
            Err(error) => return Err(error),
        }
    }
}

fn record_observed_descendants(
    paths: &LauncherPaths,
    launch_instance_id: &str,
    observed_descendants: &BTreeSet<ProcessIdentity>,
) -> Result<()> {
    if observed_descendants.is_empty() {
        return Ok(());
    }
    loop {
        let current = ControlState::load(&paths.control)?
            .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
        let active = current.active_launch.as_ref().ok_or_else(|| {
            LauncherError::Conflict("payload launch reservation disappeared".to_string())
        })?;
        if active.launch_instance_id != launch_instance_id {
            return Err(LauncherError::Conflict(
                "payload launch reservation was replaced".to_string(),
            ));
        }
        let persisted = active
            .known_descendants
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        if observed_descendants.is_subset(&persisted) {
            return Ok(());
        }
        match cas_update(
            &paths.control,
            current.revision,
            current.executor_epoch,
            |next| {
                let active = next.active_launch.as_mut().ok_or_else(|| {
                    LauncherError::Conflict("payload launch reservation disappeared".to_string())
                })?;
                if active.launch_instance_id != launch_instance_id {
                    return Err(LauncherError::Conflict(
                        "payload launch reservation was replaced".to_string(),
                    ));
                }
                let mut merged = active
                    .known_descendants
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>();
                merged.extend(observed_descendants.iter().copied());
                active.known_descendants = merged.into_iter().collect();
                Ok(())
            },
        ) {
            Ok(_) => return Ok(()),
            Err(LauncherError::Conflict(_)) => continue,
            Err(error) => return Err(error),
        }
    }
}

fn launch_identity(state: &ControlState, release_id: &str) -> Result<(bool, String, String)> {
    let candidate = state.activation.phase == ActivationPhase::SpawnPlanned
        && state
            .activation
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.candidate.release_id == release_id);
    if candidate {
        let attempt = state.activation.attempt.as_ref().expect("checked above");
        return Ok((
            true,
            attempt.launch_instance_id.clone().ok_or_else(|| {
                LauncherError::Conflict("candidate launch has no launchInstanceId".to_string())
            })?,
            attempt.spawn_attempt_id.clone().ok_or_else(|| {
                LauncherError::Conflict("candidate launch has no spawnAttemptId".to_string())
            })?,
        ));
    }
    let launch = format!("{}-{}", std::process::id(), unix_time_ms());
    Ok((false, launch.clone(), format!("runtime-{launch}")))
}

fn has_uncommitted_candidate_for(state: &ControlState, release_id: &str) -> bool {
    state
        .activation
        .attempt
        .as_ref()
        .is_some_and(|attempt| attempt.candidate.release_id == release_id)
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
    const ALLOWLIST: [&str; 17] = [
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
    let mut environment = ALLOWLIST
        .into_iter()
        .filter_map(|name| {
            inherited
                .get(name)
                .map(|value| (name.to_string(), value.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    environment.insert(
        "RUNTIME_CAPSULE_READY_PROTOCOL".to_string(),
        readiness::READY_PROTOCOL_VERSION.to_string(),
    );
    for (key, value) in [
        (
            "RUNTIME_CAPSULE_READY_PATH",
            ready_path.display().to_string(),
        ),
        ("RUNTIME_CAPSULE_READY_TOKEN", token.to_string()),
        ("RUNTIME_CAPSULE_RELEASE_ID", release_id.to_string()),
        (
            "RUNTIME_CAPSULE_LAUNCH_INSTANCE_ID",
            launch_instance_id.to_string(),
        ),
        (
            "RUNTIME_CAPSULE_SPAWN_ATTEMPT_ID",
            spawn_attempt_id.to_string(),
        ),
        (
            "RUNTIME_CAPSULE_FAILURE_EVIDENCE_PATH",
            paths.failure_evidence.display().to_string(),
        ),
        (
            "RUNTIME_CAPSULE_LAUNCHER_PATH",
            launcher_path.display().to_string(),
        ),
        (
            "RUNTIME_CAPSULE_LAUNCHER_HOME",
            paths.root.display().to_string(),
        ),
    ] {
        environment.insert(key.to_string(), value);
    }
    environment
}

fn clear_active_launch(paths: &LauncherPaths, launch_instance_id: &str) -> Result<ControlState> {
    let state = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
    if state
        .active_launch
        .as_ref()
        .is_none_or(|active| active.launch_instance_id != launch_instance_id)
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
    let stopping = cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::StoppingOld;
            Ok(())
        },
    )?;
    let stopped = cas_update(
        &paths.control,
        stopping.revision,
        stopping.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::OldStopped;
            Ok(())
        },
    )?;
    cas_update(
        &paths.control,
        stopped.revision,
        stopped.executor_epoch,
        |next| {
            let revision = next.revision;
            let attempt = next.activation.attempt.as_mut().ok_or_else(|| {
                LauncherError::Conflict("prepared activation has no attempt".to_string())
            })?;
            attempt.launch_instance_id = Some(format!("{}-{}", std::process::id(), unix_time_ms()));
            attempt.spawn_attempt_id = Some(format!("{}-{revision}", attempt.attempt_id));
            let candidate = attempt.candidate.clone();
            let current = next.external_current.clone();
            next.external_current = Some(candidate.clone());
            next.external_previous = current;
            next.selected = SelectedRuntime::external(candidate);
            next.activation.phase = ActivationPhase::SpawnPlanned;
            Ok(())
        },
    )
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
    cas_update(
        &paths.control,
        decided.revision,
        decided.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::Idle;
            next.activation.attempt = None;
            Ok(())
        },
    )
}

fn mark_candidate_observing(paths: &LauncherPaths, state: ControlState) -> Result<ControlState> {
    cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::Observing;
            let attempt = next.activation.attempt.as_mut().ok_or_else(|| {
                LauncherError::Conflict("candidate observation has no attempt".to_string())
            })?;
            attempt.observation_deadline_unix_ms =
                Some(unix_time_ms() + OBSERVATION_WINDOW.as_millis() as u64);
            Ok(())
        },
    )
}

fn rollback_failed_candidate(
    paths: &LauncherPaths,
    state: ControlState,
    reason: String,
) -> Result<ControlState> {
    if state.activation.phase == ActivationPhase::RollbackDecided {
        return execute_rollback(paths, state);
    }
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
                reason,
            });
            Ok(())
        },
    )?;
    execute_rollback(paths, decided)
}

fn execute_rollback(paths: &LauncherPaths, state: ControlState) -> Result<ControlState> {
    let attempt =
        state.activation.attempt.clone().ok_or_else(|| {
            LauncherError::Conflict("rollback has no activation attempt".to_string())
        })?;
    let restoring = cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::Restoring;
            next.external_current = attempt.previous_external_current.clone();
            next.external_previous = attempt.previous_external_previous.clone();
            next.selected = attempt.previous.clone();
            next.activation.receipt = Some(ActivationReceipt {
                attempt_id: attempt.attempt_id.clone(),
                candidate_release_id: attempt.candidate.release_id.clone(),
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
        },
    )?;
    write_failure_projection(paths, &restoring, "activation_rolled_back")?;
    cas_update(
        &paths.control,
        restoring.revision,
        restoring.executor_epoch,
        |next| {
            next.activation.phase = ActivationPhase::Idle;
            next.activation.attempt = None;
            Ok(())
        },
    )
}

fn reconcile_interrupted(paths: &LauncherPaths, state: &mut ControlState) -> Result<()> {
    if matches!(
        state.activation.phase,
        ActivationPhase::StoppingOld
            | ActivationPhase::OldStopped
            | ActivationPhase::SpawnPlanned
            | ActivationPhase::RuntimeStarted
            | ActivationPhase::AwaitingReady
            | ActivationPhase::Observing
            | ActivationPhase::RollbackDecided
            | ActivationPhase::StoppingCandidate
            | ActivationPhase::CandidateStopped
            | ActivationPhase::Restoring
    ) {
        *state = rollback_failed_candidate(
            paths,
            state.clone(),
            "recovered interrupted activation".to_string(),
        )?;
    }
    Ok(())
}

fn rebind_seed(
    paths: &LauncherPaths,
    revision: u64,
    epoch: u64,
    seed: crate::TrustedSeed,
) -> Result<ControlState> {
    let state = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
    if state.revision != revision || state.executor_epoch != epoch {
        return Err(LauncherError::Conflict(
            "Seed state changed during startup".to_string(),
        ));
    }
    if state.trusted_seed == seed {
        return Ok(state);
    }
    cas_update(&paths.control, revision, epoch, |next| {
        let selected_seed = matches!(next.selected, SelectedRuntime::Seed { .. });
        next.trusted_seed = seed.clone();
        if selected_seed {
            next.selected = SelectedRuntime::seed(seed.capsule.clone());
        }
        Ok(())
    })
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

fn load_selected(state: &ControlState, target: &CapsuleTarget) -> Result<CapsuleRecord> {
    let capsule = state.selected.capsule();
    load_and_verify_capsule(&capsule.root, target)
}

fn capsule_ref(record: &CapsuleRecord) -> CapsuleRef {
    CapsuleRef {
        release_id: record.release_id.clone(),
        root: record.root.clone(),
        entrypoint: record.executable.clone(),
        metadata: serde_json::Value::Null,
    }
}

fn validate_prepare(request: &PrepareActivationRequest) -> Result<()> {
    if request.schema_version != REQUEST_SCHEMA_VERSION
        || request.activation_id.trim().is_empty()
        || request.release_id.trim().is_empty()
    {
        return Err(LauncherError::InvalidRequest(
            "prepare request has invalid schemaVersion, activationId, or releaseId".to_string(),
        ));
    }
    Ok(())
}

fn write_runtime_diagnostic(
    paths: &LauncherPaths,
    release_id: &str,
    code: i32,
    message: &str,
) -> Result<()> {
    crate::control::write_json_atomic(
        &paths.failure_evidence,
        &FailureProjection {
            activation_id: format!("runtime-{}", unix_time_ms()),
            release_id: release_id.to_string(),
            occurred_at: rfc3339_now(),
            fallback_release_id: None,
            code: format!("payload_exit_{code}"),
            message: message.to_string(),
            failed: None,
            fallback: None,
            evidence_path: None,
            details: BTreeMap::new(),
        },
    )
}

fn write_failure_projection(paths: &LauncherPaths, state: &ControlState, code: &str) -> Result<()> {
    let Some(attempt) = state.activation.attempt.as_ref() else {
        return Ok(());
    };
    crate::control::write_json_atomic(
        &paths.failure_evidence,
        &FailureProjection {
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
            details: BTreeMap::new(),
        },
    )
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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
    use crate::control::CapsuleRef;
    use crate::control::TrustedSeed;

    fn capsule(root: &Path, id: &str) -> CapsuleRef {
        let root = root.join(id);
        CapsuleRef {
            release_id: format!("release-{id}"),
            entrypoint: root.join("bin/runtime"),
            root,
            metadata: serde_json::Value::Null,
        }
    }

    fn active_state(root: &Path, payload: ProcessIdentity) -> ControlState {
        let seed_capsule = capsule(root, "seed");
        let mut state = ControlState::initialize(TrustedSeed {
            capsule: seed_capsule.clone(),
            trust_anchor: root.join("seed-anchor"),
            metadata: serde_json::Value::Null,
        });
        state.active_launch = Some(ActiveLaunchRecord {
            selected: state.selected.clone(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            phase: ActiveLaunchPhase::Running,
            payload_registration: Some(PayloadRegistration {
                release_id: seed_capsule.release_id.clone(),
                launch_instance_id: "launch-1".to_string(),
                spawn_attempt_id: "spawn-1".to_string(),
                payload,
                process_group: ProcessGroupRecord {
                    leader: payload,
                    pgid: payload.pid,
                },
            }),
            ready_expectation: Some(readiness::ReadyExpectation {
                protocol_version: readiness::READY_PROTOCOL_VERSION,
                release_id: seed_capsule.release_id,
                launch_instance_id: "launch-1".to_string(),
                spawn_attempt_id: "spawn-1".to_string(),
                payload,
                token_verifier: "a".repeat(64),
            }),
            known_descendants: Vec::new(),
        });
        state
    }

    #[test]
    fn reused_payload_pid_is_diagnostic_not_a_sticky_launch_gate() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let live = ProcessIdentity::observe(
            i32::try_from(std::process::id()).expect("current pid fits i32"),
        )
        .expect("observe current process")
        .expect("current process is live");
        let stale = ProcessIdentity {
            pid: live.pid,
            start_identity: live.start_identity.wrapping_add(1),
        };
        assert_ne!(stale, live, "test needs a mismatched process identity");
        let state = active_state(temp.path(), stale);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write active state");

        let reconciled = reconcile_active_launch(&paths, state).expect("reconcile stale launch");
        assert!(reconciled.active_launch.is_none());
        assert!(
            ControlState::load(&paths.control)
                .expect("load state")
                .expect("state")
                .active_launch
                .is_none()
        );
        let diagnostic =
            crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
                .expect("read diagnostic")
                .expect("stale launch diagnostic");
        assert!(diagnostic.message.contains("different start identity"));
    }

    #[test]
    fn committed_candidate_is_not_rolled_back_after_its_attempt_is_cleared() {
        let candidate = capsule(Path::new("/tmp/runtime-launcher-tests"), "candidate");
        let mut state = ControlState::initialize(TrustedSeed {
            capsule: capsule(Path::new("/tmp/runtime-launcher-tests"), "seed"),
            trust_anchor: PathBuf::from("/tmp/runtime-launcher-tests/seed-anchor"),
            metadata: serde_json::Value::Null,
        });
        state.activation.receipt = Some(ActivationReceipt {
            attempt_id: "activation-1".to_string(),
            candidate_release_id: candidate.release_id.clone(),
            outcome: ActivationOutcome::Committed,
            selected: state.selected.clone(),
            completed_at_unix_ms: 1,
            reason: None,
        });
        assert!(
            !has_uncommitted_candidate_for(&state, &candidate.release_id),
            "a committed idle state has no remaining candidate attempt to roll back"
        );
    }

    #[test]
    fn blocked_direct_cleanup_releases_active_launch_but_timeout_does_not() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let payload = ProcessIdentity {
            pid: 999_999,
            start_identity: 1,
        };
        let state = active_state(temp.path(), payload);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write active state");

        let error = handle_direct_cleanup_failure(
            &paths,
            "release-seed",
            "launch-1",
            "test cleanup",
            ProcessCleanupError::Blocked("untracked member".to_string()),
        )
        .expect("classify blocked cleanup");
        assert!(error.to_string().contains("manual cleanup may be required"));
        assert!(
            ControlState::load(&paths.control)
                .expect("load after blocked cleanup")
                .expect("state")
                .active_launch
                .is_none(),
            "unattributable residuals must not leave a sticky launch reservation"
        );

        let state = active_state(temp.path(), payload);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("restore active state");
        let error = handle_direct_cleanup_failure(
            &paths,
            "release-seed",
            "launch-1",
            "test cleanup",
            ProcessCleanupError::TimedOut("still alive".to_string()),
        )
        .expect("classify timeout");
        assert!(error.to_string().contains("timed out"));
        assert!(
            ControlState::load(&paths.control)
                .expect("load after timeout")
                .expect("state")
                .active_launch
                .is_some(),
            "attributable timeout failures remain recoverable on the next launch"
        );
    }

    #[test]
    fn observed_descendants_are_durable_for_restart_recovery() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let payload = ProcessIdentity {
            pid: 999_999,
            start_identity: 1,
        };
        crate::control::write_json_atomic(
            &paths.control.control,
            &active_state(temp.path(), payload),
        )
        .expect("write active state");
        let observed = ProcessIdentity {
            pid: 999_998,
            start_identity: 2,
        };

        record_observed_descendants(&paths, "launch-1", &BTreeSet::from([observed]))
            .expect("record observed descendant");

        let persisted = ControlState::load(&paths.control)
            .expect("load persisted state")
            .expect("state")
            .active_launch
            .expect("active launch")
            .known_descendants;
        assert_eq!(persisted, vec![observed]);
    }

    #[test]
    fn direct_cleanup_persists_its_final_descendant_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let mut command = std::process::Command::new("/bin/sh");
        command.arg("-c").arg("sleep 5 & wait");
        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().expect("spawn direct payload");
        let pid = i32::try_from(child.id()).expect("payload pid fits i32");
        let process_group = ProcessGroupRecord::observe(pid)
            .expect("observe payload group")
            .expect("payload group")
            .require_dedicated()
            .expect("dedicated group");
        let payload = PayloadRegistration {
            release_id: "release-seed".to_string(),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            payload: process_group.leader,
            process_group,
        };
        crate::control::write_json_atomic(
            &paths.control.control,
            &active_state(temp.path(), payload.payload),
        )
        .expect("write active state");

        let deadline = Instant::now() + Duration::from_secs(1);
        let observed = loop {
            let snapshot = crate::process::snapshot_processes().expect("snapshot");
            let descendants = crate::process::descendants_of(payload.payload, &snapshot);
            if let Some(identity) = descendants.iter().next().copied() {
                break identity;
            }
            assert!(
                Instant::now() < deadline,
                "shell payload did not create its worker in time"
            );
            std::thread::sleep(Duration::from_millis(10));
        };

        terminate_direct_child(&paths, "launch-1", &mut child, &payload)
            .expect("terminate direct payload");

        let persisted = ControlState::load(&paths.control)
            .expect("load persisted state")
            .expect("state")
            .active_launch
            .expect("active launch")
            .known_descendants;
        assert_eq!(persisted, vec![observed]);
    }
}
