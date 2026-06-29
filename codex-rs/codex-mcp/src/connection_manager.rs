//! Aggregates MCP server connections for Codex.
//!
//! [`McpConnectionManager`] owns the set of running async RMCP clients keyed by
//! MCP server name. It coordinates startup status events, keeps server origin
//! metadata, aggregates tools/resources/templates across servers, routes tool
//! calls to the right client, and exposes the public manager API used by
//! `codex-core`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use crate::McpAuthStatusEntry;
use crate::codex_apps::CodexAppsToolsCacheContext;
use crate::codex_apps::write_cached_codex_apps_tools_if_needed;
use crate::elicitation::ElicitationRequestManager;
use crate::mcp::CODEX_APPS_MCP_SERVER_NAME;
use crate::rmcp_client::AsyncManagedClient;
use crate::rmcp_client::DEFAULT_STARTUP_TIMEOUT;
use crate::rmcp_client::MCP_TOOLS_FETCH_UNCACHED_DURATION_METRIC;
use crate::rmcp_client::MCP_TOOLS_LIST_DURATION_METRIC;
use crate::rmcp_client::ManagedClient;
use crate::rmcp_client::StartupOutcomeError;
use crate::rmcp_client::list_tools_for_client_uncached;
use crate::runtime::emit_duration;
use crate::server::McpServerMetadata;
use crate::tools::filter_tools;
use crate::tools::normalize_tools_for_model;
use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use async_channel::Sender;
use codex_config_types::Constrained;
use codex_config_types::McpServerTransportConfig;
use codex_config_types::OAuthCredentialsStoreMode;
use codex_mcp_tool_types::ToolInfo;
use codex_mcp_tool_types::tool_with_model_visible_input_schema;
use codex_mcp_types::CodexAppsToolsCacheKey;
use codex_mcp_types::EffectiveMcpServer;
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::ElicitationReviewerHandle;
use codex_mcp_types::McpClientElicitationSupport;
use codex_mcp_types::ToolPluginProvenance;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::ListResourceTemplatesResult;
use codex_protocol::mcp::ListResourcesResult;
use codex_protocol::mcp::PaginatedRequestParams;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::mcp::ReadResourceResult;
use codex_protocol::mcp::Resource;
use codex_protocol::mcp::ResourceContent;
use codex_protocol::mcp::ResourceTemplate;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::McpStartupCompleteEvent;
use codex_protocol::protocol::McpStartupFailure;
use codex_protocol::protocol::McpStartupStatus;
use codex_protocol::protocol::McpStartupUpdateEvent;
use mcp_service_api::McpConnectionRuntime;
use mcp_service_api::McpConnectionRuntimeFactory;
use mcp_service_api::McpConnectionRuntimeStart;
use mcp_service_api::McpConnectionRuntimeStartRequest;
use mcp_service_api::McpRuntimeEnvironment;
use mcp_service_api::McpRuntimeFuture;
use mcp_service_api::McpToolRuntime;
use mcp_service_api::SharedMcpAuthHeaderProvider;
use rmcp::model::ElicitationCapability;
use rmcp::model::FormElicitationCapability;
use rmcp::model::ListResourceTemplatesResult as RmcpListResourceTemplatesResult;
use rmcp::model::ListResourcesResult as RmcpListResourcesResult;
use rmcp::model::PaginatedRequestParams as RmcpPaginatedRequestParams;
use rmcp::model::ReadResourceRequestParams as RmcpReadResourceRequestParams;
use rmcp::model::ReadResourceResult as RmcpReadResourceResult;
use rmcp::model::Resource as RmcpResource;
use rmcp::model::ResourceTemplate as RmcpResourceTemplate;
use rmcp::model::UrlElicitationCapability;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::instrument;
use tracing::warn;

/// A thin wrapper around a set of running [`RmcpClient`] instances.
pub struct McpConnectionManager {
    clients: HashMap<String, AsyncManagedClient>,
    server_metadata: HashMap<String, McpServerMetadata>,
    host_owned_codex_apps_enabled: bool,
    elicitation_requests: ElicitationRequestManager,
    startup_cancellation_token: CancellationToken,
}

#[derive(Debug, Default)]
pub struct DefaultMcpConnectionRuntimeFactory;

impl McpConnectionRuntimeFactory for DefaultMcpConnectionRuntimeFactory {
    fn uninitialized(
        &self,
        approval_policy: &Constrained<AskForApproval>,
        permission_profile: PermissionProfile,
    ) -> Box<dyn McpConnectionRuntime> {
        Box::new(
            McpConnectionManager::new_uninitialized_with_permission_profile(
                approval_policy,
                &permission_profile,
            ),
        )
    }

