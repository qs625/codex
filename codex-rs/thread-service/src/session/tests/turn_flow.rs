use codex_analytics_api::CompactionPhase;
use codex_analytics_api::CompactionReason;
use protocol::error::CodexErr;
use protocol::items::TurnItem;
use protocol::openai_models::ModelInfo;
use protocol::protocol::ErrorEvent;
use protocol::protocol::Event;
use protocol::user_input::UserInput;
use serial_test::serial;
use std::collections::VecDeque;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering as AtomicOrdering;

const OLD_CONTEXT_WINDOW_FATAL_PREFIX: &str = "Codex ran out of room in the model's context window";
const OLD_CONTEXT_WINDOW_FATAL_ACTION: &str =
    "Start a new thread or clear earlier history before retrying";

async fn recv_error_event(rx: &async_channel::Receiver<Event>) -> ErrorEvent {
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("expected error event")
            .expect("event channel should remain open");
        if let EventMsg::Error(error) = event.msg {
            return error;
        }
    }
}

fn drain_error_events(rx: &async_channel::Receiver<Event>) -> Vec<ErrorEvent> {
    let mut errors = Vec::new();
    while let Ok(event) = rx.try_recv() {
        if let EventMsg::Error(error) = event.msg {
            errors.push(error);
        }
    }
    errors
}

fn assert_not_old_context_window_fatal(message: &str) {
    assert!(!message.contains(OLD_CONTEXT_WINDOW_FATAL_PREFIX));
    assert!(!message.contains(OLD_CONTEXT_WINDOW_FATAL_ACTION));
}

#[tokio::test]
async fn regular_turn_emits_turn_started_without_waiting_for_startup_prewarm() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let model_client_api = Arc::clone(&sess.services.model_client_api);
    let (_tx, startup_prewarm_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = startup_prewarm_rx.await;
        model_client_api
            .create_turn_client()
            .await
            .map_err(|err| protocol::error::CodexErr::Fatal(err.to_string()))
    });

    sess.set_session_startup_prewarm(
        crate::session_startup_prewarm::SessionStartupPrewarmHandle::new(
            handle,
            std::time::Instant::now(),
            crate::client::WEBSOCKET_CONNECT_TIMEOUT,
        ),
    )
    .await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        crate::tasks::RegularTask::new(),
    )
    .await;

    let first = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
        .await
        .expect("expected turn started event without waiting for startup prewarm")
        .expect("channel open");
    assert!(matches!(
        first.msg,
        EventMsg::TurnStarted(TurnStartedEvent { turn_id, .. }) if turn_id == tc.sub_id
    ));

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;
}

#[tokio::test]
async fn request_mcp_server_elicitation_auto_accepts_when_auto_deny_is_enabled() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;
    session
        .services
        .mcp_connection_manager
        .read()
        .await
        .set_elicitations_auto_deny(/*auto_deny*/ true);

    let requested_schema: McpElicitationSchema = serde_json::from_value(json!({
        "type": "object",
        "properties": {},
    }))
    .expect("schema should deserialize");
    let response = session
        .request_mcp_server_elicitation(
            turn_context.as_ref(),
            RequestId::String("request-1".into()),
            McpServerElicitationRequestParams {
                thread_id: session.conversation_id.to_string(),
                turn_id: Some(turn_context.sub_id.clone()),
                server_name: "codex_apps".to_string(),
                request: McpServerElicitationRequest::Form {
                    meta: None,
                    message: "Allow this request?".to_string(),
                    requested_schema,
                },
            },
        )
        .await;

    assert_eq!(
        response,
        Some(ElicitationResponse {
            action: ElicitationAction::Accept,
            content: Some(json!({})),
            meta: None,
        })
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn interrupting_regular_turn_waiting_on_startup_prewarm_emits_turn_aborted() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let model_client_api = Arc::clone(&sess.services.model_client_api);
    let (_tx, startup_prewarm_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = startup_prewarm_rx.await;
        model_client_api
            .create_turn_client()
            .await
            .map_err(|err| protocol::error::CodexErr::Fatal(err.to_string()))
    });

    sess.set_session_startup_prewarm(
        crate::session_startup_prewarm::SessionStartupPrewarmHandle::new(
            handle,
            std::time::Instant::now(),
            crate::client::WEBSOCKET_CONNECT_TIMEOUT,
        ),
    )
    .await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        crate::tasks::RegularTask::new(),
    )
    .await;

    let first = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
        .await
        .expect("expected turn started event without waiting for startup prewarm")
        .expect("channel open");
    assert!(matches!(
        first.msg,
        EventMsg::TurnStarted(TurnStartedEvent { turn_id, .. }) if turn_id == tc.sub_id
    ));

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    let second = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("expected turn aborted event")
        .expect("channel open");
    let EventMsg::TurnAborted(TurnAbortedEvent {
        turn_id,
        reason,
        completed_at,
        duration_ms,
    }) = second.msg
    else {
        panic!("expected turn aborted event");
    };
    assert_eq!(turn_id, Some(tc.sub_id.clone()));
    assert_eq!(reason, TurnAbortReason::Interrupted);
    assert!(completed_at.is_some());
    assert!(duration_ms.is_some());
}

pub(crate) fn build_test_model_service(
    config: &Config,
    session_configuration: &SessionConfiguration,
    provider_auth_manager: Option<model_service_api::SharedModelProviderAuthManager>,
    model_provider_factory: model_service_api::SharedModelProviderFactory,
) -> SharedModelServiceApi {
    Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
        codex_home: config.codex_home.to_path_buf(),
        config_model_catalog: config.model_catalog.clone(),
        api_runtime_factory: Arc::new(model_service::DefaultApiRuntimeFactory),
        provider_auth_manager,
        model_provider_factory,
        default_provider: Some(session_configuration.provider.clone()),
        providers_by_id: config.model_providers.clone(),
        model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
        attestation_provider: None,
    }))
}

pub(crate) fn build_test_model_service_for_config(
    config: &Config,
    provider_auth_manager: Option<model_service_api::SharedModelProviderAuthManager>,
    model_provider_factory: model_service_api::SharedModelProviderFactory,
) -> SharedModelServiceApi {
    Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
        codex_home: config.codex_home.to_path_buf(),
        config_model_catalog: config.model_catalog.clone(),
        api_runtime_factory: Arc::new(model_service::DefaultApiRuntimeFactory),
        provider_auth_manager,
        model_provider_factory,
        default_provider: Some(config.model_provider.clone()),
        providers_by_id: config.model_providers.clone(),
        model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
        attestation_provider: None,
    }))
}

fn developer_input_texts(items: &[ResponseItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "developer" => {
                Some(content.as_slice())
            }
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            ContentItem::InputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn developer_message_texts(items: &[ResponseItem]) -> Vec<Vec<&str>> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "developer" => {
                Some(content.as_slice())
            }
            _ => None,
        })
        .map(|content| {
            content
                .iter()
                .filter_map(|item| match item {
                    ContentItem::InputText { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

fn user_input_texts(items: &[ResponseItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                Some(content.as_slice())
            }
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            ContentItem::InputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn test_user_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn count_text(texts: &[&str], needle: &str) -> usize {
    texts.iter().filter(|&&text| text == needle).count()
}

#[test]
fn suffix_trim_plan_protects_current_input_when_followed_by_injections() {
    let untrimmed = crate::session::turn::suffix_trim_plan(
        /*total_items*/ 5,
        /*protected_range*/ Some((2, 4)),
        /*trim_suffix_items*/ 0,
    )
    .expect("untrimmed plan should be available");
    assert_eq!(untrimmed.prefix_len, 1);
    assert_eq!(untrimmed.suffix_end, 2);
    assert_eq!(untrimmed.protected_start, 2);
    assert_eq!(untrimmed.protected_end, 4);

    let trimmed = crate::session::turn::suffix_trim_plan(
        /*total_items*/ 5,
        /*protected_range*/ Some((2, 4)),
        /*trim_suffix_items*/ 1,
    )
    .expect("trimmed plan should be available");
    assert_eq!(trimmed.prefix_len, 1);
    assert_eq!(
        trimmed.suffix_end, 1,
        "trimming should remove the suffix tail before the protected current input"
    );
    assert_eq!(trimmed.protected_start, 2);
    assert_eq!(trimmed.protected_end, 4);
}

fn write_project_hooks(dot_codex: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dot_codex)?;
    std::fs::write(
        dot_codex.join("hooks.json"),
        r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo hello from hook"
          }
        ]
      }
    ]
  }
}"#,
    )
}

async fn write_project_trust_config(
    codex_home: &Path,
    trusted_projects: &[(&Path, TrustLevel)],
) -> std::io::Result<()> {
    tokio::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        toml::to_string(&ConfigToml {
            projects: Some(
                trusted_projects
                    .iter()
                    .map(|(project, trust_level)| {
                        (
                            project_trust_key(project),
                            ProjectConfig {
                                trust_level: Some(*trust_level),
                            },
                        )
                    })
                    .collect::<std::collections::HashMap<_, _>>(),
            ),
            ..Default::default()
        })
        .expect("serialize config"),
    )
    .await
}

async fn preview_session_start_hooks(
    config: &crate::config::Config,
) -> std::io::Result<Vec<protocol::protocol::HookRunSummary>> {
    let hooks = Hooks::new(HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &config.config_layer_stack,
            ),
        ),
        ..HooksConfig::default()
    });

    Ok(hooks.preview_session_start(&hooks::SessionStartRequest {
        session_id: ThreadId::new(),
        cwd: config.cwd.clone(),
        transcript_path: None,
        model: "gpt-5.2".to_string(),
        permission_mode: "default".to_string(),
        source: hooks::SessionStartSource::Startup,
    }))
}

pub(crate) fn test_tool_inputs(
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
) -> Arc<crate::session::turn::TurnToolInputs> {
    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let default_agent_type_description =
        codex_agent_roles::spawn_tool_spec::build(&turn_context.config.agent_roles);
    let result = crate::session::turn::TurnToolInputs {
        session_capability: Arc::downgrade(&session_capability),
        mcp_tools: Vec::new(),
        deferred_mcp_tools: Vec::new(),
        discoverable_tools: Vec::new(),
        default_agent_type_description,
        expose_model_visible_tools: true,
    };
    let _ = (session, turn_context);
    Arc::new(result)
}

#[tokio::test]
async fn current_user_input_exceeding_window_is_classified_locally() {
    let (_session, turn_context, _rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |config| {
            config.model_context_window = Some(32);
        },
    )
    .await;

    let input = vec![UserInput::Text {
        text: "current input ".repeat(500),
        text_elements: Vec::new(),
    }];

    assert!(crate::session::turn::user_input_exceeds_context_window(
        &input,
        turn_context.as_ref(),
    ));
}

#[tokio::test]
async fn oversized_current_input_emits_controlled_error_without_compacting() {
    let (session, turn_context, rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |config| {
            config.model_context_window = Some(32);
        },
    )
    .await;
    let input = vec![UserInput::Text {
        text: "current input ".repeat(500),
        text_elements: Vec::new(),
    }];

    crate::session::turn::run_turn(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::clone(&turn_context.extension_data),
        input,
        /*allow_empty_input_without_pending*/ false,
        Some(Box::new(ContextWindowExceededTurnClient {
            fail_before_stream: true,
        })),
        CancellationToken::new(),
    )
    .await;

    let mut saw_compaction = false;
    let mut error = None;
    while let Ok(event) = rx.try_recv() {
        match event.msg {
            EventMsg::ItemStarted(item) => {
                if matches!(item.item, TurnItem::ContextCompaction(_)) {
                    saw_compaction = true;
                }
            }
            EventMsg::ItemCompleted(item) => {
                if matches!(item.item, TurnItem::ContextCompaction(_)) {
                    saw_compaction = true;
                }
            }
            EventMsg::Error(event) => {
                error = Some(event);
            }
            _ => {}
        }
    }
    assert!(
        !saw_compaction,
        "oversized current input should not compact"
    );
    let error = error.expect("oversized input should emit an error");
    assert_eq!(
        error.message,
        crate::session::turn::CURRENT_INPUT_CONTEXT_WINDOW_ERROR_MESSAGE
    );
    assert_not_old_context_window_fatal(&error.message);
}

