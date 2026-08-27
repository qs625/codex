use std::collections::HashMap;

use codex_utils_stream_parser::AssistantTextStreamParser;
use codex_utils_stream_parser::extract_proposed_plan_text;
use codex_utils_stream_parser::strip_citations;
use codex_utils_stream_parser::strip_proposed_plan_blocks;
use protocol::items::AgentMessageContent;
use protocol::items::AgentMessageItem;
use protocol::items::TurnItem;
use protocol::models::ContentItem;
use protocol::models::ResponseItem;
use protocol::protocol::EventMsg;

pub use codex_utils_stream_parser::AssistantTextChunk as ParsedAssistantTextDelta;
pub use codex_utils_stream_parser::ProposedPlanSegment;

#[derive(Debug, Default)]
pub struct AssistantMessageStreamParsers {
    plan_mode: bool,
    parsers_by_item: HashMap<String, AssistantTextStreamParser>,
}

impl AssistantMessageStreamParsers {
    pub fn new(plan_mode: bool) -> Self {
        Self {
            plan_mode,
            parsers_by_item: HashMap::new(),
        }
    }

    fn parser_mut(&mut self, item_id: &str) -> &mut AssistantTextStreamParser {
        let plan_mode = self.plan_mode;
        self.parsers_by_item
            .entry(item_id.to_string())
            .or_insert_with(|| AssistantTextStreamParser::new(plan_mode))
    }

    pub fn seed_item_text(&mut self, item_id: &str, text: &str) -> ParsedAssistantTextDelta {
        if text.is_empty() {
            return ParsedAssistantTextDelta::default();
        }
        self.parser_mut(item_id).push_str(text)
    }

    pub fn parse_delta(&mut self, item_id: &str, delta: &str) -> ParsedAssistantTextDelta {
        self.parser_mut(item_id).push_str(delta)
    }

    pub fn finish_item(&mut self, item_id: &str) -> ParsedAssistantTextDelta {
        let Some(mut parser) = self.parsers_by_item.remove(item_id) else {
            return ParsedAssistantTextDelta::default();
        };
        parser.finish()
    }

    pub fn drain_finished(&mut self) -> Vec<(String, ParsedAssistantTextDelta)> {
        let parsers_by_item = std::mem::take(&mut self.parsers_by_item);
        let mut finished = Vec::new();

        for (item_id, mut parser) in parsers_by_item {
            let tail = parser.finish();
            finished.push((item_id, tail));
        }

        finished
    }
}

/// Agent messages are text-only today; concatenate all text entries.
pub fn agent_message_text(item: &AgentMessageItem) -> String {
    item.content
        .iter()
        .map(|entry| match entry {
            AgentMessageContent::Text { text } => text.as_str(),
        })
        .collect()
}