    fn start(
        &self,
        request: McpConnectionRuntimeStartRequest,
    ) -> McpRuntimeFuture<'_, McpConnectionRuntimeStart> {
        Box::pin(async move {
            let McpConnectionRuntimeStartRequest {
                mcp_servers,
                store_mode,
                auth_entries,
                approval_policy,
                submit_id,
                tx_event,
                initial_permission_profile,
                runtime_environment,
                codex_home,
                codex_apps_tools_cache_key,
                host_owned_codex_apps_enabled,
                client_elicitation_support,
                tool_plugin_provenance,
                codex_apps_auth_provider,
                elicitation_reviewer,
            } = request;
            let (manager, startup_cancellation_token) = McpConnectionManager::new(
                &mcp_servers,
                store_mode,
                auth_entries,
                &approval_policy,
                submit_id,
                tx_event,
                initial_permission_profile,
                runtime_environment,
                codex_home,
                codex_apps_tools_cache_key,
                host_owned_codex_apps_enabled,
                client_elicitation_support,
                tool_plugin_provenance,
                codex_apps_auth_provider,
                elicitation_reviewer,
            )
            .await;
            McpConnectionRuntimeStart {
                runtime: Box::new(manager),
                startup_cancellation_token,
            }
        })
    }
}

impl McpConnectionManager {
    pub fn new_uninitialized(
        approval_policy: &Constrained<AskForApproval>,
        permission_profile: &Constrained<PermissionProfile>,
    ) -> Self {
        Self::new_uninitialized_with_permission_profile(approval_policy, permission_profile.get())
    }

    pub fn new_uninitialized_with_permission_profile(
        approval_policy: &Constrained<AskForApproval>,
        permission_profile: &PermissionProfile,
    ) -> Self {
        Self {
            clients: HashMap::new(),
            server_metadata: HashMap::new(),
            host_owned_codex_apps_enabled: false,
            elicitation_requests: ElicitationRequestManager::new(
                approval_policy.value(),
                permission_profile.clone(),
                /*reviewer*/ None,
            ),
            startup_cancellation_token: CancellationToken::new(),
        }
    }

    pub fn has_servers(&self) -> bool {
        !self.clients.is_empty()
    }

