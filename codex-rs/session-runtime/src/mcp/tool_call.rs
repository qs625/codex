use std::time::Duration;

use crate::arc_monitor::monitor_action;
use crate::client::X_CODEX_TURN_METADATA_HEADER;
use crate::guardian::guardian_rejection_message;
use crate::guardian::guardian_timeout_message;
use crate::guardian::new_guardian_review_id;
use crate::guardian::review_approval_request;
use crate::guardian::routes_approval_to_guardian;
use crate::mcp::openai_file::rewrite_mcp_tool_arguments_for_openai_files;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::sandboxing::PermissionRequestPayload;
use crate::tools::sandboxing::permission_request_hook_payload;
use codex_auth_types::RequestAuthSnapshot;
use codex_config_types::AppToolApproval;
use codex_connectors_types::AppInfo;
use codex_hooks::run_permission_request_hooks;
use codex_hooks_api::PermissionRequestDecision;
use codex_mcp_runtime::AppToolPolicy;
#[cfg(test)]
use codex_mcp_runtime::CodexAppsAuthElicitationContext;
use codex_mcp_runtime::CodexAppsAuthElicitationHost;
use codex_mcp_runtime::MCP_CALL_COUNT_METRIC;
use codex_mcp_runtime::MCP_CALL_DURATION_METRIC;
use codex_mcp_runtime::McpApprovedToolCallLifecycleHost;
use codex_mcp_runtime::McpToolApprovalHookDecision;
use codex_mcp_runtime::McpToolApprovalMonitorOutcome;
use codex_mcp_runtime::McpToolApprovalPersistenceHost;
use codex_mcp_runtime::McpToolApprovalReviewHost;
use codex_mcp_runtime::McpToolCallContext;
use codex_mcp_runtime::McpToolCallHost;
#[cfg(test)]
use codex_mcp_runtime::McpToolExecutionContext;
use codex_mcp_runtime::McpToolExecutionHost;
use codex_mcp_runtime::McpToolMetadataLookupHost;
#[cfg(test)]
use codex_mcp_runtime::execute_mcp_tool_call as execute_mcp_tool_call_with_host;
use codex_mcp_runtime::handle_mcp_tool_call as handle_mcp_tool_call_with_host;
use codex_mcp_runtime::insert_sandbox_state_request_meta;
use codex_mcp_runtime::lookup_mcp_tool_metadata as lookup_mcp_tool_metadata_with_host;
#[cfg(test)]
use codex_mcp_runtime::maybe_persist_mcp_tool_approval as maybe_persist_mcp_tool_approval_with_host;
#[cfg(test)]
use codex_mcp_runtime::maybe_request_codex_apps_auth_elicitation as maybe_request_codex_apps_auth_elicitation_with_host;
use codex_mcp_runtime::maybe_request_mcp_tool_approval as maybe_request_mcp_tool_approval_with_host;
use codex_mcp_runtime::mcp_call_metric_tags;
#[cfg(test)]
use codex_mcp_tool_types::ToolAnnotations;
use codex_mcp_tool_types::ToolInfo;
use codex_mcp_types::ElicitationResponse;
pub(crate) use codex_mcp_types::MCP_TOOL_APPROVAL_ACCEPT;
pub(crate) use codex_mcp_types::MCP_TOOL_APPROVAL_ACCEPT_FOR_SESSION;
pub(crate) use codex_mcp_types::MCP_TOOL_APPROVAL_DECLINE_SYNTHETIC;
#[cfg(test)]
pub(crate) use codex_mcp_types::MCP_TOOL_APPROVAL_QUESTION_ID_PREFIX;
use codex_mcp_types::McpServerElicitationRequestParams;
use codex_mcp_types::McpToolApprovalDecision;
use codex_mcp_types::McpToolApprovalKey;
use codex_mcp_types::McpToolApprovalMetadata;
pub(crate) use codex_mcp_types::is_mcp_tool_approval_question_id;
#[cfg(test)]
use codex_mcp_types::session_mcp_tool_approval_key;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::RequestId;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::protocol::ReviewDecision;
use codex_protocol::request_user_input::RequestUserInputArgs;
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_tool_runtime_api::HookToolName;
use serde_json::Value as JsonValue;
use std::sync::Arc;

