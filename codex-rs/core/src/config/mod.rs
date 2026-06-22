use crate::agents_md::AgentsMdManager;
use crate::config::edit::ConfigEdit;
use crate::config::edit::ConfigEditsBuilder;
use crate::path_utils::normalize_for_native_workdir;
use crate::unified_exec::DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
use crate::windows_sandbox::WindowsSandboxLevelExt;
use crate::windows_sandbox::resolve_windows_sandbox_mode;
use crate::windows_sandbox::resolve_windows_sandbox_private_desktop;
use codex_auth_types::AuthManagerConfig;
use codex_auth_types::ForcedChatgptWorkspaceIds;
use codex_config_diagnostics::io_error_from_config_error;
use codex_config_loader::ConfigLayerLoadRequest;
use codex_config_loader::ConfigLayerLoader;
use codex_config_loader::NoopThreadConfigLoader;
use codex_config_loader::ThreadConfigLoader;
use codex_config_loader::build_cli_overrides_layer;
use codex_config_requirements::CloudRequirementsLoader;
use codex_config_requirements::ConfigRequirements;
use codex_config_requirements::ConfigRequirementsToml;
use codex_config_requirements::ConstrainedWithSource;
use codex_config_requirements::FeatureRequirementsToml;
use codex_config_requirements::FilesystemConstraints;
use codex_config_requirements::McpServerIdentity;
use codex_config_requirements::McpServerRequirement;
use codex_config_requirements::PluginRequirementsToml;
use codex_config_requirements::SandboxModeRequirement;
use codex_config_requirements::Sourced;
use codex_config_requirements::sandbox_mode_requirement_for_permission_profile;
pub use codex_config_state::ConfigLayerStack;
use codex_config_state::ConfigLayerStackOrdering;
use codex_config_state::first_layer_config_error;
use codex_config_state::merge_toml_values;
use codex_config_toml::config_toml::ConfigToml;
use codex_config_toml::config_toml::DEFAULT_PROJECT_DOC_MAX_BYTES;
pub use codex_config_toml::config_toml::RealtimeConfig;
use codex_config_toml::config_without_lock_controls;
pub use codex_config_toml::deserialize_config_toml_with_base;
use codex_config_toml::profile_toml::ConfigProfile;
use codex_config_toml::read_config_lock_from_path;
use codex_config_types::AuthCredentialsStoreMode;
use codex_config_types::ConfigLayerSource;
use codex_config_types::McpServerConfig;
use codex_config_types::McpServerDisabledReason;
use codex_config_types::McpServerTransportConfig;
use codex_config_types::OAuthCredentialsStoreMode;
pub use codex_config_types::RealtimeAudioConfig;
use codex_config_types::ThreadStoreToml;
use codex_config_types::ToolSuggestConfig;
use codex_config_types::ToolSuggestDisabledTool;
use codex_config_types::ToolSuggestDiscoverable;
use codex_config_types::UriBasedFileOpener;
use codex_config_types::WindowsSandboxModeToml;
use codex_core_plugins_api::PluginRuntime;
use codex_core_plugins_api::PluginsConfigInput;
use codex_features::AppsMcpPathOverrideConfigToml;
use codex_features::Feature;
use codex_features::FeatureConfigSource;
use codex_features::FeatureOverrides;
use codex_features::FeatureToml;
use codex_features::Features;
use codex_features::FeaturesToml;
use codex_features::MultiAgentV2ConfigToml;
use codex_features::NetworkProxyConfigToml;
use codex_file_system::ExecutorFileSystem;
use codex_file_system::LOCAL_FS;
use codex_git_info::resolve_root_git_project_for_trust;
use codex_mcp_types::McpConfig;
use codex_memories_read_api::memory_root;
use codex_model_provider_info::LEGACY_OLLAMA_CHAT_PROVIDER_ID;
use codex_model_provider_info::ModelOptionToml;
use codex_model_provider_info::OLLAMA_CHAT_PROVIDER_REMOVED_ERROR;
use codex_model_provider_info::built_in_model_providers;
use codex_model_provider_info::merge_configured_model_providers;
use codex_model_provider_info::validate_model_providers;
use codex_model_provider_info::validate_oss_provider;
use codex_models_manager_api::ModelMetadataOverride;
use codex_models_manager_api::ModelsManagerConfig;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::config_types::Personality;
use codex_protocol::config_types::ProfileV2Name;
use codex_protocol::config_types::SandboxMode;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::config_types::TrustLevel;
use codex_protocol::config_types::WebSearchConfig;
use codex_protocol::config_types::WebSearchMode;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::ActivePermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxEnforcement;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::SandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use crate::config::permissions::BUILT_IN_WORKSPACE_PROFILE;
use crate::config::permissions::apply_network_proxy_feature_config;
use crate::config::permissions::builtin_permission_profile;
use crate::config::permissions::compile_permission_profile_selection;
use crate::config::permissions::compile_permission_profile_workspace_roots;
use crate::config::permissions::default_builtin_permission_profile_name;
use crate::config::permissions::get_readable_roots_required_for_codex_runtime;
use crate::config::permissions::network_proxy_config_for_profile_selection;
use crate::config::permissions::validate_user_permission_profile_names;
use codex_network_proxy_api::NetworkProxyConfig;
use codex_sandboxing_api::compatibility_sandbox_policy_for_permission_profile;
use toml::Value as TomlValue;

