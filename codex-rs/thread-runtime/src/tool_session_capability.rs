use crate::memory_usage::emit_metric_for_tool_read_parts;
use crate::network_approval::DeferredNetworkApproval;
use crate::network_approval::begin_network_approval;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tool_dispatch_trace::ToolDispatchTrace;
use crate::tool_approval_support::permission_request_hook_payload;
use codex_hooks::PreToolUseHookResult;
use codex_hooks::run_permission_request_hooks;
use codex_thread_api::ApplyPatchSessionCapability;
use codex_thread_api::ApplyPatchTurnCapability;
use codex_thread_api::ApplyPatchEnvironment;
use codex_thread_api::PostToolUseHookOutcome;
use codex_thread_api::PostToolUsePayload;
use codex_thread_api::PreToolUseHookOutcome;
use codex_thread_api::PreToolUsePayload;
use codex_thread_api::PermissionRequestPayload;
use codex_thread_api::ResolvedApplyPatchEnvironment;
use codex_thread_api::ResolvedExecCommandEnvironment;
use codex_thread_api::SessionCapabilityFuture;
use codex_thread_api::ToolPermissionGrants;
use codex_thread_api::ToolSessionCapability;
use codex_thread_api::ToolSessionDispatchTrace;
use codex_thread_api::ToolEventSessionCapability;
use codex_thread_api::ToolEventTurnCapability;
use codex_thread_api::NetworkApprovalSpec;
use codex_thread_api::ToolRuntimeNetworkApprovalHandle;
use codex_thread_api::ToolRuntimeNetworkApprovalError;
use codex_thread_api::ToolRuntimeNetworkApprovalTrigger;
use codex_thread_api::ToolRuntimeSessionCapability;
use codex_thread_api::ToolSandboxContext;
use codex_thread_api::ToolRuntimeTurnCapability;
use codex_thread_api::ToolTelemetryTags;
use codex_thread_api::ToolTurnCapability;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::ResponseItem;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::TurnDiffEvent;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_session_telemetry_api::SharedSessionTelemetry;
use codex_tool_types::ToolCallSource;
use codex_tool_types::ToolName;
use codex_tool_types::ToolPayload;
use codex_thread_api::ApplyPatchDiffContext;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use codex_permissions_runtime::ExecApprovalRequirement;
use codex_permissions_runtime::ExecPolicyApprovalRequest;
use codex_file_system::FileSystemSandboxContext;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

struct RuntimeApplyPatchEnvironmentAdapter {
    inner: Arc<dyn codex_command_service_api::ApplyPatchEnvironment>,
}

impl ApplyPatchEnvironment for RuntimeApplyPatchEnvironmentAdapter {
    fn environment_id(&self) -> &str {
        self.inner.environment_id()
    }

    fn filesystem(&self) -> Arc<dyn codex_file_system::ExecutorFileSystem> {
        self.inner.filesystem()
    }
}

fn to_thread_tool_sandbox_context(
    value: codex_command_service_api::ToolSandboxContext,
) -> ToolSandboxContext {
    ToolSandboxContext {
        turn_id: value.turn_id,
        telemetry: value.telemetry,
        file_system_sandbox_policy: value.file_system_sandbox_policy,
        network_sandbox_policy: value.network_sandbox_policy,
        permission_profile: value.permission_profile,
        managed_network_active: value.managed_network_active,
        cwd: value.cwd,
        codex_linux_sandbox_exe: value.codex_linux_sandbox_exe,
        use_legacy_landlock: value.use_legacy_landlock,
        windows_sandbox_level: value.windows_sandbox_level,
        windows_sandbox_private_desktop: value.windows_sandbox_private_desktop,
    }
}

fn to_thread_resolved_exec_command_environment(
    value: codex_command_service_api::ResolvedExecCommandEnvironment,
) -> ResolvedExecCommandEnvironment {
    ResolvedExecCommandEnvironment {
        cwd: value.cwd,
        sandbox_cwd: value.sandbox_cwd,
        environment: value.environment,
        apply_patch_environment: Arc::new(RuntimeApplyPatchEnvironmentAdapter {
            inner: value.apply_patch_environment,
        }),
    }
}

impl ToolTurnCapability for TurnContext {
    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
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
}

impl ApplyPatchDiffContext for TurnContext {
    fn apply_patch_streaming_events_enabled(&self) -> bool {
        self.is_apply_patch_streaming_events_enabled()
    }
}

impl ToolEventTurnCapability for TurnContext {
    fn runtime_turn_id_str(&self) -> &str {
        self.turn_id_str()
    }

    fn truncation_policy(&self) -> TruncationPolicy {
        self.truncation_policy()
    }
}

