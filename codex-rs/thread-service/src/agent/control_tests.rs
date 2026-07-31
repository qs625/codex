use super::*;
use crate::CodexThread;
use crate::PendingInputItem;
use crate::StateDbHandle;
use crate::ThreadService;
use crate::TurnContext;
use crate::agent::AgentMode;
use crate::agent::agent_status_from_event;
use crate::agent::external::opencode_reconnect_descriptor;
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
use protocol::protocol::RolloutItem;
use protocol::protocol::SessionSource;
use protocol::protocol::SubAgentSource;
use protocol::protocol::ThreadLifecycleStatus;
use protocol::protocol::ThreadSource;
use protocol::protocol::TurnAbortReason;
use protocol::protocol::TurnAbortedEvent;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::TurnStartedEvent;
use protocol::user_input::UserInput;
use state_api::DirectionalThreadSpawnEdgeStatus;
use state_api::ThreadGoalStatus as StateThreadGoalStatus;
use tempfile::TempDir;
use thread_service_api::AgentDirectoryEntrySource;
use thread_service_api::AgentDirectoryListRequest;
use thread_service_api::AgentReferenceResolution;
use thread_service_api::AgentReferenceResolutionRequest;
use thread_service_api::ExternalRootThreadRuntime;
use thread_service_api::LiveThreadInspectionRuntime;
use thread_service_api::ThreadLifecycleRuntime;
use thread_service_api::ThreadRuntimeStatus;
use thread_store::LocalThreadStore;
use thread_store::LocalThreadStoreConfig;
use thread_store_api::ArchiveThreadParams;
use thread_store_api::ReadThreadParams;
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

async fn register_external_live_thread(
    harness: &AgentControlHarness,
    thread_id: ThreadId,
    agent_path: &str,
    status: AgentStatus,
) {
    let child_agent_path = AgentPath::try_from(agent_path).expect("agent path");
    let session_source = external_session_source_for(
        ThreadId::new(),
        1,
        child_agent_path.clone(),
        SpawnAgentProvider::CodexCli,
    );
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let agent_metadata = AgentMetadata {
        agent_id: Some(thread_id),
        agent_path: Some(child_agent_path),
        agent_nickname: Some("codex_cli".to_string()),
        agent_role: Some("codex_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    harness
        .control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            thread_id,
            external_live_thread_snapshot(
                &external_config,
                thread_id,
                session_source,
                &agent_metadata,
            ),
            status,
        )
        .await;
}

fn external_root_run(
    config: &Config,
    thread_id: ThreadId,
    provider: SpawnAgentProvider,
) -> ExternalAgentRun {
    let mut spawn_config = ExternalSpawnConfig::from_config(config);
    spawn_config.model_provider_id = provider_label(provider).to_string();
    ExternalAgentRun {
        thread_id,
        parent_thread_id: thread_id,
        agent_path: AgentPath::root(),
        provider,
        depth: 0,
        spawn_config: Some(spawn_config),
        input_sink: None,
        live_thread: None,
        status: AgentStatus::Running,
        active_turn_id: None,
        last_task_message: None,
        abort_handle: None,
    }
}

fn named_external_root_run(
    config: &Config,
    thread_id: ThreadId,
    provider: SpawnAgentProvider,
    agent_path: &str,
) -> ExternalAgentRun {
    let mut run = external_root_run(config, thread_id, provider);
    run.agent_path = AgentPath::try_from(agent_path).expect("valid root agent path");
    run
}

async fn persist_external_child_for_restart(
    harness: &AgentControlHarness,
    root_thread_id: ThreadId,
    agent_path: &str,
    status: AgentStatus,
) -> (ThreadId, AgentPath) {
    persist_external_child_for_restart_with_provider(
        harness,
        root_thread_id,
        agent_path,
        SpawnAgentProvider::ClaudeCli,
        status,
    )
    .await
}

async fn persist_external_child_for_restart_with_provider(
    harness: &AgentControlHarness,
    root_thread_id: ThreadId,
    agent_path: &str,
    provider: SpawnAgentProvider,
    status: AgentStatus,
) -> (ThreadId, AgentPath) {
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from(agent_path).expect("agent path");
    let provider_id = provider_label(provider).to_string();
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some(provider_id.clone()),
    });
    let mut external_config = ExternalSpawnConfig::from_config(&harness.config);
    external_config.model_provider_id = provider_id.clone();
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some(provider_id),
        last_task_message: Some("persist me".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &external_config,
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path.clone(),
            provider,
            depth: 1,
            spawn_config: Some(external_config),
            input_sink: None,
            live_thread: Some(live_thread.clone()),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("persist me".to_string()),
            abort_handle: None,
        });
    live_thread
        .persist()
        .await
        .expect("persist external thread metadata");
    harness
        .control
        .persist_external_terminal_status(external_thread_id, &status)
        .await;
    (external_thread_id, external_agent_path)
}

#[tokio::test]
async fn opencode_reconnect_descriptor_persists_to_session_meta_history() {
    let harness = AgentControlHarness::new().await;
    let thread_id = ThreadId::new();
    let mut external_config = ExternalSpawnConfig::from_config(&harness.config);
    external_config.model_provider_id = provider_label(SpawnAgentProvider::Opencode).to_string();
    let agent_metadata = AgentMetadata::default();
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &external_config,
            thread_id,
            SessionSource::Cli,
            ThreadSource::User,
            &agent_metadata,
        )
        .await
        .expect("create persisted external root");
    live_thread.persist().await.expect("persist session meta");
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id,
            parent_thread_id: thread_id,
            agent_path: AgentPath::root(),
            provider: SpawnAgentProvider::Opencode,
            depth: 0,
            spawn_config: Some(external_config),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: None,
            abort_handle: None,
        });

    let descriptor = opencode_reconnect_descriptor("opencode-session-123");
    harness
        .control
        .persist_external_reconnect_descriptor(thread_id, descriptor.clone())
        .await
        .expect("persist reconnect descriptor");

    let stored = harness
        .manager
        .read_thread(ReadThreadParams {
            thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read stored external thread");
    let history = stored.history.expect("history");
    let descriptors = history
        .items
        .iter()
        .filter_map(|item| match item {
            RolloutItem::SessionMeta(meta_line) => meta_line.meta.external_reconnect.clone(),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(descriptors, vec![descriptor]);
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
        input_sink: None,
        live_thread: None,
        status: AgentStatus::Running,
        active_turn_id: None,
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
        input_sink: None,
        live_thread: None,
        status: AgentStatus::Running,
        active_turn_id: None,
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
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Completed(Some("done".to_string())),
            active_turn_id: None,
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
        ThreadLifecycleStatus::completed(None)
    );
    let details = harness
        .control
        .read_agent(root_thread_id, &SessionSource::Exec, external_thread_id)
        .await
        .expect("read agent");
    assert_eq!(details.last_task_message.as_deref(), Some("do work"));
    assert_eq!(
        details.lifecycle_status,
        ThreadLifecycleStatus::completed(Some("done".to_string()))
    );
}

#[tokio::test]
async fn root_external_list_agents_is_scoped_to_sender_root() {
    let harness = AgentControlHarness::new().await;
    let root_a = ThreadId::new();
    let root_b = ThreadId::new();
    let worker_a = ThreadId::new();
    let worker_b = ThreadId::new();
    for (thread_id, parent_thread_id, path, task) in [
        (root_a, root_a, "/root", "root A"),
        (root_b, root_b, "/root", "root B"),
        (worker_a, root_a, "/root/worker", "worker A"),
        (worker_b, root_b, "/root/worker", "worker B"),
    ] {
        harness
            .control
            .external_agents
            .insert_running(ExternalAgentRun {
                thread_id,
                parent_thread_id,
                agent_path: AgentPath::try_from(path).expect("agent path"),
                provider: SpawnAgentProvider::ClaudeCli,
                depth: if thread_id == parent_thread_id { 0 } else { 1 },
                spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
                input_sink: None,
                live_thread: None,
                status: AgentStatus::Running,
                active_turn_id: None,
                last_task_message: Some(task.to_string()),
                abort_handle: None,
            });
    }

    let agents = harness
        .control
        .list_agents(root_a, &SessionSource::Unknown, Some("worker"))
        .await
        .expect("list root scoped agents");

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_name, "/root/worker");
    let details = harness
        .control
        .read_agent(root_a, &SessionSource::Unknown, worker_a)
        .await
        .expect("read root scoped agent");
    assert_eq!(details.last_task_message.as_deref(), Some("worker A"));
}

#[tokio::test]
async fn root_external_followup_resolves_target_within_sender_scope() {
    let harness = AgentControlHarness::new().await;
    let root_a = ThreadId::new();
    let root_b = ThreadId::new();
    let worker_a = ThreadId::new();
    let worker_b = ThreadId::new();
    let (input_tx_a, mut input_rx_a) = tokio::sync::mpsc::unbounded_channel();
    let (input_tx_b, mut input_rx_b) = tokio::sync::mpsc::unbounded_channel();

    for (thread_id, parent_thread_id, path, input_sink) in [
        (root_a, root_a, "/root", None),
        (root_b, root_b, "/root", None),
        (
            worker_a,
            root_a,
            "/root/worker",
            Some(crate::agent::external::ExternalAgentInputSink::new(
                input_tx_a,
            )),
        ),
        (
            worker_b,
            root_b,
            "/root/worker",
            Some(crate::agent::external::ExternalAgentInputSink::new(
                input_tx_b,
            )),
        ),
    ] {
        harness
            .control
            .external_agents
            .insert_running(ExternalAgentRun {
                thread_id,
                parent_thread_id,
                agent_path: AgentPath::try_from(path).expect("agent path"),
                provider: SpawnAgentProvider::ClaudeCli,
                depth: if thread_id == parent_thread_id { 0 } else { 1 },
                spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
                input_sink,
                live_thread: None,
                status: AgentStatus::Running,
                active_turn_id: None,
                last_task_message: None,
                abort_handle: None,
            });
    }

    let result = harness
        .control
        .dispatch_external_tool_call(
            root_a,
            ExternalToolCall {
                id: "call_1".to_string(),
                tool: ExternalToolName::FollowupExternalTask,
                arguments: serde_json::json!({
                    "target": "worker",
                    "message": "scoped hello"
                }),
            },
        )
        .await;

    assert!(result.ok, "followup failed: {:?}", result.error);
    let queued = input_rx_a.recv().await.expect("worker A input");
    assert_eq!(queued.content, "scoped hello");
    assert!(input_rx_b.try_recv().is_err());
}

#[tokio::test]
async fn root_external_close_resolves_live_target_through_directory() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let worker_thread_id = ThreadId::new();

    for (thread_id, parent_thread_id, path) in [
        (root_thread_id, root_thread_id, "/root"),
        (worker_thread_id, root_thread_id, "/root/worker"),
    ] {
        harness
            .control
            .external_agents
            .insert_running(ExternalAgentRun {
                thread_id,
                parent_thread_id,
                agent_path: AgentPath::try_from(path).expect("agent path"),
                provider: SpawnAgentProvider::ClaudeCli,
                depth: if thread_id == parent_thread_id { 0 } else { 1 },
                spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
                input_sink: None,
                live_thread: None,
                status: AgentStatus::Running,
                active_turn_id: None,
                last_task_message: None,
                abort_handle: None,
            });
    }

    let result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "close_worker".to_string(),
                tool: ExternalToolName::CloseExternalAgent,
                arguments: serde_json::json!({ "target": "worker" }),
            },
        )
        .await;

    assert!(result.ok, "close failed: {:?}", result.error);
    assert!(
        harness
            .control
            .external_agents
            .get(worker_thread_id)
            .is_none(),
        "closed external live target should be removed from live registry",
    );
}

