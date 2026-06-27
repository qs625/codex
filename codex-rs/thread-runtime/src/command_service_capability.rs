use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;

use codex_command_service_api::CommandServiceFuture;
use codex_command_service_api::CommandServiceSessionCapability;
use codex_command_service_api::CommandServiceSessionState;
use codex_command_service_api::CommandServiceTurnCapability;
use codex_hooks::run_permission_request_hooks;
use codex_hooks_api::PermissionRequestDecision;
use codex_protocol::ThreadId;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandBeginEvent;
use codex_protocol::protocol::ExecCommandEndEvent;
use codex_protocol::protocol::NetworkApprovalContext;
use codex_protocol::protocol::ReviewDecision;
use codex_thread_api::ToolRuntimeNetworkApprovalHandle;
use codex_thread_api::ToolRuntimeNetworkApprovalTrigger;
use codex_tool_runtime_api::NetworkApprovalSpec;
use codex_tool_runtime_api::PermissionRequestPayload;
use codex_tool_runtime_api::ResolvedExecCommand;
use codex_tool_runtime_api::ResolvedExecCommandEnvironment;
use crate::network_approval::DeferredNetworkApproval;
use crate::network_approval::begin_network_approval;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tool_approval_support::permission_request_hook_payload;
use crate::tool_approval_support::with_cached_approval;

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

impl codex_thread_api::ToolRuntimeNetworkApprovalHandle for SessionToolNetworkApprovalHandle {
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

    fn finish<'a>(&'a self) -> Pin<Box<dyn Future<Output = Result<(), codex_tool_runtime_api::ToolError>> + Send + 'a>> {
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

fn turn_context(turn: &dyn CommandServiceTurnCapability) -> Option<&TurnContext> {
    turn.as_any().downcast_ref::<TurnContext>()
}

fn session_arc(session: &Session) -> Arc<Session> {
    session
        .self_weak
        .get()
        .and_then(std::sync::Weak::upgrade)
        .expect("Session self_weak must be initialized")
}

impl CommandServiceTurnCapability for TurnContext {
    fn runtime_turn_id_str(&self) -> &str {
        self.turn_id_str()
    }

    fn runtime_turn_id(&self) -> String {
        self.turn_id()
    }

    fn can_request_original_image_detail(&self) -> bool {
        self.can_request_original_image_detail()
    }

    fn resolve_environment(
        &self,
        environment_id: Option<&str>,
    ) -> Result<Option<codex_tool_runtime_api::ResolvedApplyPatchEnvironment>, codex_tool_types::FunctionCallError> {
        self.resolve_apply_patch_environment(environment_id)
    }

    fn file_system_sandbox_context(
        &self,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cwd: &codex_utils_absolute_path::AbsolutePathBuf,
    ) -> codex_file_system::FileSystemSandboxContext {
        TurnContext::file_system_sandbox_context(self, additional_permissions, cwd)
    }

    fn single_local_environment_cwd(
        &self,
    ) -> Result<codex_utils_absolute_path::AbsolutePathBuf, codex_tool_types::FunctionCallError> {
        TurnContext::single_local_environment_cwd(self)
    }

    fn default_agent_job_max_runtime_seconds(&self) -> Option<u64> {
        TurnContext::default_agent_job_max_runtime_seconds(self)
    }

    fn routes_approval_to_guardian(&self) -> bool {
        crate::guardian::routes_approval_to_guardian(self)
    }

    fn tool_sandbox_context(&self) -> codex_tool_runtime_api::ToolSandboxContext {
        self.tool_sandbox_context()
    }

    fn approval_policy(&self) -> AskForApproval {
        self.approval_policy()
    }

    fn shell_environment_policy(&self) -> codex_protocol::config_types::ShellEnvironmentPolicy {
        self.shell_environment_policy.clone()
    }

    fn unified_exec_shell_mode(&self) -> codex_tool_config::UnifiedExecShellMode {
        self.unified_exec_shell_mode()
    }

    fn allow_login_shell(&self) -> bool {
        self.allow_login_shell()
    }

    fn active_network(&self) -> Option<codex_network_proxy_api::SharedNetworkProxyRuntime> {
        self.managed_network()
    }

