use crate::exec::ExecCapturePolicy;
use crate::exec::ExecExpiration;
use crate::exec::cancel_when_either;
use crate::guardian::GuardianApprovalRequest;
use crate::guardian::guardian_rejection_message;
use crate::guardian::guardian_timeout_message;
use crate::guardian::new_guardian_review_id;
use crate::guardian::review_approval_request;
use crate::guardian::routes_approval_to_guardian;
use crate::sandboxing::ExecOptions;
use crate::sandboxing::ExecRequest;
use crate::sandboxing::SandboxPermissions;
use crate::shell::ShellType;
use crate::tools::runtimes::build_sandbox_command;
use crate::tools::runtimes::exec_env_for_sandbox_permissions;
use crate::tools::sandboxing::PermissionRequestPayload;
use crate::tools::sandboxing::SandboxAttempt;
use crate::tools::sandboxing::SandboxAttemptExt;
use crate::tools::sandboxing::ToolCtx;
use crate::tools::sandboxing::ToolError;
use crate::tools::sandboxing::managed_network_for_sandbox_permissions;
use crate::tools::sandboxing::permission_request_hook_payload;
use codex_execpolicy_api::Policy;
use codex_features::Feature;
use codex_hooks::run_permission_request_hooks;
use codex_hooks_api::PermissionRequestDecision;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_permissions_runtime::join_program_and_argv;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::GuardianCommandSource;
use codex_protocol::protocol::ReviewDecision;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxTransformRequest;
use codex_sandboxing_api::SandboxType;
use codex_sandboxing_api::SandboxablePreference;
use codex_sandboxing_api::SharedSandboxRuntime;
use codex_shell_escalation::EscalateServer;
use codex_shell_escalation::EscalationExecution;
use codex_shell_escalation::EscalationPermissions;
use codex_shell_escalation::EscalationPolicy;
use codex_shell_escalation::EscalationPolicyDecisionParams;
use codex_shell_escalation::EscalationPolicyFuture;
use codex_shell_escalation::EscalationPromptDecision;
use codex_shell_escalation::EscalationPromptFuture;
use codex_shell_escalation::EscalationPromptHandler;
use codex_shell_escalation::EscalationPromptRequest;
use codex_shell_escalation::EscalationSession;
use codex_shell_escalation::ExecParams;
use codex_shell_escalation::ExecResult;
use codex_shell_escalation::PrepareEscalatedExecFuture;
use codex_shell_escalation::PreparedExec;
use codex_shell_escalation::ShellCommandExecutor;
use codex_shell_escalation::ShellCommandRunFuture;
use codex_shell_escalation::Stopwatch;
use codex_shell_escalation::approval_sandbox_permissions;
use codex_shell_escalation::determine_escalation_action;
use codex_shell_escalation::extract_shell_script;
use codex_shell_escalation::map_exec_result;
use codex_tool_runtime_api::ShellRequest;
use codex_tool_runtime_api::UnifiedExecRequest;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

pub(crate) struct PreparedUnifiedExecZshFork {
    pub(crate) exec_request: ExecRequest,
    pub(crate) escalation_session: EscalationSession,
}

