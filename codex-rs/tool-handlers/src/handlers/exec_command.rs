use crate::ExecCommandToolOutput;
use crate::handlers::apply_patch::ApplyPatchActiveNetworkApproval;
use crate::handlers::apply_patch::ApplyPatchDeferredNetworkApproval;
use crate::handlers::apply_patch::implicit_granted_permissions;
use crate::handlers::apply_patch::intercept_apply_patch;
use crate::handlers::apply_patch::normalize_and_validate_additional_permissions;
use codex_command_runtime::UnifiedExecError;
use codex_command_runtime::generate_chunk_id;
use codex_command_runtime::resolve_max_tokens;
use codex_protocol::protocol::AskForApproval;
#[cfg(test)]
use codex_tool_config::ToolUserShellType;
use codex_tool_planning::CommandToolOptions;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_exec_command_tool_with_environment_id;
use codex_tool_runtime::ToolInvocation;
use codex_tool_runtime_api::ExecCommandArgs;
use codex_tool_runtime_api::ExecCommandHandlerHost;
use codex_tool_runtime_api::ExecCommandRunOutput;
use codex_tool_runtime_api::ExecCommandRunRequest;
use codex_tool_runtime_api::HookToolName;
use codex_tool_runtime_api::PostToolUsePayload;
use codex_tool_runtime_api::PreToolUsePayload;
use codex_tool_runtime_api::RuntimeShell;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_runtime_api::ToolInvocationView;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolOutput;
use codex_tool_types::ToolPayload;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_absolute_path::AbsolutePathBufGuard;
use codex_utils_output_truncation::TruncationPolicy;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Copy)]
pub struct ExecCommandHandlerOptions {
    pub allow_login_shell: bool,
    pub exec_permission_approvals_enabled: bool,
    pub include_environment_id: bool,
}

pub struct ExecCommandHandler<Host> {
    host: Host,
    options: ExecCommandHandlerOptions,
}

impl<Host> ExecCommandHandler<Host> {
    pub fn new(host: Host, options: ExecCommandHandlerOptions) -> Self {
        Self { host, options }
    }
}

impl<Host> ToolExecutor<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>>
    for ExecCommandHandler<Host>
