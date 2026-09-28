use super::*;

use crate::live_thread_runtime::AppServerLiveThreadInspectionRuntime;
use codex_config_types::McpServerConfig;
use futures::future::BoxFuture;
use mcp_service_api::SharedMcpAuthHeaderProvider;
use mcp_service_api::StaticMcpAuthHeaderProvider;
use protocol::mcp::CallToolResult;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::io;

const MCP_TOOL_THREAD_ID_META_KEY: &str = "threadId";

fn mcp_runtime_environment(
    environment: Arc<codex_exec_server::Environment>,
    fallback_cwd: std::path::PathBuf,
) -> McpRuntimeEnvironment {
    let local_http_client: Arc<dyn exec_server_api::HttpClient> =
        Arc::new(codex_exec_server::ReqwestHttpClient);
    McpRuntimeEnvironment::new(mcp_service_api::McpRuntimeEnvironmentParams {
        remote_available: environment.is_remote(),
        remote_exec_backend: environment.get_exec_backend(),
        local_http_client,
        remote_http_client: environment.get_http_client(),
        fallback_cwd,
    })
}

fn codex_apps_auth_context(
    auth: Option<&codex_auth_types::RequestAuthSnapshot>,
) -> Option<mcp_types::CodexAppsAuthContext> {
    auth.map(|auth| mcp_types::CodexAppsAuthContext {
        uses_codex_backend: auth.uses_codex_backend(),
        account_id: auth.account_id().map(ToOwned::to_owned),
        chatgpt_user_id: auth.chatgpt_user_id().map(ToOwned::to_owned),
        is_workspace_account: auth.is_workspace_account(),
    })
}

fn codex_apps_auth_provider(auth: Option<&CodexAuth>) -> Option<SharedMcpAuthHeaderProvider> {
    auth.filter(|auth| auth.uses_codex_backend())
        .map(model_service::auth_provider_from_auth)
        .map(|auth_provider| StaticMcpAuthHeaderProvider::shared(auth_provider.to_auth_headers()))
}

pub(crate) trait McpProcessorRuntime: Send + Sync {
    fn queue_strict_mcp_refresh(
        self: Arc<Self>,
        config_manager: ConfigManager,
    ) -> BoxFuture<'static, io::Result<()>>;

    fn configured_mcp_servers<'a>(
        &'a self,
        config: &'a Config,
    ) -> BoxFuture<'a, HashMap<String, McpServerConfig>>;

    fn mcp_config<'a>(&'a self, config: &'a Config) -> BoxFuture<'a, mcp_types::McpConfig>;

    fn is_thread_mcp_runtime_available(&self, thread_id: ThreadId) -> BoxFuture<'_, bool>;

    fn read_thread_mcp_resource<'a>(
        &'a self,
        thread_id: ThreadId,
        server: &'a str,
        uri: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<serde_json::Value>>;

    fn call_thread_mcp_tool<'a>(
        &'a self,
        thread_id: ThreadId,
        server: &'a str,
        tool: &'a str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> BoxFuture<'a, anyhow::Result<CallToolResult>>;
}

impl McpProcessorRuntime for ThreadService {
    fn queue_strict_mcp_refresh(
        self: Arc<Self>,
        config_manager: ConfigManager,
    ) -> BoxFuture<'static, io::Result<()>> {
        Box::pin(async move {
            crate::mcp_refresh::queue_strict_refresh(self.as_ref(), &config_manager).await
        })
    }

    fn configured_mcp_servers<'a>(
        &'a self,
        config: &'a Config,
    ) -> BoxFuture<'a, HashMap<String, McpServerConfig>> {
        Box::pin(async move {
            self.mcp_service()
                .configured_servers(self.plugin_runtime().as_ref(), config)
                .await
        })
    }

    fn mcp_config<'a>(&'a self, config: &'a Config) -> BoxFuture<'a, mcp_types::McpConfig> {
        Box::pin(async move { config.to_mcp_config(self.plugin_runtime().as_ref()).await })
    }

    fn is_thread_mcp_runtime_available(&self, thread_id: ThreadId) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.get_thread(thread_id).await.is_ok() })
    }

    fn read_thread_mcp_resource<'a>(
        &'a self,
        thread_id: ThreadId,
        server: &'a str,
        uri: &'a str,
    ) -> BoxFuture<'a, anyhow::Result<serde_json::Value>> {
        Box::pin(ThreadService::read_thread_mcp_resource(
            self, thread_id, server, uri,
        ))
    }

    fn call_thread_mcp_tool<'a>(
        &'a self,
        thread_id: ThreadId,
        server: &'a str,
        tool: &'a str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> BoxFuture<'a, anyhow::Result<CallToolResult>> {
        Box::pin(ThreadService::call_thread_mcp_tool(
            self, thread_id, server, tool, arguments, meta,
        ))
    }
}

