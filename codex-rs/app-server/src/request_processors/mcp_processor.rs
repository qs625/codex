use super::*;

use codex_config_types::McpServerConfig;
use futures::future::BoxFuture;
use mcp_service_api::SharedMcpAuthHeaderProvider;
use mcp_service_api::StaticMcpAuthHeaderProvider;
use protocol::mcp::CallToolResult;
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

    fn is_thread_loaded(&self, thread_id: ThreadId) -> BoxFuture<'_, bool>;

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

    fn is_thread_loaded(&self, thread_id: ThreadId) -> BoxFuture<'_, bool> {
        Box::pin(thread_service_api::LiveThreadRegistry::is_thread_loaded(
            self, thread_id,
        ))
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
    outgoing: Arc<OutgoingMessageSender>,
    config_manager: ConfigManager,
    environment_manager: Arc<EnvironmentManager>,
}

impl McpRequestProcessor {
    pub(crate) fn new(
        auth_manager: Arc<AuthManager>,
        runtime: Arc<impl McpProcessorRuntime + 'static>,
        outgoing: Arc<OutgoingMessageSender>,
        config_manager: ConfigManager,
        environment_manager: Arc<EnvironmentManager>,
    ) -> Self {
        let runtime: Arc<dyn McpProcessorRuntime> = runtime;
        Self {
            auth_manager,
            runtime,
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

    async fn load_latest_config(
        &self,
        fallback_cwd: Option<PathBuf>,
    ) -> Result<Config, JSONRPCErrorError> {
        self.config_manager
            .load_latest_config(fallback_cwd)
            .await
            .map_err(|err| internal_error(format!("failed to reload config: {err}")))
    }

    fn parse_thread_id(thread_id: &str) -> Result<ThreadId, JSONRPCErrorError> {
        ThreadId::from_string(thread_id)
            .map_err(|err| invalid_request(format!("invalid thread id: {err}")))
    }

    async fn mcp_server_oauth_login_response(
        &self,
        params: McpServerOauthLoginParams,
    ) -> Result<McpServerOauthLoginResponse, JSONRPCErrorError> {
        let config = self.load_latest_config(/*fallback_cwd*/ None).await?;
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
        let config = self.load_latest_config(/*fallback_cwd*/ None).await?;
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
            request_id.request_id.to_string(),
            params,
            config,
            mcp_config,
            auth,
            runtime_environment,
        )
        .await;
        outgoing.send_result(request_id, result).await;
    }

    async fn list_mcp_server_status_response(
        request_id: String,
        params: ListMcpServerStatusParams,
        config: Config,
        mcp_config: mcp_types::McpConfig,
        auth: Option<CodexAuth>,
        runtime_environment: McpRuntimeEnvironment,
    ) -> Result<ListMcpServerStatusResponse, JSONRPCErrorError> {
        let detail = match params.detail.unwrap_or(McpServerStatusDetail::Full) {
            McpServerStatusDetail::Full => McpSnapshotDetail::Full,
            McpServerStatusDetail::ToolsAndAuthOnly => McpSnapshotDetail::ToolsAndAuthOnly,
        };
        let auth_snapshot = auth.as_ref().map(CodexAuth::request_auth_snapshot);
        let auth_context = codex_apps_auth_context(auth_snapshot.as_ref());

        let snapshot = collect_mcp_server_status_snapshot_with_detail(
            &mcp_config,
            auth_context.as_ref(),
            codex_apps_auth_provider(auth.as_ref()),
            request_id,
            runtime_environment,
            detail,
        )
        .await;

        let effective_servers = effective_mcp_servers(&mcp_config, auth_context.as_ref());
        let McpServerStatusSnapshot {
            tools_by_server,
            resources,
            resource_templates,
            auth_statuses,
        } = snapshot;

        let mut server_names: Vec<String> = config
            .mcp_servers
            .keys()
            .cloned()
            // Include runtime-added/plugin MCP servers that are present in the
            // effective runtime config even when they are not user-declared in
            // `config.mcp_servers`.
            .chain(effective_servers.keys().cloned())
            .chain(auth_statuses.keys().cloned())
            .chain(resources.keys().cloned())
            .chain(resource_templates.keys().cloned())
            .collect();
        server_names.sort();
        server_names.dedup();

        let total = server_names.len();
        let limit = params.limit.unwrap_or(total as u32).max(1) as usize;
        let effective_limit = limit.min(total);
        let start = match params.cursor {
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

        let data: Vec<McpServerStatus> = server_names[start..end]
            .iter()
            .map(|name| McpServerStatus {
                name: name.clone(),
                tools: tools_by_server.get(name).cloned().unwrap_or_default(),
                resources: resources.get(name).cloned().unwrap_or_default(),
                resource_templates: resource_templates.get(name).cloned().unwrap_or_default(),
                auth_status: auth_statuses
                    .get(name)
                    .cloned()
                    .unwrap_or(CoreMcpAuthStatus::Unsupported)
                    .into(),
            })
            .collect();

        let next_cursor = if end < total {
            Some(end.to_string())
        } else {
            None
        };

        Ok(ListMcpServerStatusResponse { data, next_cursor })
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
            if !self.runtime.is_thread_loaded(thread_id).await {
                return Err(invalid_request(format!("thread not found: {thread_id}")));
            }
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

        let config = self.load_latest_config(/*fallback_cwd*/ None).await?;
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
        if !self.runtime.is_thread_loaded(parsed_thread_id).await {
            return Err(invalid_request(format!(
                "thread not found: {parsed_thread_id}"
            )));
        }
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
