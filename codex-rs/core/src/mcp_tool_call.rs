use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

use codex_mcp_types::McpServerElicitationRequest;
use codex_mcp_types::McpServerElicitationRequestParams;
use tracing::error;

use crate::arc_monitor::ArcMonitorOutcome;
use crate::arc_monitor::monitor_action;
use crate::client::X_CODEX_TURN_METADATA_HEADER;
use crate::config::Config;
use crate::config::edit::ConfigEdit;
use crate::config::edit::ConfigEditsBuilder;
use crate::connectors;
use crate::guardian::GuardianApprovalRequest;
use crate::guardian::GuardianMcpAnnotations;
use crate::guardian::guardian_approval_request_to_json;
use crate::guardian::guardian_rejection_message;
use crate::guardian::guardian_timeout_message;
use crate::guardian::new_guardian_review_id;
use crate::guardian::review_approval_request;
use crate::guardian::routes_approval_to_guardian;
use crate::hook_runtime::run_permission_request_hooks;
use crate::mcp_openai_file::rewrite_mcp_tool_arguments_for_openai_files;
use crate::session::session::Session;
use crate::session::turn_context::TurnContext;
use crate::state_db_bridge as state_db;
use crate::tools::hook_names::HookToolName;
use crate::tools::sandboxing::PermissionRequestPayload;
use crate::turn_metadata::McpTurnMetadataContext;
use codex_analytics_api::AppInvocation;
use codex_analytics_api::InvocationType;
use codex_analytics_api::build_track_events_context;
use codex_config_types::AppToolApproval;
use codex_config_types::ConfigLayerSource;
use codex_config_types::McpServerConfig;
use codex_features::Feature;
use codex_hooks_api::PermissionRequestDecision;
use codex_mcp_runtime_api::McpToolRuntime;
#[cfg(test)]
use codex_mcp_tool_types::ToolAnnotations;
use codex_mcp_tool_types::sanitize_mcp_tool_result_for_model;
use codex_mcp_tool_types::truncate_mcp_tool_result_for_event;
use codex_mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use codex_mcp_types::ElicitationAction;
#[cfg(test)]
use codex_mcp_types::ElicitationResponse;
use codex_mcp_types::MCP_RESULT_TELEMETRY_SERVER_USER_FLOW_SPAN_ATTR;
use codex_mcp_types::MCP_RESULT_TELEMETRY_TARGET_ID_SPAN_ATTR;
use codex_mcp_types::MCP_SANDBOX_STATE_META_CAPABILITY;
pub(crate) use codex_mcp_types::MCP_TOOL_APPROVAL_ACCEPT;
#[cfg(test)]
use codex_mcp_types::MCP_TOOL_APPROVAL_ACCEPT_AND_REMEMBER;
pub(crate) use codex_mcp_types::MCP_TOOL_APPROVAL_ACCEPT_FOR_SESSION;
#[cfg(test)]
use codex_mcp_types::MCP_TOOL_APPROVAL_CANCEL;
pub(crate) use codex_mcp_types::MCP_TOOL_APPROVAL_DECLINE_SYNTHETIC;
pub(crate) use codex_mcp_types::MCP_TOOL_APPROVAL_QUESTION_ID_PREFIX;
use codex_mcp_types::MCP_TOOL_CODEX_APPS_META_KEY;
#[cfg(test)]
use codex_mcp_types::McpElicitationObjectType;
#[cfg(test)]
use codex_mcp_types::McpElicitationSchema;
use codex_mcp_types::McpPermissionPromptAutoApproveContext;
use codex_mcp_types::McpToolApprovalDecision;
use codex_mcp_types::McpToolApprovalElicitationRequest;
use codex_mcp_types::McpToolApprovalKey;
use codex_mcp_types::McpToolApprovalMetadata;
#[cfg(test)]
use codex_mcp_types::McpToolApprovalPromptOptions;
#[cfg(test)]
use codex_mcp_types::RenderedMcpToolApprovalParam;
use codex_mcp_types::SandboxState;
use codex_mcp_types::auth_elicitation_completed_result;
use codex_mcp_types::build_auth_elicitation_plan;
use codex_mcp_types::build_mcp_tool_approval_display_params;
#[cfg(test)]
use codex_mcp_types::build_mcp_tool_approval_elicitation_meta;
use codex_mcp_types::build_mcp_tool_approval_elicitation_request;
use codex_mcp_types::build_mcp_tool_approval_question;
pub(crate) use codex_mcp_types::is_mcp_tool_approval_question_id;
use codex_mcp_types::mcp_app_resource_uri_from_tool_meta;
use codex_mcp_types::mcp_permission_prompt_is_auto_approved;
use codex_mcp_types::mcp_tool_approval_prompt_options;
use codex_mcp_types::mcp_tool_approval_question_text;
use codex_mcp_types::mcp_tool_call_result_span_telemetry;
use codex_mcp_types::mcp_tool_call_server_fields;
use codex_mcp_types::normalize_approval_decision_for_mode;
use codex_mcp_types::openai_file_input_params_for_server;
use codex_mcp_types::parse_mcp_tool_approval_elicitation_response;
use codex_mcp_types::parse_mcp_tool_approval_response;
use codex_mcp_types::persistent_mcp_tool_approval_key;
use codex_mcp_types::render_mcp_tool_approval_template;
use codex_mcp_types::requires_mcp_tool_approval;
use codex_mcp_types::session_mcp_tool_approval_key;
use codex_mcp_types::with_mcp_tool_call_thread_id_meta;
use codex_protocol::items::McpToolCallError;
use codex_protocol::items::McpToolCallItem;
use codex_protocol::items::McpToolCallStatus;
use codex_protocol::items::TurnItem;
use codex_protocol::mcp::CallToolResult;
use codex_protocol::mcp::RequestId;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::APPROVAL_KIND_KEY as MCP_TOOL_APPROVAL_KIND_KEY;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::APPROVAL_KIND_MCP_TOOL_CALL as MCP_TOOL_APPROVAL_KIND_MCP_TOOL_CALL;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::CONNECTOR_DESCRIPTION_KEY as MCP_TOOL_APPROVAL_CONNECTOR_DESCRIPTION_KEY;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::CONNECTOR_ID_KEY as MCP_TOOL_APPROVAL_CONNECTOR_ID_KEY;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::CONNECTOR_NAME_KEY as MCP_TOOL_APPROVAL_CONNECTOR_NAME_KEY;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::PERSIST_ALWAYS as MCP_TOOL_APPROVAL_PERSIST_ALWAYS;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::PERSIST_KEY as MCP_TOOL_APPROVAL_PERSIST_KEY;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::PERSIST_SESSION as MCP_TOOL_APPROVAL_PERSIST_SESSION;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::SOURCE_CONNECTOR as MCP_TOOL_APPROVAL_SOURCE_CONNECTOR;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::SOURCE_KEY as MCP_TOOL_APPROVAL_SOURCE_KEY;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::TOOL_DESCRIPTION_KEY as MCP_TOOL_APPROVAL_TOOL_DESCRIPTION_KEY;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::TOOL_PARAMS_DISPLAY_KEY as MCP_TOOL_APPROVAL_TOOL_PARAMS_DISPLAY_KEY;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::TOOL_PARAMS_KEY as MCP_TOOL_APPROVAL_TOOL_PARAMS_KEY;
#[cfg(test)]
use codex_protocol::mcp_approval_meta::TOOL_TITLE_KEY as MCP_TOOL_APPROVAL_TOOL_TITLE_KEY;
use codex_protocol::openai_models::InputModality;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::McpInvocation;
use codex_protocol::protocol::ReviewDecision;
#[cfg(test)]
use codex_protocol::request_user_input::RequestUserInputAnswer;
use codex_protocol::request_user_input::RequestUserInputArgs;
#[cfg(test)]
use codex_protocol::request_user_input::RequestUserInputResponse;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_pty::DEFAULT_OUTPUT_BYTES_CAP;
use codex_utils_string::sanitize_metric_tag_value;
use serde::Deserialize;
use serde_json::Value as JsonValue;
use std::sync::Arc;
use tracing::Instrument;
use tracing::Span;
use tracing::field::Empty;

