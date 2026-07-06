use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use tokio::time::Duration;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::exec_env::CODEX_THREAD_ID_ENV_VAR;
use crate::exec_env::create_env;
use crate::exec_request::ExecRequest;
use crate::runtime_support::SandboxAttempt;
use crate::runtime_support::SandboxAttemptExt;
use crate::runtime_support::SandboxOverride;
use crate::runtime_support::ToolError;
use crate::runtime_support::managed_network_for_sandbox_permissions;
use crate::runtime_support::sandbox_override_for_first_attempt;
use crate::runtime_support::wants_no_sandbox_approval;
use crate::shell_support::build_sandbox_command;
use crate::shell_support::disable_powershell_profile_for_elevated_windows_sandbox;
use crate::shell_support::exec_env_for_sandbox_permissions;
use crate::shell_support::maybe_wrap_shell_lc_with_snapshot;
use crate::unified_exec::CommandNotificationFilter;
use crate::unified_exec::CommandNotificationKind;
use crate::unified_exec::CommandNotificationSnapshot;
use crate::unified_exec::CommandNotificationState;
use crate::unified_exec::CommandProcessPruneMeta;
use crate::unified_exec::CommandWaitOutput;
use crate::unified_exec::CommandWaitRequest;
use crate::unified_exec::CommandWaitStatus;
use crate::unified_exec::ExecCommandRequest;
use crate::unified_exec::ExecServerEnvConfig;
use crate::unified_exec::ExecServerSpawnRequest;
use crate::unified_exec::HeadTailBuffer;
use crate::unified_exec::MAX_UNIFIED_EXEC_PROCESSES;
use crate::unified_exec::ProcessEntry;
use crate::unified_exec::ProcessExitSubscription;
use crate::unified_exec::ProcessStore;
use crate::unified_exec::SpawnLifecycleHandle;
use crate::unified_exec::UnifiedExecContext;
use crate::unified_exec::UnifiedExecError;
use crate::unified_exec::UnifiedExecProcess;
use crate::unified_exec::UnifiedExecProcessManager;
use crate::unified_exec::WaitBackoffState;
use crate::unified_exec::WriteStdinOutput;
use crate::unified_exec::WriteStdinRequest;
use crate::unified_exec::apply_unified_exec_env;
use crate::unified_exec::async_watcher::emit_exec_end_for_unified_exec;
use crate::unified_exec::async_watcher::emit_failed_exec_end_for_unified_exec;
use crate::unified_exec::async_watcher::spawn_exit_watcher;
use crate::unified_exec::async_watcher::start_streaming_output;
use crate::unified_exec::clamp_yield_time;
use crate::unified_exec::collect_output_until_deadline;
use crate::unified_exec::command_notification_filter_to_protocol;
use crate::unified_exec::command_process_id_to_prune;
use crate::unified_exec::events::emit_unified_exec_begin;
use crate::unified_exec::exec_env_policy_from_shell_policy;
use crate::unified_exec::exec_server_spawn_params;
use crate::unified_exec::generate_chunk_id;
use codex_approval_service_api::ApprovalSessionCapability;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_output_truncation::approx_token_count;
use command_service_api::CommandSessionController;
use command_service_api::CommandSessionError;
use command_service_api::CommandSessionFuture;
use command_service_api::CommandWaitOperation;
use command_service_api::ExecApprovalRequirement;
use command_service_api::ExecCapturePolicy;
use command_service_api::ExecCommandRunOutput;
use command_service_api::ExecExpiration;
use command_service_api::ExecOptions;
use command_service_api::RunningCommandSnapshot;
use command_service_api::ToolRuntimeNetworkApprovalError;
use command_service_api::ToolRuntimeNetworkApprovalHandle;
use command_service_api::ToolRuntimeNetworkApprovalTrigger;
use hooks_api::PermissionRequestDecision;
use protocol::ThreadId;
use protocol::approvals::NetworkApprovalContext;
use protocol::error::CodexErr;
use protocol::error::SandboxErr;
use protocol::network_policy::NetworkPolicyDecisionPayload;
use protocol::protocol::AskForApproval;
use protocol::protocol::ExecCommandSource;
use protocol::protocol::NetworkPolicyRuleAction;
use protocol::protocol::ReviewDecision;

const NETWORK_ACCESS_DENIED_MESSAGE: &str =
    "Network access was denied by the Codex sandbox network proxy.";
const LATE_NETWORK_DENIAL_GRACE_PERIOD: Duration = Duration::from_millis(100);

/// Test-only override for deterministic unified exec process IDs.
///
/// In production builds this value should remain at its default (`false`) and
/// must not be toggled.
static FORCE_DETERMINISTIC_PROCESS_IDS: AtomicBool = AtomicBool::new(false);

pub(super) fn set_deterministic_process_ids_for_tests(enabled: bool) {
    FORCE_DETERMINISTIC_PROCESS_IDS.store(enabled, Ordering::Relaxed);
}

pub(crate) struct CommandWaitBegin {
    pub(crate) process_id: i32,
    pub(crate) wait_timeout: Duration,
    started_at: Instant,
    state: CommandWaitBeginState,
}

enum CommandWaitBeginState {
    Completed {
        exit_code: Option<i32>,
    },
    Pending {
        process: Arc<UnifiedExecProcess>,
        notification_state: Arc<CommandNotificationState>,
        snapshot: CommandNotificationSnapshot,
    },
}

#[derive(Clone)]
pub(crate) struct UnifiedExecCommandSessionController {
    manager: Arc<UnifiedExecProcessManager>,
}

impl UnifiedExecCommandSessionController {
    pub(crate) fn new(manager: Arc<UnifiedExecProcessManager>) -> Self {
        Self { manager }
    }
}

struct UnifiedExecCommandWaitOperation {
    manager: Arc<UnifiedExecProcessManager>,
    wait: CommandWaitBegin,
}

impl CommandWaitOperation for UnifiedExecCommandWaitOperation {
    fn process_id(&self) -> i32 {
        self.wait.process_id
    }

    fn wait_timeout(&self) -> Duration {
        self.wait.wait_timeout
    }

    fn finish(
        self: Box<Self>,
    ) -> CommandSessionFuture<'static, Result<CommandWaitOutput, CommandSessionError>> {
        Box::pin(async move {
            self.manager
                .finish_command_wait(self.wait)
                .await
                .map_err(command_session_error_from_unified_exec)
        })
    }
}

