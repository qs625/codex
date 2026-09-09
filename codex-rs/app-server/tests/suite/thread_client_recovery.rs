use anyhow::Result;
use app_server_protocol::ClientInfo;
use app_server_protocol::ClientLifecycleRegisterParams;
use app_server_protocol::ClientLifecycleRegisterResponse;
use app_server_protocol::JSONRPCMessage;
use app_server_protocol::RequestId;
use app_server_protocol::ThreadClientRecoveryRecordParams;
use app_server_protocol::ThreadClientRecoveryRecordResponse;
use app_server_protocol::ThreadSetNameParams;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::ThreadStartResponse;
use app_test_support::McpProcess;
use app_test_support::to_response;
use tempfile::TempDir;

async fn start_named_self_thread(mcp: &mut McpProcess) -> Result<String> {
    let request_id = mcp
        .send_thread_start_request(ThreadStartParams::default())
        .await?;
    let response = mcp
        .read_stream_until_response_message(RequestId::Integer(request_id))
        .await?;
    let response: ThreadStartResponse = to_response(response)?;
    let thread_id = response.thread.id;
    let name_id = mcp
        .send_thread_set_name_request(ThreadSetNameParams {
            thread_id: thread_id.clone(),
            name: "/self".into(),
        })
        .await?;
    mcp.read_stream_until_response_message(RequestId::Integer(name_id))
        .await?;
    Ok(thread_id)
}

fn recovery(thread_id: String, reason: &str) -> ThreadClientRecoveryRecordParams {
    ThreadClientRecoveryRecordParams {
        thread_id,
        recovery_id: "recovery-1".into(),
        activation_id: "activation-1".into(),
        release_id: "release-2".into(),
        reason: reason.into(),
        occurred_at: "2026-09-09T08:30:00.000Z".into(),
        fallback_release_id: Some("release-1".into()),
    }
}

#[tokio::test]
async fn client_recovery_requires_trusted_host_connection() -> Result<()> {
    let home = TempDir::new()?;
    let mut mcp = McpProcess::new(home.path()).await?;
    mcp.initialize().await?;
    let thread_id = start_named_self_thread(&mut mcp).await?;
    let request_id = mcp
        .send_thread_client_recovery_record_request(recovery(thread_id, "failed"))
        .await?;
    let error = mcp
        .read_stream_until_error_message(RequestId::Integer(request_id))
        .await?;
    assert!(error.error.message.contains("registered host lifecycle"));
    Ok(())
}

#[tokio::test]
async fn client_recovery_is_idempotent_and_rejects_conflicting_payload() -> Result<()> {
    let home = TempDir::new()?;
    let mut mcp = McpProcess::new(home.path()).await?;
    let initialized = mcp
        .initialize_with_client_info(ClientInfo {
            name: "root_worker_prototype_electron".into(),
            title: None,
            version: "test".into(),
        })
        .await?;
    assert!(matches!(initialized, JSONRPCMessage::Response(_)));
    let register_id = mcp
        .send_client_lifecycle_register_request(ClientLifecycleRegisterParams {
            host_id: "test-host".into(),
        })
        .await?;
    let registered: ClientLifecycleRegisterResponse = to_response(
        mcp.read_stream_until_response_message(RequestId::Integer(register_id))
            .await?,
    )?;
    assert!(registered.registered);

    let thread_id = start_named_self_thread(&mut mcp).await?;
    let params = recovery(thread_id.clone(), "failed");
    for expected in [true, false] {
        let request_id = mcp
            .send_thread_client_recovery_record_request(params.clone())
            .await?;
        let response: ThreadClientRecoveryRecordResponse = to_response(
            mcp.read_stream_until_response_message(RequestId::Integer(request_id))
                .await?,
        )?;
        assert_eq!(response.recorded, expected);
    }

    let conflict_id = mcp
        .send_thread_client_recovery_record_request(recovery(thread_id, "different"))
        .await?;
    let error = mcp
        .read_stream_until_error_message(RequestId::Integer(conflict_id))
        .await?;
    assert!(error.error.message.contains("different payload"));
    Ok(())
}
