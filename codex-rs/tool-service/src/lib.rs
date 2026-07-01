mod context;
mod domains;
mod event_support;
mod output;
mod planning;
mod support;

use std::sync::Arc;
use std::sync::Weak;

use codex_approval_service_api::ApprovalServiceApi;
use codex_command_service_api::CommandServiceApi;
use codex_tool_service_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_service_api::ToolDiffConsumerRequest;
use codex_tool_service_api::ToolDispatchRequest;
use codex_tool_service_api::ToolParallelRequest;
use codex_tool_service_api::ToolServiceApi;
use codex_tool_service_api::ToolServiceFuture;
use codex_tool_service_api::ToolSpecRequest;
use codex_tool_types::FunctionCallError;
use codex_workflow_api::WorkflowApi;
use goal_service_api::GoalServiceApi;
use mcp_service_api::McpServiceApi;
use thread_service_api::ThreadServiceApi;

use context::TypedToolSpecRequest;
pub(crate) use planning::*;

pub struct ToolService {
    approval_api: Arc<dyn ApprovalServiceApi>,
    command_service_api: Arc<dyn CommandServiceApi>,
    goal_api: Arc<dyn GoalServiceApi>,
    mcp_service_api: Arc<dyn McpServiceApi>,
    workflow_api: Arc<dyn WorkflowApi>,
    thread_service_api: Weak<dyn ThreadServiceApi>,
}

impl ToolService {
    pub fn new(
        approval_api: Arc<dyn ApprovalServiceApi>,
        command_service_api: Arc<dyn CommandServiceApi>,
        goal_api: Arc<dyn GoalServiceApi>,
        mcp_service_api: Arc<dyn McpServiceApi>,
        workflow_api: Arc<dyn WorkflowApi>,
        thread_service_api: Weak<dyn ThreadServiceApi>,
    ) -> Self {
        Self {
            approval_api,
            command_service_api,
            goal_api,
            mcp_service_api,
            workflow_api,
            thread_service_api,
        }
    }

    fn thread_service_api(&self) -> Result<Arc<dyn ThreadServiceApi>, FunctionCallError> {
        self.thread_service_api
            .upgrade()
            .ok_or_else(|| {
                FunctionCallError::Fatal(
                    "tool service thread service api is unavailable".to_string(),
                )
            })
    }

    fn typed_request<'a>(request: ToolSpecRequest<'a>) -> TypedToolSpecRequest<'a> {
        TypedToolSpecRequest::from_request(request)
    }
}

impl ToolServiceApi for ToolService {
    fn model_visible_specs(&self, request: ToolSpecRequest<'_>) -> Vec<codex_tool_types::ToolSpec> {
        domains::model_visible_specs(self, Self::typed_request(request))
    }

