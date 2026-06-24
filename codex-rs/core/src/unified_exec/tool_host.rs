use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::shell::Shell;
use crate::shell::get_shell_by_model_provided_path;
use crate::tools::context::SharedTurnDiffTracker;
use crate::tools::handlers::CoreToolDomainHost;
use crate::tools::handlers::resolve_tool_environment;
use crate::tools::runtimes::CoreApplyPatchEnvironment;
use codex_command_runtime::CommandSessionError;
use codex_command_runtime::CommandWaitOperation;
use codex_command_runtime::CommandWaitRequest;
use codex_command_runtime::WriteStdinOutput;
use codex_command_runtime::WriteStdinRequest;
use codex_metrics_api::TOOL_CALL_UNIFIED_EXEC_METRIC;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TerminalInteractionEvent;
use codex_tool_config::UnifiedExecShellMode;
use codex_tool_runtime_api::CommandInteractionHost;
use codex_tool_runtime_api::ExecCommandArgs;
use codex_tool_runtime_api::ExecCommandHandlerHost;
use codex_tool_runtime_api::ExecCommandRunOutput;
use codex_tool_runtime_api::ExecCommandRunRequest;
use codex_tool_runtime_api::ResolvedExecCommand;
use codex_tool_runtime_api::ResolvedExecCommandEnvironment;
use codex_tool_runtime_api::RuntimeShell;
use std::path::PathBuf;
use std::sync::Arc;

impl CommandInteractionHost for CoreToolDomainHost {
    type Session = Arc<Session>;
    type Turn = Arc<TurnContext>;
    type Tracker = SharedTurnDiffTracker;
    type DiffContext = TurnContext;

    fn new_response_item_id(&self) -> String {
        format!("response-item-{}", uuid::Uuid::new_v4())
    }

    async fn begin_command_wait(
        &self,
        session: &Arc<Session>,
        request: CommandWaitRequest,
    ) -> Result<Box<dyn CommandWaitOperation>, CommandSessionError> {
        session
            .services
            .command_session_controller
            .begin_command_wait(request)
            .await
    }

    async fn write_command_stdin(
        &self,
        session: &Arc<Session>,
        request: WriteStdinRequest<'_>,
    ) -> Result<WriteStdinOutput, CommandSessionError> {
        session
            .services
            .command_session_controller
            .write_command_stdin(request)
            .await
    }

    async fn emit_model_item_started_display_event(
        &self,
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
        item: &ResponseItem,
    ) {
        session
            .emit_model_item_started_display_event(turn.as_ref(), item)
            .await;
    }

    async fn record_model_items_and_emit_display_events(
        &self,
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
        items: &[ResponseItem],
    ) {
        session
            .record_model_items_and_emit_display_events(turn.as_ref(), items)
            .await;
    }

    async fn send_terminal_interaction(
        &self,
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
        event: TerminalInteractionEvent,
    ) {
        session
            .send_event(turn.as_ref(), EventMsg::TerminalInteraction(event))
            .await;
    }
}

#[derive(Debug)]
pub(crate) struct ResolvedCommand {
    pub(crate) command: Vec<String>,
}

pub(crate) fn get_command(
    args: &ExecCommandArgs,
    session_shell: Arc<Shell>,
    shell_mode: &UnifiedExecShellMode,
    allow_login_shell: bool,
) -> Result<ResolvedCommand, String> {
    let session_shell = crate::tools::runtimes::runtime_shell(session_shell.as_ref());
    let model_shell = args
        .shell
        .as_ref()
        .map(|shell_str| runtime_shell_from_model_path(&PathBuf::from(shell_str)));
    let resolved = codex_tool_runtime_api::resolve_exec_command(
        args,
        &session_shell,
        model_shell.as_ref(),
        shell_mode,
        allow_login_shell,
    )?;
    Ok(ResolvedCommand {
        command: resolved.command,
    })
}

impl ExecCommandHandlerHost for CoreToolDomainHost {
    fn resolve_exec_command_environment(
        &self,
        turn: &Arc<TurnContext>,
        environment_id: Option<&str>,
        workdir: Option<&str>,
    ) -> Result<Option<ResolvedExecCommandEnvironment>, crate::function_tool::FunctionCallError>
    {
        let Some(turn_environment) = resolve_tool_environment(turn.as_ref(), environment_id)?
        else {
            return Ok(None);
        };
        let cwd = workdir.filter(|workdir| !workdir.is_empty()).map_or_else(
            || turn_environment.cwd.clone(),
            |workdir| turn_environment.cwd.join(workdir),
        );
        Ok(Some(ResolvedExecCommandEnvironment {
            cwd,
            sandbox_cwd: turn_environment.cwd.clone(),
            environment: turn_environment.environment.clone(),
            apply_patch_environment: CoreApplyPatchEnvironment::new(turn_environment.clone()),
        }))
    }

