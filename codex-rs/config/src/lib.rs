mod cloud_requirements;
mod config_requirements;
pub mod config_toml;
mod constraint;
mod diagnostics;
mod hook_config;
pub mod loader;
mod mcp_edit;
mod mcp_types;
mod merge;
mod overrides;
pub mod permissions_toml;
pub mod profile_toml;
mod project_root_markers;
mod requirements_exec_policy;
mod runtime;
pub mod schema;
mod skills_config;
mod state;
pub mod types;

#[cfg(test)]
extern crate self as codex_config;

#[cfg(test)]
mod agents_md {
    pub use crate::runtime::DEFAULT_AGENTS_MD_FILENAME;
    pub use crate::runtime::LOCAL_AGENTS_MD_FILENAME;
}

#[cfg(test)]
mod config {
    pub use crate::*;
}

#[cfg(test)]
mod exec_policy {
    use crate::ConfigLayerStack;
    use crate::ConfigLayerStackOrdering;
    use codex_execpolicy_api::Policy;
    use std::path::Path;
    use std::path::PathBuf;

    const RULES_DIR_NAME: &str = "rules";

    pub(crate) async fn load_exec_policy(
        config_stack: &ConfigLayerStack,
    ) -> anyhow::Result<Policy> {
        let mut policy_paths = Vec::new();
        for layer in config_stack.get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ false,
        ) {
            if config_stack.ignore_user_and_project_exec_policy_rules()
                && matches!(
                    layer.name,
                    codex_config_types::ConfigLayerSource::User { .. }
                        | codex_config_types::ConfigLayerSource::Project { .. }
                )
            {
                continue;
            }
            if let Some(config_folder) = layer.config_folder() {
                policy_paths.extend(collect_policy_files(config_folder.join(RULES_DIR_NAME))?);
            }
        }

        let mut parser = codex_execpolicy::PolicyParser::new();
        for policy_path in &policy_paths {
            let contents = std::fs::read_to_string(policy_path)?;
            let identifier = policy_path.to_string_lossy().to_string();
            parser.parse(&identifier, &contents)?;
        }

        let policy = parser.build();
        let Some(requirements_policy) = config_stack.requirements().exec_policy.as_deref() else {
            return Ok(policy);
        };
        Ok(policy.merge_overlay(requirements_policy.as_ref()))
    }

    fn collect_policy_files(dir: impl AsRef<Path>) -> anyhow::Result<Vec<PathBuf>> {
        let dir = dir.as_ref();
        let read_dir = match std::fs::read_dir(dir) {
            Ok(read_dir) => read_dir,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };

        let mut policy_paths = Vec::new();
        for entry in read_dir {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "rules")
                && file_type.is_file()
            {
                policy_paths.push(path);
            }
        }

        policy_paths.sort();
        Ok(policy_paths)
    }
}

pub mod agent_roles {
    pub use crate::runtime::agent_roles::*;
}

pub mod edit {
    pub use crate::runtime::edit::*;
}