#[derive(Clone)]
pub(crate) struct McpRequestProcessor {
    auth_manager: Arc<AuthManager>,
    runtime: Arc<dyn McpProcessorRuntime>,
    live_thread_inspection: Arc<dyn AppServerLiveThreadInspectionRuntime>,
    outgoing: Arc<OutgoingMessageSender>,
    config_manager: ConfigManager,
    environment_manager: Arc<EnvironmentManager>,
}

impl McpRequestProcessor {
    pub(crate) fn new(
        auth_manager: Arc<AuthManager>,
        runtime: Arc<impl McpProcessorRuntime + AppServerLiveThreadInspectionRuntime + 'static>,
        outgoing: Arc<OutgoingMessageSender>,
        config_manager: ConfigManager,
        environment_manager: Arc<EnvironmentManager>,
    ) -> Self {
        let live_thread_inspection: Arc<dyn AppServerLiveThreadInspectionRuntime> = runtime.clone();
        let runtime: Arc<dyn McpProcessorRuntime> = runtime;
        Self {
            auth_manager,
            runtime,
            live_thread_inspection,
            outgoing,
            config_manager,
            environment_manager,
        }
    }

    pub(crate) async fn mcp_server_oauth_login(
        &self,
        params: McpServerOauthLoginParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.mcp_server_oauth_login_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn mcp_server_refresh(
        &self,
        params: Option<()>,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.mcp_server_refresh_response(params)
            .await
            .map(|response| Some(response.into()))
    }

    pub(crate) async fn mcp_server_status_list(
        &self,
        request_id: &ConnectionRequestId,
        params: ListMcpServerStatusParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.list_mcp_server_status(request_id, params)
            .await
            .map(|()| None)
    }

    pub(crate) async fn mcp_resource_read(
        &self,
        request_id: &ConnectionRequestId,
        params: McpResourceReadParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.read_mcp_resource(request_id, params)
            .await
            .map(|()| None)
    }

    pub(crate) async fn mcp_server_tool_call(
        &self,
        request_id: &ConnectionRequestId,
        params: McpServerToolCallParams,
    ) -> Result<Option<ClientResponsePayload>, JSONRPCErrorError> {
        self.call_mcp_server_tool(request_id, params)
            .await
            .map(|()| None)
    }

    async fn mcp_server_refresh_response(
        &self,
        _params: Option<()>,
    ) -> Result<McpServerRefreshResponse, JSONRPCErrorError> {
        Arc::clone(&self.runtime)
            .queue_strict_mcp_refresh(self.config_manager.clone())
            .await
            .map_err(|err| internal_error(format!("failed to refresh MCP servers: {err}")))?;
        Ok(McpServerRefreshResponse {})
    }

    fn parse_thread_id(thread_id: &str) -> Result<ThreadId, JSONRPCErrorError> {
        ThreadId::from_string(thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))
    }