    /// Drain all MCP clients from this manager and return a future that stops
    /// them and terminates their stdio server processes.
    pub fn begin_shutdown(&mut self) -> impl std::future::Future<Output = ()> + Send + 'static {
        self.startup_cancellation_token.cancel();
        let clients = std::mem::take(&mut self.clients);
        self.server_metadata.clear();
        async move {
            for client in clients.into_values() {
                client.shutdown().await;
            }
        }
    }

    /// Stop all MCP clients owned by this manager and terminate stdio server processes.
    pub async fn shutdown(&mut self) {
        self.begin_shutdown().await;
    }

    pub fn server_origin(&self, server_name: &str) -> Option<&str> {
        self.server_metadata
            .get(server_name)
            .and_then(|metadata| metadata.origin.as_ref())
            .map(super::server::McpServerOrigin::as_str)
    }

    pub fn server_pollutes_memory(&self, server_name: &str) -> bool {
        self.server_metadata
            .get(server_name)
            .is_none_or(|metadata| metadata.pollutes_memory)
    }

    pub fn is_host_owned_codex_apps_server(&self, server_name: &str) -> bool {
        self.host_owned_codex_apps_enabled && server_name == CODEX_APPS_MCP_SERVER_NAME
    }

    pub fn set_approval_policy(&self, approval_policy: &Constrained<AskForApproval>) {
        if let Ok(mut policy) = self.elicitation_requests.approval_policy.lock() {
            *policy = approval_policy.value();
        }
    }

    pub fn set_permission_profile(&self, permission_profile: PermissionProfile) {
        if let Ok(mut profile) = self.elicitation_requests.permission_profile.lock() {
            *profile = permission_profile;
        }
    }

    pub fn elicitations_auto_deny(&self) -> bool {
        self.elicitation_requests.auto_deny()
    }

    pub fn set_elicitations_auto_deny(&self, auto_deny: bool) {
        self.elicitation_requests.set_auto_deny(auto_deny);
    }

    #[allow(clippy::new_ret_no_self, clippy::too_many_arguments)]
    pub async fn new(
        mcp_servers: &HashMap<String, EffectiveMcpServer>,
        store_mode: OAuthCredentialsStoreMode,
        auth_entries: HashMap<String, McpAuthStatusEntry>,
        approval_policy: &Constrained<AskForApproval>,
        submit_id: String,
        tx_event: Sender<Event>,
        initial_permission_profile: PermissionProfile,
        runtime_environment: McpRuntimeEnvironment,
        codex_home: PathBuf,
        codex_apps_tools_cache_key: CodexAppsToolsCacheKey,
        host_owned_codex_apps_enabled: bool,
        client_elicitation_support: McpClientElicitationSupport,
        tool_plugin_provenance: ToolPluginProvenance,
        codex_apps_auth_provider: Option<SharedMcpAuthHeaderProvider>,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> (Self, CancellationToken) {
        let cancel_token = CancellationToken::new();
        let mut clients = HashMap::new();
        let mut server_metadata = HashMap::new();
        let mut join_set = JoinSet::new();
        let elicitation_requests = ElicitationRequestManager::new(
            approval_policy.value(),
            initial_permission_profile,
            elicitation_reviewer,
        );
        let tool_plugin_provenance = Arc::new(tool_plugin_provenance);
        let client_elicitation_capability =
            client_elicitation_capability(client_elicitation_support);
        let startup_submit_id = submit_id.clone();
        let mcp_servers = mcp_servers.clone();
        for (server_name, server) in mcp_servers
            .into_iter()
            .filter(|(_, server)| server.enabled())
        {
            server_metadata.insert(server_name.clone(), McpServerMetadata::from(&server));
            let cancel_token = cancel_token.child_token();
            let _ = emit_update(
                startup_submit_id.as_str(),
                &tx_event,
                McpStartupUpdateEvent {
                    server: server_name.clone(),
                    status: McpStartupStatus::Starting,
                },
            )
            .await;
            let codex_apps_tools_cache_context = if server_name == CODEX_APPS_MCP_SERVER_NAME {
                Some(CodexAppsToolsCacheContext {
                    codex_home: codex_home.clone(),
                    user_key: codex_apps_tools_cache_key.clone(),
                })
            } else {
                None
            };
            let uses_env_bearer_token =
                server
                    .configured_config()
                    .is_some_and(|config| match &config.transport {
                        McpServerTransportConfig::StreamableHttp {
                            bearer_token_env_var,
                            ..
                        } => bearer_token_env_var.is_some(),
                        McpServerTransportConfig::Stdio { .. } => false,
                    });
            let runtime_auth_provider =
                if server_name == CODEX_APPS_MCP_SERVER_NAME && !uses_env_bearer_token {
                    codex_apps_auth_provider.clone()
                } else {
                    None
                };
            let async_managed_client = AsyncManagedClient::new(
                server_name.clone(),
                server,
                store_mode,
                cancel_token.clone(),
                tx_event.clone(),
                elicitation_requests.clone(),
                codex_apps_tools_cache_context,
                Arc::clone(&tool_plugin_provenance),
                runtime_environment.clone(),
                runtime_auth_provider,
                client_elicitation_capability.clone(),
            );
            clients.insert(server_name.clone(), async_managed_client.clone());
            let tx_event = tx_event.clone();
            let submit_id = startup_submit_id.clone();
            let auth_entry = auth_entries.get(&server_name).cloned();
            join_set.spawn(async move {
                let mut outcome = async_managed_client.client().await;
                if cancel_token.is_cancelled() {
                    outcome = Err(StartupOutcomeError::Cancelled);
                }
                let status = match &outcome {
                    Ok(_) => McpStartupStatus::Ready,
                    Err(StartupOutcomeError::Cancelled) => McpStartupStatus::Cancelled,
                    Err(error) => {
                        let error_str = mcp_init_error_display(
                            server_name.as_str(),
                            auth_entry.as_ref(),
                            error,
                        );
                        McpStartupStatus::Failed { error: error_str }
                    }
                };

                let _ = emit_update(
                    submit_id.as_str(),
                    &tx_event,
                    McpStartupUpdateEvent {
                        server: server_name.clone(),
                        status,
                    },
                )
                .await;

                (server_name, outcome)
            });
        }
        let manager = Self {
            clients,
            server_metadata,
            host_owned_codex_apps_enabled,
            elicitation_requests: elicitation_requests.clone(),
            startup_cancellation_token: cancel_token.clone(),
        };
        tokio::spawn(async move {
            let outcomes = join_set.join_all().await;
            let mut summary = McpStartupCompleteEvent::default();
            for (server_name, outcome) in outcomes {
                match outcome {
                    Ok(_) => summary.ready.push(server_name),
                    Err(StartupOutcomeError::Cancelled) => summary.cancelled.push(server_name),
                    Err(StartupOutcomeError::Failed { error }) => {
                        summary.failed.push(McpStartupFailure {
                            server: server_name,
                            error,
                        })
                    }
                }
            }
            let _ = tx_event
                .send(Event {
                    id: startup_submit_id,
                    msg: EventMsg::McpStartupComplete(summary),
                })
                .await;
        });
        (manager, cancel_token)
    }

    pub async fn resolve_elicitation(
        &self,
        server_name: String,
        id: codex_protocol::mcp::RequestId,
        response: ElicitationResponse,
    ) -> Result<()> {
        self.elicitation_requests
            .resolve(server_name, id, response)
            .await
    }

    pub async fn wait_for_server_ready(&self, server_name: &str, timeout: Duration) -> bool {
        let Some(async_managed_client) = self.clients.get(server_name) else {
            return false;
        };

        match tokio::time::timeout(timeout, async_managed_client.client()).await {
            Ok(Ok(_)) => true,
            Ok(Err(_)) | Err(_) => false,
        }
    }

    pub async fn required_startup_failures(
        &self,
        required_servers: &[String],
    ) -> Vec<McpStartupFailure> {
        let mut failures = Vec::new();
        for server_name in required_servers {
            let Some(async_managed_client) = self.clients.get(server_name).cloned() else {
                failures.push(McpStartupFailure {
                    server: server_name.clone(),
                    error: format!("required MCP server `{server_name}` was not initialized"),
                });
                continue;
            };

            match async_managed_client.client().await {
                Ok(_) => {}
                Err(error) => failures.push(McpStartupFailure {
                    server: server_name.clone(),
                    error: startup_outcome_error_message(error),
                }),
            }
        }
        failures
    }

    /// Returns all tools with model-visible names normalized.
    #[instrument(level = "trace", skip_all)]
    pub async fn list_all_tools(&self) -> Vec<ToolInfo> {
        let mut tools = Vec::new();
        for managed_client in self.clients.values() {
            let Some(server_tools) = managed_client.listed_tools().await else {
                continue;
            };
            tools.extend(
                server_tools
                    .into_iter()
                    .map(|tool| self.with_server_metadata(tool)),
            );
        }
        normalize_tools_for_model(tools)
    }

    /// Force-refresh codex apps tools by bypassing the in-process cache.
    ///
    /// On success, the refreshed tools replace the cache contents and the
    /// latest filtered tools are returned directly to the caller. On
    /// failure, the existing cache remains unchanged.
    pub async fn hard_refresh_codex_apps_tools_cache(&self) -> Result<Vec<ToolInfo>> {
        let managed_client = self
            .clients
            .get(CODEX_APPS_MCP_SERVER_NAME)
            .ok_or_else(|| anyhow!("unknown MCP server '{CODEX_APPS_MCP_SERVER_NAME}'"))?
            .client()
            .await
            .context("failed to get client")?;

        let list_start = Instant::now();
        let fetch_start = Instant::now();
        let tools = list_tools_for_client_uncached(
            CODEX_APPS_MCP_SERVER_NAME,
            &managed_client.client,
            managed_client.tool_timeout,
            managed_client.server_instructions.as_deref(),
        )
        .await
        .with_context(|| {
            format!("failed to refresh tools for MCP server '{CODEX_APPS_MCP_SERVER_NAME}'")
        })?;
        emit_duration(
            MCP_TOOLS_FETCH_UNCACHED_DURATION_METRIC,
            fetch_start.elapsed(),
            &[],
        );

        write_cached_codex_apps_tools_if_needed(
            CODEX_APPS_MCP_SERVER_NAME,
            managed_client.codex_apps_tools_cache_context.as_ref(),
            &tools,
        );
        emit_duration(
            MCP_TOOLS_LIST_DURATION_METRIC,
            list_start.elapsed(),
            &[("cache", "miss")],
        );
        let tools = filter_tools(tools, &managed_client.tool_filter)
            .into_iter()
            .map(|mut tool| {
                tool.tool = tool_with_model_visible_input_schema(&tool.tool);
                self.with_server_metadata(tool)
            });
        Ok(normalize_tools_for_model(tools))
    }

    fn with_server_metadata(&self, mut tool: ToolInfo) -> ToolInfo {
        let Some(metadata) = self.server_metadata.get(&tool.server_name) else {
            tool.supports_parallel_tool_calls = false;
            tool.server_origin = None;
            return tool;
        };

        tool.supports_parallel_tool_calls = metadata.supports_parallel_tool_calls;
        tool.server_origin = metadata
            .origin
            .as_ref()
            .map(|origin| origin.as_str().to_string());
        tool
    }

    /// Returns a single map that contains all resources. Each key is the
    /// server name and the value is a vector of resources.
    pub async fn list_all_resources(&self) -> HashMap<String, Vec<Resource>> {
        let mut join_set = JoinSet::new();

        let clients_snapshot = &self.clients;

        for (server_name, async_managed_client) in clients_snapshot {
            let server_name = server_name.clone();
            let Ok(managed_client) = async_managed_client.client().await else {
                continue;
            };
            let timeout = managed_client.tool_timeout;
            let client = managed_client.client.clone();

            join_set.spawn(async move {
                let mut collected: Vec<Resource> = Vec::new();
                let mut cursor: Option<String> = None;

                loop {
                    let params = cursor.as_ref().map(|next| RmcpPaginatedRequestParams {
                        meta: None,
                        cursor: Some(next.clone()),
                    });
                    let response = match client.list_resources(params, timeout).await {
                        Ok(result) => result,
                        Err(err) => return (server_name, Err(err)),
                    };

                    let resources = match resources_from_rmcp(response.resources) {
                        Ok(resources) => resources,
                        Err(err) => return (server_name, Err(err)),
                    };
                    collected.extend(resources);

                    match response.next_cursor {
                        Some(next) => {
                            if cursor.as_ref() == Some(&next) {
                                return (
                                    server_name,
                                    Err(anyhow!("resources/list returned duplicate cursor")),
                                );
                            }
                            cursor = Some(next);
                        }
                        None => return (server_name, Ok(collected)),
                    }
                }
            });
        }

        let mut aggregated: HashMap<String, Vec<Resource>> = HashMap::new();

        while let Some(join_res) = join_set.join_next().await {
            match join_res {
                Ok((server_name, Ok(resources))) => {
                    aggregated.insert(server_name, resources);
                }
                Ok((server_name, Err(err))) => {
                    warn!("Failed to list resources for MCP server '{server_name}': {err:#}");
                }
                Err(err) => {
                    warn!("Task panic when listing resources for MCP server: {err:#}");
                }
            }
        }

        aggregated
    }

    /// Returns a single map that contains all resource templates. Each key is the
    /// server name and the value is a vector of resource templates.
    pub async fn list_all_resource_templates(&self) -> HashMap<String, Vec<ResourceTemplate>> {
        let mut join_set = JoinSet::new();

        let clients_snapshot = &self.clients;

        for (server_name, async_managed_client) in clients_snapshot {
            let server_name_cloned = server_name.clone();
            let Ok(managed_client) = async_managed_client.client().await else {
                continue;
            };
            let client = managed_client.client.clone();
            let timeout = managed_client.tool_timeout;

            join_set.spawn(async move {
                let mut collected: Vec<ResourceTemplate> = Vec::new();
                let mut cursor: Option<String> = None;

                loop {
                    let params = cursor.as_ref().map(|next| RmcpPaginatedRequestParams {
                        meta: None,
                        cursor: Some(next.clone()),
                    });
                    let response = match client.list_resource_templates(params, timeout).await {
                        Ok(result) => result,
                        Err(err) => return (server_name_cloned, Err(err)),
                    };

                    let templates = match resource_templates_from_rmcp(response.resource_templates)
                    {
                        Ok(templates) => templates,
                        Err(err) => return (server_name_cloned, Err(err)),
                    };
                    collected.extend(templates);

                    match response.next_cursor {
                        Some(next) => {
                            if cursor.as_ref() == Some(&next) {
                                return (
                                    server_name_cloned,
                                    Err(anyhow!(
                                        "resources/templates/list returned duplicate cursor"
                                    )),
                                );
                            }
                            cursor = Some(next);
                        }
                        None => return (server_name_cloned, Ok(collected)),
                    }
                }
            });
        }

        let mut aggregated: HashMap<String, Vec<ResourceTemplate>> = HashMap::new();

        while let Some(join_res) = join_set.join_next().await {
            match join_res {
                Ok((server_name, Ok(templates))) => {
                    aggregated.insert(server_name, templates);
                }
                Ok((server_name, Err(err))) => {
                    warn!(
                        "Failed to list resource templates for MCP server '{server_name}': {err:#}"
                    );
                }
                Err(err) => {
                    warn!("Task panic when listing resource templates for MCP server: {err:#}");
                }
            }
        }

        aggregated
    }

    /// Invoke the tool indicated by the (server, tool) pair.
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> Result<CallToolResult> {
        let client = self.client_by_name(server).await?;
        if !client.tool_filter.allows(tool) {
            return Err(anyhow!(
                "tool '{tool}' is disabled for MCP server '{server}'"
            ));
        }

        let result: rmcp::model::CallToolResult = client
            .client
            .call_tool(tool.to_string(), arguments, meta, client.tool_timeout)
            .await
            .with_context(|| format!("tool call failed for `{server}/{tool}`"))?;

        let content = result
            .content
            .into_iter()
            .map(|content| {
                serde_json::to_value(content)
                    .unwrap_or_else(|_| serde_json::Value::String("<content>".to_string()))
            })
            .collect();

        Ok(CallToolResult {
            content,
            structured_content: result.structured_content,
            is_error: result.is_error,
            meta: result.meta.and_then(|meta| serde_json::to_value(meta).ok()),
        })
    }

    pub async fn server_supports_sandbox_state_meta_capability(
        &self,
        server: &str,
    ) -> Result<bool> {
        Ok(self
            .client_by_name(server)
            .await?
            .server_supports_sandbox_state_meta_capability)
    }

    /// List resources from the specified server.
    pub async fn list_resources(
        &self,
        server: &str,
        params: Option<PaginatedRequestParams>,
    ) -> Result<ListResourcesResult> {
        let managed = self.client_by_name(server).await?;
        let timeout = managed.tool_timeout;

        let result = managed
            .client
            .list_resources(rmcp_paginated_params(params), timeout)
            .await
            .with_context(|| format!("resources/list failed for `{server}`"))?;
        list_resources_result_from_rmcp(result)
    }

    /// List resource templates from the specified server.
    pub async fn list_resource_templates(
        &self,
        server: &str,
        params: Option<PaginatedRequestParams>,
    ) -> Result<ListResourceTemplatesResult> {
        let managed = self.client_by_name(server).await?;
        let client = managed.client.clone();
        let timeout = managed.tool_timeout;

        client
            .list_resource_templates(rmcp_paginated_params(params), timeout)
            .await
            .with_context(|| format!("resources/templates/list failed for `{server}`"))
            .and_then(list_resource_templates_result_from_rmcp)
    }

    /// Read a resource from the specified server.
    pub async fn read_resource(
        &self,
        server: &str,
        params: ReadResourceRequestParams,
    ) -> Result<ReadResourceResult> {
        let managed = self.client_by_name(server).await?;
        let client = managed.client.clone();
        let timeout = managed.tool_timeout;
        let uri = params.uri.clone();

        let result = client
            .read_resource(
                RmcpReadResourceRequestParams {
                    meta: None,
                    uri: uri.clone(),
                },
                timeout,
            )
            .await
            .with_context(|| format!("resources/read failed for `{server}` ({uri})"))?;
        read_resource_result_from_rmcp(result)
    }

    async fn client_by_name(&self, name: &str) -> Result<ManagedClient> {
        self.clients
            .get(name)
            .ok_or_else(|| anyhow!("unknown MCP server '{name}'"))?
            .client()
            .await
            .context("failed to get client")
    }
}

