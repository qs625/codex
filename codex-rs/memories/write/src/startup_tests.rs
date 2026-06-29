use crate::MemoryConsolidationAgent;
use crate::MemoryRuntimeFuture;
use crate::MemoryStartupRuntime;
use crate::MemoryStartupSettings;
use crate::StageOnePromptRequest;
use crate::StageOneRequestContext;
use crate::start_memories_startup_task;
use codex_api::ResponseEvent;
use codex_features::Feature;
use codex_git_baseline::diff_since_latest_init;
use codex_git_baseline::reset_git_repository;
use codex_login::AuthManager;
use codex_login::model_provider_auth_manager;
use codex_otel::SessionTelemetry;
use codex_protocol::ThreadId;
use codex_protocol::config_types::ServiceTier;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::user_input::UserInput;
use codex_rollout_trace::InferenceTraceContext;
use codex_session_telemetry_api::SessionTelemetry as SessionTelemetryTrait;
use thread_service::CodexThread;
use thread_service::ModelClient;
use thread_service::Prompt;
use thread_service::ThreadConfigSnapshot;
use thread_service::ThreadService;
use thread_service::config::Config;
use thread_service::resolve_installation_id;
use core_test_support::responses::ResponseMock;
use core_test_support::responses::ResponsesRequest;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::TestCodex;
use core_test_support::test_codex::test_codex;
use core_test_support::wait_for_event;
use futures::StreamExt;
use pretty_assertions::assert_eq;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;
use tokio::sync::Mutex;
use tokio::sync::watch;
use tokio::time::Duration;
use tokio::time::Instant;

fn run_startup_test<F>(future: F) -> anyhow::Result<()>
where
    F: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    thread::Builder::new()
        .name("memories-startup-test".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("startup test runtime should build")
                .block_on(future)
        })?
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

#[test]
fn memories_startup_phase2_tracks_workspace_diff_across_runs() -> anyhow::Result<()> {
    run_startup_test(memories_startup_phase2_tracks_workspace_diff_across_runs_impl())
}

async fn memories_startup_phase2_tracks_workspace_diff_across_runs_impl() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let db = init_state_db(&home).await?;
    let memory_root = home.path().join("memories");

    let now = chrono::Utc::now();
    let _thread_a = seed_stage1_output(
        db.as_ref(),
        home.path(),
        now - chrono::Duration::hours(2),
        "raw memory A",
        "rollout summary A",
        "rollout-a",
    )
    .await?;

    let rollout_summaries_root = memory_root.join("rollout_summaries");
    tokio::fs::create_dir_all(&rollout_summaries_root).await?;
    tokio::fs::write(
        memory_root.join("raw_memories.md"),
        "# Raw Memories\n\nraw memory A\n",
    )
    .await?;
    tokio::fs::write(
        rollout_summaries_root.join("rollout-a.md"),
        "git_branch: branch-rollout-a\n\nrollout summary A\n",
    )
    .await?;
    reset_git_repository(&memory_root).await?;

    let _thread_b = seed_stage1_output(
        db.as_ref(),
        home.path(),
        now - chrono::Duration::hours(1),
        "raw memory B",
        "rollout summary B",
        "rollout-b",
    )
    .await?;

    let phase2 = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-phase2"),
            ev_assistant_message("msg-phase2", "phase2 complete"),
            ev_completed("resp-phase2"),
        ]),
    )
    .await;

    let test = build_test_codex(&server, home.clone()).await?;
    trigger_memories_startup(&test).await;

    let request = wait_for_single_request(&phase2).await;
    let prompt = phase2_prompt_text(&request);
    assert!(
        prompt.contains("phase2_workspace_diff.md"),
        "expected workspace diff file in prompt: {prompt}"
    );

    wait_for_phase2_workspace_reset(&memory_root).await?;
    let raw_memories = tokio::fs::read_to_string(memory_root.join("raw_memories.md")).await?;
    assert!(raw_memories.contains("raw memory B"));
    assert!(!raw_memories.contains("raw memory A"));
    let rollout_summaries = read_rollout_summary_bodies(&memory_root).await?;
    assert_eq!(rollout_summaries.len(), 1);
    assert!(
        rollout_summaries
            .iter()
            .any(|summary| summary.contains("rollout summary B"))
    );
    assert!(
        rollout_summaries
            .iter()
            .any(|summary| summary.contains("git_branch: branch-rollout-b"))
    );
    assert!(
        rollout_summaries
            .iter()
            .all(|summary| !summary.contains("rollout summary A"))
    );

    shutdown_test_codex(&test).await?;
    Ok(())
}

