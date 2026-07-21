use super::*;
use crate::CodexThread;
use crate::PendingInputItem;
use crate::StateDbHandle;
use crate::ThreadService;
use crate::TurnContext;
use crate::agent::AgentMode;
use crate::agent::agent_status_from_event;
use crate::config::AgentRoleConfig;
use crate::config::Config;
use crate::config::ConfigBuilder;
use crate::state_db_bridge::init_state_db;
use assert_matches::assert_matches;
use codex_agent_runtime::SpawnAgentForkMode;
use codex_agent_runtime::ThreadIdleReason;
use codex_agent_runtime::ThreadPostTurnState;
use codex_context_manager::ContextualUserFragment;
use codex_context_manager::SubagentNotification;
use codex_features::Feature;
use codex_login::CodexAuth;
use goal_service::GoalService;
use goal_service_api::GoalServiceApi;
use pretty_assertions::assert_eq;
use protocol::AgentPath;
use protocol::config_types::ModeKind;
use protocol::models::ContentItem;
use protocol::models::MessagePhase;
use protocol::models::ResponseItem;
use protocol::protocol::ErrorEvent;
use protocol::protocol::Event;
use protocol::protocol::EventMsg;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::protocol::ThreadLifecycleStatus;
use protocol::protocol::TurnAbortReason;
use protocol::protocol::TurnAbortedEvent;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::TurnStartedEvent;
use protocol::user_input::UserInput;
use state_api::DirectionalThreadSpawnEdgeStatus;
use state_api::ThreadGoalStatus as StateThreadGoalStatus;
use tempfile::TempDir;
use thread_store::LocalThreadStore;
use thread_store::LocalThreadStoreConfig;
use thread_store_api::ArchiveThreadParams;
use thread_store_api::ThreadStore;
use tokio::time::Duration;
use tokio::time::sleep;
use tokio::time::timeout;
use toml::Value as TomlValue;

async fn test_config_with_cli_overrides(
    cli_overrides: Vec<(String, TomlValue)>,
) -> (TempDir, Config) {
    let home = TempDir::new().expect("create temp dir");
    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(home.path().to_path_buf())
        .fallback_cwd(Some(home.path().to_path_buf()))
        .cli_overrides(cli_overrides)
        .build()
        .await
        .expect("load default test config");
    (home, config)
}

async fn test_config() -> (TempDir, Config) {
    test_config_with_cli_overrides(Vec::new()).await
}

fn text_input(text: &str) -> Op {
    vec![UserInput::Text {
        text: text.to_string(),
        text_elements: Vec::new(),
    }]
    .into()
}

fn assistant_message(text: &str, phase: Option<MessagePhase>) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase,
    }
}

fn spawn_agent_call(call_id: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: "spawn_agent".to_string(),
        namespace: None,
        arguments: "{}".to_string(),
        call_id: call_id.to_string(),
    }
}

struct AgentControlHarness {
    _home: TempDir,
    config: Config,
    state_db: Option<StateDbHandle>,
    manager: ThreadService,
    control: AgentControl,
}

impl AgentControlHarness {
    async fn new() -> Self {
        let (home, config) = test_config().await;
        let state_db = init_state_db(&config).await;
        let manager = ThreadService::with_models_provider_home_and_state_for_tests(
            CodexAuth::from_api_key("dummy"),
            config.model_provider.clone(),
            crate::test_support::model_provider_factory_for_tests(),
            config.codex_home.to_path_buf(),
            std::sync::Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
            state_db.clone(),
        );
        let control = manager.agent_control();
        Self {
            _home: home,
            config,
            state_db,
            manager,
            control,
        }
    }

    async fn start_thread(&self) -> (ThreadId, Arc<CodexThread>) {
        let new_thread = self
            .manager
            .start_thread(self.config.clone())
            .await
            .expect("start thread");
        (new_thread.thread_id, new_thread.thread)
    }

    fn restarted_manager_and_control(&self) -> (ThreadService, AgentControl) {
        let manager = ThreadService::with_models_provider_home_and_state_for_tests(
            CodexAuth::from_api_key("dummy"),
            self.config.model_provider.clone(),
            crate::test_support::model_provider_factory_for_tests(),
            self.config.codex_home.to_path_buf(),
            std::sync::Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
            self.state_db.clone(),
        );
        let control = manager.agent_control();
        (manager, control)
    }
}

fn has_subagent_notification(history_items: &[ResponseItem]) -> bool {
    history_items.iter().any(|item| {
        let ResponseItem::Message { role, content, .. } = item else {
            return false;
        };
        if role != "user" {
            return false;
        }
        content.iter().any(|content_item| match content_item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                SubagentNotification::matches_text(text)
            }
            ContentItem::InputImage { .. } => false,
        })
    })
}

/// Returns true when any message item contains `needle` in a text span.
fn history_contains_text(history_items: &[ResponseItem], needle: &str) -> bool {
    history_items.iter().any(|item| {
        let ResponseItem::Message { content, .. } = item else {
            return false;
        };
        content.iter().any(|content_item| match content_item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                text.contains(needle)
            }
            ContentItem::InputImage { .. } => false,
        })
    })
}

fn history_contains_inter_agent_communication(
    history_items: &[ResponseItem],
    expected: &InterAgentCommunication,
) -> bool {
    history_items.iter().any(|item| {
        matches!(
            item,
            ResponseItem::InterAgentCommunication { communication, .. }
                if communication == expected
        )
    })
}

async fn wait_for_subagent_notification(parent_thread: &Arc<CodexThread>) -> bool {
    let wait = async {
        loop {
            let history_items = parent_thread
                .codex
                .session
                .clone_history()
                .await
                .raw_items()
                .to_vec();
            if has_subagent_notification(&history_items) {
                return true;
            }
            sleep(Duration::from_millis(25)).await;
        }
    };
    // CI can take several seconds to schedule the detached status watcher,
    // especially on slower Windows runners.
    timeout(Duration::from_secs(10), wait).await.is_ok()
}

async fn no_subagent_notification(parent_thread: &Arc<CodexThread>) -> bool {
    !timeout(Duration::from_millis(200), async {
        loop {
            let history_items = parent_thread
                .codex
                .session
                .clone_history()
                .await
                .raw_items()
                .to_vec();
            if has_subagent_notification(&history_items) {
                return;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .is_ok()
}

async fn persist_thread_for_tree_resume(thread: &Arc<CodexThread>, message: &str) {
    thread
        .inject_user_message_without_turn(message.to_string())
        .await;
    thread.codex.session.ensure_rollout_materialized().await;
    thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("test thread rollout should flush");
}

async fn archive_thread_for_test(harness: &AgentControlHarness, thread_id: ThreadId) {
    let store = LocalThreadStore::new(
        LocalThreadStoreConfig::from_config(&harness.config),
        harness.state_db.clone(),
    );
    store
        .archive_thread(ArchiveThreadParams { thread_id })
        .await
        .expect("test thread should archive");
}

async fn delete_thread_metadata_for_test(harness: &AgentControlHarness, thread_id: ThreadId) {
    let state_db = harness
        .state_db
        .as_ref()
        .expect("sqlite state db should be available");
    assert_eq!(
        state_db
            .delete_thread(thread_id)
            .await
            .expect("test thread metadata should delete"),
        1,
        "test should delete exactly one thread metadata row",
    );
}

async fn wait_for_live_thread_spawn_children(
    control: &AgentControl,
    parent_thread_id: ThreadId,
    expected_children: &[ThreadId],
) {
    let mut expected_children = expected_children.to_vec();
    expected_children.sort_by_key(std::string::ToString::to_string);

    timeout(Duration::from_secs(5), async {
        loop {
            let mut child_ids = control
                .open_thread_spawn_children(parent_thread_id)
                .await
                .expect("live child list should load")
                .into_iter()
                .map(|(thread_id, _)| thread_id)
                .collect::<Vec<_>>();
            child_ids.sort_by_key(std::string::ToString::to_string);
            if child_ids == expected_children {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("expected persisted child tree");
}

async fn emit_turn_complete(
    thread: &Arc<CodexThread>,
    last_agent_message: &str,
) -> Arc<TurnContext> {
    let turn = thread.codex.session.new_default_turn().await;
    thread
        .codex
        .session
        .send_event(
            turn.as_ref(),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: turn.sub_id.clone(),
                last_agent_message: Some(last_agent_message.to_string()),
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        )
        .await;
    *thread.codex.session.active_turn.lock().await = None;
    turn
}

async fn replace_thread_goal(
    state_db: &StateDbHandle,
    thread_id: ThreadId,
    status: StateThreadGoalStatus,
) {
    state_db
        .replace_thread_goal(thread_id, "finish the worker task", status, None)
        .await
        .expect("thread goal should be written");
}

fn captured_child_completion(
    captured_ops: &[(ThreadId, Op)],
    parent_thread_id: ThreadId,
    child_agent_path: &AgentPath,
    parent_agent_path: &AgentPath,
) -> bool {
    count_captured_child_completions(
        captured_ops,
        parent_thread_id,
        child_agent_path,
        parent_agent_path,
    ) > 0
}

fn count_captured_child_completions(
    captured_ops: &[(ThreadId, Op)],
    parent_thread_id: ThreadId,
    child_agent_path: &AgentPath,
    parent_agent_path: &AgentPath,
) -> usize {
    captured_ops
        .iter()
        .filter(|(thread_id, op)| {
            *thread_id == parent_thread_id
                && matches!(
                    op,
                    Op::InterAgentCommunication { communication }
                        if communication.author == *child_agent_path
                            && communication.recipient == *parent_agent_path
                            && communication.operation
                                == protocol::protocol::InterAgentOperation::ChildCompletion
                )
        })
        .count()
}

#[tokio::test]
async fn send_input_errors_when_manager_dropped() {
    let control = AgentControl::default();
    let err = control
        .send_input(
            ThreadId::new(),
            vec![UserInput::Text {
                text: "hello".to_string(),
                text_elements: Vec::new(),
            }]
            .into(),
        )
        .await
        .expect_err("send_input should fail without a manager");
    assert_eq!(
        err.to_string(),
        "unsupported operation: thread manager dropped"
    );
}

#[tokio::test]
async fn get_status_returns_not_found_without_manager() {
    let control = AgentControl::default();
    let got = control.get_status(ThreadId::new()).await;
    assert_eq!(got, AgentStatus::NotFound);
}

#[tokio::test]
async fn get_status_returns_external_agent_status_without_manager() {
    let control = AgentControl::default();
    let thread_id = ThreadId::new();
    control.external_agents.insert_running(ExternalAgentRun {
        thread_id,
        parent_thread_id: ThreadId::new(),
        agent_path: AgentPath::try_from("/root/external").expect("agent path"),
        provider: SpawnAgentProvider::CodexCli,
        depth: 1,
        spawn_config: None,
        status: AgentStatus::Running,
        last_task_message: Some("do work".to_string()),
        abort_handle: None,
    });

    let got = control.get_status(thread_id).await;
    assert_eq!(got, AgentStatus::Running);
}

#[tokio::test]
async fn direct_agent_children_are_active_includes_external_runs() {
    let control = AgentControl::default();
    let parent_thread_id = ThreadId::new();
    control.external_agents.insert_running(ExternalAgentRun {
        thread_id: ThreadId::new(),
        parent_thread_id,
        agent_path: AgentPath::try_from("/root/external").expect("agent path"),
        provider: SpawnAgentProvider::ClaudeCli,
        depth: 1,
        spawn_config: None,
        status: AgentStatus::Running,
        last_task_message: Some("do work".to_string()),
        abort_handle: None,
    });

    assert!(
        control
            .direct_agent_children_are_active(parent_thread_id)
            .await
    );
}

#[tokio::test]
async fn list_agents_includes_external_runs_with_prefix_filter() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    harness.control.state.register_root_thread(root_thread_id);
    let external_thread_id = ThreadId::new();
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: AgentPath::try_from("/root/external").expect("agent path"),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(harness.config.clone()),
            status: AgentStatus::Completed(Some("done".to_string())),
            last_task_message: Some("do work".to_string()),
            abort_handle: None,
        });

    let agents = harness
        .control
        .list_agents(root_thread_id, &SessionSource::Exec, Some("external"))
        .await
        .expect("list agents");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_name, "/root/external");
    assert_eq!(agents[0].agent_nickname.as_deref(), Some("codex_cli"));
    assert_eq!(agents[0].agent_role.as_deref(), Some("codex_cli"));
    assert_eq!(
        agents[0].lifecycle_status,
        ThreadLifecycleStatus::completed(Some("done".to_string()))
    );
    assert_eq!(agents[0].last_task_message.as_deref(), Some("do work"));
}

#[tokio::test]
async fn external_completion_after_close_does_not_notify_parent() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    let child_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    harness.control.state.register_root_thread(root_thread_id);
    harness
        .control
        .state
        .register_agent_metadata(AgentMetadata {
            agent_id: Some(external_thread_id),
            agent_path: Some(child_agent_path.clone()),
            counted: false,
            ..Default::default()
        });
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: child_agent_path.clone(),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(harness.config.clone()),
            status: AgentStatus::Running,
            last_task_message: Some("do work".to_string()),
            abort_handle: None,
        });
    harness
        .control
        .close_agent(external_thread_id)
        .await
        .expect("close external agent");

    harness
        .control
        .complete_external_agent(
            external_thread_id,
            AgentStatus::Completed(Some("late".to_string())),
        )
        .await;

    assert!(!captured_child_completion(
        &harness.manager.captured_ops(),
        root_thread_id,
        &child_agent_path,
        &AgentPath::root()
    ));
    assert_eq!(
        harness.control.get_status(external_thread_id).await,
        AgentStatus::Shutdown
    );
}

#[tokio::test]
async fn external_tool_call_lists_visible_agents() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    harness.control.state.register_root_thread(root_thread_id);
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: AgentPath::try_from("/root/external").expect("agent path"),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(harness.config.clone()),
            status: AgentStatus::Running,
            last_task_message: Some("do work".to_string()),
            abort_handle: None,
        });

    let result = harness
        .control
        .dispatch_external_tool_call(
            external_thread_id,
            ExternalToolCall {
                id: "call_1".to_string(),
                tool: ExternalToolName::ListExternalAgents,
                arguments: serde_json::json!({ "path_prefix": "/root/external" }),
            },
        )
        .await;

    assert!(result.ok);
    let agents = result.result.expect("tool result")["agents"]
        .as_array()
        .expect("agents array")
        .clone();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["agent_name"], "/root/external");
    assert_eq!(agents[0]["agent_nickname"], "codex_cli");
}

#[tokio::test]
async fn external_tool_call_malformed_arguments_returns_bounded_error() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    harness.control.state.register_root_thread(root_thread_id);
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: AgentPath::try_from("/root/external").expect("agent path"),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(harness.config.clone()),
            status: AgentStatus::Running,
            last_task_message: Some("do work".to_string()),
            abort_handle: None,
        });

    let result = harness
        .control
        .dispatch_external_tool_call(
            external_thread_id,
            ExternalToolCall {
                id: "call_1".to_string(),
                tool: ExternalToolName::FollowupExternalTask,
                arguments: serde_json::json!({ "target": "/root/native" }),
            },
        )
        .await;

    assert!(!result.ok);
    let error = result.error.expect("tool error");
    assert_eq!(error.code, "tool_error");
    assert!(
        error
            .message
            .contains("failed to parse external tool arguments")
    );
}

