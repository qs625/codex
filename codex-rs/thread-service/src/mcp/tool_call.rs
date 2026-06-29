use std::time::Duration;

use crate::arc_monitor::monitor_action;
use crate::client::X_CODEX_TURN_METADATA_HEADER;
use crate::mcp::openai_file::rewrite_mcp_tool_arguments_for_openai_files;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tool_approval_support::PermissionRequestPayload;
use crate::tool_approval_support::permission_request_hook_payload;
use codex_auth_types::RequestAuthSnapshot;
use codex_config_types::AppToolApproval;
use codex_connectors_types::AppInfo;
use codex_hooks::run_permission_request_hooks;
use codex_hooks_api::PermissionRequestDecision;
#[cfg(test)]
use codex_mcp_tool_types::ToolAnnotations;
use codex_mcp_tool_types::ToolInfo;
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::McpServerElicitationRequestParams;
use codex_mcp_types::McpToolApprovalDecision;
use codex_mcp_types::McpToolApprovalKey;
use codex_mcp_types::McpToolApprovalMetadata;
#[cfg(test)]
use codex_mcp_types::session_mcp_tool_approval_key;
use codex_protocol::items::McpToolCallError;
use codex_protocol::items::McpToolCallItem;
use codex_protocol::items::McpToolCallStatus;
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
use thread_service_api::HookToolName;
use mcp_service::AppToolPolicy;
#[cfg(test)]
use mcp_service::CodexAppsAuthElicitationContext;
use mcp_service::CodexAppsAuthElicitationHost;
use mcp_service::MCP_CALL_COUNT_METRIC;
use mcp_service::MCP_CALL_DURATION_METRIC;
use mcp_service::McpApprovedToolCallLifecycleHost;
use mcp_service::McpToolApprovalHookDecision;
use mcp_service::McpToolApprovalMonitorOutcome;
use mcp_service::McpToolApprovalPersistenceHost;
use mcp_service::McpToolApprovalReviewHost;
use mcp_service::McpToolCallContext;
use mcp_service::McpToolCallHost;
#[cfg(test)]
use mcp_service::McpToolExecutionContext;
use mcp_service::McpToolExecutionHost;
use mcp_service::McpToolMetadataLookupHost;
#[cfg(test)]
use mcp_service::execute_mcp_tool_call as execute_mcp_tool_call_with_host;
use mcp_service::handle_mcp_tool_call as handle_mcp_tool_call_with_host;
use mcp_service::insert_sandbox_state_request_meta;
use mcp_service::lookup_mcp_tool_metadata as lookup_mcp_tool_metadata_with_host;
#[cfg(test)]
use mcp_service::maybe_persist_mcp_tool_approval as maybe_persist_mcp_tool_approval_with_host;
#[cfg(test)]
use mcp_service::maybe_request_codex_apps_auth_elicitation as maybe_request_codex_apps_auth_elicitation_with_host;
use mcp_service::maybe_request_mcp_tool_approval as maybe_request_mcp_tool_approval_with_host;
use mcp_service::mcp_call_metric_tags;
use serde_json::Value as JsonValue;
use std::sync::Arc;

/// Handles the specified tool call and dispatches the appropriate MCP tool-call
/// item lifecycle events to the `Session`.
pub(crate) async fn handle_mcp_tool_call(
    sess: Arc<Session>,
    turn_context: &TurnContext,
    call_id: String,
    server: String,
    tool_name: String,
    hook_tool_name: String,
    arguments: String,
) -> HandledMcpToolCall {
    let thread_id = sess.thread_id_string();
    let host = SessionMcpHost {
        sess: sess.as_ref(),
        turn_context,
    };
    let outcome = handle_mcp_tool_call_with_host(
        &host,
        McpToolCallContext {
            thread_id,
            turn_id: turn_context.turn_id(),
            call_id,
            server,
            tool_name,
            hook_tool_name,
            arguments,
            turn_metadata: turn_context.mcp_turn_metadata_value(),
            turn_metadata_header_name: X_CODEX_TURN_METADATA_HEADER,
            supports_image_input: turn_context.supports_image_input(),
            auth_elicitation_enabled: turn_context.auth_elicitation_enabled(),
            approval_policy: turn_context.approval_policy(),
        },
    )
    .await;
    HandledMcpToolCall {
        result: outcome.result,
        tool_input: outcome.tool_input,
    }
}

