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

use crate::ARTIFACT_MARKER_END;
use crate::ARTIFACT_MARKER_START_PREFIX;
use crate::strip_artifact_markers_for_display;

pub use codex_utils_stream_parser::AssistantTextChunk as ParsedAssistantTextDelta;
pub use codex_utils_stream_parser::ProposedPlanSegment;

#[derive(Debug, Default)]
pub struct AssistantMessageStreamParsers {
    plan_mode: bool,
    parsers_by_item: HashMap<String, AssistantTextStreamParser>,
    artifact_filters_by_item: HashMap<String, ArtifactMarkerStreamFilter>,
}

impl AssistantMessageStreamParsers {
    pub fn new(plan_mode: bool) -> Self {
        Self {
            plan_mode,
            parsers_by_item: HashMap::new(),
            artifact_filters_by_item: HashMap::new(),
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
        self.push_visible_text_after_artifact_filter(item_id, text)
    }

    pub fn parse_delta(&mut self, item_id: &str, delta: &str) -> ParsedAssistantTextDelta {
        self.push_visible_text_after_artifact_filter(item_id, delta)
    }

    pub fn finish_item(&mut self, item_id: &str) -> ParsedAssistantTextDelta {
        if let Some(filter) = self.artifact_filters_by_item.remove(item_id) {
            let tail = filter.finish();
            if !tail.is_empty() {
                let mut parsed_tail = self.parser_mut(item_id).push_str(&tail);
                if let Some(mut parser) = self.parsers_by_item.remove(item_id) {
                    merge_text_chunks(&mut parsed_tail, parser.finish());
                }
                return parsed_tail;
            }
        }
        let Some(mut parser) = self.parsers_by_item.remove(item_id) else {
            return ParsedAssistantTextDelta::default();
        };
        parser.finish()
    }

    pub fn drain_finished(&mut self) -> Vec<(String, ParsedAssistantTextDelta)> {
        let mut parsers_by_item = std::mem::take(&mut self.parsers_by_item);
        let filters_by_item = std::mem::take(&mut self.artifact_filters_by_item);
        let mut finished = Vec::new();

        for (item_id, filter) in filters_by_item {
            let tail = filter.finish();
            let parser = parsers_by_item
                .entry(item_id.clone())
                .or_insert_with(|| AssistantTextStreamParser::new(self.plan_mode));
            if !tail.is_empty() {
                let parsed_tail = parser.push_str(&tail);
                finished.push((item_id.clone(), parsed_tail));
            }
        }

        for (item_id, mut parser) in parsers_by_item {
            let tail = parser.finish();
            if let Some((_, existing)) = finished
                .iter_mut()
                .find(|(finished_item_id, _)| finished_item_id == &item_id)
            {
                merge_text_chunks(existing, tail);
            } else {
                finished.push((item_id, tail));
            }
        }

        finished
    }

    fn artifact_filter_mut(&mut self, item_id: &str) -> &mut ArtifactMarkerStreamFilter {
        self.artifact_filters_by_item
            .entry(item_id.to_string())
            .or_default()
    }

    fn push_visible_text_after_artifact_filter(
        &mut self,
        item_id: &str,
        text: &str,
    ) -> ParsedAssistantTextDelta {
        let visible_text = self.artifact_filter_mut(item_id).push_str(text);
        if visible_text.is_empty() {
            ParsedAssistantTextDelta::default()
        } else {
            self.parser_mut(item_id).push_str(&visible_text)
        }
    }
}

fn merge_text_chunks(target: &mut ParsedAssistantTextDelta, mut source: ParsedAssistantTextDelta) {
    target.visible_text.push_str(&source.visible_text);
    target.citations.append(&mut source.citations);
    target.plan_segments.append(&mut source.plan_segments);
}

#[derive(Debug)]
struct ArtifactMarkerStreamFilter {
    state: ArtifactMarkerStreamState,
}

#[derive(Debug)]
enum ArtifactMarkerStreamState {
    Outside { pending: String },
    Inside { hidden: String },
}

impl Default for ArtifactMarkerStreamFilter {
    fn default() -> Self {
        Self {
            state: ArtifactMarkerStreamState::Outside {
                pending: String::new(),
            },
        }
    }
}

impl ArtifactMarkerStreamFilter {
    fn push_str(&mut self, text: &str) -> String {
        match &mut self.state {
            ArtifactMarkerStreamState::Outside { pending } => {
                let combined = format!("{pending}{text}");
                pending.clear();
                if let Some(start) = combined.find(ARTIFACT_MARKER_START_PREFIX) {
                    let mut output = combined[..start].to_string();
                    let after_start =
                        combined[start + ARTIFACT_MARKER_START_PREFIX.len()..].to_string();
                    self.state = ArtifactMarkerStreamState::Inside {
                        hidden: String::new(),
                    };
                    output.push_str(&self.push_str(&after_start));
                    return output;
                }
                let keep = marker_prefix_suffix_len(&combined);
                let emit_len = combined.len().saturating_sub(keep);
                pending.push_str(&combined[emit_len..]);
                combined[..emit_len].to_string()
            }
            ArtifactMarkerStreamState::Inside { hidden } => {
                hidden.push_str(text);
                if let Some(end) = hidden.find(ARTIFACT_MARKER_END) {
                    let after_end = hidden[end + ARTIFACT_MARKER_END.len()..].to_string();
                    self.state = ArtifactMarkerStreamState::Outside {
                        pending: String::new(),
                    };
                    return self.push_str(&after_end);
                }
                String::new()
            }
        }
    }

    fn finish(self) -> String {
        match self.state {
            ArtifactMarkerStreamState::Outside { pending } => pending,
            ArtifactMarkerStreamState::Inside { hidden } => {
                format!("{ARTIFACT_MARKER_START_PREFIX}{hidden}")
            }
        }
    }
}

fn marker_prefix_suffix_len(text: &str) -> usize {
    let max_len = text.len().min(ARTIFACT_MARKER_START_PREFIX.len() - 1);
    for len in (1..=max_len).rev() {
        let start = text.len() - len;
        if text.is_char_boundary(start) && ARTIFACT_MARKER_START_PREFIX.starts_with(&text[start..])
        {
            return len;
        }
    }
    0
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
        let text = strip_artifact_markers_for_display(&text);
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
    let without_artifacts = strip_artifact_markers_for_display(&without_citations);
    if plan_mode {
        strip_proposed_plan_blocks(&without_artifacts)
    } else {
        without_artifacts
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