#[tokio::test]
async fn external_tool_call_followup_to_native_uses_agent_bus() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    harness.control.state.register_root_thread(root_thread_id);
    let (native_thread_id, _native_thread) = harness.start_thread().await;
    let native_agent_path = AgentPath::try_from("/root/native").expect("agent path");
    harness
        .control
        .state
        .register_agent_metadata(AgentMetadata {
            agent_id: Some(native_thread_id),
            agent_path: Some(native_agent_path.clone()),
            counted: false,
            ..Default::default()
        });
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path.clone(),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(harness.config.clone()),
            status: AgentStatus::Running,
            last_task_message: Some("do work".to_string()),
            abort_handle: None,
        });

    let result = harness
        .control
        .dispatch_external_tool_call(
            external_thread_id,
            ExternalToolCall {
                id: "call_1".to_string(),
                tool: ExternalToolName::FollowupExternalTask,
                arguments: serde_json::json!({
                    "target": "/root/native",
                    "message": "please review"
                }),
            },
        )
        .await;

    assert!(result.ok);
    let captured = harness.manager.captured_ops();
    assert!(captured.iter().any(|(thread_id, op)| {
        *thread_id == native_thread_id
            && matches!(
                op,
                Op::InterAgentCommunication { communication }
                    if communication.author == external_agent_path
                        && communication.recipient == native_agent_path
                        && communication.content == "please review"
                        && communication.trigger_turn
            )
    }));
}

#[tokio::test]
async fn external_run_loop_reinvokes_with_list_result_transcript() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    harness.control.state.register_root_thread(root_thread_id);
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: AgentPath::try_from("/root/external").expect("agent path"),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(harness.config.clone()),
            status: AgentStatus::Running,
            last_task_message: Some("inspect agents".to_string()),
            abort_handle: None,
        });
    let prompts = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let prompts_for_invoker = std::sync::Arc::clone(&prompts);
    let calls_for_invoker = std::sync::Arc::clone(&calls);

    let status = harness
        .control
        .run_external_agent_loop_with_invoker(
            external_thread_id,
            SpawnAgentProvider::CodexCli,
            harness.config.cwd.as_path().to_path_buf(),
            "inspect agents".to_string(),
            move |_provider, _cwd, message, transcript| {
                prompts_for_invoker
                    .lock()
                    .expect("prompts lock")
                    .push(crate::agent::external::external_agent_context_prompt(
                        &message,
                        &transcript,
                    ));
                let call_index =
                    calls_for_invoker.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::future::ready(if call_index == 0 {
                    crate::agent::external::ExternalCliTurn {
                        status: AgentStatus::Completed(Some("need list".to_string())),
                        tool_calls: vec![ExternalToolCall {
                            id: "call_1".to_string(),
                            tool: ExternalToolName::ListExternalAgents,
                            arguments: serde_json::json!({ "path_prefix": "/root/external" }),
                        }],
                        tool_call_errors: Vec::new(),
                        transcript_lines: vec![serde_json::json!({
                            "type": "external_tool_call",
                            "id": "call_1",
                            "tool": "list_external_agents",
                            "arguments": { "path_prefix": "/root/external" },
                        })
                        .to_string()],
                    }
                } else {
                    crate::agent::external::ExternalCliTurn {
                        status: AgentStatus::Completed(Some("done after result".to_string())),
                        tool_calls: Vec::new(),
                        tool_call_errors: Vec::new(),
                        transcript_lines: Vec::new(),
                    }
                })
            },
        )
        .await;

    assert_eq!(
        status,
        AgentStatus::Completed(Some("done after result".to_string()))
    );
    let prompts = prompts.lock().expect("prompts lock");
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("external_tool_result"));
    assert!(prompts[1].contains("\"agents\""));
    assert!(prompts[1].contains("/root/external"));
}

#[tokio::test]
async fn external_run_loop_followup_result_drives_continuation() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    harness.control.state.register_root_thread(root_thread_id);
    let (native_thread_id, _native_thread) = harness.start_thread().await;
    let native_agent_path = AgentPath::try_from("/root/native").expect("agent path");
    harness
        .control
        .state
        .register_agent_metadata(AgentMetadata {
            agent_id: Some(native_thread_id),
            agent_path: Some(native_agent_path.clone()),
            counted: false,
            ..Default::default()
        });
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path.clone(),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(harness.config.clone()),
            status: AgentStatus::Running,
            last_task_message: Some("coordinate".to_string()),
            abort_handle: None,
        });
    let second_transcript = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let second_transcript_for_invoker = std::sync::Arc::clone(&second_transcript);
    let calls_for_invoker = std::sync::Arc::clone(&calls);

    let status = harness
        .control
        .run_external_agent_loop_with_invoker(
            external_thread_id,
            SpawnAgentProvider::CodexCli,
            harness.config.cwd.as_path().to_path_buf(),
            "coordinate".to_string(),
            move |_provider, _cwd, _message, transcript| {
                let call_index =
                    calls_for_invoker.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if call_index == 1 {
                    *second_transcript_for_invoker
                        .lock()
                        .expect("transcript lock") = Some(transcript.clone());
                }
                std::future::ready(if call_index == 0 {
                    crate::agent::external::ExternalCliTurn {
                        status: AgentStatus::Completed(Some("need followup".to_string())),
                        tool_calls: vec![ExternalToolCall {
                            id: "call_1".to_string(),
                            tool: ExternalToolName::FollowupExternalTask,
                            arguments: serde_json::json!({
                                "target": "/root/native",
                                "message": "please check this"
                            }),
                        }],
                        tool_call_errors: Vec::new(),
                        transcript_lines: Vec::new(),
                    }
                } else {
                    crate::agent::external::ExternalCliTurn {
                        status: AgentStatus::Completed(Some("continued after delivery".to_string())),
                        tool_calls: Vec::new(),
                        tool_call_errors: Vec::new(),
                        transcript_lines: Vec::new(),
                    }
                })
            },
        )
        .await;

    assert_eq!(
        status,
        AgentStatus::Completed(Some("continued after delivery".to_string()))
    );
    let transcript = second_transcript
        .lock()
        .expect("transcript lock")
        .clone()
        .expect("second transcript");
    assert!(transcript.contains("external_tool_result"));
    assert!(transcript.contains("\"delivered\":true"));
    let captured = harness.manager.captured_ops();
    assert!(captured.iter().any(|(thread_id, op)| {
        *thread_id == native_thread_id
            && matches!(
                op,
                Op::InterAgentCommunication { communication }
                    if communication.author == external_agent_path
                        && communication.recipient == native_agent_path
                        && communication.content == "please check this"
                        && communication.trigger_turn
            )
    }));
}

#[tokio::test]
async fn external_run_loop_stops_at_max_iterations() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    harness.control.state.register_root_thread(root_thread_id);
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: AgentPath::try_from("/root/external").expect("agent path"),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(harness.config.clone()),
            status: AgentStatus::Running,
            last_task_message: Some("loop forever".to_string()),
            abort_handle: None,
        });

    let status = harness
        .control
        .run_external_agent_loop_with_invoker(
            external_thread_id,
            SpawnAgentProvider::CodexCli,
            harness.config.cwd.as_path().to_path_buf(),
            "loop forever".to_string(),
            |_provider, _cwd, _message, _transcript| {
                std::future::ready(crate::agent::external::ExternalCliTurn {
                    status: AgentStatus::Completed(Some("need list".to_string())),
                    tool_calls: vec![ExternalToolCall {
                        id: "call_1".to_string(),
                        tool: ExternalToolName::ListExternalAgents,
                        arguments: serde_json::json!({}),
                    }],
                    tool_call_errors: Vec::new(),
                    transcript_lines: Vec::new(),
                })
            },
        )
        .await;

    assert_matches!(
        status,
        AgentStatus::Errored(message)
            if message.contains("external tool iteration limit reached")
    );
}

#[tokio::test]
async fn external_run_loop_reinvokes_once_for_malformed_tool_result() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    harness.control.state.register_root_thread(root_thread_id);
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: AgentPath::try_from("/root/external").expect("agent path"),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(harness.config.clone()),
            status: AgentStatus::Running,
            last_task_message: Some("recover malformed call".to_string()),
            abort_handle: None,
        });
    let second_transcript = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let second_transcript_for_invoker = std::sync::Arc::clone(&second_transcript);
    let calls_for_invoker = std::sync::Arc::clone(&calls);

    let status = harness
        .control
        .run_external_agent_loop_with_invoker(
            external_thread_id,
            SpawnAgentProvider::CodexCli,
            harness.config.cwd.as_path().to_path_buf(),
            "recover malformed call".to_string(),
            move |_provider, _cwd, _message, transcript| {
                let call_index =
                    calls_for_invoker.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if call_index == 1 {
                    *second_transcript_for_invoker
                        .lock()
                        .expect("transcript lock") = Some(transcript.clone());
                }
                std::future::ready(if call_index == 0 {
                    crate::agent::external::ExternalCliTurn {
                        status: AgentStatus::Completed(Some("bad tool call".to_string())),
                        tool_calls: Vec::new(),
                        tool_call_errors: vec![ExternalToolResult::error(
                            "call_bad",
                            "invalid_tool_call",
                            "failed to parse external tool call",
                        )],
                        transcript_lines: Vec::new(),
                    }
                } else {
                    crate::agent::external::ExternalCliTurn {
                        status: AgentStatus::Completed(Some("recovered".to_string())),
                        tool_calls: Vec::new(),
                        tool_call_errors: Vec::new(),
                        transcript_lines: Vec::new(),
                    }
                })
            },
        )
        .await;

    assert_eq!(status, AgentStatus::Completed(Some("recovered".to_string())));
    let transcript = second_transcript
        .lock()
        .expect("transcript lock")
        .clone()
        .expect("second transcript");
    assert_eq!(transcript.matches("\"id\":\"call_bad\"").count(), 1);
    assert!(transcript.contains("invalid_tool_call"));
}

#[tokio::test]
async fn on_event_updates_status_from_task_started() {
    let status = agent_status_from_event(&EventMsg::TurnStarted(TurnStartedEvent {
        turn_id: "turn-1".to_string(),
        started_at: None,
        model_context_window: None,
        collaboration_mode_kind: ModeKind::Default,
    }));
    assert_eq!(status, Some(AgentStatus::Running));
}

#[tokio::test]
async fn on_event_updates_status_from_task_complete() {
    let status = agent_status_from_event(&EventMsg::TurnComplete(TurnCompleteEvent {
        turn_id: "turn-1".to_string(),
        last_agent_message: Some("done".to_string()),
        completed_at: None,
        duration_ms: None,
        time_to_first_token_ms: None,
    }));
    let expected = AgentStatus::Completed(Some("done".to_string()));
    assert_eq!(status, Some(expected));
}

#[tokio::test]
async fn on_event_updates_status_from_error() {
    let status = agent_status_from_event(&EventMsg::Error(ErrorEvent {
        message: "boom".to_string(),
        codex_error_info: None,
    }));

    let expected = AgentStatus::Errored("boom".to_string());
    assert_eq!(status, Some(expected));
}

#[tokio::test]
async fn on_event_updates_status_from_turn_aborted() {
    let status = agent_status_from_event(&EventMsg::TurnAborted(TurnAbortedEvent {
        turn_id: Some("turn-1".to_string()),
        reason: TurnAbortReason::Interrupted,
        completed_at: None,
        duration_ms: None,
    }));

    let expected = AgentStatus::Interrupted;
    assert_eq!(status, Some(expected));
}

#[tokio::test]
async fn on_event_updates_status_from_shutdown_complete() {
    let status = agent_status_from_event(&EventMsg::ShutdownComplete);
    assert_eq!(status, Some(AgentStatus::Shutdown));
}

#[tokio::test]
async fn spawn_agent_errors_when_manager_dropped() {
    let control = AgentControl::default();
    let (_home, config) = test_config().await;
    let err = control
        .spawn_agent(config, text_input("hello"), /*session_source*/ None)
        .await
        .expect_err("spawn_agent should fail without a manager");
    assert_eq!(
        err.to_string(),
        "unsupported operation: thread manager dropped"
    );
}

#[tokio::test]
async fn resume_agent_errors_when_manager_dropped() {
    let control = AgentControl::default();
    let (_home, config) = test_config().await;
    let err = control
        .resume_agent_from_rollout(config, ThreadId::new(), SessionSource::Exec)
        .await
        .expect_err("resume_agent should fail without a manager");
    assert_eq!(
        err.to_string(),
        "unsupported operation: thread manager dropped"
    );
}

#[tokio::test]
async fn send_input_errors_when_thread_missing() {
    let harness = AgentControlHarness::new().await;
    let thread_id = ThreadId::new();
    let err = harness
        .control
        .send_input(
            thread_id,
            vec![UserInput::Text {
                text: "hello".to_string(),
                text_elements: Vec::new(),
            }]
            .into(),
        )
        .await
        .expect_err("send_input should fail for missing thread");
    assert_matches!(err, CodexErr::ThreadNotFound(id) if id == thread_id);
}

#[tokio::test]
async fn get_status_returns_not_found_for_missing_thread() {
    let harness = AgentControlHarness::new().await;
    let status = harness.control.get_status(ThreadId::new()).await;
    assert_eq!(status, AgentStatus::NotFound);
}

#[tokio::test]
async fn get_status_returns_pending_init_for_new_thread() {
    let harness = AgentControlHarness::new().await;
    let (thread_id, _) = harness.start_thread().await;
    let status = harness.control.get_status(thread_id).await;
    assert_eq!(status, AgentStatus::PendingInit);
}

#[tokio::test]
async fn subscribe_status_errors_for_missing_thread() {
    let harness = AgentControlHarness::new().await;
    let thread_id = ThreadId::new();
    let err = harness
        .control
        .subscribe_status(thread_id)
        .await
        .expect_err("subscribe_status should fail for missing thread");
    assert_matches!(err, CodexErr::ThreadNotFound(id) if id == thread_id);
}

#[tokio::test]
async fn subscribe_status_updates_on_shutdown() {
    let harness = AgentControlHarness::new().await;
    let (thread_id, thread) = harness.start_thread().await;
    let mut status_rx = harness
        .control
        .subscribe_status(thread_id)
        .await
        .expect("subscribe_status should succeed");
    assert_eq!(status_rx.borrow().clone(), AgentStatus::PendingInit);

    let _ = thread
        .submit(Op::Shutdown {})
        .await
        .expect("shutdown should submit");

    let _ = status_rx.changed().await;
    assert_eq!(status_rx.borrow().clone(), AgentStatus::Shutdown);
}

#[tokio::test]
async fn send_input_submits_user_message() {
    let harness = AgentControlHarness::new().await;
    let (thread_id, _thread) = harness.start_thread().await;

    let submission_id = harness
        .control
        .send_input(
            thread_id,
            vec![UserInput::Text {
                text: "hello from tests".to_string(),
                text_elements: Vec::new(),
            }]
            .into(),
        )
        .await
        .expect("send_input should succeed");
    assert!(!submission_id.is_empty());
    let expected = (
        thread_id,
        Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "hello from tests".to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        },
    );
    let captured = harness
        .manager
        .captured_ops()
        .into_iter()
        .find(|entry| *entry == expected);
    assert_eq!(captured, Some(expected));
}