pub(crate) mod agent_roles;
mod builder;
pub mod edit;
mod feature_resolvers;
mod layer_stacks;
mod load_config;
mod managed_features;
mod mcp_requirements;
mod model_settings;
mod network_proxy_spec;
mod otel;
mod permission_resolution;
mod permission_settings;
mod permissions;
mod resolved_permission_profile;
mod runtime_config;
#[cfg(test)]
mod schema;
mod tool_suggest;
pub use builder::ConfigBuilder;
pub use builder::load_config_as_toml_with_cli_and_load_options;
pub use builder::load_config_as_toml_with_cli_and_load_options_and_layer_loader;
pub use builder::load_config_as_toml_with_cli_and_loader_overrides;
pub use builder::load_config_as_toml_with_cli_overrides;
pub use builder::resolve_profile_v2_config_path;
pub use codex_agent_roles::AgentCapabilityAllowlist;
pub use codex_agent_roles::AgentRoleConfig;
pub use codex_agent_roles::AgentRoleSource;
pub use codex_config_loader::ConfigLoadOptions;
pub use codex_config_loader::LoaderOverrides;
pub use codex_config_loader::ProjectConfig;
pub use codex_config_types::CONFIG_TOML_FILE;
pub use codex_config_types::Constrained;
pub use codex_config_types::ConstraintError;
pub use codex_config_types::ConstraintResult;
pub use codex_network_proxy_api::NetworkProxyAuditMetadata;
pub use layer_stacks::hook_config_layer_stack_from_config_layer_stack;
pub use layer_stacks::plugin_config_layer_stack_from_config_layer_stack;
pub use layer_stacks::skill_config_layer_stack_from_config_layer_stack;
pub use managed_features::ManagedFeatures;
use mcp_requirements::apply_requirement_constrained_value;
use mcp_requirements::constrain_mcp_servers;
use mcp_requirements::filter_mcp_servers_by_requirements;
use mcp_requirements::filter_plugin_mcp_servers_by_requirements;
use model_settings::load_model_catalog;
use model_settings::validate_model_options;
pub use network_proxy_spec::NetworkProxySpec;
pub use network_proxy_spec::StartedNetworkProxy;
use permission_resolution::PermissionConfigSyntax;
use permission_resolution::apply_managed_filesystem_constraints;
use permission_resolution::resolve_permission_config_syntax;
pub use permission_settings::Permissions;
use permission_settings::profile_allows_configured_network_proxy;
pub(crate) use resolved_permission_profile::PermissionProfileState;
pub use runtime_config::Config;
pub use runtime_config::MultiAgentV2Config;
pub use runtime_config::TerminalResizeReflowConfig;
pub use runtime_config::TerminalResizeReflowMaxRows;
pub use runtime_config::ThreadStoreConfig;
use tool_suggest::is_session_layer;
use tool_suggest::resolve_tool_suggest_config;
pub(crate) use tool_suggest::resolve_tool_suggest_config_from_layer_stack;
use tool_suggest::thread_store_config;

use feature_resolvers::apps_mcp_path_override_toml_config;
use feature_resolvers::network_proxy_toml_config;
use feature_resolvers::resolve_multi_agent_v2_config;
use feature_resolvers::resolve_terminal_resize_reflow_config;
use feature_resolvers::resolve_web_search_config;
use feature_resolvers::resolve_web_search_mode;
pub(crate) use feature_resolvers::resolve_web_search_mode_for_turn;
use feature_resolvers::validate_multi_agent_v2_wait_timeout;

const DEFAULT_IGNORE_LARGE_UNTRACKED_DIRS: i64 = 200;
const DEFAULT_IGNORE_LARGE_UNTRACKED_FILES: i64 = 10 * 1024 * 1024;

fn lock_layer_from_config(
    lock_path: &AbsolutePathBuf,
    lockfile: &codex_config_types::ConfigLockfileToml<ConfigToml>,
) -> std::io::Result<codex_config_state::ConfigLayerEntry> {
    let value = toml::Value::try_from(config_without_lock_controls(&lockfile.config))
        .map_err(|err| std::io::Error::other(format!("failed to serialize config lock: {err}")))?;
    Ok(codex_config_state::ConfigLayerEntry::new(
        ConfigLayerSource::User {
            file: lock_path.clone(),
            profile: None,
        },
        value,
    ))
}

