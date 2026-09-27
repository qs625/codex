use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use codex_auth_types::TelemetryAuthMode;
use codex_config_types::Constrained;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_login::auth_env_telemetry::collect_auth_env_telemetry;
use codex_login::default_client::originator;
use codex_otel::SessionTelemetry;
use codex_otel::Timer;
use codex_terminal_detection::user_agent;
use futures::StreamExt;
use memory_service_api::MemoryConsolidationAgent;
use memory_service_api::MemoryRuntimeFuture;
use memory_service_api::MemoryStartupRuntime;
use memory_service_api::MemoryStartupSettings;
use memory_service_api::StageOnePromptRequest;
use memory_service_api::StageOneRequestContext;
use model_service_api::CreateModelClientRequest;
use model_service_api::ModelCatalogRefresh;
use model_service_api::ModelOptionToml;
use model_service_api::ModelResponseEvent;
use model_service_api::ModelSelectionPolicy;
use model_service_api::ResponsesModelRequest;
use model_service_api::SharedModelServiceApi;
use protocol::SessionId;
use protocol::ThreadId;
use protocol::config_types::ServiceTier;
use protocol::models::ContentItem;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::AgentStatus;
use protocol::protocol::AskForApproval;
use protocol::protocol::InitialHistory;
use protocol::protocol::InternalSessionSource;
use protocol::protocol::Op;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::SessionSource;
use protocol::protocol::ThreadSource;
use protocol::protocol::TokenUsage;
use protocol::user_input::UserInput;
use state_api::SharedStateDbRuntime;
use thread_service::NewThread;
use thread_service::StartThreadOptions;
use thread_service::ThreadService;
use thread_service::config::Config;
use thread_service::resolve_installation_id;
use thread_service_api::ThreadConfigSnapshot;

use crate::live_thread_runtime::AppServerMemoryConsolidationThreadHandle;

/// Composition-root capability needed by memory-service startup/consolidation.
///
/// The memory service should not keep concrete `ThreadService` or `CodexThread`
/// handles. Implementations own model catalog lookup and consolidation thread
/// lifecycle, while memory code consumes only this app-server boundary trait.
pub(crate) trait MemoryServiceHost: Send + Sync {
    fn start_consolidation_thread<'a>(
        &'a self,
        config: Config,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<MemoryConsolidationThread>>;

    fn remove_consolidation_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> MemoryRuntimeFuture<'a, Option<Arc<dyn AppServerMemoryConsolidationThreadHandle>>>;
}

pub(crate) struct MemoryConsolidationThread {
    thread_id: ThreadId,
    thread: Arc<dyn AppServerMemoryConsolidationThreadHandle>,
}

impl MemoryServiceHost for ThreadService {
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
                    agent_metadata: None,
                    thread_source: Some(ThreadSource::MemoryConsolidation),
                    dynamic_tools: Vec::new(),
                    persist_extended_history: false,
                    metrics_service_name: None,
                    parent_trace: None,
                    environments,
                })
                .await?;
            let thread: Arc<dyn AppServerMemoryConsolidationThreadHandle> = thread;
            Ok(MemoryConsolidationThread { thread_id, thread })
        })
    }

    fn remove_consolidation_thread<'a>(
        &'a self,
        thread_id: ThreadId,
    ) -> MemoryRuntimeFuture<'a, Option<Arc<dyn AppServerMemoryConsolidationThreadHandle>>> {
        Box::pin(async move {
            self.remove_thread(&thread_id)
                .await
                .map(|thread| -> Arc<dyn AppServerMemoryConsolidationThreadHandle> { thread })
        })
    }
}

pub(crate) struct AppServerMemoryStartupAdapter {
    host: Arc<dyn MemoryServiceHost>,
    model_service: SharedModelServiceApi,
    thread_id: ThreadId,
    config_snapshot: ThreadConfigSnapshot,
    config: Arc<Config>,
    state_db: Option<SharedStateDbRuntime>,
    session_telemetry: SessionTelemetry,
}

