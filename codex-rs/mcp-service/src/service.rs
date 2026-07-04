use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use codex_approval_service_api::ApprovalServiceApi;
use codex_approval_service_api::ApprovalSessionCapability;
use codex_approval_service_api::GuardianReviewDispatch;
use codex_approval_service_api::PermissionRequestPayload;
use hooks_api::PermissionRequestDecision;
use mcp_service_api::McpAppUsageMetadata;
use mcp_service_api::McpRuntimeFuture;
use mcp_service_api::McpServiceApi;
use mcp_service_api::McpToolCallOutcome;
use mcp_service_api::McpToolExposure;
use mcp_types::ElicitationResponse;
use mcp_types::McpServerElicitationRequestParams;
use mcp_types::McpToolApprovalDecision;
use mcp_types::McpToolApprovalKey;
use mcp_types::McpToolApprovalMetadata;
use mcp_types::ToolInfo;
use protocol::items::TurnItem;
use protocol::mcp::CallToolResult;
use protocol::mcp::ListResourceTemplatesResult;
use protocol::mcp::ListResourcesResult;
use protocol::mcp::PaginatedRequestParams;
use protocol::mcp::ReadResourceRequestParams;
use protocol::mcp::ReadResourceResult;
use protocol::mcp::RequestId;
use protocol::mcp::Resource;
use protocol::mcp::ResourceTemplate;
use protocol::protocol::McpInvocation;
use protocol::protocol::ReviewDecision;
use protocol::request_user_input::RequestUserInputArgs;
use protocol::request_user_input::RequestUserInputResponse;
use skill_service_api::SkillMetadata;
use thread_service_api::AutoApprovalSafetyOutcome;
use thread_service_api::HookToolName;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;

use crate::AppToolPolicy;
use crate::CodexAppsAuthElicitationHost;
use crate::MCP_CALL_COUNT_METRIC;
use crate::MCP_CALL_DURATION_METRIC;
use crate::McpApprovedToolCallLifecycleHost;
use crate::McpSkillDependencyHost;
use crate::McpSkillDependencyTurnContext;
use crate::McpToolApprovalHookDecision;
use crate::McpToolApprovalMonitorOutcome;
use crate::McpToolApprovalPersistenceHost;
use crate::McpToolApprovalReviewContext;
use crate::McpToolApprovalReviewHost;
use crate::McpToolCallContext;
use crate::McpToolCallHost;
use crate::McpToolExecutionHost;
use crate::McpToolMetadataLookupHost;
use crate::build_mcp_tool_call_completed_item;
use crate::build_mcp_tool_call_started_item;
use crate::codex_apps_auth_context;
use crate::handle_mcp_tool_call;
use crate::insert_sandbox_state_request_meta;
use crate::list_tool_suggest_discoverable_tools_with_auth;
use crate::maybe_prompt_and_install_mcp_dependencies;
use crate::maybe_request_mcp_tool_approval;
use crate::mcp_call_metric_tags;
use tool_service_api::DiscoverableTool;
use tool_service_api::filter_request_plugin_install_discoverable_tools_for_client;

#[derive(Clone)]
pub struct McpService {
    approval_api: Arc<dyn ApprovalServiceApi>,
}

impl McpService {
    pub fn new(approval_api: Arc<dyn ApprovalServiceApi>) -> Self {
        Self { approval_api }
    }
}

struct ServiceMcpHost {
    approval_api: Arc<dyn ApprovalServiceApi>,
    session: Arc<dyn ThreadSessionCapability>,
    approval_session: Option<Arc<dyn ApprovalSessionCapability>>,
    turn: Arc<dyn ThreadRuntimeCapability>,
}

impl ServiceMcpHost {
    fn turn_view(&self) -> &dyn ThreadTurnCapability {
        self.turn.as_ref()
    }
}

struct ServiceMcpSkillDependencyHost<'a> {
    session: &'a dyn ThreadSessionCapability,
    turn: &'a dyn ThreadTurnCapability,
}

