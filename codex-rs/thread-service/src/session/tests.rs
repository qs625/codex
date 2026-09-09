use super::turn_context::TurnEnvironment;
use super::*;
use crate::config::CONFIG_TOML_FILE;
use crate::config::ConfigBuilder;
use crate::config::test_config;
use crate::runtime_shell_model::default_user_shell;
use crate::test_support::models_manager_with_provider;
use approval_service::network::NetworkApprovalService;
use codex_approval_service_api::SessionNetworkApprovalApi;
use codex_context_manager::ContextualUserFragment;
use config_service::ConfigLayerStack;
use config_service::ConfigLayerStackOrdering;
use config_service::LoaderOverrides;
use config_service::NetworkConstraints;
use config_service::NetworkDomainPermissionToml;
use config_service::NetworkDomainPermissionsToml;
use config_service::RequirementSource;
use config_service::Sourced;
use config_service::loader::project_trust_key;
use config_service::types::ToolSuggestDisabledTool;
use model_service_api::SharedApiRuntimeFactory;
use plugin_service::PluginsManager;
use rollout_api::TurnAborted;
use skill_service::SkillService;
use skill_service_api::SkillMetadata;
use skill_service_api::render::SkillMetadataBudget;
use skill_service_api::render::SkillRenderSideEffects;
use thread_service_api::TurnDiffTracker;
use tool_service_api::FunctionCallError;
use tool_service_api::ToolCallSource;
use tool_service_api::ToolPayload;

use crate::tool_output_utils::format_exec_output_str;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_login::CodexAuth;
use codex_otel::SessionTelemetry;
use goal_service::GoalService;
use goal_service_api::GoalServiceApi;
use hooks::Hooks;
use model_service::ModelService;
use model_service::ModelServiceRuntimeDeps;
use model_service::bundled_models_response;
use model_service::model_info;
use model_service::test_support::construct_model_info_offline_for_tests;
use model_service::test_support::get_model_offline_for_tests;
use model_service_api::CreateModelClientRequest;
use model_service_api::ModelCatalogRefresh;
use model_service_api::ModelProviderInfo;
use model_service_api::ModelSelectionPolicy;
use model_service_api::SharedModelServiceApi;
use protocol::AgentPath;
use protocol::SessionId;
use protocol::ThreadId;
use protocol::config_types::ServiceTier;
use protocol::config_types::TrustLevel;
use protocol::exec_output::ExecToolCallOutput;
use protocol::mcp::RequestId;
use protocol::models::ActivePermissionProfile;
use protocol::models::BUILT_IN_PERMISSION_PROFILE_WORKSPACE;
use protocol::models::FileSystemPermissions;
use protocol::models::FunctionCallOutputBody;
use protocol::models::FunctionCallOutputPayload;
use protocol::models::PermissionProfile;
use protocol::models::SandboxEnforcement;
use protocol::permissions::FileSystemAccessMode;
use protocol::permissions::FileSystemPath;
use protocol::permissions::FileSystemSandboxEntry;
use protocol::permissions::FileSystemSandboxPolicy;
use protocol::permissions::FileSystemSpecialPath;
use protocol::protocol::SandboxPolicy;
use protocol::protocol::TurnEnvironmentSelection;
use protocol::request_permissions::PermissionGrantScope;
use protocol::request_permissions::RequestPermissionProfile;
use state_api::ExternalGoalPreviousStatus;
use state_api::ExternalGoalSet;
use thread_store::LiveThread;
use tracing::Span;

