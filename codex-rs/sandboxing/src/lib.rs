#[cfg(target_os = "linux")]
mod bwrap;
pub mod landlock;
mod manager;
pub mod policy_transforms;
#[cfg(target_os = "macos")]
pub mod seatbelt;
mod windows_sandbox;
mod windows_sandbox_read_grants;

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
pub use windows_sandbox::ELEVATED_SANDBOX_NUX_ENABLED;
pub use windows_sandbox::WindowsSandboxLevelExt;
pub use windows_sandbox::WindowsSandboxSetupMode;
pub use windows_sandbox::WindowsSandboxSetupRequest;
pub use windows_sandbox::elevated_setup_failure_details;
pub use windows_sandbox::elevated_setup_failure_metric_name;
pub use windows_sandbox::legacy_windows_sandbox_mode;
pub use windows_sandbox::legacy_windows_sandbox_mode_from_entries;
pub use windows_sandbox::resolve_windows_sandbox_mode;
pub use windows_sandbox::resolve_windows_sandbox_private_desktop;
pub use windows_sandbox::run_elevated_setup;
pub use windows_sandbox::run_legacy_setup_preflight;
pub use windows_sandbox::run_setup_refresh_with_extra_read_roots;
pub use windows_sandbox::run_windows_sandbox_setup;
pub use windows_sandbox::sandbox_setup_is_complete;
pub use windows_sandbox::windows_sandbox_level_from_config;
pub use windows_sandbox::windows_sandbox_level_from_features;
pub use windows_sandbox_read_grants::grant_read_root_non_elevated;

#[cfg(not(target_os = "linux"))]
pub fn system_bwrap_warning(
    _permission_profile: &codex_protocol::models::PermissionProfile,
) -> Option<String> {
    None
}
