use crate::handlers::apply_patch::ApplyPatchActiveNetworkApproval;
use crate::handlers::apply_patch::ApplyPatchDeferredNetworkApproval;
use crate::handlers::apply_patch::ApplyPatchHandler;
use crate::handlers::code_mode::CodeModeExecuteHandler;
use crate::handlers::code_mode::CodeModeWaitHandler;
use crate::handlers::command_interaction::CommandWaitHandler;
use crate::handlers::command_interaction::WriteStdinHandler;
use crate::handlers::exec_command::ExecCommandHandler;
use crate::handlers::exec_command::ExecCommandHandlerOptions;
use crate::handlers::extension_tools::ExtensionToolHandler;
use crate::handlers::function_tools::DynamicToolHandler;
use crate::handlers::function_tools::PlanHandler;
use crate::handlers::function_tools::RequestPermissionsHandler;
use crate::handlers::function_tools::RequestUserInputHandler;
use crate::handlers::function_tools::ViewImageHandler;
use crate::handlers::goal::CreateGoalHandler;
use crate::handlers::goal::GetGoalHandler;
use crate::handlers::goal::UpdateGoalHandler;
use crate::handlers::mcp_resource::ListMcpResourceTemplatesHandler;
use crate::handlers::mcp_resource::ListMcpResourcesHandler;
use crate::handlers::mcp_resource::ReadMcpResourceHandler;
use crate::handlers::mcp_tool_call::McpHandler;
use crate::handlers::request_plugin_install::RequestPluginInstallHandler;
use crate::handlers::shell::ShellActiveNetworkApproval;
use crate::handlers::shell::ShellCommandHandler;
use crate::handlers::shell::ShellCommandHandlerOptions;
use crate::handlers::shell::ShellDeferredNetworkApproval;
use crate::handlers::test_sync::TestSyncHandler;
use crate::handlers::tool_search::ToolSearchHandler;
use crate::handlers::workflow::WorkflowAbortHandler;
use crate::handlers::workflow::WorkflowDescribeHandler;
use crate::handlers::workflow::WorkflowListHandler;
use crate::handlers::workflow::WorkflowResumeHandler;
use crate::handlers::workflow::WorkflowStartHandler;
use crate::handlers::workflow::WorkflowStatusHandler;
use codex_agent_tool_handlers::CloseAgentHandler;
use codex_agent_tool_handlers::FollowupTaskHandler;
use codex_agent_tool_handlers::ListAgentsHandler;
use codex_agent_tool_handlers::ReportAgentJobResultHandler;
use codex_agent_tool_handlers::SpawnAgentHandler;
use codex_agent_tool_handlers::SpawnAgentsOnCsvHandler;
use codex_agent_tool_handlers::WaitAgentHandler;
use codex_extension_api::ExtensionToolExecutor;
use codex_mcp_tool_types::ToolInfo;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_tool_config::ToolEnvironmentMode;
use codex_tool_config::ToolsConfig;
use codex_tool_planning::CodeModeExecPlan;
use codex_tool_planning::DiscoverableTool;
use codex_tool_planning::SpawnAgentToolOptions;
use codex_tool_planning::TOOL_SEARCH_TOOL_NAME;
use codex_tool_planning::ToolName;
use codex_tool_planning::ToolSpec;
use codex_tool_planning::code_mode_exec_plan_for_specs;
use codex_tool_planning::create_code_mode_tool;
use codex_tool_planning::hosted_model_tool_specs;
use codex_tool_planning::plan_tool_registry_entries;
use codex_tool_runtime::ToolInvocation;
use codex_tool_runtime::ToolRegistry;
use codex_tool_runtime::ToolRegistryBuilder;
use codex_tool_runtime::ToolRouter;
use codex_tool_runtime_api::ApplyPatchHandlerHost;
use codex_tool_runtime_api::RegisteredTool;
use codex_tool_runtime_api::ToolDomainHost;
use codex_tool_runtime_api::ToolHandler;
use codex_tool_types::ToolExposure;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::warn;