const MCP_CALL_COUNT_METRIC: &str = "codex.mcp.call";
const MCP_CALL_DURATION_METRIC: &str = "codex.mcp.call.duration_ms";
const MCP_TOOL_CALL_EVENT_RESULT_MAX_BYTES: usize = DEFAULT_OUTPUT_BYTES_CAP;

/// Handles the specified tool call and dispatches the appropriate MCP tool-call
/// item lifecycle events to the `Session`.
pub(crate) async fn handle_mcp_tool_call(
    sess: Arc<Session>,
    turn_context: &Arc<TurnContext>,
    call_id: String,
    server: String,
    tool_name: String,
    hook_tool_name: String,
    arguments: String,
) -> HandledMcpToolCall {
    // Parse the `arguments` as JSON. An empty string is OK, but invalid JSON
    // is not.
    let arguments_value = if arguments.trim().is_empty() {
        None
    } else {
        match serde_json::from_str::<serde_json::Value>(&arguments) {
            Ok(value) => Some(value),
            Err(e) => {
                error!("failed to parse tool call arguments: {e}");
                return HandledMcpToolCall {
                    result: CallToolResult::from_error_text(format!("err: {e}")),
                    tool_input: JsonValue::Object(serde_json::Map::new()),
                };
            }
        }
    };

    let invocation = McpInvocation {
        server: server.clone(),
        tool: tool_name.clone(),
        arguments: arguments_value.clone(),
    };

    let metadata =
        lookup_mcp_tool_metadata(sess.as_ref(), turn_context.as_ref(), &server, &tool_name).await;
    let mcp_app_resource_uri = metadata
        .as_ref()
        .and_then(|metadata| metadata.mcp_app_resource_uri.clone());
    let app_tool_policy = if server == CODEX_APPS_MCP_SERVER_NAME {
        connectors::app_tool_policy(
            &turn_context.config,
            metadata
                .as_ref()
                .and_then(|metadata| metadata.connector_id.as_deref()),
            &tool_name,
            metadata
                .as_ref()
                .and_then(|metadata| metadata.tool_title.as_deref()),
            metadata
                .as_ref()
                .and_then(|metadata| metadata.annotations.as_ref()),
        )
    } else {
        connectors::AppToolPolicy::default()
    };
    let approval_mode = if server == CODEX_APPS_MCP_SERVER_NAME {
        app_tool_policy.approval
    } else {
        custom_mcp_tool_approval_mode(sess.as_ref(), turn_context.as_ref(), &server, &tool_name)
            .await
    };

    if server == CODEX_APPS_MCP_SERVER_NAME && !app_tool_policy.enabled {
        let result = notify_mcp_tool_call_skip(
            sess.as_ref(),
            turn_context.as_ref(),
            &call_id,
            invocation,
            mcp_app_resource_uri.clone(),
            "MCP tool call blocked by app configuration".to_string(),
            /*already_started*/ false,
        )
        .await;
        let status = if result.is_ok() { "ok" } else { "error" };
        turn_context.session_telemetry.counter(
            MCP_CALL_COUNT_METRIC,
            /*inc*/ 1,
            &[("status", status)],
        );
        return HandledMcpToolCall {
            result: CallToolResult::from_result(result),
            tool_input: arguments_value
                .unwrap_or_else(|| JsonValue::Object(serde_json::Map::new())),
        };
    }
    let connector_id = metadata
        .as_ref()
        .and_then(|metadata| metadata.connector_id.clone());
    let connector_name = metadata
        .as_ref()
        .and_then(|metadata| metadata.connector_name.clone());

    notify_mcp_tool_call_started(
        sess.as_ref(),
        turn_context.as_ref(),
        &call_id,
        invocation.clone(),
        mcp_app_resource_uri.clone(),
    )
    .await;

    if let Some(decision) = maybe_request_mcp_tool_approval(
        &sess,
        turn_context,
        &call_id,
        &invocation,
        &hook_tool_name,
        metadata.as_ref(),
        approval_mode,
    )
    .await
    {
        let result = match decision {
            McpToolApprovalDecision::Accept
            | McpToolApprovalDecision::AcceptForSession
            | McpToolApprovalDecision::AcceptAndRemember => {
                return handle_approved_mcp_tool_call(
                    sess.as_ref(),
                    turn_context.as_ref(),
                    &call_id,
                    invocation,
                    metadata.as_ref(),
                    mcp_app_resource_uri,
                )
                .await;
            }
            McpToolApprovalDecision::Decline { message } => {
                let message = message.unwrap_or_else(|| "user rejected MCP tool call".to_string());
                notify_mcp_tool_call_skip(
                    sess.as_ref(),
                    turn_context.as_ref(),
                    &call_id,
                    invocation,
                    mcp_app_resource_uri.clone(),
                    message,
                    /*already_started*/ true,
                )
                .await
            }
            McpToolApprovalDecision::Cancel => {
                let message = "user cancelled MCP tool call".to_string();
                notify_mcp_tool_call_skip(
                    sess.as_ref(),
                    turn_context.as_ref(),
                    &call_id,
                    invocation,
                    mcp_app_resource_uri.clone(),
                    message,
                    /*already_started*/ true,
                )
                .await
            }
            McpToolApprovalDecision::BlockedBySafetyMonitor(message) => {
                notify_mcp_tool_call_skip(
                    sess.as_ref(),
                    turn_context.as_ref(),
                    &call_id,
                    invocation,
                    mcp_app_resource_uri.clone(),
                    message,
                    /*already_started*/ true,
                )
                .await
            }
        };

        let status = if result.is_ok() { "ok" } else { "error" };
        emit_mcp_call_metrics(
            turn_context.as_ref(),
            status,
            &tool_name,
            connector_id.as_deref(),
            connector_name.as_deref(),
            /*duration*/ None,
        );

        return HandledMcpToolCall {
            result: CallToolResult::from_result(result),
            tool_input: arguments_value
                .unwrap_or_else(|| JsonValue::Object(serde_json::Map::new())),
        };
    }

    handle_approved_mcp_tool_call(
        sess.as_ref(),
        turn_context.as_ref(),
        &call_id,
        invocation,
        metadata.as_ref(),
        mcp_app_resource_uri,
    )
    .await
}

