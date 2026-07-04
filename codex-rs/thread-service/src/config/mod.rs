pub use config_service::AgentCapabilityAllowlist;
pub use config_service::AgentRoleConfig;
pub use config_service::AgentRoleSource;
pub use config_service::CONFIG_TOML_FILE;
pub use config_service::Config;
pub use config_service::ConfigBuilder;
pub use config_service::ConfigLayerStack;
pub use config_service::ConfigLoadOptions;
pub use config_service::ConfigOverrides;
pub use config_service::Constrained;
pub use config_service::ConstraintError;
pub use config_service::ConstraintResult;
pub use config_service::EffectiveSessionConfigOverlay;
pub use config_service::GhostSnapshotConfig;
pub use config_service::LoaderOverrides;
pub use config_service::ManagedFeatures;
pub use config_service::MultiAgentV2Config;
pub use config_service::NetworkProxyAuditMetadata;
pub use config_service::NetworkProxySpec;
pub use config_service::PermissionProfileState;
pub use config_service::Permissions;
pub use config_service::ProjectConfig;
pub use config_service::RealtimeAudioConfig;
pub use config_service::RealtimeConfig;
pub use config_service::SessionConfigOverlay;
pub use config_service::StartedNetworkProxy;
pub use config_service::TerminalResizeReflowConfig;
pub use config_service::TerminalResizeReflowMaxRows;
pub use config_service::ThreadStoreConfig;
pub use config_service::build_effective_session_config_from_session_overlay;
pub use config_service::build_per_turn_config_from_session_overlay;
pub use config_service::deserialize_config_toml_with_base;
pub use config_service::find_codex_home;
pub use config_service::hook_config_layer_stack_from_config_layer_stack;
pub use config_service::load_config_as_toml_with_cli_and_load_options;
pub use config_service::load_config_as_toml_with_cli_and_load_options_and_layer_loader;
pub use config_service::load_config_as_toml_with_cli_and_loader_overrides;
pub use config_service::load_config_as_toml_with_cli_overrides;
pub use config_service::log_dir;
pub use config_service::plugin_config_layer_stack_from_config_layer_stack;
pub use config_service::resolve_oss_provider;
pub use config_service::resolve_profile_v2_config_path;
pub use config_service::resolve_web_search_mode_for_turn;
pub use config_service::set_default_oss_provider;
pub use config_service::set_project_trust_level;
pub use config_service::skill_config_layer_stack_from_config_layer_stack;
pub use config_service::validate_feature_requirements_for_config_toml;

pub mod edit {
    pub use config_service::edit::*;
}

pub mod schema {
    pub use config_service::schema::*;
}

pub(crate) mod agent_roles {
    pub(crate) use config_service::agent_roles::*;
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
