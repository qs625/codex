use super::parse_turn_item;
use codex_protocol::AgentPath;
use codex_protocol::event_command::EventCommandEvent;
use codex_protocol::event_command::EventCommandEventKind;
use codex_protocol::event_driven_tool::EventDrivenToolTrigger;
use codex_protocol::items::AgentMessageContent;
use codex_protocol::items::HookPromptFragment;
use codex_protocol::items::TurnItem;
use codex_protocol::items::WebSearchItem;
use codex_protocol::items::build_hook_prompt_message;
use codex_protocol::models::ContentItem;
use codex_protocol::models::DEFAULT_IMAGE_DETAIL;
use codex_protocol::models::ReasoningItemContent;
use codex_protocol::models::ReasoningItemReasoningSummary;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::WebSearchAction;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::InterAgentCommunication;
use codex_protocol::protocol::InterAgentOperation;
use codex_protocol::user_input::UserInput;
use pretty_assertions::assert_eq;

#[test]
fn parses_user_message_with_text_and_two_images() {
    let img1 = "https://example.com/one.png".to_string();
    let img2 = "https://example.com/two.jpg".to_string();

    let item = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![
            ContentItem::InputText {
                text: "Hello world".to_string(),
            },
            ContentItem::InputImage {
                image_url: img1.clone(),
                detail: Some(DEFAULT_IMAGE_DETAIL),
            },
            ContentItem::InputImage {
                image_url: img2.clone(),
                detail: Some(DEFAULT_IMAGE_DETAIL),
            },
        ],
        phase: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected user message turn item");

    match turn_item {
        TurnItem::UserMessage(user) => {
            let expected_content = vec![
                UserInput::Text {
                    text: "Hello world".to_string(),
                    text_elements: Vec::new(),
                },
                UserInput::Image { image_url: img1 },
                UserInput::Image { image_url: img2 },
            ];
            assert_eq!(user.content, expected_content);
        }
        other => panic!("expected TurnItem::UserMessage, got {other:?}"),
    }
}

#[test]
fn skips_local_image_label_text() {
    let image_url = "data:image/png;base64,abc".to_string();
    let label = codex_protocol::models::local_image_open_tag_text(/*label_number*/ 1);
    let user_text = "Please review this image.".to_string();

    let item = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![
            ContentItem::InputText { text: label },
            ContentItem::InputImage {
                image_url: image_url.clone(),
                detail: Some(DEFAULT_IMAGE_DETAIL),
            },
            ContentItem::InputText {
                text: "</image>".to_string(),
            },
            ContentItem::InputText {
                text: user_text.clone(),
            },
        ],
        phase: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected user message turn item");

    match turn_item {
        TurnItem::UserMessage(user) => {
            let expected_content = vec![
                UserInput::Image { image_url },
                UserInput::Text {
                    text: user_text,
                    text_elements: Vec::new(),
                },
            ];
            assert_eq!(user.content, expected_content);
        }
        other => panic!("expected TurnItem::UserMessage, got {other:?}"),
    }
}

#[test]
fn parses_assistant_message_input_text_for_backward_compatibility() {
    let item = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::InputText {
            text: "author: /root\nrecipient: /root/worker\nother_recipients: []\nContent: continue"
                .to_string(),
        }],
        phase: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected assistant message turn item");

    match turn_item {
        TurnItem::AgentMessage(message) => {
            let rendered = message
                .content
                .into_iter()
                .map(|content| {
                    let AgentMessageContent::Text { text } = content;
                    text
                })
                .collect::<Vec<_>>();
            assert_eq!(
                rendered,
                vec![
                    "author: /root\nrecipient: /root/worker\nother_recipients: []\nContent: continue"
                        .to_string()
                ]
            );
        }
        other => panic!("expected TurnItem::AgentMessage, got {other:?}"),
    }
}

