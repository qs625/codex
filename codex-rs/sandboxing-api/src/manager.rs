use codex_network_proxy_api::NetworkProxyRuntimeSnapshot;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::SandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;

use crate::policy_transforms::should_require_platform_sandbox;

#[cfg(target_os = "linux")]
pub(crate) const WSL1_BWRAP_WARNING: &str = concat!(
    "Codex's Linux sandbox uses bubblewrap, which is not supported on WSL1 ",
    "because WSL1 cannot create the required user namespaces. ",
    "Use WSL2 for sandboxed shell commands."
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxType {
    None,
    MacosSeatbelt,
    LinuxSeccomp,
    WindowsRestrictedToken,
}

impl SandboxType {
    pub fn as_metric_tag(self) -> &'static str {
        match self {
            SandboxType::None => "none",
            SandboxType::MacosSeatbelt => "seatbelt",
            SandboxType::LinuxSeccomp => "seccomp",
            SandboxType::WindowsRestrictedToken => "windows_sandbox",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxablePreference {
    Auto,
    Require,
    Forbid,
}

pub fn get_platform_sandbox(windows_sandbox_enabled: bool) -> Option<SandboxType> {
    if cfg!(target_os = "macos") {
        Some(SandboxType::MacosSeatbelt)
    } else if cfg!(target_os = "linux") {
        Some(SandboxType::LinuxSeccomp)
    } else if cfg!(target_os = "windows") {
        if windows_sandbox_enabled {
            Some(SandboxType::WindowsRestrictedToken)
        } else {
            None
        }
    } else {
        None
    }
}

pub fn select_initial_sandbox(
    file_system_policy: &FileSystemSandboxPolicy,
    network_policy: NetworkSandboxPolicy,
    pref: SandboxablePreference,
    windows_sandbox_level: WindowsSandboxLevel,
    has_managed_network_requirements: bool,
) -> SandboxType {
    match pref {
        SandboxablePreference::Forbid => SandboxType::None,
        SandboxablePreference::Require => {
            get_platform_sandbox(windows_sandbox_level != WindowsSandboxLevel::Disabled)
                .unwrap_or(SandboxType::None)
        }
        SandboxablePreference::Auto => {
            if should_require_platform_sandbox(
                file_system_policy,
                network_policy,
                has_managed_network_requirements,
            ) {
                get_platform_sandbox(windows_sandbox_level != WindowsSandboxLevel::Disabled)
                    .unwrap_or(SandboxType::None)
            } else {
                SandboxType::None
            }
        }
    }
}

#[derive(Debug)]
pub struct SandboxCommand {
    pub program: OsString,
    pub args: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub env: HashMap<String, String>,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
}

#[derive(Debug)]
pub struct SandboxExecRequest {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub env: HashMap<String, String>,
    pub network: Option<NetworkProxyRuntimeSnapshot>,
    pub sandbox: SandboxType,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
    pub permission_profile: PermissionProfile,
    pub file_system_sandbox_policy: FileSystemSandboxPolicy,
    pub network_sandbox_policy: NetworkSandboxPolicy,
    pub arg0: Option<String>,
}

/// Bundled arguments for sandbox transformation.
///
/// This keeps call sites self-documenting when several fields are optional.
pub struct SandboxTransformRequest<'a> {
    pub command: SandboxCommand,
    pub permissions: &'a PermissionProfile,
    pub sandbox: SandboxType,
    pub enforce_managed_network: bool,
    pub network: Option<&'a NetworkProxyRuntimeSnapshot>,
    pub sandbox_policy_cwd: &'a Path,
    pub codex_linux_sandbox_exe: Option<&'a Path>,
    pub use_legacy_landlock: bool,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
}

#[derive(Debug)]
pub enum SandboxTransformError {
    SandboxRuntimeUnavailable,
    MissingLinuxSandboxExecutable,
    #[cfg(target_os = "linux")]
    Wsl1UnsupportedForBubblewrap,
    #[cfg(not(target_os = "macos"))]
    SeatbeltUnavailable,
}

impl std::fmt::Display for SandboxTransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SandboxRuntimeUnavailable => write!(f, "sandbox runtime is unavailable"),
            Self::MissingLinuxSandboxExecutable => {
                write!(f, "missing codex-linux-sandbox executable path")
            }
            #[cfg(target_os = "linux")]
            Self::Wsl1UnsupportedForBubblewrap => write!(f, "{WSL1_BWRAP_WARNING}"),
            #[cfg(not(target_os = "macos"))]
            Self::SeatbeltUnavailable => write!(f, "seatbelt sandbox is only available on macOS"),
        }
    }
}

impl std::error::Error for SandboxTransformError {}

pub fn compatibility_sandbox_policy_for_permission_profile(
    permissions: &PermissionProfile,
    file_system_policy: &FileSystemSandboxPolicy,
    network_policy: NetworkSandboxPolicy,
    cwd: &Path,
) -> SandboxPolicy {
    permissions
        .to_legacy_sandbox_policy(cwd)
        .unwrap_or_else(|_| {
            compatibility_workspace_write_policy(file_system_policy, network_policy, cwd)
        })
}

fn compatibility_workspace_write_policy(
    file_system_policy: &FileSystemSandboxPolicy,
    network_policy: NetworkSandboxPolicy,
    cwd: &Path,
) -> SandboxPolicy {
    let cwd_abs = AbsolutePathBuf::from_absolute_path(cwd).ok();
    let writable_roots = file_system_policy
        .get_writable_roots_with_cwd(cwd)
        .into_iter()
        .map(|root| root.root)
        .filter(|root| cwd_abs.as_ref() != Some(root))
        .collect();
    let tmpdir_writable = std::env::var_os("TMPDIR")
        .filter(|tmpdir| !tmpdir.is_empty())
        .and_then(|tmpdir| {
            AbsolutePathBuf::from_absolute_path(std::path::PathBuf::from(tmpdir)).ok()
        })
        .is_some_and(|tmpdir| file_system_policy.can_write_path_with_cwd(tmpdir.as_path(), cwd));
    let slash_tmp = Path::new("/tmp");
    let slash_tmp_writable = slash_tmp.is_absolute()
        && slash_tmp.is_dir()
        && file_system_policy.can_write_path_with_cwd(slash_tmp, cwd);

    SandboxPolicy::WorkspaceWrite {
        writable_roots,
        network_access: network_policy.is_enabled(),
        exclude_tmpdir_env_var: !tmpdir_writable,
        exclude_slash_tmp: !slash_tmp_writable,
    }
}