#[test]
fn memories_startup_phase2_prunes_old_extension_resources() -> anyhow::Result<()> {
    run_startup_test(memories_startup_phase2_prunes_old_extension_resources_impl())
}

async fn memories_startup_phase2_prunes_old_extension_resources_impl() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let db = init_state_db(&home).await?;
    let now = chrono::Utc::now();
    let _thread_id = seed_stage1_output(
        db.as_ref(),
        home.path(),
        now - chrono::Duration::hours(1),
        "raw memory",
        "rollout summary",
        "rollout",
    )
    .await?;

    let chronicle_resources = home.path().join("memories/extensions/chronicle/resources");
    tokio::fs::create_dir_all(&chronicle_resources).await?;
    tokio::fs::write(
        home.path()
            .join("memories/extensions/chronicle/instructions.md"),
        "instructions",
    )
    .await?;
    let old_file = chronicle_resources.join(format!(
        "{}-abcd-10min-old.md",
        (now - chrono::Duration::days(8)).format("%Y-%m-%dT%H-%M-%S")
    ));
    tokio::fs::write(&old_file, "old resource").await?;
    let recent_file = chronicle_resources.join(format!(
        "{}-abcd-10min-recent.md",
        (now - chrono::Duration::days(6)).format("%Y-%m-%dT%H-%M-%S")
    ));
    tokio::fs::write(&recent_file, "recent resource").await?;

    let phase2 = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-phase2"),
            ev_assistant_message("msg-phase2", "phase2 complete"),
            ev_completed("resp-phase2"),
        ]),
    )
    .await;

    let test = build_test_codex(&server, home.clone()).await?;
    trigger_memories_startup(&test).await;

    let request = wait_for_single_request(&phase2).await;
    let prompt = phase2_prompt_text(&request);
    assert!(
        prompt.contains("phase2_workspace_diff.md"),
        "expected workspace diff file in prompt: {prompt}"
    );

    wait_for_phase2_workspace_reset(&home.path().join("memories")).await?;
    wait_for_file_removed(&old_file).await?;
    assert!(
        !tokio::fs::try_exists(&old_file).await?,
        "old extension resource should be pruned"
    );
    assert!(
        tokio::fs::try_exists(&recent_file).await?,
        "recent extension resource should be retained"
    );

    shutdown_test_codex(&test).await?;
    Ok(())
}

#[test]
fn memories_startup_phase2_prunes_old_extension_resources_without_stage1_input()
-> anyhow::Result<()> {
    run_startup_test(
        memories_startup_phase2_prunes_old_extension_resources_without_stage1_input_impl(),
    )
}

async fn memories_startup_phase2_prunes_old_extension_resources_without_stage1_input_impl()
-> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let db = init_state_db(&home).await?;
    db.enqueue_global_consolidation(/*input_watermark*/ 1)
        .await?;

    let now = chrono::Utc::now();
    let chronicle_resources = home.path().join("memories/extensions/chronicle/resources");
    tokio::fs::create_dir_all(&chronicle_resources).await?;
    tokio::fs::write(
        home.path()
            .join("memories/extensions/chronicle/instructions.md"),
        "instructions",
    )
    .await?;
    let old_file = chronicle_resources.join(format!(
        "{}-abcd-10min-old.md",
        (now - chrono::Duration::days(8)).format("%Y-%m-%dT%H-%M-%S")
    ));
    tokio::fs::write(&old_file, "old resource").await?;

    let phase2 = mount_sse_once(
        &server,
        sse(vec![
            ev_response_created("resp-phase2-empty"),
            ev_assistant_message("msg-phase2-empty", "phase2 complete"),
            ev_completed("resp-phase2-empty"),
        ]),
    )
    .await;

    let test = build_test_codex(&server, home.clone()).await?;
    trigger_memories_startup(&test).await;

    let request = wait_for_single_request(&phase2).await;
    let prompt = phase2_prompt_text(&request);
    assert!(
        prompt.contains("phase2_workspace_diff.md"),
        "expected workspace diff file in prompt: {prompt}"
    );

    wait_for_file_removed(&old_file).await?;
    wait_for_phase2_workspace_reset(&home.path().join("memories")).await?;

    shutdown_test_codex(&test).await?;
    Ok(())
}

