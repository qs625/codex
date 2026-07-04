use codex_context_manager::truncate_function_output_payload;
use codex_utils_output_truncation::TruncationPolicy;
use codex_utils_output_truncation::formatted_truncate_text;
use codex_utils_string::take_bytes_at_char_boundary;
use command_service_api::resolve_max_tokens;
use protocol::exec_output::ExecToolCallOutput;
use protocol::mcp::CallToolResult;
use protocol::models::FunctionCallOutputBody;
use protocol::models::FunctionCallOutputContentItem;
use protocol::models::FunctionCallOutputPayload;
use protocol::models::ResponseInputItem;
use protocol::models::function_call_output_content_items_to_text;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::borrow::Cow;
use std::time::Duration;
use tool_config::sanitize_original_image_detail;
use tool_service_api::ToolName;
use tool_service_api::ToolOutput;
use tool_service_api::ToolPayload;

const TELEMETRY_PREVIEW_MAX_BYTES: usize = 2 * 1024;
const TELEMETRY_PREVIEW_MAX_LINES: usize = 64;
const TELEMETRY_PREVIEW_TRUNCATION_NOTICE: &str = "[... telemetry preview truncated ...]";

#[derive(Clone, Debug)]
pub(crate) struct McpToolOutput {
    pub(crate) result: CallToolResult,
    pub(crate) wall_time: Duration,
    pub(crate) original_image_detail_supported: bool,
    pub(crate) truncation_policy: TruncationPolicy,
}

impl ToolOutput for McpToolOutput {
    fn log_preview(&self) -> String {
        let payload = self.response_payload();
        let preview = payload.body.to_text().unwrap_or_else(|| {
            serde_json::to_string(&self.result.content)
                .unwrap_or_else(|err| format!("failed to serialize mcp result: {err}"))
        });
        telemetry_preview(&preview)
    }

    fn success_for_logging(&self) -> bool {
        self.result.success()
    }

    fn to_response_item(&self, call_id: &str, _payload: &ToolPayload) -> ResponseInputItem {
        ResponseInputItem::FunctionCallOutput {
            call_id: call_id.to_string(),
            output: self.response_payload(),
        }
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        serde_json::to_value(&self.result).unwrap_or_else(|err| {
            JsonValue::String(format!("failed to serialize mcp result: {err}"))
        })
    }
}

impl McpToolOutput {
    fn response_payload(&self) -> FunctionCallOutputPayload {
        let mut payload = self.result.as_function_call_output_payload();
        if let Some(items) = payload.content_items_mut() {
            sanitize_original_image_detail(self.original_image_detail_supported, items);
        }

        let header = format!(
            "Wall time: {:.4} seconds\nOutput:",
            self.wall_time.as_secs_f64()
        );
        match &mut payload.body {
            FunctionCallOutputBody::Text(text) => {
                if text.is_empty() {
                    *text = header;
                } else {
                    *text = format!("{header}\n{text}");
                }
            }
            FunctionCallOutputBody::ContentItems(items) => {
                items.insert(0, FunctionCallOutputContentItem::InputText { text: header });
            }
        }
        truncate_function_output_payload(&payload, self.truncation_policy * 1.2)
    }
}

pub(crate) struct FunctionToolOutput {
    pub(crate) body: Vec<FunctionCallOutputContentItem>,
    pub(crate) success: Option<bool>,
    pub(crate) post_tool_use_response: Option<JsonValue>,
}

impl FunctionToolOutput {
    pub(crate) fn from_text(text: String, success: Option<bool>) -> Self {
        Self {
            body: vec![FunctionCallOutputContentItem::InputText { text }],
            success,
            post_tool_use_response: None,
        }
    }

    pub(crate) fn from_content(
        body: Vec<FunctionCallOutputContentItem>,
        success: Option<bool>,
    ) -> Self {
        Self {
            body,
            success,
            post_tool_use_response: None,
        }
    }

    pub(crate) fn into_text(self) -> String {
        function_call_output_content_items_to_text(&self.body).unwrap_or_default()
    }
}