    async fn mcp_server_oauth_login_response(
        &self,
        params: McpServerOauthLoginParams,
    ) -> Result<McpServerOauthLoginResponse, JSONRPCErrorError> {
        let config =
            load_latest_config_for_request(&self.config_manager, /*fallback_cwd*/ None).await?;
        let McpServerOauthLoginParams {
            name,
            scopes,
            timeout_secs,
        } = params;

        let configured_servers = self.runtime.configured_mcp_servers(&config).await;
        let Some(server) = configured_servers.get(&name) else {
            return Err(invalid_request(format!(
                "No MCP server named '{name}' found."
            )));
        };

        let (url, http_headers, env_http_headers) = match &server.transport {
            McpServerTransportConfig::StreamableHttp {
                url,
                http_headers,
                env_http_headers,
                ..
            } => (url.clone(), http_headers.clone(), env_http_headers.clone()),
            _ => {
                return Err(invalid_request(
                    "OAuth login is only supported for streamable HTTP servers.",
                ));
            }
        };

        let discovered_scopes = if scopes.is_none() && server.scopes.is_none() {
            discover_supported_scopes(&server.transport).await
        } else {
            None
        };
        let resolved_scopes =
            resolve_oauth_scopes(scopes, server.scopes.clone(), discovered_scopes);

        let handle = perform_oauth_login_return_url(
            &name,
            &url,
            config.mcp_oauth_credentials_store_mode,
            http_headers,
            env_http_headers,
            &resolved_scopes.scopes,
            server.oauth_client_id(),
            server.oauth_resource.as_deref(),
            timeout_secs,
            config.mcp_oauth_callback_port,
            config.mcp_oauth_callback_url.as_deref(),
        )
        .await
        .map_err(|err| internal_error(format!("failed to login to MCP server '{name}': {err}")))?;
        let authorization_url = handle.authorization_url().to_string();
        let notification_name = name.clone();
        let outgoing = Arc::clone(&self.outgoing);

        tokio::spawn(async move {
            let (success, error) = match handle.wait().await {
                Ok(()) => (true, None),
                Err(err) => (false, Some(err.to_string())),
            };

            let notification = ServerNotification::McpServerOauthLoginCompleted(
                McpServerOauthLoginCompletedNotification {
                    name: notification_name,
                    success,
                    error,
                },
            );
            outgoing.send_server_notification(notification).await;
        });

        Ok(McpServerOauthLoginResponse { authorization_url })
    }

    async fn list_mcp_server_status(
        &self,
        request_id: &ConnectionRequestId,
        params: ListMcpServerStatusParams,
    ) -> Result<(), JSONRPCErrorError> {
        let request = request_id.clone();

        let outgoing = Arc::clone(&self.outgoing);
        let config =
            load_latest_config_for_request(&self.config_manager, /*fallback_cwd*/ None).await?;
        let mcp_config = self.runtime.mcp_config(&config).await;
        let auth = self.auth_manager.auth().await;
        let environment_manager = Arc::clone(&self.environment_manager);
        let runtime_environment = match environment_manager.default_environment() {
            Some(environment) => {
                // Status listing has no turn cwd. This fallback is used only
                // by executor-backed stdio MCPs whose config omits `cwd`.
                mcp_runtime_environment(environment, config.cwd.to_path_buf())
            }
            None => mcp_runtime_environment(
                environment_manager.local_environment(),
                config.cwd.to_path_buf(),
            ),
        };

        tokio::spawn(async move {
            Self::list_mcp_server_status_task(
                outgoing,
                request,
                params,
                config,
                mcp_config,
                auth,
                runtime_environment,
            )
            .await;
        });
        Ok(())
    }

    async fn list_mcp_server_status_task(
        outgoing: Arc<OutgoingMessageSender>,
        request_id: ConnectionRequestId,
        params: ListMcpServerStatusParams,
        config: Config,
        mcp_config: mcp_types::McpConfig,
        auth: Option<CodexAuth>,
        runtime_environment: McpRuntimeEnvironment,
    ) {
        let result = Self::list_mcp_server_status_response(
            McpServerStatusListRequest::from_protocol(request_id.request_id.to_string(), params),
            config,
            mcp_config,
            auth,
            runtime_environment,
        )
        .await;
        outgoing.send_result(request_id, result).await;
    }

    async fn list_mcp_server_status_response(
        request: McpServerStatusListRequest,
        config: Config,
        mcp_config: mcp_types::McpConfig,
        auth: Option<CodexAuth>,
        runtime_environment: McpRuntimeEnvironment,
    ) -> Result<ListMcpServerStatusResponse, JSONRPCErrorError> {
        let auth_snapshot = auth.as_ref().map(CodexAuth::request_auth_snapshot);
        let auth_context = codex_apps_auth_context(auth_snapshot.as_ref());

        let snapshot = collect_mcp_server_status_snapshot_with_detail(
            &mcp_config,
            auth_context.as_ref(),
            codex_apps_auth_provider(auth.as_ref()),
            request.request_id,
            runtime_environment,
            request.detail,
        )
        .await;

        let effective_servers = effective_mcp_servers(&mcp_config, auth_context.as_ref());
        McpServerStatusInventory::from_snapshot(
            config.mcp_servers.keys().cloned(),
            effective_servers.keys().cloned(),
            snapshot,
        )
        .into_response(request.page)
    }

