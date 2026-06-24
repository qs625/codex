use crate::FunctionToolOutput;
use crate::ShellRequest;
use crate::ShellRuntime;
use crate::apply_granted_permissions_from_grants;
use crate::format_exec_output_str;
use crate::handlers::apply_patch::ApplyPatchActiveNetworkApproval;
use crate::handlers::apply_patch::ApplyPatchDeferredNetworkApproval;
use crate::implicit_granted_permissions;
use crate::intercept_apply_patch;
use crate::normalize_and_validate_additional_permissions;
use codex_permissions_runtime::ExecPolicyApprovalRequest;
use codex_protocol::models::SandboxPermissions;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::ExecCommandSource;
use codex_tool_config::ShellCommandBackendConfig;
use codex_tool_planning::CommandToolOptions;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_shell_command_tool;
use codex_tool_runtime::ToolCtx;
use codex_tool_runtime::ToolEmitter;
use codex_tool_runtime::ToolEventCtx;
use codex_tool_runtime::ToolOrchestrator;
use codex_tool_runtime_api::ApplyPatchHandlerHost;
use codex_tool_runtime_api::HookToolName;
use codex_tool_runtime_api::PostToolUsePayload;
use codex_tool_runtime_api::PreToolUsePayload;
use codex_tool_runtime_api::RunExecLikeArgs;
use codex_tool_runtime_api::ShellCommandHandlerHost;
use codex_tool_runtime_api::ShellExecutionHost;
use codex_tool_runtime_api::ShellRuntimeHost as ShellRuntimeHostTrait;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_runtime_api::ToolInvocationView;
use codex_tool_runtime_api::ToolOrchestratorHost;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use serde_json::Value as JsonValue;

pub type ShellNetworkTrigger<Host> =
    <<Host as ShellExecutionHost>::ShellHost as ShellRuntimeHostTrait>::NetworkApprovalTrigger;

pub type ShellActiveNetworkApproval<Host> =
    <<Host as ShellExecutionHost>::ShellOrchestratorHost as ToolOrchestratorHost<
        <Host as ApplyPatchHandlerHost>::Session,
        <Host as ApplyPatchHandlerHost>::Turn,
        ShellNetworkTrigger<Host>,
    >>::ActiveNetworkApproval;

pub type ShellDeferredNetworkApproval<Host> =
    <<Host as ShellExecutionHost>::ShellOrchestratorHost as ToolOrchestratorHost<
        <Host as ApplyPatchHandlerHost>::Session,
        <Host as ApplyPatchHandlerHost>::Turn,
        ShellNetworkTrigger<Host>,
    >>::DeferredNetworkApproval;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ShellCommandBackend {
    Classic,
    ZshFork,
}

pub struct ShellCommandHandler<Host> {
    host: Host,
    backend: ShellCommandBackend,
    options: Option<ShellCommandHandlerOptions>,
}

#[derive(Clone, Copy)]
pub struct ShellCommandHandlerOptions {
    pub backend_config: ShellCommandBackendConfig,
    pub allow_login_shell: bool,
    pub exec_permission_approvals_enabled: bool,
}

impl<Host> ShellCommandHandler<Host> {
    pub fn new(host: Host, options: ShellCommandHandlerOptions) -> Self {
        let backend = Self::backend_from_config(options.backend_config);
        Self {
            host,
            backend,
            options: Some(options),
        }
    }

    pub fn from_backend_config(host: Host, config: ShellCommandBackendConfig) -> Self {
        Self {
            host,
            backend: Self::backend_from_config(config),
            options: None,
        }
    }

    fn backend_from_config(config: ShellCommandBackendConfig) -> ShellCommandBackend {
        match config {
            ShellCommandBackendConfig::Classic => ShellCommandBackend::Classic,
            ShellCommandBackendConfig::ZshFork => ShellCommandBackend::ZshFork,
        }
    }

    fn shell_runtime_backend(&self) -> crate::ShellRuntimeBackend {
        match self.backend {
            ShellCommandBackend::Classic => crate::ShellRuntimeBackend::ShellCommandClassic,
            ShellCommandBackend::ZshFork => crate::ShellRuntimeBackend::ShellCommandZshFork,
        }
    }
}

