use std::io;
#[cfg(target_os = "windows")]
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use async_channel::Sender;

use crate::sandboxing::ExecOptions;
use crate::sandboxing::ExecRequest;
use crate::sandboxing::SandboxPermissions;
use crate::spawn::SpawnChildRequest;
use crate::spawn::StdioPolicy;
use crate::spawn::spawn_child_async;
pub use codex_command_service_api::DEFAULT_EXEC_COMMAND_TIMEOUT_MS;
pub use codex_command_service_api::ExecCapturePolicy;
pub use codex_command_service_api::ExecExpiration;
pub use codex_command_service_api::ExecExpirationOutcome;
pub use codex_command_service_api::IO_DRAIN_TIMEOUT_MS;
pub use codex_command_service_api::MAX_EXEC_OUTPUT_DELTAS_PER_CALL;
pub use codex_command_service_api::cancel_when_either;
use codex_process_exec::CapturedProcessOutput as RawExecToolCallOutput;
#[cfg(target_os = "windows")]
use codex_process_exec::CapturedStreamOutput;
pub use codex_process_exec::ExecParams;
use codex_process_exec::ProcessOutputChunk;
use codex_process_exec::ProcessOutputSender;
use codex_process_exec::ProcessOutputStream;
#[cfg(target_os = "windows")]
use codex_process_exec::aggregate_output;
use codex_process_exec::consume_process_output;
use codex_process_exec::finalize_captured_process_output;
#[cfg(target_os = "windows")]
use codex_process_exec::synthetic_exit_status;
use codex_protocol::error::CodexErr;
use codex_protocol::error::Result;
use codex_protocol::exec_output::ExecToolCallOutput;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::ExecCommandOutputDeltaEvent;
use codex_protocol::protocol::ExecOutputStream;
use codex_protocol::protocol::SandboxPolicy;
use codex_sandboxing_api::SandboxCommand;
use codex_sandboxing_api::SandboxRuntime;
use codex_sandboxing_api::SandboxTransformRequest;
use codex_sandboxing_api::SandboxType;
use codex_sandboxing_api::SandboxablePreference;
use codex_sandboxing_api::WindowsSandboxFilesystemOverrides;
use codex_sandboxing_api::resolve_windows_elevated_filesystem_overrides;
use codex_sandboxing_api::resolve_windows_restricted_token_filesystem_overrides;
use codex_sandboxing_api::windows_sandbox_uses_elevated_backend;
use codex_utils_absolute_path::AbsolutePathBuf;
use tokio::task::JoinHandle;

fn select_process_exec_tool_sandbox_type(
    sandbox_runtime: &dyn SandboxRuntime,
    file_system_sandbox_policy: &FileSystemSandboxPolicy,
    network_sandbox_policy: NetworkSandboxPolicy,
    windows_sandbox_level: codex_protocol::config_types::WindowsSandboxLevel,
    enforce_managed_network: bool,
) -> SandboxType {
    sandbox_runtime.select_initial(
        file_system_sandbox_policy,
        network_sandbox_policy,
        SandboxablePreference::Auto,
        windows_sandbox_level,
        enforce_managed_network,
    )
}

#[derive(Clone)]
pub struct StdoutStream {
    pub sub_id: String,
    pub call_id: String,
    pub tx_event: Sender<Event>,
}

#[allow(clippy::too_many_arguments)]
pub async fn process_exec_tool_call(
    params: ExecParams,
    permission_profile: &PermissionProfile,
    sandbox_cwd: &AbsolutePathBuf,
    codex_linux_sandbox_exe: &Option<PathBuf>,
    use_legacy_landlock: bool,
    sandbox_runtime: &dyn SandboxRuntime,
    stdout_stream: Option<StdoutStream>,
) -> Result<ExecToolCallOutput> {
    let exec_req = build_exec_request(
        params,
        permission_profile,
        sandbox_cwd,
        codex_linux_sandbox_exe,
        use_legacy_landlock,
        sandbox_runtime,
    )?;

    // Route through the sandboxing module for a single, unified execution path.
    crate::sandboxing::execute_env(exec_req, stdout_stream).await
}