#[tokio::test]
async fn send_inter_agent_communication_without_turn_queues_message_without_triggering_turn() {
    let harness = AgentControlHarness::new().await;
    let (thread_id, thread) = harness.start_thread().await;
    let communication = InterAgentCommunication::new(
        AgentPath::root(),
        AgentPath::try_from("/root/worker").expect("agent path"),
        Vec::new(),
        "hello from tests".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);

    let submission_id = harness
        .control
        .send_inter_agent_communication(thread_id, communication.clone())
        .await
        .expect("send_inter_agent_communication should succeed");
    assert!(!submission_id.is_empty());

    let expected = (
        thread_id,
        Op::InterAgentCommunication {
            communication: communication.clone(),
        },
    );
    let captured = harness
        .manager
        .captured_ops()
        .into_iter()
        .find(|entry| *entry == expected);
    assert_eq!(captured, Some(expected));

    timeout(Duration::from_secs(5), async {
        loop {
            if thread.codex.session.has_pending_input().await {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("inter-agent communication should stay pending");

    let history_items = thread
        .codex
        .session
        .clone_history()
        .await
        .raw_items()
        .to_vec();
    assert!(!history_contains_inter_agent_communication(
        &history_items,
        &communication
    ));
}

#[tokio::test]
async fn append_message_records_assistant_message() {
    let harness = AgentControlHarness::new().await;
    let (thread_id, thread) = harness.start_thread().await;
    let message =
        "author: /root\nrecipient: /root/worker\nother_recipients: []\nContent: hello from tests";

    let submission_id = harness
        .control
        .append_message(
            thread_id,
            ResponseItem::Message {
                id: None,
                role: "assistant".to_string(),
                content: vec![ContentItem::InputText {
                    text: message.to_string(),
                }],
                phase: None,
            },
        )
        .await
        .expect("append_message should succeed");
    assert!(!submission_id.is_empty());

    timeout(Duration::from_secs(5), async {
        loop {
            let history_items = thread
                .codex
                .session
                .clone_history()
                .await
                .raw_items()
                .to_vec();
            let recorded = history_items.iter().any(|item| {
                matches!(
                    item,
                    ResponseItem::Message { role, content, .. }
                        if role == "assistant"
                            && content.iter().any(|content_item| matches!(
                                content_item,
                                ContentItem::InputText { text } if text == message
                            ))
                )
            });
            if recorded {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("assistant message should be recorded");
}

#[tokio::test]
async fn spawn_agent_creates_thread_and_sends_prompt() {
    let harness = AgentControlHarness::new().await;
    let thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("spawned"),
            /*session_source*/ None,
        )
        .await
        .expect("spawn_agent should succeed");
    let _thread = harness
        .manager
        .get_thread(thread_id)
        .await
        .expect("thread should be registered");
    let expected = (
        thread_id,
        Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "spawned".to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        },
    );
    let captured = harness
        .manager
        .captured_ops()
        .into_iter()
        .find(|entry| *entry == expected);
    assert_eq!(captured, Some(expected));
}

#[tokio::test]
async fn spawn_agent_can_fork_parent_thread_history_with_sanitized_items() {
    let harness = AgentControlHarness::new().await;
    let mut parent_config = harness.config.clone();
    let _ = parent_config.features.enable(Feature::MultiAgentV2);
    parent_config.multi_agent_v2.root_agent_usage_hint_text =
        Some("Parent root guidance.".to_string());
    parent_config.multi_agent_v2.subagent_usage_hint_text =
        Some("Parent subagent guidance.".to_string());
    let mut child_config = harness.config.clone();
    let _ = child_config.features.enable(Feature::MultiAgentV2);
    child_config.multi_agent_v2.root_agent_usage_hint_text =
        Some("Child root guidance.".to_string());
    child_config.multi_agent_v2.subagent_usage_hint_text =
        Some("Child subagent guidance.".to_string());
    let new_thread = harness
        .manager
        .start_thread(parent_config.clone())
        .await
        .expect("start parent thread");
    let parent_thread_id = new_thread.thread_id;
    let parent_thread = new_thread.thread;
    parent_thread
        .inject_user_message_without_turn("parent seed context".to_string())
        .await;
    let turn_context = parent_thread.codex.session.new_default_turn().await;
    let parent_spawn_call_id = "spawn-call-history".to_string();
    let trigger_message = InterAgentCommunication::new(
        AgentPath::root(),
        AgentPath::try_from("/root/worker").expect("agent path"),
        Vec::new(),
        "parent trigger message".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    );
    parent_thread
        .codex
        .session
        .record_conversation_items(
            turn_context.as_ref(),
            &[
                ResponseItem::Message {
                    id: None,
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Parent root guidance.".to_string(),
                    }],
                    phase: None,
                },
                ResponseItem::Message {
                    id: None,
                    role: "developer".to_string(),
                    content: vec![ContentItem::InputText {
                        text: "Parent subagent guidance.".to_string(),
                    }],
                    phase: None,
                },
                assistant_message("parent commentary", Some(MessagePhase::Commentary)),
                assistant_message("parent final answer", Some(MessagePhase::FinalAnswer)),
                assistant_message("parent unknown phase", /*phase*/ None),
                ResponseItem::Reasoning {
                    id: "parent-reasoning".to_string(),
                    summary: Vec::new(),
                    content: None,
                    encrypted_content: None,
                },
                ResponseItem::InterAgentCommunication {
                    id: None,
                    communication: trigger_message,
                },
                spawn_agent_call(&parent_spawn_call_id),
            ],
        )
        .await;
    parent_thread
        .codex
        .session
        .ensure_rollout_materialized()
        .await;
    parent_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("parent rollout should flush");

    let child_thread_id = harness
        .control
        .spawn_agent_with_metadata(
            child_config,
            text_input("child task"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            })),
            SpawnAgentOptions {
                fork_parent_spawn_call_id: Some(parent_spawn_call_id.clone()),
                fork_mode: Some(SpawnAgentForkMode::FullHistory),
                ..Default::default()
            },
        )
        .await
        .expect("forked spawn should succeed")
        .thread_id;

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should be registered");
    assert_ne!(child_thread_id, parent_thread_id);
    let history = child_thread.codex.session.clone_history().await;
    let expected_history = [
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "parent seed context".to_string(),
            }],
            phase: None,
        },
        assistant_message("parent final answer", Some(MessagePhase::FinalAnswer)),
    ];
    assert_eq!(
        history.raw_items(),
        &expected_history,
        "forked child history should keep only parent user messages and assistant final answers"
    );

    let expected = (
        child_thread_id,
        Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "child task".to_string(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        },
    );
    let captured = harness
        .manager
        .captured_ops()
        .into_iter()
        .find(|entry| *entry == expected);
    assert_eq!(captured, Some(expected));

    let _ = harness
        .control
        .shutdown_live_agent(child_thread_id)
        .await
        .expect("child shutdown should submit");
    let _ = parent_thread
        .submit(Op::Shutdown {})
        .await
        .expect("parent shutdown should submit");
}

#[tokio::test]
async fn spawn_agent_fork_flushes_parent_rollout_before_loading_history() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, parent_thread) = harness.start_thread().await;
    let turn_context = parent_thread.codex.session.new_default_turn().await;
    let parent_spawn_call_id = "spawn-call-unflushed".to_string();
    parent_thread
        .codex
        .session
        .record_conversation_items(
            turn_context.as_ref(),
            &[
                assistant_message("unflushed final answer", Some(MessagePhase::FinalAnswer)),
                spawn_agent_call(&parent_spawn_call_id),
            ],
        )
        .await;

    let child_thread_id = harness
        .control
        .spawn_agent_with_metadata(
            harness.config.clone(),
            text_input("child task"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            })),
            SpawnAgentOptions {
                fork_parent_spawn_call_id: Some(parent_spawn_call_id.clone()),
                fork_mode: Some(SpawnAgentForkMode::FullHistory),
                ..Default::default()
            },
        )
        .await
        .expect("forked spawn should flush parent rollout before loading history")
        .thread_id;

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should be registered");
    let history = child_thread.codex.session.clone_history().await;
    assert!(
        history_contains_text(history.raw_items(), "unflushed final answer"),
        "forked child history should include unflushed assistant final answers after flushing the parent rollout"
    );

    let _ = harness
        .control
        .shutdown_live_agent(child_thread_id)
        .await
        .expect("child shutdown should submit");
    let _ = parent_thread
        .submit(Op::Shutdown {})
        .await
        .expect("parent shutdown should submit");
}

#[tokio::test]
async fn spawn_agent_fork_last_n_turns_keeps_only_recent_turns() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, parent_thread) = harness.start_thread().await;

    parent_thread
        .inject_user_message_without_turn("old parent context".to_string())
        .await;
    let queued_communication = InterAgentCommunication::new(
        AgentPath::root(),
        AgentPath::try_from("/root/worker").expect("agent path"),
        Vec::new(),
        "queued message".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    let queued_turn_context = parent_thread.codex.session.new_default_turn().await;
    parent_thread
        .codex
        .session
        .record_conversation_items(
            queued_turn_context.as_ref(),
            &[ResponseItem::InterAgentCommunication {
                id: None,
                communication: queued_communication,
            }],
        )
        .await;

    let triggered_communication = InterAgentCommunication::new(
        AgentPath::root(),
        AgentPath::try_from("/root/worker").expect("agent path"),
        Vec::new(),
        "triggered context".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    );
    let triggered_turn_context = parent_thread.codex.session.new_default_turn().await;
    parent_thread
        .codex
        .session
        .record_conversation_items(
            triggered_turn_context.as_ref(),
            &[ResponseItem::InterAgentCommunication {
                id: None,
                communication: triggered_communication,
            }],
        )
        .await;
    parent_thread
        .inject_user_message_without_turn("current parent task".to_string())
        .await;
    let spawn_turn_context = parent_thread.codex.session.new_default_turn().await;
    let parent_spawn_call_id = "spawn-call-last-n".to_string();
    parent_thread
        .codex
        .session
        .record_conversation_items(
            spawn_turn_context.as_ref(),
            &[spawn_agent_call(&parent_spawn_call_id)],
        )
        .await;
    parent_thread
        .codex
        .session
        .ensure_rollout_materialized()
        .await;
    parent_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("parent rollout should flush");

    let child_thread_id = harness
        .control
        .spawn_agent_with_metadata(
            harness.config.clone(),
            text_input("child task"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            })),
            SpawnAgentOptions {
                fork_parent_spawn_call_id: Some(parent_spawn_call_id.clone()),
                fork_mode: Some(SpawnAgentForkMode::LastNTurns(2)),
                ..Default::default()
            },
        )
        .await
        .expect("forked spawn should keep only the last two turns")
        .thread_id;

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should be registered");
    let history = child_thread.codex.session.clone_history().await;

    assert!(
        !history_contains_text(history.raw_items(), "old parent context"),
        "forked child history should drop parent context outside the requested last-N turn window"
    );
    assert!(
        !history_contains_text(history.raw_items(), "queued message"),
        "forked child history should drop queued inter-agent messages outside the requested last-N turn window"
    );
    assert!(
        !history_contains_text(history.raw_items(), "triggered context"),
        "forked child history should filter assistant inter-agent messages even when they fall inside the requested last-N turn window"
    );
    assert!(
        history_contains_text(history.raw_items(), "current parent task"),
        "forked child history should keep the parent user message from the requested last-N turn window"
    );

    let _ = harness
        .control
        .shutdown_live_agent(child_thread_id)
        .await
        .expect("child shutdown should submit");
    let _ = parent_thread
        .submit(Op::Shutdown {})
        .await
        .expect("parent shutdown should submit");
}

#[tokio::test]
async fn spawn_agent_respects_max_threads_limit() {
    let max_threads = 1usize;
    let (_home, config) = test_config_with_cli_overrides(vec![(
        "agents.max_threads".to_string(),
        TomlValue::Integer(max_threads as i64),
    )])
    .await;
    let manager = ThreadService::with_models_provider_and_home_for_tests(
        CodexAuth::from_api_key("dummy"),
        config.model_provider.clone(),
        crate::test_support::model_provider_factory_for_tests(),
        config.codex_home.to_path_buf(),
        std::sync::Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
    );
    let control = manager.agent_control();

    let _ = manager
        .start_thread(config.clone())
        .await
        .expect("start thread");

    let first_agent_id = control
        .spawn_agent(
            config.clone(),
            text_input("hello"),
            /*session_source*/ None,
        )
        .await
        .expect("spawn_agent should succeed");

    let err = control
        .spawn_agent(
            config,
            text_input("hello again"),
            /*session_source*/ None,
        )
        .await
        .expect_err("spawn_agent should respect max threads");
    let CodexErr::AgentLimitReached {
        max_threads: seen_max_threads,
    } = err
    else {
        panic!("expected CodexErr::AgentLimitReached");
    };
    assert_eq!(seen_max_threads, max_threads);

    let _ = control
        .shutdown_live_agent(first_agent_id)
        .await
        .expect("shutdown agent");
}

#[tokio::test]
async fn spawn_agent_releases_slot_after_shutdown() {
    let max_threads = 1usize;
    let (_home, config) = test_config_with_cli_overrides(vec![(
        "agents.max_threads".to_string(),
        TomlValue::Integer(max_threads as i64),
    )])
    .await;
    let manager = ThreadService::with_models_provider_and_home_for_tests(
        CodexAuth::from_api_key("dummy"),
        config.model_provider.clone(),
        crate::test_support::model_provider_factory_for_tests(),
        config.codex_home.to_path_buf(),
        std::sync::Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
    );
    let control = manager.agent_control();

    let first_agent_id = control
        .spawn_agent(
            config.clone(),
            text_input("hello"),
            /*session_source*/ None,
        )
        .await
        .expect("spawn_agent should succeed");
    let _ = control
        .shutdown_live_agent(first_agent_id)
        .await
        .expect("shutdown agent");

    let second_agent_id = control
        .spawn_agent(
            config.clone(),
            text_input("hello again"),
            /*session_source*/ None,
        )
        .await
        .expect("spawn_agent should succeed after shutdown");
    let _ = control
        .shutdown_live_agent(second_agent_id)
        .await
        .expect("shutdown agent");
}

#[tokio::test]
async fn spawn_agent_limit_shared_across_clones() {
    let max_threads = 1usize;
    let (_home, config) = test_config_with_cli_overrides(vec![(
        "agents.max_threads".to_string(),
        TomlValue::Integer(max_threads as i64),
    )])
    .await;
    let manager = ThreadService::with_models_provider_and_home_for_tests(
        CodexAuth::from_api_key("dummy"),
        config.model_provider.clone(),
        crate::test_support::model_provider_factory_for_tests(),
        config.codex_home.to_path_buf(),
        std::sync::Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
    );
    let control = manager.agent_control();
    let cloned = control.clone();

    let first_agent_id = cloned
        .spawn_agent(
            config.clone(),
            text_input("hello"),
            /*session_source*/ None,
        )
        .await
        .expect("spawn_agent should succeed");

    let err = control
        .spawn_agent(
            config,
            text_input("hello again"),
            /*session_source*/ None,
        )
        .await
        .expect_err("spawn_agent should respect shared guard");
    let CodexErr::AgentLimitReached { max_threads } = err else {
        panic!("expected CodexErr::AgentLimitReached");
    };
    assert_eq!(max_threads, 1);

    let _ = control
        .shutdown_live_agent(first_agent_id)
        .await
        .expect("shutdown agent");
}

