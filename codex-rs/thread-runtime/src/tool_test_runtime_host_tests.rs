use super::*;
use crate::exec::ExecCapturePolicy;
use crate::exec::ExecExpiration;
use crate::sandboxing::ExecOptions;
use crate::tool_runtime_support::SandboxAttempt;
use crate::tool_runtime_support::SandboxAttemptExt;
use crate::tool_runtime_support::managed_network_for_sandbox_permissions;
use codex_network_proxy::ConfigReloader;
use codex_network_proxy::ConfigState;
use codex_network_proxy::NetworkProxy;
use codex_network_proxy::NetworkProxyState;
use codex_network_proxy_api::NetworkProxyConfig;
use codex_network_proxy_api::NetworkProxyConstraints;
use codex_network_proxy_api::PROXY_ENV_KEYS;
#[cfg(target_os = "macos")]
use codex_network_proxy_api::PROXY_GIT_SSH_COMMAND_ENV_KEY;
use codex_network_proxy_api::SharedNetworkProxyRuntime;
use codex_protocol::config_types::WindowsSandboxLevel;
use codex_protocol::models::PermissionProfile;
use codex_protocol::models::SandboxPermissions;
use codex_sandboxing::SandboxManager;
use codex_sandboxing_api::SandboxType;
use core_test_support::PathExt;
use pretty_assertions::assert_eq;
use std::sync::Arc;
use tempfile::tempdir;

struct StaticReloader;

#[async_trait::async_trait]
impl ConfigReloader for StaticReloader {
    fn source_label(&self) -> String {
        "test config state".to_string()
    }

    async fn maybe_reload(&self) -> anyhow::Result<Option<ConfigState>> {
        Ok(None)
    }

    async fn reload_now(&self) -> anyhow::Result<ConfigState> {
        Err(anyhow::anyhow!("force reload is not supported in tests"))
    }
}

async fn test_network_proxy() -> anyhow::Result<NetworkProxy> {
    let state = codex_network_proxy::build_config_state(
        NetworkProxyConfig::default(),
        NetworkProxyConstraints::default(),
    )?;
    NetworkProxy::builder()
        .state(Arc::new(NetworkProxyState::with_reloader(
            state,
            Arc::new(StaticReloader),
        )))
        .managed_by_codex(/*managed_by_codex*/ false)
        .http_addr("127.0.0.1:43128".parse()?)
        .socks_addr("127.0.0.1:48081".parse()?)
        .build()
        .await
}

#[tokio::test]
async fn explicit_escalation_prepares_exec_without_managed_network() -> anyhow::Result<()> {
    let proxy: SharedNetworkProxyRuntime = Arc::new(test_network_proxy().await?);
    let dir = tempdir().expect("create temp dir");
    let cwd = dir.path().abs();
    let mut env = HashMap::from([("CUSTOM_ENV".to_string(), "kept".to_string())]);
    proxy.apply_to_env(&mut env);

    let command = vec!["/bin/echo".to_string(), "ok".to_string()];
    let command = build_sandbox_command(
        &command,
        &cwd,
        &exec_env_for_sandbox_permissions(&env, SandboxPermissions::RequireEscalated),
        /*additional_permissions*/ None,
    )
    .expect("build sandbox command");
    let options = ExecOptions {
        expiration: ExecExpiration::DefaultTimeout,
        capture_policy: ExecCapturePolicy::ShellTool,
    };
    let permissions = PermissionProfile::Disabled;
    let manager = SandboxManager::new();
    let attempt = SandboxAttempt {
        sandbox: SandboxType::None,
        permissions: &permissions,
        enforce_managed_network: false,
        sandbox_runtime: &manager,
        sandbox_cwd: &cwd,
        codex_linux_sandbox_exe: None,
        use_legacy_landlock: false,
        windows_sandbox_level: WindowsSandboxLevel::Disabled,
        windows_sandbox_private_desktop: false,
        network_denial_cancellation_token: None,
    };

    let exec_request = attempt
        .env_for(
            command,
            options,
            managed_network_for_sandbox_permissions(
                Some(&proxy),
                SandboxPermissions::RequireEscalated,
            ),
        )
        .expect("prepare exec request");

    assert!(exec_request.network.is_none());
    for key in PROXY_ENV_KEYS {
        assert_eq!(exec_request.env.get(*key), None, "{key} should be unset");
    }
    #[cfg(target_os = "macos")]
    assert_eq!(exec_request.env.get(PROXY_GIT_SSH_COMMAND_ENV_KEY), None);
    assert_eq!(
        exec_request.env.get("CUSTOM_ENV"),
        Some(&"kept".to_string())
    );

    Ok(())
}
