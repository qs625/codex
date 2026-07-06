use crate::agents_md::DEFAULT_AGENTS_MD_FILENAME;
use crate::agents_md::LOCAL_AGENTS_MD_FILENAME;
use crate::config::AgentCapabilityAllowlist;
use crate::config::AgentRoleSource;
use crate::config::CONFIG_TOML_FILE;
use crate::config::ThreadStoreConfig;
use crate::config::edit::ConfigEdit;
use crate::config::edit::ConfigEditsBuilder;
use crate::config::edit::apply_blocking;
use assert_matches::assert_matches;
use config_service::ConfigLayerEntry;
use config_service::ProfileV2Name;
use config_service::RequirementSource;
use config_service::config_toml::AgentRoleToml;
use config_service::config_toml::AgentsToml;
use config_service::config_toml::AutoReviewToml;
use config_service::config_toml::ConfigToml;
use config_service::config_toml::RealtimeConfig;
use config_service::config_toml::RealtimeToml;
use config_service::config_toml::RealtimeTransport;
use config_service::config_toml::RealtimeWsMode;
use config_service::config_toml::RealtimeWsVersion;
use config_service::config_toml::ToolsToml;
use config_service::permissions_toml::FilesystemPermissionToml;
use config_service::permissions_toml::FilesystemPermissionsToml;
use config_service::permissions_toml::NetworkDomainPermissionToml;
use config_service::permissions_toml::NetworkDomainPermissionsToml;
use config_service::permissions_toml::NetworkToml;
use config_service::permissions_toml::PermissionProfileToml;
use config_service::permissions_toml::PermissionsToml;
use config_service::permissions_toml::WorkspaceRootsToml;
use config_service::profile_toml::ConfigProfile;
use config_service::types::AltScreenMode;
use config_service::types::AppToolApproval;
use config_service::types::ApprovalsReviewer;
use config_service::types::BundledSkillsConfig;
use config_service::types::FeedbackConfigToml;
use config_service::types::History;
use config_service::types::HistoryPersistence;
use config_service::types::McpServerEnvVar;
use config_service::types::McpServerOAuthConfig;
use config_service::types::McpServerToolConfig;
use config_service::types::McpServerTransportConfig;
use config_service::types::MemoriesConfig;
use config_service::types::MemoriesToml;
use config_service::types::ModelAvailabilityNuxConfig;
use config_service::types::Notice;
use config_service::types::NotificationCondition;
use config_service::types::NotificationMethod;
use config_service::types::Notifications;
use config_service::types::OtelConfig;
use config_service::types::OtelConfigToml;
use config_service::types::OtelExporterKind;
use config_service::types::SandboxWorkspaceWrite;
use config_service::types::SessionPickerViewMode;
use config_service::types::SkillsConfig;
use config_service::types::ToolSuggestDisabledTool;
use config_service::types::ToolSuggestDiscoverableType;
use config_service::types::Tui;
use config_service::types::TuiKeymap;
use config_service::types::TuiNotificationSettings;
use config_service::types::TuiPetAnchor;
use config_service::types::WindowsSandboxModeToml;
use config_service::types::WindowsToml;
use crate::editing::load_global_mcp_servers;
use crate::loader::ProjectConfig;
use crate::local_loader::load_config_layers_state;
use codex_config_types::RealtimeAudioConfig;
use codex_features::Feature;
use codex_features::FeaturesToml;
use codex_file_system::LOCAL_FS;
use model_service::bundled_models_response;
use model_service_api::LMSTUDIO_OSS_PROVIDER_ID;
use model_service_api::ModelProviderInfo;
use model_service_api::OLLAMA_OSS_PROVIDER_ID;
use model_service_api::WireApi;
use plugin_service_api::LoadedPlugin;
use plugin_service_api::PluginLoadOutcome;
use plugin_service_api::PluginRuntime;
use plugin_service_api::PluginRuntimeFuture;
use plugin_service_api::PluginsConfigInput;
use plugin_service_api::ToolSuggestDiscoverablePlugin;
use protocol::config_types::ReasoningSummary;
use protocol::config_types::ServiceTier;
use protocol::config_types::ShellEnvironmentPolicy;
use protocol::config_types::Verbosity;
use protocol::models::ActivePermissionProfile;
use protocol::models::BUILT_IN_PERMISSION_PROFILE_DANGER_FULL_ACCESS;
use protocol::models::BUILT_IN_PERMISSION_PROFILE_READ_ONLY;
use protocol::models::BUILT_IN_PERMISSION_PROFILE_WORKSPACE;
use protocol::models::ManagedFileSystemPermissions;
use protocol::models::PermissionProfile;
use protocol::models::SandboxEnforcement;
use protocol::openai_models::ReasoningEffort;
use protocol::permissions::FileSystemAccessMode;
use protocol::permissions::FileSystemPath;
use protocol::permissions::FileSystemSandboxEntry;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::permissions::FileSystemSpecialPath;
use protocol::permissions::NetworkSandboxPolicy;
use protocol::protocol::NetworkAccess;
use protocol::protocol::RealtimeVoice;
use protocol::protocol::SandboxPolicy;
use serde::Deserialize;
use tempfile::tempdir;

use super::*;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::test_support::PathBufExt;
use codex_utils_absolute_path::test_support::PathExt;
use pretty_assertions::assert_eq;

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

fn test_absolute_path(unix_path: &str) -> AbsolutePathBuf {
    AbsolutePathBuf::from_absolute_path(test_path_buf(unix_path))
        .expect("test path should be absolute")
}