impl ApplyPatchTurnCapability for TurnContext {
    fn approval_policy(&self) -> AskForApproval {
        self.approval_policy()
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
}

impl ToolRuntimeTurnCapability for TurnContext {
    fn runtime_turn_id_str(&self) -> &str {
        ToolEventTurnCapability::runtime_turn_id_str(self)
    }

    fn routes_approval_to_guardian(&self) -> bool {
        crate::guardian::routes_approval_to_guardian(self)
    }

    fn tool_sandbox_context(&self) -> ToolSandboxContext {
        self.tool_sandbox_context()
    }

    fn approval_policy(&self) -> AskForApproval {
        ApplyPatchTurnCapability::approval_policy(self)
    }

    fn permission_profile(&self) -> PermissionProfile {
        ApplyPatchTurnCapability::permission_profile(self)
    }

    fn file_system_sandbox_policy(&self) -> FileSystemSandboxPolicy {
        ApplyPatchTurnCapability::file_system_sandbox_policy(self)
    }

    fn windows_sandbox_level(&self) -> codex_protocol::config_types::WindowsSandboxLevel {
        ApplyPatchTurnCapability::windows_sandbox_level(self)
    }

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cwd: &AbsolutePathBuf,
    ) -> FileSystemSandboxContext {
        self.file_system_sandbox_context(additional_permissions, cwd)
    }

    fn resolve_apply_patch_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<ResolvedApplyPatchEnvironment>, codex_tool_types::FunctionCallError> {
        ApplyPatchTurnCapability::resolve_apply_patch_environment(self, environment_id)
    }

    fn primary_apply_patch_environment(&self) -> Option<ResolvedApplyPatchEnvironment> {
        self.primary_apply_patch_environment()
    }

    fn explicit_shell_env_overrides(&self) -> HashMap<String, String> {
        self.explicit_shell_env_overrides()
    }

    fn resolve_shell_workdir(&self, workdir: Option<String>) -> AbsolutePathBuf {
        self.resolve_shell_workdir(workdir)
    }

    fn legacy_cwd(&self) -> AbsolutePathBuf {
        self.legacy_cwd()
    }

    fn resolve_exec_command_environment(
        &self,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, codex_tool_types::FunctionCallError> {
        self.resolve_exec_command_environment(environment_id, workdir)
            .map(|value| value.map(to_thread_resolved_exec_command_environment))
    }

    fn truncation_policy(&self) -> TruncationPolicy {
        ToolEventTurnCapability::truncation_policy(self)
    }

    fn allow_login_shell(&self) -> bool {
        self.allow_login_shell()
    }

    fn emit_unified_exec_tty_metric(&self, tty: bool) {
        self.emit_unified_exec_tty_metric(tty);
    }
}

impl ToolSessionCapability for Session {
    fn tool_dispatch_telemetry(&self, turn: &dyn ToolTurnCapability) -> SharedSessionTelemetry {
        turn.tool_dispatch_telemetry()
    }

    fn base_tool_result_tags(&self, turn: &dyn ToolTurnCapability) -> ToolTelemetryTags {
        turn.base_tool_result_tags()
    }

    fn record_tool_call_started<'a>(
        &'a self,
        _turn: &'a dyn ToolTurnCapability,
    ) -> SessionCapabilityFuture<'a, ()> {
        Box::pin(async move {
            self.record_tool_call_started().await;
        })
    }

    fn start_tool_dispatch_trace(
        &self,
        turn: &dyn ToolTurnCapability,
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
        turn: &'a dyn ToolTurnCapability,
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
        turn: &'a dyn ToolTurnCapability,
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
        turn: &'a dyn ToolTurnCapability,
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
        turn: &'a dyn ToolTurnCapability,
        tool_name: &'a ToolName,
    ) -> SessionCapabilityFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                return Err(invalid_turn_message());
            };
            self.account_goal_tool_completed(turn, tool_name).await
        })
    }
}

enum SessionToolNetworkApprovalState {
    Immediate(Mutex<Option<String>>),
    Deferred(DeferredNetworkApproval),
}

struct SessionToolNetworkApprovalHandle {
    service: Arc<crate::network_approval::NetworkApprovalService>,
    mode: codex_thread_api::NetworkApprovalMode,
    cancellation_token: tokio_util::sync::CancellationToken,
    state: SessionToolNetworkApprovalState,
}

impl ToolRuntimeNetworkApprovalHandle for SessionToolNetworkApprovalHandle {
    fn mode(&self) -> codex_thread_api::NetworkApprovalMode {
        self.mode
    }

    fn registration_id(&self) -> Option<String> {
        match &self.state {
            SessionToolNetworkApprovalState::Immediate(registration_id) => registration_id
                .lock()
                .expect("network approval state mutex poisoned")
                .clone(),
            SessionToolNetworkApprovalState::Deferred(deferred) => {
                Some(deferred.registration_id().to_string())
            }
        }
    }