impl McpSkillDependencyHost for ServiceMcpSkillDependencyHost<'_> {
    fn configured_servers<'a>(
        &'a self,
        config: &'a config_service::Config,
    ) -> McpRuntimeFuture<'a, HashMap<String, codex_config_types::McpServerConfig>> {
        let _ = config;
        self.session.configured_mcp_servers()
    }

    fn prompted_dependency_keys(&self) -> McpRuntimeFuture<'_, std::collections::HashSet<String>> {
        self.session.mcp_dependency_prompted()
    }

    fn record_prompted_dependency_keys<'a>(
        &'a self,
        names: Vec<String>,
    ) -> McpRuntimeFuture<'a, ()> {
        self.session.record_mcp_dependency_prompted(names)
    }

    fn request_user_input<'a>(
        &'a self,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> McpRuntimeFuture<'a, Option<RequestUserInputResponse>> {
        self.turn.request_user_input(call_id, args)
    }

    fn notify_user_input_response<'a>(
        &'a self,
        sub_id: &'a str,
        response: RequestUserInputResponse,
    ) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move {
            self.session
                .notify_user_input_response(sub_id, response)
                .await;
        })
    }

    fn oauth_login_support<'a>(
        &'a self,
        transport: &'a codex_config_types::McpServerTransportConfig,
    ) -> McpRuntimeFuture<'a, mcp_types::McpOAuthLoginSupport> {
        self.session.mcp_oauth_login_support(transport)
    }

    fn perform_oauth_login<'a>(
        &'a self,
        request: mcp_service_api::McpOAuthLoginRequest,
    ) -> McpRuntimeFuture<'a, anyhow::Result<()>> {
        self.session
            .perform_mcp_oauth_login(thread_service_api::McpOAuthLoginParams {
                server_name: request.server_name,
                server_url: request.server_url,
                store_mode: request.store_mode,
                http_headers: request.http_headers,
                env_http_headers: request.env_http_headers,
                scopes: request.scopes,
                oauth_client_id: request.oauth_client_id,
                oauth_resource: request.oauth_resource,
                callback_port: request.callback_port,
                callback_url: request.callback_url,
            })
    }

    fn should_retry_without_scopes(
        &self,
        scopes: &mcp_types::ResolvedMcpOAuthScopes,
        error: &anyhow::Error,
    ) -> bool {
        self.session
            .should_retry_mcp_oauth_without_scopes(scopes, error)
    }

    fn refresh_mcp_servers_now<'a>(
        &'a self,
        servers: HashMap<String, codex_config_types::McpServerConfig>,
        store_mode: codex_config_types::OAuthCredentialsStoreMode,
        elicitation_reviewer: Option<mcp_types::ElicitationReviewerHandle>,
    ) -> McpRuntimeFuture<'a, ()> {
        let refresh_config = protocol::protocol::McpServerRefreshConfig {
            mcp_servers: serde_json::to_value(servers)
                .unwrap_or_else(|_| serde_json::Value::Object(Default::default())),
            mcp_oauth_credentials_store_mode: serde_json::to_value(store_mode)
                .unwrap_or(serde_json::Value::Null),
        };
        self.session
            .refresh_mcp_servers_now(self.turn, refresh_config, elicitation_reviewer)
    }
}

impl McpServiceApi for McpService {
    fn list_accessible_connectors(
        &self,
        all_mcp_tools: &[ToolInfo],
        config: &config_service::Config,
    ) -> Vec<codex_connectors_api::AppInfo> {
        crate::with_app_enabled_state(
            crate::accessible_connectors_from_mcp_tools(all_mcp_tools),
            config,
        )
    }

