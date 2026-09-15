use super::*;

pub(super) struct LiveThreadReadProjectionBase {
    pub(super) thread: Thread,
    pub(super) persisted_turns: Vec<Turn>,
}

pub(super) fn live_thread_read_projection_base(
    fallback_thread: Thread,
    persisted_thread: Option<Thread>,
) -> LiveThreadReadProjectionBase {
    let mut persisted_turns = persisted_thread
        .as_ref()
        .map(|thread| thread.turns.clone())
        .unwrap_or_default();
    prune_turns_to_latest_compaction_boundary(&mut persisted_turns);

    let thread = match persisted_thread {
        Some(mut thread) => {
            if thread.path.is_none() {
                thread.path = fallback_thread.path.clone();
            }
            thread.session_id.clone_from(&fallback_thread.session_id);
            thread.ephemeral = fallback_thread.ephemeral;
            thread
        }
        None => fallback_thread,
    };

    LiveThreadReadProjectionBase {
        thread,
        persisted_turns,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use app_server_protocol::SessionSource;
    use codex_utils_absolute_path::test_support::PathBufExt;

    fn thread(id: &str, session_id: &str, path: Option<&str>, turns: Vec<Turn>) -> Thread {
        Thread {
            id: id.to_string(),
            session_id: session_id.to_string(),
            forked_from_id: None,
            preview: String::new(),
            ephemeral: false,
            model_provider: "mock_provider".to_string(),
            created_at: 1,
            updated_at: 1,
            lifecycle_status: ThreadLifecycleStatus::completed(None),
            path: path.map(PathBuf::from),
            cwd: codex_utils_absolute_path::test_support::test_path_buf("/tmp").abs(),
            cli_version: "0.0.0".to_string(),
            source: SessionSource::Cli,
            thread_source: None,
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            git_info: None,
            name: None,
            skills: Vec::new(),
            token_usage: None,
            context_usage: None,
            stats: None,
            turns,
            active_subscription_items: None,
            active_command_items: None,
        }
    }

    fn turn(id: &str, status: TurnStatus) -> Turn {
        Turn {
            id: id.to_string(),
            items: vec![ThreadItem::AgentMessage {
                id: format!("{id}-message"),
                text: format!("{id} message"),
                phase: None,
                memory_citation: None,
            }],
            items_view: TurnItemsView::Full,
            error: None,
            status,
            started_at: Some(1),
            completed_at: Some(2),
            duration_ms: Some(100),
        }
    }

    fn compaction_turn() -> Turn {
        Turn {
            id: "compaction-1".to_string(),
            items: vec![ThreadItem::ContextCompaction {
                id: "compact-1".to_string(),
                summary: Some("summary".to_string()),
                replacement_history: None,
            }],
            items_view: TurnItemsView::Full,
            error: None,
            status: TurnStatus::Completed,
            started_at: Some(1),
            completed_at: Some(2),
            duration_ms: Some(100),
        }
    }

    #[test]
    fn live_projection_base_overlays_live_identity_on_persisted_metadata() {
        let mut fallback = thread(
            "thread-1",
            "live-session",
            Some("/tmp/live-rollout.jsonl"),
            Vec::new(),
        );
        fallback.ephemeral = true;
        let mut persisted = thread(
            "thread-1",
            "persisted-session",
            None,
            vec![turn("turn-1", TurnStatus::Completed)],
        );
        persisted.preview = "persisted preview".to_string();

        let projection = live_thread_read_projection_base(fallback, Some(persisted));

        assert_eq!(projection.thread.session_id, "live-session");
        assert!(projection.thread.ephemeral);
        assert_eq!(
            projection.thread.path,
            Some(PathBuf::from("/tmp/live-rollout.jsonl"))
        );
        assert_eq!(projection.thread.preview, "persisted preview");
        assert_eq!(
            projection
                .persisted_turns
                .iter()
                .map(|turn| turn.id.as_str())
                .collect::<Vec<_>>(),
            vec!["turn-1"]
        );
    }

    #[test]
    fn live_projection_base_preserves_persisted_path_when_available() {
        let fallback = thread(
            "thread-1",
            "live-session",
            Some("/tmp/live-rollout.jsonl"),
            Vec::new(),
        );
        let persisted = thread(
            "thread-1",
            "persisted-session",
            Some("/tmp/persisted-rollout.jsonl"),
            Vec::new(),
        );

        let projection = live_thread_read_projection_base(fallback, Some(persisted));

        assert_eq!(
            projection.thread.path,
            Some(PathBuf::from("/tmp/persisted-rollout.jsonl"))
        );
    }

    #[test]
    fn live_projection_base_prunes_persisted_turns_to_latest_compaction() {
        let fallback = thread("thread-1", "live-session", None, Vec::new());
        let persisted = thread(
            "thread-1",
            "persisted-session",
            None,
            vec![
                turn("turn-before-compact", TurnStatus::Completed),
                compaction_turn(),
                turn("turn-after-compact", TurnStatus::Completed),
            ],
        );

        let projection = live_thread_read_projection_base(fallback, Some(persisted));

        assert_eq!(
            projection
                .persisted_turns
                .iter()
                .map(|turn| turn.id.as_str())
                .collect::<Vec<_>>(),
            vec!["compaction-1", "turn-after-compact"]
        );
    }

    #[test]
    fn live_projection_base_uses_fallback_when_persisted_thread_is_absent() {
        let fallback = thread(
            "thread-1",
            "live-session",
            Some("/tmp/live-rollout.jsonl"),
            Vec::new(),
        );

        let projection = live_thread_read_projection_base(fallback, None);

        assert_eq!(projection.thread.session_id, "live-session");
        assert_eq!(projection.persisted_turns, Vec::<Turn>::new());
    }
}
