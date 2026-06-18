use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use codex_api::ResponseEvent;
use codex_config::Constrained;
use codex_core::CodexThread;
use codex_core::ModelClient;
use codex_core::NewThread;
use codex_core::Prompt;
use codex_core::StartThreadOptions;
use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_core::resolve_installation_id;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::auth_env_telemetry::collect_auth_env_telemetry;
use codex_login::default_client::originator;
use codex_memories_write::MemoryConsolidationAgent;
use codex_memories_write::MemoryRuntimeFuture;
use codex_memories_write::MemoryStartupRuntime;
use codex_memories_write::MemoryStartupSettings;
use codex_memories_write::StageOnePromptRequest;
use codex_memories_write::StageOneRequestContext;
use codex_otel::SessionTelemetry;
use codex_otel::TelemetryAuthMode;
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
use futures::StreamExt;

pub(crate) struct CoreMemoryStartupRuntime {
    thread_manager: Arc<ThreadManager>,
    auth_manager: Arc<AuthManager>,
    thread_id: ThreadId,
    thread: Arc<CodexThread>,
    config: Arc<Config>,
    session_telemetry: SessionTelemetry,
}

impl CoreMemoryStartupRuntime {
    pub(crate) fn new(
        thread_manager: Arc<ThreadManager>,
        auth_manager: Arc<AuthManager>,
        thread_id: ThreadId,
        thread: Arc<CodexThread>,
        config: Arc<Config>,
        source: SessionSource,
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
            source,
        )
        .with_auth_env(auth_env_telemetry.to_otel_metadata());

        Self {
            thread_manager,
            auth_manager,
            thread_id,
            thread,
            config,
            session_telemetry,
        }
    }
}

impl MemoryStartupRuntime for CoreMemoryStartupRuntime {
    fn state_db(&self) -> Option<Arc<StateRuntime>> {
        self.thread.state_db()
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
            let config_snapshot = self.thread.config_snapshot().await;
            let model_info = self
                .thread_manager
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
                Some(Arc::clone(&self.auth_manager)),
                SessionId::from(self.thread_id),
                self.thread_id,
                installation_id,
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
            let turn_metadata_header =
                codex_core::build_turn_metadata_header(&self.config.cwd, /*sandbox*/ None).await;
            let mut client_session = model_client.new_session();
            let mut stream = client_session
                .stream(
                    &prompt,
                    &context.model_info,
                    &self.session_telemetry.clone().with_model(
                        context.model_info.slug.as_str(),
                        context.model_info.slug.as_str(),
                    ),
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

            let environments = self
                .thread_manager
                .default_environment_selections(&config.cwd);
            let NewThread {
                thread_id, thread, ..
            } = self
                .thread_manager
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

            if let Err(err) = thread
                .submit(Op::UserInput {
                    items: prompt,
                    environments: None,
                    final_output_json_schema: None,
                    responsesapi_client_metadata: None,
                })
                .await
            {
                shutdown_consolidation_thread(thread_id, Arc::clone(&self.thread_manager), thread)
                    .await?;
                return Err(err.into());
            }

            let agent: Box<dyn MemoryConsolidationAgent> = Box::new(CoreMemoryConsolidationAgent {
                thread_manager: Arc::clone(&self.thread_manager),
                thread_id,
                thread,
            });
            Ok(agent)
        })
    }
}

struct CoreMemoryConsolidationAgent {
    thread_manager: Arc<ThreadManager>,
    thread_id: ThreadId,
    thread: Arc<CodexThread>,
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
            shutdown_consolidation_thread(self.thread_id, self.thread_manager, self.thread).await
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
    thread_manager: Arc<ThreadManager>,
    thread: Arc<CodexThread>,
) -> anyhow::Result<()> {
    let thread = thread_manager
        .remove_thread(&thread_id)
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
