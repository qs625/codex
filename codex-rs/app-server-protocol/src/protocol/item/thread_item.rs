use crate::protocol::ExecPolicyAmendment;
use crate::protocol::McpToolCallError;
use crate::protocol::McpToolCallResult;
use crate::protocol::ThreadGoal;
use crate::protocol::ThreadGoalStatus;
use crate::protocol::UserInput;
use super::super::shared::camel_case_enum_from_core;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::event_command::EventCommandEventKind as CoreEventCommandEventKind;
use protocol::items::McpToolCallStatus as CoreMcpToolCallStatus;
use protocol::memory_citation::MemoryCitation as CoreMemoryCitation;
use protocol::memory_citation::MemoryCitationEntry as CoreMemoryCitationEntry;
use protocol::models::MessagePhase;
use protocol::models::ThreadGoalUpdateEventAction as CoreThreadGoalUpdateEventAction;
use protocol::models::ThreadGoalUpdateEventSource as CoreThreadGoalUpdateEventSource;
use protocol::models::WorkflowRunProgressEvent as CoreWorkflowRunProgressEvent;
use protocol::models::WorkflowRunProgressKind as CoreWorkflowRunProgressKind;
use protocol::openai_models::ReasoningEffort;
use protocol::parse_command::ParsedCommand as CoreParsedCommand;
use protocol::protocol::AgentStatus as CoreAgentStatus;
use protocol::protocol::ExecCommandSource as CoreExecCommandSource;
use protocol::protocol::ExecCommandStatus as CoreExecCommandStatus;
use protocol::protocol::InterAgentOperation as CoreInterAgentOperation;
use protocol::protocol::PatchApplyStatus as CorePatchApplyStatus;
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
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum CommandExecutionApprovalDecision {
    Accept,
    AcceptForSession,
    AcceptWithExecpolicyAmendment {
        execpolicy_amendment: ExecPolicyAmendment,
    },
    ApplyNetworkPolicyAmendment {
        network_policy_amendment: crate::protocol::NetworkPolicyAmendment,
    },
    Decline,
    Cancel,
}

