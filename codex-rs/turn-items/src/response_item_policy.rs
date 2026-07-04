use codex_utils_stream_parser::strip_citations;
use codex_utils_stream_parser::strip_proposed_plan_blocks;
use memory_service_api::citations::parse_memory_citation;
use protocol::items::AgentMessageContent;
use protocol::items::AgentMessageItem;
use protocol::items::TurnItem;
use protocol::memory_citation::MemoryCitation;
use protocol::models::MessagePhase;
use protocol::models::ResponseInputItem;
use protocol::models::ResponseItem;

use crate::last_assistant_message_from_item;

pub fn response_item_may_include_external_context(item: &ResponseItem) -> bool {
    matches!(
        item,
        ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
    )
}

pub fn completed_item_defers_mailbox_delivery_to_next_turn(
    item: &ResponseItem,
    plan_mode: bool,
) -> bool {
    match item {
        ResponseItem::Message { role, phase, .. } => {
            if role != "assistant" || matches!(phase, Some(MessagePhase::Commentary)) {
                return false;
            }
            // Treat `None` like final-answer text so untagged providers default
            // to the safer "defer mailbox mail" behavior.
            last_assistant_message_from_item(item, plan_mode).is_some()
        }
        ResponseItem::ImageGenerationCall { .. } => true,
        _ => false,
    }
}

pub fn response_input_to_response_item(input: &ResponseInputItem) -> Option<ResponseItem> {
    match input {
        ResponseInputItem::FunctionCallOutput { call_id, output } => {
            Some(ResponseItem::FunctionCallOutput {
                call_id: call_id.clone(),
                output: output.clone(),
            })
        }
        ResponseInputItem::CustomToolCallOutput {
            call_id,
            name,
            output,
        } => Some(ResponseItem::CustomToolCallOutput {
            call_id: call_id.clone(),
            name: name.clone(),
            output: output.clone(),
        }),
        ResponseInputItem::McpToolCallOutput { call_id, output } => {
            let output = output.as_function_call_output_payload();
            Some(ResponseItem::FunctionCallOutput {
                call_id: call_id.clone(),
                output,
            })
        }
        ResponseInputItem::ToolSearchOutput {
            call_id,
            status,
            execution,
            tools,
        } => Some(ResponseItem::ToolSearchOutput {
            call_id: Some(call_id.clone()),
            status: status.clone(),
            execution: execution.clone(),
            tools: tools.clone(),
        }),
        _ => None,
    }
}

#[derive(Clone, Default)]
pub struct FinalizedTurnItemFacts {
    pub memory_citation: Option<MemoryCitation>,
    pub last_agent_message: Option<String>,
    pub defers_mailbox_delivery_to_next_turn: bool,
}

pub fn finalize_agent_message_content(agent_message: &mut AgentMessageItem, plan_mode: bool) {
    let combined = agent_message_text(agent_message);
    let (stripped, memory_citation) =
        strip_hidden_assistant_markup_and_parse_memory_citation(&combined, plan_mode);
    agent_message.content = vec![AgentMessageContent::Text { text: stripped }];
    if agent_message.memory_citation.is_none() {
        agent_message.memory_citation = memory_citation;
    }
}

pub fn finalized_turn_item_facts(turn_item: &TurnItem) -> FinalizedTurnItemFacts {
    match turn_item {
        TurnItem::AgentMessage(agent_message) => {
            let combined = agent_message_text(agent_message);
            let last_agent_message = if combined.trim().is_empty() {
                None
            } else {
                Some(combined)
            };
            let defers_mailbox_delivery_to_next_turn =
                !matches!(agent_message.phase, Some(MessagePhase::Commentary))
                    && last_agent_message.is_some();
            FinalizedTurnItemFacts {
                memory_citation: agent_message.memory_citation.clone(),
                last_agent_message,
                defers_mailbox_delivery_to_next_turn,
            }
        }
        TurnItem::ImageGeneration(_) => FinalizedTurnItemFacts {
            defers_mailbox_delivery_to_next_turn: true,
            ..Default::default()
        },
        _ => FinalizedTurnItemFacts::default(),
    }
}

fn agent_message_text(agent_message: &AgentMessageItem) -> String {
    agent_message
        .content
        .iter()
        .map(|entry| match entry {
            AgentMessageContent::Text { text } => text.as_str(),
        })
        .collect()
}