    async fn read_mcp_resource(
        &self,
        request_id: &ConnectionRequestId,
        params: McpResourceReadParams,
    ) -> Result<(), JSONRPCErrorError> {
        let outgoing = Arc::clone(&self.outgoing);
        let McpResourceReadParams {
            thread_id,
            server,
            uri,
        } = params;

        if let Some(thread_id) = thread_id {
            let thread_id = Self::parse_thread_id(&thread_id)?;
            ensure_thread_mcp_runtime_available(
                self.live_thread_inspection.as_ref(),
                self.runtime.as_ref(),
                thread_id,
            )
            .await?;
            let runtime = Arc::clone(&self.runtime);
            let request_id = request_id.clone();

            tokio::spawn(async move {
                let result = runtime
                    .read_thread_mcp_resource(thread_id, &server, &uri)
                    .await;
                Self::send_mcp_resource_read_response(outgoing, request_id, result).await;
            });
            return Ok(());
        }

        let config =
            load_latest_config_for_request(&self.config_manager, /*fallback_cwd*/ None).await?;
        let mcp_config = self.runtime.mcp_config(&config).await;
        let auth = self.auth_manager.auth().await;
        let runtime_environment = {
            let environment_manager = Arc::clone(&self.environment_manager);
            let environment = environment_manager
                .default_environment()
                .unwrap_or_else(|| environment_manager.local_environment());
            // Resource reads without a thread have no turn cwd. This fallback
            // is used only by executor-backed stdio MCPs whose config omits `cwd`.
            mcp_runtime_environment(environment, config.cwd.to_path_buf())
        };
        let request_id = request_id.clone();
        let codex_apps_auth_provider = codex_apps_auth_provider(auth.as_ref());
        let auth_snapshot = auth.as_ref().map(CodexAuth::request_auth_snapshot);
        let auth_context = codex_apps_auth_context(auth_snapshot.as_ref());

        tokio::spawn(async move {
            let result = read_mcp_resource_without_thread(
                &mcp_config,
                auth_context.as_ref(),
                codex_apps_auth_provider,
                runtime_environment,
                &server,
                &uri,
            )
            .await
            .and_then(|result| serde_json::to_value(result).map_err(anyhow::Error::from));
            Self::send_mcp_resource_read_response(outgoing, request_id, result).await;
        });
        Ok(())
    }

    async fn send_mcp_resource_read_response(
        outgoing: Arc<OutgoingMessageSender>,
        request_id: ConnectionRequestId,
        result: anyhow::Result<serde_json::Value>,
    ) {
        let result = result
            .map_err(|error| internal_error(format!("{error:#}")))
            .and_then(|result| {
                serde_json::from_value::<McpResourceReadResponse>(result).map_err(|error| {
                    internal_error(format!(
                        "failed to deserialize MCP resource read response: {error}"
                    ))
                })
            });
        outgoing.send_result(request_id, result).await;
    }

    async fn call_mcp_server_tool(
        &self,
        request_id: &ConnectionRequestId,
        params: McpServerToolCallParams,
    ) -> Result<(), JSONRPCErrorError> {
        let outgoing = Arc::clone(&self.outgoing);
        let thread_id = params.thread_id.clone();
        let parsed_thread_id = Self::parse_thread_id(&thread_id)?;
        ensure_thread_mcp_runtime_available(
            self.live_thread_inspection.as_ref(),
            self.runtime.as_ref(),
            parsed_thread_id,
        )
        .await?;
        let runtime = Arc::clone(&self.runtime);
        let meta = with_mcp_tool_call_thread_id_meta(params.meta, &thread_id);
        let request_id = request_id.clone();

        tokio::spawn(async move {
            let result = runtime
                .call_thread_mcp_tool(
                    parsed_thread_id,
                    &params.server,
                    &params.tool,
                    params.arguments,
                    meta,
                )
                .await
                .map(McpServerToolCallResponse::from)
                .map_err(|error| internal_error(format!("{error:#}")));
            outgoing.send_result(request_id, result).await;
        });
        Ok(())
    }
}

struct McpServerStatusListRequest {
    request_id: String,
    detail: McpSnapshotDetail,
    page: McpServerStatusPage,
}

