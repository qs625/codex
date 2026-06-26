mod context;
mod domains;

use std::sync::Arc;

use codex_approval_service_api::ApprovalServiceApi;
use codex_command_service_api::CommandServiceApi;
use codex_thread_api::GoalApi;
use codex_thread_api::McpResourceApi;
use codex_thread_api::RequestPluginInstallApi;
use codex_tool_runtime_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_service_api::ToolDiffConsumerRequest;
use codex_tool_service_api::ToolDispatchRequest;
use codex_tool_service_api::ToolParallelRequest;
use codex_tool_service_api::ToolServiceApi;
use codex_tool_service_api::ToolServiceFuture;
use codex_tool_service_api::ToolSpecRequest;
use codex_tool_types::FunctionCallError;
use codex_workflow_api::WorkflowApi;

use context::TypedToolSpecRequest;

pub struct ToolService {
    approval_api: Arc<dyn ApprovalServiceApi>,
    command_service_api: Arc<dyn CommandServiceApi>,
    goal_api: Arc<dyn GoalApi>,
    mcp_resource_api: Arc<dyn McpResourceApi>,
    request_plugin_install_api: Arc<dyn RequestPluginInstallApi>,
    workflow_api: Arc<dyn WorkflowApi>,
}

impl ToolService {
    pub fn new(
        approval_api: Arc<dyn ApprovalServiceApi>,
        command_service_api: Arc<dyn CommandServiceApi>,
        goal_api: Arc<dyn GoalApi>,
        mcp_resource_api: Arc<dyn McpResourceApi>,
        request_plugin_install_api: Arc<dyn RequestPluginInstallApi>,
        workflow_api: Arc<dyn WorkflowApi>,
    ) -> Self {
        Self {
            approval_api,
            command_service_api,
            goal_api,
            mcp_resource_api,
            request_plugin_install_api,
            workflow_api,
        }
    }
}

impl ToolService {
    fn typed_request<'a>(
        request: ToolSpecRequest<'a>,
    ) -> Result<TypedToolSpecRequest<'a>, FunctionCallError> {
        TypedToolSpecRequest::from_request(request)
    }
}

impl ToolServiceApi for ToolService {
    fn model_visible_specs(&self, request: ToolSpecRequest<'_>) -> Vec<codex_tool_types::ToolSpec> {
        match Self::typed_request(request) {
            Ok(request) => domains::model_visible_specs(self, request),
            Err(_) => Vec::new(),
        }
    }

