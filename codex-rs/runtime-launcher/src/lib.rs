#![cfg(unix)]

mod capsule;
mod control;
mod migration;
mod process;
mod seed;
mod supervisor;

pub use capsule::CapsuleEntry;
pub use capsule::CapsuleLaunch;
pub use capsule::CapsuleManifest;
pub use capsule::CapsuleRecord;
pub use capsule::CapsuleTarget;
pub use capsule::ImportRequest;
pub use capsule::compute_release_id;
pub use capsule::compute_release_preimage;
pub use control::CapsuleRef;
pub use control::ControlState;
pub use control::FailureProjection;
pub use control::SelectedRuntime;
pub use control::TrustedSeed;
pub use supervisor::LauncherPaths;
pub use supervisor::RunOutcome;
pub use supervisor::SelectCandidateResult;
pub use supervisor::Status;

use serde::Deserialize;
use serde::Serialize;
use std::path::Path;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, LauncherError>;

#[derive(Debug, thiserror::Error)]
pub enum LauncherError {
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{context}: {source}")]
    Json {
        context: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("invalid capsule: {0}")]
    InvalidArtifact(String),
    #[error("launcher state conflict: {0}")]
    Conflict(String),
    #[error("runtime lifecycle blocked: {0}")]
    Blocked(String),
    #[error("runtime launch failed: {0}")]
    Launch(String),
}

pub(crate) fn io_error(context: impl Into<String>, source: std::io::Error) -> LauncherError {
    LauncherError::Io {
        context: context.into(),
        source,
    }
}

pub(crate) fn json_error(context: impl Into<String>, source: serde_json::Error) -> LauncherError {
    LauncherError::Json {
        context: context.into(),
        source,
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectCandidateRequest {
    pub schema_version: u32,
    pub activation_id: String,
    pub target: CapsuleTarget,
    #[serde(default)]
    pub reason: String,
}

pub fn read_request<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes =
        std::fs::read(path).map_err(|error| io_error(format!("read {}", path.display()), error))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| json_error(format!("parse {}", path.display()), error))
}

pub fn select_candidate(
    paths: &LauncherPaths,
    request_path: &Path,
) -> Result<SelectCandidateResult> {
    supervisor::select_candidate(paths, read_request(request_path)?)
}

pub fn status(paths: &LauncherPaths) -> Result<Status> {
    supervisor::status(paths)
}

pub fn run(
    paths: &LauncherPaths,
    outer_bundle: &Path,
    target: CapsuleTarget,
    launcher_path: &Path,
) -> Result<RunOutcome> {
    supervisor::run(paths, outer_bundle, target, launcher_path)
}

pub fn state_root(explicit: Option<PathBuf>) -> Result<PathBuf> {
    resolve_state_root(
        explicit,
        std::env::var_os("RUNTIME_CAPSULE_LAUNCHER_HOME").map(PathBuf::from),
        std::env::var_os("MORPHEUS_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

fn resolve_state_root(
    explicit: Option<PathBuf>,
    launcher_home: Option<PathBuf>,
    morpheus_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Result<PathBuf> {
    explicit
        .or(launcher_home)
        .or_else(|| morpheus_home.map(|home| home.join("runtime-launcher")))
        .or_else(|| home.map(|home| home.join(".morpheus/runtime-launcher")))
        .ok_or_else(|| LauncherError::InvalidRequest("launcher home is unavailable".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_home_resolution_has_installed_defaults() {
        let explicit = PathBuf::from("/explicit");
        assert_eq!(
            resolve_state_root(
                Some(explicit.clone()),
                Some(PathBuf::from("/launcher")),
                Some(PathBuf::from("/morpheus")),
                Some(PathBuf::from("/home")),
            )
            .expect("explicit"),
            explicit
        );
        assert_eq!(
            resolve_state_root(
                None,
                Some(PathBuf::from("/launcher")),
                Some(PathBuf::from("/morpheus")),
                Some(PathBuf::from("/home")),
            )
            .expect("launcher"),
            PathBuf::from("/launcher")
        );
        assert_eq!(
            resolve_state_root(
                None,
                None,
                Some(PathBuf::from("/morpheus")),
                Some(PathBuf::from("/home")),
            )
            .expect("configured home"),
            PathBuf::from("/morpheus/runtime-launcher")
        );
        assert_eq!(
            resolve_state_root(None, None, None, Some(PathBuf::from("/home"))).expect("user home"),
            PathBuf::from("/home/.morpheus/runtime-launcher")
        );
    }
}