#[tokio::test]
async fn resume_agent_respects_max_threads_limit() {
    let max_threads = 1usize;
    let (_home, config) = test_config_with_cli_overrides(vec![(
        "agents.max_threads".to_string(),
        TomlValue::Integer(max_threads as i64),
    )])
    .await;
    let manager = ThreadService::with_models_provider_and_home_for_tests(
        CodexAuth::from_api_key("dummy"),
        config.model_provider.clone(),
        crate::test_support::model_provider_factory_for_tests(),
        config.codex_home.to_path_buf(),
        std::sync::Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
    );
    let control = manager.agent_control();

    let resumable_id = control
        .spawn_agent(
            config.clone(),
            text_input("hello"),
            /*session_source*/ None,
        )
        .await
        .expect("spawn_agent should succeed");
    let _ = control
        .shutdown_live_agent(resumable_id)
        .await
        .expect("shutdown resumable thread");

    let active_id = control
        .spawn_agent(
            config.clone(),
            text_input("occupy"),
            /*session_source*/ None,
        )
        .await
        .expect("spawn_agent should succeed for active slot");

    let err = control
        .resume_agent_from_rollout(config, resumable_id, SessionSource::Exec)
        .await
        .expect_err("resume should respect max threads");
    let CodexErr::AgentLimitReached {
        max_threads: seen_max_threads,
    } = err
    else {
        panic!("expected CodexErr::AgentLimitReached");
    };
    assert_eq!(seen_max_threads, max_threads);

    let _ = control
        .shutdown_live_agent(active_id)
        .await
        .expect("shutdown active thread");
}

#[tokio::test]
async fn resume_agent_releases_slot_after_resume_failure() {
    let max_threads = 1usize;
    let (_home, config) = test_config_with_cli_overrides(vec![(
        "agents.max_threads".to_string(),
        TomlValue::Integer(max_threads as i64),
    )])
    .await;
    let manager = ThreadService::with_models_provider_and_home_for_tests(
        CodexAuth::from_api_key("dummy"),
        config.model_provider.clone(),
        crate::test_support::model_provider_factory_for_tests(),
        config.codex_home.to_path_buf(),
        std::sync::Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
    );
    let control = manager.agent_control();

    let _ = control
        .resume_agent_from_rollout(config.clone(), ThreadId::new(), SessionSource::Exec)
        .await
        .expect_err("resume should fail for missing rollout path");

    let resumed_id = control
        .spawn_agent(config, text_input("hello"), /*session_source*/ None)
        .await
        .expect("spawn should succeed after failed resume");
    let _ = control
        .shutdown_live_agent(resumed_id)
        .await
        .expect("shutdown resumed thread");
}

#[tokio::test]
async fn child_shutdown_does_not_notify_parent_history() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let _ = child_thread
        .submit(Op::Shutdown {})
        .await
        .expect("child shutdown should submit");

    assert!(
        no_subagent_notification(&parent_thread).await,
        "direct shutdown records child final status without simulating the child's post-turn finalization"
    );
}

#[tokio::test]
async fn multi_agent_v2_completion_ignores_dead_direct_parent() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root().join("worker_a").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let tester_path = worker_path.join("tester").expect("tester path");
    let tester_thread_id = harness
        .control
        .spawn_agent(
            config,
            text_input("hello tester"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: worker_thread_id,
                depth: 2,
                agent_path: Some(tester_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("tester spawn should succeed");
    harness
        .control
        .shutdown_live_agent(worker_thread_id)
        .await
        .expect("worker shutdown should succeed");

    let tester_thread = harness
        .manager
        .get_thread(tester_thread_id)
        .await
        .expect("tester thread should exist");
    let tester_turn = tester_thread.codex.session.new_default_turn().await;
    tester_thread
        .codex
        .session
        .send_event(
            tester_turn.as_ref(),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: tester_turn.sub_id.clone(),
                last_agent_message: Some("done".to_string()),
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        )
        .await;

    sleep(Duration::from_millis(100)).await;

    assert!(
        !harness
            .manager
            .captured_ops()
            .into_iter()
            .any(|(thread_id, op)| {
                thread_id == worker_thread_id
                    && matches!(
                        op,
                        Op::InterAgentCommunication { communication }
                            if communication.author == tester_path
                                && communication.recipient == worker_path
                                && communication.content == "done"
                    )
            })
    );

    let root_history_items = root_thread
        .codex
        .session
        .clone_history()
        .await
        .raw_items()
        .to_vec();
    assert!(!history_contains_inter_agent_communication(
        &root_history_items,
        &InterAgentCommunication::new(
            tester_path,
            AgentPath::root(),
            Vec::new(),
            "done".to_string(),
            protocol::protocol::InterAgentOperation::Unknown,
        )
    ));
    assert!(!has_subagent_notification(&root_history_items));
}

#[tokio::test]
async fn raw_final_status_does_not_notify_parent_with_child_completion() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root()
        .join("raw_status_worker")
        .expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            config,
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");
    worker_thread
        .codex
        .session
        .abort_all_tasks(TurnAbortReason::Interrupted)
        .await;
    sleep(Duration::from_millis(100)).await;
    let root_history_items = root_thread
        .codex
        .session
        .clone_history()
        .await
        .raw_items()
        .to_vec();
    assert!(
        !has_subagent_notification(&root_history_items),
        "interrupted setup should not notify parent before the raw final status"
    );
    let worker_turn = worker_thread.codex.session.new_default_turn().await;

    worker_thread
        .codex
        .session
        .send_event_raw(Event {
            id: worker_turn.sub_id.clone(),
            msg: EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: worker_turn.sub_id.clone(),
                last_agent_message: Some("done from raw status".to_string()),
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        })
        .await;

    assert!(
        no_subagent_notification(&root_thread).await,
        "raw status recording is not the child on_task_finished finalization path"
    );
}

#[tokio::test]
async fn multi_agent_v2_completion_waits_for_pending_mailbox_input() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            config,
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");
    worker_thread
        .codex
        .session
        .abort_all_tasks(TurnAbortReason::Replaced)
        .await;
    sleep(Duration::from_millis(100)).await;
    let queued_update = InterAgentCommunication::new(
        AgentPath::root(),
        worker_path.clone(),
        Vec::new(),
        "queued parent update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    worker_thread
        .codex
        .session
        .enqueue_mailbox_communication(queued_update)
        .await;
    assert!(worker_thread.codex.session.has_pending_input().await);
    let baseline_op_count = harness.manager.captured_ops().len();

    emit_turn_complete(&worker_thread, "done").await;
    sleep(Duration::from_millis(100)).await;
    let captured_ops = harness.manager.captured_ops();

    assert!(!captured_child_completion(
        &captured_ops[baseline_op_count..],
        root_thread_id,
        &worker_path,
        &AgentPath::root(),
    ));

    let listed_agents = harness
        .control
        .list_agents(root_thread_id, &SessionSource::Exec, None)
        .await;
    assert_eq!(
        listed_agents
            .expect("list agents should succeed")
            .into_iter()
            .find(|agent| agent.agent_name == worker_path.to_string())
            .expect("worker should be listed")
            .lifecycle_status,
        ThreadLifecycleStatus::completed(Some("done".to_string())),
    );
    let captured_ops = harness.manager.captured_ops();
    assert_eq!(
        count_captured_child_completions(
            &captured_ops[baseline_op_count..],
            root_thread_id,
            &worker_path,
            &AgentPath::root(),
        ),
        0,
        "status/list reads should not deliver child completion"
    );
}

#[tokio::test]
async fn child_spawn_does_not_register_completion_pending() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");

    harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello legacy worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("legacy worker spawn should succeed");

    assert!(no_subagent_notification(&root_thread).await);
}

#[tokio::test]
async fn list_agents_restores_completed_child_from_persisted_history_when_live_thread_is_gone() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");
    let state_db = harness
        .state_db
        .as_ref()
        .expect("sqlite state db should be available");

    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    emit_turn_complete(&worker_thread, "done").await;
    worker_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[worker_thread_id])
        .await;
    timeout(Duration::from_secs(5), async {
        loop {
            let metadata_ready = state_db
                .get_thread(worker_thread_id)
                .await
                .ok()
                .flatten()
                .is_some_and(|metadata| {
                    metadata.agent_path.as_deref() == Some(worker_path.as_str())
                });
            let edge_ready = state_db
                .list_thread_spawn_descendants_with_status(
                    root_thread_id,
                    DirectionalThreadSpawnEdgeStatus::Open,
                )
                .await
                .ok()
                .is_some_and(|descendants| descendants.contains(&worker_thread_id));
            if metadata_ready && edge_ready {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("worker metadata and history should persist before shutdown");

    let _ = harness
        .control
        .shutdown_live_agent(worker_thread_id)
        .await
        .expect("worker shutdown should succeed");
    assert_eq!(
        harness.control.get_status(worker_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness
            .control
            .persisted_final_agent_status(worker_thread_id)
            .await,
        Some(AgentStatus::Completed(Some("done".to_string()))),
    );

    let listed_agents = harness
        .control
        .list_agents(root_thread_id, &SessionSource::Exec, None)
        .await
        .expect("list agents should succeed");
    assert_eq!(
        listed_agents
            .into_iter()
            .find(|agent| agent.agent_name == worker_path.to_string())
            .expect("persisted worker should be listed")
            .lifecycle_status,
        ThreadLifecycleStatus::completed(Some("done".to_string())),
    );
}

#[tokio::test]
async fn list_agents_restores_completed_child_from_persisted_root_when_registry_is_empty() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(worker_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("worker".to_string()),
    });
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(worker_source.clone()),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    emit_turn_complete(&worker_thread, "done").await;
    worker_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[worker_thread_id])
        .await;

    let (_restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let restored_worker_source_without_path =
        SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: root_thread_id,
            depth: 1,
            agent_path: None,
            agent_nickname: None,
            agent_role: None,
        });
    let listed_agents = restarted_control
        .list_agents(worker_thread_id, &restored_worker_source_without_path, None)
        .await
        .expect("list agents should succeed from restored subagent source without path");

    assert_eq!(
        listed_agents
            .into_iter()
            .find(|agent| agent.agent_name == worker_path.to_string())
            .expect("persisted worker should be listed after registry loss")
            .lifecycle_status,
        ThreadLifecycleStatus::completed(Some("done".to_string())),
    );
}