fn test_path_buf(unix_path: &str) -> std::path::PathBuf {
    if cfg!(windows) {
        let mut path = std::path::PathBuf::from(r"C:\");
        path.extend(
            unix_path
                .trim_start_matches('/')
                .split('/')
                .filter(|segment| !segment.is_empty()),
        );
        path
    } else {
        std::path::PathBuf::from(unix_path)
    }
}

trait TempDirExt {
    fn abs(&self) -> AbsolutePathBuf;
}

impl TempDirExt for TempDir {
    fn abs(&self) -> AbsolutePathBuf {
        self.path().abs()
    }
}

fn active_permission_profile_state(
    permission_profile: PermissionProfile,
    profile_id: impl Into<String>,
) -> PermissionProfileState {
    PermissionProfileState::from_constrained_active_profile(
        Constrained::allow_any(permission_profile),
        Some(ActivePermissionProfile::new(profile_id)),
        Vec::new(),
    )
    .expect("active permission profile state should be valid")
}

fn stdio_mcp(command: &str) -> McpServerConfig {
    McpServerConfig {
        transport: McpServerTransportConfig::Stdio {
            command: command.to_string(),
            args: Vec::new(),
            env: None,
            env_vars: Vec::new(),
            cwd: None,
        },
        experimental_environment: None,
        enabled: true,
        required: false,
        supports_parallel_tool_calls: false,
        disabled_reason: None,
        startup_timeout_sec: None,
        tool_timeout_sec: None,
        default_tools_approval_mode: None,
        enabled_tools: None,
        disabled_tools: None,
        scopes: None,
        oauth: None,
        oauth_resource: None,
        tools: HashMap::new(),
    }
}

fn http_mcp(url: &str) -> McpServerConfig {
    McpServerConfig {
        transport: McpServerTransportConfig::StreamableHttp {
            url: url.to_string(),
            bearer_token_env_var: None,
            http_headers: None,
            env_http_headers: None,
        },
        experimental_environment: None,
        enabled: true,
        required: false,
        supports_parallel_tool_calls: false,
        disabled_reason: None,
        startup_timeout_sec: None,
        tool_timeout_sec: None,
        default_tools_approval_mode: None,
        enabled_tools: None,
        disabled_tools: None,
        scopes: None,
        oauth: None,
        oauth_resource: None,
        tools: HashMap::new(),
    }
}

#[derive(Clone, Default)]
struct TestPluginRuntime {
    outcome: PluginLoadOutcome,
}

impl TestPluginRuntime {
    fn with_mcp_servers(mcp_servers: impl IntoIterator<Item = (String, McpServerConfig)>) -> Self {
        Self {
            outcome: PluginLoadOutcome::from_plugins(vec![LoadedPlugin {
                config_name: "sample@test".to_string(),
                manifest_name: Some("sample".to_string()),
                manifest_description: None,
                root: test_absolute_path("/tmp/test-plugin"),
                enabled: true,
                skill_roots: Vec::new(),
                disabled_skill_paths: Default::default(),
                has_enabled_skills: false,
                mcp_servers: mcp_servers.into_iter().collect(),
                apps: Vec::new(),
                hook_sources: Vec::new(),
                hook_load_warnings: Vec::new(),
                error: None,
            }]),
        }
    }
}

impl PluginRuntime for TestPluginRuntime {
    fn plugins_for_config<'a>(
        &'a self,
        _config: &'a PluginsConfigInput,
    ) -> PluginRuntimeFuture<'a, PluginLoadOutcome> {
        let outcome = self.outcome.clone();
        Box::pin(async move { outcome })
    }

    fn is_configured_plugin_installed(
        &self,
        _config: &PluginsConfigInput,
        _plugin_id: &str,
    ) -> bool {
        false
    }

    fn list_tool_suggest_discoverable_plugins<'a>(
        &'a self,
        _config: &'a PluginsConfigInput,
        _configured_plugin_ids: &'a std::collections::HashSet<String>,
        _disabled_plugin_ids: &'a std::collections::HashSet<String>,
    ) -> PluginRuntimeFuture<'a, Result<Vec<ToolSuggestDiscoverablePlugin>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn clear_cache(&self) {}
}

async fn derive_legacy_sandbox_policy_for_test(
    cfg: &ConfigToml,
    sandbox_mode_override: Option<SandboxMode>,
    profile_sandbox_mode: Option<SandboxMode>,
    windows_sandbox_level: WindowsSandboxLevel,
    active_project: Option<&ProjectConfig>,
    permission_profile_constraint: Option<&Constrained<PermissionProfile>>,
) -> SandboxPolicy {
    let permission_profile = cfg
        .derive_permission_profile(
            sandbox_mode_override,
            profile_sandbox_mode,
            windows_sandbox_level,
            active_project,
            permission_profile_constraint,
        )
        .await;
    permission_profile
        .to_legacy_sandbox_policy(Path::new("/"))
        .unwrap_or_else(|err| {
            tracing::warn!(
                error = %err,
                "derived permission profile cannot be represented as a legacy sandbox policy; falling back to read-only"
            );
            SandboxPolicy::new_read_only_policy()
        })
}


mod agent_roles_and_plugins;
mod approval_aliases_and_tail;
mod config_edits;
mod fixtures_and_requirements;
mod load_and_parse;
mod permissions_and_sandbox;
mod profile_precedence;
mod workspace_and_profiles;