pub fn realtime_text_for_event(msg: &EventMsg) -> Option<String> {
    match msg {
        EventMsg::AgentMessage(event) => Some(event.message.clone()),
        EventMsg::ItemCompleted(event) => match &event.item {
            TurnItem::AgentMessage(item) => Some(agent_message_text(item)),
            TurnItem::UserMessage(_)
            | TurnItem::HookPrompt(_)
            | TurnItem::InjectedContext(_)
            | TurnItem::EventDrivenTool(_)
            | TurnItem::EventCommandEvent(_)
            | TurnItem::CollabAgentMessage(_)
            | TurnItem::ConversationArtifact(_)
            | TurnItem::Plan(_)
            | TurnItem::Reasoning(_)
            | TurnItem::WebSearch(_)
            | TurnItem::ImageView(_)
            | TurnItem::ImageGeneration(_)
            | TurnItem::FileChange(_)
            | TurnItem::McpToolCall(_)
            | TurnItem::ContextCompaction(_) => None,
        },
        EventMsg::Error(_)
        | EventMsg::Warning(_)
        | EventMsg::GuardianWarning(_)
        | EventMsg::RealtimeConversationStarted(_)
        | EventMsg::RealtimeConversationSdp(_)
        | EventMsg::RealtimeConversationRealtime(_)
        | EventMsg::RealtimeConversationClosed(_)
        | EventMsg::ModelReroute(_)
        | EventMsg::ModelVerification(_)
        | EventMsg::ContextCompacted(_)
        | EventMsg::ThreadRolledBack(_)
        | EventMsg::TurnStarted(_)
        | EventMsg::TurnComplete(_)
        | EventMsg::TokenCount(_)
        | EventMsg::UserMessage(_)
        | EventMsg::AgentReasoning(_)
        | EventMsg::AgentReasoningRawContent(_)
        | EventMsg::AgentReasoningSectionBreak(_)
        | EventMsg::SessionConfigured(_)
        | EventMsg::ThreadGoalUpdated(_)
        | EventMsg::ThreadSkillsUpdated(_)
        | EventMsg::McpStartupUpdate(_)
        | EventMsg::McpStartupComplete(_)
        | EventMsg::McpToolCallBegin(_)
        | EventMsg::McpToolCallEnd(_)
        | EventMsg::WebSearchBegin(_)
        | EventMsg::WebSearchEnd(_)
        | EventMsg::ExecCommandBegin(_)
        | EventMsg::ExecCommandOutputDelta(_)
        | EventMsg::TerminalInteraction(_)
        | EventMsg::ExecCommandEnd(_)
        | EventMsg::PatchApplyBegin(_)
        | EventMsg::PatchApplyUpdated(_)
        | EventMsg::PatchApplyEnd(_)
        | EventMsg::ImageGenerationBegin(_)
        | EventMsg::ImageGenerationEnd(_)
        | EventMsg::ViewImageToolCall(_)
        | EventMsg::ExecApprovalRequest(_)
        | EventMsg::RequestPermissions(_)
        | EventMsg::RequestUserInput(_)
        | EventMsg::DynamicToolCallRequest(_)
        | EventMsg::DynamicToolCallResponse(_)
        | EventMsg::GuardianAssessment(_)
        | EventMsg::ElicitationRequest(_)
        | EventMsg::ApplyPatchApprovalRequest(_)
        | EventMsg::DeprecationNotice(_)
        | EventMsg::StreamError(_)
        | EventMsg::TurnDiff(_)
        | EventMsg::RealtimeConversationListVoicesResponse(_)
        | EventMsg::PlanUpdate(_)
        | EventMsg::TurnAborted(_)
        | EventMsg::ShutdownComplete
        | EventMsg::EnteredReviewMode(_)
        | EventMsg::ExitedReviewMode(_)
        | EventMsg::RawResponseItem(_)
        | EventMsg::ItemStarted(_)
        | EventMsg::ResponseItemStarted(_)
        | EventMsg::ResponseItemCompleted(_)
        | EventMsg::CommandWaitStarted(_)
        | EventMsg::CommandWaitCompleted(_)
        | EventMsg::CommandWriteStdinCompleted(_)
        | EventMsg::CommandExecutionNotificationCompleted(_)
        | EventMsg::BuiltinToolCallStarted(_)
        | EventMsg::BuiltinToolCallCompleted(_)
        | EventMsg::ExternalToolCallStarted(_)
        | EventMsg::ExternalToolCallCompleted(_)
        | EventMsg::ExternalTerminalStatus(_)
        | EventMsg::WorkflowRunProgressCompleted(_)
        | EventMsg::EventCommandEventCompleted(_)
        | EventMsg::EventDrivenToolCompleted(_)
        | EventMsg::InterAgentCommunicationCompleted(_)
        | EventMsg::ThreadGoalUpdateCompleted(_)
        | EventMsg::HookStarted(_)
        | EventMsg::HookCompleted(_)
        | EventMsg::AgentMessageContentDelta(_)
        | EventMsg::PlanDelta(_)
        | EventMsg::ReasoningContentDelta(_)
        | EventMsg::ReasoningRawContentDelta(_)
        | EventMsg::CollabAgentSpawnBegin(_)
        | EventMsg::CollabAgentSpawnEnd(_)
        | EventMsg::CollabAgentInteractionBegin(_)
        | EventMsg::CollabAgentInteractionEnd(_)
        | EventMsg::CollabListAgentsBegin(_)
        | EventMsg::CollabListAgentsEnd(_)
        | EventMsg::CollabWaitingBegin(_)
        | EventMsg::CollabWaitingEnd(_)
        | EventMsg::CollabCloseBegin(_)
        | EventMsg::CollabCloseEnd(_)
        | EventMsg::CollabResumeBegin(_)
        | EventMsg::CollabResumeEnd(_)
        | EventMsg::ThreadContextUsageUpdated(_) => None,
    }
}

pub fn proposed_plan_text_from_assistant_response_item(item: &ResponseItem) -> Option<String> {
    if let ResponseItem::Message { role, content, .. } = item
        && role == "assistant"
    {
        let mut text = String::new();
        for entry in content {
            if let ContentItem::OutputText { text: chunk } = entry {
                text.push_str(chunk);
            }
        }
        return extract_proposed_plan_text(&text).map(|plan_text| {
            let (plan_text, _citations) = strip_citations(&plan_text);
            plan_text
        });
    }
    None
}

pub fn raw_assistant_output_text_from_item(item: &ResponseItem) -> Option<String> {
    if let ResponseItem::Message { role, content, .. } = item
        && role == "assistant"
    {
        let combined = content
            .iter()
            .filter_map(|content_item| match content_item {
                ContentItem::OutputText { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<String>();
        return Some(combined);
    }
    None
}

pub fn strip_hidden_assistant_markup(text: &str, plan_mode: bool) -> String {
    let (without_citations, _) = strip_citations(text);
    if plan_mode {
        strip_proposed_plan_blocks(&without_citations)
    } else {
        without_citations
    }
}

pub fn last_assistant_message_from_item(item: &ResponseItem, plan_mode: bool) -> Option<String> {
    let combined = raw_assistant_output_text_from_item(item)?;
    if combined.is_empty() {
        return None;
    }
    let stripped = strip_hidden_assistant_markup(&combined, plan_mode);
    if stripped.trim().is_empty() {
        return None;
    }
    Some(stripped)
}

pub fn last_assistant_message_from_turn(responses: &[ResponseItem]) -> Option<String> {
    for item in responses.iter().rev() {
        if let Some(message) = last_assistant_message_from_item(item, /*plan_mode*/ false) {
            return Some(message);
        }
    }
    None
}
