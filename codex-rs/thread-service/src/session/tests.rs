use super::turn_context::TurnEnvironment;
use super::*;
use crate::config::CONFIG_TOML_FILE;
use crate::config::ConfigBuilder;
use crate::config::test_config;
use crate::runtime_shell_model::default_user_shell;
use crate::test_support::models_manager_with_provider;
use approval_service::network::NetworkApprovalService;
use codex_approval_service_api::SessionNetworkApprovalApi;
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
use codex_context_manager::ContextualUserFragment;
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
use config_service::config_toml::ConfigToml;
use config_service::types::OAuthCredentialsStoreMode;
use config_service::ProjectConfig;
use permissions_service_api::Decision;
use permissions_service_api::NetworkRuleProtocol;
use permissions_service_api::Policy;
use codex_network_proxy_api::NetworkDecision;
use codex_network_proxy_api::NetworkPolicyDecider;
use codex_network_proxy_api::NetworkProxyConfig;
use codex_otel::MetricsClient;
use codex_otel::MetricsConfig;
use command_service_api::ExecApprovalRequirement;
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

#[tokio::test]
async fn regular_turn_emits_turn_started_without_waiting_for_startup_prewarm() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let model_client_api = Arc::clone(&sess.services.model_client_api);
    let (_tx, startup_prewarm_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = startup_prewarm_rx.await;
        model_client_api
            .create_turn_client()
            .await
            .map_err(|err| protocol::error::CodexErr::Fatal(err.to_string()))
    });

    sess.set_session_startup_prewarm(
        crate::session_startup_prewarm::SessionStartupPrewarmHandle::new(
            handle,
            std::time::Instant::now(),
            crate::client::WEBSOCKET_CONNECT_TIMEOUT,
        ),
    )
    .await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        crate::tasks::RegularTask::new(),
    )
    .await;

    let first = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
        .await
        .expect("expected turn started event without waiting for startup prewarm")
        .expect("channel open");
    assert!(matches!(
        first.msg,
        EventMsg::TurnStarted(TurnStartedEvent { turn_id, .. }) if turn_id == tc.sub_id
    ));

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;
}

#[tokio::test]
async fn request_mcp_server_elicitation_auto_accepts_when_auto_deny_is_enabled() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;
    session
        .services
        .mcp_connection_manager
        .read()
        .await
        .set_elicitations_auto_deny(/*auto_deny*/ true);

    let requested_schema: McpElicitationSchema = serde_json::from_value(json!({
        "type": "object",
        "properties": {},
    }))
    .expect("schema should deserialize");
    let response = session
        .request_mcp_server_elicitation(
            turn_context.as_ref(),
            RequestId::String("request-1".into()),
            McpServerElicitationRequestParams {
                thread_id: session.conversation_id.to_string(),
                turn_id: Some(turn_context.sub_id.clone()),
                server_name: "codex_apps".to_string(),
                request: McpServerElicitationRequest::Form {
                    meta: None,
                    message: "Allow this request?".to_string(),
                    requested_schema,
                },
            },
        )
        .await;

    assert_eq!(
        response,
        Some(ElicitationResponse {
            action: ElicitationAction::Accept,
            content: Some(json!({})),
            meta: None,
        })
    );
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn interrupting_regular_turn_waiting_on_startup_prewarm_emits_turn_aborted() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let model_client_api = Arc::clone(&sess.services.model_client_api);
    let (_tx, startup_prewarm_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = startup_prewarm_rx.await;
        model_client_api
            .create_turn_client()
            .await
            .map_err(|err| protocol::error::CodexErr::Fatal(err.to_string()))
    });

    sess.set_session_startup_prewarm(
        crate::session_startup_prewarm::SessionStartupPrewarmHandle::new(
            handle,
            std::time::Instant::now(),
            crate::client::WEBSOCKET_CONNECT_TIMEOUT,
        ),
    )
    .await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        crate::tasks::RegularTask::new(),
    )
    .await;

    let first = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
        .await
        .expect("expected turn started event without waiting for startup prewarm")
        .expect("channel open");
    assert!(matches!(
        first.msg,
        EventMsg::TurnStarted(TurnStartedEvent { turn_id, .. }) if turn_id == tc.sub_id
    ));

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    let second = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("expected turn aborted event")
        .expect("channel open");
    let EventMsg::TurnAborted(TurnAbortedEvent {
        turn_id,
        reason,
        completed_at,
        duration_ms,
    }) = second.msg
    else {
        panic!("expected turn aborted event");
    };
    assert_eq!(turn_id, Some(tc.sub_id.clone()));
    assert_eq!(reason, TurnAbortReason::Interrupted);
    assert!(completed_at.is_some());
    assert!(duration_ms.is_some());
}

pub(crate) fn build_test_model_service(
    config: &Config,
    session_configuration: &SessionConfiguration,
    provider_auth_manager: Option<model_service_api::SharedModelProviderAuthManager>,
    model_provider_factory: model_service_api::SharedModelProviderFactory,
) -> SharedModelServiceApi {
    Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
        codex_home: config.codex_home.to_path_buf(),
        config_model_catalog: config.model_catalog.clone(),
        api_runtime_factory: Arc::new(model_service::DefaultApiRuntimeFactory),
        provider_auth_manager,
        model_provider_factory,
        default_provider: Some(session_configuration.provider.clone()),
        providers_by_id: config.model_providers.clone(),
        model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
        attestation_provider: None,
    }))
}

pub(crate) fn build_test_model_service_for_config(
    config: &Config,
    provider_auth_manager: Option<model_service_api::SharedModelProviderAuthManager>,
    model_provider_factory: model_service_api::SharedModelProviderFactory,
) -> SharedModelServiceApi {
    Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
        codex_home: config.codex_home.to_path_buf(),
        config_model_catalog: config.model_catalog.clone(),
        api_runtime_factory: Arc::new(model_service::DefaultApiRuntimeFactory),
        provider_auth_manager,
        model_provider_factory,
        default_provider: Some(config.model_provider.clone()),
        providers_by_id: config.model_providers.clone(),
        model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
        attestation_provider: None,
    }))
}

fn developer_input_texts(items: &[ResponseItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "developer" => {
                Some(content.as_slice())
            }
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            ContentItem::InputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn developer_message_texts(items: &[ResponseItem]) -> Vec<Vec<&str>> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "developer" => {
                Some(content.as_slice())
            }
            _ => None,
        })
        .map(|content| {
            content
                .iter()
                .filter_map(|item| match item {
                    ContentItem::InputText { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

fn user_input_texts(items: &[ResponseItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|item| match item {
            ResponseItem::Message { role, content, .. } if role == "user" => {
                Some(content.as_slice())
            }
            _ => None,
        })
        .flat_map(|content| content.iter())
        .filter_map(|item| match item {
            ContentItem::InputText { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn write_project_hooks(dot_codex: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dot_codex)?;
    std::fs::write(
        dot_codex.join("hooks.json"),
        r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "echo hello from hook"
          }
        ]
      }
    ]
  }
}"#,
    )
}

async fn write_project_trust_config(
    codex_home: &Path,
    trusted_projects: &[(&Path, TrustLevel)],
) -> std::io::Result<()> {
    tokio::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        toml::to_string(&ConfigToml {
            projects: Some(
                trusted_projects
                    .iter()
                    .map(|(project, trust_level)| {
                        (
                            project_trust_key(project),
                            ProjectConfig {
                                trust_level: Some(*trust_level),
                            },
                        )
                    })
                    .collect::<std::collections::HashMap<_, _>>(),
            ),
            ..Default::default()
        })
        .expect("serialize config"),
    )
    .await
}

async fn preview_session_start_hooks(
    config: &crate::config::Config,
) -> std::io::Result<Vec<protocol::protocol::HookRunSummary>> {
    let hooks = Hooks::new(HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &config.config_layer_stack,
            ),
        ),
        ..HooksConfig::default()
    });

    Ok(hooks.preview_session_start(&hooks::SessionStartRequest {
        session_id: ThreadId::new(),
        cwd: config.cwd.clone(),
        transcript_path: None,
        model: "gpt-5.2".to_string(),
        permission_mode: "default".to_string(),
        source: hooks::SessionStartSource::Startup,
    }))
}

pub(crate) fn test_tool_inputs(
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
) -> Arc<crate::session::turn::TurnToolInputs> {
    let session_capability: Arc<dyn thread_service_api::ThreadSessionCapability> =
        Arc::clone(&session) as Arc<dyn thread_service_api::ThreadSessionCapability>;
    let default_agent_type_description =
        codex_agent_roles::spawn_tool_spec::build(&std::collections::BTreeMap::new());
    let result = crate::session::turn::TurnToolInputs {
        session_capability: Arc::downgrade(&session_capability),
        mcp_tools: Vec::new(),
        deferred_mcp_tools: Vec::new(),
        discoverable_tools: Vec::new(),
        default_agent_type_description,
    };
    let _ = (session, turn_context);
    Arc::new(result)
}

pub(crate) async fn dispatch_exec_command_via_tool_service(
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
    call_id: &str,
    arguments: serde_json::Value,
) -> Result<String, FunctionCallError> {
    let result = dispatch_tool_via_tool_service(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        call_id,
        tool_service_api::ToolName::plain("exec_command"),
        ToolCallSource::Direct,
        ToolPayload::Function {
            arguments: arguments.to_string(),
        },
    )
    .await?;
    let response_item = result.result.to_response_item(call_id, &result.payload);
    match response_item {
        ResponseInputItem::FunctionCallOutput { output, .. }
        | ResponseInputItem::CustomToolCallOutput { output, .. } => {
            Ok(output.body.to_text().unwrap_or_default())
        }
        other => Err(FunctionCallError::Fatal(format!(
            "unexpected exec_command response item: {other:?}"
        ))),
    }
}

pub(crate) async fn dispatch_tool_via_tool_service(
    session: Arc<Session>,
    turn_context: Arc<TurnContext>,
    call_id: &str,
    tool_name: tool_service_api::ToolName,
    source: ToolCallSource,
    payload: ToolPayload,
) -> Result<tool_service_api::AnyToolResult, FunctionCallError> {
    let tool_inputs = test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context));
    let tracker = Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new()));
    crate::session::turn::dispatch_tool_call(
        Arc::clone(&session.services.tool_service),
        Arc::clone(&session),
        Arc::clone(&turn_context),
        tool_inputs,
        tracker,
        tool_service_api::ToolCall {
            call_id: call_id.to_string(),
            tool_name,
            payload,
        },
        source,
        CancellationToken::new(),
    )
    .await
}

#[tokio::test]
async fn beta_features_header_omits_remote_compaction_v2() -> anyhow::Result<()> {
    let mut config = ConfigBuilder::default().build().await?;
    config.features.enable(Feature::RemoteCompactionV2)?;

    let header = Session::build_model_client_beta_features_header(&config);

    let advertised_features = header.unwrap_or_default();
    assert!(
        !advertised_features
            .split(',')
            .any(|feature| feature == "remote_compaction_v2")
    );
    Ok(())
}

#[tokio::test]
async fn start_managed_network_proxy_applies_execpolicy_network_rules() -> anyhow::Result<()> {
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        /*requirements*/ None,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
    )?;
    let mut exec_policy = Policy::empty();
    exec_policy.add_network_rule(
        "example.com",
        NetworkRuleProtocol::Https,
        Decision::Allow,
        /*justification*/ None,
    )?;

    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &exec_policy,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
        /*network_policy_decider*/ None,
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ false,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;

    let current_cfg = started_proxy.proxy().current_config().await?;
    assert_eq!(
        current_cfg.network.allowed_domains(),
        Some(vec!["example.com".to_string()])
    );
    Ok(())
}

#[tokio::test]
async fn start_managed_network_proxy_ignores_invalid_execpolicy_network_rules() -> anyhow::Result<()>
{
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            domains: Some(NetworkDomainPermissionsToml {
                entries: std::collections::BTreeMap::from([(
                    "managed.example.com".to_string(),
                    NetworkDomainPermissionToml::Allow,
                )]),
            }),
            managed_allowed_domains_only: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
    )?;
    let mut exec_policy = Policy::empty();
    exec_policy.add_network_rule(
        "example.com",
        NetworkRuleProtocol::Https,
        Decision::Allow,
        /*justification*/ None,
    )?;

    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &exec_policy,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::new_workspace_write_policy()),
        /*network_policy_decider*/ None,
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ false,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;

    let current_cfg = started_proxy.proxy().current_config().await?;
    assert_eq!(
        current_cfg.network.allowed_domains(),
        Some(vec!["managed.example.com".to_string()])
    );
    Ok(())
}

#[tokio::test]
async fn managed_network_proxy_decider_survives_full_access_start() -> anyhow::Result<()> {
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&SandboxPolicy::DangerFullAccess),
    )?;
    let exec_policy = Policy::empty();
    let decider_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let network_policy_decider: Arc<dyn NetworkPolicyDecider> = Arc::new({
        let decider_calls = Arc::clone(&decider_calls);
        move |_request| {
            decider_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            async { NetworkDecision::ask("not_allowed") }
        }
    });

    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &exec_policy,
        &permission_profile_for_sandbox_policy(&SandboxPolicy::DangerFullAccess),
        Some(network_policy_decider),
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ true,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;

    let spec = spec.recompute_for_permission_profile(&permission_profile_for_sandbox_policy(
        &SandboxPolicy::new_workspace_write_policy(),
    ))?;
    spec.apply_to_started_proxy(&started_proxy).await?;
    let current_cfg = started_proxy.proxy().current_config().await?;
    assert_eq!(current_cfg.network.allowed_domains(), None);

    use tokio::io::AsyncReadExt as _;
    use tokio::io::AsyncWriteExt as _;

    let mut stream = tokio::net::TcpStream::connect(started_proxy.proxy().http_addr()).await?;
    stream
        .write_all(
            b"GET http://example.com/ HTTP/1.1\r\nHost: example.com\r\nConnection: close\r\n\r\n",
        )
        .await?;
    let mut buffer = [0_u8; 4096];
    let bytes_read = tokio::time::timeout(StdDuration::from_secs(2), stream.read(&mut buffer))
        .await
        .expect("timed out waiting for proxy response")?;
    let response = String::from_utf8_lossy(&buffer[..bytes_read]);

    assert!(
        response.starts_with("HTTP/1.1 403 Forbidden"),
        "unexpected proxy response: {response}"
    );
    assert!(
        response.contains("x-proxy-error: blocked-by-allowlist"),
        "unexpected proxy response: {response}"
    );
    assert_eq!(
        decider_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "unexpected proxy response: {response}"
    );
    Ok(())
}

#[tokio::test]
async fn new_turn_refreshes_managed_network_proxy_for_sandbox_change() -> anyhow::Result<()> {
    let (mut session, _turn_context) = make_session_and_context().await;
    let initial_policy = SandboxPolicy::new_workspace_write_policy();

    let mut network_config = NetworkProxyConfig::default();
    network_config
        .network
        .set_allowed_domains(vec!["evil.com".to_string()]);
    let requirements = NetworkConstraints {
        domains: Some(NetworkDomainPermissionsToml {
            entries: std::collections::BTreeMap::from([(
                "*.example.com".to_string(),
                NetworkDomainPermissionToml::Allow,
            )]),
        }),
        ..Default::default()
    };
    let spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        network_config,
        Some(requirements),
        &permission_profile_for_sandbox_policy(&initial_policy),
    )?;
    let network_proxy_runtime_factory = codex_network_proxy::DefaultNetworkProxyRuntimeFactory;
    let (started_proxy, _) = Session::start_managed_network_proxy(
        &spec,
        &network_proxy_runtime_factory,
        &Policy::empty(),
        &permission_profile_for_sandbox_policy(&initial_policy),
        /*network_policy_decider*/ None,
        /*blocked_request_observer*/ None,
        /*managed_network_requirements_enabled*/ false,
        crate::config::NetworkProxyAuditMetadata::default(),
    )
    .await?;
    assert_eq!(
        started_proxy
            .proxy()
            .current_config()
            .await?
            .network
            .allowed_domains(),
        Some(vec!["*.example.com".to_string(), "evil.com".to_string()])
    );

    {
        let mut state = session.state.lock().await;
        let mut config = (*state.session_configuration.original_config_do_not_use).clone();
        config.permissions.network = Some(spec);
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(initial_policy.clone(), cwd.as_path())
            .expect("test setup should allow sandbox policy");
        state.session_configuration.original_config_do_not_use = Arc::new(config);
        state
            .session_configuration
            .set_permission_profile_for_tests(PermissionProfile::from_legacy_sandbox_policy(
                &initial_policy,
            ))
            .expect("test setup should allow permission profile");
    }
    session.services.network_proxy = Some(started_proxy);

    session
        .new_turn_with_sub_id(
            "sandbox-policy-change".to_string(),
            SessionSettingsUpdate {
                sandbox_policy: Some(SandboxPolicy::DangerFullAccess),
                ..Default::default()
            },
        )
        .await?;

    let started_proxy = session
        .services
        .network_proxy
        .as_ref()
        .expect("managed network proxy should be present");
    assert_eq!(
        started_proxy
            .proxy()
            .current_config()
            .await?
            .network
            .allowed_domains(),
        Some(vec!["*.example.com".to_string()])
    );

    Ok(())
}

#[tokio::test]
async fn danger_full_access_turns_do_not_expose_managed_network_proxy() -> anyhow::Result<()> {
    let network_spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&SandboxPolicy::DangerFullAccess),
    )?;

    let session = make_session_with_config(move |config| {
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(SandboxPolicy::DangerFullAccess, cwd.as_path())
            .expect("test setup should allow sandbox policy");
        config.permissions.network = Some(network_spec);
    })
    .await?;

    let turn_context = session.new_default_turn().await;
    assert!(turn_context.network.is_none());
    Ok(())
}

#[tokio::test]
async fn workspace_write_turns_continue_to_expose_managed_network_proxy() -> anyhow::Result<()> {
    let sandbox_policy = SandboxPolicy::new_workspace_write_policy();
    let network_spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&sandbox_policy),
    )?;

    let session = make_session_with_config(move |config| {
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(sandbox_policy, cwd.as_path())
            .expect("test setup should allow sandbox policy");
        config.permissions.network = Some(network_spec);
    })
    .await?;

    let turn_context = session.new_default_turn().await;
    assert!(turn_context.network.is_some());
    Ok(())
}

#[tokio::test]
async fn user_shell_commands_do_not_inherit_managed_network_proxy() -> anyhow::Result<()> {
    let sandbox_policy = SandboxPolicy::new_workspace_write_policy();
    let network_spec = crate::config::NetworkProxySpec::from_config_and_constraints(
        NetworkProxyConfig::default(),
        Some(NetworkConstraints {
            enabled: Some(true),
            ..Default::default()
        }),
        &permission_profile_for_sandbox_policy(&sandbox_policy),
    )?;

    let (session, rx) = make_session_with_config_and_rx(move |config| {
        let cwd = config.cwd.clone();
        config
            .permissions
            .set_legacy_sandbox_policy(sandbox_policy, cwd.as_path())
            .expect("test setup should allow sandbox policy");
        config.permissions.network = Some(network_spec);
    })
    .await?;

    let turn_context = session.new_default_turn().await;
    assert!(turn_context.network.is_some());

    #[cfg(windows)]
    let command = r#"$val = $env:HTTP_PROXY; if ([string]::IsNullOrEmpty($val)) { $val = 'not-set' } ; [System.Console]::Write($val)"#.to_string();
    #[cfg(not(windows))]
    let command = r#"sh -c "printf '%s' \"${HTTP_PROXY:-not-set}\"""#.to_string();

    execute_user_shell_command(
        Arc::clone(&session),
        turn_context,
        command,
        CancellationToken::new(),
        UserShellCommandMode::StandaloneTurn,
    )
    .await;

    loop {
        let event = rx.recv().await.expect("channel open");
        if let EventMsg::ExecCommandEnd(event) = event.msg {
            assert_eq!(event.exit_code, 0);
            assert_eq!(event.stdout.trim(), "not-set");
            break;
        }
    }

    Ok(())
}

#[tokio::test]
async fn get_base_instructions_no_user_content() {
    let prompt_with_apply_patch_instructions =
        include_str!("../../prompt_with_apply_patch_instructions.md");
    let models_response = bundled_models_response()
        .unwrap_or_else(|err| panic!("bundled models.json should parse: {err}"));
    let model_info_for_slug = |slug: &str, config: &Config| {
        let model = models_response
            .models
            .iter()
            .find(|candidate| candidate.slug == slug)
            .cloned()
            .unwrap_or_else(|| panic!("model slug {slug} is missing from models.json"));
        model_info::with_config_overrides(model, &config.to_models_manager_config())
    };
    let test_cases = vec![
        InstructionsTestCase {
            slug: "gpt-5.4",
            expects_apply_patch_description: false,
        },
        InstructionsTestCase {
            slug: "gpt-5.4-mini",
            expects_apply_patch_description: false,
        },
        InstructionsTestCase {
            slug: "gpt-5.3-codex",
            expects_apply_patch_description: false,
        },
        InstructionsTestCase {
            slug: "gpt-5.2",
            expects_apply_patch_description: false,
        },
    ];

    let (session, _turn_context) = make_session_and_context().await;
    let config = test_config().await;

    for test_case in test_cases {
        let model_info = model_info_for_slug(test_case.slug, &config);
        if test_case.expects_apply_patch_description {
            assert_eq!(
                model_info.base_instructions.as_str(),
                prompt_with_apply_patch_instructions
            );
        }

        {
            let mut state = session.state.lock().await;
            state.session_configuration.base_instructions = model_info.base_instructions.clone();
        }

        let base_instructions = session.get_base_instructions().await;
        assert_eq!(base_instructions.text, model_info.base_instructions);
    }
}

#[tokio::test]
async fn reload_user_config_layer_updates_effective_apps_config() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    std::fs::write(
        &config_toml_path,
        "[apps.calendar]\nenabled = false\ndestructive_enabled = false\n",
    )
    .expect("write user config");

    session.reload_user_config_layer().await;

    let config = session.get_config().await;
    let apps_toml = config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|table| table.get("apps"))
        .cloned()
        .expect("apps table");
    let apps = config_service::types::AppsConfigToml::deserialize(apps_toml)
        .expect("deserialize apps config");
    let app = apps
        .apps
        .get("calendar")
        .expect("calendar app config exists");

    assert!(!app.enabled);
    assert_eq!(app.destructive_enabled, Some(false));
}

#[tokio::test]
async fn reload_user_config_layer_updates_base_and_selected_profile_layers() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let base_config_path = codex_home.join(CONFIG_TOML_FILE);
    let profile_config_path = codex_home.join("work.config.toml");
    std::fs::write(
        &base_config_path,
        "model = \"base\"\napproval_policy = \"on-failure\"\n",
    )
    .expect("write base user config");
    std::fs::write(&profile_config_path, "model = \"profile-old\"\n")
        .expect("write profile user config");
    let config = ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.to_path_buf())
        .loader_overrides(LoaderOverrides {
            user_config_path: Some(profile_config_path.abs()),
            user_config_profile: Some("work".parse().expect("profile-v2 name")),
            ..LoaderOverrides::without_managed_config_for_tests()
        })
        .build()
        .await
        .expect("load profile config");
    {
        let mut state = session.state.lock().await;
        state.session_configuration.original_config_do_not_use = Arc::new(config);
    }
    std::fs::write(
        &base_config_path,
        "model = \"base\"\napproval_policy = \"never\"\n",
    )
    .expect("update base user config");
    std::fs::write(&profile_config_path, "model = \"profile-new\"\n")
        .expect("update profile user config");

    session.reload_user_config_layer().await;

    let config = session.get_config().await;
    assert_eq!(
        config
            .config_layer_stack
            .get_user_config_file()
            .map(codex_utils_absolute_path::AbsolutePathBuf::as_path),
        Some(profile_config_path.as_path())
    );
    let effective_user_config = config
        .config_layer_stack
        .effective_user_config()
        .expect("merged user config");
    assert_eq!(
        effective_user_config
            .get("model")
            .and_then(toml::Value::as_str),
        Some("profile-new")
    );
    assert_eq!(
        effective_user_config
            .get("approval_policy")
            .and_then(toml::Value::as_str),
        Some("never")
    );
}

#[tokio::test]
async fn reload_user_config_layer_refreshes_hooks() -> anyhow::Result<()> {
    let session = make_session_with_config(|config| {
        config
            .features
            .enable(Feature::CodexHooks)
            .expect("enable Codex hooks");
    })
    .await?;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home)?;
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    let user_config: config_service::TomlValue = serde_json::from_value(serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": "python3 /tmp/user.py",
                }],
            }],
        },
    }))?;

    let request = hooks::SessionStartRequest {
        session_id: session.conversation_id,
        cwd: session.get_config().await.cwd.clone(),
        transcript_path: None,
        model: "gpt-5.2".to_string(),
        permission_mode: "default".to_string(),
        source: hooks::SessionStartSource::Startup,
    };
    assert!(session.hooks().preview_session_start(&request).is_empty());

    let config = session.get_config().await;
    let hook_list = hooks::list_hooks(hooks::HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &config
                    .config_layer_stack
                    .with_user_config(&config_toml_path, user_config.clone()),
            ),
        ),
        ..hooks::HooksConfig::default()
    });
    assert_eq!(hook_list.hooks.len(), 1);
    assert_eq!(
        hook_list.hooks[0].trust_status,
        protocol::protocol::HookTrustStatus::Untrusted
    );

    let trusted_user_config: config_service::TomlValue = serde_json::from_value(serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": "python3 /tmp/user.py",
                }],
            }],
            "state": {
                hook_list.hooks[0].key.clone(): {
                    "trusted_hash": hook_list.hooks[0].current_hash.clone(),
                },
            },
        },
    }))?;
    std::fs::write(&config_toml_path, toml::to_string(&trusted_user_config)?)?;

    session.reload_user_config_layer().await;

    assert_eq!(session.hooks().preview_session_start(&request).len(), 1);
    Ok(())
}

#[tokio::test]
async fn refresh_runtime_config_refreshes_hooks() -> anyhow::Result<()> {
    let (session, _turn_context) = make_session_and_context().await;
    {
        let mut state = session.state.lock().await;
        let mut config = (*state.session_configuration.original_config_do_not_use).clone();
        config
            .features
            .enable(Feature::CodexHooks)
            .expect("enable Codex hooks");
        state.session_configuration.original_config_do_not_use = Arc::new(config);
    }
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home)?;
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    #[derive(serde::Serialize)]
    struct NormalizedHookIdentity {
        event_name: &'static str,
        #[serde(flatten)]
        group: config_service::MatcherGroup,
    }
    let trusted_hash = {
        let identity = NormalizedHookIdentity {
            event_name: "session_start",
            group: config_service::MatcherGroup {
                matcher: None,
                hooks: vec![config_service::HookHandlerConfig::Command {
                    command: "python3 /tmp/user.py".to_string(),
                    command_windows: None,
                    timeout_sec: Some(600),
                    r#async: false,
                    status_message: None,
                }],
            },
        };
        let identity = config_service::TomlValue::try_from(identity)?;
        config_service::version_for_toml(&identity)
    };
    let hook_key = format!("{}:session_start:0:0", config_toml_path.display());
    let trusted_user_config: config_service::TomlValue = serde_json::from_value(serde_json::json!({
        "hooks": {
            "SessionStart": [{
                "hooks": [{
                    "type": "command",
                    "command": "python3 /tmp/user.py",
                }],
            }],
            "state": {
                hook_key: {
                    "trusted_hash": trusted_hash,
                },
            },
        },
    }))?;
    std::fs::write(&config_toml_path, toml::to_string(&trusted_user_config)?)?;

    let request = hooks::SessionStartRequest {
        session_id: session.conversation_id,
        cwd: session.get_config().await.cwd.clone(),
        transcript_path: None,
        model: "gpt-5.2".to_string(),
        permission_mode: "default".to_string(),
        source: hooks::SessionStartSource::Startup,
    };
    assert!(session.hooks().preview_session_start(&request).is_empty());

    let next_config = load_latest_config_for_session(&session).await;
    session.refresh_runtime_config(next_config).await;

    assert_eq!(session.hooks().preview_session_start(&request).len(), 1);
    Ok(())
}

#[tokio::test]
async fn reload_user_config_layer_updates_effective_tool_suggest_config() {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    let config_toml_path = codex_home.join(CONFIG_TOML_FILE);
    std::fs::write(
        &config_toml_path,
        r#"[tool_suggest]
disabled_tools = [
  { type = "connector", id = " calendar " },
  { type = "plugin", id = "slack@openai-curated" },
]
"#,
    )
    .expect("write user config");

    session.reload_user_config_layer().await;

    let config = session.get_config().await;
    assert_eq!(
        config.tool_suggest.disabled_tools,
        vec![
            ToolSuggestDisabledTool::connector("calendar"),
            ToolSuggestDisabledTool::plugin("slack@openai-curated"),
        ]
    );
}

#[tokio::test]
async fn refresh_runtime_config_updates_runtime_refreshable_fields_and_keeps_session_static_settings()
 {
    let (session, _turn_context) = make_session_and_context().await;
    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    std::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        r#"[apps.calendar]
enabled = false
destructive_enabled = false

[tool_suggest]
disabled_tools = [
  { type = "connector", id = " calendar " },
  { type = "plugin", id = "slack@openai-curated" },
]
"#,
    )
    .expect("write user config");

    let original = session.get_config().await;
    let mut next_config = load_latest_config_for_session(&session).await;
    next_config.model = Some("gpt-5.4".to_string());
    next_config.notify = Some(vec!["echo".to_string()]);

    session.refresh_runtime_config(next_config).await;

    let config = session.get_config().await;
    let apps_toml = config
        .config_layer_stack
        .effective_config()
        .as_table()
        .and_then(|table| table.get("apps"))
        .cloned()
        .expect("apps table");
    let apps = config_service::types::AppsConfigToml::deserialize(apps_toml)
        .expect("deserialize apps config");
    let app = apps
        .apps
        .get("calendar")
        .expect("calendar app config exists");

    assert!(!app.enabled);
    assert_eq!(app.destructive_enabled, Some(false));
    assert_eq!(config.model, original.model);
    assert_eq!(config.notify, original.notify);
    assert_eq!(
        config.tool_suggest.disabled_tools,
        vec![
            ToolSuggestDisabledTool::connector("calendar"),
            ToolSuggestDisabledTool::plugin("slack@openai-curated"),
        ]
    );
}

#[tokio::test]
async fn reconstruct_history_matches_live_compactions() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, expected) = sample_rollout(&session, &turn_context).await;

    let reconstruction_turn = session.new_default_turn().await;
    let reconstructed = session
        .reconstruct_history_from_rollout(reconstruction_turn.as_ref(), &rollout_items)
        .await;

    assert_eq!(expected, reconstructed.history);
}

#[tokio::test]
async fn reconstruct_history_uses_replacement_history_verbatim() {
    let (session, turn_context) = make_session_and_context().await;
    let summary_item = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "summary".to_string(),
        }],
        phase: None,
    };
    let replacement_history = vec![
        summary_item.clone(),
        ResponseItem::Message {
            id: None,
            role: "developer".to_string(),
            content: vec![ContentItem::InputText {
                text: "stale developer instructions".to_string(),
            }],
            phase: None,
        },
    ];
    let rollout_items = vec![RolloutItem::Compacted(CompactedItem {
        message: String::new(),
        replacement_history: Some(replacement_history.clone()),
    })];

    let reconstructed = session
        .reconstruct_history_from_rollout(&turn_context, &rollout_items)
        .await;

    assert_eq!(reconstructed.history, replacement_history);
}

#[tokio::test]
async fn record_initial_history_reconstructs_resumed_transcript() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, expected) = sample_rollout(&session, &turn_context).await;

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let history = session.state.lock().await.clone_history();
    assert_eq!(expected, history.raw_items());
}

#[tokio::test]
async fn record_initial_history_new_materializes_initial_context_immediately() {
    let (mut session, _turn_context) = make_session_and_context().await;
    let rollout_path = attach_thread_persistence(&mut session).await;

    session.record_initial_history(InitialHistory::New).await;

    let history = session.clone_history().await;
    assert!(
        !history.raw_items().is_empty(),
        "new threads should record initial context into history immediately"
    );
    let current_context = session.reference_context_item().await;
    assert!(
        current_context.is_some(),
        "new threads should seed a context baseline"
    );
    assert_eq!(session.previous_turn_settings().await, None);

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    assert!(
        resumed.history.iter().any(|item| matches!(
            item,
            RolloutItem::ResponseItem(ResponseItem::Message { .. })
        )),
        "materialized rollout should include the initial context messages"
    );
    let persisted_turn_context = resumed.history.iter().find_map(|item| match item {
        RolloutItem::TurnContext(ctx) => Some(ctx.clone()),
        _ => None,
    });
    assert_eq!(
        serde_json::to_value(persisted_turn_context).expect("serialize persisted turn context"),
        serde_json::to_value(current_context).expect("serialize current turn context")
    );
}

#[tokio::test]
async fn resumed_history_injects_initial_context_on_first_context_update_only() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, mut expected) = sample_rollout(&session, &turn_context).await;

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let history_before_seed = session.state.lock().await.clone_history();
    assert_eq!(expected, history_before_seed.raw_items());

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    expected.extend(session.build_initial_context(&turn_context).await);
    let history_after_seed = session.clone_history().await;
    assert_eq!(expected, history_after_seed.raw_items());

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    let history_after_second_seed = session.clone_history().await;
    assert_eq!(
        history_after_seed.raw_items(),
        history_after_second_seed.raw_items()
    );
}