impl McpToolRuntime for McpConnectionManager {
    fn call_tool<'a>(
        &'a self,
        server: &'a str,
        tool: &'a str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> McpRuntimeFuture<'a, anyhow::Result<CallToolResult>> {
        Box::pin(async move {
            McpConnectionManager::call_tool(self, server, tool, arguments, meta).await
        })
    }

    fn server_origin(&self, server_name: &str) -> Option<String> {
        McpConnectionManager::server_origin(self, server_name).map(str::to_string)
    }

    fn server_pollutes_memory(&self, server_name: &str) -> bool {
        McpConnectionManager::server_pollutes_memory(self, server_name)
    }

    fn is_host_owned_codex_apps_server(&self, server_name: &str) -> bool {
        McpConnectionManager::is_host_owned_codex_apps_server(self, server_name)
    }

    fn server_supports_sandbox_state_meta_capability<'a>(
        &'a self,
        server_name: &'a str,
    ) -> McpRuntimeFuture<'a, anyhow::Result<bool>> {
        Box::pin(async move {
            McpConnectionManager::server_supports_sandbox_state_meta_capability(self, server_name)
                .await
        })
    }
}

impl McpConnectionRuntime for McpConnectionManager {
    fn has_servers(&self) -> bool {
        McpConnectionManager::has_servers(self)
    }

