use crate::arc_monitor::monitor_action;
use crate::memory_usage::emit_metric_for_tool_read_parts;
use crate::session::session::Session;
use crate::session::session::approval_review_runtime_impl;
use crate::session::session::approval_support_impl::permission_request_hook_payload;
use crate::session::turn_context::TurnContext;
use crate::tool_dispatch_trace::ToolDispatchTrace;
use codex_agent_runtime::BudgetLimitSteering;
use codex_agent_runtime::TerminalMetricEmission;
use codex_approval_service_api::ApprovalSessionCapability;
use codex_approval_service_api::DeferredNetworkApproval;
use codex_approval_service_api::GuardianReviewDispatch;
use codex_approval_service_api::PermissionRequestPayload;
use codex_approval_service_api::ReviewAssessmentRecord;
use codex_approval_service_api::ReviewRejectionRecord;
use codex_approval_service_api::ReviewRuntimeError;
use codex_approval_service_api::ReviewRuntimeOutcome;
use codex_approval_service_api::ReviewRuntimeResult;
use codex_approval_service_api::ToolPermissionGrants;
use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_guardian::GuardianApprovalRequest;
use codex_sandboxing_api::ResolvedApplyPatchEnvironment;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_sandboxing_api::ToolSandboxContext;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use hooks::PreToolUseHookResult;
use hooks::run_permission_request_hooks;
use mcp_types::ElicitationResponse;
use mcp_types::ElicitationReviewerHandle;
use mcp_types::McpServerElicitationRequestParams;
use protocol::approvals::ExecPolicyAmendment;
use protocol::approvals::NetworkApprovalContext;
use protocol::approvals::NetworkPolicyAmendment;
use protocol::config_types::ApprovalsReviewer;
use protocol::config_types::ModeKind;
use protocol::dynamic_tools::DynamicToolCallRequest;
use protocol::dynamic_tools::DynamicToolResponse;
use protocol::mcp::RequestId;
use protocol::models::AdditionalPermissionProfile;
use protocol::models::PermissionProfile;
use protocol::models::ResponseItem;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::protocol::AskForApproval;
use protocol::protocol::EventMsg;
use protocol::protocol::McpServerRefreshConfig;
use protocol::protocol::ReviewDecision;
use protocol::protocol::TerminalInteractionEvent;
use protocol::protocol::TokenUsage;
use protocol::protocol::TurnAbortReason;
use session_telemetry_api::SharedSessionTelemetry;
use state_api::ExternalGoalSet;
use state_api::SharedStateDbRuntime;
use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use thread_service_api::PostToolUseHookOutcome;
use thread_service_api::PostToolUsePayload;
use thread_service_api::PreToolUseHookOutcome;
use thread_service_api::PreToolUsePayload;
use thread_service_api::SessionCapabilityFuture;
use thread_service_api::ThreadDiscoveryContext;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;
use thread_service_api::ToolSessionDispatchTrace;
use thread_service_api::ToolTelemetryTags;
use tokio::sync::oneshot;
use tool_service_api::ToolCallSource;
use tool_service_api::ToolName;
use tool_service_api::ToolPayload;

fn track_review_analytics_event(
    session: &Session,
    tracking: &codex_analytics_api::GuardianReviewTrackContext,
    result: codex_analytics_api::GuardianReviewAnalyticsResult,
    completed_at_ms: u64,
) {
    session
        .services
        .analytics_events_client
        .track_guardian_review(tracking, result, completed_at_ms);
}

pub(crate) enum SessionToolNetworkApprovalState {
    Immediate(std::sync::Mutex<Option<String>>),
    Deferred(DeferredNetworkApproval),
}

pub(crate) struct SessionToolNetworkApprovalHandle {
    pub(crate) service: Arc<dyn codex_approval_service_api::SessionNetworkApprovalApi>,
    pub(crate) mode: thread_service_api::NetworkApprovalMode,
    pub(crate) cancellation_token: tokio_util::sync::CancellationToken,
    pub(crate) state: SessionToolNetworkApprovalState,
}

impl thread_service_api::ToolRuntimeNetworkApprovalHandle for SessionToolNetworkApprovalHandle {
    fn mode(&self) -> thread_service_api::NetworkApprovalMode {
        self.mode
    }

    fn registration_id(&self) -> Option<String> {
        match &self.state {
            SessionToolNetworkApprovalState::Immediate(registration_id) => registration_id
                .lock()
                .unwrap_or_else(|_| panic!("network approval state mutex poisoned"))
                .clone(),
            SessionToolNetworkApprovalState::Deferred(deferred) => {
                Some(deferred.registration_id().to_string())
            }
        }
    }

    fn cancellation_token(&self) -> tokio_util::sync::CancellationToken {
        self.cancellation_token.clone()
    }

    fn finish<'a>(
        &'a self,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<(), thread_service_api::ToolRuntimeNetworkApprovalError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            match &self.state {
                SessionToolNetworkApprovalState::Immediate(registration_id) => {
                    let Some(registration_id) = registration_id
                        .lock()
                        .unwrap_or_else(|_| panic!("network approval state mutex poisoned"))
                        .take()
                    else {
                        return Ok(());
                    };
                    self.service.finish_call(registration_id).await
                }
                SessionToolNetworkApprovalState::Deferred(deferred) => deferred.finish().await,
            }
        })
    }
}