pub(super) async fn try_run_zsh_fork(
    req: &ShellRequest,
    attempt: &SandboxAttempt<'_>,
    ctx: &ToolCtx,
    command: &[String],
) -> Result<Option<ExecToolCallOutput>, ToolError> {
    let Some(shell_zsh_path) = ctx.session.services.shell_zsh_path.as_ref() else {
        tracing::warn!("ZshFork backend specified, but shell_zsh_path is not configured.");
        return Ok(None);
    };
    if !ctx.session.features().enabled(Feature::ShellZshFork) {
        tracing::warn!("ZshFork backend specified, but ShellZshFork feature is not enabled.");
        return Ok(None);
    }
    if !matches!(ctx.session.user_shell().shell_type, ShellType::Zsh) {
        tracing::warn!("ZshFork backend specified, but user shell is not Zsh.");
        return Ok(None);
    }

    let env = exec_env_for_sandbox_permissions(&req.env, req.sandbox_permissions);
    let command =
        build_sandbox_command(command, &req.cwd, &env, req.additional_permissions.clone())?;
    let options = ExecOptions {
        expiration: req.timeout_ms.into(),
        capture_policy: ExecCapturePolicy::ShellTool,
    };
    let sandbox_exec_request = attempt
        .env_for(
            command,
            options,
            managed_network_for_sandbox_permissions(req.network.as_ref(), req.sandbox_permissions),
        )
        .map_err(|err| ToolError::Codex(err.into()))?;
    let crate::sandboxing::ExecRequest {
        command,
        cwd: sandbox_cwd,
        env: sandbox_env,
        network: sandbox_network,
        expiration: _sandbox_expiration,
        capture_policy: _capture_policy,
        sandbox,
        windows_sandbox_policy_cwd: sandbox_policy_cwd,
        windows_sandbox_level,
        windows_sandbox_private_desktop: _windows_sandbox_private_desktop,
        permission_profile,
        file_system_sandbox_policy,
        network_sandbox_policy,
        windows_sandbox_filesystem_overrides: _windows_sandbox_filesystem_overrides,
        arg0,
    } = sandbox_exec_request;
    let codex_shell_escalation::ParsedShellCommand { script, login, .. } =
        extract_shell_script(&command).map_err(|err| ToolError::Rejected(err.to_string()))?;
    let effective_timeout = Duration::from_millis(
        req.timeout_ms
            .unwrap_or(crate::exec::DEFAULT_EXEC_COMMAND_TIMEOUT_MS),
    );
    let exec_policy = Arc::new(RwLock::new(
        ctx.session.services.exec_policy.current().as_ref().clone(),
    ));
    let command_executor = CoreShellCommandExecutor {
        command,
        cwd: sandbox_cwd,
        permission_profile,
        file_system_sandbox_policy,
        network_sandbox_policy,
        sandbox,
        env: sandbox_env,
        network: sandbox_network,
        windows_sandbox_level,
        arg0,
        sandbox_policy_cwd,
        codex_linux_sandbox_exe: ctx.turn.codex_linux_sandbox_exe.clone(),
        use_legacy_landlock: ctx.turn.features.use_legacy_landlock(),
        sandbox_runtime: Arc::clone(&ctx.session.services.sandbox_runtime),
    };
    let main_execve_wrapper_exe = ctx
        .session
        .services
        .main_execve_wrapper_exe
        .clone()
        .ok_or_else(|| {
            ToolError::Rejected(
                "zsh fork feature enabled, but execve wrapper is not configured".to_string(),
            )
        })?;
    let exec_params = ExecParams {
        command: script,
        workdir: req.cwd.to_string_lossy().to_string(),
        timeout_ms: Some(effective_timeout.as_millis() as u64),
        login: Some(login),
    };

    // Note that Stopwatch starts immediately upon creation, so currently we try
    // to minimize the time between creating the Stopwatch and starting the
    // escalation server.
    let stopwatch = Stopwatch::new(effective_timeout);
    let mut cancel_token = stopwatch.cancellation_token();
    if let Some(cancellation) = attempt.network_denial_cancellation_token.clone() {
        cancel_token = cancel_when_either(cancel_token, cancellation);
    }
    let approval_sandbox_permissions = approval_sandbox_permissions(
        req.sandbox_permissions,
        req.additional_permissions_preapproved,
    );
    let escalation_policy = CoreShellActionProvider {
        policy: Arc::clone(&exec_policy),
        session: Arc::clone(&ctx.session),
        turn: Arc::clone(&ctx.turn),
        call_id: ctx.call_id.clone(),
        tool_name: GuardianCommandSource::Shell,
        approval_policy: ctx.turn.approval_policy.value(),
        permission_profile: command_executor.permission_profile.clone(),
        file_system_sandbox_policy: command_executor.file_system_sandbox_policy.clone(),
        sandbox_policy_cwd: command_executor.sandbox_policy_cwd.clone(),
        sandbox_permissions: req.sandbox_permissions,
        approval_sandbox_permissions,
        prompt_permissions: req.additional_permissions.clone(),
        stopwatch: stopwatch.clone(),
    };

    let escalate_server = EscalateServer::new(
        shell_zsh_path.clone(),
        main_execve_wrapper_exe,
        escalation_policy,
    );

    let exec_result = escalate_server
        .exec(exec_params, cancel_token, Arc::new(command_executor))
        .await
        .map_err(|err| ToolError::Rejected(err.to_string()))?;

    map_exec_result(attempt.sandbox, exec_result)
        .map_err(ToolError::Codex)
        .map(Some)
}

