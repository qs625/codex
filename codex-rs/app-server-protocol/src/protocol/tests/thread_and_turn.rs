use super::*;

#[test]
fn approvals_reviewer_serializes_auto_review_and_accepts_legacy_guardian_subagent() {
    assert_eq!(
        serde_json::to_string(&ApprovalsReviewer::User).expect("serialize reviewer"),
        "\"user\""
    );
    assert_eq!(
        serde_json::to_string(&ApprovalsReviewer::AutoReview).expect("serialize reviewer"),
        "\"guardian_subagent\""
    );

    for value in ["user", "auto_review", "guardian_subagent"] {
        let json = format!("\"{value}\"");
        let reviewer: ApprovalsReviewer =
            serde_json::from_str(&json).expect("deserialize reviewer");
        let expected = if value == "user" {
            ApprovalsReviewer::User
        } else {
            ApprovalsReviewer::AutoReview
        };
        assert_eq!(expected, reviewer);
    }
}

#[test]
fn turn_defaults_legacy_missing_items_view_to_full() {
    let turn: Turn = serde_json::from_value(json!({
        "id": "turn_123",
        "items": [],
        "status": "completed",
        "error": null,
        "startedAt": null,
        "completedAt": null,
        "durationMs": null,
    }))
    .expect("legacy turn should deserialize");

    assert_eq!(turn.items_view, TurnItemsView::Full);
}

#[test]
fn thread_turns_list_params_accepts_items_view() {
    let params = serde_json::from_value::<ThreadTurnsListParams>(json!({
        "threadId": "thr_123",
        "cursor": null,
        "limit": 25,
        "sortDirection": "desc",
        "itemsView": "notLoaded",
    }))
    .expect("thread turns list params should deserialize");

    assert_eq!(params.thread_id, "thr_123");
    assert_eq!(params.items_view, Some(TurnItemsView::NotLoaded));
}

#[test]
fn thread_accepts_canonical_thread_identity_fields() {
    let mut value = minimal_thread_json();
    let object = value.as_object_mut().expect("thread JSON object");
    object.remove("id");
    object.remove("sessionId");

    let thread: Thread = serde_json::from_value(value)
        .expect("canonical thread identity payload should deserialize");

    assert_eq!(thread.id, "thread-id");
    assert_eq!(thread.session_id, "runtime-tree-id");
}

#[test]
fn thread_accepts_legacy_thread_identity_fields() {
    let mut value = minimal_thread_json();
    let object = value.as_object_mut().expect("thread JSON object");
    object.remove("threadId");
    object.remove("runtimeSessionId");

    let thread: Thread =
        serde_json::from_value(value).expect("legacy thread identity payload should deserialize");

    assert_eq!(thread.id, "thread-id");
    assert_eq!(thread.session_id, "runtime-tree-id");
}

#[test]
fn thread_identity_round_trips_when_legacy_and_canonical_fields_are_present() {
    let thread: Thread =
        serde_json::from_value(minimal_thread_json()).expect("thread should deserialize");
    let value = serde_json::to_value(&thread).expect("thread should serialize");

    assert_eq!(value["id"], "thread-id");
    assert_eq!(value["threadId"], "thread-id");
    assert_eq!(value["sessionId"], "runtime-tree-id");
    assert_eq!(value["runtimeSessionId"], "runtime-tree-id");

    let round_tripped: Thread =
        serde_json::from_value(value).expect("serialized thread should deserialize");
    assert_eq!(round_tripped.id, "thread-id");
    assert_eq!(round_tripped.session_id, "runtime-tree-id");
}

#[test]
fn thread_identity_rejects_conflicting_legacy_and_canonical_fields() {
    let mut value = minimal_thread_json();
    value["threadId"] = json!("other-thread-id");

    let error = serde_json::from_value::<Thread>(value).expect_err("conflicting ids should fail");
    assert!(
        error.to_string().contains("conflicting id and threadId"),
        "unexpected error: {error}"
    );
}

#[test]
fn thread_identity_rejects_conflicting_runtime_identity_fields() {
    let mut value = minimal_thread_json();
    value["runtimeSessionId"] = json!("other-runtime-tree-id");

    let error =
        serde_json::from_value::<Thread>(value).expect_err("conflicting runtime ids should fail");
    assert!(
        error
            .to_string()
            .contains("conflicting sessionId and runtimeSessionId"),
        "unexpected error: {error}"
    );
}

fn minimal_thread_json() -> serde_json::Value {
    json!({
        "threadId": "thread-id",
        "id": "thread-id",
        "runtimeSessionId": "runtime-tree-id",
        "sessionId": "runtime-tree-id",
        "forkedFromId": null,
        "preview": "",
        "ephemeral": false,
        "modelProvider": "openai",
        "createdAt": 1,
        "updatedAt": 1,
        "lifecycleStatus": {
            "type": "final",
            "result": { "type": "completed" }
        },
        "path": null,
        "cwd": absolute_path_string("tmp"),
        "cliVersion": "0.0.0",
        "source": "exec",
        "threadSource": null,
        "agentNickname": null,
        "agentRole": null,
        "agentPath": null,
        "gitInfo": null,
        "name": null,
        "skills": [],
        "tokenUsage": null,
        "contextUsage": null,
        "turns": []
    })
}