    fn set_approval_policy(&self, approval_policy: &Constrained<AskForApproval>) {
        McpConnectionManager::set_approval_policy(self, approval_policy);
    }

    fn set_permission_profile(&self, permission_profile: PermissionProfile) {
        McpConnectionManager::set_permission_profile(self, permission_profile);
    }

    fn elicitations_auto_deny(&self) -> bool {
        McpConnectionManager::elicitations_auto_deny(self)
    }

    fn set_elicitations_auto_deny(&self, auto_deny: bool) {
        McpConnectionManager::set_elicitations_auto_deny(self, auto_deny);
    }

    fn resolve_elicitation<'a>(
        &'a self,
        server_name: String,
        id: codex_protocol::mcp::RequestId,
        response: ElicitationResponse,
    ) -> McpRuntimeFuture<'a, Result<()>> {
        Box::pin(async move {
            McpConnectionManager::resolve_elicitation(self, server_name, id, response).await
        })
    }

    fn list_all_tools<'a>(&'a self) -> McpRuntimeFuture<'a, Vec<ToolInfo>> {
        Box::pin(async move { McpConnectionManager::list_all_tools(self).await })
    }

    fn hard_refresh_codex_apps_tools_cache<'a>(
        &'a self,
    ) -> McpRuntimeFuture<'a, Result<Vec<ToolInfo>>> {
        Box::pin(
            async move { McpConnectionManager::hard_refresh_codex_apps_tools_cache(self).await },
        )
    }

    fn wait_for_server_ready<'a>(
        &'a self,
        server_name: &'a str,
        timeout: Duration,
    ) -> McpRuntimeFuture<'a, bool> {
        Box::pin(async move {
            McpConnectionManager::wait_for_server_ready(self, server_name, timeout).await
        })
    }

    fn required_startup_failures<'a>(
        &'a self,
        required_servers: &'a [String],
    ) -> McpRuntimeFuture<'a, Vec<McpStartupFailure>> {
        Box::pin(async move {
            McpConnectionManager::required_startup_failures(self, required_servers).await
        })
    }

    fn list_all_resources<'a>(&'a self) -> McpRuntimeFuture<'a, HashMap<String, Vec<Resource>>> {
        Box::pin(async move { McpConnectionManager::list_all_resources(self).await })
    }

    fn list_all_resource_templates<'a>(
        &'a self,
    ) -> McpRuntimeFuture<'a, HashMap<String, Vec<ResourceTemplate>>> {
        Box::pin(async move { McpConnectionManager::list_all_resource_templates(self).await })
    }

    fn list_resources<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, Result<ListResourcesResult>> {
        Box::pin(async move { McpConnectionManager::list_resources(self, server, params).await })
    }

    fn list_resource_templates<'a>(
        &'a self,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, Result<ListResourceTemplatesResult>> {
        Box::pin(async move {
            McpConnectionManager::list_resource_templates(self, server, params).await
        })
    }

    fn read_resource<'a>(
        &'a self,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> McpRuntimeFuture<'a, Result<ReadResourceResult>> {
        Box::pin(async move { McpConnectionManager::read_resource(self, server, params).await })
    }

    fn shutdown<'a>(&'a mut self) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move { McpConnectionManager::shutdown(self).await })
    }
}

