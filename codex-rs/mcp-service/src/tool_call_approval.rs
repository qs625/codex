use std::collections::HashMap;

use config_service::Config;
use config_service::ConfigLayerSource;
use config_service::McpServerConfig;
use config_service::edit::ConfigEdit;
use config_service::edit::ConfigEditsBuilder;
use codex_config_types::AppToolApproval;
use codex_guardian::GuardianApprovalRequest;
use codex_guardian::GuardianMcpAnnotations;
use codex_guardian::guardian_approval_request_to_json;
use mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use mcp_types::MCP_TOOL_CODEX_APPS_META_KEY;
use mcp_types::McpPermissionPromptAutoApproveContext;
use mcp_types::McpToolApprovalDecision;
use mcp_types::McpToolApprovalKey;
use mcp_types::McpToolApprovalMetadata;
use mcp_types::mcp_permission_prompt_is_auto_approved;
use mcp_types::requires_mcp_tool_approval;
use plugin_service_api::PluginRuntime;
use protocol::config_types::ApprovalsReviewer;
use protocol::models::PermissionProfile;
use protocol::protocol::AskForApproval;
use protocol::protocol::McpInvocation;
use protocol::protocol::ReviewDecision;
use protocol::request_user_input::RequestUserInputArgs;
use protocol::request_user_input::RequestUserInputResponse;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use tracing::error;

const MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_DEFAULT: &str = "mcp_tool_call__default";
const MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_ALWAYS_ALLOW: &str = "mcp_tool_call__always_allow";

#[derive(Debug, Clone, Copy)]
pub struct McpToolApprovalRequirementContext<'a> {
    pub approval_policy: AskForApproval,
    pub permission_profile: &'a PermissionProfile,
    pub approvals_reviewer: ApprovalsReviewer,
    pub approval_mode: AppToolApproval,
    pub metadata: Option<&'a McpToolApprovalMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpToolApprovalRequirement {
    NotRequired,
    Required { auto_approved_by_policy: bool },
}

/// Host capabilities needed to apply MCP approval decisions.
///
/// Implementations own the concrete approval memory store, config reload, and
/// persistence target wiring. This lets the MCP runtime own the approval
/// decision flow without depending on the core `Session` type.
pub trait McpToolApprovalPersistenceHost {
    fn remember_mcp_tool_approval(
        &self,
        key: McpToolApprovalKey,
    ) -> impl std::future::Future<Output = ()> + Send;