#[tokio::test]
async fn restored_completed_child_path_resolves_and_receives_followup_after_registry_loss() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    persist_thread_for_tree_resume(&root_thread, "root persisted").await;
    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    emit_turn_complete(&worker_thread, "done").await;
    worker_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[worker_thread_id])
        .await;
    harness
        .control
        .shutdown_live_agent(worker_thread_id)
        .await
        .expect("worker shutdown should release the live path");
    assert!(
        harness.manager.get_thread(worker_thread_id).await.is_err(),
        "worker should be absent from the live registry"
    );
    let listed_agents = harness
        .control
        .list_agents(root_thread_id, &SessionSource::Exec, None)
        .await
        .expect("list agents should succeed from persisted tree");
    assert_eq!(
        listed_agents
            .iter()
            .find(|agent| agent.agent_name == worker_path.to_string())
            .expect("persisted worker should be listed")
            .lifecycle_status,
        ThreadLifecycleStatus::completed(Some("done".to_string())),
    );

    let resolved_thread_id = harness
        .control
        .resolve_agent_reference(
            root_thread_id,
            &SessionSource::Exec,
            Some(harness.config.clone()),
            worker_path.as_str(),
        )
        .await
        .expect("persisted worker path should resolve after registry loss");
    assert_eq!(resolved_thread_id, worker_thread_id);
    assert_eq!(
        harness
            .control
            .get_agent_metadata(worker_thread_id)
            .and_then(|metadata| metadata.agent_path),
        Some(worker_path.clone()),
        "resolver should re-register metadata needed by followup_task events",
    );

    let restored_worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("resolver should restore the original worker thread");
    let baseline_op_count = harness.manager.captured_ops().len();
    let listed_agents = harness
        .control
        .list_agents(root_thread_id, &SessionSource::Exec, None)
        .await
        .expect("list agents should succeed from live restored tree");
    assert!(
        listed_agents
            .iter()
            .any(|agent| agent.agent_name == worker_path.to_string()
                && agent.lifecycle_status
                    == ThreadLifecycleStatus::completed(Some("done".to_string())))
    );
    let captured_ops = harness.manager.captured_ops();
    assert_eq!(
        count_captured_child_completions(
            &captured_ops[baseline_op_count..],
            root_thread_id,
            &worker_path,
            &AgentPath::root(),
        ),
        0,
        "status/list inspection must not deliver an old completion envelope",
    );

    let communication = InterAgentCommunication::new(
        AgentPath::root(),
        worker_path.clone(),
        Vec::new(),
        "followup after restart".to_string(),
        protocol::protocol::InterAgentOperation::FollowupTask,
    )
    .with_trigger_turn(false);
    let submission_id = harness
        .control
        .send_inter_agent_communication(resolved_thread_id, communication.clone())
        .await
        .expect("followup should route to restored worker");
    assert!(!submission_id.is_empty());

    let expected = (
        worker_thread_id,
        Op::InterAgentCommunication {
            communication: communication.clone(),
        },
    );
    let captured = harness
        .manager
        .captured_ops()
        .into_iter()
        .find(|entry| *entry == expected);
    assert_eq!(captured, Some(expected));

    timeout(Duration::from_secs(5), async {
        loop {
            if restored_worker_thread
                .codex
                .session
                .has_pending_input()
                .await
            {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("restored child should receive followup pending input");
    assert_eq!(
        restored_worker_thread
            .codex
            .session
            .get_pending_input()
            .await,
        vec![PendingInputItem::from(communication)],
        "restored child should receive and consume the followup before its next completion",
    );
    emit_turn_complete(&restored_worker_thread, "new done").await;
    harness
        .manager
        .maybe_notify_parent_of_final_status(worker_thread_id)
        .await;
    let captured_ops = harness.manager.captured_ops();
    assert_eq!(
        count_captured_child_completions(
            &captured_ops[baseline_op_count..],
            root_thread_id,
            &worker_path,
            &AgentPath::root(),
        ),
        1,
        "a real post-followup completion should still be delivered once",
    );
}

#[tokio::test]
async fn tree_resume_restores_completed_child_status_for_parent_wait_child() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    persist_thread_for_tree_resume(&root_thread, "root persisted").await;
    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    emit_turn_complete(&worker_thread, "done").await;
    worker_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[worker_thread_id])
        .await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    restarted_control
        .resume_agent_from_rollout(harness.config.clone(), root_thread_id, SessionSource::Exec)
        .await
        .expect("root tree should resume from persisted rollout");

    let restored_root_thread = restarted_manager
        .get_thread(root_thread_id)
        .await
        .expect("root thread should be restored");
    assert_eq!(
        restarted_control
            .normalized_thread_lifecycle(worker_thread_id)
            .await,
        ThreadLifecycleStatus::completed(Some("done".to_string())),
        "restored completed child must not stay at placeholder PendingInit",
    );
    assert!(
        !restarted_control
            .agent_thread_is_active(worker_thread_id)
            .await,
        "restored completed child should not keep the parent waiting",
    );
    assert_eq!(
        restored_root_thread
            .codex
            .session
            .thread_post_turn_state()
            .await,
        ThreadPostTurnState::ThreadCompletion
    );

    let listed_agents = restarted_control
        .list_agents(root_thread_id, &SessionSource::Exec, None)
        .await
        .expect("list agents should succeed from restored live tree");
    assert_eq!(
        listed_agents
            .into_iter()
            .find(|agent| agent.agent_name == worker_path.to_string())
            .expect("restored worker should be listed")
            .lifecycle_status,
        ThreadLifecycleStatus::completed(Some("done".to_string())),
    );
}

#[tokio::test]
async fn followup_task_by_thread_id_restores_persisted_child_after_restart() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    persist_thread_for_tree_resume(&root_thread, "root persisted").await;
    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    emit_turn_complete(&worker_thread, "done").await;
    worker_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[worker_thread_id])
        .await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    Box::pin(restarted_control.resume_single_agent_from_rollout(
        harness.config.clone(),
        root_thread_id,
        SessionSource::Exec,
    ))
    .await
    .expect("root should resume without manually resuming the child");
    assert!(
        restarted_manager
            .get_thread(worker_thread_id)
            .await
            .is_err(),
        "worker should not be live before followup_task resolves it",
    );

    let restored_root_thread = restarted_manager
        .get_thread(root_thread_id)
        .await
        .expect("root thread should be live after restart");
    let root_turn = restored_root_thread.codex.session.new_default_turn().await;
    crate::agent::multi_agent::followup_task_tool(
        Arc::clone(&restored_root_thread.codex.session),
        root_turn,
        "call-followup".to_string(),
        worker_thread_id.to_string(),
        "followup after restart".to_string(),
    )
    .await
    .expect("followup_task should restore and deliver to persisted child thread id");

    let restored_worker_thread = restarted_manager
        .get_thread(worker_thread_id)
        .await
        .expect("followup_task should restore the worker thread");
    let expected_communication = InterAgentCommunication::new(
        AgentPath::root(),
        worker_path.clone(),
        Vec::new(),
        "followup after restart".to_string(),
        protocol::protocol::InterAgentOperation::FollowupTask,
    )
    .with_thread_ids(root_thread_id, worker_thread_id)
    .with_trigger_turn(true);
    let expected = (
        worker_thread_id,
        Op::InterAgentCommunication {
            communication: expected_communication.clone(),
        },
    );
    assert!(
        restarted_manager
            .captured_ops()
            .into_iter()
            .any(|entry| entry == expected),
        "followup_task should submit the restored child through the normal inter-agent path",
    );

    timeout(Duration::from_secs(5), async {
        loop {
            if restored_worker_thread
                .codex
                .session
                .has_pending_input()
                .await
            {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("restored child should receive followup pending input");
    assert_eq!(
        restored_worker_thread
            .codex
            .session
            .get_pending_input()
            .await,
        vec![PendingInputItem::from(expected_communication)],
    );
}

#[tokio::test]
async fn followup_task_by_path_does_not_restore_archived_persisted_child() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    persist_thread_for_tree_resume(&root_thread, "root persisted").await;
    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    emit_turn_complete(&worker_thread, "done").await;
    worker_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[worker_thread_id])
        .await;
    harness
        .control
        .shutdown_live_agent(worker_thread_id)
        .await
        .expect("worker shutdown should release the live path");
    archive_thread_for_test(&harness, worker_thread_id).await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    Box::pin(restarted_control.resume_single_agent_from_rollout(
        harness.config.clone(),
        root_thread_id,
        SessionSource::Exec,
    ))
    .await
    .expect("root should resume without restoring the archived child");
    let restored_root_thread = restarted_manager
        .get_thread(root_thread_id)
        .await
        .expect("root thread should be live after restart");
    let root_turn = restored_root_thread.codex.session.new_default_turn().await;
    let err = crate::agent::multi_agent::followup_task_tool(
        Arc::clone(&restored_root_thread.codex.session),
        root_turn,
        "call-followup".to_string(),
        worker_path.as_str().to_string(),
        "followup after restart".to_string(),
    )
    .await
    .expect_err("followup_task should not restore an archived persisted child by path");

    assert!(
        err.to_string().contains("not found") || err.to_string().contains("unsupported"),
        "unexpected error: {err}",
    );
    assert!(
        restarted_manager
            .get_thread(worker_thread_id)
            .await
            .is_err(),
        "archived child should remain non-live after failed path followup",
    );
}

#[tokio::test]
async fn followup_task_by_thread_id_does_not_restore_archived_persisted_child() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    persist_thread_for_tree_resume(&root_thread, "root persisted").await;
    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    emit_turn_complete(&worker_thread, "done").await;
    worker_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[worker_thread_id])
        .await;
    harness
        .control
        .shutdown_live_agent(worker_thread_id)
        .await
        .expect("worker shutdown should release the live path");
    archive_thread_for_test(&harness, worker_thread_id).await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    Box::pin(restarted_control.resume_single_agent_from_rollout(
        harness.config.clone(),
        root_thread_id,
        SessionSource::Exec,
    ))
    .await
    .expect("root should resume without restoring the archived child");
    let restored_root_thread = restarted_manager
        .get_thread(root_thread_id)
        .await
        .expect("root thread should be live after restart");
    let root_turn = restored_root_thread.codex.session.new_default_turn().await;
    let err = crate::agent::multi_agent::followup_task_tool(
        Arc::clone(&restored_root_thread.codex.session),
        root_turn,
        "call-followup".to_string(),
        worker_thread_id.to_string(),
        "followup after restart".to_string(),
    )
    .await
    .expect_err("followup_task should not restore an archived persisted child by thread id");

    assert!(
        err.to_string().contains("archived"),
        "unexpected error: {err}",
    );
    assert!(
        restarted_manager
            .get_thread(worker_thread_id)
            .await
            .is_err(),
        "archived child should remain non-live after failed thread-id followup",
    );
}

#[tokio::test]
async fn followup_task_by_path_does_not_restore_deleted_metadata_child() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    persist_thread_for_tree_resume(&root_thread, "root persisted").await;
    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[worker_thread_id])
        .await;
    harness
        .control
        .shutdown_live_agent(worker_thread_id)
        .await
        .expect("worker shutdown should release the live path");
    delete_thread_metadata_for_test(&harness, worker_thread_id).await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    Box::pin(restarted_control.resume_single_agent_from_rollout(
        harness.config.clone(),
        root_thread_id,
        SessionSource::Exec,
    ))
    .await
    .expect("root should resume without restoring the deleted child");
    let restored_root_thread = restarted_manager
        .get_thread(root_thread_id)
        .await
        .expect("root thread should be live after restart");
    let root_turn = restored_root_thread.codex.session.new_default_turn().await;
    let err = crate::agent::multi_agent::followup_task_tool(
        Arc::clone(&restored_root_thread.codex.session),
        root_turn,
        "call-followup".to_string(),
        worker_path.as_str().to_string(),
        "followup after restart".to_string(),
    )
    .await
    .expect_err("followup_task should not restore a deleted metadata child by path");

    assert!(
        err.to_string().contains("not found") || err.to_string().contains("unsupported"),
        "unexpected error: {err}",
    );
    assert!(
        restarted_manager
            .get_thread(worker_thread_id)
            .await
            .is_err(),
        "deleted child should remain non-live after failed path followup",
    );
}

#[tokio::test]
async fn followup_task_by_thread_id_does_not_restore_deleted_metadata_child() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    persist_thread_for_tree_resume(&root_thread, "root persisted").await;
    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[worker_thread_id])
        .await;
    harness
        .control
        .shutdown_live_agent(worker_thread_id)
        .await
        .expect("worker shutdown should release the live path");
    delete_thread_metadata_for_test(&harness, worker_thread_id).await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    Box::pin(restarted_control.resume_single_agent_from_rollout(
        harness.config.clone(),
        root_thread_id,
        SessionSource::Exec,
    ))
    .await
    .expect("root should resume without restoring the deleted child");
    let restored_root_thread = restarted_manager
        .get_thread(root_thread_id)
        .await
        .expect("root thread should be live after restart");
    let root_turn = restored_root_thread.codex.session.new_default_turn().await;
    let err = crate::agent::multi_agent::followup_task_tool(
        Arc::clone(&restored_root_thread.codex.session),
        root_turn,
        "call-followup".to_string(),
        worker_thread_id.to_string(),
        "followup after restart".to_string(),
    )
    .await
    .expect_err("followup_task should not restore a deleted metadata child by thread id");

    assert!(
        err.to_string().contains("missing persisted agent metadata"),
        "unexpected error: {err}",
    );
    assert!(
        restarted_manager
            .get_thread(worker_thread_id)
            .await
            .is_err(),
        "deleted child should remain non-live after failed thread-id followup",
    );
}

#[tokio::test]
async fn followup_task_by_path_restores_latest_completed_generation_after_stale_duplicate() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let first_worker_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello first worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("first worker spawn should succeed");
    let first_worker = harness
        .manager
        .get_thread(first_worker_id)
        .await
        .expect("first worker thread should exist");
    persist_thread_for_tree_resume(&root_thread, "root persisted").await;
    persist_thread_for_tree_resume(&first_worker, "first worker persisted").await;
    emit_turn_complete(&first_worker, "old done").await;
    first_worker
        .codex
        .session
        .flush_rollout()
        .await
        .expect("first worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[first_worker_id]).await;
    harness
        .control
        .shutdown_live_agent(first_worker_id)
        .await
        .expect("first worker shutdown should release the live path");

    sleep(Duration::from_millis(10)).await;
    let second_worker_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello second worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("second worker spawn should succeed");
    let second_worker = harness
        .manager
        .get_thread(second_worker_id)
        .await
        .expect("second worker thread should exist");
    persist_thread_for_tree_resume(&second_worker, "second worker persisted").await;
    emit_turn_complete(&second_worker, "new done").await;
    second_worker
        .codex
        .session
        .flush_rollout()
        .await
        .expect("second worker rollout should flush");

    let state_db = harness
        .state_db
        .as_ref()
        .expect("sqlite state db should be available");
    state_db
        .set_thread_spawn_edge_status(first_worker_id, DirectionalThreadSpawnEdgeStatus::Open)
        .await
        .expect("test should simulate a stale open edge from an older server");

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    Box::pin(restarted_control.resume_agent_from_rollout(
        harness.config.clone(),
        root_thread_id,
        SessionSource::Exec,
    ))
    .await
    .expect("full tree resume should skip the stale duplicate generation");

    let restored_root_thread = restarted_manager
        .get_thread(root_thread_id)
        .await
        .expect("root thread should be live after restart");
    assert!(
        restarted_manager.get_thread(first_worker_id).await.is_err(),
        "full tree resume should not restore the stale duplicate generation",
    );
    assert!(
        restarted_manager.get_thread(second_worker_id).await.is_ok(),
        "full tree resume should restore the latest generation",
    );
    let root_turn = restored_root_thread.codex.session.new_default_turn().await;
    crate::agent::multi_agent::followup_task_tool(
        Arc::clone(&restored_root_thread.codex.session),
        root_turn,
        "call-followup".to_string(),
        worker_path.as_str().to_string(),
        "followup after restart".to_string(),
    )
    .await
    .expect("followup_task should select the latest completed generation");

    let restored_second_worker = restarted_manager
        .get_thread(second_worker_id)
        .await
        .expect("latest generation should remain live for followup_task");
    let expected_communication = InterAgentCommunication::new(
        AgentPath::root(),
        worker_path.clone(),
        Vec::new(),
        "followup after restart".to_string(),
        protocol::protocol::InterAgentOperation::FollowupTask,
    )
    .with_thread_ids(root_thread_id, second_worker_id)
    .with_trigger_turn(true);
    let expected = (
        second_worker_id,
        Op::InterAgentCommunication {
            communication: expected_communication.clone(),
        },
    );
    assert!(
        restarted_manager
            .captured_ops()
            .into_iter()
            .any(|entry| entry == expected),
        "followup_task should submit to the latest generation through the normal inter-agent path",
    );
    assert_eq!(
        restored_second_worker
            .codex
            .session
            .get_pending_input()
            .await,
        vec![PendingInputItem::from(expected_communication)],
    );
}

#[tokio::test]
async fn followup_task_by_path_ignores_archived_old_generation() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let archived_worker_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello archived worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("archived worker spawn should succeed");
    let archived_worker = harness
        .manager
        .get_thread(archived_worker_id)
        .await
        .expect("archived worker thread should exist");
    persist_thread_for_tree_resume(&root_thread, "root persisted").await;
    persist_thread_for_tree_resume(&archived_worker, "archived worker persisted").await;
    emit_turn_complete(&archived_worker, "old done").await;
    archived_worker
        .codex
        .session
        .flush_rollout()
        .await
        .expect("archived worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[archived_worker_id])
        .await;
    harness
        .control
        .shutdown_live_agent(archived_worker_id)
        .await
        .expect("archived worker shutdown should release the live path");
    archive_thread_for_test(&harness, archived_worker_id).await;

    let current_worker_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello current worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("current worker spawn should succeed");
    let current_worker = harness
        .manager
        .get_thread(current_worker_id)
        .await
        .expect("current worker thread should exist");
    persist_thread_for_tree_resume(&current_worker, "current worker persisted").await;
    emit_turn_complete(&current_worker, "new done").await;
    current_worker
        .codex
        .session
        .flush_rollout()
        .await
        .expect("current worker rollout should flush");

    let state_db = harness
        .state_db
        .as_ref()
        .expect("sqlite state db should be available");
    state_db
        .set_thread_spawn_edge_status(archived_worker_id, DirectionalThreadSpawnEdgeStatus::Open)
        .await
        .expect("test should simulate a stale open edge for archived generation");

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    Box::pin(restarted_control.resume_agent_from_rollout(
        harness.config.clone(),
        root_thread_id,
        SessionSource::Exec,
    ))
    .await
    .expect("full tree resume should skip archived old generation");

    let restored_root_thread = restarted_manager
        .get_thread(root_thread_id)
        .await
        .expect("root thread should be live after restart");
    assert!(
        restarted_manager
            .get_thread(archived_worker_id)
            .await
            .is_err(),
        "full tree resume should not restore archived generation",
    );
    assert!(
        restarted_manager
            .get_thread(current_worker_id)
            .await
            .is_ok(),
        "full tree resume should restore current non-archived generation",
    );
    let root_turn = restored_root_thread.codex.session.new_default_turn().await;
    crate::agent::multi_agent::followup_task_tool(
        Arc::clone(&restored_root_thread.codex.session),
        root_turn,
        "call-followup".to_string(),
        worker_path.as_str().to_string(),
        "followup after restart".to_string(),
    )
    .await
    .expect("followup_task should select the current non-archived generation");

    let expected_communication = InterAgentCommunication::new(
        AgentPath::root(),
        worker_path.clone(),
        Vec::new(),
        "followup after restart".to_string(),
        protocol::protocol::InterAgentOperation::FollowupTask,
    )
    .with_thread_ids(root_thread_id, current_worker_id)
    .with_trigger_turn(true);
    let expected = (
        current_worker_id,
        Op::InterAgentCommunication {
            communication: expected_communication.clone(),
        },
    );
    assert!(
        restarted_manager
            .captured_ops()
            .into_iter()
            .any(|entry| entry == expected),
        "followup_task should submit to the current generation",
    );
}

