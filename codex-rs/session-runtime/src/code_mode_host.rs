use std::collections::HashMap;
use std::sync::Arc;

use codex_code_mode_api::CodeModeBoxResultFuture;
use codex_code_mode_api::CodeModeNestedToolCall;
use codex_code_mode_api::CodeModeRuntimeService;
use codex_code_mode_api::CodeModeToolKind;
use codex_code_mode_api::CodeModeTurnHost;
use codex_code_mode_api::CodeModeTurnWorker;
use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseInputItem;
use serde_json::Value as JsonValue;
use tokio_util::sync::CancellationToken;

use crate::CoreApplyPatchHandlerHost;
use crate::SharedTurnDiffTracker;
use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn::InjectedToolRuntimeRouter;
use crate::session::turn::ToolCallRuntime;
use crate::session::turn_context::TurnContext;
use crate::tools::context::ToolPayload;
use codex_tool_planning::ToolCall;
use codex_tool_planning::ToolCallSource;
use codex_tool_planning::ToolName;
use codex_tool_runtime_api::CodeModeToolHost;

pub(crate) const PUBLIC_TOOL_NAME: &str = codex_code_mode_api::PUBLIC_TOOL_NAME;

/// Returns true for the un-namespaced code-mode `exec` tool.
pub(crate) fn is_exec_tool_name(tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none() && tool_name.name == PUBLIC_TOOL_NAME
}

#[derive(Clone)]
pub(crate) struct ExecContext {
    pub(super) session: Arc<Session>,
}

pub(crate) fn start_turn_worker(
    service: &Arc<dyn CodeModeRuntimeService>,
    session: &Arc<Session>,
    turn: &Arc<TurnContext>,
    router: Arc<crate::CoreToolRuntimeRouter>,
    tracker: SharedTurnDiffTracker,
) -> Option<Box<dyn CodeModeTurnWorker>> {
    if !turn.code_mode_enabled() {
        return None;
    }

    let exec = ExecContext {
        session: Arc::clone(session),
    };
    let tool_runtime = ToolCallRuntime::new(
        Arc::new(InjectedToolRuntimeRouter::new(router)),
        Arc::clone(session),
        Arc::clone(turn),
        tracker,
    );
    let host = Arc::new(CoreTurnHost { exec, tool_runtime });
    Some(service.start_turn_worker(host))
}

impl CodeModeToolHost for CoreApplyPatchHandlerHost {
    fn code_mode_turn_id(&self, turn: &Self::Turn) -> String {
        turn.turn_id()
    }

    fn can_request_original_image_detail(&self, turn: &Self::Turn) -> bool {
        turn.can_request_original_image_detail()
    }

    async fn code_mode_stored_values(
        &self,
        session: &Self::Session,
    ) -> HashMap<String, serde_json::Value> {
        session.code_mode_stored_values().await
    }

    async fn code_mode_replace_stored_values(
        &self,
        session: &Self::Session,
        values: HashMap<String, serde_json::Value>,
    ) {
        session.code_mode_replace_stored_values(values).await;
    }

    fn code_mode_allocate_cell_id(&self, session: &Self::Session) -> String {
        session.code_mode_allocate_cell_id()
    }

    async fn code_mode_execute(
        &self,
        session: &Self::Session,
        request: ExecuteRequest,
    ) -> Result<RuntimeResponse, String> {
        session.code_mode_execute(request).await
    }

    async fn code_mode_wait(
        &self,
        session: &Self::Session,
        request: WaitRequest,
    ) -> Result<WaitOutcome, String> {
        session.code_mode_wait(request).await
    }

    fn record_code_mode_cell_started(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        runtime_cell_id: &str,
        model_visible_call_id: &str,
        source_js: &str,
    ) {
        session.record_code_mode_cell_started(
            turn.turn_id().as_str(),
            runtime_cell_id,
            model_visible_call_id,
            source_js,
        );
    }

    fn record_code_mode_cell_initial_response(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        session.record_code_mode_cell_initial_response(
            turn.turn_id().as_str(),
            runtime_cell_id,
            response,
        );
    }

    fn record_code_mode_cell_ended(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        runtime_cell_id: &str,
        response: &RuntimeResponse,
    ) {
        session.record_code_mode_cell_ended(turn.turn_id().as_str(), runtime_cell_id, response);
    }
}

struct CoreTurnHost {
    exec: ExecContext,
    tool_runtime: ToolCallRuntime,
}

impl CodeModeTurnHost for CoreTurnHost {
    fn invoke_tool(
        &self,
        invocation: CodeModeNestedToolCall,
    ) -> CodeModeBoxResultFuture<'_, JsonValue> {
        let exec = self.exec.clone();
        let tool_runtime = self.tool_runtime.clone();
        Box::pin(async move {
            call_nested_tool(exec, tool_runtime, invocation, CancellationToken::new())
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
    _exec: ExecContext,
    tool_runtime: ToolCallRuntime,
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
    let result = tool_runtime
        .handle_tool_call_with_source(
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
    use crate::tools::context::ToolPayload;
    use codex_code_mode_api::CodeModeToolKind;
    use codex_tool_planning::ToolName;
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
