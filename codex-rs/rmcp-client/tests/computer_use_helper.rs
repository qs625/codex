use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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

fn packaged_helper_env(root: &TempDir) -> anyhow::Result<HashMap<OsString, OsString>> {
    let helper_bundle = root.path().join("Root Worker Computer Use.app");
    let executable_dir = helper_bundle.join("Contents").join("MacOS");
    let helper_executable = executable_dir.join("Root Worker Computer Use");
    let native_executable = executable_dir.join("morpheus-computer-use-native");
    std::fs::create_dir_all(&executable_dir)?;
    std::fs::write(&helper_executable, "#!/bin/sh\n")?;
    std::fs::write(&native_executable, "native")?;
    #[cfg(unix)]
    {
        std::fs::set_permissions(&helper_executable, std::fs::Permissions::from_mode(0o755))?;
        std::fs::set_permissions(&native_executable, std::fs::Permissions::from_mode(0o755))?;
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
        helper_bundle.into_os_string(),
    );
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_HELPER_EXECUTABLE"),
        helper_executable.into_os_string(),
    );
    env.insert(
        OsString::from("MORPHEUS_COMPUTER_USE_NATIVE_HELPER_EXECUTABLE"),
        native_executable.into_os_string(),
    );
    Ok(env)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn rmcp_client_can_list_and_call_computer_use_helper() -> anyhow::Result<()> {
    let repo_root = repo_root();
    let helper_script = repo_root
        .join("scripts")
        .join("morpheus-computer-use-mcp.mjs");
    let helper_env = tempfile::tempdir()?;
    let client = RmcpClient::new_stdio_client(
        OsString::from("node"),
        vec![helper_script.into_os_string()],
        Some(packaged_helper_env(&helper_env)?),
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
        json!("packaged-native-helper-executable")
    );

    client.shutdown().await;
    Ok(())
}
