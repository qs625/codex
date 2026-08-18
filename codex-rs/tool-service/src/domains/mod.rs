pub(crate) mod agent;
pub(crate) mod apply_patch;
pub(crate) mod code_mode;
pub(crate) mod command_interaction;
pub(crate) mod discovery;
pub(crate) mod exec_command;
pub(crate) mod extension;
pub(crate) mod function;
pub(crate) mod goal;
pub(crate) mod mcp;
pub(crate) mod runtime_state;
pub(crate) mod workflow;

use crate::planning::merge_tool_specs_into_namespaces;
use tool_service_api::ErasedToolArgumentDiffConsumer;
use tool_service_api::ToolCall;
use tool_service_api::ToolName;
use tool_service_api::ToolSpec;

use crate::ToolService;
use crate::context::TypedToolSpecRequest;

/// `ToolService` 内部的按-domain 直分发骨架。
///
/// 目标边界：
/// - `ToolService` 对外仍然只有四个固定入口。
/// - 各个 domain 在这里按 tool name 注册自己的 specs / parallel / dispatch。
/// - 不再引入新的通用 handler trait；每个 domain 只是普通模块函数。
/// - 未迁移 domain 临时回落到 legacy router，后续逐个替换。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolDomain {
    Agent,
    ApplyPatch,
    CodeMode,
    CommandInteraction,
    Discovery,
    ExecCommand,
    Extension,
    Function,
    Goal,
    Mcp,
    RuntimeState,
    Workflow,
    Legacy,
}

pub(crate) enum DirectToolRoute {
    Workflow,
    Legacy,
}

pub(crate) fn classify_tool_name(
    request: &TypedToolSpecRequest<'_>,
    tool_name: &ToolName,
) -> ToolDomain {
    if workflow::owns_tool_name(tool_name) {
        ToolDomain::Workflow
    } else if goal::owns_tool_name(request, tool_name) {
        ToolDomain::Goal
    } else if mcp::owns_tool_name(request, tool_name) {
        ToolDomain::Mcp
    } else if agent::owns_tool_name(request, tool_name) {
        ToolDomain::Agent
    } else if exec_command::owns_tool_name(request, tool_name) {
        ToolDomain::ExecCommand
    } else if runtime_state::owns_tool_name(request, tool_name) {
        ToolDomain::RuntimeState
    } else if command_interaction::owns_tool_name(request, tool_name) {
        ToolDomain::CommandInteraction
    } else if apply_patch::owns_tool_name(request, tool_name) {
        ToolDomain::ApplyPatch
    } else if code_mode::owns_tool_name(request, tool_name) {
        ToolDomain::CodeMode
    } else if discovery::owns_tool_name(request, tool_name) {
        ToolDomain::Discovery
    } else if function::owns_tool_name(request, tool_name) {
        ToolDomain::Function
    } else if extension::owns_tool_name(request, tool_name) {
        ToolDomain::Extension
    } else {
        ToolDomain::Legacy
    }
}

pub(crate) fn route_for_tool_name(
    request: &TypedToolSpecRequest<'_>,
    tool_name: &ToolName,
) -> DirectToolRoute {
    if matches!(classify_tool_name(request, tool_name), ToolDomain::Workflow) {
        DirectToolRoute::Workflow
    } else {
        DirectToolRoute::Legacy
    }
}

pub(crate) fn model_visible_specs(
    _service: &ToolService,
    request: TypedToolSpecRequest<'_>,
) -> Vec<ToolSpec> {
    let mut specs = Vec::new();
    specs.extend(agent::specs(&request));
    specs.extend(apply_patch::specs(&request));
    specs.extend(command_interaction::specs(&request));
    specs.extend(discovery::specs(&request));
    specs.extend(exec_command::specs(&request));
    specs.extend(runtime_state::specs(&request));
    specs.extend(extension::specs(&request));
    specs.extend(function::specs(&request));
    specs.extend(goal::specs(&request));
    specs.extend(mcp::specs(&request));
    specs.extend(workflow::specs());

    let mut merged_specs = merge_tool_specs_into_namespaces(specs);
    let code_mode_specs = code_mode::specs(&request, &merged_specs);
    merged_specs.extend(code_mode_specs);
    merged_specs
}

pub(crate) fn create_diff_consumer(
    _service: &ToolService,
    request: TypedToolSpecRequest<'_>,
    tool_name: &ToolName,
) -> Option<Box<dyn ErasedToolArgumentDiffConsumer>> {
    match classify_tool_name(&request, tool_name) {
        ToolDomain::Agent => agent::create_diff_consumer(&request, tool_name),
        ToolDomain::ApplyPatch => apply_patch::create_diff_consumer(&request, tool_name),
        ToolDomain::CodeMode => code_mode::create_diff_consumer(&request, tool_name),
        ToolDomain::CommandInteraction => {
            command_interaction::create_diff_consumer(&request, tool_name)
        }
        ToolDomain::Discovery => discovery::create_diff_consumer(&request, tool_name),
        ToolDomain::ExecCommand => exec_command::create_diff_consumer(&request, tool_name),
        ToolDomain::Extension => extension::create_diff_consumer(&request, tool_name),
        ToolDomain::Function => function::create_diff_consumer(&request, tool_name),
        ToolDomain::Goal => goal::create_diff_consumer(&request, tool_name),
        ToolDomain::Mcp => mcp::create_diff_consumer(&request, tool_name),
        ToolDomain::RuntimeState => runtime_state::create_diff_consumer(&request, tool_name),
        ToolDomain::Workflow => workflow::create_diff_consumer(tool_name),
        ToolDomain::Legacy => None,
    }
}

pub(crate) fn supports_parallel(
    _service: &ToolService,
    request: TypedToolSpecRequest<'_>,
    call: &ToolCall,
) -> bool {
    match classify_tool_name(&request, &call.tool_name) {
        ToolDomain::Agent => agent::supports_parallel(&request, call),
        ToolDomain::ApplyPatch => apply_patch::supports_parallel(&request, call),
        ToolDomain::CodeMode => code_mode::supports_parallel(&request, call),
        ToolDomain::CommandInteraction => command_interaction::supports_parallel(&request, call),
        ToolDomain::Discovery => discovery::supports_parallel(&request, call),
        ToolDomain::ExecCommand => exec_command::supports_parallel(&request, call),
        ToolDomain::Extension => extension::supports_parallel(&request, call),
        ToolDomain::Function => function::supports_parallel(&request, call),
        ToolDomain::Goal => goal::supports_parallel(&request, call),
        ToolDomain::Mcp => mcp::supports_parallel(&request, call),
        ToolDomain::RuntimeState => runtime_state::supports_parallel(&request, call),
        ToolDomain::Workflow => workflow::supports_parallel(call),
        ToolDomain::Legacy => false,
    }
}
