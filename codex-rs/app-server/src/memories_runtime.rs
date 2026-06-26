use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use codex_api::ResponseEvent;
use codex_auth_types::TelemetryAuthMode;
use codex_config_types::Constrained;
use codex_thread_runtime::ModelClient;
use codex_thread_runtime::NewThread;
use codex_thread_runtime::Prompt;
use codex_thread_runtime::StartThreadOptions;
use codex_thread_runtime::ThreadService;
use codex_thread_runtime::config::Config;
use codex_thread_runtime::resolve_installation_id;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::auth_env_telemetry::collect_auth_env_telemetry;
use codex_login::default_client::originator;
use codex_login::model_provider_auth_manager;
use codex_memories_write::MemoryConsolidationAgent;
use codex_memories_write::MemoryRuntimeFuture;
use codex_memories_write::MemoryStartupRuntime;
use codex_memories_write::MemoryStartupSettings;
use codex_memories_write::StageOnePromptRequest;
use codex_memories_write::StageOneRequestContext;
use codex_otel::SessionTelemetry;
use codex_otel::Timer;
use codex_protocol::SessionId;
use codex_protocol::ThreadId;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::InitialHistory;
use codex_protocol::protocol::InternalSessionSource;
use codex_protocol::protocol::Op;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::ThreadSource;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::user_input::UserInput;
use codex_rollout_trace::InferenceTraceContext;
use codex_state::StateRuntime;
use codex_terminal_detection::user_agent;
use codex_thread_api::ThreadConfigSnapshot;
use futures::StreamExt;

use crate::live_thread_runtime::AppServerLiveThreadHandle;

/// Composition-root capability needed by memory startup/consolidation.
///
/// The memory runtime should not keep concrete `ThreadService` or
/// `CodexThread` handles. Implementations own model catalog lookup and
/// consolidation thread lifecycle, while memory code consumes only this
/// app-server boundary trait.
pub(crate) trait MemoryStartupHost: Send + Sync {
    fn stage_one_request_context<'a>(
        &'a self,
        model_name: &'a str,
        config: &'a Config,
        reasoning_effort: ReasoningEffort,
        service_tier: Option<String>,
    ) -> MemoryRuntimeFuture<'a, StageOneRequestContext>;

    fn start_consolidation_thread<'a>(
        &'a self,
        config: Config,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<MemoryConsolidationThread>>;

    fn remove_consolidation_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> MemoryRuntimeFuture<'a, Option<Arc<dyn AppServerLiveThreadHandle>>>;
}

pub(crate) struct MemoryConsolidationThread {
    thread_id: ThreadId,
    thread: Arc<dyn AppServerLiveThreadHandle>,
}

impl MemoryStartupHost for ThreadService {
    fn stage_one_request_context<'a>(
        &'a self,
        model_name: &'a str,
        config: &'a Config,
        reasoning_effort: ReasoningEffort,
        service_tier: Option<String>,
    ) -> MemoryRuntimeFuture<'a, StageOneRequestContext> {
        Box::pin(async move {
            let model_info = self
                .get_models_manager()
                .get_model_info(model_name, &config.to_models_manager_config())
                .await;

            StageOneRequestContext {
                model_info,
                reasoning_effort: Some(reasoning_effort),
                service_tier,
            }
        })
    }

    fn start_consolidation_thread<'a>(
        &'a self,
        config: Config,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<MemoryConsolidationThread>> {
        Box::pin(async move {
            let environments = self.default_environment_selections(&config.cwd);
            let NewThread {
                thread_id, thread, ..
            } = self
                .start_thread_with_options(StartThreadOptions {
                    config,
                    initial_history: InitialHistory::New,
                    session_source: Some(SessionSource::Internal(
                        InternalSessionSource::MemoryConsolidation,
                    )),
                    thread_source: Some(ThreadSource::MemoryConsolidation),
                    dynamic_tools: Vec::new(),
                    persist_extended_history: false,
                    metrics_service_name: None,
                    parent_trace: None,
                    environments,
                })
                .await?;
            let thread: Arc<dyn AppServerLiveThreadHandle> = thread;
            Ok(MemoryConsolidationThread { thread_id, thread })
        })
    }

    fn remove_consolidation_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> MemoryRuntimeFuture<'a, Option<Arc<dyn AppServerLiveThreadHandle>>> {
        Box::pin(async move {
            self.remove_thread(&thread_id)
                .await
                .map(|thread| -> Arc<dyn AppServerLiveThreadHandle> { thread })
        })
    }
}