    fn cancellation_token(&self) -> tokio_util::sync::CancellationToken {
        self.cancellation_token.clone()
    }

    fn finish<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), ToolRuntimeNetworkApprovalError>> + Send + 'a>> {
        Box::pin(async move {
            match &self.state {
                SessionToolNetworkApprovalState::Immediate(registration_id) => {
                    let Some(registration_id) = registration_id
                        .lock()
                        .expect("network approval state mutex poisoned")
                        .take()
                    else {
                        return Ok(());
                    };
                    self.service
                        .finish_call(&registration_id)
                        .await
                        .map_err(map_network_approval_error)
                }
                SessionToolNetworkApprovalState::Deferred(deferred) => {
                    deferred
                        .finish(&self.service)
                        .await
                        .map_err(map_network_approval_error)
                }
            }
        })
    }
}

fn map_network_approval_error(
    err: crate::tool_approval_support::ToolError,
) -> ToolRuntimeNetworkApprovalError {
    match err {
        crate::tool_approval_support::ToolError::Rejected(message) => {
            ToolRuntimeNetworkApprovalError::Rejected(message)
        }
        crate::tool_approval_support::ToolError::Codex(err) => {
            ToolRuntimeNetworkApprovalError::Codex(err)
        }
    }
}

fn map_network_trigger(
    trigger: ToolRuntimeNetworkApprovalTrigger,
) -> crate::guardian::GuardianNetworkAccessTrigger {
    crate::guardian::GuardianNetworkAccessTrigger {
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

impl ToolEventSessionCapability for Session {
    async fn tool_send_exec_command_begin(
        &self,
        turn: &dyn ToolEventTurnCapability,
        event: ExecCommandBeginEvent,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        self.send_event(turn, EventMsg::ExecCommandBegin(event)).await;
    }

    async fn tool_send_exec_command_end(
        &self,
        turn: &dyn ToolEventTurnCapability,
        event: ExecCommandEndEvent,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        self.send_event(turn, EventMsg::ExecCommandEnd(event)).await;
    }

    async fn tool_emit_file_change_started(
        &self,
        turn: &dyn ToolEventTurnCapability,
        item: codex_protocol::items::FileChangeItem,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        self.emit_turn_item_started(turn, &codex_protocol::items::TurnItem::FileChange(item))
            .await;
    }

    async fn tool_emit_file_change_completed(
        &self,
        turn: &dyn ToolEventTurnCapability,
        item: codex_protocol::items::FileChangeItem,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        self.emit_turn_item_completed(turn, codex_protocol::items::TurnItem::FileChange(item))
            .await;
    }

    async fn tool_record_model_items_and_emit_display_events(
        &self,
        turn: &dyn ToolEventTurnCapability,
        items: Vec<ResponseItem>,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        self.record_model_items_and_emit_display_events(turn, &items)
            .await;
    }

    async fn tool_emit_turn_diff(
        &self,
        turn: &dyn ToolEventTurnCapability,
        event: TurnDiffEvent,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        self.send_event(turn, EventMsg::TurnDiff(event)).await;
    }

}

impl ApplyPatchSessionCapability for Session {
    fn sandbox_runtime(&self) -> SharedSandboxRuntime {
        self.sandbox_runtime()
    }

    async fn tool_permission_grants(&self) -> ToolPermissionGrants {
        ToolPermissionGrants {
            session: self.granted_session_permissions().await,
            turn: self.granted_turn_permissions().await,
        }
    }

    fn strict_auto_review_enabled_for_turn(&self) -> impl Future<Output = bool> + Send + '_ {
        self.strict_auto_review_enabled_for_turn()
    }

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ApplyPatchTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> impl Future<Output = Option<codex_hooks_api::PermissionRequestDecision>> + Send + 'a {
        async move {
            let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
                tracing::warn!(
                    "tool session capability received an unsupported turn context"
                );
                return None;
            };
            run_permission_request_hooks(
                self,
                turn,
                permission_request_run_id,
                permission_request_hook_payload(permission_request),
            )
            .await
        }
    }
}


impl ToolRuntimeSessionCapability for Session {
    fn sandbox_runtime(&self) -> SharedSandboxRuntime {
        ApplyPatchSessionCapability::sandbox_runtime(self)
    }

    async fn tool_send_exec_command_begin(
        &self,
        turn: &dyn ToolRuntimeTurnCapability,
        event: ExecCommandBeginEvent,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        ToolEventSessionCapability::tool_send_exec_command_begin(self, turn, event).await;
    }

