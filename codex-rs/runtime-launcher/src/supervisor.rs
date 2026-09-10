use crate::LauncherError;
use crate::SelectCandidateRequest;
use crate::Result;
use crate::capsule::CapsuleRecord;
use crate::capsule::CapsuleTarget;
use crate::capsule::gc_unreferenced_external_artifacts;
use crate::capsule::import_incoming;
use crate::capsule::load_and_verify_capsule;
use crate::capsule::verify_record_for_spawn;
use crate::control::ActiveLaunchPhase;
use crate::control::ActiveLaunchRecord;
use crate::control::CapsuleRef;
use crate::control::ControlPaths;
use crate::control::ControlState;
use crate::control::ExecutorEpoch;
use crate::control::FailureProjection;
use crate::control::PayloadRegistration;
use crate::control::SelectedRuntime;
use crate::control::cas_update;
use crate::control::load_unlocked;
use crate::control::update_locked;
use crate::control::StateLock;
use crate::migration::migrate_legacy_state;
use crate::process::ProcessCleanupError;
use crate::process::ProcessGroupRecord;
use crate::process::ProcessIdentity;
use crate::process::TerminationPolicy;
use crate::process::TerminationTarget;
use crate::process::signal_identity_if_exact;
use crate::process::terminate_and_observe_empty_with_reporter;
use crate::seed::discover_seed;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::ExitStatusExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

const REQUEST_SCHEMA_VERSION: u32 = 1;

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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectCandidateResult {
    pub activation_id: String,
    pub release_id: String,
    pub control: ControlState,
}

