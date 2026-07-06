use super::*;

#[tokio::test]
async fn subagent_thread_started_publishes_without_initialize() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::SubAgentThreadStarted(
                SubAgentThreadStartedInput {
                    thread_id: "thread-review".to_string(),
                    parent_thread_id: None,
                    product_client_id: "codex-tui".to_string(),
                    client_name: "codex-tui".to_string(),
                    client_version: "1.0.0".to_string(),
                    model: "gpt-5".to_string(),
                    ephemeral: false,
                    subagent_source: SubAgentSource::Review,
                    created_at: 127,
                },
            )),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_thread_initialized");
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["product_client_id"],
        "codex-tui"
    );
    assert_eq!(payload[0]["event_params"]["thread_source"], "subagent");
    assert_eq!(payload[0]["event_params"]["subagent_source"], "review");
}

#[tokio::test]
async fn subagent_thread_started_inherits_parent_connection_for_new_thread() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();
    let parent_thread_id = protocol::ThreadId::from_string("44444444-4444-4444-4444-444444444444")
        .expect("valid parent thread id");
    let parent_thread_id_string = parent_thread_id.to_string();

    reducer
        .ingest(
            AnalyticsFact::Initialize {
                connection_id: 7,
                params: InitializeParams {
                    client_info: ClientInfo {
                        name: "parent-client".to_string(),
                        title: None,
                        version: "1.0.0".to_string(),
                    },
                    capabilities: None,
                },
                product_client_id: "parent-client".to_string(),
                runtime: sample_runtime_metadata(),
                rpc_transport: AppServerRpcTransport::Stdio,
            },
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(1),
                response: Box::new(sample_thread_start_response(
                    &parent_thread_id_string,
                    /*ephemeral*/ false,
                    "gpt-5",
                )),
            },
            &mut events,
        )
        .await;

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::SubAgentThreadStarted(
                SubAgentThreadStartedInput {
                    thread_id: "thread-review".to_string(),
                    parent_thread_id: None,
                    product_client_id: "parent-client".to_string(),
                    client_name: "parent-client".to_string(),
                    client_version: "1.0.0".to_string(),
                    model: "gpt-5".to_string(),
                    ephemeral: false,
                    subagent_source: SubAgentSource::ThreadSpawn {
                        parent_thread_id,
                        depth: 1,
                        agent_path: None,
                        agent_nickname: None,
                        agent_role: None,
                    },
                    created_at: 130,
                },
            )),
            &mut events,
        )
        .await;

    events.clear();
    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::Compaction(Box::new(
                CodexCompactionEvent {
                    thread_id: "thread-review".to_string(),
                    turn_id: "turn-compact".to_string(),
                    trigger: CompactionTrigger::Manual,
                    reason: CompactionReason::UserRequested,
                    implementation: CompactionImplementation::Responses,
                    phase: CompactionPhase::StandaloneTurn,
                    strategy: CompactionStrategy::Memento,
                    status: CompactionStatus::Completed,
                    error: None,
                    active_context_tokens_before: 131_000,
                    active_context_tokens_after: 64_000,
                    started_at: 100,
                    completed_at: 101,
                    duration_ms: Some(1200),
                },
            ))),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["product_client_id"],
        "parent-client"
    );
    assert_eq!(
        payload[0]["event_params"]["parent_thread_id"],
        "44444444-4444-4444-4444-444444444444"
    );
}