    async fn tool_send_exec_command_end(
        &self,
        turn: &dyn ToolRuntimeTurnCapability,
        event: ExecCommandEndEvent,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        ToolEventSessionCapability::tool_send_exec_command_end(self, turn, event).await;
    }

    async fn tool_emit_file_change_started(
        &self,
        turn: &dyn ToolRuntimeTurnCapability,
        item: codex_protocol::items::FileChangeItem,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        ToolEventSessionCapability::tool_emit_file_change_started(self, turn, item).await;
    }

    async fn tool_emit_file_change_completed(
        &self,
        turn: &dyn ToolRuntimeTurnCapability,
        item: codex_protocol::items::FileChangeItem,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        ToolEventSessionCapability::tool_emit_file_change_completed(self, turn, item).await;
    }

    async fn tool_record_model_items_and_emit_display_events(
        &self,
        turn: &dyn ToolRuntimeTurnCapability,
        items: Vec<ResponseItem>,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        ToolEventSessionCapability::tool_record_model_items_and_emit_display_events(
            self, turn, items,
        )
        .await;
    }

    async fn tool_emit_turn_diff(
        &self,
        turn: &dyn ToolRuntimeTurnCapability,
        event: TurnDiffEvent,
    ) {
        let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
            tracing::warn!("tool session capability received an unsupported turn context");
            return;
        };
        ToolEventSessionCapability::tool_emit_turn_diff(self, turn, event).await;
    }

    async fn tool_permission_grants(&self) -> ToolPermissionGrants {
        ApplyPatchSessionCapability::tool_permission_grants(self).await
    }

    async fn dependency_env(&self) -> HashMap<String, String> {
        self.dependency_env().await
    }

    fn exec_permission_approvals_enabled(&self) -> bool {
        self.enabled(codex_features::Feature::ExecPermissionApprovals)
    }

    fn request_permissions_tool_enabled(&self) -> bool {
        self.enabled(codex_features::Feature::RequestPermissionsTool)
    }

    fn create_exec_approval_requirement<'a>(
        &'a self,
        request: ExecPolicyApprovalRequest<'a>,
    ) -> impl std::future::Future<Output = ExecApprovalRequirement> + Send + 'a {
        self.create_exec_approval_requirement(request)
    }

    fn strict_auto_review_enabled_for_turn(&self) -> impl Future<Output = bool> + Send + '_ {
        ApplyPatchSessionCapability::strict_auto_review_enabled_for_turn(self)
    }

    fn guardian_rejection_message<'a>(
        &'a self,
        review_id: &'a str,
    ) -> impl Future<Output = String> + Send + 'a {
        crate::guardian::guardian_rejection_message(self, review_id)
    }

    fn guardian_timeout_message(&self) -> String {
        crate::guardian::guardian_timeout_message()
    }

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn ToolRuntimeTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> impl Future<Output = Option<codex_hooks_api::PermissionRequestDecision>> + Send + 'a {
        ApplyPatchSessionCapability::run_permission_request_hooks(
            self,
            ToolTurnCapability::as_any(turn)
                .downcast_ref::<TurnContext>()
                .expect("tool session capability received an unsupported turn context"),
            permission_request_run_id,
            permission_request,
        )
    }

    fn begin_tool_network_approval<'a>(
        &'a self,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> impl Future<Output = Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>> + Send + 'a {
        async move {
            let spec = spec.map(|spec| crate::network_approval::NetworkApprovalSpec {
                network: spec.network,
                mode: spec.mode,
                trigger: map_network_trigger(spec.trigger),
                command: spec.command,
            });
            let active =
                begin_network_approval(self, turn_id, managed_network_active, spec).await?;
            let mode = active.mode();
            let cancellation_token = active.cancellation_token();
            let state = match mode {
                codex_thread_api::NetworkApprovalMode::Deferred => {
                    SessionToolNetworkApprovalState::Deferred(
                        active
                            .into_deferred()
                            .expect("deferred network approval should convert to deferred state"),
                    )
                }
                codex_thread_api::NetworkApprovalMode::Immediate => {
                    SessionToolNetworkApprovalState::Immediate(Mutex::new(
                        active.registration_id().map(ToString::to_string),
                    ))
                }
            };
            Some(Arc::new(SessionToolNetworkApprovalHandle {
                service: Arc::clone(&self.services.network_approval),
                mode,
                cancellation_token,
                state,
            }) as Arc<dyn ToolRuntimeNetworkApprovalHandle>)
        }
    }

}

fn turn_context(turn: &dyn ToolTurnCapability) -> Option<&TurnContext> {
    turn.as_any().downcast_ref::<TurnContext>()
}

fn invalid_turn_message() -> String {
    "tool session capability received an unsupported turn context".to_string()
}
