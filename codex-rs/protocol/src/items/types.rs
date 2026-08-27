use crate::event_command::EventCommandEvent;
use crate::mcp::CallToolResult;
use crate::memory_citation::MemoryCitation;
use crate::models::MessagePhase;
use crate::models::ResponseItem;
use crate::models::WebSearchAction;
use crate::protocol::FileChange;
use crate::protocol::InterAgentCommunication;
use crate::protocol::PatchApplyStatus;
use crate::user_input::UserInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use ts_rs::TS;

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema)]
#[serde(tag = "type")]
#[ts(tag = "type")]
pub enum TurnItem {
    UserMessage(UserMessageItem),
    HookPrompt(HookPromptItem),
    InjectedContext(InjectedContextItem),
    AgentMessage(AgentMessageItem),
    EventDrivenTool(EventDrivenToolItem),
    EventCommandEvent(EventCommandEventItem),
    CollabAgentMessage(CollabAgentMessageItem),
    ConversationArtifact(ConversationArtifactItem),
    Plan(PlanItem),
    Reasoning(ReasoningItem),
    WebSearch(WebSearchItem),
    ImageView(ImageViewItem),
    ImageGeneration(ImageGenerationItem),
    FileChange(FileChangeItem),
    McpToolCall(McpToolCallItem),
    ContextCompaction(ContextCompactionItem),
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema)]
pub struct UserMessageItem {
    pub id: String,
    pub content: Vec<UserInput>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
pub struct HookPromptItem {
    pub id: String,
    pub fragments: Vec<HookPromptFragment>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct HookPromptFragment {
    pub text: String,
    pub hook_run_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct InjectedContextItem {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub sections: Vec<InjectedContextSection>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct InjectedContextSection {
    pub label: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema)]
#[serde(tag = "type")]
#[ts(tag = "type")]
pub enum AgentMessageContent {
    Text { text: String },
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema)]
/// Assistant-authored message payload used in turn-item streams.
///
/// `phase` is optional because not all providers/models emit it. Consumers
/// should use it when present, but retain legacy completion semantics when it
/// is `None`.
pub struct AgentMessageItem {
    pub id: String,
    pub content: Vec<AgentMessageContent>,
    /// Optional phase metadata carried through from `ResponseItem::Message`.
    ///
    /// This is currently used by TUI rendering to distinguish mid-turn
    /// commentary from a final answer and avoid status-indicator jitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub phase: Option<MessagePhase>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub memory_citation: Option<MemoryCitation>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EventDrivenToolItem {
    pub id: String,
    pub tool: String,
    pub title: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct EventCommandEventItem {
    pub id: String,
    pub event: EventCommandEvent,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct CollabAgentMessageItem {
    pub id: String,
    pub communication: InterAgentCommunication,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct ConversationArtifactItem {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub source: Option<ConversationArtifactSource>,
    pub mime_type: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum ConversationArtifactSource {
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Inline {
        content: String,
        mime_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        language: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        truncated: bool,
    },
    #[serde(rename_all = "camelCase")]
    #[ts(rename_all = "camelCase")]
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        fallback_content: Option<String>,
    },
}

impl ConversationArtifactItem {
    pub fn inline_source(&self) -> ConversationArtifactSource {
        ConversationArtifactSource::Inline {
            content: self.content.clone(),
            mime_type: self.mime_type.clone(),
            language: self.language.clone(),
            truncated: self.truncated,
        }
    }

    pub fn resolved_source(&self) -> ConversationArtifactSource {
        self.source.clone().unwrap_or_else(|| self.inline_source())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema)]
pub struct PlanItem {
    pub id: String,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema)]
pub struct ReasoningItem {
    pub id: String,
    pub summary_text: Vec<String>,
    #[serde(default)]
    pub raw_content: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq)]
pub struct WebSearchItem {
    pub id: String,
    pub query: String,
    pub action: WebSearchAction,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq)]
pub struct ImageViewItem {
    pub id: String,
    pub path: AbsolutePathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq)]
pub struct ImageGenerationItem {
    pub id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub revised_prompt: Option<String>,
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub saved_path: Option<AbsolutePathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq)]
pub struct FileChangeItem {
    pub id: String,
    pub changes: HashMap<PathBuf, FileChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<PatchApplyStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub auto_approved: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub stderr: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct McpToolCallItem {
    pub id: String,
    pub server: String,
    pub tool: String,
    pub arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mcp_app_resource_uri: Option<String>,
    pub status: McpToolCallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub result: Option<CallToolResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub error: Option<McpToolCallError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "string", optional)]
    pub duration: Option<Duration>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub enum McpToolCallStatus {
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[ts(rename_all = "camelCase")]
pub struct McpToolCallError {
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema)]
pub struct ContextCompactionItem {
    pub id: String,
    #[serde(rename = "replacementHistory", alias = "replacement_history")]
    #[serde(
        default,
        deserialize_with = "deserialize_context_compaction_replacement_history"
    )]
    #[ts(rename = "replacementHistory")]
    pub replacement_history: Vec<ContextCompactionReplacementItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize, TS, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(tag = "type", rename_all = "camelCase")]