#[tokio::test]
async fn subagent_tool_items_inherit_parent_connection_metadata() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    ingest_review_prerequisites(&mut reducer, &mut events).await;
    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::SubAgentThreadStarted(
                SubAgentThreadStartedInput {
                    thread_id: "thread-subagent".to_string(),
                    parent_thread_id: Some("thread-1".to_string()),
                    product_client_id: "codex-tui".to_string(),
                    client_name: "codex-tui".to_string(),
                    client_version: "1.0.0".to_string(),
                    model: "gpt-5".to_string(),
                    ephemeral: false,
                    subagent_source: SubAgentSource::Review,
                    created_at: 128,
                },
            )),
            &mut events,
        )
        .await;
    events.clear();
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_started_notification(
                "thread-subagent",
                "turn-subagent",
            ))),
            &mut events,
        )
        .await;

    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(ServerNotification::ItemStarted(
                ItemStartedNotification {
                    thread_id: "thread-subagent".to_string(),
                    turn_id: "turn-subagent".to_string(),
                    started_at_ms: 1_000,
                    item: sample_command_execution_item(
                        CommandExecutionStatus::InProgress,
                        /*exit_code*/ None,
                        /*duration_ms*/ None,
                    ),
                },
            ))),
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(ServerNotification::ItemCompleted(
                ItemCompletedNotification {
                    thread_id: "thread-subagent".to_string(),
                    turn_id: "turn-subagent".to_string(),
                    completed_at_ms: 1_042,
                    item: sample_command_execution_item(
                        CommandExecutionStatus::Completed,
                        Some(0),
                        Some(42),
                    ),
                },
            ))),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_command_execution_event");
    assert_eq!(payload[0]["event_params"]["thread_source"], "subagent");
    assert_eq!(payload[0]["event_params"]["subagent_source"], "review");
    assert_eq!(payload[0]["event_params"]["parent_thread_id"], "thread-1");
    assert_eq!(
        payload[0]["event_params"]["app_server_client"]["client_name"],
        "codex-tui"
    );
}

#[test]
fn plugin_used_event_serializes_expected_shape() {
    let tracking = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-3".to_string(),
        turn_id: "turn-3".to_string(),
    };
    let event = TrackEventRequest::PluginUsed(CodexPluginUsedEventRequest {
        event_type: "codex_plugin_used",
        event_params: codex_plugin_used_metadata(&tracking, sample_plugin_metadata()),
    });

    let payload = serde_json::to_value(&event).expect("serialize plugin used event");

    assert_eq!(
        payload,
        json!({
            "event_type": "codex_plugin_used",
            "event_params": {
                "plugin_id": "sample@test",
                "plugin_name": "sample",
                "marketplace_name": "test",
                "has_skills": true,
                "mcp_server_count": 2,
                "connector_ids": ["calendar", "drive"],
                "product_client_id": originator().value,
                "thread_id": "thread-3",
                "turn_id": "turn-3",
                "model_slug": "gpt-5"
            }
        })
    );
}

#[test]
fn plugin_management_event_serializes_expected_shape() {
    let event = TrackEventRequest::PluginInstalled(CodexPluginEventRequest {
        event_type: "codex_plugin_installed",
        event_params: codex_plugin_metadata(sample_plugin_metadata()),
    });

    let payload = serde_json::to_value(&event).expect("serialize plugin installed event");

    assert_eq!(
        payload,
        json!({
            "event_type": "codex_plugin_installed",
            "event_params": {
                "plugin_id": "sample@test",
                "plugin_name": "sample",
                "marketplace_name": "test",
                "has_skills": true,
                "mcp_server_count": 2,
                "connector_ids": ["calendar", "drive"],
                "product_client_id": originator().value
            }
        })
    );
}

#[test]
fn plugin_management_event_can_use_remote_plugin_id_override() {
    let mut plugin = sample_plugin_metadata();
    plugin.remote_plugin_id = Some("plugins~Plugin_remote".to_string());
    let event = TrackEventRequest::PluginInstalled(CodexPluginEventRequest {
        event_type: "codex_plugin_installed",
        event_params: codex_plugin_metadata(plugin),
    });

    let payload = serde_json::to_value(&event).expect("serialize plugin installed event");

    assert_eq!(
        payload["event_params"]["plugin_id"],
        "plugins~Plugin_remote"
    );
    assert_eq!(payload["event_params"]["plugin_name"], "sample");
    assert_eq!(payload["event_params"]["marketplace_name"], "test");
}

#[test]
fn hook_run_event_serializes_expected_shape() {
    let tracking = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-3".to_string(),
        turn_id: "turn-3".to_string(),
    };
    let event = TrackEventRequest::HookRun(CodexHookRunEventRequest {
        event_type: "codex_hook_run",
        event_params: codex_hook_run_metadata(
            &tracking,
            HookRunFact {
                event_name: HookEventName::PreToolUse,
                hook_source: HookSource::User,
                status: HookRunStatus::Completed,
            },
        ),
    });

    let payload = serde_json::to_value(&event).expect("serialize hook run event");

    assert_eq!(
        payload,
        json!({
            "event_type": "codex_hook_run",
            "event_params": {
                "thread_id": "thread-3",
                "turn_id": "turn-3",
                "model_slug": "gpt-5",
                "hook_name": "PreToolUse",
                "hook_source": "user",
                "status": "completed"
            }
        })
    );
}