#[tokio::test]
async fn root_external_tools_spawn_child_and_reject_invalid_targets() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    harness
        .control
        .external_agents
        .insert_running(external_root_run(
            &harness.config,
            root_thread_id,
            SpawnAgentProvider::ClaudeCli,
        ));

    let native_spawn_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "spawn_native".to_string(),
                tool: ExternalToolName::SpawnExternalAgent,
                arguments: serde_json::json!({
                    "task_name": "native_worker",
                    "provider": "native",
                    "cwd": harness.config.cwd.display().to_string(),
                    "message": "work"
                }),
            },
        )
        .await;
    assert!(!native_spawn_result.ok);
    assert!(native_spawn_result.error.as_ref().is_some_and(|error| {
        error.code == "tool_error"
            && error
                .message
                .contains("spawn_external_agent requires an external provider")
    }));

    let spawn_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "spawn_1".to_string(),
                tool: ExternalToolName::SpawnExternalAgent,
                arguments: serde_json::json!({
                    "task_name": "worker",
                    "provider": "claude_cli",
                    "cwd": harness.config.cwd.display().to_string(),
                    "message": "work"
                }),
            },
        )
        .await;
    assert!(spawn_result.ok, "spawn failed: {:?}", spawn_result.error);
    let spawn_result_json = spawn_result.result.expect("spawn tool result");
    assert_eq!(spawn_result_json["task_name"], "/root/worker");
    assert_eq!(spawn_result_json["provider"], "claude_cli");

    let child_run = harness
        .control
        .external_agents
        .list()
        .into_iter()
        .find(|run| run.agent_path.to_string() == "/root/worker")
        .expect("spawned external child should be registered");
    assert_eq!(child_run.parent_thread_id, root_thread_id);
    assert_eq!(child_run.depth, 1);
    assert_eq!(child_run.provider, SpawnAgentProvider::ClaudeCli);
    assert_eq!(child_run.last_task_message.as_deref(), Some("work"));
    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[child_run.thread_id])
        .await;

    let list_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "list_1".to_string(),
                tool: ExternalToolName::ListExternalAgents,
                arguments: serde_json::json!({ "path_prefix": "/root/worker" }),
            },
        )
        .await;
    assert!(list_result.ok, "list failed: {:?}", list_result.error);
    let agents = list_result.result.expect("list tool result")["agents"]
        .as_array()
        .expect("agents array")
        .clone();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["agentName"], "/root/worker");
    assert_eq!(agents[0]["agentNickname"], "claude_cli");
    assert_eq!(agents[0]["agentRole"], "claude_cli");

    let duplicate_spawn_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "spawn_duplicate".to_string(),
                tool: ExternalToolName::SpawnExternalAgent,
                arguments: serde_json::json!({
                    "task_name": "worker",
                    "provider": "claude_cli",
                    "cwd": harness.config.cwd.display().to_string(),
                    "message": "duplicate"
                }),
            },
        )
        .await;
    assert!(!duplicate_spawn_result.ok);
    assert!(duplicate_spawn_result.error.as_ref().is_some_and(|error| {
        error.code == "tool_error"
            && error
                .message
                .contains("agent path `/root/worker` already exists in external root scope")
    }));

    let _previous_status = harness
        .control
        .close_agent(child_run.thread_id)
        .await
        .expect("spawned external child should close");

    let followup_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "follow_1".to_string(),
                tool: ExternalToolName::FollowupExternalTask,
                arguments: serde_json::json!({
                    "target": "/root",
                    "message": "again"
                }),
            },
        )
        .await;
    assert!(!followup_result.ok);
    assert!(
        followup_result
            .error
            .as_ref()
            .is_some_and(|error| error.code == "tool_error"
                && error.message.contains("cannot follow up to themselves"))
    );

    let close_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "close_1".to_string(),
                tool: ExternalToolName::CloseExternalAgent,
                arguments: serde_json::json!({ "target": "/root" }),
            },
        )
        .await;
    assert!(!close_result.ok);
    assert!(close_result.error.as_ref().is_some_and(
        |error| error.code == "tool_error" && error.message.contains("cannot close themselves")
    ));
}

#[tokio::test]
async fn named_root_external_tools_keep_root_sender_semantics() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    harness
        .control
        .external_agents
        .insert_running(named_external_root_run(
            &harness.config,
            root_thread_id,
            SpawnAgentProvider::ClaudeCli,
            "/foo_project",
        ));

    let spawn_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "spawn_named_root_child".to_string(),
                tool: ExternalToolName::SpawnExternalAgent,
                arguments: serde_json::json!({
                    "task_name": "worker",
                    "provider": "claude_cli",
                    "cwd": harness.config.cwd.display().to_string(),
                    "message": "work"
                }),
            },
        )
        .await;
    assert!(spawn_result.ok, "spawn failed: {:?}", spawn_result.error);
    let spawn_json = spawn_result.result.expect("spawn tool result");
    assert_eq!(spawn_json["task_name"], "/foo_project/worker");

    let list_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "list_named_root_child".to_string(),
                tool: ExternalToolName::ListExternalAgents,
                arguments: serde_json::json!({ "path_prefix": "/foo_project/worker" }),
            },
        )
        .await;
    assert!(list_result.ok, "list failed: {:?}", list_result.error);
    let agents = list_result.result.expect("list result")["agents"]
        .as_array()
        .expect("agents array")
        .clone();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["agentName"], "/foo_project/worker");

    let followup_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "follow_named_root_self".to_string(),
                tool: ExternalToolName::FollowupExternalTask,
                arguments: serde_json::json!({
                    "target": "/foo_project",
                    "message": "again"
                }),
            },
        )
        .await;
    assert!(!followup_result.ok);
    assert!(
        followup_result
            .error
            .as_ref()
            .is_some_and(|error| error.code == "tool_error"
                && error.message.contains("cannot follow up to themselves"))
    );

    let close_result = harness
        .control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "close_named_root_self".to_string(),
                tool: ExternalToolName::CloseExternalAgent,
                arguments: serde_json::json!({ "target": "/foo_project" }),
            },
        )
        .await;
    assert!(!close_result.ok);
    assert!(close_result.error.as_ref().is_some_and(
        |error| error.code == "tool_error" && error.message.contains("cannot close themselves")
    ));
}

#[tokio::test]
async fn root_external_child_spawn_paths_are_scoped_per_root_thread() {
    let harness = AgentControlHarness::new().await;
    let root_a = ThreadId::new();
    let root_b = ThreadId::new();
    harness
        .control
        .external_agents
        .insert_running(external_root_run(
            &harness.config,
            root_a,
            SpawnAgentProvider::ClaudeCli,
        ));
    harness
        .control
        .external_agents
        .insert_running(external_root_run(
            &harness.config,
            root_b,
            SpawnAgentProvider::ClaudeCli,
        ));

    let spawn_worker = |root_thread_id| {
        harness.control.dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: format!("spawn_{root_thread_id}"),
                tool: ExternalToolName::SpawnExternalAgent,
                arguments: serde_json::json!({
                    "task_name": "worker",
                    "provider": "claude_cli",
                    "cwd": harness.config.cwd.display().to_string(),
                    "message": "work"
                }),
            },
        )
    };
    let spawn_a = spawn_worker(root_a).await;
    assert!(spawn_a.ok, "root A spawn failed: {:?}", spawn_a.error);
    let spawn_b = spawn_worker(root_b).await;
    assert!(spawn_b.ok, "root B spawn failed: {:?}", spawn_b.error);

    let child_a = harness
        .control
        .external_agents
        .list()
        .into_iter()
        .find(|run| run.parent_thread_id == root_a && run.agent_path.to_string() == "/root/worker")
        .expect("root A child should be registered");
    let child_b = harness
        .control
        .external_agents
        .list()
        .into_iter()
        .find(|run| run.parent_thread_id == root_b && run.agent_path.to_string() == "/root/worker")
        .expect("root B child should be registered");
    assert_ne!(child_a.thread_id, child_b.thread_id);
    wait_for_live_thread_spawn_children(&harness.control, root_a, &[child_a.thread_id]).await;
    wait_for_live_thread_spawn_children(&harness.control, root_b, &[child_b.thread_id]).await;

    for root_thread_id in [root_a, root_b] {
        let list_result = harness
            .control
            .dispatch_external_tool_call(
                root_thread_id,
                ExternalToolCall {
                    id: format!("list_{root_thread_id}"),
                    tool: ExternalToolName::ListExternalAgents,
                    arguments: serde_json::json!({ "path_prefix": "/root/worker" }),
                },
            )
            .await;
        assert!(
            list_result.ok,
            "root {root_thread_id} list failed: {:?}",
            list_result.error
        );
        let agents = list_result.result.expect("list tool result")["agents"]
            .as_array()
            .expect("agents array")
            .clone();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["agentName"], "/root/worker");
        assert_eq!(agents[0]["agentNickname"], "claude_cli");
    }

    harness
        .control
        .close_agent(child_a.thread_id)
        .await
        .expect("root A child should close");
    harness
        .control
        .close_agent(child_b.thread_id)
        .await
        .expect("root B child should close");
}

#[tokio::test]
async fn spawn_external_agent_rejects_native_provider_before_registration() {
    let harness = AgentControlHarness::new().await;
    let err = harness
        .control
        .spawn_external_agent_with_metadata(
            harness.config.clone(),
            SpawnAgentProvider::Native,
            "do work".to_string(),
            SessionSource::Exec,
            SpawnAgentOptions {
                fork_parent_spawn_call_id: None,
                fork_mode: None,
                environments: None,
                agent_mode: AgentMode::default(),
            },
        )
        .await
        .expect_err("native provider should fail before registration");

    assert!(
        err.to_string()
            .contains("native is not an external CLI provider")
    );
    assert!(harness.control.external_agents.list().is_empty());
}

#[tokio::test]
async fn spawn_external_agent_accepts_codex_provider_before_session_source_check() {
    let harness = AgentControlHarness::new().await;
    let err = harness
        .control
        .spawn_external_agent_with_metadata(
            harness.config.clone(),
            SpawnAgentProvider::CodexCli,
            "do work".to_string(),
            SessionSource::Exec,
            SpawnAgentOptions {
                fork_parent_spawn_call_id: None,
                fork_mode: None,
                environments: None,
                agent_mode: AgentMode::default(),
            },
        )
        .await
        .expect_err("invalid session source should still fail before registration");

    assert!(
        err.to_string()
            .contains("external agents must be spawned as thread-spawn subagents")
    );
    assert!(harness.control.external_agents.list().is_empty());
}

#[tokio::test]
async fn inspection_runtime_includes_external_live_records() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    let child_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    let session_source = external_session_source_for(
        root_thread_id,
        1,
        child_agent_path.clone(),
        SpawnAgentProvider::CodexCli,
    );
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(child_agent_path),
        agent_nickname: Some("codex_cli".to_string()),
        agent_role: Some("codex_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    let external_snapshot = external_live_thread_snapshot(
        &external_config,
        external_thread_id,
        session_source,
        &agent_metadata,
    );
    let expected_external_info = external_snapshot.info.clone();
    harness
        .control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            external_thread_id,
            external_snapshot,
            AgentStatus::Running,
        )
        .await;

    let (native_thread_id, _native_thread) = harness.start_thread().await;

    let live_thread_ids = harness.manager.list_live_thread_ids().await;
    assert!(live_thread_ids.contains(&external_thread_id));
    assert!(live_thread_ids.contains(&native_thread_id));
    let external_id_count = live_thread_ids
        .iter()
        .filter(|thread_id| **thread_id == external_thread_id)
        .count();
    assert_eq!(external_id_count, 1);

    assert!(
        harness
            .manager
            .is_live_thread_loaded(external_thread_id)
            .await
    );
    assert!(
        harness
            .manager
            .is_live_thread_loaded(native_thread_id)
            .await
    );
    assert!(!harness.manager.is_live_thread_loaded(ThreadId::new()).await);

    let external_info = harness
        .manager
        .live_thread_info(external_thread_id)
        .await
        .expect("external live info");
    assert_eq!(external_info, expected_external_info);
    harness
        .manager
        .live_thread_info(native_thread_id)
        .await
        .expect("native live info");
    assert!(matches!(
        harness.manager.live_thread_info(ThreadId::new()).await,
        Err(CodexErr::ThreadNotFound(_))
    ));
}

#[tokio::test]
async fn remove_live_thread_removes_external_live_record_visibility() {
    let harness = AgentControlHarness::new().await;
    let external_thread_id = ThreadId::new();
    register_external_live_thread(
        &harness,
        external_thread_id,
        "/root/external_remove",
        AgentStatus::Running,
    )
    .await;

    assert!(
        harness
            .manager
            .is_live_thread_loaded(external_thread_id)
            .await
    );
    assert!(harness.manager.remove_live_thread(external_thread_id).await);
    assert!(!harness.manager.remove_live_thread(external_thread_id).await);

    let live_thread_ids = harness.manager.list_live_thread_ids().await;
    assert!(!live_thread_ids.contains(&external_thread_id));
    assert!(
        !harness
            .manager
            .is_live_thread_loaded(external_thread_id)
            .await
    );
    assert_matches!(
        harness.manager.live_thread_info(external_thread_id).await,
        Err(CodexErr::ThreadNotFound(id)) if id == external_thread_id
    );
    assert_matches!(
        harness
            .manager
            .live_thread_agent_status(external_thread_id)
            .await,
        Err(CodexErr::ThreadNotFound(id)) if id == external_thread_id
    );
    assert_matches!(
        harness
            .manager
            .live_thread_runtime_status(external_thread_id)
            .await,
        Err(CodexErr::ThreadNotFound(id)) if id == external_thread_id
    );
    assert_matches!(
        harness
            .manager
            .subscribe_live_thread_status(external_thread_id)
            .await,
        Err(CodexErr::ThreadNotFound(id)) if id == external_thread_id
    );
}

#[tokio::test]
async fn remove_live_thread_preserves_native_and_missing_semantics() {
    let harness = AgentControlHarness::new().await;
    let (native_thread_id, _native_thread) = harness.start_thread().await;

    assert!(
        harness
            .manager
            .is_live_thread_loaded(native_thread_id)
            .await
    );
    assert!(harness.manager.remove_live_thread(native_thread_id).await);
    assert!(!harness.manager.remove_live_thread(native_thread_id).await);
    assert!(
        !harness
            .manager
            .is_live_thread_loaded(native_thread_id)
            .await
    );
    assert_matches!(
        harness.manager.live_thread_info(native_thread_id).await,
        Err(CodexErr::ThreadNotFound(id)) if id == native_thread_id
    );

    assert!(!harness.manager.remove_live_thread(ThreadId::new()).await);
}

