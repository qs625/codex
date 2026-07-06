use super::shared::camel_case_enum_from_core;
pub use mcp_types::McpElicitationArrayType;
pub use mcp_types::McpElicitationBooleanSchema;
pub use mcp_types::McpElicitationBooleanType;
pub use mcp_types::McpElicitationConstOption;
pub use mcp_types::McpElicitationEnumSchema;
pub use mcp_types::McpElicitationLegacyTitledEnumSchema;
pub use mcp_types::McpElicitationMultiSelectEnumSchema;
pub use mcp_types::McpElicitationNumberSchema;
pub use mcp_types::McpElicitationNumberType;
pub use mcp_types::McpElicitationObjectType;
pub use mcp_types::McpElicitationPrimitiveSchema;
pub use mcp_types::McpElicitationSchema;
pub use mcp_types::McpElicitationSingleSelectEnumSchema;
pub use mcp_types::McpElicitationStringFormat;
pub use mcp_types::McpElicitationStringSchema;
pub use mcp_types::McpElicitationStringType;
pub use mcp_types::McpElicitationTitledEnumItems;
pub use mcp_types::McpElicitationTitledMultiSelectEnumSchema;
pub use mcp_types::McpElicitationTitledSingleSelectEnumSchema;
pub use mcp_types::McpElicitationUntitledEnumItems;
pub use mcp_types::McpElicitationUntitledMultiSelectEnumSchema;
pub use mcp_types::McpElicitationUntitledSingleSelectEnumSchema;
pub use mcp_types::McpServerElicitationRequest;
pub use mcp_types::McpServerElicitationRequestParams;
use protocol::items::McpToolCallError as CoreMcpToolCallError;
use protocol::mcp::CallToolResult as CoreMcpCallToolResult;
use protocol::mcp::Resource as McpResource;
pub use protocol::mcp::ResourceContent as McpResourceContent;
use protocol::mcp::ResourceTemplate as McpResourceTemplate;
use protocol::mcp::Tool as McpTool;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;
#[cfg(feature = "schema-export")]
#[cfg(feature = "schema-export")]
use ts_rs::TS;