impl<Host>
    ToolExecutor<codex_tool_runtime::ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for ShellCommandHandler<Host>
where
    Host: ShellCommandHandlerHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
{
    type Output = crate::FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("shell_command")
    }

    fn spec(&self) -> Option<ToolSpec> {
        self.options.map(|options| {
            create_shell_command_tool(CommandToolOptions {
                allow_login_shell: options.allow_login_shell,
                exec_permission_approvals_enabled: options.exec_permission_approvals_enabled,
            })
        })
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        self.options.is_some()
    }

    fn handle<'a>(
        &'a self,
        invocation: codex_tool_runtime::ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let codex_tool_runtime::ToolInvocation {
                session,
                turn,
                tracker,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let payload = metadata.payload;

            let tool_name = self.tool_name();
            let ToolPayload::Function { arguments } = payload else {
                return Err(FunctionCallError::RespondToModel(format!(
                    "unsupported payload for shell_command handler: {tool_name}"
                )));
            };

            let cwd = self.host.resolve_workdir_base_path(&turn, &arguments)?;
            let params = self.host.parse_shell_command_params(&arguments, &cwd)?;
            let workdir = self
                .host
                .resolve_shell_workdir(&turn, params.workdir.clone());
            self.host
                .maybe_emit_implicit_skill_invocation(&session, &turn, &params.command, &workdir)
                .await;
            let prefix_rule = params.prefix_rule.clone();
            let exec_params = self
                .host
                .shell_command_exec_params(&params, &session, &turn)?;
            let shell_type = self.host.shell_type(&session);
            run_exec_like(
                &self.host,
                RunExecLikeArgs {
                    tool_name,
                    exec_params,
                    hook_command: params.command,
                    shell_type,
                    additional_permissions: params.additional_permissions.clone(),
                    prefix_rule,
                    session,
                    turn,
                    tracker,
                    call_id,
                    freeform: true,
                    shell_runtime_backend: self.shell_runtime_backend(),
                },
            )
            .await
        })
    }
}

impl<Host>
    ToolHandler<
        codex_tool_runtime::ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
        Host::DiffContext,
    > for ShellCommandHandler<Host>
where
    Host: ShellCommandHandlerHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    fn pre_tool_use_payload(
        &self,
        invocation: &codex_tool_runtime::ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> Option<PreToolUsePayload> {
        shell_command_payload_command(invocation.payload()).map(|command| PreToolUsePayload {
            tool_name: HookToolName::bash(),
            tool_input: serde_json::json!({ "command": command }),
        })
    }

    fn with_updated_hook_input(
        &self,
        mut invocation: codex_tool_runtime::ToolInvocation<
            Host::Session,
            Host::Turn,
            Host::Tracker,
        >,
        updated_input: serde_json::Value,
    ) -> Result<
        codex_tool_runtime::ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
        FunctionCallError,
    > {
        let ToolPayload::Function { arguments } = &invocation.metadata.payload else {
            return Err(FunctionCallError::RespondToModel(
                "hook input rewrite received unsupported shell_command payload".to_string(),
            ));
        };
        invocation.metadata.payload = ToolPayload::Function {
            arguments: rewrite_function_string_argument(
                arguments,
                "shell_command",
                "command",
                updated_hook_command(&updated_input)?,
            )?,
        };
        Ok(invocation)
    }

    fn post_tool_use_payload(
        &self,
        invocation: &codex_tool_runtime::ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
        result: &Self::Output,
    ) -> Option<PostToolUsePayload> {
        shell_command_post_tool_use_payload(invocation.call_id(), invocation.payload(), result)
    }
}

