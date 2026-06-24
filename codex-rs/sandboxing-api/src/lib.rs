//! Lightweight sandboxing API shared by core runtime and platform implementations.
//!
//! This crate owns sandbox DTOs, sandbox selection helpers, legacy compatibility projection and
//! permission-profile transforms. Platform-specific command rewriting stays in `codex-sandboxing`.

mod manager;
pub mod policy_transforms;
mod request_permissions;
mod sandbox_tags;
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
pub use windows_deny_read::resolve_windows_deny_read_paths;
pub use windows_filesystem_overrides::WindowsSandboxFilesystemOverrides;
pub use windows_filesystem_overrides::resolve_windows_elevated_filesystem_overrides;
pub use windows_filesystem_overrides::resolve_windows_restricted_token_filesystem_overrides;
pub use windows_filesystem_overrides::should_use_windows_restricted_token_sandbox;
pub use windows_filesystem_overrides::unsupported_windows_restricted_token_sandbox_reason;
pub use windows_filesystem_overrides::windows_sandbox_uses_elevated_backend;

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