#[test]
fn hook_run_metadata_maps_sources_and_statuses() {
    let tracking = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };

    let system = serde_json::to_value(codex_hook_run_metadata(
        &tracking,
        HookRunFact {
            event_name: HookEventName::SessionStart,
            hook_source: HookSource::System,
            status: HookRunStatus::Completed,
        },
    ))
    .expect("serialize system hook");
    let project = serde_json::to_value(codex_hook_run_metadata(
        &tracking,
        HookRunFact {
            event_name: HookEventName::Stop,
            hook_source: HookSource::Project,
            status: HookRunStatus::Blocked,
        },
    ))
    .expect("serialize project hook");
    let cloud_requirements = serde_json::to_value(codex_hook_run_metadata(
        &tracking,
        HookRunFact {
            event_name: HookEventName::Stop,
            hook_source: HookSource::CloudRequirements,
            status: HookRunStatus::Blocked,
        },
    ))
    .expect("serialize cloud requirements hook");
    let unknown = serde_json::to_value(codex_hook_run_metadata(
        &tracking,
        HookRunFact {
            event_name: HookEventName::UserPromptSubmit,
            hook_source: HookSource::Unknown,
            status: HookRunStatus::Failed,
        },
    ))
    .expect("serialize unknown hook");

    assert_eq!(system["hook_source"], "system");
    assert_eq!(system["status"], "completed");
    assert_eq!(project["hook_source"], "project");
    assert_eq!(project["status"], "blocked");
    assert_eq!(cloud_requirements["hook_source"], "cloud_requirements");
    assert_eq!(cloud_requirements["status"], "blocked");
    assert_eq!(unknown["hook_source"], "unknown");
    assert_eq!(unknown["status"], "failed");
}

#[test]
fn hook_run_metadata_maps_stopped_status() {
    let tracking = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };

    let stopped = serde_json::to_value(codex_hook_run_metadata(
        &tracking,
        HookRunFact {
            event_name: HookEventName::Stop,
            hook_source: HookSource::User,
            status: HookRunStatus::Stopped,
        },
    ))
    .expect("serialize stopped hook");

    assert_eq!(stopped["hook_source"], "user");
    assert_eq!(stopped["status"], "stopped");
}

#[test]
fn plugin_used_dedupe_is_keyed_by_turn_and_plugin() {
    let (sender, _receiver) = mpsc::channel(1);
    let queue = AnalyticsEventsQueue {
        sender,
        app_used_emitted_keys: Arc::new(Mutex::new(HashSet::new())),
        plugin_used_emitted_keys: Arc::new(Mutex::new(HashSet::new())),
    };
    let plugin = sample_plugin_metadata();

    let turn_1 = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };
    let turn_2 = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-2".to_string(),
    };

    assert_eq!(queue.should_enqueue_plugin_used(&turn_1, &plugin), true);
    assert_eq!(queue.should_enqueue_plugin_used(&turn_1, &plugin), false);
    assert_eq!(queue.should_enqueue_plugin_used(&turn_2, &plugin), true);
}

#[tokio::test]
async fn reducer_ingests_skill_invoked_fact() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();
    let tracking = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };
    let skill_path = PathBuf::from("/Users/abc/.codex/skills/doc/SKILL.md");
    let expected_skill_id = skill_id_for_local_skill(
        /*repo_url*/ None,
        /*repo_root*/ None,
        skill_path.as_path(),
        "doc",
    );

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::SkillInvoked(SkillInvokedInput {
                tracking,
                invocations: vec![SkillInvocation {
                    skill_name: "doc".to_string(),
                    skill_scope: protocol::protocol::SkillScope::User,
                    skill_path,
                    plugin_id: None,
                    invocation_type: InvocationType::Explicit,
                }],
            })),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(
        payload,
        json!([{
            "event_type": "skill_invocation",
            "skill_id": expected_skill_id,
            "skill_name": "doc",
            "event_params": {
                "product_client_id": originator().value,
                "skill_scope": "user",
                "plugin_id": null,
                "repo_url": null,
                "thread_id": "thread-1",
                "turn_id": "turn-1",
                "invoke_type": "explicit",
                "model_slug": "gpt-5"
            }
        }])
    );
}