pub(crate) struct HandledMcpToolCall {
    pub(crate) result: CallToolResult,
    pub(crate) tool_input: JsonValue,
}

async fn handle_approved_mcp_tool_call(
    sess: &Session,
    turn_context: &TurnContext,
    call_id: &str,
    invocation: McpInvocation,
    metadata: Option<&McpToolApprovalMetadata>,
    mcp_app_resource_uri: Option<String>,
) -> HandledMcpToolCall {
    let server = invocation.server.clone();
    maybe_mark_thread_memory_mode_polluted(sess, turn_context, &server).await;
    let tool_name = invocation.tool.clone();
    let arguments_value = invocation.arguments.clone();
    let connector_id = metadata.and_then(|metadata| metadata.connector_id.as_deref());
    let connector_name = metadata.and_then(|metadata| metadata.connector_name.as_deref());
    let server_origin = {
        let manager = sess.services.mcp_connection_manager.read().await;
        McpToolRuntime::server_origin(manager.as_ref(), &server)
    };

    let start = Instant::now();
    let rewrite = rewrite_mcp_tool_arguments_for_openai_files(
        sess,
        turn_context,
        arguments_value.clone(),
        metadata.and_then(|metadata| metadata.openai_file_input_params.as_deref()),
    )
    .await;
    let tool_input = match &rewrite {
        Ok(Some(rewritten_arguments)) => rewritten_arguments.clone(),
        Ok(None) | Err(_) => arguments_value
            .clone()
            .unwrap_or_else(|| JsonValue::Object(serde_json::Map::new())),
    };
    let result = async {
        let rewritten_arguments = rewrite?;
        let request_meta =
            build_mcp_tool_call_request_meta(turn_context, &server, call_id, metadata);
        let result = execute_mcp_tool_call(
            sess,
            turn_context,
            call_id,
            &invocation,
            rewritten_arguments,
            metadata,
            request_meta,
        )
        .await;
        record_mcp_result_span_telemetry(&Span::current(), result.as_ref().ok());
        result
    }
    .instrument(mcp_tool_call_span(
        sess,
        turn_context,
        McpToolCallSpanFields {
            server_name: &server,
            tool_name: &tool_name,
            call_id,
            server_origin: server_origin.as_deref(),
            connector_id,
            connector_name,
        },
    ))
    .await;
    if let Err(error) = &result {
        tracing::warn!("MCP tool call error: {error:?}");
    }
    let duration = start.elapsed();
    notify_mcp_tool_call_completed(
        sess,
        turn_context,
        call_id,
        invocation,
        mcp_app_resource_uri,
        duration,
        truncate_mcp_tool_result_for_event(&result, MCP_TOOL_CALL_EVENT_RESULT_MAX_BYTES),
    )
    .await;
    maybe_track_codex_app_used(sess, turn_context, &server, &tool_name).await;

    let status = if result.is_ok() { "ok" } else { "error" };
    emit_mcp_call_metrics(
        turn_context,
        status,
        &tool_name,
        connector_id,
        connector_name,
        Some(duration),
    );

    HandledMcpToolCall {
        result: CallToolResult::from_result(result),
        tool_input,
    }
}

fn emit_mcp_call_metrics(
    turn_context: &TurnContext,
    status: &str,
    tool_name: &str,
    connector_id: Option<&str>,
    connector_name: Option<&str>,
    duration: Option<Duration>,
) {
    let tags = mcp_call_metric_tags(status, tool_name, connector_id, connector_name);
    let tag_refs: Vec<(&str, &str)> = tags
        .iter()
        .map(|(key, value)| (*key, value.as_str()))
        .collect();
    turn_context
        .session_telemetry
        .counter(MCP_CALL_COUNT_METRIC, /*inc*/ 1, &tag_refs);
    if let Some(duration) = duration {
        turn_context.session_telemetry.record_duration(
            MCP_CALL_DURATION_METRIC,
            duration,
            &tag_refs,
        );
    }
}