    fn persist_codex_app_tool_approval(
        &self,
        connector_id: String,
        tool_name: String,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;

    fn persist_non_app_mcp_tool_approval(
        &self,
        server: String,
        tool_name: String,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;

    fn reload_user_config_layer(&self) -> impl std::future::Future<Output = ()> + Send;
}

/// Host capabilities needed to review an MCP tool approval request.
///
/// Implementations own UI, hook, Guardian, and safety-monitor transport. The
/// MCP runtime owns the approval decision order and persistence sequencing.
pub trait McpToolApprovalReviewHost: McpToolApprovalPersistenceHost {
    fn mcp_tool_approval_is_remembered(
        &self,
        key: &McpToolApprovalKey,
    ) -> impl std::future::Future<Output = bool> + Send;

    fn monitor_auto_approved_mcp_tool_call(
        &self,
        action: JsonValue,
        callsite_mode: &'static str,
    ) -> impl std::future::Future<Output = McpToolApprovalMonitorOutcome> + Send;

    fn request_permission_hook(
        &self,
        call_id: &str,
        hook_tool_name: &str,
        tool_input: JsonValue,
    ) -> impl std::future::Future<Output = Option<McpToolApprovalHookDecision>> + Send;

    fn review_guardian_mcp_tool_approval(
        &self,
        request: GuardianApprovalRequest,
        monitor_reason: Option<String>,
    ) -> impl std::future::Future<Output = (ReviewDecision, Option<String>)> + Send;

    fn request_mcp_tool_approval_elicitation(
        &self,
        request_id: protocol::mcp::RequestId,
        params: mcp_types::McpServerElicitationRequestParams,
    ) -> impl std::future::Future<Output = Option<mcp_types::ElicitationResponse>> + Send;

    fn request_user_mcp_tool_approval(
        &self,
        call_id: String,
        args: RequestUserInputArgs,
    ) -> impl std::future::Future<Output = Option<RequestUserInputResponse>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpToolApprovalMonitorOutcome {
    Ok,
    AskUser(String),
    SteerModel(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpToolApprovalHookDecision {
    Allow,
    Deny { message: String },
}

#[derive(Debug, Clone, Copy)]
pub struct McpToolApprovalReviewContext<'a> {
    pub approval_policy: AskForApproval,
    pub permission_profile: &'a PermissionProfile,
    pub approvals_reviewer: ApprovalsReviewer,
    pub approval_mode: AppToolApproval,
    pub tool_call_mcp_elicitation_enabled: bool,
    pub routes_approval_to_guardian: bool,
    pub thread_id: &'a str,
    pub turn_id: Option<&'a str>,
    pub call_id: &'a str,
    pub invocation: &'a McpInvocation,
    pub hook_tool_name: &'a str,
    pub metadata: Option<&'a McpToolApprovalMetadata>,
}

pub fn mcp_tool_approval_requirement(
    context: McpToolApprovalRequirementContext<'_>,
) -> McpToolApprovalRequirement {
    if mcp_permission_prompt_is_auto_approved(
        context.approval_policy,
        context.permission_profile,
        McpPermissionPromptAutoApproveContext {
            approvals_reviewer: Some(context.approvals_reviewer),
            tool_approval_mode: Some(context.approval_mode),
        },
    ) {
        return McpToolApprovalRequirement::NotRequired;
    }

    let annotations = context
        .metadata
        .and_then(|metadata| metadata.annotations.as_ref());
    let approval_required = requires_mcp_tool_approval(annotations);
    if !approval_required && context.approval_mode != AppToolApproval::Prompt {
        return McpToolApprovalRequirement::NotRequired;
    }

    McpToolApprovalRequirement::Required {
        auto_approved_by_policy: context.approval_mode == AppToolApproval::Approve,
    }
}

pub async fn maybe_request_mcp_tool_approval(
    host: &impl McpToolApprovalReviewHost,
    context: McpToolApprovalReviewContext<'_>,
) -> Option<McpToolApprovalDecision> {
    let approval_requirement = mcp_tool_approval_requirement(McpToolApprovalRequirementContext {
        approval_policy: context.approval_policy,
        permission_profile: context.permission_profile,
        approvals_reviewer: context.approvals_reviewer,
        approval_mode: context.approval_mode,
        metadata: context.metadata,
    });
    let auto_approved_by_policy = match approval_requirement {
        McpToolApprovalRequirement::NotRequired => return None,
        McpToolApprovalRequirement::Required {
            auto_approved_by_policy,
        } => auto_approved_by_policy,
    };

    let mut monitor_reason = None;

    if auto_approved_by_policy {
        let action = mcp_tool_approval_arc_monitor_action(context.invocation, context.metadata);
        match host
            .monitor_auto_approved_mcp_tool_call(
                action,
                mcp_tool_approval_callsite_mode(context.approval_mode),
            )
            .await
        {
            McpToolApprovalMonitorOutcome::Ok => return None,
            McpToolApprovalMonitorOutcome::AskUser(reason) => {
                monitor_reason = Some(reason);
            }
            McpToolApprovalMonitorOutcome::SteerModel(reason) => {
                return Some(McpToolApprovalDecision::BlockedBySafetyMonitor(
                    arc_monitor_interrupt_message(&reason),
                ));
            }
        }
    }

    let session_approval_key = mcp_types::session_mcp_tool_approval_key(
        context.invocation,
        context.metadata,
        context.approval_mode,
    );
    let persistent_approval_key = mcp_types::persistent_mcp_tool_approval_key(
        context.invocation,
        context.metadata,
        context.approval_mode,
    );
    if let Some(key) = session_approval_key.as_ref()
        && host.mcp_tool_approval_is_remembered(key).await
    {
        return Some(McpToolApprovalDecision::Accept);
    }

    let tool_input = context
        .invocation
        .arguments
        .clone()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    match host
        .request_permission_hook(context.call_id, context.hook_tool_name, tool_input)
        .await
    {
        Some(McpToolApprovalHookDecision::Allow) => {
            return Some(McpToolApprovalDecision::Accept);
        }
        Some(McpToolApprovalHookDecision::Deny { message }) => {
            return Some(McpToolApprovalDecision::Decline {
                message: Some(message),
            });
        }
        None => {}
    }

    if context.routes_approval_to_guardian {
        let request = build_guardian_mcp_tool_review_request(
            context.call_id,
            context.invocation,
            context.metadata,
        );
        let (decision, decline_message) = host
            .review_guardian_mcp_tool_approval(request, monitor_reason.clone())
            .await;
        let decision = mcp_tool_approval_decision_from_guardian(decision, decline_message);
        apply_mcp_tool_approval_decision(
            host,
            &decision,
            session_approval_key,
            persistent_approval_key,
        )
        .await;
        return Some(decision);
    }

    let prompt_options = mcp_types::mcp_tool_approval_prompt_options(
        session_approval_key.as_ref(),
        persistent_approval_key.as_ref(),
        context.tool_call_mcp_elicitation_enabled,
    );
    let question_id = format!(
        "{}_{}",
        mcp_types::MCP_TOOL_APPROVAL_QUESTION_ID_PREFIX,
        context.call_id
    );
    let rendered_template = mcp_types::render_mcp_tool_approval_template(
        &context.invocation.server,
        context
            .metadata
            .and_then(|metadata| metadata.connector_id.as_deref()),
        context
            .metadata
            .and_then(|metadata| metadata.connector_name.as_deref()),
        context
            .metadata
            .and_then(|metadata| metadata.tool_title.as_deref()),
        context.invocation.arguments.as_ref(),
    );
    let tool_params_display = rendered_template
        .as_ref()
        .map(|rendered_template| rendered_template.tool_params_display.clone())
        .or_else(|| {
            mcp_types::build_mcp_tool_approval_display_params(context.invocation.arguments.as_ref())
        });
    let mut question = mcp_types::build_mcp_tool_approval_question(
        question_id.clone(),
        &context.invocation.server,
        &context.invocation.tool,
        context
            .metadata
            .and_then(|metadata| metadata.connector_name.as_deref()),
        prompt_options,
        rendered_template
            .as_ref()
            .map(|rendered_template| rendered_template.question.as_str()),
    );
    question.question =
        mcp_types::mcp_tool_approval_question_text(question.question, monitor_reason.as_deref());

    if context.tool_call_mcp_elicitation_enabled {
        let request_id = protocol::mcp::RequestId::String(format!(
            "{}_{}",
            mcp_types::MCP_TOOL_APPROVAL_QUESTION_ID_PREFIX,
            context.call_id
        ));
        let params = mcp_types::build_mcp_tool_approval_elicitation_request(
            mcp_types::McpToolApprovalElicitationRequest {
                thread_id: context.thread_id,
                turn_id: context.turn_id,
                server: &context.invocation.server,
                metadata: context.metadata,
                tool_params: rendered_template
                    .as_ref()
                    .and_then(|rendered_template| rendered_template.tool_params.as_ref())
                    .or(context.invocation.arguments.as_ref()),
                tool_params_display: tool_params_display.as_deref(),
                question,
                message_override: rendered_template.as_ref().and_then(|rendered_template| {
                    monitor_reason
                        .is_none()
                        .then_some(rendered_template.elicitation_message.as_str())
                }),
                prompt_options,
            },
        );
        let decision = mcp_types::parse_mcp_tool_approval_elicitation_response(
            host.request_mcp_tool_approval_elicitation(request_id, params)
                .await,
            &question_id,
        );
        let decision =
            mcp_types::normalize_approval_decision_for_mode(decision, context.approval_mode);
        apply_mcp_tool_approval_decision(
            host,
            &decision,
            session_approval_key,
            persistent_approval_key,
        )
        .await;
        return Some(decision);
    }

    let decision = mcp_types::normalize_approval_decision_for_mode(
        mcp_types::parse_mcp_tool_approval_response(
            host.request_user_mcp_tool_approval(
                context.call_id.to_string(),
                RequestUserInputArgs {
                    questions: vec![question],
                },
            )
            .await,
            &question_id,
        ),
        context.approval_mode,
    );
    apply_mcp_tool_approval_decision(
        host,
        &decision,
        session_approval_key,
        persistent_approval_key,
    )
    .await;
    Some(decision)
}

pub async fn custom_mcp_tool_approval_mode(
    config: &Config,
    plugins_runtime: &dyn PluginRuntime,
    server: &str,
    tool_name: &str,
) -> AppToolApproval {
    let user_configured_mode = config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|table| table.get("mcp_servers"))
        .cloned()
        .and_then(|value| HashMap::<String, McpServerConfig>::deserialize(value).ok())
        .and_then(|servers| {
            let server_config = servers.get(server)?;
            Some(
                server_config
                    .tools
                    .get(tool_name)
                    .and_then(|tool| tool.approval_mode)
                    .or(server_config.default_tools_approval_mode)
                    .unwrap_or_default(),
            )
        });
    if let Some(user_configured_mode) = user_configured_mode {
        return user_configured_mode;
    }

    plugins_runtime
        .plugins_for_config(&config.plugins_config_input())
        .await
        .plugins()
        .iter()
        .filter(|plugin| plugin.is_active())
        .find_map(|plugin| {
            let server_config = plugin.mcp_servers.get(server)?;
            server_config
                .tools
                .get(tool_name)
                .and_then(|tool| tool.approval_mode)
                .or(server_config.default_tools_approval_mode)
        })
        .unwrap_or_default()
}

pub fn build_mcp_tool_call_request_meta(
    turn_metadata: Option<JsonValue>,
    server: &str,
    call_id: &str,
    metadata: Option<&McpToolApprovalMetadata>,
    turn_metadata_header_name: &str,
) -> Option<JsonValue> {
    let mut request_meta = serde_json::Map::new();

    if let Some(turn_metadata) = turn_metadata {
        request_meta.insert(turn_metadata_header_name.to_string(), turn_metadata);
    }

    if server == CODEX_APPS_MCP_SERVER_NAME {
        let mut codex_apps_meta = metadata
            .and_then(|metadata| metadata.codex_apps_meta.clone())
            .unwrap_or_default();
        codex_apps_meta.insert(
            "call_id".to_string(),
            serde_json::Value::String(call_id.to_string()),
        );
        request_meta.insert(
            MCP_TOOL_CODEX_APPS_META_KEY.to_string(),
            serde_json::Value::Object(codex_apps_meta),
        );
    }

    (!request_meta.is_empty()).then_some(serde_json::Value::Object(request_meta))
}

pub fn build_guardian_mcp_tool_review_request(
    call_id: &str,
    invocation: &McpInvocation,
    metadata: Option<&McpToolApprovalMetadata>,
) -> GuardianApprovalRequest {
    GuardianApprovalRequest::McpToolCall {
        id: call_id.to_string(),
        server: invocation.server.clone(),
        tool_name: invocation.tool.clone(),
        arguments: invocation.arguments.clone(),
        connector_id: metadata.and_then(|metadata| metadata.connector_id.clone()),
        connector_name: metadata.and_then(|metadata| metadata.connector_name.clone()),
        connector_description: metadata.and_then(|metadata| metadata.connector_description.clone()),
        tool_title: metadata.and_then(|metadata| metadata.tool_title.clone()),
        tool_description: metadata.and_then(|metadata| metadata.tool_description.clone()),
        annotations: metadata
            .and_then(|metadata| metadata.annotations.as_ref())
            .map(|annotations| GuardianMcpAnnotations {
                destructive_hint: annotations.destructive_hint,
                open_world_hint: annotations.open_world_hint,
                read_only_hint: annotations.read_only_hint,
            }),
    }
}

pub fn mcp_tool_approval_arc_monitor_action(
    invocation: &McpInvocation,
    metadata: Option<&McpToolApprovalMetadata>,
) -> serde_json::Value {
    let request = build_guardian_mcp_tool_review_request("arc-monitor", invocation, metadata);
    match guardian_approval_request_to_json(&request) {
        Ok(action) => action,
        Err(error) => {
            error!(error = %error, "failed to serialize guardian MCP approval request for ARC");
            serde_json::Value::Null
        }
    }
}

pub fn mcp_tool_approval_callsite_mode(approval_mode: AppToolApproval) -> &'static str {
    match approval_mode {
        AppToolApproval::Approve => MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_ALWAYS_ALLOW,
        AppToolApproval::Auto | AppToolApproval::Prompt => {
            MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_DEFAULT
        }
    }
}

pub fn mcp_tool_approval_decision_from_guardian(
    decision: ReviewDecision,
    decline_message: Option<String>,
) -> McpToolApprovalDecision {
    match decision {
        ReviewDecision::Approved
        | ReviewDecision::ApprovedExecpolicyAmendment { .. }
        | ReviewDecision::NetworkPolicyAmendment { .. } => McpToolApprovalDecision::Accept,
        ReviewDecision::ApprovedForSession => McpToolApprovalDecision::AcceptForSession,
        ReviewDecision::Denied | ReviewDecision::TimedOut => McpToolApprovalDecision::Decline {
            message: decline_message,
        },
        ReviewDecision::Abort => McpToolApprovalDecision::Decline { message: None },
    }
}

pub fn arc_monitor_interrupt_message(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "Tool call was cancelled because of safety risks.".to_string()
    } else {
        format!("Tool call was cancelled because of safety risks: {reason}")
    }
}

pub async fn persist_codex_app_tool_approval(
    config: &Config,
    connector_id: &str,
    tool_name: &str,
) -> anyhow::Result<()> {
    ConfigEditsBuilder::for_config(config)
        .with_edits([ConfigEdit::set_string_path(
            vec![
                "apps".to_string(),
                connector_id.to_string(),
                "tools".to_string(),
                tool_name.to_string(),
                "approval_mode".to_string(),
            ],
            "approve",
        )])
        .apply()
        .await
}

pub async fn persist_non_app_mcp_tool_approval(
    config: &Config,
    plugins_runtime: &dyn PluginRuntime,
    server: &str,
    tool_name: &str,
) -> anyhow::Result<()> {
    if let Some(config_edits_builder) = custom_mcp_tool_approval_config_builder(config, server)? {
        return persist_custom_mcp_tool_approval_with(config_edits_builder, server, tool_name)
            .await;
    }

    let plugin_config_name = plugins_runtime
        .plugins_for_config(&config.plugins_config_input())
        .await
        .plugins()
        .iter()
        .filter(|plugin| plugin.is_active())
        .find(|plugin| plugin.mcp_servers.contains_key(server))
        .map(|plugin| plugin.config_name.clone());

    if let Some(plugin_config_name) = plugin_config_name {
        return ConfigEditsBuilder::for_config(config)
            .with_edits([ConfigEdit::set_string_path(
                vec![
                    "plugins".to_string(),
                    plugin_config_name,
                    "mcp_servers".to_string(),
                    server.to_string(),
                    "tools".to_string(),
                    tool_name.to_string(),
                    "approval_mode".to_string(),
                ],
                "approve",
            )])
            .apply()
            .await;
    }

    anyhow::bail!("MCP server `{server}` is not configured in config.toml or an enabled plugin")
}

pub async fn apply_mcp_tool_approval_decision(
    host: &impl McpToolApprovalPersistenceHost,
    decision: &McpToolApprovalDecision,
    session_approval_key: Option<McpToolApprovalKey>,
    persistent_approval_key: Option<McpToolApprovalKey>,
) {
    match decision {
        McpToolApprovalDecision::AcceptForSession => {
            if let Some(key) = session_approval_key {
                host.remember_mcp_tool_approval(key).await;
            }
        }
        McpToolApprovalDecision::AcceptAndRemember => {
            if let Some(key) = persistent_approval_key {
                maybe_persist_mcp_tool_approval(host, key).await;
            } else if let Some(key) = session_approval_key {
                host.remember_mcp_tool_approval(key).await;
            }
        }
        McpToolApprovalDecision::Accept
        | McpToolApprovalDecision::Decline { .. }
        | McpToolApprovalDecision::Cancel
        | McpToolApprovalDecision::BlockedBySafetyMonitor(_) => {}
    }
}

pub async fn maybe_persist_mcp_tool_approval(
    host: &impl McpToolApprovalPersistenceHost,
    key: McpToolApprovalKey,
) {
    let tool_name = key.tool_name.clone();

    let persist_result = if key.server == CODEX_APPS_MCP_SERVER_NAME {
        let Some(connector_id) = key.connector_id.clone() else {
            host.remember_mcp_tool_approval(key).await;
            return;
        };
        host.persist_codex_app_tool_approval(connector_id, tool_name.clone())
            .await
    } else {
        host.persist_non_app_mcp_tool_approval(key.server.clone(), tool_name.clone())
            .await
    };

    if let Err(err) = persist_result {
        error!(
            error = %err,
            server = key.server,
            tool_name,
            "failed to persist MCP tool approval"
        );
        host.remember_mcp_tool_approval(key).await;
        return;
    }

    host.reload_user_config_layer().await;
    host.remember_mcp_tool_approval(key).await;
}

fn custom_mcp_tool_approval_config_builder(
    config: &Config,
    server: &str,
) -> anyhow::Result<Option<ConfigEditsBuilder>> {
    if let Some(project_config_folder) = project_mcp_tool_approval_config_folder(config, server) {
        return Ok(Some(ConfigEditsBuilder::new(
            project_config_folder.as_path(),
        )));
    }

    Ok(user_mcp_server_is_configured(config, server)?
        .then(|| ConfigEditsBuilder::for_config(config)))
}

async fn persist_custom_mcp_tool_approval_with(
    config_edits_builder: ConfigEditsBuilder,
    server: &str,
    tool_name: &str,
) -> anyhow::Result<()> {
    config_edits_builder
        .with_edits([ConfigEdit::set_string_path(
            vec![
                "mcp_servers".to_string(),
                server.to_string(),
                "tools".to_string(),
                tool_name.to_string(),
                "approval_mode".to_string(),
            ],
            "approve",
        )])
        .apply()
        .await
}

fn user_mcp_server_is_configured(config: &Config, server: &str) -> anyhow::Result<bool> {
    let Some(mcp_servers_toml) = config
        .config_layer_stack
        .effective_user_config()
        .as_ref()
        .and_then(|user_config| user_config.get("mcp_servers"))
        .cloned()
    else {
        return Ok(false);
    };
    let servers = HashMap::<String, McpServerConfig>::deserialize(mcp_servers_toml)?;
    Ok(servers.contains_key(server))
}

fn project_mcp_tool_approval_config_folder(
    config: &Config,
    server: &str,
) -> Option<config_service::AbsolutePathBuf> {
    config
        .config_layer_stack
        .layers_high_to_low()
        .into_iter()
        .find_map(|layer| {
            if !matches!(layer.name, ConfigLayerSource::Project { .. }) {
                return None;
            }

            let servers = layer
                .config
                .as_table()
                .and_then(|table| table.get("mcp_servers"))
                .cloned()
                .and_then(|value| HashMap::<String, McpServerConfig>::deserialize(value).ok())?;
            if servers.contains_key(server) {
                layer.config_folder()
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use config_service::AbsolutePathBuf;
    use config_service::CONFIG_TOML_FILE;
    use config_service::ConfigBuilder;
    use config_service::config_toml::ConfigToml;
    use config_service::types::AppConfig;
    use config_service::types::AppToolConfig;
    use config_service::types::AppToolsConfig;
    use config_service::types::AppsConfigToml;
    use config_service::types::McpServerToolConfig;
    use mcp_types::ToolAnnotations;
    use plugin_service_api::LoadedPlugin;
    use plugin_service_api::PluginLoadOutcome;
    use plugin_service_api::PluginRuntime;
    use plugin_service_api::PluginRuntimeFuture;
    use plugin_service_api::PluginsConfigInput;
    use plugin_service_api::ToolSuggestDiscoverablePlugin;
    use protocol::models::ManagedFileSystemPermissions;
    use protocol::permissions::NetworkSandboxPolicy;
    use serde::Deserialize;
    use std::collections::HashMap;
    use std::collections::HashSet;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[test]
    fn guardian_review_decision_maps_to_mcp_tool_decision() {
        assert_eq!(
            mcp_tool_approval_decision_from_guardian(ReviewDecision::Approved, None),
            McpToolApprovalDecision::Accept
        );
        assert_eq!(
            mcp_tool_approval_decision_from_guardian(ReviewDecision::ApprovedForSession, None),
            McpToolApprovalDecision::AcceptForSession
        );
        assert_eq!(
            mcp_tool_approval_decision_from_guardian(
                ReviewDecision::Denied,
                Some("Reason: too risky".to_string()),
            ),
            McpToolApprovalDecision::Decline {
                message: Some("Reason: too risky".to_string())
            }
        );
        assert_eq!(
            mcp_tool_approval_decision_from_guardian(
                ReviewDecision::TimedOut,
                Some("Review timed out".to_string()),
            ),
            McpToolApprovalDecision::Decline {
                message: Some("Review timed out".to_string())
            }
        );
        assert_eq!(
            mcp_tool_approval_decision_from_guardian(ReviewDecision::Abort, None),
            McpToolApprovalDecision::Decline { message: None }
        );
    }

    #[test]
    fn guardian_mcp_review_request_includes_invocation_metadata() {
        let invocation = McpInvocation {
            server: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            tool: "browser_navigate".to_string(),
            arguments: Some(serde_json::json!({
                "url": "https://example.com",
            })),
        };

        let request = build_guardian_mcp_tool_review_request(
            "call-1",
            &invocation,
            Some(&approval_metadata(
                Some("playwright"),
                Some("Playwright"),
                Some("Browser automation"),
                Some("Navigate"),
                Some("Open a page"),
            )),
        );

        assert_eq!(
            request,
            GuardianApprovalRequest::McpToolCall {
                id: "call-1".to_string(),
                server: CODEX_APPS_MCP_SERVER_NAME.to_string(),
                tool_name: "browser_navigate".to_string(),
                arguments: Some(serde_json::json!({
                    "url": "https://example.com",
                })),
                connector_id: Some("playwright".to_string()),
                connector_name: Some("Playwright".to_string()),
                connector_description: Some("Browser automation".to_string()),
                tool_title: Some("Navigate".to_string()),
                tool_description: Some("Open a page".to_string()),
                annotations: None,
            }
        );
    }

    #[test]
    fn guardian_mcp_review_request_includes_annotations_when_present() {
        let invocation = McpInvocation {
            server: "custom_server".to_string(),
            tool: "dangerous_tool".to_string(),
            arguments: None,
        };
        let metadata = metadata_with_annotations(Some(ToolAnnotations {
            destructive_hint: Some(true),
            idempotent_hint: None,
            open_world_hint: Some(true),
            read_only_hint: Some(false),
            title: None,
        }));

        let request =
            build_guardian_mcp_tool_review_request("call-1", &invocation, Some(&metadata));

        assert_eq!(
            request,
            GuardianApprovalRequest::McpToolCall {
                id: "call-1".to_string(),
                server: "custom_server".to_string(),
                tool_name: "dangerous_tool".to_string(),
                arguments: None,
                connector_id: None,
                connector_name: None,
                connector_description: None,
                tool_title: None,
                tool_description: None,
                annotations: Some(GuardianMcpAnnotations {
                    destructive_hint: Some(true),
                    open_world_hint: Some(true),
                    read_only_hint: Some(false),
                }),
            }
        );
    }

    #[test]
    fn arc_monitor_action_serializes_mcp_tool_call_shape() {
        let invocation = McpInvocation {
            server: CODEX_APPS_MCP_SERVER_NAME.to_string(),
            tool: "browser_navigate".to_string(),
            arguments: Some(serde_json::json!({
                "url": "https://example.com",
            })),
        };

        let action = mcp_tool_approval_arc_monitor_action(
            &invocation,
            Some(&approval_metadata(
                /*connector_id*/ None,
                Some("Playwright"),
                /*connector_description*/ None,
                Some("Navigate"),
                /*tool_description*/ None,
            )),
        );

        assert_eq!(
            action,
            serde_json::json!({
                "tool": "mcp_tool_call",
                "server": CODEX_APPS_MCP_SERVER_NAME,
                "tool_name": "browser_navigate",
                "arguments": {
                    "url": "https://example.com",
                },
                "connector_name": "Playwright",
                "tool_title": "Navigate",
            })
        );
    }

    #[test]
    fn approval_callsite_mode_distinguishes_default_and_always_allow() {
        assert_eq!(
            mcp_tool_approval_callsite_mode(AppToolApproval::Auto),
            MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_DEFAULT
        );
        assert_eq!(
            mcp_tool_approval_callsite_mode(AppToolApproval::Prompt),
            MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_DEFAULT
        );
        assert_eq!(
            mcp_tool_approval_callsite_mode(AppToolApproval::Approve),
            MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_ALWAYS_ALLOW
        );
    }

    fn metadata_with_annotations(annotations: Option<ToolAnnotations>) -> McpToolApprovalMetadata {
        McpToolApprovalMetadata {
            annotations,
            connector_id: None,
            connector_name: None,
            connector_description: None,
            tool_title: None,
            tool_description: None,
            mcp_app_resource_uri: None,
            codex_apps_meta: None,
            openai_file_input_params: None,
        }
    }

    fn approval_metadata(
        connector_id: Option<&str>,
        connector_name: Option<&str>,
        connector_description: Option<&str>,
        tool_title: Option<&str>,
        tool_description: Option<&str>,
    ) -> McpToolApprovalMetadata {
        McpToolApprovalMetadata {
            annotations: None,
            connector_id: connector_id.map(str::to_string),
            connector_name: connector_name.map(str::to_string),
            connector_description: connector_description.map(str::to_string),
            tool_title: tool_title.map(str::to_string),
            tool_description: tool_description.map(str::to_string),
            mcp_app_resource_uri: None,
            codex_apps_meta: None,
            openai_file_input_params: None,
        }
    }

    fn approval_requirement_context<'a>(
        approval_mode: AppToolApproval,
        permission_profile: &'a PermissionProfile,
        metadata: Option<&'a McpToolApprovalMetadata>,
    ) -> McpToolApprovalRequirementContext<'a> {
        McpToolApprovalRequirementContext {
            approval_policy: AskForApproval::OnRequest,
            permission_profile,
            approvals_reviewer: ApprovalsReviewer::User,
            approval_mode,
            metadata,
        }
    }

    fn approval_review_context<'a>(
        approval_mode: AppToolApproval,
        permission_profile: &'a PermissionProfile,
        invocation: &'a McpInvocation,
        metadata: Option<&'a McpToolApprovalMetadata>,
    ) -> McpToolApprovalReviewContext<'a> {
        McpToolApprovalReviewContext {
            approval_policy: AskForApproval::OnRequest,
            permission_profile,
            approvals_reviewer: ApprovalsReviewer::User,
            approval_mode,
            tool_call_mcp_elicitation_enabled: false,
            routes_approval_to_guardian: false,
            thread_id: "thread-1",
            turn_id: Some("turn-1"),
            call_id: "call-1",
            invocation,
            hook_tool_name: "mcp__server__tool",
            metadata,
        }
    }