#[tokio::test]
async fn reducer_includes_plugin_id_for_plugin_skill_invocations() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();
    let tracking = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };
    let skill_path =
        PathBuf::from("/Users/abc/.codex/plugins/cache/test/sample/skills/doc/SKILL.md");

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::SkillInvoked(SkillInvokedInput {
                tracking,
                invocations: vec![SkillInvocation {
                    skill_name: "sample:doc".to_string(),
                    skill_scope: protocol::protocol::SkillScope::User,
                    skill_path,
                    plugin_id: Some("sample@test".to_string()),
                    invocation_type: InvocationType::Explicit,
                }],
            })),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(
        payload[0]["event_params"]["plugin_id"],
        json!("sample@test")
    );
}

#[tokio::test]
async fn reducer_ingests_hook_run_fact() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::HookRun(HookRunInput {
                tracking: TrackEventsContext {
                    model_slug: "gpt-5".to_string(),
                    thread_id: "thread-1".to_string(),
                    turn_id: "turn-1".to_string(),
                },
                hook: HookRunFact {
                    event_name: HookEventName::PostToolUse,
                    hook_source: HookSource::Unknown,
                    status: HookRunStatus::Failed,
                },
            })),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 1);
    assert_eq!(payload[0]["event_type"], "codex_hook_run");
    assert_eq!(payload[0]["event_params"]["hook_name"], "PostToolUse");
    assert_eq!(payload[0]["event_params"]["hook_source"], "unknown");
    assert_eq!(payload[0]["event_params"]["status"], "failed");
}

#[tokio::test]
async fn reducer_ingests_app_and_plugin_facts() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();
    let tracking = TrackEventsContext {
        model_slug: "gpt-5".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    };

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::AppMentioned(AppMentionedInput {
                tracking: tracking.clone(),
                mentions: vec![AppInvocation {
                    connector_id: Some("calendar".to_string()),
                    app_name: Some("Calendar".to_string()),
                    invocation_type: Some(InvocationType::Explicit),
                }],
            })),
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::AppUsed(AppUsedInput {
                tracking: tracking.clone(),
                app: AppInvocation {
                    connector_id: Some("drive".to_string()),
                    app_name: Some("Drive".to_string()),
                    invocation_type: Some(InvocationType::Implicit),
                },
            })),
            &mut events,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::PluginUsed(PluginUsedInput {
                tracking,
                plugin: sample_plugin_metadata(),
            })),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(payload.as_array().expect("events array").len(), 3);
    assert_eq!(payload[0]["event_type"], "codex_app_mentioned");
    assert_eq!(payload[1]["event_type"], "codex_app_used");
    assert_eq!(payload[2]["event_type"], "codex_plugin_used");
}

#[tokio::test]
async fn reducer_ingests_plugin_state_changed_fact() {
    let mut reducer = AnalyticsReducer::default();
    let mut events = Vec::new();

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::PluginStateChanged(
                PluginStateChangedInput {
                    plugin: sample_plugin_metadata(),
                    state: PluginState::Disabled,
                },
            )),
            &mut events,
        )
        .await;

    let payload = serde_json::to_value(&events).expect("serialize events");
    assert_eq!(
        payload,
        json!([{
            "event_type": "codex_plugin_disabled",
            "event_params": {
                "plugin_id": "sample@test",
                "plugin_name": "sample",
                "marketplace_name": "test",
                "has_skills": true,
                "mcp_server_count": 2,
                "connector_ids": ["calendar", "drive"],
                "product_client_id": originator().value
            }
        }])
    );
}

