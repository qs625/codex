use protocol::models::ContentItem;
use protocol::models::MessagePhase;
use protocol::models::ResponseItem;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;

#[derive(Debug, Serialize, PartialEq)]
pub struct ArcMonitorRequest {
    pub metadata: ArcMonitorMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<ArcMonitorChatMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<ResponseItem>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policies: Option<ArcMonitorPolicies>,
    pub action: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcMonitorResult {
    pub outcome: ArcMonitorResultOutcome,
    pub short_reason: String,
    pub rationale: String,
    pub risk_score: u8,
    pub risk_level: ArcMonitorRiskLevel,
    pub evidence: Vec<ArcMonitorEvidence>,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ArcMonitorChatMessage {
    pub role: String,
    pub content: Value,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct ArcMonitorPolicies {
    pub user: Option<String>,
    pub developer: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ArcMonitorMetadata {
    pub codex_thread_id: String,
    pub codex_turn_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protection_client_callsite: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArcMonitorEvidence {
    pub message: String,
    pub why: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArcMonitorResultOutcome {
    Ok,
    SteerModel,
    AskUser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArcMonitorRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

pub fn build_arc_monitor_request(
    codex_thread_id: String,
    codex_turn_id: String,
    protection_client_callsite: Option<String>,
    history_items: &[ResponseItem],
    action: Map<String, Value>,
    is_contextual_user_message: impl Fn(&[ContentItem]) -> bool,
) -> ArcMonitorRequest {
    let mut messages = build_arc_monitor_messages(history_items, is_contextual_user_message);
    if messages.is_empty() {
        messages.push(build_arc_monitor_message(
            "user",
            Value::String(
                "No prior conversation history is available for this ARC evaluation.".to_string(),
            ),
        ));
    }

    ArcMonitorRequest {
        metadata: ArcMonitorMetadata {
            codex_thread_id: codex_thread_id.clone(),
            codex_turn_id,
            conversation_id: Some(codex_thread_id),
            protection_client_callsite,
        },
        messages: Some(messages),
        input: None,
        policies: Some(ArcMonitorPolicies {
            user: None,
            developer: None,
        }),
        action,
    }
}

fn build_arc_monitor_messages(
    items: &[ResponseItem],
    is_contextual_user_message: impl Fn(&[ContentItem]) -> bool,
) -> Vec<ArcMonitorChatMessage> {
    let last_tool_call_index = items
        .iter()
        .enumerate()
        .rev()
        .find(|(_, item)| {
            matches!(
                item,
                ResponseItem::LocalShellCall { .. }
                    | ResponseItem::FunctionCall { .. }
                    | ResponseItem::CustomToolCall { .. }
                    | ResponseItem::WebSearchCall { .. }
            )
        })
        .map(|(index, _)| index);
    let last_encrypted_reasoning_index = items
        .iter()
        .enumerate()
        .rev()
        .find(|(_, item)| {
            matches!(
                item,
                ResponseItem::Reasoning {
                    encrypted_content: Some(encrypted_content),
                    ..
                } if !encrypted_content.trim().is_empty()
            )
        })
        .map(|(index, _)| index);

    items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| {
            build_arc_monitor_message_item(
                item,
                index,
                last_tool_call_index,
                last_encrypted_reasoning_index,
                &is_contextual_user_message,
            )
        })
        .collect()
}

fn build_arc_monitor_message_item(
    item: &ResponseItem,
    index: usize,
    last_tool_call_index: Option<usize>,
    last_encrypted_reasoning_index: Option<usize>,
    is_contextual_user_message: &impl Fn(&[ContentItem]) -> bool,
) -> Option<ArcMonitorChatMessage> {
    match item {
        ResponseItem::Message { role, content, .. } if role == "user" => {
            if is_contextual_user_message(content) {
                None
            } else {
                content_items_to_text(content)
                    .map(|text| build_arc_monitor_text_message("user", "input_text", text))
            }
        }
        ResponseItem::Message {
            role,
            content,
            phase: Some(MessagePhase::FinalAnswer),
            ..
        } if role == "assistant" => content_items_to_text(content)
            .map(|text| build_arc_monitor_text_message("assistant", "output_text", text)),
        ResponseItem::Message { .. } => None,
        ResponseItem::Reasoning {
            encrypted_content: Some(encrypted_content),
            ..
        } if Some(index) == last_encrypted_reasoning_index
            && !encrypted_content.trim().is_empty() =>
        {
            Some(build_arc_monitor_message(
                "assistant",
                serde_json::json!([{
                    "type": "encrypted_reasoning",
                    "encrypted_content": encrypted_content,
                }]),
            ))
        }
        ResponseItem::Reasoning { .. } => None,
        ResponseItem::LocalShellCall { action, .. } if Some(index) == last_tool_call_index => {
            Some(build_arc_monitor_message(
                "assistant",
                serde_json::json!([{
                    "type": "tool_call",
                    "tool_name": "shell",
                    "action": action,
                }]),
            ))
        }
        ResponseItem::FunctionCall {
            name, arguments, ..
        } if Some(index) == last_tool_call_index => Some(build_arc_monitor_message(
            "assistant",
            serde_json::json!([{
                "type": "tool_call",
                "tool_name": name,
                "arguments": arguments,
            }]),
        )),
        ResponseItem::CustomToolCall { name, input, .. } if Some(index) == last_tool_call_index => {
            Some(build_arc_monitor_message(
                "assistant",
                serde_json::json!([{
                    "type": "tool_call",
                    "tool_name": name,
                    "input": input,
                }]),
            ))
        }
        ResponseItem::WebSearchCall { action, .. } if Some(index) == last_tool_call_index => {
            Some(build_arc_monitor_message(
                "assistant",
                serde_json::json!([{
                    "type": "tool_call",
                    "tool_name": "web_search",
                    "action": action,
                }]),
            ))
        }
        ResponseItem::LocalShellCall { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::WorkflowRunProgress { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::EventCommandEvent { .. }
        | ResponseItem::EventDrivenTool { .. }
        | ResponseItem::ThreadGoalUpdate { .. }
        | ResponseItem::InterAgentCommunication { .. }
        | ResponseItem::Compaction { .. }
        | ResponseItem::ContextCompaction { .. }
        | ResponseItem::Other => None,
    }
}

fn content_items_to_text(content: &[ContentItem]) -> Option<String> {
    let mut pieces = Vec::new();
    for item in content {
        match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                if !text.is_empty() {
                    pieces.push(text.as_str());
                }
            }
            ContentItem::InputImage { .. } => {}
        }
    }
    if pieces.is_empty() {
        None
    } else {
        Some(pieces.join("\n"))
    }
}

fn build_arc_monitor_text_message(
    role: &str,
    part_type: &str,
    text: String,
) -> ArcMonitorChatMessage {
    build_arc_monitor_message(
        role,
        serde_json::json!([{
            "type": part_type,
            "text": text,
        }]),
    )
}

fn build_arc_monitor_message(role: &str, content: Value) -> ArcMonitorChatMessage {
    ArcMonitorChatMessage {
        role: role.to_string(),
        content,
    }
}
