use std::path::Path;
use std::sync::Arc;

use codex_approval_service_api::ApprovalServiceApi;
use codex_approval_service_api::ExecCommandApprovalDispatch;
use codex_approval_service_api::ExecCommandApprovalOutcome;
use codex_command_service_api::CommandServiceApi;
use codex_command_service_api::CommandServiceSessionCapability;
use codex_command_service_api::CommandServiceTurnCapability;
use codex_command_service_api::ExecCommandApprovalMode;
use codex_command_service_api::ExecCommandRunOutput;
use codex_command_service_api::ExecCommandRunRequest;
use codex_command_service_api::UnifiedExecApprovalKey;
use codex_command_runtime::UnifiedExecError;
use codex_command_runtime::generate_chunk_id;
use codex_command_runtime::resolve_max_tokens;
use codex_permissions_runtime::ExecPolicyApprovalRequest;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::protocol::AskForApproval;
use codex_thread_api::SharedToolTurnDiffTracker;
use codex_thread_runtime::ThreadRuntimeSession;
use codex_thread_runtime::ThreadTurnContext;
use codex_tool_planning::CommandToolOptions;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_exec_command_tool_with_environment_id;
use codex_tool_runtime::ExecCommandToolOutput;
use codex_tool_runtime_api::ExecCommandArgs;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_service_api::AnyToolResult;
use codex_tool_service_api::HookToolName;
use codex_tool_service_api::PostToolUsePayload;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolOutput;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::context::TypedToolSpecRequest;
use crate::domains::apply_patch::apply_granted_permissions_from_grants;
use crate::domains::apply_patch::implicit_granted_permissions;
use crate::domains::apply_patch::intercept_apply_patch;
use crate::domains::apply_patch::normalize_and_validate_additional_permissions;

const EXEC_COMMAND_TOOL_NAME: &str = "exec_command";

// This domain owns the `exec_command` tool. The underlying config enum still
// uses historical shell-oriented names, but there is no separate legacy shell
// tool anymore.
pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    if !request.config.environment_mode.has_environment() {
        return Vec::new();
    }

    match request.config.shell_type {
        ConfigShellToolType::Disabled => Vec::new(),
        ConfigShellToolType::UnifiedExec
        | ConfigShellToolType::Default
        | ConfigShellToolType::Local
        | ConfigShellToolType::ShellCommand => {
            let options = CommandToolOptions {
                allow_login_shell: request.config.allow_login_shell,
                exec_permission_approvals_enabled: request.config.exec_permission_approvals_enabled,
            };
            vec![create_exec_command_tool_with_environment_id(
                options,
                matches!(
                    request.config.environment_mode,
                    codex_tool_planning::ToolEnvironmentMode::Multiple
                ),
            )]
        }
    }
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none() && tool_name.name.as_str() == EXEC_COMMAND_TOOL_NAME
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    false
}

pub(crate) async fn dispatch(
    approval_api: Arc<dyn ApprovalServiceApi>,
    command_service_api: Arc<dyn CommandServiceApi>,
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    _tracker: SharedToolTurnDiffTracker,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    if call.tool_name.name.as_str() != EXEC_COMMAND_TOOL_NAME {
        return Err(FunctionCallError::Fatal(format!(
            "unsupported exec_command tool {}",
            call.tool_name
        )));
    }

    let output =
        dispatch_exec_command(approval_api, command_service_api, session, turn, &call).await?;
    let post_tool_use_payload =
        post_unified_exec_tool_use_payload(&call.call_id, &call.payload, &output);
    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(output),
        post_tool_use_payload,
    })
}