#[tokio::test]
#[serial(auto_compact_test_hook)]
async fn provider_context_window_compacts_prefix_with_one_item_suffix_then_retries() {
    let (session, turn_context, _rx) = make_session_and_context_with_rx().await;
    let reference_context_item = session
        .reference_context_item_for_turn(turn_context.as_ref())
        .await;
    session
        .replace_history(
            vec![
                test_user_message("prefix before compact"),
                test_user_message("tail before current"),
            ],
            Some(reference_context_item),
        )
        .await;

    let regular_request_count = Arc::new(AtomicUsize::new(0));
    let regular_request_inputs: Arc<StdMutex<Vec<Vec<ResponseItem>>>> =
        Arc::new(StdMutex::new(Vec::new()));
    let staged_prefix_texts: Arc<StdMutex<Vec<Vec<String>>>> = Arc::new(StdMutex::new(Vec::new()));
    let staged_prefix_texts_for_hook = Arc::clone(&staged_prefix_texts);
    let target_turn_id = turn_context.sub_id.clone();
    let _compact_hook_guard = crate::session::turn::set_auto_compact_test_hook(Arc::new(
        move |session, turn_context, reason, phase| {
            if turn_context.sub_id != target_turn_id {
                return None;
            }
            if matches!(reason, CompactionReason::ContextLimit)
                && matches!(phase, CompactionPhase::MidTurn)
            {
                let history = session
                    .state
                    .try_lock()
                    .expect("session state should be available")
                    .clone_history();
                staged_prefix_texts_for_hook
                    .lock()
                    .expect("staged prefix mutex poisoned")
                    .push(
                        user_input_texts(history.raw_items())
                            .into_iter()
                            .map(str::to_string)
                            .collect(),
                    );
                session
                    .state
                    .try_lock()
                    .expect("session state should be available")
                    .replace_history(vec![test_user_message("compacted prefix")], None);
                Some(Ok(true))
            } else {
                Some(Ok(false))
            }
        },
    ));

    crate::session::turn::run_turn(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::clone(&turn_context.extension_data),
        vec![UserInput::Text {
            text: "current request".to_string(),
            text_elements: Vec::new(),
        }],
        /*allow_empty_input_without_pending*/ false,
        Some(Box::new(ScriptedTurnClient {
            responses: VecDeque::from([
                ScriptedTurnResponse::ContextWindowExceeded,
                ScriptedTurnResponse::Completed,
            ]),
            request_count: Arc::clone(&regular_request_count),
            request_inputs: Some(Arc::clone(&regular_request_inputs)),
            response_processed_ids: None,
            provider: Some(turn_context.config.model_provider_id.clone()),
        })),
        CancellationToken::new(),
    )
    .await;

    assert_eq!(regular_request_count.load(AtomicOrdering::SeqCst), 2);
    let staged = staged_prefix_texts
        .lock()
        .expect("staged prefix mutex poisoned");
    assert_eq!(staged.len(), 1);
    assert!(
        !staged[0].contains(&"tail before current".to_string()),
        "compact prefix should temporarily exclude retained suffix items"
    );
    assert!(
        !staged[0].contains(&"current request".to_string()),
        "compact prefix should temporarily exclude the latest current input"
    );
    drop(staged);

    let request_inputs = regular_request_inputs
        .lock()
        .expect("request inputs mutex poisoned");
    let retry_texts = user_input_texts(&request_inputs[1]);
    assert_eq!(count_text(&retry_texts, "tail before current"), 1);
    assert_eq!(count_text(&retry_texts, "current request"), 1);
}

#[tokio::test]
#[serial(auto_compact_test_hook)]
async fn provider_context_window_trims_suffix_when_compact_attempt_overflows() {
    let (session, turn_context, _rx) = make_session_and_context_with_rx().await;
    let reference_context_item = session
        .reference_context_item_for_turn(turn_context.as_ref())
        .await;
    session
        .replace_history(
            vec![
                test_user_message("prefix before compact"),
                test_user_message("tail before current"),
            ],
            Some(reference_context_item),
        )
        .await;

    let regular_request_count = Arc::new(AtomicUsize::new(0));
    let regular_request_inputs: Arc<StdMutex<Vec<Vec<ResponseItem>>>> =
        Arc::new(StdMutex::new(Vec::new()));
    let compact_attempt_count = Arc::new(AtomicUsize::new(0));
    let compact_attempt_count_for_hook = Arc::clone(&compact_attempt_count);
    let staged_prefix_texts: Arc<StdMutex<Vec<Vec<String>>>> = Arc::new(StdMutex::new(Vec::new()));
    let staged_prefix_texts_for_hook = Arc::clone(&staged_prefix_texts);
    let target_turn_id = turn_context.sub_id.clone();
    let _compact_hook_guard = crate::session::turn::set_auto_compact_test_hook(Arc::new(
        move |session, turn_context, reason, phase| {
            if turn_context.sub_id != target_turn_id {
                return None;
            }
            if matches!(reason, CompactionReason::ContextLimit)
                && matches!(phase, CompactionPhase::MidTurn)
            {
                let history = session
                    .state
                    .try_lock()
                    .expect("session state should be available")
                    .clone_history();
                staged_prefix_texts_for_hook
                    .lock()
                    .expect("staged prefix mutex poisoned")
                    .push(
                        user_input_texts(history.raw_items())
                            .into_iter()
                            .map(str::to_string)
                            .collect(),
                    );
                let attempt =
                    compact_attempt_count_for_hook.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                if attempt == 1 {
                    Some(Err(CodexErr::ContextWindowExceeded))
                } else {
                    session
                        .state
                        .try_lock()
                        .expect("session state should be available")
                        .replace_history(vec![test_user_message("compacted prefix")], None);
                    Some(Ok(true))
                }
            } else {
                Some(Ok(false))
            }
        },
    ));

    crate::session::turn::run_turn(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::clone(&turn_context.extension_data),
        vec![UserInput::Text {
            text: "current request".to_string(),
            text_elements: Vec::new(),
        }],
        /*allow_empty_input_without_pending*/ false,
        Some(Box::new(ScriptedTurnClient {
            responses: VecDeque::from([
                ScriptedTurnResponse::ContextWindowExceeded,
                ScriptedTurnResponse::Completed,
            ]),
            request_count: Arc::clone(&regular_request_count),
            request_inputs: Some(Arc::clone(&regular_request_inputs)),
            response_processed_ids: None,
            provider: Some(turn_context.config.model_provider_id.clone()),
        })),
        CancellationToken::new(),
    )
    .await;

    assert_eq!(regular_request_count.load(AtomicOrdering::SeqCst), 2);
    assert_eq!(compact_attempt_count.load(AtomicOrdering::SeqCst), 2);
    let staged = staged_prefix_texts
        .lock()
        .expect("staged prefix mutex poisoned");
    assert_eq!(staged.len(), 2);
    assert!(
        !staged[0].contains(&"tail before current".to_string()),
        "first compact attempt should exclude the full retained suffix"
    );
    assert!(
        !staged[1].contains(&"tail before current".to_string()),
        "second compact attempt should keep the suffix tail trimmed"
    );
    assert!(!staged[1].contains(&"current request".to_string()));
    drop(staged);

    let request_inputs = regular_request_inputs
        .lock()
        .expect("request inputs mutex poisoned");
    let retry_texts = user_input_texts(&request_inputs[1]);
    assert_eq!(
        count_text(&retry_texts, "tail before current"),
        0,
        "retry should omit the trimmed suffix tail"
    );
    assert_eq!(count_text(&retry_texts, "current request"), 1);
}

#[tokio::test]
#[serial(auto_compact_test_hook)]
async fn provider_context_window_trims_suffix_tail_until_compact_succeeds() {
    let (session, turn_context, _rx) = make_session_and_context_with_rx().await;
    let reference_context_item = session
        .reference_context_item_for_turn(turn_context.as_ref())
        .await;
    session
        .replace_history(
            vec![
                test_user_message("prefix before compact"),
                test_user_message("suffix oldest"),
                test_user_message("suffix middle"),
                test_user_message("suffix newest"),
            ],
            Some(reference_context_item),
        )
        .await;

    let regular_request_count = Arc::new(AtomicUsize::new(0));
    let regular_request_inputs: Arc<StdMutex<Vec<Vec<ResponseItem>>>> =
        Arc::new(StdMutex::new(Vec::new()));
    let compact_attempt_count = Arc::new(AtomicUsize::new(0));
    let compact_attempt_count_for_hook = Arc::clone(&compact_attempt_count);
    let target_turn_id = turn_context.sub_id.clone();
    let _compact_hook_guard = crate::session::turn::set_auto_compact_test_hook(Arc::new(
        move |session, turn_context, reason, phase| {
            if turn_context.sub_id != target_turn_id {
                return None;
            }
            if matches!(reason, CompactionReason::ContextLimit)
                && matches!(phase, CompactionPhase::MidTurn)
            {
                let attempt =
                    compact_attempt_count_for_hook.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                if attempt < 3 {
                    Some(Err(CodexErr::ContextWindowExceeded))
                } else {
                    session
                        .state
                        .try_lock()
                        .expect("session state should be available")
                        .replace_history(vec![test_user_message("compacted prefix")], None);
                    Some(Ok(true))
                }
            } else {
                Some(Ok(false))
            }
        },
    ));

    crate::session::turn::run_turn(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::clone(&turn_context.extension_data),
        vec![UserInput::Text {
            text: "current request".to_string(),
            text_elements: Vec::new(),
        }],
        /*allow_empty_input_without_pending*/ false,
        Some(Box::new(ScriptedTurnClient {
            responses: VecDeque::from([
                ScriptedTurnResponse::ContextWindowExceeded,
                ScriptedTurnResponse::Completed,
            ]),
            request_count: Arc::clone(&regular_request_count),
            request_inputs: Some(Arc::clone(&regular_request_inputs)),
            response_processed_ids: None,
            provider: Some(turn_context.config.model_provider_id.clone()),
        })),
        CancellationToken::new(),
    )
    .await;

    assert_eq!(regular_request_count.load(AtomicOrdering::SeqCst), 2);
    assert_eq!(compact_attempt_count.load(AtomicOrdering::SeqCst), 3);
    let request_inputs = regular_request_inputs
        .lock()
        .expect("request inputs mutex poisoned");
    let retry_texts = user_input_texts(&request_inputs[1]);
    assert_eq!(count_text(&retry_texts, "suffix oldest"), 1);
    assert_eq!(count_text(&retry_texts, "suffix middle"), 0);
    assert_eq!(count_text(&retry_texts, "suffix newest"), 0);
    assert_eq!(count_text(&retry_texts, "current request"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn staged_compact_uses_isolated_sampling_without_agent_message_pollution()
-> anyhow::Result<()> {
    let server = start_mock_server().await;
    let _compact_response = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-compact"),
            ev_assistant_message("msg-compact", "real compact summary"),
            ev_completed("resp-compact"),
        ]),
    )
    .await;
    let (session, turn_context, rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |config| {
            config.model_provider.base_url = Some(format!("{}/v1", server.uri()));
            config.model_provider.supports_websockets = false;
            config.model_providers.insert(
                config.model_provider_id.clone(),
                config.model_provider.clone(),
            );
        },
    )
    .await;
    let reference_context_item = session
        .reference_context_item_for_turn(turn_context.as_ref())
        .await;
    session
        .replace_history(
            vec![
                test_user_message("prefix before compact"),
                test_user_message("tail before current"),
            ],
            Some(reference_context_item),
        )
        .await;

    let regular_request_count = Arc::new(AtomicUsize::new(0));
    crate::session::turn::run_turn(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::clone(&turn_context.extension_data),
        vec![UserInput::Text {
            text: "current request".to_string(),
            text_elements: Vec::new(),
        }],
        /*allow_empty_input_without_pending*/ false,
        Some(Box::new(ScriptedTurnClient {
            responses: VecDeque::from([
                ScriptedTurnResponse::ContextWindowExceeded,
                ScriptedTurnResponse::Completed,
            ]),
            request_count: Arc::clone(&regular_request_count),
            request_inputs: None,
            response_processed_ids: None,
            provider: Some(turn_context.config.model_provider_id.clone()),
        })),
        CancellationToken::new(),
    )
    .await;

    assert_eq!(regular_request_count.load(AtomicOrdering::SeqCst), 2);
    let mut saw_agent_message_pollution = false;
    let mut saw_compaction_summary = false;
    while let Ok(event) = rx.try_recv() {
        let turn_item = match event.msg {
            EventMsg::ItemStarted(item) => Some(item.item),
            EventMsg::ItemCompleted(item) => Some(item.item),
            _ => None,
        };
        if let Some(turn_item) = turn_item {
            match turn_item {
                TurnItem::AgentMessage(message) => {
                    saw_agent_message_pollution |= message.content.iter().any(|content| {
                        matches!(
                            content,
                            protocol::items::AgentMessageContent::Text { text }
                                if text == "real compact summary"
                        )
                    });
                }
                TurnItem::ContextCompaction(compaction) => {
                    saw_compaction_summary |= compaction.replacement_history.iter().any(|item| {
                        matches!(
                            item,
                            protocol::items::ContextCompactionReplacementItem::AgentMessage(message)
                                if message.content.iter().any(|content| matches!(
                                    content,
                                    protocol::items::AgentMessageContent::Text { text }
                                        if text == "real compact summary"
                                ))
                        )
                    });
                }
                _ => {}
            }
        }
    }
    assert!(
        !saw_agent_message_pollution,
        "staged compact summary should not be emitted as a regular agent message"
    );
    assert!(
        saw_compaction_summary,
        "staged compact summary should appear in the compaction replacement item"
    );

    let history = session.clone_history().await;
    let history_texts = user_input_texts(history.raw_items());
    assert_eq!(count_text(&history_texts, "current request"), 1);
    assert_eq!(count_text(&history_texts, "tail before current"), 1);

    Ok(())
}