pub(crate) async fn prepare_unified_exec_zsh_fork(
    req: &UnifiedExecRequest,
    _attempt: &SandboxAttempt<'_>,
    ctx: &ToolCtx,
    exec_request: ExecRequest,
    shell_zsh_path: &std::path::Path,
    main_execve_wrapper_exe: &std::path::Path,
) -> Result<Option<PreparedUnifiedExecZshFork>, ToolError> {
    let parsed = match extract_shell_script(&exec_request.command) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::warn!("ZshFork unified exec fallback: {err:?}");
            return Ok(None);
        }
    };
    if parsed.program != shell_zsh_path.to_string_lossy() {
        tracing::warn!(
            "ZshFork backend specified, but unified exec command targets `{}` instead of `{}`.",
            parsed.program,
            shell_zsh_path.display(),
        );
        return Ok(None);
    }

    let exec_policy = Arc::new(RwLock::new(
        ctx.session.services.exec_policy.current().as_ref().clone(),
    ));
    let command_executor = CoreShellCommandExecutor {
        command: exec_request.command.clone(),
        cwd: exec_request.cwd.clone(),
        permission_profile: exec_request.permission_profile.clone(),
        file_system_sandbox_policy: exec_request.file_system_sandbox_policy.clone(),
        network_sandbox_policy: exec_request.network_sandbox_policy,
        sandbox: exec_request.sandbox,
        env: exec_request.env.clone(),
        network: exec_request.network.clone(),
        windows_sandbox_level: exec_request.windows_sandbox_level,
        arg0: exec_request.arg0.clone(),
        sandbox_policy_cwd: exec_request.windows_sandbox_policy_cwd.clone(),
        codex_linux_sandbox_exe: ctx.turn.codex_linux_sandbox_exe.clone(),
        use_legacy_landlock: ctx.turn.features.use_legacy_landlock(),
        sandbox_runtime: Arc::clone(&ctx.session.services.sandbox_runtime),
    };
    let escalation_policy = CoreShellActionProvider {
        policy: Arc::clone(&exec_policy),
        session: Arc::clone(&ctx.session),
        turn: Arc::clone(&ctx.turn),
        call_id: ctx.call_id.clone(),
        tool_name: GuardianCommandSource::UnifiedExec,
        approval_policy: ctx.turn.approval_policy.value(),
        permission_profile: exec_request.permission_profile.clone(),
        file_system_sandbox_policy: exec_request.file_system_sandbox_policy.clone(),
        sandbox_policy_cwd: exec_request.windows_sandbox_policy_cwd.clone(),
        sandbox_permissions: req.sandbox_permissions,
        approval_sandbox_permissions: approval_sandbox_permissions(
            req.sandbox_permissions,
            req.additional_permissions_preapproved,
        ),
        prompt_permissions: req.additional_permissions.clone(),
        stopwatch: Stopwatch::unlimited(),
    };

    let escalate_server = EscalateServer::new(
        shell_zsh_path.to_path_buf(),
        main_execve_wrapper_exe.to_path_buf(),
        escalation_policy,
    );
    let escalation_session = escalate_server
        .start_session(CancellationToken::new(), Arc::new(command_executor))
        .map_err(|err| ToolError::Rejected(err.to_string()))?;
    let mut exec_request = exec_request;
    exec_request.env.extend(escalation_session.env().clone());
    Ok(Some(PreparedUnifiedExecZshFork {
        exec_request,
        escalation_session,
    }))
}

struct CoreShellActionProvider {
    policy: Arc<RwLock<Policy>>,
    session: Arc<crate::session::session::Session>,
    turn: Arc<crate::session::turn_context::TurnContext>,
    call_id: String,
    tool_name: GuardianCommandSource,
    approval_policy: AskForApproval,
    permission_profile: PermissionProfile,
    file_system_sandbox_policy: FileSystemSandboxPolicy,
    sandbox_policy_cwd: AbsolutePathBuf,
    sandbox_permissions: SandboxPermissions,
    approval_sandbox_permissions: SandboxPermissions,
    prompt_permissions: Option<AdditionalPermissionProfile>,
    stopwatch: Stopwatch,
}