    fn list_available_connectors<'a>(
        &self,
        plugin_runtime: &'a dyn plugin_service_api::PluginRuntime,
        all_mcp_tools: &'a [ToolInfo],
        config: &'a config_service::Config,
    ) -> McpRuntimeFuture<'a, Vec<codex_connectors_api::AppInfo>> {
        Box::pin(async move {
            let plugin_effective_apps = plugin_runtime
                .effective_apps_for_config(&config.plugins_config_input())
                .await;
            let connectors = codex_connectors_api::merge::merge_plugin_connectors_with_accessible(
                plugin_effective_apps
                    .into_iter()
                    .map(|connector_id| connector_id.0),
                crate::accessible_connectors_from_mcp_tools(all_mcp_tools),
            );
            crate::with_app_enabled_state(connectors, config)
        })
    }

    fn list_discoverable_tools<'a>(
        &self,
        turn: &'a dyn ThreadTurnCapability,
        plugin_runtime: &'a dyn plugin_service_api::PluginRuntime,
        accessible_connectors: &'a [codex_connectors_api::AppInfo],
        config: &'a config_service::Config,
        app_server_client_name: Option<&'a str>,
        tool_suggest_enabled: bool,
        apps_enabled: bool,
    ) -> McpRuntimeFuture<'a, Result<Vec<DiscoverableTool>, String>> {
        Box::pin(async move {
            if !apps_enabled || !tool_suggest_enabled {
                return Ok(Vec::new());
            }

            let auth_snapshot = turn.auth_snapshot().await;
            let connector_auth_context = codex_apps_auth_context(auth_snapshot.as_ref());
            list_tool_suggest_discoverable_tools_with_auth(
                config,
                plugin_runtime,
                connector_auth_context.as_ref(),
                accessible_connectors,
            )
            .await
            .map(|discoverable_tools| {
                filter_request_plugin_install_discoverable_tools_for_client(
                    discoverable_tools,
                    app_server_client_name,
                )
            })
            .map_err(|err| err.to_string())
        })
    }

    fn build_tool_exposure(
        &self,
        all_mcp_tools: &[ToolInfo],
        connectors: Option<&[codex_connectors_api::AppInfo]>,
        explicitly_enabled_connectors: &[codex_connectors_api::AppInfo],
        config: &config_service::Config,
        tools_config: &tool_config::ToolsConfig,
    ) -> McpToolExposure {
        let exposure = crate::build_mcp_tool_exposure(
            all_mcp_tools,
            connectors,
            explicitly_enabled_connectors,
            config,
            tools_config,
        );
        McpToolExposure {
            direct_tools: exposure.direct_tools,
            deferred_tools: exposure.deferred_tools,
        }
    }

    fn maybe_prompt_and_install_mcp_dependencies<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        config: &'a config_service::Config,
        cancellation_token: &'a tokio_util::sync::CancellationToken,
        mentioned_skills: &'a [SkillMetadata],
        elicitation_reviewer: Option<mcp_types::ElicitationReviewerHandle>,
    ) -> McpRuntimeFuture<'a, ()> {
        let host = ServiceMcpSkillDependencyHost { session, turn };
        let turn_context = McpSkillDependencyTurnContext {
            sub_id: turn.runtime_turn_id_str(),
            approval_policy: turn.approval_policy(),
            permission_profile: turn.permission_profile(),
        };
        Box::pin(async move {
            maybe_prompt_and_install_mcp_dependencies(
                &host,
                &turn_context,
                config,
                cancellation_token,
                mentioned_skills,
                elicitation_reviewer,
            )
            .await;
        })
    }

    fn lookup_app_usage_metadata(
        &self,
        all_mcp_tools: &[ToolInfo],
        server: &str,
        tool_name: &str,
    ) -> Option<McpAppUsageMetadata> {
        crate::lookup_mcp_app_usage_metadata(all_mcp_tools, server, tool_name)
    }

    fn configured_servers<'a>(
        &self,
        plugin_runtime: &'a dyn plugin_service_api::PluginRuntime,
        config: &'a config_service::Config,
    ) -> McpRuntimeFuture<'a, HashMap<String, config_service::McpServerConfig>> {
        Box::pin(async move {
            let mcp_config = config.to_mcp_config(plugin_runtime).await;
            mcp_types::configured_mcp_servers(&mcp_config)
        })
    }

    fn effective_servers<'a>(
        &self,
        plugin_runtime: &'a dyn plugin_service_api::PluginRuntime,
        config: &'a config_service::Config,
        auth_context: Option<&'a mcp_types::CodexAppsAuthContext>,
    ) -> McpRuntimeFuture<'a, HashMap<String, mcp_types::EffectiveMcpServer>> {
        Box::pin(async move {
            let mcp_config = config.to_mcp_config(plugin_runtime).await;
            mcp_types::effective_mcp_servers(&mcp_config, auth_context)
        })
    }

    fn tool_plugin_provenance<'a>(
        &self,
        plugin_runtime: &'a dyn plugin_service_api::PluginRuntime,
        config: &'a config_service::Config,
    ) -> McpRuntimeFuture<'a, mcp_types::ToolPluginProvenance> {
        Box::pin(async move {
            let mcp_config = config.to_mcp_config(plugin_runtime).await;
            mcp_types::tool_plugin_provenance(&mcp_config)
        })
    }

    fn list_accessible_and_enabled_connectors(
        &self,
        all_mcp_tools: &[ToolInfo],
        config: &config_service::Config,
    ) -> Vec<codex_connectors_api::AppInfo> {
        crate::with_app_enabled_state(
            crate::accessible_connectors_from_mcp_tools(all_mcp_tools),
            config,
        )
        .into_iter()
        .filter(|connector| connector.is_accessible && connector.is_enabled)
        .collect()
    }

    fn fetch_accessible_connectors<'a>(
        &self,
        plugin_runtime: &'a dyn plugin_service_api::PluginRuntime,
        config: &'a config_service::Config,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
        environment_provider: &'a dyn exec_server_api::ExecEnvironmentProvider,
        mcp_auth_runtime: &'a dyn mcp_service_api::McpAuthRuntime,
        mcp_connection_runtime_factory: &'a dyn mcp_service_api::McpConnectionRuntimeFactory,
    ) -> McpRuntimeFuture<'a, anyhow::Result<Vec<codex_connectors_api::AppInfo>>> {
        Box::pin(async move {
            crate::list_accessible_connectors_from_mcp_tools(
                config,
                auth_snapshot,
                plugin_runtime,
                environment_provider,
                mcp_auth_runtime,
                mcp_connection_runtime_factory,
            )
            .await
        })
    }

    fn app_tool_policy(
        &self,
        config: &config_service::Config,
        metadata: Option<&mcp_types::McpToolApprovalMetadata>,
        tool_name: &str,
    ) -> thread_service_api::ThreadAppToolPolicy {
        let policy = crate::app_tool_policy(
            config,
            metadata.and_then(|metadata| metadata.connector_id.as_deref()),
            tool_name,
            metadata.and_then(|metadata| metadata.tool_title.as_deref()),
            metadata.and_then(|metadata| metadata.annotations.as_ref()),
        );
        thread_service_api::ThreadAppToolPolicy {
            enabled: policy.enabled,
            approval: policy.approval,
        }
    }

    fn list_cached_accessible_connectors<'a>(
        &self,
        config: &'a config_service::Config,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> McpRuntimeFuture<'a, Option<Vec<codex_connectors_api::AppInfo>>> {
        Box::pin(async move {
            crate::list_cached_accessible_connectors_from_mcp_tools(config, auth_snapshot).await
        })
    }

    fn refresh_accessible_connectors_cache(
        &self,
        config: &config_service::Config,
        connector_auth_context: Option<&mcp_types::CodexAppsAuthContext>,
        mcp_tools: &[ToolInfo],
    ) {
        crate::refresh_accessible_connectors_cache_from_mcp_tools(
            config,
            connector_auth_context,
            mcp_tools,
        );
    }

    fn codex_apps_auth_context(
        &self,
        auth: Option<&codex_auth_types::RequestAuthSnapshot>,
    ) -> Option<mcp_types::CodexAppsAuthContext> {
        crate::codex_apps_auth_context(auth)
    }

    fn codex_apps_auth_provider(
        &self,
        auth: Option<&codex_auth_types::RequestAuthSnapshot>,
    ) -> Option<mcp_service_api::SharedMcpAuthHeaderProvider> {
        crate::codex_apps_auth_provider(auth)
    }

    fn build_runtime_environment(
        &self,
        environment: Arc<dyn exec_server_api::ExecEnvironment>,
        local_environment: Arc<dyn exec_server_api::ExecEnvironment>,
        fallback_cwd: std::path::PathBuf,
    ) -> mcp_service_api::McpRuntimeEnvironment {
        crate::mcp_runtime_environment(environment, local_environment, fallback_cwd)
    }

    fn start_connection_runtime<'a>(
        &self,
        factory: &'a dyn mcp_service_api::McpConnectionRuntimeFactory,
        request: mcp_service_api::McpConnectionRuntimeStartRequest,
    ) -> McpRuntimeFuture<'a, mcp_service_api::McpConnectionRuntimeStart> {
        Box::pin(async move { factory.start(request).await })
    }

    fn review_guardian_elicitation<'a>(
        &self,
        session: Arc<dyn ApprovalSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        request: mcp_types::ElicitationReviewRequest,
    ) -> McpRuntimeFuture<'a, anyhow::Result<Option<mcp_types::ElicitationResponse>>> {
        let approval_api = Arc::clone(&self.approval_api);
        Box::pin(async move {
            if !codex_approval_service_api::routes_approval_to_guardian(
                &turn.approval_policy(),
                turn.approvals_reviewer(),
            ) {
                return Ok(None);
            }

            let guardian_request = match crate::guardian_elicitation_review_request(&request) {
                crate::GuardianElicitationReview::NotRequested => return Ok(None),
                crate::GuardianElicitationReview::Decline(reason) => {
                    tracing::warn!(
                        server_name = %request.server_name,
                        request_id = %crate::mcp_elicitation_request_id(&request.request_id),
                        reason,
                        "declining Guardian MCP elicitation before review"
                    );
                    return Ok(Some(mcp_types::ElicitationResponse {
                        action: mcp_types::ElicitationAction::Decline,
                        content: None,
                        meta: Some(serde_json::json!({
                            "approvals_reviewer": protocol::config_types::ApprovalsReviewer::AutoReview,
                        })),
                    }));
                }
                crate::GuardianElicitationReview::ApprovalRequest(guardian_request) => {
                    *guardian_request
                }
            };

            let review = approval_api
                .review_guardian_request(codex_approval_service_api::GuardianReviewDispatch {
                    session,
                    turn,
                    review_id: uuid::Uuid::new_v4().to_string(),
                    request: guardian_request,
                    retry_reason: None,
                    approval_request_source:
                        codex_analytics_api::GuardianApprovalRequestSource::MainTurn,
                    cancellation_token: None,
                })
                .await;
            Ok(Some(
                crate::mcp_elicitation_response_from_guardian_decision_parts(
                    review.decision,
                    review.decline_message,
                ),
            ))
        })
    }

    fn rewrite_tool_arguments_for_openai_files<'a>(
        &self,
        uploader: &'a dyn codex_openai_files_api::OpenAiFileUploader,
        auth: Option<&'a codex_auth_types::RequestAuthSnapshot>,
        chatgpt_base_url: &'a str,
        turn: &'a dyn ThreadRuntimeCapability,
        arguments_value: Option<serde_json::Value>,
        openai_file_input_params: Option<&'a [String]>,
    ) -> McpRuntimeFuture<'a, Result<Option<serde_json::Value>, String>> {
        struct TurnPathResolver<'a> {
            turn: &'a dyn ThreadRuntimeCapability,
        }

        impl crate::OpenAiFilePathResolver for TurnPathResolver<'_> {
            fn resolve_path(&self, file_path: &str) -> std::path::PathBuf {
                self.turn
                    .resolve_turn_path(Some(file_path.to_string()))
                    .to_path_buf()
            }
        }

        let path_resolver = TurnPathResolver { turn };
        Box::pin(async move {
            crate::rewrite_mcp_tool_arguments_for_openai_files(
                uploader,
                auth,
                chatgpt_base_url,
                &path_resolver,
                arguments_value,
                openai_file_input_params,
            )
            .await
        })
    }

    fn custom_tool_approval_mode<'a>(
        &self,
        plugin_runtime: &'a dyn plugin_service_api::PluginRuntime,
        config: &'a config_service::Config,
        server: &'a str,
        tool_name: &'a str,
    ) -> McpRuntimeFuture<'a, codex_config_types::AppToolApproval> {
        Box::pin(async move {
            crate::custom_mcp_tool_approval_mode(config, plugin_runtime, server, tool_name).await
        })
    }

    fn persist_codex_app_tool_approval<'a>(
        &self,
        config: &'a config_service::Config,
        connector_id: &'a str,
        tool_name: &'a str,
    ) -> McpRuntimeFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            crate::persist_codex_app_tool_approval(config, connector_id, tool_name).await
        })
    }

    fn persist_non_app_mcp_tool_approval<'a>(
        &self,
        plugin_runtime: &'a dyn plugin_service_api::PluginRuntime,
        config: &'a config_service::Config,
        server: &'a str,
        tool_name: &'a str,
    ) -> McpRuntimeFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            crate::persist_non_app_mcp_tool_approval(config, plugin_runtime, server, tool_name)
                .await
        })
    }

    fn request_server_elicitation<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> McpRuntimeFuture<'a, Option<ElicitationResponse>> {
        Box::pin(async move {
            session
                .request_mcp_server_elicitation(turn, request_id, params)
                .await
        })
    }

    fn resolve_elicitation<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        server_name: String,
        request_id: RequestId,
        response: ElicitationResponse,
    ) -> McpRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async move {
            session
                .resolve_mcp_elicitation(server_name, request_id, response)
                .await
        })
    }

    fn refresh_servers_if_requested<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        elicitation_reviewer: Option<mcp_types::ElicitationReviewerHandle>,
    ) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move {
            session
                .refresh_mcp_servers_if_requested(turn, elicitation_reviewer)
                .await;
        })
    }

    fn queue_server_refresh<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        refresh_config: protocol::protocol::McpServerRefreshConfig,
    ) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move { session.queue_mcp_server_refresh(refresh_config).await })
    }

    fn refresh_servers_now<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        refresh_config: protocol::protocol::McpServerRefreshConfig,
        elicitation_reviewer: Option<mcp_types::ElicitationReviewerHandle>,
    ) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move {
            session
                .refresh_mcp_servers_now(turn, refresh_config, elicitation_reviewer)
                .await;
        })
    }

    fn cancel_startup<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
    ) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move { session.cancel_mcp_startup().await })
    }

    fn hard_refresh_codex_apps_tools_cache<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
    ) -> McpRuntimeFuture<'a, Result<Vec<ToolInfo>, String>> {
        Box::pin(async move { session.hard_refresh_codex_apps_tools_cache().await })
    }

    fn lookup_tool_metadata<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        server: &'a str,
        tool_name: &'a str,
    ) -> McpRuntimeFuture<'a, Option<McpToolApprovalMetadata>> {
        let host = ServiceMcpHost {
            approval_api: Arc::clone(&self.approval_api),
            session,
            approval_session: None,
            turn,
        };
        Box::pin(async move { crate::lookup_mcp_tool_metadata(&host, server, tool_name).await })
    }

    fn call_tool<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        approval_session: Arc<dyn ApprovalSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        server: String,
        tool_name: String,
        hook_tool_name: String,
        arguments: String,
    ) -> McpRuntimeFuture<'a, McpToolCallOutcome> {
        let host = ServiceMcpHost {
            approval_api: Arc::clone(&self.approval_api),
            session,
            approval_session: Some(approval_session),
            turn,
        };
        Box::pin(async move {
            let thread_id = host.session.conversation_id().to_string();
            let outcome = handle_mcp_tool_call(
                &host,
                McpToolCallContext {
                    thread_id,
                    turn_id: host.turn.runtime_turn_id().to_string(),
                    call_id,
                    server,
                    tool_name,
                    hook_tool_name,
                    arguments,
                    turn_metadata: host.turn_view().mcp_turn_metadata(),
                    turn_metadata_header_name: "x-codex-turn-metadata",
                    supports_image_input: host.turn_view().supports_image_input(),
                    auth_elicitation_enabled: host.turn_view().auth_elicitation_enabled(),
                    approval_policy: host.turn_view().approval_policy(),
                },
            )
            .await;
            McpToolCallOutcome {
                result: outcome.result,
                tool_input: outcome.tool_input,
            }
        })
    }

    fn list_resources<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, Result<ListResourcesResult, String>> {
        Box::pin(async move {
            let invocation = McpInvocation {
                server: server.to_string(),
                tool: "list_mcp_resources".to_string(),
                arguments: None,
            };
            emit_mcp_resource_started(session.as_ref(), turn.as_ref(), &call_id, &invocation).await;
            let start = Instant::now();
            let result = session.list_mcp_resources(server, params).await;
            emit_mcp_resource_completed(
                session.as_ref(),
                turn.as_ref(),
                &call_id,
                invocation,
                start.elapsed(),
                &result,
            )
            .await;
            result
        })
    }

    fn list_all_resources<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
    ) -> McpRuntimeFuture<'a, HashMap<String, Vec<Resource>>> {
        Box::pin(async move {
            let invocation = McpInvocation {
                server: "codex".to_string(),
                tool: "list_mcp_resources".to_string(),
                arguments: None,
            };
            emit_mcp_resource_started(session.as_ref(), turn.as_ref(), &call_id, &invocation).await;
            let start = Instant::now();
            let result = session.list_all_mcp_resources().await;
            let completed = Ok(ok_call_tool_result());
            session
                .emit_turn_item_completed(
                    turn.as_ref(),
                    build_mcp_tool_call_completed_item(
                        &call_id,
                        invocation,
                        None,
                        start.elapsed(),
                        completed,
                    ),
                )
                .await;
            result
        })
    }

    fn list_resource_templates<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        server: &'a str,
        params: Option<PaginatedRequestParams>,
    ) -> McpRuntimeFuture<'a, Result<ListResourceTemplatesResult, String>> {
        Box::pin(async move {
            let invocation = McpInvocation {
                server: server.to_string(),
                tool: "list_mcp_resource_templates".to_string(),
                arguments: None,
            };
            emit_mcp_resource_started(session.as_ref(), turn.as_ref(), &call_id, &invocation).await;
            let start = Instant::now();
            let result = session.list_mcp_resource_templates(server, params).await;
            emit_mcp_resource_completed(
                session.as_ref(),
                turn.as_ref(),
                &call_id,
                invocation,
                start.elapsed(),
                &result,
            )
            .await;
            result
        })
    }

    fn list_all_resource_templates<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
    ) -> McpRuntimeFuture<'a, HashMap<String, Vec<ResourceTemplate>>> {
        Box::pin(async move {
            let invocation = McpInvocation {
                server: "codex".to_string(),
                tool: "list_mcp_resource_templates".to_string(),
                arguments: None,
            };
            emit_mcp_resource_started(session.as_ref(), turn.as_ref(), &call_id, &invocation).await;
            let start = Instant::now();
            let result = session.list_all_mcp_resource_templates().await;
            let completed = Ok(ok_call_tool_result());
            session
                .emit_turn_item_completed(
                    turn.as_ref(),
                    build_mcp_tool_call_completed_item(
                        &call_id,
                        invocation,
                        None,
                        start.elapsed(),
                        completed,
                    ),
                )
                .await;
            result
        })
    }

    fn read_resource<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
        turn: Arc<dyn ThreadRuntimeCapability>,
        call_id: String,
        server: &'a str,
        params: ReadResourceRequestParams,
    ) -> McpRuntimeFuture<'a, Result<ReadResourceResult, String>> {
        Box::pin(async move {
            let invocation = McpInvocation {
                server: server.to_string(),
                tool: "read_mcp_resource".to_string(),
                arguments: None,
            };
            emit_mcp_resource_started(session.as_ref(), turn.as_ref(), &call_id, &invocation).await;
            let start = Instant::now();
            let result = session.read_mcp_resource(server, params).await;
            emit_mcp_resource_completed(
                session.as_ref(),
                turn.as_ref(),
                &call_id,
                invocation,
                start.elapsed(),
                &result,
            )
            .await;
            result
        })
    }
}

