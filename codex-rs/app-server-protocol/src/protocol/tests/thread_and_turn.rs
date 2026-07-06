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
                replacement_history: None,
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
                {"type": "contextCompaction", "id": "item_2", "replacementHistory": null}
            ],
            "nextCursor": null,
            "backwardsCursor": "cursor_0",
        })
    );
}

#[test]
fn context_compaction_serializes_replacement_history() {
    let replacement_history = vec![ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "recent request".to_string(),
        }],
        phase: None,
    }];
    let item = ThreadItem::ContextCompaction {
        id: "item_3".to_string(),
        replacement_history: Some(serde_json::to_value(replacement_history).unwrap()),
    };

    assert_eq!(
        serde_json::to_value(&item).expect("serialize context compaction"),
        json!({
            "type": "contextCompaction",
            "id": "item_3",
            "replacementHistory": [
                {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "recent request",
                        }
                    ]
                }
            ]
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
            status: CollabAgentStatus::Interrupted,
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
            "sessionId": "thread-id",
            "forkedFromId": null,
            "preview": "",
            "ephemeral": false,
            "modelProvider": "openai",
            "createdAt": 1,
            "updatedAt": 1,
            "status": { "type": "idle" },
            "path": null,
            "cwd": absolute_path_string("tmp"),
            "cliVersion": "0.0.0",
            "source": "exec",
            "agentNickname": null,
            "agentRole": null,
            "gitInfo": null,
            "name": null,
            "turns": []
        },
        "model": "gpt-5",
        "modelProvider": "openai",
        "serviceTier": null,
        "cwd": absolute_path_string("tmp"),
        "approvalPolicy": "on-failure",
        "approvalsReviewer": "user",
        "sandbox": { "type": "dangerFullAccess" },
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