    fn emit_unified_exec_tty_metric(&self, tty: bool) {
        self.emit_unified_exec_tty_metric(tty);
    }

    fn permission_profile(&self) -> PermissionProfile {
        self.permission_profile()
    }

    fn file_system_sandbox_policy(&self) -> codex_protocol::permissions::FileSystemSandboxPolicy {
        self.file_system_sandbox_policy()
    }

    fn resolve_exec_command_environment(
        &self,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, codex_tool_types::FunctionCallError> {
        self.resolve_exec_command_environment(environment_id, workdir)
    }

    fn truncation_policy(&self) -> codex_utils_output_truncation::TruncationPolicy {
        self.truncation_policy()
    }
}

impl CommandServiceSessionCapability for Session {
    fn conversation_id(&self) -> ThreadId {
        self.conversation_id
    }

    fn command_service_state(&self) -> Arc<dyn CommandServiceSessionState> {
        Arc::clone(&self.services.command_service_state)
    }

    fn sandbox_runtime(&self) -> codex_sandboxing_api::SharedSandboxRuntime {
        self.sandbox_runtime()
    }

    fn runtime_shell(&self) -> codex_tool_runtime_api::RuntimeShell {
        self.runtime_shell()
    }

    fn tool_user_shell_type(&self) -> codex_tool_config::ToolUserShellType {
        self.tool_user_shell_type()
    }

    fn subscribe_out_of_band_elicitation_pause_state(&self) -> tokio::sync::watch::Receiver<bool> {
        self.subscribe_out_of_band_elicitation_pause_state()
    }

