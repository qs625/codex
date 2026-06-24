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
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::ImageDetail;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::collect_code_mode_tool_definitions;
use codex_tool_planning::create_code_mode_wait_tool;
use codex_tool_runtime_api::ApplyPatchHandlerHost;
use codex_tool_runtime_api::CodeModeToolHost;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolPayload;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::formatted_truncate_text_content_items_with_policy;
use codex_utils_output_truncation::truncate_function_output_items_with_policy;
use serde::Deserialize;

use crate::FunctionToolOutput;
use codex_tool_runtime::ToolInvocation;

pub struct CodeModeExecuteHandler<Host> {
    host: Host,
    spec: ToolSpec,
    nested_tool_specs: Vec<ToolSpec>,
}

impl<Host> CodeModeExecuteHandler<Host> {
    pub fn new(host: Host, spec: ToolSpec, nested_tool_specs: Vec<ToolSpec>) -> Self {
        Self {
            host,
            spec,
            nested_tool_specs,
        }
    }
}

pub struct CodeModeWaitHandler<Host> {
    host: Host,
}

impl<Host> CodeModeWaitHandler<Host> {
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl<Host> Default for CodeModeWaitHandler<Host>
where
    Host: Default,
{
    fn default() -> Self {
        Self {
            host: Host::default(),
        }
    }
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

impl<Host>
    ToolExecutor<
        ToolInvocation<
            <Host as ApplyPatchHandlerHost>::Session,
            <Host as ApplyPatchHandlerHost>::Turn,
            <Host as ApplyPatchHandlerHost>::Tracker,
        >,
    > for CodeModeExecuteHandler<Host>
where
    Host: CodeModeToolHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(codex_code_mode_api::PUBLIC_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(self.spec.clone())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<
            <Host as ApplyPatchHandlerHost>::Session,
            <Host as ApplyPatchHandlerHost>::Turn,
            <Host as ApplyPatchHandlerHost>::Tracker,
        >,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let tool_name = metadata.tool_name;
            let payload = metadata.payload;

            match payload {
                ToolPayload::Custom { input } if is_exec_tool_name(&tool_name) => {
                    self.execute(session, turn, call_id, input).await
                }
                _ => Err(FunctionCallError::RespondToModel(format!(
                    "{} expects raw JavaScript source text",
                    codex_code_mode_api::PUBLIC_TOOL_NAME
                ))),
            }
        })
    }
}

impl<Host>
    ToolHandler<
        ToolInvocation<
            <Host as ApplyPatchHandlerHost>::Session,
            <Host as ApplyPatchHandlerHost>::Turn,
            <Host as ApplyPatchHandlerHost>::Tracker,
        >,
        <Host as ApplyPatchHandlerHost>::DiffContext,
    > for CodeModeExecuteHandler<Host>
where
    Host: CodeModeToolHost + ApplyPatchHandlerHost,
{
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Custom { .. })
    }
}

impl<Host> CodeModeExecuteHandler<Host>
where
    Host: CodeModeToolHost,
{
    async fn execute(
        &self,
        session: <Host as ApplyPatchHandlerHost>::Session,
        turn: <Host as ApplyPatchHandlerHost>::Turn,
        call_id: String,
        code: String,
    ) -> Result<FunctionToolOutput, FunctionCallError> {
        let args = parse_exec_source(&code).map_err(FunctionCallError::RespondToModel)?;
        let enabled_tools = collect_code_mode_tool_definitions(&self.nested_tool_specs);
        let stored_values = self.host.code_mode_stored_values(&session).await;
        // Allocate before starting V8 so the trace can create the parent
        // CodeCell before model-authored JavaScript issues nested tool calls.
        let runtime_cell_id = self.host.code_mode_allocate_cell_id(&session);
        self.host.record_code_mode_cell_started(
            &session,
            &turn,
            runtime_cell_id.as_str(),
            call_id.as_str(),
            args.code.as_str(),
        );
        let started_at = Instant::now();
        let response = self
            .host
            .code_mode_execute(
                &session,
                ExecuteRequest {
                    cell_id: runtime_cell_id.clone(),
                    tool_call_id: call_id,
                    enabled_tools,
                    source: args.code,
                    stored_values,
                    yield_time_ms: args.yield_time_ms,
                    max_output_tokens: args.max_output_tokens,
                },
            )
            .await
            .map_err(FunctionCallError::RespondToModel)?;
        // Record the raw runtime boundary. The model-visible custom-tool output
        // is produced by `handle_runtime_response` and later linked through
        // `CodeCell.output_item_ids` in the reduced trace.
        self.host.record_code_mode_cell_initial_response(
            &session,
            &turn,
            runtime_cell_id.as_str(),
            &response,
        );
        // Yielded cells keep running, so terminal lifecycle is only emitted
        // here when the first response also ended the runtime.
        if !matches!(response, RuntimeResponse::Yielded { .. }) {
            self.host.record_code_mode_cell_ended(
                &session,
                &turn,
                runtime_cell_id.as_str(),
                &response,
            );
        }
        handle_runtime_response(
            &self.host,
            &session,
            &turn,
            response,
            args.max_output_tokens,
            started_at,
        )
        .await
        .map_err(FunctionCallError::RespondToModel)
    }
}

