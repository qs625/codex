use anyhow::Result;
use codex_config_types::MemoriesConfig;
use codex_otel::Timer;
use codex_utils_absolute_path::AbsolutePathBuf;
use protocol::ThreadId;
use protocol::models::BaseInstructions;
use protocol::models::ResponseItem;
use protocol::openai_models::ModelInfo;
use protocol::openai_models::ReasoningEffort;
use protocol::protocol::AgentStatus;
use protocol::protocol::SessionSource;
use protocol::protocol::TokenUsage;
use protocol::user_input::UserInput;
use serde_json::Value;
use state_api::SharedStateDbRuntime;
use std::future::Future;
use std::pin::Pin;

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

/// Runtime-side handle for an in-flight memory consolidation agent.
pub trait MemoryConsolidationAgent: Send + Sync {
    fn thread_id(&self) -> ThreadId;

    fn agent_status<'a>(&'a self) -> MemoryRuntimeFuture<'a, AgentStatus>;

    fn wait_until_terminated<'a>(&'a self) -> MemoryRuntimeFuture<'a, ()>;

    fn total_token_usage<'a>(&'a self) -> MemoryRuntimeFuture<'a, Option<TokenUsage>>;

    fn shutdown<'a>(self: Box<Self>) -> MemoryRuntimeFuture<'a, Result<()>>;
}

/// Composition-root capability contract required by the memory startup pipeline.
pub trait MemoryStartupRuntime: Send + Sync {
    fn state_db(&self) -> Option<SharedStateDbRuntime>;

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
    ) -> MemoryRuntimeFuture<'a, Result<(String, Option<TokenUsage>)>>;

    fn spawn_consolidation_agent<'a>(
        &'a self,
        prompt: Vec<UserInput>,
        model: String,
        reasoning_effort: ReasoningEffort,
    ) -> MemoryRuntimeFuture<'a, Result<Box<dyn MemoryConsolidationAgent>>>;
}