#[tokio::test]
async fn isolated_staged_compact_acknowledges_processed_response_when_enabled() {
    let (session, turn_context, _rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |config| {
            config
                .features
                .enable(Feature::ResponsesWebsocketResponseProcessed)
                .expect("feature should be enableable in tests");
        },
    )
    .await;
    let request_count = Arc::new(AtomicUsize::new(0));
    let response_processed_ids: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
    let mut client = ScriptedTurnClient {
        responses: VecDeque::from([ScriptedTurnResponse::Completed]),
        request_count: Arc::clone(&request_count),
        request_inputs: None,
        response_processed_ids: Some(Arc::clone(&response_processed_ids)),
        provider: Some(turn_context.config.model_provider_id.clone()),
    };

    let summary = crate::compact::run_isolated_compact_sampling(
        session.as_ref(),
        turn_context.as_ref(),
        &mut client,
        None,
        vec![test_user_message("prefix before compact")],
    )
    .await
    .expect("isolated compact should succeed");

    assert_eq!(summary, crate::compact::DEFAULT_COMPACTED_MESSAGE);
    assert_eq!(request_count.load(AtomicOrdering::SeqCst), 1);
    assert_eq!(
        response_processed_ids
            .lock()
            .expect("response processed ids mutex poisoned")
            .as_slice(),
        &["response-id".to_string()]
    );
}

#[tokio::test]
#[serial(auto_compact_test_hook)]
async fn provider_context_window_recovery_trims_retained_suffix_after_second_overflow() {
    let (session, turn_context, _rx) = make_session_and_context_with_rx().await;
    let reference_context_item = session
        .reference_context_item_for_turn(turn_context.as_ref())
        .await;
    session
        .replace_history(
            vec![
                test_user_message("prefix before compact"),
                test_user_message("tail before current"),
            ],
            Some(reference_context_item),
        )
        .await;

    let regular_request_count = Arc::new(AtomicUsize::new(0));
    let regular_request_inputs: Arc<StdMutex<Vec<Vec<ResponseItem>>>> =
        Arc::new(StdMutex::new(Vec::new()));
    let compact_request_count = Arc::new(AtomicUsize::new(0));
    let compact_request_count_for_hook = Arc::clone(&compact_request_count);
    let target_turn_id = turn_context.sub_id.clone();
    let _compact_hook_guard = crate::session::turn::set_auto_compact_test_hook(Arc::new(
        move |session, turn_context, reason, phase| {
            if turn_context.sub_id != target_turn_id {
                return None;
            }
            if matches!(reason, CompactionReason::ContextLimit)
                && matches!(phase, CompactionPhase::MidTurn)
            {
                let attempt =
                    compact_request_count_for_hook.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                let current_request_present = user_input_texts(
                    session
                        .state
                        .try_lock()
                        .expect("session state should be available")
                        .history
                        .raw_items(),
                )
                .contains(&"current request");
                let mut replacement =
                    vec![test_user_message(&format!("compacted prefix {attempt}"))];
                if current_request_present {
                    replacement.push(test_user_message("current request"));
                }
                session
                    .state
                    .try_lock()
                    .expect("session state should be available")
                    .replace_history(replacement, None);
                Some(Ok(true))
            } else {
                Some(Ok(false))
            }
        },
    ));

    crate::session::turn::run_turn(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::clone(&turn_context.extension_data),
        vec![UserInput::Text {
            text: "current request".to_string(),
            text_elements: Vec::new(),
        }],
        /*allow_empty_input_without_pending*/ false,
        Some(Box::new(ScriptedTurnClient {
            responses: VecDeque::from([
                ScriptedTurnResponse::ContextWindowExceeded,
                ScriptedTurnResponse::ContextWindowExceeded,
                ScriptedTurnResponse::Completed,
            ]),
            request_count: Arc::clone(&regular_request_count),
            request_inputs: Some(Arc::clone(&regular_request_inputs)),
            response_processed_ids: None,
            provider: Some(turn_context.config.model_provider_id.clone()),
        })),
        CancellationToken::new(),
    )
    .await;

    assert_eq!(regular_request_count.load(AtomicOrdering::SeqCst), 3);
    assert_eq!(
        compact_request_count.load(AtomicOrdering::SeqCst),
        2,
        "second provider overflow should retry compact with a trimmed retained suffix"
    );

    let request_inputs = regular_request_inputs
        .lock()
        .expect("request inputs mutex poisoned");
    let first_retry_texts = user_input_texts(&request_inputs[1]);
    assert_eq!(
        count_text(&first_retry_texts, "current request"),
        1,
        "first recovery keeps the latest item for continuation"
    );
    assert_eq!(
        count_text(&first_retry_texts, "tail before current"),
        1,
        "first recovery keeps the retained suffix before retrying"
    );
    let final_retry_texts = user_input_texts(&request_inputs[2]);
    assert_eq!(count_text(&final_retry_texts, "current request"), 1);
    assert_eq!(
        count_text(&final_retry_texts, "tail before current"),
        0,
        "second recovery trims the suffix tail that preceded the current input"
    );
    assert_eq!(
        count_text(&final_retry_texts, "compacted prefix 2"),
        1,
        "second recovery should send the freshly compacted prefix"
    );
}

#[tokio::test]
#[serial(auto_compact_test_hook)]
async fn provider_context_window_full_compact_failure_emits_one_controlled_error() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;
    let reference_context_item = session
        .reference_context_item_for_turn(turn_context.as_ref())
        .await;
    session
        .replace_history(
            vec![
                test_user_message("prefix before compact"),
                test_user_message("tail before current"),
            ],
            Some(reference_context_item),
        )
        .await;
    let regular_request_count = Arc::new(AtomicUsize::new(0));
    let compact_request_count = Arc::new(AtomicUsize::new(0));
    let compact_request_count_for_hook = Arc::clone(&compact_request_count);
    let target_turn_id = turn_context.sub_id.clone();
    let _compact_hook_guard = crate::session::turn::set_auto_compact_test_hook(Arc::new(
        move |_session, turn_context, reason, phase| {
            if turn_context.sub_id != target_turn_id {
                return None;
            }
            if matches!(reason, CompactionReason::ContextLimit)
                && matches!(phase, CompactionPhase::MidTurn)
            {
                let attempt =
                    compact_request_count_for_hook.fetch_add(1, AtomicOrdering::SeqCst) + 1;
                if attempt == 1 {
                    Some(Ok(true))
                } else {
                    Some(Err(CodexErr::ContextWindowExceeded))
                }
            } else {
                Some(Ok(false))
            }
        },
    ));

    crate::session::turn::run_turn(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::clone(&turn_context.extension_data),
        vec![UserInput::Text {
            text: "hello".to_string(),
            text_elements: Vec::new(),
        }],
        /*allow_empty_input_without_pending*/ false,
        Some(Box::new(ScriptedTurnClient {
            responses: VecDeque::from([
                ScriptedTurnResponse::ContextWindowExceeded,
                ScriptedTurnResponse::ContextWindowExceeded,
            ]),
            request_count: Arc::clone(&regular_request_count),
            request_inputs: None,
            response_processed_ids: None,
            provider: Some(turn_context.config.model_provider_id.clone()),
        })),
        CancellationToken::new(),
    )
    .await;

    let errors = drain_error_events(&rx);
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].message.as_str(),
        crate::session::turn::AUTO_COMPACT_CONTEXT_WINDOW_RECOVERY_FAILED_MESSAGE
    );
    assert_eq!(regular_request_count.load(AtomicOrdering::SeqCst), 2);
    assert_eq!(compact_request_count.load(AtomicOrdering::SeqCst), 2);
}

#[tokio::test]
#[serial(auto_compact_test_hook)]
async fn provider_context_window_bounded_recovery_failure_emits_controlled_error() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;
    let reference_context_item = session
        .reference_context_item_for_turn(turn_context.as_ref())
        .await;
    session
        .replace_history(
            vec![
                test_user_message("prefix before compact"),
                test_user_message("tail before current"),
            ],
            Some(reference_context_item),
        )
        .await;
    let regular_request_count = Arc::new(AtomicUsize::new(0));
    let compact_request_count = Arc::new(AtomicUsize::new(0));
    let compact_request_count_for_hook = Arc::clone(&compact_request_count);
    let target_turn_id = turn_context.sub_id.clone();
    let _compact_hook_guard = crate::session::turn::set_auto_compact_test_hook(Arc::new(
        move |_session, turn_context, reason, phase| {
            if turn_context.sub_id != target_turn_id {
                return None;
            }
            if matches!(reason, CompactionReason::ContextLimit)
                && matches!(phase, CompactionPhase::MidTurn)
            {
                compact_request_count_for_hook.fetch_add(1, AtomicOrdering::SeqCst);
                Some(Ok(true))
            } else {
                Some(Ok(false))
            }
        },
    ));

    crate::session::turn::run_turn(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::clone(&turn_context.extension_data),
        vec![UserInput::Text {
            text: "hello".to_string(),
            text_elements: Vec::new(),
        }],
        /*allow_empty_input_without_pending*/ false,
        Some(Box::new(ScriptedTurnClient {
            responses: VecDeque::from([
                ScriptedTurnResponse::ContextWindowExceeded,
                ScriptedTurnResponse::ContextWindowExceeded,
                ScriptedTurnResponse::ContextWindowExceeded,
            ]),
            request_count: Arc::clone(&regular_request_count),
            request_inputs: None,
            response_processed_ids: None,
            provider: Some(turn_context.config.model_provider_id.clone()),
        })),
        CancellationToken::new(),
    )
    .await;

    let error = recv_error_event(&rx).await;
    assert_eq!(
        error.message,
        crate::session::turn::AUTO_COMPACT_CONTEXT_WINDOW_RECOVERY_FAILED_MESSAGE
    );
    assert_not_old_context_window_fatal(&error.message);
    assert_eq!(
        regular_request_count.load(AtomicOrdering::SeqCst),
        3,
        "regular turn should stop after exhausting the progressive suffix-trim budget"
    );
    assert_eq!(
        compact_request_count.load(AtomicOrdering::SeqCst),
        2,
        "only available suffix-trim plans should trigger compact recovery attempts"
    );
}

