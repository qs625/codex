use crate::planning::DiscoverableTool;
use crate::planning::DiscoverableToolAction;
use crate::planning::DiscoverableToolType;
use crate::planning::REQUEST_PLUGIN_INSTALL_TOOL_NAME;
use crate::planning::RequestPluginInstallArgs;
use crate::planning::RequestPluginInstallResult;
use crate::planning::ResponsesApiNamespace;
use crate::planning::ResponsesApiNamespaceTool;
use crate::planning::TOOL_SEARCH_DEFAULT_LIMIT;
use crate::planning::TOOL_SEARCH_TOOL_NAME;
use crate::planning::ToolSearchInfo;
use crate::planning::ToolSearchRuntime;
use crate::planning::ToolSearchSourceInfo;
use crate::planning::ToolSpec;
use crate::planning::build_request_plugin_install_elicitation_request;
use crate::planning::collect_request_plugin_install_entries;
use crate::planning::create_request_plugin_install_tool;
use crate::planning::create_tool_search_tool;
use crate::planning::dynamic_tool_to_responses_api_tool;
use crate::planning::filter_request_plugin_install_discoverable_tools_for_client;
use crate::planning::mcp_tool_to_deferred_responses_api_tool;
use crate::planning::mcp_tool_to_responses_api_tool;
use config_service::ConfigEdit;
use config_service::ConfigEditsBuilder;
use codex_config_types::ToolSuggestDisabledTool;
use mcp_types::CODEX_APPS_MCP_SERVER_NAME;
use mcp_types::ElicitationAction;
use mcp_types::ElicitationResponse;
use mcp_types::McpElicitationObjectType;
use mcp_types::McpElicitationSchema;
use mcp_types::McpServerElicitationRequest;
use mcp_types::McpServerElicitationRequestParams;
use mcp_types::ToolInfo;
use protocol::mcp::RequestId;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Arc;
use thread_service_api::ThreadRuntimeCapability;
use thread_service_api::ThreadSessionCapability;
use thread_service_api::ThreadTurnCapability;
use tool_service_api::AnyToolResult;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::FunctionCallError;
use tool_service_api::REQUEST_PLUGIN_INSTALL_PERSIST_ALWAYS_VALUE;
use tool_service_api::REQUEST_PLUGIN_INSTALL_PERSIST_KEY;
use tool_service_api::RequestPluginInstallElicitationRequest;
use tool_service_api::RequestPluginInstallElicitationSchema;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;
use tool_service_api::all_requested_connectors_picked_up;
use tool_service_api::verified_connector_install_completed;
use tracing::warn;

use crate::context::TypedToolSpecRequest;
use crate::output::FunctionToolOutput;

