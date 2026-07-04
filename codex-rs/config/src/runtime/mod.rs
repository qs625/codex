use codex_auth_types::AuthManagerConfig;
use codex_auth_types::ForcedChatgptWorkspaceIds;
use codex_config_state::io_error_from_config_error;
use crate::editing::ConfigEdit;
use crate::editing::ConfigEditsBuilder;
use crate::loader::ConfigLayerLoadRequest;
use crate::loader::ConfigLayerLoader;
use crate::loader::NoopThreadConfigLoader;
use crate::loader::ThreadConfigLoader;
use crate::loader::build_cli_overrides_layer;
use crate::CloudRequirementsLoader;
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
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_path::normalize_for_native_workdir;
use command_service_api::DEFAULT_MAX_BACKGROUND_TERMINAL_TIMEOUT_MS;
use mcp_types::McpConfig;
use memory_service_api::memory_root;
use model_service_api::LEGACY_OLLAMA_CHAT_PROVIDER_ID;
use model_service_api::ModelMetadataOverride;
use model_service_api::ModelOptionToml;
use model_service_api::ModelsManagerConfig;
use model_service_api::OLLAMA_CHAT_PROVIDER_REMOVED_ERROR;
use model_service_api::built_in_model_providers;
use model_service_api::merge_configured_model_providers;
use model_service_api::validate_model_providers;
use model_service_api::validate_oss_provider;
use plugin_service_api::PluginRuntime;
use plugin_service_api::PluginsConfigInput;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::Personality;
use protocol::config_types::ProfileV2Name;
use protocol::config_types::SandboxMode;
use protocol::config_types::ServiceTier;
use protocol::config_types::TrustLevel;
use protocol::config_types::WebSearchConfig;
use protocol::config_types::WebSearchMode;
use protocol::config_types::WindowsSandboxLevel;
use protocol::models::ActivePermissionProfile;
use protocol::models::PermissionProfile;
use protocol::models::SandboxEnforcement;
use protocol::openai_models::ModelsResponse;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::protocol::AskForApproval;
use protocol::protocol::SandboxPolicy;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use codex_network_proxy_api::NetworkProxyConfig;
use codex_sandboxing_api::compatibility_sandbox_policy_for_permission_profile;
use permissions::BUILT_IN_WORKSPACE_PROFILE;
use permissions::apply_network_proxy_feature_config;
use permissions::builtin_permission_profile;
use permissions::compile_permission_profile_selection;
use permissions::compile_permission_profile_workspace_roots;
use permissions::default_builtin_permission_profile_name;
use permissions::get_readable_roots_required_for_codex_runtime;
use permissions::network_proxy_config_for_profile_selection;
use permissions::validate_user_permission_profile_names;
use toml::Value as TomlValue;

pub mod agent_roles;
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
mod session_overlay;
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
pub use crate::loader::ConfigLoadOptions;
pub use crate::loader::LoaderOverrides;
pub use crate::loader::ProjectConfig;
pub use codex_config_types::CONFIG_TOML_FILE;
pub use codex_config_types::Constrained;
pub use codex_config_types::ConstraintError;
pub use codex_config_types::ConstraintResult;
pub use codex_network_proxy_api::NetworkProxyAuditMetadata;
pub use layer_stacks::child_uses_parent_exec_policy;
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
pub use resolved_permission_profile::PermissionProfileState;
pub use runtime_config::Config;
pub use runtime_config::MultiAgentV2Config;
pub use runtime_config::TerminalResizeReflowConfig;
pub use runtime_config::TerminalResizeReflowMaxRows;
pub use runtime_config::ThreadStoreConfig;
pub use session_overlay::EffectiveSessionConfigOverlay;
pub use session_overlay::SessionConfigOverlay;
pub use session_overlay::build_effective_session_config_from_session_overlay;
pub use session_overlay::build_per_turn_config_from_session_overlay;
use tool_suggest::is_session_layer;
use tool_suggest::resolve_tool_suggest_config;
pub use tool_suggest::resolve_tool_suggest_config_from_layer_stack;
use tool_suggest::thread_store_config;