impl<Host>
    ToolExecutor<
        ToolInvocation<
            <Host as ApplyPatchHandlerHost>::Session,
            <Host as ApplyPatchHandlerHost>::Turn,
            <Host as ApplyPatchHandlerHost>::Tracker,
        >,
    > for CodeModeWaitHandler<Host>
where
    Host: CodeModeToolHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(codex_code_mode_api::WAIT_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_code_mode_wait_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<
            <Host as ApplyPatchHandlerHost>::Session,
            <Host as ApplyPatchHandlerHost>::Turn,
            <Host as ApplyPatchHandlerHost>::Tracker,
        >,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let tool_name = metadata.tool_name;
            let payload = metadata.payload;

            match payload {
                ToolPayload::Function { arguments }
                    if tool_name.namespace.is_none()
                        && tool_name.name.as_str() == codex_code_mode_api::WAIT_TOOL_NAME =>
                {
                    let args: ExecWaitArgs = parse_arguments(&arguments)?;
                    let started_at = Instant::now();
                    let wait_response = self
                        .host
                        .code_mode_wait(
                            &session,
                            WaitRequest {
                                cell_id: args.cell_id,
                                yield_time_ms: args.yield_time_ms,
                                terminate: args.terminate,
                            },
                        )
                        .await
                        .map_err(FunctionCallError::RespondToModel)?;
                    if let WaitOutcome::LiveCell(response) = &wait_response
                        && !matches!(response, RuntimeResponse::Yielded { .. })
                    {
                        // Only a live-cell wait can close a CodeCell. A missing
                        // cell is still an ordinary `wait` tool result, but there
                        // is no runtime object for the reducer to complete.
                        self.host.record_code_mode_cell_ended(
                            &session,
                            &turn,
                            runtime_cell_id(response),
                            response,
                        );
                    }
                    handle_runtime_response(
                        &self.host,
                        &session,
                        &turn,
                        wait_response.into(),
                        args.max_tokens,
                        started_at,
                    )
                    .await
                    .map_err(FunctionCallError::RespondToModel)
                }
                _ => Err(FunctionCallError::RespondToModel(format!(
                    "{} expects JSON arguments",
                    codex_code_mode_api::WAIT_TOOL_NAME
                ))),
            }
        })
    }
}

impl<Host>
    ToolHandler<
        ToolInvocation<
            <Host as ApplyPatchHandlerHost>::Session,
            <Host as ApplyPatchHandlerHost>::Turn,
            <Host as ApplyPatchHandlerHost>::Tracker,
        >,
        <Host as ApplyPatchHandlerHost>::DiffContext,
    > for CodeModeWaitHandler<Host>
where
    Host: CodeModeToolHost + ApplyPatchHandlerHost,
{
}

async fn handle_runtime_response<Host>(
    host: &Host,
    session: &<Host as ApplyPatchHandlerHost>::Session,
    turn: &<Host as ApplyPatchHandlerHost>::Turn,
    response: RuntimeResponse,
    max_output_tokens: Option<usize>,
    started_at: Instant,
) -> Result<FunctionToolOutput, String>
where
    Host: CodeModeToolHost,
{
    let script_status = format_script_status(&response);

    match response {
        RuntimeResponse::Yielded { content_items, .. } => {
            let mut content_items = into_function_call_output_content_items(content_items);
            sanitize_runtime_image_detail(host, turn, &mut content_items);
            content_items = truncate_code_mode_result(content_items, max_output_tokens);
            prepend_script_status(&mut content_items, &script_status, started_at.elapsed());
            Ok(FunctionToolOutput::from_content(content_items, Some(true)))
        }
        RuntimeResponse::Terminated { content_items, .. } => {
            let mut content_items = into_function_call_output_content_items(content_items);
            sanitize_runtime_image_detail(host, turn, &mut content_items);
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
            sanitize_runtime_image_detail(host, turn, &mut content_items);
            host.code_mode_replace_stored_values(session, stored_values)
                .await;
            let success = error_text.is_none();
            if let Some(error_text) = error_text {
                content_items.push(FunctionCallOutputContentItem::InputText {
                    text: format!("Script error:\n{error_text}"),
                });
            }
            content_items = truncate_code_mode_result(content_items, max_output_tokens);
            prepend_script_status(&mut content_items, &script_status, started_at.elapsed());
            Ok(FunctionToolOutput::from_content(
                content_items,
                Some(success),
            ))
        }
    }
}