impl CodexAppsAuthElicitationHost for ServiceMcpHost {
    async fn request_codex_apps_auth_elicitation(
        &self,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> Option<ElicitationResponse> {
        self.session
            .request_mcp_server_elicitation(self.turn_view(), request_id, params)
            .await
    }

    async fn refresh_codex_apps_after_connector_auth(&self) {
        match self.session.hard_refresh_codex_apps_tools_cache().await {
            Ok(mcp_tools) => {
                let auth_snapshot = self.turn_view().auth_snapshot().await;
                let connector_auth_context = codex_apps_auth_context(auth_snapshot.as_ref());
                self.turn_view()
                    .refresh_accessible_connectors_cache_from_mcp_tools(
                        connector_auth_context.as_ref(),
                        &mcp_tools,
                    );
            }
            Err(err) => {
                tracing::warn!("failed to refresh Codex Apps tools after connector auth: {err}");
            }
        }
    }
}

impl McpToolExecutionHost for ServiceMcpHost {
    async fn augment_mcp_tool_request_meta_with_sandbox_state(
        &self,
        server: &str,
        meta: Option<serde_json::Value>,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        if !self
            .session
            .mcp_server_supports_sandbox_state_meta(server)
            .await
        {
            return Ok(meta);
        }
        insert_sandbox_state_request_meta(meta, self.turn_view().mcp_sandbox_state())
    }

