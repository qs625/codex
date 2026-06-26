use std::sync::Arc;

use codex_thread_runtime::ThreadRuntimeSession;
use codex_thread_runtime::ThreadTurnContext;
use codex_tool_service_api::ToolSpecRequest;
use codex_tool_types::FunctionCallError;

pub(crate) struct TypedToolSpecRequest<'a> {
    pub(crate) config: &'a codex_tool_config::ToolsConfig,
    pub(crate) session_capability: std::sync::Weak<dyn codex_thread_api::ToolSessionCapability>,
    pub(crate) session: Arc<ThreadRuntimeSession>,
    pub(crate) turn: Arc<ThreadTurnContext>,
    pub(crate) params: codex_tool_runtime_api::ToolServiceParams<'a>,
}

impl Clone for TypedToolSpecRequest<'_> {
    fn clone(&self) -> Self {
        Self {
            config: self.config,
            session_capability: self.session_capability.clone(),
            session: Arc::clone(&self.session),
            turn: Arc::clone(&self.turn),
            params: codex_tool_runtime_api::ToolServiceParams {
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
    pub(crate) fn from_request(
        request: ToolSpecRequest<'a>,
    ) -> Result<TypedToolSpecRequest<'a>, FunctionCallError> {
        let Ok(session) = Arc::clone(&request.session)
            .into_any_arc()
            .downcast::<ThreadRuntimeSession>()
        else {
            return Err(FunctionCallError::Fatal(
                "tool service received unsupported session context".to_string(),
            ));
        };
        let Ok(turn) = Arc::clone(&request.turn)
            .into_any_arc()
            .downcast::<ThreadTurnContext>()
        else {
            return Err(FunctionCallError::Fatal(
                "tool service received unsupported turn context".to_string(),
            ));
        };
        Ok(TypedToolSpecRequest {
            config: request.config,
            session_capability: request.session_capability,
            session,
            turn,
            params: request.params,
        })
    }
}