#[tokio::test]
async fn remove_live_thread_removes_same_id_external_fallback() {
    let harness = AgentControlHarness::new().await;
    let (thread_id, _native_thread) = harness.start_thread().await;
    register_external_live_thread(
        &harness,
        thread_id,
        "/root/external_remove_same_id",
        AgentStatus::Shutdown,
    )
    .await;

    assert!(harness.manager.remove_live_thread(thread_id).await);

    let live_thread_ids = harness.manager.list_live_thread_ids().await;
    assert!(!live_thread_ids.contains(&thread_id));
    assert!(!harness.manager.is_live_thread_loaded(thread_id).await);
    assert_matches!(
        harness.manager.live_thread_info(thread_id).await,
        Err(CodexErr::ThreadNotFound(id)) if id == thread_id
    );
    assert_matches!(
        harness.manager.live_thread_agent_status(thread_id).await,
        Err(CodexErr::ThreadNotFound(id)) if id == thread_id
    );
    assert_matches!(
        harness.manager.live_thread_runtime_status(thread_id).await,
        Err(CodexErr::ThreadNotFound(id)) if id == thread_id
    );
    assert_matches!(
        harness.manager.subscribe_live_thread_status(thread_id).await,
        Err(CodexErr::ThreadNotFound(id)) if id == thread_id
    );
}

#[tokio::test]
async fn runtime_status_maps_external_live_records_to_coarse_status() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let manager = harness
        .control
        .upgrade()
        .expect("manager should be available");

    let statuses = [
        (AgentStatus::PendingInit, ThreadRuntimeStatus::Active),
        (AgentStatus::Running, ThreadRuntimeStatus::Active),
        (AgentStatus::Interrupted, ThreadRuntimeStatus::Complete),
        (
            AgentStatus::Completed(Some("done".to_string())),
            ThreadRuntimeStatus::Complete,
        ),
        (
            AgentStatus::Errored("failed".to_string()),
            ThreadRuntimeStatus::Complete,
        ),
        (AgentStatus::Shutdown, ThreadRuntimeStatus::Complete),
        (AgentStatus::NotFound, ThreadRuntimeStatus::Complete),
    ];

    for (index, (agent_status, expected_runtime_status)) in statuses.into_iter().enumerate() {
        let external_thread_id = ThreadId::new();
        let child_agent_path =
            AgentPath::try_from(format!("/root/external_{index}")).expect("agent path");
        let session_source = external_session_source_for(
            root_thread_id,
            1,
            child_agent_path.clone(),
            SpawnAgentProvider::CodexCli,
        );
        let agent_metadata = AgentMetadata {
            agent_id: Some(external_thread_id),
            agent_path: Some(child_agent_path),
            agent_nickname: Some("codex_cli".to_string()),
            agent_role: Some("codex_cli".to_string()),
            counted: false,
            ..Default::default()
        };
        manager
            .register_external_live_thread_snapshot(
                external_thread_id,
                external_live_thread_snapshot(
                    &external_config,
                    external_thread_id,
                    session_source,
                    &agent_metadata,
                ),
                agent_status,
            )
            .await;

        let runtime_status = harness
            .manager
            .live_thread_runtime_status(external_thread_id)
            .await
            .expect("external runtime status");
        assert_eq!(runtime_status, expected_runtime_status);
    }
}

#[tokio::test]
async fn runtime_status_preserves_native_and_missing_semantics() {
    let harness = AgentControlHarness::new().await;
    let (native_thread_id, native_thread) = harness.start_thread().await;
    let expected_native_status = native_thread.runtime_thread_status().await;

    let native_status = harness
        .manager
        .live_thread_runtime_status(native_thread_id)
        .await
        .expect("native runtime status");
    assert_eq!(native_status, expected_native_status);

    assert!(matches!(
        harness
            .manager
            .live_thread_runtime_status(ThreadId::new())
            .await,
        Err(CodexErr::ThreadNotFound(_))
    ));
}

#[tokio::test]
async fn runtime_status_prefers_native_when_external_record_has_same_id() {
    let harness = AgentControlHarness::new().await;
    let (thread_id, native_thread) = harness.start_thread().await;
    let expected_native_status = native_thread.runtime_thread_status().await;
    let child_agent_path = AgentPath::try_from("/root/external_same_id").expect("agent path");
    let session_source = external_session_source_for(
        ThreadId::new(),
        1,
        child_agent_path.clone(),
        SpawnAgentProvider::CodexCli,
    );
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let agent_metadata = AgentMetadata {
        agent_id: Some(thread_id),
        agent_path: Some(child_agent_path),
        agent_nickname: Some("codex_cli".to_string()),
        agent_role: Some("codex_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    harness
        .control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            thread_id,
            external_live_thread_snapshot(
                &external_config,
                thread_id,
                session_source,
                &agent_metadata,
            ),
            AgentStatus::Shutdown,
        )
        .await;

    let runtime_status = harness
        .manager
        .live_thread_runtime_status(thread_id)
        .await
        .expect("native runtime status");
    assert_eq!(runtime_status, expected_native_status);
}

#[tokio::test]
async fn external_completion_after_close_does_not_notify_parent() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    let child_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    let session_source = external_session_source_for(
        root_thread_id,
        1,
        child_agent_path.clone(),
        SpawnAgentProvider::CodexCli,
    );
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(child_agent_path.clone()),
        agent_nickname: Some("codex_cli".to_string()),
        agent_role: Some("codex_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    harness.control.state.register_root_thread(root_thread_id);
    harness
        .control
        .state
        .register_agent_metadata(agent_metadata.clone());
    harness
        .control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            external_thread_id,
            external_live_thread_snapshot(
                &external_config,
                external_thread_id,
                session_source,
                &agent_metadata,
            ),
            AgentStatus::Running,
        )
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: child_agent_path.clone(),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(external_config),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
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
    assert!(
        !harness
            .manager
            .is_live_thread_loaded(external_thread_id)
            .await
    );
    assert_matches!(
        harness
            .manager
            .live_thread_agent_status(external_thread_id)
            .await,
        Err(CodexErr::ThreadNotFound(id)) if id == external_thread_id
    );
    assert_matches!(
        harness
            .manager
            .live_thread_runtime_status(external_thread_id)
            .await,
        Err(CodexErr::ThreadNotFound(id)) if id == external_thread_id
    );
    assert_matches!(
        harness
            .manager
            .subscribe_live_thread_status(external_thread_id)
            .await,
        Err(CodexErr::ThreadNotFound(id)) if id == external_thread_id
    );
}

#[tokio::test]
async fn external_close_status_changed_event_carries_shutdown_payload() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    let child_agent_path = AgentPath::try_from("/root/external_close_status").expect("agent path");
    let session_source = external_session_source_for(
        root_thread_id,
        1,
        child_agent_path.clone(),
        SpawnAgentProvider::CodexCli,
    );
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(child_agent_path.clone()),
        agent_nickname: Some("codex_cli".to_string()),
        agent_role: Some("codex_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    harness.control.state.register_root_thread(root_thread_id);
    harness
        .control
        .state
        .register_agent_metadata(agent_metadata.clone());
    harness
        .control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            external_thread_id,
            external_live_thread_snapshot(
                &external_config,
                external_thread_id,
                session_source,
                &agent_metadata,
            ),
            AgentStatus::Running,
        )
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: child_agent_path,
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(external_config),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("do work".to_string()),
            abort_handle: None,
        });
    let mut thread_created_rx = harness.manager.subscribe_thread_created();

    harness
        .control
        .close_agent(external_thread_id)
        .await
        .expect("close external agent");

    let event = timeout(Duration::from_secs(1), thread_created_rx.recv())
        .await
        .expect("status changed event should arrive")
        .expect("status changed event");
    assert_eq!(event.thread_id(), external_thread_id);
    assert_matches!(
        event,
        thread_service_api::ThreadCreatedEvent::StatusChanged {
            thread_id,
            agent_status: Some(AgentStatus::Shutdown),
        } if thread_id == external_thread_id
    );
    assert!(
        !harness
            .manager
            .is_live_thread_loaded(external_thread_id)
            .await
    );
    assert_matches!(
        harness.manager.live_thread_info(external_thread_id).await,
        Err(CodexErr::ThreadNotFound(id)) if id == external_thread_id
    );
    assert_eq!(
        harness.control.get_status(external_thread_id).await,
        AgentStatus::Shutdown
    );
    assert!(!harness.manager.remove_live_thread(external_thread_id).await);
}

#[tokio::test]
async fn external_completion_status_changed_event_carries_terminal_payload() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    let child_agent_path =
        AgentPath::try_from("/root/external_complete_status").expect("agent path");
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    harness.control.state.register_root_thread(root_thread_id);
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: child_agent_path,
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(external_config),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("do work".to_string()),
            abort_handle: None,
        });
    let mut thread_created_rx = harness.manager.subscribe_thread_created();
    let completed_status = AgentStatus::Completed(Some("done".to_string()));

    harness
        .control
        .complete_external_agent(external_thread_id, completed_status.clone())
        .await;

    let event = timeout(Duration::from_secs(1), thread_created_rx.recv())
        .await
        .expect("status changed event should arrive")
        .expect("status changed event");
    assert_eq!(event.thread_id(), external_thread_id);
    assert_matches!(
        event,
        thread_service_api::ThreadCreatedEvent::StatusChanged {
            thread_id,
            agent_status: Some(status),
        } if thread_id == external_thread_id && status == completed_status
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
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
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
    assert_eq!(agents[0]["agentName"], "/root/external");
    assert_eq!(agents[0]["agentNickname"], "codex_cli");

    let read_result = harness
        .control
        .dispatch_external_tool_call(
            external_thread_id,
            ExternalToolCall {
                id: "read_1".to_string(),
                tool: ExternalToolName::ReadExternalAgent,
                arguments: serde_json::json!({ "target": "/root/external" }),
            },
        )
        .await;
    assert!(read_result.ok, "read failed: {:?}", read_result.error);
    let agent = &read_result.result.expect("read result")["agent"];
    assert_eq!(agent["agentName"], "/root/external");
    assert_eq!(agent["lastTaskMessage"], "do work");
}

#[tokio::test]
async fn external_tool_call_reads_persisted_external_agent_details() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    harness.control.state.register_root_thread(root_thread_id);
    let persisted_thread_id = ThreadId::new();
    let persisted_path = AgentPath::try_from("/root/external").expect("agent path");
    let provider_id = provider_label(SpawnAgentProvider::CodexCli).to_string();
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(persisted_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some(provider_id.clone()),
    });
    let mut external_config = ExternalSpawnConfig::from_config(&harness.config);
    external_config.model_provider_id = provider_id.clone();
    let agent_metadata = AgentMetadata {
        agent_id: Some(persisted_thread_id),
        agent_path: Some(persisted_path),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some(provider_id),
        last_task_message: Some("persist me".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &external_config,
            persisted_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(persisted_thread_id, Some(&session_source))
        .await;
    live_thread
        .persist()
        .await
        .expect("persist external thread metadata");
    harness
        .control
        .persist_external_terminal_status_to_live_thread(
            persisted_thread_id,
            None,
            &AgentStatus::Completed(Some("done".to_string())),
            Some(live_thread),
        )
        .await;

    let sender_thread_id = ThreadId::new();
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: sender_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: AgentPath::try_from("/root/sender").expect("agent path"),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("inspect persisted".to_string()),
            abort_handle: None,
        });

    let result = harness
        .control
        .dispatch_external_tool_call(
            sender_thread_id,
            ExternalToolCall {
                id: "read_1".to_string(),
                tool: ExternalToolName::ReadExternalAgent,
                arguments: serde_json::json!({ "target": "/root/external" }),
            },
        )
        .await;

    assert!(result.ok, "read failed: {:?}", result.error);
    let agent = &result.result.expect("tool result")["agent"];
    assert_eq!(agent["agentName"], "/root/external");
    assert_eq!(agent["agentNickname"], "worker");
    assert_eq!(agent["agentRole"], "codex_cli");
    assert_eq!(agent["lastTaskMessage"], serde_json::Value::Null);
    assert_eq!(
        agent["lifecycleStatus"]["result"]["last_agent_message"],
        "done"
    );
}

