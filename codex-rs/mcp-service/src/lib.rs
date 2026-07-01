use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_channel::Sender;
use codex_api_auth::auth_provider_from_auth_snapshot;
use codex_auth_types::RequestAuthSnapshot;
use codex_exec_server_api::ExecEnvironment;
use codex_mcp_types::CodexAppsAuthContext;
use codex_mcp_types::CodexAppsToolsCacheKey;
use codex_mcp_types::EffectiveMcpServer;
use codex_mcp_types::ElicitationReviewerHandle;
use codex_mcp_types::McpAuthStatusEntry;
use codex_mcp_types::McpClientElicitationSupport;
use codex_mcp_types::ToolPluginProvenance;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::AskForApproval;
use codex_protocol::protocol::Event;
use mcp_service_api::McpRuntimeEnvironment;
use mcp_service_api::McpRuntimeEnvironmentParams;
use mcp_service_api::McpConnectionRuntimeFactory;
use mcp_service_api::McpConnectionRuntimeStart;
use mcp_service_api::McpConnectionRuntimeStartRequest;
use mcp_service_api::SharedMcpAuthHeaderProvider;
use mcp_service_api::StaticMcpAuthHeaderProvider;

mod app_tools;
mod connectors;
mod elicitation_review;
#[cfg(test)]
mod elicitation_review_tests;
mod openai_file;
mod service;
mod skill_dependencies;
mod tool_call_approval;
mod tool_call_display;
mod tool_call_execution;
mod tool_call_flow;
mod tool_call_metadata;
mod tool_exposure;

pub use app_tools::AppToolPolicy;
#[cfg(any(test, feature = "test-support"))]
pub use app_tools::app_is_enabled;
pub use app_tools::app_tool_policy;
#[cfg(any(test, feature = "test-support"))]
pub use app_tools::app_tool_policy_from_apps_config;
#[cfg(any(test, feature = "test-support"))]
pub use app_tools::apply_requirements_apps_constraints;
pub use app_tools::codex_app_tool_is_enabled;
#[cfg(any(test, feature = "test-support"))]
pub use app_tools::managed_app_tool_approval;
pub use app_tools::with_app_enabled_state;
pub use connectors::AccessibleConnectorsStatus;
pub use connectors::accessible_connectors_from_mcp_tools;
pub use connectors::list_accessible_and_enabled_connectors_from_manager;
pub use connectors::list_accessible_connectors_from_mcp_tools;
pub use connectors::list_accessible_connectors_from_mcp_tools_with_environment_provider;
pub use connectors::list_accessible_connectors_from_mcp_tools_with_options;
pub use connectors::list_accessible_connectors_from_mcp_tools_with_options_and_status;
pub use connectors::list_cached_accessible_connectors_from_mcp_tools;
pub use connectors::list_tool_suggest_discoverable_tools_with_auth;
pub use connectors::refresh_accessible_connectors_cache_from_mcp_tools;
pub use connectors::with_app_plugin_sources;
pub use elicitation_review::GuardianElicitationReview;
pub use elicitation_review::guardian_elicitation_review_request;
pub use elicitation_review::mcp_elicitation_request_id;
pub use elicitation_review::mcp_elicitation_response_from_guardian_decision_parts;
pub use openai_file::OpenAiFilePathResolver;
pub use openai_file::rewrite_mcp_tool_arguments_for_openai_files;
pub use service::McpService;
pub use skill_dependencies::McpSkillDependencyHost;
pub use skill_dependencies::McpSkillDependencyTurnContext;
pub use skill_dependencies::maybe_install_mcp_dependencies;
pub use skill_dependencies::maybe_prompt_and_install_mcp_dependencies;
pub use tool_call_approval::McpToolApprovalHookDecision;
pub use tool_call_approval::McpToolApprovalMonitorOutcome;
pub use tool_call_approval::McpToolApprovalPersistenceHost;
pub use tool_call_approval::McpToolApprovalRequirement;
pub use tool_call_approval::McpToolApprovalRequirementContext;
pub use tool_call_approval::McpToolApprovalReviewContext;
pub use tool_call_approval::McpToolApprovalReviewHost;
pub use tool_call_approval::apply_mcp_tool_approval_decision;
pub use tool_call_approval::arc_monitor_interrupt_message;
pub use tool_call_approval::build_guardian_mcp_tool_review_request;
pub use tool_call_approval::build_mcp_tool_call_request_meta;
pub use tool_call_approval::custom_mcp_tool_approval_mode;
pub use tool_call_approval::maybe_persist_mcp_tool_approval;
pub use tool_call_approval::maybe_request_mcp_tool_approval;
pub use tool_call_approval::mcp_tool_approval_arc_monitor_action;
pub use tool_call_approval::mcp_tool_approval_callsite_mode;
pub use tool_call_approval::mcp_tool_approval_decision_from_guardian;
pub use tool_call_approval::mcp_tool_approval_requirement;
pub use tool_call_approval::persist_codex_app_tool_approval;
pub use tool_call_approval::persist_non_app_mcp_tool_approval;
pub use tool_call_display::MCP_CALL_COUNT_METRIC;
pub use tool_call_display::MCP_CALL_DURATION_METRIC;
pub use tool_call_display::McpToolCallSpanFields;
pub use tool_call_display::build_mcp_tool_call_completed_item;
pub use tool_call_display::build_mcp_tool_call_started_item;
pub use tool_call_display::mcp_call_metric_tags;
pub use tool_call_display::mcp_tool_call_span;
pub use tool_call_display::record_mcp_result_span_telemetry;
pub use tool_call_execution::ApprovedMcpToolCallOutcome;
pub use tool_call_execution::CodexAppsAuthElicitationContext;
pub use tool_call_execution::CodexAppsAuthElicitationHost;
pub use tool_call_execution::CodexAppsAuthElicitationRequest;
pub use tool_call_execution::McpApprovedToolCallLifecycleContext;
pub use tool_call_execution::McpApprovedToolCallLifecycleHost;
pub use tool_call_execution::McpToolExecutionContext;
pub use tool_call_execution::McpToolExecutionHost;
pub use tool_call_execution::build_codex_apps_auth_elicitation_request;
pub use tool_call_execution::execute_mcp_tool_call;
pub use tool_call_execution::handle_approved_mcp_tool_call;
pub use tool_call_execution::insert_sandbox_state_request_meta;
pub use tool_call_execution::maybe_request_codex_apps_auth_elicitation;
pub use tool_call_flow::McpToolCallContext;
pub use tool_call_flow::McpToolCallHost;
pub use tool_call_flow::McpToolCallOutcome;
pub use tool_call_flow::handle_mcp_tool_call;
pub use mcp_service_api::McpAppUsageMetadata;
pub use tool_call_metadata::McpToolMetadataLookupHost;
pub use tool_call_metadata::build_mcp_tool_approval_metadata;
pub use tool_call_metadata::connector_description_for_tool;
pub use tool_call_metadata::find_mcp_tool_info;
pub use tool_call_metadata::lookup_mcp_app_usage_metadata;
pub use tool_call_metadata::lookup_mcp_tool_metadata;
pub use tool_exposure::DIRECT_MCP_TOOL_EXPOSURE_THRESHOLD;
pub use tool_exposure::McpToolExposure;
pub use tool_exposure::build_mcp_tool_exposure;

