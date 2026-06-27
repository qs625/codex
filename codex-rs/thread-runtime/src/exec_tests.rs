use super::*;
use codex_command_service_api::DEFAULT_EXEC_OUTPUT_MAX_BYTES as EXEC_OUTPUT_MAX_BYTES;
use codex_process_exec::EXEC_TIMEOUT_EXIT_CODE;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_sandboxing_api::SandboxType;
use pretty_assertions::assert_eq;
use std::collections::HashMap;
use std::time::Duration;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn exec_full_buffer_capture_ignores_expiration() -> Result<()> {
    #[cfg(windows)]
    let command = vec![
        "powershell.exe".to_string(),
        "-NonInteractive".to_string(),
        "-NoLogo".to_string(),
        "-Command".to_string(),
        "Start-Sleep -Milliseconds 50; [Console]::Out.Write('hello')".to_string(),
    ];
    #[cfg(not(windows))]
    let command = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        "sleep 0.05; printf hello".to_string(),
    ];

    let env: HashMap<String, String> = std::env::vars().collect();
    let output = exec(
        ExecParams {
            command,
            cwd: codex_utils_absolute_path::AbsolutePathBuf::current_dir()?,
            expiration: 1.into(),
            capture_policy: ExecCapturePolicy::FullBuffer,
            env,
            network: None,
            sandbox_permissions: SandboxPermissions::UseDefault,
            windows_sandbox_level: WindowsSandboxLevel::Disabled,
            windows_sandbox_private_desktop: false,
            justification: None,
            arg0: None,
        },
        NetworkSandboxPolicy::Enabled,
        /*stdout_stream*/ None,
        /*after_spawn*/ None,
    )
    .await?;

    assert_eq!(output.stdout.into_utf8_lossy().trim(), "hello");
    assert!(!output.timed_out);

    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn exec_full_buffer_capture_keeps_io_drain_timeout_when_descendant_holds_pipe_open()
-> Result<()> {
    let output = tokio::time::timeout(
        Duration::from_millis(IO_DRAIN_TIMEOUT_MS * 3),
        exec(
            ExecParams {
                command: vec![
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    "printf hello; sleep 30 &".to_string(),
                ],
                cwd: codex_utils_absolute_path::AbsolutePathBuf::current_dir()?,
                expiration: 1.into(),
                capture_policy: ExecCapturePolicy::FullBuffer,
                env: std::env::vars().collect(),
                network: None,
                sandbox_permissions: SandboxPermissions::UseDefault,
                windows_sandbox_level: WindowsSandboxLevel::Disabled,
                windows_sandbox_private_desktop: false,
                justification: None,
                arg0: None,
            },
            NetworkSandboxPolicy::Enabled,
            /*stdout_stream*/ None,
            /*after_spawn*/ None,
        ),
    )
    .await
    .expect("full-buffer exec should return once the I/O drain guard fires")?;

    assert!(!output.timed_out);

    Ok(())
}

#[tokio::test]
async fn process_exec_tool_call_preserves_full_buffer_capture_policy() -> Result<()> {
    let byte_count = EXEC_OUTPUT_MAX_BYTES.saturating_add(128 * 1024);
    #[cfg(windows)]
    let command = vec![
        "powershell.exe".to_string(),
        "-NonInteractive".to_string(),
        "-NoLogo".to_string(),
        "-Command".to_string(),
        format!("Start-Sleep -Milliseconds 50; [Console]::Out.Write('a' * {byte_count})"),
    ];
    #[cfg(not(windows))]
    let command = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        format!("sleep 0.05; head -c {byte_count} /dev/zero | tr '\\0' 'a'"),
    ];

    let cwd = codex_utils_absolute_path::AbsolutePathBuf::current_dir()?;
    let sandbox_policy = SandboxPolicy::DangerFullAccess;
    let permission_profile = PermissionProfile::from_legacy_sandbox_policy(&sandbox_policy);
    let sandbox_runtime = codex_sandboxing::SandboxManager::new();
    let output = process_exec_tool_call(
        ExecParams {
            command,
            cwd: cwd.clone(),
            expiration: 1.into(),
            capture_policy: ExecCapturePolicy::FullBuffer,
            env: std::env::vars().collect(),
            network: None,
            sandbox_permissions: SandboxPermissions::UseDefault,
            windows_sandbox_level: WindowsSandboxLevel::Disabled,
            windows_sandbox_private_desktop: false,
            justification: None,
            arg0: None,
        },
        &permission_profile,
        &cwd,
        &None,
        /*use_legacy_landlock*/ false,
        &sandbox_runtime,
        /*stdout_stream*/ None,
    )
    .await?;

    assert!(!output.timed_out);
    assert_eq!(output.stdout.text.len(), byte_count);

    Ok(())
}