fn strip_hidden_assistant_markup_and_parse_memory_citation(
    text: &str,
    plan_mode: bool,
) -> (String, Option<MemoryCitation>) {
    let (without_citations, citations) = strip_citations(text);
    let visible_text = if plan_mode {
        strip_proposed_plan_blocks(&without_citations)
    } else {
        without_citations
    };
    (visible_text, parse_memory_citation(citations))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use protocol::items::ImageGenerationItem;
    use protocol::models::ContentItem;
    use protocol::models::FunctionCallOutputPayload;
    use protocol::models::LocalShellAction;
    use protocol::models::LocalShellExecAction;
    use protocol::models::LocalShellStatus;

    fn assistant_output_text(text: &str) -> ResponseItem {
        assistant_output_text_with_phase(text, /*phase*/ None)
    }

    fn assistant_output_text_with_phase(text: &str, phase: Option<MessagePhase>) -> ResponseItem {
        ResponseItem::Message {
            id: Some("msg-1".to_string()),
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: text.to_string(),
            }],
            phase,
        }
    }

    #[test]
    fn external_context_pollution_items_include_web_search_and_tool_search() {
        let polluting_items = [
            ResponseItem::WebSearchCall {
                id: None,
                status: Some("completed".to_string()),
                action: None,
            },
            ResponseItem::ToolSearchCall {
                id: None,
                call_id: Some("search-1".to_string()),
                status: None,
                execution: "client".to_string(),
                arguments: serde_json::json!({"query": "calendar"}),
            },
            ResponseItem::ToolSearchOutput {
                call_id: Some("search-1".to_string()),
                status: "completed".to_string(),
                execution: "client".to_string(),
                tools: Vec::new(),
            },
        ];

        assert!(
            polluting_items
                .iter()
                .all(response_item_may_include_external_context)
        );
    }

    #[test]
    fn external_context_pollution_items_exclude_local_tool_calls() {
        let non_polluting_items = [
            ResponseItem::LocalShellCall {
                id: None,
                call_id: Some("shell-1".to_string()),
                status: LocalShellStatus::Completed,
                action: LocalShellAction::Exec(LocalShellExecAction {
                    command: vec!["cat".to_string(), "README.md".to_string()],
                    timeout_ms: None,
                    working_directory: None,
                    env: None,
                    user: None,
                }),
            },
            ResponseItem::FunctionCall {
                id: None,
                name: "shell".to_string(),
                namespace: None,
                arguments: "{}".to_string(),
                call_id: "call-1".to_string(),
            },
            ResponseItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: FunctionCallOutputPayload::from_text("ok".to_string()),
            },
            ResponseItem::CustomToolCall {
                id: None,
                status: None,
                call_id: "custom-1".to_string(),
                name: "apply_patch".to_string(),
                input: "*** Begin Patch\n*** End Patch\n".to_string(),
            },
            ResponseItem::CustomToolCallOutput {
                call_id: "custom-1".to_string(),
                name: Some("apply_patch".to_string()),
                output: FunctionCallOutputPayload::from_text("ok".to_string()),
            },
            assistant_output_text("plain assistant text"),
        ];

        assert!(
            !non_polluting_items
                .iter()
                .any(response_item_may_include_external_context)
        );
    }

    #[test]
    fn completed_item_defers_mailbox_delivery_for_unknown_phase_messages() {
        let item = assistant_output_text("final answer");

        assert!(completed_item_defers_mailbox_delivery_to_next_turn(
            &item, /*plan_mode*/ false,
        ));
    }

    #[test]
    fn completed_item_keeps_mailbox_delivery_open_for_commentary_messages() {
        let item =
            assistant_output_text_with_phase("still working", Some(MessagePhase::Commentary));

        assert!(!completed_item_defers_mailbox_delivery_to_next_turn(
            &item, /*plan_mode*/ false,
        ));
    }

    #[test]
    fn completed_item_defers_mailbox_delivery_for_image_generation_calls() {
        let item = ResponseItem::ImageGenerationCall {
            id: "ig-1".to_string(),
            status: "completed".to_string(),
            revised_prompt: None,
            result: "Zm9v".to_string(),
        };

        assert!(completed_item_defers_mailbox_delivery_to_next_turn(
            &item, /*plan_mode*/ false,
        ));
    }

    #[test]
    fn response_input_to_response_item_maps_function_call_output() {
        let input = ResponseInputItem::FunctionCallOutput {
            call_id: "call-1".to_string(),
            output: FunctionCallOutputPayload::from_text("ok".to_string()),
        };

        assert_eq!(
            response_input_to_response_item(&input),
            Some(ResponseItem::FunctionCallOutput {
                call_id: "call-1".to_string(),
                output: FunctionCallOutputPayload::from_text("ok".to_string()),
            })
        );
    }

    #[test]
    fn finalize_agent_message_content_strips_citations_and_records_memory_citation() {
        let mut agent_message = AgentMessageItem {
            id: "msg-1".to_string(),
            content: vec![AgentMessageContent::Text {
                text: "hello<oai-mem-citation><citation_entries>\nMEMORY.md:1-2|note=[x]\n</citation_entries>\n<rollout_ids>\n019cc2ea-1dff-7902-8d40-c8f6e5d83cc4\n</rollout_ids></oai-mem-citation> world".to_string(),
            }],
            phase: None,
            memory_citation: None,
        };

        finalize_agent_message_content(&mut agent_message, /*plan_mode*/ false);

        assert_eq!(agent_message_text(&agent_message), "hello world");
        let memory_citation = agent_message
            .memory_citation
            .expect("memory citation should be parsed");
        assert_eq!(memory_citation.entries.len(), 1);
        assert_eq!(memory_citation.entries[0].path, "MEMORY.md");
        assert_eq!(
            memory_citation.rollout_ids,
            vec!["019cc2ea-1dff-7902-8d40-c8f6e5d83cc4".to_string()]
        );
    }

    #[test]
    fn finalize_agent_message_content_preserves_contributor_memory_citation() {
        let existing = MemoryCitation {
            entries: Vec::new(),
            rollout_ids: vec!["existing".to_string()],
        };
        let mut agent_message = AgentMessageItem {
            id: "msg-1".to_string(),
            content: vec![AgentMessageContent::Text {
                text: "hello<oai-mem-citation>ignored</oai-mem-citation>".to_string(),
            }],
            phase: None,
            memory_citation: Some(existing.clone()),
        };

        finalize_agent_message_content(&mut agent_message, /*plan_mode*/ false);

        assert_eq!(agent_message.memory_citation, Some(existing));
    }

    #[test]
    fn finalized_turn_item_facts_extract_agent_message_state() {
        let turn_item = TurnItem::AgentMessage(AgentMessageItem {
            id: "msg-1".to_string(),
            content: vec![AgentMessageContent::Text {
                text: "final answer".to_string(),
            }],
            phase: None,
            memory_citation: Some(MemoryCitation {
                entries: Vec::new(),
                rollout_ids: Vec::new(),
            }),
        });

        let facts = finalized_turn_item_facts(&turn_item);

        assert_eq!(facts.last_agent_message.as_deref(), Some("final answer"));
        assert!(facts.memory_citation.is_some());
        assert!(facts.defers_mailbox_delivery_to_next_turn);
    }

    #[test]
    fn finalized_turn_item_facts_do_not_defer_commentary_message() {
        let turn_item = TurnItem::AgentMessage(AgentMessageItem {
            id: "msg-1".to_string(),
            content: vec![AgentMessageContent::Text {
                text: "still working".to_string(),
            }],
            phase: Some(MessagePhase::Commentary),
            memory_citation: None,
        });

        let facts = finalized_turn_item_facts(&turn_item);

        assert_eq!(facts.last_agent_message.as_deref(), Some("still working"));
        assert!(!facts.defers_mailbox_delivery_to_next_turn);
    }

    #[test]
    fn finalized_turn_item_facts_defer_image_generation() {
        let turn_item = TurnItem::ImageGeneration(ImageGenerationItem {
            id: "ig-1".to_string(),
            status: "completed".to_string(),
            revised_prompt: None,
            result: "Zm9v".to_string(),
            saved_path: None,
        });

        let facts = finalized_turn_item_facts(&turn_item);

        assert!(facts.last_agent_message.is_none());
        assert!(facts.memory_citation.is_none());
        assert!(facts.defers_mailbox_delivery_to_next_turn);
    }
}
