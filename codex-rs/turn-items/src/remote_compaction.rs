use codex_context_manager::content_items_to_text;
use codex_context_manager::estimate_response_item_model_visible_bytes;
use codex_context_manager::insert_initial_context_before_last_real_user_or_summary;
use codex_context_manager::is_legacy_compaction_warning_message;
use protocol::items::TurnItem;
use protocol::models::ResponseItem;

use crate::parse_turn_item;

/// Metrics derived from a failed remote-compaction request.
#[derive(Debug)]
pub struct CompactRequestLogData {
    pub failing_compaction_request_model_visible_bytes: i64,
}

pub fn build_compact_request_log_data(
    input: &[ResponseItem],
    instructions: &str,
) -> CompactRequestLogData {
    let failing_compaction_request_model_visible_bytes = input
        .iter()
        .map(estimate_response_item_model_visible_bytes)
        .fold(
            i64::try_from(instructions.len()).unwrap_or(i64::MAX),
            i64::saturating_add,
        );

    CompactRequestLogData {
        failing_compaction_request_model_visible_bytes,
    }
}

pub fn process_remote_compacted_history(
    mut compacted_history: Vec<ResponseItem>,
    initial_context: Vec<ResponseItem>,
) -> Vec<ResponseItem> {
    compacted_history.retain(should_keep_remote_compacted_history_item);
    insert_initial_context_before_last_real_user_or_summary(
        compacted_history,
        initial_context,
        /*summary_prefix*/ None,
    )
}

/// Returns whether an item from remote compaction output should be preserved.
///
/// Called while processing the model-provided compacted transcript, before fresh
/// canonical context from the current session is appended.
pub fn should_keep_remote_compacted_history_item(item: &ResponseItem) -> bool {
    match item {
        ResponseItem::Message { role, .. } if role == "developer" => false,
        ResponseItem::Message { role, content, .. } if role == "user" => {
            match parse_turn_item(item) {
                Some(TurnItem::UserMessage(user)) => {
                    !is_legacy_compaction_warning_message(&user.message())
                }
                Some(TurnItem::HookPrompt(_)) => !content_items_to_text(content)
                    .as_deref()
                    .is_some_and(is_legacy_compaction_warning_message),
                _ => false,
            }
        }
        ResponseItem::Message { role, .. } if role == "assistant" => true,
        ResponseItem::Message { .. } => false,
        ResponseItem::InterAgentCommunication { .. } => true,
        ResponseItem::Compaction { .. } | ResponseItem::ContextCompaction { .. } => true,
        ResponseItem::Reasoning { .. }
        | ResponseItem::LocalShellCall { .. }
        | ResponseItem::FunctionCall { .. }
        | ResponseItem::ToolSearchCall { .. }
        | ResponseItem::FunctionCallOutput { .. }
        | ResponseItem::ToolSearchOutput { .. }
        | ResponseItem::CustomToolCall { .. }
        | ResponseItem::CustomToolCallOutput { .. }
        | ResponseItem::WebSearchCall { .. }
        | ResponseItem::ImageGenerationCall { .. }
        | ResponseItem::CommandWait { .. }
        | ResponseItem::CommandWriteStdin { .. }
        | ResponseItem::WorkflowRunProgress { .. }
        | ResponseItem::CommandExecutionNotification { .. }
        | ResponseItem::EventCommandEvent { .. }
        | ResponseItem::EventDrivenTool { .. }
        | ResponseItem::ThreadGoalUpdate { .. }
        | ResponseItem::Other => false,
    }
}

pub fn build_remote_v2_compacted_history(
    prompt_input: &[ResponseItem],
    compaction_output: ResponseItem,
) -> Vec<ResponseItem> {
    let mut retained = prompt_input
        .iter()
        .filter(|item| is_retained_for_remote_compaction_v2(item))
        .cloned()
        .collect::<Vec<_>>();
    retained.push(compaction_output);
    retained
}

fn is_retained_for_remote_compaction_v2(item: &ResponseItem) -> bool {
    let ResponseItem::Message { role, .. } = item else {
        return false;
    };

    matches!(role.as_str(), "user" | "developer" | "system")
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use protocol::models::ContentItem;
    use protocol::models::MessagePhase;

    fn message(role: &str, text: &str, phase: Option<MessagePhase>) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: role.to_string(),
            content: vec![ContentItem::InputText {
                text: text.to_string(),
            }],
            phase,
        }
    }

    #[test]
    fn build_remote_v2_compacted_history_matches_prod_retention_shape() {
        let input = vec![
            message("developer", "dev", /*phase*/ None),
            message("system", "sys", /*phase*/ None),
            message("user", "user", /*phase*/ None),
            message("assistant", "commentary", Some(MessagePhase::Commentary)),
            message("assistant", "final", Some(MessagePhase::FinalAnswer)),
            ResponseItem::FunctionCall {
                id: None,
                name: "shell_command".to_string(),
                namespace: None,
                arguments: "{}".to_string(),
                call_id: "call_1".to_string(),
            },
            ResponseItem::Compaction {
                encrypted_content: "old".to_string(),
            },
        ];
        let output = ResponseItem::ContextCompaction {
            encrypted_content: Some("new".to_string()),
        };

        let history = build_remote_v2_compacted_history(&input, output.clone());

        assert_eq!(
            history,
            vec![
                message("developer", "dev", /*phase*/ None),
                message("system", "sys", /*phase*/ None),
                message("user", "user", /*phase*/ None),
                output,
            ]
        );
    }
}