#[test]
fn thread_turns_items_list_round_trips() {
    let params = ThreadTurnsItemsListParams {
        thread_id: "thr_123".to_string(),
        turn_id: "turn_456".to_string(),
        cursor: Some("cursor_1".to_string()),
        limit: Some(50),
        sort_direction: Some(SortDirection::Asc),
    };

    assert_eq!(
        serde_json::to_value(&params).expect("serialize params"),
        json!({
            "threadId": "thr_123",
            "turnId": "turn_456",
            "cursor": "cursor_1",
            "limit": 50,
            "sortDirection": "asc",
        })
    );
    let response = ThreadTurnsItemsListResponse {
        data: vec![
            ThreadItem::InjectedContext {
                id: "item_1".to_string(),
                title: "Initial context injected".to_string(),
                preview: "Permissions • Environment".to_string(),
                sections: vec![
                    InjectedContextSection {
                        label: "Permissions".to_string(),
                        text: "Sandbox: workspace-write".to_string(),
                    },
                    InjectedContextSection {
                        label: "Environment".to_string(),
                        text: "<cwd>/workspace</cwd>".to_string(),
                    },
                ],
            },
            ThreadItem::ContextCompaction {
                id: "item_2".to_string(),
            },
        ],
        next_cursor: None,
        backwards_cursor: Some("cursor_0".to_string()),
    };

    assert_eq!(
        serde_json::to_value(&response).expect("serialize response"),
        json!({
            "data": [
                {
                    "type": "injectedContext",
                    "id": "item_1",
                    "title": "Initial context injected",
                    "preview": "Permissions • Environment",
                    "sections": [
                        {
                            "label": "Permissions",
                            "text": "Sandbox: workspace-write",
                        },
                        {
                            "label": "Environment",
                            "text": "<cwd>/workspace</cwd>",
                        }
                    ]
                },
                {"type": "contextCompaction", "id": "item_2"}
            ],
            "nextCursor": null,
            "backwardsCursor": "cursor_0",
        })
    );
}

#[test]
fn thread_provider_capabilities_serializes_fork_thread_as_camel_case() {
    let capabilities = ThreadProviderCapabilities {
        start_thread: true,
        send_input: true,
        close_thread: true,
        list_children: true,
        restore_thread: true,
        restore_snapshot: true,
        event_stream: true,
        spawn_child: true,
        compact: true,
        workflow: true,
        poll_event: true,
        command_session: true,
        permissions: true,
        dynamic_tools: true,
        fork_thread: true,
    };

    let value = serde_json::to_value(capabilities).expect("serialize capabilities");
    assert_eq!(value["forkThread"], true);
    assert_eq!(value.get("fork_thread"), None);
}

#[test]
fn context_compaction_serializes_marker_only() {
    let item = ThreadItem::ContextCompaction {
        id: "item_3".to_string(),
    };

    assert_eq!(
        serde_json::to_value(&item).expect("serialize context compaction"),
        json!({
            "type": "contextCompaction",
            "id": "item_3"
        })
    );
}

#[test]
fn thread_list_params_accepts_single_cwd() {
    let params = serde_json::from_value::<ThreadListParams>(json!({
        "cwd": "/workspace",
    }))
    .expect("single cwd should deserialize");

    assert_eq!(
        params.cwd,
        Some(ThreadListCwdFilter::One("/workspace".to_string()))
    );
    assert!(!params.use_state_db_only);
}

#[test]
fn thread_list_params_accepts_multiple_cwds() {
    let params = serde_json::from_value::<ThreadListParams>(json!({
        "cwd": ["/workspace", "/other-workspace"],
    }))
    .expect("cwd array should deserialize");

    assert_eq!(
        params.cwd,
        Some(ThreadListCwdFilter::Many(vec![
            "/workspace".to_string(),
            "/other-workspace".to_string(),
        ]))
    );
}

#[test]
fn thread_list_params_accepts_state_db_only_flag() {
    let params = serde_json::from_value::<ThreadListParams>(json!({
        "useStateDbOnly": true,
    }))
    .expect("state db only flag should deserialize");

    assert!(params.use_state_db_only);
}

#[test]
fn collab_agent_state_maps_interrupted_status() {
    assert_eq!(
        CollabAgentState::from(CoreAgentStatus::Interrupted),
        CollabAgentState {
            path: None,
            agent_nickname: None,
            agent_role: None,
            lifecycle_status: ThreadLifecycleStatus::Final {
                result: ThreadLifecycleFinalStatus::Interrupted
            },
            message: None,
        }
    );
}

#[test]
fn external_agent_config_plugins_details_round_trip() {
    let item: ExternalAgentConfigMigrationItem = serde_json::from_value(json!({
        "itemType": "PLUGINS",
        "description": "Install supported plugins from Claude settings",
        "cwd": absolute_path_string("repo"),
        "details": {
            "plugins": [
                {
                    "marketplaceName": "team-marketplace",
                    "pluginNames": ["asana"]
                }
            ]
        }
    }))
    .expect("plugins migration item should deserialize");

    assert_eq!(
        item,
        ExternalAgentConfigMigrationItem {
            item_type: ExternalAgentConfigMigrationItemType::Plugins,
            description: "Install supported plugins from Claude settings".to_string(),
            cwd: Some(PathBuf::from(absolute_path_string("repo"))),
            details: Some(MigrationDetails {
                plugins: vec![PluginsMigration {
                    marketplace_name: "team-marketplace".to_string(),
                    plugin_names: vec!["asana".to_string()],
                }],
                ..Default::default()
            }),
        }
    );
}

