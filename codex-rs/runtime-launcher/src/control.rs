use crate::LauncherError;
use crate::Result;
use crate::io_error;
use crate::json_error;
use crate::process::ProcessGroupRecord;
use crate::process::ProcessIdentity;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub const CONTROL_SCHEMA_VERSION: u32 = 3;

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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveLaunchPhase {
    SpawnPlanned,
    Running,
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
pub struct ActiveLaunchRecord {
    pub selected: SelectedRuntime,
    pub launch_instance_id: String,
    pub spawn_attempt_id: String,
    pub phase: ActiveLaunchPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_registration: Option<PayloadRegistration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub known_descendants: Vec<ProcessIdentity>,
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
        if self.external_current.is_none() && self.external_previous.is_some() {
            return Err(LauncherError::Conflict(
                "external previous requires an external current capsule".to_string(),
            ));
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
        if let Some(active) = &self.active_launch {
            validate_active_launch(active)?;
        }
        Ok(())
    }
}

fn validate_active_launch(active: &ActiveLaunchRecord) -> Result<()> {
    if active.launch_instance_id.trim().is_empty() || active.spawn_attempt_id.trim().is_empty() {
        return Err(LauncherError::Conflict(
            "active launch requires launch and spawn identities".to_string(),
        ));
    }
    let has_payload = active.payload_registration.is_some();
    if (active.phase == ActiveLaunchPhase::Running) != has_payload {
        return Err(LauncherError::Conflict(
            "active launch facts do not match its phase".to_string(),
        ));
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
    Ok(())
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
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } != 0 {
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
    state.revision = state
        .revision
        .checked_add(1)
        .ok_or_else(|| LauncherError::Conflict("control revision exhausted".to_string()))?;
    state.validate()?;
    write_json_atomic(&paths.control, &state)?;
    Ok(state)
}

pub(crate) fn update_locked<F>(paths: &ControlPaths, update: F) -> Result<ControlState>
where
    F: FnOnce(&mut ControlState) -> Result<()>,
{
    let mut state = load_unlocked(paths)?
        .ok_or_else(|| LauncherError::Conflict("control state is not initialized".to_string()))?;
    update(&mut state)?;
    state.revision = state
        .revision
        .checked_add(1)
        .ok_or_else(|| LauncherError::Conflict("control revision exhausted".to_string()))?;
    state.validate()?;
    write_json_atomic(&paths.control, &state)?;
    Ok(state)
}

pub(crate) fn load_unlocked(paths: &ControlPaths) -> Result<Option<ControlState>> {
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
    if schema_version < CONTROL_SCHEMA_VERSION {
        migrate_control_shape(&mut value);
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

fn migrate_control_shape(value: &mut serde_json::Value) {
    let Some(root) = value.as_object_mut() else {
        return;
    };
    root.insert(
        "schemaVersion".to_string(),
        serde_json::Value::from(CONTROL_SCHEMA_VERSION),
    );
    root.remove("activation");
    if let Some(active) = root
        .get_mut("activeLaunch")
        .and_then(serde_json::Value::as_object_mut)
    {
        active.remove("guardRegistration");
        active.remove("startAuthorization");
        active.remove("readyExpectation");
        let has_payload = active.get("payloadRegistration").is_some();
        active.insert(
            "phase".to_string(),
            serde_json::Value::from(if has_payload { "running" } else { "spawn_planned" }),
        );
        if let Some(payload) = active
            .get_mut("payloadRegistration")
            .and_then(serde_json::Value::as_object_mut)
        {
            payload.remove("protocolVersion");
            payload.remove("guardRegistrationDigest");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_activation_state_migrates_to_a_running_direct_payload_record() {
        let mut value = serde_json::json!({
            "schemaVersion": 2,
            "activation": { "phase": "committed" },
            "activeLaunch": {
                "phase": "awaiting_readiness",
                "guardRegistration": { "pid": 1 },
                "startAuthorization": { "token": "old" },
                "readyExpectation": { "token": "old" },
                "payloadRegistration": {
                    "protocolVersion": 1,
                    "guardRegistrationDigest": "old"
                }
            }
        });

        migrate_control_shape(&mut value);

        assert_eq!(
            value["schemaVersion"].as_u64(),
            Some(u64::from(CONTROL_SCHEMA_VERSION))
        );
        assert!(value.get("activation").is_none());
        assert_eq!(value["activeLaunch"]["phase"], "running");
        assert!(value["activeLaunch"].get("guardRegistration").is_none());
        assert!(value["activeLaunch"].get("startAuthorization").is_none());
        assert!(value["activeLaunch"].get("readyExpectation").is_none());
        assert!(
            value["activeLaunch"]["payloadRegistration"]
                .get("protocolVersion")
                .is_none()
        );
        assert!(
            value["activeLaunch"]["payloadRegistration"]
                .get("guardRegistrationDigest")
                .is_none()
        );
    }
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
