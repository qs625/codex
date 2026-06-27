use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use codex_code_mode_api::DEFAULT_WAIT_YIELD_TIME_MS;
use codex_code_mode_api::ExecuteRequest;
use codex_code_mode_api::FunctionCallOutputContentItem as CodeModeContentItem;
use codex_code_mode_api::ImageDetail as CodeModeImageDetail;
use codex_code_mode_api::RuntimeResponse;
use codex_code_mode_api::WaitOutcome;
use codex_code_mode_api::WaitRequest;
use codex_code_mode_api::parse_exec_source;
use codex_command_runtime::resolve_max_tokens;
use codex_protocol::models::DEFAULT_IMAGE_DETAIL;
use codex_thread_api::SessionCodeModeCaller;
use codex_thread_api::ThreadRuntimeCapability;
use codex_thread_runtime::ThreadRuntimeSession;
use codex_thread_runtime::ThreadTurnContext;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::code_mode_exec_plan_for_specs;
use codex_tool_planning::collect_code_mode_tool_definitions;
use codex_tool_planning::create_code_mode_tool;
use codex_tool_planning::create_code_mode_wait_tool;
use codex_tool_runtime::FunctionToolOutput;
use codex_tool_runtime_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolName;
use codex_tool_types::ToolPayload;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::formatted_truncate_text_content_items_with_policy;
use codex_utils_output_truncation::truncate_function_output_items_with_policy;
use serde::Deserialize;

use crate::context::TypedToolSpecRequest;

pub(crate) fn specs(
    request: &TypedToolSpecRequest<'_>,
    nested_specs: &[ToolSpec],
) -> Vec<ToolSpec> {
    if !request.config.environment_mode.has_environment() {
        return Vec::new();
    }

    let exec_plan = code_mode_exec_plan_for_specs(nested_specs);
    vec![
        create_code_mode_tool(
            &exec_plan.enabled_tools,
            &exec_plan.namespace_descriptions,
            request.config.code_mode_only_enabled,
            true,
        ),
        create_code_mode_wait_tool(),
    ]
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            codex_code_mode_api::PUBLIC_TOOL_NAME | codex_code_mode_api::WAIT_TOOL_NAME
        )
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    false
}

pub(crate) async fn dispatch(
    session: Arc<ThreadRuntimeSession>,
    turn: Arc<ThreadTurnContext>,
    nested_tool_specs: Vec<ToolSpec>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let result = match &call.payload {
        ToolPayload::Custom { input }
            if call.tool_name.namespace.is_none()
                && call.tool_name.name.as_str() == codex_code_mode_api::PUBLIC_TOOL_NAME =>
        {
            dispatch_execute(session.as_ref(), turn.as_ref(), call.call_id.clone(), input, &nested_tool_specs)
                .await?
        }
        ToolPayload::Function { arguments }
            if call.tool_name.namespace.is_none()
                && call.tool_name.name.as_str() == codex_code_mode_api::WAIT_TOOL_NAME =>
        {
            dispatch_wait(session.as_ref(), turn.as_ref(), arguments).await?
        }
        _ => {
            return Err(FunctionCallError::RespondToModel(format!(
                "{} received unsupported payload",
                call.tool_name
            )));
        }
    };

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(result),
        post_tool_use_payload: None,
    })
}

#[derive(Debug, Deserialize)]
struct ExecWaitArgs {
    cell_id: String,
    #[serde(default = "default_wait_yield_time_ms")]
    yield_time_ms: u64,
    #[serde(default)]
    max_tokens: Option<usize>,
    #[serde(default)]
    terminate: bool,
}

fn default_wait_yield_time_ms() -> u64 {
    DEFAULT_WAIT_YIELD_TIME_MS
}

async fn dispatch_execute(
    session: &ThreadRuntimeSession,
    turn: &ThreadTurnContext,
    call_id: String,
    code: &str,
    nested_tool_specs: &[ToolSpec],
) -> Result<FunctionToolOutput, FunctionCallError> {
    let args = parse_exec_source(code).map_err(FunctionCallError::RespondToModel)?;
    let enabled_tools = collect_code_mode_tool_definitions(nested_tool_specs);
    let stored_values = session.code_mode_stored_values().await;
    let runtime_cell_id = session.code_mode_allocate_cell_id();
    session.record_code_mode_cell_started(turn, runtime_cell_id.as_str(), call_id.as_str(), args.code.as_str());
    let started_at = Instant::now();
    let response = session
        .code_mode_execute(ExecuteRequest {
            cell_id: runtime_cell_id.clone(),
            tool_call_id: call_id,
            enabled_tools,
            source: args.code,
            stored_values,
            yield_time_ms: args.yield_time_ms,
            max_output_tokens: args.max_output_tokens,
        })
        .await
        .map_err(FunctionCallError::RespondToModel)?;
    session.record_code_mode_cell_initial_response(turn, runtime_cell_id.as_str(), &response);
    if !matches!(response, RuntimeResponse::Yielded { .. }) {
        session.record_code_mode_cell_ended(turn, runtime_cell_id.as_str(), &response);
    }
    handle_runtime_response(session, turn, response, args.max_output_tokens, started_at)
        .await
        .map_err(FunctionCallError::RespondToModel)
}

