use crate::shell::Shell;
use crate::shell::ShellType;
use crate::tools::registry::RegisteredTool;
use crate::tools::spec_plan::collect_tool_executors;
use crate::tools::spec_plan_types::ToolRegistryBuildParams;
use codex_extension_api::ExtensionToolExecutor;
use codex_mcp_tool_types::ToolInfo;
use codex_protocol::dynamic_tools::DynamicToolSpec;
use codex_tool_config::ToolUserShellType;
use codex_tool_config::ToolsConfig;
use codex_tool_planning::DiscoverableTool;
use codex_tool_planning::hosted_model_tool_specs;
use std::sync::Arc;

pub(crate) fn tool_user_shell_type(user_shell: &Shell) -> ToolUserShellType {
    match user_shell.shell_type {
        ShellType::Zsh => ToolUserShellType::Zsh,
        ShellType::Bash => ToolUserShellType::Bash,
        ShellType::PowerShell => ToolUserShellType::PowerShell,
        ShellType::Sh => ToolUserShellType::Sh,
        ShellType::Cmd => ToolUserShellType::Cmd,
    }
}

pub(crate) struct ToolRouterParts {
    pub(crate) executors: Vec<Arc<dyn RegisteredTool>>,
    pub(crate) hosted_specs: Vec<codex_tool_planning::ToolSpec>,
}

pub(crate) fn collect_tool_router_parts(
    config: &ToolsConfig,
    mcp_tools: Option<Vec<ToolInfo>>,
    deferred_mcp_tools: Option<Vec<ToolInfo>>,
    discoverable_tools: Option<Vec<DiscoverableTool>>,
    extension_tool_executors: &[Arc<dyn ExtensionToolExecutor>],
    dynamic_tools: &[DynamicToolSpec],
) -> ToolRouterParts {
    let default_agent_type_description =
        codex_agent_roles::spawn_tool_spec::build(&std::collections::BTreeMap::new());
    let executors = collect_tool_executors(
        config,
        ToolRegistryBuildParams {
            mcp_tools: mcp_tools.as_deref(),
            deferred_mcp_tools: deferred_mcp_tools.as_deref(),
            discoverable_tools: discoverable_tools.as_deref(),
            extension_tool_executors,
            dynamic_tools,
            default_agent_type_description: &default_agent_type_description,
        },
    );
    ToolRouterParts {
        executors,
        hosted_specs: hosted_model_tool_specs(config),
    }
}

#[cfg(test)]
#[path = "spec_tests.rs"]
mod tests;