#[tokio::test]
async fn record_initial_history_seeds_token_info_from_rollout() {
    let (session, turn_context) = make_session_and_context().await;
    let (mut rollout_items, _expected) = sample_rollout(&session, &turn_context).await;

    let info1 = TokenUsageInfo {
        total_token_usage: TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 0,
            total_tokens: 30,
        },
        last_token_usage: TokenUsage {
            input_tokens: 3,
            cached_input_tokens: 0,
            output_tokens: 4,
            reasoning_output_tokens: 0,
            total_tokens: 7,
        },
        model_context_window: Some(1_000),
    };
    let info2 = TokenUsageInfo {
        total_token_usage: TokenUsage {
            input_tokens: 100,
            cached_input_tokens: 50,
            output_tokens: 200,
            reasoning_output_tokens: 25,
            total_tokens: 375,
        },
        last_token_usage: TokenUsage {
            input_tokens: 10,
            cached_input_tokens: 0,
            output_tokens: 20,
            reasoning_output_tokens: 5,
            total_tokens: 35,
        },
        model_context_window: Some(2_000),
    };

    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: Some(info1),
            rate_limits: None,
        },
    )));
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: None,
            rate_limits: None,
        },
    )));
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: Some(info2.clone()),
            rate_limits: None,
        },
    )));
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: None,
            rate_limits: None,
        },
    )));

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let actual = session.state.lock().await.token_info();
    assert_eq!(actual, Some(info2));
}

#[tokio::test]
async fn thread_context_usage_recomputes_after_resume_without_persisted_snapshot() {
    let (session, turn_context) = make_session_and_context().await;
    let (mut rollout_items, _expected) = sample_rollout(&session, &turn_context).await;
    rollout_items.push(RolloutItem::EventMsg(EventMsg::TokenCount(
        TokenCountEvent {
            info: Some(TokenUsageInfo {
                total_token_usage: TokenUsage {
                    input_tokens: 100,
                    cached_input_tokens: 50,
                    output_tokens: 200,
                    reasoning_output_tokens: 25,
                    total_tokens: 375,
                },
                last_token_usage: TokenUsage {
                    input_tokens: 10,
                    cached_input_tokens: 0,
                    output_tokens: 20,
                    reasoning_output_tokens: 5,
                    total_tokens: 35,
                },
                model_context_window: Some(1_000),
            }),
            rate_limits: None,
        },
    )));

    assert!(
        rollout_items.iter().all(|item| !matches!(
            item,
            RolloutItem::EventMsg(EventMsg::ThreadContextUsageUpdated(_))
        )),
        "test history should reproduce rollouts without persisted context usage"
    );

    session
        .record_initial_history(InitialHistory::Resumed(ResumedHistory {
            conversation_id: ThreadId::default(),
            history: rollout_items,
            rollout_path: Some(PathBuf::from("/tmp/resume.jsonl")),
        }))
        .await;

    let usage = session.thread_context_usage().await;

    assert!(usage.total_bytes > 0);
    assert!(usage.categories.user_messages > 0);
    assert!(usage.categories.llm_messages > 0);
    assert_eq!(usage.budget_used_percent, Some(37));
}

#[tokio::test]
async fn thread_context_usage_counts_compaction_summary_as_compact() {
    let (session, turn_context) = make_session_and_context().await;
    let summary = format!(
        "{}\nThe earlier conversation was compacted into this summary.",
        crate::compact::SUMMARY_PREFIX
    );
    let item = user_message(&summary);
    session
        .record_into_history(std::slice::from_ref(&item), &turn_context)
        .await;

    let usage = session.thread_context_usage().await;

    assert!(usage.categories.compact > 0);
    assert_eq!(usage.categories.user_messages, 0);
}

#[tokio::test]
async fn recompute_token_usage_uses_session_base_instructions() {
    let (session, turn_context) = make_session_and_context().await;

    let override_instructions = "SESSION_OVERRIDE_INSTRUCTIONS_ONLY".repeat(120);
    {
        let mut state = session.state.lock().await;
        state.session_configuration.base_instructions = override_instructions.clone();
    }

    let item = user_message("hello");
    session
        .record_into_history(std::slice::from_ref(&item), &turn_context)
        .await;

    let history = session.clone_history().await;
    let session_base_instructions = BaseInstructions {
        text: override_instructions,
    };
    let expected_tokens = history
        .estimate_token_count_with_base_instructions(&session_base_instructions)
        .expect("estimate with session base instructions");
    let model_estimated_tokens = history
        .estimate_token_count_with_base_instructions(&BaseInstructions {
            text: turn_context.model_info.get_model_instructions(
                turn_context.personality.or(turn_context.config.personality),
            ),
        })
        .expect("estimate with model instructions");
    assert_ne!(expected_tokens, model_estimated_tokens);

    session.recompute_token_usage(&turn_context).await;

    let actual_tokens = session
        .state
        .lock()
        .await
        .token_info()
        .expect("token info")
        .last_token_usage
        .total_tokens;
    assert_eq!(actual_tokens, expected_tokens.max(0));
}

#[tokio::test]
async fn recompute_token_usage_updates_model_context_window() {
    let (session, mut turn_context) = make_session_and_context().await;

    {
        let mut state = session.state.lock().await;
        state.set_token_info(Some(TokenUsageInfo {
            total_token_usage: TokenUsage::default(),
            last_token_usage: TokenUsage::default(),
            model_context_window: Some(258_400),
        }));
    }

    turn_context.model_info.context_window = Some(128_000);
    turn_context.model_info.effective_context_window_percent = 100;

    session.recompute_token_usage(&turn_context).await;

    let actual = session.state.lock().await.token_info().expect("token info");
    assert_eq!(actual.model_context_window, Some(128_000));
}

#[tokio::test]
async fn record_token_usage_info_notifies_extension_contributors() {
    struct SessionTokenUsageMarker;
    struct ThreadTokenUsageMarker;

    #[derive(Debug, PartialEq, Eq)]
    struct RecordedTokenUsage {
        session_level_id: String,
        thread_level_id: String,
        turn_level_id: String,
        token_usage: TokenUsageInfo,
        saw_session_store: bool,
        saw_thread_store: bool,
    }

    struct TokenUsageRecorder {
        records: Arc<std::sync::Mutex<Vec<RecordedTokenUsage>>>,
    }

    impl codex_extension_api::TokenUsageContributor for TokenUsageRecorder {
        fn on_token_usage(
            &self,
            session_store: &codex_extension_api::ExtensionData,
            thread_store: &codex_extension_api::ExtensionData,
            turn_store: &codex_extension_api::ExtensionData,
            token_usage: &TokenUsageInfo,
        ) {
            self.records
                .lock()
                .expect("token usage records lock")
                .push(RecordedTokenUsage {
                    session_level_id: session_store.level_id().to_string(),
                    thread_level_id: thread_store.level_id().to_string(),
                    turn_level_id: turn_store.level_id().to_string(),
                    token_usage: token_usage.clone(),
                    saw_session_store: session_store.get::<SessionTokenUsageMarker>().is_some(),
                    saw_thread_store: thread_store.get::<ThreadTokenUsageMarker>().is_some(),
                });
        }
    }

    let (mut session, turn_context) = make_session_and_context().await;
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.token_usage_contributor(Arc::new(TokenUsageRecorder {
        records: Arc::clone(&records),
    }));
    session.services.extensions = Arc::new(builder.build());
    session
        .services
        .session_extension_data
        .insert(SessionTokenUsageMarker);
    session
        .services
        .thread_extension_data
        .insert(ThreadTokenUsageMarker);

    let first_usage = TokenUsage {
        input_tokens: 10,
        cached_input_tokens: 2,
        output_tokens: 20,
        reasoning_output_tokens: 3,
        total_tokens: 33,
    };
    let second_usage = TokenUsage {
        input_tokens: 7,
        cached_input_tokens: 1,
        output_tokens: 8,
        reasoning_output_tokens: 5,
        total_tokens: 20,
    };

    session
        .record_token_usage_info(&turn_context, Some(&first_usage))
        .await;
    session
        .record_token_usage_info(&turn_context, Some(&second_usage))
        .await;

    let mut expected_total_usage = first_usage.clone();
    expected_total_usage.add_assign(&second_usage);
    let expected = vec![
        RecordedTokenUsage {
            session_level_id: session.session_id().to_string(),
            thread_level_id: session.conversation_id.to_string(),
            turn_level_id: turn_context.sub_id.clone(),
            token_usage: TokenUsageInfo {
                total_token_usage: first_usage.clone(),
                last_token_usage: first_usage,
                model_context_window: turn_context.model_context_window(),
            },
            saw_session_store: true,
            saw_thread_store: true,
        },
        RecordedTokenUsage {
            session_level_id: session.session_id().to_string(),
            thread_level_id: session.conversation_id.to_string(),
            turn_level_id: turn_context.sub_id.clone(),
            token_usage: TokenUsageInfo {
                total_token_usage: expected_total_usage,
                last_token_usage: second_usage,
                model_context_window: turn_context.model_context_window(),
            },
            saw_session_store: true,
            saw_thread_store: true,
        },
    ];
    let actual = records
        .lock()
        .expect("token usage records lock")
        .drain(..)
        .collect::<Vec<_>>();
    assert_eq!(expected, actual);
}

#[tokio::test]
async fn config_change_contributor_observes_effective_config_changes() {
    struct SessionConfigMarker;
    struct ThreadConfigMarker;

    #[derive(Debug, PartialEq)]
    struct RecordedConfigChange {
        previous_model: Option<String>,
        new_model: Option<String>,
        previous_disabled_tools: Vec<ToolSuggestDisabledTool>,
        new_disabled_tools: Vec<ToolSuggestDisabledTool>,
        saw_session_store: bool,
        saw_thread_store: bool,
    }

    struct ConfigRecorder {
        records: Arc<std::sync::Mutex<Vec<RecordedConfigChange>>>,
    }

    impl codex_extension_api::ConfigContributor<crate::config::Config> for ConfigRecorder {
        fn on_config_changed(
            &self,
            session_store: &codex_extension_api::ExtensionData,
            thread_store: &codex_extension_api::ExtensionData,
            previous_config: &crate::config::Config,
            new_config: &crate::config::Config,
        ) {
            self.records
                .lock()
                .expect("config change records lock")
                .push(RecordedConfigChange {
                    previous_model: previous_config.model.clone(),
                    new_model: new_config.model.clone(),
                    previous_disabled_tools: previous_config.tool_suggest.disabled_tools.clone(),
                    new_disabled_tools: new_config.tool_suggest.disabled_tools.clone(),
                    saw_session_store: session_store.get::<SessionConfigMarker>().is_some(),
                    saw_thread_store: thread_store.get::<ThreadConfigMarker>().is_some(),
                });
        }
    }

    let (mut session, _turn_context) = make_session_and_context().await;
    let records = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.config_contributor(Arc::new(ConfigRecorder {
        records: Arc::clone(&records),
    }));
    session.services.extensions = Arc::new(builder.build());
    session
        .services
        .session_extension_data
        .insert(SessionConfigMarker);
    session
        .services
        .thread_extension_data
        .insert(ThreadConfigMarker);

    let original_model = session.collaboration_mode().await.model().to_string();
    let original_disabled_tools = session
        .get_config()
        .await
        .tool_suggest
        .disabled_tools
        .clone();
    let next_model = if original_model == "gpt-5.4" {
        "gpt-5.2"
    } else {
        "gpt-5.4"
    };
    let collaboration_mode = session.collaboration_mode().await.with_updates(
        Some(next_model.to_string()),
        /*effort*/ None,
        /*developer_instructions*/ None,
    );
    session
        .update_settings(SessionSettingsUpdate {
            collaboration_mode: Some(collaboration_mode),
            ..Default::default()
        })
        .await
        .expect("update settings");

    let codex_home = session.codex_home().await;
    std::fs::create_dir_all(&codex_home).expect("create codex home");
    std::fs::write(
        codex_home.join(CONFIG_TOML_FILE),
        r#"[tool_suggest]
disabled_tools = [
  { type = "connector", id = " calendar " },
  { type = "plugin", id = "slack@openai-curated" },
]
"#,
    )
    .expect("write user config");
    let next_config = load_latest_config_for_session(&session).await;
    session.refresh_runtime_config(next_config).await;

    let expected_disabled_tools = vec![
        ToolSuggestDisabledTool::connector("calendar"),
        ToolSuggestDisabledTool::plugin("slack@openai-curated"),
    ];
    let expected = vec![
        RecordedConfigChange {
            previous_model: Some(original_model),
            new_model: Some(next_model.to_string()),
            previous_disabled_tools: original_disabled_tools.clone(),
            new_disabled_tools: original_disabled_tools.clone(),
            saw_session_store: true,
            saw_thread_store: true,
        },
        RecordedConfigChange {
            previous_model: Some(next_model.to_string()),
            new_model: Some(next_model.to_string()),
            previous_disabled_tools: original_disabled_tools,
            new_disabled_tools: expected_disabled_tools,
            saw_session_store: true,
            saw_thread_store: true,
        },
    ];
    let actual = records
        .lock()
        .expect("config change records lock")
        .drain(..)
        .collect::<Vec<_>>();
    assert_eq!(expected, actual);
}

#[tokio::test]
async fn record_initial_history_reconstructs_forked_transcript() {
    let (session, turn_context) = make_session_and_context().await;
    let (rollout_items, expected) = sample_rollout(&session, &turn_context).await;

    session
        .record_initial_history(InitialHistory::Forked(rollout_items))
        .await;

    let history = session.state.lock().await.clone_history();
    assert_eq!(expected, history.raw_items());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_configured_reports_permission_profile_for_external_sandbox() -> anyhow::Result<()>
{
    let server = start_mock_server().await;
    let sandbox_policy = SandboxPolicy::ExternalSandbox {
        network_access: protocol::protocol::NetworkAccess::Restricted,
    };
    let expected_sandbox_policy = sandbox_policy.clone();
    let mut builder = test_codex().with_config(move |config| {
        config
            .permissions
            .set_permission_profile(PermissionProfile::from_legacy_sandbox_policy(
                &sandbox_policy,
            ))
            .expect("set permission profile");
        config
            .set_legacy_sandbox_policy(sandbox_policy)
            .expect("set sandbox policy");
    });

    let test = builder.build(&server).await?;

    let expected_permission_profile =
        protocol::models::PermissionProfile::from_legacy_sandbox_policy(&expected_sandbox_policy);
    assert_eq!(
        test.session_configured.permission_profile, expected_permission_profile,
        "ExternalSandbox is represented explicitly instead of as a lossy root-write profile"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_permission_profile_rebinds_runtime_workspace_roots() -> anyhow::Result<()> {
    let codex_home = tempfile::TempDir::new()?;
    let cwd = tempfile::TempDir::new()?;
    let old_root = test_path_buf("/workspace/old").abs();
    let new_root = test_path_buf("/workspace/new").abs();
    let config = ConfigBuilder::default()
        .codex_home(codex_home.path().to_path_buf())
        .harness_overrides(crate::config::ConfigOverrides {
            cwd: Some(cwd.path().to_path_buf()),
            default_permissions: Some(BUILT_IN_PERMISSION_PROFILE_WORKSPACE.to_string()),
            additional_writable_roots: vec![old_root.to_path_buf()],
            ..Default::default()
        })
        .build()
        .await?;

    let session_permission_profile_state = session_permission_profile_state_from_config(&config)?;
    let stored_file_system_policy = session_permission_profile_state
        .permission_profile()
        .file_system_sandbox_policy();
    assert!(
        !stored_file_system_policy
            .can_write_path_with_cwd(old_root.as_path(), config.cwd.as_path()),
        "session permission profile state should keep runtime workspace roots symbolic"
    );

    let mut session_configuration = make_session_configuration_for_tests().await;
    session_configuration.cwd = config.cwd.clone();
    session_configuration.workspace_roots = config.workspace_roots.clone();
    session_configuration.permission_profile_state = session_permission_profile_state;

    let initial_policy = session_configuration.file_system_sandbox_policy();
    assert!(initial_policy.can_write_path_with_cwd(old_root.as_path(), config.cwd.as_path()));

    let updated = session_configuration.apply(&SessionSettingsUpdate {
        workspace_roots: Some(vec![new_root.clone()]),
        ..Default::default()
    })?;
    let updated_policy = updated.file_system_sandbox_policy();
    assert!(updated_policy.can_write_path_with_cwd(new_root.as_path(), updated.cwd.as_path()));
    assert!(!updated_policy.can_write_path_with_cwd(old_root.as_path(), updated.cwd.as_path()));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fork_startup_context_then_first_turn_diff_snapshot() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-1"), ev_completed("resp-1")]),
    )
    .await;
    let first_forked_request = mount_sse_once(
        &server,
        sse(vec![ev_response_created("resp-2"), ev_completed("resp-2")]),
    )
    .await;

    let mut builder = test_codex().with_config(|config| {
        config.permissions.approval_policy =
            config_service::Constrained::allow_any(AskForApproval::OnRequest);
    });
    let initial = builder.build(&server).await?;
    let rollout_path = initial
        .session_configured
        .rollout_path
        .clone()
        .expect("rollout path");

    initial
        .codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "fork seed".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;
    wait_for_event(&initial.codex, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;
    // Forking reads the persisted rollout JSONL, so force the completed source turn to disk
    // before snapshotting from it.
    initial.codex.ensure_rollout_materialized().await;
    initial
        .codex
        .flush_rollout()
        .await
        .expect("source rollout should flush before fork");

    let mut fork_config = initial.config.clone();
    fork_config.permissions.approval_policy =
        config_service::Constrained::allow_any(AskForApproval::UnlessTrusted);
    let forked = initial
        .thread_service
        .fork_thread(
            usize::MAX,
            fork_config.clone(),
            rollout_path,
            /*thread_source*/ None,
            /*persist_extended_history*/ false,
            /*parent_trace*/ None,
        )
        .await?;

    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Plan,
        settings: Settings {
            model: forked.session_configured.model.clone(),
            reasoning_effort: None,
            developer_instructions: Some("Fork turn collaboration instructions.".to_string()),
        },
    };
    forked
        .thread
        .submit(Op::OverrideTurnContext {
            cwd: None,
            approval_policy: Some(AskForApproval::Never),
            approvals_reviewer: None,
            sandbox_policy: None,
            permission_profile: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: Some(collaboration_mode),
            personality: None,
        })
        .await?;

    forked
        .thread
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "after fork".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;
    wait_for_event(&forked.thread, |ev| matches!(ev, EventMsg::TurnComplete(_))).await;

    let request = first_forked_request.single_request();
    let snapshot = context_snapshot::format_labeled_requests_snapshot(
        "First request after fork when startup preserves the parent baseline, the fork changes approval policy, and the first forked turn enters plan mode.",
        &[("First Forked Turn Request", &request)],
        &ContextSnapshotOptions::default()
            .render_mode(ContextSnapshotRenderMode::KindWithTextPrefix { max_chars: 96 })
            .strip_capability_instructions()
            .strip_agents_md_user_context(),
    );

    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path("snapshots");
    settings.set_prepend_module_to_snapshot(false);
    settings.bind(|| {
        insta::assert_snapshot!(
            "codex_core__codex_tests__fork_startup_context_then_first_turn_diff",
            snapshot
        );
    });

    Ok(())
}

#[tokio::test]
async fn record_initial_history_forked_hydrates_previous_turn_settings() {
    let (session, turn_context) = make_session_and_context().await;
    let previous_model = "forked-rollout-model";
    let previous_context_item = TurnContextItem {
        turn_id: Some(turn_context.sub_id.clone()),
        trace_id: turn_context.trace_id.clone(),
        #[allow(deprecated)]
        cwd: turn_context.cwd.to_path_buf(),
        current_date: turn_context.current_date.clone(),
        timezone: turn_context.timezone.clone(),
        approval_policy: turn_context.approval_policy.value(),
        sandbox_policy: turn_context.sandbox_policy(),
        permission_profile: None,
        network: None,
        file_system_sandbox_policy: None,
        model: previous_model.to_string(),
        personality: turn_context.personality,
        collaboration_mode: Some(turn_context.collaboration_mode.clone()),
        realtime_active: Some(turn_context.realtime_active),
        effort: turn_context.reasoning_effort,
        summary: turn_context.reasoning_summary,
        user_instructions: None,
        developer_instructions: None,
        final_output_json_schema: None,
        truncation_policy: Some(turn_context.truncation_policy),
    };
    let turn_id = previous_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let rollout_items = vec![
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(
            protocol::protocol::UserMessageEvent {
                message: "forked seed".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            },
        )),
        RolloutItem::TurnContext(previous_context_item.clone()),
        RolloutItem::EventMsg(EventMsg::TurnComplete(
            protocol::protocol::TurnCompleteEvent {
                turn_id,
                last_agent_message: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            },
        )),
    ];

    session
        .record_initial_history(InitialHistory::Forked(rollout_items))
        .await;

    let history = session.clone_history().await;
    assert_eq!(
        session.previous_turn_settings().await,
        Some(PreviousTurnSettings {
            model: previous_model.to_string(),
            realtime_active: Some(turn_context.realtime_active),
        })
    );
    assert_eq!(history.raw_items(), &[]);
    assert_eq!(
        serde_json::to_value(session.reference_context_item().await)
            .expect("serialize fork reference context item"),
        serde_json::to_value(Some(previous_context_item))
            .expect("serialize expected reference context item")
    );
}

#[tokio::test]
async fn thread_rollback_drops_last_turn_from_history() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    let rollout_path = attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    let turn_1 = vec![
        user_message("turn 1 user"),
        assistant_message("turn 1 assistant"),
    ];
    let turn_2 = vec![
        user_message("turn 2 user"),
        assistant_message("turn 2 assistant"),
    ];
    let mut full_history = Vec::new();
    full_history.extend(initial_context.clone());
    full_history.extend(turn_1.clone());
    full_history.extend(turn_2);
    sess.replace_history(full_history.clone(), Some(tc.to_turn_context_item()))
        .await;
    let rollout_items: Vec<RolloutItem> = full_history
        .into_iter()
        .map(RolloutItem::ResponseItem)
        .collect();
    sess.persist_rollout_items(&rollout_items).await;
    sess.set_previous_turn_settings(Some(PreviousTurnSettings {
        model: "stale-model".to_string(),
        realtime_active: Some(tc.realtime_active),
    }))
    .await;
    {
        let mut state = sess.state.lock().await;
        state.set_reference_context_item(Some(tc.to_turn_context_item()));
    }

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;

    let rollback_event = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(rollback_event.num_turns, 1);

    let mut expected = Vec::new();
    expected.extend(initial_context);
    expected.extend(turn_1);

    let history = sess.clone_history().await;
    assert_eq!(expected, history.raw_items());
    assert_eq!(sess.previous_turn_settings().await, None);
    assert!(sess.reference_context_item().await.is_none());

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    assert!(resumed.history.iter().any(|item| {
        matches!(
            item,
            RolloutItem::EventMsg(EventMsg::ThreadRolledBack(rollback))
            if rollback.num_turns == 1
        )
    }));
}

#[tokio::test]
async fn thread_rollback_clears_history_when_num_turns_exceeds_existing_turns() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    let turn_1 = vec![user_message("turn 1 user")];
    let mut full_history = Vec::new();
    full_history.extend(initial_context.clone());
    full_history.extend(turn_1);
    sess.replace_history(full_history.clone(), Some(tc.to_turn_context_item()))
        .await;
    let rollout_items: Vec<RolloutItem> = full_history
        .into_iter()
        .map(RolloutItem::ResponseItem)
        .collect();
    sess.persist_rollout_items(&rollout_items).await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 99).await;

    let rollback_event = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(rollback_event.num_turns, 99);

    let history = sess.clone_history().await;
    assert_eq!(initial_context, history.raw_items());
}

#[tokio::test]
async fn thread_rollback_fails_without_persisted_thread_history() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    sess.record_into_history(&initial_context, tc.as_ref())
        .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;

    let error_event = wait_for_thread_rollback_failed(&rx).await;
    assert_eq!(
        error_event.message,
        "thread rollback requires persisted thread history"
    );
    assert_eq!(
        error_event.codex_error_info,
        Some(CodexErrorInfo::ThreadRollbackFailed)
    );
    assert_eq!(sess.clone_history().await.raw_items(), initial_context);
}

#[tokio::test]
async fn thread_rollback_recomputes_previous_turn_settings_and_reference_context_from_replay() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;

    let first_context_item = tc.to_turn_context_item();
    let first_turn_id = first_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let mut rolled_back_context_item = first_context_item.clone();
    rolled_back_context_item.turn_id = Some("rolled-back-turn".to_string());
    rolled_back_context_item.model = "rolled-back-model".to_string();
    let rolled_back_turn_id = rolled_back_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let turn_one_user = user_message("turn 1 user");
    let turn_one_assistant = assistant_message("turn 1 assistant");
    let turn_two_user = user_message("turn 2 user");
    let turn_two_assistant = assistant_message("turn 2 assistant");

    sess.persist_rollout_items(&[
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: first_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(
            protocol::protocol::UserMessageEvent {
                message: "turn 1 user".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            },
        )),
        RolloutItem::TurnContext(first_context_item.clone()),
        RolloutItem::ResponseItem(turn_one_user.clone()),
        RolloutItem::ResponseItem(turn_one_assistant.clone()),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: first_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: rolled_back_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(
            protocol::protocol::UserMessageEvent {
                message: "turn 2 user".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            },
        )),
        RolloutItem::TurnContext(rolled_back_context_item),
        RolloutItem::ResponseItem(turn_two_user),
        RolloutItem::ResponseItem(turn_two_assistant),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: rolled_back_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
    ])
    .await;
    sess.replace_history(
        vec![assistant_message("stale history")],
        Some(first_context_item.clone()),
    )
    .await;
    sess.set_previous_turn_settings(Some(PreviousTurnSettings {
        model: "stale-model".to_string(),
        realtime_active: None,
    }))
    .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;
    let rollback_event = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(rollback_event.num_turns, 1);

    assert_eq!(
        sess.clone_history().await.raw_items(),
        vec![turn_one_user, turn_one_assistant]
    );
    assert_eq!(
        sess.previous_turn_settings().await,
        Some(PreviousTurnSettings {
            model: tc.model_info.slug.clone(),
            realtime_active: Some(tc.realtime_active),
        })
    );
    assert_eq!(
        serde_json::to_value(sess.reference_context_item().await)
            .expect("serialize replay reference context item"),
        serde_json::to_value(Some(first_context_item))
            .expect("serialize expected reference context item")
    );
}

#[tokio::test]
async fn thread_rollback_restores_cleared_reference_context_item_after_compaction() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;

    let first_context_item = tc.to_turn_context_item();
    let first_turn_id = first_context_item
        .turn_id
        .clone()
        .expect("turn context should have turn_id");
    let compact_turn_id = "compact-turn".to_string();
    let rolled_back_turn_id = "rolled-back-turn".to_string();
    let compacted_history = vec![
        user_message("turn 1 user"),
        user_message("summary after compaction"),
    ];

    sess.persist_rollout_items(&[
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: first_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 1 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(first_context_item.clone()),
        RolloutItem::ResponseItem(user_message("turn 1 user")),
        RolloutItem::ResponseItem(assistant_message("turn 1 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: first_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: compact_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::Compacted(CompactedItem {
            message: "summary after compaction".to_string(),
            replacement_history: Some(compacted_history.clone()),
        }),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: compact_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: rolled_back_turn_id.clone(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 2 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(TurnContextItem {
            turn_id: Some(rolled_back_turn_id.clone()),
            model: "rolled-back-model".to_string(),
            ..first_context_item.clone()
        }),
        RolloutItem::ResponseItem(user_message("turn 2 user")),
        RolloutItem::ResponseItem(assistant_message("turn 2 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: rolled_back_turn_id,
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
    ])
    .await;
    sess.replace_history(
        vec![assistant_message("stale history")],
        Some(first_context_item),
    )
    .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;
    let rollback_event = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(rollback_event.num_turns, 1);

    assert_eq!(sess.clone_history().await.raw_items(), compacted_history);
    assert!(sess.reference_context_item().await.is_none());
}

#[tokio::test]
async fn thread_rollback_persists_marker_and_replays_cumulatively() {
    let (mut sess, tc, rx) = make_session_and_context_with_rx().await;
    let rollout_path = attach_thread_persistence(
        Arc::get_mut(&mut sess).expect("session should not have additional references"),
    )
    .await;
    let turn_context_item = tc.to_turn_context_item();

    sess.persist_rollout_items(&[
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: "turn-1".to_string(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 1 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(turn_context_item.clone()),
        RolloutItem::ResponseItem(user_message("turn 1 user")),
        RolloutItem::ResponseItem(assistant_message("turn 1 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-1".to_string(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: "turn-2".to_string(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 2 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(turn_context_item.clone()),
        RolloutItem::ResponseItem(user_message("turn 2 user")),
        RolloutItem::ResponseItem(assistant_message("turn 2 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-2".to_string(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
        RolloutItem::EventMsg(EventMsg::TurnStarted(
            protocol::protocol::TurnStartedEvent {
                turn_id: "turn-3".to_string(),
                started_at: None,
                model_context_window: Some(128_000),
                collaboration_mode_kind: ModeKind::Default,
            },
        )),
        RolloutItem::EventMsg(EventMsg::UserMessage(UserMessageEvent {
            message: "turn 3 user".to_string(),
            images: None,
            local_images: Vec::new(),
            skills: Vec::new(),
            text_elements: Vec::new(),
        })),
        RolloutItem::TurnContext(turn_context_item),
        RolloutItem::ResponseItem(user_message("turn 3 user")),
        RolloutItem::ResponseItem(assistant_message("turn 3 assistant")),
        RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id: "turn-3".to_string(),
            last_agent_message: None,
            completed_at: None,
            duration_ms: None,
            time_to_first_token_ms: None,
        })),
    ])
    .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;
    let first_rollback = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(first_rollback.num_turns, 1);
    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;
    let second_rollback = wait_for_thread_rolled_back(&rx).await;
    assert_eq!(second_rollback.num_turns, 1);

    assert_eq!(
        sess.clone_history().await.raw_items(),
        vec![
            user_message("turn 1 user"),
            assistant_message("turn 1 assistant")
        ]
    );

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    let rollback_markers = resumed
        .history
        .iter()
        .filter(|item| matches!(item, RolloutItem::EventMsg(EventMsg::ThreadRolledBack(_))))
        .count();
    assert_eq!(rollback_markers, 2);
}

#[tokio::test]
async fn thread_rollback_fails_when_turn_in_progress() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    sess.record_into_history(&initial_context, tc.as_ref())
        .await;

    *sess.active_turn.lock().await = Some(crate::state::ActiveTurn::default());
    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 1).await;

    let error_event = wait_for_thread_rollback_failed(&rx).await;
    assert_eq!(
        error_event.codex_error_info,
        Some(CodexErrorInfo::ThreadRollbackFailed)
    );

    let history = sess.clone_history().await;
    assert_eq!(initial_context, history.raw_items());
}

#[tokio::test]
async fn thread_rollback_fails_when_num_turns_is_zero() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;

    let initial_context = sess.build_initial_context(tc.as_ref()).await;
    sess.record_into_history(&initial_context, tc.as_ref())
        .await;

    handlers::thread_rollback(&sess, "sub-1".to_string(), /*num_turns*/ 0).await;

    let error_event = wait_for_thread_rollback_failed(&rx).await;
    assert_eq!(error_event.message, "num_turns must be >= 1");
    assert_eq!(
        error_event.codex_error_info,
        Some(CodexErrorInfo::ThreadRollbackFailed)
    );

    let history = sess.clone_history().await;
    assert_eq!(initial_context, history.raw_items());
}

#[tokio::test]
async fn set_rate_limits_retains_previous_credits() {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let config = build_test_config(codex_home.path()).await;
    let config = Arc::new(config);
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let reasoning_effort = config.model_reasoning_effort;
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort,
            developer_instructions: None,
        },
    };
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: Vec::new(),
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let mut state = SessionState::new(session_configuration);
    let initial = RateLimitSnapshot {
        limit_id: None,
        limit_name: None,
        primary: Some(RateLimitWindow {
            used_percent: 10.0,
            window_minutes: Some(15),
            resets_at: Some(1_700),
        }),
        secondary: None,
        credits: Some(CreditsSnapshot {
            has_credits: true,
            unlimited: false,
            balance: Some("10.00".to_string()),
        }),
        plan_type: Some(protocol::account::PlanType::Plus),
        rate_limit_reached_type: None,
    };
    state.set_rate_limits(initial.clone());

    let update = RateLimitSnapshot {
        limit_id: Some("codex_other".to_string()),
        limit_name: Some("codex_other".to_string()),
        primary: Some(RateLimitWindow {
            used_percent: 40.0,
            window_minutes: Some(30),
            resets_at: Some(1_800),
        }),
        secondary: Some(RateLimitWindow {
            used_percent: 5.0,
            window_minutes: Some(60),
            resets_at: Some(1_900),
        }),
        credits: None,
        plan_type: None,
        rate_limit_reached_type: None,
    };
    state.set_rate_limits(update.clone());

    assert_eq!(
        state.latest_rate_limits,
        Some(RateLimitSnapshot {
            limit_id: Some("codex_other".to_string()),
            limit_name: Some("codex_other".to_string()),
            primary: update.primary.clone(),
            secondary: update.secondary,
            credits: initial.credits,
            plan_type: initial.plan_type,
            rate_limit_reached_type: None,
        })
    );
}

#[tokio::test]
async fn set_rate_limits_updates_plan_type_when_present() {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let config = build_test_config(codex_home.path()).await;
    let config = Arc::new(config);
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let reasoning_effort = config.model_reasoning_effort;
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort,
            developer_instructions: None,
        },
    };
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: Vec::new(),
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let mut state = SessionState::new(session_configuration);
    let initial = RateLimitSnapshot {
        limit_id: None,
        limit_name: None,
        primary: Some(RateLimitWindow {
            used_percent: 15.0,
            window_minutes: Some(20),
            resets_at: Some(1_600),
        }),
        secondary: Some(RateLimitWindow {
            used_percent: 5.0,
            window_minutes: Some(45),
            resets_at: Some(1_650),
        }),
        credits: Some(CreditsSnapshot {
            has_credits: true,
            unlimited: false,
            balance: Some("15.00".to_string()),
        }),
        plan_type: Some(protocol::account::PlanType::Plus),
        rate_limit_reached_type: None,
    };
    state.set_rate_limits(initial.clone());

    let update = RateLimitSnapshot {
        limit_id: None,
        limit_name: None,
        primary: Some(RateLimitWindow {
            used_percent: 35.0,
            window_minutes: Some(25),
            resets_at: Some(1_700),
        }),
        secondary: None,
        credits: None,
        plan_type: Some(protocol::account::PlanType::Pro),
        rate_limit_reached_type: None,
    };
    state.set_rate_limits(update.clone());

    assert_eq!(
        state.latest_rate_limits,
        Some(RateLimitSnapshot {
            limit_id: Some("codex".to_string()),
            limit_name: None,
            primary: update.primary,
            secondary: update.secondary,
            credits: initial.credits,
            plan_type: update.plan_type,
            rate_limit_reached_type: None,
        })
    );
}

