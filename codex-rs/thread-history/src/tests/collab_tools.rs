use super::*;

    #[test]
    fn reconstructs_collab_resume_end_item() {
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "resume agent".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabResumeEnd(protocol::protocol::CollabResumeEndEvent {
                call_id: "resume-1".into(),
                completed_at_ms: 0,
                sender_thread_id: ThreadId::try_from("00000000-0000-0000-0000-000000000001")
                    .expect("valid sender thread id"),
                sender_agent_path: "/root".into(),
                receiver_thread_id: ThreadId::try_from("00000000-0000-0000-0000-000000000002")
                    .expect("valid receiver thread id"),
                receiver_agent_path: "/root/scout".into(),
                receiver_agent_nickname: None,
                receiver_agent_role: None,
                status: AgentStatus::Completed(None),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "resume-1".into(),
                tool: CollabAgentTool::ResumeAgent,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: "00000000-0000-0000-0000-000000000001".into(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec!["00000000-0000-0000-0000-000000000002".into()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: None,
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: [(
                    "00000000-0000-0000-0000-000000000002".into(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        lifecycle_status: ThreadLifecycleStatus::completed(None),
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn reconstructs_list_agents_call() {
        let sender = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "list agents".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabListAgentsBegin(protocol::protocol::CollabListAgentsBeginEvent {
                call_id: "list-agents-1".into(),
                started_at_ms: 0,
                sender_thread_id: sender,
                sender_agent_path: "/root".into(),
                path_prefix: Some("/root".into()),
            }),
            EventMsg::CollabListAgentsEnd(protocol::protocol::CollabListAgentsEndEvent {
                call_id: "list-agents-1".into(),
                completed_at_ms: 1,
                sender_thread_id: sender,
                sender_agent_path: "/root".into(),
                path_prefix: Some("/root".into()),
                success: true,
                agents: vec![protocol::protocol::CollabListedAgent {
                    agent_path: "/root/scout".into(),
                    status: AgentStatus::Completed(Some("done".into())),
                    last_task_message: Some("last task".into()),
                }],
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "list-agents-1".into(),
                tool: CollabAgentTool::ListAgents,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: sender.to_string(),
                sender_path: "/root".into(),
                receiver_thread_ids: Vec::new(),
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: None,
                prompt: Some("/root".into()),
                model: None,
                reasoning_effort: None,
                agents_states: [(
                    "/root/scout".into(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        lifecycle_status: ThreadLifecycleStatus::completed(None),
                        message: Some("done".into()),
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn reconstructs_failed_list_agents_call() {
        let sender = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "list agents".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabListAgentsBegin(protocol::protocol::CollabListAgentsBeginEvent {
                call_id: "list-agents-1".into(),
                started_at_ms: 0,
                sender_thread_id: sender,
                sender_agent_path: "/root".into(),
                path_prefix: None,
            }),
            EventMsg::CollabListAgentsEnd(protocol::protocol::CollabListAgentsEndEvent {
                call_id: "list-agents-1".into(),
                completed_at_ms: 1,
                sender_thread_id: sender,
                sender_agent_path: "/root".into(),
                path_prefix: None,
                success: false,
                agents: Vec::new(),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "list-agents-1".into(),
                tool: CollabAgentTool::ListAgents,
                status: CollabAgentToolCallStatus::Failed,
                sender_thread_id: sender.to_string(),
                sender_path: "/root".into(),
                receiver_thread_ids: Vec::new(),
                receiver_paths: Vec::new(),
                timeout_ms: None,
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: HashMap::new(),
            }
        );
    }

    #[test]
    fn reconstructs_collab_spawn_end_item_with_model_metadata() {
        let sender_thread_id = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let spawned_thread_id = ThreadId::try_from("00000000-0000-0000-0000-000000000002")
            .expect("valid receiver thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "spawn agent".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabAgentSpawnEnd(protocol::protocol::CollabAgentSpawnEndEvent {
                call_id: "spawn-1".into(),
                completed_at_ms: 0,
                sender_thread_id,
                sender_agent_path: "/root".into(),
                new_thread_id: Some(spawned_thread_id),
                new_agent_path: Some("/root/scout".into()),
                new_agent_nickname: Some("Scout".into()),
                new_agent_role: Some("explorer".into()),
                prompt: "inspect the repo".into(),
                model: "gpt-5.4-mini".into(),
                reasoning_effort: protocol::openai_models::ReasoningEffort::Medium,
                status: AgentStatus::Running,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "spawn-1".into(),
                tool: CollabAgentTool::SpawnAgent,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: "00000000-0000-0000-0000-000000000001".into(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec!["00000000-0000-0000-0000-000000000002".into()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: None,
                prompt: Some("inspect the repo".into()),
                model: Some("gpt-5.4-mini".into()),
                reasoning_effort: Some(protocol::openai_models::ReasoningEffort::Medium),
                agents_states: [(
                    "00000000-0000-0000-0000-000000000002".into(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        lifecycle_status: ThreadLifecycleStatus::Active { active_flags: Vec::new() },
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn reconstructs_collab_spawn_begin_and_end_as_one_completed_item() {
        let sender_thread_id = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let spawned_thread_id = ThreadId::try_from("00000000-0000-0000-0000-000000000002")
            .expect("valid receiver thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "spawn agent".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabAgentSpawnBegin(protocol::protocol::CollabAgentSpawnBeginEvent {
                call_id: "spawn-1".into(),
                started_at_ms: 0,
                sender_thread_id,
                sender_agent_path: "/root".into(),
                prompt: "inspect the repo".into(),
                model: "gpt-5.4-mini".into(),
                reasoning_effort: protocol::openai_models::ReasoningEffort::Medium,
            }),
            EventMsg::CollabAgentSpawnEnd(protocol::protocol::CollabAgentSpawnEndEvent {
                call_id: "spawn-1".into(),
                completed_at_ms: 1,
                sender_thread_id,
                sender_agent_path: "/root".into(),
                new_thread_id: Some(spawned_thread_id),
                new_agent_path: Some("/root/scout".into()),
                new_agent_nickname: Some("Scout".into()),
                new_agent_role: Some("explorer".into()),
                prompt: "inspect the repo".into(),
                model: "gpt-5.4-mini".into(),
                reasoning_effort: protocol::openai_models::ReasoningEffort::Medium,
                status: AgentStatus::Running,
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);

        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "spawn-1".into(),
                tool: CollabAgentTool::SpawnAgent,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: "00000000-0000-0000-0000-000000000001".into(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec!["00000000-0000-0000-0000-000000000002".into()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: None,
                prompt: Some("inspect the repo".into()),
                model: Some("gpt-5.4-mini".into()),
                reasoning_effort: Some(protocol::openai_models::ReasoningEffort::Medium),
                agents_states: [(
                    "00000000-0000-0000-0000-000000000002".into(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        lifecycle_status: ThreadLifecycleStatus::Active { active_flags: Vec::new() },
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn reconstructs_interrupted_send_input_as_completed_collab_call() {
        // `send_input(interrupt=true)` first stops the child's active turn, then redirects it with
        // new input. The transient interrupted status should remain visible in agent state, but the
        // collab tool call itself is still a successful redirect rather than a failed operation.
        let sender = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let receiver = ThreadId::try_from("00000000-0000-0000-0000-000000000002")
            .expect("valid receiver thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "redirect".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabAgentInteractionBegin(
                protocol::protocol::CollabAgentInteractionBeginEvent {
                    call_id: "send-1".into(),
                    started_at_ms: 0,
                    sender_thread_id: sender,
                    sender_agent_path: "/root".into(),
                    receiver_thread_id: receiver,
                    receiver_agent_path: "/root/scout".into(),
                    prompt: "new task".into(),
                },
            ),
            EventMsg::CollabAgentInteractionEnd(
                protocol::protocol::CollabAgentInteractionEndEvent {
                    call_id: "send-1".into(),
                    completed_at_ms: 0,
                    sender_thread_id: sender,
                    sender_agent_path: "/root".into(),
                    receiver_thread_id: receiver,
                    receiver_agent_path: "/root/scout".into(),
                    receiver_agent_nickname: None,
                    receiver_agent_role: None,
                    prompt: "new task".into(),
                    status: AgentStatus::Interrupted,
                },
            ),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "send-1".into(),
                tool: CollabAgentTool::SendInput,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: sender.to_string(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec![receiver.to_string()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: None,
                prompt: Some("new task".into()),
                model: None,
                reasoning_effort: None,
                agents_states: [(
                    receiver.to_string(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        lifecycle_status: ThreadLifecycleStatus::Final { result: ThreadLifecycleFinalStatus::Interrupted },
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }

    #[test]
    fn reconstructs_wait_call_with_timeout_and_receiver_path() {
        let sender = ThreadId::try_from("00000000-0000-0000-0000-000000000001")
            .expect("valid sender thread id");
        let receiver = ThreadId::try_from("00000000-0000-0000-0000-000000000002")
            .expect("valid receiver thread id");
        let events = vec![
            EventMsg::UserMessage(UserMessageEvent {
                message: "wait".into(),
                images: None,
                text_elements: Vec::new(),
                local_images: Vec::new(),
                skills: Vec::new(),
            }),
            EventMsg::CollabWaitingBegin(protocol::protocol::CollabWaitingBeginEvent {
                started_at_ms: 0,
                sender_thread_id: sender,
                sender_agent_path: "/root".into(),
                receiver_thread_ids: vec![receiver],
                receiver_agents: vec![protocol::protocol::CollabAgentRef {
                    thread_id: receiver,
                    agent_path: Some("/root/scout".into()),
                    agent_nickname: None,
                    agent_role: None,
                }],
                timeout_ms: 30_000,
                call_id: "wait-1".into(),
            }),
            EventMsg::CollabWaitingEnd(protocol::protocol::CollabWaitingEndEvent {
                sender_thread_id: sender,
                sender_agent_path: "/root".into(),
                call_id: "wait-1".into(),
                completed_at_ms: 1,
                timeout_ms: 30_000,
                agent_lifecycles: vec![protocol::protocol::CollabAgentLifecycleEntry {
                    thread_id: receiver,
                    agent_path: Some("/root/scout".into()),
                    agent_nickname: None,
                    agent_role: None,
                    lifecycle_status: ThreadLifecycleStatus::completed(None),
                }],
                lifecycle_statuses: [(receiver, ThreadLifecycleStatus::completed(None))]
                    .into_iter()
                    .collect(),
            }),
        ];

        let items = events
            .into_iter()
            .map(RolloutItem::EventMsg)
            .collect::<Vec<_>>();
        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 2);
        assert_eq!(
            turns[0].items[1],
            ThreadItem::CollabAgentToolCall {
                id: "wait-1".into(),
                tool: CollabAgentTool::Wait,
                status: CollabAgentToolCallStatus::Completed,
                sender_thread_id: sender.to_string(),
                sender_path: "/root".into(),
                receiver_thread_ids: vec![receiver.to_string()],
                receiver_paths: vec!["/root/scout".into()],
                timeout_ms: Some(30_000),
                prompt: None,
                model: None,
                reasoning_effort: None,
                agents_states: [(
                    receiver.to_string(),
                    CollabAgentState {
                        path: Some("/root/scout".into()),
                        lifecycle_status: ThreadLifecycleStatus::completed(None),
                        message: None,
                    },
                )]
                .into_iter()
                .collect(),
            }
        );
    }
