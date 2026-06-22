use codex_connectors_types::AppInfo;
use serde::Deserialize;
use serde::Serialize;

use crate::DiscoverableTool;
use crate::DiscoverableToolAction;
use crate::DiscoverableToolType;

pub const REQUEST_PLUGIN_INSTALL_APPROVAL_KIND_VALUE: &str = "tool_suggestion";
pub const REQUEST_PLUGIN_INSTALL_PERSIST_KEY: &str = "persist";
pub const REQUEST_PLUGIN_INSTALL_PERSIST_ALWAYS_VALUE: &str = "always";

#[derive(Debug, Deserialize)]
pub struct RequestPluginInstallArgs {
    pub tool_type: DiscoverableToolType,
    pub action_type: DiscoverableToolAction,
    pub tool_id: String,
    pub suggest_reason: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RequestPluginInstallResult {
    pub completed: bool,
    pub user_confirmed: bool,
    pub tool_type: DiscoverableToolType,
    pub action_type: DiscoverableToolAction,
    pub tool_id: String,
    pub tool_name: String,
    pub suggest_reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RequestPluginInstallMeta {
    pub codex_approval_kind: &'static str,
    pub persist: &'static str,
    pub tool_type: DiscoverableToolType,
    pub suggest_type: DiscoverableToolAction,
    pub suggest_reason: String,
    pub tool_id: String,
    pub tool_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub install_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPluginInstallElicitationRequest {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub server_name: String,
    pub form: RequestPluginInstallElicitationForm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestPluginInstallElicitationForm {
    pub meta: RequestPluginInstallMeta,
    pub message: String,
    pub requested_schema: RequestPluginInstallElicitationSchema,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestPluginInstallElicitationSchema {
    EmptyObject,
}

pub fn build_request_plugin_install_elicitation_request(
    server_name: &str,
    thread_id: String,
    turn_id: String,
    args: &RequestPluginInstallArgs,
    suggest_reason: &str,
    tool: &DiscoverableTool,
) -> RequestPluginInstallElicitationRequest {
    let tool_name = tool.name().to_string();
    let install_url = tool.install_url().map(ToString::to_string);

    RequestPluginInstallElicitationRequest {
        thread_id,
        turn_id: Some(turn_id),
        server_name: server_name.to_string(),
        form: RequestPluginInstallElicitationForm {
            meta: build_request_plugin_install_meta(
                args.tool_type,
                args.action_type,
                suggest_reason,
                tool.id(),
                tool_name,
                install_url,
            ),
            message: suggest_reason.to_string(),
            requested_schema: RequestPluginInstallElicitationSchema::EmptyObject,
        },
    }
}

pub fn all_requested_connectors_picked_up(
    expected_connector_ids: &[String],
    accessible_connectors: &[AppInfo],
) -> bool {
    expected_connector_ids.iter().all(|connector_id| {
        verified_connector_install_completed(connector_id, accessible_connectors)
    })
}

pub fn verified_connector_install_completed(
    tool_id: &str,
    accessible_connectors: &[AppInfo],
) -> bool {
    accessible_connectors
        .iter()
        .find(|connector| connector.id == tool_id)
        .is_some_and(|connector| connector.is_accessible)
}

fn build_request_plugin_install_meta(
    tool_type: DiscoverableToolType,
    action_type: DiscoverableToolAction,
    suggest_reason: &str,
    tool_id: &str,
    tool_name: String,
    install_url: Option<String>,
) -> RequestPluginInstallMeta {
    RequestPluginInstallMeta {
        codex_approval_kind: REQUEST_PLUGIN_INSTALL_APPROVAL_KIND_VALUE,
        persist: REQUEST_PLUGIN_INSTALL_PERSIST_ALWAYS_VALUE,
        tool_type,
        suggest_type: action_type,
        suggest_reason: suggest_reason.to_string(),
        tool_id: tool_id.to_string(),
        tool_name,
        install_url,
    }
}

#[cfg(test)]
#[path = "request_plugin_install_tests.rs"]
mod tests;
