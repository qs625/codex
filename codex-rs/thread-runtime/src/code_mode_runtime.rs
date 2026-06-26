use std::sync::Arc;

use codex_code_mode_api::CodeModeBoxResultFuture;
use codex_code_mode_api::CodeModeNestedToolCall;
use codex_code_mode_api::CodeModeRuntimeService;
use codex_code_mode_api::CodeModeToolKind;
use codex_code_mode_api::CodeModeTurnHost;
use codex_code_mode_api::CodeModeTurnWorker;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use serde_json::Value as JsonValue;
use tokio_util::sync::CancellationToken;

use crate::SharedTurnDiffTracker;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use codex_tool_planning::ToolCall;
use codex_tool_planning::ToolCallSource;
use codex_tool_planning::ToolName;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolPayload;

pub(crate) const PUBLIC_TOOL_NAME: &str = codex_code_mode_api::PUBLIC_TOOL_NAME;

/// Returns true for the un-namespaced code-mode `exec` tool.
pub(crate) fn is_exec_tool_name(tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none() && tool_name.name == PUBLIC_TOOL_NAME
}

#[derive(Clone)]
pub(crate) struct ExecContext {
    pub(super) session: Arc<Session>,
    pub(super) turn: Arc<TurnContext>,
}

pub(crate) fn start_turn_worker(
    service: &Arc<dyn CodeModeRuntimeService>,
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    tool_inputs: Arc<crate::session::turn::TurnToolInputs>,
    tracker: SharedTurnDiffTracker,
) -> Option<Box<dyn CodeModeTurnWorker>> {
    if !turn.code_mode_enabled() {
        return None;
    }

    let exec = ExecContext {
        session: Arc::clone(session),
        turn: Arc::clone(turn),
    };
    let host = Arc::new(CoreTurnHost {
        exec,
        tool_inputs,
        tracker,
    });
    Some(service.start_turn_worker(host))
}

struct CoreTurnHost {
    exec: ExecContext,
    tool_inputs: Arc<crate::session::turn::TurnToolInputs>,
    tracker: SharedTurnDiffTracker,
}

impl CodeModeTurnHost for CoreTurnHost {
    fn invoke_tool(
        &self,
        invocation: CodeModeNestedToolCall,
    ) -> CodeModeBoxResultFuture<'_, JsonValue> {
        let exec = self.exec.clone();
        let tool_inputs = Arc::clone(&self.tool_inputs);
        let tracker = Arc::clone(&self.tracker);
        Box::pin(async move {
            call_nested_tool(exec, tool_inputs, tracker, invocation, CancellationToken::new())
                .await
                .map_err(|error| error.to_string())
        })
    }

    fn notify(
        &self,
        call_id: String,
        cell_id: String,
        text: String,
    ) -> CodeModeBoxResultFuture<'_, ()> {
        let exec = self.exec.clone();
        Box::pin(async move {
            if text.trim().is_empty() {
                return Ok(());
            }
            exec.session
                .inject_hook_inspectable_items(vec![ResponseInputItem::CustomToolCallOutput {
                    call_id,
                    name: Some(PUBLIC_TOOL_NAME.to_string()),
                    output: FunctionCallOutputPayload::from_text(text),
                }])
                .await
                .map_err(|_| {
                    format!(
                        "failed to inject exec notify message for cell {cell_id}: no active turn"
                    )
                })
        })
    }
}

async fn call_nested_tool(
    exec: ExecContext,
    tool_inputs: Arc<crate::session::turn::TurnToolInputs>,
    tracker: SharedTurnDiffTracker,
    invocation: CodeModeNestedToolCall,
    cancellation_token: CancellationToken,
) -> Result<JsonValue, FunctionCallError> {
    let CodeModeNestedToolCall {
        cell_id,
        runtime_tool_call_id,
        tool_name,
        tool_kind,
        input,
    } = invocation;
    if is_exec_tool_name(&tool_name) {
        return Err(FunctionCallError::RespondToModel(format!(
            "{PUBLIC_TOOL_NAME} cannot invoke itself"
        )));
    }

    let payload = match build_nested_tool_payload(tool_kind, &tool_name, input) {
        Ok(payload) => payload,
        Err(error) => return Err(FunctionCallError::RespondToModel(error)),
    };

    let call = ToolCall {
        tool_name,
        call_id: format!("{PUBLIC_TOOL_NAME}-{}", uuid::Uuid::new_v4()),
        payload,
    };
    let result = crate::session::turn::dispatch_tool_call(
        Arc::clone(&exec.session.services.tool_service),
        Arc::clone(&exec.session),
        Arc::clone(&exec.turn),
        tool_inputs,
        tracker,
        call,
        ToolCallSource::CodeMode {
            cell_id,
            runtime_tool_call_id,
        },
        cancellation_token,
    )
        .await?;
    Ok(result.code_mode_result())
}

fn build_nested_tool_payload(
    tool_kind: CodeModeToolKind,
    tool_name: &ToolName,
    input: Option<JsonValue>,
) -> Result<ToolPayload, String> {
    match tool_kind {
        CodeModeToolKind::Function => build_function_tool_payload(tool_name, input),
        CodeModeToolKind::Freeform => build_freeform_tool_payload(tool_name, input),
    }
}

fn build_function_tool_payload(
    tool_name: &ToolName,
    input: Option<JsonValue>,
) -> Result<ToolPayload, String> {
    let arguments = serialize_function_tool_arguments(tool_name, input)?;
    Ok(ToolPayload::Function { arguments })
}

fn serialize_function_tool_arguments(
    tool_name: &ToolName,
    input: Option<JsonValue>,
) -> Result<String, String> {
    match input {
        None => Ok("{}".to_string()),
        Some(JsonValue::Object(map)) => serde_json::to_string(&JsonValue::Object(map))
            .map_err(|err| format!("failed to serialize tool `{tool_name}` arguments: {err}")),
        Some(_) => Err(format!(
            "tool `{tool_name}` expects a JSON object for arguments"
        )),
    }
}

fn build_freeform_tool_payload(
    tool_name: &ToolName,
    input: Option<JsonValue>,
) -> Result<ToolPayload, String> {
    match input {
        Some(JsonValue::String(input)) => Ok(ToolPayload::Custom { input }),
        _ => Err(format!("tool `{tool_name}` expects a string input")),
    }
}

#[cfg(test)]
mod tests {
    use super::build_nested_tool_payload;
    use codex_code_mode_api::CodeModeToolKind;
    use codex_tool_planning::ToolName;
    use codex_tool_types::ToolPayload;
    use serde_json::json;

    #[test]
    fn build_nested_tool_payload_uses_function_kind() {
        let payload = build_nested_tool_payload(
            CodeModeToolKind::Function,
            &ToolName::plain("example"),
            Some(json!({ "value": 1 })),
        )
        .expect("function payload should serialize");

        match payload {
            ToolPayload::Function { arguments } => {
                assert_eq!(arguments, r#"{"value":1}"#.to_string());
            }
            other => panic!("expected function payload, got {other:?}"),
        }
    }

    #[test]
    fn build_nested_tool_payload_uses_freeform_kind() {
        let payload = build_nested_tool_payload(
            CodeModeToolKind::Freeform,
            &ToolName::plain("example"),
            Some(json!("hello")),
        )
        .expect("freeform payload should preserve string input");

        match payload {
            ToolPayload::Custom { input } => {
                assert_eq!(input, "hello".to_string());
            }
            other => panic!("expected freeform payload, got {other:?}"),
        }
    }
}