pub type DomainInvocation<Host> = ToolInvocation<
    <Host as ApplyPatchHandlerHost>::Session,
    <Host as ApplyPatchHandlerHost>::Turn,
    <Host as ApplyPatchHandlerHost>::Tracker,
>;

pub type DomainRegisteredTool<Host> =
    dyn RegisteredTool<DomainInvocation<Host>, <Host as ApplyPatchHandlerHost>::DiffContext>;

#[derive(Clone, Copy)]
pub struct ToolRuntimeBuildParams<'a> {
    pub mcp_tools: Option<&'a [ToolInfo]>,
    pub deferred_mcp_tools: Option<&'a [ToolInfo]>,
    pub discoverable_tools: Option<&'a [DiscoverableTool]>,
    pub extension_tool_executors: &'a [Arc<dyn ExtensionToolExecutor>],
    pub dynamic_tools: &'a [DynamicToolSpec],
    pub default_agent_type_description: &'a str,
}

pub trait RuntimeToolAssemblyHost: ToolDomainHost
where
    Self: Sized,
    ApplyPatchActiveNetworkApproval<Self>: Send,
    ApplyPatchDeferredNetworkApproval<Self>: Send,
    ShellActiveNetworkApproval<Self>: Send,
    ShellDeferredNetworkApproval<Self>: Send,
{
}

impl<Host> RuntimeToolAssemblyHost for Host
where
    Host: ToolDomainHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
{
}