#[test]
fn external_agent_config_import_params_accept_legacy_plugin_details() {
    let params: ExternalAgentConfigImportParams = serde_json::from_value(json!({
        "migrationItems": [{
            "itemType": "PLUGINS",
            "description": "Install supported plugins from Claude settings",
            "cwd": absolute_path_string("repo"),
            "details": {
                "plugins": [
                    {
                        "marketplaceName": "team-marketplace",
                        "pluginNames": ["asana"]
                    }
                ]
            }
        }]
    }))
    .expect("legacy plugin import params should deserialize");

    assert_eq!(
        params,
        ExternalAgentConfigImportParams {
            migration_items: vec![ExternalAgentConfigMigrationItem {
                item_type: ExternalAgentConfigMigrationItemType::Plugins,
                description: "Install supported plugins from Claude settings".to_string(),
                cwd: Some(PathBuf::from(absolute_path_string("repo"))),
                details: Some(MigrationDetails {
                    plugins: vec![PluginsMigration {
                        marketplace_name: "team-marketplace".to_string(),
                        plugin_names: vec!["asana".to_string()],
                    }],
                    ..Default::default()
                }),
            }],
        }
    );
}

#[test]

fn thread_start_params_preserve_explicit_null_service_tier() {
    let params: ThreadStartParams =
        serde_json::from_value(json!({ "serviceTier": null })).expect("params should deserialize");
    assert_eq!(params.service_tier, Some(None));

    let serialized = serde_json::to_value(&params).expect("params should serialize");
    assert_eq!(
        serialized.get("serviceTier"),
        Some(&serde_json::Value::Null)
    );

    let serialized_without_override =
        serde_json::to_value(ThreadStartParams::default()).expect("params should serialize");
    assert_eq!(serialized_without_override.get("serviceTier"), None);
}

#[test]
fn thread_lifecycle_responses_default_missing_optional_fields() {
    let response = json!({
        "thread": {
            "id": "thread-id",
            "threadId": "thread-id",
            "sessionId": "thread-id",
            "runtimeSessionId": "thread-id",
            "forkedFromId": null,
            "preview": "",
            "ephemeral": false,
            "modelProvider": "openai",
            "createdAt": 1,
            "updatedAt": 1,
            "lifecycleStatus": {
                "type": "final",
                "result": {
                    "type": "completed"
                }
            },
            "path": null,
            "cwd": absolute_path_string("tmp"),
            "cliVersion": "0.0.0",
            "source": "exec",
            "agentNickname": null,
            "agentRole": null,
            "agentPath": null,
            "gitInfo": null,
            "name": null,
            "skills": [],
            "tokenUsage": null,
            "contextUsage": null,
            "turns": []
        },
        "model": "gpt-5",
        "modelProvider": "openai",
        "serviceTier": null,
        "cwd": absolute_path_string("tmp"),
        "runtimeWorkspaceRoots": [],
        "instructionSources": [],
        "approvalPolicy": "on-failure",
        "approvalsReviewer": "user",
        "sandbox": { "type": "dangerFullAccess" },
        "permissionProfile": null,
        "activePermissionProfile": null,
        "reasoningEffort": null
    });

    let start: ThreadStartResponse =
        serde_json::from_value(response.clone()).expect("thread/start response");
    let resume: ThreadResumeResponse =
        serde_json::from_value(response.clone()).expect("thread/resume response");
    let fork: ThreadForkResponse = serde_json::from_value(response).expect("thread/fork response");

    assert_eq!(start.instruction_sources, Vec::<AbsolutePathBuf>::new());
    assert_eq!(resume.instruction_sources, Vec::<AbsolutePathBuf>::new());
    assert_eq!(fork.instruction_sources, Vec::<AbsolutePathBuf>::new());
    assert_eq!(start.permission_profile, None);
    assert_eq!(resume.permission_profile, None);
    assert_eq!(fork.permission_profile, None);
    assert_eq!(start.active_permission_profile, None);
    assert_eq!(resume.active_permission_profile, None);
    assert_eq!(fork.active_permission_profile, None);
}

#[test]
fn turn_start_params_preserve_explicit_null_service_tier() {
    let params: TurnStartParams = serde_json::from_value(json!({
        "threadId": "thread_123",
        "input": [],
        "serviceTier": null
    }))
    .expect("params should deserialize");
    assert_eq!(params.service_tier, Some(None));

    let serialized = serde_json::to_value(&params).expect("params should serialize");
    assert_eq!(
        serialized.get("serviceTier"),
        Some(&serde_json::Value::Null)
    );

    let without_override = TurnStartParams {
        thread_id: "thread_123".to_string(),
        input: vec![],
        responsesapi_client_metadata: None,
        environments: None,
        cwd: None,
        runtime_workspace_roots: None,
        approval_policy: None,
        approvals_reviewer: None,
        sandbox_policy: None,
        permissions: None,
        model: None,
        model_provider: None,
        service_tier: None,
        effort: None,
        summary: None,
        output_schema: None,
        collaboration_mode: None,
        personality: None,
    };
    let serialized_without_override =
        serde_json::to_value(&without_override).expect("params should serialize");
    assert_eq!(serialized_without_override.get("serviceTier"), None);
}

