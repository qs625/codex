//! Lightweight sandboxing API shared by core runtime and platform implementations.
//!
//! This crate owns sandbox DTOs, sandbox selection helpers, legacy compatibility projection and
//! permission-profile transforms. Platform-specific command rewriting stays in `codex-sandboxing`.

use std::path::PathBuf;
use std::sync::Arc;

mod manager;
pub mod policy_transforms;
mod request_permissions;
mod sandbox_tags;
pub mod shell_escalation;
mod shell_escalation_policy;
mod shell_escalation_stopwatch;
mod windows_deny_read;
mod windows_filesystem_overrides;

pub use manager::SandboxCommand;
pub use manager::SandboxExecRequest;
pub use manager::SandboxTransformError;
pub use manager::SandboxTransformRequest;
pub use manager::SandboxType;
pub use manager::SandboxablePreference;
pub use manager::compatibility_sandbox_policy_for_permission_profile;
pub use manager::get_platform_sandbox;
pub use manager::select_initial_sandbox;
pub use request_permissions::normalize_request_permissions_response;
pub use sandbox_tags::permission_profile_policy_tag;
pub use sandbox_tags::permission_profile_sandbox_tag;
pub use sandbox_tags::sandbox_tag;
pub use shell_escalation::EscalationDecision;
pub use shell_escalation::EscalationExecution;
pub use codex_protocol::approvals::EscalationPermissions;
pub use shell_escalation::EscalationPolicyDecisionParams;
pub use shell_escalation::EscalationPromptDecision;
pub use shell_escalation::EscalationPromptFuture;
pub use shell_escalation::EscalationPromptHandler;
pub use shell_escalation::EscalationPromptRequest;
pub use shell_escalation::ExecResult;
pub use shell_escalation::ParsedShellCommand;
pub use codex_protocol::approvals::ResolvedPermissionProfile;
pub use shell_escalation_stopwatch::Stopwatch;
pub use shell_escalation::approval_sandbox_permissions;
pub use shell_escalation::determine_escalation_action;
pub use shell_escalation::extract_shell_script;
pub use shell_escalation::map_exec_result;
pub use shell_escalation_policy::EscalationPolicy;
pub use shell_escalation_policy::EscalationPolicyFuture;
pub use windows_deny_read::resolve_windows_deny_read_paths;
pub use windows_filesystem_overrides::WindowsSandboxFilesystemOverrides;
pub use windows_filesystem_overrides::resolve_windows_elevated_filesystem_overrides;
pub use windows_filesystem_overrides::resolve_windows_restricted_token_filesystem_overrides;
pub use windows_filesystem_overrides::should_use_windows_restricted_token_sandbox;
pub use windows_filesystem_overrides::unsupported_windows_restricted_token_sandbox_reason;
pub use windows_filesystem_overrides::windows_sandbox_uses_elevated_backend;

use codex_exec_server_api::ExecEnvironment;
use codex_file_system::ExecutorFileSystem;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_utils_absolute_path::AbsolutePathBuf;

/// Filesystem/environment boundary needed by apply-patch and exec-style tools.
pub trait ApplyPatchEnvironment: Send + Sync {
    fn environment_id(&self) -> &str;

    fn filesystem(&self) -> Arc<dyn ExecutorFileSystem>;
}

/// Runtime sandbox inputs derived from the active turn.
pub struct ToolSandboxContext {
    pub turn_id: String,
    pub telemetry: SharedSessionTelemetry,
    pub file_system_sandbox_policy: FileSystemSandboxPolicy,
    pub network_sandbox_policy: NetworkSandboxPolicy,
    pub permission_profile: PermissionProfile,
    pub managed_network_active: bool,
    pub cwd: AbsolutePathBuf,
    pub codex_linux_sandbox_exe: Option<PathBuf>,
    pub use_legacy_landlock: bool,
    pub windows_sandbox_level: WindowsSandboxLevel,
    pub windows_sandbox_private_desktop: bool,
}

/// Apply-patch environment resolution result for one selected runtime environment.
pub struct ResolvedApplyPatchEnvironment {
    pub cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ApplyPatchEnvironment>,
}

/// Exec-command environment resolution result for one selected runtime environment.
pub struct ResolvedExecCommandEnvironment {
    pub cwd: AbsolutePathBuf,
    pub sandbox_cwd: AbsolutePathBuf,
    pub environment: Arc<dyn ExecEnvironment>,
    pub apply_patch_environment: Arc<dyn ApplyPatchEnvironment>,
}

/// Runtime capability for selecting and transforming sandboxed process commands.
///
/// Consumers that only need sandbox DTOs or permission transforms should use the free functions in
/// this crate. Consumers that actually need platform command rewriting should receive this trait
/// from a composition root so they do not depend on the concrete sandbox runtime crate.
pub trait SandboxRuntime: Send + Sync {
    fn select_initial(
        &self,
        file_system_policy: &codex_protocol::permissions::FileSystemSandboxPolicy,
        network_policy: codex_protocol::permissions::NetworkSandboxPolicy,
        pref: SandboxablePreference,
        windows_sandbox_level: codex_protocol::config_types::WindowsSandboxLevel,
        has_managed_network_requirements: bool,
    ) -> SandboxType {
        select_initial_sandbox(
            file_system_policy,
            network_policy,
            pref,
            windows_sandbox_level,
            has_managed_network_requirements,
        )
    }

    fn transform(
        &self,
        request: SandboxTransformRequest<'_>,
    ) -> Result<SandboxExecRequest, SandboxTransformError>;
}

pub type SharedSandboxRuntime = std::sync::Arc<dyn SandboxRuntime>;

#[derive(Debug, Default)]
pub struct DisabledSandboxRuntime;

impl SandboxRuntime for DisabledSandboxRuntime {
    fn transform(
        &self,
        _request: SandboxTransformRequest<'_>,
    ) -> Result<SandboxExecRequest, SandboxTransformError> {
        Err(SandboxTransformError::SandboxRuntimeUnavailable)
    }
}

impl From<SandboxTransformError> for codex_protocol::error::CodexErr {
    fn from(err: SandboxTransformError) -> Self {
        match err {
            SandboxTransformError::MissingLinuxSandboxExecutable => {
                codex_protocol::error::CodexErr::LandlockSandboxExecutableNotProvided
            }
            SandboxTransformError::SandboxRuntimeUnavailable => {
                codex_protocol::error::CodexErr::UnsupportedOperation(
                    "sandbox runtime is unavailable".to_string(),
                )
            }
            #[cfg(target_os = "linux")]
            SandboxTransformError::Wsl1UnsupportedForBubblewrap => {
                codex_protocol::error::CodexErr::UnsupportedOperation(
                    manager::WSL1_BWRAP_WARNING.to_string(),
                )
            }
            #[cfg(not(target_os = "macos"))]
            SandboxTransformError::SeatbeltUnavailable => {
                codex_protocol::error::CodexErr::UnsupportedOperation(
                    "seatbelt sandbox is only available on macOS".to_string(),
                )
            }
        }
    }
}