pub(crate) fn specs(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSpec> {
    let mut specs = Vec::new();
    let search_infos = search_infos(request);
    if !search_infos.is_empty() {
        let source_infos = search_infos
            .iter()
            .filter_map(|info| info.source_info.clone())
            .collect::<Vec<_>>();
        specs.push(create_tool_search_tool(
            &source_infos,
            TOOL_SEARCH_DEFAULT_LIMIT,
        ));
    }

    if let Some(discoverable_tools) = request.params.discoverable_tools
        && !discoverable_tools.is_empty()
    {
        specs.push(create_request_plugin_install_tool(
            &collect_request_plugin_install_entries(discoverable_tools),
        ));
    }

    specs
}

pub(crate) fn owns_tool_name(_request: &TypedToolSpecRequest<'_>, tool_name: &ToolName) -> bool {
    tool_name.namespace.is_none()
        && matches!(
            tool_name.name.as_str(),
            TOOL_SEARCH_TOOL_NAME | REQUEST_PLUGIN_INSTALL_TOOL_NAME
        )
}

pub(crate) fn create_diff_consumer(
    _request: &TypedToolSpecRequest<'_>,
    _tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    None
}

pub(crate) fn supports_parallel(_request: &TypedToolSpecRequest<'_>, _call: &ToolCall) -> bool {
    true
}

pub(crate) async fn dispatch(
    session: Arc<dyn ThreadSessionCapability>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    dynamic_tools: &[protocol::dynamic_tools::DynamicToolSpec],
    mcp_tools: Option<&[ToolInfo]>,
    deferred_mcp_tools: Option<&[ToolInfo]>,
    discoverable_tools: Option<&[DiscoverableTool]>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    match call.tool_name.name.as_str() {
        TOOL_SEARCH_TOOL_NAME => {
            dispatch_tool_search(dynamic_tools, mcp_tools, deferred_mcp_tools, call)
        }
        REQUEST_PLUGIN_INSTALL_TOOL_NAME => {
            dispatch_request_plugin_install(session, turn, discoverable_tools, call).await
        }
        _ => Err(FunctionCallError::Fatal(format!(
            "unsupported discovery tool {}",
            call.tool_name
        ))),
    }
}

fn dispatch_tool_search(
    dynamic_tools: &[protocol::dynamic_tools::DynamicToolSpec],
    mcp_tools: Option<&[ToolInfo]>,
    deferred_mcp_tools: Option<&[ToolInfo]>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let runtime = ToolSearchRuntime::new(search_infos_from_parts(
        dynamic_tools,
        mcp_tools,
        deferred_mcp_tools,
    ));
    let arguments = match &call.payload {
        tool_service_api::ToolPayload::ToolSearch { arguments } => arguments.clone(),
        _ => {
            return Err(FunctionCallError::Fatal(format!(
                "{TOOL_SEARCH_TOOL_NAME} handler received unsupported payload"
            )));
        }
    };
    let result = runtime.handle_search(arguments)?;
    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(result),
        post_tool_use_payload: None,
    })
}