#[test]
fn prefers_structured_content_when_present() {
    let ctr = McpCallToolResult {
        // Content present but should be ignored because structured_content is set.
        content: vec![text_block("ignored")],
        is_error: None,
        structured_content: Some(json!({
            "ok": true,
            "value": 42
        })),
        meta: None,
    };

    let got = ctr.into_function_call_output_payload();
    let expected = FunctionCallOutputPayload {
        body: FunctionCallOutputBody::Text(
            serde_json::to_string(&json!({
                "ok": true,
                "value": 42
            }))
            .unwrap(),
        ),
        success: Some(true),
    };

    assert_eq!(expected, got);
}

#[tokio::test]
async fn includes_timed_out_message() {
    let exec = ExecToolCallOutput {
        exit_code: 0,
        stdout: StreamOutput::new(String::new()),
        stderr: StreamOutput::new(String::new()),
        aggregated_output: StreamOutput::new("Command output".to_string()),
        duration: StdDuration::from_secs(1),
        timed_out: true,
    };
    let (_, turn_context) = make_session_and_context().await;

    let out = format_exec_output_str(&exec, turn_context.truncation_policy);

    assert_eq!(
        out,
        "command timed out after 1000 milliseconds\nCommand output"
    );
}

#[tokio::test]
async fn turn_context_with_model_updates_model_fields() {
    let (session, mut turn_context) = make_session_and_context().await;
    turn_context.reasoning_effort = Some(ReasoningEffortConfig::Minimal);
    let updated = turn_context
        .with_model("gpt-5.4".to_string(), &session.services.model_service)
        .await;
    let expected_model_info = session
        .services
        .model_service
        .get_model_info("gpt-5.4")
        .await
        .expect("model info");

    assert_eq!(updated.config.model.as_deref(), Some("gpt-5.4"));
    assert_eq!(updated.collaboration_mode.model(), "gpt-5.4");
    assert_eq!(updated.model_info, expected_model_info);
    assert_eq!(
        updated.reasoning_effort,
        Some(ReasoningEffortConfig::Medium)
    );
    assert_eq!(
        updated.collaboration_mode.reasoning_effort(),
        Some(ReasoningEffortConfig::Medium)
    );
    assert_eq!(
        updated.config.model_reasoning_effort,
        Some(ReasoningEffortConfig::Medium)
    );
    assert_eq!(
        updated.truncation_policy,
        expected_model_info.truncation_policy.into()
    );
}

#[test]
fn falls_back_to_content_when_structured_is_null() {
    let ctr = McpCallToolResult {
        content: vec![text_block("hello"), text_block("world")],
        is_error: None,
        structured_content: Some(serde_json::Value::Null),
        meta: None,
    };

    let got = ctr.into_function_call_output_payload();
    let expected = FunctionCallOutputPayload {
        body: FunctionCallOutputBody::Text(
            serde_json::to_string(&vec![text_block("hello"), text_block("world")]).unwrap(),
        ),
        success: Some(true),
    };

    assert_eq!(expected, got);
}

#[test]
fn success_flag_reflects_is_error_true() {
    let ctr = McpCallToolResult {
        content: vec![text_block("unused")],
        is_error: Some(true),
        structured_content: Some(json!({ "message": "bad" })),
        meta: None,
    };

    let got = ctr.into_function_call_output_payload();
    let expected = FunctionCallOutputPayload {
        body: FunctionCallOutputBody::Text(
            serde_json::to_string(&json!({ "message": "bad" })).unwrap(),
        ),
        success: Some(false),
    };

    assert_eq!(expected, got);
}

#[test]
fn success_flag_true_with_no_error_and_content_used() {
    let ctr = McpCallToolResult {
        content: vec![text_block("alpha")],
        is_error: Some(false),
        structured_content: None,
        meta: None,
    };

    let got = ctr.into_function_call_output_payload();
    let expected = FunctionCallOutputPayload {
        body: FunctionCallOutputBody::Text(
            serde_json::to_string(&vec![text_block("alpha")]).unwrap(),
        ),
        success: Some(true),
    };

    assert_eq!(expected, got);
}

async fn wait_for_thread_rolled_back(rx: &async_channel::Receiver<Event>) -> ThreadRolledBackEvent {
    let deadline = StdDuration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        let remaining = deadline.saturating_sub(start.elapsed());
        let evt = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("event");
        match evt.msg {
            EventMsg::ThreadRolledBack(payload) => return payload,
            _ => continue,
        }
    }
}

async fn wait_for_thread_rollback_failed(rx: &async_channel::Receiver<Event>) -> ErrorEvent {
    let deadline = StdDuration::from_secs(2);
    let start = std::time::Instant::now();
    loop {
        let remaining = deadline.saturating_sub(start.elapsed());
        let evt = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("event");
        match evt.msg {
            EventMsg::Error(payload)
                if payload.codex_error_info == Some(CodexErrorInfo::ThreadRollbackFailed) =>
            {
                return payload;
            }
            _ => continue,
        }
    }
}

async fn attach_thread_persistence(session: &mut Session) -> PathBuf {
    let config = session.get_config().await;
    let live_thread = LiveThread::create(
        Arc::clone(&session.services.thread_store),
        CreateThreadParams {
            thread_id: session.conversation_id,
            forked_from_id: None,
            source: SessionSource::Exec,
            thread_source: None,
            base_instructions: BaseInstructions::default(),
            dynamic_tools: Vec::new(),
            metadata: ThreadPersistenceMetadata {
                cwd: Some(config.cwd.to_path_buf()),
                model_provider: config.model_provider_id.clone(),
                memory_mode: if config.memories.generate_memories {
                    ThreadMemoryMode::Enabled
                } else {
                    ThreadMemoryMode::Disabled
                },
            },
            event_persistence_mode: ThreadEventPersistenceMode::Limited,
        },
    )
    .await
    .expect("create thread persistence");
    session.services.live_thread = Some(Arc::new(live_thread));
    session.ensure_rollout_materialized().await;
    session
        .flush_rollout()
        .await
        .expect("attached rollout should flush");
    session
        .current_rollout_path()
        .await
        .expect("load rollout path")
        .expect("thread should have rollout path")
}

fn text_block(s: &str) -> serde_json::Value {
    json!({
        "type": "text",
        "text": s,
    })
}

async fn build_test_config(codex_home: &Path) -> Config {
    ConfigBuilder::without_managed_config_for_tests()
        .codex_home(codex_home.to_path_buf())
        .fallback_cwd(Some(codex_home.to_path_buf()))
        .build()
        .await
        .expect("load default test config")
}

fn session_telemetry(
    conversation_id: ThreadId,
    config: &Config,
    model_info: &ModelInfo,
    session_source: SessionSource,
) -> SessionTelemetry {
    SessionTelemetry::new(
        conversation_id,
        get_model_offline_for_tests(config.model.as_deref()).as_str(),
        model_info.slug.as_str(),
        /*account_id*/ None,
        Some("test@test.com".to_string()),
        Some(TelemetryAuthMode::Chatgpt),
        "test_originator".to_string(),
        /*log_user_prompts*/ false,
        "test".to_string(),
        session_source,
    )
}

#[tokio::test]
async fn session_settings_null_service_tier_update_clears_service_tier() {
    let session_configuration = make_session_configuration_for_tests().await;

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            service_tier: Some(None),
            ..Default::default()
        })
        .expect("null service tier update should apply");

    assert_eq!(updated.service_tier, None);
}

#[tokio::test]
async fn session_settings_model_provider_update_switches_provider() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let mut config = (*session_configuration.original_config_do_not_use).clone();
    let provider = ModelProviderInfo {
        name: "Corp".to_string(),
        base_url: Some("https://corp.example.test/v1".to_string()),
        ..ModelProviderInfo::default()
    };
    config
        .model_providers
        .insert("corp".to_string(), provider.clone());
    session_configuration.original_config_do_not_use = Arc::new(config);

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            model_provider: Some("corp".to_string()),
            ..Default::default()
        })
        .expect("model provider update should apply");

    assert_eq!(updated.provider, provider);
    assert_eq!(updated.original_config_do_not_use.model_provider_id, "corp");
    assert_eq!(updated.original_config_do_not_use.model_provider, provider);
}

#[tokio::test]
async fn session_settings_model_update_infers_model_option_provider() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let mut config = (*session_configuration.original_config_do_not_use).clone();
    let provider = ModelProviderInfo {
        name: "ModelHub GPT".to_string(),
        base_url: Some("https://modelhub.example.test/v1".to_string()),
        ..ModelProviderInfo::default()
    };
    config
        .model_providers
        .insert("modelhub-gpt".to_string(), provider.clone());
    config
        .model_options
        .push(config_service::config_toml::ModelOptionToml {
            model: "gpt-5.5-2026-04-24".to_string(),
            provider: "modelhub-gpt".to_string(),
            ..Default::default()
        });
    session_configuration.original_config_do_not_use = Arc::new(config);

    let collaboration_mode = session_configuration.collaboration_mode.with_updates(
        Some("gpt-5.5-2026-04-24".to_string()),
        None,
        /*developer_instructions*/ None,
    );
    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            collaboration_mode: Some(collaboration_mode),
            ..Default::default()
        })
        .expect("model option provider should be inferred from model update");

    assert_eq!(updated.provider, provider);
    assert_eq!(
        updated.original_config_do_not_use.model_provider_id,
        "modelhub-gpt"
    );
    assert_eq!(updated.original_config_do_not_use.model_provider, provider);
}

#[tokio::test]
async fn session_settings_legacy_fast_service_tier_update_uses_priority_request_value() {
    let session_configuration = make_session_configuration_for_tests().await;

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            service_tier: Some(Some("fast".to_string())),
            ..Default::default()
        })
        .expect("legacy fast service tier update should apply");

    assert_eq!(
        updated.service_tier,
        Some(ServiceTier::Fast.request_value().to_string())
    );
}

pub(crate) async fn make_session_configuration_for_tests() -> SessionConfiguration {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let config = build_test_config(codex_home.path()).await;
    let config = Arc::new(config);
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let reasoning_effort = config.model_reasoning_effort;
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort,
            developer_instructions: None,
        },
    };

    SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: Vec::new(),
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    }
}

fn turn_environments_for_tests(
    environment: &Arc<dyn exec_server_api::ExecEnvironment>,
    cwd: &codex_utils_absolute_path::AbsolutePathBuf,
) -> crate::environment_selection::ResolvedTurnEnvironments {
    crate::environment_selection::ResolvedTurnEnvironments {
        turn_environments: vec![TurnEnvironment {
            environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
            environment: Arc::clone(environment),
            cwd: cwd.clone(),
            shell: None,
        }],
    }
}

#[tokio::test]
async fn session_configuration_apply_preserves_profile_file_system_policy_on_cwd_only_update() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let workspace = tempfile::tempdir().expect("create temp dir");
    let project_root = workspace.path().join("project");
    let original_cwd = project_root.join("subdir");
    let docs_dir = original_cwd.join("docs");
    std::fs::create_dir_all(&docs_dir).expect("create docs dir");
    let docs_dir = docs_dir.abs();

    session_configuration.cwd = original_cwd.abs();
    let sandbox_policy = SandboxPolicy::WorkspaceWrite {
        writable_roots: Vec::new(),
        network_access: false,
        exclude_tmpdir_env_var: true,
        exclude_slash_tmp: true,
    };
    let file_system_sandbox_policy = FileSystemSandboxPolicy::restricted(vec![
        FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        },
        FileSystemSandboxEntry {
            path: FileSystemPath::Path { path: docs_dir },
            access: FileSystemAccessMode::Read,
        },
    ]);
    let network_sandbox_policy = NetworkSandboxPolicy::from(&sandbox_policy);
    session_configuration
        .set_permission_profile_for_tests(
            PermissionProfile::from_runtime_permissions_with_enforcement(
                SandboxEnforcement::from_legacy_sandbox_policy(&sandbox_policy),
                &file_system_sandbox_policy,
                network_sandbox_policy,
            ),
        )
        .expect("set permission profile");
    let expected_file_system_sandbox_policy = file_system_sandbox_policy
        .materialize_project_roots_with_workspace_roots(&session_configuration.workspace_roots);

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            cwd: Some(project_root),
            ..Default::default()
        })
        .expect("cwd-only update should succeed");

    assert_eq!(
        updated.file_system_sandbox_policy(),
        expected_file_system_sandbox_policy
    );
}

#[tokio::test]
async fn session_configuration_apply_permission_profile_preserves_existing_deny_read_entries() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let cwd = tempfile::tempdir().expect("create temp dir");
    session_configuration.cwd = cwd.path().abs();

    let workspace_policy = SandboxPolicy::new_workspace_write_policy();
    let deny_entry = FileSystemSandboxEntry {
        path: FileSystemPath::GlobPattern {
            pattern: "**/*.env".to_string(),
        },
        access: FileSystemAccessMode::None,
    };
    let mut existing_file_system_policy =
        FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
            &workspace_policy,
            session_configuration.cwd.as_path(),
        );
    existing_file_system_policy.glob_scan_max_depth = Some(2);
    existing_file_system_policy.entries.push(deny_entry.clone());
    session_configuration
        .set_permission_profile_for_tests(
            PermissionProfile::from_runtime_permissions_with_enforcement(
                SandboxEnforcement::from_legacy_sandbox_policy(&workspace_policy),
                &existing_file_system_policy,
                NetworkSandboxPolicy::Restricted,
            ),
        )
        .expect("set permission profile");

    let requested_file_system_policy = FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
        &workspace_policy,
        session_configuration.cwd.as_path(),
    );
    let permission_profile = protocol::models::PermissionProfile::from_runtime_permissions(
        &requested_file_system_policy,
        NetworkSandboxPolicy::Restricted,
    );
    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            permission_profile: Some(permission_profile),
            ..Default::default()
        })
        .expect("permission profile update should succeed");

    let mut expected_file_system_policy = requested_file_system_policy
        .materialize_project_roots_with_workspace_roots(&session_configuration.workspace_roots);
    expected_file_system_policy.glob_scan_max_depth = Some(2);
    expected_file_system_policy.entries.push(deny_entry);
    assert_eq!(
        updated.file_system_sandbox_policy(),
        expected_file_system_policy
    );
}

#[tokio::test]
async fn session_configuration_apply_permission_profile_accepts_direct_write_roots() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let cwd = tempfile::tempdir().expect("create cwd");
    session_configuration.cwd = cwd.path().abs();
    let external_write_dir = tempfile::tempdir().expect("create external write root");
    let external_write_path = AbsolutePathBuf::from_absolute_path(
        codex_utils_absolute_path::canonicalize_preserving_symlinks(external_write_dir.path())
            .expect("canonical temp dir"),
    )
    .expect("canonical temp dir should be absolute");
    let file_system_sandbox_policy =
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: external_write_path.clone(),
            },
            access: FileSystemAccessMode::Write,
        }]);
    let permission_profile = PermissionProfile::from_runtime_permissions(
        &file_system_sandbox_policy,
        NetworkSandboxPolicy::Restricted,
    );

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            permission_profile: Some(permission_profile.clone()),
            ..Default::default()
        })
        .expect("permission profile update should accept direct runtime permissions");

    assert_eq!(updated.permission_profile(), permission_profile);
    assert_eq!(
        updated.file_system_sandbox_policy(),
        file_system_sandbox_policy
    );
    assert_eq!(
        updated.sandbox_policy(),
        SandboxPolicy::WorkspaceWrite {
            writable_roots: vec![external_write_path],
            network_access: false,
            exclude_tmpdir_env_var: true,
            exclude_slash_tmp: true,
        }
    );
}

#[tokio::test]
async fn session_configuration_apply_rebinds_symbolic_profile_to_updated_workspace_roots() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let old_root = tempfile::tempdir().expect("create old root");
    let new_root = tempfile::tempdir().expect("create new root");
    let profile_root = tempfile::tempdir().expect("create profile root");
    let old_root = old_root.path().abs();
    let new_root = new_root.path().abs();
    let profile_root = profile_root.path().abs();
    session_configuration.workspace_roots = vec![old_root.clone()];

    let file_system_sandbox_policy =
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        }]);
    let permission_profile = PermissionProfile::from_runtime_permissions(
        &file_system_sandbox_policy,
        NetworkSandboxPolicy::Restricted,
    );

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            workspace_roots: Some(vec![new_root.clone()]),
            permission_profile: Some(permission_profile),
            active_permission_profile: Some(ActivePermissionProfile::new("dev")),
            profile_workspace_roots: Some(vec![profile_root.clone()]),
            ..Default::default()
        })
        .expect("permission profile update should succeed");

    let updated_policy = updated.file_system_sandbox_policy();
    assert!(updated_policy.can_write_path_with_cwd(new_root.as_path(), updated.cwd.as_path()));
    assert!(!updated_policy.can_write_path_with_cwd(old_root.as_path(), updated.cwd.as_path()));
    assert_eq!(
        updated.active_permission_profile(),
        Some(ActivePermissionProfile::new("dev"))
    );
    assert_eq!(updated.profile_workspace_roots(), &[profile_root]);
}

#[tokio::test]
async fn session_configuration_apply_retargets_implicit_workspace_root_on_cwd_update() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let old_root = tempfile::tempdir().expect("create old root");
    let new_root = tempfile::tempdir().expect("create new root");
    let extra_root = tempfile::tempdir().expect("create extra root");
    let old_root = old_root.path().abs();
    let new_root = new_root.path().abs();
    let extra_root = extra_root.path().abs();
    session_configuration.cwd = old_root.clone();
    session_configuration.workspace_roots = vec![old_root.clone(), extra_root.clone()];

    let file_system_sandbox_policy =
        FileSystemSandboxPolicy::restricted(vec![FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
            },
            access: FileSystemAccessMode::Write,
        }]);
    let permission_profile = PermissionProfile::from_runtime_permissions(
        &file_system_sandbox_policy,
        NetworkSandboxPolicy::Restricted,
    );
    session_configuration
        .set_permission_profile_for_tests(permission_profile)
        .expect("set permission profile");

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            cwd: Some(new_root.to_path_buf()),
            ..Default::default()
        })
        .expect("cwd-only update should succeed");

    assert_eq!(
        updated.workspace_roots,
        vec![new_root.clone(), extra_root.clone()]
    );
    let updated_policy = updated.file_system_sandbox_policy();
    assert!(updated_policy.can_write_path_with_cwd(new_root.as_path(), updated.cwd.as_path()));
    assert!(updated_policy.can_write_path_with_cwd(extra_root.as_path(), updated.cwd.as_path()));
    assert!(!updated_policy.can_write_path_with_cwd(old_root.as_path(), updated.cwd.as_path()));
}

#[cfg_attr(windows, ignore)]
#[tokio::test]
async fn new_default_turn_uses_config_aware_skills_for_role_overrides() {
    let (session, _turn_context) = make_session_and_context().await;
    let parent_config = session.get_config().await;
    let codex_home = parent_config.codex_home.clone();
    let skill_dir = codex_home.join("skills").join("demo");
    std::fs::create_dir_all(&skill_dir).expect("create skill dir");
    let skill_path = skill_dir.join("SKILL.md");
    std::fs::write(
        &skill_path,
        "---\nname: demo-skill\ndescription: demo description\n---\n\n# Body\n",
    )
    .expect("write skill");

    let skill_fs = session
        .services
        .environment_manager
        .default_environment()
        .map(|environment| environment.get_filesystem())
        .unwrap_or_else(|| std::sync::Arc::clone(&codex_file_system::LOCAL_FS));
    let parent_outcome = session
        .services
        .skill_service
        .skills_for_cwd(
            &crate::build_skill_service_input_from_config(&parent_config, Vec::new()),
            /*force_reload*/ true,
            Some(Arc::clone(&skill_fs)),
        )
        .await;
    let parent_skill = parent_outcome
        .skills
        .iter()
        .find(|skill| skill.name == "demo-skill")
        .expect("demo skill should be discovered");
    assert_eq!(parent_outcome.is_skill_enabled(parent_skill), true);

    let role_path = codex_home.join("skills-role.toml");
    std::fs::write(
        &role_path,
        format!(
            r#"developer_instructions = "Stay focused"

[[skills.config]]
path = "{}"
enabled = false
"#,
            skill_path.display()
        ),
    )
    .expect("write role config");

    let mut child_config = (*parent_config).clone();
    child_config.agent_roles.insert(
        "custom".to_string(),
        crate::config::AgentRoleConfig {
            description: None,
            config_file: Some(role_path.to_path_buf()),
            nickname_candidates: None,
            ..Default::default()
        },
    );
    crate::agent::role::apply_role_to_config(&mut child_config, Some("custom"))
        .await
        .expect("custom role should apply");

    {
        let mut state = session.state.lock().await;
        state.session_configuration.original_config_do_not_use = Arc::new(child_config);
    }

    let child_turn = session
        .new_default_turn_with_sub_id("role-skill-turn".to_string())
        .await;
    let child_skill = child_turn
        .turn_skills
        .outcome
        .skills
        .iter()
        .find(|skill| skill.name == "demo-skill")
        .expect("demo skill should be discovered");
    assert_eq!(
        child_turn.turn_skills.outcome.is_skill_enabled(child_skill),
        false
    );
}

#[tokio::test]
async fn session_configuration_apply_retargets_legacy_workspace_root_on_cwd_update() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let workspace = tempfile::tempdir().expect("create temp dir");
    let original_cwd = workspace.path().join("repo-a").abs();
    let project_root = workspace.path().join("repo-b").abs();
    session_configuration.cwd = original_cwd.clone();
    session_configuration.workspace_roots = vec![session_configuration.cwd.clone()];
    let sandbox_policy = SandboxPolicy::WorkspaceWrite {
        writable_roots: Vec::new(),
        network_access: false,
        exclude_tmpdir_env_var: true,
        exclude_slash_tmp: true,
    };
    let file_system_sandbox_policy = FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
        &sandbox_policy,
        &session_configuration.cwd,
    );
    session_configuration
        .set_permission_profile_for_tests(
            PermissionProfile::from_runtime_permissions_with_enforcement(
                SandboxEnforcement::from_legacy_sandbox_policy(&sandbox_policy),
                &file_system_sandbox_policy,
                NetworkSandboxPolicy::from(&sandbox_policy),
            ),
        )
        .expect("set permission profile");

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            cwd: Some(project_root.to_path_buf()),
            ..Default::default()
        })
        .expect("cwd-only update should succeed");

    assert_eq!(updated.workspace_roots, vec![project_root.clone()]);
    assert!(
        updated
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(project_root.as_path(), updated.cwd.as_path()),
        "cwd-only update should keep the new cwd writable"
    );
    assert!(
        !updated
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(original_cwd.as_path(), updated.cwd.as_path()),
        "cwd-only update should not keep the old implicit cwd writable"
    );
}

#[tokio::test]
async fn session_configuration_apply_preserves_absolute_cwd_write_root_on_cwd_update() {
    let mut session_configuration = make_session_configuration_for_tests().await;
    let workspace = tempfile::tempdir().expect("create temp dir");
    let original_cwd = workspace.path().join("repo-a");
    let next_cwd = workspace.path().join("repo-b");
    std::fs::create_dir_all(&original_cwd).expect("create original cwd");
    std::fs::create_dir_all(&next_cwd).expect("create next cwd");
    let original_cwd = original_cwd.abs();

    session_configuration.cwd = original_cwd.clone();
    let file_system_sandbox_policy = FileSystemSandboxPolicy::restricted(vec![
        FileSystemSandboxEntry {
            path: FileSystemPath::Special {
                value: FileSystemSpecialPath::Root,
            },
            access: FileSystemAccessMode::Read,
        },
        FileSystemSandboxEntry {
            path: FileSystemPath::Path {
                path: original_cwd.clone(),
            },
            access: FileSystemAccessMode::Write,
        },
    ]);
    session_configuration
        .set_permission_profile_for_tests(
            PermissionProfile::from_runtime_permissions_with_enforcement(
                SandboxEnforcement::Managed,
                &file_system_sandbox_policy,
                NetworkSandboxPolicy::Restricted,
            ),
        )
        .expect("set permission profile");

    let updated = session_configuration
        .apply(&SessionSettingsUpdate {
            cwd: Some(next_cwd.clone()),
            ..Default::default()
        })
        .expect("cwd-only update should succeed");

    assert_eq!(
        updated.file_system_sandbox_policy(),
        file_system_sandbox_policy
    );
    assert!(
        updated
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(original_cwd.as_path(), updated.cwd.as_path()),
        "absolute grant to the old cwd must remain writable"
    );
    assert!(
        !updated
            .file_system_sandbox_policy()
            .can_write_path_with_cwd(next_cwd.as_path(), updated.cwd.as_path()),
        "cwd-only update must not reinterpret an absolute old-cwd grant as :workspace_roots"
    );
}

#[tokio::test]
async fn session_update_settings_does_not_rewrite_sticky_environment_cwds() {
    let (session, turn_context) = make_session_and_context().await;
    #[allow(deprecated)]
    let updated_cwd = turn_context.cwd.join("project");
    std::fs::create_dir_all(updated_cwd.as_path()).expect("create project dir");

    session
        .update_settings(SessionSettingsUpdate {
            cwd: Some(PathBuf::from("project")),
            ..Default::default()
        })
        .await
        .expect("cwd update should succeed");

    let session_cwd = {
        let state = session.state.lock().await;
        state.session_configuration.cwd.clone()
    };
    let config = session.get_config().await;
    let next_turn = session.new_default_turn().await;

    assert_eq!(session_cwd, updated_cwd);
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    #[allow(deprecated)]
    let next_turn_cwd = next_turn.cwd.clone();
    assert_eq!(config.cwd, turn_cwd);
    assert_eq!(next_turn_cwd, updated_cwd);
    assert_eq!(next_turn.config.cwd, updated_cwd);
}

#[tokio::test]
async fn relative_cwd_update_without_environments_resolves_under_session_cwd() {
    let (session, _turn_context) = make_session_and_context().await;
    let original_cwd = {
        let mut state = session.state.lock().await;
        state.session_configuration.environments = Vec::new();
        state.session_configuration.cwd.clone()
    };
    let updated_cwd = original_cwd.join("project");
    std::fs::create_dir_all(updated_cwd.as_path()).expect("create project dir");

    session
        .update_settings(SessionSettingsUpdate {
            cwd: Some(PathBuf::from("project")),
            ..Default::default()
        })
        .await
        .expect("cwd update should succeed");

    let state = session.state.lock().await;
    assert_eq!(state.session_configuration.cwd, updated_cwd);
    assert!(state.session_configuration.environments.is_empty());
}

#[tokio::test]
async fn cwd_update_does_not_rewrite_sticky_environment_cwd() {
    let (session, _turn_context) = make_session_and_context().await;
    let (original_cwd, environment_cwd) = {
        let mut state = session.state.lock().await;
        let original_cwd = state.session_configuration.cwd.clone();
        let environment_cwd = original_cwd.join("environment");
        state.session_configuration.environments = vec![TurnEnvironmentSelection {
            environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
            cwd: environment_cwd.clone(),
        }];
        (original_cwd, environment_cwd)
    };
    let updated_cwd = original_cwd.join("project");
    std::fs::create_dir_all(updated_cwd.as_path()).expect("create project dir");

    session
        .update_settings(SessionSettingsUpdate {
            cwd: Some(PathBuf::from("project")),
            ..Default::default()
        })
        .await
        .expect("cwd update should succeed");

    let state = session.state.lock().await;
    assert_eq!(state.session_configuration.cwd, updated_cwd);
    assert_eq!(
        state.session_configuration.environments[0].cwd,
        environment_cwd
    );
}

#[tokio::test]
async fn absolute_cwd_update_with_turn_environment_is_allowed() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let absolute_cwd = {
        let state = session.state.lock().await;
        state.session_configuration.cwd.join("absolute-turn")
    };
    std::fs::create_dir_all(absolute_cwd.as_path()).expect("create absolute turn dir");

    let turn_context = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                cwd: Some(absolute_cwd.to_path_buf()),
                environments: Some(vec![TurnEnvironmentSelection {
                    environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
                    cwd: absolute_cwd.clone(),
                }]),
                ..Default::default()
            },
        )
        .await
        .expect("absolute cwd with explicit environments should succeed");

    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd, absolute_cwd);
    assert_eq!(turn_context.config.cwd, absolute_cwd);
    assert_eq!(turn_context.environments.turn_environments.len(), 1);
}

#[tokio::test]
async fn session_new_fails_when_zsh_fork_enabled_without_zsh_path() {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let mut config = build_test_config(codex_home.path()).await;
    config
        .features
        .enable(Feature::ShellZshFork)
        .expect("test config should allow shell_zsh_fork");
    config.zsh_path = None;
    let config = Arc::new(config);

    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort: config.model_reasoning_effort,
            developer_instructions: None,
        },
    };
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: Vec::new(),
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let (tx_event, _rx_event) = async_channel::unbounded();
    let (agent_status_tx, _agent_status_rx) = watch::channel(AgentStatus::PendingInit);
    let plugins_manager = Arc::new(PluginsManager::new(config.codex_home.to_path_buf()));
    let mcp_service: Arc<dyn mcp_service_api::McpServiceApi> = Arc::new(
        mcp_service::McpService::new(Arc::new(approval_service::ApprovalService)),
    );
    let skill_service = Arc::new(SkillService::new(
        config.codex_home.clone(),
        /*bundled_skills_enabled*/ true,
    ));
    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    let provider_auth_manager = codex_login::model_provider_auth_manager(Some(auth_manager));
    let model_provider_factory = crate::test_support::model_provider_factory_for_tests();
    let model_service = build_test_model_service(
        config.as_ref(),
        &session_configuration,
        provider_auth_manager.clone(),
        Arc::clone(&model_provider_factory),
    );
    let command_service = Arc::new(command_service::CommandService::new());
    let result = Session::new(
        session_configuration,
        Arc::clone(&config),
        "11111111-1111-4111-8111-111111111111".to_string(),
        auth_runtime,
        provider_auth_manager,
        model_provider_factory,
        Arc::new(ExecPolicyManager::default()),
        Arc::new(crate::EmptyExecPolicyLoader),
        tx_event,
        agent_status_tx,
        InitialHistory::New,
        SessionSource::Exec,
        skill_service,
        plugins_manager,
        mcp_service,
        Arc::new(mcp_service::DefaultMcpAuthRuntime),
        Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
        Arc::new(model_service::DefaultApiRuntimeFactory),
        Arc::new(codex_otel::OtelSessionTelemetryFactory),
        Arc::new(memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider),
        model_service,
        Arc::new(hooks_api::DisabledHookRuntimeFactory),
        Arc::new(codex_sandboxing_api::DisabledSandboxRuntime),
        Arc::new(codex_network_proxy::DefaultNetworkProxyRuntimeFactory),
        command_service,
        Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        AgentControl::default(),
        Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
        /*analytics_events_client*/ None,
        Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            /*state_db*/ None,
        )),
        /*state_db*/ None,
        Arc::new(thread_store::DefaultLiveThreadFactory),
        rollout_trace::ThreadTraceContext::disabled(),
        /*attestation_provider*/ None,
        Arc::new(crate::ActiveEventSubscriptionTracker::default()),
        Arc::new(codex_openai_files_api::DisabledOpenAiFileUploader),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeService),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        Arc::new(approval_service::ApprovalService),
        Arc::new(goal_service::GoalService),
        Arc::new(DisabledToolServiceForTests),
    )
    .await;

    let err = match result {
        Ok(_) => panic!("expected startup to fail"),
        Err(err) => err,
    };
    let msg = format!("{err:#}");
    assert!(msg.contains("zsh fork feature enabled, but `zsh_path` is not configured"));
}

