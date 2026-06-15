use crate::tools::code_mode::execute_spec::create_code_mode_tool;
use crate::tools::flat_tool_name;
use crate::tools::handlers::ApplyPatchHandler;
use crate::tools::handlers::CodeModeExecuteHandler;
use crate::tools::handlers::CodeModeWaitHandler;
use crate::tools::handlers::CommandWaitHandler;
use crate::tools::handlers::CreateGoalHandler;
use crate::tools::handlers::DynamicToolHandler;
use crate::tools::handlers::ExecCommandHandler;
use crate::tools::handlers::ExecCommandHandlerOptions;
use crate::tools::handlers::GetGoalHandler;
use crate::tools::handlers::ListMcpResourceTemplatesHandler;
use crate::tools::handlers::ListMcpResourcesHandler;
use crate::tools::handlers::McpHandler;
use crate::tools::handlers::PlanHandler;
use crate::tools::handlers::ReadMcpResourceHandler;
use crate::tools::handlers::RequestPermissionsHandler;
use crate::tools::handlers::RequestPluginInstallHandler;
use crate::tools::handlers::RequestUserInputHandler;
use crate::tools::handlers::ShellCommandHandler;
use crate::tools::handlers::ShellCommandHandlerOptions;
use crate::tools::handlers::SpawnAgentHandler as SpawnAgentHandlerV2;
use crate::tools::handlers::TestSyncHandler;
use crate::tools::handlers::ToolSearchHandler;
use crate::tools::handlers::UpdateGoalHandler;
use crate::tools::handlers::ViewImageHandler;
use crate::tools::handlers::WorkflowAbortHandler;
use crate::tools::handlers::WorkflowDescribeHandler;
use crate::tools::handlers::WorkflowListHandler;
use crate::tools::handlers::WorkflowResumeHandler;
use crate::tools::handlers::WorkflowStartHandler;
use crate::tools::handlers::WorkflowStatusHandler;
use crate::tools::handlers::WriteStdinHandler;
use crate::tools::handlers::agent_jobs::ReportAgentJobResultHandler;
use crate::tools::handlers::agent_jobs::SpawnAgentsOnCsvHandler;
use crate::tools::handlers::extension_tools::ExtensionToolHandler;
use crate::tools::handlers::multi_agents_spec::SpawnAgentToolOptions;
use crate::tools::handlers::multi_agents_v2::CloseAgentHandler as CloseAgentHandlerV2;
use crate::tools::handlers::multi_agents_v2::FollowupTaskHandler as FollowupTaskHandlerV2;
use crate::tools::handlers::multi_agents_v2::ListAgentsHandler as ListAgentsHandlerV2;
use crate::tools::handlers::multi_agents_v2::WaitAgentHandler as WaitAgentHandlerV2;
use crate::tools::handlers::view_image_spec::ViewImageToolOptions;
use crate::tools::registry::RegisteredTool;
use crate::tools::registry::ToolExposure;
use crate::tools::registry::ToolRegistryBuilder;
use crate::tools::registry::override_tool_exposure;
use crate::tools::spec_plan_types::ToolRegistryBuildParams;
use crate::tools::spec_plan_types::agent_type_description;
use codex_extension_api::ExtensionToolExecutor;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_tools::TOOL_SEARCH_TOOL_NAME;
use codex_tools::ToolEnvironmentMode;
use codex_tools::ToolName;
use codex_tools::ToolSpec;
use codex_tools::ToolsConfig;
use codex_tools::WebSearchToolOptions;
use codex_tools::collect_code_mode_exec_prompt_tool_definitions;
use codex_tools::create_image_generation_tool;
use codex_tools::create_web_search_tool;
use codex_tools::default_namespace_description;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::warn;

