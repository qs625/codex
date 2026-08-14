use super::*;
use pretty_assertions::assert_eq;
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
