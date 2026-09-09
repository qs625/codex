use app_server_protocol::McpToolCallResult;
use app_server_protocol::Thread;
use app_server_protocol::ThreadItem;
use serde_json::Value as JsonValue;

// Temporary bandaid for remote clients: thread/resume can include large MCP and
// image-generation payloads. Keep this response-only so persisted rollout
// history, model resume history, and other APIs stay unchanged.
const REDACTED_PAYLOAD: &str = "[redacted]";
const CHATGPT_REMOTE_CLIENT_NAMES: &[&str] =
    &["codex_chatgpt_android_remote", "codex_chatgpt_ios_remote"];

pub(super) fn should_redact_thread_resume_payloads(client_name: Option<&str>) -> bool {
    client_name.is_some_and(|client_name| CHATGPT_REMOTE_CLIENT_NAMES.contains(&client_name))
}

pub(super) fn redact_thread_resume_payloads(thread: &mut Thread) {
    for turn in &mut thread.turns {
        turn.items.retain_mut(|item| match item {
            ThreadItem::McpToolCall {
                arguments,
                result,
                error,
                ..
            } => {
                *arguments = JsonValue::String(REDACTED_PAYLOAD.to_string());
                if result.is_some() {
                    *result = Some(Box::new(redacted_mcp_tool_call_result()));
                }
                if let Some(error) = error {
                    error.message = REDACTED_PAYLOAD.to_string();
                }
                true
            }
            ThreadItem::ImageGeneration { .. } => false,
            ThreadItem::UserMessage { .. }
            | ThreadItem::ClientRecovery { .. }
            | ThreadItem::HookPrompt { .. }
            | ThreadItem::InjectedContext { .. }
            | ThreadItem::AgentMessage { .. }
            | ThreadItem::Plan { .. }
            | ThreadItem::Reasoning { .. }
            | ThreadItem::CommandExecution { .. }
            | ThreadItem::CommandExecutionNotification { .. }
            | ThreadItem::CommandWait { .. }
            | ThreadItem::CommandWriteStdin { .. }
            | ThreadItem::FileChange { .. }
            | ThreadItem::BuiltinToolCall { .. }
            | ThreadItem::DynamicToolCall { .. }
            | ThreadItem::EventDrivenToolCall { .. }
            | ThreadItem::EventDrivenTool { .. }
            | ThreadItem::EventCommandCall { .. }
            | ThreadItem::EventCommandEvent { .. }
            | ThreadItem::ThreadGoalUpdate { .. }
            | ThreadItem::CollabAgentMessage { .. }
            | ThreadItem::ConversationArtifact { .. }
            | ThreadItem::CollabAgentToolCall { .. }
            | ThreadItem::CollabAgentStatusUpdate { .. }
            | ThreadItem::WorkflowRunProgress { .. }
            | ThreadItem::WebSearch { .. }
            | ThreadItem::ImageView { .. }
            | ThreadItem::EnteredReviewMode { .. }
            | ThreadItem::ExitedReviewMode { .. }
            | ThreadItem::ContextCompaction { .. } => true,
        });
    }
}