pub use cloud_requirements::CloudRequirementsLoadError;
pub use cloud_requirements::CloudRequirementsLoadErrorCode;
pub use cloud_requirements::CloudRequirementsLoader;
pub use codex_config_edit::MarketplaceConfigUpdate;
pub use codex_config_edit::PluginConfigEdit;
pub use codex_config_edit::RemoveMarketplaceConfigOutcome;
pub use codex_config_edit::apply_user_plugin_config_edits;
pub use codex_config_edit::clear_user_plugin;
pub use codex_config_edit::record_user_marketplace;
pub use codex_config_edit::remove_user_marketplace;
pub use codex_config_edit::remove_user_marketplace_config;
pub use codex_config_edit::set_user_plugin_enabled;
pub use codex_config_loader::ConfigLoadOptions;
pub use codex_config_loader::LoaderOverrides;
pub use codex_config_loader::NoopThreadConfigLoader;
pub use codex_config_loader::SessionThreadConfig;
pub use codex_config_loader::StaticThreadConfigLoader;
pub use codex_config_loader::ThreadConfigContext;
pub use codex_config_loader::ThreadConfigLoadError;
pub use codex_config_loader::ThreadConfigLoadErrorCode;
pub use codex_config_loader::ThreadConfigLoader;
pub use codex_config_loader::ThreadConfigSource;
pub use codex_config_loader::UserThreadConfig;
pub use codex_config_local_loader::config_error_from_ignored_toml_fields;
pub use codex_config_local_loader::host_name;
pub use codex_config_state::version_for_toml;
pub use codex_config_types::CONFIG_TOML_FILE;
pub use codex_config_types::ConfigLayerSource;
pub use codex_protocol::config_types::ProfileV2Name;
pub use codex_protocol::config_types::ProfileV2NameParseError;
pub use codex_utils_absolute_path::AbsolutePathBuf;
pub use config_requirements::AppRequirementToml;
pub use config_requirements::AppToolRequirementToml;
pub use config_requirements::AppToolsRequirementsToml;
pub use config_requirements::AppsRequirementsToml;
pub use config_requirements::ConfigRequirements;
pub use config_requirements::ConfigRequirementsToml;
pub use config_requirements::ConfigRequirementsWithSources;
pub use config_requirements::ConstrainedWithSource;
pub use config_requirements::FeatureRequirementsToml;
pub use config_requirements::FilesystemConstraints;
pub use config_requirements::FilesystemDenyReadPattern;
pub use config_requirements::McpServerIdentity;
pub use config_requirements::McpServerRequirement;
pub use config_requirements::NetworkConstraints;
pub use config_requirements::NetworkDomainPermissionToml;
pub use config_requirements::NetworkDomainPermissionsToml;
pub use config_requirements::NetworkRequirementsToml;
pub use config_requirements::NetworkUnixSocketPermissionToml;
pub use config_requirements::NetworkUnixSocketPermissionsToml;
pub use config_requirements::PluginRequirementsToml;
pub use config_requirements::RemoteSandboxConfigToml;
pub use config_requirements::RequirementSource;
pub use config_requirements::ResidencyRequirement;
pub use config_requirements::SandboxModeRequirement;
pub use config_requirements::Sourced;
pub use config_requirements::WebSearchModeRequirement;
pub use config_requirements::sandbox_mode_requirement_for_permission_profile;
pub use constraint::Constrained;
pub use constraint::ConstraintError;
pub use constraint::ConstraintResult;
pub use diagnostics::ConfigError;
pub use diagnostics::ConfigLoadError;
pub use diagnostics::TextPosition;
pub use diagnostics::TextRange;
pub use diagnostics::config_error_from_toml;
pub use diagnostics::config_error_from_typed_toml;
pub use diagnostics::first_layer_config_error;
pub use diagnostics::first_layer_config_error_from_entries;
pub use diagnostics::format_config_error;
pub use diagnostics::format_config_error_with_source;
pub use diagnostics::io_error_from_config_error;
pub use hook_config::HookEventsToml;
pub use hook_config::HookHandlerConfig;
pub use hook_config::HookStateToml;
pub use hook_config::HooksFile;
pub use hook_config::HooksToml;
pub use hook_config::ManagedHooksRequirementsToml;
pub use hook_config::MatcherGroup;
pub use hook_config::hook_events_into_matcher_groups;
pub use mcp_edit::ConfigEditsBuilder;
pub use mcp_edit::load_global_mcp_servers;
pub use mcp_types::AppToolApproval;
pub use mcp_types::McpServerConfig;
pub use mcp_types::McpServerDisabledReason;
pub use mcp_types::McpServerEnvVar;
pub use mcp_types::McpServerOAuthConfig;
pub use mcp_types::McpServerToolConfig;
pub use mcp_types::McpServerTransportConfig;
pub use mcp_types::RawMcpServerConfig;
pub use merge::merge_toml_values;
pub use overrides::build_cli_overrides_layer;
pub use project_root_markers::default_project_root_markers;
pub use project_root_markers::project_root_markers_from_config;
pub use requirements_exec_policy::RequirementsExecPolicy;
pub use requirements_exec_policy::RequirementsExecPolicyDecisionToml;
pub use requirements_exec_policy::RequirementsExecPolicyParseError;
pub use requirements_exec_policy::RequirementsExecPolicyPatternTokenToml;
pub use requirements_exec_policy::RequirementsExecPolicyPrefixRuleToml;
pub use requirements_exec_policy::RequirementsExecPolicyToml;
pub use runtime::AgentCapabilityAllowlist;
pub use runtime::AgentRoleConfig;
pub use runtime::AgentRoleSource;
pub use runtime::Config;
pub use runtime::ConfigBuilder;
pub use runtime::ConfigOverrides;
pub use runtime::EffectiveSessionConfigOverlay;
pub use runtime::GhostSnapshotConfig;
pub use runtime::ManagedFeatures;
pub use runtime::MultiAgentV2Config;
pub use runtime::NetworkProxyAuditMetadata;
pub use runtime::NetworkProxySpec;
pub use runtime::PermissionProfileState;
pub use runtime::Permissions;
pub use runtime::ProjectConfig;
pub use runtime::RealtimeAudioConfig;
pub use runtime::RealtimeConfig;
pub use runtime::SessionConfigOverlay;
pub use runtime::StartedNetworkProxy;
pub use runtime::TerminalResizeReflowConfig;
pub use runtime::TerminalResizeReflowMaxRows;
pub use runtime::ThreadStoreConfig;
pub use runtime::build_effective_session_config_from_session_overlay;
pub use runtime::build_per_turn_config_from_session_overlay;
pub use runtime::child_uses_parent_exec_policy;
pub use runtime::deserialize_config_toml_with_base;
pub use runtime::find_codex_home;
pub use runtime::hook_config_layer_stack_from_config_layer_stack;
pub use runtime::load_config_as_toml_with_cli_and_load_options;
pub use runtime::load_config_as_toml_with_cli_and_load_options_and_layer_loader;
pub use runtime::load_config_as_toml_with_cli_and_loader_overrides;
pub use runtime::load_config_as_toml_with_cli_overrides;
pub use runtime::log_dir;
pub use runtime::plugin_config_layer_stack_from_config_layer_stack;
pub use runtime::resolve_oss_provider;
pub use runtime::resolve_profile_v2_config_path;
pub use runtime::resolve_tool_suggest_config_from_layer_stack;
pub use runtime::resolve_web_search_mode_for_turn;
pub use runtime::set_default_oss_provider;
pub use runtime::set_project_trust_level;
pub use runtime::skill_config_layer_stack_from_config_layer_stack;
pub use runtime::validate_feature_requirements_for_config_toml;
pub use skills_config::BundledSkillsConfig;
pub use skills_config::SkillConfig;
pub use skills_config::SkillsConfig;
pub use state::ConfigLayerEntry;
pub use state::ConfigLayerStack;
pub use state::ConfigLayerStackOrdering;
pub use toml::Value as TomlValue;