struct LaunchOutcome {
    status: ExitStatus,
    payload_pid: Option<i32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RuntimeFailure {
    code: String,
    message: String,
    details: BTreeMap<String, String>,
}

enum ExternalFallback {
    Restored,
    SelectionChanged,
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

pub fn select_candidate(
    paths: &LauncherPaths,
    request: SelectCandidateRequest,
) -> Result<SelectCandidateResult> {
    validate_select_candidate(&request)?;
    let _lock = StateLock::acquire(&paths.control)?;
    load_unlocked(&paths.control)?.ok_or_else(|| {
        LauncherError::Conflict("run must initialize the trusted Seed before selecting a candidate".to_string())
    })?;
    let candidate = import_incoming(&paths.root, &request.activation_id, &request.target)?;
    let candidate = capsule_ref(&candidate);
    let release_id = candidate.release_id.clone();
    let current = load_unlocked(&paths.control)?.ok_or_else(|| {
        LauncherError::Conflict("control state disappeared during candidate import".to_string())
    })?;
    let control = if selection_is_already_current(&current, &candidate) {
        current
    } else {
        update_locked(&paths.control, |state| {
            state.external_previous = state.external_current.clone();
            state.external_current = Some(candidate.clone());
            state.selected = SelectedRuntime::external(candidate.clone());
            Ok(())
        })?
    };
    gc_unreferenced_external_artifacts_locked(paths, &control);
    Ok(SelectCandidateResult {
        activation_id: request.activation_id,
        release_id,
        control,
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
    let state = reconcile_active_launch(paths, state)?;
    rebind_seed(paths, state.revision, state.executor_epoch, seed)?;
    gc_unreferenced_external_artifacts_at_safe_boundary(paths);

    loop {
        let state = ControlState::load(&paths.control)?
            .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
        let selected = match load_selected(&state, &target) {
            Ok(selected) => selected,
            Err(error) if matches!(state.selected, SelectedRuntime::External { .. }) => {
                let _ = restore_failed_external(
                    paths,
                    &state,
                    RuntimeFailure::load(error.to_string()),
                    None,
                )?;
                continue;
            }
            Err(error) => return Err(error),
        };
        let outcome = match launch_selected(paths, &state, &selected, &target, launcher_path) {
            Ok(outcome) => outcome,
            Err(error) => {
                let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                    LauncherError::Conflict("control state disappeared".to_string())
                })?;
                if latest.active_launch.is_none()
                    && latest.selected.capsule().release_id != selected.release_id
                {
                    continue;
                }
                if latest.active_launch.is_some() {
                    return Err(error);
                }
                if matches!(latest.selected, SelectedRuntime::External { .. }) {
                    let _ = restore_failed_external(
                        paths,
                        &latest,
                        RuntimeFailure::spawn_or_load(error.to_string()),
                        None,
                    )?;
                    continue;
                }
                return Err(error);
            }
        };
        if is_selected_capsule_switch(&outcome.status) {
            continue;
        }
        let Some(failure) = RuntimeFailure::from_exit_status(&outcome.status) else {
            return Ok(RunOutcome::Exited(outcome.status.code().unwrap_or(0)));
        };
        handle_unexpected_payload_exit(
            paths,
            &selected.release_id,
            outcome.payload_pid,
            failure,
        )?;
    }
}

/// Exit code emitted by Electron after a successfully selected Runtime Capsule
/// update. It asks the Launcher to return to the supervise loop and spawn the
/// current persisted selection.
const CAPSULE_SWITCH_EXIT_CODE: i32 = 75;

fn is_selected_capsule_switch(status: &std::process::ExitStatus) -> bool {
    status.code() == Some(CAPSULE_SWITCH_EXIT_CODE)
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
    let (launch_instance_id, spawn_attempt_id) = launch_identity();
    let _reserved = reserve_active_launch(paths, state, &launch_instance_id, &spawn_attempt_id)?;
    let mut command = std::process::Command::new(&verified.executable);
    command
        .args(&verified.manifest.launch.arguments)
        .env_clear()
        .envs(build_payload_environment(paths, launcher_path, &verified.release_id))
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
            payload_pid: Some(pid),
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
    if let Err(error) = persist_active_launch(
        paths,
        &launch_instance_id,
        &spawn_attempt_id,
        payload.clone(),
    ) {
        let durable_target = record_cleanup_target(paths, &launch_instance_id, payload.clone());
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

    let mut observed_descendants = BTreeSet::new();
    loop {
        let snapshot = crate::process::snapshot_processes().map_err(|error| {
            LauncherError::Launch(format!("snapshot payload descendants: {error}"))
        })?;
        observed_descendants.extend(crate::process::descendants_of(payload.payload, &snapshot));
        record_observed_descendants(paths, &launch_instance_id, &observed_descendants)?;
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
                payload_pid: Some(pid),
            });
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
            if next.selected.capsule().release_id != state.selected.capsule().release_id {
                return Err(LauncherError::Conflict(
                    "selected runtime changed before payload launch".to_string(),
                ));
            }
            next.active_launch = Some(ActiveLaunchRecord {
                selected: next.selected.clone(),
                launch_instance_id: launch_instance_id.to_string(),
                spawn_attempt_id: spawn_attempt_id.to_string(),
                phase: ActiveLaunchPhase::SpawnPlanned,
                payload_registration: None,
                known_descendants: Vec::new(),
            });
            Ok(())
        },
    )
}

fn persist_active_launch(
    paths: &LauncherPaths,
    launch_instance_id: &str,
    spawn_attempt_id: &str,
    payload: PayloadRegistration,
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
                known_descendants: Vec::new(),
            });
            Ok(())
        },
    )?;
    Ok(())
}