#[test]
fn parses_typed_event_driven_tool_as_typed_turn_item() {
    let trigger = EventDrivenToolTrigger {
        tool: "process_exit_subscribe".to_string(),
        title: "Process exited".to_string(),
        text: "Session 42 exited with code 0".to_string(),
    };
    let item = ResponseItem::EventDrivenTool {
        id: Some("event-1".to_string()),
        trigger,
    };

    let turn_item = parse_turn_item(&item).expect("expected event-driven tool turn item");

    match turn_item {
        TurnItem::EventDrivenTool(event_driven_tool) => {
            assert_eq!(event_driven_tool.id, "event-1");
            assert_eq!(event_driven_tool.tool, "process_exit_subscribe");
            assert_eq!(event_driven_tool.title, "Process exited");
            assert_eq!(event_driven_tool.text, "Session 42 exited with code 0");
        }
        other => panic!("expected TurnItem::EventDrivenTool, got {other:?}"),
    }
}

#[test]
fn keeps_event_driven_tool_marker_as_user_message() {
    let trigger = EventDrivenToolTrigger {
        tool: "process_exit_subscribe".to_string(),
        title: "Process exited".to_string(),
        text: "Session 42 exited with code 0".to_string(),
    };
    let text = trigger.render_message_text();
    let item = ResponseItem::Message {
        id: Some("event-1".to_string()),
        role: "user".to_string(),
        content: vec![ContentItem::InputText { text: text.clone() }],
        phase: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected user message turn item");

    match turn_item {
        TurnItem::UserMessage(user) => {
            assert_eq!(
                user.content,
                vec![UserInput::Text {
                    text,
                    text_elements: Vec::new(),
                }]
            );
        }
        other => panic!("expected TurnItem::UserMessage, got {other:?}"),
    }
}

#[test]
fn parses_typed_inter_agent_item_as_typed_turn_item() {
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("agent path"),
        AgentPath::root(),
        Vec::new(),
        "completed".to_string(),
        InterAgentOperation::ChildCompletion,
    )
    .with_status(AgentStatus::Completed(Some("done".to_string())));
    let item = ResponseItem::InterAgentCommunication {
        id: Some("collab-1".to_string()),
        communication: communication.clone(),
    };

    let turn_item = parse_turn_item(&item).expect("expected collab turn item");

    match turn_item {
        TurnItem::CollabAgentMessage(collab) => {
            assert_eq!(collab.id, "collab-1");
            assert_eq!(collab.communication, communication);
        }
        other => panic!("expected TurnItem::CollabAgentMessage, got {other:?}"),
    }
}

#[test]
fn keeps_inter_agent_json_assistant_message_as_agent_message() {
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("agent path"),
        AgentPath::root(),
        Vec::new(),
        "completed".to_string(),
        InterAgentOperation::ChildCompletion,
    );
    let text = serde_json::to_string(&communication).expect("serialize communication");
    let item = ResponseItem::Message {
        id: Some("agent-1".to_string()),
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText { text: text.clone() }],
        phase: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected agent message turn item");

    match turn_item {
        TurnItem::AgentMessage(agent) => {
            assert_eq!(agent.id, "agent-1");
            let rendered = agent
                .content
                .into_iter()
                .map(|content| {
                    let AgentMessageContent::Text { text } = content;
                    text
                })
                .collect::<Vec<_>>();
            assert_eq!(rendered, vec![text]);
        }
        other => panic!("expected TurnItem::AgentMessage, got {other:?}"),
    }
}

#[test]
fn skips_unnamed_image_label_text() {
    let image_url = "data:image/png;base64,abc".to_string();
    let label = codex_protocol::models::image_open_tag_text();
    let user_text = "Please review this image.".to_string();

    let item = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![
            ContentItem::InputText { text: label },
            ContentItem::InputImage {
                image_url: image_url.clone(),
                detail: Some(DEFAULT_IMAGE_DETAIL),
            },
            ContentItem::InputText {
                text: codex_protocol::models::image_close_tag_text(),
            },
            ContentItem::InputText {
                text: user_text.clone(),
            },
        ],
        phase: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected user message turn item");

    match turn_item {
        TurnItem::UserMessage(user) => {
            let expected_content = vec![
                UserInput::Image { image_url },
                UserInput::Text {
                    text: user_text,
                    text_elements: Vec::new(),
                },
            ];
            assert_eq!(user.content, expected_content);
        }
        other => panic!("expected TurnItem::UserMessage, got {other:?}"),
    }
}