#[tokio::test]
async fn external_tool_call_poll_external_event_wakes_for_inter_agent_input() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    harness.control.state.register_root_thread(root_thread_id);
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut run = ExternalAgentRun {
        thread_id: external_thread_id,
        parent_thread_id: root_thread_id,
        agent_path: external_agent_path.clone(),
        provider: SpawnAgentProvider::CodexCli,
        depth: 1,
        spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
        input_sink: Some(crate::agent::external::ExternalAgentInputSink::new(
            input_tx,
        )),
        live_thread: None,
        status: AgentStatus::Running,
        active_turn_id: None,
        last_task_message: Some("do work".to_string()),
        abort_handle: None,
    };
    let spawn_config = run.spawn_config.as_mut().expect("spawn config");
    spawn_config.default_wait_timeout_ms = 200;
    spawn_config.max_wait_timeout_ms = 200;
    harness.control.external_agents.insert_running(run);

    let control = harness.control.clone();
    let poll_task = tokio::spawn(async move {
        control
            .dispatch_external_tool_call(
                external_thread_id,
                ExternalToolCall {
                    id: "poll_1".to_string(),
                    tool: ExternalToolName::PollExternalEvent,
                    arguments: serde_json::json!({}),
                },
            )
            .await
    });
    sleep(Duration::from_millis(10)).await;

    harness
        .control
        .send_inter_agent_communication(
            external_thread_id,
            InterAgentCommunication::new(
                AgentPath::root(),
                external_agent_path,
                Vec::new(),
                "please continue".to_string(),
                InterAgentOperation::FollowupTask,
            )
            .with_thread_ids(root_thread_id, external_thread_id)
            .with_trigger_turn(true),
        )
        .await
        .expect("deliver external followup");

    let queued = input_rx.recv().await.expect("external input");
    assert_eq!(queued.content, "please continue");
    let result = poll_task.await.expect("poll task");
    assert!(result.ok, "poll failed: {:?}", result.error);
    let result_json = result.result.expect("poll result");
    assert_eq!(result_json["timedOut"], false);
    assert_eq!(result_json["sourceHint"], "inter_agent");
    assert_eq!(result_json["event"]["type"], "inter_agent_communication");
    assert_eq!(
        result_json["event"]["communication"]["content"],
        "please continue"
    );
    assert_eq!(
        result_json["event"]["communication"]["operation"],
        "followupTask"
    );
    assert_eq!(
        result_json["event"]["communication"]["sender_thread_id"],
        root_thread_id.to_string()
    );
    assert_eq!(
        result_json["event"]["communication"]["recipient_thread_id"],
        external_thread_id.to_string()
    );
    assert_eq!(result_json["events"].as_array().expect("events").len(), 1);
    assert_eq!(result_json["initialTimeoutMs"], 200);
    assert_eq!(result_json["currentTimeoutMs"], 200);
    assert_eq!(result_json["hardCapTimeoutMs"], 200);
}

#[tokio::test]
async fn external_tool_call_poll_external_event_wakes_for_root_user_input() {
    let harness = AgentControlHarness::new().await;
    let external_thread_id = ThreadId::new();
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut run = external_root_run(
        &harness.config,
        external_thread_id,
        SpawnAgentProvider::CodexCli,
    );
    run.input_sink = Some(crate::agent::external::ExternalAgentInputSink::new(
        input_tx,
    ));
    let spawn_config = run.spawn_config.as_mut().expect("spawn config");
    spawn_config.default_wait_timeout_ms = 200;
    spawn_config.max_wait_timeout_ms = 200;
    harness.control.external_agents.insert_running(run);

    let control = harness.control.clone();
    let poll_task = tokio::spawn(async move {
        control
            .dispatch_external_tool_call(
                external_thread_id,
                ExternalToolCall {
                    id: "poll_1".to_string(),
                    tool: ExternalToolName::PollExternalEvent,
                    arguments: serde_json::json!({}),
                },
            )
            .await
    });
    sleep(Duration::from_millis(10)).await;

    let turn_id = harness
        .control
        .send_external_root_input(external_thread_id, "continue root".to_string())
        .await
        .expect("send external root input");

    let queued = input_rx.recv().await.expect("external input");
    assert_eq!(queued.turn_id.as_deref(), Some(turn_id.as_str()));
    assert_eq!(queued.content, "continue root");
    let result = poll_task.await.expect("poll task");
    assert!(result.ok, "poll failed: {:?}", result.error);
    let result_json = result.result.expect("poll result");
    assert_eq!(result_json["timedOut"], false);
    assert_eq!(result_json["sourceHint"], "user_input");
    assert_eq!(result_json["event"], serde_json::Value::Null);
    assert_eq!(result_json["events"], serde_json::Value::Null);
}

#[tokio::test]
async fn external_tool_call_poll_external_event_wakes_for_child_completion() {
    let harness = AgentControlHarness::new().await;
    let parent_thread_id = ThreadId::new();
    let child_thread_id = ThreadId::new();
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut parent_run = external_root_run(
        &harness.config,
        parent_thread_id,
        SpawnAgentProvider::CodexCli,
    );
    parent_run.input_sink = Some(crate::agent::external::ExternalAgentInputSink::new(
        input_tx,
    ));
    let spawn_config = parent_run.spawn_config.as_mut().expect("spawn config");
    spawn_config.default_wait_timeout_ms = 200;
    spawn_config.max_wait_timeout_ms = 200;
    harness.control.external_agents.insert_running(parent_run);
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: child_thread_id,
            parent_thread_id,
            agent_path: AgentPath::try_from("/root/child").expect("agent path"),
            provider: SpawnAgentProvider::CodexCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("child work".to_string()),
            abort_handle: None,
        });

    let control = harness.control.clone();
    let poll_task = tokio::spawn(async move {
        control
            .dispatch_external_tool_call(
                parent_thread_id,
                ExternalToolCall {
                    id: "poll_1".to_string(),
                    tool: ExternalToolName::PollExternalEvent,
                    arguments: serde_json::json!({}),
                },
            )
            .await
    });
    sleep(Duration::from_millis(10)).await;

    harness
        .control
        .complete_external_agent(
            child_thread_id,
            AgentStatus::Completed(Some("done".to_string())),
        )
        .await;

    let queued = input_rx.recv().await.expect("external input");
    assert!(queued.content.contains("/root/child"));
    let result = poll_task.await.expect("poll task");
    assert!(result.ok, "poll failed: {:?}", result.error);
    let result_json = result.result.expect("poll result");
    assert_eq!(result_json["timedOut"], false);
    assert_eq!(result_json["sourceHint"], "child_completion");
    assert_eq!(result_json["event"]["type"], "inter_agent_communication");
    let communication = &result_json["event"]["communication"];
    assert_eq!(communication["content"], queued.content);
    assert_eq!(communication["operation"], "childCompletion");
    assert_eq!(
        communication["sender_thread_id"],
        child_thread_id.to_string()
    );
    assert_eq!(
        communication["recipient_thread_id"],
        parent_thread_id.to_string()
    );
    assert_eq!(communication["status"]["completed"], "done");
    assert_eq!(result_json["events"].as_array().expect("events").len(), 1);
}

#[tokio::test]
async fn external_root_runtime_close_removes_live_root_and_persists_shutdown() {
    let harness = AgentControlHarness::new().await;
    let (native_thread_id, _native_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let root_external_control = harness.manager.root_external_agent_control_for_tests();
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(AgentPath::root()),
        agent_nickname: Some("claude_cli".to_string()),
        agent_role: Some("claude_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    let live_thread = root_external_control
        .create_external_thread_persistence(
            &external_config,
            external_thread_id,
            SessionSource::Cli,
            ThreadSource::User,
            &agent_metadata,
        )
        .await
        .expect("create persisted external root");
    root_external_control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            external_thread_id,
            external_live_thread_snapshot(
                &external_config,
                external_thread_id,
                SessionSource::Cli,
                &agent_metadata,
            ),
            AgentStatus::Running,
        )
        .await;
    root_external_control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: external_thread_id,
            agent_path: AgentPath::root(),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 0,
            spawn_config: Some(external_config),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("root work".to_string()),
            abort_handle: None,
        });
    assert!(
        harness
            .manager
            .is_live_thread_loaded(external_thread_id)
            .await
    );

    let closed =
        ExternalRootThreadRuntime::close_external_root_thread(&harness.manager, external_thread_id)
            .await
            .expect("external root close should succeed");
    assert_eq!(closed, "");
    assert!(!harness.manager.has_external_root_thread(external_thread_id));
    assert_eq!(
        harness.control.get_status(external_thread_id).await,
        AgentStatus::NotFound
    );
    assert!(
        !harness
            .manager
            .is_live_thread_loaded(external_thread_id)
            .await
    );
    assert_ne!(
        harness.control.get_status(native_thread_id).await,
        AgentStatus::NotFound,
        "external root runtime close should not close native threads",
    );

    let stored = harness
        .manager
        .read_thread(ReadThreadParams {
            thread_id: external_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external root");
    let items = stored.history.expect("history").items;
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::ExternalTerminalStatus(event))
            if event.thread_id == external_thread_id
                && event.status == protocol::protocol::ExternalTerminalStatus::Shutdown
    )));

    let missing_err =
        ExternalRootThreadRuntime::close_external_root_thread(&harness.manager, native_thread_id)
            .await
            .expect_err("native thread is not an external root");
    assert!(
        matches!(missing_err, CodexErr::ThreadNotFound(thread_id) if thread_id == native_thread_id)
    );
}

#[tokio::test]
async fn named_live_external_root_accepts_root_input() {
    let harness = AgentControlHarness::new().await;
    let external_root_thread_id = ThreadId::new();
    let root_external_control = harness.manager.root_external_agent_control_for_tests();
    let (tx_input, mut rx_input) = tokio::sync::mpsc::unbounded_channel();
    let mut run = named_external_root_run(
        &harness.config,
        external_root_thread_id,
        SpawnAgentProvider::ClaudeCli,
        "/foo_project",
    );
    run.input_sink = Some(ExternalAgentInputSink::new(tx_input));
    root_external_control.external_agents.insert_running(run);

    let turn_id = root_external_control
        .send_external_root_input(external_root_thread_id, "continue work".to_string())
        .await
        .expect("named external root should accept root input");

    let received = rx_input.recv().await.expect("input should be delivered");
    assert_eq!(received.turn_id.as_deref(), Some(turn_id.as_str()));
    assert_eq!(received.content, "continue work");
    assert_eq!(
        root_external_control
            .external_agents
            .get(external_root_thread_id)
            .expect("run should remain registered")
            .active_turn_id
            .as_deref(),
        Some(turn_id.as_str())
    );
}

#[tokio::test]
async fn live_external_root_thread_facts_classifies_root_provider() {
    let harness = AgentControlHarness::new().await;
    let (native_thread_id, _) = harness.start_thread().await;
    let external_root_thread_id = ThreadId::new();
    let external_child_thread_id = ThreadId::new();
    let root_external_control = harness.manager.root_external_agent_control_for_tests();

    root_external_control
        .external_agents
        .insert_running(external_root_run(
            &harness.config,
            external_root_thread_id,
            SpawnAgentProvider::Opencode,
        ));

    let mut child_run = external_root_run(
        &harness.config,
        external_child_thread_id,
        SpawnAgentProvider::ClaudeCli,
    );
    child_run.parent_thread_id = native_thread_id;
    child_run.agent_path = AgentPath::derive(None, "worker").expect("agent path");
    child_run.depth = 1;
    root_external_control
        .external_agents
        .insert_running(child_run);

    assert_eq!(
        ExternalRootThreadRuntime::live_external_root_thread_facts(
            &harness.manager,
            external_root_thread_id
        ),
        Some(thread_service_api::LiveExternalRootThreadFacts {
            thread_id: external_root_thread_id,
            provider: thread_service_api::ExternalRootThreadProvider::Opencode,
        })
    );
    assert_eq!(
        ExternalRootThreadRuntime::live_external_root_thread_facts(
            &harness.manager,
            external_child_thread_id
        ),
        None
    );
    assert_eq!(
        ExternalRootThreadRuntime::live_external_root_thread_facts(
            &harness.manager,
            native_thread_id
        ),
        None
    );
    assert_eq!(
        harness
            .manager
            .external_root_thread_input_route(external_root_thread_id)
            .await
            .expect("live external root route should load"),
        thread_service_api::ExternalRootThreadInputRoute::LiveExternalRoot {
            thread_id: external_root_thread_id,
            provider: thread_service_api::ExternalRootThreadProvider::Opencode,
        }
    );
    assert_eq!(
        harness
            .manager
            .external_root_thread_input_route(external_child_thread_id)
            .await
            .expect("external child route should load"),
        thread_service_api::ExternalRootThreadInputRoute::NativeRequired
    );
    assert_eq!(
        harness
            .manager
            .external_root_thread_input_route(native_thread_id)
            .await
            .expect("native route should load"),
        thread_service_api::ExternalRootThreadInputRoute::NativeRequired
    );
}

