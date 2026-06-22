use super::message_tool::FollowupTaskArgs;
use super::message_tool::handle_message_string_tool;
use super::*;
use crate::tools::context::FunctionToolOutput;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_followup_task_tool;

pub(crate) struct Handler;

impl ToolExecutor<ToolInvocation> for Handler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("followup_task")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_followup_task_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation,
    ) -> crate::tools::registry::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let arguments = function_arguments(invocation.payload.clone())?;
            let args: FollowupTaskArgs = parse_arguments(&arguments)?;
            handle_message_string_tool(invocation, args.target, args.message).await
        })
    }
}

impl ToolHandler for Handler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}
