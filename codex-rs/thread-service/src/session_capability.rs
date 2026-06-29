use crate::memory_usage::emit_metric_for_tool_read_parts;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tool_approval_support::permission_request_hook_payload;
use crate::tool_dispatch_trace::ToolDispatchTrace;
use codex_agent_runtime::CreateGoalRequest;
use codex_agent_runtime::SetGoalRequest;
use codex_hooks::PreToolUseHookResult;
use codex_hooks::run_permission_request_hooks;
use codex_guardian::GuardianApprovalRequest;
use codex_protocol::approvals::ExecPolicyAmendment;
use codex_protocol::approvals::NetworkApprovalContext;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::ResponseItem;
use codex_protocol::config_types::ApprovalsReviewer;
use codex_protocol::config_types::ModeKind;
use codex_protocol::dynamic_tools::DynamicToolCallRequest;
use codex_protocol::dynamic_tools::DynamicToolResponse;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ThreadGoal;
use codex_protocol::protocol::ThreadGoalStatus;
use codex_sandboxing_api::ResolvedApplyPatchEnvironment;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_sandboxing_api::ToolSandboxContext;
use codex_session_telemetry_api::SharedSessionTelemetry;
use thread_service_api::ApplyPatchDiffContext;
use thread_service_api::ReviewAssessmentRecord;
use thread_service_api::ReviewRuntimeError;
use thread_service_api::ReviewRuntimeOutcome;
use thread_service_api::ReviewRuntimeResult;
use thread_service_api::ReviewRejectionRecord;
use thread_service_api::PermissionRequestPayload;
use thread_service_api::PostToolUseHookOutcome;
use thread_service_api::PostToolUsePayload;
use thread_service_api::PreToolUseHookOutcome;
use thread_service_api::PreToolUsePayload;
use thread_service_api::SessionCapabilityFuture;
use thread_service_api::ToolPermissionGrants;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ToolSessionDispatchTrace;
use thread_service_api::ThreadDiscoveryContext;
use thread_service_api::ToolTelemetryTags;
use thread_service_api::ThreadTurnCapability;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolName;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use std::sync::Arc;
use tokio::sync::oneshot;

#[path = "approval_review_runtime.rs"]
mod approval_review_runtime_impl;

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

