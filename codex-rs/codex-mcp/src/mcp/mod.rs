pub use auth::DefaultMcpAuthRuntime;
pub use auth::compute_auth_statuses;
pub use auth::discover_supported_scopes;
pub use auth::oauth_login_support;
pub use auth::should_retry_without_scopes;
pub use codex_mcp_types::McpAuthStatusEntry;
pub use codex_mcp_types::McpOAuthScopesSource;
pub use codex_mcp_types::ResolvedMcpOAuthScopes;
pub use codex_mcp_types::resolve_oauth_scopes;

pub(crate) mod auth;

use std::collections::HashMap;

use async_channel::unbounded;
use codex_mcp_tool_types::McpTool;
use codex_mcp_types::McpConfig;
use codex_mcp_types::effective_mcp_servers;
use codex_mcp_types::host_owned_codex_apps_enabled;
use codex_mcp_types::tool_plugin_provenance;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::mcp::ReadResourceResult;
use codex_protocol::mcp::Resource;
use codex_protocol::mcp::ResourceTemplate;
use codex_protocol::mcp::Tool;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::McpAuthStatus;
use serde_json::Value;

use crate::connection_manager::McpConnectionManager;
use codex_mcp_runtime_api::McpRuntimeEnvironment;
use codex_mcp_runtime_api::SharedMcpAuthHeaderProvider;

pub use codex_mcp_types::CODEX_APPS_MCP_SERVER_NAME;
pub use codex_mcp_types::CodexAppsAuthContext;
pub use codex_mcp_types::McpPermissionPromptAutoApproveContext;
pub use codex_mcp_types::codex_apps_tools_cache_key;
pub use codex_mcp_types::mcp_permission_prompt_is_auto_approved;
const MCP_TOOL_NAME_PREFIX: &str = "mcp";
const MCP_TOOL_NAME_DELIMITER: &str = "__";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum McpSnapshotDetail {
    #[default]
    Full,
    ToolsAndAuthOnly,
}

impl McpSnapshotDetail {
    fn include_resources(self) -> bool {
        matches!(self, Self::Full)
    }
}

pub fn qualified_mcp_tool_name_prefix(server_name: &str) -> String {
    sanitize_responses_api_tool_name(&format!(
        "{MCP_TOOL_NAME_PREFIX}{MCP_TOOL_NAME_DELIMITER}{server_name}{MCP_TOOL_NAME_DELIMITER}"
    ))
}

pub async fn read_mcp_resource(
    config: &McpConfig,
    auth_context: Option<&CodexAppsAuthContext>,
    codex_apps_auth_provider: Option<SharedMcpAuthHeaderProvider>,
    runtime_environment: McpRuntimeEnvironment,
    server: &str,
    uri: &str,
) -> anyhow::Result<ReadResourceResult> {
    let mut mcp_servers = effective_mcp_servers(config, auth_context);
    let host_owned_codex_apps_enabled = host_owned_codex_apps_enabled(config, auth_context);
    mcp_servers.retain(|name, _| name == server);
    let auth_statuses = compute_auth_statuses(
        mcp_servers.iter(),
        config.mcp_oauth_credentials_store_mode,
        host_owned_codex_apps_enabled,
    )
    .await;
    let (tx_event, rx_event) = unbounded();
    drop(rx_event);
    let (manager, cancel_token) = McpConnectionManager::new(
        &mcp_servers,
        config.mcp_oauth_credentials_store_mode,
        auth_statuses,
        &config.approval_policy,
        String::new(),
        tx_event,
        PermissionProfile::default(),
        runtime_environment,
        config.codex_home.clone(),
        codex_apps_tools_cache_key(auth_context),
        host_owned_codex_apps_enabled,
        config.client_elicitation_support,
        tool_plugin_provenance(config),
        codex_apps_auth_provider,
        /*elicitation_reviewer*/ None,
    )
    .await;

    let result = manager
        .read_resource(
            server,
            ReadResourceRequestParams {
                uri: uri.to_string(),
            },
        )
        .await;
    cancel_token.cancel();
    result
}

#[derive(Debug, Clone)]
pub struct McpServerStatusSnapshot {
    pub tools_by_server: HashMap<String, HashMap<String, Tool>>,
    pub resources: HashMap<String, Vec<Resource>>,
    pub resource_templates: HashMap<String, Vec<ResourceTemplate>>,
    pub auth_statuses: HashMap<String, McpAuthStatus>,
}

