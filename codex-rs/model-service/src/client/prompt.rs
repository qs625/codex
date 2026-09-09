use futures::Stream;
pub use model_service_api::ResponseEvent;
use protocol::config_types::Personality;
use protocol::error::ModelInputItemKind;
use protocol::error::ModelInputItemReference;
use protocol::error::Result;
use protocol::models::BaseInstructions;
use protocol::models::FunctionCallOutputBody;
use protocol::models::ResponseItem;
use protocol::models::model_context_quarantine_notice;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::HashSet;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tool_service_api::ToolSpec;

/// Review thread system prompt. Edit `model-client/review_prompt.md` to customize.
pub const REVIEW_PROMPT: &str = include_str!("../review_prompt.md");

// Centralized templates for review-related user messages
pub const REVIEW_EXIT_SUCCESS_TMPL: &str = include_str!("../templates/review/exit_success.xml");
pub const REVIEW_EXIT_INTERRUPTED_TMPL: &str =
    include_str!("../templates/review/exit_interrupted.xml");

/// API request payload for a single model turn
#[derive(Debug, Clone)]
pub struct Prompt {
    /// Conversation context input items.
    pub input: Vec<ResponseItem>,

    /// Tools available to the model, including additional tools sourced from
    /// external MCP servers.
    pub tools: Vec<ToolSpec>,

    /// Whether parallel tool calls are permitted for this prompt.
    pub parallel_tool_calls: bool,

    pub base_instructions: BaseInstructions,

    /// Optionally specify the personality of the model.
    pub personality: Option<Personality>,

    /// Optional the output schema for the model's response.
    pub output_schema: Option<Value>,

    /// Whether the Responses API should strictly validate `output_schema`.
    pub output_schema_strict: bool,
}

pub struct PromptBuildParams {
    pub input: Vec<ResponseItem>,
    pub tools: Vec<ToolSpec>,
    pub parallel_tool_calls: bool,
    pub base_instructions: BaseInstructions,
    pub personality: Option<Personality>,
    pub output_schema: Option<Value>,
    pub output_schema_strict: bool,
}

pub fn build_prompt(params: PromptBuildParams) -> Prompt {
    Prompt {
        input: params.input,
        tools: params.tools,
        parallel_tool_calls: params.parallel_tool_calls,
        base_instructions: params.base_instructions,
        personality: params.personality,
        output_schema: params.output_schema,
        output_schema_strict: params.output_schema_strict,
    }
}

#[derive(Debug)]
pub(crate) struct PreparedResponsesInput {
    pub(crate) items: Vec<ResponseItem>,
    pub(crate) sources: Vec<Option<ModelInputItemReference>>,
}

impl Default for Prompt {
    fn default() -> Self {
        Self {
            input: Vec::new(),
            tools: Vec::new(),
            parallel_tool_calls: false,
            base_instructions: BaseInstructions::default(),
            personality: None,
            output_schema: None,
            output_schema_strict: true,
        }
    }
}

impl Prompt {
    pub fn get_formatted_input(&self) -> Vec<ResponseItem> {
        self.get_formatted_responses_input().items
    }

    pub(crate) fn get_formatted_responses_input(&self) -> PreparedResponsesInput {
        let transaction_kinds = unique_transaction_kinds(&self.input);
        let mut prepared = self
            .input
            .iter()
            .cloned()
            .filter_map(|item| {
                let source = model_input_source(&item, &transaction_kinds);
                format_typed_response_item_for_provider(item).map(|item| (item, source))
            })
            .collect::<Vec<_>>();

        // when using the *Freeform* apply_patch tool specifically, tool outputs
        // should be structured text, not json. Do NOT reserialize when using
        // the Function tool - note that this differs from the check above for
        // instructions. We declare the result as a named variable for clarity.
        let is_freeform_apply_patch_tool_present = self.tools.iter().any(|tool| match tool {
            ToolSpec::Freeform(f) => f.name == "apply_patch",
            _ => false,
        });
        if is_freeform_apply_patch_tool_present {
            let mut items = prepared
                .iter()
                .map(|(item, _)| item.clone())
                .collect::<Vec<_>>();
            reserialize_shell_outputs(&mut items);
            for ((item, _), serialized) in prepared.iter_mut().zip(items) {
                *item = serialized;
            }
        }

        let (items, sources) = prepared.into_iter().unzip();
        PreparedResponsesInput { items, sources }
    }
}

fn format_typed_response_item_for_provider(item: ResponseItem) -> Option<ResponseItem> {
    match item {
        ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::WorkflowRunProgress { .. }
        | ResponseItem::ThreadGoalUpdate { .. } => None,
        ResponseItem::EventCommandEvent { event, .. } => Some(event.to_response_item()),
        ResponseItem::EventDrivenTool { trigger, .. } => Some(trigger.to_response_item()),
        ResponseItem::InterAgentCommunication { communication, .. } => {
            Some(communication.to_response_input_item().into())
        }
        ResponseItem::ModelContextQuarantine {
            target,
            reason,
            error_code,
            error_param,
        } => Some(model_context_quarantine_notice(
            &target,
            &reason,
            error_code.as_deref(),
            error_param.as_deref(),
        )),
        item => Some(item),
    }
}