// todo: use online model info
pub(crate) async fn make_session_and_context() -> (Session, TurnContext) {
    let (tx_event, _rx_event) = async_channel::unbounded();
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let config = build_test_config(codex_home.path()).await;
    let config = Arc::new(config);
    let thread_id = ThreadId::default();
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let models_manager = models_manager_with_provider(
        config.codex_home.to_path_buf(),
        auth_manager.clone(),
        config.model_provider.clone(),
    );
    let agent_control = AgentControl::default();
    let exec_policy = Arc::new(ExecPolicyManager::default());
    let (agent_status_tx, _agent_status_rx) = watch::channel(AgentStatus::PendingInit);
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let reasoning_effort = config.model_reasoning_effort;
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort,
            developer_instructions: None,
        },
    };
    let default_environments = vec![TurnEnvironmentSelection {
        environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
        cwd: config.cwd.clone(),
    }];
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: default_environments,
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };
    let per_turn_config =
        Session::build_per_turn_config(&session_configuration, session_configuration.cwd.clone());
    let model_info = construct_model_info_offline_for_tests(
        session_configuration.collaboration_mode.model(),
        &per_turn_config.to_models_manager_config(),
    );
    let session_telemetry = Arc::new(session_telemetry(
        thread_id,
        config.as_ref(),
        &model_info,
        session_configuration.session_source.clone(),
    )) as session_telemetry_api::SharedSessionTelemetry;

    let state = SessionState::new(session_configuration.clone());
    let plugins_manager = Arc::new(PluginsManager::new(config.codex_home.to_path_buf()));
    let skill_service = Arc::new(SkillService::new(
        config.codex_home.clone(),
        /*bundled_skills_enabled*/ true,
    ));
    let network_approval: Arc<dyn SessionNetworkApprovalApi> =
        Arc::new(NetworkApprovalService::default());
    let environment: Arc<dyn exec_server_api::ExecEnvironment> = Arc::new(
        codex_exec_server::Environment::create_for_tests(/*exec_server_url*/ None)
            .expect("create environment"),
    );
    let command_service_state = Arc::new(command_service::CommandSessionState::new(
        config.background_terminal_max_timeout,
    ));
    let session_extension_data =
        codex_extension_api::ExtensionData::new(agent_control.session_id().to_string());
    session_extension_data.insert(command_service_state.manager_handle());
    let thread_extension_data = codex_extension_api::ExtensionData::new(thread_id.to_string());
    let provider_auth_manager =
        codex_login::model_provider_auth_manager(Some(auth_manager.clone()));
    let model_provider_factory = crate::test_support::model_provider_factory_for_tests();
    let api_runtime_factory: SharedApiRuntimeFactory =
        Arc::new(model_service::DefaultApiRuntimeFactory);
    let model_service: SharedModelServiceApi =
        Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
            codex_home: config.codex_home.to_path_buf(),
            config_model_catalog: config.model_catalog.clone(),
            api_runtime_factory: Arc::clone(&api_runtime_factory),
            provider_auth_manager: provider_auth_manager.clone(),
            model_provider_factory: Arc::clone(&model_provider_factory),
            default_provider: Some(session_configuration.provider.clone()),
            providers_by_id: config.model_providers.clone(),
            model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
            attestation_provider: None,
        }));
    let model_client_api = model_service
        .create_client(CreateModelClientRequest {
            selection: ModelSelectionPolicy {
                requested_model: Some(session_configuration.collaboration_mode.model().to_string()),
                provider_hint: Some(config.model_provider_id.clone()),
                allow_default_fallback: true,
                refresh: ModelCatalogRefresh::OnlineIfUncached,
            },
            installation_id: "11111111-1111-4111-8111-111111111111".to_string(),
            session_id: thread_id.into(),
            thread_id,
            session_source: session_configuration.session_source.clone(),
            reasoning_effort: session_configuration.collaboration_mode.reasoning_effort(),
            service_tier: crate::session::turn::model_service_tier(
                session_configuration.service_tier.as_deref(),
            ),
            verbosity: config.model_verbosity,
            chat_completions_max_tokens_by_model: config
                .model_options
                .iter()
                .filter(|model_option| model_option.provider == config.model_provider_id)
                .filter_map(|model_option| {
                    model_option
                        .max_tokens
                        .map(|max_tokens| (model_option.model.clone(), max_tokens))
                })
                .collect(),
            enable_request_compression: config.features.enabled(Feature::EnableRequestCompression),
            include_timing_metrics: config.features.enabled(Feature::RuntimeMetrics),
            beta_features_header: Session::build_model_client_beta_features_header(config.as_ref()),
        })
        .await
        .expect("create model client api for tests");

    let services = SessionServices {
        mcp_connection_manager: Arc::new(RwLock::new(Box::new(
            mcp_service::McpConnectionManager::new_uninitialized_with_permission_profile(
                &config.permissions.approval_policy,
                config.permissions.permission_profile(),
            ),
        ))),
        mcp_auth_runtime: Arc::new(mcp_service::DefaultMcpAuthRuntime),
        mcp_connection_runtime_factory: Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
        network_proxy_runtime_factory: Arc::new(
            codex_network_proxy::DefaultNetworkProxyRuntimeFactory,
        ),
        mcp_startup_cancellation_token: Mutex::new(CancellationToken::new()),
        command_service_state,
        command_service_api: Arc::new(command_service::CommandService::new()),
        shell_zsh_path: None,
        main_execve_wrapper_exe: config.main_execve_wrapper_exe.clone(),
        analytics_events_client: AnalyticsEventsClient::disabled(),
        hooks: std::sync::RwLock::new(Arc::new(Hooks::new(HooksConfig {
            legacy_notify_argv: config.notify.clone(),
            ..HooksConfig::default()
        })) as Arc<dyn hooks_api::HookRuntime>),
        hook_runtime_factory: Arc::new(hooks::HooksRuntimeFactory),
        rollout_thread_trace: rollout_trace::ThreadTraceContext::disabled(),
        user_shell: Arc::new(default_user_shell()),
        shell_snapshot_tx: watch::channel(None).0,
        show_raw_agent_reasoning: config.show_raw_agent_reasoning,
        exec_policy,
        exec_policy_loader: Arc::new(crate::EmptyExecPolicyLoader),
        auth_runtime: auth_manager.clone(),
        provider_auth_manager,
        model_provider_factory,
        api_runtime_factory,
        session_telemetry_factory: Arc::new(codex_otel::OtelSessionTelemetryFactory),
        memory_tool_developer_instructions_provider: Arc::new(
            memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider,
        ),
        model_service,
        sandbox_runtime: Arc::new(codex_sandboxing_api::DisabledSandboxRuntime),
        session_telemetry: session_telemetry.clone(),
        tool_approvals: Mutex::new(ApprovalStore::default()),
        guardian_rejections: Mutex::new(std::collections::HashMap::new()),
        guardian_rejection_circuit_breaker: Mutex::new(Default::default()),
        runtime_handle: tokio::runtime::Handle::current(),
        skill_service,
        plugins_manager,
        mcp_service: Arc::new(mcp_service::McpService::new(Arc::new(
            approval_service::ApprovalService,
        ))),
        extensions: Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        session_extension_data,
        thread_extension_data,
        agent_control,
        network_proxy: None,
        network_approval: Arc::clone(&network_approval),
        state_db: None,
        live_thread: None,
        thread_store: Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            /*state_db*/ None,
        )),
        live_thread_factory: Arc::new(thread_store::DefaultLiveThreadFactory),
        attestation_provider: None,
        active_event_subscriptions: Arc::new(crate::ActiveEventSubscriptionTracker::default()),
        model_client_api,
        openai_file_uploader: Arc::new(codex_openai_files_api::DisabledOpenAiFileUploader),
        code_mode_service: Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeService),
        code_mode_runtime_factory: Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        approval_service: Arc::new(approval_service::ApprovalService),
        goal_service: Arc::new(goal_service::GoalService),
        tool_service: Arc::new(DisabledToolServiceForTests),
        environment_manager: Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
    };

    let effective_skill_roots = services
        .plugins_manager
        .effective_skill_roots_for_config(&per_turn_config.plugins_config_input())
        .await;
    let skills_input =
        crate::build_skill_service_input_from_config(&per_turn_config, effective_skill_roots);
    let skill_fs = environment.get_filesystem();
    let skills_outcome = Arc::new(
        services
            .skill_service
            .skills_for_config(&skills_input, Some(Arc::clone(&skill_fs)))
            .await,
    );
    let available_models = models_manager
        .try_list_models()
        .expect("available models for tests");
    let turn_environments = turn_environments_for_tests(&environment, &session_configuration.cwd);
    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    let turn_context = Session::make_turn_context(
        thread_id,
        SessionId::from(thread_id),
        Some(auth_runtime),
        codex_login::model_provider_auth_manager(Some(Arc::clone(&auth_manager))),
        services.model_provider_factory.as_ref(),
        &session_telemetry,
        session_configuration.provider.clone(),
        &session_configuration,
        services.user_shell.as_ref(),
        services.shell_zsh_path.as_ref(),
        services.main_execve_wrapper_exe.as_ref(),
        per_turn_config,
        model_info,
        &available_models,
        /*network*/ None,
        turn_environments,
        session_configuration.cwd.clone(),
        "turn_id".to_string(),
        skills_outcome,
        /*goal_tools_supported*/ true,
    );

    let (mailbox, mailbox_rx) = crate::Mailbox::new();
    let session = Session {
        self_weak: std::sync::OnceLock::new(),
        conversation_id: thread_id,
        installation_id: "11111111-1111-4111-8111-111111111111".to_string(),
        tx_event,
        agent_status: agent_status_tx,
        out_of_band_elicitation_paused: watch::channel(false).0,
        state: Mutex::new(state),
        managed_network_proxy_refresh_lock: Semaphore::new(/*permits*/ 1),
        features: config.features.clone(),
        pending_mcp_server_refresh_config: Mutex::new(None),
        conversation: Arc::new(RealtimeConversationManager::new()),
        active_turn: Mutex::new(None),
        mailbox,
        mailbox_rx: Mutex::new(mailbox_rx),
        idle_pending_input: Mutex::new(Vec::new()),
        goal_runtime: codex_agent_runtime::GoalRuntimeState::new(),
        guardian_review_session: crate::session::session::approval_review_session_impl::GuardianReviewSessionManager::default(),
        services,
        next_internal_sub_id: AtomicU64::new(0),
        child_completion: codex_agent_runtime::ChildCompletionState::new(),
        wait_agent_backoff: Mutex::new(std::collections::HashMap::new()),
    };

    (session, turn_context)
}

async fn make_session_with_config(
    mutator: impl FnOnce(&mut Config),
) -> anyhow::Result<Arc<Session>> {
    let (session, _rx_event) = make_session_with_config_and_rx(mutator).await?;
    Ok(session)
}

async fn load_latest_config_for_session(session: &Session) -> Config {
    let config = session.get_config().await;
    ConfigBuilder::default()
        .codex_home(config.codex_home.to_path_buf())
        .fallback_cwd(Some(config.cwd.to_path_buf()))
        .build()
        .await
        .expect("load latest config for session")
}

async fn make_session_with_config_and_rx(
    mutator: impl FnOnce(&mut Config),
) -> anyhow::Result<(Arc<Session>, async_channel::Receiver<Event>)> {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let mut config = build_test_config(codex_home.path()).await;
    mutator(&mut config);
    let config = Arc::new(config);
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort: config.model_reasoning_effort,
            developer_instructions: None,
        },
    };
    let default_environments = vec![TurnEnvironmentSelection {
        environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
        cwd: config.cwd.clone(),
    }];
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: default_environments,
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let (tx_event, rx_event) = async_channel::unbounded();
    let (agent_status_tx, _agent_status_rx) = watch::channel(AgentStatus::PendingInit);
    let plugins_manager = Arc::new(PluginsManager::new(config.codex_home.to_path_buf()));
    let skill_service = Arc::new(SkillService::new(
        config.codex_home.clone(),
        /*bundled_skills_enabled*/ true,
    ));
    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    let provider_auth_manager = codex_login::model_provider_auth_manager(Some(auth_manager));
    let model_provider_factory = crate::test_support::model_provider_factory_for_tests();
    let model_service = build_test_model_service(
        config.as_ref(),
        &session_configuration,
        provider_auth_manager.clone(),
        Arc::clone(&model_provider_factory),
    );
    let command_service = Arc::new(command_service::CommandService::new());
    let mcp_service: Arc<dyn mcp_service_api::McpServiceApi> = Arc::new(
        mcp_service::McpService::new(Arc::new(approval_service::ApprovalService)),
    );
    let session = Session::new(
        session_configuration,
        Arc::clone(&config),
        "11111111-1111-4111-8111-111111111111".to_string(),
        auth_runtime,
        provider_auth_manager,
        model_provider_factory,
        Arc::new(ExecPolicyManager::default()),
        Arc::new(crate::EmptyExecPolicyLoader),
        tx_event,
        agent_status_tx,
        InitialHistory::New,
        SessionSource::Exec,
        skill_service,
        plugins_manager,
        mcp_service,
        Arc::new(mcp_service::DefaultMcpAuthRuntime),
        Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
        Arc::new(model_service::DefaultApiRuntimeFactory),
        Arc::new(codex_otel::OtelSessionTelemetryFactory),
        Arc::new(memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider),
        model_service,
        Arc::new(hooks::HooksRuntimeFactory),
        Arc::new(codex_sandboxing_api::DisabledSandboxRuntime),
        Arc::new(codex_network_proxy::DefaultNetworkProxyRuntimeFactory),
        command_service,
        Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        AgentControl::default(),
        Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
        /*analytics_events_client*/ None,
        Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            /*state_db*/ None,
        )),
        /*state_db*/ None,
        Arc::new(thread_store::DefaultLiveThreadFactory),
        rollout_trace::ThreadTraceContext::disabled(),
        /*attestation_provider*/ None,
        Arc::new(crate::ActiveEventSubscriptionTracker::default()),
        Arc::new(codex_openai_files_api::DisabledOpenAiFileUploader),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeService),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        Arc::new(approval_service::ApprovalService),
        Arc::new(goal_service::GoalService),
        Arc::new(DisabledToolServiceForTests),
    )
    .await?;

    Ok((session, rx_event))
}

async fn make_session_with_history_source_and_agent_control_and_rx(
    initial_history: InitialHistory,
    session_source: SessionSource,
    agent_control: AgentControl,
) -> anyhow::Result<(Arc<Session>, async_channel::Receiver<Event>)> {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let mut config = build_test_config(codex_home.path()).await;
    config.ephemeral = true;
    let config = Arc::new(config);
    let auth_manager = AuthManager::from_auth_for_testing(CodexAuth::from_api_key("Test API Key"));
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort: config.model_reasoning_effort,
            developer_instructions: None,
        },
    };
    let default_environments = vec![TurnEnvironmentSelection {
        environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
        cwd: config.cwd.clone(),
    }];
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: default_environments,
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: session_source.clone(),
        thread_source: None,
        dynamic_tools: Vec::new(),
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };

    let (tx_event, rx_event) = async_channel::unbounded();
    let (agent_status_tx, _agent_status_rx) = watch::channel(AgentStatus::PendingInit);
    let plugins_manager = Arc::new(PluginsManager::new(config.codex_home.to_path_buf()));
    let skill_service = Arc::new(SkillService::new(
        config.codex_home.clone(),
        /*bundled_skills_enabled*/ true,
    ));
    let state_db =
        state::StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
            .await
            .expect("state db should initialize");

    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    let provider_auth_manager = codex_login::model_provider_auth_manager(Some(auth_manager));
    let model_provider_factory = crate::test_support::model_provider_factory_for_tests();
    let model_service = build_test_model_service(
        config.as_ref(),
        &session_configuration,
        provider_auth_manager.clone(),
        Arc::clone(&model_provider_factory),
    );
    let command_service = Arc::new(command_service::CommandService::new());
    let mcp_service: Arc<dyn mcp_service_api::McpServiceApi> = Arc::new(
        mcp_service::McpService::new(Arc::new(approval_service::ApprovalService)),
    );
    let session = Session::new(
        session_configuration,
        Arc::clone(&config),
        "11111111-1111-4111-8111-111111111111".to_string(),
        auth_runtime,
        provider_auth_manager,
        model_provider_factory,
        Arc::new(ExecPolicyManager::default()),
        Arc::new(crate::EmptyExecPolicyLoader),
        tx_event,
        agent_status_tx,
        initial_history,
        session_source,
        skill_service,
        plugins_manager,
        mcp_service,
        Arc::new(mcp_service::DefaultMcpAuthRuntime),
        Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
        Arc::new(model_service::DefaultApiRuntimeFactory),
        Arc::new(codex_otel::OtelSessionTelemetryFactory),
        Arc::new(memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider),
        model_service,
        Arc::new(hooks_api::DisabledHookRuntimeFactory),
        Arc::new(codex_sandboxing_api::DisabledSandboxRuntime),
        Arc::new(codex_network_proxy::DefaultNetworkProxyRuntimeFactory),
        command_service,
        Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        agent_control,
        Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
        /*analytics_events_client*/ None,
        Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            Some(state_db.clone()),
        )),
        Some(state_db),
        Arc::new(thread_store::DefaultLiveThreadFactory),
        rollout_trace::ThreadTraceContext::disabled(),
        /*attestation_provider*/ None,
        Arc::new(crate::ActiveEventSubscriptionTracker::default()),
        Arc::new(codex_openai_files_api::DisabledOpenAiFileUploader),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeService),
        Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        Arc::new(approval_service::ApprovalService),
        Arc::new(goal_service::GoalService),
        Arc::new(DisabledToolServiceForTests),
    )
    .await?;

    Ok((session, rx_event))
}

#[tokio::test]
async fn resumed_root_session_uses_thread_id_as_session_id() {
    let thread_id = ThreadId::new();
    let (session, rx_event) = make_session_with_history_source_and_agent_control_and_rx(
        InitialHistory::Resumed(ResumedHistory {
            conversation_id: thread_id,
            history: Vec::new(),
            rollout_path: None,
        }),
        SessionSource::Exec,
        AgentControl::default(),
    )
    .await
    .expect("resume should succeed");

    assert_eq!(session.thread_id(), thread_id);
    assert_eq!(session.session_id(), SessionId::from(thread_id));

    let event = rx_event.recv().await.expect("session configured event");
    let EventMsg::SessionConfigured(event) = event.msg else {
        panic!("expected session configured event");
    };
    assert_eq!(event.session_id, SessionId::from(thread_id));
    assert_eq!(event.thread_id, thread_id);
}

#[tokio::test]
async fn resumed_subagent_session_keeps_inherited_session_id() {
    let parent_thread_id = ThreadId::new();
    let parent_session_id = SessionId::from(parent_thread_id);
    let thread_id = ThreadId::new();
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id,
        depth: 1,
        agent_path: None,
        agent_nickname: None,
        agent_role: None,
    });
    let (session, rx_event) = make_session_with_history_source_and_agent_control_and_rx(
        InitialHistory::Resumed(ResumedHistory {
            conversation_id: thread_id,
            history: Vec::new(),
            rollout_path: None,
        }),
        session_source,
        AgentControl::default().with_session_id(parent_session_id),
    )
    .await
    .expect("resume should succeed");

    assert_eq!(session.thread_id(), thread_id);
    assert_eq!(session.session_id(), parent_session_id);

    let event = rx_event.recv().await.expect("session configured event");
    let EventMsg::SessionConfigured(event) = event.msg else {
        panic!("expected session configured event");
    };
    assert_eq!(event.session_id, parent_session_id);
    assert_eq!(event.thread_id, thread_id);
}

#[tokio::test]
async fn notify_request_permissions_response_ignores_unmatched_call_id() {
    let (session, _turn_context) = make_session_and_context().await;
    *session.active_turn.lock().await = Some(ActiveTurn::default());

    session
        .notify_request_permissions_response(
            "missing",
            protocol::request_permissions::RequestPermissionsResponse {
                permissions: RequestPermissionProfile {
                    network: Some(protocol::models::NetworkPermissions {
                        enabled: Some(true),
                    }),
                    ..RequestPermissionProfile::default()
                },
                scope: PermissionGrantScope::Turn,
                strict_auto_review: false,
            },
        )
        .await;

    assert_eq!(session.granted_turn_permissions().await, None);
}

#[tokio::test]
async fn record_granted_request_permissions_for_turn_uses_originating_turn() {
    let (session, _turn_context) = make_session_and_context().await;
    let originating_active_turn = ActiveTurn::default();
    let originating_turn_state = Arc::clone(&originating_active_turn.turn_state);
    *session.active_turn.lock().await = Some(originating_active_turn);

    let current_active_turn = ActiveTurn::default();
    let current_turn_state = Arc::clone(&current_active_turn.turn_state);
    *session.active_turn.lock().await = Some(current_active_turn);

    let requested_permissions = RequestPermissionProfile {
        network: Some(protocol::models::NetworkPermissions {
            enabled: Some(true),
        }),
        ..RequestPermissionProfile::default()
    };
    session
        .record_granted_request_permissions_for_turn(
            &protocol::request_permissions::RequestPermissionsResponse {
                permissions: requested_permissions.clone(),
                scope: PermissionGrantScope::Turn,
                strict_auto_review: false,
            },
            Some(&originating_turn_state),
        )
        .await;

    assert_eq!(
        originating_turn_state.lock().await.granted_permissions(),
        Some(requested_permissions.into())
    );
    assert_eq!(current_turn_state.lock().await.granted_permissions(), None);
    assert_eq!(session.granted_turn_permissions().await, None);
}

#[tokio::test]
async fn enable_strict_auto_review_for_turn_uses_originating_turn() {
    let (session, _turn_context) = make_session_and_context().await;
    let originating_active_turn = ActiveTurn::default();
    let originating_turn_state = Arc::clone(&originating_active_turn.turn_state);
    *session.active_turn.lock().await = Some(originating_active_turn);

    let requested_permissions = RequestPermissionProfile {
        network: Some(protocol::models::NetworkPermissions {
            enabled: Some(true),
        }),
        ..RequestPermissionProfile::default()
    };
    session
        .record_granted_request_permissions_for_turn(
            &protocol::request_permissions::RequestPermissionsResponse {
                permissions: requested_permissions.clone(),
                scope: PermissionGrantScope::Turn,
                strict_auto_review: true,
            },
            Some(&originating_turn_state),
        )
        .await;

    assert!(
        originating_turn_state
            .lock()
            .await
            .strict_auto_review_enabled()
    );
}

#[tokio::test]
async fn request_permissions_emits_event_when_granular_policy_allows_requests() {
    let (session, mut turn_context, rx) = make_session_and_context_with_rx().await;
    *session.active_turn.lock().await = Some(ActiveTurn::default());
    Arc::get_mut(&mut turn_context)
        .expect("single turn context ref")
        .approval_policy
        .set(AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval: true,
            rules: true,
            skill_approval: true,
            request_permissions: true,
            mcp_elicitations: true,
        }))
        .expect("test setup should allow updating approval policy");

    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "call-1".to_string();
    let expected_response = protocol::request_permissions::RequestPermissionsResponse {
        permissions: RequestPermissionProfile {
            network: Some(protocol::models::NetworkPermissions {
                enabled: Some(true),
            }),
            ..RequestPermissionProfile::default()
        },
        scope: PermissionGrantScope::Turn,
        strict_auto_review: false,
    };

    let handle = tokio::spawn({
        let session = Arc::clone(&session);
        let turn_context = Arc::clone(&turn_context);
        let call_id = call_id.clone();
        async move {
            session
                .request_permissions(
                    &turn_context,
                    call_id,
                    protocol::request_permissions::RequestPermissionsArgs {
                        reason: Some("need network".to_string()),
                        permissions: RequestPermissionProfile {
                            network: Some(protocol::models::NetworkPermissions {
                                enabled: Some(true),
                            }),
                            ..RequestPermissionProfile::default()
                        },
                    },
                    CancellationToken::new(),
                )
                .await
        }
    });

    let request_event = tokio::time::timeout(StdDuration::from_secs(1), rx.recv())
        .await
        .expect("request_permissions event timed out")
        .expect("request_permissions event missing");
    let EventMsg::RequestPermissions(request) = request_event.msg else {
        panic!("expected request_permissions event");
    };
    assert_eq!(request.call_id, call_id);
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(request.cwd, Some(turn_cwd));

    session
        .notify_request_permissions_response(&request.call_id, expected_response.clone())
        .await;

    let response = tokio::time::timeout(StdDuration::from_secs(1), handle)
        .await
        .expect("request_permissions future timed out")
        .expect("request_permissions join error");

    assert_eq!(response, Some(expected_response));
}

#[tokio::test]
async fn request_permissions_response_materializes_session_cwd_grants_before_recording() {
    let (session, mut turn_context, rx) = make_session_and_context_with_rx().await;
    *session.active_turn.lock().await = Some(ActiveTurn::default());
    Arc::get_mut(&mut turn_context)
        .expect("single turn context ref")
        .approval_policy
        .set(AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval: true,
            rules: true,
            skill_approval: true,
            request_permissions: true,
            mcp_elicitations: true,
        }))
        .expect("test setup should allow updating approval policy");

    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "call-1".to_string();
    let requested_permissions = RequestPermissionProfile {
        file_system: Some(FileSystemPermissions {
            entries: vec![FileSystemSandboxEntry {
                path: FileSystemPath::Special {
                    value: FileSystemSpecialPath::project_roots(/*subpath*/ None),
                },
                access: FileSystemAccessMode::Write,
            }],
            glob_scan_max_depth: None,
        }),
        ..Default::default()
    };

    let handle = tokio::spawn({
        let session = Arc::clone(&session);
        let turn_context = Arc::clone(&turn_context);
        let call_id = call_id.clone();
        let requested_permissions = requested_permissions.clone();
        async move {
            session
                .request_permissions(
                    &turn_context,
                    call_id,
                    protocol::request_permissions::RequestPermissionsArgs {
                        reason: Some("need cwd write".to_string()),
                        permissions: requested_permissions,
                    },
                    CancellationToken::new(),
                )
                .await
        }
    });

    let request_event = tokio::time::timeout(StdDuration::from_secs(1), rx.recv())
        .await
        .expect("request_permissions event timed out")
        .expect("request_permissions event missing");
    let EventMsg::RequestPermissions(request) = request_event.msg else {
        panic!("expected request_permissions event");
    };
    let request_cwd = request.cwd.clone().expect("request cwd");

    session
        .notify_request_permissions_response(
            &request.call_id,
            protocol::request_permissions::RequestPermissionsResponse {
                permissions: request.permissions,
                scope: PermissionGrantScope::Session,
                strict_auto_review: false,
            },
        )
        .await;

    let expected_permissions = RequestPermissionProfile {
        file_system: Some(FileSystemPermissions::from_read_write_roots(
            /*read*/ None,
            Some(vec![request_cwd]),
        )),
        ..Default::default()
    };
    let expected_response = protocol::request_permissions::RequestPermissionsResponse {
        permissions: expected_permissions.clone(),
        scope: PermissionGrantScope::Session,
        strict_auto_review: false,
    };

    let response = tokio::time::timeout(StdDuration::from_secs(1), handle)
        .await
        .expect("request_permissions future timed out")
        .expect("request_permissions join error");

    assert_eq!(response, Some(expected_response));
    assert_eq!(
        session.granted_session_permissions().await,
        Some(expected_permissions.into())
    );
}

#[tokio::test]
async fn request_permissions_is_auto_denied_when_granular_policy_blocks_tool_requests() {
    let (session, mut turn_context, rx) = make_session_and_context_with_rx().await;
    *session.active_turn.lock().await = Some(ActiveTurn::default());
    Arc::get_mut(&mut turn_context)
        .expect("single turn context ref")
        .approval_policy
        .set(AskForApproval::Granular(GranularApprovalConfig {
            sandbox_approval: true,
            rules: true,
            skill_approval: true,
            request_permissions: false,
            mcp_elicitations: true,
        }))
        .expect("test setup should allow updating approval policy");

    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "call-1".to_string();
    let response = session
        .request_permissions(
            &turn_context,
            call_id,
            protocol::request_permissions::RequestPermissionsArgs {
                reason: Some("need network".to_string()),
                permissions: RequestPermissionProfile {
                    network: Some(protocol::models::NetworkPermissions {
                        enabled: Some(true),
                    }),
                    ..RequestPermissionProfile::default()
                },
            },
            CancellationToken::new(),
        )
        .await;

    assert_eq!(
        response,
        Some(protocol::request_permissions::RequestPermissionsResponse {
            permissions: RequestPermissionProfile::default(),
            scope: PermissionGrantScope::Turn,
            strict_auto_review: false,
        })
    );
    assert!(
        tokio::time::timeout(StdDuration::from_millis(100), rx.recv())
            .await
            .is_err(),
        "request_permissions should not emit an event when granular.request_permissions is false"
    );
}

#[tokio::test]
async fn submit_with_id_captures_current_span_trace_context() {
    let (session, _turn_context) = make_session_and_context().await;
    let (tx_sub, rx_sub) = async_channel::bounded(1);
    let (_tx_event, rx_event) = async_channel::unbounded();
    let (_agent_status_tx, agent_status) = watch::channel(AgentStatus::PendingInit);
    let codex = Codex {
        tx_sub,
        rx_event,
        agent_status,
        session: Arc::new(session),
        session_loop_termination: completed_session_loop_termination(),
    };

    let _trace_test_context = install_test_tracing("codex-core-tests");

    let request_parent = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000011-0000000000000022-01".into()),
        tracestate: Some("vendor=value".into()),
    };
    let request_span = info_span!("app_server.request");
    assert!(set_parent_from_w3c_trace_context(
        &request_span,
        &request_parent
    ));

    let expected_trace = async {
        let expected_trace =
            current_span_w3c_trace_context().expect("current span should have trace context");
        codex
            .submit_with_id(Submission {
                id: "sub-1".into(),
                op: Op::Interrupt,
                trace: None,
            })
            .await
            .expect("submit should succeed");
        expected_trace
    }
    .instrument(request_span)
    .await;

    let submitted = rx_sub.recv().await.expect("submission");
    assert_eq!(submitted.trace, Some(expected_trace));
}

#[tokio::test]
async fn new_default_turn_captures_current_span_trace_id() {
    let (session, _turn_context) = make_session_and_context().await;

    let _trace_test_context = install_test_tracing("codex-core-tests");

    let request_parent = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000011-0000000000000022-01".into()),
        tracestate: Some("vendor=value".into()),
    };
    let request_span = info_span!("app_server.request");
    assert!(set_parent_from_w3c_trace_context(
        &request_span,
        &request_parent
    ));

    let turn_context_item = async {
        let expected_trace_id = Span::current()
            .context()
            .span()
            .span_context()
            .trace_id()
            .to_string();
        let turn_context = session.new_default_turn().await;
        let turn_context_item = turn_context.to_turn_context_item();
        assert_eq!(turn_context_item.trace_id, Some(expected_trace_id));
        turn_context_item
    }
    .instrument(request_span)
    .await;

    assert_eq!(
        turn_context_item.trace_id.as_deref(),
        Some("00000000000000000000000000000011")
    );
}

#[test]
fn submission_dispatch_span_prefers_submission_trace_context() {
    let _trace_test_context = install_test_tracing("codex-core-tests");

    let ambient_parent = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000033-0000000000000044-01".into()),
        tracestate: None,
    };
    let ambient_span = info_span!("ambient");
    assert!(set_parent_from_w3c_trace_context(
        &ambient_span,
        &ambient_parent
    ));

    let submission_trace = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000055-0000000000000066-01".into()),
        tracestate: Some("vendor=value".into()),
    };
    let dispatch_span = ambient_span.in_scope(|| {
        submission_dispatch_span(&Submission {
            id: "sub-1".into(),
            op: Op::Interrupt,
            trace: Some(submission_trace),
        })
    });

    let trace_id = dispatch_span.context().span().span_context().trace_id();
    assert_eq!(
        trace_id,
        TraceId::from_hex("00000000000000000000000000000055").expect("trace id")
    );
}

#[test]
fn submission_dispatch_span_uses_debug_for_realtime_audio() {
    let _trace_test_context = install_test_tracing("codex-core-tests");

    let dispatch_span = submission_dispatch_span(&Submission {
        id: "sub-1".into(),
        op: Op::RealtimeConversationAudio(ConversationAudioParams {
            frame: RealtimeAudioFrame {
                data: "ZmFrZQ==".into(),
                sample_rate: 16_000,
                num_channels: 1,
                samples_per_channel: Some(160),
                item_id: None,
            },
        }),
        trace: None,
    });

    assert_eq!(
        dispatch_span.metadata().expect("span metadata").level(),
        &tracing::Level::DEBUG
    );
}

#[test]
fn op_kind_distinguishes_turn_ops() {
    assert_eq!(
        Op::OverrideTurnContext {
            cwd: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            permission_profile: None,
            windows_sandbox_level: None,
            model: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        }
        .kind(),
        "override_turn_context"
    );
    assert_eq!(
        Op::UserInput {
            environments: None,
            items: vec![],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        }
        .kind(),
        "user_input"
    );
    assert_eq!(
        Op::UserInputWithTurnContext {
            environments: None,
            items: vec![],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
            cwd: None,
            workspace_roots: None,
            profile_workspace_roots: None,
            approval_policy: None,
            approvals_reviewer: None,
            sandbox_policy: None,
            permission_profile: None,
            active_permission_profile: None,
            windows_sandbox_level: None,
            model: None,
            model_provider: None,
            effort: None,
            summary: None,
            service_tier: None,
            collaboration_mode: None,
            personality: None,
        }
        .kind(),
        "user_input_with_turn_context"
    );
}