#[test]
fn memories_startup_phase1_uses_live_thread_service_tier() -> anyhow::Result<()> {
    run_startup_test(memories_startup_phase1_uses_live_thread_service_tier_impl())
}

async fn memories_startup_phase1_uses_live_thread_service_tier_impl() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let home = Arc::new(TempDir::new()?);
    let test = build_test_codex(&server, home).await?;
    assert_eq!(test.config.service_tier, None);

    test.codex
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            permission_profile: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: Some(Some(ServiceTier::Fast.request_value().to_string())),
            collaboration_mode: None,
            personality: None,
        })
        .await?;

    let config_snapshot =
        wait_for_service_tier(&test, Some(ServiceTier::Fast.request_value().to_string())).await?;
    assert_eq!(
        config_snapshot.service_tier,
        Some(ServiceTier::Fast.request_value().to_string())
    );

    let runtime = memory_runtime_for_test(&test, config_snapshot.session_source.clone());
    let context =
        crate::runtime::MemoryStartupContext::new(test.session_configured.thread_id, runtime);
    let request_context = context
        .stage_one_request_context(
            test.config.model.as_deref().unwrap_or("gpt-5.4-mini"),
            ReasoningEffort::Low,
        )
        .await;
    assert_eq!(
        request_context.service_tier,
        Some(ServiceTier::Fast.request_value().to_string())
    );

    shutdown_test_codex(&test).await?;
    Ok(())
}

async fn build_test_codex(
    server: &wiremock::MockServer,
    home: Arc<TempDir>,
) -> anyhow::Result<TestCodex> {
    test_codex()
        .with_home(home)
        .with_config(|config| {
            config
                .features
                .enable(Feature::Sqlite)
                .expect("test config should allow feature update");
            config.memories.max_raw_memories_for_consolidation = 1;
        })
        .build(server)
        .await
}

async fn init_state_db(home: &Arc<TempDir>) -> anyhow::Result<Arc<codex_state::StateRuntime>> {
    let db =
        codex_state::StateRuntime::init(home.path().to_path_buf(), "test-provider".into()).await?;
    db.mark_backfill_complete(/*last_watermark*/ None).await?;
    Ok(db)
}

async fn trigger_memories_startup(test: &TestCodex) {
    let config_snapshot = test.codex.config_snapshot().await;
    let mut config = test.config.clone();
    config
        .features
        .enable(Feature::MemoryTool)
        .expect("test config should allow feature update");
    let settings = memory_startup_settings_for_test(&config, config_snapshot.session_source);
    start_memories_startup_task(
        memory_runtime_for_config(test, Arc::new(config), settings.session_source.clone()),
        Arc::clone(&test.auth_manager),
        test.session_configured.thread_id,
        settings,
    );
}

fn memory_runtime_for_test(
    test: &TestCodex,
    session_source: SessionSource,
) -> Arc<dyn MemoryStartupRuntime> {
    memory_runtime_for_config(test, Arc::new(test.config.clone()), session_source)
}

fn memory_runtime_for_config(
    test: &TestCodex,
    config: Arc<Config>,
    session_source: SessionSource,
) -> Arc<dyn MemoryStartupRuntime> {
    Arc::new(TestMemoryStartupRuntime::new(
        Arc::clone(&test.thread_service),
        Arc::clone(&test.auth_manager),
        test.session_configured.thread_id,
        Arc::clone(&test.codex),
        config,
        session_source,
    ))
}

fn memory_startup_settings_for_test(
    config: &Config,
    session_source: SessionSource,
) -> Arc<MemoryStartupSettings> {
    Arc::new(MemoryStartupSettings {
        codex_home: config.codex_home.clone(),
        memories: config.memories.clone(),
        chatgpt_base_url: config.chatgpt_base_url.clone(),
        ephemeral: config.ephemeral,
        memory_tool_enabled: config.features.enabled(Feature::MemoryTool),
        session_source,
    })
}

struct TestMemoryStartupRuntime {
    thread_service: Arc<ThreadService>,
    auth_manager: Arc<AuthManager>,
    thread_id: ThreadId,
    thread: Arc<CodexThread>,
    config: Arc<Config>,
    session_telemetry: SessionTelemetry,
}