/// Handles the specified tool call and dispatches the appropriate MCP tool-call
/// item lifecycle events to the `Session`.
pub(crate) async fn handle_mcp_tool_call(
    sess: Arc<Session>,
    turn_context: &Arc<TurnContext>,
    call_id: String,
    server: String,
    tool_name: String,
    hook_tool_name: String,
    arguments: String,
) -> HandledMcpToolCall {
    let thread_id = sess.thread_id_string();
    let host = CoreMcpToolCallHost {
        sess: &sess,
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

struct CoreMcpToolCallHost<'a> {
    sess: &'a Arc<Session>,
    turn_context: &'a Arc<TurnContext>,
}

impl McpToolExecutionHost for CoreMcpToolCallHost<'_> {
    async fn augment_mcp_tool_request_meta_with_sandbox_state(
        &self,
        server: &str,
        meta: Option<JsonValue>,
    ) -> anyhow::Result<Option<JsonValue>> {
        augment_mcp_tool_request_meta_with_sandbox_state(
            self.sess.as_ref(),
            self.turn_context.as_ref(),
            server,
            meta,
        )
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

impl McpApprovedToolCallLifecycleHost for CoreMcpToolCallHost<'_> {
    async fn mark_thread_memory_mode_polluted_if_needed(&self, server: &str) {
        maybe_mark_thread_memory_mode_polluted(
            self.sess.as_ref(),
            self.turn_context.as_ref(),
            server,
        )
        .await;
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
            self.sess.as_ref(),
            self.turn_context.as_ref(),
            arguments,
            openai_file_input_params,
        )
        .await
    }

    async fn emit_mcp_tool_call_completed(&self, item: codex_protocol::items::TurnItem) {
        self.sess
            .emit_turn_item_completed(self.turn_context.as_ref(), item)
            .await;
    }

    async fn track_codex_app_used(&self, server: &str, tool_name: &str) {
        maybe_track_codex_app_used(
            self.sess.as_ref(),
            self.turn_context.as_ref(),
            server,
            tool_name,
        )
        .await;
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
            self.turn_context.as_ref(),
            status,
            tool_name,
            connector_id,
            connector_name,
            Some(duration),
        );
    }
}

impl CodexAppsAuthElicitationHost for CoreMcpToolCallHost<'_> {
    async fn request_codex_apps_auth_elicitation(
        &self,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> Option<ElicitationResponse> {
        self.sess
            .request_mcp_server_elicitation(self.turn_context.as_ref(), request_id, params)
            .await
    }

    async fn refresh_codex_apps_after_connector_auth(&self) {
        refresh_codex_apps_after_connector_auth(self.sess.as_ref(), self.turn_context.as_ref())
            .await;
    }
}

impl McpToolCallHost for CoreMcpToolCallHost<'_> {
    async fn lookup_mcp_tool_metadata(
        &self,
        server: &str,
        tool_name: &str,
    ) -> Option<McpToolApprovalMetadata> {
        lookup_mcp_tool_metadata(
            self.sess.as_ref(),
            self.turn_context.as_ref(),
            server,
            tool_name,
        )
        .await
    }

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
            .custom_mcp_tool_approval_mode(self.turn_context.as_ref(), server, tool_name)
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
            self.sess,
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
            .emit_turn_item_started(self.turn_context.as_ref(), &item)
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
            self.turn_context.as_ref(),
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
    let host = CoreMcpToolExecutionHost { sess, turn_context };
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
struct CoreMcpToolExecutionHost<'a> {
    sess: &'a Session,
    turn_context: &'a TurnContext,
}

#[cfg(test)]
impl McpToolExecutionHost for CoreMcpToolExecutionHost<'_> {
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

#[cfg(test)]
impl CodexAppsAuthElicitationHost for CoreMcpToolExecutionHost<'_> {
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
    let host = CoreCodexAppsAuthElicitationHost { sess, turn_context };
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

#[cfg(test)]
struct CoreCodexAppsAuthElicitationHost<'a> {
    sess: &'a Session,
    turn_context: &'a TurnContext,
}

#[cfg(test)]
impl CodexAppsAuthElicitationHost for CoreCodexAppsAuthElicitationHost<'_> {
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

#[expect(
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

#[expect(
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