async fn dispatch_wait(
    session: &ThreadRuntimeSession,
    turn: &ThreadTurnContext,
    arguments: &str,
) -> Result<FunctionToolOutput, FunctionCallError> {
    let args: ExecWaitArgs = serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })?;
    let started_at = Instant::now();
    let wait_response = session
        .code_mode_wait(WaitRequest {
            cell_id: args.cell_id,
            yield_time_ms: args.yield_time_ms,
            terminate: args.terminate,
        })
        .await
        .map_err(FunctionCallError::RespondToModel)?;
    if let WaitOutcome::LiveCell(response) = &wait_response
        && !matches!(response, RuntimeResponse::Yielded { .. })
    {
        session.record_code_mode_cell_ended(turn, runtime_cell_id(response), response);
    }
    handle_runtime_response(session, turn, wait_response.into(), args.max_tokens, started_at)
        .await
        .map_err(FunctionCallError::RespondToModel)
}

async fn handle_runtime_response(
    session: &ThreadRuntimeSession,
    turn: &ThreadTurnContext,
    response: RuntimeResponse,
    max_output_tokens: Option<usize>,
    started_at: Instant,
) -> Result<FunctionToolOutput, String> {
    let script_status = format_script_status(&response);
    match response {
        RuntimeResponse::Yielded { content_items, .. }
        | RuntimeResponse::Terminated { content_items, .. } => {
            let mut content_items = into_function_call_output_content_items(content_items);
            sanitize_runtime_image_detail(turn, &mut content_items);
            content_items = truncate_code_mode_result(content_items, max_output_tokens);
            prepend_script_status(&mut content_items, &script_status, started_at.elapsed());
            Ok(FunctionToolOutput::from_content(content_items, Some(true)))
        }
        RuntimeResponse::Result {
            content_items,
            stored_values,
            error_text,
            ..
        } => {
            let mut content_items = into_function_call_output_content_items(content_items);
            sanitize_runtime_image_detail(turn, &mut content_items);
            session.code_mode_replace_stored_values(stored_values).await;
            let success = error_text.is_none();
            if let Some(error_text) = error_text {
                content_items.push(codex_protocol::models::FunctionCallOutputContentItem::InputText {
                    text: format!("Script error:\n{error_text}"),
                });
            }
            content_items = truncate_code_mode_result(content_items, max_output_tokens);
            prepend_script_status(&mut content_items, &script_status, started_at.elapsed());
            Ok(FunctionToolOutput::from_content(content_items, Some(success)))
        }
    }
}

fn sanitize_runtime_image_detail(
    turn: &ThreadTurnContext,
    items: &mut [codex_protocol::models::FunctionCallOutputContentItem],
) {
    codex_tool_config::sanitize_original_image_detail(turn.can_request_original_image_detail(), items);
}

fn format_script_status(response: &RuntimeResponse) -> String {
    match response {
        RuntimeResponse::Yielded { cell_id, .. } => format!("Script running with cell ID {cell_id}"),
        RuntimeResponse::Terminated { .. } => "Script terminated".to_string(),
        RuntimeResponse::Result { error_text, .. } => {
            if error_text.is_none() {
                "Script completed".to_string()
            } else {
                "Script failed".to_string()
            }
        }
    }
}

fn prepend_script_status(
    content_items: &mut Vec<codex_protocol::models::FunctionCallOutputContentItem>,
    status: &str,
    wall_time: Duration,
) {
    let wall_time_seconds = ((wall_time.as_secs_f32()) * 10.0).round() / 10.0;
    let header = format!("{status}\nWall time {wall_time_seconds:.1} seconds\nOutput:\n");
    content_items.insert(
        0,
        codex_protocol::models::FunctionCallOutputContentItem::InputText { text: header },
    );
}

fn truncate_code_mode_result(
    items: Vec<codex_protocol::models::FunctionCallOutputContentItem>,
    max_output_tokens: Option<usize>,
) -> Vec<codex_protocol::models::FunctionCallOutputContentItem> {
    let max_output_tokens = resolve_max_tokens(max_output_tokens);
    let policy = TruncationPolicy::Tokens(max_output_tokens);
    if items.iter().all(|item| {
        matches!(
            item,
            codex_protocol::models::FunctionCallOutputContentItem::InputText { .. }
        )
    }) {
        let (truncated_items, _) =
            formatted_truncate_text_content_items_with_policy(&items, policy);
        return truncated_items;
    }
    truncate_function_output_items_with_policy(&items, policy)
}

fn runtime_cell_id(response: &RuntimeResponse) -> &str {
    match response {
        RuntimeResponse::Yielded { cell_id, .. }
        | RuntimeResponse::Terminated { cell_id, .. }
        | RuntimeResponse::Result { cell_id, .. } => cell_id,
    }
}

fn into_function_call_output_content_items(
    content_items: Vec<CodeModeContentItem>,
) -> Vec<codex_protocol::models::FunctionCallOutputContentItem> {
    content_items
        .into_iter()
        .map(|item| match item {
            CodeModeContentItem::InputText { text } => {
                codex_protocol::models::FunctionCallOutputContentItem::InputText { text }
            }
            CodeModeContentItem::InputImage { image_url, detail } => {
                codex_protocol::models::FunctionCallOutputContentItem::InputImage {
                    image_url,
                    detail: detail
                        .map(code_mode_image_detail_into_protocol)
                        .or(Some(DEFAULT_IMAGE_DETAIL)),
                }
            }
        })
        .collect()
}

fn code_mode_image_detail_into_protocol(
    value: CodeModeImageDetail,
) -> codex_protocol::models::ImageDetail {
    match value {
        CodeModeImageDetail::Auto => codex_protocol::models::ImageDetail::Auto,
        CodeModeImageDetail::Low => codex_protocol::models::ImageDetail::Low,
        CodeModeImageDetail::High => codex_protocol::models::ImageDetail::High,
        CodeModeImageDetail::Original => codex_protocol::models::ImageDetail::Original,
    }
}