fn rmcp_paginated_params(
    params: Option<PaginatedRequestParams>,
) -> Option<RmcpPaginatedRequestParams> {
    params.map(|params| RmcpPaginatedRequestParams {
        meta: None,
        cursor: params.cursor,
    })
}

fn list_resources_result_from_rmcp(result: RmcpListResourcesResult) -> Result<ListResourcesResult> {
    Ok(ListResourcesResult {
        resources: resources_from_rmcp(result.resources)?,
        next_cursor: result.next_cursor,
    })
}

fn list_resource_templates_result_from_rmcp(
    result: RmcpListResourceTemplatesResult,
) -> Result<ListResourceTemplatesResult> {
    Ok(ListResourceTemplatesResult {
        resource_templates: resource_templates_from_rmcp(result.resource_templates)?,
        next_cursor: result.next_cursor,
    })
}

fn read_resource_result_from_rmcp(result: RmcpReadResourceResult) -> Result<ReadResourceResult> {
    let contents = result
        .contents
        .into_iter()
        .map(resource_content_from_rmcp)
        .collect::<Result<Vec<_>>>()?;
    Ok(ReadResourceResult { contents })
}

fn resources_from_rmcp(resources: Vec<RmcpResource>) -> Result<Vec<Resource>> {
    resources
        .into_iter()
        .map(resource_from_rmcp)
        .collect::<Result<Vec<_>>>()
}