impl CommandSessionController for UnifiedExecCommandSessionController {
    fn begin_command_wait<'a>(
        &'a self,
        request: CommandWaitRequest,
    ) -> CommandSessionFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>> {
        Box::pin(async move {
            let wait = self
                .manager
                .begin_command_wait(request)
                .await
                .map_err(command_session_error_from_unified_exec)?;
            Ok(Box::new(UnifiedExecCommandWaitOperation {
                manager: Arc::clone(&self.manager),
                wait,
            }) as Box<dyn CommandWaitOperation>)
        })
    }

    fn write_command_stdin<'a>(
        &'a self,
        request: WriteStdinRequest<'a>,
    ) -> CommandSessionFuture<'a, Result<WriteStdinOutput, CommandSessionError>> {
        Box::pin(async move {
            self.manager
                .write_command_stdin(request)
                .await
                .map_err(command_session_error_from_unified_exec)
        })
    }
}

fn command_session_error_from_unified_exec(err: UnifiedExecError) -> CommandSessionError {
    CommandSessionError::new(err.to_string())
}

fn deterministic_process_ids_forced_for_tests() -> bool {
    FORCE_DETERMINISTIC_PROCESS_IDS.load(Ordering::Relaxed)
}

fn should_use_deterministic_process_ids() -> bool {
    cfg!(test) || deterministic_process_ids_forced_for_tests()
}

fn exec_server_params_for_request(
    process_id: i32,
    request: &ExecRequest,
    exec_server_env_config: Option<&ExecServerEnvConfig>,
    tty: bool,
) -> codex_exec_server_protocol::ExecParams {
    exec_server_spawn_params(
        process_id,
        ExecServerSpawnRequest {
            command: request.command.clone(),
            cwd: request.cwd.to_path_buf(),
            env: request.env.clone(),
            arg0: request.arg0.clone(),
        },
        exec_server_env_config,
        tty,
    )
}

async fn unregister_network_approval_for_entry(entry: &ProcessEntry) {
    if let Some(network_approval) = entry.network_approval.as_ref()
        && let Some(registration_id) = network_approval.registration_id()
        && let Some(session) = entry.session.upgrade()
    {
        session.unregister_network_approval(&registration_id).await;
    }
}

async fn finish_network_approval(
    approval: Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>,
) -> Result<(), String> {
    let Some(approval) = approval else {
        return Ok(());
    };
    approval
        .finish()
        .await
        .map_err(network_approval_error_message_from_runtime)
}

fn network_approval_error_message_from_runtime(err: ToolRuntimeNetworkApprovalError) -> String {
    match err {
        ToolRuntimeNetworkApprovalError::Rejected(message) => message,
        ToolRuntimeNetworkApprovalError::Codex(err) => err.to_string(),
    }
}

async fn network_denial_message_for_session(
    approval: Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>,
) -> String {
    let Some(approval) = approval else {
        return NETWORK_ACCESS_DENIED_MESSAGE.to_string();
    };
    match approval.finish().await {
        Ok(()) => NETWORK_ACCESS_DENIED_MESSAGE.to_string(),
        Err(err) => network_approval_error_message_from_runtime(err),
    }
}

async fn wait_for_late_network_denial(network_cancelled: Option<CancellationToken>) -> bool {
    let Some(network_cancelled) = network_cancelled else {
        return false;
    };
    if network_cancelled.is_cancelled() {
        return true;
    }

    tokio::select! {
        _ = network_cancelled.cancelled() => true,
        _ = tokio::time::sleep(LATE_NETWORK_DENIAL_GRACE_PERIOD) => false,
    }
}

async fn finish_deferred_network_approval_after_process_exit_for_session(
    approval: Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>,
) -> Result<(), String> {
    wait_for_late_network_denial(
        approval
            .as_ref()
            .map(|approval| approval.cancellation_token()),
    )
    .await;
    finish_network_approval(approval).await
}

fn unified_exec_approval_keys(
    request: &ExecCommandRequest,
) -> Vec<thread_service_api::UnifiedExecApprovalKey> {
    vec![thread_service_api::UnifiedExecApprovalKey {
        command: request.command.clone(),
        cwd: request.cwd.clone(),
        tty: request.tty,
        sandbox_permissions: request.sandbox_permissions,
        additional_permissions: request.additional_permissions.clone(),
    }]
}

fn unified_exec_network_trigger(
    request: &ExecCommandRequest,
    context: &UnifiedExecContext,
) -> ToolRuntimeNetworkApprovalTrigger {
    ToolRuntimeNetworkApprovalTrigger {
        call_id: context.call_id.clone(),
        tool_name: "exec_command".to_string(),
        command: request.command.clone(),
        cwd: request.cwd.clone(),
        sandbox_permissions: request.sandbox_permissions,
        additional_permissions: request.additional_permissions.clone(),
        justification: request.justification.clone(),
        tty: Some(request.tty),
    }
}

fn network_approval_context_from_payload(
    payload: &NetworkPolicyDecisionPayload,
) -> Option<NetworkApprovalContext> {
    if !payload.is_ask_from_decider() {
        return None;
    }

    let protocol = payload.protocol?;
    let host = payload.host.as_deref()?.trim();
    if host.is_empty() {
        return None;
    }

    Some(NetworkApprovalContext {
        host: host.to_string(),
        protocol,
    })
}

fn sandbox_denial_reason(_output: &protocol::exec_output::ExecToolCallOutput) -> String {
    "command failed; retry without sandbox?".to_string()
}

async fn reject_unapproved_decision(
    session: &dyn ApprovalSessionCapability,
    review_id: Option<&str>,
    decision: ReviewDecision,
) -> Result<(), ToolError> {
    match decision {
        ReviewDecision::Denied | ReviewDecision::Abort => {
            let reason = if let Some(review_id) = review_id {
                let rejection = session.take_review_rejection(review_id).await;
                codex_approval_service_api::guardian_rejection_message_from_rationale(
                    rejection
                        .as_ref()
                        .map(|rejection| rejection.rationale.as_str()),
                )
            } else {
                "rejected by user".to_string()
            };
            Err(ToolError::Rejected(reason))
        }
        ReviewDecision::TimedOut => Err(ToolError::Rejected(
            codex_approval_service_api::guardian_timeout_message(),
        )),
        ReviewDecision::Approved
        | ReviewDecision::ApprovedExecpolicyAmendment { .. }
        | ReviewDecision::ApprovedForSession => Ok(()),
        ReviewDecision::NetworkPolicyAmendment {
            network_policy_amendment,
        } => match network_policy_amendment.action {
            NetworkPolicyRuleAction::Allow => Ok(()),
            NetworkPolicyRuleAction::Deny => {
                Err(ToolError::Rejected("rejected by user".to_string()))
            }
        },
    }
}

