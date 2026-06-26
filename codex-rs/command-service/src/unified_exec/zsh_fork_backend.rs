use crate::exec_request::ExecRequest;
use crate::runtime_support::SandboxAttempt;
use crate::runtime_support::ToolCtx;
use crate::runtime_support::ToolError;
use codex_tool_config::ZshForkConfig;
use codex_tool_runtime_api::PreparedUnifiedExecSpawn;
use codex_tool_runtime_api::UnifiedExecRequest;

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