pub(crate) fn build_tool_registry_builder_from_executors(
    config: &ToolsConfig,
    executors: Vec<Arc<dyn RegisteredTool>>,
    hosted_specs: Vec<ToolSpec>,
) -> ToolRegistryBuilder {
    let mut builder = ToolRegistryBuilder::new();
    let executors = filter_executors_for_agent(config, executors);
    let hosted_specs = filter_specs_for_agent(config, hosted_specs);
    let deferred_tools_available = executors
        .iter()
        .any(|executor| executor.exposure() == ToolExposure::Deferred);

    for executor in build_code_mode_executors(
        config,
        &executors,
        config.search_tool && deferred_tools_available,
    ) {
        builder.register_tool(executor);
    }

    let mut non_deferred_specs = Vec::new();
    let mut deferred_search_infos = Vec::new();
    for executor in &executors {
        match executor.exposure() {
            ToolExposure::Direct | ToolExposure::DirectModelOnly => {
                if let Some(spec) = executor.spec() {
                    non_deferred_specs.push((spec, executor.exposure()));
                }
            }
            ToolExposure::Deferred => {
                if let Some(search_info) = executor.search_info() {
                    deferred_search_infos.push(search_info);
                }
            }
        }
    }

    non_deferred_specs.extend(
        hosted_specs
            .into_iter()
            .map(|spec| (spec, ToolExposure::Direct)),
    );

    let non_deferred_specs = non_deferred_specs
        .into_iter()
        .map(|(spec, exposure)| {
            if config.code_mode_enabled && exposure != ToolExposure::DirectModelOnly {
                codex_tools::augment_tool_spec_for_code_mode(spec)
            } else {
                spec
            }
        })
        .collect();

    for spec in merge_into_namespaces(non_deferred_specs) {
        if !config.namespace_tools && matches!(spec, ToolSpec::Namespace(_)) {
            continue;
        }
        builder.push_spec(spec);
    }

    for executor in executors {
        builder.register_tool_without_spec(executor);
    }

    if config.search_tool && config.namespace_tools && !deferred_search_infos.is_empty() {
        builder.register_tool(Arc::new(ToolSearchHandler::new(deferred_search_infos)));
    }

    builder
}

fn filter_executors_for_agent(
    config: &ToolsConfig,
    executors: Vec<Arc<dyn RegisteredTool>>,
) -> Vec<Arc<dyn RegisteredTool>> {
    let Some(patterns) = config.agent_tool_patterns.as_deref() else {
        return executors;
    };
    executors
        .into_iter()
        .filter(|executor| {
            tool_name_matches_patterns(&flat_tool_name(&executor.tool_name()), patterns)
        })
        .collect()
}

fn filter_specs_for_agent(config: &ToolsConfig, specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    let Some(patterns) = config.agent_tool_patterns.as_deref() else {
        return specs;
    };
    specs
        .into_iter()
        .filter_map(|spec| filter_spec_for_agent(spec, patterns))
        .collect()
}

fn filter_spec_for_agent(mut spec: ToolSpec, patterns: &[String]) -> Option<ToolSpec> {
    match &mut spec {
        ToolSpec::Namespace(namespace) => {
            if tool_name_matches_patterns(&namespace.name, patterns) {
                return Some(spec);
            }
            let namespace_name = namespace.name.clone();
            namespace.tools.retain(|tool| match tool {
                codex_tools::ResponsesApiNamespaceTool::Function(tool) => {
                    let qualified = format!("{namespace_name}{}", tool.name);
                    tool_name_matches_patterns(&tool.name, patterns)
                        || tool_name_matches_patterns(&qualified, patterns)
                }
            });
            (!namespace.tools.is_empty()).then_some(spec)
        }
        _ => tool_name_matches_patterns(spec.name(), patterns).then_some(spec),
    }
}

fn tool_name_matches_patterns(tool_name: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim();
        if pattern == "*" {
            true
        } else if let Some(prefix) = pattern.strip_suffix('*') {
            tool_name.starts_with(prefix)
        } else {
            tool_name == pattern
        }
    })
}

pub(crate) fn hosted_model_tool_specs(config: &ToolsConfig) -> Vec<ToolSpec> {
    let mut specs = Vec::new();
    if let Some(web_search_tool) = create_web_search_tool(WebSearchToolOptions {
        web_search_mode: config.web_search_mode,
        web_search_config: config.web_search_config.as_ref(),
        web_search_tool_type: config.web_search_tool_type,
    }) {
        specs.push(web_search_tool);
    }
    if config.image_gen_tool {
        specs.push(create_image_generation_tool("png"));
    }
    specs
}