impl ToolOutput for FunctionToolOutput {
    fn log_preview(&self) -> String {
        telemetry_preview(
            &function_call_output_content_items_to_text(&self.body).unwrap_or_default(),
        )
    }

    fn success_for_logging(&self) -> bool {
        self.success.unwrap_or(true)
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        function_tool_response(call_id, payload, self.body.clone(), self.success)
    }

    fn post_tool_use_response(&self, _call_id: &str, _payload: &ToolPayload) -> Option<JsonValue> {
        self.post_tool_use_response.clone()
    }
}

pub(crate) struct ApplyPatchToolOutput {
    pub(crate) text: String,
}

impl ApplyPatchToolOutput {
    pub(crate) fn from_text(text: String) -> Self {
        Self { text }
    }
}

impl ToolOutput for ApplyPatchToolOutput {
    fn log_preview(&self) -> String {
        telemetry_preview(&self.text)
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        function_tool_response(
            call_id,
            payload,
            vec![FunctionCallOutputContentItem::InputText {
                text: self.text.clone(),
            }],
            Some(true),
        )
    }

    fn post_tool_use_response(&self, _call_id: &str, _payload: &ToolPayload) -> Option<JsonValue> {
        Some(JsonValue::String(self.text.clone()))
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        JsonValue::Object(serde_json::Map::new())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExecCommandToolOutput {
    pub(crate) event_call_id: String,
    pub(crate) chunk_id: String,
    pub(crate) wall_time: Duration,
    pub(crate) raw_output: Vec<u8>,
    pub(crate) max_output_tokens: Option<usize>,
    pub(crate) process_id: Option<i32>,
    pub(crate) exit_code: Option<i32>,
    pub(crate) original_token_count: Option<usize>,
    pub(crate) hook_command: Option<String>,
}

impl ToolOutput for ExecCommandToolOutput {
    fn log_preview(&self) -> String {
        telemetry_preview(&self.response_text())
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        function_tool_response(
            call_id,
            payload,
            vec![FunctionCallOutputContentItem::InputText {
                text: self.response_text(),
            }],
            Some(true),
        )
    }

    fn post_tool_use_response(&self, _call_id: &str, _payload: &ToolPayload) -> Option<JsonValue> {
        if self.process_id.is_some() || self.hook_command.is_none() {
            return None;
        }
        Some(JsonValue::String(self.truncated_output()))
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        #[derive(Serialize)]
        struct ExecCodeModeResult {
            #[serde(skip_serializing_if = "Option::is_none")]
            chunk_id: Option<String>,
            wall_time_seconds: f64,
            #[serde(skip_serializing_if = "Option::is_none")]
            exit_code: Option<i32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            command_id: Option<i32>,
            #[serde(skip_serializing_if = "Option::is_none")]
            original_token_count: Option<usize>,
            output: String,
        }

        serde_json::to_value(ExecCodeModeResult {
            chunk_id: (!self.chunk_id.is_empty()).then(|| self.chunk_id.clone()),
            wall_time_seconds: self.wall_time.as_secs_f64(),
            exit_code: self.exit_code,
            command_id: self.process_id,
            original_token_count: self.original_token_count,
            output: self.truncated_output(),
        })
        .unwrap_or_else(|err| JsonValue::String(format!("failed to serialize exec result: {err}")))
    }
}

impl ExecCommandToolOutput {
    pub(crate) fn truncated_output(&self) -> String {
        let text = String::from_utf8_lossy(&self.raw_output).to_string();
        let max_tokens = resolve_max_tokens(self.max_output_tokens);
        formatted_truncate_text(&text, TruncationPolicy::Tokens(max_tokens))
    }

    fn response_text(&self) -> String {
        let mut sections = Vec::new();
        if !self.chunk_id.is_empty() {
            sections.push(format!("Chunk ID: {}", self.chunk_id));
        }
        sections.push(format!(
            "Wall time: {:.4} seconds",
            self.wall_time.as_secs_f64()
        ));
        if let Some(exit_code) = self.exit_code {
            sections.push(format!("Process exited with code {exit_code}"));
        }
        if let Some(process_id) = self.process_id {
            sections.push(format!("Process running with command ID {process_id}"));
        }
        if let Some(original_token_count) = self.original_token_count {
            sections.push(format!("Original token count: {original_token_count}"));
        }
        sections.push("Output:".to_string());
        sections.push(self.truncated_output());
        sections.join("\n")
    }
}

pub(crate) fn flat_tool_name(tool_name: &ToolName) -> Cow<'_, str> {
    match tool_name.namespace.as_deref() {
        Some(namespace) => {
            let mut name = String::with_capacity(namespace.len() + tool_name.name.len());
            name.push_str(namespace);
            name.push_str(&tool_name.name);
            Cow::Owned(name)
        }
        None => Cow::Borrowed(tool_name.name.as_str()),
    }
}

pub(crate) fn format_exec_output_for_model_structured(
    exec_output: &ExecToolCallOutput,
    truncation_policy: TruncationPolicy,
) -> String {
    #[derive(Serialize)]
    struct ExecMetadata {
        exit_code: i32,
        duration_seconds: f32,
    }

    #[derive(Serialize)]
    struct ExecOutput<'a> {
        output: &'a str,
        metadata: ExecMetadata,
    }