#[test]
fn parses_event_command_event_as_distinct_turn_item() {
    let event = EventCommandEvent {
        subscription_id: "sub-command".to_string(),
        kind: EventCommandEventKind::Output,
        label: Some("build log".to_string()),
        command: "tail -f /tmp/build.log".to_string(),
        cwd: Some("/repo".to_string()),
        line: Some("changed:/tmp/build.log".to_string()),
        sequence: Some(1),
        exit_code: None,
        signal: None,
        message: None,
        truncated: false,
        created_at: 1,
    };
    let item = ResponseItem::EventCommandEvent {
        id: Some("event-command-event-1".to_string()),
        event: event.clone(),
    };

    let turn_item = parse_turn_item(&item).expect("expected event command turn item");

    match turn_item {
        TurnItem::EventCommandEvent(event_item) => {
            assert_eq!(event_item.id, "event-command-event-1");
            assert_eq!(event_item.event, event);
        }
        other => panic!("expected TurnItem::EventCommandEvent, got {other:?}"),
    }
}

#[test]
fn keeps_event_command_marker_as_user_message() {
    let event = EventCommandEvent {
        subscription_id: "sub-command".to_string(),
        kind: EventCommandEventKind::Output,
        label: Some("build log".to_string()),
        command: "tail -f /tmp/build.log".to_string(),
        cwd: Some("/repo".to_string()),
        line: Some("changed:/tmp/build.log".to_string()),
        sequence: Some(1),
        exit_code: None,
        signal: None,
        message: None,
        truncated: false,
        created_at: 1,
    };
    let mut item = event.to_response_item();
    let ResponseItem::Message { content, .. } = &item else {
        panic!("expected event command provider formatting to produce a message");
    };
    let text = match content.as_slice() {
        [ContentItem::InputText { text }] => text.clone(),
        other => panic!("expected one input text item, got {other:?}"),
    };
    if let ResponseItem::Message { id, .. } = &mut item {
        *id = Some("event-command-event-1".to_string());
    }

    let turn_item = parse_turn_item(&item).expect("expected user message turn item");

    match turn_item {
        TurnItem::UserMessage(user) => {
            assert_eq!(
                user.content,
                vec![UserInput::Text {
                    text,
                    text_elements: Vec::new(),
                }]
            );
        }
        other => panic!("expected TurnItem::UserMessage, got {other:?}"),
    }
}

#[test]
fn parses_event_command_event_without_id_using_stable_event_id() {
    let event = EventCommandEvent {
        subscription_id: "sub-command".to_string(),
        kind: EventCommandEventKind::Exited,
        label: Some("build log".to_string()),
        command: "cargo test".to_string(),
        cwd: Some("/repo".to_string()),
        line: None,
        sequence: None,
        exit_code: Some(0),
        signal: None,
        message: Some("done".to_string()),
        truncated: false,
        created_at: 1,
    };
    let item = ResponseItem::EventCommandEvent {
        id: None,
        event: event.clone(),
    };

    let turn_item = parse_turn_item(&item).expect("expected event command turn item");

    match turn_item {
        TurnItem::EventCommandEvent(event_item) => {
            assert_eq!(event_item.id, event.stable_item_id());
            assert_eq!(event_item.event, event);
        }
        other => panic!("expected TurnItem::EventCommandEvent, got {other:?}"),
    }
}

#[test]
fn skips_user_instructions_and_env() {
    let items = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "# AGENTS.md instructions for test_directory\n\n<INSTRUCTIONS>\ntest_text\n</INSTRUCTIONS>".to_string(),
                }],
            phase: None,
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<environment_context>test_text</environment_context>".to_string(),
                }],
            phase: None,
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "# AGENTS.md instructions for test_directory\n\n<INSTRUCTIONS>\ntest_text\n</INSTRUCTIONS>".to_string(),
                }],
            phase: None,
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<skill>\n<name>demo</name>\n<path>skills/demo/SKILL.md</path>\nbody\n</skill>"
                        .to_string(),
                }],
            phase: None,
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "<user_shell_command>echo 42</user_shell_command>".to_string(),
                }],
            phase: None,
            },
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![
                    ContentItem::InputText {
                        text: "<environment_context>ctx</environment_context>".to_string(),
                    },
                    ContentItem::InputText {
                        text:
                            "# AGENTS.md instructions for dir\n\n<INSTRUCTIONS>\nbody\n</INSTRUCTIONS>"
                                .to_string(),
                    },
                ],
                phase: None,
            },
        ];

    for item in items {
        let turn_item = parse_turn_item(&item);
        assert!(turn_item.is_none(), "expected none, got {turn_item:?}");
    }
}