fn resource_templates_from_rmcp(
    templates: Vec<RmcpResourceTemplate>,
) -> Result<Vec<ResourceTemplate>> {
    templates
        .into_iter()
        .map(resource_template_from_rmcp)
        .collect::<Result<Vec<_>>>()
}

fn resource_from_rmcp(resource: RmcpResource) -> Result<Resource> {
    Resource::from_mcp_value(serde_json::to_value(resource)?).context("failed to convert resource")
}

fn resource_template_from_rmcp(template: RmcpResourceTemplate) -> Result<ResourceTemplate> {
    ResourceTemplate::from_mcp_value(serde_json::to_value(template)?)
        .context("failed to convert resource template")
}

fn resource_content_from_rmcp(content: rmcp::model::ResourceContents) -> Result<ResourceContent> {
    ResourceContent::from_mcp_value(serde_json::to_value(content)?)
        .context("failed to convert resource content")
}

fn client_elicitation_capability(support: McpClientElicitationSupport) -> ElicitationCapability {
    match support {
        McpClientElicitationSupport::Disabled => ElicitationCapability::default(),
        McpClientElicitationSupport::AuthElicitation => ElicitationCapability {
            form: Some(FormElicitationCapability::default()),
            url: Some(UrlElicitationCapability::default()),
        },
    }
}

