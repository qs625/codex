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
use codex_tool_planning::REQUEST_PLUGIN_INSTALL_PERSIST_ALWAYS_VALUE;
use codex_tool_planning::REQUEST_PLUGIN_INSTALL_PERSIST_KEY;
use codex_tool_planning::RequestPluginInstallElicitationRequest;
use codex_tool_planning::RequestPluginInstallElicitationSchema;
use codex_tool_planning::all_requested_connectors_picked_up;
use codex_tool_planning::verified_connector_install_completed;
use codex_tool_runtime_api::RequestPluginInstallContext;
use codex_tool_runtime_api::RequestPluginInstallElicitationOutcome;
use codex_tool_runtime_api::RequestPluginInstallHost;
use serde_json::Value;
use serde_json::json;
use tracing::warn;

use crate::config::edit::ConfigEdit;
use crate::config::edit::ConfigEditsBuilder;
use crate::connectors;
use crate::function_tool::FunctionCallError;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::tools::handlers::CoreToolDomainHost;

impl RequestPluginInstallHost for CoreToolDomainHost {
    fn request_plugin_install_context(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
    ) -> RequestPluginInstallContext {
        RequestPluginInstallContext {
            server_name: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            thread_id: session.conversation_id.to_string(),
            turn_id: turn.sub_id.clone(),
            app_server_client_name: turn.app_server_client_name.clone(),
        }
    }

    #[expect(
        clippy::await_holding_invalid_type,
        reason = "plugin install discovery reads through the session-owned manager guard"
    )]
    async fn list_request_plugin_install_discoverable_tools(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
    ) -> Result<Vec<DiscoverableTool>, FunctionCallError> {
        let auth_snapshot = match turn.auth_runtime.as_ref() {
            Some(auth_runtime) => auth_runtime.auth().await,
            None => None,
        };
        let connector_auth_context = crate::mcp::codex_apps_auth_context(auth_snapshot.as_ref());
        let manager = session.services.mcp_connection_manager.read().await;
        let mcp_tools = manager.list_all_tools().await;
        drop(manager);
        let accessible_connectors = connectors::with_app_enabled_state(
            connectors::accessible_connectors_from_mcp_tools(&mcp_tools),
            &turn.config,
        );
        connectors::list_tool_suggest_discoverable_tools_with_auth(
            &turn.config,
            session.services.plugins_manager.as_ref(),
            connector_auth_context.as_ref(),
            &accessible_connectors,
        )
        .await
        .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))
    }

    async fn request_plugin_install_elicitation(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        call_id: &str,
        request: RequestPluginInstallElicitationRequest,
        tool: &DiscoverableTool,
    ) -> RequestPluginInstallElicitationOutcome {
        let request_id = RequestId::String(format!("request_plugin_install_{call_id}"));
        let params = request_plugin_install_elicitation_request_to_mcp_params(request);
        let response = session
            .request_mcp_server_elicitation(turn.as_ref(), request_id, params)
            .await;
        if let Some(response) = response.as_ref() {
            maybe_persist_disabled_install_request(session, turn, tool, response).await;
        }

        RequestPluginInstallElicitationOutcome {
            user_confirmed: response
                .as_ref()
                .is_some_and(|response| response.action == ElicitationAction::Accept),
        }
    }

    async fn complete_request_plugin_install_if_ready(
        &self,
        session: &Self::Session,
        turn: &Self::Turn,
        tool: &DiscoverableTool,
    ) -> bool {
        let auth_snapshot = match turn.auth_runtime.as_ref() {
            Some(auth_runtime) => auth_runtime.auth().await,
            None => None,
        };
        let connector_auth_context = crate::mcp::codex_apps_auth_context(auth_snapshot.as_ref());
        let completed = verify_request_plugin_install_completed(
            session,
            turn,
            tool,
            connector_auth_context.as_ref(),
        )
        .await;
        if completed && let DiscoverableTool::Connector(connector) = tool {
            session
                .merge_connector_selection(HashSet::from([connector.id.clone()]))
                .await;
        }

        completed
    }
}

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
    session: &Session,
    turn: &TurnContext,
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
    session: &Session,
    turn: &TurnContext,
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
    session: &Session,
    turn: &TurnContext,
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