pub(crate) fn map_network_trigger(
    trigger: thread_service_api::ToolRuntimeNetworkApprovalTrigger,
) -> codex_guardian::GuardianNetworkAccessTrigger {
    codex_guardian::GuardianNetworkAccessTrigger {
        call_id: trigger.call_id,
        tool_name: trigger.tool_name,
        command: trigger.command,
        cwd: trigger.cwd,
        sandbox_permissions: trigger.sandbox_permissions,
        additional_permissions: trigger.additional_permissions,
        justification: trigger.justification,
        tty: trigger.tty,
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GuardianNetworkAccessTriggerPayload {
    call_id: String,
    tool_name: String,
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    sandbox_permissions: protocol::models::SandboxPermissions,
    additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
    justification: Option<String>,
    tty: Option<bool>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct GuardianMcpAnnotationsPayload {
    destructive_hint: Option<bool>,
    open_world_hint: Option<bool>,
    read_only_hint: Option<bool>,
}

#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum GuardianApprovalRequestPayload {
    Shell {
        id: String,
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        sandbox_permissions: protocol::models::SandboxPermissions,
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
        justification: Option<String>,
    },
    #[cfg(unix)]
    Execve {
        id: String,
        source: protocol::approvals::GuardianCommandSource,
        program: String,
        argv: Vec<String>,
        cwd: AbsolutePathBuf,
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
    },
    ApplyPatch {
        id: String,
        cwd: AbsolutePathBuf,
        files: Vec<AbsolutePathBuf>,
        patch: String,
    },
    ExecCommand {
        id: String,
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        sandbox_permissions: protocol::models::SandboxPermissions,
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
        justification: Option<String>,
        tty: bool,
    },
    NetworkAccess {
        id: String,
        turn_id: String,
        target: String,
        host: String,
        protocol: protocol::approvals::NetworkApprovalProtocol,
        port: u16,
        trigger: Option<GuardianNetworkAccessTriggerPayload>,
    },
    McpToolCall {
        id: String,
        server: String,
        tool_name: String,
        arguments: Option<serde_json::Value>,
        connector_id: Option<String>,
        connector_name: Option<String>,
        connector_description: Option<String>,
        tool_title: Option<String>,
        tool_description: Option<String>,
        annotations: Option<GuardianMcpAnnotationsPayload>,
    },
    RequestPermissions {
        id: String,
        turn_id: String,
        reason: Option<String>,
        permissions: protocol::request_permissions::RequestPermissionProfile,
    },
}

impl ThreadTurnCapability for TurnContext {
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }

    fn tool_dispatch_telemetry(&self) -> SharedSessionTelemetry {
        self.tool_dispatch_telemetry()
    }

    fn base_tool_result_tags(&self) -> ToolTelemetryTags {
        self.base_tool_result_tags()
    }

    fn rollout_turn_id(&self) -> String {
        self.sub_id.clone()
    }

    fn discovery_context(&self) -> ThreadDiscoveryContext {
        TurnContext::discovery_context(self)
    }

    fn approval_policy(&self) -> AskForApproval {
        self.approval_policy()
    }

    fn approvals_reviewer(&self) -> ApprovalsReviewer {
        self.config.approvals_reviewer
    }

    fn permission_profile(&self) -> PermissionProfile {
        self.permission_profile()
    }

    fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy {
        self.file_system_sandbox_policy()
    }

    fn windows_sandbox_level(&self) -> protocol::config_types::WindowsSandboxLevel {
        self.windows_sandbox_level()
    }

    fn tool_sandbox_context(&self) -> ToolSandboxContext {
        self.tool_sandbox_context()
    }

    fn resolve_apply_patch_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, tool_service_api::FunctionCallError> {
        self.resolve_apply_patch_environment(environment_id)
    }

    fn thread_id(&self) -> protocol::ThreadId {
        self.session_arc().thread_id()
    }

    fn runtime_turn_id_str(&self) -> &str {
        self.turn_id_str()
    }

    fn truncation_policy(&self) -> TruncationPolicy {
        self.truncation_policy()
    }

    fn apply_patch_streaming_events_enabled(&self) -> bool {
        self.is_apply_patch_streaming_events_enabled()
    }

    fn collaboration_mode_kind(&self) -> ModeKind {
        self.collaboration_mode_kind()
    }

    fn legacy_cwd(&self) -> AbsolutePathBuf {
        self.legacy_cwd()
    }

    fn is_non_root_agent(&self) -> bool {
        self.is_non_root_agent()
    }

    fn supports_image_input(&self) -> bool {
        self.supports_image_input()
    }

    fn app_server_client_name(&self) -> Option<&str> {
        self.app_server_client_name.as_deref()
    }

    fn auth_elicitation_enabled(&self) -> bool {
        self.auth_elicitation_enabled()
    }

    fn tool_call_mcp_elicitation_enabled(&self) -> bool {
        self.tool_call_mcp_elicitation_enabled()
    }

    fn mcp_turn_metadata(&self) -> Option<serde_json::Value> {
        self.mcp_turn_metadata_value()
    }

    fn mcp_sandbox_state(&self) -> mcp_types::SandboxState {
        self.mcp_sandbox_state()
    }

    fn auth_snapshot<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Option<codex_auth_types::RequestAuthSnapshot>> {
        Box::pin(async move { self.auth_snapshot().await })
    }

    fn cached_accessible_connectors_from_mcp_tools<'a>(
        &'a self,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> SessionCapabilityFuture<'a, Option<Vec<codex_connectors_api::AppInfo>>> {
        Box::pin(async move {
            self.cached_accessible_connectors_from_mcp_tools(auth_snapshot)
                .await
        })
    }

    fn refresh_accessible_connectors_cache_from_mcp_tools(
        &self,
        connector_auth_context: Option<&mcp_types::CodexAppsAuthContext>,
        mcp_tools: &[mcp_types::ToolInfo],
    ) {
        self.refresh_accessible_connectors_cache_from_mcp_tools(connector_auth_context, mcp_tools);
    }

    fn codex_app_tool_policy(
        &self,
        metadata: Option<&mcp_types::McpToolApprovalMetadata>,
        tool_name: &str,
    ) -> thread_service_api::ThreadAppToolPolicy {
        let policy = self.codex_app_tool_policy(metadata, tool_name);
        thread_service_api::ThreadAppToolPolicy {
            enabled: policy.enabled,
            approval: policy.approval,
        }
    }

    fn session_collaboration_mode<'a>(&'a self) -> SessionCapabilityFuture<'a, ModeKind> {
        Box::pin(async move { self.session_arc().collaboration_mode_kind().await })
    }

    fn emit_event<'a>(&'a self, event: EventMsg) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.session_arc().send_event(self, event).await;
        })
    }

    fn request_permissions<'a>(
        &'a self,
        call_id: String,
        args: protocol::request_permissions::RequestPermissionsArgs,
        cancellation_token: tokio_util::sync::CancellationToken,
    ) -> SessionCapabilityFuture<
        'a,
        Option<protocol::request_permissions::RequestPermissionsResponse>,
    > {
        Box::pin(async move {
            let session = self.session_arc();
            let turn = self.self_arc();
            session
                .request_permissions(&turn, call_id, args, cancellation_token)
                .await
        })
    }

    fn request_user_input<'a>(
        &'a self,
        call_id: String,
        args: protocol::request_user_input::RequestUserInputArgs,
    ) -> SessionCapabilityFuture<'a, Option<protocol::request_user_input::RequestUserInputResponse>>
    {
        Box::pin(async move {
            self.session_arc()
                .request_user_input(self, call_id, args)
                .await
        })
    }

    fn request_dynamic_tool<'a>(
        &'a self,
        call_id: String,
        tool_name: ToolName,
        arguments: serde_json::Value,
    ) -> SessionCapabilityFuture<'a, Option<DynamicToolResponse>> {
        Box::pin(async move {
            request_dynamic_tool(&self.session_arc(), self, call_id, tool_name, arguments).await
        })
    }
}