async fn request_unified_exec_approval(
    request: &ExecCommandRequest,
    context: &UnifiedExecContext,
    permission_request_run_id: &str,
    reason: Option<String>,
    network_approval_context: Option<NetworkApprovalContext>,
    use_guardian: bool,
    evaluate_permission_request_hooks: bool,
) -> Result<(), ToolError> {
    if evaluate_permission_request_hooks
        && let Some(decision) = context
            .approval_session
            .run_permission_request_hooks(
                context.turn.as_ref(),
                permission_request_run_id,
                command_service_api::PermissionRequestPayload::bash(
                    request.hook_command.clone(),
                    request.justification.clone(),
                ),
            )
            .await
    {
        match decision {
            PermissionRequestDecision::Allow => return Ok(()),
            PermissionRequestDecision::Deny { message } => {
                return Err(ToolError::Rejected(message));
            }
        }
    }

    let review_id = use_guardian.then(|| uuid::Uuid::new_v4().to_string());
    let decision = context
        .approval_session
        .request_unified_exec_approval(
            context.turn.as_ref(),
            context.call_id.clone(),
            request.command.clone(),
            request.cwd.clone(),
            reason.clone().or_else(|| request.justification.clone()),
            request.sandbox_permissions,
            request.tty,
            network_approval_context,
            request
                .exec_approval_requirement
                .proposed_execpolicy_amendment()
                .cloned(),
            request.additional_permissions.clone(),
            unified_exec_approval_keys(request),
        )
        .await;

    reject_unapproved_decision(
        context.approval_session.as_ref(),
        review_id.as_deref(),
        decision,
    )
    .await
}

fn fail_process_with_message(process: &UnifiedExecProcess, message: String) -> UnifiedExecError {
    if let Some(message) = process.failure_message() {
        process.terminate();
        return UnifiedExecError::process_failed(message);
    }

    process.fail_and_terminate(message.clone());
    UnifiedExecError::process_failed(process.failure_message().unwrap_or(message))
}

fn unified_exec_options(
    network_denial_cancellation_token: Option<CancellationToken>,
) -> ExecOptions {
    let mut expiration = ExecExpiration::DefaultTimeout;
    if let Some(cancellation) = network_denial_cancellation_token {
        expiration = expiration.with_cancellation(cancellation);
    }
    ExecOptions {
        expiration,
        capture_policy: ExecCapturePolicy::ShellTool,
    }
}

async fn spawn_unified_exec_process(
    manager: &UnifiedExecProcessManager,
    request: &ExecCommandRequest,
    base_env: std::collections::HashMap<String, String>,
    exec_server_env_config: &ExecServerEnvConfig,
    attempt: &SandboxAttempt<'_>,
    context: &UnifiedExecContext,
) -> Result<UnifiedExecProcess, ToolError> {
    let session_shell = context.turn.runtime_shell();
    let managed_network = managed_network_for_sandbox_permissions(
        request.network.as_ref(),
        request.sandbox_permissions,
    );
    let mut env = exec_env_for_sandbox_permissions(&base_env, request.sandbox_permissions);
    if let Some(network) = managed_network.as_ref() {
        network.apply_to_env(&mut env);
    }

    let command = if request.environment.is_remote() {
        request.command.clone()
    } else {
        maybe_wrap_shell_lc_with_snapshot(
            &request.command,
            &session_shell,
            &request.cwd,
            &context.turn.shell_environment_policy().r#set,
            &env,
        )
    };
    let command = disable_powershell_profile_for_elevated_windows_sandbox(
        &command,
        Some(request.shell_type),
        attempt.sandbox,
        attempt.windows_sandbox_level,
    );
    let command = if matches!(
        session_shell.shell_type,
        tool_config::ToolUserShellType::PowerShell
    ) {
        codex_shell_utils::powershell::prefix_powershell_script_with_utf8(&command)
    } else {
        command
    };
    let command = build_sandbox_command(
        &command,
        &request.cwd,
        &env,
        request.additional_permissions.clone(),
    )
    .map_err(|_| ToolError::Rejected("missing command line for PTY".to_string()))?;
    let exec_request = attempt
        .env_for(
            command,
            unified_exec_options(attempt.network_denial_cancellation_token.clone()),
            managed_network,
        )
        .map_err(|err: codex_sandboxing_api::SandboxTransformError| ToolError::Codex(err.into()))?;

    manager
        .open_session_with_exec_env(
            request.process_id,
            &exec_request,
            Some(exec_server_env_config),
            request.tty,
            Box::new(crate::unified_exec::NoopSpawnLifecycle),
            request.environment.as_ref(),
        )
        .await
        .map_err(|err| match err {
            UnifiedExecError::SandboxDenied { output, .. } => {
                ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                    output: Box::new(output),
                    network_policy_decision: None,
                }))
            }
            other => ToolError::Rejected(other.to_string()),
        })
}

#[allow(clippy::too_many_arguments)]
async fn run_unified_exec_attempt(
    manager: &UnifiedExecProcessManager,
    request: &ExecCommandRequest,
    base_env: std::collections::HashMap<String, String>,
    exec_server_env_config: &ExecServerEnvConfig,
    attempt: &SandboxAttempt<'_>,
    context: &UnifiedExecContext,
) -> (
    Result<UnifiedExecProcess, ToolError>,
    Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>,
) {
    let network_approval: Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>> = context
        .turn
        .begin_tool_network_approval(
            managed_network_for_sandbox_permissions(
                request.network.as_ref(),
                request.sandbox_permissions,
            )
            .map(|network| thread_service_api::NetworkApprovalSpec {
                network: Some(network),
                mode: thread_service_api::NetworkApprovalMode::Deferred,
                trigger: unified_exec_network_trigger(request, context),
                command: request.hook_command.clone(),
            }),
        )
        .await;

    let attempt = SandboxAttempt {
        network_denial_cancellation_token: network_approval
            .as_ref()
            .map(|approval| approval.cancellation_token()),
        ..*attempt
    };
    let run_result = spawn_unified_exec_process(
        manager,
        request,
        base_env,
        exec_server_env_config,
        &attempt,
        context,
    )
    .await;

    let Some(network_approval) = network_approval else {
        return (run_result, None);
    };

    match network_approval.mode() {
        thread_service_api::NetworkApprovalMode::Immediate => {
            let finalize = network_approval.finish().await;
            match finalize {
                Ok(()) => (run_result, None),
                Err(err) => (Err(map_runtime_tool_error(err)), None),
            }
        }
        thread_service_api::NetworkApprovalMode::Deferred => {
            if run_result.is_err() {
                match network_approval.finish().await {
                    Ok(()) => (run_result, None),
                    Err(err) => (Err(map_runtime_tool_error(err)), None),
                }
            } else {
                (run_result, Some(network_approval))
            }
        }
    }
}