fn mcp_call_metric_tags(
    status: &str,
    tool_name: &str,
    connector_id: Option<&str>,
    connector_name: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut tags = vec![
        ("status", sanitize_metric_tag_value(status)),
        ("tool", sanitize_metric_tag_value(tool_name)),
    ];
    if let Some(connector_id) = connector_id.filter(|connector_id| !connector_id.is_empty()) {
        tags.push(("connector_id", sanitize_metric_tag_value(connector_id)));
    }
    if let Some(connector_name) = connector_name.filter(|connector_name| !connector_name.is_empty())
    {
        tags.push(("connector_name", sanitize_metric_tag_value(connector_name)));
    }
    tags
}

fn mcp_tool_call_span(
    session: &Session,
    turn_context: &TurnContext,
    fields: McpToolCallSpanFields<'_>,
) -> Span {
    let transport = match fields.server_origin {
        Some("stdio") => "stdio",
        Some("in_process") => "in_process",
        Some(_) => "streamable_http",
        None => "",
    };
    let span = tracing::info_span!(
        "mcp.tools.call",
        otel.kind = "client",
        rpc.system = "jsonrpc",
        rpc.method = "tools/call",
        mcp.server.name = fields.server_name,
        mcp.server.origin = fields.server_origin.unwrap_or(""),
        mcp.transport = transport,
        mcp.connector.id = fields.connector_id.unwrap_or(""),
        mcp.connector.name = fields.connector_name.unwrap_or(""),
        tool.name = fields.tool_name,
        tool.call_id = fields.call_id,
        conversation.id = %session.conversation_id,
        session.id = %session.conversation_id,
        turn.id = turn_context.sub_id.as_str(),
        server.address = Empty,
        server.port = Empty,
        codex.mcp.target.id = Empty,
        codex.mcp.server_user_flow.triggered = Empty,
    );
    record_server_fields(&span, fields.server_origin);
    span
}

struct McpToolCallSpanFields<'a> {
    server_name: &'a str,
    tool_name: &'a str,
    call_id: &'a str,
    server_origin: Option<&'a str>,
    connector_id: Option<&'a str>,
    connector_name: Option<&'a str>,
}

fn record_server_fields(span: &Span, url: Option<&str>) {
    let Some(url) = url else {
        return;
    };
    let Some(fields) = mcp_tool_call_server_fields(url) else {
        return;
    };
    span.record("server.address", fields.host.as_str());
    if let Some(port) = fields.port {
        span.record("server.port", port as i64);
    }
}

fn record_mcp_result_span_telemetry(span: &Span, result: Option<&CallToolResult>) {
    let Some(telemetry) = result.and_then(mcp_tool_call_result_span_telemetry) else {
        return;
    };

    if let Some(target_id) = telemetry.target_id {
        span.record(MCP_RESULT_TELEMETRY_TARGET_ID_SPAN_ATTR, target_id.as_str());
    }

    if let Some(did_trigger_server_user_flow) = telemetry.did_trigger_server_user_flow {
        span.record(
            MCP_RESULT_TELEMETRY_SERVER_USER_FLOW_SPAN_ATTR,
            did_trigger_server_user_flow,
        );
    }
}

async fn execute_mcp_tool_call(
    sess: &Session,
    turn_context: &TurnContext,
    call_id: &str,
    invocation: &McpInvocation,
    rewritten_arguments: Option<JsonValue>,
    metadata: Option<&McpToolApprovalMetadata>,
    request_meta: Option<JsonValue>,
) -> Result<CallToolResult, String> {
    let request_meta =
        with_mcp_tool_call_thread_id_meta(request_meta, &sess.conversation_id.to_string());
    let request_meta = augment_mcp_tool_request_meta_with_sandbox_state(
        sess,
        turn_context,
        &invocation.server,
        request_meta,
    )
    .await
    .map_err(|e| format!("failed to build MCP tool request metadata: {e:#}"))?;
    let mcp_call_trace = sess
        .services
        .rollout_thread_trace
        .start_mcp_call_trace(call_id);
    let request_meta = mcp_call_trace.add_request_meta(request_meta);
    let result = sess
        .call_tool(
            &invocation.server,
            &invocation.tool,
            rewritten_arguments,
            request_meta,
        )
        .await
        .map_err(|e| format!("tool call error: {e:?}"))?;
    let result = sanitize_mcp_tool_result_for_model(
        turn_context
            .model_info
            .input_modalities
            .contains(&InputModality::Image),
        Ok(result),
    )?;
    Ok(maybe_request_codex_apps_auth_elicitation(
        sess,
        turn_context,
        call_id,
        &invocation.server,
        metadata,
        result,
    )
    .await)
}

