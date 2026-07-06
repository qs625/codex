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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, rename = "replacementHistory")]
    pub replacement_history: Option<Vec<ResponseItem>>,
}