#[tokio::test]
async fn user_turn_updates_approvals_reviewer() {
    let (session, turn_context, _rx) = make_session_and_context_with_rx().await;
    let config = session.get_config().await;

    handlers::user_input_or_turn(
        &session,
        "sub-1".to_string(),
        Op::UserTurn {
            environments: None,
            items: vec![UserInput::Text {
                text: "hello".to_string(),
                text_elements: Vec::new(),
            }],
            cwd: config.cwd.to_path_buf(),
            approval_policy: config.permissions.approval_policy.value(),
            approvals_reviewer: Some(config_service::types::ApprovalsReviewer::AutoReview),
            sandbox_policy: config.legacy_sandbox_policy(),
            permission_profile: None,
            model: turn_context.model_info.slug.clone(),
            effort: config.model_reasoning_effort,
            summary: config.model_reasoning_summary,
            service_tier: None,
            final_output_json_schema: None,
            collaboration_mode: None,
            personality: config.personality,
        },
    )
    .await;

    let state = session.state.lock().await;
    assert_eq!(
        state.session_configuration.approvals_reviewer,
        config_service::types::ApprovalsReviewer::AutoReview
    );
}

#[tokio::test]
async fn turn_environments_set_primary_environment() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let selected_cwd =
        AbsolutePathBuf::try_from(session.get_config().await.cwd.as_path().join("selected"))
            .expect("absolute path");

    let turn_context = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                environments: Some(vec![TurnEnvironmentSelection {
                    environment_id: "local".to_string(),
                    cwd: selected_cwd.clone(),
                }]),
                ..Default::default()
            },
        )
        .await
        .expect("turn should start");

    let turn_environments = &turn_context.environments;
    assert_eq!(turn_environments.turn_environments.len(), 1);
    let turn_environment = turn_context
        .environments
        .primary()
        .expect("primary environment should be set");
    assert!(std::sync::Arc::ptr_eq(
        &turn_environment.environment,
        &turn_environments.turn_environments[0].environment
    ));
    assert!(!turn_context.environments.turn_environments.is_empty());
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd.as_path(), selected_cwd.as_path());
    assert_eq!(turn_context.config.cwd.as_path(), selected_cwd.as_path());
}

#[tokio::test]
async fn default_turn_overlays_session_cwd_onto_stored_thread_environments() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let session_cwd = session.get_config().await.cwd.clone();
    let selected_cwd =
        AbsolutePathBuf::try_from(session_cwd.as_path().join("selected")).expect("absolute path");

    {
        let mut state = session.state.lock().await;
        state.session_configuration.environments = vec![TurnEnvironmentSelection {
            environment_id: "local".to_string(),
            cwd: selected_cwd.clone(),
        }];
    }

    let turn_context = session.new_default_turn().await;

    let turn_environments = &turn_context.environments;
    assert_eq!(turn_environments.turn_environments.len(), 1);
    let turn_environment = turn_context
        .environments
        .primary()
        .expect("primary environment should be set");
    assert!(std::sync::Arc::ptr_eq(
        &turn_environment.environment,
        &turn_environments.turn_environments[0].environment
    ));
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd, session_cwd);
    assert_eq!(turn_context.config.cwd, session_cwd);
}

#[tokio::test]
async fn default_turn_honors_empty_stored_thread_environments() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let session_cwd = session.get_config().await.cwd.clone();

    {
        let mut state = session.state.lock().await;
        state.session_configuration.environments = Vec::new();
    }

    let turn_context = session.new_default_turn().await;

    assert!(turn_context.environments.primary().is_none());
    assert!(turn_context.environments.turn_environments.is_empty());
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd, session_cwd);
    assert_eq!(turn_context.config.cwd, session_cwd);
    assert_eq!(turn_context.environments.turn_environments.len(), 0);
}

#[tokio::test]
async fn primary_environment_uses_first_turn_environment() {
    let (_session, mut turn_context) = make_session_and_context().await;
    let first_environment = turn_context.environments.turn_environments[0].clone();
    #[allow(deprecated)]
    let second_cwd = turn_context.cwd.join("second");
    turn_context
        .environments
        .turn_environments
        .push(TurnEnvironment {
            environment_id: "second".to_string(),
            environment: Arc::clone(&first_environment.environment),
            cwd: second_cwd.clone(),
            shell: None,
        });

    assert_eq!(
        turn_context
            .environments
            .primary()
            .expect("primary environment")
            .environment_id,
        first_environment.environment_id
    );
    assert_eq!(
        turn_context
            .environments
            .turn_environments
            .iter()
            .find(|environment| environment.environment_id == "second")
            .expect("second environment")
            .cwd,
        second_cwd
    );
    assert_eq!(turn_context.environments.turn_environments.len(), 2);
    assert_eq!(
        turn_context.environments.turn_environments[1].cwd,
        second_cwd
    );
}

#[tokio::test]
async fn empty_turn_environments_clear_primary_environment() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;

    let turn_context = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                environments: Some(vec![]),
                ..Default::default()
            },
        )
        .await
        .expect("turn should start");

    assert!(turn_context.environments.primary().is_none());
    assert!(turn_context.environments.turn_environments.is_empty());
    #[allow(deprecated)]
    let turn_cwd = turn_context.cwd.clone();
    assert_eq!(turn_cwd, session.get_config().await.cwd);
    assert_eq!(turn_context.config.cwd, session.get_config().await.cwd);
}

#[tokio::test]
async fn unknown_turn_environment_returns_error() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let original_configuration = {
        let state = session.state.lock().await;
        state.session_configuration.clone()
    };

    let err = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                environments: Some(vec![TurnEnvironmentSelection {
                    environment_id: "missing".to_string(),
                    cwd: original_configuration.cwd.clone(),
                }]),
                ..Default::default()
            },
        )
        .await
        .expect_err("unknown environment should fail");

    let current_configuration = {
        let state = session.state.lock().await;
        state.session_configuration.clone()
    };
    assert!(matches!(err, CodexErr::InvalidRequest(_)));
    assert!(err.to_string().contains("missing"));
    assert_eq!(current_configuration.cwd, original_configuration.cwd);
    assert_eq!(
        current_configuration.environments,
        original_configuration.environments
    );
}

#[tokio::test]
async fn duplicate_turn_environment_returns_error_without_mutating_session() {
    let (session, _turn_context, _rx) = make_session_and_context_with_rx().await;
    let original_configuration = {
        let state = session.state.lock().await;
        state.session_configuration.clone()
    };

    let err = session
        .new_turn_with_sub_id(
            "sub-1".to_string(),
            SessionSettingsUpdate {
                environments: Some(vec![
                    TurnEnvironmentSelection {
                        environment_id: "local".to_string(),
                        cwd: original_configuration.cwd.clone(),
                    },
                    TurnEnvironmentSelection {
                        environment_id: "local".to_string(),
                        cwd: original_configuration.cwd.join("second"),
                    },
                ]),
                ..Default::default()
            },
        )
        .await
        .expect_err("duplicate environment should fail");

    let current_configuration = {
        let state = session.state.lock().await;
        state.session_configuration.clone()
    };
    assert!(matches!(err, CodexErr::InvalidRequest(_)));
    assert!(err.to_string().contains("duplicate"));
    assert_eq!(current_configuration.cwd, original_configuration.cwd);
    assert_eq!(
        current_configuration.environments,
        original_configuration.environments
    );
}

#[tokio::test]
async fn spawn_task_turn_span_inherits_dispatch_trace_context() {
    struct TraceCaptureTask {
        captured_trace: Arc<std::sync::Mutex<Option<W3cTraceContext>>>,
    }

    impl SessionTask for TraceCaptureTask {
        fn kind(&self) -> TaskKind {
            TaskKind::Regular
        }

        fn span_name(&self) -> &'static str {
            "session_task.trace_capture"
        }

        async fn run(
            self: Arc<Self>,
            _session: Arc<SessionTaskContext>,
            _ctx: Arc<TurnContext>,
            _input: Vec<UserInput>,
            _cancellation_token: CancellationToken,
        ) -> Option<String> {
            let mut trace = self
                .captured_trace
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *trace = current_span_w3c_trace_context();
            None
        }
    }

    let _trace_test_context = install_test_tracing("codex-core-tests");

    let request_parent = W3cTraceContext {
        traceparent: Some("00-00000000000000000000000000000011-0000000000000022-01".into()),
        tracestate: Some("vendor=value".into()),
    };
    let request_span = tracing::info_span!("app_server.request");
    assert!(set_parent_from_w3c_trace_context(
        &request_span,
        &request_parent
    ));

    let submission_trace =
        async { current_span_w3c_trace_context().expect("request span should have trace context") }
            .instrument(request_span)
            .await;

    let dispatch_span = submission_dispatch_span(&Submission {
        id: "sub-1".into(),
        op: Op::Interrupt,
        trace: Some(submission_trace.clone()),
    });
    let dispatch_span_id = dispatch_span.context().span().span_context().span_id();

    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let captured_trace = Arc::new(std::sync::Mutex::new(None));

    async {
        sess.spawn_task(
            Arc::clone(&tc),
            vec![UserInput::Text {
                text: "hello".to_string(),
                text_elements: Vec::new(),
            }],
            TraceCaptureTask {
                captured_trace: Arc::clone(&captured_trace),
            },
        )
        .await;
    }
    .instrument(dispatch_span)
    .await;

    let evt = tokio::time::timeout(StdDuration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for turn completion")
        .expect("event");
    assert!(matches!(evt.msg, EventMsg::TurnComplete(_)));

    let task_trace = captured_trace
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
        .expect("turn task should capture the current span trace context");
    let submission_context =
        codex_otel::context_from_w3c_trace_context(&submission_trace).expect("submission");
    let task_context = codex_otel::context_from_w3c_trace_context(&task_trace).expect("task trace");

    assert_eq!(
        task_context.span().span_context().trace_id(),
        submission_context.span().span_context().trace_id()
    );
    assert_ne!(
        task_context.span().span_context().span_id(),
        dispatch_span_id
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn shutdown_complete_does_not_append_to_thread_store_after_shutdown() {
    let (mut session, _turn_context) = make_session_and_context().await;
    let store = Arc::new(thread_store::InMemoryThreadStore::default());
    let thread_store: Arc<dyn thread_store_api::ThreadStore> = store.clone();
    let config = session.get_config().await;
    let live_thread = LiveThread::create(
        Arc::clone(&thread_store),
        CreateThreadParams {
            thread_id: session.conversation_id,
            forked_from_id: None,
            source: SessionSource::Exec,
            thread_source: None,
            base_instructions: BaseInstructions::default(),
            dynamic_tools: Vec::new(),
            metadata: ThreadPersistenceMetadata {
                cwd: Some(config.cwd.to_path_buf()),
                model_provider: config.model_provider_id.clone(),
                memory_mode: if config.memories.generate_memories {
                    ThreadMemoryMode::Enabled
                } else {
                    ThreadMemoryMode::Disabled
                },
            },
            event_persistence_mode: ThreadEventPersistenceMode::Limited,
        },
    )
    .await
    .expect("create thread persistence");
    session.services.thread_store = thread_store;
    session.services.live_thread = Some(Arc::new(live_thread));
    let session = Arc::new(session);

    assert!(handlers::shutdown(&session, "sub-1".to_string()).await);

    assert_eq!(
        thread_store::InMemoryThreadStoreCalls {
            create_thread: 1,
            shutdown_thread: 1,
            ..Default::default()
        },
        store.calls().await
    );
}

#[tokio::test]
async fn submission_loop_channel_close_emits_thread_stop_lifecycle() {
    struct SessionStopMarker;
    struct ThreadStopMarker;

    struct ThreadStopRecorder {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        expected_thread_id: ThreadId,
    }

    impl codex_extension_api::ThreadLifecycleContributor<crate::config::Config> for ThreadStopRecorder {
        fn on_thread_stop(&self, input: codex_extension_api::ThreadStopInput<'_>) {
            assert_eq!(
                self.expected_thread_id.to_string(),
                input.thread_store.level_id()
            );
            assert!(input.session_store.get::<SessionStopMarker>().is_some());
            assert!(input.thread_store.get::<ThreadStopMarker>().is_some());
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    let (mut session, turn_context) = make_session_and_context().await;
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.thread_lifecycle_contributor(Arc::new(ThreadStopRecorder {
        calls: Arc::clone(&calls),
        expected_thread_id: session.conversation_id,
    }));
    session.services.extensions = Arc::new(builder.build());
    session
        .services
        .session_extension_data
        .insert(SessionStopMarker);
    session
        .services
        .thread_extension_data
        .insert(ThreadStopMarker);

    let (tx_sub, rx_sub) = async_channel::bounded(1);
    drop(tx_sub);
    let session = Arc::new(session);
    submission_loop(session, Arc::clone(&turn_context.config), rx_sub).await;

    assert_eq!(1, calls.load(std::sync::atomic::Ordering::SeqCst));
}

#[tokio::test]
async fn submission_loop_channel_close_aborts_active_turn_before_thread_stop_lifecycle() {
    struct LifecycleRecorder {
        calls: Arc<std::sync::Mutex<Vec<&'static str>>>,
        expected_thread_id: ThreadId,
        expected_turn_id: String,
    }

    impl codex_extension_api::ThreadLifecycleContributor<crate::config::Config> for LifecycleRecorder {
        fn on_thread_stop(&self, input: codex_extension_api::ThreadStopInput<'_>) {
            assert_eq!(
                self.expected_thread_id.to_string(),
                input.thread_store.level_id()
            );
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push("thread_stop");
        }
    }

    impl codex_extension_api::TurnLifecycleContributor for LifecycleRecorder {
        fn on_turn_abort(&self, input: codex_extension_api::TurnAbortInput<'_>) {
            assert_eq!(
                self.expected_thread_id.to_string(),
                input.thread_store.level_id()
            );
            assert_eq!(self.expected_turn_id, input.turn_store.level_id());
            assert_eq!(TurnAbortReason::Interrupted, input.reason);
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push("turn_abort");
        }
    }

    let (mut session, turn_context) = make_session_and_context().await;
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = Arc::new(LifecycleRecorder {
        calls: Arc::clone(&calls),
        expected_thread_id: session.conversation_id,
        expected_turn_id: turn_context.sub_id.clone(),
    });
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::<crate::config::Config>::new();
    builder.thread_lifecycle_contributor(recorder.clone());
    builder.turn_lifecycle_contributor(recorder);
    session.services.extensions = Arc::new(builder.build());

    let session = Arc::new(session);
    session
        .spawn_task(
            Arc::new(turn_context),
            Vec::new(),
            NeverEndingTask {
                kind: TaskKind::Regular,
                listen_to_cancellation_token: true,
            },
        )
        .await;

    let (tx_sub, rx_sub) = async_channel::bounded(1);
    drop(tx_sub);
    submission_loop(Arc::clone(&session), session.get_config().await, rx_sub).await;

    assert_eq!(
        vec!["turn_abort", "thread_stop"],
        *calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    );
}

#[tokio::test]
async fn shutdown_and_wait_allows_multiple_waiters() {
    let (session, _turn_context) = make_session_and_context().await;
    let (tx_sub, rx_sub) = async_channel::bounded(4);
    let (_tx_event, rx_event) = async_channel::unbounded();
    let (_agent_status_tx, agent_status) = watch::channel(AgentStatus::PendingInit);
    let session_loop_handle = tokio::spawn(async move {
        let shutdown: Submission = rx_sub.recv().await.expect("shutdown submission");
        assert_eq!(shutdown.op, Op::Shutdown);
        tokio::time::sleep(StdDuration::from_millis(50)).await;
    });
    let codex = Arc::new(Codex {
        tx_sub,
        rx_event,
        agent_status,
        session: Arc::new(session),
        session_loop_termination: session_loop_termination_from_handle(session_loop_handle),
    });

    let waiter_1 = {
        let codex = Arc::clone(&codex);
        tokio::spawn(async move { codex.shutdown_and_wait().await })
    };
    let waiter_2 = {
        let codex = Arc::clone(&codex);
        tokio::spawn(async move { codex.shutdown_and_wait().await })
    };

    waiter_1
        .await
        .expect("first shutdown waiter join")
        .expect("first shutdown waiter");
    waiter_2
        .await
        .expect("second shutdown waiter join")
        .expect("second shutdown waiter");
}

#[tokio::test]
async fn shutdown_and_wait_waits_when_shutdown_is_already_in_progress() {
    let (session, _turn_context) = make_session_and_context().await;
    let (tx_sub, rx_sub) = async_channel::bounded(4);
    drop(rx_sub);
    let (_tx_event, rx_event) = async_channel::unbounded();
    let (_agent_status_tx, agent_status) = watch::channel(AgentStatus::PendingInit);
    let (shutdown_complete_tx, shutdown_complete_rx) = tokio::sync::oneshot::channel();
    let session_loop_handle = tokio::spawn(async move {
        let _ = shutdown_complete_rx.await;
    });
    let codex = Arc::new(Codex {
        tx_sub,
        rx_event,
        agent_status,
        session: Arc::new(session),
        session_loop_termination: session_loop_termination_from_handle(session_loop_handle),
    });

    let waiter = {
        let codex = Arc::clone(&codex);
        tokio::spawn(async move { codex.shutdown_and_wait().await })
    };

    tokio::time::sleep(StdDuration::from_millis(10)).await;
    assert!(!waiter.is_finished());

    shutdown_complete_tx
        .send(())
        .expect("session loop should still be waiting to terminate");

    waiter
        .await
        .expect("shutdown waiter join")
        .expect("shutdown waiter");
}

#[tokio::test]
async fn shutdown_and_wait_shuts_down_cached_guardian_subagent() {
    let (parent_session, parent_turn_context) = make_session_and_context().await;
    let parent_session = Arc::new(parent_session);
    let parent_config = Arc::clone(&parent_turn_context.config);
    let (parent_tx_sub, parent_rx_sub) = async_channel::bounded(4);
    let (_parent_tx_event, parent_rx_event) = async_channel::unbounded();
    let (_parent_status_tx, parent_agent_status) = watch::channel(AgentStatus::PendingInit);
    let parent_session_for_loop = Arc::clone(&parent_session);
    let parent_session_loop_handle = tokio::spawn(async move {
        submission_loop(parent_session_for_loop, parent_config, parent_rx_sub).await;
    });
    let parent_codex = Codex {
        tx_sub: parent_tx_sub,
        rx_event: parent_rx_event,
        agent_status: parent_agent_status,
        session: Arc::clone(&parent_session),
        session_loop_termination: session_loop_termination_from_handle(parent_session_loop_handle),
    };

    let (child_session, _child_turn_context) = make_session_and_context().await;
    let (child_tx_sub, child_rx_sub) = async_channel::bounded(4);
    let (_child_tx_event, child_rx_event) = async_channel::unbounded();
    let (_child_status_tx, child_agent_status) = watch::channel(AgentStatus::PendingInit);
    let (child_shutdown_tx, child_shutdown_rx) = tokio::sync::oneshot::channel();
    let child_session_loop_handle = tokio::spawn(async move {
        let shutdown: Submission = child_rx_sub
            .recv()
            .await
            .expect("child shutdown submission");
        assert_eq!(shutdown.op, Op::Shutdown);
        child_shutdown_tx
            .send(())
            .expect("child shutdown signal should be delivered");
    });
    let child_codex = Codex {
        tx_sub: child_tx_sub,
        rx_event: child_rx_event,
        agent_status: child_agent_status,
        session: Arc::new(child_session),
        session_loop_termination: session_loop_termination_from_handle(child_session_loop_handle),
    };
    let child_reuse_key = crate::session::session::approval_review_session_impl::GuardianReviewSessionReuseKey::from_spawn_config(
        child_codex.session.get_config().await.as_ref(),
    );
    parent_session
        .guardian_review_session
        .cache_session_for_test(Arc::clone(&parent_session), child_codex, child_reuse_key)
        .await;

    parent_codex
        .shutdown_and_wait()
        .await
        .expect("parent shutdown should succeed");

    child_shutdown_rx
        .await
        .expect("guardian subagent should receive a shutdown op");
}

#[tokio::test]
async fn cached_guardian_subagent_exposes_its_rollout_path() {
    let (parent_session, _parent_turn_context) = make_session_and_context().await;
    let parent_session = Arc::new(parent_session);

    let (mut child_session, _child_turn_context) = make_session_and_context().await;
    let child_rollout_path = attach_thread_persistence(&mut child_session).await;
    let (child_tx_sub, _child_rx_sub) = async_channel::bounded(4);
    let (_child_tx_event, child_rx_event) = async_channel::unbounded();
    let (_child_status_tx, child_agent_status) = watch::channel(AgentStatus::PendingInit);
    let child_session_loop_handle = tokio::spawn(async {});
    let child_codex = Codex {
        tx_sub: child_tx_sub,
        rx_event: child_rx_event,
        agent_status: child_agent_status,
        session: Arc::new(child_session),
        session_loop_termination: session_loop_termination_from_handle(child_session_loop_handle),
    };
    let child_reuse_key = crate::session::session::approval_review_session_impl::GuardianReviewSessionReuseKey::from_spawn_config(
        child_codex.session.get_config().await.as_ref(),
    );
    parent_session
        .guardian_review_session
        .cache_session_for_test(Arc::clone(&parent_session), child_codex, child_reuse_key)
        .await;

    assert_eq!(
        parent_session
            .guardian_review_session
            .trunk_rollout_path()
            .await,
        Some(child_rollout_path)
    );
}

#[tokio::test]
async fn shutdown_and_wait_shuts_down_tracked_ephemeral_guardian_review() {
    let (parent_session, parent_turn_context) = make_session_and_context().await;
    let parent_session = Arc::new(parent_session);
    let parent_config = Arc::clone(&parent_turn_context.config);
    let (parent_tx_sub, parent_rx_sub) = async_channel::bounded(4);
    let (_parent_tx_event, parent_rx_event) = async_channel::unbounded();
    let (_parent_status_tx, parent_agent_status) = watch::channel(AgentStatus::PendingInit);
    let parent_session_for_loop = Arc::clone(&parent_session);
    let parent_session_loop_handle = tokio::spawn(async move {
        submission_loop(parent_session_for_loop, parent_config, parent_rx_sub).await;
    });
    let parent_codex = Codex {
        tx_sub: parent_tx_sub,
        rx_event: parent_rx_event,
        agent_status: parent_agent_status,
        session: Arc::clone(&parent_session),
        session_loop_termination: session_loop_termination_from_handle(parent_session_loop_handle),
    };

    let (child_session, _child_turn_context) = make_session_and_context().await;
    let (child_tx_sub, child_rx_sub) = async_channel::bounded(4);
    let (_child_tx_event, child_rx_event) = async_channel::unbounded();
    let (_child_status_tx, child_agent_status) = watch::channel(AgentStatus::PendingInit);
    let (child_shutdown_tx, child_shutdown_rx) = tokio::sync::oneshot::channel();
    let child_session_loop_handle = tokio::spawn(async move {
        let shutdown: Submission = child_rx_sub
            .recv()
            .await
            .expect("child shutdown submission");
        assert_eq!(shutdown.op, Op::Shutdown);
        child_shutdown_tx
            .send(())
            .expect("child shutdown signal should be delivered");
    });
    let child_codex = Codex {
        tx_sub: child_tx_sub,
        rx_event: child_rx_event,
        agent_status: child_agent_status,
        session: Arc::new(child_session),
        session_loop_termination: session_loop_termination_from_handle(child_session_loop_handle),
    };
    let child_reuse_key = crate::session::session::approval_review_session_impl::GuardianReviewSessionReuseKey::from_spawn_config(
        child_codex.session.get_config().await.as_ref(),
    );
    parent_session
        .guardian_review_session
        .register_ephemeral_session_for_test(
            Arc::clone(&parent_session),
            child_codex,
            child_reuse_key,
        )
        .await;

    parent_codex
        .shutdown_and_wait()
        .await
        .expect("parent shutdown should succeed");

    child_shutdown_rx
        .await
        .expect("ephemeral guardian review should receive a shutdown op");
}

async fn make_session_and_context_with_auth_and_config_and_rx<F>(
    auth: CodexAuth,
    dynamic_tools: Vec<DynamicToolSpec>,
    configure_config: F,
) -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
)
where
    F: FnOnce(&mut Config),
{
    let codex_home = tempfile::tempdir().expect("create temp dir");
    make_session_and_context_with_auth_config_home_and_rx(
        auth,
        dynamic_tools,
        codex_home.path(),
        configure_config,
    )
    .await
}

async fn make_session_and_context_with_auth_config_home_and_rx<F>(
    auth: CodexAuth,
    dynamic_tools: Vec<DynamicToolSpec>,
    codex_home: &Path,
    configure_config: F,
) -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
)
where
    F: FnOnce(&mut Config),
{
    let (tx_event, rx_event) = async_channel::unbounded();
    let mut config = build_test_config(codex_home).await;
    configure_config(&mut config);
    let state_db: Option<crate::StateDbHandle> = if config.features.enabled(Feature::Goals) {
        Some(
            state::StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
                .await
                .expect("goal tests should initialize sqlite state db")
                as crate::StateDbHandle,
        )
    } else {
        None
    };
    let config = Arc::new(config);
    let thread_id = ThreadId::default();
    let auth_manager = AuthManager::from_auth_for_testing(auth);
    let models_manager = models_manager_with_provider(
        config.codex_home.to_path_buf(),
        auth_manager.clone(),
        config.model_provider.clone(),
    );
    let agent_control = AgentControl::default();
    let exec_policy = Arc::new(ExecPolicyManager::default());
    let (agent_status_tx, _agent_status_rx) = watch::channel(AgentStatus::PendingInit);
    let model = get_model_offline_for_tests(config.model.as_deref());
    let model_info =
        construct_model_info_offline_for_tests(model.as_str(), &config.to_models_manager_config());
    let reasoning_effort = config.model_reasoning_effort;
    let collaboration_mode = CollaborationMode {
        mode: ModeKind::Default,
        settings: Settings {
            model,
            reasoning_effort,
            developer_instructions: None,
        },
    };
    let default_environments = vec![TurnEnvironmentSelection {
        environment_id: exec_server_api::LOCAL_ENVIRONMENT_ID.to_string(),
        cwd: config.cwd.clone(),
    }];
    let session_configuration = SessionConfiguration {
        provider: config.model_provider.clone(),
        collaboration_mode,
        model_reasoning_summary: config.model_reasoning_summary,
        developer_instructions: config.developer_instructions.clone(),
        user_instructions: config.user_instructions.clone(),
        service_tier: None,
        personality: config.personality,
        base_instructions: config
            .base_instructions
            .clone()
            .unwrap_or_else(|| model_info.get_model_instructions(config.personality)),
        compact_prompt: config.compact_prompt.clone(),
        approval_policy: config.permissions.approval_policy.clone(),
        approvals_reviewer: config.approvals_reviewer,
        permission_profile_state: config.permissions.permission_profile_state().clone(),
        windows_sandbox_level: WindowsSandboxLevel::from_config(&config),
        cwd: config.cwd.clone(),
        workspace_roots: config.workspace_roots.clone(),
        codex_home: config.codex_home.clone(),
        thread_name: None,
        environments: default_environments,
        original_config_do_not_use: Arc::clone(&config),
        metrics_service_name: None,
        terminal_type: "test-terminal".to_string(),
        app_server_client_name: None,
        app_server_client_version: None,
        session_source: SessionSource::Exec,
        thread_source: None,
        dynamic_tools,
        persist_extended_history: false,
        inherited_shell_snapshot: None,
        user_shell_override: None,
    };
    let per_turn_config =
        Session::build_per_turn_config(&session_configuration, session_configuration.cwd.clone());
    let model_info = construct_model_info_offline_for_tests(
        session_configuration.collaboration_mode.model(),
        &per_turn_config.to_models_manager_config(),
    );
    let session_telemetry = Arc::new(session_telemetry(
        thread_id,
        config.as_ref(),
        &model_info,
        session_configuration.session_source.clone(),
    )) as session_telemetry_api::SharedSessionTelemetry;

    let state = SessionState::new(session_configuration.clone());
    let plugins_manager = Arc::new(PluginsManager::new(config.codex_home.to_path_buf()));
    let skill_service = Arc::new(SkillService::new(
        config.codex_home.clone(),
        /*bundled_skills_enabled*/ true,
    ));
    let network_approval: Arc<dyn SessionNetworkApprovalApi> =
        Arc::new(NetworkApprovalService::default());
    let environment: Arc<dyn exec_server_api::ExecEnvironment> = Arc::new(
        codex_exec_server::Environment::create_for_tests(/*exec_server_url*/ None)
            .expect("create environment"),
    );
    let command_service_state = Arc::new(command_service::CommandSessionState::new(
        config.background_terminal_max_timeout,
    ));
    let session_extension_data =
        codex_extension_api::ExtensionData::new(agent_control.session_id().to_string());
    session_extension_data.insert(command_service_state.manager_handle());
    let thread_extension_data = codex_extension_api::ExtensionData::new(thread_id.to_string());
    let provider_auth_manager =
        codex_login::model_provider_auth_manager(Some(Arc::clone(&auth_manager)));
    let model_provider_factory = crate::test_support::model_provider_factory_for_tests();
    let api_runtime_factory: SharedApiRuntimeFactory =
        Arc::new(model_service::DefaultApiRuntimeFactory);
    let model_service: SharedModelServiceApi =
        Arc::new(ModelService::from_runtime_deps(ModelServiceRuntimeDeps {
            codex_home: config.codex_home.to_path_buf(),
            config_model_catalog: config.model_catalog.clone(),
            api_runtime_factory: Arc::clone(&api_runtime_factory),
            provider_auth_manager: provider_auth_manager.clone(),
            model_provider_factory: Arc::clone(&model_provider_factory),
            default_provider: Some(session_configuration.provider.clone()),
            providers_by_id: config.model_providers.clone(),
            model_metadata_overrides: config.to_models_manager_config().model_metadata_overrides,
            attestation_provider: None,
        }));
    let model_client_api = model_service
        .create_client(CreateModelClientRequest {
            selection: ModelSelectionPolicy {
                requested_model: Some(session_configuration.collaboration_mode.model().to_string()),
                provider_hint: Some(config.model_provider_id.clone()),
                allow_default_fallback: true,
                refresh: ModelCatalogRefresh::OnlineIfUncached,
            },
            installation_id: "11111111-1111-4111-8111-111111111111".to_string(),
            session_id: thread_id.into(),
            thread_id,
            session_source: session_configuration.session_source.clone(),
            reasoning_effort: session_configuration.collaboration_mode.reasoning_effort(),
            service_tier: crate::session::turn::model_service_tier(
                session_configuration.service_tier.as_deref(),
            ),
            verbosity: config.model_verbosity,
            chat_completions_max_tokens_by_model: config
                .model_options
                .iter()
                .filter(|model_option| model_option.provider == config.model_provider_id)
                .filter_map(|model_option| {
                    model_option
                        .max_tokens
                        .map(|max_tokens| (model_option.model.clone(), max_tokens))
                })
                .collect(),
            enable_request_compression: config.features.enabled(Feature::EnableRequestCompression),
            include_timing_metrics: config.features.enabled(Feature::RuntimeMetrics),
            beta_features_header: Session::build_model_client_beta_features_header(config.as_ref()),
        })
        .await
        .expect("create model client api for tests");

    let services = SessionServices {
        mcp_connection_manager: Arc::new(RwLock::new(Box::new(
            mcp_service::McpConnectionManager::new_uninitialized_with_permission_profile(
                &config.permissions.approval_policy,
                config.permissions.permission_profile(),
            ),
        ))),
        mcp_auth_runtime: Arc::new(mcp_service::DefaultMcpAuthRuntime),
        mcp_connection_runtime_factory: Arc::new(mcp_service::DefaultMcpConnectionRuntimeFactory),
        network_proxy_runtime_factory: Arc::new(
            codex_network_proxy::DefaultNetworkProxyRuntimeFactory,
        ),
        mcp_startup_cancellation_token: Mutex::new(CancellationToken::new()),
        command_service_state,
        command_service_api: Arc::new(command_service::CommandService::new()),
        shell_zsh_path: None,
        main_execve_wrapper_exe: config.main_execve_wrapper_exe.clone(),
        analytics_events_client: AnalyticsEventsClient::disabled(),
        hooks: std::sync::RwLock::new(Arc::new(Hooks::new(HooksConfig {
            legacy_notify_argv: config.notify.clone(),
            ..HooksConfig::default()
        })) as Arc<dyn hooks_api::HookRuntime>),
        hook_runtime_factory: Arc::new(hooks::HooksRuntimeFactory),
        rollout_thread_trace: rollout_trace::ThreadTraceContext::disabled(),
        user_shell: Arc::new(default_user_shell()),
        shell_snapshot_tx: watch::channel(None).0,
        show_raw_agent_reasoning: config.show_raw_agent_reasoning,
        exec_policy,
        exec_policy_loader: Arc::new(crate::EmptyExecPolicyLoader),
        auth_runtime: auth_manager.clone(),
        provider_auth_manager,
        model_provider_factory,
        api_runtime_factory,
        session_telemetry_factory: Arc::new(codex_otel::OtelSessionTelemetryFactory),
        memory_tool_developer_instructions_provider: Arc::new(
            memory_service_api::DisabledMemoryToolDeveloperInstructionsProvider,
        ),
        model_service,
        sandbox_runtime: Arc::new(codex_sandboxing_api::DisabledSandboxRuntime),
        session_telemetry: session_telemetry.clone(),
        tool_approvals: Mutex::new(ApprovalStore::default()),
        guardian_rejections: Mutex::new(std::collections::HashMap::new()),
        guardian_rejection_circuit_breaker: Mutex::new(Default::default()),
        runtime_handle: tokio::runtime::Handle::current(),
        skill_service,
        plugins_manager,
        mcp_service: Arc::new(mcp_service::McpService::new(Arc::new(
            approval_service::ApprovalService,
        ))),
        extensions: Arc::new(codex_extension_api::ExtensionRegistryBuilder::new().build()),
        session_extension_data,
        thread_extension_data,
        agent_control,
        network_proxy: None,
        network_approval: Arc::clone(&network_approval),
        state_db: state_db.clone(),
        live_thread: None,
        thread_store: Arc::new(thread_store::LocalThreadStore::new(
            thread_store::LocalThreadStoreConfig::from_config(config.as_ref()),
            state_db,
        )),
        live_thread_factory: Arc::new(thread_store::DefaultLiveThreadFactory),
        attestation_provider: None,
        active_event_subscriptions: Arc::new(crate::ActiveEventSubscriptionTracker::default()),
        model_client_api,
        openai_file_uploader: Arc::new(codex_openai_files_api::DisabledOpenAiFileUploader),
        code_mode_service: Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeService),
        code_mode_runtime_factory: Arc::new(codex_code_mode_api::DisabledCodeModeRuntimeFactory),
        approval_service: Arc::new(approval_service::ApprovalService),
        goal_service: Arc::new(goal_service::GoalService),
        tool_service: Arc::new(DisabledToolServiceForTests),
        environment_manager: Arc::new(codex_exec_server::EnvironmentManager::default_for_tests()),
    };

    let effective_skill_roots = services
        .plugins_manager
        .effective_skill_roots_for_config(&per_turn_config.plugins_config_input())
        .await;
    let skills_input =
        crate::build_skill_service_input_from_config(&per_turn_config, effective_skill_roots);
    let skill_fs = environment.get_filesystem();
    let skills_outcome = Arc::new(
        services
            .skill_service
            .skills_for_config(&skills_input, Some(Arc::clone(&skill_fs)))
            .await,
    );
    let available_models = models_manager
        .try_list_models()
        .expect("available models for tests");
    let turn_environments = turn_environments_for_tests(&environment, &session_configuration.cwd);
    let auth_runtime: codex_auth_types::SharedAuthRuntime = auth_manager.clone();
    let turn_context = Session::make_turn_context(
        thread_id,
        SessionId::from(thread_id),
        Some(auth_runtime),
        codex_login::model_provider_auth_manager(Some(Arc::clone(&auth_manager))),
        services.model_provider_factory.as_ref(),
        &session_telemetry,
        session_configuration.provider.clone(),
        &session_configuration,
        services.user_shell.as_ref(),
        services.shell_zsh_path.as_ref(),
        services.main_execve_wrapper_exe.as_ref(),
        per_turn_config,
        model_info,
        &available_models,
        /*network*/ None,
        turn_environments,
        session_configuration.cwd.clone(),
        "turn_id".to_string(),
        skills_outcome,
        /*goal_tools_supported*/ true,
    );

    let (mailbox, mailbox_rx) = crate::Mailbox::new();
    let session = Arc::new(Session {
        self_weak: std::sync::OnceLock::new(),
        conversation_id: thread_id,
        installation_id: "11111111-1111-4111-8111-111111111111".to_string(),
        tx_event,
        agent_status: agent_status_tx,
        out_of_band_elicitation_paused: watch::channel(false).0,
        state: Mutex::new(state),
        managed_network_proxy_refresh_lock: Semaphore::new(/*permits*/ 1),
        features: config.features.clone(),
        pending_mcp_server_refresh_config: Mutex::new(None),
        conversation: Arc::new(RealtimeConversationManager::new()),
        active_turn: Mutex::new(None),
        mailbox,
        mailbox_rx: Mutex::new(mailbox_rx),
        idle_pending_input: Mutex::new(Vec::new()),
        goal_runtime: codex_agent_runtime::GoalRuntimeState::new(),
        guardian_review_session: crate::session::session::approval_review_session_impl::GuardianReviewSessionManager::default(),
        services,
        next_internal_sub_id: AtomicU64::new(0),
        child_completion: codex_agent_runtime::ChildCompletionState::new(),
        wait_agent_backoff: Mutex::new(std::collections::HashMap::new()),
    });
    let _ = session.self_weak.set(Arc::downgrade(&session));
    let mut turn_context = turn_context;
    turn_context.session = Arc::downgrade(&session);
    let turn_context = Arc::new(turn_context);

    (session, turn_context, rx_event)
}

pub(crate) async fn make_session_and_context_with_dynamic_tools_and_rx(
    dynamic_tools: Vec<DynamicToolSpec>,
) -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
) {
    make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        dynamic_tools,
        |_config| {},
    )
    .await
}