pub(crate) struct HandledMcpToolCall {
    pub(crate) result: CallToolResult,
    pub(crate) tool_input: JsonValue,
}

pub(crate) async fn call_mcp_tool_via_turn(
    turn: &TurnContext,
    call_id: String,
    server: String,
    tool_name: String,
    hook_tool_name: String,
    arguments: String,
) -> (CallToolResult, JsonValue) {
    let session = turn.session_arc();
    let handled = handle_mcp_tool_call(
        session,
        turn,
        call_id,
        server,
        tool_name,
        hook_tool_name,
        arguments,
    )
    .await;
    (handled.result, handled.tool_input)
}

pub(crate) async fn list_resources_via_turn(
    turn: &TurnContext,
    server: &str,
    params: Option<PaginatedRequestParams>,
) -> Result<ListResourcesResult, String> {
    turn.session_arc()
        .list_resources(server, params)
        .await
        .map_err(|err| format!("{err:#}"))
}

pub(crate) async fn list_all_resources_via_turn(
    turn: &TurnContext,
) -> std::collections::HashMap<String, Vec<Resource>> {
    turn.session_arc().list_all_resources().await
}

pub(crate) async fn list_resource_templates_via_turn(
    turn: &TurnContext,
    server: &str,
    params: Option<PaginatedRequestParams>,
) -> Result<ListResourceTemplatesResult, String> {
    turn.session_arc()
        .list_resource_templates(server, params)
        .await
        .map_err(|err| format!("{err:#}"))
}

pub(crate) async fn list_all_resource_templates_via_turn(
    turn: &TurnContext,
) -> std::collections::HashMap<String, Vec<ResourceTemplate>> {
    turn.session_arc().list_all_resource_templates().await
}

pub(crate) async fn read_resource_via_turn(
    turn: &TurnContext,
    server: &str,
    params: ReadResourceRequestParams,
) -> Result<ReadResourceResult, String> {
    turn.session_arc()
        .read_resource(server, params)
        .await
        .map_err(|err| format!("{err:#}"))
}

pub(crate) async fn emit_mcp_resource_tool_call_begin_via_turn(
    turn: &TurnContext,
    call_id: &str,
    invocation: McpInvocation,
) {
    let session = turn.session_arc();
    let item = mcp_resource_item_started(call_id.to_string(), invocation);
    session.emit_turn_item_started(turn, &item).await;
}

pub(crate) async fn emit_mcp_resource_tool_call_end_via_turn(
    turn: &TurnContext,
    call_id: &str,
    invocation: McpInvocation,
    duration: Duration,
    result: Result<CallToolResult, String>,
) {
    let session = turn.session_arc();
    let item = mcp_resource_item_completed(call_id.to_string(), invocation, duration, result);
    session.emit_turn_item_completed(turn, item).await;
}

fn mcp_resource_item_started(call_id: String, invocation: McpInvocation) -> TurnItem {
    let McpInvocation {
        server,
        tool,
        arguments,
    } = invocation;
    TurnItem::McpToolCall(McpToolCallItem {
        id: call_id,
        server,
        tool,
        arguments: arguments.unwrap_or(JsonValue::Null),
        mcp_app_resource_uri: None,
        status: McpToolCallStatus::InProgress,
        result: None,
        error: None,
        duration: None,
    })
}

fn mcp_resource_item_completed(
    call_id: String,
    invocation: McpInvocation,
    duration: Duration,
    result: Result<CallToolResult, String>,
) -> TurnItem {
    let (status, result, error) = match result {
        Ok(result) if result.is_error.unwrap_or(false) => {
            (McpToolCallStatus::Failed, Some(result), None)
        }
        Ok(result) => (McpToolCallStatus::Completed, Some(result), None),
        Err(message) => (
            McpToolCallStatus::Failed,
            None,
            Some(McpToolCallError { message }),
        ),
    };
    let McpInvocation {
        server,
        tool,
        arguments,
    } = invocation;
    TurnItem::McpToolCall(McpToolCallItem {
        id: call_id,
        server,
        tool,
        arguments: arguments.unwrap_or(JsonValue::Null),
        mcp_app_resource_uri: None,
        status,
        result,
        error,
        duration: Some(duration),
    })
}