#[test]
fn turn_start_params_round_trip_environments() {
    let cwd = test_absolute_path();
    let params: TurnStartParams = serde_json::from_value(json!({
        "threadId": "thread_123",
        "input": [],
        "environments": [
            {
                "environmentId": "local",
                "cwd": cwd
            }
        ],
    }))
    .expect("params should deserialize");

    assert_eq!(
        params.environments,
        Some(vec![TurnEnvironmentParams {
            environment_id: "local".to_string(),
            cwd: cwd.clone(),
        }])
    );
    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&params),
        Some("turn/start.environments")
    );

    let serialized = serde_json::to_value(&params).expect("params should serialize");
    assert_eq!(
        serialized.get("environments"),
        Some(&json!([
            {
                "environmentId": "local",
                "cwd": cwd
            }
        ]))
    );
}

#[test]
fn turn_start_params_preserve_empty_environments() {
    let params: TurnStartParams = serde_json::from_value(json!({
        "threadId": "thread_123",
        "input": [],
        "environments": [],
    }))
    .expect("params should deserialize");

    assert_eq!(params.environments, Some(Vec::new()));
    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&params),
        Some("turn/start.environments")
    );

    let serialized = serde_json::to_value(&params).expect("params should serialize");
    assert_eq!(serialized.get("environments"), Some(&json!([])));
}

#[test]
fn turn_start_params_treat_null_or_omitted_environments_as_default() {
    let null_environments: TurnStartParams = serde_json::from_value(json!({
        "threadId": "thread_123",
        "input": [],
        "environments": null,
    }))
    .expect("params should deserialize");
    let omitted_environments: TurnStartParams = serde_json::from_value(json!({
        "threadId": "thread_123",
        "input": [],
    }))
    .expect("params should deserialize");

    assert_eq!(null_environments.environments, None);
    assert_eq!(omitted_environments.environments, None);
    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&null_environments),
        None
    );
    assert_eq!(
        crate::experimental_api::ExperimentalApi::experimental_reason(&omitted_environments),
        None
    );
}

#[test]
fn turn_start_params_reject_relative_environment_cwd() {
    let err = serde_json::from_value::<TurnStartParams>(json!({
        "threadId": "thread_123",
        "input": [],
        "environments": [
            {
                "environmentId": "local",
                "cwd": "relative"
            }
        ],
    }))
    .expect_err("relative environment cwd should fail");

    assert!(
        err.to_string()
            .contains("AbsolutePathBuf deserialized without a base path"),
        "unexpected error: {err}"
    );
}

fn raw_subagent_notification_message() -> String {
    concat!(
        "<subagent_notification>\n",
        r#"{"agent_path":"/root/worker","status":{"completed":"done"}}"#,
        "\n</subagent_notification>"
    )
    .to_string()
}

fn nullable_inter_agent_envelope_message() -> String {
    serde_json::json!({
        "author": "/cp_http_api/frontend_taskstatus_fix_2",
        "recipient": "/cp_http_api",
        "other_recipients": [],
        "content": "typecheck is available ...",
        "content_parts": [],
        "operation": null,
        "trigger_turn": false,
        "sender_thread_id": null,
        "recipient_thread_id": null,
        "status": null,
        "lifecycle_status": null,
        "agent_nickname": null,
        "agent_role": null,
    })
    .to_string()
}

#[test]
fn thread_history_filters_raw_subagent_notification_user_message() {
    let items = vec![protocol::protocol::RolloutItem::EventMsg(
        protocol::protocol::EventMsg::UserMessage(protocol::protocol::UserMessageEvent {
            message: raw_subagent_notification_message(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        }),
    )];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert!(turns.is_empty());
}

#[test]
fn thread_history_preserves_user_message_that_mentions_subagent_notification_marker() {
    let message = "Please inspect <subagent_notification> output".to_string();
    let items = vec![protocol::protocol::RolloutItem::EventMsg(
        protocol::protocol::EventMsg::UserMessage(protocol::protocol::UserMessageEvent {
            message: message.clone(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        }),
    )];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].items,
        vec![ThreadItem::UserMessage {
            id: "item-1".into(),
            content: vec![UserInput::Text {
                text: message,
                text_elements: Vec::new(),
            }],
        }]
    );
}

