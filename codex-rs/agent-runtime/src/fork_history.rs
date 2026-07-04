use protocol::models::ContentItem;
use protocol::models::MessagePhase;
use protocol::models::ResponseItem;
use protocol::protocol::RolloutItem;
use rollout_api::truncate_rollout_to_last_n_fork_turns;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnAgentForkMode {
    FullHistory,
    LastNTurns(usize),
}

/// Selects the parent rollout items that may be copied into a spawned child.
///
/// The runtime caller owns reading and flushing the parent rollout. This helper
/// owns only the stable fork-history policy: apply the requested history window,
/// strip parent-only MultiAgent usage hints, and drop model/tool internals that
/// should not become the child's starting context.
pub fn select_forked_rollout_items(
    items: Vec<RolloutItem>,
    fork_mode: &SpawnAgentForkMode,
    usage_hint_texts_to_filter: &[String],
) -> Vec<RolloutItem> {
    let mut items = match fork_mode {
        SpawnAgentForkMode::FullHistory => items,
        SpawnAgentForkMode::LastNTurns(last_n_turns) => {
            truncate_rollout_to_last_n_fork_turns(&items, *last_n_turns)
        }
    };

    items.retain(|item| {
        !is_filtered_usage_hint(item, usage_hint_texts_to_filter) && keep_forked_rollout_item(item)
    });
    items
}

fn is_filtered_usage_hint(item: &RolloutItem, usage_hint_texts_to_filter: &[String]) -> bool {
    if let RolloutItem::ResponseItem(ResponseItem::Message { role, content, .. }) = item
        && role == "developer"
        && let [ContentItem::InputText { text }] = content.as_slice()
    {
        return usage_hint_texts_to_filter
            .iter()
            .any(|usage_hint_text| usage_hint_text == text);
    }

    false
}

fn keep_forked_rollout_item(item: &RolloutItem) -> bool {
    match item {
        RolloutItem::ResponseItem(ResponseItem::Message { role, phase, .. }) => match role.as_str()
        {
            "system" | "developer" | "user" => true,
            "assistant" => *phase == Some(MessagePhase::FinalAnswer),
            _ => false,
        },
        RolloutItem::ResponseItem(
            ResponseItem::CommandWait { .. }
            | ResponseItem::CommandWriteStdin { .. }
            | ResponseItem::WorkflowRunProgress { .. }
            | ResponseItem::CommandExecutionNotification { .. }
            | ResponseItem::EventCommandEvent { .. }
            | ResponseItem::EventDrivenTool { .. }
            | ResponseItem::ThreadGoalUpdate { .. }
            | ResponseItem::InterAgentCommunication { .. },
        ) => true,
        RolloutItem::ResponseItem(
            ResponseItem::Reasoning { .. }
            | ResponseItem::LocalShellCall { .. }
            | ResponseItem::FunctionCall { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::FunctionCallOutput { .. }
            | ResponseItem::CustomToolCall { .. }
            | ResponseItem::CustomToolCallOutput { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::ContextCompaction { .. }
            | ResponseItem::Other,
        ) => false,
        // A forked child gets its own runtime config, including spawned-agent
        // instructions, so it must establish a fresh context diff baseline.
        RolloutItem::TurnContext(_) => false,
        RolloutItem::Compacted(_) | RolloutItem::EventMsg(_) | RolloutItem::SessionMeta(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::AgentPath;
    use protocol::protocol::InterAgentCommunication;

    fn message(role: &str, text: &str, phase: Option<MessagePhase>) -> RolloutItem {
        RolloutItem::ResponseItem(ResponseItem::Message {
            id: None,
            role: role.to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            phase,
        })
    }

    fn message_texts(items: &[RolloutItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|item| {
                let RolloutItem::ResponseItem(ResponseItem::Message { content, .. }) = item else {
                    return None;
                };
                let [ContentItem::InputText { text }] = content.as_slice() else {
                    return None;
                };
                Some(text.clone())
            })
            .collect()
    }

    #[test]
    fn full_history_filters_parent_only_items() {
        let usage_hint = "Parent root guidance.".to_string();
        let inter_agent = InterAgentCommunication::new(
            AgentPath::root(),
            AgentPath::try_from("/root/worker").expect("agent path"),
            Vec::new(),
            "triggered context".to_string(),
            protocol::protocol::InterAgentOperation::Unknown,
        );
        let items = vec![
            message("developer", &usage_hint, None),
            message("user", "parent task", None),
            message("assistant", "commentary", Some(MessagePhase::Commentary)),
            message("assistant", "final", Some(MessagePhase::FinalAnswer)),
            RolloutItem::ResponseItem(ResponseItem::InterAgentCommunication {
                id: None,
                communication: inter_agent,
            }),
            RolloutItem::ResponseItem(ResponseItem::Reasoning {
                id: "reasoning".to_string(),
                summary: Vec::new(),
                content: None,
                encrypted_content: None,
            }),
        ];

        let selected = select_forked_rollout_items(
            items,
            &SpawnAgentForkMode::FullHistory,
            std::slice::from_ref(&usage_hint),
        );

        assert_eq!(
            message_texts(&selected),
            vec!["parent task".to_string(), "final".to_string()]
        );
        assert!(selected.iter().any(|item| matches!(
            item,
            RolloutItem::ResponseItem(ResponseItem::InterAgentCommunication { .. })
        )));
        assert!(!selected.iter().any(|item| matches!(
            item,
            RolloutItem::ResponseItem(ResponseItem::Reasoning { .. })
        )));
    }

    #[test]
    fn last_n_turns_filters_after_windowing() {
        let items = vec![
            message("user", "old", None),
            message("assistant", "old final", Some(MessagePhase::FinalAnswer)),
            message("user", "current", None),
            message(
                "assistant",
                "current final",
                Some(MessagePhase::FinalAnswer),
            ),
        ];

        let selected = select_forked_rollout_items(items, &SpawnAgentForkMode::LastNTurns(1), &[]);

        assert_eq!(
            message_texts(&selected),
            vec!["current".to_string(), "current final".to_string()]
        );
    }
}