async fn dispatch_exec_command(
    approval_api: Arc<dyn ApprovalServiceApi>,
    command_service_api: Arc<dyn CommandServiceApi>,
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    call: &ToolCall,
) -> Result<ExecCommandToolOutput, FunctionCallError> {
    let arguments = call.function_arguments()?;
    let environment_args: ExecCommandEnvironmentArgs = parse_arguments(arguments)?;
    let turn_capability = turn.as_ref() as &dyn CommandServiceTurnCapability;
    let session_capability = session.as_ref() as &dyn CommandServiceSessionCapability;
    let Some(turn_environment) = turn_capability.resolve_exec_command_environment(
        environment_args.environment_id.as_deref(),
        environment_args.workdir.as_deref(),
    )? else {
        return Err(FunctionCallError::RespondToModel(
            "unified exec is unavailable in this session".to_string(),
        ));
    };
    let cwd = turn_environment.cwd.clone();
    let args: ExecCommandArgs = parse_arguments_with_base_path(arguments, &cwd)?;
    let hook_command = args.cmd.clone();
    session_capability
        .maybe_emit_implicit_skill_invocation(turn_capability, &hook_command, &cwd)
        .await;

    let model_shell = args
        .shell
        .as_deref()
        .map(|shell| session_capability.resolve_model_shell(Path::new(shell)));
    let resolved_command = session_capability
        .resolve_exec_command(turn_capability, &args.cmd, args.login, model_shell.as_ref())
        .map_err(FunctionCallError::RespondToModel)?;
    let command = resolved_command.command;
    let command_for_display = codex_shell_command::parse_command::shlex_join(&command);

    let ExecCommandArgs {
        tty,
        yield_time_ms,
        initial_wait_ms,
        notify_on,
        max_output_tokens,
        sandbox_permissions,
        additional_permissions,
        justification,
        prefix_rule,
        ..
    } = args;
    let max_output_tokens = effective_max_output_tokens(
        max_output_tokens,
        turn_capability.truncation_policy(),
    );

    let exec_permission_approvals_enabled = session_capability.exec_permission_approvals_enabled();
    let requested_additional_permissions = additional_permissions.clone();
    let grants = session_capability.tool_permission_grants().await;
    let effective_additional_permissions = apply_granted_permissions_from_grants(
        grants,
        cwd.as_path(),
        sandbox_permissions,
        additional_permissions,
    );
    let additional_permissions_allowed = exec_permission_approvals_enabled
        || (session_capability.request_permissions_tool_enabled()
            && effective_additional_permissions.permissions_preapproved);

    if effective_additional_permissions
        .sandbox_permissions
        .requests_sandbox_override()
        && !effective_additional_permissions.permissions_preapproved
        && !matches!(turn_capability.approval_policy(), AskForApproval::OnRequest)
    {
        let approval_policy = turn_capability.approval_policy();
        return Err(FunctionCallError::RespondToModel(format!(
            "approval policy is {approval_policy:?}; reject command — you cannot ask for escalated permissions if the approval policy is {approval_policy:?}"
        )));
    }

    let normalized_additional_permissions = implicit_granted_permissions(
        sandbox_permissions,
        requested_additional_permissions.as_ref(),
        &effective_additional_permissions,
    )
    .map_or_else(
        || {
            normalize_and_validate_additional_permissions(
                additional_permissions_allowed,
                turn_capability.approval_policy(),
                effective_additional_permissions.sandbox_permissions,
                effective_additional_permissions.additional_permissions.clone(),
                effective_additional_permissions.permissions_preapproved,
                &cwd,
            )
        },
        |permissions| Ok(Some(permissions)),
    )
    .map_err(FunctionCallError::RespondToModel)?;

    let exec_approval_requirement = session_capability
        .create_exec_approval_requirement(ExecPolicyApprovalRequest {
            command: &command,
            approval_policy: turn_capability.approval_policy(),
            permission_profile: turn_capability.permission_profile(),
            file_system_sandbox_policy: &turn_capability.file_system_sandbox_policy(),
            sandbox_cwd: turn_environment.sandbox_cwd.as_path(),
            sandbox_permissions: if effective_additional_permissions.permissions_preapproved {
                codex_protocol::models::SandboxPermissions::UseDefault
            } else {
                effective_additional_permissions.sandbox_permissions
            },
            prefix_rule: prefix_rule.clone(),
        })
        .await;

    if let Some(output) = intercept_apply_patch(
        approval_api.clone(),
        session.clone(),
        turn.clone(),
        None,
        &command,
        &cwd,
        turn_environment.apply_patch_environment.clone(),
        &call.call_id,
        EXEC_COMMAND_TOOL_NAME,
    )
    .await?
    {
        return Ok(ExecCommandToolOutput {
            event_call_id: String::new(),
            chunk_id: String::new(),
            wall_time: std::time::Duration::ZERO,
            raw_output: output.into_text().into_bytes(),
            max_output_tokens: Some(max_output_tokens),
            process_id: None,
            exit_code: None,
            original_token_count: None,
            hook_command: None,
        });
    }

    let approval_outcome = approval_api
        .request_exec_command_approval(ExecCommandApprovalDispatch {
            session: session.clone(),
            turn: turn.clone(),
            call_id: call.call_id.clone(),
            command: command.clone(),
            hook_command: hook_command.clone(),
            cwd: cwd.clone().into_path_buf(),
            reason: None,
            justification: justification.clone(),
            sandbox_permissions: effective_additional_permissions.sandbox_permissions,
            additional_permissions: normalized_additional_permissions.clone(),
            tty,
            exec_approval_requirement: exec_approval_requirement.clone(),
            approval_keys: vec![UnifiedExecApprovalKey {
                command: codex_shell_command::canonicalize_command_for_approval(&command),
                cwd: cwd.clone(),
                tty,
                sandbox_permissions: effective_additional_permissions.sandbox_permissions,
                additional_permissions: normalized_additional_permissions.clone(),
            }],
            network_approval_context: None,
        })
        .await
        .map_err(FunctionCallError::RespondToModel)?;

    turn_capability.emit_unified_exec_tty_metric(tty);
    let process_id = session_capability
        .command_service_state()
        .allocate_process_id()
        .await;
    let run_request = ExecCommandRunRequest {
        command,
        shell_type: resolved_command.shell_type,
        hook_command: hook_command.clone(),
        process_id,
        yield_time_ms: initial_wait_ms.unwrap_or(yield_time_ms),
        max_output_tokens: Some(max_output_tokens),
        cwd,
        sandbox_cwd: turn_environment.sandbox_cwd,
        environment: turn_environment.environment,
        tty,
        sandbox_permissions: effective_additional_permissions.sandbox_permissions,
        additional_permissions: normalized_additional_permissions,
        additional_permissions_preapproved: effective_additional_permissions.permissions_preapproved,
        justification,
        prefix_rule,
        notify_on: notify_on.into(),
        approval_mode: match approval_outcome {
            ExecCommandApprovalOutcome::ContinueInRuntime => {
                ExecCommandApprovalMode::ContinueInRuntime
            }
            ExecCommandApprovalOutcome::Preapproved => ExecCommandApprovalMode::AlreadyApproved,
        },
        exec_approval_requirement,
    };
    match command_service_api
        .run_exec_command(
            Arc::clone(&session) as Arc<dyn CommandServiceSessionCapability>,
            Arc::clone(&turn) as Arc<dyn CommandServiceTurnCapability>,
            call.call_id.clone(),
            run_request,
        )
        .await
    {
        Ok(response) => Ok(exec_command_tool_output_from_run_output(response)),
        Err(UnifiedExecError::SandboxDenied { output, .. }) => {
            let output_text = output.aggregated_output.text;
            let original_token_count =
                codex_utils_output_truncation::approx_token_count(&output_text);
            Ok(ExecCommandToolOutput {
                event_call_id: call.call_id.clone(),
                chunk_id: generate_chunk_id(),
                wall_time: output.duration,
                raw_output: output_text.into_bytes(),
                max_output_tokens: Some(max_output_tokens),
                process_id: None,
                exit_code: Some(output.exit_code),
                original_token_count: Some(original_token_count),
                hook_command: Some(hook_command),
            })
        }
        Err(err) => Err(FunctionCallError::RespondToModel(format!(
            "exec_command failed for `{command_for_display}`: {err:?}"
        ))),
    }
}

