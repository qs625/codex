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
use std::time::Duration;
use std::time::UNIX_EPOCH;

use codex_tools::create_command_wait_tool;

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
        let item_id = format!("response-item-{}", uuid::Uuid::new_v4());
        let created_at_ms = now_unix_timestamp_ms();
        let command_wait = session
            .services
            .unified_exec_manager
            .begin_command_wait(CommandWaitRequest {
                process_id: args.command_id,
            })
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("command_wait failed: {err}"))
            })?;
        let wait_timeout = command_wait.wait_timeout;
        let started_item = command_wait_item(CommandWaitItemInput {
            id: item_id.clone(),
            command_id: command_wait.process_id,
            status: CommandWaitStatus::Running,
            notification: None,
            exit_code: None,
            wall_time: Duration::ZERO,
            wait_timeout,
            created_at_ms,
        });
        session
            .emit_model_item_started_display_event(turn.as_ref(), &started_item)
            .await;

        let output = session
            .services
            .unified_exec_manager
            .finish_command_wait(command_wait)
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("command_wait failed: {err}"))
            })?;

        let response_item = command_wait_item(CommandWaitItemInput {
            id: item_id,
            command_id: output.process_id,
            status: output.status.clone(),
            notification: output.notification,
            exit_code: output.exit_code,
            wall_time: output.wall_time,
            wait_timeout: output.wait_timeout,
            created_at_ms,
        });
        session
            .record_model_items_and_emit_display_events(
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
            wait_timeout_ms: output.wait_timeout.as_millis() as i64,
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
    wait_timeout_ms: i64,
}

struct CommandWaitItemInput {
    id: String,
    command_id: i32,
    status: CommandWaitStatus,
    notification: Option<CommandNotificationKind>,
    exit_code: Option<i32>,
    wall_time: Duration,
    wait_timeout: Duration,
    created_at_ms: i64,
}

fn command_wait_item(input: CommandWaitItemInput) -> ResponseItem {
    ResponseItem::CommandWait {
        id: Some(input.id),
        command_id: input.command_id.to_string(),
        status: match input.status {
            CommandWaitStatus::Running => ResponseCommandWaitStatus::Running,
            CommandWaitStatus::Completed => ResponseCommandWaitStatus::Completed,
        },
        notification: input.notification.map(|kind| match kind {
            CommandNotificationKind::Output => ResponseCommandWaitNotificationKind::Output,
            CommandNotificationKind::Exit => ResponseCommandWaitNotificationKind::Exit,
        }),
        exit_code: input.exit_code,
        wall_time_seconds: input.wall_time.as_secs_f64(),
        wait_timeout_ms: input.wait_timeout.as_millis() as i64,
        created_at_ms: input.created_at_ms,
    }
}

fn now_unix_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn command_wait_started_and_completed_items_reuse_id_and_window() {
        let id = "response-item-wait-1".to_string();
        let started = command_wait_item(CommandWaitItemInput {
            id: id.clone(),
            command_id: 7,
            status: CommandWaitStatus::Running,
            notification: None,
            exit_code: None,
            wall_time: Duration::ZERO,
            wait_timeout: Duration::from_millis(750),
            created_at_ms: 1234,
        });
        let completed = command_wait_item(CommandWaitItemInput {
            id: id.clone(),
            command_id: 7,
            status: CommandWaitStatus::Completed,
            notification: Some(CommandNotificationKind::Exit),
            exit_code: Some(0),
            wall_time: Duration::from_millis(25),
            wait_timeout: Duration::from_millis(750),
            created_at_ms: 1234,
        });

        let ResponseItem::CommandWait {
            id: started_id,
            status: started_status,
            wait_timeout_ms: started_wait_timeout_ms,
            ..
        } = started
        else {
            panic!("expected command wait item");
        };
        let ResponseItem::CommandWait {
            id: completed_id,
            status: completed_status,
            wait_timeout_ms: completed_wait_timeout_ms,
            ..
        } = completed
        else {
            panic!("expected command wait item");
        };

        assert_eq!(started_id, Some(id.clone()));
        assert_eq!(completed_id, Some(id));
        assert_eq!(started_status, ResponseCommandWaitStatus::Running);
        assert_eq!(completed_status, ResponseCommandWaitStatus::Completed);
        assert_eq!(started_wait_timeout_ms, 750);
        assert_eq!(completed_wait_timeout_ms, 750);
    }
}