fn sanitize_runtime_image_detail<Host>(
    host: &Host,
    turn: &<Host as ApplyPatchHandlerHost>::Turn,
    items: &mut [FunctionCallOutputContentItem],
) where
    Host: CodeModeToolHost,
{
    codex_tool_config::sanitize_original_image_detail(
        host.can_request_original_image_detail(turn),
        items,
    );
}

fn format_script_status(response: &RuntimeResponse) -> String {
    match response {
        RuntimeResponse::Yielded { cell_id, .. } => {
            format!("Script running with cell ID {cell_id}")
        }
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
    content_items: &mut Vec<FunctionCallOutputContentItem>,
    status: &str,
    wall_time: Duration,
) {
    let wall_time_seconds = ((wall_time.as_secs_f32()) * 10.0).round() / 10.0;
    let header = format!("{status}\nWall time {wall_time_seconds:.1} seconds\nOutput:\n");
    content_items.insert(0, FunctionCallOutputContentItem::InputText { text: header });
}

fn truncate_code_mode_result(
    items: Vec<FunctionCallOutputContentItem>,
    max_output_tokens: Option<usize>,
) -> Vec<FunctionCallOutputContentItem> {
    let max_output_tokens = resolve_max_tokens(max_output_tokens);
    let policy = TruncationPolicy::Tokens(max_output_tokens);
    if items
        .iter()
        .all(|item| matches!(item, FunctionCallOutputContentItem::InputText { .. }))
    {
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

fn is_exec_tool_name(tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none() && tool_name.name == codex_code_mode_api::PUBLIC_TOOL_NAME
}

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

trait IntoProtocol<T> {
    fn into_protocol(self) -> T;
}

fn into_function_call_output_content_items(
    items: Vec<CodeModeContentItem>,
) -> Vec<FunctionCallOutputContentItem> {
    items.into_iter().map(IntoProtocol::into_protocol).collect()
}

impl IntoProtocol<ImageDetail> for CodeModeImageDetail {
    fn into_protocol(self) -> ImageDetail {
        let value = self;
        match value {
            CodeModeImageDetail::Auto => ImageDetail::Auto,
            CodeModeImageDetail::Low => ImageDetail::Low,
            CodeModeImageDetail::High => ImageDetail::High,
            CodeModeImageDetail::Original => ImageDetail::Original,
        }
    }
}

impl IntoProtocol<FunctionCallOutputContentItem> for CodeModeContentItem {
    fn into_protocol(self) -> FunctionCallOutputContentItem {
        let value = self;
        match value {
            CodeModeContentItem::InputText { text } => {
                FunctionCallOutputContentItem::InputText { text }
            }
            CodeModeContentItem::InputImage { image_url, detail } => {
                FunctionCallOutputContentItem::InputImage {
                    image_url,
                    detail: detail
                        .map(IntoProtocol::into_protocol)
                        .or(Some(DEFAULT_IMAGE_DETAIL)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_code_mode_api::ImageDetail as CodeModeImageDetail;
    use codex_protocol::models::ImageDetail;

    #[test]
    fn code_mode_images_default_to_protocol_default_detail() {
        let items =
            into_function_call_output_content_items(vec![CodeModeContentItem::InputImage {
                image_url: "data:image/png;base64,abc".to_string(),
                detail: None,
            }]);

        assert_eq!(
            items,
            vec![FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,abc".to_string(),
                detail: Some(DEFAULT_IMAGE_DETAIL),
            }]
        );
    }

    #[test]
    fn code_mode_image_detail_maps_to_protocol_detail() {
        let items =
            into_function_call_output_content_items(vec![CodeModeContentItem::InputImage {
                image_url: "data:image/png;base64,abc".to_string(),
                detail: Some(CodeModeImageDetail::High),
            }]);

        assert_eq!(
            items,
            vec![FunctionCallOutputContentItem::InputImage {
                image_url: "data:image/png;base64,abc".to_string(),
                detail: Some(ImageDetail::High),
            }]
        );
    }

    #[test]
    fn script_status_distinguishes_result_success_and_failure() {
        let success = RuntimeResponse::Result {
            cell_id: "cell".to_string(),
            content_items: Vec::new(),
            stored_values: Default::default(),
            error_text: None,
        };
        let failure = RuntimeResponse::Result {
            cell_id: "cell".to_string(),
            content_items: Vec::new(),
            stored_values: Default::default(),
            error_text: Some("boom".to_string()),
        };

        assert_eq!(format_script_status(&success), "Script completed");
        assert_eq!(format_script_status(&failure), "Script failed");
    }
}