async fn request_dynamic_tool(
    session: &Arc<Session>,
    turn: &TurnContext,
    call_id: String,
    tool_name: ToolName,
    arguments: serde_json::Value,
) -> Option<DynamicToolResponse> {
    let (tx_response, rx_response) = oneshot::channel();
    if session
        .register_pending_dynamic_tool_response(call_id.clone(), tx_response)
        .await
    {
        return None;
    }

    session
        .send_event(
            turn,
            EventMsg::DynamicToolCallRequest(DynamicToolCallRequest {
                call_id,
                turn_id: turn.turn_id_str().to_string(),
                started_at_ms: crate::turn_timing::now_unix_timestamp_ms(),
                namespace: tool_name.namespace,
                tool: tool_name.name,
                arguments,
            }),
        )
        .await;

    rx_response.await.ok()
}

impl ThreadSessionCapability for Session {
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }

    fn conversation_id(&self) -> protocol::ThreadId {
        self.conversation_id
    }

    fn require_persisted_state_db<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<SharedStateDbRuntime, String>> {
        Box::pin(async move {
            self.require_state_db_for_thread_goals()
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn tool_dispatch_telemetry(&self, turn: &dyn ThreadTurnCapability) -> SharedSessionTelemetry {
        turn.tool_dispatch_telemetry()
    }

    fn base_tool_result_tags(&self, turn: &dyn ThreadTurnCapability) -> ToolTelemetryTags {
        turn.base_tool_result_tags()
    }

    fn record_tool_call_started<'a>(
        &'a self,
        _turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.record_tool_call_started().await;
        })
    }

    fn start_tool_dispatch_trace(
        &self,
        turn: &dyn ThreadTurnCapability,
        call_id: &str,
        tool_name: &ToolName,
        source: &ToolCallSource,
        payload: &ToolPayload,
    ) -> Box<dyn ToolSessionDispatchTrace> {
        Box::new(ToolDispatchTrace::start_parts(
            self.conversation_id,
            turn.rollout_turn_id(),
            call_id,
            tool_name,
            source,
            payload,
            self.services.rollout_thread_trace.clone(),
        ))
    }

    fn run_pre_tool_use_hooks_for_tool<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        call_id: String,
        payload: PreToolUsePayload,
    ) -> SessionCapabilityFuture<'a, PreToolUseHookOutcome> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return PreToolUseHookOutcome::Blocked(invalid_turn_message());
            };
            match self
                .run_pre_tool_use_hooks_for_turn(
                    turn,
                    call_id,
                    payload.tool_name.name(),
                    payload.tool_name.matcher_aliases().to_vec(),
                    &payload.tool_input,
                )
                .await
            {
                PreToolUseHookResult::Blocked(message) => PreToolUseHookOutcome::Blocked(message),
                PreToolUseHookResult::Continue { updated_input } => {
                    PreToolUseHookOutcome::Continue { updated_input }
                }
            }
        })
    }

    fn run_post_tool_use_hooks_for_tool<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        payload: PostToolUsePayload,
    ) -> SessionCapabilityFuture<'a, PostToolUseHookOutcome> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return PostToolUseHookOutcome {
                    replacement_text: Some(invalid_turn_message()),
                };
            };
            let outcome = self.run_post_tool_use_hooks_for_turn(turn, payload).await;
            let replacement_text = if outcome.should_stop {
                Some(
                    outcome
                        .feedback_message
                        .or(outcome.stop_reason)
                        .unwrap_or_else(|| "PostToolUse hook stopped execution".to_string()),
                )
            } else {
                outcome.feedback_message
            };

            PostToolUseHookOutcome { replacement_text }
        })
    }

    fn emit_tool_read_metric<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        tool_name: &'a ToolName,
        payload: &'a ToolPayload,
        success: bool,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            emit_metric_for_tool_read_parts(self, turn, tool_name, payload, success).await;
        })
    }

    fn account_goal_tool_completed<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        tool_name: &'a str,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(invalid_turn_message());
            };
            self.account_goal_tool_completed(turn, tool_name).await
        })
    }

    fn account_goal_mutation_completed<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(invalid_turn_message());
            };
            self.account_thread_goal_progress(
                turn,
                BudgetLimitSteering::Suppressed,
                TerminalMetricEmission::Suppress,
            )
            .await
            .map_err(|err| err.to_string())
        })
    }

    fn begin_turn_goal_accounting<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        token_usage: TokenUsage,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(invalid_turn_message());
            };
            self.mark_thread_goal_turn_started(turn, token_usage).await;
            Ok(())
        })
    }

    fn finish_turn_goal_accounting<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        turn_completed: bool,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(invalid_turn_message());
            };
            let Some(session) = session_arc(self) else {
                return Err(invalid_session_message());
            };
            session.finish_thread_goal_turn(turn, turn_completed).await;
            Ok(())
        })
    }

    fn handle_goal_turn_abort<'a>(
        &'a self,
        turn: Option<&'a dyn ThreadTurnCapability>,
        reason: TurnAbortReason,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let turn = match turn {
                Some(turn) => Some(turn_context(turn).ok_or_else(invalid_turn_message)?),
                None => None,
            };
            self.handle_thread_goal_task_abort(turn, reason).await;
            Ok(())
        })
    }

    fn maybe_continue_active_goal<'a>(&'a self) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(session) = session_arc(self) else {
                return Err(invalid_session_message());
            };
            session.maybe_continue_goal_if_idle_runtime().await;
            Ok(())
        })
    }

    fn prepare_external_goal_mutation<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.account_thread_goal_before_external_mutation()
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn apply_external_goal_set<'a>(
        &'a self,
        external_set: ExternalGoalSet,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(session) = session_arc(self) else {
                return Err(invalid_session_message());
            };
            session
                .apply_external_thread_goal_status(external_set)
                .await;
            Ok(())
        })
    }

    fn apply_external_goal_clear<'a>(&'a self) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.clear_stopped_thread_goal_runtime_state().await;
            Ok(())
        })
    }

    fn restore_goal_runtime_after_resume<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.restore_thread_goal_runtime_after_resume()
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn emit_event<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        event: EventMsg,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>()
            else {
                tracing::warn!("tool session capability received an unsupported turn context");
                return;
            };
            self.send_event(turn, event).await;
        })
    }

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        items: Vec<ResponseItem>,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>()
            else {
                tracing::warn!("tool session capability received an unsupported turn context");
                return;
            };
            self.record_model_items_and_emit_display_events(turn, &items)
                .await;
        })
    }

    fn append_conversation_item<'a>(
        &'a self,
        item: ResponseItem,
    ) -> SessionCapabilityFuture<'a, Result<String, String>> {
        Box::pin(async move {
            let submission_id = uuid::Uuid::new_v4().to_string();
            self.enqueue_async_input(crate::PendingInputItem::from(item));
            if let Some(session) = self.self_weak.get().and_then(|weak| weak.upgrade()) {
                session.maybe_start_turn_for_pending_work().await;
            }
            Ok(submission_id)
        })
    }

    fn sandbox_runtime(&self) -> SharedSandboxRuntime {
        self.sandbox_runtime()
    }

    fn subscribe_out_of_band_elicitation_pause_state(&self) -> tokio::sync::watch::Receiver<bool> {
        self.subscribe_out_of_band_elicitation_pause_state()
    }

    fn request_mcp_server_elicitation<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        request_id: RequestId,
        params: McpServerElicitationRequestParams,
    ) -> SessionCapabilityFuture<'a, Option<ElicitationResponse>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return None;
            };
            self.request_mcp_server_elicitation(turn, request_id, params)
                .await
        })
    }

    fn resolve_mcp_elicitation<'a>(
        &'a self,
        server_name: String,
        request_id: RequestId,
        response: ElicitationResponse,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.resolve_elicitation(server_name, request_id, response)
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn refresh_mcp_servers_if_requested<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            self.refresh_mcp_servers_if_requested(turn, elicitation_reviewer)
                .await;
        })
    }

    fn queue_mcp_server_refresh<'a>(
        &'a self,
        refresh_config: McpServerRefreshConfig,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.queue_mcp_server_refresh(refresh_config).await;
        })
    }

    fn configured_mcp_servers<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, HashMap<String, codex_config_types::McpServerConfig>> {
        Box::pin(async move {
            self.configured_mcp_servers(self.get_config().await.as_ref())
                .await
        })
    }

    fn mcp_dependency_prompted<'a>(&'a self) -> SessionCapabilityFuture<'a, HashSet<String>> {
        Box::pin(async move { self.mcp_dependency_prompted().await })
    }

    fn record_mcp_dependency_prompted<'a>(
        &'a self,
        names: Vec<String>,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move { self.record_mcp_dependency_prompted(names).await })
    }

    fn notify_user_input_response<'a>(
        &'a self,
        sub_id: &'a str,
        response: protocol::request_user_input::RequestUserInputResponse,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move { self.notify_user_input_response(sub_id, response).await })
    }

    fn mcp_oauth_login_support<'a>(
        &'a self,
        transport: &'a codex_config_types::McpServerTransportConfig,
    ) -> SessionCapabilityFuture<'a, mcp_types::McpOAuthLoginSupport> {
        Box::pin(async move { self.mcp_oauth_login_support(transport).await })
    }

    fn perform_mcp_oauth_login<'a>(
        &'a self,
        params: thread_service_api::McpOAuthLoginParams,
    ) -> SessionCapabilityFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.perform_mcp_oauth_login(mcp_service_api::McpOAuthLoginRequest {
                server_name: params.server_name,
                server_url: params.server_url,
                store_mode: params.store_mode,
                http_headers: params.http_headers,
                env_http_headers: params.env_http_headers,
                scopes: params.scopes,
                oauth_client_id: params.oauth_client_id,
                oauth_resource: params.oauth_resource,
                callback_port: params.callback_port,
                callback_url: params.callback_url,
            })
            .await
        })
    }

    fn should_retry_mcp_oauth_without_scopes(
        &self,
        scopes: &mcp_types::ResolvedMcpOAuthScopes,
        error: &anyhow::Error,
    ) -> bool {
        self.should_retry_mcp_oauth_without_scopes(scopes, error)
    }

    fn refresh_mcp_servers_now<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        refresh_config: McpServerRefreshConfig,
        elicitation_reviewer: Option<ElicitationReviewerHandle>,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            let McpServerRefreshConfig {
                mcp_servers,
                mcp_oauth_credentials_store_mode,
            } = refresh_config;
            let mcp_servers = match serde_json::from_value(mcp_servers) {
                Ok(servers) => servers,
                Err(err) => {
                    tracing::warn!("failed to parse MCP server refresh config: {err}");
                    return;
                }
            };
            let store_mode = match serde_json::from_value(mcp_oauth_credentials_store_mode) {
                Ok(mode) => mode,
                Err(err) => {
                    tracing::warn!("failed to parse MCP OAuth refresh config: {err}");
                    return;
                }
            };
            self.refresh_mcp_servers_now(turn, mcp_servers, store_mode, elicitation_reviewer)
                .await;
        })
    }

    fn cancel_mcp_startup<'a>(&'a self) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.cancel_mcp_startup().await;
        })
    }

    fn hard_refresh_codex_apps_tools_cache<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<Vec<mcp_types::ToolInfo>, String>> {
        Box::pin(async move {
            self.hard_refresh_codex_apps_tools_cache()
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn call_mcp_tool<'a>(
        &'a self,
        server: &'a str,
        tool: &'a str,
        arguments: Option<serde_json::Value>,
        meta: Option<serde_json::Value>,
    ) -> SessionCapabilityFuture<'a, Result<protocol::mcp::CallToolResult, String>> {
        #[allow(clippy::await_holding_invalid_type)]
        Box::pin(async move {
            let manager = self.services.mcp_connection_manager.read().await;
            mcp_service_api::McpToolRuntime::call_tool(
                manager.as_ref(),
                server,
                tool,
                arguments,
                meta,
            )
            .await
            .map_err(|error| format!("tool call error: {error:?}"))
        })
    }

    fn list_mcp_resources<'a>(
        &'a self,
        server: &'a str,
        params: Option<protocol::mcp::PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<protocol::mcp::ListResourcesResult, String>> {
        #[allow(clippy::await_holding_invalid_type)]
        Box::pin(async move {
            let manager = self.services.mcp_connection_manager.read().await;
            manager
                .list_resources(server, params)
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn list_all_mcp_resources(
        &self,
    ) -> SessionCapabilityFuture<'_, HashMap<String, Vec<protocol::mcp::Resource>>> {
        #[allow(clippy::await_holding_invalid_type)]
        Box::pin(async move {
            let manager = self.services.mcp_connection_manager.read().await;
            manager.list_all_resources().await
        })
    }

    fn list_mcp_resource_templates<'a>(
        &'a self,
        server: &'a str,
        params: Option<protocol::mcp::PaginatedRequestParams>,
    ) -> SessionCapabilityFuture<'a, Result<protocol::mcp::ListResourceTemplatesResult, String>>
    {
        #[allow(clippy::await_holding_invalid_type)]
        Box::pin(async move {
            let manager = self.services.mcp_connection_manager.read().await;
            manager
                .list_resource_templates(server, params)
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn list_all_mcp_resource_templates(
        &self,
    ) -> SessionCapabilityFuture<'_, HashMap<String, Vec<protocol::mcp::ResourceTemplate>>> {
        #[allow(clippy::await_holding_invalid_type)]
        Box::pin(async move {
            let manager = self.services.mcp_connection_manager.read().await;
            manager.list_all_resource_templates().await
        })
    }

    fn read_mcp_resource<'a>(
        &'a self,
        server: &'a str,
        params: protocol::mcp::ReadResourceRequestParams,
    ) -> SessionCapabilityFuture<'a, Result<protocol::mcp::ReadResourceResult, String>> {
        #[allow(clippy::await_holding_invalid_type)]
        Box::pin(async move {
            let manager = self.services.mcp_connection_manager.read().await;
            manager
                .read_resource(server, params)
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn list_all_mcp_tools<'a>(&'a self) -> SessionCapabilityFuture<'a, Vec<mcp_types::ToolInfo>> {
        Box::pin(async move { self.list_all_mcp_tools().await })
    }

    fn mcp_server_origin<'a>(
        &'a self,
        server: &'a str,
    ) -> SessionCapabilityFuture<'a, Option<String>> {
        Box::pin(async move { self.mcp_server_origin(server).await })
    }

    fn mcp_server_is_host_owned_codex_apps<'a>(
        &'a self,
        server: &'a str,
    ) -> SessionCapabilityFuture<'a, bool> {
        Box::pin(async move { self.mcp_server_is_host_owned_codex_apps(server).await })
    }

    fn mcp_server_supports_sandbox_state_meta<'a>(
        &'a self,
        server: &'a str,
    ) -> SessionCapabilityFuture<'a, bool> {
        Box::pin(async move { self.mcp_server_supports_sandbox_state_meta(server).await })
    }

    fn add_optional_mcp_call_trace_request_meta(
        &self,
        call_id: &str,
        meta: Option<serde_json::Value>,
    ) -> Option<serde_json::Value> {
        self.add_optional_mcp_call_trace_request_meta(call_id, meta)
    }

    fn rewrite_mcp_tool_arguments_for_openai_files<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        arguments: Option<serde_json::Value>,
        openai_file_input_params: Option<&'a [String]>,
    ) -> SessionCapabilityFuture<'a, Result<Option<serde_json::Value>, String>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(invalid_turn_message());
            };
            self.rewrite_mcp_tool_arguments_for_openai_files(
                turn,
                arguments,
                openai_file_input_params,
            )
            .await
        })
    }

    fn mark_thread_memory_mode_polluted_for_mcp_tool_call<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        server: &'a str,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            self.mark_thread_memory_mode_polluted_for_mcp_tool_call(turn, server)
                .await;
        })
    }

    fn track_codex_app_used_for_mcp_tool<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        server: &'a str,
        tool_name: &'a str,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            self.track_codex_app_used_for_mcp_tool(turn, server, tool_name)
                .await;
        })
    }

    fn mcp_tool_approval_is_remembered<'a>(
        &'a self,
        key: &'a mcp_types::McpToolApprovalKey,
    ) -> SessionCapabilityFuture<'a, bool> {
        Box::pin(async move { self.mcp_tool_approval_is_remembered(key).await })
    }

    fn remember_mcp_tool_approval<'a>(
        &'a self,
        key: mcp_types::McpToolApprovalKey,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move { self.remember_mcp_tool_approval(key).await })
    }

    fn custom_mcp_tool_approval_mode<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        server: &'a str,
        tool_name: &'a str,
    ) -> SessionCapabilityFuture<'a, codex_config_types::AppToolApproval> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return codex_config_types::AppToolApproval::Prompt;
            };
            self.custom_mcp_tool_approval_mode(turn, server, tool_name)
                .await
        })
    }

    fn fetch_accessible_connectors_from_mcp_tools<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        auth_snapshot: Option<&'a codex_auth_types::RequestAuthSnapshot>,
    ) -> SessionCapabilityFuture<'a, anyhow::Result<Vec<codex_connectors_api::AppInfo>>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(anyhow::anyhow!(invalid_turn_message()));
            };
            self.fetch_accessible_connectors_from_mcp_tools(turn, auth_snapshot)
                .await
        })
    }

    fn persist_codex_app_tool_approval_for_turn<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        connector_id: String,
        tool_name: String,
    ) -> SessionCapabilityFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(anyhow::anyhow!(invalid_turn_message()));
            };
            self.persist_codex_app_tool_approval_for_turn(turn, &connector_id, &tool_name)
                .await
        })
    }

    fn persist_non_app_mcp_tool_approval_for_turn<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        server: String,
        tool_name: String,
    ) -> SessionCapabilityFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(anyhow::anyhow!(invalid_turn_message()));
            };
            self.persist_non_app_mcp_tool_approval_for_turn(turn, &server, &tool_name)
                .await
        })
    }

    fn reload_user_config_layer<'a>(&'a self) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move { self.reload_user_config_layer().await })
    }

    fn configured_plugin_installed<'a>(
        &'a self,
        tool_id: &'a str,
    ) -> SessionCapabilityFuture<'a, bool> {
        Box::pin(async move { self.configured_plugin_installed(tool_id).await })
    }

    fn merge_connector_selection<'a>(
        &'a self,
        connector_ids: std::collections::HashSet<String>,
    ) -> SessionCapabilityFuture<'a, std::collections::HashSet<String>> {
        Box::pin(async move { self.merge_connector_selection(connector_ids).await })
    }

    fn monitor_auto_approved_action<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        action: serde_json::Value,
        callsite_mode: &'static str,
    ) -> SessionCapabilityFuture<'a, thread_service_api::AutoApprovalSafetyOutcome> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return thread_service_api::AutoApprovalSafetyOutcome::Ok;
            };
            match monitor_action(self, turn, action, callsite_mode).await {
                crate::arc_monitor::ArcMonitorOutcome::Ok => {
                    thread_service_api::AutoApprovalSafetyOutcome::Ok
                }
                crate::arc_monitor::ArcMonitorOutcome::AskUser(reason) => {
                    thread_service_api::AutoApprovalSafetyOutcome::AskUser(reason)
                }
                crate::arc_monitor::ArcMonitorOutcome::SteerModel(reason) => {
                    thread_service_api::AutoApprovalSafetyOutcome::SteerModel(reason)
                }
            }
        })
    }

    fn emit_model_item_started_display_event<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        item: &'a ResponseItem,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            Session::emit_model_item_started_display_event(self, turn, item).await;
        })
    }

    fn send_terminal_interaction<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        event: TerminalInteractionEvent,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            self.send_event(turn, EventMsg::TerminalInteraction(event))
                .await;
        })
    }

    fn unregister_network_approval<'a>(
        &'a self,
        registration_id: &'a str,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.services
                .network_approval
                .unregister_call(registration_id.to_string())
                .await;
        })
    }

    fn code_mode_stored_values(
        &self,
    ) -> SessionCapabilityFuture<'_, HashMap<String, serde_json::Value>> {
        Box::pin(async move { Session::code_mode_stored_values(self).await })
    }

    fn code_mode_replace_stored_values(
        &self,
        values: HashMap<String, serde_json::Value>,
    ) -> SessionCapabilityFuture<'_, ()> {
        Box::pin(async move {
            Session::code_mode_replace_stored_values(self, values).await;
        })
    }

    fn code_mode_allocate_cell_id(&self) -> String {
        Session::code_mode_allocate_cell_id(self)
    }

    fn code_mode_execute(
        &self,
        request: ExecuteRequest,
    ) -> SessionCapabilityFuture<'_, Result<RuntimeResponse, String>> {
        Box::pin(async move { Session::code_mode_execute(self, request).await })
    }

    fn code_mode_wait(
        &self,
        request: WaitRequest,
    ) -> SessionCapabilityFuture<'_, Result<WaitOutcome, String>> {
        Box::pin(async move { Session::code_mode_wait(self, request).await })
    }

    fn record_code_mode_cell_started(
        &self,
        turn: &dyn thread_service_api::ThreadRuntimeCapability,
        runtime_cell_id: &str,
        model_visible_call_id: &str,
        source_js: &str,
    ) {
        Session::record_code_mode_cell_started(
            self,
            turn.runtime_turn_id().as_str(),
            runtime_cell_id,
            model_visible_call_id,
            source_js,
        );
    }

    fn record_code_mode_cell_initial_response(
        &self,
        turn: &dyn thread_service_api::ThreadRuntimeCapability,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        Session::record_code_mode_cell_initial_response(
            self,
            turn.runtime_turn_id().as_str(),
            runtime_cell_id,
            response,
        );
    }

    fn record_code_mode_cell_ended(
        &self,
        turn: &dyn thread_service_api::ThreadRuntimeCapability,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        Session::record_code_mode_cell_ended(
            self,
            turn.runtime_turn_id().as_str(),
            runtime_cell_id,
            response,
        );
    }

    fn emit_turn_item_started<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        item: &'a protocol::items::TurnItem,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            self.emit_turn_item_started(turn, item).await;
        })
    }

    fn emit_turn_item_completed<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        item: protocol::items::TurnItem,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            self.emit_turn_item_completed(turn, item).await;
        })
    }
}