fn emit_mcp_call_metrics(
    turn_context: &TurnContext,
    status: &str,
    tool_name: &str,
    connector_id: Option<&str>,
    connector_name: Option<&str>,
    duration: Option<Duration>,
) {
    let tags = mcp_call_metric_tags(status, tool_name, connector_id, connector_name);
    let tag_refs: Vec<(&str, &str)> = tags
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    turn_context
        .session_telemetry
        .counter(MCP_CALL_COUNT_METRIC, /*inc*/ 1, &tag_refs);
    if let Some(duration) = duration {
        turn_context.session_telemetry.record_duration(
            MCP_CALL_DURATION_METRIC,
            duration,
            &tag_refs,
        );
    }
}

struct SessionMcpHost<'a> {
    sess: &'a Session,
    turn_context: &'a TurnContext,
}

impl McpToolExecutionHost for SessionMcpHost<'_> {
    async fn augment_mcp_tool_request_meta_with_sandbox_state(
        &self,
        server: &str,
        meta: Option<JsonValue>,
    ) -> anyhow::Result<Option<JsonValue>> {
        augment_mcp_tool_request_meta_with_sandbox_state(self.sess, self.turn_context, server, meta)
            .await
    }

    fn add_mcp_call_trace_request_meta(
        &self,
        call_id: &str,
        meta: Option<JsonValue>,
    ) -> Option<JsonValue> {
        self.sess
            .add_optional_mcp_call_trace_request_meta(call_id, meta)
    }

    async fn call_mcp_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<JsonValue>,
        meta: Option<JsonValue>,
    ) -> Result<CallToolResult, String> {
        self.sess
            .call_tool(server, tool, arguments, meta)
            .await
            .map_err(|error| format!("tool call error: {error:?}"))
    }
}

impl McpApprovedToolCallLifecycleHost for SessionMcpHost<'_> {
    async fn mark_thread_memory_mode_polluted_if_needed(&self, server: &str) {
        maybe_mark_thread_memory_mode_polluted(self.sess, self.turn_context, server).await;
    }

    async fn server_origin(&self, server: &str) -> Option<String> {
        self.sess.mcp_server_origin(server).await
    }

    async fn rewrite_mcp_tool_arguments_for_openai_files(
        &self,
        arguments: Option<JsonValue>,
        openai_file_input_params: Option<&[String]>,
    ) -> Result<Option<JsonValue>, String> {
        rewrite_mcp_tool_arguments_for_openai_files(
            self.sess,
            self.turn_context,
            arguments,
            openai_file_input_params,
        )
        .await
    }

    async fn emit_mcp_tool_call_completed(&self, item: codex_protocol::items::TurnItem) {
        self.sess
            .emit_turn_item_completed(self.turn_context, item)
            .await;
    }

    async fn track_codex_app_used(&self, server: &str, tool_name: &str) {
        maybe_track_codex_app_used(self.sess, self.turn_context, server, tool_name).await;
    }

    fn emit_mcp_call_metrics(
        &self,
        status: &str,
        tool_name: &str,
        connector_id: Option<&str>,
        connector_name: Option<&str>,
        duration: Duration,
    ) {
        emit_mcp_call_metrics(
            self.turn_context,
            status,
            tool_name,
            connector_id,
            connector_name,
            Some(duration),
        );
    }
}

impl CodexAppsAuthElicitationHost for SessionMcpHost<'_> {
    async fn request_codex_apps_auth_elicitation(
        &self,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> Option<ElicitationResponse> {
        self.sess
            .request_mcp_server_elicitation(self.turn_context, request_id, params)
            .await
    }

    async fn refresh_codex_apps_after_connector_auth(&self) {
        refresh_codex_apps_after_connector_auth(self.sess, self.turn_context).await;
    }
}

