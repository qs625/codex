use crate::protocol::AdditionalPermissionProfile;
use super::CommandAction;
use super::CommandExecutionApprovalDecision;
use crate::protocol::ExecPolicyAmendment;
use super::FileChangeApprovalDecision;
use crate::protocol::NetworkApprovalContext;
use crate::protocol::NetworkPolicyAmendment;
use codex_experimental_api_macros::ExperimentalApi;
use codex_utils_absolute_path::AbsolutePathBuf;
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(feature = "schema-export")]
use ts_rs::TS;

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, ExperimentalApi)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecutionRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub started_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub approval_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub network_approval_context: Option<NetworkApprovalContext>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cwd: Option<AbsolutePathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub command_actions: Option<Vec<CommandAction>>,
    #[experimental("item/commandExecution/requestApproval.additionalPermissions")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub additional_permissions: Option<AdditionalPermissionProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub proposed_execpolicy_amendment: Option<ExecPolicyAmendment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub proposed_network_policy_amendments: Option<Vec<NetworkPolicyAmendment>>,
    #[experimental("item/commandExecution/requestApproval.availableDecisions")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub available_decisions: Option<Vec<CommandExecutionApprovalDecision>>,
}

impl CommandExecutionRequestApprovalParams {
    pub fn strip_experimental_fields(&mut self) {
        self.additional_permissions = None;
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CommandExecutionRequestApprovalResponse {
    pub decision: CommandExecutionApprovalDecision,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct FileChangeRequestApprovalParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub started_at_ms: i64,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub reason: Option<String>,
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub grant_root: Option<PathBuf>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct FileChangeRequestApprovalResponse {
    pub decision: FileChangeApprovalDecision,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct DynamicToolCallParams {
    pub thread_id: String,
    pub turn_id: String,
    pub call_id: String,
    pub namespace: Option<String>,
    pub tool: String,
    pub arguments: JsonValue,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct DynamicToolCallResponse {
    pub content_items: Vec<DynamicToolCallOutputContentItem>,
    pub success: bool,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum DynamicToolCallOutputContentItem {
    #[serde(rename_all = "camelCase")]
    InputText { text: String },
    #[serde(rename_all = "camelCase")]
    InputImage { image_url: String },
}

impl From<DynamicToolCallOutputContentItem>
    for protocol::dynamic_tools::DynamicToolCallOutputContentItem
{
    fn from(item: DynamicToolCallOutputContentItem) -> Self {
        match item {
            DynamicToolCallOutputContentItem::InputText { text } => Self::InputText { text },
            DynamicToolCallOutputContentItem::InputImage { image_url } => {
                Self::InputImage { image_url }
            }
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ToolRequestUserInputOption {
    pub label: String,
    pub description: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ToolRequestUserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub is_other: bool,
    #[serde(default)]
    pub is_secret: bool,
    pub options: Option<Vec<ToolRequestUserInputOption>>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ToolRequestUserInputParams {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub questions: Vec<ToolRequestUserInputQuestion>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ToolRequestUserInputAnswer {
    pub answers: Vec<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ToolRequestUserInputResponse {
    pub answers: HashMap<String, ToolRequestUserInputAnswer>,
}