impl TestMemoryStartupRuntime {
    fn new(
        thread_service: Arc<ThreadService>,
        auth_manager: Arc<AuthManager>,
        thread_id: ThreadId,
        thread: Arc<CodexThread>,
        config: Arc<Config>,
        session_source: SessionSource,
    ) -> Self {
        let model = config.model.as_deref().unwrap_or("unknown");
        let session_telemetry = SessionTelemetry::new(
            thread_id,
            model,
            model,
            /*account_id*/ None,
            /*account_email*/ None,
            /*auth_mode*/ None,
            "test".to_string(),
            config.otel.log_user_prompt,
            "test".to_string(),
            session_source,
        );

        Self {
            thread_service,
            auth_manager,
            thread_id,
            thread,
            config,
            session_telemetry,
        }
    }
}

impl MemoryStartupRuntime for TestMemoryStartupRuntime {
    fn state_db(&self) -> Option<Arc<codex_state::StateRuntime>> {
        self.thread.state_db()
    }

    fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]) {
        self.session_telemetry.counter(name, inc, tags);
    }

    fn histogram(&self, name: &str, value: i64, tags: &[(&str, &str)]) {
        self.session_telemetry.histogram(name, value, tags);
    }

    fn start_timer(&self, name: &str) -> Option<codex_otel::Timer> {
        self.session_telemetry.start_timer(name, &[]).ok()
    }

    fn stage_one_request_context<'a>(
        &'a self,
        model_name: &'a str,
        reasoning_effort: ReasoningEffort,
    ) -> MemoryRuntimeFuture<'a, StageOneRequestContext> {
        Box::pin(async move {
            let config_snapshot = self.thread.config_snapshot().await;
            let model_info = self
                .thread_service
                .get_models_manager()
                .get_model_info(model_name, &self.config.to_models_manager_config())
                .await;

            StageOneRequestContext {
                model_info,
                reasoning_effort: Some(reasoning_effort),
                service_tier: config_snapshot.service_tier,
            }
        })
    }

    fn stream_stage_one_prompt<'a>(
        &'a self,
        request: StageOnePromptRequest,
        context: &'a StageOneRequestContext,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<(String, Option<TokenUsage>)>> {
        Box::pin(async move {
            let mut prompt = Prompt::default();
            prompt.input = request.input;
            prompt.base_instructions = request.base_instructions;
            prompt.output_schema = request.output_schema;
            prompt.output_schema_strict = request.output_schema_strict;

            let installation_id = resolve_installation_id(&self.config.codex_home).await?;
            let session_source = self.thread.config_snapshot().await.session_source;
            let model_client = ModelClient::new(
                model_provider_auth_manager(Some(Arc::clone(&self.auth_manager))),
                codex_protocol::SessionId::from(self.thread_id),
                self.thread_id,
                installation_id,
                Arc::new(codex_api::DefaultApiRuntimeFactory),
                thread_service::test_support::model_provider_factory_for_tests(),
                self.config.model_provider.clone(),
                session_source,
                self.config.model_verbosity,
                self.config
                    .model_options
                    .iter()
                    .filter(|model_option| model_option.provider == self.config.model_provider_id)
                    .filter_map(|model_option| {
                        model_option
                            .max_tokens
                            .map(|max_tokens| (model_option.model.clone(), max_tokens))
                    })
                    .collect(),
                self.config
                    .features
                    .enabled(Feature::EnableRequestCompression),
                self.config.features.enabled(Feature::RuntimeMetrics),
                /*beta_features_header*/ None,
                /*attestation_provider*/ None,
            );
            let reasoning_summary = self
                .config
                .model_reasoning_summary
                .unwrap_or(context.model_info.default_reasoning_summary);
            let turn_metadata_header = codex_turn_metadata::build_turn_metadata_header(
                &self.config.cwd,
                /*sandbox*/ None,
            )
            .await;
            let mut client_session = model_client.new_session();
            let telemetry = SessionTelemetryTrait::with_model(
                &self.session_telemetry,
                context.model_info.slug.as_str(),
                context.model_info.slug.as_str(),
            );
            let mut stream = client_session
                .stream(
                    &prompt,
                    &context.model_info,
                    &telemetry,
                    context.reasoning_effort,
                    reasoning_summary,
                    context.service_tier.clone(),
                    turn_metadata_header.as_deref(),
                    &InferenceTraceContext::disabled(),
                )
                .await?;

            let mut result = String::new();
            let mut token_usage = None;
            while let Some(message) = stream.next().await.transpose()? {
                match message {
                    ResponseEvent::OutputTextDelta(delta) => result.push_str(&delta),
                    ResponseEvent::OutputItemDone(item) => {
                        if result.is_empty()
                            && let ResponseItem::Message { content, .. } = item
                            && let Some(text) = content_items_to_text(content.as_slice())
                        {
                            result.push_str(&text);
                        }
                    }
                    ResponseEvent::Completed {
                        token_usage: usage, ..
                    } => {
                        token_usage = usage;
                        break;
                    }
                    ResponseEvent::Created
                    | ResponseEvent::OutputItemAdded(_)
                    | ResponseEvent::ServerModel(_)
                    | ResponseEvent::ModelVerifications(_)
                    | ResponseEvent::ServerReasoningIncluded(_)
                    | ResponseEvent::ToolCallInputDelta { .. }
                    | ResponseEvent::ReasoningSummaryDelta { .. }
                    | ResponseEvent::ReasoningContentDelta { .. }
                    | ResponseEvent::ReasoningSummaryPartAdded { .. }
                    | ResponseEvent::RateLimits(_)
                    | ResponseEvent::ModelsEtag(_) => {}
                }
            }

            Ok((result, token_usage))
        })
    }

    fn spawn_consolidation_agent<'a>(
        &'a self,
        user_input: Vec<UserInput>,
        model: String,
        reasoning_effort: ReasoningEffort,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<Box<dyn MemoryConsolidationAgent>>> {
        Box::pin(async move {
            let thread_id = ThreadId::new();
            let (status_tx, status_rx) = watch::channel(AgentStatus::Running);
            let token_usage = Arc::new(Mutex::new(None));
            let token_usage_for_task = Arc::clone(&token_usage);
            let config = Arc::clone(&self.config);
            let auth_manager = Arc::clone(&self.auth_manager);
            let session_telemetry = self.session_telemetry.clone();
            let thread = Arc::clone(&self.thread);
            let thread_service = Arc::clone(&self.thread_service);

            tokio::spawn(async move {
                let result = stream_consolidation_prompt(
                    thread_service,
                    thread,
                    auth_manager,
                    config,
                    thread_id,
                    user_input,
                    model,
                    reasoning_effort,
                    session_telemetry,
                )
                .await;

                match result {
                    Ok((final_message, usage)) => {
                        *token_usage_for_task.lock().await = usage;
                        let _ = status_tx.send(AgentStatus::Completed(final_message));
                    }
                    Err(err) => {
                        let _ = status_tx.send(AgentStatus::Errored(err.to_string()));
                    }
                }
            });

            let agent: Box<dyn MemoryConsolidationAgent> = Box::new(TestMemoryConsolidationAgent {
                thread_id,
                status_rx,
                token_usage,
            });
            Ok(agent)
        })
    }
}

