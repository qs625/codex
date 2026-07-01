use std::sync::Arc;

use codex_tool_service_api::ToolSpecRequest;

pub(crate) struct TypedToolSpecRequest<'a> {
    pub(crate) config: &'a codex_tool_config::ToolsConfig,
    pub(crate) session_capability: std::sync::Weak<dyn thread_service_api::ThreadSessionCapability>,
    pub(crate) session: Arc<dyn thread_service_api::ThreadSessionCapability>,
    pub(crate) session_command_state:
        Arc<dyn codex_command_service_api::CommandServiceSessionState>,
    pub(crate) session_command_interaction:
        Arc<dyn codex_command_service_api::SessionCommandInteractionCaller>,
    pub(crate) session_agent_jobs: Arc<dyn thread_service_api::SessionAgentJobCaller>,
    pub(crate) turn: Arc<dyn thread_service_api::ThreadRuntimeCapability>,
    pub(crate) params: codex_tool_service_api::ToolServiceParams<'a>,
}

impl Clone for TypedToolSpecRequest<'_> {
    fn clone(&self) -> Self {
        Self {
            config: self.config,
            session_capability: self.session_capability.clone(),
            session: Arc::clone(&self.session),
            session_command_state: Arc::clone(&self.session_command_state),
            session_command_interaction: Arc::clone(&self.session_command_interaction),
            session_agent_jobs: Arc::clone(&self.session_agent_jobs),
            turn: Arc::clone(&self.turn),
            params: codex_tool_service_api::ToolServiceParams {
                mcp_tools: self.params.mcp_tools,
                deferred_mcp_tools: self.params.deferred_mcp_tools,
                discoverable_tools: self.params.discoverable_tools,
                extension_tools: self.params.extension_tools,
                dynamic_tools: self.params.dynamic_tools,
                default_agent_type_description: self.params.default_agent_type_description,
            },
        }
    }
}

impl<'a> TypedToolSpecRequest<'a> {
    pub(crate) fn from_request(request: ToolSpecRequest<'a>) -> TypedToolSpecRequest<'a> {
        TypedToolSpecRequest {
            config: request.config,
            session_capability: request.session_capability,
            session: request.session,
            session_command_state: request.session_command_state,
            session_command_interaction: request.session_command_interaction,
            session_agent_jobs: request.session_agent_jobs,
            turn: request.turn,
            params: request.params,
        }
    }
}