#[test]
fn process_exec_tool_call_uses_platform_sandbox_for_network_only_restrictions() {
    let expected =
        codex_sandboxing_api::get_platform_sandbox(/*windows_sandbox_enabled*/ false)
            .unwrap_or(SandboxType::None);

    assert_eq!(
        select_process_exec_tool_sandbox_type(
            &codex_sandboxing::SandboxManager::new(),
            &FileSystemSandboxPolicy::unrestricted(),
            NetworkSandboxPolicy::Restricted,
            codex_protocol::config_types::WindowsSandboxLevel::Disabled,
            /*enforce_managed_network*/ false,
        ),
        expected
    );
}

#[cfg(unix)]
#[tokio::test]
async fn kill_child_process_group_kills_grandchildren_on_timeout() -> Result<()> {
    // On Linux/macOS, /bin/bash is typically present; on FreeBSD/OpenBSD,
    // prefer /bin/sh to avoid NotFound errors.
    #[cfg(any(target_os = "freebsd", target_os = "openbsd"))]
    let command = vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        "sleep 60 & echo $!; sleep 60".to_string(),
    ];
    #[cfg(all(unix, not(any(target_os = "freebsd", target_os = "openbsd"))))]
    let command = vec![
        "/bin/bash".to_string(),
        "-c".to_string(),
        "sleep 60 & echo $!; sleep 60".to_string(),
    ];
    let cwd = codex_utils_absolute_path::AbsolutePathBuf::current_dir()?;
    let env: HashMap<String, String> = std::env::vars().collect();
    let params = ExecParams {
        command,
        cwd,
        expiration: 500.into(),
        capture_policy: ExecCapturePolicy::ShellTool,
        env,
        network: None,
        sandbox_permissions: SandboxPermissions::UseDefault,
        windows_sandbox_level: codex_protocol::config_types::WindowsSandboxLevel::Disabled,
        windows_sandbox_private_desktop: false,
        justification: None,
        arg0: None,
    };

    let output = exec(
        params,
        NetworkSandboxPolicy::Restricted,
        /*stdout_stream*/ None,
        /*after_spawn*/ None,
    )
    .await?;
    assert!(output.timed_out);

    let stdout = output.stdout.into_utf8_lossy();
    let pid_line = stdout.lines().next().unwrap_or("").trim();
    let pid: i32 = pid_line.parse().map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Failed to parse pid from stdout '{pid_line}': {error}"),
        )
    })?;

    let mut killed = false;
    for _ in 0..20 {
        // Use kill(pid, 0) to check if the process is alive.
        if unsafe { libc::kill(pid, 0) } == -1
            && let Some(libc::ESRCH) = std::io::Error::last_os_error().raw_os_error()
        {
            killed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(killed, "grandchild process with pid {pid} is still alive");
    Ok(())
}

#[tokio::test]
async fn process_exec_tool_call_respects_cancellation_token() -> Result<()> {
    let command = long_running_command();
    let cwd = codex_utils_absolute_path::AbsolutePathBuf::current_dir()?;
    let env: HashMap<String, String> = std::env::vars().collect();
    let cancel_token = CancellationToken::new();
    let cancel_tx = cancel_token.clone();
    let params = ExecParams {
        command,
        cwd: cwd.clone(),
        expiration: ExecExpiration::Cancellation(cancel_token),
        capture_policy: ExecCapturePolicy::ShellTool,
        env,
        network: None,
        sandbox_permissions: SandboxPermissions::UseDefault,
        windows_sandbox_level: codex_protocol::config_types::WindowsSandboxLevel::Disabled,
        windows_sandbox_private_desktop: false,
        justification: None,
        arg0: None,
    };
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        cancel_tx.cancel();
    });
    let sandbox_runtime = codex_sandboxing::SandboxManager::new();
    let result = timeout(
        Duration::from_secs(5),
        process_exec_tool_call(
            params,
            &PermissionProfile::Disabled,
            &cwd,
            &None,
            /*use_legacy_landlock*/ false,
            &sandbox_runtime,
            /*stdout_stream*/ None,
        ),
    )
    .await
    .expect("cancellation should stop the process promptly");
    let output = result.expect("cancellation should return a non-timeout exec result");
    assert!(!output.timed_out);
    assert_ne!(output.exit_code, 0);
    assert_ne!(output.exit_code, EXEC_TIMEOUT_EXIT_CODE);
    Ok(())
}

#[cfg(unix)]
fn long_running_command() -> Vec<String> {
    vec![
        "/bin/sh".to_string(),
        "-c".to_string(),
        "sleep 30".to_string(),
    ]
}

#[cfg(windows)]
fn long_running_command() -> Vec<String> {
    vec![
        "powershell.exe".to_string(),
        "-NonInteractive".to_string(),
        "-NoLogo".to_string(),
        "-Command".to_string(),
        "Start-Sleep -Seconds 30".to_string(),
    ]
}