async fn maybe_request_codex_apps_auth_elicitation(
    sess: &Session,
    turn_context: &TurnContext,
    call_id: &str,
    server: &str,
    metadata: Option<&McpToolApprovalMetadata>,
    result: CallToolResult,
) -> CallToolResult {
    let is_host_owned_codex_apps_server = {
        let manager = sess.services.mcp_connection_manager.read().await;
        McpToolRuntime::is_host_owned_codex_apps_server(manager.as_ref(), server)
    };
    if !is_host_owned_codex_apps_server {
        return result;
    }

    if !turn_context.features.enabled(Feature::AuthElicitation) {
        return result;
    }

    match turn_context.approval_policy.value() {
        AskForApproval::Never => return result,
        AskForApproval::Granular(granular_config) if !granular_config.allows_mcp_elicitations() => {
            return result;
        }
        AskForApproval::OnFailure
        | AskForApproval::OnRequest
        | AskForApproval::UnlessTrusted
        | AskForApproval::Granular(_) => {}
    }

    let connector_id = metadata.and_then(|metadata| metadata.connector_id.as_deref());
    let connector_name = metadata.and_then(|metadata| metadata.connector_name.as_deref());
    let install_url = connector_id.map(|connector_id| {
        codex_connectors_api::metadata::connector_install_url(
            connector_name.unwrap_or(connector_id),
            connector_id,
        )
    });
    let Some(plan) =
        build_auth_elicitation_plan(call_id, &result, connector_id, connector_name, install_url)
    else {
        return result;
    };

    let request_id = RequestId::String(plan.elicitation.elicitation_id.clone());
    let params = McpServerElicitationRequestParams {
        thread_id: sess.conversation_id.to_string(),
        turn_id: Some(turn_context.sub_id.clone()),
        server_name: CODEX_APPS_MCP_SERVER_NAME.to_string(),
        request: McpServerElicitationRequest::Url {
            meta: Some(plan.elicitation.meta),
            message: plan.elicitation.message,
            url: plan.elicitation.url,
            elicitation_id: plan.elicitation.elicitation_id,
        },
    };
    let response = sess
        .request_mcp_server_elicitation(turn_context, request_id, params)
        .await;
    if !response
        .as_ref()
        .is_some_and(|response| response.action == ElicitationAction::Accept)
    {
        return result;
    }

    refresh_codex_apps_after_connector_auth(sess, turn_context).await;
    auth_elicitation_completed_result(&plan.auth_failure, result.meta)
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "Codex Apps cache refresh reads through the session-owned manager guard"
)]
async fn refresh_codex_apps_after_connector_auth(sess: &Session, turn_context: &TurnContext) {
    let mcp_tools_result = {
        let manager = sess.services.mcp_connection_manager.read().await;
        manager.hard_refresh_codex_apps_tools_cache().await
    };

    match mcp_tools_result {
        Ok(mcp_tools) => {
            let auth_snapshot = match turn_context.auth_runtime.as_ref() {
                Some(auth_runtime) => auth_runtime.auth().await,
                None => None,
            };
            let connector_auth_context =
                crate::mcp::codex_apps_auth_context(auth_snapshot.as_ref());
            connectors::refresh_accessible_connectors_cache_from_mcp_tools(
                &turn_context.config,
                connector_auth_context.as_ref(),
                &mcp_tools,
            );
        }
        Err(err) => {
            tracing::warn!("failed to refresh Codex Apps tools after connector auth: {err:#}");
        }
    }
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "MCP sandbox metadata reads through the session-owned manager guard"
)]
async fn augment_mcp_tool_request_meta_with_sandbox_state(
    sess: &Session,
    turn_context: &TurnContext,
    server: &str,
    mut meta: Option<serde_json::Value>,
) -> anyhow::Result<Option<serde_json::Value>> {
    let supports_sandbox_state_meta = {
        let manager = sess.services.mcp_connection_manager.read().await;
        McpToolRuntime::server_supports_sandbox_state_meta_capability(manager.as_ref(), server)
            .await
            .unwrap_or(false)
    };
    if !supports_sandbox_state_meta {
        return Ok(meta);
    }

    let sandbox_state = serde_json::to_value(SandboxState {
        permission_profile: Some(turn_context.permission_profile()),
        sandbox_policy: turn_context.sandbox_policy(),
        codex_linux_sandbox_exe: turn_context.codex_linux_sandbox_exe.clone(),
        #[allow(deprecated)]
        sandbox_cwd: turn_context.cwd.to_path_buf(),
        use_legacy_landlock: turn_context.features.use_legacy_landlock(),
    })?;

    match meta.as_mut() {
        Some(serde_json::Value::Object(map)) => {
            map.insert(MCP_SANDBOX_STATE_META_CAPABILITY.to_string(), sandbox_state);
        }
        Some(_) => {}
        None => {
            let mut map = serde_json::Map::new();
            map.insert(MCP_SANDBOX_STATE_META_CAPABILITY.to_string(), sandbox_state);
            meta = Some(serde_json::Value::Object(map));
        }
    }

    Ok(meta)
}

async fn maybe_mark_thread_memory_mode_polluted(
    sess: &Session,
    turn_context: &TurnContext,
    server: &str,
) {
    if !turn_context.config.memories.disable_on_external_context {
        return;
    }
    let pollutes_memory = {
        let manager = sess.services.mcp_connection_manager.read().await;
        McpToolRuntime::server_pollutes_memory(manager.as_ref(), server)
    };
    if !pollutes_memory {
        return;
    }
    state_db::mark_thread_memory_mode_polluted(
        sess.services.state_db.as_deref(),
        sess.conversation_id,
        "mcp_tool_call",
    )
    .await;
}

async fn notify_mcp_tool_call_started(
    sess: &Session,
    turn_context: &TurnContext,
    call_id: &str,
    invocation: McpInvocation,
    mcp_app_resource_uri: Option<String>,
) {
    let McpInvocation {
        server,
        tool,
        arguments,
    } = invocation;
    let item = TurnItem::McpToolCall(McpToolCallItem {
        id: call_id.to_string(),
        server,
        tool,
        arguments: arguments.unwrap_or(JsonValue::Null),
        mcp_app_resource_uri,
        status: McpToolCallStatus::InProgress,
        result: None,
        error: None,
        duration: None,
    });
    sess.emit_turn_item_started(turn_context, &item).await;
}

async fn notify_mcp_tool_call_completed(
    sess: &Session,
    turn_context: &TurnContext,
    call_id: &str,
    invocation: McpInvocation,
    mcp_app_resource_uri: Option<String>,
    duration: Duration,
    result: Result<CallToolResult, String>,
) {
    let (status, result, error) = match result {
        Ok(result) if result.is_error.unwrap_or(false) => {
            (McpToolCallStatus::Failed, Some(result), None)
        }
        Ok(result) => (McpToolCallStatus::Completed, Some(result), None),
        Err(message) => (
            McpToolCallStatus::Failed,
            None,
            Some(McpToolCallError { message }),
        ),
    };
    let McpInvocation {
        server,
        tool,
        arguments,
    } = invocation;
    let item = TurnItem::McpToolCall(McpToolCallItem {
        id: call_id.to_string(),
        server,
        tool,
        arguments: arguments.unwrap_or(JsonValue::Null),
        mcp_app_resource_uri,
        status,
        result,
        error,
        duration: Some(duration),
    });
    sess.emit_turn_item_completed(turn_context, item).await;
}

struct McpAppUsageMetadata {
    connector_id: Option<String>,
    app_name: Option<String>,
}