struct TestMemoryConsolidationAgent {
    thread_id: ThreadId,
    status_rx: watch::Receiver<AgentStatus>,
    token_usage: Arc<Mutex<Option<TokenUsage>>>,
}

impl MemoryConsolidationAgent for TestMemoryConsolidationAgent {
    fn thread_id(&self) -> ThreadId {
        self.thread_id
    }

    fn agent_status<'a>(&'a self) -> MemoryRuntimeFuture<'a, AgentStatus> {
        Box::pin(async move { self.status_rx.borrow().clone() })
    }

    fn wait_until_terminated<'a>(&'a self) -> MemoryRuntimeFuture<'a, ()> {
        Box::pin(async move {
            let mut status_rx = self.status_rx.clone();
            loop {
                if is_final_agent_status(&status_rx.borrow()) {
                    return;
                }
                if status_rx.changed().await.is_err() {
                    return;
                }
            }
        })
    }

    fn total_token_usage<'a>(&'a self) -> MemoryRuntimeFuture<'a, Option<TokenUsage>> {
        Box::pin(async move { self.token_usage.lock().await.clone() })
    }

    fn shutdown<'a>(self: Box<Self>) -> MemoryRuntimeFuture<'a, anyhow::Result<()>> {
        Box::pin(async move { Ok(()) })
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_consolidation_prompt(
    thread_service: Arc<ThreadService>,
    thread: Arc<CodexThread>,
    auth_manager: Arc<AuthManager>,
    config: Arc<Config>,
    thread_id: ThreadId,
    user_input: Vec<UserInput>,
    model: String,
    reasoning_effort: ReasoningEffort,
    session_telemetry: SessionTelemetry,
) -> anyhow::Result<(Option<String>, Option<TokenUsage>)> {
    let model_info = thread_service
        .get_models_manager()
        .get_model_info(&model, &config.to_models_manager_config())
        .await;
    let input_item: ResponseItem = ResponseInputItem::from(user_input).into();
    let mut prompt = Prompt::default();
    prompt.input = vec![input_item];

    let installation_id = resolve_installation_id(&config.codex_home).await?;
    let session_source = thread.config_snapshot().await.session_source;
    let model_client = ModelClient::new(
        model_provider_auth_manager(Some(auth_manager)),
        codex_protocol::SessionId::from(thread_id),
        thread_id,
        installation_id,
        Arc::new(codex_api::DefaultApiRuntimeFactory),
        thread_service::test_support::model_provider_factory_for_tests(),
        config.model_provider.clone(),
        session_source,
        config.model_verbosity,
        config
            .model_options
            .iter()
            .filter(|model_option| model_option.provider == config.model_provider_id)
            .filter_map(|model_option| {
                model_option
                    .max_tokens
                    .map(|max_tokens| (model_option.model.clone(), max_tokens))
            })
            .collect(),
        config.features.enabled(Feature::EnableRequestCompression),
        config.features.enabled(Feature::RuntimeMetrics),
        /*beta_features_header*/ None,
        /*attestation_provider*/ None,
    );
    let reasoning_summary = config
        .model_reasoning_summary
        .unwrap_or(model_info.default_reasoning_summary);
    let turn_metadata_header =
        codex_turn_metadata::build_turn_metadata_header(&config.cwd, /*sandbox*/ None).await;
    let mut client_session = model_client.new_session();
    let telemetry = SessionTelemetryTrait::with_model(
        &session_telemetry,
        model_info.slug.as_str(),
        model_info.slug.as_str(),
    );
    let mut stream = client_session
        .stream(
            &prompt,
            &model_info,
            &telemetry,
            Some(reasoning_effort),
            reasoning_summary,
            thread.config_snapshot().await.service_tier,
            turn_metadata_header.as_deref(),
            &InferenceTraceContext::disabled(),
        )
        .await?;

    let mut result = String::new();
    let mut token_usage = None;
    while let Some(message) = stream.next().await.transpose()? {
        match message {
            ResponseEvent::OutputTextDelta(delta) => result.push_str(&delta),
            ResponseEvent::OutputItemDone(item) => {
                if result.is_empty()
                    && let ResponseItem::Message { content, .. } = item
                    && let Some(text) = content_items_to_text(content.as_slice())
                {
                    result.push_str(&text);
                }
            }
            ResponseEvent::Completed {
                token_usage: usage, ..
            } => {
                token_usage = usage;
                break;
            }
            ResponseEvent::Created
            | ResponseEvent::OutputItemAdded(_)
            | ResponseEvent::ServerModel(_)
            | ResponseEvent::ModelVerifications(_)
            | ResponseEvent::ServerReasoningIncluded(_)
            | ResponseEvent::ToolCallInputDelta { .. }
            | ResponseEvent::ReasoningSummaryDelta { .. }
            | ResponseEvent::ReasoningContentDelta { .. }
            | ResponseEvent::ReasoningSummaryPartAdded { .. }
            | ResponseEvent::RateLimits(_)
            | ResponseEvent::ModelsEtag(_) => {}
        }
    }

    let final_message = (!result.is_empty()).then_some(result);
    Ok((final_message, token_usage))
}

fn is_final_agent_status(status: &AgentStatus) -> bool {
    !matches!(
        status,
        AgentStatus::PendingInit | AgentStatus::Running | AgentStatus::Interrupted
    )
}

fn content_items_to_text(content: &[ContentItem]) -> Option<String> {
    let pieces = content
        .iter()
        .filter_map(|item| match item {
            ContentItem::InputText { text } | ContentItem::OutputText { text } => {
                (!text.is_empty()).then_some(text.as_str())
            }
            ContentItem::InputImage { .. } => None,
        })
        .collect::<Vec<_>>();
    (!pieces.is_empty()).then(|| pieces.join("\n"))
}

async fn seed_stage1_output(
    db: &codex_state::StateRuntime,
    codex_home: &Path,
    updated_at: chrono::DateTime<chrono::Utc>,
    raw_memory: &str,
    rollout_summary: &str,
    rollout_slug: &str,
) -> anyhow::Result<ThreadId> {
    let thread_id = ThreadId::new();
    let mut metadata_builder = codex_state::ThreadMetadataBuilder::new(
        thread_id,
        codex_home.join(format!("rollout-{thread_id}.jsonl")),
        updated_at,
        SessionSource::Cli,
    );
    metadata_builder.cwd = codex_home.join(format!("workspace-{rollout_slug}"));
    metadata_builder.model_provider = Some("test-provider".to_string());
    metadata_builder.git_branch = Some(format!("branch-{rollout_slug}"));
    let metadata = metadata_builder.build("test-provider");
    db.upsert_thread(&metadata).await?;

    seed_stage1_output_for_existing_thread(
        db,
        thread_id,
        updated_at.timestamp(),
        raw_memory,
        rollout_summary,
        Some(rollout_slug),
    )
    .await?;

    Ok(thread_id)
}

async fn wait_for_single_request(mock: &ResponseMock) -> ResponsesRequest {
    wait_for_request(mock, /*expected_count*/ 1).await.remove(0)
}

async fn wait_for_file_removed(path: &Path) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if !tokio::fs::try_exists(path).await? {
            return Ok(());
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {} to be removed",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_request(mock: &ResponseMock, expected_count: usize) -> Vec<ResponsesRequest> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let requests = mock.requests();
        if requests.len() >= expected_count {
            return requests;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected_count} phase2 requests"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_service_tier(
    test: &TestCodex,
    expected_service_tier: Option<String>,
) -> anyhow::Result<ThreadConfigSnapshot> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let config_snapshot = test.codex.config_snapshot().await;
        if config_snapshot.service_tier == expected_service_tier {
            return Ok(config_snapshot);
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out waiting for service_tier to become {expected_service_tier:?}, current={:?}",
            config_snapshot.service_tier
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn phase2_prompt_text(request: &ResponsesRequest) -> String {
    request
        .message_input_texts("user")
        .into_iter()
        .find(|text| text.contains("Memory workspace diff:"))
        .expect("phase2 prompt text")
}

async fn wait_for_phase2_workspace_reset(memory_root: &Path) -> anyhow::Result<()> {
    wait_for_file_removed(&memory_root.join("phase2_workspace_diff.md")).await?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(diff) = diff_since_latest_init(memory_root).await
            && !diff.has_changes()
        {
            return Ok(());
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for clean memory workspace baseline"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn seed_stage1_output_for_existing_thread(
    db: &codex_state::StateRuntime,
    thread_id: ThreadId,
    updated_at: i64,
    raw_memory: &str,
    rollout_summary: &str,
    rollout_slug: Option<&str>,
) -> anyhow::Result<()> {
    let owner = ThreadId::new();
    let claim = db
        .try_claim_stage1_job(
            thread_id, owner, updated_at, /*lease_seconds*/ 3_600,
            /*max_running_jobs*/ 64,
        )
        .await?;
    let ownership_token = match claim {
        codex_state::Stage1JobClaimOutcome::Claimed { ownership_token } => ownership_token,
        other => panic!("unexpected stage-1 claim outcome: {other:?}"),
    };

    assert!(
        db.mark_stage1_job_succeeded(
            thread_id,
            &ownership_token,
            updated_at,
            raw_memory,
            rollout_summary,
            rollout_slug,
        )
        .await?,
        "stage-1 success should enqueue global consolidation"
    );

    Ok(())
}

async fn read_rollout_summary_bodies(memory_root: &Path) -> anyhow::Result<Vec<String>> {
    let mut dir = tokio::fs::read_dir(memory_root.join("rollout_summaries")).await?;
    let mut summaries = Vec::new();
    while let Some(entry) = dir.next_entry().await? {
        summaries.push(tokio::fs::read_to_string(entry.path()).await?);
    }
    summaries.sort();
    Ok(summaries)
}

async fn shutdown_test_codex(test: &TestCodex) -> anyhow::Result<()> {
    test.codex.submit(Op::Shutdown {}).await?;
    wait_for_event(&test.codex, |ev| matches!(ev, EventMsg::ShutdownComplete)).await;
    Ok(())
}
