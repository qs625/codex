use codex_command_runtime::CommandNotificationFilter;
use codex_protocol::models::AdditionalPermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_tool_config::ToolUserShellType;
use codex_tool_config::UnifiedExecShellMode;
use serde::Deserialize;

use crate::ResolvedExecCommand;
use crate::RuntimeShell;

#[derive(Debug, Deserialize)]
pub struct ExecCommandArgs {
    pub cmd: String,
    #[serde(default)]
    pub workdir: Option<String>,
    #[serde(default)]
    pub shell: Option<String>,
    #[serde(default)]
    pub login: Option<bool>,
    #[serde(default = "default_tty")]
    pub tty: bool,
    #[serde(default = "default_exec_yield_time_ms")]
    pub yield_time_ms: u64,
    #[serde(default)]
    pub initial_wait_ms: Option<u64>,
    #[serde(default)]
    pub notify_on: CommandNotifyOnArg,
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
    #[serde(default)]
    pub sandbox_permissions: SandboxPermissions,
    #[serde(default)]
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    #[serde(default)]
    pub justification: Option<String>,
    #[serde(default)]
    pub prefix_rule: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandNotifyOnArg {
    Output,
    #[default]
    Exit,
}

impl From<CommandNotifyOnArg> for CommandNotificationFilter {
    fn from(value: CommandNotifyOnArg) -> Self {
        match value {
            CommandNotifyOnArg::Output => Self::Output,
            CommandNotifyOnArg::Exit => Self::Exit,
        }
    }
}

pub fn resolve_exec_command(
    args: &ExecCommandArgs,
    session_shell: &RuntimeShell,
    model_shell: Option<&RuntimeShell>,
    shell_mode: &UnifiedExecShellMode,
    allow_login_shell: bool,
) -> Result<ResolvedExecCommand, String> {
    resolve_exec_command_for_parts(
        &args.cmd,
        args.login,
        session_shell,
        model_shell,
        shell_mode,
        allow_login_shell,
    )
}

pub fn resolve_exec_command_for_parts(
    command: &str,
    login: Option<bool>,
    session_shell: &RuntimeShell,
    model_shell: Option<&RuntimeShell>,
    shell_mode: &UnifiedExecShellMode,
    allow_login_shell: bool,
) -> Result<ResolvedExecCommand, String> {
    let use_login_shell = match login {
        Some(true) if !allow_login_shell => {
            return Err(
                "login shell is disabled by config; omit `login` or set it to false.".to_string(),
            );
        }
        Some(use_login_shell) => use_login_shell,
        None => allow_login_shell,
    };

    match shell_mode {
        UnifiedExecShellMode::Direct => {
            let shell = model_shell.unwrap_or(session_shell);
            Ok(ResolvedExecCommand {
                command: derive_exec_args(shell, command, use_login_shell),
                shell_type: shell.shell_type,
            })
        }
        UnifiedExecShellMode::ZshFork(zsh_fork_config) => Ok(ResolvedExecCommand {
            command: vec![
                zsh_fork_config.shell_zsh_path.to_string_lossy().to_string(),
                if use_login_shell { "-lc" } else { "-c" }.to_string(),
                command.to_string(),
            ],
            shell_type: ToolUserShellType::Zsh,
        }),
    }
}

fn derive_exec_args(shell: &RuntimeShell, command: &str, use_login_shell: bool) -> Vec<String> {
    match shell.shell_type {
        ToolUserShellType::Zsh | ToolUserShellType::Bash | ToolUserShellType::Sh => {
            let arg = if use_login_shell { "-lc" } else { "-c" };
            vec![
                shell.shell_path.to_string_lossy().to_string(),
                arg.to_string(),
                command.to_string(),
            ]
        }
        ToolUserShellType::PowerShell => {
            let mut args = vec![shell.shell_path.to_string_lossy().to_string()];
            if !use_login_shell {
                args.push("-NoProfile".to_string());
            }
            args.push("-Command".to_string());
            args.push(command.to_string());
            args
        }
        ToolUserShellType::Cmd => vec![
            shell.shell_path.to_string_lossy().to_string(),
            "/c".to_string(),
            command.to_string(),
        ],
    }
}

fn default_exec_yield_time_ms() -> u64 {
    10_000
}

fn default_tty() -> bool {
    false
}