use crate::PendingInputItem;
use crate::state::ActiveTurn;
use crate::state::TaskKind;
use crate::tasks::SessionTask;
use crate::tasks::SessionTaskContext;
use crate::tasks::UserShellCommandMode;
use crate::tasks::execute_user_shell_command;
use crate::test_support::DisabledToolServiceForTests;
use codex_auth_types::TelemetryAuthMode;
use codex_network_proxy_api::NetworkDecision;
use codex_network_proxy_api::NetworkPolicyDecider;
use codex_network_proxy_api::NetworkProxyConfig;
use codex_otel::MetricsClient;
use codex_otel::MetricsConfig;
use command_service_api::ExecApprovalRequirement;
use config_service::ProjectConfig;
use config_service::config_toml::ConfigToml;
use config_service::types::OAuthCredentialsStoreMode;
use core_test_support::PathBufExt;
use core_test_support::PathExt;
use core_test_support::context_snapshot;
use core_test_support::context_snapshot::ContextSnapshotOptions;
use core_test_support::context_snapshot::ContextSnapshotRenderMode;
use core_test_support::responses::ev_assistant_message;
use core_test_support::responses::ev_completed;
use core_test_support::responses::ev_completed_with_tokens;
use core_test_support::responses::ev_function_call;
use core_test_support::responses::ev_response_created;
use core_test_support::responses::mount_sse_once;
use core_test_support::responses::mount_sse_sequence;
use core_test_support::responses::sse;
use core_test_support::responses::start_mock_server;
use core_test_support::test_codex::test_codex;
use core_test_support::test_path_buf;
use core_test_support::tracing::install_test_tracing;
use core_test_support::wait_for_event;
use core_test_support::wait_for_event_match;
use mcp_types::ElicitationAction;
use mcp_types::ElicitationResponse;
use mcp_types::McpElicitationSchema;
use mcp_types::McpServerElicitationRequest;
use mcp_types::McpServerElicitationRequestParams;
use metrics_api::THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC;
use metrics_api::THREAD_SKILLS_ENABLED_TOTAL_METRIC;
use metrics_api::THREAD_SKILLS_KEPT_TOTAL_METRIC;
use metrics_api::THREAD_SKILLS_TRUNCATED_METRIC;
use opentelemetry::trace::TraceContextExt;
use opentelemetry::trace::TraceId;
use opentelemetry_sdk::metrics::InMemoryMetricExporter;
use opentelemetry_sdk::metrics::data::AggregatedMetrics;
use opentelemetry_sdk::metrics::data::Metric;
use opentelemetry_sdk::metrics::data::MetricData;
use opentelemetry_sdk::metrics::data::ResourceMetrics;
use permissions_service_api::Decision;
use permissions_service_api::NetworkRuleProtocol;
use permissions_service_api::Policy;
use protocol::config_types::CollaborationMode;
use protocol::config_types::ModeKind;
use protocol::config_types::Settings;
use protocol::event_command::EventCommandEvent;
use protocol::event_command::EventCommandEventKind;
use protocol::event_driven_tool::EventDrivenToolTrigger;
use protocol::items::HookPromptFragment;
use protocol::items::build_hook_prompt_message;
use protocol::models::BaseInstructions;
use protocol::models::ContentItem;
use protocol::models::ResponseInputItem;
use protocol::models::ResponseItem;
use protocol::protocol::AskForApproval;
use protocol::protocol::CompactedItem;
use protocol::protocol::ConversationAudioParams;
use protocol::protocol::CreditsSnapshot;
use protocol::protocol::GranularApprovalConfig;
use protocol::protocol::InitialHistory;
use protocol::protocol::InterAgentCommunication;
use protocol::protocol::InterAgentOperation;
use protocol::protocol::RateLimitSnapshot;
use protocol::protocol::RateLimitWindow;
use protocol::protocol::RealtimeAudioFrame;
use protocol::protocol::RealtimeConversationListVoicesResponseEvent;
use protocol::protocol::RealtimeVoice;
use protocol::protocol::RealtimeVoicesList;
use protocol::protocol::ResumedHistory;
use protocol::protocol::RolloutItem;
use protocol::protocol::SkillScope;
use protocol::protocol::Submission;
use protocol::protocol::ThreadRolledBackEvent;
use protocol::protocol::TokenCountEvent;
use protocol::protocol::TokenUsage;
use protocol::protocol::TokenUsageInfo;
use protocol::protocol::TurnAbortedEvent;
use protocol::protocol::TurnCompleteEvent;
use protocol::protocol::TurnStartedEvent;
use protocol::protocol::UserMessageEvent;
use protocol::protocol::W3cTraceContext;
use protocol::request_user_input::RequestUserInputAnswer;
use protocol::request_user_input::RequestUserInputResponse;
use rollout::RolloutRecorder;
use std::path::Path;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tokio::time::timeout;
use tracing_opentelemetry::OpenTelemetrySpanExt;

use pretty_assertions::assert_eq;
use protocol::mcp::CallToolResult as McpCallToolResult;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration as StdDuration;

mod guardian_tests;

fn permission_profile_for_sandbox_policy(sandbox_policy: &SandboxPolicy) -> PermissionProfile {
    PermissionProfile::from_legacy_sandbox_policy(sandbox_policy)
}

struct InstructionsTestCase {
    slug: &'static str,
    expects_apply_patch_description: bool,
}

fn user_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn assistant_message(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
    }
}

fn test_session_telemetry_without_metadata() -> SessionTelemetry {
    let exporter = InMemoryMetricExporter::default();
    let metrics = MetricsClient::new(
        MetricsConfig::in_memory("test", "codex-core", env!("CARGO_PKG_VERSION"), exporter)
            .with_runtime_reader(),
    )
    .expect("in-memory metrics client");
    SessionTelemetry::new(
        ThreadId::new(),
        "gpt-5.4",
        "gpt-5.4",
        /*account_id*/ None,
        /*account_email*/ None,
        /*auth_mode*/ None,
        "test_originator".to_string(),
        /*log_user_prompts*/ false,
        "tty".to_string(),
        SessionSource::Cli,
    )
    .with_metrics_without_metadata_tags(metrics)
}

fn find_metric<'a>(resource_metrics: &'a ResourceMetrics, name: &str) -> &'a Metric {
    for scope_metrics in resource_metrics.scope_metrics() {
        for metric in scope_metrics.metrics() {
            if metric.name() == name {
                return metric;
            }
        }
    }
    panic!("metric {name} missing");
}

fn histogram_sum(resource_metrics: &ResourceMetrics, name: &str) -> u64 {
    let metric = find_metric(resource_metrics, name);
    match metric.data() {
        AggregatedMetrics::F64(data) => match data {
            MetricData::Histogram(histogram) => {
                let points: Vec<_> = histogram.data_points().collect();
                assert_eq!(points.len(), 1);
                points[0].sum().round() as u64
            }
            _ => panic!("unexpected histogram aggregation"),
        },
        _ => panic!("unexpected metric data type"),
    }
}

include!("tests/turn_flow.rs");
include!("tests/runtime_features.rs");
include!("tests/permissions_and_tools.rs");
include!("tests/context_and_history.rs");
