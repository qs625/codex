use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;

use crate::network_approval::DeferredNetworkApproval;
use crate::network_approval::begin_network_approval;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tool_approval_support::with_cached_approval;
use codex_command_service_api::CommandServiceFuture;
use codex_command_service_api::CommandServiceSessionApi;
use codex_command_service_api::CommandServiceSessionState;
use codex_command_service_api::ExecApprovalRequirement;
use codex_command_service_api::ResolvedExecCommand;
use codex_command_service_api::RuntimeShell;
use codex_command_service_api::UnifiedExecApprovalKey;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::protocol::NetworkApprovalContext;
use codex_protocol::protocol::ReviewDecision;
use thread_service_api::ThreadCapability;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::NetworkApprovalMode as CommandNetworkApprovalMode;
use thread_service_api::NetworkApprovalSpec;
use thread_service_api::ToolRuntimeNetworkApprovalError;
use thread_service_api::ToolRuntimeNetworkApprovalHandle;
use thread_service_api::ToolRuntimeNetworkApprovalTrigger;

enum SessionToolNetworkApprovalState {
    Immediate(Mutex<Option<String>>),
    Deferred(DeferredNetworkApproval),
}

struct SessionToolNetworkApprovalHandle {
    service: Arc<crate::network_approval::NetworkApprovalService>,
    mode: thread_service_api::NetworkApprovalMode,
    cancellation_token: tokio_util::sync::CancellationToken,
    state: SessionToolNetworkApprovalState,
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
    ) -> Pin<Box<dyn Future<Output = Result<(), ToolRuntimeNetworkApprovalError>> + Send + 'a>>
    {
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
                    self.service
                        .finish_call(&registration_id)
                        .await
                        .map_err(map_network_approval_error)
                }
                SessionToolNetworkApprovalState::Deferred(deferred) => deferred
                    .finish(&self.service)
                    .await
                    .map_err(map_network_approval_error),
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

fn to_command_runtime_shell(value: RuntimeShell) -> RuntimeShell {
    value
}

fn to_command_resolved_exec_command(value: ResolvedExecCommand) -> ResolvedExecCommand {
    ResolvedExecCommand {
        command: value.command,
        shell_type: value.shell_type,
    }
}

fn turn_context(turn: &dyn ThreadRuntimeCapability) -> Option<&TurnContext> {
    ThreadCapability::as_any(turn).downcast_ref::<TurnContext>()
}

fn session_arc(session: &Session) -> Arc<Session> {
    match session.self_weak.get().and_then(std::sync::Weak::upgrade) {
        Some(session) => session,
        None => panic!("Session self_weak must be initialized"),
    }
}

impl CommandServiceSessionApi for Session {
    fn command_service_state(&self) -> Arc<dyn CommandServiceSessionState> {
        Arc::clone(&self.services.command_service_state)
    }