async fn make_goal_session_and_context_with_rx() -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
    tempfile::TempDir,
) {
    let codex_home = tempfile::tempdir().expect("create temp dir");
    let (session, turn_context, rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config
                .features
                .enable(Feature::Goals)
                .expect("goal mode should be enableable in tests");
        },
    )
    .await;
    upsert_goal_test_thread(session.as_ref()).await;
    (session, turn_context, rx, codex_home)
}

async fn upsert_goal_test_thread(session: &Session) {
    let config = session.get_config().await;
    let state_db = session
        .state_db()
        .expect("goal test session should have a state db");
    let mut builder = state::ThreadMetadataBuilder::new(
        session.conversation_id,
        config
            .codex_home
            .join("goal-test-rollout.jsonl")
            .to_path_buf(),
        chrono::Utc::now(),
        SessionSource::Cli,
    );
    builder.cwd = config.cwd.to_path_buf();
    builder.model_provider = Some(config.model_provider_id.clone());
    let metadata = builder.build(config.model_provider_id.as_str());
    state_db
        .upsert_thread(&metadata)
        .await
        .expect("goal test thread should be upserted");
}

// Like make_session_and_context, but returns Arc<Session> and the event receiver
// so tests can assert on emitted events.
pub(crate) async fn make_session_and_context_with_rx() -> (
    Arc<Session>,
    Arc<TurnContext>,
    async_channel::Receiver<Event>,
) {
    make_session_and_context_with_dynamic_tools_and_rx(Vec::new()).await
}

#[tokio::test]
async fn refresh_mcp_servers_is_deferred_until_next_turn() {
    let (session, turn_context) = make_session_and_context().await;
    let old_token = session.mcp_startup_cancellation_token().await;
    assert!(!old_token.is_cancelled());

    let mcp_oauth_credentials_store_mode =
        serde_json::to_value(OAuthCredentialsStoreMode::Auto).expect("serialize store mode");
    let refresh_config = McpServerRefreshConfig {
        mcp_servers: json!({}),
        mcp_oauth_credentials_store_mode,
    };
    {
        let mut guard = session.pending_mcp_server_refresh_config.lock().await;
        *guard = Some(refresh_config);
    }

    assert!(!old_token.is_cancelled());
    assert!(
        session
            .pending_mcp_server_refresh_config
            .lock()
            .await
            .is_some()
    );

    session
        .refresh_mcp_servers_if_requested(&turn_context, /*elicitation_reviewer*/ None)
        .await;

    assert!(old_token.is_cancelled());
    assert!(
        session
            .pending_mcp_server_refresh_config
            .lock()
            .await
            .is_none()
    );
    let new_token = session.mcp_startup_cancellation_token().await;
    assert!(!new_token.is_cancelled());
}

#[tokio::test]
async fn spawn_task_does_not_update_previous_turn_settings_for_non_run_turn_tasks() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    sess.set_previous_turn_settings(/*previous_turn_settings*/ None)
        .await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];

    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;
    assert_eq!(sess.previous_turn_settings().await, None);
}

#[tokio::test]
async fn build_settings_update_items_emits_environment_item_for_network_changes() {
    let (session, previous_context) = make_session_and_context().await;
    let previous_context = Arc::new(previous_context);
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;

    let mut config = (*current_context.config).clone();
    let mut requirements = config.config_layer_stack.requirements().clone();
    requirements.network = Some(Sourced::new(
        NetworkConstraints {
            domains: Some(NetworkDomainPermissionsToml {
                entries: std::collections::BTreeMap::from([
                    (
                        "api.example.com".to_string(),
                        NetworkDomainPermissionToml::Allow,
                    ),
                    (
                        "blocked.example.com".to_string(),
                        NetworkDomainPermissionToml::Deny,
                    ),
                ]),
            }),
            ..Default::default()
        },
        RequirementSource::CloudRequirements,
    ));
    let layers = config
        .config_layer_stack
        .get_layers(
            ConfigLayerStackOrdering::LowestPrecedenceFirst,
            /*include_disabled*/ true,
        )
        .into_iter()
        .cloned()
        .collect();
    config.config_layer_stack = ConfigLayerStack::new(
        layers,
        requirements,
        config.config_layer_stack.requirements_toml().clone(),
    )
    .expect("rebuild config layer stack with network requirements");
    current_context.config = Arc::new(config);

    let reference_context_item = previous_context.to_turn_context_item();
    let update_items = session
        .build_settings_update_items(Some(&reference_context_item), &current_context)
        .await;

    let environment_update = user_input_texts(&update_items)
        .into_iter()
        .find(|text| text.contains("<environment_context>"))
        .expect("environment update item should be emitted");
    assert!(environment_update.contains(
        "<network enabled=\"true\"><allowed>api.example.com</allowed><denied>blocked.example.com</denied></network>"
    ));
}

#[tokio::test]
async fn environment_context_uses_session_shell_when_environment_shell_is_absent() {
    let (mut session, mut turn_context) = make_session_and_context().await;
    session.services.user_shell = Arc::new(crate::runtime_shell_model::Shell {
        shell_type: crate::runtime_shell_model::ShellType::PowerShell,
        shell_path: PathBuf::from("powershell"),
        shell_snapshot: crate::runtime_shell_model::empty_shell_snapshot_receiver(),
    });
    for environment in &mut turn_context.environments.turn_environments {
        environment.shell = None;
    }

    let session_shell = session.user_shell();
    let environment_context = crate::context::environment_context_from_turn_context(
        &turn_context,
        session_shell.as_ref(),
    )
    .render();
    assert!(
        environment_context.contains("<shell>powershell</shell>"),
        "{environment_context}"
    );

    let primary_environment = turn_context
        .environments
        .turn_environments
        .first_mut()
        .expect("primary environment");
    primary_environment.shell = Some("cmd".to_string());

    let environment_context = crate::context::environment_context_from_turn_context(
        &turn_context,
        session_shell.as_ref(),
    )
    .render();
    assert!(
        environment_context.contains("<shell>cmd</shell>"),
        "{environment_context}"
    );
}

#[tokio::test]
async fn build_settings_update_items_emits_environment_item_for_time_changes() {
    let (session, previous_context) = make_session_and_context().await;
    let previous_context = Arc::new(previous_context);
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    current_context.current_date = Some("2026-02-27".to_string());
    current_context.timezone = Some("Europe/Berlin".to_string());

    let reference_context_item = previous_context.to_turn_context_item();
    let update_items = session
        .build_settings_update_items(Some(&reference_context_item), &current_context)
        .await;

    let environment_update = user_input_texts(&update_items)
        .into_iter()
        .find(|text| text.contains("<environment_context>"))
        .expect("environment update item should be emitted");
    assert!(environment_update.contains("<current_date>2026-02-27</current_date>"));
    assert!(environment_update.contains("<timezone>Europe/Berlin</timezone>"));
}

#[tokio::test]
async fn build_settings_update_items_omits_environment_item_when_disabled() {
    let (session, previous_context) = make_session_and_context().await;
    let previous_context = Arc::new(previous_context);
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    let mut config = (*current_context.config).clone();
    config.include_environment_context = false;
    current_context.config = Arc::new(config);
    current_context.current_date = Some("2026-02-27".to_string());

    let reference_context_item = previous_context.to_turn_context_item();
    let update_items = session
        .build_settings_update_items(Some(&reference_context_item), &current_context)
        .await;

    let user_texts = user_input_texts(&update_items);
    assert!(
        !user_texts
            .iter()
            .any(|text| text.contains("<environment_context>")),
        "did not expect environment context updates when disabled, got {user_texts:?}"
    );
}

#[tokio::test]
async fn build_settings_update_items_emits_realtime_start_when_session_becomes_live() {
    let (session, previous_context) = make_session_and_context().await;
    let previous_context = Arc::new(previous_context);
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    current_context.realtime_active = true;

    let update_items = session
        .build_settings_update_items(
            Some(&previous_context.to_turn_context_item()),
            &current_context,
        )
        .await;

    let developer_texts = developer_input_texts(&update_items);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<realtime_conversation>")),
        "expected a realtime start update, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_settings_update_items_emits_realtime_end_when_session_stops_being_live() {
    let (session, mut previous_context) = make_session_and_context().await;
    previous_context.realtime_active = true;
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    current_context.realtime_active = false;

    let update_items = session
        .build_settings_update_items(
            Some(&previous_context.to_turn_context_item()),
            &current_context,
        )
        .await;

    let developer_texts = developer_input_texts(&update_items);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("Reason: inactive")),
        "expected a realtime end update, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_settings_update_items_uses_previous_turn_settings_for_realtime_end() {
    let (session, previous_context) = make_session_and_context().await;
    let mut previous_context_item = previous_context.to_turn_context_item();
    previous_context_item.realtime_active = None;
    let previous_turn_settings = PreviousTurnSettings {
        model: previous_context.model_info.slug.clone(),
        realtime_active: Some(true),
    };
    let mut current_context = previous_context
        .with_model(
            previous_context.model_info.slug.clone(),
            &session.services.model_service,
        )
        .await;
    current_context.realtime_active = false;

    session
        .set_previous_turn_settings(Some(previous_turn_settings))
        .await;
    let update_items = session
        .build_settings_update_items(Some(&previous_context_item), &current_context)
        .await;

    let developer_texts = developer_input_texts(&update_items);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("Reason: inactive")),
        "expected a realtime end update from previous turn settings, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_uses_previous_realtime_state() {
    let (session, mut turn_context) = make_session_and_context().await;
    turn_context.realtime_active = true;

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<realtime_conversation>")),
        "expected initial context to describe active realtime state, got {developer_texts:?}"
    );

    let previous_context_item = turn_context.to_turn_context_item();
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(Some(previous_context_item));
    }
    let resumed_context = session.build_initial_context(&turn_context).await;
    let resumed_developer_texts = developer_input_texts(&resumed_context);
    assert!(
        !resumed_developer_texts
            .iter()
            .any(|text| text.contains("<realtime_conversation>")),
        "did not expect a duplicate realtime update, got {resumed_developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_emits_standalone_multiagent_context() {
    let (session, turn_context) = make_session_and_context().await;

    let initial_context = session.build_initial_context(&turn_context).await;
    let user_texts = user_input_texts(&initial_context);
    let environment_context = user_texts
        .iter()
        .find(|text| text.contains("<environment_context>"))
        .expect("expected environment context");
    let multiagent_context = user_texts
        .iter()
        .find(|text| text.contains("<multiagent_context>"))
        .expect("expected multiagent context");

    assert!(
        !environment_context.contains("<subagents>"),
        "did not expect subagents in environment context, got {environment_context}"
    );
    assert!(
        multiagent_context
            .contains("<current_thread_canonical_path>/root</current_thread_canonical_path>"),
        "expected root canonical path in multiagent context, got {multiagent_context}"
    );
}

async fn make_multi_agent_v2_usage_hint_test_session() -> (Arc<Session>, Arc<TurnContext>) {
    let (session, turn_context, _rx_event) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        |config| {
            config.multi_agent_v2.root_agent_usage_hint_text = Some("Root guidance.".to_string());
            config.multi_agent_v2.subagent_usage_hint_text = Some("Subagent guidance.".to_string());
        },
    )
    .await;
    (session, turn_context)
}

struct PromptExtensionTestContributor;
struct PromptExtensionTestState;

impl codex_extension_api::ContextContributor for PromptExtensionTestContributor {
    fn contribute<'a>(
        &'a self,
        _session_store: &'a codex_extension_api::ExtensionData,
        thread_store: &'a codex_extension_api::ExtensionData,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Vec<codex_extension_api::PromptFragment>> + Send + 'a>,
    > {
        Box::pin(async move {
            thread_store
                .get::<PromptExtensionTestState>()
                .is_some()
                .then(|| {
                    codex_extension_api::PromptFragment::developer_policy(
                        "prompt extension enabled",
                    )
                })
                .into_iter()
                .collect()
        })
    }
}

fn prompt_extension_test_registry()
-> Arc<codex_extension_api::ExtensionRegistry<crate::config::Config>> {
    let mut builder = codex_extension_api::ExtensionRegistryBuilder::new();
    builder.prompt_contributor(Arc::new(PromptExtensionTestContributor));
    Arc::new(builder.build())
}

#[tokio::test]
async fn build_initial_context_includes_prompt_fragments_from_extensions() {
    let (mut session, turn_context) = make_session_and_context().await;
    session.services.extensions = prompt_extension_test_registry();
    session
        .services
        .thread_extension_data
        .insert(PromptExtensionTestState);

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_messages = developer_message_texts(&initial_context);

    assert!(
        developer_messages
            .iter()
            .flatten()
            .any(|text| *text == "prompt extension enabled"),
        "expected prompt extension developer text, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn build_initial_context_omits_prompt_fragments_without_extension_state() {
    let (mut session, turn_context) = make_session_and_context().await;
    session.services.extensions = prompt_extension_test_registry();

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_messages = developer_message_texts(&initial_context);

    assert!(
        !developer_messages
            .iter()
            .flatten()
            .any(|text| *text == "prompt extension enabled"),
        "did not expect prompt extension developer text, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn build_initial_context_adds_multi_agent_v2_root_usage_hint_as_developer_message() {
    let (session, turn_context) = make_multi_agent_v2_usage_hint_test_session().await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;

    let developer_messages = developer_message_texts(&initial_context);
    assert!(
        developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Root guidance."]),
        "expected standalone root usage hint developer message, got {developer_messages:?}"
    );
    assert!(
        !developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Subagent guidance."]),
        "did not expect subagent usage hint for root thread, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn build_initial_context_adds_multi_agent_v2_subagent_usage_hint_as_developer_message() {
    let (session, mut turn_context) = make_multi_agent_v2_usage_hint_test_session().await;
    let session_source = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: ThreadId::new(),
        depth: 1,
        agent_path: Some(AgentPath::try_from("/root/worker").expect("agent path should parse")),
        agent_nickname: Some("worker".to_string()),
        agent_role: None,
    });
    session
        .state
        .lock()
        .await
        .session_configuration
        .session_source = session_source.clone();
    Arc::get_mut(&mut turn_context)
        .expect("turn context should not be shared")
        .session_source = session_source;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;

    let developer_messages = developer_message_texts(&initial_context);
    assert!(
        developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Subagent guidance."]),
        "expected standalone subagent usage hint developer message, got {developer_messages:?}"
    );
    assert!(
        !developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Root guidance."]),
        "did not expect root usage hint for subagent thread, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn build_initial_context_adds_multi_agent_v2_usage_hints_when_feature_disabled() {
    let (session, turn_context) = make_multi_agent_v2_usage_hint_test_session().await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;

    let developer_messages = developer_message_texts(&initial_context);
    assert!(
        developer_messages
            .iter()
            .any(|message| message.as_slice() == ["Root guidance."]),
        "expected root usage hint even when legacy feature is disabled, got {developer_messages:?}"
    );
}

#[tokio::test]
async fn configured_multi_agent_v2_usage_hint_texts_returns_configured_texts() {
    let (session, _turn_context) = make_multi_agent_v2_usage_hint_test_session().await;

    let hint_texts = session.configured_multi_agent_v2_usage_hint_texts().await;

    assert_eq!(
        hint_texts,
        vec![
            "Root guidance.".to_string(),
            "Subagent guidance.".to_string()
        ]
    );
}

#[tokio::test]
async fn build_initial_context_omits_default_image_save_location_with_image_history() {
    let (session, turn_context) = make_session_and_context().await;
    session
        .replace_history(
            vec![ResponseItem::ImageGenerationCall {
                id: "ig-test".to_string(),
                status: "completed".to_string(),
                revised_prompt: Some("a tiny blue square".to_string()),
                result: "Zm9v".to_string(),
            }],
            /*reference_context_item*/ None,
        )
        .await;

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);
    assert!(
        !developer_texts
            .iter()
            .any(|text| text.contains("Generated images are saved to")),
        "expected initial context to omit image save instructions even with image history, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_omits_default_image_save_location_without_image_history() {
    let (session, turn_context) = make_session_and_context().await;

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        !developer_texts
            .iter()
            .any(|text| text.contains("Generated images are saved to")),
        "expected initial context to omit image save instructions without image history, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_trims_skill_metadata_from_context_window_budget() {
    let (session, mut turn_context) = make_session_and_context().await;
    let mut outcome = SkillLoadOutcome::default();
    outcome.skills = vec![
        SkillMetadata {
            name: "admin-skill".to_string(),
            description: "desc".to_string(),
            short_description: None,
            interface: None,
            dependencies: None,
            policy: None,
            path_to_skills_md: test_path_buf("/tmp/admin-skill/SKILL.md").abs(),
            scope: SkillScope::Admin,
            plugin_id: None,
        },
        SkillMetadata {
            name: "repo-skill".to_string(),
            description: "desc".to_string(),
            short_description: None,
            interface: None,
            dependencies: None,
            policy: None,
            path_to_skills_md: test_path_buf("/tmp/repo-skill/SKILL.md").abs(),
            scope: SkillScope::Repo,
            plugin_id: None,
        },
    ];
    turn_context.model_info.context_window = Some(100);
    turn_context.turn_skills = TurnSkillsContext::new(Arc::new(outcome));

    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .all(|text| !text.contains("Exceeded skills context budget")),
        "expected skill budget warning to stay out of the initial context, got {developer_texts:?}"
    );
    assert!(
        developer_texts
            .iter()
            .all(|text| !text.contains("- admin-skill:") && !text.contains("- repo-skill:")),
        "expected no skill metadata entries to fit the tiny budget, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_loads_skills_from_current_cwd_local_roots() {
    fn write_skill(root: &Path, dir: &str, name: &str, description: &str) {
        let skill_dir = root.join(dir);
        std::fs::create_dir_all(&skill_dir).expect("create skill dir");
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\n# Body\n"),
        )
        .expect("write skill");
    }

    let codex_home = tempfile::tempdir().expect("create codex home");
    let child_cwd = codex_home.path().join("child");
    std::fs::create_dir_all(&child_cwd).expect("create child cwd");
    write_skill(
        &child_cwd.join(".codex/skills"),
        "cwd-dot-codex",
        "cwd-dot-codex-skill",
        "from cwd .codex",
    );
    write_skill(
        &child_cwd.join(".agents/skills"),
        "cwd-dot-agents",
        "cwd-dot-agents-skill",
        "from cwd .agents",
    );

    let (session, turn_context, _rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config.cwd = child_cwd.abs();
        },
    )
    .await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("- cwd-dot-codex-skill:")),
        "expected cwd .codex skill in initial context, got {developer_texts:?}"
    );
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("- cwd-dot-agents-skill:")),
        "expected cwd .agents skill in initial context, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_loads_project_workflows() {
    fn write_workflow(root: &Path, id: &str, description: &str) {
        let workflow_dir = root.join(id);
        std::fs::create_dir_all(&workflow_dir).expect("create workflow dir");
        std::fs::write(workflow_dir.join("workflow.ts"), "export default {};")
            .expect("write workflow entry");
        std::fs::write(
            workflow_dir.join("WORKFLOW.md"),
            format!(
                r#"---
id: {id}
name: Feature Development
description: {description}
entry: workflow.ts
when_to_use:
  - feature work
inputs:
  objective:
    type: string
    description: Goal
---
Use this workflow when feature work needs a structured process.
"#
            ),
        )
        .expect("write workflow markdown");
    }

    let codex_home = tempfile::tempdir().expect("create codex home");
    let repo_cwd = codex_home.path().join("repo");
    std::fs::create_dir_all(&repo_cwd).expect("create repo cwd");
    write_workflow(
        &repo_cwd.join(".codex/workflows"),
        "feature-dev",
        "structured feature workflow",
    );

    let (session, turn_context, _rx) = make_session_and_context_with_auth_config_home_and_rx(
        CodexAuth::from_api_key("Test API Key"),
        Vec::new(),
        codex_home.path(),
        |config| {
            config.cwd = repo_cwd.abs();
        },
    )
    .await;

    let initial_context = session.build_initial_context(turn_context.as_ref()).await;
    let developer_texts = developer_input_texts(&initial_context);

    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<workflows_instructions>")
                && text.contains("- feature-dev (project)")
                && text.contains("structured feature workflow")
                && text
                    .contains("Use this workflow when feature work needs a structured process.")),
        "expected project workflow in initial context, got {developer_texts:?}"
    );
}

#[test]
fn emit_thread_start_skill_metrics_records_enabled_kept_and_truncated_values() {
    let session_telemetry = test_session_telemetry_without_metadata();
    let mut outcome = SkillLoadOutcome::default();
    outcome.skills = vec![SkillMetadata {
        name: "repo-skill".to_string(),
        description: "desc".to_string(),
        short_description: None,
        interface: None,
        dependencies: None,
        policy: None,
        path_to_skills_md: test_path_buf("/tmp/repo-skill/SKILL.md").abs(),
        scope: SkillScope::Repo,
        plugin_id: None,
    }];
    let rendered = build_available_skills(
        &outcome,
        SkillMetadataBudget::Characters(1),
        SkillRenderSideEffects::ThreadStart {
            session_telemetry: &session_telemetry,
        },
    )
    .expect("skills should render");

    assert_eq!(
        rendered.warning_message,
        Some(
            "Exceeded skills context budget. All skill descriptions were removed and 1 additional skill was not included in the model-visible skills list."
                .to_string()
        )
    );
    let snapshot = session_telemetry
        .snapshot_metrics()
        .expect("runtime metrics snapshot");
    assert_eq!(
        histogram_sum(&snapshot, THREAD_SKILLS_ENABLED_TOTAL_METRIC),
        1
    );
    assert_eq!(histogram_sum(&snapshot, THREAD_SKILLS_KEPT_TOTAL_METRIC), 0);
    assert_eq!(histogram_sum(&snapshot, THREAD_SKILLS_TRUNCATED_METRIC), 1);
    assert_eq!(
        histogram_sum(&snapshot, THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC),
        4
    );
}

#[test]
fn emit_thread_start_skill_metrics_records_description_truncated_chars_without_omitted_skills() {
    let session_telemetry = test_session_telemetry_without_metadata();
    let alpha = SkillMetadata {
        name: "alpha-skill".to_string(),
        description: "abcdef".to_string(),
        short_description: None,
        interface: None,
        dependencies: None,
        policy: None,
        path_to_skills_md: test_path_buf("/tmp/alpha-skill/SKILL.md").abs(),
        scope: SkillScope::Repo,
        plugin_id: None,
    };
    let beta = SkillMetadata {
        name: "beta-skill".to_string(),
        description: "uvwxyz".to_string(),
        short_description: None,
        interface: None,
        dependencies: None,
        policy: None,
        path_to_skills_md: test_path_buf("/tmp/beta-skill/SKILL.md").abs(),
        scope: SkillScope::Repo,
        plugin_id: None,
    };
    let minimum_skill_line_cost = |skill: &SkillMetadata| {
        let path = skill.path_to_skills_md.to_string_lossy().replace('\\', "/");
        format!("- {}: (file: {})\n", skill.name, path)
            .chars()
            .count()
    };
    let minimum_budget = minimum_skill_line_cost(&alpha) + minimum_skill_line_cost(&beta);
    let mut outcome = SkillLoadOutcome::default();
    outcome.skills = vec![alpha, beta];

    let rendered = build_available_skills(
        &outcome,
        SkillMetadataBudget::Characters(minimum_budget + 6),
        SkillRenderSideEffects::ThreadStart {
            session_telemetry: &session_telemetry,
        },
    )
    .expect("skills should render");

    assert_eq!(rendered.report.omitted_count, 0);
    assert_eq!(rendered.report.truncated_description_chars, 8);
    let snapshot = session_telemetry
        .snapshot_metrics()
        .expect("runtime metrics snapshot");
    assert_eq!(histogram_sum(&snapshot, THREAD_SKILLS_TRUNCATED_METRIC), 0);
    assert_eq!(
        histogram_sum(&snapshot, THREAD_SKILLS_DESCRIPTION_TRUNCATED_CHARS_METRIC),
        8
    );
}

#[tokio::test]
async fn build_initial_context_emits_thread_start_skill_warning_on_repeated_builds() {
    let (session, turn_context, rx) = make_session_and_context_with_rx().await;
    let mut turn_context = Arc::into_inner(turn_context).expect("sole turn context owner");
    let mut outcome = SkillLoadOutcome::default();
    outcome.skills = vec![
        SkillMetadata {
            name: "admin-skill".to_string(),
            description: "desc".to_string(),
            short_description: None,
            interface: None,
            dependencies: None,
            policy: None,
            path_to_skills_md: test_path_buf("/tmp/admin-skill/SKILL.md").abs(),
            scope: SkillScope::Admin,
            plugin_id: None,
        },
        SkillMetadata {
            name: "repo-skill".to_string(),
            description: "desc".to_string(),
            short_description: None,
            interface: None,
            dependencies: None,
            policy: None,
            path_to_skills_md: test_path_buf("/tmp/repo-skill/SKILL.md").abs(),
            scope: SkillScope::Repo,
            plugin_id: None,
        },
    ];
    turn_context.model_info.context_window = Some(100);
    turn_context.turn_skills = TurnSkillsContext::new(Arc::new(outcome));

    let _ = session.build_initial_context(&turn_context).await;
    let warning_event = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("warning event should arrive")
        .expect("warning event should be readable");
    assert!(matches!(
        warning_event.msg,
        EventMsg::Warning(WarningEvent { message })
            if message == "Exceeded skills context budget of 2%. All skill descriptions were removed and 2 additional skills were not included in the model-visible skills list."
    ));

    let _ = session.build_initial_context(&turn_context).await;
    let warning_event = timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("warning event should arrive on repeated build")
        .expect("warning event should be readable");
    assert!(matches!(
        warning_event.msg,
        EventMsg::Warning(WarningEvent { message })
            if message == "Exceeded skills context budget of 2%. All skill descriptions were removed and 2 additional skills were not included in the model-visible skills list."
    ));
}

#[tokio::test]
async fn handle_output_item_done_records_image_save_history_message() {
    let (session, turn_context) = make_session_and_context().await;
    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "ig_history_records_message";
    let expected_saved_path = crate::stream_events_utils::image_generation_artifact_path(
        &turn_context.config.codex_home,
        &session.conversation_id.to_string(),
        call_id,
    );
    let _ = std::fs::remove_file(&expected_saved_path);
    let item = ResponseItem::ImageGenerationCall {
        id: call_id.to_string(),
        status: "completed".to_string(),
        revised_prompt: Some("a tiny blue square".to_string()),
        result: "Zm9v".to_string(),
    };

    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&session),
        turn_context: Arc::clone(&turn_context),
        turn_store: Arc::new(codex_extension_api::ExtensionData::new(
            turn_context.sub_id.clone(),
        )),
        tool_inputs: test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context)),
        turn_diff_tracker: Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new())),
        cancellation_token: CancellationToken::new(),
    };
    handle_output_item_done(&mut ctx, item.clone(), /*previously_active_item*/ None)
        .await
        .expect("image generation item should succeed");

    let history = session.clone_history().await;
    let image_output_path = crate::stream_events_utils::image_generation_artifact_path(
        &turn_context.config.codex_home,
        &session.conversation_id.to_string(),
        "<image_id>",
    );
    let image_output_dir = image_output_path
        .parent()
        .expect("generated image path should have a parent");
    let image_message: ResponseItem =
        ContextualUserFragment::into(codex_context_manager::ImageGenerationInstructions::new(
            image_output_dir.display(),
            image_output_path.display(),
        ));
    assert_eq!(history.raw_items(), &[image_message, item]);
    assert_eq!(
        std::fs::read(&expected_saved_path).expect("saved file"),
        b"foo"
    );
    let _ = std::fs::remove_file(&expected_saved_path);
}

#[tokio::test]
async fn handle_output_item_done_skips_image_save_message_when_save_fails() {
    let (session, turn_context) = make_session_and_context().await;
    let session = Arc::new(session);
    let turn_context = Arc::new(turn_context);
    let call_id = "ig_history_no_message";
    let expected_saved_path = crate::stream_events_utils::image_generation_artifact_path(
        &turn_context.config.codex_home,
        &session.conversation_id.to_string(),
        call_id,
    );
    let _ = std::fs::remove_file(&expected_saved_path);
    let item = ResponseItem::ImageGenerationCall {
        id: call_id.to_string(),
        status: "completed".to_string(),
        revised_prompt: Some("broken payload".to_string()),
        result: "_-8".to_string(),
    };

    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&session),
        turn_context: Arc::clone(&turn_context),
        turn_store: Arc::new(codex_extension_api::ExtensionData::new(
            turn_context.sub_id.clone(),
        )),
        tool_inputs: test_tool_inputs(Arc::clone(&session), Arc::clone(&turn_context)),
        turn_diff_tracker: Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new())),
        cancellation_token: CancellationToken::new(),
    };
    handle_output_item_done(&mut ctx, item.clone(), /*previously_active_item*/ None)
        .await
        .expect("image generation item should still complete");

    let history = session.clone_history().await;
    assert_eq!(history.raw_items(), &[item]);
    assert!(!expected_saved_path.exists());
}