    struct FakeApprovalReviewHost {
        remembered_keys: Mutex<Vec<McpToolApprovalKey>>,
        remembered_writes: Mutex<Vec<McpToolApprovalKey>>,
        hook_decision: Mutex<Option<McpToolApprovalHookDecision>>,
        monitor_outcome: Mutex<McpToolApprovalMonitorOutcome>,
        guardian_decision: Mutex<ReviewDecision>,
        guardian_decline_message: Mutex<Option<String>>,
        monitor_calls: Mutex<usize>,
        hook_calls: Mutex<usize>,
        guardian_calls: Mutex<usize>,
        user_prompt_calls: Mutex<usize>,
    }

    impl Default for FakeApprovalReviewHost {
        fn default() -> Self {
            Self {
                remembered_keys: Mutex::new(Vec::new()),
                remembered_writes: Mutex::new(Vec::new()),
                hook_decision: Mutex::new(None),
                monitor_outcome: Mutex::new(McpToolApprovalMonitorOutcome::Ok),
                guardian_decision: Mutex::new(ReviewDecision::Approved),
                guardian_decline_message: Mutex::new(None),
                monitor_calls: Mutex::new(0),
                hook_calls: Mutex::new(0),
                guardian_calls: Mutex::new(0),
                user_prompt_calls: Mutex::new(0),
            }
        }
    }

