use super::*;
use crate::SharedTurnDiffTracker;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use codex_thread_api::GoalApi;
use codex_thread_api::SessionCapabilityFuture;
use codex_thread_api::ThreadCapability;
use codex_protocol::protocol::ThreadGoal;
use codex_tool_planning::GET_GOAL_TOOL_NAME;
use codex_tool_planning::create_get_goal_tool;
use pretty_assertions::assert_eq;

struct TestHandler {
    tool_name: codex_tool_planning::ToolName,
}

impl ToolExecutor<ToolInvocation> for TestHandler {
    type Output = codex_tool_runtime::FunctionToolOutput;

    fn tool_name(&self) -> codex_tool_planning::ToolName {
        self.tool_name.clone()
    }

    fn handle<'a>(
        &'a self,
        _invocation: ToolInvocation,
    ) -> crate::tool_test_registry::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            Ok(codex_tool_runtime::FunctionToolOutput::from_text(
                "ok".to_string(),
                Some(true),
            ))
        })
    }
}

impl ToolHandler<ToolInvocation, crate::session::turn_context::TurnContext> for TestHandler {}

struct MockGoalApi;

impl GoalApi for MockGoalApi {
    fn get_thread_goal<'a>(
        &'a self,
        _capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<Option<ThreadGoal>, String>> {
        Box::pin(async { Ok(None) })
    }

    fn create_thread_goal<'a>(
        &'a self,
        _capability: &'a dyn ThreadCapability,
        _objective: String,
        _token_budget: Option<i64>,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        Box::pin(async { panic!("not used in registry_tests") })
    }

    fn complete_thread_goal<'a>(
        &'a self,
        _capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<ThreadGoal, String>> {
        Box::pin(async { panic!("not used in registry_tests") })
    }
}

#[test]
fn handler_looks_up_namespaced_aliases_explicitly() {
    let namespace = "mcp__codex_apps__gmail";
    let tool_name = "gmail_get_recent_emails";
    let plain_name = codex_tool_planning::ToolName::plain(tool_name);
    let namespaced_name = codex_tool_planning::ToolName::namespaced(namespace, tool_name);
    let plain_handler = Arc::new(TestHandler {
        tool_name: plain_name.clone(),
    });
    let namespaced_handler = Arc::new(TestHandler {
        tool_name: namespaced_name.clone(),
    });
    let plain_handler = registered_tool(plain_handler);
    let namespaced_handler = registered_tool(namespaced_handler);
    let registry = ToolRegistry::new(HashMap::from([
        (plain_name.clone(), Arc::clone(&plain_handler)),
        (namespaced_name.clone(), Arc::clone(&namespaced_handler)),
    ]));

    let plain = registry.handler(&plain_name);
    let namespaced = registry.handler(&namespaced_name);
    let missing_namespaced = registry.handler(&codex_tool_planning::ToolName::namespaced(
        "mcp__codex_apps__calendar",
        tool_name,
    ));

    assert_eq!(plain.is_some(), true);
    assert_eq!(namespaced.is_some(), true);
    assert_eq!(missing_namespaced.is_none(), true);
    assert!(
        plain
            .as_ref()
            .is_some_and(|handler| Arc::ptr_eq(handler, &plain_handler))
    );
    assert!(
        namespaced
            .as_ref()
            .is_some_and(|handler| Arc::ptr_eq(handler, &namespaced_handler))
    );
}

#[test]
fn register_tool_adds_executor_and_spec() {
    let mut builder = ToolRegistryBuilder::new();
    let goal_service = Arc::new(MockGoalApi);
    builder.register_tool(registered_tool(Arc::new(
        codex_tool_handlers::GetGoalHandler::new(goal_service),
    )));

    let (specs, registry) = builder.build();

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0], create_get_goal_tool());
    assert!(registry.has_handler(&codex_tool_planning::ToolName::plain(GET_GOAL_TOOL_NAME)));
}