fn unique_transaction_kinds(items: &[ResponseItem]) -> HashMap<String, Option<ModelInputItemKind>> {
    let mut kinds = HashMap::new();
    for item in items {
        let Some((call_id, kind)) = model_call_identity(item) else {
            continue;
        };
        if call_id.is_empty() {
            continue;
        }
        match kinds.entry(call_id.to_string()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(Some(kind));
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
    }
    kinds
}

fn model_input_source(
    item: &ResponseItem,
    transaction_kinds: &HashMap<String, Option<ModelInputItemKind>>,
) -> Option<ModelInputItemReference> {
    let (call_id, expected_kinds): (&str, &[ModelInputItemKind]) = match item {
        ResponseItem::FunctionCall { call_id, .. } => {
            (call_id, &[ModelInputItemKind::FunctionCall])
        }
        ResponseItem::ToolSearchCall {
            call_id: Some(call_id),
            ..
        } => (call_id, &[ModelInputItemKind::ToolSearchCall]),
        ResponseItem::CustomToolCall { call_id, .. } => {
            (call_id, &[ModelInputItemKind::CustomToolCall])
        }
        ResponseItem::LocalShellCall {
            call_id: Some(call_id),
            ..
        } => (call_id, &[ModelInputItemKind::LocalShellCall]),
        ResponseItem::FunctionCallOutput { call_id, .. } => (
            call_id,
            &[
                ModelInputItemKind::FunctionCall,
                ModelInputItemKind::LocalShellCall,
            ],
        ),
        ResponseItem::CustomToolCallOutput { call_id, .. } => {
            (call_id, &[ModelInputItemKind::CustomToolCall])
        }
        ResponseItem::ToolSearchOutput {
            call_id: Some(call_id),
            ..
        } => (call_id, &[ModelInputItemKind::ToolSearchCall]),
        _ => return None,
    };
    let kind = transaction_kinds.get(call_id).copied().flatten()?;
    if call_id.is_empty() || !expected_kinds.contains(&kind) {
        return None;
    }
    Some(ModelInputItemReference {
        kind,
        call_id: call_id.to_string(),
    })
}

fn model_call_identity(item: &ResponseItem) -> Option<(&str, ModelInputItemKind)> {
    match item {
        ResponseItem::FunctionCall { call_id, .. } => {
            Some((call_id, ModelInputItemKind::FunctionCall))
        }
        ResponseItem::ToolSearchCall {
            call_id: Some(call_id),
            ..
        } => Some((call_id, ModelInputItemKind::ToolSearchCall)),
        ResponseItem::CustomToolCall { call_id, .. } => {
            Some((call_id, ModelInputItemKind::CustomToolCall))
        }
        ResponseItem::LocalShellCall {
            call_id: Some(call_id),
            ..
        } => Some((call_id, ModelInputItemKind::LocalShellCall)),
        _ => None,
    }
}

fn reserialize_shell_outputs(items: &mut [ResponseItem]) {
    let mut shell_call_ids: HashSet<String> = HashSet::new();

    items.iter_mut().for_each(|item| match item {
        ResponseItem::LocalShellCall { call_id, id, .. } => {
            if let Some(identifier) = call_id.clone().or_else(|| id.clone()) {
                shell_call_ids.insert(identifier);
            }
        }
        ResponseItem::CustomToolCall {
            id: _,
            status: _,
            call_id,
            name,
            input: _,
        } => {
            if name == "apply_patch" {
                shell_call_ids.insert(call_id.clone());
            }
        }
        ResponseItem::FunctionCall { name, call_id, .. }
            if is_shell_tool_name(name) || name == "apply_patch" =>
        {
            shell_call_ids.insert(call_id.clone());
        }
        ResponseItem::FunctionCallOutput {
            call_id, output, ..
        }
        | ResponseItem::CustomToolCallOutput {
            call_id, output, ..
        } => {
            if shell_call_ids.remove(call_id)
                && let Some(structured) = output
                    .text_content()
                    .and_then(parse_structured_shell_output)
            {
                output.body = FunctionCallOutputBody::Text(structured);
            }
        }
        _ => {}
    })
}

fn is_shell_tool_name(name: &str) -> bool {
    name == "shell"
}

#[derive(Deserialize)]
struct ExecOutputJson {
    output: String,
    metadata: ExecOutputMetadataJson,
}

#[derive(Deserialize)]
struct ExecOutputMetadataJson {
    exit_code: i32,
    duration_seconds: f32,
}

fn parse_structured_shell_output(raw: &str) -> Option<String> {
    let parsed: ExecOutputJson = serde_json::from_str(raw).ok()?;
    Some(build_structured_output(&parsed))
}

fn build_structured_output(parsed: &ExecOutputJson) -> String {
    let mut sections = Vec::new();
    sections.push(format!("Exit code: {}", parsed.metadata.exit_code));
    sections.push(format!(
        "Wall time: {} seconds",
        parsed.metadata.duration_seconds
    ));

    let mut output = parsed.output.clone();
    if let Some((stripped, total_lines)) = strip_total_output_header(&parsed.output) {
        sections.push(format!("Total output lines: {total_lines}"));
        output = stripped.to_string();
    }

    sections.push("Output:".to_string());
    sections.push(output);

    sections.join("\n")
}

fn strip_total_output_header(output: &str) -> Option<(&str, u32)> {
    let after_prefix = output.strip_prefix("Total output lines: ")?;
    let (total_segment, remainder) = after_prefix.split_once('\n')?;
    let total_lines = total_segment.parse::<u32>().ok()?;
    let remainder = remainder.strip_prefix('\n').unwrap_or(remainder);
    Some((remainder, total_lines))
}

pub struct ResponseStream {
    pub(crate) rx_event: mpsc::Receiver<Result<ResponseEvent>>,
    /// Signals the mapper task that the consumer stopped polling before the
    /// provider stream reached its own terminal event.
    pub(crate) consumer_dropped: CancellationToken,
}

impl Stream for ResponseStream {
    type Item = Result<ResponseEvent>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx_event.poll_recv(cx)
    }
}

impl Drop for ResponseStream {
    fn drop(&mut self) {
        self.consumer_dropped.cancel();
    }
}

#[cfg(test)]
#[path = "client_common_tests.rs"]
mod tests;
