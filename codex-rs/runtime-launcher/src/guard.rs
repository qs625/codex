use crate::process::ProcessGroupRecord;
use crate::process::ProcessIdentity;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::fs::OpenOptions;
use std::io;
use std::io::Read;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::fd::FromRawFd;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

pub const GUARD_PROTOCOL_VERSION: u32 = 1;
const EXEC_STATUS_MAGIC: [u8; 4] = *b"MEXE";
const EXEC_STATUS_FRAME_LEN: usize = 12;
const EXECUTOR_LOCK_MODE: u32 = 0o600;
const READY_MARKER_MODE: u32 = 0o600;
const READY_VERIFIER_DOMAIN: &[u8] = b"runtime-capsule-ready-verifier-v1\0";
const REGISTRATION_DIGEST_DOMAIN: &[u8] = b"runtime-capsule-registration-v1\0";
pub const PARENT_LIVENESS_FD_ENV: &str = "RUNTIME_CAPSULE_GUARD_PARENT_FD";

#[derive(Debug)]
pub struct ExecutorLease {
    file: File,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl ExecutorLease {
    pub fn acquire(path: &Path) -> Result<Self, GuardError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(EXECUTOR_LOCK_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| GuardError::io(format!("open {}", path.display()), error))?;
        set_cloexec(file.as_raw_fd())?;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result != 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.raw_os_error(),
                Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN
            ) {
                return Err(GuardError::ExecutorBusy(path.to_path_buf()));
            }
            return Err(GuardError::io(
                format!("lock {}", path.display()),
                error,
            ));
        }
        let metadata = file
            .metadata()
            .map_err(|error| GuardError::io(format!("inspect {}", path.display()), error))?;
        let lease = Self {
            file,
            path: path.to_path_buf(),
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        lease.verify_stable_inode()?;
        Ok(lease)
    }

    pub fn verify_stable_inode(&self) -> Result<(), GuardError> {
        let metadata = std::fs::symlink_metadata(&self.path)
            .map_err(|error| GuardError::io(format!("inspect {}", self.path.display()), error))?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.mode() & 0o777 != EXECUTOR_LOCK_MODE
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.nlink() != 1
            || metadata.dev() != self.device
            || metadata.ino() != self.inode
        {
            return Err(GuardError::Blocked(format!(
                "executor lock path {} no longer names the leased regular-file inode",
                self.path.display()
            )));
        }
        Ok(())
    }

    pub fn as_file(&self) -> &File {
        &self.file
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardRegistration {
    pub protocol_version: u32,
    pub release_id: String,
    pub launch_instance_id: String,
    pub spawn_attempt_id: String,
    pub guard: ProcessIdentity,
    pub parent: ProcessIdentity,
}

impl GuardRegistration {
    pub fn validate(&self) -> Result<(), GuardError> {
        require_protocol(self.protocol_version)?;
        require_component("releaseId", &self.release_id)?;
        require_component("launchInstanceId", &self.launch_instance_id)?;
        require_component("spawnAttemptId", &self.spawn_attempt_id)?;
        Ok(())
    }

    pub fn digest(&self) -> Result<String, GuardError> {
        self.validate()?;
        domain_digest(REGISTRATION_DIGEST_DOMAIN, self)
    }

    pub fn register_and_sync(&self, path: &Path) -> Result<(), GuardError> {
        self.validate_live()?;
        write_private_json_synced(path, self)
    }

    pub fn validate_live(&self) -> Result<(), GuardError> {
        self.validate()?;
        let guard = ProcessIdentity::observe(self.guard.pid)
            .map_err(|error| GuardError::io("observe guard identity", error))?;
        let parent = ProcessIdentity::observe(self.parent.pid)
            .map_err(|error| GuardError::io("observe guard parent identity", error))?;
        if guard != Some(self.guard) || parent != Some(self.parent) {
            return Err(GuardError::Blocked(
                "guard registration identities are not both alive".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PayloadRegistration {
    pub protocol_version: u32,
    pub release_id: String,
    pub launch_instance_id: String,
    pub spawn_attempt_id: String,
    pub payload: ProcessIdentity,
    pub process_group: ProcessGroupRecord,
    pub guard_registration_digest: String,
}

impl PayloadRegistration {
    pub fn validate(&self, guard: &GuardRegistration) -> Result<(), GuardError> {
        require_protocol(self.protocol_version)?;
        if self.release_id != guard.release_id
            || self.launch_instance_id != guard.launch_instance_id
            || self.spawn_attempt_id != guard.spawn_attempt_id
        {
            return Err(GuardError::Blocked(
                "payload registration does not belong to the guard launch attempt".to_string(),
            ));
        }
        if self.process_group.leader != self.payload
            || self.process_group.pgid != self.payload.pid
        {
            return Err(GuardError::Blocked(
                "payload must lead its registered process group".to_string(),
            ));
        }
        if self.guard_registration_digest != guard.digest()? {
            return Err(GuardError::Blocked(
                "payload registration has the wrong guard registration digest".to_string(),
            ));
        }
        let observed = ProcessGroupRecord::observe(self.payload.pid)
            .map_err(|error| GuardError::io("observe payload process group", error))?;
        if observed != Some(self.process_group) {
            return Err(GuardError::Blocked(
                "payload identity or process group changed after registration".to_string(),
            ));
        }
        Ok(())
    }

    pub fn digest(&self) -> Result<String, GuardError> {
        domain_digest(REGISTRATION_DIGEST_DOMAIN, self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartAuthorization {
    pub protocol_version: u32,
    pub release_id: String,
    pub launch_instance_id: String,
    pub spawn_attempt_id: String,
    pub payload: ProcessIdentity,
    pub payload_registration_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuardLaunchRequest {
    pub protocol_version: u32,
    pub release_id: String,
    pub launch_instance_id: String,
    pub spawn_attempt_id: String,
    pub parent: ProcessIdentity,
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    pub guard_registration_path: PathBuf,
    pub guard_ack_path: PathBuf,
    pub payload_registration_path: PathBuf,
    pub start_authorization_path: PathBuf,
    pub exec_report_path: PathBuf,
    pub exit_report_path: PathBuf,
}

impl GuardLaunchRequest {
    pub fn validate(&self) -> Result<(), GuardError> {
        require_protocol(self.protocol_version)?;
        require_component("releaseId", &self.release_id)?;
        require_component("launchInstanceId", &self.launch_instance_id)?;
        require_component("spawnAttemptId", &self.spawn_attempt_id)?;
        if !self.executable.is_absolute() {
            return Err(GuardError::Protocol(
                "guard executable must be absolute".to_string(),
            ));
        }
        if self.cwd.as_ref().is_some_and(|cwd| !cwd.is_absolute()) {
            return Err(GuardError::Protocol(
                "guard cwd must be absolute".to_string(),
            ));
        }
        for path in [
            &self.guard_registration_path,
            &self.guard_ack_path,
            &self.payload_registration_path,
            &self.start_authorization_path,
            &self.exec_report_path,
            &self.exit_report_path,
        ] {
            if !path.is_absolute() {
                return Err(GuardError::Protocol(format!(
                    "guard protocol path must be absolute: {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GuardAcknowledgement {
    pub protocol_version: u32,
    pub guard_registration_digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome", deny_unknown_fields)]
pub enum GuardExecReport {
    ExecSucceeded { payload: ProcessIdentity },
    ExecFailed {
        payload: ProcessIdentity,
        errno: i32,
        bytes_written: u32,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome", deny_unknown_fields)]
pub enum GuardExitReport {
    Exited {
        release_id: String,
        launch_instance_id: String,
        spawn_attempt_id: String,
        payload: ProcessIdentity,
        raw_wait_status: i32,
        parent_liveness_lost: bool,
        cleanup_evidence: CleanupEvidence,
    },
    ContractViolated {
        release_id: String,
        launch_instance_id: String,
        spawn_attempt_id: String,
        payload: ProcessIdentity,
        observed_process: ProcessIdentity,
        observed_pgid: i32,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupEvidence {
    CooperativeObservedEmpty,
}

impl StartAuthorization {
    pub fn validate(&self, payload: &PayloadRegistration) -> Result<(), GuardError> {
        require_protocol(self.protocol_version)?;
        if self.release_id != payload.release_id
            || self.launch_instance_id != payload.launch_instance_id
            || self.spawn_attempt_id != payload.spawn_attempt_id
            || self.payload != payload.payload
            || self.payload_registration_digest
                != domain_digest(REGISTRATION_DIGEST_DOMAIN, payload)?
        {
            return Err(GuardError::Blocked(
                "StartAuthorized acknowledgement does not match the registered payload"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct StartGate {
    read: OwnedFd,
    write: OwnedFd,
}

impl StartGate {
    pub fn private_cloexec() -> Result<Self, GuardError> {
        let (read, write) = cloexec_pipe()?;
        Ok(Self { read, write })
    }

    pub fn split(self) -> (StartGateReader, StartGateWriter) {
        (
            StartGateReader { fd: self.read },
            StartGateWriter { fd: self.write },
        )
    }
}

#[derive(Debug)]
pub struct StartGateReader {
    fd: OwnedFd,
}

impl StartGateReader {
    pub fn as_raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub fn wait_for_authorization(self) -> Result<(), GuardError> {
        let mut byte = [0_u8; 1];
        let mut file = File::from(self.fd);
        file.read_exact(&mut byte)
            .map_err(|error| GuardError::io("wait for StartAuthorized gate", error))?;
        if byte != [1] {
            return Err(GuardError::Protocol(
                "invalid StartAuthorized gate byte".to_string(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct StartGateWriter {
    fd: OwnedFd,
}

impl StartGateWriter {
    pub fn as_raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub fn authorize(self) -> Result<(), GuardError> {
        let mut file = File::from(self.fd);
        file.write_all(&[1])
            .and_then(|_| file.flush())
            .map_err(|error| GuardError::io("open StartAuthorized gate", error))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyExpectation {
    pub protocol_version: u32,
    pub release_id: String,
    pub launch_instance_id: String,
    pub spawn_attempt_id: String,
    pub payload: ProcessIdentity,
    pub token_verifier: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyMarker {
    pub protocol_version: u32,
    pub release_id: String,
    pub launch_instance_id: String,
    pub spawn_attempt_id: String,
    pub pid: i32,
    pub start_identity: u64,
    pub token: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadyBearer {
    pub expectation: ReadyExpectation,
    pub token: String,
}

impl ReadyBearer {
    pub fn issue(
        release_id: String,
        launch_instance_id: String,
        spawn_attempt_id: String,
        payload: ProcessIdentity,
    ) -> Result<Self, GuardError> {
        require_component("releaseId", &release_id)?;
        require_component("launchInstanceId", &launch_instance_id)?;
        require_component("spawnAttemptId", &spawn_attempt_id)?;
        let token = random_token()?;
        let token_verifier = ready_token_verifier(
            GUARD_PROTOCOL_VERSION,
            &release_id,
            &launch_instance_id,
            &spawn_attempt_id,
            payload,
            &token,
        );
        Ok(Self {
            expectation: ReadyExpectation {
                protocol_version: GUARD_PROTOCOL_VERSION,
                release_id,
                launch_instance_id,
                spawn_attempt_id,
                payload,
                token_verifier,
            },
            token,
        })
    }

    pub fn bind(
        release_id: String,
        launch_instance_id: String,
        spawn_attempt_id: String,
        payload: ProcessIdentity,
        token: String,
    ) -> Result<Self, GuardError> {
        require_component("releaseId", &release_id)?;
        require_component("launchInstanceId", &launch_instance_id)?;
        require_component("spawnAttemptId", &spawn_attempt_id)?;
        require_component("readyToken", &token)?;
        let token_verifier = ready_token_verifier(
            GUARD_PROTOCOL_VERSION,
            &release_id,
            &launch_instance_id,
            &spawn_attempt_id,
            payload,
            &token,
        );
        Ok(Self {
            expectation: ReadyExpectation {
                protocol_version: GUARD_PROTOCOL_VERSION,
                release_id,
                launch_instance_id,
                spawn_attempt_id,
                payload,
                token_verifier,
            },
            token,
        })
    }

    pub fn marker(&self) -> ReadyMarker {
        ReadyMarker {
            protocol_version: self.expectation.protocol_version,
            release_id: self.expectation.release_id.clone(),
            launch_instance_id: self.expectation.launch_instance_id.clone(),
            spawn_attempt_id: self.expectation.spawn_attempt_id.clone(),
            pid: self.expectation.payload.pid,
            start_identity: self.expectation.payload.start_identity,
            token: self.token.clone(),
        }
    }
}

pub fn issue_ready_token() -> Result<String, GuardError> {
    random_token()
}

pub fn consume_ready_marker(
    path: &Path,
    expectation: &ReadyExpectation,
) -> Result<bool, GuardError> {
    expectation.validate()?;
    let file = match open_private_regular_file(path) {
        Ok(file) => file,
        Err(GuardError::Io { source, .. })
            if source.kind() == io::ErrorKind::NotFound =>
        {
            return Ok(false);
        }
        Err(error) => return Err(error),
    };
    let marker: ReadyMarker = serde_json::from_reader(&file)
        .map_err(|error| GuardError::Protocol(format!("parse ready marker: {error}")))?;
    if !expectation.matches(&marker) {
        return Err(GuardError::Blocked(
            "ready marker did not match its launch-bound bearer expectation".to_string(),
        ));
    }
    let observed = ProcessIdentity::observe(marker.pid)
        .map_err(|error| GuardError::io("observe ready payload identity", error))?;
    if observed != Some(expectation.payload) {
        return Err(GuardError::Blocked(
            "ready marker payload identity is no longer alive".to_string(),
        ));
    }
    let opened = file
        .metadata()
        .map_err(|error| GuardError::io(format!("reinspect {}", path.display()), error))?;
    let named = std::fs::symlink_metadata(path)
        .map_err(|error| GuardError::io(format!("reinspect {}", path.display()), error))?;
    if opened.dev() != named.dev() || opened.ino() != named.ino() {
        return Err(GuardError::Blocked(format!(
            "{} changed before its one-time consumption",
            path.display()
        )));
    }
    drop(file);
    std::fs::remove_file(path)
        .map_err(|error| GuardError::io(format!("consume {}", path.display()), error))?;
    sync_parent(path)?;
    Ok(true)
}

impl ReadyExpectation {
    pub fn validate(&self) -> Result<(), GuardError> {
        require_protocol(self.protocol_version)?;
        require_component("releaseId", &self.release_id)?;
        require_component("launchInstanceId", &self.launch_instance_id)?;
        require_component("spawnAttemptId", &self.spawn_attempt_id)?;
        require_hex_digest("tokenVerifier", &self.token_verifier)
    }

    fn matches(&self, marker: &ReadyMarker) -> bool {
        marker.protocol_version == self.protocol_version
            && marker.release_id == self.release_id
            && marker.launch_instance_id == self.launch_instance_id
            && marker.spawn_attempt_id == self.spawn_attempt_id
            && marker.pid == self.payload.pid
            && marker.start_identity == self.payload.start_identity
            && ready_token_verifier(
                marker.protocol_version,
                &marker.release_id,
                &marker.launch_instance_id,
                &marker.spawn_attempt_id,
                ProcessIdentity {
                    pid: marker.pid,
                    start_identity: marker.start_identity,
                },
                &marker.token,
            ) == self.token_verifier
    }
}

pub fn write_ready_marker(path: &Path, marker: &ReadyMarker) -> Result<(), GuardError> {
    write_private_json_synced(path, marker)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecFailure {
    pub errno: i32,
    pub bytes_written: u32,
}

#[derive(Debug)]
pub enum ExecStatus {
    ExecSucceeded(ProcessIdentity),
    ExecFailed(ExecFailure),
}

pub fn exec_status_pipe() -> Result<(ExecStatusReader, ExecStatusWriter), GuardError> {
    let (read, write) = cloexec_pipe()?;
    Ok((
        ExecStatusReader { fd: read },
        ExecStatusWriter { fd: write },
    ))
}

#[derive(Debug)]
pub struct ExecStatusReader {
    fd: OwnedFd,
}

impl ExecStatusReader {
    pub fn as_raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub fn read_after_spawn(
        self,
        expected_child: ProcessIdentity,
    ) -> Result<ExecStatus, GuardError> {
        let mut file = File::from(self.fd);
        let mut bytes = Vec::with_capacity(EXEC_STATUS_FRAME_LEN);
        file.read_to_end(&mut bytes)
            .map_err(|error| GuardError::io("read exec status", error))?;
        decode_exec_status(&bytes, expected_child)
    }
}

#[derive(Debug)]
pub struct ExecStatusWriter {
    fd: OwnedFd,
}

impl ExecStatusWriter {
    pub fn as_raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub fn report_exec_error(self, errno: i32) -> ! {
        let mut frame = [0_u8; EXEC_STATUS_FRAME_LEN];
        frame[..4].copy_from_slice(&EXEC_STATUS_MAGIC);
        frame[4..8].copy_from_slice(&errno.to_be_bytes());
        frame[8..12].copy_from_slice(&(EXEC_STATUS_FRAME_LEN as u32).to_be_bytes());
        let fd = self.fd.as_raw_fd();
        let mut written = 0;
        while written < frame.len() {
            let result = unsafe {
                libc::write(
                    fd,
                    frame[written..].as_ptr().cast(),
                    frame.len() - written,
                )
            };
            if result > 0 {
                written += result as usize;
                continue;
            }
            if result < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            break;
        }
        unsafe { libc::_exit(127) }
    }
}

pub fn parent_liveness_pipe() -> Result<(ParentLivenessMonitor, ParentLivenessKeeper), GuardError> {
    let (read, write) = cloexec_pipe()?;
    Ok((
        ParentLivenessMonitor { fd: read },
        ParentLivenessKeeper { fd: write },
    ))
}

#[derive(Debug)]
pub struct ParentLivenessMonitor {
    fd: OwnedFd,
}

impl ParentLivenessMonitor {
    pub unsafe fn from_raw_fd(fd: i32) -> Self {
        Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        }
    }

    pub fn as_raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }

    pub fn set_nonblocking(&self) -> Result<(), GuardError> {
        let flags = unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_GETFL) };
        if flags < 0 {
            return Err(GuardError::io(
                "read parent-liveness descriptor flags",
                io::Error::last_os_error(),
            ));
        }
        if unsafe { libc::fcntl(self.fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(GuardError::io(
                "set parent-liveness descriptor nonblocking",
                io::Error::last_os_error(),
            ));
        }
        Ok(())
    }

    pub fn parent_is_alive(&self) -> Result<bool, GuardError> {
        let mut byte = [0_u8; 1];
        let read = unsafe {
            libc::read(
                self.fd.as_raw_fd(),
                byte.as_mut_ptr().cast(),
                byte.len(),
            )
        };
        if read == 0 {
            return Ok(false);
        }
        if read > 0 {
            return Err(GuardError::Protocol(
                "parent liveness pipe carried unexpected data".to_string(),
            ));
        }
        let error = io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(code) if code == libc::EAGAIN || code == libc::EWOULDBLOCK)
        {
            Ok(true)
        } else if error.raw_os_error() == Some(libc::EINTR) {
            self.parent_is_alive()
        } else {
            Err(GuardError::io("watch parent liveness", error))
        }
    }

    pub fn allow_guard_exec(&self) -> Result<(), GuardError> {
        clear_cloexec(self.fd.as_raw_fd())
    }

    pub fn seal_after_guard_exec(&self) -> Result<(), GuardError> {
        set_cloexec(self.fd.as_raw_fd())
    }

    pub fn wait_for_parent_exit(self) -> Result<(), GuardError> {
        let mut file = File::from(self.fd);
        let mut byte = [0_u8; 1];
        match file.read(&mut byte) {
            Ok(0) => Ok(()),
            Ok(_) => Err(GuardError::Protocol(
                "parent liveness pipe carried unexpected data".to_string(),
            )),
            Err(error) => Err(GuardError::io("watch parent liveness", error)),
        }
    }
}

#[derive(Debug)]
pub struct ParentLivenessKeeper {
    fd: OwnedFd,
}

impl ParentLivenessKeeper {
    pub fn as_raw_fd(&self) -> i32 {
        self.fd.as_raw_fd()
    }
}

pub fn guard_hidden_mode_entrypoint<I, F>(
    arguments: I,
    run_guard: F,
) -> Result<bool, GuardError>
where
    I: IntoIterator<Item = OsString>,
    F: FnOnce(&Path) -> Result<(), GuardError>,
{
    let mut arguments = arguments.into_iter();
    let Some(mode) = arguments.next() else {
        return Ok(false);
    };
    if mode != "--runtime-capsule-guard" {
        return Ok(false);
    }
    let registration = arguments
        .next()
        .ok_or_else(|| GuardError::Protocol("guard mode requires registration path".to_string()))?;
    if arguments.next().is_some() {
        return Err(GuardError::Protocol(
            "guard mode accepts exactly one registration path".to_string(),
        ));
    }
    run_guard(Path::new(&registration))?;
    Ok(true)
}

pub fn run_hidden_guard(request_path: &Path) -> Result<(), GuardError> {
    let request: GuardLaunchRequest = read_private_json(request_path)?;
    request.validate()?;
    let parent_fd = std::env::var(PARENT_LIVENESS_FD_ENV)
        .map_err(|_| GuardError::Protocol("guard parent-liveness fd is missing".to_string()))?
        .parse::<i32>()
        .map_err(|_| GuardError::Protocol("guard parent-liveness fd is invalid".to_string()))?;
    let parent_monitor = unsafe { ParentLivenessMonitor::from_raw_fd(parent_fd) };
    parent_monitor.set_nonblocking()?;

    let guard_identity = crate::process::current_process_identity()
        .map_err(|error| GuardError::io("observe hidden guard identity", error))?;
    if ProcessIdentity::observe(request.parent.pid)
        .map_err(|error| GuardError::io("observe hidden guard parent", error))?
        != Some(request.parent)
    {
        return Err(GuardError::Blocked(
            "hidden guard parent identity changed before registration".to_string(),
        ));
    }
    let registration = GuardRegistration {
        protocol_version: GUARD_PROTOCOL_VERSION,
        release_id: request.release_id.clone(),
        launch_instance_id: request.launch_instance_id.clone(),
        spawn_attempt_id: request.spawn_attempt_id.clone(),
        guard: guard_identity,
        parent: request.parent,
    };
    registration.register_and_sync(&request.guard_registration_path)?;
    let acknowledgement: GuardAcknowledgement =
        wait_for_protocol_file(&request.guard_ack_path, &parent_monitor)?;
    if acknowledgement.protocol_version != GUARD_PROTOCOL_VERSION
        || acknowledgement.guard_registration_digest != registration.digest()?
    {
        return Err(GuardError::Blocked(
            "guard acknowledgement does not match durable self-registration".to_string(),
        ));
    }

    let gate = StartGate::private_cloexec()?;
    let (gate_reader, gate_writer) = gate.split();
    let (exec_reader, exec_writer) = exec_status_pipe()?;
    let forked = unsafe { libc::fork() };
    if forked < 0 {
        return Err(GuardError::io(
            "fork guarded payload",
            io::Error::last_os_error(),
        ));
    }
    if forked == 0 {
        drop(parent_monitor);
        drop(gate_writer);
        drop(exec_reader);
        if unsafe { libc::setpgid(0, 0) } != 0 {
            exec_writer.report_exec_error(io::Error::last_os_error().raw_os_error().unwrap_or(0));
        }
        if gate_reader.wait_for_authorization().is_err() {
            unsafe { libc::_exit(126) }
        }
        let payload_identity = match crate::process::current_process_identity() {
            Ok(identity) => identity,
            Err(error) => exec_writer.report_exec_error(error.raw_os_error().unwrap_or(0)),
        };
        let mut command = std::process::Command::new(&request.executable);
        command.env_clear();
        command.args(&request.arguments);
        if let Some(cwd) = &request.cwd {
            command.current_dir(cwd);
        }
        command.envs(&request.environment);
        command.env(
            "RUNTIME_CAPSULE_START_IDENTITY",
            payload_identity.start_identity.to_string(),
        );
        let error = command.exec();
        exec_writer.report_exec_error(error.raw_os_error().unwrap_or(0));
    }

    drop(gate_reader);
    drop(exec_writer);
    let payload_pid = forked;
    let payload = observe_dedicated_payload(payload_pid, &parent_monitor)?;
    let process_group = ProcessGroupRecord::observe(payload.pid)
        .map_err(|error| GuardError::io("observe registered payload group", error))?
        .ok_or_else(|| GuardError::Blocked("guarded payload disappeared".to_string()))?
        .require_dedicated()
        .map_err(|error| GuardError::Blocked(error.to_string()))?;
    let payload_registration = PayloadRegistration {
        protocol_version: GUARD_PROTOCOL_VERSION,
        release_id: request.release_id.clone(),
        launch_instance_id: request.launch_instance_id.clone(),
        spawn_attempt_id: request.spawn_attempt_id.clone(),
        payload,
        process_group,
        guard_registration_digest: registration.digest()?,
    };
    payload_registration.validate(&registration)?;
    write_private_json_synced(&request.payload_registration_path, &payload_registration)?;

    let authorization: StartAuthorization =
        match wait_for_protocol_file(&request.start_authorization_path, &parent_monitor) {
            Ok(authorization) => authorization,
            Err(error) => {
                cleanup_guarded_payload(
                    payload,
                    process_group,
                    guard_identity,
                    BTreeSet::new(),
                )?;
                return Err(error);
            }
        };
    if let Err(error) = authorization.validate(&payload_registration) {
        cleanup_guarded_payload(payload, process_group, guard_identity, BTreeSet::new())?;
        return Err(error);
    }
    if let Err(error) = gate_writer.authorize() {
        cleanup_guarded_payload(payload, process_group, guard_identity, BTreeSet::new())?;
        return Err(error);
    }

    let exec_status = match exec_reader.read_after_spawn(payload) {
        Ok(status) => status,
        Err(error) => {
            cleanup_guarded_payload(payload, process_group, guard_identity, BTreeSet::new())?;
            return Err(error);
        }
    };
    let exec_report = match exec_status {
        ExecStatus::ExecSucceeded(payload) => GuardExecReport::ExecSucceeded { payload },
        ExecStatus::ExecFailed(failure) => GuardExecReport::ExecFailed {
            payload,
            errno: failure.errno,
            bytes_written: failure.bytes_written,
        },
    };
    write_private_json_synced(&request.exec_report_path, &exec_report)?;

    let mut parent_liveness_lost = false;
    let mut known_descendants = BTreeSet::new();
    let mut payload_exit_status = None;
    let raw_wait_status = loop {
        let snapshot = crate::process::snapshot_processes()
            .map_err(|error| GuardError::Blocked(error.to_string()))?;
        extend_observed_descendants(&mut known_descendants, payload, &snapshot);
        let ambiguous_group_member = snapshot.iter().find(|process| {
            process.pgid == process_group.pgid
            && process.identity != payload
            && !known_descendants.contains(&process.identity)
        });
        if let Some(process) = ambiguous_group_member {
            known_descendants.insert(process.identity);
            let message = format!(
                "cooperative process contract violated by untracked process {}/{} in guarded pgid {}",
                process.identity.pid,
                process.identity.start_identity,
                process.pgid
            );
            write_private_json_synced(
                &request.exit_report_path,
                &GuardExitReport::ContractViolated {
                    release_id: request.release_id.clone(),
                    launch_instance_id: request.launch_instance_id.clone(),
                    spawn_attempt_id: request.spawn_attempt_id.clone(),
                    payload,
                    observed_process: process.identity,
                    observed_pgid: process.pgid,
                    message: message.clone(),
                },
            )?;
            cleanup_guarded_payload(
                payload,
                process_group,
                guard_identity,
                known_descendants.clone(),
            )?;
            return Err(GuardError::Blocked(message));
        }
        if payload_exit_status.is_none() {
            let mut status = 0_i32;
            let waited = unsafe { libc::waitpid(payload.pid, &mut status, libc::WNOHANG) };
            if waited == payload.pid {
                payload_exit_status = Some(status);
            }
            if waited < 0 {
                return Err(GuardError::io(
                    "wait for guarded payload",
                    io::Error::last_os_error(),
                ));
            }
        }
        if let Some(status) = payload_exit_status
            && observed_identities_are_gone(&known_descendants)?
        {
            break status;
        }
        if !parent_monitor.parent_is_alive()? {
            parent_liveness_lost = true;
            cleanup_guarded_payload(
                payload,
                process_group,
                guard_identity,
                known_descendants.clone(),
            )?;
            let mut final_status = payload_exit_status.unwrap_or_default();
            let waited = unsafe { libc::waitpid(payload.pid, &mut final_status, 0) };
            if waited == payload.pid {
                break final_status;
            }
            if waited < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD) {
                break final_status;
            }
            return Err(GuardError::io(
                "reap guarded payload after parent loss",
                io::Error::last_os_error(),
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    write_private_json_synced(
        &request.exit_report_path,
        &GuardExitReport::Exited {
            release_id: request.release_id.clone(),
            launch_instance_id: request.launch_instance_id.clone(),
            spawn_attempt_id: request.spawn_attempt_id.clone(),
            payload,
            raw_wait_status,
            parent_liveness_lost,
            cleanup_evidence: CleanupEvidence::CooperativeObservedEmpty,
        },
    )
}

fn extend_observed_descendants(
    known_descendants: &mut BTreeSet<ProcessIdentity>,
    payload: ProcessIdentity,
    snapshot: &[crate::process::ProcessRecord],
) {
    let roots = std::iter::once(payload)
        .chain(known_descendants.iter().copied())
        .collect::<Vec<_>>();
    for root in roots {
        known_descendants.extend(crate::process::descendants_of(root, snapshot));
    }
}

fn observed_identities_are_gone(
    identities: &BTreeSet<ProcessIdentity>,
) -> Result<bool, GuardError> {
    for identity in identities {
        if identity
            .is_alive()
            .map_err(|error| GuardError::io("observe handed-off payload descendant", error))?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn write_protocol_file<T: Serialize>(path: &Path, value: &T) -> Result<(), GuardError> {
    write_private_json_synced(path, value)
}

pub fn read_protocol_file_if_exists<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, GuardError> {
    match open_private_regular_file(path) {
        Ok(file) => serde_json::from_reader(file)
            .map(Some)
            .map_err(|error| GuardError::Protocol(format!("parse {}: {error}", path.display()))),
        Err(GuardError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_private_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, GuardError> {
    let file = open_private_regular_file(path)?;
    serde_json::from_reader(file)
        .map_err(|error| GuardError::Protocol(format!("parse {}: {error}", path.display())))
}

fn wait_for_protocol_file<T: serde::de::DeserializeOwned>(
    path: &Path,
    parent_monitor: &ParentLivenessMonitor,
) -> Result<T, GuardError> {
    loop {
        if let Some(value) = read_protocol_file_if_exists(path)? {
            return Ok(value);
        }
        if !parent_monitor.parent_is_alive()? {
            return Err(GuardError::Blocked(
                "launcher parent liveness was lost before authorization".to_string(),
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn observe_dedicated_payload(
    pid: i32,
    parent_monitor: &ParentLivenessMonitor,
) -> Result<ProcessIdentity, GuardError> {
    loop {
        if !parent_monitor.parent_is_alive()? {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, std::ptr::null_mut(), 0);
            }
            return Err(GuardError::Blocked(
                "launcher parent liveness was lost before payload registration".to_string(),
            ));
        }
        if let Some(group) = ProcessGroupRecord::observe(pid)
            .map_err(|error| GuardError::io("observe forked payload", error))?
        {
            if group.pgid == pid {
                return Ok(group.leader);
            }
        } else {
            return Err(GuardError::Blocked(
                "forked payload disappeared before durable registration".to_string(),
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn cleanup_guarded_payload(
    payload: ProcessIdentity,
    process_group: ProcessGroupRecord,
    guard_identity: ProcessIdentity,
    mut known_descendants: BTreeSet<ProcessIdentity>,
) -> Result<(), GuardError> {
    let policy = crate::process::TerminationPolicy::default();
    let initial = crate::process::snapshot_processes()
        .map_err(|error| GuardError::Blocked(error.to_string()))?;
    known_descendants.extend(crate::process::descendants_of(payload, &initial));
    let leader_alive = payload
        .is_alive()
        .map_err(|error| GuardError::io("observe guarded payload leader", error))?;
    let group_members = initial
        .iter()
        .filter(|process| process.pgid == process_group.pgid)
        .map(|process| process.identity)
        .collect::<BTreeSet<_>>();
    if !leader_alive
        && group_members
            .iter()
            .any(|identity| !known_descendants.contains(identity))
    {
        return Err(GuardError::Blocked(format!(
            "payload leader identity is gone and pgid {} contains an untracked member",
            process_group.pgid
        )));
    }
    let mut status = 0_i32;
    let mut reaped = false;
    if leader_alive {
        let term = unsafe { libc::killpg(process_group.pgid, libc::SIGTERM) };
        if term != 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
            return Err(GuardError::io(
                "terminate guarded payload group",
                io::Error::last_os_error(),
            ));
        }
    } else {
        for identity in &known_descendants {
            signal_observed_identity(*identity, libc::SIGTERM)?;
        }
    }
    let term_deadline = std::time::Instant::now() + policy.term_timeout;
    while std::time::Instant::now() < term_deadline {
        let waited = unsafe { libc::waitpid(payload.pid, &mut status, libc::WNOHANG) };
        if waited == payload.pid || (waited < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD)) {
            reaped = true;
            break;
        }
        if waited < 0 {
            return Err(GuardError::io(
                "reap terminated guarded payload",
                io::Error::last_os_error(),
            ));
        }
        std::thread::sleep(policy.poll_interval);
    }
    if !reaped {
        if leader_alive {
            let killed = unsafe { libc::killpg(process_group.pgid, libc::SIGKILL) };
            if killed != 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                return Err(GuardError::io(
                    "kill guarded payload group",
                    io::Error::last_os_error(),
                ));
            }
        } else {
            for identity in &known_descendants {
                signal_observed_identity(*identity, libc::SIGKILL)?;
            }
        }
        let waited = unsafe { libc::waitpid(payload.pid, &mut status, 0) };
        if waited != payload.pid
            && !(waited < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ECHILD))
        {
            return Err(GuardError::io(
                "reap killed guarded payload",
                io::Error::last_os_error(),
            ));
        }
    }
    let deadline = std::time::Instant::now() + policy.kill_timeout;
    loop {
        let group_alive = crate::process::snapshot_processes()
            .map_err(|error| GuardError::Blocked(error.to_string()))?
            .into_iter()
            .any(|process| process.pgid == process_group.pgid);
        let payload_alive = payload
            .is_alive()
            .map_err(|error| GuardError::io("observe guarded payload exit", error))?;
        let escaped_alive = known_descendants
            .iter()
            .map(|identity| {
                identity
                    .is_alive()
                    .map_err(|error| GuardError::io("observe guarded descendant exit", error))
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .any(|alive| alive);
        if !group_alive && !payload_alive && !escaped_alive {
            return Ok(());
        }
        for identity in &known_descendants {
            if identity
                .is_alive()
                .map_err(|error| GuardError::io("observe escaped guarded descendant", error))?
            {
                let result = unsafe { libc::kill(identity.pid, libc::SIGKILL) };
                if result != 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                    return Err(GuardError::io(
                        "kill escaped guarded descendant",
                        io::Error::last_os_error(),
                    ));
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(GuardError::Blocked(format!(
                "guard {} could not confirm cooperative cleanup for payload group {}",
                guard_identity.pid, process_group.pgid
            )));
        }
        std::thread::sleep(policy.poll_interval);
    }
}

fn signal_observed_identity(identity: ProcessIdentity, signal: i32) -> Result<(), GuardError> {
    if !identity
        .is_alive()
        .map_err(|error| GuardError::io("observe tracked descendant", error))?
    {
        return Ok(());
    }
    let result = unsafe { libc::kill(identity.pid, signal) };
    if result != 0 && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
        return Err(GuardError::io(
            "signal tracked descendant",
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[derive(Debug)]
pub enum GuardError {
    Io { context: String, source: io::Error },
    ExecutorBusy(PathBuf),
    Protocol(String),
    Blocked(String),
}

impl GuardError {
    fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}

impl fmt::Display for GuardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
            Self::ExecutorBusy(path) => {
                write!(formatter, "executor lock is already held: {}", path.display())
            }
            Self::Protocol(message) => write!(formatter, "guard protocol error: {message}"),
            Self::Blocked(message) => write!(formatter, "guard operation blocked: {message}"),
        }
    }
}

impl std::error::Error for GuardError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::ExecutorBusy(_) | Self::Protocol(_) | Self::Blocked(_) => None,
        }
    }
}

fn decode_exec_status(
    bytes: &[u8],
    expected_child: ProcessIdentity,
) -> Result<ExecStatus, GuardError> {
    if bytes.is_empty() {
        let observed = ProcessIdentity::observe(expected_child.pid)
            .map_err(|error| GuardError::io("confirm child after exec EOF", error))?;
        return if observed == Some(expected_child) {
            Ok(ExecStatus::ExecSucceeded(expected_child))
        } else {
            Err(GuardError::Blocked(
                "exec status reached EOF but the registered child identity was not alive"
                    .to_string(),
            ))
        };
    }
    if bytes.len() != EXEC_STATUS_FRAME_LEN {
        return Err(GuardError::Protocol(format!(
            "partial exec-error frame: received {} of {} bytes",
            bytes.len(),
            EXEC_STATUS_FRAME_LEN
        )));
    }
    if bytes[..4] != EXEC_STATUS_MAGIC {
        return Err(GuardError::Protocol(
            "invalid exec-error frame magic".to_string(),
        ));
    }
    let errno = i32::from_be_bytes(bytes[4..8].try_into().expect("four-byte errno"));
    let bytes_written =
        u32::from_be_bytes(bytes[8..12].try_into().expect("four-byte frame length"));
    if bytes_written as usize != EXEC_STATUS_FRAME_LEN {
        return Err(GuardError::Protocol(format!(
            "exec-error writer reported a partial frame of {bytes_written} bytes"
        )));
    }
    Ok(ExecStatus::ExecFailed(ExecFailure {
        errno,
        bytes_written,
    }))
}

fn write_private_json_synced<T: Serialize>(path: &Path, value: &T) -> Result<(), GuardError> {
    let parent = path.parent().ok_or_else(|| {
        GuardError::Protocol(format!("{} has no parent directory", path.display()))
    })?;
    let name = path.file_name().ok_or_else(|| {
        GuardError::Protocol(format!("{} has no file name", path.display()))
    })?;
    let temporary = parent.join(format!(".{}.tmp-{}", name.to_string_lossy(), unsafe {
        libc::getpid()
    }));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(READY_MARKER_MODE)
        .open(&temporary)
        .map_err(|error| GuardError::io(format!("create {}", temporary.display()), error))?;
    let result = (|| {
        serde_json::to_writer(&mut file, value)
            .map_err(|error| GuardError::Protocol(format!("serialize {}: {error}", path.display())))?;
        file.write_all(b"\n")
            .and_then(|_| file.sync_all())
            .map_err(|error| GuardError::io(format!("sync {}", temporary.display()), error))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| GuardError::io(format!("replace {}", path.display()), error))?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

fn open_private_regular_file(path: &Path) -> Result<File, GuardError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| GuardError::io(format!("inspect {}", path.display()), error))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.mode() & 0o777 != READY_MARKER_MODE
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.nlink() != 1
    {
        return Err(GuardError::Blocked(format!(
            "{} must be a 0600 regular file owned by the effective uid with link count one",
            path.display()
        )));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| GuardError::io(format!("open {}", path.display()), error))?;
    let opened = file
        .metadata()
        .map_err(|error| GuardError::io(format!("inspect opened {}", path.display()), error))?;
    if opened.dev() != metadata.dev() || opened.ino() != metadata.ino() {
        return Err(GuardError::Blocked(format!(
            "{} changed while it was being opened",
            path.display()
        )));
    }
    Ok(file)
}

fn sync_parent(path: &Path) -> Result<(), GuardError> {
    let parent = path.parent().ok_or_else(|| {
        GuardError::Protocol(format!("{} has no parent directory", path.display()))
    })?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| GuardError::io(format!("sync {}", parent.display()), error))
}

fn cloexec_pipe() -> Result<(OwnedFd, OwnedFd), GuardError> {
    let mut fds = [-1; 2];
    #[cfg(any(target_os = "linux", target_os = "android"))]
    let result = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let result = unsafe { libc::pipe(fds.as_mut_ptr()) };
    if result != 0 {
        return Err(GuardError::io(
            "create CLOEXEC pipe",
            io::Error::last_os_error(),
        ));
    }
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        set_cloexec(read.as_raw_fd())?;
        set_cloexec(write.as_raw_fd())?;
    }
    Ok((read, write))
}

pub fn set_cloexec(fd: i32) -> Result<(), GuardError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(GuardError::io(
            "read descriptor flags",
            io::Error::last_os_error(),
        ));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(GuardError::io(
            "set FD_CLOEXEC",
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

pub fn clear_cloexec(fd: i32) -> Result<(), GuardError> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(GuardError::io(
            "read descriptor flags",
            io::Error::last_os_error(),
        ));
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) } < 0 {
        return Err(GuardError::io(
            "clear FD_CLOEXEC",
            io::Error::last_os_error(),
        ));
    }
    Ok(())
}

fn random_token() -> Result<String, GuardError> {
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| GuardError::io("read ready bearer entropy", error))?;
    Ok(hex(&bytes))
}

fn ready_token_verifier(
    protocol_version: u32,
    release_id: &str,
    launch_instance_id: &str,
    spawn_attempt_id: &str,
    payload: ProcessIdentity,
    token: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(READY_VERIFIER_DOMAIN);
    update_field(&mut digest, &protocol_version.to_be_bytes());
    update_field(&mut digest, release_id.as_bytes());
    update_field(&mut digest, launch_instance_id.as_bytes());
    update_field(&mut digest, spawn_attempt_id.as_bytes());
    update_field(&mut digest, &payload.pid.to_be_bytes());
    update_field(&mut digest, &payload.start_identity.to_be_bytes());
    update_field(&mut digest, token.as_bytes());
    hex(&digest.finalize())
}

fn domain_digest<T: Serialize>(domain: &[u8], value: &T) -> Result<String, GuardError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|error| GuardError::Protocol(format!("serialize protocol digest: {error}")))?;
    let mut digest = Sha256::new();
    digest.update(domain);
    update_field(&mut digest, &bytes);
    Ok(hex(&digest.finalize()))
}

fn update_field(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_be_bytes());
    digest.update(value);
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn require_protocol(protocol_version: u32) -> Result<(), GuardError> {
    if protocol_version != GUARD_PROTOCOL_VERSION {
        return Err(GuardError::Protocol(format!(
            "unsupported guard protocol version {protocol_version}"
        )));
    }
    Ok(())
}

fn require_component(name: &str, value: &str) -> Result<(), GuardError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(GuardError::Protocol(format!("invalid {name}")));
    }
    Ok(())
}

fn require_hex_digest(name: &str, value: &str) -> Result<(), GuardError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GuardError::Protocol(format!("invalid {name}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> ProcessIdentity {
        ProcessIdentity {
            pid: 12,
            start_identity: 34,
        }
    }

    #[test]
    fn ready_verifier_is_bound_to_every_launch_identity_field() {
        let original = ready_token_verifier(1, "release", "launch", "attempt", identity(), "token");
        assert_ne!(
            original,
            ready_token_verifier(1, "other", "launch", "attempt", identity(), "token")
        );
        assert_ne!(
            original,
            ready_token_verifier(1, "release", "other", "attempt", identity(), "token")
        );
        assert_ne!(
            original,
            ready_token_verifier(1, "release", "launch", "other", identity(), "token")
        );
        assert_ne!(
            original,
            ready_token_verifier(
                1,
                "release",
                "launch",
                "attempt",
                ProcessIdentity {
                    pid: 13,
                    start_identity: 34,
                },
                "token"
            )
        );
    }

    #[test]
    fn exec_error_decoder_rejects_partial_frames() {
        let error = decode_exec_status(&EXEC_STATUS_MAGIC, identity()).expect_err("partial");
        assert!(matches!(error, GuardError::Protocol(message) if message.contains("partial")));
    }

    #[test]
    fn exec_error_decoder_preserves_errno() {
        let mut frame = [0_u8; EXEC_STATUS_FRAME_LEN];
        frame[..4].copy_from_slice(&EXEC_STATUS_MAGIC);
        frame[4..8].copy_from_slice(&libc::ENOENT.to_be_bytes());
        frame[8..].copy_from_slice(&(EXEC_STATUS_FRAME_LEN as u32).to_be_bytes());
        let status = decode_exec_status(&frame, identity()).expect("decode");
        assert!(matches!(
            status,
            ExecStatus::ExecFailed(ExecFailure {
                errno: libc::ENOENT,
                bytes_written: 12
            })
        ));
    }

    #[test]
    fn hidden_guard_mode_is_exact_and_does_not_claim_other_arguments() {
        let mut called = false;
        assert!(
            !guard_hidden_mode_entrypoint([OsString::from("--status")], |_| {
                called = true;
                Ok(())
            })
            .expect("entrypoint")
        );
        assert!(!called);
    }

    #[test]
    fn tracks_descendants_after_the_payload_hands_off_its_process_group() {
        let payload = ProcessIdentity {
            pid: 10,
            start_identity: 100,
        };
        let handed_off = ProcessIdentity {
            pid: 11,
            start_identity: 110,
        };
        let child = ProcessIdentity {
            pid: 12,
            start_identity: 120,
        };
        let mut known = BTreeSet::new();
        extend_observed_descendants(
            &mut known,
            payload,
            &[crate::process::ProcessRecord {
                identity: handed_off,
                parent_pid: payload.pid,
                pgid: handed_off.pid,
            }],
        );
        extend_observed_descendants(
            &mut known,
            payload,
            &[
                crate::process::ProcessRecord {
                    identity: handed_off,
                    parent_pid: 1,
                    pgid: handed_off.pid,
                },
                crate::process::ProcessRecord {
                    identity: child,
                    parent_pid: handed_off.pid,
                    pgid: handed_off.pid,
                },
            ],
        );
        assert_eq!(known, BTreeSet::from([handed_off, child]));
    }
}
