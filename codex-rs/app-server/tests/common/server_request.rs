use std::path::Path;
use std::time::Duration;

use app_server_protocol::JSONRPCMessage;
use app_server_protocol::JSONRPCResponse;
use app_server_protocol::RequestId;
use app_server_protocol::ServerRequest;
use app_server_protocol::ServerRequestResolvedNotification;
use app_server_protocol::ThreadStartParams;
use app_server_protocol::ThreadStartResponse;
use app_server_protocol::TurnCompletedNotification;
use app_server_protocol::TurnStartParams;
use app_server_protocol::TurnStartResponse;
use tokio::time::timeout;

use crate::McpProcess;
use crate::to_response;

pub async fn initialized_mcp(
    codex_home: &Path,
    read_timeout: Duration,
) -> anyhow::Result<McpProcess> {
    let mut mcp = McpProcess::new(codex_home).await?;
    timeout(read_timeout, mcp.initialize()).await??;
    Ok(mcp)
}

pub async fn start_thread(
    mcp: &mut McpProcess,
    params: ThreadStartParams,
    read_timeout: Duration,
) -> anyhow::Result<ThreadStartResponse> {
    let request_id = mcp.send_thread_start_request(params).await?;
    let response: JSONRPCResponse = timeout(
        read_timeout,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    to_response(response)
}

pub async fn start_turn(
    mcp: &mut McpProcess,
    params: TurnStartParams,
    read_timeout: Duration,
) -> anyhow::Result<TurnStartResponse> {
    let request_id = mcp.send_turn_start_request(params).await?;
    let response: JSONRPCResponse = timeout(
        read_timeout,
        mcp.read_stream_until_response_message(RequestId::Integer(request_id)),
    )
    .await??;
    to_response(response)
}

pub async fn read_server_request(
    mcp: &mut McpProcess,
    read_timeout: Duration,
) -> anyhow::Result<ServerRequest> {
    timeout(read_timeout, mcp.read_stream_until_request_message()).await?
}

pub async fn wait_for_server_request_resolved_before_turn_completed(
    mcp: &mut McpProcess,
    thread_id: &str,
    request_id: RequestId,
    read_timeout: Duration,
) -> anyhow::Result<TurnCompletedNotification> {
    let mut saw_resolved = false;
    loop {
        let message = timeout(read_timeout, mcp.read_next_message()).await??;
        let JSONRPCMessage::Notification(notification) = message else {
            continue;
        };
        match notification.method.as_str() {
            "serverRequest/resolved" => {
                let resolved: ServerRequestResolvedNotification = serde_json::from_value(
                    notification
                        .params
                        .clone()
                        .expect("serverRequest/resolved params"),
                )?;
                assert_eq!(
                    resolved,
                    ServerRequestResolvedNotification {
                        thread_id: thread_id.to_string(),
                        request_id: request_id.clone(),
                    }
                );
                saw_resolved = true;
            }
            "turn/completed" => {
                let completed: TurnCompletedNotification = serde_json::from_value(
                    notification.params.clone().expect("turn/completed params"),
                )?;
                assert!(saw_resolved, "serverRequest/resolved should arrive first");
                return Ok(completed);
            }
            _ => {}
        }
    }
}