impl AppServerMemoryStartupAdapter {
    pub(crate) fn new(
        host: Arc<dyn MemoryServiceHost>,
        model_service: SharedModelServiceApi,
        auth_manager: Arc<AuthManager>,
        thread_id: ThreadId,
        config_snapshot: ThreadConfigSnapshot,
        config: Arc<Config>,
        state_db: Option<SharedStateDbRuntime>,
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
            model_service,
            thread_id,
            config_snapshot,
            config,
            state_db,
            session_telemetry,
        }
    }
}

fn service_tier_from_string(value: Option<String>) -> Option<ServiceTier> {
    value.and_then(|tier| ServiceTier::from_request_value(&tier))
}

fn chat_completions_max_tokens_by_model(
    model_options: &[ModelOptionToml],
    provider_id: &str,
) -> HashMap<String, u64> {
    model_options
        .iter()
        .filter(|model_option| model_option.provider == provider_id)
        .filter_map(|model_option| {
            model_option
                .max_tokens
                .map(|max_tokens| (model_option.model.clone(), max_tokens))
        })
        .collect()
}

struct StageOneModelClientRequest<'a> {
    config: &'a Config,
    config_snapshot: &'a ThreadConfigSnapshot,
    thread_id: ThreadId,
    installation_id: String,
    model_slug: String,
}

impl StageOneModelClientRequest<'_> {
    fn into_request(self) -> CreateModelClientRequest {
        CreateModelClientRequest {
            selection: ModelSelectionPolicy {
                requested_model: Some(self.model_slug),
                provider_hint: Some(self.config.model_provider_id.clone()),
                allow_default_fallback: true,
                refresh: ModelCatalogRefresh::Offline,
            },
            installation_id: self.installation_id,
            session_id: SessionId::from(self.thread_id),
            thread_id: self.thread_id,
            session_source: self.config_snapshot.session_source.clone(),
            reasoning_effort: self.config.model_reasoning_effort,
            service_tier: service_tier_from_string(self.config_snapshot.service_tier.clone()),
            verbosity: self.config.model_verbosity,
            chat_completions_max_tokens_by_model: chat_completions_max_tokens_by_model(
                &self.config.model_options,
                &self.config.model_provider_id,
            ),
            enable_request_compression: self
                .config
                .features
                .enabled(Feature::EnableRequestCompression),
            include_timing_metrics: self.config.features.enabled(Feature::RuntimeMetrics),
            beta_features_header: None,
        }
    }
}

struct StageOneResponsesRequest {
    prompt: StageOnePromptRequest,
    model_slug: String,
    reasoning_effort: Option<ReasoningEffort>,
    reasoning_summary: protocol::config_types::ReasoningSummary,
    service_tier: Option<ServiceTier>,
    verbosity: Option<protocol::config_types::Verbosity>,
    turn_metadata_header: Option<String>,
}

impl StageOneResponsesRequest {
    fn into_request(self) -> ResponsesModelRequest {
        ResponsesModelRequest {
            input: self.prompt.input,
            tools: Vec::new(),
            parallel_tool_calls: false,
            base_instructions: self.prompt.base_instructions,
            personality: None,
            output_schema: self.prompt.output_schema,
            output_schema_strict: self.prompt.output_schema_strict,
            model: Some(self.model_slug),
            reasoning_effort: self.reasoning_effort,
            reasoning_summary: self.reasoning_summary,
            service_tier: self.service_tier,
            verbosity: self.verbosity,
            turn_metadata_header: self.turn_metadata_header,
        }
    }
}

#[derive(Default)]
struct StageOneStreamAccumulator {
    result: String,
    token_usage: Option<TokenUsage>,
    completed: bool,
}

