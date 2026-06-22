#[cfg(target_os = "linux")]
mod bwrap;
pub mod landlock;
mod manager;
pub mod policy_transforms;
#[cfg(target_os = "macos")]
pub mod seatbelt;

#[cfg(target_os = "linux")]
pub use bwrap::find_system_bwrap_in_path;
#[cfg(target_os = "linux")]
pub use bwrap::system_bwrap_warning;
pub use codex_sandboxing_api::SandboxCommand;
pub use codex_sandboxing_api::SandboxExecRequest;
pub use codex_sandboxing_api::SandboxTransformError;
pub use codex_sandboxing_api::SandboxTransformRequest;
pub use codex_sandboxing_api::SandboxType;
pub use codex_sandboxing_api::SandboxablePreference;
pub use codex_sandboxing_api::compatibility_sandbox_policy_for_permission_profile;
pub use codex_sandboxing_api::get_platform_sandbox;
pub use manager::SandboxManager;

#[cfg(not(target_os = "linux"))]
pub fn system_bwrap_warning(
    _permission_profile: &codex_protocol::models::PermissionProfile,
) -> Option<String> {
    None
}