#[tokio::test]
async fn compact_context_window_failure_emits_controlled_error() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;

    crate::compact::send_compact_context_window_error(session.as_ref(), turn_context.as_ref())
        .await;

    let error = recv_error_event(&rx).await;
    assert_eq!(
        error.message,
        crate::compact::COMPACT_CONTEXT_WINDOW_RECOVERY_FAILED_MESSAGE
    );
    assert_not_old_context_window_fatal(&error.message);
}

#[tokio::test]
async fn auto_compact_replays_turn_scoped_injections() {
    let (session, turn_context, _rx) = make_session_and_context_with_rx().await;
    let injection = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "turn scoped skill instructions".to_string(),
        }],
        phase: None,
    };

    session
        .replace_history(Vec::new(), /*reference_context_item*/ None)
        .await;
    crate::session::turn::replay_turn_scoped_injections_after_auto_compact(
        session.as_ref(),
        turn_context.as_ref(),
        std::slice::from_ref(&injection),
    )
    .await;

    assert!(
        user_input_texts(session.clone_history().await.raw_items())
            .contains(&"turn scoped skill instructions")
    );
}

struct ContextWindowExceededTurnClient {
    fail_before_stream: bool,
}

enum ScriptedTurnResponse {
    ContextWindowExceeded,
    Completed,
}

struct ScriptedTurnClient {
    responses: VecDeque<ScriptedTurnResponse>,
    request_count: Arc<AtomicUsize>,
    request_inputs: Option<Arc<StdMutex<Vec<Vec<ResponseItem>>>>>,
    response_processed_ids: Option<Arc<StdMutex<Vec<String>>>>,
    provider: Option<String>,
}

impl model_service_api::ModelTurnClientApi for ScriptedTurnClient {
    fn provider(&self) -> Option<&str> {
        self.provider.as_deref()
    }

    fn reset_websocket_session(&mut self) {}

    fn send_response_processed<'a>(
        &'a self,
        response_id: &'a str,
    ) -> model_service_api::ModelFuture<'a, ()> {
        if let Some(response_processed_ids) = &self.response_processed_ids {
            response_processed_ids
                .lock()
                .expect("response processed ids mutex poisoned")
                .push(response_id.to_string());
        }
        Box::pin(async {})
    }

    fn prewarm_websocket(
        &mut self,
        _request: model_service_api::TurnModelRequest,
    ) -> model_service_api::ModelFuture<'_, Result<(), model_service_api::ModelRequestError>> {
        Box::pin(async { Ok(()) })
    }

    fn stream_responses(
        &mut self,
        request: model_service_api::TurnModelRequest,
    ) -> model_service_api::ModelFuture<
        '_,
        Result<model_service_api::ModelResponseStream, model_service_api::ModelRequestError>,
    > {
        self.request_count.fetch_add(1, AtomicOrdering::SeqCst);
        if let Some(request_inputs) = &self.request_inputs {
            request_inputs
                .lock()
                .expect("request inputs mutex poisoned")
                .push(request.request.input.clone());
        }
        let response = self
            .responses
            .pop_front()
            .expect("scripted turn response should be available");
        Box::pin(async move {
            match response {
                ScriptedTurnResponse::ContextWindowExceeded => {
                    Err(model_service_api::ModelRequestError::context_window_exceeded())
                }
                ScriptedTurnResponse::Completed => Ok(Box::pin(futures::stream::iter(vec![Ok(
                    model_service_api::ModelResponseEvent::Completed {
                        response_id: "response-id".to_string(),
                        token_usage: None,
                        end_turn: Some(true),
                    },
                )]))
                    as model_service_api::ModelResponseStream),
            }
        })
    }

    fn try_switch_fallback_transport(
        &mut self,
        _session_telemetry: session_telemetry_api::SharedSessionTelemetry,
        _model_info: ModelInfo,
    ) -> bool {
        false
    }
}

impl model_service_api::ModelTurnClientApi for ContextWindowExceededTurnClient {
    fn provider(&self) -> Option<&str> {
        None
    }

    fn reset_websocket_session(&mut self) {}

    fn send_response_processed<'a>(
        &'a self,
        _response_id: &'a str,
    ) -> model_service_api::ModelFuture<'a, ()> {
        Box::pin(async {})
    }

    fn prewarm_websocket(
        &mut self,
        _request: model_service_api::TurnModelRequest,
    ) -> model_service_api::ModelFuture<'_, Result<(), model_service_api::ModelRequestError>> {
        Box::pin(async { Ok(()) })
    }

    fn stream_responses(
        &mut self,
        _request: model_service_api::TurnModelRequest,
    ) -> model_service_api::ModelFuture<
        '_,
        Result<model_service_api::ModelResponseStream, model_service_api::ModelRequestError>,
    > {
        let fail_before_stream = self.fail_before_stream;
        Box::pin(async move {
            if fail_before_stream {
                return Err(model_service_api::ModelRequestError::context_window_exceeded());
            }
            Ok(Box::pin(futures::stream::iter(vec![Err(
                model_service_api::ModelRequestError::context_window_exceeded(),
            )])) as model_service_api::ModelResponseStream)
        })
    }

    fn try_switch_fallback_transport(
        &mut self,
        _session_telemetry: session_telemetry_api::SharedSessionTelemetry,
        _model_info: ModelInfo,
    ) -> bool {
        false
    }
}

#[tokio::test]
async fn sampling_request_preserves_context_window_error_from_stream_start() {
    let (session, turn_context, _rx) = make_session_and_context_with_rx().await;
    let turn_diff_tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));
    let mut client = ContextWindowExceededTurnClient {
        fail_before_stream: true,
    };

    let result =
        crate::session::turn::run_sampling_request(crate::session::turn::SamplingRequest {
            tool_inputs_override: Some(test_tool_inputs(
                Arc::clone(&session),
                Arc::clone(&turn_context),
            )),
            sess: Arc::clone(&session),
            turn_context: Arc::clone(&turn_context),
            turn_store: Arc::clone(&turn_context.extension_data),
            turn_diff_tracker,
            client_session: &mut client,
            turn_metadata_header: None,
            input: Vec::new(),
            explicitly_enabled_connectors: &std::collections::HashSet::new(),
            skills_outcome: Some(turn_context.turn_skills.outcome.as_ref()),
            cancellation_token: CancellationToken::new(),
        })
        .await;

    assert!(matches!(result, Err(CodexErr::ContextWindowExceeded)));
}

#[tokio::test]
async fn sampling_request_preserves_context_window_error_from_stream_event() {
    let (session, turn_context, _rx) = make_session_and_context_with_rx().await;
    let turn_diff_tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));
    let mut client = ContextWindowExceededTurnClient {
        fail_before_stream: false,
    };

    let result =
        crate::session::turn::run_sampling_request(crate::session::turn::SamplingRequest {
            tool_inputs_override: Some(test_tool_inputs(
                Arc::clone(&session),
                Arc::clone(&turn_context),
            )),
            sess: Arc::clone(&session),
            turn_context: Arc::clone(&turn_context),
            turn_store: Arc::clone(&turn_context.extension_data),
            turn_diff_tracker,
            client_session: &mut client,
            turn_metadata_header: None,
            input: Vec::new(),
            explicitly_enabled_connectors: &std::collections::HashSet::new(),
            skills_outcome: Some(turn_context.turn_skills.outcome.as_ref()),
            cancellation_token: CancellationToken::new(),
        })
        .await;

    assert!(matches!(result, Err(CodexErr::ContextWindowExceeded)));
}

#[tokio::test]
async fn built_tools_include_custom_agent_roles_in_spawn_agent_schema() {
    let (session, turn_context, _rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |config| {
            config.agent_roles.insert(
                "custom".to_string(),
                codex_agent_roles::AgentRoleConfig {
                    description: Some("Custom agent role.".to_string()),
                    ..Default::default()
                },
            );
        },
    )
    .await;

    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let tool_inputs = crate::session::turn::built_tools(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::downgrade(&session_capability),
        &[],
        &std::collections::HashSet::new(),
        None,
        &CancellationToken::new(),
    )
    .await
    .expect("build tool inputs");
    let tool_specs = session.services.tool_service.model_visible_specs(
        crate::session::turn::tool_service_request(&session, &turn_context, &tool_inputs),
    );
    let spawn_agent_type_description = tool_specs
        .iter()
        .find_map(|tool| match tool {
            tool_service_api::ToolSpec::Function(tool) if tool.name == "spawn_agent" => tool
                .parameters
                .properties
                .as_ref()
                .and_then(|properties| properties.get("agent_type"))
                .and_then(|schema| schema.description.as_deref()),
            _ => None,
        })
        .expect("spawn_agent agent_type description");

    assert!(spawn_agent_type_description.contains("custom: {\nCustom agent role.\n}"));
}

#[tokio::test]
async fn compact_turn_hides_model_visible_tools_without_affecting_regular_turns() {
    let (session, turn_context, _rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |_config| {},
    )
    .await;

    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let regular_tool_inputs = crate::session::turn::built_tools(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        Arc::downgrade(&session_capability),
        &[],
        &std::collections::HashSet::new(),
        None,
        &CancellationToken::new(),
    )
    .await
    .expect("build regular tool inputs");
    assert!(
        regular_tool_inputs.expose_model_visible_tools,
        "regular turns should keep model-visible tools enabled by default"
    );

    let compact_session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let compact_tool_inputs = crate::session::turn::TurnToolInputs {
        session_capability: Arc::downgrade(&compact_session_capability),
        mcp_tools: Vec::new(),
        deferred_mcp_tools: Vec::new(),
        discoverable_tools: Vec::new(),
        default_agent_type_description: String::new(),
        expose_model_visible_tools: false,
    };
    assert!(
        !compact_tool_inputs.expose_model_visible_tools,
        "compact turns should disable model-visible tools"
    );
    let compact_specs = crate::session::turn::model_visible_tool_specs(
        &session,
        &turn_context,
        &compact_tool_inputs,
    );
    assert!(
        compact_specs.is_empty(),
        "compact turns should not expose model-visible tools"
    );
}

pub(crate) async fn dispatch_exec_command_via_tool_service(
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
    call_id: &str,
    arguments: serde_json::Value,
) -> Result<String, FunctionCallError> {
    let result = dispatch_tool_via_tool_service(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        call_id,
        tool_service_api::ToolName::plain("exec_command"),
        ToolCallSource::Direct,
        ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    )
    .await?;
    let response_item = result.result.to_response_item(call_id, &result.payload);
    match response_item {
        ResponseInputItem::FunctionCallOutput { output, .. }
        | ResponseInputItem::CustomToolCallOutput { output, .. } => {
            Ok(output.body.to_text().unwrap_or_default())
        }
        other => Err(FunctionCallError::Fatal(format!(
            "unexpected exec_command response item: {other:?}"
        ))),
    }
}

pub(crate) async fn dispatch_tool_via_tool_service(
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
    call_id: &str,
    tool_name: tool_service_api::ToolName,
    source: ToolCallSource,
    payload: ToolPayload,
) -> Result<tool_service_api::AnyToolResult, FunctionCallError> {
    let tool_inputs = test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context));
    let tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));
    crate::session::turn::dispatch_tool_call(
        Arc::clone(&session.services.tool_service),
        Arc::clone(&session),
        Arc::clone(&turn_context),
        tool_inputs,
        tracker,
        tool_service_api::ToolCall {
            call_id: call_id.to_string(),
            tool_name,
            payload,
        },
        source,
        CancellationToken::new(),
    )
    .await
}