fn map_runtime_tool_error(err: ToolRuntimeNetworkApprovalError) -> ToolError {
    match err {
        ToolRuntimeNetworkApprovalError::Rejected(message) => ToolError::Rejected(message),
        ToolRuntimeNetworkApprovalError::Codex(err) => ToolError::Codex(err),
    }
}

#[allow(clippy::too_many_arguments)]
async fn emit_failed_initial_exec_end_if_unstored(
    process_started_alive: bool,
    context: &UnifiedExecContext,
    request: &ExecCommandRequest,
    cwd: AbsolutePathBuf,
    transcript: Arc<tokio::sync::Mutex<HeadTailBuffer>>,
    fallback_output: String,
    message: String,
    wall_time: Duration,
) {
    if process_started_alive {
        return;
    }

    emit_failed_exec_end_for_unified_exec(
        Arc::clone(&context.session),
        Arc::clone(&context.turn),
        context.call_id.clone(),
        request.command.clone(),
        cwd,
        None,
        transcript,
        fallback_output,
        message,
        wall_time,
        request.yield_time_ms,
        command_notification_filter_to_protocol(request.notify_on),
    )
    .await;
}

fn terminate_process_on_network_denial(
    process: Arc<UnifiedExecProcess>,
    deferred: Arc<dyn ToolRuntimeNetworkApprovalHandle>,
) {
    let network_cancelled = deferred.cancellation_token();
    let process_exited = process.cancellation_token();
    tokio::spawn(async move {
        let denied = tokio::select! {
            _ = network_cancelled.cancelled() => true,
            _ = process_exited.cancelled() => {
                wait_for_late_network_denial(Some(network_cancelled.clone())).await
            }
        };
        if !denied {
            return;
        }
        let message = network_denial_message_for_session(Some(deferred)).await;
        process.fail_and_terminate(message);
    });
}

impl UnifiedExecProcessManager {
    pub(crate) async fn has_running_process_for_thread(&self, thread_id: ThreadId) -> bool {
        let store = self.process_store.lock().await;
        store.processes.values().any(|entry| {
            !entry.process.has_exited()
                && entry
                    .session
                    .upgrade()
                    .is_some_and(|session| session.conversation_id() == thread_id)
        })
    }

    pub(crate) async fn running_processes_for_thread(
        &self,
        thread_id: ThreadId,
    ) -> Vec<RunningCommandSnapshot> {
        let store = self.process_store.lock().await;
        let mut processes = store
            .processes
            .values()
            .filter(|entry| {
                !entry.process.has_exited()
                    && entry
                        .session
                        .upgrade()
                        .is_some_and(|session| session.conversation_id() == thread_id)
            })
            .map(ProcessEntry::as_running_snapshot)
            .collect::<Vec<_>>();
        processes.sort_by_key(|entry| entry.process_id);
        processes
    }

    pub(crate) async fn begin_command_wait(
        &self,
        request: CommandWaitRequest,
    ) -> Result<CommandWaitBegin, UnifiedExecError> {
        let started_at = Instant::now();
        let process_id = request.process_id;
        let pending = {
            let mut store = self.process_store.lock().await;
            let Some(entry) = store.processes.get_mut(&process_id) else {
                if let Some(entry) = store.process_ids.completed_process(process_id) {
                    return Ok(CommandWaitBegin {
                        process_id,
                        wait_timeout: Duration::ZERO,
                        started_at,
                        state: CommandWaitBeginState::Completed {
                            exit_code: entry.exit_code,
                        },
                    });
                }
                return Err(UnifiedExecError::UnknownProcessId { process_id });
            };
            entry.last_used = started_at;
            let wait_timeout = entry.command_wait_backoff.current_window();
            if entry.process.has_exited() {
                entry.command_wait_backoff.reset_after_event();
                return Ok(CommandWaitBegin {
                    process_id,
                    wait_timeout,
                    started_at,
                    state: CommandWaitBeginState::Completed {
                        exit_code: entry.process.exit_code(),
                    },
                });
            }
            (
                wait_timeout,
                Arc::clone(&entry.process),
                Arc::clone(&entry.notification_state),
            )
        };
        let (wait_timeout, process, notification_state) = pending;
        let snapshot = notification_state.snapshot().await;
        Ok(CommandWaitBegin {
            process_id,
            wait_timeout,
            started_at,
            state: CommandWaitBeginState::Pending {
                process,
                notification_state,
                snapshot,
            },
        })
    }

    #[allow(dead_code)]
    pub async fn subscribe_process_exit(&self, process_id: i32) -> Option<ProcessExitSubscription> {
        let (process, transcript) = {
            let mut store = self.process_store.lock().await;
            let entry = store.processes.get_mut(&process_id)?;
            entry.last_used = Instant::now();
            (Arc::clone(&entry.process), Arc::clone(&entry.transcript))
        };

        Some(ProcessExitSubscription {
            cancellation_token: process.cancellation_token(),
            process,
            transcript,
        })
    }

    pub(crate) async fn allocate_process_id(&self) -> i32 {
        let mut store = self.process_store.lock().await;
        store
            .process_ids
            .reserve_next(should_use_deterministic_process_ids())
    }

    pub(crate) async fn release_process_id(&self, process_id: i32) {
        let removed = {
            let mut store = self.process_store.lock().await;
            store.remove(process_id)
        };
        if let Some(entry) = removed {
            unregister_network_approval_for_entry(&entry).await;
        }
    }