    fn resolve_model_shell(&self, shell: &std::path::Path) -> RuntimeShell {
        runtime_shell_from_model_path(&shell.to_path_buf())
    }

    fn resolve_exec_command(
        &self,
        command: &str,
        login: Option<bool>,
        model_shell: Option<&RuntimeShell>,
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
    ) -> Result<ResolvedExecCommand, String> {
        codex_tool_runtime_api::resolve_exec_command_for_parts(
            command,
            login,
            &crate::tools::runtimes::runtime_shell(session.user_shell().as_ref()),
            model_shell,
            &turn.tools_config.unified_exec_shell_mode,
            turn.tools_config.allow_login_shell,
        )
    }

    async fn maybe_emit_implicit_skill_invocation(
        &self,
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
        command: &str,
        workdir: &codex_utils_absolute_path::AbsolutePathBuf,
    ) {
        crate::maybe_emit_implicit_skill_invocation(
            session.as_ref(),
            turn.as_ref(),
            command,
            workdir,
        )
        .await;
    }

    async fn allocate_exec_process_id(&self, session: &Arc<Session>) -> i32 {
        session
            .services
            .unified_exec_manager
            .allocate_process_id()
            .await
    }

    async fn release_exec_process_id(&self, session: &Arc<Session>, process_id: i32) {
        session
            .services
            .unified_exec_manager
            .release_process_id(process_id)
            .await;
    }

    async fn run_exec_command(
        &self,
        session: &Arc<Session>,
        turn: &Arc<TurnContext>,
        call_id: &str,
        request: ExecCommandRunRequest,
    ) -> Result<ExecCommandRunOutput, crate::unified_exec::UnifiedExecError> {
        let context = crate::unified_exec::UnifiedExecContext::new(
            session.clone(),
            turn.clone(),
            call_id.to_string(),
        );
        let response = session
            .services
            .unified_exec_manager
            .exec_command(
                crate::unified_exec::ExecCommandRequest {
                    command: request.command,
                    shell_type: core_shell_type(&request.shell_type),
                    hook_command: request.hook_command,
                    process_id: request.process_id,
                    yield_time_ms: request.yield_time_ms,
                    max_output_tokens: request.max_output_tokens,
                    cwd: request.cwd,
                    sandbox_cwd: request.sandbox_cwd,
                    environment: request.environment,
                    network: turn.network.clone(),
                    tty: request.tty,
                    sandbox_permissions: request.sandbox_permissions,
                    additional_permissions: request.additional_permissions,
                    additional_permissions_preapproved: request.additional_permissions_preapproved,
                    justification: request.justification,
                    prefix_rule: request.prefix_rule,
                    notify_on: request.notify_on,
                },
                &context,
            )
            .await?;
        Ok(ExecCommandRunOutput {
            event_call_id: response.event_call_id,
            chunk_id: response.chunk_id,
            wall_time: response.wall_time,
            raw_output: response.raw_output,
            max_output_tokens: response.max_output_tokens,
            process_id: response.process_id,
            exit_code: response.exit_code,
            original_token_count: response.original_token_count,
            hook_command: response.hook_command,
        })
    }

    fn emit_unified_exec_tty_metric(&self, turn: &Arc<TurnContext>, tty: bool) {
        turn.session_telemetry.counter(
            TOOL_CALL_UNIFIED_EXEC_METRIC,
            /*inc*/ 1,
            &[("tty", if tty { "true" } else { "false" })],
        );
    }
}

fn runtime_shell_from_model_path(shell_path: &PathBuf) -> RuntimeShell {
    let mut shell = get_shell_by_model_provided_path(shell_path);
    shell.shell_snapshot = crate::shell::empty_shell_snapshot_receiver();
    crate::tools::runtimes::runtime_shell(&shell)
}

fn core_shell_type(shell_type: &codex_tool_config::ToolUserShellType) -> crate::shell::ShellType {
    match shell_type {
        codex_tool_config::ToolUserShellType::Zsh => crate::shell::ShellType::Zsh,
        codex_tool_config::ToolUserShellType::Bash => crate::shell::ShellType::Bash,
        codex_tool_config::ToolUserShellType::PowerShell => crate::shell::ShellType::PowerShell,
        codex_tool_config::ToolUserShellType::Sh => crate::shell::ShellType::Sh,
        codex_tool_config::ToolUserShellType::Cmd => crate::shell::ShellType::Cmd,
    }
}