#[test]
fn thread_history_preserves_raw_marker_text_with_user_message_metadata() {
    let skill_path = PathBuf::from("/tmp/skills/demo/SKILL.md");
    let items = vec![protocol::protocol::RolloutItem::EventMsg(
        protocol::protocol::EventMsg::UserMessage(protocol::protocol::UserMessageEvent {
            message: raw_subagent_notification_message(),
            images: None,
            local_images: Vec::new(),
            skills: vec![protocol::protocol::UserMessageSkill {
                name: "demo".into(),
                path: skill_path.clone(),
            }],
            text_elements: Vec::new(),
        }),
    )];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].items[0],
        ThreadItem::UserMessage {
            id: "item-1".into(),
            content: vec![
                UserInput::Skill {
                    name: "demo".into(),
                    path: skill_path,
                },
                UserInput::Text {
                    text: raw_subagent_notification_message(),
                    text_elements: Vec::new(),
                },
            ],
        }
    );
}

#[test]
fn thread_history_filters_nullable_raw_inter_agent_envelope_agent_message() {
    let items = vec![protocol::protocol::RolloutItem::EventMsg(
        protocol::protocol::EventMsg::AgentMessage(protocol::protocol::AgentMessageEvent {
            message: nullable_inter_agent_envelope_message(),
            phase: None,
            memory_citation: None,
        }),
    )];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert!(turns.is_empty());
}

#[test]
fn thread_history_filters_nullable_raw_inter_agent_envelope_user_message() {
    let items = vec![protocol::protocol::RolloutItem::EventMsg(
        protocol::protocol::EventMsg::UserMessage(protocol::protocol::UserMessageEvent {
            message: nullable_inter_agent_envelope_message(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        }),
    )];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert!(turns.is_empty());
}

#[test]
fn thread_history_preserves_ordinary_agent_json_with_operation_field() {
    let message = serde_json::json!({
        "author": "/root/worker",
        "recipient": "/root",
        "content": "plain assistant json",
        "operation": "sendMessage",
    })
    .to_string();
    let items = vec![protocol::protocol::RolloutItem::EventMsg(
        protocol::protocol::EventMsg::AgentMessage(protocol::protocol::AgentMessageEvent {
            message: message.clone(),
            phase: None,
            memory_citation: None,
        }),
    )];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].items,
        vec![ThreadItem::AgentMessage {
            id: "item-1".into(),
            text: message,
            phase: None,
            memory_citation: None,
        }]
    );
}

#[test]
fn thread_history_preserves_ordinary_user_json_with_operation_field() {
    let message = serde_json::json!({
        "author": "/root/worker",
        "recipient": "/root",
        "content": "plain user json",
        "operation": "sendMessage",
    })
    .to_string();
    let items = vec![protocol::protocol::RolloutItem::EventMsg(
        protocol::protocol::EventMsg::UserMessage(protocol::protocol::UserMessageEvent {
            message: message.clone(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        }),
    )];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].items,
        vec![ThreadItem::UserMessage {
            id: "item-1".into(),
            content: vec![UserInput::Text {
                text: message,
                text_elements: Vec::new(),
            }],
        }]
    );
}

#[test]
fn thread_history_rebuilds_read_agent_builtin_tool_item() {
    let output = json!({
        "target": "/root/worker",
        "agentName": "/root/worker",
        "agentNickname": "Worker",
        "agentRole": "feature-owner",
        "lifecycleStatus": {
            "type": "final",
            "result": {
                "type": "completed"
            }
        },
        "lastTaskMessage": "do work",
        "lastAgentMessage": "done"
    });
    let items = vec![
        protocol::protocol::RolloutItem::EventMsg(protocol::protocol::EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            },
        )),
        protocol::protocol::RolloutItem::EventMsg(
            protocol::protocol::EventMsg::BuiltinToolCallStarted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: protocol::ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "read-agent-1".into(),
                    tool: "read_agent".into(),
                    arguments: json!({
                        "target": "/root/worker",
                    }),
                    status: protocol::protocol::BuiltinToolCallStatus::InProgress,
                    output: None,
                    lifecycle_at_ms: 100,
                },
            ),
        ),
        protocol::protocol::RolloutItem::EventMsg(
            protocol::protocol::EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: protocol::ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "read-agent-1".into(),
                    tool: "read_agent".into(),
                    arguments: json!({
                        "target": "/root/worker",
                    }),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(output.clone()),
                    lifecycle_at_ms: 123,
                },
            ),
        ),
    ];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(
        turns[0].items,
        vec![ThreadItem::BuiltinToolCall {
            id: "read-agent-1".into(),
            tool: "read_agent".into(),
            arguments: json!({
                "target": "/root/worker",
            }),
            status: DynamicToolCallStatus::Completed,
            output: Some(output),
        }]
    );
}

#[test]
fn thread_history_does_not_project_active_schedule_subscription_metadata() {
    let items = vec![protocol::protocol::RolloutItem::SessionMeta(
        protocol::protocol::SessionMetaLine {
            meta: protocol::protocol::SessionMeta {
                subscriptions: Some(vec![
                    protocol::subscriptions::PersistedSubscription::Schedule {
                        subscription_id: "sub-schedule".into(),
                        schedule: protocol::subscriptions::ScheduleSpec::EveryInterval {
                            interval_ms: 60_000,
                        },
                        label: Some("standup".into()),
                        message: Some("Clean worktrees.".into()),
                    },
                ]),
                ..Default::default()
            },
            git: None,
        },
    )];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert!(turns.is_empty());
}