pub async fn collect_mcp_server_status_snapshot_with_detail(
    config: &McpConfig,
    auth_context: Option<&CodexAppsAuthContext>,
    codex_apps_auth_provider: Option<SharedMcpAuthHeaderProvider>,
    submit_id: String,
    runtime_environment: McpRuntimeEnvironment,
    detail: McpSnapshotDetail,
) -> McpServerStatusSnapshot {
    let mcp_servers = effective_mcp_servers(config, auth_context);
    let host_owned_codex_apps_enabled = host_owned_codex_apps_enabled(config, auth_context);
    let tool_plugin_provenance = tool_plugin_provenance(config);
    if mcp_servers.is_empty() {
        return McpServerStatusSnapshot {
            tools_by_server: HashMap::new(),
            resources: HashMap::new(),
            resource_templates: HashMap::new(),
            auth_statuses: HashMap::new(),
        };
    }

    let auth_status_entries = compute_auth_statuses(
        mcp_servers.iter(),
        config.mcp_oauth_credentials_store_mode,
        host_owned_codex_apps_enabled,
    )
    .await;

    let (tx_event, rx_event) = unbounded();
    drop(rx_event);

    let (mcp_connection_manager, cancel_token) = McpConnectionManager::new(
        &mcp_servers,
        config.mcp_oauth_credentials_store_mode,
        auth_status_entries.clone(),
        &config.approval_policy,
        submit_id,
        tx_event,
        PermissionProfile::default(),
        runtime_environment,
        config.codex_home.clone(),
        codex_apps_tools_cache_key(auth_context),
        host_owned_codex_apps_enabled,
        config.client_elicitation_support,
        tool_plugin_provenance,
        codex_apps_auth_provider,
        /*elicitation_reviewer*/ None,
    )
    .await;

    let snapshot = collect_mcp_server_status_snapshot_from_manager(
        &mcp_connection_manager,
        auth_status_entries,
        detail,
    )
    .await;

    cancel_token.cancel();

    snapshot
}

/// The Responses API requires tool names to match `^[a-zA-Z0-9_-]+$`.
/// MCP server/tool names are user-controlled, so sanitize the fully-qualified
/// name we expose to the model by replacing any disallowed character with `_`.
pub(crate) fn sanitize_responses_api_tool_name(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            sanitized.push(c);
        } else {
            sanitized.push('_');
        }
    }

    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

fn protocol_tool_from_mcp_tool(name: &str, tool: &McpTool) -> Option<Tool> {
    let annotations = match tool
        .annotations
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
    {
        Ok(annotations) => annotations,
        Err(err) => {
            tracing::warn!("Failed to serialize MCP tool annotations for '{name}': {err}");
            return None;
        }
    };
    Some(Tool {
        name: tool.name.clone(),
        title: tool.title.clone(),
        description: tool.description.clone(),
        input_schema: tool.input_schema.clone(),
        output_schema: tool.output_schema.clone(),
        annotations,
        icons: tool.icons.clone(),
        meta: tool.meta.clone().map(Value::Object),
    })
}

fn auth_statuses_from_entries(
    auth_status_entries: &HashMap<String, McpAuthStatusEntry>,
) -> HashMap<String, McpAuthStatus> {
    auth_status_entries
        .iter()
        .map(|(name, entry)| (name.clone(), entry.auth_status))
        .collect::<HashMap<_, _>>()
}

async fn collect_mcp_server_status_snapshot_from_manager(
    mcp_connection_manager: &McpConnectionManager,
    auth_status_entries: HashMap<String, McpAuthStatusEntry>,
    detail: McpSnapshotDetail,
) -> McpServerStatusSnapshot {
    let (tools, resources, resource_templates) = tokio::join!(
        mcp_connection_manager.list_all_tools(),
        async {
            if detail.include_resources() {
                mcp_connection_manager.list_all_resources().await
            } else {
                HashMap::new()
            }
        },
        async {
            if detail.include_resources() {
                mcp_connection_manager.list_all_resource_templates().await
            } else {
                HashMap::new()
            }
        },
    );

    let mut tools_by_server = HashMap::<String, HashMap<String, Tool>>::new();
    for tool_info in tools {
        let raw_tool_name = tool_info.tool.name.to_string();
        let Some(tool) = protocol_tool_from_mcp_tool(&raw_tool_name, &tool_info.tool) else {
            continue;
        };
        let tool_name = tool.name.clone();
        tools_by_server
            .entry(tool_info.server_name)
            .or_default()
            .insert(tool_name, tool);
    }

    McpServerStatusSnapshot {
        tools_by_server,
        resources,
        resource_templates,
        auth_statuses: auth_statuses_from_entries(&auth_status_entries),
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
