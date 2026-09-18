use super::*;
use pretty_assertions::assert_eq;
use protocol::error::ModelInputItemReference;
use protocol::models::ContentItem;

async fn process_compacted_history_with_test_session(
    compacted_history: Vec<ResponseItem>,
    previous_turn_settings: Option<&PreviousTurnSettings>,
) -> (Vec<ResponseItem>, Vec<ResponseItem>) {
    let (session, turn_context) = crate::session::tests::make_session_and_context().await;
    session
        .set_previous_turn_settings(previous_turn_settings.cloned())
        .await;
    let initial_context = session.build_initial_context(&turn_context).await;
    let refreshed = crate::compact::process_compacted_history(
        &session,
        &turn_context,
        compacted_history,
        InitialContextInjection::BeforeLastUserMessage,
    )
    .await;
    (refreshed, initial_context)
}

fn user_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn assistant_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

#[test]
fn auto_compact_decision_gate_includes_hard_threshold() {
    let thresholds = SoftCompactThresholds::default();
    assert!(!should_evaluate_auto_compact_decision(
        thresholds.soft_lower_bound - 0.01,
        thresholds
    ));
    assert!(should_evaluate_auto_compact_decision(
        thresholds.soft_lower_bound,
        thresholds
    ));
    assert!(should_evaluate_auto_compact_decision(
        thresholds.hard_bound,
        thresholds
    ));
}

#[test]
fn auto_compact_decision_gate_uses_custom_thresholds() {
    let thresholds = SoftCompactThresholds::resolve(Some(0.60), Some(0.75)).unwrap();

    assert!(!should_evaluate_auto_compact_decision(0.59, thresholds));
    assert!(should_evaluate_auto_compact_decision(0.60, thresholds));
}

#[tokio::test]
async fn process_compacted_history_replaces_developer_messages() {
    let compacted_history = vec![
        ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: "stale permissions".to_string(),
            }],
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "summary".to_string(),
            }],
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: "stale personality".to_string(),
            }],
            phase: None,
        },
    ];
    let (refreshed, mut expected) = process_compacted_history_with_test_session(
        compacted_history,
        /*previous_turn_settings*/ None,
    )
    .await;
    expected.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    });
    assert_eq!(refreshed, expected);
}

#[test]
fn compact_prompt_control_item_is_not_a_user_message() {
    let prompt = "Custom prompt from COMPACT.md";
    let item = compact_prompt_control_item(prompt);

    match item {
        ResponseItem::Message { role, content, .. } => {
            assert_eq!(role, "developer");
            assert_eq!(
                content,
                vec![ContentItem::InputText {
                    text: prompt.to_string()
                }]
            );
        }
        other => panic!("expected compact prompt message, got {other:?}"),
    }
}

#[test]
fn compact_final_output_comes_from_current_compact_turn_only() {
    let prompt = "Summarize the conversation.";
    let history = vec![
        user_message("normal user message"),
        assistant_message("previous assistant reply"),
        compact_prompt_control_item(prompt),
        assistant_message("compact final output"),
    ];

    assert_eq!(
        compact_turn_final_output(&history, prompt).as_deref(),
        Some("compact final output")
    );
}

#[test]
fn compact_final_output_uses_custom_compact_prompt_control_item() {
    let prompt = "Custom compact prompt from COMPACT.md";
    let history = vec![
        user_message("normal user message"),
        compact_prompt_control_item(prompt),
        assistant_message("custom compact final output"),
    ];

    assert_eq!(
        compact_turn_final_output(&history, prompt).as_deref(),
        Some("custom compact final output")
    );
}

#[test]
fn compact_final_output_does_not_fall_back_to_previous_turn_assistant_message() {
    let prompt = "Summarize the conversation.";
    let history = vec![
        user_message("normal user message"),
        assistant_message("previous assistant reply"),
        compact_prompt_control_item(prompt),
    ];

    assert_eq!(compact_turn_final_output(&history, prompt), None);
}

#[tokio::test]
async fn process_compacted_history_reinjects_full_initial_context() {
    let compacted_history = vec![ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    }];
    let (refreshed, mut expected) = process_compacted_history_with_test_session(
        compacted_history,
        /*previous_turn_settings*/ None,
    )
    .await;
    expected.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    });
    assert_eq!(refreshed, expected);
}

#[test]
fn replacement_history_keeps_runtime_activity_initial_context_before_checkpoint() {
    let runtime_activity = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "<runtime_activity>\n  <running_commands count=\"1\" />\n</runtime_activity>"
                .to_string(),
        }],
        phase: None,
    };
    let checkpoint = user_message("summary");

    let replacement = prepend_initial_context_to_memory_checkpoint_history(
        vec![checkpoint.clone()],
        vec![runtime_activity.clone()],
    );

    assert_eq!(replacement, vec![runtime_activity, checkpoint]);
}