#[tokio::test]
async fn shutdown_all_threads_bounded_closes_native_and_external_roots() {
    let harness = AgentControlHarness::new().await;
    let (native_thread_id, native_thread) = harness.start_thread().await;
    let external_root_thread_id = ThreadId::new();
    let external_child_thread_id = ThreadId::new();
    let root_external_control = harness.manager.root_external_agent_control_for_tests();
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let root_agent_metadata = AgentMetadata {
        agent_id: Some(external_root_thread_id),
        agent_path: Some(AgentPath::root()),
        agent_nickname: Some("claude_cli".to_string()),
        agent_role: Some("claude_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    let live_thread = root_external_control
        .create_external_thread_persistence(
            &external_config,
            external_root_thread_id,
            SessionSource::Cli,
            ThreadSource::User,
            &root_agent_metadata,
        )
        .await
        .expect("create persisted external root");
    root_external_control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            external_root_thread_id,
            external_live_thread_snapshot(
                &external_config,
                external_root_thread_id,
                SessionSource::Cli,
                &root_agent_metadata,
            ),
            AgentStatus::Running,
        )
        .await;
    root_external_control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_root_thread_id,
            parent_thread_id: external_root_thread_id,
            agent_path: AgentPath::root(),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 0,
            spawn_config: Some(external_config.clone()),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("root work".to_string()),
            abort_handle: None,
        });
    let child_agent_control = native_thread.codex.session.services.agent_control.clone();
    let child_agent_path = AgentPath::try_from("/root/external_child").expect("agent path");
    let child_session_source = external_session_source_for(
        native_thread_id,
        1,
        child_agent_path.clone(),
        SpawnAgentProvider::ClaudeCli,
    );
    let child_agent_metadata = AgentMetadata {
        agent_id: Some(external_child_thread_id),
        agent_path: Some(child_agent_path.clone()),
        agent_nickname: Some("claude_cli".to_string()),
        agent_role: Some("claude_cli".to_string()),
        counted: true,
        ..Default::default()
    };
    let child_live_thread = child_agent_control
        .create_external_thread_persistence(
            &external_config,
            external_child_thread_id,
            child_session_source.clone(),
            ThreadSource::Subagent,
            &child_agent_metadata,
        )
        .await
        .expect("create persisted external child");
    child_agent_control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            external_child_thread_id,
            external_live_thread_snapshot(
                &external_config,
                external_child_thread_id,
                child_session_source.clone(),
                &child_agent_metadata,
            ),
            AgentStatus::Running,
        )
        .await;
    child_agent_control
        .persist_thread_spawn_edge_for_source(external_child_thread_id, Some(&child_session_source))
        .await;
    child_agent_control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_child_thread_id,
            parent_thread_id: native_thread_id,
            agent_path: child_agent_path,
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(external_config),
            input_sink: None,
            live_thread: Some(child_live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("child work".to_string()),
            abort_handle: None,
        });

    let report = harness
        .manager
        .shutdown_all_threads_bounded(Duration::from_secs(10))
        .await;
    let mut expected_completed = vec![
        native_thread_id,
        external_child_thread_id,
        external_root_thread_id,
    ];
    expected_completed.sort_by_key(std::string::ToString::to_string);
    assert_eq!(report.completed, expected_completed);
    assert!(report.submit_failed.is_empty());
    assert!(report.timed_out.is_empty());
    assert!(harness.manager.list_thread_ids().await.is_empty());
    assert!(
        !harness
            .manager
            .has_external_root_thread(external_root_thread_id)
    );
    assert!(
        child_agent_control
            .external_agents
            .get(external_child_thread_id)
            .is_none()
    );
    assert_eq!(
        harness.control.get_status(external_root_thread_id).await,
        AgentStatus::NotFound
    );
    assert_eq!(
        harness.control.get_status(external_child_thread_id).await,
        AgentStatus::NotFound
    );
    assert!(
        !harness
            .manager
            .is_live_thread_loaded(external_root_thread_id)
            .await
    );
    assert!(
        !harness
            .manager
            .is_live_thread_loaded(external_child_thread_id)
            .await
    );
    let state_db = harness.state_db.as_ref().expect("state db");
    let open_descendants = state_db
        .list_thread_spawn_descendants_with_status(
            native_thread_id,
            DirectionalThreadSpawnEdgeStatus::Open,
        )
        .await
        .expect("list open descendants");
    assert!(
        open_descendants.contains(&external_child_thread_id),
        "shutdown-all should not convert external child edge to explicit Closed",
    );
    let closed_descendants = state_db
        .list_thread_spawn_descendants_with_status(
            native_thread_id,
            DirectionalThreadSpawnEdgeStatus::Closed,
        )
        .await
        .expect("list closed descendants");
    assert!(!closed_descendants.contains(&external_child_thread_id));

    let stored = harness
        .manager
        .read_thread(ReadThreadParams {
            thread_id: external_root_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external root");
    let items = stored.history.expect("history").items;
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::ExternalTerminalStatus(event))
            if event.thread_id == external_root_thread_id
                && event.status == protocol::protocol::ExternalTerminalStatus::Shutdown
    )));
    let stored_child = harness
        .manager
        .read_thread(ReadThreadParams {
            thread_id: external_child_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external child");
    let child_items = stored_child.history.expect("history").items;
    assert!(child_items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::ExternalTerminalStatus(event))
            if event.thread_id == external_child_thread_id
                && event.status == protocol::protocol::ExternalTerminalStatus::Shutdown
    )));
}

#[tokio::test]
async fn runtime_teardown_of_external_root_does_not_persist_shutdown() {
    let harness = AgentControlHarness::new().await;
    let external_root_thread_id = ThreadId::new();
    let root_external_control = harness.manager.root_external_agent_control_for_tests();
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let root_agent_metadata = AgentMetadata {
        agent_id: Some(external_root_thread_id),
        agent_path: Some(AgentPath::root()),
        agent_nickname: Some("claude_cli".to_string()),
        agent_role: Some("claude_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    let live_thread = root_external_control
        .create_external_thread_persistence(
            &external_config,
            external_root_thread_id,
            SessionSource::Cli,
            ThreadSource::User,
            &root_agent_metadata,
        )
        .await
        .expect("create persisted external root");
    live_thread.persist().await.expect("persist external root");
    root_external_control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            external_root_thread_id,
            external_live_thread_snapshot(
                &external_config,
                external_root_thread_id,
                SessionSource::Cli,
                &root_agent_metadata,
            ),
            AgentStatus::Running,
        )
        .await;
    root_external_control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_root_thread_id,
            parent_thread_id: external_root_thread_id,
            agent_path: AgentPath::root(),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 0,
            spawn_config: Some(external_config),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("root work".to_string()),
            abort_handle: None,
        });

    let report = harness
        .manager
        .shutdown_all_threads_for_runtime_teardown_bounded(Duration::from_secs(10))
        .await;
    assert_eq!(report.completed, vec![external_root_thread_id]);
    assert!(report.submit_failed.is_empty());
    assert!(report.timed_out.is_empty());
    assert!(
        !harness
            .manager
            .has_external_root_thread(external_root_thread_id)
    );
    assert!(
        !harness
            .manager
            .is_live_thread_loaded(external_root_thread_id)
            .await
    );

    let stored = harness
        .manager
        .read_thread(ReadThreadParams {
            thread_id: external_root_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external root");
    let items = stored.history.expect("history").items;
    assert!(!items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::ExternalTerminalStatus(event))
            if event.thread_id == external_root_thread_id
                && event.status == protocol::protocol::ExternalTerminalStatus::Shutdown
    )));
}

#[tokio::test]
async fn external_tool_call_poll_external_event_times_out_without_event() {
    let harness = AgentControlHarness::new().await;
    let external_thread_id = ThreadId::new();
    let mut run = external_root_run(
        &harness.config,
        external_thread_id,
        SpawnAgentProvider::CodexCli,
    );
    let spawn_config = run.spawn_config.as_mut().expect("spawn config");
    spawn_config.default_wait_timeout_ms = 5;
    spawn_config.max_wait_timeout_ms = 5;
    harness.control.external_agents.insert_running(run);

    let result = harness
        .control
        .dispatch_external_tool_call(
            external_thread_id,
            ExternalToolCall {
                id: "poll_1".to_string(),
                tool: ExternalToolName::PollExternalEvent,
                arguments: serde_json::json!({}),
            },
        )
        .await;

    assert!(result.ok, "poll failed: {:?}", result.error);
    let result_json = result.result.expect("poll result");
    assert_eq!(result_json["timedOut"], true);
    assert_eq!(result_json["sourceHint"], serde_json::Value::Null);
    assert_eq!(result_json["event"], serde_json::Value::Null);
    assert_eq!(result_json["events"], serde_json::Value::Null);
    assert_eq!(result_json["initialTimeoutMs"], 5);
    assert_eq!(result_json["currentTimeoutMs"], 5);
    assert_eq!(result_json["hardCapTimeoutMs"], 5);
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
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
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
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
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
async fn followup_to_external_agent_enters_external_input_queue() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    harness.control.state.register_root_thread(root_thread_id);
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path.clone(),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: Some(crate::agent::external::ExternalAgentInputSink::new(
                input_tx,
            )),
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("initial".to_string()),
            abort_handle: None,
        });

    harness
        .control
        .send_inter_agent_communication(
            external_thread_id,
            InterAgentCommunication::new(
                AgentPath::root(),
                external_agent_path,
                Vec::new(),
                "please continue".to_string(),
                InterAgentOperation::FollowupTask,
            )
            .with_thread_ids(root_thread_id, external_thread_id)
            .with_trigger_turn(true),
        )
        .await
        .expect("deliver external followup");

    let queued = input_rx.recv().await.expect("external input");
    assert_eq!(queued.turn_id, None);
    assert_eq!(queued.content, "please continue");
    assert_eq!(
        harness
            .control
            .external_agents
            .get(external_thread_id)
            .expect("external run")
            .last_task_message
            .as_deref(),
        Some("please continue")
    );
}

#[tokio::test]
async fn root_external_input_records_turn_id_and_rejects_parallel_turn() {
    let harness = AgentControlHarness::new().await;
    let external_thread_id = ThreadId::new();
    let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: external_thread_id,
            agent_path: AgentPath::root(),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 0,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: Some(crate::agent::external::ExternalAgentInputSink::new(
                input_tx,
            )),
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: None,
            abort_handle: None,
        });

    let turn_id = harness
        .control
        .send_external_root_input(external_thread_id, "first".to_string())
        .await
        .expect("send first root input");
    let queued = input_rx.recv().await.expect("root external input");
    assert_eq!(queued.turn_id.as_deref(), Some(turn_id.as_str()));
    assert_eq!(queued.content, "first");

    let error = harness
        .control
        .send_external_root_input(external_thread_id, "second".to_string())
        .await
        .expect_err("parallel root turn rejected");
    assert!(matches!(
        error,
        CodexErr::UnsupportedOperation(message)
            if message == "external root thread already has an active turn"
    ));
}

struct FakeExternalStream {
    input_sink: crate::agent::external::ExternalInputSink,
    input_rx: async_channel::Receiver<String>,
    events: VecDeque<crate::agent::external::ExternalProcessEvent>,
}

impl FakeExternalStream {
    fn new(events: Vec<crate::agent::external::ExternalProcessEvent>) -> Self {
        let (input_tx, mut input_rx) = tokio::sync::mpsc::unbounded_channel();
        let (record_tx, record_rx) = async_channel::unbounded();
        tokio::spawn(async move {
            while let Some(input) = input_rx.recv().await {
                let _ = record_tx.send(input).await;
            }
        });
        Self {
            input_sink: crate::agent::external::ExternalInputSink::new(input_tx),
            input_rx: record_rx,
            events: events.into(),
        }
    }

    async fn next_input(&self) -> String {
        self.input_rx.recv().await.expect("provider input")
    }
}

impl crate::agent::external::ExternalProviderSession for FakeExternalStream {
    fn input_sink(&self) -> crate::agent::external::ExternalInputSink {
        self.input_sink.clone()
    }

    fn next_event<'a>(
        &'a mut self,
    ) -> futures::future::BoxFuture<'a, Result<crate::agent::external::ExternalProcessEvent, String>>
    {
        Box::pin(async move {
            self.events
                .pop_front()
                .ok_or_else(|| "fake stream exhausted".to_string())
        })
    }
}

struct ClosedInputExternalStream {
    input_sink: crate::agent::external::ExternalInputSink,
}

impl ClosedInputExternalStream {
    fn new() -> Self {
        let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
        drop(input_rx);
        Self {
            input_sink: crate::agent::external::ExternalInputSink::new(input_tx),
        }
    }
}

impl crate::agent::external::ExternalProviderSession for ClosedInputExternalStream {
    fn input_sink(&self) -> crate::agent::external::ExternalInputSink {
        self.input_sink.clone()
    }

    fn next_event<'a>(
        &'a mut self,
    ) -> futures::future::BoxFuture<'a, Result<crate::agent::external::ExternalProcessEvent, String>>
    {
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
async fn external_stream_loop_writes_tool_result_to_same_process() {
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
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("inspect agents".to_string()),
            abort_handle: None,
        });
    let mut stream = FakeExternalStream::new(vec![
        crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::ToolCall(ExternalToolCall {
                id: "call_1".to_string(),
                tool: ExternalToolName::ListExternalAgents,
                arguments: serde_json::json!({ "path_prefix": "/root/external" }),
            }),
        ),
        crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::Message("done after result".to_string()),
        ),
    ]);
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some("inspect agents".to_string()),
            input_rx,
            &mut stream,
        )
        .await;

    assert_eq!(
        status,
        AgentStatus::Completed(Some("done after result".to_string()))
    );
    let initial = stream.next_input().await;
    assert!(initial.contains("external-agent JSON protocol"));
    let result = stream.next_input().await;
    assert!(result.contains("external_tool_result"));
    assert!(result.contains("\"agents\""));
    assert!(result.contains("/root/external"));
}