    fn create_exec_approval_requirement<'a>(
        &'a self,
        request: codex_permissions_runtime::ExecPolicyApprovalRequest<'a>,
    ) -> CommandServiceFuture<'a, codex_tool_runtime_api::ExecApprovalRequirement> {
        Box::pin(async move { self.create_exec_approval_requirement(request).await })
    }

    fn strict_auto_review_enabled_for_turn<'a>(&'a self) -> CommandServiceFuture<'a, bool> {
        Box::pin(async move { self.strict_auto_review_enabled_for_turn().await })
    }

    fn guardian_rejection_message<'a>(
        &'a self,
        review_id: &'a str,
    ) -> CommandServiceFuture<'a, String> {
        Box::pin(async move { crate::guardian::guardian_rejection_message(self, review_id).await })
    }

    fn guardian_timeout_message(&self) -> String {
        crate::guardian::guardian_timeout_message()
    }

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        command: &'a str,
        workdir: &'a codex_utils_absolute_path::AbsolutePathBuf,
    ) -> CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("command service capability received an unsupported turn context");
                return;
            };
            self.maybe_emit_implicit_skill_invocation(turn, command, workdir)
                .await;
        })
    }

    fn exec_permission_approvals_enabled(&self) -> bool {
        self.enabled(codex_features::Feature::ExecPermissionApprovals)
    }

    fn request_permissions_tool_enabled(&self) -> bool {
        self.enabled(codex_features::Feature::RequestPermissionsTool)
    }

    fn tool_permission_grants<'a>(
        &'a self,
    ) -> CommandServiceFuture<'a, codex_tool_runtime_api::ToolPermissionGrants> {
        Box::pin(async move { self.tool_permission_grants().await })
    }

    fn resolve_model_shell(&self, shell: &std::path::Path) -> codex_tool_runtime_api::RuntimeShell {
        let mut shell = crate::runtime_shell_model::get_shell_by_model_provided_path(
            &shell.to_path_buf(),
        );
        shell.shell_snapshot = crate::runtime_shell_model::empty_shell_snapshot_receiver();
        crate::runtime_shell::runtime_shell(&shell)
    }

    fn resolve_exec_command(
        &self,
        turn: &dyn CommandServiceTurnCapability,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&codex_tool_runtime_api::RuntimeShell>,
    ) -> Result<ResolvedExecCommand, String> {
        let Some(turn) = turn_context(turn) else {
            return Err("command service capability received an unsupported turn context".to_string());
        };
        self.resolve_exec_command(turn, command, login, model_shell)
    }

    fn shell_env_overrides(&self) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    fn resolve_shell_workdir(
        &self,
        workdir: Option<String>,
    ) -> codex_utils_absolute_path::AbsolutePathBuf {
        workdir
            .and_then(|path| {
                codex_utils_absolute_path::AbsolutePathBuf::try_from(std::path::PathBuf::from(path))
                    .ok()
            })
            .unwrap_or_else(|| {
                codex_utils_absolute_path::AbsolutePathBuf::try_from(
                    std::env::current_dir().expect("current_dir should be available"),
                )
                .expect("current_dir should be absolute")
            })
    }

    fn run_permission_request_hooks<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        permission_request_run_id: &'a str,
        permission_request: PermissionRequestPayload,
    ) -> CommandServiceFuture<'a, Option<PermissionRequestDecision>> {
        Box::pin(async move {
            let Some(turn) = turn_context(turn) else {
                tracing::warn!("command service capability received an unsupported turn context");
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

    fn begin_tool_network_approval<'a>(
        &'a self,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> CommandServiceFuture<'a, Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>> {
        Box::pin(async move {
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
        })
    }

    fn request_command_approval<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        call_id: String,
        approval_id: Option<String>,
        command: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        reason: Option<String>,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<codex_protocol::approvals::ExecPolicyAmendment>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        available_decisions: Option<Vec<ReviewDecision>>,
    ) -> CommandServiceFuture<'a, ReviewDecision> {
        Box::pin(async move {
            let turn = turn_context(turn).expect("command service approval requires TurnContext");
            self.request_command_approval(
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

    fn request_unified_exec_approval<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        call_id: String,
        command: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        reason: Option<String>,
        sandbox_permissions: codex_protocol::models::SandboxPermissions,
        tty: bool,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<codex_protocol::approvals::ExecPolicyAmendment>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cache_keys: Vec<codex_tool_runtime_api::UnifiedExecApprovalKey>,
    ) -> CommandServiceFuture<'a, ReviewDecision> {
        Box::pin(async move {
            let turn = turn_context(turn).expect("command service approval requires TurnContext");
            let strict_auto_review = self.strict_auto_review_enabled_for_turn().await;
            let review_with_guardian =
                turn.routes_approval_to_guardian() || strict_auto_review;

            if review_with_guardian {
                return crate::guardian::review_approval_request(
                    &session_arc(self),
                    &turn.self_arc(),
                    uuid::Uuid::new_v4().to_string(),
                    crate::guardian::GuardianApprovalRequest::ExecCommand {
                        id: call_id,
                        command,
                        cwd,
                        sandbox_permissions,
                        additional_permissions,
                        justification: reason.clone(),
                        tty,
                    },
                    reason,
                )
                .await;
            }

            let session = session_arc(self);
            let turn = turn.self_arc();
            with_cached_approval(&self.services, "unified_exec", cache_keys, || async move {
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
            })
            .await
        })
    }

    fn unregister_network_approval<'a>(
        &'a self,
        registration_id: &'a str,
    ) -> CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            self.services
                .network_approval
                .unregister_call(registration_id)
                .await;
        })
    }

    fn send_exec_command_begin<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        event: ExecCommandBeginEvent,
    ) -> CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            let turn = turn_context(turn).expect("exec begin requires TurnContext");
            self.send_event(turn, EventMsg::ExecCommandBegin(event)).await;
        })
    }

    fn send_exec_command_end<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        event: ExecCommandEndEvent,
    ) -> CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            let turn = turn_context(turn).expect("exec end requires TurnContext");
            self.send_event(turn, EventMsg::ExecCommandEnd(event)).await;
        })
    }

    fn send_event<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        event: EventMsg,
    ) -> CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            let turn = turn_context(turn).expect("event send requires TurnContext");
            self.send_event(turn, event).await;
        })
    }

    fn record_model_items_and_emit_display_events<'a>(
        &'a self,
        turn: &'a dyn CommandServiceTurnCapability,
        items: &'a [ResponseItem],
    ) -> CommandServiceFuture<'a, ()> {
        Box::pin(async move {
            let turn = turn_context(turn).expect("record items requires TurnContext");
            self.record_model_items_and_emit_display_events(turn, items)
                .await;
        })
    }
}
