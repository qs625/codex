use super::CoreShellActionProvider;
use crate::config::CONFIG_TOML_FILE;
use crate::config::Constrained;
use crate::sandboxing::SandboxPermissions;
use crate::session::tests::make_session_and_context;
use anyhow::Context;
use codex_execpolicy_api::Policy;
use codex_hooks::Hooks;
use codex_hooks::HooksConfig;
use codex_protocol::models::PermissionProfile;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::permissions::FileSystemSpecialPath;
use codex_protocol::permissions::NetworkSandboxPolicy;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::GuardianCommandSource;
use codex_protocol::protocol::SandboxPolicy;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

fn read_only_file_system_sandbox_policy() -> FileSystemSandboxPolicy {
    FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
        path: FileSystemPath::Special {
            value: FileSystemSpecialPath::Root,
        },
        access: FileSystemAccessMode::Read,
    }])
}

fn permission_profile_from_sandbox_policy(sandbox_policy: &SandboxPolicy) -> PermissionProfile {
    PermissionProfile::from_legacy_sandbox_policy(sandbox_policy)
}

#[tokio::test(flavor = "current_thread")]
async fn execve_permission_request_hook_short_circuits_prompt() -> anyhow::Result<()> {
    let (session, mut turn_context) = make_session_and_context().await;
    std::fs::create_dir_all(&turn_context.config.codex_home)
        .context("recreate codex home for hook fixtures")?;
    let script_path = turn_context
        .config
        .codex_home
        .join("permission_request_hook.py");
    let log_path = turn_context
        .config
        .codex_home
        .join("permission_request_hook_log.jsonl");
    std::fs::write(
        &script_path,
        format!(
            "#!/bin/sh\ncat > {log_path}\nprintf '%s\\n' '{response}'\n",
            log_path = shlex::try_quote(log_path.to_string_lossy().as_ref())?,
            response = "{\"hookSpecificOutput\":{\"hookEventName\":\"PermissionRequest\",\"decision\":{\"behavior\":\"allow\"}}}",
        ),
    )
    .with_context(|| format!("write hook script to {}", script_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = std::fs::metadata(&script_path)
            .with_context(|| format!("read hook script metadata from {}", script_path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&script_path, permissions)
            .with_context(|| format!("set hook script permissions on {}", script_path.display()))?;
    }
    std::fs::write(
        turn_context.config.codex_home.join("hooks.json"),
        serde_json::json!({
            "hooks": {
                "PermissionRequest": [{
                    "hooks": [{
                        "type": "command",
                        "command": script_path.display().to_string(),
                    }]
                }]
            }
        })
        .to_string(),
    )
    .context("write hooks.json")?;
    let config_toml_path = turn_context.config.codex_home.join(CONFIG_TOML_FILE);
    let hook_list = codex_hooks::list_hooks(HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &turn_context.config.config_layer_stack,
            ),
        ),
        ..HooksConfig::default()
    });
    assert_eq!(hook_list.hooks.len(), 1);
    let trusted_config_layer_stack = turn_context.config.config_layer_stack.with_user_config(
        &config_toml_path,
        serde_json::from_value(serde_json::json!({
            "hooks": {
                "state": {
                    hook_list.hooks[0].key.clone(): {
                        "trusted_hash": hook_list.hooks[0].current_hash.clone(),
                    },
                },
            },
        }))
        .context("build trusted hook state")?,
    );

    let mut hook_shell_argv = session
        .user_shell()
        .derive_exec_args("", /*use_login_shell*/ false);
    let hook_shell_program = hook_shell_argv.remove(0);
    let _ = hook_shell_argv.pop();
    *session
        .services
        .hooks
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Arc::new(Hooks::new(HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &trusted_config_layer_stack,
            ),
        ),
        shell_program: Some(hook_shell_program),
        shell_args: hook_shell_argv,
        ..HooksConfig::default()
    }))
        as Arc<dyn codex_hooks_api::HookRuntime>;

    turn_context.approval_policy = Constrained::allow_any(AskForApproval::OnRequest);
    turn_context.permission_profile = PermissionProfile::from_runtime_permissions(
        &read_only_file_system_sandbox_policy(),
        NetworkSandboxPolicy::Restricted,
    );
    let workdir = AbsolutePathBuf::try_from(std::env::current_dir()?)?;
    let target = std::env::temp_dir().join("execve-hook-short-circuit.txt");
    let target_str = target.display().to_string();
    let command = vec!["touch".to_string(), target_str.clone()];
    let expected_hook_command =
        codex_shell_utils::shlex_join(&["/usr/bin/touch".to_string(), target_str]);
    let provider = CoreShellActionProvider {
        policy: std::sync::Arc::new(RwLock::new(Policy::empty())),
        session: std::sync::Arc::new(session),
        turn: std::sync::Arc::new(turn_context),
        call_id: "execve-hook-call".to_string(),
        tool_name: GuardianCommandSource::Shell,
        approval_policy: AskForApproval::OnRequest,
        permission_profile: permission_profile_from_sandbox_policy(
            &SandboxPolicy::new_read_only_policy(),
        ),
        file_system_sandbox_policy: read_only_file_system_sandbox_policy(),
        sandbox_policy_cwd: workdir.clone(),
        sandbox_permissions: SandboxPermissions::RequireEscalated,
        approval_sandbox_permissions: SandboxPermissions::RequireEscalated,
        prompt_permissions: None,
        stopwatch: codex_shell_escalation::Stopwatch::new(Duration::from_secs(1)),
    };

    let action = tokio::time::timeout(
        Duration::from_secs(5),
        codex_shell_escalation::EscalationPolicy::determine_action(
            &provider,
            &AbsolutePathBuf::from_absolute_path("/usr/bin/touch")
                .context("build touch absolute path")?,
            &command,
            &workdir,
        ),
    )
    .await
    .context("timed out waiting for execve permission hook decision")??;
    assert!(matches!(
        action,
        codex_shell_escalation::EscalationDecision::Escalate(
            codex_shell_escalation::EscalationExecution::Unsandboxed
        )
    ));

    let hook_inputs: Vec<Value> = std::fs::read_to_string(&log_path)
        .with_context(|| format!("read hook log at {}", log_path.display()))?
        .lines()
        .map(serde_json::from_str)
        .collect::<serde_json::Result<_>>()
        .context("parse hook log")?;
    assert_eq!(hook_inputs.len(), 1);
    assert_eq!(
        hook_inputs[0]["tool_input"]["command"],
        expected_hook_command
    );
    assert_eq!(
        hook_inputs[0]["tool_input"]["description"],
        serde_json::Value::Null
    );

    Ok(())
}