impl Drop for McpConnectionManager {
    fn drop(&mut self) {
        self.startup_cancellation_token.cancel();
        self.clients.clear();
    }
}

async fn emit_update(
    submit_id: &str,
    tx_event: &Sender<Event>,
    update: McpStartupUpdateEvent,
) -> Result<(), async_channel::SendError<Event>> {
    tx_event
        .send(Event {
            id: submit_id.to_string(),
            msg: EventMsg::McpStartupUpdate(update),
        })
        .await
}

fn mcp_init_error_display(
    server_name: &str,
    entry: Option<&McpAuthStatusEntry>,
    err: &StartupOutcomeError,
) -> String {
    if let Some(McpServerTransportConfig::StreamableHttp {
        url,
        bearer_token_env_var,
        http_headers,
        ..
    }) = entry.and_then(|entry| entry.config.as_ref().map(|config| &config.transport))
        && url == "https://api.githubcopilot.com/mcp/"
        && bearer_token_env_var.is_none()
        && http_headers.as_ref().map(HashMap::is_empty).unwrap_or(true)
    {
        format!(
            "GitHub MCP does not support OAuth. Log in by adding a personal access token (https://github.com/settings/personal-access-tokens) to your environment and config.toml:\n[mcp_servers.{server_name}]\nbearer_token_env_var = CODEX_GITHUB_PERSONAL_ACCESS_TOKEN"
        )
    } else if is_mcp_client_auth_required_error(err) {
        format!(
            "The {server_name} MCP server is not logged in. Run `codex mcp login {server_name}`."
        )
    } else if is_mcp_client_startup_timeout_error(err) {
        let startup_timeout_secs = match entry {
            Some(entry) => match entry
                .config
                .as_ref()
                .and_then(|config| config.startup_timeout_sec)
            {
                Some(timeout) => timeout,
                None => DEFAULT_STARTUP_TIMEOUT,
            },
            None => DEFAULT_STARTUP_TIMEOUT,
        }
        .as_secs();
        format!(
            "MCP client for `{server_name}` timed out after {startup_timeout_secs} seconds. Add or adjust `startup_timeout_sec` in your config.toml:\n[mcp_servers.{server_name}]\nstartup_timeout_sec = XX"
        )
    } else {
        format!("MCP client for `{server_name}` failed to start: {err:#}")
    }
}

fn startup_outcome_error_message(error: StartupOutcomeError) -> String {
    match error {
        StartupOutcomeError::Cancelled => "MCP startup cancelled".to_string(),
        StartupOutcomeError::Failed { error } => error,
    }
}

fn is_mcp_client_auth_required_error(error: &StartupOutcomeError) -> bool {
    match error {
        StartupOutcomeError::Failed { error } => error.contains("Auth required"),
        _ => false,
    }
}

fn is_mcp_client_startup_timeout_error(error: &StartupOutcomeError) -> bool {
    match error {
        StartupOutcomeError::Failed { error } => {
            error.contains("request timed out")
                || error.contains("timed out handshaking with MCP server")
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "connection_manager_tests.rs"]
mod tests;
