//! Implements the MultiAgentV2 collaboration tool surface.

use crate::agent::AgentStatus;
use crate::agent::agent_resolver::resolve_agent_target;
use crate::function_tool::FunctionCallError;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolOutput;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::multi_agents_common::*;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolExecutor;
use crate::tools::registry::ToolHandler;
use codex_protocol::AgentPath;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::protocol::CollabAgentInteractionBeginEvent;
use codex_protocol::protocol::CollabAgentInteractionEndEvent;
use codex_protocol::protocol::CollabAgentSpawnBeginEvent;
use codex_protocol::protocol::CollabAgentSpawnEndEvent;
use codex_protocol::protocol::CollabCloseBeginEvent;
use codex_protocol::protocol::CollabCloseEndEvent;
use codex_protocol::user_input::UserInput;
use codex_tools::ToolName;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value as JsonValue;

pub(crate) use close_agent::Handler as CloseAgentHandler;
pub(crate) use followup_task::Handler as FollowupTaskHandler;
pub(crate) use list_agents::Handler as ListAgentsHandler;
pub(crate) use spawn::Handler as SpawnAgentHandler;
pub(crate) use wait_agent::Handler as WaitAgentHandler;

mod close_agent;
mod followup_task;
mod list_agents;
mod message_tool;
mod spawn;
mod wait_agent;

pub(crate) async fn handle_workflow_spawn_agent(
    invocation: ToolInvocation,
) -> Result<JsonValue, FunctionCallError> {
    let result = spawn::handle_spawn_agent(invocation).await?;
    serde_json::to_value(result).map_err(|err| {
        FunctionCallError::Fatal(format!("failed to serialize workflow spawn result: {err}"))
    })
}

pub(crate) async fn handle_workflow_followup_task(
    invocation: ToolInvocation,
    target: String,
    message: String,
) -> Result<JsonValue, FunctionCallError> {
    message_tool::handle_message_string_tool(invocation, target, message).await?;
    Ok(serde_json::json!({ "ok": true }))
}

pub(crate) async fn handle_workflow_wait_agent(
    invocation: ToolInvocation,
) -> Result<JsonValue, FunctionCallError> {
    let result = wait_agent::handle_wait_agent(invocation).await?;
    serde_json::to_value(result).map_err(|err| {
        FunctionCallError::Fatal(format!("failed to serialize workflow wait result: {err}"))
    })
}
