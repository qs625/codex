use std::collections::BTreeMap;
use std::collections::HashSet;

use codex_config_types::ToolSuggestDisabledTool;
use codex_connectors_types::AppInfo;
use codex_core_plugins_api::PluginRuntime;
use codex_mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use codex_mcp_types::CodexAppsAuthContext;
use codex_mcp_types::ElicitationAction;
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::McpElicitationObjectType;
use codex_mcp_types::McpElicitationSchema;
use codex_mcp_types::McpServerElicitationRequest;
use codex_mcp_types::McpServerElicitationRequestParams;
use codex_protocol::mcp::RequestId;
use codex_tool_planning::DiscoverableTool;
use codex_tool_planning::DiscoverableToolAction;
use codex_tool_planning::DiscoverableToolType;
use codex_tool_planning::REQUEST_PLUGIN_INSTALL_PERSIST_ALWAYS_VALUE;
use codex_tool_planning::REQUEST_PLUGIN_INSTALL_PERSIST_KEY;
use codex_tool_planning::REQUEST_PLUGIN_INSTALL_TOOL_NAME;
use codex_tool_planning::RequestPluginInstallArgs;
use codex_tool_planning::RequestPluginInstallElicitationRequest;
use codex_tool_planning::RequestPluginInstallElicitationSchema;
use codex_tool_planning::RequestPluginInstallEntry;
use codex_tool_planning::RequestPluginInstallResult;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::all_requested_connectors_picked_up;
use codex_tool_planning::build_request_plugin_install_elicitation_request;
use codex_tool_planning::collect_request_plugin_install_entries;
use codex_tool_planning::create_request_plugin_install_tool;
use codex_tool_planning::filter_request_plugin_install_discoverable_tools_for_client;
use codex_tool_planning::verified_connector_install_completed;
use serde_json::Value;
use serde_json::json;
use tracing::warn;

use crate::config::edit::ConfigEdit;
use crate::config::edit::ConfigEditsBuilder;
use crate::connectors;
use crate::function_tool::FunctionCallError;
use crate::tools::context::FunctionToolOutput;
use crate::tools::context::ToolInvocation;
use crate::tools::context::ToolPayload;
use crate::tools::handlers::parse_arguments;
use crate::tools::registry::ToolExecutor;
use crate::tools::registry::ToolHandler;

#[derive(Default)]
pub struct RequestPluginInstallHandler {
    discoverable_tools: Vec<RequestPluginInstallEntry>,
}

impl RequestPluginInstallHandler {
    pub(crate) fn new(discoverable_tools: &[DiscoverableTool]) -> Self {
        Self {
            discoverable_tools: collect_request_plugin_install_entries(discoverable_tools),
        }
    }
}