impl McpServerStatusListRequest {
    fn from_protocol(request_id: String, params: ListMcpServerStatusParams) -> Self {
        let detail = match params.detail.unwrap_or(McpServerStatusDetail::Full) {
            McpServerStatusDetail::Full => McpSnapshotDetail::Full,
            McpServerStatusDetail::ToolsAndAuthOnly => McpSnapshotDetail::ToolsAndAuthOnly,
        };
        Self {
            request_id,
            detail,
            page: McpServerStatusPage {
                cursor: params.cursor,
                limit: params.limit,
            },
        }
    }
}

struct McpServerStatusPage {
    cursor: Option<String>,
    limit: Option<u32>,
}

#[derive(Debug)]
struct McpServerStatusPageWindow {
    start: usize,
    end: usize,
    next_cursor: Option<String>,
}

impl McpServerStatusPage {
    fn window(&self, total: usize) -> Result<McpServerStatusPageWindow, JSONRPCErrorError> {
        let limit = self.limit.unwrap_or(total as u32).max(1) as usize;
        let effective_limit = limit.min(total);
        let start = match self.cursor.as_deref() {
            Some(cursor) => match cursor.parse::<usize>() {
                Ok(idx) => idx,
                Err(_) => return Err(invalid_request(format!("invalid cursor: {cursor}"))),
            },
            None => 0,
        };

        if start > total {
            return Err(invalid_request(format!(
                "cursor {start} exceeds total MCP servers {total}"
            )));
        }

        let end = start.saturating_add(effective_limit).min(total);
        let next_cursor = if end < total {
            Some(end.to_string())
        } else {
            None
        };

        Ok(McpServerStatusPageWindow {
            start,
            end,
            next_cursor,
        })
    }
}

struct McpServerStatusInventory {
    server_names: Vec<String>,
    tools_by_server: HashMap<String, HashMap<String, protocol::mcp::Tool>>,
    resources: HashMap<String, Vec<protocol::mcp::Resource>>,
    resource_templates: HashMap<String, Vec<protocol::mcp::ResourceTemplate>>,
    auth_statuses: HashMap<String, CoreMcpAuthStatus>,
}

impl McpServerStatusInventory {
    fn from_snapshot(
        configured_servers: impl Iterator<Item = String>,
        effective_servers: impl Iterator<Item = String>,
        snapshot: McpServerStatusSnapshot,
    ) -> Self {
        let McpServerStatusSnapshot {
            tools_by_server,
            resources,
            resource_templates,
            auth_statuses,
        } = snapshot;
        let server_names = collect_mcp_status_server_names(
            configured_servers,
            effective_servers,
            auth_statuses.keys().cloned(),
            resources.keys().cloned(),
            resource_templates.keys().cloned(),
        );
        Self {
            server_names,
            tools_by_server,
            resources,
            resource_templates,
            auth_statuses,
        }
    }

    fn into_response(
        self,
        page: McpServerStatusPage,
    ) -> Result<ListMcpServerStatusResponse, JSONRPCErrorError> {
        let page_window = page.window(self.server_names.len())?;
        let data = self.server_names[page_window.start..page_window.end]
            .iter()
            .map(|name| McpServerStatus {
                name: name.clone(),
                tools: self.tools_by_server.get(name).cloned().unwrap_or_default(),
                resources: self.resources.get(name).cloned().unwrap_or_default(),
                resource_templates: self
                    .resource_templates
                    .get(name)
                    .cloned()
                    .unwrap_or_default(),
                auth_status: self
                    .auth_statuses
                    .get(name)
                    .cloned()
                    .unwrap_or(CoreMcpAuthStatus::Unsupported)
                    .into(),
            })
            .collect();

        Ok(ListMcpServerStatusResponse {
            data,
            next_cursor: page_window.next_cursor,
        })
    }
}