    fn add_mcp_call_trace_request_meta(
        &self,
        call_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Option<serde_json::Value> {
        self.session
            .add_optional_mcp_call_trace_request_meta(call_id, meta)
    }

    async fn call_mcp_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> Result<CallToolResult, String> {
        self.session
            .call_mcp_tool(server, tool, arguments, meta)
            .await
    }
}

impl McpApprovedToolCallLifecycleHost for ServiceMcpHost {
    async fn mark_thread_memory_mode_polluted_if_needed(&self, server: &str) {
        self.session
            .mark_thread_memory_mode_polluted_for_mcp_tool_call(self.turn_view(), server)
            .await;
    }

    async fn server_origin(&self, server: &str) -> Option<String> {
        self.session.mcp_server_origin(server).await
    }

    async fn rewrite_mcp_tool_arguments_for_openai_files(
        &self,
        arguments: Option<serde_json::Value>,
        openai_file_input_params: Option<&[String]>,
    ) -> Result<Option<serde_json::Value>, String> {
        self.session
            .rewrite_mcp_tool_arguments_for_openai_files(
                self.turn_view(),
                arguments,
                openai_file_input_params,
            )
            .await
    }

    async fn emit_mcp_tool_call_completed(&self, item: TurnItem) {
        self.session
            .emit_turn_item_completed(self.turn_view(), item)
            .await;
    }