fn build_code_mode_executors(
    config: &ToolsConfig,
    executors: &[Arc<dyn RegisteredTool>],
    deferred_tools_available: bool,
) -> Vec<Arc<dyn RegisteredTool>> {
    if !config.code_mode_enabled {
        return vec![];
    }

    let code_mode_nested_tool_specs = executors
        .iter()
        .filter_map(|executor| {
            if executor.exposure() == ToolExposure::DirectModelOnly {
                return None;
            }

            executor.spec()
        })
        .collect::<Vec<_>>();
    let namespace_descriptions = code_mode_namespace_descriptions(&code_mode_nested_tool_specs);
    let mut enabled_tools =
        collect_code_mode_exec_prompt_tool_definitions(code_mode_nested_tool_specs.iter());
    enabled_tools
        .sort_by(|left, right| compare_code_mode_tools(left, right, &namespace_descriptions));

    vec![
        Arc::new(CodeModeExecuteHandler::new(
            create_code_mode_tool(
                &enabled_tools,
                &namespace_descriptions,
                config.code_mode_only_enabled,
                deferred_tools_available,
            ),
            code_mode_nested_tool_specs,
        )),
        Arc::new(CodeModeWaitHandler),
    ]
}

fn merge_into_namespaces(specs: Vec<ToolSpec>) -> Vec<ToolSpec> {
    let mut merged_specs = Vec::with_capacity(specs.len());
    let mut namespace_indices = BTreeMap::<String, usize>::new();
    for spec in specs {
        match spec {
            ToolSpec::Namespace(mut namespace) => {
                if let Some(index) = namespace_indices.get(&namespace.name).copied() {
                    let ToolSpec::Namespace(existing_namespace) = &mut merged_specs[index] else {
                        unreachable!("namespace index must point to a namespace spec");
                    };
                    if existing_namespace.description.trim().is_empty()
                        && !namespace.description.trim().is_empty()
                    {
                        existing_namespace.description = namespace.description;
                    }
                    existing_namespace.tools.append(&mut namespace.tools);
                    continue;
                }

                namespace_indices.insert(namespace.name.clone(), merged_specs.len());
                merged_specs.push(ToolSpec::Namespace(namespace));
            }
            spec => merged_specs.push(spec),
        }
    }

    for spec in &mut merged_specs {
        let ToolSpec::Namespace(namespace) = spec else {
            continue;
        };

        namespace.tools.sort_by(|left, right| match (left, right) {
            (
                codex_tools::ResponsesApiNamespaceTool::Function(left),
                codex_tools::ResponsesApiNamespaceTool::Function(right),
            ) => left.name.cmp(&right.name),
        });

        if namespace.description.trim().is_empty() {
            namespace.description = default_namespace_description(&namespace.name);
        }
    }

    merged_specs
}

fn code_mode_namespace_descriptions(
    specs: &[ToolSpec],
) -> BTreeMap<String, codex_code_mode::ToolNamespaceDescription> {
    let mut namespace_descriptions = BTreeMap::new();
    for spec in specs {
        let ToolSpec::Namespace(namespace) = spec else {
            continue;
        };

        let entry = namespace_descriptions
            .entry(namespace.name.clone())
            .or_insert_with(|| codex_code_mode::ToolNamespaceDescription {
                name: namespace.name.clone(),
                description: namespace.description.clone(),
            });
        if entry.description.trim().is_empty() && !namespace.description.trim().is_empty() {
            entry.description = namespace.description.clone();
        }
    }
    namespace_descriptions
}

