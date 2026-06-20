//! Helpers for mapping config layer failures to file locations.

pub use codex_config_diagnostics::ConfigError;
pub use codex_config_diagnostics::ConfigLoadError;
pub use codex_config_diagnostics::TextPosition;
pub use codex_config_diagnostics::TextRange;
pub use codex_config_diagnostics::config_error_from_toml;
pub use codex_config_diagnostics::config_error_from_typed_toml;
pub use codex_config_diagnostics::format_config_error;
pub use codex_config_diagnostics::format_config_error_with_source;
pub use codex_config_diagnostics::io_error_from_config_error;
pub use codex_config_state::first_layer_config_error;
pub use codex_config_state::first_layer_config_error_from_entries;