async fn maybe_track_codex_app_used(
    sess: &Session,
    turn_context: &TurnContext,
    server: &str,
    tool_name: &str,
) {
    if server != CODEX_APPS_MCP_SERVER_NAME {
        return;
    }
    let metadata = lookup_mcp_app_usage_metadata(sess, server, tool_name).await;
    let (connector_id, app_name) = metadata
        .map(|metadata| (metadata.connector_id, metadata.app_name))
        .unwrap_or((None, None));
    let invocation_type = if let Some(connector_id) = connector_id.as_deref() {
        let mentioned_connector_ids = sess.get_connector_selection().await;
        if mentioned_connector_ids.contains(connector_id) {
            InvocationType::Explicit
        } else {
            InvocationType::Implicit
        }
    } else {
        InvocationType::Implicit
    };

    let tracking = build_track_events_context(
        turn_context.model_info.slug.clone(),
        sess.conversation_id.to_string(),
        turn_context.sub_id.clone(),
    );
    sess.services.analytics_events_client.track_app_used(
        tracking,
        AppInvocation {
            connector_id,
            app_name,
            invocation_type: Some(invocation_type),
        },
    );
}

async fn custom_mcp_tool_approval_mode(
    sess: &Session,
    turn_context: &TurnContext,
    server: &str,
    tool_name: &str,
) -> AppToolApproval {
    let user_configured_mode = turn_context
        .config
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

    sess.services
        .plugins_manager
        .plugins_for_config(&turn_context.config.plugins_config_input())
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

fn build_mcp_tool_call_request_meta(
    turn_context: &TurnContext,
    server: &str,
    call_id: &str,
    metadata: Option<&McpToolApprovalMetadata>,
) -> Option<serde_json::Value> {
    let mut request_meta = serde_json::Map::new();

    if let Some(turn_metadata) = turn_context
        .turn_metadata_state
        .current_meta_value_for_mcp_request(McpTurnMetadataContext {
            model: turn_context.model_info.slug.as_str(),
            reasoning_effort: turn_context.effective_reasoning_effort(),
        })
    {
        request_meta.insert(X_CODEX_TURN_METADATA_HEADER.to_string(), turn_metadata);
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

const MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_DEFAULT: &str = "mcp_tool_call__default";
const MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_ALWAYS_ALLOW: &str = "mcp_tool_call__always_allow";

async fn maybe_request_mcp_tool_approval(
    sess: &Arc<Session>,
    turn_context: &Arc<TurnContext>,
    call_id: &str,
    invocation: &McpInvocation,
    hook_tool_name: &str,
    metadata: Option<&McpToolApprovalMetadata>,
    approval_mode: AppToolApproval,
) -> Option<McpToolApprovalDecision> {
    if mcp_permission_prompt_is_auto_approved(
        turn_context.approval_policy.value(),
        &turn_context.permission_profile(),
        McpPermissionPromptAutoApproveContext {
            approvals_reviewer: Some(turn_context.config.approvals_reviewer),
            tool_approval_mode: Some(approval_mode),
        },
    ) {
        return None;
    }

    let annotations = metadata.and_then(|metadata| metadata.annotations.as_ref());
    let approval_required = requires_mcp_tool_approval(annotations);
    if !approval_required && approval_mode != AppToolApproval::Prompt {
        return None;
    }

    let mut monitor_reason = None;
    let auto_approved_by_policy = approval_mode == AppToolApproval::Approve;

    if auto_approved_by_policy {
        match maybe_monitor_auto_approved_mcp_tool_call(
            sess,
            turn_context,
            invocation,
            metadata,
            approval_mode,
        )
        .await
        {
            ArcMonitorOutcome::Ok => return None,
            ArcMonitorOutcome::AskUser(reason) => {
                monitor_reason = Some(reason);
            }
            ArcMonitorOutcome::SteerModel(reason) => {
                return Some(McpToolApprovalDecision::BlockedBySafetyMonitor(
                    arc_monitor_interrupt_message(&reason),
                ));
            }
        }
    }

    let session_approval_key = session_mcp_tool_approval_key(invocation, metadata, approval_mode);
    let persistent_approval_key =
        persistent_mcp_tool_approval_key(invocation, metadata, approval_mode);
    if let Some(key) = session_approval_key.as_ref()
        && mcp_tool_approval_is_remembered(sess, key).await
    {
        return Some(McpToolApprovalDecision::Accept);
    }

    match run_permission_request_hooks(
        sess,
        turn_context,
        call_id,
        PermissionRequestPayload {
            tool_name: HookToolName::new(hook_tool_name),
            tool_input: invocation
                .arguments
                .clone()
                .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new())),
        },
    )
    .await
    {
        Some(PermissionRequestDecision::Allow) => {
            return Some(McpToolApprovalDecision::Accept);
        }
        Some(PermissionRequestDecision::Deny { message }) => {
            return Some(McpToolApprovalDecision::Decline {
                message: Some(message),
            });
        }
        None => {}
    }

    let tool_call_mcp_elicitation_enabled = turn_context
        .config
        .features
        .enabled(Feature::ToolCallMcpElicitation);

    if routes_approval_to_guardian(turn_context) {
        let review_id = new_guardian_review_id();
        let decision = review_approval_request(
            sess,
            turn_context,
            review_id.clone(),
            build_guardian_mcp_tool_review_request(call_id, invocation, metadata),
            monitor_reason.clone(),
        )
        .await;
        let decision = mcp_tool_approval_decision_from_guardian(sess, &review_id, decision).await;
        apply_mcp_tool_approval_decision(
            sess,
            turn_context,
            &decision,
            session_approval_key,
            persistent_approval_key,
        )
        .await;
        return Some(decision);
    }

    let prompt_options = mcp_tool_approval_prompt_options(
        session_approval_key.as_ref(),
        persistent_approval_key.as_ref(),
        tool_call_mcp_elicitation_enabled,
    );
    let question_id = format!("{MCP_TOOL_APPROVAL_QUESTION_ID_PREFIX}_{call_id}");
    let rendered_template = render_mcp_tool_approval_template(
        &invocation.server,
        metadata.and_then(|metadata| metadata.connector_id.as_deref()),
        metadata.and_then(|metadata| metadata.connector_name.as_deref()),
        metadata.and_then(|metadata| metadata.tool_title.as_deref()),
        invocation.arguments.as_ref(),
    );
    let tool_params_display = rendered_template
        .as_ref()
        .map(|rendered_template| rendered_template.tool_params_display.clone())
        .or_else(|| build_mcp_tool_approval_display_params(invocation.arguments.as_ref()));
    let mut question = build_mcp_tool_approval_question(
        question_id.clone(),
        &invocation.server,
        &invocation.tool,
        metadata.and_then(|metadata| metadata.connector_name.as_deref()),
        prompt_options,
        rendered_template
            .as_ref()
            .map(|rendered_template| rendered_template.question.as_str()),
    );
    question.question =
        mcp_tool_approval_question_text(question.question, monitor_reason.as_deref());
    if tool_call_mcp_elicitation_enabled {
        let thread_id = sess.conversation_id.to_string();
        let request_id =
            RequestId::String(format!("{MCP_TOOL_APPROVAL_QUESTION_ID_PREFIX}_{call_id}"));
        let params =
            build_mcp_tool_approval_elicitation_request(McpToolApprovalElicitationRequest {
                thread_id: &thread_id,
                turn_id: Some(&turn_context.sub_id),
                server: &invocation.server,
                metadata,
                tool_params: rendered_template
                    .as_ref()
                    .and_then(|rendered_template| rendered_template.tool_params.as_ref())
                    .or(invocation.arguments.as_ref()),
                tool_params_display: tool_params_display.as_deref(),
                question,
                message_override: rendered_template.as_ref().and_then(|rendered_template| {
                    monitor_reason
                        .is_none()
                        .then_some(rendered_template.elicitation_message.as_str())
                }),
                prompt_options,
            });
        let decision = parse_mcp_tool_approval_elicitation_response(
            sess.request_mcp_server_elicitation(turn_context.as_ref(), request_id, params)
                .await,
            &question_id,
        );
        let decision = normalize_approval_decision_for_mode(decision, approval_mode);
        apply_mcp_tool_approval_decision(
            sess,
            turn_context,
            &decision,
            session_approval_key,
            persistent_approval_key,
        )
        .await;
        return Some(decision);
    }

    let args = RequestUserInputArgs {
        questions: vec![question],
    };
    let response = sess
        .request_user_input(turn_context.as_ref(), call_id.to_string(), args)
        .await;
    let decision = normalize_approval_decision_for_mode(
        parse_mcp_tool_approval_response(response, &question_id),
        approval_mode,
    );
    apply_mcp_tool_approval_decision(
        sess,
        turn_context,
        &decision,
        session_approval_key,
        persistent_approval_key,
    )
    .await;
    Some(decision)
}

async fn maybe_monitor_auto_approved_mcp_tool_call(
    sess: &Session,
    turn_context: &TurnContext,
    invocation: &McpInvocation,
    metadata: Option<&McpToolApprovalMetadata>,
    approval_mode: AppToolApproval,
) -> ArcMonitorOutcome {
    let action = prepare_arc_request_action(invocation, metadata);
    monitor_action(
        sess,
        turn_context,
        action,
        mcp_tool_approval_callsite_mode(approval_mode, turn_context),
    )
    .await
}

fn prepare_arc_request_action(
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

pub(crate) fn build_guardian_mcp_tool_review_request(
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

async fn mcp_tool_approval_decision_from_guardian(
    sess: &Session,
    review_id: &str,
    decision: ReviewDecision,
) -> McpToolApprovalDecision {
    match decision {
        ReviewDecision::Approved
        | ReviewDecision::ApprovedExecpolicyAmendment { .. }
        | ReviewDecision::NetworkPolicyAmendment { .. } => McpToolApprovalDecision::Accept,
        ReviewDecision::ApprovedForSession => McpToolApprovalDecision::AcceptForSession,
        ReviewDecision::Denied => McpToolApprovalDecision::Decline {
            message: Some(guardian_rejection_message(sess, review_id).await),
        },
        ReviewDecision::TimedOut => McpToolApprovalDecision::Decline {
            message: Some(guardian_timeout_message()),
        },
        ReviewDecision::Abort => McpToolApprovalDecision::Decline { message: None },
    }
}

fn mcp_tool_approval_callsite_mode(
    approval_mode: AppToolApproval,
    _turn_context: &TurnContext,
) -> &'static str {
    match approval_mode {
        AppToolApproval::Approve => MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_ALWAYS_ALLOW,
        AppToolApproval::Auto | AppToolApproval::Prompt => {
            MCP_TOOL_CALL_ARC_MONITOR_CALLSITE_DEFAULT
        }
    }
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "MCP approval metadata reads through the session-owned manager guard"
)]
pub(crate) async fn lookup_mcp_tool_metadata(
    sess: &Session,
    turn_context: &TurnContext,
    server: &str,
    tool_name: &str,
) -> Option<McpToolApprovalMetadata> {
    let tools = sess
        .services
        .mcp_connection_manager
        .read()
        .await
        .list_all_tools()
        .await;
    let tool_info = tools
        .into_iter()
        .find(|tool_info| tool_info.server_name == server && tool_info.tool.name == tool_name)?;
    let auth_snapshot = if server == CODEX_APPS_MCP_SERVER_NAME {
        match turn_context.auth_runtime.as_ref() {
            Some(auth_runtime) => auth_runtime.auth().await,
            None => None,
        }
    } else {
        None
    };
    let connector_description = if server == CODEX_APPS_MCP_SERVER_NAME {
        let connectors = match connectors::list_cached_accessible_connectors_from_mcp_tools(
            turn_context.config.as_ref(),
            auth_snapshot.as_ref(),
        )
        .await
        {
            Some(connectors) => Some(connectors),
            None => connectors::list_accessible_connectors_from_mcp_tools(
                turn_context.config.as_ref(),
                auth_snapshot.as_ref(),
                sess.services.plugins_manager.as_ref(),
                sess.services.environment_manager.as_ref(),
                sess.services.mcp_auth_runtime.as_ref(),
                sess.services.mcp_connection_runtime_factory.as_ref(),
            )
            .await
            .ok(),
        };
        connectors.and_then(|connectors| {
            let connector_id = tool_info.connector_id.as_deref()?;
            connectors
                .into_iter()
                .find(|connector| connector.id == connector_id)
                .and_then(|connector| connector.description)
        })
    } else {
        None
    };

    Some(McpToolApprovalMetadata {
        annotations: tool_info.tool.annotations,
        connector_id: tool_info.connector_id,
        connector_name: tool_info.connector_name,
        connector_description,
        tool_title: tool_info.tool.title,
        tool_description: tool_info.tool.description,
        mcp_app_resource_uri: mcp_app_resource_uri_from_tool_meta(tool_info.tool.meta.as_ref()),
        codex_apps_meta: tool_info
            .tool
            .meta
            .as_ref()
            .and_then(|meta| meta.get(MCP_TOOL_CODEX_APPS_META_KEY))
            .and_then(serde_json::Value::as_object)
            .cloned(),
        // Disallow custom MCPs from uploading files via fileParams.
        openai_file_input_params: openai_file_input_params_for_server(
            server,
            tool_info.tool.meta.as_ref(),
        ),
    })
}

#[expect(
    clippy::await_holding_invalid_type,
    reason = "MCP app metadata reads through the session-owned manager guard"
)]
async fn lookup_mcp_app_usage_metadata(
    sess: &Session,
    server: &str,
    tool_name: &str,
) -> Option<McpAppUsageMetadata> {
    let tools = sess
        .services
        .mcp_connection_manager
        .read()
        .await
        .list_all_tools()
        .await;

    tools.into_iter().find_map(|tool_info| {
        if tool_info.server_name == server && tool_info.tool.name == tool_name {
            Some(McpAppUsageMetadata {
                connector_id: tool_info.connector_id,
                app_name: tool_info.connector_name,
            })
        } else {
            None
        }
    })
}