#[test]
fn thread_history_projects_active_schedule_subscription_event_after_compact() {
    let items = vec![
        protocol::protocol::RolloutItem::EventMsg(protocol::protocol::EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            },
        )),
        protocol::protocol::RolloutItem::EventMsg(
            protocol::protocol::EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: protocol::ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "call-schedule".into(),
                    tool: "schedule_subscribe".into(),
                    arguments: json!({
                        "schedule": {
                            "kind": "every_interval",
                            "interval_ms": 60000,
                        },
                        "label": "standup",
                    }),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(json!({
                        "subscription_id": "sub-schedule",
                    })),
                    lifecycle_at_ms: 123,
                },
            ),
        ),
        protocol::protocol::RolloutItem::Compacted(protocol::protocol::CompactedItem {
            message: "old display history compacted".into(),
            replacement_history: Some(Vec::new()),
            visible_replacement_history_len: None,
        }),
    ];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);
    let active_subscriptions = turns.last().expect("active subscriptions turn");

    assert_eq!(active_subscriptions.id, "active-subscriptions");
    assert_eq!(
        active_subscriptions.items,
        vec![ThreadItem::BuiltinToolCall {
            id: "call-schedule".into(),
            tool: "schedule_subscribe".into(),
            arguments: json!({
                "schedule": {
                    "kind": "every_interval",
                    "interval_ms": 60000,
                },
                "label": "standup",
            }),
            status: DynamicToolCallStatus::Completed,
            output: Some(json!({
                "subscription_id": "sub-schedule",
            })),
        }]
    );
}

#[test]
fn thread_history_rebinds_compact_summary_when_turn_context_precedes_compacted_event() {
    let turn_id = "01a0d249-650b-73f3-89ab-7074cb750ed9";
    let init_context_id = "5c5f34a2-5543-43a0-b7b1-db49bd3cadbb";
    let items = vec![
        protocol::protocol::RolloutItem::Compacted(protocol::protocol::CompactedItem {
            message: "summary".into(),
            replacement_history: None,
            visible_replacement_history_len: None,
        }),
        protocol::protocol::RolloutItem::TurnContext(protocol::protocol::TurnContextItem {
            turn_id: Some(turn_id.into()),
            trace_id: None,
            cwd: PathBuf::from("/tmp"),
            current_date: None,
            timezone: None,
            approval_policy: protocol::protocol::AskForApproval::Never,
            sandbox_policy: protocol::protocol::SandboxPolicy::DangerFullAccess,
            permission_profile: None,
            network: None,
            file_system_sandbox_policy: None,
            model: "test-model".into(),
            personality: None,
            collaboration_mode: None,
            realtime_active: None,
            effort: None,
            summary: protocol::config_types::ReasoningSummary::Auto,
            user_instructions: None,
            developer_instructions: None,
            init_context_snapshot: None,
            final_output_json_schema: None,
            truncation_policy: None,
        }),
        protocol::protocol::RolloutItem::EventMsg(protocol::protocol::EventMsg::ContextCompacted(
            protocol::protocol::ContextCompactedEvent {},
        )),
        protocol::protocol::RolloutItem::EventMsg(protocol::protocol::EventMsg::ItemCompleted(
            protocol::protocol::ItemCompletedEvent {
                thread_id: protocol::ThreadId::from_string(
                    "00000000-0000-0000-0000-000000000001",
                )
                .expect("valid thread id"),
                turn_id: turn_id.into(),
                item: protocol::items::TurnItem::InjectedContext(
                    protocol::items::InjectedContextItem {
                        id: init_context_id.into(),
                        title: "Init Context".into(),
                        preview: "Permissions • Apps • Skills".into(),
                        sections: vec![protocol::items::InjectedContextSection {
                            label: "Permissions".into(),
                            text: "danger-full-access".into(),
                        }],
                    },
                ),
                completed_at_ms: 1,
            },
        )),
    ];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].id, turn_id);
    assert_eq!(
        turns[0].items,
        vec![
            ThreadItem::ContextCompaction {
                id: "item-1".into(),
            },
            ThreadItem::AgentMessage {
                id: "item-1:summary".into(),
                text: "summary".into(),
                phase: None,
                memory_citation: None,
            },
            ThreadItem::InjectedContext {
                id: init_context_id.into(),
                title: "Init Context".into(),
                preview: "Permissions • Apps • Skills".into(),
                sections: vec![InjectedContextSection {
                    label: "Permissions".into(),
                    text: "danger-full-access".into(),
                }],
            },
        ]
    );
}