#[tokio::test]
async fn process_compacted_history_drops_non_user_content_messages() {
    let compacted_history = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: r#"# AGENTS.md instructions for /repo

<INSTRUCTIONS>
keep me updated
</INSTRUCTIONS>"#
                    .to_string(),
            }],
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: r#"<environment_context>
  <cwd>/repo</cwd>
  <shell>zsh</shell>
</environment_context>"#
                    .to_string(),
            }],
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: r#"<turn_aborted>
  <turn_id>turn-1</turn_id>
  <reason>interrupted</reason>
</turn_aborted>"#
                    .to_string(),
            }],
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "summary".to_string(),
            }],
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: "stale developer instructions".to_string(),
            }],
            phase: None,
        },
    ];
    let (refreshed, mut expected) = process_compacted_history_with_test_session(
        compacted_history,
        /*previous_turn_settings*/ None,
    )
    .await;
    expected.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    });
    assert_eq!(refreshed, expected);
}

#[tokio::test]
async fn process_compacted_history_drops_legacy_warnings() {
    let latest_user = user_message("latest user");
    let compacted_history = vec![
        user_message(
            "Warning: The maximum number of unified exec processes you can keep open is 60 and you currently have 61 processes open. Reuse older processes or close them to prevent automatic pruning of old processes",
        ),
        user_message(
            "Warning: apply_patch was requested via exec_command. Use the apply_patch tool instead of exec_command.",
        ),
        user_message(
            "Warning: Your account was flagged for potentially high-risk cyber activity and this request was routed to gpt-5.2 as a fallback. To regain access to gpt-5.3-codex, apply for trusted access: https://chatgpt.com/cyber or learn more: https://developers.openai.com/codex/concepts/cyber-safety",
        ),
        latest_user.clone(),
    ];
    let (refreshed, initial_context) = process_compacted_history_with_test_session(
        compacted_history,
        /*previous_turn_settings*/ None,
    )
    .await;
    let mut expected = initial_context;
    expected.push(latest_user);
    assert_eq!(refreshed, expected);
}

#[tokio::test]
async fn process_compacted_history_inserts_context_before_last_real_user_message_only() {
    let compacted_history = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "older user".to_string(),
            }],
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: format!("{SUMMARY_PREFIX}\nsummary text"),
            }],
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "latest user".to_string(),
            }],
            phase: None,
        },
    ];

    let (refreshed, initial_context) = process_compacted_history_with_test_session(
        compacted_history,
        /*previous_turn_settings*/ None,
    )
    .await;
    let mut expected = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "older user".to_string(),
            }],
            phase: None,
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: format!("{SUMMARY_PREFIX}\nsummary text"),
            }],
            phase: None,
        },
    ];
    expected.extend(initial_context);
    expected.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "latest user".to_string(),
        }],
        phase: None,
    });
    assert_eq!(refreshed, expected);
}

#[tokio::test]
async fn process_compacted_history_reinjects_model_switch_message() {
    let compacted_history = vec![ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    }];
    let previous_turn_settings = PreviousTurnSettings {
        model: "previous-regular-model".to_string(),
        realtime_active: None,
    };

    let (refreshed, initial_context) = process_compacted_history_with_test_session(
        compacted_history,
        Some(&previous_turn_settings),
    )
    .await;

    let ResponseItem::Message { role, content, .. } = &initial_context[0] else {
        panic!("expected developer message");
    };
    assert_eq!(role, "developer");
    let [ContentItem::InputText { text }, ..] = content.as_slice() else {
        panic!("expected developer text");
    };
    assert!(text.contains("<model_switch>"));

    let mut expected = initial_context;
    expected.push(ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    });
    assert_eq!(refreshed, expected);
}