pub(crate) struct CoreMemoryStartupRuntime {
    host: Arc<dyn MemoryStartupHost>,
    auth_manager: Arc<AuthManager>,
    thread_id: ThreadId,
    config_snapshot: ThreadConfigSnapshot,
    config: Arc<Config>,
    state_db: Option<Arc<StateRuntime>>,
    session_telemetry: SessionTelemetry,
}

impl CoreMemoryStartupRuntime {
    pub(crate) fn new(
        host: Arc<dyn MemoryStartupHost>,
        auth_manager: Arc<AuthManager>,
        thread_id: ThreadId,
        config_snapshot: ThreadConfigSnapshot,
        config: Arc<Config>,
        state_db: Option<Arc<StateRuntime>>,
    ) -> Self {
        let auth = auth_manager.auth_cached();
        let auth = auth.as_ref();
        let auth_mode = auth.map(CodexAuth::auth_mode).map(TelemetryAuthMode::from);
        let account_id = auth.and_then(CodexAuth::get_account_id);
        let account_email = auth.and_then(CodexAuth::get_account_email);
        let model = config.model.as_deref().unwrap_or("unknown");
        let auth_env_telemetry = collect_auth_env_telemetry(
            &config.model_provider,
            auth_manager.codex_api_key_env_enabled(),
        );
        let session_telemetry = SessionTelemetry::new(
            thread_id,
            model,
            model,
            account_id,
            account_email,
            auth_mode,
            originator().value,
            config.otel.log_user_prompt,
            user_agent(),
            config_snapshot.session_source.clone(),
        )
        .with_auth_env(auth_env_telemetry.to_otel_metadata());

        Self {
            host,
            auth_manager,
            thread_id,
            config_snapshot,
            config,
            state_db,
            session_telemetry,
        }
    }
}

impl MemoryStartupRuntime for CoreMemoryStartupRuntime {
    fn state_db(&self) -> Option<Arc<StateRuntime>> {
        self.state_db.clone()
    }

    fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]) {
        self.session_telemetry.counter(name, inc, tags);
    }

    fn histogram(&self, name: &str, value: i64, tags: &[(&str, &str)]) {
        self.session_telemetry.histogram(name, value, tags);
    }

    fn start_timer(&self, name: &str) -> Option<Timer> {
        self.session_telemetry.start_timer(name, &[]).ok()
    }

    fn stage_one_request_context<'a>(
        &'a self,
        model_name: &'a str,
        reasoning_effort: ReasoningEffort,
    ) -> MemoryRuntimeFuture<'a, StageOneRequestContext> {
        Box::pin(async move {
            self.host
                .stage_one_request_context(
                    model_name,
                    self.config.as_ref(),
                    reasoning_effort,
                    self.config_snapshot.service_tier.clone(),
                )
                .await
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
            let model_client = ModelClient::new(
                model_provider_auth_manager(Some(Arc::clone(&self.auth_manager))),
                SessionId::from(self.thread_id),
                self.thread_id,
                installation_id,
                Arc::new(codex_api::DefaultApiRuntimeFactory),
                Arc::new(codex_model_provider::DefaultModelProviderFactory),
                self.config.model_provider.clone(),
                self.config_snapshot.session_source.clone(),
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
            let session_telemetry = Arc::new(self.session_telemetry.clone().with_model(
                context.model_info.slug.as_str(),
                context.model_info.slug.as_str(),
            )) as codex_otel::SharedSessionTelemetry;
            let mut client_session = model_client.new_session();
            let mut stream = client_session
                .stream(
                    &prompt,
                    &context.model_info,
                    &session_telemetry,
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
        prompt: Vec<UserInput>,
        model: String,
        reasoning_effort: ReasoningEffort,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<Box<dyn MemoryConsolidationAgent>>> {
        Box::pin(async move {
            let mut config = (*self.config).clone();
            let root = codex_memories_write::memory_root(&config.codex_home);
            config.cwd = root.clone();
            config.ephemeral = true;
            config.memories.generate_memories = false;
            config.memories.use_memories = false;
            config.include_apps_instructions = false;
            config.mcp_servers = Constrained::allow_only(HashMap::new());
            config.permissions.approval_policy = Constrained::allow_only(AskForApproval::Never);
            let _ = config.features.disable(Feature::SpawnCsv);
            let _ = config.features.disable(Feature::Collab);
            let _ = config.features.disable(Feature::MemoryTool);
            let _ = config.features.disable(Feature::Apps);
            let _ = config.features.disable(Feature::Plugins);
            let _ = config.features.disable(Feature::SkillMcpDependencyInstall);
            let sandbox_policy = SandboxPolicy::WorkspaceWrite {
                writable_roots: vec![root],
                network_access: false,
                exclude_tmpdir_env_var: true,
                exclude_slash_tmp: true,
            };
            config
                .permissions
                .set_legacy_sandbox_policy(sandbox_policy, config.cwd.as_path())
                .map_err(|err| {
                    anyhow::anyhow!("failed to set consolidation sandbox policy: {err}")
                })?;
            config.model = Some(model);
            config.model_reasoning_effort = Some(reasoning_effort);

            let MemoryConsolidationThread { thread_id, thread } =
                self.host.start_consolidation_thread(config).await?;

            if let Err(err) = thread
                .submit_op(Op::UserInput {
                    items: prompt,
                    environments: None,
                    final_output_json_schema: None,
                    responsesapi_client_metadata: None,
                })
                .await
            {
                shutdown_consolidation_thread(thread_id, Arc::clone(&self.host), thread).await?;
                return Err(err.into());
            }

            let agent: Box<dyn MemoryConsolidationAgent> = Box::new(CoreMemoryConsolidationAgent {
                host: Arc::clone(&self.host),
                thread_id,
                thread,
            });
            Ok(agent)
        })
    }
}

struct CoreMemoryConsolidationAgent {
    host: Arc<dyn MemoryStartupHost>,
    thread_id: ThreadId,
    thread: Arc<dyn AppServerLiveThreadHandle>,
}

impl MemoryConsolidationAgent for CoreMemoryConsolidationAgent {
    fn thread_id(&self) -> ThreadId {
        self.thread_id
    }

    fn agent_status<'a>(&'a self) -> MemoryRuntimeFuture<'a, AgentStatus> {
        Box::pin(async move { self.thread.agent_status().await })
    }

    fn wait_until_terminated<'a>(&'a self) -> MemoryRuntimeFuture<'a, ()> {
        Box::pin(async move {
            self.thread.wait_until_terminated().await;
        })
    }

    fn total_token_usage<'a>(&'a self) -> MemoryRuntimeFuture<'a, Option<TokenUsage>> {
        Box::pin(async move {
            self.thread
                .token_usage_info()
                .await
                .map(|info| info.total_token_usage)
        })
    }

    fn shutdown<'a>(self: Box<Self>) -> MemoryRuntimeFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            shutdown_consolidation_thread(self.thread_id, self.host, self.thread).await
        })
    }
}

pub(crate) fn memory_startup_settings(
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

async fn shutdown_consolidation_thread(
    thread_id: ThreadId,
    host: Arc<dyn MemoryStartupHost>,
    thread: Arc<dyn AppServerLiveThreadHandle>,
) -> anyhow::Result<()> {
    let thread = host
        .remove_consolidation_thread(thread_id)
        .await
        .unwrap_or(thread);

    tokio::time::timeout(Duration::from_secs(10), thread.shutdown_and_wait())
        .await
        .map_err(|_| {
            anyhow::anyhow!("memory consolidation agent {thread_id} shutdown timed out")
        })??;

    Ok(())
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