impl From<protocol::protocol::ReviewDecision> for CommandExecutionApprovalDecision {
    fn from(value: protocol::protocol::ReviewDecision) -> Self {
        match value {
            protocol::protocol::ReviewDecision::Approved => Self::Accept,
            protocol::protocol::ReviewDecision::ApprovedExecpolicyAmendment {
                proposed_execpolicy_amendment,
            } => Self::AcceptWithExecpolicyAmendment {
                execpolicy_amendment: proposed_execpolicy_amendment.into(),
            },
            protocol::protocol::ReviewDecision::ApprovedForSession => Self::AcceptForSession,
            protocol::protocol::ReviewDecision::NetworkPolicyAmendment {
                network_policy_amendment,
            } => Self::ApplyNetworkPolicyAmendment {
                network_policy_amendment: network_policy_amendment.into(),
            },
            protocol::protocol::ReviewDecision::Abort => Self::Cancel,
            protocol::protocol::ReviewDecision::Denied
            | protocol::protocol::ReviewDecision::TimedOut => Self::Decline,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum FileChangeApprovalDecision {
    Accept,
    AcceptForSession,
    Decline,
    Cancel,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum CommandAction {
    Read {
        command: String,
        name: String,
        path: AbsolutePathBuf,
    },
    ListFiles {
        command: String,
        path: Option<String>,
    },
    Search {
        command: String,
        query: Option<String>,
        path: Option<String>,
    },
    Unknown {
        command: String,
    },
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct MemoryCitation {
    pub entries: Vec<MemoryCitationEntry>,
    pub thread_ids: Vec<String>,
}

impl From<CoreMemoryCitation> for MemoryCitation {
    fn from(value: CoreMemoryCitation) -> Self {
        Self {
            entries: value.entries.into_iter().map(Into::into).collect(),
            thread_ids: value.rollout_ids,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct MemoryCitationEntry {
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub note: String,
}

impl From<CoreMemoryCitationEntry> for MemoryCitationEntry {
    fn from(value: CoreMemoryCitationEntry) -> Self {
        Self {
            path: value.path,
            line_start: value.line_start,
            line_end: value.line_end,
            note: value.note,
        }
    }
}

impl CommandAction {
    pub fn into_core(self) -> CoreParsedCommand {
        match self {
            CommandAction::Read {
                command: cmd,
                name,
                path,
            } => CoreParsedCommand::Read {
                cmd,
                name,
                path: path.into_path_buf(),
            },
            CommandAction::ListFiles { command: cmd, path } => {
                CoreParsedCommand::ListFiles { cmd, path }
            }
            CommandAction::Search {
                command: cmd,
                query,
                path,
            } => CoreParsedCommand::Search { cmd, query, path },
            CommandAction::Unknown { command: cmd } => CoreParsedCommand::Unknown { cmd },
        }
    }

    pub fn from_core_with_cwd(value: CoreParsedCommand, cwd: &AbsolutePathBuf) -> Self {
        match value {
            CoreParsedCommand::Read { cmd, name, path } => CommandAction::Read {
                command: cmd,
                name,
                path: cwd.join(path),
            },
            CoreParsedCommand::ListFiles { cmd, path } => {
                CommandAction::ListFiles { command: cmd, path }
            }
            CoreParsedCommand::Search { cmd, query, path } => CommandAction::Search {
                command: cmd,
                query,
                path,
            },
            CoreParsedCommand::Unknown { cmd } => CommandAction::Unknown { command: cmd },
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum ThreadItem {
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    UserMessage { id: String, content: Vec<UserInput> },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    HookPrompt {
        id: String,
        fragments: Vec<HookPromptFragment>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    InjectedContext {
        id: String,
        title: String,
        preview: String,
        sections: Vec<InjectedContextSection>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    AgentMessage {
        id: String,
        text: String,
        #[serde(default)]
        phase: Option<MessagePhase>,
        #[serde(default)]
        memory_citation: Option<MemoryCitation>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    Plan { id: String, text: String },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    Reasoning {
        id: String,
        #[serde(default)]
        summary: Vec<String>,
        #[serde(default)]
        content: Vec<String>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    CommandExecution {
        id: String,
        command: String,
        cwd: AbsolutePathBuf,
        process_id: Option<String>,
        #[serde(default)]
        source: CommandExecutionSource,
        status: CommandExecutionStatus,
        #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
        initial_wait_ms: Option<i64>,
        notify_on: Option<CommandExecutionNotifyOn>,
        command_actions: Vec<CommandAction>,
        aggregated_output: Option<String>,
        exit_code: Option<i32>,
        #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
        duration_ms: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    CommandExecutionNotification {
        id: String,
        command_item_id: String,
        kind: CommandExecutionNotificationKind,
        message: String,
        output: Option<String>,
        exit_code: Option<i32>,
        #[cfg_attr(feature = "schema-export", ts(type = "number"))]
        created_at_ms: i64,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    CommandWait {
        id: String,
        command_id: String,
        status: CommandWaitStatus,
        notification: Option<CommandWaitNotificationKind>,
        exit_code: Option<i32>,
        wall_time_seconds: f64,
        #[cfg_attr(feature = "schema-export", ts(type = "number"))]
        wait_timeout_ms: i64,
        #[cfg_attr(feature = "schema-export", ts(type = "number"))]
        created_at_ms: i64,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    CommandWriteStdin {
        id: String,
        command_id: String,
        bytes_written: usize,
        contains_newline: bool,
        #[cfg_attr(feature = "schema-export", ts(type = "number"))]
        created_at_ms: i64,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    FileChange {
        id: String,
        changes: Vec<FileUpdateChange>,
        status: PatchApplyStatus,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    McpToolCall {
        id: String,
        server: String,
        tool: String,
        status: McpToolCallStatus,
        arguments: JsonValue,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "schema-export", ts(optional))]
        mcp_app_resource_uri: Option<String>,
        result: Option<Box<McpToolCallResult>>,
        error: Option<McpToolCallError>,
        #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
        duration_ms: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    BuiltinToolCall {
        id: String,
        tool: String,
        #[cfg_attr(feature = "schema-export", ts(type = "unknown"))]
        arguments: JsonValue,
        status: DynamicToolCallStatus,
        #[cfg_attr(feature = "schema-export", ts(type = "unknown | null"))]
        output: Option<JsonValue>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    DynamicToolCall {
        id: String,
        namespace: Option<String>,
        tool: String,
        #[cfg_attr(feature = "schema-export", ts(type = "unknown"))]
        arguments: JsonValue,
        status: DynamicToolCallStatus,
        content_items: Option<Vec<super::DynamicToolCallOutputContentItem>>,
        success: Option<bool>,
        #[cfg_attr(feature = "schema-export", ts(type = "number | null"))]
        duration_ms: Option<i64>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    EventDrivenToolCall {
        id: String,
        tool: String,
        #[cfg_attr(feature = "schema-export", ts(type = "unknown"))]
        arguments: JsonValue,
        status: DynamicToolCallStatus,
        #[cfg_attr(feature = "schema-export", ts(type = "unknown | null"))]
        output: Option<JsonValue>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    EventDrivenTool {
        id: String,
        tool: String,
        title: String,
        text: String,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    EventCommandCall {
        id: String,
        subscription_id: String,
        command: String,
        cwd: Option<String>,
        label: Option<String>,
        status: DynamicToolCallStatus,
        #[cfg_attr(feature = "schema-export", ts(type = "unknown | null"))]
        output: Option<JsonValue>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    EventCommandEvent {
        id: String,
        subscription_id: String,
        kind: EventCommandEventKind,
        label: Option<String>,
        command: String,
        cwd: Option<String>,
        line: Option<String>,
        sequence: Option<u32>,
        exit_code: Option<i32>,
        signal: Option<String>,
        message: Option<String>,
        truncated: bool,
        #[cfg_attr(feature = "schema-export", ts(type = "number"))]
        created_at: i64,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    WorkflowRunProgress {
        id: String,
        event: ThreadWorkflowRunProgressEvent,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    ThreadGoalUpdate {
        id: String,
        goal: ThreadGoal,
        action: ThreadGoalUpdateAction,
        source: ThreadGoalUpdateSource,
        previous_status: Option<ThreadGoalStatus>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    CollabAgentMessage {
        id: String,
        operation: CollabAgentOperation,
        sender_thread_id: Option<String>,
        sender_path: String,
        recipient_thread_id: Option<String>,
        recipient_path: String,
        other_recipient_paths: Vec<String>,
        content: String,
        trigger_turn: bool,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    CollabAgentToolCall {
        id: String,
        tool: CollabAgentTool,
        status: CollabAgentToolCallStatus,
        sender_thread_id: String,
        sender_path: String,
        receiver_thread_ids: Vec<String>,
        receiver_paths: Vec<String>,
        timeout_ms: Option<i64>,
        prompt: Option<String>,
        model: Option<String>,
        reasoning_effort: Option<ReasoningEffort>,
        agents_states: HashMap<String, CollabAgentState>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    CollabAgentStatusUpdate {
        id: String,
        sender_thread_id: Option<String>,
        sender_path: String,
        recipient_thread_id: Option<String>,
        recipient_path: String,
        status: CollabAgentState,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    WebSearch {
        id: String,
        query: String,
        action: Option<WebSearchAction>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    ImageView { id: String, path: AbsolutePathBuf },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    ImageGeneration {
        id: String,
        status: String,
        revised_prompt: Option<String>,
        result: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "schema-export", ts(optional))]
        saved_path: Option<AbsolutePathBuf>,
    },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    EnteredReviewMode { id: String, review: String },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    ExitedReviewMode { id: String, review: String },
    #[serde(rename_all = "camelCase")]
    #[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
    ContextCompaction {
        id: String,
        replacement_history: Option<JsonValue>,
    },
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct HookPromptFragment {
    pub text: String,
    pub hook_run_id: String,
}

impl From<protocol::items::HookPromptFragment> for HookPromptFragment {
    fn from(value: protocol::items::HookPromptFragment) -> Self {
        Self {
            text: value.text,
            hook_run_id: value.hook_run_id,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct InjectedContextSection {
    pub label: String,
    pub text: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum CollabAgentOperation {
    Unknown,
    SpawnAgent,
    SendMessage,
    FollowupTask,
    ChildCompletion,
}

impl From<CoreInterAgentOperation> for CollabAgentOperation {
    fn from(value: CoreInterAgentOperation) -> Self {
        match value {
            CoreInterAgentOperation::Unknown => Self::Unknown,
            CoreInterAgentOperation::SpawnAgent => Self::SpawnAgent,
            CoreInterAgentOperation::SendMessage => Self::SendMessage,
            CoreInterAgentOperation::FollowupTask => Self::FollowupTask,
            CoreInterAgentOperation::ChildCompletion => Self::ChildCompletion,
        }
    }
}

impl ThreadItem {
    pub fn id(&self) -> &str {
        match self {
            ThreadItem::UserMessage { id, .. }
            | ThreadItem::HookPrompt { id, .. }
            | ThreadItem::InjectedContext { id, .. }
            | ThreadItem::AgentMessage { id, .. }
            | ThreadItem::Plan { id, .. }
            | ThreadItem::Reasoning { id, .. }
            | ThreadItem::CommandExecution { id, .. }
            | ThreadItem::CommandExecutionNotification { id, .. }
            | ThreadItem::CommandWait { id, .. }
            | ThreadItem::CommandWriteStdin { id, .. }
            | ThreadItem::FileChange { id, .. }
            | ThreadItem::McpToolCall { id, .. }
            | ThreadItem::BuiltinToolCall { id, .. }
            | ThreadItem::DynamicToolCall { id, .. }
            | ThreadItem::EventDrivenToolCall { id, .. }
            | ThreadItem::EventDrivenTool { id, .. }
            | ThreadItem::EventCommandCall { id, .. }
            | ThreadItem::EventCommandEvent { id, .. }
            | ThreadItem::WorkflowRunProgress { id, .. }
            | ThreadItem::ThreadGoalUpdate { id, .. }
            | ThreadItem::CollabAgentMessage { id, .. }
            | ThreadItem::CollabAgentToolCall { id, .. }
            | ThreadItem::CollabAgentStatusUpdate { id, .. }
            | ThreadItem::WebSearch { id, .. }
            | ThreadItem::ImageView { id, .. }
            | ThreadItem::ImageGeneration { id, .. }
            | ThreadItem::EnteredReviewMode { id, .. }
            | ThreadItem::ExitedReviewMode { id, .. }
            | ThreadItem::ContextCompaction { id, .. } => id,
        }
    }
}

pub(crate) fn assistant_message_thread_item(
    id: String,
    text: String,
    phase: Option<MessagePhase>,
    memory_citation: Option<MemoryCitation>,
) -> ThreadItem {
    ThreadItem::AgentMessage {
        id,
        text,
        phase,
        memory_citation,
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub struct ThreadWorkflowRunProgressEvent {
    pub run_id: String,
    pub workflow_id: String,
    #[cfg_attr(feature = "schema-export", ts(type = "unknown"))]
    pub status: JsonValue,
    pub runner_status: String,
    pub kind: ThreadWorkflowRunProgressKind,
    pub message: String,
    #[cfg_attr(feature = "schema-export", ts(type = "number"))]
    pub updated_at: i64,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum ThreadWorkflowRunProgressKind {
    Started,
    Resumed,
    Completed,
    Failed,
    Aborted,
}

impl From<CoreWorkflowRunProgressEvent> for ThreadWorkflowRunProgressEvent {
    fn from(value: CoreWorkflowRunProgressEvent) -> Self {
        Self {
            run_id: value.run_id,
            workflow_id: value.workflow_id,
            status: value.status,
            runner_status: value.runner_status,
            kind: value.kind.into(),
            message: value.message,
            updated_at: value.updated_at,
        }
    }
}

impl From<CoreWorkflowRunProgressKind> for ThreadWorkflowRunProgressKind {
    fn from(value: CoreWorkflowRunProgressKind) -> Self {
        match value {
            CoreWorkflowRunProgressKind::Started => Self::Started,
            CoreWorkflowRunProgressKind::Resumed => Self::Resumed,
            CoreWorkflowRunProgressKind::Completed => Self::Completed,
            CoreWorkflowRunProgressKind::Failed => Self::Failed,
            CoreWorkflowRunProgressKind::Aborted => Self::Aborted,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", schemars(rename = "ThreadEventCommandEventKind"))]
#[cfg_attr(feature = "schema-export", ts(rename = "ThreadEventCommandEventKind"))]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum EventCommandEventKind {
    Output,
    Exited,
    Cancelled,
    FailedToStart,
}

impl From<CoreEventCommandEventKind> for EventCommandEventKind {
    fn from(value: CoreEventCommandEventKind) -> Self {
        match value {
            CoreEventCommandEventKind::Output => Self::Output,
            CoreEventCommandEventKind::Exited => Self::Exited,
            CoreEventCommandEventKind::Cancelled => Self::Cancelled,
            CoreEventCommandEventKind::FailedToStart => Self::FailedToStart,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(tag = "type", rename_all = "camelCase"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum WebSearchAction {
    Search {
        query: Option<String>,
        queries: Option<Vec<String>>,
    },
    OpenPage {
        url: Option<String>,
    },
    FindInPage {
        url: Option<String>,
        pattern: Option<String>,
    },
    #[serde(other)]
    Other,
}

impl From<protocol::models::WebSearchAction> for WebSearchAction {
    fn from(value: protocol::models::WebSearchAction) -> Self {
        match value {
            protocol::models::WebSearchAction::Search { query, queries } => {
                WebSearchAction::Search { query, queries }
            }
            protocol::models::WebSearchAction::OpenPage { url } => {
                WebSearchAction::OpenPage { url }
            }
            protocol::models::WebSearchAction::FindInPage { url, pattern } => {
                WebSearchAction::FindInPage { url, pattern }
            }
            protocol::models::WebSearchAction::Other => WebSearchAction::Other,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum CommandExecutionStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", ts(export))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum CommandExecutionNotifyOn {
    Output,
    Exit,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", ts(export))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum CommandExecutionNotificationKind {
    Output,
    Exit,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", ts(export))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum CommandWaitStatus {
    Running,
    Completed,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", ts(export))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum CommandWaitNotificationKind {
    Output,
    Exit,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", ts(export))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum ThreadGoalUpdateAction {
    Created,
    Updated,
    Paused,
    Resumed,
    BudgetLimited,
    Completed,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema-export", ts(export))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(rename_all = "camelCase"))]
pub enum ThreadGoalUpdateSource {
    ModelTool,
    Client,
    System,
}

impl From<CoreThreadGoalUpdateEventAction> for ThreadGoalUpdateAction {
    fn from(value: CoreThreadGoalUpdateEventAction) -> Self {
        match value {
            CoreThreadGoalUpdateEventAction::Created => Self::Created,
            CoreThreadGoalUpdateEventAction::Updated => Self::Updated,
            CoreThreadGoalUpdateEventAction::Paused => Self::Paused,
            CoreThreadGoalUpdateEventAction::Resumed => Self::Resumed,
            CoreThreadGoalUpdateEventAction::BudgetLimited => Self::BudgetLimited,
            CoreThreadGoalUpdateEventAction::Completed => Self::Completed,
        }
    }
}

impl From<CoreThreadGoalUpdateEventSource> for ThreadGoalUpdateSource {
    fn from(value: CoreThreadGoalUpdateEventSource) -> Self {
        match value {
            CoreThreadGoalUpdateEventSource::ModelTool => Self::ModelTool,
            CoreThreadGoalUpdateEventSource::Client => Self::Client,
            CoreThreadGoalUpdateEventSource::System => Self::System,
        }
    }
}

impl From<CoreExecCommandStatus> for CommandExecutionStatus {
    fn from(value: CoreExecCommandStatus) -> Self {
        Self::from(&value)
    }
}

impl From<&CoreExecCommandStatus> for CommandExecutionStatus {
    fn from(value: &CoreExecCommandStatus) -> Self {
        match value {
            CoreExecCommandStatus::Completed => CommandExecutionStatus::Completed,
            CoreExecCommandStatus::Failed => CommandExecutionStatus::Failed,
            CoreExecCommandStatus::Declined => CommandExecutionStatus::Declined,
        }
    }
}

camel_case_enum_from_core! {
    #[derive(Default)]
    pub enum CommandExecutionSource from CoreExecCommandSource {
        #[default]
        Agent,
        UserShell,
        UnifiedExecStartup,
        UnifiedExecInteraction,
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum CollabAgentTool {
    SpawnAgent,
    SendInput,
    ResumeAgent,
    Wait,
    ListAgents,
    CloseAgent,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct FileUpdateChange {
    pub path: String,
    pub kind: PatchChangeKind,
    pub diff: String,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(tag = "type"))]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum PatchChangeKind {
    Add,
    Delete,
    Update { move_path: Option<PathBuf> },
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum PatchApplyStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

impl From<CorePatchApplyStatus> for PatchApplyStatus {
    fn from(value: CorePatchApplyStatus) -> Self {
        Self::from(&value)
    }
}

impl From<&CorePatchApplyStatus> for PatchApplyStatus {
    fn from(value: &CorePatchApplyStatus) -> Self {
        match value {
            CorePatchApplyStatus::Completed => PatchApplyStatus::Completed,
            CorePatchApplyStatus::Failed => PatchApplyStatus::Failed,
            CorePatchApplyStatus::Declined => PatchApplyStatus::Declined,
        }
    }
}

impl From<CoreMcpToolCallStatus> for McpToolCallStatus {
    fn from(value: CoreMcpToolCallStatus) -> Self {
        match value {
            CoreMcpToolCallStatus::InProgress => McpToolCallStatus::InProgress,
            CoreMcpToolCallStatus::Completed => McpToolCallStatus::Completed,
            CoreMcpToolCallStatus::Failed => McpToolCallStatus::Failed,
        }
    }
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum McpToolCallStatus {
    InProgress,
    Completed,
    Failed,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum DynamicToolCallStatus {
    InProgress,
    Completed,
    Failed,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum CollabAgentToolCallStatus {
    InProgress,
    Completed,
    Failed,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub enum CollabAgentStatus {
    PendingInit,
    Running,
    Interrupted,
    Completed,
    Errored,
    Shutdown,
    NotFound,
}

#[cfg_attr(feature = "schema-export", derive(JsonSchema, TS))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "schema-export", ts(export))]
pub struct CollabAgentState {
    pub path: Option<String>,
    pub status: CollabAgentStatus,
    pub message: Option<String>,
}

impl From<CoreAgentStatus> for CollabAgentState {
    fn from(value: CoreAgentStatus) -> Self {
        match value {
            CoreAgentStatus::PendingInit => Self {
                path: None,
                status: CollabAgentStatus::PendingInit,
                message: None,
            },
            CoreAgentStatus::Running => Self {
                path: None,
                status: CollabAgentStatus::Running,
                message: None,
            },
            CoreAgentStatus::Interrupted => Self {
                path: None,
                status: CollabAgentStatus::Interrupted,
                message: None,
            },
            CoreAgentStatus::Completed(message) => Self {
                path: None,
                status: CollabAgentStatus::Completed,
                message,
            },
            CoreAgentStatus::Errored(message) => Self {
                path: None,
                status: CollabAgentStatus::Errored,
                message: Some(message),
            },
            CoreAgentStatus::Shutdown => Self {
                path: None,
                status: CollabAgentStatus::Shutdown,
                message: None,
            },
            CoreAgentStatus::NotFound => Self {
                path: None,
                status: CollabAgentStatus::NotFound,
                message: None,
            },
        }
    }
}
