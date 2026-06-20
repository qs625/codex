//! Canonical `config.toml` shape and schema types.

pub mod config_toml;
pub mod profile_toml;
pub mod schema;
pub mod types;

pub mod permissions_toml {
    pub use codex_config_permissions::PermissionsToml;
}

pub use codex_config_types::Constrained;
pub use codex_config_types::HooksToml;