#[test]
fn thread_history_does_not_duplicate_existing_schedule_monitor_from_subscription_snapshot() {
    let items = vec![
        protocol::protocol::RolloutItem::EventMsg(protocol::protocol::EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: "turn-1".into(),
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            },
        )),
        protocol::protocol::RolloutItem::EventMsg(
            protocol::protocol::EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: protocol::ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "call-schedule".into(),
                    tool: "schedule_subscribe".into(),
                    arguments: json!({
                        "schedule": {
                            "kind": "every_interval",
                            "interval_ms": 60000,
                        },
                        "label": "standup",
                    }),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(json!({
                        "subscription_id": "sub-schedule",
                    })),
                    lifecycle_at_ms: 123,
                },
            ),
        ),
        protocol::protocol::RolloutItem::SessionMeta(protocol::protocol::SessionMetaLine {
            meta: protocol::protocol::SessionMeta {
                subscriptions: Some(vec![
                    protocol::subscriptions::PersistedSubscription::Schedule {
                        subscription_id: "sub-schedule".into(),
                        schedule: protocol::subscriptions::ScheduleSpec::EveryInterval {
                            interval_ms: 60_000,
                        },
                        label: Some("standup".into()),
                        message: None,
                    },
                ]),
                ..Default::default()
            },
            git: None,
        }),
    ];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    let schedule_items = turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter(|item| {
            matches!(
                item,
                ThreadItem::BuiltinToolCall {
                    tool,
                    output: Some(output),
                    ..
                } if tool == "schedule_subscribe"
                    && output.get("subscription_id").and_then(|value| value.as_str())
                        == Some("sub-schedule")
            )
        })
        .count();
    assert_eq!(schedule_items, 1);
}

#[test]
fn thread_history_empty_subscription_snapshot_does_not_project_inactive_schedule_cleanup() {
    let items = vec![
        protocol::protocol::RolloutItem::EventMsg(
            protocol::protocol::EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: protocol::ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "call-schedule".into(),
                    tool: "schedule_subscribe".into(),
                    arguments: json!({}),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(json!({
                        "subscription_id": "sub-schedule",
                    })),
                    lifecycle_at_ms: 123,
                },
            ),
        ),
        protocol::protocol::RolloutItem::SessionMeta(protocol::protocol::SessionMetaLine {
            meta: protocol::protocol::SessionMeta {
                subscriptions: Some(Vec::new()),
                ..Default::default()
            },
            git: None,
        }),
    ];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);
    let inactive_items = turns
        .iter()
        .flat_map(|turn| turn.items.iter())
        .filter(|item| {
            matches!(
                item,
                ThreadItem::BuiltinToolCall {
                    tool,
                    output: Some(output),
                    ..
                } if tool == "schedule_unsubscribe"
                    && output.get("subscription_id").and_then(|value| value.as_str())
                        == Some("sub-schedule")
            )
        })
        .count();

    assert_eq!(inactive_items, 0);
}

#[test]
fn thread_history_subscription_snapshot_none_does_not_clear_existing_schedule_monitor() {
    let items = vec![
        protocol::protocol::RolloutItem::EventMsg(
            protocol::protocol::EventMsg::BuiltinToolCallCompleted(
                protocol::protocol::BuiltinToolCallDisplayEvent {
                    thread_id: protocol::ThreadId::new(),
                    turn_id: "turn-1".into(),
                    id: "call-schedule".into(),
                    tool: "schedule_subscribe".into(),
                    arguments: json!({}),
                    status: protocol::protocol::BuiltinToolCallStatus::Completed,
                    output: Some(json!({
                        "subscription_id": "sub-schedule",
                    })),
                    lifecycle_at_ms: 123,
                },
            ),
        ),
        protocol::protocol::RolloutItem::SessionMeta(protocol::protocol::SessionMetaLine {
            meta: protocol::protocol::SessionMeta {
                subscriptions: None,
                ..Default::default()
            },
            git: None,
        }),
    ];

    let turns = crate::protocol::thread_history::build_turns_from_rollout_items(&items);

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].items.len(), 1);
    assert!(matches!(
        &turns[0].items[0],
        ThreadItem::BuiltinToolCall { id, .. } if id == "call-schedule"
    ));
}

#[test]
fn live_projection_filters_raw_subagent_notification_user_item() {
    let event =
        protocol::protocol::EventMsg::ItemCompleted(protocol::protocol::ItemCompletedEvent {
            thread_id: protocol::ThreadId::new(),
            turn_id: "turn-1".into(),
            item: protocol::items::TurnItem::UserMessage(protocol::items::UserMessageItem {
                id: "user-1".into(),
                content: vec![protocol::user_input::UserInput::Text {
                    text: raw_subagent_notification_message(),
                    text_elements: Vec::new(),
                }],
            }),
            completed_at_ms: 1,
        });

    assert!(crate::protocol::event_item_projection::project_event_msg_item(&event).is_none());
}

#[test]
fn live_projection_filters_nullable_raw_inter_agent_envelope_agent_item() {
    let event =
        protocol::protocol::EventMsg::ItemCompleted(protocol::protocol::ItemCompletedEvent {
            thread_id: protocol::ThreadId::new(),
            turn_id: "turn-1".into(),
            item: protocol::items::TurnItem::AgentMessage(protocol::items::AgentMessageItem {
                id: "agent-1".into(),
                content: vec![protocol::items::AgentMessageContent::Text {
                    text: nullable_inter_agent_envelope_message(),
                }],
                phase: None,
                memory_citation: None,
            }),
            completed_at_ms: 1,
        });

    assert!(crate::protocol::event_item_projection::project_event_msg_item(&event).is_none());
}

