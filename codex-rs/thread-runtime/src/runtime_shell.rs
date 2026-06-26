use std::collections::HashMap;

use crate::runtime_shell_model::Shell;
use crate::runtime_shell_model::ShellType;
use codex_tool_config::ToolUserShellType;
use codex_tool_runtime_api::RuntimeShell;
use codex_tool_runtime_api::RuntimeShellSnapshot;
use codex_utils_absolute_path::AbsolutePathBuf;

pub(crate) fn maybe_wrap_shell_lc_with_snapshot(
    command: &[String],
    session_shell: &Shell,
    cwd: &AbsolutePathBuf,
    explicit_env_overrides: &HashMap<String, String>,
    env: &HashMap<String, String>,
) -> Vec<String> {
    codex_tool_runtime::maybe_wrap_shell_lc_with_snapshot(
        command,
        &runtime_shell(session_shell),
        cwd,
        explicit_env_overrides,
        env,
    )
}

pub(crate) fn runtime_shell(session_shell: &Shell) -> RuntimeShell {
    RuntimeShell {
        shell_type: runtime_shell_type(&session_shell.shell_type),
        shell_path: session_shell.shell_path.clone(),
        shell_snapshot: session_shell
            .shell_snapshot()
            .map(|snapshot| RuntimeShellSnapshot {
                path: snapshot.path.clone(),
                cwd: snapshot.cwd.clone(),
            }),
    }
}

pub(crate) fn runtime_shell_type(shell_type: &ShellType) -> ToolUserShellType {
    match shell_type {
        ShellType::Zsh => ToolUserShellType::Zsh,
        ShellType::Bash => ToolUserShellType::Bash,
        ShellType::PowerShell => ToolUserShellType::PowerShell,
        ShellType::Sh => ToolUserShellType::Sh,
        ShellType::Cmd => ToolUserShellType::Cmd,
    }
}
