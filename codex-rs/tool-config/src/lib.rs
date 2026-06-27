//! Tool configuration and lightweight model capability helpers.
//!
//! This crate sits below the tool planning/service layers: it owns the data
//! needed to decide which tool surfaces should be exposed, while concrete tool
//! factories and planning remain in owner service crates such as `codex-tool-service`.

mod image_detail;
mod tool_config;

pub use image_detail::can_request_original_image_detail;
pub use image_detail::normalize_output_image_detail;
pub use image_detail::sanitize_original_image_detail;
pub use tool_config::ShellCommandBackendConfig;
pub use tool_config::ToolEnvironmentMode;
pub use tool_config::ToolUserShellType;
pub use tool_config::ToolsConfig;
pub use tool_config::ToolsConfigParams;
pub use tool_config::UnifiedExecShellMode;
pub use tool_config::ZshForkConfig;
pub use tool_config::request_user_input_available_modes;
