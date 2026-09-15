use super::*;

pub(super) fn project_thread_turn_items_view(turns: &mut [Turn], items_view: TurnItemsView) {
    for turn in turns {
        project_turn_items_view(turn, items_view);
    }
}

fn project_turn_items_view(turn: &mut Turn, items_view: TurnItemsView) {
    match items_view {
        TurnItemsView::NotLoaded => {
            turn.items.clear();
            turn.items_view = TurnItemsView::NotLoaded;
        }
        TurnItemsView::Summary => project_turn_summary_items_view(turn),
        TurnItemsView::Full => {
            turn.items_view = TurnItemsView::Full;
        }
    }
}

fn project_turn_summary_items_view(turn: &mut Turn) {
    let first_user_message = turn
        .items
        .iter()
        .find(|item| matches!(item, ThreadItem::UserMessage { .. }))
        .cloned();
    let final_agent_message = turn
        .items
        .iter()
        .rev()
        .find(|item| matches!(item, ThreadItem::AgentMessage { .. }))
        .cloned();
    let initial_injected_context = turn
        .items
        .iter()
        .find(|item| matches!(item, ThreadItem::InjectedContext { .. }))
        .cloned();

    turn.items = match (
        first_user_message,
        final_agent_message,
        initial_injected_context,
    ) {
        (Some(user_message), Some(agent_message), _) if user_message.id() != agent_message.id() => {
            vec![user_message, agent_message]
        }
        (Some(user_message), _, _) => vec![user_message],
        (None, Some(agent_message), _) => vec![agent_message],
        (None, None, Some(injected_context)) => vec![injected_context],
        (None, None, None) => Vec::new(),
    };
    turn.items_view = TurnItemsView::Summary;
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_server_protocol::InjectedContextSection;
    use app_server_protocol::UserInput;
    use codex_utils_absolute_path::test_support::PathBufExt;

    fn turn(items: Vec<ThreadItem>) -> Turn {
        Turn {
            id: "turn-1".to_string(),
            items,
            items_view: TurnItemsView::Full,
            error: None,
            status: TurnStatus::Completed,
            started_at: Some(1),
            completed_at: Some(2),
            duration_ms: Some(100),
        }
    }

    fn user_message(id: &str, text: &str) -> ThreadItem {
        ThreadItem::UserMessage {
            id: id.to_string(),
            content: vec![UserInput::Text {
                text: text.to_string(),
                text_elements: Vec::new(),
            }],
        }
    }

    fn agent_message(id: &str, text: &str) -> ThreadItem {
        ThreadItem::AgentMessage {
            id: id.to_string(),
            text: text.to_string(),
            phase: None,
            memory_citation: None,
        }
    }

    fn injected_context(id: &str, text: &str) -> ThreadItem {
        ThreadItem::InjectedContext {
            id: id.to_string(),
            title: "Init Context".to_string(),
            preview: "Init Context".to_string(),
            sections: vec![InjectedContextSection {
                label: "Instructions".to_string(),
                text: text.to_string(),
            }],
        }
    }

    fn command_item(id: &str) -> ThreadItem {
        ThreadItem::CommandExecution {
            id: id.to_string(),
            command: "cargo test".to_string(),
            cwd: codex_utils_absolute_path::test_support::test_path_buf("/tmp").abs(),
            process_id: Some("process-1".to_string()),
            source: app_server_protocol::CommandExecutionSource::Agent,
            status: app_server_protocol::CommandExecutionStatus::Completed,
            initial_wait_ms: None,
            notify_on: None,
            command_actions: vec![app_server_protocol::CommandAction::Unknown {
                command: "cargo test".to_string(),
            }],
            aggregated_output: Some("ok".to_string()),
            exit_code: Some(0),
            duration_ms: Some(100),
        }
    }

    #[test]
    fn summary_keeps_first_user_and_final_agent_message() {
        let mut turns = vec![turn(vec![
            user_message("user-1", "start"),
            agent_message("agent-1", "first answer"),
            command_item("exec-1"),
            agent_message("agent-2", "final answer"),
        ])];

        project_thread_turn_items_view(&mut turns, TurnItemsView::Summary);

        assert_eq!(turns[0].items_view, TurnItemsView::Summary);
        assert_eq!(
            turns[0]
                .items
                .iter()
                .map(ThreadItem::id)
                .collect::<Vec<_>>(),
            vec!["user-1", "agent-2"]
        );
    }

    #[test]
    fn summary_does_not_duplicate_same_user_and_agent_id() {
        let mut turns = vec![turn(vec![
            user_message("item-1", "user text"),
            agent_message("item-1", "agent text"),
        ])];

        project_thread_turn_items_view(&mut turns, TurnItemsView::Summary);

        assert_eq!(
            turns[0]
                .items
                .iter()
                .map(ThreadItem::id)
                .collect::<Vec<_>>(),
            vec!["item-1"]
        );
        assert!(matches!(turns[0].items[0], ThreadItem::UserMessage { .. }));
    }

    #[test]
    fn summary_falls_back_to_injected_context_without_messages() {
        let mut turns = vec![turn(vec![
            command_item("exec-1"),
            injected_context("ctx-1", "instructions"),
        ])];

        project_thread_turn_items_view(&mut turns, TurnItemsView::Summary);

        assert_eq!(
            turns[0]
                .items
                .iter()
                .map(ThreadItem::id)
                .collect::<Vec<_>>(),
            vec!["ctx-1"]
        );
    }

    #[test]
    fn not_loaded_clears_items_and_marks_view() {
        let mut turns = vec![turn(vec![user_message("user-1", "start")])];

        project_thread_turn_items_view(&mut turns, TurnItemsView::NotLoaded);

        assert!(turns[0].items.is_empty());
        assert_eq!(turns[0].items_view, TurnItemsView::NotLoaded);
    }

    #[test]
    fn full_preserves_items_and_marks_view() {
        let original_items = vec![user_message("user-1", "start"), command_item("exec-1")];
        let mut turns = vec![turn(original_items.clone())];
        turns[0].items_view = TurnItemsView::Summary;

        project_thread_turn_items_view(&mut turns, TurnItemsView::Full);

        assert_eq!(turns[0].items, original_items);
        assert_eq!(turns[0].items_view, TurnItemsView::Full);
    }
}
