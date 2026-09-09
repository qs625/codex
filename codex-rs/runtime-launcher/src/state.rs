use crate::Result;
use crate::io_error;
use crate::json_error;
use serde::Deserialize;
use serde::Serialize;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

// Schema 1 is the first shipped launcher layout. Reverted prototypes are not migration inputs.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactIdentity {
    pub transaction_id: String,
    pub build_id: String,
    pub source_commit: String,
}

impl ArtifactIdentity {
    pub fn recovery_identity(&self) -> String {
        format!("{}:{}", self.transaction_id, self.build_id)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub identity: ArtifactIdentity,
    pub artifact_root: PathBuf,
    pub app_bundle_path: PathBuf,
    pub entrypoint: PathBuf,
    pub installed_at_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherState {
    pub schema_version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<ArtifactRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous: Option<ArtifactRecord>,
}

impl Default for LauncherState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            current: None,
            previous: None,
        }
    }
}

impl LauncherState {
    pub fn load(paths: &LauncherPaths) -> Result<Self> {
        let state: Self = read_json_if_exists(&paths.state)?.unwrap_or_default();
        if state.schema_version != SCHEMA_VERSION {
            return Err(crate::LauncherError::Conflict(format!(
                "unsupported launcher state schema {}",
                state.schema_version
            )));
        }
        Ok(state)
    }

    pub fn save(&self, paths: &LauncherPaths) -> Result<()> {
        write_json_atomic(&paths.state, self)
    }
}

#[derive(Clone, Debug)]
pub struct LauncherPaths {
    pub root: PathBuf,
    pub state: PathBuf,
    pub transaction: PathBuf,
    pub failure_evidence: PathBuf,
    pub ready: PathBuf,
    pub operation_lock: PathBuf,
}

impl LauncherPaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            state: root.join("state.json"),
            transaction: root.join("transaction.json"),
            failure_evidence: root.join("failure-evidence.json"),
            ready: root.join("ready.json"),
            operation_lock: root.join(".operation-lock"),
            root,
        }
    }

    pub fn ensure(&self) -> Result<()> {
        std::fs::create_dir_all(&self.root)
            .map_err(|err| io_error(format!("create {}", self.root.display()), err))
    }
}

pub(crate) struct OperationLock {
    #[cfg(unix)]
    _file: File,
    #[cfg(not(unix))]
    path: PathBuf,
}

impl OperationLock {
    pub(crate) fn acquire(paths: &LauncherPaths) -> Result<Self> {
        paths.ensure()?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .open(&paths.operation_lock)
                .map_err(|err| {
                    io_error(format!("open {}", paths.operation_lock.display()), err)
                })?;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                return Err(crate::LauncherError::Conflict(
                    "another launcher operation is active".to_string(),
                ));
            }
            Ok(Self { _file: file })
        }
        #[cfg(not(unix))]
        {
            match std::fs::create_dir(&paths.operation_lock) {
                Ok(()) => Ok(Self {
                    path: paths.operation_lock.clone(),
                }),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(crate::LauncherError::Conflict(
                        "another launcher operation is active".to_string(),
                    ))
                }
                Err(err) => Err(io_error(
                    format!("create {}", paths.operation_lock.display()),
                    err,
                )),
            }
        }
    }
}

#[cfg(not(unix))]
impl Drop for OperationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.path);
    }
}

pub(crate) fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(crate) fn read_json_if_exists<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(crate::LauncherError::Conflict(format!(
                "state file must not be a symlink: {}",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io_error(format!("inspect {}", path.display()), err)),
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => return Err(io_error(format!("read {}", path.display()), err)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|err| json_error(format!("parse {}", path.display()), err))
}

pub(crate) fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        crate::LauncherError::InvalidRequest(format!("{} has no parent directory", path.display()))
    })?;
    std::fs::create_dir_all(parent)
        .map_err(|err| io_error(format!("create {}", parent.display()), err))?;
    let file_name = path.file_name().and_then(|value| value.to_str()).ok_or_else(|| {
        crate::LauncherError::InvalidRequest(format!("invalid state path {}", path.display()))
    })?;
    let temp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        unix_time_ms()
    ));
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|err| json_error("serialize state", err))?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temp)
        .map_err(|err| io_error(format!("create {}", temp.display()), err))?;
    file.write_all(&bytes)
        .map_err(|err| io_error(format!("write {}", temp.display()), err))?;
    file.write_all(b"\n")
        .map_err(|err| io_error(format!("write {}", temp.display()), err))?;
    file.sync_all()
        .map_err(|err| io_error(format!("sync {}", temp.display()), err))?;
    std::fs::rename(&temp, path)
        .map_err(|err| io_error(format!("replace {}", path.display()), err))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|err| io_error(format!("sync {}", parent.display()), err))
}

pub(crate) fn remove_file_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                File::open(parent)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|err| io_error(format!("sync {}", parent.display()), err))?;
            }
            Ok(())
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(io_error(format!("remove {}", path.display()), err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_json_round_trip() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = LauncherPaths::new(temp.path().to_path_buf());
        paths.ensure().expect("ensure paths");
        let state = LauncherState::default();
        state.save(&paths).expect("save");
        assert_eq!(LauncherState::load(&paths).expect("load"), state);
    }
}