impl EscalationPromptHandler for CoreShellActionProvider {
    fn prompt<'a>(&'a self, request: EscalationPromptRequest<'a>) -> EscalationPromptFuture<'a> {
        Box::pin(async move {
            let command = join_program_and_argv(request.program, request.argv);
            let workdir = request.workdir.clone();
            let session = self.session.clone();
            let turn = self.turn.clone();
            let call_id = self.call_id.clone();
            let approval_id = Some(Uuid::new_v4().to_string());
            let source = self.tool_name;
            let additional_permissions = request.additional_permissions;
            let guardian_review_id =
                routes_approval_to_guardian(&turn).then(new_guardian_review_id);
            Ok(request
                .stopwatch
                .pause_for(async move {
                    // 1) Run PermissionRequest hooks
                    let permission_request = PermissionRequestPayload::bash(
                        codex_shell_utils::shlex_join(&command),
                        /*description*/ None,
                    );
                    let effective_approval_id =
                        approval_id.clone().unwrap_or_else(|| call_id.clone());
                    match run_permission_request_hooks(
                        session.as_ref(),
                        turn.as_ref(),
                        &effective_approval_id,
                        permission_request_hook_payload(permission_request),
                    )
                    .await
                    {
                        Some(PermissionRequestDecision::Allow) => {
                            return EscalationPromptDecision {
                                decision: ReviewDecision::Approved,
                                rejection_message: None,
                            };
                        }
                        Some(PermissionRequestDecision::Deny { message }) => {
                            return EscalationPromptDecision {
                                decision: ReviewDecision::Denied,
                                rejection_message: Some(message),
                            };
                        }
                        None => {}
                    }

                    // 2) Route to Guardian if configured
                    if let Some(review_id) = guardian_review_id.clone() {
                        let decision = review_approval_request(
                            &session,
                            &turn,
                            review_id.clone(),
                            GuardianApprovalRequest::Execve {
                                id: call_id.clone(),
                                source,
                                program: request.program.to_string_lossy().into_owned(),
                                argv: request.argv.to_vec(),
                                cwd: workdir.clone(),
                                additional_permissions,
                            },
                            /*retry_reason*/ None,
                        )
                        .await;
                        let rejection_message = if matches!(decision, ReviewDecision::Denied) {
                            Some(guardian_rejection_message(session.as_ref(), &review_id).await)
                        } else {
                            None
                        };
                        return EscalationPromptDecision {
                            decision,
                            rejection_message,
                        };
                    }

                    // 3) Fall back to regular user prompt
                    let decision = session
                        .request_command_approval(
                            &turn,
                            call_id,
                            approval_id,
                            command,
                            workdir.clone(),
                            /*reason*/ None,
                            /*network_approval_context*/ None,
                            /*proposed_execpolicy_amendment*/ None,
                            additional_permissions,
                            Some(vec![ReviewDecision::Approved, ReviewDecision::Abort]),
                        )
                        .await;
                    EscalationPromptDecision {
                        decision,
                        rejection_message: None,
                    }
                })
                .await)
        })
    }

    fn timeout_message(&self) -> String {
        guardian_timeout_message()
    }
}

// Shell-wrapper parsing is weaker than direct exec interception because it can
// only see the script text, not the final resolved executable path. Keep it
// disabled by default so path-sensitive rules rely on the later authoritative
// execve interception.
const ENABLE_INTERCEPTED_EXEC_POLICY_SHELL_WRAPPER_PARSING: bool = false;