pub enum ContextCompactionReplacementItem {
    InjectedContext(InjectedContextItem),
    UserMessage(UserMessageItem),
    AgentMessage(AgentMessageItem),
    ConversationArtifact(ConversationArtifactItem),
}

fn deserialize_context_compaction_replacement_history<'de, D>(
    deserializer: D,
) -> Result<Vec<ContextCompactionReplacementItem>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value::<Vec<ContextCompactionReplacementItem>>(value.clone())
        .or_else(|_| {
            serde_json::from_value::<Vec<ResponseItem>>(value)
                .map(context_compaction_replacement_items_from_response_items)
        })
        .map_err(serde::de::Error::custom)
}

pub fn context_compaction_replacement_items_from_response_items(
    items: Vec<ResponseItem>,
) -> Vec<ContextCompactionReplacementItem> {
    items
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| {
            context_compaction_replacement_item_from_response_item(index, item)
        })
        .collect()
}

fn context_compaction_replacement_item_from_response_item(
    index: usize,
    item: ResponseItem,
) -> Option<ContextCompactionReplacementItem> {
    let id = format!("replacement-{index}");
    match item {
        ResponseItem::Message { role, content, .. } if role == "user" => Some(
            ContextCompactionReplacementItem::UserMessage(UserMessageItem {
                id,
                content: content
                    .into_iter()
                    .filter_map(|item| match item {
                        crate::models::ContentItem::InputText { text } => {
                            Some(crate::user_input::UserInput::Text {
                                text,
                                text_elements: Vec::new(),
                            })
                        }
                        crate::models::ContentItem::InputImage { image_url, .. } => {
                            Some(crate::user_input::UserInput::Image { image_url })
                        }
                        _ => None,
                    })
                    .collect(),
            }),
        ),
        ResponseItem::Message {
            role,
            content,
            phase,
            ..
        } if role == "assistant" => Some(ContextCompactionReplacementItem::AgentMessage(
            AgentMessageItem {
                id,
                content: content
                    .into_iter()
                    .filter_map(|item| match item {
                        crate::models::ContentItem::OutputText { text } => {
                            Some(AgentMessageContent::Text { text })
                        }
                        _ => None,
                    })
                    .collect(),
                phase,
                memory_citation: None,
            },
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn old_flat_conversation_artifact_resolves_to_inline_source() {
        let item: ConversationArtifactItem = serde_json::from_value(json!({
            "id": "artifact-1",
            "title": "Inline page",
            "mimeType": "text/html",
            "content": "<main>Hello</main>",
            "language": "html",
            "truncated": false
        }))
        .expect("old flat artifact should deserialize");

        assert_eq!(item.source, None);
        assert_eq!(
            item.resolved_source(),
            ConversationArtifactSource::Inline {
                content: "<main>Hello</main>".to_string(),
                mime_type: "text/html".to_string(),
                language: Some("html".to_string()),
                truncated: false,
            }
        );
    }

    #[test]
    fn conversation_artifact_source_uses_camel_case_wire_fields() {
        let item: ConversationArtifactItem = serde_json::from_value(json!({
            "id": "artifact-1",
            "title": "Preview",
            "source": {
                "type": "url",
                "url": "http://localhost:5173/",
                "mimeType": "text/html",
                "fallbackContent": "Open preview"
            },
            "mimeType": "text/uri-list",
            "content": "http://localhost:5173/"
        }))
        .expect("source union artifact should deserialize");

        assert_eq!(
            item.source,
            Some(ConversationArtifactSource::Url {
                url: "http://localhost:5173/".to_string(),
                mime_type: Some("text/html".to_string()),
                fallback_content: Some("Open preview".to_string()),
            })
        );
    }
}
