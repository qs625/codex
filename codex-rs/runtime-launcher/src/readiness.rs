use crate::LauncherError;
use crate::Result;
use crate::control::write_json_atomic;
use crate::process::ProcessIdentity;
use serde::Deserialize;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

pub const READY_PROTOCOL_VERSION: u32 = 1;
const READY_VERIFIER_DOMAIN: &[u8] = b"runtime-capsule-ready-verifier-v1\0";

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
    ) -> Result<Self> {
        validate_component("releaseId", &release_id)?;
        validate_component("launchInstanceId", &launch_instance_id)?;
        validate_component("spawnAttemptId", &spawn_attempt_id)?;
        let token = random_token()?;
        Ok(Self {
            expectation: ReadyExpectation {
                protocol_version: READY_PROTOCOL_VERSION,
                token_verifier: ready_token_verifier(
                    READY_PROTOCOL_VERSION,
                    &release_id,
                    &launch_instance_id,
                    &spawn_attempt_id,
                    payload,
                    &token,
                ),
                release_id,
                launch_instance_id,
                spawn_attempt_id,
                payload,
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

    pub fn bind(
        release_id: String,
        launch_instance_id: String,
        spawn_attempt_id: String,
        payload: ProcessIdentity,
        token: String,
    ) -> Result<Self> {
        validate_component("releaseId", &release_id)?;
        validate_component("launchInstanceId", &launch_instance_id)?;
        validate_component("spawnAttemptId", &spawn_attempt_id)?;
        validate_component("readyToken", &token)?;
        Ok(Self {
            expectation: ReadyExpectation {
                protocol_version: READY_PROTOCOL_VERSION,
                token_verifier: ready_token_verifier(
                    READY_PROTOCOL_VERSION,
                    &release_id,
                    &launch_instance_id,
                    &spawn_attempt_id,
                    payload,
                    &token,
                ),
                release_id,
                launch_instance_id,
                spawn_attempt_id,
                payload,
            },
            token,
        })
    }
}

pub fn issue_ready_token() -> Result<String> {
    random_token()
}

impl ReadyExpectation {
    pub fn validate(&self) -> Result<()> {
        if self.protocol_version != READY_PROTOCOL_VERSION {
            return Err(LauncherError::Conflict(format!(
                "unsupported readiness protocol {}",
                self.protocol_version
            )));
        }
        validate_component("releaseId", &self.release_id)?;
        validate_component("launchInstanceId", &self.launch_instance_id)?;
        validate_component("spawnAttemptId", &self.spawn_attempt_id)?;
        if self.token_verifier.len() != 64
            || !self
                .token_verifier
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(LauncherError::Conflict(
                "readiness token verifier is invalid".to_string(),
            ));
        }
        Ok(())
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

pub fn consume_ready_marker(path: &Path, expectation: &ReadyExpectation) -> Result<bool> {
    expectation.validate()?;
    let mut options = std::fs::OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    let file = match options.open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(crate::io_error(format!("open {}", path.display()), error)),
    };
    let opened = file
        .metadata()
        .map_err(|error| crate::io_error(format!("inspect {}", path.display()), error))?;
    if !opened.is_file()
        || opened.mode() & 0o777 != 0o600
        || opened.uid() != unsafe { libc::geteuid() }
    {
        return Err(LauncherError::Launch(
            "ready marker must be an owner-only regular file".to_string(),
        ));
    }
    let marker = serde_json::from_reader::<_, ReadyMarker>(&file)
        .map_err(|error| crate::json_error(format!("parse {}", path.display()), error))?;
    if !expectation.matches(&marker) {
        return Err(LauncherError::Launch(
            "ready marker did not match its payload launch".to_string(),
        ));
    }
    if ProcessIdentity::observe(marker.pid)
        .map_err(|error| crate::io_error("observe ready payload", error))?
        != Some(expectation.payload)
    {
        return Err(LauncherError::Launch(
            "ready marker payload is no longer the launched process".to_string(),
        ));
    }
    let named = std::fs::symlink_metadata(path)
        .map_err(|error| crate::io_error(format!("reinspect {}", path.display()), error))?;
    if named.file_type().is_symlink() || named.dev() != opened.dev() || named.ino() != opened.ino()
    {
        return Err(LauncherError::Launch(
            "ready marker changed before consumption".to_string(),
        ));
    }
    drop(file);
    std::fs::remove_file(path)
        .map_err(|error| crate::io_error(format!("consume {}", path.display()), error))?;
    Ok(true)
}

pub fn write_ready_marker(path: &Path, marker: &ReadyMarker) -> Result<()> {
    write_json_atomic(path, marker)
}

fn random_token() -> Result<String> {
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes))
        .map_err(|error| crate::io_error("read readiness random token", error))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
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
    digest.update(protocol_version.to_be_bytes());
    for component in [
        release_id.as_bytes(),
        launch_instance_id.as_bytes(),
        spawn_attempt_id.as_bytes(),
    ] {
        digest.update((component.len() as u64).to_be_bytes());
        digest.update(component);
    }
    digest.update(payload.pid.to_be_bytes());
    digest.update(payload.start_identity.to_be_bytes());
    digest.update(token.as_bytes());
    format!("{:x}", digest.finalize())
}

fn validate_component(name: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(LauncherError::InvalidRequest(format!(
            "{name} must not be empty"
        )));
    }
    Ok(())
}