#[tokio::test]
async fn external_stream_loop_without_initial_message_does_not_prompt_provider() {
    let harness = AgentControlHarness::new().await;
    let external_thread_id = ThreadId::new();
    let mut stream =
        FakeExternalStream::new(vec![crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::Message("idle complete".to_string()),
        )]);
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(external_thread_id, None, input_rx, &mut stream)
        .await;

    assert_eq!(
        status,
        AgentStatus::Completed(Some("idle complete".to_string()))
    );
    assert!(stream.input_rx.try_recv().is_err());
}

#[tokio::test]
async fn external_persistence_can_store_root_metadata_without_agent_path() {
    let harness = AgentControlHarness::new().await;
    let external_thread_id = ThreadId::new();
    let mut external_config = ExternalSpawnConfig::from_config(&harness.config);
    external_config.model_provider_id = "claude_cli".to_string();

    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &external_config,
            external_thread_id,
            SessionSource::Unknown,
            ThreadSource::User,
            &AgentMetadata::default(),
        )
        .await
        .expect("create persisted root external thread");
    live_thread
        .append_items(&[RolloutItem::EventMsg(EventMsg::UserMessage(
            protocol::protocol::UserMessageEvent {
                message: "metadata probe".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            },
        ))])
        .await
        .expect("append root external rollout item");
    live_thread
        .persist()
        .await
        .expect("persist root external thread");
    let stored = live_thread
        .read_thread(
            /*include_archived*/ true, /*include_history*/ false,
        )
        .await
        .expect("read stored root external thread");

    assert_eq!(stored.model_provider, "claude_cli");
    assert_eq!(stored.agent_path, None);
    assert_eq!(stored.agent_role, None);
}

#[tokio::test]
async fn external_stream_loop_persists_and_broadcasts_initial_context_prompt() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
    });
    let initial_task = "inspect the workspace";
    let expected_context_prompt =
        crate::agent::external::external_agent_context_prompt(initial_task);
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
        last_task_message: Some(initial_task.to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &ExternalSpawnConfig::from_config(&harness.config),
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path,
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some(initial_task.to_string()),
            abort_handle: None,
        });
    let mut thread_created_rx = harness.manager.subscribe_thread_created();
    let mut stream =
        FakeExternalStream::new(vec![crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::Message("finished".to_string()),
        )]);
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some(initial_task.to_string()),
            input_rx,
            &mut stream,
        )
        .await;

    assert_eq!(status, AgentStatus::Completed(Some("finished".to_string())));
    let provider_input = stream.next_input().await;
    assert_eq!(provider_input, expected_context_prompt);

    let mut broadcast_user_message = None;
    for _ in 0..6 {
        let event = timeout(Duration::from_secs(1), thread_created_rx.recv())
            .await
            .expect("live event should arrive")
            .expect("live event");
        if let thread_service_api::ThreadCreatedEvent::LiveEvent {
            thread_id,
            event: EventMsg::UserMessage(user_message),
            ..
        } = event
        {
            if thread_id == external_thread_id {
                broadcast_user_message = Some(user_message.message);
                break;
            }
        }
    }
    assert_eq!(
        broadcast_user_message.as_deref(),
        Some(expected_context_prompt.as_str())
    );

    let stored = harness
        .manager
        .read_thread(ReadThreadParams {
            thread_id: external_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external thread");
    let items = stored.history.expect("history").items;
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::UserMessage(event))
            if event.message == expected_context_prompt
    )));
    assert!(!items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::UserMessage(event))
            if event.message == initial_task
    )));
}

#[tokio::test]
async fn external_stream_loop_persists_tool_calls_as_typed_events() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
    });
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
        last_task_message: Some("inspect agents".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &ExternalSpawnConfig::from_config(&harness.config),
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path,
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("inspect agents".to_string()),
            abort_handle: None,
        });
    let mut stream = FakeExternalStream::new(vec![
        crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::ToolCall(ExternalToolCall {
                id: "call_1".to_string(),
                tool: ExternalToolName::ListExternalAgents,
                arguments: serde_json::json!({ "path_prefix": "/root/external" }),
            }),
        ),
        crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::Message("done after result".to_string()),
        ),
    ]);
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some("inspect agents".to_string()),
            input_rx,
            &mut stream,
        )
        .await;

    assert_eq!(
        status,
        AgentStatus::Completed(Some("done after result".to_string()))
    );
    let _initial = stream.next_input().await;
    let result = stream.next_input().await;
    assert!(result.contains("external_tool_result"));
    assert!(result.contains("\"agents\""));

    let stored = harness
        .manager
        .read_thread(ReadThreadParams {
            thread_id: external_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external thread");
    let items = stored.history.expect("history").items;
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::ExternalToolCallStarted(event))
            if event.id == "call_1"
                && event.tool == "list_external_agents"
                && event.arguments == serde_json::json!({ "path_prefix": "/root/external" })
                && event.status == protocol::protocol::ExternalToolCallStatus::InProgress
                && event.output.is_none()
    )));
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::ExternalToolCallCompleted(event))
            if event.id == "call_1"
                && event.tool == "list_external_agents"
                && event.status == protocol::protocol::ExternalToolCallStatus::Completed
                && event.output.as_ref().is_some_and(|output| output["agents"].is_array())
    )));
    assert!(!items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::AgentMessage(event))
            if event.message.contains("external_tool_call")
    )));
    assert!(!items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::UserMessage(event))
            if event.message.contains("external_tool_result")
    )));
}

#[tokio::test]
async fn external_stream_loop_persists_tool_call_errors_as_failed_typed_events() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
    });
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
        last_task_message: Some("recover malformed call".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &ExternalSpawnConfig::from_config(&harness.config),
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path,
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("recover malformed call".to_string()),
            abort_handle: None,
        });
    let mut stream = FakeExternalStream::new(vec![
        crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::ToolCallError(ExternalToolResult::error(
                "call_bad",
                "invalid_tool_call",
                "failed to parse external tool call",
            )),
        ),
        crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::Message("recovered".to_string()),
        ),
    ]);
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some("recover malformed call".to_string()),
            input_rx,
            &mut stream,
        )
        .await;

    assert_eq!(
        status,
        AgentStatus::Completed(Some("recovered".to_string()))
    );
    let _initial = stream.next_input().await;
    let error = stream.next_input().await;
    assert!(error.contains("external_tool_result"));
    assert!(error.contains("invalid_tool_call"));

    let stored = harness
        .manager
        .read_thread(ReadThreadParams {
            thread_id: external_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external thread");
    let items = stored.history.expect("history").items;
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::ExternalToolCallCompleted(event))
            if event.id == "call_bad"
                && event.tool == "external_tool"
                && event.arguments.is_null()
                && event.status == protocol::protocol::ExternalToolCallStatus::Failed
                && event.output.as_ref().is_some_and(|output| {
                    output["error"]["code"] == "invalid_tool_call"
                        && output["error"]["message"]
                            .as_str()
                            .is_some_and(|message| {
                                message.contains("failed to parse external tool call")
                            })
                })
    )));
    assert!(!items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::UserMessage(event))
            if event.message.contains("external_tool_result")
    )));
}

#[tokio::test]
async fn external_stream_loop_persists_closed_stdin_as_terminal_error() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let external_agent_path =
        AgentPath::try_from("/root/external_stdin_closed").expect("agent path");
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
    });
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
        last_task_message: Some("stdin will close".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &ExternalSpawnConfig::from_config(&harness.config),
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: AgentPath::try_from("/root/external_stdin_closed").expect("agent path"),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("stdin will close".to_string()),
            abort_handle: None,
        });
    let mut stream = ClosedInputExternalStream::new();
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some("stdin will close".to_string()),
            input_rx,
            &mut stream,
        )
        .await;

    assert_eq!(
        status,
        AgentStatus::Errored("external provider stdin is closed".to_string())
    );
    let stored = harness
        .manager
        .read_thread(ReadThreadParams {
            thread_id: external_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external thread");
    let items = stored.history.expect("history").items;
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::ExternalTerminalStatus(event))
            if event.status == protocol::protocol::ExternalTerminalStatus::Errored
                && event.message.as_deref() == Some("external provider stdin is closed")
    )));

    let (_restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let agents = restarted_control
        .list_agents(
            root_thread_id,
            &SessionSource::Exec,
            Some("external_stdin_closed"),
        )
        .await
        .expect("list persisted external agents");
    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0].lifecycle_status,
        ThreadLifecycleStatus::errored(Some("external provider stdin is closed".to_string()))
    );
}

#[tokio::test]
async fn external_stream_loop_handles_multiple_tool_calls_without_iteration_cap() {
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
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("inspect agents".to_string()),
            abort_handle: None,
        });
    let events = (0..10)
        .map(|index| {
            crate::agent::external::ExternalProcessEvent::Cli(
                crate::agent::external::ExternalCliEvent::ToolCall(ExternalToolCall {
                    id: format!("call_{index}"),
                    tool: ExternalToolName::ListExternalAgents,
                    arguments: serde_json::json!({}),
                }),
            )
        })
        .chain(std::iter::once(
            crate::agent::external::ExternalProcessEvent::Cli(
                crate::agent::external::ExternalCliEvent::Message("finished".to_string()),
            ),
        ))
        .collect::<Vec<_>>();
    let mut stream = FakeExternalStream::new(events);
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some("inspect agents".to_string()),
            input_rx,
            &mut stream,
        )
        .await;

    assert_eq!(status, AgentStatus::Completed(Some("finished".to_string())));
    let _initial = stream.next_input().await;
    for index in 0..10 {
        let result = stream.next_input().await;
        assert!(result.contains(&format!("\"id\":\"call_{index}\"")));
        assert!(result.contains("external_tool_result"));
    }
}

#[tokio::test]
async fn external_stream_loop_writes_malformed_tool_error_and_finishes() {
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
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: None,
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("recover malformed call".to_string()),
            abort_handle: None,
        });
    let mut stream = FakeExternalStream::new(vec![
        crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::ToolCallError(ExternalToolResult::error(
                "call_bad",
                "invalid_tool_call",
                "failed to parse external tool call",
            )),
        ),
        crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::Message("recovered".to_string()),
        ),
    ]);
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some("recover malformed call".to_string()),
            input_rx,
            &mut stream,
        )
        .await;

    assert_eq!(
        status,
        AgentStatus::Completed(Some("recovered".to_string()))
    );
    let _initial = stream.next_input().await;
    let error = stream.next_input().await;
    assert_eq!(error.matches("\"id\":\"call_bad\"").count(), 1);
    assert!(error.contains("invalid_tool_call"));
}

#[tokio::test]
async fn external_stream_loop_delivers_followup_input_to_same_process() {
    let harness = AgentControlHarness::new().await;
    let external_thread_id = ThreadId::new();
    let mut stream =
        FakeExternalStream::new(vec![crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::Message("done".to_string()),
        )]);
    let (input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();
    input_tx
        .send(crate::agent::external::ExternalAgentInput {
            turn_id: None,
            content: "follow up while running".to_string(),
        })
        .expect("send followup");

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some("initial task".to_string()),
            input_rx,
            &mut stream,
        )
        .await;

    assert_eq!(status, AgentStatus::Completed(Some("done".to_string())));
    let initial = stream.next_input().await;
    let followup = stream.next_input().await;
    assert!(initial.contains("initial task"));
    assert_eq!(followup, "follow up while running");
}

#[tokio::test]
async fn external_stream_loop_returns_stdin_error() {
    let harness = AgentControlHarness::new().await;
    let external_thread_id = ThreadId::new();
    let mut stream = FakeExternalStream::new(vec![
        crate::agent::external::ExternalProcessEvent::StdinError("broken pipe".to_string()),
    ]);
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some("initial task".to_string()),
            input_rx,
            &mut stream,
        )
        .await;

    assert_eq!(status, AgentStatus::Errored("broken pipe".to_string()));
}

#[tokio::test]
async fn external_completed_agent_is_listed_from_persisted_thread_after_restart() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
    });
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
        last_task_message: Some("persist me".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &ExternalSpawnConfig::from_config(&harness.config),
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path.clone(),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("persist me".to_string()),
            abort_handle: None,
        });
    let mut stream =
        FakeExternalStream::new(vec![crate::agent::external::ExternalProcessEvent::Cli(
            crate::agent::external::ExternalCliEvent::Message("persisted done".to_string()),
        )]);
    let (_input_tx, input_rx) = tokio::sync::mpsc::unbounded_channel();

    let status = harness
        .control
        .run_external_agent_stream_loop(
            external_thread_id,
            Some("persist me".to_string()),
            input_rx,
            &mut stream,
        )
        .await;
    assert_eq!(
        status,
        AgentStatus::Completed(Some("persisted done".to_string()))
    );

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let agents = restarted_control
        .list_agents(root_thread_id, &SessionSource::Exec, Some("external"))
        .await
        .expect("list persisted external agents");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_name, "/root/external");
    assert_eq!(agents[0].agent_nickname.as_deref(), Some("worker"));
    assert_eq!(agents[0].agent_role.as_deref(), Some("claude_cli"));
    assert_eq!(
        agents[0].lifecycle_status,
        ThreadLifecycleStatus::completed(Some("persisted done".to_string()))
    );

    let directory = restarted_control
        .list_agent_directory(AgentDirectoryListRequest {
            current_thread_id: root_thread_id,
            current_session_source: SessionSource::Exec,
            path_prefix: Some("external".to_string()),
        })
        .await
        .expect("list persisted external directory");
    assert_eq!(directory.entries.len(), 1);
    assert_eq!(directory.entries[0].thread_id, external_thread_id);
    assert_eq!(directory.entries[0].parent_thread_id, Some(root_thread_id));
    assert_eq!(directory.entries[0].depth, Some(1));
    assert_eq!(
        directory.entries[0].source,
        AgentDirectoryEntrySource::Persisted
    );

    let stored = restarted_manager
        .read_thread(ReadThreadParams {
            thread_id: external_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external thread");
    let items = stored.history.expect("history").items;
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::UserMessage(event)) if event.message == "persist me"
    )));
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::AgentMessage(event)) if event.message == "persisted done"
    )));
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::TurnComplete(event))
            if event.last_agent_message.as_deref() == Some("persisted done")
    )));
}

