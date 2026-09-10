use crate::LauncherError;
use crate::SelectCandidateRequest;
use crate::Result;
use crate::capsule::CapsuleRecord;
use crate::capsule::CapsuleTarget;
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
    if selection_is_already_current(&current, &candidate) {
        return Ok(SelectCandidateResult {
            activation_id: request.activation_id,
            release_id,
            control: current,
        });
    }
    let control = update_locked(&paths.control, |state| {
        state.external_previous = state.external_current.clone();
        state.external_current = Some(candidate.clone());
        state.selected = SelectedRuntime::external(candidate.clone());
        Ok(())
    })?;
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

    loop {
        let state = ControlState::load(&paths.control)?
            .ok_or_else(|| LauncherError::Conflict("control state disappeared".to_string()))?;
        let selected = match load_selected(&state, &target) {
            Ok(selected) => selected,
            Err(error) if matches!(state.selected, SelectedRuntime::External { .. }) => {
                let _ = restore_failed_external(paths, &state, error.to_string())?;
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
                    let _ = restore_failed_external(paths, &latest, error.to_string())?;
                    continue;
                }
                return Err(error);
            }
        };
        let code = outcome.status.code().unwrap_or(1);
        if code != 0 {
            write_runtime_diagnostic(paths, &selected.release_id, code, "payload exited")?;
        }
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
    let (launch_instance_id, spawn_attempt_id) = launch_identity();
    let _reserved = reserve_active_launch(paths, state, &launch_instance_id, &spawn_attempt_id)?;
    let mut command = std::process::Command::new(&verified.executable);
    command
        .args(&verified.manifest.launch.arguments)
        .env_clear()
        .envs(build_payload_environment(
            paths,
            launcher_path,
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
        return Ok(LaunchOutcome { status });
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
            return Ok(LaunchOutcome { status });
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
    reason: String,
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
    crate::control::write_json_atomic(
        &paths.failure_evidence,
        &FailureProjection {
            activation_id: format!("select-{}", unix_time_ms()),
            release_id: failed.release_id.clone(),
            occurred_at: rfc3339_now(),
            fallback_release_id: Some(restored.selected.capsule().release_id.clone()),
            code: "selected_runtime_spawn_failed".to_string(),
            message: reason,
            failed: Some(failed),
            fallback: Some(restored.selected.clone()),
            evidence_path: Some(paths.failure_evidence.clone()),
            details: BTreeMap::new(),
        },
    )?;
    Ok(ExternalFallback::Restored)
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

        match restore_failed_external(&paths, &state, "load failed".to_string())
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

        match restore_failed_external(&paths, &state, "spawn failed".to_string())
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
            restore_failed_external(&paths, &state, "stale load failure".to_string())
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