#[tokio::test]
async fn build_initial_context_uses_previous_turn_settings_for_realtime_end() {
    let (session, turn_context) = make_session_and_context().await;
    let previous_turn_settings = PreviousTurnSettings {
        model: turn_context.model_info.slug.clone(),
        realtime_active: Some(true),
    };

    session
        .set_previous_turn_settings(Some(previous_turn_settings))
        .await;
    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("Reason: inactive")),
        "expected initial context to describe an ended realtime session, got {developer_texts:?}"
    );
}

#[tokio::test]
async fn build_initial_context_restates_realtime_start_when_reference_context_is_missing() {
    let (session, mut turn_context) = make_session_and_context().await;
    turn_context.realtime_active = true;
    let previous_turn_settings = PreviousTurnSettings {
        model: turn_context.model_info.slug.clone(),
        realtime_active: Some(true),
    };

    session
        .set_previous_turn_settings(Some(previous_turn_settings))
        .await;
    let initial_context = session.build_initial_context(&turn_context).await;
    let developer_texts = developer_input_texts(&initial_context);
    assert!(
        developer_texts
            .iter()
            .any(|text| text.contains("<realtime_conversation>")),
        "expected initial context to restate active realtime when the reference context is missing, got {developer_texts:?}"
    );
}

fn file_system_policy_with_unreadable_glob(turn_context: &TurnContext) -> FileSystemSandboxPolicy {
    #[allow(deprecated)]
    let mut policy = FileSystemSandboxPolicy::from_legacy_sandbox_policy_for_cwd(
        &turn_context.sandbox_policy(),
        &turn_context.cwd,
    );
    #[allow(deprecated)]
    let cwd_display = turn_context.cwd.as_path().display().to_string();
    policy.entries.push(FileSystemSandboxEntry {
        path: FileSystemPath::GlobPattern {
            pattern: format!("{cwd_display}/**/*.env"),
        },
        access: FileSystemAccessMode::None,
    });
    policy
}

#[tokio::test]
async fn turn_context_item_omits_legacy_equivalent_file_system_sandbox_policy() {
    let (_session, turn_context) = make_session_and_context().await;

    let item = turn_context.to_turn_context_item();

    assert_eq!(item.file_system_sandbox_policy, None);
    assert_eq!(
        item.permission_profile,
        Some(turn_context.permission_profile())
    );
}

#[tokio::test]
async fn turn_context_item_stores_split_file_system_sandbox_policy_when_different() {
    let (_session, mut turn_context) = make_session_and_context().await;
    let file_system_sandbox_policy = file_system_policy_with_unreadable_glob(&turn_context);
    turn_context.permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
        turn_context.permission_profile.enforcement(),
        &file_system_sandbox_policy,
        turn_context.network_sandbox_policy(),
    );

    let item = turn_context.to_turn_context_item();

    assert_eq!(
        item.file_system_sandbox_policy,
        Some(file_system_sandbox_policy)
    );
    assert_eq!(
        item.permission_profile,
        Some(turn_context.permission_profile())
    );
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_injects_full_context_when_baseline_missing()
 {
    let (session, turn_context) = make_session_and_context().await;
    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    let history = session.clone_history().await;
    let initial_context = session.build_initial_context(&turn_context).await;
    assert_eq!(history.raw_items().to_vec(), initial_context);

    let current_context = session.reference_context_item().await;
    assert_eq!(
        serde_json::to_value(current_context).expect("serialize current context item"),
        serde_json::to_value(Some(turn_context.to_turn_context_item()))
            .expect("serialize expected context item")
    );
}

#[tokio::test]
async fn record_context_updates_emits_injected_context_with_agent_file_instructions() {
    let agent_file_instructions = "Agent type file body: always inspect the active task.";
    let (session, turn_context, rx) = make_session_and_context_with_auth_and_config_and_rx(
        CodexAuth::from_api_key("test-api-key"),
        Vec::new(),
        |config| {
            config.developer_instructions = Some(agent_file_instructions.to_string());
        },
    )
    .await;

    session
        .record_context_updates_and_set_reference_context_item(turn_context.as_ref())
        .await;

    let mut injected_context = None;
    for _ in 0..10 {
        let event = tokio::time::timeout(StdDuration::from_secs(1), rx.recv())
            .await
            .expect("timeout waiting for injected context event")
            .expect("event");
        if let EventMsg::ItemCompleted(ItemCompletedEvent {
            item: TurnItem::InjectedContext(item),
            ..
        }) = event.msg
        {
            injected_context = Some(item);
            break;
        }
    }
    let injected_context = injected_context.expect("expected injected context display item");

    assert_eq!(injected_context.title, "Init Context");
    assert!(
        injected_context
            .sections
            .iter()
            .any(|section| section.label == "Developer"
                && section.text.contains(agent_file_instructions)),
        "expected injected context to include agent file instructions, got {injected_context:?}"
    );
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_reinjects_full_context_after_clear()
{
    let (session, turn_context) = make_session_and_context().await;
    let compacted_summary = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: format!("{}\nsummary", crate::compact::SUMMARY_PREFIX),
        }],
        phase: None,
    };
    session
        .record_into_history(std::slice::from_ref(&compacted_summary), &turn_context)
        .await;
    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(/*item*/ None);
    }
    session
        .replace_history(
            vec![compacted_summary.clone()],
            /*reference_context_item*/ None,
        )
        .await;

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;

    let history = session.clone_history().await;
    let mut expected_history = vec![compacted_summary];
    expected_history.extend(session.build_initial_context(&turn_context).await);
    assert_eq!(history.raw_items().to_vec(), expected_history);
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_persists_baseline_without_emitting_diffs()
 {
    let (mut session, previous_context) = make_session_and_context().await;
    let next_model = if previous_context.model_info.slug == "gpt-5.4" {
        "gpt-5.2"
    } else {
        "gpt-5.4"
    };
    let turn_context = previous_context
        .with_model(next_model.to_string(), &session.services.model_service)
        .await;
    let previous_context_item = previous_context.to_turn_context_item();
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(Some(previous_context_item.clone()));
    }
    let rollout_path = attach_thread_persistence(&mut session).await;

    let update_items = session
        .build_settings_update_items(Some(&previous_context_item), &turn_context)
        .await;
    assert_eq!(update_items, Vec::new());

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;

    assert_eq!(
        session.clone_history().await.raw_items().to_vec(),
        Vec::new()
    );
    assert_eq!(
        serde_json::to_value(session.reference_context_item().await)
            .expect("serialize current context item"),
        serde_json::to_value(Some(turn_context.to_turn_context_item()))
            .expect("serialize expected context item")
    );
    session.ensure_rollout_materialized().await;
    session.flush_rollout().await.expect("rollout should flush");

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    let persisted_turn_context = resumed.history.iter().find_map(|item| match item {
        RolloutItem::TurnContext(ctx) => Some(ctx.clone()),
        _ => None,
    });
    assert_eq!(
        serde_json::to_value(persisted_turn_context)
            .expect("serialize persisted turn context item"),
        serde_json::to_value(Some(turn_context.to_turn_context_item()))
            .expect("serialize expected turn context item")
    );
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_persists_split_file_system_policy_to_rollout()
 {
    let (mut session, mut turn_context) = make_session_and_context().await;
    let file_system_sandbox_policy = file_system_policy_with_unreadable_glob(&turn_context);
    turn_context.permission_profile = PermissionProfile::from_runtime_permissions_with_enforcement(
        turn_context.permission_profile.enforcement(),
        &file_system_sandbox_policy,
        turn_context.network_sandbox_policy(),
    );
    let rollout_path = attach_thread_persistence(&mut session).await;

    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    session.ensure_rollout_materialized().await;
    session.flush_rollout().await.expect("rollout should flush");

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    let persisted_file_system_sandbox_policy = resumed.history.iter().find_map(|item| match item {
        RolloutItem::TurnContext(ctx) => ctx.file_system_sandbox_policy.clone(),
        _ => None,
    });
    assert_eq!(
        persisted_file_system_sandbox_policy,
        Some(file_system_sandbox_policy)
    );
}

#[tokio::test]
async fn build_initial_context_prepends_model_switch_message() {
    let (session, turn_context) = make_session_and_context().await;
    let previous_turn_settings = PreviousTurnSettings {
        model: "previous-regular-model".to_string(),
        realtime_active: None,
    };

    session
        .set_previous_turn_settings(Some(previous_turn_settings))
        .await;
    let initial_context = session.build_initial_context(&turn_context).await;

    let ResponseItem::Message { role, content, .. } = &initial_context[0] else {
        panic!("expected developer message");
    };
    assert_eq!(role, "developer");
    let [ContentItem::InputText { text }, ..] = content.as_slice() else {
        panic!("expected developer text");
    };
    assert!(text.contains("<model_switch>"));
}

#[tokio::test]
async fn record_context_updates_and_set_reference_context_item_persists_full_reinjection_to_rollout()
 {
    let (mut session, previous_context) = make_session_and_context().await;
    let next_model = if previous_context.model_info.slug == "gpt-5.4" {
        "gpt-5.2"
    } else {
        "gpt-5.4"
    };
    let turn_context = previous_context
        .with_model(next_model.to_string(), &session.services.model_service)
        .await;
    let rollout_path = attach_thread_persistence(&mut session).await;

    session
        .persist_rollout_items(&[RolloutItem::EventMsg(EventMsg::UserMessage(
            UserMessageEvent {
                message: "seed rollout".to_string(),
                images: None,
                local_images: Vec::new(),
                skills: Vec::new(),
                text_elements: Vec::new(),
            },
        ))])
        .await;
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(/*item*/ None);
    }

    session
        .set_previous_turn_settings(Some(PreviousTurnSettings {
            model: previous_context.model_info.slug.clone(),
            realtime_active: Some(previous_context.realtime_active),
        }))
        .await;
    session
        .record_context_updates_and_set_reference_context_item(&turn_context)
        .await;
    session.ensure_rollout_materialized().await;
    session.flush_rollout().await.expect("rollout should flush");

    let InitialHistory::Resumed(resumed) = RolloutRecorder::get_rollout_history(&rollout_path)
        .await
        .expect("read rollout history")
    else {
        panic!("expected resumed rollout history");
    };
    let persisted_turn_context = resumed.history.iter().find_map(|item| match item {
        RolloutItem::TurnContext(ctx) => Some(ctx.clone()),
        _ => None,
    });

    assert_eq!(
        serde_json::to_value(persisted_turn_context)
            .expect("serialize persisted turn context item"),
        serde_json::to_value(Some(turn_context.to_turn_context_item()))
            .expect("serialize expected turn context item")
    );
}

#[tokio::test]
async fn run_user_shell_command_does_not_set_reference_context_item() {
    let (session, _turn_context, rx) = make_session_and_context_with_rx().await;
    {
        let mut state = session.state.lock().await;
        state.set_reference_context_item(/*item*/ None);
    }

    handlers::run_user_shell_command(&session, "sub-id".to_string(), "echo shell".to_string())
        .await;

    let deadline = StdDuration::from_secs(15);
    let start = std::time::Instant::now();
    loop {
        let remaining = deadline.saturating_sub(start.elapsed());
        let evt = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("event");
        if matches!(evt.msg, EventMsg::TurnComplete(_)) {
            break;
        }
    }

    assert!(
        session.reference_context_item().await.is_none(),
        "standalone shell tasks should not mutate previous context"
    );
}

#[tokio::test]
async fn realtime_conversation_list_voices_emits_builtin_list() {
    let (session, _turn_context, rx) = make_session_and_context_with_rx().await;

    handlers::realtime_conversation_list_voices(&session, "sub-id".to_string()).await;

    let event = rx.recv().await.expect("event");
    let voices = match event.msg {
        EventMsg::RealtimeConversationListVoicesResponse(
            RealtimeConversationListVoicesResponseEvent { voices },
        ) => voices,
        msg => panic!("expected list voices response, got {msg:?}"),
    };
    assert_eq!(
        voices,
        RealtimeVoicesList {
            v1: vec![
                RealtimeVoice::Juniper,
                RealtimeVoice::Maple,
                RealtimeVoice::Spruce,
                RealtimeVoice::Ember,
                RealtimeVoice::Vale,
                RealtimeVoice::Breeze,
                RealtimeVoice::Arbor,
                RealtimeVoice::Sol,
                RealtimeVoice::Cove,
            ],
            v2: vec![
                RealtimeVoice::Alloy,
                RealtimeVoice::Ash,
                RealtimeVoice::Ballad,
                RealtimeVoice::Coral,
                RealtimeVoice::Echo,
                RealtimeVoice::Sage,
                RealtimeVoice::Shimmer,
                RealtimeVoice::Verse,
                RealtimeVoice::Marin,
                RealtimeVoice::Cedar,
            ],
            default_v1: RealtimeVoice::Cove,
            default_v2: RealtimeVoice::Marin,
        },
    );
}

#[derive(Clone, Copy)]
struct NeverEndingTask {
    kind: TaskKind,
    listen_to_cancellation_token: bool,
}

impl SessionTask for NeverEndingTask {
    fn kind(&self) -> TaskKind {
        self.kind
    }

    fn span_name(&self) -> &'static str {
        "session_task.never_ending"
    }

    async fn run(
        self: Arc<Self>,
        _session: Arc<SessionTaskContext>,
        _ctx: Arc<TurnContext>,
        _input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        if self.listen_to_cancellation_token {
            cancellation_token.cancelled().await;
            return None;
        }
        loop {
            sleep(Duration::from_secs(60)).await;
        }
    }
}

#[derive(Clone, Copy)]
struct GuardianDeniedApprovalTask;

impl SessionTask for GuardianDeniedApprovalTask {
    fn kind(&self) -> TaskKind {
        TaskKind::Regular
    }

    fn span_name(&self) -> &'static str {
        "session_task.guardian_denied_approval"
    }

    async fn run(
        self: Arc<Self>,
        session: Arc<SessionTaskContext>,
        ctx: Arc<TurnContext>,
        _input: Vec<UserInput>,
        cancellation_token: CancellationToken,
    ) -> Option<String> {
        let session = session.clone_session();
        for _ in 0..3 {
            crate::guardian::record_guardian_denial_for_test(&session, &ctx, &ctx.sub_id).await;
        }

        cancellation_token.cancelled().await;
        None
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guardian_auto_review_interrupts_after_three_consecutive_denials() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "trigger guardian denials".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(Arc::clone(&tc), input, GuardianDeniedApprovalTask)
        .await;

    let mut observed = Vec::new();
    let aborted = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::TurnAborted(event) = &event.msg {
                let event = event.clone();
                observed.push(EventMsg::TurnAborted(event.clone()));
                break event;
            }
            observed.push(event.msg);
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "guardian denial circuit breaker should interrupt the turn; observed events: {observed:?}"
        )
    });
    assert_eq!(aborted.reason, TurnAbortReason::Interrupted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guardian_helper_review_interrupts_after_three_consecutive_denials() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "keep turn active for helper reviews".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    let session_for_review = Arc::clone(&sess);
    let turn_for_review = Arc::clone(&tc);
    let turn_id = tc.sub_id.clone();
    let review_thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("helper review runtime");
        runtime.block_on(async move {
            for _ in 0..3 {
                crate::guardian::record_guardian_denial_for_test(
                    &session_for_review,
                    &turn_for_review,
                    &turn_id,
                )
                .await;
            }
        });
    });
    review_thread.join().expect("helper review thread");

    let mut observed = Vec::new();
    let aborted = timeout(StdDuration::from_secs(5), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::TurnAborted(event) = &event.msg {
                let event = event.clone();
                observed.push(EventMsg::TurnAborted(event.clone()));
                break event;
            }
            observed.push(event.msg);
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "helper review circuit breaker should interrupt the turn; observed events: {observed:?}"
        )
    });
    assert_eq!(aborted.reason, TurnAbortReason::Interrupted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[test_log::test]
async fn abort_regular_task_emits_turn_aborted_only() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    // Interrupts persist a model-visible `<turn_aborted>` marker into history, but there is no
    // separate client-visible event for that marker (only `EventMsg::TurnAborted`).
    let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("event");
    match evt.msg {
        EventMsg::TurnAborted(e) => assert_eq!(TurnAbortReason::Interrupted, e.reason),
        other => panic!("unexpected event: {other:?}"),
    }
    // No extra events should be emitted after an abort.
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn abort_gracefully_emits_turn_aborted_only() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    // Even if tasks handle cancellation gracefully, interrupts still result in `TurnAborted`
    // being the only client-visible signal.
    let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
        .await
        .expect("timeout waiting for event")
        .expect("event");
    match evt.msg {
        EventMsg::TurnAborted(e) => assert_eq!(TurnAbortReason::Interrupted, e.reason),
        other => panic!("unexpected event: {other:?}"),
    }
    // No extra events should be emitted after an abort.
    assert!(rx.try_recv().is_err());
}

async fn recv_pending_input_lifecycle_event(rx: &async_channel::Receiver<Event>) -> Event {
    loop {
        let event = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("expected pending input lifecycle event")
            .expect("channel open");
        if matches!(&event.msg, EventMsg::ThreadContextUsageUpdated(_)) {
            continue;
        }
        assert!(
            !matches!(&event.msg, EventMsg::RawResponseItem(_)),
            "pending input lifecycle should not emit raw response items"
        );
        return event;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_finish_emits_turn_item_lifecycle_for_leftover_pending_user_input() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    while rx.try_recv().is_ok() {}

    sess.inject_hook_inspectable_items(vec![ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "late pending input".to_string(),
        }],
        phase: None,
    }])
    .await
    .expect("inject pending input into active turn");

    sess.on_task_finished(Arc::clone(&tc), /*last_agent_message*/ None)
        .await;

    let history = sess.clone_history().await;
    let expected = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "late pending input".to_string(),
        }],
        phase: None,
    };
    assert!(
        history.raw_items().iter().any(|item| item == &expected),
        "expected pending input to be persisted into history on turn completion"
    );

    let first = recv_pending_input_lifecycle_event(&rx).await;
    assert!(matches!(
        first.msg,
        EventMsg::ItemStarted(ItemStartedEvent {
            item: TurnItem::UserMessage(UserMessageItem { content, .. }),
            ..
        }) if content == vec![UserInput::Text {
            text: "late pending input".to_string(),
            text_elements: Vec::new(),
        }]
    ));

    let second = recv_pending_input_lifecycle_event(&rx).await;
    assert!(matches!(
        second.msg,
        EventMsg::ItemCompleted(ItemCompletedEvent {
            item: TurnItem::UserMessage(UserMessageItem { content, .. }),
            ..
        }) if content == vec![UserInput::Text {
            text: "late pending input".to_string(),
            text_elements: Vec::new(),
        }]
    ));

    let third = recv_pending_input_lifecycle_event(&rx).await;
    assert!(matches!(
        third.msg,
        EventMsg::UserMessage(UserMessageEvent {
            message,
            images,
            text_elements,
            local_images,
            ..
        }) if message == "late pending input"
            && images == Some(Vec::new())
            && text_elements.is_empty()
            && local_images.is_empty()
    ));

    let fourth = recv_pending_input_lifecycle_event(&rx).await;
    assert!(matches!(
        fourth.msg,
        EventMsg::TurnComplete(TurnCompleteEvent {
            turn_id,
            last_agent_message: None,
            time_to_first_token_ms: None,
            ..
        }) if turn_id == tc.sub_id
    ));
}

#[tokio::test]
async fn explicit_record_conversation_items_emits_event_driven_tool_display_event() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let trigger = EventDrivenToolTrigger {
        tool: "fs_subscribe".to_string(),
        title: "File watch triggered".to_string(),
        text: "build.log changed".to_string(),
    };

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::EventDrivenTool {
            id: Some("typed-event-driven-tool".to_string()),
            trigger: trigger.clone(),
        }],
    )
    .await;

    let completed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::EventDrivenToolCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected item completed event");

    assert_eq!(completed.thread_id, sess.conversation_id);
    assert_eq!(completed.turn_id, tc.sub_id);
    assert!(completed.completed_at_ms > 0);
    assert_eq!(completed.id, "typed-event-driven-tool");
    assert_eq!(completed.trigger, trigger);
}

#[tokio::test]
async fn explicit_record_conversation_items_emits_command_wait_display_event() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::CommandWait {
            id: None,
            command_id: "cmd-1".to_string(),
            status: protocol::models::CommandWaitStatus::Completed,
            notification: Some(protocol::models::CommandWaitNotificationKind::Exit),
            exit_code: Some(0),
            wall_time_seconds: 1.25,
            wait_timeout_ms: 250,
            created_at_ms: 1234,
        }],
    )
    .await;

    let completed = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::CommandWaitCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected response item completed event");

    assert_eq!(completed.thread_id, sess.conversation_id);
    assert_eq!(completed.turn_id, tc.sub_id);
    assert!(completed.lifecycle_at_ms > 0);
    assert_eq!(completed.id.starts_with("response-item-"), true);
    assert_eq!(completed.command_id, "cmd-1");
    assert_eq!(
        completed.status,
        protocol::models::CommandWaitStatus::Completed
    );
    assert_eq!(
        completed.notification,
        Some(protocol::models::CommandWaitNotificationKind::Exit)
    );
    assert_eq!(completed.exit_code, Some(0));
    assert_eq!(completed.wall_time_seconds, 1.25);
    assert_eq!(completed.wait_timeout_ms, 250);
    assert_eq!(completed.created_at_ms, 1234);
}

#[tokio::test]
async fn record_conversation_items_does_not_emit_item_completed_for_structured_response_item() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let trigger = EventDrivenToolTrigger {
        tool: "fs_subscribe".to_string(),
        title: "File watch triggered".to_string(),
        text: "build.log changed".to_string(),
    };

    sess.record_conversation_items(
        &tc,
        &[ResponseItem::EventDrivenTool {
            id: Some("typed-event-driven-tool".to_string()),
            trigger,
        }],
    )
    .await;

    let completed = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::ItemCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await;

    assert!(
        completed.is_err(),
        "plain conversation recording should not emit a structured completed item"
    );
}

#[tokio::test]
async fn record_response_item_emits_item_completed_for_hook_prompt() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let hook_prompt_message = build_hook_prompt_message(&[HookPromptFragment::from_single_hook(
        "Retry with the requested change.",
        "hook-run-1",
    )])
    .expect("hook prompt message");

    sess.record_response_item_and_emit_turn_item(&tc, hook_prompt_message)
        .await;

    let completed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::ItemCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected item completed event");

    let TurnItem::HookPrompt(item) = completed.item else {
        panic!("expected HookPrompt item");
    };
    assert_eq!(
        item.fragments,
        vec![HookPromptFragment {
            text: "Retry with the requested change.".to_string(),
            hook_run_id: "hook-run-1".to_string(),
        }]
    );
}

#[tokio::test]
async fn explicit_record_conversation_items_emits_event_command_display_event() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let event = EventCommandEvent {
        subscription_id: "sub-command".to_string(),
        kind: EventCommandEventKind::Output,
        label: Some("build log".to_string()),
        command: "tail -f /tmp/build.log".to_string(),
        cwd: Some("/repo".to_string()),
        line: Some("done".to_string()),
        sequence: Some(1),
        exit_code: None,
        signal: None,
        message: None,
        truncated: false,
        created_at: 1,
    };

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::EventCommandEvent {
            id: Some("typed-event-command".to_string()),
            event: event.clone(),
        }],
    )
    .await;

    let completed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::EventCommandEventCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected item completed event");

    assert_eq!(completed.id, "typed-event-command");
    assert_eq!(completed.event, event);
}

#[tokio::test]
async fn explicit_record_conversation_items_emits_inter_agent_display_event() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "done".to_string(),
        InterAgentOperation::SendMessage,
    )
    .with_trigger_turn(false);

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::InterAgentCommunication {
            id: Some("typed-collab".to_string()),
            communication: communication.clone(),
        }],
    )
    .await;

    let completed = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::InterAgentCommunicationCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await
    .expect("expected response item completed event");

    assert_eq!(completed.id, "typed-collab");
    assert_eq!(completed.communication, communication);
}

#[tokio::test]
async fn explicit_record_conversation_items_ignores_unknown_collab_message() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "raw update".to_string(),
        InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);

    sess.record_model_items_and_emit_display_events(
        &tc,
        &[ResponseItem::InterAgentCommunication {
            id: Some("typed-unknown-collab".to_string()),
            communication,
        }],
    )
    .await;

    let completed = tokio::time::timeout(Duration::from_millis(200), async {
        loop {
            let event = rx.recv().await.expect("event");
            if let EventMsg::ItemCompleted(completed) = event.msg {
                break completed;
            }
        }
    })
    .await;

    assert!(
        completed.is_err(),
        "unknown collab communication should not emit a structured completed item"
    );
}

#[tokio::test]
async fn steer_input_returns_active_turn_id() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    let steer_input = vec![UserInput::Text {
        text: "steer".to_string(),
        text_elements: Vec::new(),
    }];
    let turn_id = sess
        .steer_input(
            steer_input,
            Some(&tc.sub_id),
            /*responsesapi_client_metadata*/ None,
        )
        .await
        .expect("steering with matching expected turn id should succeed");

    assert_eq!(turn_id, tc.sub_id);
    assert!(sess.has_pending_input().await);
}

#[tokio::test]
async fn prepend_pending_input_keeps_older_tail_ahead_of_newer_input() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "hello".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(
        Arc::clone(&tc),
        input,
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    let blocked = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "blocked queued prompt".to_string(),
        }],
        phase: None,
    };
    let later = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "later queued prompt".to_string(),
        }],
        phase: None,
    };
    let newer = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "newer queued prompt".to_string(),
        }],
        phase: None,
    };

    sess.inject_hook_inspectable_items(vec![blocked.clone(), later.clone()])
        .await
        .expect("inject initial pending input into active turn");

    let drained = sess.get_pending_input().await;
    assert_eq!(
        drained,
        vec![
            PendingInputItem::from(blocked),
            PendingInputItem::from(later.clone()),
        ]
    );

    sess.inject_hook_inspectable_items(vec![newer.clone()])
        .await
        .expect("inject newer pending input into active turn");

    let mut drained_iter = drained.into_iter();
    let _blocked = drained_iter.next().expect("blocked prompt should exist");
    sess.prepend_pending_input(drained_iter.collect())
        .await
        .expect("requeue later pending input at the front of the queue");

    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(later), PendingInputItem::from(newer)]
    );
}

#[tokio::test]
async fn queued_response_items_for_next_turn_move_into_next_active_turn() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let queued_item = ResponseInputItem::Message {
        role: "assistant".to_string(),
        content: vec![ContentItem::InputText {
            text: "queued before wake".to_string(),
        }],
        phase: None,
    };

    sess.queue_response_items_for_next_turn(vec![PendingInputItem::from(queued_item.clone())])
        .await;

    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(queued_item)]
    );
}

#[tokio::test]
async fn idle_interrupt_does_not_wake_queued_next_turn_items() {
    let (sess, _tc, _rx) = make_session_and_context_with_rx().await;
    let queued_item = ResponseInputItem::Message {
        role: "assistant".to_string(),
        content: vec![ContentItem::InputText {
            text: "queued before interrupt".to_string(),
        }],
        phase: None,
    };

    sess.queue_response_items_for_next_turn(vec![PendingInputItem::from(queued_item)])
        .await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    assert!(sess.active_turn.lock().await.is_none());
    assert!(sess.has_queued_response_items_for_next_turn().await);
}

#[tokio::test]
async fn abort_empty_active_turn_preserves_pending_input() {
    let (sess, _tc, _rx) = make_session_and_context_with_rx().await;
    let pending_item = ResponseInputItem::Message {
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "late pending input".to_string(),
        }],
        phase: None,
    };
    let turn_state = {
        let mut active = sess.active_turn.lock().await;
        let active_turn = active.get_or_insert_with(ActiveTurn::default);
        Arc::clone(&active_turn.turn_state)
    };
    turn_state
        .lock()
        .await
        .push_pending_input(PendingInputItem::from(pending_item.clone()));

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    assert!(sess.active_turn.lock().await.is_none());
    assert_eq!(
        turn_state.lock().await.take_pending_input(),
        vec![PendingInputItem::from(pending_item)]
    );
}

#[tokio::test]
async fn interrupt_accounts_active_goal_before_pausing() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Keep improving the benchmark".to_string(),
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;

    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;
    set_total_token_usage(&sess, post_goal_token_usage()).await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    let goal = GoalService
        .get_thread_goal(sess.as_ref())
        .await
        .map_err(anyhow::Error::msg)?
        .expect("goal should remain persisted after interrupt");
    assert_eq!(protocol::protocol::ThreadGoalStatus::Paused, goal.status);
    assert_eq!(70, goal.tokens_used);

    assert!(sess.active_turn.lock().await.is_none());

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn active_goal_continuation_runs_again_after_no_tool_turn() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(|config| {
        config
            .features
            .enable(Feature::Goals)
            .expect("goal mode should be enableable in tests");
    });
    let test = builder.build(&server).await?;
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_function_call(
                    "call-create-goal",
                    "create_goal",
                    r#"{"objective":"write a benchmark note"}"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_assistant_message("msg-1", "Draft ready."),
                ev_completed("resp-2"),
            ]),
            sse(vec![
                ev_assistant_message("msg-2", "I am still working on the benchmark note."),
                ev_completed("resp-3"),
            ]),
            sse(vec![
                ev_response_created("resp-4"),
                ev_function_call(
                    "call-complete-goal",
                    "update_goal",
                    r#"{"status":"complete"}"#,
                ),
                ev_completed("resp-4"),
            ]),
            sse(vec![
                ev_assistant_message("msg-3", "Goal complete."),
                ev_completed("resp-5"),
            ]),
        ],
    )
    .await;

    test.codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "write a benchmark note".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;
    let mut completed_turns = 0;
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        loop {
            let event = test.codex.next_event().await?;
            if matches!(event.msg, EventMsg::TurnComplete(_)) {
                completed_turns += 1;
                if completed_turns == 3 {
                    return anyhow::Ok(());
                }
            }
        }
    })
    .await??;

    let continuation_request = responses
        .requests()
        .into_iter()
        .find(|request| request.body_contains_text("<goal_context>"))
        .expect("expected a goal continuation request");
    let body = continuation_request.body_json();
    let goal_context_message = body["input"]
        .as_array()
        .expect("input should be an array")
        .iter()
        .find(|item| item.to_string().contains("<goal_context>"))
        .expect("goal context message should be present");
    assert_eq!(goal_context_message["role"].as_str(), Some("user"));
    assert!(
        goal_context_message
            .to_string()
            .contains("Continue working toward the active thread goal.")
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_request_user_input_does_not_spawn_extra_goal_continuation() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(|config| {
        config
            .features
            .enable(Feature::Goals)
            .expect("goal mode should be enableable in tests");
        config
            .features
            .enable(Feature::DefaultModeRequestUserInput)
            .expect("default-mode request_user_input should be enableable in tests");
    });
    let test = builder.build(&server).await?;
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_function_call(
                    "call-create-goal",
                    "create_goal",
                    r#"{"objective":"write a benchmark note"}"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_assistant_message("msg-1", "Draft ready."),
                ev_completed("resp-2"),
            ]),
            sse(vec![
                ev_response_created("resp-3"),
                ev_function_call(
                    "call-ask-user",
                    "request_user_input",
                    r#"{"questions":[{"header":"Choice","id":"next_step","question":"Pick one","options":[{"label":"Outline","description":"Start with an outline."},{"label":"Draft","description":"Write a full draft."}]}]}"#,
                ),
                ev_completed("resp-3"),
            ]),
            sse(vec![
                ev_response_created("resp-4"),
                ev_function_call(
                    "call-complete-goal",
                    "update_goal",
                    r#"{"status":"complete"}"#,
                ),
                ev_completed("resp-4"),
            ]),
            sse(vec![
                ev_assistant_message("msg-2", "Goal complete."),
                ev_completed("resp-5"),
            ]),
        ],
    )
    .await;

    test.codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "write a benchmark note".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;

    let request_user_input_event = wait_for_event_match(&test.codex, |event| match event {
        EventMsg::RequestUserInput(event) => Some(event.clone()),
        _ => None,
    })
    .await;
    assert_eq!(3, responses.requests().len());
    assert!(
        timeout(Duration::from_millis(200), test.codex.next_event())
            .await
            .is_err(),
        "waiting for request_user_input should keep the turn open without emitting more events"
    );
    assert_eq!(
        3,
        responses.requests().len(),
        "waiting for request_user_input should not start another continuation request"
    );

    test.codex
        .submit(Op::UserInputAnswer {
            id: request_user_input_event.turn_id,
            response: RequestUserInputResponse {
                answers: std::collections::HashMap::from([(
                    "next_step".to_string(),
                    RequestUserInputAnswer {
                        answers: vec!["Outline".to_string()],
                    },
                )]),
            },
        })
        .await?;

    let mut completed_turns = 0;
    timeout(Duration::from_secs(8), async {
        loop {
            let event = test.codex.next_event().await?;
            if matches!(event.msg, EventMsg::TurnComplete(_)) {
                completed_turns += 1;
                if completed_turns == 1 {
                    return anyhow::Ok(());
                }
            }
        }
    })
    .await??;

    assert_eq!(5, responses.requests().len());

    Ok(())
}