impl McpToolCallHost for SessionMcpHost<'_> {
    fn codex_app_tool_policy(
        &self,
        metadata: Option<&McpToolApprovalMetadata>,
        tool_name: &str,
    ) -> AppToolPolicy {
        self.turn_context.codex_app_tool_policy(metadata, tool_name)
    }

    async fn custom_mcp_tool_approval_mode(
        &self,
        server: &str,
        tool_name: &str,
    ) -> AppToolApproval {
        self.sess
            .custom_mcp_tool_approval_mode(self.turn_context, server, tool_name)
            .await
    }

    async fn is_host_owned_codex_apps_server(&self, server: &str) -> bool {
        self.sess.mcp_server_is_host_owned_codex_apps(server).await
    }

    async fn request_mcp_tool_approval(
        &self,
        call_id: &str,
        invocation: &McpInvocation,
        hook_tool_name: &str,
        metadata: Option<&McpToolApprovalMetadata>,
        approval_mode: AppToolApproval,
    ) -> Option<McpToolApprovalDecision> {
        maybe_request_mcp_tool_approval(
            &session_arc(self.sess),
            self.turn_context,
            call_id,
            invocation,
            hook_tool_name,
            metadata,
            approval_mode,
        )
        .await
    }

    async fn emit_mcp_tool_call_started(&self, item: codex_protocol::items::TurnItem) {
        self.sess
            .emit_turn_item_started(self.turn_context, &item)
            .await;
    }

    fn emit_mcp_call_count_status_only(&self, status: &str) {
        self.turn_context.session_telemetry.counter(
            MCP_CALL_COUNT_METRIC,
            /*inc*/ 1,
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
        emit_mcp_call_metrics(
            self.turn_context,
            status,
            tool_name,
            connector_id,
            connector_name,
            /*duration*/ None,
        );
    }
}

#[cfg(test)]
async fn execute_mcp_tool_call(
    sess: &Session,
    turn_context: &TurnContext,
    call_id: &str,
    invocation: &McpInvocation,
    rewritten_arguments: Option<JsonValue>,
    metadata: Option<&McpToolApprovalMetadata>,
    turn_metadata: Option<JsonValue>,
) -> Result<CallToolResult, String> {
    let server = invocation.server.clone();
    let is_host_owned_codex_apps_server = sess.mcp_server_is_host_owned_codex_apps(&server).await;
    let thread_id = sess.thread_id_string();
    let host = SessionMcpHost { sess, turn_context };
    execute_mcp_tool_call_with_host(
        &host,
        McpToolExecutionContext {
            thread_id: &thread_id,
            call_id,
            invocation,
            rewritten_arguments,
            metadata,
            turn_metadata,
            turn_metadata_header_name: X_CODEX_TURN_METADATA_HEADER,
            supports_image_input: turn_context.supports_image_input(),
            auth_elicitation_context: CodexAppsAuthElicitationContext {
                is_host_owned_codex_apps_server,
                auth_elicitation_enabled: turn_context.auth_elicitation_enabled(),
                approval_policy: turn_context.approval_policy(),
                thread_id: &thread_id,
                turn_id: Some(turn_context.turn_id_str()),
                call_id,
                metadata,
            },
        },
    )
    .await
}

#[cfg(test)]
async fn maybe_request_codex_apps_auth_elicitation(
    sess: &Session,
    turn_context: &TurnContext,
    call_id: &str,
    server: &str,
    metadata: Option<&McpToolApprovalMetadata>,
    result: CallToolResult,
) -> CallToolResult {
    let is_host_owned_codex_apps_server = sess.mcp_server_is_host_owned_codex_apps(server).await;

    let thread_id = sess.thread_id_string();
    let host = SessionMcpHost { sess, turn_context };
    maybe_request_codex_apps_auth_elicitation_with_host(
        &host,
        CodexAppsAuthElicitationContext {
            is_host_owned_codex_apps_server,
            auth_elicitation_enabled: turn_context.auth_elicitation_enabled(),
            approval_policy: turn_context.approval_policy(),
            thread_id: &thread_id,
            turn_id: Some(turn_context.turn_id_str()),
            call_id,
            metadata,
        },
        result,
    )
    .await
}

#[allow(
    clippy::await_holding_invalid_type,
    reason = "Codex Apps cache refresh reads through the session-owned manager guard"
)]
async fn refresh_codex_apps_after_connector_auth(sess: &Session, turn_context: &TurnContext) {
    let mcp_tools_result = sess.hard_refresh_codex_apps_tools_cache().await;

    match mcp_tools_result {
        Ok(mcp_tools) => {
            let auth_snapshot = turn_context.auth_snapshot().await;
            let connector_auth_context =
                crate::mcp::codex_apps_auth_context(auth_snapshot.as_ref());
            turn_context.refresh_accessible_connectors_cache_from_mcp_tools(
                connector_auth_context.as_ref(),
                &mcp_tools,
            );
        }
        Err(err) => {
            tracing::warn!("failed to refresh Codex Apps tools after connector auth: {err:#}");
        }
    }
}