#[tokio::test]
async fn restored_agent_path_resolution_rejects_ambiguous_persisted_duplicates() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let first_worker_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello first worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("first worker spawn should succeed");
    let first_worker = harness
        .manager
        .get_thread(first_worker_id)
        .await
        .expect("first worker thread should exist");
    persist_thread_for_tree_resume(&first_worker, "first worker persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[first_worker_id]).await;
    harness
        .control
        .shutdown_live_agent(first_worker_id)
        .await
        .expect("first worker shutdown should release the live path");

    let second_worker_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello second worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("second worker spawn should succeed");
    let second_worker = harness
        .manager
        .get_thread(second_worker_id)
        .await
        .expect("second worker thread should exist");
    persist_thread_for_tree_resume(&second_worker, "second worker persisted").await;
    let state_db = harness
        .state_db
        .as_ref()
        .expect("sqlite state db should be available");
    state_db
        .set_thread_spawn_edge_status(first_worker_id, DirectionalThreadSpawnEdgeStatus::Open)
        .await
        .expect("test should simulate two effective open duplicate edges");
    timeout(Duration::from_secs(5), async {
        loop {
            let descendants = state_db
                .list_thread_spawn_descendants_with_status(
                    root_thread_id,
                    DirectionalThreadSpawnEdgeStatus::Open,
                )
                .await
                .expect("persisted descendants should load");
            if descendants.contains(&first_worker_id) && descendants.contains(&second_worker_id) {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("both duplicate worker edges should persist");

    let (_restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let err = restarted_control
        .resolve_agent_reference(
            root_thread_id,
            &SessionSource::Exec,
            Some(harness.config.clone()),
            worker_path.as_str(),
        )
        .await
        .expect_err("duplicate persisted path should be rejected");
    assert!(
        err.to_string().contains("ambiguous"),
        "unexpected error: {err}",
    );
}

#[tokio::test]
async fn completed_agent_path_can_still_receive_followup_while_registered() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    emit_turn_complete(&worker_thread, "done").await;
    sleep(Duration::from_millis(100)).await;

    let resolved_thread_id = harness
        .control
        .resolve_agent_reference(
            root_thread_id,
            &SessionSource::Exec,
            Some(harness.config.clone()),
            worker_path.as_str(),
        )
        .await
        .expect("completed worker path should still resolve");
    assert_eq!(resolved_thread_id, worker_thread_id);

    let communication = InterAgentCommunication::new(
        AgentPath::root(),
        worker_path.clone(),
        Vec::new(),
        "followup after completion".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    );
    let submission_id = harness
        .control
        .send_inter_agent_communication(resolved_thread_id, communication.clone())
        .await
        .expect("followup should succeed for completed worker");
    assert!(!submission_id.is_empty());

    let expected = (
        worker_thread_id,
        Op::InterAgentCommunication {
            communication: communication.clone(),
        },
    );
    let captured = harness
        .manager
        .captured_ops()
        .into_iter()
        .find(|entry| *entry == expected);
    assert_eq!(captured, Some(expected));

    timeout(Duration::from_secs(5), async {
        loop {
            if worker_thread.codex.session.has_pending_input().await {
                break;
            }
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("completed worker should receive pending input");
    assert_eq!(
        worker_thread.codex.session.get_pending_input().await,
        vec![PendingInputItem::from(communication)],
    );
}

#[tokio::test]
async fn followup_task_to_completed_child_does_not_emit_old_completion() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            config,
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");
    emit_turn_complete(&worker_thread, "done").await;
    let baseline_op_count = harness.manager.captured_ops().len();

    let turn = root_thread.codex.session.new_default_turn().await;
    crate::agent::multi_agent::followup_task_tool(
        Arc::clone(&root_thread.codex.session),
        turn,
        "followup-call".to_string(),
        worker_path.to_string(),
        "please continue".to_string(),
    )
    .await
    .expect("followup_task should enqueue input for completed child");

    let captured_ops = harness.manager.captured_ops();
    assert_eq!(
        count_captured_child_completions(
            &captured_ops[baseline_op_count..],
            root_thread_id,
            &worker_path,
            &AgentPath::root(),
        ),
        0,
        "followup_task and its status read must not deliver an old completion",
    );
    assert!(
        worker_thread.codex.session.has_pending_input().await,
        "followup_task should still enqueue pending input for the child"
    );
}

#[tokio::test]
async fn multi_agent_v2_completion_allows_active_event_subscription() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            config,
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");
    worker_thread
        .codex
        .session
        .abort_all_tasks(TurnAbortReason::Replaced)
        .await;
    sleep(Duration::from_millis(100)).await;
    harness
        .manager
        .active_event_subscriptions()
        .set_active_count(worker_thread_id, 1);
    let baseline_op_count = harness.manager.captured_ops().len();

    let worker_turn = worker_thread.codex.session.new_default_turn().await;
    worker_thread
        .codex
        .session
        .send_event(
            worker_turn.as_ref(),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: worker_turn.sub_id.clone(),
                last_agent_message: Some("done".to_string()),
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        )
        .await;
    *worker_thread.codex.session.active_turn.lock().await = None;
    harness
        .manager
        .maybe_notify_parent_of_final_status(worker_thread_id)
        .await;
    let captured_ops = harness.manager.captured_ops();

    assert!(captured_child_completion(
        &captured_ops[baseline_op_count..],
        root_thread_id,
        &worker_path,
        &AgentPath::root(),
    ));
}

#[tokio::test]
async fn goal_post_turn_state_continues_despite_live_direct_child() {
    let harness = AgentControlHarness::new().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::Goals);
    let parent = harness
        .manager
        .start_thread(config)
        .await
        .expect("parent thread should start");
    let parent_thread_id = parent.thread_id;
    let parent_thread = parent.thread;
    let (_child_thread_id, child_thread) = harness.start_thread().await;
    replace_thread_goal(
        harness.state_db.as_ref().expect("state db should exist"),
        parent_thread_id,
        StateThreadGoalStatus::Active,
    )
    .await;
    emit_turn_complete(&parent_thread, "parent turn done").await;
    let goal_id = harness
        .state_db
        .as_ref()
        .expect("state db should exist")
        .get_thread_goal(parent_thread_id)
        .await
        .expect("goal query should succeed")
        .expect("goal should exist")
        .goal_id;

    assert_eq!(
        parent_thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::GoContextContinuation {
            goal_id: goal_id.clone(),
        }
    );

    emit_turn_complete(&child_thread, "child done").await;
    assert_eq!(
        parent_thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::GoContextContinuation {
            goal_id: goal_id.clone(),
        }
    );

    assert_eq!(
        parent_thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::GoContextContinuation { goal_id }
    );
}

#[tokio::test]
async fn post_turn_state_waits_for_active_direct_child_without_active_goal() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    harness
        .control
        .spawn_agent(
            config,
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    emit_turn_complete(&root_thread, "parent turn done").await;
    harness
        .manager
        .active_event_subscriptions()
        .set_active_count(root_thread_id, 1);

    assert_eq!(
        root_thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitChild)
    );
}

#[tokio::test]
async fn post_turn_state_stops_waiting_after_child_completion_is_consumed() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            config,
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    emit_turn_complete(&root_thread, "parent turn done").await;
    assert_eq!(
        root_thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitChild)
    );

    harness
        .manager
        .active_event_subscriptions()
        .set_active_count(worker_thread_id, 1);
    emit_turn_complete(&worker_thread, "child done").await;
    let completion = InterAgentCommunication::new(
        worker_path.clone(),
        AgentPath::root(),
        Vec::new(),
        "child done".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(true)
    .with_thread_ids(worker_thread_id, root_thread_id)
    .with_status(AgentStatus::Completed(Some("child done".to_string())));
    root_thread
        .codex
        .session
        .enqueue_mailbox_communication(completion)
        .await;
    assert!(
        root_thread.codex.session.has_pending_input().await,
        "parent should receive child completion"
    );

    let pending_input = root_thread.codex.session.get_pending_input().await;
    assert!(
        pending_input.iter().any(|item| {
            matches!(
                item,
                PendingInputItem::InterAgentCommunication(communication)
                    if communication.author == worker_path
                        && communication.operation
                            == protocol::protocol::InterAgentOperation::ChildCompletion
            )
        }),
        "parent should consume child completion input"
    );
    assert!(
        !harness
            .control
            .agent_thread_is_active(worker_thread_id)
            .await,
        "final child with stale event subscription must not remain active"
    );
    assert_eq!(
        root_thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::ThreadCompletion
    );
}

#[tokio::test]
async fn inactive_child_completion_input_does_not_trigger_wait_child() {
    let harness = AgentControlHarness::new().await;
    let parent = harness
        .manager
        .start_thread(harness.config.clone())
        .await
        .expect("parent thread should start");
    let parent_thread = parent.thread;
    emit_turn_complete(&parent_thread, "parent turn done").await;
    assert_eq!(
        parent_thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::ThreadCompletion
    );
}

#[tokio::test]
async fn goal_post_turn_state_continues_despite_active_event_subscription() {
    let harness = AgentControlHarness::new().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::Goals);
    let thread = harness
        .manager
        .start_thread(config)
        .await
        .expect("thread should start");
    replace_thread_goal(
        harness.state_db.as_ref().expect("state db should exist"),
        thread.thread_id,
        StateThreadGoalStatus::Active,
    )
    .await;
    emit_turn_complete(&thread.thread, "turn done").await;
    harness
        .manager
        .active_event_subscriptions()
        .set_active_count(thread.thread_id, 1);

    assert_eq!(
        thread.thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::GoContextContinuation {
            goal_id: harness
                .state_db
                .as_ref()
                .expect("state db should exist")
                .get_thread_goal(thread.thread_id)
                .await
                .expect("goal query should succeed")
                .expect("goal should exist")
                .goal_id,
        }
    );
}

#[tokio::test]
async fn post_turn_state_reports_active_event_subscription_without_active_goal() {
    let harness = AgentControlHarness::new().await;
    let thread = harness
        .manager
        .start_thread(harness.config.clone())
        .await
        .expect("thread should start");
    emit_turn_complete(&thread.thread, "turn done").await;
    harness
        .manager
        .active_event_subscriptions()
        .set_active_count(thread.thread_id, 1);

    assert_eq!(
        thread.thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::ThreadIdle(ThreadIdleReason::WaitEventSubscription)
    );
}

#[tokio::test]
async fn multi_agent_v2_completion_waits_for_next_turn_after_active_goal_continuation() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let _ = config.features.enable(Feature::Goals);
    let worker_path = AgentPath::root().join("goal_worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            config,
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");
    worker_thread
        .codex
        .session
        .abort_all_tasks(TurnAbortReason::Replaced)
        .await;
    sleep(Duration::from_millis(100)).await;
    replace_thread_goal(
        harness.state_db.as_ref().expect("state db should exist"),
        worker_thread_id,
        StateThreadGoalStatus::Active,
    )
    .await;
    let baseline_op_count = harness.manager.captured_ops().len();

    let completed_turn = emit_turn_complete(&worker_thread, "waiting for goal continuation").await;
    harness
        .manager
        .maybe_notify_parent_of_final_status(worker_thread_id)
        .await;
    let captured_ops = harness.manager.captured_ops();
    assert!(!captured_child_completion(
        &captured_ops[baseline_op_count..],
        root_thread_id,
        &worker_path,
        &AgentPath::root(),
    ));

    GoalService
        .complete_thread_goal(
            worker_thread.codex.session.as_ref(),
            completed_turn.as_ref(),
        )
        .await
        .expect("goal completion should be written");
    let captured_ops = harness.manager.captured_ops();
    assert_eq!(
        count_captured_child_completions(
            &captured_ops[baseline_op_count..],
            root_thread_id,
            &worker_path,
            &AgentPath::root(),
        ),
        0,
        "external goal completion is not a child on_task_finished completion point"
    );

    let next_turn = emit_turn_complete(&worker_thread, "goal follow-up done").await;
    worker_thread
        .codex
        .session
        .maybe_notify_parent_of_final_status(next_turn.as_ref())
        .await;
    let captured_ops = harness.manager.captured_ops();
    assert_eq!(
        count_captured_child_completions(
            &captured_ops[baseline_op_count..],
            root_thread_id,
            &worker_path,
            &AgentPath::root(),
        ),
        1
    );
}

#[tokio::test]
async fn multi_agent_v2_completion_allows_paused_and_budget_limited_goals() {
    for status in [
        StateThreadGoalStatus::Complete,
        StateThreadGoalStatus::Paused,
        StateThreadGoalStatus::BudgetLimited,
    ] {
        let harness = AgentControlHarness::new().await;
        let (root_thread_id, _root_thread) = harness.start_thread().await;
        let mut config = harness.config.clone();
        let _ = config.features.enable(Feature::MultiAgentV2);
        let _ = config.features.enable(Feature::Goals);
        let worker_path = AgentPath::root()
            .join(format!("goal_worker_{}", status.as_str()).as_str())
            .expect("worker path");
        let worker_thread_id = harness
            .control
            .spawn_agent(
                config,
                text_input("hello worker"),
                Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                    parent_thread_id: root_thread_id,
                    depth: 1,
                    agent_path: Some(worker_path.clone()),
                    agent_nickname: None,
                    agent_role: Some("explorer".to_string()),
                })),
            )
            .await
            .expect("worker spawn should succeed");
        let worker_thread = harness
            .manager
            .get_thread(worker_thread_id)
            .await
            .expect("worker thread should exist");
        replace_thread_goal(
            harness.state_db.as_ref().expect("state db should exist"),
            worker_thread_id,
            status,
        )
        .await;
        let baseline_op_count = harness.manager.captured_ops().len();

        emit_turn_complete(&worker_thread, "goal is no longer active").await;
        harness
            .manager
            .maybe_notify_parent_of_final_status(worker_thread_id)
            .await;
        let captured_ops = harness.manager.captured_ops();
        assert_eq!(
            count_captured_child_completions(
                &captured_ops[baseline_op_count..],
                root_thread_id,
                &worker_path,
                &AgentPath::root(),
            ),
            1,
            "{status:?} should allow child completion"
        );
    }
}

#[tokio::test]
async fn multi_agent_v2_restored_event_subscription_allows_completion() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root()
        .join("restored_worker")
        .expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            config,
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");

    // Simulates app-server startup restoring a persisted event subscription.
    harness
        .manager
        .active_event_subscriptions()
        .set_active_count(worker_thread_id, 1);
    let baseline_op_count = harness.manager.captured_ops().len();

    emit_turn_complete(&worker_thread, "waiting for event subscription").await;
    *worker_thread.codex.session.active_turn.lock().await = None;
    harness
        .manager
        .maybe_notify_parent_of_final_status(worker_thread_id)
        .await;
    let captured_ops = harness.manager.captured_ops();
    assert!(captured_child_completion(
        &captured_ops[baseline_op_count..],
        root_thread_id,
        &worker_path,
        &AgentPath::root(),
    ));
}