impl StageOneStreamAccumulator {
    fn observe(&mut self, message: ModelResponseEvent) {
        match message {
            ModelResponseEvent::OutputTextDelta { delta } => self.result.push_str(&delta),
            ModelResponseEvent::ItemDone { item } => {
                if self.result.is_empty()
                    && let protocol::models::ResponseItem::Message { content, .. } = item
                    && let Some(text) = content_items_to_text(content.as_slice())
                {
                    self.result.push_str(&text);
                }
            }
            ModelResponseEvent::Completed {
                token_usage: usage, ..
            } => {
                self.token_usage = usage;
                self.completed = true;
            }
            ModelResponseEvent::Created
            | ModelResponseEvent::ItemAdded { .. }
            | ModelResponseEvent::ServerModel { .. }
            | ModelResponseEvent::ModelVerifications { .. }
            | ModelResponseEvent::ServerReasoningIncluded { .. }
            | ModelResponseEvent::ToolCallInputDelta { .. }
            | ModelResponseEvent::ReasoningSummaryDelta { .. }
            | ModelResponseEvent::ReasoningContentDelta { .. }
            | ModelResponseEvent::ReasoningSummaryPartAdded { .. }
            | ModelResponseEvent::RateLimits { .. }
            | ModelResponseEvent::ModelsEtag { .. } => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.completed
    }

    fn finish(self) -> (String, Option<TokenUsage>) {
        (self.result, self.token_usage)
    }
}

struct ConsolidationThreadConfig {
    config: Config,
}

impl ConsolidationThreadConfig {
    fn prepare(
        base_config: &Config,
        model: String,
        reasoning_effort: ReasoningEffort,
    ) -> anyhow::Result<Self> {
        let mut config = base_config.clone();
        let root = memory_service::memory_root(&config.codex_home);
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
            .map_err(|err| anyhow::anyhow!("failed to set consolidation sandbox policy: {err}"))?;
        config.model = Some(model);
        config.model_reasoning_effort = Some(reasoning_effort);

        Ok(Self { config })
    }

    fn into_config(self) -> Config {
        self.config
    }
}

impl MemoryStartupRuntime for AppServerMemoryStartupAdapter {
    fn state_db(&self) -> Option<SharedStateDbRuntime> {
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
            let model_info = self
                .model_service
                .get_model_info(model_name)
                .await
                .unwrap_or_else(|err| {
                    panic!("failed to load model info for memory startup: {err}")
                });
            StageOneRequestContext {
                model_info,
                reasoning_effort: Some(reasoning_effort),
                service_tier: self.config_snapshot.service_tier.clone(),
            }
        })
    }

    fn stream_stage_one_prompt<'a>(
        &'a self,
        request: StageOnePromptRequest,
        context: &'a StageOneRequestContext,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<(String, Option<TokenUsage>)>> {
        Box::pin(async move {
            let installation_id = resolve_installation_id(&self.config.codex_home).await?;
            let model_client = self
                .model_service
                .create_client(
                    StageOneModelClientRequest {
                        config: &self.config,
                        config_snapshot: &self.config_snapshot,
                        thread_id: self.thread_id,
                        installation_id,
                        model_slug: context.model_info.slug.clone(),
                    }
                    .into_request(),
                )
                .await
                .map_err(anyhow::Error::msg)?;
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
            let _session_telemetry = session_telemetry;
            let mut stream = model_client
                .stream_responses(
                    StageOneResponsesRequest {
                        prompt: request,
                        model_slug: context.model_info.slug.clone(),
                        reasoning_effort: context.reasoning_effort,
                        reasoning_summary,
                        service_tier: service_tier_from_string(context.service_tier.clone()),
                        verbosity: self.config.model_verbosity,
                        turn_metadata_header,
                    }
                    .into_request(),
                )
                .await?;

            let mut accumulator = StageOneStreamAccumulator::default();
            while let Some(message) = stream.next().await.transpose()? {
                accumulator.observe(message);
                if accumulator.is_complete() {
                    break;
                }
            }

            Ok(accumulator.finish())
        })
    }

    fn spawn_consolidation_agent<'a>(
        &'a self,
        prompt: Vec<UserInput>,
        model: String,
        reasoning_effort: ReasoningEffort,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<Box<dyn MemoryConsolidationAgent>>> {
        Box::pin(async move {
            let config = ConsolidationThreadConfig::prepare(&self.config, model, reasoning_effort)?
                .into_config();

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

            let agent: Box<dyn MemoryConsolidationAgent> =
                Box::new(AppServerMemoryConsolidationAgent {
                    host: Arc::clone(&self.host),
                    thread_id,
                    thread,
                });
            Ok(agent)
        })
    }
}