#[tokio::test]
async fn beta_features_header_omits_remote_compaction_v2() -> anyhow::Result<()> {
    let mut config = ConfigBuilder::default().build().await?;
    config.features.enable(Feature::RemoteCompactionV2)?;

    let header = Session::build_model_client_beta_features_header(&config);

    let advertised_features = header.unwrap_or_default();
    assert!(
        !advertised_features
            .split(',')
            .any(|feature| feature == "remote_compaction_v2")
    );
    Ok(())
}

#[tokio::test]
async fn start_managed_network_proxy_applies_execpolicy_network_rules() -> anyhow::Result<()> {
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        /*requirements*/ None,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
    )?;
    let mut exec_policy = Policy::empty();
    exec_policy.add_network_rule(
        "example.com",
        NetworkRuleProtocol::Https,
        Decision::Allow,
        /*justification*/ None,
    )?;

    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &exec_policy,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
        /*network_policy_decider*/ None,
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ false,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;

    let current_cfg = started_proxy.proxy().current_config().await?;
    assert_eq!(
        current_cfg.network.allowed_domains(),
        Some(vec!["example.com".to_string()])
    );
    Ok(())
}

#[tokio::test]
async fn start_managed_network_proxy_ignores_invalid_execpolicy_network_rules() -> anyhow::Result<()>
{
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            domains: Some(NetworkDomainPermissionsToml {
                entries: std::collections::BTreeMap::from([(
                    "managed.example.com".to_string(),
                    NetworkDomainPermissionToml::Allow,
                )]),
            }),
            managed_allowed_domains_only: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
    )?;
    let mut exec_policy = Policy::empty();
    exec_policy.add_network_rule(
        "example.com",
        NetworkRuleProtocol::Https,
        Decision::Allow,
        /*justification*/ None,
    )?;

    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &exec_policy,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
        /*network_policy_decider*/ None,
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ false,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;

    let current_cfg = started_proxy.proxy().current_config().await?;
    assert_eq!(
        current_cfg.network.allowed_domains(),
        Some(vec!["managed.example.com".to_string()])
    );
    Ok(())
}

#[tokio::test]
async fn managed_network_proxy_decider_survives_full_access_start() -> anyhow::Result<()> {
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&SandboxPolicy::DangerFullAccess),
    )?;
    let exec_policy = Policy::empty();
    let decider_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let network_policy_decider: Arc<dyn NetworkPolicyDecider> = Arc::new({
        let decider_calls = Arc::clone(&decider_calls);
        move |_request| {
            decider_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { NetworkDecision::ask("not_allowed") }
        }
    });

    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &exec_policy,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::DangerFullAccess),
        Some(network_policy_decider),
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ true,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;

    let spec = spec.recompute_for_permission_profile(&permission_profile_for_sandbox_policy(
        &SandboxPolicy::new_workspace_write_policy(),
    ))?;
    spec.apply_to_started_proxy(&started_proxy).await?;
    let current_cfg = started_proxy.proxy().current_config().await?;
    assert_eq!(current_cfg.network.allowed_domains(), None);

    use tokio::io::AsyncReadExt as _;
    use tokio::io::AsyncWriteExt as _;

    let mut stream = tokio::net::TcpStream::connect(started_proxy.proxy().http_addr()).await?;
    stream
        .write_all(
            b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
        )
        .await?;
    let mut buffer = [0_u8; 4096];
    let bytes_read = tokio::time::timeout(StdDuration::from_secs(2), stream.read(&mut buffer))
        .await
        .expect("timed out waiting for proxy response")?;
    let response = String::from_utf8_lossy(&buffer[..bytes_read]);

    assert!(
        response.starts_with("HTTP/1.1 403 Forbidden"),
        "unexpected proxy response: {response}"
    );
    assert!(
        response.contains("x-proxy-error: blocked-by-allowlist"),
        "unexpected proxy response: {response}"
    );
    assert_eq!(
        decider_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "unexpected proxy response: {response}"
    );
    Ok(())
}

#[tokio::test]
async fn new_turn_refreshes_managed_network_proxy_for_sandbox_change() -> anyhow::Result<()> {
    let (mut session, _turn_context) = make_session_and_context().await;
    let initial_policy = SandboxPolicy::new_workspace_write_policy();

    let mut network_config = NetworkProxyConfig::default();
    network_config
        .network
        .set_allowed_domains(vec!["evil.com".to_string()]);
    let requirements = NetworkConstraints {
        domains: Some(NetworkDomainPermissionsToml {
            entries: std::collections::BTreeMap::from([(
                "*.example.com".to_string(),
                NetworkDomainPermissionToml::Allow,
            )]),
        }),
        ..Default::default()
    };
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        network_config,
        Some(requirements),
        &permission_profile_for_sandbox_policy(&initial_policy),
    )?;
    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &Policy::empty(),
        &permission_profile_for_sandbox_policy(&initial_policy),
        /*network_policy_decider*/ None,
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ false,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;
    assert_eq!(
        started_proxy
            .proxy()
            .current_config()
            .await?
            .network
            .allowed_domains(),
        Some(vec!["*.example.com".to_string(), "evil.com".to_string()])
    );

    {
        let mut state = session.state.lock().await;
        let mut config = (*state.session_configuration.original_config_do_not_use).clone();
        config.permissions.network = Some(spec);
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(initial_policy.clone(), cwd.as_path())
            .expect("test setup should allow sandbox policy");
        state.session_configuration.original_config_do_not_use = Arc::new(config);
        state
            .session_configuration
            .set_permission_profile_for_tests(PermissionProfile::from_legacy_sandbox_policy(
                &initial_policy,
            ))
            .expect("test setup should allow permission profile");
    }
    session.services.network_proxy = Some(started_proxy);

    session
        .new_turn_with_sub_id(
            "sandbox-policy-change".to_string(),
            SessionSettingsUpdate {
                sandbox_policy: Some(SandboxPolicy::DangerFullAccess),
                ..Default::default()
            },
        )
        .await?;

    let started_proxy = session
        .services
        .network_proxy
        .as_ref()
        .expect("managed network proxy should be present");
    assert_eq!(
        started_proxy
            .proxy()
            .current_config()
            .await?
            .network
            .allowed_domains(),
        Some(vec!["*.example.com".to_string()])
    );

    Ok(())
}

#[tokio::test]
async fn danger_full_access_turns_do_not_expose_managed_network_proxy() -> anyhow::Result<()> {
    let network_spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&SandboxPolicy::DangerFullAccess),
    )?;

    let session = make_session_with_config(move |config| {
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(SandboxPolicy::DangerFullAccess, cwd.as_path())
            .expect("test setup should allow sandbox policy");
        config.permissions.network = Some(network_spec);
    })
    .await?;

    let turn_context = session.new_default_turn().await;
    assert!(turn_context.network.is_none());
    Ok(())
}

#[tokio::test]
async fn workspace_write_turns_continue_to_expose_managed_network_proxy() -> anyhow::Result<()> {
    let sandbox_policy = SandboxPolicy::new_workspace_write_policy();
    let network_spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&sandbox_policy),
    )?;

    let session = make_session_with_config(move |config| {
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(sandbox_policy, cwd.as_path())
            .expect("test setup should allow sandbox policy");
        config.permissions.network = Some(network_spec);
    })
    .await?;

    let turn_context = session.new_default_turn().await;
    assert!(turn_context.network.is_some());
    Ok(())
}

#[tokio::test]
async fn user_shell_commands_do_not_inherit_managed_network_proxy() -> anyhow::Result<()> {
    let sandbox_policy = SandboxPolicy::new_workspace_write_policy();
    let network_spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&sandbox_policy),
    )?;

    let (session, rx) = make_session_with_config_and_rx(move |config| {
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(sandbox_policy, cwd.as_path())
            .expect("test setup should allow sandbox policy");
        config.permissions.network = Some(network_spec);
    })
    .await?;

    let turn_context = session.new_default_turn().await;
    assert!(turn_context.network.is_some());

    #[cfg(windows)]
    let command = r#"$val = $env:HTTP_PROXY; if ([string]::IsNullOrEmpty($val)) { $val = 'not-set' } ; [System.Console]::Write($val)"#.to_string();
    #[cfg(not(windows))]
    let command = r#"sh -c "printf '%s' \"${HTTP_PROXY:-not-set}\"""#.to_string();

    execute_user_shell_command(
        Arc::clone(&session),
        turn_context,
        command,
        CancellationToken::new(),
        UserShellCommandMode::StandaloneTurn,
    )
    .await;

    loop {
        let event = rx.recv().await.expect("channel open");
        if let EventMsg::ExecCommandEnd(event) = event.msg {
            assert_eq!(event.exit_code, 0);
            assert_eq!(event.stdout.trim(), "not-set");
            break;
        }
    }

    Ok(())
}

#[tokio::test]
async fn get_base_instructions_no_user_content() {
    let prompt_with_apply_patch_instructions =
        include_str!("../../../prompt_with_apply_patch_instructions.md");
    let models_response = bundled_models_response()
        .unwrap_or_else(|err| panic!("bundled models.json should parse: {err}"));
    let model_info_for_slug = |slug: &str, config: &Config| {
        let model = models_response
            .models
            .iter()
            .find(|candidate| candidate.slug == slug)
            .cloned()
            .unwrap_or_else(|| panic!("model slug {slug} is missing from models.json"));
        model_info::with_config_overrides(model, &config.to_models_manager_config())
    };
    let test_cases = vec![
        InstructionsTestCase {
            slug: "gpt-5.4",
            expects_apply_patch_description: false,
        },
        InstructionsTestCase {
            slug: "gpt-5.4-mini",
            expects_apply_patch_description: false,
        },
        InstructionsTestCase {
            slug: "gpt-5.3-codex",
            expects_apply_patch_description: false,
        },
        InstructionsTestCase {
            slug: "gpt-5.2",
            expects_apply_patch_description: false,
        },
    ];

    let (session, _turn_context) = make_session_and_context().await;
    let config = test_config().await;

    for test_case in test_cases {
        let model_info = model_info_for_slug(test_case.slug, &config);
        if test_case.expects_apply_patch_description {
            assert_eq!(
                model_info.base_instructions.as_str(),
                prompt_with_apply_patch_instructions
            );
        }

        {
            let mut state = session.state.lock().await;
            state.session_configuration.base_instructions = model_info.base_instructions.clone();
        }

        let base_instructions = session.get_base_instructions().await;
        assert_eq!(base_instructions.text, model_info.base_instructions);
    }
}

#[tokio::test]
async fn reload_user_config_layer_updates_effective_apps_config() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    std::fs::write(
        &config_toml_path,
        "[apps.calendar]\nenabled = false\ndestructive_enabled = false\n",
    )
    .expect("write user config");

    session.reload_user_config_layer().await;

    let config = session.get_config().await;
    let apps_toml = config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|table| table.get("apps"))
        .cloned()
        .expect("apps table");
    let apps = config_service::types::AppsConfigToml::deserialize(apps_toml)
        .expect("deserialize apps config");
    let app = apps
        .apps
        .get("calendar")
        .expect("calendar app config exists");

    assert!(!app.enabled);
    assert_eq!(app.destructive_enabled, Some(false));
}