    let payload = ExecOutput {
        output: &format_exec_output_str(exec_output, truncation_policy),
        metadata: ExecMetadata {
            exit_code: exec_output.exit_code,
            duration_seconds: ((exec_output.duration.as_secs_f32()) * 10.0).round() / 10.0,
        },
    };
    serde_json::to_string(&payload).expect("serialize exec output")
}

pub(crate) fn format_exec_output_str(
    exec_output: &ExecToolCallOutput,
    truncation_policy: TruncationPolicy,
) -> String {
    formatted_truncate_text(&build_content_with_timeout(exec_output), truncation_policy)
}

fn build_content_with_timeout(exec_output: &ExecToolCallOutput) -> String {
    if exec_output.timed_out {
        format!(
            "command timed out after {} milliseconds\n{}",
            exec_output.duration.as_millis(),
            exec_output.aggregated_output.text
        )
    } else {
        exec_output.aggregated_output.text.clone()
    }
}

fn function_tool_response(
    call_id: &str,
    payload: &ToolPayload,
    body: Vec<FunctionCallOutputContentItem>,
    success: Option<bool>,
) -> ResponseInputItem {
    let body = match body.as_slice() {
        [FunctionCallOutputContentItem::InputText { text }] => {
            FunctionCallOutputBody::Text(text.clone())
        }
        _ => FunctionCallOutputBody::ContentItems(body),
    };

    if matches!(payload, ToolPayload::Custom { .. }) {
        return ResponseInputItem::CustomToolCallOutput {
            call_id: call_id.to_string(),
            name: None,
            output: FunctionCallOutputPayload { body, success },
        };
    }

    ResponseInputItem::FunctionCallOutput {
        call_id: call_id.to_string(),
        output: FunctionCallOutputPayload { body, success },
    }
}

fn telemetry_preview(content: &str) -> String {
    let truncated_slice = take_bytes_at_char_boundary(content, TELEMETRY_PREVIEW_MAX_BYTES);
    let truncated_by_bytes = truncated_slice.len() < content.len();

    let mut preview = String::new();
    let mut lines_iter = truncated_slice.lines();
    for idx in 0..TELEMETRY_PREVIEW_MAX_LINES {
        match lines_iter.next() {
            Some(line) => {
                if idx > 0 {
                    preview.push('\n');
                }
                preview.push_str(line);
            }
            None => break,
        }
    }
    let truncated_by_lines = lines_iter.next().is_some();
    if !truncated_by_bytes && !truncated_by_lines {
        return content.to_string();
    }
    if preview.len() < truncated_slice.len()
        && truncated_slice
            .as_bytes()
            .get(preview.len())
            .is_some_and(|byte| *byte == b'\n')
    {
        preview.push('\n');
    }
    if !preview.is_empty() && !preview.ends_with('\n') {
        preview.push('\n');
    }
    preview.push_str(TELEMETRY_PREVIEW_TRUNCATION_NOTICE);
    preview
}
