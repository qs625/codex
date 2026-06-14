use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolExecutor;
use crate::tools::registry::ToolHandler;
use crate::unified_exec::CommandNotificationKind;
use crate::unified_exec::CommandWaitRequest;
use crate::unified_exec::CommandWaitStatus;
use codex_protocol::models::CommandWaitNotificationKind as ResponseCommandWaitNotificationKind;
use codex_protocol::models::CommandWaitStatus as ResponseCommandWaitStatus;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ResponseItem;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;
use serde::Serialize;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use super::super::shell_spec::create_command_wait_tool;

#[derive(Debug, Deserialize)]
struct CommandWaitArgs {
    command_id: i32,
}

pub struct CommandWaitHandler;

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for CommandWaitHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("command_wait")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_command_wait_tool())
    }

    async fn handle(&self, invocation: ToolInvocation) -> Result<Self::Output, FunctionCallError> {
        let ToolInvocation {
            session,
            turn,
            payload,
            ..
        } = invocation;

        let arguments = match payload {
            ToolPayload::Function { arguments } => arguments,
            _ => {
                return Err(FunctionCallError::RespondToModel(
                    "command_wait handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: CommandWaitArgs = parse_arguments(&arguments)?;
        let output = session
            .services
            .unified_exec_manager
            .wait_for_command_notification(CommandWaitRequest {
                process_id: args.command_id,
            })
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("command_wait failed: {err}"))
            })?;

        let response_item = ResponseItem::CommandWait {
            id: None,
            command_id: output.process_id.to_string(),
            status: match &output.status {
                CommandWaitStatus::Running => ResponseCommandWaitStatus::Running,
                CommandWaitStatus::Completed => ResponseCommandWaitStatus::Completed,
            },
            notification: output.notification.map(|kind| match kind {
                CommandNotificationKind::Output => ResponseCommandWaitNotificationKind::Output,
                CommandNotificationKind::Exit => ResponseCommandWaitNotificationKind::Exit,
            }),
            exit_code: output.exit_code,
            wall_time_seconds: output.wall_time.as_secs_f64(),
            created_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
        };
        session
            .record_conversation_items_and_emit_item_completed(
                turn.as_ref(),
                std::slice::from_ref(&response_item),
            )
            .await;

        let response = CommandWaitResponse {
            command_id: output.process_id,
            status: match &output.status {
                CommandWaitStatus::Running => "running",
                CommandWaitStatus::Completed => "completed",
            },
            notification: output.notification.map(|kind| match kind {
                CommandNotificationKind::Output => "output",
                CommandNotificationKind::Exit => "exit",
            }),
            exit_code: output.exit_code,
            wall_time_seconds: output.wall_time.as_secs_f64(),
        };
        let text = serde_json::to_string(&response)
            .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
        Ok(FunctionToolOutput {
            body: vec![FunctionCallOutputContentItem::InputText { text }],
            success: Some(true),
            post_tool_use_response: None,
        })
    }
}

impl ToolHandler for CommandWaitHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Serialize)]
struct CommandWaitResponse<'a> {
    command_id: i32,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    notification: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    wall_time_seconds: f64,
}
