use codex_tool_planning::DuplicateToolName;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolRegistryPlanBuilder as PlanningToolRegistryPlanBuilder;
use codex_tool_planning::ToolSpec;
use codex_tool_types::ToolExposure;
use codex_tool_types::ToolPayload;
use std::collections::HashMap;
use std::sync::Arc;

pub use codex_tool_runtime_api::AnyToolResult;
pub use codex_tool_runtime_api::PostToolUsePayload;
pub use codex_tool_runtime_api::PreToolUsePayload;
pub use codex_tool_runtime_api::RegisteredTool;
pub use codex_tool_runtime_api::ToolArgumentDiffConsumer;
pub use codex_tool_runtime_api::ToolHandler;
pub use codex_tool_runtime_api::ToolInvocationView;
pub use codex_tool_runtime_api::ToolRegistryView;
pub use codex_tool_runtime_api::ToolTelemetryTags;
pub use codex_tool_runtime_api::override_tool_exposure;
pub use codex_tool_runtime_api::registered_tool;

pub struct ToolRegistry<Invocation, DiffContext> {
    handlers: HashMap<ToolName, Arc<dyn RegisteredTool<Invocation, DiffContext>>>,
}

impl<Invocation, DiffContext> ToolRegistry<Invocation, DiffContext>
where
    Invocation: ToolInvocationView + Clone + Send + 'static,
    DiffContext: 'static,
{
    pub fn new(
        handlers: HashMap<ToolName, Arc<dyn RegisteredTool<Invocation, DiffContext>>>,
    ) -> Self {
        Self { handlers }
    }

    pub fn empty() -> Self {
        Self::new(HashMap::new())
    }

    pub fn with_handler<T>(handler: Arc<T>) -> Self
    where
        T: ToolHandler<Invocation, DiffContext> + 'static,
    {
        let name = handler.tool_name();
        Self::new(HashMap::from([(name, registered_tool(handler))]))
    }

    pub fn handler(
        &self,
        name: &ToolName,
    ) -> Option<Arc<dyn RegisteredTool<Invocation, DiffContext>>> {
        self.handlers.get(name).map(Arc::clone)
    }

    pub fn tool_exposure(&self, name: &ToolName) -> Option<ToolExposure> {
        self.handlers.get(name).map(|handler| handler.exposure())
    }

    pub fn has_handler(&self, name: &ToolName) -> bool {
        self.handler(name).is_some()
    }

    pub fn create_diff_consumer(
        &self,
        name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>> {
        self.handler(name)?.create_diff_consumer()
    }

    pub fn supports_parallel_tool_calls(&self, name: &ToolName) -> Option<bool> {
        let handler = self.handler(name)?;
        Some(handler.supports_parallel_tool_calls())
    }
}

impl<Invocation, DiffContext> ToolRegistryView<DiffContext>
    for ToolRegistry<Invocation, DiffContext>
where
    Invocation: ToolInvocationView + Clone + Send + 'static,
    DiffContext: 'static,
{
    fn tool_exposure(&self, name: &ToolName) -> Option<ToolExposure> {
        self.tool_exposure(name)
    }

    fn create_diff_consumer(
        &self,
        name: &ToolName,
    ) -> Option<Box<dyn ToolArgumentDiffConsumer<DiffContext>>> {
        self.create_diff_consumer(name)
    }

    fn supports_parallel_tool_calls(&self, name: &ToolName) -> Option<bool> {
        self.supports_parallel_tool_calls(name)
    }
}

pub struct ToolRegistryBuilder<Invocation, DiffContext> {
    inner: PlanningToolRegistryPlanBuilder<Arc<dyn RegisteredTool<Invocation, DiffContext>>>,
}

impl<Invocation, DiffContext> ToolRegistryBuilder<Invocation, DiffContext>
where
    Invocation: ToolInvocationView + Clone + Send + 'static,
    DiffContext: 'static,
{
    pub fn new() -> Self {
        Self {
            inner: PlanningToolRegistryPlanBuilder::new(),
        }
    }

    pub fn push_spec(&mut self, spec: ToolSpec) {
        self.inner.push_spec(spec);
    }

    pub fn register_tool(
        &mut self,
        handler: Arc<dyn RegisteredTool<Invocation, DiffContext>>,
    ) -> Result<(), DuplicateToolName> {
        self.inner.register_tool(handler)
    }

    pub fn register_tool_without_spec(
        &mut self,
        handler: Arc<dyn RegisteredTool<Invocation, DiffContext>>,
    ) -> Result<(), DuplicateToolName> {
        self.inner.register_tool_without_spec(handler)
    }

    pub fn build(self) -> (Vec<ToolSpec>, ToolRegistry<Invocation, DiffContext>) {
        let plan = self.inner.build();
        let registry = ToolRegistry::new(plan.entries);
        (plan.specs, registry)
    }
}

impl<Invocation, DiffContext> Default for ToolRegistryBuilder<Invocation, DiffContext>
where
    Invocation: ToolInvocationView + Clone + Send + 'static,
    DiffContext: 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

pub fn unsupported_tool_call_message(payload: &ToolPayload, tool_name: &ToolName) -> String {
    match payload {
        ToolPayload::Custom { .. } => format!("unsupported custom tool call: {tool_name}"),
        _ => format!("unsupported call: {tool_name}"),
    }
}