#[test]
fn turn_event_serializes_expected_shape() {
    let event = TrackEventRequest::TurnEvent(Box::new(CodexTurnEventRequest {
        event_type: "codex_turn_event",
        event_params: crate::events::CodexTurnEventParams {
            thread_id: "thread-2".to_string(),
            turn_id: "turn-2".to_string(),
            app_server_client: sample_app_server_client_metadata(),
            runtime: sample_runtime_metadata(),
            submission_type: None,
            ephemeral: false,
            thread_source: Some(ThreadSource::User),
            initialization_mode: ThreadInitializationMode::New,
            subagent_source: None,
            parent_thread_id: None,
            model: Some("gpt-5".to_string()),
            model_provider: "openai".to_string(),
            sandbox_policy: Some("read_only"),
            reasoning_effort: Some("high".to_string()),
            reasoning_summary: Some("detailed".to_string()),
            service_tier: "flex".to_string(),
            approval_policy: "on-request".to_string(),
            approvals_reviewer: "auto_review".to_string(),
            sandbox_network_access: true,
            collaboration_mode: Some("plan"),
            personality: Some("pragmatic".to_string()),
            num_input_images: 2,
            is_first_turn: true,
            status: Some(TurnStatus::Completed),
            turn_error: None,
            steer_count: Some(0),
            total_tool_call_count: None,
            shell_command_count: None,
            file_change_count: None,
            mcp_tool_call_count: None,
            dynamic_tool_call_count: None,
            subagent_tool_call_count: None,
            web_search_count: None,
            image_generation_count: None,
            input_tokens: None,
            cached_input_tokens: None,
            output_tokens: None,
            reasoning_output_tokens: None,
            total_tokens: None,
            duration_ms: Some(1234),
            started_at: Some(455),
            completed_at: Some(456),
        },
    }));

    let payload = serde_json::to_value(&event).expect("serialize turn event");
    let expected = serde_json::from_str::<serde_json::Value>(
        r#"{
            "event_type": "codex_turn_event",
            "event_params": {
                "thread_id": "thread-2",
                "turn_id": "turn-2",
                "submission_type": null,
                "app_server_client": {
                    "product_client_id": "codex_cli_rs",
                    "client_name": "codex-tui",
                    "client_version": "1.0.0",
                    "rpc_transport": "stdio",
                    "experimental_api_enabled": true
                },
                "runtime": {
                    "codex_rs_version": "0.1.0",
                    "runtime_os": "macos",
                    "runtime_os_version": "15.3.1",
                    "runtime_arch": "aarch64"
                },
                "ephemeral": false,
                "thread_source": "user",
                "initialization_mode": "new",
                "subagent_source": null,
                "parent_thread_id": null,
                "model": "gpt-5",
                "model_provider": "openai",
                "sandbox_policy": "read_only",
                "reasoning_effort": "high",
                "reasoning_summary": "detailed",
                "service_tier": "flex",
                "approval_policy": "on-request",
                "approvals_reviewer": "auto_review",
                "sandbox_network_access": true,
                "collaboration_mode": "plan",
                "personality": "pragmatic",
                "num_input_images": 2,
                "is_first_turn": true,
                "status": "completed",
                "turn_error": null,
                "steer_count": 0,
                "total_tool_call_count": null,
                "shell_command_count": null,
                "file_change_count": null,
                "mcp_tool_call_count": null,
                "dynamic_tool_call_count": null,
                "subagent_tool_call_count": null,
                "web_search_count": null,
                "image_generation_count": null,
                "input_tokens": null,
                "cached_input_tokens": null,
                "output_tokens": null,
                "reasoning_output_tokens": null,
                "total_tokens": null,
                "duration_ms": 1234,
                "started_at": 455,
                "completed_at": 456
            }
        }"#,
    )
    .expect("parse expected turn event");

    assert_eq!(payload, expected);
}