/// Compatibility-only config retained so legacy `ghost_snapshot` settings
/// continue to load even though snapshots are no longer produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GhostSnapshotConfig {
    pub ignore_large_untracked_files: Option<i64>,
    pub ignore_large_untracked_dirs: Option<i64>,
    pub disable_warnings: bool,
}

impl Default for GhostSnapshotConfig {
    fn default() -> Self {
        Self {
            ignore_large_untracked_files: Some(DEFAULT_IGNORE_LARGE_UNTRACKED_FILES),
            ignore_large_untracked_dirs: Some(DEFAULT_IGNORE_LARGE_UNTRACKED_DIRS),
            disable_warnings: false,
        }
    }
}

/// Maximum number of bytes of the documentation that will be embedded. Larger
/// files are *silently truncated* to this size so we do not take up too much of
/// the context window.
pub(crate) const AGENTS_MD_MAX_BYTES: usize = DEFAULT_PROJECT_DOC_MAX_BYTES; // 32 KiB
pub(crate) const DEFAULT_AGENT_MAX_THREADS: Option<usize> = Some(6);
pub(crate) const DEFAULT_MULTI_AGENT_V2_MAX_CONCURRENT_THREADS_PER_SESSION: usize = 4;
pub(crate) const DEFAULT_MULTI_AGENT_V2_MIN_WAIT_TIMEOUT_MS: i64 = 10_000;
pub(crate) const DEFAULT_MULTI_AGENT_V2_MAX_WAIT_TIMEOUT_MS: i64 = 30 * 60 * 1000;
pub(crate) const DEFAULT_MULTI_AGENT_V2_DEFAULT_WAIT_TIMEOUT_MS: i64 = 60_000;
pub(crate) const HARD_MIN_MULTI_AGENT_V2_TIMEOUT_MS: i64 = 0;
pub(crate) const HARD_MAX_MULTI_AGENT_V2_TIMEOUT_MS: i64 =
    DEFAULT_MULTI_AGENT_V2_MAX_WAIT_TIMEOUT_MS;
pub(crate) const DEFAULT_AGENT_MAX_DEPTH: i32 = 1;
pub(crate) const DEFAULT_AGENT_JOB_MAX_RUNTIME_SECONDS: Option<u64> = None;

const LOCAL_DEV_BUILD_VERSION: &str = "0.0.0";

const CONFIG_PROFILE_V2_SUFFIX: &str = ".config.toml";

fn resolve_sqlite_home_env(resolved_cwd: &Path) -> Option<PathBuf> {
    let raw = std::env::var(codex_state_api::SQLITE_HOME_ENV).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(resolved_cwd.join(path))
    }
}

fn resolve_cli_auth_credentials_store_mode(
    configured: AuthCredentialsStoreMode,
    package_version: &str,
) -> AuthCredentialsStoreMode {
    match (package_version, configured) {
        (
            LOCAL_DEV_BUILD_VERSION,
            AuthCredentialsStoreMode::Keyring | AuthCredentialsStoreMode::Auto,
        ) => AuthCredentialsStoreMode::File,
        (_, mode) => mode,
    }
}

fn resolve_mcp_oauth_credentials_store_mode(
    configured: OAuthCredentialsStoreMode,
    package_version: &str,
) -> OAuthCredentialsStoreMode {
    match (package_version, configured) {
        (
            LOCAL_DEV_BUILD_VERSION,
            OAuthCredentialsStoreMode::Keyring | OAuthCredentialsStoreMode::Auto,
        ) => OAuthCredentialsStoreMode::File,
        (_, mode) => mode,
    }
}

#[cfg(test)]
pub(crate) async fn test_config() -> Config {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    Config::load_from_base_config_with_overrides(
        ConfigToml::default(),
        ConfigOverrides::default(),
        AbsolutePathBuf::from_absolute_path(codex_home.path()).expect("temp dir should resolve"),
    )
    .await
    .expect("load default test config")
}

impl AuthManagerConfig for Config {
    fn codex_home(&self) -> PathBuf {
        self.codex_home.to_path_buf()
    }

    fn cli_auth_credentials_store_mode(&self) -> AuthCredentialsStoreMode {
        self.cli_auth_credentials_store_mode
    }

    fn forced_chatgpt_workspace_id(&self) -> Option<Vec<String>> {
        self.forced_chatgpt_workspace_id.clone()
    }

    fn chatgpt_base_url(&self) -> String {
        self.chatgpt_base_url.clone()
    }
}

/// Validate user-visible feature settings against managed feature requirements.
pub fn validate_feature_requirements_for_config_toml(
    cfg: &ConfigToml,
    feature_requirements: Option<&Sourced<FeatureRequirementsToml>>,
) -> std::io::Result<()> {
    managed_features::validate_explicit_feature_settings_in_config_toml(cfg, feature_requirements)?;
    managed_features::validate_feature_requirements_in_config_toml(cfg, feature_requirements)
}