#[tokio::test]
async fn process_compacted_history_reinjects_user_instructions_into_initial_context() {
    let (session, mut turn_context) = crate::session::tests::make_session_and_context().await;
    turn_context.user_instructions = Some("Loaded from instruction_files".to_string());
    session
        .set_user_instructions_for_test(Some("Loaded from instruction_files".to_string()))
        .await;
    let compacted_history = vec![ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    }];

    let refreshed = crate::compact::process_compacted_history(
        &session,
        &turn_context,
        compacted_history,
        InitialContextInjection::BeforeLastUserMessage,
    )
    .await;

    let initial_context_texts = refreshed
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { content, .. } => Some(
                content
                    .iter()
                    .filter_map(|part| match part {
                        ContentItem::InputText { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        initial_context_texts.contains("Loaded from instruction_files"),
        "expected reinjected initial context to preserve user instructions, got {initial_context_texts:?}"
    );
}

#[test]
fn prepend_initial_context_to_memory_checkpoint_history_keeps_checkpoint_block_contiguous() {
    let compacted_history = vec![
        user_message("recent user"),
        user_message(&format!("{SUMMARY_PREFIX}\nsummary text")),
        user_message("Memory checkpoint: current work\n# Current Work\n- item"),
    ];
    let initial_context = vec![ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: vec![ContentItem::InputText {
            text: "fresh permissions".to_string(),
        }],
        phase: None,
    }];

    let refreshed = prepend_initial_context_to_memory_checkpoint_history(
        compacted_history,
        initial_context.clone(),
    );

    let mut expected = initial_context;
    expected.push(user_message("recent user"));
    expected.push(user_message(&format!("{SUMMARY_PREFIX}\nsummary text")));
    expected.push(user_message(
        "Memory checkpoint: current work\n# Current Work\n- item",
    ));
    assert_eq!(refreshed, expected);
}

#[test]
fn compact_preserves_quarantine_when_retained_transaction_survives() {
    let call_id = "call-quarantined".to_string();
    let quarantine = ResponseItem::ModelContextQuarantine {
        target: ModelInputItemReference {
            kind: ModelInputItemKind::FunctionCall,
            call_id: call_id.clone(),
        }
        .into(),
        reason: "provider rejected transaction".to_string(),
        error_code: Some("invalid_value".to_string()),
        error_param: Some("input[0].arguments".to_string()),
    };
    let previous_history = vec![
        ResponseItem::FunctionCall {
            id: None,
            name: "lookup".to_string(),
            namespace: None,
            arguments: "{}".to_string(),
            call_id: call_id.clone(),
        },
        quarantine.clone(),
    ];
    let mut replacement_history = vec![ResponseItem::FunctionCall {
        id: None,
        name: "lookup".to_string(),
        namespace: None,
        arguments: "{}".to_string(),
        call_id,
    }];

    reconcile_model_context_quarantines(&previous_history, &mut replacement_history);

    assert_eq!(replacement_history.last(), Some(&quarantine));
}

#[test]
fn persisted_post_compact_items_follow_reconciled_history_without_summary_seed() {
    let call_id = "call-quarantined".to_string();
    let summary = assistant_message("compact summary");
    let call = ResponseItem::FunctionCall {
        id: None,
        name: "lookup".to_string(),
        namespace: None,
        arguments: "{}".to_string(),
        call_id: call_id.clone(),
    };
    let quarantine = ResponseItem::ModelContextQuarantine {
        target: ModelInputItemReference {
            kind: ModelInputItemKind::FunctionCall,
            call_id,
        }
        .into(),
        reason: "provider rejected transaction".to_string(),
        error_code: Some("invalid_value".to_string()),
        error_param: Some("input[0].arguments".to_string()),
    };
    let post_compact_history = vec![summary, call.clone(), quarantine.clone()];

    let persisted_items = persisted_post_compact_response_items(&post_compact_history);

    assert_eq!(persisted_items, vec![call, quarantine]);
}

#[test]
fn compact_drops_quarantine_marker_when_no_transaction_fragment_survives() {
    let quarantine = ResponseItem::ModelContextQuarantine {
        target: protocol::error::ModelContextQuarantineReference::ModelItem {
            kind: ModelInputItemKind::ToolSearchCall,
            call_id: None,
            item_id: None,
            fingerprint: "sha256:dropped".to_string(),
        },
        reason: "provider rejected transaction".to_string(),
        error_code: Some("invalid_value".to_string()),
        error_param: Some("input[0].arguments".to_string()),
    };
    let mut replacement_history = vec![quarantine];

    reconcile_model_context_quarantines(&[], &mut replacement_history);

    assert!(replacement_history.is_empty());
}

#[test]
fn compact_preserves_item_quarantine_when_fingerprinted_call_survives() {
    let call = ResponseItem::ToolSearchCall {
        id: None,
        call_id: None,
        status: Some("completed".to_string()),
        execution: "client".to_string(),
        arguments: serde_json::json!({"query": {"invalid": true}}),
    };
    let fingerprint = protocol::models::model_context_item_fingerprint(&call).expect("fingerprint");
    let quarantine = ResponseItem::ModelContextQuarantine {
        target: protocol::error::ModelContextQuarantineReference::ModelItem {
            kind: ModelInputItemKind::ToolSearchCall,
            call_id: None,
            item_id: None,
            fingerprint,
        },
        reason: "local parse failed".to_string(),
        error_code: Some("local_tool_call_parse_failed".to_string()),
        error_param: None,
    };
    let previous_history = vec![call.clone(), quarantine.clone()];
    let mut replacement_history = vec![call];

    reconcile_model_context_quarantines(&previous_history, &mut replacement_history);

    assert_eq!(replacement_history.last(), Some(&quarantine));
}