impl ToolExecutor<ToolInvocation> for RequestPluginInstallHandler {
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

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "plugin install discovery reads through the session-owned manager guard"
    )]
    fn handle<'a>(
        &'a self,
        invocation: ToolInvocation,
    ) -> crate::tools::registry::ToolExecutorFuture<'a, Self::Output>
    where
        Self: 'a,
    {
        Box::pin(async move {
            let ToolInvocation {
                payload,
                session,
                turn,
                call_id,
                ..
            } = invocation;

            let arguments = match payload {
                ToolPayload::Function { arguments } => arguments,
                _ => {
                    return Err(FunctionCallError::Fatal(format!(
                        "{REQUEST_PLUGIN_INSTALL_TOOL_NAME} handler received unsupported payload"
                    )));
                }
            };

            let args: RequestPluginInstallArgs = parse_arguments(&arguments)?;
            let suggest_reason = args.suggest_reason.trim();
            if suggest_reason.is_empty() {
                return Err(FunctionCallError::RespondToModel(
                    "suggest_reason must not be empty".to_string(),
                ));
            }
            if args.action_type != DiscoverableToolAction::Install {
                return Err(FunctionCallError::RespondToModel(
                    "plugin install requests currently support only action_type=\"install\""
                        .to_string(),
                ));
            }
            if args.tool_type == DiscoverableToolType::Plugin
                && turn.app_server_client_name.as_deref() == Some("codex-tui")
            {
                return Err(FunctionCallError::RespondToModel(
                    "plugin install requests are not available in codex-tui yet".to_string(),
                ));
            }

            let auth_snapshot = match turn.auth_runtime.as_ref() {
                Some(auth_runtime) => auth_runtime.auth().await,
                None => None,
            };
            let connector_auth_context =
                crate::mcp::codex_apps_auth_context(auth_snapshot.as_ref());
            let manager = session.services.mcp_connection_manager.read().await;
            let mcp_tools = manager.list_all_tools().await;
            drop(manager);
            let accessible_connectors = connectors::with_app_enabled_state(
                connectors::accessible_connectors_from_mcp_tools(&mcp_tools),
                &turn.config,
            );
            let discoverable_tools = connectors::list_tool_suggest_discoverable_tools_with_auth(
                &turn.config,
                session.services.plugins_manager.as_ref(),
                connector_auth_context.as_ref(),
                &accessible_connectors,
            )
            .await
            .map(|discoverable_tools| {
                filter_request_plugin_install_discoverable_tools_for_client(
                    discoverable_tools,
                    turn.app_server_client_name.as_deref(),
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

            let request_id = RequestId::String(format!("request_plugin_install_{call_id}"));
            let request = build_request_plugin_install_elicitation_request(
                CODEX_APPS_MCP_SERVER_NAME,
                session.conversation_id.to_string(),
                turn.sub_id.clone(),
                &args,
                suggest_reason,
                &tool,
            );
            let params = request_plugin_install_elicitation_request_to_mcp_params(request);
            let response = session
                .request_mcp_server_elicitation(turn.as_ref(), request_id, params)
                .await;
            if let Some(response) = response.as_ref() {
                maybe_persist_disabled_install_request(&session, &turn, &tool, response).await;
            }
            let user_confirmed = response
                .as_ref()
                .is_some_and(|response| response.action == ElicitationAction::Accept);

            let completed = if user_confirmed {
                verify_request_plugin_install_completed(
                    &session,
                    &turn,
                    &tool,
                    connector_auth_context.as_ref(),
                )
                .await
            } else {
                false
            };

            if completed && let DiscoverableTool::Connector(connector) = &tool {
                session
                    .merge_connector_selection(HashSet::from([connector.id.clone()]))
                    .await;
            }

            let content = serde_json::to_string(&RequestPluginInstallResult {
                completed,
                user_confirmed,
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

impl ToolHandler for RequestPluginInstallHandler {}

fn request_plugin_install_elicitation_request_to_mcp_params(
    request: RequestPluginInstallElicitationRequest,
) -> McpServerElicitationRequestParams {
    let requested_schema = match request.form.requested_schema {
        RequestPluginInstallElicitationSchema::EmptyObject => McpElicitationSchema {
            schema_uri: None,
            type_: McpElicitationObjectType::Object,
            properties: BTreeMap::new(),
            required: None,
        },
    };

    McpServerElicitationRequestParams {
        thread_id: request.thread_id,
        turn_id: request.turn_id,
        server_name: request.server_name,
        request: McpServerElicitationRequest::Form {
            meta: Some(json!(request.form.meta)),
            message: request.form.message,
            requested_schema,
        },
    }
}

async fn maybe_persist_disabled_install_request(
    session: &crate::session::session::Session,
    turn: &crate::session::turn_context::TurnContext,
    tool: &DiscoverableTool,
    response: &ElicitationResponse,
) {
    if !request_plugin_install_response_requests_persistent_disable(response) {
        return;
    }

    if let Err(err) = persist_disabled_install_request(&turn.config.codex_home, tool).await {
        warn!(
            error = %err,
            tool_id = tool.id(),
            "failed to persist disabled tool suggestion"
        );
        return;
    }

    session.reload_user_config_layer().await;
}

fn request_plugin_install_response_requests_persistent_disable(
    response: &ElicitationResponse,
) -> bool {
    if response.action != ElicitationAction::Decline {
        return false;
    }

    response
        .meta
        .as_ref()
        .and_then(Value::as_object)
        .and_then(|meta| meta.get(REQUEST_PLUGIN_INSTALL_PERSIST_KEY))
        .and_then(Value::as_str)
        == Some(REQUEST_PLUGIN_INSTALL_PERSIST_ALWAYS_VALUE)
}

async fn persist_disabled_install_request(
    codex_home: &codex_utils_absolute_path::AbsolutePathBuf,
    tool: &DiscoverableTool,
) -> anyhow::Result<()> {
    ConfigEditsBuilder::new(codex_home)
        .with_edits([ConfigEdit::AddToolSuggestDisabledTool(
            disabled_install_request(tool),
        )])
        .apply()
        .await
}

fn disabled_install_request(tool: &DiscoverableTool) -> ToolSuggestDisabledTool {
    match tool {
        DiscoverableTool::Connector(connector) => {
            ToolSuggestDisabledTool::connector(connector.id.as_str())
        }
        DiscoverableTool::Plugin(plugin) => ToolSuggestDisabledTool::plugin(plugin.id.as_str()),
    }
}

async fn verify_request_plugin_install_completed(
    session: &crate::session::session::Session,
    turn: &crate::session::turn_context::TurnContext,
    tool: &DiscoverableTool,
    connector_auth_context: Option<&CodexAppsAuthContext>,
) -> bool {
    match tool {
        DiscoverableTool::Connector(connector) => refresh_missing_requested_connectors(
            session,
            turn,
            connector_auth_context,
            std::slice::from_ref(&connector.id),
            connector.id.as_str(),
        )
        .await
        .is_some_and(|accessible_connectors| {
            verified_connector_install_completed(connector.id.as_str(), &accessible_connectors)
        }),
        DiscoverableTool::Plugin(plugin) => {
            session.reload_user_config_layer().await;
            let config = session.get_config().await;
            let completed = verified_plugin_install_completed(
                plugin.id.as_str(),
                config.as_ref(),
                session.services.plugins_manager.as_ref(),
            );
            let _ = refresh_missing_requested_connectors(
                session,
                turn,
                connector_auth_context,
                &plugin.app_connector_ids,
                plugin.id.as_str(),
            )
            .await;
            completed
        }
    }
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "connector cache refresh reads through the session-owned manager guard"
)]
async fn refresh_missing_requested_connectors(
    session: &crate::session::session::Session,
    turn: &crate::session::turn_context::TurnContext,
    connector_auth_context: Option<&CodexAppsAuthContext>,
    expected_connector_ids: &[String],
    tool_id: &str,
) -> Option<Vec<AppInfo>> {
    if expected_connector_ids.is_empty() {
        return Some(Vec::new());
    }

    let manager = session.services.mcp_connection_manager.read().await;
    let mcp_tools = manager.list_all_tools().await;
    let accessible_connectors = connectors::with_app_enabled_state(
        connectors::accessible_connectors_from_mcp_tools(&mcp_tools),
        &turn.config,
    );
    if all_requested_connectors_picked_up(expected_connector_ids, &accessible_connectors) {
        return Some(accessible_connectors);
    }

    match manager.hard_refresh_codex_apps_tools_cache().await {
        Ok(mcp_tools) => {
            let accessible_connectors = connectors::with_app_enabled_state(
                connectors::accessible_connectors_from_mcp_tools(&mcp_tools),
                &turn.config,
            );
            connectors::refresh_accessible_connectors_cache_from_mcp_tools(
                &turn.config,
                connector_auth_context,
                &mcp_tools,
            );
            Some(accessible_connectors)
        }
        Err(err) => {
            warn!(
                "failed to refresh codex apps tools cache after plugin install request for {tool_id}: {err:#}"
            );
            None
        }
    }
}

fn verified_plugin_install_completed(
    tool_id: &str,
    config: &crate::config::Config,
    plugins_manager: &dyn PluginRuntime,
) -> bool {
    let plugins_input = config.plugins_config_input();
    plugins_manager.is_configured_plugin_installed(&plugins_input, tool_id)
}

#[cfg(test)]
#[path = "request_plugin_install_tests.rs"]
mod tests;