where
    Host: ExecCommandHandlerHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
{
    type Output = ExecCommandToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("exec_command")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_exec_command_tool_with_environment_id(
            CommandToolOptions {
                allow_login_shell: self.options.allow_login_shell,
                exec_permission_approvals_enabled: self.options.exec_permission_approvals_enabled,
            },
            self.options.include_environment_id,
        ))
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                tracker,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let payload = metadata.payload;

            let ToolPayload::Function { arguments } = payload else {
                return Err(FunctionCallError::RespondToModel(
                    "exec_command handler received unsupported payload".to_string(),
                ));
            };

            let environment_args: ExecCommandEnvironmentArgs = parse_arguments(&arguments)?;
            let Some(turn_environment) = self.host.resolve_exec_command_environment(
                &turn,
                environment_args.environment_id.as_deref(),
                environment_args.workdir.as_deref(),
            )?
            else {
                return Err(FunctionCallError::RespondToModel(
                    "unified exec is unavailable in this session".to_string(),
                ));
            };
            let cwd = turn_environment.cwd.clone();
            let args: ExecCommandArgs = parse_arguments_with_base_path(&arguments, &cwd)?;
            let hook_command = args.cmd.clone();
            self.host
                .maybe_emit_implicit_skill_invocation(&session, &turn, &hook_command, &cwd)
                .await;

            let model_shell = args
                .shell
                .as_deref()
                .map(|shell| self.host.resolve_model_shell(&PathBuf::from(shell)));
            let resolved_command = self
                .host
                .resolve_exec_command(&args.cmd, args.login, model_shell.as_ref(), &session, &turn)
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
            let max_output_tokens =
                effective_max_output_tokens(max_output_tokens, self.host.truncation_policy(&turn));

            let exec_permission_approvals_enabled =
                self.host.exec_permission_approvals_enabled(&session);
            let requested_additional_permissions = additional_permissions.clone();
            let grants = self.host.permission_grants(&session).await;
            let effective_additional_permissions = crate::apply_granted_permissions_from_grants(
                grants,
                cwd.as_path(),
                sandbox_permissions,
                additional_permissions,
            );
            let additional_permissions_allowed = exec_permission_approvals_enabled
                || (self.host.request_permissions_tool_enabled(&session)
                    && effective_additional_permissions.permissions_preapproved);

            if effective_additional_permissions
                .sandbox_permissions
                .requests_sandbox_override()
                && !effective_additional_permissions.permissions_preapproved
                && !matches!(self.host.approval_policy(&turn), AskForApproval::OnRequest)
            {
                let approval_policy = self.host.approval_policy(&turn);
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
                        self.host.approval_policy(&turn),
                        effective_additional_permissions.sandbox_permissions,
                        effective_additional_permissions.additional_permissions,
                        effective_additional_permissions.permissions_preapproved,
                        &cwd,
                    )
                },
                |permissions| Ok(Some(permissions)),
            )
            .map_err(FunctionCallError::RespondToModel)?;

            if let Some(output) = intercept_apply_patch(
                &self.host,
                &command,
                &cwd,
                turn_environment
                    .apply_patch_environment
                    .filesystem()
                    .as_ref(),
                turn_environment.apply_patch_environment.clone(),
                session.clone(),
                turn.clone(),
                Some(&tracker),
                &call_id,
                "exec_command",
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

            self.host.emit_unified_exec_tty_metric(&turn, tty);
            let process_id = self.host.allocate_exec_process_id(&session).await;
            let exec_approval_requirement = self
                .host
                .create_exec_approval_requirement(
                    &session,
                    codex_permissions_runtime::ExecPolicyApprovalRequest {
                        command: &command,
                        approval_policy: self.host.approval_policy(&turn),
                        permission_profile: self.host.permission_profile(&turn),
                        file_system_sandbox_policy: &self.host.file_system_sandbox_policy(&turn),
                        sandbox_cwd: turn_environment.sandbox_cwd.as_path(),
                        sandbox_permissions: if effective_additional_permissions.permissions_preapproved
                        {
                            codex_protocol::models::SandboxPermissions::UseDefault
                        } else {
                            effective_additional_permissions.sandbox_permissions
                        },
                        prefix_rule: prefix_rule.clone(),
                    },
                )
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
                additional_permissions_preapproved: effective_additional_permissions
                    .permissions_preapproved,
                justification,
                prefix_rule,
                notify_on: notify_on.into(),
                approval_mode: codex_tool_runtime_api::ExecCommandApprovalMode::ContinueInRuntime,
                exec_approval_requirement,
            };
            match self
                .host
                .run_exec_command(&session, &turn, &call_id, run_request)
                .await
            {
                Ok(response) => Ok(exec_command_tool_output_from_run_output(response)),
                Err(UnifiedExecError::SandboxDenied { output, .. }) => {
                    let output_text = output.aggregated_output.text;
                    let original_token_count =
                        codex_utils_output_truncation::approx_token_count(&output_text);
                    Ok(ExecCommandToolOutput {
                        event_call_id: call_id,
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
        })
    }
}

impl<Host> ToolHandler<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, Host::DiffContext>
    for ExecCommandHandler<Host>
where
    Host: ExecCommandHandlerHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }

    fn pre_tool_use_payload(
        &self,
        invocation: &ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
    ) -> Option<PreToolUsePayload> {
        exec_command_payload_command(invocation.payload()).map(|command| PreToolUsePayload {
            tool_name: HookToolName::bash(),
            tool_input: serde_json::json!({ "command": command }),
        })
    }

    fn with_updated_hook_input(
        &self,
        mut invocation: ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
        updated_input: serde_json::Value,
    ) -> Result<ToolInvocation<Host::Session, Host::Turn, Host::Tracker>, FunctionCallError> {
        let ToolPayload::Function { arguments } = &invocation.metadata.payload else {
            return Err(FunctionCallError::RespondToModel(
                "hook input rewrite received unsupported exec_command payload".to_string(),
            ));
        };
        invocation.metadata.payload = ToolPayload::Function {
            arguments: rewrite_function_string_argument(
                arguments,
                "exec_command",
                "cmd",
                updated_hook_command(&updated_input)?,
            )?,
        };
        Ok(invocation)
    }

    fn post_tool_use_payload(
        &self,
        invocation: &ToolInvocation<Host::Session, Host::Turn, Host::Tracker>,
        result: &Self::Output,
    ) -> Option<PostToolUsePayload> {
        post_unified_exec_tool_use_payload(invocation.call_id(), invocation.payload(), result)
    }
}

#[derive(Debug, Deserialize)]
struct ExecCommandEnvironmentArgs {
    #[serde(default)]
    environment_id: Option<String>,
    #[serde(default)]
    workdir: Option<String>,
}