pub async fn run_exec_like<Host>(
    host: &Host,
    args: RunExecLikeArgs<Host::Session, Host::Turn, Host::Tracker>,
) -> Result<FunctionToolOutput, FunctionCallError>
where
    Host: ShellExecutionHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
{
    let RunExecLikeArgs {
        tool_name,
        mut exec_params,
        hook_command,
        shell_type,
        additional_permissions,
        prefix_rule,
        session,
        turn,
        tracker,
        call_id,
        freeform,
        shell_runtime_backend,
    } = args;

    let Some(turn_environment) = host.primary_environment(&turn)? else {
        return Err(FunctionCallError::RespondToModel(
            "shell is unavailable in this session".to_string(),
        ));
    };
    let fs = turn_environment.environment.filesystem();

    let dependency_env = host.dependency_env(&session).await;
    if !dependency_env.is_empty() {
        exec_params.env.extend(dependency_env.clone());
    }

    let mut explicit_env_overrides = host.explicit_env_overrides(&turn);
    for key in dependency_env.keys() {
        if let Some(value) = exec_params.env.get(key) {
            explicit_env_overrides.insert(key.clone(), value.clone());
        }
    }

    let exec_permission_approvals_enabled = host.exec_permission_approvals_enabled(&session);
    let requested_additional_permissions = additional_permissions.clone();
    let grants = host.permission_grants(&session).await;
    let effective_additional_permissions = apply_granted_permissions_from_grants(
        grants,
        exec_params.cwd.as_path(),
        exec_params.sandbox_permissions,
        additional_permissions,
    );
    let additional_permissions_allowed = exec_permission_approvals_enabled
        || (host.request_permissions_tool_enabled(&session)
            && effective_additional_permissions.permissions_preapproved);
    let normalized_additional_permissions = implicit_granted_permissions(
        exec_params.sandbox_permissions,
        requested_additional_permissions.as_ref(),
        &effective_additional_permissions,
    )
    .map_or_else(
        || {
            normalize_and_validate_additional_permissions(
                additional_permissions_allowed,
                host.approval_policy(&turn),
                effective_additional_permissions.sandbox_permissions,
                effective_additional_permissions
                    .additional_permissions
                    .clone(),
                effective_additional_permissions.permissions_preapproved,
                &exec_params.cwd,
            )
        },
        |permissions| Ok(Some(permissions)),
    )
    .map_err(FunctionCallError::RespondToModel)?;

    if effective_additional_permissions
        .sandbox_permissions
        .requests_sandbox_override()
        && !effective_additional_permissions.permissions_preapproved
        && !matches!(host.approval_policy(&turn), AskForApproval::OnRequest)
    {
        let approval_policy = host.approval_policy(&turn);
        return Err(FunctionCallError::RespondToModel(format!(
            "approval policy is {approval_policy:?}; reject command — you should not ask for escalated permissions if the approval policy is {approval_policy:?}"
        )));
    }

    if let Some(output) = intercept_apply_patch(
        host,
        &exec_params.command,
        &exec_params.cwd,
        fs.as_ref(),
        turn_environment.environment.clone(),
        session.clone(),
        turn.clone(),
        Some(&tracker),
        &call_id,
        tool_name.name.as_str(),
    )
    .await?
    {
        return Ok(output);
    }

    let source = ExecCommandSource::Agent;
    let emitter = ToolEmitter::shell(
        exec_params.command.clone(),
        exec_params.cwd.clone(),
        source,
        freeform,
    );
    emitter
        .begin(ToolEventCtx::new(
            host.event_host(&session, &turn, /*tracker*/ None),
            &call_id,
        ))
        .await;

    let file_system_sandbox_policy = host.file_system_sandbox_policy(&turn);
    let exec_approval_requirement = host
        .create_exec_approval_requirement(
            &session,
            ExecPolicyApprovalRequest {
                command: &exec_params.command,
                approval_policy: host.approval_policy(&turn),
                permission_profile: host.permission_profile(&turn),
                file_system_sandbox_policy: &file_system_sandbox_policy,
                sandbox_cwd: exec_params.cwd.as_path(),
                sandbox_permissions: if effective_additional_permissions.permissions_preapproved {
                    SandboxPermissions::UseDefault
                } else {
                    effective_additional_permissions.sandbox_permissions
                },
                prefix_rule,
            },
        )
        .await;

    let req = ShellRequest {
        command: exec_params.command.clone(),
        shell_type,
        hook_command,
        cwd: exec_params.cwd.clone(),
        timeout_ms: exec_params.expiration.timeout_ms(),
        env: exec_params.env.clone(),
        explicit_env_overrides,
        network: exec_params.network.clone(),
        sandbox_permissions: effective_additional_permissions.sandbox_permissions,
        additional_permissions: normalized_additional_permissions,
        #[cfg(unix)]
        additional_permissions_preapproved: effective_additional_permissions
            .permissions_preapproved,
        justification: exec_params.justification.clone(),
        exec_approval_requirement,
    };
    let mut orchestrator = ToolOrchestrator::new(
        host.shell_orchestrator_host(),
        host.sandbox_runtime(&session),
    );
    let mut runtime =
        ShellRuntime::for_shell_command(host.shell_runtime_host(), shell_runtime_backend);
    let tool_ctx = ToolCtx {
        session: session.clone(),
        turn: turn.clone(),
        call_id: call_id.clone(),
        tool_name,
    };
    let out = orchestrator
        .run(
            &mut runtime,
            &req,
            &tool_ctx,
            &host.tool_sandbox_context(&turn),
            host.approval_policy(&turn),
        )
        .await
        .map(|result| result.output);
    let post_tool_use_response = out
        .as_ref()
        .ok()
        .map(|output| format_exec_output_str(output, host.truncation_policy(&turn)))
        .map(JsonValue::String);
    let content = emitter
        .finish(
            ToolEventCtx::new(host.event_host(&session, &turn, /*tracker*/ None), &call_id),
            out,
            /*applied_patch_delta*/ None,
        )
        .await?;
    Ok(FunctionToolOutput {
        body: vec![
            codex_protocol::models::FunctionCallOutputContentItem::InputText { text: content },
        ],
        success: Some(true),
        post_tool_use_response,
    })
}

