use crate::tool_runtime_support::SandboxAttempt;
use crate::tool_runtime_support::ToolCtx;
use crate::tool_runtime_support::ToolError;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_tool_runtime_api::ShellRequest;

/// Runs the zsh-fork shell-command backend when this request should be handled
/// by executable-level escalation instead of the default shell runtime.
///
/// Returns `Ok(None)` when the current platform or request shape should fall
/// back to the normal shell-command path.
pub(crate) async fn maybe_run_shell_command(
    req: &ShellRequest,
    attempt: &SandboxAttempt<'_>,
    ctx: &ToolCtx,
    command: &[String],
) -> Result<Option<ExecToolCallOutput>, ToolError> {
    imp::maybe_run_shell_command(req, attempt, ctx, command).await
}

#[cfg(unix)]
mod imp {
    use super::*;
    use crate::shell_escalation_adapter;

    pub(super) async fn maybe_run_shell_command(
        req: &ShellRequest,
        attempt: &SandboxAttempt<'_>,
        ctx: &ToolCtx,
        command: &[String],
    ) -> Result<Option<ExecToolCallOutput>, ToolError> {
        shell_escalation_adapter::try_run_zsh_fork(req, attempt, ctx, command).await
    }
}

#[cfg(not(unix))]
mod imp {
    use super::*;

    pub(super) async fn maybe_run_shell_command(
        req: &ShellRequest,
        attempt: &SandboxAttempt<'_>,
        ctx: &ToolCtx,
        command: &[String],
    ) -> Result<Option<ExecToolCallOutput>, ToolError> {
        let _ = (req, attempt, ctx, command);
        Ok(None)
    }
}
