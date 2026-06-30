use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use codex_approval_service_api::ApprovalServiceApi;
use codex_approval_service_api::GuardianReviewDispatch;
use codex_hooks_api::PermissionRequestDecision;
use codex_mcp_tool_types::ToolInfo;
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::McpServerElicitationRequestParams;
use codex_mcp_types::McpToolApprovalDecision;
use codex_mcp_types::McpToolApprovalKey;
use codex_mcp_types::McpToolApprovalMetadata;
use codex_protocol::items::TurnItem;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::ListResourceTemplatesResult;
use codex_protocol::mcp::ListResourcesResult;
use codex_protocol::mcp::PaginatedRequestParams;
use codex_protocol::mcp::ReadResourceRequestParams;
use codex_protocol::mcp::ReadResourceResult;
use codex_protocol::mcp::RequestId;
use codex_protocol::mcp::Resource;
use codex_protocol::mcp::ResourceTemplate;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use mcp_service_api::McpRuntimeFuture;
use mcp_service_api::McpServiceApi;
use mcp_service_api::McpToolCallOutcome;
use thread_service_api::AutoApprovalSafetyOutcome;
use thread_service_api::HookToolName;
use thread_service_api::PermissionRequestPayload;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;

use crate::AppToolPolicy;
use crate::CodexAppsAuthElicitationHost;
use crate::MCP_CALL_COUNT_METRIC;
use crate::MCP_CALL_DURATION_METRIC;
use crate::McpApprovedToolCallLifecycleHost;
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
use crate::mcp_call_metric_tags;
use crate::maybe_request_mcp_tool_approval;

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
    turn: Arc<dyn ThreadRuntimeCapability>,
}

impl ServiceMcpHost {
    fn turn_view(&self) -> &dyn ThreadTurnCapability {
        self.turn.as_ref()
    }
}

impl McpServiceApi for McpService {
    fn request_server_elicitation<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> McpRuntimeFuture<'a, Option<ElicitationResponse>> {
        Box::pin(async move { session.request_mcp_server_elicitation(turn, request_id, params).await })
    }

    fn resolve_elicitation<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        server_name: String,
        request_id: RequestId,
        response: ElicitationResponse,
    ) -> McpRuntimeFuture<'a, Result<(), String>> {
        Box::pin(async move { session.resolve_mcp_elicitation(server_name, request_id, response).await })
    }

    fn refresh_servers_if_requested<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        elicitation_reviewer: Option<codex_mcp_types::ElicitationReviewerHandle>,
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
        refresh_config: codex_protocol::protocol::McpServerRefreshConfig,
    ) -> McpRuntimeFuture<'a, ()> {
        Box::pin(async move { session.queue_mcp_server_refresh(refresh_config).await })
    }

    fn refresh_servers_now<'a>(
        &self,
        session: &'a dyn ThreadSessionCapability,
        turn: &'a dyn ThreadTurnCapability,
        refresh_config: codex_protocol::protocol::McpServerRefreshConfig,
        elicitation_reviewer: Option<codex_mcp_types::ElicitationReviewerHandle>,
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
            turn,
        };
        Box::pin(async move { crate::lookup_mcp_tool_metadata(&host, server, tool_name).await })
    }

    fn call_tool<'a>(
        &self,
        session: Arc<dyn ThreadSessionCapability>,
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
            emit_mcp_resource_completed(session.as_ref(), turn.as_ref(), &call_id, invocation, start.elapsed(), &result).await;
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
            emit_mcp_resource_completed(session.as_ref(), turn.as_ref(), &call_id, invocation, start.elapsed(), &result).await;
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
            emit_mcp_resource_completed(session.as_ref(), turn.as_ref(), &call_id, invocation, start.elapsed(), &result).await;
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
        self.session.call_mcp_tool(server, tool, arguments, meta).await
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
        self.turn_view()
            .tool_dispatch_telemetry()
            .record_duration(MCP_CALL_DURATION_METRIC, duration, &tag_refs);
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
    ) -> Option<Vec<codex_connectors_types::AppInfo>> {
        self.turn_view()
            .cached_accessible_connectors_from_mcp_tools(auth_snapshot)
            .await
    }

    async fn fetch_accessible_connectors<'a>(
        &'a self,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> anyhow::Result<Vec<codex_connectors_types::AppInfo>> {
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
        match self
            .session
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
                session: Arc::clone(&self.session),
                turn: Arc::clone(&self.turn),
                review_id: uuid::Uuid::new_v4().to_string(),
                request,
                retry_reason: monitor_reason,
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
        self.turn_view()
            .tool_dispatch_telemetry()
            .counter(MCP_CALL_COUNT_METRIC, 1, &[("status", status)]);
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
    session
        .emit_turn_item_started(turn, &item)
        .await;
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