async fn maybe_request_mcp_tool_approval(
    sess: &Arc<Session>,
    turn_context: &Arc<TurnContext>,
    call_id: &str,
    invocation: &McpInvocation,
    hook_tool_name: &str,
    metadata: Option<&McpToolApprovalMetadata>,
    approval_mode: AppToolApproval,
) -> Option<McpToolApprovalDecision> {
    let host = CoreMcpToolApprovalReviewHost { sess, turn_context };
    let thread_id = sess.thread_id_string();
    let routes_approval_to_guardian = routes_approval_to_guardian(turn_context);
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
    let host = CoreMcpToolMetadataLookupHost { sess, turn_context };
    lookup_mcp_tool_metadata_with_host(&host, server, tool_name).await
}

struct CoreMcpToolMetadataLookupHost<'a> {
    sess: &'a Session,
    turn_context: &'a TurnContext,
}

impl McpToolMetadataLookupHost for CoreMcpToolMetadataLookupHost<'_> {
    #[expect(
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

struct CoreMcpToolApprovalReviewHost<'a> {
    sess: &'a Arc<Session>,
    turn_context: &'a Arc<TurnContext>,
}

impl McpToolApprovalPersistenceHost for CoreMcpToolApprovalReviewHost<'_> {
    async fn remember_mcp_tool_approval(&self, key: McpToolApprovalKey) {
        remember_mcp_tool_approval(self.sess.as_ref(), key).await;
    }

    async fn persist_codex_app_tool_approval(
        &self,
        connector_id: String,
        tool_name: String,
    ) -> anyhow::Result<()> {
        self.sess
            .persist_codex_app_tool_approval_for_turn(
                self.turn_context.as_ref(),
                &connector_id,
                &tool_name,
            )
            .await
    }

    async fn persist_non_app_mcp_tool_approval(
        &self,
        server: String,
        tool_name: String,
    ) -> anyhow::Result<()> {
        self.sess
            .persist_non_app_mcp_tool_approval_for_turn(
                self.turn_context.as_ref(),
                &server,
                &tool_name,
            )
            .await
    }

    async fn reload_user_config_layer(&self) {
        self.sess.reload_user_config_layer().await;
    }
}

impl McpToolApprovalReviewHost for CoreMcpToolApprovalReviewHost<'_> {
    async fn mcp_tool_approval_is_remembered(&self, key: &McpToolApprovalKey) -> bool {
        mcp_tool_approval_is_remembered(self.sess.as_ref(), key).await
    }

    async fn monitor_auto_approved_mcp_tool_call(
        &self,
        action: JsonValue,
        callsite_mode: &'static str,
    ) -> McpToolApprovalMonitorOutcome {
        match monitor_action(
            self.sess.as_ref(),
            self.turn_context.as_ref(),
            action,
            callsite_mode,
        )
        .await
        {
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
            self.sess.as_ref(),
            self.turn_context.as_ref(),
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
        let review_id = new_guardian_review_id();
        let decision = review_approval_request(
            self.sess,
            self.turn_context,
            review_id.clone(),
            request,
            monitor_reason,
        )
        .await;
        let decline_message = match decision {
            ReviewDecision::Denied => Some(guardian_rejection_message(self.sess, &review_id).await),
            ReviewDecision::TimedOut => Some(guardian_timeout_message()),
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
            .request_mcp_server_elicitation(self.turn_context.as_ref(), request_id, params)
            .await
    }

    async fn request_user_mcp_tool_approval(
        &self,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> Option<RequestUserInputResponse> {
        self.sess
            .request_user_input(self.turn_context.as_ref(), call_id, args)
            .await
    }
}

#[cfg(test)]
struct CoreMcpToolApprovalPersistenceHost<'a> {
    sess: &'a Session,
    turn_context: &'a TurnContext,
}

#[cfg(test)]
impl McpToolApprovalPersistenceHost for CoreMcpToolApprovalPersistenceHost<'_> {
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

#[cfg(test)]
async fn maybe_persist_mcp_tool_approval(
    sess: &Session,
    turn_context: &TurnContext,
    key: McpToolApprovalKey,
) {
    let host = CoreMcpToolApprovalPersistenceHost { sess, turn_context };
    maybe_persist_mcp_tool_approval_with_host(&host, key).await;
}

#[cfg(test)]
#[path = "tool_call_tests.rs"]
mod tests;
