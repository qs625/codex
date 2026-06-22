use super::*;
use crate::agent::control::ListedAgent;
use crate::turn_timing::now_unix_timestamp_ms;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_list_agents_tool;

pub(crate) struct Handler;

impl ToolExecutor<ToolInvocation> for Handler {
    type Output = ListAgentsResult;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("list_agents")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_list_agents_tool())
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation,
    ) -> crate::tools::registry::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                payload,
                call_id,
                ..
            } = invocation;
            let arguments = function_arguments(payload)?;
            let args: ListAgentsArgs = parse_arguments(&arguments)?;
            let sender_agent_path = turn
                .session_source
                .get_agent_path()
                .unwrap_or_else(AgentPath::root)
                .to_string();
            session
                .send_event(
                    &turn,
                    CollabListAgentsBeginEvent {
                        call_id: call_id.clone(),
                        started_at_ms: now_unix_timestamp_ms(),
                        sender_thread_id: session.conversation_id,
                        sender_agent_path: sender_agent_path.clone(),
                        path_prefix: args.path_prefix.clone(),
                    }
                    .into(),
                )
                .await;
            session
                .services
                .agent_control
                .register_session_root(session.conversation_id, &turn.session_source);
            let agents = session
                .services
                .agent_control
                .list_agents(
                    session.conversation_id,
                    &turn.session_source,
                    args.path_prefix.as_deref(),
                )
                .await
                .map_err(collab_spawn_error);

            let listed_agents = agents.as_ref().map_or_else(
                |_| Vec::new(),
                |agents| {
                    agents
                        .iter()
                        .map(|agent| CollabListedAgent {
                            agent_path: agent.agent_name.clone(),
                            status: agent.agent_status.clone(),
                            last_task_message: agent.last_task_message.clone(),
                        })
                        .collect()
                },
            );

            session
                .send_event(
                    &turn,
                    CollabListAgentsEndEvent {
                        call_id,
                        completed_at_ms: now_unix_timestamp_ms(),
                        sender_thread_id: session.conversation_id,
                        sender_agent_path,
                        path_prefix: args.path_prefix,
                        success: agents.is_ok(),
                        agents: listed_agents,
                    }
                    .into(),
                )
                .await;

            Ok(ListAgentsResult { agents: agents? })
        })
    }
}

impl ToolHandler for Handler {
    fn matches_kind(&self, payload: &ToolPayload) -> bool {
        matches!(payload, ToolPayload::Function { .. })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListAgentsArgs {
    path_prefix: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ListAgentsResult {
    agents: Vec<ListedAgent>,
}

impl ToolOutput for ListAgentsResult {
    fn log_preview(&self) -> String {
        tool_output_json_text(self, "list_agents")
    }

    fn success_for_logging(&self) -> bool {
        true
    }

    fn to_response_item(&self, call_id: &str, payload: &ToolPayload) -> ResponseInputItem {
        tool_output_response_item(call_id, payload, self, Some(true), "list_agents")
    }

    fn code_mode_result(&self, _payload: &ToolPayload) -> JsonValue {
        tool_output_code_mode_result(self, "list_agents")
    }
}