use feature_resolvers::apps_mcp_path_override_toml_config;
use feature_resolvers::network_proxy_toml_config;
use feature_resolvers::resolve_multi_agent_v2_config;
use feature_resolvers::resolve_terminal_resize_reflow_config;
use feature_resolvers::resolve_web_search_config;
use feature_resolvers::resolve_web_search_mode;
pub use feature_resolvers::resolve_web_search_mode_for_turn;
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
pub(crate) const DEFAULT_MULTI_AGENT_V2_MAX_CONCURRENT_THREADS_PER_SESSION: usize = 4;
pub(crate) const DEFAULT_MULTI_AGENT_V2_MIN_WAIT_TIMEOUT_MS: i64 = 10_000;
pub(crate) const DEFAULT_MULTI_AGENT_V2_MAX_WAIT_TIMEOUT_MS: i64 = 30 * 60 * 1000;
pub(crate) const DEFAULT_MULTI_AGENT_V2_DEFAULT_WAIT_TIMEOUT_MS: i64 = 60_000;
pub(crate) const HARD_MIN_MULTI_AGENT_V2_TIMEOUT_MS: i64 = 0;
pub(crate) const HARD_MAX_MULTI_AGENT_V2_TIMEOUT_MS: i64 =
    DEFAULT_MULTI_AGENT_V2_MAX_WAIT_TIMEOUT_MS;
pub(crate) const DEFAULT_AGENT_MAX_DEPTH: i32 = 1;
pub(crate) const DEFAULT_AGENT_JOB_MAX_RUNTIME_SECONDS: Option<u64> = None;

pub const DEFAULT_AGENTS_MD_FILENAME: &str = "AGENTS.md";
pub const LOCAL_AGENTS_MD_FILENAME: &str = "AGENTS.override.md";

const LOCAL_DEV_BUILD_VERSION: &str = "0.0.0";

const CONFIG_PROFILE_V2_SUFFIX: &str = ".config.toml";

fn resolve_sqlite_home_env(resolved_cwd: &Path) -> Option<PathBuf> {
    let raw = std::env::var(state_api::SQLITE_HOME_ENV).ok()?;
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

fn load_global_instructions(codex_dir: Option<&AbsolutePathBuf>) -> Option<String> {
    let base = codex_dir?;
    for candidate in [LOCAL_AGENTS_MD_FILENAME, DEFAULT_AGENTS_MD_FILENAME] {
        let path = base.join(candidate);
        if let Ok(contents) = std::fs::read_to_string(&path) {
            let trimmed = contents.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn windows_sandbox_level_from_features(features: &Features) -> WindowsSandboxLevel {
    if features.enabled(Feature::WindowsSandboxElevated) {
        return WindowsSandboxLevel::Elevated;
    }
    if features.enabled(Feature::WindowsSandbox) {
        WindowsSandboxLevel::RestrictedToken
    } else {
        WindowsSandboxLevel::Disabled
    }
}

fn resolve_windows_sandbox_mode(
    cfg: &ConfigToml,
    profile: &ConfigProfile,
) -> Option<WindowsSandboxModeToml> {
    if let Some(mode) = legacy_windows_sandbox_mode(profile.features.as_ref()) {
        return Some(mode);
    }
    if legacy_windows_sandbox_keys_present(profile.features.as_ref()) {
        return None;
    }

    profile
        .windows
        .as_ref()
        .and_then(|windows| windows.sandbox)
        .or_else(|| cfg.windows.as_ref().and_then(|windows| windows.sandbox))
        .or_else(|| legacy_windows_sandbox_mode(cfg.features.as_ref()))
}

fn resolve_windows_sandbox_private_desktop(cfg: &ConfigToml, profile: &ConfigProfile) -> bool {
    profile
        .windows
        .as_ref()
        .and_then(|windows| windows.sandbox_private_desktop)
        .or_else(|| {
            cfg.windows
                .as_ref()
                .and_then(|windows| windows.sandbox_private_desktop)
        })
        .unwrap_or(true)
}

fn legacy_windows_sandbox_keys_present(features: Option<&FeaturesToml>) -> bool {
    let Some(entries) = features.map(FeaturesToml::entries) else {
        return false;
    };
    entries.contains_key(Feature::WindowsSandboxElevated.key())
        || entries.contains_key(Feature::WindowsSandbox.key())
        || entries.contains_key("enable_experimental_windows_sandbox")
}

fn legacy_windows_sandbox_mode(features: Option<&FeaturesToml>) -> Option<WindowsSandboxModeToml> {
    let entries = features.map(FeaturesToml::entries)?;
    if entries
        .get(Feature::WindowsSandboxElevated.key())
        .copied()
        .unwrap_or(false)
    {
        return Some(WindowsSandboxModeToml::Elevated);
    }
    if entries
        .get(Feature::WindowsSandbox.key())
        .copied()
        .unwrap_or(false)
        || entries
            .get("enable_experimental_windows_sandbox")
            .copied()
            .unwrap_or(false)
    {
        Some(WindowsSandboxModeToml::Unelevated)
    } else {
        None
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
    ConfigEditsBuilder::new(codex_home)
        .set_project_trust_level(project_path, trust_level.to_string())
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