async fn set_total_token_usage(sess: &Session, total_token_usage: TokenUsage) {
    let mut state = sess.state.lock().await;
    state.set_token_info(Some(TokenUsageInfo {
        total_token_usage,
        last_token_usage: TokenUsage::default(),
        model_context_window: None,
    }));
}

fn post_goal_token_usage() -> TokenUsage {
    TokenUsage {
        input_tokens: 50,
        cached_input_tokens: 10,
        output_tokens: 30,
        reasoning_output_tokens: 5,
        total_tokens: 75,
    }
}

async fn goal_test_state_db(sess: &Session) -> anyhow::Result<crate::StateDbHandle> {
    if let Some(state_db) = sess.state_db() {
        return Ok(state_db);
    }
    let config = sess.get_config().await;
    state::StateRuntime::init(config.sqlite_home.clone(), config.model_provider_id.clone())
        .await
        .map(|state_db| state_db as crate::StateDbHandle)
}

#[tokio::test]
async fn budget_limited_accounting_steers_active_turn_without_aborting() -> anyhow::Result<()> {
    let (sess, tc, rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Keep improving the benchmark".to_string(),
            Some(10),
        )
        .await
        .map_err(anyhow::Error::msg)?;
    GoalService
        .begin_turn_goal_accounting(sess.as_ref(), tc.as_ref(), TokenUsage::default())
        .await
        .map_err(anyhow::Error::msg)?;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;
    while rx.try_recv().is_ok() {}

    set_total_token_usage(
        &sess,
        TokenUsage {
            input_tokens: 20,
            cached_input_tokens: 0,
            output_tokens: 5,
            reasoning_output_tokens: 0,
            total_tokens: 25,
        },
    )
    .await;

    GoalService
        .account_non_goal_tool_completed(sess.as_ref(), tc.as_ref(), "exec_command")
        .await
        .map_err(anyhow::Error::msg)?;

    let pending_input = sess.get_pending_input().await;
    let [PendingInputItem::HookInspectable(ResponseItem::Message { role, content, .. })] =
        pending_input.as_slice()
    else {
        panic!("expected one budget-limit steering message, got {pending_input:#?}");
    };
    assert_eq!("user", role);
    let [ContentItem::InputText { text }] = content.as_slice() else {
        panic!("expected one text span in budget-limit steering message, got {content:#?}");
    };
    assert!(text.starts_with("<goal_context>"));
    assert!(text.trim_end().ends_with("</goal_context>"));
    assert!(text.contains("budget_limited"));
    assert!(text.to_lowercase().contains("wrap up this turn soon"));
    assert!(sess.active_turn.lock().await.is_some());
    while let Ok(event) = rx.try_recv() {
        assert!(
            !matches!(event.msg, EventMsg::TurnAborted(_)),
            "budget limit should steer the active turn instead of aborting it"
        );
    }

    let state_db = goal_test_state_db(sess.as_ref()).await?;
    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted after accounting");
    assert_eq!(state_api::ThreadGoalStatus::BudgetLimited, goal.status);
    assert_eq!(25, goal.tokens_used);

    set_total_token_usage(
        &sess,
        TokenUsage {
            input_tokens: 30,
            cached_input_tokens: 0,
            output_tokens: 10,
            reasoning_output_tokens: 0,
            total_tokens: 40,
        },
    )
    .await;
    GoalService
        .account_goal_mutation_completed(sess.as_ref(), tc.as_ref())
        .await
        .map_err(anyhow::Error::msg)?;

    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted after follow-up accounting");
    assert_eq!(state_api::ThreadGoalStatus::BudgetLimited, goal.status);
    assert_eq!(40, goal.tokens_used);

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_goal_mutation_accounts_active_turn_before_status_change() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    GoalService
        .create_thread_goal(
            sess.as_ref(),
            tc.as_ref(),
            "Keep improving the benchmark".to_string(),
            None,
        )
        .await
        .map_err(anyhow::Error::msg)?;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;
    set_total_token_usage(&sess, post_goal_token_usage()).await;

    GoalService
        .prepare_external_goal_mutation(sess.as_ref())
        .await
        .map_err(anyhow::Error::msg)?;

    let state_db = goal_test_state_db(sess.as_ref()).await?;
    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted");
    assert_eq!(70, goal.tokens_used);

    let previous_goal = goal.clone();
    let goal_id = goal.goal_id.clone();
    let updated_goal = state_db
        .update_thread_goal(
            sess.conversation_id,
            state_api::ThreadGoalUpdate {
                objective: None,
                status: Some(state_api::ThreadGoalStatus::Complete),
                token_budget: None,
                expected_goal_id: Some(goal_id),
            },
        )
        .await?
        .expect("goal status update should succeed");
    GoalService
        .apply_external_goal_set(
            sess.as_ref(),
            ExternalGoalSet {
                goal: updated_goal,
                previous_status: ExternalGoalPreviousStatus::from(&previous_goal),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;

    assert!(sess.active_turn.lock().await.is_some());
    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted");
    assert_eq!(state_api::ThreadGoalStatus::Complete, goal.status);
    assert_eq!(70, goal.tokens_used);

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_objective_change_steers_active_turn() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;

    let state_db = goal_test_state_db(sess.as_ref()).await?;
    let old_goal = state_db
        .replace_thread_goal(
            sess.conversation_id,
            "Keep improving the benchmark",
            state_api::ThreadGoalStatus::Active,
            /*token_budget*/ Some(10_000),
        )
        .await?;
    let new_goal = state_db
        .replace_thread_goal(
            sess.conversation_id,
            "Write a concise benchmark summary",
            state_api::ThreadGoalStatus::Active,
            /*token_budget*/ Some(10_000),
        )
        .await?;

    GoalService
        .apply_external_goal_set(
            sess.as_ref(),
            ExternalGoalSet {
                goal: new_goal,
                previous_status: ExternalGoalPreviousStatus::from(&old_goal),
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;

    let pending_input = sess.get_pending_input().await;
    assert!(
        pending_input.iter().any(|item| {
            matches!(
                item,
                PendingInputItem::HookInspectable(ResponseItem::Message { role, content, .. })
                    if role == "user"
                        && content.iter().any(|content| matches!(
                            content,
                            ContentItem::InputText { text }
                                if text.starts_with("<goal_context>")
                                    && text.trim_end().ends_with("</goal_context>")
                                    && text.contains("The active thread goal objective was edited")
                                    && text.contains("Write a concise benchmark summary")
                        ))
            )
        }),
        "expected objective-updated steering prompt in pending input: {pending_input:?}"
    );

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn external_active_goal_set_marks_current_turn_for_accounting() -> anyhow::Result<()> {
    let (sess, tc, _rx, _codex_home) = make_goal_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: false,
        },
    )
    .await;
    set_total_token_usage(&sess, post_goal_token_usage()).await;

    let state_db = goal_test_state_db(sess.as_ref()).await?;
    let goal = state_db
        .replace_thread_goal(
            sess.conversation_id,
            "Keep improving the benchmark",
            state_api::ThreadGoalStatus::Active,
            /*token_budget*/ None,
        )
        .await?;
    GoalService
        .apply_external_goal_set(
            sess.as_ref(),
            ExternalGoalSet {
                goal,
                previous_status: ExternalGoalPreviousStatus::NewGoal,
            },
        )
        .await
        .map_err(anyhow::Error::msg)?;

    set_total_token_usage(
        &sess,
        TokenUsage {
            input_tokens: 65,
            cached_input_tokens: 10,
            output_tokens: 40,
            reasoning_output_tokens: 5,
            total_tokens: 110,
        },
    )
    .await;
    GoalService
        .account_non_goal_tool_completed(sess.as_ref(), tc.as_ref(), "exec_command")
        .await
        .map_err(anyhow::Error::msg)?;

    let goal = state_db
        .get_thread_goal(sess.conversation_id)
        .await?
        .expect("goal should remain persisted");
    assert_eq!(state_api::ThreadGoalStatus::Active, goal.status);
    assert_eq!(25, goal.tokens_used);

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completed_goal_accounts_current_turn_tokens_before_tool_response() -> anyhow::Result<()> {
    let server = start_mock_server().await;
    let mut builder = test_codex().with_config(|config| {
        config
            .features
            .enable(Feature::Goals)
            .expect("goal mode should be enableable in tests");
    });
    let test = builder.build(&server).await?;
    let responses = mount_sse_sequence(
        &server,
        vec![
            sse(vec![
                ev_response_created("resp-1"),
                ev_function_call(
                    "call-create-goal",
                    "create_goal",
                    r#"{"objective":"write a report","token_budget":500}"#,
                ),
                ev_completed("resp-1"),
            ]),
            sse(vec![
                ev_response_created("resp-2"),
                ev_function_call(
                    "call-complete-goal",
                    "update_goal",
                    r#"{"status":"complete"}"#,
                ),
                ev_completed_with_tokens("resp-2", /*total_tokens*/ 580),
            ]),
            sse(vec![
                ev_assistant_message("msg-1", "Goal complete."),
                ev_completed("resp-3"),
            ]),
        ],
    )
    .await;

    test.codex
        .submit(Op::UserInput {
            environments: None,
            items: vec![UserInput::Text {
                text: "write a report".into(),
                text_elements: Vec::new(),
            }],
            final_output_json_schema: None,
            responsesapi_client_metadata: None,
        })
        .await?;

    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let event = test.codex.next_event().await?;
            if matches!(event.msg, EventMsg::TurnComplete(_)) {
                return anyhow::Ok(());
            }
        }
    })
    .await??;

    let complete_output = responses
        .function_call_output_text("call-complete-goal")
        .expect("complete tool output should be sent to the model");
    let complete_output: serde_json::Value = serde_json::from_str(&complete_output)?;
    assert_eq!(complete_output["goal"]["tokensUsed"], 580);
    assert_eq!(complete_output["goal"]["status"], "complete");
    assert_eq!(complete_output["remainingTokens"], 0);
    assert_eq!(
        complete_output["completionBudgetReport"],
        "Goal achieved. Report final budget usage to the user: tokens used: 580 of 500."
    );
    let requests = responses.requests();
    let completion_followup_request = requests
        .last()
        .expect("completion tool output should be sent in a follow-up request");
    assert!(
        !completion_followup_request.body_contains_text("budget_limited"),
        "completion follow-up should not include budget-limit steering"
    );

    let state_db = state::StateRuntime::init(
        test.config.sqlite_home.clone(),
        test.config.model_provider_id.clone(),
    )
    .await?;
    let persisted_goal = state_db
        .get_thread_goal(test.session_configured.thread_id)
        .await?
        .expect("goal should be persisted");
    assert_eq!(state_api::ThreadGoalStatus::Complete, persisted_goal.status);
    assert_eq!(580, persisted_goal.tokens_used);

    Ok(())
}

#[tokio::test]
async fn queue_only_mailbox_mail_waits_for_next_turn_after_answer_boundary() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "late queue-only update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_mailbox_delivery_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(communication.clone());

    assert!(
        !sess.has_pending_input().await,
        "queue-only mailbox mail should stay buffered once the current turn emitted its answer"
    );
    assert_eq!(sess.get_pending_input().await, Vec::new());

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(communication)],
    );
}

#[tokio::test]
async fn typed_queue_only_inter_agent_message_does_not_trigger_idle_turn() {
    let (sess, _tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queue-only typed update".to_string(),
        protocol::protocol::InterAgentOperation::SendMessage,
    )
    .with_trigger_turn(false);

    sess.enqueue_async_input(PendingInputItem::from(
        ResponseItem::InterAgentCommunication {
            id: Some("typed-queue-only".to_string()),
            communication: communication.clone(),
        },
    ));

    assert!(
        !sess.has_trigger_turn_mailbox_items().await,
        "queue-only typed inter-agent message should not request a new idle turn"
    );
    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(communication)],
    );
}

#[tokio::test]
async fn pending_mailbox_input_can_be_peeked_without_consuming() {
    let (sess, _tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "already pending".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(false);

    sess.enqueue_mailbox_communication(communication.clone());

    let found = sess
        .find_pending_input(|item| match item {
            PendingInputItem::InterAgentCommunication(mail)
                if mail.author == communication.author =>
            {
                Some(mail.clone())
            }
            _ => None,
        })
        .await;
    assert_eq!(found, Some(communication.clone()));
    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(communication)],
    );
}

#[tokio::test]
async fn inter_agent_unknown_communication_does_not_emit_live_collab_item() -> anyhow::Result<()> {
    let parent_thread_id = ThreadId::new();
    let (session, rx_event) = make_session_with_history_source_and_agent_control_and_rx(
        InitialHistory::Resumed(ResumedHistory {
            conversation_id: parent_thread_id,
            history: Vec::new(),
            rollout_path: None,
        }),
        SessionSource::Exec,
        AgentControl::default(),
    )
    .await?;
    let _configured = rx_event.recv().await?;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "internal update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);

    crate::session::handlers::inter_agent_communication(
        &session,
        "unknown-mail".to_string(),
        communication,
    )
    .await;
    assert!(session.has_pending_mailbox_items().await);

    let result = timeout(Duration::from_millis(200), async {
        loop {
            let event = rx_event.recv().await?;
            if let EventMsg::ItemCompleted(completed) = event.msg {
                return anyhow::Ok(completed);
            }
        }
    })
    .await;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn inter_agent_child_completion_live_item_waits_for_typed_recording() -> anyhow::Result<()> {
    let (session, turn_context, rx_event) = make_session_and_context_with_rx().await;
    let parent_thread_id = session.thread_id();
    let child_thread_id = ThreadId::new();
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "done".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(false)
    .with_thread_ids(child_thread_id, parent_thread_id)
    .with_status(protocol::protocol::AgentStatus::Completed(Some(
        "done".to_string(),
    )));

    crate::session::handlers::inter_agent_communication(
        &session,
        "child-completion-turn".to_string(),
        communication.clone(),
    )
    .await;
    assert!(session.has_pending_mailbox_items().await);

    let immediate_completed = timeout(Duration::from_millis(200), async {
        loop {
            let event = rx_event.recv().await?;
            if let EventMsg::ItemCompleted(completed) = event.msg {
                return anyhow::Ok(completed);
            }
        }
    })
    .await;
    assert!(
        immediate_completed.is_err(),
        "child completion should not emit a raw live collab item before typed pending input is recorded"
    );

    hooks::record_pending_input(
        session.as_ref(),
        turn_context.as_ref(),
        hooks::PendingInputRecord::InterAgentCommunication {
            pending_input: PendingInputItem::from(communication.clone()),
        },
    )
    .await;

    let completed = timeout(Duration::from_secs(2), async {
        loop {
            let event = rx_event.recv().await?;
            if let EventMsg::ItemCompleted(completed) = event.msg {
                return anyhow::Ok(completed);
            }
        }
    })
    .await??;
    assert_eq!(completed.thread_id, parent_thread_id);
    assert_eq!(completed.turn_id, turn_context.sub_id.clone());
    assert!(completed.completed_at_ms > 0);
    let protocol::items::TurnItem::CollabAgentMessage(item) = completed.item else {
        panic!("expected completed collab agent message item");
    };
    assert_eq!(item.communication, communication);

    let duplicate_completed = timeout(Duration::from_millis(200), async {
        loop {
            let event = rx_event.recv().await?;
            if let EventMsg::ItemCompleted(completed) = event.msg {
                return anyhow::Ok(completed);
            }
        }
    })
    .await;
    assert!(
        duplicate_completed.is_err(),
        "recording one typed child completion should emit exactly one live collab item"
    );

    Ok(())
}

#[tokio::test]
async fn turn_start_consumes_child_completion_before_parent_visible_complete() {
    let (sess, tc, _rx_event) = make_session_and_context_with_rx().await;
    let child_thread_id = ThreadId::new();
    sess.mark_direct_child_completion_pending(child_thread_id)
        .await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "done".to_string(),
        protocol::protocol::InterAgentOperation::ChildCompletion,
    )
    .with_trigger_turn(true)
    .with_thread_ids(child_thread_id, sess.thread_id())
    .with_status(protocol::protocol::AgentStatus::Completed(Some(
        "done".to_string(),
    )));
    sess.enqueue_mailbox_communication(communication);
    assert!(sess.has_pending_direct_child_completions().await);

    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    assert!(
        !sess.has_pending_direct_child_completions().await,
        "child completion should become parent-visible when the parent turn consumes mailbox input"
    );
    assert!(
        !sess.has_pending_mailbox_items().await,
        "turn start should drain the consumed child completion from the mailbox"
    );
    sess.abort_all_tasks(TurnAbortReason::Replaced).await;
}

#[tokio::test]
async fn inter_agent_send_message_queue_only_does_not_emit_live_collab_item() -> anyhow::Result<()>
{
    let parent_thread_id = ThreadId::new();
    let (session, rx_event) = make_session_with_history_source_and_agent_control_and_rx(
        InitialHistory::Resumed(ResumedHistory {
            conversation_id: parent_thread_id,
            history: Vec::new(),
            rollout_path: None,
        }),
        SessionSource::Exec,
        AgentControl::default(),
    )
    .await?;
    let _configured = rx_event.recv().await?;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queued message".to_string(),
        protocol::protocol::InterAgentOperation::SendMessage,
    )
    .with_trigger_turn(false);

    crate::session::handlers::inter_agent_communication(
        &session,
        "queued-mail".to_string(),
        communication,
    )
    .await;
    assert!(session.has_pending_mailbox_items().await);

    let result = timeout(Duration::from_millis(200), async {
        loop {
            let event = rx_event.recv().await?;
            if let EventMsg::ItemCompleted(completed) = event.msg {
                return anyhow::Ok(completed);
            }
        }
    })
    .await;
    assert!(result.is_err());

    Ok(())
}

#[tokio::test]
async fn trigger_turn_mailbox_mail_waits_for_next_turn_after_answer_boundary() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_mailbox_delivery_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "late trigger update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    ));

    assert!(
        !sess.has_pending_input().await,
        "trigger-turn mailbox mail should not extend the current turn after its answer boundary"
    );

    sess.abort_all_tasks(TurnAbortReason::Replaced).await;

    assert!(sess.has_trigger_turn_mailbox_items().await);
}

#[tokio::test]
async fn steered_input_reopens_mailbox_delivery_for_current_turn() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queued child update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_mailbox_delivery_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(communication.clone());
    sess.steer_input(
        vec![UserInput::Text {
            text: "follow up".to_string(),
            text_elements: Vec::new(),
        }],
        Some(&tc.sub_id),
        /*responsesapi_client_metadata*/ None,
    )
    .await
    .expect("steered input should be accepted");

    assert_eq!(
        sess.get_pending_input().await,
        vec![
            PendingInputItem::from(ResponseInputItem::from(vec![UserInput::Text {
                text: "follow up".to_string(),
                text_elements: Vec::new(),
            }])),
            PendingInputItem::from(communication),
        ],
    );
}

#[tokio::test]
async fn stale_defer_mailbox_delivery_does_not_override_steered_input() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queued child update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_mailbox_delivery_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(communication.clone());
    sess.steer_input(
        vec![UserInput::Text {
            text: "follow up".to_string(),
            text_elements: Vec::new(),
        }],
        Some(&tc.sub_id),
        /*responsesapi_client_metadata*/ None,
    )
    .await
    .expect("steered input should be accepted");

    sess.defer_mailbox_delivery_to_next_turn(&tc.sub_id).await;

    assert_eq!(
        sess.get_pending_input().await,
        vec![
            PendingInputItem::from(ResponseInputItem::from(vec![UserInput::Text {
                text: "follow up".to_string(),
                text_elements: Vec::new(),
            }])),
            PendingInputItem::from(communication),
        ],
    );
}

#[tokio::test]
async fn tool_calls_reopen_mailbox_delivery_for_current_turn() {
    let (sess, tc, _rx) = make_session_and_context_with_rx().await;
    let communication = InterAgentCommunication::new(
        AgentPath::try_from("/root/worker").expect("worker path should parse"),
        AgentPath::root(),
        Vec::new(),
        "queued child update".to_string(),
        protocol::protocol::InterAgentOperation::Unknown,
    )
    .with_trigger_turn(false);
    sess.spawn_task(
        Arc::clone(&tc),
        Vec::new(),
        NeverEndingTask {
            kind: TaskKind::Regular,
            listen_to_cancellation_token: true,
        },
    )
    .await;

    sess.defer_mailbox_delivery_to_next_turn(&tc.sub_id).await;
    sess.enqueue_mailbox_communication(communication.clone());

    let item = ResponseItem::FunctionCall {
        id: None,
        name: "test_tool".to_string(),
        namespace: None,
        arguments: "{}".to_string(),
        call_id: "call-1".to_string(),
    };
    let mut ctx = HandleOutputCtx {
        sess: Arc::clone(&sess),
        turn_context: Arc::clone(&tc),
        turn_store: Arc::new(codex_extension_api::ExtensionData::new(tc.sub_id.clone())),
        tool_inputs: test_tool_inputs(Arc::clone(&sess), Arc::clone(&tc)),
        turn_diff_tracker: Arc::new(tokio::sync::Mutex::new(TurnDiffTracker::new())),
        cancellation_token: CancellationToken::new(),
    };

    let output = handle_output_item_done(&mut ctx, item, /*previously_active_item*/ None)
        .await
        .expect("tool call should be handled");

    assert!(output.needs_follow_up);
    assert!(output.tool_future.is_some());
    assert_eq!(
        sess.get_pending_input().await,
        vec![PendingInputItem::from(communication)],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_review_task_emits_exited_then_aborted_and_records_history() {
    let (sess, tc, rx) = make_session_and_context_with_rx().await;
    let input = vec![UserInput::Text {
        text: "start review".to_string(),
        text_elements: Vec::new(),
    }];
    sess.spawn_task(Arc::clone(&tc), input, ReviewTask::new())
        .await;

    sess.abort_all_tasks(TurnAbortReason::Interrupted).await;

    // Aborting a review task should exit review mode before surfacing the abort to the client.
    // We scan for these events (rather than relying on fixed ordering) since unrelated events
    // may interleave.
    let mut exited_review_mode_idx = None;
    let mut turn_aborted_idx = None;
    let mut idx = 0usize;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let evt = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("timeout waiting for event")
            .expect("event");
        let event_idx = idx;
        idx = idx.saturating_add(1);
        match evt.msg {
            EventMsg::ExitedReviewMode(ev) => {
                assert!(ev.review_output.is_none());
                exited_review_mode_idx = Some(event_idx);
            }
            EventMsg::TurnAborted(ev) => {
                assert_eq!(TurnAbortReason::Interrupted, ev.reason);
                turn_aborted_idx = Some(event_idx);
                break;
            }
            _ => {}
        }
    }
    assert!(
        exited_review_mode_idx.is_some(),
        "expected ExitedReviewMode after abort"
    );
    assert!(
        turn_aborted_idx.is_some(),
        "expected TurnAborted after abort"
    );
    assert!(
        exited_review_mode_idx.unwrap() < turn_aborted_idx.unwrap(),
        "expected ExitedReviewMode before TurnAborted"
    );

    let history = sess.clone_history().await;
    // The `<turn_aborted>` marker is silent in the event stream, so verify it is still
    // recorded in history for the model.
    assert!(
        history.raw_items().iter().any(|item| {
            let ResponseItem::Message { role, content, .. } = item else {
                return false;
            };
            if role != "user" {
                return false;
            }
            content.iter().any(|content_item| {
                let ContentItem::InputText { text } = content_item else {
                    return false;
                };
                TurnAborted::matches_text(text)
            })
        }),
        "expected a model-visible turn aborted marker in history after interrupt"
    );
}

async fn sample_rollout(
    session: &Session,
    _turn_context: &TurnContext,
) -> (Vec<RolloutItem>, Vec<ResponseItem>) {
    let mut rollout_items = Vec::new();
    let mut live_history = ContextManager::new();

    // Use the same turn_context source as record_initial_history so model_info (and thus
    // personality_spec) matches reconstruction.
    let reconstruction_turn = session.new_default_turn().await;
    let mut initial_context = session
        .build_initial_context(reconstruction_turn.as_ref())
        .await;
    // Ensure personality_spec is present when Personality is enabled, so expected matches
    // what reconstruction produces (build_initial_context may omit it when baked into model).
    if !initial_context.iter().any(|m| {
        matches!(m, ResponseItem::Message { role, content, .. }
        if role == "developer"
            && content.iter().any(|c| {
                matches!(c, ContentItem::InputText { text } if text.contains("<personality_spec>"))
            }))
    }) && let Some(p) = reconstruction_turn.personality
        && session.features.enabled(Feature::Personality)
        && let Some(personality_message) = reconstruction_turn
            .model_info
            .model_messages
            .as_ref()
            .and_then(|m| m.get_personality_message(Some(p)).filter(|s| !s.is_empty()))
    {
        let msg = ContextualUserFragment::into(
            codex_context_manager::PersonalitySpecInstructions::new(personality_message),
        );
        let insert_at = initial_context
            .iter()
            .position(|m| matches!(m, ResponseItem::Message { role, .. } if role == "developer"))
            .map(|i| i + 1)
            .unwrap_or(0);
        initial_context.insert(insert_at, msg);
    }
    for item in &initial_context {
        rollout_items.push(RolloutItem::ResponseItem(item.clone()));
    }
    live_history.record_items(
        initial_context.iter(),
        reconstruction_turn.truncation_policy,
    );

    let user1 = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "first user".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&user1),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(user1.clone()));

    let assistant1 = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "assistant reply one".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&assistant1),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(assistant1.clone()));

    let summary1 = "summary one";
    let snapshot1 = live_history
        .clone()
        .for_prompt(&reconstruction_turn.model_info.input_modalities);
    let user_messages1 = collect_user_messages(&snapshot1);
    let rebuilt1 = compact::build_compacted_history(Vec::new(), &user_messages1, summary1);
    live_history.replace(rebuilt1);
    rollout_items.push(RolloutItem::Compacted(CompactedItem {
        message: summary1.to_string(),
        replacement_history: None,
    }));

    let user2 = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "second user".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&user2),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(user2.clone()));

    let assistant2 = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "assistant reply two".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&assistant2),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(assistant2.clone()));

    let summary2 = "summary two";
    let snapshot2 = live_history
        .clone()
        .for_prompt(&reconstruction_turn.model_info.input_modalities);
    let user_messages2 = collect_user_messages(&snapshot2);
    let rebuilt2 = compact::build_compacted_history(Vec::new(), &user_messages2, summary2);
    live_history.replace(rebuilt2);
    rollout_items.push(RolloutItem::Compacted(CompactedItem {
        message: summary2.to_string(),
        replacement_history: None,
    }));

    let user3 = ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: "third user".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&user3),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(user3));

    let assistant3 = ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: "assistant reply three".to_string(),
        }],
        phase: None,
    };
    live_history.record_items(
        std::iter::once(&assistant3),
        reconstruction_turn.truncation_policy,
    );
    rollout_items.push(RolloutItem::ResponseItem(assistant3));

    (
        rollout_items,
        live_history.for_prompt(&reconstruction_turn.model_info.input_modalities),
    )
}

#[tokio::test]
async fn rejects_escalated_permissions_when_policy_not_on_request() {
    use permissions_service::ExecPolicyApprovalRequest as ExecApprovalRequest;
    use protocol::models::SandboxPermissions;
    use protocol::protocol::AskForApproval;

    let (session, mut turn_context_raw) = make_session_and_context().await;
    // Ensure policy is NOT OnRequest so the early rejection path triggers
    turn_context_raw
        .approval_policy
        .set(AskForApproval::OnFailure)
        .expect("test setup should allow updating approval policy");
    let session = Arc::new(session);
    let mut turn_context = Arc::new(turn_context_raw);

    let command_script = "echo hi";
    let sandbox_permissions = SandboxPermissions::RequireEscalated;

    let call_id = "test-call".to_string();
    #[allow(deprecated)]
    let workdir = Some(turn_context.cwd.to_string_lossy().to_string());
    let resp = dispatch_exec_command_via_tool_service(
        Arc::clone(&session),
        Arc::clone(&turn_context),
        &call_id,
        serde_json::json!({
            "cmd": command_script,
            "workdir": workdir,
            "sandbox_permissions": sandbox_permissions,
            "justification": Some("test"),
        }),
    )
    .await;

    let Err(FunctionCallError::RespondToModel(output)) = resp else {
        panic!("expected error result");
    };

    let expected = format!(
        "approval policy is {policy:?}; reject command — you cannot ask for escalated permissions if the approval policy is {policy:?}",
        policy = turn_context.approval_policy.value()
    );

    pretty_assertions::assert_eq!(output, expected);
    pretty_assertions::assert_eq!(session.granted_turn_permissions().await, None);

    // The rejection should not poison the non-escalated path for the same
    // command. Force DangerFullAccess so this check stays focused on approval
    // policy rather than platform-specific sandbox behavior.
    let turn_context_mut = Arc::get_mut(&mut turn_context).expect("unique turn context Arc");
    turn_context_mut.permission_profile = PermissionProfile::Disabled;

    let file_system_sandbox_policy = turn_context.file_system_sandbox_policy();
    let command = session
        .user_shell()
        .derive_exec_args(command_script, turn_context.tools_config.allow_login_shell);
    let exec_approval_requirement = session
        .services
        .exec_policy
        .create_exec_approval_requirement_for_command(ExecApprovalRequest {
            command: &command,
            approval_policy: turn_context.approval_policy.value(),
            permission_profile: turn_context.permission_profile(),
            file_system_sandbox_policy: &file_system_sandbox_policy,
            #[allow(deprecated)]
            sandbox_cwd: turn_context.cwd.as_path(),
            sandbox_permissions: SandboxPermissions::UseDefault,
            prefix_rule: None,
        })
        .await;
    assert!(matches!(
        exec_approval_requirement,
        ExecApprovalRequirement::Skip { .. }
    ));
}
#[tokio::test]
async fn session_start_hooks_only_load_from_trusted_project_layers() -> std::io::Result<()> {
    let temp = tempfile::tempdir()?;
    let codex_home = temp.path().join("home");
    let project_root = temp.path().join("project");
    let nested = project_root.join("nested");
    let root_dot_codex = project_root.join(".codex");
    let nested_dot_codex = nested.join(".codex");

    std::fs::create_dir_all(&codex_home)?;
    std::fs::create_dir_all(&nested_dot_codex)?;
    std::fs::write(project_root.join(".git"), "gitdir: here")?;
    write_project_hooks(&root_dot_codex)?;
    write_project_hooks(&nested_dot_codex)?;
    write_project_trust_config(&codex_home, &[(&nested, TrustLevel::Trusted)]).await?;

    let config = ConfigBuilder::default()
        .codex_home(codex_home)
        .fallback_cwd(Some(nested))
        .build()
        .await?;

    let hook_list = hooks::list_hooks(hooks::HooksConfig {
        feature_enabled: true,
        config_layer_stack: Some(
            crate::config::hook_config_layer_stack_from_config_layer_stack(
                &config.config_layer_stack,
            ),
        ),
        ..hooks::HooksConfig::default()
    });
    let expected_source_path = codex_utils_absolute_path::AbsolutePathBuf::from_absolute_path(
        nested_dot_codex.join("hooks.json"),
    )?;
    assert_eq!(
        hook_list
            .hooks
            .iter()
            .map(|hook| &hook.source_path)
            .collect::<Vec<_>>(),
        vec![&expected_source_path],
    );
    assert_eq!(
        hook_list.hooks[0].trust_status,
        protocol::protocol::HookTrustStatus::Untrusted
    );
    assert!(preview_session_start_hooks(&config).await?.is_empty());

    Ok(())
}

#[tokio::test]
async fn session_start_hooks_require_project_trust_without_config_toml() -> std::io::Result<()> {
    let temp = tempfile::tempdir()?;
    let project_root = temp.path().join("project");
    let nested = project_root.join("nested");
    let dot_codex = project_root.join(".codex");
    std::fs::create_dir_all(&nested)?;
    std::fs::write(project_root.join(".git"), "gitdir: here")?;
    write_project_hooks(&dot_codex)?;

    let cases = [
        ("unknown", Vec::<(&Path, TrustLevel)>::new(), 0_usize),
        (
            "untrusted",
            vec![(&project_root as &Path, TrustLevel::Untrusted)],
            0_usize,
        ),
        (
            "trusted",
            vec![(&project_root as &Path, TrustLevel::Trusted)],
            1_usize,
        ),
    ];

    for (name, trust_entries, expected_hooks) in cases {
        let codex_home = temp.path().join(format!("home_{name}"));
        std::fs::create_dir_all(&codex_home)?;
        write_project_trust_config(&codex_home, &trust_entries).await?;

        let config = ConfigBuilder::default()
            .codex_home(codex_home)
            .fallback_cwd(Some(nested.clone()))
            .build()
            .await?;

        let hook_list = hooks::list_hooks(hooks::HooksConfig {
            feature_enabled: true,
            config_layer_stack: Some(
                crate::config::hook_config_layer_stack_from_config_layer_stack(
                    &config.config_layer_stack,
                ),
            ),
            ..hooks::HooksConfig::default()
        });
        assert_eq!(
            hook_list.hooks.len(),
            expected_hooks,
            "unexpected discovered hook count for {name}",
        );
        assert!(preview_session_start_hooks(&config).await?.is_empty());
        if expected_hooks == 1 {
            assert_eq!(
                hook_list.hooks[0].trust_status,
                protocol::protocol::HookTrustStatus::Untrusted
            );
        }
    }

    Ok(())
}