#[test]
fn parses_hook_prompt_message_as_distinct_turn_item() {
    let item = build_hook_prompt_message(&[HookPromptFragment::from_single_hook(
        "Retry with exactly the phrase meow meow meow.",
        "hook-run-1",
    )])
    .expect("hook prompt message");

    let turn_item = parse_turn_item(&item).expect("expected hook prompt turn item");

    match turn_item {
        TurnItem::HookPrompt(hook_prompt) => {
            assert_eq!(hook_prompt.fragments.len(), 1);
            assert_eq!(
                hook_prompt.fragments[0],
                HookPromptFragment {
                    text: "Retry with exactly the phrase meow meow meow.".to_string(),
                    hook_run_id: "hook-run-1".to_string(),
                }
            );
        }
        other => panic!("expected TurnItem::HookPrompt, got {other:?}"),
    }
}

#[test]
fn parses_hook_prompt_and_hides_other_contextual_fragments() {
    let item = ResponseItem::Message {
        id: Some("msg-1".to_string()),
        role: "user".to_string(),
        content: vec![
            ContentItem::InputText {
                text: "<environment_context>ctx</environment_context>".to_string(),
            },
            ContentItem::InputText {
                text:
                    "<hook_prompt hook_run_id=\"hook-run-1\">Retry with care &amp; joy.</hook_prompt>"
                        .to_string(),
            },
        ],
        phase: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected hook prompt turn item");

    match turn_item {
        TurnItem::HookPrompt(hook_prompt) => {
            assert_eq!(hook_prompt.id, "msg-1");
            assert_eq!(
                hook_prompt.fragments,
                vec![HookPromptFragment {
                    text: "Retry with care & joy.".to_string(),
                    hook_run_id: "hook-run-1".to_string(),
                }]
            );
        }
        other => panic!("expected TurnItem::HookPrompt, got {other:?}"),
    }
}

#[test]
fn goal_context_does_not_parse_as_visible_turn_item() {
    let item = ResponseItem::Message {
        id: Some("msg-1".to_string()),
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text:
                "<goal_context>\nContinue working toward the active thread goal.\n</goal_context>"
                    .to_string(),
        }],
        phase: None,
    };

    assert!(parse_turn_item(&item).is_none());
}

#[test]
fn parses_agent_message() {
    let item = ResponseItem::Message {
        id: Some("msg-1".to_string()),
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "Hello from Codex".to_string(),
        }],
        phase: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected agent message turn item");

    match turn_item {
        TurnItem::AgentMessage(message) => {
            let Some(AgentMessageContent::Text { text }) = message.content.first() else {
                panic!("expected agent message text content");
            };
            assert_eq!(text, "Hello from Codex");
        }
        other => panic!("expected TurnItem::AgentMessage, got {other:?}"),
    }
}