    pub(crate) async fn exec_command(
        &self,
        request: ExecCommandRequest,
        context: &UnifiedExecContext,
    ) -> Result<ExecCommandRunOutput, UnifiedExecError> {
        let cwd = request.cwd.clone();
        let process = self
            .open_session_with_sandbox(&request, cwd.clone(), context)
            .await;

        let (process, mut deferred_network_approval) = match process {
            Ok((process, deferred_network_approval)) => {
                (Arc::new(process), deferred_network_approval)
            }
            Err(err) => {
                self.release_process_id(request.process_id).await;
                return Err(err);
            }
        };
        if let Some(deferred) = deferred_network_approval.as_ref() {
            terminate_process_on_network_denial(Arc::clone(&process), deferred.clone());
        }

        let transcript = Arc::new(tokio::sync::Mutex::new(HeadTailBuffer::default()));
        emit_unified_exec_begin(
            Arc::clone(&context.session),
            Arc::clone(&context.turn),
            &context.call_id,
            &request.command,
            &cwd,
            ExecCommandSource::UnifiedExecStartup,
            Some(request.process_id.to_string()),
            request.yield_time_ms,
            command_notification_filter_to_protocol(request.notify_on),
        )
        .await;

        let notification_state = Arc::new(CommandNotificationState::default());
        start_streaming_output(
            &process,
            context,
            Arc::clone(&transcript),
            request.notify_on,
            Arc::clone(&notification_state),
        );
        let start = Instant::now();
        // Persist live sessions before the initial yield wait so interrupting the
        // turn cannot drop the last Arc and terminate the background process.
        let process_started_alive = !process.has_exited() && process.exit_code().is_none();
        if process_started_alive {
            self.store_process(
                Arc::clone(&process),
                context,
                &request.command,
                cwd.clone(),
                start,
                request.process_id,
                request.tty,
                deferred_network_approval.clone(),
                Arc::clone(&transcript),
                Arc::clone(&notification_state),
                request.yield_time_ms,
                request.notify_on,
            )
            .await;
        }

        let yield_time_ms = clamp_yield_time(request.yield_time_ms);
        // For the initial exec_command call, we both stream output to events
        // (via start_streaming_output above) and collect a snapshot here for
        // the tool response body.
        let output_handles = process.output_handles();
        let deadline = start + Duration::from_millis(yield_time_ms);
        let collected = collect_output_until_deadline(
            &output_handles,
            Some(
                context
                    .session
                    .subscribe_out_of_band_elicitation_pause_state(),
            ),
            deadline,
        )
        .await;
        let wall_time = Instant::now().saturating_duration_since(start);

        let text = String::from_utf8_lossy(&collected).to_string();
        let chunk_id = generate_chunk_id();
        if deferred_network_approval
            .as_ref()
            .is_some_and(|approval| approval.cancellation_token().is_cancelled())
        {
            let message =
                network_denial_message_for_session(deferred_network_approval.take()).await;
            emit_failed_initial_exec_end_if_unstored(
                process_started_alive,
                context,
                &request,
                cwd.clone(),
                Arc::clone(&transcript),
                text.clone(),
                message.clone(),
                wall_time,
            )
            .await;
            self.release_process_id(request.process_id).await;
            return Err(fail_process_with_message(process.as_ref(), message));
        }
        if let Some(message) = process.failure_message() {
            let finish_result = finish_network_approval(deferred_network_approval.take()).await;
            emit_failed_initial_exec_end_if_unstored(
                process_started_alive,
                context,
                &request,
                cwd.clone(),
                Arc::clone(&transcript),
                text.clone(),
                message.clone(),
                wall_time,
            )
            .await;
            self.release_process_id(request.process_id).await;
            if let Err(message) = finish_result {
                return Err(fail_process_with_message(process.as_ref(), message));
            }
            return Err(UnifiedExecError::process_failed(message));
        }
        let process_id = request.process_id;
        let (response_process_id, exit_code) = if process_started_alive {
            match self.refresh_process_state(process_id).await {
                ProcessStatus::Alive {
                    exit_code,
                    process_id,
                    ..
                } => (Some(process_id), exit_code),
                ProcessStatus::Exited { exit_code, entry } => {
                    if let Err(message) =
                        finish_deferred_network_approval_after_process_exit_for_session(
                            deferred_network_approval.take(),
                        )
                        .await
                    {
                        return Err(fail_process_with_message(entry.process.as_ref(), message));
                    }
                    process.check_for_sandbox_denial_with_text(&text).await?;
                    (None, exit_code)
                }
                ProcessStatus::Unknown => {
                    return Err(UnifiedExecError::UnknownProcessId { process_id });
                }
            }
        } else {
            // Short‑lived command: emit ExecCommandEnd immediately using the
            // same helper as the background watcher, so all end events share
            // one implementation.
            let finish_result = finish_deferred_network_approval_after_process_exit_for_session(
                deferred_network_approval.take(),
            )
            .await;
            if let Err(message) = finish_result {
                emit_failed_initial_exec_end_if_unstored(
                    process_started_alive,
                    context,
                    &request,
                    cwd.clone(),
                    Arc::clone(&transcript),
                    text.clone(),
                    message.clone(),
                    wall_time,
                )
                .await;
                self.release_process_id(request.process_id).await;
                return Err(fail_process_with_message(process.as_ref(), message));
            }
            let exit_code = process.exit_code();
            let exit = exit_code.unwrap_or(-1);
            emit_exec_end_for_unified_exec(
                Arc::clone(&context.session),
                Arc::clone(&context.turn),
                context.call_id.clone(),
                request.command.clone(),
                cwd.clone(),
                None,
                Arc::clone(&transcript),
                text.clone(),
                exit,
                wall_time,
                request.yield_time_ms,
                command_notification_filter_to_protocol(request.notify_on),
            )
            .await;

            self.release_process_id(request.process_id).await;
            process.check_for_sandbox_denial_with_text(&text).await?;
            (None, exit_code)
        };

        let original_token_count = approx_token_count(&text);
        let response = ExecCommandRunOutput {
            event_call_id: context.call_id.clone(),
            chunk_id,
            wall_time,
            raw_output: collected,
            max_output_tokens: request.max_output_tokens,
            process_id: response_process_id,
            exit_code,
            original_token_count: Some(original_token_count),
            hook_command: Some(request.hook_command.clone()),
        };

        Ok(response)
    }

    pub(crate) async fn write_command_stdin(
        &self,
        request: WriteStdinRequest<'_>,
    ) -> Result<WriteStdinOutput, UnifiedExecError> {
        if request.input.is_empty() {
            return Err(UnifiedExecError::EmptyStdin);
        }

        let process_id = request.process_id;
        let (process, network_approval, call_id, tty) = {
            let mut store = self.process_store.lock().await;
            let entry = store
                .processes
                .get_mut(&process_id)
                .ok_or(UnifiedExecError::UnknownProcessId { process_id })?;
            entry.last_used = Instant::now();
            (
                Arc::clone(&entry.process),
                entry.network_approval.clone(),
                entry.call_id.clone(),
                entry.tty,
            )
        };

        if !tty {
            return Err(UnifiedExecError::StdinClosed);
        }
        match process.write(request.input.as_bytes()).await {
            Ok(()) => {}
            Err(err) => {
                if matches!(err, UnifiedExecError::ProcessFailed { .. }) {
                    process.terminate();
                    self.release_process_id(process_id).await;
                    return Err(err);
                }
                return Err(err);
            }
        }
        if network_approval
            .as_ref()
            .is_some_and(|approval| approval.cancellation_token().is_cancelled())
        {
            let message = network_denial_message_for_session(network_approval.clone()).await;
            self.release_process_id(process_id).await;
            return Err(fail_process_with_message(process.as_ref(), message));
        }
        if let Some(message) = process.failure_message() {
            let finish_result = finish_network_approval(network_approval.clone()).await;
            self.release_process_id(process_id).await;
            if let Err(message) = finish_result {
                return Err(fail_process_with_message(process.as_ref(), message));
            }
            return Err(UnifiedExecError::process_failed(message));
        }

        Ok(WriteStdinOutput {
            process_id,
            call_id,
            bytes_written: request.input.len(),
        })
    }