pub(crate) fn collect_tool_executors(
    config: &ToolsConfig,
    params: ToolRegistryBuildParams<'_>,
) -> Vec<Arc<dyn RegisteredTool>> {
    let exec_permission_approvals_enabled = config.exec_permission_approvals_enabled;
    let mut executors = Vec::<Arc<dyn RegisteredTool>>::new();

    if config.environment_mode.has_environment() {
        let include_environment_id =
            matches!(config.environment_mode, ToolEnvironmentMode::Multiple);
        match &config.shell_type {
            ConfigShellToolType::UnifiedExec => {
                executors.push(Arc::new(ExecCommandHandler::new(
                    ExecCommandHandlerOptions {
                        allow_login_shell: config.allow_login_shell,
                        exec_permission_approvals_enabled,
                        include_environment_id,
                    },
                )));
                executors.push(Arc::new(CommandWaitHandler));
                executors.push(Arc::new(WriteStdinHandler));
            }
            ConfigShellToolType::Disabled => {}
            ConfigShellToolType::Default
            | ConfigShellToolType::Local
            | ConfigShellToolType::ShellCommand => {
                executors.push(Arc::new(ShellCommandHandler::new(
                    ShellCommandHandlerOptions {
                        backend_config: config.shell_command_backend,
                        allow_login_shell: config.allow_login_shell,
                        exec_permission_approvals_enabled,
                    },
                )));
            }
        }
    }

    if config.environment_mode.has_environment()
        && config.shell_type != ConfigShellToolType::Disabled
    {
        match &config.shell_type {
            ConfigShellToolType::UnifiedExec => {
                executors.push(Arc::new(ShellCommandHandler::from(
                    config.shell_command_backend,
                )));
            }
            ConfigShellToolType::Default
            | ConfigShellToolType::Local
            | ConfigShellToolType::ShellCommand
            | ConfigShellToolType::Disabled => {}
        }
    }

    if params.mcp_tools.is_some() {
        executors.push(Arc::new(ListMcpResourcesHandler));
        executors.push(Arc::new(ListMcpResourceTemplatesHandler));
        executors.push(Arc::new(ReadMcpResourceHandler));
    }

    executors.push(Arc::new(PlanHandler));
    if config.goal_tools {
        executors.push(Arc::new(GetGoalHandler));
        executors.push(Arc::new(CreateGoalHandler));
        executors.push(Arc::new(UpdateGoalHandler));
    }

    executors.push(Arc::new(RequestUserInputHandler {
        available_modes: config.request_user_input_available_modes.clone(),
    }));
    executors.push(Arc::new(WorkflowListHandler));
    executors.push(Arc::new(WorkflowDescribeHandler));
    executors.push(Arc::new(WorkflowStartHandler));
    executors.push(Arc::new(WorkflowStatusHandler));
    executors.push(Arc::new(WorkflowResumeHandler));
    executors.push(Arc::new(WorkflowAbortHandler));

    if config.request_permissions_tool_enabled {
        executors.push(Arc::new(RequestPermissionsHandler));
    }

    if config.tool_suggest
        && let Some(discoverable_tools) =
            params.discoverable_tools.filter(|tools| !tools.is_empty())
    {
        executors.push(Arc::new(RequestPluginInstallHandler::new(
            discoverable_tools,
        )));
    }

    if config.environment_mode.has_environment() && config.apply_patch_tool_type.is_some() {
        let include_environment_id =
            matches!(config.environment_mode, ToolEnvironmentMode::Multiple);
        executors.push(Arc::new(ApplyPatchHandler::new(include_environment_id)));
    }

    if config
        .experimental_supported_tools
        .iter()
        .any(|tool| tool == "test_sync_tool")
    {
        executors.push(Arc::new(TestSyncHandler));
    }

    if config.environment_mode.has_environment() {
        let include_environment_id =
            matches!(config.environment_mode, ToolEnvironmentMode::Multiple);
        executors.push(Arc::new(ViewImageHandler::new(ViewImageToolOptions {
            can_request_original_image_detail: config.can_request_original_image_detail,
            include_environment_id,
        })));
    }

    if config.collab_tools {
        let exposure = if config.multi_agent_v2_non_code_mode_only {
            ToolExposure::DirectModelOnly
        } else {
            ToolExposure::Direct
        };
        let agent_type_description =
            agent_type_description(config, params.default_agent_type_description);
        executors.push(multi_agent_v2_handler(
            SpawnAgentHandlerV2::new(SpawnAgentToolOptions {
                available_models: config.available_models.clone(),
                agent_type_description,
                hide_agent_type_model_reasoning: config.hide_spawn_agent_metadata,
                include_usage_hint: config.spawn_agent_usage_hint,
                usage_hint_text: config.spawn_agent_usage_hint_text.clone(),
                max_concurrent_threads_per_session: config.max_concurrent_threads_per_session,
            }),
            exposure,
        ));
        executors.push(multi_agent_v2_handler(FollowupTaskHandlerV2, exposure));
        executors.push(multi_agent_v2_handler(WaitAgentHandlerV2, exposure));
        executors.push(multi_agent_v2_handler(CloseAgentHandlerV2, exposure));
        executors.push(multi_agent_v2_handler(ListAgentsHandlerV2, exposure));
    }

    if config.agent_jobs_tools {
        executors.push(Arc::new(SpawnAgentsOnCsvHandler));
        if config.agent_jobs_worker_tools {
            executors.push(Arc::new(ReportAgentJobResultHandler));
        }
    }

    if let Some(mcp_tools) = params.mcp_tools {
        for tool in mcp_tools {
            executors.push(Arc::new(McpHandler::new(tool.clone())));
        }
    }

    if let Some(deferred_mcp_tools) = params.deferred_mcp_tools {
        for tool in deferred_mcp_tools {
            executors.push(Arc::new(McpHandler::with_exposure(
                tool.clone(),
                ToolExposure::Deferred,
            )));
        }
    }

    for tool in params.dynamic_tools {
        let Some(handler) = DynamicToolHandler::new(tool).map(Arc::new) else {
            tracing::error!(
                "Failed to convert dynamic tool {:?} to OpenAI tool",
                tool.name
            );
            continue;
        };

        executors.push(handler);
    }

    append_extension_tool_executors(config, params.extension_tool_executors, &mut executors);

    executors
}

