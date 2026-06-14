use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolExecutor;
use crate::tools::registry::ToolHandler;
use crate::unified_exec::WriteStdinRequest;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::TerminalInteractionEvent;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use serde::Deserialize;
use serde::Serialize;

use super::super::shell_spec::create_write_stdin_tool;

#[derive(Debug, Deserialize)]
struct WriteStdinArgs {
    command_id: i32,
    #[serde(default)]
    chars: Option<String>,
}

pub struct WriteStdinHandler;

#[async_trait::async_trait]
impl ToolExecutor<ToolInvocation> for WriteStdinHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("command_write_stdin")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_write_stdin_tool())
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
                    "command_write_stdin handler received unsupported payload".to_string(),
                ));
            }
        };

        let args: WriteStdinArgs = parse_arguments(&arguments)?;
        let Some(chars) = args.chars else {
            return Err(FunctionCallError::RespondToModel(
                "command_write_stdin requires non-empty `chars`; use command_wait for command completion or output notifications instead of polling for output.".to_string(),
            ));
        };
        if chars.is_empty() {
            return Err(FunctionCallError::RespondToModel(
                "command_write_stdin requires non-empty `chars`; use command_wait for command completion or output notifications instead of polling for output.".to_string(),
            ));
        }
        let response = session
            .services
            .unified_exec_manager
            .write_command_stdin(WriteStdinRequest {
                process_id: args.command_id,
                input: &chars,
            })
            .await
            .map_err(|err| {
                FunctionCallError::RespondToModel(format!("command_write_stdin failed: {err}"))
            })?;

        let interaction = TerminalInteractionEvent {
            call_id: response.call_id.clone(),
            process_id: response.process_id.to_string(),
            stdin: chars.clone(),
        };
        session
            .send_event(turn.as_ref(), EventMsg::TerminalInteraction(interaction))
            .await;

        let text = serde_json::to_string(&CommandWriteStdinResponse {
            command_id: response.process_id,
            bytes_written: response.bytes_written,
        })
        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))?;
        Ok(FunctionToolOutput {
            body: vec![FunctionCallOutputContentItem::InputText { text }],
            success: Some(true),
            post_tool_use_response: None,
        })
    }
}

#[derive(Serialize)]
struct CommandWriteStdinResponse {
    command_id: i32,
    bytes_written: usize,
}

impl ToolHandler for WriteStdinHandler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}