fn arc_monitor_interrupt_message(reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "Tool call was cancelled because of safety risks.".to_string()
    } else {
        format!("Tool call was cancelled because of safety risks: {reason}")
    }
}

async fn mcp_tool_approval_is_remembered(sess: &Session, key: &McpToolApprovalKey) -> bool {
    let store = sess.services.tool_approvals.lock().await;
    matches!(store.get(key), Some(ReviewDecision::ApprovedForSession))
}

async fn remember_mcp_tool_approval(sess: &Session, key: McpToolApprovalKey) {
    let mut store = sess.services.tool_approvals.lock().await;
    store.put(key, ReviewDecision::ApprovedForSession);
}

async fn apply_mcp_tool_approval_decision(
    sess: &Session,
    turn_context: &TurnContext,
    decision: &McpToolApprovalDecision,
    session_approval_key: Option<McpToolApprovalKey>,
    persistent_approval_key: Option<McpToolApprovalKey>,
) {
    match decision {
        McpToolApprovalDecision::AcceptForSession => {
            if let Some(key) = session_approval_key {
                remember_mcp_tool_approval(sess, key).await;
            }
        }
        McpToolApprovalDecision::AcceptAndRemember => {
            if let Some(key) = persistent_approval_key {
                maybe_persist_mcp_tool_approval(sess, turn_context, key).await;
            } else if let Some(key) = session_approval_key {
                remember_mcp_tool_approval(sess, key).await;
            }
        }
        McpToolApprovalDecision::Accept
        | McpToolApprovalDecision::Decline { .. }
        | McpToolApprovalDecision::Cancel
        | McpToolApprovalDecision::BlockedBySafetyMonitor(_) => {}
    }
}