pub fn collect_tool_executors<Host>(
    config: &ToolsConfig,
    host: &Host,
    params: ToolRuntimeBuildParams<'_>,
) -> Vec<Arc<DomainRegisteredTool<Host>>>
where
    Host: RuntimeToolAssemblyHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
{
    let mut executors = Vec::<Arc<DomainRegisteredTool<Host>>>::new();

    if config.environment_mode.has_environment() {
        match &config.shell_type {
            ConfigShellToolType::UnifiedExec => {
                let include_environment_id =
                    matches!(config.environment_mode, ToolEnvironmentMode::Multiple);
                push_domain_tool::<Host, _>(
                    &mut executors,
                    ExecCommandHandler::new(
                        host.clone(),
                        ExecCommandHandlerOptions {
                            allow_login_shell: config.allow_login_shell,
                            exec_permission_approvals_enabled: config
                                .exec_permission_approvals_enabled,
                            include_environment_id,
                        },
                    ),
                );
                push_domain_tool::<Host, _>(&mut executors, CommandWaitHandler::new(host.clone()));
                push_domain_tool::<Host, _>(&mut executors, WriteStdinHandler::new(host.clone()));
            }
            ConfigShellToolType::Disabled => {}
            ConfigShellToolType::Default
            | ConfigShellToolType::Local
            | ConfigShellToolType::ShellCommand => {
                push_domain_tool::<Host, _>(
                    &mut executors,
                    ShellCommandHandler::new(
                        host.clone(),
                        ShellCommandHandlerOptions {
                            backend_config: config.shell_command_backend,
                            allow_login_shell: config.allow_login_shell,
                            exec_permission_approvals_enabled: config
                                .exec_permission_approvals_enabled,
                        },
                    ),
                );
            }
        }
    }

    if config.environment_mode.has_environment()
        && config.shell_type != ConfigShellToolType::Disabled
        && matches!(&config.shell_type, ConfigShellToolType::UnifiedExec)
    {
        push_domain_tool::<Host, _>(
            &mut executors,
            ShellCommandHandler::from_backend_config(host.clone(), config.shell_command_backend),
        );
    }

    if params.mcp_tools.is_some() {
        push_domain_tool::<Host, _>(&mut executors, ListMcpResourcesHandler::new(host.clone()));
        push_domain_tool::<Host, _>(
            &mut executors,
            ListMcpResourceTemplatesHandler::new(host.clone()),
        );
        push_domain_tool::<Host, _>(&mut executors, ReadMcpResourceHandler::new(host.clone()));
    }

    push_domain_tool::<Host, _>(&mut executors, PlanHandler::new(host.clone()));
    if config.goal_tools {
        push_domain_tool::<Host, _>(&mut executors, GetGoalHandler::new(host.clone()));
        push_domain_tool::<Host, _>(&mut executors, CreateGoalHandler::new(host.clone()));
        push_domain_tool::<Host, _>(&mut executors, UpdateGoalHandler::new(host.clone()));
    }

    push_domain_tool::<Host, _>(
        &mut executors,
        RequestUserInputHandler::new(
            host.clone(),
            config.request_user_input_available_modes.clone(),
        ),
    );
    push_domain_tool::<Host, _>(&mut executors, WorkflowListHandler::new(host.clone()));
    push_domain_tool::<Host, _>(&mut executors, WorkflowDescribeHandler::new(host.clone()));
    push_domain_tool::<Host, _>(&mut executors, WorkflowStartHandler::new(host.clone()));
    push_domain_tool::<Host, _>(&mut executors, WorkflowStatusHandler::new(host.clone()));
    push_domain_tool::<Host, _>(&mut executors, WorkflowResumeHandler::new(host.clone()));
    push_domain_tool::<Host, _>(&mut executors, WorkflowAbortHandler::new(host.clone()));

    if config.request_permissions_tool_enabled {
        push_domain_tool::<Host, _>(&mut executors, RequestPermissionsHandler::new(host.clone()));
    }

    if config.tool_suggest
        && params
            .discoverable_tools
            .is_some_and(|tools| !tools.is_empty())
        && let Some(discoverable_tools) = params.discoverable_tools
    {
        push_domain_tool::<Host, _>(
            &mut executors,
            RequestPluginInstallHandler::new(host.clone(), discoverable_tools),
        );
    }

    if config.environment_mode.has_environment() && config.apply_patch_tool_type.is_some() {
        let include_environment_id =
            matches!(config.environment_mode, ToolEnvironmentMode::Multiple);
        push_domain_tool::<Host, _>(
            &mut executors,
            ApplyPatchHandler::with_host(include_environment_id, host.clone()),
        );
    }

    if config
        .experimental_supported_tools
        .iter()
        .any(|tool| tool == "test_sync_tool")
    {
        push_domain_tool::<Host, _>(
            &mut executors,
            TestSyncHandler::<DomainInvocation<Host>>::new(),
        );
    }

    if config.environment_mode.has_environment() {
        let include_environment_id =
            matches!(config.environment_mode, ToolEnvironmentMode::Multiple);
        push_domain_tool::<Host, _>(
            &mut executors,
            ViewImageHandler::new(
                host.clone(),
                codex_tool_planning::ViewImageToolOptions {
                    can_request_original_image_detail: config.can_request_original_image_detail,
                    include_environment_id,
                },
            ),
        );
    }

    if config.collab_tools {
        let exposure = if config.multi_agent_v2_non_code_mode_only {
            ToolExposure::DirectModelOnly
        } else {
            ToolExposure::Direct
        };
        let agent_type_description =
            agent_type_description(config, params.default_agent_type_description);
        push_exposed_domain_tool::<Host, _>(
            &mut executors,
            SpawnAgentHandler::new(
                host.clone(),
                SpawnAgentToolOptions {
                    available_models: config.available_models.clone(),
                    agent_type_description: agent_type_description.to_string(),
                    hide_agent_type_model_reasoning: config.hide_spawn_agent_metadata,
                    include_usage_hint: config.spawn_agent_usage_hint,
                    usage_hint_text: config.spawn_agent_usage_hint_text.clone(),
                    max_concurrent_threads_per_session: config.max_concurrent_threads_per_session,
                },
            ),
            exposure,
        );
        push_exposed_domain_tool::<Host, _>(
            &mut executors,
            FollowupTaskHandler::new(host.clone()),
            exposure,
        );
        push_exposed_domain_tool::<Host, _>(
            &mut executors,
            WaitAgentHandler::new(host.clone()),
            exposure,
        );
        push_exposed_domain_tool::<Host, _>(
            &mut executors,
            CloseAgentHandler::new(host.clone()),
            exposure,
        );
        push_exposed_domain_tool::<Host, _>(
            &mut executors,
            ListAgentsHandler::new(host.clone()),
            exposure,
        );
    }

    if config.agent_jobs_tools {
        push_domain_tool::<Host, _>(&mut executors, SpawnAgentsOnCsvHandler::new(host.clone()));
        if config.agent_jobs_worker_tools {
            push_domain_tool::<Host, _>(
                &mut executors,
                ReportAgentJobResultHandler::new(host.clone()),
            );
        }
    }

    if let Some(mcp_tools) = params.mcp_tools {
        for tool in mcp_tools {
            push_domain_tool::<Host, _>(
                &mut executors,
                McpHandler::new(host.clone(), tool.clone()),
            );
        }
    }
    if let Some(deferred_mcp_tools) = params.deferred_mcp_tools {
        for tool in deferred_mcp_tools {
            push_domain_tool::<Host, _>(
                &mut executors,
                McpHandler::with_exposure(host.clone(), tool.clone(), ToolExposure::Deferred),
            );
        }
    }

    for dynamic_tool in params.dynamic_tools {
        let Some(handler) = DynamicToolHandler::new(host.clone(), dynamic_tool) else {
            tracing::error!(
                "Failed to convert dynamic tool {:?} to OpenAI tool",
                dynamic_tool.name
            );
            continue;
        };

        push_domain_tool::<Host, _>(&mut executors, handler);
    }

    append_extension_tool_executors::<Host>(
        config,
        params.extension_tool_executors,
        &mut executors,
    );

    executors
}