#[allow(
    clippy::await_holding_invalid_type,
    reason = "MCP sandbox metadata reads through the session-owned manager guard"
)]
async fn augment_mcp_tool_request_meta_with_sandbox_state(
    sess: &Session,
    turn_context: &TurnContext,
    server: &str,
    meta: Option<serde_json::Value>,
) -> anyhow::Result<Option<serde_json::Value>> {
    if !sess.mcp_server_supports_sandbox_state_meta(server).await {
        return Ok(meta);
    }

    insert_sandbox_state_request_meta(meta, turn_context.mcp_sandbox_state())
}

async fn maybe_mark_thread_memory_mode_polluted(
    sess: &Session,
    turn_context: &TurnContext,
    server: &str,
) {
    sess.mark_thread_memory_mode_polluted_for_mcp_tool_call(turn_context, server)
        .await;
}

async fn maybe_track_codex_app_used(
    sess: &Session,
    turn_context: &TurnContext,
    server: &str,
    tool_name: &str,
) {
    sess.track_codex_app_used_for_mcp_tool(turn_context, server, tool_name)
        .await;
}

fn session_arc(session: &Session) -> Arc<Session> {
    match session.self_weak.get().and_then(std::sync::Weak::upgrade) {
        Some(session) => session,
        None => panic!("Session self_weak must be initialized"),
    }
}

async fn maybe_request_mcp_tool_approval(
    sess: &Arc<Session>,
    turn_context: &TurnContext,
    call_id: &str,
    invocation: &McpInvocation,
    hook_tool_name: &str,
    metadata: Option<&McpToolApprovalMetadata>,
    approval_mode: AppToolApproval,
) -> Option<McpToolApprovalDecision> {
    let host = SessionMcpHost {
        sess: sess.as_ref(),
        turn_context,
    };
    let thread_id = sess.thread_id_string();
    let routes_approval_to_guardian = approval_service::guardian::routes_approval_to_guardian(
        &turn_context.approval_policy.value(),
        turn_context.config.approvals_reviewer,
    );
    let review_context = turn_context.mcp_approval_review_context(
        &thread_id,
        call_id,
        invocation,
        hook_tool_name,
        metadata,
        approval_mode,
        routes_approval_to_guardian,
    );
    maybe_request_mcp_tool_approval_with_host(&host, review_context).await
}

pub(crate) async fn lookup_mcp_tool_metadata(
    sess: &Session,
    turn_context: &TurnContext,
    server: &str,
    tool_name: &str,
) -> Option<McpToolApprovalMetadata> {
    let host = SessionMcpHost { sess, turn_context };
    lookup_mcp_tool_metadata_with_host(&host, server, tool_name).await
}

impl McpToolMetadataLookupHost for SessionMcpHost<'_> {
    #[allow(
        clippy::await_holding_invalid_type,
        reason = "MCP approval metadata reads through the session-owned manager guard"
    )]
    async fn list_all_mcp_tools(&self) -> Vec<ToolInfo> {
        self.sess.list_all_mcp_tools().await
    }

    async fn codex_apps_auth_snapshot(&self) -> Option<RequestAuthSnapshot> {
        self.turn_context.auth_snapshot().await
    }

    async fn cached_accessible_connectors(
        &self,
        auth_snapshot: Option<&RequestAuthSnapshot>,
    ) -> Option<Vec<AppInfo>> {
        self.turn_context
            .cached_accessible_connectors_from_mcp_tools(auth_snapshot)
            .await
    }

    async fn fetch_accessible_connectors(
        &self,
        auth_snapshot: Option<&RequestAuthSnapshot>,
    ) -> anyhow::Result<Vec<AppInfo>> {
        self.sess
            .fetch_accessible_connectors_from_mcp_tools(self.turn_context, auth_snapshot)
            .await
    }
}

async fn mcp_tool_approval_is_remembered(sess: &Session, key: &McpToolApprovalKey) -> bool {
    sess.mcp_tool_approval_is_remembered(key).await
}