    fn runtime_shell(&self) -> RuntimeShell {
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
    ) -> CommandServiceFuture<'a, ExecApprovalRequirement> {
        Box::pin(async move { self.create_exec_approval_requirement(request).await })
    }

    fn maybe_emit_implicit_skill_invocation<'a>(
        &'a self,
        turn: &'a dyn ThreadRuntimeCapability,
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

    fn resolve_model_shell(&self, shell: &std::path::Path) -> RuntimeShell {
        let mut shell =
            crate::runtime_shell_model::get_shell_by_model_provided_path(&shell.to_path_buf());
        shell.shell_snapshot = crate::runtime_shell_model::empty_shell_snapshot_receiver();
        to_command_runtime_shell(crate::runtime_shell::runtime_shell(&shell))
    }

    fn resolve_exec_command(
        &self,
        turn: &dyn ThreadRuntimeCapability,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&RuntimeShell>,
    ) -> Result<ResolvedExecCommand, String> {
        let Some(turn) = turn_context(turn) else {
            return Err(
                "command service capability received an unsupported turn context".to_string(),
            );
        };
        let session_shell = crate::runtime_shell::runtime_shell(self.user_shell().as_ref());
        codex_command_service_api::resolve_exec_command_for_parts(
            command,
            login,
            &session_shell,
            model_shell,
            &turn.unified_exec_shell_mode(),
            turn.allow_login_shell(),
        )
        .map(to_command_resolved_exec_command)
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
                let current_dir = std::env::current_dir()
                    .unwrap_or_else(|_| panic!("current_dir should be available"));
                codex_utils_absolute_path::AbsolutePathBuf::try_from(current_dir)
                    .unwrap_or_else(|_| panic!("current_dir should be absolute"))
            })
    }

    fn begin_tool_network_approval<'a>(
        &'a self,
        turn_id: &'a str,
        managed_network_active: bool,
        spec: Option<NetworkApprovalSpec<ToolRuntimeNetworkApprovalTrigger>>,
    ) -> CommandServiceFuture<'a, Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>> {
        Box::pin(async move {
            let spec = spec.map(|spec| crate::network_approval::NetworkApprovalSpec {
                network: spec.network,
                mode: match spec.mode {
                    CommandNetworkApprovalMode::Immediate => {
                        thread_service_api::NetworkApprovalMode::Immediate
                    }
                    CommandNetworkApprovalMode::Deferred => {
                        thread_service_api::NetworkApprovalMode::Deferred
                    }
                },
                trigger: map_network_trigger(spec.trigger),
                command: spec.command,
            });
            let active =
                begin_network_approval(self, turn_id, managed_network_active, spec).await?;
            let mode = active.mode();
            let cancellation_token = active.cancellation_token();
            let state = match mode {
                thread_service_api::NetworkApprovalMode::Deferred => {
                    let Some(deferred) = active.into_deferred() else {
                        panic!("deferred network approval should convert to deferred state");
                    };
                    SessionToolNetworkApprovalState::Deferred(deferred)
                }
                thread_service_api::NetworkApprovalMode::Immediate => {
                    SessionToolNetworkApprovalState::Immediate(Mutex::new(
                        active.registration_id().map(ToString::to_string),
                    ))
                }
            };
            Some(Arc::new(SessionToolNetworkApprovalHandle {
                service: Arc::clone(&self.services.network_approval),
                mode: match mode {
                    thread_service_api::NetworkApprovalMode::Immediate => {
                        thread_service_api::NetworkApprovalMode::Immediate
                    }
                    thread_service_api::NetworkApprovalMode::Deferred => {
                        thread_service_api::NetworkApprovalMode::Deferred
                    }
                },
                cancellation_token,
                state,
            }) as Arc<dyn ToolRuntimeNetworkApprovalHandle>)
        })
    }

    fn request_unified_exec_approval<'a>(
        &'a self,
        turn: &'a dyn ThreadRuntimeCapability,
        call_id: String,
        command: Vec<String>,
        cwd: codex_utils_absolute_path::AbsolutePathBuf,
        reason: Option<String>,
        sandbox_permissions: codex_protocol::models::SandboxPermissions,
        tty: bool,
        network_approval_context: Option<NetworkApprovalContext>,
        proposed_execpolicy_amendment: Option<codex_protocol::approvals::ExecPolicyAmendment>,
        additional_permissions: Option<AdditionalPermissionProfile>,
        cache_keys: Vec<UnifiedExecApprovalKey>,
    ) -> CommandServiceFuture<'a, ReviewDecision> {
        Box::pin(async move {
            let turn = required_turn_context(turn, "command service approval");
            let strict_auto_review = self.strict_auto_review_enabled_for_turn().await;
            let review_with_guardian = turn.routes_approval_to_guardian() || strict_auto_review;

            if review_with_guardian {
                return approval_service::guardian::review_approval_request(
                    session_arc(self).as_ref(),
                    turn.self_arc().as_ref(),
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

}

fn required_turn_context<'a>(
    turn: &'a dyn ThreadRuntimeCapability,
    operation: &str,
) -> &'a TurnContext {
    match turn_context(turn) {
        Some(turn) => turn,
        None => panic!("{operation} requires TurnContext"),
    }
}