/// Transform a portable exec request into the concrete argv/env that should be
/// spawned under the requested sandbox policy.
pub fn build_exec_request(
    params: ExecParams,
    permission_profile: &PermissionProfile,
    sandbox_cwd: &AbsolutePathBuf,
    codex_linux_sandbox_exe: &Option<PathBuf>,
    use_legacy_landlock: bool,
    sandbox_runtime: &dyn SandboxRuntime,
) -> Result<ExecRequest> {
    let ExecParams {
        command,
        cwd,
        mut env,
        expiration,
        capture_policy,
        network,
        windows_sandbox_level,
        windows_sandbox_private_desktop,

        // TODO: Should arg0 be set on the ExecRequest that is returned?
        arg0: _,
        // These fields are related to approvals, so can be ignored here.
        justification: _,
        sandbox_permissions: _,
    } = params;

    let enforce_managed_network = network.is_some();
    let (file_system_sandbox_policy, network_sandbox_policy) =
        permission_profile.to_runtime_permissions();
    let sandbox_type = select_process_exec_tool_sandbox_type(
        sandbox_runtime,
        &file_system_sandbox_policy,
        network_sandbox_policy,
        windows_sandbox_level,
        enforce_managed_network,
    );
    tracing::debug!("Sandbox type: {sandbox_type:?}");

    if let Some(network) = network.as_ref() {
        network.apply_to_env(&mut env);
    }
    let network_snapshot = network.as_ref().map(|network| network.runtime_snapshot());
    let (program, args) = command.split_first().ok_or_else(|| {
        CodexErr::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command args are empty",
        ))
    })?;

    let command = SandboxCommand {
        program: program.clone().into(),
        args: args.to_vec(),
        cwd,
        env,
        additional_permissions: None,
    };
    let options = ExecOptions {
        expiration,
        capture_policy,
    };
    let mut exec_req = sandbox_runtime
        .transform(SandboxTransformRequest {
            command,
            permissions: permission_profile,
            sandbox: sandbox_type,
            enforce_managed_network,
            network: network_snapshot.as_ref(),
            sandbox_policy_cwd: sandbox_cwd,
            codex_linux_sandbox_exe: codex_linux_sandbox_exe.as_deref(),
            use_legacy_landlock,
            windows_sandbox_level,
            windows_sandbox_private_desktop,
        })
        .map(|request| {
            let windows_sandbox_policy_cwd = AbsolutePathBuf::try_from(sandbox_cwd.to_path_buf())
                .unwrap_or_else(|_| request.cwd.clone());
            ExecRequest::from_sandbox_exec_request(
                request,
                options,
                windows_sandbox_policy_cwd,
                network.clone(),
            )
        })
        .map_err(CodexErr::from)?;
    let use_windows_elevated_backend = windows_sandbox_uses_elevated_backend(
        exec_req.windows_sandbox_level,
        exec_req.network.is_some(),
    );
    let sandbox_policy = exec_req.compatibility_sandbox_policy();
    exec_req.windows_sandbox_filesystem_overrides = if use_windows_elevated_backend {
        resolve_windows_elevated_filesystem_overrides(
            exec_req.sandbox,
            &sandbox_policy,
            &exec_req.file_system_sandbox_policy,
            exec_req.network_sandbox_policy,
            sandbox_cwd,
            use_windows_elevated_backend,
        )
    } else {
        resolve_windows_restricted_token_filesystem_overrides(
            exec_req.sandbox,
            &sandbox_policy,
            &exec_req.file_system_sandbox_policy,
            exec_req.network_sandbox_policy,
            sandbox_cwd,
            exec_req.windows_sandbox_level,
        )
    }
    .map_err(CodexErr::UnsupportedOperation)?;
    Ok(exec_req)
}

