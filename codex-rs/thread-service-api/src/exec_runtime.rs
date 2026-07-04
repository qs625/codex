pub use permissions_service_api::ExecApprovalRequirement;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::error::CodexErr;
use protocol::models::AdditionalPermissionProfile;
use protocol::models::SandboxPermissions;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use tokio_util::sync::CancellationToken;
use tool_config::ToolUserShellType;
use tool_config::UnifiedExecShellMode;

#[derive(Clone, Debug)]
pub struct RuntimeShellSnapshot {
    pub path: AbsolutePathBuf,
    pub cwd: AbsolutePathBuf,
}

#[derive(Clone, Debug)]
pub struct RuntimeShell {
    pub shell_type: ToolUserShellType,
    pub shell_path: PathBuf,
    pub shell_snapshot: Option<RuntimeShellSnapshot>,
}

#[derive(Clone, Debug)]
pub struct ResolvedExecCommand {
    pub command: Vec<String>,
    pub shell_type: ToolUserShellType,
}

#[derive(serde::Serialize, Clone, Debug, Eq, PartialEq, Hash)]
pub struct UnifiedExecApprovalKey {
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub tty: bool,
    pub sandbox_permissions: SandboxPermissions,
    pub additional_permissions: Option<AdditionalPermissionProfile>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRuntimeNetworkApprovalTrigger {
    pub call_id: String,
    pub tool_name: String,
    pub command: Vec<String>,
    pub cwd: AbsolutePathBuf,
    pub sandbox_permissions: SandboxPermissions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub justification: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tty: Option<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkApprovalMode {
    Immediate,
    Deferred,
}

#[derive(Clone, Debug)]
pub struct NetworkApprovalSpec<Trigger> {
    pub network: Option<codex_network_proxy_api::SharedNetworkProxyRuntime>,
    pub mode: NetworkApprovalMode,
    pub trigger: Trigger,
    pub command: String,
}

#[derive(Debug)]
pub enum ToolRuntimeNetworkApprovalError {
    Rejected(String),
    Codex(CodexErr),
}

pub trait ToolRuntimeNetworkApprovalHandle: Send + Sync + 'static {
    fn mode(&self) -> NetworkApprovalMode;

    fn registration_id(&self) -> Option<String>;

    fn cancellation_token(&self) -> CancellationToken;

    fn finish<'a>(
        &'a self,
    ) -> Pin<Box<dyn Future<Output = Result<(), ToolRuntimeNetworkApprovalError>> + Send + 'a>>;
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
