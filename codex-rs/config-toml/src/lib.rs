//! Canonical `config.toml` shape and schema types.

mod config_lock;
pub mod config_toml;
mod path_resolution;
pub mod profile_toml;
pub mod schema;
pub mod types;

pub mod permissions_toml {
    pub use codex_config_permissions::PermissionsToml;
}

pub use codex_config_types::Constrained;
pub use codex_config_types::HooksToml;
pub use config_lock::CONFIG_LOCK_VERSION;
pub use config_lock::ConfigLockReplayOptions;
pub use config_lock::clear_config_lock_debug_controls;
pub use config_lock::config_lockfile;
pub use config_lock::config_without_lock_controls;
pub use config_lock::read_config_lock_from_path;
pub use config_lock::toml_round_trip;
pub use config_lock::validate_config_lock_replay;
pub use path_resolution::deserialize_config_toml_with_base;
pub use path_resolution::resolve_relative_paths_in_config_toml;