#[tokio::test]
async fn multi_agent_v2_completion_waits_for_unfinished_subagent() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let tester_path = worker_path.join("tester").expect("tester path");
    let tester_thread_id = harness
        .control
        .spawn_agent(
            config,
            text_input("hello tester"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: worker_thread_id,
                depth: 2,
                agent_path: Some(tester_path),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("tester spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");
    let tester_thread = harness
        .manager
        .get_thread(tester_thread_id)
        .await
        .expect("tester thread should exist");
    let baseline_op_count = harness.manager.captured_ops().len();

    let worker_turn = worker_thread.codex.session.new_default_turn().await;
    worker_thread
        .codex
        .session
        .send_event(
            worker_turn.as_ref(),
            EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: worker_turn.sub_id.clone(),
                last_agent_message: Some("done".to_string()),
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        )
        .await;
    *worker_thread.codex.session.active_turn.lock().await = None;
    sleep(Duration::from_millis(100)).await;
    let captured_ops = harness.manager.captured_ops();

    assert!(!captured_child_completion(
        &captured_ops[baseline_op_count..],
        root_thread_id,
        &worker_path,
        &AgentPath::root(),
    ));

    let tester_turn = tester_thread.codex.session.new_default_turn().await;
    tester_thread
        .codex
        .session
        .send_event_raw(Event {
            id: tester_turn.sub_id.clone(),
            msg: EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: tester_turn.sub_id.clone(),
                last_agent_message: Some("tester done".to_string()),
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            }),
        })
        .await;
    *tester_thread.codex.session.active_turn.lock().await = None;
    worker_thread
        .codex
        .session
        .maybe_notify_parent_of_final_status(worker_turn.as_ref())
        .await;
    let captured_ops = harness.manager.captured_ops();

    assert!(captured_child_completion(
        &captured_ops[baseline_op_count..],
        root_thread_id,
        &worker_path,
        &AgentPath::root(),
    ));
}

#[tokio::test]
async fn multi_agent_v2_management_agent_does_not_notify_parent_on_completion() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let mut config = harness.config.clone();
    let _ = config.features.enable(Feature::MultiAgentV2);
    let worker_path = AgentPath::root().join("manager").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent_with_metadata(
            config,
            text_input("manage this project"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
            SpawnAgentOptions {
                agent_mode: AgentMode::Management,
                ..Default::default()
            },
        )
        .await
        .expect("worker spawn should succeed")
        .thread_id;
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");
    let baseline_op_count = harness.manager.captured_ops().len();

    emit_turn_complete(&worker_thread, "done").await;
    sleep(Duration::from_millis(100)).await;
    let captured_ops = harness.manager.captured_ops();

    assert!(!captured_child_completion(
        &captured_ops[baseline_op_count..],
        root_thread_id,
        &worker_path,
        &AgentPath::root(),
    ));
    assert_eq!(
        worker_thread.codex.session.thread_post_turn_state().await,
        ThreadPostTurnState::ThreadCompletion
    );
}

#[tokio::test]
async fn spawn_thread_subagent_gets_random_nickname_in_session_source() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should be registered");
    let snapshot = child_thread.config_snapshot().await;

    let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: seen_parent_thread_id,
        depth,
        agent_nickname,
        agent_role,
        ..
    }) = snapshot.session_source
    else {
        panic!("expected thread-spawn sub-agent source");
    };
    assert_eq!(seen_parent_thread_id, parent_thread_id);
    assert_eq!(depth, 1);
    assert!(agent_nickname.is_some());
    assert_eq!(agent_role, Some("explorer".to_string()));
}

#[tokio::test]
async fn spawn_thread_subagent_uses_role_specific_nickname_candidates() {
    let mut harness = AgentControlHarness::new().await;
    harness.config.agent_roles.insert(
        "researcher".to_string(),
        AgentRoleConfig {
            description: Some("Research role".to_string()),
            config_file: None,
            nickname_candidates: Some(vec!["Atlas".to_string()]),
            ..Default::default()
        },
    );
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("researcher".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should be registered");
    let snapshot = child_thread.config_snapshot().await;

    let SessionSource::SubAgent(SubAgentSource::ThreadSpawn { agent_nickname, .. }) =
        snapshot.session_source
    else {
        panic!("expected thread-spawn sub-agent source");
    };
    assert_eq!(agent_nickname, Some("Atlas".to_string()));
}

#[tokio::test]
async fn resume_thread_subagent_restores_stored_nickname_and_role() {
    let (home, mut config) = test_config().await;
    config
        .features
        .enable(Feature::Sqlite)
        .expect("test config should allow sqlite");
    let state_db = init_state_db(&config).await;
    let manager = ThreadService::with_models_provider_home_and_state_for_tests(
        CodexAuth::from_api_key("dummy"),
        config.model_provider.clone(),
        crate::test_support::model_provider_factory_for_tests(),
        config.codex_home.to_path_buf(),
        std::sync::Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
        state_db.clone(),
    );
    let control = manager.agent_control();
    let harness = AgentControlHarness {
        _home: home,
        config,
        state_db,
        manager,
        control,
    };
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;
    let agent_path = AgentPath::from_string("/root/explorer".to_string())
        .expect("test agent path should be valid");

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: Some(agent_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let mut status_rx = harness
        .control
        .subscribe_status(child_thread_id)
        .await
        .expect("status subscription should succeed");
    if matches!(status_rx.borrow().clone(), AgentStatus::PendingInit) {
        timeout(Duration::from_secs(5), async {
            loop {
                status_rx
                    .changed()
                    .await
                    .expect("child status should advance past pending init");
                if !matches!(status_rx.borrow().clone(), AgentStatus::PendingInit) {
                    break;
                }
            }
        })
        .await
        .expect("child should initialize before shutdown");
    }
    let original_snapshot = child_thread.config_snapshot().await;
    let original_nickname = original_snapshot
        .session_source
        .get_nickname()
        .expect("spawned sub-agent should have a nickname");
    let state_db = child_thread
        .state_db()
        .expect("sqlite state db should be available for nickname resume test");
    timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(Some(metadata)) = state_db.get_thread(child_thread_id).await
                && metadata.agent_nickname.is_some()
                && metadata.agent_role.as_deref() == Some("explorer")
            {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("child thread metadata should be persisted to sqlite before shutdown");

    let _ = harness
        .control
        .shutdown_live_agent(child_thread_id)
        .await
        .expect("child shutdown should submit");

    let resumed_thread_id = harness
        .control
        .resume_agent_from_rollout(
            harness.config.clone(),
            child_thread_id,
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: Some(agent_path.clone()),
                agent_nickname: None,
                agent_role: None,
            }),
        )
        .await
        .expect("resume should succeed");
    assert_eq!(resumed_thread_id, child_thread_id);

    let resumed_snapshot = harness
        .manager
        .get_thread(resumed_thread_id)
        .await
        .expect("resumed child thread should exist")
        .config_snapshot()
        .await;
    let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: resumed_parent_thread_id,
        depth: resumed_depth,
        agent_path: resumed_agent_path,
        agent_nickname: resumed_nickname,
        agent_role: resumed_role,
        ..
    }) = resumed_snapshot.session_source
    else {
        panic!("expected thread-spawn sub-agent source");
    };
    assert_eq!(resumed_parent_thread_id, parent_thread_id);
    assert_eq!(resumed_depth, 1);
    assert_eq!(resumed_agent_path, Some(agent_path));
    assert_eq!(resumed_nickname, Some(original_nickname));
    assert_eq!(resumed_role, Some("explorer".to_string()));

    let _ = harness
        .control
        .shutdown_live_agent(resumed_thread_id)
        .await
        .expect("resumed child shutdown should submit");
}

#[tokio::test]
async fn resume_thread_subagent_restores_stored_agent_path_when_resume_source_omits_it() {
    let (home, mut config) = test_config().await;
    config
        .features
        .enable(Feature::Sqlite)
        .expect("test config should allow sqlite");
    let state_db = init_state_db(&config).await;
    let manager = ThreadService::with_models_provider_home_and_state_for_tests(
        CodexAuth::from_api_key("dummy"),
        config.model_provider.clone(),
        crate::test_support::model_provider_factory_for_tests(),
        config.codex_home.to_path_buf(),
        std::sync::Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
        state_db.clone(),
    );
    let control = manager.agent_control();
    let harness = AgentControlHarness {
        _home: home,
        config,
        state_db,
        manager,
        control,
    };
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;
    let agent_path = AgentPath::from_string("/root/explorer".to_string())
        .expect("test agent path should be valid");

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: Some(agent_path.clone()),
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    persist_thread_for_tree_resume(&child_thread, "persist before resume path test").await;

    let _ = harness
        .control
        .shutdown_live_agent(child_thread_id)
        .await
        .expect("child shutdown should submit");

    let resumed_thread_id = harness
        .control
        .resume_agent_from_rollout(
            harness.config.clone(),
            child_thread_id,
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            }),
        )
        .await
        .expect("resume should succeed");
    assert_eq!(resumed_thread_id, child_thread_id);

    let resumed_snapshot = harness
        .manager
        .get_thread(resumed_thread_id)
        .await
        .expect("resumed child thread should exist")
        .config_snapshot()
        .await;
    let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: resumed_parent_thread_id,
        depth: resumed_depth,
        agent_path: resumed_agent_path,
        ..
    }) = resumed_snapshot.session_source
    else {
        panic!("expected thread-spawn sub-agent source");
    };
    assert_eq!(resumed_parent_thread_id, parent_thread_id);
    assert_eq!(resumed_depth, 1);
    assert_eq!(resumed_agent_path, Some(agent_path));

    let _ = harness
        .control
        .shutdown_live_agent(resumed_thread_id)
        .await
        .expect("resumed child shutdown should submit");
}

#[tokio::test]
async fn resume_agent_from_rollout_reads_archived_rollout_path() {
    let harness = AgentControlHarness::new().await;
    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello"),
            /*session_source*/ None,
        )
        .await
        .expect("child spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    persist_thread_for_tree_resume(&child_thread, "persist before archiving").await;
    let _ = harness
        .control
        .shutdown_live_agent(child_thread_id)
        .await
        .expect("child shutdown should succeed");
    let store = LocalThreadStore::new(
        LocalThreadStoreConfig::from_config(&harness.config),
        harness.state_db.clone(),
    );
    store
        .archive_thread(ArchiveThreadParams {
            thread_id: child_thread_id,
        })
        .await
        .expect("child thread should archive");

    let resumed_thread_id = harness
        .control
        .resume_agent_from_rollout(harness.config.clone(), child_thread_id, SessionSource::Exec)
        .await
        .expect("resume should find archived rollout");
    assert_eq!(resumed_thread_id, child_thread_id);

    let _ = harness
        .control
        .shutdown_live_agent(child_thread_id)
        .await
        .expect("resumed child shutdown should succeed");
}

#[tokio::test]
async fn list_agent_subtree_thread_ids_includes_anonymous_and_closed_descendants() {
    let mut harness = AgentControlHarness::new().await;
    harness.config.agent_max_threads = Some(5);
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let reviewer_path = AgentPath::root().join("reviewer").expect("reviewer path");

    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: worker_thread_id,
                depth: 2,
                agent_path: Some(
                    worker_path
                        .join("child")
                        .expect("worker child path should be valid"),
                ),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker child spawn should succeed");
    let no_path_child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello anonymous child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: worker_thread_id,
                depth: 2,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("no-path child spawn should succeed");
    let no_path_grandchild_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello anonymous grandchild"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: no_path_child_thread_id,
                depth: 3,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("no-path grandchild spawn should succeed");
    let _reviewer_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello reviewer"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: Some(reviewer_path),
                agent_nickname: None,
                agent_role: Some("reviewer".to_string()),
            })),
        )
        .await
        .expect("reviewer spawn should succeed");
    wait_for_live_thread_spawn_children(
        &harness.control,
        worker_thread_id,
        &[worker_child_thread_id, no_path_child_thread_id],
    )
    .await;
    wait_for_live_thread_spawn_children(
        &harness.control,
        no_path_child_thread_id,
        &[no_path_grandchild_thread_id],
    )
    .await;

    let _ = harness
        .control
        .shutdown_live_agent(no_path_grandchild_thread_id)
        .await
        .expect("no-path grandchild shutdown should succeed");

    let mut worker_subtree_thread_ids = harness
        .manager
        .list_agent_subtree_thread_ids(worker_thread_id)
        .await
        .expect("worker subtree thread ids should load");
    worker_subtree_thread_ids.sort_by_key(ToString::to_string);
    let mut expected_worker_subtree_thread_ids = vec![
        worker_thread_id,
        worker_child_thread_id,
        no_path_child_thread_id,
        no_path_grandchild_thread_id,
    ];
    expected_worker_subtree_thread_ids.sort_by_key(ToString::to_string);
    assert_eq!(
        worker_subtree_thread_ids,
        expected_worker_subtree_thread_ids
    );

    let mut no_path_child_subtree_thread_ids = harness
        .manager
        .list_agent_subtree_thread_ids(no_path_child_thread_id)
        .await
        .expect("no-path subtree thread ids should load");
    no_path_child_subtree_thread_ids.sort_by_key(ToString::to_string);
    let mut expected_no_path_child_subtree_thread_ids =
        vec![no_path_child_thread_id, no_path_grandchild_thread_id];
    expected_no_path_child_subtree_thread_ids.sort_by_key(ToString::to_string);
    assert_eq!(
        no_path_child_subtree_thread_ids,
        expected_no_path_child_subtree_thread_ids
    );
}

#[tokio::test]
async fn direct_subagent_paths_returns_only_immediate_canonical_paths() {
    let mut harness = AgentControlHarness::new().await;
    harness.config.agent_max_threads = Some(4);
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let reviewer_path = AgentPath::root().join("reviewer").expect("reviewer path");

    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: worker_thread_id,
                depth: 2,
                agent_path: Some(
                    worker_path
                        .join("child")
                        .expect("worker child path should be valid"),
                ),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker child spawn should succeed");
    let reviewer_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello reviewer"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: Some(reviewer_path.clone()),
                agent_nickname: None,
                agent_role: Some("reviewer".to_string()),
            })),
        )
        .await
        .expect("reviewer spawn should succeed");
    let anonymous_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello anonymous"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("anonymous spawn should succeed");
    wait_for_live_thread_spawn_children(
        &harness.control,
        parent_thread_id,
        &[worker_thread_id, reviewer_thread_id, anonymous_thread_id],
    )
    .await;
    wait_for_live_thread_spawn_children(
        &harness.control,
        worker_thread_id,
        &[worker_child_thread_id],
    )
    .await;

    let direct_paths = harness
        .control
        .direct_subagent_paths(parent_thread_id)
        .await;

    assert_eq!(direct_paths, vec![reviewer_path, worker_path]);
}

#[tokio::test]
async fn direct_subagent_paths_include_persisted_children_when_registry_is_empty() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    let worker_thread = harness
        .manager
        .get_thread(worker_thread_id)
        .await
        .expect("worker thread should exist");
    persist_thread_for_tree_resume(&worker_thread, "worker persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[worker_thread_id])
        .await;

    let (_restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let direct_paths = restarted_control
        .direct_subagent_paths(parent_thread_id)
        .await;

    assert_eq!(direct_paths, vec![worker_path]);
}

#[tokio::test]
async fn direct_subagent_paths_use_live_source_when_registry_metadata_is_missing() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker spawn should succeed");
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[worker_thread_id])
        .await;

    harness
        .control
        .state
        .release_spawned_thread(worker_thread_id);

    let direct_paths = harness
        .control
        .direct_subagent_paths(parent_thread_id)
        .await;
    assert_eq!(direct_paths, vec![worker_path]);
}