#[tokio::test]
async fn external_running_agent_without_live_process_is_interrupted_after_restart() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external").expect("agent path");
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
    });
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
        last_task_message: Some("unfinished".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &ExternalSpawnConfig::from_config(&harness.config),
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path.clone(),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("unfinished".to_string()),
            abort_handle: None,
        });

    harness
        .control
        .persist_external_terminal_status(external_thread_id, &AgentStatus::Running)
        .await;
    harness
        .control
        .persist_external_user_message(external_thread_id, None, "unfinished")
        .await;

    let (_restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let agents = restarted_control
        .list_agents(root_thread_id, &SessionSource::Exec, Some("external"))
        .await
        .expect("list persisted external agents");
    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0].lifecycle_status,
        ThreadLifecycleStatus::Final {
            result: protocol::protocol::ThreadLifecycleFinalStatus::Interrupted,
        }
    );
}

#[tokio::test]
async fn external_errored_agent_is_listed_from_persisted_thread_after_restart() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external_errored").expect("agent path");
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
    });
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
        last_task_message: Some("fail me".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &ExternalSpawnConfig::from_config(&harness.config),
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path,
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("fail me".to_string()),
            abort_handle: None,
        });

    let large_error = "provider failed ".repeat(2000);
    harness
        .control
        .persist_external_terminal_status(
            external_thread_id,
            &AgentStatus::Errored(large_error.clone()),
        )
        .await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let agents = restarted_control
        .list_agents(
            root_thread_id,
            &SessionSource::Exec,
            Some("external_errored"),
        )
        .await
        .expect("list persisted external agents");
    assert_eq!(agents.len(), 1);
    let ThreadLifecycleStatus::Final {
        result: protocol::protocol::ThreadLifecycleFinalStatus::Errored { message },
    } = &agents[0].lifecycle_status
    else {
        panic!(
            "expected errored lifecycle, got {:?}",
            agents[0].lifecycle_status
        );
    };
    let message = message.as_ref().expect("bounded error message");
    assert!(message.contains("provider failed"));
    assert!(message.len() < large_error.len());

    let stored = restarted_manager
        .read_thread(ReadThreadParams {
            thread_id: external_thread_id,
            include_archived: true,
            include_history: true,
        })
        .await
        .expect("read persisted external thread");
    let items = stored.history.expect("history").items;
    assert!(items.iter().any(|item| matches!(
        item,
        RolloutItem::EventMsg(EventMsg::ExternalTerminalStatus(event))
            if event.status == protocol::protocol::ExternalTerminalStatus::Errored
                && event.message.as_ref().is_some_and(|message| message.len() < large_error.len())
    )));
}

#[tokio::test]
async fn external_shutdown_agent_is_listed_from_open_persisted_thread_after_restart() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external_shutdown").expect("agent path");
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
    });
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
        last_task_message: Some("shutdown me".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &ExternalSpawnConfig::from_config(&harness.config),
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: AgentPath::try_from("/root/external_shutdown").expect("agent path"),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("shutdown me".to_string()),
            abort_handle: None,
        });

    harness
        .control
        .persist_external_terminal_status(external_thread_id, &AgentStatus::Shutdown)
        .await;

    let (_restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let agents = restarted_control
        .list_agents(
            root_thread_id,
            &SessionSource::Exec,
            Some("external_shutdown"),
        )
        .await
        .expect("list persisted external agents");
    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0].lifecycle_status,
        ThreadLifecycleStatus::Final {
            result: protocol::protocol::ThreadLifecycleFinalStatus::Shutdown,
        }
    );
}

#[tokio::test]
async fn external_completed_agent_path_resolves_from_persisted_thread_after_restart() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let (external_thread_id, external_agent_path) = persist_external_child_for_restart(
        &harness,
        root_thread_id,
        "/root/external_reference",
        AgentStatus::Completed(Some("persisted done".to_string())),
    )
    .await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let resolved_thread_id = restarted_control
        .resolve_agent_reference(
            root_thread_id,
            &SessionSource::Exec,
            Some(harness.config.clone()),
            external_agent_path.as_str(),
        )
        .await
        .expect("terminal persisted external agent path should resolve after restart");

    assert_eq!(resolved_thread_id, external_thread_id);
    assert!(
        restarted_manager
            .get_thread(external_thread_id)
            .await
            .is_err(),
        "external path resolution should not create a native live thread",
    );
    assert_eq!(
        restarted_control
            .get_agent_metadata(external_thread_id)
            .and_then(|metadata| metadata.agent_path),
        Some(external_agent_path.clone()),
        "external path resolution should register persisted metadata for later references",
    );

    let resolution = restarted_control
        .resolve_agent_reference_in_directory(AgentReferenceResolutionRequest {
            current_thread_id: root_thread_id,
            current_session_source: SessionSource::Exec,
            agent_reference: external_agent_path.to_string(),
        })
        .await
        .expect("directory resolution should load persisted external facts");
    assert_matches!(
        resolution,
        AgentReferenceResolution::PersistedExternalReadOnly {
            thread_id,
            agent_path,
        } if thread_id == external_thread_id && agent_path == external_agent_path.to_string()
    );
}

#[tokio::test]
async fn external_tool_followup_and_close_reject_persisted_completed_external_as_read_only() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let (external_thread_id, external_agent_path) = persist_external_child_for_restart(
        &harness,
        root_thread_id,
        "/root/external_completed_tool_reference",
        AgentStatus::Completed(Some("persisted done".to_string())),
    )
    .await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    restarted_control
        .external_agents
        .insert_running(external_root_run(
            &harness.config,
            root_thread_id,
            SpawnAgentProvider::ClaudeCli,
        ));

    for (call_id, tool, arguments) in [
        (
            "follow_persisted_completed",
            ExternalToolName::FollowupExternalTask,
            serde_json::json!({
                "target": external_agent_path.to_string(),
                "message": "should not deliver"
            }),
        ),
        (
            "close_persisted_completed",
            ExternalToolName::CloseExternalAgent,
            serde_json::json!({ "target": external_agent_path.to_string() }),
        ),
    ] {
        let result = restarted_control
            .dispatch_external_tool_call(
                root_thread_id,
                ExternalToolCall {
                    id: call_id.to_string(),
                    tool,
                    arguments,
                },
            )
            .await;
        assert!(!result.ok, "persisted target should reject {call_id}");
        assert!(result.error.as_ref().is_some_and(|error| {
            error.code == "tool_error"
                && error.message.contains("persisted and read-only")
                && error.message.contains(external_agent_path.as_str())
        }));
    }

    assert!(
        restarted_manager
            .get_thread(external_thread_id)
            .await
            .is_err(),
        "external tool target resolution should not create a native live thread",
    );
}

#[tokio::test]
async fn external_running_agent_path_rejects_after_restart_without_live_process() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let (external_thread_id, external_agent_path) = persist_external_child_for_restart(
        &harness,
        root_thread_id,
        "/root/external_running_reference",
        AgentStatus::Running,
    )
    .await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let err = restarted_control
        .resolve_agent_reference(
            root_thread_id,
            &SessionSource::Exec,
            Some(harness.config.clone()),
            external_agent_path.as_str(),
        )
        .await
        .expect_err("running external agent without live process should not reconnect");

    let message = err.to_string();
    assert!(
        message.contains("interrupted") && message.contains("cannot reconnect"),
        "unexpected error: {err}",
    );
    assert!(
        restarted_manager
            .get_thread(external_thread_id)
            .await
            .is_err(),
        "failed external path resolution should not create a native live thread",
    );
    assert!(
        restarted_control
            .get_agent_metadata(external_thread_id)
            .is_none(),
        "non-reconnectable external agent should not be registered as live metadata",
    );
}

#[tokio::test]
async fn external_interrupted_agent_is_listed_as_interrupted_after_restart() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let (external_thread_id, external_agent_path) = persist_external_child_for_restart(
        &harness,
        root_thread_id,
        "/root/external_interrupted_reference",
        AgentStatus::Interrupted,
    )
    .await;

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let agents = restarted_control
        .list_agents(
            root_thread_id,
            &SessionSource::Exec,
            Some("external_interrupted_reference"),
        )
        .await
        .expect("list persisted external agents");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].agent_name, external_agent_path.to_string());
    assert_eq!(
        agents[0].lifecycle_status,
        ThreadLifecycleStatus::Final {
            result: protocol::protocol::ThreadLifecycleFinalStatus::Interrupted,
        }
    );

    assert!(
        restarted_manager
            .get_thread(external_thread_id)
            .await
            .is_err(),
        "interrupted external path should stay read-only after restart",
    );

    let err = restarted_control
        .resolve_agent_reference(
            root_thread_id,
            &SessionSource::Exec,
            Some(harness.config.clone()),
            external_agent_path.as_str(),
        )
        .await
        .expect_err("interrupted external agent should not resolve as terminal read-only");
    let message = err.to_string();
    assert!(
        message.contains("interrupted") && message.contains("cannot reconnect"),
        "unexpected error: {err}",
    );
    assert!(
        restarted_manager
            .get_thread(external_thread_id)
            .await
            .is_err(),
        "failed interrupted path resolution should not create a native live thread",
    );
    assert!(
        restarted_control
            .get_agent_metadata(external_thread_id)
            .is_none(),
        "non-reconnectable interrupted external agent should not register metadata",
    );
}

#[tokio::test]
async fn external_running_agent_path_with_descriptor_rejects_as_restore_disabled_after_restart() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let (external_thread_id, external_agent_path) =
        persist_external_child_for_restart_with_provider(
            &harness,
            root_thread_id,
            "/root/external_running_opencode_reference",
            SpawnAgentProvider::Opencode,
            AgentStatus::Running,
        )
        .await;
    harness
        .control
        .persist_external_reconnect_descriptor(
            external_thread_id,
            opencode_reconnect_descriptor("opencode-session-456"),
        )
        .await
        .expect("persist reconnect descriptor");

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let agents = restarted_control
        .list_agents(
            root_thread_id,
            &SessionSource::Exec,
            Some("external_running_opencode_reference"),
        )
        .await
        .expect("list persisted external agents");
    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0].lifecycle_status,
        ThreadLifecycleStatus::Final {
            result: protocol::protocol::ThreadLifecycleFinalStatus::Interrupted,
        }
    );

    let err = restarted_control
        .resolve_agent_reference(
            root_thread_id,
            &SessionSource::Exec,
            Some(harness.config.clone()),
            external_agent_path.as_str(),
        )
        .await
        .expect_err("descriptor-present external agent should still not reconnect");

    let message = err.to_string();
    assert!(
        message.contains("reconnect descriptor is present")
            && message.contains("external live restore is disabled")
            && message.contains("transient opencode serve")
            && message.contains("no durable endpoint")
            && message.contains("status/watch ownership")
            && message.contains("wait cursor"),
        "unexpected error: {err}",
    );
    assert!(
        restarted_manager
            .get_thread(external_thread_id)
            .await
            .is_err(),
        "failed external path resolution should not create a native live thread",
    );
    assert!(
        restarted_control
            .get_agent_metadata(external_thread_id)
            .is_none(),
        "restore-disabled external agent should not be registered as live metadata",
    );
}

