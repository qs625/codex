    use super::*;
    use crate::CHANNEL_CAPACITY;
    use crate::outgoing_message::ConnectionId;
    use crate::outgoing_message::OutgoingEnvelope;
    use crate::outgoing_message::OutgoingMessage;
    use crate::outgoing_message::OutgoingMessageSender;
    use anyhow::Result;
    use anyhow::anyhow;
    use anyhow::bail;
    use app_server_protocol::AutoReviewDecisionSource;
    use app_server_protocol::GuardianApprovalReviewStatus;
    use app_server_protocol::JSONRPCErrorError;
    use app_server_protocol::TurnPlanStepStatus;
    use chrono::Utc;
    use codex_login::CodexAuth;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use codex_utils_absolute_path::test_support::PathBufExt;
    use codex_utils_absolute_path::test_support::test_path_buf;
    use core_test_support::load_default_config_for_test;
    use protocol::items::HookPromptFragment;
    use protocol::items::build_hook_prompt_message;
    use protocol::models::FileSystemPermissions as CoreFileSystemPermissions;
    use protocol::models::NetworkPermissions as CoreNetworkPermissions;
    use protocol::permissions::FileSystemAccessMode;
    use protocol::permissions::FileSystemPath;
    use protocol::permissions::FileSystemSandboxEntry;
    use protocol::permissions::FileSystemSpecialPath;
    use protocol::plan_tool::PlanItemArg;
    use protocol::plan_tool::StepStatus;
    use protocol::protocol::AgentMessageEvent;
    use protocol::protocol::AskForApproval;
    use protocol::protocol::CreditsSnapshot;
    use protocol::protocol::EventMsg;
    use protocol::protocol::GuardianAssessmentEvent;
    use protocol::protocol::GuardianAssessmentStatus;
    use protocol::protocol::RateLimitSnapshot;
    use protocol::protocol::RateLimitWindow;
    use protocol::protocol::RolloutItem;
    use protocol::protocol::SandboxPolicy;
    use protocol::protocol::SessionSource;
    use protocol::protocol::ThreadSkill;
    use protocol::protocol::ThreadSkillKind;
    use protocol::protocol::ThreadSkillsUpdatedEvent;
    use protocol::protocol::TokenUsage;
    use protocol::protocol::TokenUsageInfo;
    use protocol::protocol::UserMessageEvent;
    use serde_json::json;
    use tempfile::TempDir;
    use thread_service::CodexThread;
    use thread_service::ThreadService;
    use thread_store::StoredThread;
    use thread_store::StoredThreadHistory;
    use tokio::sync::Mutex;
    use tokio::sync::mpsc;

    fn new_thread_state() -> Arc<Mutex<ThreadState>> {
        Arc::new(Mutex::new(ThreadState::default()))
    }

    const TEST_TURN_COMPLETED_AT: i64 = 1_716_000_456;
    const TEST_TURN_DURATION_MS: i64 = 1_234;

    async fn recv_broadcast_message(
        rx: &mut mpsc::Receiver<OutgoingEnvelope>,
    ) -> Result<OutgoingMessage> {
        let envelope = rx
            .recv()
            .await
            .ok_or_else(|| anyhow!("should send one message"))?;
        match envelope {
            OutgoingEnvelope::Broadcast { message } => Ok(message),
            OutgoingEnvelope::ToConnection { message, .. } => Ok(message),
        }
    }

    fn test_outgoing(tx: mpsc::Sender<OutgoingEnvelope>) -> ThreadScopedOutgoingMessageSender {
        let outgoing = Arc::new(OutgoingMessageSender::new(
            tx,
            codex_analytics::AnalyticsEventsClient::disabled(),
        ));
        ThreadScopedOutgoingMessageSender::new(outgoing, vec![ConnectionId(1)], ThreadId::new())
    }

    #[test]
    fn rollback_response_rebuilds_pathless_thread_from_stored_history() -> Result<()> {
        let thread_id = ThreadId::from_string("00000000-0000-0000-0000-000000000789")?;
        let created_at = Utc::now();
        let history_items = vec![
            RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
                message: "before rollback".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            })),
            RolloutItem::EventMsg(EventMsg::AgentMessage(AgentMessageEvent {
                message: "after rollback".to_string(),
                phase: None,
                memory_citation: None,
            })),
        ];
        let stored_thread = StoredThread {
            thread_id,
            rollout_path: None,
            forked_from_id: None,
            preview: "fallback preview".to_string(),
            name: Some("Rollback thread".to_string()),
            model_provider: "openai".to_string(),
            model: None,
            reasoning_effort: None,
            created_at,
            updated_at: created_at,
            archived_at: None,
            cwd: test_path_buf("/tmp").abs().into(),
            cli_version: "0.0.0".to_string(),
            source: SessionSource::Cli,
            thread_source: None,
            agent_nickname: None,
            agent_role: None,
            agent_path: None,
            git_info: None,
            approval_mode: AskForApproval::OnRequest,
            sandbox_policy: SandboxPolicy::new_read_only_policy(),
            token_usage: None,
            first_user_message: Some("before rollback".to_string()),
            skills: Vec::new(),
            history: Some(StoredThreadHistory {
                thread_id,
                items: history_items,
            }),
        };
        let fallback_cwd = test_path_buf("/tmp").abs();

        let response = thread_rollback_response_from_stored_thread(
            stored_thread,
            thread_id.to_string(),
            "fallback-provider",
            &fallback_cwd,
            ThreadLifecycleStatus::NotLoaded,
        )
        .expect("rollback response should rebuild from stored history");

        assert_eq!(response.thread.id, thread_id.to_string());
        assert_eq!(response.thread.path, None);
        assert_eq!(response.thread.preview, "fallback preview");
        assert_eq!(response.thread.name.as_deref(), Some("Rollback thread"));
        assert_eq!(response.thread.lifecycle_status, ThreadLifecycleStatus::NotLoaded);
        assert_eq!(response.thread.turns.len(), 1);
        assert_eq!(response.thread.turns[0].items.len(), 2);
        Ok(())
    }

    fn turn_complete_event(turn_id: &str) -> TurnCompleteEvent {
        TurnCompleteEvent {
            turn_id: turn_id.to_string(),
            last_agent_message: None,
            completed_at: Some(TEST_TURN_COMPLETED_AT),
            duration_ms: Some(TEST_TURN_DURATION_MS),
            time_to_first_token_ms: None,
        }
    }

    fn turn_aborted_event(turn_id: &str) -> TurnAbortedEvent {
        TurnAbortedEvent {
            turn_id: Some(turn_id.to_string()),
            reason: protocol::protocol::TurnAbortReason::Interrupted,
            completed_at: Some(TEST_TURN_COMPLETED_AT),
            duration_ms: Some(TEST_TURN_DURATION_MS),
        }
    }

    fn command_execution_completion_item(command: &str) -> CommandExecutionCompletionItem {
        CommandExecutionCompletionItem {
            command: command.to_string(),
            cwd: test_path_buf("/tmp").abs(),
            command_actions: vec![V2ParsedCommand::Unknown {
                command: command.to_string(),
            }],
        }
    }

    fn guardian_command_assessment(
        id: &str,
        turn_id: &str,
        status: GuardianAssessmentStatus,
    ) -> GuardianAssessmentEvent {
        let (risk_level, user_authorization, rationale) = match status {
            GuardianAssessmentStatus::InProgress => (None, None, None),
            GuardianAssessmentStatus::Approved => (
                Some(protocol::protocol::GuardianRiskLevel::Low),
                Some(protocol::protocol::GuardianUserAuthorization::High),
                Some("looks safe".to_string()),
            ),
            GuardianAssessmentStatus::Denied => (
                Some(protocol::protocol::GuardianRiskLevel::High),
                Some(protocol::protocol::GuardianUserAuthorization::Low),
                Some("too risky".to_string()),
            ),
            GuardianAssessmentStatus::TimedOut => {
                (None, None, Some("review timed out".to_string()))
            }
            GuardianAssessmentStatus::Aborted => (None, None, None),
        };
        GuardianAssessmentEvent {
            id: format!("review-{id}"),
            target_item_id: Some(id.to_string()),
            turn_id: turn_id.to_string(),
            started_at_ms: 1_000,
            completed_at_ms: (!matches!(status, GuardianAssessmentStatus::InProgress))
                .then_some(1_042),
            status,
            risk_level,
            user_authorization,
            rationale,
            decision_source: if matches!(status, GuardianAssessmentStatus::InProgress) {
                None
            } else {
                Some(protocol::protocol::GuardianAssessmentDecisionSource::Agent)
            },
            action: serde_json::from_value(json!({
                "type": "command",
                "source": "shell",
                "command": format!("rm -f /tmp/{id}.sqlite"),
                "cwd": test_path_buf("/tmp"),
            }))
            .expect("guardian action"),
        }
    }

    struct GuardianAssessmentTestContext {
        conversation_id: ThreadId,
        conversation: Arc<CodexThread>,
        thread_service: Arc<ThreadService>,
        outgoing: ThreadScopedOutgoingMessageSender,
        thread_state: Arc<Mutex<ThreadState>>,
        thread_watch_manager: ThreadWatchManager,
    }

    impl GuardianAssessmentTestContext {
        async fn apply_guardian_assessment_event(&self, assessment: GuardianAssessmentEvent) {
            let event_turn_id = assessment.turn_id.clone();
            apply_bespoke_event_handling(
                Event {
                    id: event_turn_id,
                    msg: EventMsg::GuardianAssessment(assessment),
                },
                self.conversation_id,
                self.conversation.clone(),
                self.thread_service.clone(),
                self.thread_service.clone(),
                self.thread_service.clone(),
                self.thread_service.clone(),
                self.outgoing.clone(),
                self.thread_state.clone(),
                self.thread_watch_manager.clone(),
                Arc::new(tokio::sync::Semaphore::new(/*permits*/ 1)),
                "test-provider".to_string(),
            )
            .await;
        }
    }


mod guardian;
mod permissions;
mod runtime_events;