    impl McpToolApprovalPersistenceHost for FakeApprovalReviewHost {
        async fn remember_mcp_tool_approval(&self, key: McpToolApprovalKey) {
            self.remembered_writes.lock().unwrap().push(key);
        }

        async fn persist_codex_app_tool_approval(
            &self,
            _connector_id: String,
            _tool_name: String,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn persist_non_app_mcp_tool_approval(
            &self,
            _server: String,
            _tool_name: String,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn reload_user_config_layer(&self) {}
    }

    impl McpToolApprovalReviewHost for FakeApprovalReviewHost {
        async fn mcp_tool_approval_is_remembered(&self, key: &McpToolApprovalKey) -> bool {
            self.remembered_keys.lock().unwrap().contains(key)
        }

        async fn monitor_auto_approved_mcp_tool_call(
            &self,
            _action: JsonValue,
            _callsite_mode: &'static str,
        ) -> McpToolApprovalMonitorOutcome {
            *self.monitor_calls.lock().unwrap() += 1;
            self.monitor_outcome.lock().unwrap().clone()
        }

        async fn request_permission_hook(
            &self,
            _call_id: &str,
            _hook_tool_name: &str,
            _tool_input: JsonValue,
        ) -> Option<McpToolApprovalHookDecision> {
            *self.hook_calls.lock().unwrap() += 1;
            self.hook_decision.lock().unwrap().clone()
        }

        async fn review_guardian_mcp_tool_approval(
            &self,
            _request: GuardianApprovalRequest,
            _monitor_reason: Option<String>,
        ) -> (ReviewDecision, Option<String>) {
            *self.guardian_calls.lock().unwrap() += 1;
            (
                self.guardian_decision.lock().unwrap().clone(),
                self.guardian_decline_message.lock().unwrap().clone(),
            )
        }

        async fn request_mcp_tool_approval_elicitation(
            &self,
            _request_id: protocol::mcp::RequestId,
            _params: mcp_types::McpServerElicitationRequestParams,
        ) -> Option<mcp_types::ElicitationResponse> {
            None
        }

        async fn request_user_mcp_tool_approval(
            &self,
            _call_id: String,
            _args: RequestUserInputArgs,
        ) -> Option<RequestUserInputResponse> {
            *self.user_prompt_calls.lock().unwrap() += 1;
            None
        }
    }

    #[tokio::test]
    async fn maybe_request_mcp_tool_approval_accepts_remembered_session_decision() {
        let permission_profile = PermissionProfile::read_only();
        let invocation = McpInvocation {
            server: "custom_server".to_string(),
            tool: "dangerous_tool".to_string(),
            arguments: Some(serde_json::json!({"path": "/tmp/file"})),
        };
        let metadata = metadata_with_annotations(Some(ToolAnnotations {
            destructive_hint: Some(true),
            idempotent_hint: None,
            open_world_hint: Some(true),
            read_only_hint: Some(false),
            title: None,
        }));
        let host = FakeApprovalReviewHost::default();
        host.remembered_keys.lock().unwrap().push(
            mcp_types::session_mcp_tool_approval_key(
                &invocation,
                Some(&metadata),
                AppToolApproval::Auto,
            )
            .expect("session approval key"),
        );

        let decision = maybe_request_mcp_tool_approval(
            &host,
            approval_review_context(
                AppToolApproval::Auto,
                &permission_profile,
                &invocation,
                Some(&metadata),
            ),
        )
        .await;

        assert_eq!(decision, Some(McpToolApprovalDecision::Accept));
        assert_eq!(*host.hook_calls.lock().unwrap(), 0);
        assert_eq!(*host.user_prompt_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn maybe_request_mcp_tool_approval_honors_hook_deny() {
        let permission_profile = PermissionProfile::read_only();
        let invocation = McpInvocation {
            server: "custom_server".to_string(),
            tool: "dangerous_tool".to_string(),
            arguments: Some(serde_json::json!({"path": "/tmp/file"})),
        };
        let metadata = metadata_with_annotations(Some(ToolAnnotations {
            destructive_hint: Some(true),
            idempotent_hint: None,
            open_world_hint: Some(true),
            read_only_hint: Some(false),
            title: None,
        }));
        let host = FakeApprovalReviewHost::default();
        *host.hook_decision.lock().unwrap() = Some(McpToolApprovalHookDecision::Deny {
            message: "blocked by hook".to_string(),
        });

        let decision = maybe_request_mcp_tool_approval(
            &host,
            approval_review_context(
                AppToolApproval::Auto,
                &permission_profile,
                &invocation,
                Some(&metadata),
            ),
        )
        .await;

        assert_eq!(
            decision,
            Some(McpToolApprovalDecision::Decline {
                message: Some("blocked by hook".to_string())
            })
        );
        assert_eq!(*host.hook_calls.lock().unwrap(), 1);
        assert_eq!(*host.guardian_calls.lock().unwrap(), 0);
        assert_eq!(*host.user_prompt_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn maybe_request_mcp_tool_approval_routes_guardian_decision() {
        let permission_profile = PermissionProfile::read_only();
        let invocation = McpInvocation {
            server: "custom_server".to_string(),
            tool: "dangerous_tool".to_string(),
            arguments: Some(serde_json::json!({"path": "/tmp/file"})),
        };
        let metadata = metadata_with_annotations(Some(ToolAnnotations {
            destructive_hint: Some(true),
            idempotent_hint: None,
            open_world_hint: Some(true),
            read_only_hint: Some(false),
            title: None,
        }));
        let host = FakeApprovalReviewHost::default();
        *host.guardian_decision.lock().unwrap() = ReviewDecision::Denied;
        *host.guardian_decline_message.lock().unwrap() = Some("guardian blocked".to_string());
        let mut context = approval_review_context(
            AppToolApproval::Auto,
            &permission_profile,
            &invocation,
            Some(&metadata),
        );
        context.routes_approval_to_guardian = true;

        let decision = maybe_request_mcp_tool_approval(&host, context).await;

        assert_eq!(
            decision,
            Some(McpToolApprovalDecision::Decline {
                message: Some("guardian blocked".to_string())
            })
        );
        assert_eq!(*host.hook_calls.lock().unwrap(), 1);
        assert_eq!(*host.guardian_calls.lock().unwrap(), 1);
        assert_eq!(*host.user_prompt_calls.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn maybe_request_mcp_tool_approval_auto_approve_skips_external_review() {
        let permission_profile = PermissionProfile::read_only();
        let invocation = McpInvocation {
            server: "custom_server".to_string(),
            tool: "dangerous_tool".to_string(),
            arguments: Some(serde_json::json!({"path": "/tmp/file"})),
        };
        let metadata = metadata_with_annotations(Some(ToolAnnotations {
            destructive_hint: Some(true),
            idempotent_hint: None,
            open_world_hint: Some(true),
            read_only_hint: Some(false),
            title: None,
        }));
        let host = FakeApprovalReviewHost::default();
        let mut context = approval_review_context(
            AppToolApproval::Approve,
            &permission_profile,
            &invocation,
            Some(&metadata),
        );
        context.routes_approval_to_guardian = true;

        let decision = maybe_request_mcp_tool_approval(&host, context).await;

        assert_eq!(decision, None);
        assert_eq!(*host.monitor_calls.lock().unwrap(), 0);
        assert_eq!(*host.hook_calls.lock().unwrap(), 0);
        assert_eq!(*host.guardian_calls.lock().unwrap(), 0);
        assert_eq!(*host.user_prompt_calls.lock().unwrap(), 0);
    }

    #[derive(Clone, Default)]
    struct FakePluginRuntime {
        outcome: PluginLoadOutcome,
    }

    impl FakePluginRuntime {
        fn with_plugin_mcp_config(
            config_name: &str,
            server_name: &str,
            server_config: McpServerConfig,
        ) -> Self {
            Self {
                outcome: PluginLoadOutcome::from_plugins(vec![LoadedPlugin {
                    config_name: config_name.to_string(),
                    manifest_name: None,
                    manifest_description: None,
                    root: AbsolutePathBuf::from_absolute_path(std::env::temp_dir())
                        .expect("temp dir should be absolute"),
                    enabled: true,
                    skill_roots: Vec::new(),
                    disabled_skill_paths: HashSet::new(),
                    has_enabled_skills: false,
                    mcp_servers: HashMap::from([(server_name.to_string(), server_config)]),
                    apps: Vec::new(),
                    hook_sources: Vec::new(),
                    hook_load_warnings: Vec::new(),
                    error: None,
                }]),
            }
        }
    }

    impl PluginRuntime for FakePluginRuntime {
        fn plugins_for_config<'a>(
            &'a self,
            _config: &'a PluginsConfigInput,
        ) -> PluginRuntimeFuture<'a, PluginLoadOutcome> {
            Box::pin(async { self.outcome.clone() })
        }

        fn is_configured_plugin_installed(
            &self,
            _config: &PluginsConfigInput,
            _plugin_id: &str,
        ) -> bool {
            false
        }

        fn list_tool_suggest_discoverable_plugins<'a>(
            &'a self,
            _config: &'a PluginsConfigInput,
            _configured_plugin_ids: &'a HashSet<String>,
            _disabled_plugin_ids: &'a HashSet<String>,
        ) -> PluginRuntimeFuture<'a, Result<Vec<ToolSuggestDiscoverablePlugin>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn clear_cache(&self) {}
    }

    #[derive(Default)]
    struct FakeApprovalPersistenceHost {
        remembered: Mutex<Vec<McpToolApprovalKey>>,
        codex_app_persisted: Mutex<Vec<(String, String)>>,
        non_app_persisted: Mutex<Vec<(String, String)>>,
        reload_count: Mutex<usize>,
        fail_persist: bool,
    }

    impl FakeApprovalPersistenceHost {
        fn remembered(&self) -> Vec<McpToolApprovalKey> {
            self.remembered.lock().expect("remembered lock").clone()
        }

        fn codex_app_persisted(&self) -> Vec<(String, String)> {
            self.codex_app_persisted
                .lock()
                .expect("codex app persisted lock")
                .clone()
        }

        fn non_app_persisted(&self) -> Vec<(String, String)> {
            self.non_app_persisted
                .lock()
                .expect("non-app persisted lock")
                .clone()
        }

        fn reload_count(&self) -> usize {
            *self.reload_count.lock().expect("reload count lock")
        }
    }

    impl McpToolApprovalPersistenceHost for FakeApprovalPersistenceHost {
        async fn remember_mcp_tool_approval(&self, key: McpToolApprovalKey) {
            self.remembered.lock().expect("remembered lock").push(key);
        }

        async fn persist_codex_app_tool_approval(
            &self,
            connector_id: String,
            tool_name: String,
        ) -> anyhow::Result<()> {
            if self.fail_persist {
                anyhow::bail!("persist failed");
            }
            self.codex_app_persisted
                .lock()
                .expect("codex app persisted lock")
                .push((connector_id, tool_name));
            Ok(())
        }

        async fn persist_non_app_mcp_tool_approval(
            &self,
            server: String,
            tool_name: String,
        ) -> anyhow::Result<()> {
            if self.fail_persist {
                anyhow::bail!("persist failed");
            }
            self.non_app_persisted
                .lock()
                .expect("non-app persisted lock")
                .push((server, tool_name));
            Ok(())
        }

        async fn reload_user_config_layer(&self) {
            *self.reload_count.lock().expect("reload count lock") += 1;
        }
    }

    fn approval_key(
        server: &str,
        connector_id: Option<&str>,
        tool_name: &str,
    ) -> McpToolApprovalKey {
        McpToolApprovalKey {
            server: server.to_string(),
            connector_id: connector_id.map(str::to_string),
            tool_name: tool_name.to_string(),
        }
    }

    fn mcp_server_config_from_toml(toml: &str, server: &str) -> McpServerConfig {
        let table = toml::from_str::<toml::Value>(toml).expect("parse MCP server TOML");
        let servers = table
            .get("mcp_servers")
            .cloned()
            .expect("mcp_servers table");
        HashMap::<String, McpServerConfig>::deserialize(servers)
            .expect("deserialize MCP servers")
            .remove(server)
            .expect("server config")
    }

    #[tokio::test]
    async fn approval_decision_accept_for_session_remembers_session_key() {
        let host = FakeApprovalPersistenceHost::default();
        let session_key = approval_key("docs", /*connector_id*/ None, "search");

        apply_mcp_tool_approval_decision(
            &host,
            &McpToolApprovalDecision::AcceptForSession,
            Some(session_key.clone()),
            /*persistent_approval_key*/ None,
        )
        .await;

        assert_eq!(host.remembered(), vec![session_key]);
        assert!(host.codex_app_persisted().is_empty());
        assert!(host.non_app_persisted().is_empty());
        assert_eq!(host.reload_count(), 0);
    }

    #[tokio::test]
    async fn approval_decision_accept_and_remember_persists_then_remembers() {
        let host = FakeApprovalPersistenceHost::default();
        let session_key = approval_key("docs", /*connector_id*/ None, "search");
        let persistent_key = approval_key("docs", /*connector_id*/ None, "search");

        apply_mcp_tool_approval_decision(
            &host,
            &McpToolApprovalDecision::AcceptAndRemember,
            Some(session_key),
            Some(persistent_key.clone()),
        )
        .await;

        assert_eq!(
            host.non_app_persisted(),
            vec![("docs".to_string(), "search".to_string())]
        );
        assert_eq!(host.reload_count(), 1);
        assert_eq!(host.remembered(), vec![persistent_key]);
    }

    #[tokio::test]
    async fn approval_decision_accept_and_remember_without_persistent_key_uses_session_key() {
        let host = FakeApprovalPersistenceHost::default();
        let session_key = approval_key("docs", /*connector_id*/ None, "search");

        apply_mcp_tool_approval_decision(
            &host,
            &McpToolApprovalDecision::AcceptAndRemember,
            Some(session_key.clone()),
            /*persistent_approval_key*/ None,
        )
        .await;

        assert_eq!(host.remembered(), vec![session_key]);
        assert!(host.non_app_persisted().is_empty());
        assert_eq!(host.reload_count(), 0);
    }

    #[tokio::test]
    async fn persist_codex_app_without_connector_id_falls_back_to_session_memory() {
        let host = FakeApprovalPersistenceHost::default();
        let key = approval_key(
            CODEX_APPS_MCP_SERVER_NAME,
            /*connector_id*/ None,
            "calendar/list_events",
        );

        maybe_persist_mcp_tool_approval(&host, key.clone()).await;

        assert_eq!(host.remembered(), vec![key]);
        assert!(host.codex_app_persisted().is_empty());
        assert_eq!(host.reload_count(), 0);
    }

    #[tokio::test]
    async fn persist_error_falls_back_to_session_memory_without_reload() {
        let host = FakeApprovalPersistenceHost {
            fail_persist: true,
            ..FakeApprovalPersistenceHost::default()
        };
        let key = approval_key("docs", /*connector_id*/ None, "search");

        maybe_persist_mcp_tool_approval(&host, key.clone()).await;

        assert_eq!(host.remembered(), vec![key]);
        assert_eq!(host.reload_count(), 0);
    }

    #[tokio::test]
    async fn persist_codex_app_tool_approval_writes_tool_override() {
        let tmp = tempdir().expect("tempdir");
        let config = ConfigBuilder::default()
            .codex_home(tmp.path().to_path_buf())
            .build()
            .await
            .expect("load config");

        persist_codex_app_tool_approval(&config, "calendar", "calendar/list_events")
            .await
            .expect("persist approval");

        let contents =
            std::fs::read_to_string(tmp.path().join(CONFIG_TOML_FILE)).expect("read config");
        let parsed: ConfigToml = toml::from_str(&contents).expect("parse config");

        assert_eq!(
            parsed.apps,
            Some(AppsConfigToml {
                default: None,
                apps: HashMap::from([(
                    "calendar".to_string(),
                    AppConfig {
                        enabled: true,
                        destructive_enabled: None,
                        open_world_enabled: None,
                        default_tools_approval_mode: None,
                        default_tools_enabled: None,
                        tools: Some(AppToolsConfig {
                            tools: HashMap::from([(
                                "calendar/list_events".to_string(),
                                AppToolConfig {
                                    enabled: None,
                                    approval_mode: Some(AppToolApproval::Approve),
                                },
                            )]),
                        }),
                    },
                )]),
            })
        );
        assert!(contents.contains("[apps.calendar.tools.\"calendar/list_events\"]"));
    }

    #[tokio::test]
    async fn persist_custom_mcp_tool_approval_writes_tool_override() {
        let tmp = tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join(CONFIG_TOML_FILE),
            "[mcp_servers.docs]\ncommand = \"docs-server\"\n",
        )
        .expect("seed config");
        let config = ConfigBuilder::default()
            .codex_home(tmp.path().to_path_buf())
            .build()
            .await
            .expect("load config");

        persist_non_app_mcp_tool_approval(
            &config,
            &plugin_service_api::DisabledPluginRuntime,
            "docs",
            "search",
        )
        .await
        .expect("persist approval");

        let contents =
            std::fs::read_to_string(tmp.path().join(CONFIG_TOML_FILE)).expect("read config");
        let parsed: ConfigToml = toml::from_str(&contents).expect("parse config");
        let tool = parsed
            .mcp_servers
            .get("docs")
            .and_then(|server| server.tools.get("search"))
            .expect("docs/search tool config exists");

        assert_eq!(
            tool,
            &McpServerToolConfig {
                approval_mode: Some(AppToolApproval::Approve),
            }
        );
        assert!(contents.contains("[mcp_servers.docs.tools.search]"));
    }

    #[tokio::test]
    async fn custom_mcp_tool_approval_mode_uses_server_default_with_tool_override() {
        let tmp = tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join(CONFIG_TOML_FILE),
            r#"
[mcp_servers.docs]
command = "docs-server"
default_tools_approval_mode = "approve"

[mcp_servers.docs.tools.search]
approval_mode = "prompt"
"#,
        )
        .expect("seed config");
        let config = ConfigBuilder::default()
            .codex_home(tmp.path().to_path_buf())
            .build()
            .await
            .expect("load config");

        assert_eq!(
            custom_mcp_tool_approval_mode(
                &config,
                &plugin_service_api::DisabledPluginRuntime,
                "docs",
                "read",
            )
            .await,
            AppToolApproval::Approve
        );
        assert_eq!(
            custom_mcp_tool_approval_mode(
                &config,
                &plugin_service_api::DisabledPluginRuntime,
                "docs",
                "search",
            )
            .await,
            AppToolApproval::Prompt
        );
        assert_eq!(
            custom_mcp_tool_approval_mode(
                &config,
                &plugin_service_api::DisabledPluginRuntime,
                "unknown",
                "search",
            )
            .await,
            AppToolApproval::Auto
        );
    }

    #[tokio::test]
    async fn custom_mcp_tool_approval_mode_uses_plugin_mcp_policy() {
        let tmp = tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join(CONFIG_TOML_FILE),
            r#"
[features]
plugins = true

[plugins."sample@test"]
enabled = true
"#,
        )
        .expect("seed config");
        let config = ConfigBuilder::default()
            .codex_home(tmp.path().to_path_buf())
            .build()
            .await
            .expect("load config");
        let server_config = mcp_server_config_from_toml(
            r#"
[mcp_servers.sample]
url = "https://sample.example/mcp"
default_tools_approval_mode = "prompt"

[mcp_servers.sample.tools.search]
approval_mode = "approve"
"#,
            "sample",
        );
        let plugins_runtime =
            FakePluginRuntime::with_plugin_mcp_config("sample@test", "sample", server_config);

        assert_eq!(
            custom_mcp_tool_approval_mode(&config, &plugins_runtime, "sample", "read").await,
            AppToolApproval::Prompt
        );
        assert_eq!(
            custom_mcp_tool_approval_mode(&config, &plugins_runtime, "sample", "search").await,
            AppToolApproval::Approve
        );
    }

    #[tokio::test]
    async fn persist_non_app_mcp_tool_approval_writes_plugin_mcp_policy() {
        let tmp = tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join(CONFIG_TOML_FILE),
            r#"
[features]
plugins = true

[plugins."sample@test"]
enabled = true
"#,
        )
        .expect("seed config");
        let config = ConfigBuilder::default()
            .codex_home(tmp.path().to_path_buf())
            .build()
            .await
            .expect("load config");
        let server_config = mcp_server_config_from_toml(
            r#"
[mcp_servers.sample]
url = "https://sample.example/mcp"
"#,
            "sample",
        );
        let plugins_runtime =
            FakePluginRuntime::with_plugin_mcp_config("sample@test", "sample", server_config);

        persist_non_app_mcp_tool_approval(&config, &plugins_runtime, "sample", "search")
            .await
            .expect("persist approval");

        let contents =
            std::fs::read_to_string(tmp.path().join(CONFIG_TOML_FILE)).expect("read config");
        let parsed: ConfigToml = toml::from_str(&contents).expect("parse config");
        let tool = parsed
            .plugins
            .get("sample@test")
            .and_then(|plugin| plugin.mcp_servers.get("sample"))
            .and_then(|server| server.tools.get("search"))
            .expect("sample/search tool config exists");

        assert_eq!(
            tool,
            &McpServerToolConfig {
                approval_mode: Some(AppToolApproval::Approve),
            }
        );
        assert!(contents.contains(r#"[plugins."sample@test".mcp_servers.sample.tools.search]"#));
    }

    #[test]
    fn approval_requirement_skips_when_global_policy_auto_approves() {
        let permission_profile = PermissionProfile::Managed {
            file_system: ManagedFileSystemPermissions::Unrestricted,
            network: NetworkSandboxPolicy::Enabled,
        };
        let metadata = metadata_with_annotations(Some(ToolAnnotations {
            destructive_hint: Some(true),
            idempotent_hint: None,
            open_world_hint: Some(true),
            read_only_hint: Some(false),
            title: None,
        }));
        let context = McpToolApprovalRequirementContext {
            approval_policy: AskForApproval::Never,
            permission_profile: &permission_profile,
            approvals_reviewer: ApprovalsReviewer::User,
            approval_mode: AppToolApproval::Auto,
            metadata: Some(&metadata),
        };

        assert_eq!(
            mcp_tool_approval_requirement(context),
            McpToolApprovalRequirement::NotRequired
        );
    }

    #[test]
    fn approval_requirement_honors_read_only_annotations_and_prompt_mode() {
        let permission_profile = PermissionProfile::read_only();
        let metadata = metadata_with_annotations(Some(ToolAnnotations {
            destructive_hint: Some(false),
            idempotent_hint: None,
            open_world_hint: Some(false),
            read_only_hint: Some(true),
            title: None,
        }));

        assert_eq!(
            mcp_tool_approval_requirement(approval_requirement_context(
                AppToolApproval::Auto,
                &permission_profile,
                Some(&metadata),
            )),
            McpToolApprovalRequirement::NotRequired
        );
        assert_eq!(
            mcp_tool_approval_requirement(approval_requirement_context(
                AppToolApproval::Prompt,
                &permission_profile,
                Some(&metadata),
            )),
            McpToolApprovalRequirement::Required {
                auto_approved_by_policy: false,
            }
        );
    }

    #[test]
    fn approval_requirement_requires_review_for_risky_or_missing_annotations() {
        let permission_profile = PermissionProfile::read_only();
        let risky_annotations = [
            ToolAnnotations {
                destructive_hint: Some(true),
                idempotent_hint: None,
                open_world_hint: None,
                read_only_hint: Some(false),
                title: None,
            },
            ToolAnnotations {
                destructive_hint: None,
                idempotent_hint: None,
                open_world_hint: Some(true),
                read_only_hint: Some(false),
                title: None,
            },
            ToolAnnotations {
                destructive_hint: Some(true),
                idempotent_hint: None,
                open_world_hint: Some(true),
                read_only_hint: Some(true),
                title: None,
            },
        ];

        for annotations in risky_annotations {
            let metadata = metadata_with_annotations(Some(annotations));
            assert_eq!(
                mcp_tool_approval_requirement(approval_requirement_context(
                    AppToolApproval::Auto,
                    &permission_profile,
                    Some(&metadata),
                )),
                McpToolApprovalRequirement::Required {
                    auto_approved_by_policy: false,
                }
            );
        }

        assert_eq!(
            mcp_tool_approval_requirement(approval_requirement_context(
                AppToolApproval::Auto,
                &permission_profile,
                /*metadata*/ None,
            )),
            McpToolApprovalRequirement::Required {
                auto_approved_by_policy: false,
            }
        );
    }

    #[test]
    fn approval_requirement_auto_approves_explicit_approve_mode() {
        let permission_profile = PermissionProfile::read_only();
        let metadata = metadata_with_annotations(Some(ToolAnnotations {
            destructive_hint: Some(true),
            idempotent_hint: None,
            open_world_hint: Some(true),
            read_only_hint: Some(false),
            title: None,
        }));

        assert_eq!(
            mcp_tool_approval_requirement(approval_requirement_context(
                AppToolApproval::Approve,
                &permission_profile,
                Some(&metadata),
            )),
            McpToolApprovalRequirement::NotRequired
        );
    }

    #[test]
    fn mcp_tool_call_request_meta_includes_turn_metadata_for_custom_server() {
        let turn_metadata = serde_json::json!({
            "model": "gpt-test",
            "reasoning_effort": "high",
            "turn_started_at_unix_ms": 1_700_000_000_123_i64,
        });

        assert_eq!(
            build_mcp_tool_call_request_meta(
                Some(turn_metadata.clone()),
                "custom_server",
                "call-custom",
                /*metadata*/ None,
                "x-codex-turn-metadata",
            ),
            Some(serde_json::json!({
                "x-codex-turn-metadata": turn_metadata,
            }))
        );
    }

    #[test]
    fn codex_apps_tool_call_request_meta_includes_turn_metadata_and_codex_apps_meta() {
        let turn_metadata = serde_json::json!({
            "model": "gpt-test",
            "reasoning_effort": "high",
        });
        let metadata = McpToolApprovalMetadata {
            annotations: None,
            connector_id: Some("calendar".to_string()),
            connector_name: Some("Calendar".to_string()),
            connector_description: Some("Manage events".to_string()),
            tool_title: Some("Create Event".to_string()),
            tool_description: Some("Create a calendar event.".to_string()),
            mcp_app_resource_uri: None,
            codex_apps_meta: Some(
                serde_json::json!({
                    "resource_uri": "connector://calendar/tools/calendar_create_event",
                    "contains_mcp_source": true,
                    "connector_id": "calendar",
                })
                .as_object()
                .cloned()
                .expect("_codex_apps metadata should be an object"),
            ),
            openai_file_input_params: None,
        };

        assert_eq!(
            build_mcp_tool_call_request_meta(
                Some(turn_metadata.clone()),
                CODEX_APPS_MCP_SERVER_NAME,
                "call_abc123xyz789",
                Some(&metadata),
                "x-codex-turn-metadata",
            ),
            Some(serde_json::json!({
                "x-codex-turn-metadata": turn_metadata,
                MCP_TOOL_CODEX_APPS_META_KEY: {
                    "call_id": "call_abc123xyz789",
                    "resource_uri": "connector://calendar/tools/calendar_create_event",
                    "contains_mcp_source": true,
                    "connector_id": "calendar",
                },
            }))
        );
    }

    #[test]
    fn codex_apps_tool_call_request_meta_includes_call_id_without_existing_codex_apps_meta() {
        assert_eq!(
            build_mcp_tool_call_request_meta(
                Some(serde_json::json!({"model": "gpt-test"})),
                CODEX_APPS_MCP_SERVER_NAME,
                "call_abc123xyz789",
                /*metadata*/ None,
                "x-codex-turn-metadata",
            ),
            Some(serde_json::json!({
                "x-codex-turn-metadata": {"model": "gpt-test"},
                MCP_TOOL_CODEX_APPS_META_KEY: {
                    "call_id": "call_abc123xyz789",
                },
            }))
        );
    }

    #[test]
    fn mcp_tool_call_request_meta_is_absent_when_no_metadata_is_available() {
        assert_eq!(
            build_mcp_tool_call_request_meta(
                /*turn_metadata*/ None,
                "custom_server",
                "call-custom",
                /*metadata*/ None,
                "x-codex-turn-metadata",
            ),
            None
        );
    }
}