pub fn get_command(
    args: &ExecCommandArgs,
    session_shell: &RuntimeShell,
    model_shell: Option<&RuntimeShell>,
    shell_mode: &codex_tool_config::UnifiedExecShellMode,
    allow_login_shell: bool,
) -> Result<codex_tool_runtime_api::ResolvedExecCommand, String> {
    codex_tool_runtime_api::resolve_exec_command(
        args,
        session_shell,
        model_shell,
        shell_mode,
        allow_login_shell,
    )
}

pub fn get_command_for_parts(
    command: &str,
    login: Option<bool>,
    session_shell: &RuntimeShell,
    model_shell: Option<&RuntimeShell>,
    shell_mode: &codex_tool_config::UnifiedExecShellMode,
    allow_login_shell: bool,
) -> Result<codex_tool_runtime_api::ResolvedExecCommand, String> {
    codex_tool_runtime_api::resolve_exec_command_for_parts(
        command,
        login,
        session_shell,
        model_shell,
        shell_mode,
        allow_login_shell,
    )
}

fn effective_max_output_tokens(
    max_output_tokens: Option<usize>,
    truncation_policy: TruncationPolicy,
) -> usize {
    resolve_max_tokens(max_output_tokens).min(truncation_policy.token_budget())
}

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

fn parse_arguments_with_base_path<T>(
    arguments: &str,
    base_path: &AbsolutePathBuf,
) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    let _guard = AbsolutePathBufGuard::new(base_path);
    parse_arguments(arguments)
}

pub fn exec_command_payload_command(payload: &ToolPayload) -> Option<String> {
    let ToolPayload::Function { arguments } = payload else {
        return None;
    };

    parse_arguments::<ExecCommandArgs>(arguments)
        .ok()
        .map(|args| args.cmd)
}

