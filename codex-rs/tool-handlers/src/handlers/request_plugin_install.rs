use codex_tool_planning::DiscoverableToolAction;
use codex_tool_planning::DiscoverableToolType;
use codex_tool_planning::REQUEST_PLUGIN_INSTALL_TOOL_NAME;
use codex_tool_planning::RequestPluginInstallArgs;
use codex_tool_planning::RequestPluginInstallEntry;
use codex_tool_planning::RequestPluginInstallResult;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::build_request_plugin_install_elicitation_request;
use codex_tool_planning::collect_request_plugin_install_entries;
use codex_tool_planning::create_request_plugin_install_tool;
use codex_tool_planning::filter_request_plugin_install_discoverable_tools_for_client;
use codex_tool_runtime_api::ApplyPatchHandlerHost;
use codex_tool_runtime_api::RequestPluginInstallHost;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolExecutor;
use codex_tool_types::ToolExecutorFuture;
use codex_tool_types::ToolPayload;
use serde::Deserialize;

use crate::FunctionToolOutput;
use codex_tool_runtime::ToolInvocation;

#[derive(Default)]
pub struct RequestPluginInstallHandler<Host> {
    host: Host,
    discoverable_tools: Vec<RequestPluginInstallEntry>,
}

impl<Host> RequestPluginInstallHandler<Host> {
    pub fn new(host: Host, discoverable_tools: &[codex_tool_planning::DiscoverableTool]) -> Self {
        Self {
            host,
            discoverable_tools: collect_request_plugin_install_entries(discoverable_tools),
        }
    }
}

impl<Host>
    ToolExecutor<
        ToolInvocation<
            <Host as ApplyPatchHandlerHost>::Session,
            <Host as ApplyPatchHandlerHost>::Turn,
            <Host as ApplyPatchHandlerHost>::Tracker,
        >,
    > for RequestPluginInstallHandler<Host>
where
    Host: RequestPluginInstallHost,
{
    type Output = FunctionToolOutput;

    fn tool_name(&self) -> ToolName {
        ToolName::plain(REQUEST_PLUGIN_INSTALL_TOOL_NAME)
    }

    fn spec(&self) -> Option<ToolSpec> {
        Some(create_request_plugin_install_tool(&self.discoverable_tools))
    }

    fn supports_parallel_tool_calls(&self) -> bool {
        true
    }

    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation<
            <Host as ApplyPatchHandlerHost>::Session,
            <Host as ApplyPatchHandlerHost>::Turn,
            <Host as ApplyPatchHandlerHost>::Tracker,
        >,
    ) -> ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                session,
                turn,
                metadata,
                ..
            } = invocation;
            let call_id = metadata.call_id;
            let payload = metadata.payload;

            let arguments = match payload {
                ToolPayload::Function { arguments } => arguments,
                _ => {
                    return Err(FunctionCallError::Fatal(format!(
                        "{REQUEST_PLUGIN_INSTALL_TOOL_NAME} handler received unsupported payload"
                    )));
                }
            };

            let args: RequestPluginInstallArgs = parse_arguments(&arguments)?;
            let context = self.host.request_plugin_install_context(&session, &turn);
            let suggest_reason = validate_request_plugin_install_args(
                &args,
                context.app_server_client_name.as_deref(),
            )?;

            let discoverable_tools = self
                .host
                .list_request_plugin_install_discoverable_tools(&session, &turn)
                .await
                .map(|discoverable_tools| {
                    filter_request_plugin_install_discoverable_tools_for_client(
                        discoverable_tools,
                        context.app_server_client_name.as_deref(),
                    )
                })
                .map_err(|err| {
                    FunctionCallError::RespondToModel(format!(
                        "plugin install requests are unavailable right now: {err}"
                    ))
                })?;

            let tool = discoverable_tools
                .into_iter()
                .find(|tool| tool.tool_type() == args.tool_type && tool.id() == args.tool_id)
                .ok_or_else(|| {
                    FunctionCallError::RespondToModel(format!(
                        "tool_id must match one of the discoverable tools exposed by {REQUEST_PLUGIN_INSTALL_TOOL_NAME}"
                    ))
                })?;

            let request = build_request_plugin_install_elicitation_request(
                &context.server_name,
                context.thread_id,
                context.turn_id,
                &args,
                suggest_reason,
                &tool,
            );
            let outcome = self
                .host
                .request_plugin_install_elicitation(&session, &turn, &call_id, request, &tool)
                .await;

            let completed = if outcome.user_confirmed {
                self.host
                    .complete_request_plugin_install_if_ready(&session, &turn, &tool)
                    .await
            } else {
                false
            };

            let content = serde_json::to_string(&RequestPluginInstallResult {
                completed,
                user_confirmed: outcome.user_confirmed,
                tool_type: args.tool_type,
                action_type: args.action_type,
                tool_id: tool.id().to_string(),
                tool_name: tool.name().to_string(),
                suggest_reason: suggest_reason.to_string(),
            })
            .map_err(|err| {
                FunctionCallError::Fatal(format!(
                    "failed to serialize {REQUEST_PLUGIN_INSTALL_TOOL_NAME} response: {err}"
                ))
            })?;

            Ok(FunctionToolOutput::from_text(content, Some(true)))
        })
    }
}