pub(crate) async fn execute_exec_request(
    exec_request: ExecRequest,
    stdout_stream: Option<StdoutStream>,
    after_spawn: Option<Box<dyn FnOnce() + Send>>,
) -> Result<ExecToolCallOutput> {
    let sandbox_policy = exec_request.compatibility_sandbox_policy();
    let ExecRequest {
        command,
        cwd,
        env,
        network,
        expiration,
        capture_policy,
        sandbox,
        windows_sandbox_policy_cwd: _,
        windows_sandbox_level,
        windows_sandbox_private_desktop,
        permission_profile: _,
        file_system_sandbox_policy: _,
        network_sandbox_policy,
        windows_sandbox_filesystem_overrides,
        arg0,
    } = exec_request;

    let params = ExecParams {
        command,
        cwd,
        expiration,
        capture_policy,
        env,
        network: network.clone(),
        sandbox_permissions: SandboxPermissions::UseDefault,
        windows_sandbox_level,
        windows_sandbox_private_desktop,
        justification: None,
        arg0,
    };

    let start = Instant::now();
    let raw_output_result = get_raw_output_result(
        params,
        network_sandbox_policy,
        stdout_stream,
        after_spawn,
        sandbox,
        &sandbox_policy,
        windows_sandbox_filesystem_overrides.as_ref(),
    )
    .await;
    let duration = start.elapsed();
    finalize_captured_process_output(raw_output_result, sandbox, duration)
}

async fn get_raw_output_result(
    params: ExecParams,
    network_sandbox_policy: NetworkSandboxPolicy,
    stdout_stream: Option<StdoutStream>,
    after_spawn: Option<Box<dyn FnOnce() + Send>>,
    #[cfg_attr(not(windows), allow(unused_variables))] sandbox: SandboxType,
    #[cfg_attr(not(windows), allow(unused_variables))] sandbox_policy: &SandboxPolicy,
    #[cfg_attr(not(windows), allow(unused_variables))] windows_sandbox_filesystem_overrides: Option<
        &WindowsSandboxFilesystemOverrides,
    >,
) -> Result<RawExecToolCallOutput> {
    #[cfg(target_os = "windows")]
    if sandbox == SandboxType::WindowsRestrictedToken {
        return exec_windows_sandbox(params, sandbox_policy, windows_sandbox_filesystem_overrides)
            .await;
    }

    exec(params, network_sandbox_policy, stdout_stream, after_spawn).await
}