fn collect_mcp_status_server_names(
    configured_servers: impl Iterator<Item = String>,
    effective_servers: impl Iterator<Item = String>,
    auth_servers: impl Iterator<Item = String>,
    resource_servers: impl Iterator<Item = String>,
    resource_template_servers: impl Iterator<Item = String>,
) -> Vec<String> {
    configured_servers
        .chain(effective_servers)
        .chain(auth_servers)
        .chain(resource_servers)
        .chain(resource_template_servers)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

async fn ensure_thread_mcp_runtime_available(
    live_thread_inspection: &(impl AppServerLiveThreadInspectionRuntime + ?Sized),
    runtime: &(impl McpProcessorRuntime + ?Sized),
    thread_id: ThreadId,
) -> Result<(), JSONRPCErrorError> {
    if live_thread_inspection
        .is_live_thread_loaded(thread_id)
        .await
        && runtime.is_thread_mcp_runtime_available(thread_id).await
    {
        return Ok(());
    }
    Err(invalid_request(format!("thread not found: {thread_id}")))
}

fn with_mcp_tool_call_thread_id_meta(
    meta: Option<serde_json::Value>,
    thread_id: &str,
) -> Option<serde_json::Value> {
    match meta {
        Some(serde_json::Value::Object(mut map)) => {
            map.insert(
                MCP_TOOL_THREAD_ID_META_KEY.to_string(),
                serde_json::Value::String(thread_id.to_string()),
            );
            Some(serde_json::Value::Object(map))
        }
        None => {
            let mut map = serde_json::Map::new();
            map.insert(
                MCP_TOOL_THREAD_ID_META_KEY.to_string(),
                serde_json::Value::String(thread_id.to_string()),
            );
            Some(serde_json::Value::Object(map))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_features::Feature;
    use protocol::error::CodexErr;
    use protocol::error::Result as CodexResult;
    use std::collections::HashSet;
    use thread_service_api::LiveThreadConfigRefreshSnapshot;
    use thread_service_api::LiveThreadInfo;
    use thread_service_api::LiveThreadSnapshot;
    use thread_service_api::ThreadConfigSnapshot;

    #[tokio::test]
    async fn thread_mcp_gate_rejects_loaded_thread_without_native_mcp_runtime() {
        let thread_id = ThreadId::new();
        let runtime = FakeMcpRuntime {
            loaded_thread_ids: HashSet::from([thread_id]),
            mcp_runtime_thread_ids: HashSet::new(),
        };

        let err = ensure_thread_mcp_runtime_available(&runtime, &runtime, thread_id)
            .await
            .expect_err("external-only thread should not enter native MCP runtime");

        assert_eq!(err.message, format!("thread not found: {thread_id}"));
    }

    #[tokio::test]
    async fn thread_mcp_gate_allows_loaded_native_mcp_runtime() {
        let thread_id = ThreadId::new();
        let runtime = FakeMcpRuntime {
            loaded_thread_ids: HashSet::from([thread_id]),
            mcp_runtime_thread_ids: HashSet::from([thread_id]),
        };

        ensure_thread_mcp_runtime_available(&runtime, &runtime, thread_id)
            .await
            .expect("native MCP runtime should be accepted");
    }

    #[test]
    fn mcp_status_server_names_are_sorted_and_deduplicated_across_sources() {
        let names = collect_mcp_status_server_names(
            ["configured", "shared"].into_iter().map(str::to_string),
            ["effective", "shared"].into_iter().map(str::to_string),
            ["auth"].into_iter().map(str::to_string),
            ["resources", "configured"].into_iter().map(str::to_string),
            ["templates"].into_iter().map(str::to_string),
        );

        assert_eq!(
            names,
            vec![
                "auth".to_string(),
                "configured".to_string(),
                "effective".to_string(),
                "resources".to_string(),
                "shared".to_string(),
                "templates".to_string(),
            ]
        );
    }

    #[test]
    fn mcp_status_inventory_projects_page_and_auth_fallbacks() {
        let inventory = McpServerStatusInventory {
            server_names: vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()],
            tools_by_server: HashMap::new(),
            resources: HashMap::new(),
            resource_templates: HashMap::new(),
            auth_statuses: HashMap::from([("beta".to_string(), CoreMcpAuthStatus::OAuth)]),
        };

        let response = inventory
            .into_response(McpServerStatusPage {
                cursor: Some("1".to_string()),
                limit: Some(1),
            })
            .unwrap();

        assert_eq!(response.next_cursor, Some("2".to_string()));
        assert_eq!(response.data.len(), 1);
        assert_eq!(response.data[0].name, "beta");
        assert_eq!(
            response.data[0].auth_status,
            app_server_protocol::McpAuthStatus::OAuth
        );

        let fallback = McpServerStatusInventory {
            server_names: vec!["alpha".to_string()],
            tools_by_server: HashMap::new(),
            resources: HashMap::new(),
            resource_templates: HashMap::new(),
            auth_statuses: HashMap::new(),
        }
        .into_response(McpServerStatusPage {
            cursor: None,
            limit: None,
        })
        .unwrap();

        assert_eq!(
            fallback.data[0].auth_status,
            app_server_protocol::McpAuthStatus::Unsupported
        );
    }

    #[test]
    fn mcp_status_page_preserves_cursor_errors() {
        let invalid = McpServerStatusPage {
            cursor: Some("not-a-number".to_string()),
            limit: Some(1),
        }
        .window(3)
        .expect_err("invalid cursor should fail");
        assert_eq!(invalid.message, "invalid cursor: not-a-number");

        let out_of_range = McpServerStatusPage {
            cursor: Some("4".to_string()),
            limit: Some(1),
        }
        .window(3)
        .expect_err("out of range cursor should fail");
        assert_eq!(out_of_range.message, "cursor 4 exceeds total MCP servers 3");
    }

    struct FakeMcpRuntime {
        loaded_thread_ids: HashSet<ThreadId>,
        mcp_runtime_thread_ids: HashSet<ThreadId>,
    }

    impl McpProcessorRuntime for FakeMcpRuntime {
        fn queue_strict_mcp_refresh(
            self: Arc<Self>,
            _config_manager: ConfigManager,
        ) -> BoxFuture<'static, io::Result<()>> {
            Box::pin(async { unreachable!("not used by gate tests") })
        }

        fn configured_mcp_servers<'a>(
            &'a self,
            _config: &'a Config,
        ) -> BoxFuture<'a, HashMap<String, McpServerConfig>> {
            Box::pin(async { unreachable!("not used by gate tests") })
        }

        fn mcp_config<'a>(&'a self, _config: &'a Config) -> BoxFuture<'a, mcp_types::McpConfig> {
            Box::pin(async { unreachable!("not used by gate tests") })
        }

        fn is_thread_mcp_runtime_available(&self, thread_id: ThreadId) -> BoxFuture<'_, bool> {
            Box::pin(async move { self.mcp_runtime_thread_ids.contains(&thread_id) })
        }

        fn read_thread_mcp_resource<'a>(
            &'a self,
            _thread_id: ThreadId,
            _server: &'a str,
            _uri: &'a str,
        ) -> BoxFuture<'a, anyhow::Result<serde_json::Value>> {
            Box::pin(async { unreachable!("not used by gate tests") })
        }

        fn call_thread_mcp_tool<'a>(
            &'a self,
            _thread_id: ThreadId,
            _server: &'a str,
            _tool: &'a str,
            _arguments: Option<serde_json::Value>,
            _meta: Option<serde_json::Value>,
        ) -> BoxFuture<'a, anyhow::Result<CallToolResult>> {
            Box::pin(async { unreachable!("not used by gate tests") })
        }
    }

    impl AppServerLiveThreadInspectionRuntime for FakeMcpRuntime {
        fn list_live_thread_ids(&self) -> BoxFuture<'_, Vec<ThreadId>> {
            Box::pin(async { self.loaded_thread_ids.iter().copied().collect() })
        }

        fn is_live_thread_loaded(&self, thread_id: ThreadId) -> BoxFuture<'_, bool> {
            Box::pin(async move { self.loaded_thread_ids.contains(&thread_id) })
        }

        fn live_thread_info(
            &self,
            thread_id: ThreadId,
        ) -> BoxFuture<'_, CodexResult<LiveThreadInfo>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }

        fn live_thread_snapshot(
            &self,
            thread_id: ThreadId,
        ) -> BoxFuture<'_, CodexResult<LiveThreadSnapshot>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }

        fn live_thread_config_snapshot(
            &self,
            thread_id: ThreadId,
        ) -> BoxFuture<'_, CodexResult<ThreadConfigSnapshot>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }

        fn live_thread_config_refresh_snapshot(
            &self,
            thread_id: ThreadId,
        ) -> BoxFuture<'_, CodexResult<LiveThreadConfigRefreshSnapshot>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }

        fn live_thread_feature_enabled(
            &self,
            thread_id: ThreadId,
            _feature: Feature,
        ) -> BoxFuture<'_, CodexResult<bool>> {
            Box::pin(async move { Err(CodexErr::ThreadNotFound(thread_id)) })
        }
    }
}