pub fn shell_command_payload_command(payload: &ToolPayload) -> Option<String> {
    let ToolPayload::Function { arguments } = payload else {
        return None;
    };

    serde_json::from_str::<codex_protocol::models::ShellCommandToolCallParams>(arguments)
        .ok()
        .map(|params| params.command)
}

fn shell_command_post_tool_use_payload(
    tool_use_id: &str,
    payload: &ToolPayload,
    result: &FunctionToolOutput,
) -> Option<PostToolUsePayload> {
    let tool_response = result.post_tool_use_response(tool_use_id, payload)?;
    let command = shell_command_payload_command(payload)?;
    Some(PostToolUsePayload {
        tool_name: HookToolName::bash(),
        tool_use_id: tool_use_id.to_string(),
        tool_input: serde_json::json!({ "command": command }),
        tool_response,
    })
}

fn updated_hook_command(updated_input: &serde_json::Value) -> Result<&str, FunctionCallError> {
    updated_input
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(
                "hook returned updatedInput without string field `command`".to_string(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_protocol::models::FunctionCallOutputContentItem;
    use serde_json::json;

    #[test]
    fn shell_command_payload_command_uses_raw_command() {
        let payload = ToolPayload::Function {
            arguments: json!({ "command": "printf shell command" }).to_string(),
        };

        assert_eq!(
            shell_command_payload_command(&payload),
            Some("printf shell command".to_string())
        );
    }

    #[test]
    fn shell_command_post_tool_use_payload_uses_tool_output_wire_value() {
        let payload = ToolPayload::Function {
            arguments: json!({ "command": "printf shell command" }).to_string(),
        };
        let output = FunctionToolOutput {
            body: vec![FunctionCallOutputContentItem::InputText {
                text: "ignored display text".to_string(),
            }],
            success: Some(true),
            post_tool_use_response: Some(json!("shell output")),
        };

        assert_eq!(
            shell_command_post_tool_use_payload("call-42", &payload, &output),
            Some(PostToolUsePayload {
                tool_name: HookToolName::bash(),
                tool_use_id: "call-42".to_string(),
                tool_input: json!({ "command": "printf shell command" }),
                tool_response: json!("shell output"),
            })
        );
    }
}

fn rewrite_function_string_argument(
    arguments: &str,
    tool_name: &str,
    field_name: &str,
    value: &str,
) -> Result<String, FunctionCallError> {
    let mut arguments: serde_json::Value = serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })?;
    let serde_json::Value::Object(arguments) = &mut arguments else {
        return Err(FunctionCallError::RespondToModel(format!(
            "{tool_name} arguments must be an object"
        )));
    };
    arguments.insert(
        field_name.to_string(),
        serde_json::Value::String(value.to_string()),
    );
    serde_json::to_string(&arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to serialize rewritten {tool_name} arguments: {err}"
        ))
    })
}
