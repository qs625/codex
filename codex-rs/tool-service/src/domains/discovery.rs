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
use codex_mcp_tool_types::ToolInfo;
use thread_service_api::RequestPluginInstallApi;
use thread_service_api::ThreadCapability;
use codex_tool_service_api::AnyToolResult;
use codex_tool_service_api::ErasedToolArgumentDiffConsumer;
use codex_tool_types::FunctionCallError;
use codex_tool_types::ToolCall;
use codex_tool_types::ToolName;
use serde::Deserialize;
use std::sync::Arc;

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
    request_plugin_install_api: Arc<dyn RequestPluginInstallApi>,
    turn: &dyn ThreadCapability,
    dynamic_tools: &[codex_protocol::dynamic_tools::DynamicToolSpec],
    mcp_tools: Option<&[ToolInfo]>,
    deferred_mcp_tools: Option<&[ToolInfo]>,
    _discoverable_tools: Option<&[DiscoverableTool]>,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    match call.tool_name.name.as_str() {
        TOOL_SEARCH_TOOL_NAME => {
            dispatch_tool_search(dynamic_tools, mcp_tools, deferred_mcp_tools, call)
        }
        REQUEST_PLUGIN_INSTALL_TOOL_NAME => {
            dispatch_request_plugin_install(request_plugin_install_api, turn, call).await
        }
        _ => Err(FunctionCallError::Fatal(format!(
            "unsupported discovery tool {}",
            call.tool_name
        ))),
    }
}

fn dispatch_tool_search(
    dynamic_tools: &[codex_protocol::dynamic_tools::DynamicToolSpec],
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
        codex_tool_types::ToolPayload::ToolSearch { arguments } => arguments.clone(),
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
    service: Arc<dyn RequestPluginInstallApi>,
    turn: &dyn ThreadCapability,
    call: ToolCall,
) -> Result<AnyToolResult, FunctionCallError> {
    let args: RequestPluginInstallArgs = parse_function_arguments(&call)?;
    let context = service.request_plugin_install_context(turn);
    let suggest_reason =
        validate_request_plugin_install_args(&args, context.app_server_client_name.as_deref())?;

    let discoverable_tools = service
        .list_request_plugin_install_discoverable_tools(turn)
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
    let outcome = service
        .request_plugin_install_elicitation(turn, &call.call_id, request, &tool)
        .await;

    let completed = if outcome.user_confirmed {
        service
            .complete_request_plugin_install_if_ready(turn, &tool)
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

    Ok(AnyToolResult {
        call_id: call.call_id,
        payload: call.payload,
        result: Box::new(FunctionToolOutput::from_text(content, Some(true))),
        post_tool_use_payload: None,
    })
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
    dynamic_tools: &[codex_protocol::dynamic_tools::DynamicToolSpec],
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

fn dynamic_tool_to_spec(tool: &codex_protocol::dynamic_tools::DynamicToolSpec) -> Option<ToolSpec> {
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

fn build_dynamic_search_text(tool: &codex_protocol::dynamic_tools::DynamicToolSpec) -> String {
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
