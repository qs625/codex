use crate::memory_usage::emit_metric_for_tool_read_parts;
use crate::network_approval::DeferredNetworkApproval;
use crate::network_approval::begin_network_approval;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::runtime_shell_model::get_shell_by_model_provided_path;
use crate::tool_dispatch_trace::ToolDispatchTrace;
use crate::runtime_shell::runtime_shell;
use crate::tool_approval_support::permission_request_hook_payload;
use codex_hooks::PreToolUseHookResult;
use codex_hooks::run_permission_request_hooks;
use codex_thread_api::ApplyPatchSessionCapability;
use codex_thread_api::ApplyPatchTurnCapability;
use codex_thread_api::SessionCapabilityFuture;
use codex_thread_api::ToolSessionCapability;
use codex_thread_api::ToolSessionDispatchTrace;
use codex_thread_api::ToolEventSessionCapability;
use codex_thread_api::ToolEventTurnCapability;
use codex_thread_api::ToolRuntimeNetworkApprovalHandle;
use codex_thread_api::ToolRuntimeNetworkApprovalTrigger;
use codex_thread_api::ToolRuntimeSessionCapability;
use codex_thread_api::ToolRuntimeTurnCapability;
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
use codex_tool_planning::ToolCallSource;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolPayload;
use codex_tool_runtime_api::ExecCommandRunOutput;
use codex_tool_runtime_api::ExecCommandRunRequest;
use codex_tool_runtime_api::NetworkApprovalSpec;
use codex_tool_runtime_api::ApplyPatchDiffContext;
use codex_tool_runtime_api::ExecCommandSessionRuntime;
use codex_tool_runtime_api::ResolvedExecCommand;
use codex_tool_runtime_api::ResolvedApplyPatchEnvironment;
use codex_tool_runtime_api::ResolvedExecCommandEnvironment;
use codex_tool_runtime_api::RuntimeShell;
use codex_tool_runtime_api::ToolError;
use codex_tool_runtime_api::ToolPermissionGrants;
use codex_tool_runtime_api::PostToolUseHookOutcome;
use codex_tool_runtime_api::PostToolUsePayload;
use codex_tool_runtime_api::PreToolUseHookOutcome;
use codex_tool_runtime_api::PreToolUsePayload;
use codex_tool_runtime_api::ToolTelemetryTags;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::TruncationPolicy;
use codex_command_runtime::UnifiedExecError;
use codex_permissions_runtime::ExecPolicyApprovalRequest;
use codex_file_system::FileSystemSandboxContext;
use std::collections::HashMap;
use std::pin::Pin;
use std::path::Path;
use std::sync::{Arc, Mutex};

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

    fn tool_sandbox_context(&self) -> codex_tool_runtime_api::ToolSandboxContext {
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

    fn tool_sandbox_context(&self) -> codex_tool_runtime_api::ToolSandboxContext {
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
    mode: codex_tool_runtime_api::NetworkApprovalMode,
    cancellation_token: tokio_util::sync::CancellationToken,
    state: SessionToolNetworkApprovalState,
}

impl ToolRuntimeNetworkApprovalHandle for SessionToolNetworkApprovalHandle {
    fn mode(&self) -> codex_tool_runtime_api::NetworkApprovalMode {
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

    fn finish<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), ToolError>> + Send + 'a>> {
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
                    self.service.finish_call(&registration_id).await
                }
                SessionToolNetworkApprovalState::Deferred(deferred) => {
                    deferred.finish(&self.service).await
                }
            }
        })
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
        permission_request: codex_tool_runtime_api::PermissionRequestPayload,
    ) -> impl Future<Output = Option<codex_hooks_api::PermissionRequestDecision>> + Send + 'a {
        async move {
            let Some(turn) = ToolTurnCapability::as_any(turn).downcast_ref::<TurnContext>() else {
                tracing::warn!(
                    "tool runtime session capability received an unsupported turn context"
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

impl ExecCommandSessionRuntime<TurnContext> for Session {
    fn tool_user_shell_type(&self) -> codex_tool_config::ToolUserShellType {
        self.tool_user_shell_type()
    }

    fn runtime_shell(&self) -> RuntimeShell {
        runtime_shell(self.user_shell().as_ref())
    }

    fn resolve_model_shell(&self, shell: &Path) -> RuntimeShell {
        let mut shell = get_shell_by_model_provided_path(&shell.to_path_buf());
        shell.shell_snapshot = crate::runtime_shell_model::empty_shell_snapshot_receiver();
        runtime_shell(&shell)
    }

    fn resolve_exec_command(
        &self,
        turn: &TurnContext,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&RuntimeShell>,
    ) -> Result<ResolvedExecCommand, String> {
        codex_tool_runtime_api::resolve_exec_command_for_parts(
            command,
            login,
            &self.runtime_shell(),
            model_shell,
            &turn.unified_exec_shell_mode(),
            turn.allow_login_shell(),
        )
    }

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        turn: &'a TurnContext,
        command: &'a str,
        workdir: &'a AbsolutePathBuf,
    ) -> impl std::future::Future<Output = ()> + Send + 'a {
        crate::maybe_emit_implicit_skill_invocation(self, turn, command, workdir)
    }

    fn allocate_exec_process_id(&self) -> impl std::future::Future<Output = i32> + Send + '_ {
        self.allocate_unified_exec_process_id()
    }

    fn release_exec_process_id(
        &self,
        process_id: i32,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        self.release_unified_exec_process_id(process_id)
    }

    fn run_exec_command<'a>(
        &'a self,
        turn: &'a TurnContext,
        call_id: &'a str,
        request: ExecCommandRunRequest,
    ) -> impl std::future::Future<Output = Result<ExecCommandRunOutput, UnifiedExecError>> + Send + 'a {
        let session = turn.session_arc();
        let turn = turn.self_arc();
        let call_id = call_id.to_string();
        async move { session.run_unified_exec_command(turn, call_id, request).await }
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
            tracing::warn!("tool runtime session capability received an unsupported turn context");
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
    ) -> impl std::future::Future<Output = codex_tool_runtime_api::ExecApprovalRequirement> + Send + 'a {
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
        permission_request: codex_tool_runtime_api::PermissionRequestPayload,
    ) -> impl Future<Output = Option<codex_hooks_api::PermissionRequestDecision>> + Send + 'a {
        ApplyPatchSessionCapability::run_permission_request_hooks(
            self,
            ToolTurnCapability::as_any(turn)
                .downcast_ref::<TurnContext>()
                .expect("tool runtime session capability received an unsupported turn context"),
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
            let spec = spec.map(|spec| NetworkApprovalSpec {
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
                codex_tool_runtime_api::NetworkApprovalMode::Deferred => {
                    SessionToolNetworkApprovalState::Deferred(
                        active
                            .into_deferred()
                            .expect("deferred network approval should convert to deferred state"),
                    )
                }
                codex_tool_runtime_api::NetworkApprovalMode::Immediate => {
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