    async fn refresh_process_state(&self, process_id: i32) -> ProcessStatus {
        {
            let mut store = self.process_store.lock().await;
            let Some(entry) = store.processes.get(&process_id) else {
                return ProcessStatus::Unknown;
            };

            let exit_code = entry.process.exit_code();
            let process_id = entry.process_id;

            if entry.process.has_exited() {
                let Some(entry) = store.remove(process_id) else {
                    return ProcessStatus::Unknown;
                };
                ProcessStatus::Exited {
                    exit_code,
                    entry: Box::new(entry),
                }
            } else {
                entry.notification_state.activate_background_session();
                ProcessStatus::Alive {
                    exit_code,
                    process_id,
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn store_process(
        &self,
        process: Arc<UnifiedExecProcess>,
        context: &UnifiedExecContext,
        command: &[String],
        cwd: AbsolutePathBuf,
        started_at: Instant,
        process_id: i32,
        tty: bool,
        network_approval: Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>,
        transcript: Arc<tokio::sync::Mutex<HeadTailBuffer>>,
        notification_state: Arc<CommandNotificationState>,
        initial_wait_ms: u64,
        notify_on: CommandNotificationFilter,
    ) {
        let entry = ProcessEntry {
            process: Arc::clone(&process),
            call_id: context.call_id.clone(),
            process_id,
            command: command.join(" "),
            cwd: cwd.clone(),
            tty,
            notify_on,
            network_approval,
            session: Arc::downgrade(&context.session),
            last_used: started_at,
            transcript: Arc::clone(&transcript),
            notification_state: Arc::clone(&notification_state),
            command_wait_backoff: WaitBackoffState::new(
                Duration::from_millis(initial_wait_ms),
                self.command_wait_hard_cap,
            ),
        };
        let pruned_entry = {
            let mut store = self.process_store.lock().await;
            let pruned_entry = Self::prune_processes_if_needed(&mut store);
            store.processes.insert(process_id, entry);
            pruned_entry
        };
        // prune_processes_if_needed runs while holding process_store; do async
        // network-approval cleanup only after dropping that lock.
        if let Some(pruned_entry) = pruned_entry {
            unregister_network_approval_for_entry(&pruned_entry).await;
            pruned_entry.process.terminate();
        }

        spawn_exit_watcher(
            Arc::clone(&process),
            Arc::clone(&context.session),
            Arc::clone(&context.turn),
            context.call_id.clone(),
            command.to_vec(),
            cwd.clone(),
            process_id,
            transcript,
            started_at,
            notification_state,
            initial_wait_ms,
            notify_on,
        );
    }

    #[allow(dead_code)]
    pub(crate) async fn wait_for_command_notification(
        &self,
        request: CommandWaitRequest,
    ) -> Result<CommandWaitOutput, UnifiedExecError> {
        let wait = self.begin_command_wait(request).await?;
        self.finish_command_wait(wait).await
    }

    pub(crate) async fn finish_command_wait(
        &self,
        wait: CommandWaitBegin,
    ) -> Result<CommandWaitOutput, UnifiedExecError> {
        let process_id = wait.process_id;
        let wait_window = wait.wait_timeout;
        let started_at = wait.started_at;
        let (process, notification_state, snapshot) = match wait.state {
            CommandWaitBeginState::Completed { exit_code } => {
                return Ok(CommandWaitOutput {
                    process_id,
                    status: CommandWaitStatus::Completed,
                    notification: Some(CommandNotificationKind::Exit),
                    exit_code,
                    wall_time: std::time::Duration::ZERO,
                    wait_timeout: wait_window,
                });
            }
            CommandWaitBeginState::Pending {
                process,
                notification_state,
                snapshot,
            } => (process, notification_state, snapshot),
        };
        let cancellation_token = process.cancellation_token();

        let notification = tokio::select! {
            _ = cancellation_token.cancelled() => CommandNotificationKind::Exit,
            kind = notification_state.wait_after(snapshot) => kind,
            _ = tokio::time::sleep(wait_window) => {
                self.advance_command_wait_backoff_for_process(process_id, &process)
                    .await;
                return Ok(CommandWaitOutput {
                    process_id,
                    status: CommandWaitStatus::Running,
                    notification: None,
                    exit_code: process.exit_code(),
                    wall_time: Instant::now().saturating_duration_since(started_at),
                    wait_timeout: wait_window,
                });
            }
        };
        let status = if process.has_exited() {
            CommandWaitStatus::Completed
        } else {
            CommandWaitStatus::Running
        };
        self.reset_command_wait_backoff_for_process(process_id, &process)
            .await;
        Ok(CommandWaitOutput {
            process_id,
            status,
            notification: Some(notification),
            exit_code: process.exit_code(),
            wall_time: Instant::now().saturating_duration_since(started_at),
            wait_timeout: wait_window,
        })
    }

    async fn advance_command_wait_backoff_for_process(
        &self,
        process_id: i32,
        process: &Arc<UnifiedExecProcess>,
    ) {
        let mut store = self.process_store.lock().await;
        if let Some(entry) = store.processes.get_mut(&process_id)
            && Arc::ptr_eq(&entry.process, process)
        {
            entry.command_wait_backoff.advance_after_timeout();
        }
    }

    async fn reset_command_wait_backoff_for_process(
        &self,
        process_id: i32,
        process: &Arc<UnifiedExecProcess>,
    ) {
        let mut store = self.process_store.lock().await;
        if let Some(entry) = store.processes.get_mut(&process_id)
            && Arc::ptr_eq(&entry.process, process)
        {
            entry.command_wait_backoff.reset_after_event();
        }
    }

    pub(crate) async fn open_session_with_exec_env(
        &self,
        process_id: i32,
        request: &ExecRequest,
        exec_server_env_config: Option<&ExecServerEnvConfig>,
        tty: bool,
        mut spawn_lifecycle: SpawnLifecycleHandle,
        environment: &dyn exec_server_api::ExecEnvironment,
    ) -> Result<UnifiedExecProcess, UnifiedExecError> {
        let inherited_fds = spawn_lifecycle.inherited_fds();

        #[cfg(target_os = "windows")]
        if request.sandbox == codex_sandboxing_api::SandboxType::WindowsRestrictedToken {
            let sandbox_policy = request.compatibility_sandbox_policy();
            let policy_json = serde_json::to_string(&sandbox_policy).map_err(|err| {
                UnifiedExecError::create_process(format!(
                    "failed to serialize Windows sandbox policy: {err}"
                ))
            })?;
            let codex_home = crate::config::find_codex_home().map_err(|err| {
                UnifiedExecError::create_process(format!(
                    "windows sandbox: failed to resolve codex_home: {err}"
                ))
            })?;
            let additional_deny_write_paths = request
                .windows_sandbox_filesystem_overrides
                .as_ref()
                .map(|overrides| overrides.additional_deny_write_paths.clone())
                .unwrap_or_default();
            let additional_deny_read_paths = request
                .windows_sandbox_filesystem_overrides
                .as_ref()
                .map(|overrides| overrides.additional_deny_read_paths.clone())
                .unwrap_or_default();
            let elevated_read_roots_override = request
                .windows_sandbox_filesystem_overrides
                .as_ref()
                .and_then(|overrides| overrides.read_roots_override.clone());
            let elevated_read_roots_include_platform_defaults = request
                .windows_sandbox_filesystem_overrides
                .as_ref()
                .is_some_and(|overrides| overrides.read_roots_include_platform_defaults);
            let elevated_write_roots_override = request
                .windows_sandbox_filesystem_overrides
                .as_ref()
                .and_then(|overrides| overrides.write_roots_override.clone());
            let spawned = match request.windows_sandbox_level {
                protocol::config_types::WindowsSandboxLevel::Elevated => {
                    codex_windows_sandbox::spawn_windows_sandbox_session_elevated(
                        policy_json.as_str(),
                        request.windows_sandbox_policy_cwd.as_path(),
                        codex_home.as_ref(),
                        request.command.clone(),
                        request.cwd.as_path(),
                        request.env.clone(),
                        None,
                        elevated_read_roots_override.as_deref(),
                        elevated_read_roots_include_platform_defaults,
                        elevated_write_roots_override.as_deref(),
                        &additional_deny_read_paths,
                        &additional_deny_write_paths,
                        tty,
                        tty,
                        request.windows_sandbox_private_desktop,
                    )
                    .await
                }
                protocol::config_types::WindowsSandboxLevel::RestrictedToken
                | protocol::config_types::WindowsSandboxLevel::Disabled => {
                    codex_windows_sandbox::spawn_windows_sandbox_session_legacy(
                        policy_json.as_str(),
                        request.windows_sandbox_policy_cwd.as_path(),
                        codex_home.as_ref(),
                        request.command.clone(),
                        request.cwd.as_path(),
                        request.env.clone(),
                        None,
                        &additional_deny_read_paths,
                        &additional_deny_write_paths,
                        tty,
                        tty,
                        request.windows_sandbox_private_desktop,
                    )
                    .await
                }
            };
            spawn_lifecycle.after_spawn();
            return UnifiedExecProcess::from_spawned(
                spawned.map_err(|err| UnifiedExecError::create_process(err.to_string()))?,
                request.sandbox,
                spawn_lifecycle,
            )
            .await;
        }
        if environment.is_remote() {
            if !inherited_fds.is_empty() {
                return Err(UnifiedExecError::create_process(
                    "remote exec-server does not support inherited file descriptors".to_string(),
                ));
            }

            let started = environment
                .get_exec_backend()
                .start(exec_server_params_for_request(
                    process_id,
                    request,
                    exec_server_env_config,
                    tty,
                ))
                .await
                .map_err(|err| UnifiedExecError::create_process(err.to_string()))?;
            spawn_lifecycle.after_spawn();
            return UnifiedExecProcess::from_exec_server_started(started, request.sandbox).await;
        }

        let (program, args) = request
            .command
            .split_first()
            .ok_or(UnifiedExecError::MissingCommandLine)?;
        let spawn_result = if tty {
            codex_utils_pty::pty::spawn_process_with_inherited_fds(
                program,
                args,
                request.cwd.as_path(),
                &request.env,
                &request.arg0,
                codex_utils_pty::TerminalSize::default(),
                &inherited_fds,
            )
            .await
        } else {
            codex_utils_pty::pipe::spawn_process_no_stdin_with_inherited_fds(
                program,
                args,
                request.cwd.as_path(),
                &request.env,
                &request.arg0,
                &inherited_fds,
            )
            .await
        };
        let spawned =
            spawn_result.map_err(|err| UnifiedExecError::create_process(err.to_string()))?;
        spawn_lifecycle.after_spawn();
        UnifiedExecProcess::from_spawned(spawned, request.sandbox, spawn_lifecycle).await
    }

    pub(super) async fn open_session_with_sandbox(
        &self,
        request: &ExecCommandRequest,
        _cwd: AbsolutePathBuf,
        context: &UnifiedExecContext,
    ) -> Result<
        (
            UnifiedExecProcess,
            Option<Arc<dyn ToolRuntimeNetworkApprovalHandle>>,
        ),
        UnifiedExecError,
    > {
        let local_policy_env = create_env(
            &context.turn.shell_environment_policy(),
            /*thread_id*/ None,
        );
        let mut env = local_policy_env.clone();
        env.insert(
            CODEX_THREAD_ID_ENV_VAR.to_string(),
            context.session.conversation_id().to_string(),
        );
        let env = apply_unified_exec_env(env);
        let exec_server_env_config = ExecServerEnvConfig {
            policy: exec_env_policy_from_shell_policy(&context.turn.shell_environment_policy()),
            local_policy_env,
        };
        let sandbox_context = context.turn.tool_sandbox_context();
        let sandbox_runtime = context.session.sandbox_runtime();
        let strict_auto_review = context
            .approval_session
            .strict_auto_review_enabled_for_turn()
            .await;
        let use_guardian = strict_auto_review || context.turn.routes_approval_to_guardian();
        let approval_policy = context.turn.approval_policy();
        let requirement = request.exec_approval_requirement.clone();
        let already_approved = match &requirement {
            ExecApprovalRequirement::Skip { .. } => {
                if strict_auto_review {
                    request_unified_exec_approval(
                        request,
                        context,
                        &context.call_id,
                        None,
                        None,
                        use_guardian,
                        /*evaluate_permission_request_hooks*/ false,
                    )
                    .await
                    .map_err(|err| UnifiedExecError::create_process(format!("{err:?}")))?;
                    true
                } else {
                    matches!(
                        request.approval_mode,
                        crate::unified_exec::ExecCommandApprovalMode::AlreadyApproved
                    )
                }
            }
            ExecApprovalRequirement::Forbidden { reason } => {
                return Err(UnifiedExecError::create_process(reason.clone()));
            }
            ExecApprovalRequirement::NeedsApproval { reason, .. } => {
                request_unified_exec_approval(
                    request,
                    context,
                    &context.call_id,
                    reason.clone(),
                    None,
                    use_guardian,
                    /*evaluate_permission_request_hooks*/ !strict_auto_review,
                )
                .await
                .map_err(|err| UnifiedExecError::create_process(format!("{err:?}")))?;
                true
            }
        };

        let sandbox_override = sandbox_override_for_first_attempt(
            request.sandbox_permissions,
            &requirement,
            &sandbox_context.file_system_sandbox_policy,
        );
        let managed_network_active = sandbox_context.managed_network_active;
        let initial_sandbox = match sandbox_override {
            SandboxOverride::BypassSandboxFirstAttempt => codex_sandboxing_api::SandboxType::None,
            SandboxOverride::NoOverride => sandbox_runtime.select_initial(
                &sandbox_context.file_system_sandbox_policy,
                sandbox_context.network_sandbox_policy,
                codex_sandboxing_api::SandboxablePreference::Auto,
                sandbox_context.windows_sandbox_level,
                managed_network_active,
            ),
        };
        let initial_attempt = SandboxAttempt {
            sandbox: initial_sandbox,
            permissions: &sandbox_context.permission_profile,
            enforce_managed_network: managed_network_active,
            sandbox_runtime: sandbox_runtime.as_ref(),
            sandbox_cwd: &sandbox_context.cwd,
            codex_linux_sandbox_exe: sandbox_context.codex_linux_sandbox_exe.as_ref(),
            use_legacy_landlock: sandbox_context.use_legacy_landlock,
            windows_sandbox_level: sandbox_context.windows_sandbox_level,
            windows_sandbox_private_desktop: sandbox_context.windows_sandbox_private_desktop,
            network_denial_cancellation_token: None,
        };
        let (first_result, first_deferred_network_approval) = run_unified_exec_attempt(
            self,
            request,
            env.clone(),
            &exec_server_env_config,
            &initial_attempt,
            context,
        )
        .await;

        match first_result {
            Ok(output) => Ok((output, first_deferred_network_approval)),
            Err(ToolError::Codex(CodexErr::Sandbox(SandboxErr::Denied {
                output,
                network_policy_decision,
            }))) => {
                let network_approval_context = if managed_network_active {
                    network_policy_decision
                        .as_ref()
                        .and_then(network_approval_context_from_payload)
                } else {
                    None
                };
                if network_policy_decision.is_some() && network_approval_context.is_none() {
                    return Err(UnifiedExecError::sandbox_denied(
                        output.aggregated_output.text.clone(),
                        *output,
                    ));
                }
                if !wants_no_sandbox_approval(approval_policy) {
                    let allow_on_request_network_prompt =
                        matches!(approval_policy, AskForApproval::OnRequest)
                            && network_approval_context.is_some()
                            && matches!(
                                permissions_service::default_exec_approval_requirement(
                                    approval_policy,
                                    &sandbox_context.file_system_sandbox_policy,
                                ),
                                ExecApprovalRequirement::NeedsApproval { .. }
                            );
                    if !allow_on_request_network_prompt {
                        return Err(UnifiedExecError::sandbox_denied(
                            output.aggregated_output.text.clone(),
                            *output,
                        ));
                    }
                }

                let retry_reason =
                    if let Some(network_approval_context) = network_approval_context.as_ref() {
                        format!(
                            "Network access to \"{}\" is blocked by policy.",
                            network_approval_context.host
                        )
                    } else {
                        sandbox_denial_reason(output.as_ref())
                    };
                let bypass_retry_approval = !strict_auto_review
                    && (already_approved || matches!(approval_policy, AskForApproval::Never))
                    && network_approval_context.is_none();
                if !bypass_retry_approval {
                    request_unified_exec_approval(
                        request,
                        context,
                        &format!("{}:retry", context.call_id),
                        Some(retry_reason),
                        network_approval_context,
                        use_guardian,
                        /*evaluate_permission_request_hooks*/ !strict_auto_review,
                    )
                    .await
                    .map_err(|err| UnifiedExecError::create_process(format!("{err:?}")))?;
                }

                let retry_attempt = SandboxAttempt {
                    sandbox: codex_sandboxing_api::SandboxType::None,
                    permissions: &sandbox_context.permission_profile,
                    enforce_managed_network: managed_network_active,
                    sandbox_runtime: sandbox_runtime.as_ref(),
                    sandbox_cwd: &sandbox_context.cwd,
                    codex_linux_sandbox_exe: None,
                    use_legacy_landlock: sandbox_context.use_legacy_landlock,
                    windows_sandbox_level: sandbox_context.windows_sandbox_level,
                    windows_sandbox_private_desktop: sandbox_context
                        .windows_sandbox_private_desktop,
                    network_denial_cancellation_token: None,
                };
                run_unified_exec_attempt(
                    self,
                    request,
                    env,
                    &exec_server_env_config,
                    &retry_attempt,
                    context,
                )
                .await
                .0
                .map(|output| (output, None))
                .map_err(|err| UnifiedExecError::create_process(format!("{err:?}")))
            }
            Err(err) => Err(UnifiedExecError::create_process(format!("{err:?}"))),
        }
    }

    fn prune_processes_if_needed(store: &mut ProcessStore) -> Option<ProcessEntry> {
        if store.processes.len() < MAX_UNIFIED_EXEC_PROCESSES {
            return None;
        }

        let meta: Vec<CommandProcessPruneMeta> = store
            .processes
            .iter()
            .map(|(id, entry)| CommandProcessPruneMeta {
                process_id: *id,
                last_used: entry.last_used,
                has_exited: entry.process.has_exited(),
            })
            .collect();

        if let Some(process_id) = command_process_id_to_prune(&meta) {
            return store.remove(process_id);
        }

        None
    }

    pub(crate) async fn terminate_all_processes(&self) {
        let entries: Vec<ProcessEntry> = {
            let mut processes = self.process_store.lock().await;
            let entries: Vec<ProcessEntry> = processes
                .processes
                .drain()
                .map(|(_, entry)| entry)
                .collect();
            processes.process_ids.clear_reservations();
            entries
        };

        for entry in entries {
            unregister_network_approval_for_entry(&entry).await;
            entry.process.terminate();
        }
    }
}

enum ProcessStatus {
    Alive {
        exit_code: Option<i32>,
        process_id: i32,
    },
    Exited {
        exit_code: Option<i32>,
        entry: Box<ProcessEntry>,
    },
    Unknown,
}

#[cfg(test)]
#[path = "process_manager_tests.rs"]
mod tests;