pub fn build_tool_registry_builder_from_executors<Host>(
    config: &ToolsConfig,
    executors: Vec<Arc<DomainRegisteredTool<Host>>>,
    hosted_specs: Vec<ToolSpec>,
    host: &Host,
) -> ToolRegistryBuilder<DomainInvocation<Host>, <Host as ApplyPatchHandlerHost>::DiffContext>
where
    Host: RuntimeToolAssemblyHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
{
    let mut builder = ToolRegistryBuilder::new();
    let plan = plan_tool_registry_entries(config, executors, hosted_specs);
    let codex_tool_planning::PlannedToolRegistry {
        entries,
        model_visible_specs,
        code_mode_nested_tool_specs,
        deferred_search_infos,
        deferred_tools_available,
    } = plan;

    for executor in build_code_mode_executors(
        config,
        code_mode_nested_tool_specs,
        config.search_tool && deferred_tools_available,
        host,
    ) {
        builder
            .register_tool(executor)
            .expect("code-mode tool names should be unique");
    }

    for spec in model_visible_specs {
        builder.push_spec(spec);
    }

    for executor in entries {
        builder
            .register_tool_without_spec(executor)
            .expect("planned tool registry entries should be unique");
    }

    if config.search_tool && config.namespace_tools && !deferred_search_infos.is_empty() {
        builder
            .register_tool(codex_tool_runtime_api::registered_tool(Arc::new(
                ToolSearchHandler::<DomainInvocation<Host>>::new(deferred_search_infos),
            )))
            .expect("tool_search should be unique");
    }

    builder
}

pub fn build_tool_router<Host>(
    config: &ToolsConfig,
    host: &Host,
    params: ToolRuntimeBuildParams<'_>,
) -> ToolRouter<
    ToolRegistry<DomainInvocation<Host>, <Host as ApplyPatchHandlerHost>::DiffContext>,
    <Host as ApplyPatchHandlerHost>::DiffContext,
>
where
    Host: RuntimeToolAssemblyHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
{
    let executors = collect_tool_executors(config, host, params);
    let builder = build_tool_registry_builder_from_executors(
        config,
        executors,
        hosted_model_tool_specs(config),
        host,
    );
    let (specs, registry) = builder.build();
    ToolRouter::new(config.code_mode_only_enabled, specs, registry)
}

