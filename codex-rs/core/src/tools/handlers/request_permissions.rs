use codex_protocol::request_permissions::RequestPermissionsArgs;
use codex_sandboxing_api::policy_transforms::normalize_additional_permissions;

use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments_with_base_path;
use crate::tools::registry::ToolExecutor;
use crate::tools::registry::ToolHandler;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::create_request_permissions_tool;
use codex_tool_planning::request_permissions_tool_description;

pub struct RequestPermissionsHandler;

impl ToolExecutor<ToolInvocation> for RequestPermissionsHandler {
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain("request_permissions")
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_request_permissions_tool(
            request_permissions_tool_description(),
        ))
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
                cancellation_token,
                call_id,
                payload,
                ..
            } = invocation;

            let arguments = match payload {
                ToolPayload::Function { arguments } => arguments,
                _ => {
                    return Err(FunctionCallError::RespondToModel(
                        "request_permissions handler received unsupported payload".to_string(),
                    ));
                }
            };

            #[allow(deprecated)]
            let mut args: RequestPermissionsArgs =
                parse_arguments_with_base_path(&arguments, &turn.cwd)?;
            args.permissions = normalize_additional_permissions(args.permissions.into())
                .map(codex_protocol::request_permissions::RequestPermissionProfile::from)
                .map_err(FunctionCallError::RespondToModel)?;
            if args.permissions.is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "request_permissions requires at least one permission".to_string(),
                ));
            }

            let response = session
                .request_permissions(&turn, call_id, args, cancellation_token)
                .await
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel(
                        "request_permissions was cancelled before receiving a response".to_string(),
                    )
                })?;

            let content = serde_json::to_string(&response).map_err(|err| {
                FunctionCallError::Fatal(format!(
                    "failed to serialize request_permissions response: {err}"
                ))
            })?;

            Ok(FunctionToolOutput::from_text(content, Some(true)))
        })
    }
}

impl ToolHandler for RequestPermissionsHandler {}