async fn maybe_persist_mcp_tool_approval(
    sess: &Session,
    turn_context: &TurnContext,
    key: McpToolApprovalKey,
) {
    let tool_name = key.tool_name.clone();

    let persist_result = if key.server == CODEX_APPS_MCP_SERVER_NAME {
        let Some(connector_id) = key.connector_id.clone() else {
            remember_mcp_tool_approval(sess, key).await;
            return;
        };
        persist_codex_app_tool_approval(&turn_context.config, &connector_id, &tool_name).await
    } else {
        persist_non_app_mcp_tool_approval(sess, &turn_context.config, &key.server, &tool_name).await
    };

    if let Err(err) = persist_result {
        error!(
            error = %err,
            server = key.server,
            tool_name,
            "failed to persist MCP tool approval"
        );
        remember_mcp_tool_approval(sess, key).await;
        return;
    }

    sess.reload_user_config_layer().await;
    remember_mcp_tool_approval(sess, key).await;
}

async fn persist_codex_app_tool_approval(
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

#[cfg(test)]
async fn persist_custom_mcp_tool_approval(
    config: &Config,
    server: &str,
    tool_name: &str,
) -> anyhow::Result<()> {
    let Some(config_edits_builder) = custom_mcp_tool_approval_config_builder(config, server)?
    else {
        anyhow::bail!("MCP server `{server}` is not configured in config.toml");
    };

    persist_custom_mcp_tool_approval_with(config_edits_builder, server, tool_name).await
}

async fn persist_non_app_mcp_tool_approval(
    sess: &Session,
    config: &Config,
    server: &str,
    tool_name: &str,
) -> anyhow::Result<()> {
    if let Some(config_edits_builder) = custom_mcp_tool_approval_config_builder(config, server)? {
        return persist_custom_mcp_tool_approval_with(config_edits_builder, server, tool_name)
            .await;
    }

    let plugin_config_name = sess
        .services
        .plugins_manager
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

fn custom_mcp_tool_approval_config_builder(
    config: &Config,
    server: &str,
) -> anyhow::Result<Option<ConfigEditsBuilder>> {
    if let Some(project_config_folder) = project_mcp_tool_approval_config_folder(config, server) {
        return Ok(Some(ConfigEditsBuilder::new(&project_config_folder)));
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
) -> Option<AbsolutePathBuf> {
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

async fn notify_mcp_tool_call_skip(
    sess: &Session,
    turn_context: &TurnContext,
    call_id: &str,
    invocation: McpInvocation,
    mcp_app_resource_uri: Option<String>,
    message: String,
    already_started: bool,
) -> Result<CallToolResult, String> {
    if !already_started {
        notify_mcp_tool_call_started(
            sess,
            turn_context,
            call_id,
            invocation.clone(),
            mcp_app_resource_uri.clone(),
        )
        .await;
    }

    notify_mcp_tool_call_completed(
        sess,
        turn_context,
        call_id,
        invocation,
        mcp_app_resource_uri,
        Duration::ZERO,
        truncate_mcp_tool_result_for_event(
            &Err(message.clone()),
            MCP_TOOL_CALL_EVENT_RESULT_MAX_BYTES,
        ),
    )
    .await;
    Err(message)
}

#[cfg(test)]
#[path = "mcp_tool_call_tests.rs"]
mod tests;