    async fn track_codex_app_used(&self, server: &str, tool_name: &str) {
        self.session
            .track_codex_app_used_for_mcp_tool(self.turn_view(), server, tool_name)
            .await;
    }

    fn emit_mcp_call_metrics(
        &self,
        status: &str,
        tool_name: &str,
        connector_id: Option<&str>,
        connector_name: Option<&str>,
        duration: std::time::Duration,
    ) {
        let tags = mcp_call_metric_tags(status, tool_name, connector_id, connector_name);
        let tag_refs: Vec<(&str, &str)> = tags
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        self.turn_view()
            .tool_dispatch_telemetry()
            .counter(MCP_CALL_COUNT_METRIC, 1, &tag_refs);
        self.turn_view().tool_dispatch_telemetry().record_duration(
            MCP_CALL_DURATION_METRIC,
            duration,
            &tag_refs,
        );
    }
}

impl McpToolMetadataLookupHost for ServiceMcpHost {
    async fn list_all_mcp_tools(&self) -> Vec<ToolInfo> {
        self.session.list_all_mcp_tools().await
    }

    async fn codex_apps_auth_snapshot(&self) -> Option<codex_auth_types::RequestAuthSnapshot> {
        self.turn_view().auth_snapshot().await
    }

    async fn cached_accessible_connectors<'a>(
        &'a self,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> Option<Vec<codex_connectors_api::AppInfo>> {
        self.turn_view()
            .cached_accessible_connectors_from_mcp_tools(auth_snapshot)
            .await
    }

