use crate::sandboxing::ExecRequest;
use crate::tool_runtime_support::SandboxAttempt;
use crate::tool_runtime_support::ToolCtx;
use crate::tool_runtime_support::ToolError;
use crate::unified_exec::SpawnLifecycle;
use codex_shell_escalation::ESCALATE_SOCKET_ENV_VAR;
use codex_shell_escalation::EscalationSession;
use codex_tool_config::ZshForkConfig;
use codex_tool_runtime_api::PreparedUnifiedExecSpawn;
use codex_tool_runtime_api::UnifiedExecRequest;

#[derive(Debug)]
struct ZshForkSpawnLifecycle {
    escalation_session: EscalationSession,
}

impl SpawnLifecycle for ZshForkSpawnLifecycle {
    fn inherited_fds(&self) -> Vec<i32> {
        self.escalation_session
            .env()
            .get(ESCALATE_SOCKET_ENV_VAR)
            .and_then(|fd| fd.parse().ok())
            .into_iter()
            .collect()
    }

    fn after_spawn(&mut self) {
        self.escalation_session.close_client_socket();
    }
}

#[cfg(unix)]
pub(crate) async fn maybe_prepare_unified_exec(
    req: &UnifiedExecRequest,
    attempt: &SandboxAttempt<'_>,
    ctx: &ToolCtx,
    exec_request: ExecRequest,
    zsh_fork_config: &ZshForkConfig,
) -> Result<Option<PreparedUnifiedExecSpawn<ExecRequest>>, ToolError> {
    let Some(prepared) = crate::shell_escalation_adapter::prepare_unified_exec_zsh_fork(
        req,
        attempt,
        ctx,
        exec_request,
        zsh_fork_config.shell_zsh_path.as_path(),
        zsh_fork_config.main_execve_wrapper_exe.as_path(),
    )
    .await?
    else {
        return Ok(None);
    };

    Ok(Some(PreparedUnifiedExecSpawn {
        exec_request: prepared.exec_request,
        spawn_lifecycle: Box::new(ZshForkSpawnLifecycle {
            escalation_session: prepared.escalation_session,
        }),
    }))
}

#[cfg(not(unix))]
pub(crate) async fn maybe_prepare_unified_exec(
    req: &UnifiedExecRequest,
    attempt: &SandboxAttempt<'_>,
    ctx: &ToolCtx,
    exec_request: ExecRequest,
    zsh_fork_config: &ZshForkConfig,
) -> Result<Option<PreparedUnifiedExecSpawn<ExecRequest>>, ToolError> {
    let _ = (req, attempt, ctx, exec_request, zsh_fork_config);
    Ok(None)
}