#[derive(serde::Serialize, serde::Deserialize)]
struct GuardianNetworkAccessTriggerPayload {
    call_id: String,
    tool_name: String,
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    sandbox_permissions: codex_protocol::models::SandboxPermissions,
    additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
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
        sandbox_permissions: codex_protocol::models::SandboxPermissions,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        justification: Option<String>,
    },
    #[cfg(unix)]
    Execve {
        id: String,
        source: codex_protocol::approvals::GuardianCommandSource,
        program: String,
        argv: Vec<String>,
        cwd: AbsolutePathBuf,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
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
        sandbox_permissions: codex_protocol::models::SandboxPermissions,
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        justification: Option<String>,
        tty: bool,
    },
    NetworkAccess {
        id: String,
        turn_id: String,
        target: String,
        host: String,
        protocol: codex_protocol::approvals::NetworkApprovalProtocol,
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
        permissions: codex_protocol::request_permissions::RequestPermissionProfile,
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

    fn get_thread_goal<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
        let session = self.session_arc();
        Box::pin(async move { session.get_thread_goal().await.map_err(format_goal_error) })
    }

    fn create_thread_goal<'a>(
        &'a self,
        objective: String,
        token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        let session = self.session_arc();
        Box::pin(async move {
            session
                .create_thread_goal(
                    self,
                    CreateGoalRequest {
                        objective,
                        token_budget,
                    },
                )
                .await
                .map_err(format_goal_error)
        })
    }

    fn complete_thread_goal<'a>(
        &'a self,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        let session = self.session_arc();
        Box::pin(async move {
            session
                .set_thread_goal(
                    self,
                    SetGoalRequest {
                        objective: None,
                        status: Some(ThreadGoalStatus::Complete),
                        token_budget: None,
                    },
                )
                .await
                .map_err(format_goal_error)
        })
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

    fn windows_sandbox_level(&self) -> codex_protocol::config_types::WindowsSandboxLevel {
        self.windows_sandbox_level()
    }

    fn tool_sandbox_context(&self) -> ToolSandboxContext {
        self.tool_sandbox_context()
    }

    fn resolve_apply_patch_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, codex_tool_types::FunctionCallError> {
        self.resolve_apply_patch_environment(environment_id)
    }

    fn thread_id(&self) -> codex_protocol::ThreadId {
        self.session_arc().thread_id()
    }

    fn runtime_turn_id_str(&self) -> &str {
        self.turn_id_str()
    }

    fn truncation_policy(&self) -> TruncationPolicy {
        self.truncation_policy()
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
        args: codex_protocol::request_permissions::RequestPermissionsArgs,
        cancellation_token: tokio_util::sync::CancellationToken,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_permissions::RequestPermissionsResponse>,
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
        args: codex_protocol::request_user_input::RequestUserInputArgs,
    ) -> SessionCapabilityFuture<
        'a,
        Option<codex_protocol::request_user_input::RequestUserInputResponse>,
    > {
        Box::pin(async move { self.session_arc().request_user_input(self, call_id, args).await })
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

fn format_goal_error(err: anyhow::Error) -> String {
    let mut message = err.to_string();
    for cause in err.chain().skip(1) {
        message.push_str(": ");
        message.push_str(&cause.to_string());
    }
    message
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

impl ApplyPatchDiffContext for TurnContext {
    fn apply_patch_streaming_events_enabled(&self) -> bool {
        self.is_apply_patch_streaming_events_enabled()
    }
}

impl ThreadSessionCapability for Session {
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }

    fn into_any_arc(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
        self
    }

    fn conversation_id(&self) -> codex_protocol::ThreadId {
        self.conversation_id
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
        tool_name: &'a ToolName,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(invalid_turn_message());
            };
            self.account_goal_tool_completed(turn, tool_name).await
        })
    }

    fn emit_event<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        event: EventMsg,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
        let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
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
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
                tracing::warn!("tool session capability received an unsupported turn context");
                return;
            };
            self.record_model_items_and_emit_display_events(turn, &items)
                .await;
        })
    }

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ThreadTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> SessionCapabilityFuture<'a, Option<codex_hooks_api::PermissionRequestDecision>> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
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

    fn sandbox_runtime(&self) -> SharedSandboxRuntime {
        self.sandbox_runtime()
    }

    fn strict_auto_review_enabled_for_turn<'a>(&'a self) -> SessionCapabilityFuture<'a, bool> {
        Box::pin(async move { self.strict_auto_review_enabled_for_turn().await })
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
                        crate::guardian::GuardianRejection {
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
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
                tracing::warn!("{}", invalid_turn_message());
                return ReviewRuntimeResult {
                    outcome: ReviewRuntimeOutcome::Error(
                        ReviewRuntimeError::Cancelled,
                    ),
                    analytics_result: codex_analytics_api::GuardianReviewAnalyticsResult::without_session(),
                };
            };
            let Ok(request) = serde_json::from_value::<GuardianApprovalRequestPayload>(request)
            else {
                tracing::warn!("tool session capability received an invalid guardian approval request");
                return ReviewRuntimeResult {
                    outcome: ReviewRuntimeOutcome::Error(
                        ReviewRuntimeError::PromptBuild {
                            message: "invalid guardian approval request".to_string(),
                        },
                    ),
                    analytics_result: codex_analytics_api::GuardianReviewAnalyticsResult::without_session(),
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
                    annotations: annotations.map(|annotations| codex_guardian::GuardianMcpAnnotations {
                        destructive_hint: annotations.destructive_hint,
                        open_world_hint: annotations.open_world_hint,
                        read_only_hint: annotations.read_only_hint,
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
            let (outcome, analytics_result) =
                approval_review_runtime_impl::run_review_session(session, turn, request, retry_reason, codex_guardian::guardian_output_schema(), None).await;
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
                        approval_review_runtime_impl::GuardianReviewError::PromptBuild { message } => {
                            ReviewRuntimeError::PromptBuild { message }
                        }
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
                tracing::warn!("tool session capability lost owning session while recording guardian non-denial");
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
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
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
            let crate::guardian::GuardianRejectionCircuitBreakerAction::InterruptTurn {
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
                    EventMsg::GuardianWarning(codex_protocol::protocol::WarningEvent {
                        message: format!(
                            "Automatic approval review rejected too many approval requests for this turn ({consecutive_denials} consecutive, {recent_denials} in the last {} reviews); interrupting the turn.",
                            crate::guardian::AUTO_REVIEW_DENIAL_WINDOW_SIZE
                        ),
                    }),
                )
                .await;

            let runtime_handle = session.services.runtime_handle.clone();
            let session = Arc::clone(&session);
            let turn_id = turn_id.to_string();
            let _abort_task = runtime_handle.spawn(async move {
                session
                    .abort_turn_if_active(&turn_id, codex_protocol::protocol::TurnAbortReason::Interrupted)
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
        additional_permissions: Option<codex_protocol::models::AdditionalPermissionProfile>,
        available_decisions: Option<Vec<codex_protocol::protocol::ReviewDecision>>,
    ) -> SessionCapabilityFuture<'a, codex_protocol::protocol::ReviewDecision> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
                tracing::warn!("{}", invalid_turn_message());
                return codex_protocol::protocol::ReviewDecision::Abort;
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
        changes: std::collections::HashMap<std::path::PathBuf, codex_protocol::protocol::FileChange>,
        reason: Option<String>,
        grant_root: Option<std::path::PathBuf>,
    ) -> SessionCapabilityFuture<'a, codex_protocol::protocol::ReviewDecision> {
        Box::pin(async move {
            let Some(turn) = ThreadTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
                tracing::warn!("{}", invalid_turn_message());
                return codex_protocol::protocol::ReviewDecision::Abort;
            };
            let rx_approve = Session::request_patch_approval(
                self,
                turn,
                call_id,
                changes,
                reason,
                grant_root,
            )
                .await;
            rx_approve.await.unwrap_or_default()
        })
    }

    fn cached_approval_decision<'a>(
        &'a self,
        key: String,
    ) -> SessionCapabilityFuture<'a, Option<codex_protocol::protocol::ReviewDecision>> {
        Box::pin(async move {
            let store = self.services.tool_approvals.lock().await;
            store.get(&key)
        })
    }

    fn cache_approval_decision<'a>(
        &'a self,
        keys: Vec<String>,
        decision: codex_protocol::protocol::ReviewDecision,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            if !matches!(decision, codex_protocol::protocol::ReviewDecision::ApprovedForSession) {
                return;
            }
            let mut store = self.services.tool_approvals.lock().await;
            for key in keys {
                store.put(key, codex_protocol::protocol::ReviewDecision::ApprovedForSession);
            }
        })
    }

    fn record_approval_request_telemetry<'a>(
        &'a self,
        tool_name: &'a str,
        decision: &'a codex_protocol::protocol::ReviewDecision,
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

    fn subscribe_out_of_band_elicitation_pause_state(&self) -> tokio::sync::watch::Receiver<bool> {
        self.subscribe_out_of_band_elicitation_pause_state()
    }

    fn tool_permission_grants<'a>(&'a self) -> SessionCapabilityFuture<'a, ToolPermissionGrants> {
        Box::pin(async move {
            ToolPermissionGrants {
                session: self.granted_session_permissions().await,
                turn: self.granted_turn_permissions().await,
            }
        })
    }
}

fn turn_context(turn: &dyn ThreadTurnCapability) -> Option<&TurnContext> {
    turn.as_any().downcast_ref::<TurnContext>()
}

fn invalid_turn_message() -> String {
    "tool session capability received an unsupported turn context".to_string()
}