struct AppServerMemoryConsolidationAgent {
    host: Arc<dyn MemoryServiceHost>,
    thread_id: ThreadId,
    thread: Arc<dyn AppServerMemoryConsolidationThreadHandle>,
}

impl MemoryConsolidationAgent for AppServerMemoryConsolidationAgent {
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

pub(crate) fn build_memory_startup_settings(
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
    host: Arc<dyn MemoryServiceHost>,
    thread: Arc<dyn AppServerMemoryConsolidationThreadHandle>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::models::ResponseItem;

    #[test]
    fn service_tier_from_string_accepts_current_request_values() {
        assert_eq!(
            service_tier_from_string(Some("priority".to_string())),
            Some(ServiceTier::Fast)
        );
        assert_eq!(
            service_tier_from_string(Some("fast".to_string())),
            Some(ServiceTier::Fast)
        );
        assert_eq!(
            service_tier_from_string(Some("flex".to_string())),
            Some(ServiceTier::Flex)
        );
        assert_eq!(service_tier_from_string(Some("unknown".to_string())), None);
        assert_eq!(service_tier_from_string(None), None);
    }

    #[test]
    fn chat_completions_max_tokens_uses_only_selected_provider_with_values() {
        let model_options = vec![
            ModelOptionToml {
                model: "gpt-a".to_string(),
                provider: "openai".to_string(),
                max_tokens: Some(1024),
                ..Default::default()
            },
            ModelOptionToml {
                model: "gpt-b".to_string(),
                provider: "other".to_string(),
                max_tokens: Some(2048),
                ..Default::default()
            },
            ModelOptionToml {
                model: "gpt-c".to_string(),
                provider: "openai".to_string(),
                max_tokens: None,
                ..Default::default()
            },
        ];

        assert_eq!(
            chat_completions_max_tokens_by_model(&model_options, "openai"),
            HashMap::from([("gpt-a".to_string(), 1024)])
        );
    }

    #[test]
    fn stage_one_stream_accumulator_prefers_deltas_over_item_done_fallback() {
        let mut accumulator = StageOneStreamAccumulator::default();

        accumulator.observe(ModelResponseEvent::OutputTextDelta {
            delta: "delta".to_string(),
        });
        accumulator.observe(ModelResponseEvent::ItemDone {
            item: message_item("fallback"),
        });
        accumulator.observe(ModelResponseEvent::Completed {
            response_id: "response".to_string(),
            token_usage: Some(token_usage(12)),
            end_turn: Some(true),
        });

        assert!(accumulator.is_complete());
        assert_eq!(
            accumulator.finish(),
            ("delta".to_string(), Some(token_usage(12)))
        );
    }

    #[test]
    fn stage_one_stream_accumulator_uses_item_done_when_no_deltas_arrive() {
        let mut accumulator = StageOneStreamAccumulator::default();

        accumulator.observe(ModelResponseEvent::ItemDone {
            item: message_item("fallback"),
        });
        accumulator.observe(ModelResponseEvent::Completed {
            response_id: "response".to_string(),
            token_usage: None,
            end_turn: None,
        });

        assert_eq!(accumulator.finish(), ("fallback".to_string(), None));
    }

    fn message_item(text: &str) -> ResponseItem {
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: text.to_string(),
            }],
            phase: None,
        }
    }

    fn token_usage(total_tokens: i64) -> TokenUsage {
        TokenUsage {
            input_tokens: 0,
            cached_input_tokens: 0,
            output_tokens: total_tokens,
            reasoning_output_tokens: 0,
            total_tokens,
        }
    }
}