camel_case_enum_from_core!(
    pub enum McpAuthStatus from protocol::protocol::McpAuthStatus {
        Unsupported,
        NotLoggedIn,
        BearerToken,
        OAuth
    }
);

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ListMcpServerStatusParams {
    /// Opaque pagination cursor returned by a previous call.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub cursor: Option<String>,
    /// Optional page size; defaults to a server-defined value.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub limit: Option<u32>,
    /// Controls how much MCP inventory data to fetch for each server.
    /// Defaults to `Full` when omitted.
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub detail: Option<McpServerStatusDetail>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum McpServerStatusDetail {
    Full,
    ToolsAndAuthOnly,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerStatus {
    pub name: String,
    pub tools: std::collections::HashMap<String, McpTool>,
    pub resources: Vec<McpResource>,
    pub resource_templates: Vec<McpResourceTemplate>,
    pub auth_status: McpAuthStatus,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct ListMcpServerStatusResponse {
    pub data: Vec<McpServerStatus>,
    /// Opaque cursor to pass to the next call to continue after the last item.
    /// If None, there are no more items to return.
    pub next_cursor: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpResourceReadParams {
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub thread_id: Option<String>,
    pub server: String,
    pub uri: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpResourceReadResponse {
    pub contents: Vec<McpResourceContent>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerToolCallParams {
    pub thread_id: String,
    pub server: String,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub arguments: Option<JsonValue>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub meta: Option<JsonValue>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerToolCallResponse {
    pub content: Vec<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub structured_content: Option<JsonValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub is_error: Option<bool>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub meta: Option<JsonValue>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpToolCallResult {
    // NOTE: `rmcp::model::Content` (and its `RawContent` variants) would be a more precise Rust
    // representation of MCP content blocks. We intentionally use `serde_json::Value` here because
    // this crate exports JSON schema + TS types (`schemars`/`ts-rs`), and the rmcp model types
    // aren't set up to be schema/TS friendly (and would introduce heavier coupling to rmcp's Rust
    // representations). Using `JsonValue` keeps the payload wire-shaped and easy to export.
    pub content: Vec<JsonValue>,
    pub structured_content: Option<JsonValue>,
    #[serde(rename = "_meta")]
    #[cfg_attr(feature = "schema-export", ts(rename = "_meta"))]
    pub meta: Option<JsonValue>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpToolCallError {
    pub message: String,
}

impl From<CoreMcpCallToolResult> for McpServerToolCallResponse {
    fn from(result: CoreMcpCallToolResult) -> Self {
        Self {
            content: result.content,
            structured_content: result.structured_content,
            is_error: result.is_error,
            meta: result.meta,
        }
    }
}

impl From<CoreMcpCallToolResult> for McpToolCallResult {
    fn from(result: CoreMcpCallToolResult) -> Self {
        Self {
            content: result.content,
            structured_content: result.structured_content,
            meta: result.meta,
        }
    }
}

impl From<CoreMcpToolCallError> for McpToolCallError {
    fn from(error: CoreMcpToolCallError) -> Self {
        Self {
            message: error.message,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerRefreshParams {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerRefreshResponse {}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerOauthLoginParams {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub scopes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional = nullable))]
    pub timeout_secs: Option<i64>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerOauthLoginResponse {
    pub authorization_url: String,
}
#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpToolCallProgressNotification {
    pub thread_id: String,
    pub turn_id: String,
    pub item_id: String,
    pub message: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerOauthLoginCompletedNotification {
    pub name: String,
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-export", ts(optional))]
    pub error: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum McpServerStartupState {
    Starting,
    Ready,
    Failed,
    Cancelled,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerStatusUpdatedNotification {
    pub name: String,
    pub status: McpServerStartupState,
    pub error: Option<String>,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum McpServerElicitationAction {
    Accept,
    Decline,
    Cancel,
}

impl McpServerElicitationAction {
    pub fn to_core(self) -> protocol::approvals::ElicitationAction {
        match self {
            Self::Accept => protocol::approvals::ElicitationAction::Accept,
            Self::Decline => protocol::approvals::ElicitationAction::Decline,
            Self::Cancel => protocol::approvals::ElicitationAction::Cancel,
        }
    }
}

impl From<McpServerElicitationAction> for mcp_types::ElicitationAction {
    fn from(value: McpServerElicitationAction) -> Self {
        match value {
            McpServerElicitationAction::Accept => Self::Accept,
            McpServerElicitationAction::Decline => Self::Decline,
            McpServerElicitationAction::Cancel => Self::Cancel,
        }
    }
}

impl From<mcp_types::ElicitationAction> for McpServerElicitationAction {
    fn from(value: mcp_types::ElicitationAction) -> Self {
        match value {
            mcp_types::ElicitationAction::Accept => Self::Accept,
            mcp_types::ElicitationAction::Decline => Self::Decline,
            mcp_types::ElicitationAction::Cancel => Self::Cancel,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct McpServerElicitationRequestResponse {
    pub action: McpServerElicitationAction,
    /// Structured user input for accepted elicitations, mirroring RMCP `CreateElicitationResult`.
    ///
    /// This is nullable because decline/cancel responses have no content.
    pub content: Option<JsonValue>,
    /// Optional client metadata for form-mode action handling.
    #[serde(rename = "_meta")]
    #[cfg_attr(feature = "schema-export", ts(rename = "_meta"))]
    pub meta: Option<JsonValue>,
}

impl From<McpServerElicitationRequestResponse> for mcp_types::ElicitationResponse {
    fn from(value: McpServerElicitationRequestResponse) -> Self {
        Self {
            action: value.action.into(),
            content: value.content,
            meta: value.meta,
        }
    }
}

impl From<mcp_types::ElicitationResponse> for McpServerElicitationRequestResponse {
    fn from(value: mcp_types::ElicitationResponse) -> Self {
        Self {
            action: value.action.into(),
            content: value.content,
            meta: value.meta,
        }
    }
}