#[tokio::test]
async fn accepted_turn_steer_emits_expected_event() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut out,
        /*include_initialize*/ true,
        /*include_resolved_config*/ false,
        /*include_started*/ false,
        /*include_token_usage*/ false,
    )
    .await;
    reducer
        .ingest(
            AnalyticsFact::ClientRequest {
                connection_id: 7,
                request_id: RequestId::Integer(4),
                request: Box::new(sample_turn_steer_request(
                    "thread-2", "turn-2", /*request_id*/ 4,
                )),
            },
            &mut out,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(4),
                response: Box::new(sample_turn_steer_response("turn-2")),
            },
            &mut out,
        )
        .await;

    assert_eq!(out.len(), 1);
    let payload = serde_json::to_value(&out[0]).expect("serialize turn steer event");
    assert_eq!(payload["event_type"], json!("codex_turn_steer_event"));
    assert_eq!(payload["event_params"]["thread_id"], json!("thread-2"));
    assert_eq!(payload["event_params"]["expected_turn_id"], json!("turn-2"));
    assert_eq!(payload["event_params"]["accepted_turn_id"], json!("turn-2"));
    assert_eq!(payload["event_params"]["num_input_images"], json!(1));
    assert_eq!(payload["event_params"]["result"], json!("accepted"));
    assert_eq!(payload["event_params"]["rejection_reason"], json!(null));
    assert!(
        payload["event_params"]["created_at"]
            .as_u64()
            .expect("created_at")
            > 0
    );
    assert_eq!(
        payload["event_params"]["app_server_client"]["product_client_id"],
        json!("codex-tui")
    );
    assert_eq!(
        payload["event_params"]["runtime"]["codex_rs_version"],
        json!("0.1.0")
    );
    assert_eq!(payload["event_params"]["thread_source"], json!("user"));
    assert_eq!(payload["event_params"]["subagent_source"], json!(null));
    assert_eq!(payload["event_params"]["parent_thread_id"], json!(null));
    assert!(payload["event_params"].get("product_client_id").is_none());
}

#[tokio::test]
async fn rejected_turn_steer_uses_request_connection_metadata() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();
    let payload = ingest_rejected_turn_steer(
        &mut reducer,
        &mut out,
        no_active_turn_steer_error(),
        Some(no_active_turn_steer_error_type()),
    )
    .await;

    assert_eq!(payload["event_type"], json!("codex_turn_steer_event"));
    assert_eq!(payload["event_params"]["thread_id"], json!("thread-2"));
    assert_eq!(payload["event_params"]["expected_turn_id"], json!("turn-2"));
    assert_eq!(payload["event_params"]["accepted_turn_id"], json!(null));
    assert_eq!(payload["event_params"]["num_input_images"], json!(1));
    assert_eq!(
        payload["event_params"]["app_server_client"]["product_client_id"],
        json!("codex-tui")
    );
    assert_eq!(
        payload["event_params"]["runtime"]["codex_rs_version"],
        json!("0.1.0")
    );
    assert_eq!(payload["event_params"]["thread_source"], json!("user"));
    assert_eq!(payload["event_params"]["subagent_source"], json!(null));
    assert_eq!(payload["event_params"]["parent_thread_id"], json!(null));
    assert_eq!(payload["event_params"]["result"], json!("rejected"));
    assert_eq!(
        payload["event_params"]["rejection_reason"],
        json!("no_active_turn")
    );
    assert!(
        payload["event_params"]["created_at"]
            .as_u64()
            .expect("created_at")
            > 0
    );
}

#[tokio::test]
async fn rejected_turn_steer_maps_active_turn_not_steerable_error_type() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();
    let payload = ingest_rejected_turn_steer(
        &mut reducer,
        &mut out,
        non_steerable_review_error(),
        Some(non_steerable_review_error_type()),
    )
    .await;

    assert_eq!(
        payload["event_params"]["rejection_reason"],
        json!("non_steerable_review")
    );
}

#[tokio::test]
async fn rejected_turn_steer_maps_input_too_large_error_type() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();
    let payload = ingest_rejected_turn_steer(
        &mut reducer,
        &mut out,
        input_too_large_steer_error(),
        Some(input_too_large_error_type()),
    )
    .await;

    assert_eq!(
        payload["event_params"]["rejection_reason"],
        json!("input_too_large")
    );
}

#[tokio::test]
async fn turn_steer_does_not_emit_without_pending_request() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    reducer
        .ingest(
            AnalyticsFact::ErrorResponse {
                connection_id: 7,
                request_id: RequestId::Integer(4),
                error: no_active_turn_steer_error(),
                error_type: Some(no_active_turn_steer_error_type()),
            },
            &mut out,
        )
        .await;

    assert!(out.is_empty());
}