fn redacted_mcp_tool_call_result() -> McpToolCallResult {
    McpToolCallResult {
        content: vec![serde_json::json!({
            "type": "text",
            "text": REDACTED_PAYLOAD,
        })],
        structured_content: None,
        meta: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_server_protocol::McpToolCallError;
    use app_server_protocol::McpToolCallStatus;
    use app_server_protocol::SessionSource;
    use app_server_protocol::ThreadLifecycleStatus;
    use app_server_protocol::Turn;
    use app_server_protocol::TurnItemsView;
    use app_server_protocol::TurnStatus;
    use codex_utils_absolute_path::test_support::PathBufExt;
    use codex_utils_absolute_path::test_support::test_path_buf;
    use pretty_assertions::assert_eq;

    #[test]
    fn redacts_mcp_success_result_and_removes_image_generation() {
        let mut thread = test_thread(vec![
            ThreadItem::AgentMessage {
                id: "agent-1".to_string(),
                text: "kept".to_string(),
                phase: None,
                memory_citation: None,
            },
            ThreadItem::McpToolCall {
                id: "mcp-1".to_string(),
                server: "docs".to_string(),
                tool: "lookup".to_string(),
                status: McpToolCallStatus::Completed,
                arguments: serde_json::json!({"secret":"argument"}),
                mcp_app_resource_uri: Some("ui://widget/lookup.html".to_string()),
                result: Some(Box::new(McpToolCallResult {
                    content: vec![serde_json::json!({
                        "type": "text",
                        "text": "secret result"
                    })],
                    structured_content: Some(serde_json::json!({"secret":"structured"})),
                    meta: Some(serde_json::json!({"secret":"meta"})),
                })),
                error: None,
                duration_ms: Some(8),
            },
            ThreadItem::ImageGeneration {
                id: "ig-1".to_string(),
                status: "completed".to_string(),
                revised_prompt: Some("revised".to_string()),
                result: "base64-result".to_string(),
                saved_path: Some(test_path_buf("/tmp/ig-1.png").abs()),
            },
        ]);

        redact_thread_resume_payloads(&mut thread);

        assert_eq!(thread.turns[0].items.len(), 2);
        assert_eq!(
            thread.turns[0].items[0],
            ThreadItem::AgentMessage {
                id: "agent-1".to_string(),
                text: "kept".to_string(),
                phase: None,
                memory_citation: None,
            }
        );
        assert_eq!(
            thread.turns[0].items[1],
            ThreadItem::McpToolCall {
                id: "mcp-1".to_string(),
                server: "docs".to_string(),
                tool: "lookup".to_string(),
                status: McpToolCallStatus::Completed,
                arguments: JsonValue::String(REDACTED_PAYLOAD.to_string()),
                mcp_app_resource_uri: Some("ui://widget/lookup.html".to_string()),
                result: Some(Box::new(redacted_mcp_tool_call_result())),
                error: None,
                duration_ms: Some(8),
            }
        );
    }

    #[test]
    fn redacts_mcp_error_message() {
        let mut thread = test_thread(vec![ThreadItem::McpToolCall {
            id: "mcp-1".to_string(),
            server: "docs".to_string(),
            tool: "lookup".to_string(),
            status: McpToolCallStatus::Failed,
            arguments: serde_json::json!({"secret":"argument"}),
            mcp_app_resource_uri: None,
            result: None,
            error: Some(McpToolCallError {
                message: "secret error".to_string(),
            }),
            duration_ms: Some(8),
        }]);

        redact_thread_resume_payloads(&mut thread);

        assert_eq!(
            thread.turns[0].items[0],
            ThreadItem::McpToolCall {
                id: "mcp-1".to_string(),
                server: "docs".to_string(),
                tool: "lookup".to_string(),
                status: McpToolCallStatus::Failed,
                arguments: JsonValue::String(REDACTED_PAYLOAD.to_string()),
                mcp_app_resource_uri: None,
                result: None,
                error: Some(McpToolCallError {
                    message: REDACTED_PAYLOAD.to_string(),
                }),
                duration_ms: Some(8),
            }
        );
    }

    #[test]
    fn keeps_client_recovery_typed_payload() {
        let recovery = ThreadItem::ClientRecovery {
            id: "client-recovery:11111111-1111-4111-8111-111111111111".to_string(),
            recovery_identity: Some("11111111-1111-4111-8111-111111111111".to_string()),
            launcher_claim_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
            launcher_evidence_version: Some(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    .to_string(),
            ),
            transaction_id: "tx-1".to_string(),
            request_id: "req-1".to_string(),
            failed_build_id: "failed-build".to_string(),
            failed_build_hash: "failed-hash".to_string(),
            source_commit: "source-commit".to_string(),
            requested_by_thread_id: Some("thread-requester".to_string()),
            mode: "full".to_string(),
            failure_phase: "ready-timeout".to_string(),
            exit_code: Some(1),
            signal: Some("SIGTERM".to_string()),
            ready_timeout_ms: Some(30_000),
            log_path: Some("/tmp/recovery.log".to_string()),
            transaction_path: Some("/tmp/transaction.json".to_string()),
            recovered_build_id: "recovered-build".to_string(),
            prompt: "Inspect recovery evidence.".to_string(),
            evidence_path: "/tmp/evidence.json".to_string(),
            recorded_at_ms: 42,
        };
        let mut thread = test_thread(vec![recovery.clone()]);

        redact_thread_resume_payloads(&mut thread);

        assert_eq!(thread.turns[0].items, vec![recovery]);
    }

    fn test_thread(items: Vec<ThreadItem>) -> Thread {
        Thread {
            id: "thread-1".to_string(),
            session_id: "session-1".to_string(),
            forked_from_id: None,
            preview: "preview".to_string(),
            ephemeral: false,
            model_provider: "mock_provider".to_string(),
            created_at: 0,
            updated_at: 0,
            lifecycle_status: ThreadLifecycleStatus::completed(None),
            path: None,
            cwd: test_path_buf("/tmp").abs(),
            cli_version: "0.0.0".to_string(),
            source: SessionSource::Cli,
            thread_source: None,
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            git_info: None,
            name: None,
            skills: Vec::new(),
            token_usage: None,
            context_usage: None,
            stats: None,
            turns: vec![Turn {
                id: "turn-1".to_string(),
                items,
                items_view: TurnItemsView::Full,
                status: TurnStatus::Completed,
                error: None,
                started_at: None,
                completed_at: None,
                duration_ms: None,
            }],
            active_subscription_items: None,
            active_command_items: None,
        }
    }
}
