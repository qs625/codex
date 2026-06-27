use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Arc;

use codex_config_types::ToolSuggestDisabledTool;
use codex_connectors_types::AppInfo;
use codex_mcp_types::CodexAppsAuthContext;
use codex_mcp_types::ElicitationAction;
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::McpElicitationObjectType;
use codex_mcp_types::McpElicitationSchema;
use codex_mcp_types::McpServerElicitationRequest;
use codex_mcp_types::McpServerElicitationRequestParams;
use codex_protocol::mcp::RequestId;
use codex_thread_api::RequestPluginInstallApi;
use codex_thread_api::RequestPluginInstallContext;
use codex_thread_api::RequestPluginInstallElicitationOutcome;
use codex_thread_api::SessionCapabilityFuture;
use codex_thread_api::ThreadCapability;
use codex_tool_types::DiscoverableTool;
use codex_tool_types::REQUEST_PLUGIN_INSTALL_PERSIST_ALWAYS_VALUE;
use codex_tool_types::REQUEST_PLUGIN_INSTALL_PERSIST_KEY;
use codex_tool_types::RequestPluginInstallElicitationRequest;
use codex_tool_types::RequestPluginInstallElicitationSchema;
use codex_tool_types::all_requested_connectors_picked_up;
use codex_tool_types::verified_connector_install_completed;
use codex_tool_types::FunctionCallError;
use serde_json::Value;
use serde_json::json;
use tracing::warn;

use crate::config::edit::ConfigEdit;
use crate::config::edit::ConfigEditsBuilder;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;

#[derive(Clone, Default)]
pub struct RequestPluginInstallService;

impl RequestPluginInstallApi for RequestPluginInstallService {
    fn request_plugin_install_context(
        &self,
        capability: &dyn ThreadCapability,
    ) -> RequestPluginInstallContext {
        let turn = turn_context_from_capability(capability);
        turn.request_plugin_install_context(session_from_capability(capability).thread_id())
    }

    fn list_request_plugin_install_discoverable_tools<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
    ) -> SessionCapabilityFuture<'a, Result<Vec<DiscoverableTool>, FunctionCallError>> {
        let session = session_from_capability(capability);
        let turn = turn_context_from_capability(capability);
        Box::pin(async move {
            let auth_snapshot = turn.auth_snapshot().await;
            let connector_auth_context = crate::mcp::codex_apps_auth_context(auth_snapshot.as_ref());
            let mcp_tools = session.list_all_mcp_tools().await;
            let accessible_connectors = turn.accessible_connectors_from_mcp_tools(&mcp_tools);
            turn.list_tool_suggest_discoverable_tools_with_auth(
                session.plugins_manager(),
                connector_auth_context.as_ref(),
                &accessible_connectors,
            )
            .await
            .map_err(|err| FunctionCallError::RespondToModel(err.to_string()))
        })
    }

    fn request_plugin_install_elicitation<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        call_id: &'a str,
        request: RequestPluginInstallElicitationRequest,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, RequestPluginInstallElicitationOutcome> {
        let session = session_from_capability(capability);
        let turn = turn_context_from_capability(capability);
        Box::pin(async move {
            let request_id = RequestId::String(format!("request_plugin_install_{call_id}"));
            let params = request_plugin_install_elicitation_request_to_mcp_params(request);
            let response = session
                .request_mcp_server_elicitation(turn, request_id, params)
                .await;
            if let Some(response) = response.as_ref() {
                maybe_persist_disabled_install_request(session.as_ref(), turn, tool, response)
                    .await;
            }

            RequestPluginInstallElicitationOutcome {
                user_confirmed: response
                    .as_ref()
                    .is_some_and(|response| response.action == ElicitationAction::Accept),
            }
        })
    }

    fn complete_request_plugin_install_if_ready<'a>(
        &'a self,
        capability: &'a dyn ThreadCapability,
        tool: &'a DiscoverableTool,
    ) -> SessionCapabilityFuture<'a, bool> {
        let session = session_from_capability(capability);
        let turn = turn_context_from_capability(capability);
        Box::pin(async move {
            let auth_snapshot = turn.auth_snapshot().await;
            let connector_auth_context = crate::mcp::codex_apps_auth_context(auth_snapshot.as_ref());
            let completed = verify_request_plugin_install_completed(
                session.as_ref(),
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
        })
    }
}

fn session_from_capability(capability: &dyn ThreadCapability) -> Arc<crate::session::session::Session> {
    turn_context_from_capability(capability).session_arc()
}

fn turn_context_from_capability(capability: &dyn ThreadCapability) -> &TurnContext {
    capability
        .as_any()
        .downcast_ref::<TurnContext>()
        .expect("request plugin install capability must be backed by TurnContext")
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

    if let Err(err) = persist_disabled_install_request(turn.codex_home(), tool).await {
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
            let completed = session.configured_plugin_installed(plugin.id.as_str()).await;
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

    let mcp_tools = session.list_all_mcp_tools().await;
    let accessible_connectors = turn.accessible_connectors_from_mcp_tools(&mcp_tools);
    if all_requested_connectors_picked_up(expected_connector_ids, &accessible_connectors) {
        return Some(accessible_connectors);
    }

    match session.hard_refresh_codex_apps_tools_cache().await {
        Ok(mcp_tools) => {
            let accessible_connectors = turn.accessible_connectors_from_mcp_tools(&mcp_tools);
            turn.refresh_accessible_connectors_cache_from_mcp_tools(
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