#[tokio::test]
async fn turn_start_error_response_discards_pending_start_request() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_initialize(&mut reducer, &mut out).await;
    reducer
        .ingest(
            AnalyticsFact::ClientRequest {
                connection_id: 7,
                request_id: RequestId::Integer(3),
                request: Box::new(sample_turn_start_request("thread-2", /*request_id*/ 3)),
            },
            &mut out,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::ErrorResponse {
                connection_id: 7,
                request_id: RequestId::Integer(3),
                error: no_active_turn_steer_error(),
                error_type: None,
            },
            &mut out,
        )
        .await;

    // A late/synthetic response for the same request id must not resurrect the
    // failed turn/start request and attach request-scoped connection metadata.
    reducer
        .ingest(
            AnalyticsFact::ClientResponse {
                connection_id: 7,
                request_id: RequestId::Integer(3),
                response: Box::new(sample_turn_start_response("turn-2")),
            },
            &mut out,
        )
        .await;
    assert!(out.is_empty());

    reducer
        .ingest(
            AnalyticsFact::Custom(CustomAnalyticsFact::TurnResolvedConfig(Box::new(
                sample_turn_resolved_config("thread-2", "turn-2"),
            ))),
            &mut out,
        )
        .await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut out,
        )
        .await;

    assert!(out.is_empty());
}

#[tokio::test]
async fn turn_lifecycle_emits_turn_event() {
    let mut reducer = AnalyticsReducer::default();
    let mut out = Vec::new();

    ingest_turn_prerequisites(
        &mut reducer,
        &mut out,
        /*include_initialize*/ true,
        /*include_resolved_config*/ true,
        /*include_started*/ true,
        /*include_token_usage*/ true,
    )
    .await;
    reducer
        .ingest(
            AnalyticsFact::Notification(Box::new(sample_turn_completed_notification(
                "thread-2",
                "turn-2",
                AppServerTurnStatus::Completed,
                /*codex_error_info*/ None,
            ))),
            &mut out,
        )
        .await;

    assert_eq!(out.len(), 1);
    let payload = serde_json::to_value(&out[0]).expect("serialize turn event");
    assert_eq!(payload["event_type"], json!("codex_turn_event"));
    assert_eq!(payload["event_params"]["thread_id"], json!("thread-2"));
    assert_eq!(payload["event_params"]["turn_id"], json!("turn-2"));
    assert_eq!(
        payload["event_params"]["app_server_client"],
        json!({
            "product_client_id": "codex-tui",
            "client_name": "codex-tui",
            "client_version": "1.0.0",
            "rpc_transport": "stdio",
            "experimental_api_enabled": null,
        })
    );
    assert_eq!(
        payload["event_params"]["runtime"],
        json!({
            "codex_rs_version": "0.1.0",
            "runtime_os": "macos",
            "runtime_os_version": "15.3.1",
            "runtime_arch": "aarch64",
        })
    );
    assert!(payload["event_params"].get("product_client_id").is_none());
    assert_eq!(payload["event_params"]["ephemeral"], json!(false));
    assert_eq!(payload["event_params"]["num_input_images"], json!(1));
    assert_eq!(payload["event_params"]["status"], json!("completed"));
    assert_eq!(payload["event_params"]["steer_count"], json!(0));
    assert_eq!(payload["event_params"]["total_tool_call_count"], json!(0));
    assert_eq!(payload["event_params"]["shell_command_count"], json!(0));
    assert_eq!(payload["event_params"]["file_change_count"], json!(0));
    assert_eq!(payload["event_params"]["mcp_tool_call_count"], json!(0));
    assert_eq!(payload["event_params"]["dynamic_tool_call_count"], json!(0));
    assert_eq!(
        payload["event_params"]["subagent_tool_call_count"],
        json!(0)
    );
    assert_eq!(payload["event_params"]["web_search_count"], json!(0));
    assert_eq!(payload["event_params"]["image_generation_count"], json!(0));
    assert_eq!(payload["event_params"]["started_at"], json!(455));
    assert_eq!(payload["event_params"]["completed_at"], json!(456));
    assert_eq!(payload["event_params"]["duration_ms"], json!(1234));
    assert_eq!(payload["event_params"]["input_tokens"], json!(123));
    assert_eq!(payload["event_params"]["cached_input_tokens"], json!(45));
    assert_eq!(payload["event_params"]["output_tokens"], json!(140));
    assert_eq!(
        payload["event_params"]["reasoning_output_tokens"],
        json!(13)
    );
    assert_eq!(payload["event_params"]["total_tokens"], json!(321));
}