async fn remember_mcp_tool_approval(sess: &Session, key: McpToolApprovalKey) {
    sess.remember_mcp_tool_approval(key).await;
}

impl McpToolApprovalPersistenceHost for SessionMcpHost<'_> {
    async fn remember_mcp_tool_approval(&self, key: McpToolApprovalKey) {
        remember_mcp_tool_approval(self.sess, key).await;
    }

    async fn persist_codex_app_tool_approval(
        &self,
        connector_id: String,
        tool_name: String,
    ) -> anyhow::Result<()> {
        self.sess
            .persist_codex_app_tool_approval_for_turn(self.turn_context, &connector_id, &tool_name)
            .await
    }

    async fn persist_non_app_mcp_tool_approval(
        &self,
        server: String,
        tool_name: String,
    ) -> anyhow::Result<()> {
        self.sess
            .persist_non_app_mcp_tool_approval_for_turn(self.turn_context, &server, &tool_name)
            .await
    }

    async fn reload_user_config_layer(&self) {
        self.sess.reload_user_config_layer().await;
    }
}

impl McpToolApprovalReviewHost for SessionMcpHost<'_> {
    async fn mcp_tool_approval_is_remembered(&self, key: &McpToolApprovalKey) -> bool {
        mcp_tool_approval_is_remembered(self.sess, key).await
    }

    async fn monitor_auto_approved_mcp_tool_call(
        &self,
        action: JsonValue,
        callsite_mode: &'static str,
    ) -> McpToolApprovalMonitorOutcome {
        match monitor_action(self.sess, self.turn_context, action, callsite_mode).await {
            crate::arc_monitor::ArcMonitorOutcome::Ok => McpToolApprovalMonitorOutcome::Ok,
            crate::arc_monitor::ArcMonitorOutcome::AskUser(reason) => {
                McpToolApprovalMonitorOutcome::AskUser(reason)
            }
            crate::arc_monitor::ArcMonitorOutcome::SteerModel(reason) => {
                McpToolApprovalMonitorOutcome::SteerModel(reason)
            }
        }
    }

    async fn request_permission_hook(
        &self,
        call_id: &str,
        hook_tool_name: &str,
        tool_input: JsonValue,
    ) -> Option<McpToolApprovalHookDecision> {
        match run_permission_request_hooks(
            self.sess,
            self.turn_context,
            call_id,
            permission_request_hook_payload(PermissionRequestPayload {
                tool_name: HookToolName::new(hook_tool_name),
                tool_input,
            }),
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
        let review_id = approval_service::guardian::new_guardian_review_id();
        let decision = approval_service::guardian::review_approval_request(
            session_arc(self.sess).as_ref(),
            self.turn_context.self_arc().as_ref(),
            review_id.clone(),
            request,
            monitor_reason,
        )
        .await;
        let decline_message = match decision {
            ReviewDecision::Denied => {
                Some(approval_service::guardian::guardian_rejection_message(
                    session_arc(self.sess).as_ref(),
                    &review_id,
                )
                .await)
            }
            ReviewDecision::TimedOut => Some(approval_service::guardian::guardian_timeout_message()),
            ReviewDecision::Approved
            | ReviewDecision::ApprovedForSession
            | ReviewDecision::ApprovedExecpolicyAmendment { .. }
            | ReviewDecision::NetworkPolicyAmendment { .. }
            | ReviewDecision::Abort => None,
        };
        (decision, decline_message)
    }

    async fn request_mcp_tool_approval_elicitation(
        &self,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> Option<ElicitationResponse> {
        self.sess
            .request_mcp_server_elicitation(self.turn_context, request_id, params)
            .await
    }

    async fn request_user_mcp_tool_approval(
        &self,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> Option<RequestUserInputResponse> {
        self.sess
            .request_user_input(self.turn_context, call_id, args)
            .await
    }
}

#[cfg(test)]
async fn maybe_persist_mcp_tool_approval(
    sess: &Session,
    turn_context: &TurnContext,
    key: McpToolApprovalKey,
) {
    let host = SessionMcpHost { sess, turn_context };
    maybe_persist_mcp_tool_approval_with_host(&host, key).await;
}

#[cfg(test)]
#[path = "tool_call_tests.rs"]
mod tests;