pub struct McpConnectionStartParams {
    pub mcp_servers: HashMap<String, EffectiveMcpServer>,
    pub store_mode: codex_config_types::OAuthCredentialsStoreMode,
    pub auth_entries: HashMap<String, McpAuthStatusEntry>,
    pub approval_policy: codex_config_types::Constrained<AskForApproval>,
    pub submit_id: String,
    pub tx_event: Sender<Event>,
    pub initial_permission_profile: PermissionProfile,
    pub runtime_environment: McpRuntimeEnvironment,
    pub codex_home: PathBuf,
    pub codex_apps_tools_cache_key: CodexAppsToolsCacheKey,
    pub host_owned_codex_apps_enabled: bool,
    pub client_elicitation_support: McpClientElicitationSupport,
    pub tool_plugin_provenance: ToolPluginProvenance,
    pub codex_apps_auth_provider: Option<SharedMcpAuthHeaderProvider>,
    pub elicitation_reviewer: Option<ElicitationReviewerHandle>,
}

pub fn codex_apps_auth_provider(
    auth: Option<&RequestAuthSnapshot>,
) -> Option<SharedMcpAuthHeaderProvider> {
    auth.filter(|auth| auth.uses_codex_backend())
        .map(auth_provider_from_auth_snapshot)
        .map(|auth_provider| StaticMcpAuthHeaderProvider::shared(auth_provider.to_auth_headers()))
}

pub fn codex_apps_auth_context(auth: Option<&RequestAuthSnapshot>) -> Option<CodexAppsAuthContext> {
    auth.map(|auth| CodexAppsAuthContext {
        uses_codex_backend: auth.uses_codex_backend(),
        account_id: auth.account_id().map(ToOwned::to_owned),
        chatgpt_user_id: auth.chatgpt_user_id().map(ToOwned::to_owned),
        is_workspace_account: auth.is_workspace_account(),
    })
}

pub fn mcp_runtime_environment(
    environment: Arc<dyn ExecEnvironment>,
    local_environment: Arc<dyn ExecEnvironment>,
    fallback_cwd: PathBuf,
) -> McpRuntimeEnvironment {
    let local_http_client = local_environment.get_http_client();
    McpRuntimeEnvironment::new(McpRuntimeEnvironmentParams {
        remote_available: environment.is_remote(),
        remote_exec_backend: environment.get_exec_backend(),
        local_http_client,
        remote_http_client: environment.get_http_client(),
        fallback_cwd,
    })
}

pub async fn start_mcp_connection_runtime(
    factory: &dyn McpConnectionRuntimeFactory,
    params: McpConnectionStartParams,
) -> McpConnectionRuntimeStart {
    let McpConnectionStartParams {
        mcp_servers,
        store_mode,
        auth_entries,
        approval_policy,
        submit_id,
        tx_event,
        initial_permission_profile,
        runtime_environment,
        codex_home,
        codex_apps_tools_cache_key,
        host_owned_codex_apps_enabled,
        client_elicitation_support,
        tool_plugin_provenance,
        codex_apps_auth_provider,
        elicitation_reviewer,
    } = params;
    factory
        .start(McpConnectionRuntimeStartRequest {
            mcp_servers,
            store_mode,
            auth_entries,
            approval_policy,
            submit_id,
            tx_event,
            initial_permission_profile,
            runtime_environment,
            codex_home,
            codex_apps_tools_cache_key,
            host_owned_codex_apps_enabled,
            client_elicitation_support,
            tool_plugin_provenance,
            codex_apps_auth_provider,
            elicitation_reviewer,
        })
        .await
}