    async fn fetch_accessible_connectors<'a>(
        &'a self,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> anyhow::Result<Vec<codex_connectors_api::AppInfo>> {
        self.session
            .fetch_accessible_connectors_from_mcp_tools(self.turn_view(), auth_snapshot)
            .await
    }
}

impl McpToolApprovalPersistenceHost for ServiceMcpHost {
    async fn remember_mcp_tool_approval(&self, key: McpToolApprovalKey) {
        self.session.remember_mcp_tool_approval(key).await;
    }

    async fn persist_codex_app_tool_approval(
        &self,
        connector_id: String,
        tool_name: String,
    ) -> anyhow::Result<()> {
        self.session
            .persist_codex_app_tool_approval_for_turn(self.turn_view(), connector_id, tool_name)
            .await
    }

    async fn persist_non_app_mcp_tool_approval(
        &self,
        server: String,
        tool_name: String,
    ) -> anyhow::Result<()> {
        self.session
            .persist_non_app_mcp_tool_approval_for_turn(self.turn_view(), server, tool_name)
            .await
    }

    async fn reload_user_config_layer(&self) {
        self.session.reload_user_config_layer().await;
    }
}

impl McpToolApprovalReviewHost for ServiceMcpHost {
    async fn mcp_tool_approval_is_remembered(&self, key: &McpToolApprovalKey) -> bool {
        self.session.mcp_tool_approval_is_remembered(key).await
    }

    async fn monitor_auto_approved_mcp_tool_call(
        &self,
        action: serde_json::Value,
        callsite_mode: &'static str,
    ) -> McpToolApprovalMonitorOutcome {
        match self
            .session
            .monitor_auto_approved_action(self.turn_view(), action, callsite_mode)
            .await
        {
            AutoApprovalSafetyOutcome::Ok => McpToolApprovalMonitorOutcome::Ok,
            AutoApprovalSafetyOutcome::AskUser(reason) => {
                McpToolApprovalMonitorOutcome::AskUser(reason)
            }
            AutoApprovalSafetyOutcome::SteerModel(reason) => {
                McpToolApprovalMonitorOutcome::SteerModel(reason)
            }
        }
    }

    async fn request_permission_hook(
        &self,
        call_id: &str,
        hook_tool_name: &str,
        tool_input: serde_json::Value,
    ) -> Option<McpToolApprovalHookDecision> {
        let Some(approval_session) = self.approval_session.as_ref() else {
            return None;
        };
        match approval_session
            .run_permission_request_hooks(
                self.turn_view(),
                call_id,
                PermissionRequestPayload {
                    tool_name: HookToolName::new(hook_tool_name),
                    tool_input,
                },
            )
            .await
        {
            Some(PermissionRequestDecision::Allow) => Some(McpToolApprovalHookDecision::Allow),
            Some(PermissionRequestDecision::Deny { message }) => {
                Some(McpToolApprovalHookDecision::Deny { message })
            }
            None => None,
        }
    }

    async fn review_guardian_mcp_tool_approval(
        &self,
        request: codex_guardian::GuardianApprovalRequest,
        monitor_reason: Option<String>,
    ) -> (ReviewDecision, Option<String>) {
        let result = self
            .approval_api
            .review_guardian_request(GuardianReviewDispatch {
                session: Arc::clone(
                    self.approval_session
                        .as_ref()
                        .expect("approval session required for guardian review"),
                ),
                turn: Arc::clone(&self.turn),
                review_id: uuid::Uuid::new_v4().to_string(),
                request,
                retry_reason: monitor_reason,
                approval_request_source:
                    codex_analytics_api::GuardianApprovalRequestSource::MainTurn,
                cancellation_token: None,
            })
            .await;
        (result.decision, result.decline_message)
    }