#[tokio::test]
async fn reload_user_config_layer_updates_base_and_selected_profile_layers() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let base_config_path = codex_home.join(CONFIG_TOML_FILE);
    let profile_config_path = codex_home.join("work.config.toml");
    std::fs::write(
        &base_config_path,
        "model = \"base\"\napproval_policy = \"on-failure\"\n",
    )
    .expect("write base user config");
    std::fs::write(&profile_config_path, "model = \"profile-old\"\n")
        .expect("write profile user config");
    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.to_path_buf())
        .loader_overrides(LoaderOverrides {
            user_config_path: Some(profile_config_path.abs()),
            user_config_profile: Some("work".parse().expect("profile-v2 name")),
            ..LoaderOverrides::without_managed_config_for_tests()
        })
        .build()
        .await
        .expect("load profile config");
    {
        let mut state = session.state.lock().await;
        state.session_configuration.original_config_do_not_use = Arc::new(config);
    }
    std::fs::write(
        &base_config_path,
        "model = \"base\"\napproval_policy = \"never\"\n",
    )
    .expect("update base user config");
    std::fs::write(&profile_config_path, "model = \"profile-new\"\n")
        .expect("update profile user config");

    session.reload_user_config_layer().await;

    let config = session.get_config().await;
    assert_eq!(
        config
            .config_layer_stack
            .get_user_config_file()
            .map(codex_utils_absolute_path::AbsolutePathBuf::as_path),
        Some(profile_config_path.as_path())
    );
    let effective_user_config = config
        .config_layer_stack
        .effective_user_config()
        .expect("merged user config");
    assert_eq!(
        effective_user_config
            .get("model")
            .and_then(toml::Value::as_str),
        Some("profile-new")
    );
    assert_eq!(
        effective_user_config
            .get("approval_policy")
            .and_then(toml::Value::as_str),
        Some("never")
    );
}

#[tokio::test]
async fn reload_user_config_layer_refreshes_hooks() -> anyhow::Result<()> {
    let session = make_session_with_config(|config| {
        config
            .features
            .enable(Feature::CodexHooks)
            .expect("enable Codex hooks");
    })
    .await?;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home)?;
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    let user_config: config_service::TomlValue = serde_json::from_value(serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": "python3 /tmp/user.py",
                }],
            }],
        },
    }))?;

    let request = hooks::SessionStartRequest {
        session_id: session.conversation_id,
        cwd: session.get_config().await.cwd.clone(),
        transcript_path: None,
        model: "gpt-5.2".to_string(),
        permission_mode: "default".to_string(),
        source: hooks::SessionStartSource::Startup,
    };
    assert!(session.hooks().preview_session_start(&request).is_empty());

    let config = session.get_config().await;
    let hook_list = hooks::list_hooks(hooks::HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &config
                    .config_layer_stack
                    .with_user_config(&config_toml_path, user_config.clone()),
            ),
        ),
        ..hooks::HooksConfig::default()
    });
    assert_eq!(hook_list.hooks.len(), 1);
    assert_eq!(
        hook_list.hooks[0].trust_status,
        protocol::protocol::HookTrustStatus::Untrusted
    );

    let trusted_user_config: config_service::TomlValue =
        serde_json::from_value(serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "type": "command",
                        "command": "python3 /tmp/user.py",
                    }],
                }],
                "state": {
                    hook_list.hooks[0].key.clone(): {
                        "trusted_hash": hook_list.hooks[0].current_hash.clone(),
                    },
                },
            },
        }))?;
    std::fs::write(&config_toml_path, toml::to_string(&trusted_user_config)?)?;

    session.reload_user_config_layer().await;

    assert_eq!(session.hooks().preview_session_start(&request).len(), 1);
    Ok(())
}

#[tokio::test]
async fn refresh_runtime_config_refreshes_hooks() -> anyhow::Result<()> {
    let (session, _turn_context) = make_session_and_context().await;
    {
        let mut state = session.state.lock().await;
        let mut config = (*state.session_configuration.original_config_do_not_use).clone();
        config
            .features
            .enable(Feature::CodexHooks)
            .expect("enable Codex hooks");
        state.session_configuration.original_config_do_not_use = Arc::new(config);
    }
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home)?;
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    #[derive(serde::Serialize)]
    struct NormalizedHookIdentity {
        event_name: &'static str,
        #[serde(flatten)]
        group: config_service::MatcherGroup,
    }
    let trusted_hash = {
        let identity = NormalizedHookIdentity {
            event_name: "session_start",
            group: config_service::MatcherGroup {
                matcher: None,
                hooks: vec![config_service::HookHandlerConfig::Command {
                    command: "python3 /tmp/user.py".to_string(),
                    command_windows: None,
                    timeout_sec: Some(600),
                    r#async: false,
                    status_message: None,
                }],
            },
        };
        let identity = config_service::TomlValue::try_from(identity)?;
        config_service::version_for_toml(&identity)
    };
    let hook_key = format!("{}:session_start:0:0", config_toml_path.display());
    let trusted_user_config: config_service::TomlValue =
        serde_json::from_value(serde_json::json!({
            "hooks": {
                "SessionStart": [{
                    "hooks": [{
                        "type": "command",
                        "command": "python3 /tmp/user.py",
                    }],
                }],
                "state": {
                    hook_key: {
                        "trusted_hash": trusted_hash,
                    },
                },
            },
        }))?;
    std::fs::write(&config_toml_path, toml::to_string(&trusted_user_config)?)?;

    let request = hooks::SessionStartRequest {
        session_id: session.conversation_id,
        cwd: session.get_config().await.cwd.clone(),
        transcript_path: None,
        model: "gpt-5.2".to_string(),
        permission_mode: "default".to_string(),
        source: hooks::SessionStartSource::Startup,
    };
    assert!(session.hooks().preview_session_start(&request).is_empty());

    let next_config = load_latest_config_for_session(&session).await;
    session.refresh_runtime_config(next_config).await;

    assert_eq!(session.hooks().preview_session_start(&request).len(), 1);
    Ok(())
}

#[tokio::test]
async fn reload_user_config_layer_updates_effective_tool_suggest_config() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    std::fs::write(
        &config_toml_path,
        r#"[tool_suggest]
disabled_tools = [
  { type = "connector", id = " calendar " },
  { type = "plugin", id = "slack@openai-curated" },
]
"#,
    )
    .expect("write user config");

    session.reload_user_config_layer().await;

    let config = session.get_config().await;
    assert_eq!(
        config.tool_suggest.disabled_tools,
        vec![
            ToolSuggestDisabledTool::connector("calendar"),
            ToolSuggestDisabledTool::plugin("slack@openai-curated"),
        ]
    );
}

#[tokio::test]
async fn refresh_runtime_config_updates_runtime_refreshable_fields_and_keeps_session_static_settings()
 {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    std::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        r#"[apps.calendar]
enabled = false
destructive_enabled = false

[tool_suggest]
disabled_tools = [
  { type = "connector", id = " calendar " },
  { type = "plugin", id = "slack@openai-curated" },
]
"#,
    )
    .expect("write user config");

    let original = session.get_config().await;
    let mut next_config = load_latest_config_for_session(&session).await;
    next_config.model = Some("gpt-5.4".to_string());
    let provider = ModelProviderInfo {
        name: "Corp".to_string(),
        base_url: Some("https://corp.example.test/v1".to_string()),
        ..ModelProviderInfo::default()
    };
    next_config.model_provider_id = "corp".to_string();
    next_config.model_provider = provider.clone();
    next_config
        .model_providers
        .insert("corp".to_string(), provider);
    next_config
        .model_options
        .push(config_service::config_toml::ModelOptionToml {
            model: "corp-model".to_string(),
            provider: "corp".to_string(),
            ..Default::default()
        });
    next_config.notify = Some(vec!["echo".to_string()]);

    session.refresh_runtime_config(next_config).await;

    let config = session.get_config().await;
    let apps_toml = config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|table| table.get("apps"))
        .cloned()
        .expect("apps table");
    let apps = config_service::types::AppsConfigToml::deserialize(apps_toml)
        .expect("deserialize apps config");
    let app = apps
        .apps
        .get("calendar")
        .expect("calendar app config exists");

    assert!(!app.enabled);
    assert_eq!(app.destructive_enabled, Some(false));
    assert_eq!(config.model, original.model);
    assert_eq!(config.model_provider_id, original.model_provider_id);
    assert_eq!(config.model_provider, original.model_provider);
    assert_eq!(config.model_providers, original.model_providers);
    assert_eq!(config.model_options, original.model_options);
    assert_eq!(config.notify, original.notify);
    assert_eq!(
        config.tool_suggest.disabled_tools,
        vec![
            ToolSuggestDisabledTool::connector("calendar"),
            ToolSuggestDisabledTool::plugin("slack@openai-curated"),
        ]
    );
}

#[tokio::test]
async fn refresh_runtime_config_updates_default_approval_and_sandbox() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    let codex_self_exe = codex_home.join("codex-self").to_path_buf();
    let codex_linux_sandbox_exe = codex_home.join("codex-linux-sandbox").to_path_buf();
    let main_execve_wrapper_exe = codex_home.join("main-execve-wrapper").to_path_buf();
    let zsh_path = codex_home.join("zsh").to_path_buf();
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    {
        let mut state = session.state.lock().await;
        state.session_configuration.approval_policy =
            config_service::Constrained::allow_any(protocol::protocol::AskForApproval::OnRequest);
        state
            .session_configuration
            .approval_policy_is_session_override = false;
        state
            .session_configuration
            .permission_profile_is_session_override = false;
        let read_only =
            PermissionProfile::from_legacy_sandbox_policy(&SandboxPolicy::new_read_only_policy());
        state
            .session_configuration
            .set_permission_profile_for_tests(read_only)
            .expect("test setup should allow read-only permission profile");
        let mut config = (*state.session_configuration.original_config_do_not_use).clone();
        config.codex_self_exe = Some(codex_self_exe.clone());
        config.codex_linux_sandbox_exe = Some(codex_linux_sandbox_exe.clone());
        config.main_execve_wrapper_exe = Some(main_execve_wrapper_exe.clone());
        config.zsh_path = Some(zsh_path.clone());
        config.permissions.approval_policy =
            config_service::Constrained::allow_any(protocol::protocol::AskForApproval::OnRequest);
        config.permissions.set_permission_profile_state(
            config_service::PermissionProfileState::from_constrained_legacy(
                config_service::Constrained::allow_any(
                    PermissionProfile::from_legacy_sandbox_policy(
                        &SandboxPolicy::new_read_only_policy(),
                    ),
                ),
            )
            .expect("test setup should allow read-only permission state"),
        );
        state.session_configuration.original_config_do_not_use = Arc::new(config);
    }
    std::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        r#"approval_policy = "never"
sandbox_mode = "workspace-write"
"#,
    )
    .expect("write user config");

    let next_config = load_latest_config_for_session(&session).await;
    session.refresh_runtime_config(next_config).await;

    let state = session.state.lock().await;
    assert_eq!(
        state.session_configuration.approval_policy.value(),
        protocol::protocol::AskForApproval::Never,
    );
    assert!(matches!(
        state.session_configuration.sandbox_policy(),
        SandboxPolicy::WorkspaceWrite {
            network_access: false,
            ..
        }
    ));
    let per_turn_config = Session::build_per_turn_config(
        &state.session_configuration,
        state.session_configuration.cwd.clone(),
    );
    assert_eq!(
        per_turn_config.permissions.approval_policy.value(),
        protocol::protocol::AskForApproval::Never,
    );
    assert!(matches!(
        per_turn_config.legacy_sandbox_policy(),
        SandboxPolicy::WorkspaceWrite {
            network_access: false,
            ..
        }
    ));
    assert_eq!(per_turn_config.codex_self_exe, Some(codex_self_exe));
    assert_eq!(
        per_turn_config.codex_linux_sandbox_exe,
        Some(codex_linux_sandbox_exe)
    );
    assert_eq!(
        per_turn_config.main_execve_wrapper_exe,
        Some(main_execve_wrapper_exe)
    );
    assert_eq!(per_turn_config.zsh_path, Some(zsh_path));
}