    fn create_diff_consumer(
        &self,
        request: ToolDiffConsumerRequest<'_>,
    ) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
        let tool_request = Self::typed_request(request.tool);
        domains::create_diff_consumer(self, tool_request, request.tool_name)
    }

    fn tool_supports_parallel(&self, request: ToolParallelRequest<'_>) -> bool {
        let tool_request = Self::typed_request(request.tool);
        domains::supports_parallel(self, tool_request, request.call)
    }

    fn dispatch_tool(
        &self,
        request: ToolDispatchRequest<'_>,
    ) -> ToolServiceFuture<'_, Result<AnyToolResult, FunctionCallError>> {
        let tool_request = Self::typed_request(request.tool);
        if matches!(
            domains::route_for_tool_name(&tool_request, &request.call.tool_name),
            domains::DirectToolRoute::Workflow
        ) {
            let workflow_api = Arc::clone(&self.workflow_api);
            let turn = Arc::clone(&tool_request.turn);
            let call = request.call;
            return Box::pin(
                async move { domains::workflow::dispatch(workflow_api, turn, call).await },
            );
        }
        let domain = domains::classify_tool_name(&tool_request, &request.call.tool_name);
        let code_mode_nested_tool_specs = if matches!(domain, domains::ToolDomain::CodeMode) {
            Some(domains::model_visible_specs(self, tool_request.clone()))
        } else {
            None
        };
        let extension_executor = if matches!(domain, domains::ToolDomain::Extension) {
            Some(domains::extension::resolve_executor(
                &tool_request,
                &request.call.tool_name,
            ))
        } else {
            None
        };
        let goal_api = Arc::clone(&self.goal_api);
        let approval_api = Arc::clone(&self.approval_api);
        let command_service_api = Arc::clone(&self.command_service_api);
        let mcp_service_api = Arc::clone(&self.mcp_service_api);
        let session = Arc::clone(&tool_request.session);
        let turn = Arc::clone(&tool_request.turn);
        let request_user_input_available_modes = tool_request
            .config
            .request_user_input_available_modes
            .clone();
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
                domains::ToolDomain::Agent => {
                    domains::agent::dispatch(
                        Arc::clone(&tool_request.session_agent_jobs),
                        self.thread_service_api()?,
                        Arc::clone(&turn)
                            as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
                        call,
                    )
                    .await
                }
                domains::ToolDomain::ApplyPatch => {
                    domains::apply_patch::dispatch(
                        Arc::clone(&approval_api),
                        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>,
                        Arc::clone(&turn)
                            as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
                        tracker,
                        call,
                    )
                    .await
                }
                domains::ToolDomain::CodeMode => {
                    domains::code_mode::dispatch(
                        Arc::clone(&session),
                        Arc::clone(&turn)
                            as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
                        code_mode_nested_tool_specs.unwrap_or_default(),
                        call,
                    )
                    .await
                }
                domains::ToolDomain::CommandInteraction => {
                    domains::command_interaction::dispatch(
                        Arc::clone(&command_service_api),
                        Arc::clone(&tool_request.session_command_interaction),
                        Arc::clone(&session),
                        Arc::clone(&turn)
                            as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
                        call,
                    )
                    .await
                }
                domains::ToolDomain::Discovery => {
                    domains::discovery::dispatch(
                        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>,
                        Arc::clone(&turn)
                            as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
                        &dynamic_tools,
                        mcp_tools.as_deref(),
                        deferred_mcp_tools.as_deref(),
                        discoverable_tools.as_deref(),
                        call,
                    )
                    .await
                }
                domains::ToolDomain::Extension => {
                    domains::extension::dispatch(
                        extension_executor.expect("extension route")?,
                        call,
                    )
                    .await
                }
                domains::ToolDomain::Function => {
                    domains::function::dispatch(
                        Arc::clone(&turn)
                            as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
                        request_user_input_available_modes,
                        dynamic_tools,
                        cancellation_token,
                        call,
                    )
                    .await
                }
                domains::ToolDomain::Goal => {
                    domains::goal::dispatch(
                        Arc::clone(&goal_api),
                        session.as_ref(),
                        turn.as_ref(),
                        call,
                    )
                    .await
                }
                domains::ToolDomain::Mcp => {
                    domains::mcp::dispatch(
                        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>,
                        Arc::clone(&turn)
                            as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
                        mcp_service_api,
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
                        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>,
                        Arc::clone(&tool_request.session_command_state),
                        Arc::clone(&turn)
                            as Arc<dyn thread_service_api::ThreadRuntimeCapability>,
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
                let _ = if matches!(domain, domains::ToolDomain::Goal) {
                    session_capability
                        .account_goal_mutation_completed(turn.as_ref())
                        .await
                } else {
                    session_capability
                        .account_goal_tool_completed(turn.as_ref(), &tool_name.name)
                        .await
                };
            }

            let mut result = result;
            if matches!(domain, domains::ToolDomain::Goal) {
                let goal = goal_api
                    .get_thread_goal(session.as_ref())
                    .await
                    .map_err(FunctionCallError::RespondToModel)?;
                result.result = Box::new(domains::goal::tool_output_for_state(&tool_name, goal)?);
            }

            Ok(result)
        })
    }
}
