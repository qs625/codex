use codex_config::types::MemoriesConfig;
use codex_otel::Timer;
use codex_protocol::ThreadId;
use codex_protocol::models::BaseInstructions;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::AgentStatus;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::user_input::UserInput;
use codex_state::StateRuntime;
use codex_utils_absolute_path::AbsolutePathBuf;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub type MemoryRuntimeFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug)]
pub struct MemoryStartupSettings {
    pub codex_home: AbsolutePathBuf,
    pub memories: MemoriesConfig,
    pub chatgpt_base_url: String,
    pub ephemeral: bool,
    pub memory_tool_enabled: bool,
    pub session_source: SessionSource,
}

#[derive(Clone, Debug)]
pub struct StageOneRequestContext {
    pub model_info: ModelInfo,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub service_tier: Option<String>,
}

#[derive(Clone, Debug)]
pub struct StageOnePromptRequest {
    pub input: Vec<ResponseItem>,
    pub base_instructions: BaseInstructions,
    pub output_schema: Option<Value>,
    pub output_schema_strict: bool,
}

pub trait MemoryConsolidationAgent: Send + Sync {
    fn thread_id(&self) -> ThreadId;

    fn agent_status<'a>(&'a self) -> MemoryRuntimeFuture<'a, AgentStatus>;

    fn wait_until_terminated<'a>(&'a self) -> MemoryRuntimeFuture<'a, ()>;

    fn total_token_usage<'a>(&'a self) -> MemoryRuntimeFuture<'a, Option<TokenUsage>>;

    fn shutdown<'a>(self: Box<Self>) -> MemoryRuntimeFuture<'a, anyhow::Result<()>>;
}

pub trait MemoryStartupRuntime: Send + Sync {
    fn state_db(&self) -> Option<Arc<StateRuntime>>;

    fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]);

    fn histogram(&self, name: &str, value: i64, tags: &[(&str, &str)]);

    fn start_timer(&self, name: &str) -> Option<Timer>;

    fn stage_one_request_context<'a>(
        &'a self,
        model_name: &'a str,
        reasoning_effort: ReasoningEffort,
    ) -> MemoryRuntimeFuture<'a, StageOneRequestContext>;

    fn stream_stage_one_prompt<'a>(
        &'a self,
        request: StageOnePromptRequest,
        context: &'a StageOneRequestContext,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<(String, Option<TokenUsage>)>>;

    fn spawn_consolidation_agent<'a>(
        &'a self,
        prompt: Vec<UserInput>,
        model: String,
        reasoning_effort: ReasoningEffort,
    ) -> MemoryRuntimeFuture<'a, anyhow::Result<Box<dyn MemoryConsolidationAgent>>>;
}

pub(crate) struct MemoryStartupContext {
    thread_id: ThreadId,
    runtime: Arc<dyn MemoryStartupRuntime>,
}

impl MemoryStartupContext {
    pub(crate) fn new(thread_id: ThreadId, runtime: Arc<dyn MemoryStartupRuntime>) -> Self {
        Self { thread_id, runtime }
    }

    pub(crate) fn thread_id(&self) -> ThreadId {
        self.thread_id
    }

    pub(crate) fn state_db(&self) -> Option<Arc<StateRuntime>> {
        self.runtime.state_db()
    }

    pub(crate) fn counter(&self, name: &str, inc: i64, tags: &[(&str, &str)]) {
        self.runtime.counter(name, inc, tags);
    }

    pub(crate) fn histogram(&self, name: &str, value: i64, tags: &[(&str, &str)]) {
        self.runtime.histogram(name, value, tags);
    }

    pub(crate) fn start_timer(&self, name: &str) -> Option<Timer> {
        self.runtime.start_timer(name)
    }

    pub(crate) async fn stage_one_request_context(
        &self,
        model_name: &str,
        reasoning_effort: ReasoningEffort,
    ) -> StageOneRequestContext {
        self.runtime
            .stage_one_request_context(model_name, reasoning_effort)
            .await
    }

    pub(crate) async fn stream_stage_one_prompt(
        &self,
        request: StageOnePromptRequest,
        context: &StageOneRequestContext,
    ) -> anyhow::Result<(String, Option<TokenUsage>)> {
        self.runtime.stream_stage_one_prompt(request, context).await
    }

    pub(crate) async fn spawn_consolidation_agent(
        &self,
        prompt: Vec<UserInput>,
        model: String,
        reasoning_effort: ReasoningEffort,
    ) -> anyhow::Result<Box<dyn MemoryConsolidationAgent>> {
        self.runtime
            .spawn_consolidation_agent(prompt, model, reasoning_effort)
            .await
    }
}