#[test]
fn live_projection_preserves_ordinary_agent_json_with_operation_field() {
    let message = serde_json::json!({
        "author": "/root/worker",
        "recipient": "/root",
        "content": "plain assistant json",
        "operation": "sendMessage",
    })
    .to_string();
    let event =
        protocol::protocol::EventMsg::ItemCompleted(protocol::protocol::ItemCompletedEvent {
            thread_id: protocol::ThreadId::new(),
            turn_id: "turn-1".into(),
            item: protocol::items::TurnItem::AgentMessage(protocol::items::AgentMessageItem {
                id: "agent-1".into(),
                content: vec![protocol::items::AgentMessageContent::Text {
                    text: message.clone(),
                }],
                phase: None,
                memory_citation: None,
            }),
            completed_at_ms: 1,
        });

    let projected =
        crate::protocol::event_item_projection::project_event_msg_item(&event).expect("projected");
    let crate::protocol::event_item_projection::ProjectedEventItem::Completed { item, .. } =
        projected
    else {
        panic!("expected completed item");
    };

    assert_eq!(
        item,
        ThreadItem::AgentMessage {
            id: "agent-1".into(),
            text: message,
            phase: None,
            memory_citation: None,
        }
    );
}

#[test]
fn live_projection_maps_typed_conversation_artifact() {
    let event =
        protocol::protocol::EventMsg::ItemCompleted(protocol::protocol::ItemCompletedEvent {
            thread_id: protocol::ThreadId::new(),
            turn_id: "turn-1".into(),
            item: protocol::items::TurnItem::ConversationArtifact(
                protocol::items::ConversationArtifactItem {
                    id: "artifact-1".into(),
                    title: "Inline chart".into(),
                    source: None,
                    mime_type: "image/svg+xml".into(),
                    content: "<svg viewBox=\"0 0 10 10\"></svg>".into(),
                    language: Some("svg".into()),
                    truncated: false,
                },
            ),
            completed_at_ms: 1,
        });

    let projected =
        crate::protocol::event_item_projection::project_event_msg_item(&event).expect("projected");
    let crate::protocol::event_item_projection::ProjectedEventItem::Completed { item, .. } =
        projected
    else {
        panic!("expected completed item");
    };

    assert_eq!(
        item,
        ThreadItem::ConversationArtifact {
            id: "artifact-1".into(),
            title: "Inline chart".into(),
            source: Some(protocol::items::ConversationArtifactSource::Inline {
                content: "<svg viewBox=\"0 0 10 10\"></svg>".into(),
                mime_type: "image/svg+xml".into(),
                language: Some("svg".into()),
                truncated: false,
            }),
            mime_type: "image/svg+xml".into(),
            content: "<svg viewBox=\"0 0 10 10\"></svg>".into(),
            language: Some("svg".into()),
            truncated: false,
        }
    );
}

#[test]
fn live_projection_preserves_user_item_that_mentions_subagent_notification_marker() {
    let message = "Please inspect <subagent_notification> output".to_string();
    let event =
        protocol::protocol::EventMsg::ItemCompleted(protocol::protocol::ItemCompletedEvent {
            thread_id: protocol::ThreadId::new(),
            turn_id: "turn-1".into(),
            item: protocol::items::TurnItem::UserMessage(protocol::items::UserMessageItem {
                id: "user-1".into(),
                content: vec![protocol::user_input::UserInput::Text {
                    text: message.clone(),
                    text_elements: Vec::new(),
                }],
            }),
            completed_at_ms: 1,
        });

    let projected =
        crate::protocol::event_item_projection::project_event_msg_item(&event).expect("projected");
    let crate::protocol::event_item_projection::ProjectedEventItem::Completed { item, .. } =
        projected
    else {
        panic!("expected completed item");
    };

    assert_eq!(
        item,
        ThreadItem::UserMessage {
            id: "user-1".into(),
            content: vec![UserInput::Text {
                text: message,
                text_elements: Vec::new(),
            }],
        }
    );
}

#[test]
fn client_recovery_projects_to_non_tool_thread_item() {
    let event =
        protocol::protocol::EventMsg::ClientRecovery(protocol::protocol::ClientRecoveryEvent {
            recovery_id: "recovery-1".to_string(),
            activation_id: "activation-1".to_string(),
            release_id: "release-2".to_string(),
            reason: "health check failed".to_string(),
            occurred_at: "2026-09-09T08:30:00.000Z".to_string(),
            fallback_release_id: Some("release-1".to_string()),
        });

    assert_eq!(
        crate::protocol::event_item_projection::project_event_msg_item(&event),
        Some(
            crate::protocol::event_item_projection::ProjectedEventItem::Completed {
                turn_id: "activation-1".to_string(),
                item: ThreadItem::ClientRecovery {
                    id: "recovery-1".to_string(),
                    activation_id: "activation-1".to_string(),
                    release_id: "release-2".to_string(),
                    reason: "health check failed".to_string(),
                    occurred_at: "2026-09-09T08:30:00.000Z".to_string(),
                    fallback_release_id: Some("release-1".to_string()),
                },
                completed_at_ms: chrono::DateTime::parse_from_rfc3339("2026-09-09T08:30:00.000Z",)
                    .expect("valid timestamp")
                    .timestamp_millis(),
            }
        )
    );
}