fn build_code_mode_executors<Host>(
    config: &ToolsConfig,
    code_mode_nested_tool_specs: Vec<ToolSpec>,
    deferred_tools_available: bool,
    host: &Host,
) -> Vec<Arc<DomainRegisteredTool<Host>>>
where
    Host: RuntimeToolAssemblyHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
{
    if !config.code_mode_enabled {
        return vec![];
    }

    let CodeModeExecPlan {
        enabled_tools,
        namespace_descriptions,
    } = code_mode_exec_plan_for_specs(&code_mode_nested_tool_specs);

    vec![
        boxed_domain_tool::<Host, _>(CodeModeExecuteHandler::new(
            host.clone(),
            create_code_mode_tool(
                &enabled_tools,
                &namespace_descriptions,
                config.code_mode_only_enabled,
                deferred_tools_available,
            ),
            code_mode_nested_tool_specs,
        )),
        boxed_domain_tool::<Host, _>(CodeModeWaitHandler::new(host.clone())),
    ]
}

fn append_extension_tool_executors<Host>(
    config: &ToolsConfig,
    executors: &[Arc<dyn ExtensionToolExecutor>],
    registered_executors: &mut Vec<Arc<DomainRegisteredTool<Host>>>,
) where
    Host: RuntimeToolAssemblyHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
{
    if executors.is_empty() {
        return;
    }

    let mut reserved_tool_names = registered_executors
        .iter()
        .map(|executor| executor.tool_name())
        .collect::<HashSet<_>>();
    if config.code_mode_enabled {
        reserved_tool_names.insert(ToolName::plain(codex_code_mode_api::PUBLIC_TOOL_NAME));
        reserved_tool_names.insert(ToolName::plain(codex_code_mode_api::WAIT_TOOL_NAME));
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
        registered_executors.push(boxed_domain_tool::<Host, _>(ExtensionToolHandler::<
            DomainInvocation<Host>,
        >::new(executor)));
    }
}

fn push_domain_tool<Host, T>(executors: &mut Vec<Arc<DomainRegisteredTool<Host>>>, handler: T)
where
    Host: ToolDomainHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
    T: ToolHandler<DomainInvocation<Host>, <Host as ApplyPatchHandlerHost>::DiffContext> + 'static,
{
    executors.push(boxed_domain_tool::<Host, T>(handler));
}

fn push_exposed_domain_tool<Host, T>(
    executors: &mut Vec<Arc<DomainRegisteredTool<Host>>>,
    handler: T,
    exposure: ToolExposure,
) where
    Host: ToolDomainHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
    T: ToolHandler<DomainInvocation<Host>, <Host as ApplyPatchHandlerHost>::DiffContext> + 'static,
{
    executors.push(codex_tool_runtime_api::override_tool_exposure(
        boxed_domain_tool::<Host, T>(handler),
        exposure,
    ));
}

fn agent_type_description<'a>(
    config: &'a ToolsConfig,
    default_agent_type_description: &'a str,
) -> &'a str {
    if config.agent_type_description.is_empty() {
        default_agent_type_description
    } else {
        &config.agent_type_description
    }
}

fn boxed_domain_tool<Host, T>(handler: T) -> Arc<DomainRegisteredTool<Host>>
where
    Host: ToolDomainHost,
    ApplyPatchActiveNetworkApproval<Host>: Send,
    ApplyPatchDeferredNetworkApproval<Host>: Send,
    ShellActiveNetworkApproval<Host>: Send,
    ShellDeferredNetworkApproval<Host>: Send,
    T: ToolHandler<DomainInvocation<Host>, <Host as ApplyPatchHandlerHost>::DiffContext> + 'static,
{
    codex_tool_runtime_api::registered_tool(Arc::new(handler))
}

#[cfg(test)]
#[path = "spec_plan_tests.rs"]
mod spec_plan_tests;