#[tokio::test]
async fn persisted_agent_restore_deduplicates_by_path_with_live_registry_preferred() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let old_worker_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello old worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("old worker spawn should succeed");
    let old_worker_thread = harness
        .manager
        .get_thread(old_worker_thread_id)
        .await
        .expect("old worker thread should exist");
    persist_thread_for_tree_resume(&old_worker_thread, "old worker persisted").await;
    emit_turn_complete(&old_worker_thread, "old done").await;
    old_worker_thread
        .codex
        .session
        .flush_rollout()
        .await
        .expect("old worker rollout should flush");
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[old_worker_thread_id])
        .await;

    let (_restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let new_worker_thread_id = restarted_control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello new worker"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: root_thread_id,
                depth: 1,
                agent_path: Some(worker_path.clone()),
                agent_nickname: Some("worker".to_string()),
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("new worker spawn should succeed");
    assert_ne!(old_worker_thread_id, new_worker_thread_id);

    let listed_agents = restarted_control
        .list_agents(root_thread_id, &SessionSource::Exec, None)
        .await
        .expect("list agents should succeed");
    let matching_agents = listed_agents
        .iter()
        .filter(|agent| agent.agent_name == worker_path.to_string())
        .collect::<Vec<_>>();
    assert_eq!(matching_agents.len(), 1);
    assert_ne!(
        matching_agents[0].lifecycle_status,
        ThreadLifecycleStatus::completed(Some("old done".to_string())),
        "live registry entry should win over stale persisted entry with the same path",
    );

    let direct_paths = restarted_control
        .direct_subagent_paths(root_thread_id)
        .await;
    assert_eq!(direct_paths, vec![worker_path]);
}

#[tokio::test]
async fn shutdown_agent_tree_closes_live_descendants() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");
    let grandchild_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello grandchild"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: child_thread_id,
                depth: 2,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("grandchild spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let grandchild_thread = harness
        .manager
        .get_thread(grandchild_thread_id)
        .await
        .expect("grandchild thread should exist");
    persist_thread_for_tree_resume(&child_thread, "child persisted").await;
    persist_thread_for_tree_resume(&grandchild_thread, "grandchild persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[child_thread_id])
        .await;
    wait_for_live_thread_spawn_children(&harness.control, child_thread_id, &[grandchild_thread_id])
        .await;

    let _ = harness
        .control
        .shutdown_agent_tree(parent_thread_id)
        .await
        .expect("tree shutdown should succeed");

    assert_eq!(
        harness.control.get_status(parent_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness.control.get_status(child_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness.control.get_status(grandchild_thread_id).await,
        AgentStatus::NotFound
    );

    let shutdown_ids = harness
        .manager
        .captured_ops()
        .into_iter()
        .filter_map(|(thread_id, op)| matches!(op, Op::Shutdown).then_some(thread_id))
        .collect::<Vec<_>>();
    let mut expected_shutdown_ids = vec![parent_thread_id, child_thread_id, grandchild_thread_id];
    expected_shutdown_ids.sort_by_key(std::string::ToString::to_string);
    let mut shutdown_ids = shutdown_ids;
    shutdown_ids.sort_by_key(std::string::ToString::to_string);
    assert_eq!(shutdown_ids, expected_shutdown_ids);
}

#[tokio::test]
async fn shutdown_agent_tree_closes_descendants_when_started_at_child() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");
    let grandchild_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello grandchild"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: child_thread_id,
                depth: 2,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("grandchild spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let grandchild_thread = harness
        .manager
        .get_thread(grandchild_thread_id)
        .await
        .expect("grandchild thread should exist");
    persist_thread_for_tree_resume(&child_thread, "child persisted").await;
    persist_thread_for_tree_resume(&grandchild_thread, "grandchild persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[child_thread_id])
        .await;
    wait_for_live_thread_spawn_children(&harness.control, child_thread_id, &[grandchild_thread_id])
        .await;

    let _ = harness
        .control
        .close_agent(child_thread_id)
        .await
        .expect("child close should succeed");

    let _ = harness
        .control
        .shutdown_agent_tree(parent_thread_id)
        .await
        .expect("tree shutdown should succeed");

    assert_eq!(
        harness.control.get_status(child_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness.control.get_status(grandchild_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness.control.get_status(parent_thread_id).await,
        AgentStatus::NotFound
    );

    let shutdown_ids = harness
        .manager
        .captured_ops()
        .into_iter()
        .filter_map(|(thread_id, op)| matches!(op, Op::Shutdown).then_some(thread_id))
        .collect::<Vec<_>>();
    let mut expected_shutdown_ids = vec![parent_thread_id, child_thread_id, grandchild_thread_id];
    expected_shutdown_ids.sort_by_key(std::string::ToString::to_string);
    let mut shutdown_ids = shutdown_ids;
    shutdown_ids.sort_by_key(std::string::ToString::to_string);
    assert_eq!(shutdown_ids, expected_shutdown_ids);
}

#[tokio::test]
async fn resume_agent_from_rollout_does_not_reopen_closed_descendants() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");
    let grandchild_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello grandchild"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: child_thread_id,
                depth: 2,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("grandchild spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let grandchild_thread = harness
        .manager
        .get_thread(grandchild_thread_id)
        .await
        .expect("grandchild thread should exist");
    persist_thread_for_tree_resume(&parent_thread, "parent persisted").await;
    persist_thread_for_tree_resume(&child_thread, "child persisted").await;
    persist_thread_for_tree_resume(&grandchild_thread, "grandchild persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[child_thread_id])
        .await;
    wait_for_live_thread_spawn_children(&harness.control, child_thread_id, &[grandchild_thread_id])
        .await;

    let _ = harness
        .control
        .close_agent(child_thread_id)
        .await
        .expect("child close should succeed");
    let _ = harness
        .control
        .shutdown_live_agent(parent_thread_id)
        .await
        .expect("parent shutdown should succeed");

    let resumed_parent_thread_id = harness
        .control
        .resume_agent_from_rollout(
            harness.config.clone(),
            parent_thread_id,
            SessionSource::Exec,
        )
        .await
        .expect("single-thread resume should succeed");
    assert_eq!(resumed_parent_thread_id, parent_thread_id);
    assert_ne!(
        harness.control.get_status(parent_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness.control.get_status(child_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness.control.get_status(grandchild_thread_id).await,
        AgentStatus::NotFound
    );

    let _ = harness
        .control
        .shutdown_agent_tree(parent_thread_id)
        .await
        .expect("tree shutdown after resume should succeed");
}

#[tokio::test]
async fn resume_closed_child_reopens_open_descendants() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");
    let grandchild_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello grandchild"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: child_thread_id,
                depth: 2,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("grandchild spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let grandchild_thread = harness
        .manager
        .get_thread(grandchild_thread_id)
        .await
        .expect("grandchild thread should exist");
    persist_thread_for_tree_resume(&parent_thread, "parent persisted").await;
    persist_thread_for_tree_resume(&child_thread, "child persisted").await;
    persist_thread_for_tree_resume(&grandchild_thread, "grandchild persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[child_thread_id])
        .await;
    wait_for_live_thread_spawn_children(&harness.control, child_thread_id, &[grandchild_thread_id])
        .await;

    let _ = harness
        .control
        .close_agent(child_thread_id)
        .await
        .expect("child close should succeed");

    let resumed_child_thread_id = harness
        .control
        .resume_agent_from_rollout(
            harness.config.clone(),
            child_thread_id,
            SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: None,
            }),
        )
        .await
        .expect("child resume should succeed");
    assert_eq!(resumed_child_thread_id, child_thread_id);
    assert_ne!(
        harness.control.get_status(child_thread_id).await,
        AgentStatus::NotFound
    );
    assert_ne!(
        harness.control.get_status(grandchild_thread_id).await,
        AgentStatus::NotFound
    );

    let _ = harness
        .control
        .close_agent(child_thread_id)
        .await
        .expect("child close after resume should succeed");
    let _ = harness
        .control
        .shutdown_live_agent(parent_thread_id)
        .await
        .expect("parent shutdown should succeed");
}

#[tokio::test]
async fn resume_agent_from_rollout_reopens_open_descendants_after_manager_shutdown() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");
    let grandchild_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello grandchild"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: child_thread_id,
                depth: 2,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("grandchild spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let grandchild_thread = harness
        .manager
        .get_thread(grandchild_thread_id)
        .await
        .expect("grandchild thread should exist");
    persist_thread_for_tree_resume(&parent_thread, "parent persisted").await;
    persist_thread_for_tree_resume(&child_thread, "child persisted").await;
    persist_thread_for_tree_resume(&grandchild_thread, "grandchild persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[child_thread_id])
        .await;
    wait_for_live_thread_spawn_children(&harness.control, child_thread_id, &[grandchild_thread_id])
        .await;

    let report = harness
        .manager
        .shutdown_all_threads_bounded(Duration::from_secs(5))
        .await;
    assert_eq!(report.submit_failed, Vec::<ThreadId>::new());
    assert_eq!(report.timed_out, Vec::<ThreadId>::new());

    let resumed_parent_thread_id = harness
        .control
        .resume_agent_from_rollout(
            harness.config.clone(),
            parent_thread_id,
            SessionSource::Exec,
        )
        .await
        .expect("tree resume should succeed");
    assert_eq!(resumed_parent_thread_id, parent_thread_id);
    assert_ne!(
        harness.control.get_status(parent_thread_id).await,
        AgentStatus::NotFound
    );
    assert_ne!(
        harness.control.get_status(child_thread_id).await,
        AgentStatus::NotFound
    );
    assert_ne!(
        harness.control.get_status(grandchild_thread_id).await,
        AgentStatus::NotFound
    );

    let _ = harness
        .control
        .shutdown_agent_tree(parent_thread_id)
        .await
        .expect("tree shutdown after subtree resume should succeed");
}

#[tokio::test]
async fn resume_agent_from_rollout_uses_edge_data_when_descendant_metadata_source_is_stale() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");
    let grandchild_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello grandchild"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: child_thread_id,
                depth: 2,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("grandchild spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let grandchild_thread = harness
        .manager
        .get_thread(grandchild_thread_id)
        .await
        .expect("grandchild thread should exist");
    persist_thread_for_tree_resume(&parent_thread, "parent persisted").await;
    persist_thread_for_tree_resume(&child_thread, "child persisted").await;
    persist_thread_for_tree_resume(&grandchild_thread, "grandchild persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[child_thread_id])
        .await;
    wait_for_live_thread_spawn_children(&harness.control, child_thread_id, &[grandchild_thread_id])
        .await;

    let state_db = grandchild_thread
        .state_db()
        .expect("sqlite state db should be available");
    let mut stale_metadata = state_db
        .get_thread(grandchild_thread_id)
        .await
        .expect("grandchild metadata query should succeed")
        .expect("grandchild metadata should exist");
    stale_metadata.source =
        serde_json::to_string(&SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
            parent_thread_id: ThreadId::new(),
            depth: 99,
            agent_path: None,
            agent_nickname: None,
            agent_role: Some("worker".to_string()),
        }))
        .expect("stale session source should serialize");
    state_db
        .upsert_thread(&stale_metadata)
        .await
        .expect("stale grandchild metadata should persist");

    let report = harness
        .manager
        .shutdown_all_threads_bounded(Duration::from_secs(5))
        .await;
    assert_eq!(report.submit_failed, Vec::<ThreadId>::new());
    assert_eq!(report.timed_out, Vec::<ThreadId>::new());

    let resumed_parent_thread_id = harness
        .control
        .resume_agent_from_rollout(
            harness.config.clone(),
            parent_thread_id,
            SessionSource::Exec,
        )
        .await
        .expect("tree resume should succeed");
    assert_eq!(resumed_parent_thread_id, parent_thread_id);
    assert_ne!(
        harness.control.get_status(parent_thread_id).await,
        AgentStatus::NotFound
    );
    assert_ne!(
        harness.control.get_status(child_thread_id).await,
        AgentStatus::NotFound
    );
    assert_ne!(
        harness.control.get_status(grandchild_thread_id).await,
        AgentStatus::NotFound
    );

    let resumed_grandchild_snapshot = harness
        .manager
        .get_thread(grandchild_thread_id)
        .await
        .expect("resumed grandchild thread should exist")
        .config_snapshot()
        .await;
    let SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: resumed_parent_thread_id,
        depth: resumed_depth,
        ..
    }) = resumed_grandchild_snapshot.session_source
    else {
        panic!("expected thread-spawn sub-agent source");
    };
    assert_eq!(resumed_parent_thread_id, child_thread_id);
    assert_eq!(resumed_depth, 2);

    let _ = harness
        .control
        .shutdown_agent_tree(parent_thread_id)
        .await
        .expect("tree shutdown after subtree resume should succeed");
}

#[tokio::test]
async fn resume_agent_from_rollout_skips_descendants_when_parent_resume_fails() {
    let harness = AgentControlHarness::new().await;
    let (parent_thread_id, parent_thread) = harness.start_thread().await;

    let child_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello child"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id,
                depth: 1,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("explorer".to_string()),
            })),
        )
        .await
        .expect("child spawn should succeed");
    let grandchild_thread_id = harness
        .control
        .spawn_agent(
            harness.config.clone(),
            text_input("hello grandchild"),
            Some(SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
                parent_thread_id: child_thread_id,
                depth: 2,
                agent_path: None,
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("grandchild spawn should succeed");

    let child_thread = harness
        .manager
        .get_thread(child_thread_id)
        .await
        .expect("child thread should exist");
    let grandchild_thread = harness
        .manager
        .get_thread(grandchild_thread_id)
        .await
        .expect("grandchild thread should exist");
    persist_thread_for_tree_resume(&parent_thread, "parent persisted").await;
    persist_thread_for_tree_resume(&child_thread, "child persisted").await;
    persist_thread_for_tree_resume(&grandchild_thread, "grandchild persisted").await;
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[child_thread_id])
        .await;
    wait_for_live_thread_spawn_children(&harness.control, child_thread_id, &[grandchild_thread_id])
        .await;

    let child_rollout_path = child_thread
        .rollout_path()
        .expect("child thread should have rollout path");
    let report = harness
        .manager
        .shutdown_all_threads_bounded(Duration::from_secs(5))
        .await;
    assert_eq!(report.submit_failed, Vec::<ThreadId>::new());
    assert_eq!(report.timed_out, Vec::<ThreadId>::new());
    tokio::fs::remove_file(&child_rollout_path)
        .await
        .expect("child rollout path should be removable");

    let resumed_parent_thread_id = harness
        .control
        .resume_agent_from_rollout(
            harness.config.clone(),
            parent_thread_id,
            SessionSource::Exec,
        )
        .await
        .expect("root resume should succeed");
    assert_eq!(resumed_parent_thread_id, parent_thread_id);
    assert_ne!(
        harness.control.get_status(parent_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness.control.get_status(child_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness.control.get_status(grandchild_thread_id).await,
        AgentStatus::NotFound
    );

    let _ = harness
        .control
        .shutdown_agent_tree(parent_thread_id)
        .await
        .expect("tree shutdown after partial subtree resume should succeed");
}