impl EscalationPolicy for CoreShellActionProvider {
    fn determine_action<'a>(
        &'a self,
        program: &'a AbsolutePathBuf,
        argv: &'a [String],
        workdir: &'a AbsolutePathBuf,
    ) -> EscalationPolicyFuture<'a> {
        Box::pin(async move {
            let policy = self.policy.read().await;
            determine_escalation_action(
                EscalationPolicyDecisionParams {
                    policy: &policy,
                    approval_policy: self.approval_policy,
                    permission_profile: &self.permission_profile,
                    file_system_sandbox_policy: &self.file_system_sandbox_policy,
                    sandbox_policy_cwd: &self.sandbox_policy_cwd,
                    sandbox_permissions: self.sandbox_permissions,
                    approval_sandbox_permissions: self.approval_sandbox_permissions,
                    prompt_permissions: self.prompt_permissions.clone(),
                    stopwatch: &self.stopwatch,
                    enable_shell_wrapper_parsing:
                        ENABLE_INTERCEPTED_EXEC_POLICY_SHELL_WRAPPER_PARSING,
                },
                program,
                argv,
                workdir,
                self,
            )
            .await
        })
    }
}

struct CoreShellCommandExecutor {
    command: Vec<String>,
    cwd: AbsolutePathBuf,
    permission_profile: PermissionProfile,
    file_system_sandbox_policy: FileSystemSandboxPolicy,
    network_sandbox_policy: NetworkSandboxPolicy,
    sandbox: SandboxType,
    env: HashMap<String, String>,
    network: Option<SharedNetworkProxyRuntime>,
    windows_sandbox_level: WindowsSandboxLevel,
    arg0: Option<String>,
    sandbox_policy_cwd: AbsolutePathBuf,
    codex_linux_sandbox_exe: Option<PathBuf>,
    use_legacy_landlock: bool,
    sandbox_runtime: SharedSandboxRuntime,
}

struct PrepareSandboxedExecParams<'a> {
    command: Vec<String>,
    workdir: &'a AbsolutePathBuf,
    env: HashMap<String, String>,
    permission_profile: &'a PermissionProfile,
    additional_permissions: Option<AdditionalPermissionProfile>,
}

impl ShellCommandExecutor for CoreShellCommandExecutor {
    fn run<'a>(
        &'a self,
        _command: Vec<String>,
        _cwd: PathBuf,
        env_overlay: HashMap<String, String>,
        cancel_rx: CancellationToken,
        after_spawn: Option<Box<dyn FnOnce() + Send>>,
    ) -> ShellCommandRunFuture<'a> {
        Box::pin(async move {
            let mut exec_env = self.env.clone();
            // `env_overlay` comes from `EscalationSession::env()`, so merge only the
            // wrapper/socket variables into the base shell environment.
            for var in ["CODEX_ESCALATE_SOCKET", "EXEC_WRAPPER"] {
                if let Some(value) = env_overlay.get(var) {
                    exec_env.insert(var.to_string(), value.clone());
                }
            }

            let result = crate::sandboxing::execute_exec_request_with_after_spawn(
                crate::sandboxing::ExecRequest {
                    command: self.command.clone(),
                    cwd: self.cwd.clone(),
                    env: exec_env,
                    network: self.network.clone(),
                    expiration: ExecExpiration::Cancellation(cancel_rx),
                    capture_policy: ExecCapturePolicy::ShellTool,
                    sandbox: self.sandbox,
                    windows_sandbox_policy_cwd: self.sandbox_policy_cwd.clone(),
                    windows_sandbox_level: self.windows_sandbox_level,
                    windows_sandbox_private_desktop: false,
                    permission_profile: self.permission_profile.clone(),
                    file_system_sandbox_policy: self.file_system_sandbox_policy.clone(),
                    network_sandbox_policy: self.network_sandbox_policy,
                    windows_sandbox_filesystem_overrides: None,
                    arg0: self.arg0.clone(),
                },
                /*stdout_stream*/ None,
                after_spawn,
            )
            .await?;

            Ok(ExecResult {
                exit_code: result.exit_code,
                stdout: result.stdout.text,
                stderr: result.stderr.text,
                output: result.aggregated_output.text,
                duration: result.duration,
                timed_out: result.timed_out,
            })
        })
    }

    fn prepare_escalated_exec<'a>(
        &'a self,
        program: &'a AbsolutePathBuf,
        argv: &'a [String],
        workdir: &'a AbsolutePathBuf,
        env: HashMap<String, String>,
        execution: EscalationExecution,
    ) -> PrepareEscalatedExecFuture<'a> {
        Box::pin(async move {
            let command = join_program_and_argv(program, argv);
            let Some(first_arg) = argv.first() else {
                return Err(anyhow::anyhow!(
                    "intercepted exec request must contain argv[0]"
                ));
            };

            let prepared = match execution {
                EscalationExecution::Unsandboxed => PreparedExec {
                    command,
                    cwd: workdir.to_path_buf(),
                    env,
                    arg0: Some(first_arg.clone()),
                },
                EscalationExecution::TurnDefault => {
                    self.prepare_sandboxed_exec(PrepareSandboxedExecParams {
                        command,
                        workdir,
                        env,
                        permission_profile: &self.permission_profile,
                        additional_permissions: None,
                    })?
                }
                EscalationExecution::Permissions(
                    EscalationPermissions::AdditionalPermissionProfile(permission_profile),
                ) => {
                    // Merge additive permissions into the existing turn/request sandbox policy.
                    self.prepare_sandboxed_exec(PrepareSandboxedExecParams {
                        command,
                        workdir,
                        env,
                        permission_profile: &self.permission_profile,
                        additional_permissions: Some(permission_profile),
                    })?
                }
                EscalationExecution::Permissions(
                    EscalationPermissions::ResolvedPermissionProfile(permissions),
                ) => {
                    // Use a fully specified permission profile instead of merging into the turn policy.
                    self.prepare_sandboxed_exec(PrepareSandboxedExecParams {
                        command,
                        workdir,
                        env,
                        permission_profile: &permissions.permission_profile,
                        additional_permissions: None,
                    })?
                }
            };

            Ok(prepared)
        })
    }
}