fn post_unified_exec_tool_use_payload(
    call_id: &str,
    payload: &ToolPayload,
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
    use codex_tool_config::UnifiedExecShellMode;
    use codex_tool_runtime_api::PostToolUsePayload;
    use codex_tool_runtime_api::PreToolUsePayload;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    fn runtime_shell(shell_type: ToolUserShellType, shell_path: &str) -> RuntimeShell {
        RuntimeShell {
            shell_type,
            shell_path: PathBuf::from(shell_path),
            shell_snapshot: None,
        }
    }

    #[test]
    fn get_command_uses_default_shell_when_unspecified() -> anyhow::Result<()> {
        let args: ExecCommandArgs = parse_arguments(r#"{"cmd": "echo hello"}"#)?;
        let resolved = get_command(
            &args,
            &runtime_shell(ToolUserShellType::Sh, "/bin/sh"),
            None,
            &UnifiedExecShellMode::Direct,
            /*allow_login_shell*/ true,
        )
        .map_err(anyhow::Error::msg)?;

        assert_eq!(resolved.command, vec!["/bin/sh", "-lc", "echo hello"]);
        assert_eq!(resolved.shell_type, ToolUserShellType::Sh);
        Ok(())
    }

    #[test]
    fn get_command_respects_explicit_model_shell() -> anyhow::Result<()> {
        let args: ExecCommandArgs = parse_arguments(r#"{"cmd": "echo hello"}"#)?;
        let resolved = get_command(
            &args,
            &runtime_shell(ToolUserShellType::Sh, "/bin/sh"),
            Some(&runtime_shell(ToolUserShellType::Bash, "/bin/bash")),
            &UnifiedExecShellMode::Direct,
            /*allow_login_shell*/ true,
        )
        .map_err(anyhow::Error::msg)?;

        assert_eq!(resolved.command, vec!["/bin/bash", "-lc", "echo hello"]);
        assert_eq!(resolved.shell_type, ToolUserShellType::Bash);
        Ok(())
    }

    #[test]
    fn get_command_respects_powershell_profile_flag() -> anyhow::Result<()> {
        let args: ExecCommandArgs = parse_arguments(r#"{"cmd": "echo hello"}"#)?;
        let resolved = get_command(
            &args,
            &runtime_shell(ToolUserShellType::PowerShell, "powershell"),
            None,
            &UnifiedExecShellMode::Direct,
            /*allow_login_shell*/ false,
        )
        .map_err(anyhow::Error::msg)?;

        assert_eq!(
            resolved.command,
            vec!["powershell", "-NoProfile", "-Command", "echo hello"]
        );
        assert_eq!(resolved.shell_type, ToolUserShellType::PowerShell);
        Ok(())
    }

    #[test]
    fn get_command_rejects_explicit_login_when_disallowed() -> anyhow::Result<()> {
        let args: ExecCommandArgs = parse_arguments(r#"{"cmd": "echo hello", "login": true}"#)?;
        let err = get_command(
            &args,
            &runtime_shell(ToolUserShellType::Sh, "/bin/sh"),
            None,
            &UnifiedExecShellMode::Direct,
            /*allow_login_shell*/ false,
        )
        .expect_err("explicit login should be rejected");

        assert!(
            err.contains("login shell is disabled by config"),
            "unexpected error: {err}"
        );
        Ok(())
    }

    #[test]
    fn get_command_ignores_explicit_shell_in_zsh_fork_mode() -> anyhow::Result<()> {
        let args: ExecCommandArgs = parse_arguments(r#"{"cmd": "echo hello"}"#)?;
        let shell_zsh_path = AbsolutePathBuf::from_absolute_path(if cfg!(windows) {
            r"C:\opt\codex\zsh"
        } else {
            "/opt/codex/zsh"
        })?;
        let shell_mode = UnifiedExecShellMode::ZshFork(codex_tool_config::ZshForkConfig {
            shell_zsh_path: shell_zsh_path.clone(),
            main_execve_wrapper_exe: AbsolutePathBuf::from_absolute_path(if cfg!(windows) {
                r"C:\opt\codex\codex-execve-wrapper"
            } else {
                "/opt/codex/codex-execve-wrapper"
            })?,
        });

        let resolved = get_command(
            &args,
            &runtime_shell(ToolUserShellType::Bash, "/bin/bash"),
            Some(&runtime_shell(ToolUserShellType::Sh, "/bin/sh")),
            &shell_mode,
            /*allow_login_shell*/ true,
        )
        .map_err(anyhow::Error::msg)?;

        assert_eq!(
            resolved.command,
            vec![
                shell_zsh_path.to_string_lossy().to_string(),
                "-lc".to_string(),
                "echo hello".to_string()
            ]
        );
        assert_eq!(resolved.shell_type, ToolUserShellType::Zsh);
        Ok(())
    }

    #[test]
    fn exec_command_pre_tool_use_payload_uses_raw_command() {
        let payload = ToolPayload::Function {
            arguments: serde_json::json!({ "cmd": "printf exec command" }).to_string(),
        };

        assert_eq!(
            exec_command_payload_command(&payload).map(|command| PreToolUsePayload {
                tool_name: HookToolName::bash(),
                tool_input: serde_json::json!({ "command": command }),
            }),
            Some(PreToolUsePayload {
                tool_name: HookToolName::bash(),
                tool_input: serde_json::json!({ "command": "printf exec command" }),
            })
        );
    }

    #[test]
    fn exec_command_post_tool_use_payload_uses_output_for_completed_commands() {
        let payload = ToolPayload::Function {
            arguments: serde_json::json!({ "cmd": "echo three", "tty": false }).to_string(),
        };
        let output = ExecCommandToolOutput {
            event_call_id: "call-43".to_string(),
            chunk_id: "chunk-1".to_string(),
            wall_time: std::time::Duration::from_millis(498),
            raw_output: b"three".to_vec(),
            max_output_tokens: None,
            process_id: None,
            exit_code: Some(0),
            original_token_count: None,
            hook_command: Some("echo three".to_string()),
        };

        assert_eq!(
            post_unified_exec_tool_use_payload("call-43", &payload, &output),
            Some(PostToolUsePayload {
                tool_name: HookToolName::bash(),
                tool_use_id: "call-43".to_string(),
                tool_input: serde_json::json!({ "command": "echo three" }),
                tool_response: serde_json::json!("three"),
            })
        );
    }

    #[test]
    fn exec_command_post_tool_use_payload_skips_running_sessions() {
        let payload = ToolPayload::Function {
            arguments: serde_json::json!({ "cmd": "echo three", "tty": false }).to_string(),
        };
        let output = ExecCommandToolOutput {
            event_call_id: "event-45".to_string(),
            chunk_id: "chunk-1".to_string(),
            wall_time: std::time::Duration::from_millis(498),
            raw_output: b"three".to_vec(),
            max_output_tokens: None,
            process_id: Some(45),
            exit_code: None,
            original_token_count: None,
            hook_command: Some("echo three".to_string()),
        };

        assert_eq!(
            post_unified_exec_tool_use_payload("call-45", &payload, &output),
            None
        );
    }
}