#[test]
fn parses_reasoning_summary_and_raw_content() {
    let item = ResponseItem::Reasoning {
        id: "reasoning_1".to_string(),
        summary: vec![
            ReasoningItemReasoningSummary::SummaryText {
                text: "Step 1".to_string(),
            },
            ReasoningItemReasoningSummary::SummaryText {
                text: "Step 2".to_string(),
            },
        ],
        content: Some(vec![ReasoningItemContent::ReasoningText {
            text: "raw details".to_string(),
        }]),
        encrypted_content: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected reasoning turn item");

    match turn_item {
        TurnItem::Reasoning(reasoning) => {
            assert_eq!(
                reasoning.summary_text,
                vec!["Step 1".to_string(), "Step 2".to_string()]
            );
            assert_eq!(reasoning.raw_content, vec!["raw details".to_string()]);
        }
        other => panic!("expected TurnItem::Reasoning, got {other:?}"),
    }
}

#[test]
fn parses_reasoning_including_raw_content() {
    let item = ResponseItem::Reasoning {
        id: "reasoning_2".to_string(),
        summary: vec![ReasoningItemReasoningSummary::SummaryText {
            text: "Summarized step".to_string(),
        }],
        content: Some(vec![
            ReasoningItemContent::ReasoningText {
                text: "raw step".to_string(),
            },
            ReasoningItemContent::Text {
                text: "final thought".to_string(),
            },
        ]),
        encrypted_content: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected reasoning turn item");

    match turn_item {
        TurnItem::Reasoning(reasoning) => {
            assert_eq!(reasoning.summary_text, vec!["Summarized step".to_string()]);
            assert_eq!(
                reasoning.raw_content,
                vec!["raw step".to_string(), "final thought".to_string()]
            );
        }
        other => panic!("expected TurnItem::Reasoning, got {other:?}"),
    }
}

#[test]
fn parses_web_search_call() {
    let item = ResponseItem::WebSearchCall {
        id: Some("ws_1".to_string()),
        status: Some("completed".to_string()),
        action: Some(WebSearchAction::Search {
            query: Some("weather".to_string()),
            queries: None,
        }),
    };

    let turn_item = parse_turn_item(&item).expect("expected web search turn item");

    match turn_item {
        TurnItem::WebSearch(search) => assert_eq!(
            search,
            WebSearchItem {
                id: "ws_1".to_string(),
                query: "weather".to_string(),
                action: WebSearchAction::Search {
                    query: Some("weather".to_string()),
                    queries: None,
                },
            }
        ),
        other => panic!("expected TurnItem::WebSearch, got {other:?}"),
    }
}

#[test]
fn parses_web_search_open_page_call() {
    let item = ResponseItem::WebSearchCall {
        id: Some("ws_open".to_string()),
        status: Some("completed".to_string()),
        action: Some(WebSearchAction::OpenPage {
            url: Some("https://example.com".to_string()),
        }),
    };

    let turn_item = parse_turn_item(&item).expect("expected web search turn item");

    match turn_item {
        TurnItem::WebSearch(search) => assert_eq!(
            search,
            WebSearchItem {
                id: "ws_open".to_string(),
                query: "https://example.com".to_string(),
                action: WebSearchAction::OpenPage {
                    url: Some("https://example.com".to_string()),
                },
            }
        ),
        other => panic!("expected TurnItem::WebSearch, got {other:?}"),
    }
}

#[test]
fn parses_web_search_find_in_page_call() {
    let item = ResponseItem::WebSearchCall {
        id: Some("ws_find".to_string()),
        status: Some("completed".to_string()),
        action: Some(WebSearchAction::FindInPage {
            url: Some("https://example.com".to_string()),
            pattern: Some("needle".to_string()),
        }),
    };

    let turn_item = parse_turn_item(&item).expect("expected web search turn item");

    match turn_item {
        TurnItem::WebSearch(search) => assert_eq!(
            search,
            WebSearchItem {
                id: "ws_find".to_string(),
                query: "'needle' in https://example.com".to_string(),
                action: WebSearchAction::FindInPage {
                    url: Some("https://example.com".to_string()),
                    pattern: Some("needle".to_string()),
                },
            }
        ),
        other => panic!("expected TurnItem::WebSearch, got {other:?}"),
    }
}

#[test]
fn parses_partial_web_search_call_without_action_as_other() {
    let item = ResponseItem::WebSearchCall {
        id: Some("ws_partial".to_string()),
        status: Some("in_progress".to_string()),
        action: None,
    };

    let turn_item = parse_turn_item(&item).expect("expected web search turn item");
    match turn_item {
        TurnItem::WebSearch(search) => assert_eq!(
            search,
            WebSearchItem {
                id: "ws_partial".to_string(),
                query: String::new(),
                action: WebSearchAction::Other,
            }
        ),
        other => panic!("expected TurnItem::WebSearch, got {other:?}"),
    }
}