async fn dispatch_request_plugin_install(
    session: Arc<dyn ThreadSessionCapability>,
    turn: Arc<dyn ThreadRuntimeCapability>,
    discoverable_tools: Option<&[DiscoverableTool]>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let args: RequestPluginInstallArgs = parse_function_arguments(&call)?;
    let client_name = turn.app_server_client_name();
    let suggest_reason = validate_request_plugin_install_args(&args, client_name)?;

    let discoverable_tools = filter_request_plugin_install_discoverable_tools_for_client(
        discoverable_tools.unwrap_or_default().to_vec(),
        client_name,
    );

    let tool = discoverable_tools
        .into_iter()
        .find(|tool| tool.tool_type() == args.tool_type && tool.id() == args.tool_id)
        .ok_or_else(|| {
            FunctionCallError::RespondToModel(format!(
                "tool_id must match one of the discoverable tools exposed by {REQUEST_PLUGIN_INSTALL_TOOL_NAME}"
            ))
        })?;

    let request = build_request_plugin_install_elicitation_request(
        CODEX_APPS_MCP_SERVER_NAME,
        turn.thread_id().to_string(),
        turn.runtime_turn_id_str().to_string(),
        &args,
        suggest_reason,
        &tool,
    );
    let request_id = RequestId::String(format!("request_plugin_install_{}", call.call_id));
    let params = request_plugin_install_elicitation_request_to_mcp_params(request);
    let response = session
        .request_mcp_server_elicitation(turn.as_ref(), request_id, params)
        .await;
    if let Some(response) = response.as_ref() {
        maybe_persist_disabled_install_request(turn.as_ref(), &tool, response).await;
    }

    let user_confirmed = response
        .as_ref()
        .is_some_and(|response| response.action == ElicitationAction::Accept);
    let completed = if user_confirmed {
        complete_request_plugin_install_if_ready(session.as_ref(), turn.as_ref(), &tool).await
    } else {
        false
    };

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

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(FunctionToolOutput::from_text(content, Some(true))),
        post_tool_use_payload: None,
    })
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
    turn: &dyn ThreadTurnCapability,
    tool: &DiscoverableTool,
    response: &ElicitationResponse,
) {
    if !request_plugin_install_response_requests_persistent_disable(response) {
        return;
    }

    if let Err(err) =
        persist_disabled_install_request(&turn.discovery_context().home_root, tool).await
    {
        warn!(
            error = %err,
            tool_id = tool.id(),
            "failed to persist disabled tool suggestion"
        );
    }
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
    codex_home: &std::path::Path,
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

async fn complete_request_plugin_install_if_ready(
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadTurnCapability,
    tool: &DiscoverableTool,
) -> bool {
    let auth_snapshot = turn.auth_snapshot().await;
    match tool {
        DiscoverableTool::Connector(connector) => {
            let completed = refresh_missing_requested_connectors(
                session,
                turn,
                auth_snapshot.as_ref(),
                std::slice::from_ref(&connector.id),
                connector.id.as_str(),
            )
            .await
            .is_some_and(|accessible_connectors| {
                verified_connector_install_completed(connector.id.as_str(), &accessible_connectors)
            });
            if completed {
                let _ = session
                    .merge_connector_selection(HashSet::from([connector.id.clone()]))
                    .await;
            }
            completed
        }
        DiscoverableTool::Plugin(plugin) => {
            session.reload_user_config_layer().await;
            let completed = session
                .configured_plugin_installed(plugin.id.as_str())
                .await;
            let _ = refresh_missing_requested_connectors(
                session,
                turn,
                auth_snapshot.as_ref(),
                &plugin.app_connector_ids,
                plugin.id.as_str(),
            )
            .await;
            completed
        }
    }
}

async fn refresh_missing_requested_connectors(
    session: &dyn ThreadSessionCapability,
    turn: &dyn ThreadTurnCapability,
    auth_snapshot: Option<&codex_auth_types::RequestAuthSnapshot>,
    expected_connector_ids: &[String],
    tool_id: &str,
) -> Option<Vec<codex_connectors_api::AppInfo>> {
    if expected_connector_ids.is_empty() {
        return Some(Vec::new());
    }

    let accessible_connectors = turn
        .cached_accessible_connectors_from_mcp_tools(auth_snapshot)
        .await;
    if accessible_connectors.as_ref().is_some_and(|connectors| {
        all_requested_connectors_picked_up(expected_connector_ids, connectors)
    }) {
        return accessible_connectors;
    }

    match session.hard_refresh_codex_apps_tools_cache().await {
        Ok(_) => match session
            .fetch_accessible_connectors_from_mcp_tools(turn, auth_snapshot)
            .await
        {
            Ok(connectors) => Some(connectors),
            Err(err) => {
                warn!(
                    "failed to refresh accessible connectors after plugin install request for {tool_id}: {err:#}"
                );
                None
            }
        },
        Err(err) => {
            warn!(
                "failed to refresh codex apps tools cache after plugin install request for {tool_id}: {err}"
            );
            None
        }
    }
}

fn search_infos(request: &TypedToolSpecRequest<'_>) -> Vec<ToolSearchInfo> {
    let mut infos = Vec::new();

    for tool in request.params.dynamic_tools {
        let Some(spec) = dynamic_tool_to_spec(tool) else {
            continue;
        };
        if let Some(info) = ToolSearchInfo::from_spec(
            build_dynamic_search_text(tool),
            spec,
            Some(ToolSearchSourceInfo {
                name: "Dynamic tools".to_string(),
                description: Some("Tools provided by the current Codex thread.".to_string()),
            }),
        ) {
            infos.push(info);
        }
    }

    for tool in request.params.mcp_tools.into_iter().flatten() {
        if let Some(info) = tool_info_to_search_info(tool, /*deferred*/ false) {
            infos.push(info);
        }
    }

    for tool in request.params.deferred_mcp_tools.into_iter().flatten() {
        if let Some(info) = tool_info_to_search_info(tool, /*deferred*/ true) {
            infos.push(info);
        }
    }

    infos
}

fn search_infos_from_parts(
    dynamic_tools: &[protocol::dynamic_tools::DynamicToolSpec],
    mcp_tools: Option<&[ToolInfo]>,
    deferred_mcp_tools: Option<&[ToolInfo]>,
) -> Vec<ToolSearchInfo> {
    let mut infos = Vec::new();

    for tool in dynamic_tools {
        let Some(spec) = dynamic_tool_to_spec(tool) else {
            continue;
        };
        if let Some(info) = ToolSearchInfo::from_spec(
            build_dynamic_search_text(tool),
            spec,
            Some(ToolSearchSourceInfo {
                name: "Dynamic tools".to_string(),
                description: Some("Tools provided by the current Codex thread.".to_string()),
            }),
        ) {
            infos.push(info);
        }
    }

    for tool in mcp_tools.into_iter().flatten() {
        if let Some(info) = tool_info_to_search_info(tool, /*deferred*/ false) {
            infos.push(info);
        }
    }

    for tool in deferred_mcp_tools.into_iter().flatten() {
        if let Some(info) = tool_info_to_search_info(tool, /*deferred*/ true) {
            infos.push(info);
        }
    }

    infos
}

fn dynamic_tool_to_spec(tool: &protocol::dynamic_tools::DynamicToolSpec) -> Option<ToolSpec> {
    let output_tool = dynamic_tool_to_responses_api_tool(tool).ok()?;
    Some(match tool.namespace.as_ref() {
        Some(namespace) => ToolSpec::Namespace(ResponsesApiNamespace {
            name: namespace.clone(),
            description: crate::planning::default_namespace_description(namespace),
            tools: vec![ResponsesApiNamespaceTool::Function(output_tool)],
        }),
        None => ToolSpec::Function(output_tool),
    })
}

fn build_dynamic_search_text(tool: &protocol::dynamic_tools::DynamicToolSpec) -> String {
    match tool.namespace.as_deref() {
        Some(namespace) => format!("{namespace} {} {}", tool.name, tool.description),
        None => format!("{} {}", tool.name, tool.description),
    }
}

fn tool_info_to_search_info(tool: &ToolInfo, deferred: bool) -> Option<ToolSearchInfo> {
    let spec = tool_info_to_spec(tool, deferred)?;
    let source_name = tool
        .connector_name
        .as_deref()
        .map(str::trim)
        .filter(|name: &&str| !name.is_empty())
        .unwrap_or_else(|| tool.server_name.trim());
    let source_info = (!source_name.is_empty()).then(|| ToolSearchSourceInfo {
        name: source_name.to_string(),
        description: tool
            .namespace_description
            .as_deref()
            .map(str::trim)
            .filter(|description: &&str| !description.is_empty())
            .map(str::to_string),
    });

    ToolSearchInfo::from_spec(build_mcp_search_text(tool), spec, source_info)
}

fn tool_info_to_spec(tool: &ToolInfo, deferred: bool) -> Option<ToolSpec> {
    let tool_name = tool.canonical_tool_name();
    let namespace_name = tool_name.namespace.as_ref()?;
    let function = if deferred {
        mcp_tool_to_deferred_responses_api_tool(&tool_name, &tool.tool).ok()?
    } else {
        mcp_tool_to_responses_api_tool(&tool_name, &tool.tool).ok()?
    };
    let description = tool
        .namespace_description
        .as_deref()
        .map(str::trim)
        .filter(|value: &&str| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            tool.connector_name
                .as_deref()
                .map(str::trim)
                .filter(|value: &&str| !value.is_empty())
                .map(|value| format!("Tools for working with {value}."))
        })
        .unwrap_or_default();

    Some(ToolSpec::Namespace(ResponsesApiNamespace {
        name: namespace_name.clone(),
        description,
        tools: vec![ResponsesApiNamespaceTool::Function(function)],
    }))
}

fn build_mcp_search_text(info: &ToolInfo) -> String {
    let tool_name = info.canonical_tool_name();
    let description = info.tool.description.as_deref().unwrap_or_default();
    format!(
        "{} {} {} {}",
        info.server_name, tool_name, info.tool.name, description
    )
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

fn parse_function_arguments<T>(call: &ToolCall) -> Result<T, FunctionCallError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(call.function_arguments()?).map_err(|err| {
        FunctionCallError::RespondToModel(format!(
            "failed to parse {} arguments: {err}",
            call.tool_name
        ))
    })
}