#[cfg(target_os = "windows")]
fn extract_create_process_as_user_error_code(err: &str) -> Option<String> {
    let marker = "CreateProcessAsUserW failed: ";
    let start = err.find(marker)? + marker.len();
    let tail = &err[start..];
    let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

#[cfg(target_os = "windows")]
fn windowsapps_path_kind(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.contains("\\program files\\windowsapps\\") {
        return "windowsapps_package";
    }
    if lower.contains("\\appdata\\local\\microsoft\\windowsapps\\") {
        return "windowsapps_alias";
    }
    if lower.contains("\\windowsapps\\") {
        return "windowsapps_other";
    }
    "other"
}

#[cfg(target_os = "windows")]
fn record_windows_sandbox_spawn_failure(
    command_path: Option<&str>,
    windows_sandbox_level: codex_protocol::config_types::WindowsSandboxLevel,
    err: &str,
) {
    let Some(error_code) = extract_create_process_as_user_error_code(err) else {
        return;
    };
    let path = command_path.unwrap_or("unknown");
    let exe = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_ascii_lowercase();
    let path_kind = windowsapps_path_kind(path);
    let level = if matches!(
        windows_sandbox_level,
        codex_protocol::config_types::WindowsSandboxLevel::Elevated
    ) {
        "elevated"
    } else {
        "legacy"
    };
    codex_metrics_api::record_global_counter(
        "codex.windows_sandbox.createprocessasuserw_failed",
        /*inc*/ 1,
        &[
            ("error_code", error_code.as_str()),
            ("path_kind", path_kind),
            ("exe", exe.as_str()),
            ("level", level),
        ],
    );
}

#[cfg(target_os = "windows")]
async fn exec_windows_sandbox(
    params: ExecParams,
    sandbox_policy: &SandboxPolicy,
    windows_sandbox_filesystem_overrides: Option<&WindowsSandboxFilesystemOverrides>,
) -> Result<RawExecToolCallOutput> {
    use crate::config::find_codex_home;
    use codex_windows_sandbox::run_windows_sandbox_capture_elevated;
    use codex_windows_sandbox::run_windows_sandbox_capture_with_filesystem_overrides;

    let ExecParams {
        command,
        cwd,
        mut env,
        network,
        expiration,
        capture_policy,
        windows_sandbox_level,
        windows_sandbox_private_desktop,
        ..
    } = params;
    if let Some(network) = network.as_ref() {
        network.apply_to_env(&mut env);
    }

    // TODO(iceweasel-oai): run_windows_sandbox_capture should support all
    // variants of ExecExpiration, not just timeout.
    let timeout_ms = if capture_policy.uses_expiration() {
        expiration.timeout_ms()
    } else {
        None
    };

    let policy_str = serde_json::to_string(sandbox_policy).map_err(|err| {
        CodexErr::Io(io::Error::other(format!(
            "failed to serialize Windows sandbox policy: {err}"
        )))
    })?;
    let sandbox_cwd = cwd.clone();
    let codex_home = find_codex_home().map_err(|err| {
        CodexErr::Io(io::Error::other(format!(
            "windows sandbox: failed to resolve codex_home: {err}"
        )))
    })?;
    let command_path = command.first().cloned();
    let sandbox_level = windows_sandbox_level;
    let proxy_enforced = network.is_some();
    let use_elevated = windows_sandbox_uses_elevated_backend(sandbox_level, proxy_enforced);
    let additional_deny_write_paths = windows_sandbox_filesystem_overrides
        .map(|overrides| overrides.additional_deny_write_paths.clone())
        .unwrap_or_default();
    let additional_deny_read_paths = windows_sandbox_filesystem_overrides
        .map(|overrides| overrides.additional_deny_read_paths.clone())
        .unwrap_or_default();
    let elevated_read_roots_override = windows_sandbox_filesystem_overrides
        .and_then(|overrides| overrides.read_roots_override.clone());
    let elevated_read_roots_include_platform_defaults = windows_sandbox_filesystem_overrides
        .is_some_and(|overrides| overrides.read_roots_include_platform_defaults);
    let elevated_write_roots_override = windows_sandbox_filesystem_overrides
        .and_then(|overrides| overrides.write_roots_override.clone());
    let spawn_res = tokio::task::spawn_blocking(move || {
        if use_elevated {
            run_windows_sandbox_capture_elevated(
                codex_windows_sandbox::ElevatedSandboxCaptureRequest {
                    policy_json_or_preset: policy_str.as_str(),
                    sandbox_policy_cwd: &sandbox_cwd,
                    codex_home: codex_home.as_ref(),
                    command,
                    cwd: &cwd,
                    env_map: env,
                    timeout_ms,
                    use_private_desktop: windows_sandbox_private_desktop,
                    proxy_enforced,
                    read_roots_override: elevated_read_roots_override.as_deref(),
                    read_roots_include_platform_defaults:
                        elevated_read_roots_include_platform_defaults,
                    write_roots_override: elevated_write_roots_override.as_deref(),
                    deny_read_paths_override: &additional_deny_read_paths,
                    deny_write_paths_override: &additional_deny_write_paths,
                },
            )
        } else {
            run_windows_sandbox_capture_with_filesystem_overrides(
                policy_str.as_str(),
                &sandbox_cwd,
                codex_home.as_ref(),
                command,
                &cwd,
                env,
                timeout_ms,
                &additional_deny_read_paths,
                &additional_deny_write_paths,
                windows_sandbox_private_desktop,
            )
        }
    })
    .await;

    let capture = match spawn_res {
        Ok(Ok(v)) => v,
        Ok(Err(err)) => {
            record_windows_sandbox_spawn_failure(
                command_path.as_deref(),
                sandbox_level,
                &err.to_string(),
            );
            return Err(CodexErr::Io(io::Error::other(format!(
                "windows sandbox: {err}"
            ))));
        }
        Err(join_err) => {
            return Err(CodexErr::Io(io::Error::other(format!(
                "windows sandbox join error: {join_err}"
            ))));
        }
    };

    let exit_status = synthetic_exit_status(capture.exit_code);
    let mut stdout_text = capture.stdout;
    if let Some(max_bytes) = capture_policy.retained_bytes_cap()
        && stdout_text.len() > max_bytes
    {
        stdout_text.truncate(max_bytes);
    }
    let mut stderr_text = capture.stderr;
    if let Some(max_bytes) = capture_policy.retained_bytes_cap()
        && stderr_text.len() > max_bytes
    {
        stderr_text.truncate(max_bytes);
    }
    let stdout = CapturedStreamOutput {
        text: stdout_text,
        truncated_after_lines: None,
    };
    let stderr = CapturedStreamOutput {
        text: stderr_text,
        truncated_after_lines: None,
    };
    let aggregated_output = aggregate_output(&stdout, &stderr, capture_policy.retained_bytes_cap());

    Ok(RawExecToolCallOutput {
        exit_status,
        stdout,
        stderr,
        aggregated_output,
        timed_out: capture.timed_out,
    })
}

/// This is a general-purpose function for executing a command specified by
/// [ExecParams]. Events are reported via `stdout_stream`, if specified, and
/// `after_spawn` is invoked once the child process has been spawned, before
/// output consumption begins.
///
/// `network_sandbox_policy` is used to determine whether
/// CODEX_SANDBOX_NETWORK_DISABLED=1 is added to the environment of the spawned
/// process.
///
/// Note this command does not apply any sandboxing logic. The caller is
/// responsible for constructing [ExecParams::command] to include any sandboxing
/// wrapper args, as appropriate.
struct OutputDeltaForwarder {
    sender: ProcessOutputSender,
    task: JoinHandle<()>,
}

impl OutputDeltaForwarder {
    async fn finish(self) {
        drop(self.sender);
        let _ = self.task.await;
    }
}

fn spawn_output_delta_forwarder(stdout_stream: StdoutStream) -> OutputDeltaForwarder {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<ProcessOutputChunk>();
    let task = tokio::spawn(async move {
        while let Some(chunk) = rx.recv().await {
            let msg = EventMsg::ExecCommandOutputDelta(ExecCommandOutputDeltaEvent {
                call_id: stdout_stream.call_id.clone(),
                sequence: None,
                generates_notification: false,
                created_at_ms: 0,
                stream: match chunk.stream {
                    ProcessOutputStream::Stdout => ExecOutputStream::Stdout,
                    ProcessOutputStream::Stderr => ExecOutputStream::Stderr,
                },
                chunk: chunk.chunk,
            });
            let event = Event {
                id: stdout_stream.sub_id.clone(),
                msg,
            };
            #[allow(clippy::let_unit_value)]
            let _ = stdout_stream.tx_event.send(event).await;
        }
    });
    OutputDeltaForwarder { sender: tx, task }
}

async fn exec(
    params: ExecParams,
    network_sandbox_policy: NetworkSandboxPolicy,
    stdout_stream: Option<StdoutStream>,
    after_spawn: Option<Box<dyn FnOnce() + Send>>,
) -> Result<RawExecToolCallOutput> {
    let ExecParams {
        command,
        cwd,
        mut env,
        network,
        arg0,
        expiration,
        capture_policy,

        // If applicable, these fields should have been honored upstream of
        // this exec call.
        windows_sandbox_level: _,
        windows_sandbox_private_desktop: _,
        // These fields are related to approvals, so can be ignored here.
        sandbox_permissions: _,
        justification: _,
    } = params;
    if let Some(network) = network.as_ref() {
        network.apply_to_env(&mut env);
    }

    let (program, args) = command.split_first().ok_or_else(|| {
        CodexErr::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "command args are empty",
        ))
    })?;
    let arg0_ref = arg0.as_deref();
    let child = spawn_child_async(SpawnChildRequest {
        program: PathBuf::from(program),
        args: args.into(),
        arg0: arg0_ref,
        cwd,
        network_sandbox_policy,
        // The environment already has attempt-scoped proxy settings from
        // apply_to_env_for_attempt above. Passing network here would reapply
        // non-attempt proxy vars and drop attempt correlation metadata.
        network: None,
        stdio_policy: StdioPolicy::RedirectForShellTool,
        env,
    })
    .await?;
    if let Some(after_spawn) = after_spawn {
        after_spawn();
    }
    let output_forwarder = stdout_stream.map(spawn_output_delta_forwarder);
    let output_sender = output_forwarder
        .as_ref()
        .map(|forwarder| forwarder.sender.clone());
    let result = consume_process_output(child, expiration, capture_policy, output_sender)
        .await
        .map_err(CodexErr::Io);
    if let Some(forwarder) = output_forwarder {
        forwarder.finish().await;
    }
    result
}

#[cfg(test)]
#[path = "exec_tests.rs"]
mod tests;