#[derive(Debug, Deserialize)]
struct ExecCommandEnvironmentArgs {
    #[serde(default)]
    environment_id: Option<String>,
    #[serde(default)]
    workdir: Option<String>,
}

fn effective_max_output_tokens(
    max_output_tokens: Option<usize>,
    truncation_policy: codex_utils_output_truncation::TruncationPolicy,
) -> usize {
    resolve_max_tokens(max_output_tokens).min(truncation_policy.token_budget())
}

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn parse_arguments_with_base_path<T>(
    arguments: &str,
    base_path: &codex_utils_absolute_path::AbsolutePathBuf,
) -> Result<T, FunctionCallError>
where
    T: DeserializeOwned,
{
    let _guard = codex_utils_absolute_path::AbsolutePathBufGuard::new(base_path);
    parse_arguments(arguments)
}

fn post_unified_exec_tool_use_payload(
    call_id: &str,
    payload: &codex_tool_types::ToolPayload,
    result: &ExecCommandToolOutput,
) -> Option<PostToolUsePayload> {
    let command = result.hook_command.clone()?;
    let tool_use_id = if result.event_call_id.is_empty() {
        call_id.to_string()
    } else {
        result.event_call_id.clone()
    };
    let tool_response = result.post_tool_use_response(&tool_use_id, payload)?;
    Some(PostToolUsePayload {
        tool_name: HookToolName::bash(),
        tool_use_id,
        tool_input: serde_json::json!({ "command": command }),
        tool_response,
    })
}

