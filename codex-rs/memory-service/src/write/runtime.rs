use codex_otel::Timer;
use codex_protocol::ThreadId;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::TokenUsage;
use codex_protocol::user_input::UserInput;
use codex_state::StateRuntime;
use memory_service_api::MemoryConsolidationAgent;
use memory_service_api::MemoryStartupRuntime;
use memory_service_api::StageOnePromptRequest;
use memory_service_api::StageOneRequestContext;
use std::sync::Arc;

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
