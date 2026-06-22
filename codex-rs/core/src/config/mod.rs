pub use codex_config::AgentCapabilityAllowlist;
pub use codex_config::AgentRoleConfig;
pub use codex_config::AgentRoleSource;
pub use codex_config::CONFIG_TOML_FILE;
pub use codex_config::Config;
pub use codex_config::ConfigBuilder;
pub use codex_config::ConfigLayerStack;
pub use codex_config::ConfigLoadOptions;
pub use codex_config::ConfigOverrides;
pub use codex_config::Constrained;
pub use codex_config::ConstraintError;
pub use codex_config::ConstraintResult;
pub use codex_config::GhostSnapshotConfig;
pub use codex_config::LoaderOverrides;
pub use codex_config::ManagedFeatures;
pub use codex_config::MultiAgentV2Config;
pub use codex_config::NetworkProxyAuditMetadata;
pub use codex_config::NetworkProxySpec;
pub use codex_config::PermissionProfileState;
pub use codex_config::Permissions;
pub use codex_config::ProjectConfig;
pub use codex_config::RealtimeAudioConfig;
pub use codex_config::RealtimeConfig;
pub use codex_config::StartedNetworkProxy;
pub use codex_config::TerminalResizeReflowConfig;
pub use codex_config::TerminalResizeReflowMaxRows;
pub use codex_config::ThreadStoreConfig;
pub use codex_config::deserialize_config_toml_with_base;
pub use codex_config::find_codex_home;
pub use codex_config::hook_config_layer_stack_from_config_layer_stack;
pub use codex_config::load_config_as_toml_with_cli_and_load_options;
pub use codex_config::load_config_as_toml_with_cli_and_load_options_and_layer_loader;
pub use codex_config::load_config_as_toml_with_cli_and_loader_overrides;
pub use codex_config::load_config_as_toml_with_cli_overrides;
pub use codex_config::log_dir;
pub use codex_config::plugin_config_layer_stack_from_config_layer_stack;
pub use codex_config::resolve_oss_provider;
pub use codex_config::resolve_profile_v2_config_path;
pub(crate) use codex_config::resolve_tool_suggest_config_from_layer_stack;
pub use codex_config::resolve_web_search_mode_for_turn;
pub use codex_config::set_default_oss_provider;
pub use codex_config::set_project_trust_level;
pub use codex_config::skill_config_layer_stack_from_config_layer_stack;
pub use codex_config::validate_feature_requirements_for_config_toml;

pub mod edit {
    pub use codex_config::edit::*;
}

pub mod schema {
    pub use codex_config::schema::*;
}

pub(crate) mod agent_roles {
    pub(crate) use codex_config::agent_roles::*;
}

#[cfg(test)]
pub(crate) async fn test_config() -> Config {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    Config::load_from_base_config_with_overrides(
        codex_config_toml::config_toml::ConfigToml::default(),
        ConfigOverrides::default(),
        codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(codex_home.path())
            .expect("temp dir should resolve"),
    )
    .await
    .expect("load default test config")
}