#[tokio::test]
async fn refresh_runtime_config_keeps_project_approval_and_sandbox_precedence() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    let project_dir = tempfile::tempdir().expect("create project dir");
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    {
        let mut state = session.state.lock().await;
        let project_read_only =
            PermissionProfile::from_legacy_sandbox_policy(&SandboxPolicy::new_read_only_policy());
        state.session_configuration.cwd = project_dir.path().abs();
        state.session_configuration.approval_policy =
            config_service::Constrained::allow_any(protocol::protocol::AskForApproval::OnRequest);
        state
            .session_configuration
            .approval_policy_is_session_override = false;
        state
            .session_configuration
            .permission_profile_is_session_override = false;
        state
            .session_configuration
            .set_permission_profile_for_tests(project_read_only.clone())
            .expect("test setup should allow read-only permission profile");

        let mut config = (*state.session_configuration.original_config_do_not_use).clone();
        config.cwd = project_dir.path().abs();
        config.permissions.approval_policy =
            config_service::Constrained::allow_any(protocol::protocol::AskForApproval::OnRequest);
        config.permissions.set_permission_profile_state(
            config_service::PermissionProfileState::from_constrained_legacy(
                config_service::Constrained::allow_any(project_read_only),
            )
            .expect("test setup should allow read-only permission state"),
        );
        let project_config: toml::Value = toml::from_str(
            r#"approval_policy = "on-request"
sandbox_mode = "read-only"
"#,
        )
        .expect("project config should parse");
        config.config_layer_stack = config_service::ConfigLayerStack::new(
            vec![config_service::ConfigLayerEntry::new(
                codex_config_types::ConfigLayerSource::Project {
                    dot_codex_folder: project_dir.path().join(".codex").abs(),
                },
                project_config,
            )],
            Default::default(),
            Default::default(),
        )
        .expect("config layer stack");
        state.session_configuration.original_config_do_not_use = Arc::new(config);
    }
    std::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        r#"approval_policy = "never"
sandbox_mode = "workspace-write"
"#,
    )
    .expect("write user config");

    let next_config = load_latest_config_for_session(&session).await;
    session.refresh_runtime_config(next_config).await;

    let state = session.state.lock().await;
    assert_eq!(
        state.session_configuration.approval_policy.value(),
        protocol::protocol::AskForApproval::OnRequest,
    );
    assert_eq!(
        state.session_configuration.sandbox_policy(),
        SandboxPolicy::new_read_only_policy(),
    );
    let per_turn_config = Session::build_per_turn_config(
        &state.session_configuration,
        state.session_configuration.cwd.clone(),
    );
    assert_eq!(
        per_turn_config.permissions.approval_policy.value(),
        protocol::protocol::AskForApproval::OnRequest,
    );
    assert_eq!(
        per_turn_config.legacy_sandbox_policy(),
        SandboxPolicy::new_read_only_policy(),
    );
}

#[tokio::test]
async fn refresh_runtime_config_preserves_explicit_approval_and_sandbox_overrides() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    {
        let mut state = session.state.lock().await;
        state.session_configuration.approval_policy =
            config_service::Constrained::allow_any(protocol::protocol::AskForApproval::OnRequest);
        state
            .session_configuration
            .approval_policy_is_session_override = true;
        state
            .session_configuration
            .permission_profile_is_session_override = true;
        state
            .session_configuration
            .set_permission_profile_for_tests(PermissionProfile::from_legacy_sandbox_policy(
                &SandboxPolicy::new_read_only_policy(),
            ))
            .expect("test setup should allow read-only permission profile");
        let mut config = (*state.session_configuration.original_config_do_not_use).clone();
        config.permissions.approval_policy =
            config_service::Constrained::allow_any(protocol::protocol::AskForApproval::OnRequest);
        config.permissions.set_permission_profile_state(
            config_service::PermissionProfileState::from_constrained_legacy(
                config_service::Constrained::allow_any(
                    PermissionProfile::from_legacy_sandbox_policy(
                        &SandboxPolicy::new_read_only_policy(),
                    ),
                ),
            )
            .expect("test setup should allow read-only permission state"),
        );
        state.session_configuration.original_config_do_not_use = Arc::new(config);
    }
    std::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        r#"approval_policy = "never"
sandbox_mode = "workspace-write"
"#,
    )
    .expect("write user config");

    let next_config = load_latest_config_for_session(&session).await;
    session.refresh_runtime_config(next_config).await;

    let state = session.state.lock().await;
    assert_eq!(
        state.session_configuration.approval_policy.value(),
        protocol::protocol::AskForApproval::OnRequest,
    );
    assert_eq!(
        state.session_configuration.sandbox_policy(),
        SandboxPolicy::new_read_only_policy(),
    );
    let per_turn_config = Session::build_per_turn_config(
        &state.session_configuration,
        state.session_configuration.cwd.clone(),
    );
    assert_eq!(
        per_turn_config.permissions.approval_policy.value(),
        protocol::protocol::AskForApproval::OnRequest,
    );
    assert_eq!(
        per_turn_config.legacy_sandbox_policy(),
        SandboxPolicy::new_read_only_policy(),
    );
}

#[tokio::test]
async fn reconstruct_history_matches_live_compactions() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, expected) = sample_rollout(&session, &turn_context).await;

    let reconstruction_turn = session.new_default_turn().await;
    let reconstructed = session
        .reconstruct_history_from_rollout(reconstruction_turn.as_ref(), &rollout_items)
        .await;

    assert_eq!(expected, reconstructed.history);
}

#[tokio::test]
async fn reconstruct_history_uses_replacement_history_verbatim() {
    let (session, turn_context) = make_session_and_context().await;
    let summary_item = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    };
    let replacement_history = vec![
        summary_item.clone(),
        ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: "stale developer instructions".to_string(),
            }],
            phase: None,
        },
    ];
    let rollout_items = vec![RolloutItem::Compacted(CompactedItem {
        message: String::new(),
        replacement_history: Some(replacement_history.clone()),
        visible_replacement_history_len: None,
    })];

    let reconstructed = session
        .reconstruct_history_from_rollout(&turn_context, &rollout_items)
        .await;

    assert_eq!(reconstructed.history, replacement_history);
}

#[tokio::test]
async fn record_initial_history_reconstructs_resumed_transcript() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, expected) = sample_rollout(&session, &turn_context).await;

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let history = session.state.lock().await.clone_history();
    assert_eq!(expected, history.raw_items());
}

#[tokio::test]
async fn record_initial_history_new_materializes_initial_context_immediately() {
    let (mut session, _turn_context) = make_session_and_context().await;
    let rollout_path = attach_thread_persistence(&mut session).await;

    session.record_initial_history(InitialHistory::New).await;

    let history = session.clone_history().await;
    assert!(
        !history.raw_items().is_empty(),
        "new threads should record initial context into history immediately"
    );
    let current_context = session.reference_context_item().await;
    assert!(
        current_context.is_some(),
        "new threads should seed a context baseline"
    );
    assert_eq!(session.previous_turn_settings().await, None);

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    assert!(
        resumed.history.iter().any(|item| matches!(
            item,
            RolloutItem::ResponseItem(ResponseItem::Message { .. })
        )),
        "materialized rollout should include the initial context messages"
    );
    let persisted_turn_context = resumed.history.iter().find_map(|item| match item {
        RolloutItem::TurnContext(ctx) => Some(ctx.clone()),
        _ => None,
    });
    assert_eq!(
        serde_json::to_value(persisted_turn_context).expect("serialize persisted turn context"),
        serde_json::to_value(current_context).expect("serialize current turn context")
    );
}

#[tokio::test]
async fn resumed_history_injects_initial_context_on_first_context_update_only() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, mut expected) = sample_rollout(&session, &turn_context).await;

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let history_before_seed = session.state.lock().await.clone_history();
    assert_eq!(expected, history_before_seed.raw_items());

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    expected.extend(
        session
            .build_initial_context_for_external_agent_tools(&turn_context)
            .await,
    );
    let history_after_seed = session.clone_history().await;
    assert_eq!(expected, history_after_seed.raw_items());

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    let history_after_second_seed = session.clone_history().await;
    assert_eq!(
        history_after_seed.raw_items(),
        history_after_second_seed.raw_items()
    );
}

#[tokio::test]
async fn record_initial_history_seeds_token_info_from_rollout() {
    let (session, turn_context) = make_session_and_context().await;
    let (mut rollout_items, _expected) = sample_rollout(&session, &turn_context).await;

    let info1 = TokenUsageInfo {
        total_token_usage: TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
            total_tokens: 30,
        },
        last_token_usage: TokenUsage {
            input_tokens: 3,
            cached_input_tokens: 0,
            output_tokens: 4,
            reasoning_output_tokens: 0,
            total_tokens: 7,
        },
        model_context_window: Some(1_000),
    };
    let info2 = TokenUsageInfo {
        total_token_usage: TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 50,
            output_tokens: 200,
            reasoning_output_tokens: 25,
            total_tokens: 375,
        },
        last_token_usage: TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 5,
            total_tokens: 35,
        },
        model_context_window: Some(2_000),
    };

    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: Some(info1),
            rate_limits: None,
        },
    )));
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: None,
            rate_limits: None,
        },
    )));
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: Some(info2.clone()),
            rate_limits: None,
        },
    )));
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: None,
            rate_limits: None,
        },
    )));

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let actual = session.state.lock().await.token_info();
    assert_eq!(actual, Some(info2));
}

#[tokio::test]
async fn thread_context_usage_recomputes_after_resume_without_persisted_snapshot() {
    let (session, turn_context) = make_session_and_context().await;
    let (mut rollout_items, _expected) = sample_rollout(&session, &turn_context).await;
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: Some(TokenUsageInfo {
                total_token_usage: TokenUsage {
                    input_tokens: 100,
                    cached_input_tokens: 50,
                    output_tokens: 200,
                    reasoning_output_tokens: 25,
                    total_tokens: 375,
                },
                last_token_usage: TokenUsage {
                    input_tokens: 10,
                    cached_input_tokens: 0,
                    output_tokens: 20,
                    reasoning_output_tokens: 5,
                    total_tokens: 35,
                },
                model_context_window: Some(1_000),
            }),
            rate_limits: None,
        },
    )));

    assert!(
        rollout_items.iter().all(|item| !matches!(
            item,
            RolloutItem::EventMsg(EventMsg::ThreadContextUsageUpdated(_))
        )),
        "test history should reproduce rollouts without persisted context usage"
    );

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let usage = session.thread_context_usage().await;

    assert!(usage.total_bytes > 0);
    assert!(usage.categories.user_messages > 0);
    assert!(usage.categories.llm_messages > 0);
    assert_eq!(usage.budget_used_percent, Some(37));
}

#[tokio::test]
async fn thread_context_usage_counts_compaction_summary_as_compact() {
    let (session, turn_context) = make_session_and_context().await;
    let summary = format!(
        "{}\nThe earlier conversation was compacted into this summary.",
        crate::compact::SUMMARY_PREFIX
    );
    let item = user_message(&summary);
    session
        .record_into_history(std::slice::from_ref(&item), &turn_context)
        .await;

    let usage = session.thread_context_usage().await;

    assert!(usage.categories.compact > 0);
    assert_eq!(usage.categories.user_messages, 0);
}

#[tokio::test]
async fn thread_context_usage_counts_compaction_replacement_seed_as_compact() {
    let (session, _turn_context) = make_session_and_context().await;
    let user_item = user_message("recent user message");
    let compact_seed = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "compact final output seed".to_string(),
        }],
        phase: None,
    };
    let replacement_history = vec![user_item, compact_seed.clone()];

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: vec![RolloutItem::Compacted(CompactedItem {
                message: "compact final output seed".to_string(),
                replacement_history: Some(replacement_history),
                visible_replacement_history_len: None,
            })],
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let compact_seed_bytes =
        codex_context_manager::estimate_response_item_model_visible_bytes(&compact_seed);
    let usage = session.thread_context_usage().await;

    assert_eq!(usage.categories.compact, compact_seed_bytes);
    assert_eq!(usage.categories.llm_messages, 0);
    assert!(usage.categories.user_messages > 0);
}

