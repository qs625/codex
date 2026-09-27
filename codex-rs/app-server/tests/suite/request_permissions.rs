use anyhow::Result;
use app_server_protocol::PermissionGrantScope;
use app_server_protocol::PermissionsRequestApprovalResponse;
use app_server_protocol::ServerRequest;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::TurnStartParams;
use app_server_protocol::UserInput as V2UserInput;
use app_test_support::create_final_assistant_message_sse_response;
use app_test_support::create_mock_responses_server_sequence;
use app_test_support::create_request_permissions_sse_response;
use app_test_support::initialized_mcp;
use app_test_support::read_server_request;
use app_test_support::start_thread;
use app_test_support::start_turn;
use app_test_support::wait_for_server_request_resolved_before_turn_completed;

const DEFAULT_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn request_permissions_round_trip() -> Result<()> {
    let codex_home = tempfile::TempDir::new()?;
    let responses = vec![
        create_request_permissions_sse_response("call1")?,
        create_final_assistant_message_sse_response("done")?,
    ];
    let server = create_mock_responses_server_sequence(responses).await;
    create_config_toml(codex_home.path(), &server.uri())?;

    let mut mcp = initialized_mcp(codex_home.path(), DEFAULT_READ_TIMEOUT).await?;

    let thread = start_thread(
        &mut mcp,
        ThreadStartParams {
            model: Some("mock-model".to_string()),
            ..Default::default()
        },
        DEFAULT_READ_TIMEOUT,
    )
    .await?
    .thread;

    let turn = start_turn(
        &mut mcp,
        TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: "pick a directory".to_string(),
                text_elements: Vec::new(),
            }],
            model: Some("mock-model".to_string()),
            ..Default::default()
        },
        DEFAULT_READ_TIMEOUT,
    )
    .await?
    .turn;

    let server_req = read_server_request(&mut mcp, DEFAULT_READ_TIMEOUT).await?;
    let ServerRequest::PermissionsRequestApproval { request_id, params } = server_req else {
        panic!("expected PermissionsRequestApproval request, got: {server_req:?}");
    };

    assert_eq!(params.thread_id, thread.id);
    assert_eq!(params.turn_id, turn.id);
    assert_eq!(params.item_id, "call1");
    assert!(params.cwd.as_path().is_absolute());
    assert_eq!(params.reason, Some("Select a workspace root".to_string()));
    let requested_file_system = params
        .permissions
        .file_system
        .expect("request should include file system permissions");
    let requested_writes = requested_file_system
        .write
        .clone()
        .expect("request should include write permissions");
    assert_eq!(requested_writes.len(), 2);
    assert_eq!(
        requested_file_system.entries,
        Some(vec![
            app_server_protocol::FileSystemSandboxEntry {
                path: app_server_protocol::FileSystemPath::Path {
                    path: requested_writes[0].clone(),
                },
                access: app_server_protocol::FileSystemAccessMode::Write,
            },
            app_server_protocol::FileSystemSandboxEntry {
                path: app_server_protocol::FileSystemPath::Path {
                    path: requested_writes[1].clone(),
                },
                access: app_server_protocol::FileSystemAccessMode::Write,
            },
        ])
    );
    let resolved_request_id = request_id.clone();

    mcp.send_response(
        request_id.clone(),
        serde_json::to_value(PermissionsRequestApprovalResponse {
            permissions: app_server_protocol::GrantedPermissionProfile {
                network: None,
                file_system: Some(app_server_protocol::AdditionalFileSystemPermissions {
                    read: None,
                    write: Some(vec![requested_writes[0].clone()]),
                    glob_scan_max_depth: None,
                    entries: None,
                }),
            },
            scope: PermissionGrantScope::Turn,
            strict_auto_review: None,
        })?,
    )
    .await?;

    wait_for_server_request_resolved_before_turn_completed(
        &mut mcp,
        &thread.id,
        resolved_request_id,
        DEFAULT_READ_TIMEOUT,
    )
    .await?;

    Ok(())
}

fn create_config_toml(codex_home: &std::path::Path, server_uri: &str) -> std::io::Result<()> {
    let config_toml = codex_home.join("config.toml");
    std::fs::write(
        config_toml,
        format!(
            r#"
model = "mock-model"
approval_policy = "untrusted"
sandbox_mode = "read-only"

model_provider = "mock_provider"

[model_providers.mock_provider]
name = "Mock provider for test"
base_url = "{server_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0

[features]
request_permissions_tool = true
"#
        ),
    )
}