    fn create_diff_consumer(
        &self,
        request: ToolDiffConsumerRequest<'_>,
    ) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
        let tool_request = Self::typed_request(request.tool).ok()?;
        domains::create_diff_consumer(self, tool_request, request.tool_name)
    }

    fn tool_supports_parallel(&self, request: ToolParallelRequest<'_>) -> bool {
        let Ok(tool_request) = Self::typed_request(request.tool) else {
            return false;
        };
        domains::supports_parallel(self, tool_request, request.call)
    }

    fn dispatch_tool(
        &self,
        request: ToolDispatchRequest<'_>,
    ) -> ToolServiceFuture<'_, Result<AnyToolResult, FunctionCallError>> {
        let Ok(tool_request) = Self::typed_request(request.tool) else {
            return Box::pin(async {
                Err(FunctionCallError::Fatal(
                    "tool service received unsupported dispatch context".to_string(),
                ))
            });
        };
        if matches!(
            domains::route_for_tool_name(&tool_request, &request.call.tool_name),
            domains::DirectToolRoute::Workflow
        ) {
            let workflow_api = Arc::clone(&self.workflow_api);
            let turn = Arc::clone(&tool_request.turn);
            let call = request.call;
            return Box::pin(async move {
                domains::workflow::dispatch(workflow_api, turn, call).await
            });
        }
        let domain = domains::classify_tool_name(&tool_request, &request.call.tool_name);
        let goal_api = Arc::clone(&self.goal_api);
        let approval_api = Arc::clone(&self.approval_api);
        let command_service_api = Arc::clone(&self.command_service_api);
        let mcp_resource_api = Arc::clone(&self.mcp_resource_api);
        let request_plugin_install_api = Arc::clone(&self.request_plugin_install_api);
        let session = Arc::clone(&tool_request.session);
        let turn = Arc::clone(&tool_request.turn);
        let request_user_input_available_modes =
            tool_request.config.request_user_input_available_modes.clone();
        let dynamic_tools = tool_request.params.dynamic_tools.to_vec();
        let discoverable_tools = tool_request
            .params
            .discoverable_tools
            .map(|tools| tools.to_vec());
        let mcp_tools = tool_request.params.mcp_tools.map(|tools| tools.to_vec());
        let deferred_mcp_tools = tool_request
            .params
            .deferred_mcp_tools
            .map(|tools| tools.to_vec());
        let cancellation_token = request.cancellation_token;
        let tracker = request.tracker;
        let call = request.call;
        Box::pin(async move {
            let tool_name = call.tool_name.clone();
            let result = match domain {
                domains::ToolDomain::Agent => domains::agent::dispatch(call),
                domains::ToolDomain::ApplyPatch => {
                    domains::apply_patch::dispatch(
                        Arc::clone(&approval_api),
                        Arc::clone(&session),
                        Arc::clone(&turn),
                        tracker,
                        call,
                    )
                    .await
                }
                domains::ToolDomain::CodeMode => domains::code_mode::dispatch(call),
                domains::ToolDomain::CommandInteraction => {
                    domains::command_interaction::dispatch(
                        Arc::clone(&command_service_api),
                        Arc::clone(&session),
                        Arc::clone(&turn),
                        call,
                    )
                    .await
                }
                domains::ToolDomain::Discovery => {
                    domains::discovery::dispatch(
                        request_plugin_install_api,
                        &turn,
                        &dynamic_tools,
                        mcp_tools.as_deref(),
                        deferred_mcp_tools.as_deref(),
                        discoverable_tools.as_deref(),
                        call,
                    )
                    .await
                }
                domains::ToolDomain::Extension => domains::extension::dispatch(call),
                domains::ToolDomain::Function => {
                    domains::function::dispatch(
                        Arc::clone(&turn),
                        request_user_input_available_modes,
                        dynamic_tools,
                        cancellation_token,
                        call,
                    )
                    .await
                }
                domains::ToolDomain::Goal => {
                    domains::goal::dispatch(Arc::clone(&goal_api), turn.as_ref(), call).await
                }
                domains::ToolDomain::Mcp => {
                    domains::mcp::dispatch(
                        Arc::clone(&session),
                        Arc::clone(&turn),
                        mcp_resource_api,
                        mcp_tools.as_deref(),
                        deferred_mcp_tools.as_deref(),
                        call,
                    )
                    .await
                }
                domains::ToolDomain::ExecCommand => {
                    domains::exec_command::dispatch(
                        Arc::clone(&approval_api),
                        Arc::clone(&command_service_api),
                        Arc::clone(&session),
                        Arc::clone(&turn),
                        tracker,
                        call,
                    )
                    .await
                }
                domains::ToolDomain::Workflow => unreachable!("workflow handled above"),
                domains::ToolDomain::Legacy => Err(FunctionCallError::Fatal(format!(
                    "tool domain legacy is not classified for {}",
                    call.tool_name
                ))),
            }?;

            if let Some(session_capability) = tool_request.session_capability.upgrade() {
                let _ = session_capability
                    .account_goal_tool_completed(turn.as_ref(), &tool_name)
                    .await;
            }

            let mut result = result;
            if matches!(domain, domains::ToolDomain::Goal) {
                let goal = goal_api
                    .get_thread_goal(turn.as_ref())
                    .await
                    .map_err(FunctionCallError::RespondToModel)?;
                result.result = Box::new(domains::goal::tool_output_for_state(&tool_name, goal)?);
            }

            Ok(result)
        })
    }
}
