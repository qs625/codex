use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use codex_config_types::McpServerEnvVar;
use codex_rmcp_client::ElicitationAction;
use codex_rmcp_client::ElicitationResponse;
use codex_rmcp_client::LocalStdioServerLauncher;
use codex_rmcp_client::RmcpClient;
use futures::FutureExt as _;
use rmcp::model::ClientCapabilities;
use rmcp::model::ElicitationCapability;
use rmcp::model::FormElicitationCapability;
use rmcp::model::Implementation;
use rmcp::model::InitializeRequestParams;
use rmcp::model::ProtocolVersion;
use serde_json::json;
use serial_test::serial;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

struct PackagedHelper {
    env: HashMap<OsString, OsString>,
    executable: PathBuf,
    stable_app_path: PathBuf,
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("rmcp-client crate should have codex-rs parent")
        .parent()
        .expect("codex-rs should have repository parent")
        .to_path_buf()
}

fn init_params() -> InitializeRequestParams {
    InitializeRequestParams {
        meta: None,
        capabilities: ClientCapabilities {
            experimental: None,
            extensions: None,
            roots: None,
            sampling: None,
            elicitation: Some(ElicitationCapability {
                form: Some(FormElicitationCapability {
                    schema_validation: None,
                }),
                url: None,
            }),
            tasks: None,
        },
        client_info: Implementation {
            name: "codex-computer-use-helper-test".into(),
            version: "0.0.0-test".into(),
            title: Some("Codex Computer Use helper test".into()),
            description: None,
            icons: None,
            website_url: None,
        },
        protocol_version: ProtocolVersion::V_2025_06_18,
    }
}

fn packaged_helper(
    root: &TempDir,
    helper_script: &std::path::Path,
) -> anyhow::Result<PackagedHelper> {
    let helper_bundle = root.path().join("Root Worker Computer Use.app");
    let executable_dir = helper_bundle.join("Contents").join("MacOS");
    let contents_dir = helper_bundle.join("Contents");
    let helper_executable = executable_dir.join("Root Worker Computer Use");
    std::fs::create_dir_all(&executable_dir)?;
    std::fs::write(
        contents_dir.join("Info.plist"),
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>com.openai.root-worker-prototype.computer-use.dev</string>
  <key>CFBundleExecutable</key>
  <string>Root Worker Computer Use</string>
</dict>
</plist>
"#,
    )?;
    std::fs::write(
        &helper_executable,
        format!(
            r#"#!/bin/sh
set -eu
export MORPHEUS_COMPUTER_USE_HELPER_MODE="packaged-helper-app"
export MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID="com.openai.root-worker-prototype.computer-use.dev"
export MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH="{}"
export MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE="{}"
export MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH="{}"
export MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE="{}"
export MORPHEUS_COMPUTER_USE_NATIVE_INVOCATION_MODE="launchservices-service-socket"
export MORPHEUS_COMPUTER_USE_SERVICE_SOCKET_PATH="{}/service.sock"
exec node "{}"
"#,
            helper_bundle.display(),
            helper_executable.display(),
            helper_bundle.display(),
            helper_executable.display(),
            root.path().display(),
            helper_script.display(),
        ),
    )?;
    #[cfg(unix)]
    {
        std::fs::set_permissions(&helper_executable, std::fs::Permissions::from_mode(0o755))?;
    }

    let mut env = HashMap::new();
    if let Some(path) = std::env::var_os("PATH") {
        env.insert(OsString::from("PATH"), path);
    }
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_HELPER_MODE"),
        OsString::from("packaged-helper-app"),
    );
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_ID"),
        OsString::from("com.openai.root-worker-prototype.computer-use.dev"),
    );
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_HELPER_BUNDLE_PATH"),
        helper_bundle.clone().into_os_string(),
    );
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE"),
        helper_executable.clone().into_os_string(),
    );
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH"),
        helper_bundle.clone().into_os_string(),
    );
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE"),
        helper_executable.clone().into_os_string(),
    );
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_NATIVE_INVOCATION_MODE"),
        OsString::from("launchservices-service-socket"),
    );
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_SERVICE_SOCKET_PATH"),
        root.path().join("service.sock").into_os_string(),
    );
    Ok(PackagedHelper {
        env,
        executable: helper_executable,
        stable_app_path: helper_bundle,
    })
}