impl ApprovalSessionCapability for Session {
    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> SessionCapabilityFuture<'a, Option<hooks_api::PermissionRequestDecision>> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>()
            else {
                tracing::warn!("tool session capability received an unsupported turn context");
                return None;
            };
            run_permission_request_hooks(
                self,
                turn,
                permission_request_run_id,
                permission_request_hook_payload(permission_request),
            )
            .await
        })
    }

    fn strict_auto_review_enabled_for_turn<'a>(&'a self) -> SessionCapabilityFuture<'a, bool> {
        Box::pin(async move { self.strict_auto_review_enabled_for_turn().await })
    }

    fn active_turn_runtime<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Option<Arc<dyn thread_service_api::ThreadRuntimeCapability>>>
    {
        Box::pin(async move {
            let active_turn = self.active_turn.lock().await;
            active_turn
                .as_ref()
                .and_then(|turn| turn.tasks.first())
                .map(|(_, task)| {
                    Arc::clone(&task.turn_context)
                        as Arc<dyn thread_service_api::ThreadRuntimeCapability>
                })
        })
    }

    fn take_review_rejection<'a>(
        &'a self,
        review_id: &'a str,
    ) -> SessionCapabilityFuture<'a, Option<ReviewRejectionRecord>> {
        Box::pin(async move {
            self.services
                .guardian_rejections
                .lock()
                .await
                .remove(review_id)
                .map(|rejection| ReviewRejectionRecord {
                    rationale: rejection.rationale,
                    source: rejection.source,
                })
        })
    }

    fn set_review_rejection<'a>(
        &'a self,
        review_id: String,
        rejection: Option<ReviewRejectionRecord>,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let mut rejections = self.services.guardian_rejections.lock().await;
            match rejection {
                Some(rejection) => {
                    rejections.insert(
                        review_id,
                        codex_guardian::GuardianRejection {
                            rationale: rejection.rationale,
                            source: rejection.source,
                        },
                    );
                }
                None => {
                    rejections.remove(&review_id);
                }
            }
        })
    }

    fn track_review_analytics<'a>(
        &'a self,
        tracking: codex_analytics_api::GuardianReviewTrackContext,
        result: codex_analytics_api::GuardianReviewAnalyticsResult,
        completed_at_ms: u64,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            track_review_analytics_event(self, &tracking, result, completed_at_ms);
        })
    }

    fn run_review_session<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        request: serde_json::Value,
        retry_reason: Option<String>,
    ) -> SessionCapabilityFuture<'a, ReviewRuntimeResult> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>()
            else {
                tracing::warn!("{}", invalid_turn_message());
                return ReviewRuntimeResult {
                    outcome: ReviewRuntimeOutcome::Error(ReviewRuntimeError::Cancelled),
                    analytics_result:
                        codex_analytics_api::GuardianReviewAnalyticsResult::without_session(),
                };
            };
            let Ok(request) = serde_json::from_value::<GuardianApprovalRequestPayload>(request)
            else {
                tracing::warn!(
                    "tool session capability received an invalid guardian approval request"
                );
                return ReviewRuntimeResult {
                    outcome: ReviewRuntimeOutcome::Error(ReviewRuntimeError::PromptBuild {
                        message: "invalid guardian approval request".to_string(),
                    }),
                    analytics_result:
                        codex_analytics_api::GuardianReviewAnalyticsResult::without_session(),
                };
            };
            let request = match request {
                GuardianApprovalRequestPayload::Shell {
                    id,
                    command,
                    cwd,
                    sandbox_permissions,
                    additional_permissions,
                    justification,
                } => GuardianApprovalRequest::Shell {
                    id,
                    command,
                    cwd,
                    sandbox_permissions,
                    additional_permissions,
                    justification,
                },
                #[cfg(unix)]
                GuardianApprovalRequestPayload::Execve {
                    id,
                    source,
                    program,
                    argv,
                    cwd,
                    additional_permissions,
                } => GuardianApprovalRequest::Execve {
                    id,
                    source,
                    program,
                    argv,
                    cwd,
                    additional_permissions,
                },
                GuardianApprovalRequestPayload::ApplyPatch {
                    id,
                    cwd,
                    files,
                    patch,
                } => GuardianApprovalRequest::ApplyPatch {
                    id,
                    cwd,
                    files,
                    patch,
                },
                GuardianApprovalRequestPayload::ExecCommand {
                    id,
                    command,
                    cwd,
                    sandbox_permissions,
                    additional_permissions,
                    justification,
                    tty,
                } => GuardianApprovalRequest::ExecCommand {
                    id,
                    command,
                    cwd,
                    sandbox_permissions,
                    additional_permissions,
                    justification,
                    tty,
                },
                GuardianApprovalRequestPayload::NetworkAccess {
                    id,
                    turn_id,
                    target,
                    host,
                    protocol,
                    port,
                    trigger,
                } => GuardianApprovalRequest::NetworkAccess {
                    id,
                    turn_id,
                    target,
                    host,
                    protocol,
                    port,
                    trigger: trigger.map(|trigger| codex_guardian::GuardianNetworkAccessTrigger {
                        call_id: trigger.call_id,
                        tool_name: trigger.tool_name,
                        command: trigger.command,
                        cwd: trigger.cwd,
                        sandbox_permissions: trigger.sandbox_permissions,
                        additional_permissions: trigger.additional_permissions,
                        justification: trigger.justification,
                        tty: trigger.tty,
                    }),
                },
                GuardianApprovalRequestPayload::McpToolCall {
                    id,
                    server,
                    tool_name,
                    arguments,
                    connector_id,
                    connector_name,
                    connector_description,
                    tool_title,
                    tool_description,
                    annotations,
                } => GuardianApprovalRequest::McpToolCall {
                    id,
                    server,
                    tool_name,
                    arguments,
                    connector_id,
                    connector_name,
                    connector_description,
                    tool_title,
                    tool_description,
                    annotations: annotations.map(|annotations| {
                        codex_guardian::GuardianMcpAnnotations {
                            destructive_hint: annotations.destructive_hint,
                            open_world_hint: annotations.open_world_hint,
                            read_only_hint: annotations.read_only_hint,
                        }
                    }),
                },
                GuardianApprovalRequestPayload::RequestPermissions {
                    id,
                    turn_id,
                    reason,
                    permissions,
                } => GuardianApprovalRequest::RequestPermissions {
                    id,
                    turn_id,
                    reason,
                    permissions,
                },
            };
            let session = turn.session_arc();
            let turn = turn.self_arc();
            let (outcome, analytics_result) = approval_review_runtime_impl::run_review_session(
                session,
                turn,
                request,
                retry_reason,
                codex_guardian::guardian_output_schema(),
                None,
            )
            .await;
            let outcome = match outcome {
                approval_review_runtime_impl::GuardianReviewOutcome::Completed(assessment) => {
                    ReviewRuntimeOutcome::Completed(ReviewAssessmentRecord {
                        risk_level: assessment.risk_level,
                        user_authorization: assessment.user_authorization,
                        outcome: assessment.outcome,
                        rationale: assessment.rationale,
                    })
                }
                approval_review_runtime_impl::GuardianReviewOutcome::Error(error) => {
                    let error = match error {
                        approval_review_runtime_impl::GuardianReviewError::PromptBuild {
                            message,
                        } => ReviewRuntimeError::PromptBuild { message },
                        approval_review_runtime_impl::GuardianReviewError::Session { message } => {
                            ReviewRuntimeError::Session { message }
                        }
                        approval_review_runtime_impl::GuardianReviewError::Parse { message } => {
                            ReviewRuntimeError::Parse { message }
                        }
                        approval_review_runtime_impl::GuardianReviewError::Timeout => {
                            ReviewRuntimeError::Timeout
                        }
                        approval_review_runtime_impl::GuardianReviewError::Cancelled => {
                            ReviewRuntimeError::Cancelled
                        }
                    };
                    ReviewRuntimeOutcome::Error(error)
                }
            };
            ReviewRuntimeResult {
                outcome,
                analytics_result,
            }
        })
    }

    fn record_review_non_rejection<'a>(
        &'a self,
        turn_id: &'a str,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(session) = self.self_weak.get().and_then(std::sync::Weak::upgrade) else {
                tracing::warn!(
                    "tool session capability lost owning session while recording guardian non-denial"
                );
                return;
            };
            session
                .services
                .guardian_rejection_circuit_breaker
                .lock()
                .await
                .record_non_denial(turn_id);
        })
    }

    fn record_review_rejection<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        turn_id: &'a str,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>()
            else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            let session = turn.session_arc();
            let turn = turn.self_arc();
            let action = session
                .services
                .guardian_rejection_circuit_breaker
                .lock()
                .await
                .record_denial(turn_id);
            let codex_guardian::GuardianRejectionCircuitBreakerAction::InterruptTurn {
                consecutive_denials,
                recent_denials,
            } = action
            else {
                return;
            };

            if session.turn_context_for_sub_id(turn_id).await.is_none() {
                return;
            }

            session
                .send_event(
                    turn.as_ref(),
                    EventMsg::GuardianWarning(protocol::protocol::WarningEvent {
                        message: format!(
                            "Automatic approval review rejected too many approval requests for this turn ({consecutive_denials} consecutive, {recent_denials} in the last {} reviews); interrupting the turn.",
                            codex_guardian::AUTO_REVIEW_DENIAL_WINDOW_SIZE
                        ),
                    }),
                )
                .await;

            let runtime_handle = session.services.runtime_handle.clone();
            let session = Arc::clone(&session);
            let turn_id = turn_id.to_string();
            let _abort_task = runtime_handle.spawn(async move {
                session
                    .abort_turn_if_active(
                        &turn_id,
                        protocol::protocol::TurnAbortReason::Interrupted,
                    )
                    .await;
            });
        })
    }

    fn request_command_approval<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        call_id: String,
        approval_id: Option<String>,
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        reason: Option<String>,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
        additional_permissions: Option<protocol::models::AdditionalPermissionProfile>,
        available_decisions: Option<Vec<protocol::protocol::ReviewDecision>>,
    ) -> SessionCapabilityFuture<'a, protocol::protocol::ReviewDecision> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>()
            else {
                tracing::warn!("{}", invalid_turn_message());
                return protocol::protocol::ReviewDecision::Abort;
            };
            Session::request_command_approval(
                self,
                turn,
                call_id,
                approval_id,
                command,
                cwd,
                reason,
                network_approval_context,
                proposed_execpolicy_amendment,
                additional_permissions,
                available_decisions,
            )
            .await
        })
    }

    fn request_patch_approval<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        call_id: String,
        changes: std::collections::HashMap<std::path::PathBuf, protocol::protocol::FileChange>,
        reason: Option<String>,
        grant_root: Option<std::path::PathBuf>,
    ) -> SessionCapabilityFuture<'a, protocol::protocol::ReviewDecision> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>()
            else {
                tracing::warn!("{}", invalid_turn_message());
                return protocol::protocol::ReviewDecision::Abort;
            };
            let rx_approve =
                Session::request_patch_approval(self, turn, call_id, changes, reason, grant_root)
                    .await;
            rx_approve.await.unwrap_or_default()
        })
    }

    fn cached_approval_decision<'a>(
        &'a self,
        key: String,
    ) -> SessionCapabilityFuture<'a, Option<protocol::protocol::ReviewDecision>> {
        Box::pin(async move {
            let store = self.services.tool_approvals.lock().await;
            store.get(&key)
        })
    }

    fn cache_approval_decision<'a>(
        &'a self,
        keys: Vec<String>,
        decision: protocol::protocol::ReviewDecision,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            if !matches!(
                decision,
                protocol::protocol::ReviewDecision::ApprovedForSession
            ) {
                return;
            }
            let mut store = self.services.tool_approvals.lock().await;
            for key in keys {
                store.put(key, protocol::protocol::ReviewDecision::ApprovedForSession);
            }
        })
    }

    fn record_approval_request_telemetry<'a>(
        &'a self,
        tool_name: &'a str,
        decision: &'a protocol::protocol::ReviewDecision,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.services.session_telemetry.counter(
                "codex.approval.requested",
                /*inc*/ 1,
                &[
                    ("tool", tool_name),
                    ("approved", decision.to_opaque_string()),
                ],
            );
        })
    }

    fn persist_network_policy_amendment<'a>(
        &'a self,
        amendment: &'a NetworkPolicyAmendment,
        network_approval_context: &'a NetworkApprovalContext,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.persist_network_policy_amendment(amendment, network_approval_context)
                .await
                .map_err(|err| err.to_string())
        })
    }

    fn record_network_policy_amendment_message<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        amendment: &'a NetworkPolicyAmendment,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return;
            };
            self.record_network_policy_amendment_message(&turn.sub_id, amendment)
                .await;
        })
    }

    fn tool_permission_grants<'a>(&'a self) -> SessionCapabilityFuture<'a, ToolPermissionGrants> {
        Box::pin(async move {
            ToolPermissionGrants {
                session: self.granted_session_permissions().await,
                turn: self.granted_turn_permissions().await,
            }
        })
    }

    fn request_unified_exec_approval<'a>(
        &'a self,
        turn: &'a dyn thread_service_api::ThreadRuntimeCapability,
        call_id: String,
        command: Vec<String>,
        cwd: AbsolutePathBuf,
        reason: Option<String>,
        sandbox_permissions: protocol::models::SandboxPermissions,
        tty: bool,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cache_keys: Vec<thread_service_api::UnifiedExecApprovalKey>,
    ) -> SessionCapabilityFuture<'a, ReviewDecision> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("{}", invalid_turn_message());
                return ReviewDecision::Denied;
            };
            let strict_auto_review = self.strict_auto_review_enabled_for_turn().await;
            let review_with_guardian = turn.routes_approval_to_guardian() || strict_auto_review;

            if review_with_guardian {
                let Some(session) = session_arc(self) else {
                    tracing::warn!("{}", invalid_session_message());
                    return ReviewDecision::Denied;
                };
                return self
                    .services
                    .approval_service
                    .review_guardian_request(GuardianReviewDispatch {
                        session: session
                            as Arc<dyn codex_approval_service_api::ApprovalSessionCapability>,
                        turn: turn.self_arc()
                            as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
                        review_id: uuid::Uuid::new_v4().to_string(),
                        request: codex_guardian::GuardianApprovalRequest::ExecCommand {
                            id: call_id,
                            command,
                            cwd,
                            sandbox_permissions,
                            additional_permissions,
                            justification: reason.clone(),
                            tty,
                        },
                        retry_reason: reason,
                        approval_request_source:
                            codex_analytics_api::GuardianApprovalRequestSource::MainTurn,
                        cancellation_token: None,
                    })
                    .await
                    .decision;
            }

            let Some(session) = session_arc(self) else {
                tracing::warn!("{}", invalid_session_message());
                return ReviewDecision::Denied;
            };
            let turn = turn.self_arc();
            crate::session::session::approval_support_impl::with_cached_approval(
                &self.services,
                "unified_exec",
                cache_keys,
                || async move {
                    session
                        .request_command_approval(
                            turn.as_ref(),
                            call_id,
                            /*approval_id*/ None,
                            command,
                            cwd,
                            reason,
                            network_approval_context,
                            proposed_execpolicy_amendment,
                            additional_permissions,
                            /*available_decisions*/ None,
                        )
                        .await
                },
            )
            .await
        })
    }

    fn conversation_id(&self) -> protocol::ThreadId {
        self.conversation_id
    }
}