#[tokio::test]
async fn recompute_token_usage_uses_session_base_instructions() {
    let (session, turn_context) = make_session_and_context().await;

    let override_instructions = "SESSION_OVERRIDE_INSTRUCTIONS_ONLY".repeat(120);
    {
        let mut state = session.state.lock().await;
        state.session_configuration.base_instructions = override_instructions.clone();
    }

    let item = user_message("hello");
    session
        .record_into_history(std::slice::from_ref(&item), &turn_context)
        .await;

    let history = session.clone_history().await;
    let session_base_instructions = BaseInstructions {
        text: override_instructions,
    };
    let expected_tokens = history
        .estimate_token_count_with_base_instructions(&session_base_instructions)
        .expect("estimate with session base instructions");
    let model_estimated_tokens = history
        .estimate_token_count_with_base_instructions(&BaseInstructions {
            text: turn_context.model_info.get_model_instructions(
                turn_context.personality.or(turn_context.config.personality),
            ),
        })
        .expect("estimate with model instructions");
    assert_ne!(expected_tokens, model_estimated_tokens);

    session.recompute_token_usage(&turn_context).await;

    let actual_tokens = session
        .state
        .lock()
        .await
        .token_info()
        .expect("token info")
        .last_token_usage
        .total_tokens;
    assert_eq!(actual_tokens, expected_tokens.max(0));
}

#[tokio::test]
async fn recompute_token_usage_updates_model_context_window() {
    let (session, mut turn_context) = make_session_and_context().await;

    {
        let mut state = session.state.lock().await;
        state.set_token_info(Some(TokenUsageInfo {
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            model_context_window: Some(258_400),
        }));
    }

    turn_context.model_info.context_window = Some(128_000);
    turn_context.model_info.effective_context_window_percent = 100;

    session.recompute_token_usage(&turn_context).await;

    let actual = session.state.lock().await.token_info().expect("token info");
    assert_eq!(actual.model_context_window, Some(128_000));
}

#[tokio::test]
async fn record_token_usage_info_notifies_extension_contributors() {
    struct SessionTokenUsageMarker;
    struct ThreadTokenUsageMarker;

    #[derive(Debug, PartialEq, Eq)]
    struct RecordedTokenUsage {
        session_level_id: String,
        thread_level_id: String,
        turn_level_id: String,
        token_usage: TokenUsageInfo,
        saw_session_store: bool,
        saw_thread_store: bool,
    }

    struct TokenUsageRecorder {
        records: Arc<std::sync::Mutex<Vec<RecordedTokenUsage>>>,
    }

    impl codex_extension_api::TokenUsageContributor for TokenUsageRecorder {
        fn on_token_usage(
            &self,
            session_store: &codex_extension_api::ExtensionData,
            thread_store: &codex_extension_api::ExtensionData,
            turn_store: &codex_extension_api::ExtensionData,
            token_usage: &TokenUsageInfo,
        ) {
            self.records
                .lock()
                .expect("token usage records lock")
                .push(RecordedTokenUsage {
                    session_level_id: session_store.level_id().to_string(),
                    thread_level_id: thread_store.level_id().to_string(),
                    turn_level_id: turn_store.level_id().to_string(),
                    token_usage: token_usage.clone(),
                    saw_session_store: session_store.get::<SessionTokenUsageMarker>().is_some(),
                    saw_thread_store: thread_store.get::<ThreadTokenUsageMarker>().is_some(),
                });
        }
    }

    let (mut session, turn_context) = make_session_and_context().await;
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.token_usage_contributor(Arc::new(TokenUsageRecorder {
        records: Arc::clone(&records),
    }));
    session.services.extensions = Arc::new(builder.build());
    session
        .services
        .session_extension_data
        .insert(SessionTokenUsageMarker);
    session
        .services
        .thread_extension_data
        .insert(ThreadTokenUsageMarker);

    let first_usage = TokenUsage {
        input_tokens: 10,
        cached_input_tokens: 2,
        output_tokens: 20,
        reasoning_output_tokens: 3,
        total_tokens: 33,
    };
    let second_usage = TokenUsage {
        input_tokens: 7,
        cached_input_tokens: 1,
        output_tokens: 8,
        reasoning_output_tokens: 5,
        total_tokens: 20,
    };

    session
        .record_token_usage_info(&turn_context, Some(&first_usage))
        .await;
    session
        .record_token_usage_info(&turn_context, Some(&second_usage))
        .await;

    let mut expected_total_usage = first_usage.clone();
    expected_total_usage.add_assign(&second_usage);
    let expected = vec![
        RecordedTokenUsage {
            session_level_id: session.session_id().to_string(),
            thread_level_id: session.conversation_id.to_string(),
            turn_level_id: turn_context.sub_id.clone(),
            token_usage: TokenUsageInfo {
                total_token_usage: first_usage.clone(),
                last_token_usage: first_usage,
                model_context_window: turn_context.model_context_window(),
            },
            saw_session_store: true,
            saw_thread_store: true,
        },
        RecordedTokenUsage {
            session_level_id: session.session_id().to_string(),
            thread_level_id: session.conversation_id.to_string(),
            turn_level_id: turn_context.sub_id.clone(),
            token_usage: TokenUsageInfo {
                total_token_usage: expected_total_usage,
                last_token_usage: second_usage,
                model_context_window: turn_context.model_context_window(),
            },
            saw_session_store: true,
            saw_thread_store: true,
        },
    ];
    let actual = records
        .lock()
        .expect("token usage records lock")
        .drain(..)
        .collect::<Vec<_>>();
    assert_eq!(expected, actual);
}

#[tokio::test]
async fn config_change_contributor_observes_effective_config_changes() {
    struct SessionConfigMarker;
    struct ThreadConfigMarker;

    #[derive(Debug, PartialEq)]
    struct RecordedConfigChange {
        previous_model: Option<String>,
        new_model: Option<String>,
        previous_disabled_tools: Vec<ToolSuggestDisabledTool>,
        new_disabled_tools: Vec<ToolSuggestDisabledTool>,
        saw_session_store: bool,
        saw_thread_store: bool,
    }

    struct ConfigRecorder {
        records: Arc<std::sync::Mutex<Vec<RecordedConfigChange>>>,
    }

    impl codex_extension_api::ConfigContributor<crate::config::Config> for ConfigRecorder {
        fn on_config_changed(
            &self,
            session_store: &codex_extension_api::ExtensionData,
            thread_store: &codex_extension_api::ExtensionData,
            previous_config: &crate::config::Config,
            new_config: &crate::config::Config,
        ) {
            self.records
                .lock()
                .expect("config change records lock")
                .push(RecordedConfigChange {
                    previous_model: previous_config.model.clone(),
                    new_model: new_config.model.clone(),
                    previous_disabled_tools: previous_config.tool_suggest.disabled_tools.clone(),
                    new_disabled_tools: new_config.tool_suggest.disabled_tools.clone(),
                    saw_session_store: session_store.get::<SessionConfigMarker>().is_some(),
                    saw_thread_store: thread_store.get::<ThreadConfigMarker>().is_some(),
                });
        }
    }

    let (mut session, _turn_context) = make_session_and_context().await;
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.config_contributor(Arc::new(ConfigRecorder {
        records: Arc::clone(&records),
    }));
    session.services.extensions = Arc::new(builder.build());
    session
        .services
        .session_extension_data
        .insert(SessionConfigMarker);
    session
        .services
        .thread_extension_data
        .insert(ThreadConfigMarker);

    let original_model = session.collaboration_mode().await.model().to_string();
    let original_disabled_tools = session
        .get_config()
        .await
        .tool_suggest
        .disabled_tools
        .clone();
    let next_model = if original_model == "gpt-5.4" {
        "gpt-5.2"
    } else {
        "gpt-5.4"
    };
    let collaboration_mode = session.collaboration_mode().await.with_updates(
        Some(next_model.to_string()),
        /*effort*/ None,
        /*developer_instructions*/ None,
    );
    session
        .update_settings(SessionSettingsUpdate {
            collaboration_mode: Some(collaboration_mode),
            ..Default::default()
        })
        .await
        .expect("update settings");

    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    std::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        r#"[tool_suggest]
disabled_tools = [
  { type = "connector", id = " calendar " },
  { type = "plugin", id = "slack@openai-curated" },
]
"#,
    )
    .expect("write user config");
    let next_config = load_latest_config_for_session(&session).await;
    session.refresh_runtime_config(next_config).await;

    let expected_disabled_tools = vec![
        ToolSuggestDisabledTool::connector("calendar"),
        ToolSuggestDisabledTool::plugin("slack@openai-curated"),
    ];
    let expected = vec![
        RecordedConfigChange {
            previous_model: Some(original_model),
            new_model: Some(next_model.to_string()),
            previous_disabled_tools: original_disabled_tools.clone(),
            new_disabled_tools: original_disabled_tools.clone(),
            saw_session_store: true,
            saw_thread_store: true,
        },
        RecordedConfigChange {
            previous_model: Some(next_model.to_string()),
            new_model: Some(next_model.to_string()),
            previous_disabled_tools: original_disabled_tools,
            new_disabled_tools: expected_disabled_tools,
            saw_session_store: true,
            saw_thread_store: true,
        },
    ];
    let actual = records
        .lock()
        .expect("config change records lock")
        .drain(..)
        .collect::<Vec<_>>();
    assert_eq!(expected, actual);
}

#[tokio::test]
async fn record_initial_history_reconstructs_forked_transcript() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, expected) = sample_rollout(&session, &turn_context).await;

    session
        .record_initial_history(InitialHistory::Forked(rollout_items))
        .await;

    let history = session.state.lock().await.clone_history();
    assert_eq!(expected, history.raw_items());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_configured_reports_permission_profile_for_external_sandbox() -> anyhow::Result<()>
{
    let server = start_mock_server().await;
    let sandbox_policy = SandboxPolicy::ExternalSandbox {
        network_access: protocol::protocol::NetworkAccess::Restricted,
    };
    let expected_sandbox_policy = sandbox_policy.clone();
    let mut builder = test_codex().with_config(move |config| {
        config
            .permissions
            .set_permission_profile(PermissionProfile::from_legacy_sandbox_policy(
                &sandbox_policy,
            ))
            .expect("set permission profile");
        config
            .set_legacy_sandbox_policy(sandbox_policy)
            .expect("set sandbox policy");
    });

    let test = builder.build(&server).await?;

    let expected_permission_profile =
        protocol::models::PermissionProfile::from_legacy_sandbox_policy(&expected_sandbox_policy);
    assert_eq!(
        test.session_configured.permission_profile, expected_permission_profile,
        "ExternalSandbox is represented explicitly instead of as a lossy root-write profile"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_permission_profile_rebinds_runtime_workspace_roots() -> anyhow::Result<()> {
    let codex_home = tempfile::TempDir::new()?;
    let cwd = tempfile::TempDir::new()?;
    let old_root = test_path_buf("/workspace/old").abs();
    let new_root = test_path_buf("/workspace/new").abs();
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(crate::config::ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            default_permissions: Some(BUILT_IN_PERMISSION_PROFILE_WORKSPACE.to_string()),
            additional_writable_roots: vec![old_root.to_path_buf()],
            ..Default::default()
        })
        .build()
        .await?;

    let session_permission_profile_state = session_permission_profile_state_from_config(&config)?;
    let stored_file_system_policy = session_permission_profile_state
        .permission_profile()
        .file_system_sandbox_policy();
    assert!(
        !stored_file_system_policy
            .can_write_path_with_cwd(old_root.as_path(), config.cwd.as_path()),
        "session permission profile state should keep runtime workspace roots symbolic"
    );

    let mut session_configuration = make_session_configuration_for_tests().await;
    session_configuration.cwd = config.cwd.clone();
    session_configuration.workspace_roots = config.workspace_roots.clone();
    session_configuration.permission_profile_state = session_permission_profile_state;

    let initial_policy = session_configuration.file_system_sandbox_policy();
    assert!(initial_policy.can_write_path_with_cwd(old_root.as_path(), config.cwd.as_path()));

    let updated = session_configuration.apply(&SessionSettingsUpdate {
        workspace_roots: Some(vec![new_root.clone()]),
        ..Default::default()
    })?;
    let updated_policy = updated.file_system_sandbox_policy();
    assert!(updated_policy.can_write_path_with_cwd(new_root.as_path(), updated.cwd.as_path()));
    assert!(!updated_policy.can_write_path_with_cwd(old_root.as_path(), updated.cwd.as_path()));
    Ok(())
}
