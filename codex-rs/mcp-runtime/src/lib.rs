use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use codex_api_auth::auth_provider_from_auth_snapshot;
use codex_auth_types::RequestAuthSnapshot;
use codex_config::Config;
use codex_config::McpServerConfig;
use codex_core_plugins_api::SharedPluginRuntime;
use codex_exec_server_api::ExecEnvironment;
use codex_mcp_runtime_api::McpRuntimeEnvironment;
use codex_mcp_runtime_api::McpRuntimeEnvironmentParams;
use codex_mcp_runtime_api::SharedMcpAuthHeaderProvider;
use codex_mcp_runtime_api::StaticMcpAuthHeaderProvider;
use codex_mcp_types::CodexAppsAuthContext;
use codex_mcp_types::EffectiveMcpServer;
use codex_mcp_types::ToolPluginProvenance;
use codex_mcp_types::configured_mcp_servers;
use codex_mcp_types::effective_mcp_servers;
use codex_mcp_types::tool_plugin_provenance as collect_tool_plugin_provenance;

mod app_tools;
mod openai_file;
mod skill_dependencies;
mod tool_exposure;

pub use app_tools::AppToolPolicy;
#[cfg(any(test, feature = "test-support"))]
pub use app_tools::app_is_enabled;
pub use app_tools::app_tool_policy;
#[cfg(any(test, feature = "test-support"))]
pub use app_tools::app_tool_policy_from_apps_config;
#[cfg(any(test, feature = "test-support"))]
pub use app_tools::apply_requirements_apps_constraints;
pub use app_tools::codex_app_tool_is_enabled;
#[cfg(any(test, feature = "test-support"))]
pub use app_tools::managed_app_tool_approval;
pub use app_tools::with_app_enabled_state;
pub use openai_file::OpenAiFilePathResolver;
pub use openai_file::rewrite_mcp_tool_arguments_for_openai_files;
pub use skill_dependencies::McpSkillDependencyHost;
pub use skill_dependencies::McpSkillDependencyTurnContext;
pub use skill_dependencies::maybe_install_mcp_dependencies;
pub use skill_dependencies::maybe_prompt_and_install_mcp_dependencies;
pub use tool_exposure::DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD;
pub use tool_exposure::McpToolExposure;
pub use tool_exposure::build_mcp_tool_exposure;

pub fn codex_apps_auth_provider(
    auth: Option<&RequestAuthSnapshot>,
) -> Option<SharedMcpAuthHeaderProvider> {
    auth.filter(|auth| auth.uses_codex_backend())
        .map(auth_provider_from_auth_snapshot)
        .map(|auth_provider| StaticMcpAuthHeaderProvider::shared(auth_provider.to_auth_headers()))
}

pub fn codex_apps_auth_context(auth: Option<&RequestAuthSnapshot>) -> Option<CodexAppsAuthContext> {
    auth.map(|auth| CodexAppsAuthContext {
        uses_codex_backend: auth.uses_codex_backend(),
        account_id: auth.account_id().map(ToOwned::to_owned),
        chatgpt_user_id: auth.chatgpt_user_id().map(ToOwned::to_owned),
        is_workspace_account: auth.is_workspace_account(),
    })
}

pub fn mcp_runtime_environment(
    environment: Arc<dyn ExecEnvironment>,
    local_environment: Arc<dyn ExecEnvironment>,
    fallback_cwd: PathBuf,
) -> McpRuntimeEnvironment {
    let local_http_client = local_environment.get_http_client();
    McpRuntimeEnvironment::new(McpRuntimeEnvironmentParams {
        remote_available: environment.is_remote(),
        remote_exec_backend: environment.get_exec_backend(),
        local_http_client,
        remote_http_client: environment.get_http_client(),
        fallback_cwd,
    })
}

#[derive(Clone)]
pub struct McpManager {
    plugins_manager: SharedPluginRuntime,
}

impl McpManager {
    pub fn new(plugins_manager: SharedPluginRuntime) -> Self {
        Self { plugins_manager }
    }

    pub async fn configured_servers(&self, config: &Config) -> HashMap<String, McpServerConfig> {
        let mcp_config = config.to_mcp_config(self.plugins_manager.as_ref()).await;
        configured_mcp_servers(&mcp_config)
    }

    pub async fn effective_servers(
        &self,
        config: &Config,
        auth_context: Option<&CodexAppsAuthContext>,
    ) -> HashMap<String, EffectiveMcpServer> {
        let mcp_config = config.to_mcp_config(self.plugins_manager.as_ref()).await;
        effective_mcp_servers(&mcp_config, auth_context)
    }

    pub async fn tool_plugin_provenance(&self, config: &Config) -> ToolPluginProvenance {
        let mcp_config = config.to_mcp_config(self.plugins_manager.as_ref()).await;
        collect_tool_plugin_provenance(&mcp_config)
    }
}