fn append_extension_tool_executors(
    config: &ToolsConfig,
    executors: &[Arc<dyn ExtensionToolExecutor>],
    registered_executors: &mut Vec<Arc<dyn RegisteredTool>>,
) {
    if executors.is_empty() {
        return;
    }

    let mut reserved_tool_names = registered_executors
        .iter()
        .map(|executor| executor.tool_name())
        .collect::<HashSet<_>>();
    if config.code_mode_enabled {
        reserved_tool_names.insert(ToolName::plain(codex_code_mode::PUBLIC_TOOL_NAME));
        reserved_tool_names.insert(ToolName::plain(codex_code_mode::WAIT_TOOL_NAME));
    }
    if config.search_tool
        && config.namespace_tools
        && registered_executors
            .iter()
            .any(|executor| executor.exposure() == ToolExposure::Deferred)
    {
        reserved_tool_names.insert(ToolName::plain(TOOL_SEARCH_TOOL_NAME));
    }

    for executor in executors.iter().cloned() {
        let tool_name = executor.tool_name();
        if !reserved_tool_names.insert(tool_name.clone()) {
            warn!("Skipping extension tool `{tool_name}`: handler already registered");
            continue;
        }
        registered_executors.push(Arc::new(ExtensionToolHandler::new(executor)));
    }
}

fn multi_agent_v2_handler(
    handler: impl RegisteredTool + 'static,
    exposure: ToolExposure,
) -> Arc<dyn RegisteredTool> {
    override_tool_exposure(Arc::new(handler), exposure)
}

fn compare_code_mode_tools(
    left: &codex_code_mode::ToolDefinition,
    right: &codex_code_mode::ToolDefinition,
    namespace_descriptions: &BTreeMap<String, codex_code_mode::ToolNamespaceDescription>,
) -> std::cmp::Ordering {
    let left_namespace = code_mode_namespace_name(left, namespace_descriptions);
    let right_namespace = code_mode_namespace_name(right, namespace_descriptions);

    left_namespace
        .cmp(&right_namespace)
        .then_with(|| left.tool_name.name.cmp(&right.tool_name.name))
        .then_with(|| left.name.cmp(&right.name))
}

fn code_mode_namespace_name<'a>(
    tool: &codex_code_mode::ToolDefinition,
    namespace_descriptions: &'a BTreeMap<String, codex_code_mode::ToolNamespaceDescription>,
) -> Option<&'a str> {
    tool.tool_name
        .namespace
        .as_ref()
        .and_then(|namespace| namespace_descriptions.get(namespace))
        .map(|namespace_description| namespace_description.name.as_str())
}

#[cfg(test)]
#[path = "spec_plan_tests.rs"]
mod tests;