impl command_service_api::SessionCommandInteractionCaller for Session {
    fn begin_command_wait<'a>(
        &'a self,
        request: command_service_api::CommandWaitRequest,
    ) -> command_service_api::CommandServiceFuture<
        'a,
        Result<
            Box<dyn command_service_api::CommandWaitOperation>,
            command_service_api::CommandSessionError,
        >,
    > {
        Box::pin(async move { Session::begin_command_wait(self, request).await })
    }

    fn write_command_stdin<'a>(
        &'a self,
        request: command_service_api::WriteStdinRequest<'a>,
    ) -> command_service_api::CommandServiceFuture<
        'a,
        Result<command_service_api::WriteStdinOutput, command_service_api::CommandSessionError>,
    > {
        Box::pin(async move { Session::write_command_stdin(self, request).await })
    }
}

fn turn_context(turn: &dyn ThreadTurnCapability) -> Option<&TurnContext> {
    turn.as_any().downcast_ref::<TurnContext>()
}

fn session_arc(session: &Session) -> Option<Arc<Session>> {
    session.self_weak.get().and_then(std::sync::Weak::upgrade)
}

fn invalid_turn_message() -> String {
    "tool session capability received an unsupported turn context".to_string()
}

fn invalid_session_message() -> String {
    "tool session capability received an unsupported session runtime".to_string()
}