#[tokio::test]
async fn external_tool_followup_rejects_restore_disabled_external_after_restart() {
    let harness = AgentControlHarness::new().await;
    let root_thread_id = ThreadId::new();
    let (external_thread_id, external_agent_path) =
        persist_external_child_for_restart_with_provider(
            &harness,
            root_thread_id,
            "/root/external_restore_disabled_tool_reference",
            SpawnAgentProvider::Opencode,
            AgentStatus::Running,
        )
        .await;
    harness
        .control
        .persist_external_reconnect_descriptor(
            external_thread_id,
            opencode_reconnect_descriptor("opencode-session-tool"),
        )
        .await
        .expect("persist reconnect descriptor");

    let (restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    restarted_control
        .external_agents
        .insert_running(external_root_run(
            &harness.config,
            root_thread_id,
            SpawnAgentProvider::ClaudeCli,
        ));

    let result = restarted_control
        .dispatch_external_tool_call(
            root_thread_id,
            ExternalToolCall {
                id: "follow_restore_disabled".to_string(),
                tool: ExternalToolName::FollowupExternalTask,
                arguments: serde_json::json!({
                    "target": external_agent_path.to_string(),
                    "message": "should not reconnect"
                }),
            },
        )
        .await;

    assert!(!result.ok);
    assert!(result.error.as_ref().is_some_and(|error| {
        error.code == "tool_error"
            && error.message.contains("reconnect descriptor is present")
            && error.message.contains("external live restore is disabled")
            && error.message.contains("transient opencode serve")
    }));
    assert!(
        restarted_manager
            .get_thread(external_thread_id)
            .await
            .is_err(),
        "restore-disabled external tool resolution should not create a native live thread",
    );
    assert!(
        restarted_control
            .get_agent_metadata(external_thread_id)
            .is_none(),
        "restore-disabled external tool target should not register live metadata",
    );
}

#[tokio::test]
async fn external_closed_agent_is_not_restored_after_restart() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let external_thread_id = ThreadId::new();
    let external_agent_path = AgentPath::try_from("/root/external_closed").expect("agent path");
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: root_thread_id,
        depth: 1,
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
    });
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(external_agent_path.clone()),
        agent_nickname: Some("worker".to_string()),
        agent_role: Some("claude_cli".to_string()),
        last_task_message: Some("close me".to_string()),
        counted: true,
        ..Default::default()
    };
    let live_thread = harness
        .control
        .create_external_thread_persistence(
            &external_config,
            external_thread_id,
            session_source.clone(),
            ThreadSource::Subagent,
            &agent_metadata,
        )
        .await
        .expect("create persisted external thread");
    harness
        .control
        .persist_thread_spawn_edge_for_source(external_thread_id, Some(&session_source))
        .await;
    harness
        .control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            external_thread_id,
            external_live_thread_snapshot(
                &external_config,
                external_thread_id,
                session_source,
                &agent_metadata,
            ),
            AgentStatus::Running,
        )
        .await;
    harness
        .control
        .external_agents
        .insert_running(ExternalAgentRun {
            thread_id: external_thread_id,
            parent_thread_id: root_thread_id,
            agent_path: external_agent_path.clone(),
            provider: SpawnAgentProvider::ClaudeCli,
            depth: 1,
            spawn_config: Some(external_config),
            input_sink: None,
            live_thread: Some(live_thread),
            status: AgentStatus::Running,
            active_turn_id: None,
            last_task_message: Some("close me".to_string()),
            abort_handle: None,
        });

    harness
        .control
        .close_agent(external_thread_id)
        .await
        .expect("close external agent");

    let (_restarted_manager, restarted_control) = harness.restarted_manager_and_control();
    let agents = restarted_control
        .list_agents(root_thread_id, &SessionSource::Exec, None)
        .await
        .expect("list agents after restart");
    assert!(
        agents
            .iter()
            .all(|agent| agent.agent_name != external_agent_path.as_str()),
        "closed external agent should not be restored into default list: {agents:?}",
    );

    let err = restarted_control
        .resolve_agent_reference(
            root_thread_id,
            &SessionSource::Exec,
            Some(harness.config.clone()),
            external_agent_path.as_str(),
        )
        .await
        .expect_err("closed external agent path should not resolve after restart");
    assert!(
        err.to_string().contains("not found"),
        "unexpected error: {err}",
    );
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
async fn on_event_updates_status_from_external_terminal_status() {
    let thread_id = ThreadId::new();
    let errored = agent_status_from_event(&EventMsg::ExternalTerminalStatus(
        protocol::protocol::ExternalTerminalStatusEvent {
            thread_id,
            turn_id: "turn-1".to_string(),
            status: protocol::protocol::ExternalTerminalStatus::Errored,
            message: Some("provider failed".to_string()),
            terminal_at_ms: 1,
        },
    ));
    assert_eq!(
        errored,
        Some(AgentStatus::Errored("provider failed".to_string()))
    );

    let shutdown = agent_status_from_event(&EventMsg::ExternalTerminalStatus(
        protocol::protocol::ExternalTerminalStatusEvent {
            thread_id,
            turn_id: "turn-1".to_string(),
            status: protocol::protocol::ExternalTerminalStatus::Shutdown,
            message: None,
            terminal_at_ms: 2,
        },
    ));
    assert_eq!(shutdown, Some(AgentStatus::Shutdown));
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
async fn subscribe_status_observes_external_live_record_updates() {
    let harness = AgentControlHarness::new().await;
    let external_thread_id = ThreadId::new();
    let child_agent_path = AgentPath::try_from("/root/external_subscribe").expect("agent path");
    let session_source = external_session_source_for(
        ThreadId::new(),
        1,
        child_agent_path.clone(),
        SpawnAgentProvider::CodexCli,
    );
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let agent_metadata = AgentMetadata {
        agent_id: Some(external_thread_id),
        agent_path: Some(child_agent_path),
        agent_nickname: Some("codex_cli".to_string()),
        agent_role: Some("codex_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    let manager = harness
        .control
        .upgrade()
        .expect("manager should be available");
    manager
        .register_external_live_thread_snapshot(
            external_thread_id,
            external_live_thread_snapshot(
                &external_config,
                external_thread_id,
                session_source,
                &agent_metadata,
            ),
            AgentStatus::Running,
        )
        .await;

    let mut status_rx = harness
        .control
        .subscribe_status(external_thread_id)
        .await
        .expect("subscribe_status should succeed for external live record");
    assert_eq!(status_rx.borrow().clone(), AgentStatus::Running);

    manager
        .update_external_live_thread_status(
            external_thread_id,
            AgentStatus::Completed(Some("done".to_string())),
        )
        .await;

    status_rx
        .changed()
        .await
        .expect("external status update should notify receiver");
    assert_eq!(
        status_rx.borrow().clone(),
        AgentStatus::Completed(Some("done".to_string()))
    );
}

#[tokio::test]
async fn subscribe_status_prefers_native_when_external_record_has_same_id() {
    let harness = AgentControlHarness::new().await;
    let (thread_id, _thread) = harness.start_thread().await;
    let child_agent_path =
        AgentPath::try_from("/root/external_subscribe_same_id").expect("agent path");
    let session_source = external_session_source_for(
        ThreadId::new(),
        1,
        child_agent_path.clone(),
        SpawnAgentProvider::CodexCli,
    );
    let external_config = ExternalSpawnConfig::from_config(&harness.config);
    let agent_metadata = AgentMetadata {
        agent_id: Some(thread_id),
        agent_path: Some(child_agent_path),
        agent_nickname: Some("codex_cli".to_string()),
        agent_role: Some("codex_cli".to_string()),
        counted: false,
        ..Default::default()
    };
    harness
        .control
        .upgrade()
        .expect("manager should be available")
        .register_external_live_thread_snapshot(
            thread_id,
            external_live_thread_snapshot(
                &external_config,
                thread_id,
                session_source,
                &agent_metadata,
            ),
            AgentStatus::Shutdown,
        )
        .await;

    let status_rx = harness
        .control
        .subscribe_status(thread_id)
        .await
        .expect("subscribe_status should use native thread");
    assert_eq!(status_rx.borrow().clone(), AgentStatus::PendingInit);
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
async fn list_agent_subtree_thread_ids_traverses_mixed_persisted_edge_statuses() {
    let harness = AgentControlHarness::new().await;
    let state_db = harness.state_db.as_ref().expect("state db");
    let parent_thread_id =
        ThreadId::from_string("00000000-0000-0000-0000-000000000801").expect("parent thread id");
    let child_thread_id =
        ThreadId::from_string("00000000-0000-0000-0000-000000000802").expect("child thread id");
    let grandchild_thread_id = ThreadId::from_string("00000000-0000-0000-0000-000000000803")
        .expect("grandchild thread id");

    state_db
        .upsert_thread_spawn_edge(
            parent_thread_id,
            child_thread_id,
            DirectionalThreadSpawnEdgeStatus::Closed,
        )
        .await
        .expect("closed child edge should persist");
    state_db
        .upsert_thread_spawn_edge(
            child_thread_id,
            grandchild_thread_id,
            DirectionalThreadSpawnEdgeStatus::Open,
        )
        .await
        .expect("open grandchild edge should persist");

    let subtree_thread_ids = harness
        .manager
        .list_agent_subtree_thread_ids(parent_thread_id)
        .await
        .expect("subtree thread ids should load");

    assert_eq!(
        subtree_thread_ids,
        vec![parent_thread_id, child_thread_id, grandchild_thread_id]
    );
}

#[tokio::test]
async fn list_agent_directory_includes_source_parent_and_depth_facts() {
    let mut harness = AgentControlHarness::new().await;
    harness.config.agent_max_threads = Some(4);
    let (parent_thread_id, _parent_thread) = harness.start_thread().await;
    let worker_path = AgentPath::root().join("worker").expect("worker path");
    let worker_child_path = worker_path.join("child").expect("worker child path");

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
                agent_path: Some(worker_child_path.clone()),
                agent_nickname: None,
                agent_role: Some("worker".to_string()),
            })),
        )
        .await
        .expect("worker child spawn should succeed");

    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[worker_thread_id])
        .await;
    wait_for_live_thread_spawn_children(
        &harness.control,
        worker_thread_id,
        &[worker_child_thread_id],
    )
    .await;

    let directory = harness
        .control
        .list_agent_directory(AgentDirectoryListRequest {
            current_thread_id: parent_thread_id,
            current_session_source: SessionSource::Exec,
            path_prefix: None,
        })
        .await
        .expect("directory should list live tree facts");

    let worker = directory
        .entries
        .iter()
        .find(|entry| entry.agent_path.as_deref() == Some(worker_path.as_str()))
        .expect("worker directory entry should exist");
    assert_eq!(worker.thread_id, worker_thread_id);
    assert_eq!(worker.parent_thread_id, Some(parent_thread_id));
    assert_eq!(worker.depth, Some(1));
    assert_eq!(worker.source, AgentDirectoryEntrySource::NativeLive);

    let worker_child = directory
        .entries
        .iter()
        .find(|entry| entry.agent_path.as_deref() == Some(worker_child_path.as_str()))
        .expect("worker child directory entry should exist");
    assert_eq!(worker_child.thread_id, worker_child_thread_id);
    assert_eq!(worker_child.parent_thread_id, Some(worker_thread_id));
    assert_eq!(worker_child.depth, Some(2));
    assert_eq!(worker_child.source, AgentDirectoryEntrySource::NativeLive);
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
async fn external_close_parent_removes_live_external_descendants() {
    let harness = AgentControlHarness::new().await;
    let (root_thread_id, _root_thread) = harness.start_thread().await;
    let parent_thread_id = ThreadId::new();
    let child_thread_id = ThreadId::new();
    let grandchild_thread_id = ThreadId::new();

    for (thread_id, parent_id, depth, path) in [
        (parent_thread_id, root_thread_id, 1, "/root/external_parent"),
        (
            child_thread_id,
            parent_thread_id,
            2,
            "/root/external_parent/child",
        ),
        (
            grandchild_thread_id,
            child_thread_id,
            3,
            "/root/external_parent/child/grandchild",
        ),
    ] {
        let agent_path = AgentPath::try_from(path).expect("agent path");
        let session_source = external_session_source_for(
            parent_id,
            depth,
            agent_path.clone(),
            SpawnAgentProvider::ClaudeCli,
        );
        let external_config = ExternalSpawnConfig::from_config(&harness.config);
        let agent_metadata = AgentMetadata {
            agent_id: Some(thread_id),
            agent_path: Some(agent_path.clone()),
            agent_nickname: Some("claude_cli".to_string()),
            agent_role: Some("claude_cli".to_string()),
            counted: true,
            ..Default::default()
        };
        harness
            .control
            .upgrade()
            .expect("manager should be available")
            .register_external_live_thread_snapshot(
                thread_id,
                external_live_thread_snapshot(
                    &external_config,
                    thread_id,
                    session_source.clone(),
                    &agent_metadata,
                ),
                AgentStatus::Running,
            )
            .await;
        harness
            .control
            .persist_thread_spawn_edge_for_source(thread_id, Some(&session_source))
            .await;
        harness
            .control
            .external_agents
            .insert_running(ExternalAgentRun {
                thread_id,
                parent_thread_id: parent_id,
                agent_path,
                provider: SpawnAgentProvider::ClaudeCli,
                depth,
                spawn_config: Some(ExternalSpawnConfig::from_config(&harness.config)),
                input_sink: None,
                live_thread: None,
                status: AgentStatus::Running,
                active_turn_id: None,
                last_task_message: None,
                abort_handle: None,
            });
    }

    wait_for_live_thread_spawn_children(&harness.control, root_thread_id, &[parent_thread_id])
        .await;
    wait_for_live_thread_spawn_children(&harness.control, parent_thread_id, &[child_thread_id])
        .await;
    wait_for_live_thread_spawn_children(&harness.control, child_thread_id, &[grandchild_thread_id])
        .await;

    let _ = harness
        .control
        .close_agent(parent_thread_id)
        .await
        .expect("external parent close should succeed");

    for thread_id in [parent_thread_id, child_thread_id, grandchild_thread_id] {
        assert!(
            harness.control.external_agents.get(thread_id).is_none(),
            "external live run should be removed for {thread_id}",
        );
        assert_eq!(
            harness.control.get_status(thread_id).await,
            AgentStatus::NotFound
        );
    }
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