fn record_cleanup_target(
    paths: &LauncherPaths,
    launch_instance_id: &str,
    payload: PayloadRegistration,
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

fn launch_identity() -> (String, String) {
    let launch = format!("{}-{}", std::process::id(), unix_time_ms());
    (launch.clone(), format!("runtime-{launch}"))
}

fn build_payload_environment(
    paths: &LauncherPaths,
    launcher_path: &Path,
    release_id: &str,
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
    for (key, value) in [
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
        (
            "RUNTIME_CAPSULE_RELEASE_ID",
            release_id.to_string(),
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

fn restore_failed_external(
    paths: &LauncherPaths,
    state: &ControlState,
    failure: RuntimeFailure,
    payload_pid: Option<i32>,
) -> Result<ExternalFallback> {
    let failed = match &state.selected {
        SelectedRuntime::External { capsule } => capsule.clone(),
        SelectedRuntime::Seed { .. } => {
            return Err(LauncherError::Conflict(
                "only an external selection can fall back".to_string(),
            ));
        }
    };
    let restored = match cas_update(
        &paths.control,
        state.revision,
        state.executor_epoch,
        |next| {
            if next.selected.capsule().release_id != failed.release_id {
                return Err(LauncherError::Conflict(
                    "selected runtime changed before fallback".to_string(),
                ));
            }
            if let Some(previous) = next.external_previous.take() {
                next.external_current = Some(previous.clone());
                next.selected = SelectedRuntime::external(previous);
            } else {
                next.external_current = None;
                next.selected = SelectedRuntime::seed(next.trusted_seed.capsule.clone());
            }
            Ok(())
        },
    ) {
        Ok(restored) => restored,
        Err(LauncherError::Conflict(_)) => {
            let latest = ControlState::load(&paths.control)?.ok_or_else(|| {
                LauncherError::Conflict("control state disappeared during fallback".to_string())
            })?;
            if latest.revision != state.revision || latest.executor_epoch != state.executor_epoch {
                return Ok(ExternalFallback::SelectionChanged);
            }
            return Err(LauncherError::Conflict(
                "external fallback conflicted without a newer selection".to_string(),
            ));
        }
        Err(error) => return Err(error),
    };
    gc_unreferenced_external_artifacts_at_safe_boundary(paths);
    write_runtime_failure(
        paths,
        failed,
        failure,
        Some(restored.selected.clone()),
        payload_pid,
    )?;
    Ok(ExternalFallback::Restored)
}

fn gc_unreferenced_external_artifacts_at_safe_boundary(paths: &LauncherPaths) {
    let Ok(_lock) = StateLock::acquire(&paths.control) else {
        return;
    };
    let Ok(Some(state)) = load_unlocked(&paths.control) else {
        return;
    };
    gc_unreferenced_external_artifacts_locked(paths, &state);
}

fn gc_unreferenced_external_artifacts_locked(paths: &LauncherPaths, state: &ControlState) {
    let artifacts = paths.root.join("artifacts");
    let mut protected = BTreeSet::new();
    for capsule in [
        state.external_current.as_ref(),
        state.external_previous.as_ref(),
        match &state.selected {
            SelectedRuntime::External { capsule } => Some(capsule),
            SelectedRuntime::Seed { .. } => None,
        },
        state.active_launch.as_ref().and_then(|active| match &active.selected {
            SelectedRuntime::External { capsule } => Some(capsule),
            SelectedRuntime::Seed { .. } => None,
        }),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(digest) = canonical_artifact_digest(&artifacts, capsule) {
            protected.insert(digest);
        }
    }
    let _ = gc_unreferenced_external_artifacts(&paths.root, &protected);
}

fn canonical_artifact_digest(artifacts: &Path, capsule: &CapsuleRef) -> Option<String> {
    let digest = capsule.release_id.strip_prefix("sha256:")?;
    if !is_content_digest(digest) || capsule.root != artifacts.join(digest) {
        return None;
    }
    Some(digest.to_string())
}

fn is_content_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn handle_unexpected_payload_exit(
    paths: &LauncherPaths,
    launched_release_id: &str,
    payload_pid: Option<i32>,
    failure: RuntimeFailure,
) -> Result<()> {
    let latest = ControlState::load(&paths.control)?
        .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
    if latest.selected.capsule().release_id != launched_release_id {
        return Ok(());
    }
    if matches!(latest.selected, SelectedRuntime::External { .. }) {
        let _ = restore_failed_external(paths, &latest, failure, payload_pid)?;
    } else {
        write_runtime_failure(
            paths,
            latest.selected.capsule().clone(),
            failure,
            None,
            payload_pid,
        )?;
    }
    Ok(())
}

impl RuntimeFailure {
    fn from_exit_status(status: &ExitStatus) -> Option<Self> {
        if status.success() {
            return None;
        }
        if let Some(signal) = status.signal() {
            return Some(Self {
                code: "payload_exit_signal".to_string(),
                message: format!("payload terminated by signal {signal}"),
                details: BTreeMap::from([("signal".to_string(), signal.to_string())]),
            });
        }
        let code = status.code().unwrap_or(1);
        Some(Self {
            code: "payload_exit_code".to_string(),
            message: format!("payload exited with code {code}"),
            details: BTreeMap::from([("exitCode".to_string(), code.to_string())]),
        })
    }

    fn load(message: String) -> Self {
        Self::spawn_or_load_with_stage(message, "load")
    }

    fn spawn_or_load(message: String) -> Self {
        Self::spawn_or_load_with_stage(message, "spawn_or_load")
    }

    fn spawn_or_load_with_stage(message: String, stage: &str) -> Self {
        Self {
            code: "payload_spawn_or_load_error".to_string(),
            message: format!("payload {stage} failed: {message}"),
            details: BTreeMap::from([("stage".to_string(), stage.to_string())]),
        }
    }
}

fn write_runtime_failure(
    paths: &LauncherPaths,
    failed: CapsuleRef,
    failure: RuntimeFailure,
    fallback: Option<SelectedRuntime>,
    payload_pid: Option<i32>,
) -> Result<()> {
    let mut evidence = crate::control::read_json_if_exists::<FailureProjection>(
        &paths.failure_evidence,
    )?
    .filter(|existing| {
        existing.release_id == failed.release_id
            && existing.code == "payload_reported_error"
            && payload_pid.is_some_and(|pid| {
                existing
                    .details
                    .get("payloadPid")
                    .is_some_and(|recorded| recorded == &pid.to_string())
            })
    })
    .unwrap_or_else(|| FailureProjection {
        activation_id: format!("runtime-{}", unix_time_ms()),
        release_id: failed.release_id.clone(),
        occurred_at: rfc3339_now(),
        fallback_release_id: None,
        code: failure.code,
        message: failure.message,
        failed: Some(failed.clone()),
        fallback: None,
        evidence_path: Some(paths.failure_evidence.clone()),
        details: failure.details,
    });
    evidence.fallback_release_id = fallback
        .as_ref()
        .map(|selected| selected.capsule().release_id.clone());
    evidence.fallback = fallback;
    evidence.evidence_path = Some(paths.failure_evidence.clone());
    crate::control::write_json_atomic(&paths.failure_evidence, &evidence)
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

fn selection_is_already_current(state: &ControlState, candidate: &CapsuleRef) -> bool {
    state.external_current.as_ref() == Some(candidate)
        && matches!(
            &state.selected,
            SelectedRuntime::External { capsule } if capsule == candidate
        )
}

fn validate_select_candidate(request: &SelectCandidateRequest) -> Result<()> {
    if request.schema_version != REQUEST_SCHEMA_VERSION
        || request.activation_id.trim().is_empty()
    {
        return Err(LauncherError::InvalidRequest(
            "select-candidate request has invalid schemaVersion or activationId".to_string(),
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
    use std::process::Command;

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
            known_descendants: Vec::new(),
        });
        state
    }

    fn external_state(root: &Path, with_previous: bool) -> ControlState {
        let current = capsule(root, "current");
        let previous = capsule(root, "previous");
        let mut state = ControlState::initialize(TrustedSeed {
            capsule: capsule(root, "seed"),
            trust_anchor: root.join("seed-anchor"),
            metadata: serde_json::Value::Null,
        });
        state.external_current = Some(current.clone());
        state.external_previous = with_previous.then_some(previous);
        state.selected = SelectedRuntime::external(current);
        state
    }

    fn artifact_capsule(state_root: &Path, digit: char) -> CapsuleRef {
        let digest = digit.to_string().repeat(64);
        let root = state_root.join("artifacts").join(&digest);
        std::fs::create_dir_all(&root).expect("artifact");
        CapsuleRef {
            release_id: format!("sha256:{digest}"),
            entrypoint: root.join("bin/runtime"),
            root,
            metadata: serde_json::Value::Null,
        }
    }

    #[test]
    fn activation_gc_keeps_current_and_previous_and_removes_the_retired_generation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let retired = artifact_capsule(&paths.root, 'a');
        let previous = artifact_capsule(&paths.root, 'b');
        let current = artifact_capsule(&paths.root, 'c');
        let mut state = ControlState::initialize(TrustedSeed {
            capsule: capsule(temp.path(), "seed"),
            trust_anchor: temp.path().join("seed-anchor"),
            metadata: serde_json::Value::Null,
        });
        state.external_current = Some(current.clone());
        state.external_previous = Some(previous.clone());
        state.selected = SelectedRuntime::external(current);

        gc_unreferenced_external_artifacts_locked(&paths, &state);

        assert!(!retired.root.exists());
        assert!(previous.root.is_dir());
        assert!(state.selected.capsule().root.is_dir());
    }

    #[test]
    fn fallback_gc_removes_failed_current_after_the_durable_selection_transition() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let previous = artifact_capsule(&paths.root, 'a');
        let failed = artifact_capsule(&paths.root, 'b');
        let mut state = ControlState::initialize(TrustedSeed {
            capsule: capsule(temp.path(), "seed"),
            trust_anchor: temp.path().join("seed-anchor"),
            metadata: serde_json::Value::Null,
        });
        state.external_current = Some(failed.clone());
        state.external_previous = Some(previous.clone());
        state.selected = SelectedRuntime::external(failed);
        crate::control::write_json_atomic(&paths.control.control, &state).expect("write state");

        restore_failed_external(
            &paths,
            &state,
            RuntimeFailure::load("load failed".to_string()),
            None,
        )
        .expect("restore previous");

        let restored = ControlState::load(&paths.control)
            .expect("load state")
            .expect("state");
        assert_eq!(restored.selected.capsule(), &previous);
        assert!(previous.root.is_dir());
        assert!(!state.selected.capsule().root.exists());
    }

    #[test]
    fn seed_fallback_gc_removes_all_unreferenced_external_artifacts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let failed = artifact_capsule(&paths.root, 'a');
        let mut state = ControlState::initialize(TrustedSeed {
            capsule: capsule(temp.path(), "seed"),
            trust_anchor: temp.path().join("seed-anchor"),
            metadata: serde_json::Value::Null,
        });
        state.external_current = Some(failed.clone());
        state.selected = SelectedRuntime::external(failed.clone());
        crate::control::write_json_atomic(&paths.control.control, &state).expect("write state");

        restore_failed_external(
            &paths,
            &state,
            RuntimeFailure::load("load failed".to_string()),
            None,
        )
        .expect("restore Seed");

        let restored = ControlState::load(&paths.control)
            .expect("load state")
            .expect("state");
        assert!(matches!(restored.selected, SelectedRuntime::Seed { .. }));
        assert!(!failed.root.exists());
    }

    #[test]
    fn active_launch_reference_is_protected_during_gc() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let active = artifact_capsule(&paths.root, 'a');
        let current = artifact_capsule(&paths.root, 'b');
        let retired = artifact_capsule(&paths.root, 'c');
        let mut state = ControlState::initialize(TrustedSeed {
            capsule: capsule(temp.path(), "seed"),
            trust_anchor: temp.path().join("seed-anchor"),
            metadata: serde_json::Value::Null,
        });
        state.external_current = Some(current.clone());
        state.selected = SelectedRuntime::external(current.clone());
        state.active_launch = Some(ActiveLaunchRecord {
            selected: SelectedRuntime::external(active.clone()),
            launch_instance_id: "launch-1".to_string(),
            spawn_attempt_id: "spawn-1".to_string(),
            phase: ActiveLaunchPhase::SpawnPlanned,
            payload_registration: None,
            known_descendants: Vec::new(),
        });

        gc_unreferenced_external_artifacts_locked(&paths, &state);

        assert!(active.root.is_dir());
        assert!(current.root.is_dir());
        assert!(!retired.root.exists());
    }

    #[test]
    fn repeat_selection_of_current_capsule_is_a_control_no_op() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = external_state(temp.path(), true);
        let current = state
            .external_current
            .as_ref()
            .expect("current external capsule");

        assert!(selection_is_already_current(&state, current));
    }

    #[test]
    fn failed_external_selection_restores_previous_generation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = external_state(temp.path(), true);
        let failed = state.selected.capsule().clone();
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write external state");

        match restore_failed_external(
            &paths,
            &state,
            RuntimeFailure::load("load failed".to_string()),
            None,
        )
            .expect("restore previous")
        {
            ExternalFallback::Restored => {}
            ExternalFallback::SelectionChanged => panic!("selection did not change"),
        }
        let restored = ControlState::load(&paths.control)
            .expect("load restored state")
            .expect("restored state");
        assert_eq!(restored.selected.capsule().release_id, "release-previous");
        assert_eq!(
            restored
                .external_current
                .as_ref()
                .map(|capsule| capsule.release_id.as_str()),
            Some("release-previous")
        );
        assert!(restored.external_previous.is_none());

        let evidence =
            crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
                .expect("read fallback evidence")
                .expect("fallback evidence");
        assert_eq!(evidence.release_id, failed.release_id);
        assert_eq!(evidence.fallback_release_id.as_deref(), Some("release-previous"));
    }

    #[test]
    fn failed_only_external_selection_restores_seed() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = external_state(temp.path(), false);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write external state");

        match restore_failed_external(
            &paths,
            &state,
            RuntimeFailure::spawn_or_load("spawn failed".to_string()),
            None,
        )
            .expect("restore Seed")
        {
            ExternalFallback::Restored => {}
            ExternalFallback::SelectionChanged => panic!("selection did not change"),
        }
        let restored = ControlState::load(&paths.control)
            .expect("load restored state")
            .expect("restored state");
        assert!(matches!(restored.selected, SelectedRuntime::Seed { .. }));
        assert!(restored.external_current.is_none());
        assert!(restored.external_previous.is_none());
    }

    #[test]
    fn abnormal_external_exit_restores_previous_and_records_exit_code() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = external_state(temp.path(), true);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write external state");
        let status = Command::new("/bin/sh")
            .args(["-c", "exit 23"])
            .status()
            .expect("run exited payload");
        let failure =
            RuntimeFailure::from_exit_status(&status).expect("abnormal payload exit");

        handle_unexpected_payload_exit(&paths, "release-current", None, failure)
        .expect("restore previous");

        let restored = ControlState::load(&paths.control)
            .expect("load restored state")
            .expect("state");
        assert_eq!(restored.selected.capsule().release_id, "release-previous");
        let evidence =
            crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
                .expect("read evidence")
                .expect("evidence");
        assert_eq!(evidence.code, "payload_exit_code");
        assert_eq!(evidence.details.get("exitCode"), Some(&"23".to_string()));
        assert_eq!(evidence.fallback_release_id.as_deref(), Some("release-previous"));
    }

    #[test]
    fn signaled_external_exit_restores_seed_and_records_signal() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = external_state(temp.path(), false);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write external state");
        let status = Command::new("/bin/sh")
            .args(["-c", "kill -TERM $$"])
            .status()
            .expect("run signaled payload");
        let failure =
            RuntimeFailure::from_exit_status(&status).expect("abnormal payload exit");

        handle_unexpected_payload_exit(&paths, "release-current", None, failure)
        .expect("restore seed");

        let restored = ControlState::load(&paths.control)
            .expect("load restored state")
            .expect("state");
        assert!(matches!(restored.selected, SelectedRuntime::Seed { .. }));
        let evidence =
            crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
                .expect("read evidence")
                .expect("evidence");
        assert_eq!(evidence.code, "payload_exit_signal");
        assert_eq!(
            evidence.details.get("signal"),
            Some(&libc::SIGTERM.to_string())
        );
    }

    #[test]
    fn successful_exit_and_newer_selection_do_not_trigger_fallback() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = external_state(temp.path(), true);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write external state");
        let success = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .status()
            .expect("run successful payload");
        assert!(RuntimeFailure::from_exit_status(&success).is_none());

        let replacement = capsule(temp.path(), "replacement");
        let mut selected_after_exit = state.clone();
        selected_after_exit.external_previous = selected_after_exit.external_current.clone();
        selected_after_exit.external_current = Some(replacement.clone());
        selected_after_exit.selected = SelectedRuntime::external(replacement);
        crate::control::write_json_atomic(&paths.control.control, &selected_after_exit)
            .expect("write new selection");
        let failed = RuntimeFailure::from_exit_status(
            &Command::new("/bin/sh")
                .args(["-c", "exit 9"])
                .status()
                .expect("run failed payload"),
        )
        .expect("abnormal payload exit");

        handle_unexpected_payload_exit(&paths, "release-current", None, failed)
        .expect("ignore stale payload exit");

        let current = ControlState::load(&paths.control)
            .expect("load current state")
            .expect("state");
        assert_eq!(current.selected.capsule().release_id, "release-replacement");
        assert!(crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
            .expect("read evidence")
            .is_none());
    }

    #[test]
    fn capsule_switch_exit_accepts_identical_selection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = external_state(temp.path(), true);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write external state");

        let status = Command::new("/bin/sh")
            .args(["-c", &format!("exit {CAPSULE_SWITCH_EXIT_CODE}")])
            .status()
            .expect("run capsule switch exit");

        assert!(is_selected_capsule_switch(&status));
        assert!(
            RuntimeFailure::from_exit_status(&status).is_some(),
            "exit 75 is still an abnormal payload code outside the supervisor switch path"
        );
    }

    #[test]
    fn capsule_switch_exit_accepts_new_release_selection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = external_state(temp.path(), true);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write external state");

        let replacement = capsule(temp.path(), "replacement");
        let mut selected_after_exit = state.clone();
        selected_after_exit.external_previous = selected_after_exit.external_current.clone();
        selected_after_exit.external_current = Some(replacement.clone());
        selected_after_exit.selected = SelectedRuntime::external(replacement);
        crate::control::write_json_atomic(&paths.control.control, &selected_after_exit)
            .expect("write release selection");

        let status = Command::new("/bin/sh")
            .args(["-c", &format!("exit {CAPSULE_SWITCH_EXIT_CODE}")])
            .status()
            .expect("run capsule switch exit");

        assert!(is_selected_capsule_switch(&status));
    }

    #[test]
    fn early_exiting_payload_reason_is_preserved_when_launcher_selects_fallback() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = external_state(temp.path(), false);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write external state");
        crate::control::write_json_atomic(
            &paths.failure_evidence,
            &FailureProjection {
                activation_id: "payload-123".to_string(),
                release_id: "release-current".to_string(),
                occurred_at: "2026-09-10T00:00:00.000Z".to_string(),
                fallback_release_id: None,
                code: "payload_reported_error".to_string(),
                message: "renderer initialization failed".to_string(),
                failed: None,
                fallback: None,
                evidence_path: None,
                details: BTreeMap::from([
                    ("payloadPid".to_string(), "123".to_string()),
                    ("source".to_string(), "payload".to_string()),
                ]),
            },
        )
        .expect("write payload evidence");

        handle_unexpected_payload_exit(
            &paths,
            "release-current",
            Some(123),
            RuntimeFailure::spawn_or_load("fallback reason".to_string()),
        )
        .expect("restore seed");

        let evidence =
            crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
                .expect("read evidence")
                .expect("evidence");
        assert_eq!(evidence.code, "payload_reported_error");
        assert_eq!(evidence.message, "renderer initialization failed");
        assert_eq!(evidence.fallback_release_id.as_deref(), Some("release-seed"));
    }

    #[test]
    fn fallback_drops_a_stale_failure_after_a_new_selection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().join("state"));
        paths.ensure().expect("paths");
        let state = external_state(temp.path(), false);
        crate::control::write_json_atomic(&paths.control.control, &state)
            .expect("write external state");

        let replacement = capsule(temp.path(), "replacement");
        let mut selected_after_failure = state.clone();
        selected_after_failure.revision += 1;
        selected_after_failure.external_previous = selected_after_failure.external_current.clone();
        selected_after_failure.external_current = Some(replacement.clone());
        selected_after_failure.selected = SelectedRuntime::external(replacement);
        crate::control::write_json_atomic(&paths.control.control, &selected_after_failure)
            .expect("write newer selection");

        assert!(matches!(
            restore_failed_external(
                &paths,
                &state,
                RuntimeFailure::load("stale load failure".to_string()),
                None,
            )
            .expect("detect newer selection"),
            ExternalFallback::SelectionChanged
        ));
        assert!(crate::control::read_json_if_exists::<FailureProjection>(&paths.failure_evidence)
            .expect("read fallback evidence")
            .is_none());
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