impl CoreShellCommandExecutor {
    #[allow(clippy::too_many_arguments)]
    fn prepare_sandboxed_exec(
        &self,
        params: PrepareSandboxedExecParams<'_>,
    ) -> anyhow::Result<PreparedExec> {
        let PrepareSandboxedExecParams {
            command,
            workdir,
            env,
            permission_profile,
            additional_permissions,
        } = params;
        let (file_system_sandbox_policy, network_sandbox_policy) =
            permission_profile.to_runtime_permissions();
        let (program, args) = command
            .split_first()
            .ok_or_else(|| anyhow::anyhow!("prepared command must not be empty"))?;
        let sandbox = self.sandbox_runtime.select_initial(
            &file_system_sandbox_policy,
            network_sandbox_policy,
            SandboxablePreference::Auto,
            self.windows_sandbox_level,
            self.network.is_some(),
        );
        let command = SandboxCommand {
            program: program.clone().into(),
            args: args.to_vec(),
            cwd: workdir.clone(),
            env,
            additional_permissions,
        };
        let options = ExecOptions {
            expiration: ExecExpiration::DefaultTimeout,
            capture_policy: ExecCapturePolicy::ShellTool,
        };
        let network_snapshot = self
            .network
            .as_ref()
            .map(|network| network.runtime_snapshot());
        let exec_request = self.sandbox_runtime.transform(SandboxTransformRequest {
            command,
            permissions: permission_profile,
            sandbox,
            enforce_managed_network: self.network.is_some(),
            network: network_snapshot.as_ref(),
            sandbox_policy_cwd: &self.sandbox_policy_cwd,
            codex_linux_sandbox_exe: self.codex_linux_sandbox_exe.as_deref(),
            use_legacy_landlock: self.use_legacy_landlock,
            windows_sandbox_level: self.windows_sandbox_level,
            windows_sandbox_private_desktop: false,
        })?;
        let mut exec_request = crate::sandboxing::ExecRequest::from_sandbox_exec_request(
            exec_request,
            options,
            self.sandbox_policy_cwd.clone(),
            self.network.clone(),
        );
        if let Some(network) = exec_request.network.as_ref() {
            network.apply_to_env(&mut exec_request.env);
        }

        Ok(PreparedExec {
            command: exec_request.command,
            cwd: exec_request.cwd.to_path_buf(),
            env: exec_request.env,
            arg0: exec_request.arg0,
        })
    }
}

#[cfg(test)]
#[path = "shell_escalation_adapter_tests.rs"]
mod tests;
