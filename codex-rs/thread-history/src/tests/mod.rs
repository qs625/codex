use super::*;
use app_server_protocol::CollabAgentState;
use app_server_protocol::CollabAgentStatus;
use app_server_protocol::CollabAgentTool;
use app_server_protocol::CollabAgentToolCallStatus;
use app_server_protocol::CommandExecutionStatus;
use app_server_protocol::CommandExecutionNotificationKind;
use app_server_protocol::CommandExecutionNotifyOn;
use app_server_protocol::CommandExecutionSource;
use app_server_protocol::DynamicToolCallStatus;
use app_server_protocol::InjectedContextSection;
use app_server_protocol::McpToolCallError;
use app_server_protocol::McpToolCallResult;
use app_server_protocol::McpToolCallStatus;
use app_server_protocol::ThreadItem;
use app_server_protocol::TurnStatus;
use app_server_protocol::UserInput;
use app_server_protocol::WebSearchAction;
use codex_utils_absolute_path::test_support::PathBufExt;
use codex_utils_absolute_path::test_support::test_path_buf;
use protocol::AgentPath;
use protocol::ThreadId;
use protocol::dynamic_tools::DynamicToolCallOutputContentItem as CoreDynamicToolCallOutputContentItem;
use protocol::event_command::EventCommandEvent;
use protocol::event_driven_tool::EventDrivenToolTrigger;
use protocol::items::AgentMessageContent as CoreAgentMessageContent;
use protocol::items::AgentMessageItem as CoreAgentMessageItem;
use protocol::items::CollabAgentMessageItem as CoreCollabAgentMessageItem;
use protocol::items::EventCommandEventItem as CoreEventCommandEventItem;
use protocol::items::EventDrivenToolItem as CoreEventDrivenToolItem;
use protocol::items::InjectedContextItem as CoreInjectedContextItem;
use protocol::items::InjectedContextSection as CoreInjectedContextSection;
use protocol::items::TurnItem as CoreTurnItem;
use protocol::items::UserMessageItem as CoreUserMessageItem;
use protocol::mcp::CallToolResult;
use protocol::models::ContentItem;
use protocol::models::FunctionCallOutputPayload;
use protocol::models::MessagePhase as CoreMessagePhase;
use protocol::models::ResponseItem;
use protocol::models::WebSearchAction as CoreWebSearchAction;
use protocol::parse_command::ParsedCommand;
use protocol::protocol::AgentMessageEvent;
use protocol::protocol::AgentStatus;
use protocol::protocol::AgentReasoningEvent;
use protocol::protocol::AgentReasoningRawContentEvent;
use protocol::protocol::ApplyPatchApprovalRequestEvent;
use protocol::protocol::AskForApproval;
use protocol::protocol::CodexErrorInfo;
use protocol::protocol::CommandExecutionNotificationDisplayEvent;
use protocol::protocol::CompactedItem;
use protocol::protocol::ContextCompactedEvent;
use protocol::protocol::DynamicToolCallResponseEvent;
use protocol::protocol::ErrorEvent;
use protocol::protocol::ExecCommandEndEvent;
use protocol::protocol::ExecCommandSource;
use protocol::protocol::GuardianAssessmentEvent;
use protocol::protocol::GuardianAssessmentStatus;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use protocol::protocol::ItemCompletedEvent;
use protocol::protocol::ItemStartedEvent;
use protocol::protocol::ImageGenerationEndEvent;
use protocol::protocol::McpInvocation;
use protocol::protocol::McpToolCallEndEvent;
use protocol::protocol::PatchApplyBeginEvent;
use protocol::protocol::PatchApplyEndEvent;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::ThreadRolledBackEvent;
use protocol::protocol::TurnAbortReason;
use protocol::protocol::TurnAbortedEvent;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::TurnContextItem;
use protocol::protocol::TurnStartedEvent;
use protocol::protocol::UserMessageEvent;
use protocol::protocol::UserMessageSkill;
use protocol::protocol::WebSearchEndEvent;
use protocol::models::MessagePhase;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;
use uuid::Uuid;

fn turn_context_item_with_id(turn_id: &str) -> TurnContextItem {
    TurnContextItem {
        turn_id: Some(turn_id.to_string()),
        trace_id: None,
        cwd: PathBuf::from("/tmp"),
        current_date: None,
        timezone: None,
        approval_policy: AskForApproval::Never,
        sandbox_policy: SandboxPolicy::DangerFullAccess,
        permission_profile: None,
        network: None,
        file_system_sandbox_policy: None,
        model: "test-model".into(),
        personality: None,
        collaboration_mode: None,
        realtime_active: None,
        effort: None,
        summary: protocol::config_types::ReasoningSummary::Auto,
        user_instructions: None,
        developer_instructions: None,
        final_output_json_schema: None,
        truncation_policy: None,
    }
}

fn completed_exec_event(call_id: &str, process_id: Option<String>) -> EventMsg {
    EventMsg::ExecCommandEnd(ExecCommandEndEvent {
        call_id: call_id.into(),
        process_id,
        turn_id: "turn-1".into(),
        completed_at_ms: 123,
        command: vec!["echo".into(), "done".into()],
        cwd: test_path_buf("/tmp").abs(),
        parsed_cmd: vec![ParsedCommand::Unknown {
            cmd: "echo done".into(),
        }],
        source: ExecCommandSource::UnifiedExecStartup,
        interaction_input: None,
        initial_wait_ms: Some(1000),
        notify_on: Some(protocol::protocol::ExecCommandNotifyOn::Exit),
        stdout: "done\n".into(),
        stderr: String::new(),
        aggregated_output: "done\n".into(),
        exit_code: 0,
        duration: Duration::from_millis(5),
        formatted_output: "done\n".into(),
        status: CoreExecCommandStatus::Completed,
    })
}

mod basic_turns;
mod display_markers;
mod turn_lifecycle;
mod tool_replay;
mod display_events;
mod late_turn_items;
mod turn_state;
mod collab_tools;
mod errors_and_context;