    async fn request_mcp_tool_approval_elicitation(
        &self,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> Option<ElicitationResponse> {
        self.session
            .request_mcp_server_elicitation(self.turn_view(), request_id, params)
            .await
    }

    async fn request_user_mcp_tool_approval(
        &self,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> Option<RequestUserInputResponse> {
        self.turn_view().request_user_input(call_id, args).await
    }
}

impl McpToolCallHost for ServiceMcpHost {
    fn codex_app_tool_policy(
        &self,
        metadata: Option<&McpToolApprovalMetadata>,
        tool_name: &str,
    ) -> AppToolPolicy {
        let policy = self.turn_view().codex_app_tool_policy(metadata, tool_name);
        AppToolPolicy {
            enabled: policy.enabled,
            approval: policy.approval,
        }
    }

    fn custom_mcp_tool_approval_mode(
        &self,
        server: &str,
        tool_name: &str,
    ) -> impl std::future::Future<Output = codex_config_types::AppToolApproval> + Send {
        let session = Arc::clone(&self.session);
        let turn = Arc::clone(&self.turn);
        let server = server.to_string();
        let tool_name = tool_name.to_string();
        async move {
            session
                .custom_mcp_tool_approval_mode(turn.as_ref(), &server, &tool_name)
                .await
        }
    }

    fn is_host_owned_codex_apps_server(
        &self,
        server: &str,
    ) -> impl std::future::Future<Output = bool> + Send {
        let session = Arc::clone(&self.session);
        let server = server.to_string();
        async move { session.mcp_server_is_host_owned_codex_apps(&server).await }
    }

    fn request_mcp_tool_approval(
        &self,
        call_id: &str,
        invocation: &McpInvocation,
        hook_tool_name: &str,
        metadata: Option<&McpToolApprovalMetadata>,
        approval_mode: codex_config_types::AppToolApproval,
    ) -> impl std::future::Future<Output = Option<McpToolApprovalDecision>> + Send {
        let session = Arc::clone(&self.session);
        let approval_session = self.approval_session.clone();
        let turn = Arc::clone(&self.turn);
        let approval_api = Arc::clone(&self.approval_api);
        let call_id = call_id.to_string();
        let invocation = invocation.clone();
        let hook_tool_name = hook_tool_name.to_string();
        let metadata = metadata.cloned();
        async move {
            let host = ServiceMcpHost {
                approval_api,
                session,
                approval_session,
                turn,
            };
            let thread_id = host.session.conversation_id().to_string();
            let turn_id = host.turn.runtime_turn_id().to_string();
            let permission_profile = host.turn_view().permission_profile();
            let review_context = McpToolApprovalReviewContext {
                approval_policy: host.turn_view().approval_policy(),
                permission_profile: &permission_profile,
                approvals_reviewer: host.turn_view().approvals_reviewer(),
                approval_mode,
                tool_call_mcp_elicitation_enabled: host
                    .turn_view()
                    .tool_call_mcp_elicitation_enabled(),
                routes_approval_to_guardian: host.turn.routes_approval_to_guardian(),
                thread_id: &thread_id,
                turn_id: Some(&turn_id),
                call_id: &call_id,
                invocation: &invocation,
                hook_tool_name: &hook_tool_name,
                metadata: metadata.as_ref(),
            };
            maybe_request_mcp_tool_approval(&host, review_context).await
        }
    }

    fn emit_mcp_tool_call_started(
        &self,
        item: TurnItem,
    ) -> impl std::future::Future<Output = ()> + Send {
        let session = Arc::clone(&self.session);
        let turn = Arc::clone(&self.turn);
        async move {
            session.emit_turn_item_started(turn.as_ref(), &item).await;
        }
    }

    fn emit_mcp_call_count_status_only(&self, status: &str) {
        self.turn_view().tool_dispatch_telemetry().counter(
            MCP_CALL_COUNT_METRIC,
            1,
            &[("status", status)],
        );
    }

    fn emit_mcp_call_count_with_tags(
        &self,
        status: &str,
        tool_name: &str,
        connector_id: Option<&str>,
        connector_name: Option<&str>,
    ) {
        let tags = mcp_call_metric_tags(status, tool_name, connector_id, connector_name);
        let tag_refs: Vec<(&str, &str)> = tags
            .iter()
            .map(|(key, value)| (*key, value.as_str()))
            .collect();
        self.turn_view()
            .tool_dispatch_telemetry()
            .counter(MCP_CALL_COUNT_METRIC, 1, &tag_refs);
    }
}

async fn emit_mcp_resource_started(
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadRuntimeCapability,
    call_id: &str,
    invocation: &McpInvocation,
) {
    let item = build_mcp_tool_call_started_item(call_id, invocation.clone(), None);
    session.emit_turn_item_started(turn, &item).await;
}

async fn emit_mcp_resource_completed<T>(
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadRuntimeCapability,
    call_id: &str,
    invocation: McpInvocation,
    duration: std::time::Duration,
    result: &Result<T, String>,
) {
    let completed = match result {
        Ok(_) => Ok(ok_call_tool_result()),
        Err(err) => Err(err.clone()),
    };
    session
        .emit_turn_item_completed(
            turn,
            build_mcp_tool_call_completed_item(call_id, invocation, None, duration, completed),
        )
        .await;
}

fn ok_call_tool_result() -> CallToolResult {
    CallToolResult {
        content: vec![serde_json::json!({
            "type": "text",
            "text": "ok",
        })],
        structured_content: None,
        is_error: Some(false),
        meta: None,
    }
}