struct EnvVarGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &std::ffi::OsStr) -> Self {
        let previous = std::env::var_os(key);
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn rmcp_client_can_list_and_call_computer_use_helper() -> anyhow::Result<()> {
    let repo_root = repo_root();
    let helper_script = repo_root
        .join("scripts")
        .join("morpheus-computer-use-mcp.mjs");
    let helper_env = tempfile::tempdir()?;
    let helper = packaged_helper(&helper_env, &helper_script)?;
    let client = RmcpClient::new_stdio_client(
        OsString::from("node"),
        vec![helper_script.into_os_string()],
        Some(helper.env),
        &[],
        Some(repo_root.clone()),
        Arc::new(LocalStdioServerLauncher::new(repo_root)),
    )
    .await?;

    client
        .initialize(
            init_params(),
            Some(Duration::from_secs(5)),
            Box::new(|_, _| {
                async {
                    Ok(ElicitationResponse {
                        action: ElicitationAction::Accept,
                        content: Some(json!({})),
                        meta: None,
                    })
                }
                .boxed()
            }),
        )
        .await?;

    let tools = client
        .list_tools(/*params*/ None, Some(Duration::from_secs(5)))
        .await?;
    assert_eq!(tools.tools.len(), 6);
    assert!(
        tools
            .tools
            .iter()
            .any(|tool| tool.name == "computer.permissions_status")
    );

    let result = client
        .call_tool(
            "computer.permissions_status".to_string(),
            Some(json!({ "includeObservation": false })),
            None,
            Some(Duration::from_secs(5)),
        )
        .await?;
    let structured = result
        .structured_content
        .expect("permissions_status should return structured content");
    assert_eq!(structured["ok"], json!(true));
    assert_eq!(
        structured["diagnostics"]["permissionSubject"]["packagedNativeHelperExecutable"],
        json!(true)
    );
    assert_eq!(
        structured["diagnostics"]["permissionSubject"]["stablePermissionSubject"],
        json!(true)
    );
    assert_eq!(
        structured["diagnostics"]["permissionSubject"]["nativeControlSubject"],
        json!("stable-launchservices-service-socket")
    );

    client.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
#[serial(computer_use_helper_executable_env)]
async fn rmcp_client_can_launch_computer_use_helper_through_env_vars_allowlist()
-> anyhow::Result<()> {
    let repo_root = repo_root();
    let helper_script = repo_root
        .join("scripts")
        .join("morpheus-computer-use-mcp.mjs");
    let helper_env = tempfile::tempdir()?;
    let helper = packaged_helper(&helper_env, &helper_script)?;
    let _guard = EnvVarGuard::set(
        "MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE",
        helper.executable.as_os_str(),
    );
    let _stable_guard = EnvVarGuard::set(
        "MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH",
        helper.stable_app_path.as_os_str(),
    );
    let client = RmcpClient::new_stdio_client(
        OsString::from("sh"),
        vec![
            OsString::from("-c"),
            OsString::from(r#"exec "$MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE" mcp-server"#),
        ],
        /*env*/ None,
        &[
            McpServerEnvVar::from("MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE"),
            McpServerEnvVar::from("MORPHEUS_COMPUTER_USE_STABLE_HELPER_APP_PATH"),
        ],
        Some(repo_root.clone()),
        Arc::new(LocalStdioServerLauncher::new(repo_root)),
    )
    .await?;

    client
        .initialize(
            init_params(),
            Some(Duration::from_secs(5)),
            Box::new(|_, _| {
                async {
                    Ok(ElicitationResponse {
                        action: ElicitationAction::Accept,
                        content: Some(json!({})),
                        meta: None,
                    })
                }
                .boxed()
            }),
        )
        .await?;

    assert_eq!(
        client
            .list_tools(/*params*/ None, Some(Duration::from_secs(5)))
            .await?
            .tools
            .len(),
        6
    );

    let result = client
        .call_tool(
            "computer.permissions_status".to_string(),
            Some(json!({ "includeObservation": false })),
            None,
            Some(Duration::from_secs(5)),
        )
        .await?;
    let structured = result
        .structured_content
        .expect("permissions_status should return structured content");
    assert_eq!(
        structured["diagnostics"]["permissionSubject"]["stablePermissionSubject"],
        json!(true)
    );

    client.shutdown().await;
    Ok(())
}
