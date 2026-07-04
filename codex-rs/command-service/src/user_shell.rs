use command_service_api::ExecCapturePolicy;
use command_service_api::UserShellRunRequest;
use codex_network_proxy_api::PROXY_ACTIVE_ENV_KEY;
use codex_network_proxy_api::PROXY_ENV_KEYS;
use protocol::error::Result;
use protocol::exec_output::ExecToolCallOutput;
use protocol::models::PermissionProfile;
use protocol::protocol::Event;
use tool_config::UnifiedExecShellMode;
#[cfg(target_os = "macos")]
use codex_network_proxy_api::CODEX_PROXY_GIT_SSH_COMMAND_MARKER;
#[cfg(target_os = "macos")]
use codex_network_proxy_api::PROXY_GIT_SSH_COMMAND_ENV_KEY;
use codex_sandboxing_api::SandboxType;

use crate::ExecRequest;
use crate::StdoutStream;
use crate::create_env;
use crate::execute_exec_request;
use crate::maybe_wrap_shell_lc_with_snapshot;
use async_channel::Sender;

pub async fn run_user_shell_command(request: UserShellRunRequest) -> Result<ExecToolCallOutput> {
    let UserShellRunRequest {
        command,
        call_id,
        turn_id,
        thread_id,
        cwd,
        session_shell,
        shell_environment_policy,
        shell_env_overrides,
        windows_sandbox_level,
        windows_sandbox_private_desktop,
        timeout_ms,
        tx_event,
    } = request;

    let display_command = command_service_api::resolve_exec_command_for_parts(
        &command,
        Some(true),
        &session_shell,
        /*model_shell*/ None,
        &UnifiedExecShellMode::Direct,
        /*allow_login_shell*/ true,
    )
    .map_err(protocol::error::CodexErr::InvalidRequest)?
    .command;

    let mut exec_env_map = create_env(&shell_environment_policy, Some(thread_id));
    if exec_env_map.contains_key(PROXY_ACTIVE_ENV_KEY) {
        for key in PROXY_ENV_KEYS {
            exec_env_map.remove(*key);
        }
        #[cfg(target_os = "macos")]
        if exec_env_map
            .get(PROXY_GIT_SSH_COMMAND_ENV_KEY)
            .is_some_and(|value| value.starts_with(CODEX_PROXY_GIT_SSH_COMMAND_MARKER))
        {
            exec_env_map.remove(PROXY_GIT_SSH_COMMAND_ENV_KEY);
        }
    }

    let exec_command = maybe_wrap_shell_lc_with_snapshot(
        &display_command,
        &session_shell,
        &cwd,
        &shell_env_overrides,
        &exec_env_map,
    );

    let exec_env = ExecRequest::new(
        exec_command,
        cwd,
        exec_env_map,
        /*network*/ None,
        timeout_ms.into(),
        ExecCapturePolicy::ShellTool,
        SandboxType::None,
        windows_sandbox_level,
        windows_sandbox_private_desktop,
        PermissionProfile::Disabled,
        /*arg0*/ None,
    );

    let stdout_stream = Some(build_stdout_stream(turn_id, call_id, tx_event));
    execute_exec_request(exec_env, stdout_stream, /*after_spawn*/ None).await
}

fn build_stdout_stream(turn_id: String, call_id: String, tx_event: Sender<Event>) -> StdoutStream {
    StdoutStream {
        sub_id: turn_id,
        call_id,
        tx_event,
    }
}