/// Patch `CODEX_HOME/config.toml` project state to set trust level.
/// Use with caution.
pub fn set_project_trust_level(
    codex_home: &Path,
    project_path: &Path,
    trust_level: TrustLevel,
) -> anyhow::Result<()> {
    use crate::config::edit::ConfigEditsBuilder;

    ConfigEditsBuilder::new(codex_home)
        .set_project_trust_level(project_path, trust_level)
        .apply_blocking()
}

/// Save the default OSS provider preference to config.toml
pub fn set_default_oss_provider(codex_home: &Path, provider: &str) -> std::io::Result<()> {
    validate_oss_provider(provider)?;
    let edits = [ConfigEdit::set_string_path(
        vec!["oss_provider".to_string()],
        provider,
    )];

    ConfigEditsBuilder::new(codex_home)
        .with_edits(edits)
        .apply_blocking()
        .map_err(|err| std::io::Error::other(format!("failed to persist config.toml: {err}")))
}

/// Optional overrides for user configuration (e.g., from CLI flags).
#[derive(Default, Debug, Clone)]
pub struct ConfigOverrides {
    pub model: Option<String>,
    pub review_model: Option<String>,
    pub cwd: Option<PathBuf>,
    pub approval_policy: Option<AskForApproval>,
    pub approvals_reviewer: Option<ApprovalsReviewer>,
    pub sandbox_mode: Option<SandboxMode>,
    pub permission_profile: Option<PermissionProfile>,
    pub default_permissions: Option<String>,
    pub model_provider: Option<String>,
    pub service_tier: Option<Option<String>>,
    pub config_profile: Option<String>,
    pub codex_self_exe: Option<PathBuf>,
    pub codex_linux_sandbox_exe: Option<PathBuf>,
    pub main_execve_wrapper_exe: Option<PathBuf>,
    pub zsh_path: Option<PathBuf>,
    pub base_instructions: Option<String>,
    pub developer_instructions: Option<String>,
    pub personality: Option<Personality>,
    pub compact_prompt: Option<String>,
    pub show_raw_agent_reasoning: Option<bool>,
    pub tools_web_search_request: Option<bool>,
    pub ephemeral: Option<bool>,
    pub bypass_hook_trust: Option<bool>,
    /// Additional directories that should be treated as writable roots for this session.
    pub additional_writable_roots: Vec<PathBuf>,
    /// Explicit runtime workspace roots for this session. When set, this is
    /// the full runtime root list rather than an additive override.
    pub workspace_roots: Option<Vec<PathBuf>>,
}

fn dedupe_absolute_paths(paths: &mut Vec<AbsolutePathBuf>) {
    let mut seen = HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

/// Resolves the OSS provider from CLI override, profile config, or global config.
/// Returns `None` if no provider is configured at any level.
pub fn resolve_oss_provider(
    explicit_provider: Option<&str>,
    config_toml: &ConfigToml,
    config_profile: Option<String>,
) -> Option<String> {
    if let Some(provider) = explicit_provider {
        // Explicit provider specified (e.g., via --local-provider)
        Some(provider.to_string())
    } else {
        // Check profile config first, then global config
        let profile = config_toml.get_config_profile(config_profile).ok();
        if let Some(profile) = &profile {
            // Check if profile has an oss provider
            if let Some(profile_oss_provider) = &profile.oss_provider {
                Some(profile_oss_provider.clone())
            }
            // If not then check if the toml has an oss provider
            else {
                config_toml.oss_provider.clone()
            }
        } else {
            config_toml.oss_provider.clone()
        }
    }
}

fn guardian_policy_config_from_requirements(
    requirements_toml: &ConfigRequirementsToml,
) -> Option<String> {
    normalize_guardian_policy_config(requirements_toml.guardian_policy_config.as_deref())
}

fn normalize_guardian_policy_config(value: Option<&str>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}

/// Returns the path to the Codex configuration directory, which can be
/// specified by the `CODEX_HOME` environment variable. If not set, defaults to
/// `~/.codex`.
///
/// - If `CODEX_HOME` is set, the value must exist and be a directory. The
///   value will be canonicalized and this function will Err otherwise.
/// - If `CODEX_HOME` is not set, this function does not verify that the
///   directory exists.
pub fn find_codex_home() -> std::io::Result<AbsolutePathBuf> {
    codex_utils_home_dir::find_codex_home()
}

/// Returns the path to the folder where Codex logs are stored. Does not verify
/// that the directory exists.
pub fn log_dir(cfg: &Config) -> std::io::Result<PathBuf> {
    Ok(cfg.log_dir.clone())
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "config_loader_tests.rs"]
mod config_loader_tests;