impl<Host>
    ToolHandler<
        ToolInvocation<
            <Host as ApplyPatchHandlerHost>::Session,
            <Host as ApplyPatchHandlerHost>::Turn,
            <Host as ApplyPatchHandlerHost>::Tracker,
        >,
        <Host as ApplyPatchHandlerHost>::DiffContext,
    > for RequestPluginInstallHandler<Host>
where
    Host: RequestPluginInstallHost + ApplyPatchHandlerHost,
{
}

fn validate_request_plugin_install_args<'a>(
    args: &'a RequestPluginInstallArgs,
    app_server_client_name: Option<&str>,
) -> Result<&'a str, FunctionCallError> {
    let suggest_reason = args.suggest_reason.trim();
    if suggest_reason.is_empty() {
        return Err(FunctionCallError::RespondToModel(
            "suggest_reason must not be empty".to_string(),
        ));
    }
    if args.action_type != DiscoverableToolAction::Install {
        return Err(FunctionCallError::RespondToModel(
            "plugin install requests currently support only action_type=\"install\"".to_string(),
        ));
    }
    if args.tool_type == DiscoverableToolType::Plugin && app_server_client_name == Some("codex-tui")
    {
        return Err(FunctionCallError::RespondToModel(
            "plugin install requests are not available in codex-tui yet".to_string(),
        ));
    }

    Ok(suggest_reason)
}

fn parse_arguments<T>(arguments: &str) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(arguments).map_err(|err| {
        FunctionCallError::RespondToModel(format!("failed to parse function arguments: {err}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(
        tool_type: DiscoverableToolType,
        action_type: DiscoverableToolAction,
        suggest_reason: &str,
    ) -> RequestPluginInstallArgs {
        RequestPluginInstallArgs {
            tool_type,
            action_type,
            tool_id: "sample".to_string(),
            suggest_reason: suggest_reason.to_string(),
        }
    }

    #[test]
    fn validate_request_plugin_install_args_trims_reason() {
        let args = args(
            DiscoverableToolType::Connector,
            DiscoverableToolAction::Install,
            "  Use calendar  ",
        );

        assert_eq!(
            validate_request_plugin_install_args(&args, None).expect("valid args"),
            "Use calendar"
        );
    }

    #[test]
    fn validate_request_plugin_install_args_rejects_empty_reason() {
        let args = args(
            DiscoverableToolType::Connector,
            DiscoverableToolAction::Install,
            "   ",
        );

        assert!(matches!(
            validate_request_plugin_install_args(&args, None),
            Err(FunctionCallError::RespondToModel(message)) if message == "suggest_reason must not be empty"
        ));
    }

    #[test]
    fn validate_request_plugin_install_args_rejects_non_install_action() {
        let args = args(
            DiscoverableToolType::Connector,
            DiscoverableToolAction::Enable,
            "Use calendar",
        );

        assert!(matches!(
            validate_request_plugin_install_args(&args, None),
            Err(FunctionCallError::RespondToModel(message))
                if message == "plugin install requests currently support only action_type=\"install\""
        ));
    }

    #[test]
    fn validate_request_plugin_install_args_rejects_plugins_for_tui() {
        let args = args(
            DiscoverableToolType::Plugin,
            DiscoverableToolAction::Install,
            "Use Slack",
        );

        assert!(matches!(
            validate_request_plugin_install_args(&args, Some("codex-tui")),
            Err(FunctionCallError::RespondToModel(message))
                if message == "plugin install requests are not available in codex-tui yet"
        ));
    }
}