fn exec_command_tool_output_from_run_output(output: ExecCommandRunOutput) -> ExecCommandToolOutput {
    ExecCommandToolOutput {
        event_call_id: output.event_call_id,
        chunk_id: output.chunk_id,
        wall_time: output.wall_time,
        raw_output: output.raw_output,
        max_output_tokens: output.max_output_tokens,
        process_id: output.process_id,
        exit_code: output.exit_code,
        original_token_count: output.original_token_count,
        hook_command: output.hook_command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use codex_approval_service_api::ApprovalServiceFuture;
    use codex_command_service_api::CommandServiceFuture;
    use codex_command_runtime::CommandSessionError;
    use codex_command_runtime::CommandWaitOperation;
    use codex_command_runtime::CommandWaitRequest;
    use codex_command_runtime::WriteStdinOutput;
    use codex_command_runtime::WriteStdinRequest;
    use codex_thread_runtime::test_support;
    use codex_tool_runtime::TurnDiffTracker;

    struct PanickingApprovalService;

    impl ApprovalServiceApi for PanickingApprovalService {
        fn request_apply_patch_approval(
            &self,
            _request: codex_approval_service_api::ApplyPatchApprovalDispatch,
        ) -> ApprovalServiceFuture<'_, Result<(), String>> {
            Box::pin(async { panic!("unexpected apply_patch approval request") })
        }

        fn request_exec_command_approval(
            &self,
            _request: ExecCommandApprovalDispatch,
        ) -> ApprovalServiceFuture<'_, Result<ExecCommandApprovalOutcome, String>> {
            Box::pin(async { panic!("unexpected exec_command approval request") })
        }
    }

    struct PanickingCommandService;

    impl CommandServiceApi for PanickingCommandService {
        fn run_exec_command<'a>(
            &'a self,
            _session: Arc<dyn CommandServiceSessionCapability>,
            _turn: Arc<dyn CommandServiceTurnCapability>,
            _call_id: String,
            _request: ExecCommandRunRequest,
        ) -> CommandServiceFuture<'a, Result<ExecCommandRunOutput, UnifiedExecError>> {
            Box::pin(async { panic!("unexpected exec_command run") })
        }

        fn begin_command_wait<'a>(
            &'a self,
            _session: Arc<dyn CommandServiceSessionCapability>,
            _request: CommandWaitRequest,
        ) -> CommandServiceFuture<'a, Result<Box<dyn CommandWaitOperation>, CommandSessionError>>
        {
            Box::pin(async { panic!("unexpected command_wait") })
        }

        fn write_command_stdin<'a>(
            &'a self,
            _session: Arc<dyn CommandServiceSessionCapability>,
            _request: WriteStdinRequest<'a>,
        ) -> CommandServiceFuture<'a, Result<WriteStdinOutput, CommandSessionError>> {
            Box::pin(async { panic!("unexpected command_write_stdin") })
        }
    }

    #[tokio::test]
    async fn exec_command_rejects_incompatible_payload() {
        let (session, turn) = test_support::make_session_and_context().await;
        let tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));

        let result = dispatch(
            Arc::new(PanickingApprovalService),
            Arc::new(PanickingCommandService),
            session,
            turn,
            tracker,
            ToolCall {
                call_id: "call-1".to_string(),
                tool_name: ToolName::plain("exec_command"),
                payload: codex_tool_types::ToolPayload::Custom {
                    input: "{}".to_string(),
                },
            },
        )
        .await;

        let Err(FunctionCallError::Fatal(message)) = result else {
            panic!("expected incompatible payload error");
        };
        assert_eq!(message, "tool exec_command invoked with incompatible payload");
    }

}
